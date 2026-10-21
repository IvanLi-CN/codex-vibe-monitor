use super::*;
use serde_json::json;

fn run_timeout_future_with_large_stack<Fut>(future: Fut)
where
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name("routing-timeout-large-stack".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build large-stack timeout test runtime")
                .block_on(future)
        })
        .expect("spawn large-stack timeout test worker")
        .join()
        .expect("join large-stack timeout test worker");
}

#[test]
fn capture_target_pool_route_timeout_ignores_legacy_group_proxy_error_for_transit() {
    run_timeout_future_with_large_stack(async move {
        // This fixture must time out even when the runner stalls. Competing short
        // sleeps can make the response win after both timers have become ready.
        let upstream = Router::new().route(
            "/v1/responses",
            post(|| async {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(http_header::CONTENT_TYPE, "application/json")
                    .body(Body::from_stream(stream::pending::<
                        Result<Bytes, Infallible>,
                    >()))
                    .expect("build pending upstream response")
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind pending upstream");
        let shared_upstream_base = format!(
            "http://{}",
            listener.local_addr().expect("pending upstream address")
        );
        let shared_upstream_handle = tokio::spawn(async move {
            axum::serve(listener, upstream)
                .await
                .expect("serve pending upstream");
        });
        let (alternate_base, alternate_attempts, alternate_handle) =
            spawn_pool_retry_upstream(&[("Bearer route-broken-alt-invalid-group", 0)]).await;
        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse("https://api.openai.com/").expect("valid upstream base url");
        config.pool_upstream_responses_attempt_timeout = Duration::from_secs(2);
        let state = test_state_from_config(config, true).await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let initial_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Shared Route A",
            "route-shared-a-broken-alt",
            None,
            None,
            Some(shared_upstream_base.as_str()),
        )
        .await;
        let broken_same_route_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Broken Shared Route",
            "route-shared-b-broken-alt",
            None,
            None,
            Some(shared_upstream_base.as_str()),
        )
        .await;
        set_test_account_group_name(
            &state.pool,
            broken_same_route_id,
            Some("broken-shared-route-group"),
        )
        .await;
        let broken_alternate_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Broken Alternate Route",
            "route-broken-alt-invalid-group",
            None,
            None,
            Some(alternate_base.as_str()),
        )
        .await;
        set_test_account_group_name(&state.pool, broken_alternate_id, Some("broken-alt-group"))
            .await;
        let sticky_seen_at = format_test_recent_active_timestamp(Utc::now());
        upsert_test_sticky_route_at(
            &state.pool,
            "sticky-timeout-broken-alt-group",
            initial_account_id,
            &sticky_seen_at,
        )
        .await;

        let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-broken-alt-group"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read timeout broken-alt response body");
        let response_payload: Value =
            serde_json::from_slice(&body).expect("decode timeout broken-alt response body");
        assert_eq!(response_payload["ok"], true);
        assert_eq!(
            response_payload["authorization"], "Bearer route-broken-alt-invalid-group",
            "the alternate transit route must handle the request despite its legacy group"
        );
        wait_for_codex_invocations(&state.pool, 1).await;
        wait_for_pool_upstream_request_attempts(&state.pool, 2).await;
        let attempts = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, failure_kind FROM pool_upstream_request_attempts ORDER BY attempt_index",
        )
        .fetch_all(&state.pool)
        .await
        .expect("load controlled timeout failover attempts");
        assert_eq!(attempts.len(), 2);
        assert_eq!(
            attempts[0],
            (
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE.to_string(),
                Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string()),
            )
        );
        assert_eq!(attempts[1].0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
        assert_eq!(
            alternate_attempts
                .lock()
                .expect("lock controlled alternate attempts")
                .get("Bearer route-broken-alt-invalid-group")
                .copied(),
            Some(1)
        );

        shared_upstream_handle.abort();
        alternate_handle.abort();
    });
}

