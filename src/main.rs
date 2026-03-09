use std::{
    borrow::Cow,
    collections::hash_map::DefaultHasher,
    collections::{BTreeMap, HashMap, HashSet},
    convert::Infallible,
    env,
    error::Error as StdError,
    hash::{Hash, Hasher},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use axum::response::sse::{Event, KeepAlive};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, OriginalUri, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, uri::Authority},
    response::{IntoResponse, Json, Response, Sse},
    routing::{any, get, post, put},
};
use base64::Engine;
use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, LocalResult, NaiveDate, NaiveDateTime,
    SecondsFormat, TimeZone, Utc,
};
use chrono_tz::{Asia::Shanghai, Tz};
use clap::Parser;
use dotenvy::dotenv;
use flate2::read::GzDecoder;
use flate2::{Compression, write::GzEncoder};
use futures_util::{StreamExt, stream};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::{Client, ClientBuilder, Proxy, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    Connection, FromRow, Pool, QueryBuilder, Row, Sqlite, SqliteConnection,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::fs;
use std::io::{self, Read, Write};
use tokio::{
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    sync::{Mutex, RwLock, Semaphore, broadcast, mpsc, watch},
    task::JoinHandle,
    time::{MissedTickBehavior, interval, sleep, timeout},
};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tokio_util::sync::CancellationToken;
use tower_http::{
    cors::{AllowOrigin, Any, CorsLayer},
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{debug, error, info, warn};

#[cfg_attr(not(test), allow(dead_code))]
const SOURCE_XY: &str = "xy";
const SOURCE_CRS: &str = "crs";
const SOURCE_PROXY: &str = "proxy";
const DEFAULT_OPENAI_UPSTREAM_BASE_URL: &str = "https://api.openai.com/";
const DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES: usize = 256 * 1024 * 1024;
const DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS: u64 = 45;
const DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS: u64 = 90;
const DEFAULT_SQLITE_BUSY_TIMEOUT_SECS: u64 = 30;
const BACKFILL_BATCH_SIZE: i64 = 200;
const STARTUP_BACKFILL_SCAN_LIMIT: u64 = 2_000;
const STARTUP_BACKFILL_RUN_BUDGET_SECS: u64 = 3;
const STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS: u64 = 15;
const STARTUP_BACKFILL_IDLE_INTERVAL_SECS: u64 = 6 * 60 * 60;
const STARTUP_BACKFILL_LOG_SAMPLE_LIMIT: usize = 5;
#[cfg(test)]
const BACKFILL_LOCK_RETRY_MAX_ATTEMPTS: u32 = 2;
#[cfg(test)]
const BACKFILL_LOCK_RETRY_DELAY_SECS: u64 = 3;
const COST_BACKFILL_ALGO_VERSION: &str = "2026-02-28";
const STARTUP_BACKFILL_STATUS_IDLE: &str = "idle";
const STARTUP_BACKFILL_STATUS_RUNNING: &str = "running";
const STARTUP_BACKFILL_STATUS_OK: &str = "ok";
const STARTUP_BACKFILL_STATUS_FAILED: &str = "failed";
const STARTUP_BACKFILL_TASK_PROXY_USAGE: &str = "proxy_usage_tokens_v1";
const STARTUP_BACKFILL_TASK_PROXY_COST: &str = "proxy_cost_v1";
const STARTUP_BACKFILL_TASK_PROMPT_CACHE_KEY: &str = "proxy_prompt_cache_key_v1";
const STARTUP_BACKFILL_TASK_REQUESTED_SERVICE_TIER: &str = "proxy_requested_service_tier_v1";
const STARTUP_BACKFILL_TASK_INVOCATION_SERVICE_TIER: &str = "invocation_service_tier_v1";
const STARTUP_BACKFILL_TASK_REASONING_EFFORT: &str = "proxy_reasoning_effort_v1";
const STARTUP_BACKFILL_TASK_FAILURE_CLASSIFICATION: &str = "failure_classification_v1";
const DEFAULT_PROXY_RAW_MAX_BYTES: Option<usize> = None;
const DEFAULT_PROXY_RAW_RETENTION_DAYS: u64 = 7;
const DEFAULT_PROXY_PRICING_CATALOG_PATH: &str = "config/model-pricing.json";
const DEFAULT_PROXY_RAW_DIR: &str = "proxy_raw_payloads";
const ENV_DATABASE_PATH: &str = "DATABASE_PATH";
const LEGACY_ENV_DATABASE_PATH: &str = "XY_DATABASE_PATH";
const DETAIL_LEVEL_FULL: &str = "full";
const DETAIL_LEVEL_STRUCTURED_ONLY: &str = "structured_only";
const DETAIL_PRUNE_REASON_SUCCESS_OVER_30D: &str = "success_over_30d";
const DETAIL_PRUNE_REASON_MAX_AGE_ARCHIVED: &str = "max_age_archived";
const DEFAULT_RETENTION_ENABLED: bool = false;
const DEFAULT_RETENTION_DRY_RUN: bool = false;
const DEFAULT_RETENTION_INTERVAL_SECS: u64 = 60 * 60;
const DEFAULT_RETENTION_BATCH_ROWS: usize = 1000;
const DEFAULT_ARCHIVE_DIR: &str = "archives";
const DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS: u64 = 24 * 60 * 60;
const DEFAULT_INVOCATION_SUCCESS_FULL_DAYS: u64 = 30;
const DEFAULT_INVOCATION_MAX_DAYS: u64 = 90;
const DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS: u64 = 30;
const DEFAULT_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS: u64 = 30;
const DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS: u64 = 30;
const ARCHIVE_STATUS_COMPLETED: &str = "completed";
const PROXY_REQUEST_BODY_LIMIT_EXCEEDED: &str = "proxy request body length limit exceeded";
const PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED: &str = "proxy path contains forbidden dot segments";
const PROXY_INVALID_REQUEST_TARGET: &str = "proxy request target is malformed";
const PROXY_UPSTREAM_HANDSHAKE_TIMEOUT: &str = "upstream handshake timed out";
const PROXY_MODEL_MERGE_STATUS_HEADER: &str = "x-proxy-model-merge-upstream";
const PROXY_MODEL_MERGE_STATUS_SUCCESS: &str = "success";
const PROXY_MODEL_MERGE_STATUS_FAILED: &str = "failed";
const PROXY_FAILURE_BODY_TOO_LARGE: &str = "body_too_large";
const PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT: &str = "request_body_read_timeout";
const PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED: &str =
    "request_body_stream_error_client_closed";
const PROXY_FAILURE_FAILED_CONTACT_UPSTREAM: &str = "failed_contact_upstream";
const PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT: &str = "upstream_handshake_timeout";
const PROXY_FAILURE_UPSTREAM_STREAM_ERROR: &str = "upstream_stream_error";
const PROXY_STREAM_TERMINAL_COMPLETED: &str = "stream_completed";
const PROXY_STREAM_TERMINAL_ERROR: &str = "stream_error";
const PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED: &str = "downstream_closed";
const FAILURE_CLASS_NONE: &str = "none";
const FAILURE_CLASS_SERVICE: &str = "service_failure";
const FAILURE_CLASS_CLIENT: &str = "client_failure";
const FAILURE_CLASS_ABORT: &str = "client_abort";
const PROXY_MODEL_SETTINGS_SINGLETON_ID: i64 = 1;
const PRICING_SETTINGS_SINGLETON_ID: i64 = 1;
const FORWARD_PROXY_SETTINGS_SINGLETON_ID: i64 = 1;
const DEFAULT_FORWARD_PROXY_INSERT_DIRECT: bool = true;
const DEFAULT_FORWARD_PROXY_SUBSCRIPTION_INTERVAL_SECS: u64 = 60 * 60;
const DEFAULT_FORWARD_PROXY_ALGO: ForwardProxyAlgo = ForwardProxyAlgo::V2;
const FORWARD_PROXY_WEIGHT_RECOVERY: f64 = 0.6;
const FORWARD_PROXY_WEIGHT_SUCCESS_BONUS: f64 = 0.45;
const FORWARD_PROXY_WEIGHT_FAILURE_PENALTY_BASE: f64 = 0.9;
const FORWARD_PROXY_WEIGHT_FAILURE_PENALTY_STEP: f64 = 0.35;
const FORWARD_PROXY_WEIGHT_MIN: f64 = -12.0;
const FORWARD_PROXY_WEIGHT_MAX: f64 = 12.0;
const FORWARD_PROXY_PROBE_EVERY_REQUESTS: u64 = 100;
const FORWARD_PROXY_PROBE_INTERVAL_SECS: i64 = 30 * 60;
const FORWARD_PROXY_PROBE_RECOVERY_WEIGHT: f64 = 0.4;
const FORWARD_PROXY_V2_WEIGHT_SUCCESS_BASE: f64 = 0.55;
const FORWARD_PROXY_V2_WEIGHT_SUCCESS_LATENCY_DIVISOR: f64 = 9000.0;
const FORWARD_PROXY_V2_WEIGHT_SUCCESS_LATENCY_CAP: f64 = 0.35;
const FORWARD_PROXY_V2_WEIGHT_SUCCESS_MIN_GAIN: f64 = 0.08;
const FORWARD_PROXY_V2_WEIGHT_FAILURE_BASE: f64 = 0.5;
const FORWARD_PROXY_V2_WEIGHT_FAILURE_STEP: f64 = 0.18;
const FORWARD_PROXY_V2_WEIGHT_FAILURE_MAX: f64 = 1.2;
const FORWARD_PROXY_V2_WEIGHT_MIN: f64 = -8.0;
const FORWARD_PROXY_V2_WEIGHT_MAX: f64 = 8.0;
const FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR: f64 = 0.25;
const FORWARD_PROXY_V2_PROBE_EVERY_REQUESTS: u64 = 30;
const FORWARD_PROXY_V2_PROBE_INTERVAL_SECS: i64 = 5 * 60;
const FORWARD_PROXY_V2_PROBE_RECOVERY_WEIGHT: f64 = 0.55;
const FORWARD_PROXY_V2_DIRECT_INITIAL_WEIGHT: f64 = 0.7;
const FORWARD_PROXY_V2_MIN_POSITIVE_CANDIDATES: usize = 2;
const FORWARD_PROXY_VALIDATION_TIMEOUT_SECS: u64 = 5;
const FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_SECS: u64 = 60;
const FORWARD_PROXY_DIRECT_KEY: &str = "__direct__";
const FORWARD_PROXY_DIRECT_LABEL: &str = "Direct";
const FORWARD_PROXY_SOURCE_MANUAL: &str = "manual";
const FORWARD_PROXY_SOURCE_SUBSCRIPTION: &str = "subscription";
const FORWARD_PROXY_SOURCE_DIRECT: &str = "direct";
const FORWARD_PROXY_FAILURE_SEND_ERROR: &str = "send_error";
const FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT: &str = "handshake_timeout";
const FORWARD_PROXY_FAILURE_STREAM_ERROR: &str = "stream_error";
const FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX: &str = "upstream_http_5xx";
const DEFAULT_XRAY_BINARY: &str = "xray";
const DEFAULT_XRAY_RUNTIME_DIR: &str = ".codex/xray-forward";
const XRAY_PROXY_READY_TIMEOUT_MS: u64 = 3_000;
const DEFAULT_PRICING_CATALOG_VERSION: &str = "openai-standard-2026-03-06";
const LEGACY_DEFAULT_PRICING_CATALOG_VERSION: &str = "openai-standard-2026-02-23";
const DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE: bool = true;
const DEFAULT_PROXY_MODELS_HIJACK_ENABLED: bool = false;
const DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED: bool = false;
const DEFAULT_PROXY_FAST_MODE_REWRITE_MODE: ProxyFastModeRewriteMode =
    ProxyFastModeRewriteMode::Disabled;
const DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP: bool = true;
const GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS: i64 = 272_000;
const ENV_CORS_ALLOWED_ORIGINS: &str = "XY_CORS_ALLOWED_ORIGINS";
const PROMPT_CACHE_CONVERSATION_DEFAULT_LIMIT: i64 = 50;
const PROMPT_CACHE_CONVERSATION_CACHE_TTL_SECS: u64 = 5;
const PROXY_PRESET_MODEL_IDS: &[&str] = &[
    "gpt-5.3-codex",
    "gpt-5.2-codex",
    "gpt-5.1-codex-max",
    "gpt-5.1-codex-mini",
    "gpt-5.2",
    "gpt-5.4",
    "gpt-5.4-pro",
];
const LEGACY_PROXY_PRESET_MODEL_IDS: &[&str] = &[
    "gpt-5.3-codex",
    "gpt-5.2-codex",
    "gpt-5.1-codex-max",
    "gpt-5.1-codex-mini",
    "gpt-5.2",
];
static NEXT_PROXY_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

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

fn resolve_forward_proxy_algo_config(
    primary_raw: Option<&str>,
    legacy_raw: Option<&str>,
) -> Result<ForwardProxyAlgo> {
    if legacy_raw.is_some() {
        bail!("XY_FORWARD_PROXY_ALGO is not supported; use FORWARD_PROXY_ALGO");
    }
    match primary_raw {
        Some(primary) => ForwardProxyAlgo::from_str(primary),
        None => Ok(DEFAULT_FORWARD_PROXY_ALGO),
    }
}

#[derive(Parser, Debug, Default)]
#[command(
    name = "codex-vibe-monitor",
    about = "Monitor Codex Vibes",
    disable_help_subcommand = true
)]
struct CliArgs {
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

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    dotenvy::from_filename(".env.local").ok();
    init_tracing();
    let startup_started_at = Instant::now();

    let cli = CliArgs::parse();
    let config = AppConfig::from_sources(&cli)?;
    let (backend_ver, frontend_ver) = detect_versions(config.static_dir.as_deref());
    info!(?config, backend_version = %backend_ver, frontend_version = %frontend_ver, "starting codex vibe monitor");

    let database_url = config.database_url();
    ensure_db_directory(&config.database_path)?;
    let connect_opts = build_sqlite_connect_options(
        &database_url,
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )?;
    let db_connect_started_at = Instant::now();
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await
        .context("failed to open sqlite database")?;
    log_startup_phase("db_connect", db_connect_started_at);

    let schema_started_at = Instant::now();
    ensure_schema(&pool).await?;
    log_startup_phase("schema", schema_started_at);
    if cli.retention_run_once {
        let summary =
            run_data_retention_maintenance(&pool, &config, Some(cli.retention_dry_run)).await?;
        info!(?summary, "retention maintenance run-once finished");
        return Ok(());
    }

    let runtime_init_started_at = Instant::now();
    let pricing_catalog = load_pricing_catalog(&pool).await?;
    let proxy_model_settings = Arc::new(RwLock::new(load_proxy_model_settings(&pool).await?));
    let forward_proxy_settings = load_forward_proxy_settings(&pool).await?;
    let forward_proxy_runtime = load_forward_proxy_runtime_states(&pool).await?;
    let forward_proxy = Arc::new(Mutex::new(ForwardProxyManager::with_algo(
        forward_proxy_settings,
        forward_proxy_runtime,
        config.forward_proxy_algo,
    )));
    let resolved_proxy_raw_dir = config.resolved_proxy_raw_dir();
    fs::create_dir_all(&resolved_proxy_raw_dir).with_context(|| {
        format!(
            "failed to create proxy raw payload directory: {}",
            resolved_proxy_raw_dir.display()
        )
    })?;
    let pricing_catalog = Arc::new(RwLock::new(pricing_catalog));

    let http_clients = HttpClients::build(&config)?;
    let (tx, _rx) = broadcast::channel(128);
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));

    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster: tx.clone(),
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        startup_ready: Arc::new(AtomicBool::new(false)),
        semaphore: semaphore.clone(),
        proxy_model_settings,
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy,
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog,
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
    });

    if let Err(err) = sync_forward_proxy_routes(state.as_ref()).await {
        warn!(
            error = %err,
            "failed to initialize forward proxy xray routes at startup"
        );
    }
    log_startup_phase("runtime_init", runtime_init_started_at);

    // Shared cancellation token for graceful shutdown
    let cancel = CancellationToken::new();

    // Listen for OS signals and trigger cancellation
    let cancel_for_signals = cancel.clone();
    let signals_task = tokio::spawn(async move {
        shutdown_listener().await;
        cancel_for_signals.cancel();
        info!("shutdown signal received; beginning graceful shutdown");
    });

    let poller_handle = if state.config.crs_stats.is_some() {
        Some(spawn_scheduler(state.clone(), cancel.clone()))
    } else {
        info!("crs stats relay is disabled; scheduler will not start");
        None
    };
    let forward_proxy_handle = spawn_forward_proxy_maintenance(state.clone(), cancel.clone());
    let retention_handle = spawn_data_retention_maintenance(state.clone(), cancel.clone());
    let http_ready_started_at = Instant::now();
    let server_handle = spawn_http_server(state.clone(), cancel.clone()).await?;
    log_startup_phase("http_ready", http_ready_started_at);
    info!(
        time_to_health_ms = startup_started_at.elapsed().as_millis() as u64,
        "application readiness reached"
    );
    let startup_backfill_handle = spawn_startup_backfill_maintenance(state.clone(), cancel.clone());

    // Wait until a shutdown signal is received, then wait for tasks to finish
    let _ = signals_task.await;

    if let Err(err) = server_handle.await {
        error!(?err, "http server terminated unexpectedly");
    }
    if let Some(poller_handle) = poller_handle
        && let Err(err) = poller_handle.await
    {
        error!(?err, "poller task terminated unexpectedly");
    }
    if let Err(err) = forward_proxy_handle.await {
        error!(
            ?err,
            "forward proxy maintenance task terminated unexpectedly"
        );
    }
    if let Err(err) = retention_handle.await {
        error!(?err, "retention maintenance task terminated unexpectedly");
    }
    if let Err(err) = startup_backfill_handle.await {
        error!(
            ?err,
            "startup backfill maintenance task terminated unexpectedly"
        );
    }

    state.xray_supervisor.lock().await.shutdown_all().await;

    Ok(())
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .with_target(false)
        .init();
}

fn log_startup_phase(phase: &'static str, started_at: Instant) {
    info!(
        phase,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "startup phase finished"
    );
}

fn push_backfill_sample(samples: &mut Vec<String>, sample: String) {
    if samples.len() < STARTUP_BACKFILL_LOG_SAMPLE_LIMIT {
        samples.push(sample);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupBackfillTask {
    ProxyUsage,
    ProxyCost,
    PromptCacheKey,
    RequestedServiceTier,
    InvocationServiceTier,
    ReasoningEffort,
    FailureClassification,
}

impl StartupBackfillTask {
    fn ordered_tasks() -> &'static [Self] {
        &[
            Self::ProxyUsage,
            Self::ProxyCost,
            Self::PromptCacheKey,
            Self::RequestedServiceTier,
            Self::InvocationServiceTier,
            Self::ReasoningEffort,
            Self::FailureClassification,
        ]
    }

    fn name(self) -> &'static str {
        match self {
            Self::ProxyUsage => STARTUP_BACKFILL_TASK_PROXY_USAGE,
            Self::ProxyCost => STARTUP_BACKFILL_TASK_PROXY_COST,
            Self::PromptCacheKey => STARTUP_BACKFILL_TASK_PROMPT_CACHE_KEY,
            Self::RequestedServiceTier => STARTUP_BACKFILL_TASK_REQUESTED_SERVICE_TIER,
            Self::InvocationServiceTier => STARTUP_BACKFILL_TASK_INVOCATION_SERVICE_TIER,
            Self::ReasoningEffort => STARTUP_BACKFILL_TASK_REASONING_EFFORT,
            Self::FailureClassification => STARTUP_BACKFILL_TASK_FAILURE_CLASSIFICATION,
        }
    }

    fn log_label(self) -> &'static str {
        match self {
            Self::ProxyUsage => "proxy usage",
            Self::ProxyCost => "proxy cost",
            Self::PromptCacheKey => "proxy prompt cache key",
            Self::RequestedServiceTier => "proxy requested service tier",
            Self::InvocationServiceTier => "invocation service tier",
            Self::ReasoningEffort => "proxy reasoning effort",
            Self::FailureClassification => "invocation failure classification",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct StartupBackfillProgressRow {
    task_name: String,
    cursor_id: i64,
    next_run_after: Option<String>,
    zero_update_streak: i64,
    last_started_at: Option<String>,
    last_finished_at: Option<String>,
    last_scanned: i64,
    last_updated: i64,
    last_status: String,
}

#[derive(Debug, Clone)]
struct StartupBackfillProgress {
    task_name: String,
    cursor_id: i64,
    next_run_after: Option<String>,
    zero_update_streak: u32,
    last_started_at: Option<String>,
    last_finished_at: Option<String>,
    last_scanned: u64,
    last_updated: u64,
    last_status: String,
}

impl StartupBackfillProgress {
    fn pending(task_name: impl Into<String>) -> Self {
        Self {
            task_name: task_name.into(),
            cursor_id: 0,
            next_run_after: None,
            zero_update_streak: 0,
            last_started_at: None,
            last_finished_at: None,
            last_scanned: 0,
            last_updated: 0,
            last_status: STARTUP_BACKFILL_STATUS_IDLE.to_string(),
        }
    }

    fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.next_run_after
            .as_deref()
            .and_then(parse_to_utc_datetime)
            .is_none_or(|deadline| deadline <= now)
    }
}

impl From<StartupBackfillProgressRow> for StartupBackfillProgress {
    fn from(value: StartupBackfillProgressRow) -> Self {
        Self {
            task_name: value.task_name,
            cursor_id: value.cursor_id,
            next_run_after: value.next_run_after,
            zero_update_streak: value.zero_update_streak.max(0) as u32,
            last_started_at: value.last_started_at,
            last_finished_at: value.last_finished_at,
            last_scanned: value.last_scanned.max(0) as u64,
            last_updated: value.last_updated.max(0) as u64,
            last_status: value.last_status,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct StartupBackfillRunState {
    next_cursor_id: i64,
    scanned: u64,
    updated: u64,
    hit_scan_limit: bool,
    samples: Vec<String>,
}

fn startup_backfill_next_delay(run: &StartupBackfillRunState, zero_update_streak: u32) -> Duration {
    if run.hit_scan_limit || run.updated > 0 {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    } else if run.scanned == 0 || zero_update_streak > 0 {
        Duration::from_secs(STARTUP_BACKFILL_IDLE_INTERVAL_SECS)
    } else {
        Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS)
    }
}

fn startup_backfill_next_run_after(
    run: &StartupBackfillRunState,
    zero_update_streak: u32,
) -> String {
    format_utc_iso(
        Utc::now()
            + ChronoDuration::from_std(startup_backfill_next_delay(run, zero_update_streak))
                .unwrap_or_else(|_| {
                    ChronoDuration::seconds(STARTUP_BACKFILL_IDLE_INTERVAL_SECS as i64)
                }),
    )
}

#[derive(Debug, Clone)]
struct BackfillBatchOutcome<T> {
    summary: T,
    next_cursor_id: i64,
    hit_budget: bool,
    samples: Vec<String>,
}

fn startup_backfill_query_limit(scanned: u64, scan_limit: Option<u64>) -> i64 {
    let remaining = scan_limit
        .map(|limit| limit.saturating_sub(scanned))
        .unwrap_or(BACKFILL_BATCH_SIZE as u64);
    remaining.min(BACKFILL_BATCH_SIZE as u64).max(1) as i64
}

fn startup_backfill_budget_reached(
    started_at: Instant,
    scanned: u64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> bool {
    let hit_scan_limit = scan_limit.is_some_and(|limit| scanned >= limit);
    let hit_elapsed_limit = max_elapsed.is_some_and(|limit| started_at.elapsed() >= limit);
    hit_scan_limit || hit_elapsed_limit
}

fn startup_backfill_samples_text(samples: &[String]) -> String {
    if samples.is_empty() {
        "-".to_string()
    } else {
        samples.join(" | ")
    }
}

async fn startup_backfill_task_progress_key(state: &AppState, task: StartupBackfillTask) -> String {
    match task {
        StartupBackfillTask::ProxyCost => {
            let catalog = state.pricing_catalog.read().await;
            format!(
                "{}:{}",
                task.name(),
                pricing_backfill_attempt_version(&catalog)
            )
        }
        _ => task.name().to_string(),
    }
}

async fn load_startup_backfill_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
) -> Result<StartupBackfillProgress> {
    Ok(sqlx::query_as::<_, StartupBackfillProgressRow>(
        r#"
        SELECT
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status
        FROM startup_backfill_progress
        WHERE task_name = ?1
        LIMIT 1
        "#,
    )
    .bind(task_name)
    .fetch_optional(pool)
    .await?
    .map(Into::into)
    .unwrap_or_else(|| StartupBackfillProgress::pending(task_name.to_string())))
}

async fn mark_startup_backfill_running(
    pool: &Pool<Sqlite>,
    task_name: &str,
    cursor_id: i64,
) -> Result<()> {
    let now = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status
        )
        VALUES (?1, ?2, NULL, 0, ?3, NULL, 0, 0, ?4)
        ON CONFLICT(task_name) DO UPDATE SET
            next_run_after = NULL,
            last_started_at = excluded.last_started_at,
            last_status = excluded.last_status
        "#,
    )
    .bind(task_name)
    .bind(cursor_id)
    .bind(&now)
    .bind(STARTUP_BACKFILL_STATUS_RUNNING)
    .execute(pool)
    .await?;
    Ok(())
}

struct StartupBackfillProgressUpdate<'a> {
    cursor_id: i64,
    scanned: u64,
    updated: u64,
    zero_update_streak: u32,
    next_run_after: &'a str,
    status: &'a str,
}

async fn save_startup_backfill_progress(
    pool: &Pool<Sqlite>,
    task_name: &str,
    update: StartupBackfillProgressUpdate<'_>,
) -> Result<()> {
    let finished_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8)
        ON CONFLICT(task_name) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            next_run_after = excluded.next_run_after,
            zero_update_streak = excluded.zero_update_streak,
            last_finished_at = excluded.last_finished_at,
            last_scanned = excluded.last_scanned,
            last_updated = excluded.last_updated,
            last_status = excluded.last_status
        "#,
    )
    .bind(task_name)
    .bind(update.cursor_id)
    .bind(update.next_run_after)
    .bind(i64::from(update.zero_update_streak))
    .bind(&finished_at)
    .bind(update.scanned as i64)
    .bind(update.updated as i64)
    .bind(update.status)
    .execute(pool)
    .await?;
    Ok(())
}

async fn run_startup_backfill_maintenance_pass(state: Arc<AppState>) {
    for task in StartupBackfillTask::ordered_tasks() {
        if *task == StartupBackfillTask::ProxyUsage && !state.config.proxy_usage_backfill_on_startup
        {
            debug!(
                task = task.log_label(),
                "startup backfill task is disabled by config"
            );
            continue;
        }
        if let Err(err) = run_startup_backfill_task_if_due(&state, *task).await {
            warn!(task = task.log_label(), error = %err, "startup backfill supervisor pass failed");
        }
    }
}

fn startup_backfill_task_enabled(state: &AppState, task: StartupBackfillTask) -> bool {
    match task {
        StartupBackfillTask::ProxyUsage => state.config.proxy_usage_backfill_on_startup,
        _ => true,
    }
}

async fn run_startup_backfill_task_if_due(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
) -> Result<()> {
    if !startup_backfill_task_enabled(state.as_ref(), task) {
        debug!(
            task = task.log_label(),
            "startup backfill task is disabled by config"
        );
        return Ok(());
    }

    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name).await?;
    let now = Utc::now();
    if !progress.is_due(now) {
        debug!(
            task = task.log_label(),
            task_name = %progress.task_name,
            next_run_after = progress.next_run_after.as_deref().unwrap_or("-"),
            last_status = %progress.last_status,
            last_started_at = progress.last_started_at.as_deref().unwrap_or("-"),
            last_finished_at = progress.last_finished_at.as_deref().unwrap_or("-"),
            last_scanned = progress.last_scanned,
            last_updated = progress.last_updated,
            "startup backfill task is not due"
        );
        return Ok(());
    }

    mark_startup_backfill_running(&state.pool, &task_name, progress.cursor_id).await?;

    let started_at = Instant::now();
    match run_startup_backfill_task(state, task, progress.cursor_id).await {
        Ok((run, detail)) => {
            let zero_update_streak = if run.updated == 0 {
                progress.zero_update_streak.saturating_add(1)
            } else {
                0
            };
            let next_cursor_id = run.next_cursor_id.max(progress.cursor_id);
            let next_run_after = startup_backfill_next_run_after(&run, zero_update_streak);
            save_startup_backfill_progress(
                &state.pool,
                &task_name,
                StartupBackfillProgressUpdate {
                    cursor_id: next_cursor_id,
                    scanned: run.scanned,
                    updated: run.updated,
                    zero_update_streak,
                    next_run_after: &next_run_after,
                    status: STARTUP_BACKFILL_STATUS_OK,
                },
            )
            .await?;
            info!(
                task = task.log_label(),
                task_name = %task_name,
                scanned = run.scanned,
                updated = run.updated,
                cursor_id = next_cursor_id,
                hit_scan_limit = run.hit_scan_limit,
                zero_update_streak,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                next_run_after = %next_run_after,
                detail = %detail,
                samples = %startup_backfill_samples_text(&run.samples),
                "startup backfill pass finished"
            );
        }
        Err(err) => {
            let retry_after = format_utc_iso(
                Utc::now() + ChronoDuration::seconds(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS as i64),
            );
            save_startup_backfill_progress(
                &state.pool,
                &task_name,
                StartupBackfillProgressUpdate {
                    cursor_id: progress.cursor_id,
                    scanned: 0,
                    updated: 0,
                    zero_update_streak: progress.zero_update_streak,
                    next_run_after: &retry_after,
                    status: STARTUP_BACKFILL_STATUS_FAILED,
                },
            )
            .await?;
            warn!(
                task = task.log_label(),
                task_name = %task_name,
                cursor_id = progress.cursor_id,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                next_run_after = %retry_after,
                error = %err,
                "startup backfill pass failed"
            );
        }
    }

    Ok(())
}

async fn run_startup_backfill_task(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    cursor_id: i64,
) -> Result<(StartupBackfillRunState, String)> {
    let max_elapsed = Some(Duration::from_secs(STARTUP_BACKFILL_RUN_BUDGET_SECS));
    let raw_path_fallback_root = state.config.database_path.parent();
    match task {
        StartupBackfillTask::ProxyUsage => {
            let snapshot_max_id = current_proxy_usage_backfill_snapshot_max_id(&state.pool).await?;
            let outcome = backfill_proxy_usage_tokens_from_cursor(
                &state.pool,
                cursor_id,
                snapshot_max_id,
                raw_path_fallback_root,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_without_usage={} skipped_decode_error={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_without_usage,
                outcome.summary.skipped_decode_error
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::ProxyCost => {
            let catalog = state.pricing_catalog.read().await.clone();
            let attempt_version = pricing_backfill_attempt_version(&catalog);
            let snapshot_max_id =
                current_proxy_cost_backfill_snapshot_max_id(&state.pool, &attempt_version).await?;
            let outcome = backfill_proxy_missing_costs_from_cursor(
                &state.pool,
                cursor_id,
                snapshot_max_id,
                &catalog,
                &attempt_version,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_unpriced_model={}",
                outcome.summary.skipped_unpriced_model
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::PromptCacheKey => {
            let outcome = backfill_proxy_prompt_cache_keys_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_invalid_json={} skipped_missing_key={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_key
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::RequestedServiceTier => {
            let outcome = backfill_proxy_requested_service_tiers_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_invalid_json={} skipped_missing_tier={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_tier
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::InvocationServiceTier => {
            let outcome = backfill_invocation_service_tiers_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_missing_tier={}",
                outcome.summary.skipped_missing_file, outcome.summary.skipped_missing_tier
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::ReasoningEffort => {
            let outcome = backfill_proxy_reasoning_efforts_from_cursor(
                &state.pool,
                cursor_id,
                raw_path_fallback_root,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
                max_elapsed,
            )
            .await?;
            let detail = format!(
                "skipped_missing_file={} skipped_invalid_json={} skipped_missing_effort={}",
                outcome.summary.skipped_missing_file,
                outcome.summary.skipped_invalid_json,
                outcome.summary.skipped_missing_effort
            );
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    samples: outcome.samples,
                },
                detail,
            ))
        }
        StartupBackfillTask::FailureClassification => {
            let outcome = backfill_failure_classification_from_cursor(
                &state.pool,
                cursor_id,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
                max_elapsed,
            )
            .await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: outcome.next_cursor_id,
                    scanned: outcome.summary.scanned,
                    updated: outcome.summary.updated,
                    hit_scan_limit: outcome.hit_budget,
                    samples: Vec::new(),
                },
                "failure classification recalculated".to_string(),
            ))
        }
    }
}

fn spawn_startup_backfill_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_startup_backfill_maintenance_pass(state.clone()).await;

        let mut ticker = interval(Duration::from_secs(STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("startup backfill maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    run_startup_backfill_maintenance_pass(state.clone()).await;
                }
            }
        }
    })
}

fn spawn_scheduler(state: Arc<AppState>, cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        // Track in-flight tasks so we can wait for them on shutdown
        let mut inflight: Vec<JoinHandle<()>> = Vec::new();
        match schedule_poll(state.clone()).await {
            Ok(h) => inflight.push(h),
            Err(err) => warn!(?err, "initial poll failed"),
        }

        let mut ticker = interval(state.config.poll_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("scheduler received shutdown; waiting for in-flight polls");
                    // Drain completed tasks first
                    inflight.retain(|h| !h.is_finished());
                    // Wait for remaining tasks to finish
                    for h in inflight {
                        let _ = h.await;
                    }
                    break;
                }
                _ = ticker.tick() => {
                    match schedule_poll(state.clone()).await {
                        Ok(handle) => {
                            inflight.push(handle);
                            // Clean up finished tasks to avoid unbounded growth
                            inflight.retain(|h| !h.is_finished());
                        }
                        Err(err) => {
                            warn!(?err, "scheduled poll failed");
                        }
                    }
                }
            }
        }
    })
}

fn spawn_forward_proxy_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let startup_known_subscription_keys = {
            let manager = state.forward_proxy.lock().await;
            snapshot_known_subscription_proxy_keys(&manager)
        };
        if let Err(err) = refresh_forward_proxy_subscriptions(
            state.clone(),
            true,
            Some(startup_known_subscription_keys),
        )
        .await
        {
            warn!(error = %err, "failed to refresh forward proxy subscriptions at startup");
        }

        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("forward proxy maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(err) = refresh_forward_proxy_subscriptions(state.clone(), false, None).await {
                        warn!(error = %err, "failed to refresh forward proxy subscriptions");
                    }
                }
            }
        }
    })
}

#[derive(Debug, Default)]
struct RetentionRunSummary {
    dry_run: bool,
    invocation_details_pruned: usize,
    invocation_rows_archived: usize,
    forward_proxy_attempt_rows_archived: usize,
    stats_source_snapshot_rows_archived: usize,
    quota_snapshot_rows_archived: usize,
    archive_batches_touched: usize,
    raw_files_removed: usize,
    orphan_raw_files_removed: usize,
}

impl RetentionRunSummary {
    fn touched_anything(&self) -> bool {
        self.invocation_details_pruned > 0
            || self.invocation_rows_archived > 0
            || self.forward_proxy_attempt_rows_archived > 0
            || self.stats_source_snapshot_rows_archived > 0
            || self.quota_snapshot_rows_archived > 0
            || self.raw_files_removed > 0
            || self.orphan_raw_files_removed > 0
    }
}

#[derive(Debug, Clone, Copy)]
struct ArchiveTableSpec {
    dataset: &'static str,
    columns: &'static str,
    create_sql: &'static str,
}

#[derive(Debug)]
struct ArchiveBatchOutcome {
    dataset: &'static str,
    month_key: String,
    file_path: String,
    sha256: String,
    row_count: i64,
}

#[derive(Debug, Default)]
struct InvocationRollupDelta {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
}

#[derive(Debug, FromRow)]
struct InvocationDetailPruneCandidate {
    id: i64,
    occurred_at: String,
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
}

#[derive(Debug, FromRow, Clone)]
struct InvocationArchiveCandidate {
    id: i64,
    occurred_at: String,
    source: String,
    status: Option<String>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
}

#[derive(Debug, FromRow, Clone)]
struct TimestampedArchiveCandidate {
    id: i64,
    timestamp_value: String,
}

#[derive(Debug, FromRow)]
struct DryRunBatchCount {
    month_key: String,
    row_count: i64,
}

const CODEX_INVOCATIONS_ARCHIVE_COLUMNS: &str = "id, invoke_id, occurred_at, source, model, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, status, error_message, failure_kind, failure_class, is_actionable, payload, raw_response, cost_estimated, price_version, request_raw_path, request_raw_size, request_raw_truncated, request_raw_truncated_reason, response_raw_path, response_raw_size, response_raw_truncated, response_raw_truncated_reason, raw_expires_at, detail_level, detail_pruned_at, detail_prune_reason, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, created_at";
const FORWARD_PROXY_ATTEMPTS_ARCHIVE_COLUMNS: &str =
    "id, proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe";
const STATS_SOURCE_SNAPSHOTS_ARCHIVE_COLUMNS: &str = "id, source, period, stats_date, model, requests, input_tokens, output_tokens, cache_create_tokens, cache_read_tokens, all_tokens, cost_input, cost_output, cost_cache_write, cost_cache_read, cost_total, raw_response, captured_at, captured_at_epoch, created_at";
const CODEX_QUOTA_SNAPSHOTS_ARCHIVE_COLUMNS: &str = "id, captured_at, amount_limit, used_amount, remaining_amount, period, period_reset_time, expire_time, is_active, total_cost, total_requests, total_tokens, last_request_time, billing_type, remaining_count, used_count, sub_type_name";

const CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.codex_invocations (
    id INTEGER PRIMARY KEY,
    invoke_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'xy',
    model TEXT,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_input_tokens INTEGER,
    reasoning_tokens INTEGER,
    total_tokens INTEGER,
    cost REAL,
    status TEXT,
    error_message TEXT,
    failure_kind TEXT,
    failure_class TEXT,
    is_actionable INTEGER NOT NULL DEFAULT 0,
    payload TEXT,
    raw_response TEXT NOT NULL,
    cost_estimated INTEGER NOT NULL DEFAULT 0,
    price_version TEXT,
    request_raw_path TEXT,
    request_raw_size INTEGER,
    request_raw_truncated INTEGER NOT NULL DEFAULT 0,
    request_raw_truncated_reason TEXT,
    response_raw_path TEXT,
    response_raw_size INTEGER,
    response_raw_truncated INTEGER NOT NULL DEFAULT 0,
    response_raw_truncated_reason TEXT,
    raw_expires_at TEXT,
    detail_level TEXT NOT NULL DEFAULT 'full',
    detail_pruned_at TEXT,
    detail_prune_reason TEXT,
    t_total_ms REAL,
    t_req_read_ms REAL,
    t_req_parse_ms REAL,
    t_upstream_connect_ms REAL,
    t_upstream_ttfb_ms REAL,
    t_upstream_stream_ms REAL,
    t_resp_parse_ms REAL,
    t_persist_ms REAL,
    created_at TEXT NOT NULL,
    UNIQUE(invoke_id, occurred_at)
)
"#;

const FORWARD_PROXY_ATTEMPTS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.forward_proxy_attempts (
    id INTEGER PRIMARY KEY,
    proxy_key TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    is_success INTEGER NOT NULL,
    latency_ms REAL,
    failure_kind TEXT,
    is_probe INTEGER NOT NULL DEFAULT 0
)
"#;

const STATS_SOURCE_SNAPSHOTS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.stats_source_snapshots (
    id INTEGER PRIMARY KEY,
    source TEXT NOT NULL,
    period TEXT NOT NULL,
    stats_date TEXT NOT NULL,
    model TEXT,
    requests INTEGER NOT NULL,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cache_create_tokens INTEGER,
    cache_read_tokens INTEGER,
    all_tokens INTEGER,
    cost_input REAL,
    cost_output REAL,
    cost_cache_write REAL,
    cost_cache_read REAL,
    cost_total REAL,
    raw_response TEXT,
    captured_at TEXT NOT NULL,
    captured_at_epoch INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(source, period, stats_date, model, captured_at_epoch)
)
"#;

const CODEX_QUOTA_SNAPSHOTS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.codex_quota_snapshots (
    id INTEGER PRIMARY KEY,
    captured_at TEXT NOT NULL,
    amount_limit REAL,
    used_amount REAL,
    remaining_amount REAL,
    period TEXT,
    period_reset_time TEXT,
    expire_time TEXT,
    is_active INTEGER,
    total_cost REAL,
    total_requests INTEGER,
    total_tokens INTEGER,
    last_request_time TEXT,
    billing_type TEXT,
    remaining_count INTEGER,
    used_count INTEGER,
    sub_type_name TEXT
)
"#;

fn archive_table_spec(dataset: &'static str) -> ArchiveTableSpec {
    match dataset {
        "codex_invocations" => ArchiveTableSpec {
            dataset,
            columns: CODEX_INVOCATIONS_ARCHIVE_COLUMNS,
            create_sql: CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL,
        },
        "forward_proxy_attempts" => ArchiveTableSpec {
            dataset,
            columns: FORWARD_PROXY_ATTEMPTS_ARCHIVE_COLUMNS,
            create_sql: FORWARD_PROXY_ATTEMPTS_ARCHIVE_CREATE_SQL,
        },
        "stats_source_snapshots" => ArchiveTableSpec {
            dataset,
            columns: STATS_SOURCE_SNAPSHOTS_ARCHIVE_COLUMNS,
            create_sql: STATS_SOURCE_SNAPSHOTS_ARCHIVE_CREATE_SQL,
        },
        "codex_quota_snapshots" => ArchiveTableSpec {
            dataset,
            columns: CODEX_QUOTA_SNAPSHOTS_ARCHIVE_COLUMNS,
            create_sql: CODEX_QUOTA_SNAPSHOTS_ARCHIVE_CREATE_SQL,
        },
        other => panic!("unsupported archive dataset: {other}"),
    }
}

fn spawn_data_retention_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !state.config.retention_enabled {
            info!("data retention maintenance is disabled");
            cancel.cancelled().await;
            return;
        }

        if let Err(err) = run_data_retention_maintenance(&state.pool, &state.config, None).await {
            warn!(error = %err, "failed to run retention maintenance at startup");
        }

        let mut ticker = interval(state.config.retention_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("data retention maintenance received shutdown");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(err) = run_data_retention_maintenance(&state.pool, &state.config, None).await {
                        warn!(error = %err, "failed to run retention maintenance");
                    }
                }
            }
        }
    })
}

async fn run_data_retention_maintenance(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
) -> Result<RetentionRunSummary> {
    let dry_run = dry_run_override.unwrap_or(config.retention_dry_run);
    let mut summary = RetentionRunSummary {
        dry_run,
        ..RetentionRunSummary::default()
    };
    let raw_path_fallback_root = config.database_path.parent();

    let pruned =
        prune_old_invocation_details(pool, config, raw_path_fallback_root, dry_run).await?;
    summary.invocation_details_pruned += pruned.0;
    summary.archive_batches_touched += pruned.1;
    summary.raw_files_removed += pruned.2;

    let invocation_archive =
        archive_old_invocations(pool, config, raw_path_fallback_root, dry_run).await?;
    summary.invocation_rows_archived += invocation_archive.0;
    summary.archive_batches_touched += invocation_archive.1;
    summary.raw_files_removed += invocation_archive.2;

    let proxy_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("forward_proxy_attempts"),
        "SELECT id, occurred_at AS timestamp_value FROM forward_proxy_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
        shanghai_utc_cutoff_string(config.forward_proxy_attempts_retention_days),
        dry_run,
    )
    .await?;
    summary.forward_proxy_attempt_rows_archived += proxy_archive.0;
    summary.archive_batches_touched += proxy_archive.1;

    let snapshot_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("stats_source_snapshots"),
        "SELECT id, captured_at AS timestamp_value FROM stats_source_snapshots WHERE captured_at < ?1 ORDER BY captured_at ASC, id ASC LIMIT ?2",
        shanghai_utc_cutoff_string(config.stats_source_snapshots_retention_days),
        dry_run,
    )
    .await?;
    summary.stats_source_snapshot_rows_archived += snapshot_archive.0;
    summary.archive_batches_touched += snapshot_archive.1;

    let quota_archive = compact_old_quota_snapshots(pool, config, dry_run).await?;
    summary.quota_snapshot_rows_archived += quota_archive.0;
    summary.archive_batches_touched += quota_archive.1;

    summary.orphan_raw_files_removed +=
        sweep_orphan_proxy_raw_files(pool, config, raw_path_fallback_root, dry_run).await?;

    if !dry_run && summary.touched_anything() {
        sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
            .execute(pool)
            .await
            .context("failed to run retention wal checkpoint")?;
        sqlx::query("PRAGMA optimize")
            .execute(pool)
            .await
            .context("failed to run retention optimize pragma")?;
    }

    info!(
        dry_run = summary.dry_run,
        ?summary,
        "data retention maintenance finished"
    );
    Ok(summary)
}

async fn prune_old_invocation_details(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<(usize, usize, usize)> {
    let prune_cutoff = shanghai_local_cutoff_string(config.invocation_success_full_days);
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let spec = archive_table_spec("codex_invocations");
    if dry_run {
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(
            r#"
            SELECT id, occurred_at, request_raw_path, response_raw_path
            FROM codex_invocations
            WHERE status = 'success'
              AND detail_level = ?1
              AND occurred_at < ?2
              AND occurred_at >= ?3
            ORDER BY occurred_at ASC, id ASC
            "#,
        )
        .bind(DETAIL_LEVEL_FULL)
        .bind(&prune_cutoff)
        .bind(&archive_cutoff)
        .fetch_all(pool)
        .await?;
        let mut by_month: BTreeMap<String, usize> = BTreeMap::new();
        for candidate in &candidates {
            let month_key = shanghai_month_key_from_local_naive(&candidate.occurred_at)?;
            *by_month.entry(month_key).or_default() += 1;
        }
        for (month_key, rows) in &by_month {
            info!(
                dataset = spec.dataset,
                month_key,
                rows = *rows,
                reason = DETAIL_PRUNE_REASON_SUCCESS_OVER_30D,
                "retention dry-run planned invocation detail prune archive batch"
            );
        }
        let raw_paths = candidates
            .iter()
            .flat_map(|candidate| {
                [
                    candidate.request_raw_path.clone(),
                    candidate.response_raw_path.clone(),
                ]
            })
            .collect::<Vec<_>>();
        return Ok((
            candidates.len(),
            by_month.len(),
            count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
        ));
    }

    let mut rows_pruned = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

    loop {
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(
            r#"
            SELECT id, occurred_at, request_raw_path, response_raw_path
            FROM codex_invocations
            WHERE status = 'success'
              AND detail_level = ?1
              AND occurred_at < ?2
              AND occurred_at >= ?3
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?4
            "#,
        )
        .bind(DETAIL_LEVEL_FULL)
        .bind(&prune_cutoff)
        .bind(&archive_cutoff)
        .bind(config.retention_batch_rows as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut by_month: BTreeMap<String, Vec<InvocationDetailPruneCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key = shanghai_month_key_from_local_naive(&candidate.occurred_at)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            rows_pruned += group.len();
            archive_batches += 1;
            let raw_paths = group
                .iter()
                .flat_map(|candidate| {
                    [
                        candidate.request_raw_path.clone(),
                        candidate.response_raw_path.clone(),
                    ]
                })
                .collect::<Vec<_>>();

            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let archive_outcome =
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await?;
            let pruned_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            let mut query = QueryBuilder::<Sqlite>::new(
                "UPDATE codex_invocations SET payload = NULL, raw_response = '', request_raw_path = NULL, request_raw_size = NULL, request_raw_truncated = 0, request_raw_truncated_reason = NULL, response_raw_path = NULL, response_raw_size = NULL, response_raw_truncated = 0, response_raw_truncated_reason = NULL, raw_expires_at = NULL, detail_level = ",
            );
            query
                .push_bind(DETAIL_LEVEL_STRUCTURED_ONLY)
                .push(", detail_pruned_at = ")
                .push_bind(pruned_at)
                .push(", detail_prune_reason = ")
                .push_bind(DETAIL_PRUNE_REASON_SUCCESS_OVER_30D)
                .push(" WHERE id IN (");
            {
                let mut separated = query.separated(", ");
                for id in &ids {
                    separated.push_bind(id);
                }
            }
            query.push(")");
            query.build().execute(tx.as_mut()).await?;
            tx.commit().await?;

            raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
        }
    }

    Ok((rows_pruned, archive_batches, raw_files_removed))
}

async fn archive_old_invocations(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<(usize, usize, usize)> {
    let cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let spec = archive_table_spec("codex_invocations");

    if dry_run {
        let candidates = sqlx::query_as::<_, InvocationArchiveCandidate>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                request_raw_path,
                response_raw_path
            FROM codex_invocations
            WHERE occurred_at < ?1
            ORDER BY occurred_at ASC, id ASC
            "#,
        )
        .bind(&cutoff)
        .fetch_all(pool)
        .await?;

        let mut by_month: BTreeMap<String, usize> = BTreeMap::new();
        for candidate in &candidates {
            let month_key = shanghai_month_key_from_local_naive(&candidate.occurred_at)?;
            *by_month.entry(month_key).or_default() += 1;
        }
        for (month_key, rows) in &by_month {
            info!(
                dataset = spec.dataset,
                month_key,
                rows = *rows,
                reason = DETAIL_PRUNE_REASON_MAX_AGE_ARCHIVED,
                "retention dry-run planned invocation archive batch"
            );
        }
        let raw_paths = candidates
            .iter()
            .flat_map(|candidate| {
                [
                    candidate.request_raw_path.clone(),
                    candidate.response_raw_path.clone(),
                ]
            })
            .collect::<Vec<_>>();
        return Ok((
            candidates.len(),
            by_month.len(),
            count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

    loop {
        let candidates = sqlx::query_as::<_, InvocationArchiveCandidate>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                request_raw_path,
                response_raw_path
            FROM codex_invocations
            WHERE occurred_at < ?1
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?2
            "#,
        )
        .bind(&cutoff)
        .bind(config.retention_batch_rows as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut by_month: BTreeMap<String, Vec<InvocationArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key = shanghai_month_key_from_local_naive(&candidate.occurred_at)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            rows_archived += group.len();
            archive_batches += 1;
            let raw_paths = group
                .iter()
                .flat_map(|candidate| {
                    [
                        candidate.request_raw_path.clone(),
                        candidate.response_raw_path.clone(),
                    ]
                })
                .collect::<Vec<_>>();

            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let archive_outcome =
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await?;
            let mut tx = pool.begin().await?;
            upsert_invocation_rollups(tx.as_mut(), &group).await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            tx.commit().await?;
            raw_files_removed += delete_proxy_raw_paths(&raw_paths, raw_path_fallback_root)?;
        }
    }

    Ok((rows_archived, archive_batches, raw_files_removed))
}

async fn archive_timestamped_dataset(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    select_sql: &str,
    cutoff: String,
    dry_run: bool,
) -> Result<(usize, usize)> {
    if dry_run {
        let dry_run_sql = match spec.dataset {
            "forward_proxy_attempts" => {
                r#"
                SELECT strftime('%Y-%m', datetime(occurred_at, '+8 hours')) AS month_key,
                       COUNT(*) AS row_count
                FROM forward_proxy_attempts
                WHERE occurred_at < ?1
                GROUP BY 1
                ORDER BY 1
                "#
            }
            "stats_source_snapshots" => {
                r#"
                SELECT strftime('%Y-%m', datetime(captured_at, '+8 hours')) AS month_key,
                       COUNT(*) AS row_count
                FROM stats_source_snapshots
                WHERE captured_at < ?1
                GROUP BY 1
                ORDER BY 1
                "#
            }
            other => bail!("unsupported dry-run archive dataset: {other}"),
        };
        let batch_counts = sqlx::query_as::<_, DryRunBatchCount>(dry_run_sql)
            .bind(&cutoff)
            .fetch_all(pool)
            .await?;
        for batch in &batch_counts {
            info!(
                dataset = spec.dataset,
                month_key = %batch.month_key,
                rows = batch.row_count,
                "retention dry-run planned archive batch"
            );
        }
        return Ok((
            batch_counts
                .iter()
                .map(|batch| batch.row_count as usize)
                .sum(),
            batch_counts.len(),
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;

    loop {
        let candidates = sqlx::query_as::<_, TimestampedArchiveCandidate>(select_sql)
            .bind(&cutoff)
            .bind(config.retention_batch_rows as i64)
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key = shanghai_month_key_from_utc_naive(&candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            rows_archived += group.len();
            archive_batches += 1;
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let archive_outcome =
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await?;
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            tx.commit().await?;
        }
    }

    Ok((rows_archived, archive_batches))
}

async fn compact_old_quota_snapshots(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<(usize, usize)> {
    let cutoff = shanghai_utc_cutoff_string(config.quota_snapshot_full_days);
    let spec = archive_table_spec("codex_quota_snapshots");

    if dry_run {
        let batch_counts = sqlx::query_as::<_, DryRunBatchCount>(
            r#"
            WITH ranked AS (
                SELECT
                    captured_at,
                    ROW_NUMBER() OVER (
                        PARTITION BY strftime('%Y-%m-%d', datetime(captured_at, '+8 hours'))
                        ORDER BY captured_at DESC, id DESC
                    ) AS row_num
                FROM codex_quota_snapshots
                WHERE captured_at < ?1
            )
            SELECT strftime('%Y-%m', datetime(captured_at, '+8 hours')) AS month_key,
                   COUNT(*) AS row_count
            FROM ranked
            WHERE row_num > 1
            GROUP BY 1
            ORDER BY 1
            "#,
        )
        .bind(&cutoff)
        .fetch_all(pool)
        .await?;
        for batch in &batch_counts {
            info!(
                dataset = spec.dataset,
                month_key = %batch.month_key,
                rows = batch.row_count,
                "retention dry-run planned quota compaction batch"
            );
        }
        return Ok((
            batch_counts
                .iter()
                .map(|batch| batch.row_count as usize)
                .sum(),
            batch_counts.len(),
        ));
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;

    loop {
        let candidates = sqlx::query_as::<_, TimestampedArchiveCandidate>(
            r#"
            WITH ranked AS (
                SELECT
                    id,
                    captured_at AS timestamp_value,
                    ROW_NUMBER() OVER (
                        PARTITION BY strftime('%Y-%m-%d', datetime(captured_at, '+8 hours'))
                        ORDER BY captured_at DESC, id DESC
                    ) AS row_num
                FROM codex_quota_snapshots
                WHERE captured_at < ?1
            )
            SELECT id, timestamp_value
            FROM ranked
            WHERE row_num > 1
            ORDER BY timestamp_value ASC, id ASC
            LIMIT ?2
            "#,
        )
        .bind(&cutoff)
        .bind(config.retention_batch_rows as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key = shanghai_month_key_from_utc_naive(&candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            rows_archived += group.len();
            archive_batches += 1;
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let archive_outcome =
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await?;
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            tx.commit().await?;
        }
    }

    Ok((rows_archived, archive_batches))
}

async fn ensure_sqlite_file_initialized(path: &Path) -> Result<()> {
    let database_url = format!("sqlite://{}", path.to_string_lossy());
    let connect_opts = build_sqlite_connect_options(
        &database_url,
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )?;
    let connection = SqliteConnection::connect_with(&connect_opts)
        .await
        .with_context(|| format!("failed to initialize sqlite file {}", path.display()))?;
    connection.close().await?;
    Ok(())
}

async fn archive_rows_into_month_batch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    month_key: &str,
    ids: &[i64],
) -> Result<ArchiveBatchOutcome> {
    if ids.is_empty() {
        bail!("archive batch requires at least one row id");
    }

    let final_path = archive_batch_file_path(config, spec.dataset, month_key)?;
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create archive directory: {}", parent.display()))?;
    }

    let suffix = retention_temp_suffix();
    let work_path = PathBuf::from(format!("{}.{}.sqlite", final_path.display(), suffix));
    let temp_gzip_path = PathBuf::from(format!("{}.{}.tmp", final_path.display(), suffix));

    if work_path.exists() {
        let _ = fs::remove_file(&work_path);
    }
    if temp_gzip_path.exists() {
        let _ = fs::remove_file(&temp_gzip_path);
    }

    if final_path.exists() {
        inflate_gzip_sqlite_file(&final_path, &work_path)?;
    }
    if !work_path.exists() {
        ensure_sqlite_file_initialized(&work_path).await?;
    }

    let row_count = async {
        let mut conn = pool.acquire().await?;
        sqlx::query("ATTACH DATABASE ?1 AS archive_db")
            .bind(work_path.to_string_lossy().to_string())
            .execute(&mut *conn)
            .await
            .with_context(|| {
                format!("failed to attach archive database {}", work_path.display())
            })?;
        sqlx::query(spec.create_sql)
            .execute(&mut *conn)
            .await
            .with_context(|| format!("failed to ensure archive schema for {}", spec.dataset))?;

        let mut insert = QueryBuilder::<Sqlite>::new(format!(
            "INSERT OR IGNORE INTO archive_db.{} ({}) SELECT {} FROM main.{} WHERE id IN (",
            spec.dataset, spec.columns, spec.columns, spec.dataset
        ));
        {
            let mut separated = insert.separated(", ");
            for id in ids {
                separated.push_bind(id);
            }
        }
        insert.push(")");
        insert.build().execute(&mut *conn).await.with_context(|| {
            format!(
                "failed to copy rows into archive batch for {}",
                spec.dataset
            )
        })?;

        let count_query = format!("SELECT COUNT(*) FROM archive_db.{}", spec.dataset);
        let row_count = sqlx::query_scalar::<_, i64>(&count_query)
            .fetch_one(&mut *conn)
            .await
            .with_context(|| format!("failed to count archive rows for {}", spec.dataset))?;
        sqlx::query("DETACH DATABASE archive_db")
            .execute(&mut *conn)
            .await
            .context("failed to detach archive database")?;
        Ok::<i64, anyhow::Error>(row_count)
    }
    .await;

    let result = match row_count {
        Ok(row_count) => row_count,
        Err(err) => {
            let _ = fs::remove_file(&work_path);
            let _ = fs::remove_file(&temp_gzip_path);
            return Err(err);
        }
    };

    deflate_sqlite_file_to_gzip(&work_path, &temp_gzip_path)?;
    fs::rename(&temp_gzip_path, &final_path).with_context(|| {
        format!(
            "failed to move archive batch into place: {} -> {}",
            temp_gzip_path.display(),
            final_path.display()
        )
    })?;
    let _ = fs::remove_file(&work_path);

    let sha256 = sha256_hex_file(&final_path)?;
    Ok(ArchiveBatchOutcome {
        dataset: spec.dataset,
        month_key: month_key.to_string(),
        file_path: final_path.to_string_lossy().to_string(),
        sha256,
        row_count: result,
    })
}

async fn upsert_archive_batch_manifest(
    tx: &mut sqlx::SqliteConnection,
    batch: &ArchiveBatchOutcome,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        ON CONFLICT(dataset, month_key, file_path) DO UPDATE SET
            sha256 = excluded.sha256,
            row_count = excluded.row_count,
            status = excluded.status,
            created_at = datetime('now')
        "#,
    )
    .bind(batch.dataset)
    .bind(&batch.month_key)
    .bind(&batch.file_path)
    .bind(&batch.sha256)
    .bind(batch.row_count)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn upsert_invocation_rollups(
    tx: &mut sqlx::SqliteConnection,
    candidates: &[InvocationArchiveCandidate],
) -> Result<()> {
    let mut rollups: BTreeMap<(String, String), InvocationRollupDelta> = BTreeMap::new();
    for candidate in candidates {
        let stats_date = shanghai_day_key_from_local_naive(&candidate.occurred_at)?;
        let key = (stats_date, candidate.source.clone());
        let entry = rollups.entry(key).or_default();
        entry.total_count += 1;
        if matches!(candidate.status.as_deref(), Some("success")) {
            entry.success_count += 1;
        } else if candidate
            .status
            .as_deref()
            .is_some_and(|status| status != "success")
        {
            entry.failure_count += 1;
        }
        entry.total_tokens += candidate.total_tokens.unwrap_or_default();
        entry.total_cost += candidate.cost.unwrap_or_default();
    }

    for ((stats_date, source), delta) in rollups {
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
            ON CONFLICT(stats_date, source) DO UPDATE SET
                total_count = invocation_rollup_daily.total_count + excluded.total_count,
                success_count = invocation_rollup_daily.success_count + excluded.success_count,
                failure_count = invocation_rollup_daily.failure_count + excluded.failure_count,
                total_tokens = invocation_rollup_daily.total_tokens + excluded.total_tokens,
                total_cost = invocation_rollup_daily.total_cost + excluded.total_cost
            "#,
        )
        .bind(&stats_date)
        .bind(&source)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.total_tokens)
        .bind(delta.total_cost)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

async fn delete_rows_by_ids(
    tx: &mut sqlx::SqliteConnection,
    table: &str,
    ids: &[i64],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE id IN ("));
    {
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    Ok(())
}

async fn sweep_orphan_proxy_raw_files(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<usize> {
    let raw_dir = config.resolved_proxy_raw_dir();
    if !raw_dir.exists() {
        return Ok(0);
    }

    let referenced = sqlx::query_scalar::<_, String>(
        r#"
        SELECT path
        FROM (
            SELECT request_raw_path AS path
            FROM codex_invocations
            WHERE request_raw_path IS NOT NULL
            UNION
            SELECT response_raw_path AS path
            FROM codex_invocations
            WHERE response_raw_path IS NOT NULL
        )
        WHERE path IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut referenced_paths = HashSet::new();
    for path in referenced {
        for candidate in resolved_raw_path_candidates(&path, raw_path_fallback_root) {
            referenced_paths.insert(candidate);
        }
    }

    let min_file_age = Duration::from_secs(DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS);
    let mut removed = 0usize;
    for entry in fs::read_dir(&raw_dir)
        .with_context(|| format!("failed to read raw payload directory {}", raw_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        let age = match entry.metadata().and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified.elapsed().unwrap_or_default(),
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to inspect orphan raw payload file age");
                continue;
            }
        };
        if age < min_file_age {
            continue;
        }
        let normalized = normalize_path_for_compare(&path);
        if referenced_paths.contains(&normalized) {
            continue;
        }
        if dry_run {
            removed += 1;
            continue;
        }
        match fs::remove_file(&path) {
            Ok(_) => removed += 1,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to remove orphan raw payload file");
            }
        }
    }

    Ok(removed)
}

fn resolved_raw_path_candidates(path: &str, fallback_root: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let primary = PathBuf::from(path);
    candidates.push(normalize_path_for_compare(&primary));
    if !primary.is_absolute()
        && let Some(root) = fallback_root
    {
        let fallback = root.join(&primary);
        let normalized = normalize_path_for_compare(&fallback);
        if !candidates.contains(&normalized) {
            candidates.push(normalized);
        }
    }
    candidates
}

fn normalize_path_for_compare(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn count_existing_proxy_raw_paths(
    raw_paths: &[Option<String>],
    raw_path_fallback_root: Option<&Path>,
) -> usize {
    let mut seen = HashSet::new();
    for raw_path in raw_paths.iter().flatten() {
        for candidate in resolved_raw_path_candidates(raw_path, raw_path_fallback_root) {
            if candidate.exists() {
                seen.insert(candidate);
                break;
            }
        }
    }
    seen.len()
}

fn delete_proxy_raw_paths(
    raw_paths: &[Option<String>],
    raw_path_fallback_root: Option<&Path>,
) -> Result<usize> {
    let mut removed = 0usize;
    let mut seen = HashSet::new();
    for raw_path in raw_paths.iter().flatten() {
        for candidate in resolved_raw_path_candidates(raw_path, raw_path_fallback_root) {
            if !seen.insert(candidate.clone()) {
                continue;
            }
            match fs::remove_file(&candidate) {
                Ok(_) => {
                    removed += 1;
                    break;
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    warn!(path = %candidate.display(), error = %err, "failed to remove raw payload file");
                    break;
                }
            }
        }
    }
    Ok(removed)
}

fn shanghai_retention_cutoff(days: u64) -> DateTime<Utc> {
    start_of_local_day(Utc::now(), Shanghai) - ChronoDuration::days(days as i64)
}

fn shanghai_local_cutoff_string(days: u64) -> String {
    format_naive(
        shanghai_retention_cutoff(days)
            .with_timezone(&Shanghai)
            .naive_local(),
    )
}

fn shanghai_utc_cutoff_string(days: u64) -> String {
    format_naive(shanghai_retention_cutoff(days).naive_utc())
}

fn parse_shanghai_local_naive(value: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("invalid shanghai-local timestamp: {value}"))
}

fn parse_utc_naive(value: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("invalid utc timestamp: {value}"))
}

fn shanghai_day_key_from_local_naive(value: &str) -> Result<String> {
    Ok(parse_shanghai_local_naive(value)?
        .format("%Y-%m-%d")
        .to_string())
}

fn shanghai_month_key_from_local_naive(value: &str) -> Result<String> {
    Ok(parse_shanghai_local_naive(value)?
        .format("%Y-%m")
        .to_string())
}

fn shanghai_month_key_from_utc_naive(value: &str) -> Result<String> {
    let utc = Utc.from_utc_datetime(&parse_utc_naive(value)?);
    Ok(utc.with_timezone(&Shanghai).format("%Y-%m").to_string())
}

fn resolved_archive_dir(config: &AppConfig) -> PathBuf {
    resolve_path_from_database_parent(&config.database_path, &config.archive_dir)
}

fn resolve_path_from_database_parent(database_path: &Path, configured_path: &Path) -> PathBuf {
    if configured_path.is_absolute() {
        return configured_path.to_path_buf();
    }

    match database_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(configured_path),
        _ => configured_path.to_path_buf(),
    }
}

fn archive_batch_file_path(config: &AppConfig, dataset: &str, month_key: &str) -> Result<PathBuf> {
    let year = month_key
        .split('-')
        .next()
        .filter(|segment| segment.len() == 4)
        .ok_or_else(|| anyhow!("invalid month key: {month_key}"))?;
    Ok(resolved_archive_dir(config)
        .join(dataset)
        .join(year)
        .join(format!("{dataset}-{month_key}.sqlite.gz")))
}

fn retention_temp_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

fn inflate_gzip_sqlite_file(source: &Path, destination: &Path) -> Result<()> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open archive batch {}", source.display()))?;
    let mut decoder = GzDecoder::new(input);
    let output = fs::File::create(destination)
        .with_context(|| format!("failed to create temp archive db {}", destination.display()))?;
    let mut writer = io::BufWriter::new(output);
    io::copy(&mut decoder, &mut writer).with_context(|| {
        format!(
            "failed to decompress archive batch {} into {}",
            source.display(),
            destination.display()
        )
    })?;
    writer.flush()?;
    Ok(())
}

fn deflate_sqlite_file_to_gzip(source: &Path, destination: &Path) -> Result<()> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open temp archive db {}", source.display()))?;
    let output = fs::File::create(destination)
        .with_context(|| format!("failed to create archive gzip {}", destination.display()))?;
    let mut encoder = GzEncoder::new(io::BufWriter::new(output), Compression::default());
    let mut reader = io::BufReader::new(input);
    io::copy(&mut reader, &mut encoder).with_context(|| {
        format!(
            "failed to compress temp archive db {} into {}",
            source.display(),
            destination.display()
        )
    })?;
    let mut writer = encoder
        .finish()
        .context("failed to finish archive gzip writer")?;
    writer.flush()?;
    Ok(())
}

fn sha256_hex_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for sha256 {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

async fn schedule_poll(state: Arc<AppState>) -> Result<JoinHandle<()>> {
    let permit = state
        .semaphore
        .clone()
        .acquire_owned()
        .await
        .context("failed to acquire scheduler permit")?;

    let in_flight = state
        .config
        .max_parallel_polls
        .saturating_sub(state.semaphore.available_permits());
    let force_new_connection = in_flight > state.config.shared_connection_parallelism;
    let state_clone = state.clone();

    let handle = tokio::spawn(async move {
        let collect_broadcast_state = state_clone.broadcaster.receiver_count() > 0;
        let fut = fetch_and_store(&state_clone, force_new_connection, collect_broadcast_state);
        match timeout(state_clone.config.request_timeout, fut).await {
            Ok(Ok(publish)) => {
                let PublishResult {
                    mut summaries,
                    mut quota_snapshot,
                    collected_broadcast_state,
                } = publish;

                let receiver_count = state_clone.broadcaster.receiver_count();
                if should_collect_late_broadcast_state(receiver_count, collected_broadcast_state) {
                    match collect_broadcast_state_snapshots(
                        &state_clone.pool,
                        state_clone.config.crs_stats.as_ref(),
                    )
                    .await
                    {
                        Ok((latest_summaries, latest_quota_snapshot)) => {
                            summaries = latest_summaries;
                            quota_snapshot = latest_quota_snapshot;
                        }
                        Err(err) => {
                            warn!(?err, "failed to collect late-subscriber broadcast state");
                        }
                    }
                }

                for summary in summaries {
                    if let Err(err) = broadcast_summary_if_changed(
                        &state_clone.broadcaster,
                        state_clone.broadcast_state_cache.as_ref(),
                        &summary.window,
                        summary.summary,
                    )
                    .await
                    {
                        warn!(?err, "failed to broadcast summary payload");
                    }
                }

                if let Some(snapshot) = quota_snapshot
                    && let Err(err) = broadcast_quota_if_changed(
                        &state_clone.broadcaster,
                        state_clone.broadcast_state_cache.as_ref(),
                        snapshot,
                    )
                    .await
                {
                    warn!(?err, "failed to broadcast quota snapshot");
                }
            }
            Ok(Err(err)) => {
                warn!(?err, "poll execution failed");
            }
            Err(_) => {
                warn!("scheduler fetch timed out");
            }
        }

        drop(permit);
    });

    Ok(handle)
}

async fn spawn_http_server(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> Result<JoinHandle<()>> {
    let cors_layer = build_cors_layer(&state.config);
    let mut router = Router::new()
        .route("/health", get(health_check))
        .route("/api/version", get(get_versions))
        .route("/api/settings", get(get_settings))
        .route(
            "/api/settings/proxy-models",
            any(removed_proxy_model_settings_endpoint),
        )
        .route("/api/settings/proxy", put(put_proxy_settings))
        .route(
            "/api/settings/forward-proxy",
            put(put_forward_proxy_settings),
        )
        .route(
            "/api/settings/forward-proxy/validate",
            post(post_forward_proxy_candidate_validation),
        )
        .route("/api/settings/pricing", put(put_pricing_settings))
        .route("/api/invocations", get(list_invocations))
        .route("/api/stats", get(fetch_stats))
        .route("/api/stats/summary", get(fetch_summary))
        .route(
            "/api/stats/forward-proxy",
            get(fetch_forward_proxy_live_stats),
        )
        .route("/api/stats/timeseries", get(fetch_timeseries))
        .route("/api/stats/perf", get(fetch_perf_stats))
        .route("/api/stats/errors", get(fetch_error_distribution))
        .route("/api/stats/failures/summary", get(fetch_failure_summary))
        .route("/api/stats/errors/others", get(fetch_other_errors))
        .route(
            "/api/stats/prompt-cache-conversations",
            get(fetch_prompt_cache_conversations),
        )
        .route("/api/quota/latest", get(latest_quota_snapshot))
        .route("/events", get(sse_stream))
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone())
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer);

    // Optionally attach headers in the future; standard EventSource cannot read headers

    if let Some(static_dir) = state.config.static_dir.clone() {
        let index_file = static_dir.join("index.html");
        if index_file.exists() {
            let spa_service =
                ServeDir::new(static_dir).not_found_service(ServeFile::new(index_file));
            router = router.fallback_service(spa_service);
        } else {
            warn!(
                path = %index_file.display(),
                "static index.html not found; SPA fallback disabled"
            );
        }
    }

    let listener = TcpListener::bind(&state.config.http_bind).await?;
    let addr = listener.local_addr()?;
    info!(%addr, "http server listening");
    state.startup_ready.store(true, Ordering::Release);

    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move { cancel.cancelled().await })
        .await
        {
            error!(?err, "http server exited with error");
        }
    });

    Ok(handle)
}

async fn shutdown_listener() {
    // Wait for Ctrl+C or SIGTERM (unix)
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

struct PublishResult {
    summaries: Vec<SummaryPublish>,
    quota_snapshot: Option<QuotaSnapshotResponse>,
    collected_broadcast_state: bool,
}

struct SummaryPublish {
    window: String,
    summary: StatsResponse,
}

fn should_collect_late_broadcast_state(
    receiver_count: usize,
    collected_broadcast_state: bool,
) -> bool {
    receiver_count > 0 && !collected_broadcast_state
}

async fn collect_broadcast_state_snapshots(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
) -> Result<(Vec<SummaryPublish>, Option<QuotaSnapshotResponse>)> {
    Ok((
        collect_summary_snapshots(pool, relay).await?,
        QuotaSnapshotResponse::fetch_latest(pool).await?,
    ))
}

async fn fetch_and_store(
    state: &AppState,
    force_new_connection: bool,
    collect_broadcast_state: bool,
) -> Result<PublishResult> {
    let client = state
        .http_clients
        .client_for_parallelism(force_new_connection)?;
    let relay_config = state.config.crs_stats.clone();

    if let Some(relay) = relay_config.as_ref()
        && should_poll_crs_stats(&state.pool, relay).await?
    {
        match fetch_crs_stats(&client, relay).await {
            Ok(payload) => {
                if let Err(err) = persist_crs_stats(&state.pool, relay, payload).await {
                    warn!(?err, "failed to persist crs stats");
                }
            }
            Err(err) => {
                warn!(?err, "failed to fetch crs stats");
            }
        }
    }

    let (summaries, quota_payload) = if collect_broadcast_state {
        collect_broadcast_state_snapshots(&state.pool, relay_config.as_ref()).await?
    } else {
        (Vec::new(), None)
    };

    Ok(PublishResult {
        summaries,
        quota_snapshot: quota_payload,
        collected_broadcast_state: collect_broadcast_state,
    })
}

struct SummaryBroadcastSpec {
    window: &'static str,
    duration: Option<ChronoDuration>,
}

fn summary_broadcast_specs() -> Vec<SummaryBroadcastSpec> {
    vec![
        SummaryBroadcastSpec {
            window: "all",
            duration: None,
        },
        SummaryBroadcastSpec {
            window: "30m",
            duration: Some(ChronoDuration::minutes(30)),
        },
        SummaryBroadcastSpec {
            window: "1h",
            duration: Some(ChronoDuration::hours(1)),
        },
        SummaryBroadcastSpec {
            window: "1d",
            duration: Some(ChronoDuration::days(1)),
        },
        SummaryBroadcastSpec {
            window: "1mo",
            duration: Some(ChronoDuration::days(30)),
        },
    ]
}

async fn collect_summary_snapshots(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
) -> Result<Vec<SummaryPublish>> {
    let mut summaries = Vec::new();
    let mut cached_all: Option<StatsResponse> = None;
    let now = Utc::now();
    let source_scope = resolve_default_source_scope(pool).await?;

    for spec in summary_broadcast_specs() {
        let summary = match spec.duration {
            None => {
                if let Some(existing) = &cached_all {
                    existing.clone()
                } else {
                    let stats = query_combined_totals(pool, relay, StatsFilter::All, source_scope)
                        .await?
                        .into_response();
                    cached_all = Some(stats.clone());
                    stats
                }
            }
            Some(duration) => {
                let start = now - duration;
                query_combined_totals(pool, relay, StatsFilter::Since(start), source_scope)
                    .await?
                    .into_response()
            }
        };

        summaries.push(SummaryPublish {
            window: spec.window.to_string(),
            summary,
        });
    }

    Ok(summaries)
}

async fn should_poll_crs_stats(pool: &Pool<Sqlite>, relay: &CrsStatsConfig) -> Result<bool> {
    let last_epoch = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT captured_at_epoch
        FROM stats_source_snapshots
        WHERE source = ?1 AND period = ?2 AND model IS NULL
        ORDER BY captured_at_epoch DESC
        LIMIT 1
        "#,
    )
    .bind(SOURCE_CRS)
    .bind(&relay.period)
    .fetch_optional(pool)
    .await?;

    let now_epoch = Utc::now().timestamp();
    Ok(match last_epoch {
        Some(last) => now_epoch.saturating_sub(last) >= relay.poll_interval.as_secs() as i64,
        None => true,
    })
}

async fn fetch_crs_stats(client: &Client, relay: &CrsStatsConfig) -> Result<CrsStatsResponse> {
    let url = relay
        .base_url
        .join("apiStats/api/user-model-stats")
        .context("failed to join crs stats endpoint")?;
    let payload = json!({
        "apiId": relay.api_id,
        "period": relay.period,
    });

    let response = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .context("failed to send crs stats request")?
        .error_for_status()
        .context("crs stats request returned error status")?;

    let payload: CrsStatsResponse = response
        .json()
        .await
        .context("failed to decode crs stats JSON")?;

    if !payload.success {
        return Err(anyhow!("crs stats responded with success=false"));
    }

    Ok(payload)
}

fn aggregate_crs_totals(models: &[CrsModelStats]) -> CrsTotals {
    let mut totals = CrsTotals::default();
    for model in models {
        totals.total_count += model.requests;
        totals.total_tokens += model.all_tokens;
        totals.total_cost += model.costs.total;
        totals.input_tokens += model.input_tokens;
        totals.output_tokens += model.output_tokens;
        totals.cache_create_tokens += model.cache_create_tokens;
        totals.cache_read_tokens += model.cache_read_tokens;
        totals.cost_input += model.costs.input;
        totals.cost_output += model.costs.output;
        totals.cost_cache_write += model.costs.cache_write;
        totals.cost_cache_read += model.costs.cache_read;
    }
    totals
}

#[derive(Debug, FromRow)]
struct CrsMaxRow {
    max_requests: Option<i64>,
    max_all_tokens: Option<i64>,
    max_cost_total: Option<f64>,
}

fn compute_crs_delta(
    stats_date: &str,
    now_utc: DateTime<Utc>,
    totals: CrsTotals,
    prev: CrsMaxRow,
) -> StatsTotals {
    let max_requests = prev.max_requests.unwrap_or(0);
    let max_tokens = prev.max_all_tokens.unwrap_or(0);
    let max_cost = prev.max_cost_total.unwrap_or(0.0);

    if totals.total_count < max_requests {
        if totals.total_count == 0 {
            let local = now_utc.with_timezone(&Shanghai);
            error!(
                stats_date,
                now = %local.to_rfc3339(),
                current = totals.total_count,
                previous_max = max_requests,
                "crs stats reset to zero outside day boundary"
            );
        } else {
            warn!(
                stats_date,
                current = totals.total_count,
                previous_max = max_requests,
                "crs stats total decreased; keeping daily max"
            );
        }
    }

    let delta_count = if totals.total_count > max_requests {
        totals.total_count - max_requests
    } else {
        0
    };
    let delta_tokens = if totals.total_tokens > max_tokens {
        totals.total_tokens - max_tokens
    } else {
        0
    };
    let delta_cost = if totals.total_cost > max_cost {
        totals.total_cost - max_cost
    } else {
        0.0
    };

    StatsTotals {
        total_count: delta_count,
        success_count: delta_count,
        failure_count: 0,
        total_tokens: delta_tokens,
        total_cost: delta_cost,
    }
}

async fn persist_crs_stats(
    pool: &Pool<Sqlite>,
    relay: &CrsStatsConfig,
    payload: CrsStatsResponse,
) -> Result<Option<StatsTotals>> {
    let now_utc = Utc::now();
    let captured_at = format_naive(now_utc.naive_utc());
    let captured_at_epoch = now_utc.timestamp();
    let stats_date = now_utc
        .with_timezone(&Shanghai)
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    let period = if payload.period.is_empty() {
        relay.period.clone()
    } else {
        payload.period.clone()
    };

    if period != relay.period {
        warn!(
            expected = %relay.period,
            actual = %period,
            "crs stats period mismatch; using response period"
        );
    }

    let totals = aggregate_crs_totals(&payload.data);
    let raw_response = serde_json::to_string(&payload)?;

    let mut tx = pool.begin().await?;
    let prev = sqlx::query_as::<_, CrsMaxRow>(
        r#"
        SELECT
            MAX(requests) AS max_requests,
            MAX(all_tokens) AS max_all_tokens,
            MAX(cost_total) AS max_cost_total
        FROM stats_source_snapshots
        WHERE source = ?1 AND period = ?2 AND stats_date = ?3 AND model IS NULL
        "#,
    )
    .bind(SOURCE_CRS)
    .bind(&period)
    .bind(&stats_date)
    .fetch_one(&mut *tx)
    .await?;

    for model in &payload.data {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO stats_source_snapshots (
                source,
                period,
                stats_date,
                model,
                requests,
                input_tokens,
                output_tokens,
                cache_create_tokens,
                cache_read_tokens,
                all_tokens,
                cost_input,
                cost_output,
                cost_cache_write,
                cost_cache_read,
                cost_total,
                raw_response,
                captured_at,
                captured_at_epoch
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            "#,
        )
        .bind(SOURCE_CRS)
        .bind(&period)
        .bind(&stats_date)
        .bind(&model.model)
        .bind(model.requests)
        .bind(model.input_tokens)
        .bind(model.output_tokens)
        .bind(model.cache_create_tokens)
        .bind(model.cache_read_tokens)
        .bind(model.all_tokens)
        .bind(model.costs.input)
        .bind(model.costs.output)
        .bind(model.costs.cache_write)
        .bind(model.costs.cache_read)
        .bind(model.costs.total)
        .bind(Option::<String>::None)
        .bind(&captured_at)
        .bind(captured_at_epoch)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO stats_source_snapshots (
            source,
            period,
            stats_date,
            model,
            requests,
            input_tokens,
            output_tokens,
            cache_create_tokens,
            cache_read_tokens,
            all_tokens,
            cost_input,
            cost_output,
            cost_cache_write,
            cost_cache_read,
            cost_total,
            raw_response,
            captured_at,
            captured_at_epoch
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
        "#,
    )
    .bind(SOURCE_CRS)
    .bind(&period)
    .bind(&stats_date)
    .bind(Option::<String>::None)
    .bind(totals.total_count)
    .bind(totals.input_tokens)
    .bind(totals.output_tokens)
    .bind(totals.cache_create_tokens)
    .bind(totals.cache_read_tokens)
    .bind(totals.total_tokens)
    .bind(totals.cost_input)
    .bind(totals.cost_output)
    .bind(totals.cost_cache_write)
    .bind(totals.cost_cache_read)
    .bind(totals.total_cost)
    .bind(raw_response)
    .bind(&captured_at)
    .bind(captured_at_epoch)
    .execute(&mut *tx)
    .await?;

    let delta = compute_crs_delta(&stats_date, now_utc, totals, prev);
    let has_delta = delta.total_count > 0 || delta.total_tokens > 0 || delta.total_cost > 0.0;
    if has_delta {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO stats_source_deltas (
                source,
                period,
                stats_date,
                captured_at,
                captured_at_epoch,
                total_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(SOURCE_CRS)
        .bind(&period)
        .bind(&stats_date)
        .bind(&captured_at)
        .bind(captured_at_epoch)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.total_tokens)
        .bind(delta.total_cost)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(if has_delta { Some(delta) } else { None })
}

async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS codex_invocations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'xy',
            model TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cache_input_tokens INTEGER,
            reasoning_tokens INTEGER,
            total_tokens INTEGER,
            cost REAL,
            status TEXT,
            error_message TEXT,
            failure_kind TEXT,
            failure_class TEXT,
            is_actionable INTEGER NOT NULL DEFAULT 0,
            payload TEXT,
            raw_response TEXT NOT NULL,
            cost_estimated INTEGER NOT NULL DEFAULT 0,
            price_version TEXT,
            request_raw_path TEXT,
            request_raw_size INTEGER,
            request_raw_truncated INTEGER NOT NULL DEFAULT 0,
            request_raw_truncated_reason TEXT,
            response_raw_path TEXT,
            response_raw_size INTEGER,
            response_raw_truncated INTEGER NOT NULL DEFAULT 0,
            response_raw_truncated_reason TEXT,
            raw_expires_at TEXT,
            detail_level TEXT NOT NULL DEFAULT 'full',
            detail_pruned_at TEXT,
            detail_prune_reason TEXT,
            t_total_ms REAL,
            t_req_read_ms REAL,
            t_req_parse_ms REAL,
            t_upstream_connect_ms REAL,
            t_upstream_ttfb_ms REAL,
            t_upstream_stream_ms REAL,
            t_resp_parse_ms REAL,
            t_persist_ms REAL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(invoke_id, occurred_at)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure codex_invocations table existence")?;

    let existing: HashSet<String> = sqlx::query("PRAGMA table_info('codex_invocations')")
        .fetch_all(pool)
        .await
        .context("failed to inspect codex_invocations schema")?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect();

    for (column, ty) in [
        ("source", "TEXT NOT NULL DEFAULT 'xy'"),
        ("model", "TEXT"),
        ("input_tokens", "INTEGER"),
        ("output_tokens", "INTEGER"),
        ("cache_input_tokens", "INTEGER"),
        ("reasoning_tokens", "INTEGER"),
        ("total_tokens", "INTEGER"),
        ("cost", "REAL"),
        ("status", "TEXT"),
        ("error_message", "TEXT"),
        ("failure_kind", "TEXT"),
        ("failure_class", "TEXT"),
        ("is_actionable", "INTEGER NOT NULL DEFAULT 0"),
        ("payload", "TEXT"),
        ("cost_estimated", "INTEGER NOT NULL DEFAULT 0"),
        ("price_version", "TEXT"),
        ("request_raw_path", "TEXT"),
        ("request_raw_size", "INTEGER"),
        ("request_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("request_raw_truncated_reason", "TEXT"),
        ("response_raw_path", "TEXT"),
        ("response_raw_size", "INTEGER"),
        ("response_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("response_raw_truncated_reason", "TEXT"),
        ("raw_expires_at", "TEXT"),
        ("detail_level", "TEXT NOT NULL DEFAULT 'full'"),
        ("detail_pruned_at", "TEXT"),
        ("detail_prune_reason", "TEXT"),
        ("t_total_ms", "REAL"),
        ("t_req_read_ms", "REAL"),
        ("t_req_parse_ms", "REAL"),
        ("t_upstream_connect_ms", "REAL"),
        ("t_upstream_ttfb_ms", "REAL"),
        ("t_upstream_stream_ms", "REAL"),
        ("t_resp_parse_ms", "REAL"),
        ("t_persist_ms", "REAL"),
    ] {
        if !existing.contains(column) {
            let statement = format!("ALTER TABLE codex_invocations ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| format!("failed to add column {column}"))?;
        }
    }

    // Speed up time-range scans and ordering on the stats endpoints
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_occurred_at
        ON codex_invocations (occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_occurred_at")?;

    // Benefit queries that filter by time and status (e.g., error distribution)
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_occurred_at_status
        ON codex_invocations (occurred_at, status)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_occurred_at_status")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_source_occurred_at
        ON codex_invocations (source, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_source_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_failure_class_occurred_at
        ON codex_invocations (failure_class, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_failure_class_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_prompt_cache_key_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_prompt_cache_key_occurred_at")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS codex_quota_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            captured_at TEXT NOT NULL DEFAULT (datetime('now')),
            amount_limit REAL,
            used_amount REAL,
            remaining_amount REAL,
            period TEXT,
            period_reset_time TEXT,
            expire_time TEXT,
            is_active INTEGER,
            total_cost REAL,
            total_requests INTEGER,
            total_tokens INTEGER,
            last_request_time TEXT,
            billing_type TEXT,
            remaining_count INTEGER,
            used_count INTEGER,
            sub_type_name TEXT
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure codex_quota_snapshots table existence")?;

    // Speed up latest snapshot lookup
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_quota_snapshots_captured_at
        ON codex_quota_snapshots (captured_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_quota_snapshots_captured_at")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS stats_source_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            period TEXT NOT NULL,
            stats_date TEXT NOT NULL,
            model TEXT,
            requests INTEGER NOT NULL,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cache_create_tokens INTEGER,
            cache_read_tokens INTEGER,
            all_tokens INTEGER,
            cost_input REAL,
            cost_output REAL,
            cost_cache_write REAL,
            cost_cache_read REAL,
            cost_total REAL,
            raw_response TEXT,
            captured_at TEXT NOT NULL,
            captured_at_epoch INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(source, period, stats_date, model, captured_at_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure stats_source_snapshots table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_stats_source_snapshots_date
        ON stats_source_snapshots (source, period, stats_date, captured_at_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_stats_source_snapshots_date")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS stats_source_deltas (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            period TEXT NOT NULL,
            stats_date TEXT NOT NULL,
            captured_at TEXT NOT NULL,
            captured_at_epoch INTEGER NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(source, period, stats_date, captured_at_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure stats_source_deltas table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_stats_source_deltas_epoch
        ON stats_source_deltas (source, period, captured_at_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_stats_source_deltas_epoch")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS archive_batches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            file_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(dataset, month_key, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batches table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_month
        ON archive_batches (dataset, month_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_month")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_rollup_daily (
            stats_date TEXT NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (stats_date, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_rollup_daily table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_rollup_daily_source_date
        ON invocation_rollup_daily (source, stats_date)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_rollup_daily_source_date")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_model_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            hijack_enabled INTEGER NOT NULL DEFAULT 0,
            merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
            fast_mode_rewrite_mode TEXT NOT NULL DEFAULT 'disabled',
            enabled_preset_models_json TEXT,
            preset_models_migrated INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy_model_settings table existence")?;

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN enabled_preset_models_json TEXT
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure enabled_preset_models_json column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN fast_mode_rewrite_mode TEXT NOT NULL DEFAULT 'disabled'
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure fast_mode_rewrite_mode column");
    }

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN preset_models_migrated INTEGER NOT NULL DEFAULT 0
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure preset_models_migrated column");
    }

    let default_enabled_models_json = serde_json::to_string(&default_enabled_preset_models())
        .context("failed to serialize default enabled preset models")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO proxy_model_settings (
            id,
            hijack_enabled,
            merge_upstream_enabled,
            fast_mode_rewrite_mode,
            enabled_preset_models_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(DEFAULT_PROXY_MODELS_HIJACK_ENABLED as i64)
    .bind(DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED as i64)
    .bind(DEFAULT_PROXY_FAST_MODE_REWRITE_MODE.as_str())
    .bind(default_enabled_models_json)
    .execute(pool)
    .await
    .context("failed to ensure default proxy_model_settings row")?;

    ensure_proxy_enabled_models_contains_new_presets(pool)
        .await
        .context("failed to ensure proxy preset models list is up-to-date")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pricing_settings_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            catalog_version TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pricing_settings_meta table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pricing_settings_models (
            model TEXT PRIMARY KEY,
            input_per_1m REAL NOT NULL,
            output_per_1m REAL NOT NULL,
            cache_input_per_1m REAL,
            reasoning_per_1m REAL,
            source TEXT NOT NULL DEFAULT 'custom',
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pricing_settings_models table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            proxy_urls_json TEXT NOT NULL DEFAULT '[]',
            subscription_urls_json TEXT NOT NULL DEFAULT '[]',
            subscription_update_interval_secs INTEGER NOT NULL DEFAULT 3600,
            insert_direct INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_settings table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_runtime (
            proxy_key TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            source TEXT NOT NULL,
            endpoint_url TEXT,
            weight REAL NOT NULL,
            success_ema REAL NOT NULL,
            latency_ema_ms REAL,
            consecutive_failures INTEGER NOT NULL DEFAULT 0,
            is_penalized INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_runtime table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_attempts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            proxy_key TEXT NOT NULL,
            occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
            is_success INTEGER NOT NULL,
            latency_ms REAL,
            failure_kind TEXT,
            is_probe INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_attempts table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_weight_hourly (
            proxy_key TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            sample_count INTEGER NOT NULL,
            min_weight REAL NOT NULL,
            max_weight REAL NOT NULL,
            avg_weight REAL NOT NULL,
            last_weight REAL NOT NULL,
            last_sample_epoch_us INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (proxy_key, bucket_start_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_weight_hourly table existence")?;

    let existing_forward_proxy_weight_columns: HashSet<String> =
        sqlx::query("PRAGMA table_info('forward_proxy_weight_hourly')")
            .fetch_all(pool)
            .await
            .context("failed to inspect forward_proxy_weight_hourly schema")?
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect();
    if !existing_forward_proxy_weight_columns.contains("last_sample_epoch_us") {
        sqlx::query(
            r#"
            ALTER TABLE forward_proxy_weight_hourly
            ADD COLUMN last_sample_epoch_us INTEGER NOT NULL DEFAULT 0
            "#,
        )
        .execute(pool)
        .await
        .context("failed to add last_sample_epoch_us to forward_proxy_weight_hourly")?;
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempts_proxy_time
        ON forward_proxy_attempts (proxy_key, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempts_proxy_time")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempts_time_proxy
        ON forward_proxy_attempts (occurred_at, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempts_time_proxy")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_weight_hourly_time_proxy
        ON forward_proxy_weight_hourly (bucket_start_epoch, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_weight_hourly_time_proxy")?;

    let default_proxy_urls_json =
        serde_json::to_string(&Vec::<String>::new()).context("serialize default proxy urls")?;
    let default_subscription_urls_json = serde_json::to_string(&Vec::<String>::new())
        .context("serialize default proxy subscription urls")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO forward_proxy_settings (
            id,
            proxy_urls_json,
            subscription_urls_json,
            subscription_update_interval_secs,
            insert_direct
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(FORWARD_PROXY_SETTINGS_SINGLETON_ID)
    .bind(default_proxy_urls_json)
    .bind(default_subscription_urls_json)
    .bind(DEFAULT_FORWARD_PROXY_SUBSCRIPTION_INTERVAL_SECS as i64)
    .bind(DEFAULT_FORWARD_PROXY_INSERT_DIRECT as i64)
    .execute(pool)
    .await
    .context("failed to ensure default forward_proxy_settings row")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS startup_backfill_progress (
            task_name TEXT PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            next_run_after TEXT,
            zero_update_streak INTEGER NOT NULL DEFAULT 0,
            last_started_at TEXT,
            last_finished_at TEXT,
            last_scanned INTEGER NOT NULL DEFAULT 0,
            last_updated INTEGER NOT NULL DEFAULT 0,
            last_status TEXT NOT NULL DEFAULT 'idle'
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure startup_backfill_progress table existence")?;

    seed_default_pricing_catalog(pool).await?;

    Ok(())
}

async fn load_proxy_model_settings(pool: &Pool<Sqlite>) -> Result<ProxyModelSettings> {
    let row = sqlx::query_as::<_, ProxyModelSettingsRow>(
        r#"
        SELECT hijack_enabled, merge_upstream_enabled, fast_mode_rewrite_mode, enabled_preset_models_json
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to load proxy_model_settings row")?;

    Ok(row
        .map(Into::into)
        .unwrap_or_else(ProxyModelSettings::default))
}

async fn save_proxy_model_settings(
    pool: &Pool<Sqlite>,
    settings: ProxyModelSettings,
) -> Result<()> {
    let settings = settings.normalized();
    let enabled_models_json = serde_json::to_string(&settings.enabled_preset_models)
        .context("failed to serialize enabled preset models")?;
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET hijack_enabled = ?1,
            merge_upstream_enabled = ?2,
            fast_mode_rewrite_mode = ?3,
            enabled_preset_models_json = ?4,
            updated_at = datetime('now')
        WHERE id = ?5
        "#,
    )
    .bind(settings.hijack_enabled as i64)
    .bind(settings.merge_upstream_enabled as i64)
    .bind(settings.fast_mode_rewrite_mode.as_str())
    .bind(enabled_models_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to persist proxy_model_settings row")?;

    Ok(())
}

async fn ensure_proxy_enabled_models_contains_new_presets(pool: &Pool<Sqlite>) -> Result<()> {
    let migrated = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT preset_models_migrated
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to check proxy preset models migration flag")?
    .unwrap_or(0);
    if migrated != 0 {
        return Ok(());
    }

    let mut settings = load_proxy_model_settings(pool).await?;

    if settings.enabled_preset_models.is_empty() {
        mark_proxy_preset_models_migrated(pool).await?;
        return Ok(());
    }

    let legacy_default = LEGACY_PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    if settings.enabled_preset_models != legacy_default {
        // Respect user customizations: only auto-append when the enabled list matches
        // the legacy default preset list exactly.
        mark_proxy_preset_models_migrated(pool).await?;
        return Ok(());
    }

    let mut changed = false;
    for required in ["gpt-5.4", "gpt-5.4-pro"] {
        if !settings
            .enabled_preset_models
            .iter()
            .any(|id| id == required)
        {
            settings.enabled_preset_models.push(required.to_string());
            changed = true;
        }
    }

    if !changed {
        mark_proxy_preset_models_migrated(pool).await?;
        return Ok(());
    }

    save_proxy_model_settings(pool, settings).await?;
    mark_proxy_preset_models_migrated(pool).await
}

async fn mark_proxy_preset_models_migrated(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET preset_models_migrated = 1,
            updated_at = datetime('now')
        WHERE id = ?1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to mark proxy preset models as migrated")?;
    Ok(())
}

#[derive(Debug, FromRow)]
struct ForwardProxyAttemptStatsRow {
    proxy_key: String,
    attempts: i64,
    success_count: i64,
    avg_latency_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
struct ForwardProxyHourlyStatsRow {
    proxy_key: String,
    bucket_start_epoch: i64,
    success_count: i64,
    failure_count: i64,
}

#[derive(Debug, FromRow)]
struct ForwardProxyWeightHourlyStatsRow {
    proxy_key: String,
    bucket_start_epoch: i64,
    sample_count: i64,
    min_weight: f64,
    max_weight: f64,
    avg_weight: f64,
    last_weight: f64,
}

#[derive(Debug, FromRow)]
struct ForwardProxyWeightLastBeforeRangeRow {
    proxy_key: String,
    last_weight: f64,
}

async fn load_forward_proxy_settings(pool: &Pool<Sqlite>) -> Result<ForwardProxySettings> {
    let row = sqlx::query_as::<_, ForwardProxySettingsRow>(
        r#"
        SELECT
            proxy_urls_json,
            subscription_urls_json,
            subscription_update_interval_secs,
            insert_direct
        FROM forward_proxy_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(FORWARD_PROXY_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to load forward_proxy_settings row")?;

    Ok(row
        .map(Into::into)
        .unwrap_or_else(ForwardProxySettings::default))
}

async fn save_forward_proxy_settings(
    pool: &Pool<Sqlite>,
    settings: ForwardProxySettings,
) -> Result<()> {
    let normalized = settings.normalized();
    let proxy_urls_json = serde_json::to_string(&normalized.proxy_urls)
        .context("failed to serialize forward proxy urls")?;
    let subscription_urls_json = serde_json::to_string(&normalized.subscription_urls)
        .context("failed to serialize forward proxy subscription urls")?;

    sqlx::query(
        r#"
        UPDATE forward_proxy_settings
        SET
            proxy_urls_json = ?1,
            subscription_urls_json = ?2,
            subscription_update_interval_secs = ?3,
            insert_direct = ?4,
            updated_at = datetime('now')
        WHERE id = ?5
        "#,
    )
    .bind(proxy_urls_json)
    .bind(subscription_urls_json)
    .bind(normalized.subscription_update_interval_secs as i64)
    .bind(normalized.insert_direct as i64)
    .bind(FORWARD_PROXY_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to persist forward_proxy_settings row")?;

    Ok(())
}

async fn load_forward_proxy_runtime_states(
    pool: &Pool<Sqlite>,
) -> Result<Vec<ForwardProxyRuntimeState>> {
    let rows = sqlx::query_as::<_, ForwardProxyRuntimeRow>(
        r#"
        SELECT
            proxy_key,
            display_name,
            source,
            endpoint_url,
            weight,
            success_ema,
            latency_ema_ms,
            consecutive_failures
        FROM forward_proxy_runtime
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to load forward_proxy_runtime rows")?;
    Ok(rows.into_iter().map(Into::into).collect())
}

async fn persist_forward_proxy_runtime_state(
    pool: &Pool<Sqlite>,
    state: &ForwardProxyRuntimeState,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_runtime (
            proxy_key,
            display_name,
            source,
            endpoint_url,
            weight,
            success_ema,
            latency_ema_ms,
            consecutive_failures,
            is_penalized,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
        ON CONFLICT(proxy_key) DO UPDATE SET
            display_name = excluded.display_name,
            source = excluded.source,
            endpoint_url = excluded.endpoint_url,
            weight = excluded.weight,
            success_ema = excluded.success_ema,
            latency_ema_ms = excluded.latency_ema_ms,
            consecutive_failures = excluded.consecutive_failures,
            is_penalized = excluded.is_penalized,
            updated_at = datetime('now')
        "#,
    )
    .bind(&state.proxy_key)
    .bind(&state.display_name)
    .bind(&state.source)
    .bind(&state.endpoint_url)
    .bind(state.weight)
    .bind(state.success_ema)
    .bind(state.latency_ema_ms)
    .bind(i64::from(state.consecutive_failures))
    .bind(state.is_penalized() as i64)
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "failed to persist forward_proxy_runtime row {}",
            state.proxy_key
        )
    })?;
    Ok(())
}

async fn delete_forward_proxy_runtime_rows_not_in(
    pool: &Pool<Sqlite>,
    active_keys: &[String],
) -> Result<()> {
    if active_keys.is_empty() {
        sqlx::query("DELETE FROM forward_proxy_runtime")
            .execute(pool)
            .await
            .context("failed to clear forward_proxy_runtime rows")?;
        return Ok(());
    }
    let mut builder =
        QueryBuilder::<Sqlite>::new("DELETE FROM forward_proxy_runtime WHERE proxy_key NOT IN (");
    {
        let mut separated = builder.separated(", ");
        for key in active_keys {
            separated.push_bind(key);
        }
    }
    builder.push(")");
    builder
        .build()
        .execute(pool)
        .await
        .context("failed to prune forward_proxy_runtime rows")?;
    Ok(())
}

async fn insert_forward_proxy_attempt(
    pool: &Pool<Sqlite>,
    proxy_key: &str,
    success: bool,
    latency_ms: Option<f64>,
    failure_kind: Option<&str>,
    is_probe: bool,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_attempts (
            proxy_key,
            is_success,
            latency_ms,
            failure_kind,
            is_probe
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(proxy_key)
    .bind(success as i64)
    .bind(latency_ms)
    .bind(failure_kind)
    .bind(is_probe as i64)
    .execute(pool)
    .await
    .with_context(|| format!("failed to insert forward proxy attempt for {proxy_key}"))?;
    Ok(())
}

async fn upsert_forward_proxy_weight_hourly_bucket(
    pool: &Pool<Sqlite>,
    proxy_key: &str,
    bucket_start_epoch: i64,
    weight: f64,
    sample_epoch_us: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_weight_hourly (
            proxy_key,
            bucket_start_epoch,
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight,
            last_sample_epoch_us,
            updated_at
        )
        VALUES (?1, ?2, 1, ?3, ?3, ?3, ?3, ?4, datetime('now'))
        ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
            sample_count = forward_proxy_weight_hourly.sample_count + 1,
            min_weight = MIN(forward_proxy_weight_hourly.min_weight, excluded.min_weight),
            max_weight = MAX(forward_proxy_weight_hourly.max_weight, excluded.max_weight),
            avg_weight = (
                (forward_proxy_weight_hourly.avg_weight * forward_proxy_weight_hourly.sample_count)
                + excluded.avg_weight
            ) / (forward_proxy_weight_hourly.sample_count + 1),
            last_weight = CASE
                WHEN excluded.last_sample_epoch_us >= forward_proxy_weight_hourly.last_sample_epoch_us
                    THEN excluded.last_weight
                ELSE forward_proxy_weight_hourly.last_weight
            END,
            last_sample_epoch_us = MAX(
                forward_proxy_weight_hourly.last_sample_epoch_us,
                excluded.last_sample_epoch_us
            ),
            updated_at = datetime('now')
        "#,
    )
    .bind(proxy_key)
    .bind(bucket_start_epoch)
    .bind(weight)
    .bind(sample_epoch_us)
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "failed to upsert forward proxy weight bucket for {proxy_key} at {bucket_start_epoch}"
        )
    })?;
    Ok(())
}

async fn query_forward_proxy_window_stats(
    pool: &Pool<Sqlite>,
    window: &str,
) -> Result<HashMap<String, ForwardProxyAttemptWindowStats>> {
    let rows = sqlx::query_as::<_, ForwardProxyAttemptStatsRow>(
        r#"
        SELECT
            proxy_key,
            COUNT(*) AS attempts,
            SUM(CASE WHEN is_success != 0 THEN 1 ELSE 0 END) AS success_count,
            AVG(CASE WHEN is_success != 0 THEN latency_ms END) AS avg_latency_ms
        FROM forward_proxy_attempts
        WHERE occurred_at >= datetime('now', ?1)
        GROUP BY proxy_key
        "#,
    )
    .bind(window)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to query forward proxy attempt stats for {window}"))?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.proxy_key,
                ForwardProxyAttemptWindowStats {
                    attempts: row.attempts,
                    success_count: row.success_count,
                    avg_latency_ms: row.avg_latency_ms,
                },
            )
        })
        .collect())
}

async fn query_forward_proxy_hourly_stats(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<HashMap<String, HashMap<i64, ForwardProxyHourlyStatsPoint>>> {
    let rows = sqlx::query_as::<_, ForwardProxyHourlyStatsRow>(
        r#"
        SELECT
            proxy_key,
            (CAST(strftime('%s', occurred_at) AS INTEGER) / 3600) * 3600 AS bucket_start_epoch,
            SUM(CASE WHEN is_success != 0 THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN is_success = 0 THEN 1 ELSE 0 END) AS failure_count
        FROM forward_proxy_attempts
        WHERE occurred_at >= datetime(?1, 'unixepoch')
          AND occurred_at < datetime(?2, 'unixepoch')
        GROUP BY proxy_key, bucket_start_epoch
        "#,
    )
    .bind(range_start_epoch)
    .bind(range_end_epoch)
    .fetch_all(pool)
    .await
    .with_context(|| {
        format!(
            "failed to query forward proxy hourly stats within [{range_start_epoch}, {range_end_epoch})"
        )
    })?;

    let mut grouped: HashMap<String, HashMap<i64, ForwardProxyHourlyStatsPoint>> = HashMap::new();
    for row in rows {
        grouped.entry(row.proxy_key).or_default().insert(
            row.bucket_start_epoch,
            ForwardProxyHourlyStatsPoint {
                success_count: row.success_count,
                failure_count: row.failure_count,
            },
        );
    }

    Ok(grouped)
}

async fn query_forward_proxy_weight_hourly_stats(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<HashMap<String, HashMap<i64, ForwardProxyWeightHourlyStatsPoint>>> {
    let rows = sqlx::query_as::<_, ForwardProxyWeightHourlyStatsRow>(
        r#"
        SELECT
            proxy_key,
            bucket_start_epoch,
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight
        FROM forward_proxy_weight_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
        "#,
    )
    .bind(range_start_epoch)
    .bind(range_end_epoch)
    .fetch_all(pool)
    .await
    .with_context(|| {
        format!(
            "failed to query forward proxy weight stats within [{range_start_epoch}, {range_end_epoch})"
        )
    })?;

    let mut grouped: HashMap<String, HashMap<i64, ForwardProxyWeightHourlyStatsPoint>> =
        HashMap::new();
    for row in rows {
        grouped.entry(row.proxy_key).or_default().insert(
            row.bucket_start_epoch,
            ForwardProxyWeightHourlyStatsPoint {
                sample_count: row.sample_count,
                min_weight: row.min_weight,
                max_weight: row.max_weight,
                avg_weight: row.avg_weight,
                last_weight: row.last_weight,
            },
        );
    }

    Ok(grouped)
}

async fn query_forward_proxy_weight_last_before(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    proxy_keys: &[String],
) -> Result<HashMap<String, f64>> {
    if proxy_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT latest.proxy_key, latest.last_weight
        FROM forward_proxy_weight_hourly AS latest
        INNER JOIN (
            SELECT proxy_key, MAX(bucket_start_epoch) AS bucket_start_epoch
            FROM forward_proxy_weight_hourly
            WHERE bucket_start_epoch < "#,
    );
    builder.push_bind(range_start_epoch);
    builder.push(" AND proxy_key IN (");
    {
        let mut separated = builder.separated(", ");
        for key in proxy_keys {
            separated.push_bind(key);
        }
    }
    builder.push(
        r#")
            GROUP BY proxy_key
        ) AS prior
            ON latest.proxy_key = prior.proxy_key
           AND latest.bucket_start_epoch = prior.bucket_start_epoch
        "#,
    );

    let rows = builder
        .build_query_as::<ForwardProxyWeightLastBeforeRangeRow>()
        .fetch_all(pool)
        .await
        .with_context(|| {
            format!("failed to query forward proxy weight carry values before {range_start_epoch}")
        })?;

    Ok(rows
        .into_iter()
        .map(|row| (row.proxy_key, row.last_weight))
        .collect())
}

async fn build_forward_proxy_settings_response(
    state: &AppState,
) -> Result<ForwardProxySettingsResponse> {
    let (settings, runtime_rows) = {
        let manager = state.forward_proxy.lock().await;
        (manager.settings.clone(), manager.snapshot_runtime())
    };

    let windows = [
        ("-1 minute", 0usize),
        ("-15 minutes", 1usize),
        ("-1 hour", 2usize),
        ("-1 day", 3usize),
        ("-7 days", 4usize),
    ];
    let mut window_maps: Vec<HashMap<String, ForwardProxyAttemptWindowStats>> = Vec::new();
    for (window, _) in &windows {
        window_maps.push(query_forward_proxy_window_stats(&state.pool, window).await?);
    }

    let mut nodes = runtime_rows
        .into_iter()
        .map(|runtime| {
            let stats_for = |index: usize| {
                window_maps[index]
                    .get(&runtime.proxy_key)
                    .cloned()
                    .map(ForwardProxyWindowStatsResponse::from)
                    .unwrap_or_default()
            };
            ForwardProxyNodeResponse {
                key: runtime.proxy_key.clone(),
                source: runtime.source.clone(),
                display_name: runtime.display_name.clone(),
                endpoint_url: runtime.endpoint_url.clone(),
                weight: runtime.weight,
                penalized: runtime.is_penalized(),
                stats: ForwardProxyStatsResponse {
                    one_minute: stats_for(0),
                    fifteen_minutes: stats_for(1),
                    one_hour: stats_for(2),
                    one_day: stats_for(3),
                    seven_days: stats_for(4),
                },
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));

    Ok(ForwardProxySettingsResponse {
        proxy_urls: settings.proxy_urls,
        subscription_urls: settings.subscription_urls,
        subscription_update_interval_secs: settings.subscription_update_interval_secs,
        insert_direct: settings.insert_direct,
        nodes,
    })
}

async fn build_forward_proxy_live_stats_response(
    state: &AppState,
) -> Result<ForwardProxyLiveStatsResponse> {
    const BUCKET_SECONDS: i64 = 3600;
    const BUCKET_COUNT: i64 = 24;

    let runtime_rows = {
        let manager = state.forward_proxy.lock().await;
        manager.snapshot_runtime()
    };
    let runtime_proxy_keys = runtime_rows
        .iter()
        .map(|runtime| runtime.proxy_key.clone())
        .collect::<Vec<_>>();

    let windows = [
        ("-1 minute", 0usize),
        ("-15 minutes", 1usize),
        ("-1 hour", 2usize),
        ("-1 day", 3usize),
        ("-7 days", 4usize),
    ];
    let mut window_maps: Vec<HashMap<String, ForwardProxyAttemptWindowStats>> = Vec::new();
    for (window, _) in &windows {
        window_maps.push(query_forward_proxy_window_stats(&state.pool, window).await?);
    }

    let now_epoch = Utc::now().timestamp();
    let range_end_epoch = align_bucket_epoch(now_epoch, BUCKET_SECONDS, 0) + BUCKET_SECONDS;
    let range_start_epoch = range_end_epoch - BUCKET_COUNT * BUCKET_SECONDS;
    let hourly_map =
        query_forward_proxy_hourly_stats(&state.pool, range_start_epoch, range_end_epoch).await?;
    let weight_hourly_map =
        query_forward_proxy_weight_hourly_stats(&state.pool, range_start_epoch, range_end_epoch)
            .await?;
    let weight_carry_map =
        query_forward_proxy_weight_last_before(&state.pool, range_start_epoch, &runtime_proxy_keys)
            .await?;

    let mut nodes = runtime_rows
        .into_iter()
        .map(|runtime| {
            let proxy_key = runtime.proxy_key.clone();
            let penalized = runtime.is_penalized();
            let runtime_weight = runtime.weight;
            let stats_for = |index: usize, key: &str| {
                window_maps[index]
                    .get(key)
                    .cloned()
                    .map(ForwardProxyWindowStatsResponse::from)
                    .unwrap_or_default()
            };
            let hourly = hourly_map.get(&proxy_key);
            let weight_hourly = weight_hourly_map.get(&proxy_key);
            let mut carry_weight = weight_carry_map
                .get(&proxy_key)
                .copied()
                .unwrap_or(runtime_weight);
            let one_minute = stats_for(0, &proxy_key);
            let fifteen_minutes = stats_for(1, &proxy_key);
            let one_hour = stats_for(2, &proxy_key);
            let one_day = stats_for(3, &proxy_key);
            let seven_days = stats_for(4, &proxy_key);
            let last24h = (0..BUCKET_COUNT)
                .map(|index| {
                    let bucket_start_epoch = range_start_epoch + index * BUCKET_SECONDS;
                    let bucket_end_epoch = bucket_start_epoch + BUCKET_SECONDS;
                    let point = hourly
                        .and_then(|items| items.get(&bucket_start_epoch))
                        .cloned()
                        .unwrap_or_default();
                    let bucket_start = Utc
                        .timestamp_opt(bucket_start_epoch, 0)
                        .single()
                        .ok_or_else(|| anyhow!("invalid forward proxy bucket start epoch"))?;
                    let bucket_end = Utc
                        .timestamp_opt(bucket_end_epoch, 0)
                        .single()
                        .ok_or_else(|| anyhow!("invalid forward proxy bucket end epoch"))?;
                    Ok(ForwardProxyHourlyBucketResponse {
                        bucket_start: format_utc_iso(bucket_start),
                        bucket_end: format_utc_iso(bucket_end),
                        success_count: point.success_count,
                        failure_count: point.failure_count,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let weight24h = (0..BUCKET_COUNT)
                .map(|index| {
                    let bucket_start_epoch = range_start_epoch + index * BUCKET_SECONDS;
                    let bucket_end_epoch = bucket_start_epoch + BUCKET_SECONDS;
                    let point = weight_hourly.and_then(|items| items.get(&bucket_start_epoch));
                    let (sample_count, min_weight, max_weight, avg_weight, last_weight) =
                        if let Some(point) = point {
                            carry_weight = point.last_weight;
                            (
                                point.sample_count,
                                point.min_weight,
                                point.max_weight,
                                point.avg_weight,
                                point.last_weight,
                            )
                        } else {
                            (0, carry_weight, carry_weight, carry_weight, carry_weight)
                        };
                    let bucket_start = Utc
                        .timestamp_opt(bucket_start_epoch, 0)
                        .single()
                        .ok_or_else(|| {
                            anyhow!("invalid forward proxy weight bucket start epoch")
                        })?;
                    let bucket_end = Utc
                        .timestamp_opt(bucket_end_epoch, 0)
                        .single()
                        .ok_or_else(|| anyhow!("invalid forward proxy weight bucket end epoch"))?;
                    Ok(ForwardProxyWeightHourlyBucketResponse {
                        bucket_start: format_utc_iso(bucket_start),
                        bucket_end: format_utc_iso(bucket_end),
                        sample_count,
                        min_weight,
                        max_weight,
                        avg_weight,
                        last_weight,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(ForwardProxyLiveNodeResponse {
                key: proxy_key,
                source: runtime.source,
                display_name: runtime.display_name,
                endpoint_url: runtime.endpoint_url,
                weight: runtime_weight,
                penalized,
                stats: ForwardProxyStatsResponse {
                    one_minute,
                    fifteen_minutes,
                    one_hour,
                    one_day,
                    seven_days,
                },
                last24h,
                weight24h,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    nodes.sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));

    let range_start = Utc
        .timestamp_opt(range_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid forward proxy range start epoch"))?;
    let range_end = Utc
        .timestamp_opt(range_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid forward proxy range end epoch"))?;

    Ok(ForwardProxyLiveStatsResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        bucket_seconds: BUCKET_SECONDS,
        nodes,
    })
}

#[derive(Debug, FromRow)]
struct PricingSettingsMetaRow {
    catalog_version: String,
}

#[derive(Debug, FromRow)]
struct PricingSettingsModelRow {
    model: String,
    input_per_1m: f64,
    output_per_1m: f64,
    cache_input_per_1m: Option<f64>,
    reasoning_per_1m: Option<f64>,
    source: String,
}

async fn ensure_pricing_model_present(
    pool: &Pool<Sqlite>,
    model: &str,
    pricing: ModelPricing,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO pricing_settings_models (
            model,
            input_per_1m,
            output_per_1m,
            cache_input_per_1m,
            reasoning_per_1m,
            source
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(model)
    .bind(pricing.input_per_1m)
    .bind(pricing.output_per_1m)
    .bind(pricing.cache_input_per_1m)
    .bind(pricing.reasoning_per_1m)
    .bind(pricing.source)
    .execute(pool)
    .await
    .with_context(|| format!("failed to ensure pricing model exists: {model}"))?;

    Ok(())
}

async fn ensure_pricing_models_present(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_pricing_model_present(
        pool,
        "gpt-5.4",
        ModelPricing {
            input_per_1m: 2.5,
            output_per_1m: 15.0,
            cache_input_per_1m: Some(0.25),
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    ensure_pricing_model_present(
        pool,
        "gpt-5.4-pro",
        ModelPricing {
            input_per_1m: 30.0,
            output_per_1m: 180.0,
            cache_input_per_1m: None,
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    Ok(())
}

async fn normalize_default_pricing_sources(pool: &Pool<Sqlite>) -> Result<()> {
    // Legacy versions used `temporary` for some built-in models; keep the pricing untouched
    // but normalize the metadata so UI and reporting remain consistent.
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET source = 'official'
        WHERE model = 'gpt-5.3-codex'
          AND lower(trim(source)) = 'temporary'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to normalize default pricing sources")?;
    Ok(())
}

async fn seed_default_pricing_catalog(pool: &Pool<Sqlite>) -> Result<()> {
    let legacy_path = resolve_legacy_pricing_catalog_path();
    seed_default_pricing_catalog_with_legacy_path(pool, Some(&legacy_path)).await
}

async fn seed_default_pricing_catalog_with_legacy_path(
    pool: &Pool<Sqlite>,
    legacy_path: Option<&Path>,
) -> Result<()> {
    let meta_exists = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pricing_settings_meta
        WHERE id = ?1
        "#,
    )
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .fetch_one(pool)
    .await
    .context("failed to count pricing_settings_meta rows")?;
    if meta_exists > 0 {
        let version = sqlx::query_scalar::<_, String>(
            r#"
            SELECT catalog_version
            FROM pricing_settings_meta
            WHERE id = ?1
            LIMIT 1
            "#,
        )
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .fetch_one(pool)
        .await
        .context("failed to load pricing_settings_meta row")?;
        if version == DEFAULT_PRICING_CATALOG_VERSION
            || version == LEGACY_DEFAULT_PRICING_CATALOG_VERSION
        {
            ensure_pricing_models_present(pool).await?;
            normalize_default_pricing_sources(pool).await?;
        }
        return Ok(());
    }

    let existing_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pricing_settings_models
        "#,
    )
    .fetch_one(pool)
    .await
    .context("failed to count pricing_settings_models rows")?;

    if existing_count > 0 {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO pricing_settings_meta (id, catalog_version)
            VALUES (?1, ?2)
            "#,
        )
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .bind(DEFAULT_PRICING_CATALOG_VERSION)
        .execute(pool)
        .await
        .context("failed to ensure default pricing_settings_meta row")?;
        ensure_pricing_models_present(pool).await?;
        normalize_default_pricing_sources(pool).await?;
        return Ok(());
    }

    if let Some(path) = legacy_path {
        match load_legacy_pricing_catalog(path) {
            Ok(Some(catalog)) => {
                info!(
                    path = %path.display(),
                    version = %catalog.version,
                    model_count = catalog.models.len(),
                    "migrating legacy pricing catalog into sqlite"
                );
                save_pricing_catalog(pool, &catalog).await?;
                if catalog.version == DEFAULT_PRICING_CATALOG_VERSION
                    || catalog.version == LEGACY_DEFAULT_PRICING_CATALOG_VERSION
                {
                    ensure_pricing_models_present(pool).await?;
                    normalize_default_pricing_sources(pool).await?;
                }
                return Ok(());
            }
            Ok(None) => {}
            Err(err) => {
                warn!(
                    path = %path.display(),
                    ?err,
                    "failed to migrate legacy pricing catalog; falling back to defaults"
                );
            }
        }
    }

    save_pricing_catalog(pool, &default_pricing_catalog()).await?;
    ensure_pricing_models_present(pool).await?;
    normalize_default_pricing_sources(pool).await?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyPricingCatalogFile {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    models: HashMap<String, LegacyModelPricing>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyModelPricing {
    input_per_1m: f64,
    output_per_1m: f64,
    #[serde(default)]
    cache_input_per_1m: Option<f64>,
    #[serde(default)]
    reasoning_per_1m: Option<f64>,
    #[serde(default)]
    source: Option<String>,
}

fn resolve_legacy_pricing_catalog_path() -> PathBuf {
    env::var("PROXY_PRICING_CATALOG_PATH")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_PROXY_PRICING_CATALOG_PATH))
}

fn load_legacy_pricing_catalog(path: &Path) -> Result<Option<PricingCatalog>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read legacy pricing catalog: {}", path.display()))?;
    let parsed: LegacyPricingCatalogFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse legacy pricing catalog: {}", path.display()))?;
    if parsed.models.is_empty() {
        return Ok(None);
    }

    let version = parsed
        .version
        .and_then(normalize_pricing_catalog_version)
        .unwrap_or_else(|| DEFAULT_PRICING_CATALOG_VERSION.to_string());
    let models = parsed
        .models
        .into_iter()
        .map(|(model, pricing)| {
            (
                model,
                ModelPricing {
                    input_per_1m: pricing.input_per_1m,
                    output_per_1m: pricing.output_per_1m,
                    cache_input_per_1m: pricing.cache_input_per_1m,
                    reasoning_per_1m: pricing.reasoning_per_1m,
                    source: pricing
                        .source
                        .map(normalize_pricing_source)
                        .unwrap_or_else(default_pricing_source_custom),
                },
            )
        })
        .collect::<HashMap<_, _>>();

    Ok(Some(PricingCatalog { version, models }))
}

async fn load_pricing_catalog(pool: &Pool<Sqlite>) -> Result<PricingCatalog> {
    seed_default_pricing_catalog(pool).await?;

    let meta = sqlx::query_as::<_, PricingSettingsMetaRow>(
        r#"
        SELECT catalog_version
        FROM pricing_settings_meta
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to load pricing_settings_meta row")?;
    let version = meta
        .map(|row| row.catalog_version)
        .unwrap_or_else(|| DEFAULT_PRICING_CATALOG_VERSION.to_string());

    let rows = sqlx::query_as::<_, PricingSettingsModelRow>(
        r#"
        SELECT model, input_per_1m, output_per_1m, cache_input_per_1m, reasoning_per_1m, source
        FROM pricing_settings_models
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to load pricing_settings_models rows")?;

    let mut models = HashMap::new();
    for row in rows {
        models.insert(
            row.model,
            ModelPricing {
                input_per_1m: row.input_per_1m,
                output_per_1m: row.output_per_1m,
                cache_input_per_1m: row.cache_input_per_1m,
                reasoning_per_1m: row.reasoning_per_1m,
                source: normalize_pricing_source(row.source),
            },
        );
    }

    Ok(PricingCatalog { version, models })
}

async fn save_pricing_catalog(pool: &Pool<Sqlite>, catalog: &PricingCatalog) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin pricing transaction")?;
    sqlx::query("DELETE FROM pricing_settings_models")
        .execute(&mut *tx)
        .await
        .context("failed to clear pricing_settings_models rows")?;

    let mut keys = catalog.models.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for model in keys {
        let pricing = catalog
            .models
            .get(&model)
            .with_context(|| format!("missing pricing entry while saving: {model}"))?;
        sqlx::query(
            r#"
            INSERT INTO pricing_settings_models (
                model,
                input_per_1m,
                output_per_1m,
                cache_input_per_1m,
                reasoning_per_1m,
                source,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
            "#,
        )
        .bind(model)
        .bind(pricing.input_per_1m)
        .bind(pricing.output_per_1m)
        .bind(pricing.cache_input_per_1m)
        .bind(pricing.reasoning_per_1m)
        .bind(&pricing.source)
        .execute(&mut *tx)
        .await
        .context("failed to insert pricing_settings_models row")?;
    }

    sqlx::query(
        r#"
        INSERT INTO pricing_settings_meta (id, catalog_version, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(id) DO UPDATE SET
            catalog_version = excluded.catalog_version,
            updated_at = datetime('now')
        "#,
    )
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .bind(&catalog.version)
    .execute(&mut *tx)
    .await
    .context("failed to upsert pricing_settings_meta row")?;

    tx.commit()
        .await
        .context("failed to commit pricing transaction")?;
    Ok(())
}

fn default_pricing_catalog() -> PricingCatalog {
    let models = [
        (
            "gpt-5.3-codex",
            ModelPricing {
                input_per_1m: 1.75,
                output_per_1m: 14.0,
                cache_input_per_1m: Some(0.175),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.2-codex",
            ModelPricing {
                input_per_1m: 1.75,
                output_per_1m: 14.0,
                cache_input_per_1m: Some(0.175),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.1-codex-max",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.1-codex-mini",
            ModelPricing {
                input_per_1m: 0.25,
                output_per_1m: 2.0,
                cache_input_per_1m: Some(0.025),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.2",
            ModelPricing {
                input_per_1m: 1.75,
                output_per_1m: 14.0,
                cache_input_per_1m: Some(0.175),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.4",
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-mini",
            ModelPricing {
                input_per_1m: 0.25,
                output_per_1m: 2.0,
                cache_input_per_1m: Some(0.025),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-nano",
            ModelPricing {
                input_per_1m: 0.05,
                output_per_1m: 0.4,
                cache_input_per_1m: Some(0.005),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.2-chat-latest",
            ModelPricing {
                input_per_1m: 1.75,
                output_per_1m: 14.0,
                cache_input_per_1m: Some(0.175),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.1-chat-latest",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-chat-latest",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.1-codex",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-codex",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.2-pro",
            ModelPricing {
                input_per_1m: 21.0,
                output_per_1m: 168.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.4-pro",
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-pro",
            ModelPricing {
                input_per_1m: 15.0,
                output_per_1m: 120.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
    ]
    .into_iter()
    .map(|(model, pricing)| (model.to_string(), pricing))
    .collect::<HashMap<_, _>>();

    PricingCatalog {
        version: DEFAULT_PRICING_CATALOG_VERSION.to_string(),
        models,
    }
}

async fn list_invocations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let limit = params
        .limit
        .unwrap_or(50)
        .clamp(1, state.config.list_limit_max as i64);

    let mut query = QueryBuilder::new(
        "SELECT id, invoke_id, occurred_at, source, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name, \
         model, input_tokens, output_tokens, \
         cache_input_tokens, reasoning_tokens, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.reasoningEffort') END AS reasoning_effort, \
         total_tokens, cost, status, error_message, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint, \
         COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind, \
         failure_class, is_actionable, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key, \
         CASE \
           WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text' \
             THEN json_extract(payload, '$.requestedServiceTier') \
           WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text' \
             THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier, \
         CASE \
           WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text' \
             THEN json_extract(payload, '$.serviceTier') \
           WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text' \
             THEN json_extract(payload, '$.service_tier') END AS service_tier, \
         CASE WHEN json_valid(payload) \
           AND json_type(payload, '$.proxyWeightDelta') IN ('integer', 'real') \
           THEN json_extract(payload, '$.proxyWeightDelta') END AS proxy_weight_delta, \
         cost_estimated, price_version, \
         request_raw_path, request_raw_size, request_raw_truncated, request_raw_truncated_reason, \
         response_raw_path, response_raw_size, response_raw_truncated, response_raw_truncated_reason, \
         raw_expires_at, detail_level, detail_pruned_at, detail_prune_reason, \
         t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, \
         t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, \
         created_at \
         FROM codex_invocations WHERE 1 = 1",
    );
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    if let Some(model) = params.model.as_ref() {
        query.push(" AND model = ").push_bind(model);
    }

    if let Some(status) = params.status.as_ref() {
        query.push(" AND status = ").push_bind(status);
    }

    query
        .push(" ORDER BY occurred_at DESC LIMIT ")
        .push_bind(limit);

    let records = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&state.pool)
        .await?;

    Ok(Json(ListResponse { records }))
}

async fn fetch_stats(State(state): State<Arc<AppState>>) -> Result<Json<StatsResponse>, ApiError> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let totals = query_combined_totals(
        &state.pool,
        state.config.crs_stats.as_ref(),
        StatsFilter::All,
        source_scope,
    )
    .await?;
    Ok(Json(totals.into_response()))
}

async fn fetch_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummaryQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    let default_limit = state.config.list_limit_max as i64;
    let window = parse_summary_window(&params, default_limit)?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    let totals = match window {
        SummaryWindow::All => {
            query_combined_totals(
                &state.pool,
                state.config.crs_stats.as_ref(),
                StatsFilter::All,
                source_scope,
            )
            .await?
        }
        SummaryWindow::Current(limit) => {
            query_combined_totals(
                &state.pool,
                state.config.crs_stats.as_ref(),
                StatsFilter::RecentLimit(limit),
                source_scope,
            )
            .await?
        }
        SummaryWindow::Duration(duration) => {
            let start = Utc::now() - duration;
            query_combined_totals(
                &state.pool,
                state.config.crs_stats.as_ref(),
                StatsFilter::Since(start),
                source_scope,
            )
            .await?
        }
        SummaryWindow::Calendar(spec) => {
            let now = Utc::now();
            let start = named_range_start(spec.as_str(), now, reporting_tz)
                .ok_or_else(|| ApiError(anyhow!("unsupported calendar window: {spec}")))?;
            query_combined_totals(
                &state.pool,
                state.config.crs_stats.as_ref(),
                StatsFilter::Since(start),
                source_scope,
            )
            .await?
        }
    };

    Ok(Json(totals.into_response()))
}

async fn fetch_forward_proxy_live_stats(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ForwardProxyLiveStatsResponse>, ApiError> {
    let response = build_forward_proxy_live_stats_response(state.as_ref()).await?;
    Ok(Json(response))
}

async fn fetch_prompt_cache_conversations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PromptCacheConversationsQuery>,
) -> Result<Json<PromptCacheConversationsResponse>, ApiError> {
    let limit = normalize_prompt_cache_conversation_limit(params.limit);
    let response = fetch_prompt_cache_conversations_cached(state.as_ref(), limit).await?;
    Ok(Json(response))
}

fn normalize_prompt_cache_conversation_limit(raw: Option<i64>) -> i64 {
    match raw {
        Some(value @ (20 | 50 | 100)) => value,
        _ => PROMPT_CACHE_CONVERSATION_DEFAULT_LIMIT,
    }
}

async fn fetch_prompt_cache_conversations_cached(
    state: &AppState,
    limit: i64,
) -> Result<PromptCacheConversationsResponse> {
    loop {
        let mut wait_on: Option<watch::Receiver<bool>> = None;
        let mut flight_guard: Option<PromptCacheConversationFlightGuard> = None;
        {
            let mut cache = state.prompt_cache_conversation_cache.lock().await;
            if let Some(entry) = cache.entries.get(&limit)
                && entry.cached_at.elapsed()
                    <= Duration::from_secs(PROMPT_CACHE_CONVERSATION_CACHE_TTL_SECS)
            {
                return Ok(entry.response.clone());
            }

            if let Some(in_flight) = cache.in_flight.get(&limit) {
                wait_on = Some(in_flight.signal.subscribe());
            } else {
                let (signal, _receiver) = watch::channel(false);
                cache
                    .in_flight
                    .insert(limit, PromptCacheConversationInFlight { signal });
                flight_guard = Some(PromptCacheConversationFlightGuard::new(
                    state.prompt_cache_conversation_cache.clone(),
                    limit,
                ));
            }
        }

        if let Some(mut receiver) = wait_on {
            if !*receiver.borrow() {
                let _ = receiver.changed().await;
            }
            continue;
        }

        let result = build_prompt_cache_conversations_response(state, limit).await;

        if let Some(guard) = flight_guard.as_mut() {
            guard.disarm();
        }

        let mut cache = state.prompt_cache_conversation_cache.lock().await;
        if let Some(in_flight) = cache.in_flight.remove(&limit) {
            if let Ok(response) = &result {
                cache.entries.insert(
                    limit,
                    PromptCacheConversationsCacheEntry {
                        cached_at: Instant::now(),
                        response: response.clone(),
                    },
                );
            }
            let _ = in_flight.signal.send(true);
        }

        return result;
    }
}

async fn build_prompt_cache_conversations_response(
    state: &AppState,
    limit: i64,
) -> Result<PromptCacheConversationsResponse> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_end = Utc::now();
    let range_start = range_end - ChronoDuration::hours(24);
    let range_start_bound = db_occurred_at_lower_bound(range_start);

    let aggregates = query_prompt_cache_conversation_aggregates(
        &state.pool,
        &range_start_bound,
        source_scope,
        limit,
    )
    .await?;
    if aggregates.is_empty() {
        return Ok(PromptCacheConversationsResponse {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            conversations: Vec::new(),
        });
    }

    let selected_keys = aggregates
        .iter()
        .map(|row| row.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let events = query_prompt_cache_conversation_events(
        &state.pool,
        &range_start_bound,
        source_scope,
        &selected_keys,
    )
    .await?;

    let mut grouped_events: HashMap<String, Vec<PromptCacheConversationRequestPointResponse>> =
        HashMap::new();
    for row in events {
        let status = row.status.trim().to_string();
        let status = if status.is_empty() {
            "unknown".to_string()
        } else {
            status
        };
        let is_success = status.eq_ignore_ascii_case("success");
        let request_tokens = row.request_tokens.max(0);
        let points = grouped_events.entry(row.prompt_cache_key).or_default();
        let cumulative_tokens = points
            .last()
            .map(|point| point.cumulative_tokens)
            .unwrap_or(0)
            + request_tokens;
        points.push(PromptCacheConversationRequestPointResponse {
            occurred_at: row.occurred_at,
            status,
            is_success,
            request_tokens,
            cumulative_tokens,
        });
    }

    let conversations = aggregates
        .into_iter()
        .map(|row| PromptCacheConversationResponse {
            prompt_cache_key: row.prompt_cache_key.clone(),
            request_count: row.request_count,
            total_tokens: row.total_tokens,
            total_cost: row.total_cost,
            created_at: row.created_at,
            last_activity_at: row.last_activity_at,
            last24h_requests: grouped_events
                .remove(&row.prompt_cache_key)
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    Ok(PromptCacheConversationsResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        conversations,
    })
}

async fn query_prompt_cache_conversation_aggregates(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    limit: i64,
) -> Result<Vec<PromptCacheConversationAggregateRow>> {
    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH active AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, MIN(occurred_at) AS first_seen_24h \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IS NOT NULL AND ")
        .push(KEY_EXPR)
        .push(
            " <> '' \
             GROUP BY prompt_cache_key\
         ), aggregates AS (\
            SELECT ",
        )
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key, \
                 COUNT(*) AS request_count, \
                 COALESCE(SUM(total_tokens), 0) AS total_tokens, \
                 COALESCE(SUM(cost), 0.0) AS total_cost, \
                 MIN(occurred_at) AS created_at, \
                 MAX(occurred_at) AS last_activity_at \
             FROM codex_invocations \
             WHERE ",
        )
        .push(KEY_EXPR)
        .push(" IN (SELECT prompt_cache_key FROM active)");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(
            " GROUP BY prompt_cache_key\
         ) \
         SELECT prompt_cache_key, request_count, total_tokens, total_cost, created_at, last_activity_at \
         FROM aggregates \
         ORDER BY created_at DESC \
         LIMIT ",
        )
        .push_bind(limit);

    query
        .build_query_as::<PromptCacheConversationAggregateRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn query_prompt_cache_conversation_events(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
) -> Result<Vec<PromptCacheConversationEventRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT occurred_at, COALESCE(status, 'unknown') AS status, \
         COALESCE(total_tokens, 0) AS request_tokens, ",
    );
    query
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ")
        .push(KEY_EXPR)
        .push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(" ORDER BY prompt_cache_key ASC, occurred_at ASC, id ASC");

    query
        .build_query_as::<PromptCacheConversationEventRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

async fn fetch_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let mut bucket_seconds = if let Some(spec) = params.bucket.as_deref() {
        bucket_seconds_from_spec(spec)
            .ok_or_else(|| anyhow!("unsupported bucket specification: {spec}"))?
    } else {
        default_bucket_seconds(range_window.duration)
    };

    if bucket_seconds <= 0 {
        return Err(ApiError(anyhow!("bucket seconds must be positive")));
    }

    let range_seconds = range_window.duration.num_seconds();

    if range_seconds / bucket_seconds > 10_000 {
        // avoid accidentally returning extremely large payloads
        bucket_seconds = range_seconds / 10_000;
    }

    if bucket_seconds == 86_400 {
        return fetch_timeseries_daily(state, params, reporting_tz).await;
    }

    let offset_seconds = 0;

    let end_dt = range_window.end;
    let start_dt = range_window.start;
    let start_str_iso = format_utc_iso(start_dt);

    let mut records_query = QueryBuilder::new(
        "SELECT occurred_at, status, total_tokens, cost, t_upstream_ttfb_ms FROM codex_invocations WHERE occurred_at >= ",
    );
    records_query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        records_query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    records_query.push(" ORDER BY occurred_at ASC");
    let records = records_query
        .build_query_as::<TimeseriesRecord>()
        .fetch_all(&state.pool)
        .await?;

    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();

    let start_epoch = start_dt.timestamp();
    // Track the latest record timestamp only for internal stats, but do not
    // let it extend the visible range beyond "now". Some providers or clock
    // skews can produce future-dated records which previously caused the
    // time-series to expand past the requested window.
    let mut latest_record_epoch = end_dt.timestamp();

    for record in records {
        let naive = NaiveDateTime::parse_from_str(&record.occurred_at, "%Y-%m-%d %H:%M:%S")
            .map_err(|err| anyhow!("failed to parse occurred_at: {err}"))?;
        // Interpret stored naive time as local Asia/Shanghai and convert to UTC epoch
        let epoch = Shanghai
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc).timestamp())
            .unwrap_or_else(|| naive.and_utc().timestamp());
        if epoch > latest_record_epoch {
            latest_record_epoch = epoch;
        }
        let bucket_epoch = align_bucket_epoch(epoch, bucket_seconds, offset_seconds);
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        match record.status.as_deref() {
            Some("success") => entry.success_count += 1,
            _ => entry.failure_count += 1,
        }
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
        entry.total_tokens += record.total_tokens.unwrap_or(0);
        entry.total_cost += record.cost.unwrap_or(0.0);
    }

    let relay_deltas = if source_scope == InvocationSourceScope::All
        && let Some(relay) = state.config.crs_stats.as_ref()
    {
        query_crs_deltas(&state.pool, relay, start_epoch, end_dt.timestamp()).await?
    } else {
        Vec::new()
    };

    for delta in relay_deltas {
        let bucket_epoch =
            align_bucket_epoch(delta.captured_at_epoch, bucket_seconds, offset_seconds);
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += delta.total_count;
        entry.success_count += delta.success_count;
        entry.failure_count += delta.failure_count;
        entry.total_tokens += delta.total_tokens;
        entry.total_cost += delta.total_cost;
    }

    // Compute the inclusive fill range [fill_start_epoch, fill_end_epoch].
    // Start from the aligned bucket that intersects the requested start time.
    let mut bucket_cursor = align_bucket_epoch(start_epoch, bucket_seconds, offset_seconds);
    if bucket_cursor > start_epoch {
        bucket_cursor -= bucket_seconds;
    }
    let fill_start_epoch = bucket_cursor;

    // Clamp the filled range end to the current time (aligned to the next bucket).
    // This prevents future-dated records from pushing the chart beyond the
    // intended window (e.g., "last 24 hours").
    let fill_end_epoch =
        align_bucket_epoch(end_dt.timestamp(), bucket_seconds, offset_seconds) + bucket_seconds;
    while bucket_cursor <= fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor += bucket_seconds;
    }

    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        // Skip any buckets outside the desired window. This guards against
        // future-dated records leaking past the clamped end.
        if bucket_epoch < fill_start_epoch || bucket_epoch + bucket_seconds > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_epoch + bucket_seconds, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let first_byte_avg_ms = agg.first_byte_avg_ms();
        let first_byte_p95_ms = agg.first_byte_p95_ms();
        points.push(TimeseriesPoint {
            bucket_start: format_utc_iso(start),
            bucket_end: format_utc_iso(end),
            total_count: agg.total_count,
            success_count: agg.success_count,
            failure_count: agg.failure_count,
            total_tokens: agg.total_tokens,
            total_cost: agg.total_cost,
            first_byte_sample_count: agg.first_byte_sample_count,
            first_byte_avg_ms,
            first_byte_p95_ms,
        });
    }

    let response = TimeseriesResponse {
        range_start: start_str_iso,
        range_end: {
            let end = Utc
                .timestamp_opt(fill_end_epoch, 0)
                .single()
                .unwrap_or_else(Utc::now);
            format_utc_iso(end)
        },
        bucket_seconds,
        points,
    };

    Ok(Json(response))
}

fn resolve_daily_date_range(
    spec: &str,
    now: DateTime<Utc>,
    tz: Tz,
) -> Result<(NaiveDate, NaiveDate)> {
    if let Some((start, _raw_end)) = named_range_bounds(spec, now, tz) {
        let start_local = start.with_timezone(&tz).date_naive();
        let end_local = now.with_timezone(&tz).date_naive();
        return Ok((start_local, end_local));
    }

    let duration = parse_duration_spec(spec)?;
    let mut days = duration.num_days();
    if days <= 0 {
        days = 1;
    }
    let end_local = now.with_timezone(&tz).date_naive();
    let start_local = if days <= 1 {
        end_local
    } else {
        end_local - ChronoDuration::days(days - 1)
    };

    Ok((start_local, end_local))
}

async fn fetch_timeseries_daily(
    state: Arc<AppState>,
    params: TimeseriesQuery,
    reporting_tz: Tz,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let now = Utc::now();
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let (start_date, end_date) = resolve_daily_date_range(&params.range, now, reporting_tz)?;

    let start_naive = start_date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    let start_dt = local_naive_to_utc(start_naive, reporting_tz);

    let mut aggregates: BTreeMap<NaiveDate, BucketAggregate> = BTreeMap::new();
    let mut cursor = start_date;
    while cursor <= end_date {
        aggregates.entry(cursor).or_default();
        cursor = cursor
            .succ_opt()
            .unwrap_or(cursor + ChronoDuration::days(1));
    }

    let mut records_query = QueryBuilder::new(
        "SELECT occurred_at, status, total_tokens, cost, t_upstream_ttfb_ms FROM codex_invocations WHERE occurred_at >= ",
    );
    records_query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        records_query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    records_query.push(" ORDER BY occurred_at ASC");
    let records = records_query
        .build_query_as::<TimeseriesRecord>()
        .fetch_all(&state.pool)
        .await?;

    for record in records {
        let occurred_utc = match parse_to_utc_datetime(&record.occurred_at) {
            Some(dt) => dt,
            None => continue,
        };
        let local_date = occurred_utc.with_timezone(&reporting_tz).date_naive();
        if local_date < start_date || local_date > end_date {
            continue;
        }
        let entry = aggregates.entry(local_date).or_default();
        entry.total_count += 1;
        match record.status.as_deref() {
            Some("success") => entry.success_count += 1,
            _ => entry.failure_count += 1,
        }
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
        entry.total_tokens += record.total_tokens.unwrap_or(0);
        entry.total_cost += record.cost.unwrap_or(0.0);
    }

    if source_scope == InvocationSourceScope::All
        && let Some(relay) = state.config.crs_stats.as_ref()
    {
        let deltas =
            query_crs_deltas(&state.pool, relay, start_dt.timestamp(), now.timestamp()).await?;

        for delta in deltas {
            let captured = match Utc.timestamp_opt(delta.captured_at_epoch, 0).single() {
                Some(dt) => dt,
                None => continue,
            };
            let local_date = captured.with_timezone(&reporting_tz).date_naive();
            if local_date < start_date || local_date > end_date {
                continue;
            }
            let entry = aggregates.entry(local_date).or_default();
            entry.total_count += delta.total_count;
            entry.success_count += delta.success_count;
            entry.failure_count += delta.failure_count;
            entry.total_tokens += delta.total_tokens;
            entry.total_cost += delta.total_cost;
        }
    }

    let mut points = Vec::with_capacity(aggregates.len());
    for (date, agg) in aggregates {
        let start_naive = date
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be representable");
        let end_naive = (date + ChronoDuration::days(1))
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be representable");
        let start = local_naive_to_utc(start_naive, reporting_tz);
        let end = local_naive_to_utc(end_naive, reporting_tz);
        let first_byte_avg_ms = agg.first_byte_avg_ms();
        let first_byte_p95_ms = agg.first_byte_p95_ms();
        points.push(TimeseriesPoint {
            bucket_start: format_utc_iso(start),
            bucket_end: format_utc_iso(end),
            total_count: agg.total_count,
            success_count: agg.success_count,
            failure_count: agg.failure_count,
            total_tokens: agg.total_tokens,
            total_cost: agg.total_cost,
            first_byte_sample_count: agg.first_byte_sample_count,
            first_byte_avg_ms,
            first_byte_p95_ms,
        });
    }

    let range_start = {
        let naive = start_date
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be representable");
        format_utc_iso(local_naive_to_utc(naive, reporting_tz))
    };
    let range_end = {
        let next = end_date + ChronoDuration::days(1);
        let naive = next
            .and_hms_opt(0, 0, 0)
            .expect("midnight should be representable");
        format_utc_iso(local_naive_to_utc(naive, reporting_tz))
    };

    Ok(Json(TimeseriesResponse {
        range_start,
        range_end,
        bucket_seconds: 86_400,
        points,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureScope {
    All,
    Service,
    Client,
    Abort,
}

impl FailureScope {
    fn parse(raw: Option<&str>) -> Result<Self, ApiError> {
        let Some(scope) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
            return Ok(FailureScope::Service);
        };
        match scope.to_ascii_lowercase().as_str() {
            "all" => Ok(FailureScope::All),
            "service" => Ok(FailureScope::Service),
            "client" => Ok(FailureScope::Client),
            "abort" => Ok(FailureScope::Abort),
            _ => Err(ApiError(anyhow!(
                "unsupported failure scope: {scope}; expected one of all|service|client|abort"
            ))),
        }
    }
}

fn failure_scope_matches(scope: FailureScope, class: FailureClass) -> bool {
    match scope {
        FailureScope::All => class != FailureClass::None,
        FailureScope::Service => class == FailureClass::ServiceFailure,
        FailureScope::Client => class == FailureClass::ClientFailure,
        FailureScope::Abort => class == FailureClass::ClientAbort,
    }
}

fn extract_failure_kind_prefix(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !trimmed.starts_with('[') {
        return None;
    }
    let closing = trimmed.find(']')?;
    if closing <= 1 {
        return None;
    }
    Some(trimmed[1..closing].trim().to_string())
}

fn derive_failure_kind(status_norm: &str, err: &str, err_lower: &str) -> Option<String> {
    if err_lower.contains("downstream closed while streaming upstream response") {
        return Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED.to_string());
    }
    if err_lower.contains("upstream stream error") {
        return Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string());
    }
    if err_lower.contains("failed to contact upstream") {
        return Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
    }
    if err_lower.contains("upstream handshake timed out") {
        return Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string());
    }
    if err_lower.contains("request body read timed out") {
        return Some(PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT.to_string());
    }
    if err_lower.contains("failed to read request body stream") {
        return Some(PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED.to_string());
    }
    if err_lower.contains("invalid api key format")
        || err_lower.contains("api key format is invalid")
        || err_lower.contains("incorrect api key provided")
    {
        return Some("invalid_api_key".to_string());
    }
    if err_lower.contains("api key not found") {
        return Some("api_key_not_found".to_string());
    }
    if err_lower.contains("please provide an api key") {
        return Some("api_key_missing".to_string());
    }
    if status_norm.starts_with("http_") {
        return Some(status_norm.to_string());
    }
    if !err.is_empty() {
        return Some("untyped_failure".to_string());
    }
    None
}

fn classify_invocation_failure(
    status: Option<&str>,
    error_message: Option<&str>,
) -> FailureClassification {
    let status_norm = status.unwrap_or_default().trim().to_ascii_lowercase();
    let err = error_message.unwrap_or_default().trim();
    let err_lower = err.to_ascii_lowercase();

    if status_norm == "success" && err.is_empty() {
        return FailureClassification {
            failure_kind: None,
            failure_class: FailureClass::None,
            is_actionable: false,
        };
    }

    let failure_kind = extract_failure_kind_prefix(err)
        .or_else(|| derive_failure_kind(&status_norm, err, &err_lower));

    let failure_kind_lower = failure_kind
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_http_4xx =
        status_norm.starts_with("http_4") || status_norm == "http_401" || status_norm == "http_403";
    let is_http_5xx = status_norm.starts_with("http_5");

    let failure_class = if failure_kind_lower == PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
        || err_lower.contains("downstream closed while streaming upstream response")
    {
        FailureClass::ClientAbort
    } else if failure_kind_lower == PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED
        || err_lower.contains("invalid api key format")
        || err_lower.contains("api key format is invalid")
        || err_lower.contains("incorrect api key provided")
        || err_lower.contains("api key not found")
        || err_lower.contains("please provide an api key")
        || is_http_4xx
    {
        FailureClass::ClientFailure
    } else if failure_kind_lower == PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_STREAM_ERROR
        || failure_kind_lower == PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
        || err_lower.contains("failed to contact upstream")
        || err_lower.contains("upstream stream error")
        || err_lower.contains("request body read timed out")
        || err_lower.contains("upstream handshake timed out")
        || is_http_5xx
    {
        FailureClass::ServiceFailure
    } else if status_norm == "success" {
        FailureClass::None
    } else {
        // Conservative fallback: unknown non-success records are treated as service-impacting.
        FailureClass::ServiceFailure
    };

    FailureClassification {
        failure_kind: if failure_class == FailureClass::None {
            None
        } else {
            failure_kind
        },
        failure_class,
        is_actionable: failure_class == FailureClass::ServiceFailure,
    }
}

fn resolve_failure_classification(
    status: Option<&str>,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    failure_class: Option<&str>,
    is_actionable: Option<i64>,
) -> FailureClassification {
    let derived = classify_invocation_failure(status, error_message);
    let stored_class = failure_class.and_then(FailureClass::from_db_str);
    let resolved_class = match stored_class {
        // Legacy rows can carry migration defaults (`none`/`0`) for non-success records.
        Some(FailureClass::None) if derived.failure_class != FailureClass::None => {
            derived.failure_class
        }
        Some(value) => value,
        None => derived.failure_class,
    };
    let resolved_kind = failure_kind
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or(derived.failure_kind);
    let expected_actionable = resolved_class == FailureClass::ServiceFailure;
    let resolved_actionable = is_actionable
        .map(|value| value != 0)
        .filter(|value| *value == expected_actionable)
        .unwrap_or(expected_actionable);

    FailureClassification {
        failure_kind: resolved_kind,
        failure_class: resolved_class,
        is_actionable: resolved_actionable,
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErrorQuery {
    range: String,
    top: Option<i64>,
    scope: Option<String>,
    time_zone: Option<String>,
}

#[derive(serde::Serialize)]
struct ErrorDistributionItem {
    reason: String,
    count: i64,
}

#[derive(serde::Serialize)]
struct ErrorDistributionResponse {
    range_start: String,
    range_end: String,
    items: Vec<ErrorDistributionItem>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OtherErrorsQuery {
    range: String,
    page: Option<i64>,
    limit: Option<i64>,
    scope: Option<String>,
    time_zone: Option<String>,
}

#[derive(serde::Serialize)]
struct OtherErrorItem {
    id: i64,
    occurred_at: String,
    error_message: Option<String>,
}

#[derive(serde::Serialize)]
struct OtherErrorsResponse {
    total: i64,
    page: i64,
    limit: i64,
    items: Vec<OtherErrorItem>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct FailureSummaryQuery {
    range: String,
    time_zone: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureSummaryResponse {
    range_start: String,
    range_end: String,
    total_failures: i64,
    service_failure_count: i64,
    client_failure_count: i64,
    client_abort_count: i64,
    actionable_failure_count: i64,
    actionable_failure_rate: f64,
}

async fn fetch_error_distribution(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ErrorQuery>,
) -> Result<Json<ErrorDistributionResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let display_end = range_window.display_end;
    let scope = FailureScope::parse(params.scope.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    #[derive(sqlx::FromRow)]
    struct RawErr {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }

    let mut query = QueryBuilder::new(
        "SELECT status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND (status IS NULL OR status != 'success')");
    let rows: Vec<RawErr> = query.build_query_as().fetch_all(&state.pool).await?;

    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for r in rows {
        let classification = resolve_failure_classification(
            r.status.as_deref(),
            r.error_message.as_deref(),
            r.failure_kind.as_deref(),
            r.failure_class.as_deref(),
            r.is_actionable,
        );
        if !failure_scope_matches(scope, classification.failure_class) {
            continue;
        }
        let raw = r.error_message.unwrap_or_default();
        let key = categorize_error(&raw);
        *counts.entry(key).or_insert(0) += 1;
    }

    let mut items: Vec<ErrorDistributionItem> = counts
        .into_iter()
        .map(|(reason, count)| ErrorDistributionItem { reason, count })
        .collect();
    items.sort_by(|a, b| b.count.cmp(&a.count));
    if let Some(top) = params.top {
        let limited = top.clamp(1, 50) as usize;
        if items.len() > limited {
            items.truncate(limited);
        }
    }

    Ok(Json(ErrorDistributionResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(display_end),
        items,
    }))
}

// Classify error message by rules:
// - If contains HTTP code >= 501, group as "HTTP <code>"
// - If 4xx: try to extract concrete type (json error.type or regex phrases); otherwise "HTTP <code>"
// - Otherwise: normalize message and if still not matched, return "Other"
fn categorize_error(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "Other".to_string();
    }

    if let Some(code) = extract_http_code(s) {
        if code >= 501 {
            return format!("HTTP {}", code);
        }
        if (400..500).contains(&code) {
            if let Some(t) = extract_json_error_type(s) {
                return t.to_string();
            }
            if RE_USAGE_NOT_INCLUDED.is_match(s) {
                return "usage_not_included".to_string();
            }
            if RE_USAGE_LIMIT_REACHED.is_match(s) {
                return "usage_limit_reached".to_string();
            }
            if code == 429 {
                if RE_TOO_MANY_REQUESTS.is_match(s) {
                    return "too_many_requests".to_string();
                }
                return "http_429".to_string();
            }
            if code == 401 {
                return "unauthorized".to_string();
            }
            if code == 403 {
                return "forbidden".to_string();
            }
            if code == 404 {
                return "not_found".to_string();
            }
            return format!("HTTP {}", code);
        }
    }

    // Fallback to normalized text; if empty -> Other
    let norm = normalize_error_reason(s);
    if norm == "Unknown" || norm.is_empty() {
        "Other".to_string()
    } else {
        norm
    }
}

fn normalize_error_reason(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "Unknown".to_string();
    }
    // Extract stable info from JSON payloads if present
    if s.starts_with('{')
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(s)
        && let Some(err) = v.get("error")
        && let Some(ty) = err.get("type").and_then(|x| x.as_str())
    {
        return format!("json error: {ty}");
    }

    let mut out = s.to_lowercase();

    static RE_HTTP: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\bhttp\s*(\d{3})\b").expect("valid regex"));
    let status = RE_HTTP
        .captures(&out)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    static RE_ISO_DT: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b\d{4}-\d{2}-\d{2}[ t]\d{2}:\d{2}:\d{2}(?:\.\d+)?z?\b").expect("valid regex")
    });
    out = RE_ISO_DT.replace_all(&out, "").into_owned();

    static RE_UUID: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
            .expect("valid regex")
    });
    out = RE_UUID.replace_all(&out, "").into_owned();

    static RE_LONG_ID: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\b[a-z0-9_\-]{10,}\b").expect("valid regex"));
    out = RE_LONG_ID.replace_all(&out, "").into_owned();

    static RE_URL: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"https?://[^\s'\"]+"#).expect("valid regex"));
    out = RE_URL
        .replace_all(&out, |caps: &regex::Captures| {
            let url = &caps[0];
            if let Ok(u) = reqwest::Url::parse(url) {
                format!(
                    "{}://{}{}",
                    u.scheme(),
                    u.host_str().unwrap_or(""),
                    u.path()
                )
            } else {
                String::new()
            }
        })
        .into_owned();

    static RE_BIG_NUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{4,}\b").expect("valid regex"));
    out = RE_BIG_NUM.replace_all(&out, "").into_owned();

    out = out.replace("request failed:", "request failed");
    out = out.replace("exception recovered:", "exception");

    static RE_WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").expect("valid regex"));
    out = RE_WS.replace_all(&out, " ").trim().to_string();

    if let Some(code) = status.as_ref().filter(|c| !out.contains(&c[..])) {
        out = format!("http {code}: {out}");
    }

    if out.is_empty() {
        "Unknown".to_string()
    } else {
        out.chars().take(160).collect()
    }
}

fn extract_http_code(s: &str) -> Option<u16> {
    static RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\bhttp\s*:?\s*(\d{3})\b").expect("valid regex"));
    RE.captures(s)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u16>().ok())
}

fn extract_json_error_type(s: &str) -> Option<String> {
    if !s.trim_start().starts_with('{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let ty = v
        .get("error")
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())?;
    Some(ty.to_string())
}

static RE_USAGE_NOT_INCLUDED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)usage[_\s-]*not[_\s-]*included").expect("valid regex"));
static RE_USAGE_LIMIT_REACHED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)usage[_\s-]*limit[_\s-]*reached").expect("valid regex"));
static RE_TOO_MANY_REQUESTS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)too\s+many\s+requests").expect("valid regex"));

async fn fetch_other_errors(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OtherErrorsQuery>,
) -> Result<Json<OtherErrorsResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let scope = FailureScope::parse(params.scope.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    #[derive(sqlx::FromRow)]
    struct RowItem {
        id: i64,
        occurred_at: String,
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }
    let mut query = QueryBuilder::new(
        "SELECT id, occurred_at, status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND (status IS NULL OR status != 'success') ORDER BY occurred_at DESC");
    let rows: Vec<RowItem> = query.build_query_as().fetch_all(&state.pool).await?;

    let mut others: Vec<RowItem> = Vec::new();
    for r in rows.into_iter() {
        let classification = resolve_failure_classification(
            r.status.as_deref(),
            r.error_message.as_deref(),
            r.failure_kind.as_deref(),
            r.failure_class.as_deref(),
            r.is_actionable,
        );
        if !failure_scope_matches(scope, classification.failure_class) {
            continue;
        }
        let msg = r.error_message.clone().unwrap_or_default();
        let cat = categorize_error(&msg);
        if cat == "Other" {
            others.push(r);
        }
    }

    let total = others.len() as i64;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let page = params.page.unwrap_or(1).max(1);
    let start = ((page - 1) * limit) as usize;
    let end = (start + limit as usize).min(others.len());
    let slice = if start < end {
        &others[start..end]
    } else {
        &[]
    };

    let items = slice
        .iter()
        .map(|r| OtherErrorItem {
            id: r.id,
            occurred_at: r.occurred_at.clone(),
            error_message: r.error_message.clone(),
        })
        .collect();

    Ok(Json(OtherErrorsResponse {
        total,
        page,
        limit,
        items,
    }))
}

async fn fetch_failure_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FailureSummaryQuery>,
) -> Result<Json<FailureSummaryResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let display_end = range_window.display_end;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    #[derive(sqlx::FromRow)]
    struct Row {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }

    let mut query = QueryBuilder::new(
        "SELECT status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND (status IS NULL OR status != 'success')");

    let rows: Vec<Row> = query.build_query_as().fetch_all(&state.pool).await?;
    let total_failures = rows.len() as i64;

    let mut service_failure_count = 0_i64;
    let mut client_failure_count = 0_i64;
    let mut client_abort_count = 0_i64;
    let mut actionable_failure_count = 0_i64;

    for row in rows {
        let classification = resolve_failure_classification(
            row.status.as_deref(),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        match classification.failure_class {
            FailureClass::ServiceFailure => service_failure_count += 1,
            FailureClass::ClientFailure => client_failure_count += 1,
            FailureClass::ClientAbort => client_abort_count += 1,
            FailureClass::None => {}
        }
        if classification.is_actionable {
            actionable_failure_count += 1;
        }
    }

    let actionable_failure_rate = if total_failures > 0 {
        actionable_failure_count as f64 / total_failures as f64
    } else {
        0.0
    };

    Ok(Json(FailureSummaryResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(display_end),
        total_failures,
        service_failure_count,
        client_failure_count,
        client_abort_count,
        actionable_failure_count,
        actionable_failure_rate,
    }))
}

async fn fetch_perf_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PerfQuery>,
) -> Result<Json<PerfStatsResponse>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct PerfTimingRow {
        t_total_ms: Option<f64>,
        t_req_read_ms: Option<f64>,
        t_req_parse_ms: Option<f64>,
        t_upstream_connect_ms: Option<f64>,
        t_upstream_ttfb_ms: Option<f64>,
        t_upstream_stream_ms: Option<f64>,
        t_resp_parse_ms: Option<f64>,
        t_persist_ms: Option<f64>,
    }

    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let mut query = QueryBuilder::new(
        "SELECT \
            t_total_ms, t_req_read_ms, t_req_parse_ms, \
            t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, \
            t_resp_parse_ms, t_persist_ms \
         FROM codex_invocations \
         WHERE source = ",
    );
    query
        .push_bind(SOURCE_PROXY)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range_window.start))
        .push(" AND occurred_at <= ")
        .push_bind(db_occurred_at_lower_bound(range_window.display_end));
    let rows: Vec<PerfTimingRow> = query.build_query_as().fetch_all(&state.pool).await?;

    let stage_series: Vec<(&str, Vec<f64>)> = vec![
        (
            "total",
            rows.iter()
                .filter_map(|row| row.t_total_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "requestRead",
            rows.iter()
                .filter_map(|row| row.t_req_read_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "requestParse",
            rows.iter()
                .filter_map(|row| row.t_req_parse_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamConnect",
            rows.iter()
                .filter_map(|row| row.t_upstream_connect_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamFirstByte",
            rows.iter()
                .filter_map(|row| row.t_upstream_ttfb_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamStream",
            rows.iter()
                .filter_map(|row| row.t_upstream_stream_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "responseParse",
            rows.iter()
                .filter_map(|row| row.t_resp_parse_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "persistence",
            rows.iter()
                .filter_map(|row| row.t_persist_ms)
                .collect::<Vec<_>>(),
        ),
    ];

    let mut stages = Vec::new();
    for (stage, mut values) in stage_series {
        if values.is_empty() {
            continue;
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let count = values.len() as i64;
        let sum = values.iter().copied().sum::<f64>();
        let max_ms = values.last().copied().unwrap_or(0.0);
        stages.push(PerfStageStats {
            stage: stage.to_string(),
            count,
            avg_ms: sum / count as f64,
            p50_ms: percentile_sorted_f64(&values, 0.50),
            p90_ms: percentile_sorted_f64(&values, 0.90),
            p99_ms: percentile_sorted_f64(&values, 0.99),
            max_ms,
        });
    }

    Ok(Json(PerfStatsResponse {
        range_start: format_utc_iso(range_window.start),
        range_end: format_utc_iso(range_window.display_end),
        source: SOURCE_PROXY.to_string(),
        stages,
    }))
}

async fn latest_quota_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<QuotaSnapshotResponse>, ApiError> {
    let snapshot = QuotaSnapshotResponse::fetch_latest(&state.pool)
        .await?
        .unwrap_or_else(QuotaSnapshotResponse::degraded_default);
    Ok(Json(snapshot))
}

async fn broadcast_summary_if_changed(
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    cache: &Mutex<BroadcastStateCache>,
    window: &str,
    summary: StatsResponse,
) -> Result<bool, broadcast::error::SendError<BroadcastPayload>> {
    let mut cache = cache.lock().await;
    if cache
        .summaries
        .get(window)
        .is_some_and(|current| current == &summary)
    {
        return Ok(false);
    }

    broadcaster.send(BroadcastPayload::Summary {
        window: window.to_string(),
        summary: summary.clone(),
    })?;
    cache.summaries.insert(window.to_string(), summary);
    Ok(true)
}

async fn broadcast_quota_if_changed(
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    cache: &Mutex<BroadcastStateCache>,
    snapshot: QuotaSnapshotResponse,
) -> Result<bool, broadcast::error::SendError<BroadcastPayload>> {
    let mut cache = cache.lock().await;
    if cache
        .quota
        .as_ref()
        .is_some_and(|current| current == &snapshot)
    {
        return Ok(false);
    }

    broadcaster.send(BroadcastPayload::Quota {
        snapshot: Box::new(snapshot.clone()),
    })?;
    cache.quota = Some(snapshot);
    Ok(true)
}

async fn sse_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.broadcaster.subscribe();
    let broadcast = BroadcastStream::new(rx).filter_map(|res| async {
        match res {
            Ok(payload) => match Event::default().json_data(&payload) {
                Ok(event) => Some(Ok(event)),
                Err(err) => {
                    warn!(?err, "failed to serialize sse payload");
                    None
                }
            },
            Err(err) => {
                warn!(?err, "sse broadcast stream lagging");
                None
            }
        }
    });
    // Seed a version event on connect so clients know the current server version immediately
    let initial = {
        let (backend, _frontend) = detect_versions(state.config.static_dir.as_deref());
        let payload = BroadcastPayload::Version { version: backend };
        let ev = Event::default().json_data(&payload);
        match ev {
            Ok(event) => stream::iter(vec![Ok(event)]),
            Err(_) => stream::iter(Vec::<Result<Event, Infallible>>::new()),
        }
    };

    let merged = initial.chain(broadcast);
    Sse::new(merged).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn health_check(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if state.startup_ready.load(Ordering::Acquire) {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "starting")
    }
}

#[cfg(test)]
async fn proxy_openai_v1(
    State(state): State<Arc<AppState>>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    proxy_openai_v1_common(state, original_uri, method, headers, body, None).await
}

async fn proxy_openai_v1_with_connect_info(
    State(state): State<Arc<AppState>>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    proxy_openai_v1_common(
        state,
        original_uri,
        method,
        headers,
        body,
        connect_info.map(|info| info.0.ip()),
    )
    .await
}

async fn proxy_openai_v1_common(
    state: Arc<AppState>,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    peer_ip: Option<IpAddr>,
) -> Response {
    let proxy_request_id = next_proxy_request_id();
    let started_at = Instant::now();
    let request_content_length = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    let request_may_have_body = request_may_have_body(&method, &headers);
    let method_for_log = method.clone();
    let uri_for_log = original_uri.clone();

    info!(
        proxy_request_id,
        method = %method_for_log,
        uri = %uri_for_log,
        has_body = request_may_have_body,
        content_length = ?request_content_length,
        peer_ip = ?peer_ip,
        "openai proxy request started"
    );

    match proxy_openai_v1_inner(
        state,
        proxy_request_id,
        original_uri,
        method,
        headers,
        body,
        peer_ip,
    )
    .await
    {
        Ok(response) => {
            let status = response.status();
            info!(
                proxy_request_id,
                method = %method_for_log,
                uri = %uri_for_log,
                status = %status,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy response headers ready"
            );
            response
        }
        Err((status, message)) => {
            warn!(
                proxy_request_id,
                method = %method_for_log,
                uri = %uri_for_log,
                status = %status,
                error = %message,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy request failed"
            );
            (status, Json(json!({ "error": message }))).into_response()
        }
    }
}

async fn proxy_openai_v1_inner(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    peer_ip: Option<IpAddr>,
) -> Result<Response, (StatusCode, String)> {
    let target_url =
        build_proxy_upstream_url(&state.config.openai_upstream_base_url, &original_uri).map_err(
            |err| {
                let status = if err.to_string().contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED)
                    || err.to_string().contains(PROXY_INVALID_REQUEST_TARGET)
                    || err
                        .to_string()
                        .contains("failed to parse proxy upstream url")
                {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };
                (status, format!("failed to build upstream url: {err}"))
            },
        )?;

    if method == Method::GET && is_models_list_path(original_uri.path()) {
        let settings = state.proxy_model_settings.read().await.clone();
        if settings.hijack_enabled {
            let mut payload = build_preset_models_payload(&settings.enabled_preset_models);
            let mut merge_status: Option<&'static str> = None;
            if settings.merge_upstream_enabled {
                let selected_proxy = select_forward_proxy_for_request(state.as_ref()).await;
                match fetch_upstream_models_payload(
                    state.clone(),
                    selected_proxy,
                    target_url.clone(),
                    &headers,
                )
                .await
                {
                    Ok(upstream_payload) => {
                        match merge_models_payload_with_upstream(
                            &upstream_payload,
                            &settings.enabled_preset_models,
                        ) {
                            Ok(merged_payload) => {
                                payload = merged_payload;
                                merge_status = Some(PROXY_MODEL_MERGE_STATUS_SUCCESS);
                            }
                            Err(err) => {
                                warn!(
                                    proxy_request_id,
                                    error = %err,
                                    "failed to merge upstream model list; falling back to preset models"
                                );
                                merge_status = Some(PROXY_MODEL_MERGE_STATUS_FAILED);
                            }
                        }
                    }
                    Err(err) => {
                        warn!(
                            proxy_request_id,
                            error = %err,
                            "failed to fetch upstream model list for merge; falling back to preset models"
                        );
                        merge_status = Some(PROXY_MODEL_MERGE_STATUS_FAILED);
                    }
                }
            }

            let mut response = Json(payload).into_response();
            if let Some(status) = merge_status {
                response.headers_mut().insert(
                    HeaderName::from_static(PROXY_MODEL_MERGE_STATUS_HEADER),
                    HeaderValue::from_static(status),
                );
            }
            return Ok(response);
        }
    }

    let selected_proxy = select_forward_proxy_for_request(state.as_ref()).await;

    if let Some(target) = capture_target_for_request(original_uri.path(), &method) {
        return proxy_openai_v1_capture_target(
            state,
            proxy_request_id,
            headers,
            body,
            target,
            target_url,
            peer_ip,
            selected_proxy,
        )
        .await;
    }

    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    if let Some(content_length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        && content_length > body_limit
    {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("request body exceeds {body_limit} bytes"),
        ));
    }

    let mut seen_body_bytes = 0usize;
    let request_body_stream = body.into_data_stream().map(move |chunk| {
        let chunk = chunk.map_err(|err| {
            warn!(
                proxy_request_id,
                error = %err,
                "openai proxy request body stream error"
            );
            io::Error::other(format!("failed to read request body stream: {err}"))
        })?;
        seen_body_bytes = seen_body_bytes.saturating_add(chunk.len());
        if seen_body_bytes > body_limit {
            Err(io::Error::other(PROXY_REQUEST_BODY_LIMIT_EXCEEDED))
        } else {
            Ok(chunk)
        }
    });

    let proxy_client = state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                format!("failed to initialize forward proxy client: {err}"),
            )
        })?;
    let mut upstream_request = proxy_client
        .request(method, target_url)
        .body(reqwest::Body::wrap_stream(request_body_stream));

    let request_connection_scoped = connection_scoped_header_names(&headers);
    for (name, value) in &headers {
        if should_forward_proxy_header(name, &request_connection_scoped) {
            upstream_request = upstream_request.header(name, value);
        }
    }

    let map_upstream_error = |err: reqwest::Error| {
        if is_body_too_large_error(&err) {
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("request body exceeds {body_limit} bytes"),
            )
        } else {
            (
                StatusCode::BAD_GATEWAY,
                format!("failed to contact upstream: {err}"),
            )
        }
    };

    let connect_started = Instant::now();
    let handshake_timeout = state.config.openai_proxy_handshake_timeout;
    let upstream_response = match timeout(handshake_timeout, upstream_request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            let mapped = map_upstream_error(err);
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(elapsed_ms(connect_started)),
                Some(FORWARD_PROXY_FAILURE_SEND_ERROR),
                false,
            )
            .await;
            return Err(mapped);
        }
        Err(_) => {
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(elapsed_ms(connect_started)),
                Some(FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT),
                false,
            )
            .await;
            return Err((
                StatusCode::BAD_GATEWAY,
                format!(
                    "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
                    handshake_timeout.as_millis()
                ),
            ));
        }
    };

    let rewritten_location = match normalize_proxy_location_header(
        upstream_response.status(),
        upstream_response.headers(),
        &state.config.openai_upstream_base_url,
    ) {
        Ok(location) => location,
        Err(err) => {
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(elapsed_ms(connect_started)),
                Some(FORWARD_PROXY_FAILURE_SEND_ERROR),
                false,
            )
            .await;
            return Err((
                StatusCode::BAD_GATEWAY,
                format!("failed to process upstream redirect: {err}"),
            ));
        }
    };

    let upstream_status = upstream_response.status();
    let upstream_connection_scoped = connection_scoped_header_names(upstream_response.headers());
    let mut response_builder = Response::builder().status(upstream_status);
    for (name, value) in upstream_response.headers() {
        if should_forward_proxy_header(name, &upstream_connection_scoped) {
            if name == header::LOCATION {
                if let Some(rewritten) = rewritten_location.as_deref() {
                    response_builder = response_builder.header(name, rewritten);
                }
            } else {
                response_builder = response_builder.header(name, value);
            }
        }
    }

    let mut upstream_stream = upstream_response.bytes_stream();
    let stream_ttfb_started = Instant::now();
    let first_chunk = match upstream_stream.next().await {
        Some(Ok(chunk)) => {
            info!(
                proxy_request_id,
                ttfb_ms = stream_ttfb_started.elapsed().as_millis(),
                first_chunk_bytes = chunk.len(),
                "openai proxy upstream response first chunk ready"
            );
            Some(chunk)
        }
        Some(Err(err)) => {
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(elapsed_ms(connect_started)),
                Some(FORWARD_PROXY_FAILURE_STREAM_ERROR),
                false,
            )
            .await;
            warn!(
                proxy_request_id,
                error = %err,
                "openai proxy upstream response stream failed before first chunk"
            );
            return Err((
                StatusCode::BAD_GATEWAY,
                format!("upstream stream error before first chunk: {err}"),
            ));
        }
        None => {
            let success = !upstream_status.is_server_error();
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                success,
                Some(elapsed_ms(connect_started)),
                if success {
                    None
                } else {
                    Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
                },
                false,
            )
            .await;
            info!(
                proxy_request_id,
                ttfb_ms = stream_ttfb_started.elapsed().as_millis(),
                "openai proxy upstream response stream completed without body"
            );
            return response_builder.body(Body::empty()).map_err(|err| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("failed to build proxy response: {err}"),
                )
            });
        }
    };

    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
    let state_for_record = state.clone();
    let selected_proxy_for_record = selected_proxy.clone();
    let upstream_status_for_record = upstream_status;
    tokio::spawn(async move {
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;
        let stream_started_at = Instant::now();
        let mut stream_error_happened = false;
        let mut downstream_closed = false;

        if let Some(chunk) = first_chunk {
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            if tx.send(Ok(chunk)).await.is_err() {
                info!(
                    proxy_request_id,
                    forwarded_chunks,
                    forwarded_bytes,
                    elapsed_ms = stream_started_at.elapsed().as_millis(),
                    "openai proxy downstream closed before first streamed chunk"
                );
                downstream_closed = true;
            }
        }

        loop {
            if downstream_closed {
                break;
            }
            let Some(next_chunk) = upstream_stream.next().await else {
                break;
            };
            match next_chunk {
                Ok(chunk) => {
                    forwarded_chunks = forwarded_chunks.saturating_add(1);
                    forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                    if tx.send(Ok(chunk)).await.is_err() {
                        info!(
                            proxy_request_id,
                            forwarded_chunks,
                            forwarded_bytes,
                            elapsed_ms = stream_started_at.elapsed().as_millis(),
                            "openai proxy downstream closed while streaming upstream response"
                        );
                        break;
                    }
                }
                Err(err) => {
                    stream_error_happened = true;
                    warn!(
                        proxy_request_id,
                        error = %err,
                        forwarded_chunks,
                        forwarded_bytes,
                        elapsed_ms = stream_started_at.elapsed().as_millis(),
                        "openai proxy upstream response stream error"
                    );
                    let _ = tx
                        .send(Err(io::Error::other(format!(
                            "upstream stream error: {err}"
                        ))))
                        .await;
                    break;
                }
            }
        }

        let success = !stream_error_happened && !upstream_status_for_record.is_server_error();
        record_forward_proxy_attempt(
            state_for_record,
            selected_proxy_for_record,
            success,
            Some(elapsed_ms(connect_started)),
            if success {
                None
            } else if stream_error_happened {
                Some(FORWARD_PROXY_FAILURE_STREAM_ERROR)
            } else {
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
            },
            false,
        )
        .await;

        info!(
            proxy_request_id,
            forwarded_chunks,
            forwarded_bytes,
            elapsed_ms = stream_started_at.elapsed().as_millis(),
            "openai proxy upstream response stream completed"
        );
    });

    response_builder
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        })
}

fn capture_target_for_request(path: &str, method: &Method) -> Option<ProxyCaptureTarget> {
    if *method != Method::POST {
        return None;
    }
    match path {
        "/v1/chat/completions" => Some(ProxyCaptureTarget::ChatCompletions),
        "/v1/responses" => Some(ProxyCaptureTarget::Responses),
        "/v1/responses/compact" => Some(ProxyCaptureTarget::ResponsesCompact),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
async fn proxy_openai_v1_capture_target(
    state: Arc<AppState>,
    proxy_request_id: u64,
    headers: HeaderMap,
    body: Body,
    capture_target: ProxyCaptureTarget,
    target_url: Url,
    peer_ip: Option<IpAddr>,
    selected_proxy: SelectedForwardProxy,
) -> Result<Response, (StatusCode, String)> {
    let capture_started = Instant::now();
    let occurred_at_utc = Utc::now();
    let occurred_at = format_naive(occurred_at_utc.with_timezone(&Shanghai).naive_local());
    let invoke_id = format!(
        "proxy-{proxy_request_id}-{}",
        occurred_at_utc.timestamp_millis()
    );
    let raw_expires_at = compute_raw_expires_at(occurred_at_utc, state.config.proxy_raw_retention);
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let requester_ip = extract_requester_ip(&headers, peer_ip);
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);

    let req_read_started = Instant::now();
    let request_body_bytes = match read_request_body_with_limit(
        body,
        body_limit,
        state.config.openai_proxy_request_read_timeout,
        proxy_request_id,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(read_err) => {
            let t_req_read_ms = elapsed_ms(req_read_started);
            let request_info = RequestCaptureInfo::default();
            let req_raw = store_raw_payload_file(
                &state.config,
                &invoke_id,
                "request",
                &read_err.partial_body,
            );
            let usage = ParsedUsage::default();
            let (cost, cost_estimated, price_version) =
                estimate_proxy_cost_from_shared_catalog(&state.pricing_catalog, None, &usage).await;
            let error_message = format!("[{}] {}", read_err.failure_kind, read_err.message);

            warn!(
                proxy_request_id,
                status = %read_err.status,
                failure_kind = read_err.failure_kind,
                error = %read_err.message,
                elapsed_ms = t_req_read_ms,
                "openai proxy request body read failed"
            );

            let record = ProxyCaptureRecord {
                invoke_id,
                occurred_at,
                model: None,
                usage,
                cost,
                cost_estimated,
                price_version,
                status: if read_err.status.is_server_error() {
                    format!("http_{}", read_err.status.as_u16())
                } else {
                    "failed".to_string()
                },
                error_message: Some(error_message),
                payload: Some(build_proxy_payload_summary(
                    capture_target,
                    read_err.status,
                    request_info.is_stream,
                    None,
                    request_info.requested_service_tier.as_deref(),
                    request_info.reasoning_effort.as_deref(),
                    None,
                    None,
                    request_info.parse_error.as_deref(),
                    Some(read_err.failure_kind),
                    requester_ip.as_deref(),
                    header_prompt_cache_key.as_deref(),
                    None,
                    Some(selected_proxy.display_name.as_str()),
                    None,
                )),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
                raw_expires_at,
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms: 0.0,
                    t_upstream_connect_ms: 0.0,
                    t_upstream_ttfb_ms: 0.0,
                    t_upstream_stream_ms: 0.0,
                    t_resp_parse_ms: 0.0,
                    t_persist_ms: 0.0,
                },
            };
            if let Err(err) =
                persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
            {
                warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            }
            return Err((read_err.status, read_err.message));
        }
    };
    let t_req_read_ms = elapsed_ms(req_read_started);

    let proxy_fast_mode_rewrite_mode = state
        .proxy_model_settings
        .read()
        .await
        .fast_mode_rewrite_mode;
    let req_parse_started = Instant::now();
    let (upstream_body, request_info, body_rewritten) = prepare_target_request_body(
        capture_target,
        request_body_bytes,
        state.config.proxy_enforce_stream_include_usage,
        proxy_fast_mode_rewrite_mode,
    );
    let prompt_cache_key = request_info
        .prompt_cache_key
        .clone()
        .or_else(|| header_prompt_cache_key.clone());
    let t_req_parse_ms = elapsed_ms(req_parse_started);
    let req_raw = store_raw_payload_file(&state.config, &invoke_id, "request", &upstream_body);

    let proxy_client = state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                format!("failed to initialize forward proxy client: {err}"),
            )
        })?;
    let mut upstream_request = proxy_client
        .request(Method::POST, target_url)
        .body(upstream_body.clone());
    let request_connection_scoped = connection_scoped_header_names(&headers);
    for (name, value) in &headers {
        if !should_forward_proxy_header(name, &request_connection_scoped) {
            continue;
        }
        if name == header::CONTENT_LENGTH && body_rewritten {
            continue;
        }
        upstream_request = upstream_request.header(name, value);
    }

    let map_upstream_error = |err: reqwest::Error| {
        if is_body_too_large_error(&err) {
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("request body exceeds {body_limit} bytes"),
                PROXY_FAILURE_BODY_TOO_LARGE,
            )
        } else {
            (
                StatusCode::BAD_GATEWAY,
                format!("failed to contact upstream: {err}"),
                PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            )
        }
    };

    let connect_started = Instant::now();
    let handshake_timeout = state.config.openai_proxy_handshake_timeout;
    let upstream_response = match timeout(handshake_timeout, upstream_request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(err)) => {
            let (status, message, failure_kind) = map_upstream_error(err);
            let proxy_attempt_update = record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(elapsed_ms(connect_started)),
                Some(FORWARD_PROXY_FAILURE_SEND_ERROR),
                false,
            )
            .await;
            let usage = ParsedUsage::default();
            let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
                &state.pricing_catalog,
                request_info.model.as_deref(),
                &usage,
            )
            .await;
            let error_message = format!("[{failure_kind}] {message}");
            let record = ProxyCaptureRecord {
                invoke_id,
                occurred_at,
                model: request_info.model,
                usage,
                cost,
                cost_estimated,
                price_version,
                status: if status.is_server_error() {
                    format!("http_{}", status.as_u16())
                } else {
                    "failed".to_string()
                },
                error_message: Some(error_message),
                payload: Some(build_proxy_payload_summary(
                    capture_target,
                    status,
                    request_info.is_stream,
                    None,
                    request_info.requested_service_tier.as_deref(),
                    request_info.reasoning_effort.as_deref(),
                    None,
                    None,
                    request_info.parse_error.as_deref(),
                    Some(failure_kind),
                    requester_ip.as_deref(),
                    prompt_cache_key.as_deref(),
                    None,
                    Some(selected_proxy.display_name.as_str()),
                    proxy_attempt_update.delta(),
                )),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
                raw_expires_at,
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms,
                    t_upstream_connect_ms: elapsed_ms(connect_started),
                    t_upstream_ttfb_ms: 0.0,
                    t_upstream_stream_ms: 0.0,
                    t_resp_parse_ms: 0.0,
                    t_persist_ms: 0.0,
                },
            };
            if let Err(err) =
                persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
            {
                warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            }
            return Err((status, message));
        }
        Err(_) => {
            let message = format!(
                "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
                handshake_timeout.as_millis()
            );
            let failure_kind = PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT;
            let proxy_attempt_update = record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(elapsed_ms(connect_started)),
                Some(FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT),
                false,
            )
            .await;
            let usage = ParsedUsage::default();
            let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
                &state.pricing_catalog,
                request_info.model.as_deref(),
                &usage,
            )
            .await;
            let error_message = format!("[{failure_kind}] {message}");
            let record = ProxyCaptureRecord {
                invoke_id,
                occurred_at,
                model: request_info.model,
                usage,
                cost,
                cost_estimated,
                price_version,
                status: "http_502".to_string(),
                error_message: Some(error_message),
                payload: Some(build_proxy_payload_summary(
                    capture_target,
                    StatusCode::BAD_GATEWAY,
                    request_info.is_stream,
                    None,
                    request_info.requested_service_tier.as_deref(),
                    request_info.reasoning_effort.as_deref(),
                    None,
                    None,
                    request_info.parse_error.as_deref(),
                    Some(failure_kind),
                    requester_ip.as_deref(),
                    prompt_cache_key.as_deref(),
                    None,
                    Some(selected_proxy.display_name.as_str()),
                    proxy_attempt_update.delta(),
                )),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
                raw_expires_at,
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms,
                    t_upstream_connect_ms: elapsed_ms(connect_started),
                    t_upstream_ttfb_ms: 0.0,
                    t_upstream_stream_ms: 0.0,
                    t_resp_parse_ms: 0.0,
                    t_persist_ms: 0.0,
                },
            };
            if let Err(err) =
                persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
            {
                warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            }
            return Err((StatusCode::BAD_GATEWAY, message));
        }
    };
    let t_upstream_connect_ms = elapsed_ms(connect_started);

    let upstream_status = upstream_response.status();
    let rewritten_location = match normalize_proxy_location_header(
        upstream_status,
        upstream_response.headers(),
        &state.config.openai_upstream_base_url,
    ) {
        Ok(location) => location,
        Err(err) => {
            let message = format!("failed to process upstream redirect: {err}");
            let proxy_attempt_update = record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(t_upstream_connect_ms),
                Some(FORWARD_PROXY_FAILURE_SEND_ERROR),
                false,
            )
            .await;
            let usage = ParsedUsage::default();
            let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
                &state.pricing_catalog,
                request_info.model.as_deref(),
                &usage,
            )
            .await;
            let record = ProxyCaptureRecord {
                invoke_id,
                occurred_at,
                model: request_info.model,
                usage,
                cost,
                cost_estimated,
                price_version,
                status: "http_502".to_string(),
                error_message: Some(message.clone()),
                payload: Some(build_proxy_payload_summary(
                    capture_target,
                    StatusCode::BAD_GATEWAY,
                    request_info.is_stream,
                    None,
                    request_info.requested_service_tier.as_deref(),
                    request_info.reasoning_effort.as_deref(),
                    None,
                    None,
                    request_info.parse_error.as_deref(),
                    None,
                    requester_ip.as_deref(),
                    prompt_cache_key.as_deref(),
                    None,
                    Some(selected_proxy.display_name.as_str()),
                    proxy_attempt_update.delta(),
                )),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
                raw_expires_at,
                timings: StageTimings {
                    t_total_ms: 0.0,
                    t_req_read_ms,
                    t_req_parse_ms,
                    t_upstream_connect_ms,
                    t_upstream_ttfb_ms: 0.0,
                    t_upstream_stream_ms: 0.0,
                    t_resp_parse_ms: 0.0,
                    t_persist_ms: 0.0,
                },
            };
            if let Err(err) =
                persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record).await
            {
                warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
            }
            return Err((StatusCode::BAD_GATEWAY, message));
        }
    };

    let upstream_connection_scoped = connection_scoped_header_names(upstream_response.headers());
    let upstream_content_encoding = upstream_response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut response_builder = Response::builder().status(upstream_status);
    for (name, value) in upstream_response.headers() {
        if should_forward_proxy_header(name, &upstream_connection_scoped) {
            if name == header::LOCATION {
                if let Some(rewritten) = rewritten_location.as_deref() {
                    response_builder = response_builder.header(name, rewritten);
                }
            } else {
                response_builder = response_builder.header(name, value);
            }
        }
    }

    let state_for_task = state.clone();
    let request_info_for_task = request_info.clone();
    let req_raw_for_task = req_raw.clone();
    let invoke_id_for_task = invoke_id.clone();
    let occurred_at_for_task = occurred_at.clone();
    let raw_expires_at_for_task = raw_expires_at.clone();
    let upstream_content_encoding_for_task = upstream_content_encoding.clone();
    let requester_ip_for_task = requester_ip.clone();
    let prompt_cache_key_for_task = prompt_cache_key.clone();
    let selected_proxy_for_task = selected_proxy.clone();
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);

    tokio::spawn(async move {
        let mut stream = upstream_response.bytes_stream();
        let ttfb_started = Instant::now();
        let stream_started = Instant::now();
        let mut t_upstream_ttfb_ms = 0.0;
        let mut stream_started_at: Option<Instant> = None;
        let mut response_bytes: Vec<u8> = Vec::new();
        let mut stream_error: Option<String> = None;
        let mut downstream_closed = false;
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;

        while let Some(next_chunk) = stream.next().await {
            match next_chunk {
                Ok(chunk) => {
                    if stream_started_at.is_none() {
                        t_upstream_ttfb_ms = elapsed_ms(ttfb_started);
                        stream_started_at = Some(Instant::now());
                    }
                    response_bytes.extend_from_slice(&chunk);
                    forwarded_chunks = forwarded_chunks.saturating_add(1);
                    forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
                    if !downstream_closed && tx.send(Ok(chunk)).await.is_err() {
                        downstream_closed = true;
                    }
                }
                Err(err) => {
                    let msg = format!("upstream stream error: {err}");
                    stream_error = Some(msg.clone());
                    if !downstream_closed {
                        let _ = tx.send(Err(io::Error::other(msg))).await;
                    }
                    break;
                }
            }
        }
        drop(tx);

        let terminal_state = if stream_error.is_some() {
            PROXY_STREAM_TERMINAL_ERROR
        } else if downstream_closed {
            PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
        } else {
            PROXY_STREAM_TERMINAL_COMPLETED
        };
        if let Some(err) = stream_error.as_deref() {
            warn!(
                proxy_request_id,
                terminal_state,
                error = err,
                forwarded_chunks,
                forwarded_bytes,
                elapsed_ms = stream_started.elapsed().as_millis(),
                "openai proxy capture stream finished with upstream error"
            );
        } else {
            info!(
                proxy_request_id,
                terminal_state,
                forwarded_chunks,
                forwarded_bytes,
                elapsed_ms = stream_started.elapsed().as_millis(),
                "openai proxy capture stream finished"
            );
        }

        let t_upstream_stream_ms = stream_started_at.map(elapsed_ms).unwrap_or(0.0);
        let resp_parse_started = Instant::now();
        let mut response_info = parse_target_response_payload(
            capture_target,
            &response_bytes,
            request_info_for_task.is_stream,
            upstream_content_encoding_for_task.as_deref(),
        );
        let t_resp_parse_ms = elapsed_ms(resp_parse_started);

        if response_info.model.is_none() {
            response_info.model = request_info_for_task.model.clone();
        }
        if response_info.usage_missing_reason.is_none() && stream_error.is_some() {
            response_info.usage_missing_reason = Some("upstream_stream_error".to_string());
        }

        let failure_kind = if stream_error.is_some() {
            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
        } else if downstream_closed {
            Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
        } else {
            None
        };
        let had_stream_error = stream_error.is_some();

        let error_message = if let Some(err) = stream_error {
            Some(format!("[{}] {err}", PROXY_FAILURE_UPSTREAM_STREAM_ERROR))
        } else if downstream_closed {
            Some(format!(
                "[{}] downstream closed while streaming upstream response",
                PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
            ))
        } else if !upstream_status.is_success() {
            extract_error_message_from_response(&response_bytes)
        } else {
            None
        };
        let status = if upstream_status.is_success() && error_message.is_none() {
            "success".to_string()
        } else {
            format!("http_{}", upstream_status.as_u16())
        };
        let selected_proxy_display_name = selected_proxy_for_task.display_name.clone();
        let forward_proxy_success = !had_stream_error && !upstream_status.is_server_error();
        let proxy_attempt_update = record_forward_proxy_attempt(
            state_for_task.clone(),
            selected_proxy_for_task,
            forward_proxy_success,
            Some(t_upstream_connect_ms + t_upstream_ttfb_ms + t_upstream_stream_ms),
            if forward_proxy_success {
                None
            } else if had_stream_error {
                Some(FORWARD_PROXY_FAILURE_STREAM_ERROR)
            } else {
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
            },
            false,
        )
        .await;
        let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
            &state_for_task.pricing_catalog,
            response_info.model.as_deref(),
            &response_info.usage,
        )
        .await;
        let resp_raw = store_raw_payload_file(
            &state_for_task.config,
            &invoke_id_for_task,
            "response",
            &response_bytes,
        );
        let payload = build_proxy_payload_summary(
            capture_target,
            upstream_status,
            request_info_for_task.is_stream,
            request_info_for_task.model.as_deref(),
            request_info_for_task.requested_service_tier.as_deref(),
            request_info_for_task.reasoning_effort.as_deref(),
            response_info.model.as_deref(),
            response_info.usage_missing_reason.as_deref(),
            request_info_for_task.parse_error.as_deref(),
            failure_kind,
            requester_ip_for_task.as_deref(),
            prompt_cache_key_for_task.as_deref(),
            response_info.service_tier.as_deref(),
            Some(selected_proxy_display_name.as_str()),
            proxy_attempt_update.delta(),
        );

        let record = ProxyCaptureRecord {
            invoke_id: invoke_id_for_task,
            occurred_at: occurred_at_for_task,
            model: response_info.model,
            usage: response_info.usage,
            cost,
            cost_estimated,
            price_version,
            status,
            error_message,
            payload: Some(payload),
            raw_response: build_raw_response_preview(&response_bytes),
            req_raw: req_raw_for_task,
            resp_raw,
            raw_expires_at: raw_expires_at_for_task,
            timings: StageTimings {
                t_total_ms: 0.0,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms: 0.0,
            },
        };

        if let Err(err) =
            persist_and_broadcast_proxy_capture(state_for_task.as_ref(), capture_started, record)
                .await
        {
            warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
        }
    });

    response_builder
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        })
}

async fn read_request_body_with_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<Vec<u8>, RequestBodyReadError> {
    let mut data = Vec::new();
    let mut stream = body.into_data_stream();
    let read_deadline = Instant::now() + request_read_timeout;

    loop {
        let remaining = read_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                proxy_request_id,
                timeout_ms = request_read_timeout.as_millis(),
                read_bytes = data.len(),
                "openai proxy request body read timed out"
            );
            return Err(RequestBodyReadError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    request_read_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                partial_body: data,
            });
        }

        let next_chunk = match timeout(remaining, stream.next()).await {
            Ok(chunk) => chunk,
            Err(_) => {
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data.len(),
                    "openai proxy request body read timed out"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: data,
                });
            }
        };

        let Some(chunk) = next_chunk else {
            break;
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    error = %err,
                    read_bytes = data.len(),
                    "openai proxy request body stream error"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("failed to read request body stream: {err}"),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                    partial_body: data,
                });
            }
        };

        if data.len().saturating_add(chunk.len()) > body_limit {
            let allowed = body_limit.saturating_sub(data.len());
            if allowed > 0 {
                data.extend_from_slice(&chunk[..allowed.min(chunk.len())]);
            }
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body: data,
            });
        }

        data.extend_from_slice(&chunk);
    }

    Ok(data)
}

fn prepare_target_request_body(
    target: ProxyCaptureTarget,
    body: Vec<u8>,
    auto_include_usage: bool,
    fast_mode_rewrite_mode: ProxyFastModeRewriteMode,
) -> (Vec<u8>, RequestCaptureInfo, bool) {
    let mut info = RequestCaptureInfo {
        model: None,
        prompt_cache_key: None,
        requested_service_tier: None,
        reasoning_effort: None,
        is_stream: false,
        parse_error: None,
    };

    if body.is_empty() {
        return (body, info, false);
    }

    let mut value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => {
            info.parse_error = Some(format!("request_json_parse_error:{err}"));
            return (body, info, false);
        }
    };

    info.model = value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    info.prompt_cache_key = extract_prompt_cache_key_from_request_body(&value);
    info.reasoning_effort = extract_reasoning_effort_from_request_body(target, &value);
    info.is_stream = value
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut rewritten = if target.allows_fast_mode_rewrite() {
        rewrite_request_service_tier_for_fast_mode(&mut value, fast_mode_rewrite_mode)
    } else {
        false
    };
    if target.should_auto_include_usage()
        && info.is_stream
        && auto_include_usage
        && let Some(object) = value.as_object_mut()
    {
        let stream_options = object
            .entry("stream_options".to_string())
            .or_insert_with(|| json!({}));
        if let Some(stream_options_obj) = stream_options.as_object_mut() {
            stream_options_obj.insert("include_usage".to_string(), Value::Bool(true));
            rewritten = true;
        } else {
            object.insert(
                "stream_options".to_string(),
                json!({ "include_usage": true }),
            );
            rewritten = true;
        }
    }

    info.requested_service_tier = extract_requested_service_tier_from_request_body(&value);

    if rewritten {
        match serde_json::to_vec(&value) {
            Ok(rewritten_body) => (rewritten_body, info, true),
            Err(err) => {
                let mut fallback = info;
                fallback.parse_error = Some(format!("request_json_rewrite_error:{err}"));
                (body, fallback, false)
            }
        }
    } else {
        (body, info, false)
    }
}

fn extract_prompt_cache_key_from_request_body(value: &Value) -> Option<String> {
    const PROMPT_CACHE_KEY_POINTERS: &[&str] = &[
        "/metadata/prompt_cache_key",
        "/metadata/promptCacheKey",
        "/prompt_cache_key",
        "/promptCacheKey",
    ];

    for pointer in PROMPT_CACHE_KEY_POINTERS {
        if let Some(prompt_cache_key) = value.pointer(pointer).and_then(|v| v.as_str()) {
            let normalized = prompt_cache_key.trim();
            if !normalized.is_empty() {
                return Some(normalized.to_string());
            }
        }
    }
    None
}

fn extract_requested_service_tier_from_request_body(value: &Value) -> Option<String> {
    ["/service_tier", "/serviceTier"]
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(|entry| entry.as_str()))
        .and_then(normalize_service_tier)
}

fn rewrite_request_service_tier_for_fast_mode(
    value: &mut Value,
    fast_mode_rewrite_mode: ProxyFastModeRewriteMode,
) -> bool {
    let target_service_tier = match fast_mode_rewrite_mode {
        ProxyFastModeRewriteMode::Disabled => return false,
        ProxyFastModeRewriteMode::FillMissing => {
            extract_requested_service_tier_from_request_body(value)
                .or_else(|| Some("priority".to_string()))
        }
        ProxyFastModeRewriteMode::ForcePriority => Some("priority".to_string()),
    };

    let Some(target_service_tier) = target_service_tier else {
        return false;
    };
    let Some(object) = value.as_object_mut() else {
        return false;
    };

    let mut rewritten = object.remove("serviceTier").is_some();
    if object.get("service_tier").and_then(|entry| entry.as_str())
        != Some(target_service_tier.as_str())
    {
        object.insert(
            "service_tier".to_string(),
            Value::String(target_service_tier),
        );
        rewritten = true;
    }

    rewritten
}

fn extract_reasoning_effort_from_request_body(
    target: ProxyCaptureTarget,
    value: &Value,
) -> Option<String> {
    let raw = match target {
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact => {
            value.pointer("/reasoning/effort").and_then(|v| v.as_str())
        }
        ProxyCaptureTarget::ChatCompletions => {
            value.get("reasoning_effort").and_then(|v| v.as_str())
        }
    }?;

    let normalized = raw.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

fn parse_target_response_payload(
    _target: ProxyCaptureTarget,
    bytes: &[u8],
    request_is_stream: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    if bytes.is_empty() {
        return ResponseCaptureInfo {
            model: None,
            usage: ParsedUsage::default(),
            usage_missing_reason: Some("empty_response".to_string()),
            service_tier: None,
        };
    }

    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_parse(bytes, content_encoding);
    let parse_bytes = decoded_bytes.as_ref();
    let looks_like_stream = request_is_stream || response_payload_looks_like_sse(parse_bytes);
    let mut response_info = if looks_like_stream {
        parse_stream_response_payload(parse_bytes)
    } else {
        match serde_json::from_slice::<Value>(parse_bytes) {
            Ok(value) => {
                let model = extract_model_from_payload(&value);
                let usage = extract_usage_from_payload(&value).unwrap_or_default();
                let service_tier = extract_service_tier_from_payload(&value);
                let usage_missing_reason = if usage.total_tokens.is_none()
                    && usage.input_tokens.is_none()
                    && usage.output_tokens.is_none()
                {
                    Some("usage_missing_in_response".to_string())
                } else {
                    None
                };
                ResponseCaptureInfo {
                    model,
                    usage,
                    usage_missing_reason,
                    service_tier,
                }
            }
            Err(_) => ResponseCaptureInfo {
                model: None,
                usage: ParsedUsage::default(),
                usage_missing_reason: Some("response_not_json".to_string()),
                service_tier: None,
            },
        }
    };

    if let Some(reason) = decode_failure_reason {
        let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
            format!("response_decode_failed:{reason};{existing}")
        } else {
            format!("response_decode_failed:{reason}")
        };
        response_info.usage_missing_reason = Some(combined_reason);
    }

    response_info
}

fn response_payload_looks_like_sse(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes)
        .lines()
        .find_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                None
            } else {
                Some(
                    trimmed.starts_with("data:")
                        || trimmed.starts_with("event:")
                        || trimmed.starts_with("id:")
                        || trimmed.starts_with("retry:"),
                )
            }
        })
        .unwrap_or(false)
}

fn decode_response_payload_for_parse<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    let Some(content_encoding) = content_encoding else {
        return (Cow::Borrowed(bytes), None);
    };

    let encodings: Vec<String> = content_encoding
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect();
    if encodings.is_empty() {
        return (Cow::Borrowed(bytes), None);
    }

    let mut decoded: Cow<'a, [u8]> = Cow::Borrowed(bytes);
    for encoding in encodings.iter().rev() {
        match encoding.as_str() {
            "identity" => {}
            "gzip" | "x-gzip" => {
                let mut decoder = GzDecoder::new(decoded.as_ref());
                let mut next = Vec::new();
                match decoder.read_to_end(&mut next) {
                    Ok(_) => decoded = Cow::Owned(next),
                    Err(err) => return (Cow::Borrowed(bytes), Some(format!("gzip:{err}"))),
                }
            }
            _ => {
                return (
                    Cow::Borrowed(bytes),
                    Some(format!("unsupported_content_encoding:{encoding}")),
                );
            }
        }
    }
    (decoded, None)
}

fn parse_stream_response_payload(bytes: &[u8]) -> ResponseCaptureInfo {
    let text = String::from_utf8_lossy(bytes);
    let mut model: Option<String> = None;
    let mut usage = ParsedUsage::default();
    let mut service_tier: Option<String> = None;
    let mut usage_found = false;
    let mut parse_error_seen = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("data:") {
            continue;
        }
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        match serde_json::from_str::<Value>(payload) {
            Ok(value) => {
                if model.is_none() {
                    model = extract_model_from_payload(&value);
                }
                if service_tier.is_none() {
                    service_tier = extract_service_tier_from_payload(&value);
                }
                if let Some(parsed_usage) = extract_usage_from_payload(&value) {
                    usage = parsed_usage;
                    usage_found = true;
                }
            }
            Err(_) => {
                parse_error_seen = true;
            }
        }
    }

    ResponseCaptureInfo {
        model,
        usage,
        usage_missing_reason: if usage_found {
            None
        } else if parse_error_seen {
            Some("stream_event_parse_error".to_string())
        } else {
            Some("usage_missing_in_stream".to_string())
        },
        service_tier,
    }
}

fn decode_response_payload_for_usage<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    if !response_payload_looks_gzip(content_encoding, bytes) {
        return (Cow::Borrowed(bytes), None);
    }

    let mut decoder = GzDecoder::new(bytes);
    let mut decoded = Vec::new();
    match decoder.read_to_end(&mut decoded) {
        Ok(_) => (Cow::Owned(decoded), None),
        Err(err) => (
            Cow::Borrowed(bytes),
            Some(format!("response_gzip_decode_error:{err}")),
        ),
    }
}

fn response_payload_looks_gzip(content_encoding: Option<&str>, bytes: &[u8]) -> bool {
    if let Some(encoding) = content_encoding {
        for item in encoding.split(',') {
            if item.trim().eq_ignore_ascii_case("gzip") {
                return true;
            }
        }
    }
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

fn extract_model_from_payload(value: &Value) -> Option<String> {
    value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .pointer("/response/model")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

fn normalize_service_tier(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn extract_service_tier_from_payload(value: &Value) -> Option<String> {
    [
        "/service_tier",
        "/serviceTier",
        "/response/service_tier",
        "/response/serviceTier",
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(|v| v.as_str()))
    .and_then(normalize_service_tier)
}

fn extract_usage_from_payload(value: &Value) -> Option<ParsedUsage> {
    if let Some(usage) = value.get("usage") {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens.is_some()
            || parsed.input_tokens.is_some()
            || parsed.output_tokens.is_some()
        {
            return Some(parsed);
        }
    }
    if let Some(usage) = value.pointer("/response/usage") {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens.is_some()
            || parsed.input_tokens.is_some()
            || parsed.output_tokens.is_some()
        {
            return Some(parsed);
        }
    }
    None
}

fn parse_usage_value(value: &Value) -> ParsedUsage {
    let input_tokens = value
        .get("input_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| value.get("prompt_tokens").and_then(json_value_to_i64));
    let output_tokens = value
        .get("output_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| value.get("completion_tokens").and_then(json_value_to_i64));
    let cache_input_tokens = value
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| {
            value
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(json_value_to_i64)
        });
    let reasoning_tokens = value
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| {
            value
                .pointer("/completion_tokens_details/reasoning_tokens")
                .and_then(json_value_to_i64)
        });

    let mut parsed = ParsedUsage {
        input_tokens,
        output_tokens,
        cache_input_tokens,
        reasoning_tokens,
        total_tokens: value.get("total_tokens").and_then(json_value_to_i64),
    };

    if parsed.total_tokens.is_none() {
        parsed.total_tokens = match (parsed.input_tokens, parsed.output_tokens) {
            (Some(input), Some(output)) => Some(input + output),
            _ => None,
        };
    }

    parsed
}

fn json_value_to_i64(value: &Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    if let Some(v) = value.as_u64() {
        return i64::try_from(v).ok();
    }
    value.as_str().and_then(|v| v.parse::<i64>().ok())
}

#[allow(clippy::too_many_arguments)]
fn build_proxy_payload_summary(
    target: ProxyCaptureTarget,
    status: StatusCode,
    is_stream: bool,
    request_model: Option<&str>,
    requested_service_tier: Option<&str>,
    reasoning_effort: Option<&str>,
    response_model: Option<&str>,
    usage_missing_reason: Option<&str>,
    request_parse_error: Option<&str>,
    failure_kind: Option<&str>,
    requester_ip: Option<&str>,
    prompt_cache_key: Option<&str>,
    service_tier: Option<&str>,
    proxy_display_name: Option<&str>,
    proxy_weight_delta: Option<f64>,
) -> String {
    let payload = json!({
        "endpoint": target.endpoint(),
        "statusCode": status.as_u16(),
        "isStream": is_stream,
        "requestModel": request_model,
        "requestedServiceTier": requested_service_tier,
        "reasoningEffort": reasoning_effort,
        "responseModel": response_model,
        "usageMissingReason": usage_missing_reason,
        "requestParseError": request_parse_error,
        "failureKind": failure_kind,
        "requesterIp": requester_ip,
        "promptCacheKey": prompt_cache_key,
        "serviceTier": service_tier,
        "proxyDisplayName": proxy_display_name,
        "proxyWeightDelta": proxy_weight_delta,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn build_raw_response_preview(bytes: &[u8]) -> String {
    const PREVIEW_LIMIT: usize = 16 * 1024;
    if bytes.is_empty() {
        return "{}".to_string();
    }
    let preview = if bytes.len() > PREVIEW_LIMIT {
        &bytes[..PREVIEW_LIMIT]
    } else {
        bytes
    };
    String::from_utf8_lossy(preview).to_string()
}

fn extract_error_message_from_response(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    value
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

async fn estimate_proxy_cost_from_shared_catalog(
    catalog: &Arc<RwLock<PricingCatalog>>,
    model: Option<&str>,
    usage: &ParsedUsage,
) -> (Option<f64>, bool, Option<String>) {
    let guard = catalog.read().await;
    estimate_proxy_cost(&guard, model, usage)
}

fn has_billable_usage(usage: &ParsedUsage) -> bool {
    usage.input_tokens.unwrap_or(0).max(0) > 0
        || usage.output_tokens.unwrap_or(0).max(0) > 0
        || usage.cache_input_tokens.unwrap_or(0).max(0) > 0
        || usage.reasoning_tokens.unwrap_or(0).max(0) > 0
}

fn resolve_pricing_for_model<'a>(
    catalog: &'a PricingCatalog,
    model: &str,
) -> Option<&'a ModelPricing> {
    if let Some(pricing) = catalog.models.get(model) {
        return Some(pricing);
    }
    dated_model_alias_base(model).and_then(|base| catalog.models.get(base))
}

fn dated_model_alias_base(model: &str) -> Option<&str> {
    const DATED_SUFFIX_LEN: usize = 11; // -YYYY-MM-DD
    if model.len() <= DATED_SUFFIX_LEN {
        return None;
    }
    let suffix = &model.as_bytes()[model.len() - DATED_SUFFIX_LEN..];
    let is_dated_suffix = suffix[0] == b'-'
        && suffix[1].is_ascii_digit()
        && suffix[2].is_ascii_digit()
        && suffix[3].is_ascii_digit()
        && suffix[4].is_ascii_digit()
        && suffix[5] == b'-'
        && suffix[6].is_ascii_digit()
        && suffix[7].is_ascii_digit()
        && suffix[8] == b'-'
        && suffix[9].is_ascii_digit()
        && suffix[10].is_ascii_digit();
    if !is_dated_suffix {
        return None;
    }
    let base = &model[..model.len() - DATED_SUFFIX_LEN];
    if base.is_empty() { None } else { Some(base) }
}

fn is_gpt_5_4_long_context_surcharge_model(model: &str) -> bool {
    let base = dated_model_alias_base(model).unwrap_or(model);
    matches!(base, "gpt-5.4" | "gpt-5.4-pro")
}

fn pricing_backfill_attempt_version(catalog: &PricingCatalog) -> String {
    fn mix_fvn1a(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    }

    let mut hash = 0xcbf29ce484222325_u64;
    mix_fvn1a(&mut hash, COST_BACKFILL_ALGO_VERSION.as_bytes());
    mix_fvn1a(&mut hash, &[0xfc]);
    mix_fvn1a(&mut hash, catalog.version.as_bytes());
    mix_fvn1a(&mut hash, &[0xff]);

    let mut models = catalog.models.iter().collect::<Vec<_>>();
    models.sort_by(|(a, _), (b, _)| a.cmp(b));
    for (model, pricing) in models {
        mix_fvn1a(&mut hash, model.as_bytes());
        mix_fvn1a(&mut hash, &[0xfe]);
        mix_fvn1a(&mut hash, &pricing.input_per_1m.to_bits().to_le_bytes());
        mix_fvn1a(&mut hash, &pricing.output_per_1m.to_bits().to_le_bytes());

        match pricing.cache_input_per_1m {
            Some(value) => {
                mix_fvn1a(&mut hash, &[1]);
                mix_fvn1a(&mut hash, &value.to_bits().to_le_bytes());
            }
            None => mix_fvn1a(&mut hash, &[0]),
        }
        match pricing.reasoning_per_1m {
            Some(value) => {
                mix_fvn1a(&mut hash, &[1]);
                mix_fvn1a(&mut hash, &value.to_bits().to_le_bytes());
            }
            None => mix_fvn1a(&mut hash, &[0]),
        }
        mix_fvn1a(&mut hash, &[0xfd]);
    }

    format!("{}@{:016x}", catalog.version, hash)
}

fn estimate_proxy_cost(
    catalog: &PricingCatalog,
    model: Option<&str>,
    usage: &ParsedUsage,
) -> (Option<f64>, bool, Option<String>) {
    let price_version = Some(catalog.version.clone());
    let Some(model) = model else {
        return (None, false, price_version);
    };
    let Some(pricing) = resolve_pricing_for_model(catalog, model) else {
        return (None, false, price_version);
    };
    let input_tokens = usage.input_tokens.unwrap_or(0).max(0);
    let output_tokens = usage.output_tokens.unwrap_or(0).max(0) as f64;
    let cache_input_tokens = usage.cache_input_tokens.unwrap_or(0).max(0);
    let reasoning_tokens = usage.reasoning_tokens.unwrap_or(0).max(0) as f64;
    if !has_billable_usage(usage) {
        return (None, false, price_version);
    }

    let apply_long_context_surcharge = is_gpt_5_4_long_context_surcharge_model(model)
        && input_tokens > GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS;

    let billable_cache_tokens = if pricing.cache_input_per_1m.is_some() {
        cache_input_tokens
    } else {
        0
    };
    let non_cached_input_tokens = input_tokens.saturating_sub(billable_cache_tokens);

    let non_cached_input_cost =
        (non_cached_input_tokens as f64 / 1_000_000.0) * pricing.input_per_1m;
    let cache_input_cost = pricing
        .cache_input_per_1m
        .map(|cache_price| (billable_cache_tokens as f64 / 1_000_000.0) * cache_price)
        .unwrap_or(0.0);
    let mut input_cost = non_cached_input_cost + cache_input_cost;

    let mut output_cost = (output_tokens / 1_000_000.0) * pricing.output_per_1m;

    let mut reasoning_cost = pricing
        .reasoning_per_1m
        .map(|reasoning_price| (reasoning_tokens / 1_000_000.0) * reasoning_price)
        .unwrap_or(0.0);

    if apply_long_context_surcharge {
        input_cost *= 2.0;
        output_cost *= 1.5;
        reasoning_cost *= 1.5;
    }

    let cost = input_cost + output_cost + reasoning_cost;

    (Some(cost), true, price_version)
}

fn store_raw_payload_file(
    config: &AppConfig,
    invoke_id: &str,
    kind: &str,
    bytes: &[u8],
) -> RawPayloadMeta {
    let mut meta = RawPayloadMeta {
        path: None,
        size_bytes: bytes.len() as i64,
        truncated: false,
        truncated_reason: None,
    };

    if bytes.is_empty() {
        return meta;
    }

    let mut write_len = bytes.len();
    if let Some(limit) = config.proxy_raw_max_bytes
        && write_len > limit
    {
        write_len = limit;
        meta.truncated = true;
        meta.truncated_reason = Some("max_bytes_exceeded".to_string());
    }
    let content = &bytes[..write_len];

    let raw_dir = config.resolved_proxy_raw_dir();

    if let Err(err) = fs::create_dir_all(&raw_dir) {
        meta.truncated = true;
        meta.truncated_reason = Some(format!("write_failed:{err}"));
        return meta;
    }

    let filename = format!("{invoke_id}-{kind}.bin");
    let path = raw_dir.join(filename);
    match fs::File::create(&path).and_then(|mut f| f.write_all(content)) {
        Ok(_) => {
            meta.path = Some(path.to_string_lossy().to_string());
        }
        Err(err) => {
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{err}"));
        }
    }
    meta
}

fn compute_raw_expires_at(now_utc: DateTime<Utc>, retention: Duration) -> Option<String> {
    ChronoDuration::from_std(retention)
        .ok()
        .map(|d| format_naive((now_utc + d).naive_utc()))
}

async fn persist_and_broadcast_proxy_capture(
    state: &AppState,
    capture_started: Instant,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let inserted = persist_proxy_capture_record(&state.pool, capture_started, record).await?;
    let Some(inserted_record) = inserted else {
        return Ok(());
    };
    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let invoke_id = inserted_record.invoke_id.clone();
    if let Err(err) = state.broadcaster.send(BroadcastPayload::Records {
        records: vec![inserted_record],
    }) {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast new proxy capture record"
        );
    }

    state
        .proxy_summary_quota_broadcast_seq
        .fetch_add(1, Ordering::Relaxed);
    if state
        .proxy_summary_quota_broadcast_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Ok(());
    }

    let latest_broadcast_seq = state.proxy_summary_quota_broadcast_seq.clone();
    let broadcast_running = state.proxy_summary_quota_broadcast_running.clone();
    let pool = state.pool.clone();
    let broadcaster = state.broadcaster.clone();
    let broadcast_state_cache = state.broadcast_state_cache.clone();
    let relay_config = state.config.crs_stats.clone();
    tokio::spawn(async move {
        let mut synced_seq = 0_u64;
        loop {
            let target_seq = latest_broadcast_seq.load(Ordering::Acquire);
            if target_seq == synced_seq {
                broadcast_running.store(false, Ordering::Release);
                if latest_broadcast_seq.load(Ordering::Acquire) != synced_seq
                    && broadcast_running
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    continue;
                }
                break;
            }
            synced_seq = target_seq;

            if broadcaster.receiver_count() == 0 {
                continue;
            }

            match collect_summary_snapshots(&pool, relay_config.as_ref()).await {
                Ok(summaries) => {
                    for summary in summaries {
                        if let Err(err) = broadcast_summary_if_changed(
                            &broadcaster,
                            broadcast_state_cache.as_ref(),
                            &summary.window,
                            summary.summary,
                        )
                        .await
                        {
                            warn!(
                                ?err,
                                invoke_id = %invoke_id,
                                window = %summary.window,
                                "failed to broadcast proxy summary payload"
                            );
                        }
                    }
                }
                Err(err) => {
                    warn!(
                        ?err,
                        invoke_id = %invoke_id,
                        "failed to collect summary snapshots after proxy capture persistence"
                    );
                }
            }

            if broadcaster.receiver_count() == 0 {
                continue;
            }

            match QuotaSnapshotResponse::fetch_latest(&pool).await {
                Ok(Some(snapshot)) => {
                    if let Err(err) = broadcast_quota_if_changed(
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        snapshot,
                    )
                    .await
                    {
                        warn!(
                            ?err,
                            invoke_id = %invoke_id,
                            "failed to broadcast proxy quota snapshot"
                        );
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(
                        ?err,
                        invoke_id = %invoke_id,
                        "failed to fetch latest quota snapshot after proxy capture persistence"
                    );
                }
            }
        }
    });

    Ok(())
}

async fn persist_proxy_capture_record(
    pool: &Pool<Sqlite>,
    capture_started: Instant,
    mut record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let failure = classify_invocation_failure(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
    );
    let persist_started = Instant::now();
    let insert_result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO codex_invocations (
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
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            raw_response,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            raw_expires_at,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36
        )
        "#,
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .bind(SOURCE_PROXY)
    .bind(&record.model)
    .bind(record.usage.input_tokens)
    .bind(record.usage.output_tokens)
    .bind(record.usage.cache_input_tokens)
    .bind(record.usage.reasoning_tokens)
    .bind(record.usage.total_tokens)
    .bind(record.cost)
    .bind(record.cost_estimated as i64)
    .bind(record.price_version.as_deref())
    .bind(&record.status)
    .bind(record.error_message.as_deref())
    .bind(failure.failure_kind.as_deref())
    .bind(failure.failure_class.as_str())
    .bind(failure.is_actionable as i64)
    .bind(record.payload.as_deref())
    .bind(&record.raw_response)
    .bind(record.req_raw.path.as_deref())
    .bind(record.req_raw.size_bytes)
    .bind(record.req_raw.truncated as i64)
    .bind(record.req_raw.truncated_reason.as_deref())
    .bind(record.resp_raw.path.as_deref())
    .bind(record.resp_raw.size_bytes)
    .bind(record.resp_raw.truncated as i64)
    .bind(record.resp_raw.truncated_reason.as_deref())
    .bind(record.raw_expires_at.as_deref())
    .bind(record.timings.t_total_ms)
    .bind(record.timings.t_req_read_ms)
    .bind(record.timings.t_req_parse_ms)
    .bind(record.timings.t_upstream_connect_ms)
    .bind(record.timings.t_upstream_ttfb_ms)
    .bind(record.timings.t_upstream_stream_ms)
    .bind(record.timings.t_resp_parse_ms)
    .bind(record.timings.t_persist_ms)
    .execute(pool)
    .await?;

    let t_persist_ms = elapsed_ms(persist_started);
    let t_total_ms = elapsed_ms(capture_started);
    record.timings.t_persist_ms = t_persist_ms;
    record.timings.t_total_ms = t_total_ms;

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET t_total_ms = ?1,
            t_persist_ms = ?2
        WHERE invoke_id = ?3 AND occurred_at = ?4
        "#,
    )
    .bind(record.timings.t_total_ms)
    .bind(record.timings.t_persist_ms)
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .execute(pool)
    .await?;

    if insert_result.rows_affected() == 0 {
        return Ok(None);
    }

    let inserted = sqlx::query_as::<_, ApiInvocation>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            source,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.reasoningEffort') END AS reasoning_effort,
            total_tokens,
            cost,
            status,
            error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
            COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind,
            failure_class,
            is_actionable,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                THEN json_extract(payload, '$.requestedServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text'
                THEN json_extract(payload, '$.serviceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text'
                THEN json_extract(payload, '$.service_tier') END AS service_tier,
            CASE WHEN json_valid(payload)
              AND json_type(payload, '$.proxyWeightDelta') IN ('integer', 'real')
              THEN json_extract(payload, '$.proxyWeightDelta') END AS proxy_weight_delta,
            cost_estimated,
            price_version,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            raw_expires_at,
            detail_level,
            detail_pruned_at,
            detail_prune_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms,
            created_at
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .fetch_one(pool)
    .await?;

    Ok(Some(inserted))
}

fn read_proxy_raw_bytes(path: &str, fallback_root: Option<&Path>) -> io::Result<Vec<u8>> {
    let primary_path = Path::new(path);
    match fs::read(primary_path) {
        Ok(content) => Ok(content),
        Err(primary_err) => {
            if primary_path.is_absolute() {
                return Err(primary_err);
            }

            if let Some(root) = fallback_root {
                let fallback_path = root.join(primary_path);
                if fallback_path != primary_path
                    && let Ok(content) = fs::read(&fallback_path)
                {
                    return Ok(content);
                }
            }

            Err(primary_err)
        }
    }
}

async fn current_proxy_usage_backfill_snapshot_max_id(pool: &Pool<Sqlite>) -> Result<i64> {
    Ok(sqlx::query_scalar(
        r#"
        SELECT COALESCE(MAX(id), 0)
        FROM codex_invocations
        WHERE source = ?1
          AND status = 'success'
          AND total_tokens IS NULL
          AND response_raw_path IS NOT NULL
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(pool)
    .await?)
}

async fn backfill_proxy_usage_tokens_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    snapshot_max_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyUsageBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyUsageBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyUsageBackfillCandidate>(
            r#"
            SELECT id, response_raw_path, payload
            FROM codex_invocations
            WHERE source = ?1
              AND status = 'success'
              AND total_tokens IS NULL
              AND response_raw_path IS NOT NULL
              AND id > ?2
              AND id <= ?3
            ORDER BY id ASC
            LIMIT ?4
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(snapshot_max_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut updates = Vec::new();
        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_response =
                match read_proxy_raw_bytes(&candidate.response_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} response_raw_path={} reason=missing_file",
                                candidate.id, candidate.response_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let (target, is_stream) = parse_proxy_capture_summary(candidate.payload.as_deref());
            let (payload_for_parse, decode_error) =
                decode_response_payload_for_usage(&raw_response, None);
            let response_info =
                parse_target_response_payload(target, payload_for_parse.as_ref(), is_stream, None);
            let usage = response_info.usage;
            let has_usage = usage.total_tokens.is_some()
                || usage.input_tokens.is_some()
                || usage.output_tokens.is_some()
                || usage.cache_input_tokens.is_some()
                || usage.reasoning_tokens.is_some();
            if !has_usage {
                if decode_error.is_some() {
                    summary.skipped_decode_error += 1;
                } else {
                    summary.skipped_without_usage += 1;
                }
                continue;
            }

            updates.push(ProxyUsageBackfillUpdate {
                id: candidate.id,
                usage,
            });
        }

        if !updates.is_empty() {
            let mut tx = pool.begin().await?;
            let mut updated_this_batch = 0_u64;
            for update in updates {
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET input_tokens = ?1,
                        output_tokens = ?2,
                        cache_input_tokens = ?3,
                        reasoning_tokens = ?4,
                        total_tokens = ?5
                    WHERE id = ?6
                      AND source = ?7
                      AND total_tokens IS NULL
                    "#,
                )
                .bind(update.usage.input_tokens)
                .bind(update.usage.output_tokens)
                .bind(update.usage.cache_input_tokens)
                .bind(update.usage.reasoning_tokens)
                .bind(update.usage.total_tokens)
                .bind(update.id)
                .bind(SOURCE_PROXY)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                updated_this_batch += affected;
            }
            tx.commit().await?;
            summary.updated += updated_this_batch;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_usage_tokens(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let snapshot_max_id = current_proxy_usage_backfill_snapshot_max_id(pool).await?;
    Ok(backfill_proxy_usage_tokens_from_cursor(
        pool,
        0,
        snapshot_max_id,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
async fn backfill_proxy_usage_tokens_up_to_id(
    pool: &Pool<Sqlite>,
    snapshot_max_id: i64,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    Ok(backfill_proxy_usage_tokens_from_cursor(
        pool,
        0,
        snapshot_max_id,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
async fn run_backfill_with_retry(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let mut attempt = 1_u32;
    loop {
        match backfill_proxy_usage_tokens(pool, raw_path_fallback_root).await {
            Ok(summary) => return Ok(summary),
            Err(err)
                if attempt < BACKFILL_LOCK_RETRY_MAX_ATTEMPTS && is_sqlite_lock_error(&err) =>
            {
                warn!(
                    attempt,
                    max_attempts = BACKFILL_LOCK_RETRY_MAX_ATTEMPTS,
                    retry_delay_secs = BACKFILL_LOCK_RETRY_DELAY_SECS,
                    error = %err,
                    "proxy usage startup backfill hit sqlite lock; retrying"
                );
                attempt += 1;
                sleep(Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS)).await;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "proxy usage startup backfill failed after {attempt}/{} attempt(s)",
                        BACKFILL_LOCK_RETRY_MAX_ATTEMPTS
                    )
                });
            }
        }
    }
}

async fn current_proxy_cost_backfill_snapshot_max_id(
    pool: &Pool<Sqlite>,
    attempt_version: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar(
        r#"
        SELECT COALESCE(MAX(id), 0)
        FROM codex_invocations
        WHERE source = ?1
          AND status = 'success'
          AND cost IS NULL
          AND model IS NOT NULL
          AND (
              COALESCE(input_tokens, 0) > 0
              OR COALESCE(output_tokens, 0) > 0
              OR COALESCE(cache_input_tokens, 0) > 0
              OR COALESCE(reasoning_tokens, 0) > 0
          )
          AND (price_version IS NULL OR price_version != ?2)
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(attempt_version)
    .fetch_one(pool)
    .await?)
}

async fn backfill_proxy_missing_costs_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    snapshot_max_id: i64,
    catalog: &PricingCatalog,
    attempt_version: &str,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyCostBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyCostBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyCostBackfillCandidate>(
            r#"
            SELECT id, model, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens
            FROM codex_invocations
            WHERE source = ?1
              AND status = 'success'
              AND cost IS NULL
              AND model IS NOT NULL
              AND (
                  COALESCE(input_tokens, 0) > 0
                  OR COALESCE(output_tokens, 0) > 0
                  OR COALESCE(cache_input_tokens, 0) > 0
                  OR COALESCE(reasoning_tokens, 0) > 0
              )
              AND (price_version IS NULL OR price_version != ?4)
              AND id > ?2
              AND id <= ?3
            ORDER BY id ASC
            LIMIT ?5
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(snapshot_max_id)
        .bind(attempt_version)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut updates = Vec::new();
        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;
            let Some(model) = candidate.model.as_deref() else {
                summary.skipped_unpriced_model += 1;
                continue;
            };
            let usage = ParsedUsage {
                input_tokens: candidate.input_tokens,
                output_tokens: candidate.output_tokens,
                cache_input_tokens: candidate.cache_input_tokens,
                reasoning_tokens: candidate.reasoning_tokens,
                total_tokens: candidate.total_tokens,
            };
            if !has_billable_usage(&usage) {
                summary.skipped_unpriced_model += 1;
                continue;
            }

            let (cost, cost_estimated, price_version) =
                estimate_proxy_cost(catalog, Some(model), &usage);
            if cost.is_none() || !cost_estimated {
                summary.skipped_unpriced_model += 1;
                push_backfill_sample(
                    &mut samples,
                    format!("id={} model={} reason=unpriced_model", candidate.id, model),
                );
            }
            let persisted_price_version = if cost_estimated && cost.is_some() {
                price_version
            } else {
                Some(attempt_version.to_string())
            };
            updates.push(ProxyCostBackfillUpdate {
                id: candidate.id,
                cost,
                cost_estimated,
                price_version: persisted_price_version,
            });
        }

        if !updates.is_empty() {
            let mut tx = pool.begin().await?;
            let mut updated_this_batch = 0_u64;
            for update in updates {
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET cost = ?1,
                        cost_estimated = ?2,
                        price_version = ?3
                    WHERE id = ?4
                      AND source = ?5
                      AND cost IS NULL
                    "#,
                )
                .bind(update.cost)
                .bind(update.cost_estimated as i64)
                .bind(update.price_version.as_deref())
                .bind(update.id)
                .bind(SOURCE_PROXY)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                updated_this_batch += affected;
            }
            tx.commit().await?;
            summary.updated += updated_this_batch;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_missing_costs(
    pool: &Pool<Sqlite>,
    catalog: &PricingCatalog,
) -> Result<ProxyCostBackfillSummary> {
    let attempt_version = pricing_backfill_attempt_version(catalog);
    let snapshot_max_id =
        current_proxy_cost_backfill_snapshot_max_id(pool, &attempt_version).await?;
    Ok(backfill_proxy_missing_costs_from_cursor(
        pool,
        0,
        snapshot_max_id,
        catalog,
        &attempt_version,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
#[allow(dead_code)]
async fn backfill_proxy_missing_costs_up_to_id(
    pool: &Pool<Sqlite>,
    snapshot_max_id: i64,
    catalog: &PricingCatalog,
    attempt_version: &str,
) -> Result<ProxyCostBackfillSummary> {
    Ok(backfill_proxy_missing_costs_from_cursor(
        pool,
        0,
        snapshot_max_id,
        catalog,
        attempt_version,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
async fn run_cost_backfill_with_retry(
    pool: &Pool<Sqlite>,
    catalog: &PricingCatalog,
) -> Result<ProxyCostBackfillSummary> {
    let mut attempt = 1_u32;
    loop {
        match backfill_proxy_missing_costs(pool, catalog).await {
            Ok(summary) => return Ok(summary),
            Err(err)
                if attempt < BACKFILL_LOCK_RETRY_MAX_ATTEMPTS && is_sqlite_lock_error(&err) =>
            {
                warn!(
                    attempt,
                    max_attempts = BACKFILL_LOCK_RETRY_MAX_ATTEMPTS,
                    retry_delay_secs = BACKFILL_LOCK_RETRY_DELAY_SECS,
                    error = %err,
                    "proxy cost startup backfill hit sqlite lock; retrying"
                );
                attempt += 1;
                sleep(Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS)).await;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "proxy cost startup backfill failed after {attempt}/{} attempt(s)",
                        BACKFILL_LOCK_RETRY_MAX_ATTEMPTS
                    )
                });
            }
        }
    }
}

async fn backfill_proxy_prompt_cache_keys_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyPromptCacheKeyBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyPromptCacheKeyBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyPromptCacheKeyBackfillCandidate>(
            r#"
            SELECT id, request_raw_path
            FROM codex_invocations
            WHERE source = ?1
              AND request_raw_path IS NOT NULL
              AND id > ?2
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR json_extract(payload, '$.promptCacheKey') IS NULL
                OR TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request =
                match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} request_raw_path={} reason=missing_file",
                                candidate.id, candidate.request_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.skipped_invalid_json += 1;
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} request_raw_path={} reason=invalid_json",
                            candidate.id, candidate.request_raw_path
                        ),
                    );
                    continue;
                }
            };

            let Some(prompt_cache_key) =
                extract_prompt_cache_key_from_request_body(&request_payload)
            else {
                summary.skipped_missing_key += 1;
                continue;
            };

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = json_remove(
                    json_set(
                        CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                        '$.promptCacheKey',
                        ?1
                    ),
                    '$.codexSessionId'
                )
                WHERE id = ?2
                  AND source = ?3
                  AND request_raw_path IS NOT NULL
                  AND (
                    payload IS NULL
                    OR NOT json_valid(payload)
                    OR json_extract(payload, '$.promptCacheKey') IS NULL
                    OR TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) = ''
                  )
                "#,
            )
            .bind(prompt_cache_key)
            .bind(candidate.id)
            .bind(SOURCE_PROXY)
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_prompt_cache_keys(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyPromptCacheKeyBackfillSummary> {
    Ok(
        backfill_proxy_prompt_cache_keys_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

async fn backfill_proxy_requested_service_tiers_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyRequestedServiceTierBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyRequestedServiceTierBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyRequestedServiceTierBackfillCandidate>(
            r#"
            SELECT id, request_raw_path
            FROM codex_invocations
            WHERE source = ?1
              AND request_raw_path IS NOT NULL
              AND id > ?2
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR json_extract(payload, '$.requestedServiceTier') IS NULL
                OR TRIM(CAST(json_extract(payload, '$.requestedServiceTier') AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request =
                match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} request_raw_path={} reason=missing_file",
                                candidate.id, candidate.request_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.skipped_invalid_json += 1;
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} request_raw_path={} reason=invalid_json",
                            candidate.id, candidate.request_raw_path
                        ),
                    );
                    continue;
                }
            };

            let Some(requested_service_tier) =
                extract_requested_service_tier_from_request_body(&request_payload)
            else {
                summary.skipped_missing_tier += 1;
                continue;
            };

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = json_set(
                    CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                    '$.requestedServiceTier',
                    ?1
                )
                WHERE id = ?2
                  AND source = ?3
                  AND request_raw_path IS NOT NULL
                  AND (
                    payload IS NULL
                    OR NOT json_valid(payload)
                    OR json_extract(payload, '$.requestedServiceTier') IS NULL
                    OR TRIM(CAST(json_extract(payload, '$.requestedServiceTier') AS TEXT)) = ''
                  )
                "#,
            )
            .bind(requested_service_tier)
            .bind(candidate.id)
            .bind(SOURCE_PROXY)
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_requested_service_tiers(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyRequestedServiceTierBackfillSummary> {
    Ok(backfill_proxy_requested_service_tiers_from_cursor(
        pool,
        0,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

async fn backfill_proxy_reasoning_efforts_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyReasoningEffortBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyReasoningEffortBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyReasoningEffortBackfillCandidate>(
            r#"
            SELECT id, request_raw_path
            FROM codex_invocations
            WHERE source = ?1
              AND request_raw_path IS NOT NULL
              AND id > ?2
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR json_extract(payload, '$.reasoningEffort') IS NULL
                OR TRIM(CAST(json_extract(payload, '$.reasoningEffort') AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request =
                match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} request_raw_path={} reason=missing_file",
                                candidate.id, candidate.request_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.skipped_invalid_json += 1;
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} request_raw_path={} reason=invalid_json",
                            candidate.id, candidate.request_raw_path
                        ),
                    );
                    continue;
                }
            };

            let Some(reasoning_effort) = extract_reasoning_effort_from_request_body(
                infer_proxy_capture_target_from_payload(&request_payload),
                &request_payload,
            ) else {
                summary.skipped_missing_effort += 1;
                continue;
            };

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = json_set(
                    CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                    '$.reasoningEffort',
                    ?1
                )
                WHERE id = ?2
                  AND source = ?3
                  AND request_raw_path IS NOT NULL
                  AND (
                    payload IS NULL
                    OR NOT json_valid(payload)
                    OR json_extract(payload, '$.reasoningEffort') IS NULL
                    OR TRIM(CAST(json_extract(payload, '$.reasoningEffort') AS TEXT)) = ''
                  )
                "#,
            )
            .bind(reasoning_effort)
            .bind(candidate.id)
            .bind(SOURCE_PROXY)
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_reasoning_efforts(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyReasoningEffortBackfillSummary> {
    Ok(
        backfill_proxy_reasoning_efforts_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

fn infer_proxy_capture_target_from_payload(value: &Value) -> ProxyCaptureTarget {
    if value.get("messages").is_some() || value.get("reasoning_effort").is_some() {
        ProxyCaptureTarget::ChatCompletions
    } else if value.get("previous_response_id").is_some() {
        ProxyCaptureTarget::ResponsesCompact
    } else {
        ProxyCaptureTarget::Responses
    }
}

#[derive(Debug, FromRow)]
struct InvocationServiceTierBackfillCandidate {
    id: i64,
    source: String,
    raw_response: String,
    response_raw_path: Option<String>,
}

async fn backfill_invocation_service_tiers_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<InvocationServiceTierBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = InvocationServiceTierBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, InvocationServiceTierBackfillCandidate>(
            r#"
            SELECT id, source, raw_response, response_raw_path
            FROM codex_invocations
            WHERE id > ?1
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR COALESCE(json_extract(payload, '$.serviceTier'), json_extract(payload, '$.service_tier')) IS NULL
                OR TRIM(CAST(COALESCE(json_extract(payload, '$.serviceTier'), json_extract(payload, '$.service_tier')) AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let mut service_tier = parse_target_response_payload(
                ProxyCaptureTarget::Responses,
                candidate.raw_response.as_bytes(),
                false,
                None,
            )
            .service_tier;

            if service_tier.is_none()
                && candidate.source == SOURCE_PROXY
                && let Some(path) = candidate.response_raw_path.as_deref()
            {
                match read_proxy_raw_bytes(path, raw_path_fallback_root) {
                    Ok(bytes) => {
                        let (payload_for_parse, _) =
                            decode_response_payload_for_usage(&bytes, None);
                        service_tier = parse_target_response_payload(
                            ProxyCaptureTarget::Responses,
                            payload_for_parse.as_ref(),
                            false,
                            None,
                        )
                        .service_tier;
                    }
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} response_raw_path={} reason=missing_file",
                                candidate.id, path
                            ),
                        );
                        continue;
                    }
                }
            }

            let Some(service_tier) = service_tier else {
                summary.skipped_missing_tier += 1;
                continue;
            };

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = json_set(
                    CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                    '$.serviceTier',
                    ?1
                )
                WHERE id = ?2
                  AND (
                    payload IS NULL
                    OR NOT json_valid(payload)
                    OR COALESCE(json_extract(payload, '$.serviceTier'), json_extract(payload, '$.service_tier')) IS NULL
                    OR TRIM(CAST(COALESCE(json_extract(payload, '$.serviceTier'), json_extract(payload, '$.service_tier')) AS TEXT)) = ''
                  )
                "#,
            )
            .bind(&service_tier)
            .bind(candidate.id)
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_invocation_service_tiers(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<InvocationServiceTierBackfillSummary> {
    Ok(
        backfill_invocation_service_tiers_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

#[derive(Debug, FromRow)]
struct FailureClassificationBackfillRow {
    id: i64,
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
}

async fn backfill_failure_classification_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<FailureClassificationBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = FailureClassificationBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let rows = sqlx::query_as::<_, FailureClassificationBackfillRow>(
            r#"
            SELECT id, status, error_message, failure_kind, failure_class, is_actionable
            FROM codex_invocations
            WHERE id > ?1
              AND (
                failure_class IS NULL
                OR TRIM(COALESCE(failure_class, '')) = ''
                OR is_actionable IS NULL
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) != 'success'
                    AND TRIM(COALESCE(status, '')) != ''
                    AND TRIM(COALESCE(failure_kind, '')) = ''
                )
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) != 'success'
                    AND TRIM(COALESCE(failure_class, '')) = 'none'
                )
              )
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            break;
        }

        if let Some(last) = rows.last() {
            last_seen_id = last.id;
        }
        summary.scanned += rows.len() as u64;

        let mut tx = pool.begin().await?;
        for row in rows {
            let resolved = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );

            let existing_kind = row
                .failure_kind
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned);
            let existing_class = row
                .failure_class
                .as_deref()
                .and_then(FailureClass::from_db_str);
            let existing_actionable = row.is_actionable.map(|v| v != 0);

            let next_kind = existing_kind.clone().or(resolved.failure_kind.clone());
            let should_update = existing_class != Some(resolved.failure_class)
                || existing_actionable != Some(resolved.is_actionable)
                || existing_kind != next_kind;

            if !should_update {
                continue;
            }

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET failure_kind = ?1,
                    failure_class = ?2,
                    is_actionable = ?3
                WHERE id = ?4
                "#,
            )
            .bind(next_kind.as_deref())
            .bind(resolved.failure_class.as_str())
            .bind(resolved.is_actionable as i64)
            .bind(row.id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
        tx.commit().await?;
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples: Vec::new(),
    })
}

#[cfg(test)]
#[allow(dead_code)]
async fn backfill_failure_classification(
    pool: &Pool<Sqlite>,
) -> Result<FailureClassificationBackfillSummary> {
    Ok(
        backfill_failure_classification_from_cursor(pool, 0, None, None)
            .await?
            .summary,
    )
}

#[cfg(test)]
fn is_sqlite_lock_error(err: &anyhow::Error) -> bool {
    if err.chain().any(|cause| {
        let Some(sqlx_err) = cause.downcast_ref::<sqlx::Error>() else {
            return false;
        };
        let sqlx::Error::Database(db_err) = sqlx_err else {
            return false;
        };
        matches!(
            db_err.code().as_deref(),
            Some("5") | Some("6") | Some("SQLITE_BUSY") | Some("SQLITE_LOCKED")
        )
    }) {
        return true;
    }

    err.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("database is locked")
            || message.contains("database table is locked")
            || message.contains("sqlite_busy")
            || message.contains("sqlite_locked")
            || message.contains("(code: 5)")
            || message.contains("(code: 6)")
    })
}

fn parse_proxy_capture_summary(payload: Option<&str>) -> (ProxyCaptureTarget, bool) {
    let mut target = ProxyCaptureTarget::Responses;
    let mut is_stream = false;

    let Some(raw) = payload else {
        return (target, is_stream);
    };
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return (target, is_stream);
    };

    if let Some(endpoint) = value.get("endpoint").and_then(|v| v.as_str()) {
        target = ProxyCaptureTarget::from_endpoint(endpoint);
    }
    if let Some(stream) = value.get("isStream").and_then(|v| v.as_bool()) {
        is_stream = stream;
    }

    (target, is_stream)
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn percentile_sorted_f64(sorted_values: &[f64], p: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    if sorted_values.len() == 1 {
        return sorted_values[0];
    }
    let clamped = p.clamp(0.0, 1.0);
    let rank = clamped * (sorted_values.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return sorted_values[lower];
    }
    let weight = rank - lower as f64;
    sorted_values[lower] + (sorted_values[upper] - sorted_values[lower]) * weight
}

fn next_proxy_request_id() -> u64 {
    NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

fn is_body_too_large_error(err: &reqwest::Error) -> bool {
    if error_chain_contains(err, "length limit exceeded")
        || error_chain_contains(err, PROXY_REQUEST_BODY_LIMIT_EXCEEDED)
    {
        return true;
    }
    false
}

fn error_chain_contains(err: &(dyn StdError + 'static), needle: &str) -> bool {
    if err.to_string().contains(needle) {
        return true;
    }
    let mut source = err.source();
    while let Some(inner) = source {
        if inner.to_string().contains(needle) {
            return true;
        }
        source = inner.source();
    }
    false
}

fn build_proxy_upstream_url(base: &Url, original_uri: &Uri) -> Result<Url> {
    if path_has_forbidden_dot_segment(original_uri.path()) {
        bail!(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED);
    }
    if has_invalid_percent_encoding(original_uri.path())
        || original_uri
            .query()
            .is_some_and(has_invalid_percent_encoding)
    {
        bail!(PROXY_INVALID_REQUEST_TARGET);
    }

    let host = base
        .host_str()
        .ok_or_else(|| anyhow!("OPENAI_UPSTREAM_BASE_URL is missing host"))?;
    let mut target = String::new();
    target.push_str(base.scheme());
    target.push_str("://");
    if !base.username().is_empty() {
        target.push_str(base.username());
        if let Some(password) = base.password() {
            target.push(':');
            target.push_str(password);
        }
        target.push('@');
    }
    if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        target.push('[');
        target.push_str(host);
        target.push(']');
    } else {
        target.push_str(host);
    }
    if let Some(port) = base.port() {
        target.push(':');
        target.push_str(&port.to_string());
    }

    let base_path = if base.path() == "/" {
        ""
    } else {
        base.path().trim_end_matches('/')
    };
    target.push_str(base_path);
    let request_path = original_uri.path();
    if !request_path.starts_with('/') {
        target.push('/');
    }
    target.push_str(request_path);
    if let Some(query) = original_uri.query() {
        target.push('?');
        target.push_str(query);
    }

    Url::parse(&target).context("failed to parse proxy upstream url")
}

fn path_has_forbidden_dot_segment(path: &str) -> bool {
    let mut candidate = path.to_string();
    for _ in 0..3 {
        if decoded_path_has_forbidden_dot_segment(&candidate) {
            return true;
        }
        let decoded = percent_decode_once_lossy(&candidate);
        if decoded == candidate {
            break;
        }
        candidate = decoded;
    }
    decoded_path_has_forbidden_dot_segment(&candidate)
}

fn has_invalid_percent_encoding(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            if idx + 2 >= bytes.len()
                || decode_hex_nibble(bytes[idx + 1]).is_none()
                || decode_hex_nibble(bytes[idx + 2]).is_none()
            {
                return true;
            }
            idx += 3;
            continue;
        }
        idx += 1;
    }
    false
}

fn decoded_path_has_forbidden_dot_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(is_forbidden_dot_segment)
}

fn is_forbidden_dot_segment(segment: &str) -> bool {
    segment == "." || segment == ".."
}

fn percent_decode_once_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%'
            && idx + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (
                decode_hex_nibble(bytes[idx + 1]),
                decode_hex_nibble(bytes[idx + 2]),
            )
        {
            decoded.push((hi << 4) | lo);
            idx += 3;
            continue;
        }
        decoded.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn connection_scoped_header_names(headers: &HeaderMap) -> HashSet<HeaderName> {
    let mut names = HashSet::new();
    for value in headers.get_all(header::CONNECTION).iter() {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        for token in raw.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if let Ok(header_name) = HeaderName::from_bytes(token.as_bytes()) {
                names.insert(header_name);
            }
        }
    }
    names
}

fn should_forward_proxy_header(name: &HeaderName, connection_scoped: &HashSet<HeaderName>) -> bool {
    should_proxy_header(name) && !connection_scoped.contains(name)
}

fn request_may_have_body(method: &Method, headers: &HeaderMap) -> bool {
    if headers.contains_key(header::TRANSFER_ENCODING) {
        return true;
    }
    if let Some(content_length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        return content_length > 0;
    }
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn normalize_proxy_location_header(
    status: StatusCode,
    headers: &HeaderMap,
    upstream_base: &Url,
) -> Result<Option<String>> {
    let Some(raw_location) = headers.get(header::LOCATION) else {
        return Ok(None);
    };

    let raw_location = raw_location
        .to_str()
        .context("upstream Location header is not valid UTF-8")?;
    if raw_location.is_empty() {
        return Ok(None);
    }

    if !status.is_redirection() {
        return Ok(Some(raw_location.to_string()));
    }

    if raw_location.starts_with("//") {
        bail!("cross-origin redirect is not allowed");
    }

    if let Ok(parsed) = Url::parse(raw_location) {
        if !is_same_origin(&parsed, upstream_base) {
            bail!("cross-origin redirect is not allowed");
        }
        let mut normalized = rewrite_proxy_location_path(parsed.path(), upstream_base).to_string();
        if let Some(query) = parsed.query() {
            normalized.push('?');
            normalized.push_str(query);
        }
        if let Some(fragment) = parsed.fragment() {
            normalized.push('#');
            normalized.push_str(fragment);
        }
        return Ok(Some(normalized));
    }

    if raw_location.starts_with('/') {
        return Ok(Some(rewrite_proxy_relative_location(
            raw_location,
            upstream_base,
        )));
    }

    Ok(Some(raw_location.to_string()))
}

fn rewrite_proxy_relative_location(location: &str, upstream_base: &Url) -> String {
    let (path_and_query, fragment) = match location.split_once('#') {
        Some((pq, frag)) => (pq, Some(frag)),
        None => (location, None),
    };
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_query, None),
    };

    let mut rewritten = rewrite_proxy_location_path(path, upstream_base);
    if let Some(query) = query {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    if let Some(fragment) = fragment {
        rewritten.push('#');
        rewritten.push_str(fragment);
    }
    rewritten
}

fn rewrite_proxy_location_path(upstream_path: &str, upstream_base: &Url) -> String {
    let base_path = upstream_base.path().trim_end_matches('/');
    if base_path.is_empty() || base_path == "/" {
        return upstream_path.to_string();
    }
    if upstream_path == base_path {
        return "/".to_string();
    }
    if let Some(stripped) = upstream_path.strip_prefix(base_path)
        && stripped.starts_with('/')
    {
        return stripped.to_string();
    }
    upstream_path.to_string()
}

fn is_same_origin(lhs: &Url, rhs: &Url) -> bool {
    lhs.scheme() == rhs.scheme()
        && lhs.host_str() == rhs.host_str()
        && effective_port(lhs) == effective_port(rhs)
}

fn effective_port(url: &Url) -> Option<u16> {
    url.port_or_known_default()
}

fn should_proxy_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "host"
            | "connection"
            | "proxy-connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "accept-encoding"
            | "upgrade"
            | "forwarded"
            | "via"
            | "x-real-ip"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
            | "x-forwarded-port"
            | "x-forwarded-client-cert"
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionResponse {
    backend: String,
    frontend: String,
}

async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let proxy = state.proxy_model_settings.read().await.clone();
    let pricing = state.pricing_catalog.read().await.clone();
    let forward_proxy = build_forward_proxy_settings_response(state.as_ref()).await?;
    Ok(Json(SettingsResponse {
        proxy: proxy.into(),
        forward_proxy,
        pricing: PricingSettingsResponse::from_catalog(&pricing),
    }))
}

async fn removed_proxy_model_settings_endpoint() -> (StatusCode, &'static str) {
    (
        StatusCode::NOT_FOUND,
        "endpoint removed; use /api/settings and /api/settings/proxy",
    )
}

async fn put_proxy_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ProxyModelSettingsUpdateRequest>,
) -> Result<Json<ProxyModelSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let next = ProxyModelSettings {
        hijack_enabled: payload.hijack_enabled,
        merge_upstream_enabled: payload.merge_upstream_enabled,
        fast_mode_rewrite_mode: payload.fast_mode_rewrite_mode,
        enabled_preset_models: payload.enabled_models,
    }
    .normalized();
    let _update_guard = state.proxy_model_settings_update_lock.lock().await;
    save_proxy_model_settings(&state.pool, next.clone())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let mut guard = state.proxy_model_settings.write().await;
    *guard = next.clone();
    Ok(Json(next.into()))
}

async fn put_forward_proxy_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ForwardProxySettingsUpdateRequest>,
) -> Result<Json<ForwardProxySettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let next: ForwardProxySettings = payload.into();
    let _update_guard = state.forward_proxy_settings_update_lock.lock().await;
    save_forward_proxy_settings(&state.pool, next.clone())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let (known_subscription_keys_before_settings, added_manual_endpoints) = {
        let mut manager = state.forward_proxy.lock().await;
        let before = snapshot_active_forward_proxy_endpoints(&manager);
        manager.apply_settings(next);
        let after = snapshot_active_forward_proxy_endpoints(&manager);
        (
            before
                .iter()
                .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
                .map(|endpoint| endpoint.key.clone())
                .collect::<HashSet<_>>(),
            compute_added_forward_proxy_endpoints(&before, &after),
        )
    };
    if let Err(err) = sync_forward_proxy_routes(state.as_ref()).await {
        warn!(
            error = %err,
            "failed to sync forward proxy routes after settings update"
        );
    }
    if let Err(err) = refresh_forward_proxy_subscriptions(
        state.clone(),
        true,
        Some(known_subscription_keys_before_settings),
    )
    .await
    {
        warn!(error = %err, "failed to refresh forward proxy subscriptions after settings update");
    }
    if !added_manual_endpoints.is_empty() {
        spawn_forward_proxy_bootstrap_probe_round(
            state.clone(),
            added_manual_endpoints,
            "settings-update",
        );
    }

    let response = build_forward_proxy_settings_response(state.as_ref())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(response))
}

async fn post_forward_proxy_candidate_validation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ForwardProxyCandidateValidationRequest>,
) -> Result<Json<ForwardProxyCandidateValidationResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let result = match payload.kind {
        ForwardProxyValidationKind::ProxyUrl => {
            validate_single_forward_proxy_candidate(state.as_ref(), payload.value).await
        }
        ForwardProxyValidationKind::SubscriptionUrl => {
            validate_subscription_candidate(state.as_ref(), payload.value).await
        }
    };

    let response = match result {
        Ok(response) => response,
        Err(err) => {
            warn!(error = %err, "forward proxy candidate validation failed");
            ForwardProxyCandidateValidationResponse::failed(err.to_string())
        }
    };

    Ok(Json(response))
}

async fn validate_single_forward_proxy_candidate(
    state: &AppState,
    value: String,
) -> Result<ForwardProxyCandidateValidationResponse> {
    let parsed = parse_forward_proxy_entry(value.trim())
        .ok_or_else(|| anyhow!("unsupported proxy url or unsupported scheme"))?;
    let endpoint = ForwardProxyEndpoint {
        key: format!(
            "__validate_proxy__{:016x}",
            stable_hash_u64(&parsed.normalized)
        ),
        source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
        display_name: parsed.display_name,
        protocol: parsed.protocol,
        endpoint_url: parsed.endpoint_url,
        raw_url: Some(parsed.normalized.clone()),
    };
    let latency_ms = probe_forward_proxy_endpoint(
        state,
        &endpoint,
        forward_proxy_validation_timeout(ForwardProxyValidationKind::ProxyUrl),
    )
    .await?;
    Ok(ForwardProxyCandidateValidationResponse::success(
        "proxy validation succeeded",
        Some(parsed.normalized),
        Some(1),
        Some(latency_ms),
    ))
}

async fn validate_subscription_candidate(
    state: &AppState,
    value: String,
) -> Result<ForwardProxyCandidateValidationResponse> {
    let validation_timeout =
        forward_proxy_validation_timeout(ForwardProxyValidationKind::SubscriptionUrl);
    let validation_started = Instant::now();
    let normalized_subscription = normalize_subscription_entries(vec![value])
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("subscription url must be a valid http/https url"))?;
    let urls = fetch_subscription_proxy_urls_with_validation_budget(
        &state.http_clients.shared,
        &normalized_subscription,
        validation_timeout,
        validation_started,
    )
    .await
    .context("failed to fetch or decode subscription payload")?;
    if urls.is_empty() {
        bail!("subscription resolved zero proxy entries");
    }
    let endpoints = normalize_proxy_endpoints_from_urls(&urls, FORWARD_PROXY_SOURCE_SUBSCRIPTION);
    if endpoints.is_empty() {
        bail!("subscription contains no supported proxy entries");
    }

    let mut last_error: Option<anyhow::Error> = None;
    let mut best_latency_ms: Option<f64> = None;
    for endpoint in endpoints.iter().take(3) {
        let Some(remaining_timeout) =
            remaining_timeout_budget(validation_timeout, validation_started.elapsed())
        else {
            last_error = Some(timeout_error_for_duration(validation_timeout));
            break;
        };
        if remaining_timeout.is_zero() {
            last_error = Some(timeout_error_for_duration(validation_timeout));
            break;
        }

        match probe_forward_proxy_endpoint(state, endpoint, remaining_timeout).await {
            Ok(latency_ms) => {
                best_latency_ms = Some(latency_ms);
                break;
            }
            Err(err) => {
                if timeout_budget_exhausted(validation_timeout, validation_started.elapsed()) {
                    last_error = Some(timeout_error_for_duration(validation_timeout));
                    break;
                }
                last_error = Some(err);
            }
        }
    }

    let Some(latency_ms) = best_latency_ms else {
        if let Some(err) = last_error {
            return Err(anyhow!(
                "subscription proxy probe failed: {err}; no entry passed validation"
            ));
        }
        bail!("no subscription proxy entry passed validation");
    };

    Ok(ForwardProxyCandidateValidationResponse::success(
        "subscription validation succeeded",
        Some(normalized_subscription),
        Some(endpoints.len()),
        Some(latency_ms),
    ))
}

async fn probe_forward_proxy_endpoint(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    validation_timeout: Duration,
) -> Result<f64> {
    let probe_target = state
        .config
        .openai_upstream_base_url
        .join("v1/models")
        .context("failed to build validation probe target")?;
    let started = Instant::now();
    let (endpoint_url, temporary_xray_key) =
        resolve_forward_proxy_probe_endpoint_url(state, endpoint, validation_timeout).await?;

    let probe_result = async {
        let send_timeout = remaining_timeout_budget(validation_timeout, started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| timeout_error_for_duration(validation_timeout))?;
        let client = state
            .http_clients
            .client_for_forward_proxy(endpoint_url.as_ref())?;
        let response = timeout(send_timeout, client.get(probe_target).send())
            .await
            .map_err(|_| timeout_error_for_duration(validation_timeout))?
            .context("validation request failed")?;
        let status = response.status();
        // Validation only needs to prove the route is reachable; auth/404 still count as reachable.
        if !is_validation_probe_reachable_status(status) {
            bail!("validation probe returned status {}", status);
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    if let Some(temp_key) = temporary_xray_key {
        let mut supervisor = state.xray_supervisor.lock().await;
        supervisor.remove_instance(&temp_key).await;
    }

    probe_result?;
    Ok(elapsed_ms(started))
}

fn is_validation_probe_reachable_status(status: StatusCode) -> bool {
    status.is_success()
        || status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || status == StatusCode::NOT_FOUND
}

fn forward_proxy_validation_timeout(kind: ForwardProxyValidationKind) -> Duration {
    match kind {
        ForwardProxyValidationKind::ProxyUrl => {
            Duration::from_secs(FORWARD_PROXY_VALIDATION_TIMEOUT_SECS)
        }
        ForwardProxyValidationKind::SubscriptionUrl => {
            Duration::from_secs(FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_SECS)
        }
    }
}

fn remaining_timeout_budget(total_timeout: Duration, elapsed: Duration) -> Option<Duration> {
    total_timeout.checked_sub(elapsed)
}

fn timeout_budget_exhausted(total_timeout: Duration, elapsed: Duration) -> bool {
    match remaining_timeout_budget(total_timeout, elapsed) {
        Some(remaining) => remaining.is_zero(),
        None => true,
    }
}

fn timeout_error_for_duration(timeout: Duration) -> anyhow::Error {
    anyhow!(
        "validation request timed out after {}s",
        timeout_seconds_for_message(timeout)
    )
}

fn timeout_seconds_for_message(timeout: Duration) -> u64 {
    let secs = timeout.as_secs();
    if timeout.subsec_nanos() > 0 {
        secs.saturating_add(1).max(1)
    } else {
        secs.max(1)
    }
}

async fn resolve_forward_proxy_probe_endpoint_url(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    validation_timeout: Duration,
) -> Result<(Option<Url>, Option<String>)> {
    if !endpoint.requires_xray() {
        return Ok((endpoint.endpoint_url.clone(), None));
    }
    let raw_url = endpoint
        .raw_url
        .as_deref()
        .ok_or_else(|| anyhow!("xray proxy validation requires raw proxy url"))?;
    let temporary_key = format!(
        "__validate_xray__{:016x}_{}",
        stable_hash_u64(raw_url),
        Utc::now().timestamp_millis()
    );
    let probe_endpoint = ForwardProxyEndpoint {
        key: temporary_key.clone(),
        source: endpoint.source.clone(),
        display_name: endpoint.display_name.clone(),
        protocol: endpoint.protocol,
        endpoint_url: None,
        raw_url: Some(raw_url.to_string()),
    };
    let route_url = {
        let mut supervisor = state.xray_supervisor.lock().await;
        supervisor
            .ensure_instance_with_ready_timeout(&probe_endpoint, validation_timeout)
            .await?
    };
    Ok((Some(route_url), Some(temporary_key)))
}

async fn put_pricing_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<PricingSettingsUpdateRequest>,
) -> Result<Json<PricingSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let next = payload.normalized()?;
    let _update_guard = state.pricing_settings_update_lock.lock().await;
    save_pricing_catalog(&state.pool, &next)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let mut guard = state.pricing_catalog.write().await;
    *guard = next.clone();
    Ok(Json(PricingSettingsResponse::from_catalog(&next)))
}

async fn get_versions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<VersionResponse>, ApiError> {
    let (backend, frontend) = detect_versions(state.config.static_dir.as_deref());
    Ok(Json(VersionResponse { backend, frontend }))
}

fn build_cors_layer(config: &AppConfig) -> CorsLayer {
    let allowed = config
        .cors_allowed_origins
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let allow_origin = AllowOrigin::predicate(move |origin, _request| {
        let Ok(origin_raw) = origin.to_str() else {
            return false;
        };
        origin_allowed(origin_raw, &allowed)
    });
    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods(Any)
        .allow_headers(Any)
}

fn origin_allowed(origin_raw: &str, configured: &HashSet<String>) -> bool {
    let Some(origin) = normalize_cors_origin(origin_raw) else {
        return false;
    };
    if configured.contains(&origin) {
        return true;
    }
    is_loopback_origin(origin_raw)
}

fn is_loopback_origin(origin_raw: &str) -> bool {
    let Ok(origin) = Url::parse(origin_raw) else {
        return false;
    };
    if !matches!(origin.scheme(), "http" | "https") {
        return false;
    }
    origin
        .host_str()
        .map(is_loopback_authority_host)
        .unwrap_or(false)
}

fn parse_cors_allowed_origins_env(name: &str) -> Result<Vec<String>> {
    match env::var(name) {
        Ok(raw) => parse_cors_allowed_origins(&raw),
        Err(env::VarError::NotPresent) => Ok(Vec::new()),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

fn parse_cors_allowed_origins(raw: &str) -> Result<Vec<String>> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for candidate in raw.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        let normalized = normalize_cors_origin(candidate)
            .ok_or_else(|| anyhow!("invalid {ENV_CORS_ALLOWED_ORIGINS} entry: {candidate}"))?;
        if seen.insert(normalized.clone()) {
            entries.push(normalized);
        }
    }
    Ok(entries)
}

fn normalize_cors_origin(origin_raw: &str) -> Option<String> {
    let origin = Url::parse(origin_raw).ok()?;
    if !matches!(origin.scheme(), "http" | "https") {
        return None;
    }
    if origin.cannot_be_a_base()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return None;
    }
    if origin.path() != "/" {
        return None;
    }

    let host = origin.host_str()?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_ascii_lowercase()
    };
    let scheme = origin.scheme().to_ascii_lowercase();
    let port = origin.port();
    let default_port = default_port_for_scheme(&scheme);

    if port.is_none() || port == default_port {
        Some(format!("{scheme}://{host}"))
    } else {
        Some(format!("{scheme}://{host}:{}", port?))
    }
}

fn is_models_list_path(path: &str) -> bool {
    path == "/v1/models"
}

// Browser-side CSRF mitigation for settings writes.
//
// This is intentionally not a full authentication mechanism: non-browser clients
// (CLI/automation) may omit Origin and are allowed by policy. The security boundary
// is deployment-level network isolation (trusted gateway only), documented in
// docs/deployment.md.
fn is_same_origin_settings_write(headers: &HeaderMap) -> bool {
    if matches!(
        header_value_as_str(headers, "sec-fetch-site"),
        Some(site)
            if site.eq_ignore_ascii_case("cross-site")
    ) {
        return false;
    }

    let Some(origin_raw) = headers.get(header::ORIGIN) else {
        // Non-browser clients may omit Origin (for example curl or internal tooling).
        // We only treat explicit browser cross-site signals as forbidden above.
        return true;
    };
    let Ok(origin) = origin_raw.to_str() else {
        return false;
    };
    let Ok(origin_url) = Url::parse(origin) else {
        return false;
    };
    if !matches!(origin_url.scheme(), "http" | "https") {
        return false;
    }

    let Some(origin_host) = origin_url.host_str() else {
        return false;
    };
    let Some((request_host, request_port)) =
        forwarded_or_host_authority(headers, origin_url.scheme())
    else {
        return false;
    };

    let origin_port = origin_url.port_or_known_default();
    if origin_host.eq_ignore_ascii_case(&request_host) && origin_port == request_port {
        return true;
    }

    // Dev loopback proxies (for example Vite on 60080 -> backend on 8080) may rewrite Host and/or port,
    // but both ends remain loopback. Allow that local-only mismatch.
    //
    // For non-loopback deployments behind reverse proxies, we accept trusted forwarded
    // host/proto/port headers for origin matching, but these headers are never relayed
    // to upstream/downstream proxy traffic (see should_proxy_header).
    is_loopback_authority_host(origin_host) && is_loopback_authority_host(&request_host)
}

fn forwarded_or_host_authority(
    headers: &HeaderMap,
    origin_scheme: &str,
) -> Option<(String, Option<u16>)> {
    if let Some(forwarded_host_raw) = header_value_as_str(headers, "x-forwarded-host") {
        // This service expects a single trusted edge gateway. If forwarded headers
        // arrive as a chain, treat it as unsupported/misconfigured and reject writes.
        let forwarded_host = single_forwarded_header_value(forwarded_host_raw)?;
        let authority = Authority::from_str(forwarded_host).ok()?;
        let forwarded_proto = match header_value_as_str(headers, "x-forwarded-proto") {
            Some(raw) => {
                let proto = single_forwarded_header_value(raw)?.to_ascii_lowercase();
                if proto == "http" || proto == "https" {
                    Some(proto)
                } else {
                    return None;
                }
            }
            None => None,
        };
        let scheme = forwarded_proto.as_deref().unwrap_or(origin_scheme);
        let forwarded_port = match header_value_as_str(headers, "x-forwarded-port") {
            Some(raw) => {
                let value = single_forwarded_header_value(raw)?;
                Some(value.parse::<u16>().ok()?)
            }
            None => None,
        };
        let port = authority
            .port_u16()
            .or(forwarded_port)
            .or_else(|| default_port_for_scheme(scheme));
        return Some((authority.host().to_string(), port));
    }

    let host_raw = headers.get(header::HOST)?;
    let host_value = host_raw.to_str().ok()?;
    let authority = Authority::from_str(host_value).ok()?;
    Some((
        authority.host().to_string(),
        authority
            .port_u16()
            .or_else(|| default_port_for_scheme(origin_scheme)),
    ))
}

fn single_forwarded_header_value(raw: &str) -> Option<&str> {
    let mut parts = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let first = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(first)
}

fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

fn header_value_as_str<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<&'a str> {
    headers
        .get(HeaderName::from_static(name))
        .and_then(|value| value.to_str().ok())
}

fn extract_requester_ip(headers: &HeaderMap, peer_ip: Option<IpAddr>) -> Option<String> {
    if let Some(x_forwarded_for) = header_value_as_str(headers, "x-forwarded-for")
        && let Some(ip) = extract_first_ip_from_x_forwarded_for(x_forwarded_for)
    {
        return Some(ip);
    }

    if let Some(x_real_ip) = header_value_as_str(headers, "x-real-ip")
        && let Some(ip) = extract_ip_from_header_value(x_real_ip)
    {
        return Some(ip);
    }

    if let Some(forwarded) = header_value_as_str(headers, "forwarded")
        && let Some(ip) = extract_ip_from_forwarded_header(forwarded)
    {
        return Some(ip);
    }

    peer_ip.map(|ip| ip.to_string())
}

fn extract_prompt_cache_key_from_headers(headers: &HeaderMap) -> Option<String> {
    for header_name in [
        "x-prompt-cache-key",
        "prompt-cache-key",
        "x-openai-prompt-cache-key",
    ] {
        if let Some(raw_value) = header_value_as_str(headers, header_name) {
            let candidate = raw_value
                .split(',')
                .next()
                .map(str::trim)
                .unwrap_or(raw_value.trim())
                .trim_matches('"');
            if !candidate.is_empty() {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

fn extract_first_ip_from_x_forwarded_for(raw: &str) -> Option<String> {
    let first = raw.split(',').next()?.trim();
    extract_ip_from_header_value(first)
}

fn extract_ip_from_forwarded_header(raw: &str) -> Option<String> {
    for entry in raw.split(',') {
        for segment in entry.split(';') {
            let pair = segment.trim();
            if pair.len() >= 4 && pair[..4].eq_ignore_ascii_case("for=") {
                let value = &pair[4..];
                if let Some(ip) = extract_ip_from_header_value(value) {
                    return Some(ip);
                }
            }
        }
    }
    None
}

fn extract_ip_from_header_value(raw: &str) -> Option<String> {
    let normalized = raw.trim().trim_matches('"');
    if normalized.is_empty()
        || normalized.eq_ignore_ascii_case("unknown")
        || normalized.starts_with('_')
    {
        return None;
    }

    if let Some(value) = normalized.strip_prefix("for=") {
        return extract_ip_from_header_value(value);
    }

    if normalized.starts_with('[')
        && let Some(end) = normalized.find(']')
        && let Ok(ip) = normalized[1..end].parse::<IpAddr>()
    {
        return Some(ip.to_string());
    }

    if let Ok(ip) = normalized.parse::<IpAddr>() {
        return Some(ip.to_string());
    }

    if let Some((host, port)) = normalized.rsplit_once(':')
        && !host.contains(':')
        && port.parse::<u16>().is_ok()
        && let Ok(ip) = host.parse::<IpAddr>()
    {
        return Some(ip.to_string());
    }

    None
}

fn is_loopback_authority_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn build_preset_models_payload(enabled_model_ids: &[String]) -> Value {
    let data = enabled_model_ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "object": "model",
                "owned_by": "proxy",
                "created": 0
            })
        })
        .collect::<Vec<_>>();
    json!({
        "object": "list",
        "data": data
    })
}

fn merge_models_payload_with_upstream(
    upstream_payload: &Value,
    enabled_model_ids: &[String],
) -> Result<Value> {
    let upstream_items = upstream_payload
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("upstream models payload missing data array"))?;
    let mut merged = build_preset_models_payload(enabled_model_ids)
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut seen_ids: HashSet<String> = enabled_model_ids.iter().cloned().collect();

    for item in upstream_items {
        if let Some(id) = item.get("id").and_then(|v| v.as_str())
            && seen_ids.insert(id.to_string())
        {
            merged.push(item.clone());
        }
    }

    Ok(json!({
        "object": "list",
        "data": merged
    }))
}

fn snapshot_active_forward_proxy_endpoints(
    manager: &ForwardProxyManager,
) -> Vec<ForwardProxyEndpoint> {
    manager
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.protocol != ForwardProxyProtocol::Direct)
        .filter(|endpoint| endpoint.endpoint_url.is_some() || endpoint.requires_xray())
        .cloned()
        .collect()
}

fn compute_added_forward_proxy_endpoints(
    before: &[ForwardProxyEndpoint],
    after: &[ForwardProxyEndpoint],
) -> Vec<ForwardProxyEndpoint> {
    let known = before
        .iter()
        .map(|endpoint| endpoint.key.as_str())
        .collect::<HashSet<_>>();
    after
        .iter()
        .filter(|endpoint| !known.contains(endpoint.key.as_str()))
        .cloned()
        .collect()
}

fn snapshot_known_subscription_proxy_keys(manager: &ForwardProxyManager) -> HashSet<String> {
    manager
        .runtime
        .values()
        .filter(|entry| entry.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
        .map(|entry| entry.proxy_key.clone())
        .collect()
}

fn classify_bootstrap_forward_proxy_probe_failure(err: &anyhow::Error) -> &'static str {
    let message = err.to_string().to_ascii_lowercase();
    if message.contains("timed out") || message.contains("timeout") {
        return FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT;
    }
    if message.contains("validation probe returned status 5") {
        return FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX;
    }
    FORWARD_PROXY_FAILURE_SEND_ERROR
}

fn spawn_forward_proxy_bootstrap_probe_round(
    state: Arc<AppState>,
    added_endpoints: Vec<ForwardProxyEndpoint>,
    trigger: &'static str,
) {
    if added_endpoints.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let validation_timeout =
            forward_proxy_validation_timeout(ForwardProxyValidationKind::ProxyUrl);
        info!(
            trigger,
            added_count = added_endpoints.len(),
            timeout_secs = validation_timeout.as_secs(),
            "forward proxy bootstrap probe round started"
        );
        for endpoint in added_endpoints {
            let selected_proxy = SelectedForwardProxy::from_endpoint(&endpoint);
            let started = Instant::now();
            match probe_forward_proxy_endpoint(state.as_ref(), &endpoint, validation_timeout).await
            {
                Ok(latency_ms) => {
                    record_forward_proxy_attempt(
                        state.clone(),
                        selected_proxy,
                        true,
                        Some(latency_ms),
                        None,
                        true,
                    )
                    .await;
                }
                Err(err) => {
                    let failure_kind = classify_bootstrap_forward_proxy_probe_failure(&err);
                    warn!(
                        trigger,
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        proxy_source = endpoint.source,
                        proxy_label = endpoint.display_name,
                        proxy_url_ref = %forward_proxy_log_ref_option(endpoint.raw_url.as_deref()),
                        failure_kind,
                        error = %err,
                        "forward proxy bootstrap probe failed"
                    );
                    record_forward_proxy_attempt(
                        state.clone(),
                        selected_proxy,
                        false,
                        Some(elapsed_ms(started)),
                        Some(failure_kind),
                        true,
                    )
                    .await;
                }
            }
        }
        info!(trigger, "forward proxy bootstrap probe round finished");
    });
}

async fn refresh_forward_proxy_subscriptions(
    state: Arc<AppState>,
    force: bool,
    known_subscription_keys_override: Option<HashSet<String>>,
) -> Result<()> {
    let (subscription_urls, interval_secs, last_refresh_at) = {
        let manager = state.forward_proxy.lock().await;
        (
            manager.settings.subscription_urls.clone(),
            manager.settings.subscription_update_interval_secs,
            manager.last_subscription_refresh_at,
        )
    };

    if !force
        && let Some(last_refresh_at) = last_refresh_at
        && (Utc::now() - last_refresh_at).num_seconds()
            < i64::try_from(interval_secs).unwrap_or(i64::MAX)
    {
        return Ok(());
    }

    let mut subscription_proxy_urls = Vec::new();
    let mut fetched_any_subscription = false;
    for subscription_url in &subscription_urls {
        match fetch_subscription_proxy_urls(
            &state.http_clients.shared,
            subscription_url,
            state.config.request_timeout,
        )
        .await
        {
            Ok(urls) => {
                fetched_any_subscription = true;
                subscription_proxy_urls.extend(urls);
            }
            Err(err) => {
                warn!(
                    subscription_url,
                    error = %err,
                    "failed to fetch forward proxy subscription"
                );
            }
        }
    }

    if !subscription_urls.is_empty() && !fetched_any_subscription {
        bail!("all forward proxy subscriptions failed to refresh");
    }

    let _refresh_guard = state.forward_proxy_subscription_refresh_lock.lock().await;
    let added_subscription_endpoints = {
        let mut manager = state.forward_proxy.lock().await;
        if manager.settings.subscription_urls != subscription_urls {
            debug!("skip stale forward proxy subscription refresh after settings changed");
            return Ok(());
        }
        let mut known_subscription_keys = snapshot_active_forward_proxy_endpoints(&manager)
            .into_iter()
            .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
            .map(|endpoint| endpoint.key)
            .collect::<HashSet<_>>();
        if let Some(override_keys) = &known_subscription_keys_override {
            known_subscription_keys.extend(override_keys.iter().cloned());
        }
        manager.apply_subscription_urls(subscription_proxy_urls);
        let after = snapshot_active_forward_proxy_endpoints(&manager);
        after
            .into_iter()
            .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
            .filter(|endpoint| !known_subscription_keys.contains(&endpoint.key))
            .collect::<Vec<_>>()
    };
    sync_forward_proxy_routes(state.as_ref()).await?;
    if !added_subscription_endpoints.is_empty() {
        spawn_forward_proxy_bootstrap_probe_round(
            state.clone(),
            added_subscription_endpoints,
            "subscription-refresh",
        );
    }
    Ok(())
}

async fn sync_forward_proxy_routes(state: &AppState) -> Result<()> {
    let runtime_snapshot = {
        let mut manager = state.forward_proxy.lock().await;
        let mut xray_supervisor = state.xray_supervisor.lock().await;
        xray_supervisor
            .sync_endpoints(&mut manager.endpoints)
            .await?;
        manager.ensure_non_zero_weight();
        manager.snapshot_runtime()
    };
    persist_forward_proxy_runtime_snapshot(state, runtime_snapshot).await
}

async fn persist_forward_proxy_runtime_snapshot(
    state: &AppState,
    runtime_snapshot: Vec<ForwardProxyRuntimeState>,
) -> Result<()> {
    let active_keys = runtime_snapshot
        .iter()
        .map(|entry| entry.proxy_key.clone())
        .collect::<Vec<_>>();
    delete_forward_proxy_runtime_rows_not_in(&state.pool, &active_keys).await?;
    for runtime in &runtime_snapshot {
        persist_forward_proxy_runtime_state(&state.pool, runtime).await?;
    }
    Ok(())
}

async fn fetch_subscription_proxy_urls(
    client: &Client,
    subscription_url: &str,
    request_timeout: Duration,
) -> Result<Vec<String>> {
    let response = timeout(request_timeout, client.get(subscription_url).send())
        .await
        .map_err(|_| anyhow!("subscription request timed out"))?
        .with_context(|| format!("failed to request subscription url: {subscription_url}"))?;
    if !response.status().is_success() {
        bail!(
            "subscription url returned status {}: {subscription_url}",
            response.status()
        );
    }
    let body = timeout(request_timeout, response.text())
        .await
        .map_err(|_| anyhow!("subscription body read timed out"))?
        .context("failed to read subscription body")?;
    Ok(parse_proxy_urls_from_subscription_body(&body))
}

async fn fetch_subscription_proxy_urls_with_validation_budget(
    client: &Client,
    subscription_url: &str,
    total_timeout: Duration,
    started: Instant,
) -> Result<Vec<String>> {
    let request_timeout = remaining_timeout_budget(total_timeout, started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| timeout_error_for_duration(total_timeout))?;
    let response = timeout(request_timeout, client.get(subscription_url).send())
        .await
        .map_err(|_| timeout_error_for_duration(total_timeout))?
        .with_context(|| format!("failed to request subscription url: {subscription_url}"))?;
    if !response.status().is_success() {
        bail!(
            "subscription url returned status {}: {subscription_url}",
            response.status()
        );
    }
    let read_timeout = remaining_timeout_budget(total_timeout, started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| timeout_error_for_duration(total_timeout))?;
    let body = timeout(read_timeout, response.text())
        .await
        .map_err(|_| timeout_error_for_duration(total_timeout))?
        .context("failed to read subscription body")?;
    Ok(parse_proxy_urls_from_subscription_body(&body))
}

fn parse_proxy_urls_from_subscription_body(raw: &str) -> Vec<String> {
    let decoded = decode_subscription_payload(raw);
    normalize_proxy_url_entries(vec![decoded])
}

fn decode_subscription_payload(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains("://")
        || trimmed
            .lines()
            .filter(|line| !line.trim().is_empty())
            .any(|line| line.contains("://"))
    {
        return trimmed.to_string();
    }

    let compact = trimmed
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    for engine in [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ] {
        if let Ok(decoded) = engine.decode(compact.as_bytes())
            && let Ok(text) = String::from_utf8(decoded)
            && text.contains("://")
        {
            return text;
        }
    }
    trimmed.to_string()
}

async fn select_forward_proxy_for_request(state: &AppState) -> SelectedForwardProxy {
    let mut manager = state.forward_proxy.lock().await;
    manager.select_proxy()
}

#[derive(Debug, Clone, Copy, Default)]
struct ForwardProxyAttemptUpdate {
    weight_before: Option<f64>,
    weight_after: Option<f64>,
    weight_delta: Option<f64>,
}

impl ForwardProxyAttemptUpdate {
    fn delta(self) -> Option<f64> {
        self.weight_delta.or_else(|| {
            let (Some(before), Some(after)) = (self.weight_before, self.weight_after) else {
                return None;
            };
            if before.is_finite() && after.is_finite() {
                Some(after - before)
            } else {
                None
            }
        })
    }
}

async fn record_forward_proxy_attempt(
    state: Arc<AppState>,
    selected_proxy: SelectedForwardProxy,
    success: bool,
    latency_ms: Option<f64>,
    failure_kind: Option<&str>,
    is_probe: bool,
) -> ForwardProxyAttemptUpdate {
    let (updated_runtime, probe_candidate, attempt_update) = {
        let mut manager = state.forward_proxy.lock().await;
        let runtime_active = manager
            .endpoints
            .iter()
            .any(|endpoint| endpoint.key == selected_proxy.key);
        let weight_before = if runtime_active {
            manager
                .runtime
                .get(&selected_proxy.key)
                .map(|runtime| runtime.weight)
        } else {
            None
        };
        manager.record_attempt(&selected_proxy.key, success, latency_ms, is_probe);
        let updated_runtime = if runtime_active {
            manager.runtime.get(&selected_proxy.key).cloned()
        } else {
            None
        };
        let weight_after = updated_runtime.as_ref().map(|runtime| runtime.weight);
        let weight_delta = match (weight_before, weight_after) {
            (Some(before), Some(after)) if before.is_finite() && after.is_finite() => {
                Some(after - before)
            }
            _ => None,
        };
        let probe_candidate = if is_probe {
            None
        } else {
            manager.mark_probe_started()
        };
        (
            updated_runtime,
            probe_candidate,
            ForwardProxyAttemptUpdate {
                weight_before,
                weight_after,
                weight_delta,
            },
        )
    };

    if let Err(err) = insert_forward_proxy_attempt(
        &state.pool,
        &selected_proxy.key,
        success,
        latency_ms,
        failure_kind,
        is_probe,
    )
    .await
    {
        warn!(
            proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
            error = %err,
            "failed to persist forward proxy attempt"
        );
    }

    if let Some(runtime) = updated_runtime {
        let sample_epoch_us = Utc::now().timestamp_micros();
        let bucket_start_epoch = align_bucket_epoch(sample_epoch_us.div_euclid(1_000_000), 3600, 0);
        if let Err(err) = persist_forward_proxy_runtime_state(&state.pool, &runtime).await {
            warn!(
                proxy_key_ref = %forward_proxy_log_ref(&runtime.proxy_key),
                error = %err,
                "failed to persist forward proxy runtime state"
            );
        }
        if let Err(err) = upsert_forward_proxy_weight_hourly_bucket(
            &state.pool,
            &runtime.proxy_key,
            bucket_start_epoch,
            runtime.weight,
            sample_epoch_us,
        )
        .await
        {
            warn!(
                proxy_key_ref = %forward_proxy_log_ref(&runtime.proxy_key),
                error = %err,
                "failed to persist forward proxy weight bucket"
            );
        }
    }

    if let Some(candidate) = probe_candidate {
        spawn_penalized_forward_proxy_probe(state, candidate);
    }

    attempt_update
}

fn spawn_penalized_forward_proxy_probe(state: Arc<AppState>, candidate: SelectedForwardProxy) {
    tokio::spawn(async move {
        let probe_result = async {
            let target = state
                .config
                .openai_upstream_base_url
                .join("v1/models")
                .context("failed to build probe target url")?;
            let client = state
                .http_clients
                .client_for_forward_proxy(candidate.endpoint_url.as_ref())?;
            let started = Instant::now();
            let response = timeout(
                state.config.openai_proxy_handshake_timeout,
                client.get(target).send(),
            )
            .await
            .map_err(|_| anyhow!("probe timed out"))?
            .context("probe request failed")?;
            let success = !response.status().is_server_error();
            let latency_ms = Some(elapsed_ms(started));
            record_forward_proxy_attempt(
                state.clone(),
                candidate.clone(),
                success,
                latency_ms,
                if success {
                    None
                } else {
                    Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
                },
                true,
            )
            .await;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(err) = probe_result {
            warn!(
                proxy_key_ref = %forward_proxy_log_ref(&candidate.key),
                proxy_source = candidate.source,
                proxy_label = candidate.display_name,
                proxy_url_ref = %forward_proxy_log_ref_option(candidate.endpoint_url_raw.as_deref()),
                error = %err,
                "penalized forward proxy probe failed"
            );
        }

        let mut manager = state.forward_proxy.lock().await;
        manager.mark_probe_finished();
    });
}

async fn fetch_upstream_models_payload(
    state: Arc<AppState>,
    selected_proxy: SelectedForwardProxy,
    target_url: Url,
    headers: &HeaderMap,
) -> Result<Value> {
    let client = state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())?;
    let mut upstream_request = client.request(Method::GET, target_url);
    let request_connection_scoped = connection_scoped_header_names(headers);
    for (name, value) in headers {
        if should_forward_proxy_header(name, &request_connection_scoped) {
            upstream_request = upstream_request.header(name, value);
        }
    }

    let started = Instant::now();
    let handshake_timeout = state.config.openai_proxy_handshake_timeout;
    let upstream_response = timeout(handshake_timeout, upstream_request.send())
        .await
        .map_err(|_| {
            anyhow!(
                "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
                handshake_timeout.as_millis()
            )
        })?
        .context("failed to contact upstream")?;
    let latency_ms = Some(elapsed_ms(started));

    if upstream_response.status().is_server_error() {
        record_forward_proxy_attempt(
            state.clone(),
            selected_proxy,
            false,
            latency_ms,
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX),
            false,
        )
        .await;
        bail!(
            "upstream /v1/models returned status {}",
            upstream_response.status()
        );
    }

    let payload = timeout(
        handshake_timeout,
        upstream_response.json::<Value>(),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms while decoding upstream /v1/models response",
            handshake_timeout.as_millis()
        )
    })?
    .context("failed to decode upstream /v1/models response as JSON")?;

    payload
        .get("data")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("upstream /v1/models payload missing data array"))?;

    record_forward_proxy_attempt(state, selected_proxy, true, latency_ms, None, false).await;
    Ok(payload)
}

fn detect_versions(static_dir: Option<&Path>) -> (String, String) {
    let backend_base = option_env!("APP_EFFECTIVE_VERSION")
        .map(|s| s.to_string())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let backend = if cfg!(debug_assertions) {
        format!("{}-dev", backend_base)
    } else {
        backend_base
    };

    // Try to get frontend version from a version.json written during build
    let frontend = static_dir
        .and_then(|p| {
            let path = p.join("version.json");
            fs::File::open(&path).ok().and_then(|mut f| {
                let mut s = String::new();
                if f.read_to_string(&mut s).is_ok() {
                    serde_json::from_str::<serde_json::Value>(&s)
                        .ok()
                        .and_then(|v| {
                            v.get("version")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                } else {
                    None
                }
            })
        })
        .or_else(|| {
            // Fallback to reading the web/package.json in dev setups
            let path = Path::new("web").join("package.json");
            fs::File::open(&path).ok().and_then(|mut f| {
                let mut s = String::new();
                if f.read_to_string(&mut s).is_ok() {
                    serde_json::from_str::<serde_json::Value>(&s)
                        .ok()
                        .and_then(|v| {
                            v.get("version")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "unknown".to_string());

    let frontend = if cfg!(debug_assertions) {
        format!("{}-dev", frontend)
    } else {
        frontend
    };

    (backend, frontend)
}

fn ensure_db_directory(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create database directory: {}", parent.display())
        })?;
    }
    Ok(())
}

fn build_sqlite_connect_options(
    database_url: &str,
    busy_timeout: Duration,
) -> Result<SqliteConnectOptions> {
    let options = SqliteConnectOptions::from_str(database_url)
        .context("invalid sqlite database url")?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(busy_timeout);
    Ok(options)
}

#[derive(Debug, Clone)]
struct AppState {
    config: AppConfig,
    pool: Pool<Sqlite>,
    http_clients: HttpClients,
    broadcaster: broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: Arc<Mutex<BroadcastStateCache>>,
    proxy_summary_quota_broadcast_seq: Arc<AtomicU64>,
    proxy_summary_quota_broadcast_running: Arc<AtomicBool>,
    startup_ready: Arc<AtomicBool>,
    semaphore: Arc<Semaphore>,
    proxy_model_settings: Arc<RwLock<ProxyModelSettings>>,
    proxy_model_settings_update_lock: Arc<Mutex<()>>,
    forward_proxy: Arc<Mutex<ForwardProxyManager>>,
    xray_supervisor: Arc<Mutex<XraySupervisor>>,
    forward_proxy_settings_update_lock: Arc<Mutex<()>>,
    forward_proxy_subscription_refresh_lock: Arc<Mutex<()>>,
    pricing_settings_update_lock: Arc<Mutex<()>>,
    pricing_catalog: Arc<RwLock<PricingCatalog>>,
    prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
}

#[derive(Debug, Default)]
struct BroadcastStateCache {
    summaries: HashMap<String, StatsResponse>,
    quota: Option<QuotaSnapshotResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum BroadcastPayload {
    Version {
        version: String,
    },
    Records {
        records: Vec<ApiInvocation>,
    },
    Summary {
        window: String,
        summary: StatsResponse,
    },
    Quota {
        snapshot: Box<QuotaSnapshotResponse>,
    },
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct ApiInvocation {
    id: i64,
    invoke_id: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    occurred_at: String,
    source: String,
    #[sqlx(default)]
    proxy_display_name: Option<String>,
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    #[sqlx(default)]
    reasoning_effort: Option<String>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    status: Option<String>,
    error_message: Option<String>,
    #[sqlx(default)]
    failure_kind: Option<String>,
    #[sqlx(default)]
    failure_class: Option<String>,
    #[sqlx(default)]
    is_actionable: Option<bool>,
    #[sqlx(default)]
    endpoint: Option<String>,
    #[sqlx(default)]
    requester_ip: Option<String>,
    #[sqlx(default)]
    prompt_cache_key: Option<String>,
    #[sqlx(default)]
    requested_service_tier: Option<String>,
    #[sqlx(default)]
    service_tier: Option<String>,
    #[sqlx(default)]
    proxy_weight_delta: Option<f64>,
    #[sqlx(default)]
    cost_estimated: Option<i64>,
    #[sqlx(default)]
    price_version: Option<String>,
    #[sqlx(default)]
    request_raw_path: Option<String>,
    #[sqlx(default)]
    request_raw_size: Option<i64>,
    #[sqlx(default)]
    request_raw_truncated: Option<i64>,
    #[sqlx(default)]
    request_raw_truncated_reason: Option<String>,
    #[sqlx(default)]
    response_raw_path: Option<String>,
    #[sqlx(default)]
    response_raw_size: Option<i64>,
    #[sqlx(default)]
    response_raw_truncated: Option<i64>,
    #[sqlx(default)]
    response_raw_truncated_reason: Option<String>,
    #[sqlx(default)]
    raw_expires_at: Option<String>,
    detail_level: String,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    detail_pruned_at: Option<String>,
    #[sqlx(default)]
    detail_prune_reason: Option<String>,
    #[sqlx(default)]
    t_total_ms: Option<f64>,
    #[sqlx(default)]
    t_req_read_ms: Option<f64>,
    #[sqlx(default)]
    t_req_parse_ms: Option<f64>,
    #[sqlx(default)]
    t_upstream_connect_ms: Option<f64>,
    #[sqlx(default)]
    t_upstream_ttfb_ms: Option<f64>,
    #[sqlx(default)]
    t_upstream_stream_ms: Option<f64>,
    #[sqlx(default)]
    t_resp_parse_ms: Option<f64>,
    #[sqlx(default)]
    t_persist_ms: Option<f64>,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    records: Vec<ApiInvocation>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct StatsResponse {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_cost: f64,
    total_tokens: i64,
}

#[derive(Debug, FromRow)]
struct StatsRow {
    total_count: i64,
    success_count: Option<i64>,
    failure_count: Option<i64>,
    total_cost: f64,
    total_tokens: i64,
}

#[derive(Debug, Default, Clone, Copy)]
struct StatsTotals {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_cost: f64,
    total_tokens: i64,
}

impl StatsTotals {
    fn add(self, other: StatsTotals) -> StatsTotals {
        StatsTotals {
            total_count: self.total_count + other.total_count,
            success_count: self.success_count + other.success_count,
            failure_count: self.failure_count + other.failure_count,
            total_cost: self.total_cost + other.total_cost,
            total_tokens: self.total_tokens + other.total_tokens,
        }
    }

    fn into_response(self) -> StatsResponse {
        StatsResponse {
            total_count: self.total_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_cost: self.total_cost,
            total_tokens: self.total_tokens,
        }
    }
}

impl From<StatsRow> for StatsTotals {
    fn from(value: StatsRow) -> Self {
        Self {
            total_count: value.total_count,
            success_count: value.success_count.unwrap_or(0),
            failure_count: value.failure_count.unwrap_or(0),
            total_cost: value.total_cost,
            total_tokens: value.total_tokens,
        }
    }
}

impl From<StatsRow> for StatsResponse {
    fn from(value: StatsRow) -> Self {
        StatsTotals::from(value).into_response()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimeseriesResponse {
    range_start: String,
    range_end: String,
    bucket_seconds: i64,
    points: Vec<TimeseriesPoint>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TimeseriesPoint {
    bucket_start: String,
    bucket_end: String,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    first_byte_sample_count: i64,
    first_byte_avg_ms: Option<f64>,
    first_byte_p95_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaSnapshotResponse {
    #[serde(serialize_with = "serialize_local_or_utc_to_utc_iso")]
    captured_at: String,
    amount_limit: Option<f64>,
    used_amount: Option<f64>,
    remaining_amount: Option<f64>,
    period: Option<String>,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    period_reset_time: Option<String>,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    expire_time: Option<String>,
    is_active: bool,
    total_cost: f64,
    total_requests: i64,
    total_tokens: i64,
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    last_request_time: Option<String>,
    billing_type: Option<String>,
    remaining_count: Option<i64>,
    used_count: Option<i64>,
    sub_type_name: Option<String>,
}

#[derive(Debug, FromRow)]
struct QuotaSnapshotRow {
    captured_at: String,
    amount_limit: Option<f64>,
    used_amount: Option<f64>,
    remaining_amount: Option<f64>,
    period: Option<String>,
    period_reset_time: Option<String>,
    expire_time: Option<String>,
    is_active: Option<i64>,
    total_cost: f64,
    total_requests: i64,
    total_tokens: i64,
    last_request_time: Option<String>,
    billing_type: Option<String>,
    remaining_count: Option<i64>,
    used_count: Option<i64>,
    sub_type_name: Option<String>,
}

impl From<QuotaSnapshotRow> for QuotaSnapshotResponse {
    fn from(value: QuotaSnapshotRow) -> Self {
        Self {
            captured_at: value.captured_at,
            amount_limit: value.amount_limit,
            used_amount: value.used_amount,
            remaining_amount: value.remaining_amount,
            period: value.period,
            period_reset_time: value.period_reset_time,
            expire_time: value.expire_time,
            is_active: value.is_active.unwrap_or(0) != 0,
            total_cost: value.total_cost,
            total_requests: value.total_requests,
            total_tokens: value.total_tokens,
            last_request_time: value.last_request_time,
            billing_type: value.billing_type,
            remaining_count: value.remaining_count,
            used_count: value.used_count,
            sub_type_name: value.sub_type_name,
        }
    }
}

impl QuotaSnapshotResponse {
    async fn fetch_latest(pool: &Pool<Sqlite>) -> Result<Option<Self>> {
        let row = sqlx::query_as::<_, QuotaSnapshotRow>(
            r#"
            SELECT
                captured_at,
                amount_limit,
                used_amount,
                remaining_amount,
                period,
                period_reset_time,
                expire_time,
                is_active,
                total_cost,
                total_requests,
                total_tokens,
                last_request_time,
                billing_type,
                remaining_count,
                used_count,
                sub_type_name
            FROM codex_quota_snapshots
            ORDER BY captured_at DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await?;

        Ok(row.map(Into::into))
    }

    fn degraded_default() -> Self {
        Self {
            captured_at: format_utc_iso(Utc::now()),
            amount_limit: None,
            used_amount: None,
            remaining_amount: None,
            period: None,
            period_reset_time: None,
            expire_time: None,
            is_active: false,
            total_cost: 0.0,
            total_requests: 0,
            total_tokens: 0,
            last_request_time: None,
            billing_type: None,
            remaining_count: None,
            used_count: None,
            sub_type_name: None,
        }
    }
}

#[derive(Debug, Clone)]
struct PricingCatalog {
    version: String,
    models: HashMap<String, ModelPricing>,
}

impl Default for PricingCatalog {
    fn default() -> Self {
        Self {
            version: "unavailable".to_string(),
            models: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelPricing {
    input_per_1m: f64,
    output_per_1m: f64,
    #[serde(default)]
    cache_input_per_1m: Option<f64>,
    #[serde(default)]
    reasoning_per_1m: Option<f64>,
    #[serde(default = "default_pricing_source_custom")]
    source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricingEntry {
    model: String,
    input_per_1m: f64,
    output_per_1m: f64,
    #[serde(default)]
    cache_input_per_1m: Option<f64>,
    #[serde(default)]
    reasoning_per_1m: Option<f64>,
    #[serde(default = "default_pricing_source_custom")]
    source: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PricingSettingsUpdateRequest {
    catalog_version: String,
    #[serde(default)]
    entries: Vec<PricingEntry>,
}

impl PricingSettingsUpdateRequest {
    fn normalized(self) -> Result<PricingCatalog, (StatusCode, String)> {
        let version = normalize_pricing_catalog_version(self.catalog_version).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "catalogVersion must be a non-empty string".to_string(),
            )
        })?;
        let mut models = HashMap::new();
        for entry in self.entries {
            let model_id = entry.model.trim();
            if model_id.is_empty() || model_id.len() > 128 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid model id: {}", entry.model),
                ));
            }
            if !entry.input_per_1m.is_finite()
                || !entry.output_per_1m.is_finite()
                || entry.input_per_1m < 0.0
                || entry.output_per_1m < 0.0
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid pricing values for model: {model_id}"),
                ));
            }
            if let Some(cache) = entry.cache_input_per_1m
                && (!cache.is_finite() || cache < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid cacheInputPer1m for model: {model_id}"),
                ));
            }
            if let Some(reasoning) = entry.reasoning_per_1m
                && (!reasoning.is_finite() || reasoning < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid reasoningPer1m for model: {model_id}"),
                ));
            }

            let inserted = models.insert(
                model_id.to_string(),
                ModelPricing {
                    input_per_1m: entry.input_per_1m,
                    output_per_1m: entry.output_per_1m,
                    cache_input_per_1m: entry.cache_input_per_1m,
                    reasoning_per_1m: entry.reasoning_per_1m,
                    source: normalize_pricing_source(entry.source),
                },
            );
            if inserted.is_some() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("duplicate model id: {model_id}"),
                ));
            }
        }
        Ok(PricingCatalog { version, models })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PricingSettingsResponse {
    catalog_version: String,
    entries: Vec<PricingEntry>,
}

impl PricingSettingsResponse {
    fn from_catalog(catalog: &PricingCatalog) -> Self {
        let mut entries = catalog
            .models
            .iter()
            .map(|(model, pricing)| PricingEntry {
                model: model.clone(),
                input_per_1m: pricing.input_per_1m,
                output_per_1m: pricing.output_per_1m,
                cache_input_per_1m: pricing.cache_input_per_1m,
                reasoning_per_1m: pricing.reasoning_per_1m,
                source: pricing.source.clone(),
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.model.cmp(&b.model));
        Self {
            catalog_version: catalog.version.clone(),
            entries,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxySettings {
    #[serde(default)]
    proxy_urls: Vec<String>,
    #[serde(default)]
    subscription_urls: Vec<String>,
    #[serde(default = "default_forward_proxy_subscription_interval_secs")]
    subscription_update_interval_secs: u64,
    #[serde(default = "default_forward_proxy_insert_direct")]
    insert_direct: bool,
}

impl Default for ForwardProxySettings {
    fn default() -> Self {
        Self {
            proxy_urls: Vec::new(),
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: default_forward_proxy_subscription_interval_secs(),
            insert_direct: default_forward_proxy_insert_direct(),
        }
    }
}

impl ForwardProxySettings {
    fn normalized(self) -> Self {
        let mut normalized = Self {
            proxy_urls: normalize_proxy_url_entries(self.proxy_urls),
            subscription_urls: normalize_subscription_entries(self.subscription_urls),
            subscription_update_interval_secs: self
                .subscription_update_interval_secs
                .clamp(60, 7 * 24 * 60 * 60),
            insert_direct: self.insert_direct,
        };
        if !normalized.insert_direct
            && normalize_proxy_endpoints_from_urls(
                &normalized.proxy_urls,
                FORWARD_PROXY_SOURCE_MANUAL,
            )
            .is_empty()
        {
            normalized.insert_direct = true;
        }
        normalized
    }
}

#[derive(Debug, FromRow)]
struct ForwardProxySettingsRow {
    proxy_urls_json: Option<String>,
    subscription_urls_json: Option<String>,
    subscription_update_interval_secs: Option<i64>,
    insert_direct: Option<i64>,
}

impl From<ForwardProxySettingsRow> for ForwardProxySettings {
    fn from(value: ForwardProxySettingsRow) -> Self {
        let proxy_urls = decode_string_vec_json(value.proxy_urls_json.as_deref());
        let subscription_urls = decode_string_vec_json(value.subscription_urls_json.as_deref());
        let interval = value
            .subscription_update_interval_secs
            .and_then(|v| u64::try_from(v).ok())
            .unwrap_or_else(default_forward_proxy_subscription_interval_secs);
        let insert_direct = value
            .insert_direct
            .map(|v| v != 0)
            .unwrap_or_else(default_forward_proxy_insert_direct);
        ForwardProxySettings {
            proxy_urls,
            subscription_urls,
            subscription_update_interval_secs: interval,
            insert_direct,
        }
        .normalized()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxySettingsUpdateRequest {
    #[serde(default)]
    proxy_urls: Vec<String>,
    #[serde(default)]
    subscription_urls: Vec<String>,
    #[serde(default = "default_forward_proxy_subscription_interval_secs")]
    subscription_update_interval_secs: u64,
    #[serde(default = "default_forward_proxy_insert_direct")]
    insert_direct: bool,
}

impl From<ForwardProxySettingsUpdateRequest> for ForwardProxySettings {
    fn from(value: ForwardProxySettingsUpdateRequest) -> Self {
        ForwardProxySettings {
            proxy_urls: value.proxy_urls,
            subscription_urls: value.subscription_urls,
            subscription_update_interval_secs: value.subscription_update_interval_secs,
            insert_direct: value.insert_direct,
        }
        .normalized()
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ForwardProxyValidationKind {
    ProxyUrl,
    SubscriptionUrl,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxyCandidateValidationRequest {
    kind: ForwardProxyValidationKind,
    value: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxyCandidateValidationResponse {
    ok: bool,
    message: String,
    normalized_value: Option<String>,
    discovered_nodes: Option<usize>,
    latency_ms: Option<f64>,
}

impl ForwardProxyCandidateValidationResponse {
    fn success(
        message: impl Into<String>,
        normalized_value: Option<String>,
        discovered_nodes: Option<usize>,
        latency_ms: Option<f64>,
    ) -> Self {
        Self {
            ok: true,
            message: message.into(),
            normalized_value,
            discovered_nodes,
            latency_ms,
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            normalized_value: None,
            discovered_nodes: None,
            latency_ms: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForwardProxyProtocol {
    Direct,
    Http,
    Https,
    Socks5,
    Socks5h,
    Vmess,
    Vless,
    Trojan,
    Shadowsocks,
}

#[derive(Debug, Clone)]
struct ForwardProxyEndpoint {
    key: String,
    source: String,
    display_name: String,
    protocol: ForwardProxyProtocol,
    endpoint_url: Option<Url>,
    raw_url: Option<String>,
}

impl ForwardProxyEndpoint {
    fn direct() -> Self {
        Self {
            key: FORWARD_PROXY_DIRECT_KEY.to_string(),
            source: FORWARD_PROXY_SOURCE_DIRECT.to_string(),
            display_name: FORWARD_PROXY_DIRECT_LABEL.to_string(),
            protocol: ForwardProxyProtocol::Direct,
            endpoint_url: None,
            raw_url: None,
        }
    }

    fn is_selectable(&self) -> bool {
        self.protocol == ForwardProxyProtocol::Direct || self.endpoint_url.is_some()
    }

    fn requires_xray(&self) -> bool {
        matches!(
            self.protocol,
            ForwardProxyProtocol::Vmess
                | ForwardProxyProtocol::Vless
                | ForwardProxyProtocol::Trojan
                | ForwardProxyProtocol::Shadowsocks
        )
    }
}

#[derive(Debug, Clone)]
struct ForwardProxyRuntimeState {
    proxy_key: String,
    display_name: String,
    source: String,
    endpoint_url: Option<String>,
    weight: f64,
    success_ema: f64,
    latency_ema_ms: Option<f64>,
    consecutive_failures: u32,
}

impl ForwardProxyRuntimeState {
    fn default_for_endpoint(endpoint: &ForwardProxyEndpoint, algo: ForwardProxyAlgo) -> Self {
        Self {
            proxy_key: endpoint.key.clone(),
            display_name: endpoint.display_name.clone(),
            source: endpoint.source.clone(),
            endpoint_url: endpoint.raw_url.clone(),
            weight: if endpoint.key == FORWARD_PROXY_DIRECT_KEY {
                match algo {
                    ForwardProxyAlgo::V1 => 1.0,
                    ForwardProxyAlgo::V2 => FORWARD_PROXY_V2_DIRECT_INITIAL_WEIGHT,
                }
            } else {
                0.8
            },
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        }
    }

    fn is_penalized(&self) -> bool {
        self.weight <= 0.0
    }
}

#[derive(Debug, FromRow)]
struct ForwardProxyRuntimeRow {
    proxy_key: String,
    display_name: String,
    source: String,
    endpoint_url: Option<String>,
    weight: f64,
    success_ema: f64,
    latency_ema_ms: Option<f64>,
    consecutive_failures: i64,
}

impl From<ForwardProxyRuntimeRow> for ForwardProxyRuntimeState {
    fn from(value: ForwardProxyRuntimeRow) -> Self {
        Self {
            proxy_key: value.proxy_key,
            display_name: value.display_name,
            source: value.source,
            endpoint_url: value.endpoint_url,
            weight: value.weight,
            success_ema: value.success_ema.clamp(0.0, 1.0),
            latency_ema_ms: value.latency_ema_ms,
            consecutive_failures: value.consecutive_failures.max(0) as u32,
        }
    }
}

#[derive(Debug, Clone)]
struct ForwardProxyManager {
    algo: ForwardProxyAlgo,
    settings: ForwardProxySettings,
    endpoints: Vec<ForwardProxyEndpoint>,
    runtime: HashMap<String, ForwardProxyRuntimeState>,
    selection_counter: u64,
    requests_since_probe: u64,
    probe_in_flight: bool,
    last_probe_at: DateTime<Utc>,
    last_subscription_refresh_at: Option<DateTime<Utc>>,
}

impl ForwardProxyManager {
    #[cfg(test)]
    fn new(settings: ForwardProxySettings, runtime_rows: Vec<ForwardProxyRuntimeState>) -> Self {
        Self::with_algo(settings, runtime_rows, ForwardProxyAlgo::V1)
    }

    fn with_algo(
        settings: ForwardProxySettings,
        runtime_rows: Vec<ForwardProxyRuntimeState>,
        algo: ForwardProxyAlgo,
    ) -> Self {
        let runtime = runtime_rows
            .into_iter()
            .map(|mut entry| {
                Self::normalize_runtime_for_algo(&mut entry, algo);
                (entry.proxy_key.clone(), entry)
            })
            .collect::<HashMap<_, _>>();
        let mut manager = Self {
            algo,
            settings,
            endpoints: Vec::new(),
            runtime,
            selection_counter: 0,
            requests_since_probe: 0,
            probe_in_flight: false,
            last_probe_at: Utc::now() - ChronoDuration::seconds(algo.probe_interval_secs()),
            last_subscription_refresh_at: None,
        };
        manager.rebuild_endpoints(Vec::new());
        manager
    }

    fn normalize_runtime_for_algo(runtime: &mut ForwardProxyRuntimeState, algo: ForwardProxyAlgo) {
        runtime.success_ema = runtime.success_ema.clamp(0.0, 1.0);
        if runtime
            .latency_ema_ms
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            runtime.latency_ema_ms = None;
        }
        if !runtime.weight.is_finite() {
            runtime.weight = 0.0;
        }
        runtime.weight = match algo {
            ForwardProxyAlgo::V1 => runtime
                .weight
                .clamp(FORWARD_PROXY_WEIGHT_MIN, FORWARD_PROXY_WEIGHT_MAX),
            ForwardProxyAlgo::V2 => runtime
                .weight
                .clamp(FORWARD_PROXY_V2_WEIGHT_MIN, FORWARD_PROXY_V2_WEIGHT_MAX),
        };
    }

    fn apply_settings(&mut self, settings: ForwardProxySettings) {
        self.settings = settings;
        self.rebuild_endpoints(Vec::new());
    }

    fn apply_subscription_urls(&mut self, proxy_urls: Vec<String>) {
        let normalized_urls = normalize_proxy_url_entries(proxy_urls);
        let subscription_endpoints = normalize_proxy_endpoints_from_urls(
            &normalized_urls,
            FORWARD_PROXY_SOURCE_SUBSCRIPTION,
        );
        self.rebuild_endpoints(subscription_endpoints);
        self.last_subscription_refresh_at = Some(Utc::now());
    }

    fn rebuild_endpoints(&mut self, subscription_endpoints: Vec<ForwardProxyEndpoint>) {
        let mut merged = Vec::new();
        let manual = normalize_proxy_endpoints_from_urls(
            &self.settings.proxy_urls,
            FORWARD_PROXY_SOURCE_MANUAL,
        );
        let mut seen = HashSet::new();
        for endpoint in manual.into_iter().chain(subscription_endpoints.into_iter()) {
            if seen.insert(endpoint.key.clone()) {
                merged.push(endpoint);
            }
        }
        if self.settings.insert_direct {
            merged.push(ForwardProxyEndpoint::direct());
        }
        if merged.is_empty() {
            merged.push(ForwardProxyEndpoint::direct());
        }
        self.endpoints = merged;

        let algo = self.algo;
        for endpoint in &self.endpoints {
            match self.runtime.entry(endpoint.key.clone()) {
                std::collections::hash_map::Entry::Occupied(mut occupied) => {
                    let runtime = occupied.get_mut();
                    runtime.display_name = endpoint.display_name.clone();
                    runtime.source = endpoint.source.clone();
                    runtime.endpoint_url = endpoint.raw_url.clone();
                }
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    vacant.insert(ForwardProxyRuntimeState::default_for_endpoint(
                        endpoint, algo,
                    ));
                }
            }
        }
        self.ensure_non_zero_weight();
    }

    fn ensure_non_zero_weight(&mut self) {
        let minimum = match self.algo {
            ForwardProxyAlgo::V1 => 1,
            ForwardProxyAlgo::V2 => FORWARD_PROXY_V2_MIN_POSITIVE_CANDIDATES,
        };
        self.ensure_min_positive_candidates(minimum, self.algo.probe_recovery_weight());
    }

    fn selectable_endpoint_keys(&self) -> HashSet<&str> {
        self.endpoints
            .iter()
            .filter(|endpoint| endpoint.is_selectable())
            .map(|endpoint| endpoint.key.as_str())
            .collect::<HashSet<_>>()
    }

    fn ensure_min_positive_candidates(&mut self, minimum: usize, recovery_weight: f64) {
        if minimum == 0 {
            return;
        }

        let selectable_keys = self.selectable_endpoint_keys();
        let active_keys = if selectable_keys.is_empty() {
            self.endpoints
                .iter()
                .map(|endpoint| endpoint.key.as_str())
                .collect::<HashSet<_>>()
        } else {
            selectable_keys
        };
        let mut positive_count = self
            .runtime
            .values()
            .filter(|entry| {
                active_keys.contains(entry.proxy_key.as_str())
                    && entry.weight > 0.0
                    && entry.weight.is_finite()
            })
            .count();
        if positive_count >= minimum {
            return;
        }

        let mut candidates = self
            .runtime
            .values()
            .filter(|entry| active_keys.contains(entry.proxy_key.as_str()))
            .map(|entry| (entry.proxy_key.clone(), entry.weight))
            .collect::<Vec<_>>();
        candidates.sort_by(|lhs, rhs| rhs.1.total_cmp(&lhs.1));

        for (proxy_key, _) in candidates {
            if positive_count >= minimum {
                break;
            }
            if let Some(entry) = self.runtime.get_mut(&proxy_key)
                && !(entry.weight > 0.0 && entry.weight.is_finite())
            {
                entry.weight = recovery_weight;
                if self.algo == ForwardProxyAlgo::V2 {
                    entry.consecutive_failures = 0;
                }
                positive_count += 1;
            }
        }
    }

    fn snapshot_runtime(&self) -> Vec<ForwardProxyRuntimeState> {
        self.endpoints
            .iter()
            .filter_map(|endpoint| self.runtime.get(&endpoint.key).cloned())
            .collect()
    }

    fn select_proxy(&mut self) -> SelectedForwardProxy {
        self.selection_counter = self.selection_counter.wrapping_add(1);
        self.requests_since_probe = self.requests_since_probe.saturating_add(1);
        self.ensure_non_zero_weight();

        let mut candidates = Vec::new();
        let mut total_weight = 0.0f64;
        for endpoint in &self.endpoints {
            if !endpoint.is_selectable() {
                continue;
            }
            if let Some(runtime) = self.runtime.get(&endpoint.key)
                && runtime.weight > 0.0
                && runtime.weight.is_finite()
            {
                let effective_weight = if self.algo == ForwardProxyAlgo::V2 {
                    let success_factor = runtime.success_ema.clamp(0.0, 1.0).powi(8).max(0.01);
                    runtime.weight.powi(2) * success_factor
                } else {
                    runtime.weight
                };
                total_weight += effective_weight;
                candidates.push((endpoint, effective_weight));
            }
        }

        if self.algo == ForwardProxyAlgo::V2 && candidates.len() > 3 {
            candidates.sort_by(|lhs, rhs| rhs.1.total_cmp(&lhs.1));
            candidates.truncate(3);
            total_weight = candidates.iter().map(|(_, weight)| *weight).sum::<f64>();
        }

        if candidates.is_empty() {
            let fallback = self
                .endpoints
                .iter()
                .find(|endpoint| endpoint.protocol == ForwardProxyProtocol::Direct)
                .cloned()
                .or_else(|| {
                    self.endpoints
                        .iter()
                        .find(|endpoint| endpoint.is_selectable())
                        .cloned()
                })
                .or_else(|| self.endpoints.first().cloned())
                .unwrap_or_else(ForwardProxyEndpoint::direct);
            return SelectedForwardProxy::from_endpoint(&fallback);
        }

        let seed = self.selection_counter;
        let random = deterministic_unit_f64(seed);
        let mut threshold = random * total_weight;
        let mut last_candidate: Option<&ForwardProxyEndpoint> = None;
        for (endpoint, weight) in candidates {
            last_candidate = Some(endpoint);
            if threshold <= weight {
                return SelectedForwardProxy::from_endpoint(endpoint);
            }
            threshold -= weight;
        }
        SelectedForwardProxy::from_endpoint(last_candidate.unwrap_or_else(|| {
            self.endpoints
                .iter()
                .find(|endpoint| endpoint.is_selectable())
                .or_else(|| self.endpoints.first())
                .expect("forward proxy endpoints should not be empty")
        }))
    }

    fn record_attempt(
        &mut self,
        proxy_key: &str,
        success: bool,
        latency_ms: Option<f64>,
        is_probe: bool,
    ) {
        if !self
            .endpoints
            .iter()
            .any(|endpoint| endpoint.key == proxy_key)
        {
            return;
        }
        let Some(runtime) = self.runtime.get_mut(proxy_key) else {
            return;
        };

        Self::update_runtime_ema(runtime, success, latency_ms);
        match self.algo {
            ForwardProxyAlgo::V1 => Self::record_attempt_v1(runtime, success, is_probe),
            ForwardProxyAlgo::V2 => Self::record_attempt_v2(runtime, success, is_probe),
        }
        self.ensure_non_zero_weight();
    }

    fn update_runtime_ema(
        runtime: &mut ForwardProxyRuntimeState,
        success: bool,
        latency_ms: Option<f64>,
    ) {
        runtime.success_ema = runtime.success_ema * 0.9 + if success { 0.1 } else { 0.0 };
        if let Some(latency_ms) = latency_ms.filter(|value| value.is_finite() && *value >= 0.0) {
            runtime.latency_ema_ms = Some(match runtime.latency_ema_ms {
                Some(previous) => previous * 0.8 + latency_ms * 0.2,
                None => latency_ms,
            });
        }
    }

    fn record_attempt_v1(runtime: &mut ForwardProxyRuntimeState, success: bool, is_probe: bool) {
        if success {
            runtime.consecutive_failures = 0;
            let latency_penalty = runtime
                .latency_ema_ms
                .map(|value| (value / 2500.0).min(0.6))
                .unwrap_or(0.0);
            runtime.weight += FORWARD_PROXY_WEIGHT_SUCCESS_BONUS - latency_penalty;
            if is_probe && runtime.weight <= 0.0 {
                runtime.weight = FORWARD_PROXY_PROBE_RECOVERY_WEIGHT;
            }
        } else {
            runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
            let failure_penalty = FORWARD_PROXY_WEIGHT_FAILURE_PENALTY_BASE
                + f64::from(runtime.consecutive_failures.saturating_sub(1))
                    * FORWARD_PROXY_WEIGHT_FAILURE_PENALTY_STEP;
            runtime.weight -= failure_penalty;
        }

        runtime.weight = runtime
            .weight
            .clamp(FORWARD_PROXY_WEIGHT_MIN, FORWARD_PROXY_WEIGHT_MAX);

        if success && runtime.weight < FORWARD_PROXY_WEIGHT_RECOVERY {
            runtime.weight = runtime.weight.max(FORWARD_PROXY_WEIGHT_RECOVERY * 0.5);
        }
    }

    fn record_attempt_v2(runtime: &mut ForwardProxyRuntimeState, success: bool, is_probe: bool) {
        if success {
            runtime.consecutive_failures = 0;
            let latency_penalty = runtime
                .latency_ema_ms
                .map(|value| {
                    (value / FORWARD_PROXY_V2_WEIGHT_SUCCESS_LATENCY_DIVISOR)
                        .min(FORWARD_PROXY_V2_WEIGHT_SUCCESS_LATENCY_CAP)
                })
                .unwrap_or(0.0);
            let success_gain = (FORWARD_PROXY_V2_WEIGHT_SUCCESS_BASE - latency_penalty)
                .max(FORWARD_PROXY_V2_WEIGHT_SUCCESS_MIN_GAIN);
            runtime.weight += success_gain;
            if is_probe && runtime.weight <= 0.0 {
                runtime.weight = FORWARD_PROXY_V2_PROBE_RECOVERY_WEIGHT;
            }
        } else {
            runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
            let failure_penalty = (FORWARD_PROXY_V2_WEIGHT_FAILURE_BASE
                + f64::from(runtime.consecutive_failures) * FORWARD_PROXY_V2_WEIGHT_FAILURE_STEP)
                .min(FORWARD_PROXY_V2_WEIGHT_FAILURE_MAX);
            runtime.weight -= failure_penalty;
        }

        runtime.weight = runtime
            .weight
            .clamp(FORWARD_PROXY_V2_WEIGHT_MIN, FORWARD_PROXY_V2_WEIGHT_MAX);

        if success && runtime.weight < FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR {
            runtime.weight = FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR;
        }
    }

    fn should_probe_penalized_proxy(&self) -> bool {
        let selectable_keys = self.selectable_endpoint_keys();
        if selectable_keys.is_empty() {
            return false;
        }
        let has_penalized = self.runtime.values().any(|entry| {
            selectable_keys.contains(entry.proxy_key.as_str()) && entry.is_penalized()
        });
        if !has_penalized || self.probe_in_flight {
            return false;
        }
        self.requests_since_probe >= self.algo.probe_every_requests()
            || (Utc::now() - self.last_probe_at).num_seconds() >= self.algo.probe_interval_secs()
    }

    fn mark_probe_started(&mut self) -> Option<SelectedForwardProxy> {
        if !self.should_probe_penalized_proxy() {
            return None;
        }
        let selectable_keys = self.selectable_endpoint_keys();
        let selected = self
            .runtime
            .values()
            .filter(|entry| {
                entry.is_penalized() && selectable_keys.contains(entry.proxy_key.as_str())
            })
            .max_by(|lhs, rhs| lhs.weight.total_cmp(&rhs.weight))
            .and_then(|entry| {
                self.endpoints
                    .iter()
                    .find(|item| item.key == entry.proxy_key)
            })
            .cloned()?;
        self.probe_in_flight = true;
        self.requests_since_probe = 0;
        self.last_probe_at = Utc::now();
        Some(SelectedForwardProxy::from_endpoint(&selected))
    }

    fn mark_probe_finished(&mut self) {
        self.probe_in_flight = false;
        self.last_probe_at = Utc::now();
    }
}

#[derive(Debug, Clone)]
struct SelectedForwardProxy {
    key: String,
    source: String,
    display_name: String,
    endpoint_url: Option<Url>,
    endpoint_url_raw: Option<String>,
}

impl SelectedForwardProxy {
    fn from_endpoint(endpoint: &ForwardProxyEndpoint) -> Self {
        Self {
            key: endpoint.key.clone(),
            source: endpoint.source.clone(),
            display_name: endpoint.display_name.clone(),
            endpoint_url: endpoint.endpoint_url.clone(),
            endpoint_url_raw: endpoint.raw_url.clone(),
        }
    }
}

#[derive(Debug)]
struct XrayInstance {
    local_proxy_url: Url,
    config_path: PathBuf,
    child: Child,
}

#[derive(Debug, Default)]
struct XraySupervisor {
    binary: String,
    runtime_dir: PathBuf,
    instances: HashMap<String, XrayInstance>,
}

impl XraySupervisor {
    fn new(binary: String, runtime_dir: PathBuf) -> Self {
        Self {
            binary,
            runtime_dir,
            instances: HashMap::new(),
        }
    }

    async fn sync_endpoints(&mut self, endpoints: &mut [ForwardProxyEndpoint]) -> Result<()> {
        fs::create_dir_all(&self.runtime_dir).with_context(|| {
            format!(
                "failed to create xray runtime directory: {}",
                self.runtime_dir.display()
            )
        })?;

        let desired_keys = endpoints
            .iter()
            .filter(|endpoint| endpoint.requires_xray())
            .map(|endpoint| endpoint.key.clone())
            .collect::<HashSet<_>>();
        let stale_keys = self
            .instances
            .keys()
            .filter(|key| !desired_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in stale_keys {
            self.remove_instance(&key).await;
        }

        for endpoint in endpoints {
            if !endpoint.requires_xray() {
                continue;
            }
            match self.ensure_instance(endpoint).await {
                Ok(route_url) => endpoint.endpoint_url = Some(route_url),
                Err(err) => {
                    endpoint.endpoint_url = None;
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        proxy_source = endpoint.source,
                        proxy_label = endpoint.display_name,
                        proxy_url_ref = %forward_proxy_log_ref_option(endpoint.raw_url.as_deref()),
                        error = %err,
                        "failed to prepare xray forward proxy route"
                    );
                }
            }
        }

        Ok(())
    }

    async fn shutdown_all(&mut self) {
        let keys = self.instances.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.remove_instance(&key).await;
        }
    }

    async fn ensure_instance(&mut self, endpoint: &ForwardProxyEndpoint) -> Result<Url> {
        self.ensure_instance_with_ready_timeout(
            endpoint,
            Duration::from_millis(XRAY_PROXY_READY_TIMEOUT_MS),
        )
        .await
    }

    async fn ensure_instance_with_ready_timeout(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        ready_timeout: Duration,
    ) -> Result<Url> {
        if let Some(instance) = self.instances.get_mut(&endpoint.key) {
            match instance.child.try_wait() {
                Ok(None) => return Ok(instance.local_proxy_url.clone()),
                Ok(Some(status)) => {
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        status = %status,
                        "xray proxy process exited unexpectedly; restarting"
                    );
                }
                Err(err) => {
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        error = %err,
                        "failed to inspect xray proxy process; restarting"
                    );
                }
            }
        }

        self.remove_instance(&endpoint.key).await;
        self.spawn_instance(endpoint, ready_timeout).await
    }

    async fn spawn_instance(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        ready_timeout: Duration,
    ) -> Result<Url> {
        let outbound = build_xray_outbound_for_endpoint(endpoint)?;
        let local_port = pick_unused_local_port().context("failed to allocate xray local port")?;
        fs::create_dir_all(&self.runtime_dir).with_context(|| {
            format!(
                "failed to create xray runtime directory: {}",
                self.runtime_dir.display()
            )
        })?;
        let config_path = self.runtime_dir.join(format!(
            "forward-proxy-{:016x}.json",
            stable_hash_u64(&endpoint.key)
        ));
        let config = build_xray_instance_config(local_port, outbound);
        let serialized =
            serde_json::to_vec_pretty(&config).context("failed to serialize xray config")?;
        fs::write(&config_path, serialized)
            .with_context(|| format!("failed to write xray config: {}", config_path.display()))?;

        let mut child = match Command::new(&self.binary)
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let _ = fs::remove_file(&config_path);
                return Err(err)
                    .with_context(|| format!("failed to start xray binary: {}", self.binary));
            }
        };

        if let Err(err) = wait_for_xray_proxy_ready(&mut child, local_port, ready_timeout).await {
            let _ = child.kill().await;
            let _ = child.wait().await;
            let _ = fs::remove_file(&config_path);
            return Err(err);
        }

        let local_proxy_url = Url::parse(&format!("socks5h://127.0.0.1:{local_port}"))
            .context("failed to build local xray socks endpoint")?;
        self.instances.insert(
            endpoint.key.clone(),
            XrayInstance {
                local_proxy_url: local_proxy_url.clone(),
                config_path,
                child,
            },
        );

        Ok(local_proxy_url)
    }

    async fn remove_instance(&mut self, key: &str) {
        if let Some(mut instance) = self.instances.remove(key) {
            let still_running = matches!(instance.child.try_wait(), Ok(None));
            if still_running {
                if let Err(err) = instance.child.kill().await {
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(key),
                        error = %err,
                        "failed to terminate xray proxy process"
                    );
                }
                if let Err(err) = timeout(Duration::from_secs(2), instance.child.wait()).await {
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(key),
                        error = %err,
                        "timed out waiting xray proxy process exit"
                    );
                }
            }
            if let Err(err) = fs::remove_file(&instance.config_path)
                && err.kind() != io::ErrorKind::NotFound
            {
                warn!(
                    proxy_key_ref = %forward_proxy_log_ref(key),
                    path = %instance.config_path.display(),
                    error = %err,
                    "failed to remove xray config file"
                );
            }
        }
    }
}

fn stable_hash_u64(raw: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    hasher.finish()
}

fn forward_proxy_log_ref(raw: &str) -> String {
    format!("fp_{:016x}", stable_hash_u64(raw))
}

fn forward_proxy_log_ref_option(raw: Option<&str>) -> String {
    raw.map(forward_proxy_log_ref)
        .unwrap_or_else(|| "direct".to_string())
}

fn pick_unused_local_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("failed to bind local socket for port allocation")?;
    let port = listener
        .local_addr()
        .context("failed to read local address for allocated port")?
        .port();
    Ok(port)
}

async fn wait_for_xray_proxy_ready(
    child: &mut Child,
    local_port: u16,
    ready_timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + ready_timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .context("failed to poll xray proxy process status")?
        {
            bail!("xray process exited before ready: {status}");
        }
        if timeout(
            Duration::from_millis(250),
            TcpStream::connect(("127.0.0.1", local_port)),
        )
        .await
        .is_ok_and(|result| result.is_ok())
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("xray local socks endpoint was not ready in time");
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn build_xray_instance_config(local_port: u16, outbound: Value) -> Value {
    json!({
        "log": {
            "loglevel": "warning"
        },
        "inbounds": [
            {
                "tag": "inbound-local-socks",
                "listen": "127.0.0.1",
                "port": local_port,
                "protocol": "socks",
                "settings": {
                    "auth": "noauth",
                    "udp": false
                }
            }
        ],
        "outbounds": [
            outbound,
            {
                "tag": "direct",
                "protocol": "freedom"
            }
        ],
        "routing": {
            "domainStrategy": "AsIs",
            "rules": [
                {
                    "type": "field",
                    "inboundTag": ["inbound-local-socks"],
                    "outboundTag": "proxy"
                }
            ]
        }
    })
}

fn build_xray_outbound_for_endpoint(endpoint: &ForwardProxyEndpoint) -> Result<Value> {
    let raw = endpoint
        .raw_url
        .as_deref()
        .ok_or_else(|| anyhow!("xray endpoint missing share link url"))?;
    match endpoint.protocol {
        ForwardProxyProtocol::Vmess => build_vmess_xray_outbound(raw),
        ForwardProxyProtocol::Vless => build_vless_xray_outbound(raw),
        ForwardProxyProtocol::Trojan => build_trojan_xray_outbound(raw),
        ForwardProxyProtocol::Shadowsocks => build_shadowsocks_xray_outbound(raw),
        _ => bail!("unsupported xray protocol for endpoint"),
    }
}

fn build_vmess_xray_outbound(raw: &str) -> Result<Value> {
    let link = parse_vmess_share_link(raw)?;
    let mut outbound = json!({
        "tag": "proxy",
        "protocol": "vmess",
        "settings": {
            "vnext": [
                {
                    "address": link.address,
                    "port": link.port,
                    "users": [
                        {
                            "id": link.id,
                            "alterId": link.alter_id,
                            "security": link.security
                        }
                    ]
                }
            ]
        }
    });
    if let Some(stream_settings) = build_vmess_stream_settings(&link)
        && let Some(object) = outbound.as_object_mut()
    {
        object.insert("streamSettings".to_string(), stream_settings);
    }
    Ok(outbound)
}

fn build_vmess_stream_settings(link: &VmessShareLink) -> Option<Value> {
    let mut stream = serde_json::Map::new();
    stream.insert("network".to_string(), Value::String(link.network.clone()));
    let mut has_non_default_options = link.network != "tcp";

    let security = link
        .tls_mode
        .as_deref()
        .filter(|value| !value.is_empty() && *value != "none")
        .map(|value| value.to_ascii_lowercase());
    if let Some(security) = security.as_ref() {
        stream.insert("security".to_string(), Value::String(security.clone()));
        has_non_default_options = true;
    }

    match link.network.as_str() {
        "ws" => {
            let mut ws = serde_json::Map::new();
            if let Some(path) = link.path.as_ref().filter(|value| !value.trim().is_empty()) {
                ws.insert("path".to_string(), Value::String(path.clone()));
            }
            if let Some(host) = link.host.as_ref().filter(|value| !value.trim().is_empty()) {
                ws.insert("headers".to_string(), json!({ "Host": host }));
            }
            if !ws.is_empty() {
                stream.insert("wsSettings".to_string(), Value::Object(ws));
                has_non_default_options = true;
            }
        }
        "grpc" => {
            let service_name = link
                .path
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .unwrap_or_default();
            stream.insert(
                "grpcSettings".to_string(),
                json!({
                    "serviceName": service_name
                }),
            );
            has_non_default_options = true;
        }
        "httpupgrade" => {
            let mut settings = serde_json::Map::new();
            if let Some(host) = link.host.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("host".to_string(), Value::String(host.clone()));
            }
            if let Some(path) = link.path.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("path".to_string(), Value::String(path.clone()));
            }
            if !settings.is_empty() {
                stream.insert("httpupgradeSettings".to_string(), Value::Object(settings));
                has_non_default_options = true;
            }
        }
        _ => {}
    }

    if let Some(security) = security {
        if security == "tls" {
            let mut tls_settings = serde_json::Map::new();
            if let Some(server_name) = link
                .sni
                .as_ref()
                .or(link.host.as_ref())
                .filter(|value| !value.trim().is_empty())
            {
                tls_settings.insert("serverName".to_string(), Value::String(server_name.clone()));
            }
            if let Some(alpn) = link.alpn.as_ref().filter(|items| !items.is_empty()) {
                tls_settings.insert("alpn".to_string(), json!(alpn));
            }
            if let Some(fingerprint) = link
                .fingerprint
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                tls_settings.insert(
                    "fingerprint".to_string(),
                    Value::String(fingerprint.clone()),
                );
            }
            if !tls_settings.is_empty() {
                stream.insert("tlsSettings".to_string(), Value::Object(tls_settings));
                has_non_default_options = true;
            }
        } else if security == "reality" {
            let mut reality_settings = serde_json::Map::new();
            if let Some(server_name) = link
                .sni
                .as_ref()
                .or(link.host.as_ref())
                .filter(|value| !value.trim().is_empty())
            {
                reality_settings
                    .insert("serverName".to_string(), Value::String(server_name.clone()));
            }
            if let Some(fingerprint) = link
                .fingerprint
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                reality_settings.insert(
                    "fingerprint".to_string(),
                    Value::String(fingerprint.clone()),
                );
            }
            if !reality_settings.is_empty() {
                stream.insert(
                    "realitySettings".to_string(),
                    Value::Object(reality_settings),
                );
                has_non_default_options = true;
            }
        }
    }

    if has_non_default_options {
        Some(Value::Object(stream))
    } else {
        None
    }
}

fn build_vless_xray_outbound(raw: &str) -> Result<Value> {
    let url = Url::parse(raw).context("invalid vless share link")?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("vless host missing"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("vless port missing"))?;
    let user_id = url.username();
    if user_id.trim().is_empty() {
        bail!("vless id missing");
    }

    let query = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
    let encryption = query
        .get("encryption")
        .cloned()
        .unwrap_or_else(|| "none".to_string());
    let mut user = serde_json::Map::new();
    user.insert("id".to_string(), Value::String(user_id.to_string()));
    user.insert("encryption".to_string(), Value::String(encryption));
    if let Some(flow) = query.get("flow").filter(|value| !value.trim().is_empty()) {
        user.insert("flow".to_string(), Value::String(flow.clone()));
    }

    let mut outbound = json!({
        "tag": "proxy",
        "protocol": "vless",
        "settings": {
            "vnext": [
                {
                    "address": host,
                    "port": port,
                    "users": [Value::Object(user)]
                }
            ]
        }
    });
    if let Some(stream_settings) = build_stream_settings_from_url(&url, None)
        && let Some(object) = outbound.as_object_mut()
    {
        object.insert("streamSettings".to_string(), stream_settings);
    }
    Ok(outbound)
}

fn build_trojan_xray_outbound(raw: &str) -> Result<Value> {
    let url = Url::parse(raw).context("invalid trojan share link")?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("trojan host missing"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("trojan port missing"))?;
    let password = url.username();
    if password.trim().is_empty() {
        bail!("trojan password missing");
    }

    let mut outbound = json!({
        "tag": "proxy",
        "protocol": "trojan",
        "settings": {
            "servers": [
                {
                    "address": host,
                    "port": port,
                    "password": password
                }
            ]
        }
    });
    if let Some(stream_settings) = build_stream_settings_from_url(&url, Some("tls"))
        && let Some(object) = outbound.as_object_mut()
    {
        object.insert("streamSettings".to_string(), stream_settings);
    }
    Ok(outbound)
}

fn build_shadowsocks_xray_outbound(raw: &str) -> Result<Value> {
    let parsed = parse_shadowsocks_share_link(raw)?;
    Ok(json!({
        "tag": "proxy",
        "protocol": "shadowsocks",
        "settings": {
            "servers": [
                {
                    "address": parsed.host,
                    "port": parsed.port,
                    "method": parsed.method,
                    "password": parsed.password
                }
            ]
        }
    }))
}

fn build_stream_settings_from_url(url: &Url, default_security: Option<&str>) -> Option<Value> {
    let query = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
    let network = query
        .get("type")
        .or_else(|| query.get("net"))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "tcp".to_string());
    let security = query
        .get("security")
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .or_else(|| default_security.map(str::to_string))
        .unwrap_or_else(|| "none".to_string());

    let host = query
        .get("host")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let path = query
        .get("path")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let service_name = query
        .get("serviceName")
        .or_else(|| query.get("service_name"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| path.clone());

    let mut stream = serde_json::Map::new();
    stream.insert("network".to_string(), Value::String(network.clone()));
    let mut has_non_default_options = network != "tcp";
    if security != "none" {
        stream.insert("security".to_string(), Value::String(security.clone()));
        has_non_default_options = true;
    }

    match network.as_str() {
        "ws" => {
            let mut ws = serde_json::Map::new();
            if let Some(path) = path.as_ref() {
                ws.insert("path".to_string(), Value::String(path.clone()));
            }
            if let Some(host) = host.as_ref() {
                ws.insert("headers".to_string(), json!({ "Host": host }));
            }
            if !ws.is_empty() {
                stream.insert("wsSettings".to_string(), Value::Object(ws));
                has_non_default_options = true;
            }
        }
        "grpc" => {
            let service_name = service_name.unwrap_or_default();
            stream.insert(
                "grpcSettings".to_string(),
                json!({
                    "serviceName": service_name,
                    "multiMode": query_flag_true(&query, "multiMode")
                }),
            );
            has_non_default_options = true;
        }
        "httpupgrade" => {
            let mut settings = serde_json::Map::new();
            if let Some(host) = host.as_ref() {
                settings.insert("host".to_string(), Value::String(host.clone()));
            }
            if let Some(path) = path.as_ref() {
                settings.insert("path".to_string(), Value::String(path.clone()));
            }
            if !settings.is_empty() {
                stream.insert("httpupgradeSettings".to_string(), Value::Object(settings));
                has_non_default_options = true;
            }
        }
        _ => {}
    }

    if security == "tls" {
        let mut tls_settings = serde_json::Map::new();
        if let Some(server_name) = query
            .get("sni")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| host.clone())
            .or_else(|| url.host_str().map(str::to_string))
        {
            tls_settings.insert("serverName".to_string(), Value::String(server_name));
        }
        if query_flag_true(&query, "allowInsecure") || query_flag_true(&query, "insecure") {
            tls_settings.insert("allowInsecure".to_string(), Value::Bool(true));
        }
        if let Some(fingerprint) = query
            .get("fp")
            .or_else(|| query.get("fingerprint"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            tls_settings.insert("fingerprint".to_string(), Value::String(fingerprint));
        }
        if let Some(alpn) = query
            .get("alpn")
            .map(|value| parse_alpn_csv(value))
            .filter(|items| !items.is_empty())
        {
            tls_settings.insert("alpn".to_string(), json!(alpn));
        }
        if !tls_settings.is_empty() {
            stream.insert("tlsSettings".to_string(), Value::Object(tls_settings));
            has_non_default_options = true;
        }
    } else if security == "reality" {
        let mut reality_settings = serde_json::Map::new();
        if let Some(server_name) = query
            .get("sni")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| host.clone())
            .or_else(|| url.host_str().map(str::to_string))
        {
            reality_settings.insert("serverName".to_string(), Value::String(server_name));
        }
        if let Some(fingerprint) = query
            .get("fp")
            .or_else(|| query.get("fingerprint"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("fingerprint".to_string(), Value::String(fingerprint));
        }
        if let Some(public_key) = query
            .get("pbk")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("publicKey".to_string(), Value::String(public_key));
        }
        if let Some(short_id) = query
            .get("sid")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("shortId".to_string(), Value::String(short_id));
        }
        if let Some(spider_x) = query
            .get("spx")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("spiderX".to_string(), Value::String(spider_x));
        }
        if !reality_settings.is_empty() {
            stream.insert(
                "realitySettings".to_string(),
                Value::Object(reality_settings),
            );
            has_non_default_options = true;
        }
    }

    if has_non_default_options {
        Some(Value::Object(stream))
    } else {
        None
    }
}

fn query_flag_true(query: &HashMap<String, String>, key: &str) -> bool {
    query.get(key).is_some_and(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[derive(Debug, Clone)]
struct ForwardProxyAttemptWindowStats {
    attempts: i64,
    success_count: i64,
    avg_latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ForwardProxyWindowStatsResponse {
    attempts: i64,
    success_rate: Option<f64>,
    avg_latency_ms: Option<f64>,
}

impl From<ForwardProxyAttemptWindowStats> for ForwardProxyWindowStatsResponse {
    fn from(value: ForwardProxyAttemptWindowStats) -> Self {
        let success_rate = if value.attempts > 0 {
            Some((value.success_count as f64) / (value.attempts as f64))
        } else {
            None
        };
        Self {
            attempts: value.attempts,
            success_rate,
            avg_latency_ms: value.avg_latency_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct ForwardProxyStatsResponse {
    one_minute: ForwardProxyWindowStatsResponse,
    fifteen_minutes: ForwardProxyWindowStatsResponse,
    one_hour: ForwardProxyWindowStatsResponse,
    one_day: ForwardProxyWindowStatsResponse,
    seven_days: ForwardProxyWindowStatsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxyNodeResponse {
    key: String,
    source: String,
    display_name: String,
    endpoint_url: Option<String>,
    weight: f64,
    penalized: bool,
    stats: ForwardProxyStatsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxySettingsResponse {
    proxy_urls: Vec<String>,
    subscription_urls: Vec<String>,
    subscription_update_interval_secs: u64,
    insert_direct: bool,
    nodes: Vec<ForwardProxyNodeResponse>,
}

#[derive(Debug, Clone, Default)]
struct ForwardProxyHourlyStatsPoint {
    success_count: i64,
    failure_count: i64,
}

#[derive(Debug, Clone)]
struct ForwardProxyWeightHourlyStatsPoint {
    sample_count: i64,
    min_weight: f64,
    max_weight: f64,
    avg_weight: f64,
    last_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxyHourlyBucketResponse {
    bucket_start: String,
    bucket_end: String,
    success_count: i64,
    failure_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxyWeightHourlyBucketResponse {
    bucket_start: String,
    bucket_end: String,
    sample_count: i64,
    min_weight: f64,
    max_weight: f64,
    avg_weight: f64,
    last_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxyLiveNodeResponse {
    key: String,
    source: String,
    display_name: String,
    endpoint_url: Option<String>,
    weight: f64,
    penalized: bool,
    stats: ForwardProxyStatsResponse,
    last24h: Vec<ForwardProxyHourlyBucketResponse>,
    weight24h: Vec<ForwardProxyWeightHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ForwardProxyLiveStatsResponse {
    range_start: String,
    range_end: String,
    bucket_seconds: i64,
    nodes: Vec<ForwardProxyLiveNodeResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptCacheConversationsResponse {
    range_start: String,
    range_end: String,
    conversations: Vec<PromptCacheConversationResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptCacheConversationResponse {
    prompt_cache_key: String,
    request_count: i64,
    total_tokens: i64,
    total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    created_at: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    last_activity_at: String,
    last24h_requests: Vec<PromptCacheConversationRequestPointResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PromptCacheConversationRequestPointResponse {
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    occurred_at: String,
    status: String,
    is_success: bool,
    request_tokens: i64,
    cumulative_tokens: i64,
}

#[derive(Debug, Clone)]
struct PromptCacheConversationsCacheEntry {
    cached_at: Instant,
    response: PromptCacheConversationsResponse,
}

#[derive(Debug)]
struct PromptCacheConversationInFlight {
    signal: watch::Sender<bool>,
}

#[derive(Debug, Default)]
struct PromptCacheConversationsCacheState {
    entries: HashMap<i64, PromptCacheConversationsCacheEntry>,
    in_flight: HashMap<i64, PromptCacheConversationInFlight>,
}

#[derive(Debug)]
struct PromptCacheConversationFlightGuard {
    cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    limit: i64,
    active: bool,
}

impl PromptCacheConversationFlightGuard {
    fn new(cache: Arc<Mutex<PromptCacheConversationsCacheState>>, limit: i64) -> Self {
        Self {
            cache,
            limit,
            active: true,
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PromptCacheConversationFlightGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let cache = self.cache.clone();
        let limit = self.limit;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut state = cache.lock().await;
                if let Some(in_flight) = state.in_flight.remove(&limit) {
                    let _ = in_flight.signal.send(true);
                }
            });
            return;
        }

        if let Ok(mut state) = cache.try_lock()
            && let Some(in_flight) = state.in_flight.remove(&limit)
        {
            let _ = in_flight.signal.send(true);
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ParsedUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    total_tokens: Option<i64>,
}

#[derive(Debug, Clone, Default)]
struct RawPayloadMeta {
    path: Option<String>,
    size_bytes: i64,
    truncated: bool,
    truncated_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RequestCaptureInfo {
    model: Option<String>,
    prompt_cache_key: Option<String>,
    requested_service_tier: Option<String>,
    reasoning_effort: Option<String>,
    is_stream: bool,
    parse_error: Option<String>,
}

#[derive(Debug, Clone)]
struct ResponseCaptureInfo {
    model: Option<String>,
    usage: ParsedUsage,
    usage_missing_reason: Option<String>,
    service_tier: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct StageTimings {
    t_total_ms: f64,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
    t_upstream_connect_ms: f64,
    t_upstream_ttfb_ms: f64,
    t_upstream_stream_ms: f64,
    t_resp_parse_ms: f64,
    t_persist_ms: f64,
}

#[derive(Debug, Clone)]
struct ProxyCaptureRecord {
    invoke_id: String,
    occurred_at: String,
    model: Option<String>,
    usage: ParsedUsage,
    cost: Option<f64>,
    cost_estimated: bool,
    price_version: Option<String>,
    status: String,
    error_message: Option<String>,
    payload: Option<String>,
    raw_response: String,
    req_raw: RawPayloadMeta,
    resp_raw: RawPayloadMeta,
    raw_expires_at: Option<String>,
    timings: StageTimings,
}

#[derive(Debug)]
struct RequestBodyReadError {
    status: StatusCode,
    message: String,
    failure_kind: &'static str,
    partial_body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProxyCaptureTarget {
    ChatCompletions,
    Responses,
    ResponsesCompact,
}

impl ProxyCaptureTarget {
    fn endpoint(self) -> &'static str {
        match self {
            Self::ChatCompletions => "/v1/chat/completions",
            Self::Responses => "/v1/responses",
            Self::ResponsesCompact => "/v1/responses/compact",
        }
    }

    fn allows_fast_mode_rewrite(self) -> bool {
        matches!(self, Self::ChatCompletions | Self::Responses)
    }

    fn should_auto_include_usage(self) -> bool {
        matches!(self, Self::ChatCompletions)
    }

    fn from_endpoint(endpoint: &str) -> Self {
        match endpoint {
            "/v1/chat/completions" => Self::ChatCompletions,
            "/v1/responses/compact" => Self::ResponsesCompact,
            "/v1/responses" => Self::Responses,
            _ => Self::Responses,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvocationSourceScope {
    ProxyOnly,
    All,
}

#[derive(Debug, Default, Clone, Copy)]
struct ProxyUsageBackfillSummary {
    scanned: u64,
    updated: u64,
    skipped_missing_file: u64,
    skipped_without_usage: u64,
    skipped_decode_error: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct ProxyCostBackfillSummary {
    scanned: u64,
    updated: u64,
    skipped_unpriced_model: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct ProxyPromptCacheKeyBackfillSummary {
    scanned: u64,
    updated: u64,
    skipped_missing_file: u64,
    skipped_invalid_json: u64,
    skipped_missing_key: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct ProxyRequestedServiceTierBackfillSummary {
    scanned: u64,
    updated: u64,
    skipped_missing_file: u64,
    skipped_invalid_json: u64,
    skipped_missing_tier: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct InvocationServiceTierBackfillSummary {
    scanned: u64,
    updated: u64,
    skipped_missing_file: u64,
    skipped_missing_tier: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct ProxyReasoningEffortBackfillSummary {
    scanned: u64,
    updated: u64,
    skipped_missing_file: u64,
    skipped_invalid_json: u64,
    skipped_missing_effort: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct FailureClassificationBackfillSummary {
    scanned: u64,
    updated: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureClass {
    None,
    ServiceFailure,
    ClientFailure,
    ClientAbort,
}

impl FailureClass {
    fn as_str(self) -> &'static str {
        match self {
            FailureClass::None => FAILURE_CLASS_NONE,
            FailureClass::ServiceFailure => FAILURE_CLASS_SERVICE,
            FailureClass::ClientFailure => FAILURE_CLASS_CLIENT,
            FailureClass::ClientAbort => FAILURE_CLASS_ABORT,
        }
    }

    fn from_db_str(raw: &str) -> Option<Self> {
        match raw {
            FAILURE_CLASS_NONE => Some(FailureClass::None),
            FAILURE_CLASS_SERVICE => Some(FailureClass::ServiceFailure),
            FAILURE_CLASS_CLIENT => Some(FailureClass::ClientFailure),
            FAILURE_CLASS_ABORT => Some(FailureClass::ClientAbort),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct FailureClassification {
    failure_kind: Option<String>,
    failure_class: FailureClass,
    is_actionable: bool,
}

#[derive(Debug, FromRow)]
struct ProxyUsageBackfillCandidate {
    id: i64,
    response_raw_path: String,
    payload: Option<String>,
}

#[derive(Debug, FromRow)]
struct ProxyCostBackfillCandidate {
    id: i64,
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    total_tokens: Option<i64>,
}

#[derive(Debug, FromRow)]
struct ProxyPromptCacheKeyBackfillCandidate {
    id: i64,
    request_raw_path: String,
}

#[derive(Debug, FromRow)]
struct ProxyRequestedServiceTierBackfillCandidate {
    id: i64,
    request_raw_path: String,
}

#[derive(Debug, FromRow)]
struct ProxyReasoningEffortBackfillCandidate {
    id: i64,
    request_raw_path: String,
}

#[derive(Debug)]
struct ProxyUsageBackfillUpdate {
    id: i64,
    usage: ParsedUsage,
}

#[derive(Debug)]
struct ProxyCostBackfillUpdate {
    id: i64,
    cost: Option<f64>,
    cost_estimated: bool,
    price_version: Option<String>,
}

#[derive(Debug, FromRow)]
struct PromptCacheConversationAggregateRow {
    prompt_cache_key: String,
    request_count: i64,
    total_tokens: i64,
    total_cost: f64,
    created_at: String,
    last_activity_at: String,
}

#[derive(Debug, FromRow)]
struct PromptCacheConversationEventRow {
    occurred_at: String,
    status: String,
    request_tokens: i64,
    prompt_cache_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    limit: Option<i64>,
    model: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PromptCacheConversationsQuery {
    limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyModelSettings {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    fast_mode_rewrite_mode: ProxyFastModeRewriteMode,
    enabled_preset_models: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProxyFastModeRewriteMode {
    #[default]
    Disabled,
    FillMissing,
    ForcePriority,
}

impl ProxyFastModeRewriteMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::FillMissing => "fill_missing",
            Self::ForcePriority => "force_priority",
        }
    }
}

fn decode_proxy_fast_mode_rewrite_mode(raw: Option<&str>) -> ProxyFastModeRewriteMode {
    match raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("fill_missing") => ProxyFastModeRewriteMode::FillMissing,
        Some("force_priority") => ProxyFastModeRewriteMode::ForcePriority,
        _ => DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
    }
}

impl Default for ProxyModelSettings {
    fn default() -> Self {
        Self {
            hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            merge_upstream_enabled: DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_preset_models: default_enabled_preset_models(),
        }
    }
}

impl ProxyModelSettings {
    fn normalized(self) -> Self {
        let merge_upstream_enabled = if self.hijack_enabled {
            self.merge_upstream_enabled
        } else {
            false
        };
        Self {
            hijack_enabled: self.hijack_enabled,
            merge_upstream_enabled,
            fast_mode_rewrite_mode: self.fast_mode_rewrite_mode,
            enabled_preset_models: normalize_enabled_preset_models(self.enabled_preset_models),
        }
    }
}

#[derive(Debug, FromRow)]
struct ProxyModelSettingsRow {
    hijack_enabled: i64,
    merge_upstream_enabled: i64,
    fast_mode_rewrite_mode: Option<String>,
    enabled_preset_models_json: Option<String>,
}

impl From<ProxyModelSettingsRow> for ProxyModelSettings {
    fn from(value: ProxyModelSettingsRow) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled != 0,
            merge_upstream_enabled: value.merge_upstream_enabled != 0,
            fast_mode_rewrite_mode: decode_proxy_fast_mode_rewrite_mode(
                value.fast_mode_rewrite_mode.as_deref(),
            ),
            enabled_preset_models: decode_enabled_preset_models(
                value.enabled_preset_models_json.as_deref(),
            ),
        }
        .normalized()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyModelSettingsUpdateRequest {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    #[serde(default)]
    fast_mode_rewrite_mode: ProxyFastModeRewriteMode,
    #[serde(default = "default_enabled_preset_models")]
    enabled_models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyModelSettingsResponse {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    fast_mode_rewrite_mode: ProxyFastModeRewriteMode,
    default_hijack_enabled: bool,
    models: Vec<String>,
    enabled_models: Vec<String>,
}

impl From<ProxyModelSettings> for ProxyModelSettingsResponse {
    fn from(value: ProxyModelSettings) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled,
            merge_upstream_enabled: value.merge_upstream_enabled,
            fast_mode_rewrite_mode: value.fast_mode_rewrite_mode,
            default_hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            models: PROXY_PRESET_MODEL_IDS
                .iter()
                .map(|model| (*model).to_string())
                .collect(),
            enabled_models: value.enabled_preset_models,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsResponse {
    proxy: ProxyModelSettingsResponse,
    forward_proxy: ForwardProxySettingsResponse,
    pricing: PricingSettingsResponse,
}

fn default_enabled_preset_models() -> Vec<String> {
    PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|model| (*model).to_string())
        .collect()
}

fn normalize_enabled_preset_models(enabled_models: Vec<String>) -> Vec<String> {
    let enabled_set: HashSet<&str> = enabled_models.iter().map(String::as_str).collect();
    PROXY_PRESET_MODEL_IDS
        .iter()
        .filter(|model| enabled_set.contains(**model))
        .map(|model| (*model).to_string())
        .collect()
}

fn decode_enabled_preset_models(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(serialized) => serde_json::from_str::<Vec<String>>(serialized)
            .map(normalize_enabled_preset_models)
            .unwrap_or_else(|_| default_enabled_preset_models()),
        None => default_enabled_preset_models(),
    }
}

fn default_forward_proxy_subscription_interval_secs() -> u64 {
    DEFAULT_FORWARD_PROXY_SUBSCRIPTION_INTERVAL_SECS
}

fn default_forward_proxy_insert_direct() -> bool {
    DEFAULT_FORWARD_PROXY_INSERT_DIRECT
}

fn decode_string_vec_json(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(serialized) => serde_json::from_str::<Vec<String>>(serialized).unwrap_or_default(),
        None => Vec::new(),
    }
}

fn normalize_subscription_entries(raw_entries: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for entry in raw_entries {
        for token in split_proxy_entry_tokens(&entry) {
            let Ok(url) = Url::parse(token) else {
                continue;
            };
            if !matches!(url.scheme(), "http" | "https") {
                continue;
            }
            let canonical = url.to_string();
            if seen.insert(canonical.clone()) {
                normalized.push(canonical);
            }
        }
    }
    normalized
}

fn normalize_proxy_url_entries(raw_entries: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for entry in raw_entries {
        for token in split_proxy_entry_tokens(&entry) {
            if let Some(parsed) = parse_forward_proxy_entry(token)
                && seen.insert(parsed.normalized.clone())
            {
                normalized.push(parsed.normalized);
            }
        }
    }
    normalized
}

fn split_proxy_entry_tokens(raw: &str) -> Vec<&str> {
    raw.split(['\n', ',', ';'])
        .map(str::trim)
        .filter(|token| !token.is_empty() && !token.starts_with('#'))
        .collect()
}

#[cfg(test)]
fn normalize_single_proxy_url(raw: &str) -> Option<String> {
    parse_forward_proxy_entry(raw).map(|entry| entry.normalized)
}

fn normalize_proxy_endpoints_from_urls(urls: &[String], source: &str) -> Vec<ForwardProxyEndpoint> {
    let mut seen = HashSet::new();
    let mut endpoints = Vec::new();
    for raw in urls {
        if let Some(parsed) = parse_forward_proxy_entry(raw) {
            let key = parsed.normalized.clone();
            if !seen.insert(key.clone()) {
                continue;
            }
            endpoints.push(ForwardProxyEndpoint {
                key,
                source: source.to_string(),
                display_name: parsed.display_name,
                protocol: parsed.protocol,
                endpoint_url: parsed.endpoint_url,
                raw_url: Some(parsed.normalized),
            });
        }
    }
    endpoints
}

#[derive(Debug, Clone)]
struct ParsedForwardProxyEntry {
    normalized: String,
    display_name: String,
    protocol: ForwardProxyProtocol,
    endpoint_url: Option<Url>,
}

fn parse_forward_proxy_entry(raw: &str) -> Option<ParsedForwardProxyEntry> {
    let candidate = raw.trim();
    if candidate.is_empty() {
        return None;
    }

    if !candidate.contains("://") {
        return parse_native_forward_proxy(&format!("http://{candidate}"));
    }

    let (scheme_raw, _) = candidate.split_once("://")?;
    let scheme = scheme_raw.to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" | "socks5" | "socks5h" | "socks" => parse_native_forward_proxy(candidate),
        "vmess" => parse_vmess_forward_proxy(candidate),
        "vless" => parse_vless_forward_proxy(candidate),
        "trojan" => parse_trojan_forward_proxy(candidate),
        "ss" => parse_shadowsocks_forward_proxy(candidate),
        _ => None,
    }
}

fn parse_native_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let parsed = Url::parse(candidate).ok()?;
    let raw_scheme = parsed.scheme();
    let (protocol, normalized_scheme) = match raw_scheme {
        "http" => (ForwardProxyProtocol::Http, "http"),
        "https" => (ForwardProxyProtocol::Https, "https"),
        "socks5" | "socks" => (ForwardProxyProtocol::Socks5, "socks5"),
        "socks5h" => (ForwardProxyProtocol::Socks5h, "socks5h"),
        _ => return None,
    };

    let host = parsed.host_str()?;
    if host.trim().is_empty() {
        return None;
    }
    let port = parsed.port_or_known_default()?;
    let mut normalized = format!("{normalized_scheme}://");
    if !parsed.username().is_empty() {
        normalized.push_str(parsed.username());
        if let Some(password) = parsed.password() {
            normalized.push(':');
            normalized.push_str(password);
        }
        normalized.push('@');
    }
    if host.contains(':') {
        normalized.push('[');
        normalized.push_str(host);
        normalized.push(']');
    } else {
        normalized.push_str(&host.to_ascii_lowercase());
    }
    normalized.push(':');
    normalized.push_str(&port.to_string());
    let endpoint_url = Url::parse(&normalized).ok()?;
    Some(ParsedForwardProxyEntry {
        normalized,
        display_name: format!("{host}:{port}"),
        protocol,
        endpoint_url: Some(endpoint_url),
    })
}

fn parse_vmess_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "vmess")?;
    let parsed = parse_vmess_share_link(&normalized).ok()?;
    Some(ParsedForwardProxyEntry {
        normalized,
        display_name: parsed.display_name,
        protocol: ForwardProxyProtocol::Vmess,
        endpoint_url: None,
    })
}

fn parse_vless_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "vless")?;
    let parsed = Url::parse(&normalized).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    let display_name =
        proxy_display_name_from_url(&parsed).unwrap_or_else(|| format!("{host}:{port}"));
    Some(ParsedForwardProxyEntry {
        normalized,
        display_name,
        protocol: ForwardProxyProtocol::Vless,
        endpoint_url: None,
    })
}

fn parse_trojan_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "trojan")?;
    let parsed = Url::parse(&normalized).ok()?;
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    let display_name =
        proxy_display_name_from_url(&parsed).unwrap_or_else(|| format!("{host}:{port}"));
    Some(ParsedForwardProxyEntry {
        normalized,
        display_name,
        protocol: ForwardProxyProtocol::Trojan,
        endpoint_url: None,
    })
}

fn parse_shadowsocks_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "ss")?;
    let parsed = parse_shadowsocks_share_link(&normalized).ok()?;
    Some(ParsedForwardProxyEntry {
        normalized,
        display_name: parsed.display_name,
        protocol: ForwardProxyProtocol::Shadowsocks,
        endpoint_url: None,
    })
}

fn proxy_display_name_from_url(url: &Url) -> Option<String> {
    if let Some(fragment) = url.fragment()
        && !fragment.trim().is_empty()
    {
        return Some(fragment.to_string());
    }
    let host = url.host_str()?;
    let port = url.port_or_known_default()?;
    Some(format!("{host}:{port}"))
}

fn normalize_share_link_scheme(candidate: &str, scheme: &str) -> Option<String> {
    let (_, remainder) = candidate.split_once("://")?;
    let normalized = format!("{scheme}://{}", remainder.trim());
    if normalized.len() <= scheme.len() + 3 {
        return None;
    }
    Some(normalized)
}

fn decode_base64_any(raw: &str) -> Option<Vec<u8>> {
    let compact = raw
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    if compact.is_empty() {
        return None;
    }
    for engine in [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ] {
        if let Ok(decoded) = engine.decode(compact.as_bytes()) {
            return Some(decoded);
        }
    }
    None
}

fn decode_base64_string(raw: &str) -> Option<String> {
    decode_base64_any(raw).and_then(|bytes| String::from_utf8(bytes).ok())
}

#[derive(Debug, Clone)]
struct VmessShareLink {
    address: String,
    port: u16,
    id: String,
    alter_id: u32,
    security: String,
    network: String,
    host: Option<String>,
    path: Option<String>,
    tls_mode: Option<String>,
    sni: Option<String>,
    alpn: Option<Vec<String>>,
    fingerprint: Option<String>,
    display_name: String,
}

fn parse_vmess_share_link(raw: &str) -> Result<VmessShareLink> {
    let payload = raw
        .strip_prefix("vmess://")
        .ok_or_else(|| anyhow!("invalid vmess share link"))?;
    let decoded =
        decode_base64_string(payload).ok_or_else(|| anyhow!("failed to decode vmess payload"))?;
    let value: Value = serde_json::from_str(&decoded).context("invalid vmess json payload")?;

    let address = value
        .get("add")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("vmess payload missing add"))?
        .to_string();
    let port =
        parse_port_value(value.get("port")).ok_or_else(|| anyhow!("vmess payload missing port"))?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("vmess payload missing id"))?
        .to_string();
    let alter_id = parse_u32_value(value.get("aid")).unwrap_or(0);
    let security = value
        .get("scy")
        .and_then(Value::as_str)
        .or_else(|| value.get("security").and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("auto")
        .to_string();
    let network = value
        .get("net")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("tcp")
        .to_ascii_lowercase();
    let host = value
        .get("host")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let tls_mode = value
        .get("tls")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    let sni = value
        .get("sni")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let alpn = value
        .get("alpn")
        .and_then(Value::as_str)
        .map(parse_alpn_csv)
        .filter(|items| !items.is_empty());
    let fingerprint = value
        .get("fp")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let display_name = value
        .get("ps")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("{address}:{port}"));

    Ok(VmessShareLink {
        address,
        port,
        id,
        alter_id,
        security,
        network,
        host,
        path,
        tls_mode,
        sni,
        alpn,
        fingerprint,
        display_name,
    })
}

fn parse_u32_value(value: Option<&Value>) -> Option<u32> {
    match value {
        Some(Value::Number(num)) => num.as_u64().and_then(|v| u32::try_from(v).ok()),
        Some(Value::String(raw)) => raw.trim().parse::<u32>().ok(),
        _ => None,
    }
}

fn parse_port_value(value: Option<&Value>) -> Option<u16> {
    match value {
        Some(Value::Number(num)) => num.as_u64().and_then(|v| u16::try_from(v).ok()),
        Some(Value::String(raw)) => raw.trim().parse::<u16>().ok(),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct ShadowsocksShareLink {
    method: String,
    password: String,
    host: String,
    port: u16,
    display_name: String,
}

fn parse_shadowsocks_share_link(raw: &str) -> Result<ShadowsocksShareLink> {
    let normalized = raw
        .strip_prefix("ss://")
        .ok_or_else(|| anyhow!("invalid shadowsocks share link"))?;
    let (main, fragment) = split_once_first(normalized, '#');
    let (main, _) = split_once_first(main, '?');
    let display_name = fragment
        .map(percent_decode_once_lossy)
        .filter(|value| !value.trim().is_empty());

    if let Ok(url) = Url::parse(raw)
        && let Some(host) = url.host_str()
        && let Some(port) = url.port_or_known_default()
    {
        let credentials = if !url.username().is_empty() && url.password().is_some() {
            Some((
                percent_decode_once_lossy(url.username()),
                percent_decode_once_lossy(url.password().unwrap_or_default()),
            ))
        } else if !url.username().is_empty() {
            let username = percent_decode_once_lossy(url.username());
            decode_base64_string(&username).and_then(|decoded| {
                let (method, password) = decoded.split_once(':')?;
                Some((method.to_string(), password.to_string()))
            })
        } else {
            None
        };
        if let Some((method, password)) = credentials {
            return Ok(ShadowsocksShareLink {
                method,
                password,
                host: host.to_string(),
                port,
                display_name: display_name
                    .clone()
                    .unwrap_or_else(|| format!("{host}:{port}")),
            });
        }
    }

    let decoded_main = if main.contains('@') {
        main.to_string()
    } else {
        let main_for_decode = percent_decode_once_lossy(main);
        decode_base64_string(&main_for_decode)
            .ok_or_else(|| anyhow!("failed to decode shadowsocks payload"))?
    };

    let (credential, host_port) = decoded_main
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("invalid shadowsocks payload"))?;
    let (method, password) = if let Some((method, password)) = credential.split_once(':') {
        (
            percent_decode_once_lossy(method),
            percent_decode_once_lossy(password),
        )
    } else {
        let decoded_credential = decode_base64_string(credential)
            .ok_or_else(|| anyhow!("failed to decode shadowsocks credentials"))?;
        let (method, password) = decoded_credential
            .split_once(':')
            .ok_or_else(|| anyhow!("invalid shadowsocks credentials"))?;
        (
            percent_decode_once_lossy(method),
            percent_decode_once_lossy(password),
        )
    };
    let parsed_host = Url::parse(&format!("http://{host_port}"))
        .context("invalid shadowsocks server endpoint")?;
    let host = parsed_host
        .host_str()
        .ok_or_else(|| anyhow!("shadowsocks host missing"))?
        .to_string();
    let port = parsed_host
        .port_or_known_default()
        .ok_or_else(|| anyhow!("shadowsocks port missing"))?;
    let display_name = display_name.unwrap_or_else(|| format!("{host}:{port}"));
    Ok(ShadowsocksShareLink {
        method,
        password,
        host,
        port,
        display_name,
    })
}

fn split_once_first(raw: &str, delimiter: char) -> (&str, Option<&str>) {
    if let Some((lhs, rhs)) = raw.split_once(delimiter) {
        (lhs, Some(rhs))
    } else {
        (raw, None)
    }
}

fn parse_alpn_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn deterministic_unit_f64(seed: u64) -> f64 {
    let mut value = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51afd7ed558ccd);
    value ^= value >> 33;
    value = value.wrapping_mul(0xc4ceb9fe1a85ec53);
    value ^= value >> 33;
    (value as f64) / (u64::MAX as f64)
}

fn default_pricing_source_custom() -> String {
    "custom".to_string()
}

fn normalize_pricing_catalog_version(raw: String) -> Option<String> {
    let normalized = raw.trim().to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn normalize_pricing_source(raw: String) -> String {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        default_pricing_source_custom()
    } else {
        normalized
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SummaryQuery {
    window: Option<String>,
    limit: Option<i64>,
    time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimeseriesQuery {
    #[serde(default = "default_range")]
    range: String,
    bucket: Option<String>,
    #[allow(dead_code)]
    settlement_hour: Option<u8>,
    time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PerfQuery {
    #[serde(default = "default_range")]
    range: String,
    time_zone: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PerfStatsResponse {
    range_start: String,
    range_end: String,
    source: String,
    stages: Vec<PerfStageStats>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PerfStageStats {
    stage: String,
    count: i64,
    avg_ms: f64,
    p50_ms: f64,
    p90_ms: f64,
    p99_ms: f64,
    max_ms: f64,
}

#[derive(Debug)]
enum SummaryWindow {
    All,
    Current(i64),
    Duration(ChronoDuration),
    Calendar(String),
}

#[derive(Debug, Clone)]
enum StatsFilter {
    All,
    Since(DateTime<Utc>),
    RecentLimit(i64),
}

#[derive(Debug, FromRow)]
struct TimeseriesRecord {
    occurred_at: String,
    status: Option<String>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
struct StatsDeltaRecord {
    captured_at_epoch: i64,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
}

#[derive(Default)]
struct BucketAggregate {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    first_byte_ttfb_sum_ms: f64,
    first_byte_ttfb_values: Vec<f64>,
    first_byte_sample_count: i64,
}

impl BucketAggregate {
    fn record_ttfb_sample(&mut self, status: Option<&str>, ttfb_ms: Option<f64>) {
        if status != Some("success") {
            return;
        }
        let Some(value) = ttfb_ms else {
            return;
        };
        if !value.is_finite() || value <= 0.0 {
            return;
        }
        self.first_byte_sample_count += 1;
        self.first_byte_ttfb_sum_ms += value;
        self.first_byte_ttfb_values.push(value);
    }

    fn first_byte_avg_ms(&self) -> Option<f64> {
        if self.first_byte_sample_count <= 0 {
            return None;
        }
        Some(self.first_byte_ttfb_sum_ms / self.first_byte_sample_count as f64)
    }

    fn first_byte_p95_ms(&self) -> Option<f64> {
        if self.first_byte_ttfb_values.is_empty() {
            return None;
        }
        let mut sorted = self.first_byte_ttfb_values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(percentile_sorted_f64(&sorted, 0.95))
    }
}

#[derive(Debug, Clone)]
struct HttpClients {
    shared: Client,
    proxy: Client,
    timeout: Duration,
    user_agent: String,
}

impl HttpClients {
    fn build(config: &AppConfig) -> Result<Self> {
        let timeout = config.request_timeout;
        let user_agent = config.user_agent.clone();

        let shared = Self::builder(Some(timeout), &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .build()
            .context("failed to construct shared HTTP client")?;

        let proxy = Self::builder(None, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to construct proxy HTTP client")?;

        Ok(Self {
            shared,
            proxy,
            timeout,
            user_agent,
        })
    }

    fn client_for_parallelism(&self, force_new_connection: bool) -> Result<Client> {
        if force_new_connection {
            let client = Self::builder(Some(self.timeout), &self.user_agent)
                .pool_max_idle_per_host(0)
                .build()
                .context("failed to construct dedicated HTTP client")?;
            Ok(client)
        } else {
            Ok(self.shared.clone())
        }
    }

    fn client_for_forward_proxy(&self, endpoint_url: Option<&Url>) -> Result<Client> {
        let Some(endpoint_url) = endpoint_url else {
            return Ok(self.proxy.clone());
        };

        Self::builder(None, &self.user_agent)
            .pool_max_idle_per_host(2)
            .connect_timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .proxy(
                Proxy::all(endpoint_url.as_str())
                    .with_context(|| format!("invalid forward proxy endpoint: {endpoint_url}"))?,
            )
            .build()
            .context("failed to construct forward proxy HTTP client")
    }

    fn builder(timeout: Option<Duration>, user_agent: &str) -> ClientBuilder {
        let builder = Client::builder()
            .user_agent(user_agent)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(90))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .http2_keep_alive_timeout(Duration::from_secs(30))
            .http2_keep_alive_while_idle(true);

        if let Some(timeout) = timeout {
            builder.timeout(timeout)
        } else {
            builder
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    openai_upstream_base_url: Url,
    database_path: PathBuf,
    poll_interval: Duration,
    request_timeout: Duration,
    openai_proxy_handshake_timeout: Duration,
    openai_proxy_request_read_timeout: Duration,
    openai_proxy_max_request_body_bytes: usize,
    proxy_enforce_stream_include_usage: bool,
    proxy_usage_backfill_on_startup: bool,
    proxy_raw_max_bytes: Option<usize>,
    proxy_raw_retention: Duration,
    proxy_raw_dir: PathBuf,
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
    archive_dir: PathBuf,
    invocation_success_full_days: u64,
    invocation_max_days: u64,
    forward_proxy_attempts_retention_days: u64,
    stats_source_snapshots_retention_days: u64,
    quota_snapshot_full_days: u64,
    crs_stats: Option<CrsStatsConfig>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrsStatsConfig {
    base_url: Url,
    api_id: String,
    period: String,
    poll_interval: Duration,
}

impl AppConfig {
    fn from_sources(overrides: &CliArgs) -> Result<Self> {
        if env::var_os(LEGACY_ENV_DATABASE_PATH).is_some() {
            bail!("{LEGACY_ENV_DATABASE_PATH} is not supported; rename it to {ENV_DATABASE_PATH}");
        }
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
                env::var("XY_POLL_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(10));
        let request_timeout = overrides
            .request_timeout_secs
            .or_else(|| {
                env::var("XY_REQUEST_TIMEOUT_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(60));
        let openai_proxy_handshake_timeout = env::var("OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&v| v > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS));
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
        let proxy_raw_retention_days = env::var("PROXY_RAW_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .or_else(|| {
                env::var("PROXY_RAW_RETENTION_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|secs| secs / 86_400)
            })
            .unwrap_or(DEFAULT_PROXY_RAW_RETENTION_DAYS);
        let proxy_raw_retention =
            Duration::from_secs(proxy_raw_retention_days.saturating_mul(86_400));
        let proxy_raw_dir = env::var("PROXY_RAW_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PROXY_RAW_DIR));
        let xray_binary = env::var("XY_XRAY_BINARY")
            .or_else(|_| env::var("XRAY_BINARY"))
            .unwrap_or_else(|_| DEFAULT_XRAY_BINARY.to_string());
        let xray_runtime_dir = env::var("XY_XRAY_RUNTIME_DIR")
            .or_else(|_| env::var("XRAY_RUNTIME_DIR"))
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_XRAY_RUNTIME_DIR));
        let forward_proxy_algo_raw = env::var("FORWARD_PROXY_ALGO").ok();
        let forward_proxy_algo_legacy_raw = env::var("XY_FORWARD_PROXY_ALGO").ok();
        let forward_proxy_algo = resolve_forward_proxy_algo_config(
            forward_proxy_algo_raw.as_deref(),
            forward_proxy_algo_legacy_raw.as_deref(),
        )?;
        let max_parallel_polls = overrides
            .max_parallel_polls
            .or_else(|| {
                env::var("XY_MAX_PARALLEL_POLLS")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
            })
            .filter(|&v| v > 0)
            .unwrap_or(6);
        let shared_connection_parallelism = overrides
            .shared_connection_parallelism
            .or_else(|| {
                env::var("XY_SHARED_CONNECTION_PARALLELISM")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
            })
            .unwrap_or(2);
        let http_bind = if let Some(addr) = overrides.http_bind {
            addr
        } else {
            env::var("XY_HTTP_BIND")
                .ok()
                .map(|v| v.parse())
                .transpose()
                .context("invalid XY_HTTP_BIND socket address")?
                .unwrap_or_else(|| "127.0.0.1:8080".parse().expect("valid default address"))
        };
        let cors_allowed_origins = parse_cors_allowed_origins_env(ENV_CORS_ALLOWED_ORIGINS)?;
        let list_limit_max = overrides
            .list_limit_max
            .or_else(|| {
                env::var("XY_LIST_LIMIT_MAX")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
            })
            .filter(|&v| v > 0)
            .unwrap_or(200);
        let user_agent = overrides
            .user_agent
            .clone()
            .or_else(|| env::var("XY_USER_AGENT").ok())
            .unwrap_or_else(|| "codex-vibe-monitor/0.2.0".to_string());
        let static_dir = overrides
            .static_dir
            .clone()
            .or_else(|| env::var("XY_STATIC_DIR").ok().map(PathBuf::from))
            .or_else(|| {
                let default = PathBuf::from("web/dist");
                if default.exists() {
                    Some(default)
                } else {
                    None
                }
            });
        let retention_enabled =
            parse_bool_env_var("XY_RETENTION_ENABLED", DEFAULT_RETENTION_ENABLED)?;
        let retention_dry_run = overrides.retention_dry_run
            || parse_bool_env_var("XY_RETENTION_DRY_RUN", DEFAULT_RETENTION_DRY_RUN)?;
        let retention_interval = Duration::from_secs(parse_u64_env_var(
            "XY_RETENTION_INTERVAL_SECS",
            DEFAULT_RETENTION_INTERVAL_SECS,
        )?);
        let retention_batch_rows =
            parse_usize_env_var("XY_RETENTION_BATCH_ROWS", DEFAULT_RETENTION_BATCH_ROWS)?.max(1);
        let archive_dir = env::var("XY_ARCHIVE_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_ARCHIVE_DIR));
        let invocation_success_full_days = parse_u64_env_var(
            "XY_INVOCATION_SUCCESS_FULL_DAYS",
            DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
        )?;
        let invocation_max_days =
            parse_u64_env_var("XY_INVOCATION_MAX_DAYS", DEFAULT_INVOCATION_MAX_DAYS)?;
        let forward_proxy_attempts_retention_days = parse_u64_env_var(
            "XY_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS",
            DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
        )?;
        let stats_source_snapshots_retention_days = parse_u64_env_var(
            "XY_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS",
            DEFAULT_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
        )?;
        let quota_snapshot_full_days = parse_u64_env_var(
            "XY_QUOTA_SNAPSHOT_FULL_DAYS",
            DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
        )?;

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
            openai_proxy_handshake_timeout,
            openai_proxy_request_read_timeout,
            openai_proxy_max_request_body_bytes,
            proxy_enforce_stream_include_usage,
            proxy_usage_backfill_on_startup,
            proxy_raw_max_bytes,
            proxy_raw_retention,
            proxy_raw_dir,
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
            archive_dir,
            invocation_success_full_days,
            invocation_max_days,
            forward_proxy_attempts_retention_days,
            stats_source_snapshots_retention_days,
            quota_snapshot_full_days,
            crs_stats,
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::{Body, Bytes, to_bytes},
        extract::{Query, State},
        http::{HeaderValue, Method, StatusCode, Uri, header as http_header},
        response::IntoResponse,
        routing::{any, get, post},
    };
    use chrono::Timelike;
    use flate2::{Compression, write::GzEncoder};
    use serde_json::Value;
    use sqlx::error::{DatabaseError, ErrorKind};
    use sqlx::{Connection, SqliteConnection, SqlitePool};
    use std::{
        borrow::Cow,
        collections::HashSet,
        env, fs,
        path::{Path, PathBuf},
        sync::{Arc, Mutex as StdMutex},
        time::Duration,
    };
    use tokio::net::TcpListener;
    use tokio::sync::{Semaphore, broadcast};
    use tokio::task::JoinHandle;

    static APP_CONFIG_ENV_LOCK: once_cell::sync::Lazy<StdMutex<()>> =
        once_cell::sync::Lazy::new(|| StdMutex::new(()));

    struct CurrentDirGuard {
        original: PathBuf,
    }

    impl CurrentDirGuard {
        fn change_to(path: &Path) -> Self {
            let original = env::current_dir().expect("read current dir");
            env::set_current_dir(path).expect("set current dir");
            Self { original }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.original);
        }
    }

    #[test]
    fn named_range_today_end_respects_dst() {
        let tz = chrono_tz::America::Los_Angeles;
        let now = Utc
            .with_ymd_and_hms(2024, 3, 10, 12, 0, 0)
            .single()
            .expect("valid dt");

        let (start, end) = named_range_bounds("today", now, tz).expect("today bounds");
        // Midnight before DST jump is still PST (-08:00).
        assert_eq!(
            start,
            Utc.with_ymd_and_hms(2024, 3, 10, 8, 0, 0).single().unwrap()
        );
        // Next midnight is PDT (-07:00) after the DST jump.
        assert_eq!(
            end,
            Utc.with_ymd_and_hms(2024, 3, 11, 7, 0, 0).single().unwrap()
        );
    }

    #[test]
    fn named_range_this_week_end_respects_dst() {
        let tz = chrono_tz::America::Los_Angeles;
        let now = Utc
            .with_ymd_and_hms(2024, 3, 6, 12, 0, 0)
            .single()
            .expect("valid dt");

        let (start, end) = named_range_bounds("thisWeek", now, tz).expect("thisWeek bounds");
        // Start of week: Mon 2024-03-04 00:00 PST => 08:00Z.
        assert_eq!(
            start,
            Utc.with_ymd_and_hms(2024, 3, 4, 8, 0, 0).single().unwrap()
        );
        // End of week: Mon 2024-03-11 00:00 PDT => 07:00Z.
        assert_eq!(
            end,
            Utc.with_ymd_and_hms(2024, 3, 11, 7, 0, 0).single().unwrap()
        );
    }

    #[test]
    fn local_naive_to_utc_does_not_fall_back_to_and_utc_on_dst_gap() {
        let tz = chrono_tz::America::Los_Angeles;
        let naive = NaiveDate::from_ymd_opt(2024, 3, 10)
            .unwrap()
            .and_hms_opt(2, 30, 0)
            .unwrap();

        assert!(matches!(tz.from_local_datetime(&naive), LocalResult::None));
        let resolved = local_naive_to_utc(naive, tz);
        assert_ne!(resolved, naive.and_utc());

        let local = resolved.with_timezone(&tz);
        assert_eq!(local.hour(), 3);
        assert_eq!(local.minute(), 0);
        assert_eq!(local.second(), 0);
    }

    #[test]
    fn normalize_single_proxy_url_supports_scheme_less_host_port() {
        assert_eq!(
            normalize_single_proxy_url("127.0.0.1:7890"),
            Some("http://127.0.0.1:7890".to_string())
        );
        assert_eq!(
            normalize_single_proxy_url("socks5://127.0.0.1:1080"),
            Some("socks5://127.0.0.1:1080".to_string())
        );
        assert_eq!(normalize_single_proxy_url("vmess://example"), None);
    }

    #[test]
    fn normalize_single_proxy_url_supports_xray_share_links() {
        let vmess_payload = serde_json::to_string(&json!({
            "add": "vmess.example.com",
            "port": "443",
            "id": "11111111-1111-1111-1111-111111111111",
            "aid": "0",
            "net": "ws",
            "host": "cdn.vmess.example.com",
            "path": "/ws",
            "tls": "tls",
            "ps": "vmess-node"
        }))
        .expect("serialize vmess payload");
        let vmess_link = format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_payload)
        );
        assert!(normalize_single_proxy_url(&vmess_link).is_some());
        assert!(normalize_single_proxy_url("vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#vless").is_some());
        assert!(normalize_single_proxy_url("trojan://password@trojan.example.com:443?type=ws&path=%2Fws&host=cdn.trojan.example.com").is_some());
        assert!(
            normalize_single_proxy_url("ss://YWVzLTI1Ni1nY206cGFzc0AxMjcuMC4wLjE6ODM4OA==")
                .is_some()
        );
    }

    #[test]
    fn parse_proxy_urls_from_subscription_body_supports_xray_links() {
        let vmess_payload = serde_json::to_string(&json!({
            "add": "vmess.example.com",
            "port": "443",
            "id": "11111111-1111-1111-1111-111111111111"
        }))
        .expect("serialize vmess payload");
        let vmess_link = format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_payload)
        );
        let vless_link =
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls";
        let subscription_raw = format!("{vmess_link}\n{vless_link}");
        let encoded = base64::engine::general_purpose::STANDARD.encode(subscription_raw.as_bytes());
        let parsed = parse_proxy_urls_from_subscription_body(&encoded);
        assert!(parsed.iter().any(|item| item.starts_with("vmess://")));
        assert!(parsed.iter().any(|item| item.starts_with("vless://")));
    }

    #[test]
    fn parse_shadowsocks_share_link_decodes_percent_encoded_credentials() {
        let parsed = parse_shadowsocks_share_link(
            "ss://2022-blake3-aes-128-gcm:%2B%2F%3D@127.0.0.1:8388#ss%20node",
        )
        .expect("parse ss2022 link");
        assert_eq!(parsed.method, "2022-blake3-aes-128-gcm");
        assert_eq!(parsed.password, "+/=");
        assert_eq!(parsed.display_name, "ss node");
        assert_eq!(parsed.host, "127.0.0.1");
        assert_eq!(parsed.port, 8388);
    }

    #[test]
    fn parse_shadowsocks_share_link_decodes_percent_encoded_base64_userinfo() {
        let userinfo =
            base64::engine::general_purpose::STANDARD.encode("chacha20-ietf-poly1305:pass+/=");
        let link = format!("ss://{}@127.0.0.1:8388#node", userinfo.replace('=', "%3D"));
        let parsed = parse_shadowsocks_share_link(&link).expect("parse ss base64 userinfo link");
        assert_eq!(parsed.method, "chacha20-ietf-poly1305");
        assert_eq!(parsed.password, "pass+/=");
    }

    #[test]
    fn decode_subscription_payload_supports_base64_blob() {
        let encoded = base64::engine::general_purpose::STANDARD
            .encode("http://127.0.0.1:7890\nsocks5://127.0.0.1:1080");
        let decoded = decode_subscription_payload(&encoded);
        assert!(decoded.contains("http://127.0.0.1:7890"));
        assert!(decoded.contains("socks5://127.0.0.1:1080"));
    }

    #[test]
    fn forward_proxy_validation_timeout_is_split_by_kind() {
        assert_eq!(
            forward_proxy_validation_timeout(ForwardProxyValidationKind::ProxyUrl),
            Duration::from_secs(FORWARD_PROXY_VALIDATION_TIMEOUT_SECS)
        );
        assert_eq!(
            forward_proxy_validation_timeout(ForwardProxyValidationKind::SubscriptionUrl),
            Duration::from_secs(FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_SECS)
        );
    }

    #[test]
    fn remaining_timeout_budget_stops_when_elapsed_reaches_total() {
        let total = Duration::from_secs(60);
        assert_eq!(
            remaining_timeout_budget(total, Duration::from_secs(20)),
            Some(Duration::from_secs(40))
        );
        assert_eq!(
            remaining_timeout_budget(total, Duration::from_secs(60)),
            Some(Duration::ZERO)
        );
        assert_eq!(
            remaining_timeout_budget(total, Duration::from_secs(61)),
            None
        );
    }

    #[test]
    fn timeout_budget_exhausted_treats_zero_budget_as_exhausted() {
        let total = Duration::from_secs(60);
        assert!(!timeout_budget_exhausted(total, Duration::from_secs(59)));
        assert!(timeout_budget_exhausted(total, Duration::from_secs(60)));
        assert!(timeout_budget_exhausted(total, Duration::from_secs(61)));
    }

    #[test]
    fn timeout_seconds_for_message_rounds_subsecond_up_to_one() {
        assert_eq!(timeout_seconds_for_message(Duration::from_millis(1)), 1);
        assert_eq!(timeout_seconds_for_message(Duration::from_secs(5)), 5);
        assert_eq!(timeout_seconds_for_message(Duration::from_millis(5500)), 6);
    }

    #[test]
    fn validation_probe_reachable_status_accepts_success_auth_and_not_found() {
        for status in [
            StatusCode::OK,
            StatusCode::NO_CONTENT,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
        ] {
            assert!(
                is_validation_probe_reachable_status(status),
                "status {status} should be reachable"
            );
        }
    }

    #[test]
    fn validation_probe_reachable_status_rejects_non_reachable_codes() {
        for status in [
            StatusCode::PROXY_AUTHENTICATION_REQUIRED,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert!(
                !is_validation_probe_reachable_status(status),
                "status {status} should not be reachable"
            );
        }
    }

    #[tokio::test]
    async fn validate_proxy_url_candidate_accepts_probe_404() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;

        let response = validate_single_forward_proxy_candidate(state.as_ref(), proxy_url.clone())
            .await
            .expect("404 should be treated as reachable");

        assert!(response.ok);
        assert_eq!(response.message, "proxy validation succeeded");
        assert_eq!(
            response.normalized_value.as_deref(),
            Some(proxy_url.as_str())
        );
        assert_eq!(response.discovered_nodes, Some(1));
        assert!(
            response.latency_ms.unwrap_or_default() >= 0.0,
            "latency should be present"
        );

        proxy_handle.abort();
    }

    #[tokio::test]
    async fn validate_proxy_url_candidate_keeps_5xx_as_failure() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;

        let err = validate_single_forward_proxy_candidate(state.as_ref(), proxy_url)
            .await
            .expect_err("5xx should still fail validation");
        let message = format!("{err:#}");
        assert!(
            message.contains("validation probe returned status 500 Internal Server Error"),
            "expected 500 validation probe failure, got: {message}"
        );

        proxy_handle.abort();
    }

    #[tokio::test]
    async fn validate_subscription_candidate_accepts_probe_404() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
        let (subscription_url, subscription_handle) =
            spawn_test_subscription_source(format!("{proxy_url}\n")).await;
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;

        let response = validate_subscription_candidate(state.as_ref(), subscription_url.clone())
            .await
            .expect("404 should be treated as reachable for subscription validation");

        assert!(response.ok);
        assert_eq!(response.message, "subscription validation succeeded");
        assert_eq!(
            response.normalized_value.as_deref(),
            Some(subscription_url.as_str())
        );
        assert_eq!(response.discovered_nodes, Some(1));
        assert!(
            response.latency_ms.unwrap_or_default() >= 0.0,
            "latency should be present"
        );

        subscription_handle.abort();
        proxy_handle.abort();
    }

    #[tokio::test]
    async fn validate_subscription_candidate_keeps_5xx_as_failure() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
        let (subscription_url, subscription_handle) =
            spawn_test_subscription_source(format!("{proxy_url}\n")).await;
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;

        let err = validate_subscription_candidate(state.as_ref(), subscription_url)
            .await
            .expect_err("5xx should still fail subscription validation");
        let message = format!("{err:#}");
        assert!(
            message.contains("subscription proxy probe failed"),
            "expected subscription probe failure context, got: {message}"
        );
        assert!(
            message.contains("validation probe returned status 500 Internal Server Error"),
            "expected 500 validation probe failure, got: {message}"
        );

        subscription_handle.abort();
        proxy_handle.abort();
    }

    #[tokio::test]
    async fn xray_supervisor_ensure_instance_creates_runtime_dir_for_validation_path() {
        let temp_root = make_temp_test_dir("xray-runtime-create");
        let runtime_dir = temp_root.join("nested/runtime");
        let mut supervisor = XraySupervisor::new(
            "/path/to/non-existent-xray".to_string(),
            runtime_dir.clone(),
        );
        let endpoint = ForwardProxyEndpoint {
            key: "xray-validation-test".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            display_name: "xray-validation-test".to_string(),
            protocol: ForwardProxyProtocol::Vless,
            endpoint_url: None,
            raw_url: Some(
                "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                    .to_string(),
            ),
        };
        let expected_config_path = runtime_dir.join(format!(
            "forward-proxy-{:016x}.json",
            stable_hash_u64(&endpoint.key)
        ));

        let err = supervisor
            .ensure_instance_with_ready_timeout(&endpoint, Duration::from_millis(50))
            .await
            .expect_err("non-existent xray binary should fail to start");
        let message = format!("{err:#}");
        assert!(
            runtime_dir.is_dir(),
            "runtime dir should be created before writing xray config"
        );
        assert!(
            message.contains("failed to start xray binary"),
            "expected startup failure after config write path is available, got: {message}"
        );
        assert!(
            !message.contains("failed to write xray config"),
            "runtime dir creation regression: {message}"
        );
        assert!(
            !expected_config_path.exists(),
            "spawn failure should clean temporary xray config file"
        );

        let _ = fs::remove_dir_all(&temp_root);
    }

    #[tokio::test]
    async fn forward_proxy_settings_triggers_async_bootstrap_probe_for_added_manual_nodes() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
        let normalized_proxy =
            normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;
        let probe_count_before =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
        let success_count_before =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(true)).await;

        let Json(updated) = put_forward_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ForwardProxySettingsUpdateRequest {
                proxy_urls: vec![proxy_url],
                subscription_urls: Vec::new(),
                subscription_update_interval_secs: 3600,
                insert_direct: true,
            }),
        )
        .await
        .expect("put forward proxy settings should succeed");
        assert!(updated.proxy_urls.contains(&normalized_proxy));

        wait_for_forward_proxy_probe_attempts(
            &state.pool,
            &normalized_proxy,
            probe_count_before + 1,
        )
        .await;
        let success_count =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(true)).await;
        assert!(
            success_count > success_count_before,
            "expected at least one successful bootstrap probe attempt"
        );

        proxy_handle.abort();
    }

    #[tokio::test]
    async fn forward_proxy_settings_does_not_probe_when_no_new_nodes() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
        let normalized_proxy =
            normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;

        let request = ForwardProxySettingsUpdateRequest {
            proxy_urls: vec![proxy_url.clone()],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        };
        let probe_count_before =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
        let _ = put_forward_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ForwardProxySettingsUpdateRequest {
                proxy_urls: request.proxy_urls.clone(),
                subscription_urls: request.subscription_urls.clone(),
                subscription_update_interval_secs: request.subscription_update_interval_secs,
                insert_direct: request.insert_direct,
            }),
        )
        .await
        .expect("initial put forward proxy settings should succeed");
        wait_for_forward_proxy_probe_attempts(
            &state.pool,
            &normalized_proxy,
            probe_count_before + 1,
        )
        .await;
        let first_count =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;

        let _ = put_forward_proxy_settings(State(state.clone()), HeaderMap::new(), Json(request))
            .await
            .expect("repeated put forward proxy settings should succeed");
        tokio::time::sleep(Duration::from_millis(300)).await;

        let second_count =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
        assert_eq!(
            second_count, first_count,
            "no newly added endpoint should not trigger extra bootstrap probe"
        );

        proxy_handle.abort();
    }

    #[tokio::test]
    async fn forward_proxy_settings_does_not_reprobe_when_subscription_is_unchanged() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
        let normalized_proxy =
            normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
        let (subscription_url, subscription_handle) =
            spawn_test_subscription_source(format!("{proxy_url}\n")).await;
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;
        let probe_count_before =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;

        let _ = put_forward_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ForwardProxySettingsUpdateRequest {
                proxy_urls: Vec::new(),
                subscription_urls: vec![subscription_url.clone()],
                subscription_update_interval_secs: 3600,
                insert_direct: true,
            }),
        )
        .await
        .expect("initial put forward proxy settings should succeed");
        wait_for_forward_proxy_probe_attempts(
            &state.pool,
            &normalized_proxy,
            probe_count_before + 1,
        )
        .await;
        let first_count =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;

        let _ = put_forward_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ForwardProxySettingsUpdateRequest {
                proxy_urls: Vec::new(),
                subscription_urls: vec![subscription_url],
                subscription_update_interval_secs: 3600,
                insert_direct: true,
            }),
        )
        .await
        .expect("repeated put forward proxy settings should succeed");
        tokio::time::sleep(Duration::from_millis(300)).await;

        let second_count =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
        assert_eq!(
            second_count, first_count,
            "unchanged subscription endpoints should not trigger extra bootstrap probes"
        );

        subscription_handle.abort();
        proxy_handle.abort();
    }

    #[tokio::test]
    async fn refresh_forward_proxy_subscriptions_triggers_bootstrap_probe_for_added_nodes() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
        let normalized_proxy =
            normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
        let (subscription_url, subscription_handle) =
            spawn_test_subscription_source(format!("{proxy_url}\n")).await;
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;

        {
            let mut manager = state.forward_proxy.lock().await;
            manager.apply_settings(ForwardProxySettings {
                proxy_urls: Vec::new(),
                subscription_urls: vec![subscription_url],
                subscription_update_interval_secs: 3600,
                insert_direct: true,
            });
        }
        sync_forward_proxy_routes(state.as_ref())
            .await
            .expect("sync forward proxy routes before subscription refresh");
        let probe_count_before =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
        let success_count_before =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(true)).await;

        refresh_forward_proxy_subscriptions(state.clone(), true, None)
            .await
            .expect("refresh subscriptions should succeed");
        wait_for_forward_proxy_probe_attempts(
            &state.pool,
            &normalized_proxy,
            probe_count_before + 1,
        )
        .await;
        let success_count =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(true)).await;
        assert!(
            success_count > success_count_before,
            "expected at least one successful bootstrap probe attempt from subscription refresh"
        );

        subscription_handle.abort();
        proxy_handle.abort();
    }

    #[tokio::test]
    async fn refresh_forward_proxy_subscriptions_skips_probe_for_known_subscription_keys() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
        let normalized_proxy =
            normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
        let (subscription_url, subscription_handle) =
            spawn_test_subscription_source(format!("{proxy_url}\n")).await;
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;

        {
            let mut manager = state.forward_proxy.lock().await;
            manager.apply_settings(ForwardProxySettings {
                proxy_urls: Vec::new(),
                subscription_urls: vec![subscription_url],
                subscription_update_interval_secs: 3600,
                insert_direct: true,
            });
        }
        sync_forward_proxy_routes(state.as_ref())
            .await
            .expect("sync forward proxy routes before subscription refresh");

        let probe_count_before =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
        let known_keys = HashSet::from([normalized_proxy.clone()]);
        refresh_forward_proxy_subscriptions(state.clone(), true, Some(known_keys))
            .await
            .expect("refresh subscriptions should succeed");
        tokio::time::sleep(Duration::from_millis(300)).await;

        let probe_count_after =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
        assert_eq!(
            probe_count_after, probe_count_before,
            "known subscription keys should suppress startup-style reprobe"
        );

        subscription_handle.abort();
        proxy_handle.abort();
    }

    #[tokio::test]
    async fn forward_proxy_settings_bootstrap_probe_failure_penalizes_runtime_weight() {
        let (proxy_url, proxy_handle) =
            spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
        let normalized_proxy =
            normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
        let state = test_state_with_openai_base(
            Url::parse("http://probe-target.example/").expect("valid probe target"),
        )
        .await;
        let probe_count_before =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
        let failure_count_before =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(false)).await;

        let _ = put_forward_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ForwardProxySettingsUpdateRequest {
                proxy_urls: vec![proxy_url],
                subscription_urls: Vec::new(),
                subscription_update_interval_secs: 3600,
                insert_direct: true,
            }),
        )
        .await
        .expect("put forward proxy settings should succeed");

        wait_for_forward_proxy_probe_attempts(
            &state.pool,
            &normalized_proxy,
            probe_count_before + 1,
        )
        .await;
        let failure_count =
            count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(false)).await;
        assert!(
            failure_count > failure_count_before,
            "expected at least one failed bootstrap probe attempt"
        );

        let runtime_weight = read_forward_proxy_runtime_weight(&state.pool, &normalized_proxy)
            .await
            .expect("runtime weight should exist");
        assert!(
            runtime_weight < 1.0,
            "expected failed bootstrap probe to penalize runtime weight; got {runtime_weight}"
        );

        proxy_handle.abort();
    }

    #[test]
    fn forward_proxy_manager_keeps_one_positive_weight() {
        let mut manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
                subscription_urls: vec![],
                subscription_update_interval_secs: 3600,
                insert_direct: false,
            },
            vec![],
        );

        for runtime in manager.runtime.values_mut() {
            runtime.weight = -5.0;
        }
        manager.ensure_non_zero_weight();

        assert!(manager.runtime.values().any(|entry| entry.weight > 0.0));
    }

    #[test]
    fn forward_proxy_algo_from_str_supports_v1_and_v2() {
        assert_eq!(
            ForwardProxyAlgo::from_str("v1").expect("v1 should parse"),
            ForwardProxyAlgo::V1
        );
        assert_eq!(
            ForwardProxyAlgo::from_str("V2").expect("v2 should parse"),
            ForwardProxyAlgo::V2
        );
        assert!(ForwardProxyAlgo::from_str("unexpected").is_err());
    }

    #[test]
    fn forward_proxy_algo_config_defaults_to_latest_v2() {
        let algo =
            resolve_forward_proxy_algo_config(None, None).expect("default algo should resolve");
        assert_eq!(algo, ForwardProxyAlgo::V2);
    }

    #[test]
    fn forward_proxy_algo_config_accepts_primary_env() {
        let algo = resolve_forward_proxy_algo_config(Some("v2"), None)
            .expect("primary env should resolve");
        assert_eq!(algo, ForwardProxyAlgo::V2);
    }

    #[test]
    fn forward_proxy_algo_config_rejects_legacy_env() {
        assert!(resolve_forward_proxy_algo_config(None, Some("v1")).is_err());
    }

    #[test]
    fn forward_proxy_algo_config_rejects_when_both_env_vars_are_set() {
        assert!(resolve_forward_proxy_algo_config(Some("v2"), Some("v1")).is_err());
    }

    #[test]
    fn forward_proxy_manager_v2_keeps_two_positive_weights() {
        let mut manager = ForwardProxyManager::with_algo(
            ForwardProxySettings {
                proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
                subscription_urls: vec![],
                subscription_update_interval_secs: 3600,
                insert_direct: true,
            },
            vec![],
            ForwardProxyAlgo::V2,
        );

        for runtime in manager.runtime.values_mut() {
            runtime.weight = -5.0;
        }
        manager.ensure_non_zero_weight();

        let positive_count = manager
            .endpoints
            .iter()
            .filter_map(|endpoint| manager.runtime.get(&endpoint.key))
            .filter(|entry| entry.weight > 0.0)
            .count();
        assert_eq!(positive_count, 2);
    }

    #[test]
    fn forward_proxy_manager_v2_clamps_persisted_runtime_weight_on_startup() {
        let manager = ForwardProxyManager::with_algo(
            ForwardProxySettings {
                proxy_urls: vec![],
                subscription_urls: vec![],
                subscription_update_interval_secs: 3600,
                insert_direct: true,
            },
            vec![ForwardProxyRuntimeState {
                proxy_key: FORWARD_PROXY_DIRECT_KEY.to_string(),
                display_name: FORWARD_PROXY_DIRECT_LABEL.to_string(),
                source: FORWARD_PROXY_SOURCE_DIRECT.to_string(),
                endpoint_url: None,
                weight: 99.0,
                success_ema: 0.65,
                latency_ema_ms: None,
                consecutive_failures: 0,
            }],
            ForwardProxyAlgo::V2,
        );

        let direct_runtime = manager
            .runtime
            .get(FORWARD_PROXY_DIRECT_KEY)
            .expect("direct runtime should exist");
        assert_eq!(direct_runtime.weight, FORWARD_PROXY_V2_WEIGHT_MAX);
    }

    #[test]
    fn forward_proxy_manager_v2_counts_only_selectable_positive_candidates() {
        let mut manager = ForwardProxyManager::with_algo(
            ForwardProxySettings {
                proxy_urls: vec![
                    "http://127.0.0.1:7890".to_string(),
                    "http://127.0.0.1:7891".to_string(),
                    "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                        .to_string(),
                ],
                subscription_urls: vec![],
                subscription_update_interval_secs: 3600,
                insert_direct: false,
            },
            vec![],
            ForwardProxyAlgo::V2,
        );

        let selectable_keys = manager
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.is_selectable())
            .map(|endpoint| endpoint.key.clone())
            .collect::<Vec<_>>();
        assert_eq!(selectable_keys.len(), 2);
        let non_selectable_key = manager
            .endpoints
            .iter()
            .find(|endpoint| !endpoint.is_selectable())
            .map(|endpoint| endpoint.key.clone())
            .expect("non-selectable endpoint should exist");

        manager
            .runtime
            .get_mut(&selectable_keys[0])
            .expect("selectable runtime should exist")
            .weight = 1.0;
        manager
            .runtime
            .get_mut(&selectable_keys[1])
            .expect("selectable runtime should exist")
            .weight = -5.0;
        manager
            .runtime
            .get_mut(&non_selectable_key)
            .expect("non-selectable runtime should exist")
            .weight = 1.0;

        manager.ensure_non_zero_weight();

        let positive_selectable = selectable_keys
            .iter()
            .filter_map(|key| manager.runtime.get(key))
            .filter(|runtime| runtime.weight > 0.0)
            .count();
        assert_eq!(positive_selectable, 2);
    }

    #[test]
    fn forward_proxy_manager_v2_probe_ignores_non_selectable_penalties() {
        let mut manager = ForwardProxyManager::with_algo(
            ForwardProxySettings {
                proxy_urls: vec![
                    "http://127.0.0.1:7890".to_string(),
                    "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                        .to_string(),
                ],
                subscription_urls: vec![],
                subscription_update_interval_secs: 3600,
                insert_direct: false,
            },
            vec![],
            ForwardProxyAlgo::V2,
        );

        let selectable_key = manager
            .endpoints
            .iter()
            .find(|endpoint| endpoint.is_selectable())
            .map(|endpoint| endpoint.key.clone())
            .expect("selectable endpoint should exist");
        let non_selectable_key = manager
            .endpoints
            .iter()
            .find(|endpoint| !endpoint.is_selectable())
            .map(|endpoint| endpoint.key.clone())
            .expect("non-selectable endpoint should exist");

        manager
            .runtime
            .get_mut(&selectable_key)
            .expect("selectable runtime should exist")
            .weight = 1.0;
        manager
            .runtime
            .get_mut(&non_selectable_key)
            .expect("non-selectable runtime should exist")
            .weight = -2.0;

        assert!(!manager.should_probe_penalized_proxy());
        assert!(manager.mark_probe_started().is_none());
    }

    #[test]
    fn forward_proxy_manager_v2_success_with_high_latency_still_gains_weight() {
        let mut manager = ForwardProxyManager::with_algo(
            ForwardProxySettings {
                proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
                subscription_urls: vec![],
                subscription_update_interval_secs: 3600,
                insert_direct: false,
            },
            vec![],
            ForwardProxyAlgo::V2,
        );
        let proxy_key = manager
            .endpoints
            .first()
            .expect("endpoint should exist")
            .key
            .clone();
        let before = manager
            .runtime
            .get(&proxy_key)
            .expect("runtime should exist")
            .weight;

        manager.record_attempt(&proxy_key, true, Some(45_000.0), false);

        let after = manager
            .runtime
            .get(&proxy_key)
            .expect("runtime should exist")
            .weight;
        assert!(after > before, "v2 success should increase weight");
    }

    #[test]
    fn forward_proxy_manager_v2_success_recovers_penalized_proxy() {
        let mut manager = ForwardProxyManager::with_algo(
            ForwardProxySettings {
                proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
                subscription_urls: vec![],
                subscription_update_interval_secs: 3600,
                insert_direct: false,
            },
            vec![],
            ForwardProxyAlgo::V2,
        );
        let proxy_key = manager
            .endpoints
            .first()
            .expect("endpoint should exist")
            .key
            .clone();
        if let Some(runtime) = manager.runtime.get_mut(&proxy_key) {
            runtime.weight = -2.0;
            runtime.consecutive_failures = 5;
            runtime.latency_ema_ms = Some(30_000.0);
        }

        manager.record_attempt(&proxy_key, true, Some(30_000.0), false);

        let runtime = manager
            .runtime
            .get(&proxy_key)
            .expect("runtime should exist");
        assert!(
            runtime.weight >= FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR,
            "successful recovery should restore minimum v2 weight"
        );
        assert_eq!(
            runtime.consecutive_failures, 0,
            "successful attempt should reset failure streak"
        );
    }

    #[test]
    fn classify_invocation_failure_marks_downstream_closed_as_client_abort() {
        let result = classify_invocation_failure(
            Some("http_200"),
            Some("[downstream_closed] downstream closed while streaming upstream response"),
        );
        assert_eq!(result.failure_class, FailureClass::ClientAbort);
        assert!(!result.is_actionable);
        assert_eq!(result.failure_kind.as_deref(), Some("downstream_closed"));
    }

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
        assert!(
            err.0
                .to_string()
                .contains("unsupported failure scope: unexpected"),
            "error should mention rejected scope"
        );
    }

    #[test]
    fn app_config_from_sources_ignores_removed_xyai_env_vars() {
        let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");
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
        let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");
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
    fn app_config_from_sources_rejects_legacy_database_path_env() {
        let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");
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

    fn test_config() -> AppConfig {
        AppConfig {
            openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
            database_path: PathBuf::from(":memory:"),
            poll_interval: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            openai_proxy_handshake_timeout: Duration::from_secs(
                DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS,
            ),
            openai_proxy_request_read_timeout: Duration::from_secs(
                DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS,
            ),
            openai_proxy_max_request_body_bytes: DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
            proxy_enforce_stream_include_usage: DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
            proxy_usage_backfill_on_startup: DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
            proxy_raw_max_bytes: DEFAULT_PROXY_RAW_MAX_BYTES,
            proxy_raw_retention: Duration::from_secs(DEFAULT_PROXY_RAW_RETENTION_DAYS * 86_400),
            proxy_raw_dir: PathBuf::from("target/proxy-raw-tests"),
            xray_binary: DEFAULT_XRAY_BINARY.to_string(),
            xray_runtime_dir: PathBuf::from("target/xray-forward-tests"),
            forward_proxy_algo: ForwardProxyAlgo::V1,
            max_parallel_polls: 2,
            shared_connection_parallelism: 1,
            http_bind: "127.0.0.1:38080".parse().expect("valid socket address"),
            cors_allowed_origins: Vec::new(),
            list_limit_max: 100,
            user_agent: "codex-test".to_string(),
            static_dir: None,
            retention_enabled: DEFAULT_RETENTION_ENABLED,
            retention_dry_run: DEFAULT_RETENTION_DRY_RUN,
            retention_interval: Duration::from_secs(DEFAULT_RETENTION_INTERVAL_SECS),
            retention_batch_rows: DEFAULT_RETENTION_BATCH_ROWS,
            archive_dir: PathBuf::from("target/archive-tests"),
            invocation_success_full_days: DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
            invocation_max_days: DEFAULT_INVOCATION_MAX_DAYS,
            forward_proxy_attempts_retention_days: DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
            stats_source_snapshots_retention_days: DEFAULT_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
            quota_snapshot_full_days: DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
            crs_stats: None,
        }
    }

    fn make_temp_test_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).expect("create temp test dir");
        dir
    }

    fn set_file_mtime_seconds_ago(path: &Path, seconds: u64) {
        let modified_at = std::time::SystemTime::now() - Duration::from_secs(seconds);
        let modified_at = filetime::FileTime::from_system_time(modified_at);
        filetime::set_file_mtime(path, modified_at).expect("set file mtime");
    }

    #[test]
    fn archive_batch_file_path_resolves_relative_archive_dir_from_database_parent() {
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
    fn resolved_proxy_raw_dir_resolves_relative_dir_from_database_parent() {
        let mut config = test_config();
        config.database_path = PathBuf::from("/tmp/codex-retention/codex_vibe_monitor.db");
        config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

        assert_eq!(
            config.resolved_proxy_raw_dir(),
            PathBuf::from("/tmp/codex-retention/proxy_raw_payloads")
        );
    }

    #[test]
    fn store_raw_payload_file_anchors_relative_dir_to_database_parent() {
        let _guard = APP_CONFIG_ENV_LOCK.lock().expect("cwd lock");
        let temp_dir = make_temp_test_dir("proxy-raw-store-db-parent");
        let cwd = temp_dir.join("cwd");
        let db_root = temp_dir.join("db-root");
        fs::create_dir_all(&cwd).expect("create cwd dir");
        fs::create_dir_all(&db_root).expect("create db root");
        let _cwd_guard = CurrentDirGuard::change_to(&cwd);

        let mut config = test_config();
        config.database_path = db_root.join("codex_vibe_monitor.db");
        config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

        let meta = store_raw_payload_file(&config, "proxy-test", "request", b"{\"ok\":true}");
        let expected = db_root.join("proxy_raw_payloads/proxy-test-request.bin");

        assert_eq!(
            meta.path.as_deref(),
            Some(expected.to_string_lossy().as_ref())
        );
        assert!(
            expected.exists(),
            "raw payload should be written beside the database"
        );
        assert!(
            !cwd.join("proxy_raw_payloads/proxy-test-request.bin")
                .exists(),
            "raw payload should not follow the current working directory"
        );

        cleanup_temp_test_dir(&temp_dir);
    }

    #[test]
    fn read_proxy_raw_bytes_keeps_current_dir_compat_for_legacy_relative_paths() {
        let _guard = APP_CONFIG_ENV_LOCK.lock().expect("cwd lock");
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

    fn sqlite_url_for_path(path: &Path) -> String {
        format!("sqlite://{}", path.to_string_lossy())
    }

    async fn retention_test_pool_and_config(prefix: &str) -> (SqlitePool, AppConfig, PathBuf) {
        let temp_dir = make_temp_test_dir(prefix);
        let db_path = temp_dir.join("codex-vibe-monitor.db");
        fs::File::create(&db_path).expect("create retention sqlite file");
        let db_url = sqlite_url_for_path(&db_path);
        let pool = SqlitePool::connect(&db_url)
            .await
            .expect("connect retention sqlite");
        ensure_schema(&pool).await.expect("ensure retention schema");

        let mut config = test_config();
        config.database_path = db_path;
        config.proxy_raw_dir = temp_dir.join("proxy_raw_payloads");
        config.archive_dir = temp_dir.join("archives");
        config.retention_batch_rows = 2;
        fs::create_dir_all(&config.proxy_raw_dir).expect("create retention raw dir");
        fs::create_dir_all(&config.archive_dir).expect("create retention archive dir");
        (pool, config, temp_dir)
    }

    fn cleanup_temp_test_dir(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }

    fn shanghai_local_days_ago(days: i64, hour: u32, minute: u32, second: u32) -> String {
        let now_local = Utc::now().with_timezone(&Shanghai);
        let naive = (now_local.date_naive() - ChronoDuration::days(days))
            .and_hms_opt(hour, minute, second)
            .expect("valid shanghai local time");
        format_naive(naive)
    }

    fn utc_naive_from_shanghai_local_days_ago(
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
    async fn insert_retention_invocation(
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
                total_tokens,
                cost,
                status,
                payload,
                raw_response,
                request_raw_path,
                request_raw_size,
                response_raw_path,
                response_raw_size,
                raw_expires_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(source)
        .bind(Some("gpt-5.2-codex"))
        .bind(Some(12_i64))
        .bind(Some(3_i64))
        .bind(total_tokens)
        .bind(cost)
        .bind(status)
        .bind(payload)
        .bind(raw_response)
        .bind(request_raw_path.map(|path| path.to_string_lossy().to_string()))
        .bind(
            request_raw_path
                .and_then(|path| fs::metadata(path).ok())
                .map(|meta| meta.len() as i64),
        )
        .bind(response_raw_path.map(|path| path.to_string_lossy().to_string()))
        .bind(
            response_raw_path
                .and_then(|path| fs::metadata(path).ok())
                .map(|meta| meta.len() as i64),
        )
        .bind(Some("2099-01-01 00:00:00"))
        .execute(pool)
        .await
        .expect("insert retention invocation");
    }

    async fn insert_stats_source_snapshot_row(
        pool: &SqlitePool,
        captured_at: &str,
        stats_date: &str,
    ) {
        sqlx::query(
            r#"
            INSERT INTO stats_source_snapshots (
                source,
                period,
                stats_date,
                model,
                requests,
                input_tokens,
                output_tokens,
                cache_create_tokens,
                cache_read_tokens,
                all_tokens,
                cost_input,
                cost_output,
                cost_cache_write,
                cost_cache_read,
                cost_total,
                raw_response,
                captured_at,
                captured_at_epoch
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            "#,
        )
        .bind(SOURCE_CRS)
        .bind("daily")
        .bind(stats_date)
        .bind(Some("gpt-5.2"))
        .bind(4_i64)
        .bind(10_i64)
        .bind(6_i64)
        .bind(0_i64)
        .bind(0_i64)
        .bind(16_i64)
        .bind(0.1_f64)
        .bind(0.2_f64)
        .bind(0.0_f64)
        .bind(0.0_f64)
        .bind(0.3_f64)
        .bind("{}")
        .bind(captured_at)
        .bind(
            parse_utc_naive(captured_at)
                .expect("valid utc naive")
                .and_utc()
                .timestamp(),
        )
        .execute(pool)
        .await
        .expect("insert stats source snapshot row");
    }

    #[derive(Debug)]
    struct FakeSqliteCodeDatabaseError {
        message: &'static str,
        code: &'static str,
    }

    impl std::fmt::Display for FakeSqliteCodeDatabaseError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.message)
        }
    }

    impl std::error::Error for FakeSqliteCodeDatabaseError {}

    impl DatabaseError for FakeSqliteCodeDatabaseError {
        fn message(&self) -> &str {
            self.message
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(Cow::Borrowed(self.code))
        }

        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    fn write_backfill_response_payload(path: &Path) {
        write_backfill_response_payload_with_service_tier(path, None);
    }

    fn write_backfill_response_payload_with_service_tier(path: &Path, service_tier: Option<&str>) {
        let mut response = json!({
            "type": "response.completed",
            "response": {
                "usage": {
                    "input_tokens": 88,
                    "output_tokens": 22,
                    "total_tokens": 110,
                    "input_tokens_details": { "cached_tokens": 9 },
                    "output_tokens_details": { "reasoning_tokens": 3 }
                }
            }
        });
        if let Some(service_tier) = service_tier {
            response["response"]["service_tier"] = Value::String(service_tier.to_string());
        }
        let raw = [
            "event: response.completed".to_string(),
            format!("data: {response}"),
        ]
        .join("\n");
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(raw.as_bytes())
            .expect("write gzip payload");
        let compressed = encoder.finish().expect("finish gzip payload");
        fs::write(path, compressed).expect("write response payload");
    }

    fn write_backfill_request_payload(path: &Path, prompt_cache_key: Option<&str>) {
        write_backfill_request_payload_with_fields(
            path,
            prompt_cache_key,
            None,
            None,
            ProxyCaptureTarget::Responses,
        );
    }

    fn write_backfill_request_payload_with_requested_service_tier(
        path: &Path,
        requested_service_tier: Option<&str>,
        target: ProxyCaptureTarget,
    ) {
        write_backfill_request_payload_with_fields(
            path,
            None,
            None,
            requested_service_tier,
            target,
        );
    }

    fn write_backfill_request_payload_with_reasoning(
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

    fn write_backfill_request_payload_with_fields(
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
        };
        let encoded = serde_json::to_vec(&payload).expect("serialize request payload");
        fs::write(path, encoded).expect("write request payload");
    }

    async fn insert_proxy_backfill_row(pool: &SqlitePool, invoke_id: &str, response_path: &Path) {
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

    async fn insert_proxy_cost_backfill_row(
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

    async fn insert_proxy_prompt_cache_backfill_row(
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

    async fn test_state_with_openai_base(openai_base: Url) -> Arc<AppState> {
        test_state_with_openai_base_and_body_limit(
            openai_base,
            DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        )
        .await
    }

    async fn test_state_with_openai_base_and_body_limit(
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

    async fn test_state_with_openai_base_body_limit_and_read_timeout(
        openai_base: Url,
        body_limit: usize,
        request_read_timeout: Duration,
    ) -> Arc<AppState> {
        let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let db_url =
            format!("sqlite:file:codex-vibe-monitor-test-{db_id}?mode=memory&cache=shared");
        let pool = SqlitePool::connect(&db_url)
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let mut config = test_config();
        config.openai_upstream_base_url = openai_base;
        config.openai_proxy_max_request_body_bytes = body_limit;
        config.openai_proxy_request_read_timeout = request_read_timeout;
        let http_clients = HttpClients::build(&config).expect("http clients");
        let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
        let (broadcaster, _rx) = broadcast::channel(16);
        let pricing_catalog = load_pricing_catalog(&pool)
            .await
            .expect("pricing catalog should initialize");

        Arc::new(AppState {
            config: config.clone(),
            pool,
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            startup_ready: Arc::new(AtomicBool::new(true)),
            semaphore,
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
        })
    }

    fn test_stage_timings() -> StageTimings {
        StageTimings {
            t_total_ms: 0.0,
            t_req_read_ms: 0.0,
            t_req_parse_ms: 0.0,
            t_upstream_connect_ms: 0.0,
            t_upstream_ttfb_ms: 0.0,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        }
    }

    fn test_proxy_capture_record(invoke_id: &str, occurred_at: &str) -> ProxyCaptureRecord {
        ProxyCaptureRecord {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            model: Some("gpt-5.2-codex".to_string()),
            usage: ParsedUsage {
                input_tokens: Some(12),
                output_tokens: Some(3),
                cache_input_tokens: Some(2),
                reasoning_tokens: Some(0),
                total_tokens: Some(15),
            },
            cost: Some(0.0123),
            cost_estimated: true,
            price_version: Some("unit-test".to_string()),
            status: "success".to_string(),
            error_message: None,
            payload: Some(
                "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":false,\"requesterIp\":\"198.51.100.77\",\"promptCacheKey\":\"pck-broadcast-1\",\"requestedServiceTier\":\"priority\",\"reasoningEffort\":\"high\"}"
                    .to_string(),
            ),
            raw_response: "{}".to_string(),
            req_raw: RawPayloadMeta::default(),
            resp_raw: RawPayloadMeta::default(),
            raw_expires_at: None,
            timings: test_stage_timings(),
        }
    }

    async fn seed_quota_snapshot(pool: &SqlitePool, captured_at: &str) {
        sqlx::query(
            r#"
            INSERT INTO codex_quota_snapshots (
                captured_at,
                amount_limit,
                used_amount,
                remaining_amount,
                period,
                period_reset_time,
                expire_time,
                is_active,
                total_cost,
                total_requests,
                total_tokens,
                last_request_time,
                billing_type,
                remaining_count,
                used_count,
                sub_type_name
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            "#,
        )
        .bind(captured_at)
        .bind(Some(100.0))
        .bind(Some(10.0))
        .bind(Some(90.0))
        .bind(Some("monthly"))
        .bind(Some("2026-03-01 00:00:00"))
        .bind(None::<String>)
        .bind(1_i64)
        .bind(10.0)
        .bind(9_i64)
        .bind(150_i64)
        .bind(Some(captured_at))
        .bind(Some("prepaid"))
        .bind(Some(91_i64))
        .bind(Some(9_i64))
        .bind(Some("unit"))
        .execute(pool)
        .await
        .expect("seed quota snapshot");
    }

    async fn seed_forward_proxy_attempt_at(
        pool: &SqlitePool,
        proxy_key: &str,
        occurred_at: DateTime<Utc>,
        is_success: bool,
    ) {
        sqlx::query(
            r#"
            INSERT INTO forward_proxy_attempts (
                proxy_key,
                occurred_at,
                is_success,
                latency_ms,
                failure_kind,
                is_probe
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(proxy_key)
        .bind(occurred_at.format("%Y-%m-%d %H:%M:%S").to_string())
        .bind(is_success as i64)
        .bind(if is_success { Some(120.0) } else { None })
        .bind(if is_success {
            None::<String>
        } else {
            Some(FORWARD_PROXY_FAILURE_STREAM_ERROR.to_string())
        })
        .bind(0_i64)
        .execute(pool)
        .await
        .expect("seed forward proxy attempt");
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_forward_proxy_weight_bucket_at(
        pool: &SqlitePool,
        proxy_key: &str,
        bucket_start_epoch: i64,
        sample_count: i64,
        min_weight: f64,
        max_weight: f64,
        avg_weight: f64,
        last_weight: f64,
    ) {
        sqlx::query(
            r#"
            INSERT INTO forward_proxy_weight_hourly (
                proxy_key,
                bucket_start_epoch,
                sample_count,
                min_weight,
                max_weight,
                avg_weight,
                last_weight,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
            ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
                sample_count = excluded.sample_count,
                min_weight = excluded.min_weight,
                max_weight = excluded.max_weight,
                avg_weight = excluded.avg_weight,
                last_weight = excluded.last_weight,
                updated_at = datetime('now')
            "#,
        )
        .bind(proxy_key)
        .bind(bucket_start_epoch)
        .bind(sample_count)
        .bind(min_weight)
        .bind(max_weight)
        .bind(avg_weight)
        .bind(last_weight)
        .execute(pool)
        .await
        .expect("seed forward proxy weight bucket");
    }

    async fn drain_broadcast_messages(rx: &mut broadcast::Receiver<BroadcastPayload>) {
        loop {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(_)) => continue,
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Err(_) => break,
            }
        }
    }

    async fn spawn_test_forward_proxy_status(status: StatusCode) -> (String, JoinHandle<()>) {
        let app = Router::new().fallback(any(move || async move {
            (
                status,
                Json(json!({
                    "status": status.as_u16(),
                })),
            )
        }));

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind forward proxy status test server");
        let addr = listener
            .local_addr()
            .expect("forward proxy status test server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("forward proxy status test server should run");
        });

        (format!("http://{addr}"), handle)
    }

    async fn spawn_test_subscription_source(body: String) -> (String, JoinHandle<()>) {
        let body = Arc::new(body);
        let app = Router::new().route(
            "/subscription",
            get({
                let body = body.clone();
                move || {
                    let body = body.clone();
                    async move { (StatusCode::OK, body.as_str().to_string()) }
                }
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind subscription source test server");
        let addr = listener
            .local_addr()
            .expect("subscription source test server addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("subscription source test server should run");
        });

        (format!("http://{addr}/subscription"), handle)
    }

    async fn count_forward_proxy_probe_attempts(
        pool: &SqlitePool,
        proxy_key: &str,
        success: Option<bool>,
    ) -> i64 {
        let query = match success {
            Some(true) => {
                "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0 AND is_success != 0"
            }
            Some(false) => {
                "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0 AND is_success = 0"
            }
            None => {
                "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0"
            }
        };
        sqlx::query_scalar(query)
            .bind(proxy_key)
            .fetch_one(pool)
            .await
            .expect("count forward proxy probe attempts")
    }

    async fn wait_for_forward_proxy_probe_attempts(
        pool: &SqlitePool,
        proxy_key: &str,
        expected_min_count: i64,
    ) {
        let started = Instant::now();
        loop {
            let count = count_forward_proxy_probe_attempts(pool, proxy_key, None).await;
            if count >= expected_min_count {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "timed out waiting forward proxy probe attempts for {proxy_key}; expected at least {expected_min_count}, got {count}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn read_forward_proxy_runtime_weight(pool: &SqlitePool, proxy_key: &str) -> Option<f64> {
        sqlx::query_scalar::<_, f64>(
            "SELECT weight FROM forward_proxy_runtime WHERE proxy_key = ?1",
        )
        .bind(proxy_key)
        .fetch_optional(pool)
        .await
        .expect("read forward proxy runtime weight")
    }

    async fn test_upstream_echo(
        method: Method,
        uri: Uri,
        headers: HeaderMap,
        body: String,
    ) -> impl IntoResponse {
        let auth = headers
            .get(http_header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let host_header = headers
            .get(http_header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let connection_seen = headers.contains_key(http_header::CONNECTION);
        let x_foo_seen = headers.contains_key(http_header::HeaderName::from_static("x-foo"));
        let x_forwarded_for_seen =
            headers.contains_key(http_header::HeaderName::from_static("x-forwarded-for"));
        let forwarded_seen =
            headers.contains_key(http_header::HeaderName::from_static("forwarded"));
        let via_seen = headers.contains_key(http_header::HeaderName::from_static("via"));
        let mut response_headers = HeaderMap::new();
        response_headers.insert(
            http_header::HeaderName::from_static("x-upstream"),
            HeaderValue::from_static("ok"),
        );
        response_headers.insert(
            http_header::CONNECTION,
            HeaderValue::from_static("x-upstream-hop"),
        );
        response_headers.insert(
            http_header::HeaderName::from_static("x-upstream-hop"),
            HeaderValue::from_static("should-be-filtered"),
        );
        response_headers.insert(
            http_header::HeaderName::from_static("via"),
            HeaderValue::from_static("1.1 upstream-proxy"),
        );
        response_headers.insert(
            http_header::HeaderName::from_static("forwarded"),
            HeaderValue::from_static("for=192.0.2.1;proto=https;host=api.example.com"),
        );

        (
            StatusCode::CREATED,
            response_headers,
            Json(json!({
                "method": method.as_str(),
                "path": uri.path(),
                "query": uri.query().unwrap_or_default(),
                "authorization": auth,
                "hostHeader": host_header,
                "connectionSeen": connection_seen,
                "xFooSeen": x_foo_seen,
                "xForwardedForSeen": x_forwarded_for_seen,
                "forwardedSeen": forwarded_seen,
                "viaSeen": via_seen,
                "body": body,
            })),
        )
    }

    async fn test_upstream_stream() -> impl IntoResponse {
        let chunks = stream::iter(vec![
            Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")),
            Ok::<_, Infallible>(Bytes::from_static(b"chunk-b")),
        ]);
        (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            )],
            Body::from_stream(chunks),
        )
    }

    async fn test_upstream_stream_first_error() -> impl IntoResponse {
        let chunks = stream::unfold(0usize, |state| async move {
            match state {
                0 => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Some((
                        Err::<Bytes, io::Error>(io::Error::other("upstream-first-chunk-error")),
                        1,
                    ))
                }
                _ => None,
            }
        });
        (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            )],
            Body::from_stream(chunks),
        )
    }

    async fn test_upstream_stream_mid_error() -> impl IntoResponse {
        let chunks = stream::unfold(0usize, |state| async move {
            match state {
                0 => Some((Ok::<Bytes, io::Error>(Bytes::from_static(b"chunk-a")), 1)),
                1 => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Some((
                        Err::<Bytes, io::Error>(io::Error::other("upstream-mid-stream-error")),
                        2,
                    ))
                }
                _ => None,
            }
        });
        (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            )],
            Body::from_stream(chunks),
        )
    }

    async fn test_upstream_slow_stream() -> impl IntoResponse {
        let chunks = stream::unfold(0usize, |state| async move {
            match state {
                0 => Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")), 1)),
                1 => {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-b")), 2))
                }
                _ => None,
            }
        });
        (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            )],
            Body::from_stream(chunks),
        )
    }

    async fn test_upstream_hang() -> impl IntoResponse {
        tokio::time::sleep(Duration::from_secs(2)).await;
        StatusCode::NO_CONTENT
    }

    async fn test_upstream_redirect() -> impl IntoResponse {
        (
            StatusCode::TEMPORARY_REDIRECT,
            [(
                http_header::LOCATION,
                HeaderValue::from_static("/v1/echo?from=redirect"),
            )],
            Body::empty(),
        )
    }

    async fn test_upstream_external_redirect() -> impl IntoResponse {
        (
            StatusCode::TEMPORARY_REDIRECT,
            [(
                http_header::LOCATION,
                HeaderValue::from_static("https://example.org/outside"),
            )],
            Body::empty(),
        )
    }

    async fn test_upstream_chat_external_redirect() -> impl IntoResponse {
        (
            StatusCode::TEMPORARY_REDIRECT,
            [(
                http_header::LOCATION,
                HeaderValue::from_static("https://example.org/outside"),
            )],
            Body::empty(),
        )
    }

    async fn test_upstream_responses_gzip_stream() -> impl IntoResponse {
        let payload = [
            "event: response.created\n",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}\n\n",
        ]
        .concat();

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(payload.as_bytes())
            .expect("write gzip payload");
        let compressed = encoder.finish().expect("finish gzip payload");

        (
            StatusCode::OK,
            [
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("text/event-stream"),
                ),
                (
                    http_header::CONTENT_ENCODING,
                    HeaderValue::from_static("gzip"),
                ),
            ],
            Body::from(compressed),
        )
    }

    async fn test_upstream_responses(uri: Uri) -> Response {
        if uri.query().is_some_and(|query| query.contains("mode=gzip")) {
            test_upstream_responses_gzip_stream().await.into_response()
        } else {
            test_upstream_stream_mid_error().await.into_response()
        }
    }

    async fn test_upstream_responses_compact() -> impl IntoResponse {
        (
            StatusCode::OK,
            Json(json!({
                "id": "resp_compact_test",
                "object": "response.compaction",
                "output": [
                    {
                        "id": "cmp_001",
                        "type": "compaction",
                        "encrypted_content": "encrypted-summary"
                    }
                ],
                "usage": {
                    "input_tokens": 139,
                    "input_tokens_details": {
                        "cached_tokens": 11
                    },
                    "output_tokens": 438,
                    "output_tokens_details": {
                        "reasoning_tokens": 64
                    },
                    "total_tokens": 577
                }
            })),
        )
    }

    async fn test_upstream_models(uri: Uri) -> impl IntoResponse {
        if uri
            .query()
            .is_some_and(|query| query.contains("mode=error"))
        {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": "upstream model list unavailable"
                })),
            )
                .into_response();
        }

        if uri
            .query()
            .is_some_and(|query| query.contains("mode=slow-body"))
        {
            let chunked = stream::unfold(0u8, |state| async move {
                match state {
                    0 => Some((
                        Ok::<Bytes, Infallible>(Bytes::from_static(
                            br#"{"object":"list","data":["#,
                        )),
                        1,
                    )),
                    1 => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        Some((
                            Ok::<Bytes, Infallible>(Bytes::from_static(
                                br#"{"id":"slow-model","object":"model"}]}"#,
                            )),
                            2,
                        ))
                    }
                    _ => None,
                }
            });
            return (
                StatusCode::OK,
                [(
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )],
                Body::from_stream(chunked),
            )
                .into_response();
        }

        (
            StatusCode::OK,
            Json(json!({
                "object": "list",
                "data": [
                    {
                        "id": "upstream-model-a",
                        "object": "model",
                        "owned_by": "upstream",
                        "created": 1712345678
                    },
                    {
                        "id": "gpt-5.2-codex",
                        "object": "model",
                        "owned_by": "upstream",
                        "created": 1712345679
                    }
                ]
            })),
        )
            .into_response()
    }

    async fn spawn_test_upstream() -> (String, JoinHandle<()>) {
        let app = Router::new()
            .route("/v1/echo", any(test_upstream_echo))
            .route("/v1/stream", any(test_upstream_stream))
            .route(
                "/v1/stream-first-error",
                any(test_upstream_stream_first_error),
            )
            .route("/v1/stream-mid-error", any(test_upstream_stream_mid_error))
            .route("/v1/slow-stream", any(test_upstream_slow_stream))
            .route("/v1/hang", any(test_upstream_hang))
            .route("/v1/models", get(test_upstream_models))
            .route("/v1/redirect", any(test_upstream_redirect))
            .route(
                "/v1/redirect-external",
                any(test_upstream_external_redirect),
            )
            .route(
                "/v1/chat/completions",
                any(test_upstream_chat_external_redirect),
            )
            .route("/v1/responses", any(test_upstream_responses))
            .route(
                "/v1/responses/compact",
                post(test_upstream_responses_compact),
            );

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind upstream test server");
        let addr = listener.local_addr().expect("upstream local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("upstream test server should run");
        });

        (format!("http://{addr}/"), handle)
    }

    async fn test_upstream_capture_target_echo(
        State(captured): State<Arc<Mutex<Vec<Value>>>>,
        body: Bytes,
    ) -> impl IntoResponse {
        let payload: Value = serde_json::from_slice(&body).expect("decode upstream captured body");
        captured.lock().await.push(payload.clone());

        (
            StatusCode::OK,
            Json(json!({
                "id": "resp_test",
                "object": "response",
                "model": "gpt-5.3-codex",
                "service_tier": "priority",
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 3,
                    "total_tokens": 15
                },
                "received": payload,
            })),
        )
    }

    async fn test_upstream_capture_target_compact_echo(
        State(captured): State<Arc<Mutex<Vec<Value>>>>,
        body: Bytes,
    ) -> impl IntoResponse {
        let payload: Value = serde_json::from_slice(&body).expect("decode upstream captured body");
        captured.lock().await.push(payload.clone());

        (
            StatusCode::OK,
            Json(json!({
                "id": "resp_compact_test",
                "object": "response.compaction",
                "output": [
                    {
                        "id": "cmp_001",
                        "type": "compaction",
                        "encrypted_content": "encrypted-summary"
                    }
                ],
                "usage": {
                    "input_tokens": 139,
                    "input_tokens_details": {
                        "cached_tokens": 11
                    },
                    "output_tokens": 438,
                    "output_tokens_details": {
                        "reasoning_tokens": 64
                    },
                    "total_tokens": 577
                },
                "received": payload,
            })),
        )
    }

    async fn spawn_capture_target_body_upstream() -> (String, Arc<Mutex<Vec<Value>>>, JoinHandle<()>)
    {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let app = Router::new()
            .route(
                "/v1/chat/completions",
                post(test_upstream_capture_target_echo),
            )
            .route("/v1/responses", post(test_upstream_capture_target_echo))
            .route(
                "/v1/responses/compact",
                post(test_upstream_capture_target_compact_echo),
            )
            .with_state(captured.clone());

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind capture-target upstream test server");
        let addr = listener
            .local_addr()
            .expect("capture-target upstream local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("capture-target upstream test server should run");
        });

        (format!("http://{addr}/"), captured, handle)
    }

    fn extract_model_ids(payload: &Value) -> Vec<String> {
        payload
            .get("data")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("id").and_then(|v| v.as_str()))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn build_proxy_upstream_url_preserves_path_prefix_and_query() {
        let base = Url::parse("https://proxy.example.com/gateway").expect("valid base");
        let uri: Uri = "/v1/models?limit=10".parse().expect("valid uri");
        let target = build_proxy_upstream_url(&base, &uri).expect("url should build");
        assert_eq!(
            target.as_str(),
            "https://proxy.example.com/gateway/v1/models?limit=10"
        );
    }

    #[test]
    fn build_proxy_upstream_url_supports_ipv6_literal_base() {
        let base = Url::parse("http://[::1]:8080/gateway/").expect("valid ipv6 base");
        let uri: Uri = "/v1/models?limit=10".parse().expect("valid uri");
        let target = build_proxy_upstream_url(&base, &uri).expect("url should build");
        assert_eq!(
            target.as_str(),
            "http://[::1]:8080/gateway/v1/models?limit=10"
        );
    }

    #[test]
    fn path_has_forbidden_dot_segment_detects_plain_and_encoded_variants() {
        assert!(path_has_forbidden_dot_segment("/v1/../models"));
        assert!(path_has_forbidden_dot_segment("/v1/%2e%2e/models"));
        assert!(path_has_forbidden_dot_segment("/v1/.%2E/models"));
        assert!(path_has_forbidden_dot_segment("/v1/%2e%2e%2fadmin"));
        assert!(path_has_forbidden_dot_segment("/v1/%2e%2e%5cadmin"));
        assert!(path_has_forbidden_dot_segment("/v1/%252e%252e%252fadmin"));
        assert!(!path_has_forbidden_dot_segment("/v1/%2efoo/models"));
        assert!(!path_has_forbidden_dot_segment("/v1/models"));
    }

    #[test]
    fn build_proxy_upstream_url_rejects_dot_segment_paths() {
        let base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
        let uri: Uri = "/v1/%2e%2e%2fadmin?scope=test"
            .parse()
            .expect("valid uri with dot segments");
        let err = build_proxy_upstream_url(&base, &uri).expect_err("dot segments should fail");
        assert!(
            err.to_string().contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED),
            "error should indicate forbidden dot segments: {err}"
        );
    }

    #[test]
    fn has_invalid_percent_encoding_detects_malformed_sequences() {
        assert!(has_invalid_percent_encoding("/v1/%zz/models"));
        assert!(has_invalid_percent_encoding("/v1/%/models"));
        assert!(has_invalid_percent_encoding("/v1/%2/models"));
        assert!(!has_invalid_percent_encoding("/v1/%2F/models"));
        assert!(!has_invalid_percent_encoding("/v1/models"));
    }

    #[test]
    fn should_proxy_header_filters_hop_by_hop_headers() {
        assert!(should_proxy_header(&http_header::AUTHORIZATION));
        assert!(should_proxy_header(&http_header::CONTENT_LENGTH));
        assert!(!should_proxy_header(&http_header::HOST));
        assert!(!should_proxy_header(&http_header::CONNECTION));
        assert!(!should_proxy_header(&http_header::TRANSFER_ENCODING));
        assert!(!should_proxy_header(&http_header::ACCEPT_ENCODING));
        assert!(!should_proxy_header(&HeaderName::from_static("forwarded")));
        assert!(!should_proxy_header(&HeaderName::from_static("via")));
        assert!(!should_proxy_header(&HeaderName::from_static(
            "x-forwarded-for"
        )));
        assert!(!should_proxy_header(&HeaderName::from_static(
            "x-forwarded-host"
        )));
        assert!(!should_proxy_header(&HeaderName::from_static(
            "x-forwarded-proto"
        )));
        assert!(!should_proxy_header(&HeaderName::from_static(
            "x-forwarded-port"
        )));
        assert!(!should_proxy_header(&HeaderName::from_static("x-real-ip")));
    }

    #[test]
    fn connection_scoped_header_names_parses_connection_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-foo"),
        );
        headers.append(http_header::CONNECTION, HeaderValue::from_static("x-bar"));
        let names = connection_scoped_header_names(&headers);
        assert!(names.contains(&http_header::HeaderName::from_static("keep-alive")));
        assert!(names.contains(&http_header::HeaderName::from_static("x-foo")));
        assert!(names.contains(&http_header::HeaderName::from_static("x-bar")));
    }

    #[test]
    fn request_may_have_body_uses_method_and_headers() {
        let empty = HeaderMap::new();
        assert!(!request_may_have_body(&Method::GET, &empty));
        assert!(request_may_have_body(&Method::POST, &empty));

        let mut with_length = HeaderMap::new();
        with_length.insert(http_header::CONTENT_LENGTH, HeaderValue::from_static("0"));
        assert!(!request_may_have_body(&Method::GET, &with_length));
        with_length.insert(http_header::CONTENT_LENGTH, HeaderValue::from_static("10"));
        assert!(request_may_have_body(&Method::GET, &with_length));
    }

    #[test]
    fn parse_cors_allowed_origins_normalizes_and_deduplicates() {
        let parsed = parse_cors_allowed_origins(
            "https://EXAMPLE.com:443, http://127.0.0.1:8080, https://example.com",
        )
        .expect("parse should succeed");
        assert_eq!(
            parsed,
            vec![
                "https://example.com".to_string(),
                "http://127.0.0.1:8080".to_string(),
            ]
        );
    }

    #[test]
    fn origin_allowed_accepts_loopback_and_configured_origins() {
        let configured = HashSet::from(["https://api.example.com".to_string()]);
        assert!(origin_allowed("http://127.0.0.1:60080", &configured));
        assert!(origin_allowed("https://api.example.com", &configured));
        assert!(!origin_allowed("https://evil.example.com", &configured));
    }

    #[test]
    fn same_origin_settings_write_allows_missing_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        assert!(is_same_origin_settings_write(&headers));
    }

    #[test]
    fn same_origin_settings_write_rejects_cross_site_without_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("cross-site"),
        );
        assert!(!is_same_origin_settings_write(&headers));
    }

    #[test]
    fn same_origin_settings_write_allows_matching_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:8080"),
        );
        assert!(is_same_origin_settings_write(&headers));
    }

    #[test]
    fn same_origin_settings_write_allows_matching_origin_without_explicit_host_port() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("proxy.example.com"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://proxy.example.com"),
        );
        assert!(is_same_origin_settings_write(&headers));
    }

    #[test]
    fn same_origin_settings_write_rejects_mismatched_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://evil.example.com"),
        );
        assert!(!is_same_origin_settings_write(&headers));
    }

    #[test]
    fn same_origin_settings_write_allows_loopback_proxy_port_mismatch() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:60080"),
        );
        assert!(is_same_origin_settings_write(&headers));
    }

    #[test]
    fn same_origin_settings_write_allows_forwarded_host_match() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://proxy.example.com"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("proxy.example.com"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-proto"),
            HeaderValue::from_static("https"),
        );
        headers.insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("same-origin"),
        );
        assert!(is_same_origin_settings_write(&headers));
    }

    #[test]
    fn same_origin_settings_write_allows_forwarded_port_for_non_default_origin_port() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://proxy.example.com:8443"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("proxy.example.com"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-proto"),
            HeaderValue::from_static("https"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-port"),
            HeaderValue::from_static("8443"),
        );
        headers.insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("same-origin"),
        );
        assert!(is_same_origin_settings_write(&headers));
    }

    #[test]
    fn same_origin_settings_write_rejects_multi_hop_forwarded_host_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://evil.example.com"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("evil.example.com, proxy.example.com"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-proto"),
            HeaderValue::from_static("https"),
        );
        assert!(!is_same_origin_settings_write(&headers));
    }

    #[test]
    fn same_origin_settings_write_rejects_cross_site_request() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://evil.example.com"),
        );
        headers.insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("cross-site"),
        );
        assert!(!is_same_origin_settings_write(&headers));
    }

    #[test]
    fn rewrite_proxy_location_path_strips_upstream_base_prefix() {
        let upstream_base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
        assert_eq!(
            rewrite_proxy_location_path("/gateway/v1/echo", &upstream_base),
            "/v1/echo"
        );
        assert_eq!(
            rewrite_proxy_location_path("/v1/echo", &upstream_base),
            "/v1/echo"
        );
    }

    #[test]
    fn normalize_proxy_location_header_strips_upstream_base_prefix_for_absolute_redirect() {
        let upstream_base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::LOCATION,
            HeaderValue::from_static("https://proxy.example.com/gateway/v1/echo?from=redirect"),
        );

        let normalized = normalize_proxy_location_header(
            StatusCode::TEMPORARY_REDIRECT,
            &headers,
            &upstream_base,
        )
        .expect("normalize should succeed");
        assert_eq!(normalized.as_deref(), Some("/v1/echo?from=redirect"));
    }

    #[test]
    fn normalize_proxy_location_header_strips_upstream_base_prefix_for_relative_redirect() {
        let upstream_base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::LOCATION,
            HeaderValue::from_static("/gateway/v1/echo?from=redirect#frag"),
        );

        let normalized = normalize_proxy_location_header(
            StatusCode::TEMPORARY_REDIRECT,
            &headers,
            &upstream_base,
        )
        .expect("normalize should succeed");
        assert_eq!(normalized.as_deref(), Some("/v1/echo?from=redirect#frag"));
    }

    #[tokio::test]
    async fn proxy_openai_v1_forwards_headers_method_query_and_body() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer test-token"),
        );
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("client.example.com"),
        );
        headers.insert(
            http_header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-foo"),
        );
        headers.insert(
            http_header::HeaderName::from_static("x-foo"),
            HeaderValue::from_static("should-not-forward"),
        );
        headers.insert(
            http_header::HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("198.51.100.20"),
        );
        headers.insert(
            http_header::HeaderName::from_static("via"),
            HeaderValue::from_static("1.1 browser-proxy"),
        );

        let uri: Uri = "/v1/echo?foo=bar".parse().expect("valid uri");
        let response = proxy_openai_v1(
            State(state),
            OriginalUri(uri),
            Method::POST,
            headers,
            Body::from("hello-proxy"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(
            response.headers().get("x-upstream"),
            Some(&HeaderValue::from_static("ok"))
        );
        assert!(response.headers().contains_key(http_header::CONTENT_LENGTH));
        assert!(
            !response
                .headers()
                .contains_key(http_header::HeaderName::from_static("x-upstream-hop"))
        );
        assert!(
            !response
                .headers()
                .contains_key(http_header::HeaderName::from_static("via"))
        );
        assert!(
            !response
                .headers()
                .contains_key(http_header::HeaderName::from_static("forwarded"))
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
        assert_eq!(payload["method"], "POST");
        assert_eq!(payload["path"], "/v1/echo");
        assert_eq!(payload["query"], "foo=bar");
        assert_eq!(payload["authorization"], "Bearer test-token");
        assert_ne!(payload["hostHeader"], "client.example.com");
        assert_eq!(payload["connectionSeen"], false);
        assert_eq!(payload["xFooSeen"], false);
        assert_eq!(payload["xForwardedForSeen"], false);
        assert_eq!(payload["forwardedSeen"], false);
        assert_eq!(payload["viaSeen"], false);
        assert_eq!(payload["body"], "hello-proxy");

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_model_settings_api_reads_and_persists_updates() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let Json(initial) = get_settings(State(state.clone()))
            .await
            .expect("get settings should succeed");
        assert!(!initial.proxy.hijack_enabled);
        assert!(!initial.proxy.merge_upstream_enabled);
        assert_eq!(
            initial.proxy.fast_mode_rewrite_mode,
            ProxyFastModeRewriteMode::Disabled
        );
        assert_eq!(initial.proxy.models.len(), PROXY_PRESET_MODEL_IDS.len());
        assert_eq!(
            initial.proxy.enabled_models,
            PROXY_PRESET_MODEL_IDS
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        );

        let Json(updated) = put_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: true,
                merge_upstream_enabled: true,
                fast_mode_rewrite_mode: ProxyFastModeRewriteMode::FillMissing,
                enabled_models: vec!["gpt-5.2-codex".to_string(), "unknown-model".to_string()],
            }),
        )
        .await
        .expect("put settings should succeed");
        assert!(updated.hijack_enabled);
        assert!(updated.merge_upstream_enabled);
        assert_eq!(
            updated.fast_mode_rewrite_mode,
            ProxyFastModeRewriteMode::FillMissing
        );
        assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);

        let persisted = load_proxy_model_settings(&state.pool)
            .await
            .expect("settings should persist");
        assert!(persisted.hijack_enabled);
        assert!(persisted.merge_upstream_enabled);
        assert_eq!(
            persisted.fast_mode_rewrite_mode,
            ProxyFastModeRewriteMode::FillMissing
        );
        assert_eq!(
            persisted.enabled_preset_models,
            vec!["gpt-5.2-codex".to_string()]
        );

        let Json(normalized) = put_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: false,
                merge_upstream_enabled: true,
                fast_mode_rewrite_mode: ProxyFastModeRewriteMode::ForcePriority,
                enabled_models: Vec::new(),
            }),
        )
        .await
        .expect("put settings should normalize payload");
        assert!(!normalized.hijack_enabled);
        assert!(!normalized.merge_upstream_enabled);
        assert_eq!(
            normalized.fast_mode_rewrite_mode,
            ProxyFastModeRewriteMode::ForcePriority
        );
        assert!(normalized.enabled_models.is_empty());
    }

    #[tokio::test]
    async fn ensure_schema_adds_fast_mode_rewrite_mode_with_disabled_default() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");

        sqlx::query(
            r#"
            CREATE TABLE proxy_model_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                hijack_enabled INTEGER NOT NULL DEFAULT 0,
                merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
                enabled_preset_models_json TEXT,
                preset_models_migrated INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create legacy proxy_model_settings table");

        sqlx::query(
            r#"
            INSERT INTO proxy_model_settings (
                id,
                hijack_enabled,
                merge_upstream_enabled,
                enabled_preset_models_json,
                preset_models_migrated
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
        .bind(1_i64)
        .bind(0_i64)
        .bind(
            serde_json::to_string(&default_enabled_preset_models())
                .expect("serialize default enabled models"),
        )
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("insert legacy proxy_model_settings row");

        ensure_schema(&pool)
            .await
            .expect("ensure schema migration run");

        let settings = load_proxy_model_settings(&pool)
            .await
            .expect("load proxy model settings");
        assert_eq!(
            settings.fast_mode_rewrite_mode,
            ProxyFastModeRewriteMode::Disabled
        );
    }

    #[tokio::test]
    async fn ensure_schema_appends_new_proxy_models_when_enabled_list_matches_legacy_default() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        let legacy_enabled = LEGACY_PROXY_PRESET_MODEL_IDS
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>();
        let legacy_enabled_json =
            serde_json::to_string(&legacy_enabled).expect("serialize legacy enabled list");

        sqlx::query(
            r#"
            UPDATE proxy_model_settings
            SET enabled_preset_models_json = ?1,
                preset_models_migrated = 0
            WHERE id = ?2
            "#,
        )
        .bind(legacy_enabled_json)
        .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("force legacy enabled preset models");

        ensure_schema(&pool).await.expect("ensure schema rerun");

        let settings = load_proxy_model_settings(&pool)
            .await
            .expect("load proxy model settings");
        assert!(
            settings
                .enabled_preset_models
                .contains(&"gpt-5.4".to_string())
        );
        assert!(
            settings
                .enabled_preset_models
                .contains(&"gpt-5.4-pro".to_string())
        );
    }

    #[tokio::test]
    async fn ensure_schema_does_not_append_new_proxy_models_when_enabled_list_is_custom() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        let custom_enabled = vec!["gpt-5.2-codex".to_string()];
        let custom_enabled_json =
            serde_json::to_string(&custom_enabled).expect("serialize custom enabled list");
        sqlx::query(
            r#"
            UPDATE proxy_model_settings
            SET enabled_preset_models_json = ?1,
                preset_models_migrated = 0
            WHERE id = ?2
            "#,
        )
        .bind(custom_enabled_json)
        .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("force custom enabled preset models");

        ensure_schema(&pool).await.expect("ensure schema rerun");

        let settings = load_proxy_model_settings(&pool)
            .await
            .expect("load proxy model settings");
        assert_eq!(settings.enabled_preset_models, custom_enabled);
    }

    #[tokio::test]
    async fn ensure_schema_allows_opting_out_of_new_proxy_models_after_migration() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        let legacy_enabled = LEGACY_PROXY_PRESET_MODEL_IDS
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>();
        let legacy_enabled_json =
            serde_json::to_string(&legacy_enabled).expect("serialize legacy enabled list");

        sqlx::query(
            r#"
            UPDATE proxy_model_settings
            SET enabled_preset_models_json = ?1,
                preset_models_migrated = 0
            WHERE id = ?2
            "#,
        )
        .bind(&legacy_enabled_json)
        .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("force legacy enabled preset models");

        ensure_schema(&pool)
            .await
            .expect("ensure schema migration run");
        let migrated = load_proxy_model_settings(&pool)
            .await
            .expect("load proxy model settings after migration");
        assert!(
            migrated
                .enabled_preset_models
                .contains(&"gpt-5.4".to_string())
        );
        assert!(
            migrated
                .enabled_preset_models
                .contains(&"gpt-5.4-pro".to_string())
        );

        // User explicitly removes the new models after migration; schema re-run should not
        // force them back in.
        sqlx::query(
            r#"
            UPDATE proxy_model_settings
            SET enabled_preset_models_json = ?1,
                preset_models_migrated = 1
            WHERE id = ?2
            "#,
        )
        .bind(&legacy_enabled_json)
        .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("force legacy enabled preset models after migration");

        ensure_schema(&pool).await.expect("ensure schema rerun");

        let settings = load_proxy_model_settings(&pool)
            .await
            .expect("load proxy model settings after opt-out");
        assert_eq!(settings.enabled_preset_models, legacy_enabled);
    }

    #[tokio::test]
    async fn ensure_schema_marks_proxy_preset_models_migrated_when_enabled_list_empty() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        sqlx::query(
            r#"
            UPDATE proxy_model_settings
            SET enabled_preset_models_json = ?1,
                preset_models_migrated = 0
            WHERE id = ?2
            "#,
        )
        .bind("[]")
        .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("force empty enabled preset models list");

        ensure_schema(&pool).await.expect("ensure schema rerun");

        let settings = load_proxy_model_settings(&pool)
            .await
            .expect("load proxy model settings");
        assert!(
            settings.enabled_preset_models.is_empty(),
            "empty enabled list should be preserved"
        );

        let migrated = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT preset_models_migrated
            FROM proxy_model_settings
            WHERE id = ?1
            LIMIT 1
            "#,
        )
        .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
        .fetch_one(&pool)
        .await
        .expect("read migration flag");
        assert_eq!(migrated, 1);
    }

    #[tokio::test]
    async fn proxy_model_settings_api_rejects_cross_origin_writes() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://evil.example.com"),
        );

        let err = put_proxy_settings(
            State(state),
            headers,
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: true,
                merge_upstream_enabled: true,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_models: vec!["gpt-5.2-codex".to_string()],
            }),
        )
        .await
        .expect_err("cross-origin write should be rejected");

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn proxy_model_settings_api_rejects_cross_site_request() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://evil.example.com"),
        );
        headers.insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("cross-site"),
        );

        let err = put_proxy_settings(
            State(state),
            headers,
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: true,
                merge_upstream_enabled: false,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_models: vec!["gpt-5.2-codex".to_string()],
            }),
        )
        .await
        .expect_err("cross-site request should be rejected");

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn proxy_model_settings_api_allows_loopback_proxy_origin_mismatch() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:60080"),
        );

        let Json(updated) = put_proxy_settings(
            State(state),
            headers,
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: true,
                merge_upstream_enabled: false,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_models: vec!["gpt-5.2-codex".to_string()],
            }),
        )
        .await
        .expect("loopback proxied write should be allowed");

        assert!(updated.hijack_enabled);
        assert!(!updated.merge_upstream_enabled);
        assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
    }

    #[tokio::test]
    async fn proxy_model_settings_api_allows_forwarded_host_origin_match() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://proxy.example.com"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("proxy.example.com"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-proto"),
            HeaderValue::from_static("https"),
        );
        headers.insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("same-origin"),
        );

        let Json(updated) = put_proxy_settings(
            State(state),
            headers,
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: true,
                merge_upstream_enabled: false,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_models: vec!["gpt-5.2-codex".to_string()],
            }),
        )
        .await
        .expect("forwarded host write should be allowed");

        assert!(updated.hijack_enabled);
        assert!(!updated.merge_upstream_enabled);
        assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
    }

    #[tokio::test]
    async fn proxy_model_settings_api_allows_forwarded_port_non_default_origin_port() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://proxy.example.com:8443"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("proxy.example.com"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-proto"),
            HeaderValue::from_static("https"),
        );
        headers.insert(
            HeaderName::from_static("x-forwarded-port"),
            HeaderValue::from_static("8443"),
        );
        headers.insert(
            HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("same-origin"),
        );

        let Json(updated) = put_proxy_settings(
            State(state),
            headers,
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: true,
                merge_upstream_enabled: false,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_models: vec!["gpt-5.2-codex".to_string()],
            }),
        )
        .await
        .expect("forwarded port write should be allowed");

        assert!(updated.hijack_enabled);
        assert!(!updated.merge_upstream_enabled);
        assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
    }

    #[tokio::test]
    async fn proxy_model_settings_api_allows_matching_origin_without_explicit_host_port() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            http_header::HOST,
            HeaderValue::from_static("proxy.example.com"),
        );
        headers.insert(
            http_header::ORIGIN,
            HeaderValue::from_static("https://proxy.example.com"),
        );

        let Json(updated) = put_proxy_settings(
            State(state),
            headers,
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: true,
                merge_upstream_enabled: false,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_models: vec!["gpt-5.2-codex".to_string()],
            }),
        )
        .await
        .expect("same-origin write without explicit host port should be allowed");

        assert!(updated.hijack_enabled);
        assert!(!updated.merge_upstream_enabled);
        assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
    }

    #[tokio::test]
    async fn forward_proxy_live_stats_returns_fixed_24_hour_buckets_with_zero_fill() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let Json(settings_response) = put_forward_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ForwardProxySettingsUpdateRequest {
                proxy_urls: vec!["socks5://127.0.0.1:1080".to_string()],
                subscription_urls: vec![],
                subscription_update_interval_secs: 3600,
                insert_direct: true,
            }),
        )
        .await
        .expect("put forward proxy settings should succeed");

        let direct_key = settings_response
            .nodes
            .iter()
            .find(|node| node.source == FORWARD_PROXY_SOURCE_DIRECT)
            .map(|node| node.key.clone())
            .expect("direct node should exist");
        let direct_weight = settings_response
            .nodes
            .iter()
            .find(|node| node.source == FORWARD_PROXY_SOURCE_DIRECT)
            .map(|node| node.weight)
            .expect("direct node weight should exist");
        let manual_key = settings_response
            .nodes
            .iter()
            .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
            .map(|node| node.key.clone())
            .expect("manual node should exist");

        let now = Utc::now();
        let range_end_epoch = align_bucket_epoch(now.timestamp(), 3600, 0) + 3600;
        let range_start_epoch = range_end_epoch - 24 * 3600;
        seed_forward_proxy_weight_bucket_at(
            &state.pool,
            &manual_key,
            range_start_epoch - 3600,
            4,
            0.25,
            0.42,
            0.34,
            0.35,
        )
        .await;
        seed_forward_proxy_weight_bucket_at(
            &state.pool,
            &manual_key,
            range_start_epoch + 5 * 3600,
            2,
            0.45,
            0.82,
            0.61,
            0.80,
        )
        .await;
        seed_forward_proxy_weight_bucket_at(
            &state.pool,
            &manual_key,
            range_start_epoch + 10 * 3600,
            1,
            1.20,
            1.20,
            1.20,
            1.20,
        )
        .await;
        seed_forward_proxy_attempt_at(
            &state.pool,
            &manual_key,
            now - ChronoDuration::minutes(12),
            true,
        )
        .await;
        seed_forward_proxy_attempt_at(
            &state.pool,
            &manual_key,
            now - ChronoDuration::hours(1) - ChronoDuration::minutes(8),
            false,
        )
        .await;
        seed_forward_proxy_attempt_at(
            &state.pool,
            &manual_key,
            now - ChronoDuration::hours(30),
            true,
        )
        .await;
        seed_forward_proxy_attempt_at(
            &state.pool,
            &direct_key,
            now - ChronoDuration::minutes(40),
            true,
        )
        .await;

        let Json(response) = fetch_forward_proxy_live_stats(State(state.clone()))
            .await
            .expect("fetch forward proxy live stats should succeed");

        assert_eq!(response.bucket_seconds, 3600);
        assert_eq!(response.nodes.len(), 2);
        assert_eq!(response.range_end, response.nodes[0].last24h[23].bucket_end);
        assert_eq!(
            response.range_start,
            response.nodes[0].last24h[0].bucket_start
        );

        for node in &response.nodes {
            assert_eq!(
                node.last24h.len(),
                24,
                "node {} should include fixed 24 buckets",
                node.key
            );
            assert_eq!(
                node.weight24h.len(),
                24,
                "node {} should include fixed 24 weight buckets",
                node.key
            );
        }

        let manual = response
            .nodes
            .iter()
            .find(|node| node.key == manual_key)
            .expect("manual node should be present");
        let manual_success_total: i64 = manual
            .last24h
            .iter()
            .map(|bucket| bucket.success_count)
            .sum();
        let manual_failure_total: i64 = manual
            .last24h
            .iter()
            .map(|bucket| bucket.failure_count)
            .sum();
        let manual_zero_buckets = manual
            .last24h
            .iter()
            .filter(|bucket| bucket.success_count == 0 && bucket.failure_count == 0)
            .count();
        assert_eq!(
            manual_success_total, 1,
            "out-of-range attempts should be excluded"
        );
        assert!(
            manual_failure_total >= 1,
            "expected at least one in-range failure attempt"
        );
        assert!(
            manual_zero_buckets >= 21,
            "expected most buckets to be zero-filled, got {manual_zero_buckets}"
        );
        assert!(
            manual
                .weight24h
                .iter()
                .any(|bucket| bucket.sample_count == 0 && (bucket.last_weight - 0.35).abs() < 1e-6)
        );

        let sampled_bucket_index = manual
            .weight24h
            .iter()
            .position(|bucket| {
                bucket.sample_count == 2
                    && (bucket.min_weight - 0.45).abs() < 1e-6
                    && (bucket.max_weight - 0.82).abs() < 1e-6
                    && (bucket.avg_weight - 0.61).abs() < 1e-6
                    && (bucket.last_weight - 0.80).abs() < 1e-6
            })
            .expect("expected sampled manual weight bucket with aggregated stats");
        let sampled_bucket_carry = manual
            .weight24h
            .get(sampled_bucket_index + 1)
            .expect("expected carry-forward bucket after sampled manual weight bucket");
        assert_eq!(sampled_bucket_carry.sample_count, 0);
        assert!((sampled_bucket_carry.last_weight - 0.80).abs() < 1e-6);

        let recovered_bucket_index = manual
            .weight24h
            .iter()
            .position(|bucket| {
                bucket.sample_count == 1
                    && (bucket.min_weight - 1.20).abs() < 1e-6
                    && (bucket.max_weight - 1.20).abs() < 1e-6
                    && (bucket.avg_weight - 1.20).abs() < 1e-6
                    && (bucket.last_weight - 1.20).abs() < 1e-6
            })
            .expect("expected sampled manual weight bucket with recovered last value");
        let recovered_bucket_carry = manual
            .weight24h
            .get(recovered_bucket_index + 1)
            .expect("expected carry-forward bucket after recovered manual weight bucket");
        assert_eq!(recovered_bucket_carry.sample_count, 0);
        assert!((recovered_bucket_carry.last_weight - 1.20).abs() < 1e-6);
        let direct = response
            .nodes
            .iter()
            .find(|node| node.key == direct_key)
            .expect("direct node should be present");
        let direct_success_total: i64 = direct
            .last24h
            .iter()
            .map(|bucket| bucket.success_count)
            .sum();
        let direct_failure_total: i64 = direct
            .last24h
            .iter()
            .map(|bucket| bucket.failure_count)
            .sum();
        assert_eq!(direct_success_total, 1);
        assert_eq!(direct_failure_total, 0);
        assert!(
            direct
                .weight24h
                .iter()
                .all(|bucket| bucket.sample_count == 0
                    && (bucket.last_weight - direct_weight).abs() < 1e-6)
        );

        let display_names = response
            .nodes
            .iter()
            .map(|node| node.display_name.as_str())
            .collect::<Vec<_>>();
        let mut sorted_display_names = display_names.clone();
        sorted_display_names.sort();
        assert_eq!(
            display_names, sorted_display_names,
            "live stats nodes should stay display-name sorted"
        );
    }

    #[tokio::test]
    async fn forward_proxy_live_stats_keeps_direct_node_and_zero_metrics_when_no_attempts() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let Json(response) = fetch_forward_proxy_live_stats(State(state))
            .await
            .expect("fetch forward proxy live stats should succeed");

        assert_eq!(response.bucket_seconds, 3600);
        assert_eq!(
            response.nodes.len(),
            1,
            "default runtime should only include direct node"
        );
        let direct = &response.nodes[0];
        assert_eq!(direct.source, FORWARD_PROXY_SOURCE_DIRECT);
        assert_eq!(direct.stats.one_minute.attempts, 0);
        assert_eq!(direct.stats.one_day.attempts, 0);
        assert_eq!(direct.last24h.len(), 24);
        assert_eq!(direct.weight24h.len(), 24);
        assert!(
            direct
                .last24h
                .iter()
                .all(|bucket| bucket.success_count == 0 && bucket.failure_count == 0)
        );
        assert!(
            direct
                .weight24h
                .iter()
                .all(|bucket| bucket.sample_count == 0
                    && (bucket.last_weight - direct.weight).abs() < 1e-6)
        );
    }

    #[tokio::test]
    async fn upsert_forward_proxy_weight_hourly_bucket_keeps_latest_sample_weight() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;
        let proxy_key = "manual://latest-sample-weight";
        let bucket_start_epoch = align_bucket_epoch(Utc::now().timestamp(), 3600, 0);

        upsert_forward_proxy_weight_hourly_bucket(
            &state.pool,
            proxy_key,
            bucket_start_epoch,
            0.90,
            1_000_000,
        )
        .await
        .expect("seed first weight sample");
        upsert_forward_proxy_weight_hourly_bucket(
            &state.pool,
            proxy_key,
            bucket_start_epoch,
            1.10,
            2_000_000,
        )
        .await
        .expect("seed second weight sample");
        upsert_forward_proxy_weight_hourly_bucket(
            &state.pool,
            proxy_key,
            bucket_start_epoch,
            0.70,
            1_500_000,
        )
        .await
        .expect("seed out-of-order weight sample");

        let row = sqlx::query(
            r#"
            SELECT
                sample_count,
                min_weight,
                max_weight,
                avg_weight,
                last_weight,
                last_sample_epoch_us
            FROM forward_proxy_weight_hourly
            WHERE proxy_key = ?1 AND bucket_start_epoch = ?2
            "#,
        )
        .bind(proxy_key)
        .bind(bucket_start_epoch)
        .fetch_one(&state.pool)
        .await
        .expect("fetch aggregated weight bucket");

        assert_eq!(
            row.try_get::<i64, _>("sample_count").expect("sample_count"),
            3
        );
        assert!((row.try_get::<f64, _>("min_weight").expect("min_weight") - 0.70).abs() < 1e-6);
        assert!((row.try_get::<f64, _>("max_weight").expect("max_weight") - 1.10).abs() < 1e-6);
        assert!((row.try_get::<f64, _>("avg_weight").expect("avg_weight") - 0.90).abs() < 1e-6);
        assert!((row.try_get::<f64, _>("last_weight").expect("last_weight") - 1.10).abs() < 1e-6);
        assert_eq!(
            row.try_get::<i64, _>("last_sample_epoch_us")
                .expect("last_sample_epoch_us"),
            2_000_000
        );
    }

    #[tokio::test]
    async fn pricing_settings_api_reads_and_persists_updates() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let Json(initial) = get_settings(State(state.clone()))
            .await
            .expect("get settings should succeed");
        assert!(!initial.pricing.entries.is_empty());
        assert!(
            initial
                .pricing
                .entries
                .iter()
                .any(|entry| entry.model == "gpt-5.2-codex")
        );

        let Json(updated) = put_pricing_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(PricingSettingsUpdateRequest {
                catalog_version: "custom-ci".to_string(),
                entries: vec![PricingEntry {
                    model: "gpt-5.2-codex".to_string(),
                    input_per_1m: 8.8,
                    output_per_1m: 18.8,
                    cache_input_per_1m: Some(0.88),
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                }],
            }),
        )
        .await
        .expect("put pricing settings should succeed");

        assert_eq!(updated.catalog_version, "custom-ci");
        assert_eq!(updated.entries.len(), 1);
        assert_eq!(updated.entries[0].model, "gpt-5.2-codex");
        assert_eq!(updated.entries[0].input_per_1m, 8.8);

        let persisted = load_pricing_catalog(&state.pool)
            .await
            .expect("pricing settings should persist");
        assert_eq!(persisted.version, "custom-ci");
        assert_eq!(persisted.models.len(), 1);
        let pricing = persisted
            .models
            .get("gpt-5.2-codex")
            .expect("gpt-5.2-codex should persist");
        assert_eq!(pricing.input_per_1m, 8.8);
        assert_eq!(pricing.output_per_1m, 18.8);
    }

    #[tokio::test]
    async fn pricing_settings_api_keeps_empty_catalog_after_reload() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let Json(updated) = put_pricing_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(PricingSettingsUpdateRequest {
                catalog_version: "custom-empty".to_string(),
                entries: vec![],
            }),
        )
        .await
        .expect("put pricing settings should allow empty catalog");

        assert_eq!(updated.catalog_version, "custom-empty");
        assert!(updated.entries.is_empty());

        let first_reload = load_pricing_catalog(&state.pool)
            .await
            .expect("pricing catalog should load after update");
        assert_eq!(first_reload.version, "custom-empty");
        assert!(first_reload.models.is_empty());

        let second_reload = load_pricing_catalog(&state.pool)
            .await
            .expect("pricing catalog should stay empty across reloads");
        assert_eq!(second_reload.version, "custom-empty");
        assert!(second_reload.models.is_empty());
    }

    #[tokio::test]
    async fn pricing_settings_api_rejects_invalid_payload() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.example.com/").expect("valid upstream base url"),
        )
        .await;

        let err = put_pricing_settings(
            State(state),
            HeaderMap::new(),
            Json(PricingSettingsUpdateRequest {
                catalog_version: "   ".to_string(),
                entries: vec![],
            }),
        )
        .await
        .expect_err("blank catalog version should be rejected");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn seed_default_pricing_catalog_migrates_legacy_file_when_present() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");
        sqlx::query("DELETE FROM pricing_settings_meta")
            .execute(&pool)
            .await
            .expect("clear pricing meta");
        sqlx::query("DELETE FROM pricing_settings_models")
            .execute(&pool)
            .await
            .expect("clear pricing models");

        let legacy_path = env::temp_dir().join(format!(
            "codex-vibe-monitor-pricing-legacy-{}.json",
            NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(
            &legacy_path,
            r#"{
  "version": "legacy-custom-v1",
  "models": {
    "gpt-legacy": {
      "input_per_1m": 9.9,
      "output_per_1m": 19.9,
      "cache_input_per_1m": 0.99,
      "reasoning_per_1m": null
    }
  }
}"#,
        )
        .expect("write legacy pricing catalog");

        seed_default_pricing_catalog_with_legacy_path(&pool, Some(&legacy_path))
            .await
            .expect("seed pricing catalog from legacy file");

        let _ = fs::remove_file(&legacy_path);

        let migrated = load_pricing_catalog(&pool)
            .await
            .expect("load migrated pricing catalog");
        assert_eq!(migrated.version, "legacy-custom-v1");
        assert_eq!(migrated.models.len(), 1);
        let model = migrated
            .models
            .get("gpt-legacy")
            .expect("legacy model should be migrated");
        assert_eq!(model.input_per_1m, 9.9);
        assert_eq!(model.output_per_1m, 19.9);
        assert_eq!(model.cache_input_per_1m, Some(0.99));
        assert_eq!(model.source, "custom");
    }

    #[tokio::test]
    async fn seed_default_pricing_catalog_falls_back_when_legacy_file_empty() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");
        sqlx::query("DELETE FROM pricing_settings_meta")
            .execute(&pool)
            .await
            .expect("clear pricing meta");
        sqlx::query("DELETE FROM pricing_settings_models")
            .execute(&pool)
            .await
            .expect("clear pricing models");

        let legacy_path = env::temp_dir().join(format!(
            "codex-vibe-monitor-pricing-legacy-empty-{}.json",
            NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(
            &legacy_path,
            r#"{
  "version": "legacy-empty",
  "models": {}
}"#,
        )
        .expect("write empty legacy pricing catalog");

        seed_default_pricing_catalog_with_legacy_path(&pool, Some(&legacy_path))
            .await
            .expect("seed pricing catalog should fall back to defaults");

        let _ = fs::remove_file(&legacy_path);

        let seeded = load_pricing_catalog(&pool)
            .await
            .expect("load seeded pricing catalog");
        assert_eq!(seeded.version, DEFAULT_PRICING_CATALOG_VERSION);
        assert!(
            seeded.models.contains_key("gpt-5.2-codex"),
            "default pricing catalog should be seeded"
        );
    }

    #[tokio::test]
    async fn seed_default_pricing_catalog_auto_inserts_new_models_for_legacy_default_version() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        sqlx::query(
            r#"
            UPDATE pricing_settings_meta
            SET catalog_version = ?1
            WHERE id = ?2
            "#,
        )
        .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("downgrade pricing catalog version for test");
        sqlx::query(
            r#"
            DELETE FROM pricing_settings_models
            WHERE model IN ('gpt-5.4', 'gpt-5.4-pro')
            "#,
        )
        .execute(&pool)
        .await
        .expect("delete new pricing models for test");

        let catalog = load_pricing_catalog(&pool)
            .await
            .expect("load pricing catalog should succeed");
        assert!(catalog.models.contains_key("gpt-5.4"));
        assert!(catalog.models.contains_key("gpt-5.4-pro"));
    }

    #[tokio::test]
    async fn seed_default_pricing_catalog_normalizes_gpt_5_3_codex_source_for_legacy_default_version()
     {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        save_pricing_catalog(&pool, &default_pricing_catalog())
            .await
            .expect("seed default pricing catalog");

        sqlx::query(
            r#"
            UPDATE pricing_settings_meta
            SET catalog_version = ?1
            WHERE id = ?2
            "#,
        )
        .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("downgrade catalog version for test");

        sqlx::query(
            r#"
            UPDATE pricing_settings_models
            SET source = 'temporary'
            WHERE model = 'gpt-5.3-codex'
            "#,
        )
        .execute(&pool)
        .await
        .expect("force legacy gpt-5.3-codex source");

        let catalog = load_pricing_catalog(&pool)
            .await
            .expect("load pricing catalog");
        let pricing = catalog
            .models
            .get("gpt-5.3-codex")
            .expect("gpt-5.3-codex pricing present");
        assert_eq!(pricing.source, "official");
    }

    #[tokio::test]
    async fn seed_default_pricing_catalog_does_not_auto_insert_new_models_for_custom_catalog_version()
     {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        sqlx::query(
            r#"
            UPDATE pricing_settings_meta
            SET catalog_version = ?1
            WHERE id = ?2
            "#,
        )
        .bind("custom-ci")
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("set custom pricing catalog version for test");
        sqlx::query(
            r#"
            DELETE FROM pricing_settings_models
            WHERE model IN ('gpt-5.4', 'gpt-5.4-pro')
            "#,
        )
        .execute(&pool)
        .await
        .expect("delete new pricing models for test");

        let catalog = load_pricing_catalog(&pool)
            .await
            .expect("load pricing catalog should succeed");
        assert!(!catalog.models.contains_key("gpt-5.4"));
        assert!(!catalog.models.contains_key("gpt-5.4-pro"));
    }

    #[tokio::test]
    async fn seed_default_pricing_catalog_does_not_override_existing_pricing_for_new_models() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        // Simulate a legacy default catalog version so startup seeding will call
        // ensure_pricing_models_present, which must not overwrite existing rows.
        sqlx::query(
            r#"
            UPDATE pricing_settings_meta
            SET catalog_version = ?1
            WHERE id = ?2
            "#,
        )
        .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("set legacy pricing catalog version for test");

        sqlx::query(
            r#"
            UPDATE pricing_settings_models
            SET input_per_1m = ?1,
                output_per_1m = ?2,
                cache_input_per_1m = ?3,
                source = 'custom'
            WHERE model = 'gpt-5.4'
            "#,
        )
        .bind(99.0)
        .bind(199.0)
        .bind(Some(9.9))
        .execute(&pool)
        .await
        .expect("override gpt-5.4 pricing for test");

        sqlx::query(
            r#"
            UPDATE pricing_settings_models
            SET input_per_1m = ?1,
                output_per_1m = ?2,
                source = 'custom'
            WHERE model = 'gpt-5.4-pro'
            "#,
        )
        .bind(88.0)
        .bind(188.0)
        .execute(&pool)
        .await
        .expect("override gpt-5.4-pro pricing for test");

        let catalog = load_pricing_catalog(&pool)
            .await
            .expect("load pricing catalog should succeed");
        let gpt_5_4 = catalog.models.get("gpt-5.4").expect("gpt-5.4 should exist");
        assert_eq!(gpt_5_4.input_per_1m, 99.0);
        assert_eq!(gpt_5_4.output_per_1m, 199.0);
        assert_eq!(gpt_5_4.cache_input_per_1m, Some(9.9));
        assert_eq!(gpt_5_4.source, "custom");

        let gpt_5_4_pro = catalog
            .models
            .get("gpt-5.4-pro")
            .expect("gpt-5.4-pro should exist");
        assert_eq!(gpt_5_4_pro.input_per_1m, 88.0);
        assert_eq!(gpt_5_4_pro.output_per_1m, 188.0);
        assert_eq!(gpt_5_4_pro.cache_input_per_1m, None);
        assert_eq!(gpt_5_4_pro.source, "custom");
    }

    #[tokio::test]
    async fn proxy_openai_v1_models_passthrough_when_hijack_disabled() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/models".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
        let ids = extract_model_ids(&payload);
        assert_eq!(
            ids,
            vec!["upstream-model-a".to_string(), "gpt-5.2-codex".to_string()]
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_models_returns_preset_when_hijack_enabled_without_merge() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        {
            let mut settings = state.proxy_model_settings.write().await;
            *settings = ProxyModelSettings {
                hijack_enabled: true,
                merge_upstream_enabled: false,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_preset_models: vec!["gpt-5.3-codex".to_string(), "gpt-5.2".to_string()],
            };
        }

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/models".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .headers()
                .get(PROXY_MODEL_MERGE_STATUS_HEADER)
                .is_none()
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode hijacked payload");
        let ids = extract_model_ids(&payload);
        assert_eq!(
            ids,
            vec!["gpt-5.3-codex".to_string(), "gpt-5.2".to_string()]
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_models_returns_gpt_5_4_models_when_enabled() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        {
            let mut settings = state.proxy_model_settings.write().await;
            *settings = ProxyModelSettings {
                hijack_enabled: true,
                merge_upstream_enabled: false,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_preset_models: vec!["gpt-5.4".to_string(), "gpt-5.4-pro".to_string()],
            };
        }

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/models".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode hijacked payload");
        let ids = extract_model_ids(&payload);
        assert_eq!(ids, vec!["gpt-5.4".to_string(), "gpt-5.4-pro".to_string()]);

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_models_merges_upstream_when_enabled() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        {
            let mut settings = state.proxy_model_settings.write().await;
            *settings = ProxyModelSettings {
                hijack_enabled: true,
                merge_upstream_enabled: true,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_preset_models: vec![
                    "gpt-5.2-codex".to_string(),
                    "gpt-5.1-codex-mini".to_string(),
                ],
            };
        }

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/models".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
            Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_SUCCESS))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode merged payload");
        let ids = extract_model_ids(&payload);

        assert!(ids.contains(&"upstream-model-a".to_string()));
        assert!(ids.contains(&"gpt-5.2-codex".to_string()));
        assert!(ids.contains(&"gpt-5.1-codex-mini".to_string()));
        assert!(!ids.contains(&"gpt-5.3-codex".to_string()));
        assert_eq!(
            ids.iter()
                .filter(|id| id.as_str() == "gpt-5.2-codex")
                .count(),
            1
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_models_falls_back_to_preset_when_merge_upstream_fails() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        {
            let mut settings = state.proxy_model_settings.write().await;
            *settings = ProxyModelSettings {
                hijack_enabled: true,
                merge_upstream_enabled: true,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
            };
        }

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/models?mode=error".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
            Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
        let ids = extract_model_ids(&payload);
        assert_eq!(ids, vec!["gpt-5.1-codex-mini".to_string()]);

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_models_falls_back_when_merge_body_decode_times_out() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse(&upstream_base).expect("valid upstream base url");
        config.openai_proxy_handshake_timeout = Duration::from_millis(100);
        let http_clients = HttpClients::build(&config).expect("http clients");
        let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
        let (broadcaster, _rx) = broadcast::channel(16);
        let state = Arc::new(AppState {
            config: config.clone(),
            pool,
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            startup_ready: Arc::new(AtomicBool::new(true)),
            semaphore,
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings {
                hijack_enabled: true,
                merge_upstream_enabled: true,
                fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
                enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
            })),
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
        });

        let started = Instant::now();
        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/models?mode=slow-body".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "merge fallback should return quickly when decode times out"
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
            Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read fallback response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
        let ids = extract_model_ids(&payload);
        assert_eq!(ids, vec!["gpt-5.1-codex-mini".to_string()]);

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_preserves_streaming_response() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/stream".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(http_header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/event-stream"))
        );

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read stream body");
        assert_eq!(&body[..], b"chunk-achunk-b");

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_returns_bad_gateway_when_first_stream_chunk_fails() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/stream-first-error".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains("upstream stream error before first chunk")
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_propagates_stream_error_after_first_chunk() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/stream-mid-error".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let err = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect_err("mid-stream upstream failure should surface to downstream");
        assert!(
            err.to_string().contains("upstream stream error"),
            "unexpected stream error text: {err}"
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_preserves_redirect_without_following() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/redirect".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers().get(http_header::LOCATION),
            Some(&HeaderValue::from_static("/v1/echo?from=redirect"))
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_blocks_cross_origin_redirect() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/redirect-external".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains("cross-origin redirect is not allowed")
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_capture_target_persists_record_on_redirect_rewrite_error() {
        #[derive(sqlx::FromRow)]
        struct PersistedRow {
            source: String,
            status: Option<String>,
            error_message: Option<String>,
            t_total_ms: Option<f64>,
            t_req_read_ms: Option<f64>,
            t_req_parse_ms: Option<f64>,
            t_upstream_connect_ms: Option<f64>,
        }

        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from(
                r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"hi"}]}"#,
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains("cross-origin redirect is not allowed")
        );

        let row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT source, status, error_message, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record")
        .expect("capture record should be persisted");

        assert_eq!(row.source, SOURCE_PROXY);
        assert_eq!(row.status.as_deref(), Some("http_502"));
        assert!(
            row.error_message
                .as_deref()
                .is_some_and(|msg| msg.contains("cross-origin redirect is not allowed"))
        );
        assert!(row.t_total_ms.is_some_and(|v| v > 0.0));
        assert!(row.t_req_read_ms.is_some_and(|v| v >= 0.0));
        assert!(row.t_req_parse_ms.is_some_and(|v| v >= 0.0));
        assert!(row.t_upstream_connect_ms.is_some_and(|v| v >= 0.0));

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_capture_persist_and_broadcast_emits_records_summary_and_quota() {
        let state = test_state_with_openai_base(
            Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
        )
        .await;
        let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        seed_quota_snapshot(&state.pool, &now_local).await;

        let mut rx = state.broadcaster.subscribe();
        let invoke_id = "proxy-sse-broadcast-success";
        persist_and_broadcast_proxy_capture(
            state.as_ref(),
            Instant::now(),
            test_proxy_capture_record(invoke_id, &now_local),
        )
        .await
        .expect("persist+broadcast should succeed");

        let mut saw_record = false;
        let mut captured_record: Option<ApiInvocation> = None;
        let mut saw_quota = false;
        let mut summary_windows = HashSet::new();
        let expected_summary_windows = summary_broadcast_specs().len();
        for _ in 0..16 {
            let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
                .await
                .expect("timed out waiting for proxy broadcast event")
                .expect("broadcast channel should stay open");
            match payload {
                BroadcastPayload::Records { records } => {
                    if let Some(record) = records
                        .into_iter()
                        .find(|record| record.invoke_id == invoke_id)
                    {
                        saw_record = true;
                        captured_record = Some(record);
                    }
                }
                BroadcastPayload::Summary { window, summary } => {
                    summary_windows.insert(window.clone());
                    if window == "all" {
                        assert_eq!(summary.total_count, 1);
                    }
                }
                BroadcastPayload::Quota { snapshot } => {
                    saw_quota = true;
                    assert_eq!(snapshot.total_requests, 9);
                }
                BroadcastPayload::Version { .. } => {}
            }

            if saw_record && saw_quota && summary_windows.len() == expected_summary_windows {
                break;
            }
        }

        assert!(saw_record, "records payload should be broadcast");
        assert!(saw_quota, "quota payload should be broadcast");
        assert_eq!(
            summary_windows.len(),
            expected_summary_windows,
            "all summary windows should be broadcast"
        );
        let record = captured_record.expect("target records payload should include invoke id");
        assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
        assert_eq!(record.requester_ip.as_deref(), Some("198.51.100.77"));
        assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-broadcast-1"));
        assert_eq!(record.requested_service_tier.as_deref(), Some("priority"));
        assert_eq!(record.reasoning_effort.as_deref(), Some("high"));
        assert!(record.failure_kind.is_none());
    }

    #[tokio::test]
    async fn proxy_capture_persist_and_broadcast_skips_duplicate_records() {
        let state = test_state_with_openai_base(
            Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
        )
        .await;
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        let invoke_id = "proxy-sse-broadcast-duplicate";
        let mut rx = state.broadcaster.subscribe();

        persist_and_broadcast_proxy_capture(
            state.as_ref(),
            Instant::now(),
            test_proxy_capture_record(invoke_id, &occurred_at),
        )
        .await
        .expect("initial persist+broadcast should succeed");

        drain_broadcast_messages(&mut rx).await;

        persist_and_broadcast_proxy_capture(
            state.as_ref(),
            Instant::now(),
            test_proxy_capture_record(invoke_id, &occurred_at),
        )
        .await
        .expect("duplicate persist should not fail");

        let deadline = Instant::now() + Duration::from_millis(400);
        while Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
                Ok(Ok(BroadcastPayload::Records { records })) => {
                    assert!(
                        records.iter().all(|record| record.invoke_id != invoke_id),
                        "duplicate insert should not emit records payload for the same invoke_id"
                    );
                }
                Ok(Ok(_)) => continue,
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Err(_) => continue,
            }
        }
    }

    #[tokio::test]
    async fn fetch_and_store_skips_summary_and_quota_collection_when_broadcast_state_disabled() {
        let state = test_state_with_openai_base(
            Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
        )
        .await;
        let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        seed_quota_snapshot(&state.pool, &now_local).await;

        let publish = fetch_and_store(state.as_ref(), false, false)
            .await
            .expect("fetch_and_store should succeed");

        assert!(publish.summaries.is_empty());
        assert!(publish.quota_snapshot.is_none());
        assert!(!publish.collected_broadcast_state);
    }

    #[test]
    fn should_collect_late_broadcast_state_when_subscribers_arrive_mid_poll() {
        assert!(should_collect_late_broadcast_state(1, false));
        assert!(!should_collect_late_broadcast_state(0, false));
        assert!(!should_collect_late_broadcast_state(1, true));
    }

    #[tokio::test]
    async fn broadcast_summary_if_changed_skips_duplicate_payloads() {
        let state = test_state_with_openai_base(
            Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
        )
        .await;
        let mut rx = state.broadcaster.subscribe();
        let first = StatsResponse {
            total_count: 1,
            success_count: 1,
            failure_count: 0,
            total_cost: 0.5,
            total_tokens: 42,
        };

        assert!(
            broadcast_summary_if_changed(
                &state.broadcaster,
                state.broadcast_state_cache.as_ref(),
                "1d",
                first.clone(),
            )
            .await
            .expect("first summary broadcast should succeed")
        );

        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for first summary payload")
            .expect("broadcast should stay open");
        match payload {
            BroadcastPayload::Summary { window, summary } => {
                assert_eq!(window, "1d");
                assert_eq!(summary, first);
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        assert!(
            !broadcast_summary_if_changed(
                &state.broadcaster,
                state.broadcast_state_cache.as_ref(),
                "1d",
                first.clone(),
            )
            .await
            .expect("duplicate summary broadcast should succeed")
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );

        let updated = StatsResponse {
            total_count: 2,
            ..first
        };
        assert!(
            broadcast_summary_if_changed(
                &state.broadcaster,
                state.broadcast_state_cache.as_ref(),
                "1d",
                updated.clone(),
            )
            .await
            .expect("changed summary broadcast should succeed")
        );

        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for updated summary payload")
            .expect("broadcast should stay open");
        match payload {
            BroadcastPayload::Summary { window, summary } => {
                assert_eq!(window, "1d");
                assert_eq!(summary, updated);
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn broadcast_quota_if_changed_skips_duplicate_payloads() {
        let state = test_state_with_openai_base(
            Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
        )
        .await;
        let mut rx = state.broadcaster.subscribe();
        let first = QuotaSnapshotResponse {
            captured_at: "2026-03-07 10:00:00".to_string(),
            amount_limit: Some(100.0),
            used_amount: Some(10.0),
            remaining_amount: Some(90.0),
            period: Some("monthly".to_string()),
            period_reset_time: Some("2026-04-01 00:00:00".to_string()),
            expire_time: None,
            is_active: true,
            total_cost: 10.0,
            total_requests: 9,
            total_tokens: 150,
            last_request_time: Some("2026-03-07 10:00:00".to_string()),
            billing_type: Some("prepaid".to_string()),
            remaining_count: Some(91),
            used_count: Some(9),
            sub_type_name: Some("unit".to_string()),
        };

        assert!(
            broadcast_quota_if_changed(
                &state.broadcaster,
                state.broadcast_state_cache.as_ref(),
                first.clone(),
            )
            .await
            .expect("first quota broadcast should succeed")
        );

        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for first quota payload")
            .expect("broadcast should stay open");
        match payload {
            BroadcastPayload::Quota { snapshot } => {
                assert_eq!(*snapshot, first);
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        assert!(
            !broadcast_quota_if_changed(
                &state.broadcaster,
                state.broadcast_state_cache.as_ref(),
                first.clone(),
            )
            .await
            .expect("duplicate quota broadcast should succeed")
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );

        let updated = QuotaSnapshotResponse {
            total_requests: 10,
            ..first
        };
        assert!(
            broadcast_quota_if_changed(
                &state.broadcaster,
                state.broadcast_state_cache.as_ref(),
                updated.clone(),
            )
            .await
            .expect("changed quota broadcast should succeed")
        );

        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for updated quota payload")
            .expect("broadcast should stay open");
        match payload {
            BroadcastPayload::Quota { snapshot } => {
                assert_eq!(*snapshot, updated);
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_request_body_timeout_returns_408() {
        #[derive(sqlx::FromRow)]
        struct PersistedRow {
            status: Option<String>,
            error_message: Option<String>,
            payload: Option<String>,
        }

        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base_body_limit_and_read_timeout(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
            Duration::from_millis(50),
        )
        .await;

        let slow_body = stream::unfold(0u8, |state| async move {
            match state {
                0 => {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    Some((Ok::<Bytes, Infallible>(Bytes::from_static(br#"{}"#)), 1))
                }
                _ => None,
            }
        });

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from_stream(slow_body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains("request body read timed out")
        );

        let row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT status, error_message, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record")
        .expect("capture record should be persisted");

        assert_eq!(row.status.as_deref(), Some("failed"));
        assert!(
            row.error_message
                .as_deref()
                .is_some_and(|msg| msg.contains("[request_body_read_timeout]"))
        );
        let payload_json: Value = serde_json::from_str(
            row.payload
                .as_deref()
                .expect("capture payload should be present"),
        )
        .expect("decode capture payload");
        assert_eq!(
            payload_json["failureKind"].as_str(),
            Some("request_body_read_timeout")
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn capture_target_client_body_disconnect_returns_400_with_failure_kind() {
        #[derive(sqlx::FromRow)]
        struct PersistedRow {
            status: Option<String>,
            error_message: Option<String>,
            payload: Option<String>,
        }

        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let disconnected_body = stream::iter(vec![Err::<Bytes, io::Error>(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "client disconnected",
        ))]);

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from_stream(disconnected_body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains("failed to read request body stream")
        );

        let row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT status, error_message, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record")
        .expect("capture record should be persisted");

        assert_eq!(row.status.as_deref(), Some("failed"));
        assert!(
            row.error_message
                .as_deref()
                .is_some_and(|msg| msg.contains("[request_body_stream_error_client_closed]"))
        );
        let payload_json: Value = serde_json::from_str(
            row.payload
                .as_deref()
                .expect("capture payload should be present"),
        )
        .expect("decode capture payload");
        assert_eq!(
            payload_json["failureKind"].as_str(),
            Some("request_body_stream_error_client_closed")
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn capture_target_stream_error_emits_failure_kind_and_persists() {
        #[derive(sqlx::FromRow)]
        struct PersistedRow {
            status: Option<String>,
            error_message: Option<String>,
            payload: Option<String>,
        }

        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let request_body = serde_json::to_vec(&json!({
            "model": "gpt-5.2-codex",
            "stream": true,
            "input": "hello"
        }))
        .expect("serialize request body");

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from(request_body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let err = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect_err("mid-stream upstream failure should surface to downstream");
        assert!(
            err.to_string().contains("upstream stream error"),
            "unexpected stream error text: {err}"
        );

        let mut row: Option<PersistedRow> = None;
        for _ in 0..20 {
            row = sqlx::query_as::<_, PersistedRow>(
                r#"
                SELECT status, error_message, payload
                FROM codex_invocations
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .fetch_optional(&state.pool)
            .await
            .expect("query capture record");
            if row.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let row = row.expect("capture record should be persisted");

        assert_eq!(row.status.as_deref(), Some("http_200"));
        assert!(
            row.error_message
                .as_deref()
                .is_some_and(|msg| msg.contains("[upstream_stream_error]"))
        );
        let payload_json: Value = serde_json::from_str(
            row.payload
                .as_deref()
                .expect("capture payload should be present"),
        )
        .expect("decode capture payload");
        assert_eq!(
            payload_json["failureKind"].as_str(),
            Some("upstream_stream_error")
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_capture_target_fill_missing_rewrites_priority_for_responses() {
        #[derive(sqlx::FromRow)]
        struct PersistedRow {
            payload: Option<String>,
        }

        let (upstream_base, captured_requests, upstream_handle) =
            spawn_capture_target_body_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        {
            let mut settings = state.proxy_model_settings.write().await;
            settings.fast_mode_rewrite_mode = ProxyFastModeRewriteMode::FillMissing;
        }

        let request_body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": false,
            "input": "hello"
        }))
        .expect("serialize request body");

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from(request_body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let _response_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response body");
        let captured = captured_requests.lock().await;
        let captured_request = captured
            .first()
            .cloned()
            .expect("upstream should receive a request body");
        drop(captured);
        assert_eq!(captured_request["service_tier"], "priority");
        assert!(captured_request.get("serviceTier").is_none());

        let mut row: Option<PersistedRow> = None;
        for _ in 0..20 {
            row = sqlx::query_as::<_, PersistedRow>(
                r#"
                SELECT payload
                FROM codex_invocations
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .fetch_optional(&state.pool)
            .await
            .expect("query capture record");
            if row.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let row = row.expect("capture record should be persisted");
        let payload_json: Value = serde_json::from_str(
            row.payload
                .as_deref()
                .expect("capture payload should be present"),
        )
        .expect("decode capture payload");
        assert_eq!(payload_json["requestedServiceTier"], "priority");

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_capture_target_force_priority_overrides_existing_chat_tier() {
        #[derive(sqlx::FromRow)]
        struct PersistedRow {
            payload: Option<String>,
        }

        let (upstream_base, captured_requests, upstream_handle) =
            spawn_capture_target_body_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        {
            let mut settings = state.proxy_model_settings.write().await;
            settings.fast_mode_rewrite_mode = ProxyFastModeRewriteMode::ForcePriority;
        }

        let request_body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": false,
            "serviceTier": "flex",
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .expect("serialize request body");

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from(request_body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let _response_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response body");
        let captured = captured_requests.lock().await;
        let captured_request = captured
            .first()
            .cloned()
            .expect("upstream should receive a request body");
        drop(captured);
        assert_eq!(captured_request["service_tier"], "priority");
        assert!(captured_request.get("serviceTier").is_none());

        let mut row: Option<PersistedRow> = None;
        for _ in 0..20 {
            row = sqlx::query_as::<_, PersistedRow>(
                r#"
                SELECT payload
                FROM codex_invocations
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .fetch_optional(&state.pool)
            .await
            .expect("query capture record");
            if row.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let row = row.expect("capture record should be persisted");
        let payload_json: Value = serde_json::from_str(
            row.payload
                .as_deref()
                .expect("capture payload should be present"),
        )
        .expect("decode capture payload");
        assert_eq!(payload_json["requestedServiceTier"], "priority");

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_capture_target_compact_estimates_cost_and_flows_into_stats_without_rewrite() {
        #[derive(sqlx::FromRow)]
        struct PersistedCompactRow {
            endpoint: Option<String>,
            model: Option<String>,
            requested_service_tier: Option<String>,
            input_tokens: Option<i64>,
            cache_input_tokens: Option<i64>,
            output_tokens: Option<i64>,
            reasoning_tokens: Option<i64>,
            total_tokens: Option<i64>,
            cost: Option<f64>,
            price_version: Option<String>,
        }

        let (upstream_base, captured_requests, upstream_handle) =
            spawn_capture_target_body_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        {
            let mut settings = state.proxy_model_settings.write().await;
            settings.fast_mode_rewrite_mode = ProxyFastModeRewriteMode::ForcePriority;
        }
        {
            let mut pricing = state.pricing_catalog.write().await;
            *pricing = PricingCatalog {
                version: "compact-unit-test".to_string(),
                models: HashMap::from([(
                    "gpt-5.1-codex-max".to_string(),
                    ModelPricing {
                        input_per_1m: 2.0,
                        output_per_1m: 3.0,
                        cache_input_per_1m: Some(0.5),
                        reasoning_per_1m: Some(7.0),
                        source: "custom".to_string(),
                    },
                )]),
            };
        }

        let request_body = serde_json::to_vec(&json!({
            "model": "gpt-5.1-codex-max",
            "serviceTier": "flex",
            "previous_response_id": "resp_prev_001",
            "input": [{
                "role": "user",
                "content": "compact this thread"
            }]
        }))
        .expect("serialize compact request body");

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses/compact".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from(request_body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let _response_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response body");
        let captured = captured_requests.lock().await;
        let captured_request = captured
            .first()
            .cloned()
            .expect("upstream should receive a compact request body");
        drop(captured);
        assert_eq!(captured_request["serviceTier"], "flex");
        assert!(captured_request.get("service_tier").is_none());

        let mut row: Option<PersistedCompactRow> = None;
        for _ in 0..20 {
            row = sqlx::query_as::<_, PersistedCompactRow>(
                r#"
                SELECT
                    CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
                    model,
                    CASE
                      WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                        THEN json_extract(payload, '$.requestedServiceTier')
                      WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                        THEN json_extract(payload, '$.requested_service_tier')
                    END AS requested_service_tier,
                    input_tokens,
                    cache_input_tokens,
                    output_tokens,
                    reasoning_tokens,
                    total_tokens,
                    cost,
                    price_version
                FROM codex_invocations
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .fetch_optional(&state.pool)
            .await
            .expect("query compact capture record");
            if row.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let row = row.expect("compact capture record should be persisted");
        assert_eq!(row.endpoint.as_deref(), Some("/v1/responses/compact"));
        assert_eq!(row.model.as_deref(), Some("gpt-5.1-codex-max"));
        assert_eq!(row.requested_service_tier.as_deref(), Some("flex"));
        assert_eq!(row.input_tokens, Some(139));
        assert_eq!(row.cache_input_tokens, Some(11));
        assert_eq!(row.output_tokens, Some(438));
        assert_eq!(row.reasoning_tokens, Some(64));
        assert_eq!(row.total_tokens, Some(577));
        assert_eq!(row.price_version.as_deref(), Some("compact-unit-test"));
        assert_f64_close(row.cost.expect("compact cost should be present"), 0.0020235);

        let Json(stats) = fetch_stats(State(state.clone()))
            .await
            .expect("compact fetch_stats should succeed");
        assert_eq!(stats.total_count, 1);
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.failure_count, 0);
        assert_eq!(stats.total_tokens, 577);
        assert_f64_close(stats.total_cost, 0.0020235);

        let Json(summary) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("1d".to_string()),
                limit: None,
                time_zone: None,
            }),
        )
        .await
        .expect("compact fetch_summary should succeed");
        assert_eq!(summary.total_count, 1);
        assert_eq!(summary.total_tokens, 577);
        assert_f64_close(summary.total_cost, 0.0020235);

        let Json(timeseries) = fetch_timeseries(
            State(state),
            Query(TimeseriesQuery {
                range: "1d".to_string(),
                bucket: Some("1h".to_string()),
                settlement_hour: None,
                time_zone: None,
            }),
        )
        .await
        .expect("compact fetch_timeseries should succeed");
        assert_eq!(
            timeseries
                .points
                .iter()
                .map(|point| point.total_count)
                .sum::<i64>(),
            1
        );
        assert_eq!(
            timeseries
                .points
                .iter()
                .map(|point| point.total_tokens)
                .sum::<i64>(),
            577
        );
        assert_f64_close(
            timeseries
                .points
                .iter()
                .map(|point| point.total_cost)
                .sum::<f64>(),
            0.0020235,
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_allows_slow_upload_with_short_timeout() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse(&upstream_base).expect("valid upstream base url");
        config.request_timeout = Duration::from_millis(100);
        let http_clients = HttpClients::build(&config).expect("http clients");
        let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
        let (broadcaster, _rx) = broadcast::channel(16);
        let state = Arc::new(AppState {
            config: config.clone(),
            pool,
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            startup_ready: Arc::new(AtomicBool::new(true)),
            semaphore,
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
        });

        let slow_chunks = stream::unfold(0u8, |state| async move {
            match state {
                0 => {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    Some((Ok::<_, Infallible>(Bytes::from_static(b"hello-")), 1))
                }
                1 => {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    Some((Ok::<_, Infallible>(Bytes::from_static(b"slow-")), 2))
                }
                2 => {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    Some((Ok::<_, Infallible>(Bytes::from_static(b"upload")), 3))
                }
                _ => None,
            }
        });

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/echo?mode=slow-upload".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from_stream(slow_chunks),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy response payload");
        assert_eq!(payload["query"], "mode=slow-upload");
        assert_eq!(payload["body"], "hello-slow-upload");

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_e2e_http_roundtrip() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let app = Router::new()
            .route("/v1/*path", any(proxy_openai_v1))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy test server");
        let addr = listener.local_addr().expect("proxy test server addr");
        let server_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("proxy test server should run");
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://{addr}/v1/echo?foo=e2e"))
            .header(http_header::AUTHORIZATION, "Bearer e2e-token")
            .body("hello-e2e")
            .send()
            .await
            .expect("send proxy request");

        assert_eq!(response.status(), StatusCode::CREATED);
        let payload: Value = response
            .json()
            .await
            .expect("decode proxied upstream payload");
        assert_eq!(payload["method"], "POST");
        assert_eq!(payload["path"], "/v1/echo");
        assert_eq!(payload["query"], "foo=e2e");
        assert_eq!(payload["authorization"], "Bearer e2e-token");
        assert_eq!(payload["body"], "hello-e2e");

        server_handle.abort();
        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_e2e_stream_survives_short_request_timeout() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse(&upstream_base).expect("valid upstream base url");
        config.request_timeout = Duration::from_millis(200);
        let http_clients = HttpClients::build(&config).expect("http clients");
        let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
        let (broadcaster, _rx) = broadcast::channel(16);
        let state = Arc::new(AppState {
            config: config.clone(),
            pool,
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            startup_ready: Arc::new(AtomicBool::new(true)),
            semaphore,
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
        });

        let app = Router::new()
            .route("/v1/*path", any(proxy_openai_v1))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy test server");
        let addr = listener.local_addr().expect("proxy test server addr");
        let server_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("proxy test server should run");
        });

        let client = reqwest::Client::new();
        let response = client
            .get(format!("http://{addr}/v1/slow-stream"))
            .send()
            .await
            .expect("send proxy stream request");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.bytes().await.expect("read proxied stream");
        assert_eq!(&body[..], b"chunk-achunk-b");

        server_handle.abort();
        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_rejects_oversized_request_body() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base_and_body_limit(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            4,
        )
        .await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/echo".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from("hello"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains("request body exceeds")
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_rejects_dot_segment_path() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/%2e%2e/admin".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED)
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_rejects_malformed_percent_encoded_path() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/%zz/models".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains(PROXY_INVALID_REQUEST_TARGET)
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_returns_bad_gateway_on_upstream_handshake_timeout() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse(&upstream_base).expect("valid upstream base url");
        config.openai_proxy_handshake_timeout = Duration::from_millis(100);
        let http_clients = HttpClients::build(&config).expect("http clients");
        let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
        let (broadcaster, _rx) = broadcast::channel(16);
        let state = Arc::new(AppState {
            config: config.clone(),
            pool,
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            startup_ready: Arc::new(AtomicBool::new(true)),
            semaphore,
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
        });

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/hang".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::new(),
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains(PROXY_UPSTREAM_HANDSHAKE_TIMEOUT)
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn proxy_openai_v1_returns_bad_gateway_on_upstream_handshake_timeout_with_body() {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse(&upstream_base).expect("valid upstream base url");
        config.openai_proxy_handshake_timeout = Duration::from_millis(100);
        let http_clients = HttpClients::build(&config).expect("http clients");
        let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
        let (broadcaster, _rx) = broadcast::channel(16);
        let state = Arc::new(AppState {
            config: config.clone(),
            pool,
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            startup_ready: Arc::new(AtomicBool::new(true)),
            semaphore,
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
        });

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/hang".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::new(),
            Body::from("hello"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
        assert!(
            payload["error"]
                .as_str()
                .expect("error message should be present")
                .contains(PROXY_UPSTREAM_HANDSHAKE_TIMEOUT)
        );

        upstream_handle.abort();
    }

    #[test]
    fn prepare_target_request_body_injects_include_usage_for_chat_stream() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-4o-mini",
            "stream": true,
            "messages": [{"role":"user","content":"hi"}]
        }))
        .expect("serialize request body");
        let (rewritten, info, did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::ChatCompletions,
            body,
            true,
            DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
        );
        assert!(did_rewrite);
        assert!(info.is_stream);
        assert_eq!(info.model.as_deref(), Some("gpt-4o-mini"));
        let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
        assert_eq!(
            payload
                .pointer("/stream_options/include_usage")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn prepare_target_request_body_extracts_prompt_cache_key_from_metadata() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": true,
            "metadata": {
                "prompt_cache_key": "pck-from-body"
            }
        }))
        .expect("serialize request body");

        let (_rewritten, info, _did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::Responses,
            body,
            true,
            DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
        );

        assert_eq!(info.prompt_cache_key.as_deref(), Some("pck-from-body"));
    }

    #[test]
    fn prepare_target_request_body_extracts_requested_service_tier_without_rewriting_when_disabled()
    {
        let expected = json!({
            "model": "gpt-5.3-codex",
            "serviceTier": " Priority ",
            "stream": false
        });
        let body = serde_json::to_vec(&expected).expect("serialize request body");

        let (rewritten, info, did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::Responses,
            body,
            true,
            DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
        );

        assert!(!did_rewrite);
        assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
        let payload: Value = serde_json::from_slice(&rewritten).expect("decode body");
        assert_eq!(payload, expected);
    }

    #[test]
    fn prepare_target_request_body_fill_missing_injects_priority_for_responses() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": false,
            "input": "hi"
        }))
        .expect("serialize request body");

        let (rewritten, info, did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::Responses,
            body,
            true,
            ProxyFastModeRewriteMode::FillMissing,
        );

        assert!(did_rewrite);
        assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
        let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
        assert_eq!(payload["service_tier"].as_str(), Some("priority"));
        assert!(payload.get("serviceTier").is_none());
    }

    #[test]
    fn prepare_target_request_body_force_priority_overrides_existing_alias() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": true,
            "messages": [{"role":"user","content":"hi"}],
            "serviceTier": "default"
        }))
        .expect("serialize request body");

        let (rewritten, info, did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::ChatCompletions,
            body,
            false,
            ProxyFastModeRewriteMode::ForcePriority,
        );

        assert!(did_rewrite);
        assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
        let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
        assert_eq!(payload["service_tier"].as_str(), Some("priority"));
        assert!(payload.get("serviceTier").is_none());
    }

    #[test]
    fn prepare_target_request_body_fill_missing_preserves_existing_tier() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": false,
            "service_tier": "flex"
        }))
        .expect("serialize request body");

        let (rewritten, info, did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::Responses,
            body,
            true,
            ProxyFastModeRewriteMode::FillMissing,
        );

        assert!(!did_rewrite);
        assert_eq!(info.requested_service_tier.as_deref(), Some("flex"));
        assert_eq!(
            rewritten,
            serde_json::to_vec(&json!({
                "model": "gpt-5.3-codex",
                "stream": false,
                "service_tier": "flex"
            }))
            .expect("serialize expected body")
        );
    }

    #[test]
    fn prepare_target_request_body_fill_missing_preserves_existing_alias_and_normalizes_field_name()
    {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": false,
            "serviceTier": "flex"
        }))
        .expect("serialize request body");

        let (rewritten, info, did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::Responses,
            body,
            true,
            ProxyFastModeRewriteMode::FillMissing,
        );

        assert!(did_rewrite);
        assert_eq!(info.requested_service_tier.as_deref(), Some("flex"));
        let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
        assert_eq!(payload["service_tier"].as_str(), Some("flex"));
        assert!(payload.get("serviceTier").is_none());
    }

    #[test]
    fn prepare_target_request_body_force_priority_overrides_existing_tier() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": false,
            "serviceTier": "flex",
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .expect("serialize request body");

        let (rewritten, info, did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::ChatCompletions,
            body,
            true,
            ProxyFastModeRewriteMode::ForcePriority,
        );

        assert!(did_rewrite);
        assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
        let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
        assert_eq!(payload["service_tier"], "priority");
        assert!(payload.get("serviceTier").is_none());
    }

    #[test]
    fn prepare_target_request_body_compact_skips_fast_mode_rewrite() {
        let expected = json!({
            "model": "gpt-5.1-codex-max",
            "serviceTier": "flex",
            "previous_response_id": "resp_prev_001",
            "input": [{
                "role": "user",
                "content": "compact this thread"
            }]
        });
        let body = serde_json::to_vec(&expected).expect("serialize request body");

        let (rewritten, info, did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::ResponsesCompact,
            body,
            true,
            ProxyFastModeRewriteMode::ForcePriority,
        );

        assert!(!did_rewrite);
        assert_eq!(info.model.as_deref(), Some("gpt-5.1-codex-max"));
        assert_eq!(info.requested_service_tier.as_deref(), Some("flex"));
        let payload: Value = serde_json::from_slice(&rewritten).expect("decode body");
        assert_eq!(payload, expected);
        assert!(payload.get("service_tier").is_none());
    }

    #[test]
    fn prepare_target_request_body_extracts_reasoning_effort_for_responses() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": true,
            "reasoning": {
                "effort": "high"
            }
        }))
        .expect("serialize request body");

        let (_rewritten, info, _did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::Responses,
            body,
            true,
            DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
        );

        assert_eq!(info.reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn prepare_target_request_body_extracts_reasoning_effort_for_chat_completions() {
        let body = serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}],
            "reasoning_effort": "medium"
        }))
        .expect("serialize request body");

        let (_rewritten, info, _did_rewrite) = prepare_target_request_body(
            ProxyCaptureTarget::ChatCompletions,
            body,
            true,
            DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
        );

        assert_eq!(info.reasoning_effort.as_deref(), Some("medium"));
    }

    #[test]
    fn extract_requested_service_tier_from_request_body_reads_top_level_aliases() {
        let snake_case = json!({ "service_tier": " Priority " });
        let camel_case = json!({ "serviceTier": "PRIORITY" });

        assert_eq!(
            extract_requested_service_tier_from_request_body(&snake_case).as_deref(),
            Some("priority")
        );
        assert_eq!(
            extract_requested_service_tier_from_request_body(&camel_case).as_deref(),
            Some("priority")
        );
    }

    #[test]
    fn extract_requested_service_tier_from_request_body_ignores_nested_or_non_string_values() {
        let nested = json!({
            "response": { "service_tier": "priority" },
            "metadata": { "serviceTier": "priority" }
        });
        let non_string = json!({ "service_tier": true });
        let blank = json!({ "serviceTier": "   " });

        assert_eq!(
            extract_requested_service_tier_from_request_body(&nested),
            None
        );
        assert_eq!(
            extract_requested_service_tier_from_request_body(&non_string),
            None
        );
        assert_eq!(
            extract_requested_service_tier_from_request_body(&blank),
            None
        );
    }

    #[test]
    fn extract_requester_ip_uses_expected_header_priority() {
        let mut preferred = HeaderMap::new();
        preferred.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("198.51.100.10, 203.0.113.9"),
        );
        preferred.insert(
            HeaderName::from_static("x-real-ip"),
            HeaderValue::from_static("203.0.113.5"),
        );
        preferred.insert(
            HeaderName::from_static("forwarded"),
            HeaderValue::from_static("for=192.0.2.60;proto=https"),
        );
        assert_eq!(
            extract_requester_ip(&preferred, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
            Some("198.51.100.10")
        );

        let mut fallback_forwarded = HeaderMap::new();
        fallback_forwarded.insert(
            HeaderName::from_static("forwarded"),
            HeaderValue::from_static("for=\"[2001:db8::1]:443\";proto=https"),
        );
        assert_eq!(
            extract_requester_ip(&fallback_forwarded, Some(IpAddr::from([127, 0, 0, 1])))
                .as_deref(),
            Some("2001:db8::1")
        );

        let no_headers = HeaderMap::new();
        assert_eq!(
            extract_requester_ip(&no_headers, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
            Some("127.0.0.1")
        );
    }

    #[test]
    fn extract_prompt_cache_key_from_headers_reads_whitelist_keys() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-prompt-cache-key"),
            HeaderValue::from_static("pck-from-header"),
        );
        assert_eq!(
            extract_prompt_cache_key_from_headers(&headers).as_deref(),
            Some("pck-from-header")
        );
    }

    #[test]
    fn parse_stream_response_payload_extracts_usage_and_model() {
        let raw = [
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}],\"usage\":null}",
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[],\"service_tier\":\"priority\",\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":18}}",
            "data: [DONE]",
        ]
        .join("\n");
        let parsed = parse_stream_response_payload(raw.as_bytes());
        assert_eq!(parsed.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(parsed.usage.input_tokens, Some(11));
        assert_eq!(parsed.usage.output_tokens, Some(7));
        assert_eq!(parsed.usage.total_tokens, Some(18));
        assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
        assert!(parsed.usage_missing_reason.is_none());
    }

    #[test]
    fn estimate_proxy_cost_subtracts_cached_tokens_from_base_input_rate() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-test".to_string(),
                ModelPricing {
                    input_per_1m: 1.0,
                    output_per_1m: 2.0,
                    cache_input_per_1m: Some(0.5),
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(1_000),
            output_tokens: Some(200),
            cache_input_tokens: Some(400),
            reasoning_tokens: None,
            total_tokens: Some(1_200),
        };

        let (cost, estimated, price_version) =
            estimate_proxy_cost(&catalog, Some("gpt-test"), &usage);

        let expected = ((600.0 * 1.0) + (200.0 * 2.0) + (400.0 * 0.5)) / 1_000_000.0;
        let computed = cost.expect("cost should be present");
        assert!((computed - expected).abs() < 1e-12);
        assert!(estimated);
        assert_eq!(price_version.as_deref(), Some("unit-test"));
    }

    #[test]
    fn estimate_proxy_cost_keeps_full_input_when_cache_price_missing() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-test".to_string(),
                ModelPricing {
                    input_per_1m: 1.0,
                    output_per_1m: 2.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(1_000),
            output_tokens: Some(200),
            cache_input_tokens: Some(400),
            reasoning_tokens: None,
            total_tokens: Some(1_200),
        };

        let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-test"), &usage);

        let expected = ((1_000.0 * 1.0) + (200.0 * 2.0)) / 1_000_000.0;
        let computed = cost.expect("cost should be present");
        assert!((computed - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn estimate_proxy_cost_falls_back_to_dated_model_base_pricing() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.2".to_string(),
                ModelPricing {
                    input_per_1m: 2.0,
                    output_per_1m: 3.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(1000),
            output_tokens: Some(500),
            cache_input_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(1500),
        };

        let (cost, estimated, _) =
            estimate_proxy_cost(&catalog, Some("gpt-5.2-2025-12-11"), &usage);

        let expected = ((1000.0 * 2.0) + (500.0 * 3.0)) / 1_000_000.0;
        assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn estimate_proxy_cost_prefers_exact_model_over_dated_model_base_pricing() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([
                (
                    "gpt-5.2".to_string(),
                    ModelPricing {
                        input_per_1m: 1.0,
                        output_per_1m: 1.0,
                        cache_input_per_1m: None,
                        reasoning_per_1m: None,
                        source: "custom".to_string(),
                    },
                ),
                (
                    "gpt-5.2-2025-12-11".to_string(),
                    ModelPricing {
                        input_per_1m: 4.0,
                        output_per_1m: 5.0,
                        cache_input_per_1m: None,
                        reasoning_per_1m: None,
                        source: "custom".to_string(),
                    },
                ),
            ]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(1000),
            output_tokens: Some(1000),
            cache_input_tokens: None,
            reasoning_tokens: None,
            total_tokens: Some(2000),
        };

        let (cost, estimated, _) =
            estimate_proxy_cost(&catalog, Some("gpt-5.2-2025-12-11"), &usage);

        let expected = ((1000.0 * 4.0) + (1000.0 * 5.0)) / 1_000_000.0;
        assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_at_threshold() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.4".to_string(),
                ModelPricing {
                    input_per_1m: 2.5,
                    output_per_1m: 15.0,
                    cache_input_per_1m: Some(0.25),
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS),
            output_tokens: Some(1_000),
            cache_input_tokens: Some(1_000),
            reasoning_tokens: None,
            total_tokens: None,
        };

        let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4"), &usage);

        let expected = ((271_000.0 * 2.5) + (1_000.0 * 0.25) + (1_000.0 * 15.0)) / 1_000_000.0;
        let computed = cost.expect("cost should be present");
        assert!((computed - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_above_threshold() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.4".to_string(),
                ModelPricing {
                    input_per_1m: 2.5,
                    output_per_1m: 15.0,
                    cache_input_per_1m: Some(0.25),
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
            output_tokens: Some(1_000),
            cache_input_tokens: Some(1_000),
            reasoning_tokens: None,
            total_tokens: None,
        };

        let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4"), &usage);

        let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
        let output_part = (1_000.0 * 15.0) / 1_000_000.0;
        let expected = (input_part * 2.0) + (output_part * 1.5);
        let computed = cost.expect("cost should be present");
        assert!((computed - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_to_reasoning_cost() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.4".to_string(),
                ModelPricing {
                    input_per_1m: 2.5,
                    output_per_1m: 15.0,
                    cache_input_per_1m: Some(0.25),
                    reasoning_per_1m: Some(20.0),
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
            output_tokens: Some(1_000),
            cache_input_tokens: Some(1_000),
            reasoning_tokens: Some(2_000),
            total_tokens: None,
        };

        let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4"), &usage);

        let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
        let output_part = (1_000.0 * 15.0) / 1_000_000.0;
        let reasoning_part = (2_000.0 * 20.0) / 1_000_000.0;
        let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
        let computed = cost.expect("cost should be present");
        assert!((computed - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_above_threshold() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.4-pro".to_string(),
                ModelPricing {
                    input_per_1m: 30.0,
                    output_per_1m: 180.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
            output_tokens: Some(1_000),
            cache_input_tokens: Some(999_999),
            reasoning_tokens: None,
            total_tokens: None,
        };

        let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4-pro"), &usage);

        let input_part = (272_001.0 * 30.0) / 1_000_000.0;
        let output_part = (1_000.0 * 180.0) / 1_000_000.0;
        let expected = (input_part * 2.0) + (output_part * 1.5);
        let computed = cost.expect("cost should be present");
        assert!((computed - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_for_dated_model_suffix() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.4-pro".to_string(),
                ModelPricing {
                    input_per_1m: 30.0,
                    output_per_1m: 180.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: Some(90.0),
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
            output_tokens: Some(1_000),
            cache_input_tokens: Some(999_999),
            reasoning_tokens: Some(2_000),
            total_tokens: None,
        };

        let (cost, estimated, _) =
            estimate_proxy_cost(&catalog, Some("gpt-5.4-pro-2026-03-01"), &usage);

        let input_part = (272_001.0 * 30.0) / 1_000_000.0;
        let output_part = (1_000.0 * 180.0) / 1_000_000.0;
        let reasoning_part = (2_000.0 * 90.0) / 1_000_000.0;
        let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
        let computed = cost.expect("cost should be present");
        assert!((computed - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_for_dated_model_suffix() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.4".to_string(),
                ModelPricing {
                    input_per_1m: 2.5,
                    output_per_1m: 15.0,
                    cache_input_per_1m: Some(0.25),
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
            output_tokens: Some(1_000),
            cache_input_tokens: Some(1_000),
            reasoning_tokens: None,
            total_tokens: None,
        };

        let (cost, estimated, _) =
            estimate_proxy_cost(&catalog, Some("gpt-5.4-2026-03-01"), &usage);

        let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
        let output_part = (1_000.0 * 15.0) / 1_000_000.0;
        let expected = (input_part * 2.0) + (output_part * 1.5);
        let computed = cost.expect("cost should be present");
        assert!((computed - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_for_other_models() {
        let catalog = PricingCatalog {
            version: "unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.4o".to_string(),
                ModelPricing {
                    input_per_1m: 2.5,
                    output_per_1m: 15.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };
        let usage = ParsedUsage {
            input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
            output_tokens: Some(1_000),
            cache_input_tokens: None,
            reasoning_tokens: None,
            total_tokens: None,
        };

        let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4o"), &usage);

        let expected = ((272_001.0 * 2.5) + (1_000.0 * 15.0)) / 1_000_000.0;
        let computed = cost.expect("cost should be present");
        assert!((computed - expected).abs() < 1e-12);
        assert!(estimated);
    }

    #[test]
    fn parse_target_response_payload_decodes_gzip_stream_usage() {
        let raw = [
            "event: response.created",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
            "",
            "event: response.completed",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
            "",
        ]
        .join("\n");

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(raw.as_bytes())
            .expect("write gzip payload");
        let compressed = encoder.finish().expect("finish gzip payload");

        let parsed = parse_target_response_payload(
            ProxyCaptureTarget::Responses,
            &compressed,
            true,
            Some("gzip"),
        );
        assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(parsed.usage.input_tokens, Some(12));
        assert_eq!(parsed.usage.output_tokens, Some(3));
        assert_eq!(parsed.usage.cache_input_tokens, Some(2));
        assert_eq!(parsed.usage.total_tokens, Some(15));
        assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
        assert!(parsed.usage_missing_reason.is_none());
    }

    #[test]
    fn parse_target_response_payload_decodes_multi_value_content_encoding() {
        let raw = [
            "event: response.created",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
            "",
            "event: response.completed",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"flex\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
            "",
        ]
        .join("\n");

        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(raw.as_bytes())
            .expect("write gzip payload");
        let compressed = encoder.finish().expect("finish gzip payload");

        let parsed = parse_target_response_payload(
            ProxyCaptureTarget::Responses,
            &compressed,
            true,
            Some("identity, gzip"),
        );
        assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(parsed.usage.input_tokens, Some(12));
        assert_eq!(parsed.usage.output_tokens, Some(3));
        assert_eq!(parsed.usage.cache_input_tokens, Some(2));
        assert_eq!(parsed.usage.total_tokens, Some(15));
        assert_eq!(parsed.service_tier.as_deref(), Some("flex"));
        assert!(parsed.usage_missing_reason.is_none());
    }

    #[test]
    fn parse_target_response_payload_detects_sse_without_request_stream_hint() {
        let raw = [
            "event: response.completed",
            r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","service_tier":"priority","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
            "",
        ]
        .join("\n");

        let parsed = parse_target_response_payload(
            ProxyCaptureTarget::Responses,
            raw.as_bytes(),
            false,
            None,
        );

        assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
        assert_eq!(parsed.usage.total_tokens, Some(15));
        assert!(parsed.usage_missing_reason.is_none());
    }

    #[test]
    fn parse_target_response_payload_reads_service_tier_from_response_object() {
        let raw = json!({
            "id": "resp_json_1",
            "response": {
                "model": "gpt-5.3-codex",
                "service_tier": "priority",
                "usage": {
                    "input_tokens": 21,
                    "output_tokens": 5,
                    "total_tokens": 26
                }
            }
        });

        let parsed = parse_target_response_payload(
            ProxyCaptureTarget::Responses,
            serde_json::to_string(&raw)
                .expect("serialize raw payload")
                .as_bytes(),
            false,
            None,
        );

        assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
        assert_eq!(parsed.usage.total_tokens, Some(26));
    }

    #[test]
    fn parse_target_response_payload_records_decode_failure_reason() {
        let raw = [
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2,\"total_tokens\":12}}}",
            "data: [DONE]",
        ]
        .join("\n");

        let parsed = parse_target_response_payload(
            ProxyCaptureTarget::Responses,
            raw.as_bytes(),
            true,
            Some("gzip"),
        );

        assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
        assert_eq!(parsed.usage.total_tokens, Some(12));
        assert!(
            parsed
                .usage_missing_reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("response_decode_failed:gzip:"))
        );
    }

    #[tokio::test]
    async fn proxy_capture_target_extracts_usage_from_gzip_response_stream() {
        #[derive(sqlx::FromRow)]
        struct PersistedUsageRow {
            source: String,
            status: Option<String>,
            input_tokens: Option<i64>,
            output_tokens: Option<i64>,
            cache_input_tokens: Option<i64>,
            total_tokens: Option<i64>,
            payload: Option<String>,
        }

        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-forwarded-for"),
            HeaderValue::from_static("198.51.100.42, 203.0.113.10"),
        );
        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses?mode=gzip".parse().expect("valid uri")),
            Method::POST,
            headers,
            Body::from(
                r#"{"model":"gpt-5.3-codex","stream":true,"metadata":{"prompt_cache_key":"pck-gzip-1"},"input":"hello"}"#,
            ),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");

        let mut row: Option<PersistedUsageRow> = None;
        for _ in 0..50 {
            row = sqlx::query_as::<_, PersistedUsageRow>(
                r#"
                SELECT source, status, input_tokens, output_tokens, cache_input_tokens, total_tokens, payload
                FROM codex_invocations
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .fetch_optional(&state.pool)
            .await
            .expect("query capture record");

            if row
                .as_ref()
                .is_some_and(|record| record.input_tokens.is_some())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let row = row.expect("capture record should exist");
        assert_eq!(row.source, SOURCE_PROXY);
        assert_eq!(row.status.as_deref(), Some("success"));
        assert_eq!(row.input_tokens, Some(12));
        assert_eq!(row.output_tokens, Some(3));
        assert_eq!(row.cache_input_tokens, Some(2));
        assert_eq!(row.total_tokens, Some(15));

        let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
            .expect("decode payload summary");
        assert_eq!(payload["endpoint"], "/v1/responses");
        assert!(payload["usageMissingReason"].is_null());
        assert_eq!(payload["requesterIp"], "198.51.100.42");
        assert_eq!(payload["promptCacheKey"], "pck-gzip-1");
        assert!(
            payload["proxyWeightDelta"].is_number(),
            "proxy weight delta should be recorded for fresh proxy attempts"
        );

        upstream_handle.abort();
    }

    #[tokio::test]
    async fn resolve_default_source_scope_always_all() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let scope_before = resolve_default_source_scope(&pool)
            .await
            .expect("scope before insert");
        assert_eq!(scope_before, InvocationSourceScope::All);

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, raw_response
            )
            VALUES (?1, ?2, ?3, ?4)
            "#,
        )
        .bind("proxy-test-1")
        .bind("2026-02-22 00:00:00")
        .bind(SOURCE_PROXY)
        .bind("{}")
        .execute(&pool)
        .await
        .expect("insert proxy invocation");

        let scope_after = resolve_default_source_scope(&pool)
            .await
            .expect("scope after insert");
        assert_eq!(scope_after, InvocationSourceScope::All);
    }

    #[tokio::test]
    async fn list_invocations_projects_payload_context_fields() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind("proxy-context-1")
        .bind("2026-02-25 10:00:00")
        .bind(SOURCE_PROXY)
        .bind("failed")
        .bind(
            r#"{"endpoint":"/v1/responses","failureKind":"upstream_stream_error","requesterIp":"198.51.100.77","promptCacheKey":"pck-list-1","requestedServiceTier":"priority","serviceTier":null,"service_tier":"priority","proxyDisplayName":"jp-relay-01","proxyWeightDelta":-0.68,"reasoningEffort":"high"}"#,
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert proxy invocation");

        let Json(response) = list_invocations(
            State(state),
            Query(ListQuery {
                limit: Some(10),
                model: None,
                status: None,
            }),
        )
        .await
        .expect("list invocations should succeed");

        let record = response
            .records
            .into_iter()
            .find(|item| item.invoke_id == "proxy-context-1")
            .expect("inserted invocation should be present");
        assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
        assert_eq!(
            record.failure_kind.as_deref(),
            Some("upstream_stream_error")
        );
        assert_eq!(record.requester_ip.as_deref(), Some("198.51.100.77"));
        assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-list-1"));
        assert_eq!(record.requested_service_tier.as_deref(), Some("priority"));
        assert_eq!(record.service_tier.as_deref(), Some("priority"));
        assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-01"));
        assert_eq!(record.proxy_weight_delta, Some(-0.68));
        assert_eq!(record.reasoning_effort.as_deref(), Some("high"));
    }

    #[tokio::test]
    async fn list_invocations_tolerates_malformed_payload_json() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind("proxy-context-malformed")
        .bind("2026-02-25 10:01:00")
        .bind(SOURCE_PROXY)
        .bind("failed")
        .bind("not-json")
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert malformed payload invocation");

        let Json(response) = list_invocations(
            State(state),
            Query(ListQuery {
                limit: Some(10),
                model: None,
                status: None,
            }),
        )
        .await
        .expect("list invocations should tolerate malformed payload");

        let record = response
            .records
            .into_iter()
            .find(|item| item.invoke_id == "proxy-context-malformed")
            .expect("inserted invocation should be present");
        assert_eq!(record.endpoint, None);
        assert_eq!(record.failure_kind, None);
        assert_eq!(record.requester_ip, None);
        assert_eq!(record.prompt_cache_key, None);
        assert_eq!(record.requested_service_tier, None);
        assert_eq!(record.service_tier, None);
        assert_eq!(record.proxy_weight_delta, None);
        assert_eq!(record.reasoning_effort, None);
    }

    #[tokio::test]
    async fn list_invocations_ignores_non_numeric_proxy_weight_delta() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind("proxy-context-delta-text")
        .bind("2026-02-25 10:02:00")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(
            "{\"endpoint\":\"/v1/responses\",\"proxyDisplayName\":\"jp-relay-02\",\"proxyWeightDelta\":\"abc\"}",
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert non-numeric proxyWeightDelta invocation");

        let Json(response) = list_invocations(
            State(state),
            Query(ListQuery {
                limit: Some(10),
                model: None,
                status: None,
            }),
        )
        .await
        .expect("list invocations should ignore non-numeric proxyWeightDelta");

        let record = response
            .records
            .into_iter()
            .find(|item| item.invoke_id == "proxy-context-delta-text")
            .expect("inserted invocation should be present");
        assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-02"));
        assert_eq!(record.proxy_weight_delta, None);
    }

    #[tokio::test]
    async fn list_invocations_preserves_historical_xy_records() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                model,
                total_tokens,
                cost,
                status,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind("xy-history-1")
        .bind("2026-02-25 10:03:00")
        .bind(SOURCE_XY)
        .bind("gpt-5.3-codex")
        .bind(16_i64)
        .bind(0.0042_f64)
        .bind("success")
        .bind(r#"{"serviceTier":"priority"}"#)
        .bind(r#"{"legacy":true}"#)
        .execute(&state.pool)
        .await
        .expect("insert historical xy invocation");

        let Json(response) = list_invocations(
            State(state.clone()),
            Query(ListQuery {
                limit: Some(10),
                model: None,
                status: None,
            }),
        )
        .await
        .expect("list invocations should keep historical xy rows");

        let record = response
            .records
            .into_iter()
            .find(|item| item.invoke_id == "xy-history-1")
            .expect("historical xy row should be returned");
        assert_eq!(record.source, SOURCE_XY);
        assert_eq!(record.service_tier.as_deref(), Some("priority"));
        assert_eq!(record.requested_service_tier, None);
    }

    #[tokio::test]
    async fn stats_endpoints_preserve_historical_xy_records() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                total_tokens,
                cost,
                status,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind("xy-history-stats-1")
        .bind(&occurred_at)
        .bind(SOURCE_XY)
        .bind(16_i64)
        .bind(0.0042_f64)
        .bind("success")
        .bind(r#"{"serviceTier":"priority"}"#)
        .bind(r#"{"legacy":true}"#)
        .execute(&state.pool)
        .await
        .expect("insert historical xy stats row");

        let Json(stats) = fetch_stats(State(state.clone()))
            .await
            .expect("fetch_stats should include historical xy rows");
        assert_eq!(stats.total_count, 1);
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.failure_count, 0);
        assert_eq!(stats.total_tokens, 16);
        assert_f64_close(stats.total_cost, 0.0042);

        let Json(summary) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("1d".to_string()),
                limit: None,
                time_zone: None,
            }),
        )
        .await
        .expect("fetch_summary should include historical xy rows");
        assert_eq!(summary.total_count, 1);
        assert_eq!(summary.success_count, 1);
        assert_eq!(summary.failure_count, 0);
        assert_eq!(summary.total_tokens, 16);
        assert_f64_close(summary.total_cost, 0.0042);

        let Json(timeseries) = fetch_timeseries(
            State(state),
            Query(TimeseriesQuery {
                range: "1d".to_string(),
                bucket: Some("1h".to_string()),
                settlement_hour: None,
                time_zone: None,
            }),
        )
        .await
        .expect("fetch_timeseries should include historical xy rows");
        assert_eq!(
            timeseries
                .points
                .iter()
                .map(|point| point.total_count)
                .sum::<i64>(),
            1
        );
        assert_eq!(
            timeseries
                .points
                .iter()
                .map(|point| point.total_tokens)
                .sum::<i64>(),
            16
        );
        assert_f64_close(
            timeseries
                .points
                .iter()
                .map(|point| point.total_cost)
                .sum::<f64>(),
            0.0042,
        );
    }

    #[test]
    fn normalize_prompt_cache_conversation_limit_accepts_whitelist_values_only() {
        assert_eq!(normalize_prompt_cache_conversation_limit(None), 50);
        assert_eq!(normalize_prompt_cache_conversation_limit(Some(20)), 20);
        assert_eq!(normalize_prompt_cache_conversation_limit(Some(50)), 50);
        assert_eq!(normalize_prompt_cache_conversation_limit(Some(100)), 100);
        assert_eq!(normalize_prompt_cache_conversation_limit(Some(10)), 50);
        assert_eq!(normalize_prompt_cache_conversation_limit(Some(200)), 50);
    }

    #[tokio::test]
    async fn prompt_cache_conversations_groups_recent_keys_and_uses_history_totals() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        let now = Utc::now();

        async fn insert_row(
            pool: &Pool<Sqlite>,
            invoke_id: &str,
            occurred_at: DateTime<Utc>,
            key: Option<&str>,
            status: &str,
            total_tokens: i64,
            cost: f64,
        ) {
            let payload = match key {
                Some(key) => json!({ "promptCacheKey": key }).to_string(),
                None => "{}".to_string(),
            };
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
            )
            .bind(invoke_id)
            .bind(format_naive(occurred_at.with_timezone(&Shanghai).naive_local()))
            .bind(SOURCE_PROXY)
            .bind(status)
            .bind(total_tokens)
            .bind(cost)
            .bind(payload)
            .bind("{}")
            .execute(pool)
            .await
            .expect("insert invocation row");
        }

        // key-a: active in 24h + older history
        insert_row(
            &state.pool,
            "pck-a-history",
            now - ChronoDuration::hours(48),
            Some("pck-a"),
            "success",
            100,
            1.0,
        )
        .await;
        insert_row(
            &state.pool,
            "pck-a-24h-1",
            now - ChronoDuration::hours(2),
            Some("pck-a"),
            "success",
            20,
            0.2,
        )
        .await;
        insert_row(
            &state.pool,
            "pck-a-24h-2",
            now - ChronoDuration::hours(1),
            Some("pck-a"),
            "failed",
            30,
            0.3,
        )
        .await;

        // key-b: newer created_at so it should rank before key-a.
        insert_row(
            &state.pool,
            "pck-b-24h-1",
            now - ChronoDuration::hours(10),
            Some("pck-b"),
            "success",
            10,
            0.1,
        )
        .await;

        // key-c: not active in last 24h; should be excluded.
        insert_row(
            &state.pool,
            "pck-c-history",
            now - ChronoDuration::hours(72),
            Some("pck-c"),
            "success",
            8,
            0.08,
        )
        .await;

        // missing key in last 24h; should be ignored.
        insert_row(
            &state.pool,
            "pck-missing-24h",
            now - ChronoDuration::minutes(40),
            None,
            "success",
            999,
            9.99,
        )
        .await;

        let Json(response) = fetch_prompt_cache_conversations(
            State(state.clone()),
            Query(PromptCacheConversationsQuery { limit: Some(20) }),
        )
        .await
        .expect("prompt cache conversation stats should succeed");

        assert_eq!(response.conversations.len(), 2);
        assert_eq!(response.conversations[0].prompt_cache_key, "pck-b");
        assert_eq!(response.conversations[1].prompt_cache_key, "pck-a");

        let key_a = response
            .conversations
            .iter()
            .find(|item| item.prompt_cache_key == "pck-a")
            .expect("pck-a should be included");
        assert_eq!(key_a.request_count, 3);
        assert_eq!(key_a.total_tokens, 150);
        assert!((key_a.total_cost - 1.5).abs() < 1e-9);
        assert_eq!(key_a.last24h_requests.len(), 2);
        assert_eq!(key_a.last24h_requests[0].request_tokens, 20);
        assert_eq!(key_a.last24h_requests[0].cumulative_tokens, 20);
        assert!(key_a.last24h_requests[0].is_success);
        assert_eq!(key_a.last24h_requests[1].request_tokens, 30);
        assert_eq!(key_a.last24h_requests[1].cumulative_tokens, 50);
        assert!(!key_a.last24h_requests[1].is_success);
    }

    #[tokio::test]
    async fn prompt_cache_conversations_cache_reuses_recent_result_within_ttl() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        let now = Utc::now();
        let occurred_a = format_naive(
            (now - ChronoDuration::minutes(80))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let occurred_b = format_naive(
            (now - ChronoDuration::minutes(30))
                .with_timezone(&Shanghai)
                .naive_local(),
        );

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind("pck-cache-1")
        .bind(&occurred_a)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(10)
        .bind(0.01)
        .bind(r#"{"promptCacheKey":"pck-cache"}"#)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert first cache row");

        let Json(first) = fetch_prompt_cache_conversations(
            State(state.clone()),
            Query(PromptCacheConversationsQuery { limit: Some(20) }),
        )
        .await
        .expect("first fetch should succeed");
        let first_count = first
            .conversations
            .iter()
            .find(|item| item.prompt_cache_key == "pck-cache")
            .map(|item| item.request_count)
            .expect("pck-cache should be present");
        assert_eq!(first_count, 1);

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind("pck-cache-2")
        .bind(&occurred_b)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(15)
        .bind(0.015)
        .bind(r#"{"promptCacheKey":"pck-cache"}"#)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert second cache row");

        let Json(second) = fetch_prompt_cache_conversations(
            State(state),
            Query(PromptCacheConversationsQuery { limit: Some(20) }),
        )
        .await
        .expect("second fetch should use cached result");
        let second_count = second
            .conversations
            .iter()
            .find(|item| item.prompt_cache_key == "pck-cache")
            .map(|item| item.request_count)
            .expect("pck-cache should still be present");
        assert_eq!(second_count, 1);
    }

    #[tokio::test]
    async fn prompt_cache_conversations_concurrent_requests_same_limit_do_not_stall() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        let now = Utc::now();
        let occurred = format_naive(
            (now - ChronoDuration::minutes(20))
                .with_timezone(&Shanghai)
                .naive_local(),
        );

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind("pck-concurrent-1")
        .bind(&occurred)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(18)
        .bind(0.018)
        .bind(r#"{"promptCacheKey":"pck-concurrent"}"#)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert concurrent cache row");

        let mut handles = Vec::new();
        for _ in 0..8 {
            let state_clone = state.clone();
            handles.push(tokio::spawn(async move {
                tokio::time::timeout(
                    Duration::from_secs(2),
                    fetch_prompt_cache_conversations(
                        State(state_clone),
                        Query(PromptCacheConversationsQuery { limit: Some(20) }),
                    ),
                )
                .await
            }));
        }

        for handle in handles {
            let response = handle
                .await
                .expect("join should succeed")
                .expect("concurrent request should not timeout")
                .expect("concurrent request should succeed");
            let Json(payload) = response;
            assert!(
                payload
                    .conversations
                    .iter()
                    .any(|item| item.prompt_cache_key == "pck-concurrent"),
                "expected pck-concurrent to be present in each response",
            );
        }
    }

    #[tokio::test]
    async fn prompt_cache_conversation_flight_guard_cleans_in_flight_on_drop() {
        let cache = Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
        let (signal, _receiver) = watch::channel(false);
        {
            let mut state = cache.lock().await;
            state
                .in_flight
                .insert(20, PromptCacheConversationInFlight { signal });
        }

        {
            let _guard = PromptCacheConversationFlightGuard::new(cache.clone(), 20);
        }

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let has_entry = {
                    let state = cache.lock().await;
                    state.in_flight.contains_key(&20)
                };
                if !has_entry {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("drop cleanup should remove in-flight marker");
    }

    #[test]
    fn decode_response_payload_for_usage_decompresses_gzip_stream() {
        let raw = [
            "event: response.completed",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":123,\"output_tokens\":45,\"total_tokens\":168,\"input_tokens_details\":{\"cached_tokens\":7},\"output_tokens_details\":{\"reasoning_tokens\":4}}}}",
        ]
        .join("\n");
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(raw.as_bytes())
            .expect("write gzip payload");
        let compressed = encoder.finish().expect("finish gzip payload");

        let (decoded, decode_error) = decode_response_payload_for_usage(&compressed, Some("gzip"));
        assert!(decode_error.is_none());

        let parsed = parse_target_response_payload(
            ProxyCaptureTarget::Responses,
            decoded.as_ref(),
            true,
            None,
        );
        assert_eq!(parsed.usage.input_tokens, Some(123));
        assert_eq!(parsed.usage.output_tokens, Some(45));
        assert_eq!(parsed.usage.total_tokens, Some(168));
        assert_eq!(parsed.usage.cache_input_tokens, Some(7));
        assert_eq!(parsed.usage.reasoning_tokens, Some(4));
    }

    #[tokio::test]
    async fn backfill_proxy_prompt_cache_keys_updates_payload_and_is_idempotent() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill");
        let request_path = temp_dir.join("request.json");
        write_backfill_request_payload(&request_path, Some("pck-backfill-1"));

        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-pck-backfill-1",
            &request_path,
            "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\",\"codexSessionId\":\"legacy-session-1\"}",
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-pck-backfill-ready",
            &request_path,
            "{\"endpoint\":\"/v1/responses\",\"promptCacheKey\":\"already-present\"}",
        )
        .await;

        let summary_first = backfill_proxy_prompt_cache_keys(&pool, None)
            .await
            .expect("first prompt cache key backfill should succeed");
        assert_eq!(summary_first.scanned, 1);
        assert_eq!(summary_first.updated, 1);
        assert_eq!(summary_first.skipped_missing_file, 0);
        assert_eq!(summary_first.skipped_invalid_json, 0);
        assert_eq!(summary_first.skipped_missing_key, 0);

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
                .bind("proxy-pck-backfill-1")
                .fetch_one(&pool)
                .await
                .expect("query backfilled payload");
        let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
        assert_eq!(payload_json["promptCacheKey"], "pck-backfill-1");
        assert!(
            payload_json.get("codexSessionId").is_none(),
            "legacy codexSessionId key should be removed during backfill"
        );

        let summary_second = backfill_proxy_prompt_cache_keys(&pool, None)
            .await
            .expect("second prompt cache key backfill should succeed");
        assert_eq!(summary_second.scanned, 0);
        assert_eq!(summary_second.updated, 0);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_proxy_prompt_cache_keys_tracks_skip_counters() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill-skips");
        let ok_request_path = temp_dir.join("request-ok.json");
        let missing_key_request_path = temp_dir.join("request-missing-key.json");
        let invalid_json_request_path = temp_dir.join("request-invalid-json.json");
        let missing_file_request_path = temp_dir.join("request-missing.json");

        write_backfill_request_payload(&ok_request_path, Some("pck-backfill-ok"));
        write_backfill_request_payload(&missing_key_request_path, None);
        fs::write(&invalid_json_request_path, b"not-json").expect("write invalid request payload");

        let base_payload = "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\"}";
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-pck-backfill-ok",
            &ok_request_path,
            base_payload,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-pck-backfill-missing-file",
            &missing_file_request_path,
            base_payload,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-pck-backfill-invalid-json",
            &invalid_json_request_path,
            base_payload,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-pck-backfill-missing-key",
            &missing_key_request_path,
            base_payload,
        )
        .await;

        let summary = backfill_proxy_prompt_cache_keys(&pool, None)
            .await
            .expect("prompt cache key backfill should succeed");
        assert_eq!(summary.scanned, 4);
        assert_eq!(summary.updated, 1);
        assert_eq!(summary.skipped_missing_file, 1);
        assert_eq!(summary.skipped_invalid_json, 1);
        assert_eq!(summary.skipped_missing_key, 1);

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
                .bind("proxy-pck-backfill-ok")
                .fetch_one(&pool)
                .await
                .expect("query backfilled payload");
        let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
        assert_eq!(payload_json["promptCacheKey"], "pck-backfill-ok");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_proxy_requested_service_tiers_updates_payload_and_is_idempotent() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("proxy-requested-service-tier-backfill");
        let request_path = temp_dir.join("request.json");
        write_backfill_request_payload_with_requested_service_tier(
            &request_path,
            Some("priority"),
            ProxyCaptureTarget::Responses,
        );

        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-requested-tier-backfill-1",
            &request_path,
            r#"{"endpoint":"/v1/responses"}"#,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-requested-tier-backfill-ready",
            &request_path,
            r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority"}"#,
        )
        .await;

        let summary_first = backfill_proxy_requested_service_tiers(&pool, None)
            .await
            .expect("first requested service tier backfill should succeed");
        assert_eq!(summary_first.scanned, 1);
        assert_eq!(summary_first.updated, 1);
        assert_eq!(summary_first.skipped_missing_file, 0);
        assert_eq!(summary_first.skipped_invalid_json, 0);
        assert_eq!(summary_first.skipped_missing_tier, 0);

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
                .bind("proxy-requested-tier-backfill-1")
                .fetch_one(&pool)
                .await
                .expect("query backfilled payload");
        let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
        assert_eq!(payload_json["requestedServiceTier"], "priority");

        let summary_second = backfill_proxy_requested_service_tiers(&pool, None)
            .await
            .expect("second requested service tier backfill should be idempotent");
        assert_eq!(summary_second.scanned, 0);
        assert_eq!(summary_second.updated, 0);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_proxy_requested_service_tiers_tracks_skip_counters() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("proxy-requested-service-tier-backfill-skips");
        let missing_tier_request_path = temp_dir.join("request-missing-tier.json");
        let invalid_json_request_path = temp_dir.join("request-invalid-json.json");
        let missing_file_request_path = temp_dir.join("request-missing.json");

        write_backfill_request_payload_with_requested_service_tier(
            &missing_tier_request_path,
            None,
            ProxyCaptureTarget::Responses,
        );
        fs::write(&invalid_json_request_path, b"not-json").expect("write invalid request payload");

        let base_payload = r#"{"endpoint":"/v1/responses"}"#;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-requested-tier-missing-file",
            &missing_file_request_path,
            base_payload,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-requested-tier-invalid-json",
            &invalid_json_request_path,
            base_payload,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-requested-tier-missing-tier",
            &missing_tier_request_path,
            base_payload,
        )
        .await;

        let summary = backfill_proxy_requested_service_tiers(&pool, None)
            .await
            .expect("requested service tier backfill should succeed");
        assert_eq!(summary.scanned, 3);
        assert_eq!(summary.updated, 0);
        assert_eq!(summary.skipped_missing_file, 1);
        assert_eq!(summary.skipped_invalid_json, 1);
        assert_eq!(summary.skipped_missing_tier, 1);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_proxy_reasoning_efforts_updates_payload_and_is_idempotent() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("proxy-reasoning-effort-backfill");
        let request_path = temp_dir.join("request.json");
        write_backfill_request_payload_with_reasoning(
            &request_path,
            Some("pck-reasoning"),
            Some("high"),
            ProxyCaptureTarget::Responses,
        );

        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-reasoning-backfill-1",
            &request_path,
            r#"{"endpoint":"/v1/responses","requesterIp":"198.51.100.77"}"#,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-reasoning-backfill-ready",
            &request_path,
            r#"{"endpoint":"/v1/responses","reasoningEffort":"medium"}"#,
        )
        .await;

        let summary_first = backfill_proxy_reasoning_efforts(&pool, None)
            .await
            .expect("first reasoning effort backfill should succeed");
        assert_eq!(summary_first.scanned, 1);
        assert_eq!(summary_first.updated, 1);
        assert_eq!(summary_first.skipped_missing_file, 0);
        assert_eq!(summary_first.skipped_invalid_json, 0);
        assert_eq!(summary_first.skipped_missing_effort, 0);

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
                .bind("proxy-reasoning-backfill-1")
                .fetch_one(&pool)
                .await
                .expect("query reasoning backfilled payload");
        let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
        assert_eq!(payload_json["reasoningEffort"], "high");

        let summary_second = backfill_proxy_reasoning_efforts(&pool, None)
            .await
            .expect("second reasoning effort backfill should succeed");
        assert_eq!(summary_second.scanned, 0);
        assert_eq!(summary_second.updated, 0);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_proxy_reasoning_efforts_tracks_skip_counters_and_chat_payloads() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("proxy-reasoning-effort-backfill-skips");
        let ok_chat_path = temp_dir.join("request-chat-ok.json");
        let missing_effort_path = temp_dir.join("request-missing-effort.json");
        let invalid_json_path = temp_dir.join("request-invalid-json.json");
        let missing_file_path = temp_dir.join("request-missing.json");

        write_backfill_request_payload_with_reasoning(
            &ok_chat_path,
            None,
            Some("medium"),
            ProxyCaptureTarget::ChatCompletions,
        );
        write_backfill_request_payload_with_reasoning(
            &missing_effort_path,
            None,
            None,
            ProxyCaptureTarget::Responses,
        );
        fs::write(&invalid_json_path, b"not-json").expect("write invalid request payload");

        let base_payload = r#"{"endpoint":"/v1/chat/completions"}"#;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-reasoning-chat-ok",
            &ok_chat_path,
            base_payload,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-reasoning-missing-file",
            &missing_file_path,
            base_payload,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-reasoning-invalid-json",
            &invalid_json_path,
            base_payload,
        )
        .await;
        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-reasoning-missing-effort",
            &missing_effort_path,
            r#"{"endpoint":"/v1/responses"}"#,
        )
        .await;

        let summary = backfill_proxy_reasoning_efforts(&pool, None)
            .await
            .expect("reasoning effort backfill should succeed");
        assert_eq!(summary.scanned, 4);
        assert_eq!(summary.updated, 1);
        assert_eq!(summary.skipped_missing_file, 1);
        assert_eq!(summary.skipped_invalid_json, 1);
        assert_eq!(summary.skipped_missing_effort, 1);

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
                .bind("proxy-reasoning-chat-ok")
                .fetch_one(&pool)
                .await
                .expect("query chat reasoning payload");
        let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
        assert_eq!(payload_json["reasoningEffort"], "medium");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_proxy_prompt_cache_keys_reads_from_fallback_root_for_relative_paths() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill-fallback");
        let fallback_root = temp_dir.join("legacy-root");
        let relative_path = PathBuf::from("proxy_raw_payloads/request-fallback.json");
        let request_path = fallback_root.join(&relative_path);
        let request_dir = request_path.parent().expect("request parent dir");
        fs::create_dir_all(request_dir).expect("create fallback request dir");
        write_backfill_request_payload(&request_path, Some("pck-fallback-1"));

        insert_proxy_prompt_cache_backfill_row(
            &pool,
            "proxy-pck-backfill-fallback",
            &relative_path,
            "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\"}",
        )
        .await;

        let summary = backfill_proxy_prompt_cache_keys(&pool, Some(&fallback_root))
            .await
            .expect("prompt cache key backfill with fallback root should succeed");
        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(summary.skipped_missing_file, 0);
        assert_eq!(summary.skipped_invalid_json, 0);
        assert_eq!(summary.skipped_missing_key, 0);

        let payload: String =
            sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
                .bind("proxy-pck-backfill-fallback")
                .fetch_one(&pool)
                .await
                .expect("query fallback-backfilled payload");
        let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
        assert_eq!(payload_json["promptCacheKey"], "pck-fallback-1");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_invocation_service_tiers_updates_payload_and_is_idempotent() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("invocation-service-tier-backfill");
        let response_path = temp_dir.join("response.bin");
        write_backfill_response_payload_with_service_tier(&response_path, Some("priority"));

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind("quota-service-tier-backfill")
        .bind("2026-02-23 00:00:00")
        .bind(SOURCE_XY)
        .bind("success")
        .bind("{}")
        .bind(r#"{"service_tier":"priority"}"#)
        .execute(&pool)
        .await
        .expect("insert quota service tier row");

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind("proxy-service-tier-backfill")
        .bind("2026-02-23 00:00:01")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(r#"{"endpoint":"/v1/responses"}"#)
        .bind("{}")
        .bind(response_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("insert proxy service tier row");

        let summary_first = backfill_invocation_service_tiers(&pool, None)
            .await
            .expect("first service tier backfill should succeed");
        assert_eq!(summary_first.scanned, 2);
        assert_eq!(summary_first.updated, 2);
        assert_eq!(summary_first.skipped_missing_file, 0);
        assert_eq!(summary_first.skipped_missing_tier, 0);

        let quota_payload: String =
            sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
                .bind("quota-service-tier-backfill")
                .fetch_one(&pool)
                .await
                .expect("query quota payload");
        let quota_payload_json: Value =
            serde_json::from_str(&quota_payload).expect("decode quota payload JSON");
        assert_eq!(quota_payload_json["serviceTier"], "priority");

        let proxy_payload: String =
            sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
                .bind("proxy-service-tier-backfill")
                .fetch_one(&pool)
                .await
                .expect("query proxy payload");
        let proxy_payload_json: Value =
            serde_json::from_str(&proxy_payload).expect("decode proxy payload JSON");
        assert_eq!(proxy_payload_json["serviceTier"], "priority");

        let summary_second = backfill_invocation_service_tiers(&pool, None)
            .await
            .expect("second service tier backfill should be idempotent");
        assert_eq!(summary_second.scanned, 0);
        assert_eq!(summary_second.updated, 0);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_invocation_service_tiers_tracks_skip_counters() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind("service-tier-missing")
        .bind("2026-02-23 00:00:00")
        .bind(SOURCE_XY)
        .bind("success")
        .bind("{}")
        .bind(r#"{"status":"success"}"#)
        .execute(&pool)
        .await
        .expect("insert missing tier row");

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind("service-tier-missing-file")
        .bind("2026-02-23 00:00:01")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(r#"{"endpoint":"/v1/responses"}"#)
        .bind("{}")
        .bind("/tmp/does-not-exist-response.bin")
        .execute(&pool)
        .await
        .expect("insert missing file row");

        let summary = backfill_invocation_service_tiers(&pool, None)
            .await
            .expect("service tier backfill skip run should succeed");
        assert_eq!(summary.scanned, 2);
        assert_eq!(summary.updated, 0);
        assert_eq!(summary.skipped_missing_file, 1);
        assert_eq!(summary.skipped_missing_tier, 1);
    }

    #[tokio::test]
    async fn backfill_proxy_usage_tokens_updates_missing_tokens_idempotently() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = std::env::temp_dir().join(format!(
            "proxy-usage-backfill-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&temp_dir).expect("create temp dir");
        let response_path = temp_dir.join("response.bin");
        let raw = [
            "event: response.completed",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":88,\"output_tokens\":22,\"total_tokens\":110,\"input_tokens_details\":{\"cached_tokens\":9},\"output_tokens_details\":{\"reasoning_tokens\":3}}}}",
        ]
        .join("\n");
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(raw.as_bytes())
            .expect("write gzip payload");
        let compressed = encoder.finish().expect("finish gzip payload");
        fs::write(&response_path, compressed).expect("write response payload");

        let row_count = BACKFILL_BATCH_SIZE as usize + 5;
        for index in 0..row_count {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                "#,
            )
            .bind(format!("proxy-backfill-test-{index}"))
            .bind("2026-02-23 00:00:00")
            .bind(SOURCE_PROXY)
            .bind("success")
            .bind(
                "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":true,\"requestModel\":null,\"responseModel\":null,\"usageMissingReason\":null,\"requestParseError\":null}",
            )
            .bind("{}")
            .bind(response_path.to_string_lossy().to_string())
            .execute(&pool)
            .await
            .expect("insert proxy row");
        }

        let summary_first = backfill_proxy_usage_tokens(&pool, None)
            .await
            .expect("first backfill should succeed");
        assert_eq!(summary_first.scanned, row_count as u64);
        assert_eq!(summary_first.updated, row_count as u64);

        let row = sqlx::query(
            r#"
            SELECT
              COUNT(*) AS total_rows,
              SUM(CASE WHEN input_tokens = 88 THEN 1 ELSE 0 END) AS input_tokens_88,
              SUM(CASE WHEN output_tokens = 22 THEN 1 ELSE 0 END) AS output_tokens_22,
              SUM(CASE WHEN cache_input_tokens = 9 THEN 1 ELSE 0 END) AS cache_input_tokens_9,
              SUM(CASE WHEN reasoning_tokens = 3 THEN 1 ELSE 0 END) AS reasoning_tokens_3,
              SUM(CASE WHEN total_tokens = 110 THEN 1 ELSE 0 END) AS total_tokens_110
            FROM codex_invocations
            WHERE source = ?1
            "#,
        )
        .bind(SOURCE_PROXY)
        .fetch_one(&pool)
        .await
        .expect("fetch backfilled rows");
        assert_eq!(
            row.try_get::<i64, _>("total_rows")
                .expect("read total_rows"),
            row_count as i64
        );
        assert_eq!(
            row.try_get::<Option<i64>, _>("input_tokens_88")
                .expect("read input_tokens_88"),
            Some(row_count as i64)
        );
        assert_eq!(
            row.try_get::<Option<i64>, _>("output_tokens_22")
                .expect("read output_tokens_22"),
            Some(row_count as i64)
        );
        assert_eq!(
            row.try_get::<Option<i64>, _>("cache_input_tokens_9")
                .expect("read cache_input_tokens_9"),
            Some(row_count as i64)
        );
        assert_eq!(
            row.try_get::<Option<i64>, _>("reasoning_tokens_3")
                .expect("read reasoning_tokens_3"),
            Some(row_count as i64)
        );
        assert_eq!(
            row.try_get::<Option<i64>, _>("total_tokens_110")
                .expect("read total_tokens_110"),
            Some(row_count as i64)
        );

        let summary_second = backfill_proxy_usage_tokens(&pool, None)
            .await
            .expect("second backfill should succeed");
        assert_eq!(summary_second.scanned, 0);
        assert_eq!(summary_second.updated, 0);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_proxy_usage_tokens_reads_from_fallback_root_for_relative_paths() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("proxy-usage-backfill-fallback");
        let fallback_root = temp_dir.join("legacy-root");
        let relative_path = PathBuf::from("proxy_raw_payloads/response-fallback.bin");
        let response_path = fallback_root.join(&relative_path);
        let response_dir = response_path.parent().expect("response parent dir");
        fs::create_dir_all(response_dir).expect("create fallback response dir");
        write_backfill_response_payload(&response_path);

        insert_proxy_backfill_row(&pool, "proxy-usage-backfill-fallback", &relative_path).await;
        let row_id: i64 =
            sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
                .bind("proxy-usage-backfill-fallback")
                .fetch_one(&pool)
                .await
                .expect("query fallback row id");

        let summary = backfill_proxy_usage_tokens_up_to_id(&pool, row_id, Some(&fallback_root))
            .await
            .expect("usage backfill with fallback root should succeed");
        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(summary.skipped_missing_file, 0);
        assert_eq!(summary.skipped_without_usage, 0);
        assert_eq!(summary.skipped_decode_error, 0);

        let total_tokens: Option<i64> =
            sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
                .bind("proxy-usage-backfill-fallback")
                .fetch_one(&pool)
                .await
                .expect("query fallback usage row");
        assert_eq!(total_tokens, Some(110));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_proxy_usage_tokens_respects_snapshot_upper_bound() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let temp_dir = make_temp_test_dir("proxy-usage-backfill-snapshot");
        let response_path = temp_dir.join("response.bin");
        write_backfill_response_payload(&response_path);

        let first_invoke_id = "proxy-backfill-snapshot-first";
        let second_invoke_id = "proxy-backfill-snapshot-second";
        insert_proxy_backfill_row(&pool, first_invoke_id, &response_path).await;
        insert_proxy_backfill_row(&pool, second_invoke_id, &response_path).await;

        let first_id: i64 =
            sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
                .bind(first_invoke_id)
                .fetch_one(&pool)
                .await
                .expect("query first id");
        let second_id: i64 =
            sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
                .bind(second_invoke_id)
                .fetch_one(&pool)
                .await
                .expect("query second id");

        let summary_first = backfill_proxy_usage_tokens_up_to_id(&pool, first_id, None)
            .await
            .expect("backfill up to first id should succeed");
        assert_eq!(summary_first.scanned, 1);
        assert_eq!(summary_first.updated, 1);

        let first_total_tokens: Option<i64> =
            sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
                .bind(first_invoke_id)
                .fetch_one(&pool)
                .await
                .expect("query first row tokens");
        let second_total_tokens: Option<i64> =
            sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
                .bind(second_invoke_id)
                .fetch_one(&pool)
                .await
                .expect("query second row tokens");
        assert_eq!(first_total_tokens, Some(110));
        assert_eq!(second_total_tokens, None);

        let summary_second = backfill_proxy_usage_tokens_up_to_id(&pool, second_id, None)
            .await
            .expect("backfill up to second id should succeed");
        assert_eq!(summary_second.scanned, 1);
        assert_eq!(summary_second.updated, 1);

        let second_total_tokens_after: Option<i64> =
            sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
                .bind(second_invoke_id)
                .fetch_one(&pool)
                .await
                .expect("query second row tokens after second backfill");
        assert_eq!(second_total_tokens_after, Some(110));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn backfill_proxy_missing_costs_updates_dated_model_alias_and_is_idempotent() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        insert_proxy_cost_backfill_row(
            &pool,
            "proxy-cost-backfill-dated-model",
            Some("gpt-5.2-2025-12-11"),
            Some(1_000),
            Some(500),
        )
        .await;

        let catalog = PricingCatalog {
            version: "unit-cost-backfill".to_string(),
            models: HashMap::from([(
                "gpt-5.2".to_string(),
                ModelPricing {
                    input_per_1m: 2.0,
                    output_per_1m: 3.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };

        let summary_first = backfill_proxy_missing_costs(&pool, &catalog)
            .await
            .expect("first cost backfill should succeed");
        assert_eq!(summary_first.scanned, 1);
        assert_eq!(summary_first.updated, 1);
        assert_eq!(summary_first.skipped_unpriced_model, 0);

        let row = sqlx::query(
            "SELECT cost, cost_estimated, price_version FROM codex_invocations WHERE invoke_id = ?1",
        )
        .bind("proxy-cost-backfill-dated-model")
        .fetch_one(&pool)
        .await
        .expect("query updated cost row");
        let expected = ((1_000.0 * 2.0) + (500.0 * 3.0)) / 1_000_000.0;
        assert!(
            (row.try_get::<Option<f64>, _>("cost")
                .expect("read cost")
                .expect("cost should exist")
                - expected)
                .abs()
                < 1e-12
        );
        assert_eq!(
            row.try_get::<Option<i64>, _>("cost_estimated")
                .expect("read cost_estimated"),
            Some(1)
        );
        assert_eq!(
            row.try_get::<Option<String>, _>("price_version")
                .expect("read price_version")
                .as_deref(),
            Some("unit-cost-backfill")
        );

        let summary_second = backfill_proxy_missing_costs(&pool, &catalog)
            .await
            .expect("second cost backfill should be idempotent");
        assert_eq!(summary_second.scanned, 0);
        assert_eq!(summary_second.updated, 0);
    }

    #[tokio::test]
    async fn backfill_proxy_missing_costs_skips_missing_model_or_usage_and_retries_unpriced_rows() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect in-memory sqlite");
        ensure_schema(&pool)
            .await
            .expect("schema should initialize");

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

        let catalog = PricingCatalog {
            version: "unit-cost-backfill".to_string(),
            models: HashMap::from([(
                "gpt-5.2".to_string(),
                ModelPricing {
                    input_per_1m: 2.0,
                    output_per_1m: 3.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            )]),
        };

        let summary = backfill_proxy_missing_costs(&pool, &catalog)
            .await
            .expect("cost backfill should succeed");
        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.updated, 1);
        assert_eq!(summary.skipped_unpriced_model, 1);
        let expected_attempt_version = pricing_backfill_attempt_version(&catalog);

        let unknown_row = sqlx::query(
            "SELECT cost, cost_estimated, price_version FROM codex_invocations WHERE invoke_id = ?1",
        )
        .bind("proxy-cost-backfill-unpriced-model")
        .fetch_one(&pool)
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
            Some(expected_attempt_version.as_str())
        );

        let summary_same_version = backfill_proxy_missing_costs(&pool, &catalog)
            .await
            .expect("same-version cost backfill should skip attempted unpriced rows");
        assert_eq!(summary_same_version.scanned, 0);
        assert_eq!(summary_same_version.updated, 0);

        let updated_catalog_same_version = PricingCatalog {
            version: catalog.version.clone(),
            models: HashMap::from([
                (
                    "gpt-5.2".to_string(),
                    ModelPricing {
                        input_per_1m: 2.0,
                        output_per_1m: 3.0,
                        cache_input_per_1m: None,
                        reasoning_per_1m: None,
                        source: "custom".to_string(),
                    },
                ),
                (
                    "unknown-model".to_string(),
                    ModelPricing {
                        input_per_1m: 4.0,
                        output_per_1m: 6.0,
                        cache_input_per_1m: None,
                        reasoning_per_1m: None,
                        source: "custom".to_string(),
                    },
                ),
            ]),
        };
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
    fn is_sqlite_lock_error_detects_structured_sqlite_codes() {
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
    async fn build_sqlite_connect_options_enforces_wal_and_busy_timeout_defaults() {
        let temp_dir = make_temp_test_dir("sqlite-connect-options");
        let db_path = temp_dir.join("options.db");
        let db_url = sqlite_url_for_path(&db_path);

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

    #[tokio::test]
    async fn run_backfill_with_retry_succeeds_after_lock_release() {
        let temp_dir = make_temp_test_dir("proxy-backfill-retry-success");
        let db_path = temp_dir.join("lock-success.db");
        let db_url = sqlite_url_for_path(&db_path);
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

        tokio::time::sleep(Duration::from_millis(400)).await;
        sqlx::query("COMMIT")
            .execute(&mut lock_conn)
            .await
            .expect("release sqlite write lock");

        let summary = backfill_task
            .await
            .expect("join backfill task")
            .expect("backfill should succeed after retry");
        assert!(
            started.elapsed() >= Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
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
    async fn run_backfill_with_retry_fails_when_lock_persists() {
        let temp_dir = make_temp_test_dir("proxy-backfill-retry-fail");
        let db_path = temp_dir.join("lock-fail.db");
        let db_url = sqlite_url_for_path(&db_path);
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
            started.elapsed() >= Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
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
    async fn run_backfill_with_retry_does_not_retry_non_lock_errors() {
        let temp_dir = make_temp_test_dir("proxy-backfill-retry-non-lock");
        let db_path = temp_dir.join("non-lock.db");
        let db_url = sqlite_url_for_path(&db_path);
        let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
            .expect("build sqlite options");
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(connect_options)
            .await
            .expect("connect sqlite pool");

        // Intentionally skip schema initialization to force a deterministic non-lock error.
        let started = Instant::now();
        let err = run_backfill_with_retry(&pool, None)
            .await
            .expect_err("backfill should fail immediately on non-lock errors");
        assert!(
            started.elapsed() < Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
            "non-lock errors should not wait for retry delay"
        );
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
    async fn run_cost_backfill_with_retry_succeeds_after_lock_release() {
        let temp_dir = make_temp_test_dir("proxy-cost-backfill-retry-success");
        let db_path = temp_dir.join("lock-success.db");
        let db_url = sqlite_url_for_path(&db_path);
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

        tokio::time::sleep(Duration::from_millis(400)).await;
        sqlx::query("COMMIT")
            .execute(&mut lock_conn)
            .await
            .expect("release sqlite write lock");

        let summary = backfill_task
            .await
            .expect("join cost backfill task")
            .expect("cost backfill should succeed after retry");
        assert!(
            started.elapsed() >= Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
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
    async fn run_cost_backfill_with_retry_does_not_retry_non_lock_errors() {
        let temp_dir = make_temp_test_dir("proxy-cost-backfill-retry-non-lock");
        let db_path = temp_dir.join("non-lock.db");
        let db_url = sqlite_url_for_path(&db_path);
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
        let started = Instant::now();
        let err = run_cost_backfill_with_retry(&pool, &catalog)
            .await
            .expect_err("cost backfill should fail immediately on non-lock errors");
        assert!(
            started.elapsed() < Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
            "non-lock errors should not wait for retry delay"
        );
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
    async fn quota_latest_returns_degraded_when_empty() {
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
            pool,
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            startup_ready: Arc::new(AtomicBool::new(true)),
            semaphore,
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
        });

        let Json(snapshot) = latest_quota_snapshot(State(state))
            .await
            .expect("route should succeed");

        assert!(!snapshot.is_active);
        assert_eq!(snapshot.total_requests, 0);
        assert_eq!(snapshot.total_cost, 0.0);
    }

    #[tokio::test]
    async fn quota_latest_returns_seeded_historical_snapshot() {
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

    async fn insert_timeseries_invocation(
        pool: &SqlitePool,
        invoke_id: &str,
        occurred_at: &str,
        status: &str,
        t_upstream_ttfb_ms: Option<f64>,
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
                t_upstream_ttfb_ms,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(10_i64)
        .bind(0.01_f64)
        .bind(t_upstream_ttfb_ms)
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert timeseries invocation");
    }

    fn assert_f64_close(actual: f64, expected: f64) {
        let diff = (actual - expected).abs();
        assert!(
            diff < 1e-6,
            "expected {expected}, got {actual}, diff={diff}"
        );
    }

    #[tokio::test]
    async fn timeseries_includes_first_byte_avg_and_p95_for_success_samples() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        let occurred_at = format_naive(
            (Utc::now() - ChronoDuration::minutes(5))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-sample-1",
            &occurred_at,
            "success",
            Some(100.0),
        )
        .await;
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-sample-2",
            &occurred_at,
            "success",
            Some(200.0),
        )
        .await;
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-sample-3",
            &occurred_at,
            "success",
            Some(400.0),
        )
        .await;
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-sample-failure",
            &occurred_at,
            "failed",
            Some(800.0),
        )
        .await;

        let Json(response) = fetch_timeseries(
            State(state),
            Query(TimeseriesQuery {
                range: "1h".to_string(),
                bucket: Some("15m".to_string()),
                settlement_hour: None,
                time_zone: Some("Asia/Shanghai".to_string()),
            }),
        )
        .await
        .expect("fetch timeseries");
        let bucket = response
            .points
            .iter()
            .find(|point| point.total_count >= 4)
            .expect("should include populated bucket");

        assert_eq!(bucket.first_byte_sample_count, 3);
        assert_f64_close(
            bucket.first_byte_avg_ms.expect("avg should be present"),
            (100.0 + 200.0 + 400.0) / 3.0,
        );
        assert_f64_close(
            bucket.first_byte_p95_ms.expect("p95 should be present"),
            380.0,
        );
    }

    #[tokio::test]
    async fn timeseries_ignores_non_positive_or_missing_ttfb_samples() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        let occurred_at = format_naive(
            (Utc::now() - ChronoDuration::minutes(10))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-ignore-null",
            &occurred_at,
            "success",
            None,
        )
        .await;
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-ignore-zero",
            &occurred_at,
            "success",
            Some(0.0),
        )
        .await;
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-ignore-negative",
            &occurred_at,
            "success",
            Some(-5.0),
        )
        .await;
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-ignore-failed",
            &occurred_at,
            "failed",
            Some(250.0),
        )
        .await;
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-keep-valid",
            &occurred_at,
            "success",
            Some(250.0),
        )
        .await;

        let Json(response) = fetch_timeseries(
            State(state),
            Query(TimeseriesQuery {
                range: "1h".to_string(),
                bucket: Some("15m".to_string()),
                settlement_hour: None,
                time_zone: Some("Asia/Shanghai".to_string()),
            }),
        )
        .await
        .expect("fetch timeseries");
        let bucket = response
            .points
            .iter()
            .find(|point| point.total_count >= 5)
            .expect("should include populated bucket");

        assert_eq!(bucket.first_byte_sample_count, 1);
        assert_f64_close(
            bucket.first_byte_avg_ms.expect("avg should be present"),
            250.0,
        );
        assert_f64_close(
            bucket.first_byte_p95_ms.expect("p95 should be present"),
            250.0,
        );
    }

    #[tokio::test]
    async fn timeseries_daily_bucket_includes_first_byte_stats() {
        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        // Use "now" to avoid crossing local-day boundaries around midnight.
        let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-daily-1",
            &occurred_at,
            "success",
            Some(50.0),
        )
        .await;
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-daily-2",
            &occurred_at,
            "success",
            Some(150.0),
        )
        .await;
        insert_timeseries_invocation(
            &state.pool,
            "ttfb-daily-failed",
            &occurred_at,
            "failed",
            Some(300.0),
        )
        .await;

        let Json(response) = fetch_timeseries(
            State(state),
            Query(TimeseriesQuery {
                range: "1d".to_string(),
                bucket: Some("1d".to_string()),
                settlement_hour: None,
                time_zone: Some("Asia/Shanghai".to_string()),
            }),
        )
        .await
        .expect("fetch timeseries");
        let bucket = response
            .points
            .iter()
            .find(|point| point.total_count >= 3)
            .expect("should include populated bucket");

        assert_eq!(bucket.first_byte_sample_count, 2);
        assert_f64_close(
            bucket.first_byte_avg_ms.expect("avg should be present"),
            100.0,
        );
        assert_f64_close(
            bucket.first_byte_p95_ms.expect("p95 should be present"),
            145.0,
        );
    }

    #[tokio::test]
    async fn ensure_schema_adds_retention_columns_and_tables() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'xy',
                payload TEXT,
                raw_response TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(invoke_id, occurred_at)
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create legacy invocation schema");

        ensure_schema(&pool).await.expect("ensure schema migration");

        let columns: HashSet<String> = sqlx::query("PRAGMA table_info('codex_invocations')")
            .fetch_all(&pool)
            .await
            .expect("inspect invocation columns")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
        assert!(columns.contains("detail_level"));
        assert!(columns.contains("detail_pruned_at"));
        assert!(columns.contains("detail_prune_reason"));

        let tables: HashSet<String> = sqlx::query_scalar(
            r#"
            SELECT name
            FROM sqlite_master
            WHERE type = 'table'
              AND name IN ('archive_batches', 'invocation_rollup_daily', 'startup_backfill_progress')
            "#,
        )
        .fetch_all(&pool)
        .await
        .expect("load retention tables")
        .into_iter()
        .collect();
        assert!(tables.contains("archive_batches"));
        assert!(tables.contains("invocation_rollup_daily"));
        assert!(tables.contains("startup_backfill_progress"));
    }

    #[tokio::test]
    async fn health_check_reports_starting_until_startup_is_ready() {
        let state = test_state_with_openai_base(
            Url::parse("http://127.0.0.1:18080").expect("valid upstream url"),
        )
        .await;

        state.startup_ready.store(false, Ordering::Release);
        let response = health_check(State(state.clone())).await.into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read health body");
        assert_eq!(std::str::from_utf8(&body).expect("utf8 body"), "starting");

        state.startup_ready.store(true, Ordering::Release);
        let response = health_check(State(state)).await.into_response();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read health body");
        assert_eq!(std::str::from_utf8(&body).expect("utf8 body"), "ok");
    }

    #[tokio::test]
    async fn startup_backfill_progress_persists_terminal_missing_raw_cursor() {
        let state = test_state_with_openai_base(
            Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
        )
        .await;

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind("reasoning-missing-raw")
        .bind("2026-03-09 00:00:00")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind("{}")
        .bind("{}")
        .bind("missing-reasoning-request.json")
        .execute(&state.pool)
        .await
        .expect("insert reasoning backfill row");

        let row_id: i64 =
            sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
                .bind("reasoning-missing-raw")
                .fetch_one(&state.pool)
                .await
                .expect("fetch inserted row id");

        run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
            .await
            .expect("first startup backfill pass should succeed");

        let task_name = startup_backfill_task_progress_key(
            state.as_ref(),
            StartupBackfillTask::ReasoningEffort,
        )
        .await;
        let progress = load_startup_backfill_progress(&state.pool, &task_name)
            .await
            .expect("load backfill progress after first pass");
        assert_eq!(progress.cursor_id, row_id);
        assert_eq!(progress.last_scanned, 1);
        assert_eq!(progress.last_updated, 0);
        assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);

        sqlx::query(
            "UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2",
        )
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("force startup backfill task due again");

        run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
            .await
            .expect("second startup backfill pass should skip previously scanned row");

        let progress = load_startup_backfill_progress(&state.pool, &task_name)
            .await
            .expect("load backfill progress after second pass");
        assert_eq!(progress.cursor_id, row_id);
        assert_eq!(progress.last_scanned, 0);
        assert_eq!(progress.last_updated, 0);
    }

    #[tokio::test]
    async fn failure_classification_backfill_skips_success_rows_with_complete_defaults() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                failure_class,
                is_actionable,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind("success-no-kind")
        .bind("2026-03-09 00:00:00")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(FAILURE_CLASS_NONE)
        .bind(0_i64)
        .bind("{}")
        .execute(&pool)
        .await
        .expect("insert success row");

        let outcome = backfill_failure_classification_from_cursor(&pool, 0, Some(10), None)
            .await
            .expect("run failure classification backfill");
        assert_eq!(outcome.summary.scanned, 0);
        assert_eq!(outcome.summary.updated, 0);
        assert_eq!(outcome.next_cursor_id, 0);
        assert!(!outcome.hit_budget);
    }

    #[tokio::test]
    async fn failure_classification_backfill_from_cursor_respects_scan_limit() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("in-memory sqlite");
        ensure_schema(&pool).await.expect("ensure schema");

        for idx in 0..205 {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id,
                    occurred_at,
                    source,
                    status,
                    error_message,
                    raw_response
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
            )
            .bind(format!("failure-classification-{idx}"))
            .bind("2026-03-09 00:00:00")
            .bind(SOURCE_PROXY)
            .bind("http_500")
            .bind("boom")
            .bind("{}")
            .execute(&pool)
            .await
            .expect("insert failure classification row");
        }

        let first = backfill_failure_classification_from_cursor(&pool, 0, Some(200), None)
            .await
            .expect("first bounded failure classification pass");
        assert_eq!(first.summary.scanned, 200);
        assert_eq!(first.summary.updated, 200);
        assert!(first.hit_budget);
        assert!(first.next_cursor_id > 0);

        let second = backfill_failure_classification_from_cursor(
            &pool,
            first.next_cursor_id,
            Some(200),
            None,
        )
        .await
        .expect("second bounded failure classification pass");
        assert_eq!(second.summary.scanned, 5);
        assert_eq!(second.summary.updated, 5);
        assert!(!second.hit_budget);
    }

    #[tokio::test]
    async fn retention_prunes_old_success_invocation_details_and_sweeps_orphans() {
        let (pool, config, temp_dir) = retention_test_pool_and_config("retention-prune").await;
        let response_raw = config.proxy_raw_dir.join("old-success-response.bin");
        fs::write(&response_raw, b"response-body").expect("write response raw");
        let request_missing = config.proxy_raw_dir.join("old-success-request.bin");
        let orphan = config.proxy_raw_dir.join("orphan.bin");
        fs::write(&orphan, b"orphan").expect("write orphan raw");
        set_file_mtime_seconds_ago(&orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
        let occurred_at = shanghai_local_days_ago(31, 12, 0, 0);

        insert_retention_invocation(
            &pool,
            "old-success",
            &occurred_at,
            SOURCE_XY,
            "success",
            Some("{\"endpoint\":\"/v1/responses\"}"),
            "{\"ok\":true}",
            Some(&request_missing),
            Some(&response_raw),
            Some(321),
            Some(1.23),
        )
        .await;

        let before_pruned_at = Utc::now() - ChronoDuration::seconds(5);
        let summary = run_data_retention_maintenance(&pool, &config, Some(false))
            .await
            .expect("run retention prune");
        let after_pruned_at = Utc::now() + ChronoDuration::seconds(5);
        assert_eq!(summary.invocation_details_pruned, 1);
        assert_eq!(summary.archive_batches_touched, 1);
        assert_eq!(summary.raw_files_removed, 1);
        assert_eq!(summary.orphan_raw_files_removed, 1);
        assert!(!response_raw.exists());
        assert!(!orphan.exists());

        let row = sqlx::query(
            r#"
            SELECT
                payload,
                raw_response,
                request_raw_path,
                response_raw_path,
                detail_level,
                detail_pruned_at,
                detail_prune_reason,
                total_tokens,
                cost,
                status
            FROM codex_invocations
            WHERE invoke_id = ?1
            "#,
        )
        .bind("old-success")
        .fetch_one(&pool)
        .await
        .expect("load pruned invocation");
        assert_eq!(
            row.get::<String, _>("detail_level"),
            DETAIL_LEVEL_STRUCTURED_ONLY
        );
        assert!(row.get::<Option<String>, _>("detail_pruned_at").is_some());
        assert_eq!(
            row.get::<Option<String>, _>("detail_prune_reason")
                .as_deref(),
            Some(DETAIL_PRUNE_REASON_SUCCESS_OVER_30D)
        );
        assert!(row.get::<Option<String>, _>("payload").is_none());
        assert_eq!(row.get::<String, _>("raw_response"), "");
        assert!(row.get::<Option<String>, _>("request_raw_path").is_none());
        assert!(row.get::<Option<String>, _>("response_raw_path").is_none());
        assert_eq!(row.get::<Option<i64>, _>("total_tokens"), Some(321));
        assert_f64_close(row.get::<Option<f64>, _>("cost").unwrap_or_default(), 1.23);
        assert_eq!(
            row.get::<Option<String>, _>("status").as_deref(),
            Some("success")
        );

        let detail_pruned_at = row
            .get::<Option<String>, _>("detail_pruned_at")
            .expect("detail_pruned_at should be populated");
        let detail_pruned_at = local_naive_to_utc(
            parse_shanghai_local_naive(&detail_pruned_at)
                .expect("detail_pruned_at should be shanghai-local"),
            Shanghai,
        )
        .with_timezone(&Utc);
        assert!(detail_pruned_at >= before_pruned_at);
        assert!(detail_pruned_at <= after_pruned_at);

        let batch = sqlx::query(
            r#"
            SELECT file_path, row_count, status
            FROM archive_batches
            WHERE dataset = 'codex_invocations'
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load prune archive batch");
        let file_path = PathBuf::from(batch.get::<String, _>("file_path"));
        assert!(file_path.exists());
        assert_eq!(batch.get::<String, _>("status"), ARCHIVE_STATUS_COMPLETED);
        assert_eq!(batch.get::<i64, _>("row_count"), 1);

        let archive_db_path = temp_dir.join("retention-prune-archive.sqlite");
        inflate_gzip_sqlite_file(&file_path, &archive_db_path).expect("inflate prune archive");
        let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
            .await
            .expect("open prune archive sqlite");
        let archived = sqlx::query(
            r#"
            SELECT payload, raw_response, detail_level, detail_pruned_at, detail_prune_reason
            FROM codex_invocations
            WHERE invoke_id = ?1
            "#,
        )
        .bind("old-success")
        .fetch_one(&archive_pool)
        .await
        .expect("load archived pre-prune invocation");
        assert_eq!(
            archived.get::<Option<String>, _>("payload").as_deref(),
            Some("{\"endpoint\":\"/v1/responses\"}")
        );
        assert_eq!(archived.get::<String, _>("raw_response"), "{\"ok\":true}");
        assert_eq!(archived.get::<String, _>("detail_level"), DETAIL_LEVEL_FULL);
        assert!(
            archived
                .get::<Option<String>, _>("detail_pruned_at")
                .is_none()
        );
        assert!(
            archived
                .get::<Option<String>, _>("detail_prune_reason")
                .is_none()
        );
        archive_pool.close().await;

        cleanup_temp_test_dir(&temp_dir);
    }

    #[tokio::test]
    async fn retention_archives_old_invocations_without_changing_summary_all() {
        let (pool, config, temp_dir) = retention_test_pool_and_config("retention-archive").await;
        let old_response = config.proxy_raw_dir.join("old-archive-response.bin");
        fs::write(&old_response, b"archive-response").expect("write archive raw");
        let old_occurred_at = shanghai_local_days_ago(91, 10, 0, 0);
        let old_failed_at = shanghai_local_days_ago(92, 11, 0, 0);
        let recent_at = shanghai_local_days_ago(5, 15, 0, 0);

        insert_retention_invocation(
            &pool,
            "archive-old-success",
            &old_occurred_at,
            SOURCE_XY,
            "success",
            Some("{\"endpoint\":\"/v1/responses\"}"),
            "{\"ok\":true}",
            None,
            Some(&old_response),
            Some(100),
            Some(0.5),
        )
        .await;
        insert_retention_invocation(
            &pool,
            "archive-old-failed",
            &old_failed_at,
            SOURCE_PROXY,
            "failed",
            Some("{\"endpoint\":\"/v1/chat/completions\"}"),
            "{\"error\":true}",
            None,
            None,
            Some(50),
            Some(0.25),
        )
        .await;
        insert_retention_invocation(
            &pool,
            "archive-recent",
            &recent_at,
            SOURCE_PROXY,
            "success",
            Some("{\"endpoint\":\"/v1/responses\"}"),
            "{\"ok\":true}",
            None,
            None,
            Some(70),
            Some(0.75),
        )
        .await;

        let before =
            query_combined_totals(&pool, None, StatsFilter::All, InvocationSourceScope::All)
                .await
                .expect("query totals before retention");
        let summary = run_data_retention_maintenance(&pool, &config, Some(false))
            .await
            .expect("run retention archive");
        let after =
            query_combined_totals(&pool, None, StatsFilter::All, InvocationSourceScope::All)
                .await
                .expect("query totals after retention");

        assert_eq!(summary.invocation_rows_archived, 2);
        assert_eq!(summary.archive_batches_touched, 1);
        assert_eq!(before.total_count, after.total_count);
        assert_eq!(before.success_count, after.success_count);
        assert_eq!(before.failure_count, after.failure_count);
        assert_eq!(before.total_tokens, after.total_tokens);
        assert_f64_close(before.total_cost, after.total_cost);
        assert!(!old_response.exists());

        let live_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("count live invocations");
        assert_eq!(live_count, 1);

        let rollup = sqlx::query(
            r#"
            SELECT total_count, success_count, failure_count, total_tokens, total_cost
            FROM invocation_rollup_daily
            WHERE stats_date = ?1 AND source = ?2
            "#,
        )
        .bind(&old_occurred_at[..10])
        .bind(SOURCE_XY)
        .fetch_one(&pool)
        .await
        .expect("load invocation rollup row");
        assert_eq!(rollup.get::<i64, _>("total_count"), 1);
        assert_eq!(rollup.get::<i64, _>("success_count"), 1);
        assert_eq!(rollup.get::<i64, _>("failure_count"), 0);
        assert_eq!(rollup.get::<i64, _>("total_tokens"), 100);
        assert_f64_close(rollup.get::<f64, _>("total_cost"), 0.5);

        let batch = sqlx::query(
            r#"
            SELECT file_path, row_count, status
            FROM archive_batches
            WHERE dataset = 'codex_invocations'
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load invocation archive batch");
        let file_path = PathBuf::from(batch.get::<String, _>("file_path"));
        assert!(file_path.exists());
        assert!(batch.get::<i64, _>("row_count") >= 2);
        assert_eq!(batch.get::<String, _>("status"), ARCHIVE_STATUS_COMPLETED);

        cleanup_temp_test_dir(&temp_dir);
    }

    #[tokio::test]
    async fn retention_archives_forward_proxy_attempts_and_stats_snapshots() {
        let (pool, config, temp_dir) =
            retention_test_pool_and_config("retention-timestamped").await;
        let old_attempt = Utc::now() - ChronoDuration::days(35);
        let recent_attempt = Utc::now() - ChronoDuration::days(1);
        seed_forward_proxy_attempt_at(&pool, "proxy-old", old_attempt, true).await;
        seed_forward_proxy_attempt_at(&pool, "proxy-new", recent_attempt, true).await;

        let old_captured_at = utc_naive_from_shanghai_local_days_ago(35, 8, 0, 0);
        let recent_captured_at = utc_naive_from_shanghai_local_days_ago(1, 8, 0, 0);
        insert_stats_source_snapshot_row(&pool, &old_captured_at, &old_captured_at[..10]).await;
        insert_stats_source_snapshot_row(&pool, &recent_captured_at, &recent_captured_at[..10])
            .await;

        let summary = run_data_retention_maintenance(&pool, &config, Some(false))
            .await
            .expect("run timestamped retention");
        assert_eq!(summary.forward_proxy_attempt_rows_archived, 1);
        assert_eq!(summary.stats_source_snapshot_rows_archived, 1);

        let remaining_old_attempts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE occurred_at < ?1",
        )
        .bind(shanghai_utc_cutoff_string(
            config.forward_proxy_attempts_retention_days,
        ))
        .fetch_one(&pool)
        .await
        .expect("count old forward proxy attempts");
        assert_eq!(remaining_old_attempts, 0);

        let remaining_old_snapshots: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM stats_source_snapshots WHERE captured_at < ?1",
        )
        .bind(shanghai_utc_cutoff_string(
            config.stats_source_snapshots_retention_days,
        ))
        .fetch_one(&pool)
        .await
        .expect("count old stats snapshots");
        assert_eq!(remaining_old_snapshots, 0);

        let datasets: HashSet<String> = sqlx::query_scalar(
            r#"
            SELECT dataset
            FROM archive_batches
            WHERE dataset IN ('forward_proxy_attempts', 'stats_source_snapshots')
            "#,
        )
        .fetch_all(&pool)
        .await
        .expect("load timestamped archive batch datasets")
        .into_iter()
        .collect();
        assert!(datasets.contains("forward_proxy_attempts"));
        assert!(datasets.contains("stats_source_snapshots"));

        cleanup_temp_test_dir(&temp_dir);
    }

    #[tokio::test]
    async fn retention_compacts_old_quota_snapshots_by_shanghai_day() {
        let (pool, config, temp_dir) = retention_test_pool_and_config("retention-quota").await;
        let same_day_early = utc_naive_from_shanghai_local_days_ago(40, 8, 0, 0);
        let same_day_late = utc_naive_from_shanghai_local_days_ago(40, 23, 0, 0);
        let next_day = utc_naive_from_shanghai_local_days_ago(39, 9, 0, 0);
        seed_quota_snapshot(&pool, &same_day_early).await;
        seed_quota_snapshot(&pool, &same_day_late).await;
        seed_quota_snapshot(&pool, &next_day).await;

        let summary = run_data_retention_maintenance(&pool, &config, Some(false))
            .await
            .expect("run quota compaction");
        assert_eq!(summary.quota_snapshot_rows_archived, 1);

        let remaining: Vec<String> = sqlx::query_scalar(
            "SELECT captured_at FROM codex_quota_snapshots ORDER BY captured_at ASC",
        )
        .fetch_all(&pool)
        .await
        .expect("load remaining quota snapshots");
        assert_eq!(remaining, vec![same_day_late.clone(), next_day.clone()]);

        let quota_batch_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'codex_quota_snapshots'",
        )
        .fetch_one(&pool)
        .await
        .expect("count quota archive batches");
        assert_eq!(quota_batch_count, 1);

        cleanup_temp_test_dir(&temp_dir);
    }

    #[tokio::test]
    async fn retention_orphan_sweep_skips_fresh_raw_files() {
        let (pool, config, temp_dir) =
            retention_test_pool_and_config("retention-orphan-grace").await;
        let orphan = config.proxy_raw_dir.join("fresh-orphan.bin");
        fs::write(&orphan, b"fresh-orphan").expect("write fresh orphan");

        let summary = run_data_retention_maintenance(&pool, &config, Some(false))
            .await
            .expect("run retention with fresh orphan");
        assert_eq!(summary.orphan_raw_files_removed, 0);
        assert!(orphan.exists());

        cleanup_temp_test_dir(&temp_dir);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn retention_orphan_sweep_anchors_relative_raw_dir_to_database_parent() {
        let _guard = APP_CONFIG_ENV_LOCK.lock().expect("cwd lock");
        let temp_dir = make_temp_test_dir("retention-orphan-db-parent");
        let db_root = temp_dir.join("db-root");
        let cwd_root = temp_dir.join("cwd-root");
        fs::create_dir_all(&db_root).expect("create db root");
        fs::create_dir_all(&cwd_root).expect("create cwd root");
        let _cwd_guard = CurrentDirGuard::change_to(&cwd_root);

        let db_path = db_root.join("codex-vibe-monitor.db");
        fs::File::create(&db_path).expect("create sqlite file");
        let pool = SqlitePool::connect(&sqlite_url_for_path(&db_path))
            .await
            .expect("connect retention sqlite");
        ensure_schema(&pool).await.expect("ensure retention schema");

        let mut config = test_config();
        config.database_path = db_path;
        config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

        let anchored_dir = config.resolved_proxy_raw_dir();
        fs::create_dir_all(&anchored_dir).expect("create anchored raw dir");
        let anchored_orphan = anchored_dir.join("anchored-orphan.bin");
        fs::write(&anchored_orphan, b"anchored-orphan").expect("write anchored orphan");
        set_file_mtime_seconds_ago(&anchored_orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);

        let cwd_raw_dir = cwd_root.join("proxy_raw_payloads");
        fs::create_dir_all(&cwd_raw_dir).expect("create cwd raw dir");
        let cwd_orphan = cwd_raw_dir.join("cwd-orphan.bin");
        fs::write(&cwd_orphan, b"cwd-orphan").expect("write cwd orphan");
        set_file_mtime_seconds_ago(&cwd_orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);

        let removed = sweep_orphan_proxy_raw_files(&pool, &config, None, false)
            .await
            .expect("run orphan sweep");

        assert_eq!(removed, 1);
        assert!(
            !anchored_orphan.exists(),
            "orphan sweep should clean the database-anchored raw dir"
        );
        assert!(
            cwd_orphan.exists(),
            "orphan sweep should stop scanning cwd-relative stray files"
        );

        pool.close().await;
        cleanup_temp_test_dir(&temp_dir);
    }

    #[tokio::test]
    async fn retention_dry_run_does_not_mutate_database_or_files() {
        let (pool, config, temp_dir) = retention_test_pool_and_config("retention-dry-run").await;
        let response_raw = config.proxy_raw_dir.join("dry-run-response.bin");
        let orphan = config.proxy_raw_dir.join("dry-run-orphan.bin");
        fs::write(&response_raw, b"dry-run-response").expect("write dry-run response raw");
        fs::write(&orphan, b"dry-run-orphan").expect("write dry-run orphan");
        set_file_mtime_seconds_ago(&orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
        let occurred_at = shanghai_local_days_ago(91, 7, 0, 0);
        insert_retention_invocation(
            &pool,
            "dry-run-old",
            &occurred_at,
            SOURCE_XY,
            "success",
            Some("{\"endpoint\":\"/v1/responses\"}"),
            "{\"ok\":true}",
            None,
            Some(&response_raw),
            Some(111),
            Some(0.9),
        )
        .await;

        let summary = run_data_retention_maintenance(&pool, &config, Some(true))
            .await
            .expect("run dry-run retention");
        assert!(summary.dry_run);
        assert_eq!(summary.invocation_rows_archived, 1);
        assert_eq!(summary.archive_batches_touched, 1);
        assert_eq!(summary.raw_files_removed, 1);
        assert_eq!(summary.orphan_raw_files_removed, 1);
        assert!(response_raw.exists());
        assert!(orphan.exists());

        let row = sqlx::query(
            "SELECT detail_level, payload, raw_response FROM codex_invocations WHERE invoke_id = ?1",
        )
        .bind("dry-run-old")
        .fetch_one(&pool)
        .await
        .expect("load dry-run invocation");
        assert_eq!(row.get::<String, _>("detail_level"), DETAIL_LEVEL_FULL);
        assert!(row.get::<Option<String>, _>("payload").is_some());
        assert_eq!(row.get::<String, _>("raw_response"), "{\"ok\":true}");

        let archive_batch_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
            .fetch_one(&pool)
            .await
            .expect("count dry-run archive batches");
        assert_eq!(archive_batch_count, 0);

        let archive_files = fs::read_dir(&config.archive_dir)
            .expect("read archive dir")
            .count();
        assert_eq!(archive_files, 0);

        cleanup_temp_test_dir(&temp_dir);
    }
}

fn default_range() -> String {
    "1d".to_string()
}

fn format_naive(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn parse_reporting_tz(time_zone: Option<&str>) -> Result<Tz> {
    let tz_name = time_zone.unwrap_or("Asia/Shanghai");
    tz_name
        .parse::<Tz>()
        .with_context(|| format!("invalid timeZone: {tz_name}"))
}

// `codex_invocations.occurred_at` is stored as a naive Asia/Shanghai timestamp string
// (e.g. "2026-01-21 01:02:15"). For lexicographic filtering to work correctly,
// we must bind the lower bound using the same representation.
fn db_occurred_at_lower_bound(start_utc: DateTime<Utc>) -> String {
    let shanghai = start_utc.with_timezone(&Shanghai);
    format_naive(shanghai.naive_local())
}

fn parse_duration_spec(spec: &str) -> Result<ChronoDuration> {
    if let Some(value) = spec.strip_suffix("mo") {
        let months: i64 = value.parse()?;
        return Ok(ChronoDuration::days(30 * months));
    }
    if let Some(value) = spec.strip_suffix('d') {
        let days: i64 = value.parse()?;
        return Ok(ChronoDuration::days(days));
    }
    if let Some(value) = spec.strip_suffix('h') {
        let hours: i64 = value.parse()?;
        return Ok(ChronoDuration::hours(hours));
    }
    if let Some(value) = spec.strip_suffix('m') {
        let minutes: i64 = value.parse()?;
        return Ok(ChronoDuration::minutes(minutes));
    }

    Err(anyhow::anyhow!(
        "unsupported duration specification: {spec}"
    ))
}

struct RangeWindow {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    display_end: DateTime<Utc>,
    duration: ChronoDuration,
}

fn resolve_range_window(spec: &str, tz: Tz) -> Result<RangeWindow> {
    let now = Utc::now();
    if let Some((start, raw_end)) = named_range_bounds(spec, now, tz) {
        // Clamp to "now" so charts do not render future empty buckets.
        let end = now.min(raw_end);
        let duration = end.signed_duration_since(start).max(ChronoDuration::zero());
        return Ok(RangeWindow {
            start,
            end,
            display_end: end,
            duration,
        });
    }

    let duration = parse_duration_spec(spec)?;
    let end = now;
    let start = end - duration;
    Ok(RangeWindow {
        start,
        end,
        display_end: end,
        duration,
    })
}

fn named_range_bounds(
    spec: &str,
    now: DateTime<Utc>,
    tz: Tz,
) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    match spec {
        "today" => {
            let local_date = now.with_timezone(&tz).date_naive();
            let start = start_of_local_day(now, tz);
            let next_date = local_date
                .succ_opt()
                .unwrap_or(local_date + ChronoDuration::days(1));
            let end = local_midnight_utc(next_date, tz);
            Some((start, end))
        }
        "thisWeek" => {
            let start = start_of_local_week(now, tz);
            // Week end must be computed via the next local boundary, not a fixed +7*24h.
            // This keeps correctness across DST transitions.
            let start_local_date = start.with_timezone(&tz).date_naive();
            let end = local_midnight_utc(start_local_date + ChronoDuration::days(7), tz);
            Some((start, end))
        }
        "thisMonth" => {
            let start = start_of_local_month(now, tz);
            Some((start, start_of_next_month(start, tz)))
        }
        _ => None,
    }
}

fn named_range_start(spec: &str, now: DateTime<Utc>, tz: Tz) -> Option<DateTime<Utc>> {
    named_range_bounds(spec, now, tz).map(|(start, _)| start)
}

fn start_of_local_day(now: DateTime<Utc>, tz: Tz) -> DateTime<Utc> {
    let local = now.with_timezone(&tz);
    let date = local.date_naive();
    let naive = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    local_naive_to_utc(naive, tz)
}

fn local_midnight_utc(date: NaiveDate, tz: Tz) -> DateTime<Utc> {
    let naive = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    local_naive_to_utc(naive, tz)
}

fn start_of_local_week(now: DateTime<Utc>, tz: Tz) -> DateTime<Utc> {
    let local = now.with_timezone(&tz);
    let date = local.date_naive();
    let start_of_day = date
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    let offset_days = local.weekday().num_days_from_monday() as i64;
    local_naive_to_utc(start_of_day - ChronoDuration::days(offset_days), tz)
}

fn start_of_local_month(now: DateTime<Utc>, tz: Tz) -> DateTime<Utc> {
    let local = now.with_timezone(&tz);
    let date = local.date_naive();
    let first_day = date.with_day(1).unwrap_or(date);
    let naive = first_day
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    local_naive_to_utc(naive, tz)
}

fn start_of_next_month(start: DateTime<Utc>, tz: Tz) -> DateTime<Utc> {
    let local = start.with_timezone(&tz);
    let naive = local.naive_local();
    let mut year = naive.year();
    let mut month = naive.month();
    month += 1;
    if month > 12 {
        month = 1;
        year += 1;
    }
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let naive = first
        .and_hms_opt(0, 0, 0)
        .expect("midnight should be representable");
    local_naive_to_utc(naive, tz)
}

fn local_naive_to_utc(naive: NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(dt, _) => dt.with_timezone(&Utc),
        LocalResult::None => {
            // Handle nonexistent local times (e.g. DST spring-forward gaps) by
            // selecting the first valid local instant *after* the requested time.
            // This avoids silently interpreting a local timestamp as UTC.
            for step_minutes in 1..=(24 * 60) {
                let probe = naive + ChronoDuration::minutes(step_minutes);
                match tz.from_local_datetime(&probe) {
                    LocalResult::Single(dt) => return dt.with_timezone(&Utc),
                    LocalResult::Ambiguous(dt, _) => return dt.with_timezone(&Utc),
                    LocalResult::None => continue,
                }
            }
            // Extremely unlikely: no valid local instant found in the next 24h.
            naive.and_utc()
        }
    }
}

fn bucket_seconds_from_spec(spec: &str) -> Option<i64> {
    match spec {
        "1m" => Some(60),
        "5m" => Some(300),
        "15m" => Some(900),
        "30m" => Some(1800),
        "1h" => Some(3600),
        "6h" => Some(21_600),
        "12h" => Some(43_200),
        "1d" => Some(86_400),
        _ => None,
    }
}

fn default_bucket_seconds(range: ChronoDuration) -> i64 {
    let seconds = range.num_seconds();
    if seconds <= 3_600 {
        60
    } else if seconds <= 172_800 {
        1_800
    } else if seconds <= 2_592_000 {
        3_600
    } else {
        86_400
    }
}

fn align_bucket_epoch(epoch: i64, bucket_seconds: i64, offset_seconds: i64) -> i64 {
    ((epoch + offset_seconds) / bucket_seconds) * bucket_seconds - offset_seconds
}

fn parse_summary_window(query: &SummaryQuery, default_limit: i64) -> Result<SummaryWindow> {
    match query.window.as_deref() {
        Some("current") => {
            let limit = query.limit.unwrap_or(default_limit).clamp(1, default_limit);
            Ok(SummaryWindow::Current(limit))
        }
        Some("all") => Ok(SummaryWindow::All),
        Some(raw @ ("today" | "thisWeek" | "thisMonth")) => {
            Ok(SummaryWindow::Calendar(raw.to_string()))
        }
        Some(raw) => Ok(SummaryWindow::Duration(parse_duration_spec(raw)?)),
        None => Ok(SummaryWindow::Duration(ChronoDuration::days(1))),
    }
}

async fn query_stats_row(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsRow> {
    match (filter, source_scope) {
        (StatsFilter::All, InvocationSourceScope::ProxyOnly) => sqlx::query_as::<_, StatsRow>(
            r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                WHERE source = ?1
                "#,
        )
        .bind(SOURCE_PROXY)
        .fetch_one(pool)
        .await
        .map_err(Into::into),
        (StatsFilter::All, InvocationSourceScope::All) => sqlx::query_as::<_, StatsRow>(
            r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                "#,
        )
        .fetch_one(pool)
        .await
        .map_err(Into::into),
        (StatsFilter::Since(start), InvocationSourceScope::ProxyOnly) => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                WHERE source = ?1 AND occurred_at >= ?2
                "#,
            )
            .bind(SOURCE_PROXY)
            .bind(db_occurred_at_lower_bound(start))
            .fetch_one(pool)
            .await
            .map_err(Into::into)
        }
        (StatsFilter::Since(start), InvocationSourceScope::All) => sqlx::query_as::<_, StatsRow>(
            r#"
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM codex_invocations
                WHERE occurred_at >= ?1
                "#,
        )
        .bind(db_occurred_at_lower_bound(start))
        .fetch_one(pool)
        .await
        .map_err(Into::into),
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::ProxyOnly) => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                WITH recent AS (
                    SELECT *
                    FROM codex_invocations
                    WHERE source = ?1
                    ORDER BY occurred_at DESC
                    LIMIT ?2
                )
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM recent
                "#,
            )
            .bind(SOURCE_PROXY)
            .bind(limit)
            .fetch_one(pool)
            .await
            .map_err(Into::into)
        }
        (StatsFilter::RecentLimit(limit), InvocationSourceScope::All) => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                WITH recent AS (
                    SELECT *
                    FROM codex_invocations
                    ORDER BY occurred_at DESC
                    LIMIT ?1
                )
                SELECT
                    COUNT(*) AS total_count,
                    SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS success_count,
                    SUM(CASE WHEN status != 'success' THEN 1 ELSE 0 END) AS failure_count,
                    COALESCE(SUM(cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM recent
                "#,
            )
            .bind(limit)
            .fetch_one(pool)
            .await
            .map_err(Into::into)
        }
    }
}

async fn query_invocation_totals(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    let live = StatsTotals::from(query_stats_row(pool, filter.clone(), source_scope).await?);
    if !matches!(filter, StatsFilter::All) {
        return Ok(live);
    }

    let rollup = query_invocation_rollup_totals(pool, source_scope).await?;
    Ok(live.add(rollup))
}

async fn query_invocation_rollup_totals(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    let row = match source_scope {
        InvocationSourceScope::ProxyOnly => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                SELECT
                    COALESCE(SUM(total_count), 0) AS total_count,
                    COALESCE(SUM(success_count), 0) AS success_count,
                    COALESCE(SUM(failure_count), 0) AS failure_count,
                    COALESCE(SUM(total_cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM invocation_rollup_daily
                WHERE source = ?1
                "#,
            )
            .bind(SOURCE_PROXY)
            .fetch_one(pool)
            .await?
        }
        InvocationSourceScope::All => {
            sqlx::query_as::<_, StatsRow>(
                r#"
                SELECT
                    COALESCE(SUM(total_count), 0) AS total_count,
                    COALESCE(SUM(success_count), 0) AS success_count,
                    COALESCE(SUM(failure_count), 0) AS failure_count,
                    COALESCE(SUM(total_cost), 0.0) AS total_cost,
                    COALESCE(SUM(total_tokens), 0) AS total_tokens
                FROM invocation_rollup_daily
                "#,
            )
            .fetch_one(pool)
            .await?
        }
    };
    Ok(StatsTotals::from(row))
}

async fn query_crs_totals(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    filter: &StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    if source_scope == InvocationSourceScope::ProxyOnly {
        return Ok(StatsTotals::default());
    }
    let relay = match relay {
        Some(relay) => relay,
        None => return Ok(StatsTotals::default()),
    };
    let mut query = String::from(
        r#"
        SELECT
            COALESCE(SUM(total_count), 0) AS total_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_cost), 0.0) AS total_cost,
            COALESCE(SUM(total_tokens), 0) AS total_tokens
        FROM stats_source_deltas
        WHERE source = ?1 AND period = ?2
        "#,
    );

    let mut binds: Vec<i64> = Vec::new();
    if let StatsFilter::Since(start) = filter {
        query.push_str(" AND captured_at_epoch >= ?3");
        binds.push(start.timestamp());
    } else if matches!(filter, StatsFilter::RecentLimit(_)) {
        return Ok(StatsTotals::default());
    }

    let mut sql = sqlx::query_as::<_, StatsRow>(&query)
        .bind(SOURCE_CRS)
        .bind(&relay.period);

    if let Some(epoch) = binds.first() {
        sql = sql.bind(epoch);
    }

    let row = sql.fetch_one(pool).await?;
    Ok(StatsTotals::from(row))
}

async fn query_combined_totals(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    let base = query_invocation_totals(pool, filter.clone(), source_scope).await?;
    let relay_totals = query_crs_totals(pool, relay, &filter, source_scope).await?;
    Ok(base.add(relay_totals))
}

async fn resolve_default_source_scope(_pool: &Pool<Sqlite>) -> Result<InvocationSourceScope> {
    Ok(InvocationSourceScope::All)
}

async fn query_crs_deltas(
    pool: &Pool<Sqlite>,
    relay: &CrsStatsConfig,
    start_epoch: i64,
    end_epoch: i64,
) -> Result<Vec<StatsDeltaRecord>> {
    sqlx::query_as::<_, StatsDeltaRecord>(
        r#"
        SELECT
            captured_at_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost
        FROM stats_source_deltas
        WHERE source = ?1
          AND period = ?2
          AND captured_at_epoch >= ?3
          AND captured_at_epoch <= ?4
        ORDER BY captured_at_epoch ASC
        "#,
    )
    .bind(SOURCE_CRS)
    .bind(&relay.period)
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrsStatsResponse {
    success: bool,
    #[serde(default)]
    data: Vec<CrsModelStats>,
    #[serde(default)]
    period: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrsModelStats {
    model: String,
    requests: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_create_tokens: i64,
    cache_read_tokens: i64,
    all_tokens: i64,
    costs: CrsCosts,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrsCosts {
    input: f64,
    output: f64,
    cache_write: f64,
    cache_read: f64,
    total: f64,
}

#[derive(Debug, Default, Clone, Copy)]
struct CrsTotals {
    total_count: i64,
    total_tokens: i64,
    total_cost: f64,
    input_tokens: i64,
    output_tokens: i64,
    cache_create_tokens: i64,
    cache_read_tokens: i64,
    cost_input: f64,
    cost_output: f64,
    cost_cache_write: f64,
    cost_cache_read: f64,
}

#[derive(Debug)]
struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::INTERNAL_SERVER_ERROR;
        let message = format!("{}", self.0);
        (status, message).into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

// --- ISO8601 UTC helpers and serializers ---
fn format_utc_iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_to_utc_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        if let Some(loc) = Shanghai.from_local_datetime(&naive).single() {
            return Some(loc.with_timezone(&Utc));
        }
        return Some(Utc.from_utc_datetime(&naive));
    }
    None
}

#[allow(clippy::ptr_arg)]
fn serialize_local_naive_to_utc_iso<S>(value: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let iso = parse_to_utc_datetime(value)
        .map(format_utc_iso)
        .unwrap_or_else(|| value.clone());
    serializer.serialize_str(&iso)
}

#[allow(clippy::ptr_arg)]
fn serialize_local_or_utc_to_utc_iso<S>(value: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serialize_local_naive_to_utc_iso(value, serializer)
}

#[allow(clippy::ptr_arg)]
fn serialize_opt_local_or_utc_to_utc_iso<S>(
    value: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(s) => serialize_local_naive_to_utc_iso(s, serializer),
        None => serializer.serialize_none(),
    }
}
