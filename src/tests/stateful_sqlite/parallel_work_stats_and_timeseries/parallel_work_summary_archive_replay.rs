use super::*;

#[tokio::test]
async fn all_time_summary_preserves_archived_history_when_rollup_failures_are_stale() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );
    let archived_pending_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(15))
            .expect("archived pending time"),
    );
    let archived_failed_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("archived failed time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-archived-stale-rollup",
        &[
            (
                1_i64,
                "summary-all-archived-success",
                archived_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "summary-all-archived-pending",
                archived_pending_at.as_str(),
                SOURCE_PROXY,
                "pending",
                10_i64,
                0.10_f64,
                Some(110.0),
            ),
            (
                3_i64,
                "summary-all-archived-failed",
                archived_failed_at.as_str(),
                SOURCE_PROXY,
                "failed",
                10_i64,
                0.10_f64,
                Some(120.0),
            ),
        ],
    )
    .await;

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("mark archived invocation batch as materialized");

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_success_at)
        .expect("bucket start epoch should be derivable");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0, ?8)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(3_i64)
    .bind(1_i64)
    .bind(2_i64)
    .bind(30_i64)
    .bind(0.30_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed stale archived invocation rollup counts");

    let live_occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(10))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id,
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            t_upstream_ttfb_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(10_i64)
    .bind("summary-all-live-success")
    .bind(&live_occurred_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10_i64)
    .bind(0.01_f64)
    .bind(130.0_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert live invocation row");

    let response = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect_err("missing live-rollup cursor must fail closed instead of omitting live history");
    assert!(matches!(response, ApiError::Unavailable(_)));

    let repair_marker_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2")
    .fetch_optional(&state.pool)
    .await
    .expect("load archived all-time summary repair marker presence");
    assert_eq!(
        repair_marker_cursor, None,
        "read-only all-time summary should not materialize summary repair markers inline"
    );
    let repair_live_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2_live_cursor")
    .fetch_optional(&state.pool)
    .await
    .expect("load archived all-time summary repair live cursor presence");
    assert_eq!(
        repair_live_cursor, None,
        "read-only all-time summary should not materialize repair live-cursor markers inline"
    );
    let invocation_rollup_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations")
    .fetch_optional(&state.pool)
    .await
    .expect("load invocation hourly rollup live cursor presence after read-only summary");
    assert_eq!(
        invocation_rollup_cursor, None,
        "read-only all-time summary should not advance the shared invocation hourly cursor inline"
    );

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let Json(summary_repeat) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("repeat all-time summary after rollup repair");

    assert_eq!(summary_repeat.total_count, 4);
    assert_eq!(summary_repeat.success_count, 2);
    assert_eq!(summary_repeat.failure_count, 1);
    assert_eq!(summary_repeat.total_tokens, 40);
    assert!((summary_repeat.total_cost - 0.31).abs() < 1e-9);

    let live_tail_occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(3))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id,
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(11_i64)
    .bind("summary-all-live-tail-failed")
    .bind(&live_tail_occurred_at)
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind(5_i64)
    .bind(0.05_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert post-repair live tail invocation row");

    let live_tail =
        sqlx::query_as::<_, ApiInvocation>("SELECT * FROM codex_invocations WHERE id = ?1")
            .bind(11_i64)
            .fetch_one(&state.pool)
            .await
            .expect("load committed post-repair live tail invocation");
    let live_tail_delta = apply_dashboard_activity_terminal_record(state.as_ref(), &live_tail)
        .await
        .terminal_delta
        .expect("register committed post-repair live tail delta");
    state
        .subscription_hub
        .acknowledge_summary_delta(live_tail_delta)
        .await;

    let Json(summary_with_live_tail) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with post-repair live tail");

    assert_eq!(summary_with_live_tail.total_count, 5);
    assert_eq!(summary_with_live_tail.success_count, 2);
    assert_eq!(summary_with_live_tail.failure_count, 2);
    assert_eq!(summary_with_live_tail.total_tokens, 45);
    assert!((summary_with_live_tail.total_cost - 0.36).abs() < 1e-9);
}

#[tokio::test]
async fn all_time_summary_includes_unmaterialized_archived_history_without_inline_repair() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(6, 0, 0)
    .expect("valid archived local hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );
    let archived_failed_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("archived failed time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-unmaterialized-archive",
        &[
            (
                1_i64,
                "summary-all-unmaterialized-success",
                archived_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "summary-all-unmaterialized-failed",
                archived_failed_at.as_str(),
                SOURCE_PROXY,
                "failed",
                10_i64,
                0.20_f64,
                Some(120.0),
            ),
        ],
    )
    .await;

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with unmaterialized archived history");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 20);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);

    let repair_marker_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2")
    .fetch_optional(&state.pool)
    .await
    .expect("load pending archive repair marker presence");
    assert_eq!(
        repair_marker_cursor, None,
        "all-time summary should not materialize summary repair markers inline"
    );
}

