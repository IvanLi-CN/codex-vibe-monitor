pub(crate) async fn seed_quota_snapshot(pool: &SqlitePool, captured_at: &str) {
    sqlx::query(
        r#"
        INSERT INTO codex_quota_snapshots (
            captured_at,
            amount_limit,
            used_amount,
            remaining_amount,
            period,
            period_reset_time,
            expire_time,
            is_active,
            total_cost,
            total_requests,
            total_tokens,
            last_request_time,
            billing_type,
            remaining_count,
            used_count,
            sub_type_name
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        "#,
    )
    .bind(captured_at)
    .bind(Some(100.0))
    .bind(Some(10.0))
    .bind(Some(90.0))
    .bind(Some("monthly"))
    .bind(Some("2026-03-01 00:00:00"))
    .bind(None::<String>)
    .bind(1_i64)
    .bind(10.0)
    .bind(9_i64)
    .bind(150_i64)
    .bind(Some(captured_at))
    .bind(Some("prepaid"))
    .bind(Some(91_i64))
    .bind(Some(9_i64))
    .bind(Some("unit"))
    .execute(pool)
    .await
    .expect("seed quota snapshot");
}

pub(crate) async fn seed_forward_proxy_attempt_at(
    pool: &SqlitePool,
    proxy_key: &str,
    occurred_at: DateTime<Utc>,
    is_success: bool,
) {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_attempts (
            proxy_key,
            occurred_at,
            is_success,
            latency_ms,
            failure_kind,
            is_probe
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(proxy_key)
    .bind(occurred_at.format("%Y-%m-%d %H:%M:%S").to_string())
    .bind(is_success as i64)
    .bind(if is_success { Some(120.0) } else { None })
    .bind(if is_success {
        None::<String>
    } else {
        Some(FORWARD_PROXY_FAILURE_STREAM_ERROR.to_string())
    })
    .bind(0_i64)
    .execute(pool)
    .await
    .expect("seed forward proxy attempt");
}

pub(crate) async fn seed_pool_upstream_attempt_at(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
    proxy_binding_key_snapshot: Option<&str>,
    status: &str,
) {
    let occurred_at = format_naive(occurred_at.with_timezone(&Shanghai).naive_local());
    let phase = terminal_pool_upstream_request_attempt_phase(status);
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            group_name_snapshot,
            proxy_binding_key_snapshot,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            phase,
            http_status,
            error_message,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            created_at
        )
        VALUES (
            ?1, ?2, '/v1/responses', ?3, 'sticky-node-health', NULL, ?4, 41, 'route-node-health',
            1, 1, 0, '203.0.113.10', ?2, ?2, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now')
        )
        "#,
    )
    .bind(invoke_id)
    .bind(&occurred_at)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(proxy_binding_key_snapshot)
    .bind(status)
    .bind(phase)
    .bind((status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(200_i64))
    .bind(
        (status != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
            .then_some("seeded pool upstream failure"),
    )
    .bind((status != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(180.0))
    .bind((status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(120.0))
    .bind((status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(320.0))
    .execute(pool)
    .await
    .expect("seed pool upstream attempt");
}

pub(crate) struct ForwardProxyWeightBucket<'a> {
    pub(crate) proxy_key: &'a str,
    pub(crate) bucket_start_epoch: i64,
    pub(crate) sample_count: i64,
    pub(crate) min_weight: f64,
    pub(crate) max_weight: f64,
    pub(crate) avg_weight: f64,
    pub(crate) last_weight: f64,
}

pub(crate) async fn seed_forward_proxy_weight_bucket_at(
    pool: &SqlitePool,
    bucket: ForwardProxyWeightBucket<'_>,
) {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_weight_hourly (
            proxy_key,
            bucket_start_epoch,
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
        ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
            sample_count = excluded.sample_count,
            min_weight = excluded.min_weight,
            max_weight = excluded.max_weight,
            avg_weight = excluded.avg_weight,
            last_weight = excluded.last_weight,
            updated_at = datetime('now')
        "#,
    )
    .bind(bucket.proxy_key)
    .bind(bucket.bucket_start_epoch)
    .bind(bucket.sample_count)
    .bind(bucket.min_weight)
    .bind(bucket.max_weight)
    .bind(bucket.avg_weight)
    .bind(bucket.last_weight)
    .execute(pool)
    .await
    .expect("seed forward proxy weight bucket");
}

pub(crate) async fn drain_broadcast_messages(rx: &mut broadcast::Receiver<BroadcastPayload>) {
    loop {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(_)) => continue,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => break,
        }
    }
}

pub(crate) async fn recv_next_records(
    rx: &mut broadcast::Receiver<BroadcastPayload>,
) -> Vec<ApiInvocation> {
    loop {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("records broadcast should arrive")
            .expect("records broadcast channel should stay open");
        if let BroadcastPayload::Records { records } = payload {
            assert_eq!(records.len(), 1);
            return records;
        }
    }
}

pub(crate) async fn spawn_test_forward_proxy_status(
    status: StatusCode,
) -> (String, JoinHandle<()>) {
    let app = Router::new().fallback(any(move || async move {
        (
            status,
            Json(json!({
                "status": status.as_u16(),
            })),
        )
    }));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind forward proxy status test server");
    let addr = listener
        .local_addr()
        .expect("forward proxy status test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("forward proxy status test server should run");
    });

    (format!("http://{addr}"), handle)
}

pub(crate) async fn spawn_test_counting_forward_proxy_status(
    status: StatusCode,
    request_count: Arc<AtomicUsize>,
) -> (String, JoinHandle<()>) {
    let app = Router::new().fallback(any(move || {
        let request_count = request_count.clone();
        async move {
            request_count.fetch_add(1, Ordering::SeqCst);
            (
                status,
                Json(json!({
                    "status": status.as_u16(),
                })),
            )
        }
    }));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind counting forward proxy status test server");
    let addr = listener
        .local_addr()
        .expect("counting forward proxy status test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("counting forward proxy status test server should run");
    });

    (format!("http://{addr}"), handle)
}

