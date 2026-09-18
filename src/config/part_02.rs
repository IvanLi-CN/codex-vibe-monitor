struct CoreConfigSource {
    openai_upstream_base_url: String,
    database_path: PathBuf,
    poll_interval: Duration,
    request_timeout: Duration,
    pool_upstream_responses_attempt_timeout: Duration,
    pool_upstream_responses_total_timeout: Duration,
}
struct ProxyConfigSource {
    openai_proxy_handshake_timeout: Duration,
    openai_proxy_compact_handshake_timeout: Duration,
    openai_proxy_image_handshake_timeout: Duration,
    openai_proxy_request_read_timeout: Duration,
    openai_proxy_max_request_body_bytes: usize,
    openai_proxy_websocket_enabled: bool,
    openai_proxy_upstream_websocket_default_enabled: bool,
    openai_proxy_encrypted_session_owner_routing_enabled: bool,
    proxy_enforce_stream_include_usage: bool,
    proxy_usage_backfill_on_startup: bool,
    proxy_raw_max_bytes: Option<usize>,
    proxy_raw_dir: PathBuf,
    proxy_raw_compression: RawCompressionCodec,
    proxy_raw_immediate_gzip_bytes: Option<usize>,
    proxy_raw_hot_secs: u64,
    xray_binary: String,
    xray_runtime_dir: PathBuf,
    forward_proxy_algo: ForwardProxyAlgo,
}

struct ProxyConnectionConfigSource {
    openai_proxy_handshake_timeout: Duration,
    openai_proxy_compact_handshake_timeout: Duration,
    openai_proxy_image_handshake_timeout: Duration,
    openai_proxy_request_read_timeout: Duration,
    openai_proxy_max_request_body_bytes: usize,
    openai_proxy_websocket_enabled: bool,
    openai_proxy_upstream_websocket_default_enabled: bool,
    openai_proxy_encrypted_session_owner_routing_enabled: bool,
    proxy_enforce_stream_include_usage: bool,
    proxy_usage_backfill_on_startup: bool,
}

struct ProxyStorageConfigSource {
    proxy_raw_max_bytes: Option<usize>,
    proxy_raw_dir: PathBuf,
    proxy_raw_compression: RawCompressionCodec,
    proxy_raw_immediate_gzip_bytes: Option<usize>,
    proxy_raw_hot_secs: u64,
    xray_binary: String,
    xray_runtime_dir: PathBuf,
    forward_proxy_algo: ForwardProxyAlgo,
}

struct ServerConfigSource {
    max_parallel_polls: usize,
    shared_connection_parallelism: usize,
    http_bind: SocketAddr,
    cors_allowed_origins: Vec<String>,
    list_limit_max: usize,
    user_agent: String,
    public_origin: Option<String>,
    static_dir: Option<PathBuf>,
}

struct RetentionConfigSource {
    retention_enabled: bool,
    retention_dry_run: bool,
    retention_interval: Duration,
    retention_batch_rows: usize,
    retention_catchup_budget: Duration,
    archive_dir: PathBuf,
    codex_invocation_archive_layout: ArchiveBatchLayout,
    codex_invocation_archive_segment_granularity: ArchiveSegmentGranularity,
    invocation_archive_codec: ArchiveFileCodec,
    invocation_success_full_days: u64,
    invocation_max_days: u64,
    invocation_archive_ttl_days: u64,
    forward_proxy_attempts_retention_days: u64,
    pool_upstream_request_attempts_retention_days: u64,
    pool_upstream_request_attempts_archive_ttl_days: u64,
    quota_snapshot_full_days: u64,
    long_term_stats_hourly_retention_days: u64,
}

struct UpstreamAccountsConfigSource {
    upstream_accounts_oauth_client_id: String,
    upstream_accounts_oauth_issuer: Url,
    upstream_accounts_usage_base_url: Url,
    upstream_accounts_login_session_ttl: Duration,
    upstream_accounts_sync_interval: Duration,
    upstream_accounts_refresh_lead_time: Duration,
    upstream_accounts_history_retention_days: u64,
    upstream_accounts_kaisoumail: Option<UpstreamAccountsKaisouMailConfig>,
}

