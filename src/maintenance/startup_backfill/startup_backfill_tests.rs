use super::*;
use sqlx::SqlitePool;

async fn control_test_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect startup backfill test pool");
    sqlx::query("CREATE TABLE managed_tasks (task_key TEXT PRIMARY KEY, enabled INTEGER NOT NULL)")
        .execute(&pool)
        .await
        .expect("create managed task controls");
    sqlx::query(
        "CREATE TABLE startup_backfill_progress (
                task_name TEXT PRIMARY KEY,
                cursor_id INTEGER NOT NULL DEFAULT 0,
                next_run_after TEXT,
                zero_update_streak INTEGER NOT NULL DEFAULT 0,
                last_started_at TEXT,
                last_finished_at TEXT,
                last_scanned INTEGER NOT NULL DEFAULT 0,
                last_updated INTEGER NOT NULL DEFAULT 0,
                last_status TEXT NOT NULL DEFAULT 'idle',
                suspension_reason TEXT,
                next_probe_at TEXT,
                wake_generation INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1
            )",
    )
    .execute(&pool)
    .await
    .expect("create startup backfill controls");
    pool
}

async fn prompt_cache_materialization_test_store() -> crate::maintenance_store::MaintenanceStore {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect maintenance store test pool");
    let store = crate::maintenance_store::MaintenanceStore::from_pool(pool);
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance store schema");
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let task_key = "startup_backfill.prompt_cache_conversations_materialization";
    sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key=?")
        .bind(task_key)
        .execute(&store.pool)
        .await
        .expect("enable prompt-cache task fixture");
    store
        .initialize_prompt_cache_materialization_control(task_key, task.name())
        .await
        .expect("initialize prompt-cache control");
    store
}

#[tokio::test]
async fn disabled_proxy_cost_does_not_wake_new_catalog_rows() {
    let pool = control_test_pool().await;
    sqlx::query(
        "INSERT INTO managed_tasks (task_key,enabled) VALUES ('startup_backfill.proxy_cost',0)",
    )
    .execute(&pool)
    .await
    .expect("insert disabled proxy cost control");
    let catalog = PricingCatalog {
        version: "catalog-v2".to_string(),
        models: HashMap::new(),
    };

    assert_eq!(
        wake_startup_backfill_tasks_with_pricing_catalog(
            &pool,
            &[StartupBackfillTask::ProxyCost],
            Some(&catalog),
            "test_disabled",
        )
        .await
        .expect("wake disabled proxy cost"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM startup_backfill_progress")
            .fetch_one(&pool)
            .await
            .expect("count disabled progress rows"),
        0
    );

    sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key='startup_backfill.proxy_cost'")
        .execute(&pool)
        .await
        .expect("enable proxy cost control");
    assert_eq!(
        wake_startup_backfill_tasks_with_pricing_catalog(
            &pool,
            &[StartupBackfillTask::ProxyCost],
            Some(&catalog),
            "test_enabled",
        )
        .await
        .expect("wake enabled proxy cost"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT enabled FROM startup_backfill_progress LIMIT 1",)
            .fetch_one(&pool)
            .await
            .expect("read enabled progress row"),
        1
    );
}

#[tokio::test]
async fn wake_preserves_legacy_test_pools_without_task_registry() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect legacy startup backfill test pool");
    sqlx::query(
        "CREATE TABLE startup_backfill_progress (
                task_name TEXT PRIMARY KEY,
                cursor_id INTEGER NOT NULL DEFAULT 0,
                next_run_after TEXT,
                zero_update_streak INTEGER NOT NULL DEFAULT 0,
                last_started_at TEXT,
                last_finished_at TEXT,
                last_scanned INTEGER NOT NULL DEFAULT 0,
                last_updated INTEGER NOT NULL DEFAULT 0,
                last_status TEXT NOT NULL DEFAULT 'idle',
                suspension_reason TEXT,
                next_probe_at TEXT,
                wake_generation INTEGER NOT NULL DEFAULT 0,
                enabled INTEGER NOT NULL DEFAULT 1
            )",
    )
    .execute(&pool)
    .await
    .expect("create legacy startup backfill controls");

    assert_eq!(
        wake_startup_backfill_tasks(&pool, &[StartupBackfillTask::ProxyUsage], "legacy_test")
            .await
            .expect("wake legacy startup backfill pool"),
        1
    );
    let progress = load_startup_backfill_progress(&pool, StartupBackfillTask::ProxyUsage.name())
        .await
        .expect("load legacy startup backfill progress");
    assert!(progress.enabled);
}

