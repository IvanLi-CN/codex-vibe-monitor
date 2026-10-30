use super::*;

mod schema;
pub(crate) use schema::*;

const ANALYTICS_UNAVAILABLE_CODE: &str = "analytics_backend_unavailable";
const POSTGRES_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const REDACTED_SECRET: &str = "<redacted>";
pub(crate) const ANALYTICS_REPLICA_ID: &str = "pg-analytics";
pub(crate) const ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH: usize = 256;
pub(crate) const ANALYTICS_REPLICA_MAX_ENCODED_BATCH_BYTES: usize = 1024 * 1024;
pub(crate) const ANALYTICS_SOURCE_RETENTION_BOUND_BYTES: i64 = 256 * 1024 * 1024;
const ANALYTICS_REPLICA_MAX_BATCHES_PER_RUN: usize = 4096;

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
    async fn connect_raw(config: &AppConfig) -> Result<Self> {
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
        Ok(Self { pool })
    }

    async fn connect(config: &AppConfig) -> Result<Self> {
        let analytics_pool = Self::connect_raw(config).await?;
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

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsReplicaState {
    pub(crate) replica_id: String,
    pub(crate) status: String,
    pub(crate) generation: i64,
    pub(crate) baseline_generation: i64,
    pub(crate) baseline_change_id: i64,
    pub(crate) coverage_generation: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) coverage_watermark: i64,
    pub(crate) consumer_checkpoint: i64,
    pub(crate) source_acknowledged_through: i64,
    pub(crate) service_eligible: bool,
    pub(crate) last_error_code: Option<String>,
    pub(crate) last_error_detail: Option<String>,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Copy, FromRow)]
struct SourceChangeState {
    generation: i64,
    coverage_revision: i64,
    retained_from_change_id: i64,
    capacity: i64,
    invalidation_pending: i64,
}

#[derive(Debug, Clone, Copy, FromRow)]
struct SourceCoverage {
    generation: i64,
    covered_through_change_id: i64,
    coverage_revision: i64,
}

#[derive(Debug, Clone)]
struct SourceContext {
    state: SourceChangeState,
    latest_change_id: i64,
    retained_bytes: i64,
    coverage: Option<SourceCoverage>,
}

#[derive(Debug, Clone, FromRow)]
struct SourceChangeRow {
    change_id: i64,
    event_kind: String,
    entity_kind: String,
    source: String,
    source_row_id: Option<i64>,
    entity_key: String,
    generation: i64,
    coverage_revision: i64,
    fact_json: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalyticsSourceStatus {
    generation: i64,
    coverage_revision: i64,
    retained_from_change_id: i64,
    latest_change_id: i64,
    retained_bytes: i64,
    retention_bound_bytes: i64,
    capacity: i64,
    invalidation_pending: bool,
    coverage_generation: Option<i64>,
    covered_through_change_id: Option<i64>,
    coverage_revision_acknowledged: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalyticsReplicaStatusReport {
    schema_ready: bool,
    replica_id: String,
    service_eligible: bool,
    state: Option<AnalyticsReplicaState>,
    source: Option<AnalyticsSourceStatus>,
}

#[derive(Debug, Clone, FromRow)]
struct SourceInvocationSnapshotRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    occurred_at_epoch_ms: i64,
    stats_date: String,
    source: String,
    model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    cost_input: Option<f64>,
    cost_cache_write: Option<f64>,
    cost_cache_read: Option<f64>,
    cost_output: Option<f64>,
    cost_reasoning: Option<f64>,
    status: Option<String>,
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
    first_token_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
}

fn snapshot_payload(payload: Option<&str>) -> Option<Value> {
    payload
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
}

fn snapshot_text(payload: Option<&Value>, key: &str) -> Option<String> {
    payload
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn snapshot_i64(payload: Option<&Value>, key: &str) -> Option<i64> {
    payload.and_then(|value| value.get(key)).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
    })
}

fn snapshot_terminal(status: Option<&str>) -> bool {
    !matches!(
        status
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("running") | Some("pending")
    )
}

fn snapshot_success_like(status: Option<&str>) -> bool {
    matches!(
        status
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("success") | Some("succeeded") | Some("completed") | Some("ok")
    )
}

fn source_retention_exhausted(state: &AnalyticsReplicaState, source: &SourceContext) -> bool {
    let cursor_predates_retention =
        state.consumer_checkpoint.saturating_add(1) < source.state.retained_from_change_id;
    let generation_changed = state.generation != source.state.generation;
    cursor_predates_retention || generation_changed || source.state.invalidation_pending != 0
}

fn unconsumed_source_bytes_exceed_retention_bound(lag_bytes: i64) -> bool {
    lag_bytes > ANALYTICS_SOURCE_RETENTION_BOUND_BYTES
}

fn replica_service_eligible(state: &AnalyticsReplicaState) -> bool {
    state.status == "caught_up"
        && state.service_eligible
        && state.consumer_checkpoint == state.source_acknowledged_through
}

fn bounded_batch_end(encoded_sizes: &[usize], start: usize) -> Result<usize> {
    if start >= encoded_sizes.len() {
        return Ok(start);
    }
    let mut end = start;
    let mut encoded_bytes = 0usize;
    while end < encoded_sizes.len()
        && end.saturating_sub(start) < ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH
    {
        let fact_bytes = encoded_sizes[end];
        if fact_bytes > ANALYTICS_REPLICA_MAX_ENCODED_BATCH_BYTES {
            bail!(
                "oversized analytics replica fact at batch offset {}: {} encoded bytes exceeds {}",
                end.saturating_sub(start),
                fact_bytes,
                ANALYTICS_REPLICA_MAX_ENCODED_BATCH_BYTES
            );
        }
        if end > start
            && encoded_bytes.saturating_add(fact_bytes) > ANALYTICS_REPLICA_MAX_ENCODED_BATCH_BYTES
        {
            break;
        }
        encoded_bytes = encoded_bytes.saturating_add(fact_bytes);
        end += 1;
    }
    Ok(end)
}

