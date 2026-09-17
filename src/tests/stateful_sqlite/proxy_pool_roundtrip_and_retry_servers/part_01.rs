#[test]
pub(crate) fn pool_same_account_attempt_budget_keeps_legacy_budget_for_non_responses_routes() {
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
pub(crate) async fn proxy_capture_target_responses_uses_dedicated_first_byte_timeout() {
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
pub(crate) async fn proxy_openai_v1_allows_slow_upload_with_short_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(100);
    let state = test_state_from_config(config, true).await;

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
pub(crate) async fn proxy_openai_v1_e2e_http_roundtrip() {
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
pub(crate) struct PoolRetryUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
    pub(crate) fail_before_success: Arc<HashMap<String, usize>>,
}

#[derive(Clone)]
pub(crate) struct PoolRateLimitResponsesUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
    pub(crate) rate_limit_attempts: Arc<HashMap<String, usize>>,
}

#[derive(Clone)]
pub(crate) struct PoolRateLimitEchoUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
    pub(crate) rate_limit_attempts: Arc<HashMap<String, usize>>,
}

#[derive(Clone)]
pub(crate) struct PoolStaticFailureResponsesUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
    pub(crate) statuses: Arc<HashMap<String, StatusCode>>,
}

#[derive(Clone)]
pub(crate) struct PoolSequentialFailureResponsesUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
    pub(crate) statuses_by_attempt: Arc<HashMap<String, Vec<StatusCode>>>,
}

#[derive(Clone)]
pub(crate) struct PoolCompactUnsupportedUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
}

#[derive(Clone)]
pub(crate) struct PoolFirstChunkRetryUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
    pub(crate) fail_before_success: Arc<HashMap<String, usize>>,
}

#[derive(Clone)]
pub(crate) struct PoolResponseFailedRetryUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
    pub(crate) fail_before_success: Arc<HashMap<String, usize>>,
    pub(crate) failure_code: &'static str,
    pub(crate) failure_message: &'static str,
}

#[derive(Clone)]
pub(crate) struct PoolLateResponseFailedUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
}

#[derive(Clone)]
pub(crate) struct PoolDelayedFirstChunkUpstreamState {
    pub(crate) first_chunk_delay: Duration,
}

#[derive(Clone)]
pub(crate) struct PoolDelayedHeadersUpstreamState {
    pub(crate) header_delay: Duration,
}

#[derive(Clone)]
pub(crate) struct PoolDelayedHeadersRetryUpstreamState {
    pub(crate) attempts: Arc<StdMutex<HashMap<String, usize>>>,
    pub(crate) fail_before_success: Arc<HashMap<String, usize>>,
    pub(crate) header_delay: Duration,
}

#[derive(Clone)]
pub(crate) struct PoolHttpFailureUpstreamState {
    pub(crate) status: StatusCode,
    pub(crate) error_code: Option<String>,
    pub(crate) error_message: String,
}

pub(crate) async fn pool_retry_upstream(
    State(state): State<PoolRetryUpstreamState>,
    headers: HeaderMap,
    uri: Uri,
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
            "path": uri.path(),
            "query": uri.query().unwrap_or_default(),
        })),
    )
        .into_response()
}

pub(crate) async fn pool_rate_limit_responses_upstream(
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

pub(crate) async fn pool_rate_limit_echo_upstream(
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

pub(crate) async fn pool_static_failure_responses_upstream(
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

pub(crate) async fn pool_sequential_failure_responses_upstream(
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

pub(crate) async fn pool_compact_unsupported_upstream(
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

pub(crate) async fn pool_first_chunk_retry_upstream(
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

pub(crate) async fn pool_response_failed_retry_upstream(
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
        let failed_event = serde_json::to_string(&json!({
            "type": "response.failed",
            "response": {
                "id": "resp_overloaded_retry",
                "model": "gpt-5.4",
                "status": "failed",
                "error": {
                    "code": state.failure_code,
                    "message": state.failure_message,
                },
            },
        }))
        .expect("serialize response.failed retry event");
        let payload = format!("event: response.failed\ndata: {failed_event}\n\n");
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

pub(crate) async fn pool_metadata_prefixed_response_failed_retry_upstream(
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
        let failed_event = serde_json::to_string(&json!({
            "type": "response.failed",
            "response": {
                "id": "resp_overloaded_retry",
                "model": "gpt-5.4",
                "status": "failed",
                "error": {
                    "code": state.failure_code,
                    "message": state.failure_message,
                },
            },
        }))
        .expect("serialize metadata-prefixed response.failed retry event");
        let payload = format!(
            "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"id\":\"resp_overloaded_retry\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}}}\n\n\
             event: response.failed\ndata: {failed_event}\n\n"
        );
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

pub(crate) async fn pool_large_metadata_prefixed_response_failed_retry_upstream(
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
            .expect("lock large metadata-prefixed response.failed retry attempts");
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
        let oversized_metadata =
            less_compressible_test_string(RAW_RESPONSE_PREVIEW_LIMIT + 8 * 1024);
        let created = format!(
            "event: response.created\n\
             data: {}\n\n",
            serde_json::to_string(&json!({
                "type": "response.created",
                "response": {
                    "id": "resp_overloaded_retry",
                    "model": "gpt-5.4",
                    "status": "in_progress",
                    "metadata": oversized_metadata,
                },
            }))
            .expect("serialize oversized metadata-prefixed response.created payload")
        );
        let failed_event = serde_json::to_string(&json!({
            "type": "response.failed",
            "response": {
                "id": "resp_overloaded_retry",
                "model": "gpt-5.4",
                "status": "failed",
                "error": {
                    "code": state.failure_code,
                    "message": state.failure_message,
                },
            },
        }))
        .expect("serialize large metadata-prefixed response.failed retry event");
        let failed = format!("event: response.failed\ndata: {failed_event}\n\n");
        let chunks = stream::iter(vec![
            Ok::<_, Infallible>(Bytes::from(created)),
            Ok::<_, Infallible>(Bytes::from(failed)),
        ]);
        return (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            )],
            Body::from_stream(chunks),
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

pub(crate) async fn pool_compact_overloaded_retry_upstream(
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
                    "code": state.failure_code,
                    "message": state.failure_message,
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

use super::*;
