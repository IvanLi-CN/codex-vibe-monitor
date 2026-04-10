async fn insert_parallel_work_prompt_cache_rollup_hourly_row(
    pool: &SqlitePool,
    bucket_start: DateTime<Utc>,
    prompt_cache_key: &str,
    request_count: i64,
) {
    let first_seen_at = format_naive(bucket_start.with_timezone(&Shanghai).naive_local());
    let last_seen_at = format_naive(
        (bucket_start + ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_rollup_hourly (
            bucket_start_epoch,
            source,
            prompt_cache_key,
            request_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost,
            first_seen_at,
            last_seen_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
        "#,
    )
    .bind(bucket_start.timestamp())
    .bind(SOURCE_PROXY)
    .bind(prompt_cache_key)
    .bind(request_count)
    .bind(request_count)
    .bind(0_i64)
    .bind(request_count * 10)
    .bind(request_count as f64 * 0.01)
    .bind(first_seen_at)
    .bind(last_seen_at)
    .execute(pool)
    .await
    .expect("insert prompt cache hourly rollup row");
}

async fn seed_invocation_archive_batch(
    pool: &SqlitePool,
    config: &AppConfig,
    batch_name: &str,
    rows: &[(i64, &str, &str, &str, &str, i64, f64, Option<f64>)],
) -> PathBuf {
    let rows = rows
        .iter()
        .map(
            |(id, invoke_id, occurred_at, source, status, total_tokens, cost, ttfb_ms)| {
                SeedInvocationArchiveBatchRow {
                    id: *id,
                    invoke_id,
                    occurred_at,
                    source,
                    status,
                    total_tokens: *total_tokens,
                    cost: *cost,
                    ttfb_ms: *ttfb_ms,
                    payload: Some("{}"),
                    detail_level: DETAIL_LEVEL_FULL,
                }
            },
        )
        .collect::<Vec<_>>();
    seed_invocation_archive_batch_with_details(pool, config, batch_name, &rows).await
}

#[derive(Clone, Copy)]
struct SeedInvocationArchiveBatchRow<'a> {
    id: i64,
    invoke_id: &'a str,
    occurred_at: &'a str,
    source: &'a str,
    status: &'a str,
    total_tokens: i64,
    cost: f64,
    ttfb_ms: Option<f64>,
    payload: Option<&'a str>,
    detail_level: &'a str,
}

async fn seed_invocation_archive_batch_with_details(
    pool: &SqlitePool,
    config: &AppConfig,
    batch_name: &str,
    rows: &[SeedInvocationArchiveBatchRow<'_>],
) -> PathBuf {
    let month_key = rows
        .first()
        .map(|row| row.occurred_at[..7].to_string())
        .expect("archive batch rows should not be empty");
    let archive_path = archive_batch_file_path(config, "codex_invocations", &month_key)
        .expect("resolve invocation archive batch path");
    fs::create_dir_all(
        archive_path
            .parent()
            .expect("invocation archive batch should have parent"),
    )
    .expect("create invocation archive dir");

    let archive_db_path = config.archive_dir.join(format!("{batch_name}.sqlite"));
    let _ = fs::remove_file(&archive_db_path);
    fs::File::create(&archive_db_path).expect("create invocation archive sqlite file");
    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open invocation archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create invocation archive schema");
    for row in rows {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, source, status, total_tokens, cost, t_upstream_ttfb_ms, payload, detail_level, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
            "#,
        )
        .bind(row.id)
        .bind(row.invoke_id)
        .bind(row.occurred_at)
        .bind(row.source)
        .bind(row.status)
        .bind(row.total_tokens)
        .bind(row.cost)
        .bind(row.ttfb_ms)
        .bind(row.payload)
        .bind(row.detail_level)
        .bind("{}")
        .bind(row.occurred_at)
        .execute(&archive_pool)
        .await
        .expect("insert invocation archive row");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress invocation archive batch");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_path).expect("archive sha256"))
    .bind(rows.len() as i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(pool)
    .await
    .expect("insert invocation archive batch manifest");
    archive_path
}

fn bucket_date_in_tz(bucket_start: &str, tz: Tz) -> NaiveDate {
    DateTime::parse_from_rfc3339(bucket_start)
        .expect("valid bucket start")
        .with_timezone(&tz)
        .date_naive()
}

fn shanghai_bucket_date(bucket_start: &str) -> NaiveDate {
    bucket_date_in_tz(bucket_start, Shanghai)
}

fn assert_f64_close(actual: f64, expected: f64) {
    let diff = (actual - expected).abs();
    assert!(
        diff < 1e-6,
        "expected {expected}, got {actual}, diff={diff}"
    );
}

#[tokio::test]
async fn parallel_work_stats_counts_distinct_prompt_cache_keys_per_bucket() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_minute_epoch =
        align_reporting_bucket_epoch(Utc::now().timestamp(), 60, Shanghai).expect("align minute");
    let minute_a = Utc
        .timestamp_opt(current_minute_epoch - 3 * 60, 0)
        .single()
        .expect("minute a");
    let minute_b = minute_a + ChronoDuration::minutes(1);

    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-a-1",
        minute_a + ChronoDuration::seconds(10),
        "pck-alpha",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-a-2",
        minute_a + ChronoDuration::seconds(20),
        "pck-alpha",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-a-3",
        minute_a + ChronoDuration::seconds(30),
        "pck-beta",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-b-1",
        minute_b + ChronoDuration::seconds(10),
        "pck-alpha",
    )
    .await;

    let Json(response) = fetch_parallel_work_stats(
        State(state),
        Query(ParallelWorkStatsQuery {
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    let minute_a_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(minute_a))
        .expect("minute a point");
    let minute_b_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(minute_b))
        .expect("minute b point");

    assert_eq!(minute_a_point.parallel_count, 2);
    assert_eq!(minute_b_point.parallel_count, 1);
    assert_eq!(response.minute7d.active_bucket_count, 2);
    assert_eq!(response.minute7d.max_count, Some(2));
    assert_eq!(response.minute7d.min_count, Some(0));
}

#[tokio::test]
async fn parallel_work_stats_minute7d_supports_non_shanghai_reporting_timezones() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let reporting_tz = "UTC".parse::<Tz>().expect("valid utc tz");
    let current_minute_epoch =
        align_reporting_bucket_epoch(Utc::now().timestamp(), 60, reporting_tz)
            .expect("align minute");
    let minute_bucket = Utc
        .timestamp_opt(current_minute_epoch - 3 * 60, 0)
        .single()
        .expect("minute bucket");

    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-utc-1",
        minute_bucket + ChronoDuration::seconds(10),
        "pck-utc-alpha",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-utc-2",
        minute_bucket + ChronoDuration::seconds(20),
        "pck-utc-alpha",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-utc-3",
        minute_bucket + ChronoDuration::seconds(30),
        "pck-utc-beta",
    )
    .await;

    let Json(response) = fetch_parallel_work_stats(
        State(state),
        Query(ParallelWorkStatsQuery {
            time_zone: Some("UTC".to_string()),
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    let minute_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(minute_bucket))
        .expect("utc minute point");

    assert_eq!(minute_point.parallel_count, 2);
    assert_eq!(response.minute7d.active_bucket_count, 1);
    assert_eq!(response.minute7d.max_count, Some(2));
}

#[tokio::test]
async fn parallel_work_stats_falls_back_historical_windows_for_sub_hour_timezones() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let requested_tz = "Asia/Kolkata"
        .parse::<Tz>()
        .expect("valid kolkata reporting tz");
    let current_minute_epoch =
        align_reporting_bucket_epoch(Utc::now().timestamp(), 60, requested_tz)
            .expect("align kolkata minute");
    let minute_bucket = Utc
        .timestamp_opt(current_minute_epoch - 3 * 60, 0)
        .single()
        .expect("kolkata minute bucket");

    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-kolkata-1",
        minute_bucket + ChronoDuration::seconds(10),
        "pck-kolkata-alpha",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-minute-kolkata-2",
        minute_bucket + ChronoDuration::seconds(20),
        "pck-kolkata-beta",
    )
    .await;

    let current_hour_epoch =
        align_reporting_bucket_epoch(Utc::now().timestamp(), 3_600, Shanghai).expect("align hour");
    let fallback_hour = Utc
        .timestamp_opt(current_hour_epoch - 6 * 3_600, 0)
        .single()
        .expect("fallback hour");
    let fallback_prompt_cache_key = "pck-shanghai-fallback";
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        fallback_hour,
        fallback_prompt_cache_key,
        1,
    )
    .await;

    let current_day_start =
        local_midnight_utc(Utc::now().with_timezone(&Shanghai).date_naive(), Shanghai);
    let previous_day_start = current_day_start - ChronoDuration::days(1);
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        previous_day_start,
        fallback_prompt_cache_key,
        1,
    )
    .await;

    let Json(response) = fetch_parallel_work_stats(
        State(state),
        Query(ParallelWorkStatsQuery {
            time_zone: Some("Asia/Kolkata".to_string()),
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    let minute_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(minute_bucket))
        .expect("kolkata minute point");
    let hour_point = response
        .hour30d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(fallback_hour))
        .expect("fallback hour point");

    assert_eq!(minute_point.parallel_count, 2);
    assert_eq!(response.minute7d.effective_time_zone, "Asia/Kolkata");
    assert!(!response.minute7d.time_zone_fallback);

    assert_eq!(hour_point.parallel_count, 1);
    assert_eq!(response.hour30d.effective_time_zone, "Asia/Shanghai");
    assert!(response.hour30d.time_zone_fallback);

    assert_eq!(response.day_all.effective_time_zone, "Asia/Shanghai");
    assert!(response.day_all.time_zone_fallback);
    assert_eq!(response.day_all.points.len(), 1);
    assert_eq!(
        response.day_all.points[0].bucket_start,
        format_utc_iso(previous_day_start)
    );
    assert_eq!(response.day_all.points[0].parallel_count, 1);
}

