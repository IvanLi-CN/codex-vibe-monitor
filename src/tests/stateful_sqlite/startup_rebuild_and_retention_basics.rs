use super::*;
use serde_json::json;
use std::future::Future;

async fn assert_startup_backfill_busy_error_closes_gate_before_next_task(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
    gate: &crate::db_pressure::DbPressureGate,
) {
    let err = run_startup_backfill_task_if_due_with_gate(state, task, gate)
        .await
        .expect_err("injected SQLite lock should fail the startup backfill task");
    assert!(
        crate::is_sqlite_lock_error(&err),
        "expected an actual SQLite BUSY/LOCKED error: {err:#}"
    );
    assert_eq!(
        gate.snapshot().pressure_events,
        1,
        "the failure must close the gate before its permit is released"
    );

    // A closed pool turns this into a zero-SQL admission assertion. The only valid result is a
    // scheduler-only pressure defer after the failed task releases its permit.
    state.pool.close().await;
    let next = run_startup_backfill_task_if_due_with_gate(
        state,
        StartupBackfillTask::PromptCacheKey,
        gate,
    )
    .await
    .expect("the next task should be deferred before SQLite access");
    assert!(!next);
    assert_eq!(
        gate.snapshot().pressure_events,
        1,
        "a gate-closed admission must not record the already-classified error again"
    );
    assert!(
        gate.snapshot().background_skips >= 1,
        "the next task must not enter SQLite during the permit-release gap"
    );
}

async fn assert_startup_coverage_repair_busy_error_closes_gate_before_next_task<
    Repair,
    RepairFuture,
>(
    state: &Arc<AppState>,
    gate: &crate::db_pressure::DbPressureGate,
    repair: Repair,
) where
    Repair: FnOnce() -> RepairFuture,
    RepairFuture: Future<Output = Result<ActiveAccountActivityV2RepairOutcome>>,
{
    let outcome = run_startup_backfill_coverage_repair_if_due_with_repair(state, gate, repair)
        .await
        .expect("injected SQLite lock should defer the coverage repair task");
    assert!(outcome.is_pressure_deferred());
    assert_eq!(
        gate.snapshot().pressure_events,
        1,
        "the coverage error must close the gate before its permit is released"
    );

    // The one-slot production-shaped gate must remain closed after the failed repair releases
    // its permit. Closing the pool makes this a zero-SQL assertion for the next admission.
    state.pool.close().await;
    let next = run_startup_backfill_task_if_due_with_gate(
        state,
        StartupBackfillTask::PromptCacheKey,
        gate,
    )
    .await
    .expect("the next task should be deferred before SQLite access");
    assert!(!next);
    assert_eq!(
        gate.snapshot().pressure_events,
        1,
        "a gate-closed admission must not record the already-classified error again"
    );
    assert!(
        gate.snapshot().background_skips >= 1,
        "the next task must not enter SQLite before the pressure cooldown"
    );
}

