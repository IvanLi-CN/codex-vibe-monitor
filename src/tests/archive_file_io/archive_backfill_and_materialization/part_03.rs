use super::*;
use serde_json::json;

async fn assert_zero_budget_live_rollup(pool: &SqlitePool) {
    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(pool)
    .await
    .expect("load invocation hourly total count after zero-budget bounded pass");
    assert_eq!(total_count, 0);
    let live_cursor: Option<i64> =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .fetch_optional(pool)
            .await
            .expect("load invocation live cursor after zero-budget bounded pass");
    assert_eq!(live_cursor, None);
}

async fn assert_account_targets_replayed(pool: &SqlitePool, archive_path: &str) {
    let account_target_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay
         WHERE dataset = 'codex_invocations' AND file_path = ?1
           AND target IN (?2, ?3, ?4, ?5)",
    )
    .bind(archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .fetch_one(pool)
    .await
    .expect("count account replay markers");
    assert_eq!(account_target_markers, 4);
    let breakdown_row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_usage_breakdown_hourly")
            .fetch_one(pool)
            .await
            .expect("count usage breakdown rollup rows after replay");
    assert!(breakdown_row_count > 0);
}

async fn assert_bootstrap_reopened_account_backfill(
    pool: &SqlitePool,
    config: &AppConfig,
    archive_path: &str,
) {
    let breakdown_replay_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay
         WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target = ?2",
    )
    .bind(archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(pool)
    .await
    .expect("count remaining breakdown replay markers");
    assert_eq!(breakdown_replay_markers, 0);
    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path)
    .fetch_one(pool)
    .await
    .expect("load materialized timestamp after bootstrap");
    assert!(materialized_at.is_none());
    let backlog_snapshot = load_historical_rollup_backfill_snapshot(pool, config)
        .await
        .expect("load backlog after bootstrap repair");
    assert_eq!(backlog_snapshot.legacy_archive_pending, 1);
}

async fn assert_legacy_account_markers_repaired(pool: &SqlitePool, archive_path: &str) {
    let account_target_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay
         WHERE dataset = 'codex_invocations' AND file_path = ?1
           AND target IN (?2, ?3, ?4, ?5)",
    )
    .bind(archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .fetch_one(pool)
    .await
    .expect("count repaired legacy account replay markers");
    assert_eq!(account_target_markers, 4);
}

async fn assert_bootstrap_live_breakdown_cursors(pool: &SqlitePool) {
    let breakdown = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT request_count, success_count, performance_total_tokens
         FROM upstream_account_usage_breakdown_hourly
         WHERE upstream_account_id = ?1 AND normalized_model = 'gpt-5'",
    )
    .bind(17_i64)
    .fetch_one(pool)
    .await
    .expect("load repaired breakdown rollup row");
    assert_eq!(breakdown, (2, 2, 70));
    let shared_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .fetch_one(pool)
            .await
            .expect("load shared invocation cursor after bootstrap");
    assert_eq!(shared_cursor, 42);
    let repair_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET)
            .fetch_one(pool)
            .await
            .expect("load breakdown repair cursor after bootstrap");
    assert_eq!(repair_cursor, 41);
}

async fn assert_replayed_batches_materialized(pool: &SqlitePool, config: &AppConfig) {
    let materialized_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .expect("count replayed batches marked materialized");
    assert_eq!(materialized_batches, 2);
    let snapshot_after = load_historical_rollup_backfill_snapshot(pool, config)
        .await
        .expect("load snapshot after marking replayed batches");
    assert_eq!(snapshot_after.legacy_archive_pending, 0);
}

