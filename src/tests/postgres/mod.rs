use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, Notify};

use crate::storage::catalog::{CatalogError, ModuleCatalog, ModuleId};
use crate::storage::core::{
    AdmissionDecision, AuthorityManifestStore, AuthorityPublication, AuthorityRecoveryContext,
    AuthorityRecoveryRecord, BackendKind, MigrationState, MigrationStatus, ModuleAuthority,
    ModuleAuthorityManifest, ModuleGenerationGate, ModuleLease, StorageModule, StoragePools,
    validate_transaction_leases,
};

struct TestAuthorityStore {
    manifest: Mutex<Option<ModuleAuthorityManifest>>,
    recovery: Mutex<Option<AuthorityRecoveryRecord>>,
    publication: Mutex<AuthorityPublication>,
    hold_publication: AtomicBool,
    publication_started: Notify,
    publication_release: Notify,
}

impl Default for TestAuthorityStore {
    fn default() -> Self {
        Self {
            manifest: Mutex::new(None),
            recovery: Mutex::new(None),
            publication: Mutex::new(AuthorityPublication::Committed),
            hold_publication: AtomicBool::new(false),
            publication_started: Notify::new(),
            publication_release: Notify::new(),
        }
    }
}

#[async_trait::async_trait]
impl AuthorityManifestStore for TestAuthorityStore {
    async fn load_authority_manifest(
        &self,
        _module: StorageModule,
    ) -> Result<AuthorityRecoveryRecord> {
        if let Some(recovery) = self.recovery.lock().await.clone() {
            return Ok(recovery);
        }
        Ok(self.manifest.lock().await.as_ref().cloned().map_or(
            AuthorityRecoveryRecord::NeverMigrated {
                installation_id: "test-installation".into(),
                source_identity: "test-source".into(),
            },
            AuthorityRecoveryRecord::Manifest,
        ))
    }

    async fn publish_authority_manifest(
        &self,
        manifest: &ModuleAuthorityManifest,
    ) -> Result<AuthorityPublication> {
        if self.hold_publication.load(Ordering::Acquire) {
            self.publication_started.notify_one();
            self.publication_release.notified().await;
        }
        let publication = *self.publication.lock().await;
        if publication != AuthorityPublication::Committed {
            return Ok(publication);
        }
        *self.manifest.lock().await = Some(manifest.clone());
        Ok(publication)
    }
}

fn authority_manifest(generation: u64, backend: BackendKind) -> ModuleAuthorityManifest {
    ModuleAuthorityManifest {
        installation_id: "test-installation".into(),
        source_identity: "test-source".into(),
        module: StorageModule::TrafficArchive,
        authority: ModuleAuthority {
            backend,
            generation,
        },
        source_watermark: 42,
        target_identity: "test-target".into(),
    }
}

fn recovery_context() -> AuthorityRecoveryContext {
    AuthorityRecoveryContext {
        installation_id: "test-installation".into(),
        source_identity: "test-source".into(),
        target_identity: "test-target".into(),
        source_watermark: 42,
    }
}

async fn wait_for_admission_closed(gate: &ModuleGenerationGate) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            match gate.acquire(StorageModule::TrafficArchive).await {
                Ok(lease) => drop(lease),
                Err(_) => break,
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn current_sqlite_schema_has_closed_ownership_catalog() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::schema::ensure_schema(&pool).await.unwrap();
    crate::prompt_cache_conversations::ensure_prompt_cache_conversations_schema(&pool)
        .await
        .unwrap();
    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    assert!(!catalog.owners().is_empty());
    assert!(catalog.modules().contains(&ModuleId::TrafficArchive));
    assert!(catalog.modules().contains(&ModuleId::Analytics));
    assert!(catalog.modules().contains(&ModuleId::AccountRouting));
    assert!(
        catalog
            .schema_objects()
            .iter()
            .any(|object| object.kind == "trigger")
    );
}