async fn seed_due_startup_backfill_progress(
    state: &Arc<AppState>,
    task: StartupBackfillTask,
) -> String {
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let due = format_utc_iso(Utc::now() - ChronoDuration::seconds(1));
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 0,
            scanned: 0,
            updated: 0,
            zero_update_streak: 0,
            next_run_after: &due,
            status: STARTUP_BACKFILL_STATUS_IDLE,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed due startup backfill progress");
    task_name
}

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
            upstream_account_id: None,
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
            upstream_account_id: None,
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
            upstream_account_id: None,
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

    let proxy_usage_backfill_index_sql = sqlx::query_scalar::<_, String>(
        r#"
        SELECT sql
        FROM sqlite_master
        WHERE type = 'index'
          AND name = 'idx_codex_invocations_proxy_usage_backfill_pending'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load proxy usage backfill index");
    assert!(proxy_usage_backfill_index_sql.contains("source"));
    assert!(proxy_usage_backfill_index_sql.contains("status"));
    assert!(proxy_usage_backfill_index_sql.contains("id"));
    assert!(proxy_usage_backfill_index_sql.contains("total_tokens IS NULL"));

    let stale_attempt_index_sql = sqlx::query_scalar::<_, String>(
        r#"
        SELECT sql
        FROM sqlite_master
        WHERE type = 'index'
          AND name = 'idx_pool_upstream_request_attempts_pending_early_phase_started'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load stale attempt recovery index");
    assert!(stale_attempt_index_sql.contains("status"));
    assert!(stale_attempt_index_sql.contains("started_at"));
    assert!(stale_attempt_index_sql.contains("invoke_id"));
    assert!(stale_attempt_index_sql.contains("finished_at IS NULL"));
    assert!(stale_attempt_index_sql.contains("LOWER(TRIM(COALESCE(phase, '')))"));

    let proxy_usage_backfill_plan = sqlx::query(
        r#"
        EXPLAIN QUERY PLAN
        SELECT COALESCE(MAX(id), 0)
        FROM codex_invocations
        WHERE source = 'proxy'
          AND status = 'success'
          AND total_tokens IS NULL
          AND response_raw_path IS NOT NULL
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load proxy usage backfill explain plan")
    .into_iter()
    .map(|row| row.get::<String, _>("detail"))
    .collect::<Vec<_>>()
    .join(" | ");
    assert!(
        proxy_usage_backfill_plan.contains("idx_codex_invocations_proxy_usage_backfill_pending"),
        "unexpected proxy usage backfill plan: {proxy_usage_backfill_plan}"
    );

    let stale_attempt_plan = sqlx::query(
        r#"
        EXPLAIN QUERY PLAN
        SELECT id, invoke_id, occurred_at, sticky_key, upstream_account_id
        FROM pool_upstream_request_attempts
        WHERE status = 'pending'
          AND finished_at IS NULL
          AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
          AND COALESCE(first_byte_latency_ms, 0) <= 0
          AND (
                started_at IS NULL
                OR (endpoint = '/v1/responses' AND started_at <= '2026-03-09 10:00:00')
                OR (endpoint = '/v1/responses/compact' AND started_at <= '2026-03-09 10:00:00')
                OR (COALESCE(endpoint, '') NOT IN ('/v1/responses', '/v1/responses/compact') AND started_at <= '2026-03-09 10:00:00')
          )
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load stale attempt explain plan")
    .into_iter()
    .map(|row| row.get::<String, _>("detail"))
    .collect::<Vec<_>>()
    .join(" | ");
    assert!(
        stale_attempt_plan
            .contains("idx_pool_upstream_request_attempts_pending_early_phase_started"),
        "unexpected stale attempt plan: {stale_attempt_plan}"
    );

    let transport_decode_index_sql = sqlx::query_scalar::<_, String>(
        r#"
        SELECT sql
        FROM sqlite_master
        WHERE type = 'index'
          AND name = 'idx_pool_upstream_request_attempts_transport_decode_recent'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load transport decode recent index");
    assert!(transport_decode_index_sql.contains("upstream_account_id"));
    assert!(transport_decode_index_sql.contains("route_mode"));
    assert!(transport_decode_index_sql.contains("endpoint"));
    assert!(transport_decode_index_sql.contains("phase"));
    assert!(transport_decode_index_sql.contains("occurred_at DESC"));

    let transport_decode_plan = sqlx::query(
        r#"
        EXPLAIN QUERY PLAN
        SELECT failure_kind
        FROM pool_upstream_request_attempts
        WHERE upstream_account_id = 42
          AND route_mode = 'pool'
          AND endpoint = '/v1/responses'
          AND phase IN ('completed', 'failed')
        ORDER BY occurred_at DESC, id DESC
        LIMIT 2
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load transport decode explain plan")
    .into_iter()
    .map(|row| row.get::<String, _>("detail"))
    .collect::<Vec<_>>()
    .join(" | ");
    assert!(
        transport_decode_plan
            .contains("idx_pool_upstream_request_attempts_transport_decode_recent"),
        "unexpected transport decode plan: {transport_decode_plan}"
    );
}

#[tokio::test]
async fn ensure_schema_rebuilds_invocation_in_progress_live_from_existing_invocations() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    for trigger_name in [
        "trg_codex_invocations_live_insert",
        "trg_codex_invocations_live_update",
        "trg_codex_invocations_live_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(&state.pool)
            .await
            .expect("drop live trigger before rebuild test");
    }
    sqlx::query("DROP TABLE IF EXISTS invocation_in_progress_live")
        .execute(&state.pool)
        .await
        .expect("drop live table before rebuild test");

    for (
        invoke_id,
        source,
        status,
        prompt_cache_key,
        upstream_account_id,
        ttfb_ms,
        error_message,
        failure_kind,
    ) in [
        (
            "rebuild-live-failed",
            SOURCE_PROXY,
            "failed",
            "rebuild-live-key-a",
            Some(42_i64),
            Some(810.0_f64),
            Some("upstream response failed"),
            Some("upstream_response_failed"),
        ),
        (
            "rebuild-live-pending",
            SOURCE_PROXY,
            "pending",
            "rebuild-live-key-a",
            Some(42_i64),
            None,
            None,
            None,
        ),
        (
            "rebuild-live-interrupted",
            SOURCE_PROXY,
            "interrupted",
            "rebuild-live-key-b",
            Some(7_i64),
            Some(620.0_f64),
            Some("downstream closed while streaming upstream response"),
            Some("downstream_closed"),
        ),
        (
            "rebuild-live-running",
            SOURCE_XY,
            "running",
            "rebuild-live-key-b",
            Some(7_i64),
            Some(1700.0_f64),
            None,
            None,
        ),
        (
            "rebuild-live-unscoped",
            SOURCE_PROXY,
            "running",
            "rebuild-live-key-c",
            None,
            Some(1100.0_f64),
            None,
            None,
        ),
    ] {
        let mut payload = json!({ "promptCacheKey": prompt_cache_key });
        if let Some(account_id) = upstream_account_id {
            payload["upstreamAccountId"] = json!(account_id);
        }

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                failure_kind,
                t_upstream_ttfb_ms,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(source)
        .bind(status)
        .bind(error_message)
        .bind(failure_kind)
        .bind(ttfb_ms)
        .bind(payload.to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert rebuild source invocation row");
    }

    ensure_schema(&state.pool)
        .await
        .expect("rebuild invocation_in_progress_live on ensure_schema");

    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Option<i64>,
            Option<String>,
            i64,
            i64,
            i64,
            i64,
            Option<f64>,
        ),
    >(
        r#"
        SELECT
            inv.invoke_id,
            live.source,
            live.upstream_account_id,
            live.prompt_cache_key,
            live.is_retry_after_failure_all,
            live.is_retry_after_failure_proxy_only,
            live.is_retry_after_failure_account_all,
            live.is_retry_after_failure_account_proxy_only,
            live.upstream_ttfb_ms
        FROM invocation_in_progress_live AS live
        JOIN codex_invocations AS inv ON inv.id = live.invocation_id
        ORDER BY live.invocation_id
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load rebuilt live rows");

    assert_eq!(
        rows.len(),
        3,
        "only running/pending rows should survive rebuild"
    );

    let pending = rows
        .iter()
        .find(|row| row.0 == "rebuild-live-pending")
        .expect("pending row should be rebuilt");
    assert_eq!(pending.1, SOURCE_PROXY);
    assert_eq!(pending.2, Some(42));
    assert_eq!(pending.3.as_deref(), Some("rebuild-live-key-a"));
    assert_eq!(pending.4, 1);
    assert_eq!(pending.5, 1);
    assert_eq!(pending.6, 1);
    assert_eq!(pending.7, 1);
    assert_eq!(pending.8, None);

    let running = rows
        .iter()
        .find(|row| row.0 == "rebuild-live-running")
        .expect("running row should be rebuilt");
    assert_eq!(running.1, SOURCE_XY);
    assert_eq!(running.2, Some(7));
    assert_eq!(running.3.as_deref(), Some("rebuild-live-key-b"));
    assert_eq!(running.4, 0);
    assert_eq!(running.5, 0);
    assert_eq!(running.6, 0);
    assert_eq!(running.7, 0);
    assert_eq!(running.8, Some(1700.0));

    let unscoped = rows
        .iter()
        .find(|row| row.0 == "rebuild-live-unscoped")
        .expect("unscoped row should be rebuilt");
    assert_eq!(unscoped.1, SOURCE_PROXY);
    assert_eq!(unscoped.2, None);
    assert_eq!(unscoped.3.as_deref(), Some("rebuild-live-key-c"));
    assert_eq!(unscoped.4, 0);
    assert_eq!(unscoped.5, 0);
    assert_eq!(unscoped.6, 0);
    assert_eq!(unscoped.7, 0);
    assert_eq!(unscoped.8, Some(1100.0));
}

#[tokio::test]
async fn ensure_schema_serializes_live_trigger_rebuild_under_concurrent_reentry() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("open concurrent schema migration pool");
    ensure_schema(&pool).await.expect("seed current schema");

    let worker_a = {
        let pool = pool.clone();
        tokio::spawn(async move { ensure_schema(&pool).await })
    };
    let worker_b = {
        let pool = pool.clone();
        tokio::spawn(async move { ensure_schema(&pool).await })
    };

    worker_a
        .await
        .expect("join first concurrent ensure_schema")
        .expect("first concurrent ensure_schema should succeed");
    worker_b
        .await
        .expect("join second concurrent ensure_schema")
        .expect("second concurrent ensure_schema should succeed");

    let trigger_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM sqlite_master
        WHERE type = 'trigger'
          AND name IN (
              'trg_codex_invocations_live_insert',
              'trg_codex_invocations_live_update',
              'trg_codex_invocations_live_delete'
          )
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("count rebuilt live triggers");

    assert_eq!(trigger_count, 3);
}

#[tokio::test]
async fn ensure_schema_rebuilds_prompt_cache_working_set_live_from_existing_invocations() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for trigger_name in [
        "trg_codex_invocations_prompt_cache_working_set_insert",
        "trg_codex_invocations_prompt_cache_working_set_update",
        "trg_codex_invocations_prompt_cache_working_set_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(&state.pool)
            .await
            .expect("drop prompt cache working-set trigger before rebuild test");
    }
    sqlx::query("DROP TABLE IF EXISTS prompt_cache_working_set_live")
        .execute(&state.pool)
        .await
        .expect("drop prompt cache working-set table before rebuild test");

    for (invoke_id, source, status, prompt_cache_key, seconds_ago, total_tokens, cost) in [
        (
            "working-live-recent-success",
            SOURCE_PROXY,
            "success",
            "working-live-key-a",
            30_i64,
            120_i64,
            0.12_f64,
        ),
        (
            "working-live-recent-running",
            SOURCE_PROXY,
            "running",
            "working-live-key-a",
            15_i64,
            140_i64,
            0.14_f64,
        ),
        (
            "working-live-recent-cross-source",
            SOURCE_XY,
            "success",
            "working-live-key-b",
            40_i64,
            220_i64,
            0.22_f64,
        ),
        (
            "working-live-old-terminal",
            SOURCE_PROXY,
            "success",
            "working-live-key-old",
            720_i64,
            320_i64,
            0.32_f64,
        ),
    ] {
        let occurred_at = format_naive(
            (now - ChronoDuration::seconds(seconds_ago))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(source)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(json!({ "promptCacheKey": prompt_cache_key }).to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert prompt cache working-set rebuild source row");
    }

    ensure_schema(&state.pool)
        .await
        .expect("rebuild prompt_cache_working_set_live on ensure_schema");

    let rows = sqlx::query_as::<
        _,
        (
            String,
            i64,
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
            i64,
            f64,
        ),
    >(
        r#"
        SELECT
            prompt_cache_key,
            source_scope_all,
            source_scope_proxy_only,
            created_at,
            last_activity_at,
            last_terminal_at,
            last_in_flight_at,
            request_count,
            total_tokens,
            total_cost
        FROM prompt_cache_working_set_live
        ORDER BY prompt_cache_key
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load rebuilt working-set rows");

    assert_eq!(
        rows.len(),
        2,
        "only recent or in-flight keys should survive rebuild"
    );

    let key_a = rows
        .iter()
        .find(|row| row.0 == "working-live-key-a")
        .expect("working-live-key-a should survive rebuild");
    assert_eq!(key_a.1, 1);
    assert_eq!(key_a.2, 1);
    assert_eq!(key_a.7, 2);
    assert_eq!(key_a.8, 260);
    assert!((key_a.9 - 0.26).abs() < 1e-9);
    assert!(key_a.5.is_some());
    assert!(key_a.6.is_some());

    let key_b = rows
        .iter()
        .find(|row| row.0 == "working-live-key-b")
        .expect("working-live-key-b should survive rebuild");
    assert_eq!(key_b.1, 1);
    assert_eq!(key_b.2, 0);
    assert_eq!(key_b.7, 1);
    assert_eq!(key_b.8, 220);
    assert!((key_b.9 - 0.22).abs() < 1e-9);
    assert!(key_b.5.is_some());
    assert!(key_b.6.is_none());
}

#[tokio::test]
async fn proxy_only_working_conversation_live_aggregate_keeps_mixed_source_proxy_slice() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    for (invoke_id, source, seconds_ago, total_tokens, cost) in [
        ("mixed-proxy", SOURCE_PROXY, 30_i64, 120_i64, 0.12_f64),
        ("mixed-secondary", SOURCE_XY, 20_i64, 220_i64, 0.22_f64),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            (now - ChronoDuration::seconds(seconds_ago))
                .with_timezone(&Shanghai)
                .naive_local(),
        ))
        .bind(source)
        .bind("success")
        .bind(total_tokens)
        .bind(cost)
        .bind(json!({ "promptCacheKey": "mixed-working-key" }).to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert mixed-source working conversation row");
    }

    let rows = query_prompt_cache_working_conversation_aggregates(
        &state.pool,
        &db_occurred_at_lower_bound(now - ChronoDuration::minutes(5)),
        InvocationSourceScope::ProxyOnly,
        10,
    )
    .await
    .expect("proxy-only working conversation aggregate should succeed");

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.prompt_cache_key, "mixed-working-key");
    assert_eq!(row.request_count, 1);
    assert_eq!(row.total_tokens, 120);
    assert!((row.total_cost - 0.12).abs() < 1e-9);
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
async fn startup_hot_read_hydration_keeps_health_ready_under_sqlite_pool_pressure() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "startup-hot-read-readiness-pressure",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    state.startup_ready.store(false, Ordering::Release);
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server before startup hydration");
    let mut held_connections = Vec::new();
    for _ in 0..5 {
        held_connections.push(
            state
                .pool
                .acquire()
                .await
                .expect("saturate sqlite pool before startup hydration"),
        );
    }

    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_deadline(
            state.clone(),
            Instant::now(),
            Duration::from_secs(4),
        );
    let health = tokio::time::timeout(
        Duration::from_millis(250),
        reqwest::get(format!("http://{addr}/health")),
    )
    .await
    .expect("health must not wait for SQLite hydration")
    .expect("health endpoint should respond while SQLite pool is saturated");
    assert_eq!(health.status(), StatusCode::OK);

    for endpoint in ["/api/stats/summary?window=today", "/api/system/status"] {
        let response = tokio::time::timeout(
            Duration::from_millis(250),
            reqwest::get(format!("http://{addr}{endpoint}")),
        )
        .await
        .expect("hot-read request must not wait for SQLite during startup")
        .expect("hot-read endpoint should return its unavailable contract");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        !hydration_handle.is_finished(),
        "the background worker retries after its bounded hydration attempt times out"
    );

    drop(held_connections);
    tokio::time::timeout(Duration::from_secs(5), hydration_handle)
        .await
        .expect("background startup hydration should recover after SQLite pressure clears")
        .expect("background startup hydration task should join");

    for endpoint in ["/api/stats/summary?window=today", "/api/system/status"] {
        let response = reqwest::get(format!("http://{addr}{endpoint}"))
            .await
            .expect("hydrated hot-read endpoint should respond");
        assert_eq!(response.status(), StatusCode::OK);
    }

    state.shutdown.cancel();
    server_handle
        .await
        .expect("http server should stop after startup pressure test");
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_summary_hydration_retries_after_an_in_flight_refresh() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "startup-summary-hydration-single-flight",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let refresh_guard = state
        .subscription_hub
        .try_lock_summary_projection_refresh()
        .expect("hold the summary maintenance refresh lock");

    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration(state.clone(), Instant::now());
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state.system_status_cache.lock().await.latest.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("system status hydration should complete while summary refresh is in flight");
    assert!(
        !hydration_handle.is_finished(),
        "startup hydration must retry instead of treating a coalesced summary refresh as complete"
    );

    drop(refresh_guard);
    tokio::time::timeout(Duration::from_secs(3), hydration_handle)
        .await
        .expect("startup hydration should retry after the in-flight refresh releases")
        .expect("startup hydration task should join after the retry");

    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_system_status_hydrates_while_summary_hydration_is_delayed() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18080").expect("valid upstream url"),
    )
    .await;
    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_delay(
            state.clone(),
            Instant::now(),
            Duration::from_secs(2),
        );

    tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            if state.system_status_cache.lock().await.latest.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("system status hydration must not wait for the delayed summary worker");
    assert!(
        !hydration_handle.is_finished(),
        "the returned coordinator must continue joining the delayed summary worker"
    );

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), hydration_handle)
        .await
        .expect("startup hydration coordinator should cancel both independent workers")
        .expect("startup hydration coordinator should join after shutdown");
    state.pool.close().await;
}