#[tokio::test]
async fn load_reconciles_existing_progress_with_managed_task_control() {
    let pool = control_test_pool().await;
    let task_name = StartupBackfillTask::ProxyUsage.name();
    sqlx::query(
        "INSERT INTO managed_tasks (task_key,enabled) VALUES ('startup_backfill.proxy_usage',0)",
    )
    .execute(&pool)
    .await
    .expect("insert disabled task control");
    sqlx::query("INSERT INTO startup_backfill_progress (task_name,enabled) VALUES (?,1)")
        .bind(task_name)
        .execute(&pool)
        .await
        .expect("insert stale enabled progress");

    let disabled = load_startup_backfill_progress(&pool, task_name)
        .await
        .expect("reconcile disabled progress");
    assert!(!disabled.enabled);
    assert_eq!(
        disabled.suspension_reason.as_deref(),
        Some("operator_disabled")
    );

    sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key='startup_backfill.proxy_usage'")
        .execute(&pool)
        .await
        .expect("enable task control");
    let enabled = load_startup_backfill_progress(&pool, task_name)
        .await
        .expect("reconcile enabled progress");
    assert!(enabled.enabled);
    assert!(enabled.next_run_after.is_none());
    assert!(enabled.suspension_reason.is_none());
}

#[tokio::test]
async fn startup_backfill_control_upsert_is_idempotent() {
    let pool = control_test_pool().await;
    let first = set_startup_backfill_task_enabled(&pool, StartupBackfillTask::ProxyCost, false)
        .await
        .expect("disable proxy cost control");
    let second = set_startup_backfill_task_enabled(&pool, StartupBackfillTask::ProxyCost, false)
        .await
        .expect("repeat disabling proxy cost control");

    assert!(!first.enabled);
    assert!(!second.enabled);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM startup_backfill_progress WHERE task_name='proxy_cost_v1'",
        )
        .fetch_one(&pool)
        .await
        .expect("count exact proxy cost control rows"),
        1
    );
}

#[test]
fn scheduler_health_tracks_wakes_due_work_and_active_outcomes() {
    let scheduler = StartupBackfillScheduler::default();
    scheduler.wake(StartupBackfillTask::HistoricalRollups);
    scheduler.record_noop_suppressed();

    let woken = scheduler.health_snapshot();
    assert_eq!(woken.state, "healthy");
    assert_eq!(woken.wake_count, 1);
    assert_eq!(woken.woken_task_count, 1);

    assert_eq!(
        scheduler.drain_due_tasks(Utc::now()),
        vec![StartupBackfillTask::HistoricalRollups]
    );
    scheduler.defer_for_pressure(StartupBackfillTask::HistoricalRollups, Utc::now());
    scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, false, true);
    let deferred = scheduler.health_snapshot();
    assert_eq!(deferred.state, "deferred");
    assert_eq!(deferred.due_dispatch_count, 1);
    assert_eq!(deferred.pressure_defer_count, 1);
    assert_eq!(deferred.noop_suppressed_count, 1);

    scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, true, false);
    assert_eq!(scheduler.health_snapshot().state, "degraded");

    scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, false, false);
    let recovered = scheduler.health_snapshot();
    assert_eq!(recovered.state, "healthy");
    assert_eq!(recovered.failure_count, 1);
    assert_eq!(recovered.failed_task_count, 0);
}

#[tokio::test]
async fn startup_pass_does_not_swallow_a_wake_before_the_wait_loop() {
    let scheduler = StartupBackfillScheduler::default();
    let observed_generation = scheduler.generation();
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;

    scheduler.wake(task);
    tokio::time::timeout(
        Duration::from_secs(1),
        scheduler.wait_for_wake(observed_generation),
    )
    .await
    .expect("wake recorded during the startup pass must be observed afterward");

    assert_eq!(scheduler.drain_woken_tasks(), vec![task]);
}

#[tokio::test]
async fn prompt_cache_wake_respects_pressure_retry_deadline() {
    let store = prompt_cache_materialization_test_store().await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let task_name = task.name();

    let scheduler = StartupBackfillScheduler::default();
    scheduler.defer_for_pressure(task, Utc::now() + ChronoDuration::seconds(30));
    let before = scheduler.health_snapshot();
    assert_eq!(
        wake_prompt_cache_materialization_with_scheduler(
            &store,
            "test_terminal_write",
            &scheduler,
        )
        .await
        .expect("attempt wake during pressure defer"),
        0
    );
    assert_eq!(scheduler.health_snapshot().wake_count, before.wake_count);
    assert_eq!(
        load_startup_backfill_progress_from_pool(&store.pool, task_name)
            .await
            .expect("load deferred progress")
            .wake_generation,
        0
    );

    scheduler.record_next_due(task, Utc::now() - ChronoDuration::seconds(1));
    assert_eq!(
        wake_prompt_cache_materialization_with_scheduler(&store, "test_retry_due", &scheduler,)
            .await
            .expect("wake after retry deadline"),
        1
    );
    assert_eq!(
        scheduler.health_snapshot().wake_count,
        before.wake_count + 1
    );
    assert_eq!(
        load_startup_backfill_progress_from_pool(&store.pool, task_name)
            .await
            .expect("load due progress")
            .wake_generation,
        1
    );
}

