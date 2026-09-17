async fn load_hour_non_success_tokens(state: &AppState, bucket_epoch: i64) -> i64 {
    load_non_success_tokens_snapshot(
        state,
        InvocationSourceScope::ProxyOnly,
        None,
        ExactUtcRange {
            start: Utc
                .timestamp_opt(bucket_epoch, 0)
                .single()
                .expect("non-success range start"),
            end: Utc
                .timestamp_opt(bucket_epoch + 3_600, 0)
                .single()
                .expect("non-success range end"),
        },
        SummaryRangeBuildTelemetry::new(
            SummaryBuildRoute::Http,
            &SummaryWindow::Duration(ChronoDuration::days(1)),
            Some("1d"),
        ),
    )
    .await
    .expect("load non-success tokens")
    .expect("non-success tokens should be available")
}

async fn seed_indexed_archive_coverage_fixture(state: &AppState, occurred_at: DateTime<Utc>) {
    let month = occurred_at
        .with_timezone(&Shanghai)
        .format("%Y-%m")
        .to_string();
    let coverage_start = format_naive(occurred_at.with_timezone(&Shanghai).naive_local());
    let coverage_end = format_naive(
        (occurred_at + ChronoDuration::seconds(3_599))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        "WITH RECURSIVE hundreds(value) AS (VALUES(0) UNION ALL SELECT value + 1 FROM hundreds WHERE value < 199), \
         units(value) AS (VALUES(0) UNION ALL SELECT value + 1 FROM units WHERE value < 99) \
         INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at) \
         SELECT 'codex_invocations', ?1, '/tmp/v2-priority-coverage-' || (hundreds.value * 100 + units.value) || '.sqlite.gz', \
                'fixture-sha', 1, 'completed', ?2, ?3 FROM hundreds CROSS JOIN units",
    )
    .bind(&month)
    .bind(&coverage_start)
    .bind(&coverage_end)
    .execute(&state.pool)
    .await
    .expect("seed archive coverage fixture");
    sqlx::query(
        "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at) \
         VALUES ('forward_proxy_attempts', ?1, '/tmp/v2-priority-other-dataset.sqlite.gz', 'other-sha', 1, 'completed', ?2, ?3), \
                ('codex_invocations', ?1, '/tmp/v2-priority-pending.sqlite.gz', 'pending-sha', 1, 'pending', ?2, ?3)",
    )
    .bind(month)
    .bind(coverage_start)
    .bind(coverage_end)
    .execute(&state.pool)
    .await
    .expect("seed archive coverage filter controls");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'codex_invocations' AND status = 'completed' \
         AND coverage_start_epoch IS NOT NULL AND coverage_end_epoch IS NOT NULL",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count matching archive coverage fixture rows");
    assert_eq!(count, 20_000);
}

async fn seed_dashboard_summary_rollups(state: &AppState, bucket_start_epoch: i64) {
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch, source, total_count, success_count, failure_count,
            total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms,
            first_byte_max_ms, first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(250_i64)
    .bind(0.25_f64)
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .bind("[0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed dashboard summary materialized rollup row");
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch, source, upstream_account_id, total_count,
            success_count, failure_count, total_tokens, input_tokens, output_tokens,
            cache_input_tokens, total_cost, non_success_cost, total_latency_sample_count,
            total_latency_sum_ms, first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms, first_response_byte_total_max_ms
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(42_i64)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(250_i64)
    .bind(200_i64)
    .bind(50_i64)
    .bind(25_i64)
    .bind(0.25_f64)
    .bind(0.0_f64)
    .bind(1_i64)
    .bind(500.0_f64)
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .execute(&state.pool)
    .await
    .expect("seed dashboard account materialized rollup row");
}

