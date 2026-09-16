#[tokio::test]
async fn integrity_audit_detects_nonzero_rollups_without_canonical_hourly_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_integrity_oracle(&pool).await;
    let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
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
        .bind(start_epoch + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("stale hourly rollup");

    let mismatches = audit_long_term_integrity(&pool, date, date)
        .await
        .expect("audit stale rollups");
    assert_eq!(mismatches.len(), 1);
    assert_eq!(mismatches[0].date, date);
    assert_eq!(mismatches[0].expected.calls, 0);
    assert_eq!(mismatches[0].observed.calls, 1);
}

#[tokio::test]
async fn integrity_audit_detects_dimension_rows_on_an_empty_canonical_day() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_integrity_oracle(&pool).await;
    let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
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
            .bind(start_epoch + 10 * 60 * 60)
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension hourly rollup");
    }

    let mismatches = audit_long_term_integrity(&pool, date, date)
        .await
        .expect("audit dimension-only stale rollups");
    assert_eq!(mismatches.len(), 1);
    assert_eq!(mismatches[0].date, date);
    assert_eq!(mismatches[0].expected, LongTermIntegrityTotals::default());
    assert_eq!(mismatches[0].observed, LongTermIntegrityTotals::default());
    assert!(mismatches[0].reason.contains("one or more dimensions"));
}

#[tokio::test]
async fn integrity_audit_detects_dimension_rows_on_an_active_only_canonical_day() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_integrity_oracle(&pool).await;
    let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'active-only', 1, 0, 0, 0, 1, 100, 0.1)",
        )
        .bind(start_epoch + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("active-only canonical hourly rollup");
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
            .bind(start_epoch + 10 * 60 * 60)
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension hourly rollup");
    }

    let mismatches = audit_long_term_integrity(&pool, date, date)
        .await
        .expect("audit active-only canonical day");
    assert_eq!(mismatches.len(), 1);
    assert_eq!(mismatches[0].date, date);
    assert_eq!(mismatches[0].expected, LongTermIntegrityTotals::default());
    assert_eq!(mismatches[0].observed, LongTermIntegrityTotals::default());
    assert!(mismatches[0].reason.contains("terminal totals are empty"));
}

#[tokio::test]
async fn integrity_audit_keeps_zero_total_wall_time_continuations() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_integrity_oracle(&pool).await;
    let date = NaiveDate::from_ymd_opt(2026, 7, 23).expect("fixed date");
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");

    // A call that began before midnight may contribute wall time to this date without
    // contributing a canonical terminal invocation bucket of its own.
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, wall_time_ms, wall_time_samples) VALUES (?1, 'overall', 'overall', '全部调用', 60000, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("wall-time continuation daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, wall_time_ms, wall_time_samples) VALUES (?1, 'overall', 'overall', '全部调用', 60000, 1)",
        )
        .bind(start_epoch)
        .execute(&pool)
        .await
        .expect("wall-time continuation hourly rollup");

    let mismatches = audit_long_term_integrity(&pool, date, date)
        .await
        .expect("audit zero-total wall-time continuation");
    assert!(
        mismatches.is_empty(),
        "empty canonical totals must not erase a valid cross-day wall-time continuation"
    );
}