async fn seed_healthy_breakdown_rollup(pool: &SqlitePool, archive_path: &str, bucket_epoch: i64) {
    sqlx::query(
        "INSERT INTO upstream_account_usage_breakdown_hourly (
            bucket_start_epoch, source, upstream_account_key, upstream_account_id,
            normalized_model, normalized_reasoning_effort, request_count, success_count,
            failure_count, cache_write_tokens, cache_read_tokens, output_tokens, cost_input,
            cost_cache_write, cost_cache_read, cost_output, cost_reasoning, cost_unknown,
            has_cost, performance_total_tokens, performance_stream_output_tokens,
            performance_stream_duration_ms, performance_response_sample_count,
            performance_response_sum_ms, performance_first_byte_sample_count,
            performance_first_byte_sum_ms, performance_usage_duration_sample_count,
            performance_usage_duration_sum_ms, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, '', 1, 1, 0, 0, 0, 42,
            0.42, 0.0, 0.0, 0.0, 0.0, 0.0, 1, 42, 42, 120.0, 1, 120.0, 1, 60.0,
            1, 120.0, datetime('now'))",
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .bind("upstream:17")
    .bind(17_i64)
    .bind("gpt-5")
    .execute(pool)
    .await
    .expect("seed healthy breakdown rollup row");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay
         WHERE dataset = 'codex_invocations' AND file_path = ?1
           AND target IN (?2, ?3, ?4)",
    )
    .bind(archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .execute(pool)
    .await
    .expect("drop legacy account replay markers");
}

async fn seed_stale_breakdown_manifest(
    pool: &SqlitePool,
    archive_path: &str,
    initial_sha256: &str,
) {
    sqlx::query(
        "INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status, coverage_start_at,
            coverage_end_at, historical_rollups_materialized_at, created_at
        ) VALUES (?1, '2026-01', ?2, ?3, 1, ?4, '2026-01-15 08:00:00',
            '2026-01-15 08:30:00', datetime('now'), datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path)
    .bind(initial_sha256)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(pool)
    .await
    .expect("seed materialized archive manifest at SHA A");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay
         (target, dataset, file_path, archive_sha256, replayed_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path)
    .bind(initial_sha256)
    .execute(pool)
    .await
    .expect("seed matching usage breakdown replay marker");
}

async fn load_invocation_rollup_totals(
    pool: &SqlitePool,
) -> (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(sample_count), 0) FROM proxy_perf_stage_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_rollup_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_upstream_account_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(activity_v2_request_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_minute WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_sticky_key_hourly WHERE upstream_account_id = 17)",
    )
    .fetch_one(pool)
    .await
    .expect("load global and account rollup totals")
}

fn transitive_archive_row<'a>(
    id: i64,
    invoke_id: &'a str,
    occurred_at: &'a str,
) -> SeedInvocationArchiveBatchRow<'a> {
    SeedInvocationArchiveBatchRow {
        id,
        invoke_id,
        occurred_at,
        source: SOURCE_PROXY,
        status: "success",
        total_tokens: 12,
        cost: 0.12,
        ttfb_ms: Some(120.0),
        payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
        detail_level: DETAIL_LEVEL_FULL,
        error_message: None,
        failure_kind: None,
        failure_class: Some("none"),
        is_actionable: Some(0),
    }
}

async fn seed_transitive_breakdown_archives(
    pool: &SqlitePool,
    config: &AppConfig,
    transitive_at: &str,
    boundary_start_at: &str,
    boundary_end_at: &str,
    stale_at: &str,
) -> PathBuf {
    let stale_path = seed_invocation_archive_batch_with_details(
        pool,
        config,
        "breakdown-closure-stale-root",
        &[transitive_archive_row(
            1,
            "breakdown-closure-stale-root",
            stale_at,
        )],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        pool,
        config,
        "breakdown-closure-boundary-peer",
        &[
            transitive_archive_row(
                1,
                "breakdown-closure-boundary-peer-start",
                boundary_start_at,
            ),
            transitive_archive_row(2, "breakdown-closure-boundary-peer-end", boundary_end_at),
        ],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        pool,
        config,
        "breakdown-closure-transitive-peer",
        &[transitive_archive_row(
            1,
            "breakdown-closure-transitive-peer",
            transitive_at,
        )],
    )
    .await;
    stale_path
}

fn transitive_breakdown_times(config: &AppConfig) -> (String, String, String, String) {
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(7, 0, 0)
    .expect("valid archived hour");
    let transitive = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("valid transitive overlap timestamp"),
    );
    let boundary_start = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(30)))
            .expect("valid boundary archive start timestamp"),
    );
    let boundary_end = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(2))
            .expect("valid boundary archive end timestamp"),
    );
    let stale = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(2))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("valid stale archive timestamp"),
    );
    (transitive, boundary_start, boundary_end, stale)
}

