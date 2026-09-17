async fn seed_existing_non_materialized_summary_archive(state: &Arc<AppState>) -> (PathBuf, i64) {
    let local_hour = archived_local_hour(440, 8);
    let success_at = local_minute(local_hour, 5);
    let failed_at = local_minute(local_hour, 15);
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-nonmaterialized-existing-mixed",
        &[
            (
                1,
                "summary-all-nonmaterialized-existing-success",
                success_at.as_str(),
                SOURCE_PROXY,
                "success",
                20,
                0.20,
                Some(200.0),
            ),
            (
                2,
                "summary-all-nonmaterialized-existing-failed",
                failed_at.as_str(),
                SOURCE_PROXY,
                "failed",
                30,
                0.30,
                Some(300.0),
            ),
        ],
    )
    .await;
    let bucket_start_epoch =
        invocation_bucket_start_epoch(&success_at).expect("derive existing bucket epoch");
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch,
            total_count: 2,
            success_count: 2,
            failure_count: 0,
            total_tokens: 50,
            total_cost: 0.50,
            first_byte_samples: &[],
            context: "seed stale non-materialized rollup",
        },
    )
    .await;
    (archive_path, bucket_start_epoch)
}

#[tokio::test]
pub(crate) async fn all_time_summary_repair_rebuilds_non_materialized_archives_when_others_are_pruned()
 {
    let state = archive_retention_test_state().await;
    seed_pruned_materialized_summary_archive(&state).await;
    let (existing_archive_path, existing_bucket_start_epoch) =
        seed_existing_non_materialized_summary_archive(&state).await;

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let summary = fetch_test_summary(state.clone(), "all").await;
    assert_summary_totals(&summary, 3, 2, 1, 60, 0.60);
    assert_eq!(
        summary_replay_target_count(&state.pool, &existing_archive_path).await,
        2
    );
    assert_eq!(
        rollup_total_count(&state.pool, Some(existing_bucket_start_epoch)).await,
        2
    );
    assert_eq!(
        invocation_rollup_failure_count(&state.pool, existing_bucket_start_epoch).await,
        1
    );
}

