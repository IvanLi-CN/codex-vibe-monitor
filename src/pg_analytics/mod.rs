use super::*;

mod schema;
pub(crate) use schema::*;

const ANALYTICS_UNAVAILABLE_CODE: &str = "analytics_backend_unavailable";
const POSTGRES_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const REDACTED_SECRET: &str = "<redacted>";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AnalyticsBackend {
    #[default]
    Sqlite,
    Postgres,
}

impl AnalyticsBackend {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }
}

impl fmt::Display for AnalyticsBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for AnalyticsBackend {
    type Err = anyhow::Error;

    fn from_str(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "sqlite" => Ok(Self::Sqlite),
            "postgres" => Ok(Self::Postgres),
            _ => bail!("invalid {ENV_ANALYTICS_BACKEND} value; expected sqlite or postgres"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PostgresDatabaseUrl(String);

impl PostgresDatabaseUrl {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("{ENV_POSTGRES_DATABASE_URL} must not be empty");
        }
        let parsed = Url::parse(trimmed)
            .map_err(|_| anyhow!("invalid {ENV_POSTGRES_DATABASE_URL} value"))?;
        if !matches!(parsed.scheme(), "postgres" | "postgresql") {
            bail!("{ENV_POSTGRES_DATABASE_URL} must use a postgres:// or postgresql:// scheme");
        }
        Ok(Self(trimmed.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PostgresDatabaseUrl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("PostgresDatabaseUrl")
            .field(&REDACTED_SECRET)
            .finish()
    }
}

impl Serialize for PostgresDatabaseUrl {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(REDACTED_SECRET)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AnalyticsAvailability {
    Disabled,
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsUnavailable {
    pub(crate) backend: AnalyticsBackend,
    pub(crate) reason: String,
}

impl AnalyticsUnavailable {
    pub(crate) fn new(backend: AnalyticsBackend, reason: impl Into<String>) -> Self {
        Self {
            backend,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for AnalyticsUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "analytics backend {} unavailable: {}",
            self.backend, self.reason
        )
    }
}

impl std::error::Error for AnalyticsUnavailable {}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsRuntimeStatus {
    pub(crate) backend: AnalyticsBackend,
    pub(crate) availability: AnalyticsAvailability,
    pub(crate) reason: Option<String>,
}

#[derive(Debug)]
pub(crate) struct PgAnalyticsPool {
    pool: sqlx::PgPool,
}

impl PgAnalyticsPool {
    async fn connect(config: &AppConfig) -> Result<Self> {
        let database_url = config
            .postgres_database_url
            .as_ref()
            .context("PostgreSQL analytics URL is not configured")?;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(config.postgres_max_connections)
            .acquire_timeout(POSTGRES_CONNECTION_TIMEOUT)
            .connect(database_url.as_str())
            .await
            .context("failed to connect to the PostgreSQL analytics backend")?;
        let analytics_pool = Self { pool };
        analytics_pool
            .verify_schema()
            .await
            .context("PostgreSQL analytics schema verification failed")?;
        analytics_pool
            .verify_read_transaction()
            .await
            .context("PostgreSQL analytics read transaction verification failed")?;
        Ok(analytics_pool)
    }

    async fn verify_schema(&self) -> Result<()> {
        for relation in PG_ANALYTICS_REQUIRED_RELATIONS {
            let present: Option<String> = sqlx::query_scalar("SELECT to_regclass($1::text)::text")
                .bind(relation)
                .fetch_one(&self.pool)
                .await
                .with_context(|| format!("failed to inspect required relation {relation}"))?;
            if present.is_none() {
                bail!("required PostgreSQL analytics relation is missing: {relation}");
            }
        }
        Ok(())
    }

    async fn verify_read_transaction(&self) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SELECT 1").execute(&mut *transaction).await?;
        transaction.rollback().await?;
        Ok(())
    }

    pub(crate) fn inner(&self) -> &sqlx::PgPool {
        &self.pool
    }
}

#[derive(Debug)]
pub(crate) struct PgAnalyticsRuntime {
    backend: AnalyticsBackend,
    pool: Option<PgAnalyticsPool>,
    unavailable: Option<AnalyticsUnavailable>,
}

impl PgAnalyticsRuntime {
    pub(crate) fn disabled() -> Self {
        Self {
            backend: AnalyticsBackend::Sqlite,
            pool: None,
            unavailable: None,
        }
    }

    pub(crate) async fn initialize(config: &AppConfig) -> Self {
        if config.analytics_backend == AnalyticsBackend::Sqlite {
            return Self::disabled();
        }

        match PgAnalyticsPool::connect(config).await {
            Ok(pool) => Self {
                backend: config.analytics_backend,
                pool: Some(pool),
                unavailable: None,
            },
            Err(error) => {
                let reason = sanitize_postgres_error(
                    &error,
                    config
                        .postgres_database_url
                        .as_ref()
                        .map(PostgresDatabaseUrl::as_str),
                );
                Self {
                    backend: config.analytics_backend,
                    pool: None,
                    unavailable: Some(AnalyticsUnavailable::new(config.analytics_backend, reason)),
                }
            }
        }
    }

    pub(crate) fn backend(&self) -> AnalyticsBackend {
        self.backend
    }

    pub(crate) fn status(&self) -> AnalyticsRuntimeStatus {
        let (availability, reason) = if let Some(unavailable) = &self.unavailable {
            (
                AnalyticsAvailability::Unavailable,
                Some(unavailable.reason.clone()),
            )
        } else if self.pool.is_some() {
            (AnalyticsAvailability::Ready, None)
        } else {
            (AnalyticsAvailability::Disabled, None)
        };
        AnalyticsRuntimeStatus {
            backend: self.backend,
            availability,
            reason,
        }
    }

    pub(crate) fn unavailable_reason(&self) -> Option<&AnalyticsUnavailable> {
        self.unavailable.as_ref()
    }

    pub(crate) fn require_pool(
        &self,
    ) -> std::result::Result<&PgAnalyticsPool, AnalyticsUnavailable> {
        if let Some(pool) = &self.pool {
            return Ok(pool);
        }
        Err(self.unavailable.clone().unwrap_or_else(|| {
            AnalyticsUnavailable::new(
                self.backend,
                "analytics backend is disabled; no PostgreSQL read path is available",
            )
        }))
    }
}

fn sanitize_postgres_error(error: &anyhow::Error, database_url: Option<&str>) -> String {
    let mut reason = format!("{error:#}");
    if let Some(database_url) = database_url.filter(|value| !value.is_empty()) {
        reason = reason.replace(database_url, REDACTED_SECRET);
        if let Ok(parsed) = Url::parse(database_url) {
            for sensitive_part in [Some(parsed.username()), parsed.password()] {
                if let Some(sensitive_part) = sensitive_part.filter(|value| !value.is_empty()) {
                    reason = reason.replace(sensitive_part, REDACTED_SECRET);
                }
            }
        }
    }
    static POSTGRES_USERINFO: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)(postgres(?:ql)?://)[^/\s?#@]+@").expect("valid postgres URL regex")
    });
    let redacted_userinfo = format!("$1{REDACTED_SECRET}@");
    reason = POSTGRES_USERINFO
        .replace_all(&reason, redacted_userinfo.as_str())
        .into_owned();
    if reason.len() > 512 {
        reason.truncate(512);
        reason.push_str("...");
    }
    reason
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsFact {
    pub(crate) fact_version: i16,
    pub(crate) source: String,
    pub(crate) source_row_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) occurred_at_epoch_ms: i64,
    pub(crate) stats_date: String,
    pub(crate) model: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) response_model: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) success_like: bool,
    pub(crate) terminal: bool,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) is_actionable: Option<bool>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_input: Option<f64>,
    pub(crate) cost_cache_write: Option<f64>,
    pub(crate) cost_cache_read: Option<f64>,
    pub(crate) cost_output: Option<f64>,
    pub(crate) cost_reasoning: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsMetrics {
    pub(crate) calls: i64,
    pub(crate) tokens: Option<i64>,
    pub(crate) token_samples: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_samples: i64,
    pub(crate) usage_time_ms: Option<f64>,
    pub(crate) usage_time_samples: i64,
    pub(crate) wall_time_ms: Option<f64>,
    pub(crate) wall_time_samples: i64,
    pub(crate) output_speed_tokens_per_second: Option<f64>,
    pub(crate) output_speed_samples: i64,
    pub(crate) first_byte_ms: Option<f64>,
    pub(crate) first_byte_samples: i64,
    pub(crate) response_ms: Option<f64>,
    pub(crate) response_samples: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AnalyticsRange {
    #[serde(rename = "7d")]
    Seven,
    #[serde(rename = "30d")]
    Thirty,
    #[serde(rename = "180d")]
    OneEighty,
    #[serde(rename = "365d")]
    ThreeSixtyFive,
}

impl AnalyticsRange {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "7d" => Some(Self::Seven),
            "30d" => Some(Self::Thirty),
            "180d" => Some(Self::OneEighty),
            "365d" => Some(Self::ThreeSixtyFive),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Seven => "7d",
            Self::Thirty => "30d",
            Self::OneEighty => "180d",
            Self::ThreeSixtyFive => "365d",
        }
    }

    pub(crate) fn days(self) -> i64 {
        match self {
            Self::Seven => 7,
            Self::Thirty => 30,
            Self::OneEighty => 180,
            Self::ThreeSixtyFive => 365,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AnalyticsDimension {
    Overall,
    Model,
    Upstream,
}

impl AnalyticsDimension {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Overall => "overall",
            Self::Model => "model",
            Self::Upstream => "upstream",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsOverviewQuery {
    pub(crate) range: AnalyticsRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsSeriesQuery {
    pub(crate) range: AnalyticsRange,
    pub(crate) dimension: AnalyticsDimension,
    #[serde(default)]
    pub(crate) keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsProjectionState {
    pub(crate) status: String,
    pub(crate) statistics_start_date: Option<String>,
    pub(crate) processed_rows: i64,
    pub(crate) total_rows: i64,
    pub(crate) last_error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsDailyPoint {
    pub(crate) date: String,
    #[serde(flatten)]
    pub(crate) metrics: AnalyticsMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsSeriesSummary {
    pub(crate) series_key: String,
    pub(crate) display_name: String,
    pub(crate) reasoning_effort: Option<String>,
    #[serde(flatten)]
    pub(crate) metrics: AnalyticsMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsSeries {
    pub(crate) series_key: String,
    pub(crate) display_name: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) points: Vec<AnalyticsDailyPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsOverview {
    pub(crate) state: AnalyticsProjectionState,
    pub(crate) timezone: String,
    pub(crate) range: String,
    pub(crate) global: AnalyticsMetrics,
    pub(crate) daily: Vec<AnalyticsDailyPoint>,
    pub(crate) models: Vec<AnalyticsSeriesSummary>,
    pub(crate) upstreams: Vec<AnalyticsSeriesSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsSeriesResponse {
    pub(crate) state: AnalyticsProjectionState,
    pub(crate) timezone: String,
    pub(crate) range: String,
    pub(crate) dimension: AnalyticsDimension,
    pub(crate) series: Vec<AnalyticsSeries>,
}

#[derive(Debug)]
pub(crate) enum AnalyticsReadError {
    Unavailable(AnalyticsUnavailable),
    InvalidQuery(String),
    Internal(anyhow::Error),
}

impl fmt::Display for AnalyticsReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(error) => error.fmt(formatter),
            Self::InvalidQuery(error) => formatter.write_str(error),
            Self::Internal(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AnalyticsReadError {}

impl AnalyticsReadError {
    pub(crate) fn into_api_error(self) -> ApiError {
        match self {
            Self::Unavailable(error) => {
                ApiError::unavailable_with_code(ANALYTICS_UNAVAILABLE_CODE, error)
            }
            Self::InvalidQuery(error) => ApiError::bad_request(anyhow!(error)),
            Self::Internal(error) => ApiError::Internal(error),
        }
    }
}

pub(crate) type AnalyticsReadResult<T> = std::result::Result<T, AnalyticsReadError>;

#[async_trait::async_trait]
pub(crate) trait AnalyticsReadStore: Send + Sync {
    async fn overview(
        &self,
        query: AnalyticsOverviewQuery,
    ) -> AnalyticsReadResult<AnalyticsOverview>;
    async fn series(
        &self,
        query: AnalyticsSeriesQuery,
    ) -> AnalyticsReadResult<AnalyticsSeriesResponse>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_url_debug_and_json_are_redacted() {
        let url = PostgresDatabaseUrl::parse(
            "postgresql://analytics:secret@example.test:5432/codex?sslmode=require",
        )
        .expect("valid postgres URL");
        let debug = format!("{url:?}");
        let json = serde_json::to_string(&url).expect("serializable URL");
        assert!(!debug.contains("secret"));
        assert!(!json.contains("secret"));
        assert_eq!(json, "\"<redacted>\"");
    }

    #[test]
    fn backend_and_range_contracts_are_explicit() {
        assert_eq!(AnalyticsBackend::default(), AnalyticsBackend::Sqlite);
        assert!(matches!(
            AnalyticsBackend::from_str("postgres"),
            Ok(AnalyticsBackend::Postgres)
        ));
        assert_eq!(AnalyticsRange::parse("30d").expect("range").days(), 30);
        assert_eq!(AnalyticsDimension::Model.as_str(), "model");
        assert_eq!(
            serde_json::to_string(&AnalyticsRange::OneEighty).expect("range serializes"),
            "\"180d\""
        );
    }

    #[test]
    fn postgres_error_sanitization_removes_the_full_connection_string() {
        let database_url = "postgresql://analytics:secret@example.test/codex";
        let error = anyhow!(
            "connection failed for {database_url}; normalized postgres://analytics:secret@example.test/codex; password=secret"
        );
        let sanitized = sanitize_postgres_error(&error, Some(database_url));
        assert!(!sanitized.contains("secret"));
        assert!(sanitized.contains(REDACTED_SECRET));
    }

    #[test]
    fn disabled_runtime_does_not_expose_a_pool() {
        let runtime = PgAnalyticsRuntime::disabled();
        assert_eq!(
            runtime.status().availability,
            AnalyticsAvailability::Disabled
        );
        assert!(runtime.require_pool().is_err());
    }

    #[test]
    fn schema_contract_is_separate_from_runtime_verification() {
        assert_eq!(PG_ANALYTICS_REQUIRED_RELATIONS.len(), 4);
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("CREATE SCHEMA IF NOT EXISTS cvm_analytics"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_facts"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_daily_rollups"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_hourly_rollups"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_state"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("occurred_at_epoch_ms BIGINT NOT NULL"));
        assert!(!PG_ANALYTICS_SCHEMA_SQL.contains("payload"));
        assert!(!PG_ANALYTICS_SCHEMA_SQL.contains("requester_ip"));
        assert!(!PG_ANALYTICS_SCHEMA_SQL.contains("prompt_cache"));
        assert!(!PG_ANALYTICS_SCHEMA_SQL.contains("last_error TEXT"));
    }
}
