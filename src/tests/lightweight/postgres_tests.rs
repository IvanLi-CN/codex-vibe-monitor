use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use anyhow::Result;
use axum::extract::{Path, Query, State};
use sqlx::{SqlitePool, postgres::PgPoolOptions};
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
use crate::storage::migration::{
    ChangeBatchLimits, ChangeKind, EngineCheckpoint, MigrationSource, MigrationTarget, SourcePage,
    TableCopyPlan, VersionedRow,
};
use crate::storage::migration_sqlx::{
    MigrationTableSpec, NativeTargetContract, PostgresMigrationTarget, SqliteMigrationSource,
};
use crate::storage::native::{NativeRow, NativeValue, PostgresNativeSchemaAdapter};
use crate::storage::traffic::TrafficFactStore;
use crate::{
    ListQuery, LocateInvocationQuery, fetch_invocation_attempt_response_body,
    fetch_invocation_history_overview_with_runtime_overlay, fetch_invocation_new_records_count,
    fetch_invocation_pool_attempts, fetch_invocation_record_detail, fetch_invocation_request_body,
    fetch_invocation_response_body, fetch_invocation_suggestions, fetch_invocation_summary,
    fetch_invocation_workflow_detail, list_invocations, locate_invocation_page,
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
async fn postgres_authority_does_not_fallback_to_sqlite_for_routed_reads() -> Result<()> {
    let pool = SqlitePool::connect("sqlite::memory:").await?;
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Postgres,
            generation: 2,
        },
    );
    let authority = TrafficFactStore::with_authority(StoragePools::sqlite_only(pool), gate);

    assert_eq!(
        authority.authority_backend().await,
        BackendKind::Postgres,
        "read routing must follow the module authority"
    );
    let error = match authority.begin().await {
        Ok(_) => panic!("a PostgreSQL authority must not use the SQLite pool"),
        Err(error) => error,
    };
    assert!(
        format!("{error:#}").contains("without a PostgreSQL pool"),
        "unexpected authority routing error: {error:#}"
    );
    Ok(())
}

fn assert_postgres_authority_error<T, E: std::fmt::Debug>(result: Result<T, E>) {
    let error = match result {
        Ok(_) => panic!("Postgres authority unexpectedly completed through SQLite"),
        Err(error) => error,
    };
    assert!(
        format!("{error:?}").contains("without a PostgreSQL pool"),
        "unexpected authority routing error: {error:?}"
    );
}

#[tokio::test]
async fn postgres_authority_invocation_entrypoints_never_use_legacy_sqlite() -> Result<()> {
    let pool = SqlitePool::connect("sqlite::memory:").await?;
    let mut state = crate::tests::test_state_from_existing_pool(
        pool.clone(),
        crate::tests::test_config(),
        true,
    )
    .await;
    let gate = ModuleGenerationGate::new(
        StorageModule::TrafficArchive,
        ModuleAuthority {
            backend: BackendKind::Postgres,
            generation: 3,
        },
    );
    Arc::get_mut(&mut state)
        .expect("test state should be uniquely owned")
        .traffic_authority = Arc::new(TrafficFactStore::with_authority(
        StoragePools::sqlite_only(pool),
        gate,
    ));

    assert_postgres_authority_error(
        list_invocations(State(state.clone()), Query(ListQuery::default())).await,
    );
    assert_postgres_authority_error(
        locate_invocation_page(
            state.clone(),
            &LocateInvocationQuery {
                invoke_id: Some("missing-invocation".into()),
                request_id: None,
                attempt_id: None,
                upstream_account_id: None,
                page_size: None,
            },
        )
        .await,
    );
    assert_postgres_authority_error(
        fetch_invocation_summary(State(state.clone()), Query(ListQuery::default())).await,
    );
    assert_postgres_authority_error(
        fetch_invocation_history_overview_with_runtime_overlay(
            state.clone(),
            ListQuery::default(),
            Vec::new(),
            1,
        )
        .await,
    );
    assert_postgres_authority_error(
        fetch_invocation_new_records_count(State(state.clone()), Query(ListQuery::default())).await,
    );
    assert_postgres_authority_error(
        fetch_invocation_suggestions(State(state.clone()), Query(ListQuery::default())).await,
    );
    assert_postgres_authority_error(
        fetch_invocation_pool_attempts(State(state.clone()), Path("missing-invocation".into()))
            .await,
    );
    assert_postgres_authority_error(
        fetch_invocation_workflow_detail(State(state.clone()), Path(1_i64)).await,
    );
    assert_postgres_authority_error(
        fetch_invocation_record_detail(State(state.clone()), Path(1_i64)).await,
    );
    assert_postgres_authority_error(
        fetch_invocation_response_body(State(state.clone()), Path(1_i64)).await,
    );
    assert_postgres_authority_error(
        fetch_invocation_attempt_response_body(
            State(state.clone()),
            Path((1_i64, "missing-attempt".to_string())),
        )
        .await,
    );
    assert_postgres_authority_error(fetch_invocation_request_body(State(state), Path(1_i64)).await);
    Ok(())
}

