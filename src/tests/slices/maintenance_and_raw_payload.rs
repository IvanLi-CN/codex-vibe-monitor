fn write_backfill_response_payload_with_service_tier(path: &Path, service_tier: Option<&str>) {
    let mut response = json!({
        "type": "response.completed",
        "response": {
            "usage": {
                "input_tokens": 88,
                "output_tokens": 22,
                "total_tokens": 110,
                "input_tokens_details": { "cached_tokens": 9 },
                "output_tokens_details": { "reasoning_tokens": 3 }
            }
        }
    });
    if let Some(service_tier) = service_tier {
        response["response"]["service_tier"] = Value::String(service_tier.to_string());
    }
    let raw = [
        "event: response.completed".to_string(),
        format!("data: {response}"),
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");
    fs::write(path, compressed).expect("write response payload");
}

fn write_backfill_response_payload_with_terminal_service_tier(
    path: &Path,
    initial_service_tier: Option<&str>,
    terminal_service_tier: Option<&str>,
) {
    let mut created_response = json!({
        "type": "response.created",
        "response": {
            "id": "resp_backfill",
            "status": "in_progress"
        }
    });
    if let Some(service_tier) = initial_service_tier {
        created_response["response"]["service_tier"] = Value::String(service_tier.to_string());
    }

    let mut completed_response = json!({
        "type": "response.completed",
        "response": {
            "id": "resp_backfill",
            "status": "completed",
            "usage": {
                "input_tokens": 88,
                "output_tokens": 22,
                "total_tokens": 110,
                "input_tokens_details": { "cached_tokens": 9 },
                "output_tokens_details": { "reasoning_tokens": 3 }
            }
        }
    });
    if let Some(service_tier) = terminal_service_tier {
        completed_response["response"]["service_tier"] = Value::String(service_tier.to_string());
    }

    let raw = [
        "event: response.created".to_string(),
        format!("data: {created_response}"),
        "".to_string(),
        "event: response.completed".to_string(),
        format!("data: {completed_response}"),
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");
    fs::write(path, compressed).expect("write response payload");
}

fn write_backfill_request_payload(path: &Path, prompt_cache_key: Option<&str>) {
    write_backfill_request_payload_with_fields(
        path,
        prompt_cache_key,
        None,
        None,
        ProxyCaptureTarget::Responses,
    );
}

fn write_backfill_request_payload_with_requested_service_tier(
    path: &Path,
    requested_service_tier: Option<&str>,
    target: ProxyCaptureTarget,
) {
    write_backfill_request_payload_with_fields(path, None, None, requested_service_tier, target);
}

fn write_backfill_request_payload_with_reasoning(
    path: &Path,
    prompt_cache_key: Option<&str>,
    reasoning_effort: Option<&str>,
    target: ProxyCaptureTarget,
) {
    write_backfill_request_payload_with_fields(
        path,
        prompt_cache_key,
        reasoning_effort,
        None,
        target,
    );
}

fn write_backfill_request_payload_with_fields(
    path: &Path,
    prompt_cache_key: Option<&str>,
    reasoning_effort: Option<&str>,
    requested_service_tier: Option<&str>,
    target: ProxyCaptureTarget,
) {
    let payload = match target {
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact => {
            let mut payload = json!({
                "model": "gpt-5.3-codex",
                "stream": true,
                "metadata": {},
            });
            if let Some(key) = prompt_cache_key {
                payload["metadata"]["prompt_cache_key"] = Value::String(key.to_string());
            }
            if let Some(effort) = reasoning_effort {
                payload["reasoning"] = json!({ "effort": effort });
            }
            if let Some(service_tier) = requested_service_tier {
                payload["service_tier"] = Value::String(service_tier.to_string());
            }
            payload
        }
        ProxyCaptureTarget::ChatCompletions => {
            let mut payload = json!({
                "model": "gpt-5.3-codex",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}],
            });
            if let Some(key) = prompt_cache_key {
                payload["metadata"] = json!({ "prompt_cache_key": key });
            }
            if let Some(effort) = reasoning_effort {
                payload["reasoning_effort"] = Value::String(effort.to_string());
            }
            if let Some(service_tier) = requested_service_tier {
                payload["serviceTier"] = Value::String(service_tier.to_string());
            }
            payload
        }
    };
    let encoded = serde_json::to_vec(&payload).expect("serialize request payload");
    fs::write(path, encoded).expect("write request payload");
}

async fn insert_proxy_backfill_row(pool: &SqlitePool, invoke_id: &str, response_path: &Path) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(
        "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":true,\"requestModel\":null,\"responseModel\":null,\"usageMissingReason\":null,\"requestParseError\":null}",
    )
    .bind("{}")
    .bind(response_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("insert proxy row");
}

async fn insert_proxy_cost_backfill_row(
    pool: &SqlitePool,
    invoke_id: &str,
    model: Option<&str>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, model, input_tokens, output_tokens, total_tokens, cost, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)
        "#,
    )
    .bind(invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(model)
    .bind(input_tokens)
    .bind(output_tokens)
    .bind(match (input_tokens, output_tokens) {
        (Some(input), Some(output)) => Some(input + output),
        _ => None,
    })
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert proxy cost row");
}

async fn insert_proxy_prompt_cache_backfill_row(
    pool: &SqlitePool,
    invoke_id: &str,
    request_path: &Path,
    payload: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(payload)
    .bind("{}")
    .bind(request_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("insert proxy prompt cache key row");
}

async fn test_state_with_openai_base(openai_base: Url) -> Arc<AppState> {
    test_state_with_openai_base_and_body_limit(
        openai_base,
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
    )
    .await
}

async fn test_state_with_openai_base_and_body_limit(
    openai_base: Url,
    body_limit: usize,
) -> Arc<AppState> {
    test_state_with_openai_base_body_limit_and_read_timeout(
        openai_base,
        body_limit,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS),
    )
    .await
}

async fn test_state_with_openai_base_body_limit_and_read_timeout(
    openai_base: Url,
    body_limit: usize,
    request_read_timeout: Duration,
) -> Arc<AppState> {
    test_state_with_openai_base_and_proxy_timeouts(
        openai_base,
        body_limit,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS),
        Duration::from_secs(DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS),
        request_read_timeout,
    )
    .await
}

async fn test_state_with_openai_base_and_proxy_timeouts(
    openai_base: Url,
    body_limit: usize,
    handshake_timeout: Duration,
    compact_handshake_timeout: Duration,
    request_read_timeout: Duration,
) -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url = openai_base;
    config.openai_proxy_max_request_body_bytes = body_limit;
    config.openai_proxy_handshake_timeout = handshake_timeout;
    config.openai_proxy_compact_handshake_timeout = compact_handshake_timeout;
    config.openai_proxy_request_read_timeout = request_read_timeout;
    test_state_from_config(config, true).await
}

async fn test_state_with_openai_base_and_pool_no_available_wait(
    openai_base: Url,
    timeout: Duration,
    poll_interval: Duration,
) -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url = openai_base;
    test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout,
            poll_interval,
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await
}

