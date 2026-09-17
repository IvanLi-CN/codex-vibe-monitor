use super::*;
use serde_json::json;

async fn seed_startup_prep_manifest_account(pool: &SqlitePool, account_id: i64, created_at: &str) {
    sqlx::query(
        "INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(account_id)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Startup prep manifest account")
    .bind("active")
    .bind(1_i64)
    .bind(created_at)
    .bind(created_at)
    .execute(pool)
    .await
    .expect("insert startup prep manifest account");
}

async fn assert_one_archive_backfill_account_remains_pending(pool: &SqlitePool) {
    let remaining_pending: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE last_activity_at IS NULL
          AND last_activity_archive_backfill_completed = 0
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("count remaining archive backfill accounts");
    assert_eq!(remaining_pending, 1);
}

async fn seed_duplicate_account_archive(
    config: &AppConfig,
    temp_dir: &Path,
    primary_account_id: i64,
    secondary_account_id: i64,
) -> (PathBuf, String, String, String) {
    let base_occurred_at = parse_shanghai_local_naive(&shanghai_local_days_ago(120, 8, 0, 0))
        .expect("valid shanghai local");
    let month_key = format_naive(base_occurred_at)[..7].to_string();
    let archive_path = archive_batch_file_path(config, "codex_invocations", &month_key)
        .expect("resolve archive manifest refresh path");
    fs::create_dir_all(archive_path.parent().expect("archive parent"))
        .expect("create archive manifest refresh parent");

    let archive_db_path = temp_dir.join("archive-manifest-refresh-dedupe.sqlite");
    fs::File::create(&archive_db_path).expect("create archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create archive schema");

    let repeated_rows = BACKFILL_ACCOUNT_BIND_BATCH_SIZE + 5;
    let mut primary_latest = String::new();
    let mut secondary_latest = String::new();
    for idx in 0..repeated_rows {
        let occurred_at = format_naive(base_occurred_at + ChronoDuration::seconds(idx as i64));
        primary_latest = occurred_at.clone();
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, raw_response, created_at, payload
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(idx as i64 + 1)
        .bind(format!("manifest-refresh-primary-{idx}"))
        .bind(&occurred_at)
        .bind("{}")
        .bind(&occurred_at)
        .bind(json!({ "upstreamAccountId": primary_account_id }).to_string())
        .execute(&archive_pool)
        .await
        .expect("insert repeated primary manifest row");
    }
    for idx in 0..2 {
        let occurred_at = format_naive(
            base_occurred_at + ChronoDuration::seconds(repeated_rows as i64 + idx as i64 + 1),
        );
        secondary_latest = occurred_at.clone();
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, raw_response, created_at, payload
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(repeated_rows as i64 + idx as i64 + 1)
        .bind(format!("manifest-refresh-secondary-{idx}"))
        .bind(&occurred_at)
        .bind("{}")
        .bind(&occurred_at)
        .bind(json!({ "upstreamAccountId": secondary_account_id }).to_string())
        .execute(&archive_pool)
        .await
        .expect("insert repeated secondary manifest row");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress manifest refresh archive");
    (archive_path, month_key, primary_latest, secondary_latest)
}

async fn seed_expired_invocation_archive_fixture(
    pool: &SqlitePool,
    _config: &AppConfig,
    temp_dir: &Path,
) -> (PathBuf, String) {
    let archive_path = temp_dir.join("expired-archive.sqlite.gz");
    let archive_db_path = temp_dir.join("expired-archive.sqlite");
    fs::File::create(&archive_db_path).expect("create expired archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open expired invocation archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create expired invocation archive schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, raw_response, t_total_ms, created_at
        )
        VALUES (1, 'expired-long-wall-time', '2025-01-01 23:00:00', '{}', ?1, '2025-01-01 23:00:00')
        "#,
    )
    .bind(48.0_f64 * 60.0 * 60.0 * 1000.0)
    .execute(&archive_pool)
    .await
    .expect("insert multi-day expired invocation archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress expired invocation archive");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash expired invocation archive");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id, dataset, month_key, file_path, sha256, row_count, status,
            coverage_start_at, coverage_end_at, historical_rollups_materialized_at, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), ?10)
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind("2025-01")
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind("2025-01-01 00:00:00")
    .bind("2025-01-01 00:00:00")
    .bind("2025-01-01 00:00:00")
    .execute(pool)
    .await
    .expect("insert expired invocation archive batch");
    sqlx::query(
        "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) VALUES (?1, ?2, ?3)",
    )
    .bind(1_i64)
    .bind(7_i64)
    .bind("2025-01-01 00:00:00")
    .execute(pool)
    .await
    .expect("insert expired invocation archive manifest row");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(pool)
        .await
        .expect("mark long-term stats ready for cleanup fixture");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .execute(pool)
    .await
    .expect("mark long-term archive replay complete");
    insert_summary_archive_snapshot_proof(
        pool,
        1,
        &archive_sha256,
        "2025-01-01 00:00:00",
        "2025-01-01 00:00:00",
        1,
    )
    .await;
    (archive_path, archive_sha256)
}

