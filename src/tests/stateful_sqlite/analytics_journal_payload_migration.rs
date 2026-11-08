use super::*;
use crate::schema::analytics_changes::ensure_analytics_changes_schema;

const LEGACY_EVENTS_TABLE_SQL: &str = r#"
    CREATE TABLE analytics_change_events (
        change_id INTEGER PRIMARY KEY AUTOINCREMENT,
        event_version INTEGER NOT NULL,
        fact_date_semantics_version INTEGER NOT NULL DEFAULT 1,
        source_instance_id TEXT NOT NULL DEFAULT 'legacy-sqlite',
        event_id TEXT NOT NULL DEFAULT '',
        operation_kind TEXT NOT NULL DEFAULT 'legacy',
        fact_semantics_version INTEGER NOT NULL DEFAULT 1,
        stats_semantics_version INTEGER NOT NULL DEFAULT 1,
        payload_digest TEXT NOT NULL DEFAULT '',
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
    )
    "#;

async fn legacy_journal_pool() -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect legacy Journal migration fixture")
}

async fn seed_legacy_journal(pool: &SqlitePool, with_row: bool) {
    sqlx::query(LEGACY_EVENTS_TABLE_SQL)
        .execute(pool)
        .await
        .expect("create legacy analytics Journal table");
    if with_row {
        let fact_json = format!(r#"{{"payload":"{}"}}"#, "x".repeat(60_000));
        sqlx::query(
            "INSERT INTO analytics_change_events (
                change_id, event_version, fact_date_semantics_version,
                source_instance_id, event_id, operation_kind, fact_semantics_version,
                stats_semantics_version, payload_digest, event_kind, entity_kind, source,
                source_row_id, entity_key, generation, coverage_revision, occurred_at,
                fact_json
            ) VALUES (42, 1, 2, 'sqlite-a', 'event-42', 'upsert', 1, 2,
                      'digest-42', 'upsert', 'invocation', 'proxy', 7, 'entity-42',
                      3, 9, '2026-11-08T00:00:00Z', ?1)",
        )
        .bind(fact_json)
        .execute(pool)
        .await
        .expect("seed legacy Journal row");
        sqlx::query("UPDATE sqlite_sequence SET seq = 9000 WHERE name = 'analytics_change_events'")
            .execute(pool)
            .await
            .expect("raise legacy Journal sequence high-water mark");
    } else {
        sqlx::query(
            "INSERT INTO analytics_change_events (
                change_id, event_version, event_kind, entity_kind, source,
                entity_key, generation, coverage_revision, occurred_at, fact_json
            ) VALUES (3, 1, 'upsert', 'fixture', 'test', 'empty-sequence', 1, 0,
                      '2026-11-08T00:00:00Z', '{}')",
        )
        .execute(pool)
        .await
        .expect("seed then remove legacy Journal sequence row");
        sqlx::query("DELETE FROM analytics_change_events")
            .execute(pool)
            .await
            .expect("empty legacy Journal table");
        sqlx::query("UPDATE sqlite_sequence SET seq = 9000 WHERE name = 'analytics_change_events'")
            .execute(pool)
            .await
            .expect("raise empty legacy Journal sequence high-water mark");
    }
}