fn resolve_core_config_source(overrides: &CliArgs) -> Result<CoreConfigSource> {
    let openai_upstream_base_url = env::var("OPENAI_UPSTREAM_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_OPENAI_UPSTREAM_BASE_URL.to_string());
    let database_path = overrides
        .database_path
        .clone()
        .or_else(|| env::var(ENV_DATABASE_PATH).ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("codex_vibe_monitor.db"));
    let poll_interval = overrides
        .poll_interval_secs
        .or_else(|| {
            env::var(ENV_POLL_INTERVAL_SECS)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
        })
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(10));
    let request_timeout = overrides
        .request_timeout_secs
        .or_else(|| {
            env::var(ENV_REQUEST_TIMEOUT_SECS)
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
        })
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(60));
    let pool_upstream_responses_attempt_timeout = Duration::from_secs(parse_non_zero_u64_env_var(
        ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
        DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
    )?);
    let pool_upstream_responses_total_timeout = Duration::from_secs(parse_non_zero_u64_env_var(
        ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
        DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
    )?);
    Ok(CoreConfigSource {
        openai_upstream_base_url,
        database_path,
        poll_interval,
        request_timeout,
        pool_upstream_responses_attempt_timeout,
        pool_upstream_responses_total_timeout,
    })
}

fn resolve_proxy_config_source() -> Result<ProxyConfigSource> {
    let connection = resolve_proxy_connection_config_source()?;
    let storage = resolve_proxy_storage_config_source()?;
    Ok(ProxyConfigSource {
        openai_proxy_handshake_timeout: connection.openai_proxy_handshake_timeout,
        openai_proxy_compact_handshake_timeout: connection.openai_proxy_compact_handshake_timeout,
        openai_proxy_image_handshake_timeout: connection.openai_proxy_image_handshake_timeout,
        openai_proxy_request_read_timeout: connection.openai_proxy_request_read_timeout,
        openai_proxy_max_request_body_bytes: connection.openai_proxy_max_request_body_bytes,
        openai_proxy_websocket_enabled: connection.openai_proxy_websocket_enabled,
        openai_proxy_upstream_websocket_default_enabled: connection
            .openai_proxy_upstream_websocket_default_enabled,
        openai_proxy_encrypted_session_owner_routing_enabled: connection
            .openai_proxy_encrypted_session_owner_routing_enabled,
        proxy_enforce_stream_include_usage: connection.proxy_enforce_stream_include_usage,
        proxy_usage_backfill_on_startup: connection.proxy_usage_backfill_on_startup,
        proxy_raw_max_bytes: storage.proxy_raw_max_bytes,
        proxy_raw_dir: storage.proxy_raw_dir,
        proxy_raw_compression: storage.proxy_raw_compression,
        proxy_raw_immediate_gzip_bytes: storage.proxy_raw_immediate_gzip_bytes,
        proxy_raw_hot_secs: storage.proxy_raw_hot_secs,
        xray_binary: storage.xray_binary,
        xray_runtime_dir: storage.xray_runtime_dir,
        forward_proxy_algo: storage.forward_proxy_algo,
    })
}

