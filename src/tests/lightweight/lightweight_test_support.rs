use super::*;

pub(crate) fn test_runtime_path(name: &str) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"))
        .join(name)
}

pub(crate) fn test_config() -> AppConfig {
    AppConfig {
        openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
        database_path: PathBuf::from(":memory:"),
        analytics_backend: AnalyticsBackend::Sqlite,
        postgres_database_url: None,
        postgres_max_connections: DEFAULT_POSTGRES_MAX_CONNECTIONS as u32,
        observability: ObservabilityConfig::default(),
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
        public_metrics_cors_allowed_origins: vec![
            "https://ivanli.cc".to_string(),
            "http://127.0.0.1:12620".to_string(),
        ],
        list_limit_max: 100,
        user_agent: "codex-test".to_string(),
        static_dir: None,
        public_origin: None,
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

pub(crate) fn test_sqlite_url_for_path(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy())
}

pub(crate) async fn retention_test_pool_and_config(
    prefix: &str,
) -> (SqlitePool, AppConfig, PathBuf) {
    let schema_template = std::env::var_os(ARCHIVE_SCHEMA_TEMPLATE_PATH_ENV).map(PathBuf::from);
    retention_test_pool_and_config_from_template(prefix, schema_template.as_deref()).await
}

pub(crate) async fn retention_fresh_schema_test_pool_and_config(
    prefix: &str,
) -> (SqlitePool, AppConfig, PathBuf) {
    retention_test_pool_and_config_with_connections(prefix, None, 1).await
}

async fn retention_test_pool_and_config_from_template(
    prefix: &str,
    schema_template: Option<&Path>,
) -> (SqlitePool, AppConfig, PathBuf) {
    retention_test_pool_and_config_with_connections(prefix, schema_template, 2).await
}

async fn retention_test_pool_and_config_with_connections(
    prefix: &str,
    schema_template: Option<&Path>,
    max_connections: u32,
) -> (SqlitePool, AppConfig, PathBuf) {
    let temp_dir = make_temp_test_dir(prefix);
    let db_path = temp_dir.join("codex-vibe-monitor.db");
    let used_schema_template = schema_template.is_some();
    if let Some(schema_template) = schema_template {
        assert!(
            schema_template.is_file(),
            "archive schema template must exist: {}",
            schema_template.display()
        );
        fs::copy(schema_template, &db_path).expect("copy archive current-schema template");
    } else {
        fs::File::create(&db_path).expect("create retention sqlite file");
    }
    let db_url = test_sqlite_url_for_path(&db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect(&db_url)
        .await
        .expect("connect retention sqlite");
    if !used_schema_template {
        ensure_schema(&pool).await.expect("ensure retention schema");
    }

    let mut config = test_config();
    config.database_path = db_path;
    config.proxy_raw_dir = temp_dir.join("proxy_raw_payloads");
    config.archive_dir = temp_dir.join("archives");
    config.retention_batch_rows = 2;
    config.invocation_archive_ttl_days = 365;
    fs::create_dir_all(&config.proxy_raw_dir).expect("create retention raw dir");
    fs::create_dir_all(&config.archive_dir).expect("create retention archive dir");
    (pool, config, temp_dir)
}

#[tokio::test]
async fn retention_file_fixture_copies_current_schema_template_without_leaking_mutations() {
    let template_dir = make_temp_test_dir("retention-current-schema-template");
    let template_path = template_dir.join("current-schema.db");
    write_current_schema_template(&template_path)
        .await
        .expect("build current-schema template");

    let (first_pool, _first_config, first_dir) = retention_test_pool_and_config_from_template(
        "retention-template-first",
        Some(&template_path),
    )
    .await;
    sqlx::query("CREATE TABLE retention_template_isolation (id INTEGER PRIMARY KEY)")
        .execute(&first_pool)
        .await
        .expect("mutate first template copy");
    first_pool.close().await;

    let (second_pool, _second_config, second_dir) = retention_test_pool_and_config_from_template(
        "retention-template-second",
        Some(&template_path),
    )
    .await;
    let leaked_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'retention_template_isolation'",
    )
    .fetch_one(&second_pool)
    .await
    .expect("inspect second template copy");
    assert_eq!(leaked_table_count, 0, "template copies must stay isolated");
    second_pool.close().await;

    cleanup_temp_test_dir(&first_dir);
    cleanup_temp_test_dir(&second_dir);
    cleanup_temp_test_dir(&template_dir);
}

#[tokio::test]
async fn retention_fresh_schema_fixture_reapplies_schema_without_pooled_ddl_races() {
    let (pool, _config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-fresh-schema-reapply").await;

    ensure_schema(&pool)
        .await
        .expect("reapply fresh retention schema");

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

pub(crate) async fn retention_memory_test_pool_and_config(
    prefix: &str,
) -> (SqlitePool, AppConfig, PathBuf) {
    let temp_dir = make_temp_test_dir(prefix);
    let db_path = temp_dir.join("codex-vibe-monitor.db");
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url = format!("sqlite:file:{prefix}-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .expect("connect retention in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("ensure retention in-memory schema");

    let mut config = test_config();
    config.database_path = db_path;
    config.proxy_raw_dir = temp_dir.join("proxy_raw_payloads");
    config.archive_dir = temp_dir.join("archives");
    config.retention_batch_rows = 2;
    config.invocation_archive_ttl_days = 365;
    fs::create_dir_all(&config.proxy_raw_dir).expect("create retention raw dir");
    fs::create_dir_all(&config.archive_dir).expect("create retention archive dir");
    (pool, config, temp_dir)
}

pub(crate) fn cleanup_temp_test_dir(path: &Path) {
    let _timing = test_timing_span("cleanup");
    let _ = fs::remove_dir_all(path);
}

#[cfg(unix)]
pub(crate) fn current_process_rss_kib() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

pub(crate) fn shanghai_local_days_ago(days: i64, hour: u32, minute: u32, second: u32) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai);
    let naive = (now_local.date_naive() - ChronoDuration::days(days))
        .and_hms_opt(hour, minute, second)
        .expect("valid shanghai local time");
    format_naive(naive)
}

pub(crate) fn shanghai_local_now_minus_secs(secs: i64) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai).naive_local();
    format_naive(now_local - ChronoDuration::seconds(secs))
}

