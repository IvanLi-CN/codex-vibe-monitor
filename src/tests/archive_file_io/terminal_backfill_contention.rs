use super::*;

// This exercises the actual terminal producer and its independent confirmation owner.
// Keeping the maintenance write lock open proves business commits do not wait for it.
#[tokio::test]
async fn terminal_commits_survive_locked_backfill_confirmation_and_coalesce_notifications() {
    let (pool, _config, fixture) =
        retention_test_pool_and_config("terminal-backfill-confirmation-contention").await;
    let maintenance_path = fixture.join("independent-maintenance.sqlite");
    fs::File::create(&maintenance_path).expect("create owned maintenance fixture");
    let maintenance_pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            build_sqlite_connect_options(
                &test_sqlite_url_for_path(&maintenance_path),
                Duration::from_millis(20),
            )
            .expect("maintenance fixture connect options"),
        )
        .await
        .expect("connect independent maintenance database");
    let store = crate::maintenance_store::MaintenanceStore::from_pool(maintenance_pool.clone());
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance progress and controls");
    let mut locked = maintenance_pool
        .acquire()
        .await
        .expect("own maintenance lock");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *locked)
        .await
        .expect("block maintenance confirmations independently of business writes");

    let task = StartupBackfillTask::ReasoningEffort;
    let writer = SqliteBatchWriter::spawn_for_test();
    for invoke_id in ["terminal-before-busy", "terminal-during-backoff"] {
        assert!(
            writer
                .enqueue_terminal(BatchedTerminalInvocationWrite {
                    enqueued_at: None,
                    record: test_proxy_capture_record(invoke_id, "2026-10-16 12:00:00"),
                    attempts: Vec::new(),
                    diagnostic: None,
                    capture_started: None,
                    raw_capture: false,
                    dashboard_terminal_sequence: None,
                    terminal_projection_event_ids: Vec::new(),
                    startup_backfill_tasks: vec![task],
                })
                .enqueued
        );
        writer.flush_buffered_for_test(&pool).await;
        writer.flush_buffered_for_test(&pool).await;
        crate::maintenance::confirm_startup_backfill_wake_obligations_for_test(&maintenance_pool)
            .await;
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id IN ('terminal-before-busy','terminal-during-backoff') AND status='success'",
        )
        .fetch_one(&pool)
        .await
        .expect("read durable business facts while maintenance is locked"),
        2,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM startup_backfill_progress")
            .fetch_one(&pool)
            .await
            .expect("business database must not receive confirmation writes"),
        0,
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM startup_backfill_progress")
            .fetch_one(&mut *locked)
            .await
            .expect("uncommitted confirmations remain absent"),
        0,
    );

    sqlx::query("ROLLBACK")
        .execute(&mut *locked)
        .await
        .expect("release owned maintenance lock");
    drop(locked);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        crate::maintenance::confirm_startup_backfill_wake_obligations_for_test(&maintenance_pool)
            .await;
        let generation = sqlx::query_scalar::<_, i64>(
            "SELECT wake_generation FROM startup_backfill_progress WHERE task_name=?",
        )
        .bind(task.name())
        .fetch_optional(&maintenance_pool)
        .await
        .expect("read durable confirmation after unlock");
        if let Some(generation) = generation {
            assert_eq!(generation, 1, "one confirmation covers both notifications");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "valid obligation must recover after unlock"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    drop(store);
    maintenance_pool.close().await;
    pool.close().await;
    cleanup_temp_test_dir(&fixture);
}

// A6's real sixty-second lock window complements the deterministic scheduler
// tests. It creates no proxy load and counts actual confirmation attempts.
#[tokio::test]
async fn repeated_backfill_notifications_keep_actual_attempts_bounded_during_one_minute_lock() {
    let fixture = make_temp_test_dir("backfill-one-minute-lock");
    let path = fixture.join("maintenance.sqlite");
    fs::File::create(&path).expect("create owned maintenance database");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            build_sqlite_connect_options(
                &test_sqlite_url_for_path(&path),
                Duration::from_millis(20),
            )
            .expect("maintenance lock fixture options"),
        )
        .await
        .expect("connect maintenance lock fixture");
    let store = crate::maintenance_store::MaintenanceStore::from_pool(pool.clone());
    store
        .initialize_schema_for_test()
        .await
        .expect("seed controls");
    let metrics = crate::ObservabilityRuntime::new(true);
    let task = StartupBackfillTask::ReasoningEffort;
    let mut locked = pool.acquire().await.expect("own maintenance lock");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *locked)
        .await
        .expect("hold maintenance database for the full A6 window");
    let started = Instant::now();
    let mut notifications = 0_u64;
    crate::observability::contention::with_performance(metrics.clone(), async {
        while started.elapsed() < Duration::from_secs(60) {
            crate::maintenance::notify_startup_backfill_tasks(
                &pool,
                &[task],
                None,
                "terminal_capture",
            )
            .expect("merge another bounded notification without resetting backoff");
            notifications += 1;
            crate::maintenance::confirm_startup_backfill_wake_obligations_for_test(&pool).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    let text = metrics.render();
    let attempts = text
        .lines()
        .filter(|line| {
            line.starts_with("cvm_backfill_wake_attempts_total{")
                && line.contains(&format!("task=\"{}\"", task.name()))
        })
        .map(|line| {
            line.split_whitespace()
                .last()
                .expect("attempt metric value")
                .parse::<f64>()
                .expect("numeric attempt counter")
        })
        .sum::<f64>();
    assert!(notifications > 1, "exercise repeated notifications");
    assert!(
        (1.0..=16.0).contains(&attempts),
        "actual same-generation attempts must stay within A6: {attempts}"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM startup_backfill_progress")
            .fetch_one(&mut *locked)
            .await
            .expect("no confirmation committed while locked"),
        0,
    );
    println!(
        "backfill_locked_window elapsed_ms={} notifications={} actual_attempts={} maximum_attempts=16",
        started.elapsed().as_millis(),
        notifications,
        attempts,
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *locked)
        .await
        .expect("release maintenance lock");
    drop(locked);
    // The accepted obligation retains its capped 5s + 20% jitter deadline.
    // Recovery allows that deadline plus one second for observation and drain.
    tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            crate::maintenance::confirm_startup_backfill_wake_obligations_for_test(&pool).await;
            let generation = sqlx::query_scalar::<_, i64>(
                "SELECT wake_generation FROM startup_backfill_progress WHERE task_name=?",
            )
            .bind(task.name())
            .fetch_optional(&pool)
            .await
            .expect("read recovered durable obligation");
            if let Some(generation) = generation {
                assert_eq!(
                    generation, 1,
                    "one durable confirmation covers every notification"
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("all accepted notifications recover after unlock");
    drop(store);
    pool.close().await;
    cleanup_temp_test_dir(&fixture);
}
