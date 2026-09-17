#[test]
pub(crate) fn decode_response_payload_for_usage_decompresses_stacked_content_encodings() {
    let raw = br#"{"usage":{"input_tokens":21,"output_tokens":8,"total_tokens":29}}"#;
    let mut gzip_encoder = GzEncoder::new(Vec::new(), Compression::default());
    gzip_encoder
        .write_all(raw)
        .expect("write stacked gzip payload");
    let gzip_compressed = gzip_encoder.finish().expect("finish stacked gzip payload");
    let stacked = encode_brotli_payload(&gzip_compressed);

    let (decoded, decode_error) = decode_response_payload_for_usage(&stacked, Some("gzip, br"));
    assert!(decode_error.is_none());
    assert_eq!(decoded.as_ref(), raw);
}

#[tokio::test]
pub(crate) async fn backfill_proxy_prompt_cache_keys_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload(&request_path, Some("pck-backfill-1"));

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-1",
        &request_path,
        "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\",\"codexSessionId\":\"legacy-session-1\"}",
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-ready",
        &request_path,
        "{\"endpoint\":\"/v1/responses\",\"promptCacheKey\":\"already-present\"}",
    )
    .await;

    let summary_first = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("first prompt cache key backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_invalid_json, 0);
    assert_eq!(summary_first.skipped_missing_key, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-pck-backfill-1")
            .fetch_one(&pool)
            .await
            .expect("query backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["promptCacheKey"], "pck-backfill-1");
    assert!(
        payload_json.get("codexSessionId").is_none(),
        "legacy codexSessionId key should be removed during backfill"
    );

    let summary_second = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("second prompt cache key backfill should succeed");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn sync_hourly_rollups_rebuilds_after_prompt_cache_key_backfill_updates_existing_rows()
 {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("hourly-rollup-rebuild-after-prompt-cache-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload(&request_path, Some("pck-rollup-rebuild"));

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-rollup-rebuild",
        &request_path,
        r#"{"endpoint":"/v1/responses","requesterIp":"198.51.100.77"}"#,
    )
    .await;

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("initial hourly rollup bootstrap should succeed");

    let initial_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_rollup_hourly WHERE prompt_cache_key = ?1",
    )
    .bind("pck-rollup-rebuild")
    .fetch_one(&pool)
    .await
    .expect("query initial prompt cache rollup count");
    assert_eq!(initial_count, 0);

    let summary = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("prompt cache key backfill should succeed");
    assert_eq!(summary.updated, 1);

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("hourly rollup sync should rebuild invocation-backed rollups");

    let request_count: i64 = sqlx::query_scalar(
        "SELECT request_count FROM prompt_cache_rollup_hourly WHERE prompt_cache_key = ?1",
    )
    .bind("pck-rollup-rebuild")
    .fetch_one(&pool)
    .await
    .expect("query rebuilt prompt cache rollup row");
    assert_eq!(request_count, 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn backfill_proxy_prompt_cache_keys_tracks_skip_counters() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill-skips");
    let ok_request_path = temp_dir.join("request-ok.json");
    let missing_key_request_path = temp_dir.join("request-missing-key.json");
    let invalid_json_request_path = temp_dir.join("request-invalid-json.json");
    let missing_file_request_path = temp_dir.join("request-missing.json");

    write_backfill_request_payload(&ok_request_path, Some("pck-backfill-ok"));
    write_backfill_request_payload(&missing_key_request_path, None);
    fs::write(&invalid_json_request_path, b"not-json").expect("write invalid request payload");

    let base_payload = "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\"}";
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-ok",
        &ok_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-missing-file",
        &missing_file_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-invalid-json",
        &invalid_json_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-missing-key",
        &missing_key_request_path,
        base_payload,
    )
    .await;

    let summary = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("prompt cache key backfill should succeed");
    assert_eq!(summary.scanned, 4);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_invalid_json, 1);
    assert_eq!(summary.skipped_missing_key, 1);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-pck-backfill-ok")
            .fetch_one(&pool)
            .await
            .expect("query backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["promptCacheKey"], "pck-backfill-ok");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn backfill_proxy_requested_service_tiers_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-requested-service-tier-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload_with_requested_service_tier(
        &request_path,
        Some("priority"),
        ProxyCaptureTarget::Responses,
    );

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-backfill-1",
        &request_path,
        r#"{"endpoint":"/v1/responses"}"#,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-backfill-ready",
        &request_path,
        r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority"}"#,
    )
    .await;

    let summary_first = backfill_proxy_requested_service_tiers(&pool, None)
        .await
        .expect("first requested service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_invalid_json, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-requested-tier-backfill-1")
            .fetch_one(&pool)
            .await
            .expect("query backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["requestedServiceTier"], "priority");

    let summary_second = backfill_proxy_requested_service_tiers(&pool, None)
        .await
        .expect("second requested service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn backfill_proxy_requested_service_tiers_tracks_skip_counters() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-requested-service-tier-backfill-skips");
    let missing_tier_request_path = temp_dir.join("request-missing-tier.json");
    let invalid_json_request_path = temp_dir.join("request-invalid-json.json");
    let missing_file_request_path = temp_dir.join("request-missing.json");

    write_backfill_request_payload_with_requested_service_tier(
        &missing_tier_request_path,
        None,
        ProxyCaptureTarget::Responses,
    );
    fs::write(&invalid_json_request_path, b"not-json").expect("write invalid request payload");

    let base_payload = r#"{"endpoint":"/v1/responses"}"#;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-missing-file",
        &missing_file_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-invalid-json",
        &invalid_json_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-missing-tier",
        &missing_tier_request_path,
        base_payload,
    )
    .await;

    let summary = backfill_proxy_requested_service_tiers(&pool, None)
        .await
        .expect("requested service tier backfill should succeed");
    assert_eq!(summary.scanned, 3);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_invalid_json, 1);
    assert_eq!(summary.skipped_missing_tier, 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn backfill_proxy_reasoning_efforts_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-reasoning-effort-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload_with_reasoning(
        &request_path,
        Some("pck-reasoning"),
        Some("high"),
        ProxyCaptureTarget::Responses,
    );

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-backfill-1",
        &request_path,
        r#"{"endpoint":"/v1/responses","requesterIp":"198.51.100.77"}"#,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-backfill-ready",
        &request_path,
        r#"{"endpoint":"/v1/responses","reasoningEffort":"medium"}"#,
    )
    .await;

    let summary_first = backfill_proxy_reasoning_efforts(&pool, None)
        .await
        .expect("first reasoning effort backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_invalid_json, 0);
    assert_eq!(summary_first.skipped_missing_effort, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-reasoning-backfill-1")
            .fetch_one(&pool)
            .await
            .expect("query reasoning backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["reasoningEffort"], "high");

    let summary_second = backfill_proxy_reasoning_efforts(&pool, None)
        .await
        .expect("second reasoning effort backfill should succeed");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn backfill_proxy_reasoning_efforts_tracks_skip_counters_and_chat_payloads() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-reasoning-effort-backfill-skips");
    let ok_chat_path = temp_dir.join("request-chat-ok.json");
    let missing_effort_path = temp_dir.join("request-missing-effort.json");
    let invalid_json_path = temp_dir.join("request-invalid-json.json");
    let missing_file_path = temp_dir.join("request-missing.json");

    write_backfill_request_payload_with_reasoning(
        &ok_chat_path,
        None,
        Some("medium"),
        ProxyCaptureTarget::ChatCompletions,
    );
    write_backfill_request_payload_with_reasoning(
        &missing_effort_path,
        None,
        None,
        ProxyCaptureTarget::Responses,
    );
    fs::write(&invalid_json_path, b"not-json").expect("write invalid request payload");

    let base_payload = r#"{"endpoint":"/v1/chat/completions"}"#;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-chat-ok",
        &ok_chat_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-missing-file",
        &missing_file_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-invalid-json",
        &invalid_json_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-missing-effort",
        &missing_effort_path,
        r#"{"endpoint":"/v1/responses"}"#,
    )
    .await;

    let summary = backfill_proxy_reasoning_efforts(&pool, None)
        .await
        .expect("reasoning effort backfill should succeed");
    assert_eq!(summary.scanned, 4);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_invalid_json, 1);
    assert_eq!(summary.skipped_missing_effort, 1);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-reasoning-chat-ok")
            .fetch_one(&pool)
            .await
            .expect("query chat reasoning payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["reasoningEffort"], "medium");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn backfill_proxy_prompt_cache_keys_reads_from_fallback_root_for_relative_paths() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill-fallback");
    let fallback_root = temp_dir.join("legacy-root");
    let relative_path = PathBuf::from("proxy_raw_payloads/request-fallback.json");
    let request_path = fallback_root.join(&relative_path);
    let request_dir = request_path.parent().expect("request parent dir");
    fs::create_dir_all(request_dir).expect("create fallback request dir");
    write_backfill_request_payload(&request_path, Some("pck-fallback-1"));

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-fallback",
        &relative_path,
        "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\"}",
    )
    .await;

    let summary = backfill_proxy_prompt_cache_keys(&pool, Some(&fallback_root))
        .await
        .expect("prompt cache key backfill with fallback root should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 0);
    assert_eq!(summary.skipped_invalid_json, 0);
    assert_eq!(summary.skipped_missing_key, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-pck-backfill-fallback")
            .fetch_one(&pool)
            .await
            .expect("query fallback-backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["promptCacheKey"], "pck-fallback-1");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
pub(crate) async fn backfill_invocation_service_tiers_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("invocation-service-tier-backfill");
    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload_with_terminal_service_tier(
        &response_path,
        Some("auto"),
        Some("default"),
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("quota-service-tier-backfill")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_XY)
    .bind("success")
    .bind("{}")
    .bind(r#"{"service_tier":"priority"}"#)
    .execute(&pool)
    .await
    .expect("insert quota service tier row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("proxy-service-tier-backfill")
    .bind("2026-02-23 00:00:01")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses"}"#)
    .bind("{}")
    .bind(response_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("insert proxy service tier row");

    let summary_first = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("first service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 2);
    assert_eq!(summary_first.updated, 2);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let quota_payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("quota-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query quota payload");
    let quota_payload_json: Value =
        serde_json::from_str(&quota_payload).expect("decode quota payload JSON");
    assert_eq!(quota_payload_json["serviceTier"], "priority");

    let proxy_payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query proxy payload");
    let proxy_payload_json: Value =
        serde_json::from_str(&proxy_payload).expect("decode proxy payload JSON");
    assert_eq!(proxy_payload_json["serviceTier"], "default");
    assert_eq!(
        proxy_payload_json["serviceTierBackfillVersion"],
        "stream-terminal-v1"
    );

    let summary_second = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("second service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

use super::*;