#[tokio::test]
async fn capture_target_pool_route_timeout_replay_failover_preserves_no_alternate_terminal_reason()
{
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_route_key: Option<String>,
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (shared_upstream_base, shared_upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(120);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Shared Route A",
        "route-shared-a",
        None,
        None,
        Some(shared_upstream_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Shared Route B",
        "route-shared-b",
        None,
        None,
        Some(shared_upstream_base.as_str()),
    )
    .await;
    let exhausted_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Exhausted Other Route",
        "route-exhausted",
        None,
        None,
        Some("https://exhausted.example.com/backend-api/codex"),
    )
    .await;
    insert_test_pool_limit_sample(&state, exhausted_id, Some(100.0), Some(0.0)).await;

    let chunks = stream::iter(vec![Ok::<Bytes, io::Error>(Bytes::from_static(
        br#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-replay-no-alt-001"}"#,
    ))]);
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
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
        Body::from_stream(chunks),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout replay no-alternate response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode timeout replay no-alternate response body");
    assert!(
        response_payload["error"]
            .as_str()
            .expect("timeout replay no-alternate error should be present")
            .contains("no alternate upstream route is available after timeout")
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 2).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            upstream_route_key,
            attempt_index,
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
    .expect("load timeout replay no-alternate rows");
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load timeout replay no-alternate payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout replay no-alternate payload should be present"),
    )
    .expect("decode timeout replay no-alternate payload");
    assert!(
        row.error_message.as_deref().is_some_and(
            |msg| msg.contains("no alternate upstream route is available after timeout")
        )
    );
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT),
    );
    assert!(payload["upstreamErrorMessage"].is_null());

    shared_upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_timeout_can_switch_twice_then_succeed() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        upstream_route_key: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (slow_one_base, slow_one_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let (slow_two_base, slow_two_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let (fast_three_base, _attempts, fast_three_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-three", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(120);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Timeout Route One",
        "route-one",
        None,
        None,
        Some(slow_one_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Timeout Route Two",
        "route-two",
        None,
        None,
        Some(slow_two_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Success Route Three",
        "route-three",
        None,
        None,
        Some(fast_three_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-switch-002"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout double-switch success body");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 2).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status,
            upstream_route_key
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load timeout double-switch rows");
    assert_eq!(attempt_rows.len(), 3);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 1);
    assert_eq!(attempt_rows[2].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(
        attempt_rows[2].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    );
    assert_eq!(attempt_rows[2].attempt_index, 3);
    assert_eq!(attempt_rows[2].distinct_account_index, 3);
    assert_ne!(
        attempt_rows[0].upstream_route_key,
        attempt_rows[1].upstream_route_key
    );
    assert_ne!(
        attempt_rows[1].upstream_route_key,
        attempt_rows[2].upstream_route_key
    );

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load timeout double-switch payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout double-switch payload should be present"),
    )
    .expect("decode timeout double-switch payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(3));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert!(payload["poolAttemptTerminalReason"].is_null());

    slow_one_handle.abort();
    slow_two_handle.abort();
    fast_three_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_timeout_exhausts_after_three_routes() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (slow_one_base, slow_one_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let (slow_two_base, slow_two_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let (slow_three_base, slow_three_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let (fast_four_base, attempts, fast_four_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-four", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(120);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Timeout Route One",
        "route-one",
        None,
        None,
        Some(slow_one_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Timeout Route Two",
        "route-two",
        None,
        None,
        Some(slow_two_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Timeout Route Three",
        "route-three",
        None,
        None,
        Some(slow_three_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Unused Route Four",
        "route-four",
        None,
        None,
        Some(fast_four_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-stop-003"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout terminal response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode timeout terminal response body");
    assert!(
        response_payload["error"]
            .as_str()
            .expect("timeout terminal error should be present")
            .contains("no alternate upstream route is available after timeout")
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 3).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            attempt_index,
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
    .expect("load timeout terminal rows");
    assert_eq!(attempt_rows.len(), 3);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 1);
    assert_eq!(attempt_rows[2].same_account_retry_index, 1);
    let attempts = attempts.lock().expect("lock unused route attempts");
    assert_eq!(attempts.get("Bearer route-four").copied(), None);
    drop(attempts);

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load timeout terminal payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout terminal payload should be present"),
    )
    .expect("decode timeout terminal payload");
    assert!(
        row.error_message.as_deref().is_some_and(
            |msg| msg.contains("no alternate upstream route is available after timeout")
        )
    );
    assert_eq!(
        payload["failureKind"].as_str(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT),
    );
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(3));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT),
    );
    assert!(payload["upstreamErrorMessage"].is_null());

    slow_one_handle.abort();
    slow_two_handle.abort();
    slow_three_handle.abort();
    fast_four_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_total_timeout_can_succeed_on_second_route() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        attempt_index: i64,
        distinct_account_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (slow_one_base, slow_one_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(750)).await;
    let (fast_two_base, attempts, fast_two_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-two", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(300);
    config.pool_upstream_responses_total_timeout = Duration::from_millis(900);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let slow_one_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Timeout Route One",
        "route-one",
        None,
        None,
        Some(slow_one_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Success Route Two",
        "route-two",
        None,
        None,
        Some(fast_two_base.as_str()),
    )
    .await;

    let sticky_key = "sticky-timeout-budget-success-004";
    let sticky_seen_at = format_test_recent_active_timestamp(Utc::now());
    upsert_test_sticky_route_at(&state.pool, sticky_key, slow_one_id, &sticky_seen_at).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            format!(r#"{{"model":"gpt-5","input":"hello","stickyKey":"{sticky_key}"}}"#)
                .into_bytes(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout budget success response body");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_upstream_request_attempts(&state.pool, 2).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            attempt_index,
            distinct_account_index,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load timeout budget success rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(
        attempt_rows[0].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
    );
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(attempt_rows[1].distinct_account_index, 2);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    );

    let attempts = attempts.lock().expect("lock route-two attempts");
    assert_eq!(attempts.get("Bearer route-two").copied(), Some(1));
    drop(attempts);

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load timeout budget success payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout budget success payload should be present"),
    )
    .expect("decode timeout budget success payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(2));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
    assert!(payload["poolAttemptTerminalReason"].is_null());

    slow_one_handle.abort();
    fast_two_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_stream_timeout_does_not_cap_pre_first_byte_failover() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (slow_one_base, slow_one_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let (slow_two_base, slow_two_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let (fast_three_base, attempts, fast_three_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-three", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(180);
    config.pool_upstream_responses_total_timeout = Duration::from_millis(300);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Timeout Route One",
        "route-one",
        None,
        None,
        Some(slow_one_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Timeout Route Two",
        "route-two",
        None,
        None,
        Some(slow_two_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Unused Route Three",
        "route-three",
        None,
        None,
        Some(fast_three_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-budget-stop-005"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read third-route success response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode third-route success body");
    assert_eq!(response_payload["ok"].as_bool(), Some(true));
    assert_eq!(
        response_payload["authorization"].as_str(),
        Some("Bearer route-three"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 3).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load third-route success rows");
    assert_eq!(attempt_rows.len(), 3);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(attempt_rows[1].distinct_account_index, 2);
    assert_eq!(attempt_rows[1].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(attempt_rows[2].attempt_index, 3);
    assert_eq!(attempt_rows[2].distinct_account_index, 3);
    assert_eq!(attempt_rows[2].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[2].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    );

    let attempts = attempts.lock().expect("lock unused route attempts");
    assert_eq!(attempts.get("Bearer route-three").copied(), Some(1));
    drop(attempts);

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load third-route success payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("third-route success payload should be present"),
    )
    .expect("decode third-route success payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(3));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert!(payload["poolAttemptTerminalReason"].is_null());

    slow_one_handle.abort();
    slow_two_handle.abort();
    fast_three_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_compact_stream_timeout_does_not_cap_pre_first_byte_failover() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        attempt_index: i64,
        distinct_account_index: i64,
        status: String,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (slow_one_base, _slow_one_requests, slow_one_handle) =
        spawn_capture_target_body_upstream().await;
    let (slow_two_base, _slow_two_requests, slow_two_handle) =
        spawn_capture_target_body_upstream().await;
    let (fast_three_base, fast_three_attempts, fast_three_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-three", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(180);
    config.pool_upstream_responses_total_timeout = Duration::from_millis(300);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Compact Timeout Route One",
        "route-one",
        None,
        None,
        Some(slow_one_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Compact Timeout Route Two",
        "route-two",
        None,
        None,
        Some(slow_two_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Compact Unused Route Three",
        "route-three",
        None,
        None,
        Some(fast_three_base.as_str()),
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_002",
        "input": [{"role": "user", "content": "compact this thread"}],
    }))
    .expect("serialize compact request body");
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses/compact?mode=slow-first-chunk"
                .parse()
                .expect("valid uri"),
        ),
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
        Body::from(request_body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact third-route success response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode compact third-route success body");
    assert_eq!(response_payload["ok"].as_bool(), Some(true));
    assert_eq!(
        response_payload["authorization"].as_str(),
        Some("Bearer route-three"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 3).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            attempt_index,
            distinct_account_index,
            status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load compact third-route rows");
    assert_eq!(attempt_rows.len(), 3);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(
        attempt_rows[2].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    );
    assert_eq!(attempt_rows[2].distinct_account_index, 3);

    let fast_three_attempts = fast_three_attempts
        .lock()
        .expect("lock compact route-three attempts");
    assert_eq!(
        fast_three_attempts.get("Bearer route-three").copied(),
        Some(1)
    );
    drop(fast_three_attempts);

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load compact third-route success payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("compact third-route success payload should be present"),
    )
    .expect("decode compact third-route success payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(3));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert!(payload["poolAttemptTerminalReason"].is_null());

    slow_one_handle.abort();
    slow_two_handle.abort();
    fast_three_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_total_timeout_starts_at_first_upstream_attempt() {
    let (fast_upstream_base, attempts, fast_upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-one", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(450);
    config.pool_upstream_responses_total_timeout = Duration::from_millis(240);
    config.openai_proxy_request_read_timeout = Duration::from_millis(900);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Fast Route One",
        "route-one",
        None,
        None,
        Some(fast_upstream_base.as_str()),
    )
    .await;

    let request_body =
        br#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-budget-start-007"}"#
            .to_vec();
    let content_length =
        HeaderValue::from_str(&request_body.len().to_string()).expect("content length header");
    let slow_body = stream::unfold(Some(request_body), |state| async move {
        match state {
            Some(body) => {
                tokio::time::sleep(Duration::from_millis(420)).await;
                Some((Ok::<Bytes, Infallible>(Bytes::from(body)), None))
            }
            None => None,
        }
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
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
            (http_header::CONTENT_LENGTH, content_length),
        ]),
        Body::from_stream(slow_body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read delayed body success response");

    let attempts = attempts.lock().expect("lock fast route attempts");
    assert_eq!(attempts.get("Bearer route-one").copied(), Some(1));
    drop(attempts);

    fast_upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_total_timeout_caps_same_account_retry_before_first_byte() {
    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        error_message: Option<String>,
        payload: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        attempt_index: i64,
        distinct_account_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    let (retry_upstream_base, retry_attempts, retry_upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-one", 2)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(180);
    config.pool_upstream_responses_total_timeout = Duration::from_millis(300);
    let state = clone_state_with_fallback_proxy_429_retry_delay_override(
        &test_state_from_config(config, true).await,
        None,
    );
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Retry Route One",
        "route-one",
        None,
        None,
        Some(retry_upstream_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-budget-distinct-008"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read same-account retry timeout response");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode same-account retry timeout body");
    assert_eq!(
        response_payload["error"].as_str(),
        Some("pool upstream total timeout exhausted after 300ms"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 1).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            attempt_index,
            distinct_account_index,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load same-account retry timeout rows");
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
    );
    let attempts = retry_attempts.lock().expect("lock retry route attempts");
    assert_eq!(attempts.get("Bearer route-one").copied(), Some(1));
    drop(attempts);

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load same-account timeout exhaustion payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("same-account retry timeout payload should be present"),
    )
    .expect("decode same-account retry timeout payload");
    assert!(
        row.error_message.as_deref().is_some_and(|msg| {
            msg.contains("pool upstream total timeout exhausted after 300ms")
        })
    );
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED),
    );

    retry_upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_compact_total_timeout_caps_same_account_retry_before_first_byte()
{
    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        error_message: Option<String>,
        payload: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        attempt_index: i64,
        distinct_account_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    let (retry_upstream_base, retry_attempts, retry_upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-one", 2)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(180);
    config.pool_upstream_responses_total_timeout = Duration::from_millis(300);
    let state = clone_state_with_fallback_proxy_429_retry_delay_override(
        &test_state_from_config(config, true).await,
        None,
    );
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Retry Route One",
        "route-one",
        None,
        None,
        Some(retry_upstream_base.as_str()),
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_timeout_001",
        "input": [{"role": "user", "content": "compact timeout budget"}],
    }))
    .expect("serialize compact timeout request body");
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses/compact".parse().expect("valid compact uri")),
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
        Body::from(request_body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact same-account retry timeout response");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode compact same-account retry timeout body");
    assert_eq!(
        response_payload["error"].as_str(),
        Some("pool upstream total timeout exhausted after 300ms"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 1).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            attempt_index,
            distinct_account_index,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load compact same-account retry timeout rows");
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
    );
    let attempts = retry_attempts
        .lock()
        .expect("lock compact retry route attempts");
    assert_eq!(attempts.get("Bearer route-one").copied(), Some(1));
    drop(attempts);

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load compact same-account timeout exhaustion payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("compact same-account retry timeout payload should be present"),
    )
    .expect("decode compact same-account retry timeout payload");
    assert!(
        row.error_message.as_deref().is_some_and(|msg| {
            msg.contains("pool upstream total timeout exhausted after 300ms")
        })
    );
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED),
    );

    retry_upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_retries_same_account_on_server_overloaded_before_forwarding() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        distinct_account_index: i64,
        status: String,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct AccountActionRow {
        action: Option<String>,
        reason_code: Option<String>,
        cooldown_until: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_metadata_prefixed_response_failed_retry_upstream(&[("Bearer route-one", 3)])
            .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Retry Route One",
        "route-one",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-overloaded-retry-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read retryable overloaded response");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode retryable overloaded success body");
    assert_eq!(response_payload["ok"].as_bool(), Some(true));
    assert_eq!(
        response_payload["authorization"].as_str(),
        Some("Bearer route-one"),
    );
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 retryable overloaded body");
    assert!(!body_text.contains("response.failed"));
    assert!(!body_text.contains("server_is_overloaded"));

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 4).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT distinct_account_index, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load overloaded retry attempt rows");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(
        attempt_rows[2].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(
        attempt_rows[3].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );
    assert!(
        attempt_rows
            .iter()
            .all(|row| row.distinct_account_index == 1)
    );

    let row = sqlx::query_as::<_, AccountActionRow>(
        r#"
        SELECT last_action AS action, last_action_reason_code AS reason_code, cooldown_until
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load overloaded retry account state");
    assert_eq!(row.action.as_deref(), Some("route_recovered"));
    assert!(row.reason_code.is_none());
    assert!(row.cooldown_until.is_none());

    let recent_actions: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT action
        FROM pool_upstream_account_events
        WHERE account_id = ?1
        ORDER BY id DESC
        LIMIT 5
        "#,
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load overloaded retry account events");
    assert!(
        !recent_actions
            .iter()
            .any(|action| action == "route_cooldown_started")
    );

    let attempts = attempts.lock().expect("lock retryable overloaded attempts");
    assert_eq!(attempts.get("Bearer route-one").copied(), Some(4));
    drop(attempts);

    upstream_handle.abort();
}

#[test]
fn pool_responses_classify_server_error_overload_with_narrow_message_boundary() {
    assert_eq!(
        classify_pool_responses_overload(
            StatusCode::OK,
            Some("SERVER_ERROR"),
            Some(" Our servers are currently overloaded!!! Please try again later. "),
        ),
        Some(PoolResponsesOverloadRetryPolicy::SameAccountOnly),
    );
    assert_eq!(
        classify_pool_responses_overload(
            StatusCode::OK,
            Some("server_error"),
            Some("Our.servers\tare\ncurrently overloaded. Please try again later."),
        ),
        Some(PoolResponsesOverloadRetryPolicy::SameAccountOnly),
    );
    assert_eq!(
        classify_pool_responses_overload(
            StatusCode::OK,
            Some("server_error"),
            Some("Our servers are overloaded. Please try again later."),
        ),
        None,
    );
    assert_eq!(
        classify_pool_responses_overload(StatusCode::OK, Some("server_error"), Some("overloaded")),
        None,
    );
    assert_eq!(
        classify_pool_responses_overload(
            StatusCode::BAD_GATEWAY,
            Some("server_error"),
            Some("Our servers are currently overloaded."),
        ),
        None,
    );
    assert_eq!(
        classify_pool_responses_overload(
            StatusCode::OK,
            Some(UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED),
            Some("Our servers are currently overloaded."),
        ),
        Some(PoolResponsesOverloadRetryPolicy::LegacyFailover),
    );
}

#[test]
fn pool_responses_same_account_retry_backoff_is_capped_by_total_timeout() {
    let started_at = Instant::now() - Duration::from_millis(50);
    let capped = pool_retry_delay_with_total_timeout(
        Duration::from_secs(5),
        true,
        Some(Duration::from_millis(100)),
        Some(started_at),
    );
    assert!(capped <= Duration::from_millis(50));
    assert_eq!(
        pool_retry_delay_with_total_timeout(
            Duration::from_secs(5),
            false,
            Some(Duration::from_millis(1)),
            Some(started_at),
        ),
        Duration::from_secs(5)
    );
}

#[tokio::test]
async fn pool_openai_v1_responses_retries_server_error_overload_same_account_before_forwarding() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_route_key: Option<String>,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        proxy_binding_key_snapshot: Option<String>,
        status: String,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_metadata_prefixed_response_failed_retry_upstream_with_error(
            &[("Bearer route-one", 3)],
            "server_error",
            "Our servers are currently overloaded. Please try again later.",
        )
        .await;
    let base_state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let state =
        clone_state_with_fallback_proxy_429_retry_delay_override(&base_state, Some(Duration::ZERO));
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Server Error Retry Route",
        "route-one",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-server-error-retry-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read server_error overload retry response");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode server_error overload success body");
    assert_eq!(response_payload["ok"].as_bool(), Some(true));
    assert_eq!(
        response_payload["authorization"].as_str(),
        Some("Bearer route-one"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT upstream_route_key, distinct_account_index, same_account_retry_index,
               proxy_binding_key_snapshot, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load server_error overload retry attempt rows");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1]
    );
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.same_account_retry_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(attempt_rows.iter().all(|row| {
        row.upstream_route_key
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }));
    assert!(attempt_rows.iter().all(|row| {
        row.proxy_binding_key_snapshot
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }));
    assert!(
        attempt_rows
            .windows(2)
            .all(|rows| rows[0].upstream_route_key == rows[1].upstream_route_key)
    );
    assert!(
        attempt_rows.windows(2).all(|rows| {
            rows[0].proxy_binding_key_snapshot == rows[1].proxy_binding_key_snapshot
        })
    );
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.status.as_str())
            .collect::<Vec<_>>(),
        vec![
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ]
    );

    let attempts = attempts
        .lock()
        .expect("lock server_error overload retry attempts");
    assert_eq!(attempts.get("Bearer route-one").copied(), Some(4));
    drop(attempts);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_server_error_overload_does_not_switch_after_budget_exhaustion() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_route_key: Option<String>,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        proxy_binding_key_snapshot: Option<String>,
        status: String,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct InvocationFailureRow {
        status: String,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        error_message: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct ModelRouteFailureRow {
        state: String,
        last_failure_kind: Option<String>,
        cooldown_until: Option<String>,
    }

    let (same_route_base, same_route_attempts, same_route_handle) =
        spawn_pool_metadata_prefixed_response_failed_retry_upstream_with_error(
            &[
                ("Bearer route-one-primary", 10),
                ("Bearer route-one-secondary", 10),
            ],
            "server_error",
            "Our servers are currently overloaded. Please try again later.",
        )
        .await;
    let (same_route_proxy_base, same_route_proxy_connects, same_route_proxy_handle) =
        spawn_test_http_connect_proxy(&same_route_base).await;
    let (alternate_proxy_base, alternate_proxy_connects, alternate_proxy_handle) =
        spawn_test_http_connect_proxy(&same_route_base).await;
    let (alternate_base, alternate_attempts, alternate_handle) =
        spawn_pool_retry_upstream(&[]).await;
    let base_state =
        test_state_with_openai_base(Url::parse(&same_route_base).expect("valid upstream base url"))
            .await;
    let state =
        clone_state_with_fallback_proxy_429_retry_delay_override(&base_state, Some(Duration::ZERO));
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec![same_route_proxy_base.clone(), alternate_proxy_base.clone()],
            ..ForwardProxySettings::default()
        },
    )
    .await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Server Error Primary",
        "route-one-primary",
        None,
        None,
        Some(same_route_base.as_str()),
    )
    .await;
    let bound_proxy_keys = [
        same_route_proxy_base.as_str(),
        alternate_proxy_base.as_str(),
    ]
    .into_iter()
    .map(|proxy_url| normalize_single_proxy_key(proxy_url).expect("normalize proxy key"))
    .collect::<Vec<_>>();
    sqlx::query("UPDATE pool_upstream_accounts SET bound_proxy_keys_json = ?1 WHERE id = ?2")
        .bind(serde_json::to_string(&bound_proxy_keys).expect("serialize bound proxy keys"))
        .bind(primary_id)
        .execute(&state.pool)
        .await
        .expect("bind two forward proxy nodes to primary account");
    insert_test_pool_api_key_account_with_options(
        &state,
        "Server Error Secondary",
        "route-one-secondary",
        None,
        None,
        Some(same_route_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Server Error Alternate",
        "route-two",
        None,
        None,
        Some(alternate_base.as_str()),
    )
    .await;
    record_pool_route_success(
        &state.pool,
        primary_id,
        Utc::now(),
        Some("sticky-server-error-exhausted"),
        None,
    )
    .await
    .expect("seed server_error sticky route");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-server-error-exhausted"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read exhausted server_error overload response");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 exhausted response body");
    assert!(body_text.contains("upstream_response_failed"));
    assert!(body_text.contains("server_error"));

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    let invocation = sqlx::query_as::<_, InvocationFailureRow>(
        r#"
        SELECT status, failure_kind, failure_class, error_message
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load exhausted server_error overload invocation");
    assert_eq!(invocation.status, "failed");
    assert_eq!(
        invocation.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    );
    assert_eq!(invocation.failure_class.as_deref(), Some("service_failure"));
    assert!(
        invocation
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("server_error"))
    );
    let model_route = sqlx::query_as::<_, ModelRouteFailureRow>(
        r#"
        SELECT state, last_failure_kind, cooldown_until
        FROM pool_upstream_account_model_routes
        WHERE account_id = ?1 AND model = ?2
        "#,
    )
    .bind(primary_id)
    .bind("gpt-5.4")
    .fetch_one(&state.pool)
    .await
    .expect("load exhausted server_error overload model route");
    assert_eq!(model_route.state, "degraded");
    assert_eq!(
        model_route.last_failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    );
    assert!(model_route.cooldown_until.is_none());
    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT upstream_route_key, distinct_account_index, same_account_retry_index,
               proxy_binding_key_snapshot, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load exhausted server_error overload attempt rows");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1]
    );
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.same_account_retry_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(attempt_rows.iter().all(|row| {
        row.upstream_route_key
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }));
    assert!(attempt_rows.iter().all(|row| {
        row.proxy_binding_key_snapshot
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }));
    assert!(
        attempt_rows
            .windows(2)
            .all(|rows| rows[0].upstream_route_key == rows[1].upstream_route_key)
    );
    assert!(
        attempt_rows.windows(2).all(|rows| {
            rows[0].proxy_binding_key_snapshot == rows[1].proxy_binding_key_snapshot
        })
    );
    assert!(
        attempt_rows
            .iter()
            .all(|row| row.status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE)
    );

    let same_route_attempts = same_route_attempts
        .lock()
        .expect("lock exhausted server_error overload attempts");
    let same_route_proxy_connects = same_route_proxy_connects.load(Ordering::SeqCst);
    let alternate_proxy_connects = alternate_proxy_connects.load(Ordering::SeqCst);
    assert_eq!(same_route_proxy_connects + alternate_proxy_connects, 4);
    assert!(
        same_route_proxy_connects == 4 || alternate_proxy_connects == 4,
        "same-account retries must stay on one forward-proxy channel: primary={same_route_proxy_connects}, alternate={alternate_proxy_connects}"
    );
    assert!(
        same_route_attempts
            .get("Bearer route-one-secondary")
            .is_none()
    );
    drop(same_route_attempts);

    let alternate_attempts = alternate_attempts
        .lock()
        .expect("lock alternate server_error overload attempts");
    assert!(alternate_attempts.get("Bearer route-two").is_none());
    drop(alternate_attempts);

    same_route_handle.abort();
    same_route_proxy_handle.abort();
    alternate_proxy_handle.abort();
    alternate_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_mixed_overload_failures_share_same_account_budget() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_route_key: Option<String>,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        proxy_binding_key_snapshot: Option<String>,
        status: String,
    }

    let (same_route_base, same_route_attempts, same_route_handle) =
        spawn_pool_metadata_prefixed_response_failed_sequence_upstream(&[(
            "Bearer mixed-overload",
            &[
                (
                    "server_error",
                    "Our servers are currently overloaded. Please try again later.",
                ),
                (
                    UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED,
                    "Our servers are currently overloaded. Please try again later.",
                ),
                (
                    UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED,
                    "Our servers are currently overloaded. Please try again later.",
                ),
                (
                    UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED,
                    "Our servers are currently overloaded. Please try again later.",
                ),
            ],
        )])
        .await;
    let (alternate_base, alternate_attempts, alternate_handle) =
        spawn_pool_retry_upstream(&[]).await;
    let base_state =
        test_state_with_openai_base(Url::parse(&same_route_base).expect("valid mixed base url"))
            .await;
    let state =
        clone_state_with_fallback_proxy_429_retry_delay_override(&base_state, Some(Duration::ZERO));
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Mixed Overload Primary",
        "mixed-overload",
        None,
        None,
        Some(same_route_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Mixed Overload Alternate",
        "mixed-overload-alternate",
        None,
        None,
        Some(alternate_base.as_str()),
    )
    .await;
    record_pool_route_success(
        &state.pool,
        primary_id,
        Utc::now(),
        Some("sticky-mixed-overload"),
        None,
    )
    .await
    .expect("seed mixed overload sticky route");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-mixed-overload"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read mixed overload response");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 mixed overload response");
    assert!(body_text.contains("upstream_response_failed"));
    assert!(body_text.contains("server_is_overloaded"));

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT upstream_route_key, distinct_account_index, same_account_retry_index,
               proxy_binding_key_snapshot, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load mixed overload attempt rows");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1]
    );
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.same_account_retry_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(attempt_rows.iter().all(|row| {
        row.upstream_route_key
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }));
    assert!(attempt_rows.iter().all(|row| {
        row.proxy_binding_key_snapshot
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }));
    assert!(
        attempt_rows
            .windows(2)
            .all(|rows| rows[0].upstream_route_key == rows[1].upstream_route_key)
    );
    assert!(
        attempt_rows.windows(2).all(|rows| {
            rows[0].proxy_binding_key_snapshot == rows[1].proxy_binding_key_snapshot
        })
    );
    assert!(
        attempt_rows
            .iter()
            .all(|row| row.status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE)
    );

    let same_route_attempts = same_route_attempts
        .lock()
        .expect("lock mixed overload attempts");
    assert_eq!(
        same_route_attempts.get("Bearer mixed-overload").copied(),
        Some(4)
    );
    drop(same_route_attempts);

    let alternate_attempts = alternate_attempts
        .lock()
        .expect("lock mixed overload alternate attempts");
    assert!(
        alternate_attempts
            .get("Bearer mixed-overload-alternate")
            .is_none()
    );
    drop(alternate_attempts);

    same_route_handle.abort();
    alternate_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_server_error_overload_caps_mixed_413_and_429_attempts() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        distinct_account_index: i64,
        same_account_retry_index: i64,
        http_status: Option<i64>,
        status: String,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_responses_retry_sequence_upstream(&[(
            "Bearer mixed-413-and-429-budget",
            &[
                PoolResponsesRetrySequenceStep::CanonicalOverload,
                PoolResponsesRetrySequenceStep::PayloadTooLarge,
                PoolResponsesRetrySequenceStep::RateLimited,
                PoolResponsesRetrySequenceStep::RateLimited,
            ],
        )])
        .await;
    let base_state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let state =
        clone_state_with_pool_group_429_retry_delay_override(&base_state, Some(Duration::ZERO));
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Mixed 413 and 429 Budget",
        "mixed-413-and-429-budget",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;
    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_upstream_429_retry_enabled = 1, policy_upstream_429_max_retries = 3 WHERE id = ?1",
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("enable mixed 413 and 429 account retry settings");
    let effective_rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load mixed 413 and 429 account retry settings");
    assert!(effective_rule.upstream_429_retry_enabled);
    assert_eq!(effective_rule.upstream_429_max_retries, 3);

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-mixed-413-and-429-budget"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let _body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read mixed 413 and 429 budget failure body");
    let attempts = attempts
        .lock()
        .expect("lock mixed 413 and 429 budget attempts");
    assert_eq!(
        attempts.get("Bearer mixed-413-and-429-budget").copied(),
        Some(4)
    );
    drop(attempts);

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT distinct_account_index, same_account_retry_index,
               http_status, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load mixed 413 and 429 budget attempts");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1]
    );
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.same_account_retry_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(attempt_rows[0].http_status, Some(200));
    assert_eq!(attempt_rows[1].http_status, Some(413));
    assert_eq!(
        attempt_rows
            .iter()
            .skip(2)
            .map(|row| row.http_status)
            .collect::<Vec<_>>(),
        vec![Some(429), Some(429)]
    );
    assert!(
        attempt_rows
            .iter()
            .all(|row| row.status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_server_error_overload_caps_preceding_429_attempts() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_responses_retry_sequence_upstream(&[(
            "Bearer preceding-429-budget",
            &[
                PoolResponsesRetrySequenceStep::RateLimited,
                PoolResponsesRetrySequenceStep::RateLimited,
                PoolResponsesRetrySequenceStep::RateLimited,
                PoolResponsesRetrySequenceStep::CanonicalOverload,
            ],
        )])
        .await;
    let base_state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let state =
        clone_state_with_pool_group_429_retry_delay_override(&base_state, Some(Duration::ZERO));
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Preceding 429 Budget",
        "preceding-429-budget",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;
    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_upstream_429_retry_enabled = 1, policy_upstream_429_max_retries = 5 WHERE id = ?1",
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("enable preceding 429 account retry settings");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-preceding-429-budget"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read preceding 429 budget failure body");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 preceding 429 budget body");
    assert!(body_text.contains("server_error"));

    let attempts = attempts.lock().expect("lock preceding 429 budget attempts");
    assert_eq!(
        attempts.get("Bearer preceding-429-budget").copied(),
        Some(4)
    );
    drop(attempts);

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    let attempt_statuses = sqlx::query_scalar::<_, String>(
        "SELECT status FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load preceding 429 budget attempt statuses");
    assert_eq!(attempt_statuses.len(), 4);
    assert!(
        attempt_statuses
            .iter()
            .all(|status| status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_legacy_overload_keeps_group_429_budget() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_responses_retry_sequence_upstream(&[(
            "Bearer legacy-after-429-budget",
            &[
                PoolResponsesRetrySequenceStep::RateLimited,
                PoolResponsesRetrySequenceStep::RateLimited,
                PoolResponsesRetrySequenceStep::RateLimited,
                PoolResponsesRetrySequenceStep::RateLimited,
                PoolResponsesRetrySequenceStep::LegacyOverload,
            ],
        )])
        .await;
    let base_state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let state =
        clone_state_with_pool_group_429_retry_delay_override(&base_state, Some(Duration::ZERO));
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Legacy After 429 Budget",
        "legacy-after-429-budget",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;
    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_upstream_429_retry_enabled = 1, policy_upstream_429_max_retries = 5 WHERE id = ?1",
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("enable legacy after 429 account retry settings");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-legacy-after-429-budget"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read legacy after 429 budget body");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 legacy after 429 budget body");
    assert!(body_text.contains("server_is_overloaded"));

    let attempts = attempts
        .lock()
        .expect("lock legacy after 429 budget attempts");
    assert_eq!(
        attempts.get("Bearer legacy-after-429-budget").copied(),
        Some(5)
    );
    drop(attempts);

    upstream_handle.abort();
}

#[test]
fn pool_responses_classify_account_concurrency_rate_limit_as_retryable_overload() {
    let payload = br#"event: response.failed
data: {"type":"response.failed","response":{"id":"resp_concurrency_retry","model":"gpt-5.6-sol","status":"failed","error":{"code":"rate_limit_exceeded","message":"Concurrency limit exceeded for account, please retry later"}}}

"#;

    let decision = classify_pool_initial_responses_sse_event(StatusCode::OK, payload);
    match decision {
        PoolInitialResponsesSseEventDecision::RetrySameAccount {
            retry_policy,
            upstream_error_code,
            upstream_error_message,
            ..
        } => {
            assert_eq!(
                retry_policy,
                PoolResponsesOverloadRetryPolicy::LegacyFailover
            );
            assert_eq!(upstream_error_code.as_deref(), Some("rate_limit_exceeded"));
            assert_eq!(
                upstream_error_message.as_deref(),
                Some("Concurrency limit exceeded for account, please retry later")
            );
        }
        PoolInitialResponsesSseEventDecision::ContinueMetadata
        | PoolInitialResponsesSseEventDecision::Forward => {
            panic!("account concurrency limit should enter the existing same-account retry path")
        }
    }

    let generic_rate_limit_payload = br#"event: response.failed
data: {"type":"response.failed","response":{"id":"resp_generic_rate_limit","model":"gpt-5.6-sol","status":"failed","error":{"code":"rate_limit_exceeded","message":"Requests per minute limit exceeded, please retry later"}}}

"#;
    assert!(matches!(
        classify_pool_initial_responses_sse_event(StatusCode::OK, generic_rate_limit_payload),
        PoolInitialResponsesSseEventDecision::Forward
    ));

    let reached_payload = br#"event: response.failed
data: {"type":"response.failed","response":{"id":"resp_concurrency_reached","model":"gpt-5.6-sol","status":"failed","error":{"code":"rate_limit_exceeded","message":"Concurrent request limit reached, please retry later"}}}

"#;
    assert!(matches!(
        classify_pool_initial_responses_sse_event(StatusCode::OK, reached_payload),
        PoolInitialResponsesSseEventDecision::RetrySameAccount { .. }
    ));
}

#[test]
fn pool_responses_compact_gate_classifies_account_concurrency_rate_limit() {
    let payload = Bytes::from_static(
        br#"{"error":{"code":"rate_limit_exceeded","message":"Concurrency limit exceeded for account, please retry later"}}"#,
    );
    let headers = HeaderMap::new();
    let outcome = gate_pool_initial_compact_response(StatusCode::OK, &headers, Some(&payload));
    match outcome {
        Some(PoolInitialResponseGateOutcome::RetrySameAccount {
            retry_policy,
            upstream_error_code,
            upstream_error_message,
            ..
        }) => {
            assert_eq!(
                retry_policy,
                PoolResponsesOverloadRetryPolicy::LegacyFailover
            );
            assert_eq!(upstream_error_code.as_deref(), Some("rate_limit_exceeded"));
            assert_eq!(
                upstream_error_message.as_deref(),
                Some("Concurrency limit exceeded for account, please retry later")
            );
        }
        Some(PoolInitialResponseGateOutcome::Forward { .. }) | None => {
            panic!("compact account concurrency limit should enter the same-account retry path")
        }
    }

    let generic_payload = Bytes::from_static(
        br#"{"error":{"code":"rate_limit_exceeded","message":"Requests per minute limit exceeded, please retry later"}}"#,
    );
    assert!(
        gate_pool_initial_compact_response(StatusCode::OK, &headers, Some(&generic_payload))
            .is_none()
    );
}

#[test]
fn pool_responses_compact_gate_classifies_server_error_overload_as_same_account_only() {
    let payload = Bytes::from_static(
        br#"{"error":{"code":"server_error","message":"Our servers are currently overloaded! Please try again later."}}"#,
    );
    let headers = HeaderMap::new();
    let outcome = gate_pool_initial_compact_response(StatusCode::OK, &headers, Some(&payload));
    match outcome {
        Some(PoolInitialResponseGateOutcome::RetrySameAccount { retry_policy, .. }) => {
            assert_eq!(
                retry_policy,
                PoolResponsesOverloadRetryPolicy::SameAccountOnly
            );
        }
        Some(PoolInitialResponseGateOutcome::Forward { .. }) | None => {
            panic!("server_error canonical overload should enter same-account retry path")
        }
    }
}

#[test]
fn pool_responses_route_failure_keeps_generic_rate_limit_out_of_overload_class() {
    assert!(route_http_failure_is_retryable_responses_overload(
        StatusCode::OK,
        "[upstream_response_failed] rate_limit_exceeded: Concurrency limit exceeded for account, please retry later",
    ));
    assert!(!route_http_failure_is_retryable_responses_overload(
        StatusCode::OK,
        "[upstream_response_failed] rate_limit_exceeded: Requests per minute limit exceeded, please retry later",
    ));
    assert!(!route_http_failure_is_retryable_responses_overload(
        StatusCode::TOO_MANY_REQUESTS,
        "[upstream_response_failed] rate_limit_exceeded: Concurrency limit exceeded for account, please retry later",
    ));
    assert!(!route_http_failure_is_retryable_responses_overload(
        StatusCode::OK,
        "[upstream_response_failed] rate_limit_exceeded: Concurrent request limit is active, please retry later",
    ));
    assert!(route_http_failure_is_retryable_responses_overload(
        StatusCode::OK,
        "[upstream_response_failed] rate_limit_exceeded: Concurrent request limit reached, please retry later",
    ));
}

#[tokio::test]
async fn pool_openai_v1_responses_retries_same_account_on_account_concurrency_limit() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        distinct_account_index: i64,
        status: String,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_metadata_prefixed_response_failed_retry_upstream_with_error(
            &[("Bearer route-one", 3)],
            UPSTREAM_ERROR_CODE_RATE_LIMIT_EXCEEDED,
            "Concurrency limit exceeded for account, please retry later",
        )
        .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Concurrency Retry Route",
        "route-one",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.6-sol","stream":true,"input":"hello","stickyKey":"sticky-concurrency-retry-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read concurrency retry response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode concurrency retry success body");
    assert_eq!(response_payload["ok"].as_bool(), Some(true));

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT distinct_account_index, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load concurrency retry attempt rows");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1]
    );
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.status.as_str())
            .collect::<Vec<_>>(),
        vec![
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ]
    );

    let account_action: (Option<String>, Option<String>, Option<String>) = sqlx::query_as(
        r#"
        SELECT last_action, last_action_reason_code, cooldown_until
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load concurrency retry account state");
    assert_eq!(account_action.0.as_deref(), Some("route_recovered"));
    assert!(account_action.1.is_none());
    assert!(account_action.2.is_none());

    let attempts = attempts.lock().expect("lock concurrency retry attempts");
    assert_eq!(attempts.get("Bearer route-one").copied(), Some(4));
    drop(attempts);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_retries_after_metadata_prefix_exceeds_preview_limit() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_large_metadata_prefixed_response_failed_retry_upstream(&[(
            "Bearer route-one",
            1,
        )])
        .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Retry Route One Large Metadata",
        "route-one",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-overloaded-large-metadata-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read retryable overloaded large-metadata response");
    let response_payload: Value = serde_json::from_slice(&body)
        .expect("decode retryable overloaded large-metadata success body");
    assert_eq!(response_payload["ok"].as_bool(), Some(true));
    assert_eq!(
        response_payload["authorization"].as_str(),
        Some("Bearer route-one"),
    );
    let body_text =
        String::from_utf8(body.to_vec()).expect("utf8 retryable overloaded large-metadata body");
    assert!(!body_text.contains("response.failed"));
    assert!(!body_text.contains("server_is_overloaded"));

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 2).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT distinct_account_index, same_account_retry_index, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load overloaded large-metadata retry attempt rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);

    let attempts = attempts
        .lock()
        .expect("lock retryable overloaded large-metadata attempts");
    assert_eq!(attempts.get("Bearer route-one").copied(), Some(2));
    drop(attempts);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_overload_prefers_same_route_before_alternate_route() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        distinct_account_index: i64,
    }

    let (same_route_base, same_route_attempts, same_route_handle) =
        spawn_pool_metadata_prefixed_response_failed_retry_upstream(&[(
            "Bearer route-one-primary",
            10,
        )])
        .await;
    let (alternate_base, alternate_attempts, alternate_handle) =
        spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&same_route_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Same Route",
        "route-one-primary",
        None,
        None,
        Some(same_route_base.as_str()),
    )
    .await;
    let secondary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary Same Route",
        "route-one-secondary",
        None,
        None,
        Some(same_route_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Alternate Route",
        "route-two",
        None,
        None,
        Some(alternate_base.as_str()),
    )
    .await;
    record_pool_route_success(
        &state.pool,
        primary_id,
        Utc::now(),
        Some("sticky-overload-same-route-first"),
        None,
    )
    .await
    .expect("seed sticky route");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-overload-same-route-first"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read same-route overload response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode same-route overload body");
    assert_eq!(
        payload["authorization"].as_str(),
        Some("Bearer route-one-secondary"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 5).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT distinct_account_index
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load same-route overload attempt rows");
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1, 2]
    );
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-overload-same-route-first").await,
        Some(secondary_id),
        "successful same-route fallback should own the sticky binding",
    );
    let same_route_attempts = same_route_attempts
        .lock()
        .expect("lock same-route overload attempts");
    assert_eq!(
        same_route_attempts.get("Bearer route-one-primary").copied(),
        Some(4)
    );
    assert_eq!(
        same_route_attempts
            .get("Bearer route-one-secondary")
            .copied(),
        Some(1)
    );
    drop(same_route_attempts);

    let alternate_attempts = alternate_attempts
        .lock()
        .expect("lock alternate overload attempts");
    assert!(
        alternate_attempts.get("Bearer route-two").is_none(),
        "alternate route should remain unused while a same-route account can recover",
    );
    drop(alternate_attempts);

    same_route_handle.abort();
    alternate_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_overload_falls_back_to_alternate_route_after_same_route_exhaustion()
 {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        distinct_account_index: i64,
    }

    let (same_route_base, same_route_attempts, same_route_handle) =
        spawn_pool_metadata_prefixed_response_failed_retry_upstream(&[
            ("Bearer route-one-primary", 10),
            ("Bearer route-one-secondary", 10),
        ])
        .await;
    let (alternate_base, alternate_attempts, alternate_handle) =
        spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&same_route_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Same Route Exhausted",
        "route-one-primary",
        None,
        None,
        Some(same_route_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary Same Route Exhausted",
        "route-one-secondary",
        None,
        None,
        Some(same_route_base.as_str()),
    )
    .await;
    let alternate_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Alternate Route Recovery",
        "route-two",
        None,
        None,
        Some(alternate_base.as_str()),
    )
    .await;
    record_pool_route_success(
        &state.pool,
        primary_id,
        Utc::now(),
        Some("sticky-overload-alternate-route"),
        None,
    )
    .await
    .expect("seed sticky route");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4","stream":true,"input":"hello","stickyKey":"sticky-overload-alternate-route"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read alternate-route overload response body");
    let payload: Value =
        serde_json::from_slice(&body).expect("decode alternate-route overload body");
    assert_eq!(payload["authorization"].as_str(), Some("Bearer route-two"));

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 8).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT distinct_account_index
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load alternate-route overload attempt rows");
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1, 2, 2, 2, 3]
    );
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-overload-alternate-route").await,
        Some(alternate_id),
        "successful alternate-route fallback should replace the sticky binding",
    );

    let same_route_attempts = same_route_attempts
        .lock()
        .expect("lock exhausted same-route overload attempts");
    assert_eq!(
        same_route_attempts.get("Bearer route-one-primary").copied(),
        Some(4)
    );
    assert_eq!(
        same_route_attempts
            .get("Bearer route-one-secondary")
            .copied(),
        Some(3)
    );
    drop(same_route_attempts);

    let alternate_attempts = alternate_attempts
        .lock()
        .expect("lock alternate-route overload attempts");
    assert_eq!(alternate_attempts.get("Bearer route-two").copied(), Some(1));
    drop(alternate_attempts);

    same_route_handle.abort();
    alternate_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_compact_retries_server_error_overload_same_account_before_forwarding() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_route_key: Option<String>,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        proxy_binding_key_snapshot: Option<String>,
        status: String,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_compact_response_failed_retry_upstream_with_error(
            &[("Bearer compact-server-error", 3)],
            "server_error",
            "Our servers are currently overloaded. Please try again later.",
        )
        .await;
    let base_state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid compact base url"))
            .await;
    let state =
        clone_state_with_fallback_proxy_429_retry_delay_override(&base_state, Some(Duration::ZERO));
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Compact Server Error Retry Route",
        "compact-server-error",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses/compact".parse().expect("valid compact uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4-mini","input":"hello","stickyKey":"sticky-compact-server-error-retry"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact server_error overload retry response");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode compact server_error overload success body");
    assert_eq!(response_payload["ok"].as_bool(), Some(true));
    assert_eq!(
        response_payload["authorization"].as_str(),
        Some("Bearer compact-server-error"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT upstream_route_key, distinct_account_index, same_account_retry_index,
               proxy_binding_key_snapshot, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load compact server_error overload retry attempt rows");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1]
    );
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.same_account_retry_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(attempt_rows.iter().all(|row| {
        row.upstream_route_key
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }));
    assert!(attempt_rows.iter().all(|row| {
        row.proxy_binding_key_snapshot
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    }));
    assert!(
        attempt_rows
            .windows(2)
            .all(|rows| rows[0].upstream_route_key == rows[1].upstream_route_key)
    );
    assert!(
        attempt_rows.windows(2).all(|rows| {
            rows[0].proxy_binding_key_snapshot == rows[1].proxy_binding_key_snapshot
        })
    );
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.status.as_str())
            .collect::<Vec<_>>(),
        vec![
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ]
    );

    let attempts = attempts
        .lock()
        .expect("lock compact server_error overload retry attempts");
    assert_eq!(
        attempts.get("Bearer compact-server-error").copied(),
        Some(4)
    );
    drop(attempts);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_compact_overload_falls_back_to_alternate_route_before_body_forward() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        distinct_account_index: i64,
    }

    let (same_route_base, same_route_attempts, same_route_handle) =
        spawn_pool_compact_overloaded_retry_upstream(&[
            ("Bearer compact-primary", 10),
            ("Bearer compact-secondary", 10),
        ])
        .await;
    let (alternate_base, alternate_attempts, alternate_handle) =
        spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&same_route_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Compact Primary",
        "compact-primary",
        None,
        None,
        Some(same_route_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Compact Secondary",
        "compact-secondary",
        None,
        None,
        Some(same_route_base.as_str()),
    )
    .await;
    let alternate_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Compact Alternate",
        "compact-route-two",
        None,
        None,
        Some(alternate_base.as_str()),
    )
    .await;
    record_pool_route_success(
        &state.pool,
        primary_id,
        Utc::now(),
        Some("sticky-compact-overload"),
        None,
    )
    .await
    .expect("seed compact sticky route");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses/compact".parse().expect("valid compact uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.4-mini","input":"hello","stickyKey":"sticky-compact-overload"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact overload response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode compact overload body");
    assert_eq!(
        payload["authorization"].as_str(),
        Some("Bearer compact-route-two"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 8).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT distinct_account_index
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load compact overload attempt rows");
    assert_eq!(
        attempt_rows
            .iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1, 2, 2, 2, 3]
    );
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-compact-overload").await,
        Some(alternate_id),
    );

    let same_route_attempts = same_route_attempts
        .lock()
        .expect("lock compact same-route overload attempts");
    assert_eq!(
        same_route_attempts.get("Bearer compact-primary").copied(),
        Some(4)
    );
    assert_eq!(
        same_route_attempts.get("Bearer compact-secondary").copied(),
        Some(3)
    );
    drop(same_route_attempts);

    let alternate_attempts = alternate_attempts
        .lock()
        .expect("lock compact alternate overload attempts");
    assert_eq!(
        alternate_attempts.get("Bearer compact-route-two").copied(),
        Some(1)
    );
    drop(alternate_attempts);

    same_route_handle.abort();
    alternate_handle.abort();
}

