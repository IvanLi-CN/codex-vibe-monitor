use sqlx::SqlitePool;

use crate::storage::catalog::{CatalogError, ModuleCatalog, ModuleId};
use crate::storage::core::{
    AdmissionDecision, BackendKind, MigrationState, MigrationStatus, ModuleAuthority,
    ModuleGenerationGate, ModuleLease, StorageModule, StoragePools, validate_transaction_leases,
};

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
    let switch = tokio::spawn(async move {
        switch_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                ModuleAuthority {
                    backend: BackendKind::Postgres,
                    generation: 2,
                },
                std::time::Duration::from_secs(1),
            )
            .await
    });
    tokio::task::yield_now().await;
    assert!(gate.acquire(StorageModule::TrafficArchive).await.is_err());
    drop(old);
    switch.await.unwrap().unwrap();
    let new = gate.acquire(StorageModule::TrafficArchive).await.unwrap();
    assert_eq!(new.lease.authority.backend, BackendKind::Postgres);
    assert_eq!(new.lease.authority.generation, 2);
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
    let switch = tokio::spawn(async move {
        switch_gate
            .switch_authority(
                StorageModule::TrafficArchive,
                ModuleAuthority {
                    backend: BackendKind::Postgres,
                    generation: 2,
                },
                std::time::Duration::from_secs(1),
            )
            .await
    });
    tokio::task::yield_now().await;
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
