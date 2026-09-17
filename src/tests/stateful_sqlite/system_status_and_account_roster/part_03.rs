async fn assert_stateful_template_schema_parity(
    template_path: &Path,
    template_url: &str,
    fresh_url: &str,
) {
    let template_pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(4)
        .connect(template_url)
        .await
        .expect("connect template schema parity sqlite");
    restore_stateful_schema_template_from_path(&template_pool, template_path)
        .await
        .expect("restore template schema parity sqlite");
    let fresh_pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(4)
        .connect(fresh_url)
        .await
        .expect("connect fresh schema parity sqlite");
    ensure_schema(&fresh_pool)
        .await
        .expect("initialize fresh schema parity sqlite");

    assert_eq!(
        schema_object_signature(&template_pool).await,
        schema_object_signature(&fresh_pool).await,
        "template schema must match fresh ensure_schema output"
    );
    assert_eq!(
        schema_default_data_signature(&template_pool).await,
        schema_default_data_signature(&fresh_pool).await,
        "template default data must match fresh ensure_schema output"
    );

    template_pool.close().await;
    fresh_pool.close().await;
}

async fn assert_stateful_template_pool_isolation(
    template_path: &Path,
    template_url: &str,
    isolated_url: &str,
) {
    let template_pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(4)
        .connect(template_url)
        .await
        .expect("connect template pool isolation sqlite");
    restore_stateful_schema_template_from_path(&template_pool, template_path)
        .await
        .expect("restore template pool isolation sqlite");

    let mut first_connection = template_pool
        .acquire()
        .await
        .expect("acquire first template connection");
    sqlx::query("CREATE TABLE template_pool_visibility (value INTEGER NOT NULL)")
        .execute(&mut *first_connection)
        .await
        .expect("create pooled visibility table");
    sqlx::query("INSERT INTO template_pool_visibility (value) VALUES (7)")
        .execute(&mut *first_connection)
        .await
        .expect("write through first template connection");

    let mut second_connection = template_pool
        .acquire()
        .await
        .expect("acquire second template connection");
    let visible_value: i64 = sqlx::query_scalar("SELECT value FROM template_pool_visibility")
        .fetch_one(&mut *second_connection)
        .await
        .expect("template write must be visible to a second pooled connection");
    assert_eq!(visible_value, 7);
    sqlx::query("INSERT INTO template_pool_visibility (value) VALUES (11)")
        .execute(&mut *second_connection)
        .await
        .expect("write through second template connection");
    let visible_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM template_pool_visibility")
        .fetch_one(&mut *first_connection)
        .await
        .expect("second pooled connection write must be visible to the first connection");
    assert_eq!(visible_count, 2);

    let isolated_pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(4)
        .connect(isolated_url)
        .await
        .expect("connect isolated template sqlite");
    restore_stateful_schema_template_from_path(&isolated_pool, template_path)
        .await
        .expect("restore isolated template sqlite");
    let isolated_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'template_pool_visibility'",
    )
    .fetch_one(&isolated_pool)
    .await
    .expect("query isolated template schema");
    assert_eq!(
        isolated_count, 0,
        "template pools must not share test writes"
    );

    drop(second_connection);
    drop(first_connection);
    template_pool.close().await;
    isolated_pool.close().await;
}

#[tokio::test]
pub(crate) async fn stateful_schema_template_matches_fresh_schema_and_keeps_pooled_databases_isolated()
 {
    let temp_dir = make_temp_test_dir("stateful-schema-template-parity");
    let template_path = temp_dir.join("current-schema.db");
    write_stateful_schema_template(&template_path)
        .await
        .expect("write stateful schema template");

    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let template_url =
        format!("sqlite:file:codex-vibe-monitor-template-parity-{db_id}?mode=memory&cache=shared");
    let fresh_url =
        format!("sqlite:file:codex-vibe-monitor-fresh-parity-{db_id}?mode=memory&cache=shared");
    let isolated_url = format!(
        "sqlite:file:codex-vibe-monitor-template-isolated-{db_id}?mode=memory&cache=shared"
    );
    assert_stateful_template_schema_parity(&template_path, &template_url, &fresh_url).await;
    assert_stateful_template_pool_isolation(&template_path, &template_url, &isolated_url).await;
    fs::remove_dir_all(temp_dir).expect("remove stateful schema template parity directory");
}

