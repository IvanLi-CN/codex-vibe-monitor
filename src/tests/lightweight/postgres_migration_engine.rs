use std::{
    fs,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions};

use crate::storage::{
    core::{
        AuthorityRecoveryContext, BackendKind, ModuleAuthority, ModuleGenerationGate, StorageModule,
    },
    migration::{
        CapturedChange, ChangeInput, ChangeKind, ChunkRange, CopyLimits, EngineCheckpoint,
        FileMigrationStore, InMemoryMigrationSource, InMemoryMigrationTarget, MigrationEngine,
        MigrationLimits, MigrationSource, MigrationTarget, SourcePage, SourceWriteFence,
        TableCopyPlan, TableCoverageProof, VersionedRow,
    },
    migration_sqlx::{MigrationTableSpec, PostgresMigrationTarget, SqliteMigrationSource},
};

fn change(table_name: &str, stable_key: &str, kind: ChangeKind, payload: &[u8]) -> ChangeInput {
    ChangeInput {
        table_name: table_name.into(),
        stable_key: stable_key.into(),
        previous_key: None,
        kind,
        payload: (kind != ChangeKind::Delete).then(|| payload.to_vec()),
    }
}

fn state_path(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("codex-pg02-native-{label}-{suffix}"));
    (directory.clone(), directory.join("migration-state.json"))
}

fn engine(
    source: Arc<InMemoryMigrationSource>,
    target: Arc<InMemoryMigrationTarget>,
    store: Arc<FileMigrationStore>,
    fence: SourceWriteFence,
) -> MigrationEngine<InMemoryMigrationSource, InMemoryMigrationTarget, FileMigrationStore> {
    MigrationEngine::new(
        source,
        target,
        store,
        StorageModule::TrafficArchive,
        "installation-test",
        "postgres-test",
        1,
        MigrationLimits {
            copy: CopyLimits {
                max_rows: 2,
                max_bytes: 1024,
                source_read_budget: Duration::from_secs(1),
                batch_yield: Duration::ZERO,
            },
            ..MigrationLimits::default()
        },
        fence,
    )
    .expect("migration engine construction")
}

