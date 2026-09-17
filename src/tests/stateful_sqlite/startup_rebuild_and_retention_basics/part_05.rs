#[tokio::test]
pub(crate) async fn failure_classification_backfill_from_cursor_respects_scan_limit() {
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

#[derive(sqlx::FromRow)]
struct RetentionPrunedInvocationRow {
    payload: Option<String>,
    raw_response: String,
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
    detail_level: String,
    detail_pruned_at: Option<String>,
    detail_prune_reason: Option<String>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    status: Option<String>,
}

#[derive(sqlx::FromRow)]
struct RetentionArchiveBatchRow {
    file_path: String,
    row_count: i64,
    status: String,
    summary_source_kind: String,
}

async fn load_retention_pruned_invocation(pool: &SqlitePool) -> RetentionPrunedInvocationRow {
    sqlx::query_as::<_, RetentionPrunedInvocationRow>(
        r#"
        SELECT payload, raw_response, request_raw_path, response_raw_path, detail_level,
               detail_pruned_at, detail_prune_reason, total_tokens, cost, status
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("old-success")
    .fetch_one(pool)
    .await
    .expect("load pruned invocation")
}

fn assert_retention_pruned_invocation(
    row: &RetentionPrunedInvocationRow,
    before_pruned_at: chrono::DateTime<chrono::Utc>,
    after_pruned_at: chrono::DateTime<chrono::Utc>,
) {
    assert_eq!(row.detail_level, DETAIL_LEVEL_STRUCTURED_ONLY);
    assert!(row.detail_pruned_at.is_some());
    assert_eq!(
        row.detail_prune_reason.as_deref(),
        Some(DETAIL_PRUNE_REASON_SUCCESS_OVER_30D)
    );
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("request compression metadata should be retained"),
    )
    .expect("decode retained payload");
    assert_eq!(
        payload["requestCompressionAlgorithm"].as_str(),
        Some("zstd")
    );
    assert!(payload.get("endpoint").is_none());
    assert_eq!(row.raw_response, "");
    assert!(row.request_raw_path.is_none());
    assert!(row.response_raw_path.is_none());
    assert_eq!(row.total_tokens, Some(321));
    assert_f64_close(row.cost.unwrap_or_default(), 1.23);
    assert_eq!(row.status.as_deref(), Some("success"));

    let detail_pruned_at = row
        .detail_pruned_at
        .as_deref()
        .expect("detail_pruned_at should be populated");
    let detail_pruned_at = local_naive_to_utc(
        parse_shanghai_local_naive(detail_pruned_at)
            .expect("detail_pruned_at should be shanghai-local"),
        Shanghai,
    )
    .with_timezone(&Utc);
    assert!(detail_pruned_at >= before_pruned_at);
    assert!(detail_pruned_at <= after_pruned_at);
}

async fn assert_retention_prune_archive(pool: &SqlitePool, temp_dir: &Path) {
    let batch = sqlx::query_as::<_, RetentionArchiveBatchRow>(
        r#"
        SELECT file_path, row_count, status, summary_source_kind
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("load prune archive batch");
    let file_path = PathBuf::from(batch.file_path);
    assert!(file_path.exists());
    assert_eq!(batch.status, ARCHIVE_STATUS_COMPLETED);
    assert_eq!(
        batch.summary_source_kind,
        SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR
    );
    assert_eq!(batch.row_count, 1);
    assert!(
        crate::stats::load_completed_invocation_archive_paths(pool)
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
    assert!(!archive_columns.contains("raw_expires_at"));
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
        Some("{\"endpoint\":\"/v1/responses\",\"requestCompressionAlgorithm\":\"zstd\"}")
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
}

#[tokio::test]
pub(crate) async fn retention_prunes_old_success_invocation_details_and_sweeps_orphans() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-prune").await;
    let response_raw = config.proxy_raw_dir.join("old-success-response.bin");
    fs::write(&response_raw, b"response-body").expect("write response raw");
    let request_missing = config.proxy_raw_dir.join("old-success-request.bin");
    let orphan = config.proxy_raw_dir.join("orphan.bin");
    fs::write(&orphan, b"orphan").expect("write orphan raw");
    set_file_mtime_seconds_ago(&orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
    let occurred_at = shanghai_local_days_ago(31, 12, 0, 0);

    insert_retention_invocation_with_fixture(
        &pool,
        RetentionInvocationFixture {
            invoke_id: "old-success",
            occurred_at: &occurred_at,
            source: SOURCE_XY,
            status: "success",
            payload: Some(
                "{\"endpoint\":\"/v1/responses\",\"requestCompressionAlgorithm\":\"zstd\"}",
            ),
            raw_response: "{\"ok\":true}",
            request_raw_path: Some(&request_missing),
            response_raw_path: Some(&response_raw),
            total_tokens: Some(321),
            cost: Some(1.23),
        },
    )
    .await;

    let before_pruned_at = Utc::now() - ChronoDuration::seconds(5);
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention prune");
    let after_pruned_at = Utc::now() + ChronoDuration::seconds(5);
    assert_eq!(summary.invocation_details_pruned, 1);
    assert_eq!(summary.archive_batches_touched, 1);
    assert_eq!(summary.raw_files_removed, 1);
    assert_eq!(summary.orphan_raw_files_removed, 1);
    assert!(!response_raw.exists());
    assert!(!orphan.exists());

    assert_retention_pruned_invocation(
        &load_retention_pruned_invocation(&pool).await,
        before_pruned_at,
        after_pruned_at,
    );
    assert_retention_prune_archive(&pool, &temp_dir).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_preserves_long_term_model_fields_when_pruning_payload() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-prune-long-term-model-fields").await;
    let occurred_at = shanghai_local_days_ago(31, 12, 30, 0);
    insert_retention_invocation_with_fixture(
        &pool,
        RetentionInvocationFixture {
            invoke_id: "old-model-fields",
            occurred_at: &occurred_at,
            source: SOURCE_XY,
            status: "success",
            payload: Some(
            r#"{"upstreamAccountId":771,"requestModel":"gpt-5.4","responseModel":"gpt-5.4-routing","reasoningEffort":"high"}"#,
        ),
            raw_response: "{}",
            request_raw_path: None,
            response_raw_path: None,
            total_tokens: Some(321),
            cost: Some(1.23),
        }
    )
    .await;
    insert_retention_invocation_with_fixture(
        &pool,
        RetentionInvocationFixture {
            invoke_id: "old-model-fields-no-upstream",
            occurred_at: &occurred_at,
            source: SOURCE_XY,
            status: "success",
            payload: Some(r#"{"requestModel":"gpt-5.4","responseModel":"gpt-5.4-routing","reasoningEffort":"high"}"#),
            raw_response: "{}",
            request_raw_path: None,
            response_raw_path: None,
            total_tokens: Some(123),
            cost: Some(0.45),
        }
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
pub(crate) async fn retention_prunes_old_legacy_http_200_success_like_invocation_details() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-prune-legacy-http200").await;
    let response_raw = config.proxy_raw_dir.join("legacy-http200-response.bin");
    fs::write(&response_raw, b"legacy-http200-response").expect("write legacy http_200 raw");
    let occurred_at = shanghai_local_days_ago(31, 13, 0, 0);

    insert_retention_invocation_with_fixture(
        &pool,
        RetentionInvocationFixture {
            invoke_id: "old-legacy-http200-success-like",
            occurred_at: &occurred_at,
            source: SOURCE_PROXY,
            status: "http_200",
            payload: Some("{\"endpoint\":\"/v1/responses\"}"),
            raw_response: "{\"ok\":true}",
            request_raw_path: None,
            response_raw_path: Some(&response_raw),
            total_tokens: Some(456),
            cost: Some(1.78),
        },
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
pub(crate) async fn retention_does_not_prune_legacy_http_200_rows_with_error_message() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-prune-legacy-http200-error").await;
    config.proxy_raw_compression = RawCompressionCodec::None;
    let response_raw = config
        .proxy_raw_dir
        .join("legacy-http200-error-response.bin");
    fs::write(&response_raw, b"legacy-http200-error-response")
        .expect("write legacy http_200 error raw");
    let occurred_at = shanghai_local_days_ago(31, 14, 0, 0);

    insert_retention_invocation_with_fixture(
        &pool,
        RetentionInvocationFixture {
            invoke_id: "old-legacy-http200-error",
            occurred_at: &occurred_at,
            source: SOURCE_PROXY,
            status: "http_200",
            payload: Some("{\"endpoint\":\"/v1/responses\"}"),
            raw_response: "{\"ok\":false}",
            request_raw_path: None,
            response_raw_path: Some(&response_raw),
            total_tokens: Some(654),
            cost: Some(2.34),
        },
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

async fn seed_retention_archive_rows(pool: &SqlitePool, old_response: &Path) -> String {
    let old_occurred_at = shanghai_local_days_ago(91, 10, 0, 0);
    let old_failed_at = shanghai_local_days_ago(92, 11, 0, 0);
    let recent_at = shanghai_local_days_ago(5, 15, 0, 0);

    insert_retention_invocation_with_fixture(
        pool,
        RetentionInvocationFixture {
            invoke_id: "archive-old-success",
            occurred_at: &old_occurred_at,
            source: SOURCE_XY,
            status: "success",
            payload: Some("{\"endpoint\":\"/v1/responses\"}"),
            raw_response: "{\"ok\":true}",
            request_raw_path: None,
            response_raw_path: Some(old_response),
            total_tokens: Some(100),
            cost: Some(0.5),
        },
    )
    .await;
    insert_retention_invocation_with_fixture(
        pool,
        RetentionInvocationFixture {
            invoke_id: "archive-old-failed",
            occurred_at: &old_failed_at,
            source: SOURCE_PROXY,
            status: "failed",
            payload: Some("{\"endpoint\":\"/v1/chat/completions\"}"),
            raw_response: "{\"error\":true}",
            request_raw_path: None,
            response_raw_path: None,
            total_tokens: Some(50),
            cost: Some(0.25),
        },
    )
    .await;
    insert_retention_invocation_with_fixture(
        pool,
        RetentionInvocationFixture {
            invoke_id: "archive-recent",
            occurred_at: &recent_at,
            source: SOURCE_PROXY,
            status: "success",
            payload: Some("{\"endpoint\":\"/v1/responses\"}"),
            raw_response: "{\"ok\":true}",
            request_raw_path: None,
            response_raw_path: None,
            total_tokens: Some(70),
            cost: Some(0.75),
        },
    )
    .await;

    old_occurred_at
}

fn assert_retention_archive_totals(before: StatsTotals, after: StatsTotals) {
    assert_eq!(before.total_count, after.total_count);
    assert_eq!(before.success_count, after.success_count);
    assert_eq!(before.failure_count, after.failure_count);
    assert_eq!(before.total_tokens, after.total_tokens);
    assert_f64_close(before.total_cost, after.total_cost);
}

async fn assert_retention_archive_live_count(pool: &SqlitePool) {
    let live_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(pool)
        .await
        .expect("count live invocations");
    assert_eq!(live_count, 1);
}

async fn assert_retention_archive_rollup(pool: &SqlitePool, old_occurred_at: &str) {
    let rollup = sqlx::query(
        r#"
        SELECT total_count, success_count, failure_count, total_tokens, total_cost
        FROM invocation_rollup_daily
        WHERE stats_date = ?1 AND source = ?2
        "#,
    )
    .bind(&old_occurred_at[..10])
    .bind(SOURCE_XY)
    .fetch_one(pool)
    .await
    .expect("load invocation rollup row");
    assert_eq!(rollup.get::<i64, _>("total_count"), 1);
    assert_eq!(rollup.get::<i64, _>("success_count"), 1);
    assert_eq!(rollup.get::<i64, _>("failure_count"), 0);
    assert_eq!(rollup.get::<i64, _>("total_tokens"), 100);
    assert_f64_close(rollup.get::<f64, _>("total_cost"), 0.5);
}

async fn assert_retention_archive_batches(pool: &SqlitePool) {
    let batches = sqlx::query_as::<_, (String, i64, String, String, String)>(
        r#"
        SELECT file_path, row_count, status, layout, summary_source_kind
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
        ORDER BY file_path ASC
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("load invocation archive batches");
    assert_eq!(batches.len(), 2);
    for (file_path, row_count, status, layout, summary_source_kind) in batches {
        let file_path = PathBuf::from(file_path);
        assert!(file_path.exists());
        assert!(row_count >= 1);
        assert_eq!(status, ARCHIVE_STATUS_COMPLETED);
        assert_eq!(layout, ARCHIVE_LAYOUT_SEGMENT_V1);
        assert_eq!(
            summary_source_kind,
            SUMMARY_ARCHIVE_SOURCE_KIND_AUTHORITATIVE
        );
    }
}

#[tokio::test]
pub(crate) async fn retention_archives_old_invocations_without_changing_summary_all() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-archive").await;
    let old_response = config.proxy_raw_dir.join("old-archive-response.bin");
    fs::write(&old_response, b"archive-response").expect("write archive raw");
    let old_occurred_at = seed_retention_archive_rows(&pool, &old_response).await;
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
    assert_eq!(summary.archive_batches_touched, 2);
    assert_retention_archive_totals(before, after);
    assert!(!old_response.exists());
    assert_retention_archive_live_count(&pool).await;
    assert_retention_archive_rollup(&pool, &old_occurred_at).await;
    assert_retention_archive_batches(&pool).await;

    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_legacy_codex_archive(temp_dir: &Path, final_archive_path: &Path) {
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
        .replace("    first_token_ms REAL,\n", "");
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
    deflate_sqlite_file_to_gzip(&legacy_archive_db_path, final_archive_path)
        .expect("compress legacy archive batch");
}

async fn append_legacy_codex_archive_row(
    pool: &SqlitePool,
    config: &AppConfig,
    month_key: &str,
    occurred_at: &str,
) {
    let live_row_id: i64 = sqlx::query_scalar(
        "SELECT id FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind("archive-into-legacy-batch")
    .bind(occurred_at)
    .fetch_one(pool)
    .await
    .expect("load live invocation row id");
    let archive_outcome = archive_rows_into_month_batch(
        pool,
        config,
        archive_table_spec("codex_invocations"),
        month_key,
        &[live_row_id],
    )
    .await
    .expect("append into legacy archive batch");
    assert!(
        archive_outcome.row_count >= 1,
        "legacy archive batch should accept appended rows with legacy schema (row_count={})",
        archive_outcome.row_count
    );
}

async fn assert_legacy_codex_archive(temp_dir: &Path, final_archive_path: &Path) {
    let inflated_legacy_path = temp_dir.join("legacy-archive-inflated.sqlite");
    inflate_gzip_sqlite_file(final_archive_path, &inflated_legacy_path)
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
    assert!(archive_columns.contains("raw_expires_at"));
    assert!(archive_columns.contains("first_token_ms"));
    let archived_first_token_ms: Option<f64> =
        sqlx::query_scalar("SELECT first_token_ms FROM codex_invocations WHERE invoke_id = ?1")
            .bind("archive-into-legacy-batch")
            .fetch_one(&archived_pool)
            .await
            .expect("load archived TTFT");
    assert_eq!(archived_first_token_ms, None);
    archived_pool.close().await;
}

#[tokio::test]
pub(crate) async fn retention_archives_into_legacy_archive_batch_with_raw_expires_at_column() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-legacy-archive").await;
    let occurred_at = shanghai_local_days_ago(91, 9, 0, 0);
    let month_key = occurred_at[..7].to_string();
    let final_archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve legacy archive path");
    seed_legacy_codex_archive(&temp_dir, &final_archive_path).await;

    insert_retention_invocation_with_fixture(
        &pool,
        RetentionInvocationFixture {
            invoke_id: "archive-into-legacy-batch",
            occurred_at: &occurred_at,
            source: SOURCE_PROXY,
            status: "failed",
            payload: Some("{\"endpoint\":\"/v1/responses\"}"),
            raw_response: "{\"error\":true}",
            request_raw_path: None,
            response_raw_path: None,
            total_tokens: Some(42),
            cost: Some(0.42),
        },
    )
    .await;
    append_legacy_codex_archive_row(&pool, &config, &month_key, &occurred_at).await;
    assert_legacy_codex_archive(&temp_dir, &final_archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_legacy_pool_attempt_archive(temp_dir: &Path, final_archive_path: &Path) {
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
    deflate_sqlite_file_to_gzip(&legacy_archive_db_path, final_archive_path)
        .expect("compress legacy pool attempt archive batch");
}

async fn append_legacy_pool_attempt_archive_row(
    pool: &SqlitePool,
    config: &AppConfig,
    month_key: &str,
    occurred_at: &str,
) {
    insert_retention_pool_upstream_request_attempt(
        pool,
        RetentionPoolAttemptFixture {
            invoke_id: "legacy-pool-attempt-archive-row",
            occurred_at,
            upstream_account_id: Some(42),
            attempt_index: 1,
            distinct_account_index: 1,
            same_account_retry_index: 1,
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            http_status: Some(200),
            failure_kind: None,
            started_at: Some(occurred_at),
            finished_at: Some(occurred_at),
        },
    )
    .await;
    let live_row_id: i64 = sqlx::query_scalar(
        "SELECT id FROM pool_upstream_request_attempts WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind("legacy-pool-attempt-archive-row")
    .bind(occurred_at)
    .fetch_one(pool)
    .await
    .expect("load live pool attempt row id");
    let archive_outcome = archive_rows_into_month_batch(
        pool,
        config,
        archive_table_spec("pool_upstream_request_attempts"),
        month_key,
        &[live_row_id],
    )
    .await
    .expect("append into legacy pool attempt archive batch");
    assert!(
        archive_outcome.row_count >= 1,
        "legacy pool attempt archive batch should accept appended rows (row_count={})",
        archive_outcome.row_count
    );
}

async fn assert_legacy_pool_attempt_archive(temp_dir: &Path, final_archive_path: &Path) {
    let inflated_legacy_path = temp_dir.join("legacy-pool-attempt-archive-inflated.sqlite");
    inflate_gzip_sqlite_file(final_archive_path, &inflated_legacy_path)
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
}

#[tokio::test]
pub(crate) async fn retention_archives_into_legacy_pool_attempt_archive_batch_without_route_key_column()
 {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-legacy-pool-attempt-archive").await;
    let occurred_at = shanghai_local_days_ago(91, 9, 0, 0);
    let month_key = occurred_at[..7].to_string();
    let final_archive_path =
        archive_batch_file_path(&config, "pool_upstream_request_attempts", &month_key)
            .expect("resolve legacy pool attempt archive path");
    seed_legacy_pool_attempt_archive(&temp_dir, &final_archive_path).await;
    append_legacy_pool_attempt_archive_row(&pool, &config, &month_key, &occurred_at).await;
    assert_legacy_pool_attempt_archive(&temp_dir, &final_archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

struct ArchivedPoolAttemptFixture {
    temp_dir: PathBuf,
    invoke_id: &'static str,
    occurred_at: String,
    archive_path: PathBuf,
}

async fn prepare_pool_attempt_archive_state() -> (Arc<AppState>, PathBuf) {
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
    (state, temp_dir)
}

async fn insert_pool_attempt_archive_source(
    state: &Arc<AppState>,
    invoke_id: &str,
    occurred_at: &str,
) {
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
    insert_retention_invocation_with_fixture(
        &state.pool,
        RetentionInvocationFixture {
            invoke_id,
            occurred_at,
            source: SOURCE_PROXY,
            status: "success",
            payload: Some(r#"{"routeMode":"pool","endpoint":"/v1/responses"}"#),
            raw_response: "{\"ok\":true}",
            request_raw_path: None,
            response_raw_path: None,
            total_tokens: None,
            cost: Some(0.1),
        },
    )
    .await;
}

async fn write_pool_attempt_archive_file(
    temp_dir: &Path,
    invoke_id: &str,
    occurred_at: &str,
    route_key: &str,
) -> PathBuf {
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
    .bind(occurred_at)
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
    archive_path
}

async fn register_pool_attempt_archive_manifest(
    pool: &SqlitePool,
    archive_path: &Path,
    occurred_at: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&occurred_at[..7])
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(archive_path).expect("archive sha256"))
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(pool)
    .await
    .expect("insert archive batch manifest");
}

async fn seed_pool_attempt_archive_fixture(
    state: &Arc<AppState>,
    temp_dir: &Path,
) -> ArchivedPoolAttemptFixture {
    let occurred_at = shanghai_local_days_ago(120, 9, 0, 0);
    let invoke_id = "archived-pool-attempt-route-key";
    let route_key = "https://route.example/base";
    insert_pool_attempt_archive_source(state, invoke_id, &occurred_at).await;
    let archive_path =
        write_pool_attempt_archive_file(temp_dir, invoke_id, &occurred_at, route_key).await;
    register_pool_attempt_archive_manifest(&state.pool, &archive_path, &occurred_at).await;
    ArchivedPoolAttemptFixture {
        temp_dir: temp_dir.to_path_buf(),
        invoke_id,
        occurred_at,
        archive_path,
    }
}

#[tokio::test]
pub(crate) async fn fetch_invocation_pool_attempts_does_not_read_archived_records() {
    let (state, temp_dir) = prepare_pool_attempt_archive_state().await;
    let fixture = seed_pool_attempt_archive_fixture(&state, &temp_dir).await;

    let Json(records) = fetch_invocation_pool_attempts(
        State(state.clone()),
        axum::extract::Path(fixture.invoke_id.to_string()),
    )
    .await
    .expect("fetch archived pool attempt records");
    assert!(records.is_empty());

    assert!(fixture.archive_path.exists());
    assert!(!fixture.occurred_at.is_empty());
    cleanup_temp_test_dir(&fixture.temp_dir);
}

use super::*;
