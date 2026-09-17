#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_marks_upstream_read_root_cause_for_truncated_encoded_streams()
 {
    for (mode, encoding) in [
        ("gzip-truncated-stream", "gzip"),
        ("br-truncated-stream", "br"),
        ("deflate-truncated-stream", "deflate"),
    ] {
        assert_truncated_encoded_stream(mode, encoding).await;
    }
}

async fn assert_truncated_encoded_stream(mode: &str, encoding: &str) {
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

    assert_truncated_stream_persistence(state, mode, encoding, &response_invoke_id).await;

    upstream_handle.abort();
}

async fn assert_truncated_stream_persistence(
    state: Arc<AppState>,
    mode: &str,
    encoding: &str,
    response_invoke_id: &str,
) {
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
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_marks_content_decode_root_cause_for_complete_corrupt_streams()
 {
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
pub(crate) async fn capture_target_pool_route_marks_server_overloaded_after_forward_as_retryable_without_cooldown()
 {
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
    assert_overloaded_route_state(&state.pool, account_id).await;

    let attempts = attempts.lock().expect("lock late overloaded attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    drop(attempts);

    upstream_handle.abort();
}

#[derive(sqlx::FromRow)]
struct OverloadedRouteStateRow {
    status: String,
    last_action: Option<String>,
    last_action_reason_code: Option<String>,
    last_action_http_status: Option<i64>,
    cooldown_until: Option<String>,
    last_route_failure_kind: Option<String>,
}

async fn assert_overloaded_route_state(pool: &Pool<Sqlite>, account_id: i64) {
    let route_state = sqlx::query_as::<_, OverloadedRouteStateRow>(
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
    .fetch_one(pool)
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
    .fetch_one(pool)
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
        load_test_sticky_route_account_id(pool, "sticky-cap-overloaded-late").await,
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
    .fetch_all(pool)
    .await
    .expect("load late overloaded events");
    assert!(
        !recent_actions
            .iter()
            .any(|action| action == "route_cooldown_started")
    );
}

pub(crate) async fn spawn_raw_completed_then_trailing_upstream() -> (String, JoinHandle<()>) {
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
        serve_raw_completed_then_trailing(&mut stream).await;
    });

    (format!("http://{addr}"), handle)
}

async fn serve_raw_completed_then_trailing(mut stream: &mut tokio::net::TcpStream) {
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
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_records_post_terminal_downstream_write_error_as_success()
 {
    let (upstream_base, upstream_handle) = spawn_raw_completed_then_trailing_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
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

    disconnect_after_response_completed(addr).await;

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

async fn disconnect_after_response_completed(addr: std::net::SocketAddr) {
    let request_body = serde_json::to_string(&json!({
        "model": "gpt-5.4", "stream": true, "input": "hello",
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
            .windows(18)
            .any(|window| window == b"response.completed")
        {
            break;
        }
        assert!(read_started.elapsed() < Duration::from_secs(5));
    }
    assert!(String::from_utf8_lossy(&response_bytes).contains("response.completed"));
    set_zero_linger(&client);
}

#[cfg(unix)]
fn set_zero_linger(client: &tokio::net::TcpStream) {
    use std::os::fd::AsRawFd;

    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    let result = unsafe {
        libc::setsockopt(
            client.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            (&raw const linger).cast(),
            std::mem::size_of_val(&linger) as libc::socklen_t,
        )
    };
    assert_eq!(
        result,
        0,
        "enable zero linger: {}",
        std::io::Error::last_os_error()
    );
}

#[tokio::test]
pub(crate) async fn pool_responses_keeps_success_after_completed_then_upstream_read_error() {
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
pub(crate) async fn pool_responses_ignores_body_drop_after_completed_was_forwarded() {
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

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_openai_v1_e2e_stream_survives_short_request_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state = build_short_timeout_proxy_state(&upstream_base).await;
    assert_short_timeout_stream(state).await;
    upstream_handle.abort();
}

async fn build_short_timeout_proxy_state(upstream_base: &str) -> Arc<AppState> {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    Arc::new(AppState {
        config: config.clone(),
        sqlite_batch_writer: SqliteBatchWriter::spawn_for_test(),
        pool_account_selection_runtime: Arc::new(PoolAccountSelectionRuntime::default()),
        proxy_runtime_invocations: Arc::new(ProxyRuntimeInvocationStore::default()),
        pool,
        oauth_installation_seed: [0_u8; 32],
        http_clients,
        broadcaster,
        subscription_hub: Arc::new(crate::SubscriptionHub::new()),
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        dashboard_activity_live_broadcast_seq: Arc::new(AtomicU64::new(0)),
        dashboard_activity_live_broadcast_running: Arc::new(AtomicBool::new(false)),
        process_started_at_utc: chrono::Utc::now(),
        dashboard_network_speed_cache: Arc::new(
            crate::dashboard_network_speed::DashboardNetworkSpeedCache::new(chrono::Utc::now()),
        ),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
        proxy_raw_async_semaphore: Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config))),
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
        dashboard_activity_snapshot_cache: Arc::new(Mutex::new(
            DashboardActivitySnapshotCacheState::default(),
        )),
        terminal_projection_hub: Arc::new(crate::TerminalProjectionHub::default()),
        long_term_projection_runtime: Arc::new(Mutex::new(
            crate::LongTermProjectionRuntime::default(),
        )),
        memory_diagnostics: Arc::new(crate::MemoryDiagnosticsRuntime::default()),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        system_status_cache: Arc::new(Mutex::new(SystemStatusCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_availability: PoolRoutingAvailabilitySignal::default(),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        pool_routing_test_data_version_connection: Arc::new(Mutex::new(None)),
        pool_model_routing_cache_write_lock: Arc::new(Mutex::new(())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        pool_group_429_retry_delay_override: None,
        fallback_proxy_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    })
}

async fn assert_short_timeout_stream(state: Arc<AppState>) {
    let app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy test server");
    let addr = listener.local_addr().expect("proxy test server addr");
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("proxy test server should run");
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{addr}/v1/slow-stream"))
        .send()
        .await
        .expect("send proxy stream request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.bytes().await.expect("read proxied stream");
    assert_eq!(&body[..], b"chunk-achunk-b");

    server_handle.abort();
}

#[test]
pub(crate) fn pool_openai_v1_e2e_stream_survives_short_request_timeout() {
    run_oauth_future_with_large_stack(async move {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse(&upstream_base).expect("valid upstream base url");
        config.request_timeout = Duration::from_millis(200);
        let state = test_state_from_config(config, true).await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

        let app = Router::new()
            .route("/v1/*path", any(proxy_openai_v1))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy test server");
        let addr = listener.local_addr().expect("proxy test server addr");
        let server_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("proxy test server should run");
        });

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://{addr}/v1/slow-stream"))
            .header(http_header::AUTHORIZATION, "Bearer pool-live-key")
            .send()
            .await
            .expect("send pool proxy stream request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.bytes().await.expect("read proxied stream");
        assert_eq!(&body[..], b"chunk-achunk-b");

        server_handle.abort();
        upstream_handle.abort();
    });
}

#[test]
pub(crate) fn pool_openai_v1_times_out_before_first_chunk_with_short_request_timeout() {
    run_oauth_future_with_large_stack(async move {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse(&upstream_base).expect("valid upstream base url");
        config.request_timeout = Duration::from_millis(200);
        let state = test_state_from_config(config, true).await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

        let app = Router::new()
            .route("/v1/*path", any(proxy_openai_v1))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy test server");
        let addr = listener.local_addr().expect("proxy test server addr");
        let server_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("proxy test server should run");
        });

        let client = reqwest::Client::new();
        let response = tokio::time::timeout(
            Duration::from_secs(3),
            client
                .get(format!("http://{addr}/v1/slow-first-chunk"))
                .header(http_header::AUTHORIZATION, "Bearer pool-live-key")
                .send(),
        )
        .await
        .expect("pool request should not hang")
        .expect("send pool proxy request");

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = response.bytes().await.expect("read pool error body");
        let payload = String::from_utf8_lossy(&body);
        assert!(payload.contains("first upstream chunk"));

        server_handle.abort();
        upstream_handle.abort();
    });
}

use super::*;
