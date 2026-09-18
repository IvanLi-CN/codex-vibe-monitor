#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ForwardProxyAlgo {
    V1,
    V2,
}

impl ForwardProxyAlgo {
    pub(crate) fn probe_every_requests(self) -> u64 {
        match self {
            Self::V1 => FORWARD_PROXY_PROBE_EVERY_REQUESTS,
            Self::V2 => FORWARD_PROXY_V2_PROBE_EVERY_REQUESTS,
        }
    }

    pub(crate) fn probe_interval_secs(self) -> i64 {
        match self {
            Self::V1 => FORWARD_PROXY_PROBE_INTERVAL_SECS,
            Self::V2 => FORWARD_PROXY_V2_PROBE_INTERVAL_SECS,
        }
    }

    pub(crate) fn probe_recovery_weight(self) -> f64 {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RawCompressionCodec {
    None,
    Gzip,
    Zstd,
}

impl FromStr for RawCompressionCodec {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "gzip" => Ok(Self::Gzip),
            "zstd" => Ok(Self::Zstd),
            _ => bail!("invalid {ENV_PROXY_RAW_COMPRESSION} value: {raw}"),
        }
    }
}

pub(crate) fn resolve_forward_proxy_algo_config(
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

pub(crate) fn resolve_raw_compression_codec_config(
    raw: Option<&str>,
) -> Result<RawCompressionCodec> {
    match raw {
        Some(value) => RawCompressionCodec::from_str(value),
        None => Ok(DEFAULT_PROXY_RAW_COMPRESSION),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArchiveBatchLayout {
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
pub(crate) enum ArchiveSegmentGranularity {
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
pub(crate) enum ArchiveFileCodec {
    Gzip,
}

impl ArchiveFileCodec {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Gzip => ARCHIVE_FILE_CODEC_GZIP,
        }
    }

    pub(crate) fn file_extension(self) -> &'static str {
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

pub(crate) fn resolve_archive_batch_layout_config(raw: Option<&str>) -> Result<ArchiveBatchLayout> {
    match raw {
        Some(value) => ArchiveBatchLayout::from_str(value),
        None => Ok(DEFAULT_CODEX_INVOCATION_ARCHIVE_LAYOUT),
    }
}

pub(crate) fn resolve_archive_segment_granularity_config(
    raw: Option<&str>,
) -> Result<ArchiveSegmentGranularity> {
    match raw {
        Some(value) => ArchiveSegmentGranularity::from_str(value),
        None => Ok(DEFAULT_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY),
    }
}

pub(crate) fn resolve_archive_file_codec_config(raw: Option<&str>) -> Result<ArchiveFileCodec> {
    match raw {
        Some(value) => ArchiveFileCodec::from_str(value),
        None => Ok(DEFAULT_INVOCATION_ARCHIVE_CODEC),
    }
}

pub(crate) fn reject_legacy_env_var(legacy_name: &str, canonical_name: &str) -> Result<()> {
    if env::var_os(legacy_name).is_some() {
        bail!("{legacy_name} is not supported; rename it to {canonical_name}");
    }
    Ok(())
}

pub(crate) fn reject_legacy_env_vars(renames: &[(&str, &str)]) -> Result<()> {
    for (legacy_name, canonical_name) in renames {
        reject_legacy_env_var(legacy_name, canonical_name)?;
    }
    Ok(())
}

pub(crate) fn reject_removed_env_var(name: &str, reason: &str) -> Result<()> {
    if env::var_os(name).is_some() {
        bail!("{name} is not supported; remove it because {reason}");
    }
    Ok(())
}

#[derive(Parser, Debug, Default)]
#[command(
    name = "codex-vibe-monitor",
    about = "Monitor Codex Vibes",
    disable_help_subcommand = true
)]
pub(crate) struct CliArgs {
    #[command(subcommand)]
    pub(crate) command: Option<CliCommand>,
    /// Override the SQLite database path; falls back to DATABASE_PATH or default.
    #[arg(long, value_name = "PATH")]
    pub(crate) database_path: Option<PathBuf>,
    /// Override the polling interval in seconds.
    #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64))]
    pub(crate) poll_interval_secs: Option<u64>,
    /// Override the request timeout in seconds.
    #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64))]
    pub(crate) request_timeout_secs: Option<u64>,
    /// Override the maximum number of concurrent polls.
    #[arg(long, value_name = "COUNT", value_parser = clap::value_parser!(usize))]
    pub(crate) max_parallel_polls: Option<usize>,
    /// Override the shared connection parallelism for HTTP clients.
    #[arg(long, value_name = "COUNT", value_parser = clap::value_parser!(usize))]
    pub(crate) shared_connection_parallelism: Option<usize>,
    /// Override the HTTP bind address (ip:port).
    #[arg(long, value_name = "ADDR", value_parser = clap::value_parser!(SocketAddr))]
    pub(crate) http_bind: Option<SocketAddr>,
    /// Override the maximum list limit for paged responses.
    #[arg(long, value_name = "COUNT", value_parser = clap::value_parser!(usize))]
    pub(crate) list_limit_max: Option<usize>,
    /// Override the user agent sent to upstream services.
    #[arg(long, value_name = "UA")]
    pub(crate) user_agent: Option<String>,
    /// Override the static directory served by the HTTP server.
    #[arg(long, value_name = "PATH")]
    pub(crate) static_dir: Option<PathBuf>,
    /// Run one retention/archival maintenance pass and exit.
    #[arg(long, default_value_t = false)]
    pub(crate) retention_run_once: bool,
    /// Force retention maintenance to simulate actions without mutating data.
    #[arg(long, default_value_t = false)]
    pub(crate) retention_dry_run: bool,
}

pub(crate) fn should_recover_pending_pool_attempts_on_startup(cli: &CliArgs) -> bool {
    cli.command.is_none() && !cli.retention_run_once
}

#[derive(Subcommand, Debug)]
pub(crate) enum CliCommand {
    Maintenance(MaintenanceCliArgs),
}

#[derive(Args, Debug)]
pub(crate) struct MaintenanceCliArgs {
    #[command(subcommand)]
    pub(crate) command: MaintenanceCommand,
}

#[derive(Subcommand, Debug)]
pub(crate) enum MaintenanceCommand {
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
pub(crate) struct MaintenanceDryRunArgs {
    #[arg(long, default_value_t = false)]
    pub(crate) dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppConfig {
    pub(crate) openai_upstream_base_url: Url,
    pub(crate) database_path: PathBuf,
    pub(crate) poll_interval: Duration,
    pub(crate) request_timeout: Duration,
    pub(crate) pool_upstream_responses_attempt_timeout: Duration,
    pub(crate) pool_upstream_responses_total_timeout: Duration,
    pub(crate) openai_proxy_handshake_timeout: Duration,
    pub(crate) openai_proxy_compact_handshake_timeout: Duration,
    pub(crate) openai_proxy_image_handshake_timeout: Duration,
    pub(crate) openai_proxy_request_read_timeout: Duration,
    pub(crate) openai_proxy_max_request_body_bytes: usize,
    pub(crate) openai_proxy_websocket_enabled: bool,
    pub(crate) openai_proxy_upstream_websocket_default_enabled: bool,
    pub(crate) openai_proxy_encrypted_session_owner_routing_enabled: bool,
    pub(crate) proxy_enforce_stream_include_usage: bool,
    pub(crate) proxy_usage_backfill_on_startup: bool,
    pub(crate) proxy_raw_max_bytes: Option<usize>,
    pub(crate) proxy_raw_dir: PathBuf,
    pub(crate) proxy_raw_compression: RawCompressionCodec,
    pub(crate) proxy_raw_immediate_gzip_bytes: Option<usize>,
    pub(crate) proxy_raw_hot_secs: u64,
    pub(crate) xray_binary: String,
    pub(crate) xray_runtime_dir: PathBuf,
    pub(crate) forward_proxy_algo: ForwardProxyAlgo,
    pub(crate) max_parallel_polls: usize,
    pub(crate) shared_connection_parallelism: usize,
    pub(crate) http_bind: SocketAddr,
    pub(crate) cors_allowed_origins: Vec<String>,
    pub(crate) list_limit_max: usize,
    pub(crate) user_agent: String,
    pub(crate) static_dir: Option<PathBuf>,
    pub(crate) public_origin: Option<String>,
    pub(crate) retention_enabled: bool,
    pub(crate) retention_dry_run: bool,
    pub(crate) retention_interval: Duration,
    pub(crate) retention_batch_rows: usize,
    pub(crate) retention_catchup_budget: Duration,
    pub(crate) archive_dir: PathBuf,
    pub(crate) codex_invocation_archive_layout: ArchiveBatchLayout,
    pub(crate) codex_invocation_archive_segment_granularity: ArchiveSegmentGranularity,
    pub(crate) invocation_archive_codec: ArchiveFileCodec,
    pub(crate) invocation_success_full_days: u64,
    pub(crate) invocation_max_days: u64,
    pub(crate) invocation_archive_ttl_days: u64,
    pub(crate) forward_proxy_attempts_retention_days: u64,
    pub(crate) pool_upstream_request_attempts_retention_days: u64,
    pub(crate) pool_upstream_request_attempts_archive_ttl_days: u64,
    pub(crate) quota_snapshot_full_days: u64,
    pub(crate) long_term_stats_hourly_retention_days: u64,
    pub(crate) upstream_accounts_oauth_client_id: String,
    pub(crate) upstream_accounts_oauth_issuer: Url,
    pub(crate) upstream_accounts_usage_base_url: Url,
    pub(crate) upstream_accounts_login_session_ttl: Duration,
    pub(crate) upstream_accounts_sync_interval: Duration,
    pub(crate) upstream_accounts_refresh_lead_time: Duration,
    pub(crate) upstream_accounts_history_retention_days: u64,
    pub(crate) upstream_accounts_kaisoumail: Option<UpstreamAccountsKaisouMailConfig>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountsKaisouMailConfig {
    pub(crate) base_url: Url,
    #[serde(skip_serializing)]
    pub(crate) api_key: String,
}

impl fmt::Debug for UpstreamAccountsKaisouMailConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpstreamAccountsKaisouMailConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

pub(crate) fn normalize_public_origin(raw: &str) -> Result<String> {
    normalize_cors_origin(raw).ok_or_else(|| anyhow!("invalid {ENV_PUBLIC_ORIGIN}: {raw}"))
}

pub(crate) fn parse_bool_env_var(name: &str, default_value: bool) -> Result<bool> {
    match env::var(name) {
        Ok(raw) => parse_bool_string(&raw).ok_or_else(|| anyhow!("invalid {name}: {raw}")),
        Err(env::VarError::NotPresent) => Ok(default_value),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

pub(crate) fn parse_u64_env_var(name: &str, default_value: u64) -> Result<u64> {
    match env::var(name) {
        Ok(raw) => raw
            .parse::<u64>()
            .with_context(|| format!("invalid {name}: {raw}")),
        Err(env::VarError::NotPresent) => Ok(default_value),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

pub(crate) fn parse_non_zero_u64_env_var(name: &str, default_value: u64) -> Result<u64> {
    let value = parse_u64_env_var(name, default_value)?;
    if value == 0 {
        bail!("{name} must be greater than 0");
    }
    Ok(value)
}

pub(crate) fn parse_usize_env_var(name: &str, default_value: usize) -> Result<usize> {
    match env::var(name) {
        Ok(raw) => raw
            .parse::<usize>()
            .with_context(|| format!("invalid {name}: {raw}")),
        Err(env::VarError::NotPresent) => Ok(default_value),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

pub(crate) fn parse_bool_string(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}