async fn seed_dashboard_summary_missing_archive_account(state: &AppState) {
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(42_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Recovered Rollup")
    .bind("Primary")
    .bind("enterprise")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert dashboard activity recovered rollup account");
}

async fn seed_dashboard_summary_missing_archive_data(state: &AppState) -> String {
    let archived_hour_local = Utc::now()
        .with_timezone(&Shanghai)
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .expect("valid local noon")
        .checked_sub_signed(ChronoDuration::days(1))
        .expect("valid archived dashboard summary hour");
    let archived_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("valid archived dashboard summary invocation time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "dashboard-summary-only-missing-materialized-archive",
        &[(
            95_001_i64,
            "dashboard-summary-only-missing-materialized",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            250_i64,
            0.25_f64,
            Some(100.0),
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?2, coverage_end_at = ?3 WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(format_naive(archived_hour_local))
    .bind(format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("valid dashboard summary archive coverage end"),
    ))
    .execute(&state.pool)
    .await
    .expect("mark dashboard summary archive as materialized");
    fs::remove_file(&archive_path).expect("remove materialized archive raw file");
    archived_at
}

async fn seed_partial_hour_dashboard_account(state: &AppState) {
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        "INSERT INTO pool_upstream_accounts (id, kind, provider, display_name, group_name, plan_type, status, enabled, created_at, updated_at) VALUES (42, 'api_key_codex', 'codex', 'Partial Hour Recovery', 'Primary', 'enterprise', 'active', 1, ?1, ?1)",
    )
    .bind(created_at)
    .execute(&state.pool)
    .await
    .expect("insert partial-hour recovery account");
}

async fn seed_partial_hour_dashboard_live_edges(
    state: &AppState,
    full_hour_start: DateTime<Utc>,
    full_hour_end: DateTime<Utc>,
    after_full_hour_secs: i64,
) {
    let before_edge_at = full_hour_start - ChronoDuration::minutes(5);
    let after_edge_at = full_hour_end + ChronoDuration::seconds((after_full_hour_secs / 2).max(1));
    for (id, invoke_id, occurred_at) in [
        (
            96_001_i64,
            "dashboard-partial-hour-before-edge",
            before_edge_at,
        ),
        (
            96_002_i64,
            "dashboard-partial-hour-after-edge",
            after_edge_at,
        ),
    ] {
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response) VALUES (?1, ?2, ?3, 'proxy', 'success', 20, 0.02, ?4, '{}')",
        )
        .bind(id)
        .bind(invoke_id)
        .bind(format_naive(occurred_at.with_timezone(&Shanghai).naive_local()))
        .bind(json!({
            "promptCacheKey": format!("pck-{invoke_id}"),
            "upstreamAccountId": 42_i64,
        }).to_string())
        .execute(&state.pool)
        .await
        .expect("insert partial-hour live edge invocation");
    }
}

async fn seed_partial_hour_dashboard_archive(
    state: &AppState,
    full_hour_start: DateTime<Utc>,
    skipped_start: DateTime<Utc>,
    skipped_end: DateTime<Utc>,
) {
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "dashboard-partial-hour-materialized-gap",
        &[(
            96_100_i64,
            "dashboard-partial-hour-materialized-placeholder",
            format_naive(
                (full_hour_start + ChronoDuration::minutes(5))
                    .with_timezone(&Shanghai)
                    .naive_local(),
            )
            .as_str(),
            SOURCE_PROXY,
            "success",
            200_i64,
            0.20_f64,
            Some(100.0_f64),
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?2, coverage_end_at = ?3 WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(format_naive(skipped_start.with_timezone(&Shanghai).naive_local()))
    .bind(format_naive(skipped_end.with_timezone(&Shanghai).naive_local()))
    .execute(&state.pool)
    .await
    .expect("mark partial-hour archive as materialized");
    fs::remove_file(&archive_path).expect("remove partial-hour materialized archive raw file");
}

async fn seed_partial_hour_dashboard_rollups(state: &AppState, bucket_epoch: i64) {
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms, first_byte_max_ms, first_byte_histogram) VALUES (?1, 'proxy', 2, 2, 0, 400, 0.40, 2, 200.0, 100.0, '[0,0,0,0,0,0,0,2,0,0,0,0,0,0,0,0,0,0,0,0,0]')",
    )
    .bind(bucket_epoch)
    .execute(&state.pool)
    .await
    .expect("seed partial-hour dashboard summary rollup row");
    sqlx::query(
        "INSERT INTO upstream_account_stats_hourly (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, total_tokens, input_tokens, output_tokens, cache_input_tokens, total_cost, non_success_cost, total_latency_sample_count, total_latency_sum_ms, first_response_byte_total_sample_count, first_response_byte_total_sum_ms, first_response_byte_total_max_ms) VALUES (?1, 'proxy', 42, 2, 2, 0, 400, 320, 80, 40, 0.40, 0.0, 2, 1000.0, 2, 200.0, 100.0)",
    )
    .bind(bucket_epoch)
    .execute(&state.pool)
    .await
    .expect("seed partial-hour dashboard account rollup row");
}

