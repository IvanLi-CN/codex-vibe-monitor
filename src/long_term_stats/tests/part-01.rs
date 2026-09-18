#[tokio::test]
async fn initial_refresh_retries_after_missing_attempt_archive_recovers() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive();
    let occurred_at = format!("{date}T10:00:00+08:00");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, raw_response, total_tokens, output_tokens, cost, created_at) VALUES (1, 'initial-missing-attempt-source', ?1, 'success', 'gpt-5', '{}', '{}', 100, 40, 0.1, datetime('now'))",
        )
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("insert live invocation");
    let missing_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-initial-missing-attempt-source-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    let missing_path = missing_path.to_string_lossy().to_string();
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, created_at
            )
            VALUES ('pool_upstream_request_attempts', ?1, ?2, 'initial-missing-attempt-sha',
                1, 'completed', ?3, ?3, datetime('now'))
            "#,
    )
    .bind(date.format("%Y-%m").to_string())
    .bind(&missing_path)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("record missing attempt archive manifest");

    let error = refresh_long_term_stats(&pool, 400)
        .await
        .expect_err("missing initial archive keeps materialization retryable");
    assert!(
        error
            .to_string()
            .contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR)
    );
    let initial_state = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("initial archive failure state");
    assert_eq!(initial_state.0, LONG_TERM_STATUS_ERROR);
    assert!(
        initial_state
            .1
            .as_deref()
            .is_some_and(|message| message.contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR))
    );

    sqlx::query("DELETE FROM archive_batches WHERE file_path = ?1")
        .bind(&missing_path)
        .execute(&pool)
        .await
        .expect("remove recovered archive manifest");
    refresh_long_term_stats(&pool, 400)
        .await
        .expect("initial materialization retries after archive recovery");
    let recovered_state = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("recovered initial state");
    assert_eq!(recovered_state.0, LONG_TERM_STATUS_READY);
    assert_eq!(recovered_state.1, None);
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
            )
            .bind(date.to_string())
            .fetch_one(&pool)
            .await
            .expect("recovered daily rollup"),
            1
        );
}

#[tokio::test]
async fn initial_refresh_replays_later_archives_after_an_earlier_source_recovers() {
    let (pool, date, occurred_at, missing_archive_path, readable_db_path, readable_archive_path) =
        setup_initial_replay_fixture().await;
    assert_initial_replay_pending(&pool).await;
    let restored_db_path =
        restore_initial_replay_archive(&pool, &missing_archive_path, &occurred_at).await;
    assert_recovered_initial_replay(&pool, date).await;

    for path in [
        readable_db_path,
        readable_archive_path,
        restored_db_path,
        missing_archive_path,
    ] {
        let _ = fs::remove_file(path);
    }
}

async fn setup_initial_replay_fixture() -> (
    Pool<Sqlite>,
    NaiveDate,
    String,
    std::path::PathBuf,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive();
    let occurred_at = format!("{date}T10:00:00+08:00");
    let suffix = format!(
        "{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let missing_archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-initial-missing-invocation-{suffix}.sqlite.gz"
    ));
    let readable_db_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-initial-readable-invocation-{suffix}.sqlite"
    ));
    let readable_archive_path = readable_db_path.with_extension("sqlite.gz");
    fs::File::create(&readable_db_path).expect("create readable invocation archive database");
    let archive_options = format!("sqlite://{}", readable_db_path.to_string_lossy())
        .parse::<SqliteConnectOptions>()
        .expect("parse readable invocation archive URL")
        .create_if_missing(true);
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(archive_options)
        .await
        .expect("open readable invocation archive database");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, invoke_id TEXT, occurred_at TEXT NOT NULL, status TEXT, model TEXT, total_tokens INTEGER, output_tokens INTEGER, cost REAL)",
        )
        .execute(&archive_pool)
        .await
        .expect("create readable invocation archive schema");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, total_tokens, output_tokens, cost) VALUES (2, 'later-readable-archive', ?1, 'success', 'gpt-5', 100, 40, 0.1)",
        )
        .bind(&occurred_at)
        .execute(&archive_pool)
        .await
        .expect("insert later readable invocation archive row");
    archive_pool.close().await;
    crate::maintenance::deflate_sqlite_file_to_gzip(&readable_db_path, &readable_archive_path)
        .expect("compress readable invocation archive");
    let readable_archive_sha256 = crate::maintenance::sha256_hex_file(&readable_archive_path)
        .expect("hash readable invocation archive");

    for (id, file_path, sha256) in [
        (
            1_i64,
            missing_archive_path.to_string_lossy().to_string(),
            "missing-invocation-sha".to_string(),
        ),
        (
            2_i64,
            readable_archive_path.to_string_lossy().to_string(),
            readable_archive_sha256,
        ),
    ] {
        sqlx::query(
                r#"
                INSERT INTO archive_batches (
                    id, dataset, month_key, file_path, sha256, row_count, status,
                    coverage_start_at, coverage_end_at, created_at
                )
                VALUES (?1, 'codex_invocations', ?2, ?3, ?4, 1, 'completed', ?5, ?5, datetime('now'))
                "#,
            )
            .bind(id)
            .bind(date.format("%Y-%m").to_string())
            .bind(file_path)
            .bind(sha256)
            .bind(&occurred_at)
            .execute(&pool)
            .await
            .expect("record invocation archive manifest");
    }

    (
        pool,
        date,
        occurred_at,
        missing_archive_path,
        readable_db_path,
        readable_archive_path,
    )
}

