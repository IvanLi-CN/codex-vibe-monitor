pub(crate) struct ParallelTimeseriesInvocationRollupSpec<'a> {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_byte_samples: &'a [f64],
    pub(crate) context: &'a str,
}

pub(crate) fn archived_local_hour(days_ago: i64, hour: u32) -> NaiveDateTime {
    (Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(days_ago))
        .and_hms_opt(hour, 0, 0)
        .expect("valid archived local hour")
}

pub(crate) fn local_minute(local_hour: NaiveDateTime, minute: i64) -> String {
    format_naive(
        local_hour
            .checked_add_signed(ChronoDuration::minutes(minute))
            .expect("valid archived invocation time"),
    )
}

pub(crate) async fn insert_parallel_timeseries_invocation_rollup(
    pool: &SqlitePool,
    spec: ParallelTimeseriesInvocationRollupSpec<'_>,
) {
    let mut histogram = empty_approx_histogram();
    for sample in spec.first_byte_samples {
        add_approx_histogram_sample(&mut histogram, *sample);
    }
    let sample_count = spec.first_byte_samples.len() as i64;
    let sample_sum = spec.first_byte_samples.iter().sum::<f64>();
    let sample_max = spec
        .first_byte_samples
        .iter()
        .copied()
        .reduce(f64::max)
        .unwrap_or(0.0);
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch, source, total_count, success_count, failure_count,
            total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms,
            first_byte_max_ms, first_byte_histogram
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(spec.bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(spec.total_count)
    .bind(spec.success_count)
    .bind(spec.failure_count)
    .bind(spec.total_tokens)
    .bind(spec.total_cost)
    .bind(sample_count)
    .bind(sample_sum)
    .bind(sample_max)
    .bind(encode_approx_histogram(&histogram).expect("encode first-byte histogram"))
    .execute(pool)
    .await
    .unwrap_or_else(|error| panic!("{}: {error}", spec.context));
}

pub(crate) async fn insert_invocation_failure_rollup(
    pool: &SqlitePool,
    bucket_start_epoch: i64,
    error_category: &str,
    failure_count: i64,
) {
    sqlx::query(
        r#"
        INSERT INTO invocation_failure_rollup_hourly (
            bucket_start_epoch, source, failure_class, is_actionable,
            error_category, failure_count, updated_at
        ) VALUES (?1, ?2, 'service_failure', 1, ?3, ?4, datetime('now'))
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(error_category)
    .bind(failure_count)
    .execute(pool)
    .await
    .expect("seed invocation failure rollup");
}

pub(crate) async fn mark_archive_materialized(pool: &SqlitePool, archive_path: &Path) {
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("mark invocation archive as materialized");
}

pub(crate) async fn set_hourly_rollup_progress(pool: &SqlitePool, dataset: &str, cursor_id: i64) {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = excluded.cursor_id,
            updated_at = datetime('now')
        "#,
    )
    .bind(dataset)
    .bind(cursor_id)
    .execute(pool)
    .await
    .expect("set hourly rollup progress");
}

pub(crate) async fn fetch_archived_hour_timeseries(
    state: Arc<AppState>,
    local_hour: NaiveDateTime,
) -> TimeseriesResponse {
    let start = local_naive_to_utc(local_hour, Shanghai);
    let end = local_naive_to_utc(local_hour + ChronoDuration::hours(1), Shanghai);
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
    .expect("fetch archived hourly timeseries");
    response
}

pub(crate) fn assert_summary_totals(
    summary: &StatsResponse,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
) {
    assert_eq!(summary.total_count, total_count);
    assert_eq!(summary.success_count, success_count);
    assert_eq!(summary.failure_count, failure_count);
    assert_eq!(summary.total_tokens, total_tokens);
    assert_f64_close(summary.total_cost, total_cost);
}

pub(crate) fn assert_timeseries_totals(
    response: &TimeseriesResponse,
    bucket_start: DateTime<Utc>,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
) {
    let point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(bucket_start))
        .expect("expected hourly timeseries bucket");
    assert_eq!(point.total_count, total_count);
    assert_eq!(point.success_count, success_count);
    assert_eq!(point.failure_count, failure_count);
    assert_eq!(point.total_tokens, total_tokens);
    assert_f64_close(point.total_cost, total_cost);
}

