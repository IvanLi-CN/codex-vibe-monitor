use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use anyhow::Result;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, Semaphore};

use crate::storage::catalog::{CatalogError, ModuleCatalog, ModuleId, RecoveryObligation};
use crate::storage::config::StorageConfig;
use crate::storage::core::{
    AccountRoutingTransaction, AdmissionDecision, AnalyticsTransaction, AuthorityManifestStore,
    AuthorityPublication, AuthorityRecoveryContext, AuthorityRecoveryRecord, BackendKind,
    MigrationCheckpoint, MigrationOutboxItem, MigrationReservation, MigrationState,
    MigrationStatus, MigrationTaskAdmission, MigrationTaskPolicy, ModuleAuthority,
    ModuleAuthorityManifest, ModuleGenerationGate, ModuleLease, PerformanceTransaction,
    StorageModule, StoragePools, TaskOperationsTransaction, TrafficArchiveTransaction,
    combine_admission_decisions, validate_transaction_leases,
};

struct TestAuthorityStore {
    compare_and_publish: Mutex<()>,
    manifest: Mutex<Option<ModuleAuthorityManifest>>,
    recovery: Mutex<Option<AuthorityRecoveryRecord>>,
    publication: Mutex<AuthorityPublication>,
    hold_publication: AtomicBool,
    publication_count: AtomicUsize,
    publication_started: Semaphore,
    publication_release: Semaphore,
}

impl Default for TestAuthorityStore {
    fn default() -> Self {
        Self {
            compare_and_publish: Mutex::new(()),
            manifest: Mutex::new(None),
            recovery: Mutex::new(None),
            publication: Mutex::new(AuthorityPublication::Committed),
            hold_publication: AtomicBool::new(false),
            publication_count: AtomicUsize::new(0),
            publication_started: Semaphore::new(0),
            publication_release: Semaphore::new(0),
        }
    }
}

#[async_trait::async_trait]
impl AuthorityManifestStore for TestAuthorityStore {
    async fn load_authority_manifest(
        &self,
        module: StorageModule,
    ) -> Result<AuthorityRecoveryRecord> {
        let _compare_and_publish = self.compare_and_publish.lock().await;
        if let Some(recovery) = self.recovery.lock().await.clone() {
            return Ok(recovery);
        }
        Ok(self.manifest.lock().await.as_ref().cloned().map_or(
            AuthorityRecoveryRecord::NeverMigrated {
                migration_id: "migration-01".into(),
                installation_id: "test-installation".into(),
                module,
                source_identity: "test-source".into(),
                target_identity: "test-target".into(),
                source_watermark: 42,
            },
            AuthorityRecoveryRecord::Manifest,
        ))
    }

    async fn publish_authority_manifest(
        &self,
        manifest: &ModuleAuthorityManifest,
        expected: &AuthorityRecoveryContext,
    ) -> Result<AuthorityPublication> {
        self.publication_count.fetch_add(1, Ordering::AcqRel);
        if self.hold_publication.load(Ordering::Acquire) {
            self.publication_started.add_permits(1);
            self.publication_release.acquire().await.unwrap().forget();
        }
        let _compare_and_publish = self.compare_and_publish.lock().await;
        let publication = *self.publication.lock().await;
        if publication != AuthorityPublication::Committed {
            return Ok(publication);
        }
        if manifest.migration_id != expected.migration_id
            || manifest.installation_id != expected.installation_id
            || manifest.module != expected.module
            || manifest.source_identity != expected.source_identity
            || manifest.target_identity != expected.target_identity
            || manifest.source_watermark != expected.source_watermark
            || manifest.authority.backend != BackendKind::Postgres
        {
            return Ok(AuthorityPublication::Conflict);
        }
        let mut recovery_slot = self.recovery.lock().await;
        let recovery = recovery_slot
            .clone()
            .unwrap_or(AuthorityRecoveryRecord::NeverMigrated {
                migration_id: "migration-01".into(),
                installation_id: "test-installation".into(),
                module: manifest.module,
                source_identity: "test-source".into(),
                target_identity: "test-target".into(),
                source_watermark: 42,
            });
        if !matches!(
            recovery,
            AuthorityRecoveryRecord::NeverMigrated {
                migration_id,
                installation_id,
                module,
                source_identity,
                target_identity,
                source_watermark,
            } if migration_id == expected.migration_id
                && installation_id == expected.installation_id
                && module == expected.module
                && source_identity == expected.source_identity
                && target_identity == expected.target_identity
                && source_watermark == expected.source_watermark
        ) {
            return Ok(AuthorityPublication::Conflict);
        }
        if self.manifest.lock().await.is_some() {
            return Ok(AuthorityPublication::Conflict);
        }
        *self.manifest.lock().await = Some(manifest.clone());
        *recovery_slot = None;
        Ok(publication)
    }
}

