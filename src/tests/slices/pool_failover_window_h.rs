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
                    error_message: None,
                    failure_kind: None,
                    failure_class: Some("none"),
                    is_actionable: Some(0),
                }
            },
        )
        .collect::<Vec<_>>();
    seed_invocation_archive_batch_with_details(pool, config, batch_name, &rows).await
}

async fn run_background_invocation_summary_rollup_repair(pool: &SqlitePool) {
    crate::stats::ensure_invocation_summary_rollups_ready(pool)
        .await
        .expect("run background invocation summary rollup repair");
}

async fn insert_materialized_rollup_bucket_marker(
    pool: &SqlitePool,
    target: &str,
    bucket_start_epoch: i64,
    source: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(target)
    .bind(bucket_start_epoch)
    .bind(source)
    .execute(pool)
    .await
    .expect("insert materialized rollup bucket marker");
}

async fn insert_hourly_rollup_archive_replay_marker(
    pool: &SqlitePool,
    target: &str,
    file_path: &Path,
) {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(target)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("insert hourly rollup archive replay marker");
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
    error_message: Option<&'a str>,
    failure_kind: Option<&'a str>,
    failure_class: Option<&'a str>,
    is_actionable: Option<i64>,
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
                id, invoke_id, occurred_at, source, status, total_tokens, cost, t_upstream_ttfb_ms, payload, detail_level, error_message, failure_kind, failure_class, is_actionable, raw_response, created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
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
        .bind(row.error_message)
        .bind(row.failure_kind)
        .bind(row.failure_class)
        .bind(row.is_actionable)
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
    let inserted_current_minute = Utc
        .timestamp_opt(current_minute_epoch, 0)
        .single()
        .expect("current minute");
    let inserted_previous_minute = inserted_current_minute - ChronoDuration::minutes(1);

    insert_parallel_work_invocation(
        &state.pool,
        "parallel-prev-minute",
        inserted_previous_minute + ChronoDuration::seconds(10),
        "pck-prev-minute",
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-current-minute",
        inserted_current_minute + ChronoDuration::seconds(10),
        "pck-current-minute",
    )
    .await;

    let current_hour_epoch =
        align_reporting_bucket_epoch(now.timestamp(), 3_600, Shanghai).expect("align hour");
    let inserted_current_hour = Utc
        .timestamp_opt(current_hour_epoch, 0)
        .single()
        .expect("current hour");
    let inserted_previous_hour = inserted_current_hour - ChronoDuration::hours(1);

    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        inserted_previous_hour,
        "pck-prev-hour",
        2,
    )
    .await;
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        inserted_current_hour,
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

    let response_current_minute = DateTime::parse_from_rfc3339(&response.minute7d.range_end)
        .expect("parse minute range end")
        .with_timezone(&Utc);
    let response_previous_minute = response_current_minute - ChronoDuration::minutes(1);
    let response_empty_minute = response_current_minute - ChronoDuration::minutes(3);
    let previous_minute_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(response_previous_minute))
        .expect("previous minute point");
    let empty_minute_point = response
        .minute7d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(response_empty_minute))
        .expect("empty minute point");
    assert_eq!(previous_minute_point.parallel_count, 1);
    assert_eq!(empty_minute_point.parallel_count, 0);
    assert!(
        response
            .minute7d
            .points
            .iter()
            .all(|point| point.bucket_start != response.minute7d.range_end)
    );

    let response_current_hour = DateTime::parse_from_rfc3339(&response.hour30d.range_end)
        .expect("parse hour range end")
        .with_timezone(&Utc);
    let response_previous_hour = response_current_hour - ChronoDuration::hours(1);
    let response_empty_hour = response_current_hour - ChronoDuration::hours(3);
    let previous_hour_point = response
        .hour30d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(response_previous_hour))
        .expect("previous hour point");
    let empty_hour_point = response
        .hour30d
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(response_empty_hour))
        .expect("empty hour point");
    assert_eq!(previous_hour_point.parallel_count, 1);
    assert_eq!(empty_hour_point.parallel_count, 0);
    assert!(
        response
            .hour30d
            .points
            .iter()
            .all(|point| point.bucket_start != response.hour30d.range_end)
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
async fn timeseries_and_summary_count_completed_rows_as_success() {
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
        "timeseries-success-completed-control",
        &occurred_at,
        "success",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-completed-success-like",
        &occurred_at,
        "completed",
        Some(120.0),
    )
    .await;

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch summary for completed success-like row");
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);

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
    .expect("fetch timeseries for completed success-like row");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 2)
        .expect("should include populated bucket");

    assert_eq!(bucket.total_count, 2);
    assert_eq!(bucket.success_count, 2);
    assert_eq!(bucket.failure_count, 0);
    assert_eq!(bucket.first_byte_sample_count, 2);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        100.0,
    );
}