#[tokio::test]
async fn refresh_skips_audit_before_the_persisted_reconstructable_start() {
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
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let unavailable_date = today - ChronoDuration::days(5);
    let reconstructable_start = today - ChronoDuration::days(3);
    let (hour_start, _) =
        long_term_day_epoch_bounds(unavailable_date).expect("Shanghai day bounds");
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'canonical', 2, 2, 200, 0.2, 0, 200, 0.2)",
        )
        .bind(hour_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical unavailable-prefix hour");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(unavailable_date.to_string())
        .execute(&pool)
        .await
        .expect("stale unavailable-prefix daily rollup");
    sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 2, 200, 0.2, 1, 100, 0.1, 'unreadable archive prefix')",
        )
        .bind(unavailable_date.to_string())
        .execute(&pool)
        .await
        .expect("inactive unavailable-prefix repair");
    sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2, last_integrity_audit_at = NULL WHERE id = ?3",
        )
        .bind(LONG_TERM_STATUS_READY)
        .bind(reconstructable_start.to_string())
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("persist reconstructable suffix");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh valid suffix");

    let queued_repairs =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("queued repair count");
    let attempts = sqlx::query_scalar::<_, i64>(
        "SELECT attempts FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(unavailable_date.to_string())
    .fetch_one(&pool)
    .await
    .expect("inactive repair attempts");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("long-term status");
    assert_eq!(queued_repairs, 1);
    assert_eq!(attempts, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[test]
fn targeted_repair_merges_archive_details_into_retained_live_row() {
    let retained_live = LongTermInvocationRow {
        id: 1,
        invoke_id: Some("same-invocation".to_string()),
        occurred_at: "2026-07-26 10:00:00".to_string(),
        status: Some("success".to_string()),
        model: None,
        request_model: None,
        response_model: None,
        reasoning_effort: None,
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
    let mut archived = LongTermInvocationRow {
        model: Some("archive-model".to_string()),
        request_model: Some("request-model".to_string()),
        response_model: Some("response-model".to_string()),
        reasoning_effort: Some("high".to_string()),
        upstream_account_id: Some(42),
        ..retained_live.clone()
    };

    merge_long_term_invocation_row(&mut archived, &retained_live);

    assert_eq!(archived.status.as_deref(), Some("success"));
    assert_eq!(archived.model.as_deref(), Some("archive-model"));
    assert_eq!(archived.response_model.as_deref(), Some("response-model"));
    assert_eq!(archived.reasoning_effort.as_deref(), Some("high"));
    assert_eq!(archived.upstream_account_id, Some(42));
}

#[tokio::test]
async fn incremental_projection_prunes_expired_hourly_rollups_and_intervals() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let retention_start = long_term_projection_hourly_retention_start_date(366);
    let expired_date = retention_start.pred_opt().expect("expired date");
    let retained_date = retention_start;
    let epoch = |date: NaiveDate| {
        date.and_hms_opt(0, 0, 0)
            .and_then(|value| Shanghai.from_local_datetime(&value).single())
            .expect("shanghai hour")
            .timestamp()
    };
    let expired_epoch = epoch(expired_date);
    let retained_epoch = epoch(retained_date);
    for (bucket_start_epoch, key) in [(expired_epoch, "expired"), (retained_epoch, "retained")] {
        let bucket_date = Shanghai
            .timestamp_opt(bucket_start_epoch, 0)
            .single()
            .expect("shanghai bucket")
            .date_naive();
        sqlx::query(
                "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name) VALUES (?1, 'overall', ?2, 'All')",
            )
            .bind(bucket_start_epoch)
            .bind(key)
            .execute(&pool)
            .await
            .expect("hourly rollup");
        sqlx::query(
                "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('hourly', ?1, ?2, 'overall', ?3, 1, 0, 1)",
            )
            .bind(bucket_date.to_string())
            .bind(bucket_start_epoch.to_string())
            .bind(key)
            .execute(&pool)
            .await
            .expect("hourly interval");
    }
    sqlx::query(
            "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('daily', ?1, ?1, 'overall', 'daily-retained', 1, 0, 1)",
        )
        .bind(expired_date.to_string())
        .execute(&pool)
        .await
        .expect("daily interval");

    let (pruned_hourly_rows, pruned_interval_rows) =
        prune_long_term_projection_hourly_retention(&pool, 366)
            .await
            .expect("prune one bounded hourly retention batch");

    assert_eq!(pruned_hourly_rows, 1);
    assert_eq!(pruned_interval_rows, 0);
    let (pruned_hourly_rows, pruned_interval_rows) =
        prune_long_term_projection_hourly_retention(&pool, 366)
            .await
            .expect("prune one bounded legacy retention batch");
    assert_eq!(pruned_hourly_rows, 0);
    assert_eq!(pruned_interval_rows, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_hourly")
            .fetch_one(&pool)
            .await
            .expect("retained hourly count"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("retained interval count"),
        2
    );
}

fn projection_interval_segment(
    invocation_row_id: i64,
    interval_start_ms: i64,
    interval_end_ms: i64,
) -> LongTermProjectionIntervalSegment {
    LongTermProjectionIntervalSegment {
        invocation_row_id,
        model_series_key: "model:gpt-5:high".to_string(),
        upstream_series_key: "account:42".to_string(),
        interval_start_ms,
        interval_end_ms,
    }
}

#[tokio::test]
async fn projection_interval_state_compacts_each_invocation_and_survives_reopen() {
    let (pool, db_url, db_path) = long_term_file_backed_pool("projection-interval-state").await;
    let start_ms = Shanghai
        .with_ymd_and_hms(2026, 7, 26, 10, 0, 0)
        .single()
        .expect("Shanghai start")
        .timestamp_millis();
    let segments = (1..=256)
        .map(|id| projection_interval_segment(id, start_ms, start_ms + 1_000))
        .collect::<Vec<_>>();
    let control = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(&pool, &segments, &control)
        .await
        .expect("canonical interval state");
    let state_rows =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("canonical state row count");
    assert_eq!(state_rows, 256);
    let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
    apply_long_term_projection_incremental_with_runtime(
        &pool,
        &runtime,
        &HashMap::new(),
        &HashMap::new(),
        &[],
        321,
        0,
    )
    .await
    .expect("persist projection cursor with canonical state");
    pool.close().await;

    let reopened = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .expect("reopen durable projection state");
    let dates = HashSet::from(["2026-07-26".to_string()]);
    let index = load_long_term_projection_interval_index(&reopened, &dates)
        .await
        .expect("load canonical interval index");
    let key = projection_interval_key(
        "daily",
        "2026-07-26".to_string(),
        "model".to_string(),
        "model:gpt-5:high".to_string(),
    );
    assert_eq!(
        index.get(&key).expect("model daily union").duration_ms,
        1_000
    );
    assert_eq!(
        load_long_term_projection_cursor(&reopened)
            .await
            .expect("reopened projection cursor"),
        321
    );
    cleanup_long_term_file_backed_pool(reopened, db_path).await;
}

#[tokio::test]
async fn incremental_projection_reuses_interval_index_across_micro_batches() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let start_ms = start_epoch * 1_000;
    let control = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(
        &pool,
        &[projection_interval_segment(1, start_ms, start_ms + 1_000)],
        &control,
    )
    .await
    .expect("seed existing interval state");
    let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
    let first_segment = projection_interval_segment(2, start_ms + 1_000, start_ms + 2_000);
    let first_outcome = apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &HashMap::new(),
            daily: &HashMap::new(),
            segments: std::slice::from_ref(&first_segment),
        },
        2,
        1,
        &control,
    )
    .await
    .expect("first incremental micro-batch");
    assert_eq!(
        first_outcome,
        LongTermProjectionIncrementalOutcome::Published
    );

    // A same-date batch must reuse the loaded union. Clearing the backing rows makes an
    // accidental reload observable while leaving the in-memory cache unchanged.
    sqlx::query("DELETE FROM long_term_projection_interval_state")
        .execute(&pool)
        .await
        .expect("remove persisted rows after first cache load");
    let second_segment = projection_interval_segment(3, start_ms + 2_000, start_ms + 3_000);
    let second_outcome = apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &HashMap::new(),
            daily: &HashMap::new(),
            segments: std::slice::from_ref(&second_segment),
        },
        3,
        1,
        &control,
    )
    .await
    .expect("second incremental micro-batch");
    assert_eq!(
        second_outcome,
        LongTermProjectionIncrementalOutcome::Published
    );
    let runtime = runtime.lock().await;
    let daily_key = projection_interval_key(
        "daily",
        date.to_string(),
        "overall".to_string(),
        "overall".to_string(),
    );
    assert_eq!(
        runtime
            .interval_index
            .get(&daily_key)
            .expect("same-date cache entry")
            .duration_ms,
        3_000,
        "the second micro-batch must not discard the prior same-date union"
    );
}

