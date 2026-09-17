#[tokio::test]
pub(crate) async fn archived_failure_fallback_skips_double_count_for_readable_materialized_archive_when_same_bucket_sibling_is_unreadable()
 {
    let state = archive_retention_test_state().await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(9, 0, 0)
    .expect("valid same-bucket failure hour");
    let archived_readable_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("same-bucket readable failure archived time"),
    );
    let archived_unreadable_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("same-bucket unreadable failure archived time"),
    );

    let _readable_archive_path = seed_materialized_failure_archive(
        &state,
        "failure-same-bucket-unreadable-sibling-materialized",
        &archived_readable_at,
        10,
        0.10,
        "HTTP 429 too many requests",
        "upstream_response_failed",
    )
    .await;
    let unreadable_archive_path = seed_materialized_failure_archive(
        &state,
        "failure-same-bucket-unreadable-sibling-broken",
        &archived_unreadable_at,
        20,
        0.20,
        "HTTP 429 too many requests",
        "upstream_response_failed",
    )
    .await;
    insert_failure_rollup(&state.pool, &archived_readable_at, 2).await;
    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_readable_at)
        .expect("same-bucket failure bucket start epoch should be derivable");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-bucket failure archive");

    assert_archived_failure_views(state, 2, Some(2)).await;
}

#[tokio::test]
pub(crate) async fn historical_failure_read_path_skips_unreadable_pending_archives() {
    let state = archive_retention_test_state().await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(9, 0, 0)
    .expect("valid unreadable pending failure hour");
    let archived_failed_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("unreadable pending failure time"),
    );

    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-pending-corrupt-read-path",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-pending-corrupt-read-path-first",
            occurred_at: archived_failed_at.as_str(),
            source: SOURCE_PROXY,
            status: "failed",
            total_tokens: 10_i64,
            cost: 0.10_f64,
            ttfb_ms: Some(100.0),
            payload: Some("{}"),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: Some("HTTP 429 too many requests"),
            failure_kind: Some("upstream_response_failed"),
            failure_class: Some("service_failure"),
            is_actionable: Some(1_i64),
        }],
    )
    .await;

    insert_failure_rollup(&state.pool, &archived_failed_at, 1).await;
    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_failed_at)
        .expect("derive unreadable pending failure bucket epoch");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable pending failure archive");

    assert_archived_failure_views(state, 1, Some(1)).await;
}

#[tokio::test]
pub(crate) async fn archived_failure_fallback_skips_double_count_for_readable_materialized_archive_when_same_month_sibling_is_unreadable()
 {
    let state = archive_retention_test_state().await;

    let archived_materialized_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(9, 0, 0)
    .expect("valid readable materialized failure hour");
    let archived_unreadable_hour_local = archived_materialized_hour_local
        .checked_add_signed(ChronoDuration::hours(2))
        .expect("valid unreadable same-month failure sibling hour");
    let archived_materialized_at = format_naive(
        archived_materialized_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("readable materialized failure archived time"),
    );
    let archived_unreadable_at = format_naive(
        archived_unreadable_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("unreadable same-month failure archived time"),
    );

    let _materialized_archive_path = seed_materialized_failure_archive(
        &state,
        "failure-same-month-unreadable-sibling-materialized",
        &archived_materialized_at,
        10,
        0.10,
        "HTTP 429 too many requests",
        "upstream_response_failed",
    )
    .await;
    let unreadable_archive_path = seed_materialized_failure_archive(
        &state,
        "failure-same-month-unreadable-sibling-broken",
        &archived_unreadable_at,
        20,
        0.20,
        "upstream stream error",
        "upstream_stream_error",
    )
    .await;
    insert_failure_rollup(&state.pool, &archived_materialized_at, 1).await;
    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_materialized_at)
        .expect("same-month failure bucket start epoch should be derivable");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-month failure sibling archive");

    assert_archived_failure_views(state, 1, Some(1)).await;
}

