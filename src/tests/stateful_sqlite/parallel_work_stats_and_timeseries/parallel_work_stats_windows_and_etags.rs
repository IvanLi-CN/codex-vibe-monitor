use super::*;

#[tokio::test]
async fn parallel_work_stats_returns_not_modified_for_matching_etag() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut query_start = Utc::now();
    if query_start.timestamp().rem_euclid(60) >= 58 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        query_start = Utc::now();
    }
    let current_minute_epoch =
        align_reporting_bucket_epoch(query_start.timestamp(), 60, Shanghai).expect("align minute");
    let minute = Utc
        .timestamp_opt(current_minute_epoch - 3 * 60, 0)
        .single()
        .expect("minute");
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-etag-1",
        minute + ChronoDuration::seconds(10),
        "pck-etag",
    )
    .await;

    let query = || ParallelWorkStatsQuery {
        range: "7d".to_string(),
        bucket: Some("1m".to_string()),
        time_zone: Some("Asia/Shanghai".to_string()),
        upstream_account_id: None,
    };
    let first_response =
        fetch_parallel_work_stats_cached(State(state.clone()), HeaderMap::new(), Query(query()))
            .await
            .expect("fetch first parallel-work stats response");
    assert_eq!(first_response.status(), StatusCode::OK);
    let etag = first_response
        .headers()
        .get(axum::http::header::ETAG)
        .expect("etag header")
        .clone();
    let body = axum::body::to_bytes(first_response.into_body(), usize::MAX)
        .await
        .expect("read first response body");
    assert!(!body.is_empty());

    let mut headers = HeaderMap::new();
    headers.insert(axum::http::header::IF_NONE_MATCH, etag.clone());
    let second_response = fetch_parallel_work_stats_cached(State(state), headers, Query(query()))
        .await
        .expect("fetch cached parallel-work stats response");

    assert_eq!(second_response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        second_response.headers().get(axum::http::header::ETAG),
        Some(&etag)
    );
    let second_body = axum::body::to_bytes(second_response.into_body(), usize::MAX)
        .await
        .expect("read cached response body");
    assert!(second_body.is_empty());
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
            range: "7d".to_string(),
            bucket: Some("1m".to_string()),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
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
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-hour-tail-duplicate",
        fallback_hour + ChronoDuration::minutes(10),
        fallback_prompt_cache_key,
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-hour-tail-new",
        fallback_hour + ChronoDuration::minutes(20),
        "pck-live-tail",
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
            range: "30d".to_string(),
            bucket: Some("1h".to_string()),
            time_zone: Some("Asia/Kolkata".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    let hour_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(fallback_hour))
        .expect("fallback hour point");

    assert_eq!(hour_point.parallel_count, 2);
    assert_eq!(response.current.effective_time_zone, "Asia/Shanghai");
    assert!(response.current.time_zone_fallback);
    assert_eq!(response.hour30d.effective_time_zone, "Asia/Shanghai");
    assert!(response.hour30d.time_zone_fallback);
    assert_eq!(response.day_all.bucket_seconds, 3_600);
}

#[tokio::test]
async fn parallel_work_stats_zero_fills_current_page_period() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let observed_now = now - ChronoDuration::seconds(20);
    let current_minute_epoch =
        align_reporting_bucket_epoch(observed_now.timestamp(), 60, Shanghai).expect("align minute");
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
            range: "7d".to_string(),
            bucket: Some("1m".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    let inserted_empty_minute = inserted_current_minute - ChronoDuration::minutes(3);
    let current_minute_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(inserted_current_minute))
        .expect("inserted current minute point");
    let previous_minute_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(inserted_previous_minute))
        .expect("inserted previous minute point");
    let empty_minute_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(inserted_empty_minute))
        .expect("empty minute point");
    assert_eq!(current_minute_point.parallel_count, 1);
    assert_eq!(previous_minute_point.parallel_count, 1);
    assert_eq!(empty_minute_point.parallel_count, 0);
    assert!(
        response
            .current
            .points
            .iter()
            .all(|point| point.bucket_start != response.current.range_end)
    );
}

