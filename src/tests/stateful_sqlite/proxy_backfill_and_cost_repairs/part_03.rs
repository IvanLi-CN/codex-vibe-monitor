async fn seed_runtime_prompt_cache_snapshot(state: &AppState, occurred_at: &str) {
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        prompt_cache_key: Some("pck-follow-up-refresh".to_string()),
        requested_service_tier: Some("priority".to_string()),
        reasoning_effort: Some("high".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, upstream_base_url, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(17_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("pool-account-17")
    .bind("https://api-keys.vendor.invalid/")
    .bind("active")
    .bind(1_i64)
    .bind(occurred_at)
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("seed upstream account for runtime snapshot activity touch");
    let running_record = build_running_proxy_capture_record(
        "follow-up-refresh-running",
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-follow-up-refresh"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-01"),
        Some(3),
        Some(2),
        None,
        Some("gzip"),
        22.0,
        4.0,
        330.0,
        120.0,
    );

    persist_and_broadcast_proxy_capture_runtime_snapshot(state, running_record)
        .await
        .expect("runtime snapshot should persist");
}

async fn assert_runtime_snapshot_remains_memory_only(state: &AppState) {
    let cache_generation_before_flush = {
        let cache = state.prompt_cache_conversation_cache.lock().await;
        cache.generation
    };
    assert_eq!(
        cache_generation_before_flush, 0,
        "runtime snapshots should not invalidate prompt-cache conversations"
    );

    let runtime_snapshot_follow_up_handles = state
        .proxy_summary_quota_broadcast_handle
        .lock()
        .await
        .len();
    assert_eq!(
        runtime_snapshot_follow_up_handles, 0,
        "runtime snapshots should not schedule the summary/quota follow-up worker"
    );
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let running_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND status = 'running'",
    )
    .bind("follow-up-refresh-running")
    .fetch_one(&state.pool)
    .await
    .expect("load running placeholder after runtime snapshot flush");
    assert_eq!(
        running_rows, 0,
        "runtime snapshots should stay memory-only after sqlite batch flush"
    );

    let prompt_cache_requests: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_rollup_hourly WHERE prompt_cache_key = ?1",
    )
    .bind("pck-follow-up-refresh")
    .fetch_one(&state.pool)
    .await
    .expect("load prompt cache rollup count after runtime snapshot");
    assert_eq!(
        prompt_cache_requests, 0,
        "runtime snapshots should not advance hourly rollups before terminal persistence"
    );

    let cache_generation_after_flush = {
        let cache = state.prompt_cache_conversation_cache.lock().await;
        cache.generation
    };
    assert_eq!(
        cache_generation_after_flush, 0,
        "memory-only running snapshots should not invalidate prompt-cache conversations"
    );

    let account_last_activity_at: Option<String> =
        sqlx::query_scalar("SELECT last_activity_at FROM pool_upstream_accounts WHERE id = ?1")
            .bind(17_i64)
            .fetch_one(&state.pool)
            .await
            .expect("load upstream account last activity after runtime snapshot flush");
    assert_eq!(
        account_last_activity_at.as_deref(),
        None,
        "runtime snapshots should not touch upstream account activity before terminal persistence"
    );
}

async fn assert_runtime_snapshot_overlay(state: Arc<AppState>) {
    let Json(conversations) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: None,
            activity_hours: None,
            activity_minutes: Some(5),
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: Some("compact".to_string()),
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("working conversations should stay readable after runtime snapshot");
    assert_eq!(
        conversations.conversations.len(),
        1,
        "memory-only running snapshots should be visible through HTTP overlay before terminal persistence"
    );
    let runtime_conversation = conversations
        .conversations
        .iter()
        .find(|conversation| conversation.prompt_cache_key == "pck-follow-up-refresh")
        .expect("running prompt-cache conversation should come from runtime overlay");
    assert!(
        runtime_conversation.last_in_flight_at.is_some(),
        "runtime overlay should expose the in-flight anchor"
    );
    assert_eq!(
        runtime_conversation
            .recent_invocations
            .first()
            .map(|invocation| invocation.status.as_str()),
        Some("running"),
        "runtime overlay should hydrate a running recent preview"
    );
}

