#[tokio::test]
async fn pool_openai_v1_responses_fast_fill_missing_large_body_recomputes_content_length() {
    let (capture_base, captured_requests, capture_handle) =
        spawn_capture_target_body_upstream().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;

    let fill_missing_tag_id =
        create_test_fast_mode_tag(&state, "fill-missing-large-fast", "fill_missing", "primary")
            .await;
    create_test_tagged_pool_api_key_account(
        &state,
        "Large Fast Account",
        "upstream-large-fast",
        &capture_base,
        &[fill_missing_tag_id],
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": [{
            "role": "user",
            "content": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 4096)
        }],
        "stickyKey": "sticky-fast-large-success"
    }))
    .expect("serialize large fast request body");
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
            (
                http_header::CONTENT_LENGTH,
                HeaderValue::from_str(&request_body.len().to_string())
                    .expect("valid content length"),
            ),
        ]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read large fast response body");
    let response_payload: Value =
        serde_json::from_slice(&response_body).expect("decode large fast response body");
    assert_eq!(
        response_payload["received"]["service_tier"].as_str(),
        Some("priority")
    );
    assert!(response_payload["received"].get("serviceTier").is_none());

    wait_for_codex_invocations(&state.pool, 1).await;
    assert_fast_fill_persisted_rewrite(&state, &captured_requests).await;

    capture_handle.abort();
}

#[derive(Debug, sqlx::FromRow)]
struct FastFillPersistedRow {
    payload: Option<String>,
    request_raw_path: Option<String>,
}

async fn assert_fast_fill_persisted_rewrite(
    state: &Arc<AppState>,
    captured_requests: &Arc<Mutex<Vec<Value>>>,
) {
    let captured = captured_requests.lock().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["service_tier"].as_str(), Some("priority"));
    assert!(captured[0].get("serviceTier").is_none());
    drop(captured);
    let row = sqlx::query_as::<_, FastFillPersistedRow>(
        "SELECT payload, request_raw_path FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load persisted large fast row");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("persisted payload should exist"),
    )
    .expect("decode persisted large fast payload");
    assert_eq!(payload["requestedServiceTier"].as_str(), Some("priority"));
    let request_raw = read_proxy_raw_bytes(
        row.request_raw_path
            .as_deref()
            .expect("large fast request raw path should exist"),
        state.config.database_path.parent(),
    )
    .expect("read large fast request raw");
    let request_payload: Value =
        serde_json::from_slice(&request_raw).expect("decode large fast request raw");
    assert_eq!(request_payload["service_tier"].as_str(), Some("priority"));
    assert!(request_payload.get("serviceTier").is_none());
}

#[tokio::test]
async fn pool_openai_v1_responses_fast_fill_missing_transport_failure_persists_rewritten_request_raw()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;

    let fill_missing_tag_id = create_test_fast_mode_tag(
        &state,
        "fill-missing-transport-fast",
        "fill_missing",
        "primary",
    )
    .await;
    create_test_tagged_pool_api_key_account(
        &state,
        "Broken Fast Account",
        "upstream-broken-fast",
        "http://127.0.0.1:1/",
        &[fill_missing_tag_id],
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": [{
            "role": "user",
            "content": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 4096)
        }],
        "stickyKey": "sticky-fast-large-failure"
    }))
    .expect("serialize failed large fast request body");
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
            (
                http_header::CONTENT_LENGTH,
                HeaderValue::from_str(&request_body.len().to_string())
                    .expect("valid content length"),
            ),
        ]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failed large fast response body");
    let response_text = String::from_utf8_lossy(&response_body);
    assert!(
        response_text.contains("failed to contact upstream"),
        "unexpected failed large fast response body: {response_text}"
    );

    assert_failed_fast_fill_persisted_rewrite(&state).await;
}

async fn assert_failed_fast_fill_persisted_rewrite(state: &Arc<AppState>) {
    wait_for_codex_invocations(&state.pool, 1).await;
    let row = sqlx::query_as::<_, TransportPersistedRow>(
        "SELECT status, error_message, payload, request_raw_path FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load failed large fast row");
    assert_eq!(row.status.as_deref(), Some("http_502"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|message| message.contains("[failed_contact_upstream]"))
    );
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("failed large fast payload should exist"),
    )
    .expect("decode failed large fast payload");
    assert_eq!(payload["requestModel"].as_str(), Some("gpt-5.4"));
    assert_eq!(payload["requestedServiceTier"].as_str(), Some("priority"));
    assert_eq!(
        payload["failureKind"].as_str(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    let request_raw = read_proxy_raw_bytes(
        row.request_raw_path
            .as_deref()
            .expect("failed large fast request raw path should exist"),
        state.config.database_path.parent(),
    )
    .expect("read failed large fast request raw");
    let request_payload: Value =
        serde_json::from_slice(&request_raw).expect("decode failed large fast request raw");
    assert_eq!(request_payload["service_tier"].as_str(), Some("priority"));
    assert!(request_payload.get("serviceTier").is_none());
}

#[derive(Debug, sqlx::FromRow)]
struct TransportPersistedRow {
    status: Option<String>,
    error_message: Option<String>,
    payload: Option<String>,
    request_raw_path: Option<String>,
}

#[tokio::test]
async fn pool_route_responses_compact_retries_follow_up_accounts_before_switching() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[
        ("Bearer upstream-primary", 99),
        ("Bearer upstream-secondary", 99),
        ("Bearer upstream-tertiary", 0),
    ])
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    insert_test_pool_api_key_account(&state, "Tertiary", "upstream-tertiary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_001",
        "input": [{"role": "user", "content": "compact this thread"}],
    }))
    .expect("serialize compact request body");
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
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact retry body");

    assert_compact_retry_observations(&state, &attempts).await;

    upstream_handle.abort();
}

