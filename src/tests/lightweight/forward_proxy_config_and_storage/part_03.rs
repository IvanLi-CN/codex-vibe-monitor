#[test]
pub(crate) fn app_config_from_sources_rejects_all_legacy_public_env_renames() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();

    for (legacy_name, canonical_name) in LEGACY_ENV_RENAMES {
        let mut cases = LEGACY_ENV_RENAMES
            .iter()
            .map(|(legacy, _)| (*legacy, None))
            .collect::<Vec<_>>();
        let target = cases
            .iter_mut()
            .find(|(name, _)| *name == *legacy_name)
            .expect("legacy env should be present in helper list");
        *target = (*legacy_name, Some("legacy-value"));
        let _env = EnvVarGuard::set(&cases);

        let err = AppConfig::from_sources(&CliArgs::default())
            .expect_err("legacy env should fail fast with a rename hint");
        assert_eq!(
            err.to_string(),
            format!("{legacy_name} is not supported; rename it to {canonical_name}")
        );
    }
}

#[test]
pub(crate) fn app_config_from_sources_accepts_kaisoumail_base_url_and_api_key_only() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[
        (
            ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL,
            Some("https://km.example.test"),
        ),
        (
            ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY,
            Some("cfm_test_key"),
        ),
        (ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_MAIL_DOMAIN, None),
        (ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_SUBDOMAIN, None),
        (LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL, None),
        (LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY, None),
        (LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN, None),
    ]);

    let config = AppConfig::from_sources(&CliArgs::default())
        .expect("KaisouMail should only require base URL and API key");

    assert!(config.upstream_accounts_kaisoumail.is_some());
}

#[test]
pub(crate) fn app_config_from_sources_rejects_kaisoumail_default_generation_env_vars() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();

    for name in [
        ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_MAIL_DOMAIN,
        ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_SUBDOMAIN,
        LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN,
    ] {
        let _env = EnvVarGuard::set(&[
            (ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL, None),
            (ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY, None),
            (ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_MAIL_DOMAIN, None),
            (ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_SUBDOMAIN, None),
            (LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL, None),
            (LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY, None),
            (LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN, None),
            (name, Some("mail-tw.707979.xyz")),
        ]);

        let err = AppConfig::from_sources(&CliArgs::default())
            .expect_err("KaisouMail generated-address defaults should be rejected");
        assert!(
            err.to_string().contains("is not supported; remove it"),
            "error should ask to remove {name}, got {err}"
        );
    }
}

#[test]
pub(crate) fn app_config_from_sources_uses_proxy_timeout_defaults() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let names = [
        "OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS",
        ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
        ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
    ];
    let previous = names
        .iter()
        .map(|name| ((*name).to_string(), env::var_os(name)))
        .collect::<Vec<_>>();

    for name in names {
        unsafe { env::remove_var(name) };
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { env::set_var(name, value) },
            None => unsafe { env::remove_var(name) },
        }
    }

    let config = result.expect("proxy timeout defaults should parse");
    assert_eq!(
        config.openai_proxy_handshake_timeout,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS)
    );
    assert_eq!(
        config.openai_proxy_compact_handshake_timeout,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS)
    );
    assert_eq!(
        config.openai_proxy_request_read_timeout,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS)
    );
    assert_eq!(
        config.pool_upstream_responses_attempt_timeout,
        Duration::from_secs(DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS)
    );
    assert_eq!(
        config.pool_upstream_responses_total_timeout,
        Duration::from_secs(DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS)
    );
}

#[test]
pub(crate) fn app_config_from_sources_reads_proxy_timeout_envs() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let names = [
        "OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS",
        ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
        ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
    ];
    let previous = names
        .iter()
        .map(|name| ((*name).to_string(), env::var_os(name)))
        .collect::<Vec<_>>();

    unsafe {
        env::set_var("OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS", "61");
        env::set_var("OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS", "181");
        env::set_var("OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS", "182");
        env::set_var(ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS, "183");
        env::set_var(ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS, "301");
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { env::set_var(name, value) },
            None => unsafe { env::remove_var(name) },
        }
    }

    let config = result.expect("proxy timeout envs should parse");
    assert_eq!(
        config.openai_proxy_handshake_timeout,
        Duration::from_secs(61)
    );
    assert_eq!(
        config.openai_proxy_compact_handshake_timeout,
        Duration::from_secs(181)
    );
    assert_eq!(
        config.openai_proxy_request_read_timeout,
        Duration::from_secs(182)
    );
    assert_eq!(
        config.pool_upstream_responses_attempt_timeout,
        Duration::from_secs(183)
    );
    assert_eq!(
        config.pool_upstream_responses_total_timeout,
        Duration::from_secs(301)
    );
}