#[tokio::test]
async fn canonical_interval_lookup_seeks_from_interval_end() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let start_ms = start_epoch * 1_000;
    let end_ms = end_epoch * 1_000;
    sqlx::query(
            "INSERT INTO long_term_projection_interval_state (invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms) VALUES (1, 'model:old', 'account:old', ?1, ?2), (2, 'model:current', 'account:current', ?3, ?4)",
        )
        .bind(start_ms - 86_400_000)
        .bind(start_ms - 1)
        .bind(start_ms + 1_000)
        .bind(start_ms + 2_000)
        .execute(&pool)
        .await
        .expect("seed canonical intervals");
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
            "EXPLAIN QUERY PLAN SELECT invocation_row_id FROM long_term_projection_interval_state WHERE interval_start_ms < ?1 AND interval_end_ms > ?2",
        )
        .bind(end_ms)
        .bind(start_ms)
        .fetch_all(&pool)
        .await
        .expect("canonical interval query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_long_term_projection_interval_state_end_start")
    }));

    let index = load_long_term_projection_interval_index(&pool, &HashSet::from([date.to_string()]))
        .await
        .expect("load canonical interval index");
    let key = projection_interval_key(
        "daily",
        date.to_string(),
        "model".to_string(),
        "model:current".to_string(),
    );
    assert_eq!(
        index.get(&key).expect("current interval union").duration_ms,
        1_000
    );
}