#[tokio::test]
async fn runtime_drain_joins_pending_startup_hydration_after_http_readiness() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18080").expect("valid upstream url"),
    )
    .await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn HTTP server before startup hydration");
    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_delay(
            state.clone(),
            Instant::now(),
            Duration::from_secs(30),
        );

    let health = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond after readiness is published");
    assert_eq!(health.status(), StatusCode::OK);
    assert!(
        !hydration_handle.is_finished(),
        "the delayed startup hydration coordinator should still be pending at shutdown"
    );

    begin_runtime_shutdown(&state.shutdown);
    tokio::time::timeout(
        Duration::from_secs(1),
        drain_runtime_after_shutdown(
            state.clone(),
            Some(server_handle),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(hydration_handle),
        ),
    )
    .await
    .expect("runtime drain should not wait for abandoned startup hydration subwork")
    .expect("runtime drain should join the pending startup hydration coordinator");

    state.pool.close().await;
}

#[tokio::test]
async fn summary_startup_hydration_has_a_finite_sqlite_pressure_deadline() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "startup-summary-hydration-deadline",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let mut held_connections = Vec::new();
    for _ in 0..5 {
        held_connections.push(
            state
                .pool
                .acquire()
                .await
                .expect("saturate sqlite pool before summary hydration"),
        );
    }

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        hydrate_summary_snapshots_with_deadline(state.as_ref(), Duration::from_secs(4)),
    )
    .await
    .expect("summary hydration must finish its bounded attempt under SQLite pressure")
    .expect_err("saturated SQLite pool should fail the bounded summary hydration attempt");
    assert!(
        result.to_string().contains("exceeded"),
        "summary hydration should report its finite build deadline: {result:#}"
    );

    drop(held_connections);
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn system_status_startup_hydration_has_a_finite_sqlite_pressure_deadline() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "startup-system-status-hydration-deadline",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let mut held_connections = Vec::new();
    for _ in 0..5 {
        held_connections.push(
            state
                .pool
                .acquire()
                .await
                .expect("saturate sqlite pool before system status hydration"),
        );
    }

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        hydrate_system_status_snapshot(state.as_ref()),
    )
    .await
    .expect("system status hydration must finish its bounded attempt under SQLite pressure")
    .expect_err("saturated SQLite pool should fail the bounded system status hydration attempt");
    assert!(
        result.to_string().contains("exceeded"),
        "system status hydration should report its finite refresh deadline: {result:#}"
    );

    drop(held_connections);
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_hot_read_hydration_cancels_while_sqlite_pool_is_saturated() {
    let (state, temp_dir, _db_url) = file_backed_test_state_with_busy_timeout(
        "startup-hot-read-hydration-cancellation",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let mut held_connections = Vec::new();
    for _ in 0..5 {
        held_connections.push(
            state
                .pool
                .acquire()
                .await
                .expect("saturate sqlite pool before startup hydration"),
        );
    }

    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration(state.clone(), Instant::now());
    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), hydration_handle)
        .await
        .expect("startup hydration should observe shutdown without waiting for SQLite")
        .expect("startup hydration task should join after shutdown");

    drop(held_connections);
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

const HOURLY_ROLLUP_BOOTSTRAP_TASK_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const HOURLY_ROLLUP_BOOTSTRAP_TASK_POLL_INTERVAL: Duration = Duration::from_millis(10);

