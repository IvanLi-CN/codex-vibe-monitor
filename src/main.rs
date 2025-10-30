use std::{
    collections::HashSet, convert::Infallible, env, net::SocketAddr, path::PathBuf, str::FromStr,
    sync::Arc, time::Duration,
};

use anyhow::{Context, Result};
use axum::response::sse::{Event, KeepAlive};
use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response, Sse},
    routing::get,
};
use dotenvy::dotenv;
use futures_util::StreamExt;
use reqwest::{Client, ClientBuilder, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{
    FromRow, Pool, QueryBuilder, Row, Sqlite,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, broadcast},
    task::JoinHandle,
    time::{MissedTickBehavior, interval, timeout},
};
use tokio_stream::wrappers::BroadcastStream;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{error, info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    dotenvy::from_filename(".env.local").ok();
    init_tracing();

    let config = AppConfig::from_env()?;
    info!(?config, "starting codex vibe monitor");

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

    let poller_handle = spawn_scheduler(state.clone());
    let server_handle = spawn_http_server(state.clone()).await?;

    tokio::select! {
        res = poller_handle => {
            if let Err(err) = res {
                error!(?err, "poller task terminated unexpectedly");
            }
        }
        res = server_handle => {
            if let Err(err) = res {
                error!(?err, "http server terminated unexpectedly");
            }
        }
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

fn spawn_scheduler(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(err) = schedule_poll(state.clone()).await {
            warn!(?err, "initial poll failed");
        }

        let mut ticker = interval(state.config.poll_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            ticker.tick().await;
            if let Err(err) = schedule_poll(state.clone()).await {
                warn!(?err, "scheduled poll failed");
            }
        }
    })
}

async fn schedule_poll(state: Arc<AppState>) -> Result<()> {
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

    tokio::spawn(async move {
        let fut = fetch_and_store(&state_clone, force_new_connection);
        match timeout(state_clone.config.request_timeout, fut).await {
            Ok(Ok(new_records)) => {
                if !new_records.is_empty() {
                    let payload = BroadcastPayload::Records {
                        records: new_records,
                    };
                    if let Err(err) = state_clone.broadcaster.send(payload) {
                        warn!(?err, "failed to broadcast new records");
                    }
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

    Ok(())
}

async fn spawn_http_server(state: Arc<AppState>) -> Result<JoinHandle<()>> {
    let mut router = Router::new()
        .route("/health", get(health_check))
        .route("/api/invocations", get(list_invocations))
        .route("/api/stats", get(fetch_stats))
        .route("/events", get(sse_stream))
        .with_state(state.clone())
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

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
            .with_graceful_shutdown(shutdown_signal())
            .await
        {
            error!(?err, "http server exited with error");
        }
    });

    Ok(handle)
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!(?err, "failed to listen for shutdown signal");
    }
    info!("shutdown signal received");
}

async fn fetch_and_store(
    state: &AppState,
    force_new_connection: bool,
) -> Result<Vec<ApiInvocation>> {
    let client = state
        .http_clients
        .client_for_parallelism(force_new_connection)?;
    let records = fetch_quota(&client, &state.config).await?;

    if records.is_empty() {
        return Ok(Vec::new());
    }

    let inserted = persist_records(&state.pool, &records).await?;
    Ok(inserted)
}

async fn fetch_quota(client: &Client, config: &AppConfig) -> Result<Vec<CodexRecord>> {
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

    let records = payload
        .data
        .and_then(|data| data.codex)
        .map(|service| service.recent_records)
        .unwrap_or_default();

    Ok(records)
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

async fn list_invocations(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let limit = params
        .limit
        .unwrap_or(50)
        .clamp(1, state.config.list_limit_max as i64) as i64;

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
    let row = sqlx::query_as::<_, StatsRow>(
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
    .fetch_one(&state.pool)
    .await?;

    Ok(Json(StatsResponse {
        total_count: row.total_count,
        success_count: row.success_count.unwrap_or(0),
        failure_count: row.failure_count.unwrap_or(0),
        total_cost: row.total_cost,
        total_tokens: row.total_tokens,
    }))
}

async fn sse_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.broadcaster.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|res| async {
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

    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn health_check() -> &'static str {
    "ok"
}

fn ensure_db_directory(path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory: {}", parent.display())
            })?;
        }
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
    Records { records: Vec<ApiInvocation> },
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
struct ApiInvocation {
    id: i64,
    invoke_id: String,
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
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    records: Vec<ApiInvocation>,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListQuery {
    limit: Option<i64>,
    model: Option<String>,
    status: Option<String>,
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
            .pool_max_idle_per_host(config.shared_connection_parallelism as usize)
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
}

impl AppConfig {
    fn from_env() -> Result<Self> {
        let base_url = env::var("XY_BASE_URL").context("XY_BASE_URL is not set")?;
        let quota_endpoint = env::var("XY_VIBE_QUOTA_ENDPOINT")
            .unwrap_or_else(|_| "/frontend-api/vibe-code/quota".to_string());
        let cookie_name =
            env::var("XY_SESSION_COOKIE_NAME").context("XY_SESSION_COOKIE_NAME is not set")?;
        let cookie_value =
            env::var("XY_SESSION_COOKIE_VALUE").context("XY_SESSION_COOKIE_VALUE is not set")?;
        let database_path =
            env::var("XY_DATABASE_PATH").unwrap_or_else(|_| "codex_vibe_monitor.db".to_string());
        let poll_interval = env::var("XY_POLL_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(10));
        let request_timeout = env::var("XY_REQUEST_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(60));
        let max_parallel_polls = env::var("XY_MAX_PARALLEL_POLLS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(6);
        let shared_connection_parallelism = env::var("XY_SHARED_CONNECTION_PARALLELISM")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(2);
        let http_bind = env::var("XY_HTTP_BIND")
            .ok()
            .map(|v| v.parse())
            .transpose()
            .context("invalid XY_HTTP_BIND socket address")?
            .unwrap_or_else(|| "127.0.0.1:8080".parse().expect("valid default address"));
        let list_limit_max = env::var("XY_LIST_LIMIT_MAX")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|&v| v > 0)
            .unwrap_or(200);
        let user_agent =
            env::var("XY_USER_AGENT").unwrap_or_else(|_| "codex-vibe-monitor/0.1.0".to_string());
        let static_dir = env::var("XY_STATIC_DIR")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                let default = PathBuf::from("web/dist");
                if default.exists() {
                    Some(default)
                } else {
                    None
                }
            });

        Ok(Self {
            base_url: Url::parse(&base_url).context("invalid XY_BASE_URL")?,
            quota_endpoint,
            cookie_name,
            cookie_value,
            database_path: database_path.into(),
            poll_interval,
            request_timeout,
            max_parallel_polls,
            shared_connection_parallelism,
            http_bind,
            list_limit_max,
            user_agent,
            static_dir,
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
