#[test]
fn same_origin_settings_write_rejects_mismatched_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );
    assert!(!is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_loopback_proxy_port_mismatch() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:60080"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_forwarded_host_match() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_forwarded_port_for_non_default_origin_port() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com:8443"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-port"),
        HeaderValue::from_static("8443"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_rejects_multi_hop_forwarded_host_chain() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("evil.example.com, proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    assert!(!is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_rejects_cross_site_request() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("cross-site"),
    );
    assert!(!is_same_origin_settings_write(&headers));
}

#[test]
fn rewrite_proxy_location_path_strips_upstream_base_prefix() {
    let upstream_base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
    assert_eq!(
        rewrite_proxy_location_path("/gateway/v1/echo", &upstream_base),
        "/v1/echo"
    );
    assert_eq!(
        rewrite_proxy_location_path("/v1/echo", &upstream_base),
        "/v1/echo"
    );
}

#[test]
fn normalize_proxy_location_header_strips_upstream_base_prefix_for_absolute_redirect() {
    let upstream_base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::LOCATION,
        HeaderValue::from_static("https://proxy.example.com/gateway/v1/echo?from=redirect"),
    );

    let normalized =
        normalize_proxy_location_header(StatusCode::TEMPORARY_REDIRECT, &headers, &upstream_base)
            .expect("normalize should succeed");
    assert_eq!(normalized.as_deref(), Some("/v1/echo?from=redirect"));
}

#[test]
fn normalize_proxy_location_header_strips_upstream_base_prefix_for_relative_redirect() {
    let upstream_base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::LOCATION,
        HeaderValue::from_static("/gateway/v1/echo?from=redirect#frag"),
    );

    let normalized =
        normalize_proxy_location_header(StatusCode::TEMPORARY_REDIRECT, &headers, &upstream_base)
            .expect("normalize should succeed");
    assert_eq!(normalized.as_deref(), Some("/v1/echo?from=redirect#frag"));
}

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

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from_static(b"hello"))).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = tx.send(Ok(Bytes::from_static(b"-pool"))).await;
    });

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/echo?mode=pool-stream".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
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

#[tokio::test]
async fn proxy_model_settings_api_reads_and_persists_updates() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(updated) = put_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: Some(5),
            enabled_models: vec!["gpt-5.2-codex".to_string(), "unknown-model".to_string()],
        }),
    )
    .await
    .expect("put settings should succeed");
    assert!(updated.hijack_enabled);
    assert!(updated.merge_upstream_enabled);
    assert_eq!(updated.upstream_429_max_retries, 5);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);

    let persisted = load_proxy_model_settings(&state.pool)
        .await
        .expect("settings should persist");
    assert!(persisted.hijack_enabled);
    assert!(persisted.merge_upstream_enabled);
    assert_eq!(persisted.upstream_429_max_retries, 5);
    assert_eq!(
        persisted.enabled_preset_models,
        vec!["gpt-5.2-codex".to_string()]
    );

    let Json(normalized) = put_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: false,
            merge_upstream_enabled: true,
            upstream_429_max_retries: Some(9),
            enabled_models: Vec::new(),
        }),
    )
    .await
    .expect("put settings should normalize payload");
    assert!(!normalized.hijack_enabled);
    assert!(!normalized.merge_upstream_enabled);
    assert_eq!(
        normalized.upstream_429_max_retries,
        MAX_PROXY_UPSTREAM_429_MAX_RETRIES
    );
    assert!(normalized.enabled_models.is_empty());
}

#[tokio::test]
async fn proxy_model_settings_api_preserves_upstream_429_max_retries_when_field_missing() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(updated) = put_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: Some(5),
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("put settings should succeed");
    assert_eq!(updated.upstream_429_max_retries, 5);

    let legacy_payload = serde_json::from_value::<ProxyModelSettingsUpdateRequest>(json!({
        "hijackEnabled": true,
        "mergeUpstreamEnabled": false,
        "fastModeRewriteMode": "fill_missing",
        "enabledModels": ["gpt-5.2-codex"],
    }))
    .expect("legacy payload should deserialize");

    let Json(updated) =
        put_proxy_settings(State(state.clone()), HeaderMap::new(), Json(legacy_payload))
            .await
            .expect("legacy payload should not reset upstream429MaxRetries");
    assert_eq!(updated.upstream_429_max_retries, 5);

    let persisted = load_proxy_model_settings(&state.pool)
        .await
        .expect("settings should persist");
    assert_eq!(persisted.upstream_429_max_retries, 5);
}