pub(crate) fn utc_naive_from_shanghai_local_days_ago(
    days: i64,
    hour: u32,
    minute: u32,
    second: u32,
) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai);
    let local_naive = (now_local.date_naive() - ChronoDuration::days(days))
        .and_hms_opt(hour, minute, second)
        .expect("valid shanghai local time");
    format_naive(local_naive_to_utc(local_naive, Shanghai).naive_utc())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_retention_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    source: &str,
    status: &str,
    payload: Option<&str>,
    raw_response: &str,
    request_raw_path: Option<&Path>,
    response_raw_path: Option<&Path>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            status,
            payload,
            raw_response,
            request_raw_path,
            request_raw_codec,
            request_raw_size,
            response_raw_path,
            response_raw_codec,
            response_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(source)
    .bind(Some("gpt-5.2-codex"))
    .bind(Some(12_i64))
    .bind(Some(3_i64))
    .bind(Some(0_i64))
    .bind(Some(0_i64))
    .bind(total_tokens)
    .bind(cost)
    .bind(status)
    .bind(payload)
    .bind(raw_response)
    .bind(request_raw_path.map(|path| path.to_string_lossy().to_string()))
    .bind(raw_codec_from_path(
        request_raw_path
            .map(|path| path.to_string_lossy().to_string())
            .as_deref(),
    ))
    .bind(
        request_raw_path
            .and_then(|path| fs::metadata(path).ok())
            .map(|meta| meta.len() as i64),
    )
    .bind(response_raw_path.map(|path| path.to_string_lossy().to_string()))
    .bind(raw_codec_from_path(
        response_raw_path
            .map(|path| path.to_string_lossy().to_string())
            .as_deref(),
    ))
    .bind(
        response_raw_path
            .and_then(|path| fs::metadata(path).ok())
            .map(|meta| meta.len() as i64),
    )
    .execute(pool)
    .await
    .expect("insert retention invocation");
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_retention_pool_upstream_request_attempt(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    upstream_account_id: Option<i64>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: &str,
    http_status: Option<i64>,
    failure_kind: Option<&str>,
    started_at: Option<&str>,
    finished_at: Option<&str>,
) {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            http_status,
            failure_kind,
            error_message,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
        )
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind("/v1/responses")
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(Some("sticky-retention"))
    .bind(upstream_account_id)
    .bind(None::<String>)
    .bind(attempt_index)
    .bind(distinct_account_index)
    .bind(same_account_retry_index)
    .bind(Some("203.0.113.1"))
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(http_status)
    .bind(failure_kind)
    .bind(Some("retention test"))
    .bind(Some(12.5_f64))
    .bind(Some(6.2_f64))
    .bind(Some(30.0_f64))
    .bind(Some("req_retention"))
    .execute(pool)
    .await
    .expect("insert retention pool attempt");
}
