use super::*;

#[test]
fn metrics_keep_call_count_separate_from_success_only_timing_samples() {
    let success = LongTermInvocationRow {
        id: 1,
        invoke_id: None,
        occurred_at: "2026-07-26T00:00:00Z".to_string(),
        status: Some("success".to_string()),
        model: Some("legacy-model".to_string()),
        request_model: Some("request-model".to_string()),
        response_model: Some("response-model".to_string()),
        reasoning_effort: Some("high".to_string()),
        upstream_account_id: None,
        upstream_account_kind: None,
        upstream_account_name: None,
        total_tokens: Some(100),
        output_tokens: Some(50),
        cost: Some(1.5),
        t_total_ms: Some(900.0),
        t_req_read_ms: Some(100.0),
        t_req_parse_ms: Some(50.0),
        t_upstream_connect_ms: Some(100.0),
        t_upstream_ttfb_ms: Some(300.0),
        t_upstream_stream_ms: Some(600.0),
        error_message: None,
    };
    let failure = LongTermInvocationRow {
        id: 2,
        invoke_id: None,
        occurred_at: "2026-07-26T00:00:01Z".to_string(),
        status: Some("failed".to_string()),
        model: None,
        request_model: None,
        response_model: None,
        reasoning_effort: None,
        upstream_account_id: None,
        upstream_account_kind: None,
        upstream_account_name: None,
        total_tokens: None,
        output_tokens: Some(10),
        cost: None,
        t_total_ms: Some(100.0),
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: Some(20.0),
        t_upstream_stream_ms: Some(80.0),
        error_message: None,
    };
    let mut accumulator = LongTermAccumulator::default();
    accumulator.add_call(&success, Some((0, 900)));
    accumulator.add_call(&failure, None);
    let metrics = LongTermMetrics::from_accumulator(&accumulator);
    assert_eq!(metrics.calls, 2);
    assert_eq!(metrics.tokens, Some(100));
    assert_eq!(metrics.cost, Some(1.5));
    assert_eq!(metrics.usage_time_ms, Some(900.0));
    assert_eq!(metrics.output_speed_tokens_per_second, Some(50.0 / 0.6));
    assert_eq!(metrics.first_byte_ms, Some(550.0));
    assert_eq!(metrics.response_ms, Some(600.0));
    assert_eq!(normalize_long_term_model(&success), "response-model");
    assert_eq!(normalize_long_term_model(&failure), "未知模型");
    let mut blank_legacy = success.clone();
    blank_legacy.response_model = None;
    blank_legacy.model = Some("  \t".to_string());
    assert_eq!(normalize_long_term_model(&blank_legacy), "request-model");
}

#[test]
fn non_api_key_upstreams_share_the_other_series() {
    let row = LongTermInvocationRow {
        id: 1,
        invoke_id: None,
        occurred_at: "2026-07-26T00:00:00Z".to_string(),
        status: Some("success".to_string()),
        model: None,
        request_model: Some("gpt-5".to_string()),
        response_model: None,
        reasoning_effort: None,
        upstream_account_id: Some(7),
        upstream_account_kind: Some("oauth_codex".to_string()),
        upstream_account_name: Some("OAuth".to_string()),
        total_tokens: Some(1),
        output_tokens: Some(1),
        cost: Some(0.1),
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        t_upstream_stream_ms: None,
        error_message: None,
    };
    assert_eq!(
        normalize_long_term_upstream(&row),
        ("other".to_string(), "其他".to_string())
    );
}