#[tokio::test]
async fn parallel_work_stats_zero_fill_and_exclude_current_minute_and_hour() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let current_minute_epoch =
        align_reporting_bucket_epoch(now.timestamp(), 60, Shanghai).expect("align minute");
    let current_minute = Utc
        .timestamp_opt(current_minute_epoch, 0)
        .single()
        .expect("current minute");
    let previous_minute = current_minute - ChronoDuration::minutes(1);
    let empty_minute = current_minute - ChronoDuration::minutes(2);

    insert_parallel_work_invocation(
        &state.pool,
        "parallel-prev-minute",
        previous_minute + ChronoDuration::seconds(10),
        "pck-prev-minute",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-current-minute",
        current_minute + ChronoDuration::seconds(10),
        "pck-current-minute",
    )
    .await;

    let current_hour_epoch =
        align_reporting_bucket_epoch(now.timestamp(), 3_600, Shanghai).expect("align hour");
    let current_hour = Utc
        .timestamp_opt(current_hour_epoch, 0)
        .single()
        .expect("current hour");
    let previous_hour = current_hour - ChronoDuration::hours(1);
    let empty_hour = current_hour - ChronoDuration::hours(2);

    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        previous_hour,
        "pck-prev-hour",
        2,
    )
    .await;
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        current_hour,
        "pck-current-hour",
        2,
    )
    .await;

    let Json(response) = fetch_parallel_work_stats(
        State(state),
        Query(ParallelWorkStatsQuery {
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    let previous_minute_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(previous_minute))
        .expect("previous minute point");
    let empty_minute_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(empty_minute))
        .expect("empty minute point");
    assert_eq!(previous_minute_point.parallel_count, 1);
    assert_eq!(empty_minute_point.parallel_count, 0);
    assert!(
        response
            .minute7d
            .points
            .iter()
            .all(|point| point.bucket_start != format_utc_iso(current_minute))
    );

    let previous_hour_point = response
        .hour30d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(previous_hour))
        .expect("previous hour point");
    let empty_hour_point = response
        .hour30d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(empty_hour))
        .expect("empty hour point");
    assert_eq!(previous_hour_point.parallel_count, 1);
    assert_eq!(empty_hour_point.parallel_count, 0);
    assert!(
        response
            .hour30d
            .points
            .iter()
            .all(|point| point.bucket_start != format_utc_iso(current_hour))
    );
}

