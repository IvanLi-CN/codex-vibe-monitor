use super::*;
use percent_encoding::percent_decode_str;

mod materializer;
mod migrations;
mod schema;
pub(crate) use schema::*;
mod read;
pub(crate) use read::*;

const ANALYTICS_UNAVAILABLE_CODE: &str = "analytics_backend_unavailable";
const POSTGRES_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const ANALYTICS_REPLICA_CONTROL_LOCK_KEY: i64 = 0x4356_4d5f_4354_524c;
const POSTGRES_MIGRATION_LOCK_KEY: i64 = 0x0043_564d_5f4d_4947;
const REDACTED_SECRET: &str = "<redacted>";
pub(crate) const ANALYTICS_REPLICA_ID: &str = "pg-analytics";
pub(crate) const ANALYTICS_FACT_DATE_SEMANTICS_VERSION: i64 =
    crate::analytics_changes::ANALYTICS_SOURCE_FACT_DATE_SEMANTICS_VERSION;
pub(crate) const ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH: usize = 256;
pub(crate) const ANALYTICS_REPLICA_MAX_ENCODED_BATCH_BYTES: usize = 1024 * 1024;
pub(crate) const ANALYTICS_SOURCE_RETENTION_BOUND_BYTES: i64 = 256 * 1024 * 1024;
const ANALYTICS_REPLICA_BOOTSTRAP_LOCK_KEY: i64 = 0x4356_4d5f_5047_414c;
const PG_ANALYTICS_BOOTSTRAP_LOCK_ACQUIRE_SQL: &str = "SELECT pg_try_advisory_lock($1)";
const PG_ANALYTICS_BOOTSTRAP_LOCK_RELEASE_SQL: &str = "SELECT pg_advisory_unlock($1)";
const PG_ANALYTICS_READ_TRANSACTION_SQL: &str =
    "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY";
const ANALYTICS_REPLICA_MAX_SOURCE_TEXT_BYTES: i64 = 64 * 1024;
const ANALYTICS_REPLICA_MAX_SOURCE_PAYLOAD_BYTES: i64 = 4 * 1024 * 1024;
const ANALYTICS_REPLICA_MAX_BATCHES_PER_RUN: usize = 4096;
const ANALYTICS_BOOTSTRAP_RETRY_INITIAL: Duration = Duration::from_secs(1);
const ANALYTICS_BOOTSTRAP_RETRY_MAX: Duration = Duration::from_secs(60);
const ANALYTICS_OWNER_WRITER_ID: &str = "pg-analytics-startup";

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
        Self::parse_for(raw, ENV_POSTGRES_DATABASE_URL)
    }

    pub(crate) fn parse_for(raw: &str, env_name: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            bail!("{env_name} must not be empty");
        }
        let parsed = Url::parse(trimmed).map_err(|_| anyhow!("invalid {env_name} value"))?;
        if !matches!(parsed.scheme(), "postgres" | "postgresql") {
            bail!("{env_name} must use a postgres:// or postgresql:// scheme");
        }
        Ok(Self(trimmed.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn effective_username(&self) -> Result<String> {
        let parsed =
            Url::parse(&self.0).expect("PostgresDatabaseUrl was validated during construction");
        let mut username = percent_decode_str(parsed.username())
            .decode_utf8()
            .map_err(|_| anyhow!("invalid PostgreSQL login role encoding"))?
            .into_owned();
        for (key, value) in parsed.query_pairs() {
            if key == "user" {
                username = value.into_owned();
            }
        }
        Ok(username)
    }
}

pub(crate) fn validate_postgres_role_separation(
    application_url: Option<&PostgresDatabaseUrl>,
    maintenance_url: Option<&PostgresDatabaseUrl>,
) -> Result<()> {
    let Some((application_url, maintenance_url)) = application_url.zip(maintenance_url) else {
        return Ok(());
    };
    if application_url.as_str() == maintenance_url.as_str() {
        bail!(
            "{ENV_POSTGRES_DATABASE_URL} and {ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL} must not be identical"
        );
    }
    let application_role = application_url.effective_username().with_context(|| {
        format!("{ENV_POSTGRES_DATABASE_URL} must specify a valid PostgreSQL login role")
    })?;
    let maintenance_role = maintenance_url.effective_username().with_context(|| {
        format!(
            "{ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL} must specify a valid PostgreSQL login role"
        )
    })?;
    if application_role.is_empty() || maintenance_role.is_empty() {
        bail!(
            "{ENV_POSTGRES_DATABASE_URL} and {ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL} must specify distinct PostgreSQL login roles"
        );
    }
    if application_role == maintenance_role {
        bail!(
            "{ENV_POSTGRES_DATABASE_URL} and {ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL} must use different PostgreSQL login roles"
        );
    }
    Ok(())
}

pub(crate) fn validate_postgres_migration_role_separation(
    migration_url: Option<&PostgresDatabaseUrl>,
    application_url: Option<&PostgresDatabaseUrl>,
    maintenance_url: Option<&PostgresDatabaseUrl>,
) -> Result<()> {
    let Some(migration_url) = migration_url else {
        return Ok(());
    };
    let migration_role = migration_url.effective_username()?;
    if migration_role.is_empty() {
        bail!("{ENV_POSTGRES_MIGRATION_URL} must specify a PostgreSQL login role");
    }
    for (name, url) in [
        (ENV_POSTGRES_DATABASE_URL, application_url),
        (ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL, maintenance_url),
    ] {
        if let Some(url) = url
            && (migration_url.as_str() == url.as_str()
                || migration_role == url.effective_username()?)
        {
            bail!("{ENV_POSTGRES_MIGRATION_URL} must use a distinct login role from {name}");
        }
    }
    Ok(())
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
    async fn connect_migration(config: &AppConfig) -> Result<Self> {
        let database_url = config
            .postgres_migration_url
            .as_ref()
            .context("POSTGRES_MIGRATION_URL is required for PostgreSQL schema migration")?;
        Self::connect_url(
            database_url,
            1,
            "failed to connect to the PostgreSQL migration backend",
        )
        .await
    }
    async fn connect_raw(config: &AppConfig) -> Result<Self> {
        let database_url = config
            .postgres_database_url
            .as_ref()
            .context("PostgreSQL analytics URL is not configured")?;
        Self::connect_url(
            database_url,
            config.postgres_max_connections,
            "failed to connect to the PostgreSQL analytics backend",
        )
        .await
    }

    async fn connect_maintenance(config: &AppConfig) -> Result<Self> {
        let database_url = config.postgres_analytics_maintenance_url.as_ref().context(
            "POSTGRES_ANALYTICS_MAINTENANCE_URL is required for analytics replica control",
        )?;
        validate_postgres_role_separation(
            config.postgres_database_url.as_ref(),
            Some(database_url),
        )?;
        Self::connect_url(
            database_url,
            config.postgres_max_connections,
            "failed to connect to the PostgreSQL analytics maintenance backend",
        )
        .await
        .map_err(|error| {
            anyhow!(
                "failed to connect to the PostgreSQL analytics maintenance backend: {}",
                sanitize_postgres_error(&error, Some(database_url.as_str()))
            )
        })
    }

    async fn connect_url(
        database_url: &PostgresDatabaseUrl,
        max_connections: u32,
        error_context: &str,
    ) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(POSTGRES_CONNECTION_TIMEOUT)
            .connect(database_url.as_str())
            .await
            .with_context(|| error_context.to_string())?;
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
        for relation in PG_ANALYTICS_REQUIRED_RELATIONS
            .iter()
            .chain(PG_ANALYTICS_REPLICA_REQUIRED_RELATIONS.iter())
        {
            if !relation_exists(&self.pool, relation).await? {
                bail!("required PostgreSQL analytics relation is missing: {relation}");
            }
        }
        for (relation, column) in PG_ANALYTICS_REQUIRED_COLUMNS {
            if !column_exists(&self.pool, relation, column).await? {
                bail!(
                    "required PostgreSQL analytics column is missing: {relation}.{column}; apply the reviewed schema contract as the schema owner before selecting the backend"
                );
            }
        }
        Ok(())
    }

    async fn verify_read_transaction(&self) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(PG_ANALYTICS_READ_TRANSACTION_SQL)
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
    ownership: AnalyticsOwnershipV2,
    pool: Option<PgAnalyticsPool>,
    maintenance_pool: Option<PgAnalyticsPool>,
    unavailable: Option<AnalyticsUnavailable>,
    ready: Arc<AtomicBool>,
}

impl PgAnalyticsRuntime {
    pub(crate) fn disabled() -> Self {
        Self {
            backend: AnalyticsBackend::Sqlite,
            ownership: AnalyticsOwnershipV2::Disabled,
            pool: None,
            maintenance_pool: None,
            unavailable: None,
            ready: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) async fn initialize(config: &AppConfig) -> Self {
        if config.analytics_backend == AnalyticsBackend::Sqlite
            && config.analytics_ownership == AnalyticsOwnershipV2::Disabled
        {
            return Self::disabled();
        }

        let result: Result<(PgAnalyticsPool, PgAnalyticsPool)> = async {
            // The V2 storage primitives are not a completed handoff implementation.
            // Never interpret the switch as permission to bypass target parity or
            // claim an owner before all serving projections can actually be read.
            validate_ownership_runtime_admission_for_backend(
                config.analytics_backend,
                config.analytics_ownership,
            )?;
            migrate_postgres_schema(config).await?;
            let pool = PgAnalyticsPool::connect(config).await?;
            let maintenance_pool = PgAnalyticsPool::connect_maintenance(config).await?;
            Ok((pool, maintenance_pool))
        }
        .await;
        match result {
            Ok((pool, maintenance_pool)) => Self {
                backend: config.analytics_backend,
                ownership: config.analytics_ownership,
                pool: Some(pool),
                maintenance_pool: Some(maintenance_pool),
                unavailable: Some(AnalyticsUnavailable::new(
                    config.analytics_backend,
                    "analytics replica bootstrap is pending",
                )),
                ready: Arc::new(AtomicBool::new(false)),
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
                    ownership: config.analytics_ownership,
                    pool: None,
                    maintenance_pool: None,
                    unavailable: Some(AnalyticsUnavailable::new(config.analytics_backend, reason)),
                    ready: Arc::new(AtomicBool::new(false)),
                }
            }
        }
    }

    pub(crate) fn backend(&self) -> AnalyticsBackend {
        self.backend
    }

    pub(crate) fn status(&self) -> AnalyticsRuntimeStatus {
        let (availability, reason) = if self.ready.load(Ordering::Acquire) {
            (AnalyticsAvailability::Ready, None)
        } else if let Some(unavailable) = &self.unavailable {
            (
                AnalyticsAvailability::Unavailable,
                Some(unavailable.reason.clone()),
            )
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
        if self.ready.load(Ordering::Acquire)
            && let Some(pool) = &self.pool
        {
            return Ok(pool);
        }
        if self.backend == AnalyticsBackend::Postgres && self.pool.is_some() {
            return Err(self.unavailable.clone().unwrap_or_else(|| {
                AnalyticsUnavailable::new(self.backend, "analytics replica bootstrap is pending")
            }));
        }
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

    pub(crate) fn spawn_bootstrap_lifecycle(
        self: &Arc<Self>,
        source_pool: Pool<Sqlite>,
        shutdown: CancellationToken,
    ) {
        if self.backend != AnalyticsBackend::Postgres {
            return;
        }
        let runtime = self.clone();
        tokio::spawn(async move {
            let mut delay = ANALYTICS_BOOTSTRAP_RETRY_INITIAL;
            loop {
                if shutdown.is_cancelled() {
                    return;
                }
                let result = tokio::select! {
                    _ = shutdown.cancelled() => return,
                    result = runtime.bootstrap_once(&source_pool) => result,
                };
                match result {
                    Ok(()) => {
                        delay = ANALYTICS_BOOTSTRAP_RETRY_INITIAL;
                    }
                    Err(error) => {
                        runtime.ready.store(false, Ordering::Release);
                        warn!(error = %sanitize_postgres_error(&error, None), "PostgreSQL analytics bootstrap is unavailable; retrying");
                        delay =
                            std::cmp::min(delay.saturating_mul(2), ANALYTICS_BOOTSTRAP_RETRY_MAX);
                    }
                }
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(delay) => {}
                }
            }
        });
    }

    async fn bootstrap_once(&self, source_pool: &Pool<Sqlite>) -> Result<()> {
        let source = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
        if self.ownership == AnalyticsOwnershipV2::Disabled && source.state.owner_kind == "postgres"
        {
            self.rollback_durable_owner(source_pool, &source).await?;
            self.ready.store(false, Ordering::Release);
            return Ok(());
        }
        let target = self
            .maintenance_pool
            .as_ref()
            .context("PostgreSQL analytics maintenance pool is unavailable")?;
        let state = load_replica_state(target.inner()).await?;
        if state.status == "paused" {
            self.ready.store(false, Ordering::Release);
            return Ok(());
        }
        if state.status == "caught_up" {
            let source = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
            if replica_service_eligible_for_source(&state, &source) {
                // Polling an idle replica must not temporarily change caught_up to
                // running and reject readers on every tick. Preserve a concurrent
                // pause/fence instead of overwriting it with a stale observation.
                let refreshed = sqlx::query(
                    "UPDATE cvm_analytics.analytics_replica_state SET updated_at = NOW()
                     WHERE id = 1 AND status = 'caught_up' AND generation = $1
                       AND consumer_checkpoint = $2 AND source_acknowledged_through = $2
                       AND owner_kind = $3 AND owner_epoch = $4 AND serving_fence = $5",
                )
                .bind(state.generation)
                .bind(state.consumer_checkpoint)
                .bind(&state.owner_kind)
                .bind(state.owner_epoch)
                .bind(state.serving_fence)
                .execute(target.inner())
                .await?;
                anyhow::ensure!(
                    refreshed.rows_affected() == 1,
                    "replica changed during heartbeat"
                );
                return self
                    .coordinate_owner_transition(source_pool, target.inner())
                    .await;
            }
        }
        let _control_guard = acquire_replica_control_lock(target.inner()).await?;
        // Re-read under the shared maintenance lock: a pause may have committed
        // after the optimistic no-work check above.
        let state = load_replica_state(target.inner()).await?;
        if state.status == "paused" {
            self.ready.store(false, Ordering::Release);
            return Ok(());
        }
        let report = if matches!(
            state.status.as_str(),
            "unbootstrapped" | "bootstrapping" | "invalidated"
        ) || !fact_date_semantics_compatible(&state)
            || !replica_baseline_ready(&state)
        {
            bootstrap_replica_control_locked(source_pool, target.inner(), false).await?
        } else {
            resume_replica_control_locked(source_pool, target.inner()).await?
        };
        anyhow::ensure!(
            report.service_eligible,
            "PostgreSQL analytics replica completed without becoming service eligible"
        );
        self.coordinate_owner_transition(source_pool, target.inner())
            .await
    }

    async fn rollback_durable_owner(
        &self,
        source_pool: &Pool<Sqlite>,
        source: &SourceContext,
    ) -> Result<()> {
        anyhow::ensure!(
            crate::long_term_projection_rollback_ready(source_pool).await?,
            "SQLite rollback projection is not ready"
        );
        crate::analytics_changes::restore_analytics_owner_to_sqlite(
            source_pool,
            source.state.owner_epoch,
            source.state.serving_fence,
        )
        .await
        .context("failed to release the durable PostgreSQL analytics owner during rollback")?;
        Ok(())
    }

    async fn coordinate_owner_transition(
        &self,
        source_pool: &Pool<Sqlite>,
        target: &sqlx::PgPool,
    ) -> Result<()> {
        let mut source = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
        let state = load_replica_state(target).await?;
        let mut projection_status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM cvm_analytics.analytics_projection_state WHERE id = 1",
        )
        .fetch_optional(target)
        .await
        .context("failed to load PostgreSQL long-term projection readiness")?;
        let (target_count, ready_target_count, matching_target_count) =
            sqlx::query_as::<_, (i64, i64, i64)>(
                "SELECT COUNT(*)::bigint,
                    COUNT(*) FILTER (WHERE targets.ready)::bigint,
                    COUNT(*) FILTER (
                        WHERE targets.ready
                          AND targets.generation = checkpoints.generation
                          AND targets.source_digest IS NOT NULL
                          AND targets.source_digest = projection.verified_digest
                    )::bigint
             FROM cvm_analytics.analytics_projection_target_status AS targets
             CROSS JOIN cvm_analytics.analytics_projection_state AS projection
             CROSS JOIN cvm_analytics.analytics_projection_checkpoints AS checkpoints
             WHERE projection.id = 1 AND checkpoints.consumer = 'long_term_v1'
               AND targets.target_kind IN (
                   'daily_rollups', 'hourly_rollups', 'interval_state',
                   'read_model', 'repair_state', 'bucket_publication'
               )",
            )
            .fetch_one(target)
            .await
            .context("failed to load PostgreSQL projection target parity")?;
        let target_parity_ready = target_count == 6
            && ready_target_count == target_count
            && matching_target_count == target_count;
        if self.ownership == AnalyticsOwnershipV2::Enabled
            && source.state.owner_kind == "sqlite"
            && state.status == "caught_up"
            && projection_status.as_deref() != Some("ready")
        {
            // Build the PG shadow before evaluating the handoff gate. The materializer
            // verifies row count and canonical digest atomically; continuous Journal
            // maintenance is a separate gate and therefore parity remains closed here.
            self.materialize_long_term_projection(source_pool).await?;
            let refreshed_source = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
            anyhow::ensure!(
                refreshed_source.latest_change_id == source.latest_change_id
                    && refreshed_source.state.owner_kind == source.state.owner_kind
                    && refreshed_source.state.owner_epoch == source.state.owner_epoch
                    && refreshed_source.state.serving_fence == source.state.serving_fence
                    && refreshed_source.state.generation == source.state.generation
                    && refreshed_source.state.analytics_unavailable == 0
                    && refreshed_source.state.journal_backpressure == 0,
                "analytics source changed during PostgreSQL projection materialization"
            );
            source = refreshed_source;
            projection_status = Some("ready".to_string());
        }
        let readiness = AnalyticsOwnershipReadiness {
            replica_facts_caught_up: state.status == "caught_up"
                && state.consumer_checkpoint == source.latest_change_id
                && replica_service_eligible_for_source(&state, &source),
            long_term_projection_ready: projection_status.as_deref() == Some("ready"),
            // A ready rollup snapshot is insufficient for ownership. Every serving target,
            // including interval/read-model and repair/bucket state, must publish parity.
            long_term_projection_parity: projection_status.as_deref() == Some("ready")
                && target_parity_ready,
        };
        match admit_analytics_ownership_transition(
            self.backend,
            self.ownership,
            &source.state.owner_kind,
            readiness,
        )? {
            AnalyticsOwnershipTransition::StayUnavailable => {
                self.ready.store(false, Ordering::Release);
                bail!(
                    "analytics ownership handoff readiness gate is closed (replica_caught_up={}, long_term_projection_ready={}, long_term_projection_parity={})",
                    readiness.replica_facts_caught_up,
                    readiness.long_term_projection_ready,
                    readiness.long_term_projection_parity,
                )
            }
            AnalyticsOwnershipTransition::LegacyReplicaReady => {
                self.ready.store(true, Ordering::Release);
                Ok(())
            }
            AnalyticsOwnershipTransition::ClaimPostgres => {
                crate::analytics_changes::claim_analytics_owner(
                    source_pool,
                    "postgres",
                    ANALYTICS_OWNER_WRITER_ID,
                    source.state.owner_epoch,
                    source.state.serving_fence,
                )
                .await
                .context("failed to claim the durable PostgreSQL analytics owner")?;
                self.ready.store(false, Ordering::Release);
                bail!("analytics ownership handoff committed; awaiting serving-fence convergence")
            }
            AnalyticsOwnershipTransition::AlreadyPostgres => {
                anyhow::ensure!(
                    source.state.writer_id.as_deref() == Some(ANALYTICS_OWNER_WRITER_ID),
                    "durable PostgreSQL analytics owner lease is not held by this runtime"
                );
                self.ready.store(true, Ordering::Release);
                Ok(())
            }
        }
    }

