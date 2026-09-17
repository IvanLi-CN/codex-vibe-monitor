#[tokio::test]
pub(crate) async fn startup_historical_rollup_backfill_prioritizes_usage_breakdown_repair() {
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

pub(crate) async fn seed_missing_historical_rollup_startup_candidates(
    pool: &SqlitePool,
    count: usize,
) -> i64 {
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
pub(crate) async fn historical_rollup_startup_window_bounds_a_twenty_thousand_batch_fixture_and_wraps()
 {
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
pub(crate) async fn historical_rollup_startup_window_advances_in_keyset_order_despite_usage_breakdown_candidates()
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
pub(crate) async fn historical_rollup_startup_window_preserves_cursor_when_budget_is_exhausted() {
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
pub(crate) async fn startup_historical_rollup_backfill_persists_cursor_and_defers_under_pressure() {
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
pub(crate) async fn historical_rollup_noop_pass_does_not_create_a_system_task_run() {
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
    assert_eq!(after, before);
}

#[tokio::test]
pub(crate) async fn startup_backfill_not_due_check_does_not_claim_background_gate() {
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
pub(crate) async fn startup_backfill_idle_pass_does_not_create_a_system_task_run() {
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
    assert_eq!(
        after, before,
        "idle maintenance must not create task-run audit rows"
    );
}

#[tokio::test]
pub(crate) async fn startup_backfill_event_wakes_only_the_matching_task() {
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
pub(crate) async fn startup_backfill_pressure_defer_never_accesses_sqlite() {
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
pub(crate) async fn startup_backfill_pressure_defer_has_one_deadline_and_no_task_run_audit() {
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
pub(crate) async fn pressure_eligibility_wake_rechecks_durable_backfill_deadline() {
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
pub(crate) async fn startup_backfill_progress_lookup_busy_closes_gate_before_permit_release() {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
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
pub(crate) async fn startup_backfill_running_state_busy_closes_gate_before_permit_release() {
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
pub(crate) async fn startup_backfill_progress_persist_busy_closes_gate_before_permit_release() {
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
pub(crate) async fn startup_backfill_failure_persist_busy_closes_gate_before_permit_release() {
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
pub(crate) async fn startup_backfill_busy_failure_persists_failed_state_and_bounded_retry() {
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
pub(crate) async fn startup_backfill_busy_failure_closes_pressure_gate_before_the_next_task_reads_sqlite()
 {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let temp_dir = make_temp_test_dir("startup-backfill-busy-pressure-gate");
    seed_busy_reasoning_backfill_candidate(&state, &temp_dir).await;

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let cancel = CancellationToken::new();
    let first_tasks = [StartupBackfillTask::ReasoningEffort];
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before injected busy failure");

    let first = run_busy_failure_first_pass(&state, &cancel, &first_tasks, &gate).await;
    let (task_runs_after, task_run_status, failed_progress) =
        load_busy_failure_audit_and_progress(&state, first_tasks[0]).await;
    assert_busy_failure_recorded(
        &first,
        task_runs_before,
        task_runs_after,
        &task_run_status,
        &failed_progress,
        &gate,
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

async fn seed_busy_reasoning_backfill_candidate(state: &Arc<AppState>, temp_dir: &Path) {
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
}

async fn run_busy_failure_first_pass(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    selected_tasks: &[StartupBackfillTask],
    gate: &crate::db_pressure::DbPressureGate,
) -> StartupBackfillMaintenancePass {
    let outcome = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        cancel,
        Some(selected_tasks),
        gate,
    )
    .await;
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush injected busy failure task audit");
    outcome
}

async fn load_busy_failure_audit_and_progress(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
) -> (i64, String, StartupBackfillProgress) {
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
    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load persisted busy failure progress");
    (task_runs_after, task_run_status, progress)
}

fn assert_busy_failure_recorded(
    outcome: &StartupBackfillMaintenancePass,
    task_runs_before: i64,
    task_runs_after: i64,
    task_run_status: &str,
    progress: &StartupBackfillProgress,
    gate: &crate::db_pressure::DbPressureGate,
) {
    assert!(!outcome.ran_actionable_task);
    assert!(outcome.had_failure);
    assert_eq!(task_runs_after, task_runs_before + 1);
    assert_eq!(task_run_status, "failed");
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_FAILED);
    assert!(
        gate.pressure_cooldown_deadline_epoch_ms().is_some(),
        "the durable busy failure must close the gate before its permit is released"
    );
}

async fn coverage_repair_state_with_trigger(
    trigger_name: &str,
    trigger_message: &str,
) -> Arc<AppState> {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    seed_due_startup_backfill_progress(&state, task).await;
    sqlx::query(
        "DELETE FROM hourly_rollup_live_progress WHERE dataset = 'invocation_account_activity_v2_repair_generation'",
    )
    .execute(&state.pool)
    .await
    .expect("make the coverage repair generation initialization due");
    let trigger_sql = format!(
        "CREATE TRIGGER {trigger_name} BEFORE INSERT ON hourly_rollup_live_progress \
         WHEN NEW.dataset = 'invocation_account_activity_v2_repair_generation' BEGIN \
         SELECT RAISE(ABORT, '{trigger_message}'); END"
    );
    sqlx::query(&trigger_sql)
        .execute(&state.pool)
        .await
        .expect("inject coverage repair failure");
    state
}

async fn startup_backfill_task_run_count(state: &Arc<AppState>) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'")
        .fetch_one(&state.pool)
        .await
        .expect("count startup backfill task runs")
}

async fn assert_locked_coverage_maintenance_defers(
    task: StartupBackfillTask,
    cancel: &CancellationToken,
) {
    let state = coverage_repair_state_with_trigger(
        "startup_coverage_repair_maintenance_busy",
        "database table is locked",
    )
    .await;
    let selected_tasks = [task];
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let audits_before = startup_backfill_task_run_count(&state).await;
    let outcome = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        cancel,
        Some(&selected_tasks),
        &gate,
    )
    .await;
    let audits_after = startup_backfill_task_run_count(&state).await;
    let retry_progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load the coverage retry deadline after the lock");

    assert!(!outcome.ran_actionable_task);
    assert!(!outcome.had_failure);
    assert_eq!(audits_after, audits_before);
    assert_eq!(gate.snapshot().pressure_events, 1);
    assert!(!retry_progress.is_due(Utc::now()));

    state.pool.close().await;
    let next_tasks = [StartupBackfillTask::PromptCacheKey];
    let next_outcome =
        run_startup_backfill_maintenance_pass_with_gate(state, cancel, Some(&next_tasks), &gate)
            .await;
    assert!(!next_outcome.ran_actionable_task);
    assert!(!next_outcome.had_failure);
    assert!(gate.snapshot().background_skips >= 1);
}

async fn assert_non_lock_coverage_maintenance_audits(
    task: StartupBackfillTask,
    cancel: &CancellationToken,
) {
    let state = coverage_repair_state_with_trigger(
        "startup_coverage_repair_maintenance_error",
        "coverage repair operation failed",
    )
    .await;
    let selected_tasks = [task];
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let audits_before = startup_backfill_task_run_count(&state).await;
    let outcome = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        cancel,
        Some(&selected_tasks),
        &gate,
    )
    .await;
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush the non-lock coverage failure audit");
    let audits_after = startup_backfill_task_run_count(&state).await;
    let audit_status: String = sqlx::query_scalar(
        "SELECT status FROM system_task_runs WHERE task_kind = 'startup_backfill' ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load the non-lock coverage failure audit");

    assert!(!outcome.ran_actionable_task);
    assert!(outcome.had_failure);
    assert_eq!(audits_after, audits_before + 1);
    assert_eq!(audit_status, "failed");
    assert_eq!(gate.snapshot().pressure_events, 0);
}

#[tokio::test]
pub(crate) async fn startup_coverage_lock_maintenance_pass_defers_without_audit_but_non_lock_failure_audits()
 {
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let cancel = CancellationToken::new();
    assert_locked_coverage_maintenance_defers(task, &cancel).await;
    assert_non_lock_coverage_maintenance_audits(task, &cancel).await;
}

use super::*;
