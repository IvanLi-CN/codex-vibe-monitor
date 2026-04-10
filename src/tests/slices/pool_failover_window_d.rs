#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_rejects_oversized_request_body() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state = test_state_with_openai_base_and_body_limit(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        4,
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/echo".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from("hello"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("request body exceeds")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_rejects_dot_segment_path() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/%2e%2e/admin".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
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
            .contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED)
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_rejects_malformed_percent_encoded_path() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/%zz/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
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
            .contains(PROXY_INVALID_REQUEST_TARGET)
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_returns_bad_gateway_on_upstream_handshake_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
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
        proxy_request_concurrency_semaphore: Arc::new(Semaphore::new(config.proxy_request_concurrency_limit)),
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

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/hang".parse().expect("valid uri")),
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
            .contains(PROXY_UPSTREAM_HANDSHAKE_TIMEOUT)
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_returns_bad_gateway_on_upstream_handshake_timeout_with_body() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
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
        proxy_request_concurrency_semaphore: Arc::new(Semaphore::new(config.proxy_request_concurrency_limit)),
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

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/hang".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from("hello"),
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
            .contains(PROXY_UPSTREAM_HANDSHAKE_TIMEOUT)
    );

    upstream_handle.abort();
}

#[test]
fn prepare_target_request_body_injects_include_usage_for_chat_stream() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-4o-mini",
        "stream": true,
        "messages": [{"role":"user","content":"hi"}]
    }))
    .expect("serialize request body");
    let (rewritten, info, did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::ChatCompletions, body, true);
    assert!(did_rewrite);
    assert!(info.is_stream);
    assert_eq!(info.model.as_deref(), Some("gpt-4o-mini"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
    assert_eq!(
        payload
            .pointer("/stream_options/include_usage")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn prepare_target_request_body_extracts_prompt_cache_key_from_metadata() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "metadata": {
            "prompt_cache_key": "pck-from-body"
        }
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert_eq!(info.prompt_cache_key.as_deref(), Some("pck-from-body"));
}

#[test]
fn prepare_target_request_body_extracts_sticky_key_aliases_from_metadata() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "metadata": {
            "sticky_key": "sticky-from-body"
        }
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert_eq!(info.sticky_key.as_deref(), Some("sticky-from-body"));
    assert_eq!(info.prompt_cache_key, None);
}

#[test]
fn prepare_target_request_body_extracts_requested_service_tier_without_rewriting_when_disabled() {
    let expected = json!({
        "model": "gpt-5.3-codex",
        "serviceTier": " Priority ",
        "stream": false
    });
    let body = serde_json::to_vec(&expected).expect("serialize request body");

    let (rewritten, info, did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert!(!did_rewrite);
    assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode body");
    assert_eq!(payload, expected);
}

#[test]
fn rewrite_request_service_tier_for_fast_mode_fill_missing_injects_priority_for_responses() {
    let mut payload = json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "input": "hi"
    });

    let did_rewrite = rewrite_request_service_tier_for_fast_mode(
        &mut payload,
        TagFastModeRewriteMode::FillMissing,
    );

    assert!(did_rewrite);
    assert_eq!(payload["service_tier"].as_str(), Some("priority"));
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn rewrite_request_service_tier_for_fast_mode_fill_missing_preserves_existing_alias() {
    let mut payload = json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "serviceTier": "flex"
    });

    let did_rewrite = rewrite_request_service_tier_for_fast_mode(
        &mut payload,
        TagFastModeRewriteMode::FillMissing,
    );

    assert!(!did_rewrite);
    assert_eq!(payload["serviceTier"].as_str(), Some("flex"));
    assert!(payload.get("service_tier").is_none());
}

