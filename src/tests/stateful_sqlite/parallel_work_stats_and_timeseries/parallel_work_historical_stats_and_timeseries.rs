use super::*;

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
async fn historical_perf_stats_include_unreplayed_full_hour_tail_without_inline_catch_up() {
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

#[tokio::test]
async fn historical_perf_archive_delta_distinguishes_materialized_sibling_parts_and_stale_pending_overlap()
 {
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
            401_i64,
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
    fs::rename(
        &materialized_archive_original_path,
        &materialized_archive_path,
    )
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

#[tokio::test]
async fn historical_perf_archive_delta_keeps_pending_stage_rows_when_materialized_sibling_archive_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

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

    let materialized_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-mixed-state-unreadable-archive-a",
        &[(
            1_i64,
            "perf-mixed-state-unreadable-materialized",
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
        .join("perf-mixed-state-unreadable-archive-a.sqlite.gz");
    let _ = fs::remove_file(&materialized_archive_path);
    fs::rename(
        &materialized_archive_original_path,
        &materialized_archive_path,
    )
    .expect("move unreadable mixed-state materialized perf archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now') WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(materialized_archive_path.to_string_lossy().to_string())
    .bind(materialized_archive_original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark unreadable mixed-state materialized perf archive batch as materialized");

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

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_materialized_at)
        .expect("unreadable mixed-state perf bucket start epoch should be derivable");
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
    .bind(
        encode_approx_histogram(&histogram).expect("encode unreadable mixed-state perf histogram"),
    )
    .execute(&state.pool)
    .await
    .expect("seed unreadable mixed-state materialized perf row");
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
async fn historical_perf_stats_skip_double_count_for_readable_materialized_archive_when_same_bucket_sibling_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

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

    let readable_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-same-bucket-unreadable-sibling-materialized",
        &[(
            1_i64,
            "perf-same-bucket-unreadable-sibling-materialized",
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
        .join("perf-same-bucket-unreadable-sibling-materialized.sqlite.gz");
    let _ = fs::remove_file(&readable_archive_path);
    fs::rename(&readable_archive_original_path, &readable_archive_path)
        .expect("move readable same-bucket perf archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(readable_archive_path.to_string_lossy().to_string())
    .bind(readable_archive_original_path.to_string_lossy().to_string())
    .bind(archived_readable_at.as_str())
    .bind(archived_readable_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark readable same-bucket perf archive batch as materialized");

    let unreadable_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-same-bucket-unreadable-sibling-broken",
        &[(
            1_i64,
            "perf-same-bucket-unreadable-sibling-broken",
            archived_unreadable_at.as_str(),
            SOURCE_PROXY,
            "success",
            20_i64,
            0.20_f64,
            Some(200.0),
        )],
    )
    .await;
    let unreadable_archive_path = state
        .config
        .archive_dir
        .join("perf-same-bucket-unreadable-sibling-broken.sqlite.gz");
    let _ = fs::remove_file(&unreadable_archive_path);
    fs::rename(&unreadable_archive_original_path, &unreadable_archive_path)
        .expect("move unreadable same-bucket perf archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(unreadable_archive_path.to_string_lossy().to_string())
    .bind(unreadable_archive_original_path.to_string_lossy().to_string())
    .bind(archived_unreadable_at.as_str())
    .bind(archived_unreadable_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark unreadable same-bucket perf archive batch as materialized");

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_readable_at)
        .expect("same-bucket perf bucket start epoch should be derivable");
    let mut histogram = empty_approx_histogram();
    add_approx_histogram_sample(&mut histogram, 100.0);
    add_approx_histogram_sample(&mut histogram, 200.0);
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
    .bind(300.0_f64)
    .bind(200.0_f64)
    .bind(encode_approx_histogram(&histogram).expect("encode same-bucket perf histogram"))
    .execute(&state.pool)
    .await
    .expect("seed same-bucket perf row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-bucket perf archive");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(perf_stats) = fetch_perf_stats(
        State(state),
        Query(PerfQuery {
            range: historical_range,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical perf stats with unreadable same-bucket materialized sibling archive");

    let upstream_first_byte = perf_stats
        .stages
        .iter()
        .find(|stage| stage.stage == "upstreamFirstByte")
        .expect("same-bucket perf stats should include materialized stage");
    assert_eq!(upstream_first_byte.count, 2);
    assert_f64_close(upstream_first_byte.avg_ms, 150.0);
    assert_f64_close(upstream_first_byte.max_ms, 200.0);
}

#[tokio::test]
async fn historical_perf_stats_skip_double_count_for_readable_materialized_archive_when_same_month_sibling_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

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

    let materialized_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-same-month-unreadable-sibling-materialized",
        &[(
            1_i64,
            "perf-same-month-unreadable-sibling-materialized",
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
        .join("perf-same-month-unreadable-sibling-materialized.sqlite.gz");
    let _ = fs::remove_file(&materialized_archive_path);
    fs::rename(
        &materialized_archive_original_path,
        &materialized_archive_path,
    )
    .expect("move readable materialized perf archive batch to a unique same-month path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(materialized_archive_path.to_string_lossy().to_string())
    .bind(materialized_archive_original_path.to_string_lossy().to_string())
    .bind(archived_materialized_at.as_str())
    .bind(archived_materialized_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark readable materialized same-month perf archive batch as materialized");

    let unreadable_archive_original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "perf-same-month-unreadable-sibling-broken",
        &[(
            1_i64,
            "perf-same-month-unreadable-sibling-broken",
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
        .join("perf-same-month-unreadable-sibling-broken.sqlite.gz");
    let _ = fs::remove_file(&unreadable_archive_path);
    fs::rename(&unreadable_archive_original_path, &unreadable_archive_path)
        .expect("move unreadable same-month perf sibling archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(unreadable_archive_path.to_string_lossy().to_string())
    .bind(unreadable_archive_original_path.to_string_lossy().to_string())
    .bind(archived_unreadable_at.as_str())
    .bind(archived_unreadable_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark unreadable same-month perf sibling archive batch as materialized");

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_materialized_at)
        .expect("same-month perf bucket start epoch should be derivable");
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
    .bind(encode_approx_histogram(&histogram).expect("encode same-month perf histogram"))
    .execute(&state.pool)
    .await
    .expect("seed readable materialized perf row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-month perf sibling archive");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(perf_stats) = fetch_perf_stats(
        State(state),
        Query(PerfQuery {
            range: historical_range,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch historical perf stats with unreadable same-month materialized sibling archive");

    let upstream_first_byte = perf_stats
        .stages
        .iter()
        .find(|stage| stage.stage == "upstreamFirstByte")
        .expect("same-month perf stats should include readable materialized stage");
    assert_eq!(upstream_first_byte.count, 1);
    assert_f64_close(upstream_first_byte.avg_ms, 100.0);
    assert_f64_close(upstream_first_byte.max_ms, 100.0);
}

#[tokio::test]
async fn historical_perf_read_path_skips_unreadable_pending_archives() {
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
    assert_f64_close(archived_point.non_success_cost, 0.20);
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

    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt replayed timeseries archive");

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
async fn historical_timeseries_read_path_skips_unreadable_pending_archives() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid unreadable pending timeseries hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("unreadable pending timeseries time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "timeseries-pending-corrupt-read-path",
        &[(
            1_i64,
            "timeseries-pending-corrupt-read-path-success",
            archived_success_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let end = local_naive_to_utc(
        archived_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .expect("unreadable pending timeseries hour end"),
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
    .expect("seed unreadable pending timeseries rollup row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        start.timestamp(),
        SOURCE_PROXY,
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable pending timeseries archive");

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
    .expect("fetch historical timeseries with unreadable pending archive");

    let archived_point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("historical unreadable pending timeseries bucket should exist");
    assert_eq!(archived_point.total_count, 1);
    assert_eq!(archived_point.success_count, 1);
    assert_eq!(archived_point.failure_count, 0);
    assert_eq!(archived_point.total_tokens, 10);
    assert_f64_close(archived_point.total_cost, 0.10);
}
