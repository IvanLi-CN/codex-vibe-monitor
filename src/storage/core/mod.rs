use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
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
    backend: StorageTransactionBackend,
    _lease: ModuleGenerationLease,
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
            backend,
            _lease: lease,
        })
    }
}

impl StorageTransaction {
    pub(crate) async fn commit(self) -> Result<()> {
        let Self { backend, _lease } = self;
        match backend {
            StorageTransactionBackend::Sqlite(transaction) => {
                transaction.commit().await.map_err(Into::into)
            }
            StorageTransactionBackend::Postgres(transaction) => {
                transaction.commit().await.map_err(Into::into)
            }
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
    active: AtomicUsize,
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
            active: AtomicUsize::new(0),
            changed: Notify::new(),
        }))
    }

    pub(crate) async fn acquire(&self, module: StorageModule) -> Result<ModuleGenerationLease> {
        if module != self.0.module {
            bail!("module generation gate cannot issue a lease for another module");
        }
        let state = self.0.state.lock().await;
        if !state.accepting {
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
        authority: ModuleAuthority,
        drain_budget: Duration,
    ) -> Result<()> {
        if module != self.0.module {
            bail!("module generation gate cannot switch authority for another module");
        }
        let deadline = tokio::time::Instant::now() + drain_budget;
        {
            let mut state = self.0.state.lock().await;
            if authority.generation <= state.authority.generation {
                bail!("module authority generation must increase monotonically");
            }
            state.accepting = false;
        }
        loop {
            let notified = self.0.changed.notified();
            let drained = self.0.active.load(Ordering::Acquire) == 0;
            if drained {
                let mut state = self.0.state.lock().await;
                state.authority = authority;
                state.accepting = true;
                self.0.changed.notify_waiters();
                return Ok(());
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                self.0.state.lock().await.accepting = true;
                self.0.changed.notify_waiters();
                bail!("timed out draining {module:?} before authority commit");
            }
        }
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

pub(crate) trait MigrationMetadataStore: Send + Sync {
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