#[tokio::test]
async fn sqlite_catalog_classifies_transaction_closure_and_retained_controls() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    for table in [
        "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)",
        "CREATE TABLE proxy_raw_payload_blobs (raw_path TEXT PRIMARY KEY)",
        "CREATE TABLE proxy_raw_payload_blob_links (raw_path TEXT NOT NULL REFERENCES proxy_raw_payload_blobs(raw_path))",
        "CREATE TABLE archive_batches (id INTEGER PRIMARY KEY)",
        "CREATE TABLE summary_source_change_journal (revision INTEGER PRIMARY KEY)",
        "CREATE TABLE invocation_rollup_hourly (bucket_start_epoch INTEGER PRIMARY KEY)",
        "CREATE TABLE pool_upstream_accounts (id INTEGER PRIMARY KEY)",
        "CREATE TABLE pool_oauth_login_sessions (id INTEGER PRIMARY KEY)",
        "CREATE TABLE forward_proxy_settings (id INTEGER PRIMARY KEY)",
        "CREATE TABLE system_task_runs (id INTEGER PRIMARY KEY)",
        "CREATE INDEX idx_invocation_id ON codex_invocations (id)",
        "CREATE TRIGGER inv_touch AFTER INSERT ON codex_invocations BEGIN UPDATE codex_invocations SET id = NEW.id WHERE id = NEW.id; END",
    ] {
        sqlx::query(table).execute(&pool).await.unwrap();
    }

    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    assert!(
        catalog
            .schema_objects()
            .iter()
            .any(|object| object.name == "idx_invocation_id")
    );
    assert!(
        catalog
            .schema_objects()
            .iter()
            .any(|object| object.name == "inv_touch")
    );
    assert!(
        catalog
            .schema_objects()
            .iter()
            .any(|object| object.name.starts_with("sqlite_autoindex_"))
    );
    assert!(catalog.foreign_keys().iter().any(|edge| {
        edge.table == "proxy_raw_payload_blob_links"
            && edge.referenced_table == "proxy_raw_payload_blobs"
    }));
    catalog
        .validate_transaction([
            "codex_invocations",
            "proxy_raw_payload_blob_links",
            "proxy_raw_payload_blobs",
        ])
        .unwrap();
    catalog
        .validate_transaction(["archive_batches", "summary_source_change_journal"])
        .unwrap();
    assert!(matches!(
        catalog.validate_transaction(["codex_invocations", "invocation_rollup_hourly"]),
        Err(CatalogError::CrossModuleTransaction { .. })
    ));

    let account = catalog
        .owners()
        .iter()
        .find(|owner| owner.table == "pool_upstream_accounts")
        .unwrap();
    let credential = catalog
        .owners()
        .iter()
        .find(|owner| owner.table == "pool_oauth_login_sessions")
        .unwrap();
    let routing_control = catalog
        .owners()
        .iter()
        .find(|owner| owner.table == "forward_proxy_settings")
        .unwrap();
    assert_eq!(account.module, ModuleId::AccountRouting);
    assert_eq!(credential.module, ModuleId::AccountRouting);
    assert_eq!(routing_control.module, ModuleId::AccountRouting);
    assert_eq!(catalog.modules().len(), 4);
}

#[tokio::test]
async fn sqlite_catalog_rejects_cross_module_foreign_keys() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE invocation_rollup_hourly (id INTEGER PRIMARY KEY REFERENCES codex_invocations(id))")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        ModuleCatalog::inspect_sqlite(&pool).await,
        Err(CatalogError::CrossModuleForeignKey { .. })
    ));
}

#[tokio::test]
async fn sqlite_catalog_rejects_unclassified_tables() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE unexpected_business_state (id INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        ModuleCatalog::inspect_sqlite(&pool).await,
        Err(CatalogError::UnknownOwnership(table)) if table == "unexpected_business_state"
    ));
}

#[test]
fn transaction_lease_rejects_mixed_module_backend_or_generation() {
    let sqlite = ModuleLease {
        module: StorageModule::TrafficArchive,
        authority: ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    };
    let postgres = ModuleLease {
        module: StorageModule::TrafficArchive,
        authority: ModuleAuthority {
            backend: BackendKind::Postgres,
            generation: 2,
        },
    };
    let analytics = ModuleLease {
        module: StorageModule::Analytics,
        ..sqlite
    };
    assert_eq!(validate_transaction_leases(&[sqlite]).unwrap(), sqlite);
    assert!(validate_transaction_leases(&[sqlite, postgres]).is_err());
    assert!(validate_transaction_leases(&[sqlite, analytics]).is_err());
}

#[tokio::test]
async fn generation_gate_drains_old_leases_before_publishing_authority() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let old = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    let switch_gate = gate.clone();
    let store = Arc::new(TestAuthorityStore::default());
    let switch = tokio::spawn(async move {
        switch_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                authority_manifest(2, BackendKind::Postgres),
                recovery_context(),
                std::time::Duration::from_secs(1),
                store,
            )
            .await
    });
    wait_for_admission_closed(&gate).await;
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
    drop(old);
    switch.await.unwrap().unwrap();
    let new = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(new.lease.authority.backend, BackendKind::Postgres);
    assert_eq!(new.lease.authority.generation, 2);
}

