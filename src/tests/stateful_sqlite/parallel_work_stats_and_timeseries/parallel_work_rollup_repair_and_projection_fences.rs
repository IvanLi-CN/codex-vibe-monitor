use super::*;

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
    let row = rows
        .first()
        .expect("repaired hourly rollup row should exist");
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

    let summary = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(summary, Err(ApiError::Unavailable(_))),
        "an unreadable archive without replay coverage must not publish an inexact all-time snapshot"
    );

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
async fn summary_rollup_repair_preserves_shared_bucket_when_materialized_archive_is_missing() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(12))
    .and_hms_opt(8, 0, 0)
    .expect("valid shared archived hour");
    let first_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived time"),
    );
    let second_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived time"),
    );
    let first_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-shared-bucket-missing-materialized-a",
        &[(
            1_i64,
            "summary-shared-bucket-missing-a",
            first_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    let second_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-shared-bucket-missing-materialized-b",
        &[(
            2_i64,
            "summary-shared-bucket-missing-b",
            second_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            20_i64,
            0.20_f64,
            Some(120.0),
        )],
    )
    .await;
    for archive_path in [&first_archive_path, &second_archive_path] {
        sqlx::query(
            "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') \
             WHERE dataset = 'codex_invocations' AND file_path = ?1",
        )
        .bind(archive_path.to_string_lossy().to_string())
        .execute(&state.pool)
        .await
        .expect("mark shared-bucket archives as materialized");
    }

    let bucket_start_epoch = invocation_bucket_start_epoch(&first_occurred_at)
        .expect("shared bucket start epoch should be derivable");
    let empty_histogram =
        encode_approx_histogram(&empty_approx_histogram()).expect("encode empty histogram");
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
        VALUES (?1, ?2, 2, 2, 0, 30, 0.30, 0, 0, 0, ?3)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(empty_histogram)
    .execute(&state.pool)
    .await
    .expect("seed complete shared-bucket summary rollup");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay \
         (target, dataset, file_path, archive_sha256, replayed_at) \
         VALUES (?1, 'codex_invocations', ?2, 'stale-sha', datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(first_archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("seed stale marker for missing materialized archive");

    fs::remove_file(&first_archive_path).expect("remove first materialized archive");
    crate::stats::backfill_missing_invocation_summary_archive_rollups(&state.pool)
        .await
        .expect("defer shared-bucket repair without losing existing rollups");

    let rollup_total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("load preserved shared-bucket rollup");
    assert_eq!(rollup_total_count, 2);

    let first_marker: String = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay \
         WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(first_archive_path.to_string_lossy().to_string())
    .fetch_one(&state.pool)
    .await
    .expect("load preserved stale marker");
    assert_eq!(first_marker, "stale-sha");

    let second_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay \
         WHERE dataset = 'codex_invocations' AND file_path = ?1 \
           AND target IN (?2, ?3)",
    )
    .bind(second_archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(&state.pool)
    .await
    .expect("count deferred peer markers");
    assert_eq!(second_marker_count, 0);
}

#[tokio::test]
async fn all_time_summary_backfill_preserves_overall_rollups_when_only_failure_marker_is_missing() {
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
    let first_archive_sha: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(first_archive_path.to_string_lossy().to_string())
    .fetch_one(&state.pool)
    .await
    .expect("load first archive SHA for replay markers");
    let second_archive_sha: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(second_archive_path.to_string_lossy().to_string())
    .fetch_one(&state.pool)
    .await
    .expect("load second archive SHA for replay markers");

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
        INSERT INTO hourly_rollup_archive_replay
            (target, dataset, file_path, archive_sha256, replayed_at)
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(first_archive_path.to_string_lossy().to_string())
    .bind(&first_archive_sha)
    .execute(&state.pool)
    .await
    .expect("mark first archive overall replay target as already repaired");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay
            (target, dataset, file_path, archive_sha256, replayed_at)
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(second_archive_path.to_string_lossy().to_string())
    .bind(&second_archive_sha)
    .execute(&state.pool)
    .await
    .expect("mark second archive overall replay target as already repaired");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay
            (target, dataset, file_path, archive_sha256, replayed_at)
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(second_archive_path.to_string_lossy().to_string())
    .bind(&second_archive_sha)
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
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;

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

    let bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_success_at).expect("derive summary bucket epoch");
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
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;

    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt materialized archive batch");

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
    .expect("fetch all-time summary with unreadable materialized archive");

    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 10);
    assert!((summary.total_cost - 0.10).abs() < 1e-9);
}