async fn persist_terminal_prompt_cache_snapshot(state: &AppState, occurred_at: &str) {
    let mut terminal_record = test_proxy_capture_record("follow-up-refresh-running", occurred_at);
    terminal_record.payload = Some(
        "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":true,\"requesterIp\":\"198.51.100.88\",\"promptCacheKey\":\"pck-follow-up-refresh\",\"routeMode\":\"pool\",\"upstreamAccountId\":17,\"upstreamAccountName\":\"pool-account-17\",\"responseContentEncoding\":\"gzip\",\"requestedServiceTier\":\"priority\",\"reasoningEffort\":\"high\",\"proxyDisplayName\":\"jp-relay-01\"}"
            .to_string(),
    );

    persist_and_broadcast_proxy_capture(state, Instant::now(), terminal_record)
        .await
        .expect("terminal proxy capture should persist");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let terminal_prompt_cache_requests: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_rollup_hourly WHERE prompt_cache_key = ?1",
    )
    .bind("pck-follow-up-refresh")
    .fetch_one(&state.pool)
    .await
    .expect("load prompt cache rollup count after terminal snapshot");
    assert_eq!(
        terminal_prompt_cache_requests, 1,
        "terminal proxy captures should keep prompt-cache rollups queryable"
    );
    let terminal_follow_up_handles = state
        .proxy_summary_quota_broadcast_handle
        .lock()
        .await
        .len();
    assert!(
        terminal_follow_up_handles == 0,
        "terminal proxy captures without subscribers should not schedule the summary/quota follow-up worker"
    );
}

#[tokio::test]
pub(crate) async fn runtime_snapshot_batches_prompt_cache_rollups_without_background_follow_up() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    seed_runtime_prompt_cache_snapshot(&state, &occurred_at).await;
    assert_runtime_snapshot_remains_memory_only(&state).await;
    assert_runtime_snapshot_overlay(state.clone()).await;
    persist_terminal_prompt_cache_snapshot(&state, &occurred_at).await;
}

#[tokio::test]
pub(crate) async fn run_backfill_with_retry_succeeds_after_lock_release() {
    let temp_dir = make_temp_test_dir("proxy-backfill-retry-success");
    let db_path = temp_dir.join("lock-success.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload(&response_path);
    insert_proxy_backfill_row(&pool, "proxy-lock-retry-success", &response_path).await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let started = Instant::now();
    let pool_for_task = pool.clone();
    let backfill_task =
        tokio::spawn(async move { run_backfill_with_retry(&pool_for_task, None).await });

    tokio::time::sleep(Duration::from_millis(120)).await;
    sqlx::query("COMMIT")
        .execute(&mut lock_conn)
        .await
        .expect("release sqlite write lock");

    let summary = backfill_task
        .await
        .expect("join backfill task")
        .expect("backfill should succeed after retry");
    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "expected retry delay to be applied"
    );
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-lock-retry-success")
            .fetch_one(&pool)
            .await
            .expect("query backfilled row");
    assert_eq!(total_tokens, Some(110));

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn run_backfill_with_retry_fails_when_lock_persists() {
    let temp_dir = make_temp_test_dir("proxy-backfill-retry-fail");
    let db_path = temp_dir.join("lock-fail.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload(&response_path);
    insert_proxy_backfill_row(&pool, "proxy-lock-retry-fail", &response_path).await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let started = Instant::now();
    let pool_for_task = pool.clone();
    let backfill_task =
        tokio::spawn(async move { run_backfill_with_retry(&pool_for_task, None).await });
    let err = backfill_task
        .await
        .expect("join backfill task")
        .expect_err("backfill should fail after lock retry exhaustion");
    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "expected retry delay before final failure"
    );
    assert!(
        err.to_string().contains("failed after 2/2 attempt(s)"),
        "expected retry exhaustion context in error: {err:?}"
    );
    assert!(is_sqlite_lock_error(&err));

    let total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-lock-retry-fail")
            .fetch_one(&pool)
            .await
            .expect("query locked row");
    assert_eq!(total_tokens, None);

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("rollback lock holder");
    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn run_backfill_with_retry_does_not_retry_non_lock_errors() {
    let temp_dir = make_temp_test_dir("proxy-backfill-retry-non-lock");
    let db_path = temp_dir.join("non-lock.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");

    // Intentionally skip schema initialization to force a deterministic non-lock error.
    let err = run_backfill_with_retry(&pool, None)
        .await
        .expect_err("backfill should fail immediately on non-lock errors");
    assert!(
        err.to_string().contains("failed after 1/2 attempt(s)"),
        "expected single-attempt context in error: {err:?}"
    );
    assert!(!is_sqlite_lock_error(&err));
    assert!(err.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("no such table")
    }));

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn run_cost_backfill_with_retry_succeeds_after_lock_release() {
    let temp_dir = make_temp_test_dir("proxy-cost-backfill-retry-success");
    let db_path = temp_dir.join("lock-success.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-lock-retry-success",
        Some("gpt-5.2-2025-12-11"),
        Some(2_000),
        Some(1_000),
    )
    .await;
    let catalog = PricingCatalog {
        version: "unit-cost-retry".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let started = Instant::now();
    let pool_for_task = pool.clone();
    let catalog_for_task = catalog.clone();
    let backfill_task = tokio::spawn(async move {
        run_cost_backfill_with_retry(&pool_for_task, &catalog_for_task).await
    });

    tokio::time::sleep(Duration::from_millis(120)).await;
    sqlx::query("COMMIT")
        .execute(&mut lock_conn)
        .await
        .expect("release sqlite write lock");

    let summary = backfill_task
        .await
        .expect("join cost backfill task")
        .expect("cost backfill should succeed after retry");
    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "expected retry delay to be applied"
    );
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let cost: Option<f64> =
        sqlx::query_scalar("SELECT cost FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-cost-lock-retry-success")
            .fetch_one(&pool)
            .await
            .expect("query backfilled cost row");
    assert!(cost.is_some());

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn run_cost_backfill_with_retry_does_not_retry_non_lock_errors() {
    let temp_dir = make_temp_test_dir("proxy-cost-backfill-retry-non-lock");
    let db_path = temp_dir.join("non-lock.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    let catalog = PricingCatalog {
        version: "unit-cost-retry".to_string(),
        models: HashMap::new(),
    };

    // Intentionally skip schema initialization to force a deterministic non-lock error.
    let err = run_cost_backfill_with_retry(&pool, &catalog)
        .await
        .expect_err("cost backfill should fail immediately on non-lock errors");
    assert!(
        err.to_string().contains("failed after 1/2 attempt(s)"),
        "expected single-attempt context in error: {err:?}"
    );
    assert!(!is_sqlite_lock_error(&err));
    assert!(err.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("no such table")
    }));

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn quota_latest_returns_degraded_when_empty() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let config = test_config();
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
        pool_group_429_retry_delay_override: None,
        fallback_proxy_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    });

    let Json(snapshot) = latest_quota_snapshot(State(state))
        .await
        .expect("route should succeed");

    assert!(!snapshot.is_active);
    assert_eq!(snapshot.total_requests, 0);
    assert_eq!(snapshot.total_cost, 0.0);
}

