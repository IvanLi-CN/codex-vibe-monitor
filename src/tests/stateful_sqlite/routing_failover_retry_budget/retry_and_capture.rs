use super::*;
use serde_json::json;

#[tokio::test]
async fn pool_route_retries_same_account_before_switching() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer upstream-primary", 2)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 3);

    {
        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    }

    let route_account_id = wait_for_test_sticky_route_account_id(&state.pool, "sticky-001")
        .await
        .expect("sticky route should be rebound after success");
    assert_eq!(route_account_id, primary_id);
    assert_ne!(route_account_id, secondary_id);

    upstream_handle.abort();
}

#[test]
fn pool_route_switches_accounts_after_same_account_retries_are_exhausted() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_retry_upstream(&[("Bearer upstream-primary", 8)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        let secondary_id =
            insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-002"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        let primary_status: String =
            sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
                .bind(primary_id)
                .fetch_one(&state.pool)
                .await
                .expect("load primary status");
        assert_eq!(primary_status, "active");
        assert_eq!(
            wait_for_test_sticky_route_account_id(&state.pool, "sticky-002").await,
            Some(secondary_id)
        );

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_switches_accounts_immediately_after_upstream_429() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        let secondary_id =
            insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-switch"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        wait_for_pool_upstream_request_attempts(&state.pool, 2).await;
        let attempt_rows = sqlx::query_as::<_, (i64, i64, i64, Option<String>)>(
            r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
        )
        .fetch_all(&state.pool)
        .await
        .expect("load attempt rows");
        assert_eq!(attempt_rows.len(), 2);
        assert_eq!(attempt_rows[0].0, 1);
        assert_eq!(attempt_rows[0].1, 1);
        assert_eq!(attempt_rows[0].2, 1);
        assert_eq!(
            attempt_rows[0].3.as_deref(),
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        );
        assert_eq!(attempt_rows[1].0, 2);
        assert_eq!(attempt_rows[1].1, 2);
        assert_eq!(attempt_rows[1].2, 1);
        assert_eq!(attempt_rows[1].3, None);

        let route_account_id =
            wait_for_test_sticky_route_account_id(&state.pool, "sticky-429-switch")
                .await
                .expect("sticky route should move to the successful account");
        assert_eq!(route_account_id, secondary_id);
        assert_ne!(route_account_id, primary_id);

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_waits_for_recovered_alternate_after_upstream_429() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(180),
            Duration::from_millis(10),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        let secondary_id =
            insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
        set_test_account_status(&state.pool, secondary_id, "needs_reauth").await;

        let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
        let pool = state.pool.clone();
        let runtime_handle = tokio::runtime::Handle::current();
        let release_task = std::thread::spawn(move || {
            wait_started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("request should signal once the bounded wait starts");
            std::thread::sleep(Duration::from_millis(40));
            runtime_handle.block_on(async move {
                set_test_account_status(&pool, secondary_id, "active").await;
            });
        });

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
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-wait-recovered"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;
        let elapsed = started.elapsed();

        release_task
            .join()
            .expect("alternate release thread should join");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            elapsed >= Duration::from_millis(35),
            "request should wait for the alternate to recover, elapsed={elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(900),
            "bounded wait should still stay within a loaded-runner recovery budget, elapsed={elapsed:?}"
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_group_without_upstream_429_retry_switches_accounts_immediately() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Primary",
            "upstream-primary",
            Some("latam"),
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-group-off"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));

        drop(attempts);
        upstream_handle.abort();
    });
}