async fn seed_existing_projection_state(pool: &SqlitePool) {
    sqlx::query(
        "CREATE TABLE analytics_change_state (
            id INTEGER PRIMARY KEY CHECK(id = 1),
            generation INTEGER NOT NULL DEFAULT 1,
            coverage_revision INTEGER NOT NULL DEFAULT 0,
            retained_from_change_id INTEGER NOT NULL DEFAULT 1,
            capacity INTEGER NOT NULL DEFAULT 10000 CHECK(capacity > 0),
            invalidation_pending INTEGER NOT NULL DEFAULT 0 CHECK(invalidation_pending IN (0, 1)),
            historical_replay_required INTEGER NOT NULL DEFAULT 0 CHECK(historical_replay_required IN (0, 1)),
            historical_history_unknown INTEGER NOT NULL DEFAULT 0 CHECK(historical_history_unknown IN (0, 1)),
            historical_history_initialized INTEGER NOT NULL DEFAULT 0 CHECK(historical_history_initialized IN (0, 1)),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(pool)
    .await
    .expect("create existing analytics Journal state");
    sqlx::query(
        "INSERT INTO analytics_change_state (
            id, generation, coverage_revision, retained_from_change_id, capacity,
            historical_history_initialized
        ) VALUES (1, 7, 11, 123, 321, 1)",
    )
    .execute(pool)
    .await
    .expect("seed existing analytics Journal state");
    sqlx::query(
        "CREATE TABLE analytics_change_coverage (
            replica_id TEXT PRIMARY KEY,
            generation INTEGER NOT NULL,
            covered_through_change_id INTEGER NOT NULL,
            coverage_revision INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(pool)
    .await
    .expect("create existing analytics Journal coverage");
    sqlx::query(
        "INSERT INTO analytics_change_coverage (
            replica_id, generation, covered_through_change_id, coverage_revision
        ) VALUES ('replica-a', 7, 122, 11)",
    )
    .execute(pool)
    .await
    .expect("seed existing analytics Journal coverage");
    sqlx::query(
        "CREATE TABLE analytics_source_snapshots (
            build_id TEXT PRIMARY KEY,
            source_instance_id TEXT NOT NULL,
            generation INTEGER NOT NULL,
            high_watermark_change_id INTEGER NOT NULL CHECK(high_watermark_change_id >= 0),
            coverage_revision INTEGER NOT NULL CHECK(coverage_revision >= 0),
            retained_from_change_id INTEGER NOT NULL CHECK(retained_from_change_id >= 0),
            fact_semantics_version INTEGER NOT NULL,
            stats_semantics_version INTEGER NOT NULL,
            manifest_digest TEXT,
            state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active', 'completed', 'failed', 'expired')),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(pool)
    .await
    .expect("create existing analytics source snapshot");
    sqlx::query(
        "INSERT INTO analytics_source_snapshots (
            build_id, source_instance_id, generation, high_watermark_change_id,
            coverage_revision, retained_from_change_id, fact_semantics_version,
            stats_semantics_version, manifest_digest, state
        ) VALUES ('build-a', 'sqlite-a', 7, 122, 11, 123, 1, 2, 'manifest-a', 'active')",
    )
    .execute(pool)
    .await
    .expect("seed existing analytics source snapshot");
    sqlx::query(
        "CREATE TABLE analytics_source_retention_pins (
            pin_id TEXT PRIMARY KEY,
            build_id TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_identity TEXT NOT NULL,
            watermark_change_id INTEGER NOT NULL CHECK(watermark_change_id >= 0),
            expires_at TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'active' CHECK(state IN ('active', 'released', 'expired')),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY(build_id) REFERENCES analytics_source_snapshots(build_id)
        )",
    )
    .execute(pool)
    .await
    .expect("create existing analytics source retention pin");
    sqlx::query(
        "INSERT INTO analytics_source_retention_pins (
            pin_id, build_id, source_kind, source_identity, watermark_change_id,
            expires_at
        ) VALUES ('pin-a', 'build-a', 'journal', 'sqlite-a', 122, '2026-11-09T00:00:00Z')",
    )
    .execute(pool)
    .await
    .expect("seed existing analytics source retention pin");
}

