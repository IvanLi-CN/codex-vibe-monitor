pub(crate) async fn assert_startup_backfill_busy_error_closes_gate_before_next_task(
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

pub(crate) async fn assert_startup_coverage_repair_busy_error_closes_gate_before_next_task<
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

pub(crate) async fn seed_due_startup_backfill_progress(
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

async fn seed_legacy_invocation_schema(pool: &SqlitePool) {
    sqlx::query(
        "CREATE TABLE codex_invocations ( \
         id INTEGER PRIMARY KEY AUTOINCREMENT, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, \
         source TEXT NOT NULL DEFAULT 'xy', payload TEXT, raw_response TEXT NOT NULL, \
         raw_expires_at TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')), \
         UNIQUE(invoke_id, occurred_at))",
    )
    .execute(pool)
    .await
    .expect("create legacy invocation schema");
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, payload, raw_response, raw_expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind("legacy-row")
    .bind("2026-03-09 10:00:00")
    .bind(SOURCE_PROXY)
    .bind("{\"endpoint\":\"/v1/responses\"}")
    .bind("{\"ok\":true}")
    .bind("2099-01-01 00:00:00")
    .execute(pool)
    .await
    .expect("insert legacy invocation row");
}

async fn assert_migrated_invocation_schema(pool: &SqlitePool) {
    let columns: HashSet<String> = sqlx::query("PRAGMA table_info('codex_invocations')")
        .fetch_all(pool)
        .await
        .expect("inspect invocation columns")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(!columns.contains("raw_expires_at"));
    for column in ["detail_level", "detail_pruned_at", "detail_prune_reason"] {
        assert!(columns.contains(column));
    }
    let row = sqlx::query(
        "SELECT invoke_id, source, payload, raw_response, detail_level \
         FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("legacy-row")
    .fetch_one(pool)
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
}

async fn assert_retention_tables(pool: &SqlitePool) {
    let tables: HashSet<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name IN ( \
         'archive_batches', 'invocation_rollup_daily', 'startup_backfill_progress')",
    )
    .fetch_all(pool)
    .await
    .expect("load retention tables")
    .into_iter()
    .collect();
    for table in [
        "archive_batches",
        "invocation_rollup_daily",
        "startup_backfill_progress",
    ] {
        assert!(tables.contains(table));
    }
}

async fn assert_upstream_account_index(pool: &SqlitePool) {
    let sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND \
         name = 'idx_codex_invocations_upstream_account_occurred_at'",
    )
    .fetch_one(pool)
    .await
    .expect("load upstream account invocation index");
    assert!(sql.contains("$.upstreamAccountId"));
    assert!(sql.contains("occurred_at"));
}

async fn assert_backfill_indexes_and_plans(pool: &SqlitePool) {
    let usage_index: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND \
         name = 'idx_codex_invocations_proxy_usage_backfill_pending'",
    )
    .fetch_one(pool)
    .await
    .expect("load proxy usage backfill index");
    for fragment in ["source", "status", "id", "total_tokens IS NULL"] {
        assert!(usage_index.contains(fragment));
    }
    let attempt_index: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND \
         name = 'idx_pool_upstream_request_attempts_pending_early_phase_started'",
    )
    .fetch_one(pool)
    .await
    .expect("load stale attempt recovery index");
    for fragment in ["status", "started_at", "invoke_id", "finished_at IS NULL"] {
        assert!(attempt_index.contains(fragment));
    }
    assert!(attempt_index.contains("LOWER(TRIM(COALESCE(phase, '')))"));
    let usage_plan = explain_proxy_usage_backfill(pool).await;
    assert!(usage_plan.contains("idx_codex_invocations_proxy_usage_backfill_pending"));
}