#[test]
pub(crate) fn app_config_from_sources_reads_websocket_enabled_env() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let previous = env::var_os(ENV_OPENAI_PROXY_WEBSOCKET_ENABLED);
    let previous_upstream = env::var_os(ENV_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED);
    let previous_owner_routing =
        env::var_os(ENV_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED);

    unsafe { env::remove_var(ENV_OPENAI_PROXY_WEBSOCKET_ENABLED) };
    unsafe { env::remove_var(ENV_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED) };
    unsafe { env::remove_var(ENV_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED) };
    let default_config =
        AppConfig::from_sources(&CliArgs::default()).expect("default websocket config parses");
    assert_eq!(
        default_config.openai_proxy_websocket_enabled,
        DEFAULT_OPENAI_PROXY_WEBSOCKET_ENABLED
    );
    assert_eq!(
        default_config.openai_proxy_upstream_websocket_default_enabled,
        DEFAULT_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED
    );
    assert_eq!(
        default_config.openai_proxy_encrypted_session_owner_routing_enabled,
        DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED
    );

    unsafe { env::set_var(ENV_OPENAI_PROXY_WEBSOCKET_ENABLED, "true") };
    unsafe { env::set_var(ENV_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED, "true") };
    unsafe {
        env::set_var(
            ENV_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
            "true",
        )
    };
    let enabled_config =
        AppConfig::from_sources(&CliArgs::default()).expect("enabled websocket config parses");

    match previous {
        Some(value) => unsafe { env::set_var(ENV_OPENAI_PROXY_WEBSOCKET_ENABLED, value) },
        None => unsafe { env::remove_var(ENV_OPENAI_PROXY_WEBSOCKET_ENABLED) },
    }
    match previous_upstream {
        Some(value) => unsafe {
            env::set_var(ENV_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED, value)
        },
        None => unsafe { env::remove_var(ENV_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED) },
    }
    match previous_owner_routing {
        Some(value) => unsafe {
            env::set_var(
                ENV_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
                value,
            )
        },
        None => unsafe {
            env::remove_var(ENV_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED)
        },
    }

    assert!(enabled_config.openai_proxy_websocket_enabled);
    assert!(enabled_config.openai_proxy_upstream_websocket_default_enabled);
    assert!(enabled_config.openai_proxy_encrypted_session_owner_routing_enabled);
}

#[test]
pub(crate) fn app_config_from_sources_rejects_zero_pool_upstream_responses_attempt_timeout() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[(ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS, Some("0"))]);

    let err = AppConfig::from_sources(&CliArgs::default())
        .expect_err("zero responses attempt timeout should be rejected");
    assert_eq!(
        err.to_string(),
        format!("{ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS} must be greater than 0")
    );
}

#[test]
pub(crate) fn app_config_from_sources_rejects_zero_pool_upstream_responses_total_timeout() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[(ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS, Some("0"))]);

    let err = AppConfig::from_sources(&CliArgs::default())
        .expect_err("zero responses total timeout should be rejected");
    assert_eq!(
        err.to_string(),
        format!("{ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS} must be greater than 0")
    );
}

pub(crate) fn test_runtime_path(name: &str) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"))
        .join(name)
}

