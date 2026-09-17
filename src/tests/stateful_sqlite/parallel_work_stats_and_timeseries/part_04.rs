struct FailureArchivePairFixture<'a> {
    batch_name: &'a str,
    success_id: i64,
    success_invoke_id: &'a str,
    failed_id: i64,
    failed_invoke_id: &'a str,
    local_hour: u32,
    failed_minute: i64,
}

async fn seed_failure_archive_pair(
    state: &Arc<AppState>,
    fixture: FailureArchivePairFixture<'_>,
) -> (String, String) {
    let archived_hour = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(fixture.local_hour, 0, 0)
    .expect("valid archived fixture hour");
    let success_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("valid archived success time"),
    );
    let failed_at = format_naive(
        archived_hour
            .checked_add_signed(ChronoDuration::minutes(fixture.failed_minute))
            .expect("valid archived failure time"),
    );
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        fixture.batch_name,
        &[
            SeedInvocationArchiveBatchRow {
                id: fixture.success_id,
                invoke_id: fixture.success_invoke_id,
                occurred_at: &success_at,
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 10,
                cost: 0.10,
                ttfb_ms: Some(100.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: fixture.failed_id,
                invoke_id: fixture.failed_invoke_id,
                occurred_at: &failed_at,
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 20,
                cost: 0.20,
                ttfb_ms: Some(120.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some("HTTP 429 too many requests"),
                failure_kind: Some("upstream_response_failed"),
                failure_class: Some("service_failure"),
                is_actionable: Some(1),
            },
        ],
    )
    .await;
    (success_at, failed_at)
}

async fn set_invocation_rollup_live_cursor(pool: &SqlitePool, cursor_id: i64) {
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
    .bind(cursor_id)
    .execute(pool)
    .await
    .expect("seed invocation hourly rollup cursor");
}

struct FailurePairLiveTailFixture<'a> {
    success_at: &'a str,
    failed_at: &'a str,
    success_id: i64,
    success_invoke_id: &'a str,
    failed_id: i64,
    failed_invoke_id: &'a str,
    include_failure_metadata: bool,
}

async fn insert_failure_pair_live_tail(pool: &SqlitePool, fixture: FailurePairLiveTailFixture<'_>) {
    for (id, invoke_id, occurred_at, status, tokens, cost, ttfb) in [
        (
            fixture.success_id,
            fixture.success_invoke_id,
            fixture.success_at,
            "success",
            10,
            0.10,
            100.0,
        ),
        (
            fixture.failed_id,
            fixture.failed_invoke_id,
            fixture.failed_at,
            "failed",
            20,
            0.20,
            120.0,
        ),
    ] {
        let has_failure = fixture.include_failure_metadata && status == "failed";
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost,
                error_message, failure_kind, failure_class, is_actionable, detail_level,
                t_upstream_ttfb_ms, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, '{}')
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(tokens)
        .bind(cost)
        .bind(has_failure.then_some("HTTP 429 too many requests"))
        .bind(has_failure.then_some("upstream_response_failed"))
        .bind(has_failure.then_some("service_failure"))
        .bind(Some(if has_failure { 1_i64 } else { 0_i64 }))
        .bind(DETAIL_LEVEL_FULL)
        .bind(ttfb)
        .execute(pool)
        .await
        .expect("insert archived failure pair live-tail row");
    }
}