#[tokio::test]
async fn failure_summary_excludes_completed_success_like_rows_from_recent_totals() {
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
        "failure-summary-completed-success-like",
        &occurred_at,
        "completed",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "failure-summary-failed-control",
        &occurred_at,
        "failed",
        Some(120.0),
    )
    .await;

    let Json(summary) = fetch_failure_summary(
        State(state),
        Query(FailureSummaryQuery {
            range: "1h".to_string(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary for completed success-like row");

    assert_eq!(summary.total_failures, 1);
    assert_eq!(summary.service_failure_count, 1);
    assert_eq!(summary.client_failure_count, 0);
    assert_eq!(summary.client_abort_count, 0);
    assert_eq!(summary.actionable_failure_count, 1);
    assert_f64_close(summary.actionable_failure_rate, 1.0);
}

#[tokio::test]
async fn timeseries_reports_snapshot_id_for_live_exact_queries() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    for (id, invoke_id, status) in [(101_i64, "snapshot-row-1", "success"), (105_i64, "snapshot-row-2", "failed")] {
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
        .bind(id)
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(10_i64)
        .bind(0.01_f64)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert snapshot timeseries invocation");
    }

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

    assert_eq!(response.snapshot_id, 105);
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 2)
        .expect("should include populated bucket");
    assert_eq!(bucket.total_count, 2);
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
    assert_eq!(bucket.in_flight_count, 2);
}

#[tokio::test]
async fn timeseries_and_summary_count_http_200_rows_with_downstream_only_failure_metadata() {
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
        "timeseries-success-downstream-control",
        &occurred_at,
        "success",
        Some(80.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "timeseries-http-200-downstream-only",
        &occurred_at,
        "http_200",
        Some(120.0),
    )
    .await;
    sqlx::query("UPDATE codex_invocations SET payload = ?1 WHERE invoke_id = ?2")
        .bind(
            json!({
                "downstreamErrorMessage": "socket closed after response"
            })
            .to_string(),
        )
        .bind("timeseries-http-200-downstream-only")
        .execute(&state.pool)
        .await
        .expect("annotate http_200 row with downstream-only failure metadata");

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch summary for downstream-only http_200 row");
    assert_eq!(summary.total_count, 2);
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
    .expect("fetch timeseries for downstream-only http_200 row");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 2)
        .expect("should include populated bucket");

    assert_eq!(bucket.total_count, 2);
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

    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 2);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);

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

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
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

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
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

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some(historical_range.clone()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical summary with unmaterialized archived range");
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);

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

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some(historical_range.clone()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical summary with overlapping archive/live tail");
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 30);
    assert!((summary.total_cost - 0.30).abs() < 1e-9);

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

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
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

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
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
            1_i64,
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
            1_i64,
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

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
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
async fn all_time_summary_fallback_keeps_unmaterialized_rows_when_sibling_archive_part_is_materialized(
) {
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

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
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
async fn archived_failure_fallback_skips_already_materialized_archive_buckets() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_a_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid first archived failure hour");
    let archived_hour_b_local = archived_hour_a_local
        .checked_add_signed(ChronoDuration::hours(1))
        .expect("second archived failure hour");
    let archived_first_at = format_naive(
        archived_hour_a_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived failure time"),
    );
    let archived_second_at = format_naive(
        archived_hour_b_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("second archived failure time"),
    );

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-partial-materialized-archive",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1_i64,
                invoke_id: "failure-partial-materialized-first",
                occurred_at: archived_first_at.as_str(),
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
            },
            SeedInvocationArchiveBatchRow {
                id: 2_i64,
                invoke_id: "failure-partial-materialized-second",
                occurred_at: archived_second_at.as_str(),
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 20_i64,
                cost: 0.20_f64,
                ttfb_ms: Some(120.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some("upstream stream error"),
                failure_kind: Some("upstream_stream_error"),
                failure_class: Some("service_failure"),
                is_actionable: Some(1_i64),
            },
        ],
    )
    .await;

    let first_bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("first failure bucket start epoch should be derivable");
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
    .bind(first_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind("service_failure")
    .bind(1_i64)
    .bind("too_many_requests")
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed already materialized first failure bucket");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary with partially materialized archive");
    assert_eq!(failure_summary.total_failures, 2);
    assert_eq!(failure_summary.service_failure_count, 2);
    assert_eq!(failure_summary.client_failure_count, 0);
    assert_eq!(failure_summary.client_abort_count, 0);
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
    .expect("fetch error distribution with partially materialized archive");
    let total_distribution_count: i64 = error_distribution.items.iter().map(|item| item.count).sum();
    assert_eq!(total_distribution_count, 2);
}

