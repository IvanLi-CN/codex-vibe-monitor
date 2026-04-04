#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::{
    borrow::Cow,
    collections::hash_map::DefaultHasher,
    collections::{BTreeMap, HashMap, HashSet},
    convert::Infallible,
    env, fmt,
    future::Future,
    hash::{Hash, Hasher},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    pin::Pin,
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
    extract::{ConnectInfo, DefaultBodyLimit, OriginalUri, Query, State},
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
use clap::{Args, Parser, Subcommand};
use dotenvy::dotenv;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::{Compression, write::GzEncoder};
use futures_util::{FutureExt, StreamExt, TryStreamExt, future::Shared, stream};
use once_cell::sync::Lazy;
use rand::Rng;
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
use std::io::{self, BufRead, Read, Write};
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
const IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS: u64 = 60;
const DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS: u64 = 300;
const DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS: u64 = 180;
const DEFAULT_SQLITE_BUSY_TIMEOUT_SECS: u64 = 30;
const CVM_INVOKE_ID_HEADER: &str = "x-cvm-invoke-id";
const BACKFILL_BATCH_SIZE: i64 = 200;
const BACKFILL_ACCOUNT_BIND_BATCH_SIZE: usize = 400;
const STARTUP_BACKFILL_SCAN_LIMIT: u64 = 2_000;
const STARTUP_BACKFILL_RUN_BUDGET_SECS: u64 = 3;
const STARTUP_BACKFILL_ACTIVE_INTERVAL_SECS: u64 = 15;
const STARTUP_BACKFILL_IDLE_INTERVAL_SECS: u64 = 6 * 60 * 60;
const STARTUP_BACKFILL_LOG_SAMPLE_LIMIT: usize = 5;
const STATS_MAINTENANCE_CACHE_TTL_SECS: u64 = 15;
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
const RAW_RESPONSE_PREVIEW_LIMIT: usize = 16 * 1024;
const BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES: usize = 256 * 1024;
const STREAM_RESPONSE_LINE_BUFFER_LIMIT: usize = 256 * 1024;
#[allow(dead_code)]
const RAW_FILE_STREAM_RESPONSE_LINE_BUFFER_LIMIT: usize = 8 * 1024 * 1024;
const PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED: &str =
    "non_stream_response_parse_skipped_body_too_large";
const RAW_CODEC_IDENTITY: &str = "identity";
const RAW_CODEC_GZIP: &str = "gzip";
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
const ENV_RETENTION_CATCHUP_BUDGET_SECS: &str = "RETENTION_CATCHUP_BUDGET_SECS";
const ENV_ARCHIVE_DIR: &str = "ARCHIVE_DIR";
const LEGACY_ENV_ARCHIVE_DIR: &str = "XY_ARCHIVE_DIR";
const ENV_INVOCATION_SUCCESS_FULL_DAYS: &str = "INVOCATION_SUCCESS_FULL_DAYS";
const LEGACY_ENV_INVOCATION_SUCCESS_FULL_DAYS: &str = "XY_INVOCATION_SUCCESS_FULL_DAYS";
const ENV_INVOCATION_MAX_DAYS: &str = "INVOCATION_MAX_DAYS";
const LEGACY_ENV_INVOCATION_MAX_DAYS: &str = "XY_INVOCATION_MAX_DAYS";
const ENV_INVOCATION_ARCHIVE_TTL_DAYS: &str = "INVOCATION_ARCHIVE_TTL_DAYS";
const ENV_CODEX_INVOCATION_ARCHIVE_LAYOUT: &str = "CODEX_INVOCATION_ARCHIVE_LAYOUT";
const ENV_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY: &str =
    "CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY";
const ENV_INVOCATION_ARCHIVE_CODEC: &str = "INVOCATION_ARCHIVE_CODEC";
const ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS: &str = "FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS";
const LEGACY_ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS: &str =
    "XY_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS";
const ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS: &str =
    "POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS";
const LEGACY_ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS: &str =
    "XY_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS";
const ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS: &str =
    "POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS";
const ENV_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS: &str =
    "POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS";
const ENV_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS: &str =
    "POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS";
const LEGACY_ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS: &str =
    "XY_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS";
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
const DEFAULT_RETENTION_CATCHUP_BUDGET_SECS: u64 = 5 * 60;
const DEFAULT_ARCHIVE_DIR: &str = "archives";
const DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS: u64 = 24 * 60 * 60;
const DEFAULT_INVOCATION_SUCCESS_FULL_DAYS: u64 = 30;
const DEFAULT_INVOCATION_MAX_DAYS: u64 = 90;
const DEFAULT_INVOCATION_ARCHIVE_TTL_DAYS: u64 = 30;
const DEFAULT_CODEX_INVOCATION_ARCHIVE_LAYOUT: ArchiveBatchLayout = ArchiveBatchLayout::SegmentV1;
const DEFAULT_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY: ArchiveSegmentGranularity =
    ArchiveSegmentGranularity::Day;
const DEFAULT_INVOCATION_ARCHIVE_CODEC: ArchiveFileCodec = ArchiveFileCodec::Gzip;
const DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS: u64 = 30;
const DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS: u64 = 7;
const DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS: u64 = 30;
const DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS: u64 = 180;
const DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS: u64 = 300;
const DEFAULT_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS: u64 = 30;
const DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS: u64 = 30;
const ARCHIVE_STATUS_COMPLETED: &str = "completed";
const ARCHIVE_LAYOUT_LEGACY_MONTH: &str = "legacy_month";
const ARCHIVE_LAYOUT_SEGMENT_V1: &str = "segment_v1";
const ARCHIVE_SEGMENT_GRANULARITY_DAY: &str = "day";
const ARCHIVE_FILE_CODEC_GZIP: &str = "gzip";
const ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1: &str = "legacy_month_v1";
const ARCHIVE_WRITER_VERSION_SEGMENT_V1: &str = "segment_v1";
const ARCHIVE_CLEANUP_STATE_ACTIVE: &str = "active";
const DEFAULT_ARCHIVE_TEMP_MIN_AGE_SECS: u64 = 6 * 60 * 60;
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
const PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED: &str = "pool_attempt_interrupted";
const PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED: &str = "upstream_response_failed";
const UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED: &str = "server_is_overloaded";
const PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT: &str = "pool_no_available_account";
const PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED: &str = "pool_all_accounts_rate_limited";
const PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED: &str = "pool_all_accounts_degraded";
const PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED: &str = "max_distinct_accounts_exhausted";
const POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE: &str = "no pool account is currently available because all candidate accounts are rate limited upstream (429 / quota exhausted)";
const POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE: &str = "no pool account is currently accepting fresh conversations because all candidate accounts are in temporary degraded state";
const PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT: &str =
    "no_alternate_upstream_after_timeout";
const PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED: &str = "pool_total_timeout_exhausted";
const PROXY_STREAM_TERMINAL_COMPLETED: &str = "stream_completed";
const PROXY_STREAM_TERMINAL_ERROR: &str = "stream_error";
const PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED: &str = "downstream_closed";
const INVOCATION_UPSTREAM_SCOPE_EXTERNAL: &str = "external";
const INVOCATION_UPSTREAM_SCOPE_INTERNAL: &str = "internal";
const INVOCATION_ROUTE_MODE_FORWARD_PROXY: &str = "forward_proxy";
const INVOCATION_ROUTE_MODE_POOL: &str = "pool";
const POOL_ATTEMPT_INTERRUPTED_MESSAGE: &str =
    "pool attempt was interrupted before completion and was recovered on startup";
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
const FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED: &str =
    "upstream_http_429_quota_exhausted";
const PROXY_FAILURE_UPSTREAM_USAGE_SNAPSHOT_QUOTA_EXHAUSTED: &str =
    "upstream_usage_snapshot_quota_exhausted";
const FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX: &str = "upstream_http_5xx";
const PROXY_FAILURE_UPSTREAM_HTTP_402: &str = "upstream_http_402";
const PROXY_FAILURE_UPSTREAM_HTTP_AUTH: &str = "upstream_http_auth";
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
const MIN_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS: u64 = 1;
const MAX_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS: u64 = 10;
const MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS: u64 = 30;
const DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP: bool = true;
const GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS: i64 = 272_000;
const PROMPT_CACHE_CONVERSATION_DEFAULT_LIMIT: i64 = 50;
const PROMPT_CACHE_CONVERSATION_ACTIVITY_MODE_LIMIT: i64 = 50;
const PROMPT_CACHE_CONVERSATION_CHART_MAX_HOURS: i64 = 24;
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
        LEGACY_ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS,
        ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS,
    ),
    (
        LEGACY_ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
        ENV_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
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

#[derive(Debug, Default)]
struct StartupPersistentPrepSummary {
    stale_archive_temp_files_removed: usize,
    refreshed_manifest_batches: usize,
    refreshed_manifest_account_rows: usize,
    missing_manifest_files: usize,
    backfilled_archive_expiries: usize,
    bootstrapped_hourly_rollups: bool,
    pending_historical_rollup_archive_batches: usize,
}

#[derive(Debug, Default)]
struct StatsMaintenanceCacheState {
    cached_at: Option<Instant>,
    response: Option<StatsMaintenanceResponse>,
}

impl StatsMaintenanceCacheState {
    fn fresh_response(&self) -> Option<StatsMaintenanceResponse> {
        let cached_at = self.cached_at?;
        if cached_at.elapsed() > Duration::from_secs(STATS_MAINTENANCE_CACHE_TTL_SECS) {
            return None;
        }
        self.response.clone()
    }

    fn store(&mut self, response: StatsMaintenanceResponse) {
        self.cached_at = Some(Instant::now());
        self.response = Some(response);
    }
}

fn should_run_startup_persistent_prep(cli: &CliArgs) -> bool {
    if cli.command.is_some() {
        return false;
    }
    if cli.retention_run_once {
        return !cli.retention_dry_run;
    }
    true
}

fn should_run_blocking_startup_persistent_prep(cli: &CliArgs) -> bool {
    cli.command.is_none() && cli.retention_run_once && !cli.retention_dry_run
}

fn should_run_blocking_startup_hourly_rollup_bootstrap(cli: &CliArgs) -> bool {
    cli.command.is_none() && !cli.retention_run_once && !cli.retention_dry_run
}

async fn run_startup_persistent_prep_inner(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    cli: &CliArgs,
    include_hourly_rollup_bootstrap: bool,
) -> Result<StartupPersistentPrepSummary> {
    if !should_run_startup_persistent_prep(cli) {
        return Ok(StartupPersistentPrepSummary::default());
    }

    let janitor_summary = cleanup_stale_archive_temp_files(config, false)?;
    let manifest_refresh = refresh_archive_upstream_activity_manifest(pool, false).await?;
    let archive_expiry_backfill_count = backfill_invocation_archive_expiries(pool, config).await?;
    if include_hourly_rollup_bootstrap {
        bootstrap_hourly_rollups(pool).await?;
    }
    let historical_rollup_snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;

    Ok(StartupPersistentPrepSummary {
        stale_archive_temp_files_removed: janitor_summary.stale_temp_files_removed,
        refreshed_manifest_batches: manifest_refresh.refreshed_batches,
        refreshed_manifest_account_rows: manifest_refresh.account_rows_written,
        missing_manifest_files: manifest_refresh.missing_files,
        backfilled_archive_expiries: archive_expiry_backfill_count,
        bootstrapped_hourly_rollups: include_hourly_rollup_bootstrap,
        pending_historical_rollup_archive_batches: historical_rollup_snapshot.legacy_archive_pending
            as usize,
    })
}

async fn run_startup_persistent_prep(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    cli: &CliArgs,
) -> Result<StartupPersistentPrepSummary> {
    run_startup_persistent_prep_inner(pool, config, cli, true).await
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
    if should_recover_pending_pool_attempts_on_startup(&cli) {
        let recovered_pending_pool_attempts =
            recover_orphaned_pool_upstream_request_attempts(&pool).await?;
        if recovered_pending_pool_attempts > 0 {
            warn!(
                recovered_pending_pool_attempts,
                "recovered orphaned pending pool attempt rows at startup"
            );
        }
    }
    if should_run_blocking_startup_hourly_rollup_bootstrap(&cli) {
        let rollup_bootstrap_started_at = Instant::now();
        bootstrap_hourly_rollups(&pool).await?;
        log_startup_phase("hourly_rollup_bootstrap", rollup_bootstrap_started_at);
    }
    if should_run_blocking_startup_persistent_prep(&cli) {
        let prep_summary = run_startup_persistent_prep(&pool, &config, &cli).await?;
        info!(
            stale_archive_temp_files_removed = prep_summary.stale_archive_temp_files_removed,
            refreshed_manifest_batches = prep_summary.refreshed_manifest_batches,
            refreshed_manifest_account_rows = prep_summary.refreshed_manifest_account_rows,
            missing_manifest_files = prep_summary.missing_manifest_files,
            backfilled_archive_expiries = prep_summary.backfilled_archive_expiries,
            bootstrapped_hourly_rollups = prep_summary.bootstrapped_hourly_rollups,
            pending_historical_rollup_archive_batches =
                prep_summary.pending_historical_rollup_archive_batches,
            "startup persistent prep finished"
        );
        if prep_summary.pending_historical_rollup_archive_batches > 0 {
            warn!(
                pending_historical_rollup_archive_batches =
                    prep_summary.pending_historical_rollup_archive_batches,
                "legacy archive batches still need historical rollup materialization"
            );
        }
    }
    if cli.retention_run_once && cli.command.is_some() {
        bail!("--retention-run-once cannot be combined with maintenance subcommands");
    }
    if let Some(command) = &cli.command {
        run_cli_command(&pool, &config, command).await?;
        return Ok(());
    }
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
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
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
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_group_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts,
    });

    let signal_listener = spawn_shutdown_signal_listener(state.shutdown.clone());

    run_runtime_until_shutdown(state, startup_started_at, async move {
        let _ = signal_listener.await;
    })
    .await
}

async fn run_cli_command(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    command: &CliCommand,
) -> Result<()> {
    let raw_path_fallback_root = config.database_path.parent();
    match command {
        CliCommand::Maintenance(args) => match &args.command {
            MaintenanceCommand::RawCompression(opts) => {
                let summary = compress_cold_proxy_raw_payloads_with_budget(
                    pool,
                    config,
                    raw_path_fallback_root,
                    opts.dry_run,
                    None,
                )
                .await?;
                let backlog = load_raw_compression_backlog_snapshot(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    ?summary,
                    ?backlog,
                    "maintenance raw compression finished"
                );
            }
            MaintenanceCommand::ArchiveUpstreamActivityManifest(opts) => {
                let summary =
                    refresh_archive_upstream_activity_manifest(pool, opts.dry_run).await?;
                info!(
                    dry_run = opts.dry_run,
                    ?summary,
                    "maintenance archive upstream activity manifest finished"
                );
            }
            MaintenanceCommand::MaterializeHistoricalRollups(opts) => {
                let summary = materialize_historical_rollups(pool, config, opts.dry_run).await?;
                let snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    ?summary,
                    ?snapshot,
                    "maintenance historical rollup materialization finished"
                );
            }
            MaintenanceCommand::VerifyArchiveStorage(opts) => {
                let summary = verify_archive_storage(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    manifest_rows = summary.manifest_rows,
                    missing_files = summary.missing_files,
                    orphan_files = summary.orphan_files,
                    stale_temp_files = summary.stale_temp_files,
                    stale_temp_bytes = summary.stale_temp_bytes,
                    "maintenance archive storage verification finished"
                );
            }
            MaintenanceCommand::PruneArchiveBatches(opts) => {
                let summary = prune_archive_batches(pool, config, opts.dry_run).await?;
                let snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    expired_archive_batches_deleted = summary.expired_archive_batches_deleted,
                    legacy_archive_batches_deleted = summary.legacy_archive_batches_deleted,
                    ?snapshot,
                    "maintenance archive prune finished"
                );
            }
            MaintenanceCommand::PruneLegacyArchiveBatches(opts) => {
                let summary = prune_legacy_archive_batches(pool, config, opts.dry_run).await?;
                let snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
                info!(
                    dry_run = opts.dry_run,
                    ?summary,
                    ?snapshot,
                    "maintenance legacy archive prune finished"
                );
            }
        },
    }
    Ok(())
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
    state.upstream_accounts.drain_background_tasks().await;
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
    force_idle: bool,
    samples: Vec<String>,
}

fn startup_backfill_next_delay(run: &StartupBackfillRunState, zero_update_streak: u32) -> Duration {
    if run.force_idle {
        Duration::from_secs(STARTUP_BACKFILL_IDLE_INTERVAL_SECS)
    } else if run.hit_scan_limit || run.updated > 0 {
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

    let _guard = state.hourly_rollup_sync_lock.lock().await;
    if let Err(err) = sync_hourly_rollups_from_live_tables(&state.pool).await {
        warn!(error = %err, "startup backfill failed to refresh invocation hourly rollups");
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
                    force_idle: false,
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
                    force_idle: false,
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
                    force_idle: false,
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
                    force_idle: false,
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
                    force_idle: false,
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
                    force_idle: false,
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
                    force_idle: false,
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
                    force_idle: false,
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
            let force_idle = summary.waiting_for_manifest_backfill
                || (pending_accounts > 0 && !summary.hit_budget && summary.updated_accounts == 0);
            Ok((
                StartupBackfillRunState {
                    next_cursor_id: cursor_id,
                    scanned: summary.scanned_batches,
                    updated: summary.updated_accounts,
                    hit_scan_limit: pending_accounts > 0 && summary.hit_budget,
                    force_idle,
                    samples: Vec::new(),
                },
                format!(
                    "pending_accounts={pending_accounts} waiting_for_manifest_backfill={}",
                    summary.waiting_for_manifest_backfill
                ),
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
        let prep_cli = CliArgs::default();
        if should_run_startup_persistent_prep(&prep_cli) {
            match run_startup_persistent_prep_inner(&state.pool, &state.config, &prep_cli, false)
                .await
            {
                Ok(summary) => {
                    info!(
                        refreshed_manifest_batches = summary.refreshed_manifest_batches,
                        refreshed_manifest_account_rows = summary.refreshed_manifest_account_rows,
                        missing_manifest_files = summary.missing_manifest_files,
                        backfilled_archive_expiries = summary.backfilled_archive_expiries,
                        bootstrapped_hourly_rollups = summary.bootstrapped_hourly_rollups,
                        "startup background prep finished"
                    );
                }
                Err(err) => warn!(error = %err, "startup background prep failed"),
            }
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
    pool_upstream_request_attempt_rows_archived: usize,
    stats_source_snapshot_rows_archived: usize,
    quota_snapshot_rows_archived: usize,
    archive_batches_touched: usize,
    archive_batches_deleted: usize,
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
            || self.pool_upstream_request_attempt_rows_archived > 0
            || self.stats_source_snapshot_rows_archived > 0
            || self.quota_snapshot_rows_archived > 0
            || self.archive_batches_deleted > 0
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
    day_key: Option<String>,
    part_key: Option<String>,
    file_path: String,
    sha256: String,
    row_count: i64,
    upstream_last_activity: Vec<(i64, String)>,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
    archive_expires_at: Option<String>,
    layout: &'static str,
    codec: &'static str,
    writer_version: &'static str,
    cleanup_state: &'static str,
    superseded_by: Option<i64>,
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
struct InvocationRawCompressionFieldCandidate {
    id: i64,
    occurred_at: String,
    raw_path: String,
}

#[derive(Debug, FromRow)]
struct ArchiveBatchFileRow {
    id: i64,
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

#[derive(Debug, FromRow)]
struct InvocationBucketPresenceRow {
    occurred_at: String,
    source: String,
}

#[derive(Debug, FromRow)]
struct ArchiveManifestBatchRow {
    id: i64,
    file_path: String,
}

#[derive(Debug, FromRow)]
struct ArchiveStorageManifestRow {
    id: i64,
    dataset: String,
    layout: String,
    file_path: String,
}

#[derive(Debug, Default)]
struct ArchiveTempCleanupSummary {
    stale_temp_files_removed: usize,
    stale_temp_bytes_removed: u64,
}

#[derive(Debug, Default)]
struct ArchiveStorageVerificationSummary {
    manifest_rows: usize,
    missing_files: usize,
    orphan_files: usize,
    stale_temp_files: usize,
    stale_temp_bytes: u64,
}

#[derive(Debug, Default)]
struct ArchiveBatchPruneSummary {
    expired_archive_batches_deleted: usize,
    legacy_archive_batches_deleted: usize,
}

#[derive(Debug, FromRow)]
struct RawCompressionBacklogAggRow {
    uncompressed_count: i64,
    uncompressed_bytes: Option<i64>,
    oldest_occurred_at: Option<String>,
}

#[derive(Debug, FromRow)]
struct ArchivedAccountLastActivityRow {
    account_id: i64,
    last_activity_at: String,
}

fn dedupe_archive_upstream_last_activity(
    values: impl IntoIterator<Item = (i64, String)>,
) -> Vec<(i64, String)> {
    let mut deduped = BTreeMap::<i64, String>::new();
    for (account_id, last_activity_at) in values {
        deduped
            .entry(account_id)
            .and_modify(|current| {
                if *current < last_activity_at {
                    *current = last_activity_at.clone();
                }
            })
            .or_insert(last_activity_at);
    }
    deduped.into_iter().collect()
}

#[derive(Debug, Default)]
struct ArchiveBackfillSummary {
    scanned_batches: u64,
    updated_accounts: u64,
    hit_budget: bool,
    waiting_for_manifest_backfill: bool,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct HistoricalRollupMaterializationSummary {
    scanned_archive_batches: usize,
    materialized_archive_batches: usize,
    materialized_bucket_count: usize,
    materialized_invocation_batches: usize,
    materialized_forward_proxy_batches: usize,
    last_materialized_bucket_start_epoch: Option<i64>,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct LegacyArchivePruneSummary {
    scanned_archive_batches: usize,
    deleted_archive_batches: usize,
    skipped_unmaterialized_batches: usize,
    skipped_retained_batches: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HistoricalRollupBackfillAlertLevel {
    None,
    Warn,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct HistoricalRollupBackfillSnapshot {
    pub(crate) pending_buckets: u64,
    pub(crate) legacy_archive_pending: u64,
    pub(crate) last_materialized_hour: Option<String>,
    pub(crate) alert_level: HistoricalRollupBackfillAlertLevel,
}

const HOURLY_ROLLUP_DATASET_INVOCATIONS: &str = "codex_invocations";
const HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS: &str = "forward_proxy_attempts";
const HOURLY_ROLLUP_TARGET_INVOCATIONS: &str = "invocation_rollup_hourly";
const HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES: &str = "invocation_failure_rollup_hourly";
const HOURLY_ROLLUP_TARGET_PROXY_PERF: &str = "proxy_perf_stage_hourly";
const HOURLY_ROLLUP_TARGET_PROMPT_CACHE: &str = "prompt_cache_rollup_hourly";
const HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS: &str =
    "prompt_cache_upstream_account_hourly";
const HOURLY_ROLLUP_TARGET_STICKY_KEYS: &str = "upstream_sticky_key_hourly";
const HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS: &str = "forward_proxy_attempt_hourly";
const HISTORICAL_ROLLUP_ARCHIVE_DATASETS: [&str; 2] = [
    HOURLY_ROLLUP_DATASET_INVOCATIONS,
    HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
];
const INVOCATION_HOURLY_ROLLUP_TARGETS: [&str; 6] = [
    HOURLY_ROLLUP_TARGET_INVOCATIONS,
    HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
    HOURLY_ROLLUP_TARGET_PROXY_PERF,
    HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
    HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
    HOURLY_ROLLUP_TARGET_STICKY_KEYS,
];
const PERF_STAGE_TOTAL: &str = "total";
const PERF_STAGE_REQUEST_READ: &str = "requestRead";
const PERF_STAGE_REQUEST_PARSE: &str = "requestParse";
const PERF_STAGE_UPSTREAM_CONNECT: &str = "upstreamConnect";
const PERF_STAGE_UPSTREAM_FIRST_BYTE: &str = "upstreamFirstByte";
const PERF_STAGE_UPSTREAM_STREAM: &str = "upstreamStream";
const PERF_STAGE_RESPONSE_PARSE: &str = "responseParse";
const PERF_STAGE_PERSISTENCE: &str = "persistence";
const HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE: &str = "";

#[derive(Debug, Clone, FromRow)]
struct InvocationHourlySourceRecord {
    id: i64,
    occurred_at: String,
    source: String,
    status: Option<String>,
    detail_level: String,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
    payload: Option<String>,
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
    t_persist_ms: Option<f64>,
}

#[derive(Debug, Clone, FromRow)]
struct ForwardProxyAttemptHourlySourceRecord {
    id: i64,
    proxy_key: String,
    occurred_at: String,
    is_success: i64,
    latency_ms: Option<f64>,
}

#[derive(Debug)]
struct TempSqliteCleanup(PathBuf);

impl Drop for TempSqliteCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn sqlite_url_for_path(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy())
}

#[derive(Debug, Default)]
struct RawCompressionPassSummary {
    files_considered: usize,
    files_compressed: usize,
    bytes_before: u64,
    bytes_after: u64,
    estimated_bytes_after: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawPayloadField {
    Request,
    Response,
}

impl RawPayloadField {
    fn label(self) -> &'static str {
        match self {
            Self::Request => "request_raw_path",
            Self::Response => "response_raw_path",
        }
    }

    fn path_column(self) -> &'static str {
        self.label()
    }

    fn codec_column(self) -> &'static str {
        match self {
            Self::Request => "request_raw_codec",
            Self::Response => "response_raw_codec",
        }
    }
}

#[derive(Debug, Default)]
struct RawCompressionFileOutcome {
    candidate_counted: bool,
    compressed: bool,
    bytes_before: u64,
    bytes_after: u64,
    estimated_bytes_after: u64,
    new_db_path: Option<String>,
    new_codec: Option<String>,
    old_exact_path: Option<PathBuf>,
}

#[derive(Debug, Default)]
struct RawCompressionBacklogSnapshot {
    oldest_uncompressed_age_secs: u64,
    uncompressed_count: u64,
    uncompressed_bytes: u64,
    alert_level: RawCompressionAlertLevel,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum RawCompressionAlertLevel {
    #[default]
    Ok,
    Warn,
    Critical,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
struct ArchiveManifestRefreshSummary {
    pending_batches: usize,
    refreshed_batches: usize,
    account_rows_written: usize,
    missing_files: usize,
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

const CODEX_INVOCATIONS_ARCHIVE_COLUMNS: &str = "id, invoke_id, occurred_at, source, model, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, status, error_message, failure_kind, failure_class, is_actionable, payload, raw_response, cost_estimated, price_version, request_raw_path, request_raw_codec, request_raw_size, request_raw_truncated, request_raw_truncated_reason, response_raw_path, response_raw_codec, response_raw_size, response_raw_truncated, response_raw_truncated_reason, detail_level, detail_pruned_at, detail_prune_reason, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, created_at";
const FORWARD_PROXY_ATTEMPTS_ARCHIVE_COLUMNS: &str =
    "id, proxy_key, occurred_at, is_success, latency_ms, failure_kind, is_probe";
const POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_COLUMNS: &str = "id, invoke_id, occurred_at, endpoint, route_mode, sticky_key, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, requester_ip, started_at, finished_at, status, phase, http_status, failure_kind, error_message, connect_latency_ms, first_byte_latency_ms, stream_latency_ms, upstream_request_id, compact_support_status, compact_support_reason, created_at";
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
    request_raw_codec TEXT NOT NULL DEFAULT 'identity',
    request_raw_size INTEGER,
    request_raw_truncated INTEGER NOT NULL DEFAULT 0,
    request_raw_truncated_reason TEXT,
    response_raw_path TEXT,
    response_raw_codec TEXT NOT NULL DEFAULT 'identity',
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

const POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS archive_db.pool_upstream_request_attempts (
    id INTEGER PRIMARY KEY,
    invoke_id TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    route_mode TEXT NOT NULL,
    sticky_key TEXT,
    upstream_account_id INTEGER,
    upstream_route_key TEXT,
    attempt_index INTEGER NOT NULL,
    distinct_account_index INTEGER NOT NULL,
    same_account_retry_index INTEGER NOT NULL,
    requester_ip TEXT,
    started_at TEXT,
    finished_at TEXT,
    status TEXT NOT NULL,
    phase TEXT,
    http_status INTEGER,
    failure_kind TEXT,
    error_message TEXT,
    connect_latency_ms REAL,
    first_byte_latency_ms REAL,
    stream_latency_ms REAL,
    upstream_request_id TEXT,
    compact_support_status TEXT,
    compact_support_reason TEXT,
    created_at TEXT NOT NULL
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
        "pool_upstream_request_attempts" => ArchiveTableSpec {
            dataset,
            columns: POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_COLUMNS,
            create_sql: POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL,
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

    if !dry_run {
        sync_hourly_rollups_from_live_tables(pool).await?;
        let janitor = cleanup_stale_archive_temp_files(config, false)?;
        if janitor.stale_temp_files_removed > 0 {
            info!(
                ?janitor,
                "archive temp janitor removed stale files before retention"
            );
        }
    }

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
    if !dry_run {
        log_raw_compression_backlog_if_needed(pool, config).await?;
    }

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

    let pool_attempt_archive = archive_timestamped_dataset(
        pool,
        config,
        archive_table_spec("pool_upstream_request_attempts"),
        "SELECT id, occurred_at AS timestamp_value FROM pool_upstream_request_attempts WHERE occurred_at < ?1 ORDER BY occurred_at ASC, id ASC LIMIT ?2",
        shanghai_local_cutoff_string(config.pool_upstream_request_attempts_retention_days),
        dry_run,
    )
    .await?;
    summary.pool_upstream_request_attempt_rows_archived += pool_attempt_archive.0;
    summary.archive_batches_touched += pool_attempt_archive.1;

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

    let archive_ttl_cleanup = cleanup_expired_archive_batches(pool, config, dry_run).await?;
    summary.archive_batches_deleted += archive_ttl_cleanup;

    if should_stop_data_retention_maintenance(shutdown) {
        return Ok(summary);
    }

    if !dry_run && summary.touched_anything() {
        run_best_effort_retention_pragma(
            pool,
            "PRAGMA wal_checkpoint(PASSIVE)",
            "retention wal checkpoint",
        )
        .await?;
        run_best_effort_retention_pragma(pool, "PRAGMA optimize", "retention optimize pragma")
            .await?;
    }

    info!(
        dry_run = summary.dry_run,
        ?summary,
        "data retention maintenance finished"
    );
    Ok(summary)
}

async fn run_best_effort_retention_pragma(
    pool: &Pool<Sqlite>,
    sql: &str,
    description: &'static str,
) -> Result<()> {
    match sqlx::query(sql)
        .execute(pool)
        .await
        .with_context(|| format!("failed to run {description}"))
    {
        Ok(_) => Ok(()),
        Err(err) if is_sqlite_lock_error(&err) => {
            warn!(error = %err, sql, "{description} skipped because the database is busy");
            Ok(())
        }
        Err(err) => Err(err),
    }
}

async fn compress_cold_proxy_raw_payloads(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<RawCompressionPassSummary> {
    compress_cold_proxy_raw_payloads_with_budget(
        pool,
        config,
        raw_path_fallback_root,
        dry_run,
        Some(config.retention_catchup_budget),
    )
    .await
}

async fn compress_cold_proxy_raw_payloads_with_budget(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    catchup_budget: Option<Duration>,
) -> Result<RawCompressionPassSummary> {
    if config.proxy_raw_compression == RawCompressionCodec::None {
        return Ok(RawCompressionPassSummary::default());
    }

    let mut summary = RawCompressionPassSummary::default();
    let started_at = Instant::now();
    let batch_limit = if dry_run {
        i64::MAX as usize
    } else {
        config.retention_batch_rows
    };

    loop {
        let (request_summary, request_rows) = compress_cold_proxy_raw_payload_lane(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            RawPayloadField::Request,
            batch_limit,
        )
        .await?;
        accumulate_raw_compression_summary(&mut summary, request_summary);

        let (response_summary, response_rows) = compress_cold_proxy_raw_payload_lane(
            pool,
            config,
            raw_path_fallback_root,
            dry_run,
            RawPayloadField::Response,
            batch_limit,
        )
        .await?;
        accumulate_raw_compression_summary(&mut summary, response_summary);

        if request_rows == 0 && response_rows == 0 {
            break;
        }
        if dry_run {
            break;
        }
        if let Some(limit) = catchup_budget
            && started_at.elapsed() >= limit
        {
            break;
        }
    }

    Ok(summary)
}

fn accumulate_raw_compression_summary(
    target: &mut RawCompressionPassSummary,
    next: RawCompressionPassSummary,
) {
    target.files_considered += next.files_considered;
    target.files_compressed += next.files_compressed;
    target.bytes_before += next.bytes_before;
    target.bytes_after += next.bytes_after;
    target.estimated_bytes_after += next.estimated_bytes_after;
}

async fn compress_cold_proxy_raw_payload_lane(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
    field: RawPayloadField,
    batch_limit: usize,
) -> Result<(RawCompressionPassSummary, usize)> {
    let cutoff = shanghai_local_cutoff_for_age_secs_string(config.proxy_raw_hot_secs);
    let prune_cutoff = shanghai_local_cutoff_string(config.invocation_success_full_days);
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let success_like_condition = invocation_status_is_success_like_sql("status", "error_message");
    let sql = format!(
        r#"
        SELECT id, occurred_at, {path_column} AS raw_path
        FROM codex_invocations
        WHERE occurred_at < ?1
          AND occurred_at >= ?2
          AND (
            NOT {success_like_condition}
            OR detail_level IS NULL
            OR detail_level != ?3
            OR occurred_at >= ?4
          )
          AND {path_column} IS NOT NULL
          AND {codec_column} = ?5
          AND (
            ?6 IS NULL
            OR occurred_at > ?6
            OR (occurred_at = ?6 AND id > ?7)
          )
        ORDER BY occurred_at ASC, id ASC
        LIMIT ?8
        "#,
        path_column = field.path_column(),
        codec_column = field.codec_column(),
        success_like_condition = success_like_condition,
    );

    let mut summary = RawCompressionPassSummary::default();
    let mut rows_processed = 0usize;
    let mut last_seen_occurred_at: Option<String> = None;
    let mut last_seen_id = 0_i64;

    while summary.files_considered < batch_limit {
        let remaining = (batch_limit - summary.files_considered) as i64;
        let candidates = sqlx::query_as::<_, InvocationRawCompressionFieldCandidate>(&sql)
            .bind(&cutoff)
            .bind(&archive_cutoff)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(RAW_CODEC_IDENTITY)
            .bind(last_seen_occurred_at.as_deref())
            .bind(last_seen_id)
            .bind(remaining.max(1))
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_occurred_at = Some(candidate.occurred_at.clone());
            last_seen_id = candidate.id;
            rows_processed += 1;

            let outcome = match maybe_compress_proxy_raw_path(
                pool,
                candidate.id,
                field.label(),
                Some(candidate.raw_path.as_str()),
                config.proxy_raw_compression,
                raw_path_fallback_root,
                dry_run,
            )
            .await
            {
                Ok(outcome) => outcome,
                Err(err) => {
                    warn!(
                        invocation_id = candidate.id,
                        field = field.label(),
                        error = %err,
                        "failed to cold-compress raw payload file; continuing retention"
                    );
                    continue;
                }
            };

            let next_path = outcome
                .new_db_path
                .clone()
                .unwrap_or_else(|| candidate.raw_path.clone());
            let next_codec = outcome
                .new_codec
                .clone()
                .unwrap_or_else(|| raw_codec_from_path(Some(next_path.as_str())));

            if !dry_run
                && (next_path != candidate.raw_path || !raw_codec_is_identity(Some(&next_codec)))
            {
                let update_sql = format!(
                    "UPDATE codex_invocations SET {path_column} = ?1, {codec_column} = ?2 WHERE id = ?3",
                    path_column = field.path_column(),
                    codec_column = field.codec_column(),
                );
                sqlx::query(&update_sql)
                    .bind(&next_path)
                    .bind(&next_codec)
                    .bind(candidate.id)
                    .execute(pool)
                    .await?;

                if let Some(path) = outcome.old_exact_path.as_deref()
                    && next_path != candidate.raw_path
                {
                    delete_exact_proxy_raw_path(Some(path), raw_path_fallback_root)?;
                }
            }

            if outcome.candidate_counted {
                summary.files_considered += 1;
            }
            if outcome.compressed {
                summary.files_compressed += 1;
            }
            summary.bytes_before += outcome.bytes_before;
            summary.bytes_after += outcome.bytes_after;
            summary.estimated_bytes_after += outcome.estimated_bytes_after;

            if summary.files_considered >= batch_limit {
                break;
            }
        }
    }

    Ok((summary, rows_processed))
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
            new_codec: Some(RAW_CODEC_GZIP.to_string()),
            ..RawCompressionFileOutcome::default()
        });
    }

    let Some(source_path) = locate_existing_proxy_raw_path(raw_path, raw_path_fallback_root) else {
        let existing_compressed =
            locate_existing_proxy_raw_compressed_path(raw_path, raw_path_fallback_root);
        if existing_compressed.is_some() {
            return Ok(RawCompressionFileOutcome {
                new_db_path: Some(raw_payload_compressed_db_path(raw_path)),
                new_codec: Some(RAW_CODEC_GZIP.to_string()),
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
            new_codec: Some(raw_codec_from_path(Some(raw_path))),
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
            new_codec: Some(raw_codec_from_path(Some(raw_path))),
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
            new_codec: Some(RAW_CODEC_GZIP.to_string()),
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
        new_codec: Some(RAW_CODEC_GZIP.to_string()),
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

fn raw_codec_from_path(raw_path: Option<&str>) -> String {
    match raw_path {
        Some(path) if path.ends_with(".gz") => RAW_CODEC_GZIP.to_string(),
        _ => RAW_CODEC_IDENTITY.to_string(),
    }
}

fn raw_codec_is_identity(raw_codec: Option<&str>) -> bool {
    matches!(raw_codec, Some(RAW_CODEC_IDENTITY) | None)
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
    let success_like_condition = invocation_status_is_success_like_sql("status", "error_message");
    if dry_run {
        let sql = format!(
            r#"
            SELECT id, occurred_at, request_raw_path, response_raw_path
            FROM codex_invocations
            WHERE {success_like_condition}
              AND detail_level = ?1
              AND occurred_at < ?2
              AND occurred_at >= ?3
            ORDER BY occurred_at ASC, id ASC
            "#,
            success_like_condition = success_like_condition,
        );
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .fetch_all(pool)
            .await?;
        let mut by_group: BTreeMap<String, usize> = BTreeMap::new();
        for candidate in &candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            *by_group.entry(group_key).or_default() += 1;
        }
        for (group_key, rows) in &by_group {
            info!(
                dataset = spec.dataset,
                archive_group = group_key,
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
            by_group.len(),
            count_existing_proxy_raw_paths(&raw_paths, raw_path_fallback_root),
        ));
    }

    let mut rows_pruned = 0usize;
    let mut archive_batches = 0usize;
    let mut raw_files_removed = 0usize;

    loop {
        let sql = format!(
            r#"
            SELECT id, occurred_at, request_raw_path, response_raw_path
            FROM codex_invocations
            WHERE {success_like_condition}
              AND detail_level = ?1
              AND occurred_at < ?2
              AND occurred_at >= ?3
            ORDER BY occurred_at ASC, id ASC
            LIMIT ?4
            "#,
            success_like_condition = success_like_condition,
        );
        let candidates = sqlx::query_as::<_, InvocationDetailPruneCandidate>(&sql)
            .bind(DETAIL_LEVEL_FULL)
            .bind(&prune_cutoff)
            .bind(&archive_cutoff)
            .bind(config.retention_batch_rows as i64)
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        let mut by_group: BTreeMap<String, Vec<InvocationDetailPruneCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
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
            let mut archive_outcome = match archive_layout_for_dataset(config, spec.dataset) {
                ArchiveBatchLayout::LegacyMonth => {
                    archive_rows_into_month_batch(pool, config, spec, &group_key, &ids).await?
                }
                ArchiveBatchLayout::SegmentV1 => {
                    archive_rows_into_segment_batch(pool, config, spec, &group_key, &ids).await?
                }
            };
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                Some(config.invocation_archive_ttl_days),
            )?;
            let pruned_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
            let mut query = QueryBuilder::<Sqlite>::new(
                "UPDATE codex_invocations SET payload = CASE WHEN json_valid(payload) AND json_extract(payload, '$.upstreamAccountId') IS NOT NULL THEN json_object('upstreamAccountId', json_extract(payload, '$.upstreamAccountId')) ELSE NULL END, raw_response = '', request_raw_path = NULL, request_raw_codec = 'identity', request_raw_size = NULL, request_raw_truncated = 0, request_raw_truncated_reason = NULL, response_raw_path = NULL, response_raw_codec = 'identity', response_raw_size = NULL, response_raw_truncated = 0, response_raw_truncated_reason = NULL, detail_level = ",
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

        let mut by_group: BTreeMap<String, usize> = BTreeMap::new();
        for candidate in &candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            *by_group.entry(group_key).or_default() += 1;
        }
        for (group_key, rows) in &by_group {
            info!(
                dataset = spec.dataset,
                archive_group = group_key,
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
            by_group.len(),
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

        let mut by_group: BTreeMap<String, Vec<InvocationArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let group_key = invocation_archive_group_key(config, &candidate.occurred_at)?;
            by_group.entry(group_key).or_default().push(candidate);
        }

        for (group_key, group) in by_group {
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
            let materialized_rows = group
                .iter()
                .map(|candidate| InvocationHourlySourceRecord {
                    id: candidate.id,
                    occurred_at: candidate.occurred_at.clone(),
                    source: candidate.source.clone(),
                    status: candidate.status.clone(),
                    detail_level: DETAIL_LEVEL_FULL.to_string(),
                    total_tokens: None,
                    cost: None,
                    error_message: None,
                    failure_kind: None,
                    failure_class: None,
                    is_actionable: None,
                    payload: None,
                    t_total_ms: None,
                    t_req_read_ms: None,
                    t_req_parse_ms: None,
                    t_upstream_connect_ms: None,
                    t_upstream_ttfb_ms: None,
                    t_upstream_stream_ms: None,
                    t_resp_parse_ms: None,
                    t_persist_ms: None,
                })
                .collect::<Vec<_>>();
            let mut archive_outcome = match archive_layout_for_dataset(config, spec.dataset) {
                ArchiveBatchLayout::LegacyMonth => {
                    archive_rows_into_month_batch(pool, config, spec, &group_key, &ids).await?
                }
                ArchiveBatchLayout::SegmentV1 => {
                    archive_rows_into_segment_batch(pool, config, spec, &group_key, &ids).await?
                }
            };
            set_archive_batch_coverage_from_local_rows(
                &mut archive_outcome,
                group.iter().map(|candidate| candidate.occurred_at.as_str()),
                None,
            )?;
            archive_outcome.archive_expires_at =
                Some(shanghai_archive_expiry_from_reference_timestamp(
                    &format_utc_iso(Utc::now()),
                    config.invocation_archive_ttl_days,
                )?);
            let mut tx = pool.begin().await?;
            upsert_invocation_rollups(tx.as_mut(), &group).await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            mark_retention_archived_hourly_rollup_targets_tx(
                tx.as_mut(),
                spec.dataset,
                &materialized_rows,
                &[],
            )
            .await?;
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
            "pool_upstream_request_attempts" => {
                r#"
                SELECT strftime('%Y-%m', occurred_at) AS month_key,
                       COUNT(*) AS row_count
                FROM pool_upstream_request_attempts
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
            let month_key =
                archive_timestamped_dataset_month_key(spec.dataset, &candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            rows_archived += group.len();
            archive_batches += 1;
            let ids = group
                .iter()
                .map(|candidate| candidate.id)
                .collect::<Vec<_>>();
            let materialized_forward_proxy_rows = if spec.dataset == "forward_proxy_attempts" {
                group
                    .iter()
                    .map(|candidate| ForwardProxyAttemptHourlySourceRecord {
                        id: candidate.id,
                        proxy_key: String::new(),
                        occurred_at: candidate.timestamp_value.clone(),
                        is_success: 0,
                        latency_ms: None,
                    })
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let mut archive_outcome =
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await?;
            if spec.dataset == "pool_upstream_request_attempts" {
                set_archive_batch_coverage_from_local_rows(
                    &mut archive_outcome,
                    group
                        .iter()
                        .map(|candidate| candidate.timestamp_value.as_str()),
                    Some(config.pool_upstream_request_attempts_archive_ttl_days),
                )?;
            } else {
                set_archive_batch_coverage_from_utc_rows(
                    &mut archive_outcome,
                    group
                        .iter()
                        .map(|candidate| candidate.timestamp_value.as_str()),
                )?;
            }
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            mark_archive_batch_historical_rollups_materialized_tx(
                tx.as_mut(),
                spec.dataset,
                &archive_outcome.file_path,
            )
            .await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            mark_retention_archived_hourly_rollup_targets_tx(
                tx.as_mut(),
                spec.dataset,
                &[],
                &materialized_forward_proxy_rows,
            )
            .await?;
            tx.commit().await?;
        }
    }

    Ok((rows_archived, archive_batches))
}

fn archive_timestamped_dataset_month_key(dataset: &str, timestamp_value: &str) -> Result<String> {
    match dataset {
        "pool_upstream_request_attempts" => shanghai_month_key_from_local_naive(timestamp_value),
        _ => shanghai_month_key_from_utc_naive(timestamp_value),
    }
}

fn set_archive_batch_coverage_from_local_rows<'a>(
    batch: &mut ArchiveBatchOutcome,
    rows: impl Iterator<Item = &'a str>,
    archive_ttl_days: Option<u64>,
) -> Result<()> {
    let values = rows.collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let mut sorted = values.into_iter().map(str::to_string).collect::<Vec<_>>();
    sorted.sort();
    batch.coverage_start_at = sorted.first().cloned();
    batch.coverage_end_at = sorted.last().cloned();
    batch.archive_expires_at = match (batch.coverage_end_at.as_deref(), archive_ttl_days) {
        (Some(coverage_end_at), Some(ttl_days)) => Some(
            shanghai_archive_expiry_from_local_timestamp(coverage_end_at, ttl_days)?,
        ),
        _ => None,
    };
    Ok(())
}

fn set_archive_batch_coverage_from_utc_rows<'a>(
    batch: &mut ArchiveBatchOutcome,
    rows: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let values = rows.collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let mut sorted = values.into_iter().map(str::to_string).collect::<Vec<_>>();
    sorted.sort();
    batch.coverage_start_at = sorted.first().cloned();
    batch.coverage_end_at = sorted.last().cloned();
    batch.archive_expires_at = None;
    Ok(())
}

fn shanghai_archive_expiry_from_local_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = parse_shanghai_local_naive(value)?;
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

fn shanghai_archive_expiry_from_reference_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = match parse_to_utc_datetime(value) {
        Some(value) => value.with_timezone(&Shanghai).naive_local(),
        None => parse_shanghai_local_naive(value)?,
    };
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

fn shanghai_archive_expiry_from_local_naive(
    local: NaiveDateTime,
    archive_ttl_days: u64,
) -> Result<String> {
    let expiry = start_of_local_day(local_naive_to_utc(local, Shanghai), Shanghai)
        + ChronoDuration::days(archive_ttl_days as i64 + 1);
    Ok(format_naive(expiry.with_timezone(&Shanghai).naive_local()))
}

#[derive(Debug, FromRow)]
struct ArchiveExpiryBackfillCandidate {
    id: i64,
    coverage_end_at: String,
}

async fn backfill_invocation_archive_expiries(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<usize> {
    let candidates = sqlx::query_as::<_, ArchiveExpiryBackfillCandidate>(
        r#"
        SELECT id, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND coverage_end_at IS NOT NULL
          AND archive_expires_at IS NULL
          AND historical_rollups_materialized_at IS NOT NULL
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;

    let mut updated = 0usize;
    for candidate in candidates {
        let archive_expires_at = shanghai_archive_expiry_from_reference_timestamp(
            &candidate.coverage_end_at,
            config.invocation_archive_ttl_days,
        )?;
        sqlx::query("UPDATE archive_batches SET archive_expires_at = ?1 WHERE id = ?2")
            .bind(archive_expires_at)
            .bind(candidate.id)
            .execute(pool)
            .await?;
        updated += 1;
    }
    Ok(updated)
}

fn classify_raw_compression_alert(
    oldest_uncompressed_age_secs: u64,
    uncompressed_bytes: u64,
) -> RawCompressionAlertLevel {
    const GIB: u64 = 1024 * 1024 * 1024;
    if oldest_uncompressed_age_secs >= 48 * 3600 || uncompressed_bytes >= 20 * GIB {
        RawCompressionAlertLevel::Critical
    } else if oldest_uncompressed_age_secs >= 24 * 3600 || uncompressed_bytes >= 10 * GIB {
        RawCompressionAlertLevel::Warn
    } else {
        RawCompressionAlertLevel::Ok
    }
}

async fn load_raw_compression_backlog_snapshot(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<RawCompressionBacklogSnapshot> {
    let cutoff = shanghai_local_cutoff_for_age_secs_string(config.proxy_raw_hot_secs);
    let prune_cutoff = shanghai_local_cutoff_string(config.invocation_success_full_days);
    let archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let row = sqlx::query_as::<_, RawCompressionBacklogAggRow>(
        r#"
        SELECT
            COUNT(*) AS uncompressed_count,
            COALESCE(SUM(raw_size), 0) AS uncompressed_bytes,
            MIN(occurred_at) AS oldest_occurred_at
        FROM (
            SELECT occurred_at, COALESCE(request_raw_size, 0) AS raw_size
            FROM codex_invocations
            WHERE occurred_at < ?1
              AND occurred_at >= ?2
              AND (
                status != 'success'
                OR detail_level IS NULL
                OR detail_level != ?3
                OR occurred_at >= ?4
              )
              AND request_raw_path IS NOT NULL
              AND request_raw_codec = 'identity'
            UNION ALL
            SELECT occurred_at, COALESCE(response_raw_size, 0) AS raw_size
            FROM codex_invocations
            WHERE occurred_at < ?1
              AND occurred_at >= ?2
              AND (
                status != 'success'
                OR detail_level IS NULL
                OR detail_level != ?3
                OR occurred_at >= ?4
              )
              AND response_raw_path IS NOT NULL
              AND response_raw_codec = 'identity'
        )
        "#,
    )
    .bind(&cutoff)
    .bind(&archive_cutoff)
    .bind(DETAIL_LEVEL_FULL)
    .bind(&prune_cutoff)
    .fetch_one(pool)
    .await?;

    let oldest_uncompressed_age_secs = row
        .oldest_occurred_at
        .as_deref()
        .map(parse_shanghai_local_naive)
        .transpose()?
        .map(|oldest| {
            let now = Utc::now().with_timezone(&Shanghai).naive_local();
            now.signed_duration_since(oldest).num_seconds().max(0) as u64
        })
        .unwrap_or_default();
    let uncompressed_count = row.uncompressed_count.max(0) as u64;
    let uncompressed_bytes = row.uncompressed_bytes.unwrap_or_default().max(0) as u64;
    let alert_level =
        classify_raw_compression_alert(oldest_uncompressed_age_secs, uncompressed_bytes);
    Ok(RawCompressionBacklogSnapshot {
        oldest_uncompressed_age_secs,
        uncompressed_count,
        uncompressed_bytes,
        alert_level,
    })
}

async fn log_raw_compression_backlog_if_needed(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    let snapshot = load_raw_compression_backlog_snapshot(pool, config).await?;
    match snapshot.alert_level {
        RawCompressionAlertLevel::Ok => {}
        RawCompressionAlertLevel::Warn => {
            warn!(
                oldest_uncompressed_age_secs = snapshot.oldest_uncompressed_age_secs,
                uncompressed_count = snapshot.uncompressed_count,
                uncompressed_bytes = snapshot.uncompressed_bytes,
                alert_level = "warn",
                "raw compression backlog is above warning threshold"
            );
        }
        RawCompressionAlertLevel::Critical => {
            error!(
                oldest_uncompressed_age_secs = snapshot.oldest_uncompressed_age_secs,
                uncompressed_count = snapshot.uncompressed_count,
                uncompressed_bytes = snapshot.uncompressed_bytes,
                alert_level = "critical",
                "raw compression backlog is above critical threshold"
            );
        }
    }
    Ok(())
}

fn archive_file_is_stale_temp(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(is_archive_temp_file_name)
        .unwrap_or(false)
}

fn archive_temp_file_is_old_enough(path: &Path) -> bool {
    fs::metadata(path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.elapsed().ok())
        .map(|age| age.as_secs() >= DEFAULT_ARCHIVE_TEMP_MIN_AGE_SECS)
        .unwrap_or(false)
}

fn archive_file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or_default()
}

fn cleanup_stale_archive_temp_files(
    config: &AppConfig,
    dry_run: bool,
) -> Result<ArchiveTempCleanupSummary> {
    let archive_root = resolved_archive_dir(config);
    let mut files = Vec::new();
    collect_archive_file_paths(&archive_root, &mut files)?;
    let mut summary = ArchiveTempCleanupSummary::default();
    for file_path in files {
        if !archive_file_is_stale_temp(&file_path) || !archive_temp_file_is_old_enough(&file_path) {
            continue;
        }
        let file_size = archive_file_size(&file_path);
        if dry_run {
            summary.stale_temp_files_removed += 1;
            summary.stale_temp_bytes_removed += file_size;
            continue;
        }
        match fs::remove_file(&file_path) {
            Ok(_) => {
                summary.stale_temp_files_removed += 1;
                summary.stale_temp_bytes_removed += file_size;
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(
                    file_path = %file_path.display(),
                    error = %err,
                    "failed to remove stale archive temp file"
                );
            }
        }
    }
    Ok(summary)
}

async fn verify_archive_storage(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<ArchiveStorageVerificationSummary> {
    let manifest_rows = sqlx::query_as::<_, ArchiveStorageManifestRow>(
        r#"
        SELECT id, dataset, layout, file_path
        FROM archive_batches
        WHERE status = ?1
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    let mut summary = ArchiveStorageVerificationSummary {
        manifest_rows: manifest_rows.len(),
        ..ArchiveStorageVerificationSummary::default()
    };
    let referenced_paths = manifest_rows
        .iter()
        .map(|row| normalize_path_for_compare(Path::new(&row.file_path)))
        .collect::<HashSet<_>>();
    for row in &manifest_rows {
        if !Path::new(&row.file_path).exists() {
            summary.missing_files += 1;
            warn!(
                archive_batch_id = row.id,
                dataset = row.dataset,
                layout = row.layout,
                file_path = row.file_path,
                "archive manifest points to a missing file"
            );
        }
    }

    let archive_root = resolved_archive_dir(config);
    let mut files = Vec::new();
    collect_archive_file_paths(&archive_root, &mut files)?;
    for file_path in files {
        let normalized = normalize_path_for_compare(&file_path);
        if archive_file_is_stale_temp(&file_path) {
            summary.stale_temp_files += 1;
            summary.stale_temp_bytes += archive_file_size(&file_path);
            continue;
        }
        if !referenced_paths.contains(&normalized) {
            summary.orphan_files += 1;
        }
    }
    Ok(summary)
}

#[derive(Debug, FromRow)]
struct ArchiveBatchCleanupCandidate {
    id: i64,
    dataset: String,
    file_path: String,
    historical_rollups_materialized_at: Option<String>,
    coverage_end_at: Option<String>,
}

async fn cleanup_expired_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<usize> {
    if !dry_run {
        backfill_invocation_archive_expiries(pool, config).await?;
    }
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let candidates = sqlx::query_as::<_, ArchiveBatchCleanupCandidate>(
        r#"
        SELECT id, dataset, file_path, historical_rollups_materialized_at, coverage_end_at
        FROM archive_batches
        WHERE status = ?1
          AND archive_expires_at IS NOT NULL
          AND archive_expires_at < ?2
        ORDER BY archive_expires_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&cutoff)
    .fetch_all(pool)
    .await?;

    let mut eligible_candidates = Vec::new();
    for candidate in candidates {
        if HISTORICAL_ROLLUP_ARCHIVE_DATASETS.contains(&candidate.dataset.as_str())
            && candidate.historical_rollups_materialized_at.is_none()
        {
            continue;
        }
        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
            if candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| coverage_end_at >= invocation_archive_cutoff.as_str())
                .unwrap_or(true)
            {
                continue;
            }
        }
        eligible_candidates.push(candidate);
    }

    if dry_run {
        for candidate in &eligible_candidates {
            info!(
                dataset = candidate.dataset,
                file_path = candidate.file_path,
                "retention dry-run planned archive batch cleanup"
            );
        }
        return Ok(eligible_candidates.len());
    }

    let mut deleted = 0usize;
    for candidate in eligible_candidates {
        match fs::remove_file(&candidate.file_path) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(
                    dataset = candidate.dataset,
                    file_path = candidate.file_path,
                    error = %err,
                    "failed to remove expired archive batch file; keeping manifest"
                );
                continue;
            }
        }

        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = ?1")
            .bind(candidate.id)
            .execute(tx.as_mut())
            .await?;
        sqlx::query("DELETE FROM archive_batches WHERE id = ?1")
            .bind(candidate.id)
            .execute(tx.as_mut())
            .await?;
        sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2",
        )
        .bind(&candidate.dataset)
        .bind(&candidate.file_path)
        .execute(tx.as_mut())
        .await?;
        tx.commit().await?;
        deleted += 1;
    }

    Ok(deleted)
}

#[derive(Debug, FromRow)]
struct HistoricalRollupPendingArchiveBatchRow {
    dataset: String,
    month_key: String,
    file_path: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
}

#[derive(Debug, FromRow)]
struct LegacyArchivePruneCandidateRow {
    id: i64,
    dataset: String,
    file_path: String,
    historical_rollups_materialized_at: Option<String>,
    coverage_end_at: Option<String>,
}

fn estimate_historical_rollup_pending_bucket_count(
    row: &HistoricalRollupPendingArchiveBatchRow,
) -> u64 {
    if let (Some(start), Some(end)) = (&row.coverage_start_at, &row.coverage_end_at)
        && let (Ok(start_local), Ok(end_local)) = (
            parse_shanghai_local_naive(start),
            parse_shanghai_local_naive(end),
        )
    {
        let start_utc = local_naive_to_utc(start_local, Shanghai);
        let end_utc = local_naive_to_utc(end_local, Shanghai);
        let secs = (end_utc.timestamp() - start_utc.timestamp()).max(0);
        return ((secs + 3_599) / 3_600).max(1) as u64;
    }

    let Ok(start_date) = NaiveDate::parse_from_str(&format!("{}-01", row.month_key), "%Y-%m-%d")
    else {
        return 0;
    };
    let (next_year, next_month) = if start_date.month() == 12 {
        (start_date.year() + 1, 1)
    } else {
        (start_date.year(), start_date.month() + 1)
    };
    let Some(next_month_date) = NaiveDate::from_ymd_opt(next_year, next_month, 1) else {
        return 0;
    };
    let Some(start_naive) = start_date.and_hms_opt(0, 0, 0) else {
        return 0;
    };
    let Some(end_naive) = next_month_date.and_hms_opt(0, 0, 0) else {
        return 0;
    };
    let start_utc = local_naive_to_utc(start_naive, Shanghai);
    let end_utc = local_naive_to_utc(end_naive, Shanghai);
    ((end_utc.timestamp() - start_utc.timestamp()).max(0) / 3_600) as u64
}

async fn count_historical_rollup_archive_batches(
    pool: &Pool<Sqlite>,
    pending_only: bool,
) -> Result<i64> {
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM archive_batches WHERE status = ");
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(" AND dataset IN (");
    {
        let mut separated = query.separated(", ");
        for dataset in HISTORICAL_ROLLUP_ARCHIVE_DATASETS {
            separated.push_bind(dataset);
        }
    }
    query.push(")");
    if pending_only {
        query.push(" AND historical_rollups_materialized_at IS NULL");
    }
    Ok(query.build_query_scalar::<i64>().fetch_one(pool).await?)
}

fn historical_rollup_materialized_bucket_targets() -> [&'static str; 7] {
    [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
        HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
    ]
}

async fn load_latest_materialized_legacy_invocation_rollup_bucket_epoch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<Option<i64>> {
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let latest_coverage_end_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT MAX(coverage_end_at)
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND historical_rollups_materialized_at IS NOT NULL
          AND coverage_end_at IS NOT NULL
          AND coverage_end_at < ?2
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(invocation_archive_cutoff)
    .fetch_one(pool)
    .await?;

    Ok(latest_coverage_end_at.and_then(|coverage_end_at| {
        parse_shanghai_local_naive(&coverage_end_at)
            .ok()
            .and_then(|naive| {
                let bucket_start_epoch =
                    align_bucket_epoch(local_naive_to_utc(naive, Shanghai).timestamp(), 3_600, 0);
                Utc.timestamp_opt(bucket_start_epoch, 0)
                    .single()
                    .map(|_| bucket_start_epoch)
            })
    }))
}

async fn count_materialized_historical_rollup_buckets(pool: &Pool<Sqlite>) -> Result<i64> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COUNT(*) FROM hourly_rollup_materialized_buckets WHERE target IN (",
    );
    {
        let mut separated = query.separated(", ");
        for target in historical_rollup_materialized_bucket_targets() {
            separated.push_bind(target);
        }
    }
    query.push(")");
    Ok(query.build_query_scalar::<i64>().fetch_one(pool).await?)
}

pub(crate) async fn load_historical_rollup_backfill_snapshot(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<HistoricalRollupBackfillSnapshot> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT dataset, month_key, file_path, coverage_start_at, coverage_end_at \
         FROM archive_batches WHERE status = ",
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(" AND historical_rollups_materialized_at IS NULL AND dataset IN (");
    {
        let mut separated = query.separated(", ");
        for dataset in HISTORICAL_ROLLUP_ARCHIVE_DATASETS {
            separated.push_bind(dataset);
        }
    }
    query.push(") ORDER BY month_key ASC, id ASC");
    let pending_rows = query
        .build_query_as::<HistoricalRollupPendingArchiveBatchRow>()
        .fetch_all(pool)
        .await?;
    let pending_buckets = pending_rows
        .iter()
        .filter(|row| Path::new(&row.file_path).exists())
        .map(estimate_historical_rollup_pending_bucket_count)
        .sum::<u64>();
    let legacy_archive_pending = pending_rows
        .iter()
        .filter(|row| Path::new(&row.file_path).exists())
        .count() as u64;
    let legacy_invocation_pending = pending_rows
        .iter()
        .filter(|row| Path::new(&row.file_path).exists())
        .any(|row| row.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS);
    let last_materialized_hour =
        load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config)
            .await?
            .and_then(|epoch| Utc.timestamp_opt(epoch, 0).single())
            .map(format_utc_iso);
    let alert_level = if legacy_archive_pending == 0 {
        HistoricalRollupBackfillAlertLevel::None
    } else if legacy_invocation_pending {
        HistoricalRollupBackfillAlertLevel::Critical
    } else {
        HistoricalRollupBackfillAlertLevel::Warn
    };

    Ok(HistoricalRollupBackfillSnapshot {
        pending_buckets,
        legacy_archive_pending,
        last_materialized_hour,
        alert_level,
    })
}

async fn materialize_historical_rollups(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<HistoricalRollupMaterializationSummary> {
    let scanned_archive_batches = count_historical_rollup_archive_batches(pool, false).await?;
    let pending_snapshot = load_historical_rollup_backfill_snapshot(pool, config).await?;
    if dry_run {
        return Ok(HistoricalRollupMaterializationSummary {
            scanned_archive_batches: scanned_archive_batches as usize,
            materialized_archive_batches: pending_snapshot.legacy_archive_pending as usize,
            materialized_bucket_count: pending_snapshot.pending_buckets as usize,
            materialized_invocation_batches: 0,
            materialized_forward_proxy_batches: 0,
            last_materialized_bucket_start_epoch:
                load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config).await?,
        });
    }

    let mut tx = pool.begin().await?;
    let materialized_invocation_batches =
        replay_invocation_archives_into_hourly_rollups_tx(tx.as_mut()).await?;
    let materialized_forward_proxy_batches =
        replay_forward_proxy_archives_into_hourly_rollups_tx(tx.as_mut()).await?;
    loop {
        let updated = replay_live_invocation_hourly_rollups_tx(tx.as_mut()).await?;
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated = replay_live_forward_proxy_attempt_hourly_rollups_tx(tx.as_mut()).await?;
        if updated == 0 {
            break;
        }
    }
    tx.commit().await?;

    Ok(HistoricalRollupMaterializationSummary {
        scanned_archive_batches: scanned_archive_batches as usize,
        materialized_archive_batches: (materialized_invocation_batches
            + materialized_forward_proxy_batches) as usize,
        materialized_bucket_count: count_materialized_historical_rollup_buckets(pool).await?
            as usize,
        materialized_invocation_batches: materialized_invocation_batches as usize,
        materialized_forward_proxy_batches: materialized_forward_proxy_batches as usize,
        last_materialized_bucket_start_epoch:
            load_latest_materialized_legacy_invocation_rollup_bucket_epoch(pool, config).await?,
    })
}

async fn prune_legacy_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<LegacyArchivePruneSummary> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, dataset, file_path, historical_rollups_materialized_at, coverage_end_at \
         FROM archive_batches WHERE status = ",
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(" AND dataset IN (");
    {
        let mut separated = query.separated(", ");
        for dataset in HISTORICAL_ROLLUP_ARCHIVE_DATASETS {
            separated.push_bind(dataset);
        }
    }
    query.push(") AND COALESCE(layout, ");
    query.push_bind(ARCHIVE_LAYOUT_LEGACY_MONTH);
    query.push(") = ");
    query.push_bind(ARCHIVE_LAYOUT_LEGACY_MONTH);
    query.push(" ORDER BY month_key ASC, id ASC");
    let candidates = query
        .build_query_as::<LegacyArchivePruneCandidateRow>()
        .fetch_all(pool)
        .await?;

    let pending_account_count = count_upstream_accounts_missing_last_activity(pool).await?;
    let invocation_archive_cutoff = shanghai_local_cutoff_string(config.invocation_max_days);
    let mut summary = LegacyArchivePruneSummary {
        scanned_archive_batches: candidates.len(),
        ..LegacyArchivePruneSummary::default()
    };

    for candidate in candidates {
        let file_missing = !Path::new(&candidate.file_path).exists();

        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS && pending_account_count > 0 {
            summary.skipped_unmaterialized_batches += 1;
            continue;
        }

        if !file_missing && candidate.historical_rollups_materialized_at.is_none() {
            summary.skipped_unmaterialized_batches += 1;
            continue;
        }

        if candidate.dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS
            && candidate
                .coverage_end_at
                .as_deref()
                .map(|coverage_end_at| coverage_end_at >= invocation_archive_cutoff.as_str())
                .unwrap_or(true)
        {
            summary.skipped_retained_batches += 1;
            continue;
        }

        if dry_run {
            info!(
                dataset = candidate.dataset,
                file_path = candidate.file_path,
                "maintenance dry-run planned legacy archive prune"
            );
            summary.deleted_archive_batches += 1;
            continue;
        }

        match fs::remove_file(&candidate.file_path) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(
                    dataset = candidate.dataset,
                    file_path = candidate.file_path,
                    error = %err,
                    "failed to remove legacy archive batch file; keeping metadata"
                );
                summary.skipped_unmaterialized_batches += 1;
                continue;
            }
        }

        let mut tx = pool.begin().await?;
        sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = ?1")
            .bind(candidate.id)
            .execute(tx.as_mut())
            .await?;
        sqlx::query(
            "DELETE FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2",
        )
        .bind(&candidate.dataset)
        .bind(&candidate.file_path)
        .execute(tx.as_mut())
        .await?;
        sqlx::query("DELETE FROM archive_batches WHERE id = ?1")
            .bind(candidate.id)
            .execute(tx.as_mut())
            .await?;
        tx.commit().await?;
        summary.deleted_archive_batches += 1;
    }

    Ok(summary)
}

async fn prune_archive_batches(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: bool,
) -> Result<ArchiveBatchPruneSummary> {
    let expired_archive_batches_deleted =
        cleanup_expired_archive_batches(pool, config, dry_run).await?;
    let legacy_summary = prune_legacy_archive_batches(pool, config, dry_run).await?;
    Ok(ArchiveBatchPruneSummary {
        expired_archive_batches_deleted,
        legacy_archive_batches_deleted: legacy_summary.deleted_archive_batches,
    })
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
            let mut archive_outcome =
                archive_rows_into_month_batch(pool, config, spec, &month_key, &ids).await?;
            set_archive_batch_coverage_from_utc_rows(
                &mut archive_outcome,
                group
                    .iter()
                    .map(|candidate| candidate.timestamp_value.as_str()),
            )?;
            let mut tx = pool.begin().await?;
            upsert_archive_batch_manifest(tx.as_mut(), &archive_outcome).await?;
            delete_rows_by_ids(tx.as_mut(), spec.dataset, &ids).await?;
            tx.commit().await?;
        }
    }

    Ok((rows_archived, archive_batches))
}

async fn refresh_archive_upstream_activity_manifest(
    pool: &Pool<Sqlite>,
    dry_run: bool,
) -> Result<ArchiveManifestRefreshSummary> {
    let batches = sqlx::query_as::<_, ArchiveManifestBatchRow>(
        r#"
        SELECT id, file_path
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND upstream_activity_manifest_refreshed_at IS NULL
        ORDER BY month_key DESC, day_key DESC, part_key DESC, created_at DESC, id DESC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;

    let mut summary = ArchiveManifestRefreshSummary {
        pending_batches: batches.len(),
        ..ArchiveManifestRefreshSummary::default()
    };

    for batch in batches {
        let archive_path = PathBuf::from(&batch.file_path);
        if !archive_path.exists() {
            summary.missing_files += 1;
            warn!(
                archive_batch_id = batch.id,
                file_path = %archive_path.display(),
                "archive upstream activity manifest rebuild skipped missing archive file and will retry later"
            );
            continue;
        }

        let values = match load_archive_upstream_activity_from_file(&archive_path).await {
            Ok(values) => values,
            Err(err) => {
                warn!(
                    archive_batch_id = batch.id,
                    file_path = %archive_path.display(),
                    error = %err,
                    "archive upstream activity manifest rebuild failed and will retry later"
                );
                continue;
            }
        };
        let deduped_values = dedupe_archive_upstream_last_activity(values);
        summary.refreshed_batches += 1;
        summary.account_rows_written += deduped_values.len();
        if dry_run {
            continue;
        }

        let mut tx = pool.begin().await?;
        write_archive_batch_upstream_activity(tx.as_mut(), batch.id, &deduped_values).await?;
        tx.commit().await?;
    }

    Ok(summary)
}

async fn load_archive_upstream_activity_from_file(
    archive_path: &Path,
) -> Result<Vec<(i64, String)>> {
    let temp_path = PathBuf::from(format!(
        "{}.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    inflate_gzip_sqlite_file(archive_path, &temp_path)?;

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

    let rows = sqlx::query_as::<_, ArchivedAccountLastActivityRow>(
        r#"
        SELECT account_id, MAX(occurred_at) AS last_activity_at
        FROM (
            SELECT
                CASE
                    WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER)
                END AS account_id,
                occurred_at
            FROM codex_invocations
        )
        WHERE account_id IS NOT NULL
        GROUP BY account_id
        "#,
    )
    .fetch_all(&archive_pool)
    .await?;

    archive_pool.close().await;
    drop(temp_cleanup);

    Ok(dedupe_archive_upstream_last_activity(
        rows.into_iter()
            .map(|row| (row.account_id, row.last_activity_at)),
    ))
}

async fn backfill_upstream_account_last_activity_from_archives(
    pool: &Pool<Sqlite>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<ArchiveBackfillSummary> {
    let total_pending_accounts = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE last_activity_at IS NULL
          AND last_activity_archive_backfill_completed = 0
        "#,
    )
    .fetch_one(pool)
    .await?
    .max(0) as u64;
    if total_pending_accounts == 0 {
        return Ok(ArchiveBackfillSummary::default());
    }

    let pending_fetch_limit = scan_limit.unwrap_or(total_pending_accounts).max(1) as i64;
    let pending_account_ids = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_accounts
        WHERE last_activity_at IS NULL
          AND last_activity_archive_backfill_completed = 0
        ORDER BY id ASC
        LIMIT ?1
        "#,
    )
    .bind(pending_fetch_limit)
    .fetch_all(pool)
    .await?;

    let pending_manifest_batches = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND upstream_activity_manifest_refreshed_at IS NULL
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_one(pool)
    .await?;
    if pending_manifest_batches > 0 {
        return Ok(ArchiveBackfillSummary {
            waiting_for_manifest_backfill: true,
            ..ArchiveBackfillSummary::default()
        });
    }

    let scanned_batches = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_one(pool)
    .await?
    .max(0) as u64;
    if scanned_batches == 0 {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_archive_backfill_completed = 1
            WHERE last_activity_at IS NULL
              AND last_activity_archive_backfill_completed = 0
            "#,
        )
        .execute(pool)
        .await?;
        return Ok(ArchiveBackfillSummary::default());
    }

    let pending = pending_account_ids.into_iter().collect::<HashSet<_>>();
    let mut recovered = HashMap::<i64, String>::new();
    let pending_chunks = pending_account_ids_chunks(&pending);
    let started_at = Instant::now();
    let mut processed_account_ids = HashSet::new();
    let mut hit_budget = total_pending_accounts > pending.len() as u64;

    for (chunk_idx, account_ids) in pending_chunks.iter().enumerate() {
        if startup_backfill_budget_reached(
            started_at,
            processed_account_ids.len() as u64,
            None,
            max_elapsed,
        ) {
            hit_budget = true;
            break;
        }
        for account_id in account_ids {
            processed_account_ids.insert(*account_id);
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT account_id, MAX(last_activity_at) AS last_activity_at FROM archive_batch_upstream_activity WHERE account_id IN (",
        );
        {
            let mut separated = query.separated(", ");
            for account_id in account_ids {
                separated.push_bind(account_id);
            }
        }
        query.push(") GROUP BY account_id");
        for row in query
            .build_query_as::<ArchivedAccountLastActivityRow>()
            .fetch_all(pool)
            .await?
        {
            recovered
                .entry(row.account_id)
                .and_modify(|current| {
                    if *current < row.last_activity_at {
                        *current = row.last_activity_at.clone();
                    }
                })
                .or_insert(row.last_activity_at);
        }

        if chunk_idx + 1 < pending_chunks.len()
            && startup_backfill_budget_reached(
                started_at,
                processed_account_ids.len() as u64,
                None,
                max_elapsed,
            )
        {
            hit_budget = true;
            break;
        }
    }

    if recovered.is_empty() {
        let processed = processed_account_ids.iter().copied().collect::<Vec<_>>();
        if !processed.is_empty() {
            mark_archive_backfill_completed_for_accounts(pool, &processed).await?;
        }
        return Ok(ArchiveBackfillSummary {
            scanned_batches: processed_account_ids.len() as u64,
            updated_accounts: 0,
            hit_budget,
            waiting_for_manifest_backfill: false,
        });
    }

    let unresolved: Vec<i64> = processed_account_ids
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
            SET last_activity_at = CASE
                    WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                    ELSE last_activity_at
                END,
                last_activity_archive_backfill_completed = 1
            WHERE id = ?2
            "#,
        )
        .bind(occurred_at)
        .bind(account_id)
        .execute(tx.as_mut())
        .await?;
    }
    if !unresolved.is_empty() {
        mark_archive_backfill_completed_for_accounts_tx(tx.as_mut(), &unresolved).await?;
    }
    tx.commit().await?;

    Ok(ArchiveBackfillSummary {
        scanned_batches: processed_account_ids.len() as u64,
        updated_accounts,
        hit_budget,
        waiting_for_manifest_backfill: false,
    })
}

fn pending_account_ids_chunks(pending: &HashSet<i64>) -> Vec<Vec<i64>> {
    pending
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect()
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
        if spec.dataset == "codex_invocations" {
            ensure_codex_invocations_archive_schema(&mut conn).await?;
        } else if spec.dataset == "pool_upstream_request_attempts" {
            ensure_pool_upstream_request_attempts_archive_schema(&mut conn).await?;
        }

        let upstream_last_activity = if spec.dataset == "codex_invocations" {
            let mut rows = Vec::new();
            for chunk in ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
                let mut query = QueryBuilder::<Sqlite>::new(
                    "SELECT account_id, MAX(occurred_at) AS last_activity_at FROM (SELECT CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS account_id, occurred_at FROM main.codex_invocations WHERE id IN (",
                );
                {
                    let mut separated = query.separated(", ");
                    for id in chunk {
                        separated.push_bind(id);
                    }
                }
                query.push(")) WHERE account_id IS NOT NULL GROUP BY account_id");
                rows.extend(
                    query
                        .build_query_as::<ArchivedAccountLastActivityRow>()
                        .fetch_all(&mut *conn)
                        .await?,
                );
            }
            dedupe_archive_upstream_last_activity(
                rows.into_iter()
                    .map(|row| (row.account_id, row.last_activity_at)),
            )
        } else {
            Vec::new()
        };

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
        Ok::<(i64, Vec<(i64, String)>), anyhow::Error>((row_count, upstream_last_activity))
    }
    .await;

    let (result, upstream_last_activity) = match row_count {
        Ok(values) => values,
        Err(err) => {
            let _ = fs::remove_file(&work_path);
            let _ = fs::remove_file(&temp_gzip_path);
            return Err(err);
        }
    };

    if let Err(err) = deflate_sqlite_file_to_gzip(&work_path, &temp_gzip_path) {
        let _ = fs::remove_file(&work_path);
        let _ = fs::remove_file(&temp_gzip_path);
        return Err(err);
    }
    if let Err(err) = fs::rename(&temp_gzip_path, &final_path).with_context(|| {
        format!(
            "failed to move archive batch into place: {} -> {}",
            temp_gzip_path.display(),
            final_path.display()
        )
    }) {
        let _ = fs::remove_file(&work_path);
        let _ = fs::remove_file(&temp_gzip_path);
        return Err(err);
    }
    let _ = fs::remove_file(&work_path);

    let sha256 = sha256_hex_file(&final_path)?;
    Ok(ArchiveBatchOutcome {
        dataset: spec.dataset,
        month_key: month_key.to_string(),
        day_key: None,
        part_key: None,
        file_path: final_path.to_string_lossy().to_string(),
        sha256,
        row_count: result,
        upstream_last_activity,
        coverage_start_at: None,
        coverage_end_at: None,
        archive_expires_at: None,
        layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
        codec: ARCHIVE_FILE_CODEC_GZIP,
        writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    })
}

async fn archive_rows_into_segment_batch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    day_key: &str,
    ids: &[i64],
) -> Result<ArchiveBatchOutcome> {
    if ids.is_empty() {
        bail!("archive segment requires at least one row id");
    }
    if spec.dataset != "codex_invocations" {
        bail!("archive segment writer only supports codex_invocations");
    }
    let month_key = archive_month_key_from_day_key(day_key)?;
    let part_key = next_archive_segment_part_key(pool, spec.dataset, day_key).await?;
    let final_path = archive_segment_file_path(
        config,
        spec.dataset,
        day_key,
        &part_key,
        config.invocation_archive_codec,
    )?;
    if let Some(parent) = final_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create archive directory: {}", parent.display()))?;
    }

    let suffix = retention_temp_suffix();
    let work_path = PathBuf::from(format!(
        "{}.{}.partial.sqlite",
        final_path.display(),
        suffix
    ));
    let temp_gzip_path = PathBuf::from(format!("{}.{}.tmp", final_path.display(), suffix));
    let _temp_cleanup = TempSqliteCleanup(work_path.clone());
    let _gzip_cleanup = TempSqliteCleanup(temp_gzip_path.clone());
    ensure_sqlite_file_initialized(&work_path).await?;

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
        ensure_codex_invocations_archive_schema(&mut conn).await?;

        let mut upstream_last_activity = Vec::new();
        for chunk in ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
            let mut query = QueryBuilder::<Sqlite>::new(
                "SELECT account_id, MAX(occurred_at) AS last_activity_at FROM (SELECT CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS account_id, occurred_at FROM main.codex_invocations WHERE id IN (",
            );
            {
                let mut separated = query.separated(", ");
                for id in chunk {
                    separated.push_bind(id);
                }
            }
            query.push(")) WHERE account_id IS NOT NULL GROUP BY account_id");
            upstream_last_activity.extend(
                query
                    .build_query_as::<ArchivedAccountLastActivityRow>()
                    .fetch_all(&mut *conn)
                    .await?,
            );
        }
        let upstream_last_activity = dedupe_archive_upstream_last_activity(
            upstream_last_activity
                .into_iter()
                .map(|row| (row.account_id, row.last_activity_at)),
        );

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
                "failed to copy rows into archive segment for {}",
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
        Ok::<(i64, Vec<(i64, String)>), anyhow::Error>((row_count, upstream_last_activity))
    }
    .await?;

    deflate_sqlite_file_to_gzip(&work_path, &temp_gzip_path)?;
    fs::rename(&temp_gzip_path, &final_path).with_context(|| {
        format!(
            "failed to move archive segment into place: {} -> {}",
            temp_gzip_path.display(),
            final_path.display()
        )
    })?;

    let sha256 = sha256_hex_file(&final_path)?;
    Ok(ArchiveBatchOutcome {
        dataset: spec.dataset,
        month_key,
        day_key: Some(day_key.to_string()),
        part_key: Some(part_key),
        file_path: final_path.to_string_lossy().to_string(),
        sha256,
        row_count: row_count.0,
        upstream_last_activity: row_count.1,
        coverage_start_at: None,
        coverage_end_at: None,
        archive_expires_at: None,
        layout: ARCHIVE_LAYOUT_SEGMENT_V1,
        codec: config.invocation_archive_codec.as_str(),
        writer_version: ARCHIVE_WRITER_VERSION_SEGMENT_V1,
        cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
        superseded_by: None,
    })
}

async fn upsert_archived_upstream_last_activity(
    tx: &mut sqlx::SqliteConnection,
    values: &[(i64, String)],
) -> Result<()> {
    let deduped_values = dedupe_archive_upstream_last_activity(values.iter().cloned());
    for (account_id, occurred_at) in &deduped_values {
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
        .bind(occurred_at)
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
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
            day_key,
            part_key,
            file_path,
            sha256,
            row_count,
            status,
            layout,
            codec,
            writer_version,
            cleanup_state,
            superseded_by,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, datetime('now'))
        ON CONFLICT(dataset, month_key, file_path) DO UPDATE SET
            day_key = excluded.day_key,
            part_key = excluded.part_key,
            sha256 = excluded.sha256,
            row_count = excluded.row_count,
            status = excluded.status,
            layout = excluded.layout,
            codec = excluded.codec,
            writer_version = excluded.writer_version,
            cleanup_state = excluded.cleanup_state,
            superseded_by = excluded.superseded_by,
            coverage_start_at = excluded.coverage_start_at,
            coverage_end_at = excluded.coverage_end_at,
            archive_expires_at = excluded.archive_expires_at,
            created_at = datetime('now')
        "#,
    )
    .bind(batch.dataset)
    .bind(&batch.month_key)
    .bind(batch.day_key.as_deref())
    .bind(batch.part_key.as_deref())
    .bind(&batch.file_path)
    .bind(&batch.sha256)
    .bind(batch.row_count)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(batch.layout)
    .bind(batch.codec)
    .bind(batch.writer_version)
    .bind(batch.cleanup_state)
    .bind(batch.superseded_by)
    .bind(batch.coverage_start_at.as_deref())
    .bind(batch.coverage_end_at.as_deref())
    .bind(batch.archive_expires_at.as_deref())
    .execute(&mut *tx)
    .await?;
    let deduped_upstream_last_activity =
        dedupe_archive_upstream_last_activity(batch.upstream_last_activity.iter().cloned());
    let archive_batch_id = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM archive_batches
        WHERE dataset = ?1
          AND month_key = ?2
          AND file_path = ?3
        LIMIT 1
        "#,
    )
    .bind(batch.dataset)
    .bind(&batch.month_key)
    .bind(&batch.file_path)
    .fetch_one(&mut *tx)
    .await?;
    if batch.dataset == "codex_invocations" {
        write_archive_batch_upstream_activity(
            tx,
            archive_batch_id,
            &deduped_upstream_last_activity,
        )
        .await?;
    }
    if batch.dataset == "codex_invocations" && !deduped_upstream_last_activity.is_empty() {
        upsert_archived_upstream_last_activity(tx, &deduped_upstream_last_activity).await?;
    }
    Ok(())
}

async fn write_archive_batch_upstream_activity(
    tx: &mut sqlx::SqliteConnection,
    archive_batch_id: i64,
    values: &[(i64, String)],
) -> Result<()> {
    let deduped_values = dedupe_archive_upstream_last_activity(values.iter().cloned());
    sqlx::query("DELETE FROM archive_batch_upstream_activity WHERE archive_batch_id = ?1")
        .bind(archive_batch_id)
        .execute(&mut *tx)
        .await?;
    for chunk in deduped_values.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut insert = QueryBuilder::<Sqlite>::new(
            "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) ",
        );
        insert.push_values(chunk, |mut row, (account_id, last_activity_at)| {
            row.push_bind(archive_batch_id)
                .push_bind(account_id)
                .push_bind(last_activity_at);
        });
        insert.push(
            " ON CONFLICT(archive_batch_id, account_id) DO UPDATE SET last_activity_at = CASE \
             WHEN excluded.last_activity_at > last_activity_at THEN excluded.last_activity_at \
             ELSE last_activity_at END",
        );
        insert.build().execute(&mut *tx).await?;
    }
    sqlx::query(
        "UPDATE archive_batches SET upstream_activity_manifest_refreshed_at = datetime('now') WHERE id = ?1",
    )
    .bind(archive_batch_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn mark_retention_archived_hourly_rollup_targets_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    invocation_rows: &[InvocationHourlySourceRecord],
    forward_proxy_rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    match dataset {
        "codex_invocations" => {
            mark_invocation_hourly_rollup_buckets_materialized_tx(tx, invocation_rows).await?;
        }
        "forward_proxy_attempts" => {
            mark_forward_proxy_hourly_rollup_buckets_materialized_tx(tx, forward_proxy_rows)
                .await?;
        }
        _ => {}
    }
    Ok(())
}

async fn mark_archive_batch_historical_rollups_materialized_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn update_archive_batch_coverage_bounds_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET coverage_start_at = COALESCE(coverage_start_at, ?2),
            coverage_end_at = COALESCE(coverage_end_at, ?3)
        WHERE id = ?1
        "#,
    )
    .bind(archive_batch_id)
    .bind(coverage_start_at)
    .bind(coverage_end_at)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn mark_hourly_rollup_bucket_materialized_tx(
    tx: &mut SqliteConnection,
    target: &str,
    bucket_start_epoch: i64,
    source: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        ON CONFLICT(target, bucket_start_epoch, source) DO UPDATE SET
            materialized_at = datetime('now')
        "#,
    )
    .bind(target)
    .bind(bucket_start_epoch)
    .bind(source)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn mark_invocation_hourly_rollup_buckets_materialized_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    let mut overall_targets = HashSet::new();
    let mut sticky_targets = HashSet::new();
    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        overall_targets.insert((bucket_start_epoch, row.source.clone()));
        sticky_targets.insert(bucket_start_epoch);
    }

    let live_targets = load_live_invocation_bucket_targets_tx(tx, &overall_targets).await?;
    let live_proxy_buckets = live_targets
        .iter()
        .filter_map(|(bucket_start_epoch, source)| {
            (source == SOURCE_PROXY).then_some(*bucket_start_epoch)
        })
        .collect::<HashSet<_>>();

    for (bucket_start_epoch, source) in overall_targets {
        if live_targets.contains(&(bucket_start_epoch, source.clone())) {
            continue;
        }
        for target in [
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        ] {
            mark_hourly_rollup_bucket_materialized_tx(tx, target, bucket_start_epoch, &source)
                .await?;
        }
        if source == SOURCE_PROXY && !live_proxy_buckets.contains(&bucket_start_epoch) {
            mark_hourly_rollup_bucket_materialized_tx(
                tx,
                HOURLY_ROLLUP_TARGET_PROXY_PERF,
                bucket_start_epoch,
                SOURCE_PROXY,
            )
            .await?;
        }
    }

    for bucket_start_epoch in sticky_targets {
        if live_proxy_buckets.contains(&bucket_start_epoch) {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_STICKY_KEYS,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }

    Ok(())
}

async fn load_live_invocation_bucket_targets_tx(
    tx: &mut SqliteConnection,
    bucket_targets: &HashSet<(i64, String)>,
) -> Result<HashSet<(i64, String)>> {
    if bucket_targets.is_empty() {
        return Ok(HashSet::new());
    }

    let min_bucket_epoch = bucket_targets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum invocation bucket epoch"))?;
    let max_bucket_epoch = bucket_targets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum invocation bucket epoch"))?;
    let min_bucket_start = Utc
        .timestamp_opt(min_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum invocation bucket epoch"))?;
    let max_bucket_end = Utc
        .timestamp_opt(max_bucket_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum invocation bucket epoch"))?;

    let rows = sqlx::query_as::<_, InvocationBucketPresenceRow>(
        r#"
        SELECT occurred_at, source
        FROM codex_invocations
        WHERE occurred_at >= ?1
          AND occurred_at < ?2
        ORDER BY id ASC
        "#,
    )
    .bind(db_occurred_at_lower_bound(min_bucket_start))
    .bind(db_occurred_at_lower_bound(max_bucket_end))
    .fetch_all(&mut *tx)
    .await?;

    let mut live_targets = HashSet::new();
    for row in rows {
        let key = (invocation_bucket_start_epoch(&row.occurred_at)?, row.source);
        if bucket_targets.contains(&key) {
            live_targets.insert(key);
        }
    }
    Ok(live_targets)
}

async fn mark_forward_proxy_hourly_rollup_buckets_materialized_tx(
    tx: &mut SqliteConnection,
    rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    let mut buckets = HashSet::new();
    for row in rows {
        buckets.insert(forward_proxy_attempt_bucket_start_epoch(&row.occurred_at)?);
    }
    for bucket_start_epoch in buckets {
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
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

#[derive(Debug, Default)]
struct InvocationHourlyRollupDelta {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    first_byte_sample_count: i64,
    first_byte_sum_ms: f64,
    first_byte_max_ms: f64,
    first_byte_histogram: ApproxHistogramCounts,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    first_response_byte_total_max_ms: f64,
    first_response_byte_total_histogram: ApproxHistogramCounts,
}

#[derive(Debug, Default)]
struct ProxyPerfStageHourlyDelta {
    sample_count: i64,
    sum_ms: f64,
    max_ms: f64,
    histogram: ApproxHistogramCounts,
}

#[derive(Debug, Default)]
struct KeyedConversationHourlyDelta {
    request_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    first_seen_at: String,
    last_seen_at: String,
}

#[derive(Debug, Default)]
struct ForwardProxyAttemptHourlyDelta {
    attempts: i64,
    success_count: i64,
    failure_count: i64,
    latency_sample_count: i64,
    latency_sum_ms: f64,
    latency_max_ms: f64,
}

fn invocation_bucket_start_epoch(occurred_at: &str) -> Result<i64> {
    let occurred_at_utc = parse_to_utc_datetime(occurred_at)
        .ok_or_else(|| anyhow!("failed to parse invocation occurred_at: {occurred_at}"))?;
    Ok(align_bucket_epoch(occurred_at_utc.timestamp(), 3600, 0))
}

fn forward_proxy_attempt_bucket_start_epoch(occurred_at: &str) -> Result<i64> {
    Ok(align_bucket_epoch(
        parse_utc_naive(occurred_at)?.and_utc().timestamp(),
        3600,
        0,
    ))
}

fn keyed_conversation_delta<'a>(
    map: &'a mut BTreeMap<(i64, String, String), KeyedConversationHourlyDelta>,
    bucket_start_epoch: i64,
    source: &str,
    key: &str,
    occurred_at: &str,
) -> &'a mut KeyedConversationHourlyDelta {
    let entry = map
        .entry((bucket_start_epoch, source.to_string(), key.to_string()))
        .or_insert_with(|| KeyedConversationHourlyDelta {
            first_seen_at: occurred_at.to_string(),
            last_seen_at: occurred_at.to_string(),
            ..KeyedConversationHourlyDelta::default()
        });
    if entry.first_seen_at.is_empty() || occurred_at < entry.first_seen_at.as_str() {
        entry.first_seen_at = occurred_at.to_string();
    }
    if entry.last_seen_at.is_empty() || occurred_at > entry.last_seen_at.as_str() {
        entry.last_seen_at = occurred_at.to_string();
    }
    entry
}

fn record_proxy_perf_stage_sample(
    map: &mut BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    bucket_start_epoch: i64,
    stage: &str,
    value_ms: Option<f64>,
) {
    let Some(value_ms) = value_ms else {
        return;
    };
    if !value_ms.is_finite() || value_ms < 0.0 {
        return;
    }
    let entry = map
        .entry((bucket_start_epoch, stage.to_string()))
        .or_insert_with(|| ProxyPerfStageHourlyDelta {
            histogram: empty_approx_histogram(),
            ..ProxyPerfStageHourlyDelta::default()
        });
    entry.sample_count += 1;
    entry.sum_ms += value_ms;
    entry.max_ms = entry.max_ms.max(value_ms);
    add_approx_histogram_sample(&mut entry.histogram, value_ms);
}

fn accumulate_invocation_hourly_overall_rollups(
    overall: &mut BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        let overall_entry = overall
            .entry((bucket_start_epoch, row.source.clone()))
            .or_insert_with(|| InvocationHourlyRollupDelta {
                first_byte_histogram: empty_approx_histogram(),
                first_response_byte_total_histogram: empty_approx_histogram(),
                ..InvocationHourlyRollupDelta::default()
            });
        overall_entry.total_count += 1;
        match row.status.as_deref() {
            Some("success") => overall_entry.success_count += 1,
            Some(_) => overall_entry.failure_count += 1,
            None => {}
        }
        overall_entry.total_tokens += row.total_tokens.unwrap_or_default();
        overall_entry.total_cost += row.cost.unwrap_or_default();
        if row.status.as_deref() == Some("success")
            && let Some(ttfb_ms) = row.t_upstream_ttfb_ms
            && ttfb_ms.is_finite()
            && ttfb_ms > 0.0
        {
            overall_entry.first_byte_sample_count += 1;
            overall_entry.first_byte_sum_ms += ttfb_ms;
            overall_entry.first_byte_max_ms = overall_entry.first_byte_max_ms.max(ttfb_ms);
            add_approx_histogram_sample(&mut overall_entry.first_byte_histogram, ttfb_ms);
        }
        if let Some(first_response_byte_total_ms) = resolve_first_response_byte_total_ms(
            row.t_req_read_ms,
            row.t_req_parse_ms,
            row.t_upstream_connect_ms,
            row.t_upstream_ttfb_ms,
        ) {
            overall_entry.first_response_byte_total_sample_count += 1;
            overall_entry.first_response_byte_total_sum_ms += first_response_byte_total_ms;
            overall_entry.first_response_byte_total_max_ms = overall_entry
                .first_response_byte_total_max_ms
                .max(first_response_byte_total_ms);
            add_approx_histogram_sample(
                &mut overall_entry.first_response_byte_total_histogram,
                first_response_byte_total_ms,
            );
        }
    }

    Ok(())
}

async fn upsert_invocation_hourly_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
    targets: &[&str],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let upsert_overall = targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATIONS);
    let upsert_failures = targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES);
    let upsert_perf = targets.contains(&HOURLY_ROLLUP_TARGET_PROXY_PERF);
    let upsert_prompt_cache = targets.contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE);
    let upsert_prompt_cache_upstream_accounts =
        targets.contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS);
    let upsert_sticky_keys = targets.contains(&HOURLY_ROLLUP_TARGET_STICKY_KEYS);

    let mut overall: BTreeMap<(i64, String), InvocationHourlyRollupDelta> = BTreeMap::new();
    let mut failures: BTreeMap<(i64, String, String, i64, String), i64> = BTreeMap::new();
    let mut perf: BTreeMap<(i64, String), ProxyPerfStageHourlyDelta> = BTreeMap::new();
    let mut prompt_cache: BTreeMap<(i64, String, String), KeyedConversationHourlyDelta> =
        BTreeMap::new();
    let mut prompt_cache_upstream_accounts: BTreeMap<
        (i64, String, String, String, Option<i64>, Option<String>),
        KeyedConversationHourlyDelta,
    > = BTreeMap::new();
    let mut sticky_keys: BTreeMap<(i64, i64, String), KeyedConversationHourlyDelta> =
        BTreeMap::new();

    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        if upsert_overall {
            accumulate_invocation_hourly_overall_rollups(&mut overall, std::slice::from_ref(row))?;
        }

        if upsert_failures {
            let classification = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            if classification.failure_class != FailureClass::None {
                let error_category =
                    categorize_error(row.error_message.as_deref().unwrap_or_default());
                *failures
                    .entry((
                        bucket_start_epoch,
                        row.source.clone(),
                        classification.failure_class.as_str().to_string(),
                        classification.is_actionable as i64,
                        error_category,
                    ))
                    .or_default() += 1;
            }
        }

        if upsert_perf && row.source == SOURCE_PROXY {
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_TOTAL,
                row.t_total_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_REQUEST_READ,
                row.t_req_read_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_REQUEST_PARSE,
                row.t_req_parse_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_UPSTREAM_CONNECT,
                row.t_upstream_connect_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_UPSTREAM_FIRST_BYTE,
                row.t_upstream_ttfb_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_UPSTREAM_STREAM,
                row.t_upstream_stream_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_RESPONSE_PARSE,
                row.t_resp_parse_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_PERSISTENCE,
                row.t_persist_ms,
            );
        }

        if (upsert_prompt_cache || upsert_prompt_cache_upstream_accounts)
            && let Some(prompt_cache_key) = prompt_cache_key_from_payload(row.payload.as_deref())
        {
            if upsert_prompt_cache {
                let entry = keyed_conversation_delta(
                    &mut prompt_cache,
                    bucket_start_epoch,
                    &row.source,
                    &prompt_cache_key,
                    &row.occurred_at,
                );
                entry.request_count += 1;
                if row.status.as_deref() == Some("success") {
                    entry.success_count += 1;
                } else {
                    entry.failure_count += 1;
                }
                entry.total_tokens += row.total_tokens.unwrap_or_default();
                entry.total_cost += row.cost.unwrap_or_default();
            }

            if upsert_prompt_cache_upstream_accounts {
                let upstream_account_id = upstream_account_id_from_payload(row.payload.as_deref());
                let upstream_account_name =
                    upstream_account_name_from_payload(row.payload.as_deref());
                let rollup_key = prompt_cache_upstream_account_rollup_key(
                    upstream_account_id,
                    upstream_account_name.as_deref(),
                );
                let entry = prompt_cache_upstream_accounts
                    .entry((
                        bucket_start_epoch,
                        row.source.clone(),
                        prompt_cache_key,
                        rollup_key,
                        upstream_account_id,
                        upstream_account_name.clone(),
                    ))
                    .or_insert_with(|| KeyedConversationHourlyDelta {
                        first_seen_at: row.occurred_at.clone(),
                        last_seen_at: row.occurred_at.clone(),
                        ..KeyedConversationHourlyDelta::default()
                    });
                if row.occurred_at < entry.first_seen_at {
                    entry.first_seen_at = row.occurred_at.clone();
                }
                if row.occurred_at > entry.last_seen_at {
                    entry.last_seen_at = row.occurred_at.clone();
                }
                entry.request_count += 1;
                if row.status.as_deref() == Some("success") {
                    entry.success_count += 1;
                } else {
                    entry.failure_count += 1;
                }
                entry.total_tokens += row.total_tokens.unwrap_or_default();
                entry.total_cost += row.cost.unwrap_or_default();
            }
        }

        if upsert_sticky_keys
            && let (Some(upstream_account_id), Some(sticky_key)) = (
                upstream_account_id_from_payload(row.payload.as_deref()),
                sticky_key_from_payload(row.payload.as_deref()),
            )
        {
            let entry = sticky_keys
                .entry((bucket_start_epoch, upstream_account_id, sticky_key))
                .or_insert_with(|| KeyedConversationHourlyDelta {
                    first_seen_at: row.occurred_at.clone(),
                    last_seen_at: row.occurred_at.clone(),
                    ..KeyedConversationHourlyDelta::default()
                });
            if row.occurred_at < entry.first_seen_at {
                entry.first_seen_at = row.occurred_at.clone();
            }
            if row.occurred_at > entry.last_seen_at {
                entry.last_seen_at = row.occurred_at.clone();
            }
            entry.request_count += 1;
            if row.status.as_deref() == Some("success") {
                entry.success_count += 1;
            } else {
                entry.failure_count += 1;
            }
            entry.total_tokens += row.total_tokens.unwrap_or_default();
            entry.total_cost += row.cost.unwrap_or_default();
        }
    }

    if upsert_overall {
        #[derive(sqlx::FromRow)]
        struct InvocationRollupHistogramRow {
            first_byte_histogram: String,
            first_response_byte_total_histogram: String,
        }

        for ((bucket_start_epoch, source), delta) in overall {
            let current_histograms = sqlx::query_as::<_, InvocationRollupHistogramRow>(
                r#"
                SELECT
                    first_byte_histogram,
                    first_response_byte_total_histogram
                FROM invocation_rollup_hourly
                WHERE bucket_start_epoch = ?1 AND source = ?2
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .fetch_optional(&mut *tx)
            .await?;
            let mut merged_first_byte_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_byte_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_byte_histogram,
                &delta.first_byte_histogram,
            )?;
            let mut merged_first_response_byte_total_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_response_byte_total_histogram,
                &delta.first_response_byte_total_histogram,
            )?;
            sqlx::query(
                r#"
                INSERT INTO invocation_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    total_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_byte_sample_count,
                    first_byte_sum_ms,
                    first_byte_max_ms,
                    first_byte_histogram,
                    first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms,
                    first_response_byte_total_histogram,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET
                    total_count = invocation_rollup_hourly.total_count + excluded.total_count,
                    success_count = invocation_rollup_hourly.success_count + excluded.success_count,
                    failure_count = invocation_rollup_hourly.failure_count + excluded.failure_count,
                    total_tokens = invocation_rollup_hourly.total_tokens + excluded.total_tokens,
                    total_cost = invocation_rollup_hourly.total_cost + excluded.total_cost,
                    first_byte_sample_count = invocation_rollup_hourly.first_byte_sample_count + excluded.first_byte_sample_count,
                    first_byte_sum_ms = invocation_rollup_hourly.first_byte_sum_ms + excluded.first_byte_sum_ms,
                    first_byte_max_ms = MAX(invocation_rollup_hourly.first_byte_max_ms, excluded.first_byte_max_ms),
                    first_byte_histogram = excluded.first_byte_histogram,
                    first_response_byte_total_sample_count = invocation_rollup_hourly.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms = invocation_rollup_hourly.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms = MAX(invocation_rollup_hourly.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
                    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(delta.first_byte_sample_count)
            .bind(delta.first_byte_sum_ms)
            .bind(delta.first_byte_max_ms)
            .bind(encode_approx_histogram(&merged_first_byte_histogram)?)
            .bind(delta.first_response_byte_total_sample_count)
            .bind(delta.first_response_byte_total_sum_ms)
            .bind(delta.first_response_byte_total_max_ms)
            .bind(encode_approx_histogram(
                &merged_first_response_byte_total_histogram,
            )?)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_failures {
        for (
            (bucket_start_epoch, source, failure_class, is_actionable, error_category),
            failure_count,
        ) in failures
        {
            sqlx::query(
                r#"
                INSERT INTO invocation_failure_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    failure_class,
                    is_actionable,
                    error_category,
                    failure_count,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, failure_class, is_actionable, error_category) DO UPDATE SET
                    failure_count = invocation_failure_rollup_hourly.failure_count + excluded.failure_count,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&failure_class)
            .bind(is_actionable)
            .bind(&error_category)
            .bind(failure_count)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_perf {
        for ((bucket_start_epoch, stage), delta) in perf {
            let current_histogram = sqlx::query_scalar::<_, String>(
                r#"
                SELECT histogram
                FROM proxy_perf_stage_hourly
                WHERE bucket_start_epoch = ?1 AND stage = ?2
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&stage)
            .fetch_optional(&mut *tx)
            .await?;
            let mut merged_histogram = current_histogram
                .as_deref()
                .map(decode_approx_histogram)
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(&mut merged_histogram, &delta.histogram)?;
            sqlx::query(
                r#"
                INSERT INTO proxy_perf_stage_hourly (
                    bucket_start_epoch,
                    stage,
                    sample_count,
                    sum_ms,
                    max_ms,
                    histogram,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                ON CONFLICT(bucket_start_epoch, stage) DO UPDATE SET
                    sample_count = proxy_perf_stage_hourly.sample_count + excluded.sample_count,
                    sum_ms = proxy_perf_stage_hourly.sum_ms + excluded.sum_ms,
                    max_ms = MAX(proxy_perf_stage_hourly.max_ms, excluded.max_ms),
                    histogram = excluded.histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&stage)
            .bind(delta.sample_count)
            .bind(delta.sum_ms)
            .bind(delta.max_ms)
            .bind(encode_approx_histogram(&merged_histogram)?)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_prompt_cache {
        for ((bucket_start_epoch, source, prompt_cache_key), delta) in prompt_cache {
            sqlx::query(
                r#"
                INSERT INTO prompt_cache_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    prompt_cache_key,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, prompt_cache_key) DO UPDATE SET
                    request_count = prompt_cache_rollup_hourly.request_count + excluded.request_count,
                    success_count = prompt_cache_rollup_hourly.success_count + excluded.success_count,
                    failure_count = prompt_cache_rollup_hourly.failure_count + excluded.failure_count,
                    total_tokens = prompt_cache_rollup_hourly.total_tokens + excluded.total_tokens,
                    total_cost = prompt_cache_rollup_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(prompt_cache_rollup_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(prompt_cache_rollup_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&prompt_cache_key)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_prompt_cache_upstream_accounts {
        for (
            (
                bucket_start_epoch,
                source,
                prompt_cache_key,
                upstream_account_key,
                upstream_account_id,
                upstream_account_name,
            ),
            delta,
        ) in prompt_cache_upstream_accounts
        {
            sqlx::query(
                r#"
                INSERT INTO prompt_cache_upstream_account_hourly (
                    bucket_start_epoch,
                    source,
                    prompt_cache_key,
                    upstream_account_key,
                    upstream_account_id,
                    upstream_account_name,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, prompt_cache_key, upstream_account_key) DO UPDATE SET
                    request_count = prompt_cache_upstream_account_hourly.request_count + excluded.request_count,
                    success_count = prompt_cache_upstream_account_hourly.success_count + excluded.success_count,
                    failure_count = prompt_cache_upstream_account_hourly.failure_count + excluded.failure_count,
                    total_tokens = prompt_cache_upstream_account_hourly.total_tokens + excluded.total_tokens,
                    total_cost = prompt_cache_upstream_account_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(prompt_cache_upstream_account_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(prompt_cache_upstream_account_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&prompt_cache_key)
            .bind(&upstream_account_key)
            .bind(upstream_account_id)
            .bind(upstream_account_name.as_deref())
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_sticky_keys {
        for ((bucket_start_epoch, upstream_account_id, sticky_key), delta) in sticky_keys {
            sqlx::query(
                r#"
                INSERT INTO upstream_sticky_key_hourly (
                    bucket_start_epoch,
                    upstream_account_id,
                    sticky_key,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
                ON CONFLICT(bucket_start_epoch, upstream_account_id, sticky_key) DO UPDATE SET
                    request_count = upstream_sticky_key_hourly.request_count + excluded.request_count,
                    success_count = upstream_sticky_key_hourly.success_count + excluded.success_count,
                    failure_count = upstream_sticky_key_hourly.failure_count + excluded.failure_count,
                    total_tokens = upstream_sticky_key_hourly.total_tokens + excluded.total_tokens,
                    total_cost = upstream_sticky_key_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(upstream_sticky_key_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(upstream_sticky_key_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(upstream_account_id)
            .bind(&sticky_key)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    Ok(())
}

fn invocation_archive_target_needs_full_payload(target: &str) -> bool {
    matches!(
        target,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE
            | HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS
            | HOURLY_ROLLUP_TARGET_STICKY_KEYS
    )
}

fn invocation_archive_has_pruned_success_details(rows: &[InvocationHourlySourceRecord]) -> bool {
    rows.iter().any(|row| {
        row.detail_level != DETAIL_LEVEL_FULL
            && invocation_status_is_success_like(
                row.status.as_deref(),
                row.error_message.as_deref(),
            )
    })
}

async fn upsert_forward_proxy_attempt_hourly_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut deltas: BTreeMap<(String, i64), ForwardProxyAttemptHourlyDelta> = BTreeMap::new();
    for row in rows {
        let bucket_start_epoch = forward_proxy_attempt_bucket_start_epoch(&row.occurred_at)?;
        let entry = deltas
            .entry((row.proxy_key.clone(), bucket_start_epoch))
            .or_default();
        entry.attempts += 1;
        if row.is_success != 0 {
            entry.success_count += 1;
        } else {
            entry.failure_count += 1;
        }
        if let Some(latency_ms) = row.latency_ms
            && latency_ms.is_finite()
            && latency_ms >= 0.0
        {
            entry.latency_sample_count += 1;
            entry.latency_sum_ms += latency_ms;
            entry.latency_max_ms = entry.latency_max_ms.max(latency_ms);
        }
    }

    for ((proxy_key, bucket_start_epoch), delta) in deltas {
        sqlx::query(
            r#"
            INSERT INTO forward_proxy_attempt_hourly (
                proxy_key,
                bucket_start_epoch,
                attempts,
                success_count,
                failure_count,
                latency_sample_count,
                latency_sum_ms,
                latency_max_ms,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
            ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
                attempts = forward_proxy_attempt_hourly.attempts + excluded.attempts,
                success_count = forward_proxy_attempt_hourly.success_count + excluded.success_count,
                failure_count = forward_proxy_attempt_hourly.failure_count + excluded.failure_count,
                latency_sample_count = forward_proxy_attempt_hourly.latency_sample_count + excluded.latency_sample_count,
                latency_sum_ms = forward_proxy_attempt_hourly.latency_sum_ms + excluded.latency_sum_ms,
                latency_max_ms = MAX(forward_proxy_attempt_hourly.latency_max_ms, excluded.latency_max_ms),
                updated_at = datetime('now')
            "#,
        )
        .bind(&proxy_key)
        .bind(bucket_start_epoch)
        .bind(delta.attempts)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.latency_sample_count)
        .bind(delta.latency_sum_ms)
        .bind(delta.latency_max_ms)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

async fn delete_hourly_rollup_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    table: &str,
    bucket_epochs: &[i64],
) -> Result<()> {
    if bucket_epochs.is_empty() {
        return Ok(());
    }
    let mut query =
        QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE bucket_start_epoch IN ("));
    {
        let mut separated = query.separated(", ");
        for bucket_epoch in bucket_epochs {
            separated.push_bind(bucket_epoch);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    Ok(())
}

async fn load_live_invocation_hourly_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    bucket_epochs: &[i64],
) -> Result<Vec<InvocationHourlySourceRecord>> {
    if bucket_epochs.is_empty() {
        return Ok(Vec::new());
    }

    let min_bucket_epoch = *bucket_epochs
        .iter()
        .min()
        .ok_or_else(|| anyhow!("missing minimum invocation bucket epoch"))?;
    let max_bucket_epoch = *bucket_epochs
        .iter()
        .max()
        .ok_or_else(|| anyhow!("missing maximum invocation bucket epoch"))?;
    let min_bucket_start = Utc
        .timestamp_opt(min_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum invocation bucket epoch"))?;
    let max_bucket_end = Utc
        .timestamp_opt(max_bucket_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum invocation bucket epoch"))?;
    let bucket_epoch_set = bucket_epochs.iter().copied().collect::<HashSet<_>>();

    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
        "SELECT \
            id,
            occurred_at,
            source,
            status,
            detail_level,
            total_tokens,
            cost,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
         FROM codex_invocations
         WHERE occurred_at >= ?1
           AND occurred_at < ?2
         ORDER BY id ASC",
    )
    .bind(db_occurred_at_lower_bound(min_bucket_start))
    .bind(db_occurred_at_lower_bound(max_bucket_end))
    .fetch_all(&mut *tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            invocation_bucket_start_epoch(&row.occurred_at)
                .map(|bucket_epoch| bucket_epoch_set.contains(&bucket_epoch))
                .unwrap_or(false)
        })
        .collect())
}

async fn recompute_invocation_hourly_rollups_for_ids_tx(
    tx: &mut SqliteConnection,
    ids: &[i64],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT occurred_at FROM codex_invocations WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
    }
    query.push(")");
    let occurred_rows = query
        .build_query_scalar::<String>()
        .fetch_all(&mut *tx)
        .await?;
    if occurred_rows.is_empty() {
        return Ok(());
    }

    let mut bucket_epochs = occurred_rows
        .iter()
        .map(|occurred_at| invocation_bucket_start_epoch(occurred_at))
        .collect::<Result<Vec<_>>>()?;
    bucket_epochs.sort_unstable();
    bucket_epochs.dedup();
    if bucket_epochs.is_empty() {
        return Ok(());
    }

    for table in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
    ] {
        delete_hourly_rollup_rows_for_bucket_epochs_tx(tx, table, &bucket_epochs).await?;
    }

    let rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx, &bucket_epochs).await?;
    upsert_invocation_hourly_rollups_tx(tx, &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS).await?;
    Ok(())
}

async fn replay_live_invocation_hourly_rollups(pool: &Pool<Sqlite>) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            total_tokens,
            cost,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    let mut tx = pool.begin().await?;
    upsert_invocation_hourly_rollups_tx(tx.as_mut(), &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS)
        .await?;
    save_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id)
        .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

async fn replay_live_invocation_hourly_rollups_tx(tx: &mut SqliteConnection) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            total_tokens,
            cost,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_invocation_hourly_rollups_tx(tx, &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS).await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id).await?;
    Ok(rows.len() as u64)
}

async fn replay_live_forward_proxy_attempt_hourly_rollups(pool: &Pool<Sqlite>) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    let mut tx = pool.begin().await?;
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

async fn replay_live_forward_proxy_attempt_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &rows).await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS, last_id)
        .await?;
    Ok(rows.len() as u64)
}

async fn backfill_invocation_rollup_hourly_from_sources(pool: &Pool<Sqlite>) -> Result<usize> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    let mut overall: BTreeMap<(i64, String), InvocationHourlyRollupDelta> = BTreeMap::new();
    let mut seen_ids = HashSet::new();

    for archive_file in archive_files {
        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during invocation hourly rollup backfill"
            );
            continue;
        }

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
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        let mut archive_cursor_id = 0_i64;
        loop {
            let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
                r#"
                SELECT
                    id,
                    occurred_at,
                    source,
                    status,
                    detail_level,
                    total_tokens,
                    cost,
                    error_message,
                    failure_kind,
                    failure_class,
                    is_actionable,
                    payload,
                    t_total_ms,
                    t_req_read_ms,
                    t_req_parse_ms,
                    t_upstream_connect_ms,
                    t_upstream_ttfb_ms,
                    t_upstream_stream_ms,
                    t_resp_parse_ms,
                    t_persist_ms
                FROM codex_invocations
                WHERE id > ?1
                ORDER BY id ASC
                LIMIT ?2
                "#,
            )
            .bind(archive_cursor_id)
            .bind(BACKFILL_BATCH_SIZE)
            .fetch_all(&archive_pool)
            .await?;
            if rows.is_empty() {
                break;
            }
            archive_cursor_id = rows.last().map(|row| row.id).unwrap_or(archive_cursor_id);
            rows.retain(|row| seen_ids.insert(row.id));
            if rows.is_empty() {
                continue;
            }
            accumulate_invocation_hourly_overall_rollups(&mut overall, &rows)?;
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    let mut cursor_id = 0_i64;
    loop {
        let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                detail_level,
                total_tokens,
                cost,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms
            FROM codex_invocations
            WHERE id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(cursor_id)
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(pool)
        .await?;
        if rows.is_empty() {
            break;
        }
        cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
        rows.retain(|row| seen_ids.insert(row.id));
        if rows.is_empty() {
            continue;
        }
        accumulate_invocation_hourly_overall_rollups(&mut overall, &rows)?;
    }

    if overall.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await?;
    for ((bucket_start_epoch, source), delta) in &overall {
        sqlx::query(
            r#"
            INSERT INTO invocation_rollup_hourly (
                bucket_start_epoch,
                source,
                total_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost,
                first_byte_sample_count,
                first_byte_sum_ms,
                first_byte_max_ms,
                first_byte_histogram,
                first_response_byte_total_sample_count,
                first_response_byte_total_sum_ms,
                first_response_byte_total_max_ms,
                first_response_byte_total_histogram,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
            ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET
                total_count = excluded.total_count,
                success_count = excluded.success_count,
                failure_count = excluded.failure_count,
                total_tokens = excluded.total_tokens,
                total_cost = excluded.total_cost,
                first_byte_sample_count = excluded.first_byte_sample_count,
                first_byte_sum_ms = excluded.first_byte_sum_ms,
                first_byte_max_ms = excluded.first_byte_max_ms,
                first_byte_histogram = excluded.first_byte_histogram,
                first_response_byte_total_sample_count = excluded.first_response_byte_total_sample_count,
                first_response_byte_total_sum_ms = excluded.first_response_byte_total_sum_ms,
                first_response_byte_total_max_ms = excluded.first_response_byte_total_max_ms,
                first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                updated_at = datetime('now')
            "#,
        )
        .bind(*bucket_start_epoch)
        .bind(source)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.total_tokens)
        .bind(delta.total_cost)
        .bind(delta.first_byte_sample_count)
        .bind(delta.first_byte_sum_ms)
        .bind(delta.first_byte_max_ms)
        .bind(encode_approx_histogram(&delta.first_byte_histogram)?)
        .bind(delta.first_response_byte_total_sample_count)
        .bind(delta.first_response_byte_total_sum_ms)
        .bind(delta.first_response_byte_total_max_ms)
        .bind(encode_approx_histogram(
            &delta.first_response_byte_total_histogram,
        )?)
        .execute(tx.as_mut())
        .await?;
    }
    tx.commit().await?;

    Ok(overall.len())
}

async fn sync_hourly_rollups_from_live_tables(pool: &Pool<Sqlite>) -> Result<()> {
    loop {
        let updated = replay_live_invocation_hourly_rollups(pool).await?;
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated = replay_live_forward_proxy_attempt_hourly_rollups(pool).await?;
        if updated == 0 {
            break;
        }
    }
    Ok(())
}

async fn replay_invocation_archives_into_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND historical_rollups_materialized_at IS NULL
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(&mut *tx)
    .await?;
    let mut replayed = 0_u64;

    for archive_file in archive_files {
        let mut pending_targets = Vec::new();
        let mut blocked_targets = Vec::new();
        for target in [
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            HOURLY_ROLLUP_TARGET_PROXY_PERF,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
            HOURLY_ROLLUP_TARGET_STICKY_KEYS,
        ] {
            if !hourly_rollup_archive_replayed_tx(
                tx,
                target,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?
            {
                pending_targets.push(target);
            }
        }

        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during historical rollup materialization"
            );
            continue;
        }
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
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                detail_level,
                total_tokens,
                cost,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms
            FROM codex_invocations
            ORDER BY id ASC
            "#,
        )
        .fetch_all(&archive_pool)
        .await?;
        archive_pool.close().await;
        drop(temp_cleanup);

        let has_pruned_success_details = invocation_archive_has_pruned_success_details(&rows);
        if has_pruned_success_details {
            let mut replayable_targets = Vec::with_capacity(pending_targets.len());
            for target in pending_targets {
                if invocation_archive_target_needs_full_payload(target) {
                    blocked_targets.push(target);
                } else {
                    replayable_targets.push(target);
                }
            }
            pending_targets = replayable_targets;
        }

        if pending_targets.is_empty() && blocked_targets.is_empty() {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }

        if archive_file.coverage_start_at.is_none() || archive_file.coverage_end_at.is_none() {
            let coverage_start_at = rows.iter().map(|row| row.occurred_at.as_str()).min();
            let coverage_end_at = rows.iter().map(|row| row.occurred_at.as_str()).max();
            update_archive_batch_coverage_bounds_tx(
                tx,
                archive_file.id,
                coverage_start_at,
                coverage_end_at,
            )
            .await?;
        }

        upsert_invocation_hourly_rollups_tx(tx, &rows, &pending_targets).await?;
        mark_invocation_hourly_rollup_buckets_materialized_tx(tx, &rows).await?;
        for target in pending_targets {
            mark_hourly_rollup_archive_replayed_tx(
                tx,
                target,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
        }
        if blocked_targets.is_empty() {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                &archive_file.file_path,
            )
            .await?;
            replayed += 1;
        } else {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                blocked_targets = ?blocked_targets,
                "legacy archive batch contains pruned success details; keeping historical rollup materialization pending for keyed conversation targets"
            );
        }
    }

    Ok(replayed)
}

async fn replay_forward_proxy_archives_into_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'forward_proxy_attempts'
          AND status = ?1
          AND historical_rollups_materialized_at IS NULL
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(&mut *tx)
    .await?;
    let mut replayed = 0_u64;

    for archive_file in archive_files {
        if hourly_rollup_archive_replayed_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?
        {
            mark_archive_batch_historical_rollups_materialized_tx(
                tx,
                HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                &archive_file.file_path,
            )
            .await?;
            continue;
        }

        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during historical rollup materialization"
            );
            continue;
        }
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
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
            r#"
            SELECT
                id,
                proxy_key,
                occurred_at,
                is_success,
                latency_ms
            FROM forward_proxy_attempts
            ORDER BY id ASC
            "#,
        )
        .fetch_all(&archive_pool)
        .await?;
        archive_pool.close().await;
        drop(temp_cleanup);

        if archive_file.coverage_start_at.is_none() || archive_file.coverage_end_at.is_none() {
            let coverage_start_at = rows.iter().map(|row| row.occurred_at.as_str()).min();
            let coverage_end_at = rows.iter().map(|row| row.occurred_at.as_str()).max();
            update_archive_batch_coverage_bounds_tx(
                tx,
                archive_file.id,
                coverage_start_at,
                coverage_end_at,
            )
            .await?;
        }

        upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &rows).await?;
        mark_forward_proxy_hourly_rollup_buckets_materialized_tx(tx, &rows).await?;
        mark_archive_batch_historical_rollups_materialized_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            &archive_file.file_path,
        )
        .await?;
        replayed += 1;
    }

    Ok(replayed)
}

async fn bootstrap_hourly_rollups(pool: &Pool<Sqlite>) -> Result<()> {
    sync_hourly_rollups_from_live_tables(pool).await?;
    Ok(())
}

async fn ensure_hourly_rollups_caught_up(state: &AppState) -> Result<()> {
    let _guard = state.hourly_rollup_sync_lock.lock().await;
    sync_hourly_rollups_from_live_tables(&state.pool).await
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

fn invocation_status_is_success_like(status: Option<&str>, error_message: Option<&str>) -> bool {
    let normalized_status = status.map(str::trim).unwrap_or_default();
    let error_message_empty = error_message.map(str::trim).is_none_or(str::is_empty);

    normalized_status.eq_ignore_ascii_case("success")
        || (normalized_status.eq_ignore_ascii_case("http_200") && error_message_empty)
}

fn invocation_status_is_success_like_sql(
    status_column: &str,
    error_message_column: &str,
) -> String {
    format!(
        "(LOWER(TRIM(COALESCE({status_column}, ''))) = 'success' OR (LOWER(TRIM(COALESCE({status_column}, ''))) = 'http_200' AND TRIM(COALESCE({error_message_column}, '')) = ''))"
    )
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

fn archive_segment_file_path(
    config: &AppConfig,
    dataset: &str,
    day_key: &str,
    part_key: &str,
    codec: ArchiveFileCodec,
) -> Result<PathBuf> {
    let mut segments = day_key.split('-');
    let year = segments
        .next()
        .filter(|segment| segment.len() == 4)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?;
    let month = segments
        .next()
        .filter(|segment| segment.len() == 2)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?;
    let day = segments
        .next()
        .filter(|segment| segment.len() == 2)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?;
    Ok(resolved_archive_dir(config)
        .join(dataset)
        .join(year)
        .join(month)
        .join(day)
        .join(format!("{part_key}.sqlite.{}", codec.file_extension())))
}

fn archive_month_key_from_day_key(day_key: &str) -> Result<String> {
    Ok(day_key
        .get(..7)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?
        .to_string())
}

fn retention_temp_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

fn archive_layout_for_dataset(config: &AppConfig, dataset: &str) -> ArchiveBatchLayout {
    if dataset == "codex_invocations" {
        config.codex_invocation_archive_layout
    } else {
        ArchiveBatchLayout::LegacyMonth
    }
}

fn invocation_archive_group_key(config: &AppConfig, occurred_at: &str) -> Result<String> {
    match config.codex_invocation_archive_layout {
        ArchiveBatchLayout::LegacyMonth => shanghai_month_key_from_local_naive(occurred_at),
        ArchiveBatchLayout::SegmentV1 => {
            match config.codex_invocation_archive_segment_granularity {
                ArchiveSegmentGranularity::Day => shanghai_day_key_from_local_naive(occurred_at),
            }
        }
    }
}

async fn next_archive_segment_part_key(
    pool: &Pool<Sqlite>,
    dataset: &str,
    day_key: &str,
) -> Result<String> {
    let latest_part_key = sqlx::query_scalar::<_, String>(
        r#"
        SELECT part_key
        FROM archive_batches
        WHERE dataset = ?1
          AND layout = ?2
          AND day_key = ?3
          AND part_key IS NOT NULL
        ORDER BY part_key DESC, id DESC
        LIMIT 1
        "#,
    )
    .bind(dataset)
    .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
    .bind(day_key)
    .fetch_optional(pool)
    .await?;
    let next_seq = latest_part_key
        .as_deref()
        .and_then(|value| value.strip_prefix("part-"))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default()
        + 1;
    Ok(format!("part-{next_seq:06}"))
}

fn is_archive_temp_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.contains(".sqlite.gz.") || lower.contains(".sqlite.zst."))
        && (lower.ends_with(".sqlite")
            || lower.ends_with(".tmp")
            || lower.ends_with(".partial")
            || lower.ends_with(".sqlite-wal")
            || lower.ends_with(".sqlite-shm"))
}

fn collect_archive_file_paths(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to read archive directory {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_archive_file_paths(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
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
                        state_clone.config.invocation_max_days,
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
        .route(
            "/api/settings/proxy",
            any(removed_proxy_model_settings_endpoint),
        )
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
        .route(
            "/api/invocations/:invoke_id/pool-attempts",
            get(fetch_invocation_pool_attempts),
        )
        .route(
            "/api/invocations/:id/detail",
            get(fetch_invocation_record_detail),
        )
        .route(
            "/api/invocations/:id/response-body",
            get(fetch_invocation_response_body),
        )
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
        .route(
            "/api/stats/forward-proxy/timeseries",
            get(fetch_forward_proxy_timeseries),
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
        .route(
            "/api/pool/upstream-accounts",
            get(list_upstream_accounts_from_uri).post(bulk_update_upstream_accounts),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs",
            post(create_bulk_upstream_account_sync_job),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs/:jobId/events",
            get(stream_bulk_upstream_account_sync_job_events),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs/:jobId",
            get(get_bulk_upstream_account_sync_job).delete(cancel_bulk_upstream_account_sync_job),
        )
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
            "/api/pool/upstream-accounts/oauth/imports/validate",
            post(validate_imported_oauth_accounts)
                .layer(DefaultBodyLimit::max(IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES)),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs",
            post(create_imported_oauth_validation_job)
                .layer(DefaultBodyLimit::max(IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES)),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs/:jobId/events",
            get(stream_imported_oauth_validation_job_events),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs/:jobId",
            delete(cancel_imported_oauth_validation_job),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports",
            post(import_validated_oauth_accounts)
                .layer(DefaultBodyLimit::max(IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES)),
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
            get(get_oauth_login_session).patch(update_oauth_login_session),
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
    invocation_max_days: u64,
) -> Result<(Vec<SummaryPublish>, Option<QuotaSnapshotResponse>)> {
    Ok((
        collect_summary_snapshots(pool, relay, invocation_max_days).await?,
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
        collect_broadcast_state_snapshots(
            &state.pool,
            relay_config.as_ref(),
            state.config.invocation_max_days,
        )
        .await?
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
    invocation_max_days: u64,
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
                query_hourly_backed_summary_since_with_config(
                    pool,
                    relay,
                    invocation_max_days,
                    start,
                    source_scope,
                )
                .await
                .map_err(|err| anyhow!("{err:?}"))?
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
            request_raw_codec TEXT NOT NULL DEFAULT 'identity',
            request_raw_size INTEGER,
            request_raw_truncated INTEGER NOT NULL DEFAULT 0,
            request_raw_truncated_reason TEXT,
            response_raw_path TEXT,
            response_raw_codec TEXT NOT NULL DEFAULT 'identity',
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

async fn load_sqlite_table_columns_from_connection(
    conn: &mut SqliteConnection,
    schema_name: Option<&str>,
    table_name: &str,
) -> Result<HashSet<String>> {
    let pragma = schema_name.map_or_else(
        || format!("PRAGMA table_info('{table_name}')"),
        |schema_name| format!("PRAGMA {schema_name}.table_info('{table_name}')"),
    );
    let columns = sqlx::query(&pragma)
        .fetch_all(&mut *conn)
        .await
        .with_context(|| format!("failed to inspect {table_name} schema"))?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    Ok(columns)
}

async fn ensure_pool_upstream_request_attempts_archive_schema(
    conn: &mut SqliteConnection,
) -> Result<()> {
    let archive_columns = load_sqlite_table_columns_from_connection(
        conn,
        Some("archive_db"),
        "pool_upstream_request_attempts",
    )
    .await?;
    for (column, ty) in [
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
    ] {
        if !archive_columns.contains(column) {
            let statement = format!(
                "ALTER TABLE archive_db.pool_upstream_request_attempts ADD COLUMN {column} {ty}"
            );
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!(
                        "failed to add archive_db.pool_upstream_request_attempts column {column}"
                    )
                })?;
        }
    }
    Ok(())
}

async fn ensure_codex_invocations_archive_schema(conn: &mut SqliteConnection) -> Result<()> {
    let archive_columns =
        load_sqlite_table_columns_from_connection(conn, Some("archive_db"), "codex_invocations")
            .await?;
    for (column, ty) in [
        ("request_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
    ] {
        if !archive_columns.contains(column) {
            let statement =
                format!("ALTER TABLE archive_db.codex_invocations ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!("failed to add archive_db.codex_invocations column {column}")
                })?;
        }
    }
    sqlx::query(
        r#"
        UPDATE archive_db.codex_invocations
        SET request_raw_codec = CASE
                WHEN request_raw_path IS NOT NULL AND request_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(request_raw_codec), '') = ''
           OR (request_raw_codec = 'identity' AND request_raw_path LIKE '%.gz')
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("failed to backfill archive_db.codex_invocations request_raw_codec")?;
    sqlx::query(
        r#"
        UPDATE archive_db.codex_invocations
        SET response_raw_codec = CASE
                WHEN response_raw_path IS NOT NULL AND response_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(response_raw_codec), '') = ''
           OR (response_raw_codec = 'identity' AND response_raw_path LIKE '%.gz')
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("failed to backfill archive_db.codex_invocations response_raw_codec")?;
    Ok(())
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
        ("request_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("request_raw_size", "INTEGER"),
        ("request_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("request_raw_truncated_reason", "TEXT"),
        ("response_raw_path", "TEXT"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
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

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET request_raw_codec = CASE
                WHEN request_raw_path IS NOT NULL AND request_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(request_raw_codec), '') = ''
           OR (request_raw_codec = 'identity' AND request_raw_path LIKE '%.gz')
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill codex_invocations request_raw_codec")?;

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET response_raw_codec = CASE
                WHEN response_raw_path IS NOT NULL AND response_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(response_raw_codec), '') = ''
           OR (response_raw_codec = 'identity' AND response_raw_path LIKE '%.gz')
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill codex_invocations response_raw_codec")?;

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
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_request_raw_pending
        ON codex_invocations (occurred_at, id)
        WHERE request_raw_path IS NOT NULL
          AND request_raw_codec = 'identity'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_request_raw_pending")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_codex_invocations_response_raw_pending
        ON codex_invocations (occurred_at, id)
        WHERE response_raw_path IS NOT NULL
          AND response_raw_codec = 'identity'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_codex_invocations_response_raw_pending")?;

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
            day_key TEXT,
            part_key TEXT,
            file_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            layout TEXT NOT NULL DEFAULT 'legacy_month',
            codec TEXT NOT NULL DEFAULT 'gzip',
            writer_version TEXT NOT NULL DEFAULT 'legacy_month_v1',
            cleanup_state TEXT NOT NULL DEFAULT 'active',
            superseded_by INTEGER,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            archive_expires_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(dataset, month_key, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batches table existence")?;

    let archive_batch_columns = load_sqlite_table_columns(pool, "archive_batches").await?;
    for (column, ty) in [
        ("day_key", "TEXT"),
        ("part_key", "TEXT"),
        ("layout", "TEXT NOT NULL DEFAULT 'legacy_month'"),
        ("codec", "TEXT NOT NULL DEFAULT 'gzip'"),
        ("writer_version", "TEXT NOT NULL DEFAULT 'legacy_month_v1'"),
        ("cleanup_state", "TEXT NOT NULL DEFAULT 'active'"),
        ("superseded_by", "INTEGER"),
        ("coverage_start_at", "TEXT"),
        ("coverage_end_at", "TEXT"),
        ("archive_expires_at", "TEXT"),
        ("upstream_activity_manifest_refreshed_at", "TEXT"),
        ("historical_rollups_materialized_at", "TEXT"),
    ] {
        if !archive_batch_columns.contains(column) {
            let statement = format!("ALTER TABLE archive_batches ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| format!("failed to add archive_batches column {column}"))?;
        }
    }

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
        CREATE INDEX IF NOT EXISTS idx_archive_batches_dataset_layout_day_part
        ON archive_batches (dataset, layout, day_key, part_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_dataset_layout_day_part")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_invocation_manifest_pending
        ON archive_batches (dataset, status, upstream_activity_manifest_refreshed_at, month_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_invocation_manifest_pending")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batches_rollup_materialization
        ON archive_batches (dataset, status, historical_rollups_materialized_at, month_key, id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batches_rollup_materialization")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS archive_batch_upstream_activity (
            archive_batch_id INTEGER NOT NULL,
            account_id INTEGER NOT NULL,
            last_activity_at TEXT NOT NULL,
            PRIMARY KEY (archive_batch_id, account_id)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure archive_batch_upstream_activity table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batch_upstream_activity_account_last_activity
        ON archive_batch_upstream_activity (account_id, last_activity_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batch_upstream_activity_account_last_activity")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_archive_batch_upstream_activity_batch
        ON archive_batch_upstream_activity (archive_batch_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_archive_batch_upstream_activity_batch")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_materialized_buckets (
            target TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL DEFAULT '',
            materialized_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, bucket_start_epoch, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_materialized_buckets table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_hourly_rollup_materialized_buckets_target_bucket
        ON hourly_rollup_materialized_buckets (target, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_hourly_rollup_materialized_buckets_target_bucket")?;

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
        CREATE TABLE IF NOT EXISTS invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            total_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0,
            first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
            first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
            first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_rollup_hourly table existence")?;

    let invocation_rollup_hourly_columns =
        load_sqlite_table_columns(pool, "invocation_rollup_hourly").await?;
    let mut added_first_response_byte_total_rollup_columns = false;
    for (column, ty) in [
        (
            "first_response_byte_total_sample_count",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_sum_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_max_ms",
            "REAL NOT NULL DEFAULT 0",
        ),
        (
            "first_response_byte_total_histogram",
            "TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]'",
        ),
    ] {
        if !invocation_rollup_hourly_columns.contains(column) {
            added_first_response_byte_total_rollup_columns = true;
            let statement =
                format!("ALTER TABLE invocation_rollup_hourly ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add invocation_rollup_hourly column {column}")
                })?;
        }
    }

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_rollup_hourly_source_bucket
        ON invocation_rollup_hourly (source, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_rollup_hourly_source_bucket")?;
    if added_first_response_byte_total_rollup_columns {
        let rebuilt_rows = backfill_invocation_rollup_hourly_from_sources(pool).await?;
        info!(
            rebuilt_rows,
            "backfilled invocation hourly rollups after adding first-response-byte-total columns"
        );
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invocation_failure_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            failure_class TEXT NOT NULL,
            is_actionable INTEGER NOT NULL DEFAULT 0,
            error_category TEXT NOT NULL,
            failure_count INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, failure_class, is_actionable, error_category)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure invocation_failure_rollup_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_invocation_failure_rollup_hourly_bucket
        ON invocation_failure_rollup_hourly (bucket_start_epoch, source, failure_class)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_invocation_failure_rollup_hourly_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS proxy_perf_stage_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            stage TEXT NOT NULL,
            sample_count INTEGER NOT NULL,
            sum_ms REAL NOT NULL,
            max_ms REAL NOT NULL,
            histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, stage)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure proxy_perf_stage_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_proxy_perf_stage_hourly_stage_bucket
        ON proxy_perf_stage_hourly (stage, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_proxy_perf_stage_hourly_stage_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, prompt_cache_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_rollup_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_rollup_hourly_key_bucket
        ON prompt_cache_rollup_hourly (prompt_cache_key, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_rollup_hourly_key_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS prompt_cache_upstream_account_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            source TEXT NOT NULL,
            prompt_cache_key TEXT NOT NULL,
            upstream_account_key TEXT NOT NULL,
            upstream_account_id INTEGER,
            upstream_account_name TEXT,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source, prompt_cache_key, upstream_account_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure prompt_cache_upstream_account_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_prompt_cache_upstream_account_hourly_key_bucket
        ON prompt_cache_upstream_account_hourly (prompt_cache_key, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_prompt_cache_upstream_account_hourly_key_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS upstream_sticky_key_hourly (
            bucket_start_epoch INTEGER NOT NULL,
            upstream_account_id INTEGER NOT NULL,
            sticky_key TEXT NOT NULL,
            request_count INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL,
            first_seen_at TEXT NOT NULL,
            last_seen_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, upstream_account_id, sticky_key)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure upstream_sticky_key_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_upstream_sticky_key_hourly_account_bucket
        ON upstream_sticky_key_hourly (upstream_account_id, bucket_start_epoch)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_upstream_sticky_key_hourly_account_bucket")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forward_proxy_attempt_hourly (
            proxy_key TEXT NOT NULL,
            bucket_start_epoch INTEGER NOT NULL,
            attempts INTEGER NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            latency_sample_count INTEGER NOT NULL DEFAULT 0,
            latency_sum_ms REAL NOT NULL DEFAULT 0,
            latency_max_ms REAL NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (proxy_key, bucket_start_epoch)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_attempt_hourly table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_forward_proxy_attempt_hourly_bucket_proxy
        ON forward_proxy_attempt_hourly (bucket_start_epoch, proxy_key)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_forward_proxy_attempt_hourly_bucket_proxy")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_archive_replay (
            target TEXT NOT NULL,
            dataset TEXT NOT NULL,
            file_path TEXT NOT NULL,
            replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (target, dataset, file_path)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_archive_replay table existence")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS hourly_rollup_live_progress (
            dataset TEXT PRIMARY KEY,
            cursor_id INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure hourly_rollup_live_progress table existence")?;

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
            upstream_429_max_retries,
            enabled_preset_models_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(DEFAULT_PROXY_MODELS_HIJACK_ENABLED as i64)
    .bind(DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED as i64)
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
        CREATE TABLE IF NOT EXISTS forward_proxy_metadata_history (
            proxy_key TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            source TEXT NOT NULL,
            endpoint_url TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure forward_proxy_metadata_history table existence")?;

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
        CREATE TABLE IF NOT EXISTS pool_upstream_request_attempts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            route_mode TEXT NOT NULL,
            sticky_key TEXT,
            upstream_account_id INTEGER,
            upstream_route_key TEXT,
            attempt_index INTEGER NOT NULL,
            distinct_account_index INTEGER NOT NULL,
            same_account_retry_index INTEGER NOT NULL,
            requester_ip TEXT,
            started_at TEXT,
            finished_at TEXT,
            status TEXT NOT NULL,
            phase TEXT,
            http_status INTEGER,
            failure_kind TEXT,
            error_message TEXT,
            connect_latency_ms REAL,
            first_byte_latency_ms REAL,
            stream_latency_ms REAL,
            upstream_request_id TEXT,
            compact_support_status TEXT,
            compact_support_reason TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_request_attempts table existence")?;

    let existing_pool_attempt_columns =
        load_sqlite_table_columns(pool, "pool_upstream_request_attempts").await?;
    for (column, ty) in [
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
    ] {
        if !existing_pool_attempt_columns.contains(column) {
            let statement =
                format!("ALTER TABLE pool_upstream_request_attempts ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(pool)
                .await
                .with_context(|| {
                    format!("failed to add pool_upstream_request_attempts column {column}")
                })?;
        }
    }

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
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_invoke_attempt
        ON pool_upstream_request_attempts (invoke_id, attempt_index)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_invoke_attempt")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_account_occurred_at
        ON pool_upstream_request_attempts (upstream_account_id, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_account_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_sticky_occurred_at
        ON pool_upstream_request_attempts (sticky_key, occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_sticky_occurred_at")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_occurred_at
        ON pool_upstream_request_attempts (occurred_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure index idx_pool_upstream_request_attempts_occurred_at")?;

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
        SELECT
            hijack_enabled,
            merge_upstream_enabled,
            upstream_429_max_retries,
            enabled_preset_models_json
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
            upstream_429_max_retries = ?3,
            enabled_preset_models_json = ?4,
            updated_at = datetime('now')
        WHERE id = ?5
        "#,
    )
    .bind(settings.hijack_enabled as i64)
    .bind(settings.merge_upstream_enabled as i64)
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
    let invoke_id = format!("proxy-{proxy_request_id}-{}", Utc::now().timestamp_millis());
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
        invoke_id.clone(),
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
        Err(err) => {
            warn!(
                proxy_request_id,
                method = %method_for_log,
                uri = %uri_for_log,
                status = %err.status,
                error = %err.message,
                elapsed_ms = started_at.elapsed().as_millis(),
                "openai proxy request failed"
            );
            match err.cvm_id {
                Some(cvm_id) => {
                    let mut response = (
                        err.status,
                        Json(json!({ "error": err.message, "cvmId": cvm_id })),
                    )
                        .into_response();
                    if let Ok(header_value) = HeaderValue::from_str(&invoke_id) {
                        response
                            .headers_mut()
                            .insert(HeaderName::from_static(CVM_INVOKE_ID_HEADER), header_value);
                    }
                    if let Some(retry_after_secs) = err.retry_after_secs
                        && let Ok(header_value) =
                            HeaderValue::from_str(&retry_after_secs.to_string())
                    {
                        response
                            .headers_mut()
                            .insert(header::RETRY_AFTER, header_value);
                    }
                    response
                }
                None => {
                    let mut response =
                        (err.status, Json(json!({ "error": err.message }))).into_response();
                    if let Some(retry_after_secs) = err.retry_after_secs
                        && let Ok(header_value) =
                            HeaderValue::from_str(&retry_after_secs.to_string())
                    {
                        response
                            .headers_mut()
                            .insert(header::RETRY_AFTER, header_value);
                    }
                    response
                }
            }
        }
    }
}

#[derive(Debug)]
struct ProxyErrorResponse {
    status: StatusCode,
    message: String,
    cvm_id: Option<String>,
    retry_after_secs: Option<u64>,
}

async fn resolve_proxy_request_timeouts(
    state: &AppState,
    pool_route_active: bool,
) -> Result<PoolRoutingTimeoutSettingsResolved> {
    if pool_route_active {
        resolve_pool_routing_timeouts(&state.pool, &state.config).await
    } else {
        Ok(pool_routing_timeouts_from_config(&state.config))
    }
}

#[derive(Debug)]
pub(crate) struct ForwardProxyUpstreamResponse {
    pub(crate) selected_proxy: SelectedForwardProxy,
    pub(crate) response: ProxyUpstreamResponseBody,
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

enum ProxyUpstreamResponseBody {
    Reqwest(reqwest::Response),
    Axum(Response),
}

impl fmt::Debug for ProxyUpstreamResponseBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reqwest(response) => f
                .debug_struct("ProxyUpstreamResponseBody::Reqwest")
                .field("status", &response.status())
                .finish(),
            Self::Axum(response) => f
                .debug_struct("ProxyUpstreamResponseBody::Axum")
                .field("status", &response.status())
                .finish(),
        }
    }
}

impl ProxyUpstreamResponseBody {
    fn status(&self) -> StatusCode {
        match self {
            Self::Reqwest(response) => response.status(),
            Self::Axum(response) => response.status(),
        }
    }

    fn headers(&self) -> &HeaderMap {
        match self {
            Self::Reqwest(response) => response.headers(),
            Self::Axum(response) => response.headers(),
        }
    }

    async fn into_bytes(self) -> Result<Bytes, String> {
        match self {
            Self::Reqwest(response) => response.bytes().await.map_err(|err| err.to_string()),
            Self::Axum(response) => axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .map_err(|err| err.to_string()),
        }
    }

    async fn into_first_chunk(self) -> Result<(Self, Option<Bytes>), String> {
        match self {
            Self::Reqwest(mut response) => {
                let first_chunk = response.chunk().await.map_err(|err| err.to_string())?;
                Ok((Self::Reqwest(response), first_chunk))
            }
            Self::Axum(response) => {
                let (parts, body) = response.into_parts();
                let mut stream = body.into_data_stream();
                let first_chunk = match stream.next().await {
                    Some(Ok(chunk)) => Some(chunk),
                    Some(Err(err)) => return Err(err.to_string()),
                    None => None,
                };
                let response = Response::from_parts(parts, Body::from_stream(stream));
                Ok((Self::Axum(response), first_chunk))
            }
        }
    }

    fn into_bytes_stream(
        self,
    ) -> Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>> {
        match self {
            Self::Reqwest(response) => Box::pin(
                response
                    .bytes_stream()
                    .map_err(|err| io::Error::other(err.to_string())),
            ),
            Self::Axum(response) => Box::pin(
                response
                    .into_body()
                    .into_data_stream()
                    .map_err(|err| io::Error::other(err.to_string())),
            ),
        }
    }
}

fn pool_upstream_timeout_message(total_timeout: Duration, phase: &str) -> String {
    format!(
        "request timed out after {}ms while {phase}",
        total_timeout.as_millis()
    )
}

fn proxy_request_send_timeout_message(
    capture_target: Option<ProxyCaptureTarget>,
    total_timeout: Duration,
) -> String {
    match capture_target {
        Some(ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact) => {
            pool_upstream_timeout_message(total_timeout, "waiting for first upstream chunk")
        }
        _ => format!(
            "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
            total_timeout.as_millis()
        ),
    }
}

async fn read_pool_upstream_bytes_with_timeout(
    response: ProxyUpstreamResponseBody,
    total_timeout: Duration,
    started: Instant,
    phase: &str,
) -> Result<Bytes, String> {
    let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed()) else {
        return Err(pool_upstream_timeout_message(total_timeout, phase));
    };

    match timeout(timeout_budget, response.into_bytes()).await {
        Ok(result) => result,
        Err(_) => Err(pool_upstream_timeout_message(total_timeout, phase)),
    }
}

async fn read_pool_upstream_first_chunk_with_timeout(
    response: ProxyUpstreamResponseBody,
    total_timeout: Duration,
    started: Instant,
) -> Result<(ProxyUpstreamResponseBody, Option<Bytes>), String> {
    let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed()) else {
        return Err(pool_upstream_timeout_message(
            total_timeout,
            "waiting for first upstream chunk",
        ));
    };

    match timeout(timeout_budget, response.into_first_chunk()).await {
        Ok(result) => result,
        Err(_) => Err(pool_upstream_timeout_message(
            total_timeout,
            "waiting for first upstream chunk",
        )),
    }
}

#[derive(Debug)]
pub(crate) struct PoolUpstreamResponse {
    pub(crate) account: PoolResolvedAccount,
    pub(crate) response: ProxyUpstreamResponseBody,
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) connect_latency_ms: f64,
    pub(crate) attempt_started_at_utc: DateTime<Utc>,
    pub(crate) first_byte_latency_ms: f64,
    pub(crate) first_chunk: Option<Bytes>,
    pub(crate) pending_attempt_record: Option<PendingPoolAttemptRecord>,
    pub(crate) attempt_summary: PoolAttemptSummary,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) request_body_for_capture: Option<Bytes>,
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
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) attempt_summary: PoolAttemptSummary,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) request_body_for_capture: Option<Bytes>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PoolAttemptSummary {
    pub(crate) pool_attempt_count: usize,
    pub(crate) pool_distinct_account_count: usize,
    pub(crate) pool_attempt_terminal_reason: Option<String>,
}

fn pool_attempt_summary(
    pool_attempt_count: usize,
    pool_distinct_account_count: usize,
    pool_attempt_terminal_reason: Option<String>,
) -> PoolAttemptSummary {
    PoolAttemptSummary {
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
    }
}

fn pool_upstream_error_is_rate_limited(err: &PoolUpstreamError) -> bool {
    err.status == StatusCode::TOO_MANY_REQUESTS
        || matches!(
            err.failure_kind,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                | PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED
        )
}

fn build_pool_rate_limited_error(
    attempt_count: usize,
    distinct_account_count: usize,
    failure_kind: &'static str,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: None,
        status: StatusCode::TOO_MANY_REQUESTS,
        message: POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
        failure_kind,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(failure_kind.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

fn build_pool_no_available_account_error(
    attempt_count: usize,
    distinct_account_count: usize,
    _retry_after_secs: u64,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

fn retry_after_secs_for_proxy_error(status: StatusCode, message: &str) -> Option<u64> {
    (status == StatusCode::SERVICE_UNAVAILABLE && message == POOL_NO_AVAILABLE_ACCOUNT_MESSAGE)
        .then_some(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS)
}

fn build_pool_degraded_only_error(
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
        failure_kind: PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PendingPoolAttemptRecord {
    pub(crate) attempt_id: Option<i64>,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) upstream_account_id: i64,
    pub(crate) upstream_route_key: String,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    pub(crate) started_at: String,
    pub(crate) connect_latency_ms: f64,
    pub(crate) first_byte_latency_ms: f64,
    pub(crate) compact_support_status: Option<String>,
    pub(crate) compact_support_reason: Option<String>,
}

#[derive(Debug, Default)]
struct PoolFailoverProgress {
    excluded_account_ids: Vec<i64>,
    excluded_upstream_route_keys: HashSet<String>,
    attempt_count: usize,
    last_error: Option<PoolUpstreamError>,
    timeout_route_failover_pending: bool,
    responses_total_timeout_started_at: Option<Instant>,
    no_available_wait_deadline: Option<Instant>,
}

#[derive(Debug, Clone)]
struct PoolUpstreamAttemptTraceContext {
    invoke_id: String,
    occurred_at: String,
    endpoint: String,
    sticky_key: Option<String>,
    requester_ip: Option<String>,
}

#[derive(Debug, Clone)]
struct PoolAttemptRuntimeSnapshotContext {
    capture_target: ProxyCaptureTarget,
    request_info: RequestCaptureInfo,
    prompt_cache_key: Option<String>,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
}

const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING: &str = "pending";
struct CompactSupportObservation {
    status: &'static str,
    reason: Option<String>,
}
const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS: &str = "success";
const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE: &str = "http_failure";
const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE: &str = "transport_failure";
const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL: &str = "budget_exhausted_final";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING: &str = "connecting";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST: &str = "sending_request";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE: &str = "waiting_first_byte";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE: &str = "streaming_response";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED: &str = "completed";
const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED: &str = "failed";
const POOL_UPSTREAM_MAX_DISTINCT_ACCOUNTS: usize = 3;
const POOL_UPSTREAM_RESPONSES_MAX_TIMEOUT_ROUTE_KEYS: usize = 3;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpstreamAccountFailureDisposition {
    HardUnavailable,
    RateLimited,
    Retryable,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UpstreamAccountHttpFailureClassification {
    pub(crate) disposition: UpstreamAccountFailureDisposition,
    pub(crate) failure_kind: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) next_account_status: Option<&'static str>,
}

pub(crate) fn upstream_error_indicates_quota_exhausted(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "insufficient_quota",
        "quota exhausted",
        "quota_exhausted",
        "the usage limit has been reached",
        "usage limit has been reached",
        "usage limit reached",
        "billing",
        "payment required",
        "subscription required",
        "weekly cap",
        "weekly limit",
        "plan limit",
        "plan quota",
        "check your plan",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn upstream_error_code_is_server_overloaded(code: Option<&str>) -> bool {
    code.is_some_and(|value| value.eq_ignore_ascii_case(UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED))
}

fn route_http_failure_is_retryable_server_overloaded(
    status: StatusCode,
    error_message: &str,
) -> bool {
    if status != StatusCode::OK {
        return false;
    }

    let normalized = error_message.to_ascii_lowercase();
    normalized.contains(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        && normalized.contains(UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED)
}

fn response_info_is_retryable_server_overloaded(
    status: StatusCode,
    response_info: &ResponseCaptureInfo,
) -> bool {
    status == StatusCode::OK
        && response_info.stream_terminal_event.is_some()
        && upstream_error_code_is_server_overloaded(response_info.upstream_error_code.as_deref())
}

pub(crate) fn classify_pool_account_http_failure(
    account_kind: &str,
    status: StatusCode,
    error_message: &str,
) -> UpstreamAccountHttpFailureClassification {
    if status == StatusCode::TOO_MANY_REQUESTS
        && upstream_error_indicates_quota_exhausted(error_message)
    {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            reason_code: "upstream_http_429_quota_exhausted",
            next_account_status: Some("error"),
        };
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::RateLimited,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
            reason_code: "upstream_http_429_rate_limit",
            next_account_status: None,
        };
    }
    if status == StatusCode::PAYMENT_REQUIRED {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_402,
            reason_code: "upstream_http_402",
            next_account_status: Some("error"),
        };
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        let next_account_status = if account_kind == "oauth_codex"
            && is_explicit_reauth_error_message(error_message)
            && !is_scope_permission_error_message(error_message)
            && !is_bridge_error_message(error_message)
        {
            Some("needs_reauth")
        } else {
            Some("error")
        };
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            reason_code: if status == StatusCode::UNAUTHORIZED {
                "upstream_http_401"
            } else {
                "upstream_http_403"
            },
            next_account_status,
        };
    }
    if status.is_server_error() {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::Retryable,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
            reason_code: "upstream_http_5xx",
            next_account_status: None,
        };
    }
    UpstreamAccountHttpFailureClassification {
        disposition: UpstreamAccountFailureDisposition::Retryable,
        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        reason_code: "sync_error",
        next_account_status: None,
    }
}

fn compact_support_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    let has_compact_signal = normalized.contains("compact")
        || normalized.contains("responses/compact")
        || normalized.contains("gpt-5.4-openai-compact");
    if normalized.contains("no available channel for model") && has_compact_signal {
        return true;
    }
    has_compact_signal
        && [
            "unsupported model",
            "unsupported endpoint",
            "unsupported path",
            "unsupported route",
            "not support",
            "does not support",
            "is not supported",
            "unknown model",
            "model not found",
            "no channel",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
}

fn classify_compact_support_observation(
    original_uri: &Uri,
    status: Option<StatusCode>,
    message: Option<&str>,
) -> Option<CompactSupportObservation> {
    if original_uri.path() != "/v1/responses/compact" {
        return None;
    }
    match status {
        Some(code) if code.is_success() => Some(CompactSupportObservation {
            status: COMPACT_SUPPORT_STATUS_SUPPORTED,
            reason: Some("compact request succeeded".to_string()),
        }),
        _ => {
            let message = message
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            if message
                .as_deref()
                .is_some_and(compact_support_negative_signal)
            {
                Some(CompactSupportObservation {
                    status: COMPACT_SUPPORT_STATUS_UNSUPPORTED,
                    reason: message,
                })
            } else {
                Some(CompactSupportObservation {
                    status: COMPACT_SUPPORT_STATUS_UNKNOWN,
                    reason: message,
                })
            }
        }
    }
}

fn fallback_proxy_429_retry_delay(retry_index: u32) -> Duration {
    let exponent = retry_index.saturating_sub(1).min(16);
    let multiplier = 1_u64 << exponent;
    Duration::from_millis(500_u64.saturating_mul(multiplier)).min(Duration::from_secs(5))
}

fn pool_group_upstream_429_retry_delay(state: &AppState) -> Duration {
    if let Some(delay) = state.pool_group_429_retry_delay_override {
        return delay;
    }
    Duration::from_secs(rand::thread_rng().gen_range(
        MIN_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS..=MAX_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS,
    ))
}

const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_POLL_INTERVAL_MS: u64 = 250;
const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS: u64 = 10;
const POOL_NO_AVAILABLE_ACCOUNT_MESSAGE: &str = "no healthy pool account is available";

#[derive(Debug, Clone, Copy)]
struct PoolNoAvailableWaitSettings {
    timeout: Duration,
    poll_interval: Duration,
    retry_after_secs: u64,
}

impl Default for PoolNoAvailableWaitSettings {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_TIMEOUT_SECS),
            poll_interval: Duration::from_millis(
                DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_POLL_INTERVAL_MS,
            ),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        }
    }
}

impl PoolNoAvailableWaitSettings {
    fn normalized_poll_interval(self) -> Duration {
        if self.poll_interval.is_zero() {
            Duration::from_millis(1)
        } else {
            self.poll_interval
        }
    }
}

const POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS: u8 = 3;
const OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES: usize = 8 * 1024 * 1024;

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

    async fn to_bytes(&self) -> io::Result<Bytes> {
        match self {
            Self::Empty => Ok(Bytes::new()),
            Self::Memory(bytes) => Ok(bytes.clone()),
            Self::File { temp_file, .. } => tokio::fs::read(&temp_file.path).await.map(Bytes::from),
        }
    }

    async fn to_prefix_bytes(&self, limit: usize) -> io::Result<Bytes> {
        match self {
            Self::Empty => Ok(Bytes::new()),
            Self::Memory(bytes) => Ok(bytes.slice(..bytes.len().min(limit))),
            Self::File { temp_file, .. } => {
                let mut file = tokio::fs::File::open(&temp_file.path).await?;
                let mut buf = vec![0_u8; limit];
                let read_len = file.read(&mut buf).await?;
                buf.truncate(read_len);
                Ok(Bytes::from(buf))
            }
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedPoolRequestBody {
    snapshot: PoolReplayBodySnapshot,
    request_body_for_capture: Option<Bytes>,
    requested_service_tier: Option<String>,
}

async fn prepare_pool_request_body_for_account(
    body: Option<&PoolReplayBodySnapshot>,
    original_uri: &Uri,
    method: &Method,
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
) -> Result<PreparedPoolRequestBody, String> {
    let capture_target = capture_target_for_request(original_uri.path(), method);
    let rewrite_required = capture_target.is_some_and(|target| target.allows_fast_mode_rewrite())
        && fast_mode_rewrite_mode != TagFastModeRewriteMode::KeepOriginal;

    let Some(snapshot) = body.cloned() else {
        return Ok(PreparedPoolRequestBody {
            snapshot: PoolReplayBodySnapshot::Empty,
            request_body_for_capture: Some(Bytes::new()),
            requested_service_tier: None,
        });
    };

    if !rewrite_required {
        let (request_body_for_capture, requested_service_tier) = match &snapshot {
            PoolReplayBodySnapshot::Empty => (Some(Bytes::new()), None),
            PoolReplayBodySnapshot::Memory(bytes) => {
                let requested_service_tier = serde_json::from_slice::<Value>(bytes)
                    .ok()
                    .and_then(|value| extract_requested_service_tier_from_request_body(&value));
                (Some(bytes.clone()), requested_service_tier)
            }
            PoolReplayBodySnapshot::File { .. } => (None, None),
        };
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture,
            requested_service_tier,
        });
    }

    let original_bytes = snapshot
        .to_bytes()
        .await
        .map_err(|err| format!("failed to materialize pool request body for rewrite: {err}"))?;
    let Some(target) = capture_target else {
        return Ok(PreparedPoolRequestBody {
            snapshot: PoolReplayBodySnapshot::Memory(original_bytes.clone()),
            request_body_for_capture: Some(original_bytes),
            requested_service_tier: None,
        });
    };
    let mut value = match serde_json::from_slice::<Value>(&original_bytes) {
        Ok(value) => value,
        Err(_) => {
            return Ok(PreparedPoolRequestBody {
                snapshot: PoolReplayBodySnapshot::Memory(original_bytes.clone()),
                request_body_for_capture: Some(original_bytes),
                requested_service_tier: None,
            });
        }
    };

    let rewritten = if target.allows_fast_mode_rewrite() {
        rewrite_request_service_tier_for_fast_mode(&mut value, fast_mode_rewrite_mode)
    } else {
        false
    };
    let requested_service_tier = extract_requested_service_tier_from_request_body(&value);
    if !rewritten {
        return Ok(PreparedPoolRequestBody {
            snapshot: PoolReplayBodySnapshot::Memory(original_bytes.clone()),
            request_body_for_capture: Some(original_bytes),
            requested_service_tier,
        });
    }

    let rewritten_bytes = serde_json::to_vec(&value)
        .map(Bytes::from)
        .map_err(|err| format!("failed to serialize rewritten pool request body: {err}"))?;
    Ok(PreparedPoolRequestBody {
        snapshot: PoolReplayBodySnapshot::Memory(rewritten_bytes.clone()),
        request_body_for_capture: Some(rewritten_bytes.clone()),
        requested_service_tier,
    })
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
    request_read_timeout: Duration,
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
        let read_deadline = Instant::now() + request_read_timeout;

        loop {
            if cancel_for_task.is_cancelled() {
                let _ = status_tx.send(PoolReplayBodyStatus::Incomplete);
                return;
            }

            let remaining = read_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let read_error = RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: Vec::new(),
                };
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data_len,
                    "openai proxy request body read timed out"
                );
                let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                let _ = tx
                    .send(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        read_error.message,
                    )))
                    .await;
                return;
            }

            let next_chunk = tokio::select! {
                _ = cancel_for_task.cancelled() => {
                    let _ = status_tx.send(PoolReplayBodyStatus::Incomplete);
                    return;
                }
                chunk = timeout(remaining, stream.next()) => {
                    match chunk {
                        Ok(chunk) => chunk,
                        Err(_) => {
                            let read_error = RequestBodyReadError {
                                status: StatusCode::REQUEST_TIMEOUT,
                                message: format!(
                                    "request body read timed out after {}ms",
                                    request_read_timeout.as_millis()
                                ),
                                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                                partial_body: Vec::new(),
                            };
                            warn!(
                                proxy_request_id,
                                timeout_ms = request_read_timeout.as_millis(),
                                read_bytes = data_len,
                                "openai proxy request body read timed out"
                            );
                            let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                            let _ = tx
                                .send(Err(io::Error::new(
                                    io::ErrorKind::TimedOut,
                                    read_error.message,
                                )))
                                .await;
                            return;
                        }
                    }
                }
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
                    let read_error = RequestBodyReadError {
                        status: StatusCode::BAD_REQUEST,
                        message: msg,
                        failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                        partial_body: Vec::new(),
                    };
                    let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                    let _ = tx.send(Err(io::Error::other(read_error.message))).await;
                    return;
                }
            };

            if data_len.saturating_add(chunk.len()) > body_limit {
                let read_error = RequestBodyReadError {
                    status: StatusCode::PAYLOAD_TOO_LARGE,
                    message: format!("request body exceeds {body_limit} bytes"),
                    failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                    partial_body: Vec::new(),
                };
                let _ = status_tx.send(PoolReplayBodyStatus::ReadError(read_error.clone()));
                let _ = tx.send(Err(io::Error::other(read_error.message))).await;
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

async fn resolve_pool_account_for_request_with_wait(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &HashSet<String>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<Instant>,
    total_timeout_deadline: Option<Instant>,
) -> Result<PoolAccountResolution> {
    let poll_interval = state.pool_no_available_wait.normalized_poll_interval();

    loop {
        let resolution = resolve_pool_account_for_request(
            state,
            sticky_key,
            excluded_ids,
            excluded_upstream_route_keys,
        )
        .await?;
        match resolution {
            PoolAccountResolution::Unavailable | PoolAccountResolution::NoCandidate
                if wait_for_no_available =>
            {
                let wait_deadline = if let Some(deadline) = *wait_deadline {
                    deadline
                } else {
                    let deadline = Instant::now() + state.pool_no_available_wait.timeout;
                    *wait_deadline = Some(deadline);
                    deadline
                };
                let effective_deadline = total_timeout_deadline
                    .map(|deadline| std::cmp::min(wait_deadline, deadline))
                    .unwrap_or(wait_deadline);
                let now = Instant::now();
                if now >= effective_deadline {
                    return Ok(resolution);
                }
                tokio::time::sleep(
                    poll_interval.min(effective_deadline.saturating_duration_since(now)),
                )
                .await;
            }
            _ => return Ok(resolution),
        }
    }
}

async fn send_pool_request_with_failover(
    state: Arc<AppState>,
    proxy_request_id: u64,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: Option<PoolReplayBodySnapshot>,
    handshake_timeout: Duration,
    trace_context: Option<PoolUpstreamAttemptTraceContext>,
    runtime_snapshot_context: Option<PoolAttemptRuntimeSnapshotContext>,
    sticky_key: Option<&str>,
    preferred_account: Option<PoolResolvedAccount>,
    failover_progress: PoolFailoverProgress,
    same_account_attempts: u8,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    let request_connection_scoped = connection_scoped_header_names(headers);
    let reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let mut reservation_guard =
        PoolRoutingReservationDropGuard::new(state.clone(), reservation_key.clone());
    let runtime_timeouts = resolve_pool_routing_timeouts(&state.pool, &state.config)
        .await
        .map_err(|err| PoolUpstreamError {
            account: preferred_account.clone(),
            status: StatusCode::BAD_GATEWAY,
            message: format!("failed to resolve pool routing timeouts: {err}"),
            failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
            connect_latency_ms: 0.0,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
            oauth_responses_debug: None,
            attempt_summary: pool_attempt_summary(
                failover_progress.attempt_count,
                failover_progress.excluded_account_ids.len(),
                Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT.to_string()),
            ),
            requested_service_tier: None,
            request_body_for_capture: None,
        })?;
    let pre_first_byte_timeout =
        pool_upstream_first_chunk_timeout(&runtime_timeouts, original_uri, &method);
    let uses_timeout_route_failover =
        pool_uses_responses_timeout_failover_policy(original_uri, &method);
    let responses_total_timeout =
        pool_upstream_responses_total_timeout(&state.config, original_uri, &method);
    let mut responses_total_timeout_started_at =
        failover_progress.responses_total_timeout_started_at;
    let send_timeout = pool_upstream_send_timeout(
        original_uri,
        &method,
        handshake_timeout,
        pre_first_byte_timeout,
    );
    let mut excluded_ids = failover_progress.excluded_account_ids;
    let mut excluded_upstream_route_keys = failover_progress.excluded_upstream_route_keys;
    let mut last_error = failover_progress.last_error;
    let mut attempted_account_ids = excluded_ids.iter().copied().collect::<HashSet<_>>();
    if let Some(account_id) = last_error
        .as_ref()
        .and_then(|error| error.account.as_ref())
        .map(|account| account.account_id)
    {
        attempted_account_ids.insert(account_id);
    }
    let initial_errors_all_rate_limited = if failover_progress.attempt_count == 0 {
        true
    } else {
        last_error
            .as_ref()
            .is_some_and(pool_upstream_error_is_rate_limited)
    };
    let mut preferred_account = preferred_account
        .filter(|account| !excluded_upstream_route_keys.contains(&account.upstream_route_key()));
    let initial_same_account_attempts = same_account_attempts.max(1);
    let mut attempt_count = failover_progress.attempt_count;
    let mut timeout_route_failover_pending = failover_progress.timeout_route_failover_pending;
    let mut exhausted_accounts_all_rate_limited = initial_errors_all_rate_limited;
    let mut no_available_wait_deadline = failover_progress.no_available_wait_deadline;

    'account_loop: loop {
        let mut distinct_account_count = attempted_account_ids.len();
        if let (Some(total_timeout), Some(started_at)) =
            (responses_total_timeout, responses_total_timeout_started_at)
            && pool_total_timeout_exhausted(total_timeout, started_at)
        {
            let final_error = build_pool_total_timeout_exhausted_error(
                total_timeout,
                last_error,
                attempt_count,
                distinct_account_count,
            );
            if let Some(trace) = trace_context.as_ref()
                && let Err(err) = insert_pool_upstream_terminal_attempt(
                    &state.pool,
                    trace,
                    &final_error,
                    (attempt_count + 1) as i64,
                    distinct_account_count as i64,
                    PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
                )
                .await
            {
                warn!(
                    invoke_id = trace.invoke_id,
                    error = %err,
                    "failed to persist pool total-timeout exhaustion attempt"
                );
            }
            return Err(final_error);
        }
        if preferred_account.is_none()
            && (excluded_ids.len() >= POOL_UPSTREAM_MAX_DISTINCT_ACCOUNTS
                || (uses_timeout_route_failover
                    && timeout_route_failover_pending
                    && excluded_upstream_route_keys.len()
                        >= POOL_UPSTREAM_RESPONSES_MAX_TIMEOUT_ROUTE_KEYS))
        {
            let terminal_failure_kind =
                if uses_timeout_route_failover && timeout_route_failover_pending {
                    PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                } else {
                    PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED
                };
            let terminal_message = if terminal_failure_kind
                == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
            {
                "no alternate upstream route is available after timeout".to_string()
            } else {
                "pool distinct-account retry budget exhausted".to_string()
            };
            let mut final_error = last_error.unwrap_or(PoolUpstreamError {
                account: None,
                status: StatusCode::BAD_GATEWAY,
                message: terminal_message.clone(),
                failure_kind: terminal_failure_kind,
                connect_latency_ms: 0.0,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
                oauth_responses_debug: None,
                attempt_summary: PoolAttemptSummary::default(),
                requested_service_tier: None,
                request_body_for_capture: None,
            });
            if exhausted_accounts_all_rate_limited && distinct_account_count > 0 {
                final_error.status = StatusCode::TOO_MANY_REQUESTS;
                final_error.message = POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string();
                final_error.failure_kind = PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED;
                final_error.upstream_error_code = None;
                final_error.upstream_error_message = None;
                final_error.upstream_request_id = None;
            } else if terminal_failure_kind
                == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
            {
                final_error.status = StatusCode::BAD_GATEWAY;
                final_error.message = terminal_message;
                final_error.failure_kind = terminal_failure_kind;
                final_error.upstream_error_code = None;
                final_error.upstream_error_message = None;
                final_error.upstream_request_id = None;
            } else if final_error.status != StatusCode::TOO_MANY_REQUESTS {
                final_error.status = StatusCode::BAD_GATEWAY;
                final_error.message = terminal_message;
                final_error.failure_kind = terminal_failure_kind;
                final_error.upstream_error_code = None;
                final_error.upstream_error_message = None;
                final_error.upstream_request_id = None;
            }
            final_error.attempt_summary = pool_attempt_summary(
                attempt_count,
                distinct_account_count,
                Some(terminal_failure_kind.to_string()),
            );
            if let Some(trace) = trace_context.as_ref()
                && let Err(err) = insert_and_broadcast_pool_upstream_terminal_attempt(
                    state.as_ref(),
                    trace,
                    &final_error,
                    (attempt_count + 1) as i64,
                    distinct_account_count as i64,
                    terminal_failure_kind,
                )
                .await
            {
                warn!(
                    invoke_id = trace.invoke_id,
                    error = %err,
                    "failed to persist pool budget exhaustion attempt"
                );
            }
            return Err(final_error);
        }

        let account = if let Some(account) = preferred_account.take() {
            account
        } else {
            let wait_for_no_available = attempt_count == 0
                && last_error.is_none()
                && !(uses_timeout_route_failover && timeout_route_failover_pending)
                && !(exhausted_accounts_all_rate_limited && distinct_account_count > 0);
            let total_timeout_deadline =
                match (responses_total_timeout, responses_total_timeout_started_at) {
                    (Some(total_timeout), Some(started_at)) => Some(started_at + total_timeout),
                    _ => None,
                };
            match resolve_pool_account_for_request_with_wait(
                state.as_ref(),
                sticky_key,
                &excluded_ids,
                &excluded_upstream_route_keys,
                wait_for_no_available,
                &mut no_available_wait_deadline,
                total_timeout_deadline,
            )
            .await
            {
                Ok(PoolAccountResolution::Resolved(account)) => account,
                Ok(PoolAccountResolution::RateLimited) => {
                    return Err(build_pool_rate_limited_error(
                        attempt_count,
                        distinct_account_count,
                        PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED,
                    ));
                }
                Ok(PoolAccountResolution::DegradedOnly) => {
                    return Err(build_pool_degraded_only_error(
                        attempt_count,
                        distinct_account_count,
                    ));
                }
                Ok(PoolAccountResolution::Unavailable) => {
                    let terminal_failure_kind =
                        if uses_timeout_route_failover && timeout_route_failover_pending {
                            PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                        } else {
                            PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT
                        };
                    let mut err = if terminal_failure_kind
                        == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                    {
                        last_error.unwrap_or(PoolUpstreamError {
                            account: None,
                            status: StatusCode::BAD_GATEWAY,
                            message: "no alternate upstream route is available after timeout"
                                .to_string(),
                            failure_kind: terminal_failure_kind,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            upstream_request_id: None,
                            oauth_responses_debug: None,
                            attempt_summary: PoolAttemptSummary::default(),
                            requested_service_tier: None,
                            request_body_for_capture: None,
                        })
                    } else {
                        build_pool_no_available_account_error(
                            attempt_count,
                            distinct_account_count,
                            state.pool_no_available_wait.retry_after_secs,
                        )
                    };
                    if terminal_failure_kind
                        == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                    {
                        err.status = StatusCode::BAD_GATEWAY;
                        err.message =
                            "no alternate upstream route is available after timeout".to_string();
                        err.failure_kind = terminal_failure_kind;
                        err.upstream_error_code = None;
                        err.upstream_error_message = None;
                        err.upstream_request_id = None;
                    }
                    err.attempt_summary = pool_attempt_summary(
                        attempt_count,
                        distinct_account_count,
                        Some(terminal_failure_kind.to_string()),
                    );
                    if terminal_failure_kind
                        == PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT
                        && let Some(trace) = trace_context.as_ref()
                        && let Err(record_err) =
                            insert_and_broadcast_pool_upstream_terminal_attempt(
                                state.as_ref(),
                                trace,
                                &err,
                                (attempt_count + 1) as i64,
                                distinct_account_count as i64,
                                terminal_failure_kind,
                            )
                            .await
                    {
                        warn!(
                            invoke_id = trace.invoke_id,
                            error = %record_err,
                            "failed to persist pool no-alternate-after-timeout attempt"
                        );
                    }
                    return Err(err);
                }
                Ok(PoolAccountResolution::NoCandidate) => {
                    if uses_timeout_route_failover && timeout_route_failover_pending {
                        let mut err = last_error.unwrap_or(PoolUpstreamError {
                            account: None,
                            status: StatusCode::BAD_GATEWAY,
                            message: "no alternate upstream route is available after timeout"
                                .to_string(),
                            failure_kind: PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT,
                            connect_latency_ms: 0.0,
                            upstream_error_code: None,
                            upstream_error_message: None,
                            upstream_request_id: None,
                            oauth_responses_debug: None,
                            attempt_summary: PoolAttemptSummary::default(),
                            requested_service_tier: None,
                            request_body_for_capture: None,
                        });
                        err.status = StatusCode::BAD_GATEWAY;
                        err.message =
                            "no alternate upstream route is available after timeout".to_string();
                        err.failure_kind = PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT;
                        err.upstream_error_code = None;
                        err.upstream_error_message = None;
                        err.upstream_request_id = None;
                        err.attempt_summary = pool_attempt_summary(
                            attempt_count,
                            distinct_account_count,
                            Some(
                                PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT.to_string(),
                            ),
                        );
                        if let Some(trace) = trace_context.as_ref()
                            && let Err(record_err) =
                                insert_and_broadcast_pool_upstream_terminal_attempt(
                                    state.as_ref(),
                                    trace,
                                    &err,
                                    (attempt_count + 1) as i64,
                                    distinct_account_count as i64,
                                    PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT,
                                )
                                .await
                        {
                            warn!(
                                invoke_id = trace.invoke_id,
                                error = %record_err,
                                "failed to persist pool no-candidate no-alternate attempt"
                            );
                        }
                        return Err(err);
                    }

                    return Err(
                        if exhausted_accounts_all_rate_limited && distinct_account_count > 0 {
                            build_pool_rate_limited_error(
                                attempt_count,
                                distinct_account_count,
                                PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED,
                            )
                        } else {
                            let mut err = last_error.unwrap_or_else(|| {
                                build_pool_no_available_account_error(
                                    attempt_count,
                                    distinct_account_count,
                                    state.pool_no_available_wait.retry_after_secs,
                                )
                            });
                            err.attempt_summary = pool_attempt_summary(
                                attempt_count,
                                distinct_account_count,
                                Some(err.failure_kind.to_string()),
                            );
                            err
                        },
                    );
                }
                Ok(PoolAccountResolution::BlockedByPolicy(message)) => {
                    let terminal_failure_kind = PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT;
                    let err = PoolUpstreamError {
                        account: None,
                        status: StatusCode::SERVICE_UNAVAILABLE,
                        message,
                        failure_kind: terminal_failure_kind,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                        oauth_responses_debug: None,
                        attempt_summary: pool_attempt_summary(
                            attempt_count,
                            distinct_account_count,
                            Some(terminal_failure_kind.to_string()),
                        ),
                        requested_service_tier: None,
                        request_body_for_capture: None,
                    };
                    return Err(err);
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
                        oauth_responses_debug: None,
                        attempt_summary: pool_attempt_summary(
                            attempt_count,
                            distinct_account_count,
                            Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT.to_string()),
                        ),
                        requested_service_tier: None,
                        request_body_for_capture: None,
                    });
                }
            }
        };
        reserve_pool_routing_account(state.as_ref(), &reservation_key, &account);
        timeout_route_failover_pending = false;

        excluded_ids.push(account.account_id);
        attempted_account_ids.insert(account.account_id);
        distinct_account_count = attempted_account_ids.len();
        let distinct_account_index = distinct_account_count as i64;
        let upstream_route_key = account.upstream_route_key();
        let api_key_target_url = match &account.auth {
            PoolResolvedAuth::ApiKey { .. } => {
                match build_proxy_upstream_url(&account.upstream_base_url, original_uri) {
                    Ok(url) => Some(url),
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
                            oauth_responses_debug: None,
                            attempt_summary: pool_attempt_summary(
                                attempt_count,
                                distinct_account_count,
                                Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                            ),
                            requested_service_tier: None,
                            request_body_for_capture: None,
                        });
                    }
                }
            }
            PoolResolvedAuth::Oauth { .. } => None,
        };
        let same_account_attempt_budget = pool_same_account_attempt_budget(
            original_uri,
            &method,
            distinct_account_count,
            initial_same_account_attempts,
        );
        let group_upstream_429_max_retries = account.effective_group_upstream_429_max_retries();
        let same_account_attempt_loop_budget =
            same_account_attempt_budget.saturating_add(group_upstream_429_max_retries);
        let mut group_upstream_429_retry_count = 0_u8;

        for same_account_attempt in 0..same_account_attempt_loop_budget {
            let attempt_total_timeout_started_at = ensure_pool_total_timeout_started_at(
                responses_total_timeout,
                &mut responses_total_timeout_started_at,
            );
            let Some(attempt_pre_first_byte_timeout) = pool_timeout_budget_with_total_limit(
                pre_first_byte_timeout,
                responses_total_timeout,
                attempt_total_timeout_started_at,
            ) else {
                let final_error = build_pool_total_timeout_exhausted_error(
                    responses_total_timeout.expect("responses total timeout should be present"),
                    last_error,
                    attempt_count,
                    distinct_account_count,
                );
                if let Some(trace) = trace_context.as_ref()
                    && let Err(err) = insert_pool_upstream_terminal_attempt(
                        &state.pool,
                        trace,
                        &final_error,
                        (attempt_count + 1) as i64,
                        distinct_account_count as i64,
                        PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
                    )
                    .await
                {
                    warn!(
                        invoke_id = trace.invoke_id,
                        error = %err,
                        "failed to persist pool total-timeout exhaustion attempt"
                    );
                }
                return Err(final_error);
            };
            let Some(attempt_send_timeout) = pool_timeout_budget_with_total_limit(
                send_timeout,
                responses_total_timeout,
                attempt_total_timeout_started_at,
            ) else {
                let final_error = build_pool_total_timeout_exhausted_error(
                    responses_total_timeout.expect("responses total timeout should be present"),
                    last_error,
                    attempt_count,
                    distinct_account_count,
                );
                if let Some(trace) = trace_context.as_ref()
                    && let Err(err) = insert_pool_upstream_terminal_attempt(
                        &state.pool,
                        trace,
                        &final_error,
                        (attempt_count + 1) as i64,
                        distinct_account_count as i64,
                        PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
                    )
                    .await
                {
                    warn!(
                        invoke_id = trace.invoke_id,
                        error = %err,
                        "failed to persist pool total-timeout exhaustion attempt"
                    );
                }
                return Err(final_error);
            };
            let same_account_retry_index = i64::from(same_account_attempt) + 1;
            let attempt_started_at_utc = Utc::now();
            let connect_started = Instant::now();
            let attempt_started_at: String;
            let attempt_index: i64;
            let pending_attempt_record: Option<PendingPoolAttemptRecord>;
            let prepared_request_body = match prepare_pool_request_body_for_account(
                body.as_ref(),
                original_uri,
                &method,
                account.fast_mode_rewrite_mode,
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(message) => {
                    last_error = Some(PoolUpstreamError {
                        account: Some(account.clone()),
                        status: StatusCode::BAD_GATEWAY,
                        message,
                        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                        connect_latency_ms: 0.0,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                        oauth_responses_debug: None,
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: None,
                        request_body_for_capture: None,
                    });
                    exhausted_accounts_all_rate_limited = false;
                    continue 'account_loop;
                }
            };
            let attempted_requested_service_tier =
                prepared_request_body.requested_service_tier.clone();
            let attempted_request_body_for_capture =
                prepared_request_body.request_body_for_capture.clone();
            let (response, oauth_responses_debug, forward_proxy_selection) = match &account.auth {
                PoolResolvedAuth::ApiKey { authorization } => {
                    let (forward_proxy_scope, selected_proxy, client) =
                        match select_pool_account_forward_proxy_client(state.as_ref(), &account)
                            .await
                        {
                            Ok(selection) => selection,
                            Err(message) => {
                                if let Err(route_err) = record_pool_route_transport_failure(
                                    &state.pool,
                                    account.account_id,
                                    sticky_key,
                                    &message,
                                    trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                                )
                                .await
                                {
                                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool forward proxy selection failure");
                                }
                                last_error = Some(PoolUpstreamError {
                                    account: Some(account.clone()),
                                    status: StatusCode::BAD_GATEWAY,
                                    message: message.clone(),
                                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                    connect_latency_ms: 0.0,
                                    upstream_error_code: None,
                                    upstream_error_message: None,
                                    upstream_request_id: None,
                                    oauth_responses_debug: None,
                                    attempt_summary: PoolAttemptSummary::default(),
                                    requested_service_tier: attempted_requested_service_tier
                                        .clone(),
                                    request_body_for_capture: attempted_request_body_for_capture
                                        .clone(),
                                });
                                exhausted_accounts_all_rate_limited = false;
                                continue 'account_loop;
                            }
                        };
                    attempt_count += 1;
                    attempt_index = attempt_count as i64;
                    attempt_started_at = shanghai_now_string();
                    let mut request = client.request(
                        method.clone(),
                        api_key_target_url
                            .clone()
                            .expect("api key pool route should always have an upstream url"),
                    );
                    for (name, value) in headers {
                        if *name == header::AUTHORIZATION {
                            continue;
                        }
                        if should_forward_proxy_header(name, &request_connection_scoped) {
                            request = request.header(name, value);
                        }
                    }
                    request = request.header(header::AUTHORIZATION, authorization.clone());
                    request = request.body(prepared_request_body.snapshot.to_reqwest_body());
                    if let Err(route_err) =
                        record_account_selected(&state.pool, account.account_id).await
                    {
                        warn!(
                            account_id = account.account_id,
                            error = %route_err,
                            "failed to record selected pool account"
                        );
                    }
                    pending_attempt_record = if let Some(trace) = trace_context.as_ref() {
                        Some(
                            begin_pool_upstream_request_attempt(
                                &state.pool,
                                trace,
                                account.account_id,
                                upstream_route_key.as_str(),
                                attempt_index,
                                distinct_account_index,
                                same_account_retry_index,
                                attempt_started_at.as_str(),
                            )
                            .await,
                        )
                    } else {
                        None
                    };
                    let attempt_runtime_snapshot = runtime_snapshot_context.as_ref().map(|ctx| {
                        let mut ctx = ctx.clone();
                        ctx.request_info.requested_service_tier =
                            attempted_requested_service_tier.clone();
                        ctx
                    });
                    if let (Some(trace), Some(runtime_snapshot)) =
                        (trace_context.as_ref(), attempt_runtime_snapshot.as_ref())
                    {
                        broadcast_pool_attempt_started_runtime_snapshot(
                            state.as_ref(),
                            trace,
                            runtime_snapshot,
                            &account,
                            attempt_count,
                            distinct_account_count,
                        )
                        .await;
                    }
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                        && let Err(err) = advance_pool_upstream_request_attempt_phase(
                            state.as_ref(),
                            pending_attempt_record,
                            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
                        )
                        .await
                    {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to advance pool attempt into sending-request phase"
                        );
                    }

                    match timeout(attempt_send_timeout, request.send()).await {
                        Ok(Ok(response)) => (
                            ProxyUpstreamResponseBody::Reqwest(response),
                            None,
                            Some((forward_proxy_scope, selected_proxy)),
                        ),
                        Ok(Err(err)) => {
                            record_pool_account_forward_proxy_result(
                                state.as_ref(),
                                &forward_proxy_scope,
                                &selected_proxy,
                                ForwardProxyRouteResultKind::NetworkFailure,
                            )
                            .await;
                            let message = format!("failed to contact upstream: {err}");
                            let compact_support_observation = classify_compact_support_observation(
                                original_uri,
                                None,
                                Some(message.as_str()),
                            );
                            let timeout_shaped_failure = pool_failure_is_timeout_shaped(
                                PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                &message,
                            );
                            let should_timeout_route_failover =
                                uses_timeout_route_failover && timeout_shaped_failure;
                            let finished_at = shanghai_now_string();
                            if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                                && let Err(record_err) = finalize_pool_upstream_request_attempt(
                                    &state.pool,
                                    pending_attempt_record,
                                    finished_at.as_str(),
                                    POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                                    None,
                                    Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
                                    Some(message.as_str()),
                                    Some(elapsed_ms(connect_started)),
                                    None,
                                    None,
                                    None,
                                    compact_support_observation
                                        .as_ref()
                                        .map(|value| value.status),
                                    compact_support_observation
                                        .as_ref()
                                        .and_then(|value| value.reason.as_deref()),
                                )
                                .await
                            {
                                warn!(
                                    invoke_id = pending_attempt_record.invoke_id,
                                    error = %record_err,
                                    "failed to persist pool transport attempt"
                                );
                            }
                            if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                                && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                                    state.as_ref(),
                                    &pending_attempt_record.invoke_id,
                                )
                                .await
                            {
                                warn!(
                                    invoke_id = pending_attempt_record.invoke_id,
                                    error = %err,
                                    "failed to broadcast pool transport attempt snapshot"
                                );
                            }
                            let has_retry_budget =
                                same_account_attempt + 1 < same_account_attempt_budget;
                            if has_retry_budget && !should_timeout_route_failover {
                                let retry_delay = fallback_proxy_429_retry_delay(
                                    u32::from(same_account_attempt) + 1,
                                );
                                info!(
                                    account_id = account.account_id,
                                    retry_index = same_account_attempt + 1,
                                    max_same_account_attempts = same_account_attempt_budget,
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
                                trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
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
                                oauth_responses_debug: None,
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            });
                            exhausted_accounts_all_rate_limited = false;
                            if should_timeout_route_failover {
                                excluded_upstream_route_keys.insert(upstream_route_key.clone());
                                timeout_route_failover_pending = true;
                            }
                            continue 'account_loop;
                        }
                        Err(_) => {
                            record_pool_account_forward_proxy_result(
                                state.as_ref(),
                                &forward_proxy_scope,
                                &selected_proxy,
                                ForwardProxyRouteResultKind::NetworkFailure,
                            )
                            .await;
                            let message = proxy_request_send_timeout_message(
                                capture_target_for_request(original_uri.path(), &method),
                                attempt_send_timeout,
                            );
                            let compact_support_observation = classify_compact_support_observation(
                                original_uri,
                                None,
                                Some(message.as_str()),
                            );
                            let should_timeout_route_failover = uses_timeout_route_failover;
                            let finished_at = shanghai_now_string();
                            if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                                && let Err(record_err) = finalize_pool_upstream_request_attempt(
                                    &state.pool,
                                    pending_attempt_record,
                                    finished_at.as_str(),
                                    POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                                    None,
                                    Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT),
                                    Some(message.as_str()),
                                    Some(elapsed_ms(connect_started)),
                                    None,
                                    None,
                                    None,
                                    compact_support_observation
                                        .as_ref()
                                        .map(|value| value.status),
                                    compact_support_observation
                                        .as_ref()
                                        .and_then(|value| value.reason.as_deref()),
                                )
                                .await
                            {
                                warn!(
                                    invoke_id = pending_attempt_record.invoke_id,
                                    error = %record_err,
                                    "failed to persist pool handshake timeout attempt"
                                );
                            }
                            if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                                && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                                    state.as_ref(),
                                    &pending_attempt_record.invoke_id,
                                )
                                .await
                            {
                                warn!(
                                    invoke_id = pending_attempt_record.invoke_id,
                                    error = %err,
                                    "failed to broadcast pool handshake timeout snapshot"
                                );
                            }
                            let has_retry_budget =
                                same_account_attempt + 1 < same_account_attempt_budget;
                            if has_retry_budget && !should_timeout_route_failover {
                                let retry_delay = fallback_proxy_429_retry_delay(
                                    u32::from(same_account_attempt) + 1,
                                );
                                info!(
                                    account_id = account.account_id,
                                    retry_index = same_account_attempt + 1,
                                    max_same_account_attempts = same_account_attempt_budget,
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
                                trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
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
                                oauth_responses_debug: None,
                                attempt_summary: PoolAttemptSummary::default(),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture: attempted_request_body_for_capture
                                    .clone(),
                            });
                            exhausted_accounts_all_rate_limited = false;
                            if should_timeout_route_failover {
                                excluded_upstream_route_keys.insert(upstream_route_key.clone());
                                timeout_route_failover_pending = true;
                            }
                            continue 'account_loop;
                        }
                    }
                }
                PoolResolvedAuth::Oauth {
                    access_token,
                    chatgpt_account_id,
                } => {
                    let (forward_proxy_scope, selected_proxy, client) =
                        match select_pool_account_forward_proxy_client(state.as_ref(), &account)
                            .await
                        {
                            Ok(selection) => selection,
                            Err(message) => {
                                if let Err(route_err) = record_pool_route_transport_failure(
                                    &state.pool,
                                    account.account_id,
                                    sticky_key,
                                    &message,
                                    trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                                )
                                .await
                                {
                                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool oauth forward proxy selection failure");
                                }
                                last_error = Some(PoolUpstreamError {
                                    account: Some(account.clone()),
                                    status: StatusCode::BAD_GATEWAY,
                                    message: message.clone(),
                                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                    connect_latency_ms: 0.0,
                                    upstream_error_code: None,
                                    upstream_error_message: None,
                                    upstream_request_id: None,
                                    oauth_responses_debug: None,
                                    attempt_summary: PoolAttemptSummary::default(),
                                    requested_service_tier: attempted_requested_service_tier
                                        .clone(),
                                    request_body_for_capture: attempted_request_body_for_capture
                                        .clone(),
                                });
                                exhausted_accounts_all_rate_limited = false;
                                continue 'account_loop;
                            }
                        };
                    let oauth_body = match &prepared_request_body.snapshot {
                        PoolReplayBodySnapshot::File { size, .. }
                            if original_uri.path() == "/v1/responses"
                                && *size > OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES =>
                        {
                            last_error = Some(PoolUpstreamError {
                                account: Some(account.clone()),
                                status: StatusCode::PAYLOAD_TOO_LARGE,
                                message: format!(
                                    "oauth /v1/responses request body exceeds {} bytes rewrite limit",
                                    OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES
                                ),
                                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                                connect_latency_ms: 0.0,
                                upstream_error_code: None,
                                upstream_error_message: None,
                                upstream_request_id: None,
                                oauth_responses_debug: None,
                                attempt_summary: pool_attempt_summary(
                                    attempt_count,
                                    distinct_account_count,
                                    Some(PROXY_FAILURE_BODY_TOO_LARGE.to_string()),
                                ),
                                requested_service_tier: attempted_requested_service_tier.clone(),
                                request_body_for_capture:
                                    attempted_request_body_for_capture.clone(),
                            });
                            exhausted_accounts_all_rate_limited = false;
                            continue 'account_loop;
                        }
                        snapshot if original_uri.path() == "/v1/responses" => {
                            oauth_bridge::OauthUpstreamRequestBody::Bytes(
                                snapshot.to_bytes().await.map_err(|err| PoolUpstreamError {
                                    account: Some(account.clone()),
                                    status: StatusCode::BAD_GATEWAY,
                                    message: format!("failed to replay oauth request body: {err}"),
                                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                    connect_latency_ms: 0.0,
                                    upstream_error_code: None,
                                    upstream_error_message: None,
                                    upstream_request_id: None,
                                    oauth_responses_debug: None,
                                    attempt_summary: pool_attempt_summary(
                                        attempt_count,
                                        distinct_account_count,
                                        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                                    ),
                                    requested_service_tier:
                                        attempted_requested_service_tier.clone(),
                                    request_body_for_capture:
                                        attempted_request_body_for_capture.clone(),
                                })?,
                            )
                        }
                        snapshot => oauth_bridge::OauthUpstreamRequestBody::Stream {
                            debug_body_prefix: Some(
                                snapshot
                                    .to_prefix_bytes(
                                        oauth_bridge::OAUTH_REQUEST_BODY_PREFIX_FINGERPRINT_MAX_BYTES,
                                    )
                                    .await
                                    .map_err(|err| PoolUpstreamError {
                                        account: Some(account.clone()),
                                        status: StatusCode::BAD_GATEWAY,
                                        message: format!(
                                            "failed to replay oauth request body prefix: {err}"
                                        ),
                                        failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                                        connect_latency_ms: 0.0,
                                        upstream_error_code: None,
                                        upstream_error_message: None,
                                        upstream_request_id: None,
                                        oauth_responses_debug: None,
                                        attempt_summary: pool_attempt_summary(
                                            attempt_count,
                                            distinct_account_count,
                                            Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string()),
                                        ),
                                        requested_service_tier:
                                            attempted_requested_service_tier.clone(),
                                        request_body_for_capture:
                                            attempted_request_body_for_capture.clone(),
                                    })?,
                            ),
                            body: snapshot.to_reqwest_body(),
                        },
                    };
                    attempt_count += 1;
                    attempt_index = attempt_count as i64;
                    attempt_started_at = shanghai_now_string();
                    if let Err(route_err) =
                        record_account_selected(&state.pool, account.account_id).await
                    {
                        warn!(
                            account_id = account.account_id,
                            error = %route_err,
                            "failed to record selected pool account"
                        );
                    }
                    pending_attempt_record = if let Some(trace) = trace_context.as_ref() {
                        Some(
                            begin_pool_upstream_request_attempt(
                                &state.pool,
                                trace,
                                account.account_id,
                                upstream_route_key.as_str(),
                                attempt_index,
                                distinct_account_index,
                                same_account_retry_index,
                                attempt_started_at.as_str(),
                            )
                            .await,
                        )
                    } else {
                        None
                    };
                    let attempt_runtime_snapshot = runtime_snapshot_context.as_ref().map(|ctx| {
                        let mut ctx = ctx.clone();
                        ctx.request_info.requested_service_tier =
                            attempted_requested_service_tier.clone();
                        ctx
                    });
                    if let (Some(trace), Some(runtime_snapshot)) =
                        (trace_context.as_ref(), attempt_runtime_snapshot.as_ref())
                    {
                        broadcast_pool_attempt_started_runtime_snapshot(
                            state.as_ref(),
                            trace,
                            runtime_snapshot,
                            &account,
                            attempt_count,
                            distinct_account_count,
                        )
                        .await;
                    }
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                        && let Err(err) = advance_pool_upstream_request_attempt_phase(
                            state.as_ref(),
                            pending_attempt_record,
                            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
                        )
                        .await
                    {
                        warn!(
                            invoke_id = %pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to advance pool oauth attempt into sending-request phase"
                        );
                    }
                    {
                        let oauth_response = oauth_bridge::send_oauth_upstream_request(
                            &client,
                            method.clone(),
                            original_uri,
                            headers,
                            oauth_body,
                            attempt_send_timeout,
                            attempt_pre_first_byte_timeout,
                            Some(account.account_id),
                            access_token,
                            chatgpt_account_id.as_deref(),
                            state.upstream_accounts.crypto_key.as_ref(),
                        )
                        .await;
                        (
                            ProxyUpstreamResponseBody::Axum(oauth_response.response),
                            oauth_response.request_debug,
                            Some((forward_proxy_scope, selected_proxy)),
                        )
                    }
                }
            };

            let connect_latency_ms = elapsed_ms(connect_started);
            let status = response.status();
            if status == StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
                || matches!(
                    status,
                    StatusCode::UNAUTHORIZED | StatusCode::PAYMENT_REQUIRED | StatusCode::FORBIDDEN
                )
            {
                let has_retry_budget = same_account_attempt + 1 < same_account_attempt_budget;
                let has_group_upstream_429_retry_budget = status == StatusCode::TOO_MANY_REQUESTS
                    && group_upstream_429_retry_count < group_upstream_429_max_retries;
                let upstream_request_id_header = response
                    .headers()
                    .get("x-request-id")
                    .and_then(|value| value.to_str().ok())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());
                let retry_after_header = response.headers().get(header::RETRY_AFTER).cloned();
                let oauth_transport_failure_kind =
                    oauth_bridge::oauth_transport_failure_kind(response.headers());
                if oauth_transport_failure_kind.is_some()
                    && let Some((forward_proxy_scope, selected_proxy)) =
                        forward_proxy_selection.as_ref()
                {
                    record_pool_account_forward_proxy_result(
                        state.as_ref(),
                        forward_proxy_scope,
                        selected_proxy,
                        ForwardProxyRouteResultKind::NetworkFailure,
                    )
                    .await;
                }
                let (upstream_error_code, upstream_error_message, upstream_request_id, message) =
                    match read_pool_upstream_bytes_with_timeout(
                        response,
                        attempt_pre_first_byte_timeout,
                        connect_started,
                        "reading upstream error body",
                    )
                    .await
                    {
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
                let route_error_message = upstream_error_code
                    .as_deref()
                    .map_or_else(|| message.clone(), |code| format!("{code}: {message}"));
                let http_failure_classification =
                    classify_pool_account_http_failure(&account.kind, status, &route_error_message);
                let failure_kind = oauth_transport_failure_kind
                    .unwrap_or(http_failure_classification.failure_kind);
                let compact_support_observation = classify_compact_support_observation(
                    original_uri,
                    Some(status),
                    Some(route_error_message.as_str()),
                );
                let timeout_shaped_failure = status.is_server_error()
                    && pool_failure_is_timeout_shaped(failure_kind, &message);
                let should_timeout_route_failover =
                    uses_timeout_route_failover && timeout_shaped_failure;
                let retry_delay = (has_retry_budget
                    && !should_timeout_route_failover
                    && status.is_server_error()
                    && status != StatusCode::TOO_MANY_REQUESTS)
                    .then(|| {
                        retry_after_header
                            .as_ref()
                            .and_then(parse_retry_after_delay)
                            .unwrap_or_else(|| {
                                fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1)
                            })
                    });
                let finished_at = shanghai_now_string();
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(record_err) = finalize_pool_upstream_request_attempt(
                        &state.pool,
                        pending_attempt_record,
                        finished_at.as_str(),
                        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
                        Some(status),
                        Some(failure_kind),
                        Some(message.as_str()),
                        Some(connect_latency_ms),
                        None,
                        None,
                        upstream_request_id.as_deref(),
                        compact_support_observation
                            .as_ref()
                            .map(|value| value.status),
                        compact_support_observation
                            .as_ref()
                            .and_then(|value| value.reason.as_deref()),
                    )
                    .await
                {
                    warn!(
                        invoke_id = pending_attempt_record.invoke_id,
                        error = %record_err,
                        "failed to persist pool http failure attempt"
                    );
                }
                if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                    && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                        state.as_ref(),
                        &pending_attempt_record.invoke_id,
                    )
                    .await
                {
                    warn!(
                        invoke_id = pending_attempt_record.invoke_id,
                        error = %err,
                        "failed to broadcast pool http failure snapshot"
                    );
                }
                if has_group_upstream_429_retry_budget {
                    let retry_delay = pool_group_upstream_429_retry_delay(state.as_ref());
                    let group_retry_index = group_upstream_429_retry_count + 1;
                    info!(
                        account_id = account.account_id,
                        status = status.as_u16(),
                        retry_index = same_account_attempt + 1,
                        group_retry_index,
                        max_same_account_attempts = same_account_attempt_loop_budget,
                        group_upstream_429_max_retries,
                        retry_after_ms = retry_delay.as_millis(),
                        "pool upstream responded with group retryable 429; retrying same account"
                    );
                    group_upstream_429_retry_count += 1;
                    sleep(retry_delay).await;
                    continue;
                }
                if let Some(retry_delay) = retry_delay {
                    info!(
                        account_id = account.account_id,
                        status = status.as_u16(),
                        retry_index = same_account_attempt + 1,
                        max_same_account_attempts = same_account_attempt_budget,
                        retry_after_ms = retry_delay.as_millis(),
                        "pool upstream responded with retryable status; retrying same account"
                    );
                    sleep(retry_delay).await;
                    continue;
                }
                if let Err(route_err) = record_pool_route_http_failure(
                    &state.pool,
                    account.account_id,
                    &account.kind,
                    sticky_key,
                    status,
                    &route_error_message,
                    trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                )
                .await
                {
                    warn!(account_id = account.account_id, error = %route_err, "failed to record pool upstream http failure");
                }
                if let Some(observation) = compact_support_observation.as_ref()
                    && let Err(observation_err) = record_compact_support_observation(
                        &state.pool,
                        account.account_id,
                        observation.status,
                        observation.reason.as_deref(),
                    )
                    .await
                {
                    warn!(
                        account_id = account.account_id,
                        error = %observation_err,
                        "failed to record compact support observation"
                    );
                }
                last_error = Some(PoolUpstreamError {
                    account: Some(account.clone()),
                    status,
                    message: message.clone(),
                    failure_kind,
                    connect_latency_ms,
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                    oauth_responses_debug: oauth_responses_debug.clone(),
                    attempt_summary: PoolAttemptSummary::default(),
                    requested_service_tier: attempted_requested_service_tier.clone(),
                    request_body_for_capture: attempted_request_body_for_capture.clone(),
                });
                exhausted_accounts_all_rate_limited &= status == StatusCode::TOO_MANY_REQUESTS;
                if should_timeout_route_failover {
                    excluded_upstream_route_keys.insert(upstream_route_key.clone());
                    timeout_route_failover_pending = true;
                }
                continue 'account_loop;
            }

            if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                && let Err(err) = advance_pool_upstream_request_attempt_phase(
                    state.as_ref(),
                    pending_attempt_record,
                    POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
                )
                .await
            {
                warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to advance pool attempt into wait-first-byte phase"
                );
            }
            let first_byte_started = Instant::now();
            let (response, first_chunk) = match read_pool_upstream_first_chunk_with_timeout(
                response,
                attempt_pre_first_byte_timeout,
                connect_started,
            )
            .await
            {
                Ok(value) => value,
                Err(err) => {
                    if let Some((forward_proxy_scope, selected_proxy)) =
                        forward_proxy_selection.as_ref()
                    {
                        record_pool_account_forward_proxy_result(
                            state.as_ref(),
                            forward_proxy_scope,
                            selected_proxy,
                            ForwardProxyRouteResultKind::NetworkFailure,
                        )
                        .await;
                    }
                    let message = format!("upstream stream error before first chunk: {err}");
                    let compact_support_observation = classify_compact_support_observation(
                        original_uri,
                        None,
                        Some(message.as_str()),
                    );
                    let timeout_shaped_failure = pool_failure_is_timeout_shaped(
                        PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
                        &message,
                    );
                    let should_timeout_route_failover =
                        uses_timeout_route_failover && timeout_shaped_failure;
                    let finished_at = shanghai_now_string();
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                        && let Err(record_err) = finalize_pool_upstream_request_attempt(
                            &state.pool,
                            pending_attempt_record,
                            finished_at.as_str(),
                            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                            None,
                            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
                            Some(message.as_str()),
                            Some(connect_latency_ms),
                            None,
                            None,
                            None,
                            compact_support_observation
                                .as_ref()
                                .map(|value| value.status),
                            compact_support_observation
                                .as_ref()
                                .and_then(|value| value.reason.as_deref()),
                        )
                        .await
                    {
                        warn!(
                            invoke_id = pending_attempt_record.invoke_id,
                            error = %record_err,
                            "failed to persist pool first-chunk transport attempt"
                        );
                    }
                    if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                        && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                            state.as_ref(),
                            &pending_attempt_record.invoke_id,
                        )
                        .await
                    {
                        warn!(
                            invoke_id = pending_attempt_record.invoke_id,
                            error = %err,
                            "failed to broadcast pool first-chunk failure snapshot"
                        );
                    }
                    let has_retry_budget = same_account_attempt + 1 < same_account_attempt_budget;
                    if has_retry_budget && !should_timeout_route_failover {
                        let retry_delay =
                            fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1);
                        info!(
                            account_id = account.account_id,
                            retry_index = same_account_attempt + 1,
                            max_same_account_attempts = same_account_attempt_budget,
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
                        trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
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
                        oauth_responses_debug: oauth_responses_debug.clone(),
                        attempt_summary: PoolAttemptSummary::default(),
                        requested_service_tier: attempted_requested_service_tier.clone(),
                        request_body_for_capture: attempted_request_body_for_capture.clone(),
                    });
                    exhausted_accounts_all_rate_limited = false;
                    if should_timeout_route_failover {
                        excluded_upstream_route_keys.insert(upstream_route_key.clone());
                        timeout_route_failover_pending = true;
                    }
                    continue 'account_loop;
                }
            };

            let first_byte_latency_ms = elapsed_ms(first_byte_started);
            let response_is_event_stream = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("text/event-stream"));
            let (response, first_chunk) = if original_uri.path() == "/v1/responses"
                && status == StatusCode::OK
                && response_is_event_stream
            {
                match gate_pool_initial_response_stream(
                    response,
                    first_chunk,
                    attempt_pre_first_byte_timeout,
                    connect_started,
                )
                .await
                {
                    Ok(PoolInitialSseGateOutcome::Forward {
                        response,
                        prefetched_bytes,
                    }) => (response, prefetched_bytes),
                    Ok(PoolInitialSseGateOutcome::RetrySameAccount {
                        message,
                        upstream_error_code,
                        upstream_error_message,
                        upstream_request_id,
                    }) => {
                        let finished_at = shanghai_now_string();
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(record_err) = finalize_pool_upstream_request_attempt(
                                &state.pool,
                                pending_attempt_record,
                                finished_at.as_str(),
                                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
                                Some(status),
                                Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED),
                                Some(message.as_str()),
                                Some(connect_latency_ms),
                                Some(first_byte_latency_ms),
                                None,
                                upstream_request_id.as_deref(),
                                None,
                                None,
                            )
                            .await
                        {
                            warn!(
                                invoke_id = pending_attempt_record.invoke_id,
                                error = %record_err,
                                "failed to persist pool retryable response.failed attempt"
                            );
                        }
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                                state.as_ref(),
                                &pending_attempt_record.invoke_id,
                            )
                            .await
                        {
                            warn!(
                                invoke_id = pending_attempt_record.invoke_id,
                                error = %err,
                                "failed to broadcast retryable response.failed snapshot"
                            );
                        }

                        let has_retry_budget =
                            same_account_attempt + 1 < same_account_attempt_budget;
                        if has_retry_budget {
                            let retry_delay =
                                fallback_proxy_429_retry_delay(u32::from(same_account_attempt) + 1);
                            info!(
                                account_id = account.account_id,
                                retry_index = same_account_attempt + 1,
                                max_same_account_attempts = same_account_attempt_budget,
                                retry_after_ms = retry_delay.as_millis(),
                                "pool upstream reported retryable response.failed before forwarding; retrying same account"
                            );
                            sleep(retry_delay).await;
                            continue;
                        }

                        if let Err(route_err) = record_pool_route_retryable_overload_failure(
                            &state.pool,
                            account.account_id,
                            sticky_key,
                            &message,
                            trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                        )
                        .await
                        {
                            warn!(account_id = account.account_id, error = %route_err, "failed to record retryable response.failed route state");
                        }
                        last_error = Some(PoolUpstreamError {
                            account: Some(account.clone()),
                            status,
                            message: message.clone(),
                            failure_kind: PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
                            connect_latency_ms,
                            upstream_error_code,
                            upstream_error_message,
                            upstream_request_id,
                            oauth_responses_debug: oauth_responses_debug.clone(),
                            attempt_summary: PoolAttemptSummary::default(),
                            requested_service_tier: attempted_requested_service_tier.clone(),
                            request_body_for_capture: attempted_request_body_for_capture.clone(),
                        });
                        exhausted_accounts_all_rate_limited = false;
                        continue 'account_loop;
                    }
                    Err(err) => {
                        let message = format!("failed to gate first response event: {err}");
                        let finished_at = shanghai_now_string();
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(record_err) = finalize_pool_upstream_request_attempt(
                                &state.pool,
                                pending_attempt_record,
                                finished_at.as_str(),
                                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
                                None,
                                Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
                                Some(message.as_str()),
                                Some(connect_latency_ms),
                                Some(first_byte_latency_ms),
                                None,
                                None,
                                None,
                                None,
                            )
                            .await
                        {
                            warn!(
                                invoke_id = pending_attempt_record.invoke_id,
                                error = %record_err,
                                "failed to persist first-event gate failure attempt"
                            );
                        }
                        if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                            && let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                                state.as_ref(),
                                &pending_attempt_record.invoke_id,
                            )
                            .await
                        {
                            warn!(
                                invoke_id = pending_attempt_record.invoke_id,
                                error = %err,
                                "failed to broadcast first-event gate failure snapshot"
                            );
                        }
                        if let Err(route_err) = record_pool_route_transport_failure(
                            &state.pool,
                            account.account_id,
                            sticky_key,
                            &message,
                            trace_context.as_ref().map(|trace| trace.invoke_id.as_str()),
                        )
                        .await
                        {
                            warn!(account_id = account.account_id, error = %route_err, "failed to record first-event gate transport failure");
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
                            oauth_responses_debug: oauth_responses_debug.clone(),
                            attempt_summary: PoolAttemptSummary::default(),
                            requested_service_tier: attempted_requested_service_tier.clone(),
                            request_body_for_capture: attempted_request_body_for_capture.clone(),
                        });
                        exhausted_accounts_all_rate_limited = false;
                        continue 'account_loop;
                    }
                }
            } else {
                (response, first_chunk)
            };

            if let Some(pending_attempt_record) = pending_attempt_record.as_ref()
                && let Err(err) = advance_pool_upstream_request_attempt_phase(
                    state.as_ref(),
                    pending_attempt_record,
                    POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
                )
                .await
            {
                warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to advance pool attempt into streaming phase"
                );
            }

            let compact_support_observation =
                classify_compact_support_observation(original_uri, Some(status), None);
            if let Some(observation) = compact_support_observation.as_ref()
                && let Err(observation_err) = record_compact_support_observation(
                    &state.pool,
                    account.account_id,
                    observation.status,
                    observation.reason.as_deref(),
                )
                .await
            {
                warn!(
                    account_id = account.account_id,
                    error = %observation_err,
                    "failed to record compact support observation"
                );
            }

            if let Some((forward_proxy_scope, selected_proxy)) = forward_proxy_selection.as_ref() {
                record_pool_account_forward_proxy_result(
                    state.as_ref(),
                    forward_proxy_scope,
                    selected_proxy,
                    ForwardProxyRouteResultKind::CompletedRequest,
                )
                .await;
            }
            reservation_guard.disarm();
            return Ok(PoolUpstreamResponse {
                account: account.clone(),
                response,
                oauth_responses_debug,
                connect_latency_ms,
                attempt_started_at_utc,
                first_byte_latency_ms,
                first_chunk,
                pending_attempt_record: pending_attempt_record.map(|mut pending| {
                    pending.connect_latency_ms = connect_latency_ms;
                    pending.first_byte_latency_ms = first_byte_latency_ms;
                    pending.compact_support_status = compact_support_observation
                        .as_ref()
                        .map(|value| value.status.to_string());
                    pending.compact_support_reason = compact_support_observation
                        .as_ref()
                        .and_then(|value| value.reason.clone());
                    pending
                }),
                attempt_summary: pool_attempt_summary(attempt_count, distinct_account_count, None),
                requested_service_tier: attempted_requested_service_tier,
                request_body_for_capture: attempted_request_body_for_capture,
            });
        }
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
    proxy_request_id: u64,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    handshake_timeout: Duration,
    initial_account: PoolResolvedAccount,
    sticky_key: Option<String>,
    responses_total_timeout_started_at: Option<Instant>,
    replay_status_rx: &watch::Receiver<PoolReplayBodyStatus>,
    replay_cancel: &CancellationToken,
    first_error: PoolUpstreamError,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    let reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let replay_status = { replay_status_rx.borrow().clone() };
    match replay_status {
        PoolReplayBodyStatus::Complete(snapshot) => {
            let replay_sticky_key = extract_sticky_key_from_replay_snapshot(&snapshot)
                .await
                .or(sticky_key);
            let uses_timeout_route_failover =
                pool_uses_responses_timeout_failover_policy(original_uri, &method);
            let first_error_is_timeout_shaped = uses_timeout_route_failover
                && pool_failure_is_timeout_shaped(first_error.failure_kind, &first_error.message);
            let (preferred_account, failover_progress, same_account_attempts) =
                if first_error_is_timeout_shaped {
                    let mut excluded_upstream_route_keys = HashSet::new();
                    excluded_upstream_route_keys.insert(initial_account.upstream_route_key());
                    (
                        None,
                        PoolFailoverProgress {
                            excluded_account_ids: vec![initial_account.account_id],
                            excluded_upstream_route_keys,
                            attempt_count: 1,
                            last_error: Some(first_error),
                            timeout_route_failover_pending: true,
                            responses_total_timeout_started_at,
                            no_available_wait_deadline: None,
                        },
                        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                    )
                } else if pool_upstream_error_is_rate_limited(&first_error) {
                    (
                        None,
                        PoolFailoverProgress {
                            excluded_account_ids: vec![initial_account.account_id],
                            attempt_count: 1,
                            last_error: Some(first_error),
                            responses_total_timeout_started_at,
                            ..PoolFailoverProgress::default()
                        },
                        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                    )
                } else {
                    (
                        Some(initial_account.clone()),
                        PoolFailoverProgress {
                            attempt_count: 1,
                            responses_total_timeout_started_at,
                            ..PoolFailoverProgress::default()
                        },
                        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS.saturating_sub(1),
                    )
                };
            send_pool_request_with_failover(
                state,
                proxy_request_id,
                method,
                original_uri,
                headers,
                Some(snapshot),
                handshake_timeout,
                None,
                None,
                replay_sticky_key.as_deref(),
                preferred_account,
                failover_progress,
                same_account_attempts,
            )
            .await
        }
        PoolReplayBodyStatus::ReadError(err) => {
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            Err(PoolUpstreamError {
                account: Some(initial_account),
                status: err.status,
                message: err.message,
                failure_kind: err.failure_kind,
                connect_latency_ms: first_error.connect_latency_ms,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
                oauth_responses_debug: None,
                attempt_summary: first_error.attempt_summary.clone(),
                requested_service_tier: first_error.requested_service_tier.clone(),
                request_body_for_capture: first_error.request_body_for_capture.clone(),
            })
        }
        PoolReplayBodyStatus::InternalError(message) => {
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            Err(PoolUpstreamError {
                account: Some(initial_account),
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message,
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                connect_latency_ms: first_error.connect_latency_ms,
                upstream_error_code: None,
                upstream_error_message: None,
                upstream_request_id: None,
                oauth_responses_debug: None,
                attempt_summary: first_error.attempt_summary.clone(),
                requested_service_tier: first_error.requested_service_tier.clone(),
                request_body_for_capture: first_error.request_body_for_capture.clone(),
            })
        }
        PoolReplayBodyStatus::Reading | PoolReplayBodyStatus::Incomplete => {
            replay_cancel.cancel();
            release_pool_routing_reservation(state.as_ref(), &reservation_key);
            Err(first_error)
        }
    }
}

async fn maybe_backfill_oauth_request_debug_from_replay_status(
    debug: &mut Option<oauth_bridge::OauthResponsesDebugInfo>,
    original_uri: &Uri,
    replay_status_rx: &watch::Receiver<PoolReplayBodyStatus>,
    crypto_key: Option<&[u8; 32]>,
) {
    let Some(debug) = debug.as_mut() else {
        return;
    };
    if debug.request_body_prefix_fingerprint.is_some() || crypto_key.is_none() {
        return;
    }

    let replay_status = { replay_status_rx.borrow().clone() };
    let PoolReplayBodyStatus::Complete(snapshot) = replay_status else {
        return;
    };
    let Ok(prefix) = snapshot
        .to_prefix_bytes(oauth_bridge::OAUTH_REQUEST_BODY_PREFIX_FINGERPRINT_MAX_BYTES)
        .await
    else {
        return;
    };
    oauth_bridge::backfill_oauth_request_debug_body_prefix(
        debug,
        original_uri.path(),
        prefix.as_ref(),
        crypto_key,
    );
}

async fn proxy_openai_v1_via_pool(
    state: Arc<AppState>,
    proxy_request_id: u64,
    original_uri: &Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
) -> Result<Response, (StatusCode, String)> {
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let pool_routing_reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let capture_target = capture_target_for_request(original_uri.path(), &method);
    let handshake_timeout =
        proxy_upstream_send_timeout_for_capture_target(&runtime_timeouts, capture_target);
    let _pre_first_byte_timeout =
        pool_upstream_first_chunk_timeout(&runtime_timeouts, original_uri, &method);
    let _responses_total_timeout =
        pool_upstream_responses_total_timeout(&state.config, original_uri, &method);
    let responses_total_timeout_started_at = None;
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
                runtime_timeouts.request_read_timeout,
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
                    proxy_request_id,
                    method,
                    original_uri,
                    &headers,
                    Some(PoolReplayBodySnapshot::Memory(request_body_bytes)),
                    handshake_timeout,
                    None,
                    None,
                    body_sticky_key.as_deref(),
                    None,
                    PoolFailoverProgress {
                        responses_total_timeout_started_at,
                        ..PoolFailoverProgress::default()
                    },
                    POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                )
                .await
                .map_err(|err| (err.status, err.message))?,
                body_sticky_key,
            )
        } else {
            let unwrap_initial_pool_account = |resolution,
                                               no_available_wait_deadline|
             -> Result<
                (PoolResolvedAccount, Option<Instant>),
                (StatusCode, String),
            > {
                let initial_account = match resolution {
                    Ok(PoolAccountResolution::Resolved(account)) => account,
                    Ok(PoolAccountResolution::RateLimited) => {
                        return Err((
                            StatusCode::TOO_MANY_REQUESTS,
                            POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                        ));
                    }
                    Ok(PoolAccountResolution::DegradedOnly) => {
                        return Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                        ));
                    }
                    Ok(PoolAccountResolution::Unavailable)
                    | Ok(PoolAccountResolution::NoCandidate) => {
                        return Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
                        ));
                    }
                    Ok(PoolAccountResolution::BlockedByPolicy(message)) => {
                        return Err((StatusCode::SERVICE_UNAVAILABLE, message));
                    }
                    Err(err) => {
                        return Err((
                            StatusCode::BAD_GATEWAY,
                            format!("failed to resolve pool account: {err}"),
                        ));
                    }
                };
                Ok((initial_account, no_available_wait_deadline))
            };
            let (
                request_body_snapshot,
                body_sticky_key,
                initial_account,
                no_available_wait_deadline,
            ) = if let Some(sticky_key) = header_sticky_key.clone() {
                let state_for_wait = state.clone();
                let sticky_key_for_join_error = sticky_key.clone();
                let wait_task_sticky_key = sticky_key.clone();
                let shared_wait_deadline = Arc::new(std::sync::Mutex::new(None));
                let shared_wait_deadline_for_task = shared_wait_deadline.clone();
                let mut header_sticky_resolution = tokio::spawn(async move {
                    let excluded_ids = Vec::new();
                    let excluded_upstream_route_keys = HashSet::new();
                    let mut no_available_wait_deadline = None;
                    let poll_interval = state_for_wait
                        .pool_no_available_wait
                        .normalized_poll_interval();
                    loop {
                        let resolution = resolve_pool_account_for_request(
                            state_for_wait.as_ref(),
                            Some(wait_task_sticky_key.as_str()),
                            &excluded_ids,
                            &excluded_upstream_route_keys,
                        )
                        .await;
                        match resolution {
                            Ok(PoolAccountResolution::Unavailable)
                            | Ok(PoolAccountResolution::NoCandidate) => {
                                let wait_deadline =
                                    if let Some(deadline) = no_available_wait_deadline {
                                        deadline
                                    } else {
                                        let deadline = Instant::now()
                                            + state_for_wait.pool_no_available_wait.timeout;
                                        no_available_wait_deadline = Some(deadline);
                                        *shared_wait_deadline_for_task
                                            .lock()
                                            .expect("lock shared header wait deadline") =
                                            Some(deadline);
                                        deadline
                                    };
                                let now = Instant::now();
                                if now >= wait_deadline {
                                    break (resolution, no_available_wait_deadline);
                                }
                                tokio::time::sleep(
                                    poll_interval.min(wait_deadline.saturating_duration_since(now)),
                                )
                                .await;
                            }
                            _ => break (resolution, no_available_wait_deadline),
                        }
                    }
                });
                let mut request_body_snapshot_task = tokio::spawn(async move {
                    read_request_body_snapshot_with_limit(
                        body,
                        body_limit,
                        runtime_timeouts.request_read_timeout,
                        proxy_request_id,
                    )
                    .await
                });
                let mut header_sticky_resolution_finished = false;
                let request_body_snapshot = loop {
                    tokio::select! {
                        body_result = &mut request_body_snapshot_task => {
                            match body_result {
                                Ok(Ok(snapshot)) => break snapshot,
                                Ok(Err(err)) => {
                                    header_sticky_resolution.abort();
                                    return Err((err.status, err.message));
                                }
                                Err(err) => {
                                    header_sticky_resolution.abort();
                                    return Err((
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        format!("failed to join request body snapshot task: {err}"),
                                    ));
                                }
                            }
                        }
                        resolution_result = &mut header_sticky_resolution, if !header_sticky_resolution_finished => {
                            header_sticky_resolution_finished = true;
                            let (resolution, _no_available_wait_deadline) = resolution_result.map_err(|err| {
                                (
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    format!(
                                        "failed to join pool wait task for sticky {sticky_key_for_join_error}: {err}"
                                    ),
                                )
                            })?;
                            match resolution {
                                Ok(PoolAccountResolution::Resolved(_account)) => {}
                                Ok(PoolAccountResolution::RateLimited) => {
                                    request_body_snapshot_task.abort();
                                    return Err((
                                        StatusCode::TOO_MANY_REQUESTS,
                                        POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
                                    ));
                                }
                                Ok(PoolAccountResolution::DegradedOnly) => {
                                    request_body_snapshot_task.abort();
                                    return Err((
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
                                    ));
                                }
                                Ok(PoolAccountResolution::Unavailable)
                                | Ok(PoolAccountResolution::NoCandidate) => {
                                    request_body_snapshot_task.abort();
                                    return Err((
                                        StatusCode::SERVICE_UNAVAILABLE,
                                        POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
                                    ));
                                }
                                Ok(PoolAccountResolution::BlockedByPolicy(message)) => {
                                    request_body_snapshot_task.abort();
                                    return Err((StatusCode::SERVICE_UNAVAILABLE, message));
                                }
                                Err(err) => {
                                    request_body_snapshot_task.abort();
                                    return Err((
                                        StatusCode::BAD_GATEWAY,
                                        format!("failed to resolve pool account: {err}"),
                                    ));
                                }
                            }
                        }
                    }
                };
                if !header_sticky_resolution_finished {
                    header_sticky_resolution.abort();
                }
                let body_sticky_key =
                    extract_sticky_key_from_replay_snapshot(&request_body_snapshot)
                        .await
                        .or(Some(sticky_key));
                let mut no_available_wait_deadline = *shared_wait_deadline
                    .lock()
                    .expect("lock shared header wait deadline");
                let resolution = resolve_pool_account_for_request_with_wait(
                    state.as_ref(),
                    body_sticky_key.as_deref(),
                    &[],
                    &HashSet::new(),
                    true,
                    &mut no_available_wait_deadline,
                    None,
                )
                .await;
                let (initial_account, no_available_wait_deadline) =
                    unwrap_initial_pool_account(resolution, no_available_wait_deadline)?;
                (
                    request_body_snapshot,
                    body_sticky_key,
                    initial_account,
                    no_available_wait_deadline,
                )
            } else {
                let request_body_snapshot = read_request_body_snapshot_with_limit(
                    body,
                    body_limit,
                    runtime_timeouts.request_read_timeout,
                    proxy_request_id,
                )
                .await
                .map_err(|err| (err.status, err.message))?;
                let body_sticky_key =
                    extract_sticky_key_from_replay_snapshot(&request_body_snapshot).await;
                let mut no_available_wait_deadline = None;
                let resolution = resolve_pool_account_for_request_with_wait(
                    state.as_ref(),
                    body_sticky_key.as_deref(),
                    &[],
                    &HashSet::new(),
                    true,
                    &mut no_available_wait_deadline,
                    None,
                )
                .await;
                let (initial_account, no_available_wait_deadline) =
                    unwrap_initial_pool_account(resolution, no_available_wait_deadline)?;
                (
                    request_body_snapshot,
                    body_sticky_key,
                    initial_account,
                    no_available_wait_deadline,
                )
            };
            (
                send_pool_request_with_failover(
                    state.clone(),
                    proxy_request_id,
                    method,
                    original_uri,
                    &headers,
                    Some(request_body_snapshot),
                    handshake_timeout,
                    None,
                    None,
                    body_sticky_key.as_deref(),
                    Some(initial_account),
                    PoolFailoverProgress {
                        responses_total_timeout_started_at,
                        no_available_wait_deadline,
                        ..PoolFailoverProgress::default()
                    },
                    POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
                )
                .await
                .map_err(|err| (err.status, err.message))?,
                body_sticky_key,
            )
        }
    } else {
        (
            send_pool_request_with_failover(
                state.clone(),
                proxy_request_id,
                method,
                original_uri,
                &headers,
                None,
                handshake_timeout,
                None,
                None,
                header_sticky_key.as_deref(),
                None,
                PoolFailoverProgress {
                    responses_total_timeout_started_at,
                    ..PoolFailoverProgress::default()
                },
                POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS,
            )
            .await
            .map_err(|err| (err.status, err.message))?,
            header_sticky_key,
        )
    };

    let account = upstream.account;
    let upstream_attempt_started_at_utc = upstream.attempt_started_at_utc;
    let upstream_invoke_id = upstream
        .pending_attempt_record
        .as_ref()
        .map(|record| record.invoke_id.clone());
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

    let mut upstream_stream = upstream_response.into_bytes_stream();
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
        consume_pool_routing_reservation(state.as_ref(), &pool_routing_reservation_key);
        if let Err(route_err) = record_pool_route_success(
            &state.pool,
            account.account_id,
            upstream_attempt_started_at_utc,
            sticky_key.as_deref(),
            upstream_invoke_id.as_deref(),
        )
        .await
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
    let reservation_key_for_record = pool_routing_reservation_key.clone();
    let sticky_key_for_record = sticky_key.clone();
    let invoke_id_for_record = upstream_invoke_id.clone();
    let upstream_attempt_started_at_utc_for_record = upstream_attempt_started_at_utc;
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
            release_pool_routing_reservation(
                state_for_record.as_ref(),
                &reservation_key_for_record,
            );
            if let Err(route_err) = record_pool_route_transport_failure(
                &state_for_record.pool,
                account.account_id,
                sticky_key_for_record.as_deref(),
                message,
                invoke_id_for_record.as_deref(),
            )
            .await
            {
                warn!(account_id = account.account_id, error = %route_err, "failed to record pool stream error");
            }
        } else {
            consume_pool_routing_reservation(
                state_for_record.as_ref(),
                &reservation_key_for_record,
            );
            if let Err(route_err) = record_pool_route_success(
                &state_for_record.pool,
                account.account_id,
                upstream_attempt_started_at_utc_for_record,
                sticky_key_for_record.as_deref(),
                invoke_id_for_record.as_deref(),
            )
            .await
            {
                warn!(account_id = account.account_id, error = %route_err, "failed to record pool route success");
            }
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
    capture_target: Option<ProxyCaptureTarget>,
    upstream_429_max_retries: u8,
) -> Result<ForwardProxyUpstreamResponse, ForwardProxyUpstreamError> {
    let request_connection_scoped = connection_scoped_header_names(headers);

    for attempt in 0..=upstream_429_max_retries {
        let selected_proxy = match select_forward_proxy_for_request(state.as_ref()).await {
            Ok(selected_proxy) => selected_proxy,
            Err(err) => {
                return Err(ForwardProxyUpstreamError {
                    selected_proxy: SelectedForwardProxy::from_endpoint(
                        &ForwardProxyEndpoint::direct(),
                    ),
                    status: StatusCode::BAD_GATEWAY,
                    message: format!("failed to select forward proxy node: {err}"),
                    failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                    attempt_failure_kind: FORWARD_PROXY_FAILURE_SEND_ERROR,
                    connect_latency_ms: 0.0,
                });
            }
        };
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
                    message: proxy_request_send_timeout_message(capture_target, handshake_timeout),
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
                response: ProxyUpstreamResponseBody::Reqwest(response),
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
            response: ProxyUpstreamResponseBody::Reqwest(response),
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
    invoke_id: String,
    original_uri: Uri,
    method: Method,
    headers: HeaderMap,
    body: Body,
    peer_ip: Option<IpAddr>,
) -> Result<Response, ProxyErrorResponse> {
    let pool_route_active = request_matches_pool_route(state.as_ref(), &headers)
        .await
        .map_err(|err| ProxyErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to resolve pool routing settings: {err}"),
            cvm_id: None,
            retry_after_secs: None,
        })?;
    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), pool_route_active)
        .await
        .map_err(|err| ProxyErrorResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("failed to resolve pool routing timeouts: {err}"),
            cvm_id: None,
            retry_after_secs: None,
        })?;
    if !pool_route_active {
        return Err(ProxyErrorResponse {
            status: StatusCode::UNAUTHORIZED,
            message: "pool route key missing or invalid".to_string(),
            cvm_id: None,
            retry_after_secs: None,
        });
    }
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
                ProxyErrorResponse {
                    status,
                    message: format!("failed to build upstream url: {err}"),
                    cvm_id: None,
                    retry_after_secs: None,
                }
            },
        )?;

    if method == Method::GET && is_models_list_path(original_uri.path()) {
        return proxy_openai_v1_via_pool(
            state,
            proxy_request_id,
            &original_uri,
            method,
            headers,
            body,
            runtime_timeouts,
        )
        .await
        .map_err(|(status, message)| ProxyErrorResponse {
            retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
            status,
            message,
            cvm_id: None,
        });
    }

    if let Some(target) = capture_target_for_request(original_uri.path(), &method) {
        let tracked_invoke_id = invoke_id.clone();
        return proxy_openai_v1_capture_target(
            state,
            proxy_request_id,
            invoke_id,
            &original_uri,
            headers,
            body,
            target,
            target_url,
            peer_ip,
            pool_route_active,
            runtime_timeouts,
        )
        .await
        .map_err(|(status, message)| ProxyErrorResponse {
            retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
            status,
            message,
            cvm_id: Some(tracked_invoke_id),
        });
    }

    return proxy_openai_v1_via_pool(
        state,
        proxy_request_id,
        &original_uri,
        method,
        headers,
        body,
        runtime_timeouts,
    )
    .await
    .map_err(|(status, message)| ProxyErrorResponse {
        retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
        status,
        message,
        cvm_id: None,
    });
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
    invoke_id: String,
    original_uri: &Uri,
    headers: HeaderMap,
    body: Body,
    capture_target: ProxyCaptureTarget,
    target_url: Url,
    peer_ip: Option<IpAddr>,
    pool_route_active: bool,
    runtime_timeouts: PoolRoutingTimeoutSettingsResolved,
) -> Result<Response, (StatusCode, String)> {
    let capture_started = Instant::now();
    let pool_routing_reservation_key = build_pool_routing_reservation_key(proxy_request_id);
    let occurred_at_utc = Utc::now();
    let occurred_at = format_naive(occurred_at_utc.with_timezone(&Shanghai).naive_local());
    let body_limit = state.config.openai_proxy_max_request_body_bytes;
    let requester_ip = extract_requester_ip(&headers, peer_ip);
    let header_sticky_key = extract_sticky_key_from_headers(&headers);
    let header_prompt_cache_key = extract_prompt_cache_key_from_headers(&headers);

    let req_read_started = Instant::now();
    let request_body_bytes = match read_request_body_with_limit(
        body,
        body_limit,
        runtime_timeouts.request_read_timeout,
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
                payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                    target: capture_target,
                    status: read_err.status,
                    is_stream: request_info.is_stream,
                    request_model: None,
                    requested_service_tier: request_info.requested_service_tier.as_deref(),
                    reasoning_effort: request_info.reasoning_effort.as_deref(),
                    response_model: None,
                    usage_missing_reason: None,
                    request_parse_error: request_info.parse_error.as_deref(),
                    failure_kind: Some(read_err.failure_kind),
                    requester_ip: requester_ip.as_deref(),
                    upstream_scope: if pool_route_active {
                        INVOCATION_UPSTREAM_SCOPE_INTERNAL
                    } else {
                        INVOCATION_UPSTREAM_SCOPE_EXTERNAL
                    },
                    route_mode: if pool_route_active {
                        INVOCATION_ROUTE_MODE_POOL
                    } else {
                        INVOCATION_ROUTE_MODE_FORWARD_PROXY
                    },
                    sticky_key: header_sticky_key.as_deref(),
                    prompt_cache_key: header_prompt_cache_key.as_deref(),
                    upstream_account_id: None,
                    upstream_account_name: None,
                    oauth_account_header_attached: None,
                    oauth_account_id_shape: None,
                    oauth_forwarded_header_count: None,
                    oauth_forwarded_header_names: None,
                    oauth_fingerprint_version: None,
                    oauth_forwarded_header_fingerprints: None,
                    oauth_prompt_cache_header_forwarded: None,
                    oauth_request_body_prefix_fingerprint: None,
                    oauth_request_body_prefix_bytes: None,
                    oauth_responses_rewrite: None,
                    service_tier: None,
                    stream_terminal_event: None,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    upstream_request_id: None,
                    response_content_encoding: None,
                    proxy_display_name: None,
                    proxy_weight_delta: None,
                    pool_attempt_count: None,
                    pool_distinct_account_count: None,
                    pool_attempt_terminal_reason: None,
                })),
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
    let (upstream_body, mut request_info, body_rewritten) = prepare_target_request_body(
        capture_target,
        request_body_bytes,
        state.config.proxy_enforce_stream_include_usage,
    );
    let prompt_cache_key = request_info
        .prompt_cache_key
        .clone()
        .or_else(|| header_prompt_cache_key.clone());
    let sticky_key = request_info
        .sticky_key
        .clone()
        .or_else(|| header_sticky_key.clone());
    let pool_attempt_trace_context = pool_route_active.then(|| PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.clone(),
        occurred_at: occurred_at.clone(),
        endpoint: capture_target.endpoint().to_string(),
        sticky_key: sticky_key.clone(),
        requester_ip: requester_ip.clone(),
    });
    let t_req_parse_ms = elapsed_ms(req_parse_started);
    let upstream_body_bytes = Bytes::from(upstream_body);
    let base_request_bytes_for_capture = upstream_body_bytes.clone();

    let initial_running_record = build_running_proxy_capture_record(
        &invoke_id,
        &occurred_at,
        capture_target,
        &request_info,
        requester_ip.as_deref(),
        sticky_key.as_deref(),
        prompt_cache_key.as_deref(),
        pool_route_active,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        t_req_read_ms,
        t_req_parse_ms,
        0.0,
        0.0,
    );
    if let Err(err) =
        broadcast_proxy_capture_runtime_snapshot(&state.broadcaster, &initial_running_record)
    {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast initial running proxy capture snapshot"
        );
    }

    let mut upstream_headers = headers.clone();
    if body_rewritten {
        upstream_headers.remove(header::CONTENT_LENGTH);
    }
    let pool_attempt_runtime_snapshot =
        pool_route_active.then(|| PoolAttemptRuntimeSnapshotContext {
            capture_target,
            request_info: request_info.clone(),
            prompt_cache_key: prompt_cache_key.clone(),
            t_req_read_ms,
            t_req_parse_ms,
        });
    let handshake_timeout =
        proxy_upstream_send_timeout_for_capture_target(&runtime_timeouts, Some(capture_target));
    let first_byte_timeout =
        pool_upstream_first_chunk_timeout(&runtime_timeouts, &original_uri, &Method::POST);
    let stream_timeout = proxy_capture_target_stream_timeout(&runtime_timeouts, capture_target);
    let (
        selected_proxy,
        pool_account,
        t_upstream_connect_ms,
        prefetched_first_chunk,
        prefetched_ttfb_ms,
        oauth_responses_debug,
        attempt_already_recorded,
        final_attempt_update,
        pending_pool_attempt_record,
        pending_pool_attempt_summary,
        upstream_attempt_started_at,
        upstream_attempt_started_at_utc,
        final_request_body_for_capture,
        final_requested_service_tier,
        upstream_response,
    ) = if pool_route_active {
        match send_pool_request_with_failover(
            state.clone(),
            proxy_request_id,
            Method::POST,
            &original_uri,
            &upstream_headers,
            Some(PoolReplayBodySnapshot::Memory(upstream_body_bytes.clone())),
            handshake_timeout,
            pool_attempt_trace_context.clone(),
            pool_attempt_runtime_snapshot.clone(),
            sticky_key.as_deref(),
            None,
            PoolFailoverProgress::default(),
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
                response.oauth_responses_debug,
                true,
                None,
                response.pending_attempt_record,
                response.attempt_summary,
                None,
                Some(response.attempt_started_at_utc),
                response.request_body_for_capture,
                response.requested_service_tier,
                response.response,
            ),
            Err(err) => {
                request_info.requested_service_tier = err
                    .requested_service_tier
                    .clone()
                    .or(request_info.requested_service_tier);
                let request_body_for_capture = err
                    .request_body_for_capture
                    .clone()
                    .unwrap_or_else(|| base_request_bytes_for_capture.clone());
                let req_raw = store_raw_payload_file(
                    &state.config,
                    &invoke_id,
                    "request",
                    request_body_for_capture.as_ref(),
                );
                let usage = ParsedUsage::default();
                let (cost, cost_estimated, price_version) =
                    estimate_proxy_cost_from_shared_catalog(
                        &state.pricing_catalog,
                        request_info.model.as_deref(),
                        &usage,
                    )
                    .await;
                let error_message = format!("[{}] {}", err.failure_kind, err.message);
                let pool_proxy_display_name = resolve_invocation_proxy_display_name(None);
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
                    payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                        target: capture_target,
                        status: err.status,
                        is_stream: request_info.is_stream,
                        request_model: None,
                        requested_service_tier: request_info.requested_service_tier.as_deref(),
                        reasoning_effort: request_info.reasoning_effort.as_deref(),
                        response_model: None,
                        usage_missing_reason: None,
                        request_parse_error: request_info.parse_error.as_deref(),
                        failure_kind: Some(err.failure_kind),
                        requester_ip: requester_ip.as_deref(),
                        upstream_scope: INVOCATION_UPSTREAM_SCOPE_INTERNAL,
                        route_mode: INVOCATION_ROUTE_MODE_POOL,
                        sticky_key: sticky_key.as_deref(),
                        prompt_cache_key: prompt_cache_key.as_deref(),
                        upstream_account_id: err.account.as_ref().map(|account| account.account_id),
                        upstream_account_name: err
                            .account
                            .as_ref()
                            .map(|account| account.display_name.as_str()),
                        oauth_account_header_attached: oauth_account_header_attached_for_account(
                            err.account.as_ref(),
                        ),
                        oauth_account_id_shape: oauth_account_id_shape_for_account(
                            err.account.as_ref(),
                        ),
                        oauth_forwarded_header_count: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| debug.forwarded_header_names.len()),
                        oauth_forwarded_header_names: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| debug.forwarded_header_names.as_slice()),
                        oauth_fingerprint_version: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.fingerprint_version),
                        oauth_forwarded_header_fingerprints: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.forwarded_header_fingerprints.as_ref()),
                        oauth_prompt_cache_header_forwarded: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| debug.prompt_cache_header_forwarded),
                        oauth_request_body_prefix_fingerprint: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.request_body_prefix_fingerprint.as_deref()),
                        oauth_request_body_prefix_bytes: err
                            .oauth_responses_debug
                            .as_ref()
                            .and_then(|debug| debug.request_body_prefix_bytes),
                        oauth_responses_rewrite: err
                            .oauth_responses_debug
                            .as_ref()
                            .map(|debug| &debug.rewrite),
                        service_tier: None,
                        stream_terminal_event: None,
                        upstream_error_code: err.upstream_error_code.as_deref(),
                        upstream_error_message: err.upstream_error_message.as_deref(),
                        upstream_request_id: err.upstream_request_id.as_deref(),
                        response_content_encoding: None,
                        proxy_display_name: pool_proxy_display_name.as_deref(),
                        proxy_weight_delta: None,
                        pool_attempt_count: Some(err.attempt_summary.pool_attempt_count),
                        pool_distinct_account_count: Some(
                            err.attempt_summary.pool_distinct_account_count,
                        ),
                        pool_attempt_terminal_reason: err
                            .attempt_summary
                            .pool_attempt_terminal_reason
                            .as_deref(),
                    })),
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
            Some(upstream_body_bytes.clone()),
            handshake_timeout,
            Some(capture_target),
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
                None,
                response.attempt_recorded,
                response.attempt_update,
                None,
                PoolAttemptSummary::default(),
                Some(response.attempt_started_at),
                None,
                Some(base_request_bytes_for_capture.clone()),
                request_info.requested_service_tier.clone(),
                response.response,
            ),
            Err(err) => {
                let req_raw = store_raw_payload_file(
                    &state.config,
                    &invoke_id,
                    "request",
                    base_request_bytes_for_capture.as_ref(),
                );
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
                    payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                        target: capture_target,
                        status: err.status,
                        is_stream: request_info.is_stream,
                        request_model: None,
                        requested_service_tier: request_info.requested_service_tier.as_deref(),
                        reasoning_effort: request_info.reasoning_effort.as_deref(),
                        response_model: None,
                        usage_missing_reason: None,
                        request_parse_error: request_info.parse_error.as_deref(),
                        failure_kind: Some(err.failure_kind),
                        requester_ip: requester_ip.as_deref(),
                        upstream_scope: INVOCATION_UPSTREAM_SCOPE_EXTERNAL,
                        route_mode: INVOCATION_ROUTE_MODE_FORWARD_PROXY,
                        sticky_key: sticky_key.as_deref(),
                        prompt_cache_key: prompt_cache_key.as_deref(),
                        upstream_account_id: None,
                        upstream_account_name: None,
                        oauth_account_header_attached: None,
                        oauth_account_id_shape: None,
                        oauth_forwarded_header_count: None,
                        oauth_forwarded_header_names: None,
                        oauth_fingerprint_version: None,
                        oauth_forwarded_header_fingerprints: None,
                        oauth_prompt_cache_header_forwarded: None,
                        oauth_request_body_prefix_fingerprint: None,
                        oauth_request_body_prefix_bytes: None,
                        oauth_responses_rewrite: None,
                        service_tier: None,
                        stream_terminal_event: None,
                        upstream_error_code: None,
                        upstream_error_message: None,
                        upstream_request_id: None,
                        response_content_encoding: None,
                        proxy_display_name: Some(err.selected_proxy.display_name.as_str()),
                        proxy_weight_delta: proxy_attempt_update.delta(),
                        pool_attempt_count: None,
                        pool_distinct_account_count: None,
                        pool_attempt_terminal_reason: None,
                    })),
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
    request_info.requested_service_tier = final_requested_service_tier
        .clone()
        .or(request_info.requested_service_tier);
    let req_raw = store_raw_payload_file(
        &state.config,
        &invoke_id,
        "request",
        final_request_body_for_capture
            .as_ref()
            .unwrap_or(&base_request_bytes_for_capture)
            .as_ref(),
    );

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
            let proxy_display_name = resolve_invocation_proxy_display_name(selected_proxy.as_ref());
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
                payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
                    target: capture_target,
                    status: StatusCode::BAD_GATEWAY,
                    is_stream: request_info.is_stream,
                    request_model: None,
                    requested_service_tier: request_info.requested_service_tier.as_deref(),
                    reasoning_effort: request_info.reasoning_effort.as_deref(),
                    response_model: None,
                    usage_missing_reason: None,
                    request_parse_error: request_info.parse_error.as_deref(),
                    failure_kind: None,
                    requester_ip: requester_ip.as_deref(),
                    upstream_scope: if pool_route_active {
                        INVOCATION_UPSTREAM_SCOPE_INTERNAL
                    } else {
                        INVOCATION_UPSTREAM_SCOPE_EXTERNAL
                    },
                    route_mode: if pool_route_active {
                        INVOCATION_ROUTE_MODE_POOL
                    } else {
                        INVOCATION_ROUTE_MODE_FORWARD_PROXY
                    },
                    sticky_key: sticky_key.as_deref(),
                    prompt_cache_key: prompt_cache_key.as_deref(),
                    upstream_account_id: pool_account.as_ref().map(|account| account.account_id),
                    upstream_account_name: pool_account
                        .as_ref()
                        .map(|account| account.display_name.as_str()),
                    oauth_account_header_attached: oauth_account_header_attached_for_account(
                        pool_account.as_ref(),
                    ),
                    oauth_account_id_shape: oauth_account_id_shape_for_account(
                        pool_account.as_ref(),
                    ),
                    oauth_forwarded_header_count: None,
                    oauth_forwarded_header_names: None,
                    oauth_fingerprint_version: None,
                    oauth_forwarded_header_fingerprints: None,
                    oauth_prompt_cache_header_forwarded: None,
                    oauth_request_body_prefix_fingerprint: None,
                    oauth_request_body_prefix_bytes: None,
                    oauth_responses_rewrite: None,
                    service_tier: None,
                    stream_terminal_event: None,
                    upstream_error_code: None,
                    upstream_error_message: None,
                    upstream_request_id: None,
                    response_content_encoding: Some(
                        summarize_response_content_encoding(
                            upstream_response
                                .headers()
                                .get(header::CONTENT_ENCODING)
                                .and_then(|value| value.to_str().ok()),
                        )
                        .as_str(),
                    ),
                    proxy_display_name: proxy_display_name.as_deref(),
                    proxy_weight_delta: if selected_proxy.is_some() {
                        proxy_attempt_update.delta()
                    } else {
                        None
                    },
                    pool_attempt_count: None,
                    pool_distinct_account_count: None,
                    pool_attempt_terminal_reason: None,
                })),
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
    let response_is_event_stream = upstream_response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    let upstream_content_encoding = upstream_response
        .headers()
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let response_content_encoding =
        summarize_response_content_encoding(upstream_content_encoding.as_deref());
    let selected_proxy_display_name =
        resolve_invocation_proxy_display_name(selected_proxy.as_ref());
    let response_running_record = build_running_proxy_capture_record(
        &invoke_id,
        &occurred_at,
        capture_target,
        &request_info,
        requester_ip.as_deref(),
        sticky_key.as_deref(),
        prompt_cache_key.as_deref(),
        pool_route_active,
        pool_account.as_ref().map(|account| account.account_id),
        pool_account
            .as_ref()
            .map(|account| account.display_name.as_str()),
        selected_proxy_display_name.as_deref(),
        pool_account
            .as_ref()
            .map(|_| pending_pool_attempt_summary.pool_attempt_count),
        pool_account
            .as_ref()
            .map(|_| pending_pool_attempt_summary.pool_distinct_account_count),
        None,
        Some(response_content_encoding.as_str()),
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        prefetched_ttfb_ms,
    );
    if let Err(err) =
        broadcast_proxy_capture_runtime_snapshot(&state.broadcaster, &response_running_record)
    {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast response-ready proxy capture snapshot"
        );
    }
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
    let reservation_key_for_task = pool_routing_reservation_key.clone();
    let prompt_cache_key_for_task = prompt_cache_key.clone();
    let selected_proxy_for_task = selected_proxy.clone();
    let selected_proxy_display_name_for_task = selected_proxy_display_name.clone();
    let pool_account_for_task = pool_account.clone();
    let oauth_responses_debug_for_task = oauth_responses_debug.clone();
    let attempt_already_recorded_for_task = attempt_already_recorded;
    let final_attempt_update_for_task = final_attempt_update;
    let pending_pool_attempt_record_for_task = pending_pool_attempt_record.clone();
    let pending_pool_attempt_summary_for_task = pending_pool_attempt_summary.clone();
    let prefetched_first_chunk_for_task = prefetched_first_chunk;
    let prefetched_ttfb_ms_for_task = prefetched_ttfb_ms;
    let upstream_attempt_started_at_for_task = upstream_attempt_started_at;
    let upstream_attempt_started_at_utc_for_task = upstream_attempt_started_at_utc;
    let first_byte_timeout_for_task = first_byte_timeout;
    let stream_timeout_for_task = stream_timeout;
    let response_is_event_stream_for_task = response_is_event_stream;
    let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);

    tokio::spawn(async move {
        let mut stream = upstream_response.into_bytes_stream();
        let ttfb_started = Instant::now();
        let stream_started = Instant::now();
        let mut t_upstream_ttfb_ms = prefetched_ttfb_ms_for_task;
        let mut stream_started_at: Option<Instant> = None;
        let mut response_preview = RawResponsePreviewBuffer::default();
        let mut response_raw_writer =
            StreamingRawPayloadWriter::new(&state_for_task.config, &invoke_id_for_task, "response");
        let mut stream_response_parser = StreamResponsePayloadChunkParser::default();
        let mut nonstream_parse_buffer = (!response_is_event_stream_for_task).then(|| {
            BoundedResponseParseBuffer::new(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES)
        });
        let mut stream_error: Option<String> = None;
        let mut downstream_closed = false;
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;

        if let Some(chunk) = prefetched_first_chunk_for_task {
            response_preview.append(&chunk);
            response_raw_writer.append(&chunk).await;
            stream_response_parser.ingest_bytes(&chunk);
            if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                buffer.append(&chunk);
            }
            forwarded_chunks = forwarded_chunks.saturating_add(1);
            forwarded_bytes = forwarded_bytes.saturating_add(chunk.len());
            stream_started_at = Some(Instant::now());
            if !downstream_closed && tx.send(Ok(chunk)).await.is_err() {
                downstream_closed = true;
            }
        }

        loop {
            let next_chunk = if let Some(stream_started_at) = stream_started_at {
                if let Some(stream_timeout) = stream_timeout_for_task {
                    let Some(timeout_budget) =
                        remaining_timeout_budget(stream_timeout, stream_started_at.elapsed())
                    else {
                        let message = pool_upstream_timeout_message(
                            stream_timeout,
                            "waiting for upstream stream completion",
                        );
                        stream_error = Some(message.clone());
                        if !downstream_closed
                            && tx.send(Err(io::Error::other(message))).await.is_err()
                        {
                            downstream_closed = true;
                        }
                        break;
                    };
                    match timeout(timeout_budget, stream.next()).await {
                        Ok(next_chunk) => next_chunk,
                        Err(_) => {
                            let message = pool_upstream_timeout_message(
                                stream_timeout,
                                "waiting for upstream stream completion",
                            );
                            stream_error = Some(message.clone());
                            if !downstream_closed
                                && tx.send(Err(io::Error::other(message))).await.is_err()
                            {
                                downstream_closed = true;
                            }
                            break;
                        }
                    }
                } else {
                    stream.next().await
                }
            } else if let Some(attempt_started_at) = upstream_attempt_started_at_for_task {
                let Some(timeout_budget) = remaining_timeout_budget(
                    first_byte_timeout_for_task,
                    attempt_started_at.elapsed(),
                ) else {
                    let message = pool_upstream_timeout_message(
                        first_byte_timeout_for_task,
                        "waiting for first upstream chunk",
                    );
                    stream_error = Some(message.clone());
                    if !downstream_closed && tx.send(Err(io::Error::other(message))).await.is_err()
                    {
                        downstream_closed = true;
                    }
                    break;
                };
                match timeout(timeout_budget, stream.next()).await {
                    Ok(next_chunk) => next_chunk,
                    Err(_) => {
                        let message = pool_upstream_timeout_message(
                            first_byte_timeout_for_task,
                            "waiting for first upstream chunk",
                        );
                        stream_error = Some(message.clone());
                        if !downstream_closed
                            && tx.send(Err(io::Error::other(message))).await.is_err()
                        {
                            downstream_closed = true;
                        }
                        break;
                    }
                }
            } else {
                stream.next().await
            };
            let Some(next_chunk) = next_chunk else {
                break;
            };
            match next_chunk {
                Ok(chunk) => {
                    if stream_started_at.is_none() {
                        t_upstream_ttfb_ms = upstream_attempt_started_at_for_task
                            .map(elapsed_ms)
                            .unwrap_or_else(|| elapsed_ms(ttfb_started));
                        stream_started_at = Some(Instant::now());
                        let running_record = build_running_proxy_capture_record(
                            &invoke_id_for_task,
                            &occurred_at_for_task,
                            capture_target,
                            &request_info_for_task,
                            requester_ip_for_task.as_deref(),
                            sticky_key_for_task.as_deref(),
                            prompt_cache_key_for_task.as_deref(),
                            pool_account_for_task.is_some(),
                            pool_account_for_task
                                .as_ref()
                                .map(|account| account.account_id),
                            pool_account_for_task
                                .as_ref()
                                .map(|account| account.display_name.as_str()),
                            selected_proxy_display_name_for_task.as_deref(),
                            pool_account_for_task
                                .as_ref()
                                .map(|_| pending_pool_attempt_summary_for_task.pool_attempt_count),
                            pool_account_for_task.as_ref().map(|_| {
                                pending_pool_attempt_summary_for_task.pool_distinct_account_count
                            }),
                            None,
                            Some(response_content_encoding.as_str()),
                            t_req_read_ms,
                            t_req_parse_ms,
                            t_upstream_connect_ms,
                            t_upstream_ttfb_ms,
                        );
                        if let Err(err) = broadcast_proxy_capture_runtime_snapshot(
                            &state_for_task.broadcaster,
                            &running_record,
                        ) {
                            warn!(
                                ?err,
                                invoke_id = %invoke_id_for_task,
                                "failed to broadcast first-byte proxy capture snapshot"
                            );
                        }
                    }
                    response_preview.append(&chunk);
                    response_raw_writer.append(&chunk).await;
                    stream_response_parser.ingest_bytes(&chunk);
                    if let Some(buffer) = nonstream_parse_buffer.as_mut() {
                        buffer.append(&chunk);
                    }
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
        let resp_raw = response_raw_writer.finish().await;
        let preview_bytes = response_preview.as_slice().to_vec();
        let raw_response_preview = response_preview.into_preview();
        let streamed_response_outcome = stream_response_parser.finish();
        let preview_looks_like_sse = response_payload_looks_like_sse_after_decode(
            &preview_bytes,
            upstream_content_encoding_for_task.as_deref(),
        );
        let response_is_stream_hint = response_is_event_stream_for_task
            || streamed_response_outcome.saw_stream_fields
            || preview_looks_like_sse;
        let resp_parse_started = Instant::now();
        let mut response_info = if response_is_stream_hint {
            if streamed_response_outcome.saw_stream_fields {
                streamed_response_outcome.response_info
            } else {
                parse_target_response_preview_payload(
                    capture_target,
                    &preview_bytes,
                    true,
                    upstream_content_encoding_for_task.as_deref(),
                )
            }
        } else {
            nonstream_parse_buffer
                .take()
                .map(|buffer| {
                    buffer.into_response_info(
                        capture_target,
                        upstream_content_encoding_for_task.as_deref(),
                    )
                })
                .unwrap_or_else(|| {
                    parse_target_response_payload(
                        capture_target,
                        &preview_bytes,
                        false,
                        upstream_content_encoding_for_task.as_deref(),
                    )
                })
        };
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
            response_info
                .upstream_error_message
                .clone()
                .or_else(|| extract_error_message_from_response_preview(&preview_bytes))
        } else {
            None
        };
        let status = if upstream_status.is_success() && error_message.is_none() {
            "success".to_string()
        } else {
            format!("http_{}", upstream_status.as_u16())
        };
        let pending_pool_attempt_terminal_reason = if pool_account_for_task.is_none() {
            None
        } else if had_stream_error {
            Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string())
        } else if downstream_closed {
            Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED.to_string())
        } else if response_info.stream_terminal_event.is_some() {
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string())
        } else if !upstream_status.is_success() {
            Some(
                failure_kind
                    .unwrap_or(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
                    .to_string(),
            )
        } else {
            None
        };
        let mut pending_pool_attempt_summary = pending_pool_attempt_summary_for_task.clone();
        pending_pool_attempt_summary.pool_attempt_terminal_reason =
            pending_pool_attempt_terminal_reason.clone();
        let selected_proxy_display_name =
            resolve_invocation_proxy_display_name(selected_proxy_for_task.as_ref());
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
                    consume_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    record_pool_route_success(
                        &state_for_task.pool,
                        account.account_id,
                        upstream_attempt_started_at_utc_for_task.unwrap_or_else(Utc::now),
                        sticky_key_for_task.as_deref(),
                        None,
                    )
                    .await
                } else if had_stream_error {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream stream error")
                        .to_string();
                    release_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    record_pool_route_transport_failure(
                        &state_for_task.pool,
                        account.account_id,
                        sticky_key_for_task.as_deref(),
                        &route_message,
                        None,
                    )
                    .await
                } else {
                    let route_message = error_message
                        .as_deref()
                        .unwrap_or("upstream request failed")
                        .to_string();
                    release_pool_routing_reservation(
                        state_for_task.as_ref(),
                        &reservation_key_for_task,
                    );
                    if response_info_is_retryable_server_overloaded(upstream_status, &response_info)
                    {
                        record_pool_route_retryable_overload_failure(
                            &state_for_task.pool,
                            account.account_id,
                            sticky_key_for_task.as_deref(),
                            &route_message,
                            None,
                        )
                        .await
                    } else {
                        record_pool_route_http_failure(
                            &state_for_task.pool,
                            account.account_id,
                            &account.kind,
                            sticky_key_for_task.as_deref(),
                            upstream_status,
                            &route_message,
                            None,
                        )
                        .await
                    }
                };
                if let Err(err) = route_result {
                    warn!(account_id = account.account_id, error = %err, "failed to record pool capture route state");
                }
            }
            ForwardProxyAttemptUpdate::default()
        };
        if let Some(pending_attempt_record) = pending_pool_attempt_record_for_task.as_ref() {
            let finished_at = shanghai_now_string();
            let attempt_status = if had_stream_error || downstream_closed {
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
            } else if !upstream_status.is_success() || had_logical_stream_failure {
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
            } else {
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
            };
            if let Err(err) = finalize_pool_upstream_request_attempt(
                &state_for_task.pool,
                pending_attempt_record,
                finished_at.as_str(),
                attempt_status,
                Some(upstream_status),
                failure_kind,
                error_message.as_deref(),
                Some(t_upstream_connect_ms),
                Some(t_upstream_ttfb_ms),
                Some(t_upstream_stream_ms),
                response_info.upstream_request_id.as_deref(),
                None,
                None,
            )
            .await
            {
                warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to persist final pool attempt"
                );
            }
            if let Err(err) = broadcast_pool_upstream_attempts_snapshot(
                state_for_task.as_ref(),
                &pending_attempt_record.invoke_id,
            )
            .await
            {
                warn!(
                    invoke_id = %pending_attempt_record.invoke_id,
                    error = %err,
                    "failed to broadcast final pool attempt snapshot"
                );
            }
        }
        let (cost, cost_estimated, price_version) = estimate_proxy_cost_from_shared_catalog(
            &state_for_task.pricing_catalog,
            response_info.model.as_deref(),
            &response_info.usage,
        )
        .await;
        let payload = build_proxy_payload_summary(ProxyPayloadSummary {
            target: capture_target,
            status: upstream_status,
            is_stream: request_info_for_task.is_stream,
            request_model: request_info_for_task.model.as_deref(),
            requested_service_tier: request_info_for_task.requested_service_tier.as_deref(),
            reasoning_effort: request_info_for_task.reasoning_effort.as_deref(),
            response_model: response_info.model.as_deref(),
            usage_missing_reason: response_info.usage_missing_reason.as_deref(),
            request_parse_error: request_info_for_task.parse_error.as_deref(),
            failure_kind,
            requester_ip: requester_ip_for_task.as_deref(),
            upstream_scope: if pool_account_for_task.is_some() {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_account_for_task.is_some() {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key: sticky_key_for_task.as_deref(),
            prompt_cache_key: prompt_cache_key_for_task.as_deref(),
            upstream_account_id: pool_account_for_task
                .as_ref()
                .map(|account| account.account_id),
            upstream_account_name: pool_account_for_task
                .as_ref()
                .map(|account| account.display_name.as_str()),
            oauth_account_header_attached: oauth_account_header_attached_for_account(
                pool_account_for_task.as_ref(),
            ),
            oauth_account_id_shape: oauth_account_id_shape_for_account(
                pool_account_for_task.as_ref(),
            ),
            oauth_forwarded_header_count: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| debug.forwarded_header_names.len()),
            oauth_forwarded_header_names: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| debug.forwarded_header_names.as_slice()),
            oauth_fingerprint_version: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.fingerprint_version),
            oauth_forwarded_header_fingerprints: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.forwarded_header_fingerprints.as_ref()),
            oauth_prompt_cache_header_forwarded: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| debug.prompt_cache_header_forwarded),
            oauth_request_body_prefix_fingerprint: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.request_body_prefix_fingerprint.as_deref()),
            oauth_request_body_prefix_bytes: oauth_responses_debug_for_task
                .as_ref()
                .and_then(|debug| debug.request_body_prefix_bytes),
            oauth_responses_rewrite: oauth_responses_debug_for_task
                .as_ref()
                .map(|debug| &debug.rewrite),
            service_tier: response_info.service_tier.as_deref(),
            stream_terminal_event: response_info.stream_terminal_event.as_deref(),
            upstream_error_code: response_info.upstream_error_code.as_deref(),
            upstream_error_message: response_info.upstream_error_message.as_deref(),
            upstream_request_id: response_info.upstream_request_id.as_deref(),
            response_content_encoding: Some(response_content_encoding.as_str()),
            proxy_display_name: selected_proxy_display_name.as_deref(),
            proxy_weight_delta: if selected_proxy_for_task.is_some() {
                proxy_attempt_update.delta()
            } else {
                None
            },
            pool_attempt_count: pool_account_for_task
                .as_ref()
                .map(|_| pending_pool_attempt_summary.pool_attempt_count),
            pool_distinct_account_count: pool_account_for_task
                .as_ref()
                .map(|_| pending_pool_attempt_summary.pool_distinct_account_count),
            pool_attempt_terminal_reason: pool_account_for_task
                .as_ref()
                .and_then(|_| pending_pool_attempt_terminal_reason.as_deref()),
        });

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
            raw_response: raw_response_preview,
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

async fn read_request_body_snapshot_with_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<PoolReplayBodySnapshot, RequestBodyReadError> {
    let mut buffer = PoolReplayBodyBuffer::new(proxy_request_id);
    let mut stream = body.into_data_stream();
    let read_deadline = Instant::now() + request_read_timeout;
    let mut data_len = 0usize;

    loop {
        let remaining = read_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                proxy_request_id,
                timeout_ms = request_read_timeout.as_millis(),
                read_bytes = data_len,
                "openai proxy request body read timed out"
            );
            return Err(RequestBodyReadError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    request_read_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                partial_body: Vec::new(),
            });
        }

        let next_chunk = match timeout(remaining, stream.next()).await {
            Ok(chunk) => chunk,
            Err(_) => {
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data_len,
                    "openai proxy request body read timed out"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: Vec::new(),
                });
            }
        };

        let Some(chunk) = next_chunk else {
            return buffer.finish().await.map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for oauth replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: Vec::new(),
            });
        };

        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    error = %err,
                    read_bytes = data_len,
                    "openai proxy request body stream error"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("failed to read request body stream: {err}"),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                    partial_body: Vec::new(),
                });
            }
        };

        if data_len.saturating_add(chunk.len()) > body_limit {
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body: Vec::new(),
            });
        }
        data_len = data_len.saturating_add(chunk.len());

        buffer
            .append(&chunk)
            .await
            .map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for oauth replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: Vec::new(),
            })?;
    }
}

fn prepare_target_request_body(
    target: ProxyCaptureTarget,
    body: Vec<u8>,
    auto_include_usage: bool,
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

    let mut rewritten = false;
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

fn proxy_upstream_send_timeout_for_capture_target(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    capture_target: Option<ProxyCaptureTarget>,
) -> Duration {
    match capture_target {
        Some(ProxyCaptureTarget::Responses) => timeouts.responses_first_byte_timeout,
        Some(ProxyCaptureTarget::ResponsesCompact) => timeouts.compact_first_byte_timeout,
        _ => timeouts.default_send_timeout,
    }
}

fn pool_upstream_first_chunk_timeout(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    original_uri: &Uri,
    method: &Method,
) -> Duration {
    match capture_target_for_request(original_uri.path(), method) {
        Some(ProxyCaptureTarget::Responses) => timeouts.responses_first_byte_timeout,
        Some(ProxyCaptureTarget::ResponsesCompact) => timeouts.compact_first_byte_timeout,
        _ => timeouts.default_first_byte_timeout,
    }
}

fn pool_upstream_responses_total_timeout(
    _config: &AppConfig,
    original_uri: &Uri,
    method: &Method,
) -> Option<Duration> {
    let _ = (original_uri, method);
    None
}

fn proxy_capture_target_stream_timeout(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    capture_target: ProxyCaptureTarget,
) -> Option<Duration> {
    match capture_target {
        ProxyCaptureTarget::Responses => Some(timeouts.responses_stream_timeout),
        ProxyCaptureTarget::ResponsesCompact => Some(timeouts.compact_stream_timeout),
        ProxyCaptureTarget::ChatCompletions => None,
    }
}

fn pool_upstream_send_timeout(
    original_uri: &Uri,
    method: &Method,
    send_timeout: Duration,
    pre_first_byte_timeout: Duration,
) -> Duration {
    if pool_uses_responses_timeout_failover_policy(original_uri, method) {
        pre_first_byte_timeout
    } else {
        send_timeout
    }
}

fn pool_uses_responses_timeout_failover_policy(original_uri: &Uri, method: &Method) -> bool {
    method == Method::POST
        && matches!(
            original_uri.path(),
            "/v1/responses" | "/v1/responses/compact"
        )
}

fn pool_timeout_budget_with_total_limit(
    timeout: Duration,
    total_timeout: Option<Duration>,
    total_timeout_started_at: Option<Instant>,
) -> Option<Duration> {
    match (total_timeout, total_timeout_started_at) {
        (Some(total_timeout), Some(started_at)) => {
            remaining_timeout_budget(total_timeout, started_at.elapsed())
                .map(|remaining| remaining.min(timeout))
        }
        (Some(total_timeout), None) => Some(timeout.min(total_timeout)),
        (None, _) => Some(timeout),
    }
}

fn ensure_pool_total_timeout_started_at(
    total_timeout: Option<Duration>,
    total_timeout_started_at: &mut Option<Instant>,
) -> Option<Instant> {
    if total_timeout.is_some() && total_timeout_started_at.is_none() {
        *total_timeout_started_at = Some(Instant::now());
    }
    *total_timeout_started_at
}

fn pool_total_timeout_exhausted(total_timeout: Duration, started_at: Instant) -> bool {
    timeout_budget_exhausted(total_timeout, started_at.elapsed())
}

fn pool_total_timeout_exhausted_message(total_timeout: Duration) -> String {
    format!(
        "pool upstream total timeout exhausted after {}ms",
        total_timeout.as_millis()
    )
}

fn build_pool_total_timeout_exhausted_error(
    total_timeout: Duration,
    last_error: Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    let mut final_error = last_error.unwrap_or(PoolUpstreamError {
        account: None,
        status: StatusCode::GATEWAY_TIMEOUT,
        message: pool_total_timeout_exhausted_message(total_timeout),
        failure_kind: PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: None,
        oauth_responses_debug: None,
        attempt_summary: PoolAttemptSummary::default(),
        requested_service_tier: None,
        request_body_for_capture: None,
    });
    final_error.status = StatusCode::GATEWAY_TIMEOUT;
    final_error.message = pool_total_timeout_exhausted_message(total_timeout);
    final_error.failure_kind = PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED;
    final_error.upstream_error_code = None;
    final_error.upstream_error_message = None;
    final_error.upstream_request_id = None;
    final_error.attempt_summary = pool_attempt_summary(
        attempt_count,
        distinct_account_count,
        Some(PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED.to_string()),
    );
    final_error
}

fn pool_uses_responses_family_retry_budget_policy(original_uri: &Uri, method: &Method) -> bool {
    method == Method::POST
        && matches!(
            original_uri.path(),
            "/v1/responses" | "/v1/responses/compact"
        )
}

fn pool_same_account_attempt_budget(
    original_uri: &Uri,
    method: &Method,
    distinct_account_count: usize,
    initial_same_account_attempts: u8,
) -> u8 {
    if pool_uses_responses_family_retry_budget_policy(original_uri, method) {
        if distinct_account_count <= 1 {
            initial_same_account_attempts.max(1)
        } else {
            1
        }
    } else if distinct_account_count <= 1 {
        initial_same_account_attempts.max(1)
    } else {
        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS
    }
}

fn pool_error_message_indicates_proxy_timeout(message: &str) -> bool {
    let message_lower = message.trim().to_ascii_lowercase();
    message_lower.contains("request timed out after")
        || message_lower.contains("upstream handshake timed out after")
}

fn pool_failure_is_timeout_shaped(failure_kind: &str, message: &str) -> bool {
    matches!(
        failure_kind,
        PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
            | PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
            | PROXY_FAILURE_UPSTREAM_STREAM_ERROR
            | PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
    ) && pool_error_message_indicates_proxy_timeout(message)
}

fn pool_account_forward_proxy_scope(
    account: &PoolResolvedAccount,
) -> std::result::Result<ForwardProxyRouteScope, String> {
    Ok(account.forward_proxy_scope.clone())
}

async fn select_pool_account_forward_proxy_client(
    state: &AppState,
    account: &PoolResolvedAccount,
) -> Result<(ForwardProxyRouteScope, SelectedForwardProxy, Client), String> {
    let scope = pool_account_forward_proxy_scope(account)?;
    let selected_proxy = select_forward_proxy_for_scope(state, &scope)
        .await
        .map_err(|err| match &scope {
            ForwardProxyRouteScope::BoundGroup { group_name, .. }
                if err
                    .to_string()
                    .contains("bound forward proxy group has no selectable nodes") =>
            {
                format!(
                    "upstream account group \"{group_name}\" has no selectable bound forward proxy nodes"
                )
            }
            _ => format!("failed to select forward proxy node: {err}"),
        })?;
    let client = match state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
    {
        Ok(client) => client,
        Err(err) => {
            record_forward_proxy_scope_result(
                state,
                &scope,
                &selected_proxy.key,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
            return Err(format!("failed to initialize forward proxy client: {err}"));
        }
    };
    Ok((scope, selected_proxy, client))
}

async fn record_pool_account_forward_proxy_result(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    selected_proxy: &SelectedForwardProxy,
    result: ForwardProxyRouteResultKind,
) {
    record_forward_proxy_scope_result(state, scope, &selected_proxy.key, result).await;
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
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };

    match fast_mode_rewrite_mode {
        TagFastModeRewriteMode::KeepOriginal => false,
        TagFastModeRewriteMode::ForceRemove => {
            let removed_snake = object.remove("service_tier").is_some();
            let removed_camel = object.remove("serviceTier").is_some();
            removed_snake || removed_camel
        }
        TagFastModeRewriteMode::FillMissing => {
            let has_existing_service_tier =
                object.contains_key("service_tier") || object.contains_key("serviceTier");
            if has_existing_service_tier {
                false
            } else {
                object.insert(
                    "service_tier".to_string(),
                    Value::String("priority".to_string()),
                );
                true
            }
        }
        TagFastModeRewriteMode::ForceAdd => {
            let mut rewritten = object.remove("serviceTier").is_some();
            if object.get("service_tier").and_then(|entry| entry.as_str()) != Some("priority") {
                object.insert(
                    "service_tier".to_string(),
                    Value::String("priority".to_string()),
                );
                rewritten = true;
            }
            rewritten
        }
    }
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

fn build_response_capture_info_from_bytes(
    bytes: &[u8],
    request_is_stream: bool,
    decode_failure_reason: Option<String>,
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

    let looks_like_stream = request_is_stream || response_payload_looks_like_sse(bytes);
    let mut response_info = if looks_like_stream {
        parse_stream_response_payload(bytes)
    } else {
        match serde_json::from_slice::<Value>(bytes) {
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
            Err(_) => {
                let model = extract_partial_json_model(bytes);
                let service_tier = extract_partial_json_service_tier(bytes);
                let upstream_error_code = extract_partial_json_string_field(bytes, &["code"]);
                let upstream_error_message = extract_partial_json_string_field(bytes, &["message"]);
                let upstream_request_id =
                    extract_partial_json_string_field(bytes, &["request_id", "requestId"]).or_else(
                        || {
                            upstream_error_message
                                .as_deref()
                                .and_then(extract_request_id_from_message)
                        },
                    );
                ResponseCaptureInfo {
                    model,
                    usage: ParsedUsage::default(),
                    usage_missing_reason: Some("response_not_json".to_string()),
                    service_tier,
                    stream_terminal_event: None,
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                }
            }
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

fn parse_target_response_payload(
    _target: ProxyCaptureTarget,
    bytes: &[u8],
    request_is_stream: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_parse(bytes, content_encoding);
    build_response_capture_info_from_bytes(
        decoded_bytes.as_ref(),
        request_is_stream,
        decode_failure_reason,
    )
}
fn parse_target_response_preview_payload(
    _target: ProxyCaptureTarget,
    bytes: &[u8],
    request_is_stream: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_preview_parse(bytes, content_encoding);
    build_response_capture_info_from_bytes(
        decoded_bytes.as_ref(),
        request_is_stream,
        decode_failure_reason,
    )
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

fn response_payload_looks_like_sse_after_decode(
    bytes: &[u8],
    content_encoding: Option<&str>,
) -> bool {
    let (decoded, _) = decode_response_payload_for_preview_parse(bytes, content_encoding);
    response_payload_looks_like_sse(decoded.as_ref())
}

#[cfg(test)]
static RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
fn reset_response_capture_raw_fallback_counters() {
    RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS.store(0, Ordering::Relaxed);
    RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
fn response_capture_raw_fallback_counts() -> (usize, usize) {
    (
        RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS.load(Ordering::Relaxed),
        RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.load(Ordering::Relaxed),
    )
}

#[allow(dead_code)]
fn response_payload_looks_like_sse_from_raw_file(
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<bool, String> {
    let mut reader = open_decoded_response_reader(path, content_encoding)?;
    let mut decoded_prefix = Vec::new();
    reader
        .by_ref()
        .take((RAW_RESPONSE_PREVIEW_LIMIT + 1) as u64)
        .read_to_end(&mut decoded_prefix)
        .map_err(|err| err.to_string())?;
    Ok(response_payload_looks_like_sse(&decoded_prefix))
}

#[allow(dead_code)]
fn response_payload_looks_like_sse_from_capture(
    resp_raw: &RawPayloadMeta,
    preview_bytes: &[u8],
    content_encoding: Option<&str>,
) -> bool {
    #[cfg(test)]
    RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS.fetch_add(1, Ordering::Relaxed);

    if response_payload_looks_like_sse_after_decode(preview_bytes, content_encoding) {
        return true;
    }

    if preview_bytes.len() < RAW_RESPONSE_PREVIEW_LIMIT && content_encoding.is_none() {
        return false;
    }

    let Some(path) = resp_raw.path.as_deref() else {
        return false;
    };

    response_payload_looks_like_sse_from_raw_file(&PathBuf::from(path), content_encoding)
        .unwrap_or(false)
}

fn decode_response_payload_for_parse<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    decode_response_payload(bytes, content_encoding, false)
}

fn decode_response_payload_for_preview_parse<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        return (Cow::Borrowed(bytes), None);
    }

    let mut decoded = bytes.to_vec();
    for encoding in encodings.iter().rev() {
        match decode_single_content_encoding_lossy(decoded.as_slice(), encoding) {
            Ok((next, None)) => decoded = next,
            Ok((next, Some(err))) => return (Cow::Owned(next), Some(format!("{encoding}:{err}"))),
            Err(err) => return (Cow::Borrowed(bytes), Some(format!("{encoding}:{err}"))),
        }
    }

    (Cow::Owned(decoded), None)
}

fn read_decoder_lossy(
    mut reader: impl Read,
) -> std::result::Result<(Vec<u8>, Option<String>), String> {
    let mut decoded = Vec::new();
    match reader.read_to_end(&mut decoded) {
        Ok(_) => Ok((decoded, None)),
        Err(err) if !decoded.is_empty() => Ok((decoded, Some(err.to_string()))),
        Err(err) => Err(err.to_string()),
    }
}

fn decode_single_content_encoding_lossy(
    bytes: &[u8],
    encoding: &str,
) -> std::result::Result<(Vec<u8>, Option<String>), String> {
    match encoding {
        "identity" => Ok((bytes.to_vec(), None)),
        "gzip" | "x-gzip" => read_decoder_lossy(GzDecoder::new(bytes)),
        "br" => read_decoder_lossy(BrotliDecompressor::new(bytes, 4096)),
        "deflate" => {
            let mut zlib_decoder = ZlibDecoder::new(bytes);
            let mut decoded = Vec::new();
            match zlib_decoder.read_to_end(&mut decoded) {
                Ok(_) => Ok((decoded, None)),
                Err(zlib_err) if !decoded.is_empty() => Ok((decoded, Some(zlib_err.to_string()))),
                Err(zlib_err) => {
                    let mut raw_decoder = DeflateDecoder::new(bytes);
                    let mut raw_decoded = Vec::new();
                    match raw_decoder.read_to_end(&mut raw_decoded) {
                        Ok(_) => Ok((raw_decoded, None)),
                        Err(raw_err) if !raw_decoded.is_empty() => {
                            Ok((raw_decoded, Some(raw_err.to_string())))
                        }
                        Err(raw_err) => Err(format!("zlib={zlib_err}; raw={raw_err}")),
                    }
                }
            }
        }
        other => Err(format!("unsupported_content_encoding:{other}")),
    }
}

#[derive(Default)]
struct StreamResponsePayloadParser {
    model: Option<String>,
    usage: ParsedUsage,
    service_tier: Option<String>,
    stream_terminal_event: Option<String>,
    upstream_error_code: Option<String>,
    upstream_error_message: Option<String>,
    upstream_request_id: Option<String>,
    usage_found: bool,
    parse_error_seen: bool,
    pending_event_name: Option<String>,
    saw_stream_fields: bool,
}

impl StreamResponsePayloadParser {
    fn ingest_line(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.starts_with("event:") {
            self.saw_stream_fields = true;
            self.pending_event_name = Some(trimmed.trim_start_matches("event:").trim().to_string());
            return;
        }
        if !trimmed.starts_with("data:") {
            return;
        }
        self.saw_stream_fields = true;
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            self.pending_event_name = None;
            return;
        }
        match serde_json::from_str::<Value>(payload) {
            Ok(value) => {
                let event_name = self.pending_event_name.take();
                if self.model.is_none() {
                    self.model = extract_model_from_payload(&value);
                }
                if self.service_tier.is_none() {
                    self.service_tier = extract_service_tier_from_payload(&value);
                }
                if let Some(parsed_usage) = extract_usage_from_payload(&value) {
                    self.usage = parsed_usage;
                    self.usage_found = true;
                }
                if stream_payload_indicates_failure(event_name.as_deref(), &value) {
                    let candidate = event_name
                        .clone()
                        .or_else(|| extract_stream_payload_type(&value))
                        .unwrap_or_else(|| "response.failed".to_string());
                    if self.stream_terminal_event.is_none() || candidate == "response.failed" {
                        self.stream_terminal_event = Some(candidate);
                    }
                }
                if self.upstream_error_code.is_none() {
                    self.upstream_error_code = extract_upstream_error_code(&value);
                }
                if self.upstream_error_message.is_none() {
                    self.upstream_error_message = extract_upstream_error_message(&value);
                }
                if self.upstream_request_id.is_none() {
                    self.upstream_request_id = extract_upstream_request_id(&value);
                }
            }
            Err(_) => {
                self.pending_event_name = None;
                self.parse_error_seen = true;
            }
        }
    }

    fn finish(self) -> ResponseCaptureInfo {
        let usage_missing_reason = if self.usage_found {
            None
        } else if self.stream_terminal_event.is_some() {
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string())
        } else if self.parse_error_seen {
            Some("stream_event_parse_error".to_string())
        } else {
            Some("usage_missing_in_stream".to_string())
        };

        ResponseCaptureInfo {
            model: self.model,
            usage: self.usage,
            usage_missing_reason,
            service_tier: self.service_tier,
            stream_terminal_event: self.stream_terminal_event,
            upstream_error_code: self.upstream_error_code,
            upstream_error_message: self.upstream_error_message,
            upstream_request_id: self.upstream_request_id,
        }
    }
}

struct StreamResponsePayloadParseOutcome {
    response_info: ResponseCaptureInfo,
    saw_stream_fields: bool,
}

struct StreamResponsePayloadChunkParser {
    parser: StreamResponsePayloadParser,
    line_buffer: Vec<u8>,
    discarding_oversized_line: bool,
    line_buffer_limit: usize,
    discarded_oversized_line: bool,
}

impl Default for StreamResponsePayloadChunkParser {
    fn default() -> Self {
        Self::with_line_buffer_limit(STREAM_RESPONSE_LINE_BUFFER_LIMIT)
    }
}

impl StreamResponsePayloadChunkParser {
    fn with_line_buffer_limit(line_buffer_limit: usize) -> Self {
        Self {
            parser: StreamResponsePayloadParser::default(),
            line_buffer: Vec::new(),
            discarding_oversized_line: false,
            line_buffer_limit,
            discarded_oversized_line: false,
        }
    }

    fn line_bytes_look_like_stream_field(line: &[u8]) -> bool {
        let decoded = String::from_utf8_lossy(line);
        let trimmed = decoded.trim_start();
        trimmed.starts_with("data:")
            || trimmed.starts_with("event:")
            || trimmed.starts_with("id:")
            || trimmed.starts_with("retry:")
    }

    fn flush_line(&mut self) {
        if self.line_buffer.is_empty() {
            return;
        }
        let decoded = String::from_utf8_lossy(&self.line_buffer);
        self.parser.ingest_line(decoded.as_ref());
        self.line_buffer.clear();
    }

    fn start_discarding_oversized_line(&mut self) {
        if Self::line_bytes_look_like_stream_field(&self.line_buffer) {
            self.parser.saw_stream_fields = true;
        }
        self.parser.parse_error_seen = true;
        self.discarded_oversized_line = true;
        self.line_buffer.clear();
        self.discarding_oversized_line = true;
    }

    fn append_segment(&mut self, segment: &[u8], ends_line: bool) {
        if self.discarding_oversized_line {
            if ends_line {
                self.discarding_oversized_line = false;
            }
            return;
        }

        if self.line_buffer.len().saturating_add(segment.len()) > self.line_buffer_limit {
            self.start_discarding_oversized_line();
            if ends_line {
                self.discarding_oversized_line = false;
            }
            return;
        }

        self.line_buffer.extend_from_slice(segment);
        if ends_line {
            self.flush_line();
        }
    }

    fn ingest_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        let mut line_start = 0usize;
        for (idx, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                self.append_segment(&bytes[line_start..=idx], true);
                line_start = idx + 1;
            }
        }
        if line_start < bytes.len() {
            self.append_segment(&bytes[line_start..], false);
        }
    }

    fn finish(mut self) -> StreamResponsePayloadParseOutcome {
        if self.discarding_oversized_line {
            self.parser.parse_error_seen = true;
        } else {
            self.flush_line();
        }
        StreamResponsePayloadParseOutcome {
            saw_stream_fields: self.parser.saw_stream_fields,
            response_info: self.parser.finish(),
        }
    }
}

fn parse_stream_response_payload(bytes: &[u8]) -> ResponseCaptureInfo {
    let mut parser = StreamResponsePayloadChunkParser::default();
    parser.ingest_bytes(bytes);
    parser.finish().response_info
}

#[allow(dead_code)]
fn parse_stream_response_payload_from_reader<R: Read>(
    reader: R,
) -> io::Result<ResponseCaptureInfo> {
    let mut parser = StreamResponsePayloadChunkParser::with_line_buffer_limit(
        RAW_FILE_STREAM_RESPONSE_LINE_BUFFER_LIMIT,
    );
    let mut reader = io::BufReader::new(reader);
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        parser.ingest_bytes(&chunk[..read]);
    }
    Ok(parser.finish().response_info)
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

fn find_first_sse_event_boundary(bytes: &[u8]) -> Option<usize> {
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\n' && bytes[index + 1] == b'\n' {
            return Some(index + 2);
        }
        if index + 3 < bytes.len()
            && bytes[index] == b'\r'
            && bytes[index + 1] == b'\n'
            && bytes[index + 2] == b'\r'
            && bytes[index + 3] == b'\n'
        {
            return Some(index + 4);
        }
        index += 1;
    }
    None
}

fn rebuild_proxy_upstream_response_stream(
    status: StatusCode,
    headers: &HeaderMap,
    stream: Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>>,
) -> Result<ProxyUpstreamResponseBody, String> {
    let mut response_builder = Response::builder().status(status);
    for (name, value) in headers {
        response_builder = response_builder.header(name, value);
    }
    response_builder
        .body(Body::from_stream(stream))
        .map(ProxyUpstreamResponseBody::Axum)
        .map_err(|err| format!("failed to rebuild upstream response stream: {err}"))
}

enum PoolInitialSseGateOutcome {
    Forward {
        response: ProxyUpstreamResponseBody,
        prefetched_bytes: Option<Bytes>,
    },
    RetrySameAccount {
        message: String,
        upstream_error_code: Option<String>,
        upstream_error_message: Option<String>,
        upstream_request_id: Option<String>,
    },
}

async fn gate_pool_initial_response_stream(
    response: ProxyUpstreamResponseBody,
    prefetched_first_chunk: Option<Bytes>,
    total_timeout: Duration,
    started: Instant,
) -> Result<PoolInitialSseGateOutcome, String> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut stream = response.into_bytes_stream();
    let mut buffered = Vec::new();
    if let Some(chunk) = prefetched_first_chunk {
        buffered.extend_from_slice(&chunk);
    }

    let mut gate_stream_error: Option<io::Error> = None;
    loop {
        if let Some(event_end) = find_first_sse_event_boundary(&buffered) {
            let response_info = parse_stream_response_payload(&buffered[..event_end]);
            if response_info_is_retryable_server_overloaded(status, &response_info) {
                return Ok(PoolInitialSseGateOutcome::RetrySameAccount {
                    message: format_upstream_response_failed_message(&response_info),
                    upstream_error_code: response_info.upstream_error_code,
                    upstream_error_message: response_info.upstream_error_message,
                    upstream_request_id: response_info.upstream_request_id,
                });
            }
            break;
        }
        if buffered.len() >= RAW_RESPONSE_PREVIEW_LIMIT {
            break;
        }

        let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed())
        else {
            break;
        };
        let next_chunk = match timeout(timeout_budget, stream.next()).await {
            Ok(next_chunk) => next_chunk,
            Err(_) => break,
        };
        let Some(next_chunk) = next_chunk else {
            break;
        };
        match next_chunk {
            Ok(chunk) => buffered.extend_from_slice(&chunk),
            Err(err) => {
                gate_stream_error = Some(io::Error::other(err.to_string()));
                break;
            }
        }
    }

    let remaining_stream: Pin<
        Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>,
    > = if let Some(err) = gate_stream_error {
        Box::pin(stream::once(async move { Err(err) }))
    } else {
        stream
    };
    let rebuilt_response =
        rebuild_proxy_upstream_response_stream(status, &headers, remaining_stream)?;
    Ok(PoolInitialSseGateOutcome::Forward {
        response: rebuilt_response,
        prefetched_bytes: (!buffered.is_empty()).then_some(Bytes::from(buffered)),
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

fn extract_partial_json_string_field(bytes: &[u8], keys: &[&str]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    keys.iter().find_map(|key| {
        let pattern = format!(r#""{}"\s*:\s*"((?:\\.|[^"\\])*)""#, regex::escape(key));
        let regex = Regex::new(&pattern).ok()?;
        let captures = regex.captures(text)?;
        let value = captures.get(1)?.as_str();
        serde_json::from_str::<String>(&format!("\"{value}\"")).ok()
    })
}

fn extract_partial_json_model(bytes: &[u8]) -> Option<String> {
    extract_partial_json_string_field(bytes, &["model"])
}

fn extract_partial_json_service_tier(bytes: &[u8]) -> Option<String> {
    extract_partial_json_string_field(bytes, &["service_tier", "serviceTier"])
        .and_then(|value| normalize_service_tier(&value))
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

fn upstream_account_name_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("upstreamAccountName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn prompt_cache_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("promptCacheKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn sticky_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("stickyKey")
        .or_else(|| value.get("promptCacheKey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn shanghai_now_string() -> String {
    format_naive(Utc::now().with_timezone(&Shanghai).naive_local())
}

fn terminal_pool_upstream_request_attempt_phase(status: &str) -> &'static str {
    if status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
    }
}

async fn insert_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    upstream_account_id: Option<i64>,
    upstream_route_key: Option<&str>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: Option<&str>,
    finished_at: Option<&str>,
    status: &str,
    phase: Option<&str>,
    http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<i64> {
    let result = sqlx::query(
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
            phase,
            http_status,
            failure_kind,
            error_message,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id,
            compact_support_status,
            compact_support_reason
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
        )
        "#,
    )
    .bind(&trace.invoke_id)
    .bind(&trace.occurred_at)
    .bind(&trace.endpoint)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(trace.sticky_key.as_deref())
    .bind(upstream_account_id)
    .bind(upstream_route_key)
    .bind(attempt_index)
    .bind(distinct_account_index)
    .bind(same_account_retry_index)
    .bind(trace.requester_ip.as_deref())
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(phase)
    .bind(http_status.map(|value| i64::from(value.as_u16())))
    .bind(failure_kind)
    .bind(error_message)
    .bind(connect_latency_ms)
    .bind(first_byte_latency_ms)
    .bind(stream_latency_ms)
    .bind(upstream_request_id)
    .bind(compact_support_status)
    .bind(compact_support_reason)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

async fn begin_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    let attempt_id = match insert_pool_upstream_request_attempt(
        pool,
        trace,
        Some(upstream_account_id),
        Some(upstream_route_key),
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        Some(started_at),
        None,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    {
        Ok(attempt_id) => Some(attempt_id),
        Err(err) => {
            warn!(
                invoke_id = trace.invoke_id,
                error = %err,
                "failed to persist pending pool attempt"
            );
            None
        }
    };

    PendingPoolAttemptRecord {
        attempt_id,
        invoke_id: trace.invoke_id.clone(),
        occurred_at: trace.occurred_at.clone(),
        endpoint: trace.endpoint.clone(),
        sticky_key: trace.sticky_key.clone(),
        requester_ip: trace.requester_ip.clone(),
        upstream_account_id,
        upstream_route_key: upstream_route_key.to_string(),
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        started_at: started_at.to_string(),
        connect_latency_ms: 0.0,
        first_byte_latency_ms: 0.0,
        compact_support_status: None,
        compact_support_reason: None,
    }
}

async fn update_pool_upstream_request_attempt_phase(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<bool> {
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET phase = ?2
        WHERE id = ?1
          AND COALESCE(phase, '') <> ?2
        "#,
    )
    .bind(attempt_id)
    .bind(phase)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

async fn advance_pool_upstream_request_attempt_phase(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<()> {
    if !update_pool_upstream_request_attempt_phase(&state.pool, pending, phase).await? {
        return Ok(());
    }

    broadcast_pool_upstream_attempts_snapshot(state, &pending.invoke_id).await
}

async fn recover_orphaned_pool_upstream_request_attempts(pool: &Pool<Sqlite>) -> Result<u64> {
    let finished_at = shanghai_now_string();
    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            finished_at = COALESCE(finished_at, ?1),
            status = ?2,
            phase = ?3,
            failure_kind = COALESCE(failure_kind, ?4),
            error_message = COALESCE(error_message, ?5)
        WHERE status = ?6
          AND finished_at IS NULL
        "#,
    )
    .bind(finished_at)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    .bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

async fn broadcast_pool_upstream_attempts_snapshot(
    state: &AppState,
    invoke_id: &str,
) -> Result<()> {
    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let attempts = query_pool_attempt_records_from_live(&state.pool, invoke_id)
        .await
        .map_err(|err| anyhow!("failed to load live pool attempts for SSE broadcast: {err:?}"))?;
    state
        .broadcaster
        .send(BroadcastPayload::PoolAttempts {
            invoke_id: invoke_id.to_string(),
            attempts,
        })
        .map_err(|err| anyhow!("failed to broadcast pool attempts snapshot: {err}"))?;
    Ok(())
}

async fn broadcast_pool_attempt_started_runtime_snapshot(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    runtime_snapshot: &PoolAttemptRuntimeSnapshotContext,
    account: &PoolResolvedAccount,
    attempt_count: usize,
    distinct_account_count: usize,
) {
    let running_record = build_running_proxy_capture_record(
        &trace.invoke_id,
        &trace.occurred_at,
        runtime_snapshot.capture_target,
        &runtime_snapshot.request_info,
        trace.requester_ip.as_deref(),
        trace.sticky_key.as_deref(),
        runtime_snapshot.prompt_cache_key.as_deref(),
        true,
        Some(account.account_id),
        Some(account.display_name.as_str()),
        None,
        Some(attempt_count),
        Some(distinct_account_count),
        None,
        None,
        runtime_snapshot.t_req_read_ms,
        runtime_snapshot.t_req_parse_ms,
        0.0,
        0.0,
    );
    if let Err(err) = broadcast_proxy_capture_runtime_snapshot(&state.broadcaster, &running_record)
    {
        warn!(
            ?err,
            invoke_id = %trace.invoke_id,
            "failed to broadcast pool attempt start runtime snapshot"
        );
    }
    if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await {
        warn!(
            invoke_id = %trace.invoke_id,
            error = %err,
            "failed to broadcast pool attempt start snapshot"
        );
    }
}

async fn finalize_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    finished_at: &str,
    status: &str,
    http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<()> {
    let terminal_phase = terminal_pool_upstream_request_attempt_phase(status);
    let compact_support_status =
        compact_support_status.or(pending.compact_support_status.as_deref());
    let compact_support_reason =
        compact_support_reason.or(pending.compact_support_reason.as_deref());
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: pending.invoke_id.clone(),
        occurred_at: pending.occurred_at.clone(),
        endpoint: pending.endpoint.clone(),
        sticky_key: pending.sticky_key.clone(),
        requester_ip: pending.requester_ip.clone(),
    };
    if let Some(attempt_id) = pending.attempt_id {
        let result = sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                finished_at = ?2,
                status = ?3,
                phase = ?4,
                http_status = ?5,
                failure_kind = ?6,
                error_message = ?7,
                connect_latency_ms = ?8,
                first_byte_latency_ms = ?9,
                stream_latency_ms = ?10,
                upstream_request_id = ?11,
                compact_support_status = ?12,
                compact_support_reason = ?13
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .bind(finished_at)
        .bind(status)
        .bind(terminal_phase)
        .bind(http_status.map(|value| i64::from(value.as_u16())))
        .bind(failure_kind)
        .bind(error_message)
        .bind(connect_latency_ms)
        .bind(first_byte_latency_ms)
        .bind(stream_latency_ms)
        .bind(upstream_request_id)
        .bind(compact_support_status)
        .bind(compact_support_reason)
        .execute(pool)
        .await?;

        if result.rows_affected() > 0 {
            return Ok(());
        }
    }

    insert_pool_upstream_request_attempt(
        pool,
        &trace,
        Some(pending.upstream_account_id),
        Some(pending.upstream_route_key.as_str()),
        pending.attempt_index,
        pending.distinct_account_index,
        pending.same_account_retry_index,
        Some(pending.started_at.as_str()),
        Some(finished_at),
        status,
        Some(terminal_phase),
        http_status,
        failure_kind,
        error_message,
        connect_latency_ms,
        first_byte_latency_ms,
        stream_latency_ms,
        upstream_request_id,
        compact_support_status,
        compact_support_reason,
    )
    .await
    .map(|_| ())
}

async fn insert_pool_upstream_terminal_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let finished_at = shanghai_now_string();
    let upstream_route_key = final_error
        .account
        .as_ref()
        .map(|account| account.upstream_route_key());
    insert_pool_upstream_request_attempt(
        pool,
        trace,
        final_error
            .account
            .as_ref()
            .map(|account| account.account_id),
        upstream_route_key.as_deref(),
        attempt_index,
        distinct_account_index,
        0,
        Some(finished_at.as_str()),
        Some(finished_at.as_str()),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED),
        Some(final_error.status),
        Some(failure_kind),
        Some(final_error.message.as_str()),
        None,
        None,
        None,
        final_error.upstream_request_id.as_deref(),
        None,
        None,
    )
    .await
    .map(|_| ())
}

async fn insert_and_broadcast_pool_upstream_terminal_attempt(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    insert_pool_upstream_terminal_attempt(
        &state.pool,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    )
    .await?;
    broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await?;
    Ok(())
}

fn prompt_cache_upstream_account_rollup_key(
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
) -> String {
    let normalized_name = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (upstream_account_id, normalized_name) {
        (Some(account_id), Some(account_name)) => format!("id:{account_id}|name:{account_name}"),
        (Some(account_id), None) => format!("id:{account_id}"),
        (None, Some(account_name)) => format!("name:{account_name}"),
        (None, None) => "unknown".to_string(),
    }
}

async fn load_hourly_rollup_live_progress(pool: &Pool<Sqlite>, dataset: &str) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0))
}

async fn load_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0))
}

async fn save_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    cursor_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = MAX(hourly_rollup_live_progress.cursor_id, excluded.cursor_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(dataset)
    .bind(cursor_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn mark_hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            replayed_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3 LIMIT 1",
        )
        .bind(target)
        .bind(dataset)
        .bind(file_path)
        .fetch_optional(&mut *tx)
        .await?
        .is_some(),
    )
}

fn normalized_oauth_account_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn looks_like_uuid_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        let is_hyphen = matches!(idx, 8 | 13 | 18 | 23);
        if is_hyphen {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn oauth_account_id_shape(value: Option<&str>) -> &'static str {
    match normalized_oauth_account_id(value) {
        None => "empty",
        Some(value) if value.starts_with("org_") => "org",
        Some(value) if looks_like_uuid_shape(value) => "uuid",
        Some(_) => "other",
    }
}

fn oauth_account_header_attached_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<bool> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(normalized_oauth_account_id(chatgpt_account_id.as_deref()).is_some())
}

fn oauth_account_id_shape_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<&'static str> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(oauth_account_id_shape(chatgpt_account_id.as_deref()))
}

struct ProxyPayloadSummary<'a> {
    target: ProxyCaptureTarget,
    status: StatusCode,
    is_stream: bool,
    request_model: Option<&'a str>,
    requested_service_tier: Option<&'a str>,
    reasoning_effort: Option<&'a str>,
    response_model: Option<&'a str>,
    usage_missing_reason: Option<&'a str>,
    request_parse_error: Option<&'a str>,
    failure_kind: Option<&'a str>,
    requester_ip: Option<&'a str>,
    upstream_scope: &'a str,
    route_mode: &'a str,
    sticky_key: Option<&'a str>,
    prompt_cache_key: Option<&'a str>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&'a str>,
    oauth_account_header_attached: Option<bool>,
    oauth_account_id_shape: Option<&'a str>,
    oauth_forwarded_header_count: Option<usize>,
    oauth_forwarded_header_names: Option<&'a [String]>,
    oauth_fingerprint_version: Option<&'a str>,
    oauth_forwarded_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    oauth_prompt_cache_header_forwarded: Option<bool>,
    oauth_request_body_prefix_fingerprint: Option<&'a str>,
    oauth_request_body_prefix_bytes: Option<usize>,
    oauth_responses_rewrite: Option<&'a oauth_bridge::OauthResponsesRewriteSummary>,
    service_tier: Option<&'a str>,
    stream_terminal_event: Option<&'a str>,
    upstream_error_code: Option<&'a str>,
    upstream_error_message: Option<&'a str>,
    upstream_request_id: Option<&'a str>,
    response_content_encoding: Option<&'a str>,
    proxy_display_name: Option<&'a str>,
    proxy_weight_delta: Option<f64>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&'a str>,
}

fn build_proxy_payload_summary(summary: ProxyPayloadSummary<'_>) -> String {
    let ProxyPayloadSummary {
        target,
        status,
        is_stream,
        request_model,
        requested_service_tier,
        reasoning_effort,
        response_model,
        usage_missing_reason,
        request_parse_error,
        failure_kind,
        requester_ip,
        upstream_scope,
        route_mode,
        sticky_key,
        prompt_cache_key,
        upstream_account_id,
        upstream_account_name,
        oauth_account_header_attached,
        oauth_account_id_shape,
        oauth_forwarded_header_count,
        oauth_forwarded_header_names,
        oauth_fingerprint_version,
        oauth_forwarded_header_fingerprints,
        oauth_prompt_cache_header_forwarded,
        oauth_request_body_prefix_fingerprint,
        oauth_request_body_prefix_bytes,
        oauth_responses_rewrite,
        service_tier,
        stream_terminal_event,
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
        response_content_encoding,
        proxy_display_name,
        proxy_weight_delta,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
    } = summary;
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
        "oauthAccountHeaderAttached": oauth_account_header_attached,
        "oauthAccountIdShape": oauth_account_id_shape,
        "oauthForwardedHeaderCount": oauth_forwarded_header_count,
        "oauthForwardedHeaderNames": oauth_forwarded_header_names,
        "oauthFingerprintVersion": oauth_fingerprint_version,
        "oauthForwardedHeaderFingerprints": oauth_forwarded_header_fingerprints,
        "oauthPromptCacheHeaderForwarded": oauth_prompt_cache_header_forwarded,
        "oauthRequestBodyPrefixFingerprint": oauth_request_body_prefix_fingerprint,
        "oauthRequestBodyPrefixBytes": oauth_request_body_prefix_bytes,
        "oauthResponsesRewrite": oauth_responses_rewrite,
        "serviceTier": service_tier,
        "streamTerminalEvent": stream_terminal_event,
        "upstreamErrorCode": upstream_error_code,
        "upstreamErrorMessage": upstream_error_message,
        "upstreamRequestId": upstream_request_id,
        "responseContentEncoding": response_content_encoding,
        "proxyDisplayName": proxy_display_name,
        "proxyWeightDelta": proxy_weight_delta,
        "poolAttemptCount": pool_attempt_count,
        "poolDistinctAccountCount": pool_distinct_account_count,
        "poolAttemptTerminalReason": pool_attempt_terminal_reason,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn temporary_invocation_id(invoke_id: &str, occurred_at: &str) -> i64 {
    let mut hasher = DefaultHasher::new();
    invoke_id.hash(&mut hasher);
    occurred_at.hash(&mut hasher);
    -((hasher.finish() & (i64::MAX as u64)) as i64) - 1
}

fn runtime_invocation_iso_from_local_occurred_at(value: &str) -> String {
    parse_shanghai_local_naive(value)
        .map(|naive| format_utc_iso(local_naive_to_utc(naive, Shanghai)))
        .unwrap_or_else(|_| value.to_string())
}

fn runtime_payload_text(payload: Option<&Value>, key: &str) -> Option<String> {
    payload
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn runtime_payload_i64(payload: Option<&Value>, key: &str) -> Option<i64> {
    payload
        .and_then(|value| value.get(key))
        .and_then(json_value_to_i64)
}

fn runtime_payload_f64(payload: Option<&Value>, key: &str) -> Option<f64> {
    payload
        .and_then(|value| value.get(key))
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str()?.parse::<f64>().ok())
        })
        .filter(|value| value.is_finite())
}

fn runtime_timing_value(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn runtime_api_invocation_from_proxy_capture_record(record: &ProxyCaptureRecord) -> ApiInvocation {
    let payload = record
        .payload
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    let failure = classify_invocation_failure(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
    );
    let occurred_at = runtime_invocation_iso_from_local_occurred_at(&record.occurred_at);
    let created_at = occurred_at.clone();
    let is_running = matches!(
        record.status.trim().to_ascii_lowercase().as_str(),
        "running" | "pending"
    );

    ApiInvocation {
        id: temporary_invocation_id(&record.invoke_id, &record.occurred_at),
        invoke_id: record.invoke_id.clone(),
        occurred_at,
        source: SOURCE_PROXY.to_string(),
        proxy_display_name: runtime_payload_text(payload.as_ref(), "proxyDisplayName"),
        model: record.model.clone(),
        input_tokens: record.usage.input_tokens,
        output_tokens: record.usage.output_tokens,
        cache_input_tokens: record.usage.cache_input_tokens,
        reasoning_tokens: record.usage.reasoning_tokens,
        reasoning_effort: runtime_payload_text(payload.as_ref(), "reasoningEffort"),
        total_tokens: record.usage.total_tokens,
        cost: record.cost,
        status: Some(record.status.clone()),
        error_message: record.error_message.clone(),
        failure_kind: record
            .failure_kind
            .clone()
            .or_else(|| runtime_payload_text(payload.as_ref(), "failureKind")),
        stream_terminal_event: runtime_payload_text(payload.as_ref(), "streamTerminalEvent"),
        upstream_error_code: runtime_payload_text(payload.as_ref(), "upstreamErrorCode"),
        upstream_error_message: runtime_payload_text(payload.as_ref(), "upstreamErrorMessage"),
        upstream_request_id: runtime_payload_text(payload.as_ref(), "upstreamRequestId"),
        failure_class: Some(failure.failure_class.as_str().to_string()),
        is_actionable: Some(failure.is_actionable),
        endpoint: runtime_payload_text(payload.as_ref(), "endpoint"),
        requester_ip: runtime_payload_text(payload.as_ref(), "requesterIp"),
        prompt_cache_key: runtime_payload_text(payload.as_ref(), "promptCacheKey"),
        route_mode: runtime_payload_text(payload.as_ref(), "routeMode"),
        upstream_account_id: runtime_payload_i64(payload.as_ref(), "upstreamAccountId"),
        upstream_account_name: runtime_payload_text(payload.as_ref(), "upstreamAccountName"),
        response_content_encoding: runtime_payload_text(
            payload.as_ref(),
            "responseContentEncoding",
        ),
        pool_attempt_count: runtime_payload_i64(payload.as_ref(), "poolAttemptCount"),
        pool_distinct_account_count: runtime_payload_i64(
            payload.as_ref(),
            "poolDistinctAccountCount",
        ),
        pool_attempt_terminal_reason: runtime_payload_text(
            payload.as_ref(),
            "poolAttemptTerminalReason",
        ),
        requested_service_tier: runtime_payload_text(payload.as_ref(), "requestedServiceTier"),
        service_tier: runtime_payload_text(payload.as_ref(), "serviceTier"),
        proxy_weight_delta: runtime_payload_f64(payload.as_ref(), "proxyWeightDelta"),
        cost_estimated: Some(i64::from(record.cost_estimated)),
        price_version: record.price_version.clone(),
        request_raw_path: None,
        request_raw_size: None,
        request_raw_truncated: None,
        request_raw_truncated_reason: None,
        response_raw_path: None,
        response_raw_size: None,
        response_raw_truncated: None,
        response_raw_truncated_reason: None,
        detail_level: "full".to_string(),
        detail_pruned_at: None,
        detail_prune_reason: None,
        t_total_ms: if is_running {
            None
        } else {
            runtime_timing_value(record.timings.t_total_ms)
        },
        t_req_read_ms: runtime_timing_value(record.timings.t_req_read_ms),
        t_req_parse_ms: runtime_timing_value(record.timings.t_req_parse_ms),
        t_upstream_connect_ms: runtime_timing_value(record.timings.t_upstream_connect_ms),
        t_upstream_ttfb_ms: runtime_timing_value(record.timings.t_upstream_ttfb_ms),
        t_upstream_stream_ms: if is_running {
            None
        } else {
            runtime_timing_value(record.timings.t_upstream_stream_ms)
        },
        t_resp_parse_ms: if is_running {
            None
        } else {
            runtime_timing_value(record.timings.t_resp_parse_ms)
        },
        t_persist_ms: if is_running {
            None
        } else {
            runtime_timing_value(record.timings.t_persist_ms)
        },
        created_at,
    }
}

fn broadcast_proxy_capture_runtime_snapshot(
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    record: &ProxyCaptureRecord,
) -> Result<bool, broadcast::error::SendError<BroadcastPayload>> {
    if broadcaster.receiver_count() == 0 {
        return Ok(false);
    }

    broadcaster.send(BroadcastPayload::Records {
        records: vec![runtime_api_invocation_from_proxy_capture_record(record)],
    })?;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn build_running_proxy_capture_record(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    pool_route_active: bool,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
    proxy_display_name: Option<&str>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&str>,
    response_content_encoding: Option<&str>,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
    t_upstream_connect_ms: f64,
    t_upstream_ttfb_ms: f64,
) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: request_info.model.clone(),
        usage: ParsedUsage::default(),
        cost: None,
        cost_estimated: false,
        price_version: None,
        status: "running".to_string(),
        error_message: None,
        failure_kind: None,
        payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
            target,
            status: StatusCode::OK,
            is_stream: request_info.is_stream,
            request_model: request_info.model.as_deref(),
            requested_service_tier: request_info.requested_service_tier.as_deref(),
            reasoning_effort: request_info.reasoning_effort.as_deref(),
            response_model: None,
            usage_missing_reason: None,
            request_parse_error: request_info.parse_error.as_deref(),
            failure_kind: None,
            requester_ip,
            upstream_scope: if pool_route_active {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_route_active {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key,
            prompt_cache_key,
            upstream_account_id,
            upstream_account_name,
            oauth_account_header_attached: None,
            oauth_account_id_shape: None,
            oauth_forwarded_header_count: None,
            oauth_forwarded_header_names: None,
            oauth_fingerprint_version: None,
            oauth_forwarded_header_fingerprints: None,
            oauth_prompt_cache_header_forwarded: None,
            oauth_request_body_prefix_fingerprint: None,
            oauth_request_body_prefix_bytes: None,
            oauth_responses_rewrite: None,
            service_tier: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
            response_content_encoding,
            proxy_display_name,
            proxy_weight_delta: None,
            pool_attempt_count,
            pool_distinct_account_count,
            pool_attempt_terminal_reason,
        })),
        raw_response: "{}".to_string(),
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: StageTimings {
            t_total_ms: 0.0,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        },
    }
}

fn resolve_invocation_proxy_display_name(
    selected_proxy: Option<&SelectedForwardProxy>,
) -> Option<String> {
    selected_proxy.map(|proxy| proxy.display_name.clone())
}

fn summarize_response_content_encoding(content_encoding: Option<&str>) -> String {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        "identity".to_string()
    } else {
        encodings.join(", ")
    }
}

#[derive(Default)]
struct RawResponsePreviewBuffer {
    bytes: Vec<u8>,
}

impl RawResponsePreviewBuffer {
    fn append(&mut self, chunk: &[u8]) {
        let remaining = RAW_RESPONSE_PREVIEW_LIMIT.saturating_sub(self.bytes.len());
        if remaining == 0 || chunk.is_empty() {
            return;
        }
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn into_preview(self) -> String {
        build_raw_response_preview(&self.bytes)
    }
}

struct BoundedResponseParseBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded_limit: bool,
}

impl BoundedResponseParseBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded_limit: false,
        }
    }

    fn append(&mut self, chunk: &[u8]) {
        if self.exceeded_limit || chunk.is_empty() {
            return;
        }

        let remaining = self.limit.saturating_sub(self.bytes.len());
        let take_len = remaining.min(chunk.len());
        if take_len > 0 {
            self.bytes.extend_from_slice(&chunk[..take_len]);
        }
        if take_len < chunk.len() {
            self.exceeded_limit = true;
        }
    }

    fn into_response_info(
        self,
        target: ProxyCaptureTarget,
        content_encoding: Option<&str>,
    ) -> ResponseCaptureInfo {
        let mut response_info =
            parse_target_response_payload(target, &self.bytes, false, content_encoding);
        if self.exceeded_limit {
            merge_response_capture_reason(
                &mut response_info,
                PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
            );
        }
        response_info
    }
}

struct StreamingRawPayloadWriter {
    path: PathBuf,
    max_bytes: Option<usize>,
    written_bytes: usize,
    meta: RawPayloadMeta,
    file: Option<tokio::fs::File>,
}

impl StreamingRawPayloadWriter {
    fn new(config: &AppConfig, invoke_id: &str, kind: &str) -> Self {
        let path = config
            .resolved_proxy_raw_dir()
            .join(format!("{invoke_id}-{kind}.bin"));
        Self {
            path,
            max_bytes: config.proxy_raw_max_bytes,
            written_bytes: 0,
            meta: RawPayloadMeta::default(),
            file: None,
        }
    }

    async fn ensure_file(&mut self) -> io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        let Some(parent) = self.path.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("raw payload path has no parent: {}", self.path.display()),
            ));
        };
        tokio::fs::create_dir_all(parent).await?;
        let file = tokio::fs::File::create(&self.path).await?;
        self.meta.path = Some(self.path.to_string_lossy().to_string());
        self.file = Some(file);
        Ok(())
    }

    fn mark_max_bytes_exceeded(&mut self) {
        self.meta.truncated = true;
        self.meta
            .truncated_reason
            .get_or_insert_with(|| "max_bytes_exceeded".to_string());
    }

    async fn record_write_failure(&mut self, err: io::Error) {
        self.meta.truncated = true;
        self.meta.truncated_reason = Some(format!("write_failed:{err}"));
        self.file = None;
        if self.meta.path.is_some() {
            let _ = tokio::fs::remove_file(&self.path).await;
            self.meta.path = None;
        }
    }

    async fn append(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        self.meta.size_bytes = self.meta.size_bytes.saturating_add(bytes.len() as i64);

        if self
            .meta
            .truncated_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("write_failed:"))
        {
            return;
        }

        if let Err(err) = self.ensure_file().await {
            self.record_write_failure(err).await;
            return;
        }

        let write_len = if let Some(limit) = self.max_bytes {
            let remaining = limit.saturating_sub(self.written_bytes);
            if remaining == 0 {
                self.mark_max_bytes_exceeded();
                return;
            }
            let write_len = remaining.min(bytes.len());
            if write_len < bytes.len() {
                self.mark_max_bytes_exceeded();
            }
            write_len
        } else {
            bytes.len()
        };

        if write_len == 0 {
            return;
        }

        if let Some(file) = self.file.as_mut() {
            if let Err(err) = file.write_all(&bytes[..write_len]).await {
                self.record_write_failure(err).await;
                return;
            }
            self.written_bytes = self.written_bytes.saturating_add(write_len);
        }
    }

    async fn finish(mut self) -> RawPayloadMeta {
        if let Some(file) = self.file.as_mut()
            && let Err(err) = file.flush().await
        {
            self.record_write_failure(err).await;
        }
        self.meta
    }
}

fn build_raw_response_preview(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "{}".to_string();
    }
    let preview = if bytes.len() > RAW_RESPONSE_PREVIEW_LIMIT {
        &bytes[..RAW_RESPONSE_PREVIEW_LIMIT]
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

fn extract_error_message_from_response_preview(bytes: &[u8]) -> Option<String> {
    extract_error_message_from_response(bytes).or_else(|| summarize_plaintext_upstream_error(bytes))
}

fn merge_response_capture_reason(
    response_info: &mut ResponseCaptureInfo,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
        format!("{reason};{existing}")
    } else {
        reason
    };
    response_info.usage_missing_reason = Some(combined_reason);
}

fn deflate_stream_uses_zlib_wrapper(header: &[u8]) -> bool {
    if header.len() < 2 {
        return true;
    }

    let cmf = header[0];
    let flg = header[1];
    let method = cmf & 0x0f;
    let window_bits = cmf >> 4;
    let header_word = (u16::from(cmf) << 8) | u16::from(flg);
    method == 8 && window_bits <= 7 && header_word % 31 == 0
}

#[allow(dead_code)]
fn wrap_decoded_response_reader(
    mut reader: Box<dyn Read + Send>,
    content_encoding: Option<&str>,
) -> std::result::Result<Box<dyn Read + Send>, String> {
    let encodings = parse_content_encodings(content_encoding);
    for encoding in encodings.iter().rev() {
        reader = match encoding.as_str() {
            "identity" => reader,
            "gzip" | "x-gzip" => Box::new(GzDecoder::new(reader)),
            "br" => Box::new(BrotliDecompressor::new(reader, 4096)),
            "deflate" => {
                let mut buffered = io::BufReader::new(reader);
                let header = buffered.fill_buf().map_err(|err| err.to_string())?;
                if deflate_stream_uses_zlib_wrapper(header) {
                    Box::new(ZlibDecoder::new(buffered))
                } else {
                    Box::new(DeflateDecoder::new(buffered))
                }
            }
            other => return Err(format!("unsupported_content_encoding:{other}")),
        };
    }
    Ok(reader)
}

#[allow(dead_code)]
fn open_decoded_response_reader(
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<Box<dyn Read + Send>, String> {
    let file = fs::File::open(path).map_err(|err| err.to_string())?;
    wrap_decoded_response_reader(Box::new(file), content_encoding)
}

#[allow(dead_code)]
fn parse_nonstream_response_payload_from_raw_file(
    target: ProxyCaptureTarget,
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<ResponseCaptureInfo, String> {
    let mut reader = open_decoded_response_reader(path, content_encoding)?;
    let mut decoded = Vec::new();
    reader
        .by_ref()
        .take((BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    if decoded.len() > BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES {
        decoded.truncate(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES);
        let mut response_info = parse_target_response_payload(target, &decoded, false, None);
        merge_response_capture_reason(
            &mut response_info,
            PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
        );
        return Ok(response_info);
    }
    Ok(parse_target_response_payload(target, &decoded, false, None))
}

#[allow(dead_code)]
fn parse_target_response_payload_from_raw_file(
    target: ProxyCaptureTarget,
    path: &Path,
    is_stream_hint: bool,
    content_encoding: Option<&str>,
) -> std::result::Result<ResponseCaptureInfo, String> {
    if is_stream_hint {
        let reader = open_decoded_response_reader(path, content_encoding)?;
        parse_stream_response_payload_from_reader(reader).map_err(|err| err.to_string())
    } else {
        parse_nonstream_response_payload_from_raw_file(target, path, content_encoding)
    }
}

#[allow(dead_code)]
fn parse_target_response_payload_from_capture(
    target: ProxyCaptureTarget,
    resp_raw: &RawPayloadMeta,
    preview_bytes: &[u8],
    is_stream_hint: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    #[cfg(test)]
    RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.fetch_add(1, Ordering::Relaxed);

    if let Some(path) = resp_raw.path.as_deref() {
        let path = PathBuf::from(path);
        match parse_target_response_payload_from_raw_file(
            target,
            &path,
            is_stream_hint,
            content_encoding,
        ) {
            Ok(response_info) => response_info,
            Err(reason) => {
                let mut response_info = parse_target_response_payload(
                    target,
                    preview_bytes,
                    is_stream_hint,
                    content_encoding,
                );
                merge_response_capture_reason(&mut response_info, reason);
                response_info
            }
        }
    } else {
        parse_target_response_payload(target, preview_bytes, is_stream_hint, content_encoding)
    }
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
    invocation_max_days: u64,
    invoke_id: &str,
) {
    if broadcaster.receiver_count() == 0 {
        return;
    }

    match collect_summary_snapshots(pool, relay_config, invocation_max_days).await {
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
    invocation_max_days: u64,
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
            ctx.invocation_max_days,
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
    if inserted_record
        .prompt_cache_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }
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
            state.config.invocation_max_days,
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
            state.config.invocation_max_days,
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
    let invocation_max_days = state.config.invocation_max_days;
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
                        invocation_max_days,
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
                        invocation_max_days,
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
                        invocation_max_days,
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
                result = collect_summary_snapshots(&pool, relay_config.as_ref(), invocation_max_days) => result,
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
                        invocation_max_days,
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
    let mut tx = pool.begin().await?;
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
    .execute(tx.as_mut())
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
    .execute(tx.as_mut())
    .await?;

    if insert_result.rows_affected() == 0 {
        tx.commit().await?;
        return Ok(None);
    }

    let inserted_id = insert_result.last_insert_rowid();

    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[InvocationHourlySourceRecord {
            id: inserted_id,
            occurred_at: record.occurred_at.clone(),
            source: SOURCE_PROXY.to_string(),
            status: Some(record.status.clone()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            total_tokens: record.usage.total_tokens,
            cost: record.cost,
            error_message: record.error_message.clone(),
            failure_kind: failure_kind.map(ToOwned::to_owned),
            failure_class: Some(failure.failure_class.as_str().to_string()),
            is_actionable: Some(failure.is_actionable as i64),
            payload: record.payload.clone(),
            t_total_ms: Some(record.timings.t_total_ms),
            t_req_read_ms: Some(record.timings.t_req_read_ms),
            t_req_parse_ms: Some(record.timings.t_req_parse_ms),
            t_upstream_connect_ms: Some(record.timings.t_upstream_connect_ms),
            t_upstream_ttfb_ms: Some(record.timings.t_upstream_ttfb_ms),
            t_upstream_stream_ms: Some(record.timings.t_upstream_stream_ms),
            t_resp_parse_ms: Some(record.timings.t_resp_parse_ms),
            t_persist_ms: Some(record.timings.t_persist_ms),
        }],
        &INVOCATION_HOURLY_ROLLUP_TARGETS,
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        inserted_id,
    )
    .await?;

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
        .execute(tx.as_mut())
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
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.routeMode') END AS route_mode,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END AS upstream_account_id,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountName') END AS upstream_account_name,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.responseContentEncoding') END AS response_content_encoding,
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
    .fetch_one(tx.as_mut())
    .await?;

    tx.commit().await?;

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
            let mut updated_ids = Vec::new();
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
                if affected > 0 {
                    updated_ids.push(update.id);
                }
            }
            if !updated_ids.is_empty() {
                recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
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
            let mut updated_ids = Vec::new();
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
                if affected > 0 {
                    updated_ids.push(update.id);
                }
            }
            if !updated_ids.is_empty() {
                recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
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

        let mut updates = Vec::new();
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
            updates.push((candidate.id, prompt_cache_key));
        }

        if !updates.is_empty() {
            let mut tx = pool.begin().await?;
            let mut updated_ids = Vec::new();
            for (id, prompt_cache_key) in updates {
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
                .bind(id)
                .bind(SOURCE_PROXY)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                summary.updated += affected;
                if affected > 0 {
                    updated_ids.push(id);
                }
            }
            if !updated_ids.is_empty() {
                recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
            }
            tx.commit().await?;
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

    invocation_status_is_success_like(row.status.as_deref(), row.error_message.as_deref())
        || existing_kind.is_none()
        || existing_kind == Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
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
        let mut updated_ids = Vec::new();
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
                if affected > 0 {
                    updated_ids.push(row.id);
                }
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
            if affected > 0 {
                updated_ids.push(row.id);
            }
        }
        if !updated_ids.is_empty() {
            recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
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

#[derive(Debug, Clone)]
struct PoolRoutingReservation {
    account_id: i64,
    proxy_key: Option<String>,
    #[allow(dead_code)]
    created_at: Instant,
}

#[derive(Debug, Default, Clone)]
struct PoolRoutingReservationSnapshot {
    counts_by_account: HashMap<i64, i64>,
    proxy_keys_by_account: HashMap<i64, HashSet<String>>,
    reserved_proxy_keys: HashSet<String>,
}

impl PoolRoutingReservationSnapshot {
    fn count_for_account(&self, account_id: i64) -> i64 {
        self.counts_by_account
            .get(&account_id)
            .copied()
            .unwrap_or_default()
    }

    fn pinned_proxy_keys_for_account(
        &self,
        account_id: i64,
        valid_proxy_keys: &[String],
        occupied_proxy_keys: &HashSet<String>,
    ) -> Vec<String> {
        let Some(proxy_keys) = self.proxy_keys_by_account.get(&account_id) else {
            return Vec::new();
        };
        valid_proxy_keys
            .iter()
            .filter(|proxy_key| {
                proxy_keys.contains(proxy_key.as_str())
                    && !occupied_proxy_keys.contains(proxy_key.as_str())
            })
            .cloned()
            .collect()
    }

    fn reserved_proxy_keys_for_group(&self, valid_proxy_keys: &[String]) -> HashSet<String> {
        let valid_proxy_keys = valid_proxy_keys
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        self.reserved_proxy_keys
            .iter()
            .filter(|proxy_key| valid_proxy_keys.contains(proxy_key.as_str()))
            .cloned()
            .collect()
    }
}

#[derive(Debug)]
struct PoolRoutingReservationDropGuard {
    state: Arc<AppState>,
    reservation_key: String,
    active: bool,
}

impl PoolRoutingReservationDropGuard {
    fn new(state: Arc<AppState>, reservation_key: String) -> Self {
        Self {
            state,
            reservation_key,
            active: true,
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PoolRoutingReservationDropGuard {
    fn drop(&mut self) {
        if self.active {
            release_pool_routing_reservation(self.state.as_ref(), &self.reservation_key);
        }
    }
}

fn build_pool_routing_reservation_key(proxy_request_id: u64) -> String {
    format!("pool-route-{proxy_request_id}")
}

fn pool_routing_reservation_count(state: &AppState, account_id: i64) -> i64 {
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations
        .values()
        .filter(|reservation| reservation.account_id == account_id)
        .count() as i64
}

fn pool_routing_reservation_snapshot(state: &AppState) -> PoolRoutingReservationSnapshot {
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    let mut snapshot = PoolRoutingReservationSnapshot::default();
    for reservation in reservations.values() {
        *snapshot
            .counts_by_account
            .entry(reservation.account_id)
            .or_default() += 1;
        if let Some(proxy_key) = reservation.proxy_key.as_deref() {
            snapshot.reserved_proxy_keys.insert(proxy_key.to_string());
            snapshot
                .proxy_keys_by_account
                .entry(reservation.account_id)
                .or_default()
                .insert(proxy_key.to_string());
        }
    }
    snapshot
}

fn reserve_pool_routing_account(
    state: &AppState,
    reservation_key: &str,
    account: &PoolResolvedAccount,
) {
    let proxy_key = match &account.forward_proxy_scope {
        ForwardProxyRouteScope::PinnedProxyKey(proxy_key) => Some(proxy_key.clone()),
        _ => None,
    };
    if account.routing_source == PoolRoutingSelectionSource::StickyReuse && proxy_key.is_none() {
        return;
    }
    let mut reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations.insert(
        reservation_key.to_string(),
        PoolRoutingReservation {
            account_id: account.account_id,
            proxy_key,
            created_at: Instant::now(),
        },
    );
}

fn release_pool_routing_reservation(state: &AppState, reservation_key: &str) {
    let mut reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations.remove(reservation_key);
}

fn consume_pool_routing_reservation(state: &AppState, reservation_key: &str) {
    release_pool_routing_reservation(state, reservation_key);
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

pub(crate) fn connection_scoped_header_names(headers: &HeaderMap) -> HashSet<HeaderName> {
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

pub(crate) fn should_forward_proxy_header(
    name: &HeaderName,
    connection_scoped: &HashSet<HeaderName>,
) -> bool {
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
    hourly_rollup_sync_lock: Arc<Mutex<()>>,
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
    maintenance_stats_cache: Arc<Mutex<StatsMaintenanceCacheState>>,
    pool_routing_reservations: Arc<std::sync::Mutex<HashMap<String, PoolRoutingReservation>>>,
    pool_group_429_retry_delay_override: Option<Duration>,
    pool_no_available_wait: PoolNoAvailableWaitSettings,
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
    upstream_429_max_retries: u8,
    enabled_preset_models: Vec<String>,
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
    upstream_429_max_retries: Option<i64>,
    enabled_preset_models_json: Option<String>,
}

impl From<ProxyModelSettingsRow> for ProxyModelSettings {
    fn from(value: ProxyModelSettingsRow) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled != 0,
            merge_upstream_enabled: value.merge_upstream_enabled != 0,
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
    upstream_429_max_retries: Option<u8>,
    #[serde(default = "default_enabled_preset_models")]
    enabled_models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyModelSettingsResponse {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
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

fn default_forward_proxy_insert_direct_compat() -> bool {
    true
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

fn normalize_single_proxy_key(raw: &str) -> Option<String> {
    parse_forward_proxy_entry(raw).map(|entry| entry.stable_key)
}

fn stable_forward_proxy_binding_key(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let mut stable = String::from("fpb_");
    for byte in digest.iter().take(16) {
        stable.push_str(&format!("{byte:02x}"));
    }
    stable
}

fn is_stable_forward_proxy_key(raw: &str) -> bool {
    raw.strip_prefix("fpn_").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn is_stable_forward_proxy_binding_key(raw: &str) -> bool {
    raw.strip_prefix("fpb_").is_some_and(|suffix| {
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn normalize_bound_proxy_key(raw: &str) -> Option<String> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return None;
    }
    if normalized == FORWARD_PROXY_DIRECT_KEY
        || is_stable_forward_proxy_key(normalized)
        || is_stable_forward_proxy_binding_key(normalized)
    {
        return Some(normalized.to_string());
    }
    normalize_single_proxy_key(normalized)
}

pub(crate) fn legacy_bound_proxy_key_aliases(
    raw: &str,
    protocol: ForwardProxyProtocol,
) -> Vec<String> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return Vec::new();
    }

    let scheme = match protocol {
        ForwardProxyProtocol::Vless => Some("vless"),
        ForwardProxyProtocol::Trojan => Some("trojan"),
        _ => None,
    };
    let Some(scheme) = scheme else {
        return Vec::new();
    };

    let Some(parsed) = Url::parse(normalized).ok() else {
        return Vec::new();
    };
    if !parsed.scheme().eq_ignore_ascii_case(scheme) {
        return Vec::new();
    }

    let default_specs = match protocol {
        ForwardProxyProtocol::Vless => &[
            LegacyDefaultQueryParamSpec {
                keys: &["encryption"],
                explicit_keys: &["encryption"],
                default_value: Some("none"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["security"],
                explicit_keys: &["security"],
                default_value: Some("none"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["type", "net"],
                explicit_keys: &["type", "net"],
                default_value: Some("tcp"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["sni", "serverName"],
                explicit_keys: &["sni", "serverName"],
                default_value: None,
            },
            LegacyDefaultQueryParamSpec {
                keys: &["fp", "fingerprint"],
                explicit_keys: &["fp", "fingerprint"],
                default_value: None,
            },
            LegacyDefaultQueryParamSpec {
                keys: &["serviceName", "service_name"],
                explicit_keys: &["serviceName", "service_name"],
                default_value: None,
            },
        ][..],
        ForwardProxyProtocol::Trojan => &[
            LegacyDefaultQueryParamSpec {
                keys: &["security"],
                explicit_keys: &["security"],
                default_value: Some("tls"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["type", "net"],
                explicit_keys: &["type", "net"],
                default_value: Some("tcp"),
            },
            LegacyDefaultQueryParamSpec {
                keys: &["sni", "serverName"],
                explicit_keys: &["sni", "serverName"],
                default_value: None,
            },
            LegacyDefaultQueryParamSpec {
                keys: &["fp", "fingerprint"],
                explicit_keys: &["fp", "fingerprint"],
                default_value: None,
            },
            LegacyDefaultQueryParamSpec {
                keys: &["serviceName", "service_name"],
                explicit_keys: &["serviceName", "service_name"],
                default_value: None,
            },
        ][..],
        _ => &[][..],
    };

    let mut aliases = legacy_share_link_identity_variants(&parsed, default_specs)
        .into_iter()
        .map(|identity| stable_forward_proxy_key(&identity))
        .collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

pub(crate) fn forward_proxy_storage_aliases(raw: &str) -> Option<(String, Vec<String>)> {
    let parsed = parse_forward_proxy_entry(raw)?;
    let canonical = parsed.stable_key.clone();
    let mut aliases = Vec::new();
    if parsed.normalized != canonical {
        aliases.push(parsed.normalized.clone());
    }
    if matches!(
        parsed.protocol,
        ForwardProxyProtocol::Vless | ForwardProxyProtocol::Trojan
    ) {
        aliases.extend(legacy_bound_proxy_key_aliases(
            &parsed.normalized,
            parsed.protocol,
        ));
    }
    aliases.retain(|alias| alias != &canonical);
    aliases.sort();
    aliases.dedup();
    Some((canonical, aliases))
}

fn normalize_proxy_endpoints_from_urls(urls: &[String], source: &str) -> Vec<ForwardProxyEndpoint> {
    let mut seen = HashSet::new();
    let mut endpoints = Vec::new();
    for raw in urls {
        if let Some(parsed) = parse_forward_proxy_entry(raw) {
            let key = parsed.stable_key.clone();
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
    stable_key: String,
    display_name: String,
    protocol: ForwardProxyProtocol,
    host: String,
    port: u16,
    endpoint_url: Option<Url>,
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyBindingParts {
    pub(crate) display_name: String,
    pub(crate) protocol_key: String,
    pub(crate) host_port: String,
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
        stable_key: stable_forward_proxy_key(&normalized),
        normalized,
        display_name: format!("{host}:{port}"),
        protocol,
        host: host.to_ascii_lowercase(),
        port,
        endpoint_url: Some(endpoint_url),
    })
}

fn parse_vmess_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "vmess")?;
    let parsed = parse_vmess_share_link(&normalized).ok()?;
    Some(ParsedForwardProxyEntry {
        stable_key: stable_forward_proxy_key(&parsed.stable_identity()),
        normalized,
        display_name: parsed.display_name,
        protocol: ForwardProxyProtocol::Vmess,
        host: parsed.address.to_ascii_lowercase(),
        port: parsed.port,
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
        stable_key: stable_forward_proxy_key(&canonical_vless_share_link_identity(&parsed)),
        normalized,
        display_name,
        protocol: ForwardProxyProtocol::Vless,
        host: host.to_ascii_lowercase(),
        port,
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
        stable_key: stable_forward_proxy_key(&canonical_trojan_share_link_identity(&parsed)),
        normalized,
        display_name,
        protocol: ForwardProxyProtocol::Trojan,
        host: host.to_ascii_lowercase(),
        port,
        endpoint_url: None,
    })
}

fn parse_shadowsocks_forward_proxy(candidate: &str) -> Option<ParsedForwardProxyEntry> {
    let normalized = normalize_share_link_scheme(candidate, "ss")?;
    let parsed = parse_shadowsocks_share_link(&normalized).ok()?;
    Some(ParsedForwardProxyEntry {
        stable_key: Url::parse(&normalized)
            .ok()
            .map(|url| stable_forward_proxy_key(&canonical_share_link_identity(&url)))
            .unwrap_or_else(|| stable_forward_proxy_key(&parsed.stable_identity())),
        normalized,
        display_name: parsed.display_name,
        protocol: ForwardProxyProtocol::Shadowsocks,
        host: parsed.host.to_ascii_lowercase(),
        port: parsed.port,
        endpoint_url: None,
    })
}

fn canonical_host_port_string(host: &str, port: u16) -> String {
    let normalized_host = host.trim().to_ascii_lowercase();
    if normalized_host.contains(':') {
        format!("[{normalized_host}]:{port}")
    } else {
        format!("{normalized_host}:{port}")
    }
}

pub(crate) fn forward_proxy_binding_parts_from_raw(
    raw: &str,
    display_name_override: Option<&str>,
) -> Option<ForwardProxyBindingParts> {
    let parsed = parse_forward_proxy_entry(raw)?;
    let display_name = display_name_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(parsed.display_name.as_str())
        .trim()
        .to_string();
    if display_name.is_empty() {
        return None;
    }
    Some(ForwardProxyBindingParts {
        display_name,
        protocol_key: parsed.protocol.label().to_string(),
        host_port: canonical_host_port_string(&parsed.host, parsed.port),
    })
}

pub(crate) fn forward_proxy_binding_key_candidates(
    parts: &ForwardProxyBindingParts,
) -> [String; 3] {
    [
        stable_forward_proxy_binding_key(&format!("name:{}", parts.display_name)),
        stable_forward_proxy_binding_key(&format!(
            "name:{}|protocol:{}",
            parts.display_name, parts.protocol_key
        )),
        stable_forward_proxy_binding_key(&format!(
            "name:{}|protocol:{}|server:{}",
            parts.display_name, parts.protocol_key, parts.host_port
        )),
    ]
}

fn proxy_display_name_from_url(url: &Url) -> Option<String> {
    if let Some(fragment) = url.fragment()
        && !fragment.trim().is_empty()
    {
        return Some(percent_decode_once_lossy(fragment));
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

fn stable_forward_proxy_key(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let mut stable = String::from("fpn_");
    for byte in digest.iter().take(16) {
        stable.push_str(&format!("{byte:02x}"));
    }
    stable
}

fn push_canonical_host_port(identity: &mut String, host: &str, port: u16) {
    if host.contains(':') {
        identity.push('[');
        identity.push_str(host);
        identity.push(']');
    } else {
        identity.push_str(host);
    }
    identity.push(':');
    identity.push_str(&port.to_string());
}

fn normalized_query_value(query: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| query.get(*key))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalized_query_ascii_lowercase(
    query: &HashMap<String, String>,
    keys: &[&str],
) -> Option<String> {
    normalized_query_value(query, keys).map(|value| value.to_ascii_lowercase())
}

fn sorted_query_pairs(url: &Url) -> Vec<(String, String)> {
    let mut query_pairs = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    query_pairs.sort();
    query_pairs
}

fn canonical_query_string(query_pairs: Vec<(String, String)>) -> String {
    query_pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

#[derive(Clone, Copy)]
struct LegacyDefaultQueryParamSpec {
    keys: &'static [&'static str],
    explicit_keys: &'static [&'static str],
    default_value: Option<&'static str>,
}

fn build_legacy_query_param_variant_choices(
    matching_pairs: &[(String, String)],
    spec: &LegacyDefaultQueryParamSpec,
) -> Option<Vec<Vec<(String, String)>>> {
    let shared_value = if let Some((_, value)) = matching_pairs.first() {
        if !matching_pairs
            .iter()
            .all(|(_, candidate)| candidate.trim().eq_ignore_ascii_case(value.trim()))
        {
            return None;
        }
        value.trim().to_string()
    } else {
        spec.default_value?.to_string()
    };

    let explicit_keys = if spec.explicit_keys.is_empty() {
        spec.keys
    } else {
        spec.explicit_keys
    };
    let mut choices = Vec::new();
    if spec
        .default_value
        .is_some_and(|default_value| shared_value.eq_ignore_ascii_case(default_value))
    {
        choices.push(Vec::new());
    }
    for mask in 1usize..(1usize << explicit_keys.len()) {
        let mut pairs = Vec::new();
        for (index, key) in explicit_keys.iter().enumerate() {
            if (mask & (1usize << index)) != 0 {
                pairs.push(((*key).to_string(), shared_value.clone()));
            }
        }
        choices.push(pairs);
    }
    Some(choices)
}

fn legacy_share_link_identity_variants(
    url: &Url,
    default_specs: &[LegacyDefaultQueryParamSpec],
) -> Vec<String> {
    let original_query_pairs = sorted_query_pairs(url);
    let mut static_pairs = Vec::new();
    let mut handled_keys = HashSet::new();
    let mut variant_choices: Vec<Vec<Vec<(String, String)>>> = Vec::new();

    for spec in default_specs {
        let matching_pairs = original_query_pairs
            .iter()
            .filter(|(key, _)| spec.keys.contains(&key.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        for key in spec.keys {
            handled_keys.insert(*key);
        }

        let Some(choices) = build_legacy_query_param_variant_choices(&matching_pairs, spec) else {
            static_pairs.extend(matching_pairs);
            continue;
        };
        variant_choices.push(choices);
    }

    static_pairs.extend(
        original_query_pairs
            .into_iter()
            .filter(|(key, _)| !handled_keys.contains(key.as_str())),
    );
    static_pairs.sort();

    let mut variants = vec![static_pairs];
    for choices in variant_choices {
        let mut next = Vec::new();
        let mut seen = HashSet::new();
        for variant in &variants {
            for choice in &choices {
                let mut updated = variant.clone();
                updated.extend(choice.iter().cloned());
                updated.sort();
                let query = canonical_query_string(updated.clone());
                if seen.insert(query) {
                    next.push(updated);
                }
            }
        }
        variants = next;
    }

    variants
        .into_iter()
        .map(|query_pairs| share_link_identity_with_query_pairs(url, query_pairs))
        .collect()
}

fn canonical_stream_query_pairs(
    url: &Url,
    default_security: Option<&str>,
    consumed_keys: &mut HashSet<&'static str>,
) -> Vec<(String, String)> {
    let original_query_pairs = sorted_query_pairs(url);
    let query = original_query_pairs
        .iter()
        .cloned()
        .collect::<HashMap<String, String>>();
    let network = normalized_query_ascii_lowercase(&query, &["type", "net"])
        .unwrap_or_else(|| "tcp".to_string());
    let security = normalized_query_ascii_lowercase(&query, &["security"])
        .or_else(|| default_security.map(|value| value.to_ascii_lowercase()))
        .unwrap_or_else(|| "none".to_string());
    let host = normalized_query_value(&query, &["host"])
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let path = normalized_query_value(&query, &["path"]).unwrap_or_default();
    let service_name = normalized_query_value(&query, &["serviceName", "service_name"])
        .or_else(|| (!path.is_empty()).then_some(path.clone()))
        .unwrap_or_default();

    consumed_keys.extend(["type", "net", "security"]);
    let mut query_pairs = vec![
        ("net".to_string(), network.clone()),
        ("security".to_string(), security.clone()),
    ];

    match network.as_str() {
        "ws" => {
            consumed_keys.extend(["host", "path"]);
            query_pairs.push(("host".to_string(), host.clone()));
            query_pairs.push(("path".to_string(), path.clone()));
        }
        "grpc" => {
            consumed_keys.extend(["serviceName", "service_name", "multiMode"]);
            query_pairs.push(("serviceName".to_string(), service_name.clone()));
            query_pairs.push((
                "multiMode".to_string(),
                if query_flag_true(&query, "multiMode") {
                    "true".to_string()
                } else {
                    "false".to_string()
                },
            ));
        }
        "httpupgrade" => {
            consumed_keys.extend(["host", "path"]);
            query_pairs.push(("host".to_string(), host.clone()));
            query_pairs.push(("path".to_string(), path.clone()));
        }
        _ => {}
    }

    match security.as_str() {
        "tls" => {
            consumed_keys.extend([
                "sni",
                "serverName",
                "allowInsecure",
                "insecure",
                "fp",
                "fingerprint",
                "alpn",
            ]);
            let server_name = normalized_query_value(&query, &["sni", "serverName"])
                .map(|value| value.to_ascii_lowercase())
                .or_else(|| (!host.is_empty()).then_some(host.clone()))
                .or_else(|| url.host_str().map(|value| value.to_ascii_lowercase()))
                .unwrap_or_default();
            let fingerprint = normalized_query_ascii_lowercase(&query, &["fp", "fingerprint"])
                .unwrap_or_default();
            let alpn = normalized_query_value(&query, &["alpn"])
                .map(|value| {
                    parse_alpn_csv(&value)
                        .into_iter()
                        .map(|item| item.to_ascii_lowercase())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            query_pairs.push(("alpn".to_string(), alpn));
            query_pairs.push((
                "allowInsecure".to_string(),
                if query_flag_true(&query, "allowInsecure") || query_flag_true(&query, "insecure") {
                    "true".to_string()
                } else {
                    "false".to_string()
                },
            ));
            query_pairs.push(("fp".to_string(), fingerprint));
            query_pairs.push(("serverName".to_string(), server_name));
        }
        "reality" => {
            consumed_keys.extend([
                "sni",
                "serverName",
                "fp",
                "fingerprint",
                "pbk",
                "sid",
                "spx",
            ]);
            let server_name = normalized_query_value(&query, &["sni", "serverName"])
                .map(|value| value.to_ascii_lowercase())
                .or_else(|| (!host.is_empty()).then_some(host.clone()))
                .or_else(|| url.host_str().map(|value| value.to_ascii_lowercase()))
                .unwrap_or_default();
            let fingerprint = normalized_query_ascii_lowercase(&query, &["fp", "fingerprint"])
                .unwrap_or_default();
            let public_key = normalized_query_value(&query, &["pbk"]).unwrap_or_default();
            let short_id = normalized_query_value(&query, &["sid"]).unwrap_or_default();
            let spider_x = normalized_query_value(&query, &["spx"]).unwrap_or_default();
            query_pairs.push(("fp".to_string(), fingerprint));
            query_pairs.push(("pbk".to_string(), public_key));
            query_pairs.push(("serverName".to_string(), server_name));
            query_pairs.push(("sid".to_string(), short_id));
            query_pairs.push(("spx".to_string(), spider_x));
        }
        _ => {}
    }

    query_pairs.extend(
        original_query_pairs
            .into_iter()
            .filter(|(key, _)| !consumed_keys.contains(key.as_str())),
    );
    query_pairs.sort();
    query_pairs
}

fn canonical_stream_identity_from_url(
    url: &Url,
    default_security: Option<&str>,
    consumed_keys: &mut HashSet<&'static str>,
) -> String {
    canonical_query_string(canonical_stream_query_pairs(
        url,
        default_security,
        consumed_keys,
    ))
}

fn canonical_vless_share_link_identity(url: &Url) -> String {
    let user_id = percent_decode_once_lossy(url.username());
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = url.port_or_known_default().unwrap_or_default();
    let query_pairs = sorted_query_pairs(url);
    let query = query_pairs
        .iter()
        .cloned()
        .collect::<HashMap<String, String>>();
    let encryption = normalized_query_ascii_lowercase(&query, &["encryption"])
        .unwrap_or_else(|| "none".to_string());
    let flow = normalized_query_value(&query, &["flow"]).unwrap_or_default();

    let mut consumed_keys = HashSet::from(["encryption", "flow"]);
    let mut canonical_query_pairs = vec![
        ("encryption".to_string(), encryption),
        ("flow".to_string(), flow),
    ];
    canonical_query_pairs.extend(canonical_stream_query_pairs(url, None, &mut consumed_keys));
    canonical_query_pairs.sort();

    let mut identity = String::from("vless://");
    identity.push_str(&user_id);
    identity.push('@');
    push_canonical_host_port(&mut identity, &host, port);
    identity.push('?');
    identity.push_str(&canonical_query_string(canonical_query_pairs));
    identity
}

fn canonical_trojan_share_link_identity(url: &Url) -> String {
    let password = percent_decode_once_lossy(url.username());
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = url.port_or_known_default().unwrap_or_default();

    let mut identity = String::from("trojan://");
    identity.push_str(&password);
    identity.push('@');
    push_canonical_host_port(&mut identity, &host, port);
    identity.push('?');
    identity.push_str(&canonical_stream_identity_from_url(
        url,
        Some("tls"),
        &mut HashSet::new(),
    ));
    identity
}

fn canonical_share_link_identity(url: &Url) -> String {
    share_link_identity_with_query_pairs(url, sorted_query_pairs(url))
}

fn share_link_identity_with_query_pairs(url: &Url, query_pairs: Vec<(String, String)>) -> String {
    let scheme = url.scheme().to_ascii_lowercase();
    let username = percent_decode_once_lossy(url.username());
    let password = url.password().map(percent_decode_once_lossy);
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let port = url.port_or_known_default().unwrap_or_default();
    let path = url.path();
    let query = canonical_query_string(query_pairs);

    let mut identity = format!("{scheme}://");
    if !username.is_empty() {
        identity.push_str(&username);
        if let Some(password) = password {
            identity.push(':');
            identity.push_str(&password);
        }
        identity.push('@');
    }
    if host.contains(':') {
        identity.push('[');
        identity.push_str(&host);
        identity.push(']');
    } else {
        identity.push_str(&host);
    }
    identity.push(':');
    identity.push_str(&port.to_string());
    if !path.is_empty() && path != "/" {
        identity.push_str(path);
    }
    if !query.is_empty() {
        identity.push('?');
        identity.push_str(&query);
    }
    identity
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
    header_type: Option<String>,
    service_name: Option<String>,
    authority: Option<String>,
    mode: Option<String>,
    seed: Option<String>,
    display_name: String,
}

impl VmessShareLink {
    fn stable_identity(&self) -> String {
        let alpn = self
            .alpn
            .as_ref()
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        format!(
            "vmess://{}@{}:{}?aid={}&security={}&net={}&host={}&path={}&tls={}&sni={}&alpn={}&fp={}&type={}&serviceName={}&authority={}&mode={}&seed={}",
            self.id,
            self.address.to_ascii_lowercase(),
            self.port,
            self.alter_id,
            self.security.to_ascii_lowercase(),
            self.network.to_ascii_lowercase(),
            self.host
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.path.as_deref().unwrap_or_default(),
            self.tls_mode
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.sni.as_deref().unwrap_or_default().to_ascii_lowercase(),
            alpn,
            self.fingerprint
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.header_type
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.service_name.as_deref().unwrap_or_default(),
            self.authority.as_deref().unwrap_or_default(),
            self.mode
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase(),
            self.seed.as_deref().unwrap_or_default(),
        )
    }
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
    let header_type = value
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let service_name = value
        .get("serviceName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let authority = value
        .get("authority")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let mode = value
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let seed = value
        .get("seed")
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
        header_type,
        service_name,
        authority,
        mode,
        seed,
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

impl ShadowsocksShareLink {
    fn stable_identity(&self) -> String {
        format!(
            "ss://{}:{}@{}:{}",
            self.method.to_ascii_lowercase(),
            self.password,
            self.host.to_ascii_lowercase(),
            self.port,
        )
    }
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
    pool_upstream: Client,
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

        // Pool live upstream traffic can legitimately stream well past REQUEST_TIMEOUT_SECS.
        // Handshake and upload budgets are enforced by route-specific timeout wrappers instead.
        let pool_upstream = Self::builder(None, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .build()
            .context("failed to construct pool upstream HTTP client")?;

        let proxy = Self::builder(None, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to construct proxy HTTP client")?;

        Ok(Self {
            shared,
            pool_upstream,
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

    fn client_for_pool_upstream(&self) -> Client {
        self.pool_upstream.clone()
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
    pool_upstream_responses_attempt_timeout: Duration,
    pool_upstream_responses_total_timeout: Duration,
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