#[tokio::test]
async fn sqlite_catalog_excludes_migration_capture_metadata_from_native_tables() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    let source = SqliteMigrationSource::new(
        pool.clone(),
        "migration-01",
        "test-source",
        "migration_change_log",
        vec![
            MigrationTableSpec::canonical(StorageModule::TrafficArchive, "codex_invocations")
                .unwrap(),
        ],
    )
    .unwrap();
    source.ensure_capture_schema().await.unwrap();

    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    assert!(catalog.excluded_tables().contains("migration_change_log"));
    assert!(
        catalog
            .excluded_tables()
            .contains("migration_change_log_state")
    );
    assert!(
        !catalog
            .owners()
            .iter()
            .any(|owner| owner.table == "migration_change_log")
    );
    assert!(
        !catalog
            .owners()
            .iter()
            .any(|owner| owner.table == "migration_change_log_state")
    );

    let native = catalog
        .native_migration_catalog([ModuleId::TrafficArchive])
        .unwrap();
    assert!(native.table("codex_invocations").is_some());
    assert!(native.table("migration_change_log").is_none());
    assert!(native.table("migration_change_log_state").is_none());
}

#[tokio::test]
async fn native_sqlite_capture_triggers_capture_and_fence_legacy_writes() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query(
        "CREATE TABLE codex_invocations (
            id INTEGER PRIMARY KEY,
            payload TEXT,
            raw_payload BLOB
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    let native = catalog
        .native_migration_catalog([ModuleId::TrafficArchive])
        .unwrap();
    let source = SqliteMigrationSource::from_native_catalog(
        pool.clone(),
        "migration-native-capture",
        "sqlite:test",
        "migration_change_log",
        &native,
        StorageModule::TrafficArchive,
    )
    .unwrap();
    source.ensure_capture_schema().await.unwrap();
    source.ensure_native_capture_triggers().await.unwrap();

    sqlx::query(
        "INSERT INTO codex_invocations (id, payload, raw_payload)
         VALUES (1, 'initial', X'00FF')",
    )
    .execute(&pool)
    .await
    .unwrap();
    let changes = source
        .changes_after(0, ChangeBatchLimits::default())
        .await
        .unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, ChangeKind::Insert);
    let row = NativeRow::decode_payload(
        native.table("codex_invocations").unwrap(),
        changes[0].payload.as_deref().unwrap(),
    )
    .unwrap();
    assert!(row.values().iter().any(|(name, value)| {
        name == "payload" && value == &NativeValue::Text("initial".to_string())
    }));
    assert!(row.values().iter().any(|(name, value)| {
        name == "raw_payload" && value == &NativeValue::Bytes(vec![0, 255])
    }));

    sqlx::query("UPDATE codex_invocations SET payload = 'updated' WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM codex_invocations WHERE id = 1")
        .execute(&pool)
        .await
        .unwrap();
    source.fence_source_writes(1, 2).await.unwrap();
    let legacy_write =
        sqlx::query("INSERT INTO codex_invocations (id, payload) VALUES (2, 'blocked')")
            .execute(&pool)
            .await
            .expect_err("legacy SQLite writers must fail after the source fence");
    assert!(
        legacy_write
            .to_string()
            .contains("SQLite source writes are fenced")
    );

    let changes = source
        .changes_after(0, ChangeBatchLimits::default())
        .await
        .unwrap();
    assert_eq!(changes.len(), 3);
    assert_eq!(source.source_write_authority().await.unwrap().generation, 2);
    assert!(source.source_write_authority().await.unwrap().fenced);
}

