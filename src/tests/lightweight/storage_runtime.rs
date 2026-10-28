use crate::storage::config::StorageConfig;
use crate::storage::core::{
    AuthorityManifestStore, AuthorityPublication, AuthorityRecoveryContext, BackendKind,
    MigrationMetadataStore, ModuleAuthority, ModuleAuthorityManifest, ModuleGenerationGate,
    StorageModule, StoragePools,
};
use crate::storage::migration::SourceWriteFence;
use crate::storage::migration_status::{
    StorageMigrationControlRequest, StorageMigrationProgress, StorageMigrationRuntime,
    StorageMigrationState, ensure_persistence_schema,
};
use crate::storage::runtime::StorageRuntimeRouter;
use crate::storage::traffic::TrafficFactStore;
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn configured_postgres_builds_authority_router_and_scoped_worker() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite pool should open");
    ensure_persistence_schema(&pool)
        .await
        .expect("migration status persistence should exist");
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://user:pass@127.0.0.1:5432/cvm".to_string()),
        ..StorageConfig::default()
    };
    let migration = Arc::new(StorageMigrationRuntime::new(&config));
    let process_shutdown = CancellationToken::new();
    let router =
        StorageRuntimeRouter::initialize(pool, &config, migration, process_shutdown.child_token())
            .await
            .expect("configured PostgreSQL should build the lazy runtime pool");

    assert!(router.postgres_configured());
    assert_eq!(
        router
            .current_authority(StorageModule::TrafficArchive)
            .await,
        crate::storage::core::ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        }
    );
    router.start().await;
    router.stop().await;
    assert!(!process_shutdown.is_cancelled());
}

#[tokio::test]
async fn restart_rejects_postgres_status_without_durable_authority_manifest() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite pool should open");
    ensure_persistence_schema(&pool)
        .await
        .expect("migration status persistence should exist");
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://user:pass@127.0.0.1:5432/cvm".to_string()),
        ..StorageConfig::default()
    };
    let migration = Arc::new(
        StorageMigrationRuntime::load(&config, pool.clone())
            .await
            .expect("migration status should load"),
    );
    migration
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("migration should start");
    for phase in [
        StorageMigrationState::Copying,
        StorageMigrationState::CatchingUp,
        StorageMigrationState::Verifying,
        StorageMigrationState::Ready,
        StorageMigrationState::Switching,
    ] {
        migration
            .publish_progress(StorageMigrationProgress {
                phase,
                ..StorageMigrationProgress::default()
            })
            .await
            .expect("migration phase should persist");
    }
    migration
        .publish_postgres_authority(2, 0)
        .await
        .expect("PostgreSQL status should persist");

    let error =
        StorageRuntimeRouter::initialize(pool, &config, migration, CancellationToken::new())
            .await
            .expect_err("restart must fail closed without the durable authority manifest");
    assert!(
        error
            .to_string()
            .contains("PostgreSQL authority has no durable traffic manifest")
    );
}

#[tokio::test]
async fn restart_rejects_sqlite_authority_generation_drift() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite pool should open");
    ensure_persistence_schema(&pool)
        .await
        .expect("migration status persistence should exist");
    sqlx::query(
        "CREATE TABLE migration_change_log_state (
            migration_id TEXT NOT NULL,
            source_identity TEXT NOT NULL,
            authority_generation INTEGER NOT NULL,
            source_writes_fenced INTEGER NOT NULL,
            PRIMARY KEY (migration_id, source_identity)
        )",
    )
    .execute(&pool)
    .await
    .expect("source authority state should exist");
    sqlx::query(
        "INSERT INTO migration_change_log_state
            (migration_id, source_identity, authority_generation, source_writes_fenced)
         VALUES ('storage-migration', 'sqlite:local', 2, 0)",
    )
    .execute(&pool)
    .await
    .expect("source authority generation should persist");

    let config = StorageConfig::default();
    let migration = Arc::new(
        StorageMigrationRuntime::load(&config, pool.clone())
            .await
            .expect("migration status should load"),
    );
    let error =
        StorageRuntimeRouter::initialize(pool, &config, migration, CancellationToken::new())
            .await
            .expect_err("generation drift must fail closed before opening SQLite authority");
    assert!(
        error
            .to_string()
            .contains("SQLite source authority generation does not match")
    );
}