fn authority_manifest(generation: u64, backend: BackendKind) -> ModuleAuthorityManifest {
    authority_manifest_for_migration(generation, backend, "migration-01")
}

fn authority_manifest_for_migration(
    generation: u64,
    backend: BackendKind,
    migration_id: &str,
) -> ModuleAuthorityManifest {
    ModuleAuthorityManifest {
        migration_id: migration_id.into(),
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
    recovery_context_for_migration("migration-01")
}

fn recovery_context_for_migration(migration_id: &str) -> AuthorityRecoveryContext {
    AuthorityRecoveryContext {
        migration_id: migration_id.into(),
        installation_id: "test-installation".into(),
        module: StorageModule::TrafficArchive,
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
    let table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(catalog.owners().len(), table_count as usize);
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
    assert!(!catalog.trigger_writes().is_empty());
    let cross_module_triggers = catalog.cross_module_trigger_writes();
    assert!(cross_module_triggers.iter().any(|relation| {
        relation.source_table == "codex_invocations"
            && relation.target_table == "timeseries_minute_projection_v2_recovery"
    }));
    assert!(cross_module_triggers.iter().any(|relation| {
        relation.source_table == "archive_batches"
            && relation.target_table == "long_term_projection_dirty_buckets"
    }));
    assert!(
        catalog
            .owners()
            .iter()
            .all(|owner| !owner.transaction_group.is_empty())
    );
    assert_eq!(
        catalog
            .owners()
            .iter()
            .find(|owner| owner.table == "codex_invocations")
            .unwrap()
            .recovery_obligation,
        RecoveryObligation::TrafficFact
    );
    assert_eq!(
        catalog
            .owners()
            .iter()
            .find(|owner| owner.table == "summary_source_change_journal")
            .unwrap()
            .recovery_obligation,
        RecoveryObligation::TrafficRecovery
    );
    assert_eq!(
        catalog
            .owners()
            .iter()
            .find(|owner| owner.table == "invocation_rollup_hourly")
            .unwrap()
            .recovery_obligation,
        RecoveryObligation::AnalyticsProjection
    );
    assert_eq!(
        catalog
            .owners()
            .iter()
            .find(|owner| owner.table == "pool_upstream_accounts")
            .unwrap()
            .recovery_obligation,
        RecoveryObligation::AccountControl
    );
    let account_selection_outbox = catalog
        .owners()
        .iter()
        .find(|owner| owner.table == "pool_upstream_account_selected_touch_outbox")
        .unwrap();
    assert_eq!(account_selection_outbox.module, ModuleId::AccountRouting);
    assert_eq!(
        account_selection_outbox.transaction_group,
        "account_control"
    );
    assert_eq!(
        account_selection_outbox.recovery_obligation,
        RecoveryObligation::AccountControl
    );
    assert!(catalog.schema_objects().iter().all(|object| {
        catalog
            .owners()
            .iter()
            .any(|owner| owner.table == object.table)
    }));
    assert!(catalog.foreign_keys().iter().all(|edge| {
        catalog
            .owners()
            .iter()
            .any(|owner| owner.table == edge.table)
            && catalog
                .owners()
                .iter()
                .any(|owner| owner.table == edge.referenced_table)
    }));
    assert!(catalog.trigger_writes().iter().all(|relation| {
        catalog
            .owners()
            .iter()
            .any(|owner| owner.table == relation.source_table)
            && catalog
                .owners()
                .iter()
                .any(|owner| owner.table == relation.target_table)
    }));
    assert!(matches!(
        catalog.validate_transaction(["codex_invocations"]),
        Err(CatalogError::CrossModuleTransaction { .. })
    ));

    let maintenance_store = crate::maintenance_store::ownership_test_store().await;
    let maintenance_catalog = ModuleCatalog::inspect_sqlite(&maintenance_store.pool)
        .await
        .unwrap();
    let maintenance_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_one(&maintenance_store.pool)
    .await
    .unwrap();
    assert_eq!(
        maintenance_catalog.owners().len(),
        maintenance_table_count as usize
    );
    assert!(
        maintenance_catalog
            .modules()
            .contains(&ModuleId::TaskOperations)
    );
    assert!(maintenance_catalog.modules().contains(&ModuleId::Analytics));
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
        .validate_transaction(["proxy_raw_payload_blob_links"])
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
async fn switch_rejects_manifest_from_another_migration_attempt() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let store = Arc::new(TestAuthorityStore::default());
    let result = gate
        .switch_authority(
            StorageModule::TrafficArchive,
            authority_manifest_for_migration(2, BackendKind::Postgres, "migration-stale"),
            recovery_context_for_migration("migration-current"),
            std::time::Duration::from_secs(1),
            store.clone(),
        )
        .await;

    assert!(result.is_err());
    assert_eq!(store.publication_count.load(Ordering::Acquire), 0);
    let current = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(current.lease.authority.generation, 1);
}

#[tokio::test]
async fn recovery_rejects_authority_manifest_from_another_migration_attempt() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let store = Arc::new(TestAuthorityStore::default());
    *store.manifest.lock().await = Some(authority_manifest_for_migration(
        2,
        BackendKind::Postgres,
        "migration-stale",
    ));

    assert!(
        gate.recover_authority(
            store.as_ref(),
            &recovery_context_for_migration("migration-current")
        )
        .await
        .is_err()
    );
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn authority_switch_and_recovery_reject_another_source_identity() {
    let switch_gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let switch_store = Arc::new(TestAuthorityStore::default());
    let mut stale_manifest = authority_manifest(2, BackendKind::Postgres);
    stale_manifest.source_identity = "another-source".into();
    assert!(
        switch_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                stale_manifest.clone(),
                recovery_context(),
                std::time::Duration::from_secs(1),
                switch_store.clone(),
            )
            .await
            .is_err()
    );
    assert_eq!(switch_store.publication_count.load(Ordering::Acquire), 0);
    let source_lease = switch_gate
        .acquire(StorageModule::TrafficArchive)
        .await
        .unwrap();
    assert_eq!(source_lease.lease.authority.backend, BackendKind::Sqlite);

    let recovery_gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let recovery_store = TestAuthorityStore::default();
    *recovery_store.manifest.lock().await = Some(stale_manifest);
    assert!(
        recovery_gate
            .recover_authority(&recovery_store, &recovery_context())
            .await
            .is_err()
    );
    assert!(
        recovery_gate
            .acquire(StorageModule::TrafficArchive)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn recovery_rejects_no_cutover_marker_from_another_migration_attempt() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let store = TestAuthorityStore::default();
    *store.recovery.lock().await = Some(AuthorityRecoveryRecord::NeverMigrated {
        migration_id: "migration-stale".into(),
        installation_id: "test-installation".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "test-source".into(),
        target_identity: "test-target".into(),
        source_watermark: 42,
    });

    assert!(
        gate.recover_authority(&store, &recovery_context_for_migration("migration-current"))
            .await
            .is_err()
    );
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn recovery_rejects_no_cutover_marker_for_another_module() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let store = TestAuthorityStore::default();
    *store.recovery.lock().await = Some(AuthorityRecoveryRecord::NeverMigrated {
        migration_id: "migration-01".into(),
        installation_id: "test-installation".into(),
        module: StorageModule::Analytics,
        source_identity: "test-source".into(),
        target_identity: "test-target".into(),
        source_watermark: 42,
    });

    assert!(
        gate.recover_authority(&store, &recovery_context())
            .await
            .is_err()
    );
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn recovery_rejects_no_cutover_marker_with_another_full_context() {
    let expected = recovery_context();
    let mut markers = Vec::new();

    let mut marker = AuthorityRecoveryRecord::NeverMigrated {
        migration_id: expected.migration_id.clone(),
        installation_id: expected.installation_id.clone(),
        module: expected.module,
        source_identity: expected.source_identity.clone(),
        target_identity: expected.target_identity.clone(),
        source_watermark: expected.source_watermark,
    };
    if let AuthorityRecoveryRecord::NeverMigrated {
        installation_id, ..
    } = &mut marker
    {
        *installation_id = "another-installation".into();
    }
    markers.push(marker);

    let mut marker = AuthorityRecoveryRecord::NeverMigrated {
        migration_id: expected.migration_id.clone(),
        installation_id: expected.installation_id.clone(),
        module: expected.module,
        source_identity: expected.source_identity.clone(),
        target_identity: expected.target_identity.clone(),
        source_watermark: expected.source_watermark,
    };
    if let AuthorityRecoveryRecord::NeverMigrated {
        source_identity, ..
    } = &mut marker
    {
        *source_identity = "another-source".into();
    }
    markers.push(marker);

    let mut marker = AuthorityRecoveryRecord::NeverMigrated {
        migration_id: expected.migration_id.clone(),
        installation_id: expected.installation_id.clone(),
        module: expected.module,
        source_identity: expected.source_identity.clone(),
        target_identity: expected.target_identity.clone(),
        source_watermark: expected.source_watermark,
    };
    if let AuthorityRecoveryRecord::NeverMigrated {
        target_identity, ..
    } = &mut marker
    {
        *target_identity = "another-target".into();
    }
    markers.push(marker);

    let mut marker = AuthorityRecoveryRecord::NeverMigrated {
        migration_id: expected.migration_id.clone(),
        installation_id: expected.installation_id.clone(),
        module: expected.module,
        source_identity: expected.source_identity.clone(),
        target_identity: expected.target_identity.clone(),
        source_watermark: expected.source_watermark,
    };
    if let AuthorityRecoveryRecord::NeverMigrated {
        source_watermark, ..
    } = &mut marker
    {
        *source_watermark += 1;
    }
    markers.push(marker);

    for marker in markers {
        let gate = ModuleGenerationGate::new(
            StorageModule::TrafficArchive,
            ModuleAuthority {
                backend: BackendKind::Sqlite,
                generation: 1,
            },
        );
        let store = TestAuthorityStore::default();
        *store.recovery.lock().await = Some(marker);

        assert!(gate.recover_authority(&store, &expected).await.is_err());
        assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
    }
}

#[tokio::test]
async fn recovery_rejects_context_for_another_module() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let store = TestAuthorityStore::default();
    *store.recovery.lock().await = Some(AuthorityRecoveryRecord::NeverMigrated {
        migration_id: "migration-01".into(),
        installation_id: "test-installation".into(),
        module: StorageModule::Analytics,
        source_identity: "test-source".into(),
        target_identity: "test-target".into(),
        source_watermark: 42,
    });
    let mut expected = recovery_context();
    expected.module = StorageModule::Analytics;

    assert!(gate.recover_authority(&store, &expected).await.is_err());
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn recovery_rejects_manifest_for_another_module() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let store = TestAuthorityStore::default();
    let mut manifest = authority_manifest(2, BackendKind::Postgres);
    manifest.module = StorageModule::Analytics;
    *store.manifest.lock().await = Some(manifest);

    assert!(
        gate.recover_authority(&store, &recovery_context())
            .await
            .is_err()
    );
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn recovery_rejects_manifest_for_another_target_or_watermark() {
    let expected = recovery_context();
    let mut manifests = Vec::new();

    let mut manifest = authority_manifest(2, BackendKind::Postgres);
    manifest.target_identity = "another-target".into();
    manifests.push(manifest);

    let mut manifest = authority_manifest(2, BackendKind::Postgres);
    manifest.source_watermark += 1;
    manifests.push(manifest);

    for manifest in manifests {
        let gate = ModuleGenerationGate::new(
            StorageModule::TrafficArchive,
            ModuleAuthority {
                backend: BackendKind::Sqlite,
                generation: 1,
            },
        );
        let store = TestAuthorityStore::default();
        *store.manifest.lock().await = Some(manifest);

        assert!(gate.recover_authority(&store, &expected).await.is_err());
        assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
    }
}

#[tokio::test]
async fn durable_publication_compare_and_replace_fences_concurrent_attempts() {
    let store = Arc::new(TestAuthorityStore::default());
    store.hold_publication.store(true, Ordering::Release);
    let first_context = recovery_context_for_migration("migration-01");
    let second_context = recovery_context_for_migration("migration-02");
    let first_manifest = authority_manifest_for_migration(2, BackendKind::Postgres, "migration-01");
    let second_manifest =
        authority_manifest_for_migration(3, BackendKind::Postgres, "migration-02");
    let expected_first_manifest = first_manifest.clone();
    let expected_second_manifest = second_manifest.clone();

    let first_store = Arc::clone(&store);
    let first = tokio::spawn(async move {
        first_store
            .publish_authority_manifest(&first_manifest, &first_context)
            .await
    });
    let second_store = Arc::clone(&store);
    let second = tokio::spawn(async move {
        second_store
            .publish_authority_manifest(&second_manifest, &second_context)
            .await
    });
    for _ in 0..2 {
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            store.publication_started.acquire(),
        )
        .await
        .unwrap()
        .unwrap()
        .forget();
    }
    store.publication_release.add_permits(2);

    let first_outcome = first.await.unwrap().unwrap();
    let second_outcome = second.await.unwrap().unwrap();
    let committed_manifest = match (first_outcome, second_outcome) {
        (AuthorityPublication::Committed, AuthorityPublication::Conflict) => {
            expected_first_manifest
        }
        (AuthorityPublication::Conflict, AuthorityPublication::Committed) => {
            expected_second_manifest
        }
        outcomes => panic!("expected one committed publication, got {outcomes:?}"),
    };
    assert_eq!(
        store.manifest.lock().await.as_ref(),
        Some(&committed_manifest)
    );
    assert!(store.recovery.lock().await.is_none());
}

#[tokio::test]
async fn publication_compare_rejects_another_target_or_watermark() {
    let store = TestAuthorityStore::default();
    let expected = recovery_context();
    let manifest = authority_manifest(2, BackendKind::Postgres);
    let mut marker = AuthorityRecoveryRecord::NeverMigrated {
        migration_id: expected.migration_id.clone(),
        installation_id: expected.installation_id.clone(),
        module: expected.module,
        source_identity: expected.source_identity.clone(),
        target_identity: "another-target".into(),
        source_watermark: expected.source_watermark,
    };

    *store.recovery.lock().await = Some(marker.clone());
    assert_eq!(
        store
            .publish_authority_manifest(&manifest, &expected)
            .await
            .unwrap(),
        AuthorityPublication::Conflict
    );

    if let AuthorityRecoveryRecord::NeverMigrated {
        target_identity,
        source_watermark,
        ..
    } = &mut marker
    {
        *target_identity = expected.target_identity.clone();
        *source_watermark = expected.source_watermark + 1;
    }
    *store.recovery.lock().await = Some(marker);
    assert_eq!(
        store
            .publish_authority_manifest(&manifest, &expected)
            .await
            .unwrap(),
        AuthorityPublication::Conflict
    );
    assert!(store.manifest.lock().await.is_none());
}

#[tokio::test]
async fn publication_compare_rejects_manifest_with_another_context() {
    let expected = recovery_context();
    let mut manifests = Vec::new();

    let mut manifest = authority_manifest(2, BackendKind::Postgres);
    manifest.migration_id = "another-migration".into();
    manifests.push(manifest);

    let mut manifest = authority_manifest(2, BackendKind::Postgres);
    manifest.installation_id = "another-installation".into();
    manifests.push(manifest);

    let mut manifest = authority_manifest(2, BackendKind::Postgres);
    manifest.source_identity = "another-source".into();
    manifests.push(manifest);

    let mut manifest = authority_manifest(2, BackendKind::Postgres);
    manifest.target_identity = "another-target".into();
    manifests.push(manifest);

    let mut manifest = authority_manifest(2, BackendKind::Postgres);
    manifest.source_watermark += 1;
    manifests.push(manifest);

    let mut manifest = authority_manifest(2, BackendKind::Postgres);
    manifest.module = StorageModule::Analytics;
    manifests.push(manifest);

    for manifest in manifests {
        let store = TestAuthorityStore::default();
        assert_eq!(
            store
                .publish_authority_manifest(&manifest, &expected)
                .await
                .unwrap(),
            AuthorityPublication::Conflict
        );
        assert!(store.manifest.lock().await.is_none());
    }
}

#[tokio::test]
async fn publication_replaces_matching_no_cutover_marker_with_manifest() {
    let expected = recovery_context();
    let manifest = authority_manifest(2, BackendKind::Postgres);
    let store = TestAuthorityStore::default();
    *store.recovery.lock().await = Some(AuthorityRecoveryRecord::NeverMigrated {
        migration_id: expected.migration_id.clone(),
        installation_id: expected.installation_id.clone(),
        module: expected.module,
        source_identity: expected.source_identity.clone(),
        target_identity: expected.target_identity.clone(),
        source_watermark: expected.source_watermark,
    });

    assert_eq!(
        store
            .publish_authority_manifest(&manifest, &expected)
            .await
            .unwrap(),
        AuthorityPublication::Committed
    );
    assert_eq!(store.manifest.lock().await.as_ref(), Some(&manifest));
    assert!(store.recovery.lock().await.is_none());
}

#[tokio::test]
async fn publication_rejects_a_stale_durable_recovery_context() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let store = Arc::new(TestAuthorityStore::default());
    *store.recovery.lock().await = Some(AuthorityRecoveryRecord::NeverMigrated {
        migration_id: "migration-stale".into(),
        installation_id: "test-installation".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "test-source".into(),
        target_identity: "test-target".into(),
        source_watermark: 42,
    });

    assert!(
        gate.switch_authority(
            StorageModule::TrafficArchive,
            authority_manifest(2, BackendKind::Postgres),
            recovery_context(),
            std::time::Duration::from_secs(1),
            store.clone(),
        )
        .await
        .is_err()
    );
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn publication_conflict_keeps_gate_closed_after_foreign_manifest() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let store = Arc::new(TestAuthorityStore::default());
    *store.manifest.lock().await = Some(authority_manifest_for_migration(
        2,
        BackendKind::Postgres,
        "migration-committed",
    ));

    assert!(
        gate.switch_authority(
            StorageModule::TrafficArchive,
            authority_manifest(2, BackendKind::Postgres),
            recovery_context(),
            std::time::Duration::from_secs(1),
            store.clone(),
        )
        .await
        .is_err()
    );
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
    assert!(
        gate.recover_authority(store.as_ref(), &recovery_context())
            .await
            .is_err()
    );
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
}

#[tokio::test]
async fn unpolled_transaction_completion_cancel_fences_authority_before_releasing_lease() {
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
async fn recovery_fence_wins_race_with_lease_admission() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    gate.pause_acquire_after_check_for_test();
    let acquiring_gate = gate.clone();
    let acquire =
        tokio::spawn(async move { acquiring_gate.acquire(StorageModule::TrafficArchive).await });

    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        gate.wait_for_acquire_check_for_test(),
    )
    .await
    .unwrap();
    gate.require_recovery_for_test();
    gate.release_acquire_check_for_test();

    assert!(acquire.await.unwrap().is_err());
    assert_eq!(gate.active_leases_for_test(), 0);
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
        store.publication_started.acquire(),
    )
    .await
    .unwrap()
    .unwrap()
    .forget();
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

    store.publication_release.add_permits(1);
    recovery.await.unwrap().unwrap();
    let current = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(current.lease.authority.backend, BackendKind::Postgres);
    assert_eq!(current.lease.authority.generation, 2);
}

