pub(crate) struct FailurePairSpec<'a> {
    pub(crate) batch_name: &'a str,
    pub(crate) first_invoke_id: &'a str,
    pub(crate) second_invoke_id: &'a str,
    pub(crate) local_hour: u32,
    pub(crate) second_in_next_hour: bool,
    pub(crate) second_minute: i64,
    pub(crate) second_error_message: &'a str,
    pub(crate) second_failure_kind: &'a str,
}

pub(crate) async fn seed_failure_pair_archive(
    state: &Arc<AppState>,
    spec: FailurePairSpec<'_>,
) -> (String, String) {
    let first_hour = (Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(10))
        .and_hms_opt(spec.local_hour, 0, 0)
        .expect("valid failure-pair hour");
    let second_hour = if spec.second_in_next_hour {
        first_hour + ChronoDuration::hours(1)
    } else {
        first_hour
    };
    let first_at = format_naive(first_hour + ChronoDuration::minutes(5));
    let second_at = format_naive(second_hour + ChronoDuration::minutes(spec.second_minute));
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        spec.batch_name,
        &[
            SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: spec.first_invoke_id,
                occurred_at: &first_at,
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 10,
                cost: 0.10,
                ttfb_ms: Some(100.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some("HTTP 429 too many requests"),
                failure_kind: Some("upstream_response_failed"),
                failure_class: Some("service_failure"),
                is_actionable: Some(1),
            },
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: spec.second_invoke_id,
                occurred_at: &second_at,
                source: SOURCE_PROXY,
                status: "failed",
                total_tokens: 20,
                cost: 0.20,
                ttfb_ms: Some(120.0),
                payload: Some("{}"),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: Some(spec.second_error_message),
                failure_kind: Some(spec.second_failure_kind),
                failure_class: Some("service_failure"),
                is_actionable: Some(1),
            },
        ],
    )
    .await;
    (first_at, second_at)
}

pub(crate) async fn insert_failure_rollup(
    pool: &SqlitePool,
    occurred_at: &str,
    failure_count: i64,
) {
    let bucket = invocation_bucket_start_epoch(occurred_at).expect("failure rollup bucket");
    sqlx::query(
        r#"
        INSERT INTO invocation_failure_rollup_hourly (
            bucket_start_epoch, source, failure_class, is_actionable,
            error_category, failure_count, updated_at
        ) VALUES (?1, ?2, 'service_failure', 1, 'too_many_requests', ?3, datetime('now'))
        "#,
    )
    .bind(bucket)
    .bind(SOURCE_PROXY)
    .bind(failure_count)
    .execute(pool)
    .await
    .expect("seed archived failure rollup");
}

