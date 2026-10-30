use super::*;

#[test]
fn classify_invocation_failure_marks_invalid_key_as_client_failure() {
    let result = classify_invocation_failure(Some("http_401"), Some("Invalid API key format"));
    assert_eq!(result.failure_class, FailureClass::ClientFailure);
    assert!(!result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("invalid_api_key"));
}

#[test]
fn classify_invocation_failure_marks_upstream_errors_as_service_failure() {
    let result = classify_invocation_failure(
        Some("http_502"),
        Some(
            "[failed_contact_upstream] failed to contact upstream: error sending request for url (https://example.com/v1/responses)",
        ),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(
        result.failure_kind.as_deref(),
        Some("failed_contact_upstream")
    );
}

#[test]
fn classify_invocation_failure_treats_running_and_pending_as_none() {
    for status in ["running", "pending"] {
        let result = classify_invocation_failure(Some(status), None);
        assert_eq!(result.failure_class, FailureClass::None);
        assert!(!result.is_actionable);
        assert_eq!(result.failure_kind, None);
    }
}

#[test]
fn classify_invocation_failure_marks_upstream_response_failed_as_service_failure() {
    let result = classify_invocation_failure(
        Some("http_200"),
        Some(
            "[upstream_response_failed] server_error: An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message.",
        ),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(
        result.failure_kind.as_deref(),
        Some("upstream_response_failed")
    );
}

#[test]
fn classify_invocation_failure_marks_http_429_as_service_failure() {
    let result = classify_invocation_failure(Some("http_429"), Some("rate limited"));
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("http_429"));
}

#[test]
fn resolve_failure_classification_recomputes_actionable_for_missing_legacy_class() {
    let result = resolve_failure_classification(
        Some("http_502"),
        Some("[failed_contact_upstream] upstream unavailable"),
        None,
        None,
        Some(0),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
}

#[test]
fn resolve_failure_classification_overrides_legacy_default_none_for_failures() {
    let result = resolve_failure_classification(
        Some("http_502"),
        Some("[failed_contact_upstream] upstream unavailable"),
        None,
        Some(FailureClass::None.as_str()),
        Some(0),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(
        result.failure_kind.as_deref(),
        Some("failed_contact_upstream")
    );
}

#[test]
fn failure_scope_parse_defaults_to_service() {
    assert_eq!(
        FailureScope::parse(None).expect("default scope"),
        FailureScope::Service
    );
}

#[test]
fn failure_scope_parse_rejects_unknown_value() {
    let err = FailureScope::parse(Some("unexpected")).expect_err("invalid scope should fail");
    match err {
        ApiError::BadRequest(err) => {
            assert!(
                err.to_string()
                    .contains("unsupported failure scope: unexpected"),
                "error should mention rejected scope"
            );
        }
        other => panic!("expected BadRequest, got: {other:?}"),
    }
}

#[test]
fn app_config_from_sources_ignores_removed_xyai_env_vars() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let cases = [
        ("XY_BASE_URL", "not-a-valid-url"),
        ("XY_VIBE_QUOTA_ENDPOINT", "%%%"),
        ("XY_SESSION_COOKIE_NAME", "legacy-cookie"),
        ("XY_SESSION_COOKIE_VALUE", "legacy-secret"),
        ("XY_LEGACY_POLL_ENABLED", "definitely-not-bool"),
        ("XY_SNAPSHOT_MIN_INTERVAL_SECS", "not-a-number"),
    ];
    let previous = cases
        .iter()
        .map(|(name, _)| ((*name).to_string(), env::var_os(name)))
        .collect::<Vec<_>>();

    for (name, value) in cases {
        unsafe { env::set_var(name, value) };
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { env::set_var(name, value) },
            None => unsafe { env::remove_var(name) },
        }
    }

    let config = result.expect("removed XYAI env vars should be ignored");
    assert_eq!(config.database_path, PathBuf::from("codex_vibe_monitor.db"));
}

#[test]
fn app_config_from_sources_reads_database_path_env() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let previous_database = env::var_os(ENV_DATABASE_PATH);
    let previous_legacy = env::var_os(LEGACY_ENV_DATABASE_PATH);

    unsafe {
        env::remove_var(LEGACY_ENV_DATABASE_PATH);
        env::set_var(ENV_DATABASE_PATH, "/tmp/codex-env.sqlite");
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    match previous_database {
        Some(value) => unsafe { env::set_var(ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(ENV_DATABASE_PATH) },
    }
    match previous_legacy {
        Some(value) => unsafe { env::set_var(LEGACY_ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(LEGACY_ENV_DATABASE_PATH) },
    }

    let config = result.expect("DATABASE_PATH should configure the database path");
    assert_eq!(config.database_path, PathBuf::from("/tmp/codex-env.sqlite"));
}

#[test]
fn app_config_from_sources_reads_observability_envs() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let previous = env::var_os("OBSERVABILITY_ENABLED");
    unsafe {
        env::set_var("OBSERVABILITY_ENABLED", "false");
    }
    let result = AppConfig::from_sources(&CliArgs::default());
    match previous {
        Some(value) => unsafe { env::set_var("OBSERVABILITY_ENABLED", value) },
        None => unsafe { env::remove_var("OBSERVABILITY_ENABLED") },
    }
    assert!(
        !result
            .expect("observability configuration")
            .observability
            .enabled
    );
}

#[test]
fn startup_pending_attempt_recovery_skips_all_retention_run_once_modes() {
    let mut cli = CliArgs::default();
    assert!(should_recover_pending_pool_attempts_on_startup(&cli));

    cli.command = Some(CliCommand::Maintenance(MaintenanceCliArgs {
        command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: false }),
    }));
    assert!(!should_recover_pending_pool_attempts_on_startup(&cli));

    cli.command = None;
    cli.retention_run_once = true;
    assert!(!should_recover_pending_pool_attempts_on_startup(&cli));

    cli.retention_dry_run = true;
    assert!(!should_recover_pending_pool_attempts_on_startup(&cli));
}

