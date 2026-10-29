use crate::authorize_control_request;
use crate::storage::config::StorageConfig;
use crate::storage::core::StorageModule;
use crate::storage::migration::{CapturedChange, ChangeBatchLimits, ChangeInput, MigrationSource};
use crate::storage::migration_sqlx::{MigrationTableSpec, SqliteMigrationSource};
use crate::storage::migration_status::{
    StorageMigrationBackend, StorageMigrationControlRequest, StorageMigrationProgress,
    StorageMigrationResponse, StorageMigrationRuntime, StorageMigrationState,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use sqlx::sqlite::SqlitePoolOptions;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn default_status_exposes_safe_target_and_unknown_progress() {
    let runtime = StorageMigrationRuntime::default();
    let status = runtime.snapshot().await;

    assert_eq!(status.state, StorageMigrationState::Pending);
    assert_eq!(status.revision, 0);
    assert!(!status.target.configured);
    assert_eq!(status.target.identity, "postgres:unconfigured");
    assert!(status.progress.committed_rows.is_none());
    assert!(status.progress.eta_ms.is_none());
    assert!(status.tables.is_empty());
    assert!(status.verification.dirty_chunks.is_none());
    assert!(status.verification.verified_chunks.is_none());
    assert!(status.verification.last_error.is_none());
    assert!(status.switch.result.is_none());
    assert!(status.switch.failure_reason.is_none());
    assert!(status.task_holds.last_safe_checkpoint.is_none());
    assert_eq!(status.authority.backend, StorageMigrationBackend::Sqlite);
    assert_eq!(status.modules.len(), 1);
    assert!(status.modules[0].dependency_ready.is_none());
    assert!(status.modules[0].final_proof_coverage.is_none());
    assert!(status.modules[0].switch_result.is_none());
    assert!(status.heartbeat.healthy);
    assert_eq!(status.queued_modules, Vec::<String>::new());
    assert_eq!(status.completed_modules, Vec::<String>::new());
    let serialized = serde_json::to_string(&status).expect("status serializes");
    assert!(!serialized.contains("POSTGRES_DATABASE_URL"));
    assert!(!serialized.contains("postgresql://"));
}

#[tokio::test]
async fn controls_follow_monotonic_migration_state_contract() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let runtime = StorageMigrationRuntime::new(&config);
    assert_eq!(
        runtime.snapshot().await.queued_modules,
        vec!["traffic_archive"]
    );

    let started = runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");
    assert_eq!(started.state, StorageMigrationState::Preparing);
    assert_eq!(started.revision, 1);
    assert_eq!(started.progress.phase, started.state);
    assert!(started.controls.can_pause);

    let paused = runtime
        .apply_control(StorageMigrationControlRequest { paused: true })
        .await
        .expect("pause is accepted");
    assert_eq!(paused.state, StorageMigrationState::Paused);
    assert_eq!(paused.revision, started.revision + 1);
    assert_eq!(paused.progress.phase, paused.state);
    assert_eq!(paused.task_holds.reason.as_deref(), Some("operator_pause"));
    assert!(!paused.task_holds.held);

    let resumed = runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("resume is accepted");
    assert_eq!(resumed.state, StorageMigrationState::Preparing);
    assert_eq!(resumed.revision, paused.revision + 1);
    assert_eq!(resumed.progress.phase, resumed.state);
    assert!(resumed.task_holds.held);
    assert_eq!(resumed.task_holds.reason.as_deref(), Some("migration_hold"));
}

#[tokio::test]
async fn dynamic_catch_up_does_not_publish_a_fake_eta() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let runtime = StorageMigrationRuntime::new(&config);
    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");

    let status = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::Copying,
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("copying progress is accepted");
    assert_eq!(status.state, StorageMigrationState::Copying);

    let status = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::CatchingUp,
            pending_keys: Some(42),
            replay_throughput_per_sec: Some(10.0),
            incoming_rate_per_sec: Some(10.0),
            eta_ms: Some(1_000),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("catch-up progress is accepted");

    assert_eq!(status.state, StorageMigrationState::CatchingUp);
    assert_eq!(status.progress.converged, Some(false));
    assert!(status.progress.eta_ms.is_none());
    assert!(status.pressure.non_convergent);
}