#[tokio::test]
async fn archive_native_target_contract_maps_sqlite_identity_and_part_key_columns() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE archive_batches (
            id INTEGER PRIMARY KEY,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            day_key TEXT,
            part_key TEXT,
            file_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            layout TEXT NOT NULL,
            codec TEXT NOT NULL,
            writer_version TEXT NOT NULL,
            cleanup_state TEXT NOT NULL,
            cleanup_source_safe_start_date TEXT,
            replacement_staged_path TEXT,
            superseded_by INTEGER,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            coverage_start_epoch INTEGER,
            coverage_end_epoch INTEGER,
            archive_expires_at TEXT,
            summary_source_kind TEXT NOT NULL,
            created_at TEXT NOT NULL,
            upstream_activity_manifest_refreshed_at TEXT,
            historical_rollups_materialized_at TEXT
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    let native = catalog
        .native_migration_catalog([ModuleId::TrafficArchive])
        .unwrap();
    let spec = native.table("archive_batches").unwrap();
    let contract = NativeTargetContract::for_spec(spec).unwrap();
    assert_eq!(
        contract.target_column_for_source("id").unwrap(),
        "archive_batch_id"
    );
    assert_eq!(
        contract.target_column_for_source("part_key").unwrap(),
        "part_key"
    );
    assert!(
        contract
            .target_columns()
            .any(|column| column == "source_identity_sha256")
    );
    assert!(contract.target_columns().any(|column| column == "part_key"));
    assert!(!contract.target_columns().any(|column| column == "part"));
    assert!(!contract.target_columns().any(|column| column == "id"));
}

