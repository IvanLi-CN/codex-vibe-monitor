use super::*;
use crate::maintenance_store::TaskWorkloadSample;

#[tokio::test]
async fn managed_raw_payload_inventory_refreshes_without_pending_reset() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream URL"),
    )
    .await;
    sqlx::query(
        "UPDATE system_raw_payload_metrics SET inventory_state = 'preparing', inventory_cursor = 0, link_inventory_cursor = 0, inventory_recheck_cursor = '', inventory_recheck_active = 0 WHERE singleton = 1",
    )
    .execute(&state.pool)
    .await
    .expect("seed inventory preparing state");

    let summary = crate::runtime::managed_task_dispatch_tests::run_managed_task_once_for_test(
        &state,
        "raw_payload_metrics_inventory",
    )
    .await
    .expect("managed inventory task should refresh its snapshot");

    assert_eq!(summary, "原始载荷指标盘点已推进");
    let inventory_state: String = sqlx::query_scalar(
        "SELECT inventory_state FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("read refreshed inventory state");
    assert_eq!(inventory_state, "ready");
}

async fn coverage_repair_fixture() -> (Arc<AppState>, i64) {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream URL"),
    )
    .await;
    let current_hour = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let oldest_hour = current_hour - 2 * 3_600;
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, 1, datetime('now'))",
    )
    .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET)
    .execute(&state.pool)
    .await
    .expect("seed repair generation");
    let occurred_at = Utc
        .timestamp_opt(oldest_hour + 600, 0)
        .single()
        .expect("source time");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, detail_level, total_tokens, payload, raw_response) \
         VALUES ('workload-coverage', ?1, ?2, 'success', ?3, 10, ?4, '{}')",
    ).bind(format_naive(occurred_at.with_timezone(&Shanghai).naive_local()))
        .bind(SOURCE_PROXY).bind(DETAIL_LEVEL_FULL).bind(r#"{"upstreamAccountId":42}"#)
        .execute(&state.pool).await.expect("seed two eligible closed hours");
    let task_name = startup_backfill_task_progress_key(
        state.as_ref(),
        StartupBackfillTask::AccountActivityV2Coverage,
    )
    .await;
    let due = format_utc_iso(Utc::now() - ChronoDuration::seconds(1));
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 0,
            scanned: 0,
            updated: 0,
            zero_update_streak: 0,
            next_run_after: &due,
            status: STARTUP_BACKFILL_STATUS_IDLE,
            suspension_reason: None,
        },
    )
    .await
    .expect("make coverage repair due");
    (state, oldest_hour)
}

async fn run_coverage_repair_and_read_sample(state: &Arc<AppState>) -> TaskWorkloadSample {
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    run_startup_backfill_task_if_due_with_gate(
        state,
        StartupBackfillTask::AccountActivityV2Coverage,
        &gate,
    )
    .await
    .expect("coverage repair returns its existing task outcome");
    crate::task_runtime_observation::workload_sample(
        "startup_backfill.account_activity_v2_coverage",
    )
    .expect("coverage repair records the child attempt")
}

async fn covered_hours(state: &Arc<AppState>) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT bucket_start_epoch FROM hourly_rollup_materialized_buckets \
         WHERE target = ?1 AND source = ?2 ORDER BY bucket_start_epoch",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .fetch_all(&state.pool)
    .await
    .expect("read durable coverage markers")
}

#[tokio::test]
async fn coverage_repair_failed_later_bucket_retains_committed_workload() {
    let (state, oldest_hour) = coverage_repair_fixture().await;
    let maintenance_pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect independent maintenance store");
    let store = crate::maintenance_store::MaintenanceStore::from_pool(maintenance_pool.clone());
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance schema");
    crate::task_timeline::start_recorder(Arc::new(store)).await;
    sqlx::query(&format!(
        "CREATE TRIGGER fail_second_coverage_bucket \
         BEFORE INSERT ON hourly_rollup_materialized_buckets \
         WHEN NEW.bucket_start_epoch = {oldest_hour} \
         BEGIN SELECT RAISE(ABORT, 'injected coverage bucket failure'); END",
    ))
    .execute(&state.pool)
    .await
    .expect("fail after the first bucket commits");

    let sample = run_coverage_repair_and_read_sample(&state).await;

    assert_eq!(covered_hours(&state).await, vec![oldest_hour + 3_600]);
    assert_eq!(sample.status, "failed");
    assert_eq!(
        sample.processed.as_ref().and_then(|metric| metric.value),
        Some(1)
    );
    assert_eq!(
        sample.processed.as_ref().map(|metric| metric.unit.as_str()),
        Some("account-activity buckets")
    );
    assert_eq!(
        sample
            .processed
            .as_ref()
            .map(|metric| metric.coverage.as_str()),
        Some("window")
    );
    assert!(sample.finished_at.is_some());
    crate::task_timeline::drain_after_shutdown().await;
    let sample_json: String = sqlx::query_scalar(
        "SELECT sample_json FROM managed_task_work_runs WHERE execution_uid=?1 AND task_key=?2",
    )
    .bind(&sample.execution_uid)
    .bind(&sample.task_key)
    .fetch_one(&maintenance_pool)
    .await
    .expect("read final recorder snapshot");
    let stored: TaskWorkloadSample =
        serde_json::from_str(&sample_json).expect("decode persisted sample");
    assert_eq!(stored.status, "failed");
    assert_eq!(stored.sequence, sample.sequence);
    assert_eq!(
        stored.processed.as_ref().and_then(|metric| metric.value),
        Some(1)
    );
}

#[tokio::test]
async fn coverage_repair_success_counts_committed_buckets_once() {
    let (state, oldest_hour) = coverage_repair_fixture().await;

    let sample = run_coverage_repair_and_read_sample(&state).await;

    assert_eq!(
        covered_hours(&state).await,
        vec![oldest_hour, oldest_hour + 3_600]
    );
    assert_eq!(sample.status, "success");
    assert_eq!(
        sample.processed.as_ref().and_then(|metric| metric.value),
        Some(2)
    );
}

#[tokio::test]
async fn coverage_repair_failure_before_commit_does_not_invent_workload() {
    let (state, _) = coverage_repair_fixture().await;
    sqlx::query(
        "CREATE TRIGGER fail_first_coverage_bucket \
         BEFORE INSERT ON hourly_rollup_materialized_buckets \
         BEGIN SELECT RAISE(ABORT, 'injected coverage bucket failure'); END",
    )
    .execute(&state.pool)
    .await
    .expect("fail before any bucket commits");

    let sample = run_coverage_repair_and_read_sample(&state).await;

    assert!(covered_hours(&state).await.is_empty());
    assert_eq!(sample.status, "failed");
    assert_eq!(
        sample.processed.as_ref().and_then(|metric| metric.value),
        None
    );
}
