#[tokio::test]
pub(crate) async fn system_task_runs_schema_indexes_support_time_queries() {
    let pool = test_current_schema_pool().await;
    ensure_schema(&pool)
        .await
        .expect("apply current system task run schema");
    sqlx::query(
        r#"
        WITH RECURSIVE
            hundreds(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM hundreds WHERE value < 999
            ),
            blocks(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM blocks WHERE value < 399
            )
        INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at)
        SELECT
            CASE WHEN (hundreds.value + blocks.value) % 2 = 0
                THEN 'retention_archive' ELSE 'startup_backfill' END,
            'fixture',
            CASE WHEN hundreds.value % 3 = 0 THEN 'success' ELSE 'failed' END,
            printf(
                '2026-06-%02dT%02d:%02d:%02d.000Z',
                1 + ((hundreds.value * 400 + blocks.value) / 86400),
                ((hundreds.value * 400 + blocks.value) / 3600) % 24,
                ((hundreds.value * 400 + blocks.value) / 60) % 60,
                (hundreds.value * 400 + blocks.value) % 60
            )
        FROM hundreds CROSS JOIN blocks
        "#,
    )
    .execute(&pool)
    .await
    .expect("seed 400k system task runs");

    let default_plan: Vec<String> = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT id FROM system_task_runs ORDER BY started_at DESC, id DESC LIMIT 100",
    )
    .fetch_all(&pool)
    .await
    .expect("explain default system task query")
    .into_iter()
    .map(|row| row.get("detail"))
    .collect();
    assert!(
        default_plan
            .iter()
            .any(|detail| detail.contains("idx_system_task_runs_started_at_id")),
        "default time query must use its ordering index: {default_plan:?}"
    );

    let filtered_plan: Vec<String> = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT id FROM system_task_runs WHERE task_kind = 'retention_archive' AND status = 'success' AND started_at >= '2026-06-02T00:00:00Z' ORDER BY started_at DESC, id DESC LIMIT 100",
    )
    .fetch_all(&pool)
    .await
    .expect("explain combined system task query")
    .into_iter()
    .map(|row| row.get("detail"))
    .collect();
    assert!(
        filtered_plan
            .iter()
            .any(|detail| detail.contains("idx_system_task_runs_task_status_time")),
        "combined filter and sort query must use its index: {filtered_plan:?}"
    );
}

async fn assert_system_task_timestamp_normalization(
    pool: &SqlitePool,
    task_kind: &str,
    started_at: &str,
    finished_at: Option<&str>,
    expected_started_at: &str,
    expected_finished_at: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at, finished_at) VALUES (?1, 'fixture', 'success', ?2, ?3)",
    )
    .bind(task_kind)
    .bind(started_at)
    .bind(finished_at)
    .execute(pool)
    .await
    .expect("seed legacy system task timestamp");

    ensure_schema(pool)
        .await
        .expect("normalize legacy system task timestamps");
    let (actual_started_at, actual_finished_at): (String, Option<String>) =
        sqlx::query_as("SELECT started_at, finished_at FROM system_task_runs WHERE task_kind = ?1")
            .bind(task_kind)
            .fetch_one(pool)
            .await
            .expect("load normalized task timestamps");
    assert_eq!(actual_started_at, expected_started_at);
    assert_eq!(actual_finished_at.as_deref(), expected_finished_at);
}