async fn assert_historical_failure_views(state: Arc<AppState>, exact_error_items: bool) {
    let summary = fetch_test_summary(state.clone(), "30d").await;
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 30);
    assert_f64_close(summary.total_cost, 0.30);
    assert_f64_close(summary.non_success_cost.expect("non-success cost"), 0.20);
    assert_eq!(summary.non_success_tokens, Some(20));
    let usage = summary.usage_breakdown.expect("historical usage breakdown");
    assert_eq!(usage.cache_write_tokens, 0);
    assert_eq!(usage.cache_read_tokens, 0);
    assert_eq!(usage.output_tokens, 0);
    let costs = usage.costs.expect("historical usage costs");
    assert_f64_close(costs.input, 0.0);
    assert_f64_close(costs.cache_write, 0.0);
    assert_f64_close(costs.cache_read, 0.0);
    assert_f64_close(costs.output, 0.0);
    assert_f64_close(costs.reasoning, 0.0);
    assert_f64_close(costs.unknown, 0.30);

    let failure = fetch_test_failure_summary(state.clone(), "30d").await;
    assert_eq!(failure.total_failures, 1);
    assert_eq!(failure.service_failure_count, 1);
    assert_eq!(failure.client_failure_count, 0);
    assert_eq!(failure.client_abort_count, 0);
    assert_eq!(failure.actionable_failure_count, 1);
    assert_f64_close(failure.actionable_failure_rate, 1.0);

    let Json(errors) = fetch_error_distribution(
        State(state),
        Query(ErrorQuery {
            range: "30d".to_string(),
            top: None,
            scope: Some("service".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical error distribution");
    if exact_error_items {
        assert_eq!(errors.items.len(), 1);
        assert_eq!(errors.items[0].reason, "too_many_requests");
        assert_eq!(errors.items[0].count, 1);
    } else {
        assert!(
            errors
                .items
                .iter()
                .any(|item| item.reason == "too_many_requests" && item.count == 1)
        );
    }
}

#[tokio::test]
pub(crate) async fn all_time_summary_includes_unmaterialized_archived_history_without_inline_repair()
 {
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
pub(crate) async fn all_time_summary_skips_archive_fallback_rows_already_counted_in_live_tail() {
    let state = archive_retention_test_state().await;

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

    set_invocation_rollup_live_cursor(&state.pool, 1).await;
    insert_failure_pair_live_tail(
        &state.pool,
        FailurePairLiveTailFixture {
            success_at: &first_archived_at,
            failed_at: &second_archived_at,
            success_id: 2,
            success_invoke_id: "summary-all-live-tail-overlap-success",
            failed_id: 3,
            failed_invoke_id: "summary-all-live-tail-overlap-failed",
            include_failure_metadata: false,
        },
    )
    .await;
    let summary = fetch_test_summary(state, "all").await;

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
}

#[tokio::test]
pub(crate) async fn archived_range_reads_include_unmaterialized_batches_without_inline_repair() {
    let state = archive_retention_test_state().await;
    seed_failure_archive_pair(
        &state,
        FailureArchivePairFixture {
            batch_name: "summary-range-unmaterialized-archive",
            success_id: 1,
            success_invoke_id: "summary-range-unmaterialized-success",
            failed_id: 2,
            failed_invoke_id: "summary-range-unmaterialized-failed",
            local_hour: 9,
            failed_minute: 35,
        },
    )
    .await;
    assert_historical_failure_views(state.clone(), false).await;

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
pub(crate) async fn archived_range_reads_skip_archive_fallback_rows_already_counted_in_live_tail() {
    let state = archive_retention_test_state().await;
    let (archived_success_at, archived_failed_at) = seed_failure_archive_pair(
        &state,
        FailureArchivePairFixture {
            batch_name: "summary-range-live-tail-overlap",
            success_id: 2,
            success_invoke_id: "summary-range-live-tail-overlap-success",
            failed_id: 3,
            failed_invoke_id: "summary-range-live-tail-overlap-failed",
            local_hour: 9,
            failed_minute: 35,
        },
    )
    .await;

    set_invocation_rollup_live_cursor(&state.pool, 1).await;
    insert_failure_pair_live_tail(
        &state.pool,
        FailurePairLiveTailFixture {
            success_at: &archived_success_at,
            failed_at: &archived_failed_at,
            success_id: 2,
            success_invoke_id: "summary-range-live-tail-overlap-success",
            failed_id: 3,
            failed_invoke_id: "summary-range-live-tail-overlap-failed",
            include_failure_metadata: true,
        },
    )
    .await;
    assert_historical_failure_views(state, true).await;
}

#[tokio::test]
pub(crate) async fn previous7d_summary_matches_daily_timeseries_when_window_spans_archived_and_live_days()
 {
    let state = archive_retention_test_state().await;

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

    let summary = fetch_test_summary(state.clone(), "previous7d").await;
    let timeseries = fetch_test_timeseries(state, "7d", "1d").await;

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
pub(crate) async fn all_time_summary_fallback_skips_already_materialized_archive_buckets() {
    let state = archive_retention_test_state().await;

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

    let summary = fetch_test_summary(state, "all").await;

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
}

#[tokio::test]
pub(crate) async fn all_time_summary_fallback_includes_missing_rows_from_partially_materialized_bucket()
 {
    let state = archive_retention_test_state().await;

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

    let summary = fetch_test_summary(state, "all").await;

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);
}

use super::*;
