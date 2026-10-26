use super::*;

#[tokio::test]
async fn timeseries_ignores_non_positive_or_missing_ttfb_samples() {
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
async fn timeseries_daily_bucket_includes_first_byte_stats() {
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
async fn open_live_timeseries_range_end_stays_at_query_cutoff() {
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
async fn timeseries_includes_first_response_byte_total_avg_and_p95_for_complete_stage_samples() {
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
        "first-response-byte-total-1",
        &occurred_at,
        "success",
        Some(100.0),
        Some(200.0),
        Some(300.0),
        Some(400.0),
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        "first-response-byte-total-2",
        &occurred_at,
        "success",
        Some(200.0),
        Some(300.0),
        Some(400.0),
        Some(1_100.0),
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        "first-response-byte-total-3",
        &occurred_at,
        "success",
        Some(500.0),
        Some(500.0),
        Some(1_000.0),
        Some(2_000.0),
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
async fn timeseries_ignores_incomplete_first_response_byte_total_samples() {
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
        "first-response-byte-total-valid",
        &occurred_at,
        "success",
        Some(250.0),
        Some(250.0),
        Some(250.0),
        Some(250.0),
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        "first-response-byte-total-missing-read",
        &occurred_at,
        "success",
        None,
        Some(250.0),
        Some(250.0),
        Some(250.0),
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        "first-response-byte-total-missing-connect",
        &occurred_at,
        "success",
        Some(250.0),
        Some(250.0),
        None,
        Some(250.0),
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        "first-response-byte-total-negative-parse",
        &occurred_at,
        "success",
        Some(250.0),
        Some(-1.0),
        Some(250.0),
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
async fn timeseries_includes_failed_first_response_byte_total_samples() {
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
        "first-response-byte-total-failed",
        &occurred_at,
        "failed",
        Some(190.0),
        Some(200.0),
        Some(43_400.0),
        Some(100.0),
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
async fn timeseries_excludes_zero_ttfb_sentinel_from_first_response_byte_total_samples() {
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
        "first-response-byte-total-zero-ttfb",
        &occurred_at,
        "failed",
        Some(190.0),
        Some(200.0),
        Some(43_400.0),
        Some(0.0),
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

#[tokio::test]
async fn timeseries_ttft_uses_first_token_samples_without_ttfb_fallback() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(6))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    for (invoke_id, status, ttfb_ms, first_token_ms) in [
        ("ttft-success", "success", 80.0, Some(800.0)),
        ("ttft-failed-after-token", "failed", 90.0, Some(1_200.0)),
        ("ttft-no-token", "failed", 9_999.0, None),
    ] {
        insert_timeseries_invocation(&state.pool, invoke_id, &occurred_at, status, Some(ttfb_ms))
            .await;
        sqlx::query("UPDATE codex_invocations SET first_token_ms = ?1 WHERE invoke_id = ?2")
            .bind(first_token_ms)
            .bind(invoke_id)
            .execute(&state.pool)
            .await
            .expect("update invocation TTFT");
    }

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
    .expect("fetch TTFT timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 3)
        .expect("should include populated bucket");

    assert_eq!(bucket.failure_count, 2);
    assert_eq!(bucket.first_token_sample_count, 2);
    assert_f64_close(
        bucket
            .first_token_avg_ms
            .expect("TTFT avg should be present"),
        1_000.0,
    );
    assert_f64_close(
        bucket
            .first_token_p95_ms
            .expect("TTFT p95 should be present"),
        1_180.0,
    );
}

#[test]
fn bucket_aggregate_uses_histogram_for_mixed_rollup_and_exact_p95() {
    let mut bucket = BucketAggregate {
        first_byte_sample_count: 1,
        first_byte_ttfb_sum_ms: 1_000.0,
        first_byte_histogram: empty_approx_histogram(),
        ..Default::default()
    };
    add_approx_histogram_sample(&mut bucket.first_byte_histogram, 1_000.0);

    bucket.record_exact_ttfb_sample(Some("success"), Some(100.0));

    assert_eq!(bucket.first_byte_sample_count, 2);
    assert_f64_close(
        bucket.first_byte_avg_ms().expect("avg should be present"),
        550.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms().expect("p95 should be present"),
        1_000.0,
    );
}

#[tokio::test]
async fn timeseries_daily_includes_archived_rollup_days_without_ttfb() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(40);
    insert_invocation_rollup(&state.pool, archived_date, SOURCE_PROXY, 7, 5, 2, 700, 1.75).await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "90d".to_string(),
            bucket: Some("1d".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch daily timeseries");

    let bucket = response
        .points
        .iter()
        .find(|point| shanghai_bucket_date(&point.bucket_start) == archived_date)
        .expect("should include archived rollup day");

    assert_eq!(bucket.total_count, 7);
    assert_eq!(bucket.success_count, 5);
    assert_eq!(bucket.failure_count, 2);
    assert_eq!(bucket.total_tokens, 700);
    assert_f64_close(bucket.total_cost, 1.75);
    assert_eq!(bucket.first_byte_sample_count, 0);
    assert!(bucket.first_byte_avg_ms.is_none());
    assert!(bucket.first_byte_p95_ms.is_none());
}

#[tokio::test]
async fn timeseries_daily_stays_continuous_after_rollup_archive() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(12);
    let live_date = Utc::now().with_timezone(&Shanghai).date_naive();
    let live_occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    insert_invocation_rollup_with_latency_samples(
        &state.pool,
        archived_date,
        SOURCE_PROXY,
        3,
        2,
        1,
        300,
        3.0,
        &[],
        &[30_000.0],
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        "timeseries-live-after-rollup",
        &live_occurred_at,
        "success",
        Some(5_000.0),
        Some(10_000.0),
        Some(14_500.0),
        Some(500.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: "90d".to_string(),
            bucket: Some("1d".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch daily timeseries");

    let archived_bucket = response
        .points
        .iter()
        .find(|point| shanghai_bucket_date(&point.bucket_start) == archived_date)
        .expect("should include archived bucket");
    let live_bucket = response
        .points
        .iter()
        .find(|point| shanghai_bucket_date(&point.bucket_start) == live_date)
        .expect("should include live bucket");

    assert_eq!(archived_bucket.total_count, 3);
    assert_eq!(live_bucket.total_count, 1);
    assert_eq!(live_bucket.first_byte_sample_count, 1);
    assert_eq!(archived_bucket.first_response_byte_total_sample_count, 1);
    assert_f64_close(
        archived_bucket
            .first_response_byte_total_avg_ms
            .expect("archived first response byte total avg should be present"),
        30_000.0,
    );
    assert_f64_close(
        archived_bucket
            .first_response_byte_total_p95_ms
            .expect("archived first response byte total p95 should be present"),
        30_000.0,
    );
    assert_eq!(live_bucket.first_response_byte_total_sample_count, 1);
    assert_f64_close(
        live_bucket
            .first_response_byte_total_avg_ms
            .expect("live first response byte total avg should be present"),
        30_000.0,
    );
    assert_f64_close(
        live_bucket
            .first_response_byte_total_p95_ms
            .expect("live first response byte total p95 should be present"),
        30_000.0,
    );

    let summed_count: i64 = response.points.iter().map(|point| point.total_count).sum();
    let summed_tokens: i64 = response.points.iter().map(|point| point.total_tokens).sum();
    let summed_cost: f64 = response.points.iter().map(|point| point.total_cost).sum();
    assert_eq!(summed_count, 4);
    assert_eq!(summed_tokens, 310);
    assert_f64_close(summed_cost, 3.01);
}

#[tokio::test]
async fn timeseries_daily_combines_rollup_and_live_within_same_day() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mixed_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(8);
    let live_occurred_at = format_naive(
        mixed_date
            .and_hms_opt(18, 0, 0)
            .expect("valid mixed live time"),
    );

    insert_invocation_rollup(&state.pool, mixed_date, SOURCE_PROXY, 2, 1, 1, 20, 0.2).await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-mixed-rollup-live",
        &live_occurred_at,
        "success",
        Some(150.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "90d".to_string(),
            bucket: Some("1d".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch mixed daily timeseries");

    let bucket = response
        .points
        .iter()
        .find(|point| shanghai_bucket_date(&point.bucket_start) == mixed_date)
        .expect("should include mixed bucket");

    assert_eq!(bucket.total_count, 3);
    assert_eq!(bucket.success_count, 2);
    assert_eq!(bucket.failure_count, 1);
    assert_eq!(bucket.total_tokens, 30);
    assert_f64_close(bucket.total_cost, 0.21);
    assert_eq!(bucket.first_byte_sample_count, 1);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        150.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms.expect("p95 should be present"),
        150.0,
    );
}

#[tokio::test]
async fn timeseries_hourly_rollups_rebucket_for_different_timezones() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(30);
    insert_invocation_rollup(&state.pool, archived_date, SOURCE_PROXY, 9, 7, 2, 900, 2.25).await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "90d".to_string(),
            bucket: Some("1d".to_string()),
            settlement_hour: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch utc daily timeseries");

    let expected_utc_date = local_naive_to_utc(
        archived_date
            .and_hms_opt(0, 0, 0)
            .expect("archived_date midnight should be valid"),
        Shanghai,
    )
    .date_naive();
    let bucket = response
        .points
        .iter()
        .find(|point| {
            DateTime::parse_from_rfc3339(&point.bucket_start)
                .expect("valid utc bucket start")
                .with_timezone(&Utc)
                .date_naive()
                == expected_utc_date
        })
        .expect("should rebucket archived hourly rollup into utc day");
    assert_eq!(bucket.total_count, 9);
    assert_eq!(bucket.success_count, 7);
    assert_eq!(bucket.failure_count, 2);
    assert_eq!(bucket.total_tokens, 900);
    assert_f64_close(bucket.total_cost, 2.25);
}

#[tokio::test]
async fn timeseries_hourly_recent_non_hour_aligned_timezones_fall_back_to_raw_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-kathmandu-recent",
        &occurred_at,
        "success",
        Some(180.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Kathmandu".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("recent non-hour-aligned timezone should use raw rows");

    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.total_count)
            .sum::<i64>(),
        1
    );
}

#[tokio::test]
async fn timeseries_hourly_backed_ignores_missing_exact_archive_batch() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    let temp_dir = make_temp_test_dir("timeseries-missing-exact-archive");
    let missing_archive = temp_dir.join("missing-codex-invocations.sqlite.gz");
    let month_key = Utc::now()
        .with_timezone(&Shanghai)
        .format("%Y-%m")
        .to_string();

    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&month_key)
    .bind(missing_archive.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&state.pool)
    .await
    .expect("insert missing exact-range archive manifest");

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "48h".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("missing exact-range archive batch should not fail timeseries");
    assert!(response.points.iter().all(|point| point.total_count == 0));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_yesterday_ignores_missing_non_overlapping_archive_batch() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    let temp_dir = make_temp_test_dir("summary-yesterday-missing-non-overlap-archive");
    let missing_archive = temp_dir.join("missing-codex-invocations.sqlite.gz");

    let old_day_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(40))
    .and_hms_opt(8, 0, 0)
    .expect("valid old local day");
    let old_month_key = old_day_local.format("%Y-%m").to_string();
    let coverage_start_at = format_naive(old_day_local);
    let coverage_end_at = format_naive(
        old_day_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("valid archive coverage end"),
    );

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
    .bind(&old_month_key)
    .bind(missing_archive.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_start_at)
    .bind(&coverage_end_at)
    .execute(&state.pool)
    .await
    .expect("insert missing non-overlapping archive manifest");

    let yesterday_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(1))
    .and_hms_opt(9, 0, 0)
    .expect("valid yesterday local day");
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
    .bind("yesterday-account-summary")
    .bind(format_naive(yesterday_local))
    .bind(SOURCE_PROXY)
    .bind(33_i64)
    .bind(0.33_f64)
    .bind("success")
    .bind(r#"{"upstreamAccountId":2890}"#)
    .bind(r#"{"range":"yesterday"}"#)
    .execute(&state.pool)
    .await
    .expect("insert yesterday invocation");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("yesterday".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(2890),
        }),
    )
    .await
    .expect("fetch yesterday account summary");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 33);
    assert_f64_close(summary.total_cost, 0.33);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn account_summary_yesterday_ignores_materialized_archive_missing_account_usage_marker() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;
    let account_id = 2890_i64;

    let yesterday_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(1))
    .and_hms_opt(9, 0, 0)
    .expect("valid yesterday local day");
    let occurred_at = format_naive(
        yesterday_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "account-summary-materialized-missing-usage-marker",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "account-summary-materialized-missing-usage-marker",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 33_i64,
            cost: 0.33_f64,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":2890}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0_i64),
        }],
    )
    .await;

    materialize_historical_rollups(&state.pool, &state.config, false)
        .await
        .expect("materialize historical rollups");

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target = ?2
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .execute(&state.pool)
    .await
    .expect("drop upstream account usage replay marker");

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt materialized archive after rollups exist");

    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("yesterday".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(account_id),
        }),
    )
    .await
    .expect("fetch yesterday account summary from materialized rollups");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 33);
    assert_f64_close(summary.total_cost, 0.33);
}