async fn test_state_from_config(config: AppConfig, startup_ready: bool) -> Arc<AppState> {
    test_state_from_config_with_pool_no_available_wait(
        config,
        startup_ready,
        PoolNoAvailableWaitSettings::default(),
    )
    .await
}

async fn test_state_from_config_with_pool_no_available_wait(
    config: AppConfig,
    startup_ready: bool,
    pool_no_available_wait: PoolNoAvailableWaitSettings,
) -> Arc<AppState> {
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url = format!("sqlite:file:codex-vibe-monitor-test-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePool::connect(&db_url)
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let pricing_catalog = load_pricing_catalog(&pool)
        .await
        .expect("pricing catalog should initialize");

    Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        startup_ready: Arc::new(AtomicBool::new(startup_ready)),
        shutdown: CancellationToken::new(),
        semaphore,
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
        pricing_catalog: Arc::new(RwLock::new(pricing_catalog)),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        pool_no_available_wait,
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    })
}

fn clone_state_with_upstream_accounts(
    state: &Arc<AppState>,
    upstream_accounts: Arc<UpstreamAccountsRuntime>,
) -> Arc<AppState> {
    Arc::new(AppState {
        config: state.config.clone(),
        pool: state.pool.clone(),
        hourly_rollup_sync_lock: state.hourly_rollup_sync_lock.clone(),
        http_clients: state.http_clients.clone(),
        broadcaster: state.broadcaster.clone(),
        broadcast_state_cache: state.broadcast_state_cache.clone(),
        proxy_summary_quota_broadcast_seq: state.proxy_summary_quota_broadcast_seq.clone(),
        proxy_summary_quota_broadcast_running: state.proxy_summary_quota_broadcast_running.clone(),
        proxy_summary_quota_broadcast_handle: state.proxy_summary_quota_broadcast_handle.clone(),
        startup_ready: state.startup_ready.clone(),
        shutdown: state.shutdown.clone(),
        semaphore: state.semaphore.clone(),
        proxy_raw_async_semaphore: state.proxy_raw_async_semaphore.clone(),
        proxy_model_settings: state.proxy_model_settings.clone(),
        proxy_model_settings_update_lock: state.proxy_model_settings_update_lock.clone(),
        forward_proxy: state.forward_proxy.clone(),
        xray_supervisor: state.xray_supervisor.clone(),
        forward_proxy_settings_update_lock: state.forward_proxy_settings_update_lock.clone(),
        forward_proxy_subscription_refresh_lock: state
            .forward_proxy_subscription_refresh_lock
            .clone(),
        pricing_settings_update_lock: state.pricing_settings_update_lock.clone(),
        pricing_catalog: state.pricing_catalog.clone(),
        prompt_cache_conversation_cache: state.prompt_cache_conversation_cache.clone(),
        maintenance_stats_cache: state.maintenance_stats_cache.clone(),
        pool_routing_reservations: state.pool_routing_reservations.clone(),
        pool_live_attempt_ids: state.pool_live_attempt_ids.clone(),
        pool_group_429_retry_delay_override: state.pool_group_429_retry_delay_override,
        pool_no_available_wait: state.pool_no_available_wait,
        upstream_accounts,
    })
}

fn clone_state_with_pool_group_429_retry_delay_override(
    state: &Arc<AppState>,
    delay: Option<Duration>,
) -> Arc<AppState> {
    Arc::new(AppState {
        config: state.config.clone(),
        pool: state.pool.clone(),
        hourly_rollup_sync_lock: state.hourly_rollup_sync_lock.clone(),
        http_clients: state.http_clients.clone(),
        broadcaster: state.broadcaster.clone(),
        broadcast_state_cache: state.broadcast_state_cache.clone(),
        proxy_summary_quota_broadcast_seq: state.proxy_summary_quota_broadcast_seq.clone(),
        proxy_summary_quota_broadcast_running: state.proxy_summary_quota_broadcast_running.clone(),
        proxy_summary_quota_broadcast_handle: state.proxy_summary_quota_broadcast_handle.clone(),
        startup_ready: state.startup_ready.clone(),
        shutdown: state.shutdown.clone(),
        semaphore: state.semaphore.clone(),
        proxy_raw_async_semaphore: state.proxy_raw_async_semaphore.clone(),
        proxy_model_settings: state.proxy_model_settings.clone(),
        proxy_model_settings_update_lock: state.proxy_model_settings_update_lock.clone(),
        forward_proxy: state.forward_proxy.clone(),
        xray_supervisor: state.xray_supervisor.clone(),
        forward_proxy_settings_update_lock: state.forward_proxy_settings_update_lock.clone(),
        forward_proxy_subscription_refresh_lock: state
            .forward_proxy_subscription_refresh_lock
            .clone(),
        pricing_settings_update_lock: state.pricing_settings_update_lock.clone(),
        pricing_catalog: state.pricing_catalog.clone(),
        prompt_cache_conversation_cache: state.prompt_cache_conversation_cache.clone(),
        maintenance_stats_cache: state.maintenance_stats_cache.clone(),
        pool_routing_reservations: state.pool_routing_reservations.clone(),
        pool_live_attempt_ids: state.pool_live_attempt_ids.clone(),
        pool_group_429_retry_delay_override: delay,
        pool_no_available_wait: state.pool_no_available_wait,
        upstream_accounts: state.upstream_accounts.clone(),
    })
}

async fn test_state_from_existing_pool(
    pool: SqlitePool,
    config: AppConfig,
    startup_ready: bool,
) -> Arc<AppState> {
    ensure_schema(&pool)
        .await
        .expect("schema should initialize for existing pool");

    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let pricing_catalog = load_pricing_catalog(&pool)
        .await
        .expect("pricing catalog should initialize");

    Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        startup_ready: Arc::new(AtomicBool::new(startup_ready)),
        shutdown: CancellationToken::new(),
        semaphore,
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
        pricing_catalog: Arc::new(RwLock::new(pricing_catalog)),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    })
}

async fn apply_forward_proxy_settings_without_bootstrap(
    state: &Arc<AppState>,
    settings: ForwardProxySettings,
) -> ForwardProxySettingsResponse {
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
    }
    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("sync forward proxy routes for test settings");
    build_forward_proxy_settings_response(state.as_ref())
        .await
        .expect("build forward proxy settings response")
}

async fn seed_pool_routing_api_key(state: &Arc<AppState>, api_key: &str) {
    ensure_upstream_accounts_schema(&state.pool)
        .await
        .expect("ensure upstream account schema");
    let payload: UpdatePoolRoutingSettingsRequest = serde_json::from_value(json!({
        "apiKey": api_key,
    }))
    .expect("deserialize pool routing settings request");
    let _ = update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect("save pool routing api key");
}

fn test_required_group_name() -> &'static str {
    "test-direct-group"
}

fn test_required_group_bound_proxy_keys() -> Vec<String> {
    vec![FORWARD_PROXY_DIRECT_KEY.to_string()]
}

