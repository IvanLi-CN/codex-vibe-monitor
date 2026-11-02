use super::super::*;
use serde_json::json;

pub(crate) async fn enable_encrypted_session_owner_routing_for_test(state: &Arc<AppState>) {
    let mut settings = state.proxy_model_settings.write().await;
    settings.encrypted_session_owner_routing_enabled = true;
}

pub(crate) fn clone_state_with_upstream_accounts(
    state: &Arc<AppState>,
    upstream_accounts: Arc<UpstreamAccountsRuntime>,
) -> Arc<AppState> {
    Arc::new(AppState {
        system_storage: state.system_storage.clone(),
        config: state.config.clone(),
        sqlite_batch_writer: state.sqlite_batch_writer.clone(),
        pool_account_selection_runtime: state.pool_account_selection_runtime.clone(),
        proxy_runtime_invocations: state.proxy_runtime_invocations.clone(),
        proxy_runtime_attempts: state.proxy_runtime_attempts.clone(),
        pool: state.pool.clone(),
        oauth_installation_seed: state.oauth_installation_seed,
        hourly_rollup_sync_lock: state.hourly_rollup_sync_lock.clone(),
        http_clients: state.http_clients.clone(),
        broadcaster: state.broadcaster.clone(),
        subscription_hub: state.subscription_hub.clone(),
        broadcast_state_cache: state.broadcast_state_cache.clone(),
        proxy_summary_quota_broadcast_seq: state.proxy_summary_quota_broadcast_seq.clone(),
        proxy_summary_quota_broadcast_running: state.proxy_summary_quota_broadcast_running.clone(),
        proxy_summary_quota_broadcast_handle: state.proxy_summary_quota_broadcast_handle.clone(),
        dashboard_activity_live_broadcast_seq: state.dashboard_activity_live_broadcast_seq.clone(),
        dashboard_activity_live_broadcast_running: state
            .dashboard_activity_live_broadcast_running
            .clone(),
        process_started_at_utc: state.process_started_at_utc,
        observability: state.observability.clone(),
        dashboard_network_speed_cache: state.dashboard_network_speed_cache.clone(),
        startup_ready: state.startup_ready.clone(),
        shutdown: state.shutdown.clone(),
        semaphore: state.semaphore.clone(),
        proxy_request_in_flight: state.proxy_request_in_flight.clone(),
        proxy_raw_async_semaphore: state.proxy_raw_async_semaphore.clone(),
        raw_capture_circuit: state.raw_capture_circuit.clone(),
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
        dashboard_activity_snapshot_cache: state.dashboard_activity_snapshot_cache.clone(),
        terminal_projection_hub: state.terminal_projection_hub.clone(),
        long_term_projection_runtime: state.long_term_projection_runtime.clone(),
        memory_diagnostics: state.memory_diagnostics.clone(),
        maintenance_stats_cache: state.maintenance_stats_cache.clone(),
        system_status_cache: state.system_status_cache.clone(),
        pool_routing_reservations: state.pool_routing_reservations.clone(),
        pool_routing_availability: PoolRoutingAvailabilitySignal::default(),
        pool_routing_runtime_cache: state.pool_routing_runtime_cache.clone(),
        pool_routing_test_data_version_connection: state
            .pool_routing_test_data_version_connection
            .clone(),
        pool_model_routing_cache_write_lock: state.pool_model_routing_cache_write_lock.clone(),
        pool_live_attempt_ids: state.pool_live_attempt_ids.clone(),
        pool_group_429_retry_delay_override: state.pool_group_429_retry_delay_override,
        fallback_proxy_429_retry_delay_override: state.fallback_proxy_429_retry_delay_override,
        pool_no_available_wait: state.pool_no_available_wait,
        upstream_accounts,
    })
}