#[tokio::test]
async fn timeseries_daily_backed_ignores_pruned_legacy_archive_batch_files() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_day_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local day");
    let archived_occurred_at = format_naive(
        archived_day_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "timeseries-daily-pruned-legacy-archive",
        &[(
            1_i64,
            "timeseries-daily-pruned-legacy-archive",
            archived_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            12_i64,
            0.12_f64,
            Some(120.0),
        )],
    )
    .await;

    materialize_historical_rollups(&state.pool, &state.config, false)
        .await
        .expect("materialize legacy historical rollups");
    refresh_long_term_stats(&state.pool, 400)
        .await
        .expect("materialize long-term historical rollups");
    let (archive_batch_id, archive_sha256, coverage_start, coverage_end):
        (i64, String, String, String) = sqlx::query_as(
            "SELECT id, sha256, coverage_start_at, coverage_end_at FROM archive_batches WHERE file_path = ?1",
        )
        .bind(archive_path.to_string_lossy().to_string())
        .fetch_one(&state.pool)
        .await
        .expect("load archive identity for V2 cleanup proof");
    super::insert_summary_archive_snapshot_proof(
        &state.pool,
        archive_batch_id,
        &archive_sha256,
        &coverage_start,
        &coverage_end,
        1,
    )
    .await;
    prune_legacy_archive_batches(&state.pool, &state.config, false)
        .await
        .expect("prune legacy archive files after materialization");
    assert!(
        !archive_path.exists(),
        "legacy archive file should be removed after prune"
    );

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "30d".to_string(),
            bucket: Some("1d".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("daily timeseries should read materialized hourly history");
    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.total_count)
            .sum::<i64>(),
        1
    );
    let expected_date = archived_day_local.date();
    let point = response
        .points
        .iter()
        .find(|point| shanghai_bucket_date(&point.bucket_start) == expected_date)
        .expect("archived day should remain queryable after prune");
    assert_eq!(point.total_count, 1);
    assert_eq!(point.success_count, 1);
    assert_eq!(point.failure_count, 0);
    assert_eq!(point.total_tokens, 12);
    assert_f64_close(point.total_cost, 0.12);
}