#[tokio::test]
async fn parallel_work_stats_current_day_bucket_aggregates_distinct_keys() {
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
        current_day_start,
        "pck-current-day",
        1,
    )
    .await;

    let Json(response) = fetch_parallel_work_stats(
        State(state),
        Query(ParallelWorkStatsQuery {
            range: "1mo".to_string(),
            bucket: Some("1d".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    let previous_day_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(previous_day_start))
        .expect("previous day point");
    let current_day_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(current_day_start))
        .expect("current day point");
    assert_eq!(previous_day_point.parallel_count, 2);
    assert_eq!(current_day_point.parallel_count, 1);
    assert_eq!(response.current.active_bucket_count, 2);
    assert_eq!(response.current.max_count, Some(2));
}

#[tokio::test]
async fn parallel_work_stats_account_scoped_day_bucket_aggregates_distinct_keys() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_day_start =
        local_midnight_utc(Utc::now().with_timezone(&Shanghai).date_naive(), Shanghai);
    let previous_day_start = current_day_start - ChronoDuration::days(1);

    insert_parallel_work_prompt_cache_upstream_account_hourly_row(
        &state.pool,
        previous_day_start,
        "pck-account-alpha",
        42,
        1,
    )
    .await;
    insert_parallel_work_prompt_cache_upstream_account_hourly_row(
        &state.pool,
        previous_day_start + ChronoDuration::hours(5),
        "pck-account-alpha",
        42,
        2,
    )
    .await;
    insert_parallel_work_prompt_cache_upstream_account_hourly_row(
        &state.pool,
        previous_day_start + ChronoDuration::hours(8),
        "pck-account-beta",
        42,
        1,
    )
    .await;
    insert_parallel_work_prompt_cache_upstream_account_hourly_row(
        &state.pool,
        previous_day_start + ChronoDuration::hours(10),
        "pck-other-account",
        17,
        5,
    )
    .await;
    insert_parallel_work_prompt_cache_upstream_account_hourly_row(
        &state.pool,
        current_day_start,
        "pck-account-current",
        42,
        1,
    )
    .await;

    let Json(response) = fetch_parallel_work_stats(
        State(state),
        Query(ParallelWorkStatsQuery {
            range: "1mo".to_string(),
            bucket: Some("1d".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("fetch account-scoped parallel-work stats");

    let previous_day_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(previous_day_start))
        .expect("previous day point");
    let current_day_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(current_day_start))
        .expect("current day point");
    assert_eq!(previous_day_point.parallel_count, 2);
    assert_eq!(current_day_point.parallel_count, 1);
    assert_eq!(response.current.active_bucket_count, 2);
    assert_eq!(response.current.max_count, Some(2));
    assert_eq!(response.current.active_minute_count, Some(0));
    assert_eq!(response.current.avg_count, None);
}

#[tokio::test]
async fn parallel_work_stats_current_day_bucket_includes_current_page_period() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_day_start =
        local_midnight_utc(Utc::now().with_timezone(&Shanghai).date_naive(), Shanghai);
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        current_day_start,
        "pck-today-only",
        1,
    )
    .await;

    let Json(response) = fetch_parallel_work_stats(
        State(state),
        Query(ParallelWorkStatsQuery {
            range: "1mo".to_string(),
            bucket: Some("1d".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch parallel-work stats");

    let current_day_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(current_day_start))
        .expect("current day point");
    assert_eq!(current_day_point.parallel_count, 1);
    assert_eq!(response.current.active_bucket_count, 1);
    assert_eq!(response.current.max_count, Some(1));
}

#[tokio::test]
async fn parallel_work_stats_hourly_rollups_include_aligned_leading_bucket() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let range_start = now - ChronoDuration::days(7);
    let leading_hour_epoch = align_reporting_bucket_epoch(range_start.timestamp(), 3_600, Shanghai)
        .expect("align leading hour");
    let leading_hour = Utc
        .timestamp_opt(leading_hour_epoch, 0)
        .single()
        .expect("valid leading hour");
    insert_parallel_work_prompt_cache_rollup_hourly_row(
        &state.pool,
        leading_hour,
        "pck-before-range",
        1,
    )
    .await;
    insert_parallel_work_invocation(
        &state.pool,
        "parallel-leading-after-range",
        range_start + ChronoDuration::seconds(1),
        "pck-after-range",
    )
    .await;
    let Json(response) = fetch_parallel_work_stats_at(
        State(state),
        Query(ParallelWorkStatsQuery {
            range: "7d".to_string(),
            bucket: Some("1h".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
        now,
    )
    .await
    .expect("fetch parallel-work stats");

    let leading_point = response
        .current
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(leading_hour))
        .expect("leading hour point");
    assert_eq!(leading_point.parallel_count, 1);
    assert_eq!(response.current.active_bucket_count, 1);
}