pub(crate) async fn assert_archived_failure_views(
    state: Arc<AppState>,
    expected_total: i64,
    expected_too_many_requests: Option<i64>,
) {
    let range = format!("{}d", state.config.invocation_max_days + 30);
    let summary = fetch_test_failure_summary(state.clone(), &range).await;
    assert_eq!(summary.total_failures, expected_total);
    assert_eq!(summary.service_failure_count, expected_total);
    assert_eq!(summary.client_failure_count, 0);
    assert_eq!(summary.client_abort_count, 0);
    assert_eq!(summary.actionable_failure_count, expected_total);
    let Json(errors) = fetch_error_distribution(
        State(state),
        Query(ErrorQuery {
            range,
            top: None,
            scope: Some("service".to_string()),
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch archived error distribution");
    if let Some(count) = expected_too_many_requests {
        assert!(
            errors
                .items
                .iter()
                .any(|item| item.reason == "too_many_requests" && item.count == count)
        );
    } else {
        assert_eq!(
            errors.items.iter().map(|item| item.count).sum::<i64>(),
            expected_total
        );
    }
}

pub(crate) struct FailureArchivePartFixture<'a> {
    pub(crate) batch_name: &'a str,
    pub(crate) id: i64,
    pub(crate) invoke_id: &'a str,
    pub(crate) occurred_at: &'a str,
    pub(crate) tokens: i64,
    pub(crate) cost: f64,
    pub(crate) error_message: &'a str,
    pub(crate) failure_kind: &'a str,
}

pub(crate) async fn seed_failure_archive_part(
    state: &Arc<AppState>,
    fixture: FailureArchivePartFixture<'_>,
) -> PathBuf {
    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        fixture.batch_name,
        &[SeedInvocationArchiveBatchRow {
            id: fixture.id,
            invoke_id: fixture.invoke_id,
            occurred_at: fixture.occurred_at,
            source: SOURCE_PROXY,
            status: "failed",
            total_tokens: fixture.tokens,
            cost: fixture.cost,
            ttfb_ms: Some(if fixture.tokens == 10 { 100.0 } else { 120.0 }),
            payload: Some("{}"),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: Some(fixture.error_message),
            failure_kind: Some(fixture.failure_kind),
            failure_class: Some("service_failure"),
            is_actionable: Some(1),
        }],
    )
    .await
}

pub(crate) async fn seed_materialized_failure_archive(
    state: &Arc<AppState>,
    batch_name: &str,
    occurred_at: &str,
    tokens: i64,
    cost: f64,
    error_message: &str,
    failure_kind: &str,
) -> PathBuf {
    let original_path = seed_failure_archive_part(
        state,
        FailureArchivePartFixture {
            batch_name,
            id: 1,
            invoke_id: batch_name,
            occurred_at,
            tokens,
            cost,
            error_message,
            failure_kind,
        },
    )
    .await;
    let archive_path = state
        .config
        .archive_dir
        .join(format!("{batch_name}.sqlite.gz"));
    let _ = fs::remove_file(&archive_path);
    fs::rename(&original_path, &archive_path).expect("move materialized failure archive");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, \
         historical_rollups_materialized_at = datetime('now'), \
         coverage_start_at = ?3, coverage_end_at = ?3 \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(original_path.to_string_lossy().to_string())
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("mark failure archive materialized");
    archive_path
}

async fn seed_same_month_summary_rollups(
    state: &Arc<AppState>,
    first_at: &str,
    second_at: &str,
    paths: [&Path; 2],
) {
    for (occurred_at, tokens, cost) in [(first_at, 10, 0.10), (second_at, 20, 0.20)] {
        let bucket = invocation_bucket_start_epoch(occurred_at).expect("same-month rollup bucket");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly \
             (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost) \
             VALUES (?1, ?2, 1, 1, 0, ?3, ?4)",
        )
        .bind(bucket)
        .bind(SOURCE_PROXY)
        .bind(tokens)
        .bind(cost)
        .execute(&state.pool)
        .await
        .expect("seed same-month summary rollup");
        insert_materialized_rollup_bucket_marker(
            &state.pool,
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            bucket,
            SOURCE_PROXY,
        )
        .await;
    }
    for path in paths {
        mark_summary_archive_replay_complete(&state.pool, path).await;
    }
}

async fn assert_same_month_first_hour_timeseries(state: Arc<AppState>, hour: NaiveDateTime) {
    let start = local_naive_to_utc(hour, Shanghai);
    let end = start + ChronoDuration::hours(1);
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
    .expect("fetch same-month first-hour timeseries");
    let point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("same-month first-hour bucket");
    assert_eq!(point.total_count, 1);
    assert_eq!(point.success_count, 1);
    assert_eq!(point.failure_count, 0);
    assert_eq!(point.total_tokens, 10);
    assert_f64_close(point.total_cost, 0.10);
}

