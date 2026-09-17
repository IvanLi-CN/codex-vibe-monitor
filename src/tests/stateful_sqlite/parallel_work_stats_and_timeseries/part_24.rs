async fn replace_all_time_archive_manifest(
    state: &AppState,
    archive_path: &std::path::Path,
    replacement_sha256: &str,
    occurred_at: &str,
) {
    let file_path = archive_path.to_string_lossy().to_string();
    let mut tx = state
        .pool
        .begin()
        .await
        .expect("begin all-time archive replacement");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = ?1 \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(replacement_sha256)
    .bind(file_path)
    .execute(tx.as_mut())
    .await
    .expect("replace all-time archive manifest SHA");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response) \
         VALUES ('summary-all-time-archive-identity-terminal', ?1, 'proxy', 'success', 7, 0.07, 'full', '{}', '')",
    )
    .bind(occurred_at)
    .execute(tx.as_mut())
    .await
    .expect("write terminal beside all-time archive replacement");
    tx.commit()
        .await
        .expect("commit all-time archive replacement");
}

async fn fetch_all_time_summary(
    state: Arc<AppState>,
    upstream_account_id: Option<i64>,
) -> Result<StatsResponse, ApiError> {
    fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id,
        }),
    )
    .await
    .map(|Json(summary)| summary)
}

async fn record_summary_archive_replay_coverage(state: &AppState, file_path: &str, sha256: &str) {
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) \
             VALUES (?1, 'codex_invocations', ?2, ?3)",
        )
        .bind(target)
        .bind(file_path)
        .bind(sha256)
        .execute(&state.pool)
        .await
        .expect("record durable replay coverage");
    }
}

async fn assert_rolling_summary_is_zero(state: Arc<AppState>) {
    let Json(summary) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve exact rolling zero response without SQLite");
    assert_eq!(summary.total_count, 0);
}

async fn build_replacement_archive(
    state: &AppState,
    archive_path: &std::path::Path,
    occurred_at: &str,
) -> (std::path::PathBuf, String) {
    let database_path = state
        .config
        .archive_dir
        .join("summary-all-time-archive-identity-replacement.sqlite");
    inflate_gzip_sqlite_file(archive_path, &database_path)
        .expect("inflate replacement all-time archive");
    let pool = SqlitePool::connect(&test_sqlite_url_for_path(&database_path))
        .await
        .expect("open replacement all-time archive");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response, created_at) \
         VALUES (80002, 'summary-all-time-archive-identity-replacement', ?1, 'proxy', 'success', 99, 9.9, 'full', '{}', '{}', ?1)",
    )
    .bind(occurred_at)
    .execute(&pool)
    .await
    .expect("seed replacement all-time archive row");
    pool.close().await;
    let replacement_path = state
        .config
        .archive_dir
        .join("summary-all-time-archive-identity-replacement.sqlite.gz");
    deflate_sqlite_file_to_gzip(&database_path, &replacement_path)
        .expect("compress replacement all-time archive");
    let sha256 = sha256_hex_file(&replacement_path).expect("hash replacement all-time archive");
    fs::remove_file(database_path).expect("remove replacement all-time archive source");
    (replacement_path, sha256)
}