#[tokio::test]
async fn prompt_cache_queue_wake_preempts_coordinator_retry_deadline() {
    let store = prompt_cache_materialization_test_store().await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let future_deadline = Utc::now() + ChronoDuration::hours(6);
    sqlx::query("UPDATE startup_backfill_progress SET next_run_after=? WHERE task_name=?")
        .bind(format_utc_iso(future_deadline))
        .bind(task.name())
        .execute(&store.pool)
        .await
        .expect("seed coordinator retry deadline");

    let scheduler = StartupBackfillScheduler::default();
    scheduler.defer_for_pressure(task, Utc::now() - ChronoDuration::seconds(1));
    assert_eq!(
        scheduler.take_pressure_deferred_tasks(Utc::now()),
        vec![task]
    );
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(30));
    let outcome = startup_backfill_coordinator_defer_outcome(&scheduler, task, &gate);
    assert!(outcome.is_pressure_deferred());
    assert!(!scheduler.has_future_pressure_deadline(task, Utc::now()));
    scheduler.record_task_result(task, false, outcome.deferred);
    assert_eq!(scheduler.health_snapshot().pressure_defer_count, 0);
    assert_eq!(
        wake_prompt_cache_materialization_with_scheduler(
            &store,
            "test_terminal_queue_event_after_coordinator_yield",
            &scheduler,
        )
        .await
        .expect("queue event should preempt coordinator retry"),
        1
    );
    assert!(
        load_startup_backfill_progress_from_pool(&store.pool, task.name())
            .await
            .expect("load coordinator-woken progress")
            .is_due(Utc::now())
    );
}

#[tokio::test]
async fn prompt_cache_queue_wake_preempts_idle_deadline() {
    let store = prompt_cache_materialization_test_store().await;
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let future_deadline = Utc::now() + ChronoDuration::hours(6);
    sqlx::query("UPDATE startup_backfill_progress SET next_run_after=? WHERE task_name=?")
        .bind(format_utc_iso(future_deadline))
        .bind(task.name())
        .execute(&store.pool)
        .await
        .expect("seed ordinary idle deadline");

    let scheduler = StartupBackfillScheduler::default();
    scheduler.record_next_due(task, future_deadline);
    assert_eq!(
        wake_prompt_cache_materialization_with_scheduler(
            &store,
            "test_terminal_queue_event",
            &scheduler,
        )
        .await
        .expect("wake past ordinary idle deadline"),
        1
    );

    let progress = load_startup_backfill_progress_from_pool(&store.pool, task.name())
        .await
        .expect("load event-woken progress");
    assert!(progress.is_due(Utc::now()));
    assert!(progress.next_run_after.is_none());
    assert_eq!(scheduler.drain_woken_tasks(), vec![task]);
    assert_eq!(scheduler.drain_due_tasks(Utc::now()), vec![task]);
}

#[tokio::test]
async fn prompt_cache_queue_wake_coalesces_concurrent_events() {
    let store = Arc::new(prompt_cache_materialization_test_store().await);
    let scheduler = Arc::new(StartupBackfillScheduler::default());
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    sqlx::query("UPDATE startup_backfill_progress SET next_run_after=? WHERE task_name=?")
        .bind(format_utc_iso(Utc::now() + ChronoDuration::hours(6)))
        .bind(task.name())
        .execute(&store.pool)
        .await
        .expect("seed concurrent event deadline");

    let mut wakes = Vec::new();
    for _ in 0..8 {
        let store = Arc::clone(&store);
        let scheduler = Arc::clone(&scheduler);
        wakes.push(tokio::spawn(async move {
            wake_prompt_cache_materialization_with_scheduler(
                &store,
                "test_concurrent_terminal_queue_event",
                &scheduler,
            )
            .await
        }));
    }
    let mut total_woken = 0;
    for wake in wakes {
        total_woken += wake
            .await
            .expect("concurrent wake must not panic")
            .expect("concurrent wake must succeed");
    }

    assert_eq!(total_woken, 1);
    assert_eq!(scheduler.health_snapshot().wake_count, 1);
    assert_eq!(
        load_startup_backfill_progress_from_pool(&store.pool, task.name())
            .await
            .expect("load coalesced progress")
            .wake_generation,
        1
    );
    assert_eq!(scheduler.drain_woken_tasks(), vec![task]);
}

