use super::super::*;
use super::{
    assert_startup_backfill_busy_error_closes_gate_before_next_task,
    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task,
    seed_due_startup_backfill_progress,
};

const HOURLY_ROLLUP_BOOTSTRAP_TASK_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const HOURLY_ROLLUP_BOOTSTRAP_TASK_POLL_INTERVAL: Duration = Duration::from_millis(10);

async fn wait_for_hourly_rollup_bootstrap_task(
    state: &AppState,
    expected_status: &str,
) -> (String, Option<String>, Option<String>) {
    let mut last_task = None;
    let result = tokio::time::timeout(HOURLY_ROLLUP_BOOTSTRAP_TASK_WAIT_TIMEOUT, async {
        loop {
            if expected_status != "running" {
                state
                    .sqlite_batch_writer
                    .flush_now(&state.pool)
                    .await
                    .expect("flush deferred hourly rollup bootstrap task-history writes");
            }
            let task = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                r#"
                SELECT status, summary, detail
                FROM system_task_runs
                WHERE task_kind = 'hourly_rollup_bootstrap'
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .fetch_optional(&state.pool)
            .await
            .expect("read hourly rollup bootstrap task");
            last_task.clone_from(&task);
            if let Some(task) = task
                && task.0 == expected_status
            {
                return task;
            }
            tokio::time::sleep(HOURLY_ROLLUP_BOOTSTRAP_TASK_POLL_INTERVAL).await;
        }
    })
    .await;
    if result.is_err() {
        let pending = state.sqlite_batch_writer.accounting_snapshot();
        let all_tasks = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>)>(
            r#"
            SELECT id, status, summary, detail
            FROM system_task_runs
            WHERE task_kind = 'hourly_rollup_bootstrap'
            ORDER BY id DESC
            LIMIT 5
            "#,
        )
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();
        panic!(
            "hourly rollup bootstrap task should reach expected status {expected_status}; last_task={last_task:?}; all_tasks={all_tasks:?}; pending_depth={}; pending_bytes={}",
            pending.pending_depth, pending.pending_bytes,
        )
    }
    result.expect("hourly rollup bootstrap task should reach the expected state")
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_keeps_health_ready_while_waiting_for_lock() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    let (status, _, _) = wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;
    assert_eq!(status, "running");

    let health = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond while rollup bootstrap waits for the lock");
    assert_eq!(health.status(), StatusCode::OK);

    drop(rollup_guard);
    tokio::time::timeout(Duration::from_secs(5), bootstrap_handle)
        .await
        .expect("background bootstrap should finish")
        .expect("background bootstrap task should join");
    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "success").await;
    assert_eq!(status, "success");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("hourly_rollups_ms="))
    );
    assert!(detail.is_none());

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_defers_when_repair_cursor_remains_pending() {
    let state = test_state_from_config(test_config(), false).await;
    state.startup_ready.store(true, Ordering::Release);
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, 1, datetime('now'))",
    )
    .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET)
    .execute(&state.pool)
    .await
    .expect("seed current account activity v2 repair generation");
    let mut seed_tx = state
        .pool
        .begin()
        .await
        .expect("begin pending repair cursor seed");
    for id in 1..=513_i64 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, historical_rollups_materialized_at, created_at
            )
            VALUES (?1, 'codex_invocations', '2026-03', ?2, ?3, 1, 'completed',
                    'unknown', datetime('now'), datetime('now'))
            "#,
        )
        .bind(id)
        .bind(format!("/missing/runtime-pending-repair-{id}.sqlite.gz"))
        .bind(format!("runtime-pending-repair-sha-{id}"))
        .execute(&mut *seed_tx)
        .await
        .expect("seed pending repair cursor candidate");
    }
    seed_tx
        .commit()
        .await
        .expect("commit pending repair cursor seed");
    let seeded_page =
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&state.pool)
            .await
            .expect("seed a durable pending repair cursor page");
    assert_eq!(seeded_page, 0);
    let seeded_cursor: (i64, i64) = sqlx::query_as(
        "SELECT cursor_id, cursor_stale_rank \
         FROM hourly_rollup_archive_repair_progress \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load seeded pending repair cursor");
    assert_eq!(seeded_cursor, (64, 0));
    let seeded_candidate_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = 'completed' \
           AND historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count seeded pending repair candidates");
    assert_eq!(seeded_candidate_count, 513);

    crate::maintenance::repair_materialized_upstream_account_archive_markers(&state.pool)
        .await
        .expect("prepare legacy account archive replay markers");

    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(summary.as_deref().is_some_and(|summary| {
        summary.contains("deferred with pending archive usage breakdown repair")
    }));
    assert!(detail.is_none());
    let cursor: (i64, i64) = sqlx::query_as(
        "SELECT cursor_id, cursor_stale_rank \
         FROM hourly_rollup_archive_repair_progress \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load deferred pending repair cursor");
    assert_eq!(cursor, (320, 0));

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should stop after deferred repair cancellation")
        .expect("background bootstrap task should join after deferred repair cancellation");
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_records_failure_without_revoking_readiness() {
    let state = test_state_from_config(test_config(), false).await;
    state.startup_ready.store(true, Ordering::Release);
    sqlx::query("DROP TABLE upstream_account_stats_hourly")
        .execute(&state.pool)
        .await
        .expect("remove account rollup table to force bootstrap failure");

    spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone())
        .await
        .expect("background bootstrap task should join");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "failed").await;
    assert_eq!(status, "failed");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("existing rollups remain available"))
    );
    assert!(
        detail.is_some(),
        "failure should retain an observable error"
    );
    assert_eq!(
        health_check(State(state.clone()))
            .await
            .into_response()
            .status(),
        StatusCode::OK,
        "background bootstrap failure must not return health to starting"
    );

    state.shutdown.cancel();
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_cancels_while_waiting_for_lock() {
    let state = test_state_from_config(test_config(), false).await;
    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should stop after cancellation")
        .expect("background bootstrap task should join");
    drop(rollup_guard);
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred cancellation task-history finish");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before"))
    );
    assert!(detail.is_none());
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_cancels_while_task_history_start_is_locked() {
    let (state, temp_dir, db_url) = file_backed_test_state_from_current_schema(
        "startup-hourly-rollup-bootstrap-task-history-lock",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect task-history lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire task-history write lock");

    let mut bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    tokio::time::timeout(Duration::from_millis(100), &mut bootstrap_handle)
        .await
        .expect_err("bootstrap should wait for the task-history write lock");

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should stop without waiting for sqlite busy timeout")
        .expect("background bootstrap task should join");

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release task-history write lock");
    let orphaned_running_tasks: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM system_task_runs
        WHERE task_kind = 'hourly_rollup_bootstrap'
          AND trigger_kind = 'startup'
          AND status = 'running'
          AND summary = 'background hourly rollup bootstrap started'
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count cancelled bootstrap task-history rows");
    assert_eq!(orphaned_running_tasks, 0);
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}
#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_retries_coordinator_contention_promptly() {
    let state = test_state_from_config(test_config(), false).await;
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let write_permit = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());

    tokio::time::sleep(Duration::from_millis(100)).await;
    let task_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'hourly_rollup_bootstrap'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("read bootstrap task count while coordinator is occupied");
    assert_eq!(task_count, 0);

    drop(write_permit);
    wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;
    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("bootstrap should stop after cancellation")
        .expect("bootstrap task should join after cancellation");
    state.pool.close().await;
}

