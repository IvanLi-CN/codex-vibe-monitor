const PAYLOAD_ACCOUNT_KIND_SNAPSHOT_INVOKE_ID: &str = "proxy-api-keys-requested-tier-snapshot";

async fn seed_payload_account_kind_snapshot(pool: &SqlitePool) {
    let created_at = format_utc_iso(Utc::now());
    insert_api_key_upstream_account(
        pool,
        UpstreamAccountSeed {
            id: 2568,
            display_name: "API Keys Pool",
            upstream_base_url: "https://api-keys.vendor.invalid/",
            created_at: &created_at,
        },
    )
    .await;
    insert_proxy_cost_row(
        pool,
        ProxyCostRowSeed {
            invoke_id: PAYLOAD_ACCOUNT_KIND_SNAPSHOT_INVOKE_ID,
            status: "success",
            model: "gpt-5.4",
            input_tokens: 1_000,
            output_tokens: 500,
            cost: 0.01,
            price_version: "openai-standard-2026-02-23",
            payload: r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":2568,"upstreamAccountName":"API Keys Pool","upstreamAccountKind":"api_key_codex","upstreamBaseUrlHost":"api-keys.vendor.invalid","routeMode":"pool"}"#,
            raw_response: "{}",
        },
    )
    .await;
}

async fn regress_payload_account_kind_snapshot(pool: &SqlitePool) {
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET kind = ?1,
            upstream_base_url = ?2,
            updated_at = ?3
        WHERE id = ?4
        "#,
    )
    .bind("oauth_codex")
    .bind("https://oauth.vendor.invalid/")
    .bind(format_utc_iso(Utc::now()))
    .bind(2568_i64)
    .execute(pool)
    .await
    .expect("mutate live api keys account");

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET cost = ?1,
            cost_estimated = ?2,
            price_version = ?3,
            payload = json_set(payload, '$.billingServiceTier', NULL)
        WHERE invoke_id = ?4
        "#,
    )
    .bind(0.01_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23")
    .bind(PAYLOAD_ACCOUNT_KIND_SNAPSHOT_INVOKE_ID)
    .execute(pool)
    .await
    .expect("regress api keys snapshot row");
}

async fn assert_payload_account_kind_snapshot(pool: &SqlitePool) {
    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind(PAYLOAD_ACCOUNT_KIND_SNAPSHOT_INVOKE_ID)
    .fetch_one(pool)
    .await
    .expect("query api keys snapshot row");
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read api keys snapshot cost")
            .expect("api keys snapshot cost should exist")
            - 0.02)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("cost_estimated")
            .expect("read api keys snapshot cost_estimated"),
        Some(1)
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read api keys snapshot price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@requested-tier")
    );

    let payload: String = row
        .try_get("payload")
        .expect("read api keys snapshot payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode api keys snapshot payload JSON");
    assert_eq!(payload_json["billingServiceTier"], "priority");
    assert_eq!(payload_json["upstreamAccountKind"], "api_key_codex");
    assert_eq!(
        payload_json["upstreamBaseUrlHost"],
        "api-keys.vendor.invalid"
    );
}

#[tokio::test]
pub(crate) async fn backfill_proxy_missing_costs_prefers_payload_account_kind_snapshots_over_live_account_rows()
 {
    let pool = test_current_schema_pool().await;

    seed_payload_account_kind_snapshot(&pool).await;
    let catalog = openai_standard_backfill_catalog();

    let summary_first = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("first api keys snapshot backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);

    regress_payload_account_kind_snapshot(&pool).await;

    let summary_second = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("second api keys snapshot backfill should still use payload snapshot");
    assert_eq!(summary_second.scanned, 1);
    assert_eq!(summary_second.updated, 1);

    assert_payload_account_kind_snapshot(&pool).await;
}

#[tokio::test]
pub(crate) async fn backfill_proxy_missing_costs_falls_back_to_safe_live_api_key_account_kind_when_snapshot_missing()
 {
    let pool = test_current_schema_pool().await;
    insert_api_key_upstream_account(
        &pool,
        UpstreamAccountSeed {
            id: 6144,
            display_name: "API Keys Safe Live",
            upstream_base_url: "https://api-keys.safe.invalid/v1",
            created_at: "2026-01-01T00:00:00Z",
        },
    )
    .await;
    insert_proxy_cost_row(
        &pool,
        ProxyCostRowSeed {
            invoke_id: "proxy-safe-live-api-keys",
            status: "success",
            model: "gpt-5.4",
            input_tokens: 1_000,
            output_tokens: 500,
            cost: 0.01,
            price_version: "openai-standard-2026-02-23",
            payload: r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":6144,"upstreamAccountName":"API Keys Safe Live","routeMode":"pool"}"#,
            raw_response: "{}",
        },
    )
    .await;
    let catalog = openai_standard_backfill_catalog();

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("safe live api keys rows should use the requested-tier strategy");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-safe-live-api-keys")
    .fetch_one(&pool)
    .await
    .expect("query safe live api keys row");
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read safe live api keys cost")
            .expect("safe live api keys cost should exist")
            - 0.02)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read safe live api keys price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@requested-tier")
    );

    let payload: String = row
        .try_get("payload")
        .expect("read safe live api keys payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode safe live api keys payload JSON");
    assert_eq!(payload_json["billingServiceTier"], "priority");
    assert_eq!(payload_json["upstreamAccountKind"], "api_key_codex");
    assert_eq!(payload_json["upstreamBaseUrlHost"], "api-keys.safe.invalid");
}