#[tokio::test]
pub(crate) async fn summary_projection_rejects_replaced_unmaterialized_all_time_archive() {
    let _identity_guard = SUMMARY_PROJECTION_ARCHIVE_IDENTITY_TEST_LOCK.lock().await;
    let temp_dir = make_temp_test_dir("summary-all-time-archive-identity");
    let database_path = temp_dir.join("summary-projection.db");
    fs::File::create(&database_path).expect("create summary projection database");
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect(&test_sqlite_url_for_path(&database_path))
        .await
        .expect("open summary projection database");
    let mut config = test_config();
    config.database_path = database_path;
    config.archive_dir = temp_dir.join("archives");
    config.proxy_raw_dir = temp_dir.join("proxy-raw");
    config.invocation_max_days = 7;
    config.openai_upstream_base_url = Url::parse("http://127.0.0.1:9").expect("valid test URL");
    fs::create_dir_all(&config.archive_dir).expect("create summary projection archive directory");
    fs::create_dir_all(&config.proxy_raw_dir).expect("create summary projection raw directory");
    let state = test_state_from_existing_pool(pool, config, true).await;
    sqlx::query_scalar::<_, String>("PRAGMA journal_mode = WAL")
        .fetch_one(&state.pool)
        .await
        .expect("enable WAL for concurrent all-time snapshot writer coverage");

    // Keep this beyond the rolling exact horizon (retention plus the 31-day grace window) so
    // only the all-time generic archive aggregation can observe the replacement.
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(45))
    .and_hms_opt(6, 0, 0)
    .expect("valid archived local hour");
    let archived_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived invocation time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-time-archive-identity",
        &[(
            80_001,
            "summary-all-time-archive-identity",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            10,
            0.10,
            Some(100.0),
        )],
    )
    .await;
    insert_hourly_rollup_archive_replay_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        &archive_path,
    )
    .await;

    let (replacement_archive_path, replacement_archive_sha256) =
        build_replacement_archive(&state, &archive_path, &archived_at).await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish the initial exact rolling projection");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("publish initial exact all-time archive snapshot after reconciliation");
    fs::write(&archive_path, b"not-a-gzip-archive")
        .expect("make all-time archive unavailable before recovery");
    fs::remove_file(&replacement_archive_path).expect("remove unused replacement archive");
    replace_all_time_archive_manifest(
        &state,
        &archive_path,
        &replacement_archive_sha256,
        &archived_at,
    )
    .await;

    tokio::time::timeout(
        Duration::from_secs(2),
        refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime),
    )
    .await
    .expect("all-time recovery must not wait for an unchanged publication stage")
    .expect("retain the prior all-time aggregate without scanning a replaced archive");
    state.pool.close().await;

    let summary = fetch_all_time_summary(state.clone(), None)
        .await
        .expect("serve the prior all-time snapshot without SQLite");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.total_tokens, 10);
    assert_f64_close(summary.total_cost, 0.10);
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn summary_projection_keeps_global_all_exact_when_account_manifest_admission_exceeds_budget()
 {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let archive_start = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::days(1_000)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align archive fixture to a full hour");
    let bucket = crate::stats::align_bucket_epoch(archive_start.timestamp(), 3_600, 0);
    let file_path = "/tmp/summary-account-manifest-admission.sqlite.gz";
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(bucket)
    .execute(&state.pool)
    .await
    .expect("insert durable global rollup");
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, created_at) \
         VALUES ('codex_invocations', '2026-01', ?1, 'summary-account-manifest-admission', 1, 'completed', \
                 ?2, ?3, datetime('now'), datetime('now'))",
    )
    .bind(file_path)
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .execute(&state.pool)
    .await
    .expect("insert materialized archive manifest");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(file_path)
    .fetch_one(&state.pool)
    .await
    .expect("load archive manifest id");
    sqlx::query(
        "WITH RECURSIVE accounts(account_id) AS ( \
            SELECT 1 UNION ALL SELECT account_id + 1 FROM accounts WHERE account_id <= 50000 \
         ) \
         INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) \
         SELECT ?1, account_id, ?2 FROM accounts",
    )
    .bind(archive_batch_id)
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .execute(&state.pool)
    .await
    .expect("insert account activity beyond bounded admission");
    record_summary_archive_replay_coverage(&state, file_path, "summary-account-manifest-admission")
        .await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate exact global all-time response despite account admission overflow");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile exact global all-time response despite account admission overflow");
    state.pool.close().await;

    let global = fetch_all_time_summary(state.clone(), None)
        .await
        .expect("serve exact global all-time response without SQLite");
    assert_eq!(global.total_count, 1);
    assert_eq!(global.total_tokens, 17);
    assert_eq!(global.total_cost, 1.25);
    assert!(matches!(
        fetch_all_time_summary(state, Some(42)).await,
        Err(ApiError::Unavailable(_))
    ));
}

#[tokio::test]
pub(crate) async fn summary_projection_rejects_materialized_endpoint_hour_missing_rollup_keys() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("valid test URL"))
            .await;
    let archive_start = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::days(1_000)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align archive fixture to a full hour");
    let endpoint_hour = archive_start + ChronoDuration::hours(1);
    let file_path = "/tmp/summary-missing-endpoint-rollup-key.sqlite.gz";
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) \
         VALUES ('codex_invocations', '2026-01', ?1, 'summary-missing-endpoint-rollup-key', 2, 'completed', \
                 ?2, ?3, datetime('now'), datetime('now'), datetime('now'))",
    )
    .bind(file_path)
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    // `coverage_end_at` is inclusive: this batch also contains a row in `endpoint_hour`.
    .bind(crate::stats::db_occurred_at_lower_bound(endpoint_hour))
    .execute(&state.pool)
    .await
    .expect("insert materialized archive manifest");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(file_path)
    .fetch_one(&state.pool)
    .await
    .expect("load endpoint archive manifest id");
    sqlx::query(
        "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) \
         VALUES (?1, 42, ?2)",
    )
    .bind(archive_batch_id)
    .bind(crate::stats::db_occurred_at_lower_bound(archive_start))
    .execute(&state.pool)
    .await
    .expect("record endpoint archive account manifest");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(archive_start.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed only the pre-endpoint global rollup key");
    sqlx::query(
        "INSERT INTO upstream_account_stats_hourly \
         (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 42, 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(archive_start.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed only the pre-endpoint account rollup key");
    record_summary_archive_replay_coverage(&state, file_path, "summary-missing-rollup-key").await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate rolling projection with an incomplete all-time rollup");
    state.pool.close().await;

    assert_rolling_summary_is_zero(state.clone()).await;

    for upstream_account_id in [None, Some(42)] {
        assert!(
            matches!(
                fetch_summary(
                    State(state.clone()),
                    Query(SummaryQuery {
                        window: Some("all".to_string()),
                        limit: None,
                        time_zone: Some("UTC".to_string()),
                        upstream_account_id,
                    }),
                )
                .await,
                Err(ApiError::Unavailable(_))
            ),
            "the endpoint hour must be proven for {upstream_account_id:?}"
        );
    }
}

use super::*;