async fn ensure_test_group_binding(pool: &SqlitePool, group_name: &str, note: Option<&str>) {
    let now_iso = format_utc_iso(Utc::now());
    let bound_proxy_keys_json = serde_json::to_string(&test_required_group_bound_proxy_keys())
        .expect("encode test direct group bindings");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_group_notes (
            group_name, note, bound_proxy_keys_json, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?4)
        ON CONFLICT(group_name) DO UPDATE SET
            note = COALESCE(excluded.note, pool_upstream_account_group_notes.note),
            bound_proxy_keys_json = excluded.bound_proxy_keys_json,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(group_name)
    .bind(note.unwrap_or(""))
    .bind(bound_proxy_keys_json)
    .bind(&now_iso)
    .execute(pool)
    .await
    .expect("ensure test group binding");
}

async fn insert_test_pool_api_key_account(
    state: &Arc<AppState>,
    display_name: &str,
    api_key: &str,
) -> i64 {
    insert_test_pool_api_key_account_with_options(state, display_name, api_key, None, None, None)
        .await
}

async fn insert_test_pool_api_key_account_with_options(
    state: &Arc<AppState>,
    display_name: &str,
    api_key: &str,
    group_name: Option<&str>,
    is_mother: Option<bool>,
    upstream_base_url: Option<&str>,
) -> i64 {
    ensure_upstream_accounts_schema(&state.pool)
        .await
        .expect("ensure upstream account schema");
    let normalized_group_name = group_name.unwrap_or(test_required_group_name());
    ensure_test_group_binding(&state.pool, normalized_group_name, None).await;
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": display_name,
        "apiKey": api_key,
        "groupName": normalized_group_name,
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "isMother": is_mother,
        "upstreamBaseUrl": upstream_base_url,
    }))
    .expect("deserialize api key account request");
    let Json(detail) =
        create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("insert test pool upstream account");
    let _ = detail;
    sqlx::query_scalar("SELECT id FROM pool_upstream_accounts WHERE display_name = ?1")
        .bind(display_name)
        .fetch_one(&state.pool)
        .await
        .expect("load inserted test pool upstream account id")
}

async fn create_test_fast_mode_tag(
    state: &Arc<AppState>,
    name: &str,
    fast_mode_rewrite_mode: &str,
    priority_tier: &str,
) -> i64 {
    let payload = serde_json::from_value::<CreateTagRequest>(json!({
        "name": name,
        "guardEnabled": false,
        "allowCutOut": true,
        "allowCutIn": true,
        "priorityTier": priority_tier,
        "fastModeRewriteMode": fast_mode_rewrite_mode,
    }))
    .expect("deserialize fast mode tag payload");
    let Json(_) = create_tag(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect("create fast mode tag");
    sqlx::query_scalar("SELECT id FROM pool_tags WHERE name = ?1")
        .bind(name)
        .fetch_one(&state.pool)
        .await
        .expect("load fast mode tag id")
}

async fn create_test_tagged_pool_api_key_account(
    state: &Arc<AppState>,
    display_name: &str,
    api_key: &str,
    upstream_base_url: &str,
    tag_ids: &[i64],
) -> i64 {
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": display_name,
        "groupName": test_required_group_name(),
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "upstreamBaseUrl": upstream_base_url,
        "apiKey": api_key,
        "tagIds": tag_ids,
    }))
    .expect("deserialize tagged api-key account payload");
    let Json(_) = create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect("create tagged pool account");
    sqlx::query_scalar("SELECT id FROM pool_upstream_accounts WHERE display_name = ?1")
        .bind(display_name)
        .fetch_one(&state.pool)
        .await
        .expect("load tagged pool account id")
}

async fn insert_test_pool_limit_sample(
    state: &Arc<AppState>,
    account_id: i64,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
) {
    insert_test_pool_limit_sample_with_windows(
        state,
        account_id,
        None,
        primary_used_percent,
        Some(300),
        None,
        secondary_used_percent,
        Some(300),
        None,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn insert_test_pool_limit_sample_with_windows(
    state: &Arc<AppState>,
    account_id: i64,
    plan_type: Option<&str>,
    primary_used_percent: Option<f64>,
    primary_window_minutes: Option<i64>,
    primary_resets_at: Option<&str>,
    secondary_used_percent: Option<f64>,
    secondary_window_minutes: Option<i64>,
    secondary_resets_at: Option<&str>,
) {
    ensure_upstream_accounts_schema(&state.pool)
        .await
        .expect("ensure upstream account schema");
    let captured_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_limit_samples (
            account_id, captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        ) VALUES (
            ?1, ?2, NULL, NULL, ?3,
            ?4, ?5, ?6,
            ?7, ?8, ?9,
            NULL, NULL, NULL
        )
        "#,
    )
    .bind(account_id)
    .bind(&captured_at)
    .bind(plan_type)
    .bind(primary_used_percent)
    .bind(primary_window_minutes)
    .bind(primary_resets_at)
    .bind(secondary_used_percent)
    .bind(secondary_window_minutes)
    .bind(secondary_resets_at)
    .execute(&state.pool)
    .await
    .expect("insert test pool limit sample");
}

async fn reserve_test_pool_routing_account(
    state: &Arc<AppState>,
    reservation_key: &str,
    account_id: i64,
) {
    let account = PoolResolvedAccount {
        account_id,
        display_name: format!("reserved-{account_id}"),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: format!("Bearer reserved-{account_id}"),
        },
        upstream_base_url: Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        group_upstream_429_retry_enabled: false,
        group_upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
    };
    reserve_pool_routing_account(state.as_ref(), reservation_key, &account);
}

async fn set_test_account_local_limits(
    pool: &SqlitePool,
    account_id: i64,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
) {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET local_primary_limit = ?1,
            local_secondary_limit = ?2,
            updated_at = ?3
        WHERE id = ?4
        "#,
    )
    .bind(local_primary_limit)
    .bind(local_secondary_limit)
    .bind(&now_iso)
    .bind(account_id)
    .execute(pool)
    .await
    .expect("set test account local limits");
}

async fn set_test_account_status(pool: &SqlitePool, account_id: i64, status: &str) {
    sqlx::query("UPDATE pool_upstream_accounts SET status = ?1 WHERE id = ?2")
        .bind(status)
        .bind(account_id)
        .execute(pool)
        .await
        .expect("set test pool account status");
}

async fn clear_test_account_credentials(pool: &SqlitePool, account_id: i64) {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET encrypted_credentials = NULL,
            updated_at = ?1
        WHERE id = ?2
        "#,
    )
    .bind(&now_iso)
    .bind(account_id)
    .execute(pool)
    .await
    .expect("clear test pool account credentials");
}

async fn set_test_account_rate_limited_cooldown(
    pool: &SqlitePool,
    account_id: i64,
    cooldown_secs: i64,
) {
    set_test_account_route_cooldown(
        pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        "test rate limit cooldown",
        cooldown_secs,
    )
    .await;
}

async fn set_test_account_generic_route_cooldown(
    pool: &SqlitePool,
    account_id: i64,
    cooldown_secs: i64,
) {
    set_test_account_route_cooldown(
        pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        "test generic cooldown",
        cooldown_secs,
    )
    .await;
}

