async fn seed_materialized_perf_archive(
    state: &Arc<AppState>,
    batch_name: &str,
    occurred_at: &str,
    tokens: i64,
    cost: f64,
    ttfb_ms: f64,
    point_coverage: bool,
) -> PathBuf {
    let original = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        batch_name,
        &[(
            1,
            batch_name,
            occurred_at,
            SOURCE_PROXY,
            "success",
            tokens,
            cost,
            Some(ttfb_ms),
        )],
    )
    .await;
    let path = state
        .config
        .archive_dir
        .join(format!("{batch_name}.sqlite.gz"));
    let _ = fs::remove_file(&path);
    fs::rename(&original, &path).expect("move materialized perf archive");
    let update_sql = if point_coverage {
        "UPDATE archive_batches SET file_path = ?1, \
         historical_rollups_materialized_at = datetime('now'), \
         coverage_start_at = ?3, coverage_end_at = ?3 \
         WHERE dataset = 'codex_invocations' AND file_path = ?2"
    } else {
        "UPDATE archive_batches SET file_path = ?1, \
         historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?2 AND ?3 = ?3"
    };
    sqlx::query(update_sql)
        .bind(path.to_string_lossy().to_string())
        .bind(original.to_string_lossy().to_string())
        .bind(occurred_at)
        .execute(&state.pool)
        .await
        .expect("mark perf archive materialized");
    path
}

async fn assert_historical_perf_stats(
    state: Arc<AppState>,
    count: i64,
    average: f64,
    maximum: f64,
) {
    let range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(perf) = fetch_perf_stats(
        State(state),
        Query(PerfQuery {
            range,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical perf stats");
    let first_byte = perf
        .stages
        .iter()
        .find(|stage| stage.stage == "upstreamFirstByte")
        .expect("historical perf stats should include upstream first byte");
    assert_eq!(first_byte.count, count);
    assert_f64_close(first_byte.avg_ms, average);
    assert_f64_close(first_byte.max_ms, maximum);
}

#[tokio::test]
pub(crate) async fn historical_perf_archive_delta_keeps_pending_stage_rows_when_materialized_sibling_archive_is_unreadable()
 {
    let state = archive_retention_test_state().await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(13, 0, 0)
    .expect("valid archived unreadable mixed-state perf hour");
    let archived_materialized_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("materialized unreadable mixed-state perf time"),
    );
    let archived_pending_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("first pending unreadable mixed-state perf time"),
    );
    let archived_pending_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(45))
            .expect("second pending unreadable mixed-state perf time"),
    );

    let materialized_archive_path = seed_materialized_perf_archive(
        &state,
        "perf-mixed-state-unreadable-archive-a",
        &archived_materialized_at,
        10,
        0.10,
        200.0,
        false,
    )
    .await;

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-mixed-state-unreadable-archive-b",
        &[
            (
                1_i64,
                "perf-mixed-state-unreadable-pending-first",
                archived_pending_first_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "perf-mixed-state-unreadable-pending-second",
                archived_pending_second_at.as_str(),
                SOURCE_PROXY,
                "success",
                30_i64,
                0.30_f64,
                Some(300.0),
            ),
        ],
    )
    .await;

    insert_perf_first_byte_rollup(&state.pool, &archived_materialized_at, &[200.0, 300.0]).await;
    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_materialized_at)
        .expect("unreadable mixed-state perf bucket start epoch should be derivable");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&materialized_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable mixed-state materialized perf archive");

    let archived_start = Utc
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .expect("valid unreadable mixed-state perf archive start");
    let archived_end = archived_start + ChronoDuration::hours(1);
    let archived_perf = query_unmaterialized_proxy_perf_stage_rollups_from_archives(
        &state.pool,
        archived_start,
        archived_end,
        None,
    )
    .await
    .expect("query unreadable mixed-state perf archive delta");
    let upstream_first_byte = archived_perf
        .get("upstreamFirstByte")
        .expect("unreadable mixed-state perf delta should include upstreamFirstByte");
    assert_eq!(upstream_first_byte.sample_count, 2);
    assert_f64_close(upstream_first_byte.sum_ms, 400.0);
    assert_f64_close(upstream_first_byte.max_ms, 300.0);
}