#[tokio::test]
async fn generation_gate_does_not_publish_after_recovery_becomes_required_during_drain() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let old = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    let store = Arc::new(TestAuthorityStore::default());
    let switch_gate = gate.clone();
    let switch_store = Arc::clone(&store);
    let switch = tokio::spawn(async move {
        switch_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                authority_manifest(2, BackendKind::Postgres),
                recovery_context(),
                std::time::Duration::from_secs(1),
                switch_store,
            )
            .await
    });

    wait_for_admission_closed(&gate).await;
    gate.require_recovery_for_test();
    drop(old);
    assert!(switch.await.unwrap().is_err());
    assert!(store.manifest.lock().await.is_none());
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn cancelled_transaction_completion_fences_authority_before_releasing_lease() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let lease = gate.acquire(StorageModule::TrafficArchive).await.unwrap();

    let completion = gate.pending_completion_for_test(lease);
    completion.abort();
    assert!(completion.await.is_err());
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn publication_deadline_keeps_admission_closed_until_durable_recovery() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let store = Arc::new(TestAuthorityStore::default());
    store.hold_publication.store(true, Ordering::Release);
    let metadata: Arc<dyn AuthorityManifestStore> = store.clone();
    let result = gate
        .switch_authority(
            StorageModule::TrafficArchive,
            authority_manifest(2, BackendKind::Postgres),
            recovery_context(),
            std::time::Duration::from_millis(250),
            metadata,
        )
        .await;
    assert!(result.is_err());
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        store.publication_started.notified(),
    )
    .await
    .unwrap();
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());

    let recovery_gate = gate.clone();
    let recovery_store = Arc::clone(&store);
    let mut recovery = tokio::spawn(async move {
        recovery_gate
            .recover_authority(recovery_store.as_ref(), &recovery_context())
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut recovery)
            .await
            .is_err()
    );

    store.publication_release.notify_one();
    recovery.await.unwrap().unwrap();
    let current = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(current.lease.authority.backend, BackendKind::Postgres);
    assert_eq!(current.lease.authority.generation, 2);
}

#[tokio::test]
async fn generation_gate_serializes_concurrent_authority_switches() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let old = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    let store = Arc::new(TestAuthorityStore::default());
    let first_gate = gate.clone();
    let first_store = Arc::clone(&store);
    let first = tokio::spawn(async move {
        first_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                authority_manifest(2, BackendKind::Postgres),
                recovery_context(),
                std::time::Duration::from_secs(1),
                first_store,
            )
            .await
    });
    wait_for_admission_closed(&gate).await;
    let second_gate = gate.clone();
    let second = tokio::spawn(async move {
        second_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                authority_manifest(3, BackendKind::Postgres),
                recovery_context(),
                std::time::Duration::from_secs(1),
                store,
            )
            .await
    });
    wait_for_admission_closed(&gate).await;
    drop(old);
    first.await.unwrap().unwrap();
    assert!(second.await.unwrap().is_err());

    let current = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(current.lease.authority.generation, 2);
}

#[tokio::test]
async fn cancelling_switch_waiter_does_not_cancel_authority_transition() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let old = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    let store = Arc::new(TestAuthorityStore::default());
    let switch_gate = gate.clone();
    let switch_store = Arc::clone(&store);
    let waiter = tokio::spawn(async move {
        switch_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                authority_manifest(2, BackendKind::Postgres),
                recovery_context(),
                std::time::Duration::from_secs(1),
                switch_store,
            )
            .await
    });
    wait_for_admission_closed(&gate).await;
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
    waiter.abort();
    drop(old);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if store.manifest.lock().await.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let current = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(current.lease.authority.generation, 2);
    assert_eq!(
        store
            .manifest
            .lock()
            .await
            .as_ref()
            .unwrap()
            .authority
            .generation,
        2
    );
}

#[tokio::test]
async fn generation_gate_recovers_authority_from_durable_manifest() {
    let store = Arc::new(TestAuthorityStore::default());
    *store.manifest.lock().await = Some(authority_manifest(4, BackendKind::Postgres));
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    gate.recover_authority(store.as_ref(), &recovery_context())
        .await
        .unwrap();
    let current = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(current.lease.authority.backend, BackendKind::Postgres);
    assert_eq!(current.lease.authority.generation, 4);
}