#[tokio::test]
async fn file_migration_store_reopens_after_parent_synced_replace() -> Result<()> {
    let (directory, path) = state_path("durability");
    let initial = AuthorityRecoveryContext {
        migration_id: "migration-engine-test".into(),
        installation_id: "installation-test".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite-test".into(),
        target_identity: "postgres-test".into(),
        source_watermark: 1,
    };
    let updated = AuthorityRecoveryContext {
        source_watermark: 7,
        ..initial.clone()
    };
    let store = FileMigrationStore::new(&path, initial.clone());
    store.set_recovery_context(&updated).await?;
    drop(store);

    assert!(!path.with_extension("tmp").exists());
    let reopened = FileMigrationStore::new(&path, initial);
    assert!(matches!(
        reopened
            .load_authority_manifest(StorageModule::TrafficArchive)
            .await?,
        crate::storage::core::AuthorityRecoveryRecord::NeverMigrated {
            source_watermark: 7,
            ..
        }
    ));

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn migration_engine_copies_catches_up_verifies_and_publishes_authority() -> Result<()> {
    let (directory, path) = state_path("authority");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![change("events", "a", ChangeKind::Insert, b"one")],
        )
        .await?;
    let target = Arc::new(InMemoryMigrationTarget::default());
    let store = Arc::new(FileMigrationStore::new(
        &path,
        AuthorityRecoveryContext {
            migration_id: "migration-engine-test".into(),
            installation_id: "installation-test".into(),
            module: StorageModule::TrafficArchive,
            source_identity: "sqlite-test".into(),
            target_identity: "postgres-test".into(),
            source_watermark: 2,
        },
    ));
    let engine = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        fence.clone(),
    );

    engine.prepare(&["events".into()]).await?;
    assert!(engine.copy_next("events").await?.complete);
    source
        .write(
            &fence,
            vec![change("events", "a", ChangeKind::Update, b"two")],
        )
        .await?;
    assert_eq!(engine.catch_up("events").await?.applied_through, 2);
    assert_eq!(
        engine
            .verify_chunk("events", ChunkRange::new("a", "a")?)
            .await?,
        crate::storage::migration::ChunkVerification::Verified
    );
    let proof = engine.verify_table_coverage_at("events", 2).await?;
    assert_eq!(proof.source_digest, proof.target_digest);
    source
        .write(
            &fence,
            vec![change("events", "a", ChangeKind::Update, b"three")],
        )
        .await?;

    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    engine
        .switch_authority(
            &gate,
            AuthorityRecoveryContext {
                migration_id: "migration-engine-test".into(),
                installation_id: "installation-test".into(),
                module: StorageModule::TrafficArchive,
                source_identity: "sqlite-test".into(),
                target_identity: "postgres-test".into(),
                source_watermark: 2,
            },
            Duration::from_secs(1),
        )
        .await?;
    assert!(fence.is_fenced().await);
    assert!(
        source
            .write(
                &fence,
                vec![change("events", "b", ChangeKind::Insert, b"late")],
            )
            .await
            .is_err()
    );
    assert!(matches!(
        store
            .load_authority_manifest(StorageModule::TrafficArchive)
            .await?,
        crate::storage::core::AuthorityRecoveryRecord::Manifest(manifest)
            if manifest.authority.backend == BackendKind::Postgres
                && manifest.source_watermark == 3
    ));

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn migration_rejects_incomplete_module_table_sets() -> Result<()> {
    let (directory, path) = state_path("table-set");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    source.register_module_table("events").await?;
    source.register_module_table("accounts").await?;
    let target = Arc::new(InMemoryMigrationTarget::default());
    let fence = SourceWriteFence::new(1);
    let store = Arc::new(FileMigrationStore::new(
        &path,
        AuthorityRecoveryContext {
            migration_id: "migration-engine-test".into(),
            installation_id: "installation-test".into(),
            module: StorageModule::TrafficArchive,
            source_identity: "sqlite-test".into(),
            target_identity: "postgres-test".into(),
            source_watermark: 0,
        },
    ));
    let migration = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        fence.clone(),
    );
    assert!(migration.prepare(&["events".into()]).await.is_err());
    migration
        .prepare(&["accounts".into(), "events".into()])
        .await?;
    assert!(migration.copy_next("accounts").await?.complete);
    assert!(migration.copy_next("events").await?.complete);
    target.remove_plan_for_test("accounts").await;
    assert!(
        migration
            .switch_authority(
                &ModuleGenerationGate::new(
                    StorageModule::TrafficArchive,
                    ModuleAuthority {
                        backend: BackendKind::Sqlite,
                        generation: 1,
                    },
                ),
                AuthorityRecoveryContext {
                    migration_id: "migration-engine-test".into(),
                    installation_id: "installation-test".into(),
                    module: StorageModule::TrafficArchive,
                    source_identity: "sqlite-test".into(),
                    target_identity: "postgres-test".into(),
                    source_watermark: 0,
                },
                Duration::from_secs(1),
            )
            .await
            .is_err()
    );
    assert!(!fence.is_fenced().await);
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn switch_does_not_hold_source_writes_during_large_pre_fence_verification() -> Result<()> {
    let (directory, path) = state_path("pre-fence-verification");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    source.register_module_table("events").await?;
    let fence = SourceWriteFence::new(1);
    let initial_changes = (b'a'..=b'h')
        .map(|key| {
            change(
                "events",
                &String::from_utf8(vec![key]).expect("ASCII key"),
                ChangeKind::Insert,
                &[key],
            )
        })
        .collect();
    source.write(&fence, initial_changes).await?;
    let target = Arc::new(InMemoryMigrationTarget::default());
    let store = Arc::new(FileMigrationStore::new(
        &path,
        AuthorityRecoveryContext {
            migration_id: "migration-engine-test".into(),
            installation_id: "installation-test".into(),
            module: StorageModule::TrafficArchive,
            source_identity: "sqlite-test".into(),
            target_identity: "postgres-test".into(),
            source_watermark: 1,
        },
    ));
    let migration = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        fence.clone(),
    );
    migration.prepare(&["events".into()]).await?;
    while !migration.copy_next("events").await?.complete {}
    source
        .set_verification_delay_for_test(Duration::from_millis(100))
        .await;

    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let switching = {
        let migration = migration.clone();
        let gate = gate.clone();
        tokio::spawn(async move {
            migration
                .switch_authority(
                    &gate,
                    AuthorityRecoveryContext {
                        migration_id: "migration-engine-test".into(),
                        installation_id: "installation-test".into(),
                        module: StorageModule::TrafficArchive,
                        source_identity: "sqlite-test".into(),
                        target_identity: "postgres-test".into(),
                        source_watermark: 1,
                    },
                    Duration::from_secs(1),
                )
                .await
        })
    };
    source.wait_for_verification_for_test().await;
    assert!(
        source
            .write(
                &fence,
                vec![change("events", "z", ChangeKind::Insert, b"late")],
            )
            .await
            .is_ok()
    );
    assert!(switching.await.expect("authority switch task").is_err());
    assert!(!fence.is_fenced().await);

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn sqlite_migration_source_uses_real_pool_capture_and_keyset_pages() -> Result<()> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    sqlx::query(
        "CREATE TABLE events (
            stable_key TEXT PRIMARY KEY,
            source_version INTEGER NOT NULL,
            payload BLOB NOT NULL
        )",
    )
    .execute(&pool)
    .await?;
    let source = SqliteMigrationSource::new(
        pool.clone(),
        "migration-engine-test",
        "sqlite-test",
        "migration_change_log",
        vec![MigrationTableSpec::canonical(
            StorageModule::TrafficArchive,
            "events",
        )?],
    )?;
    source.ensure_capture_schema().await?;

    let mut transaction = pool.begin().await?;
    for (version, key, payload) in [(1_i64, "a", b"one"), (2_i64, "b", b"two")] {
        sqlx::query("INSERT INTO events (stable_key, source_version, payload) VALUES (?, ?, ?)")
            .bind(key)
            .bind(version)
            .bind(payload.as_slice())
            .execute(&mut *transaction)
            .await?;
        let captured = CapturedChange::from_input(
            "migration-engine-test",
            "sqlite-test",
            version,
            change("events", key, ChangeKind::Insert, payload),
        )?;
        source
            .append_captured_change_tx(&mut transaction, &captured)
            .await?;
    }
    transaction.commit().await?;

    assert_eq!(source.capture_watermark().await?, 2);
    let first_page = source
        .scan_page(
            "events",
            None,
            Some("b"),
            CopyLimits {
                max_rows: 1,
                max_bytes: 3,
                source_read_budget: Duration::from_secs(1),
                batch_yield: Duration::ZERO,
            },
        )
        .await?;
    assert_eq!(first_page.next_cursor.as_deref(), Some("a"));
    assert!(!first_page.complete);
    let second_page = source
        .scan_page(
            "events",
            first_page.next_cursor.as_deref(),
            Some("b"),
            CopyLimits {
                max_rows: 1,
                max_bytes: 3,
                source_read_budget: Duration::from_secs(1),
                batch_yield: Duration::ZERO,
            },
        )
        .await?;
    assert_eq!(second_page.rows[0].stable_key, "b");
    assert!(second_page.complete);
    assert_eq!(
        source
            .rows_in_range_snapshot("events", &ChunkRange::new("a", "b")?)
            .await?
            .rows
            .len(),
        2
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_capture_replay_keeps_same_version_boundary_after_failed_apply() -> Result<()> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    sqlx::query(
        "CREATE TABLE events (
            stable_key TEXT PRIMARY KEY,
            source_version INTEGER NOT NULL,
            payload BLOB NOT NULL
        )",
    )
    .execute(&pool)
    .await?;
    let source = Arc::new(SqliteMigrationSource::new(
        pool.clone(),
        "migration-engine-test",
        "sqlite-test",
        "migration_change_log",
        vec![MigrationTableSpec::canonical(
            StorageModule::TrafficArchive,
            "events",
        )?],
    )?);
    source.ensure_capture_schema().await?;
    let target = Arc::new(InMemoryMigrationTarget::default());
    let fence = SourceWriteFence::new(1);
    let (directory, path) = state_path("same-version-replay");
    let store = Arc::new(FileMigrationStore::new(
        &path,
        AuthorityRecoveryContext {
            migration_id: "migration-engine-test".into(),
            installation_id: "installation-test".into(),
            module: StorageModule::TrafficArchive,
            source_identity: "sqlite-test".into(),
            target_identity: "postgres-test".into(),
            source_watermark: 1,
        },
    ));
    let migration = MigrationEngine::new(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        StorageModule::TrafficArchive,
        "installation-test",
        "postgres-test",
        1,
        MigrationLimits {
            max_queued_operations: 2,
            ..MigrationLimits::default()
        },
        fence,
    )?;
    migration.prepare(&["events".into()]).await?;
    assert!(migration.copy_next("events").await?.complete);

    let mut transaction = pool.begin().await?;
    for key in ["a", "b", "c"] {
        sqlx::query("INSERT INTO events (stable_key, source_version, payload) VALUES (?, ?, ?)")
            .bind(key)
            .bind(1_i64)
            .bind(key.as_bytes())
            .execute(&mut *transaction)
            .await?;
        let captured = CapturedChange::from_input(
            "migration-engine-test",
            "sqlite-test",
            1,
            change("events", key, ChangeKind::Insert, key.as_bytes()),
        )?;
        source
            .append_captured_change_tx(&mut transaction, &captured)
            .await?;
    }
    transaction.commit().await?;

    assert_eq!(source.changes_through(0, 1, 2).await?.len(), 3);
    target.fail_next_operation().await;
    assert!(migration.catch_up("events").await.is_err());
    assert_eq!(
        target
            .load_checkpoint("migration-engine-test", "events")
            .await?
            .expect("prepared checkpoint")
            .applied_through,
        0
    );
    assert_eq!(migration.catch_up("events").await?.applied_through, 1);
    for key in ["a", "b", "c"] {
        assert!(target.row("events", key).await.is_some());
    }
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn sqlite_capture_rejects_corrupt_digest_and_source_identity() -> Result<()> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    let source = SqliteMigrationSource::new(
        pool.clone(),
        "migration-engine-test",
        "sqlite-test",
        "migration_change_log",
        vec![MigrationTableSpec::canonical(
            StorageModule::TrafficArchive,
            "events",
        )?],
    )?;
    source.ensure_capture_schema().await?;
    let first = source
        .append_change(1, change("events", "a", ChangeKind::Insert, b"one"))
        .await?;
    sqlx::query("UPDATE migration_change_log SET payload_digest = ? WHERE source_version = 1")
        .bind("corrupt-digest")
        .execute(&pool)
        .await?;
    assert!(source.changes_after(0, 1).await.is_err());

    sqlx::query("UPDATE migration_change_log SET payload_digest = ? WHERE source_version = 1")
        .bind(&first.payload_digest)
        .execute(&pool)
        .await?;
    source
        .append_change(2, change("events", "b", ChangeKind::Insert, b"two"))
        .await?;
    sqlx::query("UPDATE migration_change_log SET source_identity = ? WHERE source_version = 2")
        .bind("another-source")
        .execute(&pool)
        .await?;
    assert!(source.changes_after(0, 2).await.is_err());
    Ok(())
}

