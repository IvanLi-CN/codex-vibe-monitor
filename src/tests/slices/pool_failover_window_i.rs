#[tokio::test]
async fn hourly_timeseries_omits_pre_cutoff_partial_hour_rollups() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;

    let pre_cutoff_local = start_of_local_day(Utc::now(), Shanghai)
        .with_timezone(&Shanghai)
        .naive_local()
        - ChronoDuration::minutes(15);
    let occurred_at = format_naive(pre_cutoff_local);
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("null-status-exact-hourly")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(5_i64)
    .bind(0.05_f64)
    .bind(None::<String>)
    .bind("{}")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert null-status exact row");

    let bucket_start_epoch =
        invocation_bucket_start_epoch(&occurred_at).expect("derive pre-cutoff bucket epoch");
    let bucket_start = Utc
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .expect("valid pre-cutoff bucket start");
    insert_invocation_hourly_rollup_bucket(
        &state.pool,
        bucket_start,
        SOURCE_PROXY,
        1,
        0,
        0,
        5,
        0.05,
    )
    .await;

    let start = local_naive_to_utc(pre_cutoff_local - ChronoDuration::minutes(15), Shanghai);
    let end = local_naive_to_utc(pre_cutoff_local + ChronoDuration::minutes(15), Shanghai);
    let Json(response) = fetch_timeseries_from_hourly_rollups(
        state,
        TimeseriesQuery {
            range: "ignored".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        },
        Shanghai,
        InvocationSourceScope::ProxyOnly,
        RangeWindow {
            start,
            end,
            display_end: end,
            duration: end - start,
        },
        TimeseriesBucketSelection {
            bucket_seconds: 3_600,
            effective_bucket: "1h".to_string(),
            available_buckets: vec!["1h".to_string()],
            bucket_limited_to_daily: false,
        },
    )
    .await
    .expect("fetch exact hourly timeseries");

    let point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(bucket_start))
        .expect("pre-cutoff bucket should remain visible");
    assert_eq!(response.range_end, format_utc_iso(end));
    assert_eq!(point.total_count, 0);
    assert_eq!(point.success_count, 0);
    assert_eq!(point.failure_count, 0);
    assert_eq!(point.total_tokens, 0);
    assert_f64_close(point.total_cost, 0.0);
}