#[tokio::test]
async fn archived_failure_fallback_includes_missing_rows_from_partially_materialized_count() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(9, 0, 0)
    .expect("valid archived partial failure count hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived partial failure count time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived partial failure count time"),
    );

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-partial-count-archive",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1_i64,
                invoke_id: "failure-partial-count-first",
                occurred_at: archived_first_at.as_str(),
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
            },
            SeedInvocationArchiveBatchRow {
                id: 2_i64,
                invoke_id: "failure-partial-count-second",
                occurred_at: archived_second_at.as_str(),
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

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("partial failure count bucket start epoch should be derivable");
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
    .bind("too_many_requests")
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed partially materialized failure count row");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary with partially materialized failure count");
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
    .expect("fetch error distribution with partially materialized failure count");
    assert!(
        error_distribution
            .items
            .iter()
            .any(|item| item.reason == "too_many_requests" && item.count == 2)
    );
}

#[tokio::test]
async fn archived_failure_fallback_aggregates_missing_rows_across_archive_parts() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(9, 0, 0)
    .expect("valid archived multipart failure hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived multipart failure time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived multipart failure time"),
    );

    let first_archive_original_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-multipart-archive-a",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-multipart-first",
            occurred_at: archived_first_at.as_str(),
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
    let first_archive_path = state
        .config
        .archive_dir
        .join("failure-multipart-archive-a.sqlite.gz");
    let _ = fs::remove_file(&first_archive_path);
    fs::rename(&first_archive_original_path, &first_archive_path)
        .expect("move first multipart failure archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(first_archive_path.to_string_lossy().to_string())
    .bind(first_archive_original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("update first multipart failure archive batch path after move");

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-multipart-archive-b",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-multipart-second",
            occurred_at: archived_second_at.as_str(),
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
        }],
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("multipart failure bucket start epoch should be derivable");
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
    .bind("too_many_requests")
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed partially materialized multipart failure count row");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary with multipart partially materialized failure count");
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
    .expect("fetch error distribution with multipart partially materialized failure count");
    assert!(
        error_distribution
            .items
            .iter()
            .any(|item| item.reason == "too_many_requests" && item.count == 2)
    );
}

#[tokio::test]
async fn archived_failure_fallback_keeps_unmaterialized_rows_when_sibling_archive_part_is_materialized(
) {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(9, 0, 0)
    .expect("valid archived mixed-state failure hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived mixed-state failure time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived mixed-state failure time"),
    );

    let first_archive_original_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-mixed-state-archive-a",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-mixed-state-first",
            occurred_at: archived_first_at.as_str(),
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
    let first_archive_path = state
        .config
        .archive_dir
        .join("failure-mixed-state-archive-a.sqlite.gz");
    let _ = fs::remove_file(&first_archive_path);
    fs::rename(&first_archive_original_path, &first_archive_path)
        .expect("move first mixed-state failure archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now') WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(first_archive_path.to_string_lossy().to_string())
    .bind(first_archive_original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark first mixed-state failure archive batch as materialized");

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-mixed-state-archive-b",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-mixed-state-second",
            occurred_at: archived_second_at.as_str(),
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
        }],
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("mixed-state failure bucket start epoch should be derivable");
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
    .bind("too_many_requests")
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed mixed-state materialized failure count row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary with materialized sibling archive part");
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
    .expect("fetch error distribution with materialized sibling archive part");
    assert!(
        error_distribution
            .items
            .iter()
            .any(|item| item.reason == "too_many_requests" && item.count == 2)
    );
}