async fn execute_postgres_schema(pool: &sqlx::PgPool) -> Result<()> {
    let mut transaction = pool
        .begin()
        .await
        .context("failed to start PostgreSQL analytics bootstrap transaction")?;
    for statement in PG_ANALYTICS_SCHEMA_SQL
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        sqlx::query(statement)
            .execute(&mut *transaction)
            .await
            .with_context(|| {
                format!("failed to execute PostgreSQL analytics bootstrap statement: {statement}")
            })?;
    }
    transaction
        .commit()
        .await
        .context("failed to commit PostgreSQL analytics bootstrap")?;
    Ok(())
}

async fn relation_exists(pool: &sqlx::PgPool, relation: &str) -> Result<bool> {
    let present: Option<String> = sqlx::query_scalar("SELECT to_regclass($1::text)::text")
        .bind(relation)
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to inspect PostgreSQL relation {relation}"))?;
    Ok(present.is_some())
}

async fn verify_replica_schema(pool: &sqlx::PgPool) -> Result<()> {
    for relation in PG_ANALYTICS_REQUIRED_RELATIONS
        .iter()
        .chain(PG_ANALYTICS_REPLICA_REQUIRED_RELATIONS.iter())
    {
        if !relation_exists(pool, relation).await? {
            bail!("required PostgreSQL analytics relation is missing: {relation}");
        }
    }
    Ok(())
}

async fn load_replica_state(pool: &sqlx::PgPool) -> Result<AnalyticsReplicaState> {
    sqlx::query_as::<_, AnalyticsReplicaState>(
        r#"
        SELECT replica_id, status, generation, baseline_generation, baseline_change_id,
               coverage_generation, coverage_revision, coverage_watermark,
               consumer_checkpoint, source_acknowledged_through, service_eligible,
               last_error_code, last_error_detail, updated_at::text AS updated_at
        FROM cvm_analytics.analytics_replica_state
        WHERE id = 1
        "#,
    )
    .fetch_one(pool)
    .await
    .context("failed to load PostgreSQL analytics replica state")
}

async fn load_source_context(
    source_pool: &Pool<Sqlite>,
    replica_id: &str,
) -> Result<SourceContext> {
    let state = sqlx::query_as::<_, SourceChangeState>(
        r#"
        SELECT generation, coverage_revision, retained_from_change_id, capacity,
               invalidation_pending
        FROM analytics_change_state
        WHERE id = 1
        "#,
    )
    .fetch_one(source_pool)
    .await
    .context("analytics source change state is not initialized")?;
    let latest_change_id = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(change_id), 0) FROM analytics_change_events",
    )
    .fetch_one(source_pool)
    .await
    .context("failed to load the latest analytics source change")?;
    let retained_bytes = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(length(CAST(fact_json AS BLOB))), 0) FROM analytics_change_events",
    )
    .fetch_one(source_pool)
    .await
    .context("failed to measure the retained analytics source journal")?;
    let coverage = sqlx::query_as::<_, SourceCoverage>(
        r#"
        SELECT generation, covered_through_change_id, coverage_revision
        FROM analytics_change_coverage
        WHERE replica_id = ?1
        "#,
    )
    .bind(replica_id)
    .fetch_optional(source_pool)
    .await
    .context("failed to load analytics source acknowledgement")?;
    Ok(SourceContext {
        state,
        latest_change_id,
        retained_bytes,
        coverage,
    })
}

async fn load_source_lag_bytes(source_pool: &Pool<Sqlite>, checkpoint: i64) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(length(CAST(fact_json AS BLOB))), 0) FROM analytics_change_events WHERE change_id > ?1",
    )
    .bind(checkpoint)
    .fetch_one(source_pool)
    .await
    .context("failed to measure unconsumed analytics source changes")
}

fn source_status(source: &SourceContext) -> AnalyticsSourceStatus {
    AnalyticsSourceStatus {
        generation: source.state.generation,
        coverage_revision: source.state.coverage_revision,
        retained_from_change_id: source.state.retained_from_change_id,
        latest_change_id: source.latest_change_id,
        retained_bytes: source.retained_bytes,
        retention_bound_bytes: ANALYTICS_SOURCE_RETENTION_BOUND_BYTES,
        capacity: source.state.capacity,
        invalidation_pending: source.state.invalidation_pending != 0,
        coverage_generation: source.coverage.map(|coverage| coverage.generation),
        covered_through_change_id: source
            .coverage
            .map(|coverage| coverage.covered_through_change_id),
        coverage_revision_acknowledged: source.coverage.map(|coverage| coverage.coverage_revision),
    }
}

async fn mark_replica_state(
    pool: &sqlx::PgPool,
    status: &str,
    service_eligible: bool,
    error_code: Option<&str>,
    error_detail: Option<&str>,
    projection_status: &str,
) -> Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET status = $1,
            service_eligible = $2,
            last_error_code = $3,
            last_error_detail = $4,
            updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(status)
    .bind(service_eligible)
    .bind(error_code)
    .bind(error_detail)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_state
        SET status = $1, last_error_code = $2, updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(projection_status)
    .bind(error_code)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn mark_replica_invalidated(
    pool: &sqlx::PgPool,
    error_detail: impl Into<String>,
) -> Result<()> {
    let detail = error_detail.into();
    mark_replica_state(
        pool,
        "invalidated",
        false,
        Some("source_retention_exhausted"),
        Some(&detail),
        "unavailable",
    )
    .await
}

