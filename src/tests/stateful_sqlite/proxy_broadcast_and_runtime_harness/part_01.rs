#[test]
pub(crate) fn same_origin_settings_write_rejects_mismatched_origin() {
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
pub(crate) fn same_origin_settings_write_allows_loopback_proxy_port_mismatch() {
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
pub(crate) fn same_origin_settings_write_allows_forwarded_host_match() {
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
pub(crate) fn same_origin_settings_write_allows_forwarded_port_for_non_default_origin_port() {
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
pub(crate) fn same_origin_settings_write_rejects_multi_hop_forwarded_host_chain() {
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
pub(crate) fn same_origin_settings_write_rejects_cross_site_request() {
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
pub(crate) fn rewrite_proxy_location_path_strips_upstream_base_prefix() {
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
pub(crate) fn normalize_proxy_location_header_strips_upstream_base_prefix_for_absolute_redirect() {
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
pub(crate) fn normalize_proxy_location_header_strips_upstream_base_prefix_for_relative_redirect() {
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
pub(crate) async fn proxy_openai_v1_forwards_headers_method_query_and_body() {
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
pub(crate) async fn proxy_openai_v1_replays_non_capture_body_across_429_retries() {
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
pub(crate) async fn proxy_openai_v1_records_stream_error_when_final_429_stream_fails() {
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
pub(crate) async fn proxy_openai_v1_streams_request_body_when_429_retry_is_disabled() {
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
pub(crate) async fn pool_route_non_capture_request_body_read_timeout_applies_to_replay_stream() {
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
pub(crate) async fn proxy_openai_v1_records_end_to_end_latency_for_non_capture_streams() {
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
pub(crate) async fn proxy_openai_v1_returns_final_429_body_and_headers_after_retry_exhaustion() {
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
pub(crate) async fn forward_proxy_penalized_probe_treats_429_as_failure() {
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
pub(crate) async fn forward_proxy_penalized_probe_skips_recording_when_shutdown_begins_mid_probe() {
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
pub(crate) async fn proxy_capture_target_retries_429_then_persists_final_success_once() {
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
pub(crate) async fn proxy_capture_target_returns_final_429_after_retry_exhaustion() {
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
pub(crate) async fn proxy_model_settings_api_reads_and_persists_updates() {
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
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(5),
            websocket_enabled: Some(true),
            upstream_websocket_default_enabled: Some(true),
            request_body_logging_enabled: Some(false),
            response_body_logging_enabled: Some(false),
            encrypted_session_owner_routing_enabled: Some(false),
            enabled_models: vec!["gpt-5.2-codex".to_string(), "unknown-model".to_string()],
        }),
    )
    .await
    .expect("put settings should succeed");
    assert!(updated.hijack_enabled);
    assert!(updated.merge_upstream_enabled);
    assert_eq!(updated.fast_mode_rewrite_mode, "disabled");
    assert_eq!(updated.upstream_429_max_retries, 5);
    assert!(updated.websocket_enabled);
    assert!(updated.upstream_websocket_default_enabled);
    assert!(!updated.request_body_logging_enabled);
    assert!(!updated.response_body_logging_enabled);
    assert!(!updated.encrypted_session_owner_routing_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);

    let persisted = load_proxy_model_settings(&state.pool)
        .await
        .expect("settings should persist");
    assert!(persisted.hijack_enabled);
    assert!(persisted.merge_upstream_enabled);
    assert_eq!(persisted.upstream_429_max_retries, 5);
    assert!(persisted.websocket_enabled);
    assert!(persisted.upstream_websocket_default_enabled);
    assert!(!persisted.request_body_logging_enabled);
    assert!(!persisted.response_body_logging_enabled);
    assert!(!persisted.encrypted_session_owner_routing_enabled);
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
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(9),
            websocket_enabled: Some(false),
            upstream_websocket_default_enabled: Some(false),
            request_body_logging_enabled: Some(true),
            response_body_logging_enabled: Some(true),
            encrypted_session_owner_routing_enabled: Some(true),
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
    assert!(!normalized.websocket_enabled);
    assert!(!normalized.upstream_websocket_default_enabled);
    assert!(normalized.request_body_logging_enabled);
    assert!(normalized.response_body_logging_enabled);
    assert!(normalized.encrypted_session_owner_routing_enabled);
    assert!(normalized.enabled_models.is_empty());
}

use super::*;