async fn assert_initial_replay_pending(pool: &Pool<Sqlite>) {
    refresh_long_term_stats(pool, 400)
        .await
        .expect("unreadable initial archive leaves a retryable state");
    let initial_state = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT status, last_error, statistics_start_date FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(pool)
    .await
    .expect("initial archive failure state");
    assert_eq!(initial_state.0, LONG_TERM_STATUS_ERROR);
    assert_eq!(
        initial_state.1.as_deref(),
        Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
    );
    assert!(
        initial_state.2.is_some(),
        "later readable archive reached a provisional start date"
    );
}

async fn restore_initial_replay_archive(
    pool: &Pool<Sqlite>,
    missing_archive_path: &std::path::Path,
    occurred_at: &str,
) -> std::path::PathBuf {
    let restored_db_path = missing_archive_path.with_extension("sqlite");
    fs::File::create(&restored_db_path).expect("create restored invocation archive database");
    let restored_options = format!("sqlite://{}", restored_db_path.to_string_lossy())
        .parse::<SqliteConnectOptions>()
        .expect("parse restored invocation archive URL")
        .create_if_missing(true);
    let restored_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(restored_options)
        .await
        .expect("open restored invocation archive database");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, invoke_id TEXT, occurred_at TEXT NOT NULL, status TEXT, model TEXT, total_tokens INTEGER, output_tokens INTEGER, cost REAL)",
        )
        .execute(&restored_pool)
        .await
        .expect("create restored invocation archive schema");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, total_tokens, output_tokens, cost) VALUES (1, 'restored-earlier-archive', ?1, 'success', 'gpt-5', 100, 40, 0.1)",
        )
        .bind(occurred_at)
        .execute(&restored_pool)
        .await
        .expect("insert restored invocation archive row");
    restored_pool.close().await;
    crate::maintenance::deflate_sqlite_file_to_gzip(&restored_db_path, missing_archive_path)
        .expect("compress restored invocation archive");
    let restored_archive_sha256 = crate::maintenance::sha256_hex_file(missing_archive_path)
        .expect("hash restored invocation archive");
    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
        .bind(restored_archive_sha256)
        .bind(missing_archive_path.to_string_lossy().to_string())
        .execute(pool)
        .await
        .expect("restore invocation archive manifest identity");
    restored_db_path
}

async fn assert_recovered_initial_replay(pool: &Pool<Sqlite>, date: NaiveDate) {
    refresh_long_term_stats(pool, 400)
        .await
        .expect("initial recovery must replay every invocation archive");
    let recovered_state = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(pool)
    .await
    .expect("recovered initial state");
    assert_eq!(recovered_state.0, LONG_TERM_STATUS_READY);
    assert_eq!(recovered_state.1, None);
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
            )
            .bind(date.to_string())
            .fetch_one(pool)
            .await
            .expect("recovered daily rollup"),
            2,
            "the later archive must not be skipped because its first replay marker survived the failed pass"
        );
}

#[test]
fn repeated_terminal_defers_preserve_the_original_repair_deadline() {
    let now = Instant::now();
    let original = now + Duration::from_secs(30);
    assert_eq!(
        long_term_projection_repair_deadline(Some(original), now),
        original
    );
    let overdue = now - Duration::from_secs(1);
    assert_eq!(
        long_term_projection_repair_deadline(Some(overdue), now),
        overdue
    );
}

#[test]
fn terminal_projection_flush_does_not_share_the_repair_backoff() {
    assert!(long_term_projection_terminal_flush_due(true, false));
    assert!(long_term_projection_terminal_flush_due(false, true));
    assert!(!long_term_projection_terminal_flush_due(false, false));
}

async fn create_long_term_test_invocations(pool: &Pool<Sqlite>) {
    sqlx::query(
        r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'canonical',
                status TEXT,
                detail_level TEXT NOT NULL DEFAULT 'full',
                model TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_input_tokens INTEGER,
                reasoning_tokens INTEGER,
                total_tokens INTEGER,
                cost REAL,
                cost_input REAL,
                cost_cache_write REAL,
                cost_cache_read REAL,
                cost_output REAL,
                cost_reasoning REAL,
                error_message TEXT,
                failure_kind TEXT,
                failure_class TEXT,
                is_actionable INTEGER NOT NULL DEFAULT 0,
                payload TEXT,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                first_token_ms REAL,
                t_upstream_stream_ms REAL,
                t_resp_parse_ms REAL,
                t_persist_ms REAL
            )
            "#,
    )
    .execute(pool)
    .await
    .expect("invocation schema");
}