#[tokio::test]
pub(crate) async fn historical_perf_stats_skip_double_count_for_readable_materialized_archive_when_same_bucket_sibling_is_unreadable()
 {
    let state = archive_retention_test_state().await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(13, 0, 0)
    .expect("valid same-bucket perf hour");
    let archived_readable_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("same-bucket readable perf archived time"),
    );
    let archived_unreadable_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("same-bucket unreadable perf archived time"),
    );

    let _readable_archive_path = seed_materialized_perf_archive(
        &state,
        "perf-same-bucket-unreadable-sibling-materialized",
        &archived_readable_at,
        10,
        0.10,
        100.0,
        true,
    )
    .await;
    let unreadable_archive_path = seed_materialized_perf_archive(
        &state,
        "perf-same-bucket-unreadable-sibling-broken",
        &archived_unreadable_at,
        20,
        0.20,
        200.0,
        true,
    )
    .await;
    insert_perf_first_byte_rollup(&state.pool, &archived_readable_at, &[100.0, 200.0]).await;
    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_readable_at)
        .expect("same-bucket perf bucket start epoch should be derivable");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-bucket perf archive");

    assert_historical_perf_stats(state, 2, 150.0, 200.0).await;
}

#[tokio::test]
pub(crate) async fn historical_perf_stats_skip_double_count_for_readable_materialized_archive_when_same_month_sibling_is_unreadable()
 {
    let state = archive_retention_test_state().await;

    let archived_materialized_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(13, 0, 0)
    .expect("valid readable materialized perf hour");
    let archived_unreadable_hour_local = archived_materialized_hour_local
        .checked_add_signed(ChronoDuration::hours(2))
        .expect("valid unreadable same-month perf sibling hour");
    let archived_materialized_at = format_naive(
        archived_materialized_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("readable materialized perf archived time"),
    );
    let archived_unreadable_at = format_naive(
        archived_unreadable_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("unreadable same-month perf archived time"),
    );

    let _materialized_archive_path = seed_materialized_perf_archive(
        &state,
        "perf-same-month-unreadable-sibling-materialized",
        &archived_materialized_at,
        10,
        0.10,
        100.0,
        true,
    )
    .await;
    let unreadable_archive_path = seed_materialized_perf_archive(
        &state,
        "perf-same-month-unreadable-sibling-broken",
        &archived_unreadable_at,
        20,
        0.20,
        120.0,
        true,
    )
    .await;
    insert_perf_first_byte_rollup(&state.pool, &archived_materialized_at, &[100.0]).await;
    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_materialized_at)
        .expect("same-month perf bucket start epoch should be derivable");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-month perf sibling archive");

    assert_historical_perf_stats(state, 1, 100.0, 100.0).await;
}