#[tokio::test]
async fn cancelled_unpolled_publication_releases_recovery_waiter() {
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let publication = gate.pending_publication_for_test();
    publication.abort();
    assert!(publication.await.is_err());

    let store = TestAuthorityStore::default();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        gate.recover_authority(&store, &recovery_context()),
    )
    .await
    .unwrap()
    .unwrap();
    let current = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(current.lease.authority.backend, BackendKind::Sqlite);
    assert_eq!(current.lease.authority.generation, 1);
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
    store.hold_publication.store(true, Ordering::Release);
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
    drop(old);
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        store.publication_started.acquire(),
    )
    .await
    .unwrap()
    .unwrap()
    .forget();
    let second_attempt = gate.switch_attempts_for_test() + 1;
    let second_gate = gate.clone();
    let second_store = Arc::clone(&store);
    let second = tokio::spawn(async move {
        second_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                authority_manifest_for_migration(3, BackendKind::Postgres, "migration-02"),
                recovery_context_for_migration("migration-02"),
                std::time::Duration::from_secs(1),
                second_store,
            )
            .await
    });
    gate.wait_for_switch_attempts_for_test(second_attempt).await;
    assert_eq!(store.publication_count.load(Ordering::Acquire), 1);
    store.publication_release.add_permits(1);
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
    let storage = StoragePools::sqlite_only(pool);
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
    let mut status = MigrationStatus {
        migration_id: "migration-01".into(),
        installation_id: "installation-01".into(),
        module: StorageModule::TrafficArchive,
        state: MigrationState::CatchingUp,
        source_identity: "sqlite-main-installation-01".into(),
        target_identity: "postgres-cluster-01".into(),
        authority_generation: 4,
        capacity_bytes: 4096,
        held_bytes: 2048,
        hold_reason: Some("capture_capacity".into()),
        updated_at_epoch_ms: 10,
    };
    status.validate().unwrap();
    assert_eq!(status.authority_generation, 4);
    assert!(status.held_bytes <= status.capacity_bytes);
    assert!(!status.state.is_terminal());
    status.transition_to(MigrationState::Verifying, 11).unwrap();
    assert_eq!(status.state, MigrationState::Verifying);
    status.hold_reason = None;
    assert!(status.transition_to(MigrationState::Paused, 12).is_err());
    assert_eq!(status.state, MigrationState::Verifying);
    assert_eq!(status.updated_at_epoch_ms, 11);
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