#[tokio::test]
async fn ensure_schema_keeps_legacy_fast_mode_rewrite_mode_column_inert() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");

    sqlx::query(
        r#"
        CREATE TABLE proxy_model_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            hijack_enabled INTEGER NOT NULL DEFAULT 0,
            merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
            enabled_preset_models_json TEXT,
            preset_models_migrated INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy proxy_model_settings table");

    sqlx::query(
        r#"
        INSERT INTO proxy_model_settings (
            id,
            hijack_enabled,
            merge_upstream_enabled,
            enabled_preset_models_json,
            preset_models_migrated
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(1_i64)
    .bind(0_i64)
    .bind(
        serde_json::to_string(&default_enabled_preset_models())
            .expect("serialize default enabled models"),
    )
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("insert legacy proxy_model_settings row");

    ensure_schema(&pool)
        .await
        .expect("ensure schema migration run");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert_eq!(
        settings.upstream_429_max_retries,
        DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES
    );
    let columns = sqlx::query("PRAGMA table_info('proxy_model_settings')")
        .fetch_all(&pool)
        .await
        .expect("load proxy_model_settings columns")
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert!(
        columns
            .iter()
            .any(|column| column == "fast_mode_rewrite_mode"),
        "legacy fast_mode_rewrite_mode column should remain for compatibility",
    );
}

#[tokio::test]
async fn ensure_schema_appends_new_proxy_models_when_enabled_list_matches_legacy_default() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let legacy_enabled = LEGACY_PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let legacy_enabled_json =
        serde_json::to_string(&legacy_enabled).expect("serialize legacy enabled list");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force legacy enabled preset models");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.4".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.4-pro".to_string())
    );
}

#[tokio::test]
async fn ensure_schema_does_not_append_new_proxy_models_when_enabled_list_is_custom() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let custom_enabled = vec!["gpt-5.2-codex".to_string()];
    let custom_enabled_json =
        serde_json::to_string(&custom_enabled).expect("serialize custom enabled list");
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(custom_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force custom enabled preset models");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert_eq!(settings.enabled_preset_models, custom_enabled);
}

#[tokio::test]
async fn ensure_schema_allows_opting_out_of_new_proxy_models_after_migration() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let legacy_enabled = LEGACY_PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let legacy_enabled_json =
        serde_json::to_string(&legacy_enabled).expect("serialize legacy enabled list");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(&legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force legacy enabled preset models");

    ensure_schema(&pool)
        .await
        .expect("ensure schema migration run");
    let migrated = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings after migration");
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.4".to_string())
    );
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.4-pro".to_string())
    );

    // User explicitly removes the new models after migration; schema re-run should not
    // force them back in.
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 1
        WHERE id = ?2
        "#,
    )
    .bind(&legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force legacy enabled preset models after migration");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings after opt-out");
    assert_eq!(settings.enabled_preset_models, legacy_enabled);
}

#[tokio::test]
async fn ensure_schema_marks_proxy_preset_models_migrated_when_enabled_list_empty() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind("[]")
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force empty enabled preset models list");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert!(
        settings.enabled_preset_models.is_empty(),
        "empty enabled list should be preserved"
    );

    let migrated = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT preset_models_migrated
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_one(&pool)
    .await
    .expect("read migration flag");
    assert_eq!(migrated, 1);
}