#[tokio::test]
async fn legacy_interval_migration_seeks_by_invocation_id() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
        r#"
            EXPLAIN QUERY PLAN
            SELECT legacy.invocation_row_id,
                   MAX(CASE WHEN legacy.dimension = 'model' THEN legacy.series_key END),
                   MAX(CASE WHEN legacy.dimension = 'upstream' THEN legacy.series_key END),
                   MIN(legacy.interval_start_ms),
                   MAX(legacy.interval_end_ms)
            FROM long_term_projection_intervals legacy
            WHERE NOT EXISTS (
                SELECT 1
                FROM long_term_projection_interval_state state
                WHERE state.invocation_row_id = legacy.invocation_row_id
            )
            GROUP BY legacy.invocation_row_id
            ORDER BY legacy.invocation_row_id ASC
            LIMIT 512
            "#,
    )
    .fetch_all(&pool)
    .await
    .expect("legacy migration query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_long_term_projection_intervals_invocation")
    }));
}

#[tokio::test]
async fn legacy_interval_fallback_honors_rebuild_suppressions() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = "2026-07-26";
    sqlx::query(
            "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('daily', ?1, ?1, 'overall', 'overall', 77, 1784992800000, 1784992801000)",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("seed legacy-only interval");
    sqlx::query(
            "INSERT INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date) VALUES (77, ?1)",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("suppress removed legacy interval");

    let index = load_long_term_projection_interval_index(&pool, &HashSet::from([date.to_string()]))
        .await
        .expect("load legacy fallback");
    assert!(
        index.is_empty(),
        "suppressed legacy state must not resurrect"
    );
}

#[tokio::test]
async fn projection_interval_state_retention_prunes_canonical_state_and_metadata() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let retention_start = long_term_projection_hourly_retention_start_date(366);
    let old_date = retention_start.pred_opt().expect("old retention date");
    let old_start_ms = Shanghai
        .from_local_datetime(&old_date.and_hms_opt(0, 0, 0).expect("old start"))
        .single()
        .expect("old Shanghai start")
        .timestamp_millis();
    let retained_start_ms = Shanghai
        .from_local_datetime(
            &retention_start
                .and_hms_opt(0, 0, 0)
                .expect("retention start"),
        )
        .single()
        .expect("retained Shanghai start")
        .timestamp_millis();
    let control = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(
        &pool,
        &[
            projection_interval_segment(1, old_start_ms, old_start_ms + 1_000),
            projection_interval_segment(2, retained_start_ms, retained_start_ms + 1_000),
        ],
        &control,
    )
    .await
    .expect("seed canonical state");
    sqlx::query(
            "INSERT INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date) VALUES (1, ?1)",
        )
        .bind(old_date.to_string())
        .execute(&pool)
        .await
        .expect("old suppression");
    sqlx::query(
            "INSERT INTO long_term_projection_rebuild_members (rebuild_token, invocation_row_id) VALUES ('old-retention', 1)",
        )
        .execute(&pool)
        .await
        .expect("old rebuild membership");

    let mut pruned_intervals = 0;
    loop {
        let (_, pruned) =
            prune_long_term_projection_hourly_retention_with_control(&pool, 366, &control)
                .await
                .expect("bounded retention pass");
        if pruned == 0 {
            break;
        }
        assert!(pruned <= LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as u64);
        pruned_intervals += pruned;
    }
    assert_eq!(pruned_intervals, 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("old state count"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 2",
        )
        .fetch_one(&pool)
        .await
        .expect("retained state count"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_interval_suppressions",
        )
        .fetch_one(&pool)
        .await
        .expect("suppression cleanup"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_rebuild_members",)
            .fetch_one(&pool)
            .await
            .expect("member cleanup"),
        0
    );
}

#[tokio::test]
async fn projection_interval_retention_batches_defer_cancel_and_resume() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let retention_start = long_term_projection_hourly_retention_start_date(366);
    let expired_date = retention_start.pred_opt().expect("expired retention date");
    let expired_start_ms = Shanghai
        .from_local_datetime(&expired_date.and_hms_opt(0, 0, 0).expect("expired start"))
        .single()
        .expect("expired Shanghai start")
        .timestamp_millis();
    let retained_start_ms = Shanghai
        .from_local_datetime(
            &retention_start
                .and_hms_opt(0, 0, 0)
                .expect("retained start"),
        )
        .single()
        .expect("retained Shanghai start")
        .timestamp_millis();
    let mut segments = (1..=1_025)
        .map(|id| projection_interval_segment(id, expired_start_ms, expired_start_ms + 1_000))
        .collect::<Vec<_>>();
    segments.push(projection_interval_segment(
        2_000,
        retained_start_ms,
        retained_start_ms + 1_000,
    ));
    let unrestricted = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(&pool, &segments, &unrestricted)
        .await
        .expect("seed canonical interval state");
    for invocation_row_id in 1..=1_025 {
        sqlx::query(
                "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('hourly', ?1, ?2, 'overall', 'overall', ?3, ?4, ?5)",
            )
            .bind(expired_date.to_string())
            .bind(format!("retention-{invocation_row_id}"))
            .bind(invocation_row_id)
            .bind(expired_start_ms)
            .bind(expired_start_ms + 1_000)
            .execute(&pool)
            .await
            .expect("seed expanded legacy interval");
        sqlx::query(
                "INSERT INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date) VALUES (?1, ?2)",
            )
            .bind(invocation_row_id)
            .bind(expired_date.to_string())
            .execute(&pool)
            .await
            .expect("seed expired suppression");
        sqlx::query(
                "INSERT INTO long_term_projection_rebuild_members (rebuild_token, invocation_row_id) VALUES (?1, ?2)",
            )
            .bind(format!("retention-{invocation_row_id}"))
            .bind(invocation_row_id)
            .execute(&pool)
            .await
            .expect("seed expired rebuild member");
    }

    let pressure_shutdown = CancellationToken::new();
    let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = pressure_gate
        .try_begin_background("retention-test-holder")
        .expect("hold background admission");
    let pressure_control =
        LongTermProjectionWriteControl::background(&pressure_shutdown, &pressure_gate);
    let deferred =
        prune_long_term_projection_hourly_retention_with_control(&pool, 366, &pressure_control)
            .await
            .expect_err("retention must defer before opening a write transaction under pressure");
    assert!(long_term_projection_write_is_deferred(&deferred));
    drop(held);
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id <= 1025",
            )
            .fetch_one(&pool)
            .await
            .expect("canonical rows remain after pressure deferral"),
            1_025
        );

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let interrupted =
        LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &committed_batches, 1);
    let (_, pruned_intervals) =
        prune_long_term_projection_hourly_retention_with_control(&pool, 366, &interrupted)
            .await
            .expect("one bounded retention batch commits before cancellation");
    assert_eq!(
        pruned_intervals,
        LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as u64
    );
    assert_eq!(committed_batches.load(Ordering::Acquire), 1);
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id <= 1025",
            )
            .fetch_one(&pool)
            .await
                .expect("canonical rows remain before their retention turn"),
            1_025
        );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("one legacy retention batch leaves durable work for the next pass"),
        513
    );
    assert!(
        long_term_projection_maintenance_needed(&pool, 366)
            .await
            .expect("durable maintenance backlog remains scheduled after one batch")
    );
    let error = prune_long_term_projection_hourly_retention_with_control(&pool, 366, &interrupted)
        .await
        .expect_err("cancellation stops the next maintenance pass before its write");
    assert!(error.to_string().contains("cancelled"));

    let mut resumed_pruned_intervals = 0;
    loop {
        let (_, pruned) =
            prune_long_term_projection_hourly_retention_with_control(&pool, 366, &unrestricted)
                .await
                .expect("retention resumes after cancellation");
        if pruned == 0 {
            break;
        }
        assert!(pruned <= LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as u64);
        resumed_pruned_intervals += pruned;
    }
    assert_eq!(resumed_pruned_intervals, 3_588);
    assert!(
        !long_term_projection_maintenance_needed(&pool, 366)
            .await
            .expect("drained maintenance no longer schedules a tick")
    );
    for table in [
        "long_term_projection_interval_state",
        "long_term_projection_interval_suppressions",
        "long_term_projection_rebuild_members",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COUNT(*) FROM {table} WHERE invocation_row_id <= 1025"
            ))
            .fetch_one(&pool)
            .await
            .expect("expired retention rows removed after resume"),
            0,
            "{table} must resume bounded cleanup"
        );
    }
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 2000",
            )
            .fetch_one(&pool)
            .await
            .expect("retained canonical row"),
            1
        );
}