#[test]
fn app_config_from_sources_rejects_legacy_database_path_env() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let previous_database = env::var_os(ENV_DATABASE_PATH);
    let previous_legacy = env::var_os(LEGACY_ENV_DATABASE_PATH);

    unsafe {
        env::set_var(ENV_DATABASE_PATH, "/tmp/codex-env.sqlite");
        env::set_var(LEGACY_ENV_DATABASE_PATH, "/tmp/codex-legacy.sqlite");
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    match previous_database {
        Some(value) => unsafe { env::set_var(ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(ENV_DATABASE_PATH) },
    }
    match previous_legacy {
        Some(value) => unsafe { env::set_var(LEGACY_ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(LEGACY_ENV_DATABASE_PATH) },
    }

    let err = result.expect_err("legacy database env should fail fast");
    assert!(
        err.to_string()
            .contains("XY_DATABASE_PATH is not supported; rename it to DATABASE_PATH"),
        "error should point to the DATABASE_PATH migration"
    );
}

#[test]
fn app_config_from_sources_reads_renamed_public_envs() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let mut cases = LEGACY_ENV_RENAMES
        .iter()
        .map(|(legacy, _)| (*legacy, None))
        .collect::<Vec<_>>();
    cases.extend([
        (ENV_POLL_INTERVAL_SECS, Some("11")),
        (ENV_REQUEST_TIMEOUT_SECS, Some("61")),
        (ENV_XRAY_BINARY, Some("/usr/local/bin/xray-custom")),
        (ENV_XRAY_RUNTIME_DIR, Some("/tmp/xray-runtime")),
        (ENV_MAX_PARALLEL_POLLS, Some("7")),
        (ENV_SHARED_CONNECTION_PARALLELISM, Some("3")),
        (ENV_HTTP_BIND, Some("127.0.0.1:39090")),
        (
            ENV_CORS_ALLOWED_ORIGINS,
            Some("https://app.example.com, http://localhost:5173"),
        ),
        (ENV_LIST_LIMIT_MAX, Some("321")),
        (ENV_USER_AGENT, Some("custom-agent/1.0")),
        (ENV_STATIC_DIR, Some("/tmp/static")),
        (ENV_RETENTION_DRY_RUN, Some("true")),
        (ENV_RETENTION_INTERVAL_SECS, Some("7200")),
        (ENV_RETENTION_BATCH_ROWS, Some("2222")),
        (ENV_ARCHIVE_DIR, Some("/tmp/archive")),
        (ENV_INVOCATION_SUCCESS_FULL_DAYS, Some("31")),
        (ENV_INVOCATION_MAX_DAYS, Some("91")),
        (ENV_CODEX_INVOCATION_ARCHIVE_LAYOUT, Some("segment_v1")),
        (
            ENV_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY,
            Some("day"),
        ),
        (ENV_INVOCATION_ARCHIVE_CODEC, Some("gzip")),
        (ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS, Some("32")),
        (ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS, Some("7")),
        (
            ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
            Some("30"),
        ),
        (ENV_QUOTA_SNAPSHOT_FULL_DAYS, Some("34")),
        (ENV_PROXY_RAW_COMPRESSION, Some("none")),
        (ENV_PROXY_RAW_IMMEDIATE_GZIP_BYTES, Some("2097152")),
        (ENV_PROXY_RAW_HOT_SECS, Some("1234")),
        (ENV_FORWARD_PROXY_ALGO, Some("v2")),
    ]);
    let _env = EnvVarGuard::set(&cases);

    let config =
        AppConfig::from_sources(&CliArgs::default()).expect("renamed public envs should parse");

    assert_eq!(config.poll_interval, Duration::from_secs(11));
    assert_eq!(config.request_timeout, Duration::from_secs(61));
    assert_eq!(config.xray_binary, "/usr/local/bin/xray-custom");
    assert_eq!(config.xray_runtime_dir, PathBuf::from("/tmp/xray-runtime"));
    assert_eq!(config.forward_proxy_algo, ForwardProxyAlgo::V2);
    assert_eq!(config.max_parallel_polls, 7);
    assert_eq!(config.shared_connection_parallelism, 3);
    assert_eq!(
        config.http_bind,
        "127.0.0.1:39090".parse().expect("valid socket address")
    );
    assert_eq!(
        config.cors_allowed_origins,
        vec![
            "https://app.example.com".to_string(),
            "http://localhost:5173".to_string(),
        ]
    );
    assert_eq!(config.list_limit_max, 321);
    assert_eq!(config.user_agent, "custom-agent/1.0");
    assert_eq!(config.static_dir, Some(PathBuf::from("/tmp/static")));
    assert!(config.retention_dry_run);
    assert_eq!(config.retention_interval, Duration::from_secs(7200));
    assert_eq!(config.retention_batch_rows, 2222);
    assert_eq!(config.archive_dir, PathBuf::from("/tmp/archive"));
    assert_eq!(config.invocation_success_full_days, 31);
    assert_eq!(config.invocation_max_days, 91);
    assert_eq!(
        config.codex_invocation_archive_layout,
        ArchiveBatchLayout::SegmentV1
    );
    assert_eq!(
        config.codex_invocation_archive_segment_granularity,
        ArchiveSegmentGranularity::Day
    );
    assert_eq!(config.invocation_archive_codec, ArchiveFileCodec::Gzip);
    assert_eq!(config.forward_proxy_attempts_retention_days, 32);
    assert_eq!(config.pool_upstream_request_attempts_retention_days, 7);
    assert_eq!(config.pool_upstream_request_attempts_archive_ttl_days, 30);
    assert_eq!(
        config.pool_upstream_responses_attempt_timeout,
        Duration::from_secs(DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS)
    );
    assert_eq!(config.quota_snapshot_full_days, 34);
    assert_eq!(config.proxy_raw_compression, RawCompressionCodec::None);
    assert_eq!(config.proxy_raw_immediate_gzip_bytes, Some(2 * 1024 * 1024));
    assert_eq!(config.proxy_raw_hot_secs, 1234);
}

#[test]
fn app_config_from_sources_rejects_all_legacy_public_env_renames() {
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
fn app_config_from_sources_accepts_kaisoumail_base_url_and_api_key_only() {
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
fn app_config_from_sources_rejects_kaisoumail_default_generation_env_vars() {
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
fn app_config_from_sources_uses_proxy_timeout_defaults() {
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
fn app_config_from_sources_reads_proxy_timeout_envs() {
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
fn app_config_from_sources_rejects_zero_pool_upstream_responses_attempt_timeout() {
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
fn app_config_from_sources_rejects_zero_pool_upstream_responses_total_timeout() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[(ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS, Some("0"))]);

    let err = AppConfig::from_sources(&CliArgs::default())
        .expect_err("zero responses total timeout should be rejected");
    assert_eq!(
        err.to_string(),
        format!("{ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS} must be greater than 0")
    );
}

#[tokio::test]
async fn ensure_schema_steady_state_does_not_rebuild_large_derived_invocation_state() {
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

#[tokio::test]
async fn invocation_records_request_id_filter_uses_normalized_expression_index() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema test pool");
    ensure_schema(&pool).await.expect("ensure schema");

    let (index_sql, marker): (Option<String>, i64) = sqlx::query_as(
        "SELECT (SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_codex_invocations_invoke_id_filter_id'), (SELECT COUNT(*) FROM schema_refresh_migrations WHERE migration_name = 'invoke_id_filter_expression_index_v1')",
    )
    .fetch_one(&pool)
    .await
    .expect("load invoke_id expression index migration state");
    let index_sql = index_sql.expect("normalized invoke_id expression index exists");
    assert!(index_sql.contains("LOWER(TRIM(COALESCE(invoke_id, '')))"));
    assert_eq!(marker, 1, "expression index migration is recorded once");

    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, raw_response) VALUES ('  MiXeD-Request  ', datetime('now'), 'cli', 'success', '{}'), ('other-request', datetime('now'), 'cli', 'success', '{}')",
    )
    .execute(&pool)
    .await
    .expect("seed request ids");
    let matches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE LOWER(TRIM(COALESCE(invoke_id, ''))) = ?1 AND id <= ?2",
    )
    .bind("mixed-request")
    .bind(i64::MAX)
    .fetch_one(&pool)
    .await
    .expect("run normalized request id query");
    assert_eq!(
        matches, 1,
        "request id matching remains case/space insensitive"
    );

    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT id FROM codex_invocations WHERE LOWER(TRIM(COALESCE(invoke_id, ''))) = ?1 AND id <= ?2 ORDER BY id",
    )
    .bind("mixed-request")
    .bind(i64::MAX)
    .fetch_all(&pool)
    .await
    .expect("explain normalized request id query");
    assert!(
        plan.iter()
            .any(|(_, _, _, detail)| detail.contains("idx_codex_invocations_invoke_id_filter_id")),
        "bounded normalized request id lookup must use the expression index: {plan:?}"
    );
    pool.close().await;
}

#[tokio::test]
async fn ensure_schema_adds_invoke_id_filter_expression_index() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema test pool");
    ensure_schema(&pool).await.expect("ensure schema");
    let index_exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'idx_codex_invocations_invoke_id_filter_id')",
    )
    .fetch_one(&pool)
    .await
    .expect("check normalized invoke_id expression index");
    assert_eq!(index_exists, 1);
    pool.close().await;
}

