use std::{collections::HashSet, env, path::PathBuf, str::FromStr, time::Duration};

use anyhow::{Context, Result};
use dotenvy::dotenv;
use reqwest::{Client, Url, header};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{
    Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    dotenvy::from_filename(".env.local").ok();

    let config = AppConfig::from_env()?;
    let http = Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("codex-vibe-monitor/0.1.0")
        .build()
        .context("failed to construct HTTP client")?;

    let quota_records = fetch_quota(&http, &config).await?;

    if quota_records.is_empty() {
        println!("No records returned from quota endpoint; nothing to store.");
        return Ok(());
    }

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

    let inserted = persist_records(&pool, &quota_records).await?;
    println!(
        "Inserted {inserted} new record(s) out of {} fetched.",
        quota_records.len()
    );

    Ok(())
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

async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
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

async fn persist_records(pool: &SqlitePool, records: &[CodexRecord]) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let mut inserted = 0usize;

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

        inserted += result.rows_affected() as usize;
    }

    tx.commit().await?;
    Ok(inserted)
}

#[derive(Debug)]
struct AppConfig {
    base_url: Url,
    quota_endpoint: String,
    cookie_name: String,
    cookie_value: String,
    database_path: PathBuf,
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

        Ok(Self {
            base_url: Url::parse(&base_url).context("invalid XY_BASE_URL")?,
            quota_endpoint,
            cookie_name,
            cookie_value,
            database_path: database_path.into(),
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