async fn assert_expired_invocation_archive_cleanup(
    pool: &SqlitePool,
    config: &AppConfig,
    archive_path: &Path,
) {
    let deleted = cleanup_expired_archive_batches(pool, config, false)
        .await
        .expect("cleanup expired invocation archive batches");
    assert_eq!(deleted, 1);
    assert!(!archive_path.exists());
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(pool)
        .await
        .expect("count remaining archive batches");
    assert_eq!(remaining_batches, 0);
    let remaining_manifest_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM archive_batch_upstream_activity")
            .fetch_one(pool)
            .await
            .expect("count remaining archive manifest rows");
    assert_eq!(remaining_manifest_rows, 0);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await
    .expect("load long-term integrity source boundary");
    assert_eq!(
        integrity_source_start.as_deref(),
        Some("2025-01-04"),
        "archive cleanup must exclude every Shanghai day touched by a multi-day wall-time interval"
    );
}

async fn assert_startup_prep_backfilled_account(
    pool: &SqlitePool,
    account_id: i64,
    occurred_at: &str,
) {
    let backfill = backfill_upstream_account_last_activity_from_archives(pool, None, None)
        .await
        .expect("backfill upstream activity after startup prep");
    assert!(!backfill.waiting_for_manifest_backfill);
    assert_eq!(backfill.updated_accounts, 1);
    let row = sqlx::query_as::<_, (Option<String>, i64)>(
        "SELECT last_activity_at, last_activity_archive_backfill_completed
         FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load startup prep account row");
    assert_eq!(row.0.as_deref(), Some(occurred_at));
    assert_eq!(row.1, 1);
}

async fn assert_cleanup_metadata_transaction_rolled_back(
    pool: &SqlitePool,
    config: &AppConfig,
    archive_path: &Path,
) {
    cleanup_expired_archive_batches(pool, config, false)
        .await
        .expect_err("metadata transaction failure should abort archive cleanup");
    assert!(
        archive_path.exists(),
        "the readable archive source must remain when its metadata transaction rolls back"
    );
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(pool)
        .await
        .expect("count retained archive batch");
    assert_eq!(remaining_batches, 1);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await
    .expect("load rolled-back long-term integrity source boundary");
    assert!(
        integrity_source_start.is_none(),
        "a rolled-back metadata transaction must not publish a source boundary"
    );
}

pub(crate) async fn insert_summary_archive_snapshot_proof(
    pool: &SqlitePool,
    archive_batch_id: i64,
    manifest_sha256: &str,
    coverage_start: &str,
    coverage_end: &str,
    row_count: u32,
) {
    let normalized = serde_json::to_vec(
        &(0..row_count)
            .map(|offset| SummaryArchiveSnapshotV2Record {
                id: i64::from(offset),
                invoke_id: format!("snapshot-proof-{offset}"),
                occurred_at: coverage_start.to_string(),
                source: "proxy".to_string(),
                model: None,
                response_model: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_input_tokens: 0,
                reasoning_tokens: 0,
                reasoning_effort: None,
                total_tokens: 0,
                cost: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                status: "success".to_string(),
                error_message: None,
                failure_kind: None,
                failure_class: None,
                is_actionable: false,
                upstream_account_id: None,
            })
            .collect::<Vec<_>>(),
    )
    .expect("serialize Summary Snapshot proof records");
    let page = SummaryArchiveSnapshotPage {
        archive_batch_id,
        manifest_sha256: manifest_sha256.to_string(),
        page_index: 0,
        coverage_start: coverage_start.to_string(),
        coverage_end: coverage_end.to_string(),
        row_count,
        payload: zstd::stream::encode_all(normalized.as_slice(), 1)
            .expect("compress Summary Snapshot proof records"),
    };
    let mut tx = pool
        .begin()
        .await
        .expect("begin Summary Snapshot proof transaction");
    store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &page)
        .await
        .expect("store Summary Snapshot proof");
    tx.commit().await.expect("commit Summary Snapshot proof");
    ensure_summary_archive_snapshot_v2_final_proof(pool, archive_batch_id, manifest_sha256)
        .await
        .expect("commit Summary Snapshot final proof");
}

struct ExpiredAttemptArchiveFixture {
    pool: SqlitePool,
    config: AppConfig,
    temp_dir: PathBuf,
    archive_path: PathBuf,
    archive_sha256: String,
    occurred_at: &'static str,
    create_sql: String,
}

async fn create_expired_attempt_archive_fixture() -> ExpiredAttemptArchiveFixture {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("attempt-archive-long-term-boundary").await;
    let occurred_at = "2025-01-01 23:00:00";
    insert_retention_invocation(
        &pool,
        "attempt-archive-long-term-boundary",
        occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{}"),
        "{}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sqlx::query("UPDATE codex_invocations SET t_total_ms = ?1 WHERE invoke_id = ?2")
        .bind(48.0_f64 * 60.0 * 60.0 * 1000.0)
        .bind("attempt-archive-long-term-boundary")
        .execute(&pool)
        .await
        .expect("add multi-day timing to matching live invocation");
    let archive_path = temp_dir.join("expired-attempt-archive.sqlite.gz");
    let archive_db_path = temp_dir.join("expired-attempt-archive.sqlite");
    fs::File::create(&archive_db_path).expect("create expired attempt archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open expired attempt archive sqlite");
    let create_sql = POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create expired attempt archive schema");
    sqlx::query(
        "INSERT INTO pool_upstream_request_attempts
         (id, invoke_id, occurred_at, endpoint, route_mode, upstream_account_id,
          attempt_index, distinct_account_index, same_account_retry_index, status, created_at)
         VALUES (1, ?1, ?2, '/v1/responses', 'pool', 7, 0, 0, 0, 'succeeded', ?2)",
    )
    .bind("attempt-archive-long-term-boundary")
    .bind(occurred_at)
    .execute(&archive_pool)
    .await
    .expect("insert attempt account mapping");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress expired attempt archive");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash expired attempt archive");
    ExpiredAttemptArchiveFixture {
        pool,
        config,
        temp_dir,
        archive_path,
        archive_sha256,
        occurred_at,
        create_sql,
    }
}