#[tokio::test]
async fn timeseries_hourly_historical_non_hour_aligned_timezones_are_rejected() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;

    let err = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "48h".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Kathmandu".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect_err("historical non-hour-aligned timezone should be rejected");

    match err {
        ApiError::BadRequest(err) => {
            assert!(
                err.to_string().contains("whole-hour UTC offsets"),
                "unexpected error message: {err}"
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn account_timeseries_hourly_historical_non_hour_aligned_timezones_are_rejected() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;

    let err = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "48h".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Kathmandu".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect_err("account historical non-hour-aligned timezone should be rejected");

    match err {
        ApiError::BadRequest(err) => {
            assert!(
                err.to_string().contains("whole-hour UTC offsets"),
                "unexpected error message: {err}"
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn timeseries_daily_includes_rollups_for_equivalent_day_boundaries() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(18);
    insert_invocation_rollup(&state.pool, archived_date, SOURCE_PROXY, 4, 4, 0, 400, 1.0).await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "90d".to_string(),
            bucket: Some("1d".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Singapore".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch singapore daily timeseries");

    let singapore = "Asia/Singapore".parse::<Tz>().expect("valid singapore tz");
    let bucket = response
        .points
        .iter()
        .find(|point| bucket_date_in_tz(&point.bucket_start, singapore) == archived_date)
        .expect("should include archived rollup day for matching boundaries");

    assert_eq!(bucket.total_count, 4);
    assert_eq!(bucket.success_count, 4);
    assert_eq!(bucket.failure_count, 0);
    assert_eq!(bucket.total_tokens, 400);
    assert_f64_close(bucket.total_cost, 1.0);
}

#[tokio::test]
async fn timeseries_hourly_backed_bucket_stays_available_across_archive_boundary() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(12);
    let live_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(2);
    insert_invocation_rollup(&state.pool, archived_date, SOURCE_PROXY, 6, 5, 1, 600, 1.2).await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-subday-live-after-archive",
        &format_naive(live_date.and_hms_opt(12, 30, 0).expect("valid live time")),
        "success",
        Some(120.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "30d".to_string(),
            bucket: Some("12h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch timeseries with hourly rollup continuity");

    assert_eq!(response.bucket_seconds, 43_200);
    assert_eq!(response.effective_bucket, "12h");
    assert!(!response.bucket_limited_to_daily);
    assert!(response.available_buckets.contains(&"1h".to_string()));
    assert!(response.available_buckets.contains(&"12h".to_string()));
    assert!(response.available_buckets.contains(&"1d".to_string()));

    let archived_bucket = response
        .points
        .iter()
        .find(|point| {
            point.total_count == 6
                && point.success_count == 5
                && point.failure_count == 1
                && point.total_tokens == 600
        })
        .expect("should include archived rollup-backed bucket");
    assert_eq!(archived_bucket.total_count, 6);
}

#[tokio::test]
async fn summary_hourly_backed_since_omits_pre_cutoff_partial_archived_hours() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let bucket_start = local_naive_to_utc(archived_hour_local, Shanghai);
    let before_start = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("before-start local time"),
    );
    let after_start = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(50))
            .expect("after-start local time"),
    );
    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-exact-archived-start",
        &[
            (
                1_i64,
                "summary-before-start",
                before_start.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.1_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "summary-after-start",
                after_start.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.1_f64,
                Some(200.0),
            ),
        ],
    )
    .await;
    insert_invocation_hourly_rollup_bucket(
        &state.pool,
        bucket_start,
        SOURCE_PROXY,
        2,
        2,
        0,
        20,
        0.2,
    )
    .await;

    let start = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(30))
            .expect("range start local time"),
        Shanghai,
    );
    let totals =
        query_hourly_backed_summary_since(state.as_ref(), start, InvocationSourceScope::ProxyOnly)
            .await
            .expect("load exact archived summary totals");

    assert_eq!(totals.total_count, 0);
    assert_eq!(totals.success_count, 0);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 0);
    assert_f64_close(totals.total_cost, 0.0);
}