#[test]
fn rewrite_request_service_tier_for_fast_mode_force_add_overrides_existing_tier() {
    let mut payload = json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "serviceTier": "flex",
        "messages": [{"role": "user", "content": "hi"}]
    });

    let did_rewrite =
        rewrite_request_service_tier_for_fast_mode(&mut payload, TagFastModeRewriteMode::ForceAdd);

    assert!(did_rewrite);
    assert_eq!(payload["service_tier"], "priority");
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn rewrite_request_service_tier_for_fast_mode_force_remove_deletes_both_aliases() {
    let mut payload = json!({
        "model": "gpt-5.3-codex",
        "service_tier": "priority",
        "serviceTier": "flex"
    });

    let did_rewrite = rewrite_request_service_tier_for_fast_mode(
        &mut payload,
        TagFastModeRewriteMode::ForceRemove,
    );

    assert!(did_rewrite);
    assert!(payload.get("service_tier").is_none());
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn pool_request_snapshot_preserves_content_length_only_for_file_backed_replays() {
    assert!(!pool_request_snapshot_preserves_content_length(
        &PoolReplayBodySnapshot::Empty
    ));
    assert!(!pool_request_snapshot_preserves_content_length(
        &PoolReplayBodySnapshot::Memory(Bytes::from_static(b"{}"))
    ));
    assert!(pool_request_snapshot_preserves_content_length(
        &PoolReplayBodySnapshot::File {
            temp_file: Arc::new(PoolReplayTempFile {
                path: PathBuf::from("/tmp/cvm-pool-replay-test.bin"),
            }),
            size: 2,
        }
    ));
}

#[tokio::test]
async fn prepare_pool_request_body_for_account_skips_fast_mode_rewrite_for_compact() {
    let expected = json!({
        "model": "gpt-5.1-codex-max",
        "serviceTier": "flex",
        "previous_response_id": "resp_prev_001",
        "input": [{
            "role": "user",
            "content": "compact this thread"
        }]
    });
    let body = Bytes::from(serde_json::to_vec(&expected).expect("serialize compact request body"));

    let prepared = prepare_pool_request_body_for_account(
        Some(&PoolReplayBodySnapshot::Memory(body)),
        &"/v1/responses/compact".parse().expect("valid compact uri"),
        &Method::POST,
        TagFastModeRewriteMode::ForceAdd,
    )
    .await
    .expect("prepare compact pool request body");

    assert_eq!(prepared.requested_service_tier.as_deref(), Some("flex"));
    let request_body = prepared
        .request_body_for_capture
        .expect("capture request body should be materialized");
    let payload: Value = serde_json::from_slice(&request_body).expect("decode body");
    assert_eq!(payload, expected);
    assert!(payload.get("service_tier").is_none());
}

#[test]
fn prepare_target_request_body_extracts_reasoning_effort_for_responses() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "reasoning": {
            "effort": "high"
        }
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert_eq!(info.reasoning_effort.as_deref(), Some("high"));
}

#[test]
fn prepare_target_request_body_extracts_reasoning_effort_for_chat_completions() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "messages": [{"role": "user", "content": "hi"}],
        "reasoning_effort": "medium"
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::ChatCompletions, body, true);

    assert_eq!(info.reasoning_effort.as_deref(), Some("medium"));
}

#[test]
fn extract_requested_service_tier_from_request_body_reads_top_level_aliases() {
    let snake_case = json!({ "service_tier": " Priority " });
    let camel_case = json!({ "serviceTier": "PRIORITY" });

    assert_eq!(
        extract_requested_service_tier_from_request_body(&snake_case).as_deref(),
        Some("priority")
    );
    assert_eq!(
        extract_requested_service_tier_from_request_body(&camel_case).as_deref(),
        Some("priority")
    );
}

#[test]
fn extract_requested_service_tier_from_request_body_ignores_nested_or_non_string_values() {
    let nested = json!({
        "response": { "service_tier": "priority" },
        "metadata": { "serviceTier": "priority" }
    });
    let non_string = json!({ "service_tier": true });
    let blank = json!({ "serviceTier": "   " });

    assert_eq!(
        extract_requested_service_tier_from_request_body(&nested),
        None
    );
    assert_eq!(
        extract_requested_service_tier_from_request_body(&non_string),
        None
    );
    assert_eq!(
        extract_requested_service_tier_from_request_body(&blank),
        None
    );
}