#[tokio::test]
async fn terminal_projection_seek_uses_terminal_id_index_after_pending_prefix() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    create_long_term_test_invocations(&pool).await;
    sqlx::query(
        r#"
            WITH RECURSIVE source(id) AS (
                VALUES(1)
                UNION ALL
                SELECT id + 1 FROM source WHERE id < 1025
            )
            INSERT INTO codex_invocations (id, occurred_at, status)
            SELECT
                id,
                printf('2026-07-%02d 00:00:00', (id % 28) + 1),
                CASE WHEN id = 1025 THEN 'success' ELSE 'running' END
            FROM source
            "#,
    )
    .execute(&pool)
    .await
    .expect("sparse terminal source rows");
    ensure_long_term_projection_source_indexes(&pool)
        .await
        .expect("projection source indexes");
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
            "EXPLAIN QUERY PLAN SELECT id FROM codex_invocations WHERE id > ?1 AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') ORDER BY id ASC LIMIT ?2",
        )
        .bind(0_i64)
        .bind(LONG_TERM_PROJECTION_MAX_EVENTS_PER_FLUSH)
        .fetch_all(&pool)
        .await
        .expect("terminal seek query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_codex_invocations_long_term_projection_terminal_id")
            && detail.contains("id>?")
    }));
    let rows = load_long_term_projection_terminal_rows(&pool, 0)
        .await
        .expect("terminal projection seek");
    assert_eq!(
        rows.into_iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![1025]
    );
}

async fn create_long_term_integrity_oracle(pool: &Pool<Sqlite>) {
    sqlx::query(
            r#"
            CREATE TABLE invocation_rollup_hourly (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                total_count INTEGER NOT NULL,
                success_count INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                terminal_count INTEGER NOT NULL,
                terminal_tokens INTEGER NOT NULL,
                terminal_cost REAL NOT NULL,
                terminal_proof_complete INTEGER NOT NULL DEFAULT 1,
                total_tokens INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cache_input_tokens INTEGER NOT NULL DEFAULT 0,
                reasoning_tokens INTEGER NOT NULL DEFAULT 0,
                total_cost REAL NOT NULL,
                non_success_cost REAL NOT NULL DEFAULT 0,
                total_latency_sample_count INTEGER NOT NULL DEFAULT 0,
                total_latency_sum_ms REAL NOT NULL DEFAULT 0,
                first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
                first_byte_sum_ms REAL NOT NULL DEFAULT 0,
                first_byte_max_ms REAL NOT NULL DEFAULT 0,
                first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
                first_response_byte_total_sample_count INTEGER NOT NULL DEFAULT 0,
                first_response_byte_total_sum_ms REAL NOT NULL DEFAULT 0,
                first_response_byte_total_max_ms REAL NOT NULL DEFAULT 0,
                first_response_byte_total_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
                first_token_sample_count INTEGER NOT NULL DEFAULT 0,
                first_token_sum_ms REAL NOT NULL DEFAULT 0,
                first_token_max_ms REAL NOT NULL DEFAULT 0,
                first_token_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (bucket_start_epoch, source)
            )
            "#,
        )
        .execute(pool)
        .await
        .expect("hourly integrity oracle schema");
}

async fn long_term_file_backed_pool_with_busy_timeout(
    prefix: &str,
    busy_timeout: Duration,
) -> (Pool<Sqlite>, String, PathBuf) {
    let db_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-{prefix}-{}-{}.sqlite",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    fs::File::create(&db_path).expect("create sqlite test database");
    let db_url = format!("sqlite://{}", db_path.to_string_lossy());
    let options = db_url
        .parse::<SqliteConnectOptions>()
        .expect("parse sqlite test url")
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(busy_timeout);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .expect("connect sqlite test pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    (pool, db_url, db_path)
}

async fn long_term_file_backed_pool(prefix: &str) -> (Pool<Sqlite>, String, PathBuf) {
    long_term_file_backed_pool_with_busy_timeout(prefix, Duration::from_millis(50)).await
}

async fn cleanup_long_term_file_backed_pool(pool: Pool<Sqlite>, db_path: PathBuf) {
    pool.close().await;
    for suffix in ["", "-shm", "-wal"] {
        let path = PathBuf::from(format!("{}{}", db_path.display(), suffix));
        let _ = fs::remove_file(path);
    }
}

async fn insert_long_term_test_invocation(pool: &Pool<Sqlite>, id: i64, occurred_at: String) {
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost) VALUES (?1, ?2, ?3, 'success', 'gpt-5', '{}', 60, 40, 0, 0, 100, 0.1)",
        )
        .bind(id)
        .bind(format!("invoke-{id}"))
        .bind(occurred_at)
        .execute(pool)
        .await
        .expect("source invocation");
}

