use std::{
    borrow::Cow,
    collections::hash_map::DefaultHasher,
    collections::{BTreeMap, HashMap, HashSet},
    convert::Infallible,
    env,
    future::Future,
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
    body::{Body, Bytes, HttpBody},
    extract::{ConnectInfo, OriginalUri, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, uri::Authority},
    response::{IntoResponse, Json, Response, Sse},
    routing::{any, delete, get, post, put},
};
use base64::Engine;
use brotli::Decompressor as BrotliDecompressor;
use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, LocalResult, NaiveDate, NaiveDateTime,
    SecondsFormat, TimeZone, Utc,
};
use chrono_tz::{Asia::Shanghai, Tz};
use clap::Parser;
use dotenvy::dotenv;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::{Compression, write::GzEncoder};
use futures_util::{FutureExt, StreamExt, future::Shared, stream};
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
    io::{AsyncReadExt, AsyncWriteExt},
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

mod api;
mod forward_proxy;
mod oauth_bridge;
mod stats;
#[cfg(test)]
mod tests;
mod upstream_accounts;

use api::*;
use forward_proxy::*;
use stats::*;
use upstream_accounts::*;

#[cfg_attr(not(test), allow(dead_code))]
const SOURCE_XY: &str = "xy";
const SOURCE_CRS: &str = "crs";
const SOURCE_PROXY: &str = "proxy";
const DEFAULT_OPENAI_UPSTREAM_BASE_URL: &str = "https://api.openai.com/";
const DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES: usize = 256 * 1024 * 1024;
const DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS: u64 = 60;
const DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS: u64 = 180;
const DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS: u64 = 180;
const DEFAULT_SQLITE_BUSY_TIMEOUT_SECS: u64 = 30;
const BACKFILL_BATCH_SIZE: i64 = 200;
const BACKFILL_ACCOUNT_BIND_BATCH_SIZE: usize = 400;
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
const STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_LIVE: &str = "upstream_activity_live_backfill_v1";
const STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES: &str =
    "upstream_activity_archive_backfill_v1";
const STARTUP_BACKFILL_TASK_PROXY_USAGE: &str = "proxy_usage_tokens_v1";
const STARTUP_BACKFILL_TASK_PROXY_COST: &str = "proxy_cost_v1";
const STARTUP_BACKFILL_TASK_PROMPT_CACHE_KEY: &str = "proxy_prompt_cache_key_v1";
const STARTUP_BACKFILL_TASK_REQUESTED_SERVICE_TIER: &str = "proxy_requested_service_tier_v1";
const STARTUP_BACKFILL_TASK_INVOCATION_SERVICE_TIER: &str = "invocation_service_tier_v1";
const STARTUP_BACKFILL_TASK_REASONING_EFFORT: &str = "proxy_reasoning_effort_v1";
const STARTUP_BACKFILL_TASK_FAILURE_CLASSIFICATION: &str = "failure_classification_v1";
const DEFAULT_PROXY_RAW_MAX_BYTES: Option<usize> = None;
const DEFAULT_PROXY_PRICING_CATALOG_PATH: &str = "config/model-pricing.json";
const DEFAULT_PROXY_RAW_DIR: &str = "proxy_raw_payloads";
const DEFAULT_PROXY_RAW_COMPRESSION: RawCompressionCodec = RawCompressionCodec::Gzip;
const DEFAULT_PROXY_RAW_HOT_SECS: u64 = 24 * 60 * 60;
const POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES: usize = 1024 * 1024;
const ENV_DATABASE_PATH: &str = "DATABASE_PATH";
const LEGACY_ENV_DATABASE_PATH: &str = "XY_DATABASE_PATH";
const ENV_POLL_INTERVAL_SECS: &str = "POLL_INTERVAL_SECS";
const LEGACY_ENV_POLL_INTERVAL_SECS: &str = "XY_POLL_INTERVAL_SECS";
const ENV_REQUEST_TIMEOUT_SECS: &str = "REQUEST_TIMEOUT_SECS";
const LEGACY_ENV_REQUEST_TIMEOUT_SECS: &str = "XY_REQUEST_TIMEOUT_SECS";
const ENV_XRAY_BINARY: &str = "XRAY_BINARY";
const LEGACY_ENV_XRAY_BINARY: &str = "XY_XRAY_BINARY";
const ENV_XRAY_RUNTIME_DIR: &str = "XRAY_RUNTIME_DIR";
const LEGACY_ENV_XRAY_RUNTIME_DIR: &str = "XY_XRAY_RUNTIME_DIR";
const ENV_FORWARD_PROXY_ALGO: &str = "FORWARD_PROXY_ALGO";
const LEGACY_ENV_FORWARD_PROXY_ALGO: &str = "XY_FORWARD_PROXY_ALGO";
const ENV_MAX_PARALLEL_POLLS: &str = "MAX_PARALLEL_POLLS";
const LEGACY_ENV_MAX_PARALLEL_POLLS: &str = "XY_MAX_PARALLEL_POLLS";
const ENV_SHARED_CONNECTION_PARALLELISM: &str = "SHARED_CONNECTION_PARALLELISM";
const LEGACY_ENV_SHARED_CONNECTION_PARALLELISM: &str = "XY_SHARED_CONNECTION_PARALLELISM";
const ENV_HTTP_BIND: &str = "HTTP_BIND";
const LEGACY_ENV_HTTP_BIND: &str = "XY_HTTP_BIND";
const ENV_CORS_ALLOWED_ORIGINS: &str = "CORS_ALLOWED_ORIGINS";
const LEGACY_ENV_CORS_ALLOWED_ORIGINS: &str = "XY_CORS_ALLOWED_ORIGINS";
const ENV_LIST_LIMIT_MAX: &str = "LIST_LIMIT_MAX";
const LEGACY_ENV_LIST_LIMIT_MAX: &str = "XY_LIST_LIMIT_MAX";
const ENV_USER_AGENT: &str = "USER_AGENT";
const LEGACY_ENV_USER_AGENT: &str = "XY_USER_AGENT";
const ENV_STATIC_DIR: &str = "STATIC_DIR";
const LEGACY_ENV_STATIC_DIR: &str = "XY_STATIC_DIR";
const ENV_RETENTION_ENABLED: &str = "RETENTION_ENABLED";
const LEGACY_ENV_RETENTION_ENABLED: &str = "XY_RETENTION_ENABLED";
const ENV_RETENTION_DRY_RUN: &str = "RETENTION_DRY_RUN";
const LEGACY_ENV_RETENTION_DRY_RUN: &str = "XY_RETENTION_DRY_RUN";
const ENV_RETENTION_INTERVAL_SECS: &str = "RETENTION_INTERVAL_SECS";
const LEGACY_ENV_RETENTION_INTERVAL_SECS: &str = "XY_RETENTION_INTERVAL_SECS";
const ENV_RETENTION_BATCH_ROWS: &str = "RETENTION_BATCH_ROWS";
const LEGACY_ENV_RETENTION_BATCH_ROWS: &str = "XY_RETENTION_BATCH_ROWS";
const ENV_ARCHIVE_DIR: &str = "ARCHIVE_DIR";
const LEGACY_ENV_ARCHIVE_DIR: &str = "XY_ARCHIVE_DIR";
const ENV_INVOCATION_SUCCESS_FULL_DAYS: &str = "INVOCATION_SUCCESS_FULL_DAYS";
const LEGACY_ENV_INVOCATION_SUCCESS_FULL_DAYS: &str = "XY_INVOCATION_SUCCESS_FULL_DAYS";
const ENV_INVOCATION_MAX_DAYS: &str = "INVOCATION_MAX_DAYS";
const LEGACY_ENV_INVOCATION_MAX_DAYS: &str = "XY_INVOCATION_MAX_DAYS";
const ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS: &str = "FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS";
const LEGACY_ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS: &str =
    "XY_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS";
const ENV_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS: &str = "STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS";
const LEGACY_ENV_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS: &str =
    "XY_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS";
const ENV_QUOTA_SNAPSHOT_FULL_DAYS: &str = "QUOTA_SNAPSHOT_FULL_DAYS";
const ENV_PROXY_RAW_COMPRESSION: &str = "PROXY_RAW_COMPRESSION";
const ENV_PROXY_RAW_HOT_SECS: &str = "PROXY_RAW_HOT_SECS";
const LEGACY_ENV_QUOTA_SNAPSHOT_FULL_DAYS: &str = "XY_QUOTA_SNAPSHOT_FULL_DAYS";
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
const PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED: &str = "proxy path contains forbidden dot segments";
const PROXY_INVALID_REQUEST_TARGET: &str = "proxy request target is malformed";
const PROXY_UPSTREAM_HANDSHAKE_TIMEOUT: &str = "upstream handshake timed out";
const PROXY_MODEL_MERGE_STATUS_HEADER: &str = "x-proxy-model-merge-upstream";
const PROXY_MODEL_MERGE_STATUS_SUCCESS: &str = "success";
const PROXY_MODEL_MERGE_STATUS_FAILED: &str = "failed";
const PROXY_FAILURE_BODY_TOO_LARGE: &str = "body_too_large";
const PROXY_REQUEST_BODY_LIMIT_EXCEEDED: &str = "proxy request body length limit exceeded";
const PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT: &str = "request_body_read_timeout";
const PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED: &str =
    "request_body_stream_error_client_closed";
const PROXY_FAILURE_FAILED_CONTACT_UPSTREAM: &str = "failed_contact_upstream";
const PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT: &str = "upstream_handshake_timeout";
const PROXY_FAILURE_UPSTREAM_STREAM_ERROR: &str = "upstream_stream_error";
const PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED: &str = "upstream_response_failed";
const PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT: &str = "pool_no_available_account";
const PROXY_STREAM_TERMINAL_COMPLETED: &str = "stream_completed";
const PROXY_STREAM_TERMINAL_ERROR: &str = "stream_error";
const PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED: &str = "downstream_closed";
const INVOCATION_UPSTREAM_SCOPE_EXTERNAL: &str = "external";
const INVOCATION_UPSTREAM_SCOPE_INTERNAL: &str = "internal";
const INVOCATION_ROUTE_MODE_FORWARD_PROXY: &str = "forward_proxy";
const INVOCATION_ROUTE_MODE_POOL: &str = "pool";
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
const FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429: &str = "upstream_http_429";
const FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX: &str = "upstream_http_5xx";
const DEFAULT_XRAY_BINARY: &str = "xray";
const DEFAULT_XRAY_RUNTIME_DIR: &str = ".codex/xray-forward";
const XRAY_PROXY_READY_TIMEOUT_MS: u64 = 3_000;
const DEFAULT_PRICING_CATALOG_VERSION: &str = "openai-standard-2026-03-06";
const LEGACY_DEFAULT_PRICING_CATALOG_VERSION: &str = "openai-standard-2026-02-23";
const DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE: bool = true;
const DEFAULT_PROXY_MODELS_HIJACK_ENABLED: bool = false;
const DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED: bool = false;
const DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES: u8 = 3;
const MAX_PROXY_UPSTREAM_429_MAX_RETRIES: u8 = 5;
const MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS: u64 = 30;
const DEFAULT_PROXY_FAST_MODE_REWRITE_MODE: ProxyFastModeRewriteMode =
    ProxyFastModeRewriteMode::Disabled;
const DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP: bool = true;
const GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS: i64 = 272_000;
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
const LEGACY_ENV_RENAMES: &[(&str, &str)] = &[
    (LEGACY_ENV_DATABASE_PATH, ENV_DATABASE_PATH),
    (LEGACY_ENV_POLL_INTERVAL_SECS, ENV_POLL_INTERVAL_SECS),
    (LEGACY_ENV_REQUEST_TIMEOUT_SECS, ENV_REQUEST_TIMEOUT_SECS),
    (LEGACY_ENV_XRAY_BINARY, ENV_XRAY_BINARY),
    (LEGACY_ENV_XRAY_RUNTIME_DIR, ENV_XRAY_RUNTIME_DIR),
    (LEGACY_ENV_FORWARD_PROXY_ALGO, ENV_FORWARD_PROXY_ALGO),
    (LEGACY_ENV_MAX_PARALLEL_POLLS, ENV_MAX_PARALLEL_POLLS),
    (
        LEGACY_ENV_SHARED_CONNECTION_PARALLELISM,
        ENV_SHARED_CONNECTION_PARALLELISM,
    ),
    (LEGACY_ENV_HTTP_BIND, ENV_HTTP_BIND),
    (LEGACY_ENV_CORS_ALLOWED_ORIGINS, ENV_CORS_ALLOWED_ORIGINS),
    (LEGACY_ENV_LIST_LIMIT_MAX, ENV_LIST_LIMIT_MAX),
    (LEGACY_ENV_USER_AGENT, ENV_USER_AGENT),
    (LEGACY_ENV_STATIC_DIR, ENV_STATIC_DIR),
    (LEGACY_ENV_RETENTION_ENABLED, ENV_RETENTION_ENABLED),
    (LEGACY_ENV_RETENTION_DRY_RUN, ENV_RETENTION_DRY_RUN),
    (
        LEGACY_ENV_RETENTION_INTERVAL_SECS,
        ENV_RETENTION_INTERVAL_SECS,
    ),
    (LEGACY_ENV_RETENTION_BATCH_ROWS, ENV_RETENTION_BATCH_ROWS),
    (LEGACY_ENV_ARCHIVE_DIR, ENV_ARCHIVE_DIR),
    (
        LEGACY_ENV_INVOCATION_SUCCESS_FULL_DAYS,
        ENV_INVOCATION_SUCCESS_FULL_DAYS,
    ),
    (LEGACY_ENV_INVOCATION_MAX_DAYS, ENV_INVOCATION_MAX_DAYS),
    (
        LEGACY_ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
        ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
    ),
    (
        LEGACY_ENV_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
        ENV_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
    ),
    (
        LEGACY_ENV_QUOTA_SNAPSHOT_FULL_DAYS,
        ENV_QUOTA_SNAPSHOT_FULL_DAYS,
    ),
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
            run_data_retention_maintenance(&pool, &config, Some(cli.retention_dry_run), None)
                .await?;
        info!(?summary, "retention maintenance run-once finished");
        return Ok(());
    }

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
    let upstream_accounts = Arc::new(UpstreamAccountsRuntime::from_env()?);
    let (tx, _rx) = broadcast::channel(128);
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let shutdown = CancellationToken::new();

    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster: tx.clone(),
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        startup_ready: Arc::new(AtomicBool::new(false)),
        shutdown: shutdown.clone(),
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
        upstream_accounts,
    });

    let signal_listener = spawn_shutdown_signal_listener(state.shutdown.clone());

    run_runtime_until_shutdown(state, startup_started_at, async move {
        let _ = signal_listener.await;
    })
    .await
}

fn begin_runtime_shutdown_if_requested<F>(
    shutdown_signal: &Shared<F>,
    cancel: &CancellationToken,
) -> bool
where
    F: Future<Output = ()>,
{
    if cancel.is_cancelled() {
        return true;
    }
    if shutdown_signal.clone().now_or_never().is_some() {
        begin_runtime_shutdown(cancel);
        return true;
    }
    false
}

enum StartupStageOutcome<T> {
    SkippedByShutdown,
    Completed { result: T, shutdown_requested: bool },
}

struct TrackedStartupStage<Stage> {
    stage: std::pin::Pin<Box<Stage>>,
    started: bool,
}

impl<Stage> TrackedStartupStage<Stage> {
    fn new(stage: Stage) -> Self {
        Self {
            stage: Box::pin(stage),
            started: false,
        }
    }

    fn has_started(&self) -> bool {
        self.started
    }
}

impl<Stage> TrackedStartupStage<Stage>
where
    Stage: Future,
{
    async fn finish(&mut self) -> Stage::Output {
        self.started = true;
        self.stage.as_mut().await
    }
}

impl<Stage> Future for TrackedStartupStage<Stage>
where
    Stage: Future,
{
    type Output = Stage::Output;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        this.started = true;
        this.stage.as_mut().poll(cx)
    }
}

async fn run_startup_stage_until_shutdown<T, Stage, Shutdown>(
    shutdown_signal: &Shared<Shutdown>,
    cancel: &CancellationToken,
    stage: Stage,
) -> StartupStageOutcome<T>
where
    Stage: Future<Output = T>,
    Shutdown: Future<Output = ()>,
{
    if begin_runtime_shutdown_if_requested(shutdown_signal, cancel) {
        return StartupStageOutcome::SkippedByShutdown;
    }

    let stage = TrackedStartupStage::new(stage);
    tokio::pin!(stage);
    tokio::select! {
        biased;
        _ = shutdown_signal.clone() => {
            begin_runtime_shutdown(cancel);
            if stage.as_ref().get_ref().has_started() {
                StartupStageOutcome::Completed {
                    result: stage.as_mut().get_mut().finish().await,
                    shutdown_requested: true,
                }
            } else {
                StartupStageOutcome::SkippedByShutdown
            }
        }
        result = &mut stage => StartupStageOutcome::Completed {
            shutdown_requested: begin_runtime_shutdown_if_requested(shutdown_signal, cancel),
            result,
        },
    }
}

#[allow(clippy::too_many_arguments)]
async fn drain_runtime_after_pending_shutdown(
    state: Arc<AppState>,
    mut shutdown_watcher: JoinHandle<()>,
    server_handle: Option<JoinHandle<()>>,
    poller_handle: Option<JoinHandle<()>>,
    upstream_accounts_handle: Option<JoinHandle<()>>,
    forward_proxy_handle: Option<JoinHandle<()>>,
    retention_handle: Option<JoinHandle<()>>,
    startup_backfill_handle: Option<JoinHandle<()>>,
) -> Result<()> {
    let shutdown_cancel = state.shutdown.clone();
    tokio::select! {
        _ = shutdown_cancel.cancelled() => {
            shutdown_watcher.abort();
            let _ = shutdown_watcher.await;
        }
        _ = &mut shutdown_watcher => {}
    }
    drain_runtime_after_shutdown(
        state,
        server_handle,
        poller_handle,
        upstream_accounts_handle,
        forward_proxy_handle,
        retention_handle,
        startup_backfill_handle,
    )
    .await
}

