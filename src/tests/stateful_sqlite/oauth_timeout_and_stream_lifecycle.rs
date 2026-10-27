use super::oauth_route_support_and_binding::run_oauth_future_with_large_stack;
use super::*;
use serde_json::json;

#[tokio::test]
async fn oauth_responses_timeout_marks_transport_failure_header() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) =
        spawn_oauth_codex_delayed_headers_upstream(Duration::from_millis(250)).await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let oauth_response = oauth_bridge::send_oauth_upstream_request(
        &reqwest::Client::new(),
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        oauth_bridge::OauthUpstreamRequestBody::Bytes(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello"
            }))
            .expect("serialize oauth responses body"),
        )),
        Duration::from_millis(100),
        Duration::from_millis(120),
        Some(7),
        "oauth-timeout",
        Some("02355c9d-fb23-4517-a96d-35e5f6758e9e"),
        None,
        None,
    )
    .await;

    assert_eq!(oauth_response.response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        oauth_bridge::oauth_transport_failure_kind(oauth_response.response.headers()),
        Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT),
    );
    let body = to_bytes(oauth_response.response.into_body(), usize::MAX)
        .await
        .expect("read oauth timeout response body");
    let payload = String::from_utf8_lossy(&body);
    assert!(payload.contains("timed out"));

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_oauth_responses_timeout_switches_to_alternate_route() {
    #[derive(sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    #[derive(sqlx::FromRow)]
    struct AttemptRow {
        status: String,
        http_status: Option<i64>,
        downstream_http_status: Option<i64>,
        failure_kind: Option<String>,
        error_message: Option<String>,
        downstream_error_message: Option<String>,
    }

    #[derive(sqlx::FromRow)]
    struct RouteStateRow {
        last_action_reason_code: Option<String>,
        last_action_http_status: Option<i64>,
        last_route_failure_kind: Option<String>,
    }

    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (slow_upstream_base, slow_upstream_handle) =
        spawn_oauth_codex_delayed_headers_upstream(Duration::from_millis(700)).await;
    let (fast_upstream_base, _attempts, fast_upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-fast", 0)]).await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{slow_upstream_base}/backend-api/codex"))
            .expect("valid oauth base url"),
    )
    .await;

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(450);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let oauth_id = insert_test_pool_oauth_account(&state, "Timeout OAuth", "oauth-timeout").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Fast Route",
        "route-fast",
        None,
        None,
        Some(fast_upstream_base.as_str()),
    )
    .await;
    let sticky_last_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-oauth-timeout-switch",
        oauth_id,
        &sticky_last_seen_at,
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
        "stickyKey": "sticky-oauth-timeout-switch",
    }))
    .expect("serialize request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state.clone()),
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
    .expect("oauth timeout failover request should not hang");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read oauth timeout failover body");
    let payload: Value = serde_json::from_slice(&body).expect("decode oauth timeout failover body");
    assert_eq!(payload["ok"].as_bool(), Some(true));

    wait_for_codex_invocations(&state.pool, 1).await;

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load oauth timeout failover payload");
    let persisted_payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("oauth timeout failover payload should be present"),
    )
    .expect("decode oauth timeout failover payload");
    assert_eq!(persisted_payload["poolAttemptCount"].as_i64(), Some(2));
    assert_eq!(
        persisted_payload["poolDistinctAccountCount"].as_i64(),
        Some(2)
    );
    assert!(persisted_payload["poolAttemptTerminalReason"].is_null());
    let oauth_route_state = sqlx::query_as::<_, RouteStateRow>(
        r#"
        SELECT
            last_action_reason_code,
            last_action_http_status,
            last_route_failure_kind
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(oauth_id)
    .fetch_one(&state.pool)
    .await
    .expect("load oauth timeout route state");
    assert_eq!(
        oauth_route_state.last_action_reason_code.as_deref(),
        Some("transport_failure")
    );
    assert_eq!(oauth_route_state.last_action_http_status, Some(502));
    assert_eq!(
        oauth_route_state.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    let failed_attempt = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT
            status,
            http_status,
            downstream_http_status,
            failure_kind,
            error_message,
            downstream_error_message
        FROM pool_upstream_request_attempts
        WHERE invoke_id = (
            SELECT invoke_id
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
        )
          AND status != ?1
        ORDER BY id ASC
        LIMIT 1
        "#,
    )
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
    .fetch_one(&state.pool)
    .await
    .expect("load oauth timeout failed attempt");
    assert_eq!(
        failed_attempt.status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(failed_attempt.http_status, None);
    assert_eq!(failed_attempt.downstream_http_status, Some(502));
    assert_eq!(
        failed_attempt.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
    );
    assert!(
        failed_attempt
            .error_message
            .as_deref()
            .is_some_and(|value| !value.contains("pool upstream responded with 502"))
    );
    assert!(
        failed_attempt
            .downstream_error_message
            .as_deref()
            .is_some_and(|value| value.contains("pool upstream responded with 502"))
    );

    slow_upstream_handle.abort();
    fast_upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_oauth_compact_stream_timeout_does_not_cap_send_phase_before_first_byte() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(600);
    config.pool_upstream_responses_total_timeout = Duration::from_millis(300);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_oauth_account(&state, "Compact OAuth", "oauth-compact").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses/compact?mode=delay"
                .parse()
                .expect("valid compact uri"),
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
        Body::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "compact me slowly"}]
            }))
            .expect("serialize oauth compact timeout body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read oauth compact success response"),
    )
    .expect("decode oauth compact success response");
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/responses/compact"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    let failure_kind: Option<String> = sqlx::query_scalar(
        r#"
        SELECT json_extract(payload, '$.failureKind')
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load oauth compact success failure kind");
    assert_eq!(failure_kind, None);

    upstream_handle.abort();
}
#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_e2e_stream_survives_short_request_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        system_storage: Arc::new(SystemStorageRuntime::new(&config)),
        config: config.clone(),
        sqlite_batch_writer: SqliteBatchWriter::spawn_for_test(),
        pool_account_selection_runtime: Arc::new(PoolAccountSelectionRuntime::default()),
        proxy_runtime_invocations: Arc::new(ProxyRuntimeInvocationStore::default()),
        proxy_runtime_attempts: Arc::new(RuntimeAttemptStore::default()),
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
        observability: ObservabilityRuntime::disabled_for_tests(),
        dashboard_network_speed_cache: Arc::new(
            crate::dashboard_network_speed::DashboardNetworkSpeedCache::new(chrono::Utc::now()),
        ),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore: Arc::new(Semaphore::new(config.max_parallel_polls)),
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
        proxy_raw_async_semaphore: Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config))),
        raw_capture_circuit: Arc::new(RawCaptureCircuitBreaker::new(
            config.resolved_proxy_raw_dir(),
        )),
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
    });

    let app = Router::new()
        .route("/v1/{*path}", any(proxy_openai_v1))
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
    upstream_handle.abort();
}

#[test]
fn pool_openai_v1_e2e_stream_survives_short_request_timeout() {
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
            .route("/v1/{*path}", any(proxy_openai_v1))
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
fn pool_openai_v1_times_out_before_first_chunk_with_short_request_timeout() {
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
            .route("/v1/{*path}", any(proxy_openai_v1))
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

#[tokio::test]
async fn pool_openai_v1_responses_stream_survives_short_request_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let app = Router::new()
        .route("/v1/{*path}", any(proxy_openai_v1))
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
async fn pool_openai_v1_responses_waits_for_first_chunk_beyond_request_timeout() {
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
async fn pool_openai_v1_responses_waits_for_headers_beyond_handshake_timeout() {
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
async fn pool_openai_v1_responses_compact_waits_for_dedicated_first_chunk_timeout() {
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
async fn pool_openai_v1_responses_still_times_out_before_first_chunk() {
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
async fn proxy_capture_target_responses_stream_timeout_applies_after_first_byte() {
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
async fn proxy_capture_target_compact_stream_timeout_applies_after_first_byte() {
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
fn pool_route_oauth_models_preserve_response_timeout_after_headers() {
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
