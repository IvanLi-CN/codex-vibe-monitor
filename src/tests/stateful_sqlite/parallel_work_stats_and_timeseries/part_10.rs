async fn insert_live_summary_invocation(
    pool: &SqlitePool,
    id: i64,
    invoke_id: &str,
    occurred_at: &str,
    total_tokens: i64,
    cost: f64,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, raw_response
        ) VALUES (?1, ?2, ?3, ?4, 'success', ?5, ?6, '{}')
        "#,
    )
    .bind(id)
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(total_tokens)
    .bind(cost)
    .execute(pool)
    .await
    .expect("insert live summary invocation");
}

#[tokio::test]
pub(crate) async fn all_time_summary_repair_preserves_pruned_materialized_archives() {
    let state = archive_retention_test_state().await;
    let archived_success_at = local_minute(archived_local_hour(460, 8), 5);

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

    mark_archive_materialized(&state.pool, &archive_path).await;

    let bucket_start_epoch = invocation_bucket_start_epoch(&archived_success_at)
        .expect("bucket start epoch should be derivable");
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch,
            total_count: 1,
            success_count: 1,
            failure_count: 0,
            total_tokens: 10,
            total_cost: 0.10,
            first_byte_samples: &[],
            context: "seed pre-materialized summary rollups",
        },
    )
    .await;
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;

    fs::remove_file(&archive_path).expect("prune materialized archive file");

    let live_occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(10))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_live_summary_invocation(
        &state.pool,
        10,
        "summary-all-materialized-pruned-live-success",
        &live_occurred_at,
        10,
        0.01,
    )
    .await;

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let summary = fetch_test_summary(state.clone(), "all").await;
    assert_summary_totals(&summary, 2, 2, 0, 20, 0.11);
    assert_eq!(
        summary_replay_target_count(&state.pool, &archive_path).await,
        2
    );
    assert_eq!(
        hourly_rollup_progress(&state.pool, "codex_invocations_summary_rollup_v2").await,
        Some(1)
    );
    assert_eq!(
        hourly_rollup_progress(
            &state.pool,
            "codex_invocations_summary_rollup_v2_live_cursor"
        )
        .await,
        Some(10)
    );
    assert_eq!(rollup_total_count(&state.pool, None).await, 2);
}

pub(crate) async fn seed_pruned_materialized_summary_archive(state: &Arc<AppState>) {
    let occurred_at = local_minute(archived_local_hour(470, 8), 5);
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-materialized-pruned-mixed",
        &[(
            1,
            "summary-all-materialized-pruned-mixed-success",
            occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            10,
            0.10,
            Some(100.0),
        )],
    )
    .await;
    mark_archive_materialized(&state.pool, &archive_path).await;
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch: invocation_bucket_start_epoch(&occurred_at)
                .expect("derive pruned bucket epoch"),
            total_count: 1,
            success_count: 1,
            failure_count: 0,
            total_tokens: 10,
            total_cost: 0.10,
            first_byte_samples: &[],
            context: "seed preserved pruned archive rollup",
        },
    )
    .await;
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;
    fs::remove_file(archive_path).expect("prune older materialized archive");
}

async fn seed_existing_materialized_summary_archive(state: &Arc<AppState>) -> (PathBuf, i64) {
    let local_hour = archived_local_hour(440, 8);
    let success_at = local_minute(local_hour, 5);
    let failed_at = local_minute(local_hour, 15);
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-materialized-existing-mixed",
        &[
            (
                1,
                "summary-all-materialized-existing-success",
                success_at.as_str(),
                SOURCE_PROXY,
                "success",
                20,
                0.20,
                Some(200.0),
            ),
            (
                2,
                "summary-all-materialized-existing-failed",
                failed_at.as_str(),
                SOURCE_PROXY,
                "failed",
                30,
                0.30,
                Some(300.0),
            ),
        ],
    )
    .await;
    mark_archive_materialized(&state.pool, &archive_path).await;
    let bucket_start_epoch =
        invocation_bucket_start_epoch(&success_at).expect("derive existing bucket epoch");
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch,
            total_count: 2,
            success_count: 2,
            failure_count: 0,
            total_tokens: 50,
            total_cost: 0.50,
            first_byte_samples: &[],
            context: "seed stale existing materialized rollup",
        },
    )
    .await;
    (archive_path, bucket_start_epoch)
}

pub(crate) async fn invocation_rollup_failure_count(
    pool: &SqlitePool,
    bucket_start_epoch: i64,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(failure_count), 0) FROM invocation_rollup_hourly \
         WHERE bucket_start_epoch = ?1",
    )
    .bind(bucket_start_epoch)
    .fetch_one(pool)
    .await
    .expect("load invocation rollup failure count")
}

