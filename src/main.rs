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
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
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
    sync::{Mutex, RwLock, Semaphore, broadcast, mpsc, oneshot, watch},
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
mod maintenance;
mod oauth_bridge;
mod stats;
#[cfg(test)]
mod tests;
mod upstream_accounts;

use api::*;
use forward_proxy::*;
use stats::*;
use upstream_accounts::*;

include!("config.rs");
include!("app_state.rs");
include!("share_links.rs");
include!("runtime.rs");

pub(crate) mod config {
    #[allow(unused_imports)]
    pub(crate) use crate::{
        AppConfig, ArchiveBatchLayout, ArchiveFileCodec, ArchiveSegmentGranularity, CliArgs,
        CliCommand, CrsStatsConfig, ForwardProxyAlgo, MaintenanceCliArgs, MaintenanceCommand,
        MaintenanceDryRunArgs, RawCompressionCodec, UpstreamAccountsMoeMailConfig,
        should_recover_pending_pool_attempts_on_startup,
    };
}

pub(crate) mod app_state {
    #[allow(unused_imports)]
    pub(crate) use crate::{
        AppState, HttpClients, ModelPricing, PricingCatalog, PricingEntry, PricingSettingsResponse,
        PricingSettingsUpdateRequest, ProxyModelSettings, ProxyModelSettingsResponse,
        ProxyModelSettingsRow, ProxyModelSettingsUpdateRequest, SettingsResponse,
        decode_enabled_preset_models, decode_proxy_upstream_429_max_retries,
        default_enabled_preset_models, default_pricing_source_custom,
        normalize_enabled_preset_models, normalize_pricing_catalog_version,
        normalize_pricing_source, normalize_proxy_upstream_429_max_retries,
    };
}

pub(crate) mod share_links {
    #[cfg(test)]
    #[allow(unused_imports)]
    pub(crate) use crate::normalize_single_proxy_url;
    #[allow(unused_imports)]
    pub(crate) use crate::{
        ForwardProxyBindingParts, ParsedForwardProxyEntry, ShadowsocksShareLink, VmessShareLink,
        canonical_share_link_identity, canonical_trojan_share_link_identity,
        canonical_vless_share_link_identity, default_forward_proxy_insert_direct_compat,
        default_forward_proxy_subscription_interval_secs, deterministic_unit_f64,
        forward_proxy_binding_key_candidates, forward_proxy_binding_parts_from_raw,
        forward_proxy_storage_aliases, legacy_bound_proxy_key_aliases, normalize_bound_proxy_key,
        normalize_proxy_endpoints_from_urls, normalize_proxy_url_entries,
        normalize_share_link_scheme, normalize_single_proxy_key, normalize_subscription_entries,
        parse_forward_proxy_entry, parse_shadowsocks_share_link, parse_vmess_share_link,
        stable_forward_proxy_binding_key, stable_forward_proxy_key,
    };
}

pub(crate) mod runtime {
    #[allow(unused_imports)]
    pub(crate) use crate::{init_tracing, log_startup_phase, run, run_runtime_until_shutdown};
}
#[cfg_attr(not(test), allow(dead_code))]
const SOURCE_XY: &str = "xy";
const SOURCE_CRS: &str = "crs";
const SOURCE_PROXY: &str = "proxy";
const DEFAULT_OPENAI_UPSTREAM_BASE_URL: &str = "https://api.openai.com/";
const DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES: usize = 256 * 1024 * 1024;
const DEFAULT_PROXY_REQUEST_CONCURRENCY_LIMIT: usize = 12;
const DEFAULT_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS: u64 = 2_000;
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
const DEFAULT_PROXY_RAW_ASYNC_MAX_CONCURRENT_WRITERS: usize = 32;
const DEFAULT_PROXY_PRICING_CATALOG_PATH: &str = "config/model-pricing.json";
const DEFAULT_PROXY_RAW_DIR: &str = "proxy_raw_payloads";
const DEFAULT_PROXY_RAW_COMPRESSION: RawCompressionCodec = RawCompressionCodec::Gzip;
const DEFAULT_PROXY_RAW_HOT_SECS: u64 = 24 * 60 * 60;
const DEFAULT_PROXY_SUMMARY_QUOTA_BROADCAST_DEBOUNCE_MS: u64 = 100;
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
const ENV_PROXY_REQUEST_CONCURRENCY_LIMIT: &str = "PROXY_REQUEST_CONCURRENCY_LIMIT";
const ENV_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS: &str =
    "PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS";
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
const PROXY_FAILURE_PROXY_CONCURRENCY_LIMIT: &str = "proxy_concurrency_limit";
const PROXY_FAILURE_FAILED_CONTACT_UPSTREAM: &str = "failed_contact_upstream";
const PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT: &str = "upstream_handshake_timeout";
const PROXY_FAILURE_UPSTREAM_STREAM_ERROR: &str = "upstream_stream_error";
const PROXY_FAILURE_INVOCATION_INTERRUPTED: &str = "proxy_interrupted";
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
const INVOCATION_STATUS_RUNNING: &str = "running";
const INVOCATION_STATUS_PENDING: &str = "pending";
const INVOCATION_STATUS_INTERRUPTED: &str = "interrupted";
const INVOCATION_INTERRUPTED_MESSAGE: &str =
    "proxy request was interrupted before completion and was recovered on startup";
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

include!("maintenance/startup_prep.rs");
include!("maintenance/cli.rs");
include!("maintenance/startup_backfill.rs");
include!("maintenance/retention.rs");
include!("maintenance/archive.rs");
include!("maintenance/hourly_rollups.rs");

include!("schema.rs");
include!("pricing.rs");
include!("proxy.rs");

#[tokio::main]
async fn main() -> Result<()> {
    runtime::run().await
}