#[tokio::test]
async fn timeseries_hourly_backed_omits_pre_cutoff_partial_archived_hours() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let bucket_start = local_naive_to_utc(archived_hour_local, Shanghai);
    let before_start = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("before-start local time"),
    );
    let after_start = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(50))
            .expect("after-start local time"),
    );
    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "timeseries-exact-archived-start",
        &[
            (
                1_i64,
                "timeseries-before-start",
                before_start.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.1_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "timeseries-after-start",
                after_start.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.1_f64,
                Some(200.0),
            ),
        ],
    )
    .await;
    insert_invocation_hourly_rollup_bucket(
        &state.pool,
        bucket_start,
        SOURCE_PROXY,
        2,
        2,
        0,
        20,
        0.2,
    )
    .await;

    let start = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(30))
            .expect("range start local time"),
        Shanghai,
    );
    let end = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(55))
            .expect("range end local time"),
        Shanghai,
    );
    let Json(response) = fetch_timeseries_from_hourly_rollups(
        state.clone(),
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
    .expect("fetch exact archived timeseries");

    let point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(bucket_start))
        .expect("archived hour bucket should remain visible");
    assert_eq!(point.total_count, 0);
    assert_eq!(point.success_count, 0);
    assert_eq!(point.failure_count, 0);
    assert_eq!(point.total_tokens, 0);
    assert_f64_close(point.total_cost, 0.0);
}