#[tokio::test]
async fn projection_interval_state_batches_stop_for_cancel_and_pressure() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    let start_ms = Shanghai
        .with_ymd_and_hms(2026, 7, 26, 10, 0, 0)
        .single()
        .expect("Shanghai start")
        .timestamp_millis();
    let segments = (1..=1_025)
        .map(|id| projection_interval_segment(id, start_ms, start_ms + 1_000))
        .collect::<Vec<_>>();
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let cancel_control =
        LongTermProjectionWriteControl::cancelling_after(&shutdown, &gate, &committed_batches, 1);
    let cancelled =
        upsert_long_term_projection_interval_segments(&pool, &segments, &cancel_control)
            .await
            .expect_err("shutdown cancels the next internal write batch");
    assert!(cancelled.to_string().contains("cancelled"));
    assert!(shutdown.is_cancelled());
    assert_eq!(committed_batches.load(Ordering::Acquire), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state",)
            .fetch_one(&pool)
            .await
            .expect("committed canonical rows"),
        LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64
    );

    let pressure_shutdown = CancellationToken::new();
    let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = pressure_gate
        .try_begin_background("test-holder")
        .expect("hold background admission");
    let pressure_control =
        LongTermProjectionWriteControl::background(&pressure_shutdown, &pressure_gate);
    let deferred = upsert_long_term_projection_interval_segments(
        &pool,
        &[projection_interval_segment(
            2_000,
            start_ms,
            start_ms + 1_000,
        )],
        &pressure_control,
    )
    .await
    .expect_err("pressure must defer before opening a write transaction");
    assert!(long_term_projection_write_is_deferred(&deferred));
    drop(held);
}