fn resolve_proxy_connection_config_source() -> Result<ProxyConnectionConfigSource> {
    let openai_proxy_handshake_timeout = env::var("OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS));
    let openai_proxy_compact_handshake_timeout =
        env::var("OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&v| v > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| {
                Duration::from_secs(DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS)
            });
    let openai_proxy_image_handshake_timeout =
        env::var("OPENAI_PROXY_IMAGE_HANDSHAKE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&v| v > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| {
                Duration::from_secs(DEFAULT_OPENAI_PROXY_IMAGE_HANDSHAKE_TIMEOUT_SECS)
            });
    let openai_proxy_request_read_timeout = env::var("OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&v| v > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS));
    let openai_proxy_max_request_body_bytes = env::var("OPENAI_PROXY_MAX_REQUEST_BODY_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES);
    let openai_proxy_websocket_enabled = parse_bool_env_var(
        ENV_OPENAI_PROXY_WEBSOCKET_ENABLED,
        DEFAULT_OPENAI_PROXY_WEBSOCKET_ENABLED,
    )?;
    let openai_proxy_upstream_websocket_default_enabled = parse_bool_env_var(
        ENV_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED,
        DEFAULT_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED,
    )?;
    let openai_proxy_encrypted_session_owner_routing_enabled = parse_bool_env_var(
        ENV_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
        DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
    )?;
    let proxy_enforce_stream_include_usage = parse_bool_env_var(
        "PROXY_ENFORCE_STREAM_INCLUDE_USAGE",
        DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
    )?;
    let proxy_usage_backfill_on_startup = parse_bool_env_var(
        "PROXY_USAGE_BACKFILL_ON_STARTUP",
        DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
    )?;
    Ok(ProxyConnectionConfigSource {
        openai_proxy_handshake_timeout,
        openai_proxy_compact_handshake_timeout,
        openai_proxy_image_handshake_timeout,
        openai_proxy_request_read_timeout,
        openai_proxy_max_request_body_bytes,
        openai_proxy_websocket_enabled,
        openai_proxy_upstream_websocket_default_enabled,
        openai_proxy_encrypted_session_owner_routing_enabled,
        proxy_enforce_stream_include_usage,
        proxy_usage_backfill_on_startup,
    })
}

fn resolve_proxy_storage_config_source() -> Result<ProxyStorageConfigSource> {
    let proxy_raw_max_bytes = match env::var("PROXY_RAW_MAX_BYTES") {
        Ok(value) => {
            let parsed = value
                .parse::<usize>()
                .with_context(|| format!("invalid PROXY_RAW_MAX_BYTES: {value}"))?;
            if parsed == 0 { None } else { Some(parsed) }
        }
        Err(env::VarError::NotPresent) => DEFAULT_PROXY_RAW_MAX_BYTES,
        Err(err) => {
            return Err(anyhow!("failed to read PROXY_RAW_MAX_BYTES: {err}"));
        }
    };
    let proxy_raw_dir = env::var("PROXY_RAW_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_PROXY_RAW_DIR));
    let proxy_raw_compression =
        resolve_raw_compression_codec_config(env::var(ENV_PROXY_RAW_COMPRESSION).ok().as_deref())?;
    let proxy_raw_immediate_gzip_bytes = match env::var(ENV_PROXY_RAW_IMMEDIATE_GZIP_BYTES) {
        Ok(value) => {
            let parsed = value.parse::<usize>().with_context(|| {
                format!("invalid {ENV_PROXY_RAW_IMMEDIATE_GZIP_BYTES}: {value}")
            })?;
            if parsed == 0 { None } else { Some(parsed) }
        }
        Err(env::VarError::NotPresent) => DEFAULT_PROXY_RAW_IMMEDIATE_GZIP_BYTES,
        Err(err) => {
            return Err(anyhow!(
                "failed to read {ENV_PROXY_RAW_IMMEDIATE_GZIP_BYTES}: {err}"
            ));
        }
    };
    let proxy_raw_hot_secs = parse_u64_env_var(ENV_PROXY_RAW_HOT_SECS, DEFAULT_PROXY_RAW_HOT_SECS)?;
    let xray_binary = env::var(ENV_XRAY_BINARY).unwrap_or_else(|_| DEFAULT_XRAY_BINARY.to_string());
    let xray_runtime_dir = env::var(ENV_XRAY_RUNTIME_DIR)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_XRAY_RUNTIME_DIR));
    let forward_proxy_algo_raw = env::var(ENV_FORWARD_PROXY_ALGO).ok();
    let forward_proxy_algo_legacy_raw = env::var(LEGACY_ENV_FORWARD_PROXY_ALGO).ok();
    let forward_proxy_algo = resolve_forward_proxy_algo_config(
        forward_proxy_algo_raw.as_deref(),
        forward_proxy_algo_legacy_raw.as_deref(),
    )?;
    Ok(ProxyStorageConfigSource {
        proxy_raw_max_bytes,
        proxy_raw_dir,
        proxy_raw_compression,
        proxy_raw_immediate_gzip_bytes,
        proxy_raw_hot_secs,
        xray_binary,
        xray_runtime_dir,
        forward_proxy_algo,
    })
}