#[derive(Debug, sqlx::FromRow)]
struct CompactAttemptRow {
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: String,
}

async fn assert_compact_retry_observations(
    state: &Arc<AppState>,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 7).await;
    let attempt_rows = sqlx::query_as::<_, CompactAttemptRow>(
        "SELECT distinct_account_index, same_account_retry_index, status FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load compact retry rows");
    assert_eq!(attempt_rows.len(), 7);
    for (index, row) in attempt_rows.iter().enumerate() {
        let expected_account = if index < 3 {
            1
        } else if index < 6 {
            2
        } else {
            3
        };
        let expected_retry: i64 = if index == 6 {
            1
        } else {
            (index % 3 + 1) as i64
        };
        assert_eq!(row.distinct_account_index, expected_account);
        assert_eq!(row.same_account_retry_index, expected_retry);
    }
    assert_eq!(
        attempt_rows[6].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );
    let attempts = attempts.lock().expect("lock compact attempt counters");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(1));
    drop(attempts);
    let payload: Value = sqlx::query_scalar::<_, Option<String>>(
        "SELECT payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load compact invocation payload")
    .and_then(|value| serde_json::from_str(&value).ok())
    .expect("decode compact payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(7));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert!(payload["poolAttemptTerminalReason"].is_null());
}

#[tokio::test]
async fn pool_route_compact_502_returns_cvm_id_and_attempt_observations() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_compact_unsupported_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    insert_test_pool_api_key_account(&state, "Tertiary", "upstream-tertiary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_002",
        "input": [{"role": "user", "content": "compact this thread"}],
    }))
    .expect("serialize compact request body");
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

    assert_compact_unsupported_result(&state, &attempts, response).await;

    upstream_handle.abort();
}

async fn assert_compact_unsupported_result(
    state: &Arc<AppState>,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    response: axum::response::Response,
) {
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let cvm_id = response
        .headers()
        .get(CVM_INVOKE_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .expect("cvm id header should be present");
    assert!(proxy_invoke_id_has_short_format(&cvm_id));
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact 502 response");
    let payload: Value = serde_json::from_slice(&body).expect("decode compact 502 payload");
    assert_eq!(payload["cvmId"].as_str(), Some(cvm_id.as_str()));
    assert_eq!(
        payload["error"].as_str(),
        Some("pool distinct-account retry budget exhausted")
    );
    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 9).await;
    let invocation_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
            .bind(&cvm_id)
            .fetch_optional(&state.pool)
            .await
            .expect("load compact invocation status");
    assert_eq!(invocation_status.as_deref(), Some("http_502"));
    let Json(attempt_rows) =
        fetch_invocation_pool_attempts(State(state.clone()), axum::extract::Path(cvm_id))
            .await
            .expect("fetch invocation pool attempts");
    assert_eq!(attempt_rows.len(), 3);
    assert!(attempt_rows.iter().all(
        |row| row.compact_support_status.as_deref() == Some(COMPACT_SUPPORT_STATUS_UNSUPPORTED)
    ));
    assert!(
        attempt_rows[0]
            .compact_support_reason
            .as_deref()
            .is_some_and(|value| value.contains("No available channel for model"))
    );
    let account_support_states = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT compact_support_status, compact_support_reason FROM pool_upstream_accounts ORDER BY id ASC",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load compact support states");
    assert_eq!(account_support_states.len(), 3);
    assert!(
        account_support_states
            .iter()
            .all(|row| row.0 == COMPACT_SUPPORT_STATUS_UNSUPPORTED)
    );
    let attempts = attempts.lock().expect("lock compact unsupported attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(1));
}

#[tokio::test]
async fn pool_route_chat_completions_keeps_three_attempts_for_follow_up_accounts() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[
        ("Bearer upstream-primary", 99),
        ("Bearer upstream-secondary", 2),
    ])
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "messages": [{"role": "user", "content": "hello"}],
    }))
    .expect("serialize chat completions body");
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/chat/completions"
                .parse()
                .expect("valid chat completions uri"),
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
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read chat completions retry body");

    assert_chat_retry_observations(&state, &attempts).await;

    upstream_handle.abort();
}

async fn assert_chat_retry_observations(
    state: &Arc<AppState>,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 6).await;
    let attempt_rows = sqlx::query_as::<_, CompactAttemptRow>(
        "SELECT distinct_account_index, same_account_retry_index, status FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load chat completions retry rows");
    assert_eq!(attempt_rows.len(), 6);
    for (index, row) in attempt_rows.iter().enumerate() {
        assert_eq!(row.distinct_account_index, if index < 3 { 1 } else { 2 });
        assert_eq!(row.same_account_retry_index, (index % 3 + 1) as i64);
    }
    assert_eq!(
        attempt_rows[5].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );
    let attempts = attempts
        .lock()
        .expect("lock chat completions attempt counters");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
    drop(attempts);
    let payload: Value = sqlx::query_scalar::<_, Option<String>>(
        "SELECT payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load chat completions payload")
    .and_then(|value| serde_json::from_str(&value).ok())
    .expect("decode chat completions payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(6));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
    assert!(payload["poolAttemptTerminalReason"].is_null());
}
