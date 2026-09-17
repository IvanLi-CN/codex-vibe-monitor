pub(crate) fn run_timeout_future_with_large_stack<Fut>(future: Fut)
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

async fn insert_timeout_exhaustion_routes(state: &Arc<AppState>, routes: [(&str, &str, &str); 4]) {
    for (display_name, route_key, upstream_base) in routes {
        insert_test_pool_api_key_account_with_options(
            state,
            display_name,
            route_key,
            None,
            None,
            Some(upstream_base),
        )
        .await;
    }
}

#[test]
pub(crate) fn capture_target_pool_route_timeout_ignores_legacy_group_proxy_error_for_transit() {
    run_timeout_future_with_large_stack(async move {
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
            Some("https://broken-alt.example.com/backend-api/codex"),
        )
        .await;
        set_test_account_group_name(&state.pool, broken_alternate_id, Some("broken-alt-group"))
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
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read timeout broken-alt response body");
        let response_payload: Value =
            serde_json::from_slice(&body).expect("decode timeout broken-alt response body");
        let error = response_payload["error"]
            .as_str()
            .expect("transit route error should be present");
        assert!(!error.contains("has no bound forward proxy nodes"));

        shared_upstream_handle.abort();
    });
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_timeout_replay_failover_preserves_no_alternate_terminal_reason()
 {
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

    let attempt_rows = load_timeout_attempt_rows(&state.pool).await;
    assert_timeout_attempt_sequence(
        &attempt_rows,
        &[POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE],
    );
    assert_eq!(
        attempt_rows[0].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
    );
    assert_timeout_terminal_payload(&state.pool, 1).await;

    shared_upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_timeout_can_switch_twice_then_succeed() {
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

    assert_timeout_double_switch(&state.pool).await;

    slow_one_handle.abort();
    slow_two_handle.abort();
    fast_three_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_timeout_exhausts_after_three_routes() {
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
    insert_timeout_exhaustion_routes(
        &state,
        [
            ("Timeout Route One", "route-one", slow_one_base.as_str()),
            ("Timeout Route Two", "route-two", slow_two_base.as_str()),
            (
                "Timeout Route Three",
                "route-three",
                slow_three_base.as_str(),
            ),
            ("Unused Route Four", "route-four", fast_four_base.as_str()),
        ],
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

    let attempt_rows = load_timeout_attempt_rows(&state.pool).await;
    assert_timeout_attempt_sequence(
        &attempt_rows,
        &[
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        ],
    );
    assert_timeout_stream_failure_kinds(&attempt_rows);
    let attempts = attempts.lock().expect("lock unused route attempts");
    assert_eq!(attempts.get("Bearer route-four").copied(), None);
    drop(attempts);
    assert_timeout_terminal_payload(&state.pool, 3).await;

    slow_one_handle.abort();
    slow_two_handle.abort();
    slow_three_handle.abort();
    fast_four_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_total_timeout_can_succeed_on_second_route() {
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

    let attempt_rows = load_timeout_attempt_rows(&state.pool).await;
    assert_timeout_attempt_sequence(
        &attempt_rows,
        &[
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ],
    );
    assert_eq!(
        attempt_rows[0].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
    );

    let attempts = attempts.lock().expect("lock route-two attempts");
    assert_eq!(attempts.get("Bearer route-two").copied(), Some(1));
    drop(attempts);

    assert_timeout_success_payload(&state.pool, 2).await;

    slow_one_handle.abort();
    fast_two_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_stream_timeout_does_not_cap_pre_first_byte_failover()
{
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

    let attempt_rows = load_timeout_attempt_rows(&state.pool).await;
    assert_timeout_attempt_sequence(
        &attempt_rows,
        &[
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ],
    );

    let attempts = attempts.lock().expect("lock unused route attempts");
    assert_eq!(attempts.get("Bearer route-three").copied(), Some(1));
    drop(attempts);

    assert_timeout_success_payload(&state.pool, 3).await;

    slow_one_handle.abort();
    slow_two_handle.abort();
    fast_three_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_compact_stream_timeout_does_not_cap_pre_first_byte_failover()
 {
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
    assert_timeout_route_three_response(response).await;

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 3).await;

    let attempt_rows = load_timeout_attempt_rows(&state.pool).await;
    assert_timeout_attempt_sequence(
        &attempt_rows,
        &[
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ],
    );

    let fast_three_attempts = fast_three_attempts
        .lock()
        .expect("lock compact route-three attempts");
    assert_eq!(
        fast_three_attempts.get("Bearer route-three").copied(),
        Some(1)
    );
    drop(fast_three_attempts);

    assert_timeout_success_payload(&state.pool, 3).await;

    slow_one_handle.abort();
    slow_two_handle.abort();
    fast_three_handle.abort();
}

use super::*;
#[derive(Debug, sqlx::FromRow)]
struct TimeoutAttemptRow {
    upstream_route_key: Option<String>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: String,
    failure_kind: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct TimeoutPayloadRow {
    error_message: Option<String>,
    payload: Option<String>,
}

async fn timeout_test_state(attempt_timeout: Duration, total_timeout: Duration) -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = attempt_timeout;
    config.pool_upstream_responses_total_timeout = total_timeout;
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    state
}

async fn insert_timeout_routes(state: &Arc<AppState>, routes: &[(&str, &str, &str)]) -> Vec<i64> {
    let mut ids = Vec::with_capacity(routes.len());
    for (display_name, api_key, upstream_base) in routes {
        ids.push(
            insert_test_pool_api_key_account_with_options(
                state,
                display_name,
                api_key,
                None,
                None,
                Some(upstream_base),
            )
            .await,
        );
    }
    ids
}

async fn send_timeout_json(state: &Arc<AppState>, path: &str, body: Vec<u8>) -> Response {
    proxy_openai_v1(
        State(state.clone()),
        OriginalUri(path.parse().expect("valid timeout test uri")),
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
        Body::from(body),
    )
    .await
}

async fn load_timeout_attempt_rows(pool: &SqlitePool) -> Vec<TimeoutAttemptRow> {
    sqlx::query_as::<_, TimeoutAttemptRow>(
        "SELECT upstream_route_key, attempt_index, distinct_account_index, \
         same_account_retry_index, status, failure_kind \
         FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(pool)
    .await
    .expect("load timeout attempt rows")
}

fn assert_timeout_attempt_sequence(rows: &[TimeoutAttemptRow], statuses: &[&str]) {
    assert_eq!(rows.len(), statuses.len());
    for (index, (row, status)) in rows.iter().zip(statuses).enumerate() {
        let expected_index = i64::try_from(index + 1).expect("attempt index fits i64");
        assert_eq!(row.attempt_index, expected_index);
        assert_eq!(row.distinct_account_index, expected_index);
        assert_eq!(row.same_account_retry_index, 1);
        assert_eq!(row.status, *status);
    }
}

fn assert_timeout_stream_failure_kinds(rows: &[TimeoutAttemptRow]) {
    assert!(
        rows.iter().all(|row| {
            row.failure_kind.as_deref() == Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
        })
    );
}

async fn assert_timeout_route_three_response(response: Response) {
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact third-route success response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode third-route success body");
    assert_eq!(payload["ok"].as_bool(), Some(true));
    assert_eq!(
        payload["authorization"].as_str(),
        Some("Bearer route-three")
    );
}

async fn load_timeout_payload_row(pool: &SqlitePool) -> (TimeoutPayloadRow, Value) {
    let row = sqlx::query_as::<_, TimeoutPayloadRow>(
        "SELECT error_message, payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load timeout invocation payload");
    let payload = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout invocation payload should be present"),
    )
    .expect("decode timeout invocation payload");
    (row, payload)
}

async fn assert_timeout_success_payload(pool: &SqlitePool, attempts: i64) {
    let (_, payload) = load_timeout_payload_row(pool).await;
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(attempts));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(attempts));
    assert!(payload["poolAttemptTerminalReason"].is_null());
}

async fn assert_timeout_double_switch(pool: &SqlitePool) {
    let rows = load_timeout_attempt_rows(pool).await;
    assert_timeout_attempt_sequence(
        &rows,
        &[
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ],
    );
    assert_ne!(rows[0].upstream_route_key, rows[1].upstream_route_key);
    assert_ne!(rows[1].upstream_route_key, rows[2].upstream_route_key);
    assert_timeout_success_payload(pool, 3).await;
}

async fn assert_timeout_terminal_payload(pool: &SqlitePool, attempts: i64) {
    let (row, payload) = load_timeout_payload_row(pool).await;
    assert!(row.error_message.as_deref().is_some_and(|message| {
        message.contains("no alternate upstream route is available after timeout")
    }));
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(attempts));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(attempts));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT)
    );
    assert!(payload["upstreamErrorMessage"].is_null());
}
