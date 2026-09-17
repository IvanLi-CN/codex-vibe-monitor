#[cfg(test)]
mod dashboard_network_timeseries_tests {
    use super::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn network_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                invoke_id TEXT NOT NULL DEFAULT '',
                occurred_at TEXT NOT NULL,
                payload TEXT,
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
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                upstream_account_id INTEGER,
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
            CREATE TABLE upstream_socket_network_minute (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_base_url_host TEXT NOT NULL,
                upstream_account_id INTEGER,
                upload_bytes INTEGER NOT NULL DEFAULT 0,
                download_bytes INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (
                    bucket_start_epoch,
                    source,
                    upstream_base_url_host,
                    upstream_account_id
                )
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_socket_network_minute table");
        pool
    }

    #[tokio::test]
    async fn dashboard_network_bucket_rows_read_real_socket_minute_rows() {
        let pool = network_pool().await;
        sqlx::query(
            r#"
            INSERT INTO upstream_socket_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id,
                upload_bytes,
                download_bytes
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(
            Utc.with_ymd_and_hms(2026, 7, 16, 4, 35, 0)
                .single()
                .expect("valid bucket start")
                .timestamp(),
        )
        .bind(SOURCE_PROXY)
        .bind("api.openai.com")
        .bind(42_i64)
        .bind(128_i64)
        .bind(256_i64)
        .execute(&pool)
        .await
        .expect("insert dashboard socket minute row");

        let rows = query_dashboard_network_bucket_rows(
            &pool,
            InvocationSourceScope::ProxyOnly,
            ExactUtcRange {
                start: Utc
                    .with_ymd_and_hms(2026, 7, 16, 4, 34, 0)
                    .single()
                    .expect("valid range start"),
                end: Utc
                    .with_ymd_and_hms(2026, 7, 16, 4, 40, 0)
                    .single()
                    .expect("valid range end"),
            },
            Some(Some(42)),
            None,
        )
        .await
        .expect("query dashboard socket minute rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].upload_bytes, 128);
        assert_eq!(rows[0].download_bytes, 256);
    }

    #[tokio::test]
    async fn dashboard_network_host_minute_rows_roll_up_into_five_minute_buckets() {
        let pool = network_pool().await;
        let bucket_start_epoch = Utc
            .with_ymd_and_hms(2026, 7, 16, 4, 35, 0)
            .single()
            .expect("valid bucket start")
            .timestamp();
        sqlx::query(
            r#"
            INSERT INTO upstream_socket_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id,
                upload_bytes,
                download_bytes
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6), (?7, ?8, ?9, ?10, ?11, ?12)
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(SOURCE_PROXY)
        .bind("api.openai.com")
        .bind(Some(42_i64))
        .bind(100_i64)
        .bind(200_i64)
        .bind(bucket_start_epoch + 60)
        .bind(SOURCE_PROXY)
        .bind("backup.openai.com")
        .bind(Some(77_i64))
        .bind(40_i64)
        .bind(80_i64)
        .execute(&pool)
        .await
        .expect("insert upstream socket minute rows");

        let rows = query_dashboard_network_host_minute_bucket_rows(
            &pool,
            InvocationSourceScope::ProxyOnly,
            ExactUtcRange {
                start: Utc
                    .timestamp_opt(bucket_start_epoch, 0)
                    .single()
                    .expect("valid start"),
                end: Utc
                    .timestamp_opt(bucket_start_epoch + 300, 0)
                    .single()
                    .expect("valid end"),
            },
        )
        .await
        .expect("query socket minute network rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bucket_start_epoch_second, bucket_start_epoch);
        assert_eq!(rows[0].upload_bytes, 140);
        assert_eq!(rows[0].download_bytes, 280);
    }

    #[test]
    fn dashboard_network_realtime_rate_response_serializes_complete_second_snapshot() {
        let response =
            build_dashboard_network_realtime_rate_response(DashboardNetworkRealtimeByteSnapshot {
                sample_start_epoch_second: Utc
                    .with_ymd_and_hms(2026, 7, 19, 18, 3, 59)
                    .single()
                    .expect("valid sample start")
                    .timestamp(),
                sample_end_epoch_second: Utc
                    .with_ymd_and_hms(2026, 7, 19, 18, 4, 0)
                    .single()
                    .expect("valid sample end")
                    .timestamp(),
                sample_seconds: 1,
                totals: DashboardNetworkByteTotals {
                    upload_bytes: 2048,
                    download_bytes: 4096,
                },
            });

        assert_eq!(response.sample_start, "2026-07-19T18:03:59Z");
        assert_eq!(response.sample_end, "2026-07-19T18:04:00Z");
        assert_eq!(response.sample_seconds, 1);
        assert_eq!(response.upload_bytes_per_second, 2048.0);
        assert_eq!(response.download_bytes_per_second, 4096.0);

        let payload =
            serde_json::to_value(&response).expect("serialize dashboard network realtime rate");
        assert_eq!(
            payload,
            json!({
                "sampleStart": "2026-07-19T18:03:59Z",
                "sampleEnd": "2026-07-19T18:04:00Z",
                "sampleSeconds": 1,
                "uploadBytesPerSecond": 2048.0,
                "downloadBytesPerSecond": 4096.0,
                "uploadBytes": 2048,
                "downloadBytes": 4096
            })
        );
    }
}