pub(crate) async fn enable_encrypted_session_owner_routing_for_test(state: &Arc<AppState>) {
    let mut settings = state.proxy_model_settings.write().await;
    settings.encrypted_session_owner_routing_enabled = true;
}

pub(crate) fn clone_state_with_upstream_accounts(
    state: &Arc<AppState>,
    upstream_accounts: Arc<UpstreamAccountsRuntime>,
) -> Arc<AppState> {
    Arc::new(AppState {
        config: state.config.clone(),
        sqlite_batch_writer: state.sqlite_batch_writer.clone(),
        pool_account_selection_runtime: state.pool_account_selection_runtime.clone(),
        proxy_runtime_invocations: state.proxy_runtime_invocations.clone(),
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
        dashboard_network_speed_cache: state.dashboard_network_speed_cache.clone(),
        startup_ready: state.startup_ready.clone(),
        shutdown: state.shutdown.clone(),
        semaphore: state.semaphore.clone(),
        proxy_request_in_flight: state.proxy_request_in_flight.clone(),
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

pub(crate) fn clone_state_with_retry_delay_overrides(
    state: &Arc<AppState>,
    pool_group_delay: Option<Duration>,
    fallback_delay: Option<Duration>,
) -> Arc<AppState> {
    Arc::new(AppState {
        config: state.config.clone(),
        sqlite_batch_writer: state.sqlite_batch_writer.clone(),
        pool_account_selection_runtime: state.pool_account_selection_runtime.clone(),
        proxy_runtime_invocations: state.proxy_runtime_invocations.clone(),
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
        dashboard_network_speed_cache: state.dashboard_network_speed_cache.clone(),
        startup_ready: state.startup_ready.clone(),
        shutdown: state.shutdown.clone(),
        semaphore: state.semaphore.clone(),
        proxy_request_in_flight: state.proxy_request_in_flight.clone(),
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

    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let pricing_catalog = load_pricing_catalog(&pool)
        .await
        .expect("pricing catalog should initialize");

    Arc::new(AppState {
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
        startup_ready: Arc::new(AtomicBool::new(startup_ready)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
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
        TestPoolLimitSample {
            account_id,
            plan_type: None,
            primary_used_percent,
            primary_window_minutes: Some(300),
            primary_resets_at: None,
            secondary_used_percent,
            secondary_window_minutes: Some(300),
            secondary_resets_at: None,
        },
    )
    .await;
}

pub(crate) struct TestPoolLimitSample<'a> {
    pub(crate) account_id: i64,
    pub(crate) plan_type: Option<&'a str>,
    pub(crate) primary_used_percent: Option<f64>,
    pub(crate) primary_window_minutes: Option<i64>,
    pub(crate) primary_resets_at: Option<&'a str>,
    pub(crate) secondary_used_percent: Option<f64>,
    pub(crate) secondary_window_minutes: Option<i64>,
    pub(crate) secondary_resets_at: Option<&'a str>,
}

pub(crate) async fn insert_test_pool_limit_sample_with_windows(
    state: &Arc<AppState>,
    sample: TestPoolLimitSample<'_>,
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
    .bind(sample.account_id)
    .bind(&captured_at)
    .bind(sample.plan_type)
    .bind(sample.primary_used_percent)
    .bind(sample.primary_window_minutes)
    .bind(sample.primary_resets_at)
    .bind(sample.secondary_used_percent)
    .bind(sample.secondary_window_minutes)
    .bind(sample.secondary_resets_at)
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

pub(crate) async fn set_test_account_route_cooldown(
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

use super::*;