async fn wait_for_hourly_rollup_bootstrap_task(
    state: &AppState,
    expected_status: &str,
) -> (String, Option<String>, Option<String>) {
    let mut last_task = None;
    let result = tokio::time::timeout(HOURLY_ROLLUP_BOOTSTRAP_TASK_WAIT_TIMEOUT, async {
        loop {
            if expected_status != "running" {
                state
                    .sqlite_batch_writer
                    .flush_now(&state.pool)
                    .await
                    .expect("flush deferred hourly rollup bootstrap task-history writes");
            }
            let task = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
                r#"
                SELECT status, summary, detail
                FROM system_task_runs
                WHERE task_kind = 'hourly_rollup_bootstrap'
                ORDER BY id DESC
                LIMIT 1
                "#,
            )
            .fetch_optional(&state.pool)
            .await
            .expect("read hourly rollup bootstrap task");
            last_task.clone_from(&task);
            if let Some(task) = task
                && task.0 == expected_status
            {
                return task;
            }
            tokio::time::sleep(HOURLY_ROLLUP_BOOTSTRAP_TASK_POLL_INTERVAL).await;
        }
    })
    .await;
    if result.is_err() {
        let pending = state.sqlite_batch_writer.accounting_snapshot();
        let all_tasks = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>)>(
            r#"
            SELECT id, status, summary, detail
            FROM system_task_runs
            WHERE task_kind = 'hourly_rollup_bootstrap'
            ORDER BY id DESC
            LIMIT 5
            "#,
        )
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default();
        panic!(
            "hourly rollup bootstrap task should reach expected status {expected_status}; last_task={last_task:?}; all_tasks={all_tasks:?}; pending_depth={}; pending_bytes={}",
            pending.pending_depth, pending.pending_bytes,
        )
    }
    result.expect("hourly rollup bootstrap task should reach the expected state")
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_keeps_health_ready_while_waiting_for_lock() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    let (status, _, _) = wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;
    assert_eq!(status, "running");

    let health = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond while rollup bootstrap waits for the lock");
    assert_eq!(health.status(), StatusCode::OK);

    drop(rollup_guard);
    tokio::time::timeout(Duration::from_secs(5), bootstrap_handle)
        .await
        .expect("background bootstrap should finish")
        .expect("background bootstrap task should join");
    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "success").await;
    assert_eq!(status, "success");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("hourly_rollups_ms="))
    );
    assert!(detail.is_none());

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_records_failure_without_revoking_readiness() {
    let state = test_state_from_config(test_config(), false).await;
    state.startup_ready.store(true, Ordering::Release);
    sqlx::query("DROP TABLE upstream_account_stats_hourly")
        .execute(&state.pool)
        .await
        .expect("remove account rollup table to force bootstrap failure");

    spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone())
        .await
        .expect("background bootstrap task should join");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "failed").await;
    assert_eq!(status, "failed");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("existing rollups remain available"))
    );
    assert!(
        detail.is_some(),
        "failure should retain an observable error"
    );
    assert_eq!(
        health_check(State(state.clone()))
            .await
            .into_response()
            .status(),
        StatusCode::OK,
        "background bootstrap failure must not return health to starting"
    );

    state.shutdown.cancel();
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_cancels_while_waiting_for_lock() {
    let state = test_state_from_config(test_config(), false).await;
    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should stop after cancellation")
        .expect("background bootstrap task should join");
    drop(rollup_guard);
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred cancellation task-history finish");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before acquiring"))
    );
    assert!(detail.is_none());
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_cancels_while_task_history_start_is_locked() {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-hourly-rollup-bootstrap-task-history-lock",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect task-history lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire task-history write lock");

    let mut bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    tokio::time::timeout(Duration::from_millis(100), &mut bootstrap_handle)
        .await
        .expect_err("bootstrap should wait for the task-history write lock");

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should stop without waiting for sqlite busy timeout")
        .expect("background bootstrap task should join");

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release task-history write lock");
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_defers_task_history_finish_during_shutdown_lock()
 {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-hourly-rollup-bootstrap-task-history-finish-lock",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;
    let bootstrap_handle =
        spawn_runtime_startup_hourly_rollup_bootstrap(state.clone(), state.shutdown.clone());
    wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "running").await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect task-history lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire task-history write lock");

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), bootstrap_handle)
        .await
        .expect("background bootstrap should defer its task-history finish during shutdown")
        .expect("background bootstrap task should join");
    drop(rollup_guard);

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release task-history write lock");
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred task-history finish");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before acquiring"))
    );
    assert!(detail.is_none());

    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_hourly_rollup_task_history_finish_defers_when_cancellation_races_with_write_lock()
{
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-hourly-rollup-bootstrap-task-history-finish-cancel-race",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let task_run = begin_system_task_run(
        &state.pool,
        SystemTaskKind::HourlyRollupBootstrap,
        "startup",
        Some("background hourly rollup bootstrap started".to_string()),
    )
    .await
    .expect("record task-history start");
    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect task-history lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire task-history write lock");

    let cancel = CancellationToken::new();
    let state_for_finish = state.clone();
    let cancel_for_finish = cancel.clone();
    let mut finish_handle = tokio::spawn(async move {
        finish_runtime_startup_hourly_rollup_bootstrap_task(
            state_for_finish.as_ref(),
            &cancel_for_finish,
            Some(&task_run),
            SystemTaskStatus::Skipped,
            "background hourly rollup bootstrap cancelled before acquiring its synchronization lock",
            None,
        )
        .await;
    });
    tokio::time::timeout(Duration::from_millis(100), &mut finish_handle)
        .await
        .expect_err("task-history finish should wait for the sqlite write lock");

    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(1), finish_handle)
        .await
        .expect("task-history finish should defer after cancellation")
        .expect("task-history finish task should join");

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release task-history write lock");
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred task-history finish");

    let (status, summary, detail) =
        wait_for_hourly_rollup_bootstrap_task(state.as_ref(), "skipped").await;
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before acquiring"))
    );
    assert!(detail.is_none());

    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn retention_run_once_keeps_blocking_hourly_rollup_bootstrap() {
    let state = test_state_from_config(test_config(), false).await;
    let cli = CliArgs {
        retention_run_once: true,
        ..CliArgs::default()
    };

    let summary = run_startup_persistent_prep(&state.pool, &state.config, &cli)
        .await
        .expect("retention run-once startup prep should finish");
    assert!(summary.bootstrapped_hourly_rollups);

    state.shutdown.cancel();
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
async fn startup_historical_rollup_backfill_prioritizes_usage_breakdown_repair() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18082").expect("valid upstream url"),
    )
    .await;
    let archived_occurred_at = shanghai_local_days_ago(120, 9, 0, 0);
    let payload = json!({
        "upstreamAccountId": 17,
        "upstreamAccountName": "Replay",
        "promptCacheKey": "pck-startup-priority",
        "stickyKey": "sticky-startup-priority",
        "responseModel": "gpt-5.4",
        "reasoningEffort": "high",
    })
    .to_string();
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "startup-priority-usage-breakdown",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "startup-priority-usage-breakdown",
            occurred_at: &archived_occurred_at,
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 42_i64,
            cost: 0.42_f64,
            ttfb_ms: Some(120.0),
            payload: Some(payload.as_str()),
            detail_level: "summary",
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let snapshot_before = load_historical_rollup_backfill_snapshot(&state.pool, &state.config)
        .await
        .expect("load startup historical rollup snapshot before priority pass");
    assert_eq!(snapshot_before.legacy_archive_pending, 1);
    assert_eq!(snapshot_before.pending_usage_breakdown_batches, 1);

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::HistoricalRollups)
        .await
        .expect("startup historical rollup backfill should run priority breakdown repair");

    let snapshot_after = load_historical_rollup_backfill_snapshot(&state.pool, &state.config)
        .await
        .expect("load startup historical rollup snapshot after priority pass");
    assert_eq!(snapshot_after.pending_usage_breakdown_batches, 0);
    assert_eq!(snapshot_after.legacy_archive_pending, 1);

    let task_name =
        startup_backfill_task_progress_key(state.as_ref(), StartupBackfillTask::HistoricalRollups)
            .await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load startup historical rollup progress after priority pass");
    assert_eq!(progress.last_scanned, 1);
    assert!(progress.last_updated > 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
}

async fn seed_missing_historical_rollup_startup_candidates(pool: &SqlitePool, count: usize) -> i64 {
    let mut tx = pool
        .begin()
        .await
        .expect("begin startup candidate seed transaction");
    for index in 0..count {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status, created_at
            )
            VALUES ('forward_proxy_attempts', '2025-01', ?1, ?2, 1, 'completed', datetime('now'))
            "#,
        )
        .bind(format!(
            "/missing/historical-rollup-startup-{index}.sqlite.gz"
        ))
        .bind(format!("startup-candidate-{index}"))
        .execute(tx.as_mut())
        .await
        .expect("insert missing historical rollup startup candidate");
    }
    let last_id = sqlx::query_scalar::<_, i64>("SELECT MAX(id) FROM archive_batches")
        .fetch_one(tx.as_mut())
        .await
        .expect("load last startup candidate id");
    tx.commit()
        .await
        .expect("commit startup candidate seed transaction");
    last_id
}

#[tokio::test]
async fn historical_rollup_startup_window_bounds_a_twenty_thousand_batch_fixture_and_wraps() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let last_id = seed_missing_historical_rollup_startup_candidates(&state.pool, 20_000).await;

    let pending_hint = count_historical_rollup_startup_pending_hint(&state.pool)
        .await
        .expect("load bounded historical rollup startup pending hint");
    assert_eq!(pending_hint.pending_archive_batches, 0);
    assert_eq!(pending_hint.candidate_count, 32);
    assert_eq!(pending_hint.inspected_path_count, 32);
    assert!(pending_hint.candidate_count <= 32);
    assert!(pending_hint.inspected_path_count <= 32);

    let first =
        materialize_historical_rollups_startup_window(&state.pool, 0, Duration::from_secs(6))
            .await
            .expect("run bounded historical rollup startup window");
    assert_eq!(first.candidate_count, 32);
    assert_eq!(first.inspected_path_count, 16);
    assert_eq!(first.summary.scanned_archive_batches, 16);
    assert!(first.candidate_count <= 32);
    assert!(first.inspected_path_count <= 32);
    assert!(first.summary.scanned_archive_batches > 2);
    assert!(first.next_cursor_id > 0);

    let resumed = materialize_historical_rollups_startup_window(
        &state.pool,
        first.next_cursor_id,
        Duration::from_secs(6),
    )
    .await
    .expect("resume bounded historical rollup startup window");
    assert_eq!(resumed.candidate_count, 32);
    assert_eq!(resumed.inspected_path_count, 16);
    assert!(resumed.next_cursor_id > first.next_cursor_id);

    let wrapped =
        materialize_historical_rollups_startup_window(&state.pool, last_id, Duration::from_secs(6))
            .await
            .expect("wrap bounded historical rollup startup window");
    assert!(wrapped.wrapped);
    assert_eq!(wrapped.candidate_count, 32);
    assert_eq!(wrapped.inspected_path_count, 16);
    assert!(wrapped.next_cursor_id < last_id);
}

#[tokio::test]
async fn historical_rollup_startup_window_advances_in_keyset_order_despite_usage_breakdown_candidates()
 {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_missing_historical_rollup_startup_candidates(&state.pool, 20).await;
    let sixteenth_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'forward_proxy_attempts' ORDER BY id LIMIT 1 OFFSET 15",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load sixteenth forward proxy candidate id");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status, created_at
        )
        VALUES ('codex_invocations', '2025-01', '/missing/usage-breakdown-priority.sqlite.gz', 'usage-breakdown-priority', 1, 'completed', datetime('now'))
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("insert usage breakdown candidate after the first keyset batch");

    let first =
        materialize_historical_rollups_startup_window(&state.pool, 0, Duration::from_secs(6))
            .await
            .expect("run first ordered historical rollup startup window");
    assert_eq!(first.inspected_path_count, 16);
    assert_eq!(first.next_cursor_id, sixteenth_id);

    let second = materialize_historical_rollups_startup_window(
        &state.pool,
        first.next_cursor_id,
        Duration::from_secs(6),
    )
    .await
    .expect("resume ordered historical rollup startup window");
    assert_eq!(second.inspected_path_count, 5);
    assert!(second.next_cursor_id > first.next_cursor_id);
}

#[tokio::test]
async fn historical_rollup_startup_window_preserves_cursor_when_budget_is_exhausted() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_missing_historical_rollup_startup_candidates(&state.pool, 32).await;

    let blocked = materialize_historical_rollups_startup_window(&state.pool, 0, Duration::ZERO)
        .await
        .expect("run budget-exhausted historical rollup startup window");
    assert_eq!(blocked.candidate_count, 32);
    assert_eq!(blocked.inspected_path_count, 0);
    assert_eq!(blocked.next_cursor_id, 0);
    assert!(blocked.hit_budget);
}