#[tokio::test]
async fn prompt_cache_queue_wake_survives_late_run_checkpoint() {
    let store = prompt_cache_materialization_test_store().await;
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let future_deadline = Utc::now() + ChronoDuration::hours(6);
    sqlx::query("UPDATE startup_backfill_progress SET next_run_after=? WHERE task_name=?")
        .bind(format_utc_iso(future_deadline))
        .bind(task.name())
        .execute(&store.pool)
        .await
        .expect("seed late checkpoint deadline");
    let before_event = load_startup_backfill_progress_from_pool(&store.pool, task.name())
        .await
        .expect("load pre-event checkpoint");

    assert_eq!(
        wake_prompt_cache_materialization_with_scheduler(
            &store,
            "test_inflight_terminal_queue_event",
            &scheduler,
        )
        .await
        .expect("wake in-flight materialization"),
        1
    );
    let generation = store
        .prompt_cache_materialization_control
        .snapshot()
        .expect("initialized control")
        .generation;
    let _checkpoint_guard = store
        .prompt_cache_materialization_control
        .lock_current_generation(generation)
        .await
        .expect("admit late run checkpoint");
    let checkpoint_deadline = format_utc_iso(future_deadline);
    save_startup_backfill_progress_for_task(
        &store.pool,
        task,
        task.name(),
        before_event.wake_generation,
        StartupBackfillProgressUpdate {
            cursor_id: before_event.cursor_id,
            scanned: before_event.last_scanned,
            updated: before_event.last_updated,
            zero_update_streak: before_event.zero_update_streak,
            next_run_after: &checkpoint_deadline,
            status: STARTUP_BACKFILL_STATUS_OK,
            suspension_reason: None,
        },
    )
    .await
    .expect("reconcile late run checkpoint");

    let after_checkpoint = load_startup_backfill_progress_from_pool(&store.pool, task.name())
        .await
        .expect("load reconciled checkpoint");
    assert!(after_checkpoint.is_due(Utc::now()));
    assert!(after_checkpoint.next_run_after.is_none());
    assert_eq!(scheduler.drain_woken_tasks(), vec![task]);
}

#[tokio::test]
async fn prompt_cache_queue_wake_survives_coordinator_defer_checkpoint() {
    let store = Arc::new(prompt_cache_materialization_test_store().await);
    let scheduler = Arc::new(StartupBackfillScheduler::default());
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let future_deadline = Utc::now() + ChronoDuration::hours(6);
    sqlx::query("UPDATE startup_backfill_progress SET next_run_after=? WHERE task_name=?")
        .bind(format_utc_iso(future_deadline))
        .bind(task.name())
        .execute(&store.pool)
        .await
        .expect("seed coordinator checkpoint deadline");
    let before_event = load_startup_backfill_progress_from_pool(&store.pool, task.name())
        .await
        .expect("load pre-event coordinator checkpoint");
    let generation = store
        .prompt_cache_materialization_control
        .snapshot()
        .expect("initialized control")
        .generation;
    let checkpoint_guard = store
        .prompt_cache_materialization_control
        .lock_current_generation(generation)
        .await
        .expect("admit coordinator checkpoint");
    let wake_store = Arc::clone(&store);
    let wake_scheduler = Arc::clone(&scheduler);
    let wake = tokio::spawn(async move {
        wake_prompt_cache_materialization_with_scheduler(
            &wake_store,
            "test_queue_event_during_coordinator_checkpoint",
            &wake_scheduler,
        )
        .await
    });
    tokio::task::yield_now().await;

    let retry_after = format_utc_iso(future_deadline);
    save_startup_backfill_progress_for_task(
        &store.pool,
        task,
        task.name(),
        before_event.wake_generation,
        StartupBackfillProgressUpdate {
            cursor_id: before_event.cursor_id,
            scanned: before_event.last_scanned,
            updated: before_event.last_updated,
            zero_update_streak: before_event.zero_update_streak,
            next_run_after: &retry_after,
            status: STARTUP_BACKFILL_STATUS_IDLE,
            suspension_reason: None,
        },
    )
    .await
    .expect("save coordinator retry checkpoint");
    scheduler.defer_for_background_busy(task, future_deadline);
    drop(checkpoint_guard);

    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), wake)
            .await
            .expect("queue event should not remain behind coordinator retry")
            .expect("queue event wake task must not panic")
            .expect("queue event wake must succeed"),
        1
    );
    let after_wake = load_startup_backfill_progress_from_pool(&store.pool, task.name())
        .await
        .expect("load coordinator-reconciled checkpoint");
    assert!(after_wake.is_due(Utc::now()));
    assert!(after_wake.next_run_after.is_none());
    assert_eq!(scheduler.drain_woken_tasks(), vec![task]);
}

#[test]
fn pressure_defer_uses_the_gate_absolute_deadline() {
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    gate.record_pressure("test", "forced");
    let expected_deadline = gate
        .pressure_cooldown_deadline_epoch_ms()
        .expect("active pressure cooldown deadline");

    let retry_at = startup_backfill_pressure_retry_at(
        &gate,
        crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms: 1 },
    );

    assert_eq!(retry_at.timestamp_millis() as u64, expected_deadline);
}