#[tokio::test]
async fn refresh_replaces_stale_rows_when_the_complete_live_source_is_empty() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(1);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    create_long_term_integrity_oracle(&pool).await;
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms) VALUES (1, 'test-invoke-1', ?1, 'success', 'gpt-5', '{\"reasoningEffort\":\"high\"}', 8, 4, 0, 0, 12, 0.2, 100, 10, 5, 5, 20, 80)",
        )
        .bind(format!("{date}T12:00:00+08:00"))
        .execute(&pool)
        .await
        .expect("invocation row");
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, 'canonical', 1, 1, 12, 0.2, 12, 0.2)",
        )
        .bind(day_start + 12 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical hourly proof");
    refresh_long_term_stats(&pool, 400).await.expect("refresh");
    let daily_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_daily WHERE dimension = 'overall'",
    )
    .fetch_one(&pool)
    .await
    .expect("daily count");
    assert_eq!(daily_rows, 1);
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("state after initial refresh");
    assert_eq!(status, LONG_TERM_STATUS_READY);
    sqlx::query("DELETE FROM codex_invocations")
        .execute(&pool)
        .await
        .expect("remove live source row");
    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh after source removal");
    let remaining_daily_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_daily WHERE dimension = 'overall'",
    )
    .fetch_one(&pool)
    .await
    .expect("remaining daily count");
    assert_eq!(remaining_daily_rows, 0);
    let remaining_hourly_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_hourly WHERE dimension = 'overall'",
    )
    .fetch_one(&pool)
    .await
    .expect("remaining hourly count");
    assert_eq!(remaining_hourly_rows, 0);
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("state");
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_does_not_overwrite_complete_rollups_with_partial_rebuilds() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query(
        r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT,
                occurred_at TEXT NOT NULL,
                status TEXT,
                model TEXT,
                payload TEXT,
                total_tokens INTEGER,
                output_tokens INTEGER,
                cost REAL,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                t_upstream_stream_ms REAL,
                error_message TEXT
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect("invocation schema");
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let occurred_at = format!("{today}T12:00:00+08:00");
    let day_start_epoch = today
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .expect("Shanghai start")
        .timestamp();
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, total_tokens, output_tokens, cost) VALUES (1, 'partial', ?1, 'success', 'gpt-5', '{}', 7, 2, 0.07)",
        )
        .bind(occurred_at)
        .execute(&pool)
        .await
        .expect("partial source row");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 10, 100, 10, 1.0, 10)",
        )
        .bind(today.to_string())
        .execute(&pool)
        .await
        .expect("complete daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 10, 100, 10, 1.0, 10)",
        )
        .bind(day_start_epoch + 12 * 60 * 60)
        .execute(&pool)
        .await
        .expect("complete hourly rollup");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("ready state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh protects complete rollups");

    let daily_calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(today.to_string())
    .fetch_one(&pool)
    .await
    .expect("retained daily calls");
    let hourly_calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_hourly WHERE bucket_start_epoch = ?1 AND dimension = 'overall'",
        )
        .bind(day_start_epoch + 12 * 60 * 60)
        .fetch_one(&pool)
        .await
        .expect("retained hourly calls");
    assert_eq!(daily_calls, 10);
    assert_eq!(hourly_calls, 10);
}

#[tokio::test]
async fn refresh_rebuilds_completed_rollups_after_complete_source_reconciliation() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(1);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    let hour_start = day_start + 10 * 60 * 60;
    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'legacy', 2, 2, 200, 0.2, 0, 200, 0.2)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("untrusted canonical rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("durable daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("durable hourly rollup");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("ready state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh after complete source reconciliation");

    let daily = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("rebuilt daily rollup");
    let hourly = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_hourly WHERE bucket_start_epoch = ?1 AND dimension = 'overall'",
        )
        .bind(hour_start)
        .fetch_one(&pool)
        .await
        .expect("rebuilt hourly rollup");
    let queued_repairs =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("repair queue count");
    assert_eq!(daily, (1, 100, 0.1));
    assert_eq!(hourly, (1, 100, 0.1));
    assert_eq!(queued_repairs, 0);
}

#[tokio::test]
async fn initial_full_rebuild_publishes_a_complete_source_snapshot_without_hourly_proof() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("publish a complete initial source snapshot without canonical hourly proof");

    let materialized_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count materialized daily rollups");
    let queued_repairs = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("count deferred repairs");
    let state = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, statistics_start_date FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("load initial full rebuild state");

    assert_eq!(materialized_rows, 1);
    assert_eq!(queued_repairs, 0);
    assert_eq!(state.0, LONG_TERM_STATUS_READY);
    let expected_start = date.to_string();
    assert_eq!(state.1.as_deref(), Some(expected_start.as_str()));
}

#[tokio::test]
async fn initial_full_rebuild_recovers_a_queued_repair_from_a_complete_source_snapshot() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
    sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 2, 200, 0.2, 1, 100, 0.1, 'untrusted hourly proof')",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("queue repair without canonical hourly proof");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("complete initial source snapshot resolves the queued repair");

    let materialized_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count materialized daily rollups");
    let queued_repairs = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("count resolved repairs");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("load completed initial materialization state");

    assert_eq!(materialized_rows, 1);
    assert_eq!(queued_repairs, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_audits_and_repairs_historical_long_term_rollups_idempotently() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (first_hour, second_hour) = seed_long_term_integrity_case(&pool, date, 2).await;

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("historical integrity repair");

    let daily = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("repaired daily rollup");
    assert_eq!(daily.0, 2);
    assert_eq!(daily.1, 200);
    assert!((daily.2 - 0.2).abs() < 1e-9);
    let hourly = sqlx::query_as::<_, (i64, i64)>(
            "SELECT bucket_start_epoch, calls FROM long_term_usage_hourly WHERE dimension = 'overall' AND bucket_start_epoch IN (?1, ?2) ORDER BY bucket_start_epoch",
        )
        .bind(first_hour)
        .bind(second_hour)
        .fetch_all(&pool)
        .await
        .expect("repaired hourly rollups");
    assert_eq!(hourly, vec![(first_hour, 1), (second_hour, 1)]);
    let model_calls = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(calls), 0) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'model'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("model dimension");
    let upstream_calls = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(calls), 0) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'upstream'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("upstream dimension");
    assert_eq!(model_calls, 2);
    assert_eq!(upstream_calls, 2);
    let queue_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("repair queue count");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("repair status");
    assert_eq!(queue_count, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("repeat historical integrity repair");
    let repeated_daily = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("idempotent daily rollup");
    let repeated_hourly = sqlx::query_as::<_, (i64, i64)>(
            "SELECT bucket_start_epoch, calls FROM long_term_usage_hourly WHERE dimension = 'overall' AND bucket_start_epoch IN (?1, ?2) ORDER BY bucket_start_epoch",
        )
        .bind(first_hour)
        .bind(second_hour)
        .fetch_all(&pool)
        .await
        .expect("idempotent hourly rollups");
    assert_eq!(repeated_daily, daily);
    assert_eq!(repeated_hourly, hourly);
}

