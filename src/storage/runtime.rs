//! Runtime wiring for module-scoped storage authority and online migration workers.
//!
//! The router is the lifecycle owner for shared pools and generation gates. Production writers
//! receive stores constructed from this router; test-only SQLite constructors remain available for
//! isolated fixtures.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use sqlx::{Pool, Row, Sqlite};
use tokio::{
    sync::{Mutex, Notify},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use super::{
    analytics::AnalyticsProjectionStore,
    catalog::{ModuleCatalog, ModuleId},
    config::StorageConfig,
    core::{
        AuthorityManifestStore, AuthorityPublication, AuthorityRecoveryContext,
        AuthorityRecoveryRecord, BackendKind, MigrationCheckpoint, MigrationMetadataStore,
        MigrationOutboxItem, MigrationReservation, MigrationStatus, ModuleAuthorityManifest,
        ModuleGenerationGate, StorageModule, StoragePools,
    },
    migration::{
        ChangeBatchLimits, CopyLimits, MigrationEngine, MigrationLimits, MigrationSource,
        SourceWriteAuthority, SourceWriteFence,
    },
    migration_sqlx::{PostgresMigrationTarget, SqliteMigrationSource},
    migration_status::{
        StorageMigrationProgress, StorageMigrationResponse, StorageMigrationRuntime,
        StorageMigrationState,
    },
    traffic::TrafficFactStore,
};

const MIGRATION_ID: &str = "storage-migration";
const INSTALLATION_ID: &str = "local";
const SOURCE_IDENTITY: &str = "sqlite:local";
const AUTHORITY_TABLE: &str = "storage_migration_authority";
const CHECKPOINT_TABLE: &str = "storage_migration_engine_checkpoints";
const OUTBOX_TABLE: &str = "storage_migration_engine_outbox";
const RESERVATION_TABLE: &str = "storage_migration_engine_reservations";
const STATUS_TABLE: &str = "storage_migration_engine_status";
const CAPTURE_STATE_TABLE: &str = "migration_change_log_state";
const WORKER_POLL_INTERVAL: Duration = Duration::from_secs(1);
const WORKER_RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(crate) struct StorageRuntimeRouter {
    pools: StoragePools,
    traffic_authority: Arc<TrafficFactStore>,
    analytics_authority: Arc<AnalyticsProjectionStore>,
    traffic_gate: ModuleGenerationGate,
    analytics_gate: ModuleGenerationGate,
    worker: Arc<StorageMigrationWorker>,
}

impl StorageRuntimeRouter {
    pub(crate) async fn initialize(
        pool: Pool<Sqlite>,
        config: &StorageConfig,
        migration: Arc<StorageMigrationRuntime>,
        shutdown: CancellationToken,
    ) -> Result<Arc<Self>> {
        let pools = StoragePools::from_sqlite_and_config_lazy(pool.clone(), config)?;
        let traffic_gate = ModuleGenerationGate::new(
            StorageModule::TrafficArchive,
            super::core::ModuleAuthority {
                backend: BackendKind::Sqlite,
                generation: 1,
            },
        );
        let analytics_gate = ModuleGenerationGate::new(
            StorageModule::Analytics,
            super::core::ModuleAuthority {
                backend: BackendKind::Sqlite,
                generation: 1,
            },
        );
        let source_fence = SourceWriteFence::new(1);
        let metadata = Arc::new(SqliteMigrationMetadataStore::new(
            pool.clone(),
            config
                .postgres_database_url
                .as_deref()
                .is_some_and(|url| !url.trim().is_empty()),
        ));
        metadata.ensure_schema().await?;

        let traffic_authority = Arc::new(TrafficFactStore::with_authority_and_fence(
            pools.clone(),
            traffic_gate.clone(),
            source_fence.clone(),
        ));
        let analytics_authority = Arc::new(AnalyticsProjectionStore::with_authority(
            pools.clone(),
            analytics_gate.clone(),
            traffic_gate.clone(),
        ));
        let worker = Arc::new(StorageMigrationWorker::new(
            pool,
            config.clone(),
            migration,
            shutdown,
            metadata,
            traffic_gate.clone(),
            source_fence,
        ));
        let router = Arc::new(Self {
            pools,
            traffic_authority,
            analytics_authority,
            traffic_gate,
            analytics_gate,
            worker,
        });
        router.recover_authority().await?;
        Ok(router)
    }

    async fn recover_authority(&self) -> Result<()> {
        self.worker.migration.validate_runtime_identity().await?;
        let status = self.worker.migration.snapshot().await;
        let record = self
            .worker
            .metadata
            .load_authority_manifest(StorageModule::TrafficArchive)
            .await?;
        match record {
            AuthorityRecoveryRecord::Manifest(manifest) => {
                validate_manifest_identity(
                    &manifest,
                    &status,
                    expected_postgres_target_identity(&self.worker.config),
                )?;
                if manifest.authority.backend != BackendKind::Postgres {
                    bail!("durable traffic authority manifest is not PostgreSQL");
                }
                self.pools
                    .require_postgres_pool()
                    .context("recover durable PostgreSQL traffic authority")?;
                if !matches!(
                    status.state,
                    StorageMigrationState::Postgres
                        | StorageMigrationState::Switching
                        | StorageMigrationState::Recovering
                ) {
                    bail!(
                        "durable PostgreSQL authority manifest conflicts with migration state {:?}",
                        status.state
                    );
                }
                if status.authority.backend
                    == super::migration_status::StorageMigrationBackend::Postgres
                    && status.authority.generation > manifest.authority.generation
                {
                    bail!(
                        "durable migration status generation {} is ahead of PostgreSQL authority manifest generation {}",
                        status.authority.generation,
                        manifest.authority.generation
                    );
                }
                if status.state != StorageMigrationState::Postgres
                    && manifest.authority.generation <= status.authority.generation
                {
                    bail!(
                        "PostgreSQL authority manifest does not advance migration generation {}",
                        status.authority.generation
                    );
                }
                self.ensure_native_source_fenced(manifest.authority.generation)
                    .await?;
                let expected = recovery_context_from_record(AuthorityRecoveryRecord::Manifest(
                    manifest.clone(),
                ))?;
                self.traffic_gate
                    .recover_authority(self.worker.metadata.as_ref(), &expected)
                    .await
                    .context("recover traffic module authority")?;
                self.worker
                    .migration
                    .recover_postgres(manifest.authority.generation, manifest.source_watermark)
                    .await?;
            }
            AuthorityRecoveryRecord::NeverMigrated { .. }
                if status.authority.backend
                    == super::migration_status::StorageMigrationBackend::Postgres =>
            {
                bail!("PostgreSQL authority has no durable traffic manifest");
            }
            AuthorityRecoveryRecord::NeverMigrated { .. } => {
                let source_authority =
                    load_durable_source_write_authority(&self.worker.pool).await?;
                let gate_authority = self.traffic_gate.current_authority().await;
                if status.authority.backend
                    == super::migration_status::StorageMigrationBackend::Sqlite
                    && status.authority.generation != gate_authority.generation
                {
                    bail!(
                        "durable migration status generation {} does not match the selected SQLite generation {}",
                        status.authority.generation,
                        gate_authority.generation
                    );
                }
                if source_authority
                    .is_some_and(|authority| authority.generation != gate_authority.generation)
                {
                    bail!(
                        "SQLite source authority generation does not match the selected SQLite generation"
                    );
                }
                if source_authority.is_some_and(|authority| authority.fenced) {
                    bail!(
                        "SQLite source writes are durably fenced without a PostgreSQL authority manifest"
                    );
                }
            }
            AuthorityRecoveryRecord::Indeterminate => {
                bail!("traffic authority recovery is indeterminate");
            }
        }
        Ok(())
    }

    async fn ensure_native_source_fenced(&self, expected_generation: u64) -> Result<()> {
        let authority = load_durable_source_write_authority(&self.worker.pool)
            .await?
            .ok_or_else(|| anyhow!("SQLite source authority state is missing"))?;
        if !authority.fenced || authority.generation != expected_generation {
            bail!(
                "PostgreSQL authority manifest requires SQLite source fence generation {}, found generation {} fenced={}",
                expected_generation,
                authority.generation,
                authority.fenced
            );
        }
        Ok(())
    }

    pub(crate) fn traffic_authority(&self) -> Arc<TrafficFactStore> {
        Arc::clone(&self.traffic_authority)
    }

    pub(crate) fn analytics_authority(&self) -> Arc<AnalyticsProjectionStore> {
        Arc::clone(&self.analytics_authority)
    }

    pub(crate) fn worker(&self) -> &StorageMigrationWorker {
        self.worker.as_ref()
    }

    pub(crate) async fn start(&self) {
        self.worker.start().await;
    }

    pub(crate) async fn stop(&self) {
        self.worker.stop().await;
    }

    pub(crate) fn wake(&self) {
        self.worker.notify.notify_one();
    }

    pub(crate) fn postgres_configured(&self) -> bool {
        self.pools.postgres_pool().is_some()
    }

    #[cfg(test)]
    pub(crate) async fn current_authority(
        &self,
        module: StorageModule,
    ) -> super::core::ModuleAuthority {
        match module {
            StorageModule::TrafficArchive => self.traffic_gate.current_authority().await,
            StorageModule::Analytics => self.analytics_gate.current_authority().await,
            _ => super::core::ModuleAuthority {
                backend: BackendKind::Sqlite,
                generation: 1,
            },
        }
    }
}

async fn load_durable_source_write_authority(
    pool: &Pool<Sqlite>,
) -> Result<Option<SourceWriteAuthority>> {
    let exists = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '{CAPTURE_STATE_TABLE}')"
    ))
    .fetch_one(pool)
    .await?;
    if exists == 0 {
        return Ok(None);
    }
    let row = sqlx::query(&format!(
        "SELECT authority_generation, source_writes_fenced
           FROM {CAPTURE_STATE_TABLE}
          WHERE migration_id = ? AND source_identity = ?"
    ))
    .bind(MIGRATION_ID)
    .bind(SOURCE_IDENTITY)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("SQLite source authority state row is missing"))?;
    let authority = SourceWriteAuthority {
        generation: u64::try_from(row.try_get::<i64, _>("authority_generation")?)
            .context("SQLite source authority generation is invalid")?,
        fenced: row.try_get::<i64, _>("source_writes_fenced")? != 0,
    };
    authority.validate()?;
    Ok(Some(authority))
}