async fn mark_replica_blocked(
    pool: &sqlx::PgPool,
    error_code: &str,
    error_detail: impl Into<String>,
) -> Result<()> {
    let detail = error_detail.into();
    mark_replica_state(
        pool,
        "blocked",
        false,
        Some(error_code),
        Some(&detail),
        "unavailable",
    )
    .await
}

async fn set_replica_running(pool: &sqlx::PgPool) -> Result<()> {
    mark_replica_state(pool, "running", false, None, None, "preparing").await
}

async fn set_replica_acknowledged(
    pool: &sqlx::PgPool,
    through_change_id: i64,
    caught_up: bool,
) -> Result<()> {
    let status = if caught_up { "caught_up" } else { "running" };
    let projection_status = if caught_up { "ready" } else { "preparing" };
    let mut transaction = pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET source_acknowledged_through = $1,
            status = $2,
            service_eligible = $3,
            last_error_code = NULL,
            last_error_detail = NULL,
            updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(through_change_id)
    .bind(status)
    .bind(caught_up)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE cvm_analytics.analytics_state SET status = $1, last_error_code = NULL, updated_at = NOW() WHERE id = 1",
    )
    .bind(projection_status)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn clear_replica_target(pool: &sqlx::PgPool, generation: i64) -> Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_replica_applied_changes")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_facts")
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET status = 'bootstrapping', generation = $1,
            baseline_generation = 0, baseline_change_id = 0,
            coverage_generation = 0, coverage_revision = 0,
            coverage_watermark = 0, consumer_checkpoint = 0,
            source_acknowledged_through = 0, service_eligible = FALSE,
            last_error_code = NULL, last_error_detail = NULL, updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(generation)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE cvm_analytics.analytics_state SET status = 'preparing', last_error_code = NULL, updated_at = NOW() WHERE id = 1",
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn set_replica_baseline(pool: &sqlx::PgPool, source: &SourceContext) -> Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET status = 'running', generation = $1,
            baseline_generation = $1, baseline_change_id = $2,
            coverage_generation = $1, coverage_revision = $3,
            coverage_watermark = $2, consumer_checkpoint = $2,
            source_acknowledged_through = 0, service_eligible = FALSE,
            last_error_code = NULL, last_error_detail = NULL, updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(source.state.generation)
    .bind(source.latest_change_id)
    .bind(source.state.coverage_revision)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE cvm_analytics.analytics_state SET status = 'preparing', last_error_code = NULL, updated_at = NOW() WHERE id = 1",
    )
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn load_source_changes(
    source_pool: &Pool<Sqlite>,
    after_change_id: i64,
) -> Result<Vec<SourceChangeRow>> {
    sqlx::query_as::<_, SourceChangeRow>(
        r#"
        SELECT change_id, event_kind, entity_kind, source, source_row_id,
               entity_key, generation, coverage_revision, fact_json
        FROM analytics_change_events
        WHERE change_id > ?1
        ORDER BY change_id
        LIMIT ?2
        "#,
    )
    .bind(after_change_id)
    .bind(i64::try_from(ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH)?)
    .fetch_all(source_pool)
    .await
    .context("failed to load the bounded analytics source batch")
}

async fn load_source_invocation_snapshot(source_pool: &Pool<Sqlite>) -> Result<Vec<AnalyticsFact>> {
    let rows = sqlx::query_as::<_, SourceInvocationSnapshotRow>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            CAST(COALESCE(ROUND((julianday(
                occurred_at,
                CASE WHEN instr(occurred_at, 'T') > 0 THEN '+0 hours' ELSE '-8 hours' END
            ) - 2440587.5) * 86400000.0), 0) AS INTEGER) AS occurred_at_epoch_ms,
            substr(occurred_at, 1, 10) AS stats_date,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            status,
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
            first_token_ms,
            t_resp_parse_ms
        FROM codex_invocations
        ORDER BY id
        "#,
    )
    .fetch_all(source_pool)
    .await
    .context("failed to load the SQLite analytics baseline snapshot")?;

    rows.into_iter()
        .map(|row| {
            let payload = snapshot_payload(row.payload.as_deref());
            let status = row.status.clone();
            Ok(AnalyticsFact {
                fact_version: 1,
                source: row.source,
                source_row_id: row.id,
                invoke_id: row.invoke_id,
                occurred_at: row.occurred_at,
                occurred_at_epoch_ms: row.occurred_at_epoch_ms,
                stats_date: row.stats_date,
                model: row.model,
                request_model: snapshot_text(payload.as_ref(), "requestModel"),
                response_model: snapshot_text(payload.as_ref(), "responseModel"),
                reasoning_effort: snapshot_text(payload.as_ref(), "reasoningEffort"),
                upstream_account_id: snapshot_i64(payload.as_ref(), "upstreamAccountId"),
                upstream_account_name: snapshot_text(payload.as_ref(), "upstreamAccountName"),
                status: status.clone(),
                success_like: snapshot_success_like(status.as_deref()),
                terminal: snapshot_terminal(status.as_deref()),
                failure_kind: row.failure_kind,
                failure_class: row.failure_class,
                is_actionable: row.is_actionable.map(|value| value != 0),
                input_tokens: row.input_tokens,
                output_tokens: row.output_tokens,
                cache_input_tokens: row.cache_input_tokens,
                reasoning_tokens: row.reasoning_tokens,
                total_tokens: row.total_tokens,
                cost: row.cost,
                cost_input: row.cost_input,
                cost_cache_write: row.cost_cache_write,
                cost_cache_read: row.cost_cache_read,
                cost_output: row.cost_output,
                cost_reasoning: row.cost_reasoning,
                t_total_ms: row.t_total_ms,
                t_req_read_ms: row.t_req_read_ms,
                t_req_parse_ms: row.t_req_parse_ms,
                t_upstream_connect_ms: row.t_upstream_connect_ms,
                t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
                t_upstream_stream_ms: row.t_upstream_stream_ms,
                first_token_ms: row.first_token_ms,
                t_resp_parse_ms: row.t_resp_parse_ms,
            })
        })
        .collect()
}

