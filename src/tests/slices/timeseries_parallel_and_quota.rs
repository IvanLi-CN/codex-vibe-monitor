#[test]
fn pool_same_account_attempt_budget_keeps_legacy_budget_for_non_responses_routes() {
    assert_eq!(
        pool_same_account_attempt_budget(
            &"/v1/chat/completions".parse().expect("valid uri"),
            &Method::POST,
            1,
            2,
        ),
        2
    );
    assert_eq!(
        pool_same_account_attempt_budget(
            &"/v1/chat/completions".parse().expect("valid uri"),
            &Method::POST,
            2,
            2,
        ),
        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS
    );
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_responses_uses_dedicated_first_byte_timeout() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(400);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(100);
    let state = test_state_from_config(config, true).await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "input": "hello"
    }))
    .expect("serialize responses request body");

    let response = proxy_openai_v1(
        State(state),
        OriginalUri(
            "/v1/responses?mode=slow-first-chunk"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("responses body should time out before the first chunk");
    assert!(
        err.to_string()
            .contains("request timed out after 100ms while waiting for first upstream chunk")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_allows_slow_upload_with_short_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(100);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        oauth_installation_seed: [0_u8; 32],
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
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
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        pool_group_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    });

    let slow_chunks = stream::unfold(0u8, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"hello-")), 1))
            }
            1 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"slow-")), 2))
            }
            2 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"upload")), 3))
            }
            _ => None,
        }
    });

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/echo?mode=slow-upload".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(slow_chunks),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response payload");
    assert_eq!(payload["query"], "mode=slow-upload");
    assert_eq!(payload["body"], "hello-slow-upload");

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_e2e_http_roundtrip() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

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
        .post(format!("http://{addr}/v1/echo?foo=e2e"))
        .header(http_header::AUTHORIZATION, "Bearer e2e-token")
        .body("hello-e2e")
        .send()
        .await
        .expect("send proxy request");

    assert_eq!(response.status(), StatusCode::CREATED);
    let payload: Value = response
        .json()
        .await
        .expect("decode proxied upstream payload");
    assert_eq!(payload["method"], "POST");
    assert_eq!(payload["path"], "/v1/echo");
    assert_eq!(payload["query"], "foo=e2e");
    assert_eq!(payload["authorization"], "Bearer e2e-token");
    assert_eq!(payload["body"], "hello-e2e");

    server_handle.abort();
    upstream_handle.abort();
}

#[derive(Clone)]
struct PoolRetryUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
    fail_before_success: Arc<HashMap<String, usize>>,
}

#[derive(Clone)]
struct PoolRateLimitResponsesUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
    rate_limit_attempts: Arc<HashMap<String, usize>>,
}

#[derive(Clone)]
struct PoolRateLimitEchoUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
    rate_limit_attempts: Arc<HashMap<String, usize>>,
}

#[derive(Clone)]
struct PoolStaticFailureResponsesUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
    statuses: Arc<HashMap<String, StatusCode>>,
}

#[derive(Clone)]
struct PoolSequentialFailureResponsesUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
    statuses_by_attempt: Arc<HashMap<String, Vec<StatusCode>>>,
}

#[derive(Clone)]
struct PoolCompactUnsupportedUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
}

#[derive(Clone)]
struct PoolFirstChunkRetryUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
    fail_before_success: Arc<HashMap<String, usize>>,
}

#[derive(Clone)]
struct PoolResponseFailedRetryUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
    fail_before_success: Arc<HashMap<String, usize>>,
}

#[derive(Clone)]
struct PoolLateResponseFailedUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
}

#[derive(Clone)]
struct PoolDelayedFirstChunkUpstreamState {
    first_chunk_delay: Duration,
}

#[derive(Clone)]
struct PoolDelayedHeadersUpstreamState {
    header_delay: Duration,
}

#[derive(Clone)]
struct PoolDelayedHeadersRetryUpstreamState {
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
    fail_before_success: Arc<HashMap<String, usize>>,
    header_delay: Duration,
}

#[derive(Clone)]
struct PoolHttpFailureUpstreamState {
    status: StatusCode,
    error_code: Option<String>,
    error_message: String,
}

async fn pool_retry_upstream(
    State(state): State<PoolRetryUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state.attempts.lock().expect("lock pool retry attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    if attempt
        <= state
            .fail_before_success
            .get(&authorization)
            .copied()
            .unwrap_or(0)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "authorization": authorization,
                "attempt": attempt,
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn pool_rate_limit_responses_upstream(
    State(state): State<PoolRateLimitResponsesUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock pool rate-limit attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    if attempt
        <= state
            .rate_limit_attempts
            .get(&authorization)
            .copied()
            .unwrap_or(0)
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": {
                    "code": "rate_limit_exceeded",
                    "message": format!("rate limited for {authorization}"),
                }
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn pool_rate_limit_echo_upstream(
    State(state): State<PoolRateLimitEchoUpstreamState>,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    body: String,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock pool echo rate-limit attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    if attempt
        <= state
            .rate_limit_attempts
            .get(&authorization)
            .copied()
            .unwrap_or(0)
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": {
                    "code": "rate_limit_exceeded",
                    "message": format!("rate limited for {authorization}"),
                },
                "attempt": attempt,
            })),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(json!({
            "attempt": attempt,
            "authorization": authorization,
            "method": method.as_str(),
            "path": uri.path(),
            "query": uri.query().unwrap_or_default(),
            "body": body,
        })),
    )
        .into_response()
}