async fn run_runtime_until_shutdown<F>(
    state: Arc<AppState>,
    startup_started_at: Instant,
    shutdown_signal: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let cancel = state.shutdown.clone();
    let runtime_init_started_at = Instant::now();
    let shutdown_signal = shutdown_signal.shared();
    let shutdown_cancel = cancel.clone();
    let shutdown_relay_signal = shutdown_signal.clone();
    let shutdown_watcher = tokio::spawn(async move {
        shutdown_relay_signal.await;
        begin_runtime_shutdown(&shutdown_cancel);
    });
    let mut poller_handle = None;
    let mut upstream_accounts_handle = None;
    let mut forward_proxy_handle = None;
    let mut retention_handle = None;
    let mut server_handle = None;
    let mut startup_backfill_handle = None;

    let sync_stage = run_startup_stage_until_shutdown(
        &shutdown_signal,
        &cancel,
        sync_forward_proxy_routes(state.as_ref()),
    )
    .await;
    let sync_shutdown_requested = match sync_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                retention_handle,
                startup_backfill_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            if let Err(err) = result {
                warn!(error = %err, "failed to initialize forward proxy xray routes at startup");
            }
            shutdown_requested
        }
    };
    log_startup_phase("runtime_init", runtime_init_started_at);
    if sync_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            retention_handle,
            startup_backfill_handle,
        )
        .await;
    }

    let scheduler_stage = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        if state.config.crs_stats.is_some() {
            Some(spawn_scheduler(state.clone(), cancel.clone()))
        } else {
            info!("crs stats relay is disabled; scheduler will not start");
            None
        }
    })
    .await;
    let scheduler_shutdown_requested = match scheduler_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                retention_handle,
                startup_backfill_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            poller_handle = result;
            shutdown_requested
        }
    };
    if scheduler_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            retention_handle,
            startup_backfill_handle,
        )
        .await;
    }

    let upstream_accounts_stage =
        run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
            Some(spawn_upstream_account_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        })
        .await;
    let upstream_accounts_shutdown_requested = match upstream_accounts_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                retention_handle,
                startup_backfill_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            upstream_accounts_handle = result;
            shutdown_requested
        }
    };
    if upstream_accounts_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            retention_handle,
            startup_backfill_handle,
        )
        .await;
    }

    let forward_proxy_stage = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        Some(spawn_forward_proxy_maintenance(
            state.clone(),
            cancel.clone(),
        ))
    })
    .await;
    let forward_proxy_shutdown_requested = match forward_proxy_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                retention_handle,
                startup_backfill_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            forward_proxy_handle = result;
            shutdown_requested
        }
    };
    if forward_proxy_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            retention_handle,
            startup_backfill_handle,
        )
        .await;
    }

    let retention_stage = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        Some(spawn_data_retention_maintenance(
            state.clone(),
            cancel.clone(),
        ))
    })
    .await;
    let retention_shutdown_requested = match retention_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                retention_handle,
                startup_backfill_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            retention_handle = result;
            shutdown_requested
        }
    };
    if retention_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            retention_handle,
            startup_backfill_handle,
        )
        .await;
    }
    let http_ready_started_at = Instant::now();
    let http_stage = run_startup_stage_until_shutdown(
        &shutdown_signal,
        &cancel,
        spawn_http_server(state.clone()),
    )
    .await;
    let http_shutdown_requested = match http_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                retention_handle,
                startup_backfill_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            let (_http_addr, handle) = result?;
            server_handle = Some(handle);
            shutdown_requested
        }
    };
    if http_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            retention_handle,
            startup_backfill_handle,
        )
        .await;
    }

    let startup_backfill_stage =
        run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
            Some(spawn_startup_backfill_maintenance(
                state.clone(),
                cancel.clone(),
            ))
        })
        .await;
    let startup_backfill_shutdown_requested = match startup_backfill_stage {
        StartupStageOutcome::SkippedByShutdown => {
            return drain_runtime_after_pending_shutdown(
                state,
                shutdown_watcher,
                server_handle,
                poller_handle,
                upstream_accounts_handle,
                forward_proxy_handle,
                retention_handle,
                startup_backfill_handle,
            )
            .await;
        }
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            startup_backfill_handle = result;
            shutdown_requested
        }
    };
    if startup_backfill_shutdown_requested {
        return drain_runtime_after_pending_shutdown(
            state,
            shutdown_watcher,
            server_handle,
            poller_handle,
            upstream_accounts_handle,
            forward_proxy_handle,
            retention_handle,
            startup_backfill_handle,
        )
        .await;
    }

    state.startup_ready.store(true, Ordering::Release);
    log_startup_phase("http_ready", http_ready_started_at);
    info!(
        time_to_health_ms = startup_started_at.elapsed().as_millis() as u64,
        "application readiness reached"
    );

    tokio::select! {
        biased;
        _ = shutdown_signal => begin_runtime_shutdown(&cancel),
        _ = cancel.cancelled() => {}
    }

    drain_runtime_after_pending_shutdown(
        state,
        shutdown_watcher,
        server_handle,
        poller_handle,
        upstream_accounts_handle,
        forward_proxy_handle,
        retention_handle,
        startup_backfill_handle,
    )
    .await
}

fn begin_runtime_shutdown(cancel: &CancellationToken) {
    if !cancel.is_cancelled() {
        info!("shutdown signal received; beginning graceful shutdown");
        cancel.cancel();
    }
}

async fn drain_scheduler_inflight(mut inflight: Vec<JoinHandle<()>>) {
    inflight.retain(|handle| !handle.is_finished());
    for handle in inflight {
        let _ = handle.await;
    }
}

async fn drain_runtime_after_shutdown(
    state: Arc<AppState>,
    server_handle: Option<JoinHandle<()>>,
    poller_handle: Option<JoinHandle<()>>,
    upstream_accounts_handle: Option<JoinHandle<()>>,
    forward_proxy_handle: Option<JoinHandle<()>>,
    retention_handle: Option<JoinHandle<()>>,
    startup_backfill_handle: Option<JoinHandle<()>>,
) -> Result<()> {
    if let Some(server_handle) = server_handle {
        info!("http server graceful drain started");
        if let Err(err) = server_handle.await {
            error!(?err, "http server terminated unexpectedly");
        }
        info!("http server graceful drain finished");
    }

    if let Some(poller_handle) = poller_handle {
        if let Err(err) = poller_handle.await {
            error!(?err, "poller task terminated unexpectedly");
        }
        info!("scheduler drained");
    }
    if let Some(upstream_accounts_handle) = upstream_accounts_handle
        && let Err(err) = upstream_accounts_handle.await
    {
        error!(
            ?err,
            "upstream account maintenance task terminated unexpectedly"
        );
    }
    if let Some(forward_proxy_handle) = forward_proxy_handle
        && let Err(err) = forward_proxy_handle.await
    {
        error!(
            ?err,
            "forward proxy maintenance task terminated unexpectedly"
        );
    }
    if let Some(retention_handle) = retention_handle
        && let Err(err) = retention_handle.await
    {
        error!(?err, "retention maintenance task terminated unexpectedly");
    }
    if let Some(startup_backfill_handle) = startup_backfill_handle
        && let Err(err) = startup_backfill_handle.await
    {
        error!(
            ?err,
            "startup backfill maintenance task terminated unexpectedly"
        );
    }

    let broadcast_handles = {
        let mut guard = state.proxy_summary_quota_broadcast_handle.lock().await;
        std::mem::take(&mut *guard)
    };
    if !broadcast_handles.is_empty() {
        for broadcast_handle in broadcast_handles {
            if let Err(err) = broadcast_handle.await {
                error!(
                    ?err,
                    "summary/quota broadcast worker terminated unexpectedly"
                );
            }
        }
        info!("summary/quota broadcast worker drained");
    }

    state.xray_supervisor.lock().await.shutdown_all().await;
    info!("shutdown complete");

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
    UpstreamActivityLive,
    UpstreamActivityArchives,
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
            Self::UpstreamActivityLive,
            Self::UpstreamActivityArchives,
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
            Self::UpstreamActivityLive => STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_LIVE,
            Self::UpstreamActivityArchives => STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES,
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
            Self::UpstreamActivityLive => "upstream activity live rows",
            Self::UpstreamActivityArchives => "upstream activity archives",
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

async fn run_startup_backfill_maintenance_pass(state: Arc<AppState>, cancel: &CancellationToken) {
    for task in StartupBackfillTask::ordered_tasks() {
        if cancel.is_cancelled() {
            info!(
                task = task.log_label(),
                "startup backfill maintenance stopped at a task boundary because shutdown is in progress"
            );
            break;
        }
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
                raw_path_fallback_root,
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
                    samples: outcome.samples,
                },
                "failure classification recalculated".to_string(),
            ))
        }
        StartupBackfillTask::UpstreamActivityLive => {
            let updated_accounts =
                backfill_upstream_account_last_activity_from_live_invocations(&state.pool).await?;
            let pending_accounts =
                count_upstream_accounts_missing_live_last_activity(&state.pool).await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: 0,
                    updated: updated_accounts,
                    hit_scan_limit: false,
                    samples: Vec::new(),
                },
                format!("pending_accounts={pending_accounts}"),
            ))
        }
        StartupBackfillTask::UpstreamActivityArchives => {
            let summary = backfill_upstream_account_last_activity_from_archives(
                &state.pool,
                Some(STARTUP_BACKFILL_SCAN_LIMIT),
                max_elapsed,
            )
            .await?;
            let pending_accounts =
                count_upstream_accounts_missing_last_activity(&state.pool).await?;
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: summary.scanned_batches,
                    updated: summary.updated_accounts,
                    hit_scan_limit: pending_accounts > 0 && summary.hit_budget,
                    samples: Vec::new(),
                },
                format!("pending_accounts={pending_accounts}"),
            ))
        }
    }
}

fn spawn_startup_backfill_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if cancel.is_cancelled() {
            info!("startup backfill maintenance skipped because shutdown is already in progress");
            return;
        }
        run_startup_backfill_maintenance_pass(state.clone(), &cancel).await;

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
                    run_startup_backfill_maintenance_pass(state.clone(), &cancel).await;
                }
            }
        }
    })
}

fn spawn_scheduler(state: Arc<AppState>, cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut inflight: Vec<JoinHandle<()>> = Vec::new();
        if cancel.is_cancelled() {
            info!("scheduler startup skipped because shutdown is already in progress");
            return;
        }
        match schedule_poll(state.clone(), &cancel).await {
            Ok(Some(handle)) => inflight.push(handle),
            Ok(None) => {
                info!("scheduler startup skipped because shutdown is already in progress");
                return;
            }
            Err(err) => warn!(?err, "initial poll failed"),
        }

        let mut ticker = interval(state.config.poll_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("scheduler received shutdown; waiting for in-flight polls");
                    drain_scheduler_inflight(inflight).await;
                    break;
                }
                _ = ticker.tick() => {
                    match schedule_poll(state.clone(), &cancel).await {
                        Ok(Some(handle)) => {
                            inflight.push(handle);
                            inflight.retain(|handle| !handle.is_finished());
                        }
                        Ok(None) => {
                            info!("scheduler received shutdown while waiting to start a new poll; waiting for in-flight polls");
                            drain_scheduler_inflight(inflight).await;
                            break;
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
        if cancel.is_cancelled() {
            info!("forward proxy maintenance skipped because shutdown is already in progress");
            return;
        }
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
    raw_files_compression_candidates: usize,
    raw_files_compressed: usize,
    raw_bytes_before: u64,
    raw_bytes_after: u64,
    raw_bytes_after_estimated: u64,
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
        self.raw_files_compression_candidates > 0
            || self.raw_files_compressed > 0
            || self.invocation_details_pruned > 0
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
struct InvocationRawCompressionCandidate {
    id: i64,
    occurred_at: String,
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
}

#[derive(Debug, FromRow)]
struct ArchiveBatchFileRow {
    _id: i64,
    file_path: String,
}

#[derive(Debug, FromRow)]
struct ArchivedAccountLastActivityRow {
    account_id: i64,
    last_activity_at: String,
}

#[derive(Debug, Default)]
struct ArchiveBackfillSummary {
    scanned_batches: u64,
    updated_accounts: u64,
    hit_budget: bool,
}

#[derive(Debug)]
struct TempSqliteCleanup(PathBuf);

impl Drop for TempSqliteCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[derive(Debug, Default)]
struct RawCompressionPassSummary {
    files_considered: usize,
    files_compressed: usize,
    bytes_before: u64,
    bytes_after: u64,
    estimated_bytes_after: u64,
}

#[derive(Debug, Default)]
struct RawCompressionFileOutcome {
    candidate_counted: bool,
    compressed: bool,
    bytes_before: u64,
    bytes_after: u64,
    estimated_bytes_after: u64,
    new_db_path: Option<String>,
    old_exact_path: Option<PathBuf>,
}

struct CountingWriter<W> {
    inner: W,
    bytes_written: u64,
}

impl<W> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            bytes_written: 0,
        }
    }

    fn bytes_written(&self) -> u64 {
        self.bytes_written
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.bytes_written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
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

const CODEX_INVOCATIONS_ARCHIVE_COLUMNS: &str = "id, invoke_id, occurred_at, source, model, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, status, error_message, failure_kind, failure_class, is_actionable, payload, raw_response, cost_estimated, price_version, request_raw_path, request_raw_size, request_raw_truncated, request_raw_truncated_reason, response_raw_path, response_raw_size, response_raw_truncated, response_raw_truncated_reason, detail_level, detail_pruned_at, detail_prune_reason, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, created_at";
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

        if cancel.is_cancelled() {
            info!("data retention maintenance skipped because shutdown is already in progress");
            return;
        }
        if let Err(err) =
            run_data_retention_maintenance(&state.pool, &state.config, None, Some(&cancel)).await
        {
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
                    if let Err(err) = run_data_retention_maintenance(
                        &state.pool,
                        &state.config,
                        None,
                        Some(&cancel),
                    ).await {
                        warn!(error = %err, "failed to run retention maintenance");
                    }
                }
            }
        }
    })
}

fn should_stop_data_retention_maintenance(shutdown: Option<&CancellationToken>) -> bool {
    let should_stop = shutdown.is_some_and(CancellationToken::is_cancelled);
    if should_stop {
        info!(
            "data retention maintenance stopped at a safe boundary because shutdown is in progress"
        );
    }
    should_stop
}