#[tokio::test]
pub(crate) async fn account_activity_v2_priority_selection_skips_interrupted_sqlite_round() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, payload, raw_response
        )
        VALUES ('v2-interrupted-selection', ?1, ?2, 'success', 1, ?3, '{}')
        "#,
    )
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert selection interruption fixture");

    let progress_probe = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started_at = std::time::Instant::now();
    let outcome = crate::select_active_account_activity_v2_priority_buckets_with_deadline(
        &state.pool,
        current_hour_epoch,
        started_at,
        started_at + std::time::Duration::from_secs(1),
        Some(progress_probe.clone()),
        1,
        true,
    )
    .await
    .expect("handle interrupted selection round");
    assert!(outcome.is_none());
    assert!(progress_probe.load(std::sync::atomic::Ordering::Relaxed));
}

#[tokio::test]
pub(crate) async fn account_activity_v2_priority_repair_preserves_unknown_archive_coverage() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let repair_hour_epoch = current_hour_epoch - 2 * 3_600;
    let repair_time = Utc
        .timestamp_opt(repair_hour_epoch + 10 * 60, 0)
        .single()
        .expect("repair invocation time");
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, 1, datetime('now'))",
    )
    .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET)
    .execute(&state.pool)
    .await
    .expect("seed current account activity v2 repair generation");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES ('v2-unknown-archive-live', ?1, ?2, 'success', ?3, 10, 0.10, ?4, '{}')
        "#,
    )
    .bind(format_naive(
        repair_time.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_FULL)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert live row in unknown archive month");
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch, source, upstream_account_id,
            activity_v2_request_count, activity_v2_total_tokens
        )
        VALUES (?1, ?2, 42, 9, 900)
        "#,
    )
    .bind(repair_hour_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("insert archive-derived values with unknown coverage");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status, created_at
        )
        VALUES (?1, ?2, ?3, 'unknown-coverage-sha', 1, ?4, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(
        repair_time
            .with_timezone(&Shanghai)
            .format("%Y-%m")
            .to_string(),
    )
    .bind("/tmp/account-activity-v2-unknown-coverage.sqlite.gz")
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&state.pool)
    .await
    .expect("insert archive manifest without coverage bounds");

    repair_active_account_activity_v2_coverage(&state.pool)
        .await
        .expect("run active coverage repair");

    let rollup = sqlx::query_as::<_, (i64, i64)>(
        "SELECT activity_v2_request_count, activity_v2_total_tokens \
         FROM upstream_account_stats_hourly \
         WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42",
    )
    .bind(repair_hour_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load preserved unknown-coverage rollup");
    assert_eq!(rollup, (9, 900));
    let covered = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM hourly_rollup_materialized_buckets \
         WHERE target = ?1 AND bucket_start_epoch = ?2 AND source = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(repair_hour_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .fetch_one(&state.pool)
    .await
    .expect("load unknown-coverage marker");
    assert_eq!(covered, 0);
}

#[tokio::test]
pub(crate) async fn account_activity_v2_priority_repair_uses_indexed_archive_epoch_coverage_within_budget()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let oldest_hour_epoch = current_hour_epoch - 7 * 24 * 3_600;
    let oldest_occurred_at = Utc
        .timestamp_opt(oldest_hour_epoch + 60, 0)
        .single()
        .expect("oldest active window timestamp");

    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, 1, datetime('now'))",
    )
    .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET)
    .execute(&state.pool)
    .await
    .expect("seed current account activity v2 repair generation");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, detail_level,
            total_tokens, cost, payload, raw_response
        )
        VALUES ('v2-priority-window-floor', ?1, ?2, 'success', ?3, 1, 0.01, ?4, '{}')
        "#,
    )
    .bind(format_naive(
        oldest_occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(DETAIL_LEVEL_FULL)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert active-window floor invocation");
    seed_indexed_archive_coverage_fixture(&state, oldest_occurred_at).await;

    let mut explain_query = build_active_account_activity_v2_archive_epoch_coverage_query(
        "EXPLAIN QUERY PLAN ",
        oldest_hour_epoch,
        current_hour_epoch,
    );
    assert!(explain_query.sql().contains("coverage_start_epoch <"));
    let explain = explain_query
        .build_query_as::<(i64, i64, i64, String)>()
        .fetch_all(&state.pool)
        .await
        .expect("explain indexed archive epoch coverage query");
    let explain_details = explain
        .iter()
        .map(|(_, _, _, detail)| detail.as_str())
        .collect::<Vec<_>>();
    assert!(
        explain_details.iter().any(|detail| {
            detail.contains(
                "SEARCH archive_batches USING INDEX idx_archive_batches_invocation_coverage_epoch",
            ) && detail.contains("coverage_end_epoch")
        }),
        "unexpected archive epoch coverage plan: {explain_details:?}"
    );

    let selection_started_at = std::time::Instant::now();
    let selected = crate::select_active_account_activity_v2_priority_buckets_with_deadline(
        &state.pool,
        current_hour_epoch,
        selection_started_at,
        selection_started_at + Duration::from_secs(2),
        None,
        1_000,
        false,
    )
    .await
    .expect("select priority coverage against archive fixture")
    .expect("priority selection should finish within budget");
    assert_eq!(selected.len(), 2);
    assert!(
        selection_started_at.elapsed() < Duration::from_secs(2),
        "priority coverage selection exceeded its two-second budget: {:?}",
        selection_started_at.elapsed()
    );

    let outcome = repair_active_account_activity_v2_coverage(&state.pool)
        .await
        .expect("repair active coverage against archive fixture");
    assert_eq!(outcome.priority_bucket_count, 2);
    assert_eq!(outcome.repaired_bucket_count, 2);
}

#[tokio::test]
pub(crate) async fn summary_non_success_tokens_includes_unreplayed_live_tail_in_covered_hour() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let bucket_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0) - 2 * 3_600;
    let occurred_at = Utc
        .timestamp_opt(bucket_epoch + 10 * 60, 0)
        .single()
        .expect("covered invocation time");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, payload, raw_response
        )
        VALUES ('unreplayed-non-success', ?1, ?2, 'failed', 7, ?3, '{}')
        "#,
    )
    .bind(format_naive(
        occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert unreplayed invocation");
    sqlx::query(
        r#"
        INSERT INTO upstream_account_stats_hourly (
            bucket_start_epoch, source, upstream_account_id,
            activity_v2_non_success_tokens
        )
        VALUES (?1, ?2, 42, 3)
        "#,
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("insert covered v2 non-success rollup");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target, bucket_start_epoch, source, materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(bucket_epoch)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .execute(&state.pool)
    .await
    .expect("mark covered v2 hour");

    let value = load_hour_non_success_tokens(state.as_ref(), bucket_epoch).await;

    assert_eq!(value, 10);

    sqlx::query(
        r#"
        UPDATE upstream_account_stats_hourly
        SET activity_v2_non_success_tokens = 7
        WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42
        "#,
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("update authoritative v2 non-success rollup");
    sqlx::query(
        r#"
        INSERT INTO account_activity_v2_bucket_repair_watermarks (
            bucket_start_epoch, cursor_id, updated_at
        )
        VALUES (?1, 1, datetime('now'))
        ON CONFLICT(bucket_start_epoch) DO UPDATE SET cursor_id = excluded.cursor_id
        "#,
    )
    .bind(bucket_epoch)
    .execute(&state.pool)
    .await
    .expect("mark non-success row included by bucket recompute");

    let recomputed_value = load_hour_non_success_tokens(state.as_ref(), bucket_epoch).await;
    assert_eq!(recomputed_value, 7);
}

async fn seed_v2_rollup_rows(state: &AppState, bucket_epoch: i64) {
    for (offset_minutes, invoke_id, status, tokens, cost, ttfb, total_ms, failure_class) in [
        (
            5_i64,
            "v2-success-old",
            "success",
            30_i64,
            0.30_f64,
            10.0_f64,
            100.0_f64,
            "none",
        ),
        (
            25,
            "v2-success-latest",
            "success",
            40,
            0.40,
            25.0,
            250.0,
            "none",
        ),
        (
            35,
            "v2-failure",
            "failed",
            50,
            0.50,
            90.0,
            900.0,
            "service_failure",
        ),
        (45, "v2-running", "running", 60, 0.60, 95.0, 950.0, "none"),
    ] {
        let occurred_at = Utc
            .timestamp_opt(bucket_epoch + offset_minutes * 60, 0)
            .single()
            .expect("v2 invocation timestamp");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, detail_level, total_tokens, cost, cache_input_tokens, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_total_ms, failure_class, payload, raw_response) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 5, 0, 0, 0, ?8, ?9, ?10, ?11, '{}')",
        )
        .bind(invoke_id)
        .bind(format_naive(occurred_at.with_timezone(&Shanghai).naive_local()))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(DETAIL_LEVEL_FULL)
        .bind(tokens)
        .bind(cost)
        .bind(ttfb)
        .bind(total_ms)
        .bind(failure_class)
        .bind(r#"{"upstreamAccountId":42}"#)
        .execute(&state.pool)
        .await
        .expect("insert v2 rollup source row");
    }
}

async fn assert_v2_rollup_latest_values(state: &AppState, bucket_epoch: i64) {
    let stored = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, f64, String, String, f64, String, f64)>(
        "SELECT activity_v2_request_count, activity_v2_success_count, activity_v2_failure_count, activity_v2_non_success_count, activity_v2_total_tokens, activity_v2_failure_tokens, activity_v2_failure_cost, activity_v2_latest_unkeyed_conversation_at, activity_v2_latest_first_response_at, activity_v2_latest_first_response_ms, activity_v2_latest_total_latency_at, activity_v2_latest_total_latency_ms FROM upstream_account_stats_hourly WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = 42",
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load stored v2 account activity");
    assert_eq!(
        (stored.0, stored.1, stored.2, stored.3, stored.4, stored.5),
        (3, 2, 1, 1, 120, 50)
    );
    assert_f64_close(stored.6, 0.50);
    let latest_success_at = format_naive(
        Utc.timestamp_opt(bucket_epoch + 25 * 60, 0)
            .single()
            .expect("latest success timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let latest_unkeyed_at = format_naive(
        Utc.timestamp_opt(bucket_epoch + 35 * 60, 0)
            .single()
            .expect("latest unkeyed terminal timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    assert_eq!(stored.7, latest_unkeyed_at);
    assert_eq!(stored.8, latest_success_at);
    assert_eq!(stored.9, 25.0);
    assert_eq!(stored.10, latest_success_at);
    assert_eq!(stored.11, 250.0);
}

#[tokio::test]
pub(crate) async fn account_activity_v2_rollup_keeps_latest_timestamp_and_value_paired() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let bucket_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0) - 3_600;
    seed_v2_rollup_rows(state.as_ref(), bucket_epoch).await;

    let mut tx = state.pool.begin().await.expect("begin v2 rollup tx");
    let rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx.as_mut(), &[bucket_epoch])
        .await
        .expect("load v2 source rows");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &rows,
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
    )
    .await
    .expect("upsert v2 account activity");
    tx.commit().await.expect("commit v2 account activity");

    assert_v2_rollup_latest_values(state.as_ref(), bucket_epoch).await;
}