#[tokio::test]
async fn archived_failure_fallback_includes_missing_categories_from_partially_materialized_bucket() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(10, 0, 0)
    .expect("valid archived partial failure hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived partial failure time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived partial failure time"),
    );

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-partial-category-archive",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1_i64,
                invoke_id: "failure-partial-category-first",
                occurred_at: archived_first_at.as_str(),
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
            },
            SeedInvocationArchiveBatchRow {
                id: 2_i64,
                invoke_id: "failure-partial-category-second",
                occurred_at: archived_second_at.as_str(),
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 20_i64,
                cost: 0.20_f64,
                ttfb_ms: Some(120.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some("upstream stream error"),
                failure_kind: Some("upstream_stream_error"),
                failure_class: Some("service_failure"),
                is_actionable: Some(1_i64),
            },
        ],
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("partial failure bucket start epoch should be derivable");
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
    .bind("too_many_requests")
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed partially materialized failure category");

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
async fn historical_perf_stats_include_unmaterialized_archived_hours() {
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

#[tokio::test]
async fn historical_perf_stats_fill_missing_samples_from_partially_materialized_archived_hours() {
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

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("partial perf bucket start epoch should be derivable");
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
    .bind(encode_approx_histogram(&histogram).expect("encode partial perf histogram"))
    .execute(&state.pool)
    .await
    .expect("seed partially materialized perf row");

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
async fn historical_perf_archive_delta_distinguishes_materialized_sibling_parts_and_stale_pending_overlap(
) {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(13, 0, 0)
    .expect("valid archived mixed-state perf hour");
    let archived_materialized_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("materialized mixed-state perf time"),
    );
    let archived_pending_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("first pending mixed-state perf time"),
    );
    let archived_pending_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(45))
            .expect("second pending mixed-state perf time"),
    );

    let materialized_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-mixed-state-archive-a",
        &[(
            1_i64,
            "perf-mixed-state-materialized",
            archived_materialized_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(200.0),
        )],
    )
    .await;
    let materialized_archive_path = state
        .config
        .archive_dir
        .join("perf-mixed-state-archive-a.sqlite.gz");
    let _ = fs::remove_file(&materialized_archive_path);
    fs::rename(&materialized_archive_original_path, &materialized_archive_path)
        .expect("move mixed-state materialized perf archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now') WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(materialized_archive_path.to_string_lossy().to_string())
    .bind(materialized_archive_original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark mixed-state materialized perf archive batch as materialized");

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-mixed-state-archive-b",
        &[
            (
                1_i64,
                "perf-mixed-state-pending-first",
                archived_pending_first_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "perf-mixed-state-pending-second",
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

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_materialized_at)
        .expect("mixed-state perf bucket start epoch should be derivable");
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

#[tokio::test]
async fn historical_timeseries_includes_unmaterialized_archived_hours_without_inline_repair() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(12, 0, 0)
    .expect("valid archived timeseries hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived timeseries success time"),
    );
    let archived_failed_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(35))
            .expect("archived timeseries failure time"),
    );

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
    let end = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("archived timeseries hour end"),
        Shanghai,
    );
    let Json(response) = fetch_timeseries_from_hourly_rollups(
        state,
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
    .expect("fetch historical timeseries with unmaterialized archived hour");

    let archived_point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("historical timeseries bucket should exist");
    assert_eq!(archived_point.total_count, 2);
    assert_eq!(archived_point.success_count, 1);
    assert_eq!(archived_point.failure_count, 1);
    assert_eq!(archived_point.total_tokens, 30);
    assert_f64_close(archived_point.total_cost, 0.30);
}

#[tokio::test]
async fn historical_timeseries_skips_archive_fallback_rows_already_counted_in_live_tail() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(12, 0, 0)
    .expect("valid overlapping archived timeseries hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived overlap timeseries success time"),
    );
    let archived_failed_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(35))
            .expect("archived overlap timeseries failure time"),
    );

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "timeseries-live-tail-overlap",
        &[
            SeedInvocationArchiveBatchRow {
                id: 2_i64,
                invoke_id: "timeseries-live-tail-overlap-success",
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
                invoke_id: "timeseries-live-tail-overlap-failed",
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
    .expect("seed overlapping timeseries shared hourly rollup cursor");

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
            "timeseries-live-tail-overlap-success",
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
            "timeseries-live-tail-overlap-failed",
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
        .expect("insert overlapping timeseries live row");
    }

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let end = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("overlapping archived timeseries hour end"),
        Shanghai,
    );
    let Json(response) = fetch_timeseries_from_hourly_rollups(
        state,
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
    .expect("fetch historical timeseries with overlapping archive/live tail");

    let archived_point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("overlapping historical timeseries bucket should exist");
    assert_eq!(archived_point.total_count, 2);
    assert_eq!(archived_point.success_count, 1);
    assert_eq!(archived_point.failure_count, 1);
    assert_eq!(archived_point.total_tokens, 30);
    assert_f64_close(archived_point.total_cost, 0.30);
}