async fn run_data_retention_maintenance(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run_override: Option<bool>,
    shutdown: Option<&CancellationToken>,
) -> Result<RetentionRunSummary> {
    let dry_run = dry_run_override.unwrap_or(config.retention_dry_run);
    let mut summary = RetentionRunSummary {
        dry_run,
        ..RetentionRunSummary::default()
    };
    let raw_path_fallback_root = config.database_path.parent();

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let raw_compression =
        compress_cold_proxy_raw_payloads(pool, config, raw_path_fallback_root, dry_run).await?;
    summary.raw_files_compression_candidates += raw_compression.files_considered;
    summary.raw_files_compressed += raw_compression.files_compressed;
    summary.raw_bytes_before += raw_compression.bytes_before;
    summary.raw_bytes_after += raw_compression.bytes_after;
    summary.raw_bytes_after_estimated += raw_compression.estimated_bytes_after;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let pruned =
        prune_old_invocation_details(pool, config, raw_path_fallback_root, dry_run).await?;
    summary.invocation_details_pruned += pruned.0;
    summary.archive_batches_touched += pruned.1;
    summary.raw_files_removed += pruned.2;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let invocation_archive =
        archive_old_invocations(pool, config, raw_path_fallback_root, dry_run).await?;
    summary.invocation_rows_archived += invocation_archive.0;
    summary.archive_batches_touched += invocation_archive.1;
    summary.raw_files_removed += invocation_archive.2;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

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

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

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

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    let quota_archive = compact_old_quota_snapshots(pool, config, dry_run).await?;
    summary.quota_snapshot_rows_archived += quota_archive.0;
    summary.archive_batches_touched += quota_archive.1;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    summary.orphan_raw_files_removed +=
        sweep_orphan_proxy_raw_files(pool, config, raw_path_fallback_root, dry_run).await?;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

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

async fn compress_cold_proxy_raw_payloads(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<RawCompressionPassSummary> {
    if config.proxy_raw_compression == RawCompressionCodec::None {
        return Ok(RawCompressionPassSummary::default());
    }

    let cutoff = shanghai_local_cutoff_for_age_secs_string(config.proxy_raw_hot_secs);
    let prune_cutoff = shanghai_local_cutoff_string(config.invocation_success_full_days);
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let mut summary = RawCompressionPassSummary::default();
    let mut last_seen_occurred_at: Option<String> = None;
    let mut last_seen_id = 0_i64;

    loop {
        let candidates = sqlx::query_as::<_, InvocationRawCompressionCandidate>(
            r#"
            SELECT id, occurred_at, request_raw_path, response_raw_path
            FROM codex_invocations
            WHERE occurred_at < ?1
              AND occurred_at >= ?2
              AND (
                status != 'success'
                OR detail_level IS NULL
                OR detail_level != ?3
                OR occurred_at >= ?4
              )
              AND (
                (request_raw_path IS NOT NULL AND request_raw_path NOT LIKE '%.gz')
                OR (response_raw_path IS NOT NULL AND response_raw_path NOT LIKE '%.gz')
              )
              AND (
                ?5 IS NULL
                OR occurred_at > ?5
                OR (occurred_at = ?5 AND id > ?6)
              )
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?7
            "#,
        )
        .bind(&cutoff)
        .bind(&archive_cutoff)
        .bind(DETAIL_LEVEL_FULL)
        .bind(&prune_cutoff)
        .bind(last_seen_occurred_at.as_deref())
        .bind(last_seen_id)
        .bind(config.retention_batch_rows as i64)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_occurred_at = Some(candidate.occurred_at.clone());
            last_seen_id = candidate.id;
            let request_outcome = match maybe_compress_proxy_raw_path(
                pool,
                candidate.id,
                "request_raw_path",
                candidate.request_raw_path.as_deref(),
                config.proxy_raw_compression,
                raw_path_fallback_root,
                dry_run,
            )
            .await
            {
                Ok(outcome) => Some(outcome),
                Err(err) => {
                    warn!(
                        invocation_id = candidate.id,
                        field = "request_raw_path",
                        error = %err,
                        "failed to cold-compress raw payload file; continuing retention"
                    );
                    None
                }
            };
            let response_outcome = match maybe_compress_proxy_raw_path(
                pool,
                candidate.id,
                "response_raw_path",
                candidate.response_raw_path.as_deref(),
                config.proxy_raw_compression,
                raw_path_fallback_root,
                dry_run,
            )
            .await
            {
                Ok(outcome) => Some(outcome),
                Err(err) => {
                    warn!(
                        invocation_id = candidate.id,
                        field = "response_raw_path",
                        error = %err,
                        "failed to cold-compress raw payload file; continuing retention"
                    );
                    None
                }
            };

            let request_db_path = request_outcome
                .as_ref()
                .and_then(|outcome| outcome.new_db_path.clone())
                .or_else(|| candidate.request_raw_path.clone());
            let response_db_path = response_outcome
                .as_ref()
                .and_then(|outcome| outcome.new_db_path.clone())
                .or_else(|| candidate.response_raw_path.clone());

            if !dry_run {
                let request_path_changed = request_db_path != candidate.request_raw_path;
                let response_path_changed = response_db_path != candidate.response_raw_path;
                if request_path_changed || response_path_changed {
                    let mut tx = pool.begin().await?;
                    sqlx::query(
                        r#"
                        UPDATE codex_invocations
                        SET request_raw_path = ?1,
                            response_raw_path = ?2
                        WHERE id = ?3
                        "#,
                    )
                    .bind(request_db_path.as_deref())
                    .bind(response_db_path.as_deref())
                    .bind(candidate.id)
                    .execute(tx.as_mut())
                    .await?;
                    tx.commit().await?;
                }

                if let Some(outcome) = request_outcome
                    .as_ref()
                    .filter(|outcome| outcome.compressed)
                {
                    delete_exact_proxy_raw_path(
                        outcome.old_exact_path.as_deref(),
                        raw_path_fallback_root,
                    )?;
                }
                if let Some(outcome) = response_outcome
                    .as_ref()
                    .filter(|outcome| outcome.compressed)
                {
                    delete_exact_proxy_raw_path(
                        outcome.old_exact_path.as_deref(),
                        raw_path_fallback_root,
                    )?;
                }
            }

            for outcome in [request_outcome.as_ref(), response_outcome.as_ref()]
                .into_iter()
                .flatten()
            {
                if outcome.candidate_counted {
                    summary.files_considered += 1;
                }
                if outcome.compressed {
                    summary.files_compressed += 1;
                }
                summary.bytes_before += outcome.bytes_before;
                summary.bytes_after += outcome.bytes_after;
                summary.estimated_bytes_after += outcome.estimated_bytes_after;
            }
        }
    }

    Ok(summary)
}

async fn maybe_compress_proxy_raw_path(
    _pool: &Pool<Sqlite>,
    invocation_id: i64,
    field_name: &str,
    raw_path: Option<&str>,
    codec: RawCompressionCodec,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<RawCompressionFileOutcome> {
    let Some(raw_path) = raw_path else {
        return Ok(RawCompressionFileOutcome::default());
    };
    if codec == RawCompressionCodec::None || raw_path.ends_with(".gz") {
        return Ok(RawCompressionFileOutcome {
            new_db_path: Some(raw_path.to_string()),
            ..RawCompressionFileOutcome::default()
        });
    }

    let Some(source_path) = locate_existing_proxy_raw_path(raw_path, raw_path_fallback_root) else {
        let existing_compressed =
            locate_existing_proxy_raw_compressed_path(raw_path, raw_path_fallback_root);
        if existing_compressed.is_some() {
            return Ok(RawCompressionFileOutcome {
                new_db_path: Some(raw_payload_compressed_db_path(raw_path)),
                ..RawCompressionFileOutcome::default()
            });
        }
        warn!(
            invocation_id,
            field = field_name,
            raw_path,
            "skipping raw cold compression because source raw file is missing"
        );
        return Ok(RawCompressionFileOutcome {
            new_db_path: Some(raw_path.to_string()),
            ..RawCompressionFileOutcome::default()
        });
    };

    let source_meta = fs::metadata(&source_path).with_context(|| {
        format!(
            "failed to inspect raw payload before cold compression: {}",
            source_path.display()
        )
    })?;
    if !source_meta.is_file() {
        return Ok(RawCompressionFileOutcome {
            new_db_path: Some(raw_path.to_string()),
            ..RawCompressionFileOutcome::default()
        });
    }

    let target_db_path = raw_payload_compressed_db_path(raw_path);
    let target_path = raw_payload_compressed_file_path(&source_path);
    let bytes_before = source_meta.len();
    if dry_run {
        let estimated_bytes_after = estimate_gzip_file_size(&source_path)?;
        return Ok(RawCompressionFileOutcome {
            candidate_counted: true,
            bytes_before,
            estimated_bytes_after,
            new_db_path: Some(target_db_path),
            old_exact_path: Some(source_path),
            ..RawCompressionFileOutcome::default()
        });
    }

    let bytes_after = compress_file_to_gzip(&source_path, &target_path)?;
    Ok(RawCompressionFileOutcome {
        candidate_counted: true,
        compressed: true,
        bytes_before,
        bytes_after,
        new_db_path: Some(target_db_path),
        old_exact_path: Some(source_path),
        ..RawCompressionFileOutcome::default()
    })
}

fn compress_file_to_gzip(source: &Path, destination: &Path) -> Result<u64> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create raw compression directory {}",
                parent.display()
            )
        })?;
    }

    let temp_destination = PathBuf::from(format!("{}.tmp", destination.display()));
    if temp_destination.exists() {
        let _ = fs::remove_file(&temp_destination);
    }

    let result = (|| -> Result<u64> {
        let input = fs::File::open(source)
            .with_context(|| format!("failed to open raw payload {}", source.display()))?;
        let output = fs::File::create(&temp_destination).with_context(|| {
            format!(
                "failed to create compressed raw payload {}",
                temp_destination.display()
            )
        })?;
        let mut reader = io::BufReader::new(input);
        let counting_writer = CountingWriter::new(io::BufWriter::new(output));
        let mut encoder = GzEncoder::new(counting_writer, Compression::default());
        io::copy(&mut reader, &mut encoder).with_context(|| {
            format!(
                "failed to compress raw payload {} into {}",
                source.display(),
                temp_destination.display()
            )
        })?;
        let mut counting_writer = encoder.finish().with_context(|| {
            format!(
                "failed to finish raw payload compression {}",
                temp_destination.display()
            )
        })?;
        counting_writer.flush()?;
        let bytes_after = counting_writer.bytes_written();
        let mut output = counting_writer.inner;
        output.flush()?;
        fs::rename(&temp_destination, destination).with_context(|| {
            format!(
                "failed to move compressed raw payload into place: {} -> {}",
                temp_destination.display(),
                destination.display()
            )
        })?;
        Ok(bytes_after)
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_destination);
    }
    result
}

fn estimate_gzip_file_size(source: &Path) -> Result<u64> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open raw payload {}", source.display()))?;
    let mut reader = io::BufReader::new(input);
    let counting_writer = CountingWriter::new(io::sink());
    let mut encoder = GzEncoder::new(counting_writer, Compression::default());
    io::copy(&mut reader, &mut encoder).with_context(|| {
        format!(
            "failed to estimate gzip size for raw payload {}",
            source.display()
        )
    })?;
    let counting_writer = encoder.finish().with_context(|| {
        format!(
            "failed to finish gzip size estimate for raw payload {}",
            source.display()
        )
    })?;
    Ok(counting_writer.bytes_written())
}

fn raw_payload_compressed_db_path(raw_path: &str) -> String {
    if raw_path.ends_with(".gz") {
        raw_path.to_string()
    } else {
        format!("{raw_path}.gz")
    }
}

fn raw_payload_compressed_file_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.gz", path.display()))
}

fn locate_existing_proxy_raw_path(path: &str, fallback_root: Option<&Path>) -> Option<PathBuf> {
    resolved_raw_path_candidates(path, fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn locate_existing_proxy_raw_compressed_path(
    path: &str,
    fallback_root: Option<&Path>,
) -> Option<PathBuf> {
    resolved_raw_path_candidates(&raw_payload_compressed_db_path(path), fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn delete_exact_proxy_raw_path(
    raw_path: Option<&Path>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<()> {
    let Some(raw_path) = raw_path else {
        return Ok(());
    };
    let raw_path = raw_path.to_string_lossy();
    for candidate in resolved_raw_path_candidates(&raw_path, raw_path_fallback_root) {
        match fs::remove_file(&candidate) {
            Ok(_) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => {
                warn!(
                    path = %candidate.display(),
                    error = %err,
                    "failed to remove replaced raw payload after cold compression"
                );
                return Ok(());
            }
        }
    }
    Ok(())
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
                "UPDATE codex_invocations SET payload = NULL, raw_response = '', request_raw_path = NULL, request_raw_size = NULL, request_raw_truncated = 0, request_raw_truncated_reason = NULL, response_raw_path = NULL, response_raw_size = NULL, response_raw_truncated = 0, response_raw_truncated_reason = NULL, detail_level = ",
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

async fn backfill_upstream_account_last_activity_from_archives(
    pool: &Pool<Sqlite>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<ArchiveBackfillSummary> {
    let started_at = Instant::now();
    let pending_account_ids = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_accounts
        WHERE last_activity_at IS NULL
          AND last_activity_archive_backfill_completed = 0
        "#,
    )
    .fetch_all(pool)
    .await?;
    if pending_account_ids.is_empty() {
        return Ok(ArchiveBackfillSummary::default());
    }

    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id AS _id, file_path
        FROM archive_batches
        WHERE dataset = 'codex_invocations' AND status = ?1
        ORDER BY month_key DESC, created_at DESC, id DESC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    if archive_files.is_empty() {
        mark_archive_backfill_completed_for_accounts(pool, &pending_account_ids).await?;
        return Ok(ArchiveBackfillSummary::default());
    }

    let pending = pending_account_ids.into_iter().collect::<HashSet<_>>();
    let mut recovered = HashMap::<i64, String>::new();
    let mut scanned_batches = 0_u64;
    let mut exhausted_archives = true;
    let mut hit_budget = false;

    for archive_file in archive_files {
        if startup_backfill_budget_reached(started_at, scanned_batches, scan_limit, max_elapsed) {
            exhausted_archives = false;
            hit_budget = true;
            break;
        }
        if recovered.len() == pending.len() {
            exhausted_archives = false;
            break;
        }

        let archive_path = PathBuf::from(archive_file.file_path);
        if !archive_path.exists() {
            exhausted_archives = false;
            continue;
        }
        scanned_batches += 1;

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());

        inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
        let database_url = format!("sqlite://{}", temp_path.to_string_lossy());
        let connect_opts = build_sqlite_connect_options(
            &database_url,
            Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
        )?;
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(connect_opts)
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;

        let rows = {
            const ACCOUNT_EXPR: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
            let mut rows = Vec::new();
            let remaining_account_ids = pending
                .iter()
                .copied()
                .filter(|account_id| !recovered.contains_key(account_id))
                .collect::<Vec<_>>();
            for account_ids in remaining_account_ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
                let mut query = QueryBuilder::<Sqlite>::new(
                    "SELECT account_id, MAX(occurred_at) AS last_activity_at FROM (SELECT ",
                );
                query
                    .push(ACCOUNT_EXPR)
                    .push(" AS account_id, occurred_at FROM codex_invocations WHERE ")
                    .push(ACCOUNT_EXPR)
                    .push(" IN (");
                {
                    let mut separated = query.separated(", ");
                    for account_id in account_ids {
                        separated.push_bind(account_id);
                    }
                }
                query.push(")) WHERE account_id IS NOT NULL GROUP BY account_id");
                rows.extend(
                    query
                        .build_query_as::<ArchivedAccountLastActivityRow>()
                        .fetch_all(&archive_pool)
                        .await?,
                );
            }
            rows
        };

        archive_pool.close().await;
        drop(temp_cleanup);

        for row in rows {
            recovered
                .entry(row.account_id)
                .and_modify(|current| {
                    if *current < row.last_activity_at {
                        *current = row.last_activity_at.clone();
                    }
                })
                .or_insert(row.last_activity_at);
        }
    }

    if recovered.is_empty() {
        if exhausted_archives {
            let pending_account_ids = pending.iter().copied().collect::<Vec<_>>();
            mark_archive_backfill_completed_for_accounts(pool, &pending_account_ids).await?;
        }
        return Ok(ArchiveBackfillSummary {
            scanned_batches,
            updated_accounts: 0,
            hit_budget,
        });
    }

    let unresolved: Vec<i64> = pending
        .iter()
        .copied()
        .filter(|account_id| !recovered.contains_key(account_id))
        .collect();
    let updated_accounts = recovered.len() as u64;
    let mut tx = pool.begin().await?;
    for (account_id, occurred_at) in recovered {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = ?1,
                last_activity_archive_backfill_completed = 1
            WHERE id = ?2 AND last_activity_at IS NULL
            "#,
        )
        .bind(occurred_at)
        .bind(account_id)
        .execute(tx.as_mut())
        .await?;
    }
    if exhausted_archives && !unresolved.is_empty() {
        mark_archive_backfill_completed_for_accounts_tx(tx.as_mut(), &unresolved).await?;
    }
    tx.commit().await?;

    Ok(ArchiveBackfillSummary {
        scanned_batches,
        updated_accounts,
        hit_budget,
    })
}

async fn mark_archive_backfill_completed_for_accounts(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<()> {
    if account_ids.is_empty() {
        return Ok(());
    }
    for account_chunk in account_ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut update = QueryBuilder::<Sqlite>::new(
            "UPDATE pool_upstream_accounts SET last_activity_archive_backfill_completed = 1 WHERE id IN (",
        );
        {
            let mut separated = update.separated(", ");
            for account_id in account_chunk {
                separated.push_bind(account_id);
            }
        }
        update.push(")");
        update.build().execute(pool).await?;
    }
    Ok(())
}

async fn mark_archive_backfill_completed_for_accounts_tx(
    tx: &mut SqliteConnection,
    account_ids: &[i64],
) -> Result<()> {
    if account_ids.is_empty() {
        return Ok(());
    }
    for account_chunk in account_ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut update = QueryBuilder::<Sqlite>::new(
            "UPDATE pool_upstream_accounts SET last_activity_archive_backfill_completed = 1 WHERE id IN (",
        );
        {
            let mut separated = update.separated(", ");
            for account_id in account_chunk {
                separated.push_bind(account_id);
            }
        }
        update.push(")");
        update.build().execute(&mut *tx).await?;
    }
    Ok(())
}

async fn count_upstream_accounts_missing_last_activity(pool: &Pool<Sqlite>) -> Result<u64> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
            SELECT COUNT(*)
            FROM pool_upstream_accounts
            WHERE last_activity_at IS NULL
              AND last_activity_archive_backfill_completed = 0
            "#,
    )
    .fetch_one(pool)
    .await?
    .max(0) as u64)
}

async fn count_upstream_accounts_missing_live_last_activity(pool: &Pool<Sqlite>) -> Result<u64> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
            SELECT COUNT(*)
            FROM pool_upstream_accounts
            WHERE last_activity_at IS NULL
              AND last_activity_live_backfill_completed = 0
            "#,
    )
    .fetch_one(pool)
    .await?
    .max(0) as u64)
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
    if batch.dataset == "codex_invocations" {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_archive_backfill_completed = 0
            WHERE last_activity_at IS NULL
            "#,
        )
        .execute(&mut *tx)
        .await?;
    }
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