#[tokio::test]
async fn refresh_repairs_bad_rollups_after_complete_source_reconciliation() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    seed_long_term_integrity_case(&pool, date, 1).await;

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("complete source repair");

    let repaired_calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("repaired daily rollup");
    let queue_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("repair queue count");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("ready state");
    assert_eq!(repaired_calls, 1);
    assert_eq!(queue_count, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_replaces_a_stale_day_with_empty_canonical_totals() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start, day_end) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("stale daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("stale hourly rollup");
    for (dimension, series_key, display_name) in [
        ("model", "model:v2:stale", "stale model"),
        ("upstream", "upstream:stale", "stale upstream"),
    ] {
        sqlx::query(
                "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(date.to_string())
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension daily rollup");
        sqlx::query(
                "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(day_start + 10 * 60 * 60)
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension hourly rollup");
    }
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("ready state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("repair empty canonical day");

    let daily_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("empty repaired daily rows");
    let hourly_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
        )
        .bind(day_start)
        .bind(day_end)
        .fetch_one(&pool)
        .await
        .expect("empty repaired hourly rows");
    let queue_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("cleared repair queue");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("recovered status");
    assert_eq!(daily_rows, 0);
    assert_eq!(hourly_rows, 0);
    assert_eq!(queue_count, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_applies_backoff_when_a_queued_repair_is_source_incomplete() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("durable complete daily rollup");
    sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 2, 200, 0.2, 1, 100, 0.1, 'source data unavailable')",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("due repair queue entry");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("ready state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("incomplete repair is deferred");

    let first = sqlx::query_as::<_, (i64, i64, String)>(
            "SELECT attempts, expected_calls, next_retry_at FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("backed-off repair queue entry");
    let durable_calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("preserved daily rollup");
    assert_eq!(first.0, 1);
    assert_eq!(first.1, 2);
    assert!(!first.2.is_empty());
    assert_eq!(durable_calls, 2);

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("backoff suppresses immediate retry");
    let attempts = sqlx::query_scalar::<_, i64>(
        "SELECT attempts FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("persistent retry attempts");
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn refresh_replaces_a_queued_repair_with_an_empty_complete_source() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, 'canonical', 1, 1, 100, 0.1, 100, 0.1)",
        )
        .bind(day_start_epoch + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical hourly rollup");
    sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 1, 100, 0.1, 0, 0, 0, 'source data unavailable')",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("queued repair");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("error state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("empty complete source is reconciled");

    let first_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("pending error state");
    let queue_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("queued repair count");
    let empty_daily_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("empty daily row count");
    assert_eq!(first_status, LONG_TERM_STATUS_EMPTY);
    assert_eq!(queue_count, 0);
    assert_eq!(empty_daily_rows, 0);

    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh materializes after source arrives");

    let repaired_calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("repaired daily rollup");
    let queue_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("cleared repair queue");
    let final_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("recovered ready state");
    assert_eq!(repaired_calls, 1);
    assert_eq!(queue_count, 0);
    assert_eq!(final_status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_recovers_after_a_real_sqlite_lock_releases() {
    let (pool, db_url, db_path) = long_term_file_backed_pool("long-term-lock-release").await;
    create_long_term_test_invocations(&pool).await;
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    insert_long_term_test_invocation(&pool, 1, format!("{today}T12:00:00+08:00")).await;

    let mut lock_connection = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_connection)
        .await
        .expect("acquire sqlite write lock");
    let refresh_pool = pool.clone();
    let refresh_task =
        tokio::spawn(async move { refresh_long_term_stats(&refresh_pool, 400).await });

    sleep(Duration::from_millis(125)).await;
    sqlx::query("COMMIT")
        .execute(&mut lock_connection)
        .await
        .expect("release sqlite write lock");
    refresh_task
        .await
        .expect("join refresh task")
        .expect("refresh should retry after lock release");

    let calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(today.to_string())
    .fetch_one(&pool)
    .await
    .expect("published daily rollup");
    assert_eq!(calls, 1);

    lock_connection.close().await.expect("close lock holder");
    cleanup_long_term_file_backed_pool(pool, db_path).await;
}

#[tokio::test]
async fn refresh_exhausts_real_sqlite_locks_without_partial_publication() {
    let (pool, db_url, db_path) = long_term_file_backed_pool("long-term-lock-exhaustion").await;
    create_long_term_test_invocations(&pool).await;
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    insert_long_term_test_invocation(&pool, 1, format!("{today}T12:00:00+08:00")).await;
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 10, 1000, 10, 1.0, 10)",
        )
        .bind(today.to_string())
        .execute(&pool)
        .await
        .expect("durable daily rollup");

    let mut lock_connection = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_connection)
        .await
        .expect("acquire sqlite write lock");
    let error = refresh_long_term_stats(&pool, 400)
        .await
        .expect_err("persistent sqlite lock should exhaust retry budget");
    assert!(crate::is_sqlite_lock_error(&error));

    sqlx::query("ROLLBACK")
        .execute(&mut lock_connection)
        .await
        .expect("release sqlite write lock");
    let calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(today.to_string())
    .fetch_one(&pool)
    .await
    .expect("preserved daily rollup");
    let hourly_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
        )
        .bind(
            long_term_day_epoch_bounds(today)
                .expect("Shanghai day bounds")
                .0,
        )
        .bind(
            long_term_day_epoch_bounds(today)
                .expect("Shanghai day bounds")
                .1,
        )
        .fetch_one(&pool)
        .await
        .expect("no partial hourly rollup");
    assert_eq!(calls, 10);
    assert_eq!(hourly_rows, 0);

    lock_connection.close().await.expect("close lock holder");
    cleanup_long_term_file_backed_pool(pool, db_path).await;
}