#[tokio::test]
async fn forward_journal_migration_preserves_rows_state_and_large_inline_payloads() {
    let pool = legacy_journal_pool().await;
    seed_legacy_journal(&pool, true).await;
    seed_existing_projection_state(&pool).await;

    ensure_analytics_changes_schema(&pool)
        .await
        .expect("apply complete-inline-payload Journal migration");

    let table_sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'analytics_change_events'",
    )
    .fetch_one(&pool)
    .await
    .expect("load migrated Journal schema");
    assert!(!table_sql.contains("length(fact_json)"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM analytics_change_journal_migrations
             WHERE migration_id = 'analytics-change-events-complete-inline-payload-v1'",
        )
        .fetch_one(&pool)
        .await
        .expect("count Journal migration marker"),
        1
    );

    let (change_id, event_id, digest, event_kind, fact_json): (
        i64,
        String,
        String,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT change_id, event_id, payload_digest, event_kind, fact_json
             FROM analytics_change_events WHERE change_id = 42",
    )
    .fetch_one(&pool)
    .await
    .expect("load copied Journal row");
    assert_eq!(change_id, 42);
    assert_eq!(event_id, "event-42");
    assert_eq!(digest, "digest-42");
    assert_eq!(event_kind, "upsert");
    assert_eq!(fact_json.len(), 60_014);

    let state: (i64, i64, i64) = sqlx::query_as(
        "SELECT generation, coverage_revision, retained_from_change_id
         FROM analytics_change_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved Journal state");
    assert_eq!(state, (7, 11, 123));
    let coverage: (i64, i64, i64) = sqlx::query_as(
        "SELECT generation, covered_through_change_id, coverage_revision
         FROM analytics_change_coverage WHERE replica_id = 'replica-a'",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved Journal coverage");
    assert_eq!(coverage, (7, 122, 11));
    let pin: (String, String, i64) = sqlx::query_as(
        "SELECT pin_id, source_identity, watermark_change_id
         FROM analytics_source_retention_pins WHERE pin_id = 'pin-a'",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved Journal retention pin");
    assert_eq!(pin, ("pin-a".to_string(), "sqlite-a".to_string(), 122));

    let large_fact = format!(r#"{{"payload":"{}"}}"#, "z".repeat(1_100_000));
    sqlx::query(
        "INSERT INTO analytics_change_events (
            event_version, fact_date_semantics_version, source_instance_id, event_id,
            operation_kind, fact_semantics_version, stats_semantics_version, payload_digest,
            event_kind, entity_kind, source, entity_key, generation, coverage_revision,
            occurred_at, fact_json
        ) VALUES (1, 2, 'sqlite-a', 'large-event', 'upsert', 1, 2, 'large-digest',
                  'upsert', 'invocation', 'proxy', 'large', 3, 10,
                  '2026-11-08T00:00:01Z', ?1)",
    )
    .bind(&large_fact)
    .execute(&pool)
    .await
    .expect("store a complete payload larger than one MiB");
    let stored_bytes: i64 = sqlx::query_scalar(
        "SELECT length(CAST(fact_json AS BLOB))
         FROM analytics_change_events WHERE event_id = 'large-event'",
    )
    .fetch_one(&pool)
    .await
    .expect("measure complete inline payload bytes");
    assert!(stored_bytes > 1_048_576);
}

#[tokio::test]
async fn forward_journal_migration_preserves_empty_table_sequence_high_water_mark() {
    let pool = legacy_journal_pool().await;
    seed_legacy_journal(&pool, false).await;

    ensure_analytics_changes_schema(&pool)
        .await
        .expect("migrate empty Journal with historical sequence");

    let sequence: i64 = sqlx::query_scalar(
        "SELECT seq FROM sqlite_sequence WHERE name = 'analytics_change_events'",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved empty Journal sequence");
    assert_eq!(sequence, 9000);

    sqlx::query(
        "INSERT INTO analytics_change_events (
            event_version, fact_date_semantics_version, source_instance_id, event_id,
            operation_kind, fact_semantics_version, stats_semantics_version, payload_digest,
            event_kind, entity_kind, source, entity_key, generation, coverage_revision,
            occurred_at, fact_json
        ) VALUES (1, 2, 'sqlite-a', 'after-empty', 'upsert', 1, 2, 'digest',
                  'upsert', 'fixture', 'test', 'after-empty', 1, 1,
                  '2026-11-08T00:00:00Z', '{}')",
    )
    .execute(&pool)
    .await
    .expect("insert after preserving empty Journal sequence");
    let inserted_id: i64 = sqlx::query_scalar(
        "SELECT change_id FROM analytics_change_events WHERE event_id = 'after-empty'",
    )
    .fetch_one(&pool)
    .await
    .expect("load inserted Journal id");
    assert_eq!(inserted_id, 9001);
}

#[tokio::test]
async fn forward_journal_migration_rejects_unknown_schema_before_rewriting_it() {
    let pool = legacy_journal_pool().await;
    sqlx::query(
        "CREATE TABLE analytics_change_events (
            change_id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_version INTEGER NOT NULL,
            event_kind TEXT NOT NULL,
            entity_kind TEXT NOT NULL,
            source TEXT NOT NULL,
            entity_key TEXT NOT NULL,
            generation INTEGER NOT NULL,
            coverage_revision INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            fact_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            future_payload BLOB
        )",
    )
    .execute(&pool)
    .await
    .expect("create unknown Journal schema");

    let error = ensure_analytics_changes_schema(&pool)
        .await
        .expect_err("unknown Journal schema must fail closed");
    assert!(
        error
            .to_string()
            .contains("unsupported analytics Journal schema")
    );
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_table_info('analytics_change_events') ORDER BY cid",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect untouched unknown Journal schema");
    assert!(columns.iter().any(|column| column == "future_payload"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'analytics_change_journal_migrations'",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect migration marker after rejected schema"),
        0
    );
}