fn clone_state_with_retry_delay_overrides(
    state: &Arc<AppState>,
    pool_group_delay: Option<Duration>,
    fallback_delay: Option<Duration>,
) -> Arc<AppState> {
    Arc::new(AppState {
        system_storage: state.system_storage.clone(),
        config: state.config.clone(),
        sqlite_batch_writer: state.sqlite_batch_writer.clone(),
        pool_account_selection_runtime: state.pool_account_selection_runtime.clone(),
        proxy_runtime_invocations: state.proxy_runtime_invocations.clone(),
        proxy_runtime_attempts: state.proxy_runtime_attempts.clone(),
        pool: state.pool.clone(),
        oauth_installation_seed: state.oauth_installation_seed,
        hourly_rollup_sync_lock: state.hourly_rollup_sync_lock.clone(),
        http_clients: state.http_clients.clone(),
        broadcaster: state.broadcaster.clone(),
        subscription_hub: state.subscription_hub.clone(),
        broadcast_state_cache: state.broadcast_state_cache.clone(),
        proxy_summary_quota_broadcast_seq: state.proxy_summary_quota_broadcast_seq.clone(),
        proxy_summary_quota_broadcast_running: state.proxy_summary_quota_broadcast_running.clone(),
        proxy_summary_quota_broadcast_handle: state.proxy_summary_quota_broadcast_handle.clone(),
        dashboard_activity_live_broadcast_seq: state.dashboard_activity_live_broadcast_seq.clone(),
        dashboard_activity_live_broadcast_running: state
            .dashboard_activity_live_broadcast_running
            .clone(),
        process_started_at_utc: state.process_started_at_utc,
        observability: state.observability.clone(),
        dashboard_network_speed_cache: state.dashboard_network_speed_cache.clone(),
        startup_ready: state.startup_ready.clone(),
        shutdown: state.shutdown.clone(),
        semaphore: state.semaphore.clone(),
        proxy_request_in_flight: state.proxy_request_in_flight.clone(),
        proxy_raw_async_semaphore: state.proxy_raw_async_semaphore.clone(),
        raw_capture_circuit: state.raw_capture_circuit.clone(),
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
        dashboard_activity_snapshot_cache: state.dashboard_activity_snapshot_cache.clone(),
        terminal_projection_hub: state.terminal_projection_hub.clone(),
        long_term_projection_runtime: state.long_term_projection_runtime.clone(),
        memory_diagnostics: state.memory_diagnostics.clone(),
        maintenance_stats_cache: state.maintenance_stats_cache.clone(),
        system_status_cache: state.system_status_cache.clone(),
        pool_routing_reservations: state.pool_routing_reservations.clone(),
        pool_routing_availability: PoolRoutingAvailabilitySignal::default(),
        pool_routing_runtime_cache: state.pool_routing_runtime_cache.clone(),
        pool_routing_test_data_version_connection: state
            .pool_routing_test_data_version_connection
            .clone(),
        pool_model_routing_cache_write_lock: state.pool_model_routing_cache_write_lock.clone(),
        pool_live_attempt_ids: state.pool_live_attempt_ids.clone(),
        pool_group_429_retry_delay_override: pool_group_delay,
        fallback_proxy_429_retry_delay_override: fallback_delay,
        pool_no_available_wait: state.pool_no_available_wait,
        upstream_accounts: state.upstream_accounts.clone(),
    })
}

pub(crate) fn clone_state_with_pool_group_429_retry_delay_override(
    state: &Arc<AppState>,
    delay: Option<Duration>,
) -> Arc<AppState> {
    clone_state_with_retry_delay_overrides(
        state,
        delay,
        state.fallback_proxy_429_retry_delay_override,
    )
}

pub(crate) fn clone_state_with_fallback_proxy_429_retry_delay_override(
    state: &Arc<AppState>,
    delay: Option<Duration>,
) -> Arc<AppState> {
    clone_state_with_retry_delay_overrides(state, state.pool_group_429_retry_delay_override, delay)
}