#[tokio::test]
pub(crate) async fn backfill_proxy_missing_costs_keeps_response_tier_when_live_account_created_after_invocation()
 {
    let pool = test_current_schema_pool().await;
    insert_api_key_upstream_account(
        &pool,
        UpstreamAccountSeed {
            id: 5120,
            display_name: "Late API Keys Account",
            upstream_base_url: "https://late-api-keys.invalid/",
            created_at: "2026-03-01T00:00:00Z",
        },
    )
    .await;
    insert_proxy_cost_row(
        &pool,
        ProxyCostRowSeed {
            invoke_id: "proxy-late-live-api-keys",
            status: "success",
            model: "gpt-5.4",
            input_tokens: 1_000,
            output_tokens: 500,
            cost: 0.01,
            price_version: "openai-standard-2026-02-23",
            payload: r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountId":5120,"upstreamAccountName":"Late API Keys Account","routeMode":"pool"}"#,
            raw_response: "{}",
        },
    )
    .await;
    let catalog = openai_standard_backfill_catalog();

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("late live accounts should fall back to the response-tier strategy");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-late-live-api-keys")
    .fetch_one(&pool)
    .await
    .expect("query late live api keys row");
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read late live api keys cost")
            .expect("late live api keys cost should exist")
            - 0.01)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read late live api keys price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@response-tier")
    );

    let payload: String = row
        .try_get("payload")
        .expect("read late live api keys payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode late live api keys payload JSON");
    assert_eq!(payload_json["billingServiceTier"], "default");
    assert_eq!(payload_json.get("upstreamAccountKind"), Some(&Value::Null));
    assert_eq!(payload_json.get("upstreamBaseUrlHost"), Some(&Value::Null));
}

#[tokio::test]
pub(crate) async fn backfill_proxy_missing_costs_keeps_non_api_keys_rows_on_response_tier_strategy()
{
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-oauth-response-tier")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.4")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.01_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23")
    .bind(r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"default","upstreamAccountKind":"oauth_codex","routeMode":"pool"}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert non-api-keys response-tier row");

    let catalog = PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("non-api-keys rows should stay on the response-tier strategy");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let row = sqlx::query(
        "SELECT cost, price_version, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-oauth-response-tier")
    .fetch_one(&pool)
    .await
    .expect("query non-api-keys response-tier row");
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read non-api-keys cost")
            .expect("non-api-keys cost should exist")
            - 0.01)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read non-api-keys price_version")
            .as_deref(),
        Some("openai-standard-2026-02-23@response-tier")
    );

    let payload: String = row.try_get("payload").expect("read non-api-keys payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode non-api-keys payload JSON");
    assert_eq!(payload_json["billingServiceTier"], "default");
}

