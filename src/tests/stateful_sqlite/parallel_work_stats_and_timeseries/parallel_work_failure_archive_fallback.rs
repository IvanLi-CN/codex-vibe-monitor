use super::*;

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
    let total_distribution_count: i64 =
        error_distribution.items.iter().map(|item| item.count).sum();
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
            id: 301_i64,
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
async fn archived_failure_fallback_keeps_unmaterialized_rows_when_sibling_archive_part_is_materialized()
 {
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
async fn archived_failure_fallback_keeps_unmaterialized_rows_when_materialized_sibling_archive_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(9, 0, 0)
    .expect("valid archived unreadable mixed-state failure hour");
    let archived_first_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("first archived unreadable mixed-state failure time"),
    );
    let archived_second_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("second archived unreadable mixed-state failure time"),
    );

    let first_archive_original_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-mixed-state-unreadable-archive-a",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-mixed-state-unreadable-first",
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
        .join("failure-mixed-state-unreadable-archive-a.sqlite.gz");
    let _ = fs::remove_file(&first_archive_path);
    fs::rename(&first_archive_original_path, &first_archive_path)
        .expect("move first unreadable mixed-state failure archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now') WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(first_archive_path.to_string_lossy().to_string())
    .bind(first_archive_original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark first unreadable mixed-state failure archive batch as materialized");

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-mixed-state-unreadable-archive-b",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-mixed-state-unreadable-second",
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
        .expect("unreadable mixed-state failure bucket start epoch should be derivable");
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
    .expect("seed unreadable mixed-state materialized failure count row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&first_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable mixed-state failure archive batch");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary with unreadable materialized sibling archive part");
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
    .expect("fetch error distribution with unreadable materialized sibling archive part");
    assert!(
        error_distribution
            .items
            .iter()
            .any(|item| item.reason == "too_many_requests" && item.count == 2)
    );
}

#[tokio::test]
async fn archived_failure_fallback_skips_double_count_for_readable_materialized_archive_when_same_bucket_sibling_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

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

    let readable_archive_original_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-same-bucket-unreadable-sibling-materialized",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-same-bucket-unreadable-sibling-materialized",
            occurred_at: archived_readable_at.as_str(),
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
    let readable_archive_path = state
        .config
        .archive_dir
        .join("failure-same-bucket-unreadable-sibling-materialized.sqlite.gz");
    let _ = fs::remove_file(&readable_archive_path);
    fs::rename(&readable_archive_original_path, &readable_archive_path)
        .expect("move readable same-bucket failure archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(readable_archive_path.to_string_lossy().to_string())
    .bind(readable_archive_original_path.to_string_lossy().to_string())
    .bind(archived_readable_at.as_str())
    .bind(archived_readable_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark readable same-bucket failure archive batch as materialized");

    let unreadable_archive_original_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-same-bucket-unreadable-sibling-broken",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-same-bucket-unreadable-sibling-broken",
            occurred_at: archived_unreadable_at.as_str(),
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
    let unreadable_archive_path = state
        .config
        .archive_dir
        .join("failure-same-bucket-unreadable-sibling-broken.sqlite.gz");
    let _ = fs::remove_file(&unreadable_archive_path);
    fs::rename(&unreadable_archive_original_path, &unreadable_archive_path)
        .expect("move unreadable same-bucket failure archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(unreadable_archive_path.to_string_lossy().to_string())
    .bind(unreadable_archive_original_path.to_string_lossy().to_string())
    .bind(archived_unreadable_at.as_str())
    .bind(archived_unreadable_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark unreadable same-bucket failure archive batch as materialized");

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_readable_at)
        .expect("same-bucket failure bucket start epoch should be derivable");
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
    .bind(2_i64)
    .execute(&state.pool)
    .await
    .expect("seed same-bucket failure count row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-bucket failure archive");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary with unreadable same-bucket materialized sibling archive");
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
    .expect("fetch error distribution with unreadable same-bucket materialized sibling archive");
    let too_many_requests = error_distribution
        .items
        .iter()
        .find(|item| item.reason == "too_many_requests")
        .expect("same-bucket failure distribution should keep materialized category count");
    assert_eq!(too_many_requests.count, 2);
}

#[tokio::test]
async fn historical_failure_read_path_skips_unreadable_pending_archives() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

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

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_failed_at)
        .expect("derive unreadable pending failure bucket epoch");
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
    .expect("seed unreadable pending failure count row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable pending failure archive");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary with unreadable pending archive");
    assert_eq!(failure_summary.total_failures, 1);
    assert_eq!(failure_summary.service_failure_count, 1);
    assert_eq!(failure_summary.actionable_failure_count, 1);

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
    .expect("fetch error distribution with unreadable pending archive");
    assert!(
        error_distribution
            .items
            .iter()
            .any(|item| item.reason == "too_many_requests" && item.count == 1)
    );
}

#[tokio::test]
async fn archived_failure_fallback_skips_double_count_for_readable_materialized_archive_when_same_month_sibling_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

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

    let materialized_archive_original_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-same-month-unreadable-sibling-materialized",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-same-month-unreadable-sibling-materialized",
            occurred_at: archived_materialized_at.as_str(),
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
    let materialized_archive_path = state
        .config
        .archive_dir
        .join("failure-same-month-unreadable-sibling-materialized.sqlite.gz");
    let _ = fs::remove_file(&materialized_archive_path);
    fs::rename(
        &materialized_archive_original_path,
        &materialized_archive_path,
    )
    .expect("move readable materialized failure archive batch to a unique same-month path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(materialized_archive_path.to_string_lossy().to_string())
    .bind(materialized_archive_original_path.to_string_lossy().to_string())
    .bind(archived_materialized_at.as_str())
    .bind(archived_materialized_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark readable materialized same-month failure archive batch as materialized");

    let unreadable_archive_original_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "failure-same-month-unreadable-sibling-broken",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "failure-same-month-unreadable-sibling-broken",
            occurred_at: archived_unreadable_at.as_str(),
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
        }],
    )
    .await;
    let unreadable_archive_path = state
        .config
        .archive_dir
        .join("failure-same-month-unreadable-sibling-broken.sqlite.gz");
    let _ = fs::remove_file(&unreadable_archive_path);
    fs::rename(&unreadable_archive_original_path, &unreadable_archive_path)
        .expect("move unreadable same-month failure sibling archive batch to a unique path");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), coverage_start_at = ?3, coverage_end_at = ?4 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(unreadable_archive_path.to_string_lossy().to_string())
    .bind(unreadable_archive_original_path.to_string_lossy().to_string())
    .bind(archived_unreadable_at.as_str())
    .bind(archived_unreadable_at.as_str())
    .execute(&state.pool)
    .await
    .expect("mark unreadable same-month failure sibling archive batch as materialized");

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_materialized_at)
        .expect("same-month failure bucket start epoch should be derivable");
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
    .expect("seed readable materialized failure count row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-month failure sibling archive");

    let historical_range = format!("{}d", state.config.invocation_max_days + 30);
    let Json(failure_summary) = fetch_failure_summary(
        State(state.clone()),
        Query(FailureSummaryQuery {
            range: historical_range.clone(),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch failure summary with unreadable same-month materialized sibling archive");
    assert_eq!(failure_summary.total_failures, 1);
    assert_eq!(failure_summary.service_failure_count, 1);
    assert_eq!(failure_summary.actionable_failure_count, 1);

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
    .expect("fetch error distribution with unreadable same-month materialized sibling archive");
    let too_many_requests = error_distribution
        .items
        .iter()
        .find(|item| item.reason == "too_many_requests")
        .expect("same-month failure distribution should keep readable materialized category");
    assert_eq!(too_many_requests.count, 1);
}

#[tokio::test]
async fn archived_failure_fallback_includes_missing_categories_from_partially_materialized_bucket()
{
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