#[tokio::test]
pub(crate) async fn quota_latest_returns_seeded_historical_snapshot() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let captured_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &captured_at).await;

    let Json(snapshot) = latest_quota_snapshot(State(state))
        .await
        .expect("route should return seeded quota snapshot");

    assert_eq!(snapshot.captured_at, captured_at);
    let snapshot_json = serde_json::to_value(&snapshot).expect("serialize quota snapshot");
    assert!(
        snapshot_json["capturedAt"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z')),
        "serialized quota snapshot should emit UTC ISO timestamps"
    );
    assert!(snapshot.is_active);
    assert_eq!(snapshot.total_requests, 9);
    assert_f64_close(snapshot.total_cost, 10.0);
}

pub(crate) async fn insert_timeseries_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    status: &str,
    t_upstream_ttfb_ms: Option<f64>,
) {
    insert_timeseries_invocation_with_stages(
        pool,
        TimeseriesInvocationFixture {
            invoke_id,
            occurred_at,
            status,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms,
        },
    )
    .await;
}

pub(crate) struct TimeseriesInvocationFixture<'a> {
    pub(crate) invoke_id: &'a str,
    pub(crate) occurred_at: &'a str,
    pub(crate) status: &'a str,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
}

pub(crate) async fn insert_timeseries_invocation_with_stages(
    pool: &SqlitePool,
    fixture: TimeseriesInvocationFixture<'_>,
) {
    let TimeseriesInvocationFixture {
        invoke_id,
        occurred_at,
        status,
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        t_upstream_ttfb_ms,
    } = fixture;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(status)
    .bind(10_i64)
    .bind(0.01_f64)
    .bind(t_req_read_ms)
    .bind(t_req_parse_ms)
    .bind(t_upstream_connect_ms)
    .bind(t_upstream_ttfb_ms)
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert timeseries invocation");
}

pub(crate) async fn insert_parallel_work_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
    prompt_cache_key: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(invoke_id)
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10_i64)
    .bind(0.01_f64)
    .bind(json!({ "promptCacheKey": prompt_cache_key }).to_string())
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert parallel-work invocation");
}

pub(crate) async fn insert_invocation_rollup(
    pool: &SqlitePool,
    fixture: InvocationRollupFixture<'_>,
) {
    insert_invocation_rollup_with_latency_samples(pool, fixture).await;
}

pub(crate) fn encode_histogram_from_samples(samples: &[f64]) -> String {
    let mut histogram = empty_approx_histogram();
    for sample in samples {
        add_approx_histogram_sample(&mut histogram, *sample);
    }
    encode_approx_histogram(&histogram).expect("encode approximate histogram from samples")
}

