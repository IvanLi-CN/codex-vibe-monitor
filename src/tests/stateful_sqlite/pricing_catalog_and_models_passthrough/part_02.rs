#[tokio::test]
async fn proxy_openai_v1_models_returns_preset_when_hijack_enabled_without_merge() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let headers = seed_pool_models_route(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.3-codex".to_string(), "gpt-5.2".to_string()],
            ..ProxyModelSettings::default()
        };
    }
    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        headers,
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get(PROXY_MODEL_MERGE_STATUS_HEADER)
            .is_none()
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode hijacked payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(
        ids,
        vec!["gpt-5.3-codex".to_string(), "gpt-5.2".to_string()]
    );

    upstream_handle.abort();
}
#[tokio::test]
async fn proxy_openai_v1_models_returns_gpt_5_4_models_when_enabled() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let headers = seed_pool_models_route(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.4".to_string(), "gpt-5.4-pro".to_string()],
            ..ProxyModelSettings::default()
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        headers,
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode hijacked payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(ids, vec!["gpt-5.4".to_string(), "gpt-5.4-pro".to_string()]);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_merges_upstream_when_enabled() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let headers = seed_pool_models_route(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec![
                "gpt-5.2-codex".to_string(),
                "gpt-5.1-codex-mini".to_string(),
            ],
            ..ProxyModelSettings::default()
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        headers,
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_SUCCESS))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode merged payload");
    let ids = extract_model_ids(&payload);

    assert!(ids.contains(&"upstream-model-a".to_string()));
    assert!(ids.contains(&"gpt-5.2-codex".to_string()));
    assert!(ids.contains(&"gpt-5.1-codex-mini".to_string()));
    assert!(!ids.contains(&"gpt-5.3-codex".to_string()));
    assert_eq!(
        ids.iter()
            .filter(|id| id.as_str() == "gpt-5.2-codex")
            .count(),
        1
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_applies_hijack_for_pool_route_requests() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let headers = seed_pool_models_route(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
            ..ProxyModelSettings::default()
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        headers,
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_SUCCESS))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read models body");
    let payload: Value = serde_json::from_slice(&body).expect("decode models payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(ids[0], "gpt-5.1-codex-mini".to_string());
    assert!(ids.contains(&"upstream-model-a".to_string()));
    assert!(ids.contains(&"gpt-5.2-codex".to_string()));

    upstream_handle.abort();
}

#[test]
fn proxy_openai_v1_models_pool_failures_do_not_return_untracked_cvm_id() {
    run_pricing_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_retrying_models_upstream(99, Some("0")).await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/models".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().get(CVM_INVOKE_ID_HEADER).is_none());
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read models failure body");
        let payload: Value = serde_json::from_slice(&body).expect("decode models failure payload");
        assert_eq!(
            payload["error"].as_str(),
            Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
        );
        assert!(payload.get("cvmId").is_none());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn proxy_openai_v1_models_merges_upstream_after_429_retry() {
    let (upstream_base, attempts, upstream_handle) = spawn_retrying_models_upstream(1, None).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let headers = seed_pool_models_route(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: 1,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
            ..ProxyModelSettings::default()
        };
    }

    let started = Instant::now();
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        headers,
        Body::empty(),
    )
    .await;

    assert!(
        started.elapsed() < Duration::from_millis(250),
        "test-only 429 retry delay override should apply when upstream omits Retry-After"
    );

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_SUCCESS))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode merged payload");
    let ids = extract_model_ids(&payload);
    assert!(ids.contains(&"upstream-model-after-retry".to_string()));
    assert!(ids.contains(&"gpt-5.1-codex-mini".to_string()));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
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
async fn proxy_openai_v1_models_falls_back_to_preset_when_merge_upstream_fails() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let headers = seed_pool_models_route(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
            ..ProxyModelSettings::default()
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models?mode=error".parse().expect("valid uri")),
        Method::GET,
        headers,
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(ids, vec!["gpt-5.1-codex-mini".to_string()]);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_retries_429_then_falls_back_once_exhausted() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_retrying_models_upstream(99, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let headers = seed_pool_models_route(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: 2,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
            ..ProxyModelSettings::default()
        };
    }

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        headers,
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read fallback response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
    assert_eq!(
        extract_model_ids(&payload),
        vec!["gpt-5.1-codex-mini".to_string()]
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
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
async fn proxy_openai_v1_models_falls_back_when_merge_body_decode_times_out() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = test_current_schema_pool().await;

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        sqlite_batch_writer: SqliteBatchWriter::spawn_for_test(),
        pool_account_selection_runtime: Arc::new(PoolAccountSelectionRuntime::default()),
        proxy_runtime_invocations: Arc::new(ProxyRuntimeInvocationStore::default()),
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
        dashboard_network_speed_cache: Arc::new(
            crate::dashboard_network_speed::DashboardNetworkSpeedCache::new(chrono::Utc::now()),
        ),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
        proxy_raw_async_semaphore: Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config))),
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
            ..ProxyModelSettings::default()
        })),
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
        #[cfg(test)]
        pool_routing_test_data_version_connection: Arc::new(Mutex::new(None)),
        pool_model_routing_cache_write_lock: Arc::new(Mutex::new(())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        pool_group_429_retry_delay_override: None,
        fallback_proxy_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    });
    let headers = seed_pool_models_route(&state).await;

    assert_models_timeout_fallback(&state, headers).await;

    upstream_handle.abort();
}

async fn assert_models_timeout_fallback(state: &Arc<AppState>, headers: HeaderMap) {
    let started = Instant::now();
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/models?mode=slow-body".parse().expect("valid uri")),
        Method::GET,
        headers,
        Body::empty(),
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read fallback response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
    assert_eq!(
        extract_model_ids(&payload),
        vec!["gpt-5.1-codex-mini".to_string()]
    );
}

#[derive(sqlx::FromRow)]
struct PersistedResponseFailedRow {
    status: Option<String>,
    error_message: Option<String>,
    payload: Option<String>,
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

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_preserves_streaming_response() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/stream".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(http_header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/event-stream"))
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read stream body");
    assert_eq!(&body[..], b"chunk-achunk-b");

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_returns_bad_gateway_when_first_stream_chunk_fails() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/stream-first-error".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("upstream stream error before first chunk")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_propagates_stream_error_after_first_chunk() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/stream-mid-error".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
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

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_preserves_redirect_without_following() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/redirect".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers().get(http_header::LOCATION),
        Some(&HeaderValue::from_static("/v1/echo?from=redirect"))
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_blocks_cross_origin_redirect() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/redirect-external".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("cross-origin redirect is not allowed")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_persists_record_on_redirect_rewrite_error() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        source: String,
        status: Option<String>,
        error_message: Option<String>,
        t_total_ms: Option<f64>,
        t_req_read_ms: Option<f64>,
        t_req_parse_ms: Option<f64>,
        t_upstream_connect_ms: Option<f64>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(
            r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"hi"}]}"#,
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("cross-origin redirect is not allowed")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT source, status, error_message, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.source, SOURCE_PROXY);
    assert_eq!(row.status.as_deref(), Some("http_502"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("cross-origin redirect is not allowed"))
    );
    assert!(row.t_total_ms.is_some_and(|v| v > 0.0));
    assert!(row.t_req_read_ms.is_some_and(|v| v >= 0.0));
    assert!(row.t_req_parse_ms.is_some_and(|v| v >= 0.0));
    assert!(row.t_upstream_connect_ms.is_some_and(|v| v >= 0.0));

    upstream_handle.abort();
}
