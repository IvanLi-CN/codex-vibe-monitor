use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, bail};
use sqlx::{PgPool, Sqlite, SqlitePool, Transaction};
use tokio::sync::{Mutex, Notify};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum StorageModule {
    TrafficArchive,
    Analytics,
    AccountRouting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendKind {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModuleAuthority {
    pub(crate) backend: BackendKind,
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleAuthorityManifest {
    pub(crate) installation_id: String,
    pub(crate) source_identity: String,
    pub(crate) module: StorageModule,
    pub(crate) authority: ModuleAuthority,
    pub(crate) source_watermark: i64,
    pub(crate) target_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorityRecoveryContext {
    pub(crate) installation_id: String,
    pub(crate) source_identity: String,
    pub(crate) target_identity: String,
    pub(crate) source_watermark: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthorityRecoveryRecord {
    /// The durable store confirms this installation has never cut over this module.
    NeverMigrated {
        installation_id: String,
        source_identity: String,
    },
    Manifest(ModuleAuthorityManifest),
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorityPublication {
    NotCommitted,
    Committed,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModuleLease {
    pub(crate) module: StorageModule,
    pub(crate) authority: ModuleAuthority,
}

#[derive(Clone)]
pub(crate) struct StoragePools {
    pub(crate) sqlite: SqlitePool,
    pub(crate) postgres: Option<PgPool>,
}

enum StorageTransactionBackend {
    Sqlite(Transaction<'static, Sqlite>),
    Postgres(Transaction<'static, sqlx::Postgres>),
}

pub(crate) struct StorageTransaction {
    backend: Option<StorageTransactionBackend>,
    lease: Option<ModuleGenerationLease>,
}

impl StoragePools {
    pub(crate) async fn begin(&self, lease: ModuleGenerationLease) -> Result<StorageTransaction> {
        let backend = match lease.lease.authority.backend {
            BackendKind::Sqlite => StorageTransactionBackend::Sqlite(self.sqlite.begin().await?),
            BackendKind::Postgres => {
                let pool = self.postgres.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("PostgreSQL authority selected without a PostgreSQL pool")
                })?;
                StorageTransactionBackend::Postgres(pool.begin().await?)
            }
        };
        Ok(StorageTransaction {
            backend: Some(backend),
            lease: Some(lease),
        })
    }
}

#[derive(Clone, Copy)]
enum TransactionCompletion {
    Commit,
    Rollback,
}

async fn complete_transaction(
    backend: StorageTransactionBackend,
    completion: TransactionCompletion,
    lease: Option<ModuleGenerationLease>,
) -> Result<()> {
    let result = match (backend, completion) {
        (StorageTransactionBackend::Sqlite(transaction), TransactionCompletion::Commit) => {
            transaction.commit().await.map_err(Into::into)
        }
        (StorageTransactionBackend::Postgres(transaction), TransactionCompletion::Commit) => {
            transaction.commit().await.map_err(Into::into)
        }
        (StorageTransactionBackend::Sqlite(transaction), TransactionCompletion::Rollback) => {
            transaction.rollback().await.map_err(Into::into)
        }
        (StorageTransactionBackend::Postgres(transaction), TransactionCompletion::Rollback) => {
            transaction.rollback().await.map_err(Into::into)
        }
    };
    if result.is_err()
        && let Some(lease) = lease.as_ref()
    {
        lease.gate.recovery_required.store(true, Ordering::Release);
    }
    drop(lease);
    result
}

impl StorageTransaction {
    pub(crate) async fn commit(mut self) -> Result<()> {
        let Some(backend) = self.backend.take() else {
            bail!("storage transaction is already complete");
        };
        let lease = self.lease.take();
        tokio::spawn(async move {
            complete_transaction(backend, TransactionCompletion::Commit, lease).await
        })
        .await??;
        Ok(())
    }

    pub(crate) async fn rollback(mut self) -> Result<()> {
        let Some(backend) = self.backend.take() else {
            bail!("storage transaction is already complete");
        };
        let lease = self.lease.take();
        tokio::spawn(async move {
            complete_transaction(backend, TransactionCompletion::Rollback, lease).await
        })
        .await??;
        Ok(())
    }
}

impl Drop for StorageTransaction {
    fn drop(&mut self) {
        let Some(backend) = self.backend.take() else {
            return;
        };
        let lease = self.lease.take();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = complete_transaction(backend, TransactionCompletion::Rollback, lease).await;
            });
        } else {
            if let Some(lease) = lease.as_ref() {
                lease.gate.recovery_required.store(true, Ordering::Release);
            }
            drop(backend);
            // Without a runtime, rollback cannot be awaited and the gate must stay closed.
            std::mem::forget(lease);
        }
    }
}

pub(crate) fn validate_transaction_leases(leases: &[ModuleLease]) -> Result<ModuleLease> {
    let Some(first) = leases.first().copied() else {
        bail!("a storage transaction requires at least one module lease");
    };
    if leases.iter().any(|lease| *lease != first) {
        bail!("a storage transaction cannot mix modules or backend generations");
    }
    Ok(first)
}

#[derive(Debug)]
struct AdmissionState {
    authority: ModuleAuthority,
    accepting: bool,
}

#[derive(Debug)]
struct ModuleGate {
    module: StorageModule,
    state: Mutex<AdmissionState>,
    switching: Mutex<()>,
    active: AtomicUsize,
    recovery_required: AtomicBool,
    changed: Notify,
}

#[derive(Debug, Clone)]
pub(crate) struct ModuleGenerationGate(Arc<ModuleGate>);

#[derive(Debug)]
pub(crate) struct ModuleGenerationLease {
    pub(crate) lease: ModuleLease,
    gate: Arc<ModuleGate>,
}

impl Drop for ModuleGenerationLease {
    fn drop(&mut self) {
        self.gate.active.fetch_sub(1, Ordering::AcqRel);
        self.gate.changed.notify_one();
    }
}

impl ModuleGenerationGate {
    pub(crate) fn new(module: StorageModule, authority: ModuleAuthority) -> Self {
        Self(Arc::new(ModuleGate {
            module,
            state: Mutex::new(AdmissionState {
                authority,
                accepting: true,
            }),
            switching: Mutex::new(()),
            active: AtomicUsize::new(0),
            recovery_required: AtomicBool::new(false),
            changed: Notify::new(),
        }))
    }

    pub(crate) async fn acquire(&self, module: StorageModule) -> Result<ModuleGenerationLease> {
        if module != self.0.module {
            bail!("module generation gate cannot issue a lease for another module");
        }
        let state = self.0.state.lock().await;
        if !state.accepting || self.0.recovery_required.load(Ordering::Acquire) {
            bail!("module storage admission is closed during authority switching");
        }
        self.0.active.fetch_add(1, Ordering::AcqRel);
        Ok(ModuleGenerationLease {
            lease: ModuleLease {
                module,
                authority: state.authority,
            },
            gate: Arc::clone(&self.0),
        })
    }

    pub(crate) async fn switch_authority(
        &self,
        module: StorageModule,
        manifest: ModuleAuthorityManifest,
        expected: AuthorityRecoveryContext,
        drain_budget: Duration,
        metadata: Arc<dyn AuthorityManifestStore>,
    ) -> Result<()> {
        if module != self.0.module {
            bail!("module generation gate cannot switch authority for another module");
        }
        validate_switch_manifest(module, &manifest, &expected)?;
        let gate = self.clone();
        tokio::spawn(async move {
            gate.switch_authority_inner(module, manifest, expected, drain_budget, metadata)
                .await
        })
        .await??;
        Ok(())
    }

    async fn switch_authority_inner(
        &self,
        module: StorageModule,
        manifest: ModuleAuthorityManifest,
        expected: AuthorityRecoveryContext,
        drain_budget: Duration,
        metadata: Arc<dyn AuthorityManifestStore>,
    ) -> Result<()> {
        let _switch = self.0.switching.lock().await;
        let deadline = tokio::time::Instant::now() + drain_budget;
        {
            let mut state = self.0.state.lock().await;
            if self.0.recovery_required.load(Ordering::Acquire) {
                bail!("module authority requires recovery before another switch");
            }
            validate_switch_manifest(module, &manifest, &expected)?;
            if state.authority.backend != BackendKind::Sqlite
                || manifest.authority.generation <= state.authority.generation
            {
                bail!("module authority manifest does not advance the selected module generation");
            }
            state.accepting = false;
        }
        loop {
            let notified = self.0.changed.notified();
            if self.0.active.load(Ordering::Acquire) == 0 {
                break;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                self.reopen_admission().await;
                bail!("timed out draining {module:?} before authority commit");
            }
        }

        self.0.recovery_required.store(true, Ordering::Release);
        let publication = match metadata.publish_authority_manifest(&manifest).await {
            Ok(publication) => publication,
            Err(error) => {
                self.0.recovery_required.store(true, Ordering::Release);
                return Err(error);
            }
        };
        match publication {
            AuthorityPublication::NotCommitted => {
                self.0.recovery_required.store(false, Ordering::Release);
                self.reopen_admission().await;
                bail!(
                    "module authority manifest was not committed; source authority remains active"
                );
            }
            AuthorityPublication::Committed => {
                let mut state = self.0.state.lock().await;
                if manifest.authority.generation <= state.authority.generation {
                    bail!("published module authority generation is no longer monotonic");
                }
                state.authority = manifest.authority;
                self.0.recovery_required.store(false, Ordering::Release);
                state.accepting = true;
                self.0.changed.notify_waiters();
                Ok(())
            }
            AuthorityPublication::Indeterminate => {
                self.0.recovery_required.store(true, Ordering::Release);
                bail!(
                    "module authority publication is indeterminate; admission remains closed for recovery"
                )
            }
        }
    }

    async fn reopen_admission(&self) {
        self.0.state.lock().await.accepting = true;
        self.0.changed.notify_waiters();
    }

    pub(crate) async fn recover_authority(
        &self,
        metadata: &dyn AuthorityManifestStore,
        expected: &AuthorityRecoveryContext,
    ) -> Result<()> {
        let _switch = self.0.switching.lock().await;
        self.0.state.lock().await.accepting = false;
        self.0.recovery_required.store(true, Ordering::Release);
        if self.0.active.load(Ordering::Acquire) != 0 {
            bail!("cannot recover module authority while generation leases are active");
        }
        match metadata.load_authority_manifest(self.0.module).await? {
            AuthorityRecoveryRecord::NeverMigrated {
                installation_id,
                source_identity,
            } => {
                if installation_id != expected.installation_id
                    || source_identity != expected.source_identity
                {
                    bail!("no-cutover recovery marker belongs to another installation or source");
                }
                if self.0.state.lock().await.authority.backend != BackendKind::Sqlite {
                    bail!("missing authority manifest cannot restore a PostgreSQL-selected module");
                }
            }
            AuthorityRecoveryRecord::Manifest(manifest) => {
                if manifest.module != self.0.module
                    || manifest.installation_id != expected.installation_id
                    || manifest.source_identity != expected.source_identity
                    || manifest.target_identity != expected.target_identity
                    || manifest.source_watermark != expected.source_watermark
                    || manifest.authority.backend != BackendKind::Postgres
                {
                    bail!(
                        "recovered authority manifest does not match the active migration identity"
                    );
                }
                let mut state = self.0.state.lock().await;
                if manifest.authority.generation < state.authority.generation
                    || (manifest.authority.generation == state.authority.generation
                        && manifest.authority.backend != state.authority.backend)
                {
                    bail!("recovered module authority manifest regresses the selected generation");
                }
                state.authority = manifest.authority;
            }
            AuthorityRecoveryRecord::Indeterminate => {
                bail!("module authority recovery is indeterminate; admission remains closed");
            }
        }
        self.0.recovery_required.store(false, Ordering::Release);
        self.reopen_admission().await;
        Ok(())
    }

    pub(crate) async fn current_authorities(
        gates: &BTreeMap<StorageModule, Self>,
    ) -> BTreeMap<StorageModule, ModuleAuthority> {
        let mut authorities = BTreeMap::new();
        for (module, gate) in gates {
            authorities.insert(*module, gate.0.state.lock().await.authority);
        }
        authorities
    }
}

fn validate_switch_manifest(
    module: StorageModule,
    manifest: &ModuleAuthorityManifest,
    expected: &AuthorityRecoveryContext,
) -> Result<()> {
    if manifest.module != module
        || manifest.installation_id != expected.installation_id
        || manifest.source_identity != expected.source_identity
        || manifest.target_identity != expected.target_identity
        || manifest.source_watermark != expected.source_watermark
        || manifest.authority.backend != BackendKind::Postgres
    {
        bail!("authority manifest does not match the verified PostgreSQL migration target");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MigrationState {
    Pending,
    Preparing,
    Copying,
    CatchingUp,
    Verifying,
    Ready,
    Switching,
    Postgres,
    Paused,
    Blocked,
    Recovering,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationStatus {
    pub(crate) migration_id: String,
    pub(crate) module: StorageModule,
    pub(crate) state: MigrationState,
    pub(crate) source_identity: String,
    pub(crate) authority_generation: u64,
    pub(crate) capacity_bytes: u64,
    pub(crate) held_bytes: u64,
    pub(crate) hold_reason: Option<String>,
    pub(crate) updated_at_epoch_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationCheckpoint {
    pub(crate) migration_id: String,
    pub(crate) table_name: String,
    pub(crate) stable_key: String,
    pub(crate) cursor: Option<String>,
    pub(crate) captured_through: Option<i64>,
    pub(crate) applied_through: Option<i64>,
    pub(crate) source_identity: String,
    pub(crate) authority_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationOutboxItem {
    pub(crate) outbox_id: String,
    pub(crate) source_identity: String,
    pub(crate) source_version: i64,
    pub(crate) module: StorageModule,
    pub(crate) authority_generation: u64,
    pub(crate) payload_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationReservation {
    pub(crate) reservation_id: String,
    pub(crate) migration_id: String,
    pub(crate) source_identity: String,
    pub(crate) module: StorageModule,
    pub(crate) authority_generation: u64,
    pub(crate) first_stable_id: String,
    pub(crate) last_stable_id: String,
    pub(crate) reserved_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmissionDecision {
    Allow,
    Hold { reason: &'static str },
}

pub(crate) trait MigrationTaskAdmission: Send + Sync {
    fn decision(&self, task_key: &str) -> AdmissionDecision;
}

#[async_trait::async_trait]
pub(crate) trait AuthorityManifestStore: Send + Sync {
    async fn load_authority_manifest(
        &self,
        module: StorageModule,
    ) -> Result<AuthorityRecoveryRecord>;
    async fn publish_authority_manifest(
        &self,
        manifest: &ModuleAuthorityManifest,
    ) -> Result<AuthorityPublication>;
}

pub(crate) trait MigrationMetadataStore: AuthorityManifestStore {
    async fn load_status(&self, module: StorageModule) -> Result<Option<MigrationStatus>>;
    async fn save_status(&self, status: &MigrationStatus) -> Result<()>;
    async fn load_checkpoint(
        &self,
        migration_id: &str,
        table_name: &str,
    ) -> Result<Option<MigrationCheckpoint>>;
    async fn save_checkpoint(&self, checkpoint: &MigrationCheckpoint) -> Result<()>;
    async fn enqueue_outbox(&self, item: &MigrationOutboxItem) -> Result<bool>;
    async fn acknowledge_outbox(&self, outbox_id: &str, applied_through: i64) -> Result<()>;
    async fn reserve_capacity(&self, reservation: &MigrationReservation) -> Result<bool>;
    async fn release_capacity(&self, reservation_id: &str) -> Result<()>;
}