pub(crate) async fn spawn_test_blocking_forward_proxy_status(
    status: StatusCode,
    request_started: Arc<Notify>,
    release_request: Arc<Notify>,
) -> (String, JoinHandle<()>) {
    let app = Router::new().fallback(any(move || {
        let request_started = request_started.clone();
        let release_request = release_request.clone();
        async move {
            request_started.notify_waiters();
            release_request.notified().await;
            (
                status,
                Json(json!({
                    "status": status.as_u16(),
                })),
            )
        }
    }));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind blocking forward proxy status test server");
    let addr = listener
        .local_addr()
        .expect("blocking forward proxy status test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("blocking forward proxy status test server should run");
    });

    (format!("http://{addr}"), handle)
}

pub(crate) async fn spawn_test_subscription_source(body: String) -> (String, JoinHandle<()>) {
    let body = Arc::new(body);
    let app = Router::new().route(
        "/subscription",
        get({
            let body = body.clone();
            move || {
                let body = body.clone();
                async move { (StatusCode::OK, body.as_str().to_string()) }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind subscription source test server");
    let addr = listener
        .local_addr()
        .expect("subscription source test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("subscription source test server should run");
    });

    (format!("http://{addr}/subscription"), handle)
}

pub(crate) async fn count_forward_proxy_probe_attempts(
    pool: &SqlitePool,
    proxy_key: &str,
    success: Option<bool>,
) -> i64 {
    let query = match success {
        Some(true) => {
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0 AND is_success != 0"
        }
        Some(false) => {
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0 AND is_success = 0"
        }
        None => {
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0"
        }
    };
    sqlx::query_scalar(query)
        .bind(proxy_key)
        .fetch_one(pool)
        .await
        .expect("count forward proxy probe attempts")
}

pub(crate) async fn wait_for_forward_proxy_probe_attempts(
    pool: &SqlitePool,
    proxy_key: &str,
    expected_min_count: i64,
) {
    let started = Instant::now();
    loop {
        let count = count_forward_proxy_probe_attempts(pool, proxy_key, None).await;
        if count >= expected_min_count {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timed out waiting forward proxy probe attempts for {proxy_key}; expected at least {expected_min_count}, got {count}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(crate) async fn count_request_forward_proxy_attempts(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM forward_proxy_attempts WHERE is_probe = 0")
        .fetch_one(pool)
        .await
        .expect("count request forward proxy attempts")
}

pub(crate) async fn count_request_forward_proxy_attempts_with_failure_kind(
    pool: &SqlitePool,
    failure_kind: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM forward_proxy_attempts WHERE is_probe = 0 AND failure_kind = ?1",
    )
    .bind(failure_kind)
    .fetch_one(pool)
    .await
    .expect("count request forward proxy attempts by failure kind")
}

pub(crate) async fn count_pool_upstream_request_attempts(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts")
        .fetch_one(pool)
        .await
        .expect("count pool upstream request attempts")
}

pub(crate) async fn wait_for_pool_upstream_request_attempts(
    pool: &SqlitePool,
    expected_min_count: i64,
) {
    let started = Instant::now();
    loop {
        let count = count_pool_upstream_request_attempts(pool).await;
        if count >= expected_min_count {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timed out waiting pool upstream attempts; expected at least {expected_min_count}, got {count}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(crate) async fn latest_request_forward_proxy_attempt_latency_ms(
    pool: &SqlitePool,
) -> Option<f64> {
    sqlx::query_scalar::<_, Option<f64>>(
        "SELECT latency_ms FROM forward_proxy_attempts WHERE is_probe = 0 ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .expect("fetch latest request forward proxy latency")
    .flatten()
}

pub(crate) async fn count_codex_invocations(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(pool)
        .await
        .expect("count codex invocations")
}

pub(crate) async fn count_in_flight_codex_invocations(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("count in-flight codex invocations")
}

pub(crate) async fn wait_for_codex_invocations(pool: &SqlitePool, expected_min_count: i64) {
    let started = Instant::now();
    loop {
        let count = count_codex_invocations(pool).await;
        let in_flight = count_in_flight_codex_invocations(pool).await;
        if count >= expected_min_count && in_flight == 0 {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timed out waiting for codex invocations; expected at least {expected_min_count}, got {count}, in_flight={in_flight}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[derive(Clone)]
pub(crate) struct Retry429EchoState {
    rate_limit_attempts: usize,
    attempts: Arc<AtomicUsize>,
    seen_bodies: Arc<StdMutex<Vec<String>>>,
    retry_after: Option<HeaderValue>,
}

pub(crate) async fn retrying_echo_upstream(
    State(state): State<Retry429EchoState>,
    method: Method,
    uri: Uri,
    body: String,
) -> Response {
    state
        .seen_bodies
        .lock()
        .expect("lock retrying echo bodies")
        .push(body.clone());
    let attempt = state.attempts.fetch_add(1, Ordering::SeqCst) + 1;

    if attempt <= state.rate_limit_attempts {
        let mut headers = HeaderMap::new();
        if let Some(retry_after) = state.retry_after.clone() {
            headers.insert(http_header::RETRY_AFTER, retry_after);
        }
        headers.insert(
            http_header::HeaderName::from_static("x-upstream-attempt"),
            HeaderValue::from_str(&attempt.to_string()).expect("valid attempt header"),
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            Json(json!({
                "error": "rate limited",
                "attempt": attempt,
                "method": method.as_str(),
                "query": uri.query().unwrap_or_default(),
                "body": body,
            })),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(json!({
            "attempt": attempt,
            "method": method.as_str(),
            "path": uri.path(),
            "query": uri.query().unwrap_or_default(),
            "body": body,
        })),
    )
        .into_response()
}

pub(crate) async fn spawn_retrying_echo_upstream(
    rate_limit_attempts: usize,
    retry_after: Option<&str>,
) -> (
    String,
    Arc<AtomicUsize>,
    Arc<StdMutex<Vec<String>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen_bodies = Arc::new(StdMutex::new(Vec::<String>::new()));
    let app = Router::new()
        .route("/v1/echo", any(retrying_echo_upstream))
        .with_state(Retry429EchoState {
            rate_limit_attempts,
            attempts: attempts.clone(),
            seen_bodies: seen_bodies.clone(),
            retry_after: retry_after
                .map(|value| HeaderValue::from_str(value).expect("valid retry-after header")),
        });

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind retrying echo test server");
    let addr = listener
        .local_addr()
        .expect("retrying echo test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("retrying echo test server should run");
    });

    (format!("http://{addr}/"), attempts, seen_bodies, handle)
}

#[derive(Clone)]
pub(crate) struct Retry429CaptureState {
    rate_limit_attempts: usize,
    attempts: Arc<AtomicUsize>,
    seen_payloads: Arc<StdMutex<Vec<Value>>>,
    retry_after: Option<HeaderValue>,
}

pub(crate) async fn retrying_capture_upstream(
    State(state): State<Retry429CaptureState>,
    body: Bytes,
) -> Response {
    let payload: Value = serde_json::from_slice(&body).expect("decode retrying capture body");
    state
        .seen_payloads
        .lock()
        .expect("lock retrying capture payloads")
        .push(payload.clone());
    let attempt = state.attempts.fetch_add(1, Ordering::SeqCst) + 1;

    if attempt <= state.rate_limit_attempts {
        let mut headers = HeaderMap::new();
        if let Some(retry_after) = state.retry_after.clone() {
            headers.insert(http_header::RETRY_AFTER, retry_after);
        }
        return (
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            Json(json!({
                "error": "rate limited",
                "attempt": attempt,
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "id": "resp_retry_success",
            "object": "response",
            "model": "gpt-5.3-codex",
            "usage": {
                "input_tokens": 12,
                "output_tokens": 3,
                "total_tokens": 15
            },
            "received": payload,
            "attempt": attempt,
        })),
    )
        .into_response()
}

pub(crate) async fn spawn_retrying_capture_upstream(
    rate_limit_attempts: usize,
    retry_after: Option<&str>,
) -> (
    String,
    Arc<AtomicUsize>,
    Arc<StdMutex<Vec<Value>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen_payloads = Arc::new(StdMutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route("/v1/chat/completions", post(retrying_capture_upstream))
        .route("/v1/responses", post(retrying_capture_upstream))
        .with_state(Retry429CaptureState {
            rate_limit_attempts,
            attempts: attempts.clone(),
            seen_payloads: seen_payloads.clone(),
            retry_after: retry_after
                .map(|value| HeaderValue::from_str(value).expect("valid retry-after header")),
        });

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind retrying capture test server");
    let addr = listener
        .local_addr()
        .expect("retrying capture test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("retrying capture test server should run");
    });

    (format!("http://{addr}/"), attempts, seen_payloads, handle)
}

#[derive(Clone)]
pub(crate) struct Retry429ModelsState {
    rate_limit_attempts: usize,
    attempts: Arc<AtomicUsize>,
    retry_after: Option<HeaderValue>,
}

pub(crate) async fn retrying_models_upstream(State(state): State<Retry429ModelsState>) -> Response {
    let attempt = state.attempts.fetch_add(1, Ordering::SeqCst) + 1;

    if attempt <= state.rate_limit_attempts {
        let mut headers = HeaderMap::new();
        if let Some(retry_after) = state.retry_after.clone() {
            headers.insert(http_header::RETRY_AFTER, retry_after);
        }
        headers.insert(
            http_header::HeaderName::from_static("x-upstream-attempt"),
            HeaderValue::from_str(&attempt.to_string()).expect("valid attempt header"),
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            Json(json!({
                "error": "rate limited",
                "attempt": attempt,
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "object": "list",
            "data": [
                {
                    "id": "upstream-model-after-retry",
                    "object": "model",
                    "owned_by": "upstream",
                    "created": 1712345680
                }
            ]
        })),
    )
        .into_response()
}

pub(crate) async fn spawn_retrying_models_upstream(
    rate_limit_attempts: usize,
    retry_after: Option<&str>,
) -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
    let attempts = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/v1/models", get(retrying_models_upstream))
        .with_state(Retry429ModelsState {
            rate_limit_attempts,
            attempts: attempts.clone(),
            retry_after: retry_after
                .map(|value| HeaderValue::from_str(value).expect("valid retry-after header")),
        });

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind retrying models test server");
    let addr = listener
        .local_addr()
        .expect("retrying models test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("retrying models test server should run");
    });

    (format!("http://{addr}/"), attempts, handle)
}

pub(crate) async fn read_forward_proxy_runtime_weight(
    pool: &SqlitePool,
    proxy_key: &str,
) -> Option<f64> {
    sqlx::query_scalar::<_, f64>("SELECT weight FROM forward_proxy_runtime WHERE proxy_key = ?1")
        .bind(proxy_key)
        .fetch_optional(pool)
        .await
        .expect("read forward proxy runtime weight")
}

pub(crate) async fn test_upstream_echo(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let auth = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let host_header = headers
        .get(http_header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let connection_seen = headers.contains_key(http_header::CONNECTION);
    let x_foo_seen = headers.contains_key(http_header::HeaderName::from_static("x-foo"));
    let x_forwarded_for_seen =
        headers.contains_key(http_header::HeaderName::from_static("x-forwarded-for"));
    let forwarded_seen = headers.contains_key(http_header::HeaderName::from_static("forwarded"));
    let via_seen = headers.contains_key(http_header::HeaderName::from_static("via"));
    let accept_encoding = headers
        .get(http_header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        http_header::HeaderName::from_static("x-upstream"),
        HeaderValue::from_static("ok"),
    );
    response_headers.insert(
        http_header::CONNECTION,
        HeaderValue::from_static("x-upstream-hop"),
    );
    response_headers.insert(
        http_header::HeaderName::from_static("x-upstream-hop"),
        HeaderValue::from_static("should-be-filtered"),
    );
    response_headers.insert(
        http_header::HeaderName::from_static("via"),
        HeaderValue::from_static("1.1 upstream-proxy"),
    );
    response_headers.insert(
        http_header::HeaderName::from_static("forwarded"),
        HeaderValue::from_static("for=192.0.2.1;proto=https;host=api.example.com"),
    );

    (
        StatusCode::CREATED,
        response_headers,
        Json(json!({
            "method": method.as_str(),
            "path": uri.path(),
            "query": uri.query().unwrap_or_default(),
            "authorization": auth,
            "hostHeader": host_header,
            "connectionSeen": connection_seen,
            "xFooSeen": x_foo_seen,
            "xForwardedForSeen": x_forwarded_for_seen,
            "forwardedSeen": forwarded_seen,
            "viaSeen": via_seen,
            "acceptEncoding": accept_encoding,
            "body": body,
        })),
    )
}

pub(crate) async fn test_upstream_stream() -> impl IntoResponse {
    let chunks = stream::iter(vec![
        Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")),
        Ok::<_, Infallible>(Bytes::from_static(b"chunk-b")),
    ]);
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_stream_first_error() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((
                    Err::<Bytes, io::Error>(io::Error::other("upstream-first-chunk-error")),
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
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_stream_mid_error() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => Some((Ok::<Bytes, io::Error>(Bytes::from_static(b"chunk-a")), 1)),
            1 => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((
                    Err::<Bytes, io::Error>(io::Error::other("upstream-mid-stream-error")),
                    2,
                ))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_429_mid_error() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => Some((
                Ok::<Bytes, io::Error>(Bytes::from_static(
                    br#"{"error":{"message":"rate limited"}}"#,
                )),
                1,
            )),
            1 => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((
                    Err::<Bytes, io::Error>(io::Error::other("upstream-429-mid-stream-error")),
                    2,
                ))
            }
            _ => None,
        }
    });
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(http_header::RETRY_AFTER, HeaderValue::from_static("0"))],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_slow_stream() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")), 1)),
            1 => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-b")), 2))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

use super::*;
