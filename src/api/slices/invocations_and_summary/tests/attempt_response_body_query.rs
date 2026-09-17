#[cfg(test)]
mod attempt_response_body_query_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn attempt_response_body_query_pool() -> sqlx::SqlitePool {
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
                payload TEXT,
                raw_response TEXT NOT NULL DEFAULT '',
                request_raw_path TEXT,
                request_raw_size INTEGER,
                request_raw_truncated INTEGER,
                request_raw_truncated_reason TEXT,
                response_raw_path TEXT,
                response_raw_size INTEGER,
                response_raw_truncated INTEGER,
                response_raw_truncated_reason TEXT,
                detail_level TEXT NOT NULL,
                detail_prune_reason TEXT,
                failure_class TEXT,
                status TEXT,
                error_message TEXT,
                failure_kind TEXT
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
                attempt_index INTEGER NOT NULL,
                status TEXT,
                response_raw_path TEXT,
                response_raw_size INTEGER,
                response_raw_truncated INTEGER,
                response_raw_truncated_reason TEXT,
                response_content_encoding TEXT,
                upstream_request_id TEXT,
                attempt_public_id TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create pool_upstream_request_attempts table");
        pool
    }

    async fn seed_attempt_response_body_query_fixture(pool: &sqlx::SqlitePool) -> String {
        let response_body = "x".repeat(71);
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, raw_response, response_raw_size,
                detail_level, status, error_message, failure_kind
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(42_i64)
        .bind("fixture-invocation")
        .bind("2026-07-28 00:26:54")
        .bind(&response_body)
        .bind(71_i64)
        .bind("full")
        .bind("http_502")
        .bind("upstream returned 502")
        .bind("upstream_http_5xx")
        .execute(pool)
        .await
        .expect("insert invocation");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                id, invoke_id, occurred_at, attempt_index, status, attempt_public_id
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(7_i64)
        .bind("fixture-invocation")
        .bind("2026-07-28 00:26:54")
        .bind(1_i64)
        .bind("http_failure")
        .bind("pp8dDsSb")
        .execute(pool)
        .await
        .expect("insert attempt");
        response_body
    }

    #[tokio::test]
    async fn attempt_response_body_query_qualifies_invocation_failure_columns() {
        let pool = attempt_response_body_query_pool().await;
        let response_body = seed_attempt_response_body_query_fixture(&pool).await;

        let row = fetch_invocation_attempt_response_body_row(&pool, 42, "pp8dDsSb")
            .await
            .expect("load attempt response body")
            .expect("attempt response body row");

        assert_eq!(row.failure_class.as_deref(), Some("service_failure"));
        let response =
            build_response_body_response_with_source(&row, None, Some("attempt_raw_file"));
        assert!(response.available);
        assert_eq!(response.body_text.as_deref(), Some(response_body.as_str()));
        assert_eq!(response.body_size, Some(71));
    }
}