#[tokio::test]
async fn proxy_model_settings_api_rejects_cross_origin_writes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );

    let err = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect_err("cross-origin write should be rejected");

    assert_eq!(err.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn proxy_model_settings_api_rejects_cross_site_request() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("cross-site"),
    );

    let err = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect_err("cross-site request should be rejected");

    assert_eq!(err.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn proxy_model_settings_api_allows_loopback_proxy_origin_mismatch() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:60080"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("loopback proxied write should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
async fn proxy_model_settings_api_allows_forwarded_host_origin_match() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("forwarded host write should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
async fn proxy_model_settings_api_allows_forwarded_port_non_default_origin_port() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com:8443"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-port"),
        HeaderValue::from_static("8443"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("forwarded port write should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
async fn proxy_model_settings_api_allows_matching_origin_without_explicit_host_port() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("same-origin write without explicit host port should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
async fn forward_proxy_live_stats_returns_fixed_24_hour_buckets_with_zero_fill() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(settings_response) = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1080".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");

    let manual_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .map(|node| node.key.clone())
        .expect("manual node should exist");

    let now = Utc::now();
    let range_end_epoch = align_bucket_epoch(now.timestamp(), 3600, 0) + 3600;
    let range_start_epoch = range_end_epoch - 24 * 3600;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        range_start_epoch - 3600,
        4,
        0.25,
        0.42,
        0.34,
        0.35,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        range_start_epoch + 5 * 3600,
        2,
        0.45,
        0.82,
        0.61,
        0.80,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        range_start_epoch + 10 * 3600,
        1,
        1.20,
        1.20,
        1.20,
        1.20,
    )
    .await;
    seed_forward_proxy_hourly_bucket_at(
        &state.pool,
        &manual_key,
        range_start_epoch + 5 * 3600,
        1,
        0,
    )
    .await;
    seed_forward_proxy_hourly_bucket_at(
        &state.pool,
        &manual_key,
        range_start_epoch + 10 * 3600,
        0,
        1,
    )
    .await;
    seed_forward_proxy_hourly_bucket_at(&state.pool, &manual_key, range_start_epoch - 3600, 1, 0)
        .await;

    let Json(response) = fetch_forward_proxy_live_stats(State(state.clone()))
        .await
        .expect("fetch forward proxy live stats should succeed");

    assert_eq!(response.bucket_seconds, 3600);
    assert_eq!(response.nodes.len(), 1);
    assert_eq!(response.range_end, response.nodes[0].last24h[23].bucket_end);
    assert_eq!(
        response.range_start,
        response.nodes[0].last24h[0].bucket_start
    );

    for node in &response.nodes {
        assert_eq!(
            node.last24h.len(),
            24,
            "node {} should include fixed 24 buckets",
            node.key
        );
        assert_eq!(
            node.weight24h.len(),
            24,
            "node {} should include fixed 24 weight buckets",
            node.key
        );
    }

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("manual node should be present");
    let manual_success_total: i64 = manual
        .last24h
        .iter()
        .map(|bucket| bucket.success_count)
        .sum();
    let manual_failure_total: i64 = manual
        .last24h
        .iter()
        .map(|bucket| bucket.failure_count)
        .sum();
    let manual_zero_buckets = manual
        .last24h
        .iter()
        .filter(|bucket| bucket.success_count == 0 && bucket.failure_count == 0)
        .count();
    assert_eq!(
        manual_success_total, 1,
        "out-of-range attempts should be excluded"
    );
    assert!(
        manual_failure_total >= 1,
        "expected at least one in-range failure attempt"
    );
    assert!(
        manual_zero_buckets >= 21,
        "expected most buckets to be zero-filled, got {manual_zero_buckets}"
    );
    assert!(
        manual
            .weight24h
            .iter()
            .any(|bucket| bucket.sample_count == 0 && (bucket.last_weight - 0.35).abs() < 1e-6)
    );

    let sampled_bucket_index = manual
        .weight24h
        .iter()
        .position(|bucket| {
            bucket.sample_count == 2
                && (bucket.min_weight - 0.45).abs() < 1e-6
                && (bucket.max_weight - 0.82).abs() < 1e-6
                && (bucket.avg_weight - 0.61).abs() < 1e-6
                && (bucket.last_weight - 0.80).abs() < 1e-6
        })
        .expect("expected sampled manual weight bucket with aggregated stats");
    let sampled_bucket_carry = manual
        .weight24h
        .get(sampled_bucket_index + 1)
        .expect("expected carry-forward bucket after sampled manual weight bucket");
    assert_eq!(sampled_bucket_carry.sample_count, 0);
    assert!((sampled_bucket_carry.last_weight - 0.80).abs() < 1e-6);

    let recovered_bucket_index = manual
        .weight24h
        .iter()
        .position(|bucket| {
            bucket.sample_count == 1
                && (bucket.min_weight - 1.20).abs() < 1e-6
                && (bucket.max_weight - 1.20).abs() < 1e-6
                && (bucket.avg_weight - 1.20).abs() < 1e-6
                && (bucket.last_weight - 1.20).abs() < 1e-6
        })
        .expect("expected sampled manual weight bucket with recovered last value");
    let recovered_bucket_carry = manual
        .weight24h
        .get(recovered_bucket_index + 1)
        .expect("expected carry-forward bucket after recovered manual weight bucket");
    assert_eq!(recovered_bucket_carry.sample_count, 0);
    assert!((recovered_bucket_carry.last_weight - 1.20).abs() < 1e-6);

    let display_names = response
        .nodes
        .iter()
        .map(|node| node.display_name.as_str())
        .collect::<Vec<_>>();
    let mut sorted_display_names = display_names.clone();
    sorted_display_names.sort();
    assert_eq!(
        display_names, sorted_display_names,
        "live stats nodes should stay display-name sorted"
    );
}

#[tokio::test]
async fn forward_proxy_binding_nodes_preserve_direct_hourly_buckets() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1080".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");

    let now = Utc::now();
    let range_end_epoch = align_bucket_epoch(now.timestamp(), 3600, 0) + 3600;
    let range_start_epoch = range_end_epoch - 24 * 3600;
    seed_forward_proxy_hourly_bucket_at(
        &state.pool,
        FORWARD_PROXY_DIRECT_KEY,
        range_start_epoch + 5 * 3600,
        1,
        0,
    )
    .await;
    seed_forward_proxy_hourly_bucket_at(
        &state.pool,
        FORWARD_PROXY_DIRECT_KEY,
        range_start_epoch + 10 * 3600,
        0,
        1,
    )
    .await;
    seed_forward_proxy_hourly_bucket_at(
        &state.pool,
        FORWARD_PROXY_DIRECT_KEY,
        range_start_epoch - 3600,
        1,
        0,
    )
    .await;

    let extra_proxy_keys = Vec::<String>::new();
    let nodes = build_forward_proxy_binding_nodes_response(state.as_ref(), &extra_proxy_keys)
        .await
        .expect("build forward proxy binding nodes should succeed");
    let direct = nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("direct binding node should be present");

    assert_eq!(direct.protocol_label, "DIRECT");
    assert_eq!(direct.last24h.len(), 24);
    assert_eq!(
        direct
            .last24h
            .iter()
            .map(|bucket| bucket.success_count)
            .sum::<i64>(),
        1,
        "in-range direct successes should remain visible",
    );
    assert_eq!(
        direct
            .last24h
            .iter()
            .map(|bucket| bucket.failure_count)
            .sum::<i64>(),
        1,
        "in-range direct failures should remain visible",
    );
}