#[tokio::test]
async fn controls_reject_unavailable_transitions() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let runtime = StorageMigrationRuntime::new(&config);

    let error = runtime
        .apply_control(StorageMigrationControlRequest { paused: true })
        .await
        .expect_err("pending migration cannot be paused");
    assert!(error.to_string().contains("pause control is unavailable"));

    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");
    let error = runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect_err("running migration cannot be started or resumed");
    assert!(error.to_string().contains("resume control is unavailable"));
}

#[tokio::test]
async fn progress_events_keep_the_latest_full_snapshot_and_revision() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let runtime = StorageMigrationRuntime::new(&config);
    let mut events = runtime.subscribe();
    let initial_revision = events.borrow().revision;

    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");
    events.changed().await.expect("status event is retained");

    let first = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::Copying,
            pending_keys: Some(10),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("first progress is accepted");
    events
        .changed()
        .await
        .expect("first progress event is retained");
    assert_eq!(
        events.borrow_and_update().status.progress.pending_keys,
        Some(10)
    );

    let latest = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::CatchingUp,
            pending_keys: Some(2),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("latest progress is accepted");

    let event = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            events
                .changed()
                .await
                .expect("latest progress event is retained");
            let event = events.borrow_and_update().clone();
            if event.kind == "progress" && event.status.progress.pending_keys == Some(2) {
                break event;
            }
        }
    })
    .await
    .expect("latest progress should flush within the coalescing window");
    assert_eq!(event.kind, "progress");
    assert_eq!(event.revision, latest.revision);
    assert_eq!(event.status.revision, latest.revision);
    assert_eq!(event.status.progress.pending_keys, Some(2));
    assert!(first.revision < latest.revision);
    assert!(initial_revision < event.revision);
}

#[tokio::test]
async fn progress_persistence_and_events_are_coalesced_to_one_hz() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite persistence fixture should connect");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("migration persistence schema should initialize");

    let runtime = StorageMigrationRuntime::load(&config, pool.clone())
        .await
        .expect("migration runtime should load");
    let mut events = runtime.subscribe();
    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");
    events.changed().await.expect("start event is retained");

    let first = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::Copying,
            pending_keys: Some(10),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("first progress is accepted");
    assert_eq!(runtime.progress_publish_count_for_test(), 1);
    events
        .changed()
        .await
        .expect("first progress event is retained");
    assert_eq!(events.borrow_and_update().status.revision, first.revision);

    let latest = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::CatchingUp,
            pending_keys: Some(2),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("latest progress is accepted");
    let newest = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::CatchingUp,
            pending_keys: Some(1),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("newest progress is accepted");
    assert_eq!(runtime.progress_publish_count_for_test(), 1);
    assert!(latest.revision < newest.revision);

    let in_memory = runtime.snapshot().await;
    assert_eq!(in_memory.progress.pending_keys, Some(1));
    assert_eq!(in_memory.revision, newest.revision);
    let persisted_before_flush: StorageMigrationResponse = serde_json::from_str(
        &sqlx::query_scalar::<_, String>(
            "SELECT response_json FROM storage_migration_runtime WHERE migration_id = ?1",
        )
        .bind("storage-migration")
        .fetch_one(&pool)
        .await
        .expect("first progress should be persisted"),
    )
    .expect("persisted response should decode");
    assert_eq!(persisted_before_flush.progress.pending_keys, Some(10));

    let flushed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            events
                .changed()
                .await
                .expect("coalesced progress event is retained");
            let event = events.borrow_and_update().clone();
            if event.kind == "progress" && event.status.progress.pending_keys == Some(1) {
                break event;
            }
        }
    })
    .await
    .expect("latest progress should eventually be published");
    assert_eq!(runtime.progress_publish_count_for_test(), 2);
    assert_eq!(flushed.revision, newest.revision);
    assert_eq!(flushed.status.revision, newest.revision);

    let persisted_after_flush: StorageMigrationResponse = serde_json::from_str(
        &sqlx::query_scalar::<_, String>(
            "SELECT response_json FROM storage_migration_runtime WHERE migration_id = ?1",
        )
        .bind("storage-migration")
        .fetch_one(&pool)
        .await
        .expect("coalesced progress should be persisted"),
    )
    .expect("persisted coalesced response should decode");
    assert_eq!(persisted_after_flush.progress.pending_keys, Some(1));
    assert_eq!(persisted_after_flush.revision, newest.revision);
}