async fn register_expired_attempt_archive(fixture: &ExpiredAttemptArchiveFixture) {
    sqlx::query(
        "INSERT INTO archive_batches
         (id, dataset, month_key, file_path, sha256, row_count, status, coverage_start_at,
          coverage_end_at, archive_expires_at, historical_rollups_materialized_at, created_at)
         VALUES (1, 'pool_upstream_request_attempts', '2025-01', ?1, ?4, 1, ?2, ?3, ?3,
                 '2025-01-02 00:00:00', datetime('now'), ?3)",
    )
    .bind(fixture.archive_path.to_string_lossy().to_string())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(fixture.occurred_at)
    .bind(&fixture.archive_sha256)
    .execute(&fixture.pool)
    .await
    .expect("insert expired attempt archive manifest");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&fixture.pool)
        .await
        .expect("mark long-term stats ready for attempt cleanup");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
        LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay
             (target, dataset, file_path, archive_sha256)
             VALUES (?1, 'pool_upstream_request_attempts', ?2, ?3)",
        )
        .bind(target)
        .bind(fixture.archive_path.to_string_lossy().to_string())
        .bind(&fixture.archive_sha256)
        .execute(&fixture.pool)
        .await
        .expect("mark attempt archive replay complete");
    }
}

#[tokio::test]
pub(crate) async fn legacy_summary_snapshot_backfill_materializes_v2_before_raw_source_loss() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("summary-snapshot-v2-backfill").await;
    let archive_db_path = temp_dir.join("legacy-summary-v2-backfill.sqlite");
    let archive_path = temp_dir.join("legacy-summary-v2-backfill.sqlite.gz");
    fs::File::create(&archive_db_path).expect("create legacy Summary archive sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open legacy Summary archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create legacy Summary archive schema");
    let occurred_at = "2026-08-01 00:00:00";
    sqlx::query(
        "INSERT INTO codex_invocations
         (id, invoke_id, occurred_at, raw_response, created_at, payload)
         VALUES (1, 'legacy-summary-v2-row', ?1, '{}', ?1, NULL)",
    )
    .bind(occurred_at)
    .execute(&archive_pool)
    .await
    .expect("insert legacy Summary archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress legacy Summary archive");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash legacy Summary archive");
    sqlx::query(
        "INSERT INTO archive_batches
         (id, dataset, month_key, file_path, sha256, row_count, status,
          summary_source_kind, coverage_start_at, coverage_end_at)
         VALUES (1, 'codex_invocations', '2026-08', ?1, ?2, 1, 'completed',
                 'unknown', ?3, ?3)",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .bind(occurred_at)
    .execute(&pool)
    .await
    .expect("seed legacy Summary archive manifest");

    let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(5))
        .await
        .expect("backfill legacy Summary archive Snapshot V2");
    assert_eq!(result.materialized_archive_batches, 1);
    assert!(
        summary_archive_snapshot_has_proof(&pool, 1, &archive_sha256)
            .await
            .expect("validate backfilled V2 proof")
    );
    let (snapshot_format, snapshot_payload): (i64, Vec<u8>) = sqlx::query_as(
        "SELECT format_version, payload FROM summary_archive_snapshot
         WHERE archive_batch_id = 1 AND manifest_sha256 = ?1",
    )
    .bind(&archive_sha256)
    .fetch_one(&pool)
    .await
    .expect("load backfilled Snapshot format");
    assert_eq!(snapshot_format, SUMMARY_ARCHIVE_SNAPSHOT_V2);
    let snapshot_records = decode_summary_archive_snapshot_v2_payload(&snapshot_payload)
        .expect("decode backfilled Snapshot payload");
    assert_eq!(snapshot_records[0].invoke_id, "legacy-summary-v2-row");

    fs::remove_file(&archive_path).expect("remove legacy raw source after V2 proof");
    let second = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("reuse V2 authority after raw source loss");
    assert_eq!(second.materialized_archive_batches, 0);
    assert_eq!(second.unavailable_archive_batches, 0);
    assert!(
        summary_archive_snapshot_has_proof(&pool, 1, &archive_sha256)
            .await
            .expect("retain V2 proof after raw source loss")
    );
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn summary_snapshot_v2_backfill_uses_occurred_at_id_order() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("summary-snapshot-v2-time-order").await;
    let archive_db_path = temp_dir.join("legacy-summary-v2-time-order.sqlite");
    let archive_path = temp_dir.join("legacy-summary-v2-time-order.sqlite.gz");
    fs::File::create(&archive_db_path).expect("create legacy Summary archive sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open legacy Summary archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create legacy Summary archive schema");

    // The first ID is deliberately the newest event. An ID-ordered pager puts it in the first
    // page ahead of older rows and cannot satisfy the V2 chronological proof across pages.
    let row_count = 401_i64;
    let base = parse_shanghai_local_naive("2026-08-01 00:00:00").expect("valid base timestamp");
    for id in 1..=row_count {
        let offset = if id == 1 { row_count } else { id - 2 };
        let occurred_at = format_naive(base + ChronoDuration::seconds(offset));
        sqlx::query(
            "INSERT INTO codex_invocations
             (id, invoke_id, occurred_at, raw_response, created_at, payload)
             VALUES (?1, ?2, ?3, '{}', ?3, NULL)",
        )
        .bind(id)
        .bind(format!("summary-time-order-{id}"))
        .bind(&occurred_at)
        .execute(&archive_pool)
        .await
        .expect("insert legacy Summary archive row");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress legacy Summary archive");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash legacy Summary archive");
    let coverage_start = format_naive(base);
    let coverage_end = format_naive(base + ChronoDuration::seconds(row_count));
    sqlx::query(
        "INSERT INTO archive_batches
         (id, dataset, month_key, file_path, sha256, row_count, status,
          summary_source_kind, coverage_start_at, coverage_end_at)
         VALUES (1, 'codex_invocations', '2026-08', ?1, ?2, ?3, 'completed',
                 'unknown', ?4, ?5)",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .bind(row_count)
    .bind(&coverage_start)
    .bind(&coverage_end)
    .execute(&pool)
    .await
    .expect("seed legacy Summary archive manifest");

    let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(5))
        .await
        .expect("backfill legacy Summary archive Snapshot V2");
    assert_eq!(result.materialized_archive_batches, 1);
    assert!(
        summary_archive_snapshot_has_proof(&pool, 1, &archive_sha256)
            .await
            .expect("validate chronological V2 proof")
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn summary_snapshot_backfill_reactivates_after_manifest_sha_change() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("summary-snapshot-v2-sha-reactivation").await;
    let archive_path = temp_dir.join("summary-snapshot-v2-sha-reactivation.sqlite.gz");
    fs::write(&archive_path, b"not-a-sqlite-archive").expect("write invalid archive fixture");
    let initial_sha = sha256_hex_file(&archive_path).expect("hash archive fixture");
    let stale_sha = format!("{initial_sha}-stale");
    sqlx::query(
        "INSERT INTO archive_batches
         (id, dataset, month_key, file_path, sha256, row_count, status,
          summary_source_kind, coverage_start_at, coverage_end_at)
         VALUES (1, 'codex_invocations', '2026-08', ?1, ?2, 1, 'completed',
                 'unknown', '2026-08-01 00:00:00', '2026-08-01 01:00:00')",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&stale_sha)
    .execute(&pool)
    .await
    .expect("seed stale Summary archive manifest");

    let first = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("run stale SHA backfill");
    assert_eq!(first.unavailable_archive_batches, 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT failure_kind FROM summary_archive_snapshot_backfill_outcome
             WHERE archive_batch_id = 1 AND manifest_sha256 = ?1",
        )
        .bind(&stale_sha)
        .fetch_one(&pool)
        .await
        .expect("load stale SHA outcome"),
        "manifest_sha_mismatch"
    );

    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE id = 1")
        .bind(&initial_sha)
        .execute(&pool)
        .await
        .expect("update manifest SHA");
    let second = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("run reactivated SHA backfill");
    assert_eq!(second.unavailable_archive_batches, 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT failure_kind FROM summary_archive_snapshot_backfill_outcome
             WHERE archive_batch_id = 1 AND manifest_sha256 = ?1",
        )
        .bind(&initial_sha)
        .fetch_one(&pool)
        .await
        .expect("load reactivated SHA outcome"),
        "source_open_failed"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn archive_manifest_refresh_dedupes_duplicate_account_rows_from_archive_file() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-refresh-dedupe").await;
    let primary_account_id = 996_i64;
    let secondary_account_id = 997_i64;
    let created_at = format_utc_iso(Utc::now());
    for (account_id, display_name) in [
        (primary_account_id, "Manifest duplicate primary"),
        (secondary_account_id, "Manifest duplicate secondary"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, status, enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(account_id)
        .bind("api_key_codex")
        .bind("codex")
        .bind(display_name)
        .bind("active")
        .bind(1_i64)
        .bind(&created_at)
        .bind(&created_at)
        .execute(&pool)
        .await
        .expect("insert manifest refresh account");
    }

    let (archive_path, month_key, primary_latest, secondary_latest) =
        seed_duplicate_account_archive(
            &config,
            &temp_dir,
            primary_account_id,
            secondary_account_id,
        )
        .await;

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_path).expect("archive sha"))
    .bind((BACKFILL_ACCOUNT_BIND_BATCH_SIZE + 7) as i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(shanghai_local_days_ago(120, 8, 0, 0))
    .bind(secondary_latest.clone())
    .execute(&pool)
    .await
    .expect("insert manifest refresh batch");

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("refresh manifest rows for duplicate accounts");
    assert_eq!(refresh.pending_batches, 1);
    assert_eq!(refresh.refreshed_batches, 1);
    assert_eq!(refresh.account_rows_written, 2);

    let manifest_rows = sqlx::query_as::<_, (i64, String)>(
        r#"
        SELECT account_id, last_activity_at
        FROM archive_batch_upstream_activity
        ORDER BY account_id ASC
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load refreshed manifest rows");
    assert_eq!(
        manifest_rows,
        vec![
            (primary_account_id, primary_latest),
            (secondary_account_id, secondary_latest),
        ]
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn archive_manifest_refresh_respects_the_retention_candidate_budget() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-refresh-budget").await;
    config.retention_batch_rows = 1;
    let created_at = format_utc_iso(Utc::now());

    for id in 1_i64..=3 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status, created_at
            )
            VALUES (?1, 'codex_invocations', '2026-01', ?2, 'missing', 1, ?3, ?4)
            "#,
        )
        .bind(id)
        .bind(
            temp_dir
                .join(format!("missing-manifest-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(&created_at)
        .execute(&pool)
        .await
        .expect("insert missing manifest candidate");
    }

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("refresh bounded manifest candidates");
    let pending_after_refresh: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE upstream_activity_manifest_refreshed_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count pending manifest candidates");
    assert_eq!(refresh.pending_batches, 1);
    assert_eq!(refresh.missing_files, 1);
    assert_eq!(pending_after_refresh, 3);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn archive_manifest_activity_replacement_clears_and_writes_in_micro_batches() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-activity-micro-batches").await;
    config.retention_batch_rows = 2;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id, dataset, month_key, file_path, sha256, row_count, status, created_at
        )
        VALUES (99, 'codex_invocations', '2026-01', 'memory://manifest-activity', 'manifest', 5, ?1, ?2)
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert archive manifest batch");
    for account_id in 10_i64..15 {
        sqlx::query(
            "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) VALUES (99, ?1, ?2)",
        )
        .bind(account_id)
        .bind("2026-01-01 00:00:00")
        .execute(&pool)
        .await
        .expect("seed stale manifest activity");
    }

    let next_values = (20_i64..25)
        .map(|account_id| (account_id, format!("2026-01-01 00:00:{account_id:02}")))
        .collect::<Vec<_>>();
    let written = replace_archive_batch_upstream_activity_in_micro_batches(
        &pool,
        &config,
        99,
        &next_values,
        0,
    )
    .await
    .expect("replace manifest activity in micro batches");
    let actual = sqlx::query_as::<_, (i64, String)>(
        "SELECT account_id, last_activity_at FROM archive_batch_upstream_activity WHERE archive_batch_id = 99 ORDER BY account_id ASC",
    )
    .fetch_all(&pool)
    .await
    .expect("load rebuilt manifest activity");
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT upstream_activity_manifest_refreshed_at FROM archive_batches WHERE id = 99",
    )
    .fetch_one(&pool)
    .await
    .expect("load manifest refresh marker");
    assert_eq!(written, Some(5));
    assert_eq!(actual, next_values);
    assert!(marker.is_some());

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn startup_persistent_prep_skips_mutations_for_dry_run_commands() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("startup-prep-dry-run").await;
    let occurred_at = shanghai_local_days_ago(45, 9, 0, 0);
    let month_key = occurred_at[..7].to_string();

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&month_key)
    .bind(
        temp_dir
            .join("pending-manifest.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert pending manifest batch");

    let cli = CliArgs {
        command: Some(CliCommand::Maintenance(MaintenanceCliArgs {
            command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: true }),
        })),
        ..Default::default()
    };

    let summary = run_startup_persistent_prep(&pool, &config, &cli)
        .await
        .expect("run startup prep for dry-run maintenance command");
    assert_eq!(summary.refreshed_manifest_batches, 0);
    assert_eq!(summary.backfilled_archive_expiries, 0);
    assert!(!summary.bootstrapped_hourly_rollups);

    let refreshed_at: Option<String> = sqlx::query_scalar(
        "SELECT upstream_activity_manifest_refreshed_at FROM archive_batches WHERE dataset = 'codex_invocations'",
    )
    .fetch_one(&pool)
    .await
    .expect("load manifest refreshed flag");
    assert!(refreshed_at.is_none());

    let archive_expires_at: Option<String> = sqlx::query_scalar(
        "SELECT archive_expires_at FROM archive_batches WHERE dataset = 'codex_invocations'",
    )
    .fetch_one(&pool)
    .await
    .expect("load archive expiry");
    assert!(archive_expires_at.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn startup_persistent_prep_keeps_manifest_backlog_pending_after_a_bounded_pass() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("startup-prep-bounded-manifest-backlog").await;
    config.retention_batch_rows = 1;
    let occurred_at = shanghai_local_days_ago(45, 9, 0, 0);

    for (index, month_key) in ["2025-01", "2025-02"].into_iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                coverage_start_at,
                coverage_end_at,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
            "#,
        )
        .bind("codex_invocations")
        .bind(month_key)
        .bind(
            temp_dir
                .join(format!("pending-manifest-{index}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(format!("deadbeef-{index}"))
        .bind(1_i64)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(&occurred_at)
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("insert pending manifest batch");
    }

    let summary = run_startup_persistent_prep_inner(&pool, &config, &CliArgs::default(), false)
        .await
        .expect("run bounded startup prep");

    assert_eq!(summary.refreshed_manifest_batches, 0);
    assert_eq!(summary.pending_manifest_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn startup_rollup_bootstrap_runs_in_background_only_for_normal_server_start() {
    let default_cli = CliArgs::default();
    assert!(should_spawn_background_startup_hourly_rollup_bootstrap(
        &default_cli
    ));
    assert!(!should_run_blocking_startup_persistent_prep(&default_cli));

    let retention_cli = CliArgs {
        retention_run_once: true,
        ..CliArgs::default()
    };
    assert!(!should_spawn_background_startup_hourly_rollup_bootstrap(
        &retention_cli
    ));
    assert!(should_run_blocking_startup_persistent_prep(&retention_cli));

    let maintenance_cli = CliArgs {
        command: Some(CliCommand::Maintenance(MaintenanceCliArgs {
            command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: false }),
        })),
        ..CliArgs::default()
    };
    assert!(!should_spawn_background_startup_hourly_rollup_bootstrap(
        &maintenance_cli
    ));
    assert!(!should_run_blocking_startup_persistent_prep(
        &maintenance_cli
    ));
}

#[tokio::test]
pub(crate) async fn startup_persistent_prep_rebuilds_manifest_before_archive_backfill() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("startup-prep-manifest").await;
    let account_id = 992_i64;
    let created_at = format_utc_iso(Utc::now());
    seed_startup_prep_manifest_account(&pool, account_id, &created_at).await;

    let occurred_at = shanghai_local_days_ago(90, 10, 15, 0);
    let month_key = occurred_at[..7].to_string();
    let archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve startup prep archive path");
    fs::create_dir_all(archive_path.parent().expect("archive parent"))
        .expect("create startup prep archive parent");
    let archive_db_path = temp_dir.join("startup-prep-manifest.sqlite");
    fs::File::create(&archive_db_path).expect("create startup prep sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open startup prep sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create startup prep archive schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, raw_response, created_at, payload
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(1_i64)
    .bind("startup-prep-manifest-row")
    .bind(&occurred_at)
    .bind("{}")
    .bind(&occurred_at)
    .bind(json!({ "upstreamAccountId": account_id }).to_string())
    .execute(&archive_pool)
    .await
    .expect("insert startup prep archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress startup prep archive");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_path).expect("startup prep archive sha"))
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert startup prep batch");

    let summary = run_startup_persistent_prep(&pool, &config, &CliArgs::default())
        .await
        .expect("run startup persistent prep");
    assert_eq!(summary.refreshed_manifest_batches, 1);
    assert_eq!(summary.refreshed_manifest_account_rows, 1);
    assert_eq!(summary.missing_manifest_files, 0);
    assert!(summary.bootstrapped_hourly_rollups);

    assert_startup_prep_backfilled_account(&pool, account_id, &occurred_at).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn archive_backfill_respects_scan_limit_budget() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("archive-backfill-budget").await;
    let created_at = format_utc_iso(Utc::now());
    for account_id in [993_i64, 994_i64] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, status, enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(account_id)
        .bind("api_key_codex")
        .bind("codex")
        .bind(format!("Archive budget account {account_id}"))
        .bind("active")
        .bind(1_i64)
        .bind(&created_at)
        .bind(&created_at)
        .execute(&pool)
        .await
        .expect("insert archive budget account");
    }

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            upstream_activity_manifest_refreshed_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'), datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind("2025-01")
    .bind(
        temp_dir
            .join("budget.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .bind("deadbeef")
    .bind(2_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("insert archive budget batch");

    let first_activity_at = shanghai_local_days_ago(20, 8, 0, 0);
    let second_activity_at = shanghai_local_days_ago(19, 9, 0, 0);
    sqlx::query(
        r#"
        INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at)
        VALUES (?1, ?2, ?3), (?4, ?5, ?6)
        "#,
    )
    .bind(1_i64)
    .bind(993_i64)
    .bind(&first_activity_at)
    .bind(1_i64)
    .bind(994_i64)
    .bind(&second_activity_at)
    .execute(&pool)
    .await
    .expect("insert archive budget manifest rows");

    let first_pass = backfill_upstream_account_last_activity_from_archives(
        &pool,
        Some(1),
        Some(Duration::from_secs(60)),
    )
    .await
    .expect("run first archive budget pass");
    assert_eq!(first_pass.updated_accounts, 1);
    assert!(first_pass.hit_budget);

    assert_one_archive_backfill_account_remains_pending(&pool).await;

    let second_pass = backfill_upstream_account_last_activity_from_archives(
        &pool,
        Some(1),
        Some(Duration::from_secs(60)),
    )
    .await
    .expect("run second archive budget pass");
    assert_eq!(second_pass.updated_accounts, 1);
    assert!(!second_pass.hit_budget);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn cleanup_expired_invocation_archive_batches_removes_manifest_rows() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-ttl-cleanup").await;
    config.invocation_archive_ttl_days = 0;

    let (archive_path, _archive_sha256) =
        seed_expired_invocation_archive_fixture(&pool, &config, &temp_dir).await;

    assert_expired_invocation_archive_cleanup(&pool, &config, &archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn cleanup_expired_archive_keeps_a_missing_materialized_invocation_manifest_after_source_loss()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("cleanup-missing-invocation-manifest-finalize").await;
    let missing_archive_path = temp_dir.join("missing-finalizable-manifest.sqlite.gz");
    let coverage_end_at =
        shanghai_local_days_ago((config.invocation_max_days + 30) as i64, 9, 0, 0);

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '2000-01-01 00:00:00', datetime('now'), datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&coverage_end_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("missing-cleanup-sha")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert missing replayed invocation archive metadata");
    sqlx::query(
        "UPDATE long_term_stats_state SET status = 'error', last_error = 'terminal integrity proof reconciliation is incomplete' WHERE id = 1",
    )
        .execute(&pool)
        .await
        .expect("mark long-term statistics unavailable after source loss");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("missing-cleanup-sha")
    .execute(&pool)
    .await
    .expect("insert long-term replay marker");

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("retain missing source manifest during expiry cleanup");
    assert_eq!(deleted, 0);
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count retained archive batches");
    assert_eq!(remaining_batches, 1);
    let remaining_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .fetch_one(&pool)
    .await
    .expect("count retained replay markers");
    assert_eq!(remaining_markers, 1);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved long-term source boundary");
    assert!(integrity_source_start.is_none());
    let status: String =
        sqlx::query_scalar("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("load durable source-loss status");
    assert_eq!(status, "error");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn cleanup_expired_invocation_archive_requires_every_source_timestamp_to_be_parseable()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-ttl-cleanup-unparseable-source").await;
    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "archive-ttl-cleanup-unparseable-source",
        &[
            (
                1_i64,
                "archive-ttl-cleanup-parseable-source",
                "2025-01-01 09:00:00",
                SOURCE_PROXY,
                "success",
                42_i64,
                0.42_f64,
                Some(120.0),
            ),
            (
                2_i64,
                "archive-ttl-cleanup-unparseable-source",
                "invalid-timestamp",
                SOURCE_PROXY,
                "success",
                42_i64,
                0.42_f64,
                Some(120.0),
            ),
        ],
    )
    .await;
    let archive_sha256: String =
        sqlx::query_scalar("SELECT sha256 FROM archive_batches WHERE file_path = ?1")
            .bind(archive_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load archive checksum");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET coverage_start_at = '2025-01-01 00:00:00',
            coverage_end_at = '2025-01-01 00:00:00',
            archive_expires_at = '2000-01-01 00:00:00',
            historical_rollups_materialized_at = datetime('now')
        WHERE file_path = ?1
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("mark unparseable archive eligible for cleanup");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats ready for cleanup fixture");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(archive_sha256)
    .execute(&pool)
    .await
    .expect("mark long-term archive replay complete");

    let boundary =
        crate::long_term_stats::long_term_integrity_source_safe_start_for_archive_cleanup(
            &pool,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_path.to_string_lossy(),
            Some("2025-01-01 00:00:00"),
        )
        .await;
    assert!(
        boundary.is_err(),
        "manifest coverage must not substitute for an unparseable source timestamp"
    );
    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("unparseable source should retain the archive instead of failing cleanup");
    assert_eq!(deleted, 0);
    assert!(archive_path.exists());
    let cleanup_state: String =
        sqlx::query_scalar("SELECT cleanup_state FROM archive_batches WHERE file_path = ?1")
            .bind(archive_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load retained archive cleanup state");
    assert_eq!(cleanup_state, ARCHIVE_CLEANUP_STATE_ACTIVE);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved source boundary");
    assert!(integrity_source_start.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn cleanup_expired_invocation_archive_preserves_source_when_metadata_transaction_fails()
 {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-ttl-cleanup-transaction-failure").await;
    config.invocation_archive_ttl_days = 0;
    let occurred_at = "2025-01-01 09:00:00";
    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "archive-ttl-cleanup-transaction-failure",
        &[(
            1_i64,
            "archive-ttl-cleanup-transaction-failure",
            occurred_at,
            SOURCE_PROXY,
            "success",
            42_i64,
            0.42_f64,
            Some(120.0),
        )],
    )
    .await;
    let archive_sha256: String =
        sqlx::query_scalar("SELECT sha256 FROM archive_batches WHERE file_path = ?1")
            .bind(archive_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load archive checksum");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET coverage_start_at = ?1,
            coverage_end_at = ?1,
            archive_expires_at = '2025-01-02 00:00:00',
            historical_rollups_materialized_at = datetime('now')
        WHERE file_path = ?2
        "#,
    )
    .bind(occurred_at)
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("mark archive eligible for cleanup");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats ready for cleanup fixture");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .execute(&pool)
    .await
    .expect("mark long-term archive replay complete");
    let archive_batch_id: i64 =
        sqlx::query_scalar("SELECT id FROM archive_batches WHERE file_path = ?1")
            .bind(archive_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load archive batch id for Snapshot proof");
    insert_summary_archive_snapshot_proof(
        &pool,
        archive_batch_id,
        &archive_sha256,
        occurred_at,
        occurred_at,
        1,
    )
    .await;
    sqlx::query(
        r#"
        CREATE TRIGGER reject_archive_cleanup
        BEFORE UPDATE OF cleanup_state ON archive_batches
        WHEN NEW.cleanup_state = 'delete_pending'
        BEGIN
            SELECT RAISE(ABORT, 'forced archive metadata transaction failure');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("install archive cleanup failure trigger");

    assert_cleanup_metadata_transaction_rolled_back(&pool, &config, &archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn cleanup_expired_archive_retries_pending_file_deletion_without_advancing_source_boundary_early()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-ttl-cleanup-pending-delete").await;
    let pending_path = temp_dir.join("pending-delete-directory");
    fs::create_dir_all(&pending_path).expect("create pending-delete directory");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            cleanup_state,
            cleanup_source_safe_start_date,
            archive_expires_at,
            created_at
        )
        VALUES ('codex_quota_snapshots', '2025-01', ?1, 'pending-delete-sha', 1, ?2, 'delete_pending', '2025-01-04', '2000-01-01 00:00:00', datetime('now'))
        "#,
    )
    .bind(pending_path.to_string_lossy().to_string())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("insert pending archive deletion record");

    let first_attempt = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("failed file deletion should remain retryable");
    assert_eq!(first_attempt, 0);
    let pending_state: (String, String) =
        sqlx::query_as("SELECT status, cleanup_state FROM archive_batches WHERE file_path = ?1")
            .bind(pending_path.to_string_lossy().to_string())
            .fetch_one(&pool)
            .await
            .expect("load retained pending deletion record");
    assert_eq!(pending_state.0, ARCHIVE_STATUS_COMPLETED);
    assert_eq!(pending_state.1, ARCHIVE_CLEANUP_STATE_DELETE_PENDING);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load source boundary after failed pending cleanup");
    assert!(
        integrity_source_start.is_none(),
        "a failed pending file cleanup must not publish its staged source boundary"
    );

    fs::remove_dir(&pending_path).expect("remove directory that blocks file deletion");
    fs::write(&pending_path, b"retryable archive file").expect("restore removable archive file");
    let retryable_sha256 = sha256_hex_file(&pending_path).expect("hash retryable archive file");
    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
        .bind(retryable_sha256)
        .bind(pending_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("update retryable archive identity");
    let second_attempt = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("pending archive deletion should retry successfully");
    assert_eq!(second_attempt, 1);
    assert!(!pending_path.exists());
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count retired pending deletion record");
    assert_eq!(remaining_batches, 0);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load source boundary after finalized pending cleanup");
    assert_eq!(integrity_source_start.as_deref(), Some("2025-01-04"));

    cleanup_temp_test_dir(&temp_dir);
}

async fn assert_unverifiable_attempt_archive_is_retained(
    pool: &SqlitePool,
    config: &AppConfig,
    temp_dir: &Path,
    occurred_at: &str,
    create_sql: &str,
) {
    let unverifiable_archive_path = temp_dir.join("unverifiable-attempt-archive.sqlite.gz");
    let unverifiable_archive_db_path = temp_dir.join("unverifiable-attempt-archive.sqlite");
    fs::File::create(&unverifiable_archive_db_path)
        .expect("create unverifiable attempt archive sqlite file");
    let archive_pool =
        SqlitePool::connect(&test_sqlite_url_for_path(&unverifiable_archive_db_path))
            .await
            .expect("open unverifiable attempt archive sqlite");
    sqlx::query(create_sql)
        .execute(&archive_pool)
        .await
        .expect("create unverifiable attempt archive schema");
    sqlx::query(
    r#"
    INSERT INTO pool_upstream_request_attempts (
        id,
        invoke_id,
        occurred_at,
        endpoint,
        route_mode,
        upstream_account_id,
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        status,
        created_at
    )
    VALUES (1, 'missing-invocation-source', ?1, '/v1/responses', 'pool', 7, 0, 0, 0, 'succeeded', ?1)
    "#,
)
.bind(occurred_at)
.execute(&archive_pool)
.await
.expect("insert unverifiable attempt account mapping");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&unverifiable_archive_db_path, &unverifiable_archive_path)
        .expect("compress unverifiable attempt archive");
    sqlx::query(
    r#"
    INSERT INTO archive_batches (
        id,
        dataset,
        month_key,
        file_path,
        sha256,
        row_count,
        status,
        coverage_start_at,
        coverage_end_at,
        archive_expires_at,
        historical_rollups_materialized_at,
        created_at
    )
    VALUES (2, 'pool_upstream_request_attempts', '2025-01', ?1, 'unverifiable-attempt-sha', 1, ?2, ?3, ?3, '2025-01-02 00:00:00', datetime('now'), ?3)
    "#,
)
.bind(unverifiable_archive_path.to_string_lossy().to_string())
.bind(ARCHIVE_STATUS_COMPLETED)
    .bind(occurred_at)
    .execute(pool)
.await
.expect("insert unverifiable attempt archive manifest");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
        LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, 'unverifiable-attempt-sha')",
    )
    .bind(target)
    .bind(unverifiable_archive_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("mark unverifiable attempt archive replay complete");
    }

    let deleted = cleanup_expired_archive_batches(pool, config, false)
        .await
        .expect("attempt cleanup should retain unverifiable mapping");
    assert_eq!(deleted, 0);
    assert!(unverifiable_archive_path.exists());
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(pool)
        .await
        .expect("count retained unverifiable archive batch");
    assert_eq!(remaining_batches, 1);
}

#[tokio::test]
pub(crate) async fn cleanup_expired_attempt_archive_preserves_the_long_term_source_boundary_and_retains_unverifiable_mapping()
 {
    let fixture = create_expired_attempt_archive_fixture().await;
    register_expired_attempt_archive(&fixture).await;
    let pool = &fixture.pool;
    let config = &fixture.config;
    let temp_dir = &fixture.temp_dir;
    let archive_path = &fixture.archive_path;
    let occurred_at = fixture.occurred_at;
    let create_sql = &fixture.create_sql;

    let verified_safe_start =
        crate::long_term_stats::long_term_integrity_source_safe_start_for_archive_cleanup(
            pool,
            "pool_upstream_request_attempts",
            &archive_path.to_string_lossy(),
            Some(occurred_at),
        )
        .await
        .expect("resolve the live invocation for the archived attempt mapping");
    assert_eq!(
        verified_safe_start,
        Some(NaiveDate::from_ymd_opt(2025, 1, 4).expect("fixed safe date"))
    );

    let deleted = cleanup_expired_archive_batches(pool, config, false)
        .await
        .expect("cleanup expired attempt archive");
    assert_eq!(deleted, 1);
    assert!(!archive_path.exists());
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await
    .expect("load long-term integrity source boundary after attempt cleanup");
    assert_eq!(integrity_source_start.as_deref(), Some("2025-01-04"));

    assert_unverifiable_attempt_archive_is_retained(
        pool,
        config,
        temp_dir,
        occurred_at,
        create_sql,
    )
    .await;

    cleanup_temp_test_dir(temp_dir);
}