#[test]
fn pool_route_transit_upstream_429_switches_account_without_group_retry() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_responses_upstream(&[("Bearer upstream-primary", 2)]).await;
        let base_state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        let state =
            clone_state_with_pool_group_429_retry_delay_override(&base_state, Some(Duration::ZERO));
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Primary",
            "upstream-primary",
            Some("latam"),
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-group-retry"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        wait_for_pool_upstream_request_attempts(&state.pool, 2).await;
        let attempt_rows = sqlx::query_as::<_, (i64, i64, i64, Option<String>)>(
            r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
        )
        .fetch_all(&state.pool)
        .await
        .expect("load retry attempt rows");
        assert_eq!(attempt_rows.len(), 2);
        assert_eq!(
            attempt_rows[0],
            (
                1,
                1,
                1,
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429.to_string())
            )
        );
        assert_eq!(attempt_rows[1], (2, 2, 1, None));

        let route_account_id =
            wait_for_test_sticky_route_account_id(&state.pool, "sticky-429-group-retry")
                .await
                .expect("sticky route should stay on primary account");
        assert_ne!(route_account_id, primary_id);

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_transit_upstream_429_uses_account_failover_after_server_error() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_sequential_failure_responses_upstream(vec![(
                "Bearer upstream-primary",
                vec![
                    StatusCode::INTERNAL_SERVER_ERROR,
                    StatusCode::TOO_MANY_REQUESTS,
                    StatusCode::TOO_MANY_REQUESTS,
                ],
            )])
            .await;
        let base_state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        let state =
            clone_state_with_pool_group_429_retry_delay_override(&base_state, Some(Duration::ZERO));
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Primary",
            "upstream-primary",
            Some("latam"),
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-mixed-budget"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(2));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        wait_for_pool_upstream_request_attempts(&state.pool, 3).await;
        let attempt_rows = sqlx::query_as::<_, (i64, i64, i64, Option<String>)>(
            r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
        )
        .fetch_all(&state.pool)
        .await
        .expect("load retry attempt rows");
        assert_eq!(attempt_rows.len(), 3);
        assert_eq!(
            attempt_rows[0],
            (
                1,
                1,
                1,
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX.to_string())
            )
        );
        assert_eq!(
            attempt_rows[1],
            (
                2,
                1,
                2,
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429.to_string())
            )
        );
        assert_eq!(attempt_rows[2], (3, 2, 1, None));

        let route_account_id =
            wait_for_test_sticky_route_account_id(&state.pool, "sticky-429-mixed-budget")
                .await
                .expect("sticky route should stay on primary account");
        assert_ne!(route_account_id, primary_id);

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_switches_accounts_immediately_after_upstream_402() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_static_failure_responses_upstream(&[(
                "Bearer upstream-primary",
                StatusCode::PAYMENT_REQUIRED,
            )])
            .await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        let secondary_id =
            insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-402-switch"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        let primary_status: String =
            sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
                .bind(primary_id)
                .fetch_one(&state.pool)
                .await
                .expect("load primary status");
        assert_eq!(primary_status, "error");
        assert_eq!(
            wait_for_test_sticky_route_account_id(&state.pool, "sticky-402-switch").await,
            Some(secondary_id)
        );

        wait_for_pool_upstream_request_attempts(&state.pool, 2).await;
        let attempt_rows = sqlx::query_as::<_, (i64, Option<String>)>(
            r#"
        SELECT distinct_account_index, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
        )
        .fetch_all(&state.pool)
        .await
        .expect("load attempt rows");
        assert_eq!(attempt_rows[0].0, 1);
        assert_eq!(
            attempt_rows[0].1.as_deref(),
            Some(PROXY_FAILURE_UPSTREAM_HTTP_402)
        );
        assert_eq!(attempt_rows[1].0, 2);
        assert_eq!(attempt_rows[1].1, None);

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn pool_route_http_4xx_does_not_create_sticky_route() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        status: String,
        http_status: Option<i64>,
        error_message: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::BAD_REQUEST)],
    )
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-400-no-route-success"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read passthrough 4xx response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode passthrough 4xx body");
    assert_eq!(payload["error"]["code"], "server_error");
    assert!(
        payload["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("upstream failure for Bearer upstream-primary"))
    );
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    wait_for_pool_attempt_status(
        &state.pool,
        1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
    )
    .await;
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-400-no-route-success").await,
        None,
        "HTTP 4xx must not be treated as pool route success for sticky upsert",
    );

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT status, http_status, error_message
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load 4xx attempt rows");
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(attempt_rows[0].http_status, Some(400));
    assert!(
        attempt_rows[0]
            .error_message
            .as_deref()
            .is_some_and(|message| !message.is_empty()),
        "4xx attempt should preserve error information: {attempt_rows:?}",
    );
    let account_route_state = sqlx::query_as::<_, (String, Option<String>, Option<String>, i64)>(
        r#"
            SELECT status, last_route_failure_at, cooldown_until, consecutive_route_failures
            FROM pool_upstream_accounts
            WHERE id = ?1
            "#,
    )
    .bind(primary_id)
    .fetch_one(&state.pool)
    .await
    .expect("load account route state after 4xx");
    assert_eq!(account_route_state.0, "active");
    assert!(account_route_state.1.is_none());
    assert!(account_route_state.2.is_none());
    assert_eq!(account_route_state.3, 0);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_live_request_switches_accounts_immediately_after_upstream_429() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_rate_limit_echo_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base_body_limit_and_read_timeout(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
            Duration::from_millis(50),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/echo?mode=pool-live-429".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from("hello-live-429"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);
        assert_eq!(payload["body"], "hello-live-429");

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn pool_route_retries_first_chunk_failure_before_switching() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_first_chunk_retry_upstream(&[("Bearer upstream-primary", 2)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-003"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 3);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    let route_account_id = wait_for_test_sticky_route_account_id(&state.pool, "sticky-003")
        .await
        .expect("sticky route should remain on the recovered account");
    assert_eq!(route_account_id, primary_id);
    assert_ne!(route_account_id, secondary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_non_responses_timeouts_retry_same_account_before_switching() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_account_id: Option<i64>,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) = spawn_pool_delayed_headers_retry_upstream(
        Duration::from_millis(400),
        &[("Bearer upstream-primary", 2)],
    )
    .await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(150);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "upstream-primary",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;
    let secondary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "upstream-secondary",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(
            r#"{"model":"gpt-5","messages":[{"role":"user","content":"hello"}],"stickyKey":"sticky-non-responses-timeout-retry-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read non-responses retry response");
    let payload: Value = serde_json::from_slice(&body).expect("decode non-responses retry body");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 3).await;
    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            upstream_account_id,
            distinct_account_index,
            same_account_retry_index,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load non-responses timeout retry rows");
    assert!(
        attempt_rows.len() == 3 || attempt_rows.len() == 4,
        "expected 3 or 4 attempts, got {}",
        attempt_rows.len()
    );
    let is_timeout_or_stream_failure = |row: &AttemptRouteRow| {
        row.status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
            && row.failure_kind.as_deref().is_some_and(|kind| {
                kind == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
                    || kind == PROXY_FAILURE_UPSTREAM_STREAM_ERROR
            })
    };
    assert_eq!(attempt_rows[0].upstream_account_id, Some(primary_id));
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert!(is_timeout_or_stream_failure(&attempt_rows[0]));
    assert_eq!(attempt_rows[1].upstream_account_id, Some(primary_id));
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert!(is_timeout_or_stream_failure(&attempt_rows[1]));
    assert_eq!(attempt_rows[2].upstream_account_id, Some(primary_id));
    assert_eq!(attempt_rows[2].distinct_account_index, 1);
    assert_eq!(attempt_rows[2].same_account_retry_index, 3);
    let sticky_account_id = wait_for_test_sticky_route_account_id(
        &state.pool,
        "sticky-non-responses-timeout-retry-001",
    )
    .await
    .expect("sticky route should be recorded after success");

    let attempts = attempts
        .lock()
        .expect("lock delayed headers retry attempts");
    match payload["authorization"].as_str() {
        Some("Bearer upstream-primary") => {
            assert_eq!(payload["attempt"], 3);
            assert_eq!(attempt_rows.len(), 3);
            assert_eq!(
                attempt_rows[2].status,
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            );
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
            assert_eq!(sticky_account_id, primary_id);
        }
        Some("Bearer upstream-secondary") => {
            assert_eq!(payload["attempt"], 1);
            assert_eq!(attempt_rows.len(), 4);
            assert!(is_timeout_or_stream_failure(&attempt_rows[2]));
            assert_eq!(attempt_rows[3].upstream_account_id, Some(secondary_id));
            assert_eq!(attempt_rows[3].distinct_account_index, 2);
            assert_eq!(attempt_rows[3].same_account_retry_index, 1);
            assert_eq!(
                attempt_rows[3].status,
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            );
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
            assert_eq!(sticky_account_id, secondary_id);
        }
        other => panic!("unexpected upstream authorization: {other:?}"),
    }
    drop(attempts);

    upstream_handle.abort();
}

#[test]
fn pool_route_switches_accounts_after_first_chunk_failures_are_exhausted() {
    run_routing_failover_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_first_chunk_retry_upstream(&[("Bearer upstream-primary", 8)]).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        let secondary_id =
            insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(
                r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-004"}"#
                    .as_bytes()
                    .to_vec(),
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 1);

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        let primary_status: String =
            sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
                .bind(primary_id)
                .fetch_one(&state.pool)
                .await
                .expect("load primary status");
        assert_eq!(primary_status, "active");
        assert_eq!(
            wait_for_test_sticky_route_account_id(&state.pool, "sticky-004").await,
            Some(secondary_id)
        );

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn capture_target_pool_route_retries_first_chunk_failure_and_persists_single_invocation() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_first_chunk_retry_upstream(&[("Bearer upstream-primary", 2)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let request_payload = json!({
        "model": "gpt-5.2-codex",
        "stream": false,
        "input": "hello",
        "stickyKey": "sticky-cap-001",
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            serde_json::to_vec(&request_payload).expect("serialize capture retry request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode capture retry payload");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 3);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    wait_for_codex_invocations(&state.pool, 1).await;
    assert_eq!(count_codex_invocations(&state.pool).await, 1);
    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 0);

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");
    assert_eq!(row.status.as_deref(), Some("success"));

    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(payload_json["upstreamScope"].as_str(), Some("internal"));
    assert_eq!(payload_json["routeMode"].as_str(), Some("pool"));
    assert_eq!(payload_json["stickyKey"].as_str(), Some("sticky-cap-001"));
    assert_eq!(payload_json["upstreamAccountId"].as_i64(), Some(primary_id));
    assert_eq!(
        payload_json["upstreamAccountName"].as_str(),
        Some("Primary")
    );
    assert!(payload_json["proxyDisplayName"].is_null());
    assert_eq!(
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-cap-001").await,
        Some(primary_id)
    );
    assert_ne!(primary_id, secondary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_no_content_success_finalizes_pending_attempt() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_payload = json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
        "stickyKey": "sticky-cap-204",
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses?mode=no-content".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            serde_json::to_vec(&request_payload)
                .expect("serialize capture no-content request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read no-content response body");
    assert!(body.is_empty(), "204 response body should stay empty");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;

    let mut attempt_row = None;
    for _ in 0..20 {
        attempt_row = sqlx::query_as::<
            _,
            (
                String,
                Option<String>,
                Option<i64>,
                Option<f64>,
                Option<String>,
                Option<i64>,
            ),
        >(
            r#"
            SELECT
                status,
                finished_at,
                http_status,
                stream_latency_ms,
                failure_kind,
                upstream_account_id
            FROM pool_upstream_request_attempts
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query pool attempt row");
        if attempt_row
            .as_ref()
            .is_some_and(|row| row.0 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let attempt_row = attempt_row.expect("pool attempt row should be persisted");

    assert_eq!(attempt_row.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert!(
        attempt_row
            .1
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(attempt_row.2, Some(204));
    assert_eq!(attempt_row.3, Some(0.0));
    assert_eq!(attempt_row.4, None);
    assert_eq!(attempt_row.5, Some(primary_id));

    tokio::time::sleep(Duration::from_millis(50)).await;
    let invocation_row = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE source = 'proxy'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load no-content invocation row");
    assert_eq!(invocation_row.0, "success");
    assert_eq!(invocation_row.1, None);

    upstream_handle.abort();
}