    pub(crate) async fn require_read_transaction(
        &self,
    ) -> AnalyticsReadResult<sqlx::Transaction<'_, sqlx::Postgres>> {
        let pool = self
            .require_pool()
            .map_err(AnalyticsReadError::Unavailable)?;
        let mut transaction = pool.inner().begin().await.map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to begin PostgreSQL analytics read transaction: {error}"
            ))
        })?;
        sqlx::query(PG_ANALYTICS_READ_TRANSACTION_SQL)
            .execute(&mut *transaction)
            .await
            .map_err(|error| {
                AnalyticsReadError::Internal(anyhow!(
                    "failed to configure PostgreSQL analytics read transaction: {error}"
                ))
            })?;
        let replica_state = sqlx::query_as::<_, AnalyticsReplicaState>(
            r#"
            SELECT replica_id, status, owner_kind, owner_epoch, serving_fence,
                   serving_ack_change_id, generation, baseline_generation, baseline_change_id,
                   coverage_generation, coverage_revision, coverage_watermark,
                   consumer_checkpoint, source_acknowledged_through, service_eligible,
                   fact_date_semantics_version,
                   last_error_code,
                   to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS updated_at
            FROM cvm_analytics.analytics_replica_state
            WHERE id = 1
            "#,
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to load PostgreSQL analytics replica read state: {error}"
            ))
        })?;
        let Some(replica_state) = replica_state else {
            return Err(AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
                AnalyticsBackend::Postgres,
                "analytics replica state is missing",
            )));
        };
        validate_analytics_replica_freshness(&replica_state)?;
        if !replica_read_eligible(&replica_state) || !replica_baseline_ready(&replica_state) {
            return Err(AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
                AnalyticsBackend::Postgres,
                format!(
                    "analytics replica coverage is not service eligible (status={})",
                    replica_state.status
                ),
            )));
        }
        let projection_status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM cvm_analytics.analytics_state WHERE id = 1",
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to load PostgreSQL analytics projection state: {error}"
            ))
        })?;
        if projection_status.as_deref() != Some("ready") {
            return Err(AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
                AnalyticsBackend::Postgres,
                format!(
                    "analytics replica projection is not ready (status={})",
                    projection_status.as_deref().unwrap_or("missing")
                ),
            )));
        }
        Ok(transaction)
    }
}

fn validate_ownership_runtime_admission(ownership: AnalyticsOwnershipV2) -> Result<()> {
    validate_ownership_runtime_admission_for_backend(AnalyticsBackend::Postgres, ownership)
}

fn validate_ownership_runtime_admission_for_backend(
    backend: AnalyticsBackend,
    ownership: AnalyticsOwnershipV2,
) -> Result<()> {
    anyhow::ensure!(
        ownership == AnalyticsOwnershipV2::Disabled || backend == AnalyticsBackend::Postgres,
        "ANALYTICS_OWNERSHIP_V2=enabled requires ANALYTICS_BACKEND=postgres"
    );
    Ok(())
}

/// Readiness facts required before the durable source owner may move to PostgreSQL.
///
/// The long-term projection and parity values are deliberately supplied by a callback-shaped
/// value. Until the projection writer publishes those facts, the startup coordinator keeps the
/// source unavailable instead of inferring readiness from replica catch-up alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AnalyticsOwnershipReadiness {
    pub(crate) replica_facts_caught_up: bool,
    pub(crate) long_term_projection_ready: bool,
    pub(crate) long_term_projection_parity: bool,
}

impl AnalyticsOwnershipReadiness {
    fn admits_handoff(self) -> bool {
        self.replica_facts_caught_up
            && self.long_term_projection_ready
            && self.long_term_projection_parity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnalyticsOwnershipTransition {
    StayUnavailable,
    LegacyReplicaReady,
    ClaimPostgres,
    AlreadyPostgres,
}

fn admit_analytics_ownership_transition(
    backend: AnalyticsBackend,
    ownership: AnalyticsOwnershipV2,
    durable_owner: &str,
    readiness: AnalyticsOwnershipReadiness,
) -> Result<AnalyticsOwnershipTransition> {
    validate_ownership_runtime_admission_for_backend(backend, ownership)?;
    if ownership == AnalyticsOwnershipV2::Disabled {
        return Ok(
            if durable_owner == "sqlite" && readiness.replica_facts_caught_up {
                AnalyticsOwnershipTransition::LegacyReplicaReady
            } else {
                AnalyticsOwnershipTransition::StayUnavailable
            },
        );
    }
    anyhow::ensure!(
        matches!(durable_owner, "sqlite" | "postgres"),
        "analytics durable owner is invalid: {durable_owner}"
    );
    if !readiness.admits_handoff() {
        return Ok(AnalyticsOwnershipTransition::StayUnavailable);
    }
    Ok(match durable_owner {
        "sqlite" => AnalyticsOwnershipTransition::ClaimPostgres,
        "postgres" => AnalyticsOwnershipTransition::AlreadyPostgres,
        _ => unreachable!("durable owner was validated above"),
    })
}

async fn migrate_postgres_schema(config: &AppConfig) -> Result<()> {
    let migration = PgAnalyticsPool::connect_migration(config).await?;
    let mut connection = migration.inner().acquire().await?;
    connection.close_on_drop();
    let version: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(&mut *connection)
        .await?;
    if !(180000..190000).contains(&version) {
        bail!("unsupported PostgreSQL major version; expected 18.x");
    }
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(POSTGRES_MIGRATION_LOCK_KEY)
        .execute(&mut *connection)
        .await?;
    let result = migrations::apply_ordered_migrations(&mut connection).await;
    let unlock = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(POSTGRES_MIGRATION_LOCK_KEY)
        .execute(&mut *connection)
        .await;
    result?;
    unlock.map_err(anyhow::Error::from)?;
    Ok(())
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
        let truncate_at = reason
            .char_indices()
            .map(|(index, _)| index)
            .take_while(|index| *index <= 512)
            .last()
            .unwrap_or(0);
        reason.truncate(truncate_at);
        reason.push_str("...");
    }
    reason
}

#[derive(Debug, Clone, FromRow, PartialEq, Serialize, Deserialize)]
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
    #[sqlx(skip)]
    pub(crate) prompt_cache_key_digest: Option<String>,
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

#[derive(Debug, FromRow)]
pub(crate) struct AnalyticsAccountUsageRow {
    pub(crate) source_row_id: i64,
    pub(crate) occurred_at_epoch_ms: i64,
    pub(crate) upstream_account_id: i64,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct AnalyticsParallelWorkKeyRow {
    pub(crate) occurred_at_epoch_ms: i64,
    pub(crate) key_digest: String,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct AnalyticsParallelWorkConversationRow {
    pub(crate) key_digest: String,
    pub(crate) first_occurred_at_epoch_ms: i64,
    pub(crate) last_occurred_at_epoch_ms: i64,
    pub(crate) request_count: i64,
}

#[derive(Debug, Clone)]
struct AnalyticsParallelWorkKey {
    source: String,
    source_row_id: i64,
    occurred_at_epoch_ms: i64,
    key_digest: String,
    upstream_account_id: Option<i64>,
}

fn parallel_work_key_digest(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let key = value.get("promptCacheKey")?.as_str()?.trim();
    if key.is_empty() {
        return None;
    }
    Some(format!("sha256:{:x}", Sha256::digest(key.as_bytes())))
}

pub(crate) async fn query_account_window_usage(
    runtime: &PgAnalyticsRuntime,
    account_ids: &[i64],
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
) -> AnalyticsReadResult<Vec<AnalyticsAccountUsageRow>> {
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut transaction = runtime.require_read_transaction().await?;
    let mut query = QueryBuilder::<sqlx::Postgres>::new(
        r#"
        SELECT source_row_id, occurred_at_epoch_ms, upstream_account_id,
               input_tokens, output_tokens, cache_input_tokens, total_tokens,
               cost::double precision AS cost
        FROM cvm_analytics.analytics_facts
        WHERE upstream_account_id IN ("#,
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query
        .push(") AND occurred_at_epoch_ms >= ")
        .push_bind(start_at.timestamp_millis())
        .push(" AND occurred_at_epoch_ms <= ")
        .push_bind(end_at.timestamp_millis())
        .push(" ORDER BY occurred_at_epoch_ms ASC, source_row_id ASC");
    let rows = query
        .build_query_as::<AnalyticsAccountUsageRow>()
        .fetch_all(&mut *transaction)
        .await
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to query PostgreSQL analytics account usage facts: {error}"
            ))
        })?;
    for row in &rows {
        validate_account_window_usage_row(row)?;
    }
    transaction.commit().await.map_err(|error| {
        AnalyticsReadError::Internal(anyhow!(
            "failed to commit PostgreSQL analytics account usage read transaction: {error}"
        ))
    })?;
    Ok(rows)
}

pub(crate) async fn query_parallel_work_key_rows(
    runtime: &PgAnalyticsRuntime,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    proxy_only: bool,
    upstream_account_id: Option<i64>,
) -> AnalyticsReadResult<Vec<AnalyticsParallelWorkKeyRow>> {
    let mut transaction = runtime.require_read_transaction().await?;
    let rows = query_parallel_work_key_rows_on_connection(
        &mut transaction,
        start_at,
        end_at,
        proxy_only,
        upstream_account_id,
    )
    .await?;
    transaction.commit().await.map_err(|error| {
        AnalyticsReadError::Internal(anyhow!(
            "failed to commit PostgreSQL analytics parallel-work key read transaction: {error}"
        ))
    })?;
    Ok(rows)
}

pub(crate) async fn query_parallel_work_key_rows_on_connection(
    connection: &mut sqlx::PgConnection,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    proxy_only: bool,
    upstream_account_id: Option<i64>,
) -> AnalyticsReadResult<Vec<AnalyticsParallelWorkKeyRow>> {
    let mut query = QueryBuilder::<sqlx::Postgres>::new(
        r#"
        SELECT occurred_at_epoch_ms, key_digest, upstream_account_id
        FROM cvm_analytics.analytics_parallel_work_keys
        WHERE occurred_at_epoch_ms >= "#,
    );
    query
        .push_bind(start_at.timestamp_millis())
        .push(" AND occurred_at_epoch_ms < ")
        .push_bind(end_at.timestamp_millis());
    if proxy_only {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND upstream_account_id = ")
            .push_bind(upstream_account_id);
    }
    query.push(" ORDER BY occurred_at_epoch_ms ASC, source_row_id ASC");
    let rows = query
        .build_query_as::<AnalyticsParallelWorkKeyRow>()
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to query PostgreSQL analytics parallel-work keys: {error}"
            ))
        })?;
    Ok(rows)
}

pub(crate) async fn query_parallel_work_conversation_rows(
    runtime: &PgAnalyticsRuntime,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    proxy_only: bool,
    upstream_account_id: Option<i64>,
) -> AnalyticsReadResult<Vec<AnalyticsParallelWorkConversationRow>> {
    let mut transaction = runtime.require_read_transaction().await?;
    let rows = query_parallel_work_conversation_rows_on_connection(
        &mut transaction,
        start_at,
        end_at,
        proxy_only,
        upstream_account_id,
    )
    .await?;
    transaction.commit().await.map_err(|error| {
        AnalyticsReadError::Internal(anyhow!(
            "failed to commit PostgreSQL analytics conversation read transaction: {error}"
        ))
    })?;
    Ok(rows)
}

pub(crate) async fn query_parallel_work_conversation_rows_on_connection(
    connection: &mut sqlx::PgConnection,
    start_at: DateTime<Utc>,
    end_at: DateTime<Utc>,
    proxy_only: bool,
    upstream_account_id: Option<i64>,
) -> AnalyticsReadResult<Vec<AnalyticsParallelWorkConversationRow>> {
    let mut query = QueryBuilder::<sqlx::Postgres>::new(
        r#"
        SELECT key_digest,
               MIN(occurred_at_epoch_ms) AS first_occurred_at_epoch_ms,
               MAX(occurred_at_epoch_ms) AS last_occurred_at_epoch_ms,
               COUNT(*)::bigint AS request_count
        FROM cvm_analytics.analytics_parallel_work_keys
        WHERE occurred_at_epoch_ms >= "#,
    );
    query
        .push_bind(start_at.timestamp_millis())
        .push(" AND occurred_at_epoch_ms < ")
        .push_bind(end_at.timestamp_millis());
    if proxy_only {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND upstream_account_id = ")
            .push_bind(upstream_account_id);
    }
    query.push(
        " GROUP BY key_digest ORDER BY last_occurred_at_epoch_ms DESC, request_count DESC LIMIT ",
    );
    query.push_bind(80_i64);
    let rows = query
        .build_query_as::<AnalyticsParallelWorkConversationRow>()
        .fetch_all(&mut *connection)
        .await
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to query PostgreSQL analytics parallel-work conversations: {error}"
            ))
        })?;
    Ok(rows)
}