#[tokio::test]
async fn gate_pool_initial_response_stream_keeps_non_overload_response_failed_on_original_stream() {
    let payload = [
        "event: response.created\n",
        r#"data: {"type":"response.created","response":{"id":"resp_gate_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "\n\n",
        "event: response.failed\n",
        r#"data: {"type":"response.failed","response":{"id":"resp_gate_test","model":"gpt-5.4","status":"failed","error":{"code":"server_error","message":"processing failed"}}}"#,
        "\n\n",
    ]
    .concat();
    let response = ProxyUpstreamResponseBody::Axum(
        Response::builder()
            .status(StatusCode::OK)
            .header(http_header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from(payload))
            .expect("build gate test response"),
    );

    let outcome =
        gate_pool_initial_response_stream(response, None, Duration::from_secs(1), Instant::now())
            .await
            .expect("gate initial response stream");

    let PoolInitialResponseGateOutcome::Forward {
        response,
        prefetched_bytes,
        ..
    } = outcome
    else {
        panic!("non-overload response.failed should stay on the original stream");
    };

    let mut full_body = prefetched_bytes
        .expect("forwarded stream should keep prefetched metadata window")
        .to_vec();
    full_body.extend_from_slice(
        response
            .into_bytes()
            .await
            .expect("read rebuilt gate response body")
            .as_ref(),
    );
    let body_text = String::from_utf8(full_body).expect("utf8 gate body");
    assert!(body_text.contains("response.created"));
    assert!(body_text.contains("server_error"));
    assert!(!body_text.contains("server_is_overloaded"));
}

