#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_total_timeout_starts_at_first_upstream_attempt() {
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
pub(crate) async fn pool_openai_v1_responses_total_timeout_caps_same_account_retry_before_first_byte()
 {
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

    assert_total_timeout_exhaustion(&state.pool, &retry_attempts).await;

    retry_upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_compact_total_timeout_caps_same_account_retry_before_first_byte()
 {
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

    assert_total_timeout_exhaustion(&state.pool, &retry_attempts).await;

    retry_upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_retries_same_account_on_server_overloaded_before_forwarding()
 {
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

    assert_same_account_overload_recovery(&state.pool, account_id, &attempts).await;

    upstream_handle.abort();
}

#[test]
pub(crate) fn pool_responses_classify_account_concurrency_rate_limit_as_retryable_overload() {
    let payload = br#"event: response.failed
data: {"type":"response.failed","response":{"id":"resp_concurrency_retry","model":"gpt-5.6-sol","status":"failed","error":{"code":"rate_limit_exceeded","message":"Concurrency limit exceeded for account, please retry later"}}}

"#;

    let decision = classify_pool_initial_responses_sse_event(StatusCode::OK, payload);
    match decision {
        PoolInitialResponsesSseEventDecision::RetrySameAccount {
            upstream_error_code,
            upstream_error_message,
            ..
        } => {
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
pub(crate) fn pool_responses_compact_gate_classifies_account_concurrency_rate_limit() {
    let payload = Bytes::from_static(
        br#"{"error":{"code":"rate_limit_exceeded","message":"Concurrency limit exceeded for account, please retry later"}}"#,
    );
    let headers = HeaderMap::new();
    let outcome = gate_pool_initial_compact_response(StatusCode::OK, &headers, Some(&payload));
    match outcome {
        Some(PoolInitialResponseGateOutcome::RetrySameAccount {
            upstream_error_code,
            upstream_error_message,
            ..
        }) => {
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
pub(crate) fn pool_responses_route_failure_keeps_generic_rate_limit_out_of_overload_class() {
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
pub(crate) async fn pool_openai_v1_responses_retries_same_account_on_account_concurrency_limit() {
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

    assert_same_account_overload_recovery(&state.pool, account_id, &attempts).await;

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_retries_after_metadata_prefix_exceeds_preview_limit() {
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
pub(crate) async fn pool_openai_v1_responses_overload_prefers_same_route_before_alternate_route() {
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

    assert_same_route_overload_observations(
        &state.pool,
        secondary_id,
        &same_route_attempts,
        &alternate_attempts,
    )
    .await;

    same_route_handle.abort();
    alternate_handle.abort();
}

use super::*;
#[derive(Debug, sqlx::FromRow)]
struct OverloadAttemptRow {
    attempt_index: i64,
    distinct_account_index: i64,
    status: String,
}

#[derive(Debug, sqlx::FromRow)]
struct RetryAccountStateRow {
    action: Option<String>,
    reason_code: Option<String>,
    cooldown_until: Option<String>,
}

async fn load_part_02_attempt_rows(pool: &SqlitePool) -> Vec<OverloadAttemptRow> {
    sqlx::query_as::<_, OverloadAttemptRow>(
        "SELECT attempt_index, distinct_account_index, status \
         FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(pool)
    .await
    .expect("load timeout or overload attempt rows")
}

async fn assert_total_timeout_exhaustion(
    pool: &SqlitePool,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    wait_for_codex_invocations(pool, 1).await;
    wait_for_pool_attempt_row_count(pool, 1).await;
    let rows = load_part_02_attempt_rows(pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].attempt_index, rows[0].distinct_account_index),
        (1, 1)
    );
    assert_eq!(
        rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    let attempts = attempts.lock().expect("lock retry route attempts");
    assert_eq!(attempts.get("Bearer route-one").copied(), Some(1));
    drop(attempts);
    let (error_message, payload): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT error_message, payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load timeout exhaustion payload");
    assert!(error_message.as_deref().is_some_and(|message| {
        message.contains("pool upstream total timeout exhausted after 300ms")
    }));
    let payload: Value = serde_json::from_str(payload.as_deref().expect("payload should exist"))
        .expect("decode timeout exhaustion payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED)
    );
}

async fn assert_same_account_overload_recovery(
    pool: &SqlitePool,
    account_id: i64,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    wait_for_codex_invocations(pool, 1).await;
    wait_for_pool_attempt_row_count(pool, 4).await;
    let rows = load_part_02_attempt_rows(pool).await;
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|row| row.distinct_account_index == 1));
    assert!(
        rows[..3]
            .iter()
            .all(|row| row.status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE)
    );
    assert_eq!(rows[3].status, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    let state = sqlx::query_as::<_, RetryAccountStateRow>(
        "SELECT last_action AS action, last_action_reason_code AS reason_code, cooldown_until \
         FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load recovered account state");
    assert_eq!(state.action.as_deref(), Some("route_recovered"));
    assert!(state.reason_code.is_none());
    assert!(state.cooldown_until.is_none());
    let recent_actions: Vec<String> = sqlx::query_scalar(
        "SELECT action FROM pool_upstream_account_events \
         WHERE account_id = ?1 ORDER BY id DESC LIMIT 5",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .expect("load recovered account events");
    assert!(
        !recent_actions
            .iter()
            .any(|action| action == "route_cooldown_started")
    );
    let attempts = attempts.lock().expect("lock overload attempts");
    assert_eq!(attempts.get("Bearer route-one").copied(), Some(4));
}

async fn assert_same_route_overload_observations(
    pool: &SqlitePool,
    secondary_id: i64,
    same_route_attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    alternate_attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    wait_for_codex_invocations(pool, 1).await;
    wait_for_pool_attempt_row_count(pool, 5).await;
    let rows = load_part_02_attempt_rows(pool).await;
    assert_eq!(
        rows.iter()
            .map(|row| row.distinct_account_index)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1, 2]
    );
    assert_eq!(
        load_test_sticky_route_account_id(pool, "sticky-overload-same-route-first").await,
        Some(secondary_id)
    );
    let same_route_attempts = same_route_attempts
        .lock()
        .expect("lock same-route attempts");
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
    let alternate_attempts = alternate_attempts.lock().expect("lock alternate attempts");
    assert!(alternate_attempts.get("Bearer route-two").is_none());
}
