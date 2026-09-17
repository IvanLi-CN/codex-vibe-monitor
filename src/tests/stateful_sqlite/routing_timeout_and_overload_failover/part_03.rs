#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_overload_falls_back_to_alternate_route_after_same_route_exhaustion()
 {
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

    assert_overload_fallback_observations(
        &state.pool,
        &same_route_attempts,
        &alternate_attempts,
        OverloadFallbackExpectation {
            sticky_key: "sticky-overload-alternate-route",
            alternate_id,
            primary_auth: "Bearer route-one-primary",
            secondary_auth: "Bearer route-one-secondary",
            alternate_auth: "Bearer route-two",
        },
    )
    .await;

    same_route_handle.abort();
    alternate_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_compact_overload_falls_back_to_alternate_route_before_body_forward()
 {
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

    assert_overload_fallback_observations(
        &state.pool,
        &same_route_attempts,
        &alternate_attempts,
        OverloadFallbackExpectation {
            sticky_key: "sticky-compact-overload",
            alternate_id,
            primary_auth: "Bearer compact-primary",
            secondary_auth: "Bearer compact-secondary",
            alternate_auth: "Bearer compact-route-two",
        },
    )
    .await;

    same_route_handle.abort();
    alternate_handle.abort();
}

#[tokio::test]
pub(crate) async fn gate_pool_initial_response_stream_keeps_non_overload_response_failed_on_original_stream()
 {
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
pub(crate) async fn gate_pool_initial_response_stream_preserves_first_forward_event_boundary() {
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
pub(crate) async fn gate_pool_initial_response_stream_preserves_replayed_chunk_timestamp() {
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
pub(crate) async fn gate_pool_initial_response_stream_retries_overload_after_metadata_prefix_exceeds_preview_limit()
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

#[tokio::test]
pub(crate) async fn pool_route_marks_oauth_missing_scopes_as_error_and_persists_upstream_details() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

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

    assert_missing_scope_persistence(&state, account_id, scope_message).await;
    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
pub(crate) async fn pool_route_marks_explicit_invalidated_oauth_as_needs_reauth() {
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
pub(crate) async fn pool_route_marks_invalid_grant_error_code_as_needs_reauth() {
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
pub(crate) async fn pool_route_oauth_passthrough_replays_large_file_backed_body() {
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

    let account = large_oauth_account(account_id);

    let upstream = send_pool_request_with_failover(PoolFailoverRequest {
        state,
        proxy_request_id: 424242,
        method: Method::POST,
        original_uri: &"/v1/chat/completions".parse().expect("valid uri"),
        headers: &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        body: Some(PoolReplayBodySnapshot::File {
            temp_file: temp_file.clone(),
            size: body.len(),
        }),
        handshake_timeout: Duration::from_secs(5),
        trace_context: None,
        runtime_snapshot_context: None,
        sticky_key: None,
        preferred_account: Some(account),
        failover_progress: PoolFailoverProgress::default(),
        same_account_attempts: 1,
    })
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
pub(crate) async fn pool_route_oauth_responses_sends_uuid_account_header_and_persists_observability()
 {
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
        oauth_observability_headers(),
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
    assert_oauth_capture_payload(&payload);
    assert_oauth_persisted_observability(&state, account_id).await;

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

use super::*;
struct OverloadFallbackExpectation<'a> {
    sticky_key: &'a str,
    alternate_id: i64,
    primary_auth: &'a str,
    secondary_auth: &'a str,
    alternate_auth: &'a str,
}

#[derive(sqlx::FromRow)]
struct OauthRouteStateRow {
    status: String,
    last_error: Option<String>,
}

async fn assert_overload_fallback_observations(
    pool: &SqlitePool,
    same_route_attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    alternate_attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    expected: OverloadFallbackExpectation<'_>,
) {
    wait_for_codex_invocations(pool, 1).await;
    wait_for_pool_attempt_row_count(pool, 8).await;
    let indexes: Vec<i64> = sqlx::query_scalar(
        "SELECT distinct_account_index FROM pool_upstream_request_attempts \
         ORDER BY attempt_index ASC",
    )
    .fetch_all(pool)
    .await
    .expect("load overload attempt rows");
    assert_eq!(indexes, vec![1, 1, 1, 1, 2, 2, 2, 3]);
    assert_eq!(
        load_test_sticky_route_account_id(pool, expected.sticky_key).await,
        Some(expected.alternate_id)
    );
    let same_route_attempts = same_route_attempts
        .lock()
        .expect("lock same-route attempts");
    assert_eq!(
        same_route_attempts.get(expected.primary_auth).copied(),
        Some(4)
    );
    assert_eq!(
        same_route_attempts.get(expected.secondary_auth).copied(),
        Some(3)
    );
    drop(same_route_attempts);
    let alternate_attempts = alternate_attempts.lock().expect("lock alternate attempts");
    assert_eq!(
        alternate_attempts.get(expected.alternate_auth).copied(),
        Some(1)
    );
}

async fn assert_missing_scope_persistence(
    state: &Arc<AppState>,
    account_id: i64,
    scope_message: &str,
) {
    let route_state = sqlx::query_as::<_, OauthRouteStateRow>(
        "SELECT status, last_error FROM pool_upstream_accounts WHERE id = ?1",
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
            .is_some_and(|value| value.contains(scope_message))
    );
    assert!(
        load_test_sticky_route_account_id(&state.pool, "sticky-scope-001")
            .await
            .is_none()
    );
    let clear_event = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT action, routing_context_json, sticky_before_json \
         FROM prompt_cache_conversation_operation_events WHERE prompt_cache_key = ?1 \
         ORDER BY id DESC LIMIT 1",
    )
    .bind("sticky-scope-001")
    .fetch_one(&state.pool)
    .await
    .expect("automatic sticky clear event should be persisted");
    assert_eq!(clear_event.0, "stickyTargetCleared");
    assert!(
        clear_event
            .1
            .as_deref()
            .is_some_and(|value| value.contains("upstream_http_401"))
    );
    assert!(clear_event.2.is_some());
    wait_for_codex_invocations(&state.pool, 1).await;
    let payload: Option<String> =
        sqlx::query_scalar("SELECT payload FROM codex_invocations ORDER BY id DESC LIMIT 1")
            .fetch_one(&state.pool)
            .await
            .expect("load invocation payload");
    let payload: Value = serde_json::from_str(payload.as_deref().expect("payload should exist"))
        .expect("decode invocation payload");
    assert_eq!(payload["upstreamAccountId"].as_i64(), Some(account_id));
    assert_eq!(
        payload["upstreamErrorCode"].as_str(),
        Some("missing_scopes")
    );
    assert!(
        payload["upstreamErrorMessage"]
            .as_str()
            .is_some_and(|value| value.contains(scope_message))
    );
}

fn large_oauth_account(account_id: i64) -> PoolResolvedAccount {
    PoolResolvedAccount {
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
        sticky_affinity_generation: None,
        routing_selection_audit: None,
        priority_handoff_permit: None,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        single_account_rotation_enabled: false,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
    }
}

fn oauth_observability_headers() -> HeaderMap {
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
    ])
}

fn assert_oauth_capture_payload(payload: &Value) {
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/responses")
    );
    assert_eq!(payload["authorization"].as_str(), Some("Bearer oauth-uuid"));
    assert_eq!(
        payload["chatgptAccountId"].as_str(),
        Some("02355c9d-fb23-4517-a96d-35e5f6758e9e")
    );
    let expected_headers = [
        (
            "xOpenAiPromptCacheKeyHeader",
            "prompt-cache-oauth-responses",
        ),
        ("clientTraceId", "trace-oauth-responses"),
        ("sessionIdHeader", "session-oauth-responses"),
        (
            "traceparentHeader",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
        ),
        ("xClientRequestIdHeader", "client-request-oauth-responses"),
        ("xCodexTurnMetadataHeader", "{\"turn\":42}"),
        ("originatorHeader", "Codex Desktop"),
    ];
    for (field, expected) in expected_headers {
        assert_eq!(payload[field].as_str(), Some(expected));
    }
    let forwarded = payload["forwardedHeaderNames"]
        .as_array()
        .expect("forwarded header names");
    for name in [
        "x-openai-prompt-cache-key",
        "x-client-trace-id",
        "session_id",
        "traceparent",
        "x-client-request-id",
    ] {
        assert!(
            forwarded
                .iter()
                .filter_map(Value::as_str)
                .any(|item| item == name)
        );
    }
    assert_eq!(payload["received"]["stream"], true);
    assert_eq!(payload["received"]["store"], false);
    assert_eq!(payload["received"]["instructions"], "");
}

async fn load_oauth_observability_payload(state: &Arc<AppState>) -> (String, Value) {
    wait_for_codex_invocations(&state.pool, 1).await;
    let payload: Option<String> =
        sqlx::query_scalar("SELECT payload FROM codex_invocations ORDER BY id DESC LIMIT 1")
            .fetch_one(&state.pool)
            .await
            .expect("load persisted invocation payload");
    let payload_text = payload.expect("payload should be persisted for oauth responses");
    let payload_json =
        serde_json::from_str(&payload_text).expect("decode persisted invocation payload");
    (payload_text, payload_json)
}

fn assert_oauth_fingerprint_observability(payload: &Value) {
    assert_eq!(payload["oauthFingerprintVersion"].as_str(), Some("v1"));
    assert_eq!(payload["oauthPromptCacheHeaderForwarded"], true);
    assert_eq!(
        payload["oauthRequestBodySnapshotKind"].as_str(),
        Some("memory")
    );
    assert_eq!(
        payload["oauthResponsesBodyMode"].as_str(),
        Some("small_body_rewrite")
    );
    assert!(
        payload["oauthForwardedHeaderCount"]
            .as_u64()
            .is_some_and(|count| count >= 2)
    );
    let names = payload["oauthForwardedHeaderNames"]
        .as_array()
        .expect("forwarded header names");
    for name in ["x-openai-prompt-cache-key", "x-client-trace-id"] {
        assert!(
            names
                .iter()
                .filter_map(Value::as_str)
                .any(|item| item == name)
        );
    }
    assert!(
        payload["oauthRequestBodyPrefixBytes"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert_eq!(
        payload["oauthRequestBodyPrefixFingerprint"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    for name in [
        "session_id",
        "traceparent",
        "x-client-request-id",
        "x-codex-turn-metadata",
        "originator",
    ] {
        assert_eq!(
            payload["oauthForwardedHeaderFingerprints"][name]
                .as_str()
                .map(str::len),
            Some(16)
        );
    }
    assert!(payload["oauthForwardedHeaderFingerprints"]["x-client-trace-id"].is_null());
}

fn assert_oauth_rewrite_observability(payload: &Value) {
    let rewrite = &payload["oauthResponsesRewrite"];
    assert_eq!(rewrite["applied"], true);
    assert_eq!(rewrite["addedInstructions"], true);
    assert_eq!(rewrite["addedStore"], true);
    assert_eq!(rewrite["forcedStreamTrue"], true);
    assert_eq!(rewrite["removedMaxOutputTokens"], false);
}

async fn assert_oauth_persisted_observability(state: &Arc<AppState>, account_id: i64) {
    let (payload_text, payload) = load_oauth_observability_payload(state).await;
    assert_eq!(payload["upstreamAccountId"].as_i64(), Some(account_id));
    assert_eq!(payload["oauthAccountHeaderAttached"], true);
    assert_eq!(payload["oauthAccountIdShape"].as_str(), Some("uuid"));
    assert_eq!(payload["endpoint"].as_str(), Some("/v1/responses"));
    assert!(!payload_text.contains("session-oauth-responses"));
    assert_oauth_fingerprint_observability(&payload);
    assert_oauth_rewrite_observability(&payload);
}