#[tokio::test]
async fn gate_pool_initial_response_stream_keeps_canonical_server_error_after_forward_event() {
    let payload = [
        "event: response.created\n",
        r#"data: {"type":"response.created","response":{"id":"resp_gate_canonical_late_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "\n\n",
        "event: response.output_text.delta\n",
        r#"data: {"type":"response.output_text.delta","delta":"answer"}"#,
        "\n\n",
        "event: response.failed\n",
        r#"data: {"type":"response.failed","response":{"id":"resp_gate_canonical_late_test","model":"gpt-5.4","status":"failed","error":{"code":"server_error","message":"Our servers are currently overloaded. Please try again later."}}}"#,
        "\n\n",
    ]
    .concat();
    let response = ProxyUpstreamResponseBody::Axum(
        Response::builder()
            .status(StatusCode::OK)
            .header(http_header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from(payload))
            .expect("build canonical late gate response"),
    );

    let outcome =
        gate_pool_initial_response_stream(response, None, Duration::from_secs(1), Instant::now())
            .await
            .expect("gate canonical late response stream");

    let PoolInitialResponseGateOutcome::Forward {
        response,
        prefetched_bytes,
        ..
    } = outcome
    else {
        panic!("canonical response.failed after a business event must stay on the stream");
    };

    let mut full_body = prefetched_bytes
        .expect("canonical late stream should keep prefetched event")
        .to_vec();
    full_body.extend_from_slice(
        response
            .into_bytes()
            .await
            .expect("read canonical late gate response")
            .as_ref(),
    );
    let body_text = String::from_utf8(full_body).expect("utf8 canonical late gate body");
    assert!(body_text.contains("response.output_text.delta"));
    assert!(body_text.contains("Our servers are currently overloaded"));
}