pub(crate) async fn test_state_from_existing_pool(
    pool: SqlitePool,
    config: AppConfig,
    startup_ready: bool,
) -> Arc<AppState> {
    ensure_schema(&pool)
        .await
        .expect("schema should initialize for existing pool");
    complete_prompt_cache_conversation_materialization_for_test(&pool).await;

    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let pricing_catalog = load_pricing_catalog(&pool)
        .await
        .expect("pricing catalog should initialize");

    Arc::new(AppState {
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
        startup_ready: Arc::new(AtomicBool::new(startup_ready)),
        shutdown: CancellationToken::new(),
        semaphore,
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
        pricing_catalog: Arc::new(RwLock::new(pricing_catalog)),
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
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_availability: PoolRoutingAvailabilitySignal::default(),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        pool_routing_test_data_version_connection: Arc::new(Mutex::new(None)),
        pool_model_routing_cache_write_lock: Arc::new(Mutex::new(())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        fallback_proxy_429_retry_delay_override: Some(Duration::ZERO),
        pool_no_available_wait: immediate_test_pool_no_available_wait_settings(),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    })
}

pub(crate) async fn apply_forward_proxy_settings_without_bootstrap(
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

pub(crate) async fn seed_pool_routing_api_key(state: &Arc<AppState>, api_key: &str) {
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

pub(crate) fn test_required_group_name() -> &'static str {
    "test-direct-group"
}

pub(crate) fn test_required_group_bound_proxy_keys() -> Vec<String> {
    vec![FORWARD_PROXY_DIRECT_KEY.to_string()]
}

pub(crate) async fn ensure_test_group_binding(
    pool: &SqlitePool,
    group_name: &str,
    note: Option<&str>,
) {
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

pub(crate) async fn insert_test_pool_api_key_account(
    state: &Arc<AppState>,
    display_name: &str,
    api_key: &str,
) -> i64 {
    insert_test_pool_api_key_account_with_options(state, display_name, api_key, None, None, None)
        .await
}

pub(crate) async fn insert_test_pool_api_key_account_with_options(
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
        "upstreamBaseUrl": upstream_base_url,
    }))
    .expect("deserialize api key account request");
    let Json(detail) =
        create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("insert test pool upstream account");
    let _ = detail;
    let account_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM pool_upstream_accounts WHERE display_name = ?1",
    )
    .bind(display_name)
    .fetch_one(&state.pool)
    .await
    .expect("load inserted test pool upstream account id");
    restore_test_legacy_api_key_group(
        &state.pool,
        account_id,
        normalized_group_name,
        is_mother.unwrap_or(false),
    )
    .await;
    account_id
}

pub(crate) async fn restore_test_legacy_api_key_group(
    pool: &SqlitePool,
    account_id: i64,
    group_name: &str,
    is_mother: bool,
) {
    ensure_test_group_binding(pool, group_name, None).await;
    if is_mother {
        sqlx::query(
            "UPDATE pool_upstream_accounts SET is_mother = 0 WHERE group_name = ?1 AND id != ?2",
        )
        .bind(group_name)
        .bind(account_id)
        .execute(pool)
        .await
        .expect("clear existing legacy api-key mother account");
    }
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2, is_mother = ?3 WHERE id = ?1")
        .bind(account_id)
        .bind(group_name)
        .bind(if is_mother { 1 } else { 0 })
        .execute(pool)
        .await
        .expect("restore legacy api-key group state");
}

pub(crate) async fn set_test_account_group_name(
    pool: &SqlitePool,
    account_id: i64,
    group_name: Option<&str>,
) {
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind(group_name)
        .execute(pool)
        .await
        .expect("set test account group name");
}