#[tokio::test]
async fn all_time_summary_skips_archive_fallback_rows_already_counted_in_live_tail() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(6, 0, 0)
    .expect("valid archived overlap hour");
    let first_archived_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived overlap time"),
    );
    let second_archived_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived overlap time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-live-tail-overlap",
        &[
            (
                2_i64,
                "summary-all-live-tail-overlap-success",
                first_archived_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                3_i64,
                "summary-all-live-tail-overlap-failed",
                second_archived_at.as_str(),
                SOURCE_PROXY,
                "failed",
                20_i64,
                0.20_f64,
                Some(120.0),
            ),
        ],
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            updated_at = datetime('now')
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed shared hourly rollup cursor before overlapping live tail");

    for (id, invoke_id, occurred_at, status, total_tokens, cost, ttfb_ms) in [
        (
            2_i64,
            "summary-all-live-tail-overlap-success",
            first_archived_at.as_str(),
            "success",
            10_i64,
            0.10_f64,
            100.0_f64,
        ),
        (
            3_i64,
            "summary-all-live-tail-overlap-failed",
            second_archived_at.as_str(),
            "failed",
            20_i64,
            0.20_f64,
            120.0_f64,
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                t_upstream_ttfb_ms,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(ttfb_ms)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert overlapping live tail invocation row");
    }

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with overlapping archive/live tail");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
}