async fn upsert_analytics_fact(
    connection: &mut sqlx::postgres::PgConnection,
    fact: &AnalyticsFact,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO cvm_analytics.analytics_facts (
            fact_version, source, source_row_id, invoke_id, occurred_at,
            occurred_at_epoch_ms, stats_date, model, request_model, response_model,
            reasoning_effort, upstream_account_id, upstream_account_name, status,
            success_like, terminal, failure_kind, failure_class, is_actionable,
            input_tokens, output_tokens, cache_input_tokens, reasoning_tokens,
            total_tokens, cost, cost_input, cost_cache_write, cost_cache_read,
            cost_output, cost_reasoning, t_total_ms, t_req_read_ms, t_req_parse_ms,
            t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms,
            first_token_ms, t_resp_parse_ms
        )
        VALUES (
            $1, $2, $3, $4,
            to_timestamp($5::double precision / 1000.0), $5, $6::date, $7, $8, $9,
            $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21,
            $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32, $33,
            $34, $35, $36, $37
        )
        ON CONFLICT (source, source_row_id) DO UPDATE SET
            fact_version = EXCLUDED.fact_version,
            invoke_id = EXCLUDED.invoke_id,
            occurred_at = EXCLUDED.occurred_at,
            occurred_at_epoch_ms = EXCLUDED.occurred_at_epoch_ms,
            stats_date = EXCLUDED.stats_date,
            model = EXCLUDED.model,
            request_model = EXCLUDED.request_model,
            response_model = EXCLUDED.response_model,
            reasoning_effort = EXCLUDED.reasoning_effort,
            upstream_account_id = EXCLUDED.upstream_account_id,
            upstream_account_name = EXCLUDED.upstream_account_name,
            status = EXCLUDED.status,
            success_like = EXCLUDED.success_like,
            terminal = EXCLUDED.terminal,
            failure_kind = EXCLUDED.failure_kind,
            failure_class = EXCLUDED.failure_class,
            is_actionable = EXCLUDED.is_actionable,
            input_tokens = EXCLUDED.input_tokens,
            output_tokens = EXCLUDED.output_tokens,
            cache_input_tokens = EXCLUDED.cache_input_tokens,
            reasoning_tokens = EXCLUDED.reasoning_tokens,
            total_tokens = EXCLUDED.total_tokens,
            cost = EXCLUDED.cost,
            cost_input = EXCLUDED.cost_input,
            cost_cache_write = EXCLUDED.cost_cache_write,
            cost_cache_read = EXCLUDED.cost_cache_read,
            cost_output = EXCLUDED.cost_output,
            cost_reasoning = EXCLUDED.cost_reasoning,
            t_total_ms = EXCLUDED.t_total_ms,
            t_req_read_ms = EXCLUDED.t_req_read_ms,
            t_req_parse_ms = EXCLUDED.t_req_parse_ms,
            t_upstream_connect_ms = EXCLUDED.t_upstream_connect_ms,
            t_upstream_ttfb_ms = EXCLUDED.t_upstream_ttfb_ms,
            t_upstream_stream_ms = EXCLUDED.t_upstream_stream_ms,
            first_token_ms = EXCLUDED.first_token_ms,
            t_resp_parse_ms = EXCLUDED.t_resp_parse_ms
        "#,
    )
    .bind(fact.fact_version)
    .bind(&fact.source)
    .bind(fact.source_row_id)
    .bind(&fact.invoke_id)
    .bind(fact.occurred_at_epoch_ms)
    .bind(&fact.stats_date)
    .bind(&fact.model)
    .bind(&fact.request_model)
    .bind(&fact.response_model)
    .bind(&fact.reasoning_effort)
    .bind(fact.upstream_account_id)
    .bind(&fact.upstream_account_name)
    .bind(&fact.status)
    .bind(fact.success_like)
    .bind(fact.terminal)
    .bind(&fact.failure_kind)
    .bind(&fact.failure_class)
    .bind(fact.is_actionable)
    .bind(fact.input_tokens)
    .bind(fact.output_tokens)
    .bind(fact.cache_input_tokens)
    .bind(fact.reasoning_tokens)
    .bind(fact.total_tokens)
    .bind(fact.cost)
    .bind(fact.cost_input)
    .bind(fact.cost_cache_write)
    .bind(fact.cost_cache_read)
    .bind(fact.cost_output)
    .bind(fact.cost_reasoning)
    .bind(fact.t_total_ms)
    .bind(fact.t_req_read_ms)
    .bind(fact.t_req_parse_ms)
    .bind(fact.t_upstream_connect_ms)
    .bind(fact.t_upstream_ttfb_ms)
    .bind(fact.t_upstream_stream_ms)
    .bind(fact.first_token_ms)
    .bind(fact.t_resp_parse_ms)
    .execute(connection)
    .await
    .context("failed to upsert a PostgreSQL analytics fact")?;
    Ok(())
}

