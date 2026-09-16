#[cfg(test)]
mod upstream_host_network_minute_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                status TEXT,
                detail_level TEXT,
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
                is_actionable INTEGER,
                payload TEXT,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                t_upstream_stream_ms REAL,
                t_resp_parse_ms REAL,
                t_persist_ms REAL,
                response_raw_size INTEGER,
                raw_response TEXT NOT NULL DEFAULT '',
                request_raw_size INTEGER,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create codex_invocations table");
        sqlx::query(
            r#"
            CREATE TABLE pool_upstream_request_attempts (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                upstream_base_url_host TEXT,
                upstream_account_id INTEGER,
                attempt_index INTEGER,
                upstream_request_header_bytes_approx INTEGER,
                upstream_request_transmitted_body_bytes INTEGER,
                upstream_response_header_bytes_approx INTEGER,
                upstream_response_body_bytes INTEGER
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create pool_upstream_request_attempts table");
        sqlx::query(
            r#"
            CREATE TABLE upstream_host_network_minute (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_base_url_host TEXT NOT NULL,
                upload_bytes INTEGER NOT NULL DEFAULT 0,
                download_bytes INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (bucket_start_epoch, source, upstream_base_url_host)
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_host_network_minute table");
        sqlx::query(
            r#"
            CREATE TABLE upstream_account_usage_breakdown_hourly (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_account_key TEXT NOT NULL,
                upstream_account_id INTEGER,
                normalized_model TEXT NOT NULL,
                normalized_reasoning_effort TEXT NOT NULL DEFAULT '',
                request_count INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cost_input REAL NOT NULL DEFAULT 0,
                cost_cache_write REAL NOT NULL DEFAULT 0,
                cost_cache_read REAL NOT NULL DEFAULT 0,
                cost_output REAL NOT NULL DEFAULT 0,
                cost_reasoning REAL NOT NULL DEFAULT 0,
                cost_unknown REAL NOT NULL DEFAULT 0,
                has_cost INTEGER NOT NULL DEFAULT 0,
                performance_total_tokens INTEGER NOT NULL DEFAULT 0,
                performance_stream_output_tokens INTEGER NOT NULL DEFAULT 0,
                performance_stream_duration_ms REAL NOT NULL DEFAULT 0,
                performance_response_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_response_sum_ms REAL NOT NULL DEFAULT 0,
                performance_first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_first_byte_sum_ms REAL NOT NULL DEFAULT 0,
                performance_first_token_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_first_token_sum_ms REAL NOT NULL DEFAULT 0,
                performance_usage_duration_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_usage_duration_sum_ms REAL NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (
                    bucket_start_epoch,
                    source,
                    upstream_account_key,
                    normalized_model,
                    normalized_reasoning_effort
                )
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_account_usage_breakdown_hourly table");
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS hourly_rollup_live_progress (
                dataset TEXT PRIMARY KEY,
                cursor_id INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create hourly_rollup_live_progress table");
        pool
    }

    #[tokio::test]
    async fn legacy_compatible_archive_query_reads_archives_without_optional_columns() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory legacy archive pool");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create minimal legacy invocation archive schema");
        sqlx::query("INSERT INTO codex_invocations (id, occurred_at, source) VALUES (1, ?1, ?2)")
            .bind("2026-07-23 10:00:00")
            .bind(SOURCE_PROXY)
            .execute(&pool)
            .await
            .expect("insert minimal legacy invocation archive row");

        let columns = load_archive_table_columns(&pool, "codex_invocations")
            .await
            .expect("inspect minimal legacy archive schema");
        let query = build_legacy_compatible_invocation_archive_query(&columns);
        let row = sqlx::query_as::<_, InvocationHourlySourceRecord>(&query)
            .bind(0_i64)
            .bind(10_i64)
            .fetch_one(&pool)
            .await
            .expect("read minimal legacy invocation archive row");

        assert_eq!(row.id, 1);
        assert_eq!(row.detail_level, DETAIL_LEVEL_FULL);
        assert!(row.input_tokens.is_none());
        assert!(row.output_tokens.is_none());
        assert!(row.reasoning_tokens.is_none());
        assert!(row.t_total_ms.is_none());
        assert!(row.t_req_read_ms.is_none());
        assert!(row.t_req_parse_ms.is_none());
        assert!(row.t_upstream_connect_ms.is_none());
        assert!(row.t_upstream_ttfb_ms.is_none());
        assert!(row.first_token_ms.is_none());
        assert!(row.t_upstream_stream_ms.is_none());
        assert!(row.t_resp_parse_ms.is_none());
        assert!(row.t_persist_ms.is_none());
    }

    #[tokio::test]
    async fn legacy_compatible_archive_query_keeps_partial_token_components_unknown() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory archive pool");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_input_tokens INTEGER,
                reasoning_tokens INTEGER,
                total_tokens INTEGER
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create partial token archive schema");
        sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, source, output_tokens, total_tokens) VALUES (1, ?1, ?2, 30, 30)",
        )
        .bind("2026-07-23 10:00:00")
        .bind(SOURCE_PROXY)
        .execute(&pool)
        .await
        .expect("insert partial token archive row");

        let columns = load_archive_table_columns(&pool, "codex_invocations")
            .await
            .expect("inspect partial token archive schema");
        let query = build_legacy_compatible_invocation_archive_query(&columns);
        let row = sqlx::query_as::<_, InvocationHourlySourceRecord>(&query)
            .bind(0_i64)
            .bind(10_i64)
            .fetch_one(&pool)
            .await
            .expect("read partial token archive row");

        assert_eq!(row.total_tokens, Some(30));
        assert!(row.input_tokens.is_none());
        assert!(row.output_tokens.is_none());
        assert!(row.cache_input_tokens.is_none());
        assert!(row.reasoning_tokens.is_none());
    }

    async fn create_upstream_account_stats_rebuild_tables(pool: &Pool<Sqlite>) {
        for table_name in [
            "upstream_account_stats_hourly",
            "upstream_account_stats_minute",
        ] {
            sqlx::query(&format!(
                "CREATE TABLE {table_name} (bucket_start_epoch INTEGER PRIMARY KEY)"
            ))
            .execute(pool)
            .await
            .expect("create upstream account stats rollup table");
            sqlx::query(&format!(
                "INSERT INTO {table_name} (bucket_start_epoch) VALUES (1)"
            ))
            .execute(pool)
            .await
            .expect("seed upstream account stats rollup");
        }
        sqlx::query(
            r#"
            CREATE TABLE archive_batches (
                id INTEGER PRIMARY KEY,
                file_path TEXT NOT NULL,
                coverage_start_at TEXT,
                coverage_end_at TEXT,
                dataset TEXT NOT NULL,
                status TEXT NOT NULL,
                month_key TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .expect("create archive batches table");
    }

    #[tokio::test]
    async fn account_stats_rebuild_preserves_rollups_when_archive_is_missing() {
        let pool = test_pool().await;
        create_upstream_account_stats_rebuild_tables(&pool).await;
        sqlx::query(
            "INSERT INTO archive_batches (id, file_path, dataset, status, month_key, created_at) VALUES (1, '/missing/invocations.sqlite.gz', 'codex_invocations', 'completed', '2026-07', '2026-07-01 00:00:00')",
        )
        .execute(&pool)
        .await
        .expect("seed missing invocation archive");

        let counts = rebuild_upstream_account_stats_rollups_from_sources(&pool)
            .await
            .expect("rebuild should preserve existing rollups");

        assert_eq!(counts, (1, 1));
    }

    #[tokio::test]
    async fn account_stats_rebuild_preserves_rollups_when_live_token_components_are_partial() {
        let pool = test_pool().await;
        create_upstream_account_stats_rebuild_tables(&pool).await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, created_at, detail_level,
                input_tokens, output_tokens, total_tokens
            ) VALUES (1, 'partial-components', '2026-07-23 10:00:00', 'proxy', '2026-07-23 10:00:00', 'full', 80, 20, 100)
            "#,
        )
        .execute(&pool)
        .await
        .expect("seed partial live invocation");

        let counts = rebuild_upstream_account_stats_rollups_from_sources(&pool)
            .await
            .expect("rebuild should preserve existing rollups");

        assert_eq!(counts, (1, 1));
    }

    async fn save_progress(pool: &Pool<Sqlite>, dataset: &str, cursor_id: i64) {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_live_progress (dataset, cursor_id)
            VALUES (?1, ?2)
            ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id
            "#,
        )
        .bind(dataset)
        .bind(cursor_id)
        .execute(pool)
        .await
        .expect("save live progress");
    }

    #[tokio::test]
    async fn upstream_host_network_direct_rollup_seeds_cursor_without_backfill() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                payload,
                response_raw_size,
                raw_response,
                request_raw_size,
                source,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(7_i64)
        .bind("invoke-seed")
        .bind("2026-07-18 15:00:00")
        .bind(r#"{"upstreamBaseUrlHost":"api.openai.com","upstreamApproxUploadBytes":10,"upstreamApproxDownloadBytes":20}"#)
        .bind(0_i64)
        .bind("")
        .bind(0_i64)
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:00:00Z")
        .execute(&pool)
        .await
        .expect("insert seed invocation");

        let updated = replay_live_upstream_host_network_minute_rollups_from_invocations(&pool)
            .await
            .expect("seed direct rollup cursor");
        assert_eq!(updated, 0);

        let row_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM upstream_host_network_minute")
                .fetch_one(&pool)
                .await
                .expect("count upstream host minute rows");
        assert_eq!(row_count, 0);

        let cursor_id: i64 = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
        )
        .bind(HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT)
        .fetch_one(&pool)
        .await
        .expect("load seeded cursor");
        assert_eq!(cursor_id, 7);
    }

    #[tokio::test]
    async fn upstream_host_network_direct_rollup_persists_normalized_host_bytes() {
        let pool = test_pool().await;
        save_progress(&pool, HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT, 0).await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                payload,
                response_raw_size,
                raw_response,
                request_raw_size,
                source,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(1_i64)
        .bind("invoke-direct-host")
        .bind("2026-07-18 15:01:00")
        .bind(r#"{"upstreamBaseUrlHost":"API.OpenAI.com","upstreamApproxUploadBytes":120,"upstreamApproxDownloadBytes":240}"#)
        .bind(0_i64)
        .bind("")
        .bind(0_i64)
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:01:00Z")
        .execute(&pool)
        .await
        .expect("insert direct invocation");

        let updated = replay_live_upstream_host_network_minute_rollups_from_invocations(&pool)
            .await
            .expect("replay direct host minute rollups");
        assert_eq!(updated, 1);

        let row = sqlx::query_as::<_, (i64, String, String, i64, i64)>(
            r#"
            SELECT
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upload_bytes,
                download_bytes
            FROM upstream_host_network_minute
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load direct host minute row");
        assert_eq!(row.2, "api.openai.com");
        assert_eq!(row.3, 120);
        assert_eq!(row.4, 240);
    }

    #[tokio::test]
    async fn upstream_host_network_pool_rollup_splits_retry_hosts() {
        let pool = test_pool().await;
        save_progress(
            &pool,
            HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_POOL_ATTEMPTS,
            0,
        )
        .await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                payload,
                response_raw_size,
                raw_response,
                request_raw_size,
                source,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(1_i64)
        .bind("invoke-pool-hosts")
        .bind("2026-07-18 15:02:00")
        .bind(r#"{"routeMode":"pool"}"#)
        .bind(0_i64)
        .bind("")
        .bind(0_i64)
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:02:00Z")
        .execute(&pool)
        .await
        .expect("insert pool invocation");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                id,
                invoke_id,
                occurred_at,
                upstream_base_url_host,
                upstream_request_header_bytes_approx,
                upstream_request_transmitted_body_bytes,
                upstream_response_header_bytes_approx,
                upstream_response_body_bytes
            )
            VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8),
                (?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            "#,
        )
        .bind(1_i64)
        .bind("invoke-pool-hosts")
        .bind("2026-07-18 15:02:00")
        .bind("primary.example.com")
        .bind(20_i64)
        .bind(80_i64)
        .bind(10_i64)
        .bind(30_i64)
        .bind(2_i64)
        .bind("invoke-pool-hosts")
        .bind("2026-07-18 15:02:00")
        .bind("backup.example.com")
        .bind(15_i64)
        .bind(45_i64)
        .bind(5_i64)
        .bind(25_i64)
        .execute(&pool)
        .await
        .expect("insert pool attempts");

        let updated = replay_live_upstream_host_network_minute_rollups_from_pool_attempts(&pool)
            .await
            .expect("replay pool host minute rollups");
        assert_eq!(updated, 2);

        let rows = sqlx::query_as::<_, (String, i64, i64)>(
            r#"
            SELECT upstream_base_url_host, upload_bytes, download_bytes
            FROM upstream_host_network_minute
            ORDER BY upstream_base_url_host ASC
            "#,
        )
        .fetch_all(&pool)
        .await
        .expect("load pool host minute rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("backup.example.com".to_string(), 60, 30));
        assert_eq!(rows[1], ("primary.example.com".to_string(), 100, 40));
    }

    #[tokio::test]
    async fn live_invocation_hourly_rows_preserve_attempt_fallback_account_id() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                payload,
                raw_response,
                source,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            "#,
        )
        .bind(11_i64)
        .bind("invoke-fallback-account")
        .bind("2026-07-18 15:03:00")
        .bind("success")
        .bind(DETAIL_LEVEL_FULL)
        .bind("gpt-5")
        .bind(40_i64)
        .bind(60_i64)
        .bind(10_i64)
        .bind(100_i64)
        .bind(0.25_f64)
        .bind(r#"{"routeMode":"pool","responseModel":"gpt-5"}"#)
        .bind("")
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:03:00Z")
        .execute(&pool)
        .await
        .expect("insert invocation without payload account");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                id,
                invoke_id,
                occurred_at,
                attempt_index,
                upstream_account_id
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(17_i64)
        .bind("invoke-fallback-account")
        .bind("2026-07-18 15:03:00")
        .bind(0_i64)
        .bind(42_i64)
        .execute(&pool)
        .await
        .expect("insert fallback attempt account");

        let bucket_start_epoch =
            invocation_bucket_start_epoch("2026-07-18 15:03:00").expect("bucket start");
        let mut tx = pool.begin().await.expect("begin tx");
        let rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(
            tx.as_mut(),
            &[bucket_start_epoch],
        )
        .await
        .expect("load live hourly rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].upstream_account_id, Some(42));
        assert_eq!(rows[0].resolved_upstream_account_id(), Some(42));
    }

    #[tokio::test]
    async fn usage_breakdown_live_repair_backfills_rows_behind_shared_cursor() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                payload,
                raw_response,
                source,
                created_at
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19, ?20
            )
            "#,
        )
        .bind(41_i64)
        .bind("invoke-upgrade-breakdown-repair")
        .bind("2026-07-18 15:04:00")
        .bind("success")
        .bind(DETAIL_LEVEL_FULL)
        .bind("gpt-5")
        .bind(40_i64)
        .bind(60_i64)
        .bind(10_i64)
        .bind(100_i64)
        .bind(0.25_f64)
        .bind(0.11_f64)
        .bind(0.02_f64)
        .bind(0.03_f64)
        .bind(0.09_f64)
        .bind(0.0_f64)
        .bind(r#"{"routeMode":"pool","responseModel":"gpt-5"}"#)
        .bind("")
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:04:00Z")
        .execute(&pool)
        .await
        .expect("insert retained invocation behind shared cursor");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                id,
                invoke_id,
                occurred_at,
                attempt_index,
                upstream_account_id
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(21_i64)
        .bind("invoke-upgrade-breakdown-repair")
        .bind("2026-07-18 15:04:00")
        .bind(0_i64)
        .bind(42_i64)
        .execute(&pool)
        .await
        .expect("insert fallback attempt for retained invocation");
        save_progress(&pool, HOURLY_ROLLUP_DATASET_INVOCATIONS, 41).await;

        repair_live_invocation_usage_breakdown_rollups(&pool)
            .await
            .expect("repair missing usage breakdown live rows");

        let repaired = sqlx::query_as::<_, (Option<i64>, String, i64, f64)>(
            r#"
            SELECT upstream_account_id, normalized_model, request_count, cost_input
            FROM upstream_account_usage_breakdown_hourly
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load repaired usage breakdown rollup row");
        assert_eq!(repaired.0, Some(42));
        assert_eq!(repaired.1, "gpt-5");
        assert_eq!(repaired.2, 1);
        assert_eq!(repaired.3, 0.11_f64);

        let shared_cursor: i64 = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .fetch_one(&pool)
        .await
        .expect("load shared invocation cursor");
        assert_eq!(shared_cursor, 41);

        let repair_cursor: i64 = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
        )
        .bind(INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET)
        .fetch_one(&pool)
        .await
        .expect("load breakdown repair cursor");
        assert_eq!(repair_cursor, 41);

        let repair_marker: i64 = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
        )
        .bind(INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET)
        .fetch_one(&pool)
        .await
        .expect("load breakdown repair marker");
        assert_eq!(
            repair_marker,
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE
        );
    }
}
