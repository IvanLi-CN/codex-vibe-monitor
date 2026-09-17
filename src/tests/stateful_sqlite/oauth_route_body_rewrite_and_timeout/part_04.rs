#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_stream_survives_short_request_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
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

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
    }))
    .expect("serialize request body");
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/v1/responses?mode=slow-success"))
        .header(http_header::AUTHORIZATION, "Bearer pool-live-key")
        .header(http_header::CONTENT_TYPE, "application/json")
        .body(request_body)
        .send()
        .await
        .expect("send pool responses request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.bytes().await.expect("read pool responses body");
    let payload = String::from_utf8_lossy(&body);
    assert!(payload.contains("response.created"));
    assert!(payload.contains("response.completed"));
    assert!(payload.contains("resp_slow_test"));

    server_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_waits_for_first_chunk_beyond_request_timeout() {
    let (upstream_base, upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(100);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(600);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
    }))
    .expect("serialize request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
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
    .expect("responses pool request should not hang");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read responses pool body");
    let payload: Value = serde_json::from_slice(&body).expect("decode responses pool response");
    assert_eq!(payload["ok"].as_bool(), Some(true));

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_waits_for_headers_beyond_handshake_timeout() {
    let (upstream_base, upstream_handle) =
        spawn_pool_delayed_headers_upstream(Duration::from_millis(450)).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(900);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
    }))
    .expect("serialize request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
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
    .expect("responses pool request should not hang");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read responses pool body");
    let payload: Value = serde_json::from_slice(&body).expect("decode responses pool response");
    assert_eq!(payload["ok"].as_bool(), Some(true));
    assert_eq!(payload["phase"].as_str(), Some("headers-delayed"));

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_compact_waits_for_dedicated_first_chunk_timeout() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(700);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_001",
        "input": [{"role": "user", "content": "compact this thread"}],
    }))
    .expect("serialize compact request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state),
            OriginalUri(
                "/v1/responses/compact?mode=slow-first-chunk"
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
    .expect("compact pool request should not hang");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact pool body");
    let payload: Value = serde_json::from_slice(&body).expect("decode compact pool response");
    assert_eq!(
        payload.get("object").and_then(Value::as_str),
        Some("response.compaction")
    );
    assert_eq!(
        payload.get("id").and_then(Value::as_str),
        Some("resp_compact_test")
    );

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_still_times_out_before_first_chunk() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_secs(5);
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(400);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
    }))
    .expect("serialize responses request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state),
            OriginalUri(
                "/v1/responses?mode=slow-first-chunk"
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
    .expect("responses pool request should not hang");

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read responses pool error body");
    let payload = String::from_utf8_lossy(&body);
    assert!(payload.contains("no alternate upstream route is available after timeout"));

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_responses_stream_timeout_applies_after_first_byte() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
    }))
    .expect("serialize responses request body");
    let response = proxy_openai_v1(
        State(state),
        OriginalUri(
            "/v1/responses?mode=slow-stream-end"
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
        .expect_err("responses stream should time out after first byte");
    assert!(
        err.to_string()
            .contains("request timed out after 200ms while waiting for upstream stream completion")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_compact_stream_timeout_applies_after_first_byte() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_001",
        "input": [{"role": "user", "content": "compact this thread"}],
    }))
    .expect("serialize compact request body");
    let response = proxy_openai_v1(
        State(state),
        OriginalUri(
            "/v1/responses/compact?mode=slow-stream-end"
                .parse()
                .expect("valid compact uri"),
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
        .expect_err("compact stream should time out after first byte");
    assert!(
        err.to_string()
            .contains("request timed out after 200ms while waiting for upstream stream completion")
    );

    upstream_handle.abort();
}

#[test]
pub(crate) fn pool_route_oauth_models_preserve_response_timeout_after_headers() {
    run_oauth_future_with_large_stack(async move {
        let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;

        let (upstream_base, upstream_handle) = spawn_oauth_codex_slow_models_upstream().await;
        oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("{upstream_base}/backend-api/codex"))
                .expect("valid oauth base url"),
        )
        .await;

        let mut config = test_config();
        config.request_timeout = Duration::from_millis(200);
        let state = test_state_from_config(config, true).await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_oauth_account(&state, "Primary OAuth", "oauth-primary").await;

        let response = tokio::time::timeout(
            Duration::from_secs(3),
            proxy_openai_v1(
                State(state),
                OriginalUri("/v1/models".parse().expect("valid uri")),
                Method::GET,
                HeaderMap::from_iter([(
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                )]),
                Body::empty(),
            ),
        )
        .await
        .expect("oauth pool request should not hang");

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read oauth pool error body");
        let payload = String::from_utf8_lossy(&body);
        assert!(payload.contains("timed out"));

        upstream_handle.abort();
        oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
    });
}