#[test]
fn background_startup_hourly_rollup_bootstrap_backs_off_after_p2_preemption() {
    let mut retry_after = Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS);
    let mut observed = Vec::new();
    for _ in 0..7 {
        observed.push(retry_after);
        retry_after = crate::runtime::next_startup_hourly_rollup_p2_preemption_retry(retry_after);
    }

    assert_eq!(
        observed,
        vec![
            Duration::from_secs(15),
            Duration::from_secs(30),
            Duration::from_secs(60),
            Duration::from_secs(120),
            Duration::from_secs(240),
            Duration::from_secs(300),
            Duration::from_secs(300),
        ]
    );
    assert_eq!(
        crate::runtime::next_startup_hourly_rollup_p2_preemption_retry(Duration::from_secs(300)),
        Duration::from_secs(300),
        "the capped delay must remain bounded"
    );
    assert_eq!(
        Duration::from_secs(BACKGROUND_DB_PRESSURE_RETRY_INTERVAL_SECS),
        Duration::from_secs(15),
        "a fresh successful bootstrap starts the next lifecycle at the initial delay"
    );
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_defers_task_history_finish_during_shutdown_lock()
 {
    let (state, temp_dir, db_url) = file_backed_test_state_from_current_schema(
        "startup-hourly-rollup-bootstrap-task-history-finish-lock",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect task-history lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire task-history write lock");

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should defer its task-history finish during shutdown")
        .expect("background bootstrap task should join");
    drop(rollup_guard);

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release task-history write lock");
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred task-history finish");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before"))
    );
    assert!(detail.is_none());

    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_hourly_rollup_task_history_finish_does_not_wait_for_write_lock() {
    let (state, temp_dir, db_url) = file_backed_test_state_from_current_schema(
        "startup-hourly-rollup-bootstrap-task-history-finish-cancel-race",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let task_run = begin_system_task_run(
        &state.pool,
        SystemTaskKind::HourlyRollupBootstrap,
        "startup",
        Some("background hourly rollup bootstrap started".to_string()),
    )
    .await
    .expect("record task-history start");
    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect task-history lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire task-history write lock");

    let cancel = CancellationToken::new();
    let state_for_finish = state.clone();
    let cancel_for_finish = cancel.clone();
    let finish_handle = tokio::spawn(async move {
        finish_runtime_startup_hourly_rollup_bootstrap_task(
            state_for_finish.as_ref(),
            &cancel_for_finish,
            Some(&task_run),
            SystemTaskStatus::Skipped,
            "background hourly rollup bootstrap cancelled before acquiring its synchronization lock",
            None,
        )
        .await;
    });
    tokio::time::timeout(Duration::from_millis(100), finish_handle)
        .await
        .expect("task-history finish should enqueue without waiting for the sqlite write lock")
        .expect("task-history finish task should join");

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release task-history write lock");
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred task-history finish");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before"))
    );
    assert!(detail.is_none());

    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn retention_run_once_keeps_blocking_hourly_rollup_bootstrap() {
    let state = test_state_from_config(test_config(), false).await;
    let cli = CliArgs {
        retention_run_once: true,
        ..CliArgs::default()
    };

    let summary = run_startup_persistent_prep(&state.pool, &state.config, &cli)
        .await
        .expect("retention run-once startup prep should finish");
    assert!(summary.bootstrapped_hourly_rollups);

    state.shutdown.cancel();
}

#[tokio::test]
async fn startup_backfill_progress_persists_terminal_missing_raw_cursor() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("reasoning-missing-raw")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .bind("missing-reasoning-request.json")
    .execute(&state.pool)
    .await
    .expect("insert reasoning backfill row");

    let row_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
            .bind("reasoning-missing-raw")
            .fetch_one(&state.pool)
            .await
            .expect("fetch inserted row id");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
        .await
        .expect("first startup backfill pass should succeed");

    let task_name =
        startup_backfill_task_progress_key(state.as_ref(), StartupBackfillTask::ReasoningEffort)
            .await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load backfill progress after first pass");
    assert_eq!(progress.cursor_id, row_id);
    assert_eq!(progress.last_scanned, 1);
    assert_eq!(progress.last_updated, 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("force startup backfill task due again");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
        .await
        .expect("second startup backfill pass should skip previously scanned row");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load backfill progress after second pass");
    assert_eq!(progress.cursor_id, row_id);
    assert_eq!(progress.last_scanned, 0);
    assert_eq!(progress.last_updated, 0);
}

#[tokio::test]
async fn startup_historical_rollup_backfill_prioritizes_usage_breakdown_repair() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18082").expect("valid upstream url"),
    )
    .await;
    let archived_occurred_at = shanghai_local_days_ago(120, 9, 0, 0);
    let payload = json!({
        "upstreamAccountId": 17,
        "upstreamAccountName": "Replay",
        "promptCacheKey": "pck-startup-priority",
        "stickyKey": "sticky-startup-priority",
        "responseModel": "gpt-5.4",
        "reasoningEffort": "high",
    })
    .to_string();
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "startup-priority-usage-breakdown",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "startup-priority-usage-breakdown",
            occurred_at: &archived_occurred_at,
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 42_i64,
            cost: 0.42_f64,
            ttfb_ms: Some(120.0),
            payload: Some(payload.as_str()),
            detail_level: "summary",
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let snapshot_before = load_historical_rollup_backfill_snapshot(&state.pool, &state.config)
        .await
        .expect("load startup historical rollup snapshot before priority pass");
    assert_eq!(snapshot_before.legacy_archive_pending, 1);
    assert_eq!(snapshot_before.pending_usage_breakdown_batches, 1);

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::HistoricalRollups)
        .await
        .expect("startup historical rollup backfill should run priority breakdown repair");

    let snapshot_after = load_historical_rollup_backfill_snapshot(&state.pool, &state.config)
        .await
        .expect("load startup historical rollup snapshot after priority pass");
    assert_eq!(snapshot_after.pending_usage_breakdown_batches, 0);
    assert_eq!(snapshot_after.legacy_archive_pending, 1);

    let task_name =
        startup_backfill_task_progress_key(state.as_ref(), StartupBackfillTask::HistoricalRollups)
            .await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load startup historical rollup progress after priority pass");
    assert_eq!(progress.last_scanned, 1);
    assert!(progress.last_updated > 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
}

