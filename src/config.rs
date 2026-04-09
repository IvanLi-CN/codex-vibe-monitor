#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ForwardProxyAlgo {
    V1,
    V2,
}

impl ForwardProxyAlgo {
    fn probe_every_requests(self) -> u64 {
        match self {
            Self::V1 => FORWARD_PROXY_PROBE_EVERY_REQUESTS,
            Self::V2 => FORWARD_PROXY_V2_PROBE_EVERY_REQUESTS,
        }
    }

    fn probe_interval_secs(self) -> i64 {
        match self {
            Self::V1 => FORWARD_PROXY_PROBE_INTERVAL_SECS,
            Self::V2 => FORWARD_PROXY_V2_PROBE_INTERVAL_SECS,
        }
    }

    fn probe_recovery_weight(self) -> f64 {
        match self {
            Self::V1 => FORWARD_PROXY_PROBE_RECOVERY_WEIGHT,
            Self::V2 => FORWARD_PROXY_V2_PROBE_RECOVERY_WEIGHT,
        }
    }
}

impl FromStr for ForwardProxyAlgo {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "v1" => Ok(Self::V1),
            "v2" => Ok(Self::V2),
            _ => bail!("invalid FORWARD_PROXY_ALGO value: {raw}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RawCompressionCodec {
    None,
    Gzip,
}

impl FromStr for RawCompressionCodec {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "gzip" => Ok(Self::Gzip),
            _ => bail!("invalid {ENV_PROXY_RAW_COMPRESSION} value: {raw}"),
        }
    }
}

fn resolve_forward_proxy_algo_config(
    primary_raw: Option<&str>,
    legacy_raw: Option<&str>,
) -> Result<ForwardProxyAlgo> {
    if legacy_raw.is_some() {
        bail!(
            "{LEGACY_ENV_FORWARD_PROXY_ALGO} is not supported; rename it to {ENV_FORWARD_PROXY_ALGO}"
        );
    }
    match primary_raw {
        Some(primary) => ForwardProxyAlgo::from_str(primary),
        None => Ok(DEFAULT_FORWARD_PROXY_ALGO),
    }
}

fn resolve_raw_compression_codec_config(raw: Option<&str>) -> Result<RawCompressionCodec> {
    match raw {
        Some(value) => RawCompressionCodec::from_str(value),
        None => Ok(DEFAULT_PROXY_RAW_COMPRESSION),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArchiveBatchLayout {
    LegacyMonth,
    SegmentV1,
}

impl ArchiveBatchLayout {}

impl FromStr for ArchiveBatchLayout {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            ARCHIVE_LAYOUT_LEGACY_MONTH => Ok(Self::LegacyMonth),
            ARCHIVE_LAYOUT_SEGMENT_V1 => Ok(Self::SegmentV1),
            _ => bail!("invalid {ENV_CODEX_INVOCATION_ARCHIVE_LAYOUT} value: {raw}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArchiveSegmentGranularity {
    Day,
}

impl ArchiveSegmentGranularity {}

impl FromStr for ArchiveSegmentGranularity {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            ARCHIVE_SEGMENT_GRANULARITY_DAY => Ok(Self::Day),
            _ => bail!("invalid {ENV_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY} value: {raw}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ArchiveFileCodec {
    Gzip,
}

impl ArchiveFileCodec {
    fn as_str(self) -> &'static str {
        match self {
            Self::Gzip => ARCHIVE_FILE_CODEC_GZIP,
        }
    }

    fn file_extension(self) -> &'static str {
        match self {
            Self::Gzip => "gz",
        }
    }
}

impl FromStr for ArchiveFileCodec {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            ARCHIVE_FILE_CODEC_GZIP => Ok(Self::Gzip),
            _ => bail!("invalid {ENV_INVOCATION_ARCHIVE_CODEC} value: {raw}"),
        }
    }
}