async fn delete_analytics_fact(
    connection: &mut sqlx::postgres::PgConnection,
    source: &str,
    source_row_id: i64,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM cvm_analytics.analytics_facts WHERE source = $1 AND source_row_id = $2",
    )
    .bind(source)
    .bind(source_row_id)
    .execute(connection)
    .await
    .context("failed to apply a PostgreSQL analytics tombstone")?;
    Ok(())
}

async fn apply_replica_change(
    connection: &mut sqlx::postgres::PgConnection,
    change: &SourceChangeRow,
) -> Result<()> {
    match (change.event_kind.as_str(), change.entity_kind.as_str()) {
        ("upsert", "invocation") => {
            let fact =
                serde_json::from_str::<AnalyticsFact>(&change.fact_json).with_context(|| {
                    format!(
                        "invalid invocation fact at source change {}",
                        change.change_id
                    )
                })?;
            anyhow::ensure!(
                fact.source == change.source && Some(fact.source_row_id) == change.source_row_id,
                "analytics fact identity disagrees with source change {}",
                change.change_id
            );
            upsert_analytics_fact(connection, &fact).await?;
        }
        ("tombstone", "invocation") => {
            let source_row_id = change
                .source_row_id
                .context("invocation tombstone is missing source row id")?;
            delete_analytics_fact(connection, &change.source, source_row_id).await?;
        }
        ("upsert", _) | ("coverage", _) => {}
        (event_kind, entity_kind) => {
            bail!(
                "unsupported analytics source change kind {event_kind}/{entity_kind} at {}",
                change.change_id
            );
        }
    }
    Ok(())
}

async fn record_applied_change(
    connection: &mut sqlx::postgres::PgConnection,
    change: &SourceChangeRow,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO cvm_analytics.analytics_replica_applied_changes (
            change_id, generation, coverage_revision, event_kind, entity_kind,
            source, source_row_id, entity_key, fact_bytes
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ON CONFLICT (change_id) DO NOTHING
        "#,
    )
    .bind(change.change_id)
    .bind(change.generation)
    .bind(change.coverage_revision)
    .bind(&change.event_kind)
    .bind(&change.entity_kind)
    .bind(&change.source)
    .bind(change.source_row_id)
    .bind(&change.entity_key)
    .bind(i32::try_from(change.fact_json.len())?)
    .execute(connection)
    .await
    .context("failed to record an applied PostgreSQL analytics source change")?;
    Ok(())
}

async fn apply_replica_batch(
    pool: &sqlx::PgPool,
    state: &AnalyticsReplicaState,
    source: &SourceContext,
    batch: &[SourceChangeRow],
) -> Result<i64> {
    let first_change_id = batch
        .first()
        .map(|change| change.change_id)
        .context("analytics replica cannot commit an empty batch")?;
    anyhow::ensure!(
        first_change_id == state.consumer_checkpoint.saturating_add(1),
        "analytics replica batch starts at {}, expected {}",
        first_change_id,
        state.consumer_checkpoint.saturating_add(1)
    );
    for change in batch {
        anyhow::ensure!(
            change.generation == source.state.generation,
            "analytics source change {} belongs to generation {}, expected {}",
            change.change_id,
            change.generation,
            source.state.generation
        );
    }
    let last_change_id = batch
        .last()
        .map(|change| change.change_id)
        .expect("non-empty analytics replica batch");
    let mut transaction = pool.begin().await?;
    for change in batch {
        apply_replica_change(&mut transaction, change).await?;
        record_applied_change(&mut transaction, change).await?;
    }
    let updated = sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET generation = $1,
            coverage_generation = $1,
            coverage_revision = $2,
            coverage_watermark = $3,
            consumer_checkpoint = $3,
            status = 'running',
            service_eligible = FALSE,
            last_error_code = NULL,
            last_error_detail = NULL,
            updated_at = NOW()
        WHERE id = 1 AND consumer_checkpoint = $4
        "#,
    )
    .bind(source.state.generation)
    .bind(source.state.coverage_revision)
    .bind(last_change_id)
    .bind(state.consumer_checkpoint)
    .execute(&mut *transaction)
    .await?;
    anyhow::ensure!(
        updated.rows_affected() == 1,
        "analytics replica checkpoint changed while applying source batch"
    );
    sqlx::query(
        "UPDATE cvm_analytics.analytics_state SET status = 'preparing', last_error_code = NULL, updated_at = NOW() WHERE id = 1",
    )
    .execute(&mut *transaction)
    .await?;
    transaction
        .commit()
        .await
        .context("failed to commit PostgreSQL analytics facts and checkpoint together")?;
    Ok(last_change_id)
}

async fn apply_snapshot_batch(pool: &sqlx::PgPool, facts: &[AnalyticsFact]) -> Result<()> {
    anyhow::ensure!(
        !facts.is_empty(),
        "analytics snapshot batch must not be empty"
    );
    let mut transaction = pool.begin().await?;
    for fact in facts {
        upsert_analytics_fact(&mut transaction, fact).await?;
    }
    transaction
        .commit()
        .await
        .context("failed to commit PostgreSQL analytics baseline batch")?;
    Ok(())
}

async fn acknowledge_source_batch(
    source_pool: &Pool<Sqlite>,
    source: &SourceContext,
    from_change_id: i64,
    through_change_id: i64,
) -> Result<()> {
    crate::analytics_changes::claim_analytics_change_coverage(
        source_pool,
        ANALYTICS_REPLICA_ID,
        source.state.generation,
        source.state.coverage_revision,
        from_change_id,
        through_change_id,
    )
    .await
    .map(|_| ())
    .context("failed to acknowledge the committed analytics replica batch")
}