async fn seed_missing_historical_rollup_startup_candidates(pool: &SqlitePool, count: usize) -> i64 {
    let mut tx = pool
        .begin()
        .await
        .expect("begin startup candidate seed transaction");
    for index in 0..count {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status, created_at
            )
            VALUES ('forward_proxy_attempts', '2025-01', ?1, ?2, 1, 'completed', datetime('now'))
            "#,
        )
        .bind(format!(
            "/missing/historical-rollup-startup-{index}.sqlite.gz"
        ))
        .bind(format!("startup-candidate-{index}"))
        .execute(tx.as_mut())
        .await
        .expect("insert missing historical rollup startup candidate");
    }
    let last_id = sqlx::query_scalar::<_, i64>("SELECT MAX(id) FROM archive_batches")
        .fetch_one(tx.as_mut())
        .await
        .expect("load last startup candidate id");
    tx.commit()
        .await
        .expect("commit startup candidate seed transaction");
    last_id
}

#[tokio::test]
async fn historical_rollup_startup_window_bounds_a_twenty_thousand_batch_fixture_and_wraps() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let last_id = seed_missing_historical_rollup_startup_candidates(&state.pool, 20_000).await;

    let pending_hint = count_historical_rollup_startup_pending_hint(&state.pool)
        .await
        .expect("load bounded historical rollup startup pending hint");
    assert_eq!(pending_hint.pending_archive_batches, 0);
    assert_eq!(pending_hint.candidate_count, 32);
    assert_eq!(pending_hint.inspected_path_count, 32);
    assert!(pending_hint.candidate_count <= 32);
    assert!(pending_hint.inspected_path_count <= 32);

    let first =
        materialize_historical_rollups_startup_window(&state.pool, 0, Duration::from_secs(6))
            .await
            .expect("run bounded historical rollup startup window");
    assert_eq!(first.candidate_count, 32);
    assert_eq!(first.inspected_path_count, 16);
    assert_eq!(first.summary.scanned_archive_batches, 16);
    assert!(first.candidate_count <= 32);
    assert!(first.inspected_path_count <= 32);
    assert!(first.summary.scanned_archive_batches > 2);
    assert!(first.next_cursor_id > 0);

    let resumed = materialize_historical_rollups_startup_window(
        &state.pool,
        first.next_cursor_id,
        Duration::from_secs(6),
    )
    .await
    .expect("resume bounded historical rollup startup window");
    assert_eq!(resumed.candidate_count, 32);
    assert_eq!(resumed.inspected_path_count, 16);
    assert!(resumed.next_cursor_id > first.next_cursor_id);

    let wrapped =
        materialize_historical_rollups_startup_window(&state.pool, last_id, Duration::from_secs(6))
            .await
            .expect("wrap bounded historical rollup startup window");
    assert!(wrapped.wrapped);
    assert_eq!(wrapped.candidate_count, 32);
    assert_eq!(wrapped.inspected_path_count, 16);
    assert!(wrapped.next_cursor_id < last_id);
}

#[tokio::test]
async fn historical_rollup_startup_window_advances_in_keyset_order_despite_usage_breakdown_candidates()
 {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_missing_historical_rollup_startup_candidates(&state.pool, 20).await;
    let sixteenth_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'forward_proxy_attempts' ORDER BY id LIMIT 1 OFFSET 15",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load sixteenth forward proxy candidate id");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status, created_at
        )
        VALUES ('codex_invocations', '2025-01', '/missing/usage-breakdown-priority.sqlite.gz', 'usage-breakdown-priority', 1, 'completed', datetime('now'))
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("insert usage breakdown candidate after the first keyset batch");

    let first =
        materialize_historical_rollups_startup_window(&state.pool, 0, Duration::from_secs(6))
            .await
            .expect("run first ordered historical rollup startup window");
    assert_eq!(first.inspected_path_count, 16);
    assert_eq!(first.next_cursor_id, sixteenth_id);

    let second = materialize_historical_rollups_startup_window(
        &state.pool,
        first.next_cursor_id,
        Duration::from_secs(6),
    )
    .await
    .expect("resume ordered historical rollup startup window");
    assert_eq!(second.inspected_path_count, 5);
    assert!(second.next_cursor_id > first.next_cursor_id);
}

#[tokio::test]
async fn historical_rollup_startup_window_preserves_cursor_when_budget_is_exhausted() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_missing_historical_rollup_startup_candidates(&state.pool, 32).await;

    let blocked = materialize_historical_rollups_startup_window(&state.pool, 0, Duration::ZERO)
        .await
        .expect("run budget-exhausted historical rollup startup window");
    assert_eq!(blocked.candidate_count, 32);
    assert_eq!(blocked.inspected_path_count, 0);
    assert_eq!(blocked.next_cursor_id, 0);
    assert!(blocked.hit_budget);
}

#[tokio::test]
async fn startup_historical_rollup_backfill_persists_cursor_and_defers_under_pressure() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_missing_historical_rollup_startup_candidates(&state.pool, 32).await;
    let task = StartupBackfillTask::HistoricalRollups;
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(1));
    let held = gate
        .try_begin_background("test_historical_rollup_pressure")
        .expect("hold startup backfill gate");

    assert!(
        !run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
            .await
            .expect("defer historical startup backfill under pressure")
    );
    let deferred = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load pressure-deferred historical progress");
    assert_eq!(deferred.cursor_id, 0);
    assert!(
        deferred.is_due(Utc::now()),
        "a pre-SQL pressure defer must leave durable progress untouched; the scheduler owns its deadline"
    );
    drop(held);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("make historical startup backfill due");
    assert!(
        !run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
            .await
            .expect("run bounded historical startup backfill")
    );
    let resumed = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load resumed historical progress");
    assert!(resumed.cursor_id > 0);
    assert_eq!(resumed.last_scanned, 16);
}

#[tokio::test]
async fn historical_rollup_noop_pass_records_proxy_cost_audit_detail() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_missing_historical_rollup_startup_candidates(&state.pool, 16).await;
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count startup task runs before historical noop pass");

    let cancel = CancellationToken::new();
    let outcome = run_startup_backfill_maintenance_pass(state.clone(), &cancel, None).await;
    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count startup task runs after historical noop pass");
    assert!(!outcome.ran_actionable_task);
    assert!(!outcome.had_failure);
    assert!(outcome.detail.is_some());
    assert_eq!(after, before + 1);
}