#[tokio::test]
async fn error_distribution_strips_only_matching_failure_kind_prefix() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    for (id, invoke_id, error_message, failure_kind) in [
        (
            1_i64,
            "error-distribution-matching-prefix",
            "[provider] upstream stream error",
            Some("provider"),
        ),
        (
            2_i64,
            "error-distribution-mismatched-prefix",
            "[provider] upstream stream error",
            Some("different_kind"),
        ),
        (
            3_i64,
            "error-distribution-missing-failure-kind",
            "[provider] upstream stream error",
            None,
        ),
        (
            4_i64,
            "error-distribution-no-prefix",
            "upstream stream error",
            Some("provider"),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                total_tokens,
                cost,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind("failed")
        .bind(error_message)
        .bind(failure_kind)
        .bind("service_failure")
        .bind(1_i64)
        .bind(1_i64)
        .bind(0.01_f64)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert error distribution regression row");
    }

    let Json(response) = fetch_error_distribution(
        State(state),
        Query(ErrorQuery {
            range: "today".to_string(),
            top: Some(8),
            scope: Some("service".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch error distribution regression rows");

    assert_eq!(response.items.iter().map(|item| item.count).sum::<i64>(), 4);
    assert_eq!(
        response
            .items
            .iter()
            .find(|item| item.reason == "upstream stream error")
            .map(|item| item.count),
        Some(2),
    );
    assert_eq!(
        response
            .items
            .iter()
            .find(|item| item.reason == "[provider] upstream stream error")
            .map(|item| item.count),
        Some(2),
    );
}

#[tokio::test]
async fn archived_range_reads_include_unmaterialized_batches_without_inline_repair() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(9, 0, 0)
    .expect("valid archived local hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );
    let archived_failed_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(35))
            .expect("archived failed time"),
    );

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-range-unmaterialized-archive",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1_i64,
                invoke_id: "summary-range-unmaterialized-success",
                occurred_at: archived_success_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 10_i64,
                cost: 0.10_f64,
                ttfb_ms: Some(100.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
            SeedInvocationArchiveBatchRow {
                id: 2_i64,
                invoke_id: "summary-range-unmaterialized-failed",
                occurred_at: archived_failed_at.as_str(),
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 20_i64,
                cost: 0.20_f64,
                ttfb_ms: Some(120.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some("HTTP 429 too many requests"),
                failure_kind: Some("upstream_response_failed"),
                failure_class: Some("service_failure"),
                is_actionable: Some(1_i64),
            },
        ],
    )
    .await;

    let historical_range = "30d".to_string();
    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some(historical_range.clone()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch historical summary with unmaterialized archived range");
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
    assert_f64_close(
        summary
            .non_success_cost
            .expect("historical non-success cost should include archived failures"),
        0.20,
    );
    assert_eq!(summary.non_success_tokens, Some(20));
    let usage_breakdown = summary
        .usage_breakdown
        .expect("historical summary should include usage breakdown");
    assert_eq!(usage_breakdown.cache_write_tokens, 0);
    assert_eq!(usage_breakdown.cache_read_tokens, 0);
    assert_eq!(usage_breakdown.output_tokens, 0);
    let usage_costs = usage_breakdown
        .costs
        .expect("historical usage breakdown should preserve known total cost");
    assert_f64_close(usage_costs.input, 0.0);
    assert_f64_close(usage_costs.cache_write, 0.0);
    assert_f64_close(usage_costs.cache_read, 0.0);
    assert_f64_close(usage_costs.output, 0.0);
    assert_f64_close(usage_costs.reasoning, 0.0);
    assert_f64_close(usage_costs.unknown, 0.30);

    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical failure summary with unmaterialized archived range");
    assert_eq!(failure_summary.total_failures, 1);
    assert_eq!(failure_summary.service_failure_count, 1);
    assert_eq!(failure_summary.client_failure_count, 0);
    assert_eq!(failure_summary.client_abort_count, 0);
    assert_eq!(failure_summary.actionable_failure_count, 1);
    assert_f64_close(failure_summary.actionable_failure_rate, 1.0);

    let Json(error_distribution) = fetch_error_distribution(
        State(state.clone()),
        Query(ErrorQuery {
            range: historical_range,
            top: None,
            scope: Some("service".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical error distribution with unmaterialized archived range");
    assert!(
        error_distribution
            .items
            .iter()
            .any(|item| item.reason == "too_many_requests" && item.count == 1),
        "historical error distribution should include archived service failures before background repair"
    );

    let repair_marker_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2")
    .fetch_optional(&state.pool)
    .await
    .expect("load historical range repair marker presence");
    assert_eq!(
        repair_marker_cursor, None,
        "historical archived reads should not materialize summary repair markers inline"
    );
}

#[tokio::test]
async fn archived_range_reads_skip_archive_fallback_rows_already_counted_in_live_tail() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(9, 0, 0)
    .expect("valid archived range overlap hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived range overlap success time"),
    );
    let archived_failed_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(35))
            .expect("archived range overlap failed time"),
    );

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-range-live-tail-overlap",
        &[
            SeedInvocationArchiveBatchRow {
                id: 2_i64,
                invoke_id: "summary-range-live-tail-overlap-success",
                occurred_at: archived_success_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 10_i64,
                cost: 0.10_f64,
                ttfb_ms: Some(100.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0_i64),
            },
            SeedInvocationArchiveBatchRow {
                id: 3_i64,
                invoke_id: "summary-range-live-tail-overlap-failed",
                occurred_at: archived_failed_at.as_str(),
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 20_i64,
                cost: 0.20_f64,
                ttfb_ms: Some(120.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some("HTTP 429 too many requests"),
                failure_kind: Some("upstream_response_failed"),
                failure_class: Some("service_failure"),
                is_actionable: Some(1_i64),
            },
        ],
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            updated_at = datetime('now')
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed archived range shared hourly rollup cursor");

    for (
        id,
        invoke_id,
        occurred_at,
        status,
        total_tokens,
        cost,
        ttfb_ms,
        error_message,
        failure_kind,
        failure_class,
        is_actionable,
    ) in [
        (
            2_i64,
            "summary-range-live-tail-overlap-success",
            archived_success_at.as_str(),
            "success",
            10_i64,
            0.10_f64,
            100.0_f64,
            None,
            None,
            Some("none"),
            Some(0_i64),
        ),
        (
            3_i64,
            "summary-range-live-tail-overlap-failed",
            archived_failed_at.as_str(),
            "failed",
            20_i64,
            0.20_f64,
            120.0_f64,
            Some("HTTP 429 too many requests"),
            Some("upstream_response_failed"),
            Some("service_failure"),
            Some(1_i64),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                source,
                status,
                total_tokens,
                cost,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                detail_level,
                t_upstream_ttfb_ms,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(error_message)
        .bind(failure_kind)
        .bind(failure_class)
        .bind(is_actionable)
        .bind(DETAIL_LEVEL_FULL)
        .bind(ttfb_ms)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert overlapping archived range live row");
    }

    let historical_range = "30d".to_string();
    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some(historical_range.clone()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch historical summary with overlapping archive/live tail");
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
    assert_eq!(summary.non_success_tokens, Some(20));
    let usage_breakdown = summary
        .usage_breakdown
        .expect("overlapping archive/live summary should include usage breakdown");
    assert_eq!(usage_breakdown.cache_write_tokens, 0);
    assert_eq!(usage_breakdown.cache_read_tokens, 0);
    assert_eq!(usage_breakdown.output_tokens, 0);
    let usage_costs = usage_breakdown
        .costs
        .expect("overlapping archive/live summary should preserve known total cost");
    assert_f64_close(usage_costs.unknown, 0.30);

    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical failure summary with overlapping archive/live tail");
    assert_eq!(failure_summary.total_failures, 1);
    assert_eq!(failure_summary.service_failure_count, 1);
    assert_eq!(failure_summary.client_failure_count, 0);
    assert_eq!(failure_summary.client_abort_count, 0);
    assert_eq!(failure_summary.actionable_failure_count, 1);
    assert_f64_close(failure_summary.actionable_failure_rate, 1.0);

    let Json(error_distribution) = fetch_error_distribution(
        State(state),
        Query(ErrorQuery {
            range: historical_range,
            top: None,
            scope: Some("service".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical error distribution with overlapping archive/live tail");
    assert_eq!(error_distribution.items.len(), 1);
    assert_eq!(error_distribution.items[0].reason, "too_many_requests");
    assert_eq!(error_distribution.items[0].count, 1);
}

#[tokio::test]
async fn previous7d_summary_matches_daily_timeseries_when_window_spans_archived_and_live_days() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let archived_day = today - ChronoDuration::days(6);
    let live_day = today - ChronoDuration::days(1);
    let archived_at = format_naive(
        archived_day
            .and_hms_opt(10, 0, 0)
            .expect("valid archived previous7d invocation"),
    );
    let live_at = format_naive(
        live_day
            .and_hms_opt(10, 0, 0)
            .expect("valid live previous7d invocation"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "previous7d-mixed-window",
        &[(
            1_i64,
            "previous7d-archived",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            100_i64,
            1.25_f64,
            Some(90.0),
        )],
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id,
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(2_i64)
    .bind("previous7d-live")
    .bind(&live_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(200_i64)
    .bind(2.5_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert live previous7d invocation");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("previous7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch previous7d summary with mixed archive/live days");

    let Json(timeseries) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "7d".to_string(),
            bucket: Some("1d".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch 7d daily timeseries with mixed archive/live days");

    let summed_cost = timeseries
        .points
        .iter()
        .take(7)
        .map(|point| point.total_cost)
        .sum::<f64>();
    let summed_count = timeseries
        .points
        .iter()
        .take(7)
        .map(|point| point.total_count)
        .sum::<i64>();
    let summed_tokens = timeseries
        .points
        .iter()
        .take(7)
        .map(|point| point.total_tokens)
        .sum::<i64>();

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 300);
    assert_f64_close(summary.total_cost, 3.75);
    assert_eq!(summary.total_count, summed_count);
    assert_eq!(summary.total_tokens, summed_tokens);
    assert_f64_close(summary.total_cost, summed_cost);
}

#[tokio::test]
async fn all_time_summary_fallback_skips_already_materialized_archive_buckets() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_a_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(6, 0, 0)
    .expect("valid first archived local hour");
    let archived_hour_b_local = archived_hour_a_local
        .checked_add_signed(ChronoDuration::hours(1))
        .expect("second archived local hour");
    let archived_first_at = format_naive(
        archived_hour_a_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived time"),
    );
    let archived_second_at = format_naive(
        archived_hour_b_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("second archived time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-partial-materialized-archive",
        &[
            (
                1_i64,
                "summary-partial-materialized-first",
                archived_first_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "summary-partial-materialized-second",
                archived_second_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(120.0),
            ),
        ],
    )
    .await;

    let first_bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("first bucket start epoch should be derivable");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0, ?8)
        "#,
    )
    .bind(first_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed already materialized first summary bucket");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with partially materialized archive");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
}

#[tokio::test]
async fn all_time_summary_fallback_includes_missing_rows_from_partially_materialized_bucket() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(7, 0, 0)
    .expect("valid archived partial summary hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived partial summary time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived partial summary time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-partial-rowcount-archive",
        &[
            (
                1_i64,
                "summary-partial-rowcount-first",
                archived_first_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "summary-partial-rowcount-second",
                archived_second_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(120.0),
            ),
        ],
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("partial summary bucket start epoch should be derivable");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .bind("[0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed partially materialized summary rollup row");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with partially materialized archive bucket");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
}

#[tokio::test]
async fn all_time_summary_fallback_aggregates_missing_rows_across_archive_parts() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(7, 0, 0)
    .expect("valid archived multipart summary hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived multipart summary time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived multipart summary time"),
    );

    let first_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-multipart-archive-a",
        &[(
            101_i64,
            "summary-multipart-first",
            archived_first_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    let first_archive_path = state
        .config
        .archive_dir
        .join("summary-multipart-archive-a.sqlite.gz");
    let _ = fs::remove_file(&first_archive_path);
    fs::rename(&first_archive_original_path, &first_archive_path)
        .expect("move first multipart summary archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(first_archive_path.to_string_lossy().to_string())
    .bind(first_archive_original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("update first multipart summary archive batch path after move");

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-multipart-archive-b",
        &[(
            201_i64,
            "summary-multipart-second",
            archived_second_at.as_str(),
            SOURCE_PROXY,
            "success",
            20_i64,
            0.20_f64,
            Some(120.0),
        )],
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("multipart summary bucket start epoch should be derivable");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .bind("[0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed partially materialized multipart summary rollup row");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with multipart partially materialized archive");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
}

#[tokio::test]
async fn all_time_summary_fallback_keeps_unmaterialized_rows_when_sibling_archive_part_is_materialized()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(7, 0, 0)
    .expect("valid archived mixed-state summary hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived mixed-state summary time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived mixed-state summary time"),
    );

    let first_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-mixed-state-archive-a",
        &[(
            1_i64,
            "summary-mixed-state-first",
            archived_first_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    let first_archive_path = state
        .config
        .archive_dir
        .join("summary-mixed-state-archive-a.sqlite.gz");
    let _ = fs::remove_file(&first_archive_path);
    fs::rename(&first_archive_original_path, &first_archive_path)
        .expect("move first mixed-state summary archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now') WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(first_archive_path.to_string_lossy().to_string())
    .bind(first_archive_original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark first mixed-state summary archive batch as materialized");

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-mixed-state-archive-b",
        &[(
            1_i64,
            "summary-mixed-state-second",
            archived_second_at.as_str(),
            SOURCE_PROXY,
            "success",
            20_i64,
            0.20_f64,
            Some(120.0),
        )],
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("mixed-state summary bucket start epoch should be derivable");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .bind("[0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed mixed-state materialized summary rollup row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;
    insert_hourly_rollup_archive_replay_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &first_archive_path,
    )
    .await;

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with materialized sibling archive part");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
}

#[tokio::test]
async fn all_time_summary_fallback_keeps_unmaterialized_rows_when_materialized_sibling_archive_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(7, 0, 0)
    .expect("valid archived unreadable mixed-state summary hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived unreadable mixed-state summary time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived unreadable mixed-state summary time"),
    );

    let first_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-mixed-state-unreadable-archive-a",
        &[(
            1_i64,
            "summary-mixed-state-unreadable-first",
            archived_first_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    let first_archive_path = state
        .config
        .archive_dir
        .join("summary-mixed-state-unreadable-archive-a.sqlite.gz");
    let _ = fs::remove_file(&first_archive_path);
    fs::rename(&first_archive_original_path, &first_archive_path)
        .expect("move first unreadable mixed-state summary archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now') WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(first_archive_path.to_string_lossy().to_string())
    .bind(first_archive_original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark first unreadable mixed-state summary archive batch as materialized");

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-mixed-state-unreadable-archive-b",
        &[(
            1_i64,
            "summary-mixed-state-unreadable-second",
            archived_second_at.as_str(),
            SOURCE_PROXY,
            "success",
            20_i64,
            0.20_f64,
            Some(120.0),
        )],
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("unreadable mixed-state summary bucket start epoch should be derivable");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .bind("[0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed unreadable mixed-state materialized summary rollup row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;
    mark_summary_archive_replay_complete(&state.pool, &first_archive_path).await;

    fs::write(&first_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable mixed-state summary archive batch");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with unreadable materialized sibling archive part");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
}

#[tokio::test]
async fn summary_projection_rejects_clipped_rolling_boundary_for_unreadable_replayed_archive() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let boundary = Utc::now() - ChronoDuration::days(7);
    let before_boundary = format_naive(
        (boundary - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let after_boundary = format_naive(
        (boundary + ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-clipped-boundary-unreadable",
        &[
            (
                1_i64,
                "summary-clipped-boundary-before",
                before_boundary.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "summary-clipped-boundary-after",
                after_boundary.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(120.0),
            ),
        ],
    )
    .await;
    let archive_path = state
        .config
        .archive_dir
        .join("summary-clipped-boundary-unreadable.sqlite.gz");
    let _ = fs::remove_file(&archive_path);
    fs::rename(&original_path, &archive_path)
        .expect("move clipped-boundary archive to a unique path");
    sqlx::query(
        "UPDATE archive_batches \
         SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), \
             coverage_start_at = ?3, coverage_end_at = ?4 \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(original_path.to_string_lossy().to_string())
    .bind(&before_boundary)
    .bind(&after_boundary)
    .execute(&state.pool)
    .await
    .expect("mark clipped-boundary archive as materialized");

    let mut materialized_buckets = std::collections::HashSet::new();
    for (occurred_at, total_tokens, total_cost) in [
        (before_boundary.as_str(), 10_i64, 0.10_f64),
        (after_boundary.as_str(), 20_i64, 0.20_f64),
    ] {
        let bucket_start_epoch = invocation_bucket_start_epoch(occurred_at)
            .expect("clipped-boundary hour should have a durable bucket");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly \
             (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost) \
             VALUES (?1, ?2, 1, 1, 0, ?3, ?4) \
             ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET \
                 total_count = total_count + excluded.total_count, \
                 success_count = success_count + excluded.success_count, \
                 total_tokens = total_tokens + excluded.total_tokens, \
                 total_cost = total_cost + excluded.total_cost",
        )
        .bind(bucket_start_epoch)
        .bind(SOURCE_PROXY)
        .bind(total_tokens)
        .bind(total_cost)
        .execute(&state.pool)
        .await
        .expect("seed full-hour durable rollup");
        if materialized_buckets.insert(bucket_start_epoch) {
            insert_materialized_rollup_bucket_marker(
                &state.pool,
                HOURLY_ROLLUP_TARGET_INVOCATIONS,
                bucket_start_epoch,
                SOURCE_PROXY,
            )
            .await;
        }
    }
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;
    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt clipped-boundary archive after durable replay");

    let Json(all_time) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("all-time durable rollup remains exact");
    assert_eq!(all_time.total_count, 2);
    assert_eq!(all_time.total_tokens, 30);

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("a materialized unreadable boundary must publish the rollup-backed projection");
    state.pool.close().await;
    let rolling = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(rolling, Err(ApiError::Unavailable(_))),
        "a clipped rolling boundary must remain unavailable without request-time I/O: {rolling:?}"
    );
}

#[tokio::test]
async fn summary_projection_replaces_rollup_for_point_materialized_archive_without_replay() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;
    let archived_at_utc = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::hours(6)).timestamp(),
                3_600,
                0,
            ) + 15 * 60,
            0,
        )
        .single()
        .expect("align point archive fixture");
    let archived_at = format_naive(archived_at_utc.with_timezone(&Shanghai).naive_local());
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-point-materialized-without-replay",
        &[(
            9_001,
            "summary-point-materialized-without-replay",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            17,
            1.25,
            Some(120.0),
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches \
         SET coverage_start_at = ?1, coverage_end_at = ?1, historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(&archived_at)
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark point archive materialized without replay");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(crate::stats::align_bucket_epoch(
        archived_at_utc.timestamp(),
        3_600,
        0,
    ))
    .execute(&state.pool)
    .await
    .expect("seed compact point rollup");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate point materialized archive replacement");
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve point archive range without SQLite or archive I/O");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.success_count, 1);
    assert_eq!(response.total_tokens, 17);
    assert_f64_close(response.total_cost, 1.25);
}

#[tokio::test]
async fn summary_projection_keeps_rollup_ranges_when_materialized_current_archive_exceeds_admission()
 {
    with_summary_projection_test_exact_record_limit(10_000, async {
        summary_projection_keeps_rollup_ranges_when_materialized_current_archive_exceeds_admission_with_limit().await;
    })
    .await;
}

async fn summary_projection_keeps_rollup_ranges_when_materialized_current_archive_exceeds_admission_with_limit()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;
    let archive_hour = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::hours(6)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align current archive fixture");
    let archive_end = archive_hour + ChronoDuration::hours(1);
    let archive_last_row_at = archive_end - ChronoDuration::seconds(1);
    let archive_middle_row_at = archive_hour + ChronoDuration::minutes(30);
    let archive_db_path = state
        .config
        .archive_dir
        .join("summary-current-materialized-over-admission.sqlite");
    let archive_path = state
        .config
        .archive_dir
        .join("summary-current-materialized-over-admission.sqlite.gz");
    fs::create_dir_all(&state.config.archive_dir).expect("create current archive directory");
    fs::File::create(&archive_db_path).expect("create current archive SQLite source");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open current archive SQLite source");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create current archive schema");
    let archive_start = format_naive(archive_hour.with_timezone(&Shanghai).naive_local());
    let archive_last = format_naive(archive_last_row_at.with_timezone(&Shanghai).naive_local());
    let archive_middle = format_naive(archive_middle_row_at.with_timezone(&Shanghai).naive_local());
    sqlx::query(
        "WITH RECURSIVE rows(id) AS ( \
            SELECT 1 UNION ALL SELECT id + 1 FROM rows WHERE id < 10001 \
         ) \
         INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response, created_at) \
         SELECT id, 'summary-current-materialized-over-admission-' || id, \
                CASE WHEN id = 1 THEN ?1 WHEN id = 10001 THEN ?2 ELSE ?3 END, \
                'proxy', 'success', 1, 0.01, 'full', '{}', '', ?3 \
         FROM rows",
    )
    .bind(&archive_start)
    .bind(&archive_last)
    .bind(&archive_middle)
    .execute(&archive_pool)
    .await
    .expect("seed current archive over bounded candidate admission");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress current archive source");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash current archive source");
    fs::remove_file(&archive_db_path).expect("remove current archive SQLite source");
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, created_at) \
         VALUES ('codex_invocations', ?1, ?2, ?3, 10001, 'completed', ?4, ?5, datetime('now'), datetime('now'))",
    )
    .bind(archive_start[..7].to_string())
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .bind(&archive_start)
    .bind(&archive_last)
    .execute(&state.pool)
    .await
    .expect("insert materialized current archive manifest");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 10001, 10001, 0, 10001, 100.01, 0)",
    )
    .bind(archive_hour.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed complete materialized current summary rollup");
    sqlx::query(
        "INSERT INTO upstream_account_usage_breakdown_hourly \
         (bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort, \
          request_count, output_tokens, cost_unknown, has_cost) \
         VALUES (?1, 'proxy', '-1', '', '', 10001, 10001, 100.01, 1)",
    )
    .bind(archive_hour.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed complete materialized current usage rollup");
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("current candidate admission must not abort rollup-backed projection hydration");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile the exact all-time rollup-backed projection");
    state.pool.close().await;

    for window in ["1d", "all"] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("serve {window} from the rollup-backed memory projection");
        assert_eq!(response.total_count, 10_001, "{window} count");
        assert_eq!(response.total_tokens, 10_001, "{window} tokens");
        assert_f64_close(response.total_cost, 100.01);
    }
    assert!(matches!(
        fetch_summary(
            State(state),
            Query(SummaryQuery {
                window: Some("current".to_string()),
                limit: Some(1),
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            }),
        )
        .await,
        Err(ApiError::Unavailable(_))
    ));
}

#[tokio::test]
async fn summary_projection_pruned_compact_current_candidate_keeps_rolling_range_exact() {
    let state =
        test_state_with_openai_base(Url::parse("https://api.openai.com/").expect("valid test URL"))
            .await;
    let now = Utc::now();
    let live_at = db_occurred_at_lower_bound(now - ChronoDuration::minutes(1));
    sqlx::query(
        "WITH RECURSIVE rows(value) AS ( \
            SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < 50000 \
         ) \
         INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         SELECT 'summary-compact-current-live-' || value, ?1, 'proxy', 'success', 1, 0.1, '{}', '', 'full' \
         FROM rows",
    )
    .bind(&live_at)
    .execute(&state.pool)
    .await
    .expect("seed bounded current resident prefix");

    let archive_hour = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (now - ChronoDuration::hours(3)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align compact archive hour");
    let archive_at = archive_hour + ChronoDuration::minutes(15);
    let archive_occurred_at = format_naive(archive_at.with_timezone(&Shanghai).naive_local());
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-compact-current-prune",
        &[(
            60_001,
            "summary-compact-current-pruned",
            archive_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            17,
            1.25,
            None,
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches \
         SET coverage_start_at = ?1, coverage_end_at = ?2, \
             historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?3",
    )
    .bind(db_occurred_at_lower_bound(archive_at))
    .bind(db_occurred_at_lower_bound(
        archive_at + ChronoDuration::seconds(1),
    ))
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark exact compact archive coverage");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(archive_hour.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed compact archive totals");
    sqlx::query(
        "INSERT INTO upstream_account_usage_breakdown_hourly \
         (bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort, \
          request_count, output_tokens, cost_unknown, has_cost) \
         VALUES (?1, 'proxy', '-1', '', '', 1, 17, 1.25, 1)",
    )
    .bind(archive_hour.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed compact archive usage");
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate the compact-proof current candidate");
    state.pool.close().await;

    let Json(rolling) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve compact-rollup-proven rolling range without SQLite");
    assert_eq!(rolling.total_count, 50_001);
    assert_eq!(rolling.total_tokens, 50_017);
    assert_f64_close(rolling.total_cost, 5_001.25);

    let Json(current) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("current cutoff remains exact after compact candidate pruning");
    assert_eq!(current.total_count, 1);
}

#[tokio::test]
async fn all_time_summary_skips_double_count_for_readable_materialized_archive_when_same_bucket_sibling_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(7, 0, 0)
    .expect("valid same-bucket summary hour");
    let archived_readable_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("same-bucket readable summary archived time"),
    );
    let archived_unreadable_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("same-bucket unreadable summary archived time"),
    );

    let readable_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-same-bucket-unreadable-sibling-materialized",
        &[(
            1_i64,
            "summary-same-bucket-unreadable-sibling-materialized",
            archived_readable_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    let readable_archive_path = state
        .config
        .archive_dir
        .join("summary-same-bucket-unreadable-sibling-materialized.sqlite.gz");
    let _ = fs::remove_file(&readable_archive_path);
    fs::rename(&readable_archive_original_path, &readable_archive_path)
        .expect("move readable same-bucket summary archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(readable_archive_path.to_string_lossy().to_string())
    .bind(readable_archive_original_path.to_string_lossy().to_string())
    .bind(archived_readable_at.as_str())
    .bind(archived_readable_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark readable same-bucket summary archive batch as materialized");

    let unreadable_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-same-bucket-unreadable-sibling-broken",
        &[(
            1_i64,
            "summary-same-bucket-unreadable-sibling-broken",
            archived_unreadable_at.as_str(),
            SOURCE_PROXY,
            "success",
            20_i64,
            0.20_f64,
            Some(120.0),
        )],
    )
    .await;
    let unreadable_archive_path = state
        .config
        .archive_dir
        .join("summary-same-bucket-unreadable-sibling-broken.sqlite.gz");
    let _ = fs::remove_file(&unreadable_archive_path);
    fs::rename(&unreadable_archive_original_path, &unreadable_archive_path)
        .expect("move unreadable same-bucket summary archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(unreadable_archive_path.to_string_lossy().to_string())
    .bind(unreadable_archive_original_path.to_string_lossy().to_string())
    .bind(archived_unreadable_at.as_str())
    .bind(archived_unreadable_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark unreadable same-bucket summary archive batch as materialized");

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_readable_at)
        .expect("same-bucket summary bucket start epoch should be derivable");
    let mut histogram = empty_approx_histogram();
    add_approx_histogram_sample(&mut histogram, 100.0);
    add_approx_histogram_sample(&mut histogram, 120.0);
    let encoded_histogram =
        encode_approx_histogram(&histogram).expect("encode same-bucket summary histogram");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(2_i64)
    .bind(2_i64)
    .bind(0_i64)
    .bind(30_i64)
    .bind(0.30_f64)
    .bind(2_i64)
    .bind(220.0_f64)
    .bind(120.0_f64)
    .bind(encoded_histogram)
    .execute(&state.pool)
    .await
    .expect("seed same-bucket summary rollup row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;
    // The shared bucket marker proves that a compact row exists. Each immutable batch also
    // needs complete replay markers for totals, account stats, and usage before the all-time
    // projection can prove the row includes both sibling archives without reopening the
    // unreadable one.
    for archive_path in [&readable_archive_path, &unreadable_archive_path] {
        mark_summary_archive_replay_complete(&state.pool, archive_path).await;
    }

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-bucket summary archive");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with unreadable same-bucket materialized sibling archive");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let end = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("same-bucket summary timeseries end"),
        Shanghai,
    );
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
    .expect("fetch same-bucket historical timeseries with unreadable materialized sibling archive");

    let archived_point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("same-bucket summary timeseries bucket should exist");
    assert_eq!(archived_point.total_count, 2);
    assert_eq!(archived_point.success_count, 2);
    assert_eq!(archived_point.failure_count, 0);
    assert_eq!(archived_point.total_tokens, 30);
    assert_f64_close(archived_point.total_cost, 0.30);
}

#[tokio::test]
async fn all_time_summary_skips_double_count_for_readable_materialized_archive_when_same_month_sibling_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_materialized_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(7, 0, 0)
    .expect("valid archived readable materialized summary hour");
    let archived_unreadable_hour_local = archived_materialized_hour_local
        .checked_add_signed(ChronoDuration::hours(2))
        .expect("same-month unreadable summary sibling hour");
    let archived_materialized_at = format_naive(
        archived_materialized_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("readable materialized summary archived time"),
    );
    let archived_unreadable_at = format_naive(
        archived_unreadable_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("unreadable summary sibling archived time"),
    );

    let materialized_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-same-month-unreadable-sibling-materialized",
        &[(
            1_i64,
            "summary-same-month-unreadable-sibling-materialized",
            archived_materialized_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    let materialized_archive_path = state
        .config
        .archive_dir
        .join("summary-same-month-unreadable-sibling-materialized.sqlite.gz");
    let _ = fs::remove_file(&materialized_archive_path);
    fs::rename(
        &materialized_archive_original_path,
        &materialized_archive_path,
    )
    .expect("move readable materialized summary archive batch to a unique same-month path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(materialized_archive_path.to_string_lossy().to_string())
    .bind(materialized_archive_original_path.to_string_lossy().to_string())
    .bind(archived_materialized_at.as_str())
    .bind(archived_materialized_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark readable materialized same-month summary archive batch as materialized");

    let unreadable_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-same-month-unreadable-sibling-broken",
        &[(
            1_i64,
            "summary-same-month-unreadable-sibling-broken",
            archived_unreadable_at.as_str(),
            SOURCE_PROXY,
            "success",
            20_i64,
            0.20_f64,
            Some(120.0),
        )],
    )
    .await;
    let unreadable_archive_path = state
        .config
        .archive_dir
        .join("summary-same-month-unreadable-sibling-broken.sqlite.gz");
    let _ = fs::remove_file(&unreadable_archive_path);
    fs::rename(&unreadable_archive_original_path, &unreadable_archive_path)
        .expect("move unreadable summary sibling archive batch to a unique same-month path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(unreadable_archive_path.to_string_lossy().to_string())
    .bind(unreadable_archive_original_path.to_string_lossy().to_string())
    .bind(archived_unreadable_at.as_str())
    .bind(archived_unreadable_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark unreadable same-month summary sibling archive batch as materialized");

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_materialized_at)
        .expect("same-month summary bucket start epoch should be derivable");
    let unreadable_bucket_start_epoch = invocation_bucket_start_epoch(&archived_unreadable_at)
        .expect("same-month unreadable summary bucket start epoch should be derivable");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch,
            source,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .bind("[0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed readable materialized summary rollup row");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost) \
         VALUES (?1, ?2, 1, 1, 0, 20, 0.20)",
    )
    .bind(unreadable_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .execute(&state.pool)
    .await
    .expect("seed unreadable materialized same-month summary rollup row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        unreadable_bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;
    for archive_path in [&materialized_archive_path, &unreadable_archive_path] {
        mark_summary_archive_replay_complete(&state.pool, archive_path).await;
    }

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-month summary sibling archive");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch all-time summary with unreadable same-month materialized sibling archive");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);

    let start = local_naive_to_utc(archived_materialized_hour_local, Shanghai);
    let end = local_naive_to_utc(
        archived_materialized_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("same-month summary timeseries end"),
        Shanghai,
    );
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
    .expect("fetch historical timeseries with unreadable same-month materialized sibling archive");

    let archived_point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("same-month summary timeseries bucket should exist");
    assert_eq!(archived_point.total_count, 1);
    assert_eq!(archived_point.success_count, 1);
    assert_eq!(archived_point.failure_count, 0);
    assert_eq!(archived_point.total_tokens, 10);
    assert_f64_close(archived_point.total_cost, 0.10);
}