#[tokio::test]
async fn startup_historical_rollup_backfill_persists_cursor_and_defers_under_pressure() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_missing_historical_rollup_startup_candidates(&state.pool, 32).await;
    let task = StartupBackfillTask::HistoricalRollups;
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(1));
    let held = gate
        .try_begin_background("test_historical_rollup_pressure")
        .expect("hold startup backfill gate");

    assert!(
        !run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
            .await
            .expect("defer historical startup backfill under pressure")
    );
    let deferred = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load pressure-deferred historical progress");
    assert_eq!(deferred.cursor_id, 0);
    assert!(
        deferred.is_due(Utc::now()),
        "a pre-SQL pressure defer must leave durable progress untouched; the scheduler owns its deadline"
    );
    drop(held);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("make historical startup backfill due");
    assert!(
        !run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
            .await
            .expect("run bounded historical startup backfill")
    );
    let resumed = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load resumed historical progress");
    assert!(resumed.cursor_id > 0);
    assert_eq!(resumed.last_scanned, 16);
}

#[tokio::test]
async fn historical_rollup_noop_pass_does_not_create_a_system_task_run() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_missing_historical_rollup_startup_candidates(&state.pool, 16).await;
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count startup task runs before historical noop pass");

    let cancel = CancellationToken::new();
    let outcome = run_startup_backfill_maintenance_pass(state.clone(), &cancel, None).await;
    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count startup task runs after historical noop pass");
    assert!(!outcome.ran_actionable_task);
    assert!(!outcome.had_failure);
    assert_eq!(after, before);
}

#[tokio::test]
async fn startup_backfill_not_due_check_does_not_claim_background_gate() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::ReasoningEffort;
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let next_run_after = format_utc_iso(Utc::now() + ChronoDuration::hours(6));
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 0,
            scanned: 0,
            updated: 0,
            zero_update_streak: 0,
            next_run_after: &next_run_after,
            status: STARTUP_BACKFILL_STATUS_OK,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed not-due startup progress");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(1));
    let held_permit = gate
        .try_begin_background("upstream_account_maintenance")
        .expect("hold local gate slot");

    run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
        .await
        .expect("not-due startup backfill should not require a background slot");

    assert_eq!(
        gate.try_begin_background("upstream_account_maintenance")
            .unwrap_err(),
        crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
        "not-due backfill should leave the already held slot untouched"
    );
    drop(held_permit);
}

#[tokio::test]
async fn startup_backfill_idle_pass_does_not_create_a_system_task_run() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count startup backfill task runs before idle pass");

    let cancel = CancellationToken::new();
    let outcome = run_startup_backfill_maintenance_pass(state.clone(), &cancel, None).await;

    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count startup backfill task runs after idle pass");
    assert!(!outcome.ran_actionable_task);
    assert!(!outcome.had_failure);
    assert_eq!(
        after, before,
        "idle maintenance must not create task-run audit rows"
    );
}

#[tokio::test]
async fn startup_backfill_event_wakes_only_the_matching_task() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let archive_task = StartupBackfillTask::UpstreamActivityArchives;
    let historical_task = StartupBackfillTask::HistoricalRollups;
    let suspended_until = format_utc_iso(Utc::now() + ChronoDuration::days(1));

    for task in [archive_task, historical_task] {
        save_startup_backfill_progress(
            &state.pool,
            task.name(),
            StartupBackfillProgressUpdate {
                cursor_id: 7,
                scanned: 100,
                updated: 0,
                zero_update_streak: 4,
                next_run_after: &suspended_until,
                status: STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE,
                suspension_reason: Some("source_unavailable"),
            },
        )
        .await
        .expect("seed source-unavailable startup backfill progress");
    }

    wake_startup_backfill_tasks(&state.pool, &[archive_task], "test_archive_available")
        .await
        .expect("wake affected archive task");

    let archive_progress = load_startup_backfill_progress(&state.pool, archive_task.name())
        .await
        .expect("load woken archive progress");
    assert!(archive_progress.is_due(Utc::now()));
    assert_eq!(archive_progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
    assert_eq!(archive_progress.suspension_reason, None);
    assert_eq!(archive_progress.next_probe_at, None);
    assert_eq!(archive_progress.wake_generation, 1);

    let historical_progress = load_startup_backfill_progress(&state.pool, historical_task.name())
        .await
        .expect("load unaffected historical progress");
    assert!(!historical_progress.is_due(Utc::now()));
    assert_eq!(
        historical_progress.last_status,
        STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE
    );
}

#[tokio::test]
async fn startup_backfill_pressure_defer_never_accesses_sqlite() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::ReasoningEffort;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(1));
    let _held = gate
        .try_begin_background("test_pressure")
        .expect("hold background slot");
    state.pool.close().await;

    let ran = run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
        .await
        .expect("closed pool proves the pressure defer does not access SQLite");
    assert!(!ran);
}

#[tokio::test]
async fn startup_backfill_pressure_defer_has_one_deadline_and_no_task_run_audit() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::ReasoningEffort;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    gate.record_pressure("test_pressure", "forced_cooldown");
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before pressure defer");

    let cancel = CancellationToken::new();
    let selected_tasks = [task];
    let first = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        &cancel,
        Some(&selected_tasks),
        &gate,
    )
    .await;
    let second = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        &cancel,
        Some(&selected_tasks),
        &gate,
    )
    .await;
    let after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after pressure defer");
    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load untouched progress after scheduler-only pressure defer");

    assert!(!first.ran_actionable_task);
    assert!(!first.had_failure);
    assert!(!second.ran_actionable_task);
    assert!(!second.had_failure);
    assert_eq!(after, before, "deferred passes must not create audit rows");
    assert!(progress.is_due(Utc::now()));
    assert_eq!(progress.suspension_reason, None);
}

#[tokio::test]
async fn startup_backfill_repeated_cooldown_notifications_do_not_redispatch_or_read_sqlite() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::InvocationServiceTier;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    gate.record_pressure("test_pressure", "forced_cooldown");
    let cancel = CancellationToken::new();
    let selected_tasks = [task];
    let audits_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before repeated cooldown notifications");

    let first = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        &cancel,
        Some(&selected_tasks),
        &gate,
    )
    .await;
    let audits_after_first: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after the first cooldown defer");
    assert!(!first.ran_actionable_task);
    assert!(!first.had_failure);
    assert_eq!(audits_after_first, audits_before);
    assert_eq!(gate.snapshot().background_skips, 1);

    // A closed pool turns every repeated event path into a zero-I/O assertion. The scheduler
    // must retain the original registration instead of reaching either the wake SQL or task read.
    state.pool.close().await;
    for _ in 0..3 {
        assert_eq!(
            wake_startup_backfill_tasks_with_gate(
                &state.pool,
                &selected_tasks,
                "repeated_cooldown_input",
                &gate,
            )
            .await
            .expect("a pending pressure defer must suppress input wake SQLite work"),
            0
        );
        gate.notify_background_eligibility();
        run_pressure_eligible_startup_backfill_tasks(state.clone(), &cancel, &gate).await;
    }
    assert_eq!(
        gate.snapshot().background_skips,
        1,
        "repeated notifications in one closed cooldown must not redispatch the deferred task"
    );
}

#[tokio::test]
async fn startup_backfill_input_wake_busy_admission_avoids_sqlite() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::PoolAttemptPublicIdLive;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let _held = gate
        .try_begin_background("test_input_wake_busy")
        .expect("occupy the sole background slot");

    state.pool.close().await;
    assert_eq!(
        wake_startup_backfill_tasks_with_gate(
            &state.pool,
            &[task],
            "input_wake_busy_admission",
            &gate,
        )
        .await
        .expect("busy admission must defer without SQLite access"),
        0
    );
    assert_eq!(
        gate.snapshot().background_skips,
        1,
        "the input wake must be denied by the gate before its progress write"
    );
}

#[tokio::test]
async fn pressure_eligibility_wake_rechecks_durable_backfill_deadline() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::PoolUpstreamNodeHealthArchives;
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let future_due = format_utc_iso(Utc::now() + ChronoDuration::hours(1));
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 17,
            scanned: 8,
            updated: 3,
            zero_update_streak: 0,
            next_run_after: &future_due,
            status: STARTUP_BACKFILL_STATUS_FAILED,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed a durable future failure backoff");
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before pressure eligibility wake");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let held = gate
        .try_begin_background("test_pressure_eligibility_busy")
        .expect("occupy the background slot");
    let first = run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
        .await
        .expect("background-busy admission should be a scheduler-only defer");
    assert!(!first);
    assert_eq!(gate.snapshot().background_skips, 1);

    let observed_eligibility = gate.eligibility_generation();
    drop(held);
    tokio::time::timeout(
        Duration::from_secs(1),
        gate.wait_for_eligibility_change(observed_eligibility),
    )
    .await
    .expect("background-slot release should emit an eligibility event");

    let cancel = CancellationToken::new();
    run_pressure_eligible_startup_backfill_tasks(state.clone(), &cancel, &gate).await;

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load progress after pressure eligibility wake");
    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after pressure eligibility wake");
    assert_eq!(progress.cursor_id, 17);
    assert_eq!(progress.last_scanned, 8);
    assert_eq!(progress.last_updated, 3);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_FAILED);
    assert_eq!(
        progress.next_run_after.as_deref(),
        Some(future_due.as_str())
    );
    assert!(!progress.is_due(Utc::now()));
    assert_eq!(
        task_runs_after, task_runs_before,
        "an eligibility event must not bypass a real durable failure backoff"
    );
}

