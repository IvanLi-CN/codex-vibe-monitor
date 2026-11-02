use super::super::*;
#[tokio::test]
async fn failure_classification_backfill_skips_success_rows_with_complete_defaults() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            failure_class,
            is_actionable,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("success-no-kind")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(FAILURE_CLASS_NONE)
    .bind(0_i64)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert success row");

    let outcome = backfill_failure_classification_from_cursor(&pool, 0, None, Some(10), None)
        .await
        .expect("run failure classification backfill");
    assert_eq!(outcome.summary.scanned, 0);
    assert_eq!(outcome.summary.updated, 0);
    assert_eq!(outcome.next_cursor_id, 0);
    assert!(!outcome.hit_budget);
}

#[tokio::test]
async fn failure_classification_backfill_recovers_response_failed_records() {
    #[derive(sqlx::FromRow)]
    struct BackfilledRow {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
        payload: Option<String>,
    }

    let pool = test_current_schema_pool().await;

    let raw_response = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "event: response.failed",
        r#"data: {"type":"response.failed","response":{"id":"resp_test","model":"gpt-5.4","status":"failed","error":{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}}"#,
    ]
    .join("\n");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            failure_class,
            is_actionable,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("response-failed-success")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(FAILURE_CLASS_NONE)
    .bind(0_i64)
    .bind(r#"{"endpoint":"/v1/responses","isStream":true,"usageMissingReason":"usage_missing_in_stream"}"#)
    .bind(&raw_response)
    .execute(&pool)
    .await
    .expect("insert misrecorded success row");

    let outcome = backfill_failure_classification_from_cursor(&pool, 0, None, Some(10), None)
        .await
        .expect("run failure classification backfill");
    assert_eq!(outcome.summary.scanned, 1);
    assert_eq!(outcome.summary.updated, 1);

    let row = sqlx::query_as::<_, BackfilledRow>(
        r#"
        SELECT status, error_message, failure_kind, failure_class, is_actionable, payload
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("response-failed-success")
    .fetch_one(&pool)
    .await
    .expect("load backfilled row");

    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_response_failed] server_error"))
    );
    assert_eq!(
        row.failure_kind.as_deref(),
        Some("upstream_response_failed")
    );
    assert_eq!(row.failure_class.as_deref(), Some("service_failure"));
    assert_eq!(row.is_actionable, Some(1));

    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("payload should still be present"),
    )
    .expect("decode payload json");
    assert_eq!(
        payload_json["streamTerminalEvent"].as_str(),
        Some("response.failed")
    );
    assert_eq!(
        payload_json["upstreamErrorCode"].as_str(),
        Some("server_error")
    );
    assert_eq!(
        payload_json["upstreamRequestId"].as_str(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );
}

#[tokio::test]
async fn failure_classification_backfill_reads_long_stream_failures_from_raw_file() {
    #[derive(sqlx::FromRow)]
    struct BackfilledRow {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        payload: Option<String>,
    }

    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let temp_dir = make_temp_test_dir("response-failed-backfill");
    let response_path = temp_dir.join("response.bin");
    let long_prefix = format!(
        r#"event: response.created
data: {{"type":"response.output_text.delta","delta":"{}"}}

"#,
        "x".repeat(16_400)
    );
    let raw_file = format!(
        r#"{}event: response.failed
data: {{"type":"response.failed","response":{{"id":"resp_test","model":"gpt-5.4","status":"failed","error":{{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}}}}}
"#,
        long_prefix,
    );
    fs::write(&response_path, raw_file.as_bytes()).expect("write response raw file");

    let preview = build_raw_response_preview(raw_file.as_bytes());
    assert!(!preview.contains("response.failed"));

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            failure_class,
            is_actionable,
            payload,
            raw_response,
            response_raw_path,
            response_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("response-failed-from-file")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(FAILURE_CLASS_NONE)
    .bind(0_i64)
    .bind(r#"{"endpoint":"/v1/responses","isStream":true}"#)
    .bind(&preview)
    .bind(response_path.to_string_lossy().to_string())
    .bind(raw_file.len() as i64)
    .execute(&pool)
    .await
    .expect("insert long success row");

    let outcome = backfill_failure_classification_from_cursor(&pool, 0, None, Some(10), None)
        .await
        .expect("run failure classification backfill");
    assert_eq!(outcome.summary.scanned, 1);
    assert_eq!(outcome.summary.updated, 1);

    let row = sqlx::query_as::<_, BackfilledRow>(
        r#"
        SELECT status, error_message, failure_kind, payload
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("response-failed-from-file")
    .fetch_one(&pool)
    .await
    .expect("load backfilled row");

    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_response_failed] server_error"))
    );
    assert_eq!(
        row.failure_kind.as_deref(),
        Some("upstream_response_failed")
    );

    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("payload should still be present"),
    )
    .expect("decode payload json");
    assert_eq!(
        payload_json["upstreamRequestId"].as_str(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );

    fs::remove_dir_all(&temp_dir).expect("cleanup temp dir");
}

#[tokio::test]
async fn failure_classification_backfill_from_cursor_respects_scan_limit() {
    let pool = test_current_schema_pool().await;

    for idx in 0..205 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(format!("failure-classification-{idx}"))
        .bind("2026-03-09 00:00:00")
        .bind(SOURCE_PROXY)
        .bind("http_500")
        .bind("boom")
        .bind("{}")
        .execute(&pool)
        .await
        .expect("insert failure classification row");
    }

    let first = backfill_failure_classification_from_cursor(&pool, 0, None, Some(200), None)
        .await
        .expect("first bounded failure classification pass");
    assert_eq!(first.summary.scanned, 200);
    assert_eq!(first.summary.updated, 200);
    assert!(first.hit_budget);
    assert!(first.next_cursor_id > 0);

    let second = backfill_failure_classification_from_cursor(
        &pool,
        first.next_cursor_id,
        None,
        Some(200),
        None,
    )
    .await
    .expect("second bounded failure classification pass");
    assert_eq!(second.summary.scanned, 5);
    assert_eq!(second.summary.updated, 5);
    assert!(!second.hit_budget);
}