#[tokio::test]
async fn startup_backfill_not_due_check_does_not_claim_background_gate() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::ReasoningEffort;
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let next_run_after = format_utc_iso(Utc::now() + ChronoDuration::hours(6));
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 0,
            scanned: 0,
            updated: 0,
            zero_update_streak: 0,
            next_run_after: &next_run_after,
            status: STARTUP_BACKFILL_STATUS_OK,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed not-due startup progress");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(1));
    let held_permit = gate
        .try_begin_background("upstream_account_maintenance")
        .expect("hold local gate slot");

    run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
        .await
        .expect("not-due startup backfill should not require a background slot");

    assert_eq!(
        gate.try_begin_background("upstream_account_maintenance")
            .unwrap_err(),
        crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
        "not-due backfill should leave the already held slot untouched"
    );
    drop(held_permit);
}

#[tokio::test]
async fn startup_backfill_idle_pass_records_proxy_cost_audit_detail() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count startup backfill task runs before idle pass");

    let cancel = CancellationToken::new();
    let outcome = run_startup_backfill_maintenance_pass(state.clone(), &cancel, None).await;

    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count startup backfill task runs after idle pass");
    assert!(!outcome.ran_actionable_task);
    assert!(!outcome.had_failure);
    assert!(outcome.detail.is_some());
    assert_eq!(
        after,
        before + 1,
        "idle maintenance should record the proxy cost audit detail"
    );
}

#[tokio::test]
async fn startup_backfill_proxy_cost_audit_includes_catalog_and_cursor_detail() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    insert_proxy_cost_backfill_row(
        &state.pool,
        "proxy-cost-backfill-system-task-detail",
        Some("gpt-6-terra-2026-09-20"),
        Some(1_000),
        Some(500),
    )
    .await;

    let cancel = CancellationToken::new();
    let selected_tasks = [StartupBackfillTask::ProxyCost];
    let outcome =
        run_startup_backfill_maintenance_pass(state.clone(), &cancel, Some(&selected_tasks)).await;
    let detail = outcome
        .detail
        .as_deref()
        .expect("proxy cost maintenance should expose audit detail");
    assert!(detail.contains("catalog_version="));
    assert!(detail.contains("attempt_version="));
    assert!(detail.contains("scanned=1"));
    assert!(detail.contains("updated=1"));
    assert!(detail.contains("cursor_id="));
    assert!(detail.contains("state=drained"));

    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush startup backfill task audit detail");
    let persisted_detail: String = sqlx::query_scalar(
        "SELECT detail FROM system_task_runs WHERE task_kind = 'startup_backfill' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("query startup backfill audit detail");
    assert_eq!(persisted_detail, detail);
}

#[tokio::test]
async fn startup_backfill_event_wakes_only_the_matching_task() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let archive_task = StartupBackfillTask::UpstreamActivityArchives;
    let historical_task = StartupBackfillTask::HistoricalRollups;
    let suspended_until = format_utc_iso(Utc::now() + ChronoDuration::days(1));

    for task in [archive_task, historical_task] {
        save_startup_backfill_progress(
            &state.pool,
            task.name(),
            StartupBackfillProgressUpdate {
                cursor_id: 7,
                scanned: 100,
                updated: 0,
                zero_update_streak: 4,
                next_run_after: &suspended_until,
                status: STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE,
                suspension_reason: Some("source_unavailable"),
            },
        )
        .await
        .expect("seed source-unavailable startup backfill progress");
    }

    wake_startup_backfill_tasks(&state.pool, &[archive_task], "test_archive_available")
        .await
        .expect("wake affected archive task");

    let archive_progress = load_startup_backfill_progress(&state.pool, archive_task.name())
        .await
        .expect("load woken archive progress");
    assert!(archive_progress.is_due(Utc::now()));
    assert_eq!(archive_progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
    assert_eq!(archive_progress.suspension_reason, None);
    assert_eq!(archive_progress.next_probe_at, None);
    assert_eq!(archive_progress.wake_generation, 1);

    let historical_progress = load_startup_backfill_progress(&state.pool, historical_task.name())
        .await
        .expect("load unaffected historical progress");
    assert!(!historical_progress.is_due(Utc::now()));
    assert_eq!(
        historical_progress.last_status,
        STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE
    );
}

#[tokio::test]
async fn startup_backfill_pressure_defer_never_accesses_sqlite() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::ReasoningEffort;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(1));
    let _held = gate
        .try_begin_background("test_pressure")
        .expect("hold background slot");
    state.pool.close().await;

    let ran = run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
        .await
        .expect("closed pool proves the pressure defer does not access SQLite");
    assert!(!ran);
}

#[tokio::test]
async fn startup_backfill_pressure_defer_has_one_deadline_and_no_task_run_audit() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::ReasoningEffort;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    gate.record_pressure("test_pressure", "forced_cooldown");
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before pressure defer");

    let cancel = CancellationToken::new();
    let selected_tasks = [task];
    let first = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        &cancel,
        Some(&selected_tasks),
        &gate,
    )
    .await;
    let second = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        &cancel,
        Some(&selected_tasks),
        &gate,
    )
    .await;
    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after pressure defer");
    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load untouched progress after scheduler-only pressure defer");

    assert!(!first.ran_actionable_task);
    assert!(!first.had_failure);
    assert!(!second.ran_actionable_task);
    assert!(!second.had_failure);
    assert_eq!(after, before, "deferred passes must not create audit rows");
    assert!(progress.is_due(Utc::now()));
    assert_eq!(progress.suspension_reason, None);
}

#[tokio::test]
async fn pressure_eligibility_wake_rechecks_durable_backfill_deadline() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::PoolUpstreamNodeHealthArchives;
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let future_due = format_utc_iso(Utc::now() + ChronoDuration::hours(1));
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 17,
            scanned: 8,
            updated: 3,
            zero_update_streak: 0,
            next_run_after: &future_due,
            status: STARTUP_BACKFILL_STATUS_FAILED,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed a durable future failure backoff");
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before pressure eligibility wake");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let held = gate
        .try_begin_background("test_pressure_eligibility_busy")
        .expect("occupy the background slot");
    let first = run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
        .await
        .expect("background-busy admission should be a scheduler-only defer");
    assert!(!first);
    assert_eq!(gate.snapshot().background_skips, 1);

    let observed_eligibility = gate.eligibility_generation();
    drop(held);
    tokio::time::timeout(
        Duration::from_secs(1),
        gate.wait_for_eligibility_change(observed_eligibility),
    )
    .await
    .expect("background-slot release should emit an eligibility event");

    let cancel = CancellationToken::new();
    run_pressure_eligible_startup_backfill_tasks(state.clone(), &cancel, &gate).await;

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load progress after pressure eligibility wake");
    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after pressure eligibility wake");
    assert_eq!(progress.cursor_id, 17);
    assert_eq!(progress.last_scanned, 8);
    assert_eq!(progress.last_updated, 3);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_FAILED);
    assert_eq!(
        progress.next_run_after.as_deref(),
        Some(future_due.as_str())
    );
    assert!(!progress.is_due(Utc::now()));
    assert_eq!(
        task_runs_after, task_runs_before,
        "an eligibility event must not bypass a real durable failure backoff"
    );
}