async fn set_test_account_degraded_route_state(
    pool: &SqlitePool,
    account_id: i64,
    failure_kind: &str,
    error_message: &str,
) {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?1,
            last_error = ?2,
            last_error_at = ?3,
            last_route_failure_at = ?3,
            last_route_failure_kind = ?4,
            cooldown_until = NULL,
            consecutive_route_failures = 1,
            temporary_route_failure_streak_started_at = ?3,
            updated_at = ?3
        WHERE id = ?5
        "#,
    )
    .bind("active")
    .bind(error_message)
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(account_id)
    .execute(pool)
    .await
    .expect("set test pool account degraded route state");
}

async fn set_test_account_route_cooldown(
    pool: &SqlitePool,
    account_id: i64,
    failure_kind: &str,
    error_message: &str,
    cooldown_secs: i64,
) {
    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    let cooldown_until = format_utc_iso(now + ChronoDuration::seconds(cooldown_secs));
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?1,
            last_error = ?2,
            last_error_at = ?3,
            last_route_failure_at = ?3,
            last_route_failure_kind = ?4,
            cooldown_until = ?5,
            consecutive_route_failures = 1,
            temporary_route_failure_streak_started_at = NULL,
            updated_at = ?3
        WHERE id = ?6
        "#,
    )
    .bind("active")
    .bind(error_message)
    .bind(&now_iso)
    .bind(failure_kind)
    .bind(cooldown_until)
    .bind(account_id)
    .execute(pool)
    .await
    .expect("set test pool account route cooldown");
}

async fn upsert_test_sticky_route_at(
    pool: &SqlitePool,
    sticky_key: &str,
    account_id: i64,
    last_seen_at: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (
            sticky_key, account_id, created_at, updated_at, last_seen_at
        ) VALUES (?1, ?2, ?3, ?3, ?3)
        ON CONFLICT(sticky_key) DO UPDATE SET
            account_id = excluded.account_id,
            updated_at = excluded.updated_at,
            last_seen_at = excluded.last_seen_at
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(last_seen_at)
    .execute(pool)
    .await
    .expect("upsert test sticky route");
}

fn format_test_recent_active_timestamp(now: DateTime<Utc>) -> String {
    format_utc_iso(now - ChronoDuration::minutes(4))
}

fn format_test_stale_active_timestamp(now: DateTime<Utc>) -> String {
    format_utc_iso(now - ChronoDuration::minutes(6))
}

async fn insert_test_pool_oauth_account(
    state: &Arc<AppState>,
    display_name: &str,
    access_token: &str,
) -> i64 {
    insert_test_pool_oauth_account_with_chatgpt_account_id(
        state,
        display_name,
        access_token,
        "org_test",
    )
    .await
}

async fn insert_test_pool_oauth_account_with_chatgpt_account_id(
    state: &Arc<AppState>,
    display_name: &str,
    access_token: &str,
    chatgpt_account_id: &str,
) -> i64 {
    ensure_upstream_accounts_schema(&state.pool)
        .await
        .expect("ensure upstream account schema");
    ensure_test_group_binding(&state.pool, test_required_group_name(), None).await;
    let encrypted_credentials = encrypt_test_oauth_credentials(access_token);
    let now_iso = format_utc_iso(Utc::now());

    let token_expires_at = format_utc_iso(Utc::now() + ChronoDuration::days(30));
    sqlx::query_scalar(
        r#"
        INSERT INTO pool_upstream_accounts (
            kind, provider, display_name, group_name, is_mother, note, status, enabled,
            email, chatgpt_account_id, chatgpt_user_id, plan_type, masked_api_key, encrypted_credentials,
            token_expires_at, last_refreshed_at, last_synced_at, last_successful_sync_at, last_error,
            last_error_at, local_primary_limit, local_secondary_limit, local_limit_unit, created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, 0, NULL, ?5, 1,
            ?6, ?7, ?8, ?9, NULL, ?10,
            ?11, NULL, NULL, NULL, NULL,
            NULL, NULL, NULL, NULL, ?12, ?12
        ) RETURNING id
        "#,
    )
    .bind("oauth_codex")
    .bind("codex")
    .bind(display_name)
    .bind(test_required_group_name())
    .bind("active")
    .bind("oauth@example.com")
    .bind(chatgpt_account_id)
    .bind("user_test")
    .bind("team")
    .bind(encrypted_credentials)
    .bind(&token_expires_at)
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert test oauth account")
}

fn encrypt_test_oauth_credentials(access_token: &str) -> String {
    let key = Sha256::digest(b"test-upstream-account-secret");
    let cipher = Aes256Gcm::new_from_slice(key.as_slice()).expect("valid aes key");
    let plaintext = serde_json::to_vec(&json!({
        "kind": "oauth",
        "accessToken": access_token,
        "refreshToken": "refresh-token",
        "idToken": "header.payload.signature",
        "tokenType": "Bearer",
    }))
    .expect("serialize oauth credentials");
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), plaintext.as_ref())
        .expect("encrypt oauth credentials");
    json!({
        "v": 1,
        "nonce": BASE64_STANDARD.encode(nonce),
        "ciphertext": BASE64_STANDARD.encode(ciphertext),
    })
    .to_string()
}