#[tokio::test]
pub(crate) async fn all_time_summary_repair_does_not_advance_shared_live_cursor_without_hourly_sync()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(430))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-repair-no-shared-cursor",
        &[(
            1_i64,
            "summary-all-repair-archived-success",
            archived_success_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;

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
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(10_i64)
    .bind("summary-all-repair-live-success")
    .bind(&live_occurred_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10_i64)
    .bind(0.01_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert live invocation row");

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let totals = query_combined_totals(&state.pool, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("query all-time totals through summary repair path");

    assert_eq!(totals.total_count, 2);
    assert_eq!(totals.success_count, 2);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 20);
    assert!((totals.total_cost - 0.11).abs() < 1e-9);

    let shared_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations")
    .fetch_optional(&state.pool)
    .await
    .expect("load shared invocation hourly rollup cursor after summary repair");
    assert_eq!(
        shared_cursor, None,
        "summary repair must not advance the shared codex_invocations live cursor outside hourly sync",
    );

    let repair_live_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind("codex_invocations_summary_rollup_v2_live_cursor")
            .fetch_one(&state.pool)
            .await
            .expect("load summary repair live cursor");
    assert_eq!(repair_live_cursor, 10);
}

#[tokio::test]
pub(crate) async fn summary_rollup_repair_refreshes_stale_repair_live_cursor_from_shared_progress()
{
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(430))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-repair-stale-live-cursor",
        &[(
            1_i64,
            "summary-repair-stale-live-cursor-archived-success",
            archived_success_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
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
    .bind(25_i64)
    .execute(&state.pool)
    .await
    .expect("seed shared invocation hourly rollup cursor");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            updated_at = datetime('now')
        "#,
    )
    .bind("codex_invocations_summary_rollup_v2")
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed summary repair completion marker");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            updated_at = datetime('now')
        "#,
    )
    .bind("codex_invocations_summary_rollup_v2_live_cursor")
    .bind(10_i64)
    .execute(&state.pool)
    .await
    .expect("seed stale summary repair live cursor");

    crate::stats::ensure_invocation_summary_rollups_ready(&state.pool)
        .await
        .expect("refresh stale summary repair live cursor");

    let repair_live_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind("codex_invocations_summary_rollup_v2_live_cursor")
            .fetch_one(&state.pool)
            .await
            .expect("load repaired summary live cursor");
    assert_eq!(repair_live_cursor, 25);
}

#[tokio::test]
pub(crate) async fn all_time_summary_rollup_repair_counts_mixed_case_success_status() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(400))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-mixed-case-success",
        &[(
            1_i64,
            "summary-all-mixed-case-success",
            archived_success_at.as_str(),
            SOURCE_PROXY,
            "Success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;

    run_background_invocation_summary_rollup_repair(&state.pool).await;

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
    .expect("fetch all-time summary with mixed-case archived success");

    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
}

#[tokio::test]
pub(crate) async fn timeseries_ignores_non_positive_or_missing_ttfb_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(10))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-ignore-null",
        &occurred_at,
        "success",
        None,
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-ignore-zero",
        &occurred_at,
        "success",
        Some(0.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-ignore-negative",
        &occurred_at,
        "success",
        Some(-5.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-ignore-failed",
        &occurred_at,
        "failed",
        Some(250.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-keep-valid",
        &occurred_at,
        "success",
        Some(250.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 5)
        .expect("should include populated bucket");

    assert_eq!(bucket.first_byte_sample_count, 1);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        250.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms.expect("p95 should be present"),
        250.0,
    );
}

#[tokio::test]
pub(crate) async fn timeseries_daily_bucket_includes_first_byte_stats() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    // Use "now" to avoid crossing local-day boundaries around midnight.
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-daily-1",
        &occurred_at,
        "success",
        Some(50.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-daily-2",
        &occurred_at,
        "success",
        Some(150.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-daily-failed",
        &occurred_at,
        "failed",
        Some(300.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1d".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 3)
        .expect("should include populated bucket");

    assert_eq!(bucket.first_byte_sample_count, 2);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        100.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms.expect("p95 should be present"),
        145.0,
    );
}

#[tokio::test]
pub(crate) async fn open_live_timeseries_range_end_stays_at_query_cutoff() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let mut before = Utc::now();
    if before.timestamp().rem_euclid(60) >= 58 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        before = Utc::now();
    }

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch open live timeseries");
    let after = Utc::now();
    let response_end =
        parse_to_utc_datetime(&response.range_end).expect("response range_end should parse");

    assert!(
        response_end <= after,
        "open-window range_end should not exceed wall-clock query cutoff: response_end={response_end:?}, after={after:?}"
    );
    assert!(
        response_end >= before - ChronoDuration::seconds(2),
        "open-window range_end should stay near the live query cutoff: response_end={response_end:?}, before={before:?}"
    );
}

#[tokio::test]
pub(crate) async fn timeseries_includes_first_response_byte_total_avg_and_p95_for_complete_stage_samples()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "first-response-byte-total-1",
            occurred_at: &occurred_at,
            status: "success",
            t_req_read_ms: Some(100.0),
            t_req_parse_ms: Some(200.0),
            t_upstream_connect_ms: Some(300.0),
            t_upstream_ttfb_ms: Some(400.0),
        },
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "first-response-byte-total-2",
            occurred_at: &occurred_at,
            status: "success",
            t_req_read_ms: Some(200.0),
            t_req_parse_ms: Some(300.0),
            t_upstream_connect_ms: Some(400.0),
            t_upstream_ttfb_ms: Some(1_100.0),
        },
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "first-response-byte-total-3",
            occurred_at: &occurred_at,
            status: "success",
            t_req_read_ms: Some(500.0),
            t_req_parse_ms: Some(500.0),
            t_upstream_connect_ms: Some(1_000.0),
            t_upstream_ttfb_ms: Some(2_000.0),
        },
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 3)
        .expect("should include populated bucket");

    assert_eq!(bucket.first_response_byte_total_sample_count, 3);
    assert_f64_close(
        bucket
            .first_response_byte_total_avg_ms
            .expect("first response byte total avg should be present"),
        (1_000.0 + 2_000.0 + 4_000.0) / 3.0,
    );
    assert_f64_close(
        bucket
            .first_response_byte_total_p95_ms
            .expect("first response byte total p95 should be present"),
        3_800.0,
    );
}

