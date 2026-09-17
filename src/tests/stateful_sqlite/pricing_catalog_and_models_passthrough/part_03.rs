#[tokio::test]
pub(crate) async fn capture_targets_reject_non_pool_requests_before_proxying() {
    let state = test_state_with_openai_base(
        Url::parse("https://example.invalid").expect("valid upstream base url"),
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello",
                "stream": false
            }))
            .expect("serialize request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy error body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert_eq!(
        payload["error"].as_str(),
        Some("pool route key missing or invalid")
    );
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn read_request_body_timeout_returns_408() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state = test_state_with_openai_base_body_limit_and_read_timeout(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(50),
    )
    .await;

    let slow_body = stream::unfold(0u8, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<Bytes, Infallible>(Bytes::from_static(br#"{}"#)), 1))
            }
            _ => None,
        }
    });

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(slow_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("request body read timed out")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("failed"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[request_body_read_timeout]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("request_body_read_timeout")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn capture_target_retries_429_and_persists_single_invocation() {
    let (upstream_base, attempts, seen_payloads, upstream_handle) =
        spawn_retrying_capture_upstream(1, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 1;
    }

    let request_payload = json!({
        "model": "gpt-5.2-codex",
        "stream": false,
        "input": "hello",
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(
            serde_json::to_vec(&request_payload).expect("serialize capture retry request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode capture retry payload");
    assert_eq!(payload["attempt"], 2);
    assert_eq!(payload["received"], request_payload);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        seen_payloads
            .lock()
            .expect("lock retrying capture payloads")
            .clone(),
        vec![request_payload.clone(), request_payload.clone()]
    );

    let mut invocation_count: i64 = 0;
    let mut attempt_count: i64 = 0;
    let mut rate_limit_count: i64 = 0;
    for _ in 0..20 {
        invocation_count = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&state.pool)
            .await
            .expect("count persisted invocations");
        attempt_count = count_request_forward_proxy_attempts(&state.pool).await;
        rate_limit_count = count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await;

        if invocation_count == 1 && attempt_count == 2 && rate_limit_count == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert_eq!(invocation_count, 1);
    assert_eq!(attempt_count, 2);
    assert_eq!(rate_limit_count, 1);

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn capture_target_client_body_disconnect_returns_400_with_failure_kind() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let disconnected_body = stream::iter(vec![Err::<Bytes, io::Error>(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "client disconnected",
    ))]);

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(disconnected_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("failed to read request body stream")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("failed"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[request_body_stream_error_client_closed]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("request_body_stream_error_client_closed")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn capture_target_stream_error_emits_failure_kind_and_persists() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.2-codex",
        "stream": true,
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
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("mid-stream upstream failure should surface to downstream");
    assert!(
        err.to_string().contains("upstream stream error"),
        "unexpected stream error text: {err}"
    );

    let mut row: Option<PersistedRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT status, error_message, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_stream_error]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("upstream_stream_error")
    );

    upstream_handle.abort();
}

#[derive(sqlx::FromRow)]
struct PersistedFailedStreamRow {
    status: Option<String>,
    error_message: Option<String>,
    payload: Option<String>,
}

async fn wait_for_failed_stream_row(pool: &SqlitePool) -> PersistedFailedStreamRow {
    for _ in 0..20 {
        let row = sqlx::query_as::<_, PersistedFailedStreamRow>(
            r#"
            SELECT status, error_message, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await
        .expect("query capture record");
        if let Some(row) = row {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("capture record should be persisted");
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn capture_target_response_failed_stream_persists_service_failure_details() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello"
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
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let text = String::from_utf8(body.to_vec()).expect("response body should be utf8");
    assert!(text.contains("response.failed"));

    let row = wait_for_failed_stream_row(&state.pool).await;

    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 1);
    assert_eq!(
        count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
        )
        .await,
        1
    );

    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_response_failed] server_error"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("upstream_response_failed")
    );
    assert_eq!(
        payload_json["streamTerminalEvent"].as_str(),
        Some("response.failed")
    );
    assert_eq!(
        payload_json["upstreamErrorCode"].as_str(),
        Some("server_error")
    );
    assert!(
        payload_json["upstreamErrorMessage"]
            .as_str()
            .is_some_and(|msg| msg.contains("request ID 060a328d-5cb6-433c-9025-1da2d9c632f1"))
    );
    assert_eq!(
        payload_json["upstreamRequestId"].as_str(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );

    upstream_handle.abort();
}

#[derive(sqlx::FromRow)]
struct PersistedCompactRow {
    endpoint: Option<String>,
    model: Option<String>,
    requested_service_tier: Option<String>,
    input_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    price_version: Option<String>,
}

async fn wait_for_compact_row(pool: &SqlitePool) -> PersistedCompactRow {
    for _ in 0..20 {
        let row = sqlx::query_as::<_, PersistedCompactRow>(
            r#"
            SELECT
                CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
                model,
                CASE
                  WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                    THEN json_extract(payload, '$.requestedServiceTier')
                  WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                    THEN json_extract(payload, '$.requested_service_tier')
                END AS requested_service_tier,
                input_tokens,
                cache_input_tokens,
                output_tokens,
                reasoning_tokens,
                total_tokens,
                cost,
                price_version
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await
        .expect("query compact capture record");
        if let Some(row) = row {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("compact capture record should be persisted");
}

fn assert_compact_row(row: &PersistedCompactRow) {
    assert_eq!(row.endpoint.as_deref(), Some("/v1/responses/compact"));
    assert_eq!(row.model.as_deref(), Some("gpt-5.1-codex-max"));
    assert_eq!(row.requested_service_tier.as_deref(), Some("flex"));
    assert_eq!(row.input_tokens, Some(139));
    assert_eq!(row.cache_input_tokens, Some(11));
    assert_eq!(row.output_tokens, Some(438));
    assert_eq!(row.reasoning_tokens, Some(64));
    assert_eq!(row.total_tokens, Some(577));
    assert_eq!(row.price_version.as_deref(), Some("compact-unit-test"));
    assert_f64_close(row.cost.expect("compact cost should be present"), 0.0020235);
}

async fn assert_compact_aggregate_stats(state: Arc<AppState>) {
    let Json(stats) = fetch_stats(State(state.clone()))
        .await
        .expect("compact fetch_stats should succeed");
    assert_eq!(stats.total_count, 1);
    assert_eq!(stats.success_count, 1);
    assert_eq!(stats.failure_count, 0);
    assert_eq!(stats.total_tokens, 577);
    assert_f64_close(stats.total_cost, 0.0020235);

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: None,
            upstream_account_id: None,
        }),
    )
    .await
    .expect("compact fetch_summary should succeed");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.total_tokens, 577);
    assert_f64_close(summary.total_cost, 0.0020235);

    let Json(timeseries) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: None,
            upstream_account_id: None,
        }),
    )
    .await
    .expect("compact fetch_timeseries should succeed");
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_count)
            .sum::<i64>(),
        1
    );
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_tokens)
            .sum::<i64>(),
        577
    );
    assert_f64_close(
        timeseries
            .points
            .iter()
            .map(|point| point.total_cost)
            .sum::<f64>(),
        0.0020235,
    );
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_compact_estimates_cost_and_flows_into_stats_without_rewrite()
 {
    let (upstream_base, captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut pricing = state.pricing_catalog.write().await;
        *pricing = PricingCatalog {
            version: "compact-unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.1-codex-max".to_string(),
                ModelPricing {
                    input_per_1m: 2.0,
                    output_per_1m: 3.0,
                    cache_input_per_1m: Some(0.5),
                    cache_read_per_1m: Some(0.5),
                    cache_write_per_1m: None,
                    reasoning_per_1m: Some(7.0),
                    source: "custom".to_string(),
                },
            )]),
        };
    }

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "serviceTier": "flex",
        "previous_response_id": "resp_prev_001",
        "input": [{
            "role": "user",
            "content": "compact this thread"
        }]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses/compact".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let captured = captured_requests.lock().await;
    let captured_request = captured
        .first()
        .cloned()
        .expect("upstream should receive a compact request body");
    drop(captured);
    assert_eq!(captured_request["serviceTier"], "flex");
    assert!(captured_request.get("service_tier").is_none());

    let row = wait_for_compact_row(&state.pool).await;
    assert_compact_row(&row);
    assert_compact_aggregate_stats(state).await;

    upstream_handle.abort();
}

