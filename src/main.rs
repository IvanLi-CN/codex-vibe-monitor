use std::{
    collections::{BTreeMap, HashSet},
    convert::Infallible,
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use axum::response::sse::{Event, KeepAlive};
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response, Sse},
    routing::get,
};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use chrono_tz::Asia::Shanghai;
use clap::Parser;
use dotenvy::dotenv;
use futures_util::{StreamExt, stream};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::{Client, ClientBuilder, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{
    FromRow, Pool, QueryBuilder, Row, Sqlite,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::fs;
use std::io::Read;
use tokio::{
    net::TcpListener,
    sync::{Semaphore, broadcast},
    task::JoinHandle,
    time::{MissedTickBehavior, interval, timeout},
};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{error, info, warn};

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
    let connect_opts = SqliteConnectOptions::from_str(&database_url)
        .context("invalid sqlite database url")?
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_opts)
        .await
        .context("failed to open sqlite database")?;

    ensure_schema(&pool).await?;

    let http_clients = HttpClients::build(&config)?;
    let (tx, _rx) = broadcast::channel(128);
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));

    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster: tx.clone(),
        semaphore: semaphore.clone(),
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

    let poller_handle = spawn_scheduler(state.clone(), cancel.clone());
    let server_handle = spawn_http_server(state.clone(), cancel.clone()).await?;

    // Wait until a shutdown signal is received, then wait for tasks to finish
    let _ = signals_task.await;

    if let Err(err) = server_handle.await {
        error!(?err, "http server terminated unexpectedly");
    }
    if let Err(err) = poller_handle.await {
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
    let mut router = Router::new()
        .route("/health", get(health_check))
        .route("/api/version", get(get_versions))
        .route("/api/invocations", get(list_invocations))
        .route("/api/stats", get(fetch_stats))
        .route("/api/stats/summary", get(fetch_summary))
        .route("/api/stats/timeseries", get(fetch_timeseries))
        .route("/api/stats/errors", get(fetch_error_distribution))
        .route("/api/stats/errors/others", get(fetch_other_errors))
        .route("/api/quota/latest", get(latest_quota_snapshot))
        .route("/events", get(sse_stream))
        .with_state(state.clone())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

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
        if let Err(err) = axum::serve(listener, router)
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

    let inserted = if records.is_empty() {
        Vec::new()
    } else {
        persist_records(&state.pool, &records).await?
    };

    let summaries = collect_summary_snapshots(&state.pool).await?;
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

async fn collect_summary_snapshots(pool: &Pool<Sqlite>) -> Result<Vec<SummaryPublish>> {
    let mut summaries = Vec::new();
    let mut cached_all: Option<StatsResponse> = None;
    let now = Utc::now();

    for spec in summary_broadcast_specs() {
        let summary = match spec.duration {
            None => {
                if let Some(existing) = &cached_all {
                    existing.clone()
                } else {
                    let stats: StatsResponse =
                        query_stats_row(pool, StatsFilter::All).await?.into();
                    cached_all = Some(stats.clone());
                    stats
                }
            }
            Some(duration) => {
                let start = now - duration;
                let start_str = format_naive(start.naive_utc());
                query_stats_row(pool, StatsFilter::Since(start_str))
                    .await?
                    .into()
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

async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS codex_invocations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            model TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cache_input_tokens INTEGER,
            reasoning_tokens INTEGER,
            total_tokens INTEGER,
            cost REAL,
            status TEXT,
            error_message TEXT,
            payload TEXT,
            raw_response TEXT NOT NULL,
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
        ("model", "TEXT"),
        ("input_tokens", "INTEGER"),
        ("output_tokens", "INTEGER"),
        ("cache_input_tokens", "INTEGER"),
        ("reasoning_tokens", "INTEGER"),
        ("total_tokens", "INTEGER"),
        ("cost", "REAL"),
        ("status", "TEXT"),
        ("error_message", "TEXT"),
        ("payload", "TEXT"),
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

    Ok(())
}

async fn persist_records(
    pool: &Pool<Sqlite>,
    records: &[CodexRecord],
) -> Result<Vec<ApiInvocation>> {
    let mut tx = pool.begin().await?;
    let mut inserted = Vec::new();

    for record in records {
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
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
                total_tokens,
                cost,
                status,
                error_message,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
        )
        .bind(&record.request_id)
        .bind(&record.request_time)
        .bind(&record.model)
        .bind(record.input_tokens)
        .bind(record.output_tokens)
        .bind(record.cache_input_tokens)
        .bind(record.reasoning_tokens)
        .bind(record.total_tokens)
        .bind(record.cost)
        .bind(&record.status)
        .bind(&record.error_message)
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
                    model,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    reasoning_tokens,
                    total_tokens,
                    cost,
                    status,
                    error_message,
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
        "SELECT id, invoke_id, occurred_at, model, input_tokens, output_tokens, \
         cache_input_tokens, reasoning_tokens, total_tokens, cost, status, error_message, created_at \
         FROM codex_invocations WHERE 1 = 1",
    );

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
    let row = query_stats_row(&state.pool, StatsFilter::All).await?;
    Ok(Json(row.into()))
}

async fn fetch_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SummaryQuery>,
) -> Result<Json<StatsResponse>, ApiError> {
    let default_limit = state.config.list_limit_max as i64;
    let window = parse_summary_window(&params, default_limit)?;

    let row = match window {
        SummaryWindow::All => query_stats_row(&state.pool, StatsFilter::All).await?,
        SummaryWindow::Current(limit) => {
            query_stats_row(&state.pool, StatsFilter::RecentLimit(limit)).await?
        }
        SummaryWindow::Duration(duration) => {
            let start_dt = (Utc::now() - duration).naive_utc();
            let start = format_naive(start_dt);
            query_stats_row(&state.pool, StatsFilter::Since(start)).await?
        }
    };

    Ok(Json(row.into()))
}

async fn fetch_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let range_duration = parse_duration_spec(&params.range)?;
    let mut bucket_seconds = if let Some(spec) = params.bucket.as_deref() {
        bucket_seconds_from_spec(spec)
            .ok_or_else(|| anyhow!("unsupported bucket specification: {spec}"))?
    } else {
        default_bucket_seconds(range_duration)
    };

    if bucket_seconds <= 0 {
        return Err(ApiError(anyhow!("bucket seconds must be positive")));
    }

    let range_seconds = range_duration.num_seconds();
    if range_seconds < bucket_seconds {
        return Err(ApiError(anyhow!(
            "bucket duration must not exceed selected range"
        )));
    }

    if range_seconds / bucket_seconds > 10_000 {
        // avoid accidentally returning extremely large payloads
        bucket_seconds = range_seconds / 10_000;
    }

    let settlement_hour = params.settlement_hour.unwrap_or(0);
    if settlement_hour >= 24 {
        return Err(ApiError(anyhow!(
            "settlement hour must be between 0 and 23 inclusive"
        )));
    }

    let offset_seconds = if bucket_seconds >= 86_400 {
        (settlement_hour as i64) * 3_600
    } else {
        0
    };

    let end_dt = Utc::now();
    let start_dt = end_dt - range_duration;
    let start_str_iso = format_utc_iso(start_dt);

    let records = sqlx::query_as::<_, TimeseriesRecord>(
        r#"
        SELECT occurred_at, status, total_tokens, cost
        FROM codex_invocations
        WHERE occurred_at >= ?1
        ORDER BY occurred_at ASC
        "#,
    )
    .bind(format_naive(start_dt.naive_utc()))
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
        entry.total_tokens += record.total_tokens.unwrap_or(0);
        entry.total_cost += record.cost.unwrap_or(0.0);
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
        points.push(TimeseriesPoint {
            bucket_start: format_utc_iso(start),
            bucket_end: format_utc_iso(end),
            total_count: agg.total_count,
            success_count: agg.success_count,
            failure_count: agg.failure_count,
            total_tokens: agg.total_tokens,
            total_cost: agg.total_cost,
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

#[derive(serde::Deserialize)]
struct ErrorQuery {
    range: String,
    top: Option<i64>,
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
struct OtherErrorsQuery {
    range: String,
    page: Option<i64>,
    limit: Option<i64>,
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

async fn fetch_error_distribution(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ErrorQuery>,
) -> Result<Json<ErrorDistributionResponse>, ApiError> {
    let range_duration = parse_duration_spec(&params.range)?;
    let end_dt = Utc::now();
    let start_dt = end_dt - range_duration;

    #[derive(sqlx::FromRow)]
    struct RawErr {
        error_message: Option<String>,
    }

    let rows: Vec<RawErr> = sqlx::query_as(
        r#"
        SELECT error_message
        FROM codex_invocations
        WHERE occurred_at >= ?1 AND (status IS NULL OR status != 'success')
        "#,
    )
    .bind(format_naive(start_dt.naive_utc()))
    .fetch_all(&state.pool)
    .await?;

    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for r in rows {
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
        range_end: format_utc_iso(end_dt),
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
    let range_duration = parse_duration_spec(&params.range)?;
    let end_dt = Utc::now();
    let start_dt = end_dt - range_duration;

    #[derive(sqlx::FromRow)]
    struct RowItem {
        id: i64,
        occurred_at: String,
        error_message: Option<String>,
    }
    let rows: Vec<RowItem> = sqlx::query_as(
        r#"
        SELECT id, occurred_at, error_message
        FROM codex_invocations
        WHERE occurred_at >= ?1 AND (status IS NULL OR status != 'success')
        ORDER BY occurred_at DESC
        "#,
    )
    .bind(format_naive(start_dt.naive_utc()))
    .fetch_all(&state.pool)
    .await?;

    let mut others: Vec<RowItem> = Vec::new();
    for r in rows.into_iter() {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VersionResponse {
    backend: String,
    frontend: String,
}

async fn get_versions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<VersionResponse>, ApiError> {
    let (backend, frontend) = detect_versions(state.config.static_dir.as_deref());
    Ok(Json(VersionResponse { backend, frontend }))
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

#[derive(Debug, Clone)]
struct AppState {
    config: AppConfig,
    pool: Pool<Sqlite>,
    http_clients: HttpClients,
    broadcaster: broadcast::Sender<BroadcastPayload>,
    semaphore: Arc<Semaphore>,
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
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    status: Option<String>,
    error_message: Option<String>,
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

impl From<StatsRow> for StatsResponse {
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    limit: Option<i64>,
    model: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SummaryQuery {
    window: Option<String>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TimeseriesQuery {
    #[serde(default = "default_range")]
    range: String,
    bucket: Option<String>,
    settlement_hour: Option<u8>,
}

#[derive(Debug)]
enum SummaryWindow {
    All,
    Current(i64),
    Duration(ChronoDuration),
}

#[derive(Debug)]
enum StatsFilter {
    All,
    Since(String),
    RecentLimit(i64),
}

#[derive(Debug, FromRow)]
struct TimeseriesRecord {
    occurred_at: String,
    status: Option<String>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
}

#[derive(Default)]
struct BucketAggregate {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
}

#[derive(Debug, Clone)]
struct HttpClients {
    shared: Client,
    timeout: Duration,
    user_agent: String,
}

impl HttpClients {
    fn build(config: &AppConfig) -> Result<Self> {
        let timeout = config.request_timeout;
        let user_agent = config.user_agent.clone();

        let shared = Self::builder(timeout, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .build()
            .context("failed to construct shared HTTP client")?;

        Ok(Self {
            shared,
            timeout,
            user_agent,
        })
    }

    fn client_for_parallelism(&self, force_new_connection: bool) -> Result<Client> {
        if force_new_connection {
            let client = Self::builder(self.timeout, &self.user_agent)
                .pool_max_idle_per_host(0)
                .build()
                .context("failed to construct dedicated HTTP client")?;
            Ok(client)
        } else {
            Ok(self.shared.clone())
        }
    }

    fn builder(timeout: Duration, user_agent: &str) -> ClientBuilder {
        Client::builder()
            .timeout(timeout)
            .user_agent(user_agent)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(90))
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    base_url: Url,
    quota_endpoint: String,
    cookie_name: String,
    cookie_value: String,
    database_path: PathBuf,
    poll_interval: Duration,
    request_timeout: Duration,
    max_parallel_polls: usize,
    shared_connection_parallelism: usize,
    http_bind: SocketAddr,
    list_limit_max: usize,
    user_agent: String,
    static_dir: Option<PathBuf>,
    snapshot_min_interval: Duration,
}

impl AppConfig {
    fn from_sources(overrides: &CliArgs) -> Result<Self> {
        let base_url_raw = overrides
            .base_url
            .clone()
            .or_else(|| env::var("XY_BASE_URL").ok())
            .ok_or_else(|| anyhow!("XY_BASE_URL is not set"))?;
        let quota_endpoint = overrides
            .quota_endpoint
            .clone()
            .or_else(|| env::var("XY_VIBE_QUOTA_ENDPOINT").ok())
            .unwrap_or_else(|| "/frontend-api/vibe-code/quota".to_string());
        let cookie_name = overrides
            .session_cookie_name
            .clone()
            .or_else(|| env::var("XY_SESSION_COOKIE_NAME").ok())
            .ok_or_else(|| anyhow!("XY_SESSION_COOKIE_NAME is not set"))?;
        let cookie_value = overrides
            .session_cookie_value
            .clone()
            .or_else(|| env::var("XY_SESSION_COOKIE_VALUE").ok())
            .ok_or_else(|| anyhow!("XY_SESSION_COOKIE_VALUE is not set"))?;
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
            .unwrap_or_else(|| "codex-vibe-monitor/0.1.0".to_string());
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

        Ok(Self {
            base_url: Url::parse(&base_url_raw).context("invalid XY_BASE_URL")?,
            quota_endpoint,
            cookie_name,
            cookie_value,
            database_path,
            poll_interval,
            request_timeout,
            max_parallel_polls,
            shared_connection_parallelism,
            http_bind,
            list_limit_max,
            user_agent,
            static_dir,
            snapshot_min_interval,
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::extract::State;
    use sqlx::SqlitePool;
    use std::{path::PathBuf, sync::Arc, time::Duration};
    use tokio::sync::{Semaphore, broadcast};

    fn test_config() -> AppConfig {
        AppConfig {
            base_url: Url::parse("https://example.com/").expect("valid url"),
            quota_endpoint: "/quota".to_string(),
            cookie_name: "session".to_string(),
            cookie_value: "test".to_string(),
            database_path: PathBuf::from(":memory:"),
            poll_interval: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            max_parallel_polls: 2,
            shared_connection_parallelism: 1,
            http_bind: "127.0.0.1:38080".parse().expect("valid socket address"),
            list_limit_max: 100,
            user_agent: "codex-test".to_string(),
            static_dir: None,
            snapshot_min_interval: Duration::from_secs(60),
        }
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
            semaphore,
        });

        let Json(snapshot) = latest_quota_snapshot(State(state))
            .await
            .expect("route should succeed");

        assert!(!snapshot.is_active);
        assert_eq!(snapshot.total_requests, 0);
        assert_eq!(snapshot.total_cost, 0.0);
    }
}

fn default_range() -> String {
    "1d".to_string()
}

fn format_naive(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
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
        Some(raw) => Ok(SummaryWindow::Duration(parse_duration_spec(raw)?)),
        None => Ok(SummaryWindow::Duration(ChronoDuration::days(1))),
    }
}

async fn query_stats_row(pool: &Pool<Sqlite>, filter: StatsFilter) -> Result<StatsRow> {
    match filter {
        StatsFilter::All => sqlx::query_as::<_, StatsRow>(
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
        StatsFilter::Since(start) => sqlx::query_as::<_, StatsRow>(
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
        .bind(start)
        .fetch_one(pool)
        .await
        .map_err(Into::into),
        StatsFilter::RecentLimit(limit) => sqlx::query_as::<_, StatsRow>(
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
        .map_err(Into::into),
    }
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
