use std::{
    collections::BTreeMap,
    future::Future,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, bail};
use sqlx::{PgPool, Sqlite, SqlitePool, Transaction};
use tokio::sync::{Mutex, Notify};

use super::config::StorageConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum StorageModule {
    TrafficArchive,
    Analytics,
    AccountRouting,
    TaskOperations,
    Performance,
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
    pub(crate) migration_id: String,
    pub(crate) installation_id: String,
    pub(crate) source_identity: String,
    pub(crate) module: StorageModule,
    pub(crate) authority: ModuleAuthority,
    pub(crate) source_watermark: i64,
    pub(crate) target_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorityRecoveryContext {
    pub(crate) migration_id: String,
    pub(crate) installation_id: String,
    pub(crate) module: StorageModule,
    pub(crate) source_identity: String,
    pub(crate) target_identity: String,
    pub(crate) source_watermark: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthorityRecoveryRecord {
    /// The durable store confirms this migration attempt has not cut over the module.
    NeverMigrated {
        migration_id: String,
        installation_id: String,
        module: StorageModule,
        source_identity: String,
        target_identity: String,
        source_watermark: i64,
    },
    Manifest(ModuleAuthorityManifest),
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthorityPublication {
    /// The store confirms the expected recovery context still selects the source authority.
    NotCommitted,
    Committed,
    /// Compare-and-publish found a different durable recovery context or authority.
    Conflict,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModuleLease {
    pub(crate) module: StorageModule,
    pub(crate) authority: ModuleAuthority,
}

/// Contract-only storage pools for module-scoped transaction tests and later adapters.
/// pg01 does not install these pools into the runtime router or migrate production writers.
#[derive(Clone)]
pub(crate) struct StoragePools {
    sqlite: SqlitePool,
    postgres: Option<PgPool>,
}

pub(crate) trait StorageTransactionModule {
    const MODULE: StorageModule;
}

pub(crate) struct TrafficArchiveTransaction;
pub(crate) struct AnalyticsTransaction;
pub(crate) struct AccountRoutingTransaction;
pub(crate) struct TaskOperationsTransaction;
pub(crate) struct PerformanceTransaction;

impl StorageTransactionModule for TrafficArchiveTransaction {
    const MODULE: StorageModule = StorageModule::TrafficArchive;
}

impl StorageTransactionModule for AnalyticsTransaction {
    const MODULE: StorageModule = StorageModule::Analytics;
}

impl StorageTransactionModule for AccountRoutingTransaction {
    const MODULE: StorageModule = StorageModule::AccountRouting;
}

impl StorageTransactionModule for TaskOperationsTransaction {
    const MODULE: StorageModule = StorageModule::TaskOperations;
}

impl StorageTransactionModule for PerformanceTransaction {
    const MODULE: StorageModule = StorageModule::Performance;
}

enum StorageTransactionBackend {
    Sqlite(Transaction<'static, Sqlite>),
    Postgres(Transaction<'static, sqlx::Postgres>),
}

pub(crate) struct StorageTransaction {
    backend: Option<StorageTransactionBackend>,
    lease: Option<ModuleGenerationLease>,
}

pub(crate) struct TypedStorageTransaction<M: StorageTransactionModule> {
    transaction: StorageTransaction,
    _module: PhantomData<M>,
}

impl StoragePools {
    pub(crate) fn sqlite_only(sqlite: SqlitePool) -> Self {
        Self {
            sqlite,
            postgres: None,
        }
    }

    pub(crate) async fn from_sqlite_and_config(
        sqlite: SqlitePool,
        config: &StorageConfig,
    ) -> Result<Self> {
        Ok(Self {
            sqlite,
            postgres: config.connect_postgres().await?,
        })
    }

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

    pub(crate) async fn begin_typed<M: StorageTransactionModule>(
        &self,
        lease: ModuleGenerationLease,
    ) -> Result<TypedStorageTransaction<M>> {
        if lease.lease.module != M::MODULE {
            bail!(
                "typed storage transaction expects {:?}, received {:?}",
                M::MODULE,
                lease.lease.module
            );
        }
        Ok(TypedStorageTransaction {
            transaction: self.begin(lease).await?,
            _module: PhantomData,
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
    mut completion_lease: TransactionCompletionLease,
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
    if result.is_ok() {
        completion_lease.mark_completed();
    }
    drop(completion_lease);
    result
}

impl StorageTransaction {
    pub(crate) fn sqlite_transaction(&mut self) -> Result<&mut Transaction<'static, Sqlite>> {
        match self.backend.as_mut() {
            Some(StorageTransactionBackend::Sqlite(transaction)) => Ok(transaction),
            Some(StorageTransactionBackend::Postgres(_)) => {
                bail!("storage transaction is owned by PostgreSQL")
            }
            None => bail!("storage transaction is already complete"),
        }
    }

    pub(crate) fn postgres_transaction(
        &mut self,
    ) -> Result<&mut Transaction<'static, sqlx::Postgres>> {
        match self.backend.as_mut() {
            Some(StorageTransactionBackend::Sqlite(_)) => {
                bail!("storage transaction is owned by SQLite")
            }
            Some(StorageTransactionBackend::Postgres(transaction)) => Ok(transaction),
            None => bail!("storage transaction is already complete"),
        }
    }

    pub(crate) async fn commit(mut self) -> Result<()> {
        let Some(backend) = self.backend.take() else {
            bail!("storage transaction is already complete");
        };
        let lease = self.lease.take();
        let completion_lease = TransactionCompletionLease::new(lease);
        tokio::spawn(async move {
            complete_transaction(backend, TransactionCompletion::Commit, completion_lease).await
        })
        .await??;
        Ok(())
    }

    pub(crate) async fn rollback(mut self) -> Result<()> {
        let Some(backend) = self.backend.take() else {
            bail!("storage transaction is already complete");
        };
        let lease = self.lease.take();
        let completion_lease = TransactionCompletionLease::new(lease);
        tokio::spawn(async move {
            complete_transaction(backend, TransactionCompletion::Rollback, completion_lease).await
        })
        .await??;
        Ok(())
    }
}

impl<M: StorageTransactionModule> TypedStorageTransaction<M> {
    pub(crate) fn sqlite_transaction(&mut self) -> Result<&mut Transaction<'static, Sqlite>> {
        self.transaction.sqlite_transaction()
    }

    pub(crate) fn postgres_transaction(
        &mut self,
    ) -> Result<&mut Transaction<'static, sqlx::Postgres>> {
        self.transaction.postgres_transaction()
    }

    pub(crate) async fn commit(self) -> Result<()> {
        self.transaction.commit().await
    }

    pub(crate) async fn rollback(self) -> Result<()> {
        self.transaction.rollback().await
    }

    pub(crate) fn module(&self) -> StorageModule {
        M::MODULE
    }
}

impl Drop for StorageTransaction {
    fn drop(&mut self) {
        let Some(backend) = self.backend.take() else {
            return;
        };
        let lease = self.lease.take();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let completion_lease = TransactionCompletionLease::new(lease);
            runtime.spawn(async move {
                let _ = complete_transaction(
                    backend,
                    TransactionCompletion::Rollback,
                    completion_lease,
                )
                .await;
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
    publication_in_flight: AtomicBool,
    changed: Notify,
    #[cfg(test)]
    switch_attempts: AtomicUsize,
    #[cfg(test)]
    switch_attempts_changed: Notify,
    #[cfg(test)]
    pause_acquire_after_check: AtomicBool,
    #[cfg(test)]
    acquire_check_reached: Notify,
    #[cfg(test)]
    acquire_check_release: Notify,
}

struct PublicationInFlightGuard(Arc<ModuleGate>);

impl Drop for PublicationInFlightGuard {
    fn drop(&mut self) {
        self.0.publication_in_flight.store(false, Ordering::Release);
        self.0.changed.notify_one();
    }
}

fn spawn_tracked_publication<F, T>(
    gate: Arc<ModuleGate>,
    publication: F,
) -> tokio::task::JoinHandle<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    gate.publication_in_flight.store(true, Ordering::Release);
    let guard = PublicationInFlightGuard(Arc::clone(&gate));
    tokio::spawn(async move {
        let _publication = guard;
        publication.await
    })
}

#[derive(Debug, Clone)]
pub(crate) struct ModuleGenerationGate(Arc<ModuleGate>);

#[derive(Debug)]
pub(crate) struct ModuleGenerationLease {
    pub(crate) lease: ModuleLease,
    gate: Arc<ModuleGate>,
}

struct TransactionCompletionLease {
    lease: Option<ModuleGenerationLease>,
    completed: bool,
}

impl TransactionCompletionLease {
    fn new(lease: Option<ModuleGenerationLease>) -> Self {
        Self {
            lease,
            completed: false,
        }
    }

    fn mark_completed(&mut self) {
        self.completed = true;
    }
}

impl Drop for TransactionCompletionLease {
    fn drop(&mut self) {
        if !self.completed
            && let Some(lease) = self.lease.as_ref()
        {
            lease.gate.recovery_required.store(true, Ordering::Release);
        }
        drop(self.lease.take());
    }
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
            publication_in_flight: AtomicBool::new(false),
            changed: Notify::new(),
            #[cfg(test)]
            switch_attempts: AtomicUsize::new(0),
            #[cfg(test)]
            switch_attempts_changed: Notify::new(),
            #[cfg(test)]
            pause_acquire_after_check: AtomicBool::new(false),
            #[cfg(test)]
            acquire_check_reached: Notify::new(),
            #[cfg(test)]
            acquire_check_release: Notify::new(),
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
        #[cfg(test)]
        if self.0.pause_acquire_after_check.load(Ordering::Acquire) {
            self.0.acquire_check_reached.notify_one();
            self.0.acquire_check_release.notified().await;
        }
        self.0.active.fetch_add(1, Ordering::AcqRel);
        if self.0.recovery_required.load(Ordering::Acquire) {
            self.0.active.fetch_sub(1, Ordering::AcqRel);
            self.0.changed.notify_one();
            bail!("module storage admission is closed during authority recovery");
        }
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
        #[cfg(test)]
        {
            self.0.switch_attempts.fetch_add(1, Ordering::AcqRel);
            self.0.switch_attempts_changed.notify_one();
        }
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

        {
            let _state = self.0.state.lock().await;
            if self.0.recovery_required.load(Ordering::Acquire) {
                bail!("module authority requires recovery after lease drain");
            }
            self.0.recovery_required.store(true, Ordering::Release);
        }
        if tokio::time::Instant::now() >= deadline {
            self.0.recovery_required.store(false, Ordering::Release);
            self.reopen_admission().await;
            bail!("timed out before publishing {module:?} authority");
        }

        let publication_manifest = manifest.clone();
        let publication_expected = expected.clone();
        // Bound the wait without cancelling a store operation that may still commit.
        let publication_task = spawn_tracked_publication(Arc::clone(&self.0), async move {
            metadata
                .publish_authority_manifest(&publication_manifest, &publication_expected)
                .await
        });
        let publication = match tokio::time::timeout_at(deadline, publication_task).await {
            Err(_) => {
                bail!(
                    "module authority publication exceeded the switch deadline; outcome is indeterminate and admission remains closed for recovery"
                )
            }
            Ok(Err(error)) => {
                bail!(
                    "module authority publication task failed; outcome is indeterminate and admission remains closed for recovery: {error}"
                )
            }
            Ok(Ok(Err(error))) => {
                self.0.recovery_required.store(true, Ordering::Release);
                return Err(error);
            }
            Ok(Ok(Ok(publication))) => publication,
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
            AuthorityPublication::Conflict => {
                self.0.recovery_required.store(true, Ordering::Release);
                bail!(
                    "module authority publication conflicted with durable state; admission remains closed for recovery"
                )
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
        if expected.module != self.0.module {
            bail!("module authority recovery context targets another module");
        }
        if self.0.active.load(Ordering::Acquire) != 0 {
            bail!("cannot recover module authority while generation leases are active");
        }
        loop {
            let notified = self.0.changed.notified();
            if !self.0.publication_in_flight.load(Ordering::Acquire) {
                break;
            }
            notified.await;
        }
        match metadata.load_authority_manifest(self.0.module).await? {
            AuthorityRecoveryRecord::NeverMigrated {
                migration_id,
                installation_id,
                module,
                source_identity,
                target_identity,
                source_watermark,
            } => {
                if migration_id != expected.migration_id
                    || installation_id != expected.installation_id
                    || module != expected.module
                    || source_identity != expected.source_identity
                    || target_identity != expected.target_identity
                    || source_watermark != expected.source_watermark
                {
                    bail!("no-cutover recovery marker belongs to another migration identity");
                }
                if self.0.state.lock().await.authority.backend != BackendKind::Sqlite {
                    bail!("missing authority manifest cannot restore a PostgreSQL-selected module");
                }
            }
            AuthorityRecoveryRecord::Manifest(manifest) => {
                if manifest.module != self.0.module
                    || manifest.migration_id != expected.migration_id
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

    #[cfg(test)]
    pub(crate) fn require_recovery_for_test(&self) {
        self.0.recovery_required.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn pause_acquire_after_check_for_test(&self) {
        self.0
            .pause_acquire_after_check
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) async fn wait_for_acquire_check_for_test(&self) {
        self.0.acquire_check_reached.notified().await;
    }

    #[cfg(test)]
    pub(crate) fn release_acquire_check_for_test(&self) {
        self.0.acquire_check_release.notify_one();
    }

    #[cfg(test)]
    pub(crate) fn active_leases_for_test(&self) -> usize {
        self.0.active.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn pending_completion_for_test(
        &self,
        lease: ModuleGenerationLease,
    ) -> tokio::task::JoinHandle<()> {
        let completion_lease = TransactionCompletionLease::new(Some(lease));
        tokio::spawn(async move {
            let _completion_lease = completion_lease;
            std::future::pending::<()>().await;
        })
    }

    #[cfg(test)]
    pub(crate) fn pending_publication_for_test(&self) -> tokio::task::JoinHandle<()> {
        spawn_tracked_publication(Arc::clone(&self.0), std::future::pending())
    }

    #[cfg(test)]
    pub(crate) async fn wait_for_switch_attempts_for_test(&self, target: usize) {
        loop {
            let notified = self.0.switch_attempts_changed.notified();
            if self.0.switch_attempts.load(Ordering::Acquire) >= target {
                return;
            }
            notified.await;
        }
    }

    #[cfg(test)]
    pub(crate) fn switch_attempts_for_test(&self) -> usize {
        self.0.switch_attempts.load(Ordering::Acquire)
    }
}

fn validate_switch_manifest(
    module: StorageModule,
    manifest: &ModuleAuthorityManifest,
    expected: &AuthorityRecoveryContext,
) -> Result<()> {
    if expected.module != module
        || manifest.module != module
        || manifest.migration_id != expected.migration_id
        || manifest.installation_id != expected.installation_id
        || manifest.source_identity != expected.source_identity
        || manifest.target_identity != expected.target_identity
        || manifest.source_watermark != expected.source_watermark
        || manifest.authority.backend != BackendKind::Postgres
    {
        bail!("authority manifest does not match the verified PostgreSQL migration attempt");
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

impl MigrationState {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Postgres | Self::Failed)
    }

    pub(crate) fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match self {
            Self::Pending => matches!(
                next,
                Self::Preparing | Self::Paused | Self::Blocked | Self::Failed
            ),
            Self::Preparing => matches!(
                next,
                Self::Copying | Self::Paused | Self::Blocked | Self::Failed
            ),
            Self::Copying => matches!(
                next,
                Self::CatchingUp | Self::Paused | Self::Blocked | Self::Failed
            ),
            Self::CatchingUp => matches!(
                next,
                Self::Copying | Self::Verifying | Self::Paused | Self::Blocked | Self::Failed
            ),
            Self::Verifying => matches!(
                next,
                Self::Ready | Self::CatchingUp | Self::Paused | Self::Blocked | Self::Failed
            ),
            Self::Ready => matches!(
                next,
                Self::Switching | Self::Paused | Self::Blocked | Self::Failed
            ),
            Self::Switching => matches!(
                next,
                Self::Postgres | Self::CatchingUp | Self::Recovering | Self::Failed
            ),
            Self::Paused => matches!(
                next,
                Self::Preparing
                    | Self::Copying
                    | Self::CatchingUp
                    | Self::Verifying
                    | Self::Ready
                    | Self::Blocked
                    | Self::Failed
            ),
            Self::Blocked => matches!(
                next,
                Self::Preparing
                    | Self::Copying
                    | Self::CatchingUp
                    | Self::Verifying
                    | Self::Ready
                    | Self::Failed
            ),
            Self::Recovering => matches!(next, Self::Postgres | Self::CatchingUp | Self::Failed),
            Self::Postgres | Self::Failed => false,
        }
    }
}

/// Contract value for durable migration status.
///
/// pg01 validates identity, state, timestamp, and capacity invariants only. A later
/// metadata-store and worker implementation must persist and recover this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationStatus {
    pub(crate) migration_id: String,
    pub(crate) installation_id: String,
    pub(crate) module: StorageModule,
    pub(crate) state: MigrationState,
    pub(crate) source_identity: String,
    pub(crate) target_identity: String,
    pub(crate) authority_generation: u64,
    pub(crate) capacity_bytes: u64,
    pub(crate) held_bytes: u64,
    pub(crate) hold_reason: Option<String>,
    pub(crate) updated_at_epoch_ms: i64,
}

impl MigrationStatus {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.migration_id.trim().is_empty() {
            bail!("migration status requires a migration identity");
        }
        if self.installation_id.trim().is_empty() {
            bail!("migration status requires an installation identity");
        }
        if self.source_identity.trim().is_empty() {
            bail!("migration status requires a source identity");
        }
        if self.target_identity.trim().is_empty() {
            bail!("migration status requires a target identity");
        }
        if self.authority_generation == 0 {
            bail!("migration status requires a non-zero authority generation");
        }
        if self.held_bytes > self.capacity_bytes {
            bail!("migration hold exceeds its declared capacity");
        }
        if self.updated_at_epoch_ms < 0 {
            bail!("migration status timestamp must not be negative");
        }
        if matches!(self.state, MigrationState::Paused | MigrationState::Blocked)
            && self
                .hold_reason
                .as_deref()
                .is_none_or(|reason| reason.trim().is_empty())
        {
            bail!("paused or blocked migration status requires a hold reason");
        }
        Ok(())
    }

    pub(crate) fn transition_to(
        &mut self,
        next: MigrationState,
        updated_at_epoch_ms: i64,
    ) -> Result<()> {
        if !self.state.can_transition_to(next) {
            bail!(
                "invalid migration state transition: {:?} -> {:?}",
                self.state,
                next
            );
        }
        if updated_at_epoch_ms < self.updated_at_epoch_ms {
            bail!("migration status timestamps must be monotonic");
        }
        let mut candidate = self.clone();
        candidate.state = next;
        candidate.updated_at_epoch_ms = updated_at_epoch_ms;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}

/// Contract value for durable per-table migration progress.
///
/// pg01 validates watermark ordering only; checkpoint persistence and restart recovery
/// belong to the later migration engine and runtime adapter.
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

impl MigrationCheckpoint {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.migration_id.trim().is_empty()
            || self.table_name.trim().is_empty()
            || self.stable_key.trim().is_empty()
        {
            bail!("migration checkpoint requires identity, table and stable key");
        }
        if self.source_identity.trim().is_empty() {
            bail!("migration checkpoint requires a source identity");
        }
        if self.authority_generation == 0 {
            bail!("migration checkpoint requires a non-zero authority generation");
        }
        if let (Some(captured), Some(applied)) = (self.captured_through, self.applied_through)
            && applied > captured
        {
            bail!("migration checkpoint cannot apply past its captured watermark");
        }
        Ok(())
    }
}

/// Contract value for a durable source-change delivery obligation.
///
/// pg01 defines the identity and acknowledgement invariants without implementing an
/// outbox worker or its durable storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationOutboxItem {
    pub(crate) outbox_id: String,
    pub(crate) migration_id: String,
    pub(crate) source_identity: String,
    pub(crate) source_version: i64,
    pub(crate) module: StorageModule,
    pub(crate) authority_generation: u64,
    pub(crate) payload_digest: String,
}

impl MigrationOutboxItem {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.outbox_id.trim().is_empty()
            || self.migration_id.trim().is_empty()
            || self.source_identity.trim().is_empty()
            || self.payload_digest.trim().is_empty()
        {
            bail!("migration outbox item requires stable identity and payload digest");
        }
        if self.source_version < 0 {
            bail!("migration outbox source version must not be negative");
        }
        if self.authority_generation == 0 {
            bail!("migration outbox item requires a non-zero authority generation");
        }
        Ok(())
    }
}

/// Contract value for bounded migration capacity reservation.
///
/// pg01 validates reservation identity and bounds without implementing a persistent
/// reservation store or capacity worker.
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

impl MigrationReservation {
    pub(crate) fn validate(&self) -> Result<()> {
        if self.reservation_id.trim().is_empty()
            || self.migration_id.trim().is_empty()
            || self.source_identity.trim().is_empty()
            || self.first_stable_id.trim().is_empty()
            || self.last_stable_id.trim().is_empty()
        {
            bail!("migration reservation requires stable identity and range");
        }
        if self.authority_generation == 0 {
            bail!("migration reservation requires a non-zero authority generation");
        }
        if self.reserved_bytes == 0 {
            bail!("migration reservation must reserve at least one byte");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdmissionDecision {
    Allow,
    Hold { reason: &'static str },
}

/// Migration admission overlay contract.
///
/// Implementations must hold unnecessary work for the active module while allowing
/// explicitly preserved authentication, routing, and recovery tasks to continue.
pub(crate) trait MigrationTaskAdmission: Send + Sync {
    fn decision(&self, task_key: &str) -> AdmissionDecision;

    fn decision_for_module(&self, _module: StorageModule, task_key: &str) -> AdmissionDecision {
        self.decision(task_key)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MigrationTaskPolicy {
    active_module: Option<StorageModule>,
    held_task_keys: BTreeMap<String, ()>,
    preserved_task_keys: BTreeMap<String, ()>,
}

impl MigrationTaskPolicy {
    pub(crate) fn hold_module(&mut self, module: StorageModule) {
        self.active_module = Some(module);
    }

    pub(crate) fn release_module(&mut self) {
        self.active_module = None;
    }

    pub(crate) fn hold_task(&mut self, task_key: impl Into<String>) {
        self.held_task_keys.insert(task_key.into(), ());
    }

    pub(crate) fn release_task(&mut self, task_key: &str) {
        self.held_task_keys.remove(task_key);
    }

    pub(crate) fn preserve_task_during_module(&mut self, task_key: impl Into<String>) {
        self.preserved_task_keys.insert(task_key.into(), ());
    }

    pub(crate) fn release_preserved_task(&mut self, task_key: &str) {
        self.preserved_task_keys.remove(task_key);
    }

    pub(crate) fn active_module(&self) -> Option<StorageModule> {
        self.active_module
    }
}

impl MigrationTaskAdmission for MigrationTaskPolicy {
    fn decision(&self, task_key: &str) -> AdmissionDecision {
        if self.held_task_keys.contains_key(task_key) || self.held_task_keys.contains_key("*") {
            AdmissionDecision::Hold {
                reason: "migration_hold",
            }
        } else {
            AdmissionDecision::Allow
        }
    }

    fn decision_for_module(&self, module: StorageModule, task_key: &str) -> AdmissionDecision {
        let decision = self.decision(task_key);
        if !matches!(decision, AdmissionDecision::Allow) {
            return decision;
        }
        if self.active_module == Some(module) && !self.preserved_task_keys.contains_key(task_key) {
            AdmissionDecision::Hold {
                reason: "migration_hold",
            }
        } else {
            AdmissionDecision::Allow
        }
    }
}

pub(crate) fn combine_admission_decisions(
    operator: AdmissionDecision,
    migration: AdmissionDecision,
) -> AdmissionDecision {
    match (operator, migration) {
        (AdmissionDecision::Hold { reason }, _) | (_, AdmissionDecision::Hold { reason }) => {
            AdmissionDecision::Hold { reason }
        }
        (AdmissionDecision::Allow, AdmissionDecision::Allow) => AdmissionDecision::Allow,
    }
}

/// Authority persistence must compare migration, installation, module, source, target and
/// watermark identities before publishing and atomically replace the matching no-cutover marker
/// with the manifest. `NotCommitted` is valid
/// only when the store confirms the source is still selected; changed durable state returns
/// `Conflict`, keeping admission closed for recovery. This fences stale writers and prevents two
/// migration attempts from publishing over one another.
#[async_trait::async_trait]
pub(crate) trait AuthorityManifestStore: Send + Sync {
    async fn load_authority_manifest(
        &self,
        module: StorageModule,
    ) -> Result<AuthorityRecoveryRecord>;
    async fn publish_authority_manifest(
        &self,
        manifest: &ModuleAuthorityManifest,
        expected: &AuthorityRecoveryContext,
    ) -> Result<AuthorityPublication>;
}

/// Durable migration progress boundary.
///
/// pg01 specifies the load/save, outbox, and capacity obligations and their typed
/// values. Concrete persistence, worker execution, and startup recovery are later
/// migration-engine/runtime work; this trait is not a claim that those paths exist.
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