pub(crate) async fn create_test_fast_mode_tag(
    state: &Arc<AppState>,
    name: &str,
    fast_mode_rewrite_mode: &str,
    priority_tier: &str,
) -> i64 {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, system_key, protected, allow_cut_out, allow_cut_in,
            priority_tier, fast_mode_rewrite_mode, concurrency_limit, upstream_429_retry_enabled,
            upstream_429_max_retries, available_models_json, created_at, updated_at
        ) VALUES (?1, ?2, 0, 1, 1, ?3, ?4, 0, 0, 0, '[]', ?5, ?5)
        RETURNING id
        "#,
    )
    .bind(name)
    .bind(None::<String>)
    .bind(priority_tier)
    .bind(fast_mode_rewrite_mode)
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert fast mode system tag")
}

pub(crate) async fn create_test_tagged_pool_api_key_account(
    state: &Arc<AppState>,
    display_name: &str,
    api_key: &str,
    upstream_base_url: &str,
    tag_ids: &[i64],
) -> i64 {
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": display_name,
        "upstreamBaseUrl": upstream_base_url,
        "apiKey": api_key,
    }))
    .expect("deserialize tagged api-key account payload");
    let Json(_) = create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect("create tagged pool account");
    let account_id: i64 =
        sqlx::query_scalar("SELECT id FROM pool_upstream_accounts WHERE display_name = ?1")
            .bind(display_name)
            .fetch_one(&state.pool)
            .await
            .expect("load tagged pool account id");
    restore_test_legacy_api_key_group(&state.pool, account_id, test_required_group_name(), false)
        .await;
    if !tag_ids.is_empty() {
        let now_iso = format_utc_iso(Utc::now());
        for tag_id in tag_ids {
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
            .expect("attach tagged pool account system tag");
        }
    }
    account_id
}