#[tokio::test]
async fn gate_pool_initial_response_stream_preserves_first_forward_event_boundary() {
    let created = [
        "event: response.created\n",
        r#"data: {"type":"response.created","response":{"id":"resp_gate_boundary_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "\n\n",
    ]
    .concat();
    let completed = [
        "event: response.completed\n",
        r#"data: {"type":"response.completed","response":{"id":"resp_gate_boundary_test","model":"gpt-5.4","status":"completed"},"usage":{"input_tokens":12,"output_tokens":34,"total_tokens":46}}"#,
        "\n\n",
    ]
    .concat();
    let response = ProxyUpstreamResponseBody::Axum(
        Response::builder()
            .status(StatusCode::OK)
            .header(http_header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from_stream(stream::iter(vec![
                Ok::<Bytes, Infallible>(Bytes::from(created)),
                Ok::<Bytes, Infallible>(Bytes::from(completed)),
            ])))
            .expect("build gate boundary response"),
    );

    let outcome =
        gate_pool_initial_response_stream(response, None, Duration::from_secs(1), Instant::now())
            .await
            .expect("gate boundary response stream");

    let PoolInitialResponseGateOutcome::Forward {
        response,
        prefetched_bytes,
        ..
    } = outcome
    else {
        panic!("response.completed should stay on the original stream");
    };

    let prefetched_text = String::from_utf8(
        prefetched_bytes
            .expect("metadata prefix should remain prefetched")
            .to_vec(),
    )
    .expect("utf8 gate prefetched metadata");
    assert!(prefetched_text.contains("response.created"));
    assert!(!prefetched_text.contains("response.completed"));

    let remaining_text = String::from_utf8(
        response
            .into_bytes()
            .await
            .expect("read rebuilt boundary stream")
            .to_vec(),
    )
    .expect("utf8 gate rebuilt response");
    assert!(remaining_text.contains("response.completed"));
    assert!(!remaining_text.contains("response.created"));
}