async fn seed_long_term_integrity_case(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
    source_rows: i64,
) -> (i64, i64) {
    let (day_start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    for offset in 0..source_rows {
        let hour = 10 + offset;
        sqlx::query(
                "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, model, payload, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost) VALUES (?1, ?2, ?3, ?4, 'success', 'gpt-5', '{\"reasoningEffort\":\"high\"}', 60, 40, 0, 0, 100, 0.1)",
            )
            .bind(offset + 1)
            .bind(format!("invoke-{}", offset + 1))
            .bind(format!("{date}T{:02}:00:00+08:00", hour))
            .bind(format!("canonical-{}", offset + 1))
            .execute(pool)
            .await
            .expect("source invocation");
    }
    for offset in 0..2_i64 {
        let hour_epoch = day_start_epoch + (10 + offset) * 60 * 60;
        sqlx::query(
                "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, ?2, 1, 1, 100, 0.1, 100, 0.1)",
            )
            .bind(hour_epoch)
            .bind(format!("canonical-{}", offset + 1))
            .execute(pool)
            .await
            .expect("canonical hourly rollup");
    }
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(pool)
        .await
        .expect("corrupt daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(day_start_epoch + 10 * 60 * 60)
        .execute(pool)
        .await
        .expect("corrupt hourly rollup");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(pool)
        .await
        .expect("ready state");
    (
        day_start_epoch + 10 * 60 * 60,
        day_start_epoch + 11 * 60 * 60,
    )
}

#[tokio::test]
async fn legacy_source_timing_queries_accept_missing_optional_columns() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query("CREATE TABLE codex_invocations (occurred_at TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("legacy invocation schema without optional columns");
    sqlx::query("INSERT INTO codex_invocations (occurred_at) VALUES ('2025-01-01 10:00:00')")
        .execute(&pool)
        .await
        .expect("legacy invocation row without optional columns");

    let archive_columns = load_archive_table_columns(&pool, "codex_invocations")
        .await
        .expect("legacy archive columns");
    let archive_rows = sqlx::query_as::<_, LongTermSourceTimingRow>(
        &long_term_source_timing_archive_query(&archive_columns),
    )
    .fetch_all(&pool)
    .await
    .expect("source timing query accepts absent optional archive columns");
    assert_eq!(archive_rows.len(), 1);
    assert_eq!(archive_rows[0].invoke_id, None);
    assert_eq!(archive_rows[0].t_total_ms, None);

    sqlx::query("DROP TABLE codex_invocations")
        .execute(&pool)
        .await
        .expect("replace legacy invocation schema");
    sqlx::query(
        "CREATE TABLE codex_invocations (invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL)",
    )
    .execute(&pool)
    .await
    .expect("legacy invocation schema without timing column");
    sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at) VALUES ('legacy-invoke', '2025-01-01 10:00:00')",
        )
        .execute(&pool)
        .await
        .expect("legacy invocation row without timing column");

    let archive_columns = load_archive_table_columns(&pool, "codex_invocations")
        .await
        .expect("legacy archive columns with invoke id");
    let pairs = HashSet::from([(
        "legacy-invoke".to_string(),
        "2025-01-01 10:00:00".to_string(),
    )]);
    let matched_rows = load_long_term_source_timing_rows_for_pairs(
        &pool,
        &pairs,
        &long_term_legacy_column_expr(&archive_columns, "t_total_ms"),
    )
    .await
    .expect("matched source timing query accepts absent timing column");
    assert_eq!(matched_rows.len(), 1);
    assert_eq!(matched_rows[0].invoke_id.as_deref(), Some("legacy-invoke"));
    assert_eq!(matched_rows[0].t_total_ms, None);
}

#[tokio::test]
async fn queued_repair_attempt_scan_covers_the_replayed_archive_date() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let month_key = date.format("%Y-%m").to_string();
    let occurred_at = format!("{date} 10:00:00");
    let archive_db_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-repair-attempt-source-{}-{}.sqlite",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    let archive_path = archive_db_path.with_extension("sqlite.gz");
    fs::File::create(&archive_db_path).expect("create attempt archive database");
    let archive_options = format!("sqlite://{}", archive_db_path.to_string_lossy())
        .parse::<SqliteConnectOptions>()
        .expect("parse attempt archive URL")
        .create_if_missing(true);
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(archive_options)
        .await
        .expect("open attempt archive database");
    sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, upstream_account_id INTEGER)",
        )
        .execute(&archive_pool)
        .await
        .expect("create attempt archive schema");
    sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, upstream_account_id) VALUES (1, 'repair-invoke', ?1, 42)",
        )
        .bind(&occurred_at)
        .execute(&archive_pool)
        .await
        .expect("insert archived attempt mapping");
    archive_pool.close().await;
    crate::maintenance::deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress attempt archive");
    let archive_sha256 =
        crate::maintenance::sha256_hex_file(&archive_path).expect("hash attempt archive");
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
            VALUES ('pool_upstream_request_attempts', ?1, ?2, ?3, 1, 'completed', ?4, ?4, datetime('now'))
            "#,
        )
        .bind(month_key)
        .bind(archive_path.to_string_lossy().to_string())
        .bind(archive_sha256)
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("record attempt archive manifest");

    let previous_date = date.pred_opt().expect("previous date");
    let next_date = date.succ_opt().expect("next date");
    let (outside_range, _) =
        load_long_term_archive_attempt_accounts(&pool, Some((next_date, next_date)))
            .await
            .expect("scan outside queued repair date");
    let (queued_repair_accounts, _) =
        load_long_term_archive_attempt_accounts(&pool, Some((date, date)))
            .await
            .expect("scan queued repair date");
    let (cross_midnight_repair_accounts, _) =
        load_long_term_archive_attempt_accounts(&pool, Some((previous_date, date)))
            .await
            .expect("scan final rebuild range including the preceding date");

    assert!(outside_range.is_empty());
    assert_eq!(
        queued_repair_accounts.get(&("repair-invoke".to_string(), occurred_at.clone())),
        Some(&42)
    );
    assert_eq!(
        cross_midnight_repair_accounts.get(&("repair-invoke".to_string(), occurred_at)),
        Some(&42)
    );

    let _ = fs::remove_file(&archive_db_path);
    let _ = fs::remove_file(&archive_path);
}