#[tokio::test]
pub(crate) async fn all_time_summary_repair_replays_existing_materialized_archives_when_others_are_pruned()
 {
    let state = archive_retention_test_state().await;
    seed_pruned_materialized_summary_archive(&state).await;
    let (existing_archive_path, existing_bucket_start_epoch) =
        seed_existing_materialized_summary_archive(&state).await;

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let summary = fetch_test_summary(state.clone(), "all").await;
    assert_summary_totals(&summary, 3, 2, 1, 60, 0.60);
    assert_eq!(
        summary_replay_target_count(&state.pool, &existing_archive_path).await,
        2
    );
    assert_eq!(
        invocation_rollup_failure_count(&state.pool, existing_bucket_start_epoch).await,
        1
    );
}

#[tokio::test]
pub(crate) async fn all_time_summary_read_path_skips_unreadable_materialized_archives() {
    let state = archive_retention_test_state().await;
    let archived_success_at = local_minute(archived_local_hour(10, 8), 5);
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

    mark_archive_materialized(&state.pool, &archive_path).await;

    let bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_success_at).expect("derive summary bucket epoch");
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch,
            total_count: 1,
            success_count: 1,
            failure_count: 0,
            total_tokens: 10,
            total_cost: 0.10,
            first_byte_samples: &[100.0],
            context: "seed materialized summary rollup row",
        },
    )
    .await;
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;

    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt materialized archive batch");

    let summary = fetch_test_summary(state, "all").await;
    assert_summary_totals(&summary, 1, 1, 0, 10, 0.10);
}

#[tokio::test]
pub(crate) async fn all_time_summary_fails_closed_when_unreadable_replay_lacks_usage_and_account_coverage()
 {
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
pub(crate) async fn summary_refresh_revokes_all_time_when_new_archive_is_unreadable() {
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
pub(crate) async fn all_time_stats_tolerate_unreadable_pending_archives_while_summary_fails_closed()
{
    let state = archive_retention_test_state().await;
    let archived_success_at = local_minute(archived_local_hour(10, 8), 5);
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
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch,
            total_count: 1,
            success_count: 1,
            failure_count: 0,
            total_tokens: 10,
            total_cost: 0.10,
            first_byte_samples: &[100.0],
            context: "seed unreadable pending summary rollup row",
        },
    )
    .await;
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        bucket_start_epoch,
        SOURCE_PROXY,
    )
    .await;

    fs::write(&archive_path, b"not-a-gzip-archive").expect("corrupt pending archive batch");

    let Json(stats) = fetch_stats(State(state.clone()))
        .await
        .expect("fetch stats with unreadable pending archive");
    assert_summary_totals(&stats, 1, 1, 0, 10, 0.10);

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
pub(crate) async fn summary_rollup_repair_best_effort_skips_unreadable_pending_archives() {
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

async fn seed_boundary_archive_and_live_row(state: &Arc<AppState>) -> i64 {
    let boundary_hour_local = archived_local_hour(440, 8);
    let archived_boundary_at = local_minute(boundary_hour_local, 5);
    let live_boundary_at = local_minute(boundary_hour_local, 45);
    let boundary_archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-all-boundary-live-existing-materialized",
        &[(
            2,
            "summary-all-boundary-live-archived-success",
            archived_boundary_at.as_str(),
            SOURCE_PROXY,
            "success",
            20,
            0.20,
            Some(200.0),
        )],
    )
    .await;
    mark_archive_materialized(&state.pool, &boundary_archive_path).await;
    let bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_boundary_at).expect("derive boundary bucket epoch");
    insert_parallel_timeseries_invocation_rollup(
        &state.pool,
        ParallelTimeseriesInvocationRollupSpec {
            bucket_start_epoch,
            total_count: 2,
            success_count: 2,
            failure_count: 0,
            total_tokens: 50,
            total_cost: 0.50,
            first_byte_samples: &[],
            context: "seed pre-repair boundary rollup",
        },
    )
    .await;
    insert_live_summary_invocation(
        &state.pool,
        10,
        "summary-all-boundary-live-live-success",
        &live_boundary_at,
        30,
        0.30,
    )
    .await;
    set_hourly_rollup_progress(&state.pool, HOURLY_ROLLUP_DATASET_INVOCATIONS, 10).await;
    bucket_start_epoch
}

#[tokio::test]
pub(crate) async fn all_time_summary_repair_restores_live_rows_in_boundary_hours_when_preserving_pruned_materialized_archives()
 {
    let state = archive_retention_test_state().await;
    seed_pruned_materialized_summary_archive(&state).await;
    let boundary_bucket_start_epoch = seed_boundary_archive_and_live_row(&state).await;

    run_background_invocation_summary_rollup_repair(&state.pool).await;

    let summary = fetch_test_summary(state.clone(), "all").await;
    assert_summary_totals(&summary, 3, 3, 0, 60, 0.60);
    assert_eq!(
        rollup_total_count(&state.pool, Some(boundary_bucket_start_epoch)).await,
        2
    );
    assert_eq!(
        hourly_rollup_progress(
            &state.pool,
            "codex_invocations_summary_rollup_v2_live_cursor"
        )
        .await,
        Some(10)
    );
}

use super::*;