#[tokio::test]
async fn startup_backfill_progress_lookup_busy_closes_gate_before_permit_release() {
    let (state, temp_dir, db_url) = file_backed_test_state_from_current_schema(
        "startup-backfill-progress-lookup-busy",
        Duration::from_millis(50),
    )
    .await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect progress lookup lock holder");
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut lock_conn)
        .await
        .expect("begin exclusive schema lock transaction");
    sqlx::query("CREATE TABLE startup_backfill_progress_lookup_lock_guard (id INTEGER)")
        .execute(&mut lock_conn)
        .await
        .expect("hold the schema lock across the progress lookup");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_backfill_busy_error_closes_gate_before_next_task(
        &state,
        StartupBackfillTask::ReasoningEffort,
        &gate,
    )
    .await;

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release progress lookup lock");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_backfill_running_state_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::ReasoningEffort).await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_running_state_busy
        BEFORE UPDATE OF last_status ON startup_backfill_progress
        WHEN NEW.task_name = 'proxy_reasoning_effort_v1' AND NEW.last_status = 'running'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into running-state persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_backfill_busy_error_closes_gate_before_next_task(
        &state,
        StartupBackfillTask::ReasoningEffort,
        &gate,
    )
    .await;
}

#[tokio::test]
async fn startup_backfill_progress_persist_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::ReasoningEffort).await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_progress_persist_busy
        BEFORE UPDATE OF last_status ON startup_backfill_progress
        WHEN NEW.task_name = 'proxy_reasoning_effort_v1' AND NEW.last_status = 'ok'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into backfill progress persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_backfill_busy_error_closes_gate_before_next_task(
        &state,
        StartupBackfillTask::ReasoningEffort,
        &gate,
    )
    .await;
}

#[tokio::test]
async fn startup_backfill_failure_persist_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::ReasoningEffort).await;
    let temp_dir = make_temp_test_dir("startup-backfill-failure-persist-busy");
    let request_path = temp_dir.join("request.json");
    fs::write(
        &request_path,
        r#"{"model":"gpt-5.3-codex","reasoning":{"effort":"low"}}"#,
    )
    .expect("write reasoning backfill request");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("startup-backfill-failure-persist-busy")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .bind(request_path.to_string_lossy().as_ref())
    .execute(&state.pool)
    .await
    .expect("insert reasoning backfill candidate");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_inner_busy
        BEFORE UPDATE OF payload ON codex_invocations
        WHEN NEW.invoke_id = 'startup-backfill-failure-persist-busy'
        BEGIN
            SELECT RAISE(ABORT, 'database is busy');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite busy failure into backfill work");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_failure_persist_busy
        BEFORE UPDATE OF last_status ON startup_backfill_progress
        WHEN NEW.task_name = 'proxy_reasoning_effort_v1' AND NEW.last_status = 'failed'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into failure persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_backfill_busy_error_closes_gate_before_next_task(
        &state,
        StartupBackfillTask::ReasoningEffort,
        &gate,
    )
    .await;

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_backfill_busy_failure_persists_failed_state_and_bounded_retry() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::ReasoningEffort;
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let suspended_until = format_utc_iso(Utc::now() + ChronoDuration::hours(1));
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 12,
            scanned: 4,
            updated: 0,
            zero_update_streak: 2,
            next_run_after: &suspended_until,
            status: STARTUP_BACKFILL_STATUS_IDLE,
            suspension_reason: Some("sqlite_pressure_cooldown"),
        },
    )
    .await
    .expect("seed pressure-deferred progress");
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load seeded startup backfill progress");
    let before_retry = Utc::now();
    persist_startup_backfill_task_failure(
        &state,
        task,
        &task_name,
        &progress,
        std::time::Instant::now(),
        &anyhow::anyhow!("database table is locked"),
    )
    .await
    .expect("record SQLite locked backfill failure");

    let failed = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load failed startup backfill progress");
    let retry_at = failed
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .expect("parse bounded failure retry");
    assert_eq!(failed.last_status, STARTUP_BACKFILL_STATUS_FAILED);
    assert_eq!(failed.suspension_reason, None);
    assert!(
        retry_at >= before_retry + ChronoDuration::seconds(10),
        "SQLite BUSY/LOCKED must use the bounded failure retry rather than a pressure defer"
    );
}

