#[tokio::test]
async fn capture_target_pool_route_timeout_prefers_real_alternate_group_proxy_error() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        attempt_index: i64,
        status: String,
        failure_kind: Option<String>,
        error_message: Option<String>,
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
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(broken_same_route_id)
        .bind("broken-shared-route-group")
        .execute(&state.pool)
        .await
        .expect("mark broken same-route account");
    let broken_alternate_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Broken Alternate Route",
        "route-broken-alt-invalid-group",
        None,
        None,
        Some("https://broken-alt.example.com/backend-api/codex"),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(broken_alternate_id)
        .bind("broken-alt-group")
        .execute(&state.pool)
        .await
        .expect("mark broken alternate-route account");

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
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get(http_header::RETRY_AFTER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout broken-alt response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode timeout broken-alt response body");
    assert!(
        response_payload["error"]
            .as_str()
            .expect("timeout broken-alt error should be present")
            .contains(
                "upstream account group \"broken-alt-group\" has no bound forward proxy nodes"
            )
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 2).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            attempt_index,
            status,
            failure_kind,
            error_message
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load timeout broken-alt attempt rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(
        attempt_rows[1].attempt_index, 2,
        "blocked-policy exits should still persist a terminal attempt row"
    );
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    assert_eq!(
        attempt_rows[1].failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT),
    );
    assert!(attempt_rows[1].error_message.as_deref().is_some_and(|msg| {
        msg.contains("upstream account group \"broken-alt-group\" has no bound forward proxy nodes")
    }));

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
    .expect("load timeout broken-alt payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout broken-alt payload should be present"),
    )
    .expect("decode timeout broken-alt payload");
    assert!(row.error_message.as_deref().is_some_and(|msg| {
        msg.contains("upstream account group \"broken-alt-group\" has no bound forward proxy nodes")
    }));
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT),
    );
    assert!(payload["upstreamErrorMessage"].is_null());

    shared_upstream_handle.abort();
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
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    assert_eq!(
        attempt_rows[1].failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT),
    );
    assert_eq!(attempt_rows[1].same_account_retry_index, 0);
    assert_eq!(
        attempt_rows[0].upstream_route_key,
        attempt_rows[1].upstream_route_key,
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
    wait_for_pool_attempt_row_count(&state.pool, 4).await;

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
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 1);
    assert_eq!(attempt_rows[2].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[3].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    assert_eq!(attempt_rows[3].attempt_index, 4);
    assert_eq!(attempt_rows[3].distinct_account_index, 3);
    assert_eq!(
        attempt_rows[3].failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT),
    );

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
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let (fast_two_base, attempts, fast_two_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-two", 0)]).await;
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
        "Success Route Two",
        "route-two",
        None,
        None,
        Some(fast_two_base.as_str()),
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
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-budget-success-004"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout budget success response body");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 2).await;

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
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(180);
    config.pool_upstream_responses_total_timeout = Duration::from_millis(300);
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
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
                tokio::time::sleep(Duration::from_millis(220)).await;
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
    let state = test_state_from_config(config, true).await;
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
    wait_for_pool_attempt_row_count(&state.pool, 2).await;

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
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
    );
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    assert_eq!(
        attempt_rows[1].failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED),
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
    wait_for_pool_attempt_row_count(&state.pool, 6).await;

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
        vec![1, 1, 1, 1, 2, 3]
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
        Some(1)
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
    wait_for_pool_attempt_row_count(&state.pool, 6).await;

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
        vec![1, 1, 1, 1, 2, 3]
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
        Some(1)
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
        prefetched_bytes, ..
    } = outcome
    else {
        panic!("non-overload response.failed should stay on the original stream");
    };

    let body_text = String::from_utf8(
        prefetched_bytes
            .expect("forwarded stream should keep prefetched metadata window")
            .to_vec(),
    )
    .expect("utf8 gate prefetched bytes");
    assert!(body_text.contains("response.created"));
    assert!(body_text.contains("server_error"));
    assert!(!body_text.contains("server_is_overloaded"));
}