#[tokio::test]
async fn all_time_summary_fails_closed_when_unreadable_replay_lacks_usage_and_account_coverage() {
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

    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt replayed legacy archive batch");

    let summary = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(summary, Err(ApiError::Unavailable(_))),
        "global replay alone cannot prove the account and usage dimensions of Summary"
    );
}

#[tokio::test]
async fn summary_refresh_revokes_all_time_when_new_archive_is_unreadable() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         VALUES ('summary-last-good-live', datetime('now'), 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
    )
    .execute(&state.pool)
    .await
    .expect("insert initial exact summary row");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate initial exact summary projection");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile initial exact all-time summary projection");

    let archived_at = format_naive(
        ((Utc::now() - ChronoDuration::days(10)) + ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-last-good-unreadable-refresh",
        &[(
            1_i64,
            "summary-last-good-unreadable-archive",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt newly discovered archive");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("an unreadable archive must publish unaffected rolling coverage");

    let response = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(response, Err(ApiError::Unavailable(_))),
        "an unreadable newly discovered archive must revoke stale all-time authority"
    );
}

#[tokio::test]
async fn all_time_stats_tolerate_unreadable_pending_archives_while_summary_fails_closed() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid unreadable pending summary hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("unreadable pending summary time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-pending-corrupt-read-path",
        &[(
            1_i64,
            "summary-pending-corrupt-read-path-success",
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
        .expect("derive unreadable pending summary bucket epoch");
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
    .expect("seed unreadable pending summary rollup row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt pending archive batch");

    let stats = fetch_stats(State(state.clone())).await;
    assert!(
        matches!(stats, Err(ApiError::Unavailable(_))),
        "stats must fail closed before an unreadable archive has a published proof"
    );

    let summary = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(summary, Err(ApiError::Unavailable(_))),
        "a materialized bucket marker is not an archive-level replay proof for the memory projection"
    );
}

#[tokio::test]
async fn summary_rollup_repair_best_effort_skips_unreadable_pending_archives() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(8, 0, 0)
    .expect("valid unreadable pending summary hour");
    let archived_success_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("unreadable pending success time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-pending-corrupt-best-effort",
        &[(
            1_i64,
            "summary-pending-corrupt-best-effort-success",
            archived_success_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt pending archive batch");

    crate::stats::ensure_invocation_summary_rollups_ready_best_effort(&state.pool)
        .await
        .expect("best-effort summary repair should skip unreadable pending archive");

    let repair_marker_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2")
    .fetch_optional(&state.pool)
    .await
    .expect("load unreadable pending repair marker");
    assert_eq!(repair_marker_cursor, None);
}

#[tokio::test]
async fn all_time_summary_repair_restores_live_rows_in_boundary_hours_when_preserving_pruned_materialized_archives()
 {
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

    let pruned_bucket_start_epoch =
        invocation_bucket_start_epoch(&pruned_success_at).expect("derive pruned bucket epoch");
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
    mark_summary_archive_replay_complete(&state.pool, &pruned_archive_path).await;

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

    let boundary_bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_boundary_at).expect("derive boundary bucket epoch");
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

    let repair_live_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind("codex_invocations_summary_rollup_v2_live_cursor")
            .fetch_one(&state.pool)
            .await
            .expect("load summary repair live cursor after boundary repair");
    assert_eq!(repair_live_cursor, 10);
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
async fn summary_rollup_repair_refreshes_stale_repair_live_cursor_from_shared_progress() {
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