#[tokio::test]
async fn parallel_work_stats_day_all_aggregates_distinct_keys_per_day() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_day_start =
        local_midnight_utc(Utc::now().with_timezone(&Shanghai).date_naive(), Shanghai);
    let previous_day_start = current_day_start - ChronoDuration::days(1);

    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        previous_day_start,
        "pck-day-alpha",
        1,
    )
    .await;
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        previous_day_start + ChronoDuration::hours(5),
        "pck-day-alpha",
        2,
    )
    .await;
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        previous_day_start + ChronoDuration::hours(8),
        "pck-day-beta",
        1,
    )
    .await;
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        current_day_start + ChronoDuration::hours(1),
        "pck-current-day",
        1,
    )
    .await;

    let Json(response) = fetch_parallel_work_stats(
        State(state),
        Query(ParallelWorkStatsQuery {
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    assert_eq!(response.day_all.complete_bucket_count, 1);
    assert_eq!(response.day_all.active_bucket_count, 1);
    assert_eq!(response.day_all.min_count, Some(2));
    assert_eq!(response.day_all.max_count, Some(2));
    assert_eq!(response.day_all.avg_count, Some(2.0));
    assert_eq!(response.day_all.points.len(), 1);
    assert_eq!(
        response.day_all.points[0].bucket_start,
        format_utc_iso(previous_day_start)
    );
    assert_eq!(response.day_all.points[0].parallel_count, 2);
}

#[tokio::test]
async fn parallel_work_stats_day_all_returns_null_summary_without_complete_days() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_day_start =
        local_midnight_utc(Utc::now().with_timezone(&Shanghai).date_naive(), Shanghai);
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        current_day_start + ChronoDuration::hours(2),
        "pck-today-only",
        1,
    )
    .await;

    let Json(response) = fetch_parallel_work_stats(
        State(state),
        Query(ParallelWorkStatsQuery {
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    assert!(response.day_all.points.is_empty());
    assert_eq!(response.day_all.complete_bucket_count, 0);
    assert_eq!(response.day_all.active_bucket_count, 0);
    assert_eq!(response.day_all.min_count, None);
    assert_eq!(response.day_all.max_count, None);
    assert_eq!(response.day_all.avg_count, None);
    assert_eq!(
        response.day_all.range_start,
        format_utc_iso(current_day_start)
    );
    assert_eq!(
        response.day_all.range_end,
        format_utc_iso(current_day_start)
    );
}

#[tokio::test]
async fn timeseries_includes_first_byte_avg_and_p95_for_success_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-1",
        &occurred_at,
        "success",
        Some(100.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-2",
        &occurred_at,
        "success",
        Some(200.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-3",
        &occurred_at,
        "success",
        Some(400.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-failure",
        &occurred_at,
        "failed",
        Some(800.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 4)
        .expect("should include populated bucket");

    assert_eq!(bucket.first_byte_sample_count, 3);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        (100.0 + 200.0 + 400.0) / 3.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms.expect("p95 should be present"),
        380.0,
    );
}

#[tokio::test]
async fn timeseries_includes_legacy_http_200_success_like_ttfb_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-legacy-http-200",
        &occurred_at,
        "http_200",
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
        }),
    )
    .await
    .expect("fetch timeseries for legacy http_200");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 1)
        .expect("should include populated bucket");

    assert_eq!(bucket.success_count, 1);
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
async fn timeseries_and_summary_do_not_treat_running_rows_with_failure_metadata_as_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    insert_timeseries_invocation(
        &state.pool,
        "timeseries-success",
        &occurred_at,
        "success",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-running",
        &occurred_at,
        "running",
        Some(120.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-pending",
        &occurred_at,
        "pending",
        Some(160.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-failed",
        &occurred_at,
        "failed",
        Some(240.0),
    )
    .await;
    sqlx::query(
        "UPDATE codex_invocations SET failure_kind = ?1, failure_class = ?2, error_message = ?3 WHERE invoke_id = ?4",
    )
    .bind("upstream_response_failed")
    .bind("service_failure")
    .bind("[upstream_response_failed] upstream response stream reported failure")
    .bind("timeseries-running")
    .execute(&state.pool)
    .await
    .expect("annotate running row with failure metadata");
    sqlx::query(
        "UPDATE codex_invocations SET failure_kind = ?1, failure_class = ?2, error_message = ?3 WHERE invoke_id = ?4",
    )
    .bind("downstream_closed")
    .bind("client_abort")
    .bind("[downstream_closed] downstream closed while streaming upstream response")
    .bind("timeseries-pending")
    .execute(&state.pool)
    .await
    .expect("annotate pending row with failure metadata");

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch summary for mixed running statuses");
    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch timeseries for mixed running statuses");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 4)
        .expect("should include populated bucket");

    assert_eq!(bucket.total_count, 4);
    assert_eq!(bucket.success_count, 1);
    assert_eq!(bucket.failure_count, 1);
}

#[tokio::test]
async fn all_time_summary_ignores_stale_rollup_failure_counts_for_running_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    insert_timeseries_invocation(
        &state.pool,
        "summary-all-success",
        &occurred_at,
        "success",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "summary-all-running",
        &occurred_at,
        "running",
        Some(120.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "summary-all-pending",
        &occurred_at,
        "pending",
        Some(160.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "summary-all-failed",
        &occurred_at,
        "failed",
        Some(240.0),
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&occurred_at)
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
    .bind(4_i64)
    .bind(1_i64)
    .bind(3_i64)
    .bind(40_i64)
    .bind(0.4_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed stale invocation rollup counts");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            updated_at = datetime('now')
        "#,
    )
    .bind("codex_invocations")
    .bind(4_i64)
    .execute(&state.pool)
    .await
    .expect("mark invocation rollup progress as caught up");

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with stale rollup counts");

    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
}

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

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with archived history present");

    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 40);
    assert!((summary.total_cost - 0.31).abs() < 1e-9);

    let repair_marker_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind("codex_invocations_summary_rollup_v2")
            .fetch_one(&state.pool)
            .await
            .expect("load archived all-time summary repair marker");
    assert_eq!(repair_marker_cursor, 1);
    let repair_live_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind("codex_invocations_summary_rollup_v2_live_cursor")
            .fetch_one(&state.pool)
            .await
            .expect("load archived all-time summary repair live cursor");
    assert_eq!(repair_live_cursor, 10);
    let invocation_rollup_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations")
    .fetch_one(&state.pool)
    .await
    .expect("load invocation hourly rollup live cursor after repair");
    assert_eq!(invocation_rollup_cursor, 10);

    let Json(summary_repeat) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
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

    let Json(summary_with_live_tail) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
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
async fn all_time_summary_missing_archive_does_not_mark_repair_complete() {
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
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-missing-archive",
        &[(
            1_i64,
            "summary-all-missing-archive-success",
            archived_success_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;

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
    .bind(99_i64)
    .bind(99_i64)
    .bind(0_i64)
    .bind(990_i64)
    .bind(9.9_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed stale invocation rollup counts before failed repair");

    fs::remove_file(&archive_path).expect("remove archived invocation batch from disk");

    let response = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect_err("missing archive should fail all-time summary repair")
    .into_response();
    assert_eq!(response.status().as_u16(), 500);

    let repair_marker_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2")
    .fetch_optional(&state.pool)
    .await
    .expect("load missing-archive repair marker");
    assert_eq!(repair_marker_cursor, None);

    let rollup_total_count: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly")
            .fetch_one(&state.pool)
            .await
            .expect("load rollup total count after failed repair");
    assert_eq!(rollup_total_count, 99);
}

#[tokio::test]
async fn all_time_summary_missing_summary_markers_do_not_replay_materialized_archives() {
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
    let archived_failed_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("archived failed time"),
    );

    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-marker-only-backfill",
        &[
            (
                1_i64,
                "summary-all-marker-only-success",
                archived_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "summary-all-marker-only-failed",
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

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark archived invocation batch as already materialized");

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
    .bind(2_i64)
    .bind(1_i64)
    .bind(1_i64)
    .bind(20_i64)
    .bind(0.30_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed pre-materialized summary rollups");
    sqlx::query(
        r#"
        INSERT INTO invocation_failure_rollup_hourly (
            bucket_start_epoch,
            source,
            failure_class,
            is_actionable,
            error_category,
            failure_count,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind("service_failure")
    .bind(1_i64)
    .bind("upstream_response_failed")
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed pre-materialized failure rollups");

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
    .expect("mark summary repair as complete");
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
    .bind(0_i64)
    .execute(&state.pool)
    .await
    .expect("seed summary repair live cursor");

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with missing summary replay markers");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 20);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);

    let replayed_targets = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target IN (?2, ?3)",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired summary replay markers");
    assert_eq!(replayed_targets, 2);

    let rollup_total_count: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly")
            .fetch_one(&state.pool)
            .await
            .expect("load rollup total count after metadata-only marker repair");
    assert_eq!(
        rollup_total_count, 2,
        "missing summary replay markers on already-materialized archives must not double-count archived invocations",
    );
}

#[tokio::test]
async fn all_time_summary_repair_preserves_pruned_materialized_archives() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(460))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );

    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-materialized-pruned-history",
        &[(
            1_i64,
            "summary-all-materialized-pruned-success",
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
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark archived invocation batch as already materialized");

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
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed pre-materialized summary rollups");

    fs::remove_file(&archive_path).expect("prune materialized archive file");

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
    .bind("summary-all-materialized-pruned-live-success")
    .bind(&live_occurred_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10_i64)
    .bind(0.01_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert live invocation row");

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with pruned materialized archives");

    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 20);
    assert!((summary.total_cost - 0.11).abs() < 1e-9);

    let replayed_targets = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target IN (?2, ?3)",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired summary replay markers for pruned archive");
    assert_eq!(replayed_targets, 2);

    let repair_marker_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2")
    .fetch_optional(&state.pool)
    .await
    .expect("load summary repair marker");
    assert_eq!(repair_marker_cursor, Some(1));

    let repair_live_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2_live_cursor")
    .fetch_one(&state.pool)
    .await
    .expect("load summary repair live cursor");
    assert_eq!(repair_live_cursor, 10);

    let rollup_total_count: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly")
            .fetch_one(&state.pool)
            .await
            .expect("load rollup total count after repair");
    assert_eq!(rollup_total_count, 2);
}

#[tokio::test]
async fn all_time_summary_repair_does_not_advance_shared_live_cursor_without_hourly_sync() {
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

    let totals = query_combined_totals(
        &state.pool,
        state.config.crs_stats.as_ref(),
        StatsFilter::All,
        InvocationSourceScope::All,
    )
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
async fn all_time_summary_rollup_repair_counts_mixed_case_success_status() {
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

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with mixed-case archived success");

    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
}

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
async fn timeseries_hourly_backed_includes_crs_deltas() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.crs_stats = Some(CrsStatsConfig {
        base_url: Url::parse("https://crs.example.com/").expect("valid crs base url"),
        api_id: "test-api".to_string(),
        period: "daily".to_string(),
        poll_interval: Duration::from_secs(3600),
    });
    let state = test_state_from_config(config, true).await;
    let captured_at = Utc::now() - ChronoDuration::minutes(30);
    let stats_date = captured_at
        .with_timezone(&Shanghai)
        .date_naive()
        .to_string();

    sqlx::query(
        r#"
        INSERT INTO stats_source_deltas (
            source,
            period,
            stats_date,
            captured_at,
            captured_at_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(SOURCE_CRS)
    .bind("daily")
    .bind(&stats_date)
    .bind(format_utc_iso(captured_at))
    .bind(captured_at.timestamp())
    .bind(3_i64)
    .bind(2_i64)
    .bind(1_i64)
    .bind(300_i64)
    .bind(0.9_f64)
    .execute(&state.pool)
    .await
    .expect("insert crs delta");

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("UTC".to_string()),
        }),
    )
    .await
    .expect("fetch hourly-backed timeseries with crs deltas");

    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.total_count)
            .sum::<i64>(),
        3
    );
    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.success_count)
            .sum::<i64>(),
        2
    );
    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.failure_count)
            .sum::<i64>(),
        1
    );
    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.total_tokens)
            .sum::<i64>(),
        300
    );
    assert_f64_close(
        response
            .points
            .iter()
            .map(|point| point.total_cost)
            .sum::<f64>(),
        0.9,
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
        }),
    )
    .await
    .expect("missing exact-range archive batch should not fail timeseries");
    assert!(response.points.iter().all(|point| point.total_count == 0));

    cleanup_temp_test_dir(&temp_dir);
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
async fn collect_summary_snapshots_uses_hourly_backed_duration_windows() {
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
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(45))
            .expect("archived local time"),
    );
    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-broadcast-hourly-window",
        &[(
            1_i64,
            "summary-broadcast-archived-row",
            archived_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            25_i64,
            0.25_f64,
            Some(250.0),
        )],
    )
    .await;
    insert_invocation_hourly_rollup_bucket(
        &state.pool,
        bucket_start,
        SOURCE_PROXY,
        1,
        1,
        0,
        25,
        0.25,
    )
    .await;

    let summaries = collect_summary_snapshots(
        &state.pool,
        state.config.crs_stats.as_ref(),
        state.config.invocation_max_days,
    )
    .await
    .expect("collect summary snapshots");

    let month = summaries
        .iter()
        .find(|summary| summary.window == "1mo")
        .expect("1mo summary should be present");
    assert_eq!(month.summary.total_count, 1);
    assert_eq!(month.summary.success_count, 1);
    assert_eq!(month.summary.failure_count, 0);
    assert_eq!(month.summary.total_tokens, 25);
    assert_f64_close(month.summary.total_cost, 0.25);

    let day = summaries
        .iter()
        .find(|summary| summary.window == "1d")
        .expect("1d summary should be present");
    assert_eq!(day.summary.total_count, 0);
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
async fn hourly_backed_summary_trims_crs_totals_to_effective_proxy_range() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    config.crs_stats = Some(CrsStatsConfig {
        base_url: Url::parse("https://crs.example.com/").expect("valid crs base url"),
        api_id: "test-api".to_string(),
        period: "daily".to_string(),
        poll_interval: Duration::from_secs(3600),
    });
    let state = test_state_from_config(config, true).await;

    let pre_cutoff_local = start_of_local_day(Utc::now(), Shanghai)
        .with_timezone(&Shanghai)
        .naive_local()
        - ChronoDuration::minutes(15);
    let captured_at = local_naive_to_utc(pre_cutoff_local, Shanghai);
    let stats_date = captured_at
        .with_timezone(&Shanghai)
        .date_naive()
        .to_string();
    sqlx::query(
        r#"
        INSERT INTO stats_source_deltas (
            source,
            period,
            stats_date,
            captured_at,
            captured_at_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            total_cost
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(SOURCE_CRS)
    .bind("daily")
    .bind(&stats_date)
    .bind(format_utc_iso(captured_at))
    .bind(captured_at.timestamp())
    .bind(3_i64)
    .bind(2_i64)
    .bind(1_i64)
    .bind(300_i64)
    .bind(0.9_f64)
    .execute(&state.pool)
    .await
    .expect("insert pre-cutoff crs delta");

    let start = local_naive_to_utc(pre_cutoff_local - ChronoDuration::minutes(15), Shanghai);
    let totals =
        query_hourly_backed_summary_since(state.as_ref(), start, InvocationSourceScope::All)
            .await
            .expect("load summary totals across retention cutoff with crs");

    assert_eq!(totals.total_count, 0);
    assert_eq!(totals.success_count, 0);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 0);
    assert_f64_close(totals.total_cost, 0.0);
}