#[derive(Debug)]
pub(crate) struct StorageMigrationWorker {
    pool: Pool<Sqlite>,
    config: StorageConfig,
    pub(crate) migration: Arc<StorageMigrationRuntime>,
    shutdown: CancellationToken,
    pub(crate) metadata: Arc<SqliteMigrationMetadataStore>,
    traffic_gate: ModuleGenerationGate,
    source_fence: SourceWriteFence,
    started: AtomicBool,
    notify: Arc<Notify>,
    handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl StorageMigrationWorker {
    fn new(
        pool: Pool<Sqlite>,
        config: StorageConfig,
        migration: Arc<StorageMigrationRuntime>,
        shutdown: CancellationToken,
        metadata: Arc<SqliteMigrationMetadataStore>,
        traffic_gate: ModuleGenerationGate,
        source_fence: SourceWriteFence,
    ) -> Self {
        Self {
            pool,
            config,
            migration,
            shutdown,
            metadata,
            traffic_gate,
            source_fence,
            started: AtomicBool::new(false),
            notify: Arc::new(Notify::new()),
            handle: Arc::new(Mutex::new(None)),
        }
    }

    async fn start(&self) {
        if self
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.notify.notify_one();
            return;
        }
        let worker = self.clone_for_task();
        let handle = tokio::spawn(async move {
            worker.run().await;
        });
        *self.handle.lock().await = Some(handle);
    }

