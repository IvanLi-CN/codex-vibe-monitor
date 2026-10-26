use std::{
    fs,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;

use crate::storage::{
    core::{
        AuthorityManifestStore, AuthorityRecoveryContext, BackendKind, MigrationMetadataStore,
        MigrationOutboxItem, ModuleAuthority, ModuleGenerationGate, StorageModule,
    },
    migration::{
        ChangeInput, ChangeKind, ChunkRange, CopyLimits, FileMigrationStore,
        InMemoryMigrationSource, InMemoryMigrationTarget, MigrationEngine, MigrationLimits,
        SourceWriteFence,
    },
};

fn recovery_context(source_watermark: i64) -> AuthorityRecoveryContext {
    AuthorityRecoveryContext {
        migration_id: "migration-engine-test".into(),
        installation_id: "installation-test".into(),
        module: StorageModule::TrafficArchive,
        source_identity: "sqlite-test".into(),
        target_identity: "postgres-test".into(),
        source_watermark,
    }
}

fn migration_limits(max_rows: usize) -> MigrationLimits {
    MigrationLimits {
        copy: CopyLimits {
            max_rows,
            max_bytes: 1024 * 1024,
            source_read_budget: Duration::from_secs(1),
            batch_yield: Duration::ZERO,
        },
        ..MigrationLimits::default()
    }
}

fn state_path(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!("codex-pg02-{label}-{suffix}"));
    (directory.clone(), directory.join("migration-state.json"))
}

fn change(table_name: &str, stable_key: &str, kind: ChangeKind, payload: &[u8]) -> ChangeInput {
    ChangeInput {
        table_name: table_name.into(),
        stable_key: stable_key.into(),
        previous_key: None,
        kind,
        payload: (kind != ChangeKind::Delete).then(|| payload.to_vec()),
    }
}

fn new_engine(
    source: Arc<InMemoryMigrationSource>,
    target: Arc<InMemoryMigrationTarget>,
    store: Arc<FileMigrationStore>,
    fence: SourceWriteFence,
    limits: MigrationLimits,
) -> MigrationEngine<InMemoryMigrationSource, InMemoryMigrationTarget, FileMigrationStore> {
    MigrationEngine::new(
        source,
        target,
        store,
        StorageModule::TrafficArchive,
        "installation-test",
        "postgres-test",
        1,
        limits,
        fence,
    )
    .expect("migration engine construction")
}