fn pool_routing_timeout_test_config() -> AppConfig {
    let mut config = test_config();
    config.request_timeout = Duration::from_secs(61);
    config.pool_upstream_responses_attempt_timeout = Duration::from_secs(121);
    config.pool_upstream_responses_total_timeout = Duration::from_secs(301);
    config.openai_proxy_handshake_timeout = Duration::from_secs(71);
    config.openai_proxy_compact_handshake_timeout = Duration::from_secs(305);
    config.openai_proxy_image_handshake_timeout = Duration::from_secs(306);
    config.openai_proxy_request_read_timeout = Duration::from_secs(181);
    config
}

async fn load_unresolved_timeout_row(
    pool: &SqlitePool,
) -> (
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
) {
    sqlx::query_as(
        r#"
        SELECT
            responses_first_byte_timeout_secs,
            compact_first_byte_timeout_secs,
            image_first_byte_timeout_secs,
            responses_stream_timeout_secs,
            compact_stream_timeout_secs
        FROM pool_routing_settings
        WHERE id = 1
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("load unresolved timeout row")
}

async fn assert_resolved_timeout_settings(state: &AppState) {
    let resolved = resolve_pool_routing_timeouts(&state.pool, &state.config)
        .await
        .expect("resolve updated pool routing timeouts");
    assert_eq!(resolved.default_first_byte_timeout, Duration::from_secs(61));
    assert_eq!(
        resolved.responses_first_byte_timeout,
        Duration::from_secs(135)
    );
    assert_eq!(
        resolved.compact_first_byte_timeout,
        Duration::from_secs(325)
    );
    assert_eq!(resolved.image_first_byte_timeout, Duration::from_secs(300));
    assert_eq!(resolved.responses_stream_timeout, Duration::from_secs(405));
    assert_eq!(resolved.compact_stream_timeout, Duration::from_secs(505));
    assert_eq!(resolved.default_send_timeout, Duration::from_secs(71));
    assert_eq!(resolved.request_read_timeout, Duration::from_secs(181));
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_compact_uses_dedicated_handshake_timeout() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let state = test_state_with_openai_base_and_proxy_timeouts(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(100),
        Duration::from_millis(400),
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS),
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "previous_response_id": "resp_prev_001",
        "input": [{"role": "user", "content": "compact this thread"}]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state),
        OriginalUri(
            "/v1/responses/compact?mode=delay"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    upstream_handle.abort();
}

#[test]
pub(crate) fn pool_upstream_first_chunk_timeout_uses_compact_budget_for_compact_route() {
    let mut config = test_config();
    config.request_timeout = Duration::from_millis(200);
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(400);
    let timeouts = pool_routing_timeouts_from_config(&config);

    let timeout = pool_upstream_first_chunk_timeout(
        &timeouts,
        &"/v1/responses/compact".parse().expect("valid uri"),
        &Method::POST,
    );

    assert_eq!(timeout, Duration::from_millis(400));
}

#[test]
pub(crate) fn pool_upstream_first_chunk_timeout_uses_responses_budget_for_responses_route() {
    let mut config = test_config();
    config.request_timeout = Duration::from_millis(200);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(1200);
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(400);
    let timeouts = pool_routing_timeouts_from_config(&config);

    let timeout = pool_upstream_first_chunk_timeout(
        &timeouts,
        &"/v1/responses".parse().expect("valid uri"),
        &Method::POST,
    );

    assert_eq!(timeout, Duration::from_millis(1200));
}

#[test]
pub(crate) fn image_routes_use_dedicated_send_and_first_chunk_budget() {
    let mut config = test_config();
    config.request_timeout = Duration::from_millis(200);
    config.openai_proxy_image_handshake_timeout = Duration::from_millis(900);
    let timeouts = pool_routing_timeouts_from_config(&config);

    for (path, target) in [
        (
            "/v1/images/generations",
            ProxyCaptureTarget::ImageGenerations,
        ),
        ("/v1/images/edits", ProxyCaptureTarget::ImageEdits),
    ] {
        assert_eq!(
            proxy_upstream_send_timeout_for_capture_target(&timeouts, Some(target)),
            Duration::from_millis(900),
        );
        assert_eq!(
            pool_upstream_first_chunk_timeout(
                &timeouts,
                &path.parse().expect("valid image uri"),
                &Method::POST,
            ),
            Duration::from_millis(900),
        );
    }
}

#[test]
pub(crate) fn pool_upstream_send_timeout_uses_responses_budget_for_responses_route() {
    let handshake_timeout = Duration::from_millis(100);
    let responses_timeout = Duration::from_millis(1200);

    let timeout = pool_upstream_send_timeout(
        &"/v1/responses".parse().expect("valid uri"),
        &Method::POST,
        handshake_timeout,
        responses_timeout,
    );

    assert_eq!(timeout, responses_timeout);
}

#[test]
pub(crate) fn pool_upstream_first_chunk_timeout_keeps_default_budget_for_non_responses_route() {
    let mut config = test_config();
    config.request_timeout = Duration::from_millis(200);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(1200);
    let timeouts = pool_routing_timeouts_from_config(&config);

    let timeout = pool_upstream_first_chunk_timeout(
        &timeouts,
        &"/v1/chat/completions".parse().expect("valid uri"),
        &Method::POST,
    );

    assert_eq!(timeout, Duration::from_millis(200));
}

#[test]
pub(crate) fn pool_upstream_send_timeout_caps_non_responses_route_by_first_byte_budget() {
    let handshake_timeout = Duration::from_millis(1200);
    let first_byte_timeout = Duration::from_millis(100);

    let timeout = pool_upstream_send_timeout(
        &"/v1/chat/completions".parse().expect("valid uri"),
        &Method::POST,
        handshake_timeout,
        first_byte_timeout,
    );

    assert_eq!(timeout, first_byte_timeout);
}

#[test]
pub(crate) fn classify_compact_support_observation_is_conservative() {
    let compact_uri: Uri = "/v1/responses/compact".parse().expect("valid compact uri");

    let supported = classify_compact_support_observation(&compact_uri, Some(StatusCode::OK), None)
        .expect("compact success observation");
    assert_eq!(supported.status, COMPACT_SUPPORT_STATUS_SUPPORTED);

    let unsupported = classify_compact_support_observation(
        &compact_uri,
        Some(StatusCode::SERVICE_UNAVAILABLE),
        Some("No available channel for model gpt-5.4-openai-compact under group default (distributor)"),
    )
    .expect("compact unsupported observation");
    assert_eq!(unsupported.status, COMPACT_SUPPORT_STATUS_UNSUPPORTED);

    let unknown = classify_compact_support_observation(
        &compact_uri,
        None,
        Some("upstream handshake timed out after 300000ms"),
    )
    .expect("compact unknown observation");
    assert_eq!(unknown.status, COMPACT_SUPPORT_STATUS_UNKNOWN);

    assert!(
        classify_compact_support_observation(
            &"/v1/responses".parse().expect("valid responses uri"),
            Some(StatusCode::OK),
            None,
        )
        .is_none()
    );
}

#[tokio::test]
pub(crate) async fn pool_routing_settings_backfill_defaults_and_persist_timeout_updates() {
    let _priority_handoff_guard = crate::upstream_accounts::priority_handoff_test_guard();
    let config = pool_routing_timeout_test_config();
    let state = test_state_from_config(config.clone(), true).await;

    let Json(initial) = get_pool_routing_settings(State(state.clone()))
        .await
        .expect("load initial pool routing settings");
    assert_eq!(initial.timeouts.responses_first_byte_timeout_secs, 121);
    assert_eq!(initial.timeouts.compact_first_byte_timeout_secs, 305);
    assert_eq!(initial.timeouts.image_first_byte_timeout_secs, 306);
    assert_eq!(initial.timeouts.responses_stream_timeout_secs, 301);
    assert_eq!(initial.timeouts.compact_stream_timeout_secs, 301);
    assert!(initial.priority_handoff_admission_enabled);

    let persisted = load_unresolved_timeout_row(&state.pool).await;
    assert_eq!(persisted.0, None);
    assert_eq!(persisted.1, None);
    assert_eq!(persisted.2, None);
    assert_eq!(persisted.3, None);
    assert_eq!(persisted.4, None);

    let payload = UpdatePoolRoutingSettingsRequest {
        api_key: None,
        maintenance: None,
        request_compression_algorithm: None,
        request_compression_level_preset: None,
        codex_imagegen_rewrite_mode: None,
        available_models: None,
        available_models_mode: None,
        cache_hit_protection: None,
        priority_handoff_admission_enabled: Some(false),
        timeouts: Some(UpdatePoolRoutingTimeoutSettingsRequest {
            responses_first_byte_timeout_secs: Some(135),
            compact_first_byte_timeout_secs: Some(325),
            image_first_byte_timeout_secs: Some(300),
            responses_stream_timeout_secs: Some(405),
            compact_stream_timeout_secs: Some(505),
        }),
    };
    let Json(updated) =
        update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("update pool routing timeouts");
    assert_eq!(updated.timeouts.responses_first_byte_timeout_secs, 135);
    assert_eq!(updated.timeouts.compact_first_byte_timeout_secs, 325);
    assert_eq!(updated.timeouts.image_first_byte_timeout_secs, 300);
    assert_eq!(updated.timeouts.responses_stream_timeout_secs, 405);
    assert_eq!(updated.timeouts.compact_stream_timeout_secs, 505);
    assert!(!updated.priority_handoff_admission_enabled);

    let Json(reloaded) = get_pool_routing_settings(State(state.clone()))
        .await
        .expect("reload pool routing settings after admission update");
    assert!(!reloaded.priority_handoff_admission_enabled);

    sqlx::query(
        "UPDATE pool_routing_settings SET priority_handoff_admission_enabled = 1 WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("simulate stale persisted admission setting");
    let Json(local_mirror) = get_pool_routing_settings(State(state.clone()))
        .await
        .expect("read local admission mirror after stale persistence");
    assert!(!local_mirror.priority_handoff_admission_enabled);

    let _ = update_pool_routing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(UpdatePoolRoutingSettingsRequest {
            api_key: None,
            maintenance: None,
            request_compression_algorithm: None,
            request_compression_level_preset: None,
            codex_imagegen_rewrite_mode: None,
            available_models: None,
            available_models_mode: None,
            cache_hit_protection: None,
            priority_handoff_admission_enabled: Some(true),
            timeouts: None,
        }),
    )
    .await
    .expect("restore priority handoff admission setting");

    assert_resolved_timeout_settings(&state).await;
}

use super::*;