#[tokio::test]
async fn historical_timeseries_skip_unreadable_materialized_archives() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(12, 0, 0)
    .expect("valid archived materialized timeseries hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived timeseries success time"),
    );
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
    .expect("mark timeseries archive as materialized");

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let end = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("archived timeseries hour end"),
        Shanghai,
    );
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
    .bind(start.timestamp())
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
    .expect("seed materialized timeseries rollup row");

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt materialized timeseries archive");

    let Json(response) = fetch_timeseries_from_hourly_rollups(
        state,
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
    .expect("fetch historical timeseries with unreadable materialized archive");

    let archived_point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("historical materialized timeseries bucket should exist");
    assert_eq!(archived_point.total_count, 1);
    assert_eq!(archived_point.success_count, 1);
    assert_eq!(archived_point.failure_count, 0);
    assert_eq!(archived_point.total_tokens, 10);
    assert_f64_close(archived_point.total_cost, 0.10);
}

#[tokio::test]
async fn historical_timeseries_skips_unreadable_replayed_legacy_archives() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(12, 0, 0)
    .expect("valid archived replayed timeseries hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived replayed timeseries success time"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "timeseries-replayed-corrupt-read-path",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "timeseries-replayed-corrupt-read-path-success",
            occurred_at: archived_success_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 10_i64,
            cost: 0.10_f64,
            ttfb_ms: Some(100.0),
            payload: Some(r#"{"promptCacheKey":"legacy-replayed"}"#),
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0_i64),
        }],
    )
    .await;

    insert_hourly_rollup_archive_replay_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &archive_path,
    )
    .await;

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let end = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("archived replayed timeseries hour end"),
        Shanghai,
    );
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
    .bind(start.timestamp())
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
    .expect("seed replayed timeseries rollup row");

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt replayed timeseries archive");

    let Json(response) = fetch_timeseries_from_hourly_rollups(
        state,
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
    .expect("fetch historical timeseries with unreadable replayed legacy archive");

    let archived_point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("historical replayed timeseries bucket should exist");
    assert_eq!(archived_point.total_count, 1);
    assert_eq!(archived_point.success_count, 1);
    assert_eq!(archived_point.failure_count, 0);
    assert_eq!(archived_point.total_tokens, 10);
    assert_f64_close(archived_point.total_cost, 0.10);
}