#[tokio::test]
async fn list_upstream_accounts_includes_last_activity_at() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("account-last-activity")
    .bind("2026-03-11 20:35:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(42_i64)
    .bind(0.12_f64)
    .bind(
        json!({
            "upstreamAccountId": account_id,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert account invocation");
    sqlx::query("UPDATE pool_upstream_accounts SET last_activity_at = ?1 WHERE id = ?2")
        .bind("2026-03-11 20:35:00")
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("persist account last activity");

    let Json(response) =
        list_upstream_accounts(State(state), Query(ListUpstreamAccountsQuery::default()))
            .await
            .expect("list upstream accounts");
    let response_json = serde_json::to_value(response).expect("serialize upstream accounts");
    let account = response_json
        .get("items")
        .and_then(serde_json::Value::as_array)
        .expect("items array")
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(account_id))
        .expect("account summary");

    assert_eq!(response_json["page"].as_u64(), Some(1));
    assert_eq!(response_json["pageSize"].as_u64(), Some(20));
    assert_eq!(response_json["total"].as_u64(), Some(1));
    assert_eq!(response_json["metrics"]["total"].as_u64(), Some(1));
    assert_eq!(response_json["metrics"]["oauth"].as_u64(), Some(0));
    assert_eq!(response_json["metrics"]["apiKey"].as_u64(), Some(1));
    assert_eq!(response_json["metrics"]["attention"].as_u64(), Some(0));
    assert_eq!(
        account
            .get("displayStatus")
            .and_then(serde_json::Value::as_str),
        Some("active")
    );
    assert_eq!(
        account
            .get("lastActivityAt")
            .and_then(serde_json::Value::as_str),
        Some("2026-03-11T12:35:00Z")
    );
}

#[tokio::test]
async fn list_upstream_accounts_filters_groups_and_tags_server_side() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let alpha_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Alpha",
        "upstream-alpha",
        Some("prod-blue"),
        Some(false),
        None,
    )
    .await;
    let beta_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Beta",
        "upstream-beta",
        Some("prod-blue"),
        Some(false),
        None,
    )
    .await;
    let gamma_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Gamma",
        "upstream-gamma",
        None,
        Some(false),
        None,
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
        .bind(gamma_id)
        .execute(&state.pool)
        .await
        .expect("clear gamma group to simulate legacy ungrouped account");
    let now_iso = format_utc_iso(Utc::now());

    let vip_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, guard_enabled, lookback_hours, max_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, NULL, NULL, 1, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("vip")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert vip tag");
    let burst_safe_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, guard_enabled, lookback_hours, max_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, NULL, NULL, 1, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("burst-safe")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert burst-safe tag");

    for (account_id, tag_id) in [
        (alpha_id, vip_tag_id),
        (alpha_id, burst_safe_tag_id),
        (beta_id, vip_tag_id),
        (gamma_id, vip_tag_id),
        (gamma_id, burst_safe_tag_id),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_tags (
                account_id, tag_id, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?3)
            "#,
        )
        .bind(account_id)
        .bind(tag_id)
        .bind(&now_iso)
        .execute(&state.pool)
        .await
        .expect("insert account tag link");
    }

    let Json(group_filtered) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            group_search: Some("prod".to_string()),
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            tag_ids: vec![vip_tag_id, burst_safe_tag_id, vip_tag_id],
        }),
    )
    .await
    .expect("list filtered upstream accounts");
    let group_filtered_json =
        serde_json::to_value(group_filtered).expect("serialize filtered upstream accounts");
    let group_filtered_names = group_filtered_json["items"]
        .as_array()
        .expect("filtered items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(group_filtered_names, vec!["Alpha"]);
    assert_eq!(
        group_filtered_json["hasUngroupedAccounts"].as_bool(),
        Some(true)
    );

    let Json(ungrouped_filtered) = list_upstream_accounts(
        State(state),
        Query(ListUpstreamAccountsQuery {
            group_search: None,
            group_ungrouped: Some(true),
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            tag_ids: vec![vip_tag_id, burst_safe_tag_id],
        }),
    )
    .await
    .expect("list ungrouped filtered upstream accounts");
    let ungrouped_filtered_json = serde_json::to_value(ungrouped_filtered)
        .expect("serialize ungrouped filtered upstream accounts");
    let ungrouped_filtered_names = ungrouped_filtered_json["items"]
        .as_array()
        .expect("ungrouped filtered items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(ungrouped_filtered_names, vec!["Gamma"]);
}

#[tokio::test]
async fn list_upstream_accounts_filters_by_display_status_and_paginate_server_side() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let alpha_id = insert_test_pool_api_key_account(&state, "Alpha", "upstream-alpha").await;
    let beta_id = insert_test_pool_api_key_account(&state, "Beta", "upstream-beta").await;
    let gamma_id = insert_test_pool_api_key_account(&state, "Gamma", "upstream-gamma").await;
    for index in 0..19 {
        let display_name = format!("Extra {index:02}");
        let api_key = format!("upstream-extra-{index:02}");
        insert_test_pool_api_key_account(&state, &display_name, &api_key).await;
    }

    let now = Utc::now();
    sqlx::query(
        "UPDATE pool_upstream_accounts SET last_selected_at = ?2, cooldown_until = ?3 WHERE id = ?1",
    )
    .bind(alpha_id)
    .bind(format_test_recent_active_timestamp(now))
    .bind::<Option<String>>(None)
    .execute(&state.pool)
    .await
    .expect("mark alpha working");
    set_test_account_rate_limited_cooldown(&state.pool, beta_id, 600).await;
    sqlx::query("UPDATE pool_upstream_accounts SET enabled = 0 WHERE id = ?1")
        .bind(beta_id)
        .execute(&state.pool)
        .await
        .expect("disable beta account");
    sqlx::query(
        "UPDATE pool_upstream_accounts SET status = ?2, last_error = ?3, last_error_at = ?4, last_route_failure_at = NULL, last_route_failure_kind = NULL WHERE id = ?1",
    )
    .bind(beta_id)
    .bind("syncing")
    .bind("Authentication token has been invalidated, please sign in again")
    .bind(format_utc_iso(now))
    .execute(&state.pool)
    .await
    .expect("seed beta stale disabled syncing state");
    set_test_account_rate_limited_cooldown(&state.pool, gamma_id, 600).await;

    let Json(active_page_two) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            group_search: None,
            group_ungrouped: None,
            status: Some("active".to_string()),
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(2),
            page_size: Some(20),
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list active upstream accounts page two");
    let active_page_two_json =
        serde_json::to_value(active_page_two).expect("serialize active page two response");
    let active_names = active_page_two_json["items"]
        .as_array()
        .expect("active page items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(active_names, vec!["Alpha"]);
    assert_eq!(active_page_two_json["total"].as_u64(), Some(21));
    assert_eq!(active_page_two_json["page"].as_u64(), Some(2));
    assert_eq!(active_page_two_json["pageSize"].as_u64(), Some(20));
    assert_eq!(active_page_two_json["metrics"]["total"].as_u64(), Some(21));
    assert_eq!(active_page_two_json["metrics"]["apiKey"].as_u64(), Some(21));
    assert_eq!(
        active_page_two_json["metrics"]["attention"].as_u64(),
        Some(1)
    );

    let Json(disabled_only) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            group_search: None,
            group_ungrouped: None,
            status: Some("disabled".to_string()),
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(1),
            page_size: Some(20),
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list disabled upstream accounts");
    let disabled_only_json =
        serde_json::to_value(disabled_only).expect("serialize disabled response");
    let disabled_items = disabled_only_json["items"]
        .as_array()
        .expect("disabled items array");
    assert_eq!(disabled_only_json["total"].as_u64(), Some(1));
    assert_eq!(disabled_items.len(), 1);
    assert_eq!(
        disabled_items[0]
            .get("id")
            .and_then(serde_json::Value::as_i64),
        Some(beta_id)
    );
    assert_eq!(
        disabled_items[0]
            .get("displayStatus")
            .and_then(serde_json::Value::as_str),
        Some("disabled")
    );
    assert_eq!(
        disabled_items[0]
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("normal")
    );
    assert_eq!(
        disabled_items[0]
            .get("syncState")
            .and_then(serde_json::Value::as_str),
        Some("idle")
    );
    assert_eq!(disabled_only_json["metrics"]["attention"].as_u64(), Some(0));

    let Json(split_status_filtered) = list_upstream_accounts(
        State(state),
        Query(ListUpstreamAccountsQuery {
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: vec!["rate_limited".to_string()],
            enable_status: vec!["enabled".to_string()],
            health_status: vec!["normal".to_string()],
            page: Some(1),
            page_size: Some(20),
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list split status filtered upstream accounts");
    let split_status_filtered_json =
        serde_json::to_value(split_status_filtered).expect("serialize split status response");
    let split_items = split_status_filtered_json["items"]
        .as_array()
        .expect("split status items array");
    assert_eq!(split_status_filtered_json["total"].as_u64(), Some(1));
    assert_eq!(split_items.len(), 1);
    assert_eq!(
        split_items[0].get("id").and_then(serde_json::Value::as_i64),
        Some(gamma_id)
    );
    assert_eq!(
        split_items[0]
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("rate_limited")
    );
    assert_eq!(
        split_items[0]
            .get("enableStatus")
            .and_then(serde_json::Value::as_str),
        Some("enabled")
    );
    assert_eq!(
        split_items[0]
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("normal")
    );
    assert_eq!(
        split_status_filtered_json["metrics"]["attention"].as_u64(),
        Some(1)
    );
    assert_ne!(alpha_id, gamma_id);
}

#[tokio::test]
async fn list_upstream_accounts_clamps_work_status_for_abnormal_or_syncing_accounts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let reauth_id =
        insert_test_pool_api_key_account(&state, "Needs Reauth", "upstream-reauth").await;
    let syncing_id =
        insert_test_pool_api_key_account(&state, "Currently Syncing", "upstream-syncing").await;

    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    let cooldown_until = format_utc_iso(now + ChronoDuration::minutes(10));
    let recently_selected = format_test_recent_active_timestamp(now);

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = ?3,
            last_error_at = ?4,
            cooldown_until = ?5,
            last_selected_at = ?6
        WHERE id = ?1
        "#,
    )
    .bind(reauth_id)
    .bind("needs_reauth")
    .bind("refresh token expired")
    .bind(&now_iso)
    .bind(&cooldown_until)
    .bind(&recently_selected)
    .execute(&state.pool)
    .await
    .expect("mark reauth account abnormal");

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = NULL,
            last_error_at = NULL,
            cooldown_until = ?3,
            last_selected_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(syncing_id)
    .bind("syncing")
    .bind(&cooldown_until)
    .bind(&recently_selected)
    .execute(&state.pool)
    .await
    .expect("mark syncing account in cooldown");

    let Json(response) = list_upstream_accounts(
        State(state),
        Query(ListUpstreamAccountsQuery {
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(1),
            page_size: Some(20),
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list upstream accounts with abnormal states");
    let response_json =
        serde_json::to_value(response).expect("serialize abnormal upstream accounts");
    let items = response_json["items"]
        .as_array()
        .expect("abnormal items array");

    let reauth_item = items
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(reauth_id))
        .expect("reauth item present");
    assert_eq!(
        reauth_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("unavailable")
    );
    assert_eq!(
        reauth_item
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("needs_reauth")
    );
    assert_eq!(
        reauth_item
            .get("syncState")
            .and_then(serde_json::Value::as_str),
        Some("idle")
    );

    let syncing_item = items
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(syncing_id))
        .expect("syncing item present");
    assert_eq!(
        syncing_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("idle")
    );
    assert_eq!(
        syncing_item
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("normal")
    );
    assert_eq!(
        syncing_item
            .get("syncState")
            .and_then(serde_json::Value::as_str),
        Some("syncing")
    );
}

#[tokio::test]
async fn list_upstream_accounts_work_status_uses_five_minute_activity_window() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let recent_id =
        insert_test_pool_api_key_account(&state, "Recent Working", "upstream-recent-working").await;
    let stale_id =
        insert_test_pool_api_key_account(&state, "Stale Working", "upstream-stale-working").await;

    let now = Utc::now();
    sqlx::query("UPDATE pool_upstream_accounts SET last_selected_at = ?2 WHERE id = ?1")
        .bind(recent_id)
        .bind(format_test_recent_active_timestamp(now))
        .execute(&state.pool)
        .await
        .expect("mark recent working account");
    sqlx::query("UPDATE pool_upstream_accounts SET last_selected_at = ?2 WHERE id = ?1")
        .bind(stale_id)
        .bind(format_test_stale_active_timestamp(now))
        .execute(&state.pool)
        .await
        .expect("mark stale working account");

    let Json(response) =
        list_upstream_accounts(State(state), Query(ListUpstreamAccountsQuery::default()))
            .await
            .expect("list upstream accounts");
    let payload = serde_json::to_value(response).expect("serialize upstream accounts");
    let items = payload["items"].as_array().expect("items array");

    let recent_item = items
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(recent_id))
        .expect("recent item present");
    assert_eq!(
        recent_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("working")
    );

    let stale_item = items
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(stale_id))
        .expect("stale item present");
    assert_eq!(
        stale_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("idle")
    );
}