#[tokio::test]
async fn incremental_projection_defers_before_interval_and_rollup_publication() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let event = build_long_term_projection_event(&LongTermInvocationRow {
        id: 1,
        invoke_id: Some("atomic-pressure".to_string()),
        occurred_at: "2026-07-26 10:00:00".to_string(),
        status: Some("success".to_string()),
        model: Some("gpt-5".to_string()),
        request_model: None,
        response_model: None,
        reasoning_effort: None,
        upstream_account_id: None,
        upstream_account_kind: None,
        upstream_account_name: None,
        total_tokens: Some(100),
        output_tokens: None,
        cost: None,
        t_total_ms: Some(1_000.0),
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        t_upstream_stream_ms: None,
        error_message: None,
    });
    let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = gate
        .try_begin_background("test-holder")
        .expect("hold background admission");
    let pressure_control = LongTermProjectionWriteControl::background(&shutdown, &gate);
    let deferred = apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &event.hourly,
            daily: &event.daily,
            segments: &event.segments,
        },
        event.row_id,
        1,
        &pressure_control,
    )
    .await
    .expect_err("pressure rejects the atomic incremental publication");
    assert!(long_term_projection_write_is_deferred(&deferred));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("no canonical interval before publication"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_daily")
            .fetch_one(&pool)
            .await
            .expect("no rollup before publication"),
        0
    );
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE((SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1), 0)",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_one(&pool)
            .await
            .expect("projection cursor"),
            0
        );

    drop(held);
    let control = LongTermProjectionWriteControl::unrestricted();
    apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &event.hourly,
            daily: &event.daily,
            segments: &event.segments,
        },
        event.row_id,
        1,
        &control,
    )
    .await
    .expect("retry publishes the entire event once");
    let daily = sqlx::query_as::<_, (i64, i64)>(
            "SELECT calls, token_total FROM long_term_usage_daily WHERE stats_date = '2026-07-26' AND dimension = 'overall' AND series_key = 'overall'",
        )
        .fetch_one(&pool)
        .await
        .expect("daily projection");
    assert_eq!(daily, (1, 100));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("canonical interval after retry"),
        1
    );
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("advanced projection cursor"),
        event.row_id
    );
}

#[tokio::test]
async fn incremental_projection_rebuilds_when_a_dirty_marker_arrives_after_ready_read() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let event = build_long_term_projection_event(&LongTermInvocationRow {
        id: 1,
        invoke_id: Some("dirty-race".to_string()),
        occurred_at: "2026-07-26 10:00:00".to_string(),
        status: Some("success".to_string()),
        model: Some("gpt-5".to_string()),
        request_model: None,
        response_model: None,
        reasoning_effort: None,
        upstream_account_id: None,
        upstream_account_kind: None,
        upstream_account_name: None,
        total_tokens: Some(100),
        output_tokens: None,
        cost: None,
        t_total_ms: Some(1_000.0),
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        t_upstream_stream_ms: None,
        error_message: None,
    });
    let date = "2026-07-26";
    sqlx::query(
            "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 1)",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("seed ready bucket");
    let ready_dates = load_long_term_projection_ready_dates(&pool, &event.bucket_dates)
        .await
        .expect("ready snapshot");
    assert!(event.bucket_dates.is_subset(&ready_dates));

    // A correction can land after the ready read and before the incremental transaction.
    sqlx::query(
            "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, 'test-race')",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("queue correction after ready read");
    let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
    let control = LongTermProjectionWriteControl::unrestricted();
    let outcome = apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &event.hourly,
            daily: &event.daily,
            segments: &event.segments,
        },
        event.row_id,
        1,
        &control,
    )
    .await
    .expect("dirty revalidation");
    assert_eq!(
        outcome,
        LongTermProjectionIncrementalOutcome::RebuildRequired
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("no canonical interval publication"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_daily")
            .fetch_one(&pool)
            .await
            .expect("no mixed daily publication"),
        0
    );
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("unadvanced cursor"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
        )
        .bind(date)
        .fetch_one(&pool)
        .await
        .expect("durable dirty marker"),
        1
    );
}