fn resolve_archive_batch_layout_config(raw: Option<&str>) -> Result<ArchiveBatchLayout> {
    match raw {
        Some(value) => ArchiveBatchLayout::from_str(value),
        None => Ok(DEFAULT_CODEX_INVOCATION_ARCHIVE_LAYOUT),
    }
}

fn resolve_archive_segment_granularity_config(
    raw: Option<&str>,
) -> Result<ArchiveSegmentGranularity> {
    match raw {
        Some(value) => ArchiveSegmentGranularity::from_str(value),
        None => Ok(DEFAULT_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY),
    }
}

fn resolve_archive_file_codec_config(raw: Option<&str>) -> Result<ArchiveFileCodec> {
    match raw {
        Some(value) => ArchiveFileCodec::from_str(value),
        None => Ok(DEFAULT_INVOCATION_ARCHIVE_CODEC),
    }
}

fn reject_legacy_env_var(legacy_name: &str, canonical_name: &str) -> Result<()> {
    if env::var_os(legacy_name).is_some() {
        bail!("{legacy_name} is not supported; rename it to {canonical_name}");
    }
    Ok(())
}

fn reject_legacy_env_vars(renames: &[(&str, &str)]) -> Result<()> {
    for (legacy_name, canonical_name) in renames {
        reject_legacy_env_var(legacy_name, canonical_name)?;
    }
    Ok(())
}

#[derive(Parser, Debug, Default)]
#[command(
    name = "codex-vibe-monitor",
    about = "Monitor Codex Vibes",
    disable_help_subcommand = true
)]
struct CliArgs {
    #[command(subcommand)]
    command: Option<CliCommand>,
    /// Override the SQLite database path; falls back to DATABASE_PATH or default.
    #[arg(long, value_name = "PATH")]
    database_path: Option<PathBuf>,
    /// Override the polling interval in seconds.
    #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64))]
    poll_interval_secs: Option<u64>,
    /// Override the request timeout in seconds.
    #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64))]
    request_timeout_secs: Option<u64>,
    /// Override the maximum number of concurrent polls.
    #[arg(long, value_name = "COUNT", value_parser = clap::value_parser!(usize))]
    max_parallel_polls: Option<usize>,
    /// Override the shared connection parallelism for HTTP clients.
    #[arg(long, value_name = "COUNT", value_parser = clap::value_parser!(usize))]
    shared_connection_parallelism: Option<usize>,
    /// Override the HTTP bind address (ip:port).
    #[arg(long, value_name = "ADDR", value_parser = clap::value_parser!(SocketAddr))]
    http_bind: Option<SocketAddr>,
    /// Override the maximum list limit for paged responses.
    #[arg(long, value_name = "COUNT", value_parser = clap::value_parser!(usize))]
    list_limit_max: Option<usize>,
    /// Override the user agent sent to upstream services.
    #[arg(long, value_name = "UA")]
    user_agent: Option<String>,
    /// Override the static directory served by the HTTP server.
    #[arg(long, value_name = "PATH")]
    static_dir: Option<PathBuf>,
    /// Run one retention/archival maintenance pass and exit.
    #[arg(long, default_value_t = false)]
    retention_run_once: bool,
    /// Force retention maintenance to simulate actions without mutating data.
    #[arg(long, default_value_t = false)]
    retention_dry_run: bool,
}

fn should_recover_pending_pool_attempts_on_startup(cli: &CliArgs) -> bool {
    cli.command.is_none() && !cli.retention_run_once
}

#[derive(Subcommand, Debug)]
enum CliCommand {
    Maintenance(MaintenanceCliArgs),
}

#[derive(Args, Debug)]
struct MaintenanceCliArgs {
    #[command(subcommand)]
    command: MaintenanceCommand,
}

