use super::*;
use serde_json::json;

#[test]
fn summarize_pool_upstream_http_failure_ignores_html_bodies() {
    let (code, message, request_id, summary) = summarize_pool_upstream_http_failure(
        StatusCode::UNAUTHORIZED,
        None,
        b"<html>blocked</html>",
    );
    assert_eq!(code, None);
    assert_eq!(message, None);
    assert_eq!(request_id, None);
    assert_eq!(summary, "pool upstream responded with 401");
}

#[test]
fn summarize_pool_upstream_http_failure_prefers_request_id_header() {
    let body = br#"{"error":{"message":"Missing scopes: api.responses.write","code":"insufficient_permissions"}}"#;
    let (code, message, request_id, summary) =
        summarize_pool_upstream_http_failure(StatusCode::FORBIDDEN, Some("req_123abc"), body);
    assert_eq!(code.as_deref(), Some("insufficient_permissions"));
    assert_eq!(
        message.as_deref(),
        Some("Missing scopes: api.responses.write")
    );
    assert_eq!(request_id.as_deref(), Some("req_123abc"));
    assert_eq!(
        summary,
        "pool upstream responded with 403: Missing scopes: api.responses.write"
    );
}

#[test]
fn summarize_pool_upstream_http_failure_keeps_plaintext_reauth_signal() {
    let (code, message, request_id, summary) = summarize_pool_upstream_http_failure(
        StatusCode::UNAUTHORIZED,
        None,
        b"invalid_grant: please sign in again",
    );
    assert_eq!(code, None);
    assert_eq!(
        message.as_deref(),
        Some("invalid_grant: please sign in again")
    );
    assert_eq!(request_id, None);
    assert_eq!(
        summary,
        "pool upstream responded with 401: invalid_grant: please sign in again"
    );
}

#[test]
fn summarize_pool_upstream_http_failure_reads_nested_request_id() {
    let body = br#"{"response":{"error":{"message":"Request failed","code":"bad_request","request_id":"req_nested_123"}}}"#;
    let (code, message, request_id, summary) =
        summarize_pool_upstream_http_failure(StatusCode::BAD_REQUEST, None, body);
    assert_eq!(code.as_deref(), Some("bad_request"));
    assert_eq!(message.as_deref(), Some("Request failed"));
    assert_eq!(request_id.as_deref(), Some("req_nested_123"));
    assert_eq!(summary, "pool upstream responded with 400: Request failed");
}

#[test]
fn summarize_pool_upstream_http_failure_reads_top_level_error_fields() {
    let body =
        br#"{"message":"Gateway says no","code":"gateway_forbidden","request_id":"req_top_456"}"#;
    let (code, message, request_id, summary) =
        summarize_pool_upstream_http_failure(StatusCode::FORBIDDEN, None, body);
    assert_eq!(code.as_deref(), Some("gateway_forbidden"));
    assert_eq!(message.as_deref(), Some("Gateway says no"));
    assert_eq!(request_id.as_deref(), Some("req_top_456"));
    assert_eq!(summary, "pool upstream responded with 403: Gateway says no");
}

