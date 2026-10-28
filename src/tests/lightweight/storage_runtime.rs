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
use sqlx::{Pool, Sqlite, sqlite::SqlitePoolOptions};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[test]
fn postgres_target_identity_includes_schema_options_without_credentials() {
    let public_a = StorageConfig {
        postgres_database_url: Some(
            "postgresql://user:secret-a@127.0.0.1:5432/cvm?options=-csearch_path%3Dpublic"
                .to_string(),
        ),
        ..StorageConfig::default()
    };
    let public_b = StorageConfig {
        postgres_database_url: Some(
            "postgresql://user:secret-b@127.0.0.1:5432/cvm?options=-csearch_path%3Dpublic"
                .to_string(),
        ),
        ..StorageConfig::default()
    };
    let private = StorageConfig {
        postgres_database_url: Some(
            "postgresql://user:secret-a@127.0.0.1:5432/cvm?options=-csearch_path%3Dprivate"
                .to_string(),
        ),
        ..StorageConfig::default()
    };

    assert_eq!(
        public_a.postgres_target_identity(),
        public_b.postgres_target_identity()
    );
    assert_ne!(
        public_a.postgres_target_identity(),
        private.postgres_target_identity()
    );
    assert!(
        public_a
            .postgres_target_identity()
            .contains("options=-csearch_path%3Dpublic")
    );
}

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
async fn restart_rejects_mismatched_never_migrated_recovery_marker() {
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
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS storage_migration_authority (
            module TEXT PRIMARY KEY,
            recovery_json TEXT,
            manifest_json TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("authority metadata should exist");
    let stale_marker = AuthorityRecoveryContext {
        migration_id: "storage-migration".to_string(),
        installation_id: "local".to_string(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite:local".to_string(),
        target_identity: "postgres://127.0.0.1:5432/other".to_string(),
        source_watermark: 0,
    };
    sqlx::query(
        "INSERT INTO storage_migration_authority (module, recovery_json, manifest_json)
         VALUES ('traffic_archive', ?1, NULL)",
    )
    .bind(serde_json::to_string(&stale_marker).expect("recovery marker should encode"))
    .execute(&pool)
    .await
    .expect("stale recovery marker should persist");

    let migration = Arc::new(
        StorageMigrationRuntime::load(&config, pool.clone())
            .await
            .expect("migration status should load"),
    );
    let error =
        StorageRuntimeRouter::initialize(pool, &config, migration, CancellationToken::new())
            .await
            .expect_err("startup must reject a stale no-cutover recovery marker");
    assert!(error.to_string().contains("recovery marker identity"));
}

#[tokio::test]
async fn restart_repairs_stale_pre_switch_progress_from_committed_manifest() {
    let database_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-storage-runtime-recovery-{}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&database_path);
    let database_url = format!("sqlite://{}?mode=rwc", database_path.to_string_lossy());
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
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
        target_identity: config.postgres_target_identity(),
        source_watermark: 7,
    };
    router
        .worker()
        .metadata
        .set_recovery_context(&context)
        .await
        .expect("authority recovery context should persist");
    migration
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("migration should start");
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
            .await
            .expect("migration phase should persist");
    }
    // Model an older release whose pre-switch boundary was still durable as Ready.
    migration
        .publish_progress_durable(StorageMigrationProgress {
            phase: StorageMigrationState::Ready,
            captured_through: Some(6),
            applied_through: Some(6),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("pre-switch checkpoint should persist");

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

    pool.close().await;
    let error = migration
        .publish_progress_durable(StorageMigrationProgress {
            phase: StorageMigrationState::Switching,
            captured_through: Some(7),
            applied_through: Some(7),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect_err("final Switching progress should fail after persistence is closed");
    assert!(
        error
            .to_string()
            .contains("persist storage migration status")
    );
    drop(router);
    drop(migration);

    let restarted_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("SQLite persistence should reopen after the simulated crash");
    let restarted_migration = Arc::new(
        StorageMigrationRuntime::load(&config, restarted_pool.clone())
            .await
            .expect("stale pre-switch status should reload"),
    );
    let recovered = StorageRuntimeRouter::initialize(
        restarted_pool,
        &config,
        restarted_migration.clone(),
        CancellationToken::new(),
    )
    .await
    .expect("committed manifest should repair stale pre-switch status");

    assert_eq!(
        recovered
            .current_authority(StorageModule::TrafficArchive)
            .await,
        ModuleAuthority {
            backend: BackendKind::Postgres,
            generation: 2,
        }
    );
    let status = restarted_migration.snapshot().await;
    assert_eq!(status.state, StorageMigrationState::Postgres);
    assert_eq!(status.progress.captured_through, Some(7));
    assert_eq!(status.progress.applied_through, Some(7));
    recovered.stop().await;
    drop(recovered);
    let _ = std::fs::remove_file(database_path);
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
        target_identity: config.postgres_target_identity(),
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

    let restarted_migration = Arc::new(
        StorageMigrationRuntime::load(&config, pool.clone())
            .await
            .expect("durable migration status should reload"),
    );
    let recovered = StorageRuntimeRouter::initialize(
        pool.clone(),
        &config,
        restarted_migration.clone(),
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
        restarted_migration.snapshot().await.state,
        StorageMigrationState::Postgres
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'migration_change_log'
            )",
        )
        .fetch_one(&pool)
        .await
        .expect("capture table existence should be queryable"),
        0
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

#[tokio::test]
async fn restart_rejects_every_durable_postgres_manifest_identity_mismatch() {
    for field in [
        "migration_id",
        "installation_id",
        "module",
        "source_identity",
        "target_identity",
        "source_watermark",
    ] {
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
        let mut manifest = seed_postgres_authority_fixture(&pool, &config, &migration).await;
        match field {
            "migration_id" => manifest.migration_id = "other-migration".to_string(),
            "installation_id" => manifest.installation_id = "other-installation".to_string(),
            "module" => manifest.module = StorageModule::Analytics,
            "source_identity" => manifest.source_identity = "sqlite:other".to_string(),
            "target_identity" => {
                manifest.target_identity = "postgres://127.0.0.1:5432/other".to_string()
            }
            "source_watermark" => manifest.source_watermark = 6,
            _ => unreachable!("test field is listed above"),
        }
        sqlx::query(
            "UPDATE storage_migration_authority
             SET manifest_json = ?1
             WHERE module = 'traffic_archive'",
        )
        .bind(serde_json::to_string(&manifest).expect("manifest should encode"))
        .execute(&pool)
        .await
        .expect("corrupted manifest should persist");

        let restarted_migration = Arc::new(
            StorageMigrationRuntime::load(&config, pool.clone())
                .await
                .expect("durable migration status should reload"),
        );
        let error = StorageRuntimeRouter::initialize(
            pool,
            &config,
            restarted_migration,
            CancellationToken::new(),
        )
        .await
        .expect_err("corrupted manifest identity must fail closed");
        assert!(
            error.to_string().contains("manifest") || error.to_string().contains("watermark"),
            "{field}: {error:#}"
        );
    }
}

async fn seed_postgres_authority_fixture(
    pool: &Pool<Sqlite>,
    config: &StorageConfig,
    migration: &Arc<StorageMigrationRuntime>,
) -> ModuleAuthorityManifest {
    let router = StorageRuntimeRouter::initialize(
        pool.clone(),
        config,
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
        target_identity: config.postgres_target_identity(),
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
    .execute(pool)
    .await
    .expect("source authority state should exist");
    sqlx::query(
        "INSERT INTO migration_change_log_state
            (migration_id, source_identity, authority_generation, source_writes_fenced)
         VALUES ('storage-migration', 'sqlite:local', 2, 1)",
    )
    .execute(pool)
    .await
    .expect("source authority fence should persist");
    manifest
}
