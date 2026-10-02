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
    assert_eq!(
        store
            .prompt_cache_materialization_control
            .snapshot()
            .expect("failed transaction must retain trusted control")
            .enabled,
        true
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