#[test]
fn migration_progress_contract_covers_checkpoint_outbox_and_capacity_obligations() {
    MigrationCheckpoint {
        migration_id: "migration-01".into(),
        table_name: "codex_invocations".into(),
        stable_key: "42".into(),
        cursor: Some("42".into()),
        captured_through: Some(10),
        applied_through: Some(10),
        source_identity: "sqlite-main-installation-01".into(),
        authority_generation: 1,
    }
    .validate()
    .unwrap();

    MigrationOutboxItem {
        outbox_id: "outbox-01".into(),
        migration_id: "migration-01".into(),
        source_identity: "sqlite-main-installation-01".into(),
        source_version: 10,
        module: StorageModule::TrafficArchive,
        authority_generation: 1,
        payload_digest: "digest-01".into(),
    }
    .validate()
    .unwrap();

    MigrationReservation {
        reservation_id: "reservation-01".into(),
        migration_id: "migration-01".into(),
        source_identity: "sqlite-main-installation-01".into(),
        module: StorageModule::TrafficArchive,
        authority_generation: 1,
        first_stable_id: "1".into(),
        last_stable_id: "42".into(),
        reserved_bytes: 4096,
    }
    .validate()
    .unwrap();
}

#[tokio::test]
async fn typed_storage_transactions_route_all_modules_and_hold_leases_until_completion() {
    let storage = StoragePools::sqlite_only(SqlitePool::connect("sqlite::memory:").await.unwrap());
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let wrong_gate = ModuleGenerationGate::new(
        StorageModule::Analytics,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    assert!(
        storage
            .begin_typed::<TrafficArchiveTransaction>(
                wrong_gate.acquire(StorageModule::Analytics).await.unwrap()
            )
            .await
            .is_err()
    );

    let transaction = storage
        .begin_typed::<TrafficArchiveTransaction>(
            gate.acquire(StorageModule::TrafficArchive).await.unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(transaction.module(), StorageModule::TrafficArchive);
    let mut transaction = transaction;
    assert!(transaction.sqlite_transaction().is_ok());
    assert!(transaction.postgres_transaction().is_err());
    assert_eq!(gate.active_leases_for_test(), 1);
    transaction.commit().await.unwrap();
    assert_eq!(gate.active_leases_for_test(), 0);

    macro_rules! assert_sqlite_marker {
        ($marker:ty, $module:expr) => {{
            let gate = ModuleGenerationGate::new(
                $module,
                ModuleAuthority {
                    backend: BackendKind::Sqlite,
                    generation: 1,
                },
            );
            let transaction = storage
                .begin_typed::<$marker>(gate.acquire($module).await.unwrap())
                .await
                .unwrap();
            assert_eq!(transaction.module(), $module);
            assert_eq!(gate.active_leases_for_test(), 1);
            transaction.rollback().await.unwrap();
            assert_eq!(gate.active_leases_for_test(), 0);
        }};
    }

    assert_sqlite_marker!(AnalyticsTransaction, StorageModule::Analytics);
    assert_sqlite_marker!(AccountRoutingTransaction, StorageModule::AccountRouting);
    assert_sqlite_marker!(TaskOperationsTransaction, StorageModule::TaskOperations);
    assert_sqlite_marker!(PerformanceTransaction, StorageModule::Performance);

    let postgres_gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Postgres,
            generation: 1,
        },
    );
    assert!(
        storage
            .begin_typed::<TrafficArchiveTransaction>(
                postgres_gate
                    .acquire(StorageModule::TrafficArchive)
                    .await
                    .unwrap(),
            )
            .await
            .is_err()
    );
    assert_eq!(postgres_gate.active_leases_for_test(), 0);
}