#[tokio::test]
async fn gate_pool_initial_response_stream_preserves_replayed_chunk_timestamp() {
    let payload = [
        "event: response.created\n",
        r#"data: {"type":"response.created","response":{"id":"resp_gate_timestamp_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "\n\n",
        "event: response.output_text.delta\n",
        r#"data: {"type":"response.output_text.delta","delta":"answer"}"#,
        "\n\n",
    ]
    .concat();
    let response = ProxyUpstreamResponseBody::Axum(
        Response::builder()
            .status(StatusCode::OK)
            .header(http_header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from(payload.clone()))
            .expect("build gate timestamp response"),
    );
    let received_at = Instant::now() - Duration::from_millis(25);

    let outcome = gate_pool_initial_response_stream_with_timestamp(
        response,
        Some(Bytes::from(payload)),
        Some(received_at),
        Duration::from_secs(1),
        Instant::now(),
    )
    .await
    .expect("gate timestamp response stream");

    let PoolInitialResponseGateOutcome::Forward {
        prefetched_bytes,
        prefetched_bytes_received_at,
        replayed_bytes_received_at,
        ..
    } = outcome
    else {
        panic!("model delta should remain on the original stream");
    };
    assert!(prefetched_bytes.is_some());
    assert_eq!(prefetched_bytes_received_at, Some(received_at));
    assert_eq!(replayed_bytes_received_at, Some(received_at));
}