#[tokio::test]
async fn upstream_account_summary_and_detail_include_active_conversation_count() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Sticky conversations", "upstream-sticky").await;

    let now = Utc::now();
    let created_at = format_utc_iso(now - ChronoDuration::minutes(20));
    let first_recent_seen_at = format_utc_iso(now - ChronoDuration::minutes(2));
    let second_recent_seen_at = format_test_recent_active_timestamp(now);
    let stale_seen_at = format_test_stale_active_timestamp(now);

    for (sticky_key, last_seen_at) in [
        ("sticky-recent-1", first_recent_seen_at.as_str()),
        ("sticky-recent-2", second_recent_seen_at.as_str()),
        ("sticky-stale", stale_seen_at.as_str()),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_routes (
                sticky_key, account_id, created_at, updated_at, last_seen_at
            ) VALUES (?1, ?2, ?3, ?3, ?4)
            "#,
        )
        .bind(sticky_key)
        .bind(account_id)
        .bind(&created_at)
        .bind(last_seen_at)
        .execute(&state.pool)
        .await
        .expect("insert sticky route");
    }

    let Json(list_response) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery::default()),
    )
    .await
    .expect("list upstream accounts");
    let list_json = serde_json::to_value(list_response).expect("serialize upstream account list");
    let list_item = list_json["items"]
        .as_array()
        .and_then(|items| items.first())
        .expect("list item present");
    assert_eq!(
        list_item
            .get("activeConversationCount")
            .and_then(serde_json::Value::as_i64),
        Some(2)
    );

    let Json(detail_response) = get_upstream_account(State(state), axum::extract::Path(account_id))
        .await
        .expect("load upstream account detail");
    let detail_json =
        serde_json::to_value(detail_response).expect("serialize upstream account detail");
    assert_eq!(
        detail_json
            .get("activeConversationCount")
            .and_then(serde_json::Value::as_i64),
        Some(2)
    );
}