#[tokio::test]
async fn optional_postgres_migration_target_uses_real_pool_transactions() -> Result<()> {
    let Ok(database_url) = std::env::var("POSTGRES_TEST_DATABASE_URL") else {
        eprintln!(
            "skipping optional PostgreSQL migration target test: POSTGRES_TEST_DATABASE_URL is unset"
        );
        return Ok(());
    };
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let prefix = format!("pg02_migration_smoke_{suffix}");
    let target_table = format!("{prefix}_rows");
    let target = PostgresMigrationTarget::new(
        pool.clone(),
        &prefix,
        vec![MigrationTableSpec::new(
            StorageModule::TrafficArchive,
            "events",
            target_table.clone(),
            target_table.clone(),
            "stable_key",
            "source_version",
            "payload",
            "stable_key",
            "source_version",
            "payload",
        )?],
    )?;
    sqlx::query(&format!(
        "CREATE TABLE \"{target_table}\" (
            \"stable_key\" TEXT PRIMARY KEY,
            \"source_version\" BIGINT NOT NULL,
            \"payload\" BYTEA NOT NULL
        )"
    ))
    .execute(&pool)
    .await?;
    target.ensure_schema().await?;

    let plan = TableCopyPlan {
        migration_id: "migration-engine-test".into(),
        table_name: "events".into(),
        scan_lower_bound: Some("a".into()),
        scan_upper_bound: Some("a".into()),
        capture_start: 0,
    };
    target.save_plan(&plan).await?;
    let baseline_checkpoint = EngineCheckpoint {
        migration_id: plan.migration_id.clone(),
        table_name: plan.table_name.clone(),
        cursor: Some("a".into()),
        scan_upper_bound: plan.scan_upper_bound.clone(),
        capture_start: 0,
        captured_through: 0,
        applied_through: 0,
        copied_rows: 1,
        copied_bytes: 3,
        complete: true,
    };
    target
        .apply_baseline_page(
            &plan,
            &SourcePage {
                rows: vec![VersionedRow {
                    stable_key: "a".into(),
                    source_version: 0,
                    payload: b"one".to_vec(),
                }],
                next_cursor: Some("a".into()),
                complete: true,
            },
            &baseline_checkpoint,
        )
        .await?;
    let replay = CapturedChange::from_input(
        &plan.migration_id,
        "sqlite-test",
        1,
        change("events", "a", ChangeKind::Update, b"two"),
    )?;
    target
        .apply_change_batch(
            &plan.migration_id,
            "sqlite-test",
            &[replay],
            &EngineCheckpoint {
                captured_through: 1,
                applied_through: 1,
                ..baseline_checkpoint.clone()
            },
        )
        .await?;
    let rows = target
        .rows_in_range("events", &ChunkRange::new("a", "a")?)
        .await?;
    assert_eq!(rows[0].source_version, 1);
    assert_eq!(rows[0].payload, b"two");

    let proof = TableCoverageProof {
        migration_id: plan.migration_id.clone(),
        table_name: plan.table_name.clone(),
        source_watermark: 1,
        range: Some(ChunkRange::new("a", "a")?),
        source_lower_bound: Some("a".into()),
        source_upper_bound: Some("a".into()),
        target_lower_bound: Some("a".into()),
        target_upper_bound: Some("a".into()),
        source_row_count: 1,
        target_row_count: 1,
        source_byte_count: 3,
        target_byte_count: 3,
        source_digest: "digest".into(),
        target_digest: "digest".into(),
    };
    target.save_table_coverage_proof(&proof).await?;
    assert_eq!(
        target
            .load_table_coverage_proof(&plan.migration_id, &plan.table_name)
            .await?,
        Some(proof)
    );

    for table in [
        target_table,
        format!("{prefix}_plans"),
        format!("{prefix}_checkpoints"),
        format!("{prefix}_chunk_proofs"),
        format!("{prefix}_coverage_proofs"),
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS \"{table}\""))
            .execute(&pool)
            .await?;
    }
    Ok(())
}