#[tokio::test]
async fn startup_backfill_busy_failure_closes_pressure_gate_before_the_next_task_reads_sqlite() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let temp_dir = make_temp_test_dir("startup-backfill-busy-pressure-gate");
    let request_path = temp_dir.join("request.json");
    fs::write(
        &request_path,
        r#"{"model":"gpt-5.3-codex","reasoning":{"effort":"low"}}"#,
    )
    .expect("write reasoning backfill request");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("startup-backfill-injected-busy")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .bind(request_path.to_string_lossy().as_ref())
    .execute(&state.pool)
    .await
    .expect("insert reasoning backfill candidate");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_injected_busy
        BEFORE UPDATE OF payload ON codex_invocations
        WHEN NEW.invoke_id = 'startup-backfill-injected-busy'
        BEGIN
            SELECT RAISE(ABORT, 'database is busy');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject an actual SQLite busy failure into backfill work");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let cancel = CancellationToken::new();
    let first_tasks = [StartupBackfillTask::ReasoningEffort];
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before injected busy failure");

    let first = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        &cancel,
        Some(&first_tasks),
        &gate,
    )
    .await;
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush injected busy failure task audit");

    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after injected busy failure");
    let task_run_status: String = sqlx::query_scalar(
        "SELECT status FROM system_task_runs WHERE task_kind = 'startup_backfill' ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load injected busy failure task audit");
    let failed_progress = load_startup_backfill_progress(&state.pool, first_tasks[0].name())
        .await
        .expect("load persisted busy failure progress");
    assert!(!first.ran_actionable_task);
    assert!(first.had_failure);
    assert_eq!(task_runs_after, task_runs_before + 1);
    assert_eq!(task_run_status, "failed");
    assert_eq!(failed_progress.last_status, STARTUP_BACKFILL_STATUS_FAILED);
    assert!(
        gate.pressure_cooldown_deadline_epoch_ms().is_some(),
        "the durable busy failure must close the gate before its permit is released"
    );

    // A closed pool turns this into a zero-SQL admission assertion. The only valid result is a
    // scheduler-only pressure defer that waits for the gate deadline or eligibility event.
    state.pool.close().await;
    let second_tasks = [StartupBackfillTask::PromptCacheKey];
    let second = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        &cancel,
        Some(&second_tasks),
        &gate,
    )
    .await;
    assert!(!second.ran_actionable_task);
    assert!(!second.had_failure);
    assert!(
        gate.snapshot().background_skips >= 1,
        "the next task must be refused by the closed gate without touching SQLite"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_coverage_lock_maintenance_pass_defers_without_audit_but_non_lock_failure_audits() {
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let cancel = CancellationToken::new();
    let selected_tasks = [task];

    let pressure_state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&pressure_state, task).await;
    sqlx::query(
        "DELETE FROM hourly_rollup_live_progress WHERE dataset = 'invocation_account_activity_v2_repair_generation'",
    )
    .execute(&pressure_state.pool)
    .await
    .expect("make the coverage repair generation initialization due");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_maintenance_busy
        BEFORE INSERT ON hourly_rollup_live_progress
        WHEN NEW.dataset = 'invocation_account_activity_v2_repair_generation'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&pressure_state.pool)
    .await
    .expect("inject a SQLite lock into the coverage repair");

    let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let pressure_audits_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&pressure_state.pool)
    .await
    .expect("count task runs before the locked coverage maintenance pass");

    let pressure_outcome = run_startup_backfill_maintenance_pass_with_gate(
        pressure_state.clone(),
        &cancel,
        Some(&selected_tasks),
        &pressure_gate,
    )
    .await;
    let pressure_audits_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&pressure_state.pool)
    .await
    .expect("count task runs after the locked coverage maintenance pass");
    let retry_progress = load_startup_backfill_progress(&pressure_state.pool, task.name())
        .await
        .expect("load the coverage retry deadline after the lock");

    assert!(!pressure_outcome.ran_actionable_task);
    assert!(!pressure_outcome.had_failure);
    assert_eq!(pressure_audits_after, pressure_audits_before);
    assert_eq!(pressure_gate.snapshot().pressure_events, 1);
    assert!(!retry_progress.is_due(Utc::now()));

    // Closing the pool proves the next task's pressure-cooldown path has zero SQLite I/O.
    pressure_state.pool.close().await;
    let next_tasks = [StartupBackfillTask::PromptCacheKey];
    let next_outcome = run_startup_backfill_maintenance_pass_with_gate(
        pressure_state.clone(),
        &cancel,
        Some(&next_tasks),
        &pressure_gate,
    )
    .await;
    assert!(!next_outcome.ran_actionable_task);
    assert!(!next_outcome.had_failure);
    assert!(pressure_gate.snapshot().background_skips >= 1);

    let non_pressure_state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&non_pressure_state, task).await;
    sqlx::query(
        "DELETE FROM hourly_rollup_live_progress WHERE dataset = 'invocation_account_activity_v2_repair_generation'",
    )
    .execute(&non_pressure_state.pool)
    .await
    .expect("make the non-lock coverage repair generation initialization due");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_maintenance_error
        BEFORE INSERT ON hourly_rollup_live_progress
        WHEN NEW.dataset = 'invocation_account_activity_v2_repair_generation'
        BEGIN
            SELECT RAISE(ABORT, 'coverage repair operation failed');
        END
        "#,
    )
    .execute(&non_pressure_state.pool)
    .await
    .expect("inject a non-lock coverage repair failure");

    let non_pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let non_pressure_audits_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&non_pressure_state.pool)
    .await
    .expect("count task runs before the non-lock coverage maintenance pass");

    let non_pressure_outcome = run_startup_backfill_maintenance_pass_with_gate(
        non_pressure_state.clone(),
        &cancel,
        Some(&selected_tasks),
        &non_pressure_gate,
    )
    .await;
    non_pressure_state
        .sqlite_batch_writer
        .flush_now(&non_pressure_state.pool)
        .await
        .expect("flush the non-lock coverage failure audit");
    let non_pressure_audits_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&non_pressure_state.pool)
    .await
    .expect("count task runs after the non-lock coverage maintenance pass");
    let non_pressure_audit_status: String = sqlx::query_scalar(
        "SELECT status FROM system_task_runs WHERE task_kind = 'startup_backfill' ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_one(&non_pressure_state.pool)
    .await
    .expect("load the non-lock coverage failure audit");

    assert!(!non_pressure_outcome.ran_actionable_task);
    assert!(non_pressure_outcome.had_failure);
    assert_eq!(non_pressure_audits_after, non_pressure_audits_before + 1);
    assert_eq!(non_pressure_audit_status, "failed");
    assert_eq!(non_pressure_gate.snapshot().pressure_events, 0);
}

#[tokio::test]
async fn coverage_repair_defer_persists_its_own_retry_deadline() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    defer_startup_backfill_coverage_repair(state.as_ref())
        .await
        .expect("schedule the coverage repair retry");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load coverage retry progress");
    let retry_at = progress
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .expect("coverage retry deadline");
    assert!(retry_at > Utc::now());
    assert!(retry_at <= Utc::now() + ChronoDuration::seconds(30));
    assert!(!progress.is_due(Utc::now()));
    assert_eq!(progress.zero_update_streak, 1);
}