#[tokio::test]
async fn gate_pool_initial_response_stream_retries_overload_after_metadata_prefix_exceeds_preview_limit()
 {
    let oversized_metadata = less_compressible_test_string(RAW_RESPONSE_PREVIEW_LIMIT + 8 * 1024);
    let created = format!(
        "event: response.created\n\
         data: {}\n\n",
        serde_json::to_string(&json!({
            "type": "response.created",
            "response": {
                "id": "resp_gate_large_test",
                "model": "gpt-5.4",
                "status": "in_progress",
                "metadata": oversized_metadata,
            },
        }))
        .expect("serialize oversized metadata gate payload")
    );
    let failed = [
        "event: response.failed\n",
        r#"data: {"type":"response.failed","response":{"id":"resp_gate_large_test","model":"gpt-5.4","status":"failed","error":{"code":"server_is_overloaded","message":"Our servers are currently overloaded. Please try again later."}}}"#,
        "\n\n",
    ]
    .concat();
    let response = ProxyUpstreamResponseBody::Axum(
        Response::builder()
            .status(StatusCode::OK)
            .header(http_header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from_stream(stream::iter(vec![
                Ok::<Bytes, Infallible>(Bytes::from(created)),
                Ok::<Bytes, Infallible>(Bytes::from(failed)),
            ])))
            .expect("build oversized metadata gate response"),
    );

    let outcome =
        gate_pool_initial_response_stream(response, None, Duration::from_secs(1), Instant::now())
            .await
            .expect("gate oversized metadata initial response stream");

    let PoolInitialResponseGateOutcome::RetrySameAccount { .. } = outcome else {
        panic!(
            "oversized metadata-only prefix should still keep overload retryable before forwarding"
        );
    };
}