#[tokio::test]
async fn durable_progress_bypasses_the_coalescing_window() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite persistence fixture should connect");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("migration persistence schema should initialize");

    let runtime = StorageMigrationRuntime::load(&config, pool.clone())
        .await
        .expect("migration runtime should load");
    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");
    runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::Copying,
            pending_keys: Some(1),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("initial progress is accepted");
    for phase in [
        StorageMigrationState::CatchingUp,
        StorageMigrationState::Verifying,
        StorageMigrationState::Ready,
    ] {
        runtime
            .publish_progress(StorageMigrationProgress {
                phase,
                ..StorageMigrationProgress::default()
            })
            .await
            .expect("migration phase is accepted");
    }

    let switching = runtime
        .publish_progress_durable(StorageMigrationProgress {
            phase: StorageMigrationState::Switching,
            captured_through: Some(2),
            applied_through: Some(2),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("cutover progress should persist immediately");
    assert_eq!(runtime.progress_publish_count_for_test(), 2);

    let persisted: StorageMigrationResponse = serde_json::from_str(
        &sqlx::query_scalar::<_, String>(
            "SELECT response_json FROM storage_migration_runtime WHERE migration_id = ?1",
        )
        .bind("storage-migration")
        .fetch_one(&pool)
        .await
        .expect("durable cutover progress should be persisted"),
    )
    .expect("persisted response should decode");
    assert_eq!(persisted.state, StorageMigrationState::Switching);
    assert_eq!(persisted.progress.captured_through, Some(2));
    assert_eq!(persisted.progress.applied_through, Some(2));
    assert_eq!(persisted.revision, switching.revision);
}

#[tokio::test]
async fn pg17_simulated_target_commit_with_lost_source_confirmation_is_safe_to_retry()
-> anyhow::Result<()> {
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
    let source = SqliteMigrationSource::new(
        source_pool.clone(),
        "pg17-source-confirmation",
        "sqlite-pg17",
        "migration_change_log",
        vec![MigrationTableSpec::canonical(
            StorageModule::TrafficArchive,
            "events",
        )?],
    )?;
    source.ensure_capture_schema().await?;

    let mut source_transaction = source_pool.begin().await?;
    for (stable_key, source_version, payload) in [
        ("applied", 1_i64, b"committed".as_slice()),
        ("pending", 2_i64, b"still-pending".as_slice()),
    ] {
        sqlx::query("INSERT INTO events (stable_key, source_version, payload) VALUES (?, ?, ?)")
            .bind(stable_key)
            .bind(source_version)
            .bind(payload)
            .execute(&mut *source_transaction)
            .await?;
        source
            .append_captured_change_tx(
                &mut source_transaction,
                &CapturedChange::from_input(
                    "pg17-source-confirmation",
                    "sqlite-pg17",
                    source_version,
                    ChangeInput::insert("events", stable_key, payload.to_vec()),
                )?,
            )
            .await?;
    }
    source_transaction.commit().await?;

    // This is a deterministic SQLite model of a target transaction that committed before the
    // caller lost the source-acknowledgement result; it is not PostgreSQL or process-restart
    // evidence.
    let target_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    sqlx::query(
        "CREATE TABLE target_events (
            stable_key TEXT PRIMARY KEY,
            source_version INTEGER NOT NULL,
            payload BLOB NOT NULL
        )",
    )
    .execute(&target_pool)
    .await?;
    let mut target_transaction = target_pool.begin().await?;
    sqlx::query("INSERT INTO target_events (stable_key, source_version, payload) VALUES (?, ?, ?)")
        .bind("applied")
        .bind(1_i64)
        .bind(b"committed".as_slice())
        .execute(&mut *target_transaction)
        .await?;
    target_transaction.commit().await?;

    let lost_ack: anyhow::Result<()> = async {
        source.release_capture_through(1).await?;
        anyhow::bail!("simulated loss of source acknowledgement after commit");
    }
    .await;
    assert!(lost_ack.is_err());

    // A recovery retry is harmless after either side of the source commit/ack boundary has
    // already completed, and it must not consume the newer source version.
    assert_eq!(source.release_capture_through(1).await?, 0);
    let pending =
        MigrationSource::changes_through(&source, 1, 2, ChangeBatchLimits::default()).await?;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].stable_key, "pending");

    let source_fact_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(&source_pool)
        .await?;
    assert_eq!(
        source_fact_count, 2,
        "source business facts were not deleted"
    );

    // Reapplying the committed target fact by its stable identity models the idempotent target
    // side of recovery and must not create a second business row.
    sqlx::query(
        "INSERT INTO target_events (stable_key, source_version, payload) VALUES (?, ?, ?)
         ON CONFLICT(stable_key) DO UPDATE SET
           source_version = excluded.source_version,
           payload = excluded.payload",
    )
    .bind("applied")
    .bind(1_i64)
    .bind(b"committed".as_slice())
    .execute(&target_pool)
    .await?;
    let target_fact_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM target_events WHERE stable_key = 'applied'")
            .fetch_one(&target_pool)
            .await?;
    assert_eq!(target_fact_count, 1);
    Ok(())
}