#[tokio::test]
async fn migration_copies_pages_and_applies_incremental_replace_delete() -> Result<()> {
    let (directory, path) = state_path("copy-catch-up");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![
                change("events", "a", ChangeKind::Insert, b"one"),
                change("events", "b", ChangeKind::Insert, b"two"),
                change("events", "c", ChangeKind::Insert, b"three"),
            ],
        )
        .await?;
    let target = Arc::new(InMemoryMigrationTarget::new());
    let store = Arc::new(FileMigrationStore::new(&path, recovery_context(0)));
    let engine = new_engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        fence.clone(),
        migration_limits(2),
    );

    engine.prepare(&["events".into()]).await?;
    while !engine.copy_next("events").await?.complete {}
    source
        .write(
            &fence,
            vec![change("events", "a", ChangeKind::Update, b"one-updated")],
        )
        .await?;
    source
        .write(
            &fence,
            vec![
                ChangeInput {
                    table_name: "events".into(),
                    stable_key: "z".into(),
                    previous_key: Some("b".into()),
                    kind: ChangeKind::Replace,
                    payload: Some(b"two-replaced".to_vec()),
                },
                change("events", "c", ChangeKind::Delete, b""),
            ],
        )
        .await?;
    engine.catch_up("events").await?;

    assert_eq!(
        engine
            .verify_chunk("events", ChunkRange::new("a", "z")?)
            .await?,
        crate::storage::migration::ChunkVerification::Verified
    );
    assert_eq!(
        target.row("events", "a").await.unwrap().payload,
        b"one-updated"
    );
    assert_eq!(
        target.row("events", "z").await.unwrap().payload,
        b"two-replaced"
    );
    assert!(target.row("events", "b").await.is_none());
    assert!(target.row("events", "c").await.is_none());

    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    engine
        .switch_authority(&gate, recovery_context(3), Duration::from_secs(1))
        .await?;
    assert!(fence.is_fenced().await);
    assert!(matches!(
        store
            .load_authority_manifest(StorageModule::TrafficArchive)
            .await?,
        crate::storage::core::AuthorityRecoveryRecord::Manifest(manifest)
            if manifest.authority.backend == BackendKind::Postgres
    ));

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn catch_up_applies_only_the_requested_table_and_keeps_prefix_progress() -> Result<()> {
    let (directory, path) = state_path("table-scoped-catch-up");
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
    source
        .write(
            &fence,
            vec![change("accounts", "a", ChangeKind::Insert, b"account")],
        )
        .await?;
    let target = Arc::new(InMemoryMigrationTarget::new());
    let store = Arc::new(FileMigrationStore::new(&path, recovery_context(2)));
    let engine = new_engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        fence.clone(),
        migration_limits(8),
    );

    engine
        .prepare(&["events".into(), "accounts".into()])
        .await?;
    while !engine.copy_next("events").await?.complete {}
    while !engine.copy_next("accounts").await?.complete {}
    source
        .write(
            &fence,
            vec![change("accounts", "b", ChangeKind::Insert, b"second")],
        )
        .await?;
    source
        .write(
            &fence,
            vec![change("events", "b", ChangeKind::Insert, b"two")],
        )
        .await?;

    let events_progress = engine.catch_up("events").await?;
    assert_eq!(events_progress.applied_through, 4);
    assert_eq!(target.row("events", "b").await.unwrap().payload, b"two");
    assert!(target.row("accounts", "b").await.is_none());

    let accounts_progress = engine.catch_up("accounts").await?;
    assert_eq!(accounts_progress.applied_through, 4);
    assert_eq!(
        target.row("accounts", "b").await.unwrap().payload,
        b"second"
    );

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn failed_target_page_does_not_advance_checkpoint_and_can_retry() -> Result<()> {
    let (directory, path) = state_path("page-retry");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    let fence = SourceWriteFence::new(1);
    source
        .write(
            &fence,
            vec![
                change("events", "a", ChangeKind::Insert, b"one"),
                change("events", "b", ChangeKind::Insert, b"two"),
            ],
        )
        .await?;
    let target = Arc::new(InMemoryMigrationTarget::new());
    let store = Arc::new(FileMigrationStore::new(&path, recovery_context(1)));
    let engine = new_engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        fence,
        migration_limits(1),
    );
    engine.prepare(&["events".into()]).await?;
    target.fail_next_operation().await;
    assert!(engine.copy_next("events").await.is_err());
    assert!(target.row("events", "a").await.is_none());
    assert!(
        !target
            .load_checkpoint("migration-engine-test", "events")
            .await?
            .unwrap()
            .cursor
            .is_some()
    );

    let progress = engine.copy_next("events").await?;
    assert!(!progress.complete);
    assert_eq!(target.row("events", "a").await.unwrap().payload, b"one");

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn checkpoint_restart_resumes_from_target_cursor() -> Result<()> {
    let (directory, path) = state_path("checkpoint-restart");
    let source = Arc::new(InMemoryMigrationSource::new(
        "migration-engine-test",
        "sqlite-test",
    ));
    let setup_fence = SourceWriteFence::new(1);
    source
        .write(
            &setup_fence,
            vec![
                change("events", "a", ChangeKind::Insert, b"one"),
                change("events", "b", ChangeKind::Insert, b"two"),
                change("events", "c", ChangeKind::Insert, b"three"),
            ],
        )
        .await?;
    let target = Arc::new(InMemoryMigrationTarget::new());
    let store = Arc::new(FileMigrationStore::new(&path, recovery_context(1)));
    let first = new_engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        setup_fence,
        migration_limits(1),
    );
    first.prepare(&["events".into()]).await?;
    assert!(!first.copy_next("events").await?.complete);

    let restarted = new_engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        SourceWriteFence::new(1),
        migration_limits(1),
    );
    restarted.prepare(&["events".into()]).await?;
    while !restarted.copy_next("events").await?.complete {}
    assert_eq!(target.row("events", "a").await.unwrap().payload, b"one");
    assert_eq!(target.row("events", "c").await.unwrap().payload, b"three");

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn source_fence_timeout_reopens_before_authority_publication() -> Result<()> {
    let fence = SourceWriteFence::new(1);
    let lease = fence.begin_write().await?;
    assert!(
        fence
            .close_and_drain(Duration::from_millis(1))
            .await
            .is_err()
    );
    assert!(!fence.is_fenced().await);
    drop(lease);
    fence.begin_write().await?.complete();
    Ok(())
}