#[tokio::test]
pub(crate) async fn historical_perf_read_path_skips_unreadable_pending_archives() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(13, 0, 0)
    .expect("valid unreadable pending perf hour");
    let archived_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("unreadable pending perf time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-pending-corrupt-read-path",
        &[(
            1_i64,
            "perf-pending-corrupt-read-path-first",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_at)
        .expect("derive unreadable pending perf bucket epoch");
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
    .bind(encode_approx_histogram(&histogram).expect("encode unreadable pending perf histogram"))
    .execute(&state.pool)
    .await
    .expect("seed unreadable pending perf row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable pending perf archive");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(perf_stats) = fetch_perf_stats(
        State(state),
        Query(PerfQuery {
            range: historical_range,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical perf stats with unreadable pending archive");

    let upstream_first_byte = perf_stats
        .stages
        .iter()
        .find(|stage| stage.stage == "upstreamFirstByte")
        .expect("unreadable pending perf stats should include materialized stage");
    assert_eq!(upstream_first_byte.count, 1);
    assert_f64_close(upstream_first_byte.avg_ms, 100.0);
    assert_f64_close(upstream_first_byte.max_ms, 100.0);
}

#[tokio::test]
pub(crate) async fn historical_timeseries_includes_unmaterialized_archived_hours_without_inline_repair()
 {
    let state = archive_retention_test_state().await;
    let archived_hour_local = archived_local_hour(10, 12);
    let archived_success_at = local_minute(archived_hour_local, 5);
    let archived_failed_at = local_minute(archived_hour_local, 35);

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "timeseries-unmaterialized-archive",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1_i64,
                invoke_id: "timeseries-unmaterialized-success",
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
                invoke_id: "timeseries-unmaterialized-failed",
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

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let response = fetch_archived_hour_timeseries(state, archived_hour_local).await;
    assert_timeseries_totals(&response, start, 2, 1, 1, 30, 0.30);
    let point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("historical timeseries bucket should exist");
    assert_f64_close(point.non_success_cost, 0.20);
}

async fn seed_timeseries_overlap_rows(state: &Arc<AppState>) -> NaiveDateTime {
    let archived_hour_local = archived_local_hour(10, 12);
    let archived_success_at = local_minute(archived_hour_local, 5);
    let archived_failed_at = local_minute(archived_hour_local, 35);
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "timeseries-live-tail-overlap",
        &[
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: "timeseries-live-tail-overlap-success",
                occurred_at: archived_success_at.as_str(),
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
                id: 3,
                invoke_id: "timeseries-live-tail-overlap-failed",
                occurred_at: archived_failed_at.as_str(),
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
    set_hourly_rollup_progress(&state.pool, HOURLY_ROLLUP_DATASET_INVOCATIONS, 1).await;
    archived_hour_local
}

async fn insert_timeseries_overlap_live_rows(
    pool: &SqlitePool,
    archived_hour_local: NaiveDateTime,
) {
    let rows = [
        (
            2,
            "timeseries-live-tail-overlap-success",
            5,
            "success",
            10,
            0.10,
            100.0,
        ),
        (
            3,
            "timeseries-live-tail-overlap-failed",
            35,
            "failed",
            20,
            0.20,
            120.0,
        ),
    ];
    for (id, invoke_id, minute, status, tokens, cost, ttfb_ms) in rows {
        let occurred_at = local_minute(archived_hour_local, minute);
        let (error_message, failure_kind, failure_class, is_actionable) = if status == "failed" {
            (
                Some("HTTP 429 too many requests"),
                Some("upstream_response_failed"),
                Some("service_failure"),
                Some(1_i64),
            )
        } else {
            (None, None, Some("none"), Some(0_i64))
        };
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost,
                error_message, failure_kind, failure_class, is_actionable,
                detail_level, t_upstream_ttfb_ms, raw_response
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(tokens)
        .bind(cost)
        .bind(error_message)
        .bind(failure_kind)
        .bind(failure_class)
        .bind(is_actionable)
        .bind(DETAIL_LEVEL_FULL)
        .bind(ttfb_ms)
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert overlapping timeseries live row");
    }
}

#[tokio::test]
pub(crate) async fn historical_timeseries_skips_archive_fallback_rows_already_counted_in_live_tail()
{
    let state = archive_retention_test_state().await;
    let archived_hour_local = seed_timeseries_overlap_rows(&state).await;
    insert_timeseries_overlap_live_rows(&state.pool, archived_hour_local).await;

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let response = fetch_archived_hour_timeseries(state, archived_hour_local).await;
    assert_timeseries_totals(&response, start, 2, 1, 1, 30, 0.30);
}

#[tokio::test]
pub(crate) async fn historical_timeseries_skip_unreadable_materialized_archives() {
    let state = archive_retention_test_state().await;
    let archived_hour_local = archived_local_hour(10, 12);
    let archived_success_at = local_minute(archived_hour_local, 5);
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "timeseries-materialized-corrupt-read-path",
        &[(
            1_i64,
            "timeseries-materialized-corrupt-read-path-success",
            archived_success_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;

    mark_archive_materialized(&state.pool, &archive_path).await;

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch: start.timestamp(),
            total_count: 1,
            success_count: 1,
            failure_count: 0,
            total_tokens: 10,
            total_cost: 0.10,
            first_byte_samples: &[100.0],
            context: "seed materialized timeseries rollup row",
        },
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt materialized timeseries archive");

    let response = fetch_archived_hour_timeseries(state, archived_hour_local).await;
    assert_timeseries_totals(&response, start, 1, 1, 0, 10, 0.10);
}

use super::*;