async fn insert_legacy_archive_replay_marker(pool: &SqlitePool, target: &str, path: &Path) {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(target)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("insert legacy archive replay marker");
}

async fn archive_replay_marker_count(pool: &SqlitePool, target: &str, path: &Path) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay \
         WHERE dataset = 'codex_invocations' AND target = ?1 AND file_path = ?2",
    )
    .bind(target)
    .bind(path.to_string_lossy().to_string())
    .fetch_one(pool)
    .await
    .expect("load archive replay marker count")
}

pub(crate) async fn summary_replay_target_count(pool: &SqlitePool, path: &Path) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay \
         WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target IN (?2, ?3)",
    )
    .bind(path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_one(pool)
    .await
    .expect("load summary replay target count")
}

pub(crate) async fn hourly_rollup_progress(pool: &SqlitePool, dataset: &str) -> Option<i64> {
    sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
        .bind(dataset)
        .fetch_optional(pool)
        .await
        .expect("load hourly rollup progress")
}

pub(crate) async fn rollup_total_count(pool: &SqlitePool, bucket_start_epoch: Option<i64>) -> i64 {
    match bucket_start_epoch {
        Some(bucket) => {
            sqlx::query_scalar(
                "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly \
             WHERE bucket_start_epoch = ?1",
            )
            .bind(bucket)
            .fetch_one(pool)
            .await
        }
        None => {
            sqlx::query_scalar("SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly")
                .fetch_one(pool)
                .await
        }
    }
    .expect("load invocation rollup total count")
}

#[tokio::test]
pub(crate) async fn historical_timeseries_skips_unreadable_replayed_legacy_archives() {
    let state = archive_retention_test_state().await;
    let archived_hour_local = archived_local_hour(10, 12);
    let archived_success_at = local_minute(archived_hour_local, 5);
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
            context: "seed replayed timeseries rollup row",
        },
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt replayed timeseries archive");

    let response = fetch_archived_hour_timeseries(state, archived_hour_local).await;

    assert_timeseries_totals(&response, start, 1, 1, 0, 10, 0.10);
}

#[tokio::test]
pub(crate) async fn historical_timeseries_read_path_skips_unreadable_pending_archives() {
    let state = archive_retention_test_state().await;
    let archived_hour_local = archived_local_hour(10, 8);
    let archived_success_at = local_minute(archived_hour_local, 5);
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
            context: "seed unreadable pending timeseries rollup row",
        },
    )
    .await;
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        start.timestamp(),
        SOURCE_PROXY,
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable pending timeseries archive");

    let response = fetch_archived_hour_timeseries(state, archived_hour_local).await;

    assert_timeseries_totals(&response, start, 1, 1, 0, 10, 0.10);
}

#[tokio::test]
pub(crate) async fn timeseries_hourly_backed_repairs_stale_archived_rollup_counts_before_querying()
{
    let state = archive_retention_test_state().await;
    let archived_hour_local = archived_local_hour(10, 8);
    let archived_success_at = local_minute(archived_hour_local, 5);
    let archived_pending_at = local_minute(archived_hour_local, 15);
    let archived_failed_at = local_minute(archived_hour_local, 25);

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
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch,
            total_count: 3,
            success_count: 1,
            failure_count: 2,
            total_tokens: 30,
            total_cost: 0.30,
            first_byte_samples: &[],
            context: "seed stale archived invocation rollup counts",
        },
    )
    .await;

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let start = local_naive_to_utc(archived_hour_local, Shanghai);
    let response = fetch_archived_hour_timeseries(state.clone(), archived_hour_local).await;

    assert_timeseries_totals(&response, start, 3, 1, 1, 30, 0.30);

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
pub(crate) async fn all_time_summary_missing_archive_does_not_mark_repair_complete() {
    let state = archive_retention_test_state().await;
    let archived_success_at = local_minute(archived_local_hour(10, 8), 5);
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
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch,
            total_count: 99,
            success_count: 99,
            failure_count: 0,
            total_tokens: 990,
            total_cost: 9.9,
            first_byte_samples: &[],
            context: "seed stale invocation rollup counts before failed repair",
        },
    )
    .await;

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
pub(crate) async fn all_time_summary_missing_summary_markers_do_not_replay_materialized_archives() {
    let state = archive_retention_test_state().await;
    let archived_hour_local = archived_local_hour(10, 8);
    let archived_success_at = local_minute(archived_hour_local, 5);
    let archived_failed_at = local_minute(archived_hour_local, 25);

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

    mark_archive_materialized(&state.pool, &archive_path).await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_success_at)
        .expect("bucket start epoch should be derivable");
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch,
            total_count: 2,
            success_count: 1,
            failure_count: 1,
            total_tokens: 20,
            total_cost: 0.30,
            first_byte_samples: &[],
            context: "seed pre-materialized summary rollups",
        },
    )
    .await;
    insert_invocation_failure_rollup(
        &state.pool,
        bucket_start_epoch,
        "upstream_response_failed",
        1,
    )
    .await;
    set_hourly_rollup_progress(&state.pool, "codex_invocations_summary_rollup_v2", 1).await;
    set_hourly_rollup_progress(
        &state.pool,
        "codex_invocations_summary_rollup_v2_live_cursor",
        0,
    )
    .await;

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let summary = fetch_test_summary(state.clone(), "all").await;
    assert_summary_totals(&summary, 2, 1, 1, 20, 0.30);

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

    let rollup_total_count = rollup_total_count(&state.pool, None).await;
    assert_eq!(
        rollup_total_count, 2,
        "missing summary replay markers on already-materialized archives must not double-count archived invocations",
    );
}