async fn pool_static_failure_responses_upstream(
    State(state): State<PoolStaticFailureResponsesUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock pool static failure attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    let status = state
        .statuses
        .get(&authorization)
        .copied()
        .unwrap_or(StatusCode::OK);
    if !status.is_success() {
        let (error_code, error_message) = if status == StatusCode::TOO_MANY_REQUESTS {
            (
                "rate_limit_exceeded",
                format!("rate limited for {authorization}"),
            )
        } else {
            (
                "server_error",
                format!("upstream failure for {authorization}"),
            )
        };
        return (
            status,
            Json(json!({
                "error": {
                    "code": error_code,
                    "message": error_message,
                },
                "attempt": attempt,
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn pool_sequential_failure_responses_upstream(
    State(state): State<PoolSequentialFailureResponsesUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock pool sequential failure attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    let status = state
        .statuses_by_attempt
        .get(&authorization)
        .and_then(|statuses| statuses.get(attempt.saturating_sub(1)))
        .copied()
        .unwrap_or(StatusCode::OK);
    if !status.is_success() {
        let (error_code, error_message) = if status == StatusCode::TOO_MANY_REQUESTS {
            (
                "rate_limit_exceeded",
                format!("rate limited for {authorization}"),
            )
        } else {
            (
                "server_error",
                format!("upstream failure for {authorization}"),
            )
        };
        return (
            status,
            Json(json!({
                "error": {
                    "code": error_code,
                    "message": error_message,
                },
                "attempt": attempt,
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn pool_compact_unsupported_upstream(
    State(state): State<PoolCompactUnsupportedUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock compact unsupported attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": {
                "code": "channel_unavailable",
                "message": "No available channel for model gpt-5.4-openai-compact under group default (distributor)",
            },
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn pool_first_chunk_retry_upstream(
    State(state): State<PoolFirstChunkRetryUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock pool first chunk retry attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    if attempt
        <= state
            .fail_before_success
            .get(&authorization)
            .copied()
            .unwrap_or(0)
    {
        let stream = futures_util::stream::once(async {
            Err::<Bytes, io::Error>(io::Error::other("first-chunk-boom"))
        });
        return Response::builder()
            .status(StatusCode::OK)
            .body(Body::from_stream(stream))
            .expect("build first chunk error response");
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn pool_response_failed_retry_upstream(
    State(state): State<PoolResponseFailedRetryUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock pool response.failed retry attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    if attempt
        <= state
            .fail_before_success
            .get(&authorization)
            .copied()
            .unwrap_or(0)
    {
        let payload = [
            "event: response.failed\n",
            r#"data: {"type":"response.failed","response":{"id":"resp_overloaded_retry","model":"gpt-5.4","status":"failed","error":{"code":"server_is_overloaded","message":"Our servers are currently overloaded. Please try again later."}}}"#,
            "\n\n",
        ]
        .concat();
        return (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            )],
            Body::from(payload),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn pool_metadata_prefixed_response_failed_retry_upstream(
    State(state): State<PoolResponseFailedRetryUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock metadata-prefixed response.failed retry attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    if attempt
        <= state
            .fail_before_success
            .get(&authorization)
            .copied()
            .unwrap_or(0)
    {
        let payload = [
            "event: response.created\n",
            r#"data: {"type":"response.created","response":{"id":"resp_overloaded_retry","model":"gpt-5.4","status":"in_progress"}}"#,
            "\n\n",
            "event: response.failed\n",
            r#"data: {"type":"response.failed","response":{"id":"resp_overloaded_retry","model":"gpt-5.4","status":"failed","error":{"code":"server_is_overloaded","message":"Our servers are currently overloaded. Please try again later."}}}"#,
            "\n\n",
        ]
        .concat();
        return (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            )],
            Body::from(payload),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn pool_compact_overloaded_retry_upstream(
    State(state): State<PoolResponseFailedRetryUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock compact overloaded retry attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    if attempt
        <= state
            .fail_before_success
            .get(&authorization)
            .copied()
            .unwrap_or(0)
    {
        return (
            StatusCode::OK,
            Json(json!({
                "error": {
                    "code": "server_is_overloaded",
                    "message": "Our servers are currently overloaded. Please try again later.",
                    "request_id": format!("compact-overloaded-{attempt}"),
                }
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn pool_late_response_failed_upstream(
    State(state): State<PoolLateResponseFailedUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock pool late response.failed attempts");
        let entry = attempts.entry(authorization).or_insert(0);
        *entry += 1;
    }

    let payload = [
        "event: response.created\n",
        r#"data: {"type":"response.created","response":{"id":"resp_overloaded_late","model":"gpt-5.4","status":"in_progress"}}"#,
        "\n\n",
        "event: response.output_text.delta\n",
        r#"data: {"type":"response.output_text.delta","delta":"hello"}"#,
        "\n\n",
        "event: response.failed\n",
        r#"data: {"type":"response.failed","response":{"id":"resp_overloaded_late","model":"gpt-5.4","status":"failed","error":{"code":"server_is_overloaded","message":"Our servers are currently overloaded. Please try again later."}}}"#,
        "\n\n",
    ]
    .concat();

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from(payload),
    )
        .into_response()
}

async fn pool_delayed_first_chunk_upstream(
    State(state): State<PoolDelayedFirstChunkUpstreamState>,
) -> Response {
    let delay = state.first_chunk_delay;
    let stream = futures_util::stream::once(async move {
        tokio::time::sleep(delay).await;
        Ok::<Bytes, Infallible>(Bytes::from_static(br#"{"ok":true}"#))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(http_header::CONTENT_TYPE, "application/json")
        .body(Body::from_stream(stream))
        .expect("build delayed first chunk response")
}

async fn spawn_pool_delayed_first_chunk_upstream(delay: Duration) -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/responses", post(pool_delayed_first_chunk_upstream))
        .with_state(PoolDelayedFirstChunkUpstreamState {
            first_chunk_delay: delay,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed first chunk upstream");
    let addr = listener
        .local_addr()
        .expect("delayed first chunk upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("delayed first chunk upstream should run");
    });
    (format!("http://{addr}"), handle)
}

async fn pool_delayed_headers_upstream(
    State(state): State<PoolDelayedHeadersUpstreamState>,
) -> Response {
    tokio::time::sleep(state.header_delay).await;
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "phase": "headers-delayed",
        })),
    )
        .into_response()
}

async fn spawn_pool_delayed_headers_upstream(delay: Duration) -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/responses", post(pool_delayed_headers_upstream))
        .with_state(PoolDelayedHeadersUpstreamState {
            header_delay: delay,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed headers upstream");
    let addr = listener
        .local_addr()
        .expect("delayed headers upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("delayed headers upstream should run");
    });
    (format!("http://{addr}"), handle)
}

async fn pool_delayed_headers_retry_upstream(
    State(state): State<PoolDelayedHeadersRetryUpstreamState>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let attempt = {
        let mut attempts = state
            .attempts
            .lock()
            .expect("lock delayed headers retry attempts");
        let entry = attempts.entry(authorization.clone()).or_insert(0);
        *entry += 1;
        *entry
    };

    if attempt
        <= state
            .fail_before_success
            .get(&authorization)
            .copied()
            .unwrap_or(0)
    {
        tokio::time::sleep(state.header_delay).await;
    }

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authorization": authorization,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn spawn_pool_delayed_headers_retry_upstream(
    delay: Duration,
    fail_before_success: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let fail_before_success = Arc::new(
        fail_before_success
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(pool_delayed_headers_retry_upstream),
        )
        .with_state(PoolDelayedHeadersRetryUpstreamState {
            attempts: attempts.clone(),
            fail_before_success,
            header_delay: delay,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed headers retry upstream");
    let addr = listener
        .local_addr()
        .expect("delayed headers retry upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("delayed headers retry upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn pool_http_failure_upstream(State(state): State<PoolHttpFailureUpstreamState>) -> Response {
    (
        state.status,
        Json(json!({
            "error": {
                "code": state.error_code,
                "message": state.error_message,
            }
        })),
    )
        .into_response()
}

async fn spawn_oauth_codex_http_failure(
    status: StatusCode,
    error_code: Option<&str>,
    error_message: &str,
) -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route(
            "/backend-api/codex/responses",
            post(pool_http_failure_upstream),
        )
        .with_state(PoolHttpFailureUpstreamState {
            status,
            error_code: error_code.map(str::to_string),
            error_message: error_message.to_string(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind oauth codex upstream");
    let addr = listener.local_addr().expect("oauth codex upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("oauth codex upstream should run");
    });
    (format!("http://{addr}"), handle)
}

async fn oauth_codex_capture_upstream(request: axum::extract::Request) -> Response {
    if request
        .uri()
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(350)).await;
    }
    let path = request.uri().path().to_string();
    let mut forwarded_header_names = request
        .headers()
        .keys()
        .map(|name| name.as_str().to_ascii_lowercase())
        .collect::<Vec<_>>();
    forwarded_header_names.sort();
    let authorization = request
        .headers()
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let chatgpt_account_id = request
        .headers()
        .get("ChatGPT-Account-Id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let sticky_key_header = request
        .headers()
        .get("x-sticky-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let prompt_cache_key_header = request
        .headers()
        .get("x-prompt-cache-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let x_openai_prompt_cache_key_header = request
        .headers()
        .get("x-openai-prompt-cache-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let client_trace_id = request
        .headers()
        .get("x-client-trace-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let session_id = request
        .headers()
        .get("session_id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let traceparent = request
        .headers()
        .get("traceparent")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let x_client_request_id = request
        .headers()
        .get("x-client-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let x_codex_turn_metadata = request
        .headers()
        .get("x-codex-turn-metadata")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let originator = request
        .headers()
        .get("originator")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let forwarded_for = request
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("read oauth codex capture request body");
    (
        StatusCode::OK,
        Json(json!({
            "path": path,
            "authorization": authorization,
            "chatgptAccountId": chatgpt_account_id,
            "stickyKeyHeader": sticky_key_header,
            "promptCacheKeyHeader": prompt_cache_key_header,
            "xOpenAiPromptCacheKeyHeader": x_openai_prompt_cache_key_header,
            "clientTraceId": client_trace_id,
            "sessionIdHeader": session_id,
            "traceparentHeader": traceparent,
            "xClientRequestIdHeader": x_client_request_id,
            "xCodexTurnMetadataHeader": x_codex_turn_metadata,
            "originatorHeader": originator,
            "forwardedFor": forwarded_for,
            "forwardedHeaderNames": forwarded_header_names,
            "bodyLength": body.len(),
        })),
    )
        .into_response()
}

async fn spawn_oauth_codex_capture_upstream() -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        "/backend-api/codex/*path",
        any(oauth_codex_capture_upstream),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind oauth codex capture upstream");
    let addr = listener
        .local_addr()
        .expect("oauth codex capture upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("oauth codex capture upstream should run");
    });
    (format!("http://{addr}"), handle)
}

async fn oauth_codex_responses_capture_upstream(request: axum::extract::Request) -> Response {
    let path = request.uri().path().to_string();
    let mut forwarded_header_names = request
        .headers()
        .keys()
        .map(|name| name.as_str().to_ascii_lowercase())
        .collect::<Vec<_>>();
    forwarded_header_names.sort();
    let authorization = request
        .headers()
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let chatgpt_account_id = request
        .headers()
        .get("ChatGPT-Account-Id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let x_openai_prompt_cache_key_header = request
        .headers()
        .get("x-openai-prompt-cache-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let prompt_cache_key_header = request
        .headers()
        .get("x-prompt-cache-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let client_trace_id = request
        .headers()
        .get("x-client-trace-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let session_id = request
        .headers()
        .get("session_id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let traceparent = request
        .headers()
        .get("traceparent")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let x_client_request_id = request
        .headers()
        .get("x-client-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let x_codex_turn_metadata = request
        .headers()
        .get("x-codex-turn-metadata")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let originator = request
        .headers()
        .get("originator")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let content_encoding = request
        .headers()
        .get(http_header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("read oauth codex responses capture request body");
    let (decoded_body, decode_error) = decode_response_payload(&body, content_encoding.as_deref(), false);
    assert!(
        decode_error.is_none(),
        "decode oauth capture body: {decode_error:?}"
    );
    let body_value: Value =
        serde_json::from_slice(decoded_body.as_ref()).expect("decode oauth capture body");
    let completed_event = serde_json::json!({
        "type": "response.completed",
        "response": {
            "id": "resp_oauth_capture",
            "model": "gpt-5.4",
            "status": "completed",
            "path": path,
            "authorization": authorization,
            "chatgptAccountId": chatgpt_account_id,
            "xOpenAiPromptCacheKeyHeader": x_openai_prompt_cache_key_header,
            "promptCacheKeyHeader": prompt_cache_key_header,
            "clientTraceId": client_trace_id,
            "sessionIdHeader": session_id,
            "traceparentHeader": traceparent,
            "xClientRequestIdHeader": x_client_request_id,
            "xCodexTurnMetadataHeader": x_codex_turn_metadata,
            "originatorHeader": originator,
            "contentEncodingHeader": content_encoding,
            "forwardedHeaderNames": forwarded_header_names,
            "received": body_value,
            "usage": {
                "input_tokens": 12,
                "output_tokens": 3,
                "total_tokens": 15,
            }
        }
    });

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        [
            "event: response.completed".to_string(),
            format!("data: {}", completed_event),
            String::new(),
        ]
        .join("\n"),
    )
        .into_response()
}

async fn spawn_oauth_codex_responses_capture_upstream() -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        "/backend-api/codex/responses",
        post(oauth_codex_responses_capture_upstream),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind oauth codex responses capture upstream");
    let addr = listener
        .local_addr()
        .expect("oauth codex responses capture upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("oauth codex responses capture upstream should run");
    });
    (format!("http://{addr}"), handle)
}

async fn oauth_codex_delayed_headers_upstream(
    State(state): State<PoolDelayedHeadersUpstreamState>,
) -> Response {
    tokio::time::sleep(state.header_delay).await;
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "phase": "oauth-headers-delayed",
        })),
    )
        .into_response()
}

async fn spawn_oauth_codex_delayed_headers_upstream(delay: Duration) -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route(
            "/backend-api/codex/responses",
            post(oauth_codex_delayed_headers_upstream),
        )
        .with_state(PoolDelayedHeadersUpstreamState {
            header_delay: delay,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed oauth responses upstream");
    let addr = listener
        .local_addr()
        .expect("delayed oauth responses upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("delayed oauth responses upstream should run");
    });
    (format!("http://{addr}"), handle)
}

async fn oauth_codex_slow_models_upstream() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Some((
                    Ok::<_, Infallible>(Bytes::from_static(
                        br#"{"data":[{"id":"slow-model","object":"model"}]}"#,
                    )),
                    1,
                ))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Body::from_stream(chunks),
    )
}

async fn spawn_oauth_codex_slow_models_upstream() -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        "/backend-api/codex/models",
        get(oauth_codex_slow_models_upstream),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind oauth codex slow models upstream");
    let addr = listener
        .local_addr()
        .expect("oauth codex slow models upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("oauth codex slow models upstream should run");
    });
    (format!("http://{addr}"), handle)
}

async fn spawn_pool_retry_upstream(
    fail_before_success: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let fail_before_success = Arc::new(
        fail_before_success
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route("/v1/responses", post(pool_retry_upstream))
        .route("/v1/responses/compact", post(pool_retry_upstream))
        .route("/v1/chat/completions", post(pool_retry_upstream))
        .with_state(PoolRetryUpstreamState {
            attempts: attempts.clone(),
            fail_before_success,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool retry upstream");
    let addr = listener.local_addr().expect("pool retry upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool retry upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_rate_limit_responses_upstream(
    rate_limit_attempts: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let rate_limit_attempts = Arc::new(
        rate_limit_attempts
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route("/v1/responses", post(pool_rate_limit_responses_upstream))
        .with_state(PoolRateLimitResponsesUpstreamState {
            attempts: attempts.clone(),
            rate_limit_attempts,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool rate-limit responses upstream");
    let addr = listener
        .local_addr()
        .expect("pool rate-limit responses upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool rate-limit responses upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_static_failure_responses_upstream(
    statuses: &[(&str, StatusCode)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let statuses = Arc::new(
        statuses
            .iter()
            .map(|(authorization, status)| ((*authorization).to_string(), *status))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route(
            "/v1/responses",
            post(pool_static_failure_responses_upstream),
        )
        .with_state(PoolStaticFailureResponsesUpstreamState {
            attempts: attempts.clone(),
            statuses,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool static failure upstream");
    let addr = listener
        .local_addr()
        .expect("pool static failure upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool static failure upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_sequential_failure_responses_upstream(
    statuses_by_attempt: Vec<(&str, Vec<StatusCode>)>,
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let statuses_by_attempt = Arc::new(
        statuses_by_attempt
            .into_iter()
            .map(|(authorization, statuses)| (authorization.to_string(), statuses))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route(
            "/v1/responses",
            post(pool_sequential_failure_responses_upstream),
        )
        .with_state(PoolSequentialFailureResponsesUpstreamState {
            attempts: attempts.clone(),
            statuses_by_attempt,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool sequential failure upstream");
    let addr = listener
        .local_addr()
        .expect("pool sequential failure upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool sequential failure upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_compact_unsupported_upstream() -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let app = Router::new()
        .route(
            "/v1/responses/compact",
            post(pool_compact_unsupported_upstream),
        )
        .with_state(PoolCompactUnsupportedUpstreamState {
            attempts: attempts.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool compact unsupported upstream");
    let addr = listener
        .local_addr()
        .expect("pool compact unsupported upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool compact unsupported upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_rate_limit_echo_upstream(
    rate_limit_attempts: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let rate_limit_attempts = Arc::new(
        rate_limit_attempts
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route("/v1/echo", any(pool_rate_limit_echo_upstream))
        .with_state(PoolRateLimitEchoUpstreamState {
            attempts: attempts.clone(),
            rate_limit_attempts,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool rate-limit echo upstream");
    let addr = listener
        .local_addr()
        .expect("pool rate-limit echo upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool rate-limit echo upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_first_chunk_retry_upstream(
    fail_before_success: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let fail_before_success = Arc::new(
        fail_before_success
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route("/v1/responses", post(pool_first_chunk_retry_upstream))
        .with_state(PoolFirstChunkRetryUpstreamState {
            attempts: attempts.clone(),
            fail_before_success,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool first chunk retry upstream");
    let addr = listener
        .local_addr()
        .expect("pool first chunk retry upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool first chunk retry upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_response_failed_retry_upstream(
    fail_before_success: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let fail_before_success = Arc::new(
        fail_before_success
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route("/v1/responses", post(pool_response_failed_retry_upstream))
        .with_state(PoolResponseFailedRetryUpstreamState {
            attempts: attempts.clone(),
            fail_before_success,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool response.failed retry upstream");
    let addr = listener
        .local_addr()
        .expect("pool response.failed retry upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool response.failed retry upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_metadata_prefixed_response_failed_retry_upstream(
    fail_before_success: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let fail_before_success = Arc::new(
        fail_before_success
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route(
            "/v1/responses",
            post(pool_metadata_prefixed_response_failed_retry_upstream),
        )
        .with_state(PoolResponseFailedRetryUpstreamState {
            attempts: attempts.clone(),
            fail_before_success,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind metadata-prefixed response.failed retry upstream");
    let addr = listener
        .local_addr()
        .expect("metadata-prefixed response.failed retry upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("metadata-prefixed response.failed retry upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_compact_overloaded_retry_upstream(
    fail_before_success: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let fail_before_success = Arc::new(
        fail_before_success
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route(
            "/v1/responses/compact",
            post(pool_compact_overloaded_retry_upstream),
        )
        .with_state(PoolResponseFailedRetryUpstreamState {
            attempts: attempts.clone(),
            fail_before_success,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind compact overloaded retry upstream");
    let addr = listener
        .local_addr()
        .expect("compact overloaded retry upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("compact overloaded retry upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_late_response_failed_upstream() -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let app = Router::new()
        .route("/v1/responses", post(pool_late_response_failed_upstream))
        .with_state(PoolLateResponseFailedUpstreamState {
            attempts: attempts.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool late response.failed upstream");
    let addr = listener
        .local_addr()
        .expect("pool late response.failed upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool late response.failed upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn spawn_pool_http_failure_upstream(
    status: StatusCode,
    error_code: Option<&str>,
    error_message: &str,
) -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/responses", post(pool_http_failure_upstream))
        .with_state(PoolHttpFailureUpstreamState {
            status,
            error_code: error_code.map(str::to_string),
            error_message: error_message.to_string(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool http failure upstream");
    let addr = listener
        .local_addr()
        .expect("pool http failure upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool http failure upstream should run");
    });
    (format!("http://{addr}"), handle)
}

async fn load_test_sticky_route_account_id(pool: &SqlitePool, sticky_key: &str) -> Option<i64> {
    sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1")
        .bind(sticky_key)
        .fetch_optional(pool)
        .await
        .expect("load test sticky route")
}

async fn wait_for_pool_attempt_row_count(pool: &SqlitePool, min_count: i64) {
    for _ in 0..20 {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts")
            .fetch_one(pool)
            .await
            .expect("count pool attempt rows");
        if count >= min_count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_test_sticky_route_account_id(pool: &SqlitePool, sticky_key: &str) -> Option<i64> {
    for _ in 0..10 {
        if let Some(account_id) = load_test_sticky_route_account_id(pool, sticky_key).await {
            return Some(account_id);
        }
        sleep(Duration::from_millis(25)).await;
    }
    None
}

#[tokio::test]
async fn resolve_pool_account_for_request_keeps_existing_sticky_binding_when_source_is_over_soft_limit()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, primary_id, Some(100.0), Some(100.0)).await;
    set_test_account_local_limits(&state.pool, secondary_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, primary_id, Some(90.0), Some(90.0)).await;
    insert_test_pool_limit_sample(&state, secondary_id, Some(5.0), Some(5.0)).await;
    for sticky_key in [
        "sticky-bound",
        "sticky-bound-extra-001",
        "sticky-bound-extra-002",
        "sticky-bound-extra-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, primary_id, &recent_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-bound"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, primary_id);
    assert_ne!(account.account_id, secondary_id);
}

#[tokio::test]
async fn resolver_skips_degraded_accounts_for_fresh_assignment() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let degraded_id =
        insert_test_pool_api_key_account(&state, "Degraded", "upstream-degraded").await;
    let healthy_id = insert_test_pool_api_key_account(&state, "Healthy", "upstream-healthy").await;
    set_test_account_degraded_route_state(
        &state.pool,
        degraded_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        "test degraded plain 429",
    )
    .await;

    let account = match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, healthy_id);
    assert_ne!(account.account_id, degraded_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
async fn resolver_keeps_existing_sticky_owner_reusable_during_temporary_cooldown() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let degraded_id =
        insert_test_pool_api_key_account(&state, "Degraded", "upstream-degraded").await;
    let healthy_id = insert_test_pool_api_key_account(&state, "Healthy", "upstream-healthy").await;
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(&state.pool, "sticky-degraded", degraded_id, &sticky_seen_at).await;
    set_test_account_degraded_route_state(
        &state.pool,
        degraded_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        "test degraded 5xx",
    )
    .await;

    let sticky_account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-degraded"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve degraded sticky account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("sticky degraded account should resolve, got {other:?}"),
    };

    assert_eq!(sticky_account.account_id, degraded_id);
    assert_eq!(
        sticky_account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );

    set_test_account_generic_route_cooldown(&state.pool, degraded_id, 120).await;

    let sticky_account_during_cooldown = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-degraded"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve after temporary cooldown")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => {
            panic!("sticky owner should remain reusable during temporary cooldown, got {other:?}")
        }
    };

    assert_eq!(sticky_account_during_cooldown.account_id, degraded_id);
    assert_eq!(
        sticky_account_during_cooldown.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
    assert_ne!(sticky_account_during_cooldown.account_id, healthy_id);
}

#[tokio::test]
async fn resolver_returns_degraded_only_when_only_temporary_failure_accounts_remain() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let plain_429_id =
        insert_test_pool_api_key_account(&state, "Plain429", "upstream-plain-429").await;
    let upstream_5xx_id =
        insert_test_pool_api_key_account(&state, "Upstream5xx", "upstream-5xx").await;
    set_test_account_degraded_route_state(
        &state.pool,
        plain_429_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        "test degraded plain 429",
    )
    .await;
    set_test_account_degraded_route_state(
        &state.pool,
        upstream_5xx_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        "test degraded 5xx",
    )
    .await;

    let resolution = resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve degraded-only pool");
    assert!(matches!(resolution, PoolAccountResolution::DegradedOnly));
}

#[tokio::test]
async fn resolve_pool_account_for_request_prefers_candidates_within_soft_sticky_limit() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let overloaded_id =
        insert_test_pool_api_key_account(&state, "Overloaded", "upstream-overloaded").await;
    let available_id =
        insert_test_pool_api_key_account(&state, "Available", "upstream-available").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, overloaded_id, Some(100.0), Some(100.0)).await;
    set_test_account_local_limits(&state.pool, available_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, overloaded_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, available_id, Some(80.0), Some(80.0)).await;
    for sticky_key in [
        "sticky-overloaded-001",
        "sticky-overloaded-002",
        "sticky-overloaded-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, overloaded_id, &recent_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-prefer-available"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, available_id);
    assert_ne!(account.account_id, overloaded_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_falls_back_to_over_soft_limit_bucket_when_needed() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let preferred_id =
        insert_test_pool_api_key_account(&state, "Preferred", "upstream-preferred").await;
    let fallback_id =
        insert_test_pool_api_key_account(&state, "Fallback", "upstream-fallback").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, preferred_id, Some(100.0), Some(100.0)).await;
    set_test_account_local_limits(&state.pool, fallback_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, preferred_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, fallback_id, Some(65.0), Some(65.0)).await;
    for sticky_key in [
        "sticky-preferred-001",
        "sticky-preferred-002",
        "sticky-preferred-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, preferred_id, &recent_seen_at).await;
    }
    for sticky_key in [
        "sticky-fallback-001",
        "sticky-fallback-002",
        "sticky-fallback-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, fallback_id, &recent_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-soft-fallback"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, preferred_id);
    assert_ne!(account.account_id, fallback_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_falls_back_after_soft_bucket_candidate_rejects_cut_in() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let source_id = insert_test_pool_api_key_account(&state, "Source", "upstream-source").await;
    let guarded_id = insert_test_pool_api_key_account(&state, "Guarded", "upstream-guarded").await;
    let overloaded_id =
        insert_test_pool_api_key_account(&state, "Overloaded", "upstream-overloaded").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now_iso = format_utc_iso(Utc::now());

    upsert_test_sticky_route_at(&state.pool, "sticky-transfer", source_id, &recent_seen_at).await;
    set_test_account_status(&state.pool, source_id, "error").await;
    set_test_account_local_limits(&state.pool, guarded_id, Some(100.0), Some(100.0)).await;
    set_test_account_local_limits(&state.pool, overloaded_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, guarded_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, overloaded_id, Some(80.0), Some(80.0)).await;
    for sticky_key in [
        "sticky-overloaded-cut-in-001",
        "sticky-overloaded-cut-in-002",
        "sticky-overloaded-cut-in-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, overloaded_id, &recent_seen_at).await;
    }

    let disallow_cut_in_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, guard_enabled, lookback_hours, max_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, NULL, NULL, 1, 0, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("no-cut-in")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert no-cut-in tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(guarded_id)
    .bind(disallow_cut_in_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach no-cut-in tag");

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-transfer"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, overloaded_id);
    assert_ne!(account.account_id, guarded_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_allows_timeout_failover_past_cut_out_rule() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let source_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Source",
        "upstream-source",
        None,
        None,
        Some("https://route-a.example.com/backend-api/"),
    )
    .await;
    let alternate_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Alternate",
        "upstream-alternate",
        None,
        None,
        Some("https://route-b.example.com/backend-api/"),
    )
    .await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now_iso = format_utc_iso(Utc::now());

    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-timeout-cut-out",
        source_id,
        &recent_seen_at,
    )
    .await;

    let disallow_cut_out_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, guard_enabled, lookback_hours, max_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, NULL, NULL, 0, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("no-cut-out")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert no-cut-out tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(source_id)
    .bind(disallow_cut_out_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach no-cut-out tag");

    let mut excluded_upstream_route_keys = HashSet::new();
    excluded_upstream_route_keys.insert(
        crate::upstream_accounts::canonical_pool_upstream_route_key(
            &Url::parse("https://route-a.example.com/backend-api/").expect("valid route a url"),
        ),
    );

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-timeout-cut-out"),
        &[],
        &excluded_upstream_route_keys,
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve after timeout route exclusion, got {other:?}"),
    };

    assert_eq!(account.account_id, alternate_id);
    assert_ne!(account.account_id, source_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_allows_timeout_failover_past_cut_out_rule_with_invalid_sticky_group()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let source_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Broken Sticky Source",
        "upstream-broken-source",
        None,
        None,
        Some("https://route-a.example.com/backend-api/"),
    )
    .await;
    let alternate_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Healthy Alternate",
        "upstream-healthy-alternate",
        None,
        None,
        Some("https://route-b.example.com/backend-api/"),
    )
    .await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now_iso = format_utc_iso(Utc::now());

    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-timeout-invalid-group",
        source_id,
        &recent_seen_at,
    )
    .await;

    let disallow_cut_out_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, guard_enabled, lookback_hours, max_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, NULL, NULL, 0, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("invalid-group-no-cut-out")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert no-cut-out tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(source_id)
    .bind(disallow_cut_out_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach no-cut-out tag");
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(source_id)
        .bind("broken-sticky-group")
        .execute(&state.pool)
        .await
        .expect("set broken sticky group");

    let mut excluded_upstream_route_keys = HashSet::new();
    excluded_upstream_route_keys.insert(
        crate::upstream_accounts::canonical_pool_upstream_route_key(
            &Url::parse("https://route-a.example.com/backend-api/").expect("valid route a url"),
        ),
    );

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-timeout-invalid-group"),
        &[],
        &excluded_upstream_route_keys,
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => {
            panic!("pool account should resolve after excluding broken sticky route, got {other:?}")
        }
    };

    assert_eq!(account.account_id, alternate_id);
    assert_ne!(account.account_id, source_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_soft_deprioritizes_accounts_with_only_remote_limit_signals()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let exempt_id = insert_test_pool_api_key_account(&state, "Exempt", "upstream-exempt").await;
    let limited_id = insert_test_pool_api_key_account(&state, "Limited", "upstream-limited").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, limited_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, exempt_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, limited_id, Some(80.0), Some(80.0)).await;
    for sticky_key in [
        "sticky-exempt-001",
        "sticky-exempt-002",
        "sticky-exempt-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, exempt_id, &recent_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-exempt-target"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, limited_id);
    assert_ne!(account.account_id, exempt_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_does_not_soft_deprioritize_stale_sticky_routes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let exempt_id =
        insert_test_pool_api_key_account(&state, "Exempt", "upstream-exempt-stale").await;
    let limited_id =
        insert_test_pool_api_key_account(&state, "Limited", "upstream-limited-stale").await;
    let stale_seen_at = format_test_stale_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, limited_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, exempt_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, limited_id, Some(80.0), Some(80.0)).await;
    for sticky_key in [
        "sticky-stale-exempt-001",
        "sticky-stale-exempt-002",
        "sticky-stale-exempt-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, exempt_id, &stale_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-stale-exempt-target"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, exempt_id);
    assert_ne!(account.account_id, limited_id);
}

#[tokio::test]
async fn resolve_pool_account_for_request_prefers_reset_aware_pressure_over_raw_percent() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let team_id = insert_test_pool_api_key_account(&state, "Team", "upstream-team").await;
    let free_id = insert_test_pool_api_key_account(&state, "Free", "upstream-free").await;
    let now = Utc::now();

    insert_test_pool_limit_sample_with_windows(
        &state,
        team_id,
        Some("team"),
        Some(70.0),
        Some(300),
        Some(&format_utc_iso(now + ChronoDuration::minutes(5))),
        Some(40.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(1))),
    )
    .await;
    insert_test_pool_limit_sample_with_windows(
        &state,
        free_id,
        Some("free"),
        None,
        None,
        None,
        Some(30.0),
        Some(7 * 24 * 60),
        Some(&format_utc_iso(now + ChronoDuration::days(6))),
    )
    .await;

    let account = match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, team_id);
    assert_ne!(account.account_id, free_id);
}
