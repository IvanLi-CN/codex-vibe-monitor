use super::*;

pub(super) async fn create_long_term_test_invocations(pool: &Pool<Sqlite>) {
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
pub(super) async fn terminal_projection_seek_uses_terminal_id_index_after_pending_prefix() {
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

pub(super) async fn create_long_term_integrity_oracle(pool: &Pool<Sqlite>) {
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

pub(super) async fn long_term_file_backed_pool_with_busy_timeout(
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

pub(super) async fn long_term_file_backed_pool(prefix: &str) -> (Pool<Sqlite>, String, PathBuf) {
    long_term_file_backed_pool_with_busy_timeout(prefix, Duration::from_millis(50)).await
}

pub(super) async fn cleanup_long_term_file_backed_pool(pool: Pool<Sqlite>, db_path: PathBuf) {
    pool.close().await;
    for suffix in ["", "-shm", "-wal"] {
        let path = PathBuf::from(format!("{}{}", db_path.display(), suffix));
        let _ = fs::remove_file(path);
    }
}

pub(super) async fn insert_long_term_test_invocation(
    pool: &Pool<Sqlite>,
    id: i64,
    occurred_at: String,
) {
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

pub(super) async fn seed_long_term_integrity_case(
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

pub(super) fn projection_interval_segment(
    invocation_row_id: i64,
    interval_start_ms: i64,
    interval_end_ms: i64,
) -> LongTermProjectionIntervalSegment {
    LongTermProjectionIntervalSegment {
        invocation_row_id,
        model_series_key: "model:gpt-5:high".to_string(),
        upstream_series_key: "account:42".to_string(),
        interval_start_ms,
        interval_end_ms,
    }
}