async fn explain_proxy_usage_backfill(pool: &SqlitePool) -> String {
    sqlx::query(
        "EXPLAIN QUERY PLAN SELECT COALESCE(MAX(id), 0) FROM codex_invocations \
         WHERE source = 'proxy' AND status = 'success' AND total_tokens IS NULL \
         AND response_raw_path IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .expect("load proxy usage backfill explain plan")
    .into_iter()
    .map(|row| row.get::<String, _>("detail"))
    .collect::<Vec<_>>()
    .join(" | ")
}

async fn assert_rebuild_and_transport_indexes(pool: &SqlitePool) {
    let stale_plan = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT id, invoke_id, occurred_at, sticky_key, upstream_account_id \
         FROM pool_upstream_request_attempts WHERE status = 'pending' AND finished_at IS NULL \
         AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte') \
         AND COALESCE(first_byte_latency_ms, 0) <= 0 AND (started_at IS NULL \
         OR (endpoint = '/v1/responses' AND started_at <= '2026-03-09 10:00:00') \
         OR (endpoint = '/v1/responses/compact' AND started_at <= '2026-03-09 10:00:00') \
         OR (COALESCE(endpoint, '') NOT IN ('/v1/responses', '/v1/responses/compact') \
             AND started_at <= '2026-03-09 10:00:00'))",
    )
    .fetch_all(pool)
    .await
    .expect("load stale attempt explain plan")
    .into_iter()
    .map(|row| row.get::<String, _>("detail"))
    .collect::<Vec<_>>()
    .join(" | ");
    assert!(stale_plan.contains("idx_pool_upstream_request_attempts_pending_early_phase_started"));
    let transport_index: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND \
         name = 'idx_pool_upstream_request_attempts_transport_decode_recent'",
    )
    .fetch_one(pool)
    .await
    .expect("load transport decode recent index");
    for fragment in [
        "upstream_account_id",
        "route_mode",
        "endpoint",
        "phase",
        "occurred_at DESC",
    ] {
        assert!(transport_index.contains(fragment));
    }
    let transport_plan = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT failure_kind FROM pool_upstream_request_attempts \
         WHERE upstream_account_id = 42 AND route_mode = 'pool' AND endpoint = '/v1/responses' \
         AND phase IN ('completed', 'failed') ORDER BY occurred_at DESC, id DESC LIMIT 2",
    )
    .fetch_all(pool)
    .await
    .expect("load transport decode explain plan")
    .into_iter()
    .map(|row| row.get::<String, _>("detail"))
    .collect::<Vec<_>>()
    .join(" | ");
    assert!(transport_plan.contains("idx_pool_upstream_request_attempts_transport_decode_recent"));
}