#[tokio::test]
async fn forward_proxy_live_stats_returns_empty_nodes_when_no_endpoints_are_configured() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(response) = fetch_forward_proxy_live_stats(State(state))
        .await
        .expect("fetch forward proxy live stats should succeed");

    assert_eq!(response.bucket_seconds, 3600);
    assert!(response.nodes.is_empty());
}

#[tokio::test]
async fn forward_proxy_timeseries_keeps_hourly_attempt_history_after_retention() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(settings_response) = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1081".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");
    let manual_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .map(|node| node.key.clone())
        .expect("manual node should exist");

    let historical_bucket_start =
        align_bucket_epoch((Utc::now() - ChronoDuration::days(45)).timestamp(), 3600, 0);
    let historical_attempt_at = Utc
        .timestamp_opt(historical_bucket_start + 5 * 60, 0)
        .single()
        .expect("historical attempt timestamp should be valid");
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        historical_bucket_start,
        2,
        0.55,
        0.75,
        0.65,
        0.70,
    )
    .await;
    seed_forward_proxy_attempt_at(&state.pool, &manual_key, historical_attempt_at, true).await;
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual_key,
        historical_attempt_at + ChronoDuration::minutes(10),
        false,
    )
    .await;
    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("seed forward proxy hourly rollups before retention");

    let summary = run_data_retention_maintenance(&state.pool, &state.config, Some(false), None)
        .await
        .expect("run retention maintenance");
    assert_eq!(summary.forward_proxy_attempt_rows_archived, 2);

    let live_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND occurred_at < ?2",
    )
    .bind(&manual_key)
    .bind(
        Utc.timestamp_opt(historical_bucket_start + 3_600, 0)
            .single()
            .expect("historical bucket end should be valid")
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    )
    .fetch_one(&state.pool)
    .await
    .expect("count live forward proxy attempts");
    assert_eq!(live_count, 0);

    let Json(response) = fetch_forward_proxy_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "90d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("UTC".to_string()),
        }),
    )
    .await
    .expect("fetch forward proxy timeseries should succeed");

    assert_eq!(response.bucket_seconds, 3600);
    assert_eq!(response.effective_bucket, "1h");
    assert_eq!(response.available_buckets, vec!["1h".to_string()]);

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("manual node should remain queryable");
    let bucket_start = format_utc_iso(
        Utc.timestamp_opt(historical_bucket_start, 0)
            .single()
            .expect("historical bucket start should be valid"),
    );
    let request_bucket = manual
        .buckets
        .iter()
        .find(|bucket| bucket.bucket_start == bucket_start)
        .expect("historical request bucket should be present");
    assert_eq!(request_bucket.success_count, 1);
    assert_eq!(request_bucket.failure_count, 1);
    let weight_bucket = manual
        .weight_buckets
        .iter()
        .find(|bucket| bucket.bucket_start == bucket_start)
        .expect("historical weight bucket should be present");
    assert_eq!(weight_bucket.sample_count, 2);
    assert_f64_close(weight_bucket.min_weight, 0.55);
    assert_f64_close(weight_bucket.max_weight, 0.75);
    assert_f64_close(weight_bucket.avg_weight, 0.65);
    assert_f64_close(weight_bucket.last_weight, 0.70);
}