#[tokio::test]
pub(crate) async fn backfill_proxy_missing_costs_skips_rows_already_settled_with_requested_tier_strategy()
 {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind("proxy-api-keys-requested-tier-settled")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("gpt-5.4")
    .bind(1_000_i64)
    .bind(500_i64)
    .bind(1_500_i64)
    .bind(0.02_f64)
    .bind(1_i64)
    .bind("openai-standard-2026-02-23@requested-tier")
    .bind(r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority","serviceTier":"flex","billingServiceTier":"priority","upstreamAccountKind":"api_key_codex","upstreamBaseUrlHost":"api-keys.vendor.invalid","routeMode":"pool"}"#)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert settled requested-tier invocation");

    let catalog = PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("settled requested-tier rows should remain idempotent");
    assert_eq!(summary.scanned, 0);
    assert_eq!(summary.updated, 0);
}

fn catalog_with_unknown_model(version: &str) -> PricingCatalog {
    let mut catalog = unit_cost_backfill_catalog();
    catalog.version = version.to_string();
    catalog.models.insert(
        "unknown-model".to_string(),
        ModelPricing {
            input_per_1m: 4.0,
            output_per_1m: 6.0,
            cache_input_per_1m: None,
            cache_read_per_1m: None,
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "custom".to_string(),
        },
    );
    catalog
}

async fn assert_unpriced_attempt(pool: &SqlitePool, expected_attempt_version: &str) {
    let unknown_row = sqlx::query(
        "SELECT cost, cost_estimated, price_version FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-cost-backfill-unpriced-model")
    .fetch_one(pool)
    .await
    .expect("query unpriced model row");
    assert_eq!(
        unknown_row
            .try_get::<Option<f64>, _>("cost")
            .expect("read unknown cost"),
        None
    );
    assert_eq!(
        unknown_row
            .try_get::<Option<i64>, _>("cost_estimated")
            .expect("read unknown cost_estimated"),
        Some(0)
    );
    assert_eq!(
        unknown_row
            .try_get::<Option<String>, _>("price_version")
            .expect("read unknown price_version")
            .as_deref(),
        Some(expected_attempt_version)
    );
}

#[tokio::test]
pub(crate) async fn backfill_proxy_missing_costs_skips_missing_model_or_usage_and_retries_unpriced_rows()
 {
    let pool = test_current_schema_pool().await;

    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-missing-model",
        None,
        Some(1_000),
        Some(500),
    )
    .await;
    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-unpriced-model",
        Some("unknown-model"),
        Some(1_000),
        Some(500),
    )
    .await;
    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-missing-usage",
        Some("gpt-5.2"),
        None,
        None,
    )
    .await;

    let catalog = unit_cost_backfill_catalog();

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("cost backfill should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_unpriced_model, 1);
    let expected_attempt_version = pricing_backfill_attempt_version(&catalog);

    assert_unpriced_attempt(&pool, &expected_attempt_version).await;

    let summary_same_version = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("same-version cost backfill should skip attempted unpriced rows");
    assert_eq!(summary_same_version.scanned, 0);
    assert_eq!(summary_same_version.updated, 0);

    let updated_catalog_same_version = catalog_with_unknown_model(&catalog.version);
    let summary_same_version_after_pricing_update =
        backfill_proxy_missing_costs(&pool, &updated_catalog_same_version)
            .await
            .expect("same-version pricing update should retry previously unpriced rows");
    assert_eq!(summary_same_version_after_pricing_update.scanned, 1);
    assert_eq!(summary_same_version_after_pricing_update.updated, 1);
    assert_eq!(
        summary_same_version_after_pricing_update.skipped_unpriced_model,
        0
    );

    let unknown_cost_after_update: Option<f64> =
        sqlx::query_scalar("SELECT cost FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-cost-backfill-unpriced-model")
            .fetch_one(&pool)
            .await
            .expect("query unknown model cost after pricing update");
    let expected_unknown_cost = ((1_000.0 * 4.0) + (500.0 * 6.0)) / 1_000_000.0;
    assert!(
        (unknown_cost_after_update.expect("unknown cost should be backfilled")
            - expected_unknown_cost)
            .abs()
            < 1e-12
    );
}

#[test]
pub(crate) fn is_sqlite_lock_error_detects_structured_sqlite_codes() {
    let busy_code_error = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "simulated sqlite driver failure",
            code: "5",
        },
    )));
    assert!(is_sqlite_lock_error(&busy_code_error));

    let sqlite_busy_name_error = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "simulated sqlite driver failure",
            code: "SQLITE_BUSY",
        },
    )));
    assert!(is_sqlite_lock_error(&sqlite_busy_name_error));

    let non_lock_error = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "simulated sqlite driver failure",
            code: "SQLITE_CONSTRAINT",
        },
    )));
    assert!(!is_sqlite_lock_error(&non_lock_error));
}

#[tokio::test]
pub(crate) async fn run_best_effort_retention_pragma_tolerates_sqlite_lock_errors() {
    let err = run_best_effort_retention_pragma(
        &SqlitePool::connect_lazy("sqlite::memory:").expect("construct lazy sqlite pool"),
        "SELECT 1",
        "retention wal checkpoint",
    )
    .await;
    assert!(err.is_ok());

    let locked = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "database table is locked",
            code: "SQLITE_LOCKED",
        },
    )));
    assert!(is_sqlite_lock_error(&locked));
}