#[tokio::test]
async fn long_term_refresh_retries_a_sqlite_lock_until_the_write_is_available() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let outcome = run_long_term_refresh_with_retry_delays(
        None,
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            async move {
                let attempt = operation_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt < 3 {
                    Err(anyhow::anyhow!("database is locked"))
                } else {
                    Ok(attempt)
                }
            }
        },
        &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
    )
    .await
    .expect("lock should clear before retry budget is exhausted");
    assert_eq!(outcome, 3);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn long_term_refresh_stops_after_its_bounded_sqlite_lock_retry_budget() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let error = run_long_term_refresh_with_retry_delays(
        None,
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            async move {
                operation_attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(anyhow::anyhow!("database is locked"))
            }
        },
        &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
    )
    .await
    .expect_err("persistent locks must exhaust the bounded retry budget");
    assert!(crate::is_sqlite_lock_error(&error));
    assert_eq!(attempts.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn long_term_refresh_cancels_before_a_second_sqlite_lock_retry() {
    let shutdown = CancellationToken::new();
    let operation_shutdown = shutdown.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let error = run_long_term_refresh_with_retry_delays(
        Some(&shutdown),
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            let operation_shutdown = operation_shutdown.clone();
            async move {
                operation_attempts.fetch_add(1, Ordering::SeqCst);
                operation_shutdown.cancel();
                Err::<(), _>(anyhow::anyhow!("database is locked"))
            }
        },
        &[Duration::from_secs(1)],
    )
    .await
    .expect_err("shutdown stops the initial refresh before another lock retry");
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn projection_flush_retries_a_transient_sqlite_lock() {
    let shutdown = CancellationToken::new();
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let outcome = run_long_term_projection_flush_with_retry_delays(
        &shutdown,
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            async move {
                let attempt = operation_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt < 3 {
                    Err(anyhow::anyhow!("database is locked"))
                } else {
                    Ok(attempt)
                }
            }
        },
        &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
    )
    .await
    .expect("transient lock retries before deferring a repair");
    assert_eq!(outcome, 3);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn projection_flush_cancels_during_sqlite_lock_retry() {
    let shutdown = CancellationToken::new();
    let operation_shutdown = shutdown.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let error = run_long_term_projection_flush_with_retry_delays(
        &shutdown,
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            let operation_shutdown = operation_shutdown.clone();
            async move {
                operation_attempts.fetch_add(1, Ordering::SeqCst);
                operation_shutdown.cancel();
                Err::<(), _>(anyhow::anyhow!("database is locked"))
            }
        },
        &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
    )
    .await
    .expect_err("shutdown stops the lock retry without another write attempt");
    assert!(long_term_projection_write_is_deferred(&error));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}