#[tokio::test]
async fn startup_backfill_progress_lookup_busy_closes_gate_before_permit_release() {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-backfill-progress-lookup-busy",
        Duration::from_millis(50),
    )
    .await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect progress lookup lock holder");
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut lock_conn)
        .await
        .expect("begin exclusive schema lock transaction");
    sqlx::query("CREATE TABLE startup_backfill_progress_lookup_lock_guard (id INTEGER)")
        .execute(&mut lock_conn)
        .await
        .expect("hold the schema lock across the progress lookup");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_backfill_busy_error_closes_gate_before_next_task(
        &state,
        StartupBackfillTask::ReasoningEffort,
        &gate,
    )
    .await;

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release progress lookup lock");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_backfill_running_state_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::ReasoningEffort).await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_running_state_busy
        BEFORE UPDATE OF last_status ON startup_backfill_progress
        WHEN NEW.task_name = 'proxy_reasoning_effort_v1' AND NEW.last_status = 'running'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into running-state persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_backfill_busy_error_closes_gate_before_next_task(
        &state,
        StartupBackfillTask::ReasoningEffort,
        &gate,
    )
    .await;
}

#[tokio::test]
async fn startup_backfill_progress_persist_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::ReasoningEffort).await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_progress_persist_busy
        BEFORE UPDATE OF last_status ON startup_backfill_progress
        WHEN NEW.task_name = 'proxy_reasoning_effort_v1' AND NEW.last_status = 'ok'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into backfill progress persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_backfill_busy_error_closes_gate_before_next_task(
        &state,
        StartupBackfillTask::ReasoningEffort,
        &gate,
    )
    .await;
}

#[tokio::test]
async fn startup_backfill_failure_persist_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::ReasoningEffort).await;
    let temp_dir = make_temp_test_dir("startup-backfill-failure-persist-busy");
    let request_path = temp_dir.join("request.json");
    fs::write(
        &request_path,
        r#"{"model":"gpt-5.3-codex","reasoning":{"effort":"low"}}"#,
    )
    .expect("write reasoning backfill request");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("startup-backfill-failure-persist-busy")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .bind(request_path.to_string_lossy().as_ref())
    .execute(&state.pool)
    .await
    .expect("insert reasoning backfill candidate");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_inner_busy
        BEFORE UPDATE OF payload ON codex_invocations
        WHEN NEW.invoke_id = 'startup-backfill-failure-persist-busy'
        BEGIN
            SELECT RAISE(ABORT, 'database is busy');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite busy failure into backfill work");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_failure_persist_busy
        BEFORE UPDATE OF last_status ON startup_backfill_progress
        WHEN NEW.task_name = 'proxy_reasoning_effort_v1' AND NEW.last_status = 'failed'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into failure persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_backfill_busy_error_closes_gate_before_next_task(
        &state,
        StartupBackfillTask::ReasoningEffort,
        &gate,
    )
    .await;

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_backfill_busy_failure_persists_failed_state_and_bounded_retry() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::ReasoningEffort;
    let task_name = startup_backfill_task_progress_key(state.as_ref(), task).await;
    let suspended_until = format_utc_iso(Utc::now() + ChronoDuration::hours(1));
    save_startup_backfill_progress(
        &state.pool,
        &task_name,
        StartupBackfillProgressUpdate {
            cursor_id: 12,
            scanned: 4,
            updated: 0,
            zero_update_streak: 2,
            next_run_after: &suspended_until,
            status: STARTUP_BACKFILL_STATUS_IDLE,
            suspension_reason: Some("sqlite_pressure_cooldown"),
        },
    )
    .await
    .expect("seed pressure-deferred progress");
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load seeded startup backfill progress");
    let before_retry = Utc::now();
    persist_startup_backfill_task_failure(
        &state,
        task,
        &task_name,
        &progress,
        std::time::Instant::now(),
        &anyhow::anyhow!("database table is locked"),
    )
    .await
    .expect("record SQLite locked backfill failure");

    let failed = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load failed startup backfill progress");
    let retry_at = failed
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .expect("parse bounded failure retry");
    assert_eq!(failed.last_status, STARTUP_BACKFILL_STATUS_FAILED);
    assert_eq!(failed.suspension_reason, None);
    assert!(
        retry_at >= before_retry + ChronoDuration::seconds(10),
        "SQLite BUSY/LOCKED must use the bounded failure retry rather than a pressure defer"
    );
}

#[tokio::test]
async fn startup_backfill_busy_failure_closes_pressure_gate_before_the_next_task_reads_sqlite() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let temp_dir = make_temp_test_dir("startup-backfill-busy-pressure-gate");
    let request_path = temp_dir.join("request.json");
    fs::write(
        &request_path,
        r#"{"model":"gpt-5.3-codex","reasoning":{"effort":"low"}}"#,
    )
    .expect("write reasoning backfill request");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("startup-backfill-injected-busy")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .bind(request_path.to_string_lossy().as_ref())
    .execute(&state.pool)
    .await
    .expect("insert reasoning backfill candidate");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_backfill_injected_busy
        BEFORE UPDATE OF payload ON codex_invocations
        WHEN NEW.invoke_id = 'startup-backfill-injected-busy'
        BEGIN
            SELECT RAISE(ABORT, 'database is busy');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject an actual SQLite busy failure into backfill work");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let cancel = CancellationToken::new();
    let first_tasks = [StartupBackfillTask::ReasoningEffort];
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before injected busy failure");

    let first = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        &cancel,
        Some(&first_tasks),
        &gate,
    )
    .await;
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush injected busy failure task audit");

    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after injected busy failure");
    let task_run_status: String = sqlx::query_scalar(
        "SELECT status FROM system_task_runs WHERE task_kind = 'startup_backfill' ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load injected busy failure task audit");
    let failed_progress = load_startup_backfill_progress(&state.pool, first_tasks[0].name())
        .await
        .expect("load persisted busy failure progress");
    assert!(!first.ran_actionable_task);
    assert!(first.had_failure);
    assert_eq!(task_runs_after, task_runs_before + 1);
    assert_eq!(task_run_status, "failed");
    assert_eq!(failed_progress.last_status, STARTUP_BACKFILL_STATUS_FAILED);
    assert!(
        gate.pressure_cooldown_deadline_epoch_ms().is_some(),
        "the durable busy failure must close the gate before its permit is released"
    );

    // A closed pool turns this into a zero-SQL admission assertion. The only valid result is a
    // scheduler-only pressure defer that waits for the gate deadline or eligibility event.
    state.pool.close().await;
    let second_tasks = [StartupBackfillTask::PromptCacheKey];
    let second = run_startup_backfill_maintenance_pass_with_gate(
        state.clone(),
        &cancel,
        Some(&second_tasks),
        &gate,
    )
    .await;
    assert!(!second.ran_actionable_task);
    assert!(!second.had_failure);
    assert!(
        gate.snapshot().background_skips >= 1,
        "the next task must be refused by the closed gate without touching SQLite"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_coverage_lock_maintenance_pass_defers_without_audit_but_non_lock_failure_audits() {
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let cancel = CancellationToken::new();
    let selected_tasks = [task];

    let pressure_state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&pressure_state, task).await;
    sqlx::query(
        "DELETE FROM hourly_rollup_live_progress WHERE dataset = 'invocation_account_activity_v2_repair_generation'",
    )
    .execute(&pressure_state.pool)
    .await
    .expect("make the coverage repair generation initialization due");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_maintenance_busy
        BEFORE INSERT ON hourly_rollup_live_progress
        WHEN NEW.dataset = 'invocation_account_activity_v2_repair_generation'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&pressure_state.pool)
    .await
    .expect("inject a SQLite lock into the coverage repair");

    let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let pressure_audits_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&pressure_state.pool)
    .await
    .expect("count task runs before the locked coverage maintenance pass");

    let pressure_outcome = run_startup_backfill_maintenance_pass_with_gate(
        pressure_state.clone(),
        &cancel,
        Some(&selected_tasks),
        &pressure_gate,
    )
    .await;
    let pressure_audits_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&pressure_state.pool)
    .await
    .expect("count task runs after the locked coverage maintenance pass");
    let retry_progress = load_startup_backfill_progress(&pressure_state.pool, task.name())
        .await
        .expect("load the coverage retry deadline after the lock");

    assert!(!pressure_outcome.ran_actionable_task);
    assert!(!pressure_outcome.had_failure);
    assert_eq!(pressure_audits_after, pressure_audits_before);
    assert_eq!(pressure_gate.snapshot().pressure_events, 1);
    assert!(!retry_progress.is_due(Utc::now()));

    // Closing the pool proves the next task's pressure-cooldown path has zero SQLite I/O.
    pressure_state.pool.close().await;
    let next_tasks = [StartupBackfillTask::PromptCacheKey];
    let next_outcome = run_startup_backfill_maintenance_pass_with_gate(
        pressure_state.clone(),
        &cancel,
        Some(&next_tasks),
        &pressure_gate,
    )
    .await;
    assert!(!next_outcome.ran_actionable_task);
    assert!(!next_outcome.had_failure);
    assert!(pressure_gate.snapshot().background_skips >= 1);

    let non_pressure_state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&non_pressure_state, task).await;
    sqlx::query(
        "DELETE FROM hourly_rollup_live_progress WHERE dataset = 'invocation_account_activity_v2_repair_generation'",
    )
    .execute(&non_pressure_state.pool)
    .await
    .expect("make the non-lock coverage repair generation initialization due");
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_maintenance_error
        BEFORE INSERT ON hourly_rollup_live_progress
        WHEN NEW.dataset = 'invocation_account_activity_v2_repair_generation'
        BEGIN
            SELECT RAISE(ABORT, 'coverage repair operation failed');
        END
        "#,
    )
    .execute(&non_pressure_state.pool)
    .await
    .expect("inject a non-lock coverage repair failure");

    let non_pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let non_pressure_audits_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&non_pressure_state.pool)
    .await
    .expect("count task runs before the non-lock coverage maintenance pass");

    let non_pressure_outcome = run_startup_backfill_maintenance_pass_with_gate(
        non_pressure_state.clone(),
        &cancel,
        Some(&selected_tasks),
        &non_pressure_gate,
    )
    .await;
    non_pressure_state
        .sqlite_batch_writer
        .flush_now(&non_pressure_state.pool)
        .await
        .expect("flush the non-lock coverage failure audit");
    let non_pressure_audits_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&non_pressure_state.pool)
    .await
    .expect("count task runs after the non-lock coverage maintenance pass");
    let non_pressure_audit_status: String = sqlx::query_scalar(
        "SELECT status FROM system_task_runs WHERE task_kind = 'startup_backfill' ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_one(&non_pressure_state.pool)
    .await
    .expect("load the non-lock coverage failure audit");

    assert!(!non_pressure_outcome.ran_actionable_task);
    assert!(non_pressure_outcome.had_failure);
    assert_eq!(non_pressure_audits_after, non_pressure_audits_before + 1);
    assert_eq!(non_pressure_audit_status, "failed");
    assert_eq!(non_pressure_gate.snapshot().pressure_events, 0);
}