#[tokio::test]
pub(crate) async fn build_sqlite_connect_options_enforces_wal_and_busy_timeout_defaults() {
    let temp_dir = make_temp_test_dir("sqlite-connect-options");
    let db_path = temp_dir.join("options.db");
    let db_url = test_sqlite_url_for_path(&db_path);

    let options = build_sqlite_connect_options(
        &db_url,
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .expect("build sqlite connect options");
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .expect("connect sqlite with options");

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode;")
        .fetch_one(&mut conn)
        .await
        .expect("read pragma journal_mode");
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

    let busy_timeout_ms: i64 = sqlx::query_scalar("PRAGMA busy_timeout;")
        .fetch_one(&mut conn)
        .await
        .expect("read pragma busy_timeout");
    assert_eq!(
        busy_timeout_ms,
        (DEFAULT_SQLITE_BUSY_TIMEOUT_SECS * 1_000) as i64
    );

    conn.close().await.expect("close sqlite connection");
    let _ = fs::remove_dir_all(&temp_dir);
}

fn file_backed_test_config(temp_dir: &Path, db_path: PathBuf) -> AppConfig {
    let mut config = test_config();
    config.database_path = db_path;
    config.archive_dir = temp_dir.join("archive");
    config.proxy_raw_dir = temp_dir.join("proxy-raw");
    config.xray_runtime_dir = temp_dir.join("xray-runtime");
    fs::create_dir_all(&config.archive_dir).expect("create archive dir");
    fs::create_dir_all(&config.proxy_raw_dir).expect("create proxy raw dir");
    fs::create_dir_all(&config.xray_runtime_dir).expect("create xray runtime dir");
    config
}

pub(crate) async fn file_backed_test_state_with_busy_timeout(
    prefix: &str,
    busy_timeout: Duration,
) -> (Arc<AppState>, PathBuf, String) {
    let temp_dir = make_temp_test_dir(prefix);
    let db_path = temp_dir.join("state.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let connect_options =
        build_sqlite_connect_options(&db_url, busy_timeout).expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let config = file_backed_test_config(&temp_dir, db_path);

    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let pricing_catalog = load_pricing_catalog(&pool)
        .await
        .expect("pricing catalog should initialize");

    let prompt_cache_conversation_cache =
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    let state = Arc::new(AppState {
        config: config.clone(),
        sqlite_batch_writer: SqliteBatchWriter::spawn_for_test_with_prompt_cache(
            prompt_cache_conversation_cache.clone(),
        ),
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
        pricing_catalog: Arc::new(RwLock::new(pricing_catalog)),
        prompt_cache_conversation_cache,
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
        fallback_proxy_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    });

    (state, temp_dir, db_url)
}

pub(crate) async fn wait_for_summary_quota_workers(state: &AppState) {
    let handles = {
        let mut guard = state.proxy_summary_quota_broadcast_handle.lock().await;
        std::mem::take(&mut *guard)
    };
    for handle in handles {
        handle
            .await
            .expect("summary/quota worker should join cleanly");
    }
}

#[tokio::test]
pub(crate) async fn dashboard_read_endpoints_stay_queryable_under_sqlite_write_lock() {
    let (state, temp_dir, db_url) =
        file_backed_test_state_with_busy_timeout("dashboard-read-lock", Duration::from_millis(100))
            .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    persist_proxy_capture_record(
        &state.pool,
        Instant::now(),
        test_proxy_capture_record("dashboard-lock-read", &occurred_at),
    )
    .await
    .expect("seed dashboard lock record");
    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("seed hourly rollups before lock");
    insert_parallel_work_invocation(
        &state.pool,
        "dashboard-lock-unsynced-working-conversation",
        Utc::now(),
        "dashboard-lock-read",
    )
    .await;

    let summary_query = SummaryQuery {
        window: Some("today".to_string()),
        limit: None,
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    };
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate summary projection before acquiring the write lock");

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let Json(summary) = fetch_summary(State(state.clone()), Query(summary_query))
        .await
        .expect("today summary should stay readable under a concurrent write lock");
    assert!(
        summary.total_count >= 1,
        "today summary should still return persisted totals"
    );

    let Json(timeseries) = fetch_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: "today".to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("today timeseries should stay readable under a concurrent write lock");
    assert!(
        timeseries.points.iter().any(|point| point.total_count >= 1),
        "today timeseries should still expose the live point"
    );

    let Json(conversations) = fetch_prompt_cache_conversations(
        State(state.clone()),
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
    .expect("working conversations should stay readable under a concurrent write lock");
    assert!(
        !conversations.conversations.is_empty(),
        "working conversations should still return the seeded prompt-cache key"
    );

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release sqlite write lock");
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

use super::*;