#[tokio::test]
pub(crate) async fn build_account_sticky_keys_response_keeps_attached_keys_without_recent_activity()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("sticky-stale")
    .bind(account_id)
    .bind("2026-03-10 00:00:00")
    .bind("2026-03-10 00:00:00")
    .bind("2026-03-10 00:00:00")
    .execute(&state.pool)
    .await
    .expect("insert sticky route");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("sticky-stale-invoke")
    .bind("2026-03-10 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(123_i64)
    .bind(0.42_f64)
    .bind(
        json!({
            "stickyKey": "sticky-stale",
            "upstreamAccountId": account_id,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert stale sticky invocation");
    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky aggregate response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(20),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    assert_eq!(json["selectionMode"].as_str(), Some("count"));
    assert_eq!(json["selectedLimit"].as_i64(), Some(20));
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 1);
    let conversation = &conversations[0];
    assert_eq!(conversation["stickyKey"].as_str(), Some("sticky-stale"));
    assert_eq!(conversation["requestCount"].as_i64(), Some(1));
    assert_eq!(conversation["totalTokens"].as_i64(), Some(123));
    assert_eq!(
        conversation["last24hRequests"].as_array().map(Vec::len),
        Some(0)
    );
}

#[tokio::test]
pub(crate) async fn upstream_account_sticky_keys_reads_inline_rollups_without_read_time_catch_up() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let sticky_key = "sticky-inline-rollup";
    let recent_time = format_naive(
        (Utc::now() - ChronoDuration::minutes(15))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    upsert_sticky_route(&state.pool, sticky_key, account_id, &recent_time)
        .await
        .expect("seed sticky route");

    let mut record = test_proxy_capture_record("sticky-inline-rollup-invoke", &recent_time);
    record.model = Some("gpt-5.4".to_string());
    record.usage.input_tokens = Some(51);
    record.usage.output_tokens = Some(26);
    record.usage.cache_input_tokens = Some(0);
    record.usage.reasoning_tokens = Some(0);
    record.usage.total_tokens = Some(77);
    record.cost = Some(0.18);
    record.payload = Some(
        json!({
            "stickyKey": sticky_key,
            "upstreamAccountId": account_id,
            "upstreamAccountName": "Primary",
            "routeMode": "sticky",
            "endpoint": "/v1/responses",
            "model": "gpt-5.4",
        })
        .to_string(),
    );
    persist_proxy_capture_record(&state.pool, std::time::Instant::now(), record)
        .await
        .expect("persist sticky proxy capture")
        .expect("sticky invocation should persist");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(20),
    )
    .await
    .expect("sticky-key endpoint should stay fresh without read-time catch-up");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 1);
    let conversation = &conversations[0];
    assert_eq!(conversation["stickyKey"].as_str(), Some(sticky_key));
    assert_eq!(conversation["requestCount"].as_i64(), Some(1));
    assert_eq!(conversation["totalTokens"].as_i64(), Some(77));
    assert_eq!(conversation["totalCost"].as_f64(), Some(0.18));
    assert_eq!(
        conversation["recentInvocations"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        conversation["recentInvocations"][0]["invokeId"].as_str(),
        Some("sticky-inline-rollup-invoke")
    );
}

#[test]
pub(crate) fn resolve_sticky_key_selection_rejects_mutually_exclusive_params() {
    let err = crate::upstream_accounts::resolve_sticky_key_selection(&AccountStickyKeysQuery {
        limit: Some(20),
        activity_hours: Some(3),
    })
    .expect_err("selection should reject mutually exclusive params");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("provide either limit or activityHours"));
}

#[test]
pub(crate) fn resolve_sticky_key_selection_rejects_unsupported_activity_hours() {
    let err = crate::upstream_accounts::resolve_sticky_key_selection(&AccountStickyKeysQuery {
        limit: None,
        activity_hours: Some(2),
    })
    .expect_err("selection should reject unsupported activityHours");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1
            .contains("activityHours must be one of 1, 3, 6, 12, or 24")
    );
}