#[tokio::test]
async fn pool_route_marks_oauth_missing_scopes_as_error_and_persists_upstream_details() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    #[derive(sqlx::FromRow)]
    struct RouteStateRow {
        status: String,
        last_error: Option<String>,
    }

    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        payload: Option<String>,
    }

    let scope_message = "You have insufficient permissions for this operation. Missing scopes: api.responses.write.";
    let (upstream_base, upstream_handle) = spawn_oauth_codex_http_failure(
        StatusCode::UNAUTHORIZED,
        Some("missing_scopes"),
        scope_message,
    )
    .await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_oauth_account(&state, "Scope OAuth", "oauth-scope").await;
    record_pool_route_success(
        &state.pool,
        account_id,
        Utc::now(),
        Some("sticky-scope-001"),
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
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-scope-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure body");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|value| value.contains("Missing scopes: api.responses.write"))
    );

    let route_state = sqlx::query_as::<_, RouteStateRow>(
        r#"
        SELECT status, last_error
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load route state");
    assert_eq!(route_state.status, "error");
    assert!(
        route_state
            .last_error
            .as_deref()
            .is_some_and(|value| value.contains("Missing scopes: api.responses.write"))
    );
    assert!(
        load_test_sticky_route_account_id(&state.pool, "sticky-scope-001")
            .await
            .is_none(),
        "permission failures should detach the sticky binding",
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load invocation payload");
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("payload should be persisted for failed capture"),
    )
    .expect("decode invocation payload");
    assert_eq!(payload_json["upstreamAccountId"].as_i64(), Some(account_id));
    assert_eq!(
        payload_json["upstreamErrorCode"].as_str(),
        Some("missing_scopes")
    );
    assert!(
        payload_json["upstreamErrorMessage"]
            .as_str()
            .is_some_and(|value| value.contains("Missing scopes: api.responses.write"))
    );
    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_marks_explicit_invalidated_oauth_as_needs_reauth() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_http_failure(
        StatusCode::FORBIDDEN,
        Some("token_invalidated"),
        "Authentication token has been invalidated, please sign in again.",
    )
    .await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id =
        insert_test_pool_oauth_account(&state, "Invalidated OAuth", "oauth-invalidated").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello"}"#.as_bytes().to_vec()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load oauth account status");
    assert_eq!(status, "needs_reauth");

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_marks_invalid_grant_error_code_as_needs_reauth() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_http_failure(
        StatusCode::UNAUTHORIZED,
        Some("invalid_grant"),
        "Unauthorized",
    )
    .await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id =
        insert_test_pool_oauth_account(&state, "Grant OAuth", "oauth-invalid-grant").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello"}"#.as_bytes().to_vec()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load oauth account status");
    assert_eq!(status, "needs_reauth");

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_oauth_passthrough_replays_large_file_backed_body() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_oauth_account(&state, "Large OAuth", "oauth-large").await;

    let body = serde_json::to_vec(&json!({
        "messages": [{
            "role": "user",
            "content": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 4096),
        }],
    }))
    .expect("serialize large oauth passthrough body");
    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(424242),
    });
    tokio::fs::write(&temp_file.path, &body)
        .await
        .expect("write replay temp file");

    let account = PoolResolvedAccount {
        account_id,
        display_name: "Large OAuth".to_string(),
        kind: "oauth_codex".to_string(),
        auth: PoolResolvedAuth::Oauth {
            access_token: "oauth-large".to_string(),
            chatgpt_account_id: Some("org_test".to_string()),
        },
        upstream_base_url: oauth_bridge::oauth_codex_upstream_base_url()
            .expect("oauth upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        group_upstream_429_retry_enabled: false,
        group_upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
    };

    let upstream = send_pool_request_with_failover(
        state,
        424242,
        Method::POST,
        &"/v1/chat/completions".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::File {
            temp_file: temp_file.clone(),
            size: body.len(),
            sticky_key: None,
        }),
        Duration::from_secs(5),
        None,
        None,
        None,
        Some(account),
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect("oauth passthrough should succeed");

    assert_eq!(upstream.response.status(), StatusCode::OK);
    let mut response_body = upstream.first_chunk.clone().unwrap_or_default().to_vec();
    response_body.extend_from_slice(
        &upstream
            .response
            .into_bytes()
            .await
            .expect("read oauth passthrough response"),
    );
    let payload =
        serde_json::from_slice::<Value>(&response_body).expect("decode oauth passthrough response");
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/chat/completions")
    );
    assert_eq!(
        payload["authorization"].as_str(),
        Some("Bearer oauth-large")
    );
    assert_eq!(payload["chatgptAccountId"].as_str(), Some("org_test"));
    assert_eq!(payload["bodyLength"].as_u64(), Some(body.len() as u64));

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_oauth_responses_sends_uuid_account_header_and_persists_observability() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        payload: Option<String>,
    }

    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_responses_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_oauth_account_with_chatgpt_account_id(
        &state,
        "UUID OAuth",
        "oauth-uuid",
        "02355c9d-fb23-4517-a96d-35e5f6758e9e",
    )
    .await;

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
                HeaderName::from_static("x-openai-prompt-cache-key"),
                HeaderValue::from_static("prompt-cache-oauth-responses"),
            ),
            (
                HeaderName::from_static("x-client-trace-id"),
                HeaderValue::from_static("trace-oauth-responses"),
            ),
            (
                HeaderName::from_static("session_id"),
                HeaderValue::from_static("session-oauth-responses"),
            ),
            (
                HeaderName::from_static("traceparent"),
                HeaderValue::from_static("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00"),
            ),
            (
                HeaderName::from_static("x-client-request-id"),
                HeaderValue::from_static("client-request-oauth-responses"),
            ),
            (
                HeaderName::from_static("x-codex-turn-metadata"),
                HeaderValue::from_static("{\"turn\":42}"),
            ),
            (
                HeaderName::from_static("originator"),
                HeaderValue::from_static("Codex Desktop"),
            ),
            (
                HeaderName::from_static("chatgpt-account-id"),
                HeaderValue::from_static("client-should-not-win"),
            ),
        ]),
        Body::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello"
            }))
            .expect("serialize oauth responses body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read oauth response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode oauth response body");
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/responses")
    );
    assert_eq!(payload["authorization"].as_str(), Some("Bearer oauth-uuid"));
    assert_eq!(
        payload["chatgptAccountId"].as_str(),
        Some("02355c9d-fb23-4517-a96d-35e5f6758e9e")
    );
    assert_eq!(
        payload["xOpenAiPromptCacheKeyHeader"].as_str(),
        Some("prompt-cache-oauth-responses")
    );
    assert_eq!(
        payload["clientTraceId"].as_str(),
        Some("trace-oauth-responses")
    );
    assert_eq!(
        payload["sessionIdHeader"].as_str(),
        Some("session-oauth-responses")
    );
    assert_eq!(
        payload["traceparentHeader"].as_str(),
        Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00")
    );
    assert_eq!(
        payload["xClientRequestIdHeader"].as_str(),
        Some("client-request-oauth-responses")
    );
    assert_eq!(
        payload["xCodexTurnMetadataHeader"].as_str(),
        Some("{\"turn\":42}")
    );
    assert_eq!(payload["originatorHeader"].as_str(), Some("Codex Desktop"));
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-openai-prompt-cache-key")
    );
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-client-trace-id")
    );
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "session_id")
    );
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "traceparent")
    );
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-client-request-id")
    );
    assert_eq!(payload["received"]["stream"], true);
    assert_eq!(payload["received"]["store"], false);
    assert_eq!(payload["received"]["instructions"], "");

    wait_for_codex_invocations(&state.pool, 1).await;
    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load persisted invocation payload");
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("payload should be persisted for oauth responses"),
    )
    .expect("decode persisted invocation payload");
    assert_eq!(payload_json["upstreamAccountId"].as_i64(), Some(account_id));
    assert_eq!(payload_json["oauthAccountHeaderAttached"], true);
    assert_eq!(payload_json["oauthAccountIdShape"].as_str(), Some("uuid"));
    assert_eq!(payload_json["endpoint"].as_str(), Some("/v1/responses"));
    assert_eq!(payload_json["oauthFingerprintVersion"].as_str(), Some("v1"));
    assert_eq!(payload_json["oauthPromptCacheHeaderForwarded"], true);
    assert!(
        payload_json["oauthForwardedHeaderCount"]
            .as_u64()
            .expect("forwarded header count")
            >= 2
    );
    assert!(
        payload_json["oauthForwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-openai-prompt-cache-key")
    );
    assert!(
        payload_json["oauthForwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-client-trace-id")
    );
    assert!(
        payload_json["oauthRequestBodyPrefixBytes"]
            .as_u64()
            .expect("body prefix byte count")
            > 0
    );
    assert!(
        payload_json["oauthRequestBodyPrefixFingerprint"]
            .as_str()
            .expect("body prefix fingerprint")
            .len()
            == 16
    );
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["session_id"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["traceparent"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["x-client-request-id"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["x-codex-turn-metadata"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["originator"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert!(payload_json["oauthForwardedHeaderFingerprints"]["x-client-trace-id"].is_null());
    assert!(
        !row.payload
            .as_deref()
            .expect("persisted payload text")
            .contains("session-oauth-responses")
    );
    assert_eq!(payload_json["oauthResponsesRewrite"]["applied"], true);
    assert_eq!(
        payload_json["oauthResponsesRewrite"]["addedInstructions"],
        true
    );
    assert_eq!(payload_json["oauthResponsesRewrite"]["addedStore"], true);
    assert_eq!(
        payload_json["oauthResponsesRewrite"]["forcedStreamTrue"],
        true
    );
    assert_eq!(
        payload_json["oauthResponsesRewrite"]["removedMaxOutputTokens"],
        false
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