#[tokio::test]
async fn legacy_projection_intervals_migrate_without_losing_the_canonical_interval() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    for (bucket_kind, bucket_key, dimension, series_key) in [
        ("daily", "2026-07-26", "overall", "overall"),
        ("daily", "2026-07-26", "model", "model:gpt-5:high"),
        ("daily", "2026-07-26", "upstream", "account:42"),
        ("hourly", "1785031200", "overall", "overall"),
        ("hourly", "1785031200", "model", "model:gpt-5:high"),
        ("hourly", "1785031200", "upstream", "account:42"),
    ] {
        sqlx::query(
                "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES (?1, '2026-07-26', ?2, ?3, ?4, 7, 10, 20)",
            )
            .bind(bucket_kind)
            .bind(bucket_key)
            .bind(dimension)
            .bind(series_key)
            .execute(&pool)
            .await
            .expect("legacy expanded interval");
    }
    let control = LongTermProjectionWriteControl::unrestricted();
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &control)
            .await
            .expect("canonical legacy interval migration")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 7",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical migration state"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("legacy expansion remains for a later bounded cleanup"),
        6
    );
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &control)
            .await
            .expect("bounded legacy cleanup")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("legacy cleanup complete"),
        0
    );
}

#[tokio::test]
async fn legacy_projection_interval_migration_compresses_expansion_in_one_cancellable_batch() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    for invocation_row_id in 1..=256 {
        for bucket_kind in ["daily", "hourly"] {
            for (dimension, series_key) in [
                ("overall", "overall"),
                ("model", "model:gpt-5:high"),
                ("upstream", "account:42"),
            ] {
                sqlx::query(
                        "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES (?1, '2026-07-26', ?2, ?3, ?4, ?5, 10, 20)",
                    )
                    .bind(bucket_kind)
                    .bind(format!("{bucket_kind}-{invocation_row_id}"))
                    .bind(dimension)
                    .bind(series_key)
                    .bind(invocation_row_id)
                    .execute(&pool)
                    .await
                    .expect("legacy expanded interval");
            }
        }
    }

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let interrupted =
        LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &committed_batches, 1);
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &interrupted)
            .await
            .expect("one canonical compression batch")
    );
    assert_eq!(committed_batches.load(Ordering::Acquire), 1);
    let interrupted_state = sqlx::query_as::<_, LongTermProjectionIntervalStateRow>(
            "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms FROM long_term_projection_interval_state WHERE invocation_row_id = 9",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical interval after interrupted cleanup");
    assert_eq!(interrupted_state.model_series_key, "model:gpt-5:high");
    assert_eq!(interrupted_state.upstream_series_key, "account:42");
    assert_eq!(interrupted_state.interval_start_ms, 10);
    assert_eq!(interrupted_state.interval_end_ms, 20);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("all expanded rows remain before cleanup"),
        1_536
    );
    assert!(
        long_term_projection_maintenance_needed(&pool, 366)
            .await
            .expect("legacy expansion keeps the next maintenance pass scheduled")
    );
    let error = migrate_long_term_projection_legacy_interval_state(&pool, &interrupted)
        .await
        .expect_err("cancellation stops the next legacy cleanup batch");
    assert!(error.to_string().contains("cancelled"));

    let unrestricted = LongTermProjectionWriteControl::unrestricted();
    for expected_remaining in [1_024_i64, 512, 0] {
        assert!(
            migrate_long_term_projection_legacy_interval_state(&pool, &unrestricted)
                .await
                .expect("resume one bounded legacy cleanup batch")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
                .fetch_one(&pool)
                .await
                .expect("remaining expanded legacy rows"),
            expected_remaining
        );
    }
    assert!(
        !migrate_long_term_projection_legacy_interval_state(&pool, &unrestricted)
            .await
            .expect("completed migration has no further work")
    );
    let resumed_state = sqlx::query_as::<_, LongTermProjectionIntervalStateRow>(
            "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms FROM long_term_projection_interval_state WHERE invocation_row_id = 9",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical interval after resumed cleanup");
    assert_eq!(resumed_state.model_series_key, "model:gpt-5:high");
    assert_eq!(resumed_state.upstream_series_key, "account:42");
    assert_eq!(resumed_state.interval_start_ms, 10);
    assert_eq!(resumed_state.interval_end_ms, 20);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("completed legacy cleanup"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("one durable interval state per invocation"),
        256
    );
}