#[tokio::test]
async fn attempt_account_scan_rejects_missing_completed_archive() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let occurred_at = format!("{date} 10:00:00");
    let missing_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-missing-attempt-source-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, created_at
            )
            VALUES ('pool_upstream_request_attempts', ?1, ?2, 'missing-attempt-sha', 1,
                'completed', ?3, ?3, datetime('now'))
            "#,
    )
    .bind(date.format("%Y-%m").to_string())
    .bind(missing_path.to_string_lossy().to_string())
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("record missing attempt archive manifest");

    let error = load_long_term_archive_attempt_accounts(&pool, Some((date, date)))
        .await
        .expect_err("a missing completed archive cannot be treated as an empty mapping");
    assert!(error.to_string().contains("attempt archive is unavailable"));
}

#[tokio::test]
async fn attempt_account_scan_rejects_damaged_completed_archive() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let occurred_at = format!("{date} 10:00:00");
    let damaged_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-damaged-attempt-source-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    fs::write(&damaged_path, b"not a gzip archive").expect("write damaged archive");
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, created_at
            )
            VALUES ('pool_upstream_request_attempts', ?1, ?2, 'damaged-attempt-sha', 1,
                'completed', ?3, ?3, datetime('now'))
            "#,
    )
    .bind(date.format("%Y-%m").to_string())
    .bind(damaged_path.to_string_lossy().to_string())
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("record damaged attempt archive manifest");

    let error = load_long_term_archive_attempt_accounts(&pool, Some((date, date)))
        .await
        .expect_err("a damaged completed archive cannot be treated as an empty mapping");
    assert!(
        error
            .to_string()
            .contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR)
    );
    let _ = fs::remove_file(damaged_path);
}

#[tokio::test]
async fn refresh_hides_ready_stats_when_attempt_attribution_source_is_unavailable() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive();
    let occurred_at = format!("{date}T10:00:00+08:00");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, raw_response, total_tokens, output_tokens, cost, created_at) VALUES (1, 'missing-attribution-source', ?1, 'success', 'gpt-5', '{}', '{}', 100, 40, 0.1, datetime('now'))",
        )
        .bind(&occurred_at)
        .execute(&pool)
        .await
        .expect("insert live invocation requiring account attribution");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("seed visible durable long-term data");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("seed ready state");
    let missing_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-missing-ready-attempt-source-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, created_at
            )
            VALUES ('pool_upstream_request_attempts', ?1, ?2, 'missing-ready-attempt-sha',
                1, 'completed', ?3, ?3, datetime('now'))
            "#,
    )
    .bind(date.format("%Y-%m").to_string())
    .bind(missing_path.to_string_lossy().to_string())
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("record missing attribution source");

    let error = refresh_long_term_stats(&pool, 400)
        .await
        .expect_err("missing attribution source must block a ready refresh");
    assert!(
        error
            .to_string()
            .contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR)
    );
    let state: (String, Option<String>) =
        sqlx::query_as("SELECT status, last_error FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("load blocked long-term state");
    assert_eq!(state.0, LONG_TERM_STATUS_ERROR);
    assert!(
        state
            .1
            .as_deref()
            .is_some_and(|message| message.contains(LONG_TERM_ATTEMPT_ARCHIVE_UNAVAILABLE_ERROR))
    );
    let calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("load preserved durable data");
    assert_eq!(calls, 1);
}

#[test]
fn matched_attempt_source_rows_require_parseable_effective_dates() {
    let pair = (
        "matched-attempt-source".to_string(),
        "invalid-timestamp".to_string(),
    );
    let mut unmatched_pairs = HashSet::from([pair.clone()]);
    let mut latest_effective_date = None;

    let error = record_long_term_matched_attempt_source_rows(
        &mut unmatched_pairs,
        &mut latest_effective_date,
        vec![LongTermSourceTimingRow {
            invoke_id: Some(pair.0.clone()),
            occurred_at: pair.1.clone(),
            t_total_ms: None,
        }],
    )
    .expect_err("a matched source with an invalid timestamp cannot prove an archive boundary");

    assert!(error.to_string().contains("unparseable timestamp"));
    assert_eq!(unmatched_pairs, HashSet::from([pair]));
    assert_eq!(latest_effective_date, None);
}

#[test]
fn range_defaults_to_seven_days_and_rejects_unknown_values() {
    assert_eq!(LongTermRange::parse(None), Some(LongTermRange::Seven));
    assert_eq!(
        LongTermRange::parse(Some("365d")),
        Some(LongTermRange::ThreeSixtyFive)
    );
    assert_eq!(LongTermRange::parse(Some("1d")), None);
}

#[test]
fn refresh_keeps_error_visible_while_an_integrity_repair_is_pending() {
    assert_eq!(
        long_term_refresh_start_state(true, true),
        (LONG_TERM_STATUS_ERROR, false)
    );
    assert_eq!(
        long_term_refresh_start_state(true, false),
        (LONG_TERM_STATUS_READY, true)
    );
    assert_eq!(
        long_term_refresh_start_state(false, false),
        (LONG_TERM_STATUS_RUNNING, true)
    );
    assert_eq!(
        long_term_refresh_start_state(false, true),
        (LONG_TERM_STATUS_ERROR, false)
    );
    assert_eq!(
        long_term_refresh_pending_marker(true, LONG_TERM_STATUS_READY),
        None
    );
    assert_eq!(
        long_term_refresh_pending_marker(false, LONG_TERM_STATUS_RUNNING),
        Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
    );
}