async fn load_status_report(
    pool: &sqlx::PgPool,
    source_pool: &Pool<Sqlite>,
) -> Result<AnalyticsReplicaStatusReport> {
    let mut all_relations_ready = true;
    for relation in PG_ANALYTICS_REQUIRED_RELATIONS
        .iter()
        .chain(PG_ANALYTICS_REPLICA_REQUIRED_RELATIONS.iter())
    {
        if !relation_exists(pool, relation).await? {
            all_relations_ready = false;
        }
    }
    let source = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
    let mut state = if all_relations_ready {
        Some(load_replica_state(pool).await?)
    } else {
        None
    };
    let lag_bytes = if let Some(current_state) = state.as_ref()
        && current_state.status != "unbootstrapped"
    {
        load_source_lag_bytes(source_pool, current_state.consumer_checkpoint).await?
    } else {
        0
    };
    if let Some(current_state) = state.as_ref()
        && current_state.status != "unbootstrapped"
        && (source_retention_exhausted(current_state, &source)
            || unconsumed_source_bytes_exceed_retention_bound(lag_bytes))
    {
        let detail = format!(
            "source retention no longer covers consumer checkpoint {}: source generation={}, retained_from_change_id={}, invalidation_pending={}, unconsumed_bytes={}",
            current_state.consumer_checkpoint,
            source.state.generation,
            source.state.retained_from_change_id,
            source.state.invalidation_pending != 0,
            lag_bytes
        );
        mark_replica_invalidated(pool, detail).await?;
        state = Some(load_replica_state(pool).await?);
    }
    let service_eligible = state
        .as_ref()
        .map(replica_service_eligible)
        .unwrap_or(false);
    Ok(AnalyticsReplicaStatusReport {
        schema_ready: all_relations_ready,
        replica_id: ANALYTICS_REPLICA_ID.to_string(),
        service_eligible,
        state,
        source: Some(source_status(&source)),
    })
}

async fn snapshot_facts_into_target(pool: &sqlx::PgPool, facts: &[AnalyticsFact]) -> Result<()> {
    let encoded_sizes = facts
        .iter()
        .map(|fact| serde_json::to_vec(fact).map(|encoded| encoded.len()))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut offset = 0usize;
    while offset < facts.len() {
        let end = bounded_batch_end(&encoded_sizes, offset).map_err(|error| {
            anyhow!(
                "analytics baseline is blocked by an oversized fact at snapshot offset {}: {error}",
                offset
            )
        })?;
        apply_snapshot_batch(pool, &facts[offset..end]).await?;
        offset = end;
    }
    Ok(())
}

async fn bootstrap_replica(
    source_pool: &Pool<Sqlite>,
    pool: &sqlx::PgPool,
) -> Result<AnalyticsReplicaStatusReport> {
    execute_postgres_schema(pool).await?;
    verify_replica_schema(pool).await?;
    let source_before = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
    clear_replica_target(pool, source_before.state.generation).await?;
    let facts = match load_source_invocation_snapshot(source_pool).await {
        Ok(facts) => facts,
        Err(error) => {
            let detail = error.to_string();
            mark_replica_blocked(pool, "snapshot_failed", detail.clone()).await?;
            return Err(anyhow!(detail));
        }
    };
    if let Err(error) = snapshot_facts_into_target(pool, &facts).await {
        let detail = error.to_string();
        mark_replica_blocked(pool, "oversized_fact", detail.clone()).await?;
        return Err(anyhow!(detail));
    }
    let source_after = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
    if source_before.state.generation != source_after.state.generation
        || source_before.latest_change_id != source_after.latest_change_id
        || source_before.state.coverage_revision != source_after.state.coverage_revision
    {
        let detail = format!(
            "SQLite source changed while analytics replica baseline was being built: before generation={}, latest_change_id={}, coverage_revision={}; after generation={}, latest_change_id={}, coverage_revision={}",
            source_before.state.generation,
            source_before.latest_change_id,
            source_before.state.coverage_revision,
            source_after.state.generation,
            source_after.latest_change_id,
            source_after.state.coverage_revision,
        );
        mark_replica_blocked(pool, "source_changed_during_snapshot", detail.clone()).await?;
        return Err(anyhow!(detail));
    }
    set_replica_baseline(pool, &source_after).await?;
    if let Err(error) = crate::analytics_changes::acknowledge_analytics_change_generation(
        source_pool,
        ANALYTICS_REPLICA_ID,
        source_after.state.generation,
        source_after.latest_change_id,
    )
    .await
    {
        let detail = error.to_string();
        mark_replica_blocked(pool, "source_ack_failed", detail.clone()).await?;
        return Err(anyhow!(detail));
    }
    set_replica_acknowledged(pool, source_after.latest_change_id, true).await?;
    load_status_report(pool, source_pool).await
}