fn resolve_server_config_source(overrides: &CliArgs) -> Result<ServerConfigSource> {
    let max_parallel_polls = overrides
        .max_parallel_polls
        .or_else(|| {
            env::var(ENV_MAX_PARALLEL_POLLS)
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
        })
        .filter(|&v| v > 0)
        .unwrap_or(6);
    let shared_connection_parallelism = overrides
        .shared_connection_parallelism
        .or_else(|| {
            env::var(ENV_SHARED_CONNECTION_PARALLELISM)
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
        })
        .unwrap_or(2);
    let http_bind = if let Some(addr) = overrides.http_bind {
        addr
    } else {
        env::var(ENV_HTTP_BIND)
            .ok()
            .map(|v| v.parse())
            .transpose()
            .context("invalid HTTP_BIND socket address")?
            .unwrap_or_else(|| "127.0.0.1:8080".parse().expect("valid default address"))
    };
    let cors_allowed_origins = parse_cors_allowed_origins_env(ENV_CORS_ALLOWED_ORIGINS)?;
    let list_limit_max = overrides
        .list_limit_max
        .or_else(|| {
            env::var(ENV_LIST_LIMIT_MAX)
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
        })
        .filter(|&v| v > 0)
        .unwrap_or(200);
    let user_agent = overrides
        .user_agent
        .clone()
        .or_else(|| env::var(ENV_USER_AGENT).ok())
        .unwrap_or_else(|| "codex-vibe-monitor/0.2.0".to_string());
    let public_origin = env::var(ENV_PUBLIC_ORIGIN)
        .ok()
        .map(|value| normalize_public_origin(&value))
        .transpose()?;
    let static_dir = overrides
        .static_dir
        .clone()
        .or_else(|| env::var(ENV_STATIC_DIR).ok().map(PathBuf::from))
        .or_else(|| {
            let default = PathBuf::from("web/dist");
            if default.exists() {
                Some(default)
            } else {
                None
            }
        });
    Ok(ServerConfigSource {
        max_parallel_polls,
        shared_connection_parallelism,
        http_bind,
        cors_allowed_origins,
        list_limit_max,
        user_agent,
        public_origin,
        static_dir,
    })
}

fn resolve_retention_config_source(overrides: &CliArgs) -> Result<RetentionConfigSource> {
    let retention_enabled = parse_bool_env_var(ENV_RETENTION_ENABLED, DEFAULT_RETENTION_ENABLED)?;
    let retention_dry_run = overrides.retention_dry_run
        || parse_bool_env_var(ENV_RETENTION_DRY_RUN, DEFAULT_RETENTION_DRY_RUN)?;
    let retention_interval = Duration::from_secs(parse_u64_env_var(
        ENV_RETENTION_INTERVAL_SECS,
        DEFAULT_RETENTION_INTERVAL_SECS,
    )?);
    let retention_batch_rows =
        parse_usize_env_var(ENV_RETENTION_BATCH_ROWS, DEFAULT_RETENTION_BATCH_ROWS)?.max(1);
    let retention_catchup_budget = Duration::from_secs(parse_u64_env_var(
        ENV_RETENTION_CATCHUP_BUDGET_SECS,
        DEFAULT_RETENTION_CATCHUP_BUDGET_SECS,
    )?);
    let archive_dir = env::var(ENV_ARCHIVE_DIR)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ARCHIVE_DIR));
    let invocation_success_full_days = parse_u64_env_var(
        ENV_INVOCATION_SUCCESS_FULL_DAYS,
        DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
    )?;
    let invocation_max_days =
        parse_u64_env_var(ENV_INVOCATION_MAX_DAYS, DEFAULT_INVOCATION_MAX_DAYS)?;
    let invocation_archive_ttl_days = parse_u64_env_var(
        ENV_INVOCATION_ARCHIVE_TTL_DAYS,
        DEFAULT_INVOCATION_ARCHIVE_TTL_DAYS,
    )?;
    let codex_invocation_archive_layout = resolve_archive_batch_layout_config(
        env::var(ENV_CODEX_INVOCATION_ARCHIVE_LAYOUT)
            .ok()
            .as_deref(),
    )?;
    let codex_invocation_archive_segment_granularity = resolve_archive_segment_granularity_config(
        env::var(ENV_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY)
            .ok()
            .as_deref(),
    )?;
    let invocation_archive_codec =
        resolve_archive_file_codec_config(env::var(ENV_INVOCATION_ARCHIVE_CODEC).ok().as_deref())?;
    let forward_proxy_attempts_retention_days = parse_u64_env_var(
        ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
        DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
    )?;
    let pool_upstream_request_attempts_retention_days = parse_u64_env_var(
        ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS,
        DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS,
    )?;
    let pool_upstream_request_attempts_archive_ttl_days = parse_u64_env_var(
        ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
        DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
    )?;
    let quota_snapshot_full_days = parse_u64_env_var(
        ENV_QUOTA_SNAPSHOT_FULL_DAYS,
        DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
    )?;
    let long_term_stats_hourly_retention_days = parse_u64_env_var(
        ENV_LONG_TERM_STATS_HOURLY_RETENTION_DAYS,
        DEFAULT_LONG_TERM_STATS_HOURLY_RETENTION_DAYS,
    )?
    .max(MIN_LONG_TERM_STATS_HOURLY_RETENTION_DAYS);
    Ok(RetentionConfigSource {
        retention_enabled,
        retention_dry_run,
        retention_interval,
        retention_batch_rows,
        retention_catchup_budget,
        archive_dir,
        codex_invocation_archive_layout,
        codex_invocation_archive_segment_granularity,
        invocation_archive_codec,
        invocation_success_full_days,
        invocation_max_days,
        invocation_archive_ttl_days,
        forward_proxy_attempts_retention_days,
        pool_upstream_request_attempts_retention_days,
        pool_upstream_request_attempts_archive_ttl_days,
        quota_snapshot_full_days,
        long_term_stats_hourly_retention_days,
    })
}