#[tokio::test]
async fn forward_proxy_timeseries_rejects_non_hour_aligned_timezones() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let err = fetch_forward_proxy_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "24h".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Kolkata".to_string()),
        }),
    )
    .await
    .expect_err("non-hour-aligned timezones should be rejected");

    match err {
        ApiError::BadRequest(err) => {
            assert!(
                err.to_string().contains("whole-hour UTC offsets"),
                "unexpected error message: {err}"
            );
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[tokio::test]
async fn timeseries_subday_bucket_stays_available_inside_live_window() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let live_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(2);
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-subday-stays-12h",
        &format_naive(live_date.and_hms_opt(3, 0, 0).expect("valid live time")),
        "success",
        Some(90.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "7d".to_string(),
            bucket: Some("12h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch live-window subday timeseries");

    assert_eq!(response.bucket_seconds, 43_200);
    assert_eq!(response.effective_bucket, "12h");
    assert!(!response.bucket_limited_to_daily);
    assert!(response.available_buckets.contains(&"12h".to_string()));
    assert!(response.available_buckets.contains(&"1d".to_string()));
}

#[tokio::test]
async fn invocation_hourly_rollup_range_respects_proxy_only_scope() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let stats_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(20);
    insert_invocation_rollup(&state.pool, stats_date, SOURCE_PROXY, 2, 1, 1, 20, 0.2).await;
    insert_invocation_rollup(&state.pool, stats_date, SOURCE_XY, 5, 5, 0, 50, 0.5).await;

    let range_start_epoch = local_naive_to_utc(
        stats_date
            .and_hms_opt(0, 0, 0)
            .expect("stats_date midnight should be valid"),
        Shanghai,
    )
    .timestamp();
    let range_end_epoch = range_start_epoch + 3_600;

    let proxy_rows = query_invocation_hourly_rollup_range(
        &state.pool,
        range_start_epoch,
        range_end_epoch,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query proxy hourly rollup range");
    let all_rows = query_invocation_hourly_rollup_range(
        &state.pool,
        range_start_epoch,
        range_end_epoch,
        InvocationSourceScope::All,
    )
    .await
    .expect("query all hourly rollup range");

    assert_eq!(proxy_rows.len(), 1);
    assert_eq!(proxy_rows[0].total_count, 2);
    assert_eq!(all_rows.len(), 2);
    assert_eq!(all_rows.iter().map(|row| row.total_count).sum::<i64>(), 7);
}

#[tokio::test]
async fn ensure_schema_migrates_codex_invocations_off_raw_expires_at_and_adds_retention_tables() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(
        r#"
        CREATE TABLE codex_invocations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'xy',
            payload TEXT,
            raw_response TEXT NOT NULL,
            raw_expires_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(invoke_id, occurred_at)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy invocation schema");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            payload,
            raw_response,
            raw_expires_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("legacy-row")
    .bind("2026-03-09 10:00:00")
    .bind(SOURCE_PROXY)
    .bind("{\"endpoint\":\"/v1/responses\"}")
    .bind("{\"ok\":true}")
    .bind("2099-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("insert legacy invocation row");

    ensure_schema(&pool).await.expect("ensure schema migration");

    let columns: HashSet<String> = sqlx::query("PRAGMA table_info('codex_invocations')")
        .fetch_all(&pool)
        .await
        .expect("inspect invocation columns")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(!columns.contains("raw_expires_at"));
    assert!(columns.contains("detail_level"));
    assert!(columns.contains("detail_pruned_at"));
    assert!(columns.contains("detail_prune_reason"));

    let row = sqlx::query(
        r#"
        SELECT invoke_id, source, payload, raw_response, detail_level
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("legacy-row")
    .fetch_one(&pool)
    .await
    .expect("load migrated invocation row");
    assert_eq!(row.get::<String, _>("invoke_id"), "legacy-row");
    assert_eq!(row.get::<String, _>("source"), SOURCE_PROXY);
    assert_eq!(
        row.get::<Option<String>, _>("payload").as_deref(),
        Some("{\"endpoint\":\"/v1/responses\"}")
    );
    assert_eq!(row.get::<String, _>("raw_response"), "{\"ok\":true}");
    assert_eq!(row.get::<String, _>("detail_level"), DETAIL_LEVEL_FULL);

    let tables: HashSet<String> = sqlx::query_scalar(
        r#"
        SELECT name
        FROM sqlite_master
        WHERE type = 'table'
          AND name IN ('archive_batches', 'invocation_rollup_daily', 'startup_backfill_progress')
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load retention tables")
    .into_iter()
    .collect();
    assert!(tables.contains("archive_batches"));
    assert!(tables.contains("invocation_rollup_daily"));
    assert!(tables.contains("startup_backfill_progress"));

    let upstream_account_index_sql = sqlx::query_scalar::<_, String>(
        r#"
        SELECT sql
        FROM sqlite_master
        WHERE type = 'index'
          AND name = 'idx_codex_invocations_upstream_account_occurred_at'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load upstream account invocation index");
    assert!(upstream_account_index_sql.contains("$.upstreamAccountId"));
    assert!(upstream_account_index_sql.contains("occurred_at"));
}

#[tokio::test]
async fn health_check_reports_starting_until_startup_is_ready() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18080").expect("valid upstream url"),
    )
    .await;

    state.startup_ready.store(false, Ordering::Release);
    let response = health_check(State(state.clone())).await.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read health body");
    assert_eq!(std::str::from_utf8(&body).expect("utf8 body"), "starting");

    state.startup_ready.store(true, Ordering::Release);
    let response = health_check(State(state)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read health body");
    assert_eq!(std::str::from_utf8(&body).expect("utf8 body"), "ok");
}

#[tokio::test]
async fn startup_backfill_progress_persists_terminal_missing_raw_cursor() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("reasoning-missing-raw")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .bind("missing-reasoning-request.json")
    .execute(&state.pool)
    .await
    .expect("insert reasoning backfill row");

    let row_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
            .bind("reasoning-missing-raw")
            .fetch_one(&state.pool)
            .await
            .expect("fetch inserted row id");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
        .await
        .expect("first startup backfill pass should succeed");

    let task_name =
        startup_backfill_task_progress_key(state.as_ref(), StartupBackfillTask::ReasoningEffort)
            .await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load backfill progress after first pass");
    assert_eq!(progress.cursor_id, row_id);
    assert_eq!(progress.last_scanned, 1);
    assert_eq!(progress.last_updated, 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("force startup backfill task due again");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
        .await
        .expect("second startup backfill pass should skip previously scanned row");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load backfill progress after second pass");
    assert_eq!(progress.cursor_id, row_id);
    assert_eq!(progress.last_scanned, 0);
    assert_eq!(progress.last_updated, 0);
}

#[tokio::test]
async fn failure_classification_backfill_skips_success_rows_with_complete_defaults() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

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

    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

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
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

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
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&request_missing),
        Some(&response_raw),
        Some(321),
        Some(1.23),
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
    assert!(row.get::<Option<String>, _>("payload").is_none());
    assert_eq!(row.get::<String, _>("raw_response"), "");
    assert!(row.get::<Option<String>, _>("request_raw_path").is_none());
    assert!(row.get::<Option<String>, _>("response_raw_path").is_none());
    assert_eq!(row.get::<Option<i64>, _>("total_tokens"), Some(321));
    assert_f64_close(row.get::<Option<f64>, _>("cost").unwrap_or_default(), 1.23);
    assert_eq!(
        row.get::<Option<String>, _>("status").as_deref(),
        Some("success")
    );

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
        SELECT file_path, row_count, status
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load prune archive batch");
    let file_path = PathBuf::from(batch.get::<String, _>("file_path"));
    assert!(file_path.exists());
    assert_eq!(batch.get::<String, _>("status"), ARCHIVE_STATUS_COMPLETED);
    assert_eq!(batch.get::<i64, _>("row_count"), 1);

    let archive_db_path = temp_dir.join("retention-prune-archive.sqlite");
    inflate_gzip_sqlite_file(&file_path, &archive_db_path).expect("inflate prune archive");
    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
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
        Some("{\"endpoint\":\"/v1/responses\"}")
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
async fn retention_archives_old_invocations_without_changing_summary_all() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-archive").await;
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

    let before = query_combined_totals(&pool, None, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("query totals before retention");
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention archive");
    let after = query_combined_totals(&pool, None, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("query totals after retention");

    assert_eq!(summary.invocation_rows_archived, 2);
    assert_eq!(summary.archive_batches_touched, 2);
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

    let batches = sqlx::query_as::<_, (String, i64, String, String)>(
        r#"
        SELECT file_path, row_count, status, layout
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
        ORDER BY file_path ASC
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load invocation archive batches");
    assert_eq!(batches.len(), 2);
    for (file_path, row_count, status, layout) in batches {
        let file_path = PathBuf::from(file_path);
        assert!(file_path.exists());
        assert!(row_count >= 1);
        assert_eq!(status, ARCHIVE_STATUS_COMPLETED);
        assert_eq!(layout, ARCHIVE_LAYOUT_SEGMENT_V1);
    }

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archives_into_legacy_archive_batch_with_raw_expires_at_column() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-legacy-archive").await;
    let occurred_at = shanghai_local_days_ago(91, 9, 0, 0);
    let month_key = occurred_at[..7].to_string();
    let final_archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve legacy archive path");
    fs::create_dir_all(
        final_archive_path
            .parent()
            .expect("legacy archive path should have parent"),
    )
    .expect("create legacy archive dir");

    let legacy_archive_db_path = temp_dir.join("legacy-archive.sqlite");
    fs::File::create(&legacy_archive_db_path).expect("create legacy archive sqlite file");
    let legacy_archive_pool = SqlitePool::connect(&sqlite_url_for_path(&legacy_archive_db_path))
        .await
        .expect("open legacy archive sqlite");
    let legacy_create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
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
    deflate_sqlite_file_to_gzip(&legacy_archive_db_path, &final_archive_path)
        .expect("compress legacy archive batch");

    insert_retention_invocation(
        &pool,
        "archive-into-legacy-batch",
        &occurred_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"error\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    let live_row_id: i64 = sqlx::query_scalar(
        "SELECT id FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind("archive-into-legacy-batch")
    .bind(&occurred_at)
    .fetch_one(&pool)
    .await
    .expect("load live invocation row id");
    let archive_outcome = archive_rows_into_month_batch(
        &pool,
        &config,
        archive_table_spec("codex_invocations"),
        &month_key,
        &[live_row_id],
    )
    .await
    .expect("append into legacy archive batch");
    assert!(
        archive_outcome.row_count >= 1,
        "legacy archive batch should accept appended rows with legacy schema (row_count={})",
        archive_outcome.row_count
    );

    let inflated_legacy_path = temp_dir.join("legacy-archive-inflated.sqlite");
    inflate_gzip_sqlite_file(&final_archive_path, &inflated_legacy_path)
        .expect("inflate retained legacy archive batch");
    let archived_pool = SqlitePool::connect(&sqlite_url_for_path(&inflated_legacy_path))
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
    assert!(
        archive_columns.contains("raw_expires_at"),
        "historical archive files should keep their legacy schema"
    );
    archived_pool.close().await;

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
    let legacy_archive_pool = SqlitePool::connect(&sqlite_url_for_path(&legacy_archive_db_path))
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
    let archived_pool = SqlitePool::connect(&sqlite_url_for_path(&inflated_legacy_path))
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
    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
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
    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
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

    let refresh = refresh_archive_upstream_activity_manifest(&pool, false)
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
            row_count: 1,
            upstream_last_activity: vec![(account_id, occurred_at.to_string())],
            coverage_start_at: None,
            coverage_end_at: None,
            archive_expires_at: None,
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
            row_count: 1,
            upstream_last_activity: vec![(account_id, occurred_at.to_string())],
            coverage_start_at: None,
            coverage_end_at: None,
            archive_expires_at: None,
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
    let (pool, config, temp_dir) = retention_test_pool_and_config("archive-manifest-rebuild").await;
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
    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
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

    let dry_run = refresh_archive_upstream_activity_manifest(&pool, true)
        .await
        .expect("dry-run manifest rebuild");
    assert_eq!(dry_run.pending_batches, 1);
    assert_eq!(dry_run.refreshed_batches, 1);
    assert_eq!(dry_run.account_rows_written, 1);

    let rebuild = refresh_archive_upstream_activity_manifest(&pool, false)
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
        retention_test_pool_and_config("manifest-missing-terminal").await;
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

    let refresh = refresh_archive_upstream_activity_manifest(&pool, false)
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
        SELECT account_id, last_activity_at
        FROM archive_batch_upstream_activity
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load deduped archive manifest rows");
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