#[tokio::test]
async fn restart_recovers_durable_postgres_authority_without_capture_scan() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite pool should open");
    ensure_persistence_schema(&pool)
        .await
        .expect("migration status persistence should exist");
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://user:pass@127.0.0.1:5432/cvm".to_string()),
        ..StorageConfig::default()
    };
    let migration = Arc::new(
        StorageMigrationRuntime::load(&config, pool.clone())
            .await
            .expect("migration status should load"),
    );
    let router = StorageRuntimeRouter::initialize(
        pool.clone(),
        &config,
        migration.clone(),
        CancellationToken::new(),
    )
    .await
    .expect("initial SQLite authority should initialize");
    let context = AuthorityRecoveryContext {
        migration_id: "storage-migration".to_string(),
        installation_id: "local".to_string(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite:local".to_string(),
        target_identity: "postgres:configured".to_string(),
        source_watermark: 7,
    };
    router
        .worker()
        .metadata
        .set_recovery_context(&context)
        .await
        .expect("authority recovery context should persist");
    let manifest = ModuleAuthorityManifest {
        migration_id: context.migration_id.clone(),
        installation_id: context.installation_id.clone(),
        source_identity: context.source_identity.clone(),
        module: context.module,
        authority: ModuleAuthority {
            backend: BackendKind::Postgres,
            generation: 2,
        },
        source_watermark: context.source_watermark,
        target_identity: context.target_identity.clone(),
    };
    assert_eq!(
        router
            .worker()
            .metadata
            .publish_authority_manifest(&manifest, &context)
            .await
            .expect("authority manifest should publish"),
        AuthorityPublication::Committed
    );
    migration
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("migration should start");
    for phase in [
        StorageMigrationState::Copying,
        StorageMigrationState::CatchingUp,
        StorageMigrationState::Verifying,
        StorageMigrationState::Ready,
        StorageMigrationState::Switching,
    ] {
        migration
            .publish_progress(StorageMigrationProgress {
                phase,
                ..StorageMigrationProgress::default()
            })
            .await
            .expect("migration phase should persist");
    }
    migration
        .publish_postgres_authority(2, 7)
        .await
        .expect("migration status should publish PostgreSQL authority");
    sqlx::query(
        "CREATE TABLE migration_change_log_state (
            migration_id TEXT NOT NULL,
            source_identity TEXT NOT NULL,
            authority_generation INTEGER NOT NULL,
            source_writes_fenced INTEGER NOT NULL,
            PRIMARY KEY (migration_id, source_identity)
        )",
    )
    .execute(&pool)
    .await
    .expect("source authority state should exist");
    sqlx::query(
        "INSERT INTO migration_change_log_state
            (migration_id, source_identity, authority_generation, source_writes_fenced)
         VALUES ('storage-migration', 'sqlite:local', 2, 1)",
    )
    .execute(&pool)
    .await
    .expect("source authority fence should persist");

    let recovered = StorageRuntimeRouter::initialize(
        pool,
        &config,
        migration.clone(),
        CancellationToken::new(),
    )
    .await
    .expect("durable PostgreSQL authority should recover");
    assert_eq!(
        recovered
            .current_authority(StorageModule::TrafficArchive)
            .await,
        ModuleAuthority {
            backend: BackendKind::Postgres,
            generation: 2,
        }
    );
    assert_eq!(
        migration.snapshot().await.state,
        StorageMigrationState::Postgres
    );
}

#[tokio::test]
async fn postgres_authority_does_not_consume_the_fenced_sqlite_source_lease() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite pool should open");
    let source_fence = SourceWriteFence::new(1);
    source_fence
        .close_and_drain(std::time::Duration::from_secs(1))
        .await
        .expect("source fence should close");
    let authority = TrafficFactStore::with_authority_and_fence(
        StoragePools::sqlite_only(pool),
        ModuleGenerationGate::new(
            StorageModule::TrafficArchive,
            ModuleAuthority {
                backend: BackendKind::Postgres,
                generation: 2,
            },
        ),
        source_fence,
    );

    let error = match authority.begin().await {
        Ok(_) => panic!("the missing PostgreSQL pool should be the selected failure"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("without a PostgreSQL pool"));
}