#[test]
fn failed_prompt_cache_materialization_does_not_report_cumulative_work() {
    let outcome = prompt_cache_materialization_failed_outcome(Some("stats_rebuild".to_string()));

    assert_eq!(outcome.phase, "stats_rebuild");
    assert_eq!(outcome.scanned, 0);
    assert_eq!(outcome.updated, 0);
    assert_eq!(outcome.batch_count, 0);
}

#[test]
fn pressure_defer_schedules_one_deadline_and_dispatches_once() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::ReasoningEffort;
    let deadline = DateTime::<Utc>::from_timestamp_millis(1_800_000_000_750)
        .expect("valid fixed pressure deadline");

    scheduler.defer_for_pressure(task, deadline);
    assert!(
        scheduler
            .drain_due_tasks(deadline - ChronoDuration::milliseconds(1))
            .is_empty()
    );
    let waiting = scheduler.health_snapshot();
    assert_eq!(waiting.wake_count, 0);
    assert_eq!(waiting.due_dispatch_count, 0);
    assert_eq!(waiting.pressure_defer_count, 0);
    assert_eq!(waiting.scheduled_task_count, 1);

    assert_eq!(scheduler.drain_due_tasks(deadline), vec![task]);
    scheduler.record_task_result(task, false, true);
    assert!(scheduler.drain_due_tasks(deadline).is_empty());
    let deferred = scheduler.health_snapshot();
    assert_eq!(deferred.wake_count, 0);
    assert_eq!(deferred.due_dispatch_count, 1);
    assert_eq!(deferred.pressure_defer_count, 1);
    assert_eq!(deferred.scheduled_task_count, 0);
    assert_eq!(deferred.deferred_task_count, 1);
}

#[test]
fn disabled_startup_backfill_root_keeps_prompt_cache_control_independent() {
    let prompt_cache_task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let selected = [
        StartupBackfillTask::ProxyUsage,
        prompt_cache_task,
        StartupBackfillTask::ReasoningEffort,
    ];

    assert_eq!(
        prompt_cache_tasks_when_startup_backfill_root_is_skipped(Some(&selected)),
        vec![prompt_cache_task]
    );
    assert_eq!(
        prompt_cache_tasks_when_startup_backfill_root_is_skipped(None),
        vec![prompt_cache_task]
    );
    assert!(
        prompt_cache_tasks_when_startup_backfill_root_is_skipped(Some(&[
            StartupBackfillTask::ProxyUsage,
        ]))
        .is_empty()
    );

    let scheduler = StartupBackfillScheduler::default();
    let generic_task = StartupBackfillTask::ProxyUsage;
    let deadline = DateTime::<Utc>::from_timestamp_millis(1_800_000_000_750)
        .expect("valid fixed pressure deadline");
    scheduler.defer_for_pressure(generic_task, deadline);
    scheduler.defer_for_pressure(prompt_cache_task, deadline);

    assert_eq!(
        scheduler.take_pressure_deferred_tasks_matching(deadline, Some(&[prompt_cache_task])),
        vec![prompt_cache_task]
    );
    assert_eq!(
        scheduler.take_pressure_deferred_tasks(deadline),
        vec![generic_task],
        "root-disabled dispatch must leave generic pressure work queued"
    );
}

#[tokio::test]
async fn pressure_eligibility_change_preserves_background_busy_deadline() {
    let scheduler = Arc::new(StartupBackfillScheduler::default());
    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(30),
    ));
    let permit = gate
        .try_begin_background("test-holder")
        .expect("occupy the sole background slot");
    let observed_eligibility = gate.eligibility_generation();
    let task = StartupBackfillTask::ReasoningEffort;
    let deadline = Utc::now() + ChronoDuration::minutes(5);

    scheduler.defer_for_pressure(task, deadline);
    scheduler.record_task_result(task, false, true);

    assert!(
        scheduler.drain_due_tasks(Utc::now()).is_empty(),
        "the in-memory fallback deadline must not be due yet"
    );

    let wake_gate = gate.clone();
    let wake_scheduler = scheduler.clone();
    let wake = tokio::spawn(async move {
        wake_gate
            .wait_for_eligibility_change(observed_eligibility)
            .await;
        wake_scheduler.take_pressure_deferred_tasks(Utc::now())
    });
    tokio::task::yield_now().await;
    assert!(
        !wake.is_finished(),
        "the task must wait for an eligibility-clear event before its fallback deadline"
    );

    drop(permit);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), wake)
            .await
            .expect("permit release must wake the deferred task before its deadline")
            .expect("eligibility waiter must not panic")
            .is_empty(),
        "eligibility changes must not bypass the fallback deadline"
    );
    assert_eq!(scheduler.next_due(), Some(deadline));
    assert_eq!(
        scheduler.take_pressure_deferred_tasks(deadline),
        vec![task],
        "the deferred task becomes eligible exactly at its deadline"
    );
}