#[tokio::test]
async fn forward_proxy_timeseries_includes_intersecting_edge_hours() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1082".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
    let manual_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .map(|node| node.key.clone())
        .expect("manual node should exist");

    let bucket0 = align_bucket_epoch((Utc::now() - ChronoDuration::hours(6)).timestamp(), 3600, 0);
    let bucket1 = bucket0 + 3_600;
    let bucket2 = bucket1 + 3_600;
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual_key,
        Utc.timestamp_opt(bucket0 + 30 * 60, 0)
            .single()
            .expect("bucket0 partial attempt timestamp should be valid"),
        true,
    )
    .await;
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual_key,
        Utc.timestamp_opt(bucket1 + 30 * 60, 0)
            .single()
            .expect("bucket1 full attempt timestamp should be valid"),
        true,
    )
    .await;
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual_key,
        Utc.timestamp_opt(bucket2 + 30 * 60, 0)
            .single()
            .expect("bucket2 partial attempt timestamp should be valid"),
        false,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        bucket0,
        1,
        0.80,
        0.80,
        0.80,
        0.80,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        bucket1,
        1,
        0.70,
        0.70,
        0.70,
        0.70,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        bucket2,
        1,
        0.60,
        0.60,
        0.60,
        0.60,
    )
    .await;

    ensure_hourly_rollups_caught_up(state.as_ref())
        .await
        .expect("hourly rollups should sync");

    let range_start = Utc
        .timestamp_opt(bucket0 + 15 * 60, 0)
        .single()
        .expect("range start should be valid");
    let range_end = Utc
        .timestamp_opt(bucket2 + 45 * 60, 0)
        .single()
        .expect("range end should be valid");
    let response = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: range_start,
            end: range_end,
            display_end: range_end,
            duration: range_end - range_start,
        },
    )
    .await
    .expect("forward proxy timeseries should succeed");

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("manual node should remain queryable");
    assert_eq!(response.range_start, format_utc_iso(range_start));
    assert_eq!(response.range_end, format_utc_iso(range_end));
    assert_eq!(manual.buckets.len(), 3);
    assert_eq!(manual.weight_buckets.len(), 3);
    assert_eq!(
        manual
            .buckets
            .iter()
            .map(|bucket| bucket.bucket_start.as_str())
            .collect::<Vec<_>>(),
        vec![
            format_utc_iso(
                Utc.timestamp_opt(bucket0, 0)
                    .single()
                    .expect("leading bucket start should be valid")
            ),
            format_utc_iso(
                Utc.timestamp_opt(bucket1, 0)
                    .single()
                    .expect("middle bucket start should be valid")
            ),
            format_utc_iso(
                Utc.timestamp_opt(bucket2, 0)
                    .single()
                    .expect("trailing bucket start should be valid")
            ),
        ]
    );
    assert_eq!(manual.buckets[0].success_count, 1);
    assert_eq!(manual.buckets[0].failure_count, 0);
    assert_eq!(manual.buckets[1].success_count, 1);
    assert_eq!(manual.buckets[1].failure_count, 0);
    assert_eq!(manual.buckets[2].success_count, 0);
    assert_eq!(manual.buckets[2].failure_count, 1);
    assert_eq!(manual.weight_buckets[0].sample_count, 1);
    assert_f64_close(manual.weight_buckets[0].last_weight, 0.80);
    assert_eq!(manual.weight_buckets[1].sample_count, 1);
    assert_f64_close(manual.weight_buckets[1].last_weight, 0.70);
    assert_eq!(manual.weight_buckets[2].sample_count, 1);
    assert_f64_close(manual.weight_buckets[2].last_weight, 0.60);
}