fn resolved_raw_path_read_candidates(path: &str, fallback_root: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = resolved_raw_path_candidates(path, fallback_root);
    if let Some(alternate_path) = raw_payload_alternate_db_path(path) {
        for candidate in resolved_raw_path_candidates(&alternate_path, fallback_root) {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn raw_payload_alternate_db_path(path: &str) -> Option<String> {
    if path.ends_with(".bin.gz") {
        Some(path.trim_end_matches(".gz").to_string())
    } else if path.ends_with(".bin") {
        Some(format!("{path}.gz"))
    } else {
        None
    }
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
        for candidate in resolved_raw_path_read_candidates(raw_path, raw_path_fallback_root) {
            if candidate.exists() {
                seen.insert(candidate);
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
        for candidate in resolved_raw_path_read_candidates(raw_path, raw_path_fallback_root) {
            if !seen.insert(candidate.clone()) {
                continue;
            }
            match fs::remove_file(&candidate) {
                Ok(_) => {
                    removed += 1;
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    warn!(path = %candidate.display(), error = %err, "failed to remove raw payload file");
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

fn shanghai_local_cutoff_for_age_secs_string(age_secs: u64) -> String {
    format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local()
            - ChronoDuration::seconds(age_secs as i64),
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

async fn schedule_poll(
    state: Arc<AppState>,
    cancel: &CancellationToken,
) -> Result<Option<JoinHandle<()>>> {
    let permit = tokio::select! {
        _ = cancel.cancelled() => return Ok(None),
        permit = state.semaphore.clone().acquire_owned() => {
            permit.context("failed to acquire scheduler permit")?
        }
    };
    if cancel.is_cancelled() {
        return Ok(None);
    }

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

    Ok(Some(handle))
}

async fn spawn_http_server(state: Arc<AppState>) -> Result<(SocketAddr, JoinHandle<()>)> {
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
        .route("/api/invocations/summary", get(fetch_invocation_summary))
        .route(
            "/api/invocations/suggestions",
            get(fetch_invocation_suggestions),
        )
        .route(
            "/api/invocations/new-count",
            get(fetch_invocation_new_records_count),
        )
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
        .route(
            "/api/pool/routing-settings",
            get(get_pool_routing_settings).put(update_pool_routing_settings),
        )
        .route("/api/pool/tags", get(list_tags).post(create_tag))
        .route(
            "/api/pool/tags/:id",
            get(get_tag).patch(update_tag).delete(delete_tag),
        )
        .route("/api/pool/upstream-accounts", get(list_upstream_accounts))
        .route(
            "/api/pool/upstream-account-groups/*groupName",
            put(update_upstream_account_group),
        )
        .route(
            "/api/pool/upstream-accounts/:id/sticky-keys",
            get(get_upstream_account_sticky_keys),
        )
        .route(
            "/api/pool/upstream-accounts/:id",
            get(get_upstream_account)
                .patch(update_upstream_account)
                .delete(delete_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/:id/sync",
            post(sync_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/:id/oauth/relogin",
            post(relogin_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/api-keys",
            post(create_api_key_account),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions",
            post(create_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/mailbox-sessions",
            post(create_oauth_mailbox_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/mailbox-sessions/status",
            post(get_oauth_mailbox_session_status),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/mailbox-sessions/:sessionId",
            delete(delete_oauth_mailbox_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/:loginId",
            get(get_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/:loginId/complete",
            post(complete_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/callback",
            get(oauth_callback),
        )
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

    let shutdown = state.shutdown.clone();
    let handle = tokio::spawn(async move {
        if let Err(err) = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await
        {
            error!(?err, "http server exited with error");
        }
    });

    Ok((addr, handle))
}

fn spawn_shutdown_signal_listener(cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        shutdown_listener().await;
        cancel.cancel();
        info!("shutdown signal received; beginning graceful shutdown");
    })
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

fn codex_invocations_create_sql(table_name: &str) -> String {
    format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table_name} (
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
        table_name = table_name,
    )
}

async fn load_sqlite_table_columns(
    pool: &Pool<Sqlite>,
    table_name: &str,
) -> Result<HashSet<String>> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await
        .with_context(|| format!("failed to inspect {table_name} schema"))?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    Ok(columns)
}

async fn migrate_codex_invocations_drop_raw_expires_at(
    pool: &Pool<Sqlite>,
    existing: &HashSet<String>,
) -> Result<()> {
    const TEMP_TABLE: &str = "codex_invocations_drop_raw_expires_at_new";

    let mut tx = pool.begin().await?;
    let drop_temp_sql = format!("DROP TABLE IF EXISTS {TEMP_TABLE}");
    sqlx::query(&drop_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to clear stale codex_invocations migration temp table")?;
    let create_temp_sql = codex_invocations_create_sql(TEMP_TABLE);
    sqlx::query(&create_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to create codex_invocations migration temp table")?;

    let temp_pragma_sql = format!("PRAGMA table_info('{TEMP_TABLE}')");
    let new_columns: Vec<String> = sqlx::query(&temp_pragma_sql)
        .fetch_all(tx.as_mut())
        .await
        .context("failed to inspect codex_invocations migration temp schema")?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect();
    let copy_columns: Vec<String> = new_columns
        .into_iter()
        .filter(|column| existing.contains(column))
        .collect();
    if copy_columns.is_empty() {
        bail!("codex_invocations migration found no shared columns to copy");
    }

    let copy_columns_csv = copy_columns.join(", ");
    let copy_sql = format!(
        "INSERT INTO {TEMP_TABLE} ({copy_columns_csv}) SELECT {copy_columns_csv} FROM codex_invocations"
    );
    sqlx::query(&copy_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to copy codex_invocations rows into migration temp table")?;
    sqlx::query("DROP TABLE codex_invocations")
        .execute(tx.as_mut())
        .await
        .context("failed to drop legacy codex_invocations table during migration")?;
    let rename_sql = format!("ALTER TABLE {TEMP_TABLE} RENAME TO codex_invocations");
    sqlx::query(&rename_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to swap migrated codex_invocations table into place")?;
    tx.commit().await?;
    Ok(())
}

async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let create_sql = codex_invocations_create_sql("codex_invocations");
    sqlx::query(&create_sql)
        .execute(pool)
        .await
        .context("failed to ensure codex_invocations table existence")?;

    let mut existing = load_sqlite_table_columns(pool, "codex_invocations").await?;
    if existing.contains("raw_expires_at") {
        migrate_codex_invocations_drop_raw_expires_at(pool, &existing).await?;
        existing = load_sqlite_table_columns(pool, "codex_invocations").await?;
    }

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

    sqlx::query("DROP INDEX IF EXISTS idx_codex_invocations_prompt_cache_key_occurred_at")
        .execute(pool)
        .await
        .context("failed to drop stale idx_codex_invocations_prompt_cache_key_occurred_at")?;

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
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_model_occurred_at
        ON codex_invocations (model, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_model_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_failure_kind_occurred_at
        ON codex_invocations (failure_kind, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_failure_kind_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_endpoint_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.endpoint') AS TEXT)) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_endpoint_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_requester_ip_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.requesterIp') AS TEXT)) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_requester_ip_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_upstream_account_occurred_at
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_upstream_account_occurred_at")?;

    // The records analytics page compares trimmed lowercase text for exact-match filters.
    // Mirror those expressions in dedicated indexes so high-volume searches avoid full index scans.
    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_model_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(model, '')))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_model_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_failure_kind_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END,
                failure_kind
            ), '')))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_failure_kind_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_endpoint_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.endpoint') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_endpoint_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_requester_ip_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requesterIp') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_requester_ip_filter_occurred_at")?;

    sqlx::query("DROP INDEX IF EXISTS idx_codex_invocations_prompt_cache_key_filter_occurred_at")
        .execute(pool)
        .await
        .context(
            "failed to drop stale idx_codex_invocations_prompt_cache_key_filter_occurred_at",
        )?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_prompt_cache_key_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.promptCacheKey') AS TEXT) END,
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_prompt_cache_key_filter_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_proxy_filter_occurred_at
        ON codex_invocations (
            (LOWER(TRIM(COALESCE(
                COALESCE(
                    NULLIF(TRIM(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.proxyDisplayName') AS TEXT) END), ''),
                    CASE WHEN TRIM(source) != 'proxy' THEN TRIM(source) END
                ),
                ''
            )))),
            occurred_at
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_proxy_filter_occurred_at")?;

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
            upstream_429_max_retries INTEGER NOT NULL DEFAULT 3,
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

    if let Err(err) = sqlx::query(
        r#"
        ALTER TABLE proxy_model_settings
        ADD COLUMN upstream_429_max_retries INTEGER NOT NULL DEFAULT 3
        "#,
    )
    .execute(pool)
    .await
        && !err.to_string().contains("duplicate column name")
    {
        return Err(err).context("failed to ensure upstream_429_max_retries column");
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
            upstream_429_max_retries,
            enabled_preset_models_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(DEFAULT_PROXY_MODELS_HIJACK_ENABLED as i64)
    .bind(DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED as i64)
    .bind(DEFAULT_PROXY_FAST_MODE_REWRITE_MODE.as_str())
    .bind(i64::from(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES))
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
    ensure_upstream_accounts_schema(pool).await?;

    Ok(())
}

async fn load_proxy_model_settings(pool: &Pool<Sqlite>) -> Result<ProxyModelSettings> {
    let row = sqlx::query_as::<_, ProxyModelSettingsRow>(
        r#"
        SELECT hijack_enabled, merge_upstream_enabled, fast_mode_rewrite_mode, upstream_429_max_retries, enabled_preset_models_json
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
            upstream_429_max_retries = ?4,
            enabled_preset_models_json = ?5,
            updated_at = datetime('now')
        WHERE id = ?6
        "#,
    )
    .bind(settings.hijack_enabled as i64)
    .bind(settings.merge_upstream_enabled as i64)
    .bind(settings.fast_mode_rewrite_mode.as_str())
    .bind(i64::from(settings.upstream_429_max_retries))
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

#[derive(Debug)]
pub(crate) struct ForwardProxyUpstreamResponse {
    pub(crate) selected_proxy: SelectedForwardProxy,
    pub(crate) response: reqwest::Response,
    pub(crate) connect_latency_ms: f64,
    /// `Instant` captured right before sending the upstream request for the final attempt.
    /// Used to record end-to-end latency once streaming finishes.
    pub(crate) attempt_started_at: Instant,
    pub(crate) attempt_recorded: bool,
    pub(crate) attempt_update: Option<ForwardProxyAttemptUpdate>,
}

#[derive(Debug)]
pub(crate) struct ForwardProxyUpstreamError {
    pub(crate) selected_proxy: SelectedForwardProxy,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) failure_kind: &'static str,
    pub(crate) attempt_failure_kind: &'static str,
    pub(crate) connect_latency_ms: f64,
}

#[derive(Debug)]
pub(crate) struct PoolUpstreamResponse {
    pub(crate) account: PoolResolvedAccount,
    pub(crate) response: reqwest::Response,
    pub(crate) connect_latency_ms: f64,
    pub(crate) first_byte_latency_ms: f64,
    pub(crate) first_chunk: Option<Bytes>,
}

#[derive(Debug)]
pub(crate) struct PoolUpstreamError {
    pub(crate) account: Option<PoolResolvedAccount>,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) failure_kind: &'static str,
    pub(crate) connect_latency_ms: f64,
    pub(crate) upstream_error_code: Option<String>,
    pub(crate) upstream_error_message: Option<String>,
    pub(crate) upstream_request_id: Option<String>,
}

#[derive(Debug)]
struct PoolReplayTempFile {
    path: PathBuf,
}

impl Drop for PoolReplayTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone)]
enum PoolReplayBodySnapshot {
    Empty,
    Memory(Bytes),
    File {
        temp_file: Arc<PoolReplayTempFile>,
        size: usize,
    },
}

#[derive(Debug, Clone)]
enum PoolReplayBodyStatus {
    Reading,
    Complete(PoolReplayBodySnapshot),
    ReadError(RequestBodyReadError),
    InternalError(String),
    Incomplete,
}

struct PoolReplayBodyBuffer {
    proxy_request_id: u64,
    len: usize,
    memory: Vec<u8>,
    file: Option<(Arc<PoolReplayTempFile>, tokio::fs::File)>,
}

struct PoolReplayableRequestBody {
    body: reqwest::Body,
    status_rx: watch::Receiver<PoolReplayBodyStatus>,
    cancel: CancellationToken,
}

fn proxy_forward_response_status_is_success(status: StatusCode, stream_error: bool) -> bool {
    !stream_error && status != StatusCode::TOO_MANY_REQUESTS && !status.is_server_error()
}

fn proxy_capture_response_status_is_success(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
) -> bool {
    !logical_stream_failure && proxy_forward_response_status_is_success(status, stream_error)
}

fn proxy_forward_response_failure_kind(
    status: StatusCode,
    stream_error: bool,
) -> Option<&'static str> {
    if stream_error {
        Some(FORWARD_PROXY_FAILURE_STREAM_ERROR)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    } else if status.is_server_error() {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    } else {
        None
    }
}

fn proxy_capture_response_failure_kind(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
) -> Option<&'static str> {
    if logical_stream_failure {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    } else {
        proxy_forward_response_failure_kind(status, stream_error)
    }
}

fn fallback_proxy_429_retry_delay(retry_index: u32) -> Duration {
    let exponent = retry_index.saturating_sub(1).min(16);
    let multiplier = 1_u64 << exponent;
    Duration::from_millis(500_u64.saturating_mul(multiplier)).min(Duration::from_secs(5))
}

const POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS: u8 = 3;

impl PoolReplayBodyBuffer {
    fn new(proxy_request_id: u64) -> Self {
        Self {
            proxy_request_id,
            len: 0,
            memory: Vec::new(),
            file: None,
        }
    }

    async fn append(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.len = self.len.saturating_add(chunk.len());
        if let Some((_, file)) = self.file.as_mut() {
            file.write_all(chunk).await?;
            return Ok(());
        }

        if self.memory.len().saturating_add(chunk.len())
            <= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES
        {
            self.memory.extend_from_slice(chunk);
            return Ok(());
        }

        let temp_file = Arc::new(PoolReplayTempFile {
            path: build_pool_replay_temp_path(self.proxy_request_id),
        });
        let mut file = tokio::fs::File::create(&temp_file.path).await?;
        if !self.memory.is_empty() {
            file.write_all(&self.memory).await?;
            self.memory.clear();
        }
        file.write_all(chunk).await?;
        self.file = Some((temp_file, file));
        Ok(())
    }

    async fn finish(mut self) -> io::Result<PoolReplayBodySnapshot> {
        if let Some((temp_file, mut file)) = self.file.take() {
            file.flush().await?;
            return Ok(PoolReplayBodySnapshot::File {
                temp_file,
                size: self.len,
            });
        }

        if self.memory.is_empty() {
            Ok(PoolReplayBodySnapshot::Empty)
        } else {
            Ok(PoolReplayBodySnapshot::Memory(Bytes::from(self.memory)))
        }
    }
}

impl PoolReplayBodySnapshot {
    fn to_reqwest_body(&self) -> reqwest::Body {
        match self {
            Self::Empty => reqwest::Body::from(Bytes::new()),
            Self::Memory(bytes) => reqwest::Body::from(bytes.clone()),
            Self::File { temp_file, size } => {
                let path = temp_file.path.clone();
                let expected_size = *size;
                let stream = stream::unfold(
                    Some((path, expected_size, None::<tokio::fs::File>)),
                    |state| async move {
                        let Some((path, remaining, file)) = state else {
                            return None;
                        };
                        if remaining == 0 {
                            return None;
                        }
                        let mut file = match file {
                            Some(file) => file,
                            None => match tokio::fs::File::open(&path).await {
                                Ok(file) => file,
                                Err(err) => {
                                    return Some((Err(io::Error::other(err.to_string())), None));
                                }
                            },
                        };
                        let mut buf = vec![0_u8; remaining.min(64 * 1024)];
                        match file.read(&mut buf).await {
                            Ok(0) => None,
                            Ok(read_len) => {
                                buf.truncate(read_len);
                                Some((
                                    Ok(Bytes::from(buf)),
                                    Some((path, remaining - read_len, Some(file))),
                                ))
                            }
                            Err(err) => Some((Err(io::Error::other(err.to_string())), None)),
                        }
                    },
                );
                reqwest::Body::wrap_stream(stream)
            }
        }
    }
}

fn build_pool_replay_temp_path(proxy_request_id: u64) -> PathBuf {
    let mut path = env::temp_dir();
    path.push(format!(
        "cvm-pool-replay-{proxy_request_id}-{}.bin",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    path
}

fn spawn_pool_replayable_request_body(
    body: Body,
    body_limit: usize,
    _request_read_timeout: Duration,
    proxy_request_id: u64,
) -> PoolReplayableRequestBody {
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
    let (status_tx, status_rx) = watch::channel(PoolReplayBodyStatus::Reading);
    let cancel = CancellationToken::new();
    let cancel_for_task = cancel.clone();

    tokio::spawn(async move {
        let mut buffer = PoolReplayBodyBuffer::new(proxy_request_id);
        let mut data_len = 0usize;
        let mut stream = body.into_data_stream();

        loop {
            if cancel_for_task.is_cancelled() {
                let _ = status_tx.send(PoolReplayBodyStatus::Incomplete);
                return;
            }

            let next_chunk = tokio::select! {
                _ = cancel_for_task.cancelled() => {
                    let _ = status_tx.send(PoolReplayBodyStatus::Incomplete);
                    return;
                }
                chunk = stream.next() => chunk,
            };

            let Some(chunk) = next_chunk else {
                match buffer.finish().await {
                    Ok(snapshot) => {
                        let _ = status_tx.send(PoolReplayBodyStatus::Complete(snapshot));
                    }
                    Err(err) => {
                        let _ = status_tx.send(PoolReplayBodyStatus::InternalError(format!(
                            "failed to finalize replay body cache: {err}"
                        )));
                    }
                }
                return;
            };

            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(err) => {
                    let msg = format!("failed to read request body stream: {err}");
                    let _ = tx.send(Err(io::Error::other(msg.clone()))).await;
                    let _ = status_tx.send(PoolReplayBodyStatus::ReadError(RequestBodyReadError {
                        status: StatusCode::BAD_REQUEST,
                        message: msg,
                        failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                        partial_body: Vec::new(),
                    }));
                    return;
                }
            };

            if data_len.saturating_add(chunk.len()) > body_limit {
                let _ = tx
                    .send(Err(io::Error::other("request body exceeds limit")))
                    .await;
                let _ = status_tx.send(PoolReplayBodyStatus::ReadError(RequestBodyReadError {
                    status: StatusCode::PAYLOAD_TOO_LARGE,
                    message: format!("request body exceeds {body_limit} bytes"),
                    failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                    partial_body: Vec::new(),
                }));
                return;
            }
            data_len = data_len.saturating_add(chunk.len());

            if let Err(err) = buffer.append(&chunk).await {
                let msg = format!("failed to cache replayable request body: {err}");
                let _ = tx.send(Err(io::Error::other(msg.clone()))).await;
                let _ = status_tx.send(PoolReplayBodyStatus::InternalError(msg));
                return;
            }

            if tx.send(Ok(chunk)).await.is_err() {
                let _ = status_tx.send(PoolReplayBodyStatus::Incomplete);
                return;
            }
        }
    });

    PoolReplayableRequestBody {
        body: reqwest::Body::wrap_stream(ReceiverStream::new(rx)),
        status_rx,
        cancel,
    }
}

fn parse_retry_after_delay(value: &HeaderValue) -> Option<Duration> {
    let text = value.to_str().ok()?.trim();
    if text.is_empty() {
        return None;
    }

    if let Ok(seconds) = text.parse::<u64>() {
        return Some(Duration::from_secs(seconds).min(Duration::from_secs(
            MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS,
        )));
    }

    let retry_at = httpdate::parse_http_date(text).ok()?;
    let delay = retry_at.duration_since(std::time::SystemTime::now()).ok()?;
    Some(delay.min(Duration::from_secs(
        MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS,
    )))
}

async fn send_pool_request_with_failover(
    state: Arc<AppState>,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: Option<PoolReplayBodySnapshot>,
    handshake_timeout: Duration,
    sticky_key: Option<&str>,
    preferred_account: Option<PoolResolvedAccount>,
    same_account_attempts: u8,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    let request_connection_scoped = connection_scoped_header_names(headers);
    let mut excluded_ids = Vec::new();
    let mut last_error: Option<PoolUpstreamError> = None;
    let mut preferred_account = preferred_account;
    let mut same_account_attempts = same_account_attempts.max(1);

    'account_loop: loop {
        let mut account = if let Some(account) = preferred_account.take() {
            account
        } else {
            match resolve_pool_account_for_request(state.as_ref(), sticky_key, &excluded_ids).await
            {
                Ok(PoolAccountResolution::Resolved(account)) => account,
                Ok(PoolAccountResolution::NoCandidate) => {
                    return Err(last_error.unwrap_or(PoolUpstreamError {
                        account: None,
                        status: StatusCode::BAD_GATEWAY,
                        message: "no healthy pool account is available".to_string(),
                        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                    }));
                }
                Ok(PoolAccountResolution::BlockedByPolicy(message)) => {
                    return Err(PoolUpstreamError {
                        account: None,
                        status: StatusCode::BAD_GATEWAY,
                        message,
                        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                    });
                }
                Err(err) => {
                    return Err(PoolUpstreamError {
                        account: None,
                        status: StatusCode::BAD_GATEWAY,
                        message: format!("failed to resolve pool account: {err}"),
                        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                    });
                }
            }
        };

        excluded_ids.push(account.account_id);
        let mut target_url =
            match build_proxy_upstream_url(&account.upstream_base_url, original_uri) {
                Ok(url) => url,
                Err(err) => {
                    return Err(PoolUpstreamError {
                        account: Some(account),
                        status: StatusCode::BAD_GATEWAY,
                        message: format!("failed to build pool upstream url: {err}"),
                        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                    });
                }
            };
        let client = match state.http_clients.client_for_parallelism(false) {
            Ok(client) => client,
            Err(err) => {
                return Err(PoolUpstreamError {
                    account: Some(account),
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to initialize upstream client: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    connect_latency_ms: 0.0,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    upstream_request_id: None,
                });
            }
        };

        for same_account_attempt in 0..same_account_attempts {
            let mut request = client.request(method.clone(), target_url.clone());
            for (name, value) in headers {
                if *name == header::AUTHORIZATION {
                    continue;
                }
                if should_forward_proxy_header(name, &request_connection_scoped) {
                    request = request.header(name, value);
                }
            }
            request = request.header(header::AUTHORIZATION, account.authorization.clone());
            if let Some(body_snapshot) = body.as_ref() {
                request = request.body(body_snapshot.to_reqwest_body());
            }

            let connect_started = Instant::now();
            let response = match timeout(handshake_timeout, request.send()).await {
                Ok(Ok(response)) => response,
                Ok(Err(err)) => {
                    let message = format!("failed to contact upstream: {err}");
                    let has_retry_budget = same_account_attempt + 1 < same_account_attempts;
                    if has_retry_budget {
                        let retry_delay =
                            fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1);
                        info!(
                            account_id = account.account_id,
                            retry_index = same_account_attempt + 1,
                            max_same_account_attempts = same_account_attempts,
                            retry_after_ms = retry_delay.as_millis(),
                            "pool upstream transport failure; retrying same account"
                        );
                        sleep(retry_delay).await;
                        continue;
                    }
                    if let Err(route_err) = record_pool_route_transport_failure(
                        &state.pool,
                        account.account_id,
                        sticky_key,
                        &message,
                    )
                    .await
                    {
                        warn!(account_id = account.account_id, error = %route_err, "failed to record pool transport failure");
                    }
                    last_error = Some(PoolUpstreamError {
                        account: Some(account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: message.clone(),
                        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                        connect_latency_ms: elapsed_ms(connect_started),
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                    });
                    if excluded_ids.len() >= 64 {
                        return Err(last_error.expect("pool transport failure should be recorded"));
                    }
                    continue 'account_loop;
                }
                Err(_) => {
                    let message = format!(
                        "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
                        handshake_timeout.as_millis()
                    );
                    let has_retry_budget = same_account_attempt + 1 < same_account_attempts;
                    if has_retry_budget {
                        let retry_delay =
                            fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1);
                        info!(
                            account_id = account.account_id,
                            retry_index = same_account_attempt + 1,
                            max_same_account_attempts = same_account_attempts,
                            retry_after_ms = retry_delay.as_millis(),
                            "pool upstream handshake timeout; retrying same account"
                        );
                        sleep(retry_delay).await;
                        continue;
                    }
                    if let Err(route_err) = record_pool_route_transport_failure(
                        &state.pool,
                        account.account_id,
                        sticky_key,
                        &message,
                    )
                    .await
                    {
                        warn!(account_id = account.account_id, error = %route_err, "failed to record pool handshake timeout");
                    }
                    last_error = Some(PoolUpstreamError {
                        account: Some(account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: message.clone(),
                        failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                        connect_latency_ms: elapsed_ms(connect_started),
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                    });
                    if excluded_ids.len() >= 64 {
                        return Err(last_error.expect("pool handshake failure should be recorded"));
                    }
                    continue 'account_loop;
                }
            };

            let connect_latency_ms = elapsed_ms(connect_started);
            let status = response.status();
            if status == StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
                || matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
            {
                let has_retry_budget = same_account_attempt + 1 < same_account_attempts;
                let should_retry_same_account = has_retry_budget
                    && (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error());
                if should_retry_same_account {
                    let retry_delay = response
                        .headers()
                        .get(header::RETRY_AFTER)
                        .and_then(parse_retry_after_delay)
                        .unwrap_or_else(|| {
                            fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1)
                        });
                    info!(
                        account_id = account.account_id,
                        status = status.as_u16(),
                        retry_index = same_account_attempt + 1,
                        max_same_account_attempts = same_account_attempts,
                        retry_after_ms = retry_delay.as_millis(),
                        "pool upstream responded with retryable status; retrying same account"
                    );
                    sleep(retry_delay).await;
                    continue;
                }
                let upstream_request_id_header = response
                    .headers()
                    .get("x-request-id")
                    .and_then(|value| value.to_str().ok())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());
                let (upstream_error_code, upstream_error_message, upstream_request_id, message) =
                    match response.bytes().await {
                        Ok(body_bytes) => summarize_pool_upstream_http_failure(
                            status,
                            upstream_request_id_header.as_deref(),
                            &body_bytes,
                        ),
                        Err(err) => (
                            None,
                            None,
                            upstream_request_id_header,
                            format!(
                                "pool upstream responded with {} (failed to read error body: {err})",
                                status.as_u16()
                            ),
                        ),
                    };
                if has_retry_budget
                    && should_retry_same_account_after_bridge_token_rejection(
                        &account.kind,
                        status,
                        upstream_error_code.as_deref(),
                        upstream_error_message.as_deref(),
                        &message,
                    )
                {
                    clear_cached_oauth_bridge_token(state.as_ref(), account.account_id).await;
                    match refresh_pool_account_for_retry(state.as_ref(), account.account_id).await {
                        Ok(Some(refreshed_account)) => {
                            account = refreshed_account;
                            target_url = match build_proxy_upstream_url(
                                &account.upstream_base_url,
                                original_uri,
                            ) {
                                Ok(url) => url,
                                Err(err) => {
                                    warn!(
                                        account_id = account.account_id,
                                        error = %err,
                                        "failed to rebuild pool upstream url after oauth bridge token refresh"
                                    );
                                    continue 'account_loop;
                                }
                            };
                        }
                        Ok(None) => {
                            continue 'account_loop;
                        }
                        Err(err) => {
                            warn!(
                                account_id = account.account_id,
                                error = %err,
                                "failed to refresh oauth bridge registration after token rejection"
                            );
                            continue 'account_loop;
                        }
                    }
                    info!(
                        account_id = account.account_id,
                        status = status.as_u16(),
                        retry_index = same_account_attempt + 1,
                        max_same_account_attempts = POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                        "pool oauth bridge token was rejected; clearing cached bridge token and retrying same account"
                    );
                    continue;
                }
                let route_error_message = upstream_error_code
                    .as_deref()
                    .map_or_else(|| message.clone(), |code| format!("{code}: {message}"));
                if let Err(route_err) = record_pool_route_http_failure(
                    &state.pool,
                    account.account_id,
                    &account.kind,
                    sticky_key,
                    status,
                    &route_error_message,
                )
                .await
                {
                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool upstream http failure");
                }
                let failure_kind = if status == StatusCode::TOO_MANY_REQUESTS {
                    FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                } else if status.is_server_error() {
                    FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX
                } else {
                    PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT
                };
                last_error = Some(PoolUpstreamError {
                    account: Some(account.clone()),
                    status,
                    message: message.clone(),
                    failure_kind,
                    connect_latency_ms,
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                });
                if excluded_ids.len() >= 64 {
                    return Err(last_error.expect("pool http failure should be recorded"));
                }
                continue 'account_loop;
            }

            let mut response = response;
            let first_byte_started = Instant::now();
            let first_chunk = match response.chunk().await {
                Ok(chunk) => chunk,
                Err(err) => {
                    let message = format!("upstream stream error before first chunk: {err}");
                    let has_retry_budget = same_account_attempt + 1 < same_account_attempts;
                    if has_retry_budget {
                        let retry_delay =
                            fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1);
                        info!(
                            account_id = account.account_id,
                            retry_index = same_account_attempt + 1,
                            max_same_account_attempts = same_account_attempts,
                            retry_after_ms = retry_delay.as_millis(),
                            "pool upstream first chunk failed; retrying same account"
                        );
                        sleep(retry_delay).await;
                        continue;
                    }
                    if let Err(route_err) = record_pool_route_transport_failure(
                        &state.pool,
                        account.account_id,
                        sticky_key,
                        &message,
                    )
                    .await
                    {
                        warn!(account_id = account.account_id, error = %route_err, "failed to record pool first chunk failure");
                    }
                    last_error = Some(PoolUpstreamError {
                        account: Some(account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: message.clone(),
                        failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                        connect_latency_ms,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                    });
                    if excluded_ids.len() >= 64 {
                        return Err(
                            last_error.expect("pool first chunk failure should be recorded")
                        );
                    }
                    continue 'account_loop;
                }
            };

            return Ok(PoolUpstreamResponse {
                account,
                response,
                connect_latency_ms,
                first_byte_latency_ms: elapsed_ms(first_byte_started),
                first_chunk,
            });
        }

        same_account_attempts = POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS;
    }
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let authorization = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let normalized = token.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

async fn request_matches_pool_route(state: &AppState, headers: &HeaderMap) -> Result<bool> {
    let Some(api_key) = extract_bearer_token(headers) else {
        return Ok(false);
    };
    pool_api_key_matches(state, &api_key).await
}

async fn extract_sticky_key_from_replay_snapshot(
    snapshot: &PoolReplayBodySnapshot,
) -> Option<String> {
    let bytes = match snapshot {
        PoolReplayBodySnapshot::Empty => return None,
        PoolReplayBodySnapshot::Memory(bytes) => bytes.to_vec(),
        PoolReplayBodySnapshot::File { temp_file, .. } => {
            tokio::fs::read(&temp_file.path).await.ok()?
        }
    };

    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| extract_sticky_key_from_request_body(&value))
}

async fn continue_or_retry_pool_live_request(
    state: Arc<AppState>,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    handshake_timeout: Duration,
    initial_account: PoolResolvedAccount,
    sticky_key: Option<String>,
    replay_status_rx: &watch::Receiver<PoolReplayBodyStatus>,
    replay_cancel: &CancellationToken,
    first_error: PoolUpstreamError,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    let replay_status = { replay_status_rx.borrow().clone() };
    match replay_status {
        PoolReplayBodyStatus::Complete(snapshot) => {
            let replay_sticky_key = extract_sticky_key_from_replay_snapshot(&snapshot)
                .await
                .or(sticky_key);
            send_pool_request_with_failover(
                state,
                method,
                original_uri,
                headers,
                Some(snapshot),
                handshake_timeout,
                replay_sticky_key.as_deref(),
                Some(initial_account),
                POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS.saturating_sub(1),
            )
            .await
        }
        PoolReplayBodyStatus::ReadError(err) => Err(PoolUpstreamError {
            account: Some(initial_account),
            status: err.status,
            message: err.message,
            failure_kind: err.failure_kind,
            connect_latency_ms: first_error.connect_latency_ms,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
        }),
        PoolReplayBodyStatus::InternalError(message) => Err(PoolUpstreamError {
            account: Some(initial_account),
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
            failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            connect_latency_ms: first_error.connect_latency_ms,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
        }),
        PoolReplayBodyStatus::Reading | PoolReplayBodyStatus::Incomplete => {
            replay_cancel.cancel();
            Err(first_error)
        }
    }
}

async fn proxy_openai_v1_via_pool(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, (StatusCode, String)> {
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let handshake_timeout = state.config.proxy_upstream_handshake_timeout(None);
    let header_sticky_key = extract_sticky_key_from_headers(&headers);
    let body_size_hint_exact = body
        .size_hint()
        .exact()
        .and_then(|value| usize::try_from(value).ok());
    let (upstream, sticky_key) = if request_may_have_body(&method, &headers) {
        let should_prebuffer_for_body_sticky = header_sticky_key.is_none()
            && headers
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"))
            && headers
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                .or(body_size_hint_exact)
                .is_some_and(|value| value <= body_limit);

        if should_prebuffer_for_body_sticky {
            let request_body_bytes = read_request_body_with_limit(
                body,
                body_limit,
                state.config.openai_proxy_request_read_timeout,
                proxy_request_id,
            )
            .await
            .map_err(|err| (err.status, err.message))?;
            let request_body_bytes = Bytes::from(request_body_bytes);
            let body_sticky_key = serde_json::from_slice::<Value>(&request_body_bytes)
                .ok()
                .and_then(|value| extract_sticky_key_from_request_body(&value));
            (
                send_pool_request_with_failover(
                    state.clone(),
                    method,
                    original_uri,
                    &headers,
                    Some(PoolReplayBodySnapshot::Memory(request_body_bytes)),
                    handshake_timeout,
                    body_sticky_key.as_deref(),
                    None,
                    POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                )
                .await
                .map_err(|err| (err.status, err.message))?,
                body_sticky_key,
            )
        } else {
            let sticky_key = header_sticky_key;
            let initial_account =
                match resolve_pool_account_for_request(state.as_ref(), sticky_key.as_deref(), &[])
                    .await
                {
                    Ok(PoolAccountResolution::Resolved(account)) => account,
                    Ok(PoolAccountResolution::NoCandidate) => {
                        return Err((
                            StatusCode::BAD_GATEWAY,
                            "no healthy pool account is available".to_string(),
                        ));
                    }
                    Ok(PoolAccountResolution::BlockedByPolicy(message)) => {
                        return Err((StatusCode::BAD_GATEWAY, message));
                    }
                    Err(err) => {
                        return Err((
                            StatusCode::BAD_GATEWAY,
                            format!("failed to resolve pool account: {err}"),
                        ));
                    }
                };
            let target_url =
                build_proxy_upstream_url(&initial_account.upstream_base_url, original_uri)
                    .map_err(|err| {
                        (
                            StatusCode::BAD_GATEWAY,
                            format!("failed to build pool upstream url: {err}"),
                        )
                    })?;
            let client = state
                .http_clients
                .client_for_parallelism(false)
                .map_err(|err| {
                    (
                        StatusCode::BAD_GATEWAY,
                        format!("failed to initialize upstream client: {err}"),
                    )
                })?;
            let request_connection_scoped = connection_scoped_header_names(&headers);
            let replayable = spawn_pool_replayable_request_body(
                body,
                body_limit,
                state.config.openai_proxy_request_read_timeout,
                proxy_request_id,
            );
            let replay_status_rx = replayable.status_rx.clone();
            let replay_cancel = replayable.cancel.clone();
            let mut request = client
                .request(method.clone(), target_url)
                .body(replayable.body);
            for (name, value) in &headers {
                if *name == header::AUTHORIZATION {
                    continue;
                }
                if should_forward_proxy_header(name, &request_connection_scoped) {
                    request = request.header(name, value);
                }
            }
            request = request.header(header::AUTHORIZATION, initial_account.authorization.clone());

            let connect_started = Instant::now();
            let upstream = match timeout(handshake_timeout, request.send()).await {
                Ok(Ok(mut response)) => {
                    let connect_latency_ms = elapsed_ms(connect_started);
                    let status = response.status();
                    if status == StatusCode::TOO_MANY_REQUESTS
                        || status.is_server_error()
                        || matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
                    {
                        let upstream_request_id_header = response
                            .headers()
                            .get("x-request-id")
                            .and_then(|value| value.to_str().ok())
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(|value| value.to_string());
                        let (
                            upstream_error_code,
                            upstream_error_message,
                            upstream_request_id,
                            message,
                        ) = match response.bytes().await {
                            Ok(body_bytes) => summarize_pool_upstream_http_failure(
                                status,
                                upstream_request_id_header.as_deref(),
                                &body_bytes,
                            ),
                            Err(err) => (
                                None,
                                None,
                                upstream_request_id_header,
                                format!(
                                    "pool upstream responded with {} (failed to read error body: {err})",
                                    status.as_u16()
                                ),
                            ),
                        };
                        let failure_kind = if status == StatusCode::TOO_MANY_REQUESTS {
                            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                        } else if status.is_server_error() {
                            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX
                        } else {
                            PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT
                        };
                        let first_error = PoolUpstreamError {
                            account: Some(initial_account.clone()),
                            status,
                            message,
                            failure_kind,
                            connect_latency_ms,
                            upstream_error_code,
                            upstream_error_message,
                            upstream_request_id,
                        };
                        continue_or_retry_pool_live_request(
                            state.clone(),
                            method,
                            original_uri,
                            &headers,
                            handshake_timeout,
                            initial_account,
                            sticky_key.clone(),
                            &replay_status_rx,
                            &replay_cancel,
                            first_error,
                        )
                        .await
                        .map_err(|err| (err.status, err.message))?
                    } else {
                        let first_byte_started = Instant::now();
                        match response.chunk().await {
                            Ok(first_chunk) => PoolUpstreamResponse {
                                account: initial_account,
                                response,
                                connect_latency_ms,
                                first_byte_latency_ms: elapsed_ms(first_byte_started),
                                first_chunk,
                            },
                            Err(err) => {
                                let first_error = PoolUpstreamError {
                                    account: Some(initial_account.clone()),
                                    status: StatusCode::BAD_GATEWAY,
                                    message: format!(
                                        "upstream stream error before first chunk: {err}"
                                    ),
                                    failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                                    connect_latency_ms,
                                    upstream_error_code: None,
                                    upstream_error_message: None,
                                    upstream_request_id: None,
                                };
                                continue_or_retry_pool_live_request(
                                    state.clone(),
                                    method,
                                    original_uri,
                                    &headers,
                                    handshake_timeout,
                                    initial_account,
                                    sticky_key.clone(),
                                    &replay_status_rx,
                                    &replay_cancel,
                                    first_error,
                                )
                                .await
                                .map_err(|err| (err.status, err.message))?
                            }
                        }
                    }
                }
                Ok(Err(err)) => {
                    let first_error = PoolUpstreamError {
                        account: Some(initial_account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: format!("failed to contact upstream: {err}"),
                        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                        connect_latency_ms: elapsed_ms(connect_started),
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                    };
                    continue_or_retry_pool_live_request(
                        state.clone(),
                        method,
                        original_uri,
                        &headers,
                        handshake_timeout,
                        initial_account,
                        sticky_key.clone(),
                        &replay_status_rx,
                        &replay_cancel,
                        first_error,
                    )
                    .await
                    .map_err(|err| (err.status, err.message))?
                }
                Err(_) => {
                    let first_error = PoolUpstreamError {
                        account: Some(initial_account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message: format!(
                            "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
                            handshake_timeout.as_millis()
                        ),
                        failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                        connect_latency_ms: elapsed_ms(connect_started),
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                    };
                    continue_or_retry_pool_live_request(
                        state.clone(),
                        method,
                        original_uri,
                        &headers,
                        handshake_timeout,
                        initial_account,
                        sticky_key.clone(),
                        &replay_status_rx,
                        &replay_cancel,
                        first_error,
                    )
                    .await
                    .map_err(|err| (err.status, err.message))?
                }
            };
            (upstream, sticky_key)
        }
    } else {
        (
            send_pool_request_with_failover(
                state.clone(),
                method,
                original_uri,
                &headers,
                None,
                handshake_timeout,
                header_sticky_key.as_deref(),
                None,
                POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
            )
            .await
            .map_err(|err| (err.status, err.message))?,
            header_sticky_key,
        )
    };

    let account = upstream.account;
    let t_upstream_connect_ms = upstream.connect_latency_ms;
    let t_upstream_ttfb_ms = upstream.first_byte_latency_ms;
    let upstream_response = upstream.response;
    let rewritten_location = normalize_proxy_location_header(
        upstream_response.status(),
        upstream_response.headers(),
        &account.upstream_base_url,
    )
    .map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to process upstream redirect: {err}"),
        )
    })?;

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
    let first_chunk = upstream.first_chunk;
    if let Some(chunk) = first_chunk.as_ref() {
        info!(
            proxy_request_id,
            account_id = account.account_id,
            ttfb_ms = t_upstream_ttfb_ms,
            first_chunk_bytes = chunk.len(),
            "pool upstream response first chunk ready"
        );
    } else {
        if let Err(route_err) =
            record_pool_route_success(&state.pool, account.account_id, sticky_key.as_deref()).await
        {
            warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
        }
        return response_builder.body(Body::empty()).map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build proxy response: {err}"),
            )
        });
    }

    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
    let state_for_record = state.clone();
    let sticky_key_for_record = sticky_key.clone();
    tokio::spawn(async move {
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;
        let stream_started_at = Instant::now();
        let mut stream_error_message: Option<String> = None;
        let mut downstream_closed = false;

        if let Some(chunk) = first_chunk {
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            if tx.send(Ok(chunk)).await.is_err() {
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
                        break;
                    }
                }
                Err(err) => {
                    let message = format!("upstream stream error: {err}");
                    stream_error_message = Some(message.clone());
                    let _ = tx.send(Err(io::Error::other(message))).await;
                    break;
                }
            }
        }

        if let Some(message) = stream_error_message.as_deref() {
            if let Err(route_err) = record_pool_route_transport_failure(
                &state_for_record.pool,
                account.account_id,
                sticky_key_for_record.as_deref(),
                message,
            )
            .await
            {
                warn!(account_id = account.account_id, error = %route_err, "failed to record pool stream error");
            }
        } else if let Err(route_err) = record_pool_route_success(
            &state_for_record.pool,
            account.account_id,
            sticky_key_for_record.as_deref(),
        )
        .await
        {
            warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
        }

        info!(
            proxy_request_id,
            account_id = account.account_id,
            t_upstream_connect_ms,
            forwarded_chunks,
            forwarded_bytes,
            elapsed_ms = stream_started_at.elapsed().as_millis(),
            "pool upstream response stream completed"
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

pub(crate) async fn send_forward_proxy_request_with_429_retry(
    state: Arc<AppState>,
    method: Method,
    target_url: Url,
    headers: &HeaderMap,
    body: Option<Bytes>,
    handshake_timeout: Duration,
    upstream_429_max_retries: u8,
) -> Result<ForwardProxyUpstreamResponse, ForwardProxyUpstreamError> {
    let request_connection_scoped = connection_scoped_header_names(headers);

    for attempt in 0..=upstream_429_max_retries {
        let selected_proxy = select_forward_proxy_for_request(state.as_ref()).await;
        let client = match state
            .http_clients
            .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        {
            Ok(client) => client,
            Err(err) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to initialize forward proxy client: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: 0.0,
                });
            }
        };

        let mut request = client.request(method.clone(), target_url.clone());
        for (name, value) in headers {
            if should_forward_proxy_header(name, &request_connection_scoped) {
                request = request.header(name, value);
            }
        }
        if let Some(body_bytes) = body.clone() {
            request = request.body(body_bytes);
        }

        let connect_started = Instant::now();
        let response = match timeout(handshake_timeout, request.send()).await {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to contact upstream: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: elapsed_ms(connect_started),
                });
            }
            Err(_) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy,
                    status: StatusCode::BAD_GATEWAY,
                    message: format!(
                        "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
                        handshake_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT,
                    connect_latency_ms: elapsed_ms(connect_started),
                });
            }
        };

        let connect_latency_ms = elapsed_ms(connect_started);
        if response.status() != StatusCode::TOO_MANY_REQUESTS {
            return Ok(ForwardProxyUpstreamResponse {
                selected_proxy,
                response,
                connect_latency_ms,
                attempt_started_at: connect_started,
                attempt_recorded: false,
                attempt_update: None,
            });
        }

        if attempt < upstream_429_max_retries {
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy.clone(),
                false,
                Some(connect_latency_ms),
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429),
                false,
            )
            .await;

            let retry_delay = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(parse_retry_after_delay)
                .unwrap_or_else(|| fallback_proxy_429_retry_delay(u32::from(attempt) + 1));
            info!(
                proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
                proxy_source = selected_proxy.source,
                proxy_label = selected_proxy.display_name,
                proxy_url_ref = %forward_proxy_log_ref_option(selected_proxy.endpoint_url_raw.as_deref()),
                retry_index = attempt + 1,
                max_429_retries = upstream_429_max_retries,
                retry_after_ms = retry_delay.as_millis(),
                "upstream responded 429; retrying forward proxy request"
            );
            sleep(retry_delay).await;
            continue;
        }

        // Final 429: defer attempt recording until the caller finishes consuming / forwarding
        // the response body, so a later stream error can override this classification.
        return Ok(ForwardProxyUpstreamResponse {
            selected_proxy,
            response,
            connect_latency_ms,
            attempt_started_at: connect_started,
            attempt_recorded: false,
            attempt_update: None,
        });
    }

    unreachable!("429 retry loop should always return a response or error")
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

    let pool_route_active = request_matches_pool_route(state.as_ref(), &headers)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to resolve pool routing settings: {err}"),
            )
        })?;

    if method == Method::GET && is_models_list_path(original_uri.path()) {
        if pool_route_active {
            return proxy_openai_v1_via_pool(
                state,
                proxy_request_id,
                &original_uri,
                method,
                headers,
                body,
            )
            .await;
        }
        let settings = state.proxy_model_settings.read().await.clone();
        if settings.hijack_enabled {
            let mut payload = build_preset_models_payload(&settings.enabled_preset_models);
            let mut merge_status: Option<&'static str> = None;
            if settings.merge_upstream_enabled {
                match fetch_upstream_models_payload(
                    state.clone(),
                    target_url.clone(),
                    &headers,
                    settings.upstream_429_max_retries,
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

    if let Some(target) = capture_target_for_request(original_uri.path(), &method) {
        return proxy_openai_v1_capture_target(
            state,
            proxy_request_id,
            &original_uri,
            headers,
            body,
            target,
            target_url,
            peer_ip,
        )
        .await;
    }

    if pool_route_active {
        return proxy_openai_v1_via_pool(
            state,
            proxy_request_id,
            &original_uri,
            method,
            headers,
            body,
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

    let handshake_timeout = state.config.proxy_upstream_handshake_timeout(None);
    let upstream_429_max_retries = state
        .proxy_model_settings
        .read()
        .await
        .upstream_429_max_retries;
    let upstream = if upstream_429_max_retries == 0 {
        let selected_proxy = select_forward_proxy_for_request(state.as_ref()).await;
        let proxy_client = match state
            .http_clients
            .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        {
            Ok(client) => client,
            Err(err) => {
                record_forward_proxy_attempt(
                    state.clone(),
                    selected_proxy.clone(),
                    false,
                    Some(0.0),
                    Some(FORWARD_PROXY_FAILURE_SEND_ERROR),
                    false,
                )
                .await;
                return Err((
                    StatusCode::BAD_GATEWAY,
                    format!("failed to initialize forward proxy client: {err}"),
                ));
            }
        };

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

        Ok(ForwardProxyUpstreamResponse {
            selected_proxy,
            response: upstream_response,
            connect_latency_ms: elapsed_ms(connect_started),
            attempt_started_at: connect_started,
            attempt_recorded: false,
            attempt_update: None,
        })
    } else {
        let request_body_bytes = read_request_body_with_limit(
            body,
            body_limit,
            state.config.openai_proxy_request_read_timeout,
            proxy_request_id,
        )
        .await
        .map_err(|err| (err.status, err.message))?;
        let request_body_bytes = Bytes::from(request_body_bytes);
        match send_forward_proxy_request_with_429_retry(
            state.clone(),
            method,
            target_url,
            &headers,
            Some(request_body_bytes),
            handshake_timeout,
            upstream_429_max_retries,
        )
        .await
        {
            Ok(response) => Ok(response),
            Err(err) => {
                record_forward_proxy_attempt(
                    state.clone(),
                    err.selected_proxy,
                    false,
                    Some(err.connect_latency_ms),
                    Some(err.attempt_failure_kind),
                    false,
                )
                .await;
                Err((err.status, err.message))
            }
        }
    }?;
    let upstream_attempt_started_at = upstream.attempt_started_at;
    let selected_proxy = upstream.selected_proxy;
    let t_upstream_connect_ms = upstream.connect_latency_ms;
    let attempt_already_recorded = upstream.attempt_recorded;
    let upstream_response = upstream.response;

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
                Some(t_upstream_connect_ms),
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
            if !attempt_already_recorded {
                record_forward_proxy_attempt(
                    state.clone(),
                    selected_proxy.clone(),
                    false,
                    Some(elapsed_ms(upstream_attempt_started_at)),
                    Some(FORWARD_PROXY_FAILURE_STREAM_ERROR),
                    false,
                )
                .await;
            }
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
            if !attempt_already_recorded {
                let success = proxy_forward_response_status_is_success(upstream_status, false);
                record_forward_proxy_attempt(
                    state.clone(),
                    selected_proxy.clone(),
                    success,
                    Some(elapsed_ms(upstream_attempt_started_at)),
                    proxy_forward_response_failure_kind(upstream_status, false),
                    false,
                )
                .await;
            }
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
    let upstream_attempt_started_at_for_record = upstream_attempt_started_at;
    let attempt_already_recorded_for_response = attempt_already_recorded;
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

        if !attempt_already_recorded_for_response {
            let success = proxy_forward_response_status_is_success(
                upstream_status_for_record,
                stream_error_happened,
            );
            record_forward_proxy_attempt(
                state_for_record,
                selected_proxy_for_record,
                success,
                Some(elapsed_ms(upstream_attempt_started_at_for_record)),
                proxy_forward_response_failure_kind(
                    upstream_status_for_record,
                    stream_error_happened,
                ),
                false,
            )
            .await;
        }

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
    original_uri: &Uri,
    headers: HeaderMap,
    body: Body,
    capture_target: ProxyCaptureTarget,
    target_url: Url,
    peer_ip: Option<IpAddr>,
) -> Result<Response, (StatusCode, String)> {
    let capture_started = Instant::now();
    let occurred_at_utc = Utc::now();
    let occurred_at = format_naive(occurred_at_utc.with_timezone(&Shanghai).naive_local());
    let invoke_id = format!(
        "proxy-{proxy_request_id}-{}",
        occurred_at_utc.timestamp_millis()
    );
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let requester_ip = extract_requester_ip(&headers, peer_ip);
    let header_sticky_key = extract_sticky_key_from_headers(&headers);
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);
    let pool_route_active = request_matches_pool_route(state.as_ref(), &headers)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to resolve pool routing settings: {err}"),
            )
        })?;

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
                failure_kind: Some(read_err.failure_kind.to_string()),
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
                    if pool_route_active {
                        INVOCATION_UPSTREAM_SCOPE_INTERNAL
                    } else {
                        INVOCATION_UPSTREAM_SCOPE_EXTERNAL
                    },
                    if pool_route_active {
                        INVOCATION_ROUTE_MODE_POOL
                    } else {
                        INVOCATION_ROUTE_MODE_FORWARD_PROXY
                    },
                    header_sticky_key.as_deref(),
                    header_prompt_cache_key.as_deref(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
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

    let proxy_settings = state.proxy_model_settings.read().await.clone();
    let req_parse_started = Instant::now();
    let (upstream_body, request_info, body_rewritten) = prepare_target_request_body(
        capture_target,
        request_body_bytes,
        state.config.proxy_enforce_stream_include_usage,
        proxy_settings.fast_mode_rewrite_mode,
    );
    let prompt_cache_key = request_info
        .prompt_cache_key
        .clone()
        .or_else(|| header_prompt_cache_key.clone());
    let sticky_key = request_info
        .sticky_key
        .clone()
        .or_else(|| header_sticky_key.clone());
    let t_req_parse_ms = elapsed_ms(req_parse_started);
    let req_raw = store_raw_payload_file(&state.config, &invoke_id, "request", &upstream_body);
    let upstream_body_bytes = Bytes::from(upstream_body);

    let mut upstream_headers = headers.clone();
    if body_rewritten {
        upstream_headers.remove(header::CONTENT_LENGTH);
    }
    let handshake_timeout = state
        .config
        .proxy_upstream_handshake_timeout(Some(capture_target));
    let (
        selected_proxy,
        pool_account,
        t_upstream_connect_ms,
        prefetched_first_chunk,
        prefetched_ttfb_ms,
        attempt_already_recorded,
        final_attempt_update,
        upstream_response,
    ) = if pool_route_active {
        match send_pool_request_with_failover(
            state.clone(),
            Method::POST,
            &original_uri,
            &upstream_headers,
            Some(PoolReplayBodySnapshot::Memory(upstream_body_bytes)),
            handshake_timeout,
            sticky_key.as_deref(),
            None,
            POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
        )
        .await
        {
            Ok(response) => (
                None,
                Some(response.account),
                response.connect_latency_ms,
                response.first_chunk,
                response.first_byte_latency_ms,
                true,
                None,
                response.response,
            ),
            Err(err) => {
                let usage = ParsedUsage::default();
                let (cost, cost_estimated, price_version) =
                    estimate_proxy_cost_from_shared_catalog(
                        &state.pricing_catalog,
                        request_info.model.as_deref(),
                        &usage,
                    )
                    .await;
                let error_message = format!("[{}] {}", err.failure_kind, err.message);
                let record = ProxyCaptureRecord {
                    invoke_id,
                    occurred_at,
                    model: request_info.model,
                    usage,
                    cost,
                    cost_estimated,
                    price_version,
                    status: if err.status.is_server_error() {
                        format!("http_{}", err.status.as_u16())
                    } else {
                        "failed".to_string()
                    },
                    error_message: Some(error_message),
                    failure_kind: Some(err.failure_kind.to_string()),
                    payload: Some(build_proxy_payload_summary(
                        capture_target,
                        err.status,
                        request_info.is_stream,
                        None,
                        request_info.requested_service_tier.as_deref(),
                        request_info.reasoning_effort.as_deref(),
                        None,
                        None,
                        request_info.parse_error.as_deref(),
                        Some(err.failure_kind),
                        requester_ip.as_deref(),
                        INVOCATION_UPSTREAM_SCOPE_INTERNAL,
                        INVOCATION_ROUTE_MODE_POOL,
                        sticky_key.as_deref(),
                        prompt_cache_key.as_deref(),
                        err.account.as_ref().map(|account| account.account_id),
                        err.account
                            .as_ref()
                            .map(|account| account.display_name.as_str()),
                        None,
                        None,
                        err.upstream_error_code.as_deref(),
                        err.upstream_error_message.as_deref(),
                        err.upstream_request_id.as_deref(),
                        None,
                        None,
                    )),
                    raw_response: "{}".to_string(),
                    req_raw,
                    resp_raw: RawPayloadMeta::default(),
                    timings: StageTimings {
                        t_total_ms: 0.0,
                        t_req_read_ms,
                        t_req_parse_ms,
                        t_upstream_connect_ms: err.connect_latency_ms,
                        t_upstream_ttfb_ms: 0.0,
                        t_upstream_stream_ms: 0.0,
                        t_resp_parse_ms: 0.0,
                        t_persist_ms: 0.0,
                    },
                };
                if let Err(err) =
                    persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record)
                        .await
                {
                    warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
                }
                return Err((err.status, err.message));
            }
        }
    } else {
        match send_forward_proxy_request_with_429_retry(
            state.clone(),
            Method::POST,
            target_url,
            &upstream_headers,
            Some(upstream_body_bytes),
            handshake_timeout,
            proxy_settings.upstream_429_max_retries,
        )
        .await
        {
            Ok(response) => (
                Some(response.selected_proxy),
                None,
                response.connect_latency_ms,
                None,
                0.0,
                response.attempt_recorded,
                response.attempt_update,
                response.response,
            ),
            Err(err) => {
                let proxy_attempt_update = record_forward_proxy_attempt(
                    state.clone(),
                    err.selected_proxy.clone(),
                    false,
                    Some(err.connect_latency_ms),
                    Some(err.attempt_failure_kind),
                    false,
                )
                .await;
                let usage = ParsedUsage::default();
                let (cost, cost_estimated, price_version) =
                    estimate_proxy_cost_from_shared_catalog(
                        &state.pricing_catalog,
                        request_info.model.as_deref(),
                        &usage,
                    )
                    .await;
                let error_message = format!("[{}] {}", err.failure_kind, err.message);
                let record = ProxyCaptureRecord {
                    invoke_id,
                    occurred_at,
                    model: request_info.model,
                    usage,
                    cost,
                    cost_estimated,
                    price_version,
                    status: if err.status.is_server_error() {
                        format!("http_{}", err.status.as_u16())
                    } else {
                        "failed".to_string()
                    },
                    error_message: Some(error_message),
                    failure_kind: Some(err.failure_kind.to_string()),
                    payload: Some(build_proxy_payload_summary(
                        capture_target,
                        err.status,
                        request_info.is_stream,
                        None,
                        request_info.requested_service_tier.as_deref(),
                        request_info.reasoning_effort.as_deref(),
                        None,
                        None,
                        request_info.parse_error.as_deref(),
                        Some(err.failure_kind),
                        requester_ip.as_deref(),
                        INVOCATION_UPSTREAM_SCOPE_EXTERNAL,
                        INVOCATION_ROUTE_MODE_FORWARD_PROXY,
                        sticky_key.as_deref(),
                        prompt_cache_key.as_deref(),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(err.selected_proxy.display_name.as_str()),
                        proxy_attempt_update.delta(),
                    )),
                    raw_response: "{}".to_string(),
                    req_raw,
                    resp_raw: RawPayloadMeta::default(),
                    timings: StageTimings {
                        t_total_ms: 0.0,
                        t_req_read_ms,
                        t_req_parse_ms,
                        t_upstream_connect_ms: err.connect_latency_ms,
                        t_upstream_ttfb_ms: 0.0,
                        t_upstream_stream_ms: 0.0,
                        t_resp_parse_ms: 0.0,
                        t_persist_ms: 0.0,
                    },
                };
                if let Err(err) =
                    persist_and_broadcast_proxy_capture(state.as_ref(), capture_started, record)
                        .await
                {
                    warn!(proxy_request_id, error = %err, "failed to persist proxy capture record");
                }
                return Err((err.status, err.message));
            }
        }
    };

    let upstream_status = upstream_response.status();
    let location_base_url = location_rewrite_upstream_base(
        pool_account.as_ref(),
        &state.config.openai_upstream_base_url,
    );
    let rewritten_location = match normalize_proxy_location_header(
        upstream_status,
        upstream_response.headers(),
        location_base_url,
    ) {
        Ok(location) => location,
        Err(err) => {
            let message = format!("failed to process upstream redirect: {err}");
            let proxy_attempt_update = if let Some(selected_proxy) = selected_proxy.as_ref() {
                record_forward_proxy_attempt(
                    state.clone(),
                    selected_proxy.clone(),
                    false,
                    Some(t_upstream_connect_ms),
                    Some(FORWARD_PROXY_FAILURE_SEND_ERROR),
                    false,
                )
                .await
            } else {
                ForwardProxyAttemptUpdate::default()
            };
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
                failure_kind: None,
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
                    if pool_route_active {
                        INVOCATION_UPSTREAM_SCOPE_INTERNAL
                    } else {
                        INVOCATION_UPSTREAM_SCOPE_EXTERNAL
                    },
                    if pool_route_active {
                        INVOCATION_ROUTE_MODE_POOL
                    } else {
                        INVOCATION_ROUTE_MODE_FORWARD_PROXY
                    },
                    sticky_key.as_deref(),
                    prompt_cache_key.as_deref(),
                    pool_account.as_ref().map(|account| account.account_id),
                    pool_account
                        .as_ref()
                        .map(|account| account.display_name.as_str()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    selected_proxy
                        .as_ref()
                        .map(|proxy| proxy.display_name.as_str()),
                    if selected_proxy.is_some() {
                        proxy_attempt_update.delta()
                    } else {
                        None
                    },
                )),
                raw_response: "{}".to_string(),
                req_raw,
                resp_raw: RawPayloadMeta::default(),
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
    let upstream_content_encoding_for_task = upstream_content_encoding.clone();
    let requester_ip_for_task = requester_ip.clone();
    let sticky_key_for_task = sticky_key.clone();
    let prompt_cache_key_for_task = prompt_cache_key.clone();
    let selected_proxy_for_task = selected_proxy.clone();
    let pool_account_for_task = pool_account.clone();
    let attempt_already_recorded_for_task = attempt_already_recorded;
    let final_attempt_update_for_task = final_attempt_update;
    let prefetched_first_chunk_for_task = prefetched_first_chunk;
    let prefetched_ttfb_ms_for_task = prefetched_ttfb_ms;
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);

    tokio::spawn(async move {
        let mut stream = upstream_response.bytes_stream();
        let ttfb_started = Instant::now();
        let stream_started = Instant::now();
        let mut t_upstream_ttfb_ms = prefetched_ttfb_ms_for_task;
        let mut stream_started_at: Option<Instant> = None;
        let mut response_bytes: Vec<u8> = Vec::new();
        let mut stream_error: Option<String> = None;
        let mut downstream_closed = false;
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;

        if let Some(chunk) = prefetched_first_chunk_for_task {
            response_bytes.extend_from_slice(&chunk);
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            stream_started_at = Some(Instant::now());
            if !downstream_closed && tx.send(Ok(chunk)).await.is_err() {
                downstream_closed = true;
            }
        }

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
        } else if response_info.stream_terminal_event.is_some() {
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        } else if upstream_status == StatusCode::TOO_MANY_REQUESTS {
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        } else if upstream_status.is_server_error() {
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
        } else {
            None
        };
        let had_stream_error = stream_error.is_some();
        let had_logical_stream_failure = response_info.stream_terminal_event.is_some();

        let error_message = if let Some(err) = stream_error {
            Some(format!("[{}] {err}", PROXY_FAILURE_UPSTREAM_STREAM_ERROR))
        } else if downstream_closed {
            Some(format!(
                "[{}] downstream closed while streaming upstream response",
                PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
            ))
        } else if response_info.stream_terminal_event.is_some() {
            Some(format_upstream_response_failed_message(&response_info))
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
        let selected_proxy_display_name = selected_proxy_for_task
            .as_ref()
            .map(|proxy| proxy.display_name.clone());
        let proxy_attempt_update = if let Some(selected_proxy) = selected_proxy_for_task.as_ref() {
            let forward_proxy_success = proxy_capture_response_status_is_success(
                upstream_status,
                had_stream_error,
                had_logical_stream_failure,
            );
            if attempt_already_recorded_for_task {
                final_attempt_update_for_task.unwrap_or_default()
            } else {
                record_forward_proxy_attempt(
                    state_for_task.clone(),
                    selected_proxy.clone(),
                    forward_proxy_success,
                    Some(t_upstream_connect_ms + t_upstream_ttfb_ms + t_upstream_stream_ms),
                    proxy_capture_response_failure_kind(
                        upstream_status,
                        had_stream_error,
                        had_logical_stream_failure,
                    ),
                    false,
                )
                .await
            }
        } else {
            if let Some(account) = pool_account_for_task.as_ref() {
                let pool_route_success = proxy_capture_response_status_is_success(
                    upstream_status,
                    had_stream_error,
                    had_logical_stream_failure,
                );
                let route_result = if pool_route_success {
                    record_pool_route_success(
                        &state_for_task.pool,
                        account.account_id,
                        sticky_key_for_task.as_deref(),
                    )
                    .await
                } else if had_stream_error {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream stream error")
                        .to_string();
                    record_pool_route_transport_failure(
                        &state_for_task.pool,
                        account.account_id,
                        sticky_key_for_task.as_deref(),
                        &route_message,
                    )
                    .await
                } else {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream request failed")
                        .to_string();
                    record_pool_route_http_failure(
                        &state_for_task.pool,
                        account.account_id,
                        &account.kind,
                        sticky_key_for_task.as_deref(),
                        upstream_status,
                        &route_message,
                    )
                    .await
                };
                if let Err(err) = route_result {
                    warn!(account_id = account.account_id, error = %err, "failed to record pool capture route state");
                }
            }
            ForwardProxyAttemptUpdate::default()
        };
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
            if pool_account_for_task.is_some() {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            if pool_account_for_task.is_some() {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key_for_task.as_deref(),
            prompt_cache_key_for_task.as_deref(),
            pool_account_for_task
                .as_ref()
                .map(|account| account.account_id),
            pool_account_for_task
                .as_ref()
                .map(|account| account.display_name.as_str()),
            response_info.service_tier.as_deref(),
            response_info.stream_terminal_event.as_deref(),
            response_info.upstream_error_code.as_deref(),
            response_info.upstream_error_message.as_deref(),
            response_info.upstream_request_id.as_deref(),
            selected_proxy_display_name.as_deref(),
            if selected_proxy_display_name.is_some() {
                proxy_attempt_update.delta()
            } else {
                None
            },
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
            failure_kind: failure_kind.map(|kind| kind.to_string()),
            payload: Some(payload),
            raw_response: build_raw_response_preview(&response_bytes),
            req_raw: req_raw_for_task,
            resp_raw,
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
        sticky_key: None,
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
    info.sticky_key = extract_sticky_key_from_request_body(&value);
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

fn proxy_upstream_handshake_timeout_for_capture_target(
    config: &AppConfig,
    capture_target: Option<ProxyCaptureTarget>,
) -> Duration {
    if capture_target.is_some_and(ProxyCaptureTarget::uses_compact_upstream_timeout) {
        config.openai_proxy_compact_handshake_timeout
    } else {
        config.openai_proxy_handshake_timeout
    }
}

fn extract_sticky_key_from_request_body(value: &Value) -> Option<String> {
    const STICKY_KEY_POINTERS: &[&str] = &[
        "/metadata/sticky_key",
        "/metadata/stickyKey",
        "/metadata/prompt_cache_key",
        "/metadata/promptCacheKey",
        "/sticky_key",
        "/stickyKey",
        "/prompt_cache_key",
        "/promptCacheKey",
    ];

    for pointer in STICKY_KEY_POINTERS {
        if let Some(sticky_key) = value.pointer(pointer).and_then(|v| v.as_str()) {
            let normalized = sticky_key.trim();
            if !normalized.is_empty() {
                return Some(normalized.to_string());
            }
        }
    }
    None
}

fn extract_prompt_cache_key_from_request_body(value: &Value) -> Option<String> {
    for pointer in [
        "/metadata/prompt_cache_key",
        "/metadata/promptCacheKey",
        "/prompt_cache_key",
        "/promptCacheKey",
    ] {
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
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
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
                    stream_terminal_event: None,
                    upstream_error_code: extract_upstream_error_code(&value),
                    upstream_error_message: extract_upstream_error_message(&value),
                    upstream_request_id: extract_upstream_request_id(&value),
                }
            }
            Err(_) => ResponseCaptureInfo {
                model: None,
                usage: ParsedUsage::default(),
                usage_missing_reason: Some("response_not_json".to_string()),
                service_tier: None,
                stream_terminal_event: None,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
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
    decode_response_payload(bytes, content_encoding, false)
}

fn parse_stream_response_payload(bytes: &[u8]) -> ResponseCaptureInfo {
    let text = String::from_utf8_lossy(bytes);
    let mut model: Option<String> = None;
    let mut usage = ParsedUsage::default();
    let mut service_tier: Option<String> = None;
    let mut stream_terminal_event: Option<String> = None;
    let mut upstream_error_code: Option<String> = None;
    let mut upstream_error_message: Option<String> = None;
    let mut upstream_request_id: Option<String> = None;
    let mut usage_found = false;
    let mut parse_error_seen = false;
    let mut pending_event_name: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("event:") {
            pending_event_name = Some(trimmed.trim_start_matches("event:").trim().to_string());
            continue;
        }
        if !trimmed.starts_with("data:") {
            continue;
        }
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            pending_event_name = None;
            continue;
        }
        match serde_json::from_str::<Value>(payload) {
            Ok(value) => {
                let event_name = pending_event_name.take();
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
                if stream_payload_indicates_failure(event_name.as_deref(), &value) {
                    let candidate = event_name
                        .clone()
                        .or_else(|| extract_stream_payload_type(&value))
                        .unwrap_or_else(|| "response.failed".to_string());
                    if stream_terminal_event.is_none() || candidate == "response.failed" {
                        stream_terminal_event = Some(candidate);
                    }
                }
                if upstream_error_code.is_none() {
                    upstream_error_code = extract_upstream_error_code(&value);
                }
                if upstream_error_message.is_none() {
                    upstream_error_message = extract_upstream_error_message(&value);
                }
                if upstream_request_id.is_none() {
                    upstream_request_id = extract_upstream_request_id(&value);
                }
            }
            Err(_) => {
                pending_event_name = None;
                parse_error_seen = true;
            }
        }
    }

    let usage_missing_reason = if usage_found {
        None
    } else if stream_terminal_event.is_some() {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string())
    } else if parse_error_seen {
        Some("stream_event_parse_error".to_string())
    } else {
        Some("usage_missing_in_stream".to_string())
    };

    ResponseCaptureInfo {
        model,
        usage,
        usage_missing_reason,
        service_tier,
        stream_terminal_event,
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
    }
}

fn extract_stream_payload_type(value: &Value) -> Option<String> {
    value
        .get("type")
        .and_then(|entry| entry.as_str())
        .map(|entry| entry.to_string())
}

fn stream_payload_indicates_failure(event_name: Option<&str>, value: &Value) -> bool {
    matches!(event_name, Some("response.failed") | Some("error"))
        || value
            .get("type")
            .and_then(|entry| entry.as_str())
            .is_some_and(|kind| kind == "response.failed" || kind == "error")
        || value
            .pointer("/response/status")
            .and_then(|entry| entry.as_str())
            .is_some_and(|status| status.eq_ignore_ascii_case("failed"))
}

fn extract_upstream_error_object(value: &Value) -> Option<&Value> {
    value
        .get("error")
        .filter(|entry| entry.is_object())
        .or_else(|| {
            value
                .pointer("/response/error")
                .filter(|entry| entry.is_object())
        })
}

fn extract_upstream_error_code(value: &Value) -> Option<String> {
    extract_upstream_error_object(value)
        .and_then(|entry| entry.get("code"))
        .and_then(|entry| entry.as_str())
        .map(|entry| entry.to_string())
        .or_else(|| {
            value
                .get("code")
                .and_then(|entry| entry.as_str())
                .map(|entry| entry.to_string())
        })
}

fn extract_upstream_error_message(value: &Value) -> Option<String> {
    extract_upstream_error_object(value)
        .and_then(|entry| entry.get("message"))
        .and_then(|entry| entry.as_str())
        .map(|entry| entry.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|entry| entry.as_str())
                .map(|entry| entry.to_string())
        })
}

fn extract_upstream_request_id(value: &Value) -> Option<String> {
    extract_upstream_error_object(value)
        .and_then(|entry| {
            entry
                .get("request_id")
                .or_else(|| entry.get("requestId"))
                .and_then(|value| value.as_str())
        })
        .map(|entry| entry.to_string())
        .or_else(|| {
            value
                .get("request_id")
                .or_else(|| value.get("requestId"))
                .and_then(|entry| entry.as_str())
                .map(|entry| entry.to_string())
        })
        .or_else(|| {
            extract_upstream_error_message(value)
                .and_then(|message| extract_request_id_from_message(&message))
        })
}

fn extract_request_id_from_message(message: &str) -> Option<String> {
    let lower_message = message.to_ascii_lowercase();
    let start = lower_message
        .find("request id ")
        .map(|index| index + "request id ".len())
        .or_else(|| {
            lower_message
                .find("request_id=")
                .map(|index| index + "request_id=".len())
        })
        .or_else(|| {
            lower_message
                .find("x-request-id: ")
                .map(|index| index + "x-request-id: ".len())
        })?;
    let tail = &message[start..];
    let request_id: String = tail
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '-' | '_'))
        .collect();
    if request_id.is_empty() {
        None
    } else {
        Some(request_id)
    }
}