#[tokio::test]
pub(crate) async fn ensure_schema_steady_state_does_not_rebuild_large_derived_invocation_state() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema test pool");
    ensure_schema(&pool).await.expect("seed current schema");
    sqlx::query("DELETE FROM schema_refresh_migrations")
        .execute(&pool)
        .await
        .expect("simulate an existing database before schema refresh markers");

    sqlx::query("INSERT INTO invocation_in_progress_live (invocation_id, source) VALUES (1, 'xy')")
        .execute(&pool)
        .await
        .expect("seed in-progress derived row");
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_working_set_live (
            prompt_cache_key, created_at, last_activity_at, sort_anchor_at
        )
        VALUES ('steady-state-guard', datetime('now'), datetime('now'), datetime('now'))
        "#,
    )
    .execute(&pool)
    .await
    .expect("seed prompt-cache derived row");

    for (name, table) in [
        (
            "trg_test_schema_steady_state_in_progress_guard",
            "invocation_in_progress_live",
        ),
        (
            "trg_test_schema_steady_state_prompt_cache_guard",
            "prompt_cache_working_set_live",
        ),
    ] {
        sqlx::query(&format!(
            "CREATE TRIGGER {name} BEFORE DELETE ON {table} BEGIN SELECT RAISE(ABORT, 'steady-state schema refresh rebuilt a derived table'); END"
        ))
        .execute(&pool)
        .await
        .expect("install derived-table rebuild guard");
    }

    ensure_schema(&pool)
        .await
        .expect("steady-state schema ensure must not rebuild derived invocation state");
    let recorded_refreshes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM schema_refresh_migrations WHERE migration_name IN (?1, ?2)",
    )
    .bind("prompt_cache_expression_indexes_v1")
    .bind("invocation_live_projection_v1")
    .fetch_one(&pool)
    .await
    .expect("load adopted schema refresh markers");

    // The guard triggers above fail this test if a second startup clears either derived table.
    // Existing installations must instead adopt their already-present objects once.
    assert_eq!(
        recorded_refreshes, 2,
        "existing schema objects must be adopted without a destructive refresh"
    );
}

pub(crate) fn test_config() -> AppConfig {
    AppConfig {
        openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
        database_path: PathBuf::from(":memory:"),
        poll_interval: Duration::from_secs(10),
        request_timeout: Duration::from_secs(30),
        pool_upstream_responses_attempt_timeout: Duration::from_secs(
            DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
        ),
        pool_upstream_responses_total_timeout: Duration::from_secs(
            DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
        ),
        openai_proxy_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_compact_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_image_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_IMAGE_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_request_read_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS,
        ),
        openai_proxy_max_request_body_bytes: DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        openai_proxy_websocket_enabled: DEFAULT_OPENAI_PROXY_WEBSOCKET_ENABLED,
        openai_proxy_upstream_websocket_default_enabled:
            DEFAULT_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED,
        openai_proxy_encrypted_session_owner_routing_enabled:
            DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
        proxy_enforce_stream_include_usage: DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
        proxy_usage_backfill_on_startup: DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
        proxy_raw_max_bytes: DEFAULT_PROXY_RAW_MAX_BYTES,
        proxy_raw_dir: test_runtime_path("proxy-raw-tests"),
        proxy_raw_compression: DEFAULT_PROXY_RAW_COMPRESSION,
        proxy_raw_immediate_gzip_bytes: DEFAULT_PROXY_RAW_IMMEDIATE_GZIP_BYTES,
        proxy_raw_hot_secs: DEFAULT_PROXY_RAW_HOT_SECS,
        xray_binary: DEFAULT_XRAY_BINARY.to_string(),
        xray_runtime_dir: test_runtime_path("xray-forward-tests"),
        forward_proxy_algo: ForwardProxyAlgo::V1,
        max_parallel_polls: 2,
        shared_connection_parallelism: 1,
        http_bind: "127.0.0.1:0".parse().expect("valid socket address"),
        cors_allowed_origins: Vec::new(),
        list_limit_max: 100,
        user_agent: "codex-test".to_string(),
        static_dir: None,
        public_origin: None,
        retention_enabled: DEFAULT_RETENTION_ENABLED,
        retention_dry_run: DEFAULT_RETENTION_DRY_RUN,
        retention_interval: Duration::from_secs(DEFAULT_RETENTION_INTERVAL_SECS),
        retention_batch_rows: DEFAULT_RETENTION_BATCH_ROWS,
        retention_catchup_budget: Duration::from_secs(DEFAULT_RETENTION_CATCHUP_BUDGET_SECS),
        archive_dir: test_runtime_path("archive-tests"),
        codex_invocation_archive_layout: DEFAULT_CODEX_INVOCATION_ARCHIVE_LAYOUT,
        codex_invocation_archive_segment_granularity:
            DEFAULT_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY,
        invocation_archive_codec: DEFAULT_INVOCATION_ARCHIVE_CODEC,
        invocation_success_full_days: DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
        invocation_max_days: DEFAULT_INVOCATION_MAX_DAYS,
        invocation_archive_ttl_days: DEFAULT_INVOCATION_ARCHIVE_TTL_DAYS,
        forward_proxy_attempts_retention_days: DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
        pool_upstream_request_attempts_retention_days:
            DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS,
        pool_upstream_request_attempts_archive_ttl_days:
            DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
        quota_snapshot_full_days: DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
        long_term_stats_hourly_retention_days: DEFAULT_LONG_TERM_STATS_HOURLY_RETENTION_DAYS,
        upstream_accounts_oauth_client_id: DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID.to_string(),
        upstream_accounts_oauth_issuer: Url::parse(DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER)
            .expect("valid oauth issuer"),
        upstream_accounts_usage_base_url: Url::parse(DEFAULT_UPSTREAM_ACCOUNTS_USAGE_BASE_URL)
            .expect("valid usage base url"),
        upstream_accounts_login_session_ttl: Duration::from_secs(
            DEFAULT_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS,
        ),
        upstream_accounts_sync_interval: Duration::from_secs(
            DEFAULT_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
        ),
        upstream_accounts_refresh_lead_time: Duration::from_secs(
            DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS,
        ),
        upstream_accounts_history_retention_days: DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
        upstream_accounts_kaisoumail: None,
    }
}