pub(crate) async fn insert_test_pool_limit_sample(
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
pub(crate) async fn insert_test_pool_limit_sample_with_windows(
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

pub(crate) async fn reserve_test_pool_routing_account(
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
        routing_control_fingerprint: None,
        sticky_affinity_generation: None,
        routing_selection_audit: None,
        priority_handoff_permit: None,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        single_account_rotation_enabled: false,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        oauth_request_compression_algorithm: RequestCompressionAlgorithm::Zstd,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
    };
    reserve_pool_routing_account(state.as_ref(), reservation_key, &account);
}

pub(crate) async fn set_test_account_local_limits(
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

pub(crate) async fn set_test_account_status(pool: &SqlitePool, account_id: i64, status: &str) {
    sqlx::query("UPDATE pool_upstream_accounts SET status = ?1 WHERE id = ?2")
        .bind(status)
        .bind(account_id)
        .execute(pool)
        .await
        .expect("set test pool account status");
}

pub(crate) async fn clear_test_account_credentials(pool: &SqlitePool, account_id: i64) {
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

pub(crate) async fn set_test_account_rate_limited_cooldown(
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

pub(crate) async fn set_test_account_generic_route_cooldown(
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

pub(crate) async fn set_test_account_degraded_route_state(
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

pub(crate) async fn upsert_test_sticky_route_at(
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

pub(crate) fn format_test_recent_active_timestamp(now: DateTime<Utc>) -> String {
    format_utc_iso(now - ChronoDuration::minutes(4))
}

pub(crate) fn format_test_stale_active_timestamp(now: DateTime<Utc>) -> String {
    format_utc_iso(now - ChronoDuration::minutes(6))
}

pub(crate) async fn insert_test_pool_oauth_account(
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

pub(crate) async fn insert_test_pool_oauth_account_with_chatgpt_account_id(
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
    let account_id = sqlx::query_scalar(
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
    .expect("insert test oauth account");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_refresh_state (
            account_id, operation, state, last_attempt_at, last_success_at,
            next_attempt_at, source, error_code, error_message, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, NULL, NULL, ?4)
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS)
    .bind(UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED)
    .bind(&now_iso)
    .bind(&token_expires_at)
    .bind("test_fixture")
    .execute(&state.pool)
    .await
    .expect("seed test oauth credential refresh state");
    account_id
}

pub(crate) async fn set_test_account_credential_refresh_due(pool: &SqlitePool, account_id: i64) {
    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    sqlx::query(
        r#"
        UPDATE pool_upstream_account_refresh_state
        SET last_attempt_at = ?2,
            last_success_at = NULL,
            next_attempt_at = ?2,
            updated_at = ?2
        WHERE account_id = ?1 AND operation = ?3
        "#,
    )
    .bind(account_id)
    .bind(&now_iso)
    .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS)
    .execute(pool)
    .await
    .expect("mark test oauth credential refresh due");
}

fn encrypt_test_oauth_credentials(access_token: &str) -> String {
    let key = Sha256::digest(b"test-upstream-account-secret");
    let cipher = Aes256Gcm::new(&key);
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
    let nonce_value = aes_gcm::Nonce::from(nonce);
    let ciphertext = cipher
        .encrypt(&nonce_value, plaintext.as_ref())
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
    let alpha_id = insert_test_pool_oauth_account(&state, "Alpha", "upstream-alpha").await;
    set_test_account_group_name(&state.pool, alpha_id, Some("prod")).await;
    let beta_id = insert_test_pool_oauth_account(&state, "Beta", "upstream-beta").await;
    set_test_account_group_name(&state.pool, beta_id, Some("production")).await;
    let gamma_id = insert_test_pool_oauth_account(&state, "Gamma", "upstream-gamma").await;
    let delta_id = insert_test_pool_oauth_account(&state, "Delta", "upstream-delta").await;
    set_test_account_group_name(&state.pool, delta_id, Some("Prod")).await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
        .bind(gamma_id)
        .execute(&state.pool)
        .await
        .expect("clear gamma group to simulate legacy ungrouped account");
    let now_iso = format_utc_iso(Utc::now());

    let vip_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 1, 1, ?2, ?2)
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
            name,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 1, 1, ?2, ?2)
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
        (delta_id, vip_tag_id),
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
            kind: Some("oauth_codex".to_string()),
            group_exact: Vec::new(),
            group_search: Some("prod".to_string()),
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
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

    let Json(exact_group_filtered) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            kind: Some("oauth_codex".to_string()),
            group_exact: vec!["Prod".to_string()],
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
            tag_ids: vec![vip_tag_id],
        }),
    )
    .await
    .expect("list exact-group filtered upstream accounts");
    let exact_group_filtered_json =
        serde_json::to_value(exact_group_filtered).expect("serialize exact-group filtered roster");
    let exact_group_filtered_names = exact_group_filtered_json["items"]
        .as_array()
        .expect("exact-group filtered items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(exact_group_filtered_names, vec!["Delta"]);

    let Json(multi_group_filtered) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            kind: Some("oauth_codex".to_string()),
            group_exact: vec!["Prod".to_string(), "prod".to_string()],
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
            tag_ids: vec![vip_tag_id],
        }),
    )
    .await
    .expect("list multi exact-group filtered upstream accounts");
    let multi_group_filtered_json =
        serde_json::to_value(multi_group_filtered).expect("serialize multi exact-group roster");
    let multi_group_filtered_names = multi_group_filtered_json["items"]
        .as_array()
        .expect("multi exact-group filtered items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(multi_group_filtered_names, vec!["Delta", "Alpha"]);

    let Json(ungrouped_filtered) = list_upstream_accounts(
        State(state),
        Query(ListUpstreamAccountsQuery {
            kind: Some("oauth_codex".to_string()),
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: Some(true),
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
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
async fn upstream_account_schema_normalizes_blank_group_names_to_default_group() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_oauth_account(&state, "Legacy Blank Group", "oauth-legacy-blank-group")
            .await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = '   ' WHERE id = ?1")
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("write legacy blank group");

    ensure_upstream_accounts_schema(&state.pool)
        .await
        .expect("normalize upstream account schema");

    let group_name: Option<String> =
        sqlx::query_scalar("SELECT group_name FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load normalized group");
    assert_eq!(
        group_name.as_deref(),
        Some(DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME)
    );

    let Json(ungrouped_filtered) = list_upstream_accounts(
        State(state),
        Query(ListUpstreamAccountsQuery {
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: Some(true),
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list legacy ungrouped compatibility after normalization");
    let ungrouped_filtered_json =
        serde_json::to_value(ungrouped_filtered).expect("serialize normalized ungrouped roster");
    let ungrouped_filtered_names = ungrouped_filtered_json["items"]
        .as_array()
        .expect("normalized ungrouped items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(ungrouped_filtered_names, vec!["Legacy Blank Group"]);
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
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: Some("active".to_string()),
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(2),
            page_size: Some(20),
            include_all: None,
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
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: Some("disabled".to_string()),
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(1),
            page_size: Some(20),
            include_all: None,
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
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: vec!["rate_limited".to_string()],
            enable_status: vec!["enabled".to_string()],
            health_status: vec!["normal".to_string()],
            page: Some(1),
            page_size: Some(20),
            include_all: None,
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
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(1),
            page_size: Some(20),
            include_all: None,
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
    for (sticky_key, model_key) in [
        ("sticky-recent-1", "gpt-5.4"),
        ("sticky-model-only", "gpt-5.1-codex-max"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_model_routes (
                sticky_key, model_key, account_id, created_at, updated_at, last_seen_at
            ) VALUES (?1, ?2, ?3, ?4, ?4, ?5)
            "#,
        )
        .bind(sticky_key)
        .bind(model_key)
        .bind(account_id)
        .bind(&created_at)
        .bind(&second_recent_seen_at)
        .execute(&state.pool)
        .await
        .expect("insert model sticky route");
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
        Some(3)
    );

    let Json(detail_response) = get_upstream_account(
        State(state),
        axum::extract::Path(account_id),
        axum::extract::Query(GetUpstreamAccountQuery::default()),
    )
    .await
    .expect("load upstream account detail");
    let detail_json =
        serde_json::to_value(detail_response).expect("serialize upstream account detail");
    assert_eq!(
        detail_json
            .get("activeConversationCount")
            .and_then(serde_json::Value::as_i64),
        Some(3)
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
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(1),
            page_size: Some(20),
            include_all: None,
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
async fn get_upstream_account_omits_recent_actions_by_default_and_loads_them_on_demand() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Recent actions gated", "upstream-gated").await;

    let attempt_id = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            upstream_account_id,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status,
            request_model
        )
        VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, 0, ?6, ?7)
        "#,
    )
    .bind("recent-action-model-impact")
    .bind("2026-06-25 11:30:00")
    .bind("/v1/responses")
    .bind("pool")
    .bind(account_id)
    .bind("failed")
    .bind("gpt-5.6-terra")
    .execute(&state.pool)
    .await
    .expect("insert upstream request attempt")
    .last_insert_rowid();

    let event_id = sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id,
            occurred_at,
            action,
            source,
            account_display_name,
            result,
            result_description,
            reason_code,
            attempt_id,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(account_id)
    .bind("2026-06-25 11:30:00")
    .bind("account_updated")
    .bind("manual")
    .bind("Recent actions gated")
    .bind("success")
    .bind("settings saved")
    .bind("account_updated")
    .bind(attempt_id)
    .bind("2026-06-25 11:30:00")
    .execute(&state.pool)
    .await
    .expect("insert upstream account event")
    .last_insert_rowid();

    for (occurred_at, stored_model, request_model) in [
        ("2026-06-25 11:29:00", "gpt-5.4-response", "gpt-5.4-request"),
        (
            "2026-06-25 11:31:00",
            "gpt-5.7-response",
            "gpt-5.7-unrelated",
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                model,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind("reused-invocation-model-impact")
        .bind(occurred_at)
        .bind("proxy")
        .bind("failed")
        .bind(stored_model)
        .bind(json!({ "requestModel": request_model }).to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert invocation model fallback fixture");
    }

    let invocation_event_id = sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id,
            occurred_at,
            action,
            source,
            account_display_name,
            result,
            result_description,
            reason_code,
            invoke_id,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(account_id)
    .bind("2026-06-25T03:29:20Z")
    .bind("route_cooldown_started")
    .bind("call")
    .bind("Recent actions gated")
    .bind("failed")
    .bind("upstream unavailable")
    .bind("upstream_http_5xx")
    .bind("reused-invocation-model-impact")
    .bind("2026-06-25T03:29:20Z")
    .execute(&state.pool)
    .await
    .expect("insert invocation-linked upstream account event")
    .last_insert_rowid();

    let Json(default_detail) = get_upstream_account(
        State(state.clone()),
        axum::extract::Path(account_id),
        axum::extract::Query(GetUpstreamAccountQuery::default()),
    )
    .await
    .expect("load default upstream account detail");
    let default_detail_json =
        serde_json::to_value(default_detail).expect("serialize default upstream account detail");
    assert!(
        default_detail_json["recentActions"]
            .as_array()
            .is_some_and(|items| items.is_empty()),
        "default detail fetch should skip recent actions for overview first paint"
    );

    let Json(detail_with_recent_actions) = get_upstream_account(
        State(state.clone()),
        axum::extract::Path(account_id),
        axum::extract::Query(GetUpstreamAccountQuery {
            include_recent_actions: Some(true),
        }),
    )
    .await
    .expect("load upstream account detail with recent actions");
    let detail_with_recent_actions_json = serde_json::to_value(detail_with_recent_actions)
        .expect("serialize upstream account detail with recent actions");
    assert!(
        detail_with_recent_actions_json["recentActions"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default()
            >= 1
    );
    assert_eq!(
        detail_with_recent_actions_json["recentActions"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("action").and_then(serde_json::Value::as_str)
                        == Some("account_updated")
                })
            })
            .and_then(|item| item.get("action"))
            .and_then(serde_json::Value::as_str),
        Some("account_updated")
    );
    assert_eq!(
        detail_with_recent_actions_json["recentActions"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("action").and_then(serde_json::Value::as_str)
                        == Some("account_updated")
                })
            })
            .and_then(|item| item.get("model"))
            .and_then(serde_json::Value::as_str),
        Some("gpt-5.6-terra"),
        "account events must expose the model already captured by their linked attempt"
    );
    assert_eq!(
        detail_with_recent_actions_json["recentActions"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(serde_json::Value::as_i64) == Some(invocation_event_id)
                })
            })
            .and_then(|item| item.get("model"))
            .and_then(serde_json::Value::as_str),
        Some("gpt-5.4-request"),
        "invocation fallback must use the matching request model, not a response or newer call"
    );

    let Json(event_list) = list_upstream_account_action_events(
        State(state),
        axum::extract::Query(ListUpstreamAccountActionEventsQuery {
            account: Some("Recent actions gated".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("load global upstream account event list");
    let event_list_json =
        serde_json::to_value(event_list).expect("serialize upstream account event list");
    assert_eq!(
        event_list_json["items"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(serde_json::Value::as_i64) == Some(event_id)
                })
            })
            .and_then(|item| item.get("model"))
            .and_then(serde_json::Value::as_str),
        Some("gpt-5.6-terra"),
        "global account event lists must expose the linked attempt model"
    );
    assert_eq!(
        event_list_json["items"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(serde_json::Value::as_i64) == Some(invocation_event_id)
                })
            })
            .and_then(|item| item.get("model"))
            .and_then(serde_json::Value::as_str),
        Some("gpt-5.4-request"),
        "global event fallback must correlate duplicate invocation ids by occurrence"
    );
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
