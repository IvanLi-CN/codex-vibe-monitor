use crate::storage::config::StorageConfig;
use crate::storage::migration_status::{
    StorageMigrationControlAction, StorageMigrationControlRequest, StorageMigrationProgress,
    StorageMigrationRuntime, StorageMigrationState,
};

#[tokio::test]
async fn default_status_exposes_safe_target_and_unknown_progress() {
    let runtime = StorageMigrationRuntime::default();
    let status = runtime.snapshot().await;

    assert_eq!(status.state, StorageMigrationState::Pending);
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
        .apply_control(StorageMigrationControlRequest {
            action: StorageMigrationControlAction::Start,
        })
        .await
        .expect("start is accepted");
    assert_eq!(started.state, StorageMigrationState::Preparing);
    assert!(started.controls.can_pause);

    let paused = runtime
        .apply_control(StorageMigrationControlRequest {
            action: StorageMigrationControlAction::Pause,
        })
        .await
        .expect("pause is accepted");
    assert_eq!(paused.state, StorageMigrationState::Paused);
    assert_eq!(paused.task_holds.reason.as_deref(), Some("operator_pause"));

    let resumed = runtime
        .apply_control(StorageMigrationControlRequest {
            action: StorageMigrationControlAction::Resume,
        })
        .await
        .expect("resume is accepted");
    assert_eq!(resumed.state, StorageMigrationState::Preparing);
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
        .apply_control(StorageMigrationControlRequest {
            action: StorageMigrationControlAction::Start,
        })
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
