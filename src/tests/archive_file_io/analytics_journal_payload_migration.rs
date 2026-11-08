use super::*;
use crate::schema::analytics_changes::ensure_analytics_changes_schema;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wal_file_journal_migration_restarts_idempotently_with_complete_payload() {
    let directory = make_temp_test_dir("analytics-journal-payload-migration");
    let database = directory.join("analytics.sqlite");
    let options = SqliteConnectOptions::new()
        .filename(&database)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .expect("open WAL Journal fixture");
    sqlx::query(
        "CREATE TABLE analytics_change_events (
            change_id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_version INTEGER NOT NULL,
            event_kind TEXT NOT NULL CHECK(event_kind IN ('upsert', 'tombstone', 'coverage')),
            entity_kind TEXT NOT NULL,
            source TEXT NOT NULL,
            source_row_id INTEGER,
            entity_key TEXT NOT NULL,
            generation INTEGER NOT NULL,
            coverage_revision INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            fact_json TEXT NOT NULL CHECK(length(fact_json) <= 65536),
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&pool)
    .await
    .expect("create WAL legacy Journal table");
    let fact_json = format!(r#"{{"payload":"{}"}}"#, "wal".repeat(20_000));
    sqlx::query(
        "INSERT INTO analytics_change_events (
            change_id, event_version, event_kind, entity_kind, source, source_row_id,
            entity_key, generation, coverage_revision, occurred_at, fact_json
        ) VALUES (17, 1, 'upsert', 'fixture', 'wal', 17, 'wal-17', 4, 8,
                  '2026-11-08T00:00:00Z', ?1)",
    )
    .bind(&fact_json)
    .execute(&pool)
    .await
    .expect("seed WAL legacy Journal row");

    ensure_analytics_changes_schema(&pool)
        .await
        .expect("migrate WAL Journal fixture");
    drop(pool);

    let reopened = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("reopen WAL Journal fixture");
    ensure_analytics_changes_schema(&reopened)
        .await
        .expect("reconcile Journal migration marker after restart");

    let stored_fact: String =
        sqlx::query_scalar("SELECT fact_json FROM analytics_change_events WHERE change_id = 17")
            .fetch_one(&reopened)
            .await
            .expect("load restarted WAL Journal row");
    assert_eq!(stored_fact, fact_json);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM analytics_change_journal_migrations
             WHERE migration_id = 'analytics-change-events-complete-inline-payload-v1'",
        )
        .fetch_one(&reopened)
        .await
        .expect("count idempotent WAL migration marker"),
        1
    );
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&reopened)
        .await
        .expect("read restarted Journal mode");
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

    drop(reopened);
    fs::remove_dir_all(directory).expect("remove WAL Journal fixture");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wal_file_journal_migration_refuses_interrupted_rebuild_residue() {
    let directory = make_temp_test_dir("analytics-journal-payload-interrupted");
    let database = directory.join("analytics.sqlite");
    let options = SqliteConnectOptions::new()
        .filename(&database)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open interrupted WAL Journal fixture");

    sqlx::query(
        "CREATE TABLE analytics_change_events (
            change_id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_version INTEGER NOT NULL,
            event_kind TEXT NOT NULL CHECK(event_kind IN ('upsert', 'tombstone', 'coverage')),
            entity_kind TEXT NOT NULL,
            source TEXT NOT NULL,
            source_row_id INTEGER,
            entity_key TEXT NOT NULL,
            generation INTEGER NOT NULL,
            coverage_revision INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            fact_json TEXT NOT NULL CHECK(length(fact_json) <= 65536),
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&pool)
    .await
    .expect("create interrupted WAL legacy Journal table");
    sqlx::query(
        "INSERT INTO analytics_change_events (
            change_id, event_version, event_kind, entity_kind, source, entity_key,
            generation, coverage_revision, occurred_at, fact_json
        ) VALUES (9, 1, 'upsert', 'fixture', 'wal', 'interrupted', 1, 0,
                  '2026-11-08T00:00:00Z', '{\"payload\":\"original\"}')",
    )
    .execute(&pool)
    .await
    .expect("seed interrupted WAL legacy Journal row");
    sqlx::query("CREATE TABLE analytics_change_events__journal_payload_v1 (change_id INTEGER)")
        .execute(&pool)
        .await
        .expect("seed interrupted Journal rebuild residue");

    let error = ensure_analytics_changes_schema(&pool)
        .await
        .expect_err("leftover rebuild residue must fail closed");
    assert!(
        error
            .to_string()
            .contains("leftover analytics Journal rebuild table")
    );
    let stored_fact: String =
        sqlx::query_scalar("SELECT fact_json FROM analytics_change_events WHERE change_id = 9")
            .fetch_one(&pool)
            .await
            .expect("load source row after interrupted migration");
    assert_eq!(stored_fact, r#"{"payload":"original"}"#);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'analytics_change_journal_migrations'",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect marker after interrupted migration"),
        0
    );

    drop(pool);
    fs::remove_dir_all(directory).expect("remove interrupted WAL Journal fixture");
}