#[tokio::test]
async fn coverage_repair_progress_resets_its_own_retry_backoff() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let deferred_until = format_utc_iso(Utc::now() + ChronoDuration::minutes(15));
    save_startup_backfill_progress(
        &state.pool,
        task.name(),
        StartupBackfillProgressUpdate {
            cursor_id: 7,
            scanned: 11,
            updated: 0,
            zero_update_streak: 4,
            next_run_after: &deferred_until,
            status: STARTUP_BACKFILL_STATUS_OK,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed a deferred coverage repair");

    record_startup_backfill_coverage_repair_progress(
        state.as_ref(),
        ActiveAccountActivityV2RepairOutcome {
            priority_bucket_count: 2,
            repaired_bucket_count: 2,
            elapsed_ms: 1,
        },
    )
    .await
    .expect("record coverage repair progress");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load coverage repair progress");
    let retry_at = progress
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .expect("coverage follow-up deadline");
    assert_eq!(progress.zero_update_streak, 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert!(retry_at > Utc::now());
    assert!(retry_at <= Utc::now() + ChronoDuration::seconds(30));
}

#[tokio::test]
async fn coverage_repair_progress_does_not_wake_source_unavailable_historical_backfill() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let historical_task = StartupBackfillTask::HistoricalRollups;
    let coverage_task = StartupBackfillTask::AccountActivityV2Coverage;
    let suspended_until = format_utc_iso(Utc::now() + ChronoDuration::hours(24));
    save_startup_backfill_progress(
        &state.pool,
        historical_task.name(),
        StartupBackfillProgressUpdate {
            cursor_id: 7,
            scanned: 11,
            updated: 0,
            zero_update_streak: 4,
            next_run_after: &suspended_until,
            status: STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE,
            suspension_reason: Some("source_unavailable"),
        },
    )
    .await
    .expect("seed a source-unavailable historical backfill");

    record_startup_backfill_coverage_repair_progress(
        state.as_ref(),
        ActiveAccountActivityV2RepairOutcome {
            priority_bucket_count: 2,
            repaired_bucket_count: 2,
            elapsed_ms: 1,
        },
    )
    .await
    .expect("retain the source-unavailable backfill suspension");

    let historical_progress = load_startup_backfill_progress(&state.pool, historical_task.name())
        .await
        .expect("load source-unavailable historical backfill");
    assert_eq!(historical_progress.zero_update_streak, 4);
    assert_eq!(
        historical_progress.last_status,
        STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE
    );
    assert_eq!(
        historical_progress.next_run_after.as_deref(),
        Some(suspended_until.as_str())
    );

    let coverage_progress = load_startup_backfill_progress(&state.pool, coverage_task.name())
        .await
        .expect("load independent coverage repair progress");
    assert_eq!(coverage_progress.zero_update_streak, 0);
    assert_eq!(coverage_progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert!(!coverage_progress.is_due(Utc::now()));
}

#[test]
fn coverage_repair_retry_backoff_is_bounded_and_exponential() {
    assert_eq!(coverage_repair_retry_delay(1), Duration::from_secs(15));
    assert_eq!(coverage_repair_retry_delay(2), Duration::from_secs(60));
    assert_eq!(coverage_repair_retry_delay(3), Duration::from_secs(5 * 60));
    assert_eq!(coverage_repair_retry_delay(4), Duration::from_secs(15 * 60));
    assert_eq!(
        coverage_repair_retry_delay(20),
        Duration::from_secs(15 * 60)
    );
}

#[tokio::test]
async fn live_activity_v2_coverage_progress_wakes_its_dedicated_repair_task() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    wake_account_activity_v2_coverage_repair(&state.pool, 0)
        .await
        .expect("skip an unchanged live v2 coverage cursor");
    assert_eq!(
        load_startup_backfill_progress(&state.pool, task.name())
            .await
            .expect("load unchanged coverage progress")
            .wake_generation,
        0
    );

    wake_account_activity_v2_coverage_repair(&state.pool, 1)
        .await
        .expect("wake after live v2 coverage cursor progress");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load coverage-woken startup backfill progress");
    assert!(progress.wake_generation > 0);
    assert!(progress.is_due(Utc::now()));
}

#[tokio::test]
async fn live_activity_v2_coverage_wake_preserves_an_active_retry_backoff() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let deferred_until = format_utc_iso(Utc::now() + ChronoDuration::minutes(15));
    save_startup_backfill_progress(
        &state.pool,
        task.name(),
        StartupBackfillProgressUpdate {
            cursor_id: 9,
            scanned: 12,
            updated: 0,
            zero_update_streak: 4,
            next_run_after: &deferred_until,
            status: STARTUP_BACKFILL_STATUS_OK,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed active coverage retry backoff");

    let before = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load active coverage retry before the live update");

    wake_account_activity_v2_coverage_repair(&state.pool, 1)
        .await
        .expect("record live coverage progress without bypassing the retry deadline");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load preserved coverage retry progress");
    assert_eq!(progress.zero_update_streak, 4);
    assert_eq!(
        progress.next_run_after.as_deref(),
        Some(deferred_until.as_str())
    );
    assert!(!progress.is_due(Utc::now()));
    assert_eq!(progress.wake_generation, before.wake_generation);
}

#[tokio::test]
async fn live_activity_v2_coverage_wake_preserves_a_success_follow_up_deadline() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    record_startup_backfill_coverage_repair_progress(
        state.as_ref(),
        ActiveAccountActivityV2RepairOutcome {
            priority_bucket_count: 1,
            repaired_bucket_count: 1,
            elapsed_ms: 1,
        },
    )
    .await
    .expect("record successful coverage repair progress");
    let before = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load scheduled coverage follow-up");

    wake_account_activity_v2_coverage_repair(&state.pool, 1)
        .await
        .expect("keep the successful coverage follow-up deadline");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load preserved coverage follow-up");
    assert_eq!(progress.zero_update_streak, 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert_eq!(progress.next_run_after, before.next_run_after);
    assert_eq!(progress.wake_generation, before.wake_generation);
    assert!(!progress.is_due(Utc::now()));
}

#[tokio::test]
async fn idle_coverage_repair_persists_its_next_probe_deadline() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    let ran = run_startup_backfill_task_if_due(&state, task)
        .await
        .expect("run an idle coverage repair pass");
    assert!(!ran);

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load idle coverage repair progress");
    let next_due = progress
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .expect("persisted idle coverage deadline");
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
    assert_eq!(progress.zero_update_streak, 0);
    assert!(next_due > Utc::now() + ChronoDuration::hours(5));
    assert!(next_due <= Utc::now() + ChronoDuration::hours(7));
}

#[tokio::test]
async fn startup_coverage_repair_defers_when_hourly_rollup_lock_is_busy() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;

    let run = run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
        .await
        .expect("busy hourly rollup lock should defer without SQLite access");
    assert!(
        !run,
        "coverage repair should defer while its synchronization lock is busy"
    );
    {
        let _probe = gate
            .try_begin_background("coverage_admission_probe")
            .expect("a lock-only defer must not consume the pressure permit");
    }
    drop(rollup_guard);

    assert!(
        !run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
            .await
            .expect("coverage repair should finish after the rollup lock is released")
    );
    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("coverage repair must persist its idle follow-up after executing");
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
}

#[tokio::test]
async fn startup_coverage_gate_denial_skips_sqlite_progress_and_task_run_audit() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let _held = gate
        .try_begin_background("coverage_gate_holder")
        .expect("occupy the production-shaped gate");
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before the denied coverage repair");

    assert!(
        !run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
            .await
            .expect("closed coverage gate should defer without an error")
    );

    let progress_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM startup_backfill_progress WHERE task_name = ?1")
            .bind(task.name())
            .fetch_one(&state.pool)
            .await
            .expect("count coverage progress after the denied repair");
    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after the denied coverage repair");
    assert_eq!(
        progress_rows, 0,
        "gate denial must not write coverage progress"
    );
    assert_eq!(
        task_runs_after, task_runs_before,
        "gate denial must not write an audit"
    );

    state.pool.close().await;
    assert!(
        !run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
            .await
            .expect("a closed pool proves denied coverage never accesses SQLite")
    );
}

