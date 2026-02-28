use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap, HashSet},
    convert::Infallible,
    env,
    error::Error as StdError,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
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
    routing::{any, get, put},
};
use chrono::{
    DateTime, Datelike, Duration as ChronoDuration, LocalResult, NaiveDate, NaiveDateTime,
    SecondsFormat, TimeZone, Utc,
};
use chrono_tz::{Asia::Shanghai, Tz};
use clap::Parser;
use dotenvy::dotenv;
use flate2::read::GzDecoder;
use futures_util::{StreamExt, stream};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::{Client, ClientBuilder, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{
    FromRow, Pool, QueryBuilder, Row, Sqlite,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::fs;
use std::io::{self, Read, Write};
use tokio::{
    net::TcpListener,
    sync::{Mutex, RwLock, Semaphore, broadcast, mpsc},
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
use tracing::{error, info, warn};

const SOURCE_XY: &str = "xy";
const SOURCE_CRS: &str = "crs";
const SOURCE_PROXY: &str = "proxy";
const DEFAULT_OPENAI_UPSTREAM_BASE_URL: &str = "https://api.openai.com/";
const DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES: usize = 256 * 1024 * 1024;
const DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS: u64 = 45;
const DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS: u64 = 90;
const DEFAULT_SQLITE_BUSY_TIMEOUT_SECS: u64 = 30;
const BACKFILL_BATCH_SIZE: i64 = 200;
const BACKFILL_LOCK_RETRY_MAX_ATTEMPTS: u32 = 2;
const BACKFILL_LOCK_RETRY_DELAY_SECS: u64 = 3;
const DEFAULT_PROXY_RAW_MAX_BYTES: Option<usize> = None;
const DEFAULT_PROXY_RAW_RETENTION_DAYS: u64 = 7;
const DEFAULT_PROXY_PRICING_CATALOG_PATH: &str = "config/model-pricing.json";
const DEFAULT_PROXY_RAW_DIR: &str = "proxy_raw_payloads";
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
const DEFAULT_PRICING_CATALOG_VERSION: &str = "openai-standard-2026-02-23";
const DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE: bool = true;
const DEFAULT_XY_LEGACY_POLL_ENABLED: bool = false;
const DEFAULT_PROXY_MODELS_HIJACK_ENABLED: bool = false;
const DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED: bool = false;
const DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP: bool = true;
const ENV_CORS_ALLOWED_ORIGINS: &str = "XY_CORS_ALLOWED_ORIGINS";
const PROXY_PRESET_MODEL_IDS: &[&str] = &[
    "gpt-5.3-codex",
    "gpt-5.2-codex",
    "gpt-5.1-codex-max",
    "gpt-5.1-codex-mini",
    "gpt-5.2",
];
static NEXT_PROXY_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Parser, Debug, Default)]
#[command(
    name = "codex-vibe-monitor",
    about = "Monitor Codex Vibes",
    disable_help_subcommand = true
)]
struct CliArgs {
    /// Override the base URL used for upstream requests.
    #[arg(long, value_name = "URL")]
    base_url: Option<String>,
    /// Override the quota endpoint path or URL.
    #[arg(long, value_name = "ENDPOINT")]
    quota_endpoint: Option<String>,
    /// Override the session cookie name.
    #[arg(long, value_name = "NAME")]
    session_cookie_name: Option<String>,
    /// Override the session cookie value.
    #[arg(long, value_name = "VALUE")]
    session_cookie_value: Option<String>,
    /// Override the SQLite database path; falls back to env or default.
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
    /// Override the minimum interval between quota snapshots in seconds.
    #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64))]
    snapshot_min_interval_secs: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    dotenvy::from_filename(".env.local").ok();
    init_tracing();

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
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await
        .context("failed to open sqlite database")?;