#[tokio::test]
pub(crate) async fn dashboard_activity_summary_only_uses_rollups_when_materialized_archive_file_is_missing()
 {
    let mut config = test_config();
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    seed_dashboard_summary_missing_archive_account(&state).await;
    let archived_at = seed_dashboard_summary_missing_archive_data(&state).await;
    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_at)
        .expect("dashboard summary rollup bucket start should be derivable");
    seed_dashboard_summary_rollups(&state, bucket_start_epoch).await;

    let Json(response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: false,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch summary-only dashboard activity from materialized rollup");

    assert_eq!(response.summary.stats.total_count, 1);
    assert_eq!(response.summary.stats.success_count, 1);
    assert_eq!(response.summary.stats.failure_count, 0);
    assert_eq!(response.summary.stats.total_tokens, 250);
    assert_f64_close(response.summary.stats.total_cost, 0.25);
    assert!(response.summary.stats.usage_breakdown.is_none());
    assert!(response.summary.stats.non_success_tokens.is_none());

    let Json(full_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch full dashboard activity from materialized rollup");

    assert_eq!(full_response.summary.stats.total_count, 1);
    assert_eq!(full_response.summary.stats.success_count, 1);
    assert_eq!(full_response.summary.stats.failure_count, 0);
    assert_eq!(full_response.summary.stats.total_tokens, 250);
    assert_f64_close(full_response.summary.stats.total_cost, 0.25);
    assert!(full_response.summary.stats.usage_breakdown.is_none());
    assert!(full_response.summary.stats.non_success_tokens.is_none());
    let full_accounts = full_response
        .accounts
        .as_ref()
        .expect("full dashboard accounts should be included");
    let recovered_account = full_accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("full dashboard should include recovered rollup account");
    assert_eq!(recovered_account.display_name, "Recovered Rollup");
    assert_eq!(recovered_account.request_count, 1);
    assert_eq!(recovered_account.success_count, 1);
    assert_eq!(recovered_account.total_tokens, 250);
    assert_f64_close(recovered_account.total_cost, 0.25);
    assert_eq!(
        full_accounts
            .iter()
            .map(|account| account.request_count)
            .sum::<i64>(),
        full_response.summary.stats.total_count,
    );

    let Json(activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity from materialized rollup");
    let upstream_account = activity
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == 42)
        .expect("upstream activity should include recovered rollup account");
    assert_eq!(upstream_account.request_count, 1);
    assert_eq!(upstream_account.success_count, 1);
    assert_eq!(upstream_account.total_tokens, 250);
    assert_f64_close(upstream_account.total_cost, 0.25);
}

#[tokio::test]
pub(crate) async fn dashboard_activity_rollup_fallback_preserves_account_attribution_for_partial_hour_ranges()
 {
    let mut config = test_config();
    config.invocation_max_days = 1;
    let state = test_state_from_config(config, true).await;
    seed_partial_hour_dashboard_account(&state).await;

    let range_window =
        resolve_range_window("7d", Shanghai).expect("7d dashboard range should resolve");
    let full_hour_end_epoch = align_bucket_epoch(range_window.end.timestamp(), 3_600, 0);
    let full_hour_start_epoch = full_hour_end_epoch - 3_600;
    let full_hour_start = Utc
        .timestamp_opt(full_hour_start_epoch, 0)
        .single()
        .expect("valid full-hour start");
    let full_hour_end = Utc
        .timestamp_opt(full_hour_end_epoch, 0)
        .single()
        .expect("valid full-hour end");
    let skipped_start = full_hour_start - ChronoDuration::minutes(10);
    let after_full_hour_secs = (range_window.end.timestamp() - full_hour_end_epoch).max(2);
    let skipped_end = full_hour_end + ChronoDuration::seconds(after_full_hour_secs.min(600));
    assert!(skipped_start >= range_window.start);
    assert!(skipped_end <= range_window.end);

    seed_partial_hour_dashboard_live_edges(
        &state,
        full_hour_start,
        full_hour_end,
        after_full_hour_secs,
    )
    .await;

    seed_partial_hour_dashboard_archive(&state, full_hour_start, skipped_start, skipped_end).await;
    seed_partial_hour_dashboard_rollups(&state, full_hour_start_epoch).await;

    let Json(response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch dashboard activity with partial-hour rollup fallback");

    assert_eq!(response.summary.stats.total_count, 4);
    assert_eq!(response.summary.stats.success_count, 4);
    assert_eq!(response.summary.stats.total_tokens, 440);
    assert_f64_close(response.summary.stats.total_cost, 0.44);

    let accounts = response
        .accounts
        .expect("dashboard activity should include accounts");
    let recovered_account = accounts
        .iter()
        .find(|account| account.upstream_account_id == Some(42))
        .expect("partial-hour fallback should remain on the source account");
    assert_eq!(recovered_account.request_count, 4);
    assert_eq!(recovered_account.success_count, 4);
    assert_eq!(recovered_account.total_tokens, 440);
    assert_f64_close(recovered_account.total_cost, 0.44);
    assert!(
        accounts
            .iter()
            .filter(|account| account.is_unassigned)
            .all(|account| account.request_count == 0 && account.total_tokens == 0)
    );

    let Json(activity) = fetch_upstream_account_activity(
        State(state.clone()),
        Query(UpstreamAccountActivityQuery {
            range: "7d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch upstream account activity with partial-hour rollup fallback");
    let upstream_account = activity
        .accounts
        .iter()
        .find(|account| account.upstream_account_id == 42)
        .expect("upstream activity should keep the recovered account attribution");
    assert_eq!(upstream_account.request_count, 4);
    assert_eq!(upstream_account.success_count, 4);
    assert_eq!(upstream_account.total_tokens, 440);
    assert_f64_close(upstream_account.total_cost, 0.44);
}

use super::*;