#[tokio::test]
pub(crate) async fn ensure_schema_normalizes_legacy_system_task_run_timestamps() {
    let pool = test_current_schema_pool().await;
    assert_system_task_timestamp_normalization(
        &pool,
        "startup_backfill",
        "2026-06-22 08:45:00.125",
        Some("2026-06-22T17:15:00+08:00"),
        "2026-06-22T00:45:00.125Z",
        Some("2026-06-22T09:15:00.000Z"),
    )
    .await;
    assert_system_task_timestamp_normalization(
        &pool,
        "startup_backfill_with_offset",
        "2026-06-22 08:45:00.125+08:00",
        Some("2026-06-22 17:15:00+08:00"),
        "2026-06-22T00:45:00.125Z",
        Some("2026-06-22T09:15:00.000Z"),
    )
    .await;
    assert_system_task_timestamp_normalization(
        &pool,
        "legacy_rfc3339_without_millis",
        "2026-06-22T08:45:00Z",
        None,
        "2026-06-22T08:45:00.000Z",
        None,
    )
    .await;
    assert_system_task_timestamp_normalization(
        &pool,
        "invalid_legacy_task_timestamp",
        "2026-02-30 08:45:00",
        Some("2026-02-30 17:15:00+08:00"),
        "2026-02-30 08:45:00",
        Some("2026-02-30 17:15:00+08:00"),
    )
    .await;
    assert_system_task_timestamp_normalization(
        &pool,
        "invalid_legacy_task_timestamp_suffix",
        "2026-06-22 08:45:00foo",
        Some("2026-06-22 17:15:00+08:00foo"),
        "2026-06-22 08:45:00foo",
        Some("2026-06-22 17:15:00+08:00foo"),
    )
    .await;
}

#[tokio::test]
pub(crate) async fn system_task_run_retention_preserves_recent_rows_and_bounds_deletes() {
    let _schedule_guard = SYSTEM_TASK_RUN_RETENTION_TEST_LOCK.lock().await;
    reset_system_task_run_retention_schedule();
    let pool = test_current_schema_pool().await;
    ensure_schema(&pool)
        .await
        .expect("apply current system task run schema");
    sqlx::query(
        r#"
        WITH RECURSIVE
            hundreds(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM hundreds WHERE value < 999
            ),
            blocks(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM blocks WHERE value < 5
            )
        INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at)
        SELECT 'retention_archive', 'fixture', 'success', '2020-01-01T00:00:00.000Z'
        FROM hundreds CROSS JOIN blocks
        "#,
    )
    .execute(&pool)
    .await
    .expect("seed expired terminal task runs");
    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at) VALUES ('retention_archive', 'fixture', 'running', '2019-01-01T00:00:00.000Z')",
    )
    .execute(&pool)
    .await
    .expect("seed running task run");
    sqlx::query(
        "INSERT INTO system_task_runs (task_kind, trigger_kind, status, started_at) VALUES ('retention_archive', 'fixture', 'success', '2026-02-30T08:45:00.000Z')",
    )
    .execute(&pool)
    .await
    .expect("seed canonical-shaped invalid retention timestamp");

    let pruned = prune_system_task_runs(&pool, false)
        .await
        .expect("prune expired task runs");
    assert_eq!(pruned, 5_000, "one retention pass has a hard delete cap");
    let terminal_remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'retention_archive' AND status = 'success'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining terminal task runs");
    assert_eq!(terminal_remaining, 1_001);
    let oldest_retained_id: i64 = sqlx::query_scalar(
        "SELECT MIN(id) FROM system_task_runs WHERE task_kind = 'retention_archive' AND status = 'success'",
    )
    .fetch_one(&pool)
    .await
    .expect("find oldest retained terminal task run");
    assert_eq!(
        oldest_retained_id, 5_001,
        "the pass deletes the oldest records before newer equal-timestamp ties"
    );
    let invalid_timestamp_remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'retention_archive' AND status = 'success' AND started_at = '2026-02-30T08:45:00.000Z'",
    )
    .fetch_one(&pool)
    .await
    .expect("count preserved invalid retention timestamp");
    assert_eq!(invalid_timestamp_remaining, 1);
    let running_remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM system_task_runs WHERE status = 'running'")
            .fetch_one(&pool)
            .await
            .expect("count running task runs");
    assert_eq!(running_remaining, 1, "running task runs are never pruned");
    assert_eq!(
        prune_system_task_runs(&pool, false)
            .await
            .expect("immediate retention pass should be throttled"),
        0,
        "retention passes are spaced by at least 15 seconds"
    );
    reset_system_task_run_retention_schedule();
    assert_eq!(
        prune_system_task_runs(&pool, true)
            .await
            .expect("count remaining retention candidates"),
        800,
        "dry run counts each eligible row once and respects the per-pass cap"
    );
    reset_system_task_run_retention_schedule();
}

