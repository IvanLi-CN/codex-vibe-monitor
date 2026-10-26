use crate::storage::config::StorageConfig;
use crate::storage::migration_status::{
    StorageMigrationControlRequest, StorageMigrationProgress, StorageMigrationRuntime,
    StorageMigrationState,
};

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

    let resumed = runtime
        .apply_control(StorageMigrationControlRequest { paused: false })
        .await
        .expect("resume is accepted");
    assert_eq!(resumed.state, StorageMigrationState::Preparing);
    assert_eq!(resumed.revision, paused.revision + 1);
    assert_eq!(resumed.progress.phase, resumed.state);
    assert!(!resumed.task_holds.held);
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
    let latest = runtime
        .publish_progress(StorageMigrationProgress {
            phase: StorageMigrationState::CatchingUp,
            pending_keys: Some(2),
            ..StorageMigrationProgress::default()
        })
        .await
        .expect("latest progress is accepted");

    events
        .changed()
        .await
        .expect("latest progress event is retained");
    let event = events.borrow().clone();
    assert_eq!(event.kind, "progress");
    assert_eq!(event.revision, latest.revision);
    assert_eq!(event.status.revision, latest.revision);
    assert_eq!(event.status.progress.pending_keys, Some(2));
    assert!(first.revision < latest.revision);
    assert!(initial_revision < event.revision);
}