#[tokio::test]
pub(crate) async fn build_account_sticky_keys_response_activity_window_filters_recent_keys_only() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let recent_time = format_naive(
        (Utc::now() - ChronoDuration::hours(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let stale_time = format_naive(
        (Utc::now() - ChronoDuration::hours(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    for (sticky_key, occurred_at) in [
        ("sticky-window-recent", recent_time.as_str()),
        ("sticky-window-stale", stale_time.as_str()),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(sticky_key)
        .bind(account_id)
        .bind(occurred_at)
        .bind(occurred_at)
        .bind(occurred_at)
        .execute(&state.pool)
        .await
        .expect("insert sticky route");
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("{sticky_key}-invoke"))
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(64_i64)
        .bind(0.08_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky activity-window response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::ActivityWindow(3),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    assert_eq!(json["selectionMode"].as_str(), Some("activityWindow"));
    assert_eq!(json["selectedLimit"], Value::Null);
    assert_eq!(json["selectedActivityHours"].as_i64(), Some(3));
    assert_eq!(
        json["implicitFilter"]["kind"].as_str(),
        Some("inactiveOutside24h")
    );
    assert_eq!(json["implicitFilter"]["filteredCount"].as_i64(), Some(1));
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 1);
    assert_eq!(
        conversations[0]["stickyKey"].as_str(),
        Some("sticky-window-recent")
    );
}

#[tokio::test]
pub(crate) async fn build_account_sticky_keys_response_activity_window_previews_respect_time_window()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let sticky_key = "sticky-window-preview";
    let recent_time = format_naive(
        (Utc::now() - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let stale_time = format_naive(
        (Utc::now() - ChronoDuration::hours(8))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(&stale_time)
    .bind(&recent_time)
    .bind(&recent_time)
    .execute(&state.pool)
    .await
    .expect("insert sticky route");

    for (invoke_id, occurred_at) in [
        ("sticky-window-preview-recent", recent_time.as_str()),
        ("sticky-window-preview-stale", stale_time.as_str()),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(64_i64)
        .bind(0.08_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky preview response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::ActivityWindow(3),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let previews = json["conversations"][0]["recentInvocations"]
        .as_array()
        .expect("recent invocations array");
    assert_eq!(previews.len(), 1);
    assert_eq!(
        previews[0]["invokeId"].as_str(),
        Some("sticky-window-preview-recent")
    );
}

#[tokio::test]
pub(crate) async fn build_account_sticky_keys_response_activity_window_caps_results_to_fifty() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let now = Utc::now();

    for index in 0..55 {
        let occurred_at = format_naive(
            (now - ChronoDuration::minutes(index as i64))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let sticky_key = format!("sticky-window-cap-{index}");
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(&sticky_key)
        .bind(account_id)
        .bind(&occurred_at)
        .bind(&occurred_at)
        .bind(&occurred_at)
        .execute(&state.pool)
        .await
        .expect("insert sticky route");
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("sticky-window-cap-invoke-{index}"))
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(64_i64)
        .bind(0.08_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky activity-window response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::ActivityWindow(3),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 50);
    assert_eq!(json["implicitFilter"]["kind"].as_str(), Some("cappedTo50"));
    assert_eq!(json["implicitFilter"]["filteredCount"].as_i64(), Some(5));
}

#[tokio::test]
pub(crate) async fn build_account_sticky_keys_response_includes_recent_invocations_sorted_and_capped()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let sticky_key = "sticky-preview-cap";

    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(format_naive(
        (Utc::now() - ChronoDuration::minutes(6))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .execute(&state.pool)
    .await
    .expect("insert sticky preview route");

    for minutes_ago in 0..6 {
        let occurred_at = format_naive(
            (Utc::now() - ChronoDuration::minutes(minutes_ago))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("sticky-preview-invoke-{minutes_ago}"))
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(100_i64 + minutes_ago)
        .bind(0.01_f64 + (minutes_ago as f64) * 0.001_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
                "endpoint": "/v1/responses",
                "routeMode": "sticky",
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert preview invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky preview response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(20),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let previews = json["conversations"][0]["recentInvocations"]
        .as_array()
        .expect("recent invocations array");
    assert_eq!(previews.len(), 5);
    assert_eq!(
        previews[0]["invokeId"].as_str(),
        Some("sticky-preview-invoke-0")
    );
    assert_eq!(
        previews[4]["invokeId"].as_str(),
        Some("sticky-preview-invoke-4")
    );
}

use super::*;