#[test]
fn sqlite_busy_and_locked_are_actual_backfill_failures() {
    for error in [
        anyhow::anyhow!("database is busy"),
        anyhow::anyhow!("database table is locked"),
    ] {
        assert_eq!(
            startup_backfill_failure_kind(&error),
            StartupBackfillFailureKind::SqliteBusyOrLocked
        );
    }
    assert_eq!(
        startup_backfill_failure_kind(&anyhow::anyhow!("archive directory lock busy")),
        StartupBackfillFailureKind::ArchiveLockBusy
    );
    assert_eq!(
        startup_backfill_failure_kind(&anyhow::anyhow!("source unavailable")),
        StartupBackfillFailureKind::Operation
    );

    let scheduler = StartupBackfillScheduler::default();
    scheduler.record_task_result(StartupBackfillTask::ReasoningEffort, true, false);
    let health = scheduler.health_snapshot();
    assert_eq!(health.state, "degraded");
    assert_eq!(health.failure_count, 1);
    assert_eq!(health.pressure_defer_count, 0);
    assert_eq!(health.failed_task_count, 1);
    assert_eq!(health.deferred_task_count, 0);
}

#[test]
fn coverage_repair_health_is_independent_from_historical_rollups() {
    let scheduler = StartupBackfillScheduler::default();
    scheduler.record_task_result(StartupBackfillTask::HistoricalRollups, true, false);

    scheduler.defer_for_pressure(StartupBackfillTask::AccountActivityV2Coverage, Utc::now());
    scheduler.record_task_result(StartupBackfillTask::AccountActivityV2Coverage, false, true);

    let health = scheduler.health_snapshot();
    assert_eq!(health.state, "degraded");
    assert_eq!(health.failed_task_count, 1);
    assert_eq!(health.pressure_defer_count, 1);
}

#[test]
fn coverage_repair_does_not_repeat_its_planner_in_the_following_hourly_refresh() {
    assert_eq!(
        startup_backfill_hourly_rollup_refresh_scope(),
        HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
    );
}

#[test]
fn actionable_no_progress_backoff_caps_at_fifteen_minutes() {
    let run = StartupBackfillRunState {
        scanned: 2,
        updated: 0,
        hit_scan_limit: true,
        ..StartupBackfillRunState::default()
    };
    assert_eq!(
        startup_backfill_next_delay(&run, 1),
        Duration::from_secs(15)
    );
    assert_eq!(
        startup_backfill_next_delay(&run, 2),
        Duration::from_secs(60)
    );
    assert_eq!(
        startup_backfill_next_delay(&run, 3),
        Duration::from_secs(5 * 60)
    );
    assert_eq!(
        startup_backfill_next_delay(&run, 4),
        Duration::from_secs(15 * 60)
    );
    assert_eq!(
        startup_backfill_next_delay(&run, 99),
        Duration::from_secs(15 * 60)
    );
}

#[test]
fn historical_rollup_cursor_advance_after_budget_exhaustion_retries_without_a_task_run() {
    let run = StartupBackfillRunState {
        next_cursor_id: 12,
        scanned: 1,
        retry_soon: true,
        ..StartupBackfillRunState::default()
    };

    assert_eq!(
        startup_backfill_next_delay(&run, 1),
        Duration::from_secs(15)
    );
    assert!(!startup_backfill_run_is_actionable(&run));
}

#[test]
fn historical_rollup_budget_retry_stays_short_after_cursor_wrap() {
    let retry_soon = historical_rollup_should_retry_soon(true, 1);
    assert!(retry_soon);
    assert!(historical_rollup_should_retry_soon(true, 32));
    assert!(!historical_rollup_should_retry_soon(true, 0));
    assert!(!historical_rollup_should_retry_soon(false, 1));

    let run = StartupBackfillRunState {
        retry_soon,
        ..StartupBackfillRunState::default()
    };
    assert_eq!(
        startup_backfill_next_delay(&run, 0),
        Duration::from_secs(15)
    );
    assert!(!startup_backfill_run_is_actionable(&run));
}

#[test]
fn overdue_backfill_deadline_runs_without_an_idle_sleep() {
    assert_eq!(
        startup_backfill_wait_duration(Some(Utc::now() - ChronoDuration::seconds(1))),
        Duration::ZERO
    );
}

#[test]
fn scheduler_drains_only_tasks_with_an_expired_deadline() {
    let scheduler = StartupBackfillScheduler::default();
    let future_due = Utc::now() + ChronoDuration::hours(1);
    scheduler.record_next_due(
        StartupBackfillTask::HistoricalRollups,
        Utc::now() - ChronoDuration::seconds(1),
    );
    scheduler.record_next_due(StartupBackfillTask::ReasoningEffort, future_due);

    assert_eq!(
        scheduler.drain_due_tasks(Utc::now()),
        vec![StartupBackfillTask::HistoricalRollups]
    );
    assert_eq!(scheduler.next_due(), Some(future_due));
}