#[test]
fn migration_task_policy_composes_with_operator_admission() {
    let mut policy = MigrationTaskPolicy::default();
    policy.hold_module(StorageModule::TrafficArchive);
    policy.preserve_task_during_module("routing");
    policy.preserve_task_during_module("oauth_renewal");
    assert_eq!(
        policy.decision_for_module(StorageModule::TrafficArchive, "archive"),
        AdmissionDecision::Hold {
            reason: "migration_hold",
        }
    );
    assert_eq!(
        policy.decision_for_module(StorageModule::TrafficArchive, "routing"),
        AdmissionDecision::Allow
    );
    assert_eq!(
        policy.decision_for_module(StorageModule::TrafficArchive, "oauth_renewal"),
        AdmissionDecision::Allow
    );
    policy.hold_task("routing");
    assert_eq!(
        policy.decision_for_module(StorageModule::TrafficArchive, "routing"),
        AdmissionDecision::Hold {
            reason: "migration_hold",
        }
    );
    policy.release_task("routing");
    assert_eq!(
        policy.decision_for_module(StorageModule::TrafficArchive, "routing"),
        AdmissionDecision::Allow
    );
    assert_eq!(
        policy.decision_for_module(StorageModule::AccountRouting, "routing"),
        AdmissionDecision::Allow
    );
    policy.hold_task("retention");
    assert_eq!(policy.active_module(), Some(StorageModule::TrafficArchive));
    assert_eq!(
        policy.decision("retention"),
        AdmissionDecision::Hold {
            reason: "migration_hold",
        }
    );
    assert_eq!(policy.decision("routing"), AdmissionDecision::Allow);
    assert_eq!(
        combine_admission_decisions(AdmissionDecision::Allow, policy.decision("retention")),
        AdmissionDecision::Hold {
            reason: "migration_hold",
        }
    );
    policy.release_module();
    assert_eq!(
        policy.decision_for_module(StorageModule::TrafficArchive, "archive"),
        AdmissionDecision::Allow
    );
    assert_eq!(
        policy.decision("retention"),
        AdmissionDecision::Hold {
            reason: "migration_hold",
        }
    );
    policy.release_task("retention");
    assert_eq!(policy.decision("retention"), AdmissionDecision::Allow);
    policy.release_preserved_task("routing");
    policy.release_preserved_task("oauth_renewal");
    policy.hold_module(StorageModule::TrafficArchive);
    assert_eq!(
        policy.decision_for_module(StorageModule::TrafficArchive, "routing"),
        AdmissionDecision::Hold {
            reason: "migration_hold",
        }
    );
}