struct FailureOnlyBackfillFixture {
    first_archive_path: PathBuf,
    second_archive_path: PathBuf,
    bucket_start_epoch: i64,
}

async fn seed_failure_only_backfill_archives(state: &Arc<AppState>) -> FailureOnlyBackfillFixture {
    let archived_hour_local = archived_local_hour(12, 8);
    let archived_success_at = local_minute(archived_hour_local, 5);
    let archived_failed_at = local_minute(archived_hour_local, 25);
    let archived_second_success_at = local_minute(archived_hour_local, 45);
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
    FailureOnlyBackfillFixture {
        first_archive_path,
        second_archive_path,
        bucket_start_epoch: invocation_bucket_start_epoch(&archived_success_at)
            .expect("bucket start epoch should be derivable"),
    }
}

async fn seed_failure_only_backfill_metadata(
    state: &Arc<AppState>,
    fixture: &FailureOnlyBackfillFixture,
) {
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch: fixture.bucket_start_epoch,
            total_count: 3,
            success_count: 2,
            failure_count: 1,
            total_tokens: 35,
            total_cost: 0.45,
            first_byte_samples: &[],
            context: "seed correct overall summary rollup across both archives",
        },
    )
    .await;
    insert_legacy_archive_replay_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &fixture.first_archive_path,
    )
    .await;
    insert_legacy_archive_replay_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &fixture.second_archive_path,
    )
    .await;
    insert_legacy_archive_replay_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        &fixture.second_archive_path,
    )
    .await;
    set_hourly_rollup_progress(&state.pool, "codex_invocations_summary_rollup_v2", 1).await;
    set_hourly_rollup_progress(
        &state.pool,
        "codex_invocations_summary_rollup_v2_live_cursor",
        0,
    )
    .await;
    set_hourly_rollup_progress(&state.pool, HOURLY_ROLLUP_DATASET_INVOCATIONS, 0).await;
}

async fn assert_failure_only_backfill(state: &Arc<AppState>, fixture: &FailureOnlyBackfillFixture) {
    let summary = fetch_test_summary(state.clone(), "all").await;
    assert_summary_totals(&summary, 3, 2, 1, 35, 0.45);
    assert_eq!(
        rollup_total_count(&state.pool, Some(fixture.bucket_start_epoch)).await,
        3
    );
    assert_eq!(
        archive_replay_marker_count(
            &state.pool,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            &fixture.first_archive_path,
        )
        .await,
        1
    );
    assert_eq!(
        archive_replay_marker_count(
            &state.pool,
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            &fixture.first_archive_path,
        )
        .await,
        1
    );
}

#[tokio::test]
pub(crate) async fn all_time_summary_backfill_preserves_overall_rollups_when_only_failure_marker_is_missing()
 {
    let state = archive_retention_test_state().await;
    let fixture = seed_failure_only_backfill_archives(&state).await;
    seed_failure_only_backfill_metadata(&state, &fixture).await;
    run_background_invocation_summary_rollup_repair(&state.pool).await;
    assert_failure_only_backfill(&state, &fixture).await;
}

use super::*;
