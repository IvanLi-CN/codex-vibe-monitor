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
use anyhow::{Context, Result, bail};
use sqlx::{
    Connection, Pool, Sqlite, SqliteConnection, postgres::PgPoolOptions, sqlite::SqlitePoolOptions,
};
use std::{
    ffi::CStr,
    os::raw::{c_char, c_int, c_void},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

static STARTUP_ARCHIVE_BATCHES_READS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn startup_history_authorizer(
    _user_data: *mut c_void,
    action: c_int,
    arg1: *const c_char,
    _arg2: *const c_char,
    _database: *const c_char,
    _trigger: *const c_char,
) -> c_int {
    if action == libsqlite3_sys::SQLITE_READ && !arg1.is_null() {
        let table_name = unsafe { CStr::from_ptr(arg1) }.to_bytes();
        if table_name == b"archive_batches" {
            STARTUP_ARCHIVE_BATCHES_READS.fetch_add(1, Ordering::Relaxed);
        }
    }
    libsqlite3_sys::SQLITE_OK
}

async fn install_startup_history_authorizer(connection: &mut SqliteConnection) -> sqlx::Result<()> {
    let mut handle = connection.lock_handle().await?;
    let result = unsafe {
        libsqlite3_sys::sqlite3_set_authorizer(
            handle.as_raw_handle().as_ptr(),
            Some(startup_history_authorizer),
            std::ptr::null_mut(),
        )
    };
    if result != libsqlite3_sys::SQLITE_OK {
        return Err(sqlx::Error::Protocol(format!(
            "install startup history SQLite authorizer failed with code {result}"
        )));
    }
    Ok(())
}

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

fn startup_acceptance_evidence_dir() -> Option<PathBuf> {
    std::env::var_os("POSTGRES_ACCEPTANCE_EVIDENCE_DIR").map(PathBuf::from)
}

fn startup_acceptance_sha(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn write_startup_acceptance_evidence(payload: serde_json::Value) -> Result<()> {
    let Some(directory) = startup_acceptance_evidence_dir() else {
        if std::env::var("POSTGRES_ACCEPTANCE_REQUIRED").as_deref() == Ok("1") {
            bail!("PostgreSQL acceptance evidence directory is not configured")
        }
        return Ok(());
    };
    std::fs::create_dir_all(&directory).with_context(|| {
        format!(
            "create PostgreSQL startup evidence directory {}",
            directory.display()
        )
    })?;
    let path = directory.join("startup-performance.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&payload)?)
        .with_context(|| format!("write PostgreSQL startup evidence {}", path.display()))?;
    Ok(())
}

async fn startup_postgres_server_version(database_url: &str) -> Result<String> {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
        .context("connect PostgreSQL startup acceptance probe")?;
    let version = sqlx::query_scalar::<_, String>("SHOW server_version")
        .fetch_one(&pool)
        .await
        .context("read PostgreSQL startup acceptance server version")?;
    pool.close().await;
    if !version.starts_with("18.") {
        bail!("PostgreSQL acceptance requires pinned major version 18, found {version}");
    }
    Ok(version)
}

async fn seed_startup_history(pool: &Pool<Sqlite>, row_count: usize) -> Result<()> {
    ensure_persistence_schema(pool).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS archive_batches (
            archive_batch_id INTEGER PRIMARY KEY,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            file_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            source_identity_sha256 TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            layout TEXT NOT NULL,
            codec TEXT NOT NULL,
            cleanup_state TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .context("create startup performance archive history fixture")?;
    for index in 0..row_count {
        sqlx::query(
            "INSERT INTO archive_batches
                (archive_batch_id, dataset, month_key, file_path, sha256,
                 source_identity_sha256, row_count, status, layout, codec, cleanup_state)
             VALUES (?, 'codex_invocations', ?, ?, ?, ?, 1, 'completed', 'legacy_month', 'gzip', 'active')",
        )
        .bind(i64::try_from(index + 1).context("startup fixture row id overflow")?)
        .bind(format!("2026-{month:02}", month = (index % 12) + 1))
        .bind(format!("archive-{index:06}.db"))
        .bind(format!("sha-{index:064x}"))
        .bind(format!("source-{index:064x}"))
        .execute(pool)
        .await
        .with_context(|| format!("seed startup history row {index}"))?;
    }
    Ok(())
}

struct StartupMeasurement {
    samples_us: Vec<u128>,
    archive_batches_read_counts: Vec<usize>,
}

async fn reset_startup_measurement_probe(pool: &Pool<Sqlite>) -> Result<()> {
    let mut connection = pool.acquire().await?;
    connection.clear_cached_statements().await?;
    STARTUP_ARCHIVE_BATCHES_READS.store(0, Ordering::Relaxed);
    Ok(())
}

async fn measure_startup_initialization(
    database_url: Option<&str>,
    row_count: usize,
) -> Result<StartupMeasurement> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _metadata| {
            Box::pin(install_startup_history_authorizer(connection))
        })
        .connect("sqlite::memory:")
        .await
        .context("open startup performance SQLite fixture")?;
    seed_startup_history(&pool, row_count).await?;
    let config = StorageConfig {
        postgres_database_url: database_url.map(str::to_owned),
        ..StorageConfig::default()
    };

    // Warm metadata creation so samples measure the production initialization path rather than
    // one-time SQLite DDL. The historical archive rows remain present for every sample.
    let warmup_migration = Arc::new(StorageMigrationRuntime::new(&config));
    let warmup = StorageRuntimeRouter::initialize(
        pool.clone(),
        &config,
        warmup_migration,
        CancellationToken::new(),
    )
    .await?;
    warmup.stop().await;
    reset_startup_measurement_probe(&pool).await?;

    let mut samples = Vec::with_capacity(5);
    let mut archive_batches_read_counts = Vec::with_capacity(5);
    for _ in 0..5 {
        reset_startup_measurement_probe(&pool).await?;
        let migration = Arc::new(StorageMigrationRuntime::new(&config));
        let started = Instant::now();
        let router = StorageRuntimeRouter::initialize(
            pool.clone(),
            &config,
            migration,
            CancellationToken::new(),
        )
        .await?;
        let elapsed = started.elapsed().as_micros();
        if database_url.is_some() && !router.postgres_configured() {
            bail!("configured PostgreSQL startup fixture did not build a lazy target pool");
        }
        let archive_batches_read_count = STARTUP_ARCHIVE_BATCHES_READS.load(Ordering::Relaxed);
        if archive_batches_read_count != 0 {
            bail!(
                "startup initialization read archive_batches {archive_batches_read_count} time(s)"
            );
        }
        router.stop().await;
        samples.push(elapsed);
        archive_batches_read_counts.push(archive_batches_read_count);
    }
    pool.close().await;
    Ok(StartupMeasurement {
        samples_us: samples,
        archive_batches_read_counts,
    })
}

fn startup_p95_microseconds(samples: &[u128]) -> u128 {
    assert!(!samples.is_empty(), "startup performance needs samples");
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let index = (sorted.len() * 95).div_ceil(100).saturating_sub(1);
    sorted[index]
}

#[tokio::test]
async fn optional_postgres_storage_runtime_startup_performance_is_within_budget() -> Result<()> {
    let Some(database_url) = crate::tests::optional_postgres_test_database_url(
        "optional_postgres_storage_runtime_startup_performance_is_within_budget",
    ) else {
        return Ok(());
    };
    let server_version = startup_postgres_server_version(&database_url).await?;
    let target_identity = StorageConfig {
        postgres_database_url: Some(database_url.clone()),
        ..StorageConfig::default()
    }
    .postgres_target_identity();
    let mut measurements = Vec::new();
    for row_count in [32_usize, 4096_usize] {
        let control = measure_startup_initialization(None, row_count).await?;
        let enabled = measure_startup_initialization(Some(&database_url), row_count).await?;
        let control_p95 = startup_p95_microseconds(&control.samples_us);
        let enabled_p95 = startup_p95_microseconds(&enabled.samples_us);
        let added_p95 = enabled_p95.saturating_sub(control_p95);
        if added_p95 > 500_000 {
            bail!(
                "configured PostgreSQL startup added {added_p95}us p95 for {row_count} historical rows"
            );
        }
        let control_archive_batches_read_total =
            control.archive_batches_read_counts.iter().sum::<usize>();
        let enabled_archive_batches_read_total =
            enabled.archive_batches_read_counts.iter().sum::<usize>();
        let control_historical_scan_detected = control
            .archive_batches_read_counts
            .iter()
            .any(|count| *count != 0);
        let enabled_historical_scan_detected = enabled
            .archive_batches_read_counts
            .iter()
            .any(|count| *count != 0);
        measurements.push(serde_json::json!({
            "historical_rows": row_count,
            "control": {
                "migration_enabled": false,
                "sample_count": control.samples_us.len(),
                "samples_us": control.samples_us,
                "p95_us": control_p95,
                "archive_batches_read_counts": control.archive_batches_read_counts,
                "archive_batches_read_total": control_archive_batches_read_total,
                "historical_scan_detected": control_historical_scan_detected,
            },
            "postgres_configured": {
                "migration_enabled": true,
                "sample_count": enabled.samples_us.len(),
                "samples_us": enabled.samples_us,
                "p95_us": enabled_p95,
                "archive_batches_read_counts": enabled.archive_batches_read_counts,
                "archive_batches_read_total": enabled_archive_batches_read_total,
                "historical_scan_detected": enabled_historical_scan_detected,
            },
            "added_p95_us": added_p95,
            "threshold_us": 500_000,
        }));
    }
    write_startup_acceptance_evidence(serde_json::json!({
        "schema_version": 1,
        "acceptance": "postgres-startup-performance",
        "fixture_version": std::env::var("POSTGRES_ACCEPTANCE_FIXTURE_VERSION").unwrap_or_else(|_| "4.13.11".into()),
        "base_sha": startup_acceptance_sha("BASE_SHA"),
        "candidate_sha": startup_acceptance_sha("CANDIDATE_SHA"),
        "server_version": server_version,
        "source_identity": "sqlite:local",
        "target_identity": target_identity,
        "workload": "StorageRuntimeRouter::initialize with 32-row and 4096-row archive_batches history fixtures",
        "measured_path": "StorageRuntimeRouter::initialize; worker start and historical migration scan are excluded",
        "duration_unit": "microseconds",
        "maintenance_held": false,
        "sample_count_per_scenario": 5,
        "threshold_us": 500_000,
        "historical_scan_expected": false,
        "historical_scan_detection": {
            "method": "SQLite authorizer SQLITE_READ action for archive_batches during each StorageRuntimeRouter::initialize call",
            "scope": "after metadata warmup and statement-cache reset, before router.stop",
            "expected_archive_batches_read_count_per_sample": 0,
        },
        "measurements": measurements,
        "raw_result_path": startup_acceptance_evidence_dir().map(|path| path.join("stdout.txt")),
        "command": "cargo nextest run --profile lightweight --test-filter tests::lightweight::storage_runtime::optional_postgres_storage_runtime_startup_performance_is_within_budget"
    }))?;
    Ok(())
}