#[tokio::test]
pub(crate) async fn all_time_summary_skips_double_count_for_readable_materialized_archive_when_same_month_sibling_is_unreadable()
 {
    let state = archive_retention_test_state().await;

    let archived_materialized_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(7, 0, 0)
    .expect("valid archived readable materialized summary hour");
    let archived_unreadable_hour_local = archived_materialized_hour_local
        .checked_add_signed(ChronoDuration::hours(2))
        .expect("same-month unreadable summary sibling hour");
    let archived_materialized_at = format_naive(
        archived_materialized_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("readable materialized summary archived time"),
    );
    let archived_unreadable_at = format_naive(
        archived_unreadable_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("unreadable summary sibling archived time"),
    );

    let materialized_archive_path = seed_materialized_summary_archive(
        &state,
        "summary-same-month-unreadable-sibling-materialized",
        &archived_materialized_at,
        10,
        0.10,
        100.0,
    )
    .await;
    let unreadable_archive_path = seed_materialized_summary_archive(
        &state,
        "summary-same-month-unreadable-sibling-broken",
        &archived_unreadable_at,
        20,
        0.20,
        120.0,
    )
    .await;

    seed_same_month_summary_rollups(
        &state,
        &archived_materialized_at,
        &archived_unreadable_at,
        [&materialized_archive_path, &unreadable_archive_path],
    )
    .await;
    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-month summary sibling archive");
    let summary = fetch_test_summary(state.clone(), "all").await;
    assert_two_success_summary(&summary);
    assert_same_month_first_hour_timeseries(state, archived_materialized_hour_local).await;
}

#[tokio::test]
pub(crate) async fn archived_failure_fallback_skips_already_materialized_archive_buckets() {
    let state = archive_retention_test_state().await;
    let (first_at, _) = seed_failure_pair_archive(
        &state,
        FailurePairSpec {
            batch_name: "failure-partial-materialized-archive",
            first_invoke_id: "failure-partial-materialized-first",
            second_invoke_id: "failure-partial-materialized-second",
            local_hour: 8,
            second_in_next_hour: true,
            second_minute: 5,
            second_error_message: "upstream stream error",
            second_failure_kind: "upstream_stream_error",
        },
    )
    .await;
    insert_failure_rollup(&state.pool, &first_at, 1).await;
    assert_archived_failure_views(state, 2, None).await;
}

#[tokio::test]
pub(crate) async fn archived_failure_fallback_includes_missing_rows_from_partially_materialized_count()
 {
    let state = archive_retention_test_state().await;
    let (first_at, _) = seed_failure_pair_archive(
        &state,
        FailurePairSpec {
            batch_name: "failure-partial-count-archive",
            first_invoke_id: "failure-partial-count-first",
            second_invoke_id: "failure-partial-count-second",
            local_hour: 9,
            second_in_next_hour: false,
            second_minute: 25,
            second_error_message: "HTTP 429 too many requests",
            second_failure_kind: "upstream_response_failed",
        },
    )
    .await;
    insert_failure_rollup(&state.pool, &first_at, 1).await;
    assert_archived_failure_views(state, 2, Some(2)).await;
}

#[tokio::test]
pub(crate) async fn archived_failure_fallback_aggregates_missing_rows_across_archive_parts() {
    let state = archive_retention_test_state().await;

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

    insert_failure_rollup(&state.pool, &archived_first_at, 1).await;
    assert_archived_failure_views(state, 2, Some(2)).await;
}

#[tokio::test]
pub(crate) async fn archived_failure_fallback_keeps_unmaterialized_rows_when_sibling_archive_part_is_materialized()
 {
    let state = archive_retention_test_state().await;

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

    insert_failure_rollup(&state.pool, &archived_first_at, 1).await;
    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("mixed-state failure bucket start epoch should be derivable");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    assert_archived_failure_views(state, 2, Some(2)).await;
}

#[tokio::test]
pub(crate) async fn archived_failure_fallback_keeps_unmaterialized_rows_when_materialized_sibling_archive_is_unreadable()
 {
    let state = archive_retention_test_state().await;

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

    insert_failure_rollup(&state.pool, &archived_first_at, 1).await;
    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_first_at)
        .expect("unreadable mixed-state failure bucket start epoch should be derivable");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&first_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable mixed-state failure archive batch");

    assert_archived_failure_views(state, 2, Some(2)).await;
}

use super::*;