#[tokio::test]
pub(crate) async fn hourly_timeseries_omits_pre_cutoff_partial_hour_rollups() {
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
        HourlyRollupFixture::without_latency(bucket_start, SOURCE_PROXY, 1, 0, 0, 5, 0.05),
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
pub(crate) async fn forward_proxy_timeseries_rejects_non_hour_aligned_timezones() {
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
pub(crate) async fn timeseries_subday_bucket_stays_available_inside_live_window() {
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
pub(crate) async fn invocation_hourly_rollup_range_respects_proxy_only_scope() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let stats_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(20);
    insert_invocation_rollup(
        &state.pool,
        InvocationRollupFixture {
            stats_date,
            source: SOURCE_PROXY,
            total_count: 2,
            success_count: 1,
            failure_count: 1,
            total_tokens: 20,
            total_cost: 0.2,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        },
    )
    .await;
    insert_invocation_rollup(
        &state.pool,
        InvocationRollupFixture {
            stats_date,
            source: SOURCE_XY,
            total_count: 5,
            success_count: 5,
            failure_count: 0,
            total_tokens: 50,
            total_cost: 0.5,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        },
    )
    .await;

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
pub(crate) async fn ensure_schema_migrates_codex_invocations_off_raw_expires_at_and_adds_retention_tables()
 {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    seed_legacy_invocation_schema(&pool).await;

    ensure_schema(&pool).await.expect("ensure schema migration");
    assert_migrated_invocation_schema(&pool).await;
    assert_retention_tables(&pool).await;
    assert_upstream_account_index(&pool).await;
    assert_backfill_indexes_and_plans(&pool).await;
    assert_rebuild_and_transport_indexes(&pool).await;
}

type InvocationInProgressLiveRow = (
    String,
    String,
    Option<i64>,
    Option<String>,
    i64,
    i64,
    i64,
    i64,
    Option<f64>,
);

struct InvocationInProgressSeed {
    invoke_id: &'static str,
    source: &'static str,
    status: &'static str,
    prompt_cache_key: &'static str,
    upstream_account_id: Option<i64>,
    ttfb_ms: Option<f64>,
    error_message: Option<&'static str>,
    failure_kind: Option<&'static str>,
}

fn invocation_in_progress_seeds() -> [InvocationInProgressSeed; 5] {
    [
        InvocationInProgressSeed {
            invoke_id: "rebuild-live-failed",
            source: SOURCE_PROXY,
            status: "failed",
            prompt_cache_key: "rebuild-live-key-a",
            upstream_account_id: Some(42),
            ttfb_ms: Some(810.0),
            error_message: Some("upstream response failed"),
            failure_kind: Some("upstream_response_failed"),
        },
        InvocationInProgressSeed {
            invoke_id: "rebuild-live-pending",
            source: SOURCE_PROXY,
            status: "pending",
            prompt_cache_key: "rebuild-live-key-a",
            upstream_account_id: Some(42),
            ttfb_ms: None,
            error_message: None,
            failure_kind: None,
        },
        InvocationInProgressSeed {
            invoke_id: "rebuild-live-interrupted",
            source: SOURCE_PROXY,
            status: "interrupted",
            prompt_cache_key: "rebuild-live-key-b",
            upstream_account_id: Some(7),
            ttfb_ms: Some(620.0),
            error_message: Some("downstream closed while streaming upstream response"),
            failure_kind: Some("downstream_closed"),
        },
        InvocationInProgressSeed {
            invoke_id: "rebuild-live-running",
            source: SOURCE_XY,
            status: "running",
            prompt_cache_key: "rebuild-live-key-b",
            upstream_account_id: Some(7),
            ttfb_ms: Some(1700.0),
            error_message: None,
            failure_kind: None,
        },
        InvocationInProgressSeed {
            invoke_id: "rebuild-live-unscoped",
            source: SOURCE_PROXY,
            status: "running",
            prompt_cache_key: "rebuild-live-key-c",
            upstream_account_id: None,
            ttfb_ms: Some(1100.0),
            error_message: None,
            failure_kind: None,
        },
    ]
}

async fn drop_invocation_in_progress_live_schema(pool: &SqlitePool) {
    for trigger_name in [
        "trg_codex_invocations_live_insert",
        "trg_codex_invocations_live_update",
        "trg_codex_invocations_live_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(pool)
            .await
            .expect("drop live trigger before rebuild test");
    }
    sqlx::query("DROP TABLE IF EXISTS invocation_in_progress_live")
        .execute(pool)
        .await
        .expect("drop live table before rebuild test");
}

async fn insert_invocation_in_progress_seed(
    pool: &SqlitePool,
    occurred_at: &str,
    seed: InvocationInProgressSeed,
) {
    let mut payload = json!({ "promptCacheKey": seed.prompt_cache_key });
    if let Some(account_id) = seed.upstream_account_id {
        payload["upstreamAccountId"] = json!(account_id);
    }
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, error_message, failure_kind,
            t_upstream_ttfb_ms, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(seed.invoke_id)
    .bind(occurred_at)
    .bind(seed.source)
    .bind(seed.status)
    .bind(seed.error_message)
    .bind(seed.failure_kind)
    .bind(seed.ttfb_ms)
    .bind(payload.to_string())
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert rebuild source invocation row");
}

async fn seed_invocation_in_progress_sources(pool: &SqlitePool, occurred_at: &str) {
    for seed in invocation_in_progress_seeds() {
        insert_invocation_in_progress_seed(pool, occurred_at, seed).await;
    }
}

async fn load_rebuilt_invocation_in_progress_rows(
    pool: &SqlitePool,
) -> Vec<InvocationInProgressLiveRow> {
    sqlx::query_as::<_, InvocationInProgressLiveRow>(
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
    .fetch_all(pool)
    .await
    .expect("load rebuilt live rows")
}

fn rebuilt_invocation_row<'a>(
    rows: &'a [InvocationInProgressLiveRow],
    invoke_id: &str,
) -> &'a InvocationInProgressLiveRow {
    rows.iter()
        .find(|row| row.0 == invoke_id)
        .unwrap_or_else(|| panic!("{invoke_id} should be rebuilt"))
}

fn assert_rebuilt_invocation_in_progress_rows(rows: &[InvocationInProgressLiveRow]) {
    assert_eq!(
        rows.len(),
        3,
        "only running/pending rows should survive rebuild"
    );
    assert_rebuilt_pending_row(rebuilt_invocation_row(rows, "rebuild-live-pending"));
    assert_rebuilt_running_row(rebuilt_invocation_row(rows, "rebuild-live-running"));
    assert_rebuilt_unscoped_row(rebuilt_invocation_row(rows, "rebuild-live-unscoped"));
}

fn assert_rebuilt_pending_row(row: &InvocationInProgressLiveRow) {
    assert_eq!(row.1, SOURCE_PROXY);
    assert_eq!(row.2, Some(42));
    assert_eq!(row.3.as_deref(), Some("rebuild-live-key-a"));
    assert_eq!((row.4, row.5, row.6, row.7), (1, 1, 1, 1));
    assert_eq!(row.8, None);
}

fn assert_rebuilt_running_row(row: &InvocationInProgressLiveRow) {
    assert_eq!(row.1, SOURCE_XY);
    assert_eq!(row.2, Some(7));
    assert_eq!(row.3.as_deref(), Some("rebuild-live-key-b"));
    assert_eq!((row.4, row.5, row.6, row.7), (0, 0, 0, 0));
    assert_eq!(row.8, Some(1700.0));
}

fn assert_rebuilt_unscoped_row(row: &InvocationInProgressLiveRow) {
    assert_eq!(row.1, SOURCE_PROXY);
    assert_eq!(row.2, None);
    assert_eq!(row.3.as_deref(), Some("rebuild-live-key-c"));
    assert_eq!((row.4, row.5, row.6, row.7), (0, 0, 0, 0));
    assert_eq!(row.8, Some(1100.0));
}

#[tokio::test]
pub(crate) async fn ensure_schema_rebuilds_invocation_in_progress_live_from_existing_invocations() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    drop_invocation_in_progress_live_schema(&state.pool).await;
    seed_invocation_in_progress_sources(&state.pool, &occurred_at).await;

    ensure_schema(&state.pool)
        .await
        .expect("rebuild invocation_in_progress_live on ensure_schema");
    let rows = load_rebuilt_invocation_in_progress_rows(&state.pool).await;
    assert_rebuilt_invocation_in_progress_rows(&rows);
}

#[tokio::test]
pub(crate) async fn ensure_schema_serializes_live_trigger_rebuild_under_concurrent_reentry() {
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

type PromptCacheWorkingSetLiveRow = (
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
);

struct PromptCacheWorkingSetSeed {
    invoke_id: &'static str,
    source: &'static str,
    status: &'static str,
    prompt_cache_key: &'static str,
    seconds_ago: i64,
    total_tokens: i64,
    cost: f64,
}

fn prompt_cache_working_set_seeds() -> [PromptCacheWorkingSetSeed; 4] {
    [
        PromptCacheWorkingSetSeed {
            invoke_id: "working-live-recent-success",
            source: SOURCE_PROXY,
            status: "success",
            prompt_cache_key: "working-live-key-a",
            seconds_ago: 30,
            total_tokens: 120,
            cost: 0.12,
        },
        PromptCacheWorkingSetSeed {
            invoke_id: "working-live-recent-running",
            source: SOURCE_PROXY,
            status: "running",
            prompt_cache_key: "working-live-key-a",
            seconds_ago: 15,
            total_tokens: 140,
            cost: 0.14,
        },
        PromptCacheWorkingSetSeed {
            invoke_id: "working-live-recent-cross-source",
            source: SOURCE_XY,
            status: "success",
            prompt_cache_key: "working-live-key-b",
            seconds_ago: 40,
            total_tokens: 220,
            cost: 0.22,
        },
        PromptCacheWorkingSetSeed {
            invoke_id: "working-live-old-terminal",
            source: SOURCE_PROXY,
            status: "success",
            prompt_cache_key: "working-live-key-old",
            seconds_ago: 720,
            total_tokens: 320,
            cost: 0.32,
        },
    ]
}

async fn drop_prompt_cache_working_set_schema(pool: &SqlitePool) {
    for trigger_name in [
        "trg_codex_invocations_prompt_cache_working_set_insert",
        "trg_codex_invocations_prompt_cache_working_set_update",
        "trg_codex_invocations_prompt_cache_working_set_delete",
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(pool)
            .await
            .expect("drop prompt cache working-set trigger before rebuild test");
    }
    sqlx::query("DROP TABLE IF EXISTS prompt_cache_working_set_live")
        .execute(pool)
        .await
        .expect("drop prompt cache working-set table before rebuild test");
}

async fn insert_prompt_cache_working_set_seed(
    pool: &SqlitePool,
    now: chrono::DateTime<chrono::Utc>,
    seed: PromptCacheWorkingSetSeed,
) {
    let occurred_at = format_naive(
        (now - ChronoDuration::seconds(seed.seconds_ago))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(seed.invoke_id)
    .bind(occurred_at)
    .bind(seed.source)
    .bind(seed.status)
    .bind(seed.total_tokens)
    .bind(seed.cost)
    .bind(json!({ "promptCacheKey": seed.prompt_cache_key }).to_string())
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert prompt cache working-set rebuild source row");
}

async fn seed_prompt_cache_working_set_sources(
    pool: &SqlitePool,
    now: chrono::DateTime<chrono::Utc>,
) {
    for seed in prompt_cache_working_set_seeds() {
        insert_prompt_cache_working_set_seed(pool, now, seed).await;
    }
}

async fn load_rebuilt_prompt_cache_working_set_rows(
    pool: &SqlitePool,
) -> Vec<PromptCacheWorkingSetLiveRow> {
    sqlx::query_as::<_, PromptCacheWorkingSetLiveRow>(
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
    .fetch_all(pool)
    .await
    .expect("load rebuilt working-set rows")
}

fn rebuilt_prompt_cache_row<'a>(
    rows: &'a [PromptCacheWorkingSetLiveRow],
    prompt_cache_key: &str,
) -> &'a PromptCacheWorkingSetLiveRow {
    rows.iter()
        .find(|row| row.0 == prompt_cache_key)
        .unwrap_or_else(|| panic!("{prompt_cache_key} should survive rebuild"))
}

fn assert_rebuilt_prompt_cache_working_set_rows(rows: &[PromptCacheWorkingSetLiveRow]) {
    assert_eq!(
        rows.len(),
        2,
        "only recent or in-flight keys should survive rebuild"
    );
    assert_rebuilt_prompt_cache_key_a(rebuilt_prompt_cache_row(rows, "working-live-key-a"));
    assert_rebuilt_prompt_cache_key_b(rebuilt_prompt_cache_row(rows, "working-live-key-b"));
}

fn assert_rebuilt_prompt_cache_key_a(row: &PromptCacheWorkingSetLiveRow) {
    assert_eq!((row.1, row.2, row.7, row.8), (1, 1, 2, 260));
    assert!((row.9 - 0.26).abs() < 1e-9);
    assert!(row.5.is_some());
    assert!(row.6.is_some());
}

fn assert_rebuilt_prompt_cache_key_b(row: &PromptCacheWorkingSetLiveRow) {
    assert_eq!((row.1, row.2, row.7, row.8), (1, 0, 1, 220));
    assert!((row.9 - 0.22).abs() < 1e-9);
    assert!(row.5.is_some());
    assert!(row.6.is_none());
}

#[tokio::test]
pub(crate) async fn ensure_schema_rebuilds_prompt_cache_working_set_live_from_existing_invocations()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    drop_prompt_cache_working_set_schema(&state.pool).await;
    seed_prompt_cache_working_set_sources(&state.pool, now).await;

    ensure_schema(&state.pool)
        .await
        .expect("rebuild prompt_cache_working_set_live on ensure_schema");
    let rows = load_rebuilt_prompt_cache_working_set_rows(&state.pool).await;
    assert_rebuilt_prompt_cache_working_set_rows(&rows);
}

use super::*;