#[test]
fn extract_requester_ip_uses_expected_header_priority() {
    let mut preferred = HeaderMap::new();
    preferred.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static("198.51.100.10, 203.0.113.9"),
    );
    preferred.insert(
        HeaderName::from_static("x-real-ip"),
        HeaderValue::from_static("203.0.113.5"),
    );
    preferred.insert(
        HeaderName::from_static("forwarded"),
        HeaderValue::from_static("for=192.0.2.60;proto=https"),
    );
    assert_eq!(
        extract_requester_ip(&preferred, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
        Some("198.51.100.10")
    );

    let mut fallback_forwarded = HeaderMap::new();
    fallback_forwarded.insert(
        HeaderName::from_static("forwarded"),
        HeaderValue::from_static("for=\"[2001:db8::1]:443\";proto=https"),
    );
    assert_eq!(
        extract_requester_ip(&fallback_forwarded, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
        Some("2001:db8::1")
    );

    let no_headers = HeaderMap::new();
    assert_eq!(
        extract_requester_ip(&no_headers, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
        Some("127.0.0.1")
    );
}

#[test]
fn extract_prompt_cache_key_from_headers_reads_whitelist_keys() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-prompt-cache-key"),
        HeaderValue::from_static("pck-from-header"),
    );
    assert_eq!(
        extract_prompt_cache_key_from_headers(&headers).as_deref(),
        Some("pck-from-header")
    );
}

#[test]
fn extract_sticky_key_from_headers_accepts_sticky_and_prompt_cache_aliases() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-prompt-cache-key"),
        HeaderValue::from_static("pck-from-header"),
    );
    headers.insert(
        HeaderName::from_static("x-sticky-key"),
        HeaderValue::from_static("sticky-from-header"),
    );

    assert_eq!(
        extract_sticky_key_from_headers(&headers).as_deref(),
        Some("sticky-from-header")
    );
    assert_eq!(
        extract_prompt_cache_key_from_headers(&headers).as_deref(),
        Some("pck-from-header")
    );
}

#[test]
fn parse_stream_response_payload_extracts_usage_and_model() {
    let raw = [
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}],\"usage\":null}",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[],\"service_tier\":\"priority\",\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":18}}",
        "data: [DONE]",
    ]
    .join("\n");
    let parsed = parse_stream_response_payload(raw.as_bytes());
    assert_eq!(parsed.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(parsed.usage.input_tokens, Some(11));
    assert_eq!(parsed.usage.output_tokens, Some(7));
    assert_eq!(parsed.usage.total_tokens, Some(18));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_stream_response_payload_extracts_terminal_failure_details() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "event: response.failed",
        r#"data: {"type":"response.failed","response":{"id":"resp_test","model":"gpt-5.4","status":"failed","error":{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}}"#,
    ]
    .join("\n");

    let parsed = parse_stream_response_payload(raw.as_bytes());
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(
        parsed.stream_terminal_event.as_deref(),
        Some("response.failed")
    );
    assert_eq!(parsed.upstream_error_code.as_deref(), Some("server_error"));
    assert!(
        parsed.upstream_error_message.as_deref().is_some_and(
            |message| message.contains("request ID 060a328d-5cb6-433c-9025-1da2d9c632f1")
        )
    );
    assert_eq!(
        parsed.upstream_request_id.as_deref(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );
    assert_eq!(
        parsed.usage_missing_reason.as_deref(),
        Some("upstream_response_failed")
    );
}

