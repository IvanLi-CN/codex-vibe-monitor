use super::*;

const RETENTION_TASK_KEY: &str = "retention_archive";

struct PersistedTaskObservation {
    run_id: i64,
    execution_uid: String,
    actual_started_at: String,
    actual_finished_at: String,
    actual_duration_ms: i64,
    status: String,
    details: serde_json::Value,
    sample: serde_json::Value,
}

type PersistedTaskRow = (
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    String,
    Option<String>,
);

async fn seed_offline_cli_retention_fixture(pool: &SqlitePool, config: &AppConfig) {
    let expired_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    let recent_at = shanghai_local_days_ago(0, 0, 0, 0);
    let expired_payload = format!(r#"{{"padding":"{}"}}"#, "x".repeat(16 * 1024));

    for index in 0..16 {
        insert_retention_invocation(
            pool,
            &format!("offline-cli-expired-{index}"),
            &expired_at,
            SOURCE_PROXY,
            "success",
            Some(&expired_payload),
            "{}",
            None,
            None,
            Some(7),
            Some(0.07),
        )
        .await;
    }
    for index in 0..32 {
        insert_retention_invocation(
            pool,
            &format!("offline-cli-recent-{index}"),
            &recent_at,
            SOURCE_PROXY,
            "success",
            Some("{}"),
            "{}",
            None,
            None,
            Some(7),
            Some(0.07),
        )
        .await;
    }
}

async fn load_persisted_task_observation(pool: &SqlitePool) -> PersistedTaskObservation {
    let (
        run_id,
        execution_uid,
        actual_started_at,
        actual_finished_at,
        actual_duration_ms,
        status,
        details,
    ): PersistedTaskRow = sqlx::query_as(
        "SELECT id,execution_uid,actual_started_at,actual_finished_at,actual_duration_ms,status,details
         FROM managed_task_runs
         WHERE task_key=?1
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(RETENTION_TASK_KEY)
    .fetch_one(pool)
    .await
    .expect("load offline retention task run");
    let execution_uid = execution_uid.expect("offline task must have an execution uid");
    let actual_started_at = actual_started_at.expect("offline task must persist actual start");
    let actual_finished_at = actual_finished_at.expect("offline task must persist actual finish");
    let actual_duration_ms = actual_duration_ms.expect("offline task must persist actual duration");
    let details = details
        .and_then(|details| serde_json::from_str(&details).ok())
        .expect("offline task must persist JSON details");
    let sample_json: String = sqlx::query_scalar(
        "SELECT sample_json
         FROM managed_task_work_runs
         WHERE managed_run_id=?1 AND task_key=?2",
    )
    .bind(run_id)
    .bind(RETENTION_TASK_KEY)
    .fetch_one(pool)
    .await
    .expect("offline task must persist a workload sample");
    let sample = serde_json::from_str(&sample_json).expect("workload sample must be JSON");

    PersistedTaskObservation {
        run_id,
        execution_uid,
        actual_started_at,
        actual_finished_at,
        actual_duration_ms,
        status,
        details,
        sample,
    }
}

fn assert_close(actual: f64, expected: f64, label: &str) {
    let tolerance = expected.abs().max(1.0) * 1e-9;
    assert!(
        (actual - expected).abs() <= tolerance,
        "{label}: actual={actual}, expected={expected}, tolerance={tolerance}"
    );
}

fn assert_persisted_timing(observation: &PersistedTaskObservation, status: &str) {
    assert!(!observation.execution_uid.is_empty());
    assert!(!observation.actual_started_at.is_empty());
    assert!(!observation.actual_finished_at.is_empty());
    assert!(observation.actual_duration_ms >= 0);
    assert_eq!(observation.status, status);
    assert_eq!(
        observation.details["elapsedMs"].as_i64(),
        Some(observation.actual_duration_ms),
        "owner details and timeline must share the frozen execution duration"
    );
    assert_eq!(
        observation.sample["executionUid"].as_str(),
        Some(observation.execution_uid.as_str())
    );
    assert_eq!(
        observation.sample["managedRunId"].as_i64(),
        Some(observation.run_id)
    );
    assert_eq!(
        observation.sample["durationMs"].as_i64(),
        Some(observation.actual_duration_ms),
        "workload sample and managed run must share the frozen execution duration"
    );
    assert_eq!(observation.sample["status"].as_str(), Some(status));
}

fn assert_retention_rates_use_persisted_duration(observation: &PersistedTaskObservation) {
    assert!(
        observation.actual_duration_ms > 0,
        "archive fixture must take at least one millisecond so service rates are observable"
    );
    let run_seconds = observation.actual_duration_ms as f64 / 1_000.0;
    let batches = observation.details["archiveBatches"]
        .as_array()
        .expect("retention owner details must include archive batches");
    assert!(!batches.is_empty(), "archive fixture must commit a batch");
    for batch in batches {
        let committed_rows = batch["committedRows"]
            .as_u64()
            .expect("archive batch must record committed rows");
        assert!(committed_rows > 0);
        let service_rate = batch["committedRowsPerSecond"]
            .as_f64()
            .expect("archive batch must record its service rate");
        let arrival_rate = batch["arrivalRowsPerSecond"]
            .as_f64()
            .filter(|rate| *rate > 0.0)
            .expect("archive batch must record a positive arrival rate");
        let service_rate_multiple = batch["serviceRateMultiple"]
            .as_f64()
            .expect("archive batch must record its service-rate multiple");
        assert_close(
            service_rate,
            committed_rows as f64 / run_seconds,
            "committed rows per second",
        );
        assert_close(
            service_rate_multiple,
            service_rate / arrival_rate,
            "service-rate multiple",
        );
    }
}

#[tokio::test]
async fn offline_retention_cli_persists_frozen_owner_timing_and_rates() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("offline-cli-task-observation").await;
    seed_offline_cli_retention_fixture(&pool, &config).await;
    pool.close().await;

    crate::maintenance::run_owned_maintenance_cli(&config, RETENTION_TASK_KEY, false)
        .await
        .expect("offline retention CLI should complete successfully");

    let maintenance_pool = SqlitePool::connect(&test_sqlite_url_for_path(
        &config.maintenance_database_path(),
    ))
    .await
    .expect("open persisted maintenance database");
    let observation = load_persisted_task_observation(&maintenance_pool).await;
    assert_persisted_timing(&observation, "success");
    assert_eq!(observation.details["ownershipVersion"].as_i64(), Some(2));
    assert_eq!(observation.details["result"].as_str(), Some("complete"));
    assert_eq!(observation.details["committed"].as_i64(), Some(16));
    assert_retention_rates_use_persisted_duration(&observation);

    maintenance_pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn offline_retention_cli_keeps_committed_owner_complete_when_backlog_observation_fails() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("offline-cli-backlog-observation-failure").await;
    seed_offline_cli_retention_fixture(&pool, &config).await;
    pool.close().await;

    crate::maintenance::RETENTION_TEST_BACKLOG_OBSERVATION_FAILURE
        .scope(
            (),
            crate::maintenance::run_owned_maintenance_cli(&config, RETENTION_TASK_KEY, false),
        )
        .await
        .expect("offline retention CLI should preserve a completed owner result");

    let source_pool = SqlitePool::connect(&test_sqlite_url_for_path(&config.database_path))
        .await
        .expect("open converted source database");
    let remaining_expired: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations
         WHERE invoke_id LIKE 'offline-cli-expired-%'",
    )
    .fetch_one(&source_pool)
    .await
    .expect("count converted source rows");
    assert_eq!(remaining_expired, 0);
    let remaining_recent: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations
         WHERE invoke_id LIKE 'offline-cli-recent-%'",
    )
    .fetch_one(&source_pool)
    .await
    .expect("count retained source rows");
    assert_eq!(remaining_recent, 32);
    let archived_rows: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(row_count), 0) FROM archive_batches
         WHERE dataset = 'codex_invocations' AND status = 'completed'",
    )
    .fetch_one(&source_pool)
    .await
    .expect("count completed archive rows");
    assert_eq!(archived_rows, 16);
    let archive_paths: Vec<String> = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches
         WHERE dataset = 'codex_invocations' AND status = 'completed'",
    )
    .fetch_all(&source_pool)
    .await
    .expect("load completed archive paths");
    assert!(!archive_paths.is_empty());
    for archive_path in archive_paths {
        assert!(
            Path::new(&archive_path).is_file(),
            "completed archive file should exist: {archive_path}"
        );
    }
    source_pool.close().await;

    let maintenance_pool = SqlitePool::connect(&test_sqlite_url_for_path(
        &config.maintenance_database_path(),
    ))
    .await
    .expect("open persisted maintenance database");
    let observation = load_persisted_task_observation(&maintenance_pool).await;
    assert_persisted_timing(&observation, "success");
    assert_eq!(observation.details["ownershipVersion"].as_i64(), Some(2));
    assert_eq!(observation.details["result"].as_str(), Some("complete"));
    assert_eq!(observation.details["committed"].as_i64(), Some(16));
    assert_eq!(
        observation.details["waitReason"].as_str(),
        Some("backlog_unknown")
    );
    assert_eq!(
        observation.details["selectedScope"]["hasMore"].as_bool(),
        Some(true)
    );

    maintenance_pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn offline_retention_cli_persists_frozen_owner_timing_on_failure() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("offline-cli-task-observation-failure").await;
    seed_offline_cli_retention_fixture(&pool, &config).await;
    sqlx::query(
        r#"
        CREATE TRIGGER offline_cli_abort_archive_finalize
        BEFORE UPDATE OF status ON archive_batches
        WHEN NEW.status = 'completed'
        BEGIN
            SELECT RAISE(ABORT, 'offline CLI archive finalization failure');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("install offline CLI failure trigger");
    pool.close().await;

    let error = crate::maintenance::run_owned_maintenance_cli(&config, RETENTION_TASK_KEY, false)
        .await
        .expect_err("offline retention CLI should report the injected owner failure");
    assert!(error.to_string().contains("maintenance owner failed"));

    let maintenance_pool = SqlitePool::connect(&test_sqlite_url_for_path(
        &config.maintenance_database_path(),
    ))
    .await
    .expect("open persisted failed maintenance database");
    let observation = load_persisted_task_observation(&maintenance_pool).await;
    assert_persisted_timing(&observation, "failed");
    assert_eq!(observation.details["ownershipVersion"].as_i64(), Some(2));
    assert_eq!(observation.details["result"].as_str(), Some("failed"));

    maintenance_pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
