pub(crate) async fn pool_late_response_failed_upstream(
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

pub(crate) async fn pool_delayed_first_chunk_upstream(
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

pub(crate) async fn spawn_pool_delayed_first_chunk_upstream(
    delay: Duration,
) -> (String, JoinHandle<()>) {
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

pub(crate) async fn pool_delayed_headers_upstream(
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

pub(crate) async fn spawn_pool_delayed_headers_upstream(
    delay: Duration,
) -> (String, JoinHandle<()>) {
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

pub(crate) async fn pool_delayed_headers_retry_upstream(
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

pub(crate) async fn spawn_pool_delayed_headers_retry_upstream(
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

pub(crate) async fn pool_http_failure_upstream(
    State(state): State<PoolHttpFailureUpstreamState>,
) -> Response {
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

pub(crate) async fn spawn_oauth_codex_http_failure(
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

struct OAuthCaptureHeaders {
    forwarded_header_names: Vec<String>,
    authorization: Option<String>,
    chatgpt_account_id: Option<String>,
    sticky_key: Option<String>,
    prompt_cache_key: Option<String>,
    x_openai_prompt_cache_key: Option<String>,
    client_trace_id: Option<String>,
    session_id: Option<String>,
    traceparent: Option<String>,
    x_client_request_id: Option<String>,
    x_codex_turn_metadata: Option<String>,
    originator: Option<String>,
    forwarded_for: Option<String>,
    content_encoding: Option<String>,
}

fn oauth_header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn capture_oauth_headers(headers: &HeaderMap) -> OAuthCaptureHeaders {
    let mut forwarded_header_names = headers
        .keys()
        .map(|name| name.as_str().to_ascii_lowercase())
        .collect::<Vec<_>>();
    forwarded_header_names.sort();
    OAuthCaptureHeaders {
        forwarded_header_names,
        authorization: oauth_header(headers, http_header::AUTHORIZATION.as_str()),
        chatgpt_account_id: oauth_header(headers, "ChatGPT-Account-Id"),
        sticky_key: oauth_header(headers, "x-sticky-key"),
        prompt_cache_key: oauth_header(headers, "x-prompt-cache-key"),
        x_openai_prompt_cache_key: oauth_header(headers, "x-openai-prompt-cache-key"),
        client_trace_id: oauth_header(headers, "x-client-trace-id"),
        session_id: oauth_header(headers, "session_id"),
        traceparent: oauth_header(headers, "traceparent"),
        x_client_request_id: oauth_header(headers, "x-client-request-id"),
        x_codex_turn_metadata: oauth_header(headers, "x-codex-turn-metadata"),
        originator: oauth_header(headers, "originator"),
        forwarded_for: oauth_header(headers, "x-forwarded-for"),
        content_encoding: oauth_header(headers, http_header::CONTENT_ENCODING.as_str()),
    }
}

pub(crate) async fn oauth_codex_capture_upstream(request: axum::extract::Request) -> Response {
    if request
        .uri()
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(350)).await;
    }
    let path = request.uri().path().to_string();
    let request_uri_query = request.uri().query().unwrap_or_default().to_string();
    let headers = capture_oauth_headers(request.headers());
    let body = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("read oauth codex capture request body");
    (
        StatusCode::OK,
        Json(json!({
            "path": path,
            "query": request_uri_query,
            "authorization": headers.authorization,
            "chatgptAccountId": headers.chatgpt_account_id,
            "stickyKeyHeader": headers.sticky_key,
            "promptCacheKeyHeader": headers.prompt_cache_key,
            "xOpenAiPromptCacheKeyHeader": headers.x_openai_prompt_cache_key,
            "clientTraceId": headers.client_trace_id,
            "sessionIdHeader": headers.session_id,
            "traceparentHeader": headers.traceparent,
            "xClientRequestIdHeader": headers.x_client_request_id,
            "xCodexTurnMetadataHeader": headers.x_codex_turn_metadata,
            "originatorHeader": headers.originator,
            "forwardedFor": headers.forwarded_for,
            "forwardedHeaderNames": headers.forwarded_header_names,
            "bodyLength": body.len(),
            "body": String::from_utf8_lossy(&body),
        })),
    )
        .into_response()
}

pub(crate) async fn spawn_oauth_codex_capture_upstream() -> (String, JoinHandle<()>) {
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

pub(crate) async fn oauth_codex_responses_capture_upstream(
    request: axum::extract::Request,
) -> Response {
    let path = request.uri().path().to_string();
    let headers = capture_oauth_headers(request.headers());
    let body = to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("read oauth codex responses capture request body");
    let (decoded_body, decode_error) =
        decode_response_payload(&body, headers.content_encoding.as_deref(), false);
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
            "authorization": headers.authorization,
            "chatgptAccountId": headers.chatgpt_account_id,
            "xOpenAiPromptCacheKeyHeader": headers.x_openai_prompt_cache_key,
            "promptCacheKeyHeader": headers.prompt_cache_key,
            "clientTraceId": headers.client_trace_id,
            "sessionIdHeader": headers.session_id,
            "traceparentHeader": headers.traceparent,
            "xClientRequestIdHeader": headers.x_client_request_id,
            "xCodexTurnMetadataHeader": headers.x_codex_turn_metadata,
            "originatorHeader": headers.originator,
            "contentEncodingHeader": headers.content_encoding,
            "forwardedHeaderNames": headers.forwarded_header_names,
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

pub(crate) async fn spawn_oauth_codex_responses_capture_upstream() -> (String, JoinHandle<()>) {
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

pub(crate) async fn oauth_codex_delayed_headers_upstream(
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

pub(crate) async fn spawn_oauth_codex_delayed_headers_upstream(
    delay: Duration,
) -> (String, JoinHandle<()>) {
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

pub(crate) async fn oauth_codex_slow_models_upstream() -> impl IntoResponse {
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

pub(crate) async fn spawn_oauth_codex_slow_models_upstream() -> (String, JoinHandle<()>) {
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

pub(crate) async fn spawn_pool_retry_upstream(
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
        .route("/v1/models", get(pool_retry_upstream))
        .route("/v1/responses", post(pool_retry_upstream))
        .route("/v1/responses/compact", post(pool_retry_upstream))
        .route("/v1/chat/completions", post(pool_retry_upstream))
        .route("/v1/alpha/search", post(pool_retry_upstream))
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

pub(crate) async fn spawn_pool_rate_limit_responses_upstream(
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

pub(crate) async fn spawn_pool_static_failure_responses_upstream(
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
        .route(
            "/v1/alpha/search",
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

pub(crate) async fn spawn_pool_sequential_failure_responses_upstream(
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

pub(crate) async fn spawn_pool_compact_unsupported_upstream() -> (
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

pub(crate) async fn spawn_pool_rate_limit_echo_upstream(
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

pub(crate) async fn spawn_pool_first_chunk_retry_upstream(
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

pub(crate) async fn spawn_pool_response_failed_retry_upstream(
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
            failure_code: UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED,
            failure_message: "Our servers are currently overloaded. Please try again later.",
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

pub(crate) async fn spawn_pool_metadata_prefixed_response_failed_retry_upstream(
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
            failure_code: UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED,
            failure_message: "Our servers are currently overloaded. Please try again later.",
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

pub(crate) async fn spawn_pool_metadata_prefixed_response_failed_retry_upstream_with_error(
    fail_before_success: &[(&str, usize)],
    failure_code: &'static str,
    failure_message: &'static str,
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
            failure_code,
            failure_message,
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind custom response.failed retry upstream");
    let addr = listener
        .local_addr()
        .expect("custom response.failed retry upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("custom response.failed retry upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

use super::*;