#[tokio::test]
async fn native_module_tables_copy_historical_parents_before_children() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE archive_batches (
            id INTEGER PRIMARY KEY
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "CREATE TABLE archive_batch_upstream_activity (
            archive_batch_id INTEGER NOT NULL REFERENCES archive_batches(id),
            account_id INTEGER NOT NULL,
            last_activity_at TEXT NOT NULL,
            PRIMARY KEY (archive_batch_id, account_id)
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO archive_batches (id) VALUES (41)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO archive_batch_upstream_activity \
         (archive_batch_id, account_id, last_activity_at) VALUES (41, 7, '2026-10-27T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    let native = catalog
        .native_migration_catalog([ModuleId::TrafficArchive])
        .unwrap();
    let source = SqliteMigrationSource::from_native_catalog(
        pool,
        "migration-native-fk-order",
        "sqlite:test",
        "migration_change_log",
        &native,
        StorageModule::TrafficArchive,
    )
    .unwrap();
    let tables = source
        .module_tables(StorageModule::TrafficArchive)
        .await
        .unwrap();
    let parent = tables
        .iter()
        .position(|table| table == "archive_batches")
        .unwrap();
    let child = tables
        .iter()
        .position(|table| table == "archive_batch_upstream_activity")
        .unwrap();
    assert!(parent < child, "parent must precede child: {tables:?}");
}

#[tokio::test]
async fn archive_native_insert_path_uses_owner_target_columns_when_postgres_is_available()
-> Result<()> {
    let Ok(database_url) = std::env::var("POSTGRES_TEST_DATABASE_URL") else {
        return Ok(());
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .unwrap();
    let prefix = format!("pg09r_archive_insert_{}", std::process::id());
    let sqlite = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE archive_batches (
            id INTEGER PRIMARY KEY,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            day_key TEXT,
            part_key TEXT,
            file_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            layout TEXT NOT NULL,
            codec TEXT NOT NULL,
            writer_version TEXT NOT NULL,
            cleanup_state TEXT NOT NULL,
            cleanup_source_safe_start_date TEXT,
            replacement_staged_path TEXT,
            superseded_by INTEGER,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            coverage_start_epoch INTEGER,
            coverage_end_epoch INTEGER,
            archive_expires_at TEXT,
            summary_source_kind TEXT NOT NULL,
            created_at TEXT NOT NULL,
            upstream_activity_manifest_refreshed_at TEXT,
            historical_rollups_materialized_at TEXT
        )",
    )
    .execute(&sqlite)
    .await
    .unwrap();
    let catalog = ModuleCatalog::inspect_sqlite(&sqlite).await.unwrap();
    let native = catalog
        .native_migration_catalog([ModuleId::TrafficArchive])
        .unwrap();
    let spec = native.table("archive_batches").unwrap();
    let payload_for = |id: i64, part_key: Option<&str>| {
        let values = spec
            .columns
            .iter()
            .map(|column| match column.postgres_type {
                crate::storage::catalog::NativeColumnType::BigInt => {
                    NativeValue::BigInt(if column.name == "id" { id } else { 1 })
                }
                crate::storage::catalog::NativeColumnType::Text => {
                    if column.name == "part_key" {
                        part_key.map_or(NativeValue::Null, |value| {
                            NativeValue::Text(value.to_owned())
                        })
                    } else if column.name == "sha256" {
                        NativeValue::Text(
                            "0000000000000000000000000000000000000000000000000000000000000000"
                                .to_owned(),
                        )
                    } else if column.name == "file_path" {
                        NativeValue::Text(format!("pg09r-test-{id}"))
                    } else {
                        NativeValue::Text("pg09r-test".to_owned())
                    }
                }
                crate::storage::catalog::NativeColumnType::DoublePrecision => {
                    NativeValue::Double(1.0)
                }
                crate::storage::catalog::NativeColumnType::Boolean => NativeValue::Boolean(true),
                crate::storage::catalog::NativeColumnType::Bytea => NativeValue::Bytes(vec![1]),
            })
            .collect::<Vec<_>>();
        NativeRow::from_values(spec, values)
            .unwrap()
            .encode_payload()
            .unwrap()
    };
    let segment_payload = payload_for(900_001, Some("part-000001"));
    let monthly_payload = payload_for(900_002, None);
    let target = PostgresMigrationTarget::new_native(
        pool.clone(),
        &prefix,
        native,
        StorageModule::TrafficArchive,
    )
    .unwrap();
    let result: Result<()> = async {
        target.ensure_schema().await?;
        let plan = TableCopyPlan {
            migration_id: "migration-native-archive-insert".into(),
            table_name: "archive_batches".into(),
            installation_id: "installation-test".into(),
            source_identity: "sqlite-test".into(),
            target_identity: "postgres-test".into(),
            scan_lower_bound: Some("900001".into()),
            scan_upper_bound: Some("900002".into()),
            capture_start: 0,
        };
        target.save_plan(&plan).await?;
        target
            .apply_baseline_page(
                &plan,
                &SourcePage {
                    rows: vec![
                        VersionedRow {
                            stable_key: "900001".into(),
                            source_version: 0,
                            payload: segment_payload,
                        },
                        VersionedRow {
                            stable_key: "900002".into(),
                            source_version: 0,
                            payload: monthly_payload,
                        },
                    ],
                    next_cursor: Some("900002".into()),
                    complete: true,
                },
                &EngineCheckpoint {
                    migration_id: plan.migration_id.clone(),
                    table_name: plan.table_name.clone(),
                    installation_id: plan.installation_id.clone(),
                    source_identity: plan.source_identity.clone(),
                    target_identity: plan.target_identity.clone(),
                    cursor: Some("900002".into()),
                    scan_upper_bound: plan.scan_upper_bound.clone(),
                    capture_start: 0,
                    captured_through: 0,
                    applied_through: 0,
                    copied_rows: 2,
                    copied_bytes: 2,
                    complete: true,
                },
            )
            .await?;
        let rows = sqlx::query_as::<_, (i64, i32, Option<String>, String, String)>(
            "SELECT archive_batch_id, part, part_key, sha256, source_identity_sha256 \
             FROM archive_batches WHERE archive_batch_id IN ($1, $2) ORDER BY archive_batch_id",
        )
        .bind(900_001_i64)
        .bind(900_002_i64)
        .fetch_all(&pool)
        .await?;
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, 900_001);
        assert_eq!(rows[0].1, 0);
        assert_eq!(rows[0].2.as_deref(), Some("part-000001"));
        assert_eq!(rows[0].3, rows[0].4);
        assert_eq!(rows[1].0, 900_002);
        assert_eq!(rows[1].1, 0);
        assert_eq!(rows[1].2, None);
        assert_eq!(rows[1].3, rows[1].4);
        Ok(())
    }
    .await;
    let cleanup = async {
        sqlx::query("DELETE FROM archive_batches WHERE archive_batch_id IN ($1, $2)")
            .bind(900_001_i64)
            .bind(900_002_i64)
            .execute(&pool)
            .await?;
        for table in [
            format!("{prefix}_plans"),
            format!("{prefix}_checkpoints"),
            format!("{prefix}_chunk_proofs"),
            format!("{prefix}_coverage_checkpoints"),
            format!("{prefix}_coverage_proofs"),
        ] {
            sqlx::query(&format!("DROP TABLE IF EXISTS \"{table}\""))
                .execute(&pool)
                .await?;
        }
        Result::<()>::Ok(())
    }
    .await;
    result?;
    cleanup?;
    Ok(())
}