#[tokio::test]
async fn coverage_repair_defer_persists_its_own_retry_deadline() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    defer_startup_backfill_coverage_repair(state.as_ref())
        .await
        .expect("schedule the coverage repair retry");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load coverage retry progress");
    let retry_at = progress
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .expect("coverage retry deadline");
    assert!(retry_at > Utc::now());
    assert!(retry_at <= Utc::now() + ChronoDuration::seconds(30));
    assert!(!progress.is_due(Utc::now()));
    assert_eq!(progress.zero_update_streak, 1);
}

#[tokio::test]
async fn coverage_repair_progress_resets_its_own_retry_backoff() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let deferred_until = format_utc_iso(Utc::now() + ChronoDuration::minutes(15));
    save_startup_backfill_progress(
        &state.pool,
        task.name(),
        StartupBackfillProgressUpdate {
            cursor_id: 7,
            scanned: 11,
            updated: 0,
            zero_update_streak: 4,
            next_run_after: &deferred_until,
            status: STARTUP_BACKFILL_STATUS_OK,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed a deferred coverage repair");

    record_startup_backfill_coverage_repair_progress(
        state.as_ref(),
        ActiveAccountActivityV2RepairOutcome {
            priority_bucket_count: 2,
            repaired_bucket_count: 2,
            elapsed_ms: 1,
        },
    )
    .await
    .expect("record coverage repair progress");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load coverage repair progress");
    let retry_at = progress
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .expect("coverage follow-up deadline");
    assert_eq!(progress.zero_update_streak, 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert!(retry_at > Utc::now());
    assert!(retry_at <= Utc::now() + ChronoDuration::seconds(30));
}

#[tokio::test]
async fn coverage_repair_progress_does_not_wake_source_unavailable_historical_backfill() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let historical_task = StartupBackfillTask::HistoricalRollups;
    let coverage_task = StartupBackfillTask::AccountActivityV2Coverage;
    let suspended_until = format_utc_iso(Utc::now() + ChronoDuration::hours(24));
    save_startup_backfill_progress(
        &state.pool,
        historical_task.name(),
        StartupBackfillProgressUpdate {
            cursor_id: 7,
            scanned: 11,
            updated: 0,
            zero_update_streak: 4,
            next_run_after: &suspended_until,
            status: STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE,
            suspension_reason: Some("source_unavailable"),
        },
    )
    .await
    .expect("seed a source-unavailable historical backfill");

    record_startup_backfill_coverage_repair_progress(
        state.as_ref(),
        ActiveAccountActivityV2RepairOutcome {
            priority_bucket_count: 2,
            repaired_bucket_count: 2,
            elapsed_ms: 1,
        },
    )
    .await
    .expect("retain the source-unavailable backfill suspension");

    let historical_progress = load_startup_backfill_progress(&state.pool, historical_task.name())
        .await
        .expect("load source-unavailable historical backfill");
    assert_eq!(historical_progress.zero_update_streak, 4);
    assert_eq!(
        historical_progress.last_status,
        STARTUP_BACKFILL_STATUS_SOURCE_UNAVAILABLE
    );
    assert_eq!(
        historical_progress.next_run_after.as_deref(),
        Some(suspended_until.as_str())
    );

    let coverage_progress = load_startup_backfill_progress(&state.pool, coverage_task.name())
        .await
        .expect("load independent coverage repair progress");
    assert_eq!(coverage_progress.zero_update_streak, 0);
    assert_eq!(coverage_progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert!(!coverage_progress.is_due(Utc::now()));
}

#[test]
fn coverage_repair_retry_backoff_is_bounded_and_exponential() {
    assert_eq!(coverage_repair_retry_delay(1), Duration::from_secs(15));
    assert_eq!(coverage_repair_retry_delay(2), Duration::from_secs(60));
    assert_eq!(coverage_repair_retry_delay(3), Duration::from_secs(5 * 60));
    assert_eq!(coverage_repair_retry_delay(4), Duration::from_secs(15 * 60));
    assert_eq!(
        coverage_repair_retry_delay(20),
        Duration::from_secs(15 * 60)
    );
}

#[tokio::test]
async fn live_activity_v2_coverage_progress_wakes_its_dedicated_repair_task() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    wake_account_activity_v2_coverage_repair(&state.pool, 0)
        .await
        .expect("skip an unchanged live v2 coverage cursor");
    assert_eq!(
        load_startup_backfill_progress(&state.pool, task.name())
            .await
            .expect("load unchanged coverage progress")
            .wake_generation,
        0
    );

    wake_account_activity_v2_coverage_repair(&state.pool, 1)
        .await
        .expect("wake after live v2 coverage cursor progress");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load coverage-woken startup backfill progress");
    assert!(progress.wake_generation > 0);
    assert!(progress.is_due(Utc::now()));
}

#[tokio::test]
async fn live_activity_v2_coverage_wake_preserves_an_active_retry_backoff() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let deferred_until = format_utc_iso(Utc::now() + ChronoDuration::minutes(15));
    save_startup_backfill_progress(
        &state.pool,
        task.name(),
        StartupBackfillProgressUpdate {
            cursor_id: 9,
            scanned: 12,
            updated: 0,
            zero_update_streak: 4,
            next_run_after: &deferred_until,
            status: STARTUP_BACKFILL_STATUS_OK,
            suspension_reason: None,
        },
    )
    .await
    .expect("seed active coverage retry backoff");

    let before = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load active coverage retry before the live update");

    wake_account_activity_v2_coverage_repair(&state.pool, 1)
        .await
        .expect("record live coverage progress without bypassing the retry deadline");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load preserved coverage retry progress");
    assert_eq!(progress.zero_update_streak, 4);
    assert_eq!(
        progress.next_run_after.as_deref(),
        Some(deferred_until.as_str())
    );
    assert!(!progress.is_due(Utc::now()));
    assert_eq!(progress.wake_generation, before.wake_generation);
}

#[tokio::test]
async fn live_activity_v2_coverage_wake_preserves_a_success_follow_up_deadline() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    record_startup_backfill_coverage_repair_progress(
        state.as_ref(),
        ActiveAccountActivityV2RepairOutcome {
            priority_bucket_count: 1,
            repaired_bucket_count: 1,
            elapsed_ms: 1,
        },
    )
    .await
    .expect("record successful coverage repair progress");
    let before = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load scheduled coverage follow-up");

    wake_account_activity_v2_coverage_repair(&state.pool, 1)
        .await
        .expect("keep the successful coverage follow-up deadline");

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load preserved coverage follow-up");
    assert_eq!(progress.zero_update_streak, 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);
    assert_eq!(progress.next_run_after, before.next_run_after);
    assert_eq!(progress.wake_generation, before.wake_generation);
    assert!(!progress.is_due(Utc::now()));
}

#[tokio::test]
async fn idle_coverage_repair_persists_its_next_probe_deadline() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;

    let ran = run_startup_backfill_task_if_due(&state, task)
        .await
        .expect("run an idle coverage repair pass");
    assert!(!ran);

    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("load idle coverage repair progress");
    let next_due = progress
        .next_run_after
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .expect("persisted idle coverage deadline");
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
    assert_eq!(progress.zero_update_streak, 0);
    assert!(next_due > Utc::now() + ChronoDuration::hours(5));
    assert!(next_due <= Utc::now() + ChronoDuration::hours(7));
}