pub(crate) fn make_temp_test_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

pub(crate) fn set_file_mtime_seconds_ago(path: &Path, seconds: u64) {
    let modified_at = std::time::SystemTime::now() - Duration::from_secs(seconds);
    let modified_at = filetime::FileTime::from_system_time(modified_at);
    filetime::set_file_mtime(path, modified_at).expect("set file mtime");
}

pub(crate) fn write_gzip_test_file(path: &Path, content: &[u8]) {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(content).expect("write gzip payload");
    let bytes = encoder.finish().expect("finish gzip payload");
    fs::write(path, bytes).expect("write gzip file");
}

#[test]
pub(crate) fn archive_batch_file_path_resolves_relative_archive_dir_from_database_parent() {
    let mut config = test_config();
    config.database_path = PathBuf::from("/tmp/codex-retention/codex_vibe_monitor.db");
    config.archive_dir = PathBuf::from("archives");

    let path = archive_batch_file_path(&config, "codex_invocations", "2026-03")
        .expect("resolve archive batch path");

    assert_eq!(
        path,
        PathBuf::from(
            "/tmp/codex-retention/archives/codex_invocations/2026/codex_invocations-2026-03.sqlite.gz",
        )
    );
}

#[test]
pub(crate) fn resolved_proxy_raw_dir_resolves_relative_dir_from_database_parent() {
    let mut config = test_config();
    config.database_path = PathBuf::from("/tmp/codex-retention/codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    assert_eq!(
        config.resolved_proxy_raw_dir(),
        PathBuf::from("/tmp/codex-retention/proxy_raw_payloads")
    );
}

