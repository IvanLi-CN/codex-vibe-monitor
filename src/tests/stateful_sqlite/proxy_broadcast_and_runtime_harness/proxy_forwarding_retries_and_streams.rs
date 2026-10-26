use super::*;
use serde_json::json;

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_forwards_headers_method_query_and_body() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer test-token"),
    );
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("client.example.com"),
    );
    headers.insert(
        http_header::CONNECTION,
        HeaderValue::from_static("keep-alive, x-foo"),
    );
    headers.insert(
        http_header::HeaderName::from_static("x-foo"),
        HeaderValue::from_static("should-not-forward"),
    );
    headers.insert(
        http_header::HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static("198.51.100.20"),
    );
    headers.insert(
        http_header::HeaderName::from_static("via"),
        HeaderValue::from_static("1.1 browser-proxy"),
    );
    headers.insert(
        http_header::ACCEPT_ENCODING,
        HeaderValue::from_static("gzip, br"),
    );

    let uri: Uri = "/v1/echo?foo=bar".parse().expect("valid uri");
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(uri),
        Method::POST,
        headers,
        Body::from("hello-proxy"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers().get("x-upstream"),
        Some(&HeaderValue::from_static("ok"))
    );
    assert!(response.headers().contains_key(http_header::CONTENT_LENGTH));
    assert!(
        !response
            .headers()
            .contains_key(http_header::HeaderName::from_static("x-upstream-hop"))
    );
    assert!(
        !response
            .headers()
            .contains_key(http_header::HeaderName::from_static("via"))
    );
    assert!(
        !response
            .headers()
            .contains_key(http_header::HeaderName::from_static("forwarded"))
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
    assert_eq!(payload["method"], "POST");
    assert_eq!(payload["path"], "/v1/echo");
    assert_eq!(payload["query"], "foo=bar");
    assert_eq!(payload["authorization"], "Bearer test-token");
    assert_ne!(payload["hostHeader"], "client.example.com");
    assert_eq!(payload["connectionSeen"], false);
    assert_eq!(payload["xFooSeen"], false);
    assert_eq!(payload["xForwardedForSeen"], false);
    assert_eq!(payload["forwardedSeen"], false);
    assert_eq!(payload["viaSeen"], false);
    assert_eq!(payload["acceptEncoding"], "gzip, br");
    assert_eq!(payload["body"], "hello-proxy");

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_replays_non_capture_body_across_429_retries() {
    let (upstream_base, attempts, seen_bodies, upstream_handle) =
        spawn_retrying_echo_upstream(1, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 1;
    }

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/echo?mode=retry".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from("retry-body"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
    assert_eq!(payload["attempt"], 2);
    assert_eq!(payload["query"], "mode=retry");
    assert_eq!(payload["body"], "retry-body");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        seen_bodies
            .lock()
            .expect("lock retrying echo bodies")
            .clone(),
        vec!["retry-body".to_string(), "retry-body".to_string()]
    );
    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 2);
    assert_eq!(
        count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await,
        1
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_records_stream_error_when_final_429_stream_fails() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 1;
    }

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/429-mid-error".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("upstream 429 stream should fail mid-body");
    assert!(
        err.to_string().contains("upstream stream error"),
        "unexpected stream error text: {err}"
    );

    let mut attempt_count: i64 = 0;
    let mut rate_limit_count: i64 = 0;
    let mut stream_error_count: i64 = 0;
    for _ in 0..20 {
        attempt_count = count_request_forward_proxy_attempts(&state.pool).await;
        rate_limit_count = count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await;
        stream_error_count = count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_STREAM_ERROR,
        )
        .await;

        if attempt_count == 2 && rate_limit_count == 1 && stream_error_count == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert_eq!(attempt_count, 2);
    assert_eq!(rate_limit_count, 1);
    assert_eq!(stream_error_count, 1);

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_streams_request_body_when_429_retry_is_disabled() {
    let (upstream_base, _attempts, _seen_bodies, upstream_handle) =
        spawn_retrying_echo_upstream(0, None).await;
    let state = test_state_with_openai_base_body_limit_and_read_timeout(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(50),
    )
    .await;

    let uri: Uri = "/v1/echo?mode=slow-body".parse().expect("valid uri");

    // Disabled => should keep legacy stream-through semantics (no eager buffering).
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 0;
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from_static(b"hello"))).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = tx.send(Ok(Bytes::from_static(b"-proxy"))).await;
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(uri.clone()),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
    assert_eq!(payload["body"], "hello-proxy");

    // Enabled => buffering path enforces OPENAI_PROXY_REQUEST_READ_TIMEOUT.
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 1;
    }
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from_static(b"hello"))).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = tx.send(Ok(Bytes::from_static(b"-proxy"))).await;
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(uri),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
    )
    .await;
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_non_capture_request_body_read_timeout_applies_to_replay_stream() {
    let (upstream_base, _attempts, _seen_bodies, upstream_handle) =
        spawn_retrying_echo_upstream(0, None).await;
    let state = test_state_with_openai_base_body_limit_and_read_timeout(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(50),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    // Delay on body polling so admission cannot prefill the timeout fixture.
    let body = stream::once(async { Ok::<_, io::Error>(Bytes::from_static(b"hello")) }).chain(
        stream::once(async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Ok::<_, io::Error>(Bytes::from_static(b"-pool"))
        }),
    );

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/echo?mode=pool-stream".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from_stream(body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode pool timeout payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("request body read timed out")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_records_end_to_end_latency_for_non_capture_streams() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 1;
    }

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/slow-stream".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    assert_eq!(body, Bytes::from_static(b"chunk-achunk-b"));

    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 1);
    let latency_ms = latest_request_forward_proxy_attempt_latency_ms(&state.pool)
        .await
        .expect("latency should be recorded");
    assert!(
        latency_ms >= 350.0,
        "expected end-to-end latency to include streaming delay, got {latency_ms}ms"
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_returns_final_429_body_and_headers_after_retry_exhaustion() {
    let (upstream_base, attempts, seen_bodies, upstream_handle) =
        spawn_retrying_echo_upstream(99, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 2;
    }

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/echo?mode=always-429".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from("retry-body"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers().get(http_header::RETRY_AFTER),
        Some(&HeaderValue::from_static("0"))
    );
    assert_eq!(
        response
            .headers()
            .get(http_header::HeaderName::from_static("x-upstream-attempt")),
        Some(&HeaderValue::from_static("3"))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
    assert_eq!(payload["attempt"], 3);
    assert_eq!(payload["body"], "retry-body");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(
        seen_bodies
            .lock()
            .expect("lock retrying echo bodies")
            .clone(),
        vec![
            "retry-body".to_string(),
            "retry-body".to_string(),
            "retry-body".to_string(),
        ]
    );
    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 3);
    assert_eq!(
        count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await,
        3
    );
    // 429 failures should not trigger penalized-proxy probes (they ignore Retry-After and add load).
    let probe_guard_started = Instant::now();
    loop {
        let probe_attempts =
            count_forward_proxy_probe_attempts(&state.pool, FORWARD_PROXY_DIRECT_KEY, None).await;
        assert_eq!(
            probe_attempts, 0,
            "unexpected penalized-proxy probe attempt spawned after upstream 429 exhaustion"
        );
        if probe_guard_started.elapsed() > Duration::from_millis(500) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    upstream_handle.abort();
}

#[tokio::test]
async fn forward_proxy_penalized_probe_treats_429_as_failure() {
    let (upstream_base, upstream_handle) =
        spawn_test_forward_proxy_status(StatusCode::TOO_MANY_REQUESTS).await;
    let state = test_state_with_openai_base(
        Url::parse(&format!("{upstream_base}/")).expect("valid upstream base url"),
    )
    .await;
    let candidate = SelectedForwardProxy::from_endpoint(&ForwardProxyEndpoint::direct());

    spawn_penalized_forward_proxy_probe(state.clone(), candidate.clone());
    wait_for_forward_proxy_probe_attempts(&state.pool, &candidate.key, 1).await;
    assert_eq!(
        count_forward_proxy_probe_attempts(&state.pool, &candidate.key, Some(false)).await,
        1
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn forward_proxy_penalized_probe_skips_recording_when_shutdown_begins_mid_probe() {
    let request_started = Arc::new(Notify::new());
    let release_request = Arc::new(Notify::new());
    let (proxy_url, proxy_handle) = spawn_test_blocking_forward_proxy_status(
        StatusCode::OK,
        request_started.clone(),
        release_request.clone(),
    )
    .await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize forward proxy url");
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize forward proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid upstream base url"),
    )
    .await;
    let endpoint = ForwardProxyEndpoint {
        key: proxy_key.clone(),
        source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
        display_name: normalized_proxy.clone(),
        protocol: ForwardProxyProtocol::Http,
        endpoint_url: Some(Url::parse(&normalized_proxy).expect("valid normalized proxy url")),
        raw_url: Some(normalized_proxy.clone()),
    };

    spawn_penalized_forward_proxy_probe(
        state.clone(),
        SelectedForwardProxy::from_endpoint(&endpoint),
    );

    tokio::time::timeout(Duration::from_secs(1), request_started.notified())
        .await
        .expect("penalized probe should reach the forward proxy before shutdown");
    state.shutdown.cancel();
    release_request.notify_waiters();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await,
        0,
        "shutdown should stop an in-flight penalized probe without recording a probe attempt"
    );

    proxy_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_retries_429_then_persists_final_success_once() {
    let (upstream_base, attempts, seen_payloads, upstream_handle) =
        spawn_retrying_capture_upstream(1, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 1;
    }

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "input": "hello"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response body");
    assert_eq!(payload["attempt"], 2);
    assert_eq!(payload["received"]["input"], "hello");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        seen_payloads
            .lock()
            .expect("lock retrying capture payloads")
            .clone(),
        vec![
            json!({
                "model": "gpt-5.3-codex",
                "stream": false,
                "input": "hello"
            }),
            json!({
                "model": "gpt-5.3-codex",
                "stream": false,
                "input": "hello"
            }),
        ]
    );
    wait_for_codex_invocations(&state.pool, 1).await;
    assert_eq!(count_codex_invocations(&state.pool).await, 1);
    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 2);
    assert_eq!(
        count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await,
        1
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_returns_final_429_after_retry_exhaustion() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
        payload: Option<String>,
    }

    let (upstream_base, attempts, seen_payloads, upstream_handle) =
        spawn_retrying_capture_upstream(99, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 2;
    }

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "input": "hello"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers().get(http_header::RETRY_AFTER),
        Some(&HeaderValue::from_static("0"))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response body");
    assert_eq!(payload["attempt"], 3);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(
        seen_payloads
            .lock()
            .expect("lock retrying capture payloads")
            .len(),
        3
    );
    wait_for_codex_invocations(&state.pool, 1).await;
    assert_eq!(count_codex_invocations(&state.pool).await, 1);
    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, failure_kind, failure_class, is_actionable, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");
    assert_eq!(row.status.as_deref(), Some("http_429"));
    assert_eq!(
        row.failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert_eq!(row.failure_class.as_deref(), Some(FAILURE_CLASS_SERVICE));
    assert_eq!(row.is_actionable, Some(1));
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("upstream_http_429")
    );
    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 3);
    assert_eq!(
        count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await,
        3
    );

    upstream_handle.abort();
}