mod oauth_route_cases;

#[tokio::test]
async fn failover_preserves_assigned_account_when_sticky_owner_is_preflight_blocked() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid url")).await;
    let sticky_account = insert_test_pool_oauth_account(
        &state,
        "Sticky Missing Binding",
        "sticky-preflight-missing",
    )
    .await;
    set_test_account_group_name(
        &state.pool,
        sticky_account,
        Some("sticky-preflight-missing"),
    )
    .await;
    let _fallback_account =
        insert_test_pool_oauth_account(&state, "Fallback Healthy Account", "fallback-healthy")
            .await;
    let now_iso = format_utc_iso(Utc::now());
    let lock_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, system_key, protected, allow_cut_out, allow_cut_in,
            priority_tier, fast_mode_rewrite_mode, concurrency_limit, upstream_429_retry_enabled,
            upstream_429_max_retries, available_models_json, created_at, updated_at
        ) VALUES (?1, ?2, 0, 0, 1, 'normal', 'keep_original', 0, 0, 0, '[]', ?3, ?3)
        RETURNING id
        "#,
    )
    .bind("sticky-preflight-lock")
    .bind(None::<String>)
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert sticky lock tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(sticky_account)
    .bind(lock_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach sticky lock tag");
    upsert_sticky_route(
        &state.pool,
        "sticky-preflight-blocked",
        sticky_account,
        &now_iso,
    )
    .await
    .expect("seed sticky route");

    let err = send_pool_request_with_failover(
        state.clone(),
        700701,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::Memory(Bytes::from_static(
            br#"{"input":"hello"}"#,
        ))),
        Duration::from_secs(5),
        Some(PoolUpstreamAttemptTraceContext {
            invoke_id: "sticky-preflight-blocked-invoke".to_string(),
            occurred_at: shanghai_now_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some("sticky-preflight-blocked".to_string()),
            requester_ip: None,
            upstream_base_url_host: None,
            request_model: None,
        }),
        None,
        Some("sticky-preflight-blocked"),
        None,
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect_err("sticky preflight block should fail");

    assert_eq!(
        err.failure_kind,
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED
    );
    assert_eq!(
        err.account.as_ref().map(|account| account.account_id),
        Some(sticky_account)
    );
    assert!(err.message.contains(
        "upstream account group \"sticky-preflight-missing\" has no bound forward proxy nodes"
    ));

    wait_for_pool_attempt_row_count(&state.pool, 0).await;
    let attempt_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count sticky preflight blocked attempts");
    assert_eq!(attempt_count, 0);
}