#[tokio::test]
pub(crate) async fn system_task_run_retention_pressure_marks_the_maintenance_pass_deferred() {
    let _schedule_guard = SYSTEM_TASK_RUN_RETENTION_TEST_LOCK.lock().await;
    let generation_before = retention_defer_generation();
    let pressure_error = anyhow::anyhow!("pool timed out while waiting for an open connection");
    assert!(system_task_run_retention_handle_pressure(&pressure_error));
    assert!(
        retention_defer_generation() > generation_before,
        "pressure must wake the five-minute maintenance retry path"
    );
    reset_system_task_run_retention_schedule();
}

pub(crate) fn write_backfill_response_payload_with_terminal_service_tier(
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

pub(crate) fn write_backfill_request_payload(path: &Path, prompt_cache_key: Option<&str>) {
    write_backfill_request_payload_with_fields(
        path,
        prompt_cache_key,
        None,
        None,
        ProxyCaptureTarget::Responses,
    );
}

pub(crate) fn write_backfill_request_payload_with_requested_service_tier(
    path: &Path,
    requested_service_tier: Option<&str>,
    target: ProxyCaptureTarget,
) {
    write_backfill_request_payload_with_fields(path, None, None, requested_service_tier, target);
}

pub(crate) fn write_backfill_request_payload_with_reasoning(
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

pub(crate) fn write_backfill_request_payload_with_fields(
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
        ProxyCaptureTarget::StandaloneSearch => json!({
            "model": "gpt-5.3-codex",
            "query": "hello",
        }),
        ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits => {
            let mut payload = json!({
                "model": "gpt-image-1",
                "prompt": "hello",
            });
            if let Some(service_tier) = requested_service_tier {
                payload["service_tier"] = Value::String(service_tier.to_string());
            }
            payload
        }
    };
    let encoded = serde_json::to_vec(&payload).expect("serialize request payload");
    fs::write(path, encoded).expect("write request payload");
}

pub(crate) async fn insert_proxy_backfill_row(
    pool: &SqlitePool,
    invoke_id: &str,
    response_path: &Path,
) {
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

pub(crate) async fn insert_proxy_cost_backfill_row(
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

pub(crate) async fn insert_proxy_prompt_cache_backfill_row(
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

pub(crate) async fn test_state_with_openai_base(openai_base: Url) -> Arc<AppState> {
    test_state_with_openai_base_and_body_limit(
        openai_base,
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
    )
    .await
}

pub(crate) async fn test_state_with_openai_base_and_runtime_projection_mode(
    openai_base: Url,
    runtime_projection_mode: RuntimeProjectionMode,
) -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url = openai_base;
    test_state_from_config_with_pool_no_available_wait_and_runtime_projection_mode(
        config,
        true,
        immediate_test_pool_no_available_wait_settings(),
        runtime_projection_mode,
    )
    .await
}

pub(crate) async fn test_state_with_openai_base_and_body_limit(
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

pub(crate) async fn test_state_with_openai_base_body_limit_and_read_timeout(
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

pub(crate) async fn test_state_with_openai_base_and_proxy_timeouts(
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

pub(crate) async fn test_state_with_openai_base_and_pool_no_available_wait(
    openai_base: Url,
    timeout: Duration,
    _poll_interval: Duration,
) -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url = openai_base;
    test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout,
            poll_interval: _poll_interval,
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await
}

pub(crate) async fn test_state_from_config(
    config: AppConfig,
    startup_ready: bool,
) -> Arc<AppState> {
    test_state_from_config_with_pool_no_available_wait(
        config,
        startup_ready,
        immediate_test_pool_no_available_wait_settings(),
    )
    .await
}

pub(crate) fn immediate_test_pool_no_available_wait_settings() -> PoolNoAvailableWaitSettings {
    PoolNoAvailableWaitSettings {
        timeout: Duration::ZERO,
        poll_interval: Duration::ZERO,
        retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
    }
}

pub(crate) fn isolate_default_test_runtime_path(
    path: &Path,
    default_root: &Path,
    db_id: u64,
) -> PathBuf {
    if path != default_root {
        return path.to_path_buf();
    }
    let isolated = default_root.join(format!("{}-{db_id}", std::process::id()));
    fs::create_dir_all(&isolated).expect("create isolated test runtime dir");
    isolated
}

pub(crate) fn isolate_stateful_test_config_runtime_paths(
    mut config: AppConfig,
    db_id: u64,
) -> AppConfig {
    config.archive_dir = isolate_default_test_runtime_path(
        &config.archive_dir,
        &test_runtime_path("archive-tests"),
        db_id,
    );
    config.proxy_raw_dir = isolate_default_test_runtime_path(
        &config.proxy_raw_dir,
        &test_runtime_path("proxy-raw-tests"),
        db_id,
    );
    config.xray_runtime_dir = isolate_default_test_runtime_path(
        &config.xray_runtime_dir,
        &test_runtime_path("xray-forward-tests"),
        db_id,
    );
    config
}

pub(crate) async fn test_state_from_config_with_pool_no_available_wait(
    config: AppConfig,
    startup_ready: bool,
    pool_no_available_wait: PoolNoAvailableWaitSettings,
) -> Arc<AppState> {
    test_state_from_config_with_pool_no_available_wait_and_runtime_projection_mode(
        config,
        startup_ready,
        pool_no_available_wait,
        RuntimeProjectionMode::Auto,
    )
    .await
}

pub(crate) async fn test_current_schema_pool() -> SqlitePool {
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url =
        format!("sqlite:file:codex-vibe-monitor-test-pool-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        // Keep the shared in-memory schema available while a test acquires connections.
        .min_connections(1)
        .max_connections(4)
        .connect(&db_url)
        .await
        .expect("connect in-memory sqlite");
    restore_stateful_schema_template(&pool)
        .await
        .expect("initialize current-schema test pool");
    pool
}

pub(crate) async fn test_state_from_config_with_pool_no_available_wait_and_runtime_projection_mode(
    config: AppConfig,
    startup_ready: bool,
    pool_no_available_wait: PoolNoAvailableWaitSettings,
    runtime_projection_mode: RuntimeProjectionMode,
) -> Arc<AppState> {
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let config = isolate_stateful_test_config_runtime_paths(config, db_id);
    let db_url = format!("sqlite:file:codex-vibe-monitor-test-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        // A shared in-memory database is destroyed when its final connection closes.
        // Keep its schema alive while concurrent test tasks acquire and release connections.
        .min_connections(1)
        .max_connections(4)
        .connect(&db_url)
        .await
        .expect("connect in-memory sqlite");
    restore_stateful_schema_template(&pool)
        .await
        .expect("schema should initialize from the stateful template");

    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let pricing_catalog = load_pricing_catalog(&pool)
        .await
        .expect("pricing catalog should initialize");
    let prompt_cache_conversation_cache =
        Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    let sqlite_batch_writer = SqliteBatchWriter::spawn_for_test_with_prompt_cache(
        prompt_cache_conversation_cache.clone(),
    );
    let proxy_runtime_invocations =
        Arc::new(ProxyRuntimeInvocationStore::new(runtime_projection_mode));
    sqlite_batch_writer.set_terminal_runtime_store(proxy_runtime_invocations.clone());

    Arc::new(AppState {
        config: config.clone(),
        sqlite_batch_writer,
        pool_account_selection_runtime: Arc::new(PoolAccountSelectionRuntime::default()),
        proxy_runtime_invocations,
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
        fallback_proxy_429_retry_delay_override: Some(Duration::ZERO),
        pool_no_available_wait,
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    })
}

pub(crate) async fn write_stateful_schema_template(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "create stateful schema template directory {}",
                parent.display()
            )
        })?;
    }
    if path.exists() {
        fs::remove_file(path)
            .with_context(|| format!("remove stale stateful schema template {}", path.display()))?;
    }

    let options = SqliteConnectOptions::from_str(&test_sqlite_url_for_path(path))
        .context("build stateful schema template sqlite options")?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .min_connections(1)
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| format!("open stateful schema template {}", path.display()))?;
    ensure_schema(&pool)
        .await
        .context("initialize stateful schema template")?;
    pool.close().await;
    Ok(())
}

pub(crate) async fn restore_stateful_schema_template(pool: &SqlitePool) -> anyhow::Result<()> {
    let Some(template_path) = std::env::var_os(STATEFUL_SCHEMA_TEMPLATE_PATH_ENV) else {
        return ensure_schema(pool).await;
    };
    let template_path = PathBuf::from(template_path);
    if !template_path.is_file() {
        anyhow::bail!(
            "stateful schema template does not exist: {}",
            template_path.display()
        );
    }
    restore_stateful_schema_template_from_path(pool, &template_path).await
}

pub(crate) async fn restore_stateful_schema_template_from_path(
    pool: &SqlitePool,
    template_path: &Path,
) -> anyhow::Result<()> {
    let options = SqliteConnectOptions::from_str(&test_sqlite_url_for_path(template_path))
        .context("build stateful schema template reader options")?
        .read_only(true)
        .create_if_missing(false);
    let mut source = SqliteConnection::connect_with(&options)
        .await
        .with_context(|| format!("open stateful schema template {}", template_path.display()))?;
    let mut destination = pool
        .acquire()
        .await
        .context("acquire stateful test sqlite")?;
    let mut destination_handle = destination
        .lock_handle()
        .await
        .context("lock stateful test sqlite handle")?;
    let mut source_handle = source
        .lock_handle()
        .await
        .context("lock stateful schema template handle")?;

    // The SQLite backup API copies the already-built template without replaying DDL per test.
    let backup = unsafe {
        sqlite3_backup_init(
            destination_handle.as_raw_handle().as_ptr(),
            c"main".as_ptr(),
            source_handle.as_raw_handle().as_ptr(),
            c"main".as_ptr(),
        )
    };
    if backup.is_null() {
        anyhow::bail!("start stateful schema SQLite backup");
    }
    let step_code = unsafe { sqlite3_backup_step(backup, -1) };
    let finish_code = unsafe { sqlite3_backup_finish(backup) };
    if step_code != SQLITE_DONE || finish_code != SQLITE_OK {
        anyhow::bail!(
            "copy stateful schema SQLite backup failed: step={step_code}, finish={finish_code}"
        );
    }

    drop(source_handle);
    drop(destination_handle);
    drop(destination);
    source
        .close()
        .await
        .context("close stateful schema template")?;
    Ok(())
}

pub(crate) fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub(crate) fn quote_sqlite_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) async fn schema_object_signature(
    pool: &SqlitePool,
) -> Vec<(String, String, String, Option<String>)> {
    sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE type IN ('index', 'table', 'trigger', 'view') AND name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .fetch_all(pool)
    .await
    .expect("load schema signature")
}

pub(crate) async fn schema_default_data_signature(pool: &SqlitePool) -> Vec<(String, Vec<String>)> {
    let table_names = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .expect("load schema table names");
    let mut table_rows = Vec::with_capacity(table_names.len());
    for table_name in table_names {
        let columns = sqlx::query_scalar::<_, String>(&format!(
            "SELECT name FROM pragma_table_info({}) ORDER BY cid",
            quote_sqlite_string(&table_name)
        ))
        .fetch_all(pool)
        .await
        .expect("load schema signature columns");
        let values = columns
            .iter()
            .map(|column| {
                let identifier = quote_sqlite_identifier(column);
                if column.ends_with("_at") {
                    format!(
                        "CASE WHEN {identifier} IS NULL THEN 'NULL' ELSE '<initialization-timestamp>' END"
                    )
                } else {
                    format!("quote({identifier})")
                }
            })
            .collect::<Vec<_>>()
            .join(" || char(31) || ");
        let rows = if values.is_empty() {
            Vec::new()
        } else {
            sqlx::query_scalar::<_, String>(&format!(
                "SELECT {values} FROM {} ORDER BY 1",
                quote_sqlite_identifier(&table_name)
            ))
            .fetch_all(pool)
            .await
            .expect("load schema default data signature")
        };
        table_rows.push((table_name, rows));
    }
    table_rows
}

use super::*;