#[derive(Debug, Clone, FromRow, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnalyticsReplicaState {
    pub(crate) replica_id: String,
    pub(crate) status: String,
    pub(crate) owner_kind: String,
    pub(crate) owner_epoch: i64,
    pub(crate) serving_fence: i64,
    pub(crate) serving_ack_change_id: i64,
    pub(crate) fact_date_semantics_version: i64,
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
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, FromRow)]
struct SourceChangeState {
    generation: i64,
    coverage_revision: i64,
    retained_from_change_id: i64,
    capacity: i64,
    invalidation_pending: i64,
    historical_replay_required: i64,
    historical_history_unknown: i64,
    owner_kind: String,
    owner_epoch: i64,
    serving_fence: i64,
    writer_id: Option<String>,
    serving_ack_change_id: i64,
    journal_backpressure: i64,
    analytics_unavailable: i64,
    handoff_state: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceAckRecoveryPlan {
    AlreadyCovered,
    FullGeneration,
    Range {
        from_change_id: i64,
        through_change_id: i64,
    },
    Rebuild(String),
    Inconsistent(String),
}

#[derive(Debug, Clone, FromRow)]
struct SourceChangeRow {
    change_id: i64,
    event_version: i64,
    fact_date_semantics_version: i64,
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
    historical_replay_required: bool,
    historical_history_unknown: bool,
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

#[derive(Debug, Clone, Default, FromRow)]
struct SourceInvocationSnapshotRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    occurred_at_epoch_ms: i64,
    stats_date: String,
    source: String,
    model: Option<String>,
    upstream_account_id: Option<i64>,
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
    first_token_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
struct SourceInvocationSnapshotSizeRow {
    id: i64,
    invoke_id_bytes: i64,
    occurred_at_bytes: i64,
    source_bytes: i64,
    model_bytes: Option<i64>,
    status_bytes: Option<i64>,
    error_message_bytes: Option<i64>,
    failure_kind_bytes: Option<i64>,
    failure_class_bytes: Option<i64>,
    payload_bytes: Option<i64>,
}

#[derive(Debug, FromRow)]
struct SourceParallelWorkMetadataRow {
    id: i64,
    payload: Option<String>,
    upstream_account_id: Option<i64>,
}

async fn load_source_parallel_work_metadata_page(
    source_connection: &mut SqliteConnection,
    after_id: i64,
    through_id: i64,
) -> Result<Vec<SourceParallelWorkMetadataRow>> {
    if after_id >= through_id {
        return Ok(Vec::new());
    }
    let upstream_account_id_sql =
        crate::api::invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id, payload, ");
    query
        .push(upstream_account_id_sql)
        .push(" AS upstream_account_id FROM codex_invocations WHERE id > ")
        .push_bind(after_id)
        .push(" AND id <= ")
        .push_bind(through_id)
        .push(" ORDER BY id ASC");
    query
        .build_query_as::<SourceParallelWorkMetadataRow>()
        .fetch_all(&mut *source_connection)
        .await
        .context("failed to load bounded SQLite parallel-work key metadata")
}

async fn load_source_parallel_work_metadata(
    source_pool: &Pool<Sqlite>,
    source_row_id: i64,
) -> Result<Option<SourceParallelWorkMetadataRow>> {
    let upstream_account_id_sql =
        crate::api::invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id, payload, ");
    query
        .push(upstream_account_id_sql)
        .push(" AS upstream_account_id FROM codex_invocations WHERE id = ")
        .push_bind(source_row_id);
    query
        .build_query_as::<SourceParallelWorkMetadataRow>()
        .fetch_optional(source_pool)
        .await
        .context("failed to load SQLite parallel-work key metadata")
}

fn parallel_work_keys_for_snapshot(
    facts: &[AnalyticsFact],
    metadata: Vec<SourceParallelWorkMetadataRow>,
) -> Vec<AnalyticsParallelWorkKey> {
    let facts_by_id = facts
        .iter()
        .map(|fact| (fact.source_row_id, fact))
        .collect::<HashMap<_, _>>();
    metadata
        .into_iter()
        .filter_map(|row| {
            let fact = facts_by_id.get(&row.id)?;
            parallel_work_key_for_fact(fact, Some(row))
        })
        .collect()
}

fn parallel_work_key_for_fact(
    fact: &AnalyticsFact,
    metadata: Option<SourceParallelWorkMetadataRow>,
) -> Option<AnalyticsParallelWorkKey> {
    let key_digest = fact.prompt_cache_key_digest.clone().or_else(|| {
        metadata
            .as_ref()
            .and_then(|metadata| parallel_work_key_digest(metadata.payload.as_deref()))
    })?;
    let upstream_account_id = fact.upstream_account_id.or_else(|| {
        metadata
            .as_ref()
            .and_then(|metadata| metadata.upstream_account_id)
    });
    Some(AnalyticsParallelWorkKey {
        source: fact.source.clone(),
        source_row_id: fact.source_row_id,
        occurred_at_epoch_ms: fact.occurred_at_epoch_ms,
        key_digest,
        upstream_account_id,
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

fn snapshot_success_like(
    status: Option<&str>,
    error_message: Option<&str>,
    failure_class: crate::api::FailureClass,
) -> bool {
    crate::api::prompt_invocation_status_is_success_like(status, error_message)
        && failure_class == crate::api::FailureClass::None
}

fn snapshot_row_to_fact(row: SourceInvocationSnapshotRow) -> Result<AnalyticsFact> {
    let payload = crate::analytics_changes::normalize_analytics_payload(row.payload.as_deref());
    let status = crate::analytics_changes::normalize_analytics_text(row.status.as_deref());
    let error_message =
        crate::analytics_changes::normalize_analytics_text(row.error_message.as_deref());
    let classification_error_message =
        crate::analytics_changes::normalize_analytics_classification_error_message(
            payload.as_ref(),
            row.error_message.as_deref(),
        );
    let stored_failure_kind = crate::analytics_changes::normalize_analytics_failure_kind(
        payload.as_ref(),
        row.failure_kind.as_deref(),
    );
    let stored_failure_class =
        crate::analytics_changes::normalize_analytics_text(row.failure_class.as_deref());
    let classification = crate::api::resolve_failure_classification(
        status.as_deref(),
        classification_error_message.as_deref(),
        stored_failure_kind.as_deref(),
        stored_failure_class.as_deref(),
        row.is_actionable,
    );
    let failure_kind = stored_failure_kind.or(classification.failure_kind.clone());
    let legacy_success_status = status.as_deref().is_some_and(|status| {
        matches!(
            status.to_ascii_lowercase().as_str(),
            "http_200" | "warning_success"
        )
    });
    let failure_class = match stored_failure_class {
        Some(_) => Some(classification.failure_class.as_str().to_string()),
        None => (classification.failure_class != crate::api::FailureClass::None
            || (legacy_success_status && error_message.as_deref().is_none_or(str::is_empty)))
        .then(|| classification.failure_class.as_str().to_string()),
    };
    let is_actionable = row
        .is_actionable
        .filter(|value| (*value != 0) == classification.is_actionable)
        .or_else(|| {
            (classification.failure_class != crate::api::FailureClass::None)
                .then_some(i64::from(classification.is_actionable))
        });
    Ok(AnalyticsFact {
        fact_version: 1,
        source: crate::analytics_changes::normalize_analytics_required_text(&row.source, "source")?,
        source_row_id: row.id,
        invoke_id: crate::analytics_changes::normalize_analytics_required_text(
            &row.invoke_id,
            "invoke_id",
        )?,
        occurred_at: crate::analytics_changes::normalize_analytics_required_text(
            &row.occurred_at,
            "occurred_at",
        )?,
        occurred_at_epoch_ms: row.occurred_at_epoch_ms,
        stats_date: row.stats_date,
        model: crate::analytics_changes::normalize_analytics_text(row.model.as_deref()),
        request_model: crate::analytics_changes::normalize_analytics_payload_text(
            payload.as_ref(),
            "requestModel",
        ),
        response_model: crate::analytics_changes::normalize_analytics_payload_text(
            payload.as_ref(),
            "responseModel",
        ),
        reasoning_effort: crate::analytics_changes::normalize_analytics_payload_text(
            payload.as_ref(),
            "reasoningEffort",
        ),
        prompt_cache_key_digest:
            crate::analytics_changes::normalize_analytics_prompt_cache_key_digest(payload.as_ref()),
        upstream_account_id: crate::analytics_changes::normalize_analytics_payload_i64(
            payload.as_ref(),
            "upstreamAccountId",
        )
        .or(row.upstream_account_id),
        upstream_account_name: crate::analytics_changes::normalize_analytics_payload_text(
            payload.as_ref(),
            "upstreamAccountName",
        ),
        status: status.clone(),
        success_like: snapshot_success_like(
            status.as_deref(),
            error_message.as_deref(),
            classification.failure_class,
        ),
        terminal: snapshot_terminal(status.as_deref()),
        failure_kind,
        failure_class,
        is_actionable: is_actionable.map(|value| value != 0),
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
}

fn source_retention_exhausted(state: &AnalyticsReplicaState, source: &SourceContext) -> bool {
    let cursor_predates_retention =
        state.consumer_checkpoint.saturating_add(1) < source.state.retained_from_change_id;
    let generation_changed = state.generation != source.state.generation;
    cursor_predates_retention || generation_changed || source.state.invalidation_pending != 0
}

fn replica_can_recover_pending_generation_ack(
    state: &AnalyticsReplicaState,
    source: &SourceContext,
) -> bool {
    source.state.invalidation_pending != 0
        && state.generation == source.state.generation
        && replica_baseline_ready(state)
        && state.baseline_change_id == source.latest_change_id
        && state.consumer_checkpoint == source.latest_change_id
}

fn unconsumed_source_bytes_exceed_retention_bound(lag_bytes: i64) -> bool {
    lag_bytes > ANALYTICS_SOURCE_RETENTION_BOUND_BYTES
}

fn replica_service_eligible(state: &AnalyticsReplicaState) -> bool {
    // Keep the shared flag false so pre-FU01 programs fail closed when they ignore the
    // additive semantic-version column. Current code derives eligibility from the v2 marker,
    // lifecycle state, and matching checkpoints instead.
    state.owner_kind == "postgres"
        && state.owner_epoch > 0
        && state.serving_fence > 0
        && state.serving_ack_change_id >= state.consumer_checkpoint
        && replica_read_eligible(state)
}

// Selecting the existing PostgreSQL read replica does not transfer projection ownership.
// Its legacy eligibility is based on complete, current source coverage, independently of V2.
fn replica_read_eligible(state: &AnalyticsReplicaState) -> bool {
    matches!(state.owner_kind.as_str(), "sqlite" | "postgres")
        && state.owner_epoch > 0
        && state.serving_fence > 0
        && (state.owner_kind == "sqlite"
            || state.serving_ack_change_id >= state.consumer_checkpoint)
        && fact_date_semantics_compatible(state)
        && state.status == "caught_up"
        && state.consumer_checkpoint == state.source_acknowledged_through
}

fn fact_date_semantics_compatible(state: &AnalyticsReplicaState) -> bool {
    state.fact_date_semantics_version == ANALYTICS_FACT_DATE_SEMANTICS_VERSION
}

fn fact_date_semantics_mismatch_detail(state: &AnalyticsReplicaState) -> String {
    format!(
        "analytics replica fact-date semantics version {} is incompatible with required version {}; run explicit bootstrap to rebuild the bounded target snapshot",
        state.fact_date_semantics_version, ANALYTICS_FACT_DATE_SEMANTICS_VERSION
    )
}

fn replica_service_eligible_for_latest_change(
    state: &AnalyticsReplicaState,
    latest_change_id: i64,
) -> bool {
    replica_service_eligible(state) && state.consumer_checkpoint == latest_change_id
}

fn source_coverage_reaches_checkpoint(
    state: &AnalyticsReplicaState,
    source: &SourceContext,
) -> bool {
    source.state.generation == state.generation
        && source.state.invalidation_pending == 0
        && source.state.historical_replay_required == 0
        && source.state.historical_history_unknown == 0
        && source.coverage.is_some_and(|coverage| {
            coverage.generation == state.generation
                && coverage.covered_through_change_id == state.consumer_checkpoint
        })
}

fn source_serving_fence_ready(source: &SourceContext) -> bool {
    source.state.owner_kind == "postgres"
        && source.state.owner_epoch > 0
        && source.state.serving_fence > 0
        && source.state.journal_backpressure == 0
        && source.state.analytics_unavailable == 0
        && source.state.handoff_state == "stable"
}

fn replica_service_eligible_for_source(
    state: &AnalyticsReplicaState,
    source: &SourceContext,
) -> bool {
    replica_baseline_ready(state)
        && replica_read_eligible(state)
        && state.consumer_checkpoint == source.latest_change_id
        && source.state.owner_kind == state.owner_kind
        && source.state.owner_epoch == state.owner_epoch
        && source.state.serving_fence == state.serving_fence
        && source.state.journal_backpressure == 0
        && source.state.analytics_unavailable == 0
        && source.state.handoff_state == "stable"
        && source_coverage_reaches_checkpoint(state, source)
}

fn replica_baseline_ready(state: &AnalyticsReplicaState) -> bool {
    state.generation > 0
        && state.baseline_generation == state.generation
        && state.baseline_change_id <= state.consumer_checkpoint
}

fn source_ack_recovery_plan(
    state: &AnalyticsReplicaState,
    source: &SourceContext,
) -> SourceAckRecoveryPlan {
    if state.source_acknowledged_through > state.consumer_checkpoint {
        return SourceAckRecoveryPlan::Inconsistent(format!(
            "analytics replica source acknowledgement {} is ahead of target checkpoint {}",
            state.source_acknowledged_through, state.consumer_checkpoint
        ));
    }
    if state.consumer_checkpoint > source.latest_change_id {
        return SourceAckRecoveryPlan::Inconsistent(format!(
            "analytics replica target checkpoint {} is ahead of source latest change {}",
            state.consumer_checkpoint, source.latest_change_id
        ));
    }

    if let Some(coverage) = source.coverage {
        if coverage.generation == source.state.generation {
            if coverage.covered_through_change_id > state.consumer_checkpoint {
                return SourceAckRecoveryPlan::Inconsistent(format!(
                    "source coverage {} is ahead of target checkpoint {}",
                    coverage.covered_through_change_id, state.consumer_checkpoint
                ));
            }
            if coverage.covered_through_change_id == state.consumer_checkpoint {
                return SourceAckRecoveryPlan::AlreadyCovered;
            }
            if state.consumer_checkpoint == source.latest_change_id {
                return SourceAckRecoveryPlan::FullGeneration;
            }
            let from_change_id = coverage.covered_through_change_id.saturating_add(1);
            if from_change_id < source.state.retained_from_change_id {
                return SourceAckRecoveryPlan::Rebuild(format!(
                    "source coverage through {} predates retained source journal boundary {}; run explicit bootstrap",
                    coverage.covered_through_change_id, source.state.retained_from_change_id
                ));
            }
            return SourceAckRecoveryPlan::Range {
                from_change_id,
                through_change_id: state.consumer_checkpoint,
            };
        }
        if state.consumer_checkpoint == source.latest_change_id {
            return SourceAckRecoveryPlan::FullGeneration;
        }
        return SourceAckRecoveryPlan::Rebuild(format!(
            "source coverage belongs to generation {}, expected {}; run explicit bootstrap",
            coverage.generation, source.state.generation
        ));
    }

    if state.consumer_checkpoint == source.latest_change_id {
        return SourceAckRecoveryPlan::FullGeneration;
    }
    let from_change_id = source.state.retained_from_change_id;
    if from_change_id > state.consumer_checkpoint.saturating_add(1) {
        return SourceAckRecoveryPlan::Rebuild(format!(
            "source journal retained from change {}, beyond target checkpoint {}; run explicit bootstrap",
            from_change_id, state.consumer_checkpoint
        ));
    }
    SourceAckRecoveryPlan::Range {
        from_change_id,
        through_change_id: state.consumer_checkpoint,
    }
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

async fn relation_exists(pool: &sqlx::PgPool, relation: &str) -> Result<bool> {
    sqlx::query_scalar(PG_ANALYTICS_RELATION_EXISTS_SQL)
        .bind(relation)
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to inspect PostgreSQL relation {relation}"))
}

async fn relation_exists_on_connection(
    connection: &mut sqlx::postgres::PgConnection,
    relation: &str,
) -> Result<bool> {
    sqlx::query_scalar(PG_ANALYTICS_RELATION_EXISTS_SQL)
        .bind(relation)
        .fetch_one(&mut *connection)
        .await
        .with_context(|| format!("failed to inspect PostgreSQL relation {relation}"))
}

const PG_ANALYTICS_RELATION_EXISTS_SQL: &str = r#"
        SELECT EXISTS(
            SELECT 1
            FROM pg_catalog.pg_class AS relation
            JOIN pg_catalog.pg_namespace AS namespace
              ON namespace.oid = relation.relnamespace
            WHERE namespace.nspname = split_part($1::text, '.', 1)
              AND relation.relname = split_part($1::text, '.', 2)
        )
        "#;

async fn column_exists(pool: &sqlx::PgPool, relation: &str, column: &str) -> Result<bool> {
    // information_schema hides columns unavailable to the current role. The catalog query keeps
    // startup validation compatible with the documented least-privilege application role.
    sqlx::query_scalar(PG_ANALYTICS_COLUMN_EXISTS_SQL)
        .bind(relation)
        .bind(column)
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to inspect PostgreSQL column {relation}.{column}"))
}

async fn column_exists_on_connection(
    connection: &mut sqlx::postgres::PgConnection,
    relation: &str,
    column: &str,
) -> Result<bool> {
    sqlx::query_scalar(PG_ANALYTICS_COLUMN_EXISTS_SQL)
        .bind(relation)
        .bind(column)
        .fetch_one(&mut *connection)
        .await
        .with_context(|| format!("failed to inspect PostgreSQL column {relation}.{column}"))
}

const PG_ANALYTICS_COLUMN_EXISTS_SQL: &str = r#"
        SELECT EXISTS(
            SELECT 1
            FROM pg_catalog.pg_attribute AS attribute
            JOIN pg_catalog.pg_class AS relation
              ON relation.oid = attribute.attrelid
            JOIN pg_catalog.pg_namespace AS namespace
              ON namespace.oid = relation.relnamespace
            WHERE namespace.nspname = split_part($1::text, '.', 1)
              AND relation.relname = split_part($1::text, '.', 2)
              AND attribute.attname = $2
              AND attribute.attnum > 0
              AND NOT attribute.attisdropped
        )
        "#;

async fn verify_replica_schema(pool: &sqlx::PgPool) -> Result<()> {
    for relation in PG_ANALYTICS_REQUIRED_RELATIONS
        .iter()
        .chain(PG_ANALYTICS_REPLICA_REQUIRED_RELATIONS.iter())
    {
        if !relation_exists(pool, relation).await? {
            bail!("required PostgreSQL analytics relation is missing: {relation}");
        }
    }
    for (relation, column) in PG_ANALYTICS_REQUIRED_COLUMNS {
        if !column_exists(pool, relation, column).await? {
            bail!(
                "required PostgreSQL analytics column is missing: {relation}.{column}; apply the reviewed schema contract as the schema owner before running analytics-replica bootstrap"
            );
        }
    }
    Ok(())
}

async fn verify_replica_schema_on_connection(
    connection: &mut sqlx::postgres::PgConnection,
) -> Result<()> {
    for relation in PG_ANALYTICS_REQUIRED_RELATIONS
        .iter()
        .chain(PG_ANALYTICS_REPLICA_REQUIRED_RELATIONS.iter())
    {
        if !relation_exists_on_connection(connection, relation).await? {
            bail!("required PostgreSQL analytics relation is missing: {relation}");
        }
    }
    for (relation, column) in PG_ANALYTICS_REQUIRED_COLUMNS {
        if !column_exists_on_connection(connection, relation, column).await? {
            bail!(
                "required PostgreSQL analytics column is missing: {relation}.{column}; apply the reviewed schema contract as the schema owner before running analytics-replica bootstrap"
            );
        }
    }
    Ok(())
}

async fn load_replica_state(pool: &sqlx::PgPool) -> Result<AnalyticsReplicaState> {
    sqlx::query_as::<_, AnalyticsReplicaState>(
        r#"
        SELECT replica_id, status, owner_kind, owner_epoch, serving_fence,
               serving_ack_change_id, generation, baseline_generation, baseline_change_id,
               coverage_generation, coverage_revision, coverage_watermark,
               consumer_checkpoint, source_acknowledged_through, service_eligible,
               fact_date_semantics_version,
               last_error_code, updated_at::text AS updated_at
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
               invalidation_pending, historical_replay_required, historical_history_unknown,
               owner_kind, owner_epoch, serving_fence, writer_id, serving_ack_change_id,
               journal_backpressure, analytics_unavailable, handoff_state
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
        historical_replay_required: source.state.historical_replay_required != 0,
        historical_history_unknown: source.state.historical_history_unknown != 0,
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
    error_code: Option<&str>,
    _error_detail: Option<&str>,
    projection_status: &str,
) -> Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET status = $1,
            service_eligible = FALSE,
            last_error_code = $2,
            updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(status)
    .bind(error_code)
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

async fn mark_replica_state_on_connection(
    connection: &mut sqlx::postgres::PgConnection,
    status: &str,
    error_code: Option<&str>,
    _error_detail: Option<&str>,
    projection_status: &str,
) -> Result<()> {
    let mut transaction = connection.begin().await?;
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET status = $1,
            service_eligible = FALSE,
            last_error_code = $2,
            updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(status)
    .bind(error_code)
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
        Some(error_code),
        Some(&detail),
        "unavailable",
    )
    .await
}

async fn mark_replica_blocked_on_connection(
    connection: &mut sqlx::postgres::PgConnection,
    error_code: &str,
    error_detail: impl Into<String>,
) -> Result<()> {
    let detail = error_detail.into();
    mark_replica_state_on_connection(
        connection,
        "blocked",
        Some(error_code),
        Some(&detail),
        "unavailable",
    )
    .await
}

async fn invalidate_legacy_fact_date_semantics(
    pool: &sqlx::PgPool,
    observed: &AnalyticsReplicaState,
    _detail: &str,
) -> Result<AnalyticsReplicaState> {
    let mut transaction = pool.begin().await?;
    let updated = sqlx::query(PG_ANALYTICS_LEGACY_INVALIDATION_SQL)
        .bind(observed.fact_date_semantics_version)
        .bind(&observed.status)
        .bind(observed.generation)
        .bind(observed.consumer_checkpoint)
        .execute(&mut *transaction)
        .await?;
    if updated.rows_affected() == 1 {
        sqlx::query(
            "UPDATE cvm_analytics.analytics_state SET status = 'unavailable', last_error_code = 'fact_date_semantics_mismatch', updated_at = NOW() WHERE id = 1",
        )
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    load_replica_state(pool).await
}

const PG_ANALYTICS_LEGACY_INVALIDATION_SQL: &str = r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET status = 'invalidated', service_eligible = FALSE,
            last_error_code = 'fact_date_semantics_mismatch',
            updated_at = NOW()
        WHERE id = 1
          AND fact_date_semantics_version = $1
          AND status = $2
          AND generation = $3
          AND consumer_checkpoint = $4
        "#;

async fn ensure_fact_date_semantics_compatible(
    pool: &sqlx::PgPool,
    state: &AnalyticsReplicaState,
) -> Result<AnalyticsReplicaState> {
    if state.status == "unbootstrapped" || fact_date_semantics_compatible(state) {
        return Ok(state.clone());
    }
    let detail = fact_date_semantics_mismatch_detail(state);
    let current = invalidate_legacy_fact_date_semantics(pool, state, &detail).await?;
    if !fact_date_semantics_compatible(&current) {
        return Err(anyhow!(fact_date_semantics_mismatch_detail(&current)));
    }
    Ok(current)
}

async fn set_replica_running(pool: &sqlx::PgPool) -> Result<()> {
    mark_replica_state(pool, "running", None, None, "preparing").await
}

async fn sync_replica_serving_fence(pool: &sqlx::PgPool, source: &SourceContext) -> Result<()> {
    let result = sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET owner_kind = $1,
            owner_epoch = $2,
            serving_fence = $3,
            serving_ack_change_id = GREATEST(serving_ack_change_id, $4),
            generation = $5,
            baseline_generation = $5,
            service_eligible = FALSE,
            status = CASE WHEN status = 'caught_up' THEN 'running' ELSE status END,
            updated_at = NOW()
        WHERE id = 1 AND generation <= $5
        "#,
    )
    .bind(&source.state.owner_kind)
    .bind(source.state.owner_epoch)
    .bind(source.state.serving_fence)
    .bind(source.state.serving_ack_change_id)
    .bind(source.state.generation)
    .execute(pool)
    .await
    .context("failed to synchronize the PostgreSQL analytics serving fence")?;
    anyhow::ensure!(
        result.rows_affected() == 1,
        "PostgreSQL analytics serving fence generation is stale"
    );
    Ok(())
}

async fn set_replica_acknowledged(
    pool: &sqlx::PgPool,
    through_change_id: i64,
    caught_up: bool,
    expected_generation: i64,
) -> Result<()> {
    let status = if caught_up { "caught_up" } else { "running" };
    let projection_status = if caught_up { "ready" } else { "preparing" };
    let mut transaction = pool.begin().await?;
    let result = sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET source_acknowledged_through = $1,
            serving_ack_change_id = GREATEST(serving_ack_change_id, $1),
            status = $2,
            service_eligible = FALSE,
            last_error_code = NULL,
            updated_at = NOW()
        WHERE id = 1 AND generation = $3
        "#,
    )
    .bind(through_change_id)
    .bind(status)
    .bind(expected_generation)
    .execute(&mut *transaction)
    .await?;
    anyhow::ensure!(
        result.rows_affected() == 1,
        "PostgreSQL analytics replica acknowledgement fence is stale"
    );
    sqlx::query(
        "UPDATE cvm_analytics.analytics_state SET status = $1, last_error_code = NULL, updated_at = NOW() WHERE id = 1",
    )
    .bind(projection_status)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn set_replica_acknowledged_on_connection(
    connection: &mut sqlx::postgres::PgConnection,
    through_change_id: i64,
    caught_up: bool,
    expected_generation: i64,
) -> Result<()> {
    let status = if caught_up { "caught_up" } else { "running" };
    let projection_status = if caught_up { "ready" } else { "preparing" };
    let mut transaction = connection.begin().await?;
    let result = sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET source_acknowledged_through = $1,
            serving_ack_change_id = GREATEST(serving_ack_change_id, $1),
            status = $2,
            service_eligible = FALSE,
            last_error_code = NULL,
            updated_at = NOW()
        WHERE id = 1 AND generation = $3
        "#,
    )
    .bind(through_change_id)
    .bind(status)
    .bind(expected_generation)
    .execute(&mut *transaction)
    .await?;
    anyhow::ensure!(
        result.rows_affected() == 1,
        "PostgreSQL analytics replica acknowledgement fence is stale"
    );
    sqlx::query(
        "UPDATE cvm_analytics.analytics_state SET status = $1, last_error_code = NULL, updated_at = NOW() WHERE id = 1",
    )
    .bind(projection_status)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn fence_replica_service_eligibility_on_connection(
    connection: &mut sqlx::postgres::PgConnection,
) -> Result<()> {
    let result = sqlx::query(
        "UPDATE cvm_analytics.analytics_replica_state SET service_eligible = FALSE, status = CASE WHEN status = 'caught_up' THEN 'bootstrapping' ELSE status END, updated_at = NOW() WHERE id = 1",
    )
    .execute(&mut *connection)
    .await?;
    anyhow::ensure!(
        result.rows_affected() == 1,
        "PostgreSQL analytics replica state row is missing while applying the rollback fence"
    );
    Ok(())
}

async fn clear_replica_target(
    connection: &mut sqlx::postgres::PgConnection,
    generation: i64,
) -> Result<()> {
    let mut transaction = connection.begin().await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_replica_applied_changes")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_parallel_work_keys")
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_facts")
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET status = 'bootstrapping', generation = $1,
            fact_date_semantics_version = $2,
            baseline_generation = 0, baseline_change_id = 0,
            coverage_generation = 0, coverage_revision = 0,
            coverage_watermark = 0, consumer_checkpoint = 0,
            source_acknowledged_through = 0, serving_ack_change_id = 0,
            service_eligible = FALSE,
            last_error_code = NULL, updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(generation)
    .bind(ANALYTICS_FACT_DATE_SEMANTICS_VERSION)
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

async fn set_replica_baseline(
    connection: &mut sqlx::postgres::PgConnection,
    source: &SourceContext,
) -> Result<()> {
    let mut transaction = connection.begin().await?;
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET status = 'running', generation = $1,
            owner_kind = $5, owner_epoch = $6, serving_fence = $7,
            serving_ack_change_id = 0,
            fact_date_semantics_version = $4,
            baseline_generation = $1, baseline_change_id = $2,
            coverage_generation = $1, coverage_revision = $3,
            coverage_watermark = $2, consumer_checkpoint = $2,
            source_acknowledged_through = 0, service_eligible = FALSE,
            last_error_code = NULL, updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .bind(source.state.generation)
    .bind(source.latest_change_id)
    .bind(source.state.coverage_revision)
    .bind(ANALYTICS_FACT_DATE_SEMANTICS_VERSION)
    .bind(&source.state.owner_kind)
    .bind(source.state.owner_epoch)
    .bind(source.state.serving_fence)
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
        SELECT change_id, event_version, fact_date_semantics_version,
               event_kind, entity_kind, source, source_row_id,
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

fn ensure_snapshot_source_field_size(
    row_id: i64,
    field: &str,
    size: Option<i64>,
    max_bytes: i64,
) -> Result<()> {
    if let Some(size) = size
        && size > max_bytes
    {
        bail!(
            "oversized analytics replica source field {field} on row {row_id}: {size} bytes exceeds {max_bytes}-byte bound"
        );
    }
    Ok(())
}

async fn preflight_source_invocation_snapshot_page(
    source_connection: &mut SqliteConnection,
    after_id: i64,
) -> Result<()> {
    let rows = sqlx::query_as::<_, SourceInvocationSnapshotSizeRow>(
        r#"
        SELECT
            id,
            length(CAST(invoke_id AS BLOB)) AS invoke_id_bytes,
            length(CAST(occurred_at AS BLOB)) AS occurred_at_bytes,
            length(CAST(source AS BLOB)) AS source_bytes,
            length(CAST(model AS BLOB)) AS model_bytes,
            length(CAST(status AS BLOB)) AS status_bytes,
            length(CAST(error_message AS BLOB)) AS error_message_bytes,
            length(CAST(failure_kind AS BLOB)) AS failure_kind_bytes,
            length(CAST(failure_class AS BLOB)) AS failure_class_bytes,
            length(CAST(payload AS BLOB)) AS payload_bytes
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id
        LIMIT ?2
        "#,
    )
    .bind(after_id)
    .bind(i64::try_from(ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH)?)
    .fetch_all(&mut *source_connection)
    .await
    .context("failed to preflight a bounded SQLite analytics baseline page")?;

    for row in rows {
        for (field, size) in [
            ("invoke_id", Some(row.invoke_id_bytes)),
            ("occurred_at", Some(row.occurred_at_bytes)),
            ("source", Some(row.source_bytes)),
            ("model", row.model_bytes),
            ("status", row.status_bytes),
            ("error_message", row.error_message_bytes),
            ("failure_kind", row.failure_kind_bytes),
            ("failure_class", row.failure_class_bytes),
        ] {
            ensure_snapshot_source_field_size(
                row.id,
                field,
                size,
                ANALYTICS_REPLICA_MAX_SOURCE_TEXT_BYTES,
            )?;
        }
        ensure_snapshot_source_field_size(
            row.id,
            "payload",
            row.payload_bytes,
            ANALYTICS_REPLICA_MAX_SOURCE_PAYLOAD_BYTES,
        )?;
    }
    Ok(())
}

async fn load_source_invocation_snapshot_page(
    source_connection: &mut SqliteConnection,
    after_id: i64,
) -> Result<Vec<AnalyticsFact>> {
    // Preflight bounds the raw values before SQLx materializes them. The raw row is then streamed
    // one at a time so NUL-bearing text follows the same Rust normalization as incremental facts.
    preflight_source_invocation_snapshot_page(source_connection, after_id).await?;

    let upstream_account_id_sql =
        crate::api::invocation_upstream_account_id_from_attempt_fallback_sql("codex_invocations");
    let snapshot_sql = format!(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            CAST(COALESCE(ROUND((julianday(
                occurred_at,
                CASE WHEN instr(occurred_at, 'T') > 0 THEN '+0 hours' ELSE '-8 hours' END
            ) - 2440587.5) * 86400000.0), 0) AS INTEGER) AS occurred_at_epoch_ms,
            date(
                occurred_at,
                CASE WHEN instr(occurred_at, 'T') > 0 THEN '+8 hours' ELSE '+0 hours' END
            ) AS stats_date,
            source,
            model,
            {upstream_account_id_sql} AS upstream_account_id,
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
            first_token_ms,
            t_resp_parse_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id
        LIMIT ?2
        "#
    );
    let mut rows = sqlx::query_as::<_, SourceInvocationSnapshotRow>(&snapshot_sql)
        .bind(after_id)
        .bind(i64::try_from(ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH)?)
        .fetch(&mut *source_connection);

    let mut facts = Vec::new();
    while let Some(row) = rows
        .try_next()
        .await
        .context("failed to stream a bounded SQLite analytics baseline page")?
    {
        facts.push(snapshot_row_to_fact(row)?);
    }
    Ok(facts)
}

fn archive_snapshot_column(columns: &HashSet<String>, name: &str) -> String {
    if columns.contains(name) {
        name.to_string()
    } else {
        format!("NULL AS {name}")
    }
}

fn build_archive_invocation_snapshot_query(columns: &HashSet<String>) -> Result<String> {
    for required in ["id", "invoke_id", "occurred_at", "source"] {
        anyhow::ensure!(
            columns.contains(required),
            "invocation archive is missing required column {required}"
        );
    }
    let optional = |name: &str| archive_snapshot_column(columns, name);
    Ok(format!(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            CAST(COALESCE(ROUND((julianday(
                occurred_at,
                CASE WHEN instr(occurred_at, 'T') > 0 THEN '+0 hours' ELSE '-8 hours' END
            ) - 2440587.5) * 86400000.0), 0) AS INTEGER) AS occurred_at_epoch_ms,
            date(
                occurred_at,
                CASE WHEN instr(occurred_at, 'T') > 0 THEN '+8 hours' ELSE '+0 hours' END
            ) AS stats_date,
            source,
            {model},
            {upstream_account_id},
            {input_tokens},
            {output_tokens},
            {cache_input_tokens},
            {reasoning_tokens},
            {total_tokens},
            {cost},
            {cost_input},
            {cost_cache_write},
            {cost_cache_read},
            {cost_output},
            {cost_reasoning},
            {status},
            {error_message},
            {failure_kind},
            {failure_class},
            {is_actionable},
            {payload},
            {t_total_ms},
            {t_req_read_ms},
            {t_req_parse_ms},
            {t_upstream_connect_ms},
            {t_upstream_ttfb_ms},
            {t_upstream_stream_ms},
            {first_token_ms},
            {t_resp_parse_ms}
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id
        LIMIT ?2
        "#,
        model = optional("model"),
        upstream_account_id = optional("upstream_account_id"),
        input_tokens = optional("input_tokens"),
        output_tokens = optional("output_tokens"),
        cache_input_tokens = optional("cache_input_tokens"),
        reasoning_tokens = optional("reasoning_tokens"),
        total_tokens = optional("total_tokens"),
        cost = optional("cost"),
        cost_input = optional("cost_input"),
        cost_cache_write = optional("cost_cache_write"),
        cost_cache_read = optional("cost_cache_read"),
        cost_output = optional("cost_output"),
        cost_reasoning = optional("cost_reasoning"),
        status = optional("status"),
        error_message = optional("error_message"),
        failure_kind = optional("failure_kind"),
        failure_class = optional("failure_class"),
        is_actionable = optional("is_actionable"),
        payload = optional("payload"),
        t_total_ms = optional("t_total_ms"),
        t_req_read_ms = optional("t_req_read_ms"),
        t_req_parse_ms = optional("t_req_parse_ms"),
        t_upstream_connect_ms = optional("t_upstream_connect_ms"),
        t_upstream_ttfb_ms = optional("t_upstream_ttfb_ms"),
        t_upstream_stream_ms = optional("t_upstream_stream_ms"),
        first_token_ms = optional("first_token_ms"),
        t_resp_parse_ms = optional("t_resp_parse_ms"),
    ))
}

async fn snapshot_invocation_archive_into_target(
    archive_pool: &Pool<Sqlite>,
    connection: &mut sqlx::postgres::PgConnection,
) -> Result<()> {
    let columns = crate::load_sqlite_table_columns(archive_pool, "codex_invocations").await?;
    let query = build_archive_invocation_snapshot_query(&columns)?;
    let mut after_id = 0;
    loop {
        let facts = sqlx::query_as::<_, SourceInvocationSnapshotRow>(&query)
            .bind(after_id)
            .bind(i64::try_from(ANALYTICS_REPLICA_MAX_FACTS_PER_BATCH)?)
            .fetch_all(archive_pool)
            .await
            .context("failed to read a bounded invocation archive snapshot page")?
            .into_iter()
            .map(snapshot_row_to_fact)
            .collect::<Result<Vec<_>>>()?;
        let Some(next_after_id) = facts.last().map(|fact| fact.source_row_id) else {
            break;
        };
        snapshot_facts_into_target(connection, &facts, &[]).await?;
        after_id = next_after_id;
    }
    Ok(())
}

async fn upsert_analytics_fact(
    connection: &mut sqlx::postgres::PgConnection,
    fact: &AnalyticsFact,
) -> Result<()> {
    // ON CONFLICT DO UPDATE requires SELECT on every EXCLUDED column in PostgreSQL. MERGE keeps
    // the maintenance role's read surface limited to the source identity match columns.
    sqlx::query(
        r#"
        MERGE INTO cvm_analytics.analytics_facts AS target
        USING (
            VALUES (
                $1::smallint, $2::text, $3::bigint, $4::text,
                to_timestamp($5::double precision / 1000.0), $5::bigint, $6::date,
                $7::text, $8::text, $9::text, $10::text, $11::bigint, $12::text,
                $13::text, $14::boolean, $15::boolean, $16::text, $17::text,
                $18::boolean, $19::bigint, $20::bigint, $21::bigint, $22::bigint,
                $23::bigint, $24::double precision, $25::double precision,
                $26::double precision, $27::double precision, $28::double precision,
                $29::double precision, $30::double precision, $31::double precision,
                $32::double precision, $33::double precision, $34::double precision,
                $35::double precision, $36::double precision, $37::double precision
            )
        ) AS incoming(
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
        ON target.source = incoming.source
           AND target.source_row_id = incoming.source_row_id
        WHEN MATCHED THEN UPDATE SET
            fact_version = incoming.fact_version,
            invoke_id = incoming.invoke_id,
            occurred_at = incoming.occurred_at,
            occurred_at_epoch_ms = incoming.occurred_at_epoch_ms,
            stats_date = incoming.stats_date,
            model = incoming.model,
            request_model = incoming.request_model,
            response_model = incoming.response_model,
            reasoning_effort = incoming.reasoning_effort,
            upstream_account_id = incoming.upstream_account_id,
            upstream_account_name = incoming.upstream_account_name,
            status = incoming.status,
            success_like = incoming.success_like,
            terminal = incoming.terminal,
            failure_kind = incoming.failure_kind,
            failure_class = incoming.failure_class,
            is_actionable = incoming.is_actionable,
            input_tokens = incoming.input_tokens,
            output_tokens = incoming.output_tokens,
            cache_input_tokens = incoming.cache_input_tokens,
            reasoning_tokens = incoming.reasoning_tokens,
            total_tokens = incoming.total_tokens,
            cost = incoming.cost,
            cost_input = incoming.cost_input,
            cost_cache_write = incoming.cost_cache_write,
            cost_cache_read = incoming.cost_cache_read,
            cost_output = incoming.cost_output,
            cost_reasoning = incoming.cost_reasoning,
            t_total_ms = incoming.t_total_ms,
            t_req_read_ms = incoming.t_req_read_ms,
            t_req_parse_ms = incoming.t_req_parse_ms,
            t_upstream_connect_ms = incoming.t_upstream_connect_ms,
            t_upstream_ttfb_ms = incoming.t_upstream_ttfb_ms,
            t_upstream_stream_ms = incoming.t_upstream_stream_ms,
            first_token_ms = incoming.first_token_ms,
            t_resp_parse_ms = incoming.t_resp_parse_ms
        WHEN NOT MATCHED THEN INSERT (
            fact_version, source, source_row_id, invoke_id, occurred_at,
            occurred_at_epoch_ms, stats_date, model, request_model, response_model,
            reasoning_effort, upstream_account_id, upstream_account_name, status,
            success_like, terminal, failure_kind, failure_class, is_actionable,
            input_tokens, output_tokens, cache_input_tokens, reasoning_tokens,
            total_tokens, cost, cost_input, cost_cache_write, cost_cache_read,
            cost_output, cost_reasoning, t_total_ms, t_req_read_ms, t_req_parse_ms,
            t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms,
            first_token_ms, t_resp_parse_ms
        ) VALUES (
            incoming.fact_version, incoming.source, incoming.source_row_id, incoming.invoke_id,
            incoming.occurred_at, incoming.occurred_at_epoch_ms, incoming.stats_date,
            incoming.model, incoming.request_model, incoming.response_model,
            incoming.reasoning_effort, incoming.upstream_account_id,
            incoming.upstream_account_name, incoming.status, incoming.success_like,
            incoming.terminal, incoming.failure_kind, incoming.failure_class,
            incoming.is_actionable, incoming.input_tokens, incoming.output_tokens,
            incoming.cache_input_tokens, incoming.reasoning_tokens, incoming.total_tokens,
            incoming.cost, incoming.cost_input, incoming.cost_cache_write,
            incoming.cost_cache_read, incoming.cost_output, incoming.cost_reasoning,
            incoming.t_total_ms, incoming.t_req_read_ms, incoming.t_req_parse_ms,
            incoming.t_upstream_connect_ms, incoming.t_upstream_ttfb_ms,
            incoming.t_upstream_stream_ms, incoming.first_token_ms, incoming.t_resp_parse_ms
        )
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

async fn replace_parallel_work_key(
    connection: &mut sqlx::postgres::PgConnection,
    source: &str,
    source_row_id: i64,
    key: Option<&AnalyticsParallelWorkKey>,
) -> Result<()> {
    if let Some(key) = key {
        anyhow::ensure!(
            key.source == source && key.source_row_id == source_row_id,
            "parallel-work key identity disagrees with source change"
        );
        // Keep conflict matching on identity columns without granting reads on the key payload.
        sqlx::query(
            r#"
            MERGE INTO cvm_analytics.analytics_parallel_work_keys AS target
            USING (
                VALUES ($1::text, $2::bigint, $3::bigint, $4::text, $5::bigint)
            ) AS incoming(source, source_row_id, occurred_at_epoch_ms, key_digest, upstream_account_id)
            ON target.source = incoming.source
               AND target.source_row_id = incoming.source_row_id
            WHEN MATCHED THEN UPDATE SET
                occurred_at_epoch_ms = incoming.occurred_at_epoch_ms,
                key_digest = incoming.key_digest,
                upstream_account_id = incoming.upstream_account_id
            WHEN NOT MATCHED THEN INSERT (
                source, source_row_id, occurred_at_epoch_ms, key_digest, upstream_account_id
            ) VALUES (
                incoming.source, incoming.source_row_id, incoming.occurred_at_epoch_ms,
                incoming.key_digest, incoming.upstream_account_id
            )
            "#,
        )
        .bind(&key.source)
        .bind(key.source_row_id)
        .bind(key.occurred_at_epoch_ms)
        .bind(&key.key_digest)
        .bind(key.upstream_account_id)
        .execute(connection)
        .await
        .context("failed to upsert a PostgreSQL analytics parallel-work key")?;
    } else {
        sqlx::query(
            "DELETE FROM cvm_analytics.analytics_parallel_work_keys WHERE source = $1 AND source_row_id = $2",
        )
        .bind(source)
        .bind(source_row_id)
        .execute(connection)
        .await
        .context("failed to remove a PostgreSQL analytics parallel-work key")?;
    }
    Ok(())
}

fn validate_source_change_compatibility(
    change: &SourceChangeRow,
    required_fact_date_semantics_version: i64,
) -> Result<()> {
    anyhow::ensure!(
        change.event_version == crate::analytics_changes::ANALYTICS_SOURCE_EVENT_VERSION,
        "analytics source change {} uses unsupported journal event version {}; expected {}",
        change.change_id,
        change.event_version,
        crate::analytics_changes::ANALYTICS_SOURCE_EVENT_VERSION
    );
    if change.entity_kind == "invocation"
        && matches!(change.event_kind.as_str(), "upsert" | "tombstone")
    {
        anyhow::ensure!(
            change.fact_date_semantics_version == required_fact_date_semantics_version,
            "analytics source change {} uses fact-date semantics version {}; target requires {}; run explicit bootstrap to rebuild the bounded target snapshot",
            change.change_id,
            change.fact_date_semantics_version,
            required_fact_date_semantics_version
        );
    }
    Ok(())
}

async fn apply_replica_change(
    connection: &mut sqlx::postgres::PgConnection,
    change: &SourceChangeRow,
    parallel_work_key: Option<&AnalyticsParallelWorkKey>,
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
            replace_parallel_work_key(
                connection,
                &change.source,
                fact.source_row_id,
                parallel_work_key,
            )
            .await?;
        }
        ("tombstone", "invocation") => {
            let source_row_id = change
                .source_row_id
                .context("invocation tombstone is missing source row id")?;
            delete_analytics_fact(connection, &change.source, source_row_id).await?;
            replace_parallel_work_key(connection, &change.source, source_row_id, None).await?;
        }
        ("upsert", _) => {}
        // Archive coverage events are recorded in the Journal for source accounting, while the
        // corresponding facts are loaded from the completed archive during bootstrap. Treat the
        // event as an idempotent marker here; a newly appended archive sets the source replay
        // fence and is handled by the next fenced bootstrap attempt.
        ("coverage", _) => {}
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
    source_pool: &Pool<Sqlite>,
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
        validate_source_change_compatibility(change, state.fact_date_semantics_version)?;
    }
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
    let mut parallel_work_keys = HashMap::new();
    for change in batch {
        if change.event_kind != "upsert" || change.entity_kind != "invocation" {
            continue;
        }
        let source_row_id = change
            .source_row_id
            .context("invocation upsert is missing source row id")?;
        let key = if change.source == "codex_invocations" {
            let fact =
                serde_json::from_str::<AnalyticsFact>(&change.fact_json).with_context(|| {
                    format!(
                        "invalid invocation fact at source change {}",
                        change.change_id
                    )
                })?;
            parallel_work_key_for_fact(
                &fact,
                load_source_parallel_work_metadata(source_pool, source_row_id).await?,
            )
        } else {
            None
        };
        parallel_work_keys.insert(change.change_id, key);
    }
    let mut transaction = pool.begin().await?;
    for change in batch {
        apply_replica_change(
            &mut transaction,
            change,
            parallel_work_keys
                .get(&change.change_id)
                .and_then(Option::as_ref),
        )
        .await?;
        record_applied_change(&mut transaction, change).await?;
    }
    let updated = sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_replica_state
        SET generation = $1,
            owner_kind = $5,
            owner_epoch = $6,
            serving_fence = $7,
            coverage_generation = $1,
            coverage_revision = $2,
            coverage_watermark = $3,
            consumer_checkpoint = $3,
            status = 'running',
            service_eligible = FALSE,
            last_error_code = NULL,
            updated_at = NOW()
        WHERE id = 1
          AND consumer_checkpoint = $4
          AND owner_kind = $5
          AND owner_epoch = $6
          AND serving_fence = $7
        "#,
    )
    .bind(source.state.generation)
    .bind(source.state.coverage_revision)
    .bind(last_change_id)
    .bind(state.consumer_checkpoint)
    .bind(&source.state.owner_kind)
    .bind(source.state.owner_epoch)
    .bind(source.state.serving_fence)
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

async fn apply_snapshot_batch(
    connection: &mut sqlx::postgres::PgConnection,
    facts: &[AnalyticsFact],
    parallel_work_keys: &[AnalyticsParallelWorkKey],
) -> Result<()> {
    anyhow::ensure!(
        !facts.is_empty(),
        "analytics snapshot batch must not be empty"
    );
    let mut transaction = connection.begin().await?;
    for fact in facts {
        upsert_analytics_fact(&mut transaction, fact).await?;
    }
    for key in parallel_work_keys {
        replace_parallel_work_key(&mut transaction, &key.source, key.source_row_id, Some(key))
            .await?;
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
    let mut all_columns_ready = all_relations_ready;
    if all_columns_ready {
        for (relation, column) in PG_ANALYTICS_REQUIRED_COLUMNS {
            if !column_exists(pool, relation, column).await? {
                all_columns_ready = false;
            }
        }
    }
    let schema_ready = all_relations_ready && all_columns_ready;
    let source = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
    let mut state = if schema_ready {
        Some(load_replica_state(pool).await?)
    } else {
        None
    };
    if let Some(current_state) = state.as_ref()
        && current_state.status != "unbootstrapped"
        && current_state.status != "invalidated"
        && !fact_date_semantics_compatible(current_state)
    {
        let detail = fact_date_semantics_mismatch_detail(current_state);
        state = Some(invalidate_legacy_fact_date_semantics(pool, current_state, &detail).await?);
    }
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
            && !replica_can_recover_pending_generation_ack(current_state, &source)
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
        .as_mut()
        .map(|current_state| {
            let eligible = replica_service_eligible_for_source(current_state, &source);
            current_state.service_eligible = eligible;
            eligible
        })
        .unwrap_or(false);
    Ok(AnalyticsReplicaStatusReport {
        schema_ready,
        replica_id: ANALYTICS_REPLICA_ID.to_string(),
        service_eligible,
        state,
        source: Some(source_status(&source)),
    })
}

async fn snapshot_facts_into_target(
    connection: &mut sqlx::postgres::PgConnection,
    facts: &[AnalyticsFact],
    parallel_work_keys: &[AnalyticsParallelWorkKey],
) -> Result<()> {
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
        let source_row_ids = facts[offset..end]
            .iter()
            .map(|fact| fact.source_row_id)
            .collect::<HashSet<_>>();
        let parallel_work_keys = parallel_work_keys
            .iter()
            .filter(|key| source_row_ids.contains(&key.source_row_id))
            .cloned()
            .collect::<Vec<_>>();
        apply_snapshot_batch(connection, &facts[offset..end], &parallel_work_keys).await?;
        offset = end;
    }
    Ok(())
}

async fn snapshot_invocations_into_target(
    source_pool: &Pool<Sqlite>,
    connection: &mut sqlx::postgres::PgConnection,
) -> Result<(SourceContext, bool)> {
    let mut source_transaction = source_pool
        .begin()
        .await
        .context("failed to start the SQLite analytics baseline snapshot")?;
    let snapshot_source = load_source_context_transaction(&mut source_transaction).await?;
    let archive_rows =
        crate::stats::load_completed_invocation_archive_paths(&mut *source_transaction)
            .await
            .context(
                "failed to enumerate completed invocation archives for PostgreSQL bootstrap",
            )?;
    if snapshot_source.state.historical_replay_required != 0 && archive_rows.is_empty() {
        bail!(
            "SQLite requires historical archive replay but no completed invocation archive is available"
        );
    }
    let archive_replay_completed = !archive_rows.is_empty();
    for archive_row in archive_rows {
        let archive_path = archive_row.file_path().to_string();
        let Some((archive_pool, archive_cleanup)) =
            crate::stats::open_invocation_archive_batch_pool(
                &archive_row,
                "postgres-analytics-bootstrap",
            )
            .await
            .with_context(|| format!("failed to open invocation archive {archive_path}"))?
        else {
            bail!("invocation archive {archive_path} is unavailable for PostgreSQL bootstrap");
        };
        let result = snapshot_invocation_archive_into_target(&archive_pool, connection)
            .await
            .with_context(|| format!("failed to replay invocation archive {archive_path}"));
        archive_pool.close().await;
        drop(archive_cleanup);
        result?;
    }
    let mut after_id = 0;
    loop {
        let facts = load_source_invocation_snapshot_page(&mut source_transaction, after_id).await?;
        let Some(next_after_id) = facts.last().map(|fact| fact.source_row_id) else {
            break;
        };
        let metadata = load_source_parallel_work_metadata_page(
            &mut source_transaction,
            after_id,
            next_after_id,
        )
        .await?;
        let parallel_work_keys = parallel_work_keys_for_snapshot(&facts, metadata);
        snapshot_facts_into_target(connection, &facts, &parallel_work_keys).await?;
        after_id = next_after_id;
    }
    source_transaction
        .commit()
        .await
        .context("failed to commit the SQLite analytics baseline snapshot")?;
    Ok((snapshot_source, archive_replay_completed))
}

async fn load_source_context_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<SourceContext> {
    let state = sqlx::query_as::<_, SourceChangeState>(
        "SELECT generation, coverage_revision, retained_from_change_id, capacity, invalidation_pending, historical_replay_required, historical_history_unknown, owner_kind, owner_epoch, serving_fence, writer_id, serving_ack_change_id, journal_backpressure, analytics_unavailable, handoff_state FROM analytics_change_state WHERE id = 1",
    )
    .fetch_one(&mut **transaction)
    .await
    .context("analytics source change state is not initialized")?;
    let latest_change_id = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(change_id), 0) FROM analytics_change_events",
    )
    .fetch_one(&mut **transaction)
    .await
    .context("failed to load the latest analytics source change")?;
    let retained_bytes = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(length(CAST(fact_json AS BLOB))), 0) FROM analytics_change_events",
    )
    .fetch_one(&mut **transaction)
    .await
    .context("failed to measure the retained analytics source journal")?;
    let coverage = sqlx::query_as::<_, SourceCoverage>(
        "SELECT generation, covered_through_change_id, coverage_revision FROM analytics_change_coverage WHERE replica_id = ?1",
    )
    .bind(ANALYTICS_REPLICA_ID)
    .fetch_optional(&mut **transaction)
    .await
    .context("failed to load analytics source acknowledgement")?;
    Ok(SourceContext {
        state,
        latest_change_id,
        retained_bytes,
        coverage,
    })
}

async fn acknowledge_unverified_legacy_history(source_pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "UPDATE analytics_change_state SET historical_history_unknown = 0, historical_history_initialized = 1, updated_at = datetime('now') WHERE id = 1 AND historical_history_unknown <> 0",
    )
    .execute(source_pool)
    .await
    .context("failed to acknowledge unverified legacy analytics history")?;
    Ok(())
}

// Use a dedicated, short-lived session for control serialization. Holding a
// pooled connection while the operation uses that same pool deadlocks at pool
// size one. At most one additional session is needed per control operation;
// dropping it on cancellation releases its session advisory lock.
async fn acquire_replica_control_lock(pool: &sqlx::PgPool) -> Result<sqlx::postgres::PgConnection> {
    tokio::time::timeout(POSTGRES_CONNECTION_TIMEOUT, async {
        let mut guard =
            sqlx::postgres::PgConnection::connect_with(pool.connect_options().as_ref()).await?;
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(ANALYTICS_REPLICA_CONTROL_LOCK_KEY)
            .execute(&mut guard)
            .await?;
        Ok::<_, anyhow::Error>(guard)
    })
    .await
    .context("analytics replica control operation is busy")?
}

async fn bootstrap_replica(
    source_pool: &Pool<Sqlite>,
    pool: &sqlx::PgPool,
    acknowledge_unverified_history: bool,
) -> Result<AnalyticsReplicaStatusReport> {
    let _control_guard = acquire_replica_control_lock(pool).await?;
    bootstrap_replica_control_locked(source_pool, pool, acknowledge_unverified_history).await
}

async fn bootstrap_replica_control_locked(
    source_pool: &Pool<Sqlite>,
    pool: &sqlx::PgPool,
    acknowledge_unverified_history: bool,
) -> Result<AnalyticsReplicaStatusReport> {
    let mut connection = pool
        .acquire()
        .await
        .context("failed to acquire the PostgreSQL analytics bootstrap connection")?;
    // Session advisory locks survive PoolConnection::drop. Close this connection on cancellation
    // so a held lock can never be returned to the pool for a later retry.
    connection.close_on_drop();
    let acquired = sqlx::query_scalar::<_, bool>(PG_ANALYTICS_BOOTSTRAP_LOCK_ACQUIRE_SQL)
        .bind(ANALYTICS_REPLICA_BOOTSTRAP_LOCK_KEY)
        .fetch_one(&mut *connection)
        .await
        .context("failed to acquire the PostgreSQL analytics bootstrap lock")?;
    if !acquired {
        bail!("analytics replica bootstrap is already running");
    }
    let result =
        bootstrap_replica_locked(source_pool, &mut connection, acknowledge_unverified_history)
            .await;
    let unlock_result = sqlx::query_scalar::<_, bool>(PG_ANALYTICS_BOOTSTRAP_LOCK_RELEASE_SQL)
        .bind(ANALYTICS_REPLICA_BOOTSTRAP_LOCK_KEY)
        .fetch_one(&mut *connection)
        .await;
    match unlock_result {
        Ok(true) => {}
        Ok(false) => {
            let _ = connection.close().await;
            return match result {
                Ok(()) => Err(anyhow!(
                    "PostgreSQL analytics bootstrap lock was not held during release"
                )),
                Err(error) => Err(error
                    .context("PostgreSQL analytics bootstrap lock was not held during release")),
            };
        }
        Err(unlock_error) => {
            let _ = connection.close().await;
            return match result {
                Ok(()) => Err(anyhow!(
                    "failed to release the PostgreSQL analytics bootstrap lock: {unlock_error}"
                )),
                Err(error) => Err(error.context(format!(
                    "also failed to release the PostgreSQL analytics bootstrap lock: {unlock_error}"
                ))),
            };
        }
    }
    drop(connection);
    result?;
    let report = resume_replica_control_locked(source_pool, pool).await?;
    Ok(report)
}

async fn bootstrap_replica_locked(
    source_pool: &Pool<Sqlite>,
    connection: &mut sqlx::postgres::PgConnection,
    acknowledge_unverified_history: bool,
) -> Result<()> {
    fence_replica_service_eligibility_on_connection(connection).await?;
    verify_replica_schema_on_connection(connection).await?;
    let source_before = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
    clear_replica_target(connection, source_before.state.generation).await?;
    let (snapshot_source, archive_replay_completed) =
        match snapshot_invocations_into_target(source_pool, connection).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let detail = error.to_string();
                let error_code = if detail.contains("oversized analytics replica") {
                    "oversized_fact"
                } else {
                    "snapshot_failed"
                };
                mark_replica_blocked_on_connection(connection, error_code, detail.clone()).await?;
                return Err(anyhow!(detail));
            }
        };
    if source_before.state.historical_history_unknown != 0 {
        if !archive_replay_completed && !acknowledge_unverified_history {
            let detail = "SQLite historical coverage predates the analytics source journal and no completed invocation archive is available for controlled replay";
            mark_replica_blocked_on_connection(connection, "historical_source_unavailable", detail)
                .await?;
            return Err(anyhow!(detail));
        }
        acknowledge_unverified_legacy_history(source_pool).await?;
    }
    set_replica_baseline(connection, &snapshot_source).await?;
    if source_before.state.historical_replay_required != 0 {
        sqlx::query(
            "UPDATE analytics_change_state SET historical_replay_required = 0, updated_at = datetime('now') WHERE id = 1 AND generation = ?1",
        )
        .bind(snapshot_source.state.generation)
        .execute(source_pool)
        .await
        .context("failed to acknowledge completed SQLite archive replay")?;
    }
    let source_after = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
    anyhow::ensure!(
        source_after.state.generation == snapshot_source.state.generation,
        "analytics source generation changed during PostgreSQL bootstrap snapshot"
    );
    // The snapshot is valid for its captured watermark. Leave the replica running and let the
    // bounded Journal catch-up apply changes committed while archive replay ran.
    set_replica_acknowledged_on_connection(
        connection,
        snapshot_source.latest_change_id,
        false,
        snapshot_source.state.generation,
    )
    .await?;
    Ok(())
}

async fn resume_replica(
    source_pool: &Pool<Sqlite>,
    pool: &sqlx::PgPool,
) -> Result<AnalyticsReplicaStatusReport> {
    let _control_guard = acquire_replica_control_lock(pool).await?;
    resume_replica_control_locked(source_pool, pool).await
}

async fn resume_replica_control_locked(
    source_pool: &Pool<Sqlite>,
    pool: &sqlx::PgPool,
) -> Result<AnalyticsReplicaStatusReport> {
    verify_replica_schema(pool).await?;
    let mut state = load_replica_state(pool).await?;
    state = ensure_fact_date_semantics_compatible(pool, &state).await?;
    anyhow::ensure!(
        state.status != "unbootstrapped",
        "analytics replica is not bootstrapped; run maintenance analytics-replica bootstrap"
    );
    anyhow::ensure!(
        state.status != "invalidated",
        "analytics replica generation is invalidated; run explicit bootstrap before resume"
    );
    anyhow::ensure!(
        state.status != "bootstrapping",
        "analytics replica baseline is incomplete; run explicit bootstrap before resume"
    );
    anyhow::ensure!(
        replica_baseline_ready(&state),
        "analytics replica baseline is not established; run explicit bootstrap before resume"
    );
    if state.status == "paused" {
        // A paused replica is resumed by the same bounded drain below.
    } else if state.status == "blocked"
        && !matches!(
            state.last_error_code.as_deref(),
            Some("source_ack_failed")
                | Some("target_apply_failed")
                | Some("worker_batch_limit")
                | Some("owner_fence_unavailable")
        )
    {
        bail!(
            "analytics replica is blocked with {}: {}",
            state.last_error_code.as_deref().unwrap_or("unknown"),
            "inspect status"
        );
    }
    set_replica_running(pool).await?;
    state = load_replica_state(pool).await?;

    for _ in 0..ANALYTICS_REPLICA_MAX_BATCHES_PER_RUN {
        let source = load_source_context(source_pool, ANALYTICS_REPLICA_ID).await?;
        // A SQLite-owned source is valid for the pre-V2 PostgreSQL read replica. A
        // PostgreSQL-owned source must additionally prove its durable serving fence.
        if (source.state.owner_kind == "postgres" && !source_serving_fence_ready(&source))
            || source.state.analytics_unavailable != 0
            || source.state.journal_backpressure != 0
            || source.state.handoff_state != "stable"
        {
            let detail = format!(
                "analytics PostgreSQL owner fence is unavailable: owner_kind={}, owner_epoch={}, serving_fence={}, journal_backpressure={}, analytics_unavailable={}, handoff_state={}",
                source.state.owner_kind,
                source.state.owner_epoch,
                source.state.serving_fence,
                source.state.journal_backpressure,
                source.state.analytics_unavailable,
                source.state.handoff_state,
            );
            mark_replica_blocked(pool, "owner_fence_unavailable", detail.clone()).await?;
            return Err(anyhow!(detail));
        }
        if state.owner_kind != source.state.owner_kind
            || state.owner_epoch != source.state.owner_epoch
            || state.serving_fence != source.state.serving_fence
        {
            sync_replica_serving_fence(pool, &source).await?;
            state = load_replica_state(pool).await?;
        }
        if source.state.historical_history_unknown != 0 {
            let detail = "SQLite historical coverage predates the analytics source journal; inspect retained history and rerun maintenance analytics-replica bootstrap --acknowledge-unverified-history";
            mark_replica_blocked(pool, "historical_source_unavailable", detail).await?;
            return Err(anyhow!(detail));
        }
        let lag_bytes = load_source_lag_bytes(source_pool, state.consumer_checkpoint).await?;
        if source_retention_exhausted(&state, &source)
            && !replica_can_recover_pending_generation_ack(&state, &source)
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
        match source_ack_recovery_plan(&state, &source) {
            SourceAckRecoveryPlan::AlreadyCovered => {
                if state.source_acknowledged_through < state.consumer_checkpoint {
                    set_replica_acknowledged(
                        pool,
                        state.consumer_checkpoint,
                        state.consumer_checkpoint >= source.latest_change_id,
                        state.generation,
                    )
                    .await
                    .context(
                        "failed to persist the recovered analytics replica source acknowledgement",
                    )?;
                    state = load_replica_state(pool).await?;
                    continue;
                }
            }
            SourceAckRecoveryPlan::FullGeneration => {
                if let Err(error) =
                    crate::analytics_changes::acknowledge_analytics_change_generation(
                        source_pool,
                        ANALYTICS_REPLICA_ID,
                        source.state.generation,
                        source.latest_change_id,
                    )
                    .await
                {
                    let detail = error.to_string();
                    mark_replica_blocked(pool, "source_ack_failed", detail.clone()).await?;
                    return Err(anyhow!(detail));
                }
                set_replica_acknowledged(
                    pool,
                    source.latest_change_id,
                    true,
                    source.state.generation,
                )
                .await
                .context(
                    "failed to persist the recovered analytics replica generation acknowledgement",
                )?;
                state = load_replica_state(pool).await?;
                continue;
            }
            SourceAckRecoveryPlan::Range {
                from_change_id,
                through_change_id,
            } => {
                if let Err(error) = acknowledge_source_batch(
                    source_pool,
                    &source,
                    from_change_id,
                    through_change_id,
                )
                .await
                {
                    let detail = error.to_string();
                    mark_replica_blocked(pool, "source_ack_failed", detail.clone()).await?;
                    return Err(anyhow!(detail));
                }
                set_replica_acknowledged(
                    pool,
                    through_change_id,
                    through_change_id >= source.latest_change_id,
                    source.state.generation,
                )
                .await
                .context("failed to persist recovered analytics replica source acknowledgement")?;
                state = load_replica_state(pool).await?;
                continue;
            }
            SourceAckRecoveryPlan::Rebuild(detail) => {
                mark_replica_invalidated(pool, detail.clone()).await?;
                return Err(anyhow!(detail));
            }
            SourceAckRecoveryPlan::Inconsistent(detail) => {
                mark_replica_blocked(pool, "state_inconsistent", detail.clone()).await?;
                return Err(anyhow!(detail));
            }
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
            set_replica_acknowledged(pool, source.latest_change_id, true, source.state.generation)
                .await?;
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
        if let Some(error) = batch.iter().find_map(|change| {
            validate_source_change_compatibility(change, state.fact_date_semantics_version).err()
        }) {
            let detail = error.to_string();
            invalidate_legacy_fact_date_semantics(pool, &state, &detail).await?;
            return Err(anyhow!(detail));
        }
        let last_change_id =
            match apply_replica_batch(source_pool, pool, &state, &source, batch).await {
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
        set_replica_acknowledged(pool, last_change_id, caught_up, state.generation).await?;
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
    let _control_guard = acquire_replica_control_lock(pool).await?;
    verify_replica_schema(pool).await?;
    let state = load_replica_state(pool).await?;
    let state = ensure_fact_date_semantics_compatible(pool, &state).await?;
    anyhow::ensure!(
        state.status != "unbootstrapped",
        "analytics replica is not bootstrapped; run maintenance analytics-replica bootstrap"
    );
    anyhow::ensure!(
        state.status != "invalidated",
        "analytics replica generation is invalidated; run explicit bootstrap before pause"
    );
    match state.status.as_str() {
        "running" | "paused" | "caught_up" => {}
        "bootstrapping" => {
            bail!("analytics replica baseline is incomplete; run explicit bootstrap before pause")
        }
        "blocked" => {
            bail!(
                "analytics replica is blocked with {}; resume or run explicit bootstrap before pause",
                state.last_error_code.as_deref().unwrap_or("unknown")
            )
        }
        status => bail!("analytics replica cannot be paused from status {status}"),
    }
    anyhow::ensure!(
        replica_baseline_ready(&state),
        "analytics replica baseline is not established; run explicit bootstrap before pause"
    );
    mark_replica_state(pool, "paused", None, None, "preparing").await?;
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
    if state.status != "unbootstrapped" && !fact_date_semantics_compatible(&state) {
        let detail = fact_date_semantics_mismatch_detail(&state);
        if state.status != "invalidated" {
            state = invalidate_legacy_fact_date_semantics(pool, &state, &detail).await?;
        }
        if !fact_date_semantics_compatible(&state) {
            errors.push(fact_date_semantics_mismatch_detail(&state));
        }
    }
    let lag_bytes = load_source_lag_bytes(source_pool, state.consumer_checkpoint).await?;
    if state.status == "bootstrapping" {
        errors.push("replica baseline is incomplete; run explicit bootstrap".to_string());
    }
    if !replica_baseline_ready(&state) {
        errors.push("replica baseline state is not established".to_string());
    }
    if state.status != "unbootstrapped"
        && (source_retention_exhausted(&state, &source)
            && !replica_can_recover_pending_generation_ack(&state, &source)
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
    if state.consumer_checkpoint != state.source_acknowledged_through {
        errors.push(format!(
            "target checkpoint {} differs from source acknowledgement {}",
            state.consumer_checkpoint, state.source_acknowledged_through
        ));
    }
    if state.consumer_checkpoint != source.latest_change_id {
        errors.push(format!(
            "target checkpoint {} differs from current source latest change {}",
            state.consumer_checkpoint, source.latest_change_id
        ));
    }
    if source.state.invalidation_pending != 0 {
        errors.push("source generation still has pending invalidation".to_string());
    }
    if source.state.historical_replay_required != 0 {
        errors.push(
            "source has historical invocation coverage that requires archive replay".to_string(),
        );
    }
    if source.state.historical_history_unknown != 0 {
        errors.push(
            "source historical coverage predates the analytics source journal and is unverified"
                .to_string(),
        );
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
    let service_eligible = replica_service_eligible_for_source(&state, &source);
    if !service_eligible {
        errors.push("replica is not service eligible for the current source watermark".to_string());
    }
    let report = AnalyticsReplicaStatusReport {
        schema_ready: true,
        replica_id: ANALYTICS_REPLICA_ID.to_string(),
        service_eligible,
        state: Some(AnalyticsReplicaState {
            service_eligible,
            ..state
        }),
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
    let postgres = PgAnalyticsPool::connect_maintenance(config).await?;
    match command {
        AnalyticsReplicaCommand::Bootstrap {
            acknowledge_unverified_history,
        } => {
            let report = bootstrap_replica(
                source_pool,
                postgres.inner(),
                acknowledge_unverified_history,
            )
            .await?;
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
            Self::Internal(_) => ApiError::unavailable_with_code(
                ANALYTICS_UNAVAILABLE_CODE,
                AnalyticsUnavailable::new(
                    AnalyticsBackend::Postgres,
                    "PostgreSQL analytics read failed",
                ),
            ),
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
    fn archive_snapshot_query_accepts_legacy_optional_columns_but_requires_identity() {
        let columns = ["id", "invoke_id", "occurred_at", "source"]
            .into_iter()
            .map(str::to_string)
            .collect();
        let query = build_archive_invocation_snapshot_query(&columns).expect("query");
        assert!(query.contains("NULL AS first_token_ms"));
        assert!(query.contains("FROM codex_invocations"));
        assert!(
            build_archive_invocation_snapshot_query(&HashSet::from(["id".to_string()])).is_err()
        );
    }

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
    fn postgres_role_contract_requires_three_distinct_effective_roles() {
        let migration = PostgresDatabaseUrl::parse_for(
            "postgres://migration@db.example/analytics",
            ENV_POSTGRES_MIGRATION_URL,
        )
        .unwrap();
        let application =
            PostgresDatabaseUrl::parse("postgres://application@db.example/analytics").unwrap();
        let maintenance = PostgresDatabaseUrl::parse_for(
            "postgres://maintenance@db.example/analytics",
            ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL,
        )
        .unwrap();
        validate_postgres_migration_role_separation(
            Some(&migration),
            Some(&application),
            Some(&maintenance),
        )
        .unwrap();
        let duplicate = PostgresDatabaseUrl::parse_for(
            "postgres://application@other.example/analytics",
            ENV_POSTGRES_MIGRATION_URL,
        )
        .unwrap();
        assert!(
            validate_postgres_migration_role_separation(
                Some(&duplicate),
                Some(&application),
                Some(&maintenance),
            )
            .is_err()
        );
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
    fn postgres_error_sanitization_truncates_at_a_utf8_boundary() {
        let error = anyhow!("{}", "界".repeat(600));
        let sanitized = sanitize_postgres_error(&error, None);
        assert_eq!(sanitized, format!("{}...", "界".repeat(170)));
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
        assert_eq!(PG_ANALYTICS_REQUIRED_RELATIONS.len(), 8);
        assert_eq!(PG_ANALYTICS_REPLICA_REQUIRED_RELATIONS.len(), 2);
        assert_eq!(PG_ANALYTICS_REQUIRED_COLUMNS.len(), 5);
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("CREATE SCHEMA IF NOT EXISTS cvm_analytics"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_facts"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_daily_rollups"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_hourly_rollups"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_state"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_parallel_work_keys"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("owner_epoch"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("serving_fence"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("serving_ack_change_id"));
        assert!(
            PG_ANALYTICS_SCHEMA_SQL.contains("service_eligible BOOLEAN NOT NULL DEFAULT FALSE")
        );
        assert!(
            PG_ANALYTICS_SCHEMA_SQL
                .contains("fact_date_semantics_version BIGINT NOT NULL DEFAULT 2")
        );
        assert!(
            PG_ANALYTICS_SCHEMA_SQL
                .contains("ADD COLUMN IF NOT EXISTS fact_date_semantics_version")
        );
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains(
            "status = CASE WHEN status = 'caught_up' THEN 'bootstrapping' ELSE status END"
        ));
        assert!(PG_ANALYTICS_COLUMN_EXISTS_SQL.contains("pg_catalog.pg_attribute"));
        assert!(PG_ANALYTICS_RELATION_EXISTS_SQL.contains("pg_catalog.pg_class"));
        assert!(PG_ANALYTICS_RELATION_EXISTS_SQL.contains("pg_catalog.pg_namespace"));
        assert!(PG_ANALYTICS_COLUMN_EXISTS_SQL.contains("pg_catalog.pg_class"));
        assert!(PG_ANALYTICS_COLUMN_EXISTS_SQL.contains("pg_catalog.pg_namespace"));
        assert!(!PG_ANALYTICS_COLUMN_EXISTS_SQL.contains("to_regclass"));
        assert!(!PG_ANALYTICS_COLUMN_EXISTS_SQL.contains("information_schema.columns"));
        assert_ne!(ANALYTICS_REPLICA_BOOTSTRAP_LOCK_KEY, 0);
        assert!(PG_ANALYTICS_BOOTSTRAP_LOCK_ACQUIRE_SQL.contains("pg_try_advisory_lock"));
        assert!(PG_ANALYTICS_BOOTSTRAP_LOCK_RELEASE_SQL.contains("pg_advisory_unlock"));
        assert!(PG_ANALYTICS_READ_TRANSACTION_SQL.contains("REPEATABLE READ"));
        assert!(PG_ANALYTICS_READ_TRANSACTION_SQL.contains("READ ONLY"));
        assert!(PG_ANALYTICS_LEGACY_INVALIDATION_SQL.contains("fact_date_semantics_version = $1"));
        assert!(PG_ANALYTICS_LEGACY_INVALIDATION_SQL.contains("status = $2"));
        assert!(PG_ANALYTICS_LEGACY_INVALIDATION_SQL.contains("generation = $3"));
        assert!(PG_ANALYTICS_LEGACY_INVALIDATION_SQL.contains("consumer_checkpoint = $4"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("occurred_at_epoch_ms BIGINT NOT NULL"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_facts_projection_cursor_idx"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("analytics_facts_account_projection_cursor_idx"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains(
            "ON cvm_analytics.analytics_facts (stats_date, occurred_at_epoch_ms, source, source_row_id)"
        ));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains(
            "upstream_account_id, stats_date, occurred_at_epoch_ms, source, source_row_id"
        ));
        assert!(!PG_ANALYTICS_SCHEMA_SQL.contains("payload"));
        assert!(!PG_ANALYTICS_SCHEMA_SQL.contains("requester_ip"));
        assert!(!PG_ANALYTICS_SCHEMA_SQL.contains("prompt_cache"));
        assert!(!PG_ANALYTICS_SCHEMA_SQL.contains("last_error TEXT"));
        assert!(PG_ANALYTICS_SCHEMA_SQL.contains("last_error_detail IS NULL"));
    }

    #[test]
    fn current_fact_date_semantics_uses_versioned_state_and_legacy_fence() {
        let mut state = AnalyticsReplicaState {
            replica_id: ANALYTICS_REPLICA_ID.to_string(),
            status: "caught_up".to_string(),
            owner_kind: "postgres".to_string(),
            owner_epoch: 1,
            serving_fence: 1,
            serving_ack_change_id: 100,
            fact_date_semantics_version: ANALYTICS_FACT_DATE_SEMANTICS_VERSION,
            generation: 4,
            baseline_generation: 4,
            baseline_change_id: 12,
            coverage_generation: 4,
            coverage_revision: 13,
            coverage_watermark: 12,
            consumer_checkpoint: 12,
            source_acknowledged_through: 12,
            service_eligible: false,
            last_error_code: None,
            updated_at: String::new(),
        };
        assert!(fact_date_semantics_compatible(&state));
        assert!(!state.service_eligible);
        assert!(replica_service_eligible(&state));

        state.owner_kind = "sqlite".to_string();
        state.serving_ack_change_id = 0;
        assert!(!replica_service_eligible(&state));
        assert!(replica_read_eligible(&state));
        state.owner_kind = "postgres".to_string();
        assert!(!replica_read_eligible(&state));
        state.serving_ack_change_id = state.consumer_checkpoint;

        state.status = "bootstrapping".to_string();
        assert!(!replica_service_eligible(&state));

        state.fact_date_semantics_version = 1;
        assert!(!fact_date_semantics_compatible(&state));
        assert!(!replica_service_eligible(&state));
        assert!(fact_date_semantics_mismatch_detail(&state).contains("run explicit bootstrap"));
    }

    #[test]
    fn ownership_runtime_requires_postgres_for_enabled_cutover() {
        assert!(validate_ownership_runtime_admission(AnalyticsOwnershipV2::Disabled).is_ok());
        assert!(validate_ownership_runtime_admission(AnalyticsOwnershipV2::Enabled).is_ok());
        let error = validate_ownership_runtime_admission_for_backend(
            AnalyticsBackend::Sqlite,
            AnalyticsOwnershipV2::Enabled,
        )
        .expect_err("enabled ownership must require the PostgreSQL backend");
        assert!(
            error
                .to_string()
                .contains("requires ANALYTICS_BACKEND=postgres")
        );
    }

    #[test]
    fn ownership_handoff_admission_requires_all_readiness_facts() {
        let ready = AnalyticsOwnershipReadiness {
            replica_facts_caught_up: true,
            long_term_projection_ready: true,
            long_term_projection_parity: true,
        };
        assert_eq!(
            admit_analytics_ownership_transition(
                AnalyticsBackend::Postgres,
                AnalyticsOwnershipV2::Enabled,
                "sqlite",
                ready,
            )
            .expect("ready handoff admission"),
            AnalyticsOwnershipTransition::ClaimPostgres
        );
        for readiness in [
            AnalyticsOwnershipReadiness {
                replica_facts_caught_up: false,
                ..ready
            },
            AnalyticsOwnershipReadiness {
                long_term_projection_ready: false,
                ..ready
            },
            AnalyticsOwnershipReadiness {
                long_term_projection_parity: false,
                ..ready
            },
        ] {
            assert_eq!(
                admit_analytics_ownership_transition(
                    AnalyticsBackend::Postgres,
                    AnalyticsOwnershipV2::Enabled,
                    "sqlite",
                    readiness,
                )
                .expect("closed handoff admission"),
                AnalyticsOwnershipTransition::StayUnavailable
            );
        }
        assert_eq!(
            admit_analytics_ownership_transition(
                AnalyticsBackend::Postgres,
                AnalyticsOwnershipV2::Disabled,
                "postgres",
                ready,
            )
            .expect("disabled rollback admission"),
            AnalyticsOwnershipTransition::StayUnavailable
        );
    }

    #[tokio::test]
    async fn enabled_ownership_is_unavailable_for_both_backend_selections() {
        for backend in [AnalyticsBackend::Sqlite, AnalyticsBackend::Postgres] {
            let mut config = crate::tests::test_config();
            config.analytics_backend = backend;
            config.analytics_ownership = AnalyticsOwnershipV2::Enabled;
            let runtime = PgAnalyticsRuntime::initialize(&config).await;
            assert_eq!(
                runtime.status().availability,
                AnalyticsAvailability::Unavailable
            );
            assert!(runtime.pool.is_none());
            let reason = runtime.status().reason.unwrap();
            if backend == AnalyticsBackend::Sqlite {
                assert!(reason.contains("requires ANALYTICS_BACKEND=postgres"));
            }
        }
    }

    #[tokio::test]
    async fn source_snapshot_loads_the_complete_ownership_state() -> Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        crate::analytics_changes::ensure_analytics_changes_schema(&pool).await?;
        let mut tx = pool.begin().await?;
        let source = load_source_context_transaction(&mut tx).await?;
        assert_eq!(source.state.owner_kind, "sqlite");
        assert_eq!(source.state.owner_epoch, 1);
        assert_eq!(source.state.serving_fence, 1);
        assert_eq!(source.state.handoff_state, "stable");
        tx.rollback().await?;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an isolated PostgreSQL *_fixture database and explicit schema reset authorization"]
    async fn postgres_runtime_continues_replay_after_ready_and_restart() -> Result<()> {
        anyhow::ensure!(
            env::var("PG_ANALYTICS_TEST_ALLOW_SCHEMA_RESET").as_deref() == Ok("1"),
            "fixture reset authorization required"
        );
        let target = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&env::var("PG_ANALYTICS_TEST_OWNER_URL")?)
            .await?;
        let database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&target)
            .await?;
        anyhow::ensure!(
            database.ends_with("_fixture"),
            "isolated fixture database required"
        );
        sqlx::raw_sql("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
            .execute(&target)
            .await?;
        let result = async {
            {
                let mut connection = target.acquire().await?;
                migrations::apply_ordered_migrations(&mut connection).await?;
            }
            let guard = acquire_replica_control_lock(&target).await?;
            let holding = tokio::spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            });
            holding.abort();
            let _ = holding.await;
            drop(acquire_replica_control_lock(&target).await?);
            let source = SqlitePoolOptions::new().max_connections(1)
                .connect("sqlite::memory:").await?;
            crate::schema::ensure_schema(&source).await?;
            let make_runtime = || Arc::new(PgAnalyticsRuntime {
                backend: AnalyticsBackend::Postgres,
                ownership: AnalyticsOwnershipV2::Disabled,
                pool: Some(PgAnalyticsPool { pool: target.clone() }),
                maintenance_pool: Some(PgAnalyticsPool { pool: target.clone() }),
                unavailable: None,
                ready: Arc::new(AtomicBool::new(false)),
            });
            let runtime = make_runtime();
            runtime.bootstrap_once(&source).await?;
            assert!(runtime.ready.load(Ordering::Acquire));
            let baseline = load_replica_state(&target).await?;
            let stop = CancellationToken::new();
            runtime.spawn_bootstrap_lifecycle(source.clone(), stop.clone());
            for index in 1..=2 {
                let mut tx = source.begin().await?;
                let row_id = sqlx::query("INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (?1, '2026-10-30 12:00:00', 'proxy', 'success', 7, 0.07, '{}', '{}')")
                    .bind(format!("lifecycle-{index}")).execute(&mut *tx).await?.last_insert_rowid();
                crate::analytics_changes::append_invocation_upsert_for_row_tx(tx.as_mut(), row_id).await?;
                tx.commit().await?;
                tokio::time::timeout(Duration::from_secs(20), async {
                    loop {
                        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM cvm_analytics.analytics_facts")
                            .fetch_one(&target).await?;
                        let state = load_replica_state(&target).await?;
                        if count == index && replica_read_eligible(&state) {
                            assert_eq!(state.baseline_change_id, baseline.baseline_change_id, "replay must not rebuild the baseline");
                            break Ok::<(), anyhow::Error>(());
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }).await??;
            }
            pause_replica(&source, &target).await?;
            tokio::time::sleep(Duration::from_secs(3)).await;
            assert_eq!(load_replica_state(&target).await?.status, "paused", "the active worker must preserve a maintenance pause");
            assert!(!runtime.ready.load(Ordering::Acquire));
            stop.cancel();
            // Cancellation must also end an idle worker; the next process resumes the same baseline.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let checkpoint = load_replica_state(&target).await?.consumer_checkpoint;
            resume_replica(&source, &target).await?;
            let restarted = make_runtime();
            restarted.bootstrap_once(&source).await?;
            let resumed = load_replica_state(&target).await?;
            assert_eq!(resumed.consumer_checkpoint, checkpoint);
            assert_eq!(resumed.baseline_change_id, baseline.baseline_change_id);
            sqlx::query("UPDATE cvm_analytics.analytics_replica_state SET status='paused' WHERE id=1").execute(&target).await?;
            restarted.bootstrap_once(&source).await?;
            assert!(!restarted.ready.load(Ordering::Acquire));
            assert_eq!(load_replica_state(&target).await?.status, "paused");
            source.close().await;
            Ok::<(), anyhow::Error>(())
        }.await;
        let cleanup = sqlx::raw_sql("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
            .execute(&target)
            .await;
        target.close().await;
        result?;
        cleanup?;
        Ok(())
    }

    #[test]
    fn invocation_source_changes_require_current_fact_date_semantics() {
        let mut change = SourceChangeRow {
            change_id: 7,
            event_version: crate::analytics_changes::ANALYTICS_SOURCE_EVENT_VERSION,
            fact_date_semantics_version: ANALYTICS_FACT_DATE_SEMANTICS_VERSION,
            event_kind: "upsert".to_string(),
            entity_kind: "invocation".to_string(),
            source: "codex_invocations".to_string(),
            source_row_id: Some(42),
            entity_key: "invocation:proxy:42".to_string(),
            generation: 1,
            coverage_revision: 1,
            fact_json: "{}".to_string(),
        };
        validate_source_change_compatibility(&change, ANALYTICS_FACT_DATE_SEMANTICS_VERSION)
            .expect("current invocation source change should be accepted");

        change.fact_date_semantics_version = 1;
        let error =
            validate_source_change_compatibility(&change, ANALYTICS_FACT_DATE_SEMANTICS_VERSION)
                .expect_err("legacy invocation source change should fail closed");
        assert!(error.to_string().contains("run explicit bootstrap"));

        change.event_kind = "tombstone".to_string();
        let error =
            validate_source_change_compatibility(&change, ANALYTICS_FACT_DATE_SEMANTICS_VERSION)
                .expect_err("legacy invocation tombstone should fail closed");
        assert!(error.to_string().contains("fact-date semantics version"));
    }

    #[test]
    fn parallel_work_key_digest_preserves_trimmed_membership_without_raw_keys() {
        let first = parallel_work_key_digest(Some(r#"{"promptCacheKey":"  same-key  "}"#))
            .expect("valid prompt-cache key");
        let second = parallel_work_key_digest(Some(r#"{"promptCacheKey":"same-key"}"#))
            .expect("valid prompt-cache key");
        let different = parallel_work_key_digest(Some(r#"{"promptCacheKey":"other-key"}"#))
            .expect("valid prompt-cache key");

        assert_eq!(first, second);
        assert_ne!(first, different);
        assert!(first.starts_with("sha256:"));
        assert!(!first.contains("same-key"));
        assert!(parallel_work_key_digest(Some("not-json")).is_none());
        assert!(parallel_work_key_digest(Some(r#"{"promptCacheKey":"  "}"#)).is_none());
    }

    #[tokio::test]
    async fn selected_postgres_without_a_ready_replica_fails_closed_for_usage_reads() {
        let runtime = PgAnalyticsRuntime {
            backend: AnalyticsBackend::Postgres,
            ownership: AnalyticsOwnershipV2::Disabled,
            pool: None,
            maintenance_pool: None,
            unavailable: None,
            ready: Arc::new(AtomicBool::new(false)),
        };
        let error = query_account_window_usage(
            &runtime,
            &[7],
            Utc::now() - ChronoDuration::minutes(5),
            Utc::now(),
        )
        .await
        .expect_err("usage reads must not fall back to SQLite");

        assert!(matches!(
            error,
            AnalyticsReadError::Unavailable(AnalyticsUnavailable {
                backend: AnalyticsBackend::Postgres,
                ..
            })
        ));
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
            owner_kind: "postgres".to_string(),
            owner_epoch: 1,
            serving_fence: 1,
            serving_ack_change_id: 100,
            fact_date_semantics_version: ANALYTICS_FACT_DATE_SEMANTICS_VERSION,
            generation: 3,
            baseline_generation: 3,
            baseline_change_id: 10,
            coverage_generation: 3,
            coverage_revision: 11,
            coverage_watermark: 10,
            consumer_checkpoint: 10,
            source_acknowledged_through: 10,
            service_eligible: false,
            last_error_code: None,
            updated_at: String::new(),
        };
        let source = SourceContext {
            state: SourceChangeState {
                generation: 3,
                coverage_revision: 12,
                retained_from_change_id: 11,
                capacity: 10_000,
                invalidation_pending: 0,
                historical_replay_required: 0,
                historical_history_unknown: 0,
                owner_kind: "postgres".to_string(),
                owner_epoch: 1,
                serving_fence: 1,
                writer_id: None,
                serving_ack_change_id: 100,
                journal_backpressure: 0,
                analytics_unavailable: 0,
                handoff_state: "stable".to_string(),
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

        let mut pending_generation = next_generation;
        pending_generation.state.generation = 3;
        pending_generation.state.invalidation_pending = 1;
        pending_generation.latest_change_id = state.consumer_checkpoint;
        assert!(source_retention_exhausted(&state, &pending_generation));
        assert!(replica_can_recover_pending_generation_ack(
            &state,
            &pending_generation
        ));
        pending_generation.latest_change_id += 1;
        assert!(!replica_can_recover_pending_generation_ack(
            &state,
            &pending_generation
        ));
    }

    #[test]
    fn interrupted_batch_cannot_be_service_eligible_before_source_ack() {
        let mut state = AnalyticsReplicaState {
            replica_id: ANALYTICS_REPLICA_ID.to_string(),
            status: "running".to_string(),
            owner_kind: "postgres".to_string(),
            owner_epoch: 1,
            serving_fence: 1,
            serving_ack_change_id: 100,
            fact_date_semantics_version: ANALYTICS_FACT_DATE_SEMANTICS_VERSION,
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
            updated_at: String::new(),
        };
        assert!(!replica_service_eligible(&state));
        let source = SourceContext {
            state: SourceChangeState {
                generation: 1,
                coverage_revision: 8,
                retained_from_change_id: 1,
                capacity: 10_000,
                invalidation_pending: 0,
                historical_replay_required: 0,
                historical_history_unknown: 0,
                owner_kind: "postgres".to_string(),
                owner_epoch: 1,
                serving_fence: 1,
                writer_id: None,
                serving_ack_change_id: 100,
                journal_backpressure: 0,
                analytics_unavailable: 0,
                handoff_state: "stable".to_string(),
            },
            latest_change_id: 7,
            retained_bytes: 128,
            coverage: Some(SourceCoverage {
                generation: 1,
                covered_through_change_id: 4,
                coverage_revision: 8,
            }),
        };
        assert!(!replica_service_eligible_for_source(&state, &source));

        state.status = "caught_up".to_string();
        state.source_acknowledged_through = 7;
        assert!(!state.service_eligible);
        assert!(replica_service_eligible(&state));
        assert!(replica_service_eligible_for_latest_change(&state, 7));
        assert!(!replica_service_eligible_for_latest_change(&state, 8));
        let mut acknowledged_source = source.clone();
        acknowledged_source.coverage = Some(SourceCoverage {
            covered_through_change_id: 7,
            ..source.coverage.expect("coverage fixture")
        });
        assert!(replica_service_eligible_for_source(
            &state,
            &acknowledged_source
        ));
        acknowledged_source.coverage = None;
        assert!(!replica_service_eligible_for_source(
            &state,
            &acknowledged_source
        ));
        acknowledged_source.state.historical_replay_required = 1;
        assert!(!replica_service_eligible_for_source(
            &state,
            &acknowledged_source
        ));
    }

    #[test]
    fn baseline_and_source_ack_recovery_ranges_are_explicit() {
        let mut state = AnalyticsReplicaState {
            replica_id: ANALYTICS_REPLICA_ID.to_string(),
            status: "running".to_string(),
            owner_kind: "postgres".to_string(),
            owner_epoch: 1,
            serving_fence: 1,
            serving_ack_change_id: 100,
            fact_date_semantics_version: ANALYTICS_FACT_DATE_SEMANTICS_VERSION,
            generation: 2,
            baseline_generation: 2,
            baseline_change_id: 4,
            coverage_generation: 2,
            coverage_revision: 9,
            coverage_watermark: 7,
            consumer_checkpoint: 7,
            source_acknowledged_through: 4,
            service_eligible: false,
            last_error_code: None,
            updated_at: String::new(),
        };

        assert!(replica_baseline_ready(&state));
        let source = SourceContext {
            state: SourceChangeState {
                generation: 2,
                coverage_revision: 9,
                retained_from_change_id: 1,
                capacity: 10_000,
                invalidation_pending: 0,
                historical_replay_required: 0,
                historical_history_unknown: 0,
                owner_kind: "postgres".to_string(),
                owner_epoch: 1,
                serving_fence: 1,
                writer_id: None,
                serving_ack_change_id: 100,
                journal_backpressure: 0,
                analytics_unavailable: 0,
                handoff_state: "stable".to_string(),
            },
            latest_change_id: 9,
            retained_bytes: 128,
            coverage: Some(SourceCoverage {
                generation: 2,
                covered_through_change_id: 4,
                coverage_revision: 9,
            }),
        };
        assert_eq!(
            source_ack_recovery_plan(&state, &source),
            SourceAckRecoveryPlan::Range {
                from_change_id: 5,
                through_change_id: 7,
            }
        );
        let mut covered_source = source.clone();
        covered_source.coverage = Some(SourceCoverage {
            covered_through_change_id: 7,
            ..source.coverage.expect("coverage fixture")
        });
        assert_eq!(
            source_ack_recovery_plan(&state, &covered_source),
            SourceAckRecoveryPlan::AlreadyCovered
        );
        let mut complete_source = source.clone();
        complete_source.latest_change_id = state.consumer_checkpoint;
        complete_source.coverage = None;
        assert_eq!(
            source_ack_recovery_plan(&state, &complete_source),
            SourceAckRecoveryPlan::FullGeneration
        );
        let mut retention_gap_source = source.clone();
        retention_gap_source.state.retained_from_change_id = 6;
        assert!(matches!(
            source_ack_recovery_plan(&state, &retention_gap_source),
            SourceAckRecoveryPlan::Rebuild(_)
        ));

        state.baseline_generation = 1;
        assert!(!replica_baseline_ready(&state));
        state.baseline_generation = 2;
        state.source_acknowledged_through = 7;
        assert!(matches!(
            source_ack_recovery_plan(&state, &covered_source),
            SourceAckRecoveryPlan::AlreadyCovered
        ));
    }

    #[test]
    fn snapshot_fact_uses_incremental_normalization_contract() {
        let long_model = "m".repeat(300);
        let fact = snapshot_row_to_fact(SourceInvocationSnapshotRow {
            id: 42,
            invoke_id: " invoke-42 ".to_string(),
            occurred_at: "2026-10-31T00:00:00Z".to_string(),
            occurred_at_epoch_ms: 1,
            stats_date: "2026-10-31".to_string(),
            source: " source ".to_string(),
            model: Some(format!("  {long_model}\0  ")),
            upstream_account_id: None,
            status: Some(" completed \0".to_string()),
            failure_kind: Some(" failure ".to_string()),
            failure_class: Some(" class ".to_string()),
            payload: Some(
                r#"{"requestModel":"  request\u0000model  ","upstreamAccountId":"42","upstreamAccountName":"  account  "}"#
                    .to_string(),
            ),
            ..Default::default()
        })
        .expect("snapshot row should normalize into a fact");

        let expected_model = "m".repeat(256);
        assert_eq!(fact.source, "source");
        assert_eq!(fact.invoke_id, "invoke-42");
        assert_eq!(fact.model.as_deref(), Some(expected_model.as_str()));
        assert_eq!(fact.request_model.as_deref(), Some("requestmodel"));
        assert_eq!(fact.upstream_account_id, Some(42));
        assert_eq!(fact.upstream_account_name.as_deref(), Some("account"));
        assert_eq!(fact.status.as_deref(), Some("completed"));
        assert_eq!(fact.failure_kind.as_deref(), Some("failure"));
        assert_eq!(fact.failure_class.as_deref(), Some("service_failure"));
    }

    #[test]
    fn snapshot_fact_prefers_payload_failure_kind_for_projection_parity() {
        let payload_failure = snapshot_row_to_fact(SourceInvocationSnapshotRow {
            id: 43,
            invoke_id: "invoke-43".to_string(),
            occurred_at: "2026-10-31T00:00:00Z".to_string(),
            stats_date: "2026-10-31".to_string(),
            source: "proxy".to_string(),
            status: Some("completed".to_string()),
            failure_kind: Some("none".to_string()),
            payload: Some(r#"{"failureKind":"upstream_stream_error"}"#.to_string()),
            ..Default::default()
        })
        .expect("payload failure kind should classify before projection");
        assert_eq!(
            payload_failure.failure_kind.as_deref(),
            Some("upstream_stream_error")
        );
        assert_eq!(
            payload_failure.failure_class.as_deref(),
            Some("service_failure")
        );
        assert!(!payload_failure.success_like);

        let payload_none = snapshot_row_to_fact(SourceInvocationSnapshotRow {
            id: 44,
            invoke_id: "invoke-44".to_string(),
            occurred_at: "2026-10-31T00:00:00Z".to_string(),
            stats_date: "2026-10-31".to_string(),
            source: "proxy".to_string(),
            status: Some("completed".to_string()),
            failure_kind: Some("upstream_stream_error".to_string()),
            payload: Some(r#"{"failureKind":"none"}"#.to_string()),
            ..Default::default()
        })
        .expect("explicit payload none should retain success classification");
        assert_eq!(payload_none.failure_kind, None);
        assert_eq!(payload_none.failure_class, None);
        assert!(payload_none.success_like);

        let downstream_error = snapshot_row_to_fact(SourceInvocationSnapshotRow {
            id: 45,
            invoke_id: "invoke-45".to_string(),
            occurred_at: "2026-10-31T00:00:00Z".to_string(),
            stats_date: "2026-10-31".to_string(),
            source: "proxy".to_string(),
            status: Some("completed".to_string()),
            failure_kind: Some("none".to_string()),
            payload: Some(
                r#"{"downstreamErrorMessage":"downstream closed while streaming upstream response"}"#
                    .to_string(),
            ),
            ..Default::default()
        })
        .expect("downstream payload error should classify as an abort");
        assert_eq!(
            downstream_error.failure_class.as_deref(),
            Some("client_abort")
        );
        assert!(!downstream_error.success_like);
    }

    #[test]
    fn snapshot_fact_resolves_legacy_error_message_before_projection() {
        let fact = snapshot_row_to_fact(SourceInvocationSnapshotRow {
            id: 44,
            invoke_id: "invoke-44".to_string(),
            occurred_at: "2026-10-31T00:00:00Z".to_string(),
            stats_date: "2026-10-31".to_string(),
            source: "proxy".to_string(),
            status: Some("http_200".to_string()),
            error_message: Some("failed to contact upstream".to_string()),
            failure_class: Some("none".to_string()),
            is_actionable: Some(0),
            ..Default::default()
        })
        .expect("legacy error message should classify before fact persistence");

        assert!(!fact.success_like);
        assert_eq!(fact.failure_class.as_deref(), Some("service_failure"));
        assert!(fact.failure_kind.is_some());
        assert_eq!(fact.is_actionable, Some(true));
    }

    #[tokio::test]
    async fn snapshot_page_uses_shared_normalization_bounds_raw_text_and_shanghai_fact_dates() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect snapshot normalization test database");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL,
                model TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_input_tokens INTEGER,
                reasoning_tokens INTEGER,
                total_tokens INTEGER,
                cost REAL,
                cost_input REAL,
                cost_cache_write REAL,
                cost_cache_read REAL,
                cost_output REAL,
                cost_reasoning REAL,
                status TEXT,
                error_message TEXT,
                failure_kind TEXT,
                failure_class TEXT,
                is_actionable INTEGER,
                payload TEXT,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                t_upstream_stream_ms REAL,
                first_token_ms REAL,
                t_resp_parse_ms REAL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create snapshot normalization table");
        sqlx::query(
            r#"
            CREATE TABLE pool_upstream_request_attempts (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                attempt_index INTEGER NOT NULL,
                upstream_account_id INTEGER
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create snapshot account fallback table");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, model, payload) VALUES (1, ?1, ?2, ?3, CAST(X'6162006364' AS TEXT), ?4)",
        )
        .bind("invoke-nul")
        .bind("2026-10-31T00:00:00Z")
        .bind("proxy")
        .bind(r#"{"requestModel":"request\u0000model","upstreamAccountName":"account\u0000name","upstreamAccountId":"4\u00002"}"#)
        .execute(&pool)
        .await
            .expect("insert NUL snapshot normalization row");

        let mut connection = pool
            .acquire()
            .await
            .expect("acquire snapshot normalization connection");
        let facts = load_source_invocation_snapshot_page(connection.as_mut(), 0)
            .await
            .expect("load snapshot normalization row");
        let fact = facts.into_iter().next().expect("snapshot fact");
        assert_eq!(fact.model.as_deref(), Some("abcd"));
        assert_eq!(fact.request_model.as_deref(), Some("requestmodel"));
        assert_eq!(fact.upstream_account_name.as_deref(), Some("accountname"));
        assert_eq!(fact.upstream_account_id, None);
        assert_eq!(fact.stats_date, "2026-10-31");

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (id, invoke_id, occurred_at, source)
            VALUES
                (2, 'invoke-shanghai-before-boundary', '2026-10-30T15:59:59Z', 'proxy'),
                (3, 'invoke-shanghai-at-boundary', '2026-10-30T16:00:00Z', 'proxy'),
                (4, 'invoke-shanghai-naive', '2026-10-31 00:00:00', 'proxy')
            "#,
        )
        .execute(connection.as_mut())
        .await
        .expect("insert Shanghai fact-date boundary rows");
        let boundary_facts = load_source_invocation_snapshot_page(connection.as_mut(), 1)
            .await
            .expect("load Shanghai fact-date boundary rows");
        assert_eq!(
            boundary_facts
                .iter()
                .map(|fact| (fact.source_row_id, fact.stats_date.as_str()))
                .collect::<Vec<_>>(),
            vec![(2, "2026-10-30"), (3, "2026-10-31"), (4, "2026-10-31"),]
        );

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, payload)
            VALUES (5, 'invoke-attempt-account', '2026-10-31T00:00:00Z', 'proxy', ?1)
            "#,
        )
        .bind(r#"{"upstreamAccountId":"invalid","upstreamAccountName":"attempt-account"}"#)
        .execute(connection.as_mut())
        .await
        .expect("insert attempt-account snapshot row");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts
                (id, invoke_id, occurred_at, attempt_index, upstream_account_id)
            VALUES (1, 'invoke-attempt-account', '2026-10-31T00:00:00.000Z', 1, 42)
            "#,
        )
        .execute(connection.as_mut())
        .await
        .expect("insert attempt-account fallback row");
        let fallback_facts = load_source_invocation_snapshot_page(connection.as_mut(), 4)
            .await
            .expect("load attempt-account fallback row");
        assert_eq!(fallback_facts.len(), 1);
        assert_eq!(fallback_facts[0].upstream_account_id, Some(42));

        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, model) VALUES (6, 'invoke-oversized', '2026-10-31T00:00:00Z', 'proxy', ?1)",
        )
        .bind("x".repeat((ANALYTICS_REPLICA_MAX_SOURCE_TEXT_BYTES + 1) as usize))
        .execute(connection.as_mut())
        .await
        .expect("insert oversized snapshot normalization row");
        let error = load_source_invocation_snapshot_page(connection.as_mut(), 0)
            .await
            .expect_err("oversized raw source text must fail before materialization");
        assert!(
            error
                .to_string()
                .contains("oversized analytics replica source field model")
        );
    }

    async fn apply_fixture_schema(
        owner: &mut sqlx::postgres::PgConnection,
        context: &str,
    ) -> Result<()> {
        let mut transaction = owner.begin().await?;
        // Semicolons inside SQL comments are not statement delimiters.
        sqlx::raw_sql(PG_ANALYTICS_SCHEMA_SQL)
            .execute(&mut *transaction)
            .await
            .with_context(|| context.to_string())?;
        transaction
            .commit()
            .await
            .with_context(|| format!("commit {context}"))?;
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an explicitly authorized disposable PostgreSQL *_fixture database and owner/app/maintenance URLs"]
    async fn postgres_analytics_fixture_verifies_role_separation_and_bounded_reset() -> Result<()> {
        let reset_authorization = env::var("PG_ANALYTICS_TEST_ALLOW_SCHEMA_RESET").context(
            "PG_ANALYTICS_TEST_ALLOW_SCHEMA_RESET=1 is required for the destructive fixture",
        )?;
        anyhow::ensure!(
            reset_authorization == "1",
            "PG_ANALYTICS_TEST_ALLOW_SCHEMA_RESET must equal 1 for the destructive fixture"
        );
        let owner_url = env::var("PG_ANALYTICS_TEST_OWNER_URL")
            .context("PG_ANALYTICS_TEST_OWNER_URL is required for the PostgreSQL fixture")?;
        let app_url = env::var("PG_ANALYTICS_TEST_APP_URL")
            .context("PG_ANALYTICS_TEST_APP_URL is required for the PostgreSQL fixture")?;
        let maintenance_url = env::var("PG_ANALYTICS_TEST_MAINTENANCE_URL")
            .context("PG_ANALYTICS_TEST_MAINTENANCE_URL is required for the PostgreSQL fixture")?;

        let mut owner = sqlx::postgres::PgConnection::connect(&owner_url)
            .await
            .context("connect PostgreSQL fixture owner")?;
        let database_name: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&mut owner)
            .await
            .context("inspect PostgreSQL fixture database name")?;
        anyhow::ensure!(
            database_name.ends_with("_fixture"),
            "PostgreSQL fixture owner URL must target a database whose name ends with _fixture"
        );
        sqlx::query("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
            .execute(&mut owner)
            .await
            .context("reset PostgreSQL fixture schema")?;
        // Keep cleanup outside the fallible body so every setup/assertion error drops the
        // fixture pools before the destructive schema is removed.
        let fixture_result = async {
            apply_fixture_schema(&mut owner, "apply PostgreSQL fixture schema").await?;
            for statement in [
                "GRANT USAGE ON SCHEMA cvm_analytics TO cvm_analytics_reader",
                "GRANT SELECT (fact_version, source, source_row_id, invoke_id, occurred_at, occurred_at_epoch_ms, stats_date, model, request_model, response_model, reasoning_effort, upstream_account_id, upstream_account_name, status, success_like, terminal, failure_kind, failure_class, is_actionable, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, first_token_ms, t_resp_parse_ms) ON TABLE cvm_analytics.analytics_facts TO cvm_analytics_reader",
                "GRANT SELECT (source, source_row_id, occurred_at_epoch_ms, key_digest, upstream_account_id) ON TABLE cvm_analytics.analytics_parallel_work_keys TO cvm_analytics_reader",
                "GRANT SELECT (id, replica_id, status, owner_kind, owner_epoch, serving_fence, serving_ack_change_id, generation, baseline_generation, baseline_change_id, coverage_generation, coverage_revision, coverage_watermark, consumer_checkpoint, source_acknowledged_through, service_eligible, fact_date_semantics_version, last_error_code, updated_at) ON TABLE cvm_analytics.analytics_replica_state TO cvm_analytics_reader",
                "GRANT SELECT (id, status) ON TABLE cvm_analytics.analytics_state TO cvm_analytics_reader",
                "GRANT SELECT ON TABLE cvm_analytics.analytics_daily_rollups, cvm_analytics.analytics_hourly_rollups, cvm_analytics.analytics_projection_state, cvm_analytics.analytics_projection_checkpoints, cvm_analytics.analytics_projection_target_status TO cvm_analytics_reader",
                "GRANT USAGE ON SCHEMA cvm_analytics TO cvm_analytics_maintenance",
                "GRANT INSERT, UPDATE, DELETE ON TABLE cvm_analytics.analytics_facts, cvm_analytics.analytics_parallel_work_keys TO cvm_analytics_maintenance",
                "GRANT SELECT (source, source_row_id) ON TABLE cvm_analytics.analytics_facts, cvm_analytics.analytics_parallel_work_keys TO cvm_analytics_maintenance",
                "GRANT SELECT, UPDATE ON TABLE cvm_analytics.analytics_replica_state, cvm_analytics.analytics_state TO cvm_analytics_maintenance",
                "GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE cvm_analytics.analytics_daily_rollups, cvm_analytics.analytics_hourly_rollups, cvm_analytics.analytics_projection_state, cvm_analytics.analytics_projection_checkpoints, cvm_analytics.analytics_projection_target_status TO cvm_analytics_maintenance",
                "GRANT INSERT, DELETE ON TABLE cvm_analytics.analytics_replica_applied_changes TO cvm_analytics_maintenance",
                "GRANT SELECT (change_id) ON TABLE cvm_analytics.analytics_replica_applied_changes TO cvm_analytics_maintenance",
                "GRANT USAGE ON SEQUENCE cvm_analytics.analytics_facts_fact_id_seq TO cvm_analytics_maintenance",
            ] {
                sqlx::query(statement)
                    .execute(&mut owner)
                    .await
                    .with_context(|| {
                        format!("grant PostgreSQL fixture maintenance privilege: {statement}")
                    })?;
            }

            let app_pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(5))
                .connect(&app_url)
                .await
                .context("connect PostgreSQL fixture application role")?;
            let maintenance_pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(1)
                .acquire_timeout(Duration::from_secs(5))
                .connect(&maintenance_url)
                .await
                .context("connect PostgreSQL fixture maintenance role")?;
            let body_result = async {
            let app = PgAnalyticsPool {
            pool: app_pool.clone(),
            };
            app.verify_schema().await?;
            app.verify_read_transaction().await?;
            let app_inherits_reader: bool = sqlx::query_scalar(
                "SELECT pg_has_role(current_user, 'cvm_analytics_reader', 'USAGE')",
            )
            .fetch_one(&app_pool)
            .await?;
            anyhow::ensure!(
                app_inherits_reader,
                "application role does not inherit the PostgreSQL analytics reader role"
            );
            let app_runtime = PgAnalyticsRuntime {
                backend: AnalyticsBackend::Postgres,
                ownership: AnalyticsOwnershipV2::Disabled,
                pool: Some(PgAnalyticsPool {
                    pool: app_pool.clone(),
                }),
                maintenance_pool: None,
                unavailable: None,
                ready: Arc::new(AtomicBool::new(true)),
            };

        let app_ddl = sqlx::query("CREATE TABLE cvm_analytics.app_must_not_ddl (id INTEGER)")
            .execute(&app_pool)
            .await;
        anyhow::ensure!(
            app_ddl.is_err(),
            "application role unexpectedly retained schema DDL privilege"
        );

        let mut maintenance = maintenance_pool.acquire().await?;
        let maintenance_ddl =
            sqlx::query("CREATE TABLE cvm_analytics.maintenance_must_not_ddl (id INTEGER)")
                .execute(&mut *maintenance)
                .await;
        anyhow::ensure!(
            maintenance_ddl.is_err(),
            "maintenance role unexpectedly retained schema DDL privilege"
        );
        let maintenance_rollup_dml = sqlx::query(
            "INSERT INTO cvm_analytics.analytics_daily_rollups (stats_date, dimension, series_key, display_name) VALUES ('1970-01-01', 'overall', 'fixture', 'Fixture')",
        )
        .execute(&mut *maintenance)
        .await;
        anyhow::ensure!(
            maintenance_rollup_dml.is_ok(),
            "maintenance role could not write the owned long-term rollup target"
        );
        sqlx::query(
            "DELETE FROM cvm_analytics.analytics_daily_rollups WHERE stats_date = '1970-01-01' AND dimension = 'overall' AND series_key = 'fixture'",
        )
        .execute(&mut *maintenance)
        .await?;
        let maintenance_fact_read =
            sqlx::query("SELECT invoke_id FROM cvm_analytics.analytics_facts LIMIT 1")
                .execute(&mut *maintenance)
                .await;
        anyhow::ensure!(
            maintenance_fact_read.is_err(),
            "maintenance role unexpectedly retained analytics fact-content read privilege"
        );
        let maintenance_identity_read = sqlx::query(
            "SELECT source, source_row_id FROM cvm_analytics.analytics_facts LIMIT 1",
        )
        .fetch_optional(&mut *maintenance)
        .await;
        anyhow::ensure!(
            maintenance_identity_read.is_ok(),
            "maintenance role could not read the identity columns required by replica DML"
        );
        let replica_state_insert: bool = sqlx::query_scalar(
            "SELECT has_table_privilege(current_user, 'cvm_analytics.analytics_replica_state', 'INSERT')",
        )
        .fetch_one(&mut *maintenance)
        .await?;
        let replica_state_delete: bool = sqlx::query_scalar(
            "SELECT has_table_privilege(current_user, 'cvm_analytics.analytics_replica_state', 'DELETE')",
        )
        .fetch_one(&mut *maintenance)
        .await?;
        let analytics_state_insert: bool = sqlx::query_scalar(
            "SELECT has_table_privilege(current_user, 'cvm_analytics.analytics_state', 'INSERT')",
        )
        .fetch_one(&mut *maintenance)
        .await?;
        let analytics_state_delete: bool = sqlx::query_scalar(
            "SELECT has_table_privilege(current_user, 'cvm_analytics.analytics_state', 'DELETE')",
        )
        .fetch_one(&mut *maintenance)
        .await?;
        anyhow::ensure!(
            !replica_state_insert && !replica_state_delete,
            "maintenance role retained unnecessary replica-state INSERT/DELETE privileges"
        );
        anyhow::ensure!(
            !analytics_state_insert && !analytics_state_delete,
            "maintenance role retained unnecessary analytics-state INSERT/DELETE privileges"
        );
        let fact = snapshot_row_to_fact(SourceInvocationSnapshotRow {
            id: 1,
            invoke_id: "fixture-1".to_string(),
            occurred_at: "1970-01-01T00:00:00Z".to_string(),
            occurred_at_epoch_ms: 0,
            stats_date: "1970-01-01".to_string(),
            source: "fixture".to_string(),
            ..Default::default()
        })?;
        upsert_analytics_fact(&mut maintenance, &fact).await?;
        let mut corrected_fact = fact.clone();
        corrected_fact.invoke_id = "fixture-1-corrected".to_string();
        upsert_analytics_fact(&mut maintenance, &corrected_fact).await?;

        let key = AnalyticsParallelWorkKey {
            source: fact.source.clone(),
            source_row_id: fact.source_row_id,
            occurred_at_epoch_ms: fact.occurred_at_epoch_ms,
            key_digest: "sha256:fixture".to_string(),
            upstream_account_id: None,
        };
        replace_parallel_work_key(&mut maintenance, &key.source, key.source_row_id, Some(&key))
            .await?;
        let mut corrected_key = key.clone();
        corrected_key.key_digest = "sha256:fixture-corrected".to_string();
        replace_parallel_work_key(
            &mut maintenance,
            &corrected_key.source,
            corrected_key.source_row_id,
            Some(&corrected_key),
        )
        .await?;
        delete_analytics_fact(&mut maintenance, &corrected_fact.source, corrected_fact.source_row_id)
            .await?;
        replace_parallel_work_key(
            &mut maintenance,
            &corrected_key.source,
            corrected_key.source_row_id,
            None,
        )
        .await?;
        upsert_analytics_fact(&mut maintenance, &corrected_fact).await?;
        replace_parallel_work_key(
            &mut maintenance,
            &corrected_key.source,
            corrected_key.source_row_id,
            Some(&corrected_key),
        )
        .await?;

        let change = SourceChangeRow {
            change_id: 1,
            event_version: crate::analytics_changes::ANALYTICS_SOURCE_EVENT_VERSION,
            fact_date_semantics_version: ANALYTICS_FACT_DATE_SEMANTICS_VERSION,
            event_kind: "upsert".to_string(),
            entity_kind: "invocation".to_string(),
            source: "fixture".to_string(),
            source_row_id: Some(1),
            entity_key: "fixture-1".to_string(),
            generation: 1,
            coverage_revision: 1,
            fact_json: "{}".to_string(),
        };
        record_applied_change(&mut maintenance, &change).await?;
        record_applied_change(&mut maintenance, &change).await?;
        sqlx::query(
            r#"
            UPDATE cvm_analytics.analytics_replica_state
            SET status = 'caught_up', owner_kind = 'postgres', owner_epoch = 2,
                serving_fence = 2, serving_ack_change_id = 1,
                fact_date_semantics_version = 2,
                generation = 1, baseline_generation = 1, baseline_change_id = 1,
                coverage_generation = 1, coverage_revision = 1, coverage_watermark = 1,
                consumer_checkpoint = 1, source_acknowledged_through = 1,
                service_eligible = TRUE
            WHERE id = 1
            "#,
        )
        .execute(&mut *maintenance)
        .await?;
        sqlx::query("UPDATE cvm_analytics.analytics_state SET status = 'ready' WHERE id = 1")
            .execute(&mut *maintenance)
            .await?;
        let read_start = Utc
            .timestamp_millis_opt(-1)
            .single()
            .context("build PostgreSQL fixture read start")?;
        let read_end = Utc
            .timestamp_millis_opt(1)
            .single()
            .context("build PostgreSQL fixture read end")?;
        let mut production_read = app_runtime.require_read_transaction().await?;
        let production_key_rows = query_parallel_work_key_rows_on_connection(
            &mut production_read,
            read_start,
            read_end,
            false,
            None,
        )
        .await?;
        anyhow::ensure!(
            production_key_rows.len() == 1,
            "production PostgreSQL key query did not read the fixture row"
        );
        let production_conversation_rows = query_parallel_work_conversation_rows_on_connection(
            &mut production_read,
            read_start,
            read_end,
            false,
            None,
        )
        .await?;
        anyhow::ensure!(
            production_conversation_rows.len() == 1,
            "production PostgreSQL conversation query did not read the fixture row"
        );
        production_read.commit().await?;
        let mut in_flight_reader = app_runtime.require_read_transaction().await?;
        fence_replica_service_eligibility_on_connection(&mut maintenance).await?;
        clear_replica_target(&mut maintenance, 2).await?;
        let snapshot_fact_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cvm_analytics.analytics_facts",
        )
        .fetch_one(&mut *in_flight_reader)
        .await?;
        anyhow::ensure!(
            snapshot_fact_count == 1,
            "repeatable-read analytics reader did not retain the pre-fence snapshot"
        );
        in_flight_reader.rollback().await?;
        let post_fence_reader = app_runtime.require_read_transaction().await;
        anyhow::ensure!(
            post_fence_reader.is_err(),
            "PostgreSQL reader admitted a new transaction after the rebuild fence"
        );
        sqlx::query(
            r#"
            UPDATE cvm_analytics.analytics_replica_state
            SET status = 'caught_up', fact_date_semantics_version = 1,
                generation = 1, baseline_generation = 1, baseline_change_id = 1,
                coverage_generation = 1, coverage_revision = 1, coverage_watermark = 1,
                consumer_checkpoint = 1, source_acknowledged_through = 1,
                service_eligible = TRUE
            WHERE id = 1
            "#,
        )
        .execute(&mut *maintenance)
        .await?;

        sqlx::query(
            "ALTER TABLE cvm_analytics.analytics_replica_state DROP COLUMN fact_date_semantics_version",
        )
        .execute(&mut owner)
        .await?;
        apply_fixture_schema(&mut owner, "reapply PostgreSQL fixture schema").await?;
        let owner_fenced: bool = sqlx::query_scalar(
            "SELECT service_eligible FROM cvm_analytics.analytics_replica_state WHERE id = 1",
        )
        .fetch_one(&mut owner)
        .await?;
        anyhow::ensure!(
            !owner_fenced,
            "owner-applied compatibility contract did not fence legacy eligibility"
        );
        let owner_status: String = sqlx::query_scalar(
            "SELECT status FROM cvm_analytics.analytics_replica_state WHERE id = 1",
        )
        .fetch_one(&mut owner)
        .await?;
        anyhow::ensure!(
            owner_status == "bootstrapping",
            "owner-applied compatibility contract did not fence current eligibility"
        );
        sqlx::query(
            "UPDATE cvm_analytics.analytics_replica_state SET status = 'caught_up', service_eligible = TRUE WHERE id = 1",
        )
        .execute(&mut *maintenance)
        .await?;
        fence_replica_service_eligibility_on_connection(&mut maintenance).await?;
        let bootstrap_fenced: bool = sqlx::query_scalar(
            "SELECT service_eligible FROM cvm_analytics.analytics_replica_state WHERE id = 1",
        )
        .fetch_one(&mut *maintenance)
        .await?;
        anyhow::ensure!(
            !bootstrap_fenced,
            "bootstrap preflight did not persist the rollback fence"
        );
        let bootstrap_status: String = sqlx::query_scalar(
            "SELECT status FROM cvm_analytics.analytics_replica_state WHERE id = 1",
        )
        .fetch_one(&mut *maintenance)
        .await?;
        anyhow::ensure!(
            bootstrap_status == "bootstrapping",
            "bootstrap preflight did not fence current eligibility"
        );

        let app_dml = sqlx::query("DELETE FROM cvm_analytics.analytics_facts")
            .execute(&app_pool)
            .await;
        anyhow::ensure!(
            app_dml.is_err(),
            "application role unexpectedly retained analytics DML privilege"
        );

        clear_replica_target(&mut maintenance, 7).await?;
        drop(maintenance);
        let fact_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cvm_analytics.analytics_facts")
                .fetch_one(&mut owner)
                .await?;
        let applied_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM cvm_analytics.analytics_replica_applied_changes",
        )
        .fetch_one(&mut owner)
        .await?;
        let parallel_work_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM cvm_analytics.analytics_parallel_work_keys")
                .fetch_one(&mut owner)
                .await?;
        let state = load_replica_state(&maintenance_pool).await?;
        anyhow::ensure!(fact_count == 0, "bounded reset retained analytics facts");
        anyhow::ensure!(applied_count == 0, "bounded reset retained applied changes");
        anyhow::ensure!(
            parallel_work_count == 0,
            "bounded reset retained parallel-work keys"
        );
        anyhow::ensure!(
            state.status == "bootstrapping",
            "bounded reset did not gate state"
        );
        anyhow::ensure!(
            state.fact_date_semantics_version == ANALYTICS_FACT_DATE_SEMANTICS_VERSION,
            "bounded reset did not promote current fact-date semantics"
        );
        anyhow::ensure!(
            state.consumer_checkpoint == 0,
            "bounded reset retained checkpoint"
        );
        anyhow::ensure!(
            !state.service_eligible,
            "bounded reset left service eligibility set"
        );

        let mut maintenance = maintenance_pool.acquire().await?;
        let first_locked: bool = sqlx::query_scalar(PG_ANALYTICS_BOOTSTRAP_LOCK_ACQUIRE_SQL)
            .bind(ANALYTICS_REPLICA_BOOTSTRAP_LOCK_KEY)
            .fetch_one(&mut *maintenance)
            .await?;
        anyhow::ensure!(
            first_locked,
            "fixture could not acquire bootstrap advisory lock"
        );
        let second_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .connect(&maintenance_url)
            .await?;
        let second_locked: bool = sqlx::query_scalar(PG_ANALYTICS_BOOTSTRAP_LOCK_ACQUIRE_SQL)
            .bind(ANALYTICS_REPLICA_BOOTSTRAP_LOCK_KEY)
            .fetch_one(&second_pool)
            .await?;
        anyhow::ensure!(!second_locked, "bootstrap advisory lock was not exclusive");
        let released: bool = sqlx::query_scalar(PG_ANALYTICS_BOOTSTRAP_LOCK_RELEASE_SQL)
            .bind(ANALYTICS_REPLICA_BOOTSTRAP_LOCK_KEY)
            .fetch_one(&mut *maintenance)
            .await?;
        anyhow::ensure!(
            released,
            "fixture could not release bootstrap advisory lock"
        );
        drop(maintenance);
        second_pool.close().await;

        sqlx::query(
            "ALTER TABLE cvm_analytics.analytics_replica_state DROP COLUMN fact_date_semantics_version",
        )
        .execute(&mut owner)
        .await?;
        let missing_column = app.verify_schema().await;
        anyhow::ensure!(
            missing_column.is_err(),
            "application schema verification accepted a missing compatibility column"
        );
        sqlx::query(
            "ALTER TABLE cvm_analytics.analytics_replica_state ADD COLUMN fact_date_semantics_version BIGINT NOT NULL DEFAULT 1",
        )
        .execute(&mut owner)
        .await?;
        app.verify_schema().await?;

            Ok::<_, anyhow::Error>(())
        }
        .await;
            app_pool.close().await;
            maintenance_pool.close().await;
            body_result
        }
        .await;
        let cleanup_result = sqlx::query("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
            .execute(&mut owner)
            .await
            .context("clean up PostgreSQL fixture schema");
        let owner_close_result = owner.close().await;
        fixture_result?;
        cleanup_result?;
        owner_close_result?;
        Ok(())
    }

    #[test]
    fn snapshot_payload_uses_shared_json_duplicate_and_numeric_normalization() {
        let fact = snapshot_row_to_fact(SourceInvocationSnapshotRow {
            id: 43,
            invoke_id: "invoke-43".to_string(),
            occurred_at: "2026-10-31T00:00:00Z".to_string(),
            stats_date: "2026-10-31".to_string(),
            source: "proxy".to_string(),
            payload: Some(
                r#"{"requestModel":"first","requestModel":"second","upstreamAccountId":"   42   "}"#
                    .to_string(),
            ),
            ..Default::default()
        })
        .expect("snapshot payload should normalize through the shared JSON path");
        assert_eq!(fact.request_model.as_deref(), Some("second"));
        assert_eq!(fact.upstream_account_id, Some(42));
    }
}