#[tokio::test]
async fn hourly_backed_summary_omits_pre_cutoff_partial_hour_rollups() {
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
    .bind("pre-cutoff-live-exact")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(12_i64)
    .bind(0.12_f64)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert pre-cutoff live exact row");

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
        1,
        0,
        12,
        0.12,
    )
    .await;

    let start = local_naive_to_utc(pre_cutoff_local - ChronoDuration::minutes(15), Shanghai);
    let totals =
        query_hourly_backed_summary_since(state.as_ref(), start, InvocationSourceScope::ProxyOnly)
            .await
            .expect("load summary totals across retention cutoff");

    assert_eq!(totals.total_count, 0);
    assert_eq!(totals.success_count, 0);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 0);
    assert_f64_close(totals.total_cost, 0.0);
}

#[tokio::test]
async fn hourly_backed_summary_replays_pre_cutoff_full_hour_live_rows_after_rollup_cursor() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;

    let full_hour_local = start_of_local_day(Utc::now(), Shanghai)
        .with_timezone(&Shanghai)
        .naive_local()
        - ChronoDuration::hours(2);
    let occurred_at = format_naive(
        full_hour_local
            .checked_add_signed(ChronoDuration::minutes(15))
            .expect("valid occurred_at"),
    );
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
    .bind("pre-cutoff-full-hour-live-tail")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(12_i64)
    .bind(0.12_f64)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert pre-cutoff full-hour live row");

    let start = local_naive_to_utc(full_hour_local - ChronoDuration::hours(1), Shanghai);
    let totals =
        query_hourly_backed_summary_since(state.as_ref(), start, InvocationSourceScope::ProxyOnly)
            .await
            .expect("load summary totals across full archived hour");

    assert_eq!(totals.total_count, 1);
    assert_eq!(totals.success_count, 1);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 12);
    assert_f64_close(totals.total_cost, 0.12);
}