fn decode_response_payload_for_usage<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    decode_response_payload(bytes, content_encoding, true)
}

fn decode_response_payload<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
    allow_gzip_magic_fallback: bool,
) -> (Cow<'a, [u8]>, Option<String>) {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        if allow_gzip_magic_fallback && response_payload_looks_like_gzip_magic(bytes) {
            return decode_single_content_encoding(bytes, "gzip")
                .map(|decoded| (decoded, None))
                .unwrap_or_else(|err| {
                    (
                        Cow::Borrowed(bytes),
                        Some(format!("response_gzip_decode_error:{err}")),
                    )
                });
        }
        return (Cow::Borrowed(bytes), None);
    }

    let mut encodings = encodings.iter().rev();
    let first_encoding = encodings.next().expect("non-empty encodings checked above");
    let mut decoded = match decode_single_content_encoding(bytes, first_encoding) {
        Ok(next) => next.into_owned(),
        Err(err) => {
            return (
                Cow::Borrowed(bytes),
                Some(format!("{first_encoding}:{err}")),
            );
        }
    };
    for encoding in encodings {
        match decode_single_content_encoding(decoded.as_slice(), encoding) {
            Ok(next) => decoded = next.into_owned(),
            Err(err) => {
                return (Cow::Borrowed(bytes), Some(format!("{encoding}:{err}")));
            }
        }
    }
    (Cow::Owned(decoded), None)
}