fn resolve_upstream_accounts_config_source() -> Result<UpstreamAccountsConfigSource> {
    let upstream_accounts_oauth_client_id = env::var(ENV_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID.to_string());
    let upstream_accounts_oauth_issuer = Url::parse(
        &env::var(ENV_UPSTREAM_ACCOUNTS_OAUTH_ISSUER)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER.to_string()),
    )
    .context("invalid UPSTREAM_ACCOUNTS_OAUTH_ISSUER")?;
    let upstream_accounts_usage_base_url = Url::parse(
        &env::var(ENV_UPSTREAM_ACCOUNTS_USAGE_BASE_URL)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_UPSTREAM_ACCOUNTS_USAGE_BASE_URL.to_string()),
    )
    .context("invalid UPSTREAM_ACCOUNTS_USAGE_BASE_URL")?;
    let upstream_accounts_login_session_ttl = Duration::from_secs(parse_u64_env_var(
        ENV_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS,
        DEFAULT_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS,
    )?);
    let upstream_accounts_sync_interval = Duration::from_secs(parse_u64_env_var(
        ENV_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
        DEFAULT_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
    )?);
    let upstream_accounts_refresh_lead_time = Duration::from_secs(parse_u64_env_var(
        ENV_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS,
        DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS,
    )?);
    let upstream_accounts_history_retention_days = parse_u64_env_var(
        ENV_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
        DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
    )?;
    let kaisoumail_base_url_raw = env::var(ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let kaisoumail_api_key = env::var(ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let kaisoumail_default_removed_reason =
        "KaisouMail chooses generated mailbox local parts and domains upstream";
    reject_removed_env_var(
        ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_MAIL_DOMAIN,
        kaisoumail_default_removed_reason,
    )?;
    reject_removed_env_var(
        ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_SUBDOMAIN,
        kaisoumail_default_removed_reason,
    )?;
    reject_removed_env_var(
        LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN,
        kaisoumail_default_removed_reason,
    )?;
    let upstream_accounts_kaisoumail = match (kaisoumail_base_url_raw, kaisoumail_api_key) {
        (None, None) => None,
        (Some(base_url), Some(api_key)) => Some(UpstreamAccountsKaisouMailConfig {
            base_url: Url::parse(&base_url)
                .context("invalid UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL")?,
            api_key,
        }),
        _ => {
            return Err(anyhow!(
                "{} and {} must be set together",
                ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL,
                ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY
            ));
        }
    };
    Ok(UpstreamAccountsConfigSource {
        upstream_accounts_oauth_client_id,
        upstream_accounts_oauth_issuer,
        upstream_accounts_usage_base_url,
        upstream_accounts_login_session_ttl,
        upstream_accounts_sync_interval,
        upstream_accounts_refresh_lead_time,
        upstream_accounts_history_retention_days,
        upstream_accounts_kaisoumail,
    })
}

impl AppConfig {
    pub(crate) fn from_sources(overrides: &CliArgs) -> Result<Self> {
        reject_legacy_env_vars(LEGACY_ENV_RENAMES)?;
        let core = resolve_core_config_source(overrides)?;
        let proxy = resolve_proxy_config_source()?;
        let server = resolve_server_config_source(overrides)?;
        let retention = resolve_retention_config_source(overrides)?;
        let upstream_accounts = resolve_upstream_accounts_config_source()?;

        Ok(Self {
            openai_upstream_base_url: Url::parse(&core.openai_upstream_base_url)
                .context("invalid OPENAI_UPSTREAM_BASE_URL")?,
            database_path: core.database_path,
            poll_interval: core.poll_interval,
            request_timeout: core.request_timeout,
            pool_upstream_responses_attempt_timeout: core.pool_upstream_responses_attempt_timeout,
            pool_upstream_responses_total_timeout: core.pool_upstream_responses_total_timeout,
            openai_proxy_handshake_timeout: proxy.openai_proxy_handshake_timeout,
            openai_proxy_compact_handshake_timeout: proxy.openai_proxy_compact_handshake_timeout,
            openai_proxy_image_handshake_timeout: proxy.openai_proxy_image_handshake_timeout,
            openai_proxy_request_read_timeout: proxy.openai_proxy_request_read_timeout,
            openai_proxy_max_request_body_bytes: proxy.openai_proxy_max_request_body_bytes,
            openai_proxy_websocket_enabled: proxy.openai_proxy_websocket_enabled,
            openai_proxy_upstream_websocket_default_enabled: proxy
                .openai_proxy_upstream_websocket_default_enabled,
            openai_proxy_encrypted_session_owner_routing_enabled: proxy
                .openai_proxy_encrypted_session_owner_routing_enabled,
            proxy_enforce_stream_include_usage: proxy.proxy_enforce_stream_include_usage,
            proxy_usage_backfill_on_startup: proxy.proxy_usage_backfill_on_startup,
            proxy_raw_max_bytes: proxy.proxy_raw_max_bytes,
            proxy_raw_dir: proxy.proxy_raw_dir,
            proxy_raw_compression: proxy.proxy_raw_compression,
            proxy_raw_immediate_gzip_bytes: proxy.proxy_raw_immediate_gzip_bytes,
            proxy_raw_hot_secs: proxy.proxy_raw_hot_secs,
            xray_binary: proxy.xray_binary,
            xray_runtime_dir: proxy.xray_runtime_dir,
            forward_proxy_algo: proxy.forward_proxy_algo,
            max_parallel_polls: server.max_parallel_polls,
            shared_connection_parallelism: server.shared_connection_parallelism,
            http_bind: server.http_bind,
            cors_allowed_origins: server.cors_allowed_origins,
            list_limit_max: server.list_limit_max,
            user_agent: server.user_agent,
            static_dir: server.static_dir,
            public_origin: server.public_origin,
            retention_enabled: retention.retention_enabled,
            retention_dry_run: retention.retention_dry_run,
            retention_interval: retention.retention_interval,
            retention_batch_rows: retention.retention_batch_rows,
            retention_catchup_budget: retention.retention_catchup_budget,
            archive_dir: retention.archive_dir,
            codex_invocation_archive_layout: retention.codex_invocation_archive_layout,
            codex_invocation_archive_segment_granularity: retention
                .codex_invocation_archive_segment_granularity,
            invocation_archive_codec: retention.invocation_archive_codec,
            invocation_success_full_days: retention.invocation_success_full_days,
            invocation_max_days: retention.invocation_max_days,
            invocation_archive_ttl_days: retention.invocation_archive_ttl_days,
            forward_proxy_attempts_retention_days: retention.forward_proxy_attempts_retention_days,
            pool_upstream_request_attempts_retention_days: retention
                .pool_upstream_request_attempts_retention_days,
            pool_upstream_request_attempts_archive_ttl_days: retention
                .pool_upstream_request_attempts_archive_ttl_days,
            quota_snapshot_full_days: retention.quota_snapshot_full_days,
            long_term_stats_hourly_retention_days: retention.long_term_stats_hourly_retention_days,
            upstream_accounts_oauth_client_id: upstream_accounts.upstream_accounts_oauth_client_id,
            upstream_accounts_oauth_issuer: upstream_accounts.upstream_accounts_oauth_issuer,
            upstream_accounts_usage_base_url: upstream_accounts.upstream_accounts_usage_base_url,
            upstream_accounts_login_session_ttl: upstream_accounts
                .upstream_accounts_login_session_ttl,
            upstream_accounts_sync_interval: upstream_accounts.upstream_accounts_sync_interval,
            upstream_accounts_refresh_lead_time: upstream_accounts
                .upstream_accounts_refresh_lead_time,
            upstream_accounts_history_retention_days: upstream_accounts
                .upstream_accounts_history_retention_days,
            upstream_accounts_kaisoumail: upstream_accounts.upstream_accounts_kaisoumail,
        })
    }

    pub(crate) fn database_url(&self) -> String {
        format!("sqlite://{}", self.database_path.to_string_lossy())
    }

    pub(crate) fn resolved_proxy_raw_dir(&self) -> PathBuf {
        resolve_path_from_database_parent(&self.database_path, &self.proxy_raw_dir)
    }

    pub(crate) fn proxy_raw_immediate_compression_threshold(&self) -> Option<usize> {
        (self.proxy_raw_compression != RawCompressionCodec::None)
            .then_some(self.proxy_raw_immediate_gzip_bytes)
            .flatten()
    }
}
