#[tokio::test]
pub(crate) async fn coverage_repair_defer_persists_its_own_retry_deadline() {
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
pub(crate) async fn coverage_repair_progress_resets_its_own_retry_backoff() {
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
pub(crate) async fn coverage_repair_progress_does_not_wake_source_unavailable_historical_backfill()
{
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
pub(crate) fn coverage_repair_retry_backoff_is_bounded_and_exponential() {
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
pub(crate) async fn live_activity_v2_coverage_progress_wakes_its_dedicated_repair_task() {
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
pub(crate) async fn live_activity_v2_coverage_wake_preserves_an_active_retry_backoff() {
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
pub(crate) async fn live_activity_v2_coverage_wake_preserves_a_success_follow_up_deadline() {
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
pub(crate) async fn idle_coverage_repair_persists_its_next_probe_deadline() {
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
pub(crate) async fn startup_coverage_repair_defers_when_hourly_rollup_lock_is_busy() {
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
pub(crate) async fn startup_coverage_gate_denial_skips_sqlite_progress_and_task_run_audit() {
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
pub(crate) async fn startup_coverage_repair_progress_busy_closes_gate_before_permit_release() {
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
pub(crate) async fn startup_coverage_repair_defer_busy_closes_gate_before_permit_release() {
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
pub(crate) async fn startup_coverage_repair_and_retry_progress_persist_busy_record_one_pressure_event()
 {
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
pub(crate) async fn startup_coverage_repair_and_retry_progress_lookup_busy_record_one_pressure_event()
 {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
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
pub(crate) async fn startup_coverage_repair_idle_progress_lookup_busy_closes_gate_before_permit_release()
 {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
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
pub(crate) async fn startup_coverage_repair_idle_progress_persist_busy_closes_gate_before_permit_release()
 {
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

#[tokio::test]
pub(crate) async fn failure_classification_backfill_skips_success_rows_with_complete_defaults() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            failure_class,
            is_actionable,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("success-no-kind")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(FAILURE_CLASS_NONE)
    .bind(0_i64)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert success row");

    let outcome = backfill_failure_classification_from_cursor(&pool, 0, None, Some(10), None)
        .await
        .expect("run failure classification backfill");
    assert_eq!(outcome.summary.scanned, 0);
    assert_eq!(outcome.summary.updated, 0);
    assert_eq!(outcome.next_cursor_id, 0);
    assert!(!outcome.hit_budget);
}

#[tokio::test]
pub(crate) async fn failure_classification_backfill_recovers_response_failed_records() {
    #[derive(sqlx::FromRow)]
    struct BackfilledRow {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
        payload: Option<String>,
    }

    let pool = test_current_schema_pool().await;

    let raw_response = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "event: response.failed",
        r#"data: {"type":"response.failed","response":{"id":"resp_test","model":"gpt-5.4","status":"failed","error":{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}}"#,
    ]
    .join("\n");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            failure_class,
            is_actionable,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("response-failed-success")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(FAILURE_CLASS_NONE)
    .bind(0_i64)
    .bind(r#"{"endpoint":"/v1/responses","isStream":true,"usageMissingReason":"usage_missing_in_stream"}"#)
    .bind(&raw_response)
    .execute(&pool)
    .await
    .expect("insert misrecorded success row");

    let outcome = backfill_failure_classification_from_cursor(&pool, 0, None, Some(10), None)
        .await
        .expect("run failure classification backfill");
    assert_eq!(outcome.summary.scanned, 1);
    assert_eq!(outcome.summary.updated, 1);

    let row = sqlx::query_as::<_, BackfilledRow>(
        r#"
        SELECT status, error_message, failure_kind, failure_class, is_actionable, payload
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("response-failed-success")
    .fetch_one(&pool)
    .await
    .expect("load backfilled row");

    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_response_failed] server_error"))
    );
    assert_eq!(
        row.failure_kind.as_deref(),
        Some("upstream_response_failed")
    );
    assert_eq!(row.failure_class.as_deref(), Some("service_failure"));
    assert_eq!(row.is_actionable, Some(1));

    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("payload should still be present"),
    )
    .expect("decode payload json");
    assert_eq!(
        payload_json["streamTerminalEvent"].as_str(),
        Some("response.failed")
    );
    assert_eq!(
        payload_json["upstreamErrorCode"].as_str(),
        Some("server_error")
    );
    assert_eq!(
        payload_json["upstreamRequestId"].as_str(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );
}

#[derive(sqlx::FromRow)]
struct LongStreamBackfilledRow {
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    payload: Option<String>,
}

struct LongStreamFailureFixture {
    response_path: PathBuf,
    raw_file: String,
    preview: String,
}

fn write_long_stream_failure_fixture(temp_dir: &Path) -> LongStreamFailureFixture {
    let response_path = temp_dir.join("response.bin");
    let long_prefix = format!(
        r#"event: response.created
data: {{"type":"response.output_text.delta","delta":"{}"}}

"#,
        "x".repeat(16_400)
    );
    let raw_file = format!(
        r#"{}event: response.failed
data: {{"type":"response.failed","response":{{"id":"resp_test","model":"gpt-5.4","status":"failed","error":{{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}}}}}
"#,
        long_prefix,
    );
    fs::write(&response_path, raw_file.as_bytes()).expect("write response raw file");
    let preview = build_raw_response_preview(raw_file.as_bytes());
    assert!(!preview.contains("response.failed"));
    LongStreamFailureFixture {
        response_path,
        raw_file,
        preview,
    }
}

async fn insert_long_stream_failure_fixture(pool: &SqlitePool, fixture: &LongStreamFailureFixture) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, failure_class, is_actionable,
            payload, raw_response, response_raw_path, response_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("response-failed-from-file")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(FAILURE_CLASS_NONE)
    .bind(0_i64)
    .bind(r#"{"endpoint":"/v1/responses","isStream":true}"#)
    .bind(&fixture.preview)
    .bind(fixture.response_path.to_string_lossy().to_string())
    .bind(fixture.raw_file.len() as i64)
    .execute(pool)
    .await
    .expect("insert long success row");
}

async fn load_long_stream_backfilled_row(pool: &SqlitePool) -> LongStreamBackfilledRow {
    sqlx::query_as::<_, LongStreamBackfilledRow>(
        r#"
        SELECT status, error_message, failure_kind, payload
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("response-failed-from-file")
    .fetch_one(pool)
    .await
    .expect("load backfilled row")
}

fn assert_long_stream_backfill_row(row: &LongStreamBackfilledRow) {
    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_response_failed] server_error"))
    );
    assert_eq!(
        row.failure_kind.as_deref(),
        Some("upstream_response_failed")
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("payload should still be present"),
    )
    .expect("decode payload json");
    assert_eq!(
        payload_json["upstreamRequestId"].as_str(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );
}

#[tokio::test]
pub(crate) async fn failure_classification_backfill_reads_long_stream_failures_from_raw_file() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");
    let temp_dir = make_temp_test_dir("response-failed-backfill");
    let fixture = write_long_stream_failure_fixture(&temp_dir);
    insert_long_stream_failure_fixture(&pool, &fixture).await;

    let outcome = backfill_failure_classification_from_cursor(&pool, 0, None, Some(10), None)
        .await
        .expect("run failure classification backfill");
    assert_eq!(outcome.summary.scanned, 1);
    assert_eq!(outcome.summary.updated, 1);
    assert_long_stream_backfill_row(&load_long_stream_backfilled_row(&pool).await);
    fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
}

use super::*;