#[tokio::test]
async fn generation_gate_rejects_foreign_authority_manifest() {
    let store = Arc::new(TestAuthorityStore::default());
    let mut manifest = authority_manifest(4, BackendKind::Postgres);
    manifest.installation_id = "another-installation".into();
    *store.manifest.lock().await = Some(manifest);
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );

    assert!(
        gate.recover_authority(store.as_ref(), &recovery_context())
            .await
            .is_err()
    );
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn indeterminate_authority_publication_stays_closed_until_recovery() {
    let store = Arc::new(TestAuthorityStore::default());
    *store.publication.lock().await = AuthorityPublication::Indeterminate;
    let publisher: Arc<dyn AuthorityManifestStore> = store.clone();
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    assert!(
        gate.switch_authority(
            StorageModule::TrafficArchive,
            authority_manifest(2, BackendKind::Postgres),
            recovery_context(),
            std::time::Duration::from_secs(1),
            publisher,
        )
        .await
        .is_err()
    );
    *store.recovery.lock().await = Some(AuthorityRecoveryRecord::Indeterminate);
    assert!(
        gate.recover_authority(store.as_ref(), &recovery_context())
            .await
            .is_err()
    );
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn rejected_authority_publication_reopens_the_source_authority() {
    let store = Arc::new(TestAuthorityStore::default());
    *store.publication.lock().await = AuthorityPublication::NotCommitted;
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    assert!(
        gate.switch_authority(
            StorageModule::TrafficArchive,
            authority_manifest(2, BackendKind::Postgres),
            recovery_context(),
            std::time::Duration::from_secs(1),
            store,
        )
        .await
        .is_err()
    );
    let current = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(current.lease.authority.backend, BackendKind::Sqlite);
    assert_eq!(current.lease.authority.generation, 1);
}

#[tokio::test]
async fn authority_switch_rejects_unverified_identity_and_sqlite_failback() {
    let store = Arc::new(TestAuthorityStore::default());
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Postgres,
            generation: 2,
        },
    );
    let publisher: Arc<dyn AuthorityManifestStore> = store.clone();
    assert!(
        gate.switch_authority(
            StorageModule::TrafficArchive,
            authority_manifest(3, BackendKind::Sqlite),
            recovery_context(),
            std::time::Duration::from_secs(1),
            publisher,
        )
        .await
        .is_err()
    );
    assert!(store.manifest.lock().await.is_none());

    let mut expected = recovery_context();
    expected.target_identity = "different-target".into();
    assert!(
        gate.switch_authority(
            StorageModule::TrafficArchive,
            authority_manifest(3, BackendKind::Postgres),
            expected,
            std::time::Duration::from_secs(1),
            store.clone(),
        )
        .await
        .is_err()
    );
    assert!(store.manifest.lock().await.is_none());
}

#[tokio::test]
async fn storage_transaction_holds_generation_lease_until_commit() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let storage = StoragePools {
        sqlite: pool,
        postgres: None,
    };
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let transaction = storage
        .begin(gate.acquire(StorageModule::TrafficArchive).await.unwrap())
        .await
        .unwrap();
    let switch_gate = gate.clone();
    let store = Arc::new(TestAuthorityStore::default());
    let switch = tokio::spawn(async move {
        switch_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                authority_manifest(2, BackendKind::Postgres),
                recovery_context(),
                std::time::Duration::from_secs(1),
                store,
            )
            .await
    });
    wait_for_admission_closed(&gate).await;
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
    transaction.commit().await.unwrap();
    switch.await.unwrap().unwrap();
}

#[test]
fn migration_status_exposes_identity_generation_and_capacity_hold() {
    let status = MigrationStatus {
        migration_id: "migration-01".into(),
        module: StorageModule::TrafficArchive,
        state: MigrationState::CatchingUp,
        source_identity: "sqlite-main-installation-01".into(),
        authority_generation: 4,
        capacity_bytes: 4096,
        held_bytes: 2048,
        hold_reason: Some("capture_capacity".into()),
        updated_at_epoch_ms: 10,
    };
    assert_eq!(status.authority_generation, 4);
    assert!(status.held_bytes <= status.capacity_bytes);
    let admission = AdmissionDecision::Hold {
        reason: "migration_catchup",
    };
    assert_eq!(
        admission,
        AdmissionDecision::Hold {
            reason: "migration_catchup"
        }
    );
}