#[tokio::test]
async fn forward_proxy_timeseries_keeps_single_partial_hour_ranges_non_empty() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1083".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
    let manual_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .map(|node| node.key.clone())
        .expect("manual node should exist");

    let bucket_start_epoch =
        align_bucket_epoch((Utc::now() - ChronoDuration::hours(4)).timestamp(), 3600, 0);
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual_key,
        Utc.timestamp_opt(bucket_start_epoch + 10 * 60, 0)
            .single()
            .expect("partial bucket attempt timestamp should be valid"),
        true,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        bucket_start_epoch,
        1,
        0.58,
        0.58,
        0.58,
        0.58,
    )
    .await;

    ensure_hourly_rollups_caught_up(state.as_ref())
        .await
        .expect("hourly rollups should sync");

    let range_start = Utc
        .timestamp_opt(bucket_start_epoch + 5 * 60, 0)
        .single()
        .expect("range start should be valid");
    let range_end = Utc
        .timestamp_opt(bucket_start_epoch + 20 * 60, 0)
        .single()
        .expect("range end should be valid");
    let response = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: range_start,
            end: range_end,
            display_end: range_end,
            duration: range_end - range_start,
        },
    )
    .await
    .expect("forward proxy timeseries should succeed");

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("manual node should remain queryable");
    assert_eq!(manual.buckets.len(), 1);
    assert_eq!(manual.weight_buckets.len(), 1);
    assert_eq!(manual.buckets[0].success_count, 1);
    assert_eq!(manual.buckets[0].failure_count, 0);
    assert_eq!(manual.weight_buckets[0].sample_count, 1);
    assert_f64_close(manual.weight_buckets[0].last_weight, 0.58);
}