fn parse_content_encodings(content_encoding: Option<&str>) -> Vec<String> {
    content_encoding
        .into_iter()
        .flat_map(|raw| raw.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

fn decode_single_content_encoding<'a>(
    bytes: &'a [u8],
    encoding: &str,
) -> std::result::Result<Cow<'a, [u8]>, String> {
    match encoding {
        "identity" => Ok(Cow::Borrowed(bytes)),
        "gzip" | "x-gzip" => decode_gzip_payload(bytes),
        "br" => decode_brotli_payload(bytes),
        "deflate" => decode_deflate_payload(bytes),
        other => Err(format!("unsupported_content_encoding:{other}")),
    }
}

fn decode_gzip_payload<'a>(bytes: &'a [u8]) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut decoder = GzDecoder::new(bytes);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    Ok(Cow::Owned(decoded))
}

fn decode_brotli_payload<'a>(bytes: &'a [u8]) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut decoder = BrotliDecompressor::new(bytes, 4096);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    Ok(Cow::Owned(decoded))
}

fn decode_deflate_payload<'a>(bytes: &'a [u8]) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut zlib_decoder = ZlibDecoder::new(bytes);
    let mut decoded = Vec::new();
    match zlib_decoder.read_to_end(&mut decoded) {
        Ok(_) => Ok(Cow::Owned(decoded)),
        Err(zlib_err) => {
            let mut raw_decoder = DeflateDecoder::new(bytes);
            let mut raw_decoded = Vec::new();
            raw_decoder
                .read_to_end(&mut raw_decoded)
                .map_err(|raw_err| format!("zlib={zlib_err}; raw={raw_err}"))?;
            Ok(Cow::Owned(raw_decoded))
        }
    }
}

