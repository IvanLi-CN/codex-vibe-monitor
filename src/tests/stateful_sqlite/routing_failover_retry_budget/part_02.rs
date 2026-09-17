#[tokio::test]
async fn pool_route_http_4xx_does_not_create_sticky_route() {
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

    assert_http_4xx_does_not_create_route(&state, &attempts, primary_id).await;

    upstream_handle.abort();
}

async fn assert_http_4xx_does_not_create_route(
    state: &Arc<AppState>,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    primary_id: i64,
) {
    let attempt_rows = sqlx::query_as::<_, (String, Option<i64>, Option<String>)>(
        "SELECT status, http_status, error_message FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load 4xx attempt rows");
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(
        attempt_rows[0].0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(attempt_rows[0].1, Some(400));
    assert!(
        attempt_rows[0]
            .2
            .as_deref()
            .is_some_and(|message| !message.is_empty()),
        "4xx attempt should preserve error information: {attempt_rows:?}",
    );
    let account_route_state = sqlx::query_as::<_, (String, Option<String>, Option<String>, i64)>(
        "SELECT status, last_route_failure_at, cooldown_until, consecutive_route_failures FROM pool_upstream_accounts WHERE id = ?1",
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

    assert_non_responses_retry_result(&state, &attempts, payload, primary_id, secondary_id).await;

    upstream_handle.abort();
}

#[derive(Debug, sqlx::FromRow)]
struct AttemptRouteRow {
    upstream_account_id: Option<i64>,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: String,
    failure_kind: Option<String>,
}

async fn assert_non_responses_retry_result(
    state: &Arc<AppState>,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    payload: Value,
    primary_id: i64,
    secondary_id: i64,
) {
    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 3).await;
    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        "SELECT upstream_account_id, distinct_account_index, same_account_retry_index, status, failure_kind FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
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
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
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
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
            );
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
            assert_eq!(sticky_account_id, secondary_id);
        }
        other => panic!("unexpected upstream authorization: {other:?}"),
    }
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

    assert_no_content_success_persisted(&state, primary_id).await;

    upstream_handle.abort();
}

async fn assert_no_content_success_persisted(state: &Arc<AppState>, primary_id: i64) {
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
            "SELECT status, finished_at, http_status, stream_latency_ms, failure_kind, upstream_account_id FROM pool_upstream_request_attempts ORDER BY id DESC LIMIT 1",
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
        "SELECT status, failure_kind FROM codex_invocations WHERE source = 'proxy' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load no-content invocation row");
    assert_eq!(invocation_row.0, "success");
    assert_eq!(invocation_row.1, None);
}

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