#[tokio::test]
async fn native_capture_capacity_holds_and_releases_without_aborting_business_writes() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(
        "CREATE TABLE codex_invocations (
            id INTEGER PRIMARY KEY,
            payload TEXT
        )",
    )
    .execute(&pool)
    .await
    .unwrap();
    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    let native = catalog
        .native_migration_catalog([ModuleId::TrafficArchive])
        .unwrap();
    let source = SqliteMigrationSource::from_native_catalog(
        pool.clone(),
        "migration-native-capacity",
        "sqlite:test",
        "migration_change_log",
        &native,
        StorageModule::TrafficArchive,
    )
    .unwrap()
    .with_max_capture_bytes(1)
    .unwrap();
    source.ensure_capture_schema().await.unwrap();
    source.ensure_native_capture_triggers().await.unwrap();

    sqlx::query("INSERT INTO codex_invocations (id, payload) VALUES (1, 'held')")
        .execute(&pool)
        .await
        .expect("business write remains successful while capture is held");
    assert!(source.capture_capacity_held().await.unwrap());
    let through = source.capture_watermark().await.unwrap();
    assert!(
        !source
            .changes_through(0, through, ChangeBatchLimits::default())
            .await
            .unwrap()
            .is_empty()
    );

    source.release_capture_through(through).await.unwrap();
    assert!(!source.capture_capacity_held().await.unwrap());
    sqlx::query("INSERT INTO codex_invocations (id, payload) VALUES (2, NULL)")
        .execute(&pool)
        .await
        .expect("business writes resume after capture consumption");
}

