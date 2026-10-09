use std::{str::FromStr, time::Duration};

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

async fn open_file_pool(path: &std::path::Path, busy_timeout: Duration) -> sqlx::SqlitePool {
    let options = SqliteConnectOptions::from_str(&crate::tests::test_sqlite_url_for_path(path))
        .expect("parse file-backed SQLite URL")
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(busy_timeout);
    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .expect("connect file-backed SQLite pool")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_cache_fair_queue_file_lock_preserves_staging_and_resumes_next_key() {
    let temp_dir = crate::tests::make_temp_test_dir("prompt-cache-fair-queue-lock");
    let path = temp_dir.join("business.sqlite");
    let pool = open_file_pool(&path, Duration::from_millis(100)).await;
    crate::ensure_schema(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    for (key, count) in [("a-hot", 512), ("z-cold", 1)] {
        for index in 0..count {
            sqlx::query("INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,total_tokens,payload,raw_response) VALUES (?1,'2026-09-01T00:00:00Z',?2,'success',1,?3,'{}')")
                .bind(format!("fair-{key}-{index}"))
                .bind(crate::SOURCE_PROXY)
                .bind(serde_json::json!({"promptCacheKey":key}).to_string())
                .execute(&mut *tx).await.unwrap();
        }
    }
    tx.commit().await.unwrap();
    crate::run_prompt_cache_conversations_materialization(&pool, 2, None)
        .await
        .unwrap();
    sqlx::query("UPDATE prompt_cache_conversation_migration_progress SET phase='queue_drain',cursor_key=NULL")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT OR IGNORE INTO schema_refresh_migrations (migration_name) VALUES ('prompt_cache_conversations_v1')")
        .execute(&pool).await.unwrap();
    let should_yield = || {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_staging WHERE prompt_cache_key='a-hot' AND cursor_id > 0")
                .fetch_one(&pool).await.unwrap() > 0
        })
        })
    };
    let first = crate::run_prompt_cache_conversations_materialization_with_pressure(
        &pool,
        1,
        None,
        &should_yield,
    )
    .await
    .unwrap();
    assert_eq!((first.scanned, first.updated), (1, 0));
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let error = crate::run_prompt_cache_conversations_materialization(
        &pool,
        400,
        Some(Duration::from_secs(3)),
    )
    .await
    .expect_err("business write lock must stop the attempt");
    let error_debug = format!("{error:#?}").to_ascii_lowercase();
    assert!(
        error_debug.contains("locked"),
        "business write lock returned unexpected error: {error:#?}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    let (cursor, staged, queued): (String, i64, i64) = sqlx::query_as("SELECT cursor_key,(SELECT cursor_id FROM prompt_cache_conversation_stats_refresh_staging WHERE prompt_cache_key='a-hot'),(SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue) FROM prompt_cache_conversation_migration_progress")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(cursor, "a-hot");
    assert!(staged > 0);
    assert_eq!(queued, 2);
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .unwrap();
    drop(blocker);
    pool.close().await;

    let restarted = open_file_pool(&path, Duration::from_millis(100)).await;
    crate::ensure_prompt_cache_conversations_schema(&restarted)
        .await
        .unwrap();
    let next = crate::run_prompt_cache_conversations_materialization(&restarted, 1, None)
        .await
        .unwrap();
    assert_eq!((next.scanned, next.updated), (1, 1));
    for _ in 0..8 {
        if crate::run_prompt_cache_conversations_materialization(&restarted, 400, None)
            .await
            .unwrap()
            .complete
        {
            break;
        }
    }
    assert!(
        crate::prompt_cache_conversation_materialization_is_complete(&restarted)
            .await
            .unwrap()
    );
    let counts: Vec<i64> = sqlx::query_scalar(
        "SELECT request_count FROM prompt_cache_conversations ORDER BY prompt_cache_key",
    )
    .fetch_all(&restarted)
    .await
    .unwrap();
    assert_eq!(counts, vec![512, 1]);
    restarted.close().await;
    crate::tests::cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn invocation_ranges_timed_out_sql_drains_before_replacement_generation() {
    use crate::prompt_cache_conversations::invocation_ranges::InvocationRangeManager;
    use std::sync::Arc;

    let temp_dir = crate::tests::make_temp_test_dir("invocation-range-return-worker-fence");
    let pool = open_file_pool(&temp_dir.join("business.sqlite"), Duration::from_secs(1)).await;
    crate::ensure_schema(&pool).await.unwrap();
    // Warm the pool after additive schema setup, before measuring a deliberate
    // database lock. Connection setup is not the return operation under test.
    let mut warm_connections = Vec::new();
    for _ in 0..4 {
        warm_connections.push(pool.acquire().await.unwrap());
    }
    drop(warm_connections);
    let manager = Arc::new(InvocationRangeManager::default());
    let first = issue_range_fixture_id(&manager, &pool, "worker-fence").await;
    let mut blocker = pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *blocker)
        .await
        .unwrap();

    manager.test_retire(&pool, "worker-fence");
    tokio::time::sleep(Duration::from_millis(140)).await;
    // The return budget has elapsed, but BEGIN is still executing in SQLite's
    // worker. Replacement must wait on the generation fence, not recover early.
    let started = std::time::Instant::now();
    assert!(manager.allocate(&pool, Some("worker-fence")).await.is_err());
    assert!(started.elapsed() < Duration::from_millis(250));
    assert_eq!(
        manager.test_resize(chrono::Utc::now().timestamp_millis()).1,
        1
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *blocker)
        .await
        .unwrap();
    drop(blocker);
    tokio::time::timeout(Duration::from_secs(2), async {
        while manager.test_resize(chrono::Utc::now().timestamp_millis()).1 != 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();

    let replacement = issue_range_fixture_id(&manager, &pool, "worker-fence").await;
    assert_eq!(&first[..6], &replacement[..6]);
    assert_eq!(
        &replacement[6..],
        crate::encode_prompt_cache_conversation_sequence(64).unwrap()
    );
    let next = manager.allocate(&pool, Some("worker-fence")).await.unwrap();
    assert_eq!(
        &next[6..],
        crate::encode_prompt_cache_conversation_sequence(65).unwrap()
    );
    let ceiling: i64 = sqlx::query_scalar("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='worker-fence'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        ceiling, 127,
        "old completion cannot mutate the replacement reservation"
    );
    pool.close().await;
    crate::tests::cleanup_temp_test_dir(&temp_dir);
}

// Setup may meet the allocator's permitted cold timeout under parallel disk
// pressure. Retry only that outcome before/after the measured return race;
// the single allocation under the write lock keeps its original deadline.
async fn issue_range_fixture_id(
    manager: &std::sync::Arc<
        crate::prompt_cache_conversations::invocation_ranges::InvocationRangeManager,
    >,
    pool: &sqlx::SqlitePool,
    key: &str,
) -> String {
    for attempt in 1..=5 {
        match manager.allocate(pool, Some(key)).await {
            Ok(id) => return id,
            Err(error)
                if attempt < 5
                    && error.to_string() == "invocation range allocation timed out after 100ms" =>
            {
                eprintln!("range fixture cold admission timed out on attempt {attempt}");
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => panic!("range fixture admission failed on attempt {attempt}: {error}"),
        }
    }
    unreachable!("the last fixture attempt returns or fails")
}

#[tokio::test]
async fn maintenance_file_write_lock_does_not_block_business_database_or_publish_control() {
    let temp_dir = crate::tests::make_temp_test_dir("prompt-cache-control-maintenance-lock");
    let business_path = temp_dir.join("business.sqlite");
    let maintenance_path = temp_dir.join("maintenance.sqlite");
    let business_pool = open_file_pool(&business_path, Duration::from_millis(100)).await;
    let maintenance_pool = open_file_pool(&maintenance_path, Duration::from_millis(100)).await;
    let store = crate::maintenance_store::MaintenanceStore::from_pool(maintenance_pool.clone());
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance schema");

    let task_key = "startup_backfill.prompt_cache_conversations_materialization";
    let task_name = "prompt_cache_conversations_materialization_v1";
    sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key=?")
        .bind(task_key)
        .execute(&maintenance_pool)
        .await
        .expect("enable the synthetic maintenance task");
    store
        .initialize_prompt_cache_materialization_control(task_key, task_name)
        .await
        .expect("initialize committed prompt-cache control");
    assert!(
        store
            .prompt_cache_materialization_control
            .snapshot()
            .expect("read initial control snapshot")
            .enabled
    );

    sqlx::query("CREATE TABLE business_writes (id INTEGER PRIMARY KEY)")
        .execute(&business_pool)
        .await
        .expect("create business-side write fixture");

    let mut lock_connection = maintenance_pool
        .acquire()
        .await
        .expect("acquire maintenance lock connection");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *lock_connection)
        .await
        .expect("hold a real maintenance-database write lock");

    let business_write = tokio::time::timeout(
        Duration::from_millis(500),
        sqlx::query("INSERT INTO business_writes DEFAULT VALUES").execute(&business_pool),
    )
    .await
    .expect("business write should not wait for the maintenance database")
    .expect("business write should succeed");
    assert_eq!(business_write.rows_affected(), 1);

    let failed_control_update = tokio::time::timeout(
        Duration::from_secs(1),
        store.update_prompt_cache_materialization_control(task_key, task_name, false),
    )
    .await
    .expect("control update should fail within its configured SQLite busy timeout");
    assert!(failed_control_update.is_err());
    assert!(
        store
            .prompt_cache_materialization_control
            .snapshot()
            .expect("failed transaction must retain trusted control")
            .enabled,
    );
    let committed_enabled: bool =
        sqlx::query_scalar("SELECT enabled FROM managed_tasks WHERE task_key=?")
            .bind(task_key)
            .fetch_one(&maintenance_pool)
            .await
            .expect("read committed maintenance control after failed update");
    assert!(committed_enabled);

    sqlx::query("ROLLBACK")
        .execute(&mut *lock_connection)
        .await
        .expect("release maintenance database write lock");
    drop(lock_connection);
    business_pool.close().await;
    maintenance_pool.close().await;
    crate::tests::cleanup_temp_test_dir(&temp_dir);
}