#[tokio::test]
async fn forward_proxy_timeseries_seeds_leading_weight_buckets_from_first_historical_sample() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1084".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
    let manual_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .map(|node| node.key.clone())
        .expect("manual node should exist");

    {
        let mut manager = state.forward_proxy.lock().await;
        let runtime = manager
            .runtime
            .get_mut(&manual_key)
            .expect("manual runtime should exist");
        runtime.weight = 1.45;
    }

    let bucket0 = align_bucket_epoch((Utc::now() - ChronoDuration::hours(6)).timestamp(), 3600, 0);
    let bucket1 = bucket0 + 3_600;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        bucket1,
        1,
        0.35,
        0.35,
        0.35,
        0.35,
    )
    .await;

    let range_start = Utc
        .timestamp_opt(bucket0, 0)
        .single()
        .expect("range start should be valid");
    let range_end = Utc
        .timestamp_opt(bucket1 + 30 * 60, 0)
        .single()
        .expect("range end should be valid");
    let response = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: range_start,
            end: range_end,
            display_end: range_end,
            duration: range_end - range_start,
        },
    )
    .await
    .expect("forward proxy timeseries should succeed");

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("manual node should remain queryable");
    assert_eq!(manual.weight_buckets.len(), 2);
    assert_eq!(manual.weight_buckets[0].sample_count, 0);
    assert_f64_close(manual.weight_buckets[0].last_weight, 0.35);
    assert_eq!(manual.weight_buckets[1].sample_count, 1);
    assert_f64_close(manual.weight_buckets[1].last_weight, 0.35);
}

#[tokio::test]
async fn forward_proxy_timeseries_preserves_retired_proxy_metadata() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let settings_response = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec!["socks5://127.0.0.1:1085".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;
    let manual = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .cloned()
        .expect("manual node should exist");

    let bucket_start =
        align_bucket_epoch((Utc::now() - ChronoDuration::hours(5)).timestamp(), 3600, 0);
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual.key,
        Utc.timestamp_opt(bucket_start + 15 * 60, 0)
            .single()
            .expect("historical attempt timestamp should be valid"),
        true,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual.key,
        bucket_start,
        1,
        0.72,
        0.72,
        0.72,
        0.72,
    )
    .await;

    let _ = apply_forward_proxy_settings_without_bootstrap(
        &state,
        ForwardProxySettings {
            proxy_urls: vec![],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
    )
    .await;

    let range_start = Utc
        .timestamp_opt(bucket_start, 0)
        .single()
        .expect("range start should be valid");
    let range_end = Utc
        .timestamp_opt(bucket_start + 30 * 60, 0)
        .single()
        .expect("range end should be valid");
    let response = build_forward_proxy_timeseries_response(
        state.as_ref(),
        RangeWindow {
            start: range_start,
            end: range_end,
            display_end: range_end,
            duration: range_end - range_start,
        },
    )
    .await
    .expect("forward proxy timeseries should succeed");

    let archived = response
        .nodes
        .iter()
        .find(|node| node.key == manual.key)
        .expect("retired proxy should remain queryable via historical metadata");
    assert_eq!(archived.display_name, manual.display_name);
    assert_eq!(archived.source, manual.source);
    assert_eq!(archived.endpoint_url, manual.endpoint_url);
}

#[tokio::test]
async fn reporting_tz_hour_alignment_rejects_sub_hour_dst_transition_windows() {
    let start = Utc
        .with_ymd_and_hms(2026, 10, 3, 15, 20, 0)
        .single()
        .expect("transition test start should be valid");
    let end = Utc
        .with_ymd_and_hms(2026, 10, 3, 15, 40, 0)
        .single()
        .expect("transition test end should be valid");

    assert!(!reporting_tz_has_whole_hour_offsets(
        chrono_tz::Australia::Lord_Howe,
        &RangeWindow {
            start,
            end,
            display_end: end,
            duration: end - start,
        }
    ));
}

#[tokio::test]
async fn parallel_work_day_all_fallbacks_when_requested_window_is_missing_in_sub_hour_zone() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 7, 12, 0, 0)
        .single()
        .expect("fixed now");
    assert!(should_fallback_parallel_work_day_all_window(
        "Asia/Kolkata".parse::<Tz>().expect("valid kolkata tz"),
        None,
        now,
    ));
    assert!(!should_fallback_parallel_work_day_all_window(
        "UTC".parse::<Tz>().expect("valid utc tz"),
        None,
        now,
    ));
}