    async fn stop(&self) {
        self.shutdown.cancel();
        self.notify.notify_waiters();
        if let Some(handle) = self.handle.lock().await.take() {
            let _ = handle.await;
        }
    }

    fn ensure_worker_running(&self) -> Result<()> {
        if self.shutdown.is_cancelled() {
            bail!("storage migration worker is shutting down");
        }
        Ok(())
    }

    fn clone_for_task(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            config: self.config.clone(),
            migration: Arc::clone(&self.migration),
            shutdown: self.shutdown.clone(),
            metadata: Arc::clone(&self.metadata),
            traffic_gate: self.traffic_gate.clone(),
            source_fence: self.source_fence.clone(),
            started: AtomicBool::new(true),
            notify: Arc::clone(&self.notify),
            handle: Arc::clone(&self.handle),
        }
    }

    async fn run(self) {
        loop {
            if self.shutdown.is_cancelled() {
                return;
            }
            if self.traffic_gate.requires_recovery() {
                return;
            }
            let status = self.migration.snapshot().await;
            let should_run = matches!(
                status.state,
                StorageMigrationState::Preparing
                    | StorageMigrationState::Copying
                    | StorageMigrationState::CatchingUp
                    | StorageMigrationState::Verifying
                    | StorageMigrationState::Ready
                    | StorageMigrationState::Switching
                    | StorageMigrationState::Recovering
            );
            if should_run && let Err(error) = self.run_migration(status).await {
                tracing::warn!(error = %error, "online storage migration worker iteration failed");
                let _ = self.migration.publish_worker_error(error.to_string()).await;
                tokio::select! {
                    _ = self.shutdown.cancelled() => return,
                    _ = tokio::time::sleep(WORKER_RETRY_INTERVAL) => {},
                }
                continue;
            }
            tokio::select! {
                _ = self.shutdown.cancelled() => return,
                _ = self.notify.notified() => {},
                _ = tokio::time::sleep(WORKER_POLL_INTERVAL) => {},
            }
        }
    }

    async fn run_migration(&self, status: StorageMigrationResponse) -> Result<()> {
        self.migration.validate_runtime_identity().await?;
        let authority = self.traffic_gate.current_authority().await;
        if authority.backend == BackendKind::Postgres {
            let manifest = self.load_traffic_authority_manifest().await?;
            validate_manifest_identity(
                &manifest,
                &status,
                expected_postgres_target_identity(&self.config),
            )?;
            if manifest.authority != authority {
                bail!(
                    "PostgreSQL authority does not match the durable traffic manifest: gate={authority:?} manifest={:?}",
                    manifest.authority
                );
            }
            self.migration
                .recover_postgres(authority.generation, manifest.source_watermark)
                .await?;
            return Ok(());
        }
        if let Some(source_authority) = load_durable_source_write_authority(&self.pool).await?
            && source_authority.generation != authority.generation
        {
            self.traffic_gate.require_recovery();
            bail!(
                "SQLite source authority generation {} does not match the selected SQLite generation {}",
                source_authority.generation,
                authority.generation
            );
        }
        let Some(postgres) = self.config.connect_postgres_lazy()? else {
            bail!("PostgreSQL storage is not configured");
        };
        let catalog = ModuleCatalog::inspect_sqlite(&self.pool).await?;
        let native_catalog = catalog
            .native_migration_catalog([ModuleId::TrafficArchive, ModuleId::Analytics])
            .map_err(|error| anyhow!(error))?;
        let source = Arc::new(
            SqliteMigrationSource::from_native_catalog(
                self.pool.clone(),
                MIGRATION_ID,
                SOURCE_IDENTITY,
                "migration_change_log",
                &native_catalog,
                StorageModule::TrafficArchive,
            )?
            .with_writer_generation(status.authority.generation)?,
        );
        source.ensure_capture_schema().await?;
        source.ensure_native_capture_triggers().await?;
        let target = Arc::new(PostgresMigrationTarget::new_native(
            postgres,
            "storage_migration",
            native_catalog,
            StorageModule::TrafficArchive,
        )?);
        target.ensure_schema().await?;
        let engine = MigrationEngine::new(
            source.clone(),
            target.clone(),
            Arc::clone(&self.metadata),
            StorageModule::TrafficArchive,
            INSTALLATION_ID,
            status.target.identity.clone(),
            status.authority.generation,
            MigrationLimits {
                copy: CopyLimits {
                    max_rows: 128,
                    max_bytes: 2 * 1024 * 1024,
                    ..CopyLimits::default()
                },
                change_batch: ChangeBatchLimits::default(),
                ..MigrationLimits::default()
            },
            self.source_fence.clone(),
        )?
        .with_cancellation_token(self.shutdown.clone());
        let tables = source.module_tables(StorageModule::TrafficArchive).await?;
        // Plans and checkpoints are durable engine state. Re-loading them is safe for every
        // resumable phase and does not change the persisted migration phase.
        engine.prepare(&tables).await?;
        let resume_work = migration_resume_work(status.state)?;
        if matches!(status.state, StorageMigrationState::Preparing) {
            self.migration
                .publish_progress(StorageMigrationProgress {
                    phase: StorageMigrationState::Copying,
                    ..StorageMigrationProgress::default()
                })
                .await?;
        }
        if matches!(resume_work, MigrationResumeWork::CopyAndCatchUp) {
            for table in &tables {
                loop {
                    self.ensure_worker_running()?;
                    let current = self.migration.snapshot().await;
                    if matches!(current.state, StorageMigrationState::Paused) {
                        return Ok(());
                    }
                    let progress = engine.copy_next(table).await?;
                    self.migration
                        .publish_progress(StorageMigrationProgress {
                            phase: StorageMigrationState::Copying,
                            committed_rows: Some(progress.copied_rows),
                            committed_bytes: Some(progress.copied_bytes),
                            scan_cursor: progress.cursor,
                            captured_through: Some(progress.applied_through),
                            applied_through: Some(progress.applied_through),
                            pending_keys: Some(progress.pending_changes as u64),
                            ..StorageMigrationProgress::default()
                        })
                        .await?;
                    if progress.complete {
                        break;
                    }
                }
            }
        }
        let watermark = if matches!(
            resume_work,
            MigrationResumeWork::CopyAndCatchUp | MigrationResumeWork::CatchUp
        ) {
            self.migration
                .publish_progress(StorageMigrationProgress {
                    phase: StorageMigrationState::CatchingUp,
                    ..StorageMigrationProgress::default()
                })
                .await?;
            let watermark = source.capture_watermark().await?;
            for table in &tables {
                engine.catch_up(table).await?;
            }
            engine.release_capture_through(watermark).await?;
            self.migration
                .publish_progress(StorageMigrationProgress {
                    phase: StorageMigrationState::Verifying,
                    captured_through: Some(watermark),
                    applied_through: Some(watermark),
                    ..StorageMigrationProgress::default()
                })
                .await?;
            for table in &tables {
                engine.verify_table_coverage_at(table, watermark).await?;
            }
            self.migration
                .publish_progress(StorageMigrationProgress {
                    phase: StorageMigrationState::Ready,
                    captured_through: Some(watermark),
                    applied_through: Some(watermark),
                    converged: Some(true),
                    ..StorageMigrationProgress::default()
                })
                .await?;
            watermark
        } else {
            source.capture_watermark().await?
        };
        let expected = AuthorityRecoveryContext {
            migration_id: MIGRATION_ID.to_string(),
            installation_id: INSTALLATION_ID.to_string(),
            module: StorageModule::TrafficArchive,
            source_identity: SOURCE_IDENTITY.to_string(),
            target_identity: status.target.identity,
            source_watermark: watermark,
        };
        self.migration
            .publish_progress(StorageMigrationProgress {
                phase: StorageMigrationState::Switching,
                captured_through: Some(watermark),
                applied_through: Some(watermark),
                ..StorageMigrationProgress::default()
            })
            .await?;
        engine
            .switch_authority(
                &self.traffic_gate,
                expected,
                self.config.migration_switch_budget,
            )
            .await?;
        let manifest = self.load_traffic_authority_manifest().await?;
        let authority = self.traffic_gate.current_authority().await;
        let status = self.migration.snapshot().await;
        validate_manifest_identity(
            &manifest,
            &status,
            expected_postgres_target_identity(&self.config),
        )?;
        if manifest.authority != authority {
            bail!(
                "PostgreSQL authority does not match the durable traffic manifest: gate={authority:?} manifest={:?}",
                manifest.authority
            );
        }
        self.migration
            .publish_postgres_authority(authority.generation, manifest.source_watermark)
            .await?;
        Ok(())
    }

    async fn load_traffic_authority_manifest(&self) -> Result<ModuleAuthorityManifest> {
        match self
            .metadata
            .load_authority_manifest(StorageModule::TrafficArchive)
            .await?
        {
            AuthorityRecoveryRecord::Manifest(manifest) => Ok(manifest),
            AuthorityRecoveryRecord::NeverMigrated { .. } => {
                bail!("PostgreSQL authority has no durable traffic manifest")
            }
            AuthorityRecoveryRecord::Indeterminate => {
                bail!("traffic authority recovery is indeterminate")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MigrationResumeWork {
    CopyAndCatchUp,
    CatchUp,
    Switch,
}

fn migration_resume_work(state: StorageMigrationState) -> Result<MigrationResumeWork> {
    match state {
        StorageMigrationState::Preparing | StorageMigrationState::Copying => {
            Ok(MigrationResumeWork::CopyAndCatchUp)
        }
        StorageMigrationState::CatchingUp
        | StorageMigrationState::Verifying
        | StorageMigrationState::Recovering => Ok(MigrationResumeWork::CatchUp),
        StorageMigrationState::Ready | StorageMigrationState::Switching => {
            Ok(MigrationResumeWork::Switch)
        }
        state => bail!("storage migration cannot resume from {state:?}"),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SqliteMigrationMetadataStore {
    pool: Pool<Sqlite>,
    target_configured: bool,
}

impl SqliteMigrationMetadataStore {
    fn new(pool: Pool<Sqlite>, target_configured: bool) -> Self {
        Self {
            pool,
            target_configured,
        }
    }

    async fn ensure_schema(&self) -> Result<()> {
        for query in [
            format!(
                "CREATE TABLE IF NOT EXISTS {AUTHORITY_TABLE} (
                    module TEXT PRIMARY KEY,
                    recovery_json TEXT,
                    manifest_json TEXT
                )"
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {CHECKPOINT_TABLE} (
                    migration_id TEXT NOT NULL,
                    table_name TEXT NOT NULL,
                    value_json TEXT NOT NULL,
                    PRIMARY KEY (migration_id, table_name)
                )"
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {OUTBOX_TABLE} (
                    outbox_id TEXT PRIMARY KEY,
                    value_json TEXT NOT NULL,
                    applied_through INTEGER
                )"
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {RESERVATION_TABLE} (
                    reservation_id TEXT PRIMARY KEY,
                    value_json TEXT NOT NULL
                )"
            ),
            format!(
                "CREATE TABLE IF NOT EXISTS {STATUS_TABLE} (
                    module TEXT PRIMARY KEY,
                    value_json TEXT NOT NULL
                )"
            ),
        ] {
            sqlx::query(&query)
                .execute(&self.pool)
                .await
                .context("ensure SQLite migration engine metadata")?;
        }
        Ok(())
    }

    async fn load_json<T: serde::de::DeserializeOwned>(
        &self,
        table: &str,
        key_column: &str,
        key: &str,
        value_column: &str,
    ) -> Result<Option<T>> {
        let query = format!("SELECT {value_column} FROM {table} WHERE {key_column} = ?1");
        let value = sqlx::query_scalar::<_, String>(&query)
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        value
            .map(|value| serde_json::from_str(&value).context("decode SQLite migration metadata"))
            .transpose()
    }
}

#[async_trait]
impl AuthorityManifestStore for SqliteMigrationMetadataStore {
    async fn load_authority_manifest(
        &self,
        module: StorageModule,
    ) -> Result<AuthorityRecoveryRecord> {
        let module_name = serde_json::to_string(&module)?;
        let row = sqlx::query(&format!(
            "SELECT recovery_json, manifest_json FROM {AUTHORITY_TABLE} WHERE module = ?1"
        ))
        .bind(module_name.trim_matches('"'))
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(AuthorityRecoveryRecord::NeverMigrated {
                migration_id: MIGRATION_ID.to_string(),
                installation_id: INSTALLATION_ID.to_string(),
                module,
                source_identity: SOURCE_IDENTITY.to_string(),
                target_identity: if self.target_configured {
                    "postgres:configured".to_string()
                } else {
                    "postgres:unconfigured".to_string()
                },
                source_watermark: 0,
            });
        };
        if let Some(value) = row.try_get::<Option<String>, _>("manifest_json")? {
            return Ok(AuthorityRecoveryRecord::Manifest(serde_json::from_str(
                &value,
            )?));
        }
        if let Some(value) = row.try_get::<Option<String>, _>("recovery_json")? {
            let context: AuthorityRecoveryContext = serde_json::from_str(&value)?;
            return Ok(AuthorityRecoveryRecord::NeverMigrated {
                migration_id: context.migration_id,
                installation_id: context.installation_id,
                module: context.module,
                source_identity: context.source_identity,
                target_identity: context.target_identity,
                source_watermark: context.source_watermark,
            });
        }
        Ok(AuthorityRecoveryRecord::Indeterminate)
    }

    async fn publish_authority_manifest(
        &self,
        manifest: &ModuleAuthorityManifest,
        expected: &AuthorityRecoveryContext,
    ) -> Result<AuthorityPublication> {
        let module = serde_json::to_string(&manifest.module)?;
        let manifest_json = serde_json::to_string(manifest)?;
        let expected_json = serde_json::to_string(expected)?;
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(&format!(
            "SELECT recovery_json, manifest_json FROM {AUTHORITY_TABLE}
              WHERE module = ?1"
        ))
        .bind(module.trim_matches('"'))
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(row) = row else {
            return Ok(AuthorityPublication::Conflict);
        };
        if let Some(existing) = row.try_get::<Option<String>, _>("manifest_json")? {
            let existing: ModuleAuthorityManifest = serde_json::from_str(&existing)?;
            transaction.commit().await?;
            return Ok(if existing == *manifest {
                AuthorityPublication::Committed
            } else {
                AuthorityPublication::Conflict
            });
        }
        let Some(existing) = row.try_get::<Option<String>, _>("recovery_json")? else {
            return Ok(AuthorityPublication::Conflict);
        };
        if existing != expected_json {
            return Ok(AuthorityPublication::Conflict);
        }
        sqlx::query(&format!(
            "UPDATE {AUTHORITY_TABLE}
                SET recovery_json = NULL, manifest_json = ?2
              WHERE module = ?1"
        ))
        .bind(module.trim_matches('"'))
        .bind(manifest_json)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(AuthorityPublication::Committed)
    }
}

#[async_trait]
impl MigrationMetadataStore for SqliteMigrationMetadataStore {
    async fn set_recovery_context(&self, context: &AuthorityRecoveryContext) -> Result<()> {
        if context.source_watermark < 0 {
            bail!("invalid recovery watermark");
        }
        let module = serde_json::to_string(&context.module)?;
        let value = serde_json::to_string(context)?;
        let existing = sqlx::query_scalar::<_, Option<String>>(&format!(
            "SELECT recovery_json FROM {AUTHORITY_TABLE} WHERE module = ?1"
        ))
        .bind(module.trim_matches('"'))
        .fetch_optional(&self.pool)
        .await?
        .flatten();
        if let Some(existing) = existing
            && existing != value
        {
            bail!("SQLite migration recovery context conflicts with durable state");
        }
        sqlx::query(&format!(
            "INSERT INTO {AUTHORITY_TABLE} (module, recovery_json, manifest_json)
             VALUES (?1, ?2, NULL)
             ON CONFLICT(module) DO UPDATE SET recovery_json = excluded.recovery_json"
        ))
        .bind(module.trim_matches('"'))
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_status(&self, module: StorageModule) -> Result<Option<MigrationStatus>> {
        self.load_json(STATUS_TABLE, "module", module_name(module), "value_json")
            .await
    }

    async fn save_status(&self, status: &MigrationStatus) -> Result<()> {
        status.validate()?;
        sqlx::query(&format!(
            "INSERT INTO {STATUS_TABLE} (module, value_json) VALUES (?1, ?2)
             ON CONFLICT(module) DO UPDATE SET value_json = excluded.value_json"
        ))
        .bind(module_name(status.module))
        .bind(serde_json::to_string(status)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn load_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<MigrationCheckpoint>> {
        let query = format!(
            "SELECT value_json FROM {CHECKPOINT_TABLE}
              WHERE migration_id = ?1 AND table_name = ?2"
        );
        sqlx::query_scalar::<_, String>(&query)
            .bind(migration_id)
            .bind(table_name)
            .fetch_optional(&self.pool)
            .await?
            .map(|value| Ok(serde_json::from_str(&value)?))
            .transpose()
    }

    async fn save_checkpoint(&self, checkpoint: &MigrationCheckpoint) -> Result<()> {
        checkpoint.validate()?;
        sqlx::query(&format!(
            "INSERT INTO {CHECKPOINT_TABLE} (migration_id, table_name, value_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(migration_id, table_name) DO UPDATE SET value_json = excluded.value_json"
        ))
        .bind(&checkpoint.migration_id)
        .bind(&checkpoint.table_name)
        .bind(serde_json::to_string(checkpoint)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn enqueue_outbox(&self, item: &MigrationOutboxItem) -> Result<bool> {
        item.validate()?;
        let result = sqlx::query(&format!(
            "INSERT OR IGNORE INTO {OUTBOX_TABLE} (outbox_id, value_json)
             VALUES (?1, ?2)"
        ))
        .bind(&item.outbox_id)
        .bind(serde_json::to_string(item)?)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn acknowledge_outbox(&self, outbox_id: &str, applied_through: i64) -> Result<()> {
        if applied_through < 0 {
            bail!("acknowledged migration watermark must not be negative");
        }
        sqlx::query(&format!("DELETE FROM {OUTBOX_TABLE} WHERE outbox_id = ?1"))
            .bind(outbox_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn reserve_capacity(&self, reservation: &MigrationReservation) -> Result<bool> {
        reservation.validate()?;
        let result = sqlx::query(&format!(
            "INSERT OR IGNORE INTO {RESERVATION_TABLE} (reservation_id, value_json)
             VALUES (?1, ?2)"
        ))
        .bind(&reservation.reservation_id)
        .bind(serde_json::to_string(reservation)?)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn release_capacity(&self, reservation_id: &str) -> Result<()> {
        sqlx::query(&format!(
            "DELETE FROM {RESERVATION_TABLE} WHERE reservation_id = ?1"
        ))
        .bind(reservation_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn recovery_context_from_record(
    record: AuthorityRecoveryRecord,
) -> Result<AuthorityRecoveryContext> {
    match record {
        AuthorityRecoveryRecord::NeverMigrated {
            migration_id,
            installation_id,
            module,
            source_identity,
            target_identity,
            source_watermark,
        }
        | AuthorityRecoveryRecord::Manifest(super::core::ModuleAuthorityManifest {
            migration_id,
            installation_id,
            module,
            source_identity,
            target_identity,
            source_watermark,
            ..
        }) => Ok(AuthorityRecoveryContext {
            migration_id,
            installation_id,
            module,
            source_identity,
            target_identity,
            source_watermark,
        }),
        AuthorityRecoveryRecord::Indeterminate => {
            bail!("module authority recovery is indeterminate")
        }
    }
}

fn expected_postgres_target_identity(config: &StorageConfig) -> &'static str {
    if config
        .postgres_database_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty())
    {
        "postgres:configured"
    } else {
        "postgres:unconfigured"
    }
}

fn validate_manifest_identity(
    manifest: &ModuleAuthorityManifest,
    status: &StorageMigrationResponse,
    expected_target_identity: &str,
) -> Result<()> {
    let status_module = "traffic_archive";
    if manifest.migration_id != MIGRATION_ID
        || manifest.installation_id != INSTALLATION_ID
        || manifest.module != StorageModule::TrafficArchive
        || manifest.source_identity != SOURCE_IDENTITY
        || manifest.target_identity != expected_target_identity
        || status.migration_id.as_deref() != Some(MIGRATION_ID)
        || status.module != status_module
        || status.source.identity != SOURCE_IDENTITY
        || status.target.identity != expected_target_identity
        || manifest.target_identity != status.target.identity
        || manifest.source_watermark < 0
    {
        bail!("durable traffic authority manifest identity does not match runtime state");
    }

    if matches!(
        status.state,
        StorageMigrationState::Postgres
            | StorageMigrationState::Switching
            | StorageMigrationState::Recovering
    ) {
        if status.progress.captured_through != Some(manifest.source_watermark)
            || status.progress.applied_through != Some(manifest.source_watermark)
        {
            bail!("durable traffic authority manifest watermark does not match migration status");
        }
    } else {
        let status_watermarks = [
            status.progress.captured_through,
            status.progress.applied_through,
        ];
        if status_watermarks
            .into_iter()
            .flatten()
            .any(|watermark| watermark > manifest.source_watermark)
        {
            bail!("durable traffic authority manifest watermark regresses migration progress");
        }
    }
    Ok(())
}

fn module_name(module: StorageModule) -> &'static str {
    match module {
        StorageModule::TrafficArchive => "traffic_archive",
        StorageModule::Analytics => "analytics",
        StorageModule::AccountRouting => "account_routing",
        StorageModule::TaskOperations => "task_operations",
        StorageModule::Performance => "performance",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn runtime_router_initializes_and_stops_without_postgres() -> Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        let config = StorageConfig::default();
        let migration = Arc::new(StorageMigrationRuntime::new(&config));
        let shutdown = CancellationToken::new();
        let router =
            StorageRuntimeRouter::initialize(pool, &config, migration, shutdown.clone()).await?;

        let catalog = ModuleCatalog::inspect_sqlite(&router.worker.pool).await?;
        for table in [
            AUTHORITY_TABLE,
            CHECKPOINT_TABLE,
            OUTBOX_TABLE,
            RESERVATION_TABLE,
            STATUS_TABLE,
        ] {
            let owner = catalog
                .owners()
                .iter()
                .find(|owner| owner.table == table)
                .expect("runtime metadata table has an ownership classification");
            assert_eq!(
                owner.module,
                super::super::catalog::ModuleId::TaskOperations
            );
            assert_eq!(owner.transaction_group, "task_operations");
        }

        assert_eq!(
            router
                .current_authority(StorageModule::TrafficArchive)
                .await,
            super::super::core::ModuleAuthority {
                backend: BackendKind::Sqlite,
                generation: 1,
            }
        );
        assert!(!router.postgres_configured());
        router.start().await;
        router.stop().await;
        assert!(shutdown.is_cancelled());
        Ok(())
    }

    #[test]
    fn persisted_migration_phases_resume_from_their_durable_checkpoint() {
        assert_eq!(
            migration_resume_work(StorageMigrationState::Preparing).unwrap(),
            MigrationResumeWork::CopyAndCatchUp
        );
        assert_eq!(
            migration_resume_work(StorageMigrationState::Copying).unwrap(),
            MigrationResumeWork::CopyAndCatchUp
        );
        assert_eq!(
            migration_resume_work(StorageMigrationState::CatchingUp).unwrap(),
            MigrationResumeWork::CatchUp
        );
        assert_eq!(
            migration_resume_work(StorageMigrationState::Verifying).unwrap(),
            MigrationResumeWork::CatchUp
        );
        assert_eq!(
            migration_resume_work(StorageMigrationState::Recovering).unwrap(),
            MigrationResumeWork::CatchUp
        );
        assert_eq!(
            migration_resume_work(StorageMigrationState::Ready).unwrap(),
            MigrationResumeWork::Switch
        );
        assert_eq!(
            migration_resume_work(StorageMigrationState::Switching).unwrap(),
            MigrationResumeWork::Switch
        );
        assert!(migration_resume_work(StorageMigrationState::Postgres).is_err());
    }

    #[tokio::test]
    async fn manifest_identity_requires_exact_watermark_during_transitional_recovery() -> Result<()>
    {
        let config = StorageConfig {
            postgres_database_url: Some("postgresql://user:pass@127.0.0.1:5432/cvm".to_string()),
            ..StorageConfig::default()
        };
        for phase in [
            StorageMigrationState::Switching,
            StorageMigrationState::Recovering,
        ] {
            let migration = StorageMigrationRuntime::new(&config);
            let status = prepare_transitional_migration_status(&migration, phase, 6).await?;
            let manifest = ModuleAuthorityManifest {
                migration_id: MIGRATION_ID.to_string(),
                installation_id: INSTALLATION_ID.to_string(),
                source_identity: SOURCE_IDENTITY.to_string(),
                module: StorageModule::TrafficArchive,
                authority: super::super::core::ModuleAuthority {
                    backend: BackendKind::Postgres,
                    generation: 2,
                },
                source_watermark: 7,
                target_identity: "postgres:configured".to_string(),
            };

            let error = validate_manifest_identity(&manifest, &status, "postgres:configured")
                .expect_err("transitional status must match the manifest watermark exactly");
            assert!(error.to_string().contains("watermark"));
        }
        Ok(())
    }

    #[tokio::test]
    async fn postgres_worker_recovery_rejects_manifest_identity_mismatch() -> Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        let config = StorageConfig {
            postgres_database_url: Some("postgresql://user:pass@127.0.0.1:5432/cvm".to_string()),
            ..StorageConfig::default()
        };
        let migration = Arc::new(StorageMigrationRuntime::new(&config));
        let status = prepare_transitional_migration_status(
            migration.as_ref(),
            StorageMigrationState::Switching,
            7,
        )
        .await?;
        let metadata = Arc::new(SqliteMigrationMetadataStore::new(pool.clone(), true));
        metadata.ensure_schema().await?;
        let context = AuthorityRecoveryContext {
            migration_id: MIGRATION_ID.to_string(),
            installation_id: INSTALLATION_ID.to_string(),
            module: StorageModule::TrafficArchive,
            source_identity: SOURCE_IDENTITY.to_string(),
            target_identity: "postgres:configured".to_string(),
            source_watermark: 7,
        };
        metadata.set_recovery_context(&context).await?;
        let manifest = ModuleAuthorityManifest {
            migration_id: context.migration_id.clone(),
            installation_id: context.installation_id.clone(),
            source_identity: context.source_identity.clone(),
            module: context.module,
            authority: super::super::core::ModuleAuthority {
                backend: BackendKind::Postgres,
                generation: 2,
            },
            source_watermark: context.source_watermark,
            target_identity: context.target_identity.clone(),
        };
        assert_eq!(
            metadata
                .publish_authority_manifest(&manifest, &context)
                .await?,
            AuthorityPublication::Committed
        );
        let mut corrupted_manifest = manifest;
        corrupted_manifest.source_identity = "sqlite:other".to_string();
        sqlx::query(
            "UPDATE storage_migration_authority
             SET manifest_json = ?1
             WHERE module = 'traffic_archive'",
        )
        .bind(serde_json::to_string(&corrupted_manifest)?)
        .execute(&pool)
        .await?;

        let worker = StorageMigrationWorker::new(
            pool,
            config,
            migration,
            CancellationToken::new(),
            metadata,
            ModuleGenerationGate::new(
                StorageModule::TrafficArchive,
                super::super::core::ModuleAuthority {
                    backend: BackendKind::Postgres,
                    generation: 2,
                },
            ),
            SourceWriteFence::new(1),
        );
        let error = worker
            .run_migration(status)
            .await
            .expect_err("worker recovery must reject a manifest with a mismatched identity");
        assert!(error.to_string().contains("identity"));
        Ok(())
    }

    async fn prepare_transitional_migration_status(
        migration: &StorageMigrationRuntime,
        phase: StorageMigrationState,
        watermark: i64,
    ) -> Result<StorageMigrationResponse> {
        migration
            .apply_control(
                crate::storage::migration_status::StorageMigrationControlRequest { paused: false },
            )
            .await?;
        for phase in [
            StorageMigrationState::Copying,
            StorageMigrationState::CatchingUp,
            StorageMigrationState::Verifying,
            StorageMigrationState::Ready,
        ] {
            migration
                .publish_progress(StorageMigrationProgress {
                    phase,
                    ..StorageMigrationProgress::default()
                })
                .await?;
        }
        let status = migration
            .publish_progress(StorageMigrationProgress {
                phase: StorageMigrationState::Switching,
                captured_through: Some(watermark),
                applied_through: Some(watermark),
                ..StorageMigrationProgress::default()
            })
            .await?;
        if phase == StorageMigrationState::Switching {
            Ok(status)
        } else {
            migration
                .publish_progress(StorageMigrationProgress {
                    phase,
                    captured_through: Some(watermark),
                    applied_through: Some(watermark),
                    ..StorageMigrationProgress::default()
                })
                .await
        }
    }
}