#[test]
fn only_progress_or_non_idle_backlog_triggers_rollup_refresh() {
    assert!(startup_backfill_run_is_actionable(
        &StartupBackfillRunState {
            updated: 1,
            ..StartupBackfillRunState::default()
        }
    ));
    assert!(startup_backfill_run_is_actionable(
        &StartupBackfillRunState {
            hit_scan_limit: true,
            ..StartupBackfillRunState::default()
        }
    ));
    assert!(!startup_backfill_run_is_actionable(
        &StartupBackfillRunState {
            scanned: 1,
            force_idle: true,
            ..StartupBackfillRunState::default()
        }
    ));
    assert!(!startup_backfill_run_is_actionable(
        &StartupBackfillRunState::default()
    ));
}

#[test]
fn terminal_payload_input_wakes_only_missing_field_repairs() {
    let mut record = crate::tests::test_proxy_capture_record(
        "startup-backfill-terminal-wake",
        "2026-08-09 12:00:00",
    );
    record.usage.total_tokens = None;
    record.cost = None;
    record.payload = Some("{}".to_string());
    record.req_raw.path = Some("/tmp/request.raw".to_string());
    record.resp_raw.path = Some("/tmp/response.raw".to_string());

    let tasks = startup_backfill_tasks_for_terminal(&api_invocation_from_runtime_record(&record));

    assert_eq!(
        tasks,
        vec![
            StartupBackfillTask::ProxyUsage,
            StartupBackfillTask::PromptCacheKey,
            StartupBackfillTask::RequestedServiceTier,
            StartupBackfillTask::ReasoningEffort,
            StartupBackfillTask::InvocationServiceTier,
        ]
    );

    let mut materialization_record =
        api_invocation_from_runtime_record(&crate::tests::test_proxy_capture_record(
            "startup-backfill-materialization",
            "2026-08-09 12:00:30",
        ));
    materialization_record.prompt_cache_key = Some("startup-materialization-key".to_string());
    assert_eq!(
        startup_backfill_tasks_for_terminal(&materialization_record),
        vec![StartupBackfillTask::PromptCacheConversationsMaterialization]
    );

    let complete = api_invocation_from_runtime_record(&crate::tests::test_proxy_capture_record(
        "startup-backfill-terminal-complete",
        "2026-08-09 12:01:00",
    ));
    assert_eq!(
        startup_backfill_tasks_for_terminal(&complete),
        vec![StartupBackfillTask::PromptCacheConversationsMaterialization]
    );
}

#[test]
fn source_unavailable_probe_uses_one_shared_budget() {
    assert_eq!(startup_backfill_scan_limit(true), 100);
    assert_eq!(startup_backfill_run_budget(true), Duration::from_secs(2));
    assert_eq!(
        startup_backfill_scan_limit(false),
        STARTUP_BACKFILL_SCAN_LIMIT
    );
    assert_eq!(
        startup_backfill_run_budget(false),
        Duration::from_secs(STARTUP_BACKFILL_RUN_BUDGET_SECS)
    );
}

#[test]
fn historical_rollup_backfill_run_state_backs_off_when_only_blocked_archives_remain() {
    let before = HistoricalRollupBackfillSnapshot {
        pending_buckets: 2,
        legacy_archive_pending: 1,
        pending_usage_breakdown_batches: 1,
        last_materialized_hour: None,
        alert_level: HistoricalRollupBackfillAlertLevel::Critical,
    };
    let after = before.clone();
    let summary = HistoricalRollupMaterializationSummary {
        scanned_archive_batches: 1,
        blocked_archive_batches: 1,
        ..HistoricalRollupMaterializationSummary::default()
    };

    let run = historical_rollup_startup_backfill_run_state(7, 0, &before, &after, &summary, 1, 1);

    assert_eq!(run.next_cursor_id, 8);
    assert_eq!(run.scanned, 1);
    assert_eq!(run.updated, 0);
    assert!(!run.hit_scan_limit);
    assert!(run.force_idle);
}