fn response_payload_looks_like_gzip_magic(bytes: &[u8]) -> bool {
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

fn upstream_account_id_from_payload(payload: Option<&str>) -> Option<i64> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value.get("upstreamAccountId").and_then(json_value_to_i64)
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
    upstream_scope: &str,
    route_mode: &str,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
    service_tier: Option<&str>,
    stream_terminal_event: Option<&str>,
    upstream_error_code: Option<&str>,
    upstream_error_message: Option<&str>,
    upstream_request_id: Option<&str>,
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
        "upstreamScope": upstream_scope,
        "routeMode": route_mode,
        "stickyKey": sticky_key,
        "promptCacheKey": prompt_cache_key,
        "upstreamAccountId": upstream_account_id,
        "upstreamAccountName": upstream_account_name,
        "serviceTier": service_tier,
        "streamTerminalEvent": stream_terminal_event,
        "upstreamErrorCode": upstream_error_code,
        "upstreamErrorMessage": upstream_error_message,
        "upstreamRequestId": upstream_request_id,
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

fn summarize_plaintext_upstream_error(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if text.starts_with('<')
        || lower.starts_with("<!doctype")
        || lower.starts_with("<html")
        || lower.starts_with("<body")
    {
        return None;
    }
    Some(text.chars().take(240).collect())
}

fn summarize_pool_upstream_http_failure(
    status: StatusCode,
    upstream_request_id_header: Option<&str>,
    bytes: &[u8],
) -> (Option<String>, Option<String>, Option<String>, String) {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        let detail = summarize_plaintext_upstream_error(bytes);
        let message = detail.as_deref().map_or_else(
            || format!("pool upstream responded with {}", status.as_u16()),
            |detail| {
                format!(
                    "pool upstream responded with {}: {}",
                    status.as_u16(),
                    detail
                )
            },
        );
        return (
            None,
            detail,
            upstream_request_id_header.map(|value| value.to_string()),
            message,
        );
    };
    let upstream_error_code = extract_upstream_error_code(&value);
    let upstream_error_message = extract_upstream_error_message(&value);
    let upstream_request_id = upstream_request_id_header
        .map(|value| value.to_string())
        .or_else(|| extract_upstream_request_id(&value));

    let detail = upstream_error_message
        .as_deref()
        .or_else(|| value.get("message").and_then(|entry| entry.as_str()))
        .map(str::trim)
        .filter(|detail| !detail.is_empty())
        .map(|detail| detail.chars().take(240).collect::<String>());

    let message = if let Some(detail) = detail {
        format!(
            "pool upstream responded with {}: {}",
            status.as_u16(),
            detail
        )
    } else {
        format!("pool upstream responded with {}", status.as_u16())
    };

    (
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
        message,
    )
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

async fn broadcast_proxy_capture_follow_up(
    pool: &Pool<Sqlite>,
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: &Mutex<BroadcastStateCache>,
    relay_config: Option<&CrsStatsConfig>,
    invoke_id: &str,
) {
    if broadcaster.receiver_count() == 0 {
        return;
    }

    match collect_summary_snapshots(pool, relay_config).await {
        Ok(summaries) => {
            for summary in summaries {
                if let Err(err) = broadcast_summary_if_changed(
                    broadcaster,
                    broadcast_state_cache,
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
        return;
    }

    match QuotaSnapshotResponse::fetch_latest(pool).await {
        Ok(Some(snapshot)) => {
            if let Err(err) =
                broadcast_quota_if_changed(broadcaster, broadcast_state_cache, snapshot).await
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

struct SummaryQuotaBroadcastIdleContext<'a> {
    latest_broadcast_seq: &'a AtomicU64,
    broadcast_running: &'a AtomicBool,
    shutdown: &'a CancellationToken,
    pool: &'a Pool<Sqlite>,
    broadcaster: &'a broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: &'a Mutex<BroadcastStateCache>,
    relay_config: Option<&'a CrsStatsConfig>,
    invoke_id: &'a str,
}

async fn finish_summary_quota_broadcast_idle(
    ctx: SummaryQuotaBroadcastIdleContext<'_>,
    synced_seq: u64,
) -> bool {
    ctx.broadcast_running.store(false, Ordering::Release);

    let pending_seq = ctx.latest_broadcast_seq.load(Ordering::Acquire);
    if pending_seq == synced_seq {
        return false;
    }

    if ctx.shutdown.is_cancelled() {
        info!(
            invoke_id = %ctx.invoke_id,
            pending_seq,
            synced_seq,
            "flushing final summary/quota snapshots inline because shutdown arrived during broadcast worker idle handoff"
        );
        broadcast_proxy_capture_follow_up(
            ctx.pool,
            ctx.broadcaster,
            ctx.broadcast_state_cache,
            ctx.relay_config,
            ctx.invoke_id,
        )
        .await;
        return false;
    }

    ctx.broadcast_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
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

    if state.shutdown.is_cancelled() {
        info!(
            invoke_id = %invoke_id,
            "broadcasting final summary/quota snapshots inline because shutdown is in progress"
        );
        broadcast_proxy_capture_follow_up(
            &state.pool,
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            state.config.crs_stats.as_ref(),
            &invoke_id,
        )
        .await;
        return Ok(());
    }

    state
        .proxy_summary_quota_broadcast_seq
        .fetch_add(1, Ordering::Relaxed);
    if state.shutdown.is_cancelled() {
        info!(
            invoke_id = %invoke_id,
            "broadcasting final summary/quota snapshots inline because shutdown started after record broadcast"
        );
        broadcast_proxy_capture_follow_up(
            &state.pool,
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            state.config.crs_stats.as_ref(),
            &invoke_id,
        )
        .await;
        return Ok(());
    }
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
    let shutdown = state.shutdown.clone();
    let broadcast_handle_slot = state.proxy_summary_quota_broadcast_handle.clone();
    let handle = tokio::spawn(async move {
        let mut synced_seq = 0_u64;
        loop {
            let target_seq = latest_broadcast_seq.load(Ordering::Acquire);
            if shutdown.is_cancelled() {
                if target_seq != synced_seq {
                    info!(
                        invoke_id = %invoke_id,
                        "flushing final summary/quota snapshots inline before shutdown"
                    );
                    broadcast_proxy_capture_follow_up(
                        &pool,
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        relay_config.as_ref(),
                        &invoke_id,
                    )
                    .await;
                }
                broadcast_running.store(false, Ordering::Release);
                info!(
                    invoke_id = %invoke_id,
                    "stopping summary/quota broadcast worker because shutdown is in progress"
                );
                break;
            }

            if target_seq == synced_seq {
                if finish_summary_quota_broadcast_idle(
                    SummaryQuotaBroadcastIdleContext {
                        latest_broadcast_seq: latest_broadcast_seq.as_ref(),
                        broadcast_running: broadcast_running.as_ref(),
                        shutdown: &shutdown,
                        pool: &pool,
                        broadcaster: &broadcaster,
                        broadcast_state_cache: broadcast_state_cache.as_ref(),
                        relay_config: relay_config.as_ref(),
                        invoke_id: &invoke_id,
                    },
                    synced_seq,
                )
                .await
                {
                    continue;
                }
                break;
            }
            synced_seq = target_seq;

            if broadcaster.receiver_count() == 0 {
                continue;
            }

            let summaries = tokio::select! {
                _ = shutdown.cancelled() => {
                    broadcast_proxy_capture_follow_up(
                        &pool,
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        relay_config.as_ref(),
                        &invoke_id,
                    )
                    .await;
                    broadcast_running.store(false, Ordering::Release);
                    info!(
                        invoke_id = %invoke_id,
                        "summary/quota broadcast worker flushed follow-up before collecting summaries during shutdown"
                    );
                    break;
                }
                result = collect_summary_snapshots(&pool, relay_config.as_ref()) => result,
            };
            match summaries {
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

            let quota = tokio::select! {
                _ = shutdown.cancelled() => {
                    broadcast_proxy_capture_follow_up(
                        &pool,
                        &broadcaster,
                        broadcast_state_cache.as_ref(),
                        relay_config.as_ref(),
                        &invoke_id,
                    )
                    .await;
                    broadcast_running.store(false, Ordering::Release);
                    info!(
                        invoke_id = %invoke_id,
                        "summary/quota broadcast worker flushed follow-up before fetching quota during shutdown"
                    );
                    break;
                }
                result = QuotaSnapshotResponse::fetch_latest(&pool) => result,
            };
            match quota {
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

    let finished_handles = {
        let mut guard = broadcast_handle_slot.lock().await;
        let mut active_handles = std::mem::take(&mut *guard);
        let mut finished_handles = Vec::new();
        let mut idx = 0;
        while idx < active_handles.len() {
            if active_handles[idx].is_finished() {
                finished_handles.push(active_handles.remove(idx));
            } else {
                idx += 1;
            }
        }
        active_handles.push(handle);
        *guard = active_handles;
        finished_handles
    };
    for finished_handle in finished_handles {
        if let Err(err) = finished_handle.await {
            error!(
                ?err,
                "summary/quota broadcast worker terminated unexpectedly"
            );
        }
    }

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
    let failure_kind = record
        .failure_kind
        .as_deref()
        .or(failure.failure_kind.as_deref());
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
            ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35
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
    .bind(failure_kind)
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

    if let Some(upstream_account_id) = upstream_account_id_from_payload(record.payload.as_deref()) {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = CASE
                WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                ELSE last_activity_at
            END
            WHERE id = ?2
            "#,
        )
        .bind(&record.occurred_at)
        .bind(upstream_account_id)
        .execute(pool)
        .await?;
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
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.streamTerminalEvent') END AS stream_terminal_event,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorCode') END AS upstream_error_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorMessage') END AS upstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamRequestId') END AS upstream_request_id,
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
    let mut last_error = None;
    for candidate in resolved_raw_path_read_candidates(path, fallback_root) {
        match fs::read(&candidate) {
            Ok(content) => return decode_proxy_raw_file_bytes(&candidate, content),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                last_error = Some(err);
            }
            Err(err) => return Err(err),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("raw payload file not found for path {path}"),
        )
    }))
}

fn decode_proxy_raw_file_bytes(path: &Path, bytes: Vec<u8>) -> io::Result<Vec<u8>> {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gz"))
    {
        let mut decoder = GzDecoder::new(bytes.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to decompress raw payload {}: {err}", path.display()),
            )
        })?;
        Ok(decoded)
    } else {
        Ok(bytes)
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
    source: String,
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
    payload: Option<String>,
    raw_response: String,
    response_raw_path: Option<String>,
}

fn parse_proxy_response_capture_from_stored_bytes(
    target: ProxyCaptureTarget,
    bytes: &[u8],
    is_stream: bool,
) -> ResponseCaptureInfo {
    let (payload_for_parse, _) = decode_response_payload_for_usage(bytes, None);
    parse_target_response_payload(target, payload_for_parse.as_ref(), is_stream, None)
}

fn format_upstream_response_failed_message(response_info: &ResponseCaptureInfo) -> String {
    let upstream_message = response_info
        .upstream_error_message
        .as_deref()
        .unwrap_or("upstream response failed");
    if let Some(code) = response_info.upstream_error_code.as_deref() {
        format!(
            "[{}] {}: {}",
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED, code, upstream_message
        )
    } else {
        format!(
            "[{}] {}",
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED, upstream_message
        )
    }
}

fn update_proxy_payload_failure_details(
    payload: Option<&str>,
    failure_kind: Option<&str>,
    response_info: &ResponseCaptureInfo,
) -> String {
    let mut value = payload
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(|value| value.is_object())
        .unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .expect("payload summary must be an object");

    object.insert(
        "failureKind".to_string(),
        failure_kind
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "streamTerminalEvent".to_string(),
        response_info
            .stream_terminal_event
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "upstreamErrorCode".to_string(),
        response_info
            .upstream_error_code
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "upstreamErrorMessage".to_string(),
        response_info
            .upstream_error_message
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "upstreamRequestId".to_string(),
        response_info
            .upstream_request_id
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "usageMissingReason".to_string(),
        response_info
            .usage_missing_reason
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );

    serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
}

fn should_upgrade_to_upstream_response_failed(
    row: &FailureClassificationBackfillRow,
    existing_kind: Option<&str>,
) -> bool {
    let status_norm = row
        .status
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let error_message_empty = row
        .error_message
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty);

    if matches!(
        existing_kind,
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
            | Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
            | Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
            | Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
            | Some(PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT)
            | Some(PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED)
    ) {
        return false;
    }

    status_norm == "success"
        || existing_kind.is_none()
        || existing_kind == Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        || (status_norm == "http_200" && error_message_empty)
}

fn parse_proxy_response_failure_from_persisted_record(
    row: &FailureClassificationBackfillRow,
    raw_path_fallback_root: Option<&Path>,
) -> Result<Option<ResponseCaptureInfo>> {
    if row.source != SOURCE_PROXY {
        return Ok(None);
    }

    let (target, is_stream) = parse_proxy_capture_summary(row.payload.as_deref());
    let preview_info = parse_proxy_response_capture_from_stored_bytes(
        target,
        row.raw_response.as_bytes(),
        is_stream,
    );
    let preview_has_failure = preview_info.stream_terminal_event.is_some();
    let preview_is_complete = preview_has_failure
        && preview_info.upstream_error_message.is_some()
        && preview_info.upstream_request_id.is_some();

    if preview_is_complete || row.response_raw_path.is_none() {
        return Ok(preview_has_failure.then_some(preview_info));
    }

    let Some(path) = row.response_raw_path.as_deref() else {
        return Ok(preview_has_failure.then_some(preview_info));
    };

    match read_proxy_raw_bytes(path, raw_path_fallback_root) {
        Ok(bytes) => {
            let full_info =
                parse_proxy_response_capture_from_stored_bytes(target, &bytes, is_stream);
            if full_info.stream_terminal_event.is_some() {
                Ok(Some(full_info))
            } else {
                Ok(preview_has_failure.then_some(preview_info))
            }
        }
        Err(_err) if preview_has_failure => Ok(Some(preview_info)),
        Err(err) => Err(err.into()),
    }
}

async fn backfill_failure_classification_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<FailureClassificationBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = FailureClassificationBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let rows = sqlx::query_as::<_, FailureClassificationBackfillRow>(
            r#"
            SELECT
                id,
                source,
                status,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                raw_response,
                response_raw_path
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
                OR (
                    source = ?2
                    AND LOWER(TRIM(COALESCE(status, ''))) = 'success'
                    AND (
                        raw_response LIKE '%response.failed%'
                        OR raw_response LIKE '%"type":"error"%'
                        OR (
                            json_valid(payload)
                            AND (
                                TRIM(COALESCE(CAST(json_extract(payload, '$.usageMissingReason') AS TEXT), '')) IN ('usage_missing_in_stream', 'upstream_response_failed')
                                OR TRIM(COALESCE(CAST(json_extract(payload, '$.streamTerminalEvent') AS TEXT), '')) != ''
                            )
                        )
                        OR (
                            response_raw_path IS NOT NULL
                            AND COALESCE(response_raw_size, LENGTH(raw_response)) >= 16384
                            AND json_valid(payload)
                            AND COALESCE(CAST(json_extract(payload, '$.endpoint') AS TEXT), '') = '/v1/responses'
                            AND COALESCE(json_extract(payload, '$.isStream'), 0) = 1
                            AND TRIM(COALESCE(failure_kind, '')) = ''
                        )
                    )
                )
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(last_seen_id)
        .bind(SOURCE_PROXY)
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
            let existing_kind = row
                .failure_kind
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let existing_class = row
                .failure_class
                .as_deref()
                .and_then(FailureClass::from_db_str);
            let existing_actionable = row.is_actionable.map(|value| value != 0);

            let response_failure = match parse_proxy_response_failure_from_persisted_record(
                &row,
                raw_path_fallback_root,
            ) {
                Ok(result) => result,
                Err(err) => {
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} reason=response_failure_parse_error err={err}",
                            row.id
                        ),
                    );
                    None
                }
            };

            if let Some(response_info) = response_failure.as_ref().filter(|_| {
                should_upgrade_to_upstream_response_failed(&row, existing_kind.as_deref())
            }) {
                let error_message = format_upstream_response_failed_message(response_info);
                let resolved = classify_invocation_failure(Some("http_200"), Some(&error_message));
                let next_payload = update_proxy_payload_failure_details(
                    row.payload.as_deref(),
                    Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED),
                    response_info,
                );
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET status = ?1,
                        error_message = ?2,
                        failure_kind = ?3,
                        failure_class = ?4,
                        is_actionable = ?5,
                        payload = ?6
                    WHERE id = ?7
                    "#,
                )
                .bind("http_200")
                .bind(&error_message)
                .bind(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
                .bind(resolved.failure_class.as_str())
                .bind(resolved.is_actionable as i64)
                .bind(next_payload)
                .bind(row.id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                summary.updated += affected;
                continue;
            }

            let resolved = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );

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
        samples,
    })
}