#[tokio::test]
async fn current_sqlite_schema_builds_native_traffic_analytics_catalog() {
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
    let selected_modules = [ModuleId::TrafficArchive, ModuleId::Analytics];
    let native = catalog
        .native_migration_catalog(selected_modules)
        .expect("TrafficArchive and Analytics native closure");
    let source = SqliteMigrationSource::from_native_catalog(
        pool.clone(),
        "migration-native-schema",
        "sqlite:test",
        "migration_change_log",
        &native,
        StorageModule::TrafficArchive,
    )
    .unwrap();
    source.ensure_capture_schema().await.unwrap();
    source.ensure_native_capture_triggers().await.unwrap();
    let expected_table_count = catalog
        .owners()
        .iter()
        .filter(|owner| selected_modules.contains(&owner.module))
        .count();
    assert_eq!(native.tables.len(), expected_table_count);
    assert!(native.table("codex_invocations").is_some());
    assert!(native.table("invocation_rollup_hourly").is_some());
    assert!(
        native
            .table("prompt_cache_conversation_stats_freshness")
            .is_some()
    );
    assert!(
        catalog
            .cross_module_trigger_writes()
            .iter()
            .all(|relation| {
                let source_module = catalog
                    .owners()
                    .iter()
                    .find(|owner| owner.table == relation.source_table)
                    .map(|owner| owner.module);
                let target_module = catalog
                    .owners()
                    .iter()
                    .find(|owner| owner.table == relation.target_table)
                    .map(|owner| owner.module);
                source_module == target_module
                    || (source_module.is_some_and(|module| selected_modules.contains(&module))
                        && target_module.is_some_and(|module| selected_modules.contains(&module)))
            })
    );

    let invocations = native.table("codex_invocations").unwrap();
    assert_eq!(invocations.stable_key_columns(), &["id"]);
    assert!(
        invocations
            .columns
            .iter()
            .any(|column| column.name == "payload")
    );
    assert!(
        invocations
            .resource_references
            .iter()
            .any(|reference| reference.column == "request_raw_path")
    );
    assert_eq!(
        invocations.source_watermark.business_source_version_column,
        None
    );
    assert_eq!(
        invocations.source_watermark.capture.table_name_column,
        "table_name"
    );
    assert_eq!(
        invocations.source_watermark.capture.stable_key_column,
        "stable_key"
    );
    assert_eq!(
        invocations.source_watermark.capture.delete.delete_value,
        "delete"
    );
    assert_eq!(
        invocations.source_watermark.capture.generation_column,
        "capture_generation"
    );
    let invocation_ddl = invocations.postgres_create_table_sql().unwrap();
    assert!(invocation_ddl.contains("\"payload\""));
    assert!(invocation_ddl.contains("PRIMARY KEY (\"id\")"));
    assert!(!invocation_ddl.contains("payload BYTEA NOT NULL"));

    let wide_row = NativeRow::from_values(
        invocations,
        invocations
            .columns
            .iter()
            .map(|column| match column.name.as_str() {
                "id" => NativeValue::BigInt(7),
                "payload" => NativeValue::Text("{\"wide\":true}".into()),
                "cost" => NativeValue::Double(1.25),
                "request_raw_path" => NativeValue::Text("raw/request.bin".into()),
                _ if !column.nullable => match column.postgres_type {
                    crate::storage::catalog::NativeColumnType::BigInt => NativeValue::BigInt(0),
                    crate::storage::catalog::NativeColumnType::DoublePrecision => {
                        NativeValue::Double(0.0)
                    }
                    crate::storage::catalog::NativeColumnType::Boolean => {
                        NativeValue::Boolean(false)
                    }
                    crate::storage::catalog::NativeColumnType::Bytea => {
                        NativeValue::Bytes(Vec::new())
                    }
                    crate::storage::catalog::NativeColumnType::Text => {
                        NativeValue::Text(String::new())
                    }
                },
                _ => NativeValue::Null,
            }),
    )
    .unwrap();
    assert_eq!(wide_row.table_name(), "codex_invocations");
    assert_eq!(wide_row.values().len(), invocations.columns.len());
    assert!(wide_row.values().iter().any(|(column, value)| {
        column == "payload" && value == &NativeValue::Text("{\"wide\":true}".into())
    }));
}