    ensure_schema(&pool).await?;
    let pricing_catalog = load_pricing_catalog(&pool).await?;
    let raw_path_fallback_root = config.database_path.parent();
    if config.proxy_usage_backfill_on_startup {
        let summary = run_backfill_with_retry(&pool, raw_path_fallback_root).await?;
        info!(
            scanned = summary.scanned,
            updated = summary.updated,
            skipped_missing_file = summary.skipped_missing_file,
            skipped_without_usage = summary.skipped_without_usage,
            skipped_decode_error = summary.skipped_decode_error,
            "proxy usage startup backfill finished"
        );
    } else {
        info!("proxy usage startup backfill is disabled");
    }
    let cost_backfill_summary = run_cost_backfill_with_retry(&pool, &pricing_catalog).await?;
    info!(
        scanned = cost_backfill_summary.scanned,
        updated = cost_backfill_summary.updated,
        skipped_unpriced_model = cost_backfill_summary.skipped_unpriced_model,
        "proxy cost startup backfill finished"
    );
    let prompt_cache_summary =
        backfill_proxy_prompt_cache_keys(&pool, raw_path_fallback_root).await?;
    info!(
        scanned = prompt_cache_summary.scanned,
        updated = prompt_cache_summary.updated,
        skipped_missing_file = prompt_cache_summary.skipped_missing_file,
        skipped_invalid_json = prompt_cache_summary.skipped_invalid_json,
        skipped_missing_key = prompt_cache_summary.skipped_missing_key,
        "proxy prompt cache key startup backfill finished"
    );
    let failure_summary = backfill_failure_classification(&pool).await?;
    info!(
        scanned = failure_summary.scanned,
        updated = failure_summary.updated,
        "invocation failure classification startup backfill finished"
    );
    let proxy_model_settings = Arc::new(RwLock::new(load_proxy_model_settings(&pool).await?));
    fs::create_dir_all(&config.proxy_raw_dir).with_context(|| {
        format!(
            "failed to create proxy raw payload directory: {}",
            config.proxy_raw_dir.display()
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
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        semaphore: semaphore.clone(),
        proxy_model_settings,
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog,
    });

    // Shared cancellation token for graceful shutdown
    let cancel = CancellationToken::new();

    // Listen for OS signals and trigger cancellation
    let cancel_for_signals = cancel.clone();
    let signals_task = tokio::spawn(async move {
        shutdown_listener().await;
        cancel_for_signals.cancel();
        info!("shutdown signal received; beginning graceful shutdown");
    });

    let poller_handle = if state.config.legacy_poll_enabled || state.config.crs_stats.is_some() {
        Some(spawn_scheduler(state.clone(), cancel.clone()))
    } else {
        info!("legacy poller is disabled; scheduler will not start");
        None
    };
    let server_handle = spawn_http_server(state.clone(), cancel.clone()).await?;

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
        let fut = fetch_and_store(&state_clone, force_new_connection);
        match timeout(state_clone.config.request_timeout, fut).await {
            Ok(Ok(publish)) => {
                let PublishResult {
                    records,
                    summaries,
                    quota_snapshot,
                } = publish;

                if let Some(records) = records.filter(|v| !v.is_empty())
                    && let Err(err) = state_clone
                        .broadcaster
                        .send(BroadcastPayload::Records { records })
                {
                    warn!(?err, "failed to broadcast new records");
                }

                for summary in summaries {
                    if let Err(err) = state_clone.broadcaster.send(BroadcastPayload::Summary {
                        window: summary.window,
                        summary: summary.summary,
                    }) {
                        warn!(?err, "failed to broadcast summary payload");
                    }
                }

                if let Some(snapshot) = quota_snapshot
                    && let Err(err) = state_clone.broadcaster.send(BroadcastPayload::Quota {
                        snapshot: Box::new(snapshot),
                    })
                {
                    warn!(?err, "failed to broadcast quota snapshot");
                }
            }
            Ok(Err(err)) => {
                warn!(?err, "poll execution failed");
            }
            Err(_) => {
                warn!("quota fetch timed out");
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
        .route("/api/settings/pricing", put(put_pricing_settings))
        .route("/api/invocations", get(list_invocations))
        .route("/api/stats", get(fetch_stats))
        .route("/api/stats/summary", get(fetch_summary))
        .route("/api/stats/timeseries", get(fetch_timeseries))
        .route("/api/stats/perf", get(fetch_perf_stats))
        .route("/api/stats/errors", get(fetch_error_distribution))
        .route("/api/stats/failures/summary", get(fetch_failure_summary))
        .route("/api/stats/errors/others", get(fetch_other_errors))
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
    records: Option<Vec<ApiInvocation>>,
    summaries: Vec<SummaryPublish>,
    quota_snapshot: Option<QuotaSnapshotResponse>,
}

struct SummaryPublish {
    window: String,
    summary: StatsResponse,
}

async fn fetch_and_store(state: &AppState, force_new_connection: bool) -> Result<PublishResult> {
    let client = state
        .http_clients
        .client_for_parallelism(force_new_connection)?;
    let relay_config = state.config.crs_stats.clone();
    let mut inserted = Vec::new();

    if state.config.legacy_poll_enabled {
        let QuotaFetch {
            records,
            usage,
            subscription,
        } = fetch_quota(&client, &state.config).await?;

        maybe_persist_snapshot(
            &state.pool,
            usage,
            subscription,
            state.config.snapshot_min_interval,
        )
        .await?;

        if !records.is_empty() {
            inserted = persist_records(&state.pool, &records).await?;
        }
    }

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

    let summaries = collect_summary_snapshots(&state.pool, relay_config.as_ref()).await?;
    let quota_payload = QuotaSnapshotResponse::fetch_latest(&state.pool).await?;

    Ok(PublishResult {
        records: if inserted.is_empty() {
            None
        } else {
            Some(inserted)
        },
        summaries,
        quota_snapshot: quota_payload,
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

async fn fetch_quota(client: &Client, config: &AppConfig) -> Result<QuotaFetch> {
    let url = config.quota_url()?;
    let cookie_header = format!("{}={}", config.cookie_name, config.cookie_value);

    let response = client
        .get(url)
        .header(header::COOKIE, cookie_header)
        .send()
        .await
        .context("failed to send quota request")?
        .error_for_status()
        .context("quota request returned error status")?;

    let payload: QuotaResponse = response
        .json()
        .await
        .context("failed to decode quota response JSON")?;

    let mut records = Vec::new();
    let mut usage = None;
    let mut subscription = None;

    if let Some(data) = payload.data
        && let Some(service) = data.codex
    {
        records = service.recent_records;
        usage = service.current_usage;
        subscription = service.subscriptions;
    }

    Ok(QuotaFetch {
        records,
        usage,
        subscription,
    })
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
        CREATE TABLE IF NOT EXISTS proxy_model_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            hijack_enabled INTEGER NOT NULL DEFAULT 0,
            merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
            enabled_preset_models_json TEXT,
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

    let default_enabled_models_json = serde_json::to_string(&default_enabled_preset_models())
        .context("failed to serialize default enabled preset models")?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO proxy_model_settings (
            id,
            hijack_enabled,
            merge_upstream_enabled,
            enabled_preset_models_json
        )
        VALUES (?1, ?2, ?3, ?4)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(DEFAULT_PROXY_MODELS_HIJACK_ENABLED as i64)
    .bind(DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED as i64)
    .bind(default_enabled_models_json)
    .execute(pool)
    .await
    .context("failed to ensure default proxy_model_settings row")?;

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

    seed_default_pricing_catalog(pool).await?;

    Ok(())
}

async fn load_proxy_model_settings(pool: &Pool<Sqlite>) -> Result<ProxyModelSettings> {
    let row = sqlx::query_as::<_, ProxyModelSettingsRow>(
        r#"
        SELECT hijack_enabled, merge_upstream_enabled, enabled_preset_models_json
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
            enabled_preset_models_json = ?3,
            updated_at = datetime('now')
        WHERE id = ?4
        "#,
    )
    .bind(settings.hijack_enabled as i64)
    .bind(settings.merge_upstream_enabled as i64)
    .bind(enabled_models_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to persist proxy_model_settings row")?;

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

    save_pricing_catalog(pool, &default_pricing_catalog()).await
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
                source: "temporary".to_string(),
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

async fn persist_records(
    pool: &Pool<Sqlite>,
    records: &[CodexRecord],
) -> Result<Vec<ApiInvocation>> {
    let mut tx = pool.begin().await?;
    let mut inserted = Vec::new();

    for record in records {
        let failure = classify_invocation_failure(
            Some(record.status.as_str()),
            Some(record.error_message.as_str()),
        );
        let payload_json = json!({
            "model": record.model,
            "inputTokens": record.input_tokens,
            "outputTokens": record.output_tokens,
            "cacheInputTokens": record.cache_input_tokens,
            "reasoningTokens": record.reasoning_tokens,
            "totalTokens": record.total_tokens,
            "cost": record.cost,
            "status": record.status,
            "errorMessage": record.error_message,
        });

        let payload_text = serde_json::to_string(&payload_json)?;
        let raw_text = serde_json::to_string(record)?;

        let result = sqlx::query(
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
                status,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
            "#,
        )
        .bind(&record.request_id)
        .bind(&record.request_time)
        .bind(SOURCE_XY)
        .bind(&record.model)
        .bind(record.input_tokens)
        .bind(record.output_tokens)
        .bind(record.cache_input_tokens)
        .bind(record.reasoning_tokens)
        .bind(record.total_tokens)
        .bind(record.cost)
        .bind(&record.status)
        .bind(&record.error_message)
        .bind(failure.failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(payload_text)
        .bind(raw_text)
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() > 0 {
            let row = sqlx::query_as::<_, ApiInvocation>(
                r#"
                SELECT
                    id,
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
                    error_message,
                    failure_kind,
                    failure_class,
                    is_actionable,
                    created_at
                FROM codex_invocations
                WHERE invoke_id = ?1 AND occurred_at = ?2
                "#,
            )
            .bind(&record.request_id)
            .bind(&record.request_time)
            .fetch_one(&mut *tx)
            .await?;

            inserted.push(row);
        }
    }

    tx.commit().await?;
    Ok(inserted)
}

async fn maybe_persist_snapshot(
    pool: &Pool<Sqlite>,
    usage: Option<CurrentUsage>,
    subscription: Option<Subscription>,
    min_interval: Duration,
) -> Result<Option<QuotaSnapshotResponse>> {
    let usage = match usage {
        Some(usage) => usage,
        None => return Ok(None),
    };
    let subscription = match subscription {
        Some(subscription) => subscription,
        None => return Ok(None),
    };

    let last_row = sqlx::query_as::<_, QuotaSnapshotRow>(
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

    let now = Utc::now().naive_utc();
    let min_interval =
        ChronoDuration::from_std(min_interval).unwrap_or_else(|_| ChronoDuration::minutes(5));

    if let Some(ref last) = last_row
        && let Ok(last_captured) =
            NaiveDateTime::parse_from_str(&last.captured_at, "%Y-%m-%d %H:%M:%S")
    {
        let recent_enough = now - last_captured < min_interval;
        let cost_close = (usage.total_cost - last.total_cost).abs() < 0.000_001;
        let requests_same = usage.total_requests == last.total_requests;
        let tokens_same = usage.total_tokens == last.total_tokens;
        let subs_used = subscription.used_amount.unwrap_or(0.0);
        let last_used = last.used_amount.unwrap_or(0.0);
        let usage_same = (subs_used - last_used).abs() < 0.000_001;

        if recent_enough && cost_close && requests_same && tokens_same && usage_same {
            return Ok(None);
        }
    }

    sqlx::query(
        r#"
        INSERT INTO codex_quota_snapshots (
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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
        "#,
    )
    .bind(subscription.amount_limit.or(subscription.limit))
    .bind(subscription.used_amount)
    .bind(subscription.remaining_amount)
    .bind(subscription.period)
    .bind(subscription.period_reset_time)
    .bind(subscription.expire_time)
    .bind(subscription.is_active.unwrap_or(false) as i64)
    .bind(usage.total_cost)
    .bind(usage.total_requests)
    .bind(usage.total_tokens)
    .bind(usage.last_request_time)
    .bind(subscription.billing_type)
    .bind(subscription.remaining_count)
    .bind(subscription.used_count)
    .bind(subscription.sub_type_name)
    .execute(pool)
    .await?;

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

async fn list_invocations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let limit = params
        .limit
        .unwrap_or(50)
        .clamp(1, state.config.list_limit_max as i64);

    let mut query = QueryBuilder::new(
        "SELECT id, invoke_id, occurred_at, source, model, input_tokens, output_tokens, \
         cache_input_tokens, reasoning_tokens, total_tokens, cost, status, error_message, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint, \
         COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind, \
         failure_class, is_actionable, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip, \
         CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key, \
         cost_estimated, price_version, \
         request_raw_path, request_raw_size, request_raw_truncated, request_raw_truncated_reason, \
         response_raw_path, response_raw_size, response_raw_truncated, response_raw_truncated_reason, \
         raw_expires_at, \
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

async fn health_check() -> &'static str {
    "ok"
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
                match fetch_upstream_models_payload(&state, target_url.clone(), &headers).await {
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
            headers,
            body,
            target,
            target_url,
            peer_ip,
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

    let mut upstream_request = state
        .http_clients
        .proxy
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

    let handshake_timeout = state.config.openai_proxy_handshake_timeout;
    let upstream_response = timeout(handshake_timeout, upstream_request.send())
        .await
        .map_err(|_| {
            (
                StatusCode::BAD_GATEWAY,
                format!(
                    "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
                    handshake_timeout.as_millis()
                ),
            )
        })?
        .map_err(map_upstream_error)?;

    let rewritten_location = normalize_proxy_location_header(
        upstream_response.status(),
        upstream_response.headers(),
        &state.config.openai_upstream_base_url,
    )
    .map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to process upstream redirect: {err}"),
        )
    })?;

    let upstream_connection_scoped = connection_scoped_header_names(upstream_response.headers());
    let mut response_builder = Response::builder().status(upstream_response.status());
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
    tokio::spawn(async move {
        let mut forwarded_chunks = 0usize;
        let mut forwarded_bytes = 0usize;
        let stream_started_at = Instant::now();

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
                return;
            }
        }

        while let Some(next_chunk) = upstream_stream.next().await {
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
                        return;
                    }
                }
                Err(err) => {
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
                    return;
                }
            }
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
        _ => None,
    }
}

async fn proxy_openai_v1_capture_target(
    state: Arc<AppState>,
    proxy_request_id: u64,
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
                    None,
                    None,
                    request_info.parse_error.as_deref(),
                    Some(read_err.failure_kind),
                    requester_ip.as_deref(),
                    header_prompt_cache_key.as_deref(),
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

    let req_parse_started = Instant::now();
    let (upstream_body, request_info, body_rewritten) = prepare_target_request_body(
        capture_target,
        request_body_bytes,
        state.config.proxy_enforce_stream_include_usage,
    );
    let prompt_cache_key = request_info
        .prompt_cache_key
        .clone()
        .or_else(|| header_prompt_cache_key.clone());
    let t_req_parse_ms = elapsed_ms(req_parse_started);
    let req_raw = store_raw_payload_file(&state.config, &invoke_id, "request", &upstream_body);

    let mut upstream_request = state
        .http_clients
        .proxy
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
                    None,
                    None,
                    request_info.parse_error.as_deref(),
                    Some(failure_kind),
                    requester_ip.as_deref(),
                    prompt_cache_key.as_deref(),
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
                    None,
                    None,
                    request_info.parse_error.as_deref(),
                    Some(failure_kind),
                    requester_ip.as_deref(),
                    prompt_cache_key.as_deref(),
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
                    None,
                    None,
                    request_info.parse_error.as_deref(),
                    None,
                    requester_ip.as_deref(),
                    prompt_cache_key.as_deref(),
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
            response_info.model.as_deref(),
            response_info.usage_missing_reason.as_deref(),
            request_info_for_task.parse_error.as_deref(),
            failure_kind,
            requester_ip_for_task.as_deref(),
            prompt_cache_key_for_task.as_deref(),
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
) -> (Vec<u8>, RequestCaptureInfo, bool) {
    let mut info = RequestCaptureInfo {
        model: None,
        prompt_cache_key: None,
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
    info.is_stream = value
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut rewritten = false;
    if target == ProxyCaptureTarget::ChatCompletions
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
        };
    }

    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_parse(bytes, content_encoding);
    let parse_bytes = decoded_bytes.as_ref();
    let looks_like_stream = request_is_stream || parse_bytes.starts_with(b"data:");
    let mut response_info = if looks_like_stream {
        parse_stream_response_payload(parse_bytes)
    } else {
        match serde_json::from_slice::<Value>(parse_bytes) {
            Ok(value) => {
                let model = extract_model_from_payload(&value);
                let usage = extract_usage_from_payload(&value).unwrap_or_default();
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
                }
            }
            Err(_) => ResponseCaptureInfo {
                model: None,
                usage: ParsedUsage::default(),
                usage_missing_reason: Some("response_not_json".to_string()),
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
    response_model: Option<&str>,
    usage_missing_reason: Option<&str>,
    request_parse_error: Option<&str>,
    failure_kind: Option<&str>,
    requester_ip: Option<&str>,
    prompt_cache_key: Option<&str>,
) -> String {
    let endpoint = match target {
        ProxyCaptureTarget::ChatCompletions => "/v1/chat/completions",
        ProxyCaptureTarget::Responses => "/v1/responses",
    };
    let payload = json!({
        "endpoint": endpoint,
        "statusCode": status.as_u16(),
        "isStream": is_stream,
        "requestModel": request_model,
        "responseModel": response_model,
        "usageMissingReason": usage_missing_reason,
        "requestParseError": request_parse_error,
        "failureKind": failure_kind,
        "requesterIp": requester_ip,
        "promptCacheKey": prompt_cache_key,
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

fn pricing_backfill_attempt_version(catalog: &PricingCatalog) -> String {
    fn mix_fvn1a(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x100000001b3);
        }
    }

    let mut hash = 0xcbf29ce484222325_u64;
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

    let billable_cache_tokens = if pricing.cache_input_per_1m.is_some() {
        cache_input_tokens
    } else {
        0
    };
    let non_cached_input_tokens = input_tokens.saturating_sub(billable_cache_tokens);

    let mut cost = (non_cached_input_tokens as f64 / 1_000_000.0) * pricing.input_per_1m
        + (output_tokens / 1_000_000.0) * pricing.output_per_1m;
    if let Some(cache_price) = pricing.cache_input_per_1m {
        cost += (billable_cache_tokens as f64 / 1_000_000.0) * cache_price;
    }
    if let Some(reasoning_price) = pricing.reasoning_per_1m {
        cost += (reasoning_tokens / 1_000_000.0) * reasoning_price;
    }

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

    if let Err(err) = fs::create_dir_all(&config.proxy_raw_dir) {
        meta.truncated = true;
        meta.truncated_reason = Some(format!("write_failed:{err}"));
        return meta;
    }

    let filename = format!("{invoke_id}-{kind}.bin");
    let path = config.proxy_raw_dir.join(filename);
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
                        if let Err(err) = broadcaster.send(BroadcastPayload::Summary {
                            window: summary.window.clone(),
                            summary: summary.summary,
                        }) {
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
                    if let Err(err) = broadcaster.send(BroadcastPayload::Quota {
                        snapshot: Box::new(snapshot),
                    }) {
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
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
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

async fn backfill_proxy_usage_tokens(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let snapshot_max_id: i64 = sqlx::query_scalar(
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
    .await?;

    backfill_proxy_usage_tokens_up_to_id(pool, snapshot_max_id, raw_path_fallback_root).await
}

async fn backfill_proxy_usage_tokens_up_to_id(
    pool: &Pool<Sqlite>,
    snapshot_max_id: i64,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let mut summary = ProxyUsageBackfillSummary::default();
    let mut last_seen_id = 0_i64;
    loop {
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
        .bind(BACKFILL_BATCH_SIZE)
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
                    Err(err) => {
                        summary.skipped_missing_file += 1;
                        warn!(
                            id = candidate.id,
                            path = %candidate.response_raw_path,
                            error = %err,
                            "proxy usage backfill skipped because response raw file is unavailable"
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

        if updates.is_empty() {
            continue;
        }

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

    Ok(summary)
}

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

async fn backfill_proxy_missing_costs(
    pool: &Pool<Sqlite>,
    catalog: &PricingCatalog,
) -> Result<ProxyCostBackfillSummary> {
    let attempt_version = pricing_backfill_attempt_version(catalog);
    let snapshot_max_id: i64 = sqlx::query_scalar(
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
    .bind(&attempt_version)
    .fetch_one(pool)
    .await?;

    backfill_proxy_missing_costs_up_to_id(pool, snapshot_max_id, catalog, &attempt_version).await
}

async fn backfill_proxy_missing_costs_up_to_id(
    pool: &Pool<Sqlite>,
    snapshot_max_id: i64,
    catalog: &PricingCatalog,
    attempt_version: &str,
) -> Result<ProxyCostBackfillSummary> {
    let mut summary = ProxyCostBackfillSummary::default();
    let mut last_seen_id = 0_i64;
    loop {
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
        .bind(BACKFILL_BATCH_SIZE)
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

        if updates.is_empty() {
            continue;
        }

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

    Ok(summary)
}

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

async fn backfill_proxy_prompt_cache_keys(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyPromptCacheKeyBackfillSummary> {
    let mut summary = ProxyPromptCacheKeyBackfillSummary::default();
    let mut last_seen_id = 0_i64;

    loop {
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
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request = match read_proxy_raw_bytes(
                &candidate.request_raw_path,
                raw_path_fallback_root,
            ) {
                Ok(content) => content,
                Err(err) => {
                    summary.skipped_missing_file += 1;
                    warn!(
                        id = candidate.id,
                        path = %candidate.request_raw_path,
                        error = %err,
                        "proxy prompt cache key backfill skipped because request raw file is unavailable"
                    );
                    continue;
                }
            };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(err) => {
                    summary.skipped_invalid_json += 1;
                    warn!(
                        id = candidate.id,
                        path = %candidate.request_raw_path,
                        error = %err,
                        "proxy prompt cache key backfill skipped because request raw file is not valid JSON"
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

    Ok(summary)
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

async fn backfill_failure_classification(
    pool: &Pool<Sqlite>,
) -> Result<FailureClassificationBackfillSummary> {
    let mut summary = FailureClassificationBackfillSummary::default();
    let mut last_seen_id = 0_i64;

    loop {
        let rows = sqlx::query_as::<_, FailureClassificationBackfillRow>(
            r#"
            SELECT id, status, error_message, failure_kind, failure_class, is_actionable
            FROM codex_invocations
            WHERE id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(last_seen_id)
        .bind(BACKFILL_BATCH_SIZE)
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

    Ok(summary)
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
        target = match endpoint {
            "/v1/chat/completions" => ProxyCaptureTarget::ChatCompletions,
            "/v1/responses" => ProxyCaptureTarget::Responses,
            _ => ProxyCaptureTarget::Responses,
        };
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
    Ok(Json(SettingsResponse {
        proxy: proxy.into(),
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

async fn fetch_upstream_models_payload(
    state: &AppState,
    target_url: Url,
    headers: &HeaderMap,
) -> Result<Value> {
    let mut upstream_request = state.http_clients.proxy.request(Method::GET, target_url);
    let request_connection_scoped = connection_scoped_header_names(headers);
    for (name, value) in headers {
        if should_forward_proxy_header(name, &request_connection_scoped) {
            upstream_request = upstream_request.header(name, value);
        }
    }

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

    if !upstream_response.status().is_success() {
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
    proxy_summary_quota_broadcast_seq: Arc<AtomicU64>,
    proxy_summary_quota_broadcast_running: Arc<AtomicBool>,
    semaphore: Arc<Semaphore>,
    proxy_model_settings: Arc<RwLock<ProxyModelSettings>>,
    proxy_model_settings_update_lock: Arc<Mutex<()>>,
    pricing_settings_update_lock: Arc<Mutex<()>>,
    pricing_catalog: Arc<RwLock<PricingCatalog>>,
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
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
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

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
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
    is_stream: bool,
    parse_error: Option<String>,
}

#[derive(Debug, Clone)]
struct ResponseCaptureInfo {
    model: Option<String>,
    usage: ParsedUsage,
    usage_missing_reason: Option<String>,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    limit: Option<i64>,
    model: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyModelSettings {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    enabled_preset_models: Vec<String>,
}

impl Default for ProxyModelSettings {
    fn default() -> Self {
        Self {
            hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            merge_upstream_enabled: DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED,
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
            enabled_preset_models: normalize_enabled_preset_models(self.enabled_preset_models),
        }
    }
}

#[derive(Debug, FromRow)]
struct ProxyModelSettingsRow {
    hijack_enabled: i64,
    merge_upstream_enabled: i64,
    enabled_preset_models_json: Option<String>,
}

impl From<ProxyModelSettingsRow> for ProxyModelSettings {
    fn from(value: ProxyModelSettingsRow) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled != 0,
            merge_upstream_enabled: value.merge_upstream_enabled != 0,
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
    #[serde(default = "default_enabled_preset_models")]
    enabled_models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyModelSettingsResponse {
    hijack_enabled: bool,
    merge_upstream_enabled: bool,
    default_hijack_enabled: bool,
    models: Vec<String>,
    enabled_models: Vec<String>,
}

impl From<ProxyModelSettings> for ProxyModelSettingsResponse {
    fn from(value: ProxyModelSettings) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled,
            merge_upstream_enabled: value.merge_upstream_enabled,
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
    legacy_poll_enabled: bool,
    base_url: Url,
    openai_upstream_base_url: Url,
    quota_endpoint: String,
    cookie_name: String,
    cookie_value: String,
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
    max_parallel_polls: usize,
    shared_connection_parallelism: usize,
    http_bind: SocketAddr,
    cors_allowed_origins: Vec<String>,
    list_limit_max: usize,
    user_agent: String,
    static_dir: Option<PathBuf>,
    snapshot_min_interval: Duration,
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
        let legacy_poll_enabled =
            parse_bool_env_var("XY_LEGACY_POLL_ENABLED", DEFAULT_XY_LEGACY_POLL_ENABLED)?;
        let base_url_raw_opt = overrides
            .base_url
            .clone()
            .or_else(|| env::var("XY_BASE_URL").ok());
        let base_url_raw = if legacy_poll_enabled {
            base_url_raw_opt.ok_or_else(|| anyhow!("XY_BASE_URL is not set"))?
        } else {
            base_url_raw_opt.unwrap_or_else(|| "http://127.0.0.1/".to_string())
        };
        let openai_upstream_base_url = env::var("OPENAI_UPSTREAM_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_OPENAI_UPSTREAM_BASE_URL.to_string());
        let quota_endpoint = overrides
            .quota_endpoint
            .clone()
            .or_else(|| env::var("XY_VIBE_QUOTA_ENDPOINT").ok())
            .unwrap_or_else(|| "/frontend-api/vibe-code/quota".to_string());
        let cookie_name_opt = overrides
            .session_cookie_name
            .clone()
            .or_else(|| env::var("XY_SESSION_COOKIE_NAME").ok());
        let cookie_value_opt = overrides
            .session_cookie_value
            .clone()
            .or_else(|| env::var("XY_SESSION_COOKIE_VALUE").ok());
        let cookie_name = if legacy_poll_enabled {
            cookie_name_opt.ok_or_else(|| anyhow!("XY_SESSION_COOKIE_NAME is not set"))?
        } else {
            cookie_name_opt.unwrap_or_else(|| "xy_session".to_string())
        };
        let cookie_value = if legacy_poll_enabled {
            cookie_value_opt.ok_or_else(|| anyhow!("XY_SESSION_COOKIE_VALUE is not set"))?
        } else {
            cookie_value_opt.unwrap_or_default()
        };
        let database_path = overrides
            .database_path
            .clone()
            .or_else(|| env::var("XY_DATABASE_PATH").ok().map(PathBuf::from))
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
        let snapshot_min_interval = overrides
            .snapshot_min_interval_secs
            .or_else(|| {
                env::var("XY_SNAPSHOT_MIN_INTERVAL_SECS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
            })
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(300));

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
            legacy_poll_enabled,
            base_url: Url::parse(&base_url_raw).context("invalid XY_BASE_URL")?,
            openai_upstream_base_url: Url::parse(&openai_upstream_base_url)
                .context("invalid OPENAI_UPSTREAM_BASE_URL")?,
            quota_endpoint,
            cookie_name,
            cookie_value,
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
            max_parallel_polls,
            shared_connection_parallelism,
            http_bind,
            cors_allowed_origins,
            list_limit_max,
            user_agent,
            static_dir,
            snapshot_min_interval,
            crs_stats,
        })
    }

    fn quota_url(&self) -> Result<Url> {
        if self.quota_endpoint.starts_with("http") {
            Url::parse(&self.quota_endpoint).context("invalid XY_VIBE_QUOTA_ENDPOINT URL")
        } else {
            self.base_url
                .join(self.quota_endpoint.trim_start_matches('/'))
                .context("failed to join quota endpoint onto base URL")
        }
    }

    fn database_url(&self) -> String {
        format!("sqlite://{}", self.database_path.to_string_lossy())
    }
}

fn parse_bool_env_var(name: &str, default_value: bool) -> Result<bool> {
    match env::var(name) {
        Ok(raw) => parse_bool_string(&raw).ok_or_else(|| anyhow!("invalid {name}: {raw}")),
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
        routing::{any, get},
    };
    use chrono::Timelike;
    use flate2::{Compression, write::GzEncoder};
    use serde_json::Value;
    use sqlx::error::{DatabaseError, ErrorKind};
    use sqlx::{Connection, SqliteConnection, SqlitePool};
    use std::{
        borrow::Cow,
        collections::HashSet,
        fs,
        path::{Path, PathBuf},
        sync::Arc,
        time::Duration,
    };
    use tokio::net::TcpListener;
    use tokio::sync::{Semaphore, broadcast};
    use tokio::task::JoinHandle;

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

    fn test_config() -> AppConfig {
        AppConfig {
            legacy_poll_enabled: false,
            base_url: Url::parse("https://example.com/").expect("valid url"),
            openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
            quota_endpoint: "/quota".to_string(),
            cookie_name: "session".to_string(),
            cookie_value: "test".to_string(),
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
            max_parallel_polls: 2,
            shared_connection_parallelism: 1,
            http_bind: "127.0.0.1:38080".parse().expect("valid socket address"),
            cors_allowed_origins: Vec::new(),
            list_limit_max: 100,
            user_agent: "codex-test".to_string(),
            static_dir: None,
            snapshot_min_interval: Duration::from_secs(60),
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

    fn sqlite_url_for_path(path: &Path) -> String {
        format!("sqlite://{}", path.to_string_lossy())
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
        fs::write(path, compressed).expect("write response payload");
    }

    fn write_backfill_request_payload(path: &Path, prompt_cache_key: Option<&str>) {
        let payload = if let Some(key) = prompt_cache_key {
            json!({
                "model": "gpt-5.3-codex",
                "stream": true,
                "metadata": {
                    "prompt_cache_key": key
                }
            })
        } else {
            json!({
                "model": "gpt-5.3-codex",
                "stream": true,
                "metadata": {}
            })
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
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
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
            config,
            pool,
            http_clients,
            broadcaster,
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            semaphore,
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(pricing_catalog)),
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
                "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":false,\"requesterIp\":\"198.51.100.77\",\"promptCacheKey\":\"pck-broadcast-1\"}"
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
            .route("/v1/responses", any(test_upstream_responses));

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
                enabled_models: vec!["gpt-5.2-codex".to_string(), "unknown-model".to_string()],
            }),
        )
        .await
        .expect("put settings should succeed");
        assert!(updated.hijack_enabled);
        assert!(updated.merge_upstream_enabled);
        assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);

        let persisted = load_proxy_model_settings(&state.pool)
            .await
            .expect("settings should persist");
        assert!(persisted.hijack_enabled);
        assert!(persisted.merge_upstream_enabled);
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
                enabled_models: Vec::new(),
            }),
        )
        .await
        .expect("put settings should normalize payload");
        assert!(!normalized.hijack_enabled);
        assert!(!normalized.merge_upstream_enabled);
        assert!(normalized.enabled_models.is_empty());
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
            config,
            pool,
            http_clients,
            broadcaster,
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            semaphore,
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings {
                hijack_enabled: true,
                merge_upstream_enabled: true,
                enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
            })),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
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
            config,
            pool,
            http_clients,
            broadcaster,
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            semaphore,
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
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
            config,
            pool,
            http_clients,
            broadcaster,
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            semaphore,
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
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
            config,
            pool,
            http_clients,
            broadcaster,
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            semaphore,
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
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
            config,
            pool,
            http_clients,
            broadcaster,
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            semaphore,
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
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
        let (rewritten, info, did_rewrite) =
            prepare_target_request_body(ProxyCaptureTarget::ChatCompletions, body, true);
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

        let (_rewritten, info, _did_rewrite) =
            prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

        assert_eq!(info.prompt_cache_key.as_deref(), Some("pck-from-body"));
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
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":18}}",
            "data: [DONE]",
        ]
        .join("\n");
        let parsed = parse_stream_response_payload(raw.as_bytes());
        assert_eq!(parsed.model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(parsed.usage.input_tokens, Some(11));
        assert_eq!(parsed.usage.output_tokens, Some(7));
        assert_eq!(parsed.usage.total_tokens, Some(18));
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
    fn parse_target_response_payload_decodes_gzip_stream_usage() {
        let raw = [
            "event: response.created",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
            "",
            "event: response.completed",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
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
        assert!(parsed.usage_missing_reason.is_none());
    }

    #[test]
    fn parse_target_response_payload_decodes_multi_value_content_encoding() {
        let raw = [
            "event: response.created",
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
            "",
            "event: response.completed",
            "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
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
        assert!(parsed.usage_missing_reason.is_none());
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
            "{\"endpoint\":\"/v1/responses\",\"failureKind\":\"upstream_stream_error\",\"requesterIp\":\"198.51.100.77\",\"promptCacheKey\":\"pck-list-1\"}",
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
    async fn backfill_proxy_missing_costs_tracks_skip_reasons() {
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
            config,
            pool,
            http_clients,
            broadcaster,
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            semaphore,
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        });

        let Json(snapshot) = latest_quota_snapshot(State(state))
            .await
            .expect("route should succeed");

        assert!(!snapshot.is_active);
        assert_eq!(snapshot.total_requests, 0);
        assert_eq!(snapshot.total_cost, 0.0);
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
        let occurred_at = format_naive(
            (Utc::now() - ChronoDuration::minutes(15))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
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
    let row = query_stats_row(pool, filter, source_scope).await?;
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

#[derive(Debug, Deserialize)]
struct QuotaResponse {
    #[allow(dead_code)]
    code: i32,
    data: Option<QuotaData>,
}

#[derive(Debug, Deserialize)]
struct QuotaData {
    codex: Option<ServiceQuota>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServiceQuota {
    #[serde(default)]
    recent_records: Vec<CodexRecord>,
    #[serde(default)]
    current_usage: Option<CurrentUsage>,
    #[serde(default)]
    subscriptions: Option<Subscription>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CurrentUsage {
    #[serde(default)]
    last_request_time: Option<String>,
    #[serde(default)]
    total_cost: f64,
    #[serde(default)]
    total_requests: i64,
    #[serde(default)]
    total_tokens: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Subscription {
    #[serde(default)]
    amount_limit: Option<f64>,
    #[serde(default)]
    billing_type: Option<String>,
    #[serde(default)]
    expire_time: Option<String>,
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    is_active: Option<bool>,
    #[serde(default)]
    limit: Option<f64>,
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    period_reset_time: Option<String>,
    #[serde(default)]
    remaining_amount: Option<f64>,
    #[serde(default)]
    remaining_count: Option<i64>,
    #[serde(default)]
    sub_type_id: Option<i64>,
    #[serde(default)]
    sub_type_name: Option<String>,
    #[serde(default)]
    used_amount: Option<f64>,
    #[serde(default)]
    used_count: Option<i64>,
}

#[derive(Debug)]
struct QuotaFetch {
    records: Vec<CodexRecord>,
    usage: Option<CurrentUsage>,
    subscription: Option<Subscription>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexRecord {
    request_id: String,
    request_time: String,
    model: String,
    input_tokens: i64,
    output_tokens: i64,
    cache_input_tokens: i64,
    reasoning_tokens: i64,
    total_tokens: i64,
    cost: f64,
    status: String,
    #[serde(default)]
    error_message: String,
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