#[derive(Subcommand, Debug)]
enum MaintenanceCommand {
    /// Compress cold raw payload backlog without running the full retention pipeline.
    RawCompression(MaintenanceDryRunArgs),
    /// Rebuild codex_invocations archive upstream-activity manifests.
    ArchiveUpstreamActivityManifest(MaintenanceDryRunArgs),
    /// Materialize legacy archive-backed history into hourly rollup tables.
    MaterializeHistoricalRollups(MaintenanceDryRunArgs),
    /// Verify archive manifest/file consistency and stale temporary residues.
    VerifyArchiveStorage(MaintenanceDryRunArgs),
    /// Prune archive batches that are safe to delete under current retention guarantees.
    PruneArchiveBatches(MaintenanceDryRunArgs),
    /// Prune legacy archive batches that are no longer needed for online history.
    PruneLegacyArchiveBatches(MaintenanceDryRunArgs),
}

#[derive(Args, Debug, Default)]
struct MaintenanceDryRunArgs {
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}


#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    openai_upstream_base_url: Url,
    database_path: PathBuf,
    poll_interval: Duration,
    request_timeout: Duration,
    pool_upstream_responses_attempt_timeout: Duration,
    pool_upstream_responses_total_timeout: Duration,
    openai_proxy_handshake_timeout: Duration,
    openai_proxy_compact_handshake_timeout: Duration,
    openai_proxy_request_read_timeout: Duration,
    openai_proxy_max_request_body_bytes: usize,
    proxy_request_concurrency_limit: usize,
    proxy_request_concurrency_wait_timeout: Duration,
    proxy_enforce_stream_include_usage: bool,
    proxy_usage_backfill_on_startup: bool,
    proxy_raw_max_bytes: Option<usize>,
    proxy_raw_dir: PathBuf,
    proxy_raw_compression: RawCompressionCodec,
    proxy_raw_hot_secs: u64,
    xray_binary: String,
    xray_runtime_dir: PathBuf,
    forward_proxy_algo: ForwardProxyAlgo,
    max_parallel_polls: usize,
    shared_connection_parallelism: usize,
    http_bind: SocketAddr,
    cors_allowed_origins: Vec<String>,
    list_limit_max: usize,
    user_agent: String,
    static_dir: Option<PathBuf>,
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
    stats_source_snapshots_retention_days: u64,
    quota_snapshot_full_days: u64,
    crs_stats: Option<CrsStatsConfig>,
    upstream_accounts_oauth_client_id: String,
    upstream_accounts_oauth_issuer: Url,
    upstream_accounts_usage_base_url: Url,
    upstream_accounts_login_session_ttl: Duration,
    upstream_accounts_sync_interval: Duration,
    upstream_accounts_refresh_lead_time: Duration,
    upstream_accounts_history_retention_days: u64,
    upstream_accounts_moemail: Option<UpstreamAccountsMoeMailConfig>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrsStatsConfig {
    base_url: Url,
    api_id: String,
    period: String,
    poll_interval: Duration,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpstreamAccountsMoeMailConfig {
    base_url: Url,
    #[serde(skip_serializing)]
    api_key: String,
    default_domain: String,
}

impl AppConfig {
    fn from_sources(overrides: &CliArgs) -> Result<Self> {
        reject_legacy_env_vars(LEGACY_ENV_RENAMES)?;
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
        let pool_upstream_responses_attempt_timeout =
            Duration::from_secs(parse_non_zero_u64_env_var(
                ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
                DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
            )?);
        let pool_upstream_responses_total_timeout =
            Duration::from_secs(parse_non_zero_u64_env_var(
                ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
                DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
            )?);
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
        let proxy_request_concurrency_limit =
            match env::var(ENV_PROXY_REQUEST_CONCURRENCY_LIMIT) {
                Ok(value) => {
                    let parsed = value.parse::<usize>().with_context(|| {
                        format!(
                            "invalid {ENV_PROXY_REQUEST_CONCURRENCY_LIMIT}: {value}"
                        )
                    })?;
                    if parsed == 0 {
                        bail!("{ENV_PROXY_REQUEST_CONCURRENCY_LIMIT} must be > 0");
                    }
                    warn!(
                        configured_limit = parsed,
                        "{ENV_PROXY_REQUEST_CONCURRENCY_LIMIT} is deprecated and ignored for /v1/* admission"
                    );
                    DEFAULT_PROXY_REQUEST_CONCURRENCY_LIMIT
                }
                Err(env::VarError::NotPresent) => DEFAULT_PROXY_REQUEST_CONCURRENCY_LIMIT,
                Err(err) => {
                    return Err(anyhow!(
                        "failed to read {ENV_PROXY_REQUEST_CONCURRENCY_LIMIT}: {err}"
                    ));
                }
            };
        let proxy_request_concurrency_wait_timeout = match env::var(
            ENV_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS,
        ) {
            Ok(value) => {
                let parsed = value.parse::<u64>().with_context(|| {
                    format!(
                        "invalid {ENV_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS}: {value}"
                    )
                })?;
                if parsed == 0 {
                    bail!("{ENV_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS} must be > 0");
                }
                warn!(
                    configured_wait_timeout_ms = parsed,
                    "{ENV_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS} is deprecated and ignored for /v1/* admission"
                );
                Duration::from_millis(DEFAULT_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS)
            }
            Err(env::VarError::NotPresent) => {
                Duration::from_millis(DEFAULT_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS)
            }
            Err(err) => {
                return Err(anyhow!(
                    "failed to read {ENV_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS}: {err}"
                ));
            }
        };
        let proxy_enforce_stream_include_usage = parse_bool_env_var(
            "PROXY_ENFORCE_STREAM_INCLUDE_USAGE",
            DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
        )?;
        let proxy_usage_backfill_on_startup = parse_bool_env_var(
            "PROXY_USAGE_BACKFILL_ON_STARTUP",
            DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
        )?;
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
        let proxy_raw_compression = resolve_raw_compression_codec_config(
            env::var(ENV_PROXY_RAW_COMPRESSION).ok().as_deref(),
        )?;
        let proxy_raw_hot_secs =
            parse_u64_env_var(ENV_PROXY_RAW_HOT_SECS, DEFAULT_PROXY_RAW_HOT_SECS)?;
        let xray_binary =
            env::var(ENV_XRAY_BINARY).unwrap_or_else(|_| DEFAULT_XRAY_BINARY.to_string());
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
        let retention_enabled =
            parse_bool_env_var(ENV_RETENTION_ENABLED, DEFAULT_RETENTION_ENABLED)?;
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
        let codex_invocation_archive_segment_granularity =
            resolve_archive_segment_granularity_config(
                env::var(ENV_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY)
                    .ok()
                    .as_deref(),
            )?;
        let invocation_archive_codec = resolve_archive_file_codec_config(
            env::var(ENV_INVOCATION_ARCHIVE_CODEC).ok().as_deref(),
        )?;
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
        let stats_source_snapshots_retention_days = parse_u64_env_var(
            ENV_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
            DEFAULT_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
        )?;
        let quota_snapshot_full_days = parse_u64_env_var(
            ENV_QUOTA_SNAPSHOT_FULL_DAYS,
            DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
        )?;
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
        let moemail_base_url_raw = env::var(ENV_UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let moemail_api_key = env::var(ENV_UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let moemail_default_domain = env::var(ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let upstream_accounts_moemail = match (
            moemail_base_url_raw,
            moemail_api_key,
            moemail_default_domain,
        ) {
            (None, None, None) => None,
            (Some(base_url), Some(api_key), Some(default_domain)) => {
                Some(UpstreamAccountsMoeMailConfig {
                    base_url: Url::parse(&base_url)
                        .context("invalid UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL")?,
                    api_key,
                    default_domain,
                })
            }
            _ => {
                return Err(anyhow!(
                    "{} , {}, and {} must be set together",
                    ENV_UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL,
                    ENV_UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY,
                    ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN
                ));
            }
        };

        let crs_stats_base_url = env::var("CRS_STATS_BASE_URL").ok();
        let crs_stats_api_id = env::var("CRS_STATS_API_ID").ok();
        if crs_stats_base_url.is_some() ^ crs_stats_api_id.is_some() {
            return Err(anyhow!(
                "CRS_STATS_BASE_URL and CRS_STATS_API_ID must be set together"
            ));
        }

        let crs_stats_period = env::var("CRS_STATS_PERIOD")
            .ok()
            .unwrap_or_else(|| "daily".to_string());
        if crs_stats_period != "daily" {
            return Err(anyhow!("CRS_STATS_PERIOD only supports 'daily' for now"));
        }

        let crs_stats_poll_interval = env::var("CRS_STATS_POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(poll_interval);
        let crs_stats = match (crs_stats_base_url, crs_stats_api_id) {
            (Some(url), Some(api_id)) => Some(CrsStatsConfig {
                base_url: Url::parse(&url).context("invalid CRS_STATS_BASE_URL")?,
                api_id,
                period: crs_stats_period,
                poll_interval: crs_stats_poll_interval,
            }),
            _ => None,
        };

        Ok(Self {
            openai_upstream_base_url: Url::parse(&openai_upstream_base_url)
                .context("invalid OPENAI_UPSTREAM_BASE_URL")?,
            database_path,
            poll_interval,
            request_timeout,
            pool_upstream_responses_attempt_timeout,
            pool_upstream_responses_total_timeout,
            openai_proxy_handshake_timeout,
            openai_proxy_compact_handshake_timeout,
            openai_proxy_request_read_timeout,
            openai_proxy_max_request_body_bytes,
            proxy_request_concurrency_limit,
            proxy_request_concurrency_wait_timeout,
            proxy_enforce_stream_include_usage,
            proxy_usage_backfill_on_startup,
            proxy_raw_max_bytes,
            proxy_raw_dir,
            proxy_raw_compression,
            proxy_raw_hot_secs,
            xray_binary,
            xray_runtime_dir,
            forward_proxy_algo,
            max_parallel_polls,
            shared_connection_parallelism,
            http_bind,
            cors_allowed_origins,
            list_limit_max,
            user_agent,
            static_dir,
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
            stats_source_snapshots_retention_days,
            quota_snapshot_full_days,
            crs_stats,
            upstream_accounts_oauth_client_id,
            upstream_accounts_oauth_issuer,
            upstream_accounts_usage_base_url,
            upstream_accounts_login_session_ttl,
            upstream_accounts_sync_interval,
            upstream_accounts_refresh_lead_time,
            upstream_accounts_history_retention_days,
            upstream_accounts_moemail,
        })
    }

    fn database_url(&self) -> String {
        format!("sqlite://{}", self.database_path.to_string_lossy())
    }

    fn resolved_proxy_raw_dir(&self) -> PathBuf {
        resolve_path_from_database_parent(&self.database_path, &self.proxy_raw_dir)
    }
}

fn parse_bool_env_var(name: &str, default_value: bool) -> Result<bool> {
    match env::var(name) {
        Ok(raw) => parse_bool_string(&raw).ok_or_else(|| anyhow!("invalid {name}: {raw}")),
        Err(env::VarError::NotPresent) => Ok(default_value),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

fn parse_u64_env_var(name: &str, default_value: u64) -> Result<u64> {
    match env::var(name) {
        Ok(raw) => raw
            .parse::<u64>()
            .with_context(|| format!("invalid {name}: {raw}")),
        Err(env::VarError::NotPresent) => Ok(default_value),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

fn parse_non_zero_u64_env_var(name: &str, default_value: u64) -> Result<u64> {
    let value = parse_u64_env_var(name, default_value)?;
    if value == 0 {
        bail!("{name} must be greater than 0");
    }
    Ok(value)
}

fn parse_usize_env_var(name: &str, default_value: usize) -> Result<usize> {
    match env::var(name) {
        Ok(raw) => raw
            .parse::<usize>()
            .with_context(|| format!("invalid {name}: {raw}")),
        Err(env::VarError::NotPresent) => Ok(default_value),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

fn parse_bool_string(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}