#[test]
pub(crate) fn store_raw_payload_file_anchors_relative_dir_to_database_parent() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let temp_dir = make_temp_test_dir("proxy-raw-store-db-parent");
    let cwd = temp_dir.join("cwd");
    let db_root = temp_dir.join("db-root");
    fs::create_dir_all(&cwd).expect("create cwd dir");
    fs::create_dir_all(&db_root).expect("create db root");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd);

    let mut config = test_config();
    config.database_path = db_root.join("codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let meta = runtime.block_on(store_raw_payload_file(
        &config,
        "proxy-test",
        "request",
        Bytes::from_static(b"{\"ok\":true}"),
    ));
    let expected = db_root.join("proxy_raw_payloads/proxy-test-request.bin.zst");

    assert_eq!(
        meta.path.as_deref(),
        Some(expected.to_string_lossy().as_ref())
    );
    assert!(
        expected.exists(),
        "raw payload should be written beside the database"
    );
    assert!(
        !cwd.join("proxy_raw_payloads/proxy-test-request.bin.zst")
            .exists(),
        "raw payload should not follow the current working directory"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn store_raw_payload_file_born_gzips_large_payloads_when_threshold_is_reached() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let temp_dir = make_temp_test_dir("proxy-raw-store-born-gzip");
    let db_root = temp_dir.join("db-root");
    fs::create_dir_all(&db_root).expect("create db root");

    let mut config = test_config();
    config.database_path = db_root.join("codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");
    config.proxy_raw_compression = RawCompressionCodec::Gzip;
    config.proxy_raw_immediate_gzip_bytes = Some(16);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");
    let payload = br#"{"large":"payload-that-should-be-gzipped"}"#;
    let meta = runtime.block_on(store_raw_payload_file(
        &config,
        "proxy-born-gzip",
        "request",
        Bytes::from_static(payload),
    ));

    let path = PathBuf::from(meta.path.expect("born-gzip path"));
    assert!(path.ends_with("proxy-born-gzip-request.bin.gz"));
    assert!(path.exists(), "gzip raw payload should be written");
    let decoded = read_proxy_raw_bytes(path.to_string_lossy().as_ref(), None)
        .expect("read born-gzip payload back");
    assert_eq!(decoded, payload);

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn store_raw_payload_file_born_zstds_identity_payloads() {
    let temp_dir = make_temp_test_dir("proxy-raw-store-born-zstd");
    let mut config = test_config();
    config.database_path = temp_dir.join("codex_vibe_monitor.db");
    config.proxy_raw_compression = RawCompressionCodec::Zstd;
    let payload = br#"{"identity":"stored-with-zstd"}"#;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");
    let meta = runtime.block_on(store_raw_payload_file(
        &config,
        "proxy-born-zstd",
        "response",
        Bytes::from_static(payload),
    ));
    let path = PathBuf::from(meta.path.expect("zstd path"));
    assert!(path.ends_with("proxy-born-zstd-response.bin.zst"));
    assert_eq!(
        read_proxy_raw_bytes(path.to_string_lossy().as_ref(), None).expect("read zstd raw"),
        payload
    );
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn write_streaming_raw_payload_to_file_born_gzips_large_streams() {
    let temp_dir = make_temp_test_dir("proxy-stream-born-gzip");
    let raw_path = temp_dir.join("stream-response.bin");
    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();
    tx.send(Bytes::from_static(b"hello-"))
        .expect("send first chunk");
    tx.send(Bytes::from_static(b"world-large"))
        .expect("send second chunk");
    drop(tx);

    let meta = write_streaming_raw_payload_to_file(
        raw_path.clone(),
        None,
        Some(8),
        RawCompressionCodec::Gzip,
        &mut rx,
    )
    .await;

    let stored_path = PathBuf::from(meta.path.expect("streaming born-gzip path"));
    assert!(stored_path.ends_with("stream-response.bin.gz"));
    let decoded = read_proxy_raw_bytes(stored_path.to_string_lossy().as_ref(), None)
        .expect("read streaming born-gzip payload");
    assert_eq!(decoded, b"hello-world-large");

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn direct_streaming_raw_payload_writer_respects_gzip_immediate_threshold() {
    let temp_dir = make_temp_test_dir("proxy-direct-stream-gzip-threshold");
    let raw_path = temp_dir.join("stream-response.bin");
    let (tx, rx) = std::sync::mpsc::sync_channel::<Bytes>(2);
    tx.send(Bytes::from_static(b"small"))
        .expect("send small chunk");
    drop(tx);

    let meta = write_direct_streaming_raw_payload_to_file(
        raw_path.clone(),
        None,
        Some(16),
        RawCompressionCodec::Gzip,
        rx,
    );
    let stored_path = PathBuf::from(meta.path.expect("plaintext direct stream path"));
    assert_eq!(stored_path, raw_path);
    assert_eq!(
        read_proxy_raw_bytes(stored_path.to_string_lossy().as_ref(), None)
            .expect("read plaintext direct stream"),
        b"small"
    );

    let compressed_path = temp_dir.join("stream-response-compressed.bin");
    let (tx, rx) = std::sync::mpsc::sync_channel::<Bytes>(2);
    tx.send(Bytes::from_static(b"large"))
        .expect("send large chunk");
    drop(tx);
    let meta = write_direct_streaming_raw_payload_to_file(
        compressed_path.clone(),
        None,
        Some(4),
        RawCompressionCodec::Gzip,
        rx,
    );
    let stored_path = PathBuf::from(meta.path.expect("gzip direct stream path"));
    assert!(stored_path.ends_with("stream-response-compressed.bin.gz"));
    assert_eq!(
        read_proxy_raw_bytes(stored_path.to_string_lossy().as_ref(), None)
            .expect("read gzip direct stream"),
        b"large"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(target_os = "linux")]
#[tokio::test]
pub(crate) async fn write_streaming_raw_payload_to_file_removes_plain_path_after_write_failure() {
    use std::os::unix::fs::symlink;

    let temp_dir = make_temp_test_dir("proxy-stream-write-failure-plain");
    let raw_path = temp_dir.join("stream-response.bin");
    symlink("/dev/full", &raw_path).expect("symlink plain raw path to /dev/full");

    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();
    tx.send(Bytes::from_static(b"hello-world"))
        .expect("send chunk");
    drop(tx);

    let meta = write_streaming_raw_payload_to_file(
        raw_path.clone(),
        None,
        None,
        RawCompressionCodec::None,
        &mut rx,
    )
    .await;

    assert!(meta.path.is_none(), "failed write must not keep raw path");
    assert!(meta.truncated, "failed write should mark payload truncated");
    assert!(
        meta.truncated_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("write_failed")),
        "failed write should surface write_failed reason"
    );
    assert!(
        !raw_path.exists(),
        "failed write should clean up the partially written plain path"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(target_os = "linux")]
#[tokio::test]
pub(crate) async fn write_streaming_raw_payload_to_file_removes_gzip_path_after_write_failure() {
    use std::os::unix::fs::symlink;

    let temp_dir = make_temp_test_dir("proxy-stream-write-failure-gzip");
    let raw_path = temp_dir.join("stream-response.bin");
    let gzip_path = temp_dir.join("stream-response.bin.gz");
    symlink("/dev/full", &gzip_path).expect("symlink gzip raw path to /dev/full");

    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();
    tx.send(Bytes::from_static(b"hello-world"))
        .expect("send chunk");
    drop(tx);

    let meta = write_streaming_raw_payload_to_file(
        raw_path,
        None,
        Some(1),
        RawCompressionCodec::Gzip,
        &mut rx,
    )
    .await;

    assert!(meta.path.is_none(), "failed write must not keep raw path");
    assert!(meta.truncated, "failed write should mark payload truncated");
    assert!(
        meta.truncated_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("write_failed")),
        "failed write should surface write_failed reason"
    );
    assert!(
        !gzip_path.exists(),
        "failed write should clean up the partially written gzip path"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn read_proxy_raw_bytes_keeps_current_dir_compat_for_legacy_relative_paths() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let temp_dir = make_temp_test_dir("proxy-raw-read-legacy-cwd");
    let cwd = temp_dir.join("cwd");
    let fallback_root = temp_dir.join("fallback");
    let relative_path = PathBuf::from("proxy_raw_payloads/legacy-request.bin");
    let cwd_path = cwd.join(&relative_path);
    let fallback_path = fallback_root.join(&relative_path);
    fs::create_dir_all(cwd_path.parent().expect("cwd parent")).expect("create cwd raw dir");
    fs::create_dir_all(fallback_path.parent().expect("fallback parent"))
        .expect("create fallback raw dir");
    fs::write(&cwd_path, b"cwd-copy").expect("write cwd raw file");
    fs::write(&fallback_path, b"fallback-copy").expect("write fallback raw file");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd);

    let raw = read_proxy_raw_bytes(
        relative_path.to_str().expect("utf-8 path"),
        Some(&fallback_root),
    )
    .expect("read legacy cwd-relative raw file");

    assert_eq!(raw, b"cwd-copy");
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn read_proxy_raw_bytes_transparently_decompresses_gzip_files() {
    let temp_dir = make_temp_test_dir("proxy-raw-read-gzip");
    let raw_path = temp_dir.join("request.bin.gz");
    write_gzip_test_file(&raw_path, b"{\"hello\":\"gzip\"}");

    let raw = read_proxy_raw_bytes(raw_path.to_str().expect("utf-8 path"), None)
        .expect("read gzip raw payload");

    assert_eq!(raw, b"{\"hello\":\"gzip\"}");
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn read_proxy_raw_bytes_keeps_plain_bin_payloads_that_start_with_gzip_magic() {
    let temp_dir = make_temp_test_dir("proxy-raw-read-bin-gzip-magic");
    let raw_path = temp_dir.join("request.bin");
    let bytes = vec![0x1f, 0x8b, b'n', b'o', b't', b'-', b'g', b'z'];
    fs::write(&raw_path, &bytes).expect("write plain raw payload");

    let raw = read_proxy_raw_bytes(raw_path.to_str().expect("utf-8 path"), None)
        .expect("read plain raw payload");

    assert_eq!(raw, bytes);
    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn search_raw_script_matches_plain_and_gzip_files() {
    let temp_dir = make_temp_test_dir("search-raw-script");
    let root = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&root).expect("create raw root");
    let plain_path = root.join("plain.bin");
    let gzip_path = root.join("cold.bin.gz");
    fs::write(&plain_path, b"line-1\nshared-token\n").expect("write plain raw");
    write_gzip_test_file(&gzip_path, b"line-a\nshared-token\n");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("shared-token")
    .output()
    .expect("run search-raw script");

    assert!(
        output.status.success(),
        "search-raw should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("search-raw stdout");
    assert!(
        stdout.contains(&format!("{}:2:shared-token", plain_path.display())),
        "plain raw file should match, got: {stdout}"
    );
    assert!(
        stdout.contains(&format!("{}:2:shared-token", gzip_path.display())),
        "gzip raw file should match, got: {stdout}"
    );

    let miss_output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("absent-token")
    .output()
    .expect("run search-raw miss case");
    assert_eq!(
        miss_output.status.code(),
        Some(1),
        "search-raw should return 1 when no file matches"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn search_raw_script_resolves_root_from_database_and_proxy_envs() {
    let temp_dir = make_temp_test_dir("search-raw-script-env-root");
    let db_root = temp_dir.join("db");
    let db_path = db_root.join("codex_vibe_monitor.db");
    let raw_root = db_root.join("proxy_raw_payloads");
    fs::create_dir_all(&raw_root).expect("create resolved raw root");
    fs::write(&db_path, "").expect("create db file");
    let plain_path = raw_root.join("resolved.bin");
    fs::write(&plain_path, b"resolved-token\n").expect("write resolved raw");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .env("DATABASE_PATH", &db_path)
    .env("PROXY_RAW_DIR", "proxy_raw_payloads")
    .arg("resolved-token")
    .output()
    .expect("run search-raw with env-derived root");

    assert!(
        output.status.success(),
        "search-raw should resolve root from envs: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("search-raw stdout");
    assert!(
        stdout.contains(&format!("{}:1:resolved-token", plain_path.display())),
        "env-derived root should find the plain file, got: {stdout}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn search_raw_script_reports_missing_root_as_configuration_error() {
    let temp_dir = make_temp_test_dir("search-raw-script-missing-root");
    let db_path = temp_dir.join("missing/codex_vibe_monitor.db");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .env("DATABASE_PATH", &db_path)
    .env("PROXY_RAW_DIR", "proxy_raw_payloads")
    .arg("anything")
    .output()
    .expect("run search-raw with missing root");

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing root should be treated as configuration error"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("root directory not found"),
        "missing root should explain the configuration error, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

use super::*;