#[test]
fn reconstructable_start_uses_the_persisted_source_boundary() {
    let retention_start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("fixed retention start");
    let archived_through = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed archive end");

    assert_eq!(
        long_term_source_safe_start_after_effective_date(archived_through),
        NaiveDate::from_ymd_opt(2026, 7, 24).expect("exact successor safe start")
    );
    assert_eq!(
        long_term_reconstructable_start(retention_start, None, Some("2026-07-25")),
        NaiveDate::from_ymd_opt(2026, 7, 25).expect("persisted safe start")
    );
}

#[tokio::test]
async fn unreadable_source_without_a_coverage_start_blocks_the_full_retention_window() {
    let retention_start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("fixed retention start");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                coverage_end_at,
                created_at
            )
            VALUES ('codex_invocations', '2026-07', '/tmp/missing-archive.sqlite.gz', 'missing-sha', 1, 'completed', '2026-07-23 23:59:59', datetime('now'))
            "#,
        )
        .execute(&pool)
        .await
        .expect("missing archive manifest without a coverage start");
    let archive_path = load_completed_invocation_archive_paths(&pool)
        .await
        .expect("load completed archive manifest")
        .into_iter()
        .next()
        .expect("one archive manifest");

    assert_eq!(
        long_term_unreadable_source_start(&archive_path, retention_start),
        retention_start,
        "an end timestamp cannot prove that an unreadable archive has no earlier rows"
    );
}

#[tokio::test]
async fn backfill_start_preserves_error_for_a_pending_integrity_repair() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES ('2026-07-23', 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .execute(&pool)
        .await
        .expect("durable daily rollup");
    sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES ('2026-07-23', 2, 200, 0.2, 1, 100, 0.1, 'integrity mismatch')",
        )
        .execute(&pool)
        .await
        .expect("pending repair queue");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("integrity error state");

    mark_long_term_stats_backfill_preparing(&pool)
        .await
        .expect("preserve error state");

    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("long-term state");
    assert_eq!(status, LONG_TERM_STATUS_ERROR);
}

#[tokio::test]
async fn integrity_source_boundary_waits_for_contiguous_archive_retirement() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
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
                created_at
            )
            VALUES (2, 'codex_invocations', '2025-01', '/tmp/retained-overlap.sqlite.gz', 'retained-overlap-sha', 1, 'completed', '2025-01-02 00:00:00', datetime('now'))
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert later retained archive coverage");

    let first_safe_start = NaiveDate::from_ymd_opt(2025, 1, 5).expect("fixed date");
    let mut tx = pool.begin().await.expect("start first cleanup transaction");
    advance_long_term_integrity_source_start_tx(tx.as_mut(), 1, first_safe_start)
        .await
        .expect("defer boundary behind retained overlapping source");
    tx.commit().await.expect("commit deferred boundary");

    let state: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT integrity_source_start_date, integrity_source_pending_start_date FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("load deferred boundary state");
    assert_eq!(state.0, None);
    assert_eq!(state.1.as_deref(), Some("2025-01-05"));

    let mut tx = pool
        .begin()
        .await
        .expect("start contiguous cleanup transaction");
    advance_long_term_integrity_source_start_tx(
        tx.as_mut(),
        2,
        NaiveDate::from_ymd_opt(2025, 1, 4).expect("fixed date"),
    )
    .await
    .expect("commit accumulated safe boundary after final retained source retires");
    tx.commit().await.expect("commit contiguous boundary");

    let state: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT integrity_source_start_date, integrity_source_pending_start_date FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("load committed contiguous boundary");
    assert_eq!(state.0.as_deref(), Some("2025-01-05"));
    assert_eq!(state.1, None);
}

#[test]
fn series_query_parser_accepts_repeated_keys() {
    let uri: Uri = "/api/stats/long-term/series?range=30d&dimension=model&key=one&key=two"
        .parse()
        .expect("valid series URI");
    let query = parse_long_term_series_query(&uri);
    assert_eq!(query.range.as_deref(), Some("30d"));
    assert_eq!(query.dimension.as_deref(), Some("model"));
    assert_eq!(query.key, ["one", "two"]);
}

#[test]
fn model_series_key_encodes_model_and_reasoning_without_collisions() {
    let left = long_term_model_series_key("a|reasoning:b", "c");
    let right = long_term_model_series_key("a", "b|reasoning:c");
    assert_ne!(left, right);
    assert!(left.starts_with("model:v2:"));
}

#[test]
fn wall_time_union_deduplicates_overlapping_accounts_and_hour_boundaries() {
    assert_eq!(union_interval_duration(&[(0, 10), (5, 15), (20, 25)]), 20);
    assert_eq!(
        union_interval_duration(&[(0, 3_600_000), (0, 3_600_000)]),
        3_600_000
    );
}