#[tokio::test]
async fn list_upstream_accounts_keeps_generic_retry_cooldown_idle() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let generic_cooldown_id =
        insert_test_pool_api_key_account(&state, "Generic Cooldown", "upstream-generic").await;

    set_test_account_generic_route_cooldown(&state.pool, generic_cooldown_id, 600).await;

    let Json(response) = list_upstream_accounts(
        State(state),
        Query(ListUpstreamAccountsQuery {
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(1),
            page_size: Some(20),
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list upstream accounts with generic cooldown");
    let response_json =
        serde_json::to_value(response).expect("serialize generic cooldown upstream accounts");
    let items = response_json["items"]
        .as_array()
        .expect("generic cooldown items array");

    let generic_item = items
        .iter()
        .find(|item| {
            item.get("id").and_then(serde_json::Value::as_i64) == Some(generic_cooldown_id)
        })
        .expect("generic cooldown item present");
    assert_eq!(
        generic_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("degraded")
    );
    assert_eq!(
        generic_item
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("normal")
    );
    assert_eq!(response_json["metrics"]["attention"].as_u64(), Some(1));
}

#[tokio::test]
async fn list_upstream_accounts_includes_archived_last_activity_at() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("create archive activity pool");
    let account_id = 17_i64;

    sqlx::query(
        "CREATE TABLE pool_upstream_accounts (id INTEGER PRIMARY KEY, last_activity_at TEXT)",
    )
    .execute(&pool)
    .await
    .expect("create accounts table");
    sqlx::query("CREATE TABLE codex_invocations (occurred_at TEXT NOT NULL, payload TEXT)")
        .execute(&pool)
        .await
        .expect("create active invocation table");
    sqlx::query("INSERT INTO pool_upstream_accounts (id, last_activity_at) VALUES (?1, ?2)")
        .bind(account_id)
        .bind("2026-03-12 07:05:00")
        .execute(&pool)
        .await
        .expect("seed persisted last activity");

    let last_activity = load_account_last_activity_map(&pool, &[account_id])
        .await
        .expect("load last activity map");

    assert_eq!(
        last_activity.get(&account_id).map(String::as_str),
        Some("2026-03-12 07:05:00")
    );
}

#[tokio::test]
async fn create_api_key_account_enforces_single_mother_per_group() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let first_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "sk-primary",
        Some("prod"),
        Some(true),
        None,
    )
    .await;

    let second_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "sk-secondary",
        Some("prod"),
        Some(true),
        None,
    )
    .await;

    let first_is_mother: i64 =
        sqlx::query_scalar("SELECT is_mother FROM pool_upstream_accounts WHERE id = ?1")
            .bind(first_id)
            .fetch_one(&state.pool)
            .await
            .expect("load first mother flag");
    let second_is_mother: i64 =
        sqlx::query_scalar("SELECT is_mother FROM pool_upstream_accounts WHERE id = ?1")
            .bind(second_id)
            .fetch_one(&state.pool)
            .await
            .expect("load second mother flag");

    assert_eq!(first_is_mother, 0);
    assert_eq!(second_is_mother, 1);
}

#[tokio::test]
async fn create_api_key_account_persists_upstream_base_url() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Gateway Key",
        "apiKey": "sk-gateway",
        "groupName": test_required_group_name(),
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "upstreamBaseUrl": "https://proxy.example.com/gateway",
    }))
    .expect("deserialize api key account request");
    let Json(detail) =
        create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("create api key account");

    let detail_json = serde_json::to_value(detail).expect("serialize detail");
    assert_eq!(
        detail_json["upstreamBaseUrl"].as_str(),
        Some("https://proxy.example.com/gateway")
    );

    let stored: Option<String> = sqlx::query_scalar(
        "SELECT upstream_base_url FROM pool_upstream_accounts WHERE display_name = ?1",
    )
    .bind("Gateway Key")
    .fetch_one(&state.pool)
    .await
    .expect("load stored upstream base url");
    assert_eq!(stored.as_deref(), Some("https://proxy.example.com/gateway"));
}

#[tokio::test]
async fn update_upstream_account_can_clear_upstream_base_url_with_null_payload() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Gateway Key",
        "sk-gateway",
        None,
        None,
        Some("https://proxy.example.com/gateway"),
    )
    .await;

    let payload: UpdateUpstreamAccountRequest = serde_json::from_value(json!({
        "upstreamBaseUrl": null,
    }))
    .expect("deserialize update request");
    let Json(detail) = update_upstream_account(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path(account_id),
        Json(payload),
    )
    .await
    .expect("clear upstream base url");

    let detail_json = serde_json::to_value(detail).expect("serialize detail");
    assert!(detail_json["upstreamBaseUrl"].is_null());

    let stored: Option<String> =
        sqlx::query_scalar("SELECT upstream_base_url FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load cleared upstream base url");
    assert_eq!(stored, None);
}

#[tokio::test]
async fn delete_upstream_account_removes_related_rows_in_one_transaction() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Delete Target",
        "sk-delete-target",
        Some("prod"),
        Some(false),
        None,
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    let tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, guard_enabled, lookback_hours, max_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, NULL, NULL, 1, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("delete-tag")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert tag");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(account_id)
    .bind(tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("insert account tag link");

    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, group_name, is_mother, note, tag_ids_json, group_note,
            state, pkce_verifier, redirect_uri, status, auth_url, error_message, expires_at, consumed_at,
            created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, 0, NULL, NULL, NULL,
            ?5, ?6, ?7, ?8, ?9, NULL, ?10, NULL,
            ?11, ?11
        )
        "#,
    )
    .bind("login-delete-target")
    .bind(account_id)
    .bind("Delete Target")
    .bind("prod")
    .bind("state-delete-target")
    .bind("pkce-delete-target")
    .bind("https://example.com/callback")
    .bind("completed")
    .bind("https://example.com/auth")
    .bind(&now_iso)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("insert oauth login session");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_group_notes (
            group_name, note, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        ON CONFLICT(group_name) DO UPDATE SET
            note = excluded.note,
            updated_at = excluded.updated_at
        "#,
    )
    .bind("prod")
    .bind("cleanup me")
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("insert group note");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_limit_samples (
            account_id, captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        ) VALUES (
            ?1, ?2, 'primary', 'Primary', 'team',
            12.5, 300, ?2,
            25.0, 10080, ?2,
            1, 0, '42'
        )
        "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("insert limit sample");

    let status = delete_upstream_account(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path(account_id),
    )
    .await
    .expect("delete upstream account");
    assert_eq!(status, StatusCode::NO_CONTENT);

    let remaining_accounts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("count remaining accounts");
    assert_eq!(remaining_accounts, 0);

    let remaining_tag_links: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_account_tags WHERE account_id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("count remaining tag links");
    assert_eq!(remaining_tag_links, 0);

    let remaining_sessions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_oauth_login_sessions WHERE account_id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("count remaining login sessions");
    assert_eq!(remaining_sessions, 0);

    let remaining_samples: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_limit_samples WHERE account_id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("count remaining limit samples");
    assert_eq!(remaining_samples, 0);

    let remaining_group_notes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_group_notes WHERE group_name = ?1",
    )
    .bind("prod")
    .fetch_one(&state.pool)
    .await
    .expect("count remaining group notes");
    assert_eq!(remaining_group_notes, 0);
}

#[tokio::test]
async fn create_api_key_account_rejects_invalid_upstream_base_url() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Broken Key",
        "apiKey": "sk-broken",
        "groupName": test_required_group_name(),
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "upstreamBaseUrl": "not-a-url",
    }))
    .expect("deserialize api key account request");

    let err = create_api_key_account(State(state), HeaderMap::new(), Json(payload))
        .await
        .expect_err("invalid upstream base url should fail");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(err.1, "upstreamBaseUrl must be a valid absolute URL");
}

#[tokio::test]
async fn update_upstream_account_group_rejects_bindings_without_selectable_nodes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "LATAM Key",
        "sk-latam",
        Some("latam"),
        None,
        None,
    )
    .await;

    let payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "boundProxyKeys": ["fpn_missing_legacy_vless"]
    }))
    .expect("deserialize update upstream account group request");
    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(payload),
    )
    .await
    .expect_err("group binding without selectable nodes should fail");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "select at least one available proxy node or clear bindings before saving"
    );
}