#[tokio::test]
async fn startup_coverage_repair_progress_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_progress_busy
        BEFORE INSERT ON startup_backfill_progress
        WHEN NEW.task_name = 'account_activity_v2_coverage_repair_v1'
            AND NEW.last_status = 'ok'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into coverage repair progress persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task(
        &state,
        &gate,
        || async {
            Ok(ActiveAccountActivityV2RepairOutcome {
                priority_bucket_count: 1,
                repaired_bucket_count: 1,
                elapsed_ms: 1,
            })
        },
    )
    .await;
}

#[tokio::test]
async fn startup_coverage_repair_defer_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::AccountActivityV2Coverage)
        .await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_defer_busy
        BEFORE UPDATE OF next_run_after ON startup_backfill_progress
        WHEN NEW.task_name = 'account_activity_v2_coverage_repair_v1'
            AND NEW.last_status = 'idle'
        BEGIN
            SELECT RAISE(ABORT, 'database is busy');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into coverage repair defer persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task(
        &state,
        &gate,
        || async {
            Ok(ActiveAccountActivityV2RepairOutcome {
                priority_bucket_count: 1,
                repaired_bucket_count: 0,
                elapsed_ms: 1,
            })
        },
    )
    .await;
}

#[tokio::test]
async fn startup_coverage_repair_and_retry_progress_persist_busy_record_one_pressure_event() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    seed_due_startup_backfill_progress(&state, task).await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_retry_progress_persist_busy
        BEFORE UPDATE OF next_run_after ON startup_backfill_progress
        WHEN NEW.task_name = 'account_activity_v2_coverage_repair_v1'
            AND NEW.last_status = 'idle'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into retry progress persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before combined repair and retry-progress failures");

    let outcome =
        run_startup_backfill_coverage_repair_if_due_with_repair(&state, &gate, || async {
            Err(anyhow::anyhow!("database is busy"))
        })
        .await
        .expect("repair and retry progress locks should defer coverage repair");
    assert!(outcome.is_pressure_deferred());
    assert_eq!(
        gate.snapshot().pressure_events,
        1,
        "one failed coverage attempt must record one pressure event even when retry progress also locks"
    );

    let next = run_startup_backfill_task_if_due_with_gate(
        &state,
        StartupBackfillTask::PromptCacheKey,
        &gate,
    )
    .await
    .expect("the next task should be deferred before SQLite access");
    assert!(!next);
    assert!(
        gate.snapshot().background_skips >= 1,
        "the closed gate must prevent an early next task"
    );
    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after combined repair and retry-progress failures");
    assert_eq!(
        task_runs_after, task_runs_before,
        "direct coverage repair failure must not create a task-run audit"
    );
}

#[tokio::test]
async fn startup_coverage_repair_and_retry_progress_lookup_busy_record_one_pressure_event() {
    let (state, temp_dir, db_url) = file_backed_test_state_from_current_schema(
        "startup-coverage-repair-retry-progress-lookup-busy",
        Duration::from_millis(50),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    seed_due_startup_backfill_progress(&state, task).await;
    let lock_holder = Arc::new(Mutex::new(None));
    let lock_holder_for_repair = lock_holder.clone();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before combined repair and retry-progress failures");

    let outcome = run_startup_backfill_coverage_repair_if_due_with_repair(&state, &gate, move || {
        let db_url = db_url.clone();
        let lock_holder = lock_holder_for_repair.clone();
        async move {
            let mut lock_conn = SqliteConnection::connect(&db_url).await?;
            sqlx::query("BEGIN EXCLUSIVE")
                .execute(&mut lock_conn)
                .await?;
            sqlx::query(
                "CREATE TABLE startup_coverage_repair_retry_progress_lookup_lock_guard (id INTEGER)",
            )
            .execute(&mut lock_conn)
            .await?;
            *lock_holder.lock().await = Some(lock_conn);
            Err(anyhow::anyhow!("database is busy"))
        }
    })
    .await
    .expect("repair and retry progress locks should defer coverage repair");
    assert!(outcome.is_pressure_deferred());
    assert_eq!(
        gate.snapshot().pressure_events,
        1,
        "one failed coverage attempt must record one pressure event even when retry progress also locks"
    );

    let next = run_startup_backfill_task_if_due_with_gate(
        &state,
        StartupBackfillTask::PromptCacheKey,
        &gate,
    )
    .await
    .expect("the next task should be deferred before SQLite access");
    assert!(!next);
    assert!(
        gate.snapshot().background_skips >= 1,
        "the closed gate must prevent an early next task"
    );

    let mut lock_conn = lock_holder
        .lock()
        .await
        .take()
        .expect("keep the exclusive lock through retry progress lookup");
    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release retry progress lookup lock");
    lock_conn
        .close()
        .await
        .expect("close retry progress lookup lock holder");

    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after combined repair and retry-progress failures");
    assert_eq!(
        task_runs_after, task_runs_before,
        "direct coverage repair failure must not create a task-run audit"
    );
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_coverage_repair_idle_progress_lookup_busy_closes_gate_before_permit_release() {
    let (state, temp_dir, db_url) = file_backed_test_state_from_current_schema(
        "startup-coverage-repair-idle-progress-lookup-busy",
        Duration::from_millis(50),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::AccountActivityV2Coverage)
        .await;
    let lock_holder = Arc::new(Mutex::new(None));
    let lock_holder_for_repair = lock_holder.clone();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));

    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task(
        &state,
        &gate,
        move || {
            let db_url = db_url.clone();
            let lock_holder = lock_holder_for_repair.clone();
            async move {
                let mut lock_conn = SqliteConnection::connect(&db_url).await?;
                sqlx::query("BEGIN EXCLUSIVE")
                    .execute(&mut lock_conn)
                    .await?;
                sqlx::query(
                    "CREATE TABLE startup_coverage_repair_idle_progress_lookup_lock_guard (id INTEGER)",
                )
                .execute(&mut lock_conn)
                .await?;
                *lock_holder.lock().await = Some(lock_conn);
                Ok(ActiveAccountActivityV2RepairOutcome::default())
            }
        },
    )
    .await;

    let mut lock_conn = lock_holder
        .lock()
        .await
        .take()
        .expect("keep the exclusive lock through idle progress lookup");
    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release idle progress lookup lock");
    lock_conn
        .close()
        .await
        .expect("close idle progress lookup lock holder");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_coverage_repair_idle_progress_persist_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::AccountActivityV2Coverage)
        .await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_idle_progress_persist_busy
        BEFORE UPDATE OF last_status ON startup_backfill_progress
        WHEN NEW.task_name = 'account_activity_v2_coverage_repair_v1'
            AND NEW.last_status = 'idle'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into idle coverage progress persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task(
        &state,
        &gate,
        || async { Ok(ActiveAccountActivityV2RepairOutcome::default()) },
    )
    .await;
}