#[test]
fn parallel_work_complete_window_preserves_local_hour_across_dst() {
    let reporting_tz = chrono_tz::America::New_York;
    let now = Utc
        .with_ymd_and_hms(2026, 4, 7, 5, 23, 0)
        .single()
        .expect("fixed now");

    let window =
        resolve_complete_parallel_work_window(now, ChronoDuration::days(30), 3_600, reporting_tz)
            .expect("resolve window");

    assert_eq!(
        window.end,
        Utc.with_ymd_and_hms(2026, 4, 7, 5, 0, 0)
            .single()
            .expect("fixed window end")
    );
    assert_eq!(
        window.start,
        Utc.with_ymd_and_hms(2026, 3, 8, 6, 0, 0)
            .single()
            .expect("fixed window start")
    );
    assert_eq!(
        window.end.with_timezone(&reporting_tz).time(),
        window.start.with_timezone(&reporting_tz).time()
    );
}

#[tokio::test]
async fn upsert_forward_proxy_weight_hourly_bucket_keeps_latest_sample_weight() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;
    let proxy_key = "manual://latest-sample-weight";
    let bucket_start_epoch = align_bucket_epoch(Utc::now().timestamp(), 3600, 0);

    upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        proxy_key,
        bucket_start_epoch,
        0.90,
        1_000_000,
    )
    .await
    .expect("seed first weight sample");
    upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        proxy_key,
        bucket_start_epoch,
        1.10,
        2_000_000,
    )
    .await
    .expect("seed second weight sample");
    upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        proxy_key,
        bucket_start_epoch,
        0.70,
        1_500_000,
    )
    .await
    .expect("seed out-of-order weight sample");

    let row = sqlx::query(
        r#"
        SELECT
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight,
            last_sample_epoch_us
        FROM forward_proxy_weight_hourly
        WHERE proxy_key = ?1 AND bucket_start_epoch = ?2
        "#,
    )
    .bind(proxy_key)
    .bind(bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("fetch aggregated weight bucket");

    assert_eq!(
        row.try_get::<i64, _>("sample_count").expect("sample_count"),
        3
    );
    assert!((row.try_get::<f64, _>("min_weight").expect("min_weight") - 0.70).abs() < 1e-6);
    assert!((row.try_get::<f64, _>("max_weight").expect("max_weight") - 1.10).abs() < 1e-6);
    assert!((row.try_get::<f64, _>("avg_weight").expect("avg_weight") - 0.90).abs() < 1e-6);
    assert!((row.try_get::<f64, _>("last_weight").expect("last_weight") - 1.10).abs() < 1e-6);
    assert_eq!(
        row.try_get::<i64, _>("last_sample_epoch_us")
            .expect("last_sample_epoch_us"),
        2_000_000
    );
}

#[tokio::test]
async fn pricing_settings_api_reads_and_persists_updates() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(initial) = get_settings(State(state.clone()))
        .await
        .expect("get settings should succeed");
    assert!(!initial.pricing.entries.is_empty());
    assert!(
        initial
            .pricing
            .entries
            .iter()
            .any(|entry| entry.model == "gpt-5.2-codex")
    );

    let Json(updated) = put_pricing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "custom-ci".to_string(),
            entries: vec![PricingEntry {
                model: "gpt-5.2-codex".to_string(),
                input_per_1m: 8.8,
                output_per_1m: 18.8,
                cache_input_per_1m: Some(0.88),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            }],
        }),
    )
    .await
    .expect("put pricing settings should succeed");

    assert_eq!(updated.catalog_version, "custom-ci");
    assert_eq!(updated.entries.len(), 1);
    assert_eq!(updated.entries[0].model, "gpt-5.2-codex");
    assert_eq!(updated.entries[0].input_per_1m, 8.8);

    let persisted = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing settings should persist");
    assert_eq!(persisted.version, "custom-ci");
    assert_eq!(persisted.models.len(), 1);
    let pricing = persisted
        .models
        .get("gpt-5.2-codex")
        .expect("gpt-5.2-codex should persist");
    assert_eq!(pricing.input_per_1m, 8.8);
    assert_eq!(pricing.output_per_1m, 18.8);
}