#[tokio::test]
async fn hourly_timeseries_trims_crs_deltas_to_effective_proxy_range() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 0;
    config.crs_stats = Some(CrsStatsConfig {
        base_url: Url::parse("https://crs.example.com/").expect("valid crs base url"),
        api_id: "test-api".to_string(),
        period: "daily".to_string(),
        poll_interval: Duration::from_secs(3600),
    });
    let state = test_state_from_config(config, true).await;

    let cutoff_local = start_of_local_day(Utc::now(), Shanghai)
        .with_timezone(&Shanghai)
        .naive_local();
    let pre_cutoff_captured_at =
        local_naive_to_utc(cutoff_local - ChronoDuration::minutes(15), Shanghai);
    let post_cutoff_captured_at =
        local_naive_to_utc(cutoff_local + ChronoDuration::minutes(5), Shanghai);
    for (captured_at, total_count, success_count, failure_count, total_tokens, total_cost) in [
        (
            pre_cutoff_captured_at,
            3_i64,
            2_i64,
            1_i64,
            300_i64,
            0.9_f64,
        ),
        (
            post_cutoff_captured_at,
            2_i64,
            2_i64,
            0_i64,
            200_i64,
            0.4_f64,
        ),
    ] {
        let stats_date = captured_at
            .with_timezone(&Shanghai)
            .date_naive()
            .to_string();
        sqlx::query(
            r#"
            INSERT INTO stats_source_deltas (
                source,
                period,
                stats_date,
                captured_at,
                captured_at_epoch,
                total_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(SOURCE_CRS)
        .bind("daily")
        .bind(&stats_date)
        .bind(format_utc_iso(captured_at))
        .bind(captured_at.timestamp())
        .bind(total_count)
        .bind(success_count)
        .bind(failure_count)
        .bind(total_tokens)
        .bind(total_cost)
        .execute(&state.pool)
        .await
        .expect("insert crs delta around retention cutoff");
    }

    let start = local_naive_to_utc(cutoff_local - ChronoDuration::minutes(30), Shanghai);
    let end = local_naive_to_utc(cutoff_local + ChronoDuration::minutes(10), Shanghai);
    let Json(response) = fetch_timeseries_from_hourly_rollups(
        state.clone(),
        TimeseriesQuery {
            range: "ignored".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        },
        Shanghai,
        InvocationSourceScope::All,
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
    .expect("fetch hourly-backed timeseries with crs trim across retention cutoff");

    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.total_count)
            .sum::<i64>(),
        2
    );
    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.success_count)
            .sum::<i64>(),
        2
    );
    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.failure_count)
            .sum::<i64>(),
        0
    );
    assert_eq!(
        response
            .points
            .iter()
            .map(|point| point.total_tokens)
            .sum::<i64>(),
        200
    );
    assert_f64_close(
        response
            .points
            .iter()
            .map(|point| point.total_cost)
            .sum::<f64>(),
        0.4,
    );
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
            total_tokens: Some(7),
            cost: Some(0.07),
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
    assert_f64_close(row.total_cost, 0.07);
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
                total_tokens: Some(7),
                cost: Some(0.07),
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
                total_tokens: Some(9),
                cost: Some(0.09),
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
                total_tokens: Some(11),
                cost: Some(0.11),
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
                total_tokens: Some(13),
                cost: Some(0.13),
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
                total_tokens: Some(10),
                cost: Some(0.10),
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
                total_tokens: Some(20),
                cost: Some(0.20),
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
        None,
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