async fn resume_replica(
    source_pool: &Pool<Sqlite>,
    pool: &sqlx::PgPool,
) -> Result<AnalyticsReplicaStatusReport> {
    verify_replica_schema(pool).await?;
    let mut state = load_replica_state(pool).await?;
    anyhow::ensure!(
        state.status != "unbootstrapped",
        "analytics replica is not bootstrapped; run maintenance analytics-replica bootstrap"
    );
    anyhow::ensure!(
        state.status != "invalidated",
        "analytics replica generation is invalidated; run explicit bootstrap before resume"
    );
    if state.status == "paused" {
        // A paused replica is resumed by the same bounded drain below.
    } else if state.status == "blocked"
        && !matches!(state.last_error_code.as_deref(), Some("source_ack_failed"))
    {
        bail!(
            "analytics replica is blocked with {}: {}",
            state.last_error_code.as_deref().unwrap_or("unknown"),
            state
                .last_error_detail
                .as_deref()
                .unwrap_or("inspect status")
        );
    }
    set_replica_running(pool).await?;
    state = load_replica_state(pool).await?;

    for _ in 0..ANALYTICS_REPLICA_MAX_BATCHES_PER_RUN {
        let source = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
        let lag_bytes = load_source_lag_bytes(source_pool, state.consumer_checkpoint).await?;
        if source_retention_exhausted(&state, &source)
            || unconsumed_source_bytes_exceed_retention_bound(lag_bytes)
        {
            let detail = format!(
                "source retention exhausted before checkpoint {}: generation={}, retained_from_change_id={}, invalidation_pending={}, unconsumed_bytes={}",
                state.consumer_checkpoint,
                source.state.generation,
                source.state.retained_from_change_id,
                source.state.invalidation_pending != 0,
                lag_bytes
            );
            mark_replica_invalidated(pool, detail.clone()).await?;
            return Err(anyhow!(detail));
        }
        let from_change_id = state.consumer_checkpoint.saturating_add(1);
        let rows = load_source_changes(source_pool, state.consumer_checkpoint).await?;
        if rows.is_empty() {
            if let Err(error) = acknowledge_source_batch(
                source_pool,
                &source,
                from_change_id,
                source.latest_change_id,
            )
            .await
            {
                let detail = error.to_string();
                mark_replica_blocked(pool, "source_ack_failed", detail.clone()).await?;
                return Err(anyhow!(detail));
            }
            set_replica_acknowledged(pool, source.latest_change_id, true).await?;
            return load_status_report(pool, source_pool).await;
        }
        if rows[0].change_id != from_change_id {
            let detail = format!(
                "analytics source journal has a gap before change {}; run explicit bootstrap",
                rows[0].change_id
            );
            mark_replica_invalidated(pool, detail.clone()).await?;
            return Err(anyhow!(detail));
        }
        let encoded_sizes = rows
            .iter()
            .map(|change| change.fact_json.len())
            .collect::<Vec<_>>();
        let end = match bounded_batch_end(&encoded_sizes, 0) {
            Ok(end) => end,
            Err(error) => {
                let detail = format!(
                    "analytics source change {} is a diagnosable oversized fact: {error}",
                    rows[0].change_id
                );
                mark_replica_blocked(pool, "oversized_fact", detail.clone()).await?;
                return Err(anyhow!(detail));
            }
        };
        let batch = &rows[..end];
        let last_change_id = match apply_replica_batch(pool, &state, &source, batch).await {
            Ok(change_id) => change_id,
            Err(error) => {
                let detail = error.to_string();
                mark_replica_blocked(pool, "target_apply_failed", detail.clone()).await?;
                return Err(anyhow!(detail));
            }
        };
        if let Err(error) =
            acknowledge_source_batch(source_pool, &source, from_change_id, last_change_id).await
        {
            let detail = error.to_string();
            mark_replica_blocked(pool, "source_ack_failed", detail.clone()).await?;
            return Err(anyhow!(detail));
        }
        let caught_up = last_change_id >= source.latest_change_id;
        set_replica_acknowledged(pool, last_change_id, caught_up).await?;
        state = load_replica_state(pool).await?;
        if caught_up {
            return load_status_report(pool, source_pool).await;
        }
    }
    let detail = format!(
        "analytics replica worker stopped after {} bounded batches with checkpoint {}",
        ANALYTICS_REPLICA_MAX_BATCHES_PER_RUN, state.consumer_checkpoint
    );
    mark_replica_blocked(pool, "worker_batch_limit", detail.clone()).await?;
    Err(anyhow!(detail))
}

async fn pause_replica(
    source_pool: &Pool<Sqlite>,
    pool: &sqlx::PgPool,
) -> Result<AnalyticsReplicaStatusReport> {
    verify_replica_schema(pool).await?;
    let state = load_replica_state(pool).await?;
    anyhow::ensure!(
        state.status != "unbootstrapped",
        "analytics replica is not bootstrapped; run maintenance analytics-replica bootstrap"
    );
    anyhow::ensure!(
        state.status != "invalidated",
        "analytics replica generation is invalidated; run explicit bootstrap before pause"
    );
    mark_replica_state(pool, "paused", false, None, None, "preparing").await?;
    load_status_report(pool, source_pool).await
}