#[tokio::test]
async fn native_catalog_preserves_wide_rows_and_constraints() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    for statement in [
        "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, payload TEXT NOT NULL, cost REAL NOT NULL DEFAULT 0, raw_payload BLOB, request_raw_path TEXT, source TEXT NOT NULL)",
        "CREATE TABLE proxy_raw_payload_blobs (raw_path TEXT PRIMARY KEY, payload BLOB NOT NULL)",
        "CREATE TABLE proxy_raw_payload_blob_links (raw_path TEXT NOT NULL REFERENCES proxy_raw_payload_blobs(raw_path) ON DELETE CASCADE, owner_id INTEGER NOT NULL, owner_kind TEXT NOT NULL, raw_role TEXT NOT NULL, PRIMARY KEY (raw_path, owner_id, owner_kind, raw_role))",
        "CREATE TABLE invocation_rollup_hourly (bucket_start_epoch INTEGER PRIMARY KEY, total_cost REAL NOT NULL DEFAULT 0, source_version INTEGER NOT NULL DEFAULT 1)",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    let native = catalog
        .native_migration_catalog([ModuleId::TrafficArchive, ModuleId::Analytics])
        .unwrap();
    assert_eq!(native.table_names().count(), 4);

    let invocations = native.table("codex_invocations").unwrap();
    assert_eq!(invocations.stable_key_columns(), &["id"]);
    assert_eq!(
        invocations.source_watermark.business_source_version_column,
        None
    );
    assert!(
        !invocations
            .columns
            .iter()
            .any(|column| column.name == "source_version")
    );
    assert!(
        invocations
            .resource_references
            .iter()
            .any(|reference| { reference.column == "request_raw_path" })
    );
    let invocation_ddl = invocations.postgres_create_table_sql().unwrap();
    assert!(invocation_ddl.contains("\"payload\" TEXT NOT NULL"));
    assert!(invocation_ddl.contains("\"raw_payload\" BYTEA"));
    assert!(invocation_ddl.contains("PRIMARY KEY (\"id\")"));

    let wide_row = NativeRow::from_values(
        invocations,
        [
            NativeValue::BigInt(7),
            NativeValue::Text("{\"wide\":true}".into()),
            NativeValue::Double(1.25),
            NativeValue::Bytes(vec![1, 2, 3]),
            NativeValue::Text("raw/request.bin".into()),
            NativeValue::Text("proxy".into()),
        ],
    )
    .unwrap();
    assert_eq!(wide_row.table_name(), "codex_invocations");
    assert_eq!(wide_row.values().len(), invocations.columns.len());
    assert!(wide_row.values().iter().any(|(column, value)| {
        column == "raw_payload" && value == &NativeValue::Bytes(vec![1, 2, 3])
    }));

    let links = native.table("proxy_raw_payload_blob_links").unwrap();
    let foreign_key_ddl = links.postgres_foreign_key_sql().unwrap();
    let repeated_foreign_key_ddl = links.postgres_foreign_key_sql().unwrap();
    assert_eq!(foreign_key_ddl, repeated_foreign_key_ddl);
    assert!(foreign_key_ddl.iter().any(|statement| {
        statement.contains("IF NOT EXISTS")
            && statement.contains("FROM pg_constraint AS pc")
            && statement.contains("REFERENCES \"proxy_raw_payload_blobs\" (\"raw_path\")")
            && statement.contains("ON DELETE CASCADE")
    }));

    let rollup = native.table("invocation_rollup_hourly").unwrap();
    assert_eq!(
        rollup
            .source_watermark
            .business_source_version_column
            .as_deref(),
        Some("source_version")
    );

    let adapter = PostgresNativeSchemaAdapter::new(
        sqlx::PgPool::connect_lazy("postgres://postgres@localhost/postgres").unwrap(),
        native.clone(),
    );
    let adapter_ddl = adapter.ddl_statements().unwrap();
    let repeated_adapter_ddl = adapter.ddl_statements().unwrap();
    assert_eq!(adapter_ddl, repeated_adapter_ddl);
    assert!(adapter_ddl.iter().any(|statement| {
        statement.contains("ALTER TABLE \"proxy_raw_payload_blob_links\"")
            && statement.contains("FROM pg_constraint AS pc")
    }));
}

