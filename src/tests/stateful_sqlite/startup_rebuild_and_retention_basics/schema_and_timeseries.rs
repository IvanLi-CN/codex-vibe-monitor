use super::super::*;
use serde_json::json;
use std::future::Future;

pub(in crate::tests::stateful_sqlite) async fn assert_startup_backfill_busy_error_closes_gate_before_next_task(
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

pub(in crate::tests::stateful_sqlite) async fn assert_startup_coverage_repair_busy_error_closes_gate_before_next_task<
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

pub(in crate::tests::stateful_sqlite) async fn seed_due_startup_backfill_progress(
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
        // A pre-live source fixture must also omit dependent working-set triggers.
        // Current definitions read live IDs and cannot run after this table is dropped.
        "trg_codex_invocations_prompt_cache_working_set_insert",
        "trg_codex_invocations_prompt_cache_working_set_update",
        "trg_codex_invocations_prompt_cache_working_set_delete",
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
async fn old_terminal_invocation_delete_skips_live_working_set_rebuild() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    for (invoke_id, seconds_ago) in [
        ("working-window-old", 720_i64),
        ("working-window-recent", 30_i64),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, 'success', 1, 0.01, ?4, '{}')
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            (now - ChronoDuration::seconds(seconds_ago))
                .with_timezone(&Shanghai)
                .naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(json!({ "promptCacheKey": "working-window-key" }).to_string())
        .execute(&state.pool)
        .await
        .expect("insert working-set trigger source row");
    }

    let before: (i64, i64) = sqlx::query_as(
        "SELECT request_count, total_tokens FROM prompt_cache_working_set_live WHERE prompt_cache_key = ?1",
    )
    .bind("working-window-key")
    .fetch_one(&state.pool)
    .await
    .expect("recent source row should populate working-set live row");
    assert_eq!(before, (1, 1));

    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id = ?1")
        .bind("working-window-old")
        .execute(&state.pool)
        .await
        .expect("delete old source row");

    let after: (i64, i64) = sqlx::query_as(
        "SELECT request_count, total_tokens FROM prompt_cache_working_set_live WHERE prompt_cache_key = ?1",
    )
    .bind("working-window-key")
    .fetch_one(&state.pool)
    .await
    .expect("working-set live row should remain after old delete");
    assert_eq!(after, before);
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
