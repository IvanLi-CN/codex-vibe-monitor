#[tokio::test]
pub(crate) async fn timeseries_ttft_uses_first_token_samples_without_ttfb_fallback() {
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
pub(crate) fn bucket_aggregate_uses_histogram_for_mixed_rollup_and_exact_p95() {
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
pub(crate) async fn timeseries_daily_includes_archived_rollup_days_without_ttfb() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(40);
    insert_invocation_rollup(
        &state.pool,
        InvocationRollupFixture {
            stats_date: archived_date,
            source: SOURCE_PROXY,
            total_count: 7,
            success_count: 5,
            failure_count: 2,
            total_tokens: 700,
            total_cost: 1.75,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        },
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
pub(crate) async fn timeseries_daily_stays_continuous_after_rollup_archive() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(12);
    let live_date = Utc::now().with_timezone(&Shanghai).date_naive();
    let live_occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    insert_invocation_rollup_with_latency_samples(
        &state.pool,
        InvocationRollupFixture {
            stats_date: archived_date,
            source: SOURCE_PROXY,
            total_count: 3,
            success_count: 2,
            failure_count: 1,
            total_tokens: 300,
            total_cost: 3.0,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[30_000.0],
        },
    )
    .await;
    insert_timeseries_invocation_with_stages(
        &state.pool,
        TimeseriesInvocationFixture {
            invoke_id: "timeseries-live-after-rollup",
            occurred_at: &live_occurred_at,
            status: "success",
            t_req_read_ms: Some(5_000.0),
            t_req_parse_ms: Some(10_000.0),
            t_upstream_connect_ms: Some(14_500.0),
            t_upstream_ttfb_ms: Some(500.0),
        },
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
pub(crate) async fn timeseries_daily_combines_rollup_and_live_within_same_day() {
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

    insert_invocation_rollup(
        &state.pool,
        InvocationRollupFixture {
            stats_date: mixed_date,
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
pub(crate) async fn timeseries_hourly_rollups_rebucket_for_different_timezones() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(30);
    insert_invocation_rollup(
        &state.pool,
        InvocationRollupFixture {
            stats_date: archived_date,
            source: SOURCE_PROXY,
            total_count: 9,
            success_count: 7,
            failure_count: 2,
            total_tokens: 900,
            total_cost: 2.25,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        },
    )
    .await;

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
pub(crate) async fn timeseries_hourly_recent_non_hour_aligned_timezones_fall_back_to_raw_rows() {
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
pub(crate) async fn timeseries_hourly_backed_ignores_missing_exact_archive_batch() {
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
pub(crate) async fn summary_yesterday_ignores_missing_non_overlapping_archive_batch() {
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
pub(crate) async fn account_summary_yesterday_ignores_materialized_archive_missing_account_usage_marker()
 {
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
pub(crate) async fn timeseries_daily_backed_ignores_pruned_legacy_archive_batch_files() {
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
pub(crate) async fn timeseries_hourly_historical_non_hour_aligned_timezones_are_rejected() {
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
pub(crate) async fn account_timeseries_hourly_historical_non_hour_aligned_timezones_are_rejected() {
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
pub(crate) async fn timeseries_daily_includes_rollups_for_equivalent_day_boundaries() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(18);
    insert_invocation_rollup(
        &state.pool,
        InvocationRollupFixture {
            stats_date: archived_date,
            source: SOURCE_PROXY,
            total_count: 4,
            success_count: 4,
            failure_count: 0,
            total_tokens: 400,
            total_cost: 1.0,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        },
    )
    .await;

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
pub(crate) async fn timeseries_hourly_backed_bucket_stays_available_across_archive_boundary() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(12);
    let live_date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(2);
    insert_invocation_rollup(
        &state.pool,
        InvocationRollupFixture {
            stats_date: archived_date,
            source: SOURCE_PROXY,
            total_count: 6,
            success_count: 5,
            failure_count: 1,
            total_tokens: 600,
            total_cost: 1.2,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        },
    )
    .await;
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
pub(crate) async fn summary_hourly_backed_since_omits_pre_cutoff_partial_archived_hours() {
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
        HourlyRollupFixture {
            bucket_start,
            source: SOURCE_PROXY,
            total_count: 2,
            success_count: 2,
            failure_count: 0,
            total_tokens: 20,
            total_cost: 0.2,
            first_byte_samples: &[],
            first_response_byte_total_samples: &[],
        },
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

use super::*;