#[tokio::test]
async fn pg17_long_sqlite_connection_preserves_final_postgres_status_after_switch() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite persistence fixture should connect");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("migration persistence schema should initialize");

    let runtime = Arc::new(
        StorageMigrationRuntime::load(&config, pool.clone())
            .await
            .expect("migration runtime should load"),
    );
    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");
    for phase in [
        StorageMigrationState::Copying,
        StorageMigrationState::CatchingUp,
        StorageMigrationState::Verifying,
        StorageMigrationState::Ready,
    ] {
        runtime
            .publish_progress(StorageMigrationProgress {
                phase,
                ..StorageMigrationProgress::default()
            })
            .await
            .expect("migration phase is accepted");
    }

    let held_connection = pool
        .acquire()
        .await
        .expect("the long SQLite connection should be available");
    let switching_runtime = runtime.clone();
    let switching = tokio::spawn(async move {
        switching_runtime
            .publish_progress_durable(StorageMigrationProgress {
                phase: StorageMigrationState::Switching,
                captured_through: Some(7),
                applied_through: Some(7),
                ..StorageMigrationProgress::default()
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while !runtime.progress_persistence_in_flight_for_test() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("switching status should be blocked by the long connection");

    let postgres_runtime = runtime.clone();
    let postgres =
        tokio::spawn(async move { postgres_runtime.publish_postgres_authority(2, 7).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = runtime.snapshot().await;
            if status.state == StorageMigrationState::Postgres && status.authority.generation == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("newer Postgres completion should win in memory while persistence is blocked");

    drop(held_connection);
    switching
        .await
        .expect("switching persistence task should join")
        .expect("stale switching persistence should resolve to the newer snapshot");
    postgres
        .await
        .expect("Postgres authority persistence task should join")
        .expect("Postgres authority should persist after the connection is released");

    let recovered = StorageMigrationRuntime::load(&config, pool)
        .await
        .expect("migration runtime should reload the final snapshot");
    let status = recovered.snapshot().await;
    assert_eq!(status.state, StorageMigrationState::Postgres);
    assert_eq!(status.authority.generation, 2);
    assert_eq!(status.progress.captured_through, Some(7));
    assert_eq!(status.progress.applied_through, Some(7));
    assert_eq!(status.switch.result.as_deref(), Some("published"));
    assert!(!status.task_holds.held);
}

#[tokio::test]
async fn progress_flush_releases_runtime_lock_during_persistence() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite persistence fixture should connect");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("migration persistence schema should initialize");

    let runtime = StorageMigrationRuntime::load(&config, pool.clone())
        .await
        .expect("migration runtime should load");
    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");
    runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::Copying,
            pending_keys: Some(10),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("first progress is accepted");

    let held_connection = pool
        .acquire()
        .await
        .expect("the single SQLite connection should be available");
    runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::CatchingUp,
            pending_keys: Some(2),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("coalesced progress is accepted");

    tokio::time::timeout(Duration::from_secs(2), async {
        while !runtime.progress_persistence_in_flight_for_test() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("progress flush should reach SQLite while the connection is held");

    let snapshot = tokio::time::timeout(Duration::from_millis(250), runtime.snapshot())
        .await
        .expect("snapshot should not wait for the SQLite flush");
    assert_eq!(snapshot.progress.pending_keys, Some(2));

    drop(held_connection);
    tokio::time::timeout(Duration::from_secs(2), async {
        while runtime.progress_publish_count_for_test() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the blocked progress flush should complete after SQLite is released");
}

#[tokio::test]
async fn control_and_immediate_progress_persistence_do_not_block_snapshot() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite persistence fixture should connect");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("migration persistence schema should initialize");

    let runtime = Arc::new(
        StorageMigrationRuntime::load(&config, pool.clone())
            .await
            .expect("migration runtime should load"),
    );
    let held_connection = pool
        .acquire()
        .await
        .expect("the single SQLite connection should be available");
    let control = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .apply_control(StorageMigrationControlRequest { paused: false })
                .await
        })
    };

    tokio::time::timeout(Duration::from_secs(2), async {
        while !runtime.progress_persistence_in_flight_for_test() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("control persistence should reach SQLite while the connection is held");
    let snapshot = tokio::time::timeout(Duration::from_millis(250), runtime.snapshot())
        .await
        .expect("snapshot should not wait for control persistence");
    assert_eq!(snapshot.state, StorageMigrationState::Preparing);

    drop(held_connection);
    control
        .await
        .expect("control task should join")
        .expect("migration should start");

    let held_connection = pool
        .acquire()
        .await
        .expect("the single SQLite connection should be available again");
    let progress = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .publish_progress(StorageMigrationProgress {
                    phase: StorageMigrationState::Copying,
                    pending_keys: Some(7),
                    ..StorageMigrationProgress::default()
                })
                .await
        })
    };

    tokio::time::timeout(Duration::from_secs(2), async {
        while !runtime.progress_persistence_in_flight_for_test() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("immediate progress persistence should reach SQLite while the connection is held");
    let snapshot = tokio::time::timeout(Duration::from_millis(250), runtime.snapshot())
        .await
        .expect("snapshot should not wait for immediate progress persistence");
    assert_eq!(snapshot.state, StorageMigrationState::Copying);
    assert_eq!(snapshot.progress.pending_keys, Some(7));

    drop(held_connection);
    progress
        .await
        .expect("progress task should join")
        .expect("copying progress should persist");
}

#[tokio::test]
async fn newer_control_snapshot_wins_over_a_blocked_progress_flush() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite persistence fixture should connect");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("migration persistence schema should initialize");

    let runtime = Arc::new(
        StorageMigrationRuntime::load(&config, pool.clone())
            .await
            .expect("migration runtime should load"),
    );
    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("migration should start");
    runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::Copying,
            pending_keys: Some(10),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("copying progress should persist");

    let held_connection = pool
        .acquire()
        .await
        .expect("the single SQLite connection should be available");
    runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::CatchingUp,
            pending_keys: Some(2),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("catch-up progress should be coalesced");
    tokio::time::timeout(Duration::from_secs(2), async {
        while !runtime.progress_persistence_in_flight_for_test() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the coalesced progress flush should reach SQLite while the connection is held");

    let pause = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .apply_control(StorageMigrationControlRequest { paused: true })
                .await
        })
    };
    let snapshot = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            let snapshot = runtime.snapshot().await;
            if snapshot.state == StorageMigrationState::Paused {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("snapshot should not wait for the blocked flush or pause persistence");
    assert_eq!(snapshot.state, StorageMigrationState::Paused);

    drop(held_connection);
    let paused = pause
        .await
        .expect("pause task should join")
        .expect("pause should persist");
    let recovered = StorageMigrationRuntime::load(&config, pool)
        .await
        .expect("migration runtime should reload the newest snapshot");
    let status = recovered.snapshot().await;
    assert_eq!(status.state, StorageMigrationState::Paused);
    assert_eq!(status.revision, paused.revision);
    assert_eq!(status.progress.pending_keys, Some(2));
}

#[tokio::test]
async fn durable_status_recovers_pause_progress_and_revision_after_restart() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite persistence fixture should connect");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("migration persistence schema should initialize");

    let runtime = StorageMigrationRuntime::load(&config, pool.clone())
        .await
        .expect("initial migration runtime should load");
    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");
    runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::Copying,
            committed_rows: Some(12),
            committed_bytes: Some(4_096),
            source_rows: Some(100),
            source_bytes: Some(32_768),
            scan_cursor: Some("codex_invocations:12".to_string()),
            captured_through: Some(42),
            applied_through: Some(41),
            pending_keys: Some(3),
            oldest_change_age_ms: Some(250),
            replay_throughput_per_sec: Some(12.5),
            incoming_rate_per_sec: Some(4.5),
            converged: Some(true),
            eta_ms: Some(1_234),
        })
        .await
        .expect("copying progress is accepted");
    let paused = runtime
        .apply_control(StorageMigrationControlRequest { paused: true })
        .await
        .expect("pause is accepted");
    drop(runtime);

    let recovered = StorageMigrationRuntime::load(&config, pool)
        .await
        .expect("migration runtime should recover persisted state");
    let status = recovered.snapshot().await;
    assert_eq!(status.state, StorageMigrationState::Paused);
    assert_eq!(status.revision, paused.revision);
    assert_eq!(status.progress.committed_rows, Some(12));
    assert_eq!(status.progress.committed_bytes, Some(4_096));
    assert_eq!(status.progress.source_rows, Some(100));
    assert_eq!(status.progress.source_bytes, Some(32_768));
    assert_eq!(
        status.progress.scan_cursor.as_deref(),
        Some("codex_invocations:12")
    );
    assert_eq!(status.progress.captured_through, Some(42));
    assert_eq!(status.progress.applied_through, Some(41));
    assert_eq!(status.progress.pending_keys, Some(3));
    assert_eq!(status.progress.oldest_change_age_ms, Some(250));
    assert_eq!(status.progress.replay_throughput_per_sec, Some(12.5));
    assert_eq!(status.progress.incoming_rate_per_sec, Some(4.5));
    assert_eq!(status.progress.converged, Some(true));
    assert_eq!(status.progress.eta_ms, Some(1_234));
    assert_eq!(status.progress.phase, StorageMigrationState::Paused);
    assert_eq!(status.task_holds.reason.as_deref(), Some("operator_pause"));
    assert!(!status.task_holds.held);
    assert_eq!(status.authority.backend, StorageMigrationBackend::Sqlite);
}

#[tokio::test]
async fn progress_persistence_failure_restores_the_previous_runtime_snapshot() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("SQLite persistence fixture should connect");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("migration persistence schema should initialize");

    let runtime = StorageMigrationRuntime::load(&config, pool.clone())
        .await
        .expect("migration runtime should load");
    let before = runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("migration should start before closing persistence");
    pool.close().await;

    let error = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::Copying,
            committed_rows: Some(12),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect_err("progress persistence should fail after the pool closes");
    assert!(
        error
            .to_string()
            .contains("persist storage migration status")
    );

    let after = runtime.snapshot().await;
    assert_eq!(after.state, before.state);
    assert_eq!(after.revision, before.revision);
    assert_eq!(
        after.progress.committed_rows,
        before.progress.committed_rows
    );
}