#[tokio::test]
async fn integrity_oracle_uses_shanghai_day_boundaries() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    create_long_term_integrity_oracle(&pool).await;
    let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
    let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("day bounds");
    for (offset, source) in [
        (-1_i64, "previous-day"),
        (0_i64, "day-start"),
        (23_i64, "day-end"),
        (24_i64, "next-day"),
    ] {
        sqlx::query(
                "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, ?2, 1, 1, 100, 0.1, 100, 0.1)",
            )
            .bind(start_epoch + offset * 60 * 60)
            .bind(source)
            .execute(&pool)
            .await
            .expect("canonical hourly rollup");
    }

    let oracle = load_long_term_integrity_oracle(&pool, date)
        .await
        .expect("load integrity oracle")
        .expect("oracle rows for requested date");
    assert_eq!(oracle.daily.calls, 2);
    assert_eq!(oracle.daily.token_total, 200);
    assert_eq!(oracle.hourly.len(), 2);
    assert!(oracle.hourly.contains_key(&start_epoch));
    assert!(oracle.hourly.contains_key(&(end_epoch - 60 * 60)));
    assert_eq!(long_term_bucket_date(start_epoch - 1), date.pred_opt());
    assert_eq!(long_term_bucket_date(start_epoch), Some(date));
    assert_eq!(long_term_bucket_date(end_epoch - 1), Some(date));
    assert_eq!(long_term_bucket_date(end_epoch), date.succ_opt());
}

#[tokio::test]
async fn integrity_oracle_represents_an_empty_canonical_day() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    create_long_term_integrity_oracle(&pool).await;
    let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");

    let oracle = load_long_term_integrity_oracle(&pool, date)
        .await
        .expect("load empty integrity oracle")
        .expect("an empty canonical day is still integrity evidence");

    assert_eq!(oracle.date, date);
    assert_eq!(oracle.daily, LongTermIntegrityTotals::default());
    assert!(oracle.hourly.is_empty());
}

#[tokio::test]
async fn integrity_audit_skips_hours_without_terminal_proof() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_integrity_oracle(&pool).await;
    let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'legacy', 1, 1, 100, 0.1, 0, 100, 0.1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("untrusted legacy rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("durable daily rollup");

    assert!(
        load_long_term_integrity_oracle(&pool, date)
            .await
            .expect("load integrity oracle")
            .is_none(),
        "a legacy hourly row cannot prove a complete day until terminal totals are backfilled"
    );
    let mismatches = audit_long_term_integrity(&pool, date, date)
        .await
        .expect("audit untrusted hourly rollup");
    assert!(
        mismatches.is_empty(),
        "unknown canonical data must not be treated as a zero-value integrity oracle"
    );
}

#[tokio::test]
async fn terminal_integrity_oracle_ignores_active_invocations() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost) VALUES (2, 'active', ?1, 'running', 'gpt-5', '{}', 60, 40, 0, 0, 100, 0.1)",
        )
        .bind(format!("{date}T10:10:00+08:00"))
        .execute(&pool)
        .await
        .expect("active source invocation");
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, 'canonical', 2, 1, 100, 0.1, 200, 0.2)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical hourly rollup");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh with active invocation");

    let calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("terminal daily rollup");
    let queued_repairs =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("repair queue count");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("long-term status");
    assert_eq!(calls, 1);
    assert_eq!(queued_repairs, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn hourly_audit_hides_stats_when_a_previously_trusted_archive_source_disappears() {
    let pool = long_term_stats_memory_pool().await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    let missing_archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-missing-trusted-source-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    let missing_archive_path = missing_archive_path.to_string_lossy().to_string();
    seed_hourly_audit_missing_source(&pool, date, day_start, &missing_archive_path).await;
    assert_hourly_audit_source_loss(&pool, &missing_archive_path).await;
}

async fn long_term_stats_memory_pool() -> Pool<Sqlite> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    pool
}

async fn seed_hourly_audit_missing_source(
    pool: &Pool<Sqlite>,
    date: NaiveDate,
    day_start: i64,
    missing_archive_path: &str,
) {
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'trusted', 1, 1, 0, 1, 100, 0.1, 1, 100, 0.1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(pool)
        .await
        .expect("trusted canonical hourly rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(pool)
        .await
        .expect("materialized daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(pool)
        .await
        .expect("materialized hourly rollup");
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
            VALUES ('codex_invocations', '2026-01', ?1, 'missing-trusted-source-sha', 1, 'completed', ?2, ?2, datetime('now'))
            "#,
        )
        .bind(missing_archive_path)
        .bind(format!("{date} 10:00:00"))
        .execute(pool)
        .await
        .expect("missing completed archive manifest");
    sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, 'missing-trusted-source-sha')",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(missing_archive_path)
        .execute(pool)
        .await
        .expect("existing replay marker");
    sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(pool)
        .await
        .expect("make hourly audit due");
}

async fn assert_hourly_audit_source_loss(pool: &Pool<Sqlite>, missing_archive_path: &str) {
    refresh_long_term_stats(pool, 400)
        .await
        .expect("hourly audit should tolerate a missing replayed archive");

    let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(pool)
    .await
    .expect("long-term state after source loss");
    let proof = sqlx::query_scalar::<_, i64>(
        "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE source = 'trusted'",
    )
    .fetch_one(pool)
    .await
    .expect("revoked terminal proof");
    assert_eq!(status, LONG_TERM_STATUS_ERROR);
    assert_eq!(
        last_error.as_deref(),
        Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
    );
    assert_eq!(proof, 0);
    let replay_markers = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(missing_archive_path)
        .fetch_one(pool)
        .await
        .expect("count replay markers after source loss");
    assert_eq!(
        replay_markers, 0,
        "a missing replayed source must be retried if the archive is restored with the same identity"
    );
}