#[cfg(test)]
#[allow(dead_code)]
async fn backfill_failure_classification(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<FailureClassificationBackfillSummary> {
    Ok(
        backfill_failure_classification_from_cursor(pool, 0, raw_path_fallback_root, None, None)
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
    error_chain_contains(err, "length limit exceeded")
        || error_chain_contains(err, PROXY_REQUEST_BODY_LIMIT_EXCEEDED)
}

fn error_chain_contains(err: &(dyn std::error::Error + 'static), needle: &str) -> bool {
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
    should_transport_proxy_header(name) && !connection_scoped.contains(name)
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

fn location_rewrite_upstream_base<'a>(
    pool_account: Option<&'a PoolResolvedAccount>,
    global_upstream_base_url: &'a Url,
) -> &'a Url {
    pool_account
        .map(|account| &account.upstream_base_url)
        .unwrap_or(global_upstream_base_url)
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

fn should_transport_proxy_header(name: &HeaderName) -> bool {
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

fn extract_sticky_key_from_headers(headers: &HeaderMap) -> Option<String> {
    for header_name in [
        "x-sticky-key",
        "sticky-key",
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

#[derive(Debug, Clone)]
struct AppState {
    config: AppConfig,
    pool: Pool<Sqlite>,
    http_clients: HttpClients,
    broadcaster: broadcast::Sender<BroadcastPayload>,
    broadcast_state_cache: Arc<Mutex<BroadcastStateCache>>,
    proxy_summary_quota_broadcast_seq: Arc<AtomicU64>,
    proxy_summary_quota_broadcast_running: Arc<AtomicBool>,
    proxy_summary_quota_broadcast_handle: Arc<Mutex<Vec<JoinHandle<()>>>>,
    startup_ready: Arc<AtomicBool>,
    shutdown: CancellationToken,
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
    upstream_accounts: Arc<UpstreamAccountsRuntime>,
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
struct ProxyModelSettings {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    fast_mode_rewrite_mode: ProxyFastModeRewriteMode,
    upstream_429_max_retries: u8,
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

fn normalize_proxy_upstream_429_max_retries(value: u8) -> u8 {
    value.min(MAX_PROXY_UPSTREAM_429_MAX_RETRIES)
}

fn decode_proxy_upstream_429_max_retries(raw: Option<i64>) -> u8 {
    raw.and_then(|value| u8::try_from(value).ok())
        .map(normalize_proxy_upstream_429_max_retries)
        .unwrap_or(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES)
}

impl Default for ProxyModelSettings {
    fn default() -> Self {
        Self {
            hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            merge_upstream_enabled: DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
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
            upstream_429_max_retries: normalize_proxy_upstream_429_max_retries(
                self.upstream_429_max_retries,
            ),
            enabled_preset_models: normalize_enabled_preset_models(self.enabled_preset_models),
        }
    }
}

#[derive(Debug, FromRow)]
struct ProxyModelSettingsRow {
    hijack_enabled: i64,
    merge_upstream_enabled: i64,
    fast_mode_rewrite_mode: Option<String>,
    upstream_429_max_retries: Option<i64>,
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
            upstream_429_max_retries: decode_proxy_upstream_429_max_retries(
                value.upstream_429_max_retries,
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
    #[serde(default)]
    upstream_429_max_retries: Option<u8>,
    #[serde(default = "default_enabled_preset_models")]
    enabled_models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyModelSettingsResponse {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    fast_mode_rewrite_mode: ProxyFastModeRewriteMode,
    upstream_429_max_retries: u8,
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
            upstream_429_max_retries: value.upstream_429_max_retries,
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
    openai_proxy_compact_handshake_timeout: Duration,
    openai_proxy_request_read_timeout: Duration,
    openai_proxy_max_request_body_bytes: usize,
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
    archive_dir: PathBuf,
    invocation_success_full_days: u64,
    invocation_max_days: u64,
    forward_proxy_attempts_retention_days: u64,
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
    fn proxy_upstream_handshake_timeout(
        &self,
        capture_target: Option<ProxyCaptureTarget>,
    ) -> Duration {
        proxy_upstream_handshake_timeout_for_capture_target(self, capture_target)
    }

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
        let forward_proxy_attempts_retention_days = parse_u64_env_var(
            ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
            DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
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
            openai_proxy_handshake_timeout,
            openai_proxy_compact_handshake_timeout,
            openai_proxy_request_read_timeout,
            openai_proxy_max_request_body_bytes,
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
            archive_dir,
            invocation_success_full_days,
            invocation_max_days,
            forward_proxy_attempts_retention_days,
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