#[tokio::test]
pub(crate) async fn timeseries_ignores_incomplete_first_response_byte_total_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(10))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "first-response-byte-total-valid",
            occurred_at: &occurred_at,
            status: "success",
            t_req_read_ms: Some(250.0),
            t_req_parse_ms: Some(250.0),
            t_upstream_connect_ms: Some(250.0),
            t_upstream_ttfb_ms: Some(250.0),
        },
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "first-response-byte-total-missing-read",
            occurred_at: &occurred_at,
            status: "success",
            t_req_read_ms: None,
            t_req_parse_ms: Some(250.0),
            t_upstream_connect_ms: Some(250.0),
            t_upstream_ttfb_ms: Some(250.0),
        },
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "first-response-byte-total-missing-connect",
            occurred_at: &occurred_at,
            status: "success",
            t_req_read_ms: Some(250.0),
            t_req_parse_ms: Some(250.0),
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: Some(250.0),
        },
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "first-response-byte-total-negative-parse",
            occurred_at: &occurred_at,
            status: "success",
            t_req_read_ms: Some(250.0),
            t_req_parse_ms: Some(-1.0),
            t_upstream_connect_ms: Some(250.0),
            t_upstream_ttfb_ms: Some(250.0),
        },
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 4)
        .expect("should include populated bucket");

    assert_eq!(bucket.first_response_byte_total_sample_count, 1);
    assert_f64_close(
        bucket
            .first_response_byte_total_avg_ms
            .expect("first response byte total avg should be present"),
        1_000.0,
    );
    assert_f64_close(
        bucket
            .first_response_byte_total_p95_ms
            .expect("first response byte total p95 should be present"),
        1_000.0,
    );
}

#[tokio::test]
pub(crate) async fn timeseries_includes_failed_first_response_byte_total_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(8))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "first-response-byte-total-failed",
            occurred_at: &occurred_at,
            status: "failed",
            t_req_read_ms: Some(190.0),
            t_req_parse_ms: Some(200.0),
            t_upstream_connect_ms: Some(43_400.0),
            t_upstream_ttfb_ms: Some(100.0),
        },
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 1)
        .expect("should include populated bucket");

    assert_eq!(bucket.failure_count, 1);
    assert_eq!(bucket.first_response_byte_total_sample_count, 1);
    assert_f64_close(
        bucket
            .first_response_byte_total_avg_ms
            .expect("first response byte total avg should be present"),
        43_890.0,
    );
    assert_f64_close(
        bucket
            .first_response_byte_total_p95_ms
            .expect("first response byte total p95 should be present"),
        43_890.0,
    );
}

#[tokio::test]
pub(crate) async fn timeseries_excludes_zero_ttfb_sentinel_from_first_response_byte_total_samples()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(7))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "first-response-byte-total-zero-ttfb",
            occurred_at: &occurred_at,
            status: "failed",
            t_req_read_ms: Some(190.0),
            t_req_parse_ms: Some(200.0),
            t_upstream_connect_ms: Some(43_400.0),
            t_upstream_ttfb_ms: Some(0.0),
        },
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 1)
        .expect("should include populated bucket");

    assert_eq!(bucket.failure_count, 1);
    assert_eq!(bucket.first_response_byte_total_sample_count, 0);
    assert!(bucket.first_response_byte_total_avg_ms.is_none());
    assert!(bucket.first_response_byte_total_p95_ms.is_none());
}

use super::*;
