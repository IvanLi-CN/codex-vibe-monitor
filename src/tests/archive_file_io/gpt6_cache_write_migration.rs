use super::*;

#[tokio::test]
async fn concurrent_reported_cache_write_column_migration_is_database_serialized() {
    let temp_dir = make_temp_test_dir("reported-cache-write-concurrent-migration");
    let db_path = temp_dir.join("state.db");
    let db_url = test_sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(
        &db_url,
        std::time::Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .expect("build migration sqlite options");
    let pool_a = SqlitePoolOptions::new()
        .connect_with(connect_options.clone())
        .await
        .expect("open first migration pool");
    let pool_b = SqlitePoolOptions::new()
        .connect_with(connect_options)
        .await
        .expect("open second migration pool");
    let legacy_create_sql = codex_invocations_create_sql("codex_invocations")
        .replace("            reported_cache_write_tokens INTEGER,\n", "");
    sqlx::query(&legacy_create_sql)
        .execute(&pool_a)
        .await
        .expect("create legacy invocation schema");

    let (result_a, result_b) = tokio::join!(
        ensure_reported_cache_write_tokens_column(&pool_a),
        ensure_reported_cache_write_tokens_column(&pool_b),
    );

    result_a.expect("first concurrent migration should succeed");
    result_b.expect("second concurrent migration should succeed");
    let column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('codex_invocations') WHERE name = 'reported_cache_write_tokens'",
    )
    .fetch_one(&pool_a)
    .await
    .expect("inspect migrated invocation schema");
    assert_eq!(column_count, 1);

    pool_a.close().await;
    pool_b.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archives_into_legacy_archive_batch_with_raw_expires_at_column() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-legacy-archive").await;
    let occurred_at = shanghai_local_days_ago(91, 9, 0, 0);
    let month_key = occurred_at[..7].to_string();
    let final_archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve legacy archive path");
    fs::create_dir_all(
        final_archive_path
            .parent()
            .expect("legacy archive path should have parent"),
    )
    .expect("create legacy archive dir");

    let legacy_archive_db_path = temp_dir.join("legacy-archive.sqlite");
    fs::File::create(&legacy_archive_db_path).expect("create legacy archive sqlite file");
    let legacy_archive_pool =
        SqlitePool::connect(&test_sqlite_url_for_path(&legacy_archive_db_path))
            .await
            .expect("open legacy archive sqlite");
    let legacy_create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL
        .replace("archive_db.", "")
        .replace("    first_token_ms REAL,\n", "")
        .replace("    reported_cache_write_tokens INTEGER,\n", "");
    sqlx::query(&legacy_create_sql)
        .execute(&legacy_archive_pool)
        .await
        .expect("create legacy archive schema baseline");
    sqlx::query("ALTER TABLE codex_invocations ADD COLUMN raw_expires_at TEXT")
        .execute(&legacy_archive_pool)
        .await
        .expect("add legacy raw_expires_at column");
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&legacy_archive_pool)
        .await
        .expect("checkpoint legacy archive sqlite before compression");
    legacy_archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&legacy_archive_db_path, &final_archive_path)
        .expect("compress legacy archive batch");

    insert_retention_invocation(
        &pool,
        "archive-into-legacy-batch",
        &occurred_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"error\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sqlx::query(
        "UPDATE codex_invocations SET reported_cache_write_tokens = 123 WHERE invoke_id = ?1",
    )
    .bind("archive-into-legacy-batch")
    .execute(&pool)
    .await
    .expect("set exact cache-write count before archiving");

    let live_row_id: i64 = sqlx::query_scalar(
        "SELECT id FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind("archive-into-legacy-batch")
    .bind(&occurred_at)
    .fetch_one(&pool)
    .await
    .expect("load live invocation row id");
    let archive_outcome = archive_rows_into_month_batch(
        &pool,
        &config,
        archive_table_spec("codex_invocations"),
        &month_key,
        &[live_row_id],
    )
    .await
    .expect("append into legacy archive batch");
    assert!(
        archive_outcome.row_count >= 1,
        "legacy archive batch should accept appended rows with legacy schema (row_count={})",
        archive_outcome.row_count
    );

    let inflated_legacy_path = temp_dir.join("legacy-archive-inflated.sqlite");
    inflate_gzip_sqlite_file(&final_archive_path, &inflated_legacy_path)
        .expect("inflate retained legacy archive batch");
    let archived_pool = SqlitePool::connect(&test_sqlite_url_for_path(&inflated_legacy_path))
        .await
        .expect("open retained legacy archive batch");
    let archived_ids: HashSet<String> =
        sqlx::query_scalar("SELECT invoke_id FROM codex_invocations")
            .fetch_all(&archived_pool)
            .await
            .expect("load legacy archive invoke ids")
            .into_iter()
            .collect();
    assert!(archived_ids.contains("archive-into-legacy-batch"));
    let archive_columns: HashSet<String> = sqlx::query("PRAGMA table_info('codex_invocations')")
        .fetch_all(&archived_pool)
        .await
        .expect("inspect retained legacy archive schema")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(
        archive_columns.contains("raw_expires_at"),
        "historical archive files should keep their legacy schema"
    );
    assert!(
        archive_columns.contains("first_token_ms"),
        "append should upgrade legacy archives with nullable TTFT storage"
    );
    assert!(
        archive_columns.contains("reported_cache_write_tokens"),
        "append should upgrade legacy archives with nullable exact cache-write storage"
    );
    let archived_first_token_ms: Option<f64> =
        sqlx::query_scalar("SELECT first_token_ms FROM codex_invocations WHERE invoke_id = ?1")
            .bind("archive-into-legacy-batch")
            .fetch_one(&archived_pool)
            .await
            .expect("load archived TTFT");
    assert_eq!(archived_first_token_ms, None);
    let archived_reported_cache_write_tokens: Option<i64> = sqlx::query_scalar(
        "SELECT reported_cache_write_tokens FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("archive-into-legacy-batch")
    .fetch_one(&archived_pool)
    .await
    .expect("load archived exact cache-write count");
    assert_eq!(archived_reported_cache_write_tokens, Some(123));
    archived_pool.close().await;

    cleanup_temp_test_dir(&temp_dir);
}