#[tokio::test]
async fn startup_coverage_repair_executes_under_its_single_admitted_gate_permit() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let rollup_guard = state.hourly_rollup_sync_lock.lock().await;

    let run = run_startup_backfill_task_if_due_with_gate(&state, task, &gate);
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => panic!("coverage repair must wait for the held rollup lock, got {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(20)) => {}
    }

    assert!(
        matches!(
            gate.try_begin_background("coverage_admission_probe"),
            Err(crate::db_pressure::DbPressureDenyReason::BackgroundBusy)
        ),
        "the coverage repair must retain its single production-shaped gate permit while it waits"
    );
    drop(rollup_guard);

    assert!(
        !run.await
            .expect("coverage repair should finish after the rollup lock is released")
    );
    let progress = load_startup_backfill_progress(&state.pool, task.name())
        .await
        .expect("coverage repair must persist its idle follow-up after executing");
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_IDLE);
}

#[tokio::test]
async fn startup_coverage_gate_denial_skips_sqlite_progress_and_task_run_audit() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let _held = gate
        .try_begin_background("coverage_gate_holder")
        .expect("occupy the production-shaped gate");
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before the denied coverage repair");

    assert!(
        !run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
            .await
            .expect("closed coverage gate should defer without an error")
    );

    let progress_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM startup_backfill_progress WHERE task_name = ?1")
            .bind(task.name())
            .fetch_one(&state.pool)
            .await
            .expect("count coverage progress after the denied repair");
    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after the denied coverage repair");
    assert_eq!(
        progress_rows, 0,
        "gate denial must not write coverage progress"
    );
    assert_eq!(
        task_runs_after, task_runs_before,
        "gate denial must not write an audit"
    );

    state.pool.close().await;
    assert!(
        !run_startup_backfill_task_if_due_with_gate(&state, task, &gate)
            .await
            .expect("a closed pool proves denied coverage never accesses SQLite")
    );
}

#[tokio::test]
async fn startup_coverage_repair_progress_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_progress_busy
        BEFORE INSERT ON startup_backfill_progress
        WHEN NEW.task_name = 'account_activity_v2_coverage_repair_v1'
            AND NEW.last_status = 'ok'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into coverage repair progress persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task(
        &state,
        &gate,
        || async {
            Ok(ActiveAccountActivityV2RepairOutcome {
                priority_bucket_count: 1,
                repaired_bucket_count: 1,
                elapsed_ms: 1,
            })
        },
    )
    .await;
}

#[tokio::test]
async fn startup_coverage_repair_defer_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::AccountActivityV2Coverage)
        .await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_defer_busy
        BEFORE UPDATE OF next_run_after ON startup_backfill_progress
        WHEN NEW.task_name = 'account_activity_v2_coverage_repair_v1'
            AND NEW.last_status = 'idle'
        BEGIN
            SELECT RAISE(ABORT, 'database is busy');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into coverage repair defer persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task(
        &state,
        &gate,
        || async {
            Ok(ActiveAccountActivityV2RepairOutcome {
                priority_bucket_count: 1,
                repaired_bucket_count: 0,
                elapsed_ms: 1,
            })
        },
    )
    .await;
}

#[tokio::test]
async fn startup_coverage_repair_and_retry_progress_persist_busy_record_one_pressure_event() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    seed_due_startup_backfill_progress(&state, task).await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_retry_progress_persist_busy
        BEFORE UPDATE OF next_run_after ON startup_backfill_progress
        WHEN NEW.task_name = 'account_activity_v2_coverage_repair_v1'
            AND NEW.last_status = 'idle'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into retry progress persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before combined repair and retry-progress failures");

    let outcome =
        run_startup_backfill_coverage_repair_if_due_with_repair(&state, &gate, || async {
            Err(anyhow::anyhow!("database is busy"))
        })
        .await
        .expect("repair and retry progress locks should defer coverage repair");
    assert!(outcome.is_pressure_deferred());
    assert_eq!(
        gate.snapshot().pressure_events,
        1,
        "one failed coverage attempt must record one pressure event even when retry progress also locks"
    );

    let next = run_startup_backfill_task_if_due_with_gate(
        &state,
        StartupBackfillTask::PromptCacheKey,
        &gate,
    )
    .await
    .expect("the next task should be deferred before SQLite access");
    assert!(!next);
    assert!(
        gate.snapshot().background_skips >= 1,
        "the closed gate must prevent an early next task"
    );
    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after combined repair and retry-progress failures");
    assert_eq!(
        task_runs_after, task_runs_before,
        "direct coverage repair failure must not create a task-run audit"
    );
}

#[tokio::test]
async fn startup_coverage_repair_and_retry_progress_lookup_busy_record_one_pressure_event() {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-coverage-repair-retry-progress-lookup-busy",
        Duration::from_millis(50),
    )
    .await;
    let task = StartupBackfillTask::AccountActivityV2Coverage;
    seed_due_startup_backfill_progress(&state, task).await;
    let lock_holder = Arc::new(Mutex::new(None));
    let lock_holder_for_repair = lock_holder.clone();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    let task_runs_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs before combined repair and retry-progress failures");

    let outcome = run_startup_backfill_coverage_repair_if_due_with_repair(&state, &gate, move || {
        let db_url = db_url.clone();
        let lock_holder = lock_holder_for_repair.clone();
        async move {
            let mut lock_conn = SqliteConnection::connect(&db_url).await?;
            sqlx::query("BEGIN EXCLUSIVE")
                .execute(&mut lock_conn)
                .await?;
            sqlx::query(
                "CREATE TABLE startup_coverage_repair_retry_progress_lookup_lock_guard (id INTEGER)",
            )
            .execute(&mut lock_conn)
            .await?;
            *lock_holder.lock().await = Some(lock_conn);
            Err(anyhow::anyhow!("database is busy"))
        }
    })
    .await
    .expect("repair and retry progress locks should defer coverage repair");
    assert!(outcome.is_pressure_deferred());
    assert_eq!(
        gate.snapshot().pressure_events,
        1,
        "one failed coverage attempt must record one pressure event even when retry progress also locks"
    );

    let next = run_startup_backfill_task_if_due_with_gate(
        &state,
        StartupBackfillTask::PromptCacheKey,
        &gate,
    )
    .await
    .expect("the next task should be deferred before SQLite access");
    assert!(!next);
    assert!(
        gate.snapshot().background_skips >= 1,
        "the closed gate must prevent an early next task"
    );

    let mut lock_conn = lock_holder
        .lock()
        .await
        .take()
        .expect("keep the exclusive lock through retry progress lookup");
    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release retry progress lookup lock");
    lock_conn
        .close()
        .await
        .expect("close retry progress lookup lock holder");

    let task_runs_after: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM system_task_runs WHERE task_kind = 'startup_backfill'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count task runs after combined repair and retry-progress failures");
    assert_eq!(
        task_runs_after, task_runs_before,
        "direct coverage repair failure must not create a task-run audit"
    );
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_coverage_repair_idle_progress_lookup_busy_closes_gate_before_permit_release() {
    let (state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-coverage-repair-idle-progress-lookup-busy",
        Duration::from_millis(50),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::AccountActivityV2Coverage)
        .await;
    let lock_holder = Arc::new(Mutex::new(None));
    let lock_holder_for_repair = lock_holder.clone();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));

    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task(
        &state,
        &gate,
        move || {
            let db_url = db_url.clone();
            let lock_holder = lock_holder_for_repair.clone();
            async move {
                let mut lock_conn = SqliteConnection::connect(&db_url).await?;
                sqlx::query("BEGIN EXCLUSIVE")
                    .execute(&mut lock_conn)
                    .await?;
                sqlx::query(
                    "CREATE TABLE startup_coverage_repair_idle_progress_lookup_lock_guard (id INTEGER)",
                )
                .execute(&mut lock_conn)
                .await?;
                *lock_holder.lock().await = Some(lock_conn);
                Ok(ActiveAccountActivityV2RepairOutcome::default())
            }
        },
    )
    .await;

    let mut lock_conn = lock_holder
        .lock()
        .await
        .take()
        .expect("keep the exclusive lock through idle progress lookup");
    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("release idle progress lookup lock");
    lock_conn
        .close()
        .await
        .expect("close idle progress lookup lock holder");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_coverage_repair_idle_progress_persist_busy_closes_gate_before_permit_release() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;
    seed_due_startup_backfill_progress(&state, StartupBackfillTask::AccountActivityV2Coverage)
        .await;
    sqlx::query(
        r#"
        CREATE TRIGGER startup_coverage_repair_idle_progress_persist_busy
        BEFORE UPDATE OF last_status ON startup_backfill_progress
        WHEN NEW.task_name = 'account_activity_v2_coverage_repair_v1'
            AND NEW.last_status = 'idle'
        BEGIN
            SELECT RAISE(ABORT, 'database table is locked');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("inject a SQLite lock into idle coverage progress persistence");

    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(60));
    assert_startup_coverage_repair_busy_error_closes_gate_before_next_task(
        &state,
        &gate,
        || async { Ok(ActiveAccountActivityV2RepairOutcome::default()) },
    )
    .await;
}

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
        Some("{\"endpoint\":\"/v1/responses\",\"requestCompressionAlgorithm\":\"zstd\"}"),
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
    assert!(
        archive_columns.contains("raw_expires_at"),
        "historical archive files should keep their legacy schema"
    );
    assert!(
        archive_columns.contains("first_token_ms"),
        "append should upgrade legacy archives with nullable TTFT storage"
    );
    let archived_first_token_ms: Option<f64> =
        sqlx::query_scalar("SELECT first_token_ms FROM codex_invocations WHERE invoke_id = ?1")
            .bind("archive-into-legacy-batch")
            .fetch_one(&archived_pool)
            .await
            .expect("load archived TTFT");
    assert_eq!(archived_first_token_ms, None);
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