#[tokio::test]
pub(crate) async fn archived_failure_fallback_includes_missing_categories_from_partially_materialized_bucket()
 {
    let state = archive_retention_test_state().await;
    let (first_at, _) = seed_failure_pair_archive(
        &state,
        FailurePairSpec {
            batch_name: "failure-partial-category-archive",
            first_invoke_id: "failure-partial-category-first",
            second_invoke_id: "failure-partial-category-second",
            local_hour: 10,
            second_in_next_hour: false,
            second_minute: 25,
            second_error_message: "upstream stream error",
            second_failure_kind: "upstream_stream_error",
        },
    )
    .await;
    insert_failure_rollup(&state.pool, &first_at, 1).await;

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary with partially materialized failure categories");
    assert_eq!(failure_summary.total_failures, 2);
    assert_eq!(failure_summary.service_failure_count, 2);
    assert_eq!(failure_summary.actionable_failure_count, 2);

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
    .expect("fetch error distribution with partially materialized failure categories");
    assert!(
        error_distribution
            .items
            .iter()
            .any(|item| item.reason == "too_many_requests" && item.count == 1)
    );
    assert!(
        error_distribution
            .items
            .iter()
            .any(|item| item.reason == "upstream stream error" && item.count == 1)
    );
}

#[tokio::test]
pub(crate) async fn historical_perf_stats_include_unmaterialized_archived_hours() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(11, 0, 0)
    .expect("valid archived perf hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived perf time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived perf time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-unmaterialized-archive",
        &[
            (
                1_i64,
                "perf-unmaterialized-first",
                archived_first_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "perf-unmaterialized-second",
                archived_second_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(200.0),
            ),
        ],
    )
    .await;

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(perf_stats) = fetch_perf_stats(
        State(state),
        Query(PerfQuery {
            range: historical_range,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical perf stats with unmaterialized archived hours");

    let upstream_first_byte = perf_stats
        .stages
        .iter()
        .find(|stage| stage.stage == "upstreamFirstByte")
        .expect("historical perf stats should include archived upstream first byte samples");
    assert_eq!(upstream_first_byte.count, 2);
    assert_f64_close(upstream_first_byte.avg_ms, 150.0);
    assert_f64_close(upstream_first_byte.max_ms, 200.0);
}

pub(crate) async fn insert_perf_first_byte_rollup(
    pool: &SqlitePool,
    occurred_at: &str,
    samples: &[f64],
) {
    let bucket = invocation_bucket_start_epoch(occurred_at).expect("perf rollup bucket");
    let mut histogram = empty_approx_histogram();
    for sample in samples {
        add_approx_histogram_sample(&mut histogram, *sample);
    }
    sqlx::query(
        r#"
        INSERT INTO proxy_perf_stage_hourly (
            bucket_start_epoch, stage, sample_count, sum_ms, max_ms, histogram, updated_at
        ) VALUES (?1, 'upstreamFirstByte', ?2, ?3, ?4, ?5, datetime('now'))
        "#,
    )
    .bind(bucket)
    .bind(samples.len() as i64)
    .bind(samples.iter().sum::<f64>())
    .bind(samples.iter().copied().fold(0.0_f64, f64::max))
    .bind(encode_approx_histogram(&histogram).expect("encode perf histogram"))
    .execute(pool)
    .await
    .expect("seed perf first-byte rollup");
}

#[tokio::test]
pub(crate) async fn historical_perf_stats_fill_missing_samples_from_partially_materialized_archived_hours()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(13, 0, 0)
    .expect("valid archived partial perf hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived partial perf time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived partial perf time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-partial-materialized-archive",
        &[
            (
                1_i64,
                "perf-partial-materialized-first",
                archived_first_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "perf-partial-materialized-second",
                archived_second_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(200.0),
            ),
        ],
    )
    .await;

    insert_perf_first_byte_rollup(&state.pool, &archived_first_at, &[100.0]).await;

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(perf_stats) = fetch_perf_stats(
        State(state),
        Query(PerfQuery {
            range: historical_range,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical perf stats with partially materialized archive bucket");

    let upstream_first_byte = perf_stats
        .stages
        .iter()
        .find(|stage| stage.stage == "upstreamFirstByte")
        .expect("historical perf stats should include partially materialized archived samples");
    assert_eq!(upstream_first_byte.count, 2);
    assert_f64_close(upstream_first_byte.avg_ms, 150.0);
    assert_f64_close(upstream_first_byte.max_ms, 200.0);
}

#[tokio::test]
pub(crate) async fn historical_perf_stats_include_unreplayed_full_hour_tail_without_inline_catch_up()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let historical_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(14, 0, 0)
    .expect("valid historical perf hour");
    let historical_tail_at = format_naive(
        historical_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("historical perf tail time"),
    );
    let bucket_start_epoch = invocation_bucket_start_epoch(&historical_tail_at)
        .expect("historical perf bucket start epoch should be derivable");

    let mut histogram = empty_approx_histogram();
    add_approx_histogram_sample(&mut histogram, 100.0);
    sqlx::query(
        r#"
        INSERT INTO proxy_perf_stage_hourly (
            bucket_start_epoch,
            stage,
            sample_count,
            sum_ms,
            max_ms,
            histogram,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind(bucket_start_epoch)
    .bind("upstreamFirstByte")
    .bind(1_i64)
    .bind(100.0_f64)
    .bind(100.0_f64)
    .bind(encode_approx_histogram(&histogram).expect("encode historical perf histogram"))
    .execute(&state.pool)
    .await
    .expect("seed historical perf rollup row");

    insert_timeseries_invocation(
        &state.pool,
        "historical-perf-unreplayed-tail",
        &historical_tail_at,
        "success",
        Some(200.0),
    )
    .await;

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(perf_stats) = fetch_perf_stats(
        State(state),
        Query(PerfQuery {
            range: historical_range,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical perf stats with unreplayed full-hour tail");

    let upstream_first_byte = perf_stats
        .stages
        .iter()
        .find(|stage| stage.stage == "upstreamFirstByte")
        .expect("historical perf stats should include unreplayed full-hour tail samples");
    assert_eq!(upstream_first_byte.count, 2);
    assert_f64_close(upstream_first_byte.avg_ms, 150.0);
    assert_f64_close(upstream_first_byte.max_ms, 200.0);
}

async fn seed_mixed_state_perf_archives(state: &Arc<AppState>) -> i64 {
    let hour = (Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(10))
        .and_hms_opt(13, 0, 0)
        .expect("valid archived mixed-state perf hour");
    let materialized_at = format_naive(hour + ChronoDuration::minutes(5));
    let pending_first_at = format_naive(hour + ChronoDuration::minutes(25));
    let pending_second_at = format_naive(hour + ChronoDuration::minutes(45));
    let original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-mixed-state-archive-a",
        &[(
            401,
            "perf-mixed-state-materialized",
            &materialized_at,
            SOURCE_PROXY,
            "success",
            10,
            0.10,
            Some(200.0),
        )],
    )
    .await;
    let archive_path = state
        .config
        .archive_dir
        .join("perf-mixed-state-archive-a.sqlite.gz");
    let _ = fs::remove_file(&archive_path);
    fs::rename(&original_path, &archive_path).expect("move materialized perf archive part");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, \
         historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark mixed-state perf archive part materialized");
    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-mixed-state-archive-b",
        &[
            (
                1,
                "perf-mixed-state-pending-first",
                &pending_first_at,
                SOURCE_PROXY,
                "success",
                20,
                0.20,
                Some(100.0),
            ),
            (
                2,
                "perf-mixed-state-pending-second",
                &pending_second_at,
                SOURCE_PROXY,
                "success",
                30,
                0.30,
                Some(300.0),
            ),
        ],
    )
    .await;
    invocation_bucket_start_epoch(&materialized_at).expect("mixed-state perf bucket start")
}

#[tokio::test]
pub(crate) async fn historical_perf_archive_delta_distinguishes_materialized_sibling_parts_and_stale_pending_overlap()
 {
    let state = archive_retention_test_state().await;
    let bucket_start_epoch = seed_mixed_state_perf_archives(&state).await;
    let mut histogram = empty_approx_histogram();
    add_approx_histogram_sample(&mut histogram, 200.0);
    add_approx_histogram_sample(&mut histogram, 300.0);
    sqlx::query(
        r#"
        INSERT INTO proxy_perf_stage_hourly (
            bucket_start_epoch,
            stage,
            sample_count,
            sum_ms,
            max_ms,
            histogram,
            updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind(bucket_start_epoch)
    .bind("upstreamFirstByte")
    .bind(2_i64)
    .bind(500.0_f64)
    .bind(300.0_f64)
    .bind(encode_approx_histogram(&histogram).expect("encode mixed-state perf histogram"))
    .execute(&state.pool)
    .await
    .expect("seed mixed-state materialized perf row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    let archived_start = Utc
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .expect("valid mixed-state perf archive start");
    let archived_end = archived_start + ChronoDuration::hours(1);
    let archived_perf = query_unmaterialized_proxy_perf_stage_rollups_from_archives(
        &state.pool,
        archived_start,
        archived_end,
        None,
    )
    .await
    .expect("query mixed-state perf archive delta");
    let upstream_first_byte = archived_perf
        .get("upstreamFirstByte")
        .expect("mixed-state perf delta should include upstreamFirstByte");
    assert_eq!(upstream_first_byte.sample_count, 1);
    assert_f64_close(upstream_first_byte.sum_ms, 100.0);
    assert_f64_close(upstream_first_byte.max_ms, 100.0);
}

use super::*;
