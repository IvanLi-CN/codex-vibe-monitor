use super::*;

#[tokio::test]
async fn backfill_invocation_service_tiers_revisits_inline_proxy_auto_tiers_without_raw_files() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-inline-service-tier-backfill")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses","serviceTier":"auto"}"#)
    .bind(r#"{"service_tier":"default"}"#)
    .execute(&pool)
    .await
    .expect("insert inline proxy service tier row");

    let summary_first = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("inline service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-inline-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query inline proxy payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode inline payload JSON");
    assert_eq!(payload_json["serviceTier"], "default");
    assert_eq!(
        payload_json["serviceTierBackfillVersion"],
        "stream-terminal-v1"
    );

    let summary_second = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("inline service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);
}

#[tokio::test]
async fn backfill_invocation_service_tiers_revisits_inline_proxy_non_auto_stream_tiers() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-inline-non-auto-service-tier-backfill")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses","serviceTier":"priority"}"#)
    .bind(
        [
            "event: response.created",
            r#"data: {"type":"response.created","response":{"service_tier":"priority"}}"#,
            "",
            "event: response.completed",
            r#"data: {"type":"response.completed","response":{"service_tier":"default"}}"#,
            "",
        ]
        .join("\n"),
    )
    .execute(&pool)
    .await
    .expect("insert inline proxy non-auto service tier row");

    let summary_first = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("inline non-auto service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-inline-non-auto-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query inline proxy non-auto payload");
    let payload_json: Value =
        serde_json::from_str(&payload).expect("decode inline non-auto payload JSON");
    assert_eq!(payload_json["serviceTier"], "default");
    assert_eq!(
        payload_json["serviceTierBackfillVersion"],
        "stream-terminal-v1"
    );

    let summary_second = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("inline non-auto service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);
}

#[tokio::test]
async fn backfill_invocation_service_tiers_tracks_skip_counters() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("service-tier-missing")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_XY)
    .bind("success")
    .bind("{}")
    .bind(r#"{"status":"success"}"#)
    .execute(&pool)
    .await
    .expect("insert missing tier row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("service-tier-missing-file")
    .bind("2026-02-23 00:00:01")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses"}"#)
    .bind("{}")
    .bind("/tmp/does-not-exist-response.bin")
    .execute(&pool)
    .await
    .expect("insert missing file row");

    let summary = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("service tier backfill skip run should succeed");
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_missing_tier, 1);
}
