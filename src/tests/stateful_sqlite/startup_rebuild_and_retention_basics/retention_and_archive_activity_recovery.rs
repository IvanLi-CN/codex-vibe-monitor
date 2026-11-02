use super::super::*;
#[tokio::test]
async fn retention_prunes_old_success_invocation_details_and_sweeps_orphans() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-prune").await;
    let response_raw = config.proxy_raw_dir.join("old-success-response.bin");
    fs::write(&response_raw, b"response-body").expect("write response raw");
    let request_missing = config.proxy_raw_dir.join("old-success-request.bin");
    let orphan = config.proxy_raw_dir.join("orphan.bin");
    fs::write(&orphan, b"orphan").expect("write orphan raw");
    set_file_mtime_seconds_ago(&orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
    let occurred_at = shanghai_local_days_ago(31, 12, 0, 0);

    insert_retention_invocation(
        &pool,
        "old-success",
        &occurred_at,
        SOURCE_XY,
        "success",
        Some(
            "{\"endpoint\":\"/v1/responses\",\"promptCacheKey\":\"detail-prune-key\",\"requestCompressionAlgorithm\":\"zstd\"}",
        ),
        "{\"ok\":true}",
        Some(&request_missing),
        Some(&response_raw),
        Some(321),
        Some(1.23),
    )
    .await;

    crate::ensure_prompt_cache_conversation_row(&pool, "detail-prune-key")
        .await
        .expect("create detail-prune prompt-cache conversation");
    let prompt_cache_keys = std::collections::HashSet::from(["detail-prune-key".to_owned()]);
    crate::refresh_prompt_cache_conversation_stats(&pool, &prompt_cache_keys)
        .await
        .expect("seed detail-prune prompt-cache statistics");

    let before_pruned_at = Utc::now() - ChronoDuration::seconds(5);
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention prune");
    let after_pruned_at = Utc::now() + ChronoDuration::seconds(5);
    assert_eq!(summary.invocation_details_pruned, 1);
    assert_eq!(summary.archive_batches_touched, 1);
    assert_eq!(summary.raw_files_removed, 1);
    assert_eq!(summary.orphan_raw_files_removed, 0);
    assert!(!response_raw.exists());
    assert!(orphan.exists());

    let row = sqlx::query(
        r#"
        SELECT
            payload,
            raw_response,
            request_raw_path,
            response_raw_path,
            detail_level,
            detail_pruned_at,
            detail_prune_reason,
            total_tokens,
            cost,
            status
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("old-success")
    .fetch_one(&pool)
    .await
    .expect("load pruned invocation");
    assert_eq!(
        row.get::<String, _>("detail_level"),
        DETAIL_LEVEL_STRUCTURED_ONLY
    );
    assert!(row.get::<Option<String>, _>("detail_pruned_at").is_some());
    assert_eq!(
        row.get::<Option<String>, _>("detail_prune_reason")
            .as_deref(),
        Some(DETAIL_PRUNE_REASON_SUCCESS_OVER_30D)
    );
    let payload: Value = serde_json::from_str(
        row.get::<Option<String>, _>("payload")
            .as_deref()
            .expect("request compression metadata should be retained"),
    )
    .expect("decode retained payload");
    assert_eq!(
        payload["requestCompressionAlgorithm"].as_str(),
        Some("zstd")
    );
    assert!(payload.get("endpoint").is_none());
    assert_eq!(row.get::<String, _>("raw_response"), "");
    assert!(row.get::<Option<String>, _>("request_raw_path").is_none());
    assert!(row.get::<Option<String>, _>("response_raw_path").is_none());
    assert_eq!(row.get::<Option<i64>, _>("total_tokens"), Some(321));
    assert_f64_close(row.get::<Option<f64>, _>("cost").unwrap_or_default(), 1.23);
    assert_eq!(
        row.get::<Option<String>, _>("status").as_deref(),
        Some("success")
    );

    let prompt_cache_stats: (i64, i64, f64) = sqlx::query_as(
        "SELECT request_count, total_tokens, cost FROM prompt_cache_conversations WHERE prompt_cache_key = 'detail-prune-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("load detail-prune prompt-cache statistics");
    // Retention invalidates the materialized snapshot and queues the owner for an
    // asynchronous bounded refresh. The old snapshot remains intact until that
    // refresh publishes an exact replacement.
    assert_eq!(prompt_cache_stats.0, 1);
    assert_eq!(prompt_cache_stats.1, 321);
    assert_f64_close(prompt_cache_stats.2, 1.23);
    let refresh_queue_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_stats_refresh_queue WHERE prompt_cache_key = 'detail-prune-key'",
    )
    .fetch_one(&pool)
    .await
    .expect("count detail-prune prompt-cache refresh queue");
    assert_eq!(refresh_queue_count, 1);

    let detail_pruned_at = row
        .get::<Option<String>, _>("detail_pruned_at")
        .expect("detail_pruned_at should be populated");
    let detail_pruned_at = local_naive_to_utc(
        parse_shanghai_local_naive(&detail_pruned_at)
            .expect("detail_pruned_at should be shanghai-local"),
        Shanghai,
    )
    .with_timezone(&Utc);
    assert!(detail_pruned_at >= before_pruned_at);
    assert!(detail_pruned_at <= after_pruned_at);

    let batch = sqlx::query(
        r#"
        SELECT file_path, row_count, status, summary_source_kind
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load prune archive batch");
    let file_path = PathBuf::from(batch.get::<String, _>("file_path"));
    assert!(file_path.exists());
    assert!(
        file_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("live-mirror")),
        "detail-prune archives must use a mirror-only path"
    );
    assert_eq!(batch.get::<String, _>("status"), ARCHIVE_STATUS_COMPLETED);
    assert_eq!(
        batch.get::<String, _>("summary_source_kind"),
        SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR
    );
    assert_eq!(batch.get::<i64, _>("row_count"), 1);
    let prepared_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM retention_prepared_archives WHERE dataset = 'codex_invocations'",
    )
    .fetch_one(&pool)
    .await
    .expect("count detail-prune recovery ledger rows");
    assert_eq!(
        prepared_count, 0,
        "successful detail prune retires its ledger"
    );
    assert!(
        crate::stats::load_completed_invocation_archive_paths(&pool)
            .await
            .expect("load Summary archive sources")
            .is_empty(),
        "a detail-prune archive duplicates a retained live row and must not become a Summary source"
    );

    let archive_db_path = temp_dir.join("retention-prune-archive.sqlite");
    inflate_gzip_sqlite_file(&file_path, &archive_db_path).expect("inflate prune archive");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open prune archive sqlite");
    let archive_columns: HashSet<String> = sqlx::query("PRAGMA table_info('codex_invocations')")
        .fetch_all(&archive_pool)
        .await
        .expect("inspect prune archive schema")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(
        !archive_columns.contains("raw_expires_at"),
        "new archive batches should not carry raw_expires_at anymore"
    );
    let archived = sqlx::query(
        r#"
        SELECT payload, raw_response, detail_level, detail_pruned_at, detail_prune_reason
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("old-success")
    .fetch_one(&archive_pool)
    .await
    .expect("load archived pre-prune invocation");
    assert_eq!(
        archived.get::<Option<String>, _>("payload").as_deref(),
        Some(
            "{\"endpoint\":\"/v1/responses\",\"promptCacheKey\":\"detail-prune-key\",\"requestCompressionAlgorithm\":\"zstd\"}",
        )
    );
    assert_eq!(archived.get::<String, _>("raw_response"), "{\"ok\":true}");
    assert_eq!(archived.get::<String, _>("detail_level"), DETAIL_LEVEL_FULL);
    assert!(
        archived
            .get::<Option<String>, _>("detail_pruned_at")
            .is_none()
    );
    assert!(
        archived
            .get::<Option<String>, _>("detail_prune_reason")
            .is_none()
    );
    archive_pool.close().await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_preserves_long_term_model_fields_when_pruning_payload() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-prune-long-term-model-fields").await;
    let occurred_at = shanghai_local_days_ago(31, 12, 30, 0);
    insert_retention_invocation(
        &pool,
        "old-model-fields",
        &occurred_at,
        SOURCE_XY,
        "success",
        Some(
            r#"{"upstreamAccountId":771,"requestModel":"gpt-5.4","responseModel":"gpt-5.4-routing","reasoningEffort":"high"}"#,
        ),
        "{}",
        None,
        None,
        Some(321),
        Some(1.23),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "old-model-fields-no-upstream",
        &occurred_at,
        SOURCE_XY,
        "success",
        Some(r#"{"requestModel":"gpt-5.4","responseModel":"gpt-5.4-routing","reasoningEffort":"high"}"#),
        "{}",
        None,
        None,
        Some(123),
        Some(0.45),
    )
    .await;

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention prune for long-term model fields");

    let payload: Option<String> =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("old-model-fields")
            .fetch_one(&pool)
            .await
            .expect("load pruned long-term payload");
    let payload = serde_json::from_str::<serde_json::Value>(
        payload
            .as_deref()
            .expect("model fields should remain in payload"),
    )
    .expect("decode pruned long-term payload");
    assert_eq!(payload["upstreamAccountId"].as_i64(), Some(771));
    assert_eq!(payload["requestModel"].as_str(), Some("gpt-5.4"));
    assert_eq!(payload["responseModel"].as_str(), Some("gpt-5.4-routing"));
    assert_eq!(payload["reasoningEffort"].as_str(), Some("high"));

    let payload: Option<String> =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("old-model-fields-no-upstream")
            .fetch_one(&pool)
            .await
            .expect("load pruned model-only payload");
    let payload = serde_json::from_str::<serde_json::Value>(
        payload
            .as_deref()
            .expect("model-only fields should remain in payload"),
    )
    .expect("decode pruned model-only payload");
    assert_eq!(payload["requestModel"].as_str(), Some("gpt-5.4"));
    assert_eq!(payload["responseModel"].as_str(), Some("gpt-5.4-routing"));
    assert_eq!(payload["reasoningEffort"].as_str(), Some("high"));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_prunes_old_legacy_http_200_success_like_invocation_details() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-prune-legacy-http200").await;
    let response_raw = config.proxy_raw_dir.join("legacy-http200-response.bin");
    fs::write(&response_raw, b"legacy-http200-response").expect("write legacy http_200 raw");
    let occurred_at = shanghai_local_days_ago(31, 13, 0, 0);

    insert_retention_invocation(
        &pool,
        "old-legacy-http200-success-like",
        &occurred_at,
        SOURCE_PROXY,
        "http_200",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        Some(&response_raw),
        Some(456),
        Some(1.78),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention prune for legacy http_200 success-like row");
    assert_eq!(summary.invocation_details_pruned, 1);
    assert_eq!(summary.archive_batches_touched, 1);
    assert_eq!(summary.raw_files_removed, 1);
    assert!(!response_raw.exists());

    let row = sqlx::query(
        r#"
        SELECT
            detail_level,
            detail_prune_reason,
            request_raw_path,
            response_raw_path,
            status,
            error_message
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("old-legacy-http200-success-like")
    .fetch_one(&pool)
    .await
    .expect("load pruned legacy http_200 invocation");
    assert_eq!(
        row.get::<String, _>("detail_level"),
        DETAIL_LEVEL_STRUCTURED_ONLY
    );
    assert_eq!(
        row.get::<Option<String>, _>("detail_prune_reason")
            .as_deref(),
        Some(DETAIL_PRUNE_REASON_SUCCESS_OVER_30D)
    );
    assert!(row.get::<Option<String>, _>("request_raw_path").is_none());
    assert!(row.get::<Option<String>, _>("response_raw_path").is_none());
    assert_eq!(
        row.get::<Option<String>, _>("status").as_deref(),
        Some("http_200")
    );
    assert!(row.get::<Option<String>, _>("error_message").is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_does_not_prune_legacy_http_200_rows_with_error_message() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-prune-legacy-http200-error").await;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let response_raw = config
        .proxy_raw_dir
        .join("legacy-http200-error-response.bin");
    fs::write(&response_raw, b"legacy-http200-error-response")
        .expect("write legacy http_200 error raw");
    let occurred_at = shanghai_local_days_ago(31, 14, 0, 0);

    insert_retention_invocation(
        &pool,
        "old-legacy-http200-error",
        &occurred_at,
        SOURCE_PROXY,
        "http_200",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":false}",
        None,
        Some(&response_raw),
        Some(654),
        Some(2.34),
    )
    .await;
    sqlx::query("UPDATE codex_invocations SET error_message = ?1 WHERE invoke_id = ?2")
        .bind("[upstream_response_failed] server_error")
        .bind("old-legacy-http200-error")
        .execute(&pool)
        .await
        .expect("attach error message to legacy http_200 row");

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention for legacy http_200 error row");
    assert_eq!(summary.invocation_details_pruned, 0);
    assert_eq!(summary.raw_files_removed, 0);
    assert!(response_raw.exists());

    let row = sqlx::query(
        r#"
        SELECT detail_level, response_raw_path, status, error_message
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("old-legacy-http200-error")
    .fetch_one(&pool)
    .await
    .expect("load unpruned legacy http_200 error row");
    assert_eq!(row.get::<String, _>("detail_level"), DETAIL_LEVEL_FULL);
    assert_eq!(
        row.get::<Option<String>, _>("response_raw_path").as_deref(),
        Some(response_raw.to_string_lossy().as_ref())
    );
    assert_eq!(
        row.get::<Option<String>, _>("status").as_deref(),
        Some("http_200")
    );
    assert_eq!(
        row.get::<Option<String>, _>("error_message").as_deref(),
        Some("[upstream_response_failed] server_error")
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_prioritizes_expired_archive_before_detail_prune() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-archive-priority").await;
    let detail_occurred_at = shanghai_local_days_ago(31, 14, 0, 0);
    let archive_occurred_at = shanghai_local_days_ago(91, 15, 0, 0);

    insert_retention_invocation(
        &pool,
        "detail-priority-candidate",
        &detail_occurred_at,
        SOURCE_XY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(10),
        Some(0.1),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "archive-priority-candidate",
        &archive_occurred_at,
        SOURCE_XY,
        "failed",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":false}",
        None,
        None,
        Some(20),
        Some(0.2),
    )
    .await;

    let stage_trace = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let summary = crate::maintenance::RETENTION_TEST_INVOCATION_STAGE_TRACE
        .scope(
            stage_trace.clone(),
            run_data_retention_maintenance(&pool, &config, Some(false), None),
        )
        .await
        .expect("run retention with mixed invocation backlog");

    assert_eq!(summary.invocation_rows_archived, 1);
    assert_eq!(summary.invocation_details_pruned, 1);
    assert_eq!(
        stage_trace
            .lock()
            .expect("load retention stage trace")
            .as_slice(),
        ["invocation_archive", "invocation_detail_prune"]
    );

    let archived_live_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = 'archive-priority-candidate'",
    )
    .fetch_one(&pool)
    .await
    .expect("count archived priority candidate");
    assert_eq!(archived_live_rows, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archives_old_invocations_without_changing_summary_all() {
    let (pool, mut config, temp_dir) = retention_test_pool_and_config("retention-archive").await;
    config.codex_invocation_archive_layout = ArchiveBatchLayout::LegacyMonth;
    let old_response = config.proxy_raw_dir.join("old-archive-response.bin");
    fs::write(&old_response, b"archive-response").expect("write archive raw");
    let old_occurred_at = shanghai_local_days_ago(91, 10, 0, 0);
    let old_failed_at = shanghai_local_days_ago(92, 11, 0, 0);
    let recent_at = shanghai_local_days_ago(5, 15, 0, 0);
    insert_retention_invocation(
        &pool,
        "archive-old-success",
        &old_occurred_at,
        SOURCE_XY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        Some(&old_response),
        Some(100),
        Some(0.5),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "archive-old-failed",
        &old_failed_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/chat/completions\"}"),
        "{\"error\":true}",
        None,
        None,
        Some(50),
        Some(0.25),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "archive-recent",
        &recent_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(70),
        Some(0.75),
    )
    .await;

    let before = query_combined_totals(&pool, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("query totals before retention");
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention archive");
    let after = query_combined_totals(&pool, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("query totals after retention");

    assert_eq!(summary.invocation_rows_archived, 2);
    let expected_months = [&old_occurred_at[..7], &old_failed_at[..7]]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(summary.archive_batches_touched, expected_months.len());
    assert_eq!(before.total_count, after.total_count);
    assert_eq!(before.success_count, after.success_count);
    assert_eq!(before.failure_count, after.failure_count);
    assert_eq!(before.total_tokens, after.total_tokens);
    assert_f64_close(before.total_cost, after.total_cost);
    assert!(!old_response.exists());

    let live_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&pool)
        .await
        .expect("count live invocations");
    assert_eq!(live_count, 1);

    let rollup = sqlx::query(
        r#"
        SELECT total_count, success_count, failure_count, total_tokens, total_cost
        FROM invocation_rollup_daily
        WHERE stats_date = ?1 AND source = ?2
        "#,
    )
    .bind(&old_occurred_at[..10])
    .bind(SOURCE_XY)
    .fetch_one(&pool)
    .await
    .expect("load invocation rollup row");
    assert_eq!(rollup.get::<i64, _>("total_count"), 1);
    assert_eq!(rollup.get::<i64, _>("success_count"), 1);
    assert_eq!(rollup.get::<i64, _>("failure_count"), 0);
    assert_eq!(rollup.get::<i64, _>("total_tokens"), 100);
    assert_f64_close(rollup.get::<f64, _>("total_cost"), 0.5);

    let batches = sqlx::query_as::<_, (String, i64, String, String, String)>(
        r#"
        SELECT file_path, row_count, status, layout, summary_source_kind
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
        ORDER BY file_path ASC
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load invocation archive batches");
    assert_eq!(batches.len(), expected_months.len());
    for (file_path, row_count, status, layout, summary_source_kind) in batches {
        let file_path = PathBuf::from(file_path);
        assert!(file_path.exists());
        assert!(row_count >= 1);
        assert_eq!(status, ARCHIVE_STATUS_COMPLETED);
        assert_eq!(layout, ARCHIVE_LAYOUT_LEGACY_MONTH);
        assert!(expected_months.iter().any(|month| {
            file_path
                == archive_batch_file_path(&config, "codex_invocations", month)
                    .expect("canonical month target")
        }));
        assert_eq!(
            summary_source_kind,
            SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE
        );
    }

    let prepared_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM retention_prepared_archives")
            .fetch_one(&pool)
            .await
            .expect("count completed retention preparation rows");
    assert_eq!(
        prepared_count, 0,
        "published batches should retire their journal rows"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn ensure_schema_recreates_retention_recovery_tables_idempotently() {
    let (pool, _config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-recovery-schema-reentry").await;
    sqlx::query("DROP TABLE retention_recovery_cursors")
        .execute(&pool)
        .await
        .expect("remove cursor table to emulate an earlier schema");
    sqlx::query(
        r#"
        CREATE TABLE retention_recovery_cursors (
            scope TEXT PRIMARY KEY,
            cursor TEXT NOT NULL DEFAULT '',
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy cursor table shape");
    sqlx::query(
        "INSERT INTO retention_recovery_cursors (scope, cursor) VALUES ('legacy_archive_segments', 'legacy-cursor')",
    )
    .execute(&pool)
    .await
    .expect("seed legacy cursor row");
    sqlx::query(
        "INSERT INTO retention_recovery_cursors (scope, cursor) VALUES ('raw_payload_files', 'raw-legacy-cursor')",
    )
    .execute(&pool)
    .await
    .expect("seed legacy raw cursor row");
    sqlx::query("DROP TABLE retention_prepared_archives")
        .execute(&pool)
        .await
        .expect("remove journal table to emulate an earlier schema");
    sqlx::query("DROP TABLE retention_raw_reconciliation")
        .execute(&pool)
        .await
        .expect("remove raw reconciliation table to emulate an earlier schema");
    sqlx::query(
        r#"
        CREATE TABLE retention_raw_reconciliation (
            raw_path TEXT PRIMARY KEY,
            file_identity TEXT NOT NULL,
            byte_size INTEGER NOT NULL CHECK(byte_size >= 0),
            quarantined_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy raw reconciliation table shape");

    ensure_schema(&pool)
        .await
        .expect("upgrade earlier schema with retention recovery tables");
    ensure_schema(&pool)
        .await
        .expect("re-enter retention recovery schema upgrade");

    for table in ["retention_prepared_archives", "retention_recovery_cursors"] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .expect("check recovery table");
        assert_eq!(exists, 1, "schema re-entry should ensure {table}");
    }
    let cursor: String = sqlx::query_scalar(
        "SELECT cursor FROM retention_recovery_cursors WHERE scope = 'legacy_archive_segments'",
    )
    .fetch_one(&pool)
    .await
    .expect("load initialized legacy recovery cursor");
    assert_eq!(cursor, "legacy-cursor");
    let cursor_columns: HashSet<String> =
        sqlx::query("PRAGMA table_info('retention_recovery_cursors')")
            .fetch_all(&pool)
            .await
            .expect("inspect recovery cursor columns")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
    for column in [
        "next_retry_at",
        "consecutive_failure_count",
        "last_failure_fingerprint",
        "defer_reason",
        "last_progress_at",
        "last_admission_stage",
        "last_admission_cause",
        "last_settled_pass_at",
        "last_settled_pass_complete",
        "last_settled_pass_inspected_entries",
        "last_settled_pass_referenced_skipped",
        "last_settled_pass_quarantined",
        "last_settled_pass_removed",
        "last_settled_pass_removed_bytes",
        "last_nonzero_removal_at",
        "last_nonzero_removal",
        "last_nonzero_removal_bytes",
    ] {
        assert!(
            cursor_columns.contains(column),
            "missing cursor migration column {column}"
        );
    }
    let raw_reconciliation_columns: HashSet<String> =
        sqlx::query("PRAGMA table_info('retention_raw_reconciliation')")
            .fetch_all(&pool)
            .await
            .expect("inspect raw reconciliation columns")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
    for column in [
        "release_pending",
        "release_pending_at",
        "removal_evidence_persisted",
    ] {
        assert!(
            raw_reconciliation_columns.contains(column),
            "missing raw reconciliation migration column {column}"
        );
    }
    let raw_reconciliation_indexes: HashSet<String> =
        sqlx::query("PRAGMA index_list('retention_raw_reconciliation')")
            .fetch_all(&pool)
            .await
            .expect("inspect raw reconciliation indexes")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
    assert!(
        raw_reconciliation_indexes.contains("idx_retention_raw_reconciliation_pending"),
        "missing raw reconciliation pending index"
    );
    assert!(
        raw_reconciliation_indexes.contains("idx_retention_raw_reconciliation_active"),
        "missing raw reconciliation active index"
    );
    let raw_cursor = sqlx::query(
        r#"
        SELECT cursor,
               last_admission_stage,
               last_admission_cause,
               last_settled_pass_at,
               last_settled_pass_complete,
               last_settled_pass_inspected_entries,
               last_settled_pass_referenced_skipped,
               last_settled_pass_quarantined,
               last_settled_pass_removed,
               last_settled_pass_removed_bytes,
               last_nonzero_removal_at,
               last_nonzero_removal,
               last_nonzero_removal_bytes
        FROM retention_recovery_cursors
        WHERE scope = 'raw_payload_files'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load migrated legacy raw cursor row");
    assert_eq!(raw_cursor.get::<String, _>("cursor"), "raw-legacy-cursor");
    for column in [
        "last_admission_stage",
        "last_admission_cause",
        "last_settled_pass_at",
        "last_nonzero_removal_at",
    ] {
        assert!(
            raw_cursor.get::<Option<String>, _>(column).is_none(),
            "legacy raw cursor evidence should remain unknown for {column}"
        );
    }
    for column in [
        "last_settled_pass_complete",
        "last_settled_pass_inspected_entries",
        "last_settled_pass_referenced_skipped",
        "last_settled_pass_quarantined",
        "last_settled_pass_removed",
        "last_settled_pass_removed_bytes",
        "last_nonzero_removal",
        "last_nonzero_removal_bytes",
    ] {
        assert!(
            raw_cursor.get::<Option<i64>, _>(column).is_none(),
            "legacy raw cursor evidence should remain unknown for {column}"
        );
    }

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn ensure_schema_migrates_staged_archive_path_without_dropping_journal_rows() {
    let (pool, _config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-staged-path-migration").await;
    sqlx::query("DROP TABLE retention_prepared_archives")
        .execute(&pool)
        .await
        .expect("remove current prepared archive table");
    sqlx::query(
        r#"
        CREATE TABLE retention_prepared_archives (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            prepared_key TEXT NOT NULL UNIQUE,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            day_key TEXT,
            part_key TEXT,
            file_path TEXT NOT NULL,
            source_ids_json TEXT NOT NULL,
            source_identity_sha256 TEXT NOT NULL,
            state TEXT NOT NULL,
            artifact_sha256 TEXT,
            artifact_bytes INTEGER,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            next_retry_at TEXT,
            last_failure_stage TEXT,
            last_failure_fingerprint TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            quarantined_at TEXT
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy prepared archive table shape");
    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key, dataset, month_key, file_path, source_ids_json,
            source_identity_sha256, state
        ) VALUES ('legacy-staged-row', 'codex_invocations', '2026-01', '/tmp/legacy.sqlite.gz', '[]', 'identity', 'preparing')
        "#,
    )
    .execute(&pool)
    .await
    .expect("seed legacy prepared archive row");

    ensure_schema(&pool)
        .await
        .expect("migrate staged archive path column");
    let staged_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('retention_prepared_archives') WHERE name = 'staged_file_path'",
    )
    .fetch_one(&pool)
    .await
    .expect("check staged archive path column");
    assert_eq!(staged_column, 1);
    let publication_kind_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('retention_prepared_archives') WHERE name = 'publication_kind'",
    )
    .fetch_one(&pool)
    .await
    .expect("check publication kind column");
    assert_eq!(publication_kind_column, 1);
    let replacement_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('archive_batches') WHERE name = 'replacement_staged_path'",
    )
    .fetch_one(&pool)
    .await
    .expect("check replacement staging column");
    assert_eq!(replacement_column, 1);
    let preserved: (String, String) = sqlx::query_as(
        "SELECT prepared_key, state FROM retention_prepared_archives WHERE prepared_key = 'legacy-staged-row'",
    )
    .fetch_one(&pool)
    .await
    .expect("load preserved legacy prepared archive row");
    assert_eq!(
        preserved,
        ("legacy-staged-row".to_string(), "preparing".to_string())
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_task_local_isolation_skips_already_quarantined_prepared_rows() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-recovery-actionable-queue").await;
    let recovery_archive_root = config.archive_dir.join("codex_invocations");
    std::fs::create_dir_all(&recovery_archive_root).expect("create recovery archive root");
    for index in 0..32 {
        sqlx::query(
            r#"
            INSERT INTO retention_prepared_archives (
                prepared_key, dataset, month_key, file_path, source_ids_json,
                source_identity_sha256, state, attempt_count, quarantined_at, updated_at
            )
            VALUES (?1, 'codex_invocations', '', ?2, '[]', 'legacy-unverified',
                    'quarantined', 0, datetime('now'), datetime('now', '-2 days'))
            "#,
        )
        .bind(format!("unexpired-quarantine-{index:02}"))
        .bind(
            recovery_archive_root
                .join(format!("unexpired-quarantine-{index:02}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .execute(&pool)
        .await
        .expect("seed unexpired quarantined journal row");
    }

    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key, dataset, month_key, file_path, source_ids_json,
            source_identity_sha256, state, artifact_sha256, attempt_count, updated_at
        )
        VALUES ('due-published-row', 'codex_invocations', '', ?1, '[]',
                'source-identity', 'published', 'expected-sha', 1, datetime('now'))
        "#,
    )
    .bind(
        recovery_archive_root
            .join("missing-published.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .execute(&pool)
    .await
    .expect("seed actionable published journal row");
    std::fs::write(
        recovery_archive_root.join("missing-published.sqlite.gz"),
        b"not a valid archive",
    )
    .expect("write actionable published artifact");

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("reconcile due published journal past unexpired quarantines");

    let due_state: (String, Option<String>) = sqlx::query_as(
        "SELECT state, last_failure_stage FROM retention_prepared_archives WHERE prepared_key = 'due-published-row'",
    )
    .fetch_one(&pool)
    .await
    .expect("load actionable journal state");
    assert_eq!(due_state.0, "quarantined");
    assert_eq!(due_state.1.as_deref(), Some("prepared_reconcile"));
    let unexpired_quarantine_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM retention_prepared_archives WHERE state = 'quarantined' AND prepared_key LIKE 'unexpired-quarantine-%'",
    )
    .fetch_one(&pool)
    .await
    .expect("count untouched unexpired quarantines");
    assert_eq!(unexpired_quarantine_count, 32);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_recovery_failure_persistence_waits_for_p1_admission() {
    let (pool, _config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-recovery-p1-admission").await;
    sqlx::query(
        r#"
        INSERT INTO retention_prepared_archives (
            prepared_key, dataset, month_key, file_path, source_ids_json,
            source_identity_sha256, state, attempt_count
        )
        VALUES ('p1-admission-test', 'codex_invocations', '', '/private/test/archive',
                '[]', 'identity-digest', 'published', 1)
        "#,
    )
    .execute(&pool)
    .await
    .expect("insert prepared archive failure fixture");

    let coordinator = crate::proxy_sqlite_write_coordinator::test_proxy_sqlite_write_coordinator();
    let p1_permit = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let maintenance_waiters_before = coordinator.snapshot().await.maintenance_waiter_count;
    let failure_pool = pool.clone();
    let failure_coordinator = coordinator.clone();
    let persist = tokio::spawn(async move {
        crate::maintenance::RETENTION_TEST_WRITE_COORDINATOR
            .scope(
                failure_coordinator,
                crate::maintenance::retention_recovery_persist_failure(
                    &failure_pool,
                    "p1-admission-test",
                    "finalizing",
                    &anyhow::anyhow!("simulated archive failure"),
                ),
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if coordinator.snapshot().await.maintenance_waiter_count
                == maintenance_waiters_before + 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("failure persistence must queue for maintenance admission while P1 is active");
    let retry_before_p1_release: Option<String> = sqlx::query_scalar(
        "SELECT next_retry_at FROM retention_prepared_archives WHERE prepared_key = 'p1-admission-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect retry deadline while P1 owns admission");
    assert_eq!(retry_before_p1_release, None);

    drop(p1_permit);
    persist
        .await
        .expect("failure persistence task should finish")
        .expect("persist failure after P1 releases admission");
    let retry_after_p1_release: Option<String> = sqlx::query_scalar(
        "SELECT next_retry_at FROM retention_prepared_archives WHERE prepared_key = 'p1-admission-test'",
    )
    .fetch_one(&pool)
    .await
    .expect("load retry deadline after P1 release");
    assert!(retry_after_p1_release.is_some());

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archives_into_legacy_pool_attempt_archive_batch_without_route_key_column() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-legacy-pool-attempt-archive").await;
    let occurred_at = shanghai_local_days_ago(91, 9, 0, 0);
    let month_key = occurred_at[..7].to_string();
    let final_archive_path =
        archive_batch_file_path(&config, "pool_upstream_request_attempts", &month_key)
            .expect("resolve legacy pool attempt archive path");
    fs::create_dir_all(
        final_archive_path
            .parent()
            .expect("legacy pool attempt archive path should have parent"),
    )
    .expect("create legacy pool attempt archive dir");

    let legacy_archive_db_path = temp_dir.join("legacy-pool-attempt-archive.sqlite");
    fs::File::create(&legacy_archive_db_path).expect("create legacy pool attempt sqlite file");
    let legacy_archive_pool =
        SqlitePool::connect(&test_sqlite_url_for_path(&legacy_archive_db_path))
            .await
            .expect("open legacy pool attempt archive sqlite");
    let legacy_create_sql = POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL
        .replace("archive_db.", "")
        .replace("    upstream_route_key TEXT,\n", "");
    sqlx::query(&legacy_create_sql)
        .execute(&legacy_archive_pool)
        .await
        .expect("create legacy pool attempt archive schema baseline");
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&legacy_archive_pool)
        .await
        .expect("checkpoint legacy pool attempt archive sqlite before compression");
    legacy_archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&legacy_archive_db_path, &final_archive_path)
        .expect("compress legacy pool attempt archive batch");

    insert_retention_pool_upstream_request_attempt(
        &pool,
        "legacy-pool-attempt-archive-row",
        &occurred_at,
        Some(42),
        1,
        1,
        1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(200),
        None,
        Some(&occurred_at),
        Some(&occurred_at),
    )
    .await;

    let live_row_id: i64 = sqlx::query_scalar(
        "SELECT id FROM pool_upstream_request_attempts WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind("legacy-pool-attempt-archive-row")
    .bind(&occurred_at)
    .fetch_one(&pool)
    .await
    .expect("load live pool attempt row id");
    let archive_outcome = archive_rows_into_month_batch(
        &pool,
        &config,
        archive_table_spec("pool_upstream_request_attempts"),
        &month_key,
        &[live_row_id],
    )
    .await
    .expect("append into legacy pool attempt archive batch");
    assert!(
        archive_outcome.row_count >= 1,
        "legacy pool attempt archive batch should accept appended rows (row_count={})",
        archive_outcome.row_count
    );

    let inflated_legacy_path = temp_dir.join("legacy-pool-attempt-archive-inflated.sqlite");
    inflate_gzip_sqlite_file(&final_archive_path, &inflated_legacy_path)
        .expect("inflate retained legacy pool attempt archive batch");
    let archived_pool = SqlitePool::connect(&test_sqlite_url_for_path(&inflated_legacy_path))
        .await
        .expect("open retained legacy pool attempt archive batch");
    let archived_invoke_ids: HashSet<String> =
        sqlx::query_scalar("SELECT invoke_id FROM pool_upstream_request_attempts")
            .fetch_all(&archived_pool)
            .await
            .expect("load legacy pool attempt archive invoke ids")
            .into_iter()
            .collect();
    assert!(archived_invoke_ids.contains("legacy-pool-attempt-archive-row"));
    let archive_columns: HashSet<String> =
        sqlx::query("PRAGMA table_info('pool_upstream_request_attempts')")
            .fetch_all(&archived_pool)
            .await
            .expect("inspect retained legacy pool attempt archive schema")
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
    assert!(
        archive_columns.contains("upstream_route_key"),
        "legacy pool attempt archive batches should be upgraded with upstream_route_key"
    );
    archived_pool.close().await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn fetch_invocation_pool_attempts_does_not_read_archived_records() {
    let temp_dir = make_temp_test_dir("api-pool-attempts-archive-route-key");
    let mut config = test_config();
    config.archive_dir = temp_dir.join("archives");
    fs::create_dir_all(&config.archive_dir).expect("create archive dir");
    let state = test_state_from_existing_pool(
        SqlitePool::connect("sqlite:file:pool-attempt-archive-route-key?mode=memory&cache=shared")
            .await
            .expect("connect archive route-key sqlite"),
        config,
        true,
    )
    .await;
    ensure_upstream_accounts_schema(&state.pool)
        .await
        .expect("ensure upstream accounts schema");

    let occurred_at = shanghai_local_days_ago(120, 9, 0, 0);
    let month_key = occurred_at[..7].to_string();
    let invoke_id = "archived-pool-attempt-route-key";
    let route_key = "https://route.example/base";
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Archive account")
    .bind("active")
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("insert upstream account");
    insert_retention_invocation(
        &state.pool,
        invoke_id,
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some(r#"{"routeMode":"pool","endpoint":"/v1/responses"}"#),
        "{\"ok\":true}",
        None,
        None,
        None,
        Some(0.1),
    )
    .await;

    let archive_db_path = temp_dir.join("pool-attempts-archive-route-key.sqlite");
    fs::File::create(&archive_db_path).expect("create archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open archive sqlite");
    let create_sql = POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create archive pool attempt schema");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            id,
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            http_status,
            failure_kind,
            error_message,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id,
            created_at
        )
        VALUES (
            1, ?1, ?2, '/v1/responses', ?3, 'sticky-key', ?4, ?5, 1, 1, 1, '203.0.113.5', ?2,
            ?2, ?6, 200, NULL, NULL, 12.5, 34.5, 56.5, 'req_archived', datetime('now')
        )
        "#,
    )
    .bind(invoke_id)
    .bind(&occurred_at)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(42_i64)
    .bind(route_key)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
    .execute(&archive_pool)
    .await
    .expect("insert archive pool attempt row");
    archive_pool.close().await;

    let archive_path = temp_dir
        .join("archives")
        .join("pool-attempts-archive-route-key.sqlite.gz");
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress archive pool attempt batch");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_path).expect("archive sha256"))
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&state.pool)
    .await
    .expect("insert archive batch manifest");

    let Json(records) = fetch_invocation_pool_attempts(
        State(state.clone()),
        axum::extract::Path(invoke_id.to_string()),
    )
    .await
    .expect("fetch archived pool attempt records");
    assert!(records.is_empty());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn upstream_last_activity_backfill_reads_archived_batches() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("upstream-last-activity-archive-backfill").await;
    let created_at = format_utc_iso(Utc::now());
    let account_id = 501_i64;
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
    .bind("Archived-only account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert upstream account");

    let occurred_at = shanghai_local_days_ago(120, 9, 30, 0);
    let month_key = occurred_at[..7].to_string();
    let archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve archived invocation batch");
    fs::create_dir_all(
        archive_path
            .parent()
            .expect("archived invocation batch should have parent"),
    )
    .expect("create archived invocation batch dir");

    let archive_db_path = temp_dir.join("upstream-last-activity-archive.sqlite");
    fs::File::create(&archive_db_path).expect("create archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create archive schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, raw_response, created_at, payload
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(1_i64)
    .bind("archived-upstream-activity")
    .bind(&occurred_at)
    .bind("{}")
    .bind(&occurred_at)
    .bind(json!({ "upstreamAccountId": account_id }).to_string())
    .execute(&archive_pool)
    .await
    .expect("insert archived invocation");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress archived invocation batch");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_path).expect("archive sha256"))
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("insert archive batch manifest");

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("rebuild archive upstream activity manifest");
    assert_eq!(refresh.refreshed_batches, 1);
    assert_eq!(refresh.account_rows_written, 1);

    backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("backfill upstream last activity from archives");

    let last_activity_at: Option<String> =
        sqlx::query_scalar("SELECT last_activity_at FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("load persisted last activity");
    assert_eq!(last_activity_at.as_deref(), Some(occurred_at.as_str()));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn upstream_last_activity_archive_backfill_retries_after_failed_progress() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let pool = state.pool.clone();

    let task_name = STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES;
    let retry_due = format_utc_iso(Utc::now() - ChronoDuration::seconds(1));
    mark_startup_backfill_running(&pool, task_name, 0)
        .await
        .expect("seed running startup progress");
    save_startup_backfill_progress(
        &pool,
        task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 0,
            scanned: 0,
            updated: 0,
            zero_update_streak: 0,
            next_run_after: &retry_due,
            status: STARTUP_BACKFILL_STATUS_FAILED,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed failed startup progress");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityArchives)
        .await
        .expect("retry failed archive backfill progress");

    let progress = load_startup_backfill_progress(&pool, task_name)
        .await
        .expect("load startup backfill progress");
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert!(progress.last_finished_at.is_some());
    assert!(!progress.is_due(Utc::now()));
}

#[tokio::test]
async fn upstream_last_activity_archive_backfill_marks_exhausted_accounts_complete() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task_name = startup_backfill_task_progress_key(
        state.as_ref(),
        StartupBackfillTask::UpstreamActivityArchives,
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    let account_id = 902_i64;

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
    .bind("Never used account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream account");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityArchives)
        .await
        .expect("run archive activity backfill");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load archive backfill progress");
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert_eq!(progress.last_updated, 0);
    assert_eq!(progress.last_scanned, 0);

    let completed: i64 = sqlx::query_scalar(
        r#"
        SELECT last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load archive completion flag");
    assert_eq!(completed, 1);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("force archive task due again");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityArchives)
        .await
        .expect("rerun archive activity backfill");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("reload archive backfill progress");
    assert_eq!(progress.last_scanned, 0);
    assert_eq!(progress.last_updated, 0);
}

#[tokio::test]
async fn upstream_last_activity_live_backfill_marks_unmatched_rows_complete() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task_name = startup_backfill_task_progress_key(
        state.as_ref(),
        StartupBackfillTask::UpstreamActivityLive,
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(903_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("No live invocation")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream account");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityLive)
        .await
        .expect("run live activity backfill");

    let row = sqlx::query_as::<_, (Option<String>, i64)>(
        r#"
        SELECT last_activity_at, last_activity_live_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(903_i64)
    .fetch_one(&state.pool)
    .await
    .expect("load live backfill row");
    assert!(row.0.is_none());
    assert_eq!(row.1, 1);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("force live task due again");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityLive)
        .await
        .expect("rerun live activity backfill");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load live backfill progress");
    assert_eq!(progress.last_updated, 0);
}

#[tokio::test]
async fn upstream_last_activity_archive_backfill_keeps_pending_when_archive_missing() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task_name = startup_backfill_task_progress_key(
        state.as_ref(),
        StartupBackfillTask::UpstreamActivityArchives,
    )
    .await;
    let created_at = format_utc_iso(Utc::now());

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(904_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Missing archive account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert upstream account");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind("2025-01")
    .bind("/tmp/definitely-missing-upstream-activity.sqlite.gz")
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&state.pool)
    .await
    .expect("insert missing archive manifest");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::UpstreamActivityArchives)
        .await
        .expect("run archive activity backfill with missing file");

    let completed: i64 = sqlx::query_scalar(
        r#"
        SELECT last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(904_i64)
    .fetch_one(&state.pool)
    .await
    .expect("load archive completion flag");
    assert_eq!(completed, 0);

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load archive backfill progress");
    assert_eq!(progress.last_updated, 0);
}

#[tokio::test]
async fn upstream_last_activity_archive_backfill_refreshes_existing_activity_when_new_archive_arrives()
 {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let pool = state.pool.clone();
    let temp_dir = make_temp_test_dir("upstream-archive-activity-refresh");
    let account_id = 905_i64;
    let created_at = format_utc_iso(Utc::now());

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
    .bind("Archive refresh account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert upstream account");

    let first_activity_at = format_utc_iso(Utc::now() - ChronoDuration::days(14));
    {
        let month_key = "2025-01";
        let suffix = "first";
        let occurred_at = &first_activity_at;
        let archive_path = temp_dir.join(format!("{month_key}-{suffix}.sqlite.gz"));
        let archive_db_path = temp_dir.join(format!("{month_key}-{suffix}.sqlite"));
        let archive_url = format!("sqlite://{}", archive_db_path.to_string_lossy());
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                build_sqlite_connect_options(
                    &archive_url,
                    Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
                )
                .expect("build archive sqlite options"),
            )
            .await
            .expect("open archive sqlite");

        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                invoke_id TEXT NOT NULL,
                requester TEXT,
                occurred_at TEXT NOT NULL,
                request_method TEXT,
                payload TEXT
            )
            "#,
        )
        .execute(&archive_pool)
        .await
        .expect("create archive codex_invocations");

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, requester, occurred_at, request_method, payload
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(format!("archive-{suffix}"))
        .bind("archived-upstream-activity")
        .bind(occurred_at)
        .bind("{}")
        .bind(json!({ "upstreamAccountId": account_id }).to_string())
        .execute(&archive_pool)
        .await
        .expect("insert archived invocation");
        archive_pool.close().await;
        deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
            .expect("compress archived invocation batch");

        let batch = ArchiveBatchOutcome {
            dataset: "codex_invocations",
            month_key: month_key.to_string(),
            day_key: None,
            part_key: None,
            file_path: archive_path.to_string_lossy().to_string(),
            sha256: sha256_hex_file(&archive_path).expect("archive sha256"),
            source_identity_sha256: None,
            row_count: 1,
            upstream_last_activity: vec![(account_id, occurred_at.to_string())],
            coverage_start_at: None,
            coverage_end_at: None,
            archive_expires_at: None,
            summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN,
            layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
            codec: ARCHIVE_FILE_CODEC_GZIP,
            writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
            cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
            superseded_by: None,
        };
        let mut tx = pool.begin().await.expect("begin archive batch tx");
        upsert_archive_batch_manifest(tx.as_mut(), &batch)
            .await
            .expect("upsert archive batch manifest");
        tx.commit().await.expect("commit archive batch manifest");
    }

    backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("backfill first archive activity");

    let first_row = sqlx::query_as::<_, (Option<String>, i64)>(
        r#"
        SELECT last_activity_at, last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load first archive backfill row");
    assert_eq!(first_row.0.as_deref(), Some(first_activity_at.as_str()));
    assert_eq!(first_row.1, 0);

    let second_activity_at = format_utc_iso(Utc::now() - ChronoDuration::days(1));
    {
        let month_key = "2025-02";
        let suffix = "second";
        let occurred_at = &second_activity_at;
        let archive_path = temp_dir.join(format!("{month_key}-{suffix}.sqlite.gz"));
        let archive_db_path = temp_dir.join(format!("{month_key}-{suffix}.sqlite"));
        let archive_url = format!("sqlite://{}", archive_db_path.to_string_lossy());
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                build_sqlite_connect_options(
                    &archive_url,
                    Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
                )
                .expect("build archive sqlite options"),
            )
            .await
            .expect("open archive sqlite");

        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                invoke_id TEXT NOT NULL,
                requester TEXT,
                occurred_at TEXT NOT NULL,
                request_method TEXT,
                payload TEXT
            )
            "#,
        )
        .execute(&archive_pool)
        .await
        .expect("create archive codex_invocations");

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, requester, occurred_at, request_method, payload
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(format!("archive-{suffix}"))
        .bind("archived-upstream-activity")
        .bind(occurred_at)
        .bind("{}")
        .bind(json!({ "upstreamAccountId": account_id }).to_string())
        .execute(&archive_pool)
        .await
        .expect("insert archived invocation");
        archive_pool.close().await;
        deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
            .expect("compress archived invocation batch");

        let batch = ArchiveBatchOutcome {
            dataset: "codex_invocations",
            month_key: month_key.to_string(),
            day_key: None,
            part_key: None,
            file_path: archive_path.to_string_lossy().to_string(),
            sha256: sha256_hex_file(&archive_path).expect("archive sha256"),
            source_identity_sha256: None,
            row_count: 1,
            upstream_last_activity: vec![(account_id, occurred_at.to_string())],
            coverage_start_at: None,
            coverage_end_at: None,
            archive_expires_at: None,
            summary_source_kind: SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN,
            layout: ARCHIVE_LAYOUT_LEGACY_MONTH,
            codec: ARCHIVE_FILE_CODEC_GZIP,
            writer_version: ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1,
            cleanup_state: ARCHIVE_CLEANUP_STATE_ACTIVE,
            superseded_by: None,
        };
        let mut tx = pool.begin().await.expect("begin archive batch tx");
        upsert_archive_batch_manifest(tx.as_mut(), &batch)
            .await
            .expect("upsert archive batch manifest");
        tx.commit().await.expect("commit archive batch manifest");
    }

    let refreshed_row = sqlx::query_as::<_, (Option<String>, i64)>(
        r#"
        SELECT last_activity_at, last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load refreshed archive row after new archive");
    assert_eq!(
        refreshed_row.0.as_deref(),
        Some(second_activity_at.as_str())
    );
    assert_eq!(refreshed_row.1, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_backfill_waits_for_manifest_until_rebuilt() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-rebuild").await;
    let account_id = 991_i64;
    let created_at = format_utc_iso(Utc::now());
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
    .bind("Manifest backlog account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert manifest backlog account");

    let occurred_at = shanghai_local_days_ago(120, 9, 45, 0);
    let month_key = occurred_at[..7].to_string();
    let archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve manifest backlog archive path");
    fs::create_dir_all(archive_path.parent().expect("archive parent"))
        .expect("create manifest backlog archive parent");
    let archive_db_path = temp_dir.join("manifest-backlog.sqlite");
    fs::File::create(&archive_db_path).expect("create manifest backlog archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open manifest backlog archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create manifest backlog archive schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, raw_response, created_at, payload
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(1_i64)
    .bind("manifest-backlog-row")
    .bind(&occurred_at)
    .bind("{}")
    .bind(&occurred_at)
    .bind(json!({ "upstreamAccountId": account_id }).to_string())
    .execute(&archive_pool)
    .await
    .expect("insert manifest backlog archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress manifest backlog archive");

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
    .bind(sha256_hex_file(&archive_path).expect("manifest backlog archive sha"))
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert manifest backlog batch");

    let waiting = backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("run archive backfill before manifest rebuild");
    assert!(waiting.waiting_for_manifest_backfill);
    assert_eq!(waiting.updated_accounts, 0);

    let dry_run = refresh_archive_upstream_activity_manifest(&pool, &config, true)
        .await
        .expect("dry-run manifest rebuild");
    assert_eq!(dry_run.pending_batches, 1);
    assert_eq!(dry_run.refreshed_batches, 1);
    assert_eq!(dry_run.account_rows_written, 1);

    let rebuild = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("live manifest rebuild");
    assert_eq!(rebuild.pending_batches, 1);
    assert_eq!(rebuild.refreshed_batches, 1);
    assert_eq!(rebuild.account_rows_written, 1);

    let summary = backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("run archive backfill after manifest rebuild");
    assert!(!summary.waiting_for_manifest_backfill);
    assert_eq!(summary.updated_accounts, 1);

    let row = sqlx::query_as::<_, (Option<String>, i64)>(
        r#"
        SELECT last_activity_at, last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load manifest backlog account row");
    assert_eq!(row.0.as_deref(), Some(occurred_at.as_str()));
    assert_eq!(row.1, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_manifest_refresh_leaves_missing_batches_pending_for_retry() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("manifest-missing-terminal").await;
    let account_id = 993_i64;
    let created_at = format_utc_iso(Utc::now());
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
    .bind("Missing manifest archive account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert missing manifest account");

    let occurred_at = shanghai_local_days_ago(90, 10, 15, 0);
    let month_key = occurred_at[..7].to_string();
    let missing_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve missing archive batch path");

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
    .bind(missing_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing manifest batch");

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("refresh manifest with missing archive file");
    assert_eq!(refresh.pending_batches, 1);
    assert_eq!(refresh.refreshed_batches, 0);
    assert_eq!(refresh.missing_files, 1);

    let refreshed_at: Option<String> = sqlx::query_scalar(
        "SELECT upstream_activity_manifest_refreshed_at FROM archive_batches WHERE dataset = 'codex_invocations'",
    )
    .fetch_one(&pool)
    .await
    .expect("load missing batch retry marker");
    assert!(refreshed_at.is_none());

    let summary = backfill_upstream_account_last_activity_from_archives(&pool, None, None)
        .await
        .expect("backfill upstream activity while waiting for missing batch retry");
    assert!(summary.waiting_for_manifest_backfill);
    assert_eq!(summary.updated_accounts, 0);

    let row = sqlx::query_as::<_, (Option<String>, i64)>(
        r#"
        SELECT last_activity_at, last_activity_archive_backfill_completed
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load missing manifest account row");
    assert!(row.0.is_none());
    assert_eq!(row.1, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archives_duplicate_upstream_activity_across_chunks() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-archive-manifest-dedupe").await;
    config.retention_batch_rows = BACKFILL_ACCOUNT_BIND_BATCH_SIZE + 5;

    let account_id = 995_i64;
    let created_at = format_utc_iso(Utc::now());
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
    .bind("Duplicate archive account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert duplicate archive account");

    let base_occurred_at = parse_shanghai_local_naive(&shanghai_local_days_ago(120, 9, 0, 0))
        .expect("valid shanghai local");
    let row_count = BACKFILL_ACCOUNT_BIND_BATCH_SIZE + 5;
    let mut newest_occurred_at = String::new();
    for idx in 0..row_count {
        let occurred_at = format_naive(base_occurred_at + ChronoDuration::seconds(idx as i64));
        newest_occurred_at = occurred_at.clone();
        let response_raw = config
            .proxy_raw_dir
            .join(format!("duplicate-account-{idx}.bin.gz"));
        write_gzip_test_file(
            &response_raw,
            format!("{{\"index\":{idx},\"accountId\":{account_id}}}").as_bytes(),
        );
        insert_retention_invocation(
            &pool,
            &format!("duplicate-account-{idx}"),
            &occurred_at,
            SOURCE_PROXY,
            "success",
            Some(
                &json!({ "endpoint": "/v1/responses", "upstreamAccountId": account_id })
                    .to_string(),
            ),
            "{\"ok\":true}",
            None,
            Some(&response_raw),
            Some(42),
            Some(0.42),
        )
        .await;
    }

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention archive for duplicate account rows");
    assert_eq!(summary.invocation_rows_archived, row_count);
    assert!(summary.raw_files_removed >= row_count);

    let manifest_rows = sqlx::query_as::<_, (i64, String)>(
        r#"
        SELECT account_id, MAX(last_activity_at) AS last_activity_at
        FROM archive_batch_upstream_activity
        GROUP BY account_id
        ORDER BY account_id ASC
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load latest account activity across archive segments");
    assert_eq!(
        manifest_rows,
        vec![(account_id, newest_occurred_at.clone())]
    );

    let last_activity_at: Option<String> =
        sqlx::query_scalar("SELECT last_activity_at FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&pool)
            .await
            .expect("load updated account activity");
    assert_eq!(
        last_activity_at.as_deref(),
        Some(newest_occurred_at.as_str())
    );

    let live_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&pool)
        .await
        .expect("count remaining live invocations");
    assert_eq!(live_count, 0);
    assert_eq!(
        fs::read_dir(&config.proxy_raw_dir)
            .expect("read raw dir after archive cleanup")
            .count(),
        0
    );

    cleanup_temp_test_dir(&temp_dir);
}