#[tokio::test]
async fn combined_totals_count_legacy_null_status_failures_when_error_metadata_exists() {
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
            error_message,
            failure_kind,
            total_tokens,
            cost,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(204_i64)
    .bind("summary-null-status-failure")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(Option::<String>::None)
    .bind("upstream exploded")
    .bind("unknown_future_failure_kind")
    .bind(5_i64)
    .bind(0.05_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy null-status failure row");

    let totals = query_combined_totals(
        &state.pool,
        None,
        StatsFilter::All,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query combined totals");
    assert_eq!(totals.total_count, 1);
    assert_eq!(totals.success_count, 0);
    assert_eq!(totals.failure_count, 1);
    assert_eq!(totals.total_tokens, 5);
    assert_f64_close(totals.total_cost, 0.05);
}

#[tokio::test]
async fn combined_totals_treat_legacy_http_200_without_error_as_success() {
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
    .bind(203_i64)
    .bind("summary-http-200-success-like")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("http_200")
    .bind(9_i64)
    .bind(0.09_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy http_200 invocation row");

    let totals = query_combined_totals(
        &state.pool,
        None,
        StatsFilter::All,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("query combined totals");
    assert_eq!(totals.total_count, 1);
    assert_eq!(totals.success_count, 1);
    assert_eq!(totals.failure_count, 0);
    assert_eq!(totals.total_tokens, 9);
    assert_f64_close(totals.total_cost, 0.09);
}

#[test]
fn resolve_failure_classification_keeps_unknown_legacy_http_200_failure_kinds_actionable() {
    let classification = resolve_failure_classification(
        Some("http_200"),
        Some(""),
        Some("unknown_future_failure_kind"),
        None,
        None,
    );

    assert_eq!(classification.failure_class, FailureClass::ServiceFailure);
    assert_eq!(
        classification.failure_kind.as_deref(),
        Some("unknown_future_failure_kind"),
    );
    assert!(classification.is_actionable);
}

#[test]
fn invocation_archive_pruned_success_details_require_empty_legacy_http_200_error_message() {
    let failed_legacy_http_200 = InvocationHourlySourceRecord {
        id: 1,
        occurred_at: "2026-03-28 00:00:00".to_string(),
        source: SOURCE_PROXY.to_string(),
        status: Some("http_200".to_string()),
        detail_level: DETAIL_LEVEL_STRUCTURED_ONLY.to_string(),
        total_tokens: None,
        cost: None,
        error_message: Some("upstream parse failed".to_string()),
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        payload: None,
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
    };
    assert!(
        !invocation_archive_has_pruned_success_details(&[failed_legacy_http_200]),
        "legacy http_200 rows with a non-empty error message must not suppress archive rollups",
    );

    let success_like_legacy_http_200 = InvocationHourlySourceRecord {
        id: 2,
        occurred_at: "2026-03-28 00:00:00".to_string(),
        source: SOURCE_PROXY.to_string(),
        status: Some("http_200".to_string()),
        detail_level: DETAIL_LEVEL_STRUCTURED_ONLY.to_string(),
        total_tokens: None,
        cost: None,
        error_message: Some("   ".to_string()),
        failure_kind: None,
        failure_class: None,
        is_actionable: None,
        payload: None,
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
    };
    assert!(
        invocation_archive_has_pruned_success_details(&[success_like_legacy_http_200]),
        "legacy http_200 rows with an empty error message should still count as pruned success-like rows",
    );

    let structured_failure_legacy_http_200 = InvocationHourlySourceRecord {
        id: 3,
        occurred_at: "2026-03-28 00:00:00".to_string(),
        source: SOURCE_PROXY.to_string(),
        status: Some("http_200".to_string()),
        detail_level: DETAIL_LEVEL_STRUCTURED_ONLY.to_string(),
        total_tokens: None,
        cost: None,
        error_message: Some("   ".to_string()),
        failure_kind: Some("upstream_response_failed".to_string()),
        failure_class: Some("service_failure".to_string()),
        is_actionable: Some(1),
        payload: None,
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
    };
    assert!(
        !invocation_archive_has_pruned_success_details(&[structured_failure_legacy_http_200]),
        "legacy http_200 rows with structured failure metadata must not be treated as pruned successes",
    );
}