#[tokio::test]
async fn update_upstream_account_group_canonicalizes_historical_runtime_keys_to_logical_binding_keys()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "LATAM Key",
        "sk-latam",
        Some("latam"),
        None,
        None,
    )
    .await;

    let current_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Fcurrent&sni=current.example.com#东京专线".to_string();
    let legacy_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Flegacy&sni=legacy.example.com#东京专线".to_string();
    let settings = ForwardProxySettings {
        proxy_urls: vec![current_proxy_url.clone()],
        subscription_urls: Vec::new(),
        subscription_update_interval_secs: 3600,
        insert_direct: false,
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("persist current forward proxy settings");
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
        for endpoint in &mut manager.endpoints {
            endpoint.endpoint_url = Some(
                Url::parse("socks5://127.0.0.1:11083")
                    .expect("parse synthesized binding endpoint url"),
            );
        }
    }

    let legacy_proxy_key =
        normalize_single_proxy_key(&legacy_proxy_url).expect("normalize legacy runtime proxy key");
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: legacy_proxy_key.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(
                normalize_share_link_scheme(&legacy_proxy_url, "vless")
                    .expect("normalize legacy share link"),
            ),
            weight: 0.61,
            success_ema: 0.81,
            latency_ema_ms: Some(140.0),
            consecutive_failures: 0,
        },
    )
    .await
    .expect("persist legacy runtime state for metadata history");

    let payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "boundProxyKeys": [legacy_proxy_key]
    }))
    .expect("deserialize update upstream account group request");
    let Json(updated) = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(payload),
    )
    .await
    .expect("legacy runtime key should canonicalize during save");

    let binding_key = forward_proxy_binding_key_candidates(
        &forward_proxy_binding_parts_from_raw(&current_proxy_url, None)
            .expect("binding parts from current proxy url"),
    )[0]
    .clone();
    let updated_value = serde_json::to_value(&updated).expect("serialize updated group summary");
    assert_eq!(
        updated_value
            .get("boundProxyKeys")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        vec![Value::String(binding_key.clone())]
    );
    let stored_json: String = sqlx::query_scalar(
        "SELECT bound_proxy_keys_json FROM pool_upstream_account_group_notes WHERE group_name = ?1",
    )
    .bind("latam")
    .fetch_one(&state.pool)
    .await
    .expect("load stored group metadata after update");
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&stored_json)
            .expect("decode stored bound proxy keys json"),
        vec![binding_key]
    );
}

#[tokio::test]
async fn update_upstream_account_group_returns_not_found_before_binding_validation() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "boundProxyKeys": ["fpn_missing_legacy_vless"]
    }))
    .expect("deserialize update upstream account group request");
    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        axum::extract::Path("missing-group".to_string()),
        Json(payload),
    )
    .await
    .expect_err("missing group should still return not found");

    assert_eq!(err.0, StatusCode::NOT_FOUND);
    assert_eq!(err.1, "group not found");
}

#[tokio::test]
async fn ensure_schema_adds_group_upstream_429_retry_columns_with_disabled_defaults() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");

    sqlx::query(
        r#"
        CREATE TABLE pool_upstream_account_group_notes (
            group_name TEXT PRIMARY KEY,
            note TEXT NOT NULL,
            bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy group metadata table");

    ensure_schema(&pool)
        .await
        .expect("schema migration should succeed");

    let columns = load_sqlite_table_columns(&pool, "pool_upstream_account_group_notes")
        .await
        .expect("load migrated columns");
    assert!(columns.contains("upstream_429_retry_enabled"));
    assert!(columns.contains("upstream_429_max_retries"));

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_group_notes (
            group_name,
            note,
            bound_proxy_keys_json,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?4)
        "#,
    )
    .bind("latam")
    .bind("Legacy group")
    .bind("[]")
    .bind(&now_iso)
    .execute(&pool)
    .await
    .expect("insert migrated group metadata");

    let retry_settings = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT upstream_429_retry_enabled, upstream_429_max_retries
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind("latam")
    .fetch_one(&pool)
    .await
    .expect("load retry settings");
    assert_eq!(retry_settings.0, 0);
    assert_eq!(retry_settings.1, 0);
}

#[tokio::test]
async fn update_upstream_account_group_preserves_upstream_429_retry_settings_when_omitted() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "LATAM Key",
        "sk-latam",
        Some("latam"),
        None,
        None,
    )
    .await;

    let initial_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "note": "LATAM premium",
        "upstream429RetryEnabled": true,
        "upstream429MaxRetries": 4
    }))
    .expect("deserialize initial group payload");
    let Json(initial_saved) = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(initial_payload),
    )
    .await
    .expect("save group retry settings");
    let initial_saved_json = serde_json::to_value(initial_saved).expect("serialize saved group");
    assert_eq!(
        initial_saved_json["upstream429RetryEnabled"].as_bool(),
        Some(true)
    );
    assert_eq!(
        initial_saved_json["upstream429MaxRetries"].as_u64(),
        Some(4)
    );

    let legacy_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "note": "LATAM refreshed"
    }))
    .expect("deserialize legacy payload");
    let Json(updated) = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(legacy_payload),
    )
    .await
    .expect("legacy update should preserve retry settings");
    let updated_json = serde_json::to_value(updated).expect("serialize updated group");
    assert_eq!(updated_json["note"].as_str(), Some("LATAM refreshed"));
    assert_eq!(
        updated_json["upstream429RetryEnabled"].as_bool(),
        Some(true)
    );
    assert_eq!(updated_json["upstream429MaxRetries"].as_u64(), Some(4));

    let persisted = sqlx::query_as::<_, (String, i64, i64)>(
        r#"
        SELECT note, upstream_429_retry_enabled, upstream_429_max_retries
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind("latam")
    .fetch_one(&state.pool)
    .await
    .expect("load persisted group retry settings");
    assert_eq!(persisted.0, "LATAM refreshed");
    assert_eq!(persisted.1, 1);
    assert_eq!(persisted.2, 4);
}

#[tokio::test]
async fn update_upstream_account_group_enabling_retry_defaults_missing_retry_count_to_one() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "LATAM Key",
        "sk-latam",
        Some("latam"),
        None,
        None,
    )
    .await;

    let enable_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "upstream429RetryEnabled": true
    }))
    .expect("deserialize enable payload");
    let Json(updated) = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(enable_payload),
    )
    .await
    .expect("enable group retry settings");
    let updated_json = serde_json::to_value(updated).expect("serialize updated group");
    assert_eq!(
        updated_json["upstream429RetryEnabled"].as_bool(),
        Some(true)
    );
    assert_eq!(updated_json["upstream429MaxRetries"].as_u64(), Some(1));

    let persisted = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT upstream_429_retry_enabled, upstream_429_max_retries
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind("latam")
    .fetch_one(&state.pool)
    .await
    .expect("load persisted group retry settings");
    assert_eq!(persisted.0, 1);
    assert_eq!(persisted.1, 1);
}