#[tokio::test]
async fn invocation_detail_retention_uses_its_ordering_index() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open schema test pool");
    ensure_schema(&pool).await.expect("ensure schema");

    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT id FROM codex_invocations WHERE (LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', 'warning') OR (LOWER(TRIM(COALESCE(status, ''))) = 'http_200' AND TRIM(COALESCE(error_message, '')) = '')) AND detail_level = ?1 AND occurred_at < ?2 AND occurred_at >= ?3 ORDER BY occurred_at ASC, id ASC LIMIT ?4",
    )
    .bind("full")
    .bind("2026-08-01 00:00:00")
    .bind("2026-05-01 00:00:00")
    .bind(2_i64)
    .fetch_all(&pool)
    .await
    .expect("explain invocation detail retention query");
    assert!(
        plan.iter().any(|(_, _, _, detail)| {
            detail.contains("idx_codex_invocations_detail_level_occurred_at")
        }),
        "invocation detail retention must use its bounded ordering index: {plan:?}"
    );
    pool.close().await;
}

#[test]
fn app_config_ignores_retired_retention_switches_for_all_values() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    for name in ["RETENTION_ENABLED", "XY_RETENTION_ENABLED"] {
        let previous = env::var_os(name);
        for value in ["true", "false", "invalid-retired-value"] {
            unsafe {
                env::set_var(name, value);
            }
            AppConfig::from_sources(&CliArgs::default())
                .expect("retired switches cannot gate or reject configuration");
        }
        match previous {
            Some(value) => unsafe {
                env::set_var(name, value);
            },
            None => unsafe {
                env::remove_var(name);
            },
        }
    }
}
