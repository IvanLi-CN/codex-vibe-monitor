use std::{
    fs,
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use sqlx::{PgPool, postgres::PgPoolOptions, sqlite::SqlitePoolOptions};

use crate::storage::{
    core::{
        AuthorityRecoveryContext, BackendKind, MigrationCheckpoint, MigrationMetadataStore,
        MigrationOutboxItem, ModuleAuthority, ModuleAuthorityManifest, ModuleGenerationGate,
        StorageModule,
    },
    migration::{
        CapturedChange, ChangeBatchLimits, ChangeInput, ChangeKind, ChunkProof, ChunkRange,
        CopyLimits, CoverageCheckpoint, DEFAULT_COPY_MAX_BYTES, DEFAULT_COPY_MAX_ROWS,
        EngineCheckpoint, FileMigrationStore, InMemoryMigrationSource, InMemoryMigrationTarget,
        MigrationEngine, MigrationLimits, MigrationSource, MigrationTarget, SourcePage,
        SourceWriteFence, TableCopyPlan, TableCoverageProof, VersionCapture, VersionedRow,
        validate_contiguous_change_prefix,
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

fn replace_change(
    table_name: &str,
    previous_key: &str,
    stable_key: &str,
    payload: &[u8],
) -> ChangeInput {
    ChangeInput {
        table_name: table_name.into(),
        stable_key: stable_key.into(),
        previous_key: Some(previous_key.into()),
        kind: ChangeKind::Replace,
        payload: Some(payload.to_vec()),
    }
}

fn replay_limits(max_rows: usize, max_bytes: usize) -> ChangeBatchLimits {
    ChangeBatchLimits {
        max_rows,
        max_bytes,
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

async fn cleanup_postgres_migration_fixture(
    pool: &PgPool,
    prefix: &str,
    target_table: &str,
) -> Result<()> {
    for table in [
        target_table.to_owned(),
        format!("{prefix}_plans"),
        format!("{prefix}_checkpoints"),
        format!("{prefix}_chunk_proofs"),
        format!("{prefix}_coverage_checkpoints"),
        format!("{prefix}_coverage_proofs"),
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS \"{table}\""))
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn with_postgres_migration_fixture_cleanup<F>(
    pool: &PgPool,
    prefix: &str,
    target_table: &str,
    operation: F,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    let result = operation.await;
    let cleanup = cleanup_postgres_migration_fixture(pool, prefix, target_table).await;
    result?;
    cleanup
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
async fn file_migration_store_rejects_stale_recovery_marker_and_consumes_valid_marker() -> Result<()>
{
    let (directory, path) = state_path("recovery-marker");
    let context = AuthorityRecoveryContext {
        migration_id: "migration-engine-test".into(),
        installation_id: "installation-test".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite-test".into(),
        target_identity: "postgres-test".into(),
        source_watermark: 2,
    };
    let stale = AuthorityRecoveryContext {
        source_watermark: 1,
        ..context.clone()
    };
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
    let store = FileMigrationStore::new(&path, context.clone());
    store.set_recovery_context(&stale).await?;
    assert_eq!(
        store
            .publish_authority_manifest(&manifest, &context)
            .await?,
        crate::storage::core::AuthorityPublication::Conflict
    );
    assert!(matches!(
        store
            .load_authority_manifest(StorageModule::TrafficArchive)
            .await?,
        crate::storage::core::AuthorityRecoveryRecord::NeverMigrated {
            source_watermark: 1,
            ..
        }
    ));

    fs::remove_dir_all(&directory)?;
    let (directory, path) = state_path("recovery-marker-consume");
    let store = FileMigrationStore::new(&path, context.clone());
    store.set_recovery_context(&context).await?;
    assert_eq!(
        store
            .publish_authority_manifest(&manifest, &context)
            .await?,
        crate::storage::core::AuthorityPublication::Committed
    );
    let stored: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    assert!(
        stored
            .get("recovery_context")
            .is_some_and(serde_json::Value::is_null)
    );
    assert!(matches!(
        store
            .load_authority_manifest(StorageModule::TrafficArchive)
            .await?,
        crate::storage::core::AuthorityRecoveryRecord::Manifest(_)
    ));
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn migration_engine_requires_explicit_baseline_completion_at_every_stage() -> Result<()> {
    let (directory, path) = state_path("baseline-guard");
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
    source.start_new_capture_generation().await?;
    assert!(migration.prepare(&["events".into()]).await.is_err());
    assert!(migration.copy_next("events").await.is_err());
    assert!(migration.catch_up("events").await.is_err());
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    assert!(
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
            .is_err()
    );
    migration.mark_baseline_complete(1).await?;
    assert!(!source.capture_checkpoint().await?.baseline_required);
    assert!(migration.copy_next("events").await?.complete);
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
fn migration_replay_rejects_non_contiguous_watermark_prefixes() -> Result<()> {
    let first = CapturedChange::from_input(
        "migration-engine-test",
        "sqlite-test",
        1,
        change("events", "a", ChangeKind::Insert, b"a"),
    )?;
    let gap = CapturedChange::from_input(
        "migration-engine-test",
        "sqlite-test",
        3,
        change("events", "c", ChangeKind::Insert, b"c"),
    )?;
    assert!(validate_contiguous_change_prefix(&[first, gap], 0, 3).is_err());
    Ok(())
}

#[tokio::test]
async fn migration_target_persistence_rejects_identity_reuse() -> Result<()> {
    let target = InMemoryMigrationTarget::default();
    let plan = TableCopyPlan {
        migration_id: "migration-engine-test".into(),
        table_name: "events".into(),
        installation_id: "installation-a".into(),
        source_identity: "sqlite-a".into(),
        target_identity: "postgres-a".into(),
        scan_lower_bound: Some("a".into()),
        scan_upper_bound: Some("a".into()),
        capture_start: 0,
    };
    target.save_plan(&plan).await?;
    let conflicting_plan = TableCopyPlan {
        target_identity: "postgres-b".into(),
        ..plan.clone()
    };
    assert!(target.save_plan(&conflicting_plan).await.is_err());

    let coverage = CoverageCheckpoint {
        migration_id: plan.migration_id.clone(),
        table_name: plan.table_name.clone(),
        installation_id: plan.installation_id.clone(),
        source_identity: plan.source_identity.clone(),
        target_identity: plan.target_identity.clone(),
        lower_bound: Some("a".into()),
        upper_bound: Some("a".into()),
        cursor: Some("a".into()),
        source_watermark: 0,
        source_row_count: 1,
        target_row_count: 1,
        source_byte_count: 1,
        target_byte_count: 1,
        source_digest: "source".into(),
        target_digest: "target".into(),
        complete: true,
    };
    target.save_coverage_checkpoint(&coverage).await?;
    assert!(
        target
            .save_coverage_checkpoint(&CoverageCheckpoint {
                source_identity: "sqlite-b".into(),
                ..coverage.clone()
            })
            .await
            .is_err()
    );

    let proof = ChunkProof {
        migration_id: plan.migration_id.clone(),
        table_name: plan.table_name.clone(),
        installation_id: plan.installation_id.clone(),
        source_identity: plan.source_identity.clone(),
        target_identity: plan.target_identity.clone(),
        range: ChunkRange::new("a", "a")?,
        source_watermark: 0,
        row_count: 1,
        byte_count: 1,
        digest: "digest".into(),
    };
    target.save_chunk_proof(&proof).await?;
    assert!(
        target
            .save_chunk_proof(&ChunkProof {
                installation_id: "installation-b".into(),
                ..proof
            })
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn file_migration_store_rejects_checkpoint_identity_reuse_and_outbox_payload_conflict()
-> Result<()> {
    let (directory, path) = state_path("identity-reuse");
    let context = AuthorityRecoveryContext {
        migration_id: "migration-engine-test".into(),
        installation_id: "installation-a".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite-a".into(),
        target_identity: "postgres-a".into(),
        source_watermark: 0,
    };
    let store = FileMigrationStore::new(&path, context.clone());
    let checkpoint = MigrationCheckpoint {
        migration_id: context.migration_id.clone(),
        table_name: "events".into(),
        stable_key: "a".into(),
        cursor: Some("a".into()),
        captured_through: Some(0),
        applied_through: Some(0),
        installation_id: context.installation_id.clone(),
        source_identity: context.source_identity.clone(),
        target_identity: context.target_identity.clone(),
        authority_generation: 1,
    };
    store.save_checkpoint(&checkpoint).await?;
    assert!(
        store
            .save_checkpoint(&MigrationCheckpoint {
                installation_id: "installation-b".into(),
                ..checkpoint
            })
            .await
            .is_err()
    );
    let item = MigrationOutboxItem {
        outbox_id: "outbox-1".into(),
        migration_id: context.migration_id.clone(),
        source_identity: context.source_identity.clone(),
        source_version: 1,
        module: context.module,
        authority_generation: 1,
        payload_digest: "digest-a".into(),
    };
    assert!(store.enqueue_outbox(&item).await?);
    assert!(!store.enqueue_outbox(&item).await?);
    assert!(
        store
            .enqueue_outbox(&MigrationOutboxItem {
                payload_digest: "digest-b".into(),
                ..item
            })
            .await
            .is_err()
    );
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
async fn migration_switch_keeps_source_fenced_after_indeterminate_publication() -> Result<()> {
    let (directory, path) = state_path("authority-indeterminate");
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
    assert!(migration.copy_next("events").await?.complete);

    store.fail_next_authority_publication();
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    assert!(
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
            .is_err()
    );
    assert!(gate.requires_recovery());
    assert!(fence.is_fenced().await);
    assert!(
        source
            .write(
                &fence,
                vec![change("events", "b", ChangeKind::Insert, b"blocked")],
            )
            .await
            .is_err()
    );

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn migration_switch_serializes_concurrent_authority_attempts() -> Result<()> {
    let (directory, path) = state_path("authority-concurrent");
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
    assert!(migration.copy_next("events").await?.complete);

    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let expected = AuthorityRecoveryContext {
        migration_id: "migration-engine-test".into(),
        installation_id: "installation-test".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite-test".into(),
        target_identity: "postgres-test".into(),
        source_watermark: 1,
    };
    let first = tokio::spawn({
        let migration = migration.clone();
        let gate = gate.clone();
        let expected = expected.clone();
        async move {
            migration
                .switch_authority(&gate, expected, Duration::from_secs(1))
                .await
        }
    });
    let second = tokio::spawn({
        let migration = migration.clone();
        let gate = gate.clone();
        async move {
            migration
                .switch_authority(&gate, expected, Duration::from_secs(1))
                .await
        }
    });
    let first = first.await?;
    let second = second.await?;

    assert!(first.is_ok() ^ second.is_ok());
    assert_eq!(
        gate.current_authority().await.backend,
        BackendKind::Postgres
    );
    assert!(fence.is_fenced().await);
    assert!(
        source
            .write(
                &fence,
                vec![change("events", "b", ChangeKind::Insert, b"blocked")],
            )
            .await
            .is_err()
    );

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn migration_switch_abort_does_not_leave_source_fenced() -> Result<()> {
    let (directory, path) = state_path("authority-cancelled");
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
    assert!(migration.copy_next("events").await?.complete);

    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    let active_write = fence.begin_write().await?;
    let waiter = tokio::spawn({
        let migration = migration.clone();
        let gate = gate.clone();
        async move {
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
                    Duration::from_millis(50),
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if fence.is_fenced().await {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;
    waiter.abort();
    assert!(
        waiter
            .await
            .expect_err("switch waiter should be aborted")
            .is_cancelled()
    );

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if !fence.is_fenced().await {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;
    drop(active_write);
    assert_eq!(gate.current_authority().await.backend, BackendKind::Sqlite);
    source
        .write(
            &fence,
            vec![change("events", "b", ChangeKind::Insert, b"after-cancel")],
        )
        .await?;

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
async fn migration_copy_progresses_after_fixed_upper_bound_delete_or_replace() -> Result<()> {
    let (directory, path) = state_path("copy-upper-bound-delete");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![
                change("events", "a", ChangeKind::Insert, b"a"),
                change("events", "b", ChangeKind::Insert, b"b"),
                change("events", "c", ChangeKind::Insert, b"c"),
            ],
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
    assert!(!migration.copy_next("events").await?.complete);
    source
        .write(&fence, vec![change("events", "c", ChangeKind::Delete, b"")])
        .await?;
    assert!(migration.copy_next("events").await?.complete);
    let delete_watermark = source.capture_watermark().await?;
    migration.catch_up("events").await?;
    let delete_proof = migration
        .verify_table_coverage_at("events", delete_watermark)
        .await?;
    assert_eq!(delete_proof.source_digest, delete_proof.target_digest);
    assert!(target.row("events", "c").await.is_none());
    fs::remove_dir_all(directory)?;

    let (directory, path) = state_path("copy-upper-bound-replace");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![
                change("events", "a", ChangeKind::Insert, b"a"),
                change("events", "b", ChangeKind::Insert, b"b"),
                change("events", "c", ChangeKind::Insert, b"c"),
            ],
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
    assert!(!migration.copy_next("events").await?.complete);
    source
        .write(&fence, vec![replace_change("events", "c", "d", b"d")])
        .await?;
    assert!(migration.copy_next("events").await?.complete);
    let replace_watermark = source.capture_watermark().await?;
    migration.catch_up("events").await?;
    let replace_proof = migration
        .verify_table_coverage_at("events", replace_watermark)
        .await?;
    assert_eq!(replace_proof.source_digest, replace_proof.target_digest);
    assert!(target.row("events", "c").await.is_none());
    assert_eq!(
        target
            .row("events", "d")
            .await
            .expect("replacement row")
            .payload,
        b"d"
    );
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn migration_prepare_reuses_durable_plans_after_partial_restart() -> Result<()> {
    let (directory, path) = state_path("prepare-restart");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    source.register_module_table("events").await?;
    source.register_module_table("accounts").await?;
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![change("events", "a", ChangeKind::Insert, b"before")],
        )
        .await?;
    let target = Arc::new(InMemoryMigrationTarget::default());
    let context = AuthorityRecoveryContext {
        migration_id: "migration-engine-test".into(),
        installation_id: "installation-test".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite-test".into(),
        target_identity: "postgres-test".into(),
        source_watermark: 1,
    };
    let store = Arc::new(FileMigrationStore::new(&path, context.clone()));
    let migration = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        fence.clone(),
    );
    let capture_start = source.capture_watermark().await?;
    let (scan_lower_bound, scan_upper_bound) = source.table_bounds("events").await?;
    let events_plan = TableCopyPlan {
        migration_id: source.migration_id().to_owned(),
        table_name: "events".into(),
        installation_id: "installation-test".into(),
        source_identity: source.source_identity().to_owned(),
        target_identity: "postgres-test".into(),
        scan_lower_bound,
        scan_upper_bound,
        capture_start,
    };
    target.save_plan(&events_plan).await?;
    source
        .write(
            &fence,
            vec![change("events", "a", ChangeKind::Update, b"after")],
        )
        .await?;
    target.fail_next_operation().await;
    assert!(
        migration
            .prepare(&["events".into(), "accounts".into()])
            .await
            .is_err()
    );

    let restart_store = Arc::new(FileMigrationStore::new(&path, context));
    let restarted = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&restart_store),
        fence,
    );
    restarted
        .prepare(&["events".into(), "accounts".into()])
        .await?;
    let restarted_events = target
        .load_plan("migration-engine-test", "events")
        .await?
        .expect("durable events plan");
    let restarted_accounts = target
        .load_plan("migration-engine-test", "accounts")
        .await?
        .expect("restarted accounts plan");
    assert_eq!(restarted_events, events_plan);
    assert_eq!(restarted_accounts.capture_start, capture_start);
    assert_eq!(source.capture_watermark().await?, 2);
    assert!(restarted.copy_next("events").await?.complete);
    assert!(restarted.copy_next("accounts").await?.complete);
    assert_eq!(
        restarted.catch_up("events").await?.applied_through,
        source.capture_watermark().await?
    );

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn switch_catches_up_source_writes_during_large_pre_fence_verification() -> Result<()> {
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
    switching
        .await
        .expect("authority switch task")
        .expect("authority switch should include the bounded tail delta");
    assert!(fence.is_fenced().await);
    assert_eq!(
        gate.current_authority().await.backend,
        BackendKind::Postgres
    );
    assert_eq!(
        target.row("events", "z").await.expect("tail row").payload,
        b"late"
    );
    let checkpoint = target
        .load_coverage_checkpoint("migration-engine-test", "events")
        .await?
        .expect("coverage checkpoint");
    assert!(checkpoint.complete);
    assert_eq!(checkpoint.source_row_count, 8);

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn migration_engine_replays_concurrent_change_kinds_and_same_version_boundary() -> Result<()>
{
    let (directory, path) = state_path("a2-change-boundaries");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![
                change("events", "update", ChangeKind::Insert, b"old-update"),
                change("events", "delete", ChangeKind::Insert, b"old-delete"),
                change("events", "replace-old", ChangeKind::Insert, b"old-replace"),
                change(
                    "events",
                    "boundary-update",
                    ChangeKind::Insert,
                    b"old-boundary-update",
                ),
                change(
                    "events",
                    "boundary-delete",
                    ChangeKind::Insert,
                    b"old-boundary-delete",
                ),
                change(
                    "events",
                    "boundary-replace-old",
                    ChangeKind::Insert,
                    b"old-boundary-replace",
                ),
            ],
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
            max_queued_operations: 4,
            copy: CopyLimits {
                max_rows: 2,
                max_bytes: 1024,
                source_read_budget: Duration::from_secs(1),
                batch_yield: Duration::ZERO,
            },
            ..MigrationLimits::default()
        },
        fence.clone(),
    )?;
    migration.prepare(&["events".into()]).await?;
    while !migration.copy_next("events").await?.complete {}

    let (inserted, updated, deleted, replaced) = tokio::join!(
        source.write(
            &fence,
            vec![change(
                "events",
                "inserted",
                ChangeKind::Insert,
                b"new-insert"
            )],
        ),
        source.write(
            &fence,
            vec![change(
                "events",
                "update",
                ChangeKind::Update,
                b"new-update"
            )],
        ),
        source.write(
            &fence,
            vec![change("events", "delete", ChangeKind::Delete, b"")],
        ),
        source.write(
            &fence,
            vec![replace_change(
                "events",
                "replace-old",
                "replace-new",
                b"new-replace",
            )],
        ),
    );
    let concurrent_versions = [inserted?, updated?, deleted?, replaced?];
    assert!(
        concurrent_versions
            .iter()
            .all(|version| (2..=5).contains(version))
    );
    assert_eq!(source.capture_watermark().await?, 5);
    migration.catch_up("events").await?;

    let same_version = source
        .write(
            &fence,
            vec![
                change(
                    "events",
                    "boundary-insert",
                    ChangeKind::Insert,
                    b"new-boundary-insert",
                ),
                change(
                    "events",
                    "boundary-update",
                    ChangeKind::Update,
                    b"new-boundary-update",
                ),
                change("events", "boundary-delete", ChangeKind::Delete, b""),
                replace_change(
                    "events",
                    "boundary-replace-old",
                    "boundary-replace-new",
                    b"new-boundary-replace",
                ),
            ],
        )
        .await?;
    assert_eq!(same_version, 6);
    assert_eq!(migration.catch_up("events").await?.applied_through, 6);

    for (key, version, payload) in [
        ("inserted", 2, b"new-insert".as_slice()),
        ("update", 3, b"new-update".as_slice()),
        ("replace-new", 5, b"new-replace".as_slice()),
        ("boundary-insert", 6, b"new-boundary-insert".as_slice()),
        ("boundary-update", 6, b"new-boundary-update".as_slice()),
        (
            "boundary-replace-new",
            6,
            b"new-boundary-replace".as_slice(),
        ),
    ] {
        let row = target
            .row("events", key)
            .await
            .expect("applied migration row");
        assert_eq!(row.source_version, version);
        assert_eq!(row.payload, payload);
    }
    for key in [
        "delete",
        "replace-old",
        "boundary-delete",
        "boundary-replace-old",
    ] {
        assert!(target.row("events", key).await.is_none(), "stale row {key}");
    }

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn in_memory_capture_returns_complete_same_version_boundary_within_budget() -> Result<()> {
    let source = InMemoryMigrationSource::new("migration-engine-test", "sqlite-test");
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![
                change("events", "a", ChangeKind::Insert, b"a"),
                change("events", "b", ChangeKind::Insert, b"b"),
                change("events", "c", ChangeKind::Insert, b"c"),
            ],
        )
        .await?;

    let changes = <InMemoryMigrationSource as MigrationSource>::changes_through(
        &source,
        0,
        1,
        replay_limits(3, DEFAULT_COPY_MAX_BYTES),
    )
    .await?;
    assert_eq!(changes.len(), 3);
    assert!(changes.iter().all(|change| change.source_version == 1));
    assert_eq!(
        changes
            .iter()
            .map(|change| change.stable_key.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    Ok(())
}

#[tokio::test]
async fn in_memory_capture_enforces_copy_batch_row_and_byte_boundaries() -> Result<()> {
    let source = InMemoryMigrationSource::new("migration-engine-test", "sqlite-test");
    let fence = SourceWriteFence::new(1);
    let row_changes = (0..DEFAULT_COPY_MAX_ROWS)
        .map(|index| {
            change(
                "events",
                &format!("row-{index:03}"),
                ChangeKind::Insert,
                b"row",
            )
        })
        .collect();
    source.write(&fence, row_changes).await?;

    let exact_payload = vec![0_u8; DEFAULT_COPY_MAX_BYTES];
    source
        .write(
            &fence,
            vec![change(
                "events",
                "exact-byte-boundary",
                ChangeKind::Insert,
                &exact_payload,
            )],
        )
        .await?;

    let row_boundary = source
        .changes_through(
            0,
            2,
            replay_limits(DEFAULT_COPY_MAX_ROWS, DEFAULT_COPY_MAX_BYTES),
        )
        .await?;
    assert_eq!(row_boundary.len(), DEFAULT_COPY_MAX_ROWS);
    assert!(row_boundary.iter().all(|change| change.source_version == 1));

    let byte_boundary = source
        .changes_through(
            1,
            2,
            replay_limits(DEFAULT_COPY_MAX_ROWS, DEFAULT_COPY_MAX_BYTES),
        )
        .await?;
    assert_eq!(byte_boundary.len(), 1);
    assert_eq!(
        byte_boundary[0].payload.as_deref(),
        Some(exact_payload.as_slice())
    );

    let oversized_payload = vec![0_u8; DEFAULT_COPY_MAX_BYTES + 1];
    source
        .write(
            &fence,
            vec![change(
                "events",
                "oversized-byte-boundary",
                ChangeKind::Insert,
                &oversized_payload,
            )],
        )
        .await?;
    let error = source
        .changes_through(
            2,
            3,
            replay_limits(DEFAULT_COPY_MAX_ROWS, DEFAULT_COPY_MAX_BYTES),
        )
        .await
        .expect_err("a source transaction over 2 MiB must be rejected");
    assert!(error.to_string().contains("bounded change byte budget"));
    Ok(())
}

#[tokio::test]
async fn version_capture_is_bounded_and_requires_a_new_baseline_after_loss() -> Result<()> {
    let capture =
        VersionCapture::with_max_capture_bytes("migration-engine-test", "sqlite-test", 3)?;
    assert_eq!(
        capture
            .capture_transaction(vec![change("events", "a", ChangeKind::Insert, b"one")])
            .await?,
        1
    );
    let error = capture
        .capture_transaction(vec![change("events", "b", ChangeKind::Insert, b"two")])
        .await
        .expect_err("capture must reject writes beyond its durable capacity");
    assert!(error.to_string().contains("bounded storage capacity"));
    let checkpoint = capture.checkpoint().await?;
    assert_eq!(checkpoint.captured_bytes, 3);
    assert_eq!(checkpoint.next_version, 1);

    capture.mark_continuity_lost().await;
    let checkpoint = capture.checkpoint().await?;
    assert!(checkpoint.continuity_lost);
    assert!(checkpoint.baseline_required);
    assert!(capture.changes_after(0, 1).await.is_err());

    let new_generation = capture.start_new_capture_generation().await?;
    assert_eq!(new_generation.capture_generation, 2);
    assert_eq!(new_generation.next_version, 1);
    assert_eq!(new_generation.captured_bytes, 0);
    assert!(new_generation.baseline_required);
    capture
        .capture_transaction(vec![change("events", "b", ChangeKind::Insert, b"two")])
        .await?;
    capture.mark_baseline_complete(2, 2).await?;
    assert!(!capture.checkpoint().await?.baseline_required);
    Ok(())
}

#[tokio::test]
async fn in_memory_capture_rejects_transaction_exceeding_row_budget() -> Result<()> {
    let source = InMemoryMigrationSource::new("migration-engine-test", "sqlite-test");
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![
                change("events", "a", ChangeKind::Insert, b"a"),
                change("events", "b", ChangeKind::Insert, b"b"),
                change("events", "c", ChangeKind::Insert, b"c"),
            ],
        )
        .await?;

    let error = <InMemoryMigrationSource as MigrationSource>::changes_through(
        &source,
        0,
        1,
        replay_limits(2, DEFAULT_COPY_MAX_BYTES),
    )
    .await
    .expect_err("same-version capture must respect the row budget");
    assert!(error.to_string().contains("bounded change row budget"));
    Ok(())
}

#[tokio::test]
async fn in_memory_capture_rejects_transaction_exceeding_byte_budget() -> Result<()> {
    let source = InMemoryMigrationSource::new("migration-engine-test", "sqlite-test");
    let fence = SourceWriteFence::new(1);
    let payload = vec![0_u8; DEFAULT_COPY_MAX_BYTES + 1];
    source
        .write(
            &fence,
            vec![change("events", "large", ChangeKind::Insert, &payload)],
        )
        .await?;

    let error = <InMemoryMigrationSource as MigrationSource>::changes_through(
        &source,
        0,
        1,
        replay_limits(2, DEFAULT_COPY_MAX_BYTES),
    )
    .await
    .expect_err("oversized capture must respect the byte budget");
    assert!(error.to_string().contains("bounded change byte budget"));
    Ok(())
}

#[tokio::test]
async fn migration_engine_restarts_after_apply_ack_failure_without_loss_or_resurrection()
-> Result<()> {
    let (directory, path) = state_path("a2-ack-restart");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![
                change("events", "updated", ChangeKind::Insert, b"old"),
                change("events", "deleted", ChangeKind::Insert, b"remove"),
                change("events", "replace-old", ChangeKind::Insert, b"before"),
            ],
        )
        .await?;
    let target = Arc::new(InMemoryMigrationTarget::default());
    let context = AuthorityRecoveryContext {
        migration_id: "migration-engine-test".into(),
        installation_id: "installation-test".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite-test".into(),
        target_identity: "postgres-test".into(),
        source_watermark: 1,
    };
    let store = Arc::new(FileMigrationStore::new(&path, context.clone()));
    let migration = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        fence.clone(),
    );
    migration.prepare(&["events".into()]).await?;
    while !migration.copy_next("events").await?.complete {}

    source
        .write(
            &fence,
            vec![
                change("events", "updated", ChangeKind::Update, b"after"),
                change("events", "deleted", ChangeKind::Delete, b""),
                replace_change("events", "replace-old", "replace-new", b"replaced"),
            ],
        )
        .await?;
    target.fail_after_apply_for_test().await;
    assert!(migration.catch_up("events").await.is_err());
    assert_eq!(
        target
            .load_checkpoint("migration-engine-test", "events")
            .await?
            .expect("target checkpoint after committed apply")
            .applied_through,
        2
    );

    let restart_store = Arc::new(FileMigrationStore::new(&path, context));
    let restarted = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&restart_store),
        fence,
    );
    assert_eq!(restarted.catch_up("events").await?.applied_through, 2);
    assert_eq!(
        restart_store
            .load_checkpoint("migration-engine-test", "events")
            .await?
            .expect("restarted metadata checkpoint")
            .applied_through,
        Some(2)
    );
    let updated = target.row("events", "updated").await.expect("updated row");
    assert_eq!(updated.source_version, 2);
    assert_eq!(updated.payload, b"after");
    assert!(target.row("events", "deleted").await.is_none());
    assert!(target.row("events", "replace-old").await.is_none());
    let replaced = target
        .row("events", "replace-new")
        .await
        .expect("replacement row");
    assert_eq!(replaced.source_version, 2);
    assert_eq!(replaced.payload, b"replaced");

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn table_coverage_proof_rejects_extra_rows_empty_to_nonempty_and_bound_changes() -> Result<()>
{
    let (directory, path) = state_path("coverage-extra-row");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    source.register_module_table("events").await?;
    let target = Arc::new(InMemoryMigrationTarget::default());
    target
        .seed_row_for_test("events", "orphan", 1, b"orphan")
        .await;
    let migration = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::new(FileMigrationStore::new(
            &path,
            AuthorityRecoveryContext {
                migration_id: "migration-engine-test".into(),
                installation_id: "installation-test".into(),
                module: StorageModule::TrafficArchive,
                source_identity: "sqlite-test".into(),
                target_identity: "postgres-test".into(),
                source_watermark: 0,
            },
        )),
        SourceWriteFence::new(1),
    );
    assert!(
        migration
            .verify_table_coverage_at("events", 0)
            .await
            .is_err()
    );
    if directory.exists() {
        fs::remove_dir_all(directory)?;
    }

    let (directory, path) = state_path("coverage-empty-transition");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    source.register_module_table("events").await?;
    let target = Arc::new(InMemoryMigrationTarget::default());
    let fence = SourceWriteFence::new(1);
    let migration = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::new(FileMigrationStore::new(
            &path,
            AuthorityRecoveryContext {
                migration_id: "migration-engine-test".into(),
                installation_id: "installation-test".into(),
                module: StorageModule::TrafficArchive,
                source_identity: "sqlite-test".into(),
                target_identity: "postgres-test".into(),
                source_watermark: 0,
            },
        )),
        fence.clone(),
    );
    let empty_proof = migration.verify_table_coverage_at("events", 0).await?;
    assert!(empty_proof.range.is_none());
    source
        .write(
            &fence,
            vec![change("events", "a", ChangeKind::Insert, b"now-present")],
        )
        .await?;
    assert!(
        migration
            .verify_table_coverage_at("events", 1)
            .await
            .is_err()
    );
    if directory.exists() {
        fs::remove_dir_all(directory)?;
    }

    let (directory, path) = state_path("coverage-boundary-change");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    let target = Arc::new(InMemoryMigrationTarget::default());
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![
                change("events", "b", ChangeKind::Insert, b"b"),
                change("events", "c", ChangeKind::Insert, b"c"),
            ],
        )
        .await?;
    target.seed_row_for_test("events", "b", 1, b"b").await;
    target.seed_row_for_test("events", "c", 1, b"c").await;
    let migration = engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::new(FileMigrationStore::new(
            &path,
            AuthorityRecoveryContext {
                migration_id: "migration-engine-test".into(),
                installation_id: "installation-test".into(),
                module: StorageModule::TrafficArchive,
                source_identity: "sqlite-test".into(),
                target_identity: "postgres-test".into(),
                source_watermark: 1,
            },
        )),
        fence.clone(),
    );
    let initial_proof = migration.verify_table_coverage_at("events", 1).await?;
    assert_eq!(initial_proof.source_lower_bound.as_deref(), Some("b"));
    assert_eq!(initial_proof.source_upper_bound.as_deref(), Some("c"));
    source
        .write(
            &fence,
            vec![
                change("events", "a", ChangeKind::Insert, b"a"),
                change("events", "d", ChangeKind::Insert, b"d"),
            ],
        )
        .await?;
    assert!(
        migration
            .verify_table_coverage_at("events", 2)
            .await
            .is_err()
    );
    if directory.exists() {
        fs::remove_dir_all(directory)?;
    }
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
    sqlx::query("DELETE FROM events WHERE stable_key = ?")
        .bind("b")
        .execute(&pool)
        .await?;
    let deleted_upper_page = source
        .scan_page(
            "events",
            Some("a"),
            Some("b"),
            CopyLimits {
                max_rows: 1,
                max_bytes: 3,
                source_read_budget: Duration::from_secs(1),
                batch_yield: Duration::ZERO,
            },
        )
        .await?;
    assert!(deleted_upper_page.rows.is_empty());
    assert!(deleted_upper_page.complete);
    Ok(())
}

#[tokio::test]
async fn sqlite_migration_source_uses_binary_key_collation_for_bounds_and_pages() -> Result<()> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    sqlx::query(
        "CREATE TABLE events (
            stable_key TEXT COLLATE NOCASE PRIMARY KEY,
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
    for (key, payload) in [("a", b"a".as_slice()), ("B", b"B".as_slice())] {
        sqlx::query("INSERT INTO events (stable_key, source_version, payload) VALUES (?, ?, ?)")
            .bind(key)
            .bind(1_i64)
            .bind(payload)
            .execute(&pool)
            .await?;
    }

    let (lower_bound, upper_bound) = source.table_bounds("events").await?;
    assert_eq!(lower_bound.as_deref(), Some("B"));
    assert_eq!(upper_bound.as_deref(), Some("a"));
    let page = source
        .scan_page(
            "events",
            None,
            Some("a"),
            CopyLimits {
                max_rows: 2,
                max_bytes: 2,
                source_read_budget: Duration::from_secs(1),
                batch_yield: Duration::ZERO,
            },
        )
        .await?;
    assert_eq!(
        page.rows
            .iter()
            .map(|row| row.stable_key.as_str())
            .collect::<Vec<_>>(),
        vec!["B", "a"]
    );
    assert!(page.complete);
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
            max_queued_operations: 4,
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

    assert_eq!(
        source
            .changes_through(0, 1, replay_limits(4, DEFAULT_COPY_MAX_BYTES))
            .await?
            .len(),
        3
    );
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
async fn sqlite_capture_rejects_corrupt_digest_and_isolates_source_identity() -> Result<()> {
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
    let other = CapturedChange::from_input(
        "migration-engine-test",
        "another-source",
        1,
        change("events", "other", ChangeKind::Insert, b"other"),
    )?;
    sqlx::query(
        "INSERT INTO migration_change_log
            (migration_id, source_identity, source_version, table_name, stable_key,
             previous_key, kind, payload, payload_digest)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&other.migration_id)
    .bind(&other.source_identity)
    .bind(other.source_version)
    .bind(&other.table_name)
    .bind(&other.stable_key)
    .bind(&other.previous_key)
    .bind("insert")
    .bind(&other.payload)
    .bind(&other.payload_digest)
    .execute(&pool)
    .await?;
    sqlx::query(
        "UPDATE migration_change_log
            SET payload_digest = ?
          WHERE source_identity = ? AND source_version = ?",
    )
    .bind("corrupt-digest")
    .bind("sqlite-test")
    .bind(1_i64)
    .execute(&pool)
    .await?;
    assert!(
        source
            .changes_after(0, replay_limits(1, DEFAULT_COPY_MAX_BYTES))
            .await
            .is_err()
    );

    sqlx::query(
        "UPDATE migration_change_log
            SET payload_digest = ?
          WHERE source_identity = ? AND source_version = ?",
    )
    .bind(&first.payload_digest)
    .bind("sqlite-test")
    .bind(1_i64)
    .execute(&pool)
    .await?;
    assert_eq!(
        source
            .changes_after(0, replay_limits(1, DEFAULT_COPY_MAX_BYTES))
            .await?
            .len(),
        1
    );
    source
        .append_change(2, change("events", "b", ChangeKind::Insert, b"two"))
        .await?;
    sqlx::query(
        "UPDATE migration_change_log
            SET source_identity = ?
          WHERE source_identity = ? AND source_version = ?",
    )
    .bind("another-source")
    .bind("sqlite-test")
    .bind(2_i64)
    .execute(&pool)
    .await?;
    assert_eq!(
        source
            .changes_after(0, replay_limits(2, DEFAULT_COPY_MAX_BYTES))
            .await?
            .len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_capture_persists_continuity_loss_and_requires_a_new_generation() -> Result<()> {
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
    )?
    .with_max_capture_bytes(3)?;
    source.ensure_capture_schema().await?;
    source
        .append_change(1, change("events", "a", ChangeKind::Insert, b"one"))
        .await?;
    assert_eq!(source.capture_checkpoint().await?.captured_bytes, 3);

    let error = source
        .append_change(3, change("events", "gap", ChangeKind::Insert, b"gap"))
        .await
        .expect_err("a capture version gap must invalidate the attempt");
    assert!(error.to_string().contains("continuity"));
    let checkpoint = source.capture_checkpoint().await?;
    assert!(checkpoint.continuity_lost);
    assert!(checkpoint.baseline_required);

    let restarted = SqliteMigrationSource::new(
        pool.clone(),
        "migration-engine-test",
        "sqlite-test",
        "migration_change_log",
        vec![MigrationTableSpec::canonical(
            StorageModule::TrafficArchive,
            "events",
        )?],
    )?;
    restarted.ensure_capture_schema().await?;
    assert!(restarted.capture_checkpoint().await?.continuity_lost);
    assert!(
        restarted
            .changes_after(0, replay_limits(1, DEFAULT_COPY_MAX_BYTES))
            .await
            .is_err()
    );

    let new_generation = restarted.start_new_capture_generation().await?;
    assert_eq!(new_generation.capture_generation, 2);
    assert_eq!(new_generation.next_version, 1);
    assert!(new_generation.baseline_required);
    restarted
        .append_change_for_generation(1, 2, change("events", "b", ChangeKind::Insert, b"two"))
        .await?;
    restarted.mark_baseline_complete(2, 2).await?;
    let checkpoint = restarted.capture_checkpoint().await?;
    assert!(!checkpoint.continuity_lost);
    assert!(!checkpoint.baseline_required);
    assert_eq!(checkpoint.captured_bytes, 3);
    Ok(())
}

#[tokio::test]
async fn sqlite_capture_persists_source_fence_and_rejects_stale_generations() -> Result<()> {
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
    source
        .append_change(1, change("events", "a", ChangeKind::Insert, b"one"))
        .await?;
    source.fence_source_writes(1, 2).await?;
    assert_eq!(
        source.source_write_authority().await?,
        crate::storage::migration::SourceWriteAuthority {
            generation: 2,
            fenced: true,
        }
    );
    assert!(
        source
            .append_change(2, change("events", "stale", ChangeKind::Insert, b"stale"))
            .await
            .is_err()
    );

    let restarted = SqliteMigrationSource::new(
        pool.clone(),
        "migration-engine-test",
        "sqlite-test",
        "migration_change_log",
        vec![MigrationTableSpec::canonical(
            StorageModule::TrafficArchive,
            "events",
        )?],
    )?;
    restarted.ensure_capture_schema().await?;
    assert_eq!(
        restarted.source_write_authority().await?,
        crate::storage::migration::SourceWriteAuthority {
            generation: 2,
            fenced: true,
        }
    );
    assert!(
        restarted
            .append_change_for_generation(
                1,
                2,
                change("events", "stale-restarted", ChangeKind::Insert, b"stale")
            )
            .await
            .is_err()
    );

    let stale_reopen = restarted
        .reopen_source_writes(1)
        .await
        .expect_err("stale source generation must not reopen writes");
    assert!(stale_reopen.to_string().contains("stale"));

    // A failed pre-commit switch may explicitly reopen the source at the current generation;
    // the previous generation remains rejected even after that recovery path. Repeating the
    // same-generation reopen is intentionally idempotent.
    restarted.reopen_source_writes(2).await?;
    restarted.reopen_source_writes(2).await?;
    assert_eq!(
        restarted.source_write_authority().await?,
        crate::storage::migration::SourceWriteAuthority {
            generation: 2,
            fenced: false,
        }
    );

    let reopened_after_restart = SqliteMigrationSource::new(
        pool.clone(),
        "migration-engine-test",
        "sqlite-test",
        "migration_change_log",
        vec![MigrationTableSpec::canonical(
            StorageModule::TrafficArchive,
            "events",
        )?],
    )?;
    reopened_after_restart.ensure_capture_schema().await?;
    assert_eq!(
        reopened_after_restart.source_write_authority().await?,
        crate::storage::migration::SourceWriteAuthority {
            generation: 2,
            fenced: false,
        }
    );
    let stale_reopen_after_restart = reopened_after_restart
        .reopen_source_writes(1)
        .await
        .expect_err("stale source generation must remain rejected after restart");
    assert!(stale_reopen_after_restart.to_string().contains("stale"));
    reopened_after_restart.reopen_source_writes(2).await?;
    assert!(
        restarted
            .append_change_for_generation(
                1,
                2,
                change("events", "stale-reopened", ChangeKind::Insert, b"stale")
            )
            .await
            .is_err()
    );
    restarted
        .append_change_for_generation(2, 2, change("events", "b", ChangeKind::Insert, b"two"))
        .await?;
    Ok(())
}

#[tokio::test]
async fn sqlite_capture_rejects_oversized_same_version_transaction() -> Result<()> {
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
    for key in ["a", "b", "c"] {
        source
            .append_change(1, change("events", key, ChangeKind::Insert, key.as_bytes()))
            .await?;
    }
    let error = source
        .changes_through(0, 1, replay_limits(2, DEFAULT_COPY_MAX_BYTES))
        .await
        .expect_err("same-version capture must respect the row budget");
    assert!(error.to_string().contains("bounded change row budget"));
    Ok(())
}

#[tokio::test]
async fn sqlite_capture_enforces_copy_batch_row_and_byte_boundaries() -> Result<()> {
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

    let mut transaction = pool.begin().await?;
    for index in 0..DEFAULT_COPY_MAX_ROWS {
        let captured = CapturedChange::from_input(
            "migration-engine-test",
            "sqlite-test",
            1,
            change(
                "events",
                &format!("row-{index:03}"),
                ChangeKind::Insert,
                b"row",
            ),
        )?;
        source
            .append_captured_change_tx(&mut transaction, &captured)
            .await?;
    }
    transaction.commit().await?;

    let exact_payload = vec![0_u8; DEFAULT_COPY_MAX_BYTES];
    source
        .append_change(
            2,
            change(
                "events",
                "exact-byte-boundary",
                ChangeKind::Insert,
                &exact_payload,
            ),
        )
        .await?;

    let row_boundary = source
        .changes_through(
            0,
            2,
            replay_limits(DEFAULT_COPY_MAX_ROWS, DEFAULT_COPY_MAX_BYTES),
        )
        .await?;
    assert_eq!(row_boundary.len(), DEFAULT_COPY_MAX_ROWS);
    assert!(row_boundary.iter().all(|change| change.source_version == 1));

    let byte_boundary = source
        .changes_through(
            1,
            2,
            replay_limits(DEFAULT_COPY_MAX_ROWS, DEFAULT_COPY_MAX_BYTES),
        )
        .await?;
    assert_eq!(byte_boundary.len(), 1);
    assert_eq!(
        byte_boundary[0].payload.as_deref(),
        Some(exact_payload.as_slice())
    );

    source
        .append_change(
            3,
            change(
                "events",
                "oversized-byte-boundary",
                ChangeKind::Insert,
                &vec![0_u8; DEFAULT_COPY_MAX_BYTES + 1],
            ),
        )
        .await?;
    let error = source
        .changes_through(
            2,
            3,
            replay_limits(DEFAULT_COPY_MAX_ROWS, DEFAULT_COPY_MAX_BYTES),
        )
        .await
        .expect_err("a source transaction over 2 MiB must be rejected");
    assert!(error.to_string().contains("bounded change byte budget"));

    let restarted = SqliteMigrationSource::new(
        pool,
        "migration-engine-test",
        "sqlite-test",
        "migration_change_log",
        vec![MigrationTableSpec::canonical(
            StorageModule::TrafficArchive,
            "events",
        )?],
    )?;
    restarted.ensure_capture_schema().await?;
    assert_eq!(
        restarted
            .changes_through(
                0,
                2,
                replay_limits(DEFAULT_COPY_MAX_ROWS, DEFAULT_COPY_MAX_BYTES),
            )
            .await?
            .len(),
        DEFAULT_COPY_MAX_ROWS
    );
    Ok(())
}

#[tokio::test]
async fn sqlite_capture_oversized_batch_does_not_advance_checkpoint_across_retry() -> Result<()> {
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
    let (directory, path) = state_path("oversized-batch-retry");
    let context = AuthorityRecoveryContext {
        migration_id: "migration-engine-test".into(),
        installation_id: "installation-test".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite-test".into(),
        target_identity: "postgres-test".into(),
        source_watermark: 0,
    };
    let limits = MigrationLimits {
        change_batch: replay_limits(DEFAULT_COPY_MAX_ROWS, DEFAULT_COPY_MAX_BYTES),
        ..MigrationLimits::default()
    };
    let store = Arc::new(FileMigrationStore::new(&path, context.clone()));
    let migration = MigrationEngine::new(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        StorageModule::TrafficArchive,
        "installation-test",
        "postgres-test",
        1,
        limits,
        fence.clone(),
    )?;
    migration.prepare(&["events".into()]).await?;
    assert!(migration.copy_next("events").await?.complete);

    source
        .append_change(
            1,
            change(
                "events",
                "oversized",
                ChangeKind::Insert,
                &vec![0_u8; DEFAULT_COPY_MAX_BYTES + 1],
            ),
        )
        .await?;
    let first_error = migration
        .catch_up("events")
        .await
        .expect_err("an oversized source transaction must reject catch-up");
    assert!(
        first_error
            .to_string()
            .contains("bounded change byte budget")
    );
    assert_eq!(
        target
            .load_checkpoint("migration-engine-test", "events")
            .await?
            .expect("baseline checkpoint")
            .applied_through,
        0
    );

    let retry_error = migration
        .catch_up("events")
        .await
        .expect_err("retry must reject the same oversized source transaction");
    assert!(
        retry_error
            .to_string()
            .contains("bounded change byte budget")
    );
    assert_eq!(
        target
            .load_checkpoint("migration-engine-test", "events")
            .await?
            .expect("checkpoint after retry")
            .applied_through,
        0
    );

    let restarted_source = Arc::new(SqliteMigrationSource::new(
        pool,
        "migration-engine-test",
        "sqlite-test",
        "migration_change_log",
        vec![MigrationTableSpec::canonical(
            StorageModule::TrafficArchive,
            "events",
        )?],
    )?);
    restarted_source.ensure_capture_schema().await?;
    let restart_store = Arc::new(FileMigrationStore::new(&path, context));
    let restarted = MigrationEngine::new(
        Arc::clone(&restarted_source),
        Arc::clone(&target),
        Arc::clone(&restart_store),
        StorageModule::TrafficArchive,
        "installation-test",
        "postgres-test",
        1,
        limits,
        fence,
    )?;
    let restart_error = restarted
        .catch_up("events")
        .await
        .expect_err("restart must reject the same oversized source transaction");
    assert!(
        restart_error
            .to_string()
            .contains("bounded change byte budget")
    );
    assert_eq!(
        target
            .load_checkpoint("migration-engine-test", "events")
            .await?
            .expect("checkpoint after restart")
            .applied_through,
        0
    );

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn optional_postgres_migration_engine_uses_real_source_and_target_fixture() -> Result<()> {
    let Ok(database_url) = std::env::var("POSTGRES_TEST_DATABASE_URL") else {
        eprintln!(
            "skipping optional PostgreSQL migration engine test: POSTGRES_TEST_DATABASE_URL is unset"
        );
        return Ok(());
    };
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let prefix = format!("pg02_migration_engine_{suffix}");
    let target_table = format!("{prefix}_rows");
    let (directory, path) = state_path("postgres-real-engine");
    let result = with_postgres_migration_fixture_cleanup(&pool, &prefix, &target_table, async {
        let target = Arc::new(PostgresMigrationTarget::new(
            pool.clone(),
            &prefix,
            vec![MigrationTableSpec::new(
                StorageModule::TrafficArchive,
                "events",
                "events",
                target_table.clone(),
                "stable_key",
                "source_version",
                "payload",
                "stable_key",
                "source_version",
                "payload",
            )?],
        )?);
        target.ensure_schema().await?;

        let source_pool = SqlitePoolOptions::new()
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
        .execute(&source_pool)
        .await?;
        let source = Arc::new(SqliteMigrationSource::new(
            source_pool.clone(),
            "migration-engine-test",
            "sqlite-test",
            "migration_change_log",
            vec![MigrationTableSpec::new(
                StorageModule::TrafficArchive,
                "events",
                "events",
                target_table.clone(),
                "stable_key",
                "source_version",
                "payload",
                "stable_key",
                "source_version",
                "payload",
            )?],
        )?);
        source.ensure_capture_schema().await?;
        let mut source_transaction = source_pool.begin().await?;
        for (key, payload) in [
            ("delete-me", b"old-delete".as_slice()),
            ("replace-old", b"old-replace".as_slice()),
            ("update-me", b"old-update".as_slice()),
        ] {
            sqlx::query(
                "INSERT INTO events (stable_key, source_version, payload) VALUES (?, ?, ?)",
            )
            .bind(key)
            .bind(1_i64)
            .bind(payload)
            .execute(&mut *source_transaction)
            .await?;
            source
                .append_captured_change_tx(
                    &mut source_transaction,
                    &CapturedChange::from_input(
                        "migration-engine-test",
                        "sqlite-test",
                        1,
                        change("events", key, ChangeKind::Insert, payload),
                    )?,
                )
                .await?;
        }
        source_transaction.commit().await?;

        let fence = SourceWriteFence::new(1);
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
                max_queued_operations: 4,
                copy: CopyLimits {
                    max_rows: 2,
                    max_bytes: 1024,
                    source_read_budget: Duration::from_secs(1),
                    batch_yield: Duration::ZERO,
                },
                ..MigrationLimits::default()
            },
            fence.clone(),
        )?;
        migration.prepare(&["events".into()]).await?;
        while !migration.copy_next("events").await?.complete {}

        let mut source_transaction = source_pool.begin().await?;
        sqlx::query("UPDATE events SET source_version = 2, payload = ? WHERE stable_key = ?")
            .bind(b"new-update".as_slice())
            .bind("update-me")
            .execute(&mut *source_transaction)
            .await?;
        sqlx::query("DELETE FROM events WHERE stable_key = ?")
            .bind("delete-me")
            .execute(&mut *source_transaction)
            .await?;
        sqlx::query("DELETE FROM events WHERE stable_key = ?")
            .bind("replace-old")
            .execute(&mut *source_transaction)
            .await?;
        sqlx::query("INSERT INTO events (stable_key, source_version, payload) VALUES (?, ?, ?)")
            .bind("replace-new")
            .bind(2_i64)
            .bind(b"new-replace".as_slice())
            .execute(&mut *source_transaction)
            .await?;
        sqlx::query("INSERT INTO events (stable_key, source_version, payload) VALUES (?, ?, ?)")
            .bind("inserted")
            .bind(2_i64)
            .bind(b"new-insert".as_slice())
            .execute(&mut *source_transaction)
            .await?;
        for change in [
            change("events", "update-me", ChangeKind::Update, b"new-update"),
            change("events", "delete-me", ChangeKind::Delete, b""),
            replace_change("events", "replace-old", "replace-new", b"new-replace"),
            change("events", "inserted", ChangeKind::Insert, b"new-insert"),
        ] {
            source
                .append_captured_change_tx(
                    &mut source_transaction,
                    &CapturedChange::from_input("migration-engine-test", "sqlite-test", 2, change)?,
                )
                .await?;
        }
        source_transaction.commit().await?;

        assert_eq!(migration.catch_up("events").await?.applied_through, 2);
        let rows = target
            .rows_in_range("events", &ChunkRange::new("inserted", "update-me")?)
            .await?;
        assert_eq!(
            rows.iter()
                .map(|row| row.stable_key.as_str())
                .collect::<Vec<_>>(),
            vec!["inserted", "replace-new", "update-me"]
        );
        assert_eq!(rows[0].source_version, 2);
        assert_eq!(rows[1].payload, b"new-replace");
        assert_eq!(rows[2].payload, b"new-update");
        assert!(
            target
                .load_table_coverage_proof("migration-engine-test", "events")
                .await?
                .is_none()
        );
        let proof = migration.verify_table_coverage_at("events", 2).await?;
        assert_eq!(proof.source_digest, proof.target_digest);
        assert_eq!(proof.source_row_count, 3);
        assert_eq!(proof.target_row_count, 3);
        let coverage_checkpoint = target
            .load_coverage_checkpoint("migration-engine-test", "events")
            .await?
            .expect("PostgreSQL coverage checkpoint");
        assert!(coverage_checkpoint.complete);
        assert_eq!(coverage_checkpoint.source_row_count, 3);

        Ok(())
    })
    .await;
    let directory_cleanup = if directory.exists() {
        fs::remove_dir_all(&directory).map_err(Into::into)
    } else {
        Ok(())
    };
    match (result, directory_cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
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
        .acquire_timeout(Duration::from_secs(10))
        .connect(&database_url)
        .await?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let prefix = format!("pg02_migration_smoke_{suffix}");
    let target_table = format!("{prefix}_rows");
    with_postgres_migration_fixture_cleanup(&pool, &prefix, &target_table, async {
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
        target.ensure_schema().await?;

        let plan = TableCopyPlan {
            migration_id: "migration-engine-test".into(),
            table_name: "events".into(),
            installation_id: "installation-test".into(),
            source_identity: "sqlite-test".into(),
            target_identity: "postgres-test".into(),
            scan_lower_bound: Some("a".into()),
            scan_upper_bound: Some("a".into()),
            capture_start: 0,
        };
        target.save_plan(&plan).await?;
        let baseline_checkpoint = EngineCheckpoint {
            migration_id: plan.migration_id.clone(),
            table_name: plan.table_name.clone(),
            installation_id: plan.installation_id.clone(),
            source_identity: plan.source_identity.clone(),
            target_identity: plan.target_identity.clone(),
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
            installation_id: plan.installation_id.clone(),
            source_identity: plan.source_identity.clone(),
            target_identity: plan.target_identity.clone(),
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

        Ok(())
    })
    .await
}