async fn verify_replica(
    source_pool: &Pool<Sqlite>,
    pool: &sqlx::PgPool,
) -> Result<AnalyticsReplicaStatusReport> {
    verify_replica_schema(pool).await?;
    let mut state = load_replica_state(pool).await?;
    let source = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
    let mut errors = Vec::new();
    let lag_bytes = load_source_lag_bytes(source_pool, state.consumer_checkpoint).await?;
    if state.status != "unbootstrapped"
        && (source_retention_exhausted(&state, &source)
            || unconsumed_source_bytes_exceed_retention_bound(lag_bytes))
    {
        let detail = format!(
            "source retention exhausted before checkpoint {} (unconsumed_bytes={})",
            state.consumer_checkpoint, lag_bytes
        );
        mark_replica_invalidated(pool, detail.clone()).await?;
        state = load_replica_state(pool).await?;
        errors.push(detail);
    }
    if state.status != "caught_up" {
        errors.push(format!("replica status is {}", state.status));
    }
    if !state.service_eligible {
        errors.push("replica is not service eligible".to_string());
    }
    if state.consumer_checkpoint != state.source_acknowledged_through {
        errors.push(format!(
            "target checkpoint {} differs from source acknowledgement {}",
            state.consumer_checkpoint, state.source_acknowledged_through
        ));
    }
    if source.state.invalidation_pending != 0 {
        errors.push("source generation still has pending invalidation".to_string());
    }
    match source.coverage {
        Some(coverage)
            if coverage.generation == state.generation
                && coverage.covered_through_change_id >= state.consumer_checkpoint => {}
        Some(coverage) => errors.push(format!(
            "source acknowledgement is generation {} through {}, expected generation {} through at least {}",
            coverage.generation,
            coverage.covered_through_change_id,
            state.generation,
            state.consumer_checkpoint
        )),
        None => errors.push("source acknowledgement row is missing".to_string()),
    }
    let report = AnalyticsReplicaStatusReport {
        schema_ready: true,
        replica_id: ANALYTICS_REPLICA_ID.to_string(),
        service_eligible: replica_service_eligible(&state),
        state: Some(state),
        source: Some(source_status(&source)),
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    if errors.is_empty() {
        Ok(report)
    } else {
        bail!(
            "analytics replica verification failed: {}",
            errors.join("; ")
        )
    }
}

pub(crate) async fn pg_analytics_replica_control(
    config: &AppConfig,
    source_pool: &Pool<Sqlite>,
    command: AnalyticsReplicaCommand,
) -> Result<()> {
    let postgres = PgAnalyticsPool::connect_raw(config).await?;
    match command {
        AnalyticsReplicaCommand::Bootstrap => {
            let report = bootstrap_replica(source_pool, postgres.inner()).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        AnalyticsReplicaCommand::Status => {
            let report = load_status_report(postgres.inner(), source_pool).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        AnalyticsReplicaCommand::Pause => {
            let report = pause_replica(source_pool, postgres.inner()).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        AnalyticsReplicaCommand::Resume => {
            let report = resume_replica(source_pool, postgres.inner()).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        AnalyticsReplicaCommand::Verify => {
            verify_replica(source_pool, postgres.inner()).await?;
        }
    }
    Ok(())
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
        assert_eq!(PG_ANALYTICS_REPLICA_REQUIRED_RELATIONS.len(), 2);
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

    #[test]
    fn replica_batch_bounds_are_deterministic() {
        let sizes = vec![300_000; 5];
        assert_eq!(bounded_batch_end(&sizes, 0).expect("bounded batch"), 3);

        let sizes = vec![1; ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH + 1];
        assert_eq!(
            bounded_batch_end(&sizes, 0).expect("fact-count bounded batch"),
            ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH
        );

        let error = bounded_batch_end(&[ANALYTICS_REPLICA_MAX_ENCODED_BATCH_BYTES + 1], 0)
            .expect_err("oversized fact must block the worker");
        assert!(
            error
                .to_string()
                .contains("oversized analytics replica fact")
        );
    }

    #[test]
    fn source_retention_bound_is_exclusive_and_deterministic() {
        assert!(!unconsumed_source_bytes_exceed_retention_bound(
            ANALYTICS_SOURCE_RETENTION_BOUND_BYTES
        ));
        assert!(unconsumed_source_bytes_exceed_retention_bound(
            ANALYTICS_SOURCE_RETENTION_BOUND_BYTES + 1
        ));
    }

    #[test]
    fn source_retention_exhaustion_is_generation_and_cursor_bound() {
        let state = AnalyticsReplicaState {
            replica_id: ANALYTICS_REPLICA_ID.to_string(),
            status: "caught_up".to_string(),
            generation: 3,
            baseline_generation: 3,
            baseline_change_id: 10,
            coverage_generation: 3,
            coverage_revision: 11,
            coverage_watermark: 10,
            consumer_checkpoint: 10,
            source_acknowledged_through: 10,
            service_eligible: true,
            last_error_code: None,
            last_error_detail: None,
            updated_at: String::new(),
        };
        let source = SourceContext {
            state: SourceChangeState {
                generation: 3,
                coverage_revision: 12,
                retained_from_change_id: 11,
                capacity: 10_000,
                invalidation_pending: 0,
            },
            latest_change_id: 12,
            retained_bytes: 128,
            coverage: None,
        };
        assert!(!source_retention_exhausted(&state, &source));

        let mut exhausted = source.clone();
        exhausted.state.retained_from_change_id = 12;
        assert!(source_retention_exhausted(&state, &exhausted));

        let mut next_generation = source;
        next_generation.state.generation = 4;
        assert!(source_retention_exhausted(&state, &next_generation));
    }

    #[test]
    fn interrupted_batch_cannot_be_service_eligible_before_source_ack() {
        let mut state = AnalyticsReplicaState {
            replica_id: ANALYTICS_REPLICA_ID.to_string(),
            status: "running".to_string(),
            generation: 1,
            baseline_generation: 1,
            baseline_change_id: 4,
            coverage_generation: 1,
            coverage_revision: 8,
            coverage_watermark: 7,
            consumer_checkpoint: 7,
            source_acknowledged_through: 4,
            service_eligible: false,
            last_error_code: None,
            last_error_detail: None,
            updated_at: String::new(),
        };
        assert!(!replica_service_eligible(&state));

        state.status = "caught_up".to_string();
        state.source_acknowledged_through = 7;
        state.service_eligible = true;
        assert!(replica_service_eligible(&state));
    }
}