#[tokio::test]
async fn hourly_audit_repairs_stats_when_complete_sources_disagree_with_canonical() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    let hour_start = day_start + 10 * 60 * 60;

    sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, detail_level, model, payload, raw_response, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost) VALUES ('reconciliation-source', ?1, ?2, 'success', 'full', 'gpt-5', '{}', '{}', 60, 40, 0, 0, 100, 0.1)",
        )
        .bind(format!("{date} 10:00:00"))
        .bind(SOURCE_XY)
        .execute(&pool)
        .await
        .expect("complete source row");
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, ?2, 2, 2, 0, 2, 200, 0.2, 1, 200, 0.2)",
        )
        .bind(hour_start)
        .bind(SOURCE_XY)
        .execute(&pool)
        .await
        .expect("contradictory canonical hourly rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("stale materialized daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("stale materialized hourly rollup");
    sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("make reconciliation due");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("contradictory source reconciliation is repaired");

    let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("long-term state");
    let proof = sqlx::query_scalar::<_, i64>(
            "SELECT terminal_proof_complete FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
        )
        .bind(hour_start)
        .bind(SOURCE_XY)
        .fetch_one(&pool)
        .await
        .expect("repaired terminal proof");
    let queue_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("repair queue count");
    let repaired_daily_calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("repaired daily rollup");

    assert_eq!(status, LONG_TERM_STATUS_READY);
    assert_eq!(last_error, None);
    assert_eq!(proof, 1);
    assert_eq!(queue_count, 0);
    assert_eq!(repaired_daily_calls, 1);
}

#[tokio::test]
async fn refresh_hides_stats_when_terminal_proof_reconciliation_requires_a_missing_column() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    let hour_start = day_start + 10 * 60 * 60;

    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'trusted', 1, 1, 0, 1, 100, 0.1, 1, 100, 0.1)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("trusted canonical hourly rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("materialized daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("materialized hourly rollup");
    sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("make reconciliation due");

    // The long-term query can read this legacy shape, but the canonical proof scan requires
    // `source`. Treating that failure as harmless would incorrectly publish `ready`.
    sqlx::query("DROP TABLE codex_invocations")
        .execute(&pool)
        .await
        .expect("replace invocation source table");
    sqlx::query(
        r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT,
                occurred_at TEXT NOT NULL,
                status TEXT,
                model TEXT,
                payload TEXT,
                total_tokens INTEGER,
                output_tokens INTEGER,
                cost REAL,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                t_upstream_stream_ms REAL,
                error_message TEXT
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect("create long-term-readable legacy invocation schema");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("schema reconciliation failure is contained as an availability error");

    let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("long-term state after reconciliation failure");
    assert_eq!(status, LONG_TERM_STATUS_ERROR);
    assert_eq!(
        last_error.as_deref(),
        Some(LONG_TERM_TERMINAL_PROOF_UNAVAILABLE_ERROR)
    );
}

#[tokio::test]
async fn refresh_retries_terminal_proof_reconciliation_after_source_availability_recovers() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    let missing_archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-recoverable-trusted-source-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    let missing_archive_path = missing_archive_path.to_string_lossy().to_string();

    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'trusted', 1, 1, 0, 1, 100, 0.1, 1, 100, 0.1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("trusted canonical hourly rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("materialized daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("materialized hourly rollup");
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
            VALUES ('codex_invocations', '2026-01', ?1, 'recoverable-trusted-source-sha', 1, 'completed', ?2, ?2, datetime('now'))
            "#,
        )
        .bind(&missing_archive_path)
        .bind(format!("{date} 10:00:00"))
        .execute(&pool)
        .await
        .expect("missing completed archive manifest");
    sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("make first reconciliation due");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("missing source should become a retryable availability error");
    sqlx::query("DELETE FROM archive_batches WHERE file_path = ?1")
        .bind(&missing_archive_path)
        .execute(&pool)
        .await
        .expect("simulate restored source availability before the next hourly audit");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("next refresh should retry terminal proof reconciliation immediately");

    let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("long-term state after source recovery");
    assert_eq!(status, LONG_TERM_STATUS_READY);
    assert_eq!(last_error, None);
}

#[tokio::test]
async fn refresh_keeps_error_after_proof_revocation_when_a_later_source_read_fails() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    let missing_archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-missing-source-before-later-error-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    let missing_archive_path = missing_archive_path.to_string_lossy().to_string();

    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'trusted', 1, 1, 0, 1, 100, 0.1, 1, 100, 0.1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("trusted canonical hourly rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("materialized daily rollup");
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
            VALUES ('codex_invocations', '2026-01', ?1, 'missing-source-before-later-error', 1, 'completed', ?2, ?2, datetime('now'))
            "#,
        )
        .bind(&missing_archive_path)
        .bind(format!("{date} 10:00:00"))
        .execute(&pool)
        .await
        .expect("missing completed archive manifest");
    sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(date.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("make hourly audit due");

    // Reconciliation above revokes the proof; this malformed relation then fails a later
    // source read in the same refresh and exercises the fallback status path.
    sqlx::query("DROP TABLE pool_upstream_accounts")
        .execute(&pool)
        .await
        .expect("drop account source table");
    sqlx::query("CREATE TABLE pool_upstream_accounts (id INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("create malformed account source table");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect_err("later source read should fail after the proof is revoked");

    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("load long-term state after later error");
    assert_eq!(status, LONG_TERM_STATUS_ERROR);
}