pub(crate) fn sum_f64_samples(samples: &[f64]) -> f64 {
    samples.iter().copied().sum::<f64>()
}

pub(crate) fn max_f64_sample(samples: &[f64]) -> f64 {
    samples
        .iter()
        .copied()
        .fold(0.0_f64, |current, value| current.max(value))
}

pub(crate) struct InvocationRollupFixture<'a> {
    pub(crate) stats_date: NaiveDate,
    pub(crate) source: &'a str,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_byte_samples: &'a [f64],
    pub(crate) first_response_byte_total_samples: &'a [f64],
}

impl<'a> InvocationRollupFixture<'a> {
    pub(crate) fn without_latency(
        stats_date: NaiveDate,
        source: &'a str,
        total_count: i64,
        success_count: i64,
        failure_count: i64,
        total_tokens: i64,
        total_cost: f64,
    ) -> Self {
        Self {
            stats_date,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        }
    }
}

pub(crate) async fn insert_invocation_rollup_with_latency_samples(
    pool: &SqlitePool,
    fixture: InvocationRollupFixture<'_>,
) {
    let InvocationRollupFixture {
        stats_date,
        source,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        total_cost,
        first_byte_samples,
        first_response_byte_total_samples,
    } = fixture;
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_daily (
            stats_date,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
        "#,
    )
    .bind(stats_date.to_string())
    .bind(source)
    .bind(total_count)
    .bind(success_count)
    .bind(failure_count)
    .bind(total_tokens)
    .bind(total_cost)
    .execute(pool)
    .await
    .expect("insert invocation rollup");

    let bucket_start_epoch = local_naive_to_utc(
        stats_date
            .and_hms_opt(0, 0, 0)
            .expect("stats_date midnight should be valid"),
        Shanghai,
    )
    .timestamp();
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(source)
    .bind(total_count)
    .bind(success_count)
    .bind(failure_count)
    .bind(total_tokens)
    .bind(total_cost)
    .bind(first_byte_samples.len() as i64)
    .bind(sum_f64_samples(first_byte_samples))
    .bind(max_f64_sample(first_byte_samples))
    .bind(encode_histogram_from_samples(first_byte_samples))
    .bind(first_response_byte_total_samples.len() as i64)
    .bind(sum_f64_samples(first_response_byte_total_samples))
    .bind(max_f64_sample(first_response_byte_total_samples))
    .bind(encode_histogram_from_samples(
        first_response_byte_total_samples,
    ))
    .execute(pool)
    .await
    .expect("insert invocation hourly rollup");
}

pub(crate) async fn insert_invocation_hourly_rollup_bucket(
    pool: &SqlitePool,
    fixture: HourlyRollupFixture<'_>,
) {
    insert_invocation_hourly_rollup_bucket_with_latency_samples(pool, fixture).await;
}

pub(crate) struct HourlyRollupFixture<'a> {
    pub(crate) bucket_start: DateTime<Utc>,
    pub(crate) source: &'a str,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_byte_samples: &'a [f64],
    pub(crate) first_response_byte_total_samples: &'a [f64],
}

impl<'a> HourlyRollupFixture<'a> {
    pub(crate) fn without_latency(
        bucket_start: DateTime<Utc>,
        source: &'a str,
        total_count: i64,
        success_count: i64,
        failure_count: i64,
        total_tokens: i64,
        total_cost: f64,
    ) -> Self {
        Self {
            bucket_start,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        }
    }
}

pub(crate) async fn insert_invocation_hourly_rollup_bucket_with_latency_samples(
    pool: &SqlitePool,
    fixture: HourlyRollupFixture<'_>,
) {
    let HourlyRollupFixture {
        bucket_start,
        source,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        total_cost,
        first_byte_samples,
        first_response_byte_total_samples,
    } = fixture;
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
        "#,
    )
    .bind(bucket_start.timestamp())
    .bind(source)
    .bind(total_count)
    .bind(success_count)
    .bind(failure_count)
    .bind(total_tokens)
    .bind(total_cost)
    .bind(first_byte_samples.len() as i64)
    .bind(sum_f64_samples(first_byte_samples))
    .bind(max_f64_sample(first_byte_samples))
    .bind(encode_histogram_from_samples(first_byte_samples))
    .bind(first_response_byte_total_samples.len() as i64)
    .bind(sum_f64_samples(first_response_byte_total_samples))
    .bind(max_f64_sample(first_response_byte_total_samples))
    .bind(encode_histogram_from_samples(
        first_response_byte_total_samples,
    ))
    .execute(pool)
    .await
    .expect("insert invocation hourly rollup bucket");
}

use super::*;