async fn seed_replayed_archive_markers(pool: &SqlitePool) -> (String, String) {
    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load invocation archive path");
    sqlx::query("DELETE FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2")
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&invocation_archive_path)
        .execute(pool)
        .await
        .expect("clear atomic publication proofs to mimic pre-upgrade replay state");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256, replayed_at)
             SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
             FROM archive_batches AS batches
             WHERE batches.dataset = ?2 AND batches.status = 'completed' AND batches.file_path = ?3",
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&invocation_archive_path)
        .execute(pool)
        .await
        .expect("insert invocation replay marker");
    }
    let forward_proxy_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'forward_proxy_attempts' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load forward-proxy archive path");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256, replayed_at)
         SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
         FROM archive_batches AS batches
         WHERE batches.dataset = ?2 AND batches.status = 'completed' AND batches.file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(&forward_proxy_archive_path)
    .execute(pool)
    .await
    .expect("insert forward-proxy replay marker");
    (invocation_archive_path, forward_proxy_archive_path)
}

async fn assert_legacy_replay_markers_stay_quarantined(
    pool: &SqlitePool,
    archive_path: &str,
    before: (i64, i64, i64),
) {
    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = NULL
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path)
    .execute(pool)
    .await
    .expect("seed legacy null replay markers");
    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = ''
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path)
    .execute(pool)
    .await
    .expect("seed legacy blank replay marker");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = 'replacement-archive-sha-b'
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path)
    .execute(pool)
    .await
    .expect("publish replacement manifest SHA B");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        let replay_sha256: Option<String> = sqlx::query_scalar(
            "SELECT archive_sha256 FROM hourly_rollup_archive_replay
             WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(archive_path)
        .fetch_one(pool)
        .await
        .expect("load quarantined replay marker");
        let expected = if target == HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN {
            Some("")
        } else {
            None
        };
        assert_eq!(replay_sha256.as_deref(), expected);
    }
    assert_eq!(
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(pool)
            .await
            .expect("legacy marker quarantine must not reopen the archive"),
        0
    );
    let mut tx = pool.begin().await.expect("begin legacy proof check");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        assert!(
            !hourly_rollup_archive_replayed_tx(
                tx.as_mut(),
                target,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                archive_path,
            )
            .await
            .expect("legacy replay marker must fail closed")
        );
    }
    tx.commit().await.expect("commit legacy proof check");
    let after: (i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)",
    )
    .fetch_one(pool)
    .await
    .expect("load totals after legacy marker upgrade");
    assert_eq!(after, before);
}