#[tokio::test]
async fn interrupted_date_rebuild_keeps_cross_day_interval_for_neighboring_date() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let first_date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("first date");
    let second_date = first_date.succ_opt().expect("second date");
    let (first_start, first_end) = long_term_day_epoch_bounds(first_date).expect("first bounds");
    let (_, second_end) = long_term_day_epoch_bounds(second_date).expect("second bounds");
    let segment =
        projection_interval_segment(11, first_end * 1_000 - 1_000, second_end * 1_000 - 1_000);
    let unrestricted = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(&pool, &[segment], &unrestricted)
        .await
        .expect("seed cross-day canonical interval");
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let cancelling_control =
        LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &committed_batches, 1);
    let rebuild = LongTermProjectionDateRebuild {
        bucket_date: first_date.to_string(),
        start_epoch: first_start,
        end_epoch: first_end,
        hourly: HashMap::new(),
        daily: HashMap::new(),
        interval_segments: Vec::new(),
        source_row_count: 0,
    };
    commit_long_term_projection_date_rebuilds_with_control(
        &pool,
        &[rebuild],
        None,
        &[],
        false,
        &cancelling_control,
    )
    .await
    .expect_err("rebuild cancellation after the staging batch");
    let neighboring_index =
        load_long_term_projection_interval_index(&pool, &HashSet::from([second_date.to_string()]))
            .await
            .expect("neighboring durable interval index");
    assert!(
        neighboring_index
            .values()
            .any(|union| union.duration_ms > 0)
    );
}

#[tokio::test]
async fn daily_backup_claim_keeps_competing_rebuild_from_publishing_partial_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let date_text = date.to_string();
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls) VALUES (?1, 'model', 'model:last-good', 'last good', 41)",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("seed last-good daily row");
    sqlx::query(
            "INSERT INTO long_term_projection_daily_backup_claims (bucket_date, rebuild_token) VALUES (?1, 'owner-one')",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("reserve first owner");
    let control = LongTermProjectionWriteControl::unrestricted();

    let error = ensure_long_term_projection_daily_backup_for_date(
        &pool,
        &date_text,
        "owner-two",
        true,
        &control,
    )
    .await
    .expect_err("competing owner cannot replace an uncommitted snapshot");
    assert!(error.to_string().contains("owner-one"));
    let public_rows = load_long_term_daily_rows(&pool, "model", None, &date_text, &date_text)
        .await
        .expect("public live rows while snapshot is reserved");
    assert_eq!(public_rows.len(), 1);
    assert_eq!(public_rows[0].series_key, "model:last-good");
    assert_eq!(public_rows[0].calls, 41);

    ensure_long_term_projection_daily_backup_for_date(
        &pool,
        &date_text,
        "owner-one",
        true,
        &control,
    )
    .await
    .expect("first owner completes a snapshot");
    let active = sqlx::query_scalar::<_, Option<String>>(
            "SELECT active_daily_backup_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
        )
        .bind(&date_text)
        .fetch_one(&pool)
        .await
        .expect("active backup owner");
    assert_eq!(active.as_deref(), Some("owner-one"));
    let error = ensure_long_term_projection_daily_backup_for_date(
        &pool,
        &date_text,
        "owner-two",
        true,
        &control,
    )
    .await
    .expect_err("competing owner cannot replace a published snapshot");
    assert!(error.to_string().contains("owner-one"));

    release_long_term_projection_daily_backups(
        &pool,
        &[(date_text.clone(), "owner-one".to_string())],
        &control,
    )
    .await
    .expect("release completed owner");
    let claims = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_projection_daily_backup_claims WHERE bucket_date = ?1",
    )
    .bind(&date_text)
    .fetch_one(&pool)
    .await
    .expect("released claim");
    assert_eq!(claims, 0);
}

#[tokio::test]
async fn initial_marker_blocks_a_stale_baseline_from_publishing_ready() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1, last_error = ?2 WHERE id = ?3")
        .bind(LONG_TERM_STATUS_RUNNING)
        .bind(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("persist incomplete initial marker");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let rebuild = LongTermProjectionDateRebuild {
        bucket_date: date.to_string(),
        start_epoch,
        end_epoch,
        hourly: HashMap::new(),
        daily: HashMap::new(),
        interval_segments: Vec::new(),
        source_row_count: 0,
    };
    let control = LongTermProjectionWriteControl::unrestricted();

    let error = commit_long_term_projection_date_rebuilds_with_control(
        &pool,
        &[rebuild],
        Some(321),
        &[],
        true,
        &control,
    )
    .await
    .expect_err("stale P2 baseline must not publish over the initial marker");
    assert!(
        error
            .to_string()
            .contains("incomplete initial materialization")
    );
    let state = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("initial state remains retryable");
    assert_eq!(state.0, LONG_TERM_STATUS_RUNNING);
    assert_eq!(
        state.1.as_deref(),
        Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
    );
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("cursor remains before rejected publication"),
        0
    );
}