#[test]
fn historical_rollup_backfill_run_state_stays_active_while_catching_up() {
    let before = HistoricalRollupBackfillSnapshot {
        pending_buckets: 8,
        legacy_archive_pending: 3,
        pending_usage_breakdown_batches: 3,
        last_materialized_hour: None,
        alert_level: HistoricalRollupBackfillAlertLevel::Critical,
    };
    let after = HistoricalRollupBackfillSnapshot {
        pending_buckets: 4,
        legacy_archive_pending: 2,
        pending_usage_breakdown_batches: 2,
        last_materialized_hour: None,
        alert_level: HistoricalRollupBackfillAlertLevel::Warn,
    };
    let summary = HistoricalRollupMaterializationSummary {
        scanned_archive_batches: 1,
        materialized_archive_batches: 1,
        materialized_invocation_batches: 1,
        ..HistoricalRollupMaterializationSummary::default()
    };

    let run = historical_rollup_startup_backfill_run_state(11, 0, &before, &after, &summary, 3, 2);

    assert_eq!(run.next_cursor_id, 12);
    assert_eq!(run.scanned, 1);
    assert_eq!(run.updated, 4);
    assert!(run.hit_scan_limit);
    assert!(!run.force_idle);
}

#[test]
fn historical_rollup_backfill_run_state_stays_active_when_partial_scan_found_only_blocked_work() {
    let before = HistoricalRollupBackfillSnapshot {
        pending_buckets: 8,
        legacy_archive_pending: 3,
        pending_usage_breakdown_batches: 3,
        last_materialized_hour: None,
        alert_level: HistoricalRollupBackfillAlertLevel::Critical,
    };
    let after = before.clone();
    let summary = HistoricalRollupMaterializationSummary {
        scanned_archive_batches: 1,
        blocked_archive_batches: 1,
        ..HistoricalRollupMaterializationSummary::default()
    };

    let run = historical_rollup_startup_backfill_run_state(5, 0, &before, &after, &summary, 3, 3);

    assert_eq!(run.next_cursor_id, 6);
    assert_eq!(run.scanned, 1);
    assert_eq!(run.updated, 0);
    assert!(run.hit_scan_limit);
    assert!(!run.force_idle);
}

#[test]
fn historical_rollup_backfill_run_state_does_not_back_off_when_only_blocked_archive_was_after_skip()
{
    let before = HistoricalRollupBackfillSnapshot {
        pending_buckets: 8,
        legacy_archive_pending: 2,
        pending_usage_breakdown_batches: 2,
        last_materialized_hour: None,
        alert_level: HistoricalRollupBackfillAlertLevel::Critical,
    };
    let after = before.clone();
    let summary = HistoricalRollupMaterializationSummary {
        scanned_archive_batches: 2,
        skipped_archive_batches: 1,
        blocked_archive_batches: 1,
        ..HistoricalRollupMaterializationSummary::default()
    };

    let run = historical_rollup_startup_backfill_run_state(9, 0, &before, &after, &summary, 2, 2);

    assert_eq!(run.next_cursor_id, 10);
    assert_eq!(run.scanned, 2);
    assert_eq!(run.updated, 0);
    assert!(run.hit_scan_limit);
    assert!(!run.force_idle);
}

#[test]
fn historical_rollup_backfill_run_state_backs_off_after_blocked_cycle_across_multiple_passes() {
    let before = HistoricalRollupBackfillSnapshot {
        pending_buckets: 8,
        legacy_archive_pending: 2,
        pending_usage_breakdown_batches: 2,
        last_materialized_hour: None,
        alert_level: HistoricalRollupBackfillAlertLevel::Critical,
    };
    let after = before.clone();
    let summary = HistoricalRollupMaterializationSummary {
        scanned_archive_batches: 2,
        skipped_archive_batches: 1,
        blocked_archive_batches: 1,
        ..HistoricalRollupMaterializationSummary::default()
    };

    let run = historical_rollup_startup_backfill_run_state(9, 1, &before, &after, &summary, 2, 2);

    assert_eq!(run.next_cursor_id, 10);
    assert_eq!(run.scanned, 2);
    assert_eq!(run.updated, 0);
    assert!(!run.hit_scan_limit);
    assert!(run.force_idle);
}

#[test]
fn managed_prompt_cache_observation_keeps_root_task_identity() {
    assert_eq!(
        startup_backfill_observation_task_key(Some("prompt_cache_materialization")),
        "prompt_cache_materialization"
    );
    assert_eq!(
        startup_backfill_observation_task_key(None),
        "startup_backfill"
    );
}

#[test]
fn stale_prompt_cache_disable_schedule_cannot_erase_a_resume_wake() {
    let control = crate::maintenance_store::PromptCacheMaterializationControl::default();
    control.initialize(true);
    let disabled = control.publish_committed(false);
    let resumed = control.publish_committed(true);
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    apply_prompt_cache_control_schedule(&control, resumed, &scheduler, task);
    let resumed_deadline = scheduler.next_due_for(task);
    apply_prompt_cache_control_schedule(&control, disabled, &scheduler, task);
    assert_eq!(scheduler.next_due_for(task), resumed_deadline);
    assert_eq!(scheduler.drain_woken_tasks(), vec![task]);
    assert_eq!(scheduler.drain_due_tasks(Utc::now()), vec![task]);
    assert_eq!(scheduler.wake_count.load(Ordering::Relaxed), 1);
}