#[tokio::test]
async fn uncommitted_authority_switch_reopens_prepared_source_fence() -> Result<()> {
    let (directory, path) = state_path("authority-timeout");
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
    let target = Arc::new(InMemoryMigrationTarget::new());
    let context = recovery_context(1);
    let store = Arc::new(FileMigrationStore::new(&path, context.clone()));
    let engine = new_engine(
        Arc::clone(&source),
        Arc::clone(&target),
        Arc::clone(&store),
        fence.clone(),
        migration_limits(8),
    );
    engine.prepare(&["events".into()]).await?;
    while !engine.copy_next("events").await?.complete {}
    engine.catch_up("events").await?;
    engine
        .verify_chunk("events", ChunkRange::new("a", "a")?)
        .await?;
    let switch_context = AuthorityRecoveryContext {
        target_identity: "wrong-target".into(),
        ..context.clone()
    };

    assert!(
        engine
            .switch_authority(
                &ModuleGenerationGate::new(
                    StorageModule::TrafficArchive,
                    ModuleAuthority {
                        backend: BackendKind::Sqlite,
                        generation: 1,
                    },
                ),
                switch_context,
                Duration::from_secs(1)
            )
            .await
            .is_err()
    );
    assert!(!fence.is_fenced().await);
    source
        .write(
            &fence,
            vec![change("events", "b", ChangeKind::Insert, b"two")],
        )
        .await?;

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn file_authority_manifest_survives_restart_and_recovery() -> Result<()> {
    let (directory, path) = state_path("authority-recovery");
    let context = recovery_context(7);
    let store = Arc::new(FileMigrationStore::new(&path, context.clone()));
    let manifest = crate::storage::core::ModuleAuthorityManifest {
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
        store
            .publish_authority_manifest(&manifest, &context)
            .await?,
        crate::storage::core::AuthorityPublication::Committed
    );

    let restarted = FileMigrationStore::new(&path, context.clone());
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Sqlite,
            generation: 1,
        },
    );
    gate.recover_authority(&restarted, &context).await?;
    let lease = gate.acquire(StorageModule::TrafficArchive).await?;
    assert_eq!(lease.lease.authority.backend, BackendKind::Postgres);
    assert_eq!(lease.lease.authority.generation, 2);

    fs::remove_dir_all(directory)?;
    Ok(())
}

#[tokio::test]
async fn file_outbox_ack_requires_the_applied_watermark() -> Result<()> {
    let (directory, path) = state_path("outbox-ack");
    let context = recovery_context(1);
    let store = FileMigrationStore::new(&path, context.clone());
    let item = MigrationOutboxItem {
        outbox_id: "outbox-1".into(),
        migration_id: context.migration_id.clone(),
        source_identity: context.source_identity.clone(),
        source_version: 3,
        module: context.module,
        authority_generation: 1,
        payload_digest: "digest".into(),
    };
    assert!(store.enqueue_outbox(&item).await?);
    assert!(!store.enqueue_outbox(&item).await?);
    assert!(store.acknowledge_outbox("outbox-1", 2).await.is_err());
    store.acknowledge_outbox("outbox-1", 3).await?;
    store.acknowledge_outbox("outbox-1", 3).await?;

    fs::remove_dir_all(directory)?;
    Ok(())
}