#[tokio::test]
async fn invocation_hourly_rollup_ignores_null_status_for_success_failure_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let bucket_start_epoch = invocation_bucket_start_epoch(&occurred_at)
        .expect("bucket start epoch should be derivable");
    let mut tx = state.pool.begin().await.expect("begin transaction");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[InvocationHourlySourceRecord {
            id: 1,
            occurred_at,
            source: SOURCE_PROXY.to_string(),
            status: None,
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: None,
            input_tokens: Some(4),
            output_tokens: Some(3),
            cache_input_tokens: Some(2),
            reasoning_tokens: Some(1),
            total_tokens: Some(7),
            cost: Some(0.07),
            upstream_account_id: Some(42),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: None,
            payload: None,
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
        }],
        &INVOCATION_HOURLY_ROLLUP_TARGETS,
    )
    .await
    .expect("upsert hourly rollup source row");
    tx.commit().await.expect("commit transaction");

    let rows = query_invocation_hourly_rollup_range(
        &state.pool,
        bucket_start_epoch,
        bucket_start_epoch + 3_600,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query hourly rollup range");
    let row = rows.first().expect("rollup row should exist");
    assert_eq!(row.total_count, 1);
    assert_eq!(row.success_count, 0);
    assert_eq!(row.failure_count, 0);
    assert_eq!(row.total_tokens, 7);
    assert_eq!(row.input_tokens, 4);
    assert_eq!(row.output_tokens, 3);
    assert_eq!(row.cache_input_tokens, 2);
    assert_eq!(row.reasoning_tokens, 1);
    assert_f64_close(row.total_cost, 0.07);

    for table in [
        "upstream_account_stats_hourly",
        "upstream_account_stats_minute",
    ] {
        let account_tokens = sqlx::query_as::<_, (i64, i64, i64, i64, i64)>(&format!(
            "SELECT total_tokens, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens FROM {table} WHERE upstream_account_id = 42",
        ))
        .fetch_one(&state.pool)
        .await
        .unwrap_or_else(|error| panic!("load {table} token components: {error}"));
        assert_eq!(account_tokens, (7, 4, 3, 2, 1));
    }
}