#[tokio::test]
async fn timeseries_hourly_backed_repairs_stale_archived_rollup_counts_before_querying() {
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
        "timeseries-hourly-stale-archived-rollup",
        &[
            (
                1_i64,
                "timeseries-hourly-archived-success",
                archived_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "timeseries-hourly-archived-pending",
                archived_pending_at.as_str(),
                SOURCE_PROXY,
                "pending",
                10_i64,
                0.10_f64,
                Some(110.0),
            ),
            (
                3_i64,
                "timeseries-hourly-archived-failed",
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

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let end = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("archived hour end"),
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
    .expect("fetch hourly-backed timeseries with stale archived rollup counts");

    let archived_point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("archived hourly bucket should exist");
    assert_eq!(archived_point.total_count, 3);
    assert_eq!(archived_point.success_count, 1);
    assert_eq!(archived_point.failure_count, 1);
    assert_eq!(archived_point.total_tokens, 30);
    assert_f64_close(archived_point.total_cost, 0.30);

    let rows = query_invocation_hourly_rollup_range(
        &state.pool,
        bucket_start_epoch,
        bucket_start_epoch + 3_600,
        InvocationSourceScope::ProxyOnly,
    )
    .await
    .expect("load repaired invocation hourly rollup row");
    let row = rows.first().expect("repaired hourly rollup row should exist");
    assert_eq!(row.total_count, 3);
    assert_eq!(row.success_count, 1);
    assert_eq!(row.failure_count, 1);

    let repair_marker_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2")
    .fetch_one(&state.pool)
    .await
    .expect("load summary rollup repair marker");
    assert_eq!(repair_marker_cursor, 1);
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

    let repair_err = crate::stats::ensure_invocation_summary_rollups_ready(&state.pool)
        .await
        .expect_err("missing archive should fail background summary repair");
    assert!(
        repair_err
            .to_string()
            .contains("completed invocation archive is missing"),
        "missing archive should bubble the repair cause"
    );

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("read-only all-time summary should fall back to current materialized rollups");
    assert_eq!(summary.total_count, 99);
    assert_eq!(summary.success_count, 99);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 990);
    assert!((summary.total_cost - 9.9).abs() < 1e-9);

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

    run_background_invocation_summary_rollup_repair(&state.pool).await;

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
async fn all_time_summary_backfill_preserves_overall_rollups_when_only_failure_marker_is_missing()
{
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(12))
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
    let archived_second_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(45))
            .expect("second archived success time"),
    );

    let first_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-failure-only-marker-backfill-a",
        &[
            (
                1_i64,
                "summary-all-failure-only-success",
                archived_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "summary-all-failure-only-failed",
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
    let first_archive_path = state
        .config
        .archive_dir
        .join("summary-all-failure-only-marker-backfill-a.sqlite.gz");
    let _ = fs::remove_file(&first_archive_path);
    fs::rename(&first_archive_original_path, &first_archive_path)
        .expect("move first archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(first_archive_path.to_string_lossy().to_string())
    .bind(first_archive_original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("update first archive batch path after move");

    let second_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-failure-only-marker-backfill-b",
        &[(
            3_i64,
            "summary-all-failure-only-second-archive-success",
            archived_second_success_at.as_str(),
            SOURCE_PROXY,
            "success",
            15_i64,
            0.15_f64,
            Some(140.0),
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
    .bind(3_i64)
    .bind(2_i64)
    .bind(1_i64)
    .bind(35_i64)
    .bind(0.45_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed correct overall summary rollup across both archives");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(first_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark first archive overall replay target as already repaired");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(second_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark second archive overall replay target as already repaired");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(second_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark second archive failure replay target as already repaired");

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
    .bind(0_i64)
    .execute(&state.pool)
    .await
    .expect("seed shared invocation hourly rollup cursor for failure-only backfill");

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary after failure-only marker backfill");

    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 35);
    assert!((summary.total_cost - 0.45).abs() < 1e-9);

    let rollup_total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired overall total count after failure-only marker backfill");
    assert_eq!(rollup_total_count, 3);

    let repaired_failure_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND target = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .bind(first_archive_path.to_string_lossy().to_string())
    .fetch_one(&state.pool)
    .await
    .expect("load repaired failure replay marker count after failure-only backfill");
    assert_eq!(repaired_failure_marker_count, 1);

    let first_archive_overall_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND target = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(first_archive_path.to_string_lossy().to_string())
    .fetch_one(&state.pool)
    .await
    .expect("load preserved overall replay marker count after failure-only backfill");
    assert_eq!(first_archive_overall_marker_count, 1);
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

    run_background_invocation_summary_rollup_repair(&state.pool).await;

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
async fn all_time_summary_repair_replays_existing_materialized_archives_when_others_are_pruned() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let pruned_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(470))
    .and_hms_opt(8, 0, 0)
    .expect("valid pruned local hour");
    let pruned_success_at = format_naive(
        pruned_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("pruned success time"),
    );
    let pruned_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-materialized-pruned-mixed",
        &[(
            1_i64,
            "summary-all-materialized-pruned-mixed-success",
            pruned_success_at.as_str(),
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
    .bind(pruned_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark pruned archive as materialized");

    let pruned_bucket_start_epoch = invocation_bucket_start_epoch(&pruned_success_at)
        .expect("derive pruned bucket epoch");
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
    .bind(pruned_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed preserved pruned archive rollup");

    fs::remove_file(&pruned_archive_path).expect("prune older materialized archive");

    let existing_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(440))
    .and_hms_opt(8, 0, 0)
    .expect("valid existing local hour");
    let existing_success_at = format_naive(
        existing_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("existing success time"),
    );
    let existing_failed_at = format_naive(
        existing_hour_local
            .checked_add_signed(ChronoDuration::minutes(15))
            .expect("existing failed time"),
    );
    let existing_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-materialized-existing-mixed",
        &[
            (
                1_i64,
                "summary-all-materialized-existing-success",
                existing_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(200.0),
            ),
            (
                2_i64,
                "summary-all-materialized-existing-failed",
                existing_failed_at.as_str(),
                SOURCE_PROXY,
                "failed",
                30_i64,
                0.30_f64,
                Some(300.0),
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
    .bind(existing_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark existing archive as materialized");

    let existing_bucket_start_epoch = invocation_bucket_start_epoch(&existing_success_at)
        .expect("derive existing bucket epoch");
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
    .bind(existing_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(2_i64)
    .bind(2_i64)
    .bind(0_i64)
    .bind(50_i64)
    .bind(0.50_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed stale existing materialized rollup");

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with mixed materialized archive states");

    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 60);
    assert!((summary.total_cost - 0.60).abs() < 1e-9);

    let existing_replayed_targets = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target IN (?2, ?3)",
    )
    .bind(existing_archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(&state.pool)
    .await
    .expect("load replay markers for existing materialized archive");
    assert_eq!(existing_replayed_targets, 2);

    let rollup_failure_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(failure_count), 0) FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(existing_bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired failure count for existing materialized archive");
    assert_eq!(rollup_failure_count, 1);
}

#[tokio::test]
async fn all_time_summary_read_path_skips_unreadable_materialized_archives() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived materialized summary hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived success time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-materialized-corrupt-read-path",
        &[(
            1_i64,
            "summary-materialized-corrupt-read-path-success",
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
    .expect("mark summary archive as materialized");

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_success_at)
        .expect("derive summary bucket epoch");
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
    .expect("seed materialized summary rollup row");

    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt materialized archive batch");

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with unreadable materialized archive");

    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 10);
    assert!((summary.total_cost - 0.10).abs() < 1e-9);
}

#[tokio::test]
async fn all_time_summary_read_path_skips_unreadable_replayed_legacy_archives() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived replayed summary hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived replayed success time"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "summary-replayed-corrupt-read-path",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "summary-replayed-corrupt-read-path-success",
            occurred_at: archived_success_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 10_i64,
            cost: 0.10_f64,
            ttfb_ms: Some(100.0),
            payload: Some(r#"{"promptCacheKey":"legacy-replayed"}"#),
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0_i64),
        }],
    )
    .await;

    insert_hourly_rollup_archive_replay_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &archive_path,
    )
    .await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_success_at)
        .expect("derive replayed summary bucket epoch");
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
    .expect("seed replayed summary rollup row");

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt replayed legacy archive batch");

    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with unreadable replayed legacy archive");

    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 10);
    assert!((summary.total_cost - 0.10).abs() < 1e-9);
}

#[tokio::test]
async fn all_time_summary_repair_restores_live_rows_in_boundary_hours_when_preserving_pruned_materialized_archives(
) {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let pruned_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(470))
    .and_hms_opt(8, 0, 0)
    .expect("valid pruned local hour");
    let pruned_success_at = format_naive(
        pruned_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("pruned success time"),
    );
    let pruned_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-boundary-live-pruned-materialized",
        &[(
            1_i64,
            "summary-all-boundary-live-pruned-success",
            pruned_success_at.as_str(),
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
    .bind(pruned_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark pruned archive as materialized");

    let pruned_bucket_start_epoch = invocation_bucket_start_epoch(&pruned_success_at)
        .expect("derive pruned bucket epoch");
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
    .bind(pruned_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed preserved pruned archive rollup");

    fs::remove_file(&pruned_archive_path).expect("prune older materialized archive");

    let boundary_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(440))
    .and_hms_opt(8, 0, 0)
    .expect("valid boundary local hour");
    let archived_boundary_at = format_naive(
        boundary_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived boundary success time"),
    );
    let live_boundary_at = format_naive(
        boundary_hour_local
            .checked_add_signed(ChronoDuration::minutes(45))
            .expect("live boundary success time"),
    );
    let boundary_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-boundary-live-existing-materialized",
        &[(
            2_i64,
            "summary-all-boundary-live-archived-success",
            archived_boundary_at.as_str(),
            SOURCE_PROXY,
            "success",
            20_i64,
            0.20_f64,
            Some(200.0),
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
    .bind(boundary_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark boundary archive as materialized");

    let boundary_bucket_start_epoch = invocation_bucket_start_epoch(&archived_boundary_at)
        .expect("derive boundary bucket epoch");
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
    .bind(boundary_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(2_i64)
    .bind(2_i64)
    .bind(0_i64)
    .bind(50_i64)
    .bind(0.50_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed pre-repair boundary rollup");

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
    .bind("summary-all-boundary-live-live-success")
    .bind(&live_boundary_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(30_i64)
    .bind(0.30_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert boundary live invocation row");

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
    .bind(10_i64)
    .execute(&state.pool)
    .await
    .expect("seed shared invocation hourly rollup cursor");

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with boundary live replay");

    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.success_count, 3);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 60);
    assert!((summary.total_cost - 0.60).abs() < 1e-9);

    let boundary_rollup_total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(boundary_bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired boundary rollup total count");
    assert_eq!(boundary_rollup_total_count, 2);

    let repair_live_cursor: i64 = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2_live_cursor")
    .fetch_one(&state.pool)
    .await
    .expect("load summary repair live cursor after boundary repair");
    assert_eq!(repair_live_cursor, 10);
}

#[tokio::test]
async fn all_time_summary_repair_rebuilds_non_materialized_archives_when_others_are_pruned() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let pruned_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(470))
    .and_hms_opt(8, 0, 0)
    .expect("valid pruned local hour");
    let pruned_success_at = format_naive(
        pruned_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("pruned success time"),
    );
    let pruned_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-nonmaterialized-pruned-mixed",
        &[(
            1_i64,
            "summary-all-nonmaterialized-pruned-success",
            pruned_success_at.as_str(),
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
    .bind(pruned_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark pruned archive as materialized");

    let pruned_bucket_start_epoch = invocation_bucket_start_epoch(&pruned_success_at)
        .expect("derive pruned bucket epoch");
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
    .bind(pruned_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(10_i64)
    .bind(0.10_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed preserved pruned archive rollup");

    fs::remove_file(&pruned_archive_path).expect("prune older materialized archive");

    let existing_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(440))
    .and_hms_opt(8, 0, 0)
    .expect("valid existing local hour");
    let existing_success_at = format_naive(
        existing_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("existing success time"),
    );
    let existing_failed_at = format_naive(
        existing_hour_local
            .checked_add_signed(ChronoDuration::minutes(15))
            .expect("existing failed time"),
    );
    let existing_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-nonmaterialized-existing-mixed",
        &[
            (
                1_i64,
                "summary-all-nonmaterialized-existing-success",
                existing_success_at.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(200.0),
            ),
            (
                2_i64,
                "summary-all-nonmaterialized-existing-failed",
                existing_failed_at.as_str(),
                SOURCE_PROXY,
                "failed",
                30_i64,
                0.30_f64,
                Some(300.0),
            ),
        ],
    )
    .await;

    let existing_bucket_start_epoch = invocation_bucket_start_epoch(&existing_success_at)
        .expect("derive existing bucket epoch");
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
    .bind(existing_bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(2_i64)
    .bind(2_i64)
    .bind(0_i64)
    .bind(50_i64)
    .bind(0.50_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed stale non-materialized rollup");

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch all-time summary with mixed non-materialized archive states");

    assert_eq!(summary.total_count, 3);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 1);
    assert_eq!(summary.total_tokens, 60);
    assert!((summary.total_cost - 0.60).abs() < 1e-9);

    let existing_replayed_targets = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target IN (?2, ?3)",
    )
    .bind(existing_archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(&state.pool)
    .await
    .expect("load replay markers for existing non-materialized archive");
    assert_eq!(existing_replayed_targets, 2);

    let rollup_total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(existing_bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired total count for existing non-materialized archive");
    assert_eq!(rollup_total_count, 2);

    let rollup_failure_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(failure_count), 0) FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(existing_bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load repaired failure count for existing non-materialized archive");
    assert_eq!(rollup_failure_count, 1);
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

    run_background_invocation_summary_rollup_repair(&state.pool).await;

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

    run_background_invocation_summary_rollup_repair(&state.pool).await;

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
async fn combined_totals_count_legacy_null_status_failures_when_only_downstream_error_exists() {
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
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(205_i64)
    .bind("summary-null-status-downstream-only")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(Option::<String>::None)
    .bind(6_i64)
    .bind(0.06_f64)
    .bind(
        json!({
            "downstreamErrorMessage": "downstream closed while streaming upstream response"
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy null-status downstream-only failure row");

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
    assert_eq!(totals.total_tokens, 6);
    assert_f64_close(totals.total_cost, 0.06);
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

#[tokio::test]
async fn combined_totals_count_legacy_http_200_failures_when_only_downstream_error_exists() {
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
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(206_i64)
    .bind("summary-http-200-downstream-only")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("http_200")
    .bind(8_i64)
    .bind(0.08_f64)
    .bind(
        json!({
            "downstreamErrorMessage": "socket closed after response"
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy http_200 downstream-only failure row");

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
    assert_eq!(totals.total_tokens, 8);
    assert_f64_close(totals.total_cost, 0.08);
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
fn resolve_failure_classification_keeps_completed_rows_with_failure_kind_as_failures() {
    let classification = resolve_failure_classification(
        Some("completed"),
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