#[tokio::test]
async fn postgres_cutover_publishes_postgres_authority_and_completed_module() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let runtime = StorageMigrationRuntime::new(&config);
    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");
    for phase in [
        StorageMigrationState::Copying,
        StorageMigrationState::CatchingUp,
        StorageMigrationState::Verifying,
        StorageMigrationState::Ready,
        StorageMigrationState::Switching,
    ] {
        runtime
            .publish_progress(StorageMigrationProgress {
                phase,
                ..StorageMigrationProgress::default()
            })
            .await
            .expect("migration phase is accepted");
    }

    let switching = runtime.snapshot().await;
    assert_eq!(switching.authority.backend, StorageMigrationBackend::Sqlite);
    assert_eq!(switching.authority.generation, 1);
    let error = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::Postgres,
            ..StorageMigrationProgress::default()
        })
        .await
        .expect_err("status progress cannot publish PostgreSQL before authority commit");
    assert!(error.to_string().contains("durable authority manifest"));
    let mut events = runtime.subscribe();
    runtime
        .publish_postgres_authority(2, 0)
        .await
        .expect("durable authority publication is accepted");
    events
        .changed()
        .await
        .expect("published status event is retained");
    assert_eq!(events.borrow_and_update().kind, "status");

    let status = runtime.snapshot().await;
    assert_eq!(status.authority.backend, StorageMigrationBackend::Postgres);
    assert_eq!(status.authority.generation, 2);
    assert_eq!(
        status.recovery.authority_backend,
        Some(StorageMigrationBackend::Postgres)
    );
    assert_eq!(status.completed_modules, vec!["traffic_archive"]);
    assert!(!status.task_holds.held);
}