#[tokio::test]
async fn native_ddl_translates_known_defaults_and_rejects_sqlite_only_expressions() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    for statement in [
        "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')))",
        "CREATE TABLE invocation_rollup_hourly (bucket_start_epoch INTEGER PRIMARY KEY, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        "CREATE TABLE prompt_cache_conversations (conversation_id TEXT PRIMARY KEY, payload TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')), CHECK (json_valid(payload)))",
        "CREATE TABLE archive_batches (id INTEGER PRIMARY KEY, status TEXT NOT NULL, coverage_start_at TEXT, CHECK (julianday(coverage_start_at) >= 0))",
        "CREATE INDEX archive_batches_completed_at ON archive_batches(status) WHERE datetime(coverage_start_at) IS NOT NULL",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    let native = catalog
        .native_migration_catalog([ModuleId::TrafficArchive, ModuleId::Analytics])
        .unwrap();

    let invocation_ddl = native
        .table("codex_invocations")
        .unwrap()
        .postgres_create_table_sql()
        .unwrap();
    assert!(invocation_ddl.contains("to_char(CURRENT_TIMESTAMP AT TIME ZONE 'UTC'"));
    assert!(!invocation_ddl.to_ascii_uppercase().contains("STRFTIME("));

    let rollup_ddl = native
        .table("invocation_rollup_hourly")
        .unwrap()
        .postgres_create_table_sql()
        .unwrap();
    assert!(rollup_ddl.contains("CURRENT_TIMESTAMP"));
    assert!(!rollup_ddl.to_ascii_uppercase().contains("DATETIME("));

    let prompt_cache_error = native
        .table("prompt_cache_conversations")
        .unwrap()
        .postgres_create_table_sql()
        .unwrap_err();
    assert!(matches!(
        prompt_cache_error,
        CatalogError::NativeCatalogIncomplete { detail, .. }
            if detail.contains("json_valid()")
    ));

    let archive = native.table("archive_batches").unwrap();
    let archive_table_error = archive.postgres_create_table_sql().unwrap_err();
    assert!(matches!(
        archive_table_error,
        CatalogError::NativeCatalogIncomplete { detail, .. }
            if detail.contains("julianday()")
    ));
    let archive_index_error = archive.postgres_index_sql().unwrap_err();
    assert!(matches!(
        archive_index_error,
        CatalogError::NativeCatalogIncomplete { detail, .. }
            if detail.contains("datetime()")
    ));
}

#[tokio::test]
async fn native_catalog_rejects_unapproved_cross_module_trigger_edges() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    for statement in [
        "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)",
        "CREATE TABLE invocation_rollup_hourly (id INTEGER PRIMARY KEY)",
        "CREATE TRIGGER invocation_cross_module AFTER INSERT ON codex_invocations BEGIN UPDATE invocation_rollup_hourly SET id = NEW.id WHERE id = NEW.id; END",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }
    let catalog = ModuleCatalog::inspect_sqlite(&pool).await.unwrap();
    assert!(matches!(
        catalog.native_migration_catalog([ModuleId::TrafficArchive]),
        Err(CatalogError::NativeCrossModuleEdge {
            edge_kind: "trigger",
            source_table,
            target_table,
        }) if source_table == "codex_invocations" && target_table == "invocation_rollup_hourly"
    ));
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
        catalog.native_migration_catalog([ModuleId::TrafficArchive]),
        Err(CatalogError::NativeCatalogIncomplete { table, detail })
            if table == "proxy_raw_payload_blob_links"
                && detail.contains("deterministic primary or non-null unique key")
    ));
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
        installation_id: "installation-01".into(),
        source_identity: "sqlite-main-installation-01".into(),
        target_identity: "postgres-main-installation-01".into(),
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