#[test]
fn storage_config_defaults_keep_postgres_optional_and_switch_budget_bounded() {
    let config = StorageConfig::default();
    assert!(config.postgres_database_url.is_none());
    assert_eq!(config.postgres_max_connections, 5);
    assert_eq!(
        config.migration_switch_budget,
        std::time::Duration::from_secs(3)
    );
    let mut configured = config.clone();
    configured.postgres_database_url = Some("postgresql://user:secret@example.invalid/db".into());
    let debug = format!("{configured:?}");
    assert!(!debug.contains("postgresql://"));
    let serialized = serde_json::to_string(&configured).unwrap();
    assert!(!serialized.contains("postgresql://"));
}

#[tokio::test]
async fn optional_postgres_storage_transaction_smoke_test() {
    let Ok(database_url) = std::env::var("POSTGRES_TEST_DATABASE_URL") else {
        return;
    };
    let config = StorageConfig {
        postgres_database_url: Some(database_url),
        migration_admin_token: None,
        postgres_max_connections: 1,
        migration_switch_budget: std::time::Duration::from_secs(3),
    };
    let storage = StoragePools::from_sqlite_and_config(
        SqlitePool::connect("sqlite::memory:")
            .await
            .expect("SQLite storage fixture must connect"),
        &config,
    )
    .await
    .unwrap_or_else(|_| panic!("PostgreSQL storage fixture must connect"));
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Postgres,
            generation: 1,
        },
    );
    let mut transaction = storage
        .begin_typed::<TrafficArchiveTransaction>(
            gate.acquire(StorageModule::TrafficArchive)
                .await
                .expect("PostgreSQL storage fixture must admit a lease"),
        )
        .await
        .expect("PostgreSQL storage fixture must begin a transaction");
    let result: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&mut **transaction.postgres_transaction().unwrap())
        .await
        .expect("PostgreSQL storage transaction query must succeed");
    assert_eq!(result, 1);
    assert_eq!(gate.active_leases_for_test(), 1);
    transaction
        .commit()
        .await
        .expect("PostgreSQL storage transaction commit must succeed");
    assert_eq!(gate.active_leases_for_test(), 0);

    let transaction = storage
        .begin_typed::<TrafficArchiveTransaction>(
            gate.acquire(StorageModule::TrafficArchive)
                .await
                .expect("PostgreSQL storage fixture must admit a second lease"),
        )
        .await
        .expect("PostgreSQL storage fixture must begin a second transaction");
    transaction
        .rollback()
        .await
        .expect("PostgreSQL storage transaction rollback must succeed");
    assert_eq!(gate.active_leases_for_test(), 0);
}