#[tokio::test]
async fn migration_hold_admission_preserves_non_competing_tasks() {
    let config = StorageConfig {
        postgres_database_url: Some("postgresql://private.example/db".to_string()),
        ..StorageConfig::default()
    };
    let runtime = StorageMigrationRuntime::new(&config);
    runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("start is accepted");

    assert_eq!(
        runtime.migration_hold_reason("retention_archive").await,
        Some("migration_hold")
    );
    assert_eq!(
        runtime
            .migration_hold_reason("startup_backfill.historical_rollups")
            .await,
        Some("migration_hold")
    );
    assert_eq!(runtime.migration_hold_reason("oauth_refresh").await, None);
}

#[test]
fn migration_control_authentication_fails_closed_without_the_dedicated_token() {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer migration-admin-token-1234"),
    );
    let unconfigured = StorageConfig::default();
    let error = authorize_control_request(&headers, &unconfigured)
        .expect_err("control must stay closed when the token is unconfigured");
    assert_eq!(error.0, StatusCode::FORBIDDEN);

    let configured = StorageConfig {
        migration_admin_token: Some("migration-admin-token-1234".to_string()),
        ..StorageConfig::default()
    };
    let missing = authorize_control_request(&HeaderMap::new(), &configured)
        .expect_err("missing token must be unauthorized");
    assert_eq!(missing.0, StatusCode::UNAUTHORIZED);

    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer wrong-migration-admin-token"),
    );
    let invalid = authorize_control_request(&headers, &configured)
        .expect_err("invalid token must be unauthorized");
    assert_eq!(invalid.0, StatusCode::UNAUTHORIZED);

    headers.insert(
        header::AUTHORIZATION,
        HeaderValue::from_static("Bearer migration-admin-token-1234"),
    );
    authorize_control_request(&headers, &configured).expect("the dedicated token authorizes");
}