async fn assert_summary_recovery_replayed_once(
    pool: &SqlitePool,
    archive_path: &str,
    before: (i64, i64, i64),
) {
    let recovered = materialize_historical_rollups_startup_window(pool, 0, Duration::from_secs(6))
        .await
        .expect("startup recovery must reopen and replay the verified archive");
    assert_eq!(recovered.summary.materialized_invocation_batches, 1);
    assert_eq!(recovered.changed_path_count, 1);
    let after: (i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)",
    )
    .fetch_one(pool)
    .await
    .expect("load Summary totals after startup recovery replay");
    assert_eq!(after, before);
    let summary_proof_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay
         WHERE dataset = ?1 AND file_path = ?2 AND target IN (?3, ?4, ?5)",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(pool)
    .await
    .expect("count complete Summary proofs after automatic replay");
    assert_eq!(
        summary_proof_count,
        SUMMARY_PROJECTION_ARCHIVE_REPLAY_TARGETS.len() as i64
    );
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_bounded_skips_live_replay_when_elapsed_budget_is_zero()
 {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-bounded-live-budget-zero").await;
    let recent_invocation = shanghai_local_days_ago(0, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "historical-rollup-bounded-live-budget-zero",
        &recent_invocation,
        SOURCE_PROXY,
        "success",
        Some(r#"{"promptCacheKey":"live-budget-zero"}"#),
        "{\"ok\":true}",
        None,
        None,
        Some(8),
        Some(0.08),
    )
    .await;

    let summary =
        materialize_historical_rollups_bounded(&pool, &config, false, None, Some(Duration::ZERO))
            .await
            .expect("bounded materialization with zero elapsed budget");
    assert_eq!(summary.materialized_archive_batches, 0);

    assert_zero_budget_live_rollup(&pool).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_marks_replayed_batches_as_materialized_after_usage_breakdown_replay()
 {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-mark-replayed").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);
    let old_attempt = parse_to_utc_datetime(&utc_naive_from_shanghai_local_days_ago(
        (config.forward_proxy_attempts_retention_days + 2) as i64,
        7,
        0,
        0,
    ))
    .expect("parse old forward proxy attempt timestamp");

    insert_retention_invocation(
        &pool,
        "historical-rollup-mark-replayed",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"promptCacheKey\":\"replayed\",\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay\",\"stickyKey\":\"sticky-replayed\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    seed_forward_proxy_attempt_at(&pool, "proxy-replayed", old_attempt, true).await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before materialize");
    assert_eq!(retention.invocation_rows_archived, 1);
    assert_eq!(retention.forward_proxy_attempt_rows_archived, 1);

    sqlx::query("UPDATE archive_batches SET historical_rollups_materialized_at = NULL")
        .execute(&pool)
        .await
        .expect("clear materialized markers to mimic pre-upgrade replay state");

    let (_invocation_archive_path, _forward_proxy_archive_path) =
        seed_replayed_archive_markers(&pool).await;

    let snapshot_before = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load snapshot before marking replayed batches");
    assert_eq!(snapshot_before.legacy_archive_pending, 2);

    let materialize = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize should only mark replayed batches");
    assert_eq!(materialize.materialized_invocation_batches, 1);
    assert_eq!(materialize.materialized_forward_proxy_batches, 0);

    assert_replayed_batches_materialized(&pool, &config).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_replays_usage_breakdown_when_account_targets_are_pending()
 {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-account-target-markers").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "historical-rollup-account-target-markers",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before materialize");
    assert_eq!(retention.invocation_rows_archived, 1);

    sqlx::query("UPDATE archive_batches SET historical_rollups_materialized_at = NULL")
        .execute(&pool)
        .await
        .expect("clear materialized markers to mimic pre-upgrade replay state");

    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive path");
    sqlx::query("DELETE FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2")
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&invocation_archive_path)
        .execute(&pool)
        .await
        .expect("clear automatic publication proofs to mimic pre-upgrade replay state");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (
                target,
                dataset,
                file_path,
                archive_sha256,
                replayed_at
            )
            SELECT ?1, ?2, batches.file_path, batches.sha256, datetime('now')
            FROM archive_batches AS batches
            WHERE batches.dataset = ?2
              AND batches.status = 'completed'
              AND batches.file_path = ?3
            "#,
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&invocation_archive_path)
        .execute(&pool)
        .await
        .expect("insert invocation replay marker");
    }

    let materialize = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize should replay pending usage breakdown target");
    assert_eq!(materialize.materialized_invocation_batches, 1);

    assert_account_targets_replayed(&pool, &invocation_archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn usage_breakdown_priority_materialization_drains_backlog_without_clearing_blocked_batches()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("usage-breakdown-priority-materialize").await;
    let archived_occurred_at = shanghai_local_days_ago(120, 9, 0, 0);
    let payload = json!({
        "upstreamAccountId": 17,
        "upstreamAccountName": "Replay",
        "promptCacheKey": "pck-priority-materialize",
        "stickyKey": "sticky-priority-materialize",
        "responseModel": "gpt-5.4",
        "reasoningEffort": "high",
    })
    .to_string();
    seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "usage-breakdown-priority-materialize",
        &[SeedInvocationArchiveBatchRow {
            id: 1_i64,
            invoke_id: "usage-breakdown-priority-materialize",
            occurred_at: &archived_occurred_at,
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 42_i64,
            cost: 0.42_f64,
            ttfb_ms: Some(120.0),
            payload: Some(payload.as_str()),
            detail_level: "summary",
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let snapshot_before = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load backlog before priority materialization");
    assert_eq!(snapshot_before.legacy_archive_pending, 1);
    assert_eq!(snapshot_before.pending_usage_breakdown_batches, 1);

    let summary = materialize_usage_breakdown_historical_rollups_bounded_from_skip(
        &pool,
        &config,
        Some(2),
        Some(Duration::from_secs(6)),
        0,
    )
    .await
    .expect("priority materialization should drain usage breakdown backlog");
    assert_eq!(summary.scanned_archive_batches, 1);
    assert_eq!(summary.materialized_invocation_batches, 0);
    assert_eq!(summary.blocked_archive_batches, 1);

    let snapshot_after = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load backlog after priority materialization");
    assert_eq!(snapshot_after.pending_usage_breakdown_batches, 0);
    assert_eq!(snapshot_after.legacy_archive_pending, 1);

    let usage_breakdown_marker_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND target = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("count usage breakdown replay markers after priority materialization");
    assert_eq!(usage_breakdown_marker_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn bootstrap_hourly_rollups_reopens_materialized_batches_missing_usage_breakdown_backfill()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("bootstrap-repairs-account-markers").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "bootstrap-repairs-account-markers",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before bootstrap repair");
    assert_eq!(retention.invocation_rows_archived, 1);

    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive path");

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target IN (?2, ?3, ?4, ?5)
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .execute(&pool)
    .await
    .expect("drop account replay markers to mimic old materialized state");

    let snapshot_before_bootstrap = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup snapshot before bootstrap repair");
    assert_eq!(snapshot_before_bootstrap.legacy_archive_pending, 0);
    assert_eq!(snapshot_before_bootstrap.pending_usage_breakdown_batches, 1);
    assert_eq!(
        snapshot_before_bootstrap.alert_level,
        HistoricalRollupBackfillAlertLevel::Critical
    );

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should reopen missing usage breakdown backfill");

    assert_bootstrap_reopened_account_backfill(&pool, &config, &invocation_archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn bootstrap_hourly_rollups_repairs_pre_upgrade_breakdown_rows_before_syncing_new_live_rows()
 {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("bootstrap-live-breakdown-repair-before-sync").await;
    let first_local = shanghai_local_days_ago(0, 9, 10, 0);
    let second_local = shanghai_local_days_ago(0, 9, 20, 0);

    for (id, invoke_id, occurred_at, total_tokens, cost, upstream_account_id) in [
        (
            41_i64,
            "bootstrap-live-breakdown-old-cursor",
            first_local.as_str(),
            30_i64,
            0.01_f64,
            17_i64,
        ),
        (
            42_i64,
            "bootstrap-live-breakdown-new-row",
            second_local.as_str(),
            40_i64,
            0.02_f64,
            17_i64,
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
                detail_level,
                model,
                input_tokens,
                output_tokens,
                total_tokens,
                cost,
                payload,
                raw_response,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
        )
        .bind(id)
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(DETAIL_LEVEL_FULL)
        .bind("gpt-5")
        .bind(12_i64)
        .bind(6_i64)
        .bind(total_tokens)
        .bind(cost)
        .bind(
            json!({
                "upstreamAccountId": upstream_account_id,
                "responseModel": "gpt-5",
            })
            .to_string(),
        )
        .bind("{}")
        .bind(occurred_at)
        .execute(&pool)
        .await
        .expect("insert live invocation for breakdown bootstrap repair");
    }

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id)
        VALUES (?1, ?2)
        ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(41_i64)
    .execute(&pool)
    .await
    .expect("seed pre-upgrade shared invocation cursor");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should repair old breakdown rows before syncing new live rows");

    assert_bootstrap_live_breakdown_cursors(&pool).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn bootstrap_hourly_rollups_repairs_legacy_account_replay_markers_when_breakdown_is_healthy()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("bootstrap-repairs-legacy-account-markers").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "bootstrap-repairs-legacy-account-markers",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old rows before bootstrap repair");
    assert_eq!(retention.invocation_rows_archived, 1);

    let invocation_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive path");

    let bucket_epoch: i64 = sqlx::query_scalar(
        "SELECT bucket_start_epoch FROM upstream_account_usage_hourly ORDER BY bucket_start_epoch DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load usage rollup bucket epoch");

    seed_healthy_breakdown_rollup(&pool, &invocation_archive_path, bucket_epoch).await;

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should repair legacy account replay markers");

    assert_legacy_account_markers_repaired(&pool, &invocation_archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn usage_breakdown_repair_reopens_a_replaced_archive_with_a_stale_replay_sha() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-stale-replay-sha").await;
    let archive_path = temp_dir
        .join("archives")
        .join("codex_invocations")
        .join("breakdown-repair-stale-replay-sha.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let archive_file = PathBuf::from(&archive_path);
    fs::create_dir_all(
        archive_file
            .parent()
            .expect("archive fixture has a parent directory"),
    )
    .expect("create archive fixture directory");
    fs::write(&archive_file, b"initial archive bytes").expect("write initial archive fixture");
    let initial_sha256 = sha256_hex_file(&archive_file).expect("hash initial archive fixture");

    seed_stale_breakdown_manifest(&pool, &archive_path, &initial_sha256).await;

    let untouched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("matching marker must preserve materialized archive state");
    assert_eq!(untouched, 0);

    fs::write(&archive_file, b"replacement archive bytes").expect("replace archive fixture bytes");
    let replacement_sha256 =
        sha256_hex_file(&archive_file).expect("hash replacement archive fixture");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = ?1
        WHERE dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(&replacement_sha256)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("replace completed archive manifest at the same path with SHA B");

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("stale marker must reopen the replacement archive");
    assert_eq!(touched, 1);

    let materialized_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT historical_rollups_materialized_at
        FROM archive_batches
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load reopened replacement archive state");
    assert!(materialized_at.is_none());

    let replay_marker_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("count stale usage breakdown replay markers");
    assert_eq!(replay_marker_count, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn usage_breakdown_repair_rebuilds_transitive_boundary_overlap_closure() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("breakdown-transitive-boundary-closure").await;
    let (transitive_at, boundary_start_at, boundary_end_at, stale_at) =
        transitive_breakdown_times(&config);
    let stale_archive_path = seed_transitive_breakdown_archives(
        &pool,
        &config,
        &transitive_at,
        &boundary_start_at,
        &boundary_end_at,
        &stale_at,
    )
    .await;

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize the overlap closure archives");
    assert_eq!(initial.materialized_invocation_batches, 3);
    let before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly",
    )
    .fetch_one(&pool)
    .await
    .expect("capture initial usage breakdown total");
    assert_eq!(before, 4);

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1 AND dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(stale_archive_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("remove only the stale root usage breakdown marker");

    assert_eq!(
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
            .await
            .expect("reopen the full transitive overlap closure"),
        3
    );
    let rebuilt = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("replay the full transitive overlap closure");
    assert_eq!(rebuilt.materialized_invocation_batches, 3);
    let after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly",
    )
    .fetch_one(&pool)
    .await
    .expect("load usage breakdown total after closure replay");
    assert_eq!(after, before);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn startup_repair_quarantines_unverified_legacy_archive_without_replaying_rollups()
{
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("startup-repair-quarantines-unverified-legacy").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);

    insert_retention_invocation(
        &pool,
        "startup-repair-quarantines-unverified-legacy",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old invocation before legacy marker quarantine");
    assert_eq!(retention.invocation_rows_archived, 1);

    let archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archived invocation path");
    let before = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("capture materialized rollup totals before legacy marker quarantine");
    assert_eq!(before, (1, 1, 0));

    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = NULL WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("restore legacy null replay markers");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = '   ' WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("remove the completed archive manifest proof");

    for _ in 0..2 {
        bootstrap_hourly_rollups(&pool)
            .await
            .expect("startup repair must quarantine the unverified legacy archive");
        let summary = materialize_historical_rollups(&pool, &config, false)
            .await
            .expect("materializer must leave an unverified legacy archive quarantined");
        assert_eq!(summary.materialized_invocation_batches, 0);
    }

    let after_quarantine = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load rollup totals after legacy marker quarantine");
    assert_eq!(after_quarantine, before);
    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load archive materialization state after quarantine");
    assert!(materialized_at.is_some());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn legacy_replay_markers_do_not_upgrade_from_sha_a_to_sha_b() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("legacy-replay-marker-upgrade").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "legacy-replay-marker-a-to-b",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "legacy-replay-marker-a-to-b",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"legacy-replay-marker-a-to-b"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize archive before legacy marker upgrade");
    assert_eq!(initial.materialized_invocation_batches, 1);

    let before = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("capture materialized global and account totals");
    assert_eq!(before, (1, 1, 1));

    assert_legacy_replay_markers_stay_quarantined(&pool, &archive_file_path, before).await;

    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_stale_replay_sha_archive(pool: &SqlitePool, config: &AppConfig) -> (PathBuf, String) {
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        pool,
        config,
        "stale-replay-sha-full-rebuild",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "stale-replay-sha-full-rebuild",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"stale-replay-sha-full-rebuild"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    (
        archive_path.clone(),
        archive_path.to_string_lossy().to_string(),
    )
}

async fn replace_stale_replay_sha_archive(
    pool: &SqlitePool,
    temp_dir: &Path,
    archive_path: &Path,
    archive_file_path: &str,
) -> String {
    let replacement_db_path = temp_dir.join("stale-replay-sha-full-rebuild.sqlite");
    inflate_gzip_sqlite_file(archive_path, &replacement_db_path)
        .expect("inflate replacement invocation archive");
    let replacement_pool = SqlitePool::connect(&test_sqlite_url_for_path(&replacement_db_path))
        .await
        .expect("open replacement invocation archive sqlite");
    sqlx::query("PRAGMA user_version = 1")
        .execute(&replacement_pool)
        .await
        .expect("rewrite replacement invocation archive metadata");
    replacement_pool.close().await;
    deflate_sqlite_file_to_gzip(&replacement_db_path, archive_path)
        .expect("compress replacement invocation archive");
    let replacement_sha =
        sha256_hex_file(archive_path).expect("hash replacement invocation archive");
    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE dataset = ?2 AND file_path = ?3")
        .bind(&replacement_sha)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(archive_file_path)
        .execute(pool)
        .await
        .expect("publish replacement archive manifest SHA");
    replacement_sha
}

async fn assert_stale_replay_sha_rebuilt(
    pool: &SqlitePool,
    config: &AppConfig,
    archive_file_path: &str,
    replacement_sha: &str,
    before: (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64),
) {
    let rebuilt = materialize_historical_rollups(pool, config, false)
        .await
        .expect("materialize the replacement archive after clearing old contributions");
    assert_eq!(rebuilt.materialized_invocation_batches, 1);
    let after = load_invocation_rollup_totals(pool).await;
    assert_eq!(after, before);

    let replacement_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2 AND archive_sha256 = ?3",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_file_path)
    .bind(replacement_sha)
    .fetch_one(pool)
    .await
    .expect("count replacement replay markers");
    assert_eq!(
        replacement_marker_count,
        INVOCATION_HOURLY_ROLLUP_TARGETS.len() as i64
    );
}

#[tokio::test]
pub(crate) async fn stale_replay_sha_rebuilds_all_rollup_targets_without_double_counting() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("stale-replay-sha-full-rebuild").await;
    let (archive_path, archive_file_path) = seed_stale_replay_sha_archive(&pool, &config).await;

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize original archive");
    assert_eq!(initial.materialized_invocation_batches, 1);
    let before = load_invocation_rollup_totals(&pool).await;
    assert_eq!(before, (1, 1, 1, 1, 1, 1, 1, 1, 1, 1));

    let replacement_sha =
        replace_stale_replay_sha_archive(&pool, &temp_dir, &archive_path, &archive_file_path).await;
    sqlx::query(
        r#"
        DELETE FROM upstream_account_usage_breakdown_hourly
        WHERE upstream_account_id = 17
        "#,
    )
    .execute(&pool)
    .await
    .expect("clear only the stale usage breakdown rows");
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1 AND dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("clear only the stale usage breakdown marker");

    assert_eq!(
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
            .await
            .expect("rebuild stale archive contributions before replay"),
        1
    );
    let stale_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count cleared stale replay markers");
    assert_eq!(stale_marker_count, 0);

    assert_stale_replay_sha_rebuilt(&pool, &config, &archive_file_path, &replacement_sha, before)
        .await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn startup_summary_proof_recovery_reopens_materialized_archive_and_replays_exactly()
 {
    let (pool, config, temp_dir) = retention_test_pool_and_config("summary-proof-recovery").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "summary-proof-recovery",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "summary-proof-recovery",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"summary-proof-recovery"}"#,
            ),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize original archive");
    assert_eq!(initial.materialized_invocation_batches, 1);
    let before = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("capture original Summary rollup totals");
    assert_eq!(before, (1, 1, 1));

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1 AND dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("remove only the global Summary proof");

    assert_summary_recovery_replayed_once(&pool, &archive_file_path, before).await;

    cleanup_temp_test_dir(&temp_dir);
}