#[tokio::test]
async fn invocation_hourly_rollup_ignores_running_and_pending_for_failure_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let bucket_start_epoch = invocation_bucket_start_epoch(&occurred_at)
        .expect("bucket start epoch should be derivable");
    let mut tx = state.pool.begin().await.expect("begin transaction");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[
            InvocationHourlySourceRecord {
                id: 1,
                occurred_at: occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some("success".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: None,
                input_tokens: None,
                output_tokens: None,
                cache_input_tokens: None,
                reasoning_tokens: None,
                total_tokens: Some(7),
                cost: Some(0.07),
                upstream_account_id: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: None,
                failure_kind: None,
                failure_class: None,
                is_actionable: None,
                payload: None,
                t_total_ms: None,
                t_req_read_ms: None,
                t_req_parse_ms: None,
                t_upstream_connect_ms: None,
                t_upstream_ttfb_ms: None,
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            },
            InvocationHourlySourceRecord {
                id: 2,
                occurred_at: occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some("running".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: None,
                input_tokens: None,
                output_tokens: None,
                cache_input_tokens: None,
                reasoning_tokens: None,
                total_tokens: Some(9),
                cost: Some(0.09),
                upstream_account_id: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: Some(
                    "[upstream_response_failed] upstream response stream reported failure"
                        .to_string(),
                ),
                failure_kind: Some("upstream_response_failed".to_string()),
                failure_class: Some("service_failure".to_string()),
                is_actionable: Some(1),
                payload: None,
                t_total_ms: None,
                t_req_read_ms: None,
                t_req_parse_ms: None,
                t_upstream_connect_ms: None,
                t_upstream_ttfb_ms: None,
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            },
            InvocationHourlySourceRecord {
                id: 3,
                occurred_at: occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some("pending".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: None,
                input_tokens: None,
                output_tokens: None,
                cache_input_tokens: None,
                reasoning_tokens: None,
                total_tokens: Some(11),
                cost: Some(0.11),
                upstream_account_id: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: Some(
                    "[downstream_closed] downstream closed while streaming upstream response"
                        .to_string(),
                ),
                failure_kind: Some("downstream_closed".to_string()),
                failure_class: Some("client_abort".to_string()),
                is_actionable: Some(0),
                payload: None,
                t_total_ms: None,
                t_req_read_ms: None,
                t_req_parse_ms: None,
                t_upstream_connect_ms: None,
                t_upstream_ttfb_ms: None,
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            },
            InvocationHourlySourceRecord {
                id: 4,
                occurred_at,
                source: SOURCE_PROXY.to_string(),
                status: Some("failed".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: None,
                input_tokens: None,
                output_tokens: None,
                cache_input_tokens: None,
                reasoning_tokens: None,
                total_tokens: Some(13),
                cost: Some(0.13),
                upstream_account_id: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: Some("upstream stream error".to_string()),
                failure_kind: Some("upstream_stream_error".to_string()),
                failure_class: None,
                is_actionable: Some(1),
                payload: None,
                t_total_ms: None,
                t_req_read_ms: None,
                t_req_parse_ms: None,
                t_upstream_connect_ms: None,
                t_upstream_ttfb_ms: None,
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            },
        ],
        &INVOCATION_HOURLY_ROLLUP_TARGETS,
    )
    .await
    .expect("upsert hourly rollup source rows");
    tx.commit().await.expect("commit transaction");

    let rows = query_invocation_hourly_rollup_range(
        &state.pool,
        bucket_start_epoch,
        bucket_start_epoch + 3_600,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query hourly rollup range");
    let row = rows.first().expect("rollup row should exist");
    assert_eq!(row.total_count, 4);
    assert_eq!(row.success_count, 1);
    assert_eq!(row.failure_count, 1);
    assert_eq!(row.total_tokens, 40);
    assert_f64_close(row.total_cost, 0.4);
}

#[tokio::test]
async fn invocation_hourly_rollup_excludes_structured_legacy_http_200_failures_from_ttfb_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let bucket_start_epoch = invocation_bucket_start_epoch(&occurred_at)
        .expect("bucket start epoch should be derivable");
    let mut tx = state.pool.begin().await.expect("begin transaction");
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &[
            InvocationHourlySourceRecord {
                id: 11,
                occurred_at: occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some("http_200".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: None,
                input_tokens: None,
                output_tokens: None,
                cache_input_tokens: None,
                reasoning_tokens: None,
                total_tokens: Some(10),
                cost: Some(0.10),
                upstream_account_id: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: Some("".to_string()),
                failure_kind: None,
                failure_class: None,
                is_actionable: None,
                payload: None,
                t_total_ms: None,
                t_req_read_ms: None,
                t_req_parse_ms: None,
                t_upstream_connect_ms: None,
                t_upstream_ttfb_ms: Some(120.0),
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            },
            InvocationHourlySourceRecord {
                id: 12,
                occurred_at,
                source: SOURCE_PROXY.to_string(),
                status: Some("http_200".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: None,
                input_tokens: None,
                output_tokens: None,
                cache_input_tokens: None,
                reasoning_tokens: None,
                total_tokens: Some(20),
                cost: Some(0.20),
                upstream_account_id: None,
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: Some("".to_string()),
                failure_kind: Some("upstream_response_failed".to_string()),
                failure_class: Some("service_failure".to_string()),
                is_actionable: Some(1),
                payload: None,
                t_total_ms: None,
                t_req_read_ms: None,
                t_req_parse_ms: None,
                t_upstream_connect_ms: None,
                t_upstream_ttfb_ms: Some(840.0),
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            },
        ],
        &INVOCATION_HOURLY_ROLLUP_TARGETS,
    )
    .await
    .expect("upsert hourly rollup source rows");
    tx.commit().await.expect("commit transaction");

    let rows = query_invocation_hourly_rollup_range(
        &state.pool,
        bucket_start_epoch,
        bucket_start_epoch + 3_600,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query hourly rollup range");
    let row = rows.first().expect("rollup row should exist");
    assert_eq!(row.total_count, 2);
    assert_eq!(row.success_count, 1);
    assert_eq!(row.failure_count, 1);
    assert_eq!(row.first_byte_sample_count, 1);
    assert_f64_close(row.first_byte_sum_ms, 120.0);
}

#[tokio::test]
async fn combined_totals_ignore_null_status_for_success_failure_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

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
    .bind(201_i64)
    .bind("summary-null-status")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(Option::<String>::None)
    .bind(7_i64)
    .bind(0.07_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert null-status invocation row");

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
    .bind(202_i64)
    .bind("summary-success")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(11_i64)
    .bind(0.11_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert success invocation row");

    let totals = query_combined_totals(
        &state.pool,
        StatsFilter::All,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query combined totals");
    assert_eq!(totals.total_count, 2);
    assert_eq!(totals.success_count, 1);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 18);
    assert_f64_close(totals.total_cost, 0.18);
}