#[tokio::test]
async fn capture_target_pool_route_keeps_unclassified_response_failed_stream_diagnostic_only() {
    #[derive(sqlx::FromRow)]
    struct RouteStateRow {
        status: String,
        last_error: Option<String>,
        consecutive_route_failures: i64,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    record_pool_route_success(
        &state.pool,
        account_id,
        Utc::now(),
        Some("sticky-cap-logical"),
        None,
    )
    .await
    .expect("seed sticky route");

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
        "stickyKey": "sticky-cap-logical"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=response_failed"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");

    wait_for_codex_invocations(&state.pool, 1).await;
    let route_state = sqlx::query_as::<_, RouteStateRow>(
        r#"
        SELECT status, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load route state");
    assert_eq!(route_state.status, "active");
    assert_eq!(route_state.consecutive_route_failures, 0);
    assert!(route_state.last_error.is_none());
    let model_route = sqlx::query_as::<_, (String, String, i64, Option<String>)>(
        "SELECT state, priority, consecutive_failures, last_failure_kind FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = 'gpt-5.4'",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load logical failure model route");
    let latest_event = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT action, reason_code, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load logical failure event");
    assert_eq!(model_route.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(model_route.1, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(model_route.2, 0);
    assert!(model_route.3.is_none());
    assert_eq!(
        latest_event.0,
        UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED
    );
    assert_eq!(
        latest_event.1.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert!(latest_event.2.is_none());
    assert!(
        load_test_sticky_route_account_id(&state.pool, "sticky-cap-logical")
            .await
            .is_some_and(|sticky_account_id| sticky_account_id == account_id),
        "logical stream failure should preserve sticky binding until cooldown begins",
    );

    upstream_handle.abort();
}

#[derive(sqlx::FromRow)]
struct LatestInvocationPayloadRow {
    invoke_id: String,
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    payload: Option<String>,
}

async fn load_latest_invocation_payload_row(
    state: &AppState,
) -> (LatestInvocationPayloadRow, Value) {
    let row = sqlx::query_as::<_, LatestInvocationPayloadRow>(
        r#"
        SELECT invoke_id, status, error_message, failure_kind, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load latest invocation");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("latest invocation payload should exist"),
    )
    .expect("decode latest invocation payload");
    (row, payload)
}

#[tokio::test]
async fn capture_target_pool_route_keeps_late_logical_failure_when_downstream_disconnects() {
    #[derive(sqlx::FromRow)]
    struct InvocationRow {
        invoke_id: String,
        occurred_at: String,
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        payload: Option<String>,
    }

    #[derive(sqlx::FromRow)]
    struct AttemptRow {
        status: String,
        downstream_http_status: Option<i64>,
        failure_kind: Option<String>,
        error_message: Option<String>,
        downstream_error_message: Option<String>,
    }

    let (upstream_base, _attempts, upstream_handle) =
        spawn_pool_late_response_failed_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    record_pool_route_success(
        &state.pool,
        account_id,
        Utc::now(),
        Some("sticky-cap-disconnect"),
        None,
    )
    .await
    .expect("seed sticky route");

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
        "stickyKey": "sticky-cap-disconnect"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    drop(response);

    wait_for_codex_invocations(&state.pool, 1).await;

    let invocation = sqlx::query_as::<_, InvocationRow>(
        r#"
        SELECT invoke_id, occurred_at, status, error_message, failure_kind, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load latest invocation");
    assert_eq!(invocation.status.as_deref(), Some("http_200"));
    assert_eq!(
        invocation.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    );
    assert!(
        invocation
            .error_message
            .as_deref()
            .is_some_and(|value| value.contains("upstream_response_failed"))
    );
    let invocation_payload: Value = serde_json::from_str(
        invocation
            .payload
            .as_deref()
            .expect("latest invocation payload should exist"),
    )
    .expect("decode latest invocation payload");
    assert_eq!(
        invocation_payload["downstreamStatusCode"].as_i64(),
        Some(200)
    );
    assert!(
        invocation_payload["downstreamErrorMessage"]
            .as_str()
            .is_some_and(
                |value| value.contains("downstream closed while streaming upstream response")
            )
    );
    let reloaded =
        load_persisted_api_invocation(&state.pool, &invocation.invoke_id, &invocation.occurred_at)
            .await
            .expect("reload persisted api invocation");
    assert_eq!(reloaded.downstream_status_code, Some(200));
    assert!(
        reloaded.downstream_error_message.as_deref().is_some_and(
            |value| value.contains("downstream closed while streaming upstream response")
        )
    );

    let attempt = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT status, downstream_http_status, failure_kind, error_message, downstream_error_message
        FROM pool_upstream_request_attempts
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load latest pool attempt");
    assert_eq!(
        attempt.status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(attempt.downstream_http_status, Some(200));
    assert_eq!(
        attempt.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    );
    assert!(
        attempt
            .error_message
            .as_deref()
            .is_some_and(|value| value.contains("upstream_response_failed"))
    );
    assert!(
        attempt.downstream_error_message.as_deref().is_some_and(
            |value| value.contains("downstream closed while streaming upstream response")
        )
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_marks_upstream_read_root_cause_for_truncated_encoded_streams() {
    for (mode, encoding) in [
        ("gzip-truncated-stream", "gzip"),
        ("br-truncated-stream", "br"),
        ("deflate-truncated-stream", "deflate"),
    ] {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

        let request_body = serde_json::to_vec(&json!({
            "model": "gpt-5.4",
            "stream": true,
            "input": "hello",
        }))
        .expect("serialize truncated encoded request body");
        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri(
                format!("/v1/responses?mode={mode}")
                    .parse()
                    .expect("valid truncated stream uri"),
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
        let response_invoke_id = response
            .headers()
            .get(CVM_INVOKE_ID_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
            .expect("tracked streaming response should expose x-cvm-invoke-id");
        let err = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect_err("truncated encoded upstream stream should fail after first chunk");
        assert!(
            err.to_string().contains("upstream stream error"),
            "unexpected truncated stream error text: {err}"
        );

        wait_for_codex_invocations(&state.pool, 1).await;
        let (row, payload) = load_latest_invocation_payload_row(state.as_ref()).await;
        assert_eq!(
            row.failure_kind.as_deref(),
            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
            "mode={mode}"
        );
        assert!(
            row.error_message
                .as_deref()
                .is_some_and(|value| value.contains("upstream_stream_error")),
            "mode={mode}"
        );
        assert_eq!(
            payload["streamFailureOrigin"].as_str(),
            Some("upstream_read"),
            "mode={mode}"
        );
        assert_eq!(
            payload["contentEncodingChain"].as_str(),
            Some(encoding),
            "mode={mode}"
        );
        assert_eq!(
            payload["responseContentEncoding"].as_str(),
            Some(encoding),
            "mode={mode}"
        );
        assert_eq!(
            payload["usageObserved"].as_bool(),
            Some(false),
            "mode={mode}"
        );
        assert!(
            payload["upstreamReadErrorKind"].as_str().is_some(),
            "mode={mode} should persist upstream read error kind"
        );
        assert!(
            payload["forwardedChunkCount"]
                .as_u64()
                .is_some_and(|value| value >= 1),
            "mode={mode} should forward at least one chunk before failing"
        );
        assert!(
            payload["usageMissingReason"]
                .as_str()
                .is_some_and(|value| value.contains("response_decode_failed:")),
            "mode={mode} should preserve decode evidence in usageMissingReason"
        );
        assert_eq!(row.invoke_id, response_invoke_id, "mode={mode}");

        upstream_handle.abort();
    }
}

#[tokio::test]
async fn pool_openai_v1_responses_marks_content_decode_root_cause_for_complete_corrupt_streams() {
    for (mode, encoding) in [
        ("gzip-corrupt-complete", "gzip"),
        ("br-corrupt-complete", "br"),
        ("deflate-corrupt-complete", "deflate"),
    ] {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

        let request_body = serde_json::to_vec(&json!({
            "model": "gpt-5.4",
            "stream": true,
            "input": "hello",
        }))
        .expect("serialize corrupt encoded request body");
        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri(
                format!("/v1/responses?mode={mode}")
                    .parse()
                    .expect("valid corrupt stream uri"),
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
            .expect("complete corrupt stream should still transfer bytes");
        assert!(
            !body.is_empty(),
            "mode={mode} should forward raw encoded bytes"
        );

        wait_for_codex_invocations(&state.pool, 1).await;
        let (row, payload) = load_latest_invocation_payload_row(state.as_ref()).await;
        let expected_origin = if encoding == "deflate" {
            "stream_parse"
        } else {
            "content_decode"
        };
        assert_eq!(row.status.as_deref(), Some("success"), "mode={mode}");
        assert_eq!(row.failure_kind.as_deref(), None, "mode={mode}");
        assert_eq!(
            payload["streamFailureOrigin"].as_str(),
            Some(expected_origin),
            "mode={mode}"
        );
        assert_eq!(
            payload["contentEncodingChain"].as_str(),
            Some(encoding),
            "mode={mode}"
        );
        assert_eq!(
            payload["responseContentEncoding"].as_str(),
            Some(encoding),
            "mode={mode}"
        );
        assert_eq!(
            payload["usageObserved"].as_bool(),
            Some(false),
            "mode={mode}"
        );
        assert!(
            payload["usageMissingReason"].as_str().is_some_and(|value| {
                if expected_origin == "content_decode" {
                    value.contains("response_decode_failed:")
                } else {
                    value.contains("stream_event_parse_error")
                }
            }),
            "mode={mode} should record the parser-stage evidence for {expected_origin}"
        );

        upstream_handle.abort();
    }
}

#[tokio::test]
async fn capture_target_pool_route_marks_server_overloaded_after_forward_as_retryable_without_cooldown()
 {
    #[derive(sqlx::FromRow)]
    struct RouteStateRow {
        status: String,
        last_action: Option<String>,
        last_action_reason_code: Option<String>,
        last_action_http_status: Option<i64>,
        cooldown_until: Option<String>,
        last_route_failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_late_response_failed_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "upstream-primary",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;
    record_pool_route_success(
        &state.pool,
        account_id,
        Utc::now(),
        Some("sticky-cap-overloaded-late"),
        None,
    )
    .await
    .expect("seed sticky route");

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
        "stickyKey": "sticky-cap-overloaded-late"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read late overloaded response body");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 late overloaded body");
    assert!(body_text.contains("response.created"));
    assert!(body_text.contains("server_is_overloaded"));

    wait_for_codex_invocations(&state.pool, 1).await;
    let route_state = sqlx::query_as::<_, RouteStateRow>(
        r#"
        SELECT
            status,
            last_action,
            last_action_reason_code,
            last_action_http_status,
            cooldown_until,
            last_route_failure_kind
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load late overloaded route state");
    assert_eq!(route_state.status, "active");
    assert_eq!(route_state.last_action.as_deref(), Some("route_recovered"));
    assert!(route_state.last_action_reason_code.is_none());
    assert!(route_state.last_action_http_status.is_none());
    assert!(route_state.cooldown_until.is_none());
    assert!(route_state.last_route_failure_kind.is_none());
    let model_route = sqlx::query_as::<_, (String, String, i64, Option<String>)>(
        "SELECT state, priority, consecutive_failures, last_failure_kind FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = 'gpt-5.4'",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load overloaded model route");
    assert_eq!(model_route.0, MODEL_ROUTE_STATE_DEGRADED);
    assert_eq!(model_route.1, MODEL_ROUTE_PRIORITY_DEMOTED);
    assert_eq!(model_route.2, 1);
    assert_eq!(
        model_route.3.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    );
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-cap-overloaded-late").await,
        Some(account_id),
        "retryable overload should keep the sticky binding",
    );

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
    .expect("load late overloaded events");
    assert!(
        !recent_actions
            .iter()
            .any(|action| action == "route_cooldown_started")
    );

    let attempts = attempts.lock().expect("lock late overloaded attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    drop(attempts);

    upstream_handle.abort();
}

async fn spawn_raw_completed_then_trailing_upstream() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind raw slow-success upstream");
    let addr = listener
        .local_addr()
        .expect("raw slow-success upstream addr");
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept raw upstream connection");
        stream
            .set_nodelay(true)
            .expect("enable TCP_NODELAY on raw upstream");
        let mut request_bytes = Vec::new();
        let mut read_buf = [0_u8; 1024];
        let mut header_end = None;
        while header_end.is_none() {
            let read = tokio::io::AsyncReadExt::read(&mut stream, &mut read_buf)
                .await
                .expect("read raw upstream request");
            assert!(read > 0, "raw upstream request closed before headers");
            request_bytes.extend_from_slice(&read_buf[..read]);
            header_end = request_bytes
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|index| index + 4);
        }

        let header_end = header_end.expect("raw upstream headers should end");
        let header_text = String::from_utf8_lossy(&request_bytes[..header_end]);
        let content_length = header_text
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        let body_bytes_read = request_bytes.len().saturating_sub(header_end);
        if body_bytes_read < content_length {
            let mut remaining = vec![0_u8; content_length - body_bytes_read];
            tokio::io::AsyncReadExt::read_exact(&mut stream, &mut remaining)
                .await
                .expect("read raw upstream request body");
        }

        let response_head = concat!(
            "HTTP/1.1 200 OK\r\n",
            "content-type: text/event-stream\r\n",
            "transfer-encoding: chunked\r\n",
            "connection: keep-alive\r\n\r\n",
        );
        let first = concat!(
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_slow_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
        );
        let second = concat!(
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_slow_test\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15}}}\n\n",
        );
        let trailing = "event: response.ping\ndata: {}\n\n";
        let first_chunk = format!("{:X}\r\n{first}\r\n", first.len());
        let second_chunk = format!("{:X}\r\n{second}\r\n", second.len());
        let trailing_chunk = format!("{:X}\r\n{trailing}\r\n", trailing.len());

        tokio::io::AsyncWriteExt::write_all(&mut stream, response_head.as_bytes())
            .await
            .expect("write raw upstream response headers");
        tokio::io::AsyncWriteExt::write_all(&mut stream, first_chunk.as_bytes())
            .await
            .expect("write raw upstream first chunk");
        tokio::io::AsyncWriteExt::flush(&mut stream)
            .await
            .expect("flush raw upstream first chunk");

        tokio::time::sleep(Duration::from_millis(1500)).await;

        tokio::io::AsyncWriteExt::write_all(&mut stream, second_chunk.as_bytes())
            .await
            .expect("write raw upstream second chunk");
        tokio::io::AsyncWriteExt::flush(&mut stream)
            .await
            .expect("flush raw upstream completed chunk");

        tokio::time::sleep(Duration::from_millis(300)).await;

        tokio::io::AsyncWriteExt::write_all(&mut stream, trailing_chunk.as_bytes())
            .await
            .expect("write raw upstream trailing chunk");
        tokio::io::AsyncWriteExt::write_all(&mut stream, b"0\r\n\r\n")
            .await
            .expect("write raw upstream terminator");
        tokio::io::AsyncWriteExt::flush(&mut stream)
            .await
            .expect("flush raw upstream final chunk");
    });

    (format!("http://{addr}"), handle)
}

#[tokio::test]
async fn pool_openai_v1_responses_records_post_terminal_downstream_write_error_as_success() {
    let (upstream_base, upstream_handle) = spawn_raw_completed_then_trailing_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let app = Router::new()
        .route("/v1/{*path}", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy test server");
    let addr = listener.local_addr().expect("proxy test server addr");
    let server_handle = tokio::spawn(async move {
        crate::serve_router_with_graceful_shutdown(listener, app, std::future::pending())
            .await
            .expect("proxy test server should run");
    });

    let request_body = serde_json::to_string(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
    }))
    .expect("serialize downstream close request body");
    let mut client = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect downstream close client");
    let http_request = format!(
        "POST /v1/responses?mode=completed-then-trailing HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer pool-live-key\r\nContent-Type: application/json\r\nUser-Agent: codex-test-downstream-close/1.0\r\nX-Forwarded-For: 198.51.100.8, 192.168.31.1\r\nX-Real-IP: 198.51.100.9\r\nForwarded: for=198.51.100.10;proto=https;host=example.test\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
        request_body.len(),
        request_body
    );
    tokio::io::AsyncWriteExt::write_all(&mut client, http_request.as_bytes())
        .await
        .expect("write downstream close request");

    let mut response_bytes = Vec::new();
    let mut read_buf = [0_u8; 1024];
    let read_started = Instant::now();
    loop {
        let read = tokio::io::AsyncReadExt::read(&mut client, &mut read_buf)
            .await
            .expect("read downstream close response");
        assert!(read > 0, "socket closed before response.completed");
        response_bytes.extend_from_slice(&read_buf[..read]);
        if response_bytes
            .windows("response.completed".len())
            .any(|window| window == b"response.completed")
        {
            break;
        }
        assert!(
            read_started.elapsed() < Duration::from_secs(5),
            "timed out waiting for response.completed"
        );
    }
    let completed_chunk_text = String::from_utf8_lossy(&response_bytes);
    assert!(
        completed_chunk_text.contains("response.completed"),
        "expected downstream response to contain response.completed, got: {completed_chunk_text}"
    );
    #[allow(deprecated)]
    client
        .set_linger(Some(Duration::ZERO))
        .expect("enable zero linger for downstream reset");
    drop(client);

    wait_for_codex_invocations(&state.pool, 1).await;
    let (row, payload) = load_latest_invocation_payload_row(state.as_ref()).await;
    assert_eq!(row.status.as_deref(), Some("success"));
    assert!(row.error_message.is_none());
    assert!(row.failure_kind.is_none());
    assert!(payload["streamFailureOrigin"].is_null());
    assert_eq!(payload["upstreamOutcome"].as_str(), Some("completed"));
    assert!(payload["downstreamClosePhase"].is_null());
    assert!(
        matches!(
            payload["postTerminalDownstreamWriteErrorKind"].as_str(),
            Some("broken_pipe" | "connection_reset" | "connection_aborted")
        ),
        "unexpected post-terminal downstream write error kind: {:?}",
        payload["postTerminalDownstreamWriteErrorKind"].as_str()
    );
    let expected_forwarded_bytes = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_slow_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_slow_test\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15}}}\n\n",
    )
    .len() as u64;
    assert!(
        payload["forwardedChunkCount"]
            .as_u64()
            .is_some_and(|value| value >= 2)
    );
    assert!(
        payload["forwardedBytes"]
            .as_u64()
            .is_some_and(|value| value >= expected_forwarded_bytes),
        "forwarded bytes should include both upstream body chunks"
    );
    assert_eq!(payload["usageObserved"].as_bool(), Some(true));
    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, failure_kind, error_message
        FROM pool_upstream_request_attempts
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load downstream-close pool attempt");
    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert!(attempt.1.is_none());
    assert!(attempt.2.is_none());

    server_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn pool_responses_keeps_success_after_completed_then_upstream_read_error() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
    }))
    .expect("serialize post-terminal error request");
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        proxy_openai_v1(
            State(state.clone()),
            OriginalUri(
                "/v1/responses?mode=completed-stream-error"
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
        ),
    )
    .await
    .expect("post-terminal upstream response should start");

    assert_eq!(response.status(), StatusCode::OK);
    let body = tokio::time::timeout(
        Duration::from_secs(5),
        to_bytes(response.into_body(), usize::MAX),
    )
    .await
    .expect("post-terminal response body should close")
    .expect("post-terminal upstream error must not reach downstream");
    assert!(String::from_utf8_lossy(&body).contains("response.completed"));

    wait_for_codex_invocations(&state.pool, 1).await;
    let (row, payload) = load_latest_invocation_payload_row(state.as_ref()).await;
    assert_eq!(row.status.as_deref(), Some("success"));
    assert!(row.error_message.is_none());
    assert!(row.failure_kind.is_none());
    assert_eq!(payload["upstreamOutcome"].as_str(), Some("completed"));
    assert!(payload["streamFailureOrigin"].is_null());
    assert!(payload["upstreamReadErrorKind"].is_null());
    assert!(payload["postTerminalUpstreamReadErrorKind"].is_string());
    assert!(
        payload["postTerminalUpstreamReadErrorMessage"]
            .as_str()
            .is_some_and(|message| message.contains("upstream stream error"))
    );
    let failure_class = sqlx::query_scalar::<_, Option<String>>(
        "SELECT failure_class FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind(&row.invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load persisted post-terminal failure class");
    assert_eq!(failure_class.as_deref(), Some("none"));

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, failure_kind, error_message
        FROM pool_upstream_request_attempts
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load post-terminal pool attempt");
    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert!(attempt.1.is_none());
    assert!(attempt.2.is_none());

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_responses_ignores_body_drop_after_completed_was_forwarded() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
    }))
    .expect("serialize post-terminal body-drop request");
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=slow-success"
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

    let mut body = response.into_body().into_data_stream();
    let mut received = Vec::new();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.expect("read response body through completed");
        received.extend_from_slice(&chunk);
        if received
            .windows("response.completed".len())
            .any(|window| window == b"response.completed")
        {
            break;
        }
    }
    assert!(String::from_utf8_lossy(&received).contains("response.completed"));
    drop(body);

    wait_for_codex_invocations(&state.pool, 1).await;
    let (row, payload) = load_latest_invocation_payload_row(state.as_ref()).await;
    assert_eq!(row.status.as_deref(), Some("success"));
    assert!(row.error_message.is_none());
    assert!(row.failure_kind.is_none());
    assert_eq!(payload["upstreamOutcome"].as_str(), Some("completed"));
    assert!(payload["downstreamErrorMessage"].is_null());
    assert!(payload["downstreamWriteErrorKind"].is_null());
    assert!(payload["postTerminalDownstreamWriteErrorKind"].is_null());

    upstream_handle.abort();
}