#[test]
fn estimate_proxy_cost_subtracts_cached_tokens_from_base_input_rate() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-test".to_string(),
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: Some(0.5),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, price_version) = estimate_proxy_cost(
        &catalog,
        Some("gpt-test"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((600.0 * 1.0) + (200.0 * 2.0) + (400.0 * 0.5)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
    assert_eq!(price_version.as_deref(), Some("unit-test@response-tier"));
}

#[test]
fn estimate_proxy_cost_keeps_full_input_when_cache_price_missing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-test".to_string(),
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-test"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((1_000.0 * 1.0) + (200.0 * 2.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_falls_back_to_dated_model_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1000),
        output_tokens: Some(500),
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1500),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.2-2025-12-11"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((1000.0 * 2.0) + (500.0 * 3.0)) / 1_000_000.0;
    assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_prefers_exact_model_over_dated_model_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([
            (
                "gpt-5.2".to_string(),
                ModelPricing {
                    input_per_1m: 1.0,
                    output_per_1m: 1.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
            (
                "gpt-5.2-2025-12-11".to_string(),
                ModelPricing {
                    input_per_1m: 4.0,
                    output_per_1m: 5.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
        ]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1000),
        output_tokens: Some(1000),
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(2000),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.2-2025-12-11"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((1000.0 * 4.0) + (1000.0 * 5.0)) / 1_000_000.0;
    assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_at_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((271_000.0 * 2.5) + (1_000.0 * 0.25) + (1_000.0 * 15.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_above_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_to_reasoning_cost() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: Some(20.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: Some(2_000),
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let reasoning_part = (2_000.0 * 20.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_above_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4-pro".to_string(),
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(999_999),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4-pro"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = (272_001.0 * 30.0) / 1_000_000.0;
    let output_part = (1_000.0 * 180.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_for_dated_model_suffix() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4-pro".to_string(),
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                reasoning_per_1m: Some(90.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(999_999),
        reasoning_tokens: Some(2_000),
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4-pro-2026-03-01"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = (272_001.0 * 30.0) / 1_000_000.0;
    let output_part = (1_000.0 * 180.0) / 1_000_000.0;
    let reasoning_part = (2_000.0 * 90.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_for_dated_model_suffix() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4-2026-03-01"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_for_other_models() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4o".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4o"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((272_001.0 * 2.5) + (1_000.0 * 15.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_requested_tier_priority_multiplier_and_price_version_suffix() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: Some(20.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: Some(50),
        total_tokens: Some(1_200),
    };

    let (cost, estimated, price_version) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("priority"),
        ProxyPricingMode::RequestedTier,
    );

    let base_expected =
        ((600.0 * 2.5) + (400.0 * 0.25) + (200.0 * 15.0) + (50.0 * 20.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - (base_expected * 2.0)).abs() < 1e-12);
    assert!(estimated);
    assert_eq!(price_version.as_deref(), Some("unit-test@requested-tier"));
}

#[test]
fn resolve_proxy_billing_service_tier_and_pricing_mode_prefers_requested_tier_for_api_keys() {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        Some("priority"),
        Some("default"),
        Some("api_key_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("priority"));
    assert_eq!(pricing_mode, ProxyPricingMode::RequestedTier);
}

#[test]
fn resolve_proxy_billing_service_tier_and_pricing_mode_prefers_explicit_billing_metadata() {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        Some("default"),
        Some("priority"),
        Some("priority"),
        Some("api_key_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("default"));
    assert_eq!(pricing_mode, ProxyPricingMode::ExplicitBilling);
}

#[test]
fn resolve_proxy_billing_service_tier_and_pricing_mode_keeps_response_tier_for_non_api_keys() {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        Some("priority"),
        Some("default"),
        Some("oauth_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("default"));
    assert_eq!(pricing_mode, ProxyPricingMode::ResponseTier);
}

#[test]
fn resolve_proxy_billing_service_tier_and_pricing_mode_falls_back_to_response_tier_when_api_keys_request_is_missing()
 {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        None,
        Some("default"),
        Some("api_key_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("default"));
    assert_eq!(pricing_mode, ProxyPricingMode::ResponseTier);
}

#[test]
fn parse_target_response_payload_decodes_gzip_stream_usage() {
    let raw = [
        "event: response.created",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
        "",
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        "",
    ]
    .join("\n");

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        &compressed,
        true,
        Some("gzip"),
    );
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.input_tokens, Some(12));
    assert_eq!(parsed.usage.output_tokens, Some(3));
    assert_eq!(parsed.usage.cache_input_tokens, Some(2));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_target_response_payload_decodes_multi_value_content_encoding() {
    let raw = [
        "event: response.created",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
        "",
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"flex\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        "",
    ]
    .join("\n");

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        &compressed,
        true,
        Some("identity, gzip"),
    );
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.input_tokens, Some(12));
    assert_eq!(parsed.usage.output_tokens, Some(3));
    assert_eq!(parsed.usage.cache_input_tokens, Some(2));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert_eq!(parsed.service_tier.as_deref(), Some("flex"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_target_response_payload_detects_sse_without_request_stream_hint() {
    let raw = [
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","service_tier":"priority","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), false, None);

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_target_response_payload_prefers_terminal_stream_service_tier_over_initial_auto() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"auto"}}"#,
        "",
        "event: response.in_progress",
        r#"data: {"type":"response.in_progress","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"auto"}}"#,
        "",
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.4","status":"completed","service_tier":"default","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), true, None);

    assert_eq!(parsed.service_tier.as_deref(), Some("default"));
    assert_eq!(parsed.usage.total_tokens, Some(15));
}

#[test]
fn parse_target_response_payload_does_not_downgrade_same_rank_stream_tier_to_auto() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"default"}}"#,
        "",
        "event: response.in_progress",
        r#"data: {"type":"response.in_progress","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"auto"}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), true, None);

    assert_eq!(parsed.service_tier.as_deref(), Some("default"));
}

#[test]
fn parse_target_response_payload_from_raw_file_falls_back_to_raw_deflate_streams() {
    let raw = [
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","usage":{"input_tokens":17,"output_tokens":4,"total_tokens":21}}}"#,
        "",
    ]
    .join("\n");

    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write raw deflate stream");
    let compressed = encoder.finish().expect("finish raw deflate stream");

    let temp_dir = make_temp_test_dir("raw-deflate-response");
    let raw_path = temp_dir.join("response.bin");
    fs::write(&raw_path, compressed).expect("write raw deflate response payload");

    let parsed = parse_target_response_payload_from_raw_file(
        ProxyCaptureTarget::Responses,
        &raw_path,
        true,
        Some("deflate"),
    )
    .expect("parse raw deflate response payload");

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.total_tokens, Some(21));
    assert!(parsed.usage_missing_reason.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn parse_target_response_payload_reads_service_tier_from_response_object() {
    let raw = json!({
        "id": "resp_json_1",
        "response": {
            "model": "gpt-5.3-codex",
            "service_tier": "priority",
            "usage": {
                "input_tokens": 21,
                "output_tokens": 5,
                "total_tokens": 26
            }
        }
    });

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        serde_json::to_string(&raw)
            .expect("serialize raw payload")
            .as_bytes(),
        false,
        None,
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert_eq!(parsed.usage.total_tokens, Some(26));
}

#[test]
fn parse_target_response_payload_records_decode_failure_reason() {
    let raw = [
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2,\"total_tokens\":12}}}",
        "data: [DONE]",
    ]
    .join("\n");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        raw.as_bytes(),
        true,
        Some("gzip"),
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.total_tokens, Some(12));
    assert!(
        parsed
            .usage_missing_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("response_decode_failed:gzip:"))
    );
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_extracts_usage_from_gzip_response_stream() {
    #[derive(sqlx::FromRow)]
    struct PersistedUsageRow {
        source: String,
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        cache_input_tokens: Option<i64>,
        total_tokens: Option<i64>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static("198.51.100.42, 203.0.113.10"),
    );
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses?mode=gzip".parse().expect("valid uri")),
        Method::POST,
        headers,
        Body::from(
            r#"{"model":"gpt-5.3-codex","stream":true,"metadata":{"prompt_cache_key":"pck-gzip-1"},"input":"hello"}"#,
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");

    let mut row: Option<PersistedUsageRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedUsageRow>(
            r#"
            SELECT source, status, input_tokens, output_tokens, cache_input_tokens, total_tokens, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");

        if row
            .as_ref()
            .is_some_and(|record| record.input_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("capture record should exist");
    assert_eq!(row.source, SOURCE_PROXY);
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(12));
    assert_eq!(row.output_tokens, Some(3));
    assert_eq!(row.cache_input_tokens, Some(2));
    assert_eq!(row.total_tokens, Some(15));
    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode payload summary");
    assert_eq!(payload["endpoint"], "/v1/responses");
    assert!(payload["usageMissingReason"].is_null());
    assert_eq!(payload["requesterIp"], "198.51.100.42");
    assert_eq!(payload["promptCacheKey"], "pck-gzip-1");
    assert!(
        payload["proxyWeightDelta"].is_number(),
        "proxy weight delta should be recorded for fresh proxy attempts"
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_gzip_stream_without_event_stream_header_still_extracts_usage() {
    #[derive(sqlx::FromRow)]
    struct PersistedUsageRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=gzip-no-content-type"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.3-codex","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");

    let mut row: Option<PersistedUsageRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedUsageRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query gzip stream usage row without event-stream header");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("gzip stream usage row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(19));
    assert_eq!(row.output_tokens, Some(6));
    assert_eq!(row.total_tokens, Some(25));
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
}

fn reset_proxy_capture_hot_path_raw_fallbacks() {
    reset_response_capture_raw_fallback_counters();
}

fn assert_proxy_capture_hot_path_skips_raw_fallbacks() {
    let (sse_hint_fallbacks, parse_fallbacks) = response_capture_raw_fallback_counts();
    assert_eq!(
        sse_hint_fallbacks, 0,
        "proxy capture hot path should not reread raw files for SSE hinting"
    );
    assert_eq!(
        parse_fallbacks, 0,
        "proxy capture hot path should not reread raw files for response parsing"
    );
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_large_gzip_stream_without_event_stream_header_keeps_raw_capture_without_raw_reread()
 {
    #[derive(sqlx::FromRow)]
    struct PersistedUsageRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
        response_raw_path: Option<String>,
        response_raw_size: Option<i64>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-gzip-stream-no-content-type");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=gzip-large-no-content-type"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.3-codex","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");

    let mut row: Option<PersistedUsageRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedUsageRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                response_raw_path,
                response_raw_size,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query large gzip stream usage row without event-stream header");
        if row
            .as_ref()
            .is_some_and(|record| record.response_raw_path.is_some() && record.payload.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("large gzip stream usage row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert!(
        row.response_raw_size
            .is_some_and(|size| size > RAW_RESPONSE_PREVIEW_LIMIT as i64)
    );
    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes = read_proxy_raw_bytes(response_raw_path, None)
        .expect("read persisted large gzip response raw");
    let (decoded_raw_bytes, decode_failure_reason) =
        decode_response_payload_for_usage(&raw_bytes, Some("gzip"));
    assert!(
        decode_failure_reason.is_none(),
        "persisted raw gzip response should remain decodable"
    );
    let raw_text = String::from_utf8(decoded_raw_bytes.into_owned())
        .expect("raw gzip response should decode to utf8");
    assert!(raw_text.contains("response.completed"));
    assert!(raw_text.contains("\"total_tokens\":30"));
    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode large gzip payload summary");
    match (row.input_tokens, row.output_tokens, row.total_tokens) {
        (Some(input), Some(output), Some(total)) => {
            assert_eq!(input, 23);
            assert_eq!(output, 7);
            assert_eq!(total, 30);
            assert!(payload["usageMissingReason"].is_null());
        }
        (None, None, None) => {
            assert!(
                payload["usageMissingReason"].as_str().is_some(),
                "bounded preview parsing may degrade metadata, but it should still record a reason"
            );
        }
        other => panic!("unexpected partial usage extraction state: {other:?}"),
    }
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_large_stream_keeps_preview_bounded_without_raw_reread() {
    #[derive(sqlx::FromRow)]
    struct PersistedLargeRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
        raw_response: String,
        response_raw_path: Option<String>,
        response_raw_size: Option<i64>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-stream-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=large-stream"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let body_text = String::from_utf8(body.to_vec()).expect("stream body should be utf8");
    assert!(body_text.contains("response.completed"));

    let mut row: Option<PersistedLargeRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedLargeRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                raw_response,
                response_raw_path,
                response_raw_size,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query large stream capture row");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("large stream capture row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(42));
    assert_eq!(row.output_tokens, Some(13));
    assert_eq!(row.total_tokens, Some(55));
    assert_eq!(row.raw_response.len(), RAW_RESPONSE_PREVIEW_LIMIT);
    assert!(
        !row.raw_response.contains("response.completed"),
        "preview should not contain the terminal event once the delta exceeds the preview budget"
    );
    assert!(
        row.response_raw_size
            .is_some_and(|size| size > RAW_RESPONSE_PREVIEW_LIMIT as i64)
    );

    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read persisted large response raw");
    let raw_text = String::from_utf8(raw_bytes).expect("raw response should remain utf8");
    assert!(raw_text.contains("response.completed"));
    assert!(raw_text.len() > row.raw_response.len());

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode persisted payload summary");
    assert!(payload["usageMissingReason"].is_null());
    assert_eq!(payload["serviceTier"].as_str(), Some("priority"));
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_large_stream_terminal_event_keeps_live_metadata_without_raw_reread() {
    #[derive(sqlx::FromRow)]
    struct PersistedLargeTerminalRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
        response_raw_path: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-terminal-stream-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=large-terminal-stream"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let body_text = String::from_utf8(body.to_vec()).expect("stream body should be utf8");
    assert!(body_text.contains("response.completed"));

    let mut row: Option<PersistedLargeTerminalRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedLargeTerminalRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                response_raw_path,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query large terminal stream capture row");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("large terminal stream capture row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert!(
        row.response_raw_path.is_some(),
        "full raw response should be stored"
    );
    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read persisted large terminal raw");
    let raw_text =
        String::from_utf8(raw_bytes).expect("large terminal raw response should be utf8");
    assert!(raw_text.contains("response.completed"));
    assert!(raw_text.contains("\"total_tokens\":96"));

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode large terminal payload summary");
    match (row.input_tokens, row.output_tokens, row.total_tokens) {
        (Some(input), Some(output), Some(total)) => {
            assert_eq!(input, 77);
            assert_eq!(output, 19);
            assert_eq!(total, 96);
            assert_eq!(payload["serviceTier"], "priority");
            assert!(payload["usageMissingReason"].is_null());
        }
        (None, None, None) => {
            assert!(
                payload["usageMissingReason"].as_str().is_some(),
                "oversized terminal events may no longer backfill metadata from raw rereads"
            );
        }
        other => panic!("unexpected partial usage extraction state: {other:?}"),
    }
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_oversized_stream_keeps_live_metadata_when_raw_file_is_truncated() {
    #[derive(sqlx::FromRow)]
    struct PersistedOversizedRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
        response_raw_truncated: i64,
        response_raw_truncated_reason: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-oversized-stream-truncated-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    config.proxy_raw_max_bytes = Some(24 * 1024);
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=oversized-delta-stream"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let body_text = String::from_utf8(body.to_vec()).expect("stream body should be utf8");
    assert!(body_text.contains("response.completed"));

    let mut row: Option<PersistedOversizedRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedOversizedRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                response_raw_truncated,
                response_raw_truncated_reason,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query oversized stream capture row");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("oversized stream capture row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(61));
    assert_eq!(row.output_tokens, Some(17));
    assert_eq!(row.total_tokens, Some(78));
    assert_eq!(row.response_raw_truncated, 1);
    assert_eq!(
        row.response_raw_truncated_reason.as_deref(),
        Some("max_bytes_exceeded")
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode oversized stream payload summary");
    assert_eq!(payload["serviceTier"], "priority");
    assert!(payload["usageMissingReason"].is_null());
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_large_stream_keeps_usage_when_response_raw_is_truncated() {
    #[derive(sqlx::FromRow)]
    struct PersistedLargeRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
        raw_response: String,
        response_raw_path: Option<String>,
        response_raw_size: Option<i64>,
        response_raw_truncated: i64,
        response_raw_truncated_reason: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-stream-raw-truncated");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    config.proxy_raw_max_bytes = Some(8 * 1024);
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=large-stream"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let body_text = String::from_utf8(body.to_vec()).expect("stream body should be utf8");
    assert!(body_text.contains("response.completed"));

    let mut row: Option<PersistedLargeRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedLargeRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                raw_response,
                response_raw_path,
                response_raw_size,
                response_raw_truncated,
                response_raw_truncated_reason,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query truncated large stream capture row");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("truncated large stream capture row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(42));
    assert_eq!(row.output_tokens, Some(13));
    assert_eq!(row.total_tokens, Some(55));
    assert_eq!(row.raw_response.len(), RAW_RESPONSE_PREVIEW_LIMIT);
    assert_eq!(row.response_raw_truncated, 1);
    assert_eq!(
        row.response_raw_truncated_reason.as_deref(),
        Some("max_bytes_exceeded")
    );
    assert!(
        row.response_raw_size.is_some_and(|size| size > 8 * 1024),
        "response raw size should still reflect the full upstream stream"
    );

    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read truncated response raw");
    assert!(
        raw_bytes.len() <= 8 * 1024,
        "persisted raw bytes should respect the configured storage cap"
    );
    let raw_text = String::from_utf8(raw_bytes).expect("truncated raw response should remain utf8");
    assert!(
        !raw_text.contains("response.completed"),
        "persisted raw bytes should be truncated before the terminal event"
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode persisted payload summary");
    assert!(payload["usageMissingReason"].is_null());
    assert_eq!(payload["serviceTier"].as_str(), Some("priority"));
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_stream_request_json_error_uses_nonstream_parse_fallback() {
    #[derive(sqlx::FromRow)]
    struct PersistedErrorRow {
        status: Option<String>,
        error_message: Option<String>,
        response_raw_truncated: i64,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-stream-json-error");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    config.proxy_raw_max_bytes = Some(128);
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses?mode=json-error".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read json error response body");

    let mut row: Option<PersistedErrorRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedErrorRow>(
            r#"
            SELECT
                status,
                error_message,
                response_raw_truncated,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query json error capture row");
        if row
            .as_ref()
            .and_then(|record| record.error_message.as_deref())
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("json error capture row should exist");
    assert_eq!(row.status.as_deref(), Some("http_400"));
    assert_eq!(row.response_raw_truncated, 1);
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|message| message.contains("tail-marker")),
        "stream request should keep the full JSON error message even when raw storage truncates"
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode json error payload");
    assert_eq!(
        payload["upstreamErrorCode"].as_str(),
        Some("invalid_request_error")
    );
    assert!(
        payload["upstreamErrorMessage"]
            .as_str()
            .is_some_and(|message| message.contains("tail-marker")),
        "payload should preserve the full upstream error message"
    );
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "manual RSS soak harness for large proxy response capture"]
async fn proxy_capture_target_large_stream_soak_keeps_rss_within_stable_window() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-stream-soak");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let mut rss_samples = Vec::new();
    for iteration in 0..8_i64 {
        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri(
                "/v1/responses?mode=large-stream"
                    .parse()
                    .expect("valid uri"),
            ),
            Method::POST,
            HeaderMap::new(),
            Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response body");
        wait_for_codex_invocations(&state.pool, iteration + 1).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Some(rss_kib) = current_process_rss_kib() {
            rss_samples.push(rss_kib);
        }
    }

    eprintln!("large-stream-soak rss_kib={rss_samples:?}");
    assert!(
        rss_samples.len() >= 4,
        "RSS harness should collect multiple samples"
    );
    let steady_state = &rss_samples[3..];
    let min_rss = *steady_state
        .iter()
        .min()
        .expect("steady-state RSS should have a minimum");
    let max_rss = *steady_state
        .iter()
        .max()
        .expect("steady-state RSS should have a maximum");
    assert!(
        max_rss.saturating_sub(min_rss) < 128 * 1024,
        "steady-state RSS window too wide: min={min_rss}KiB max={max_rss}KiB"
    );
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_large_nonstream_json_skips_bounded_parse_and_keeps_full_raw_file() {
    #[derive(sqlx::FromRow)]
    struct PersistedCompactRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
        raw_response: String,
        response_raw_path: Option<String>,
        response_raw_size: Option<i64>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-json-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "previous_response_id": "resp_prev_large",
        "input": [{ "role": "user", "content": "compact this thread" }]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses/compact?mode=large-json"
                .parse()
                .expect("valid compact uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact proxy response body");

    let mut row: Option<PersistedCompactRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedCompactRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                raw_response,
                response_raw_path,
                response_raw_size,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query large compact capture row");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("large compact capture row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert!(row.input_tokens.is_none());
    assert!(row.output_tokens.is_none());
    assert!(row.total_tokens.is_none());
    assert_eq!(row.raw_response.len(), RAW_RESPONSE_PREVIEW_LIMIT);
    assert!(
        row.response_raw_size
            .is_some_and(|size| size > BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES as i64)
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode payload summary");
    assert!(
        payload["usageMissingReason"]
            .as_str()
            .is_some_and(|reason| reason.contains(PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED))
    );

    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read large compact raw response");
    let raw_text = String::from_utf8(raw_bytes).expect("raw compact response should be utf8");
    assert!(raw_text.contains("\"total_tokens\":300"));
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}
