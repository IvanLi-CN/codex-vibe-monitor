use super::*;

async fn assert_materialized_archive_is_pruned_and_not_replayed(
    pool: &SqlitePool,
    config: &AppConfig,
    archive_path: &Path,
    archived_hour_local: chrono::NaiveDateTime,
) {
    let archive_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'codex_invocations' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load archive checksum for long-term replay marker");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(pool)
        .await
        .expect("mark long-term stats ready for archive pruning");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(archive_sha256)
    .execute(pool)
    .await
    .expect("mark archive replay complete for long-term stats");
    let (archive_batch_id, archive_sha256, coverage_start, coverage_end):
        (i64, String, String, String) = sqlx::query_as(
        "SELECT id, sha256, coverage_start_at, coverage_end_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(pool)
    .await
    .expect("load archive identity for V2 cleanup proof");
    super::part_01::insert_summary_archive_snapshot_proof(
        pool,
        archive_batch_id,
        &archive_sha256,
        &coverage_start,
        &coverage_end,
        1,
    )
    .await;
    let prune_dry_run = prune_legacy_archive_batches(pool, config, true)
        .await
        .expect("dry-run prune legacy archive batches");
    assert_eq!(prune_dry_run.deleted_archive_batches, 1);
    assert!(archive_path.exists(), "dry-run should keep archive file");
    let prune_summary = prune_legacy_archive_batches(pool, config, false)
        .await
        .expect("prune legacy archive batches");
    assert_eq!(prune_summary.deleted_archive_batches, 1);
    assert!(
        !archive_path.exists(),
        "pruned legacy archive file should be removed"
    );
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await
    .expect("load long-term integrity source boundary after legacy prune");
    assert_eq!(
        integrity_source_start.as_deref(),
        Some(
            archived_hour_local
                .date()
                .succ_opt()
                .expect("archived date has successor")
                .to_string()
                .as_str(),
        ),
        "legacy archive prune must persist the exact source boundary before deleting the source"
    );
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(pool)
        .await
        .expect("count remaining archive batches after prune");
    assert_eq!(remaining_batches, 0);
    let rerun_summary = materialize_historical_rollups(pool, config, false)
        .await
        .expect("rerun materialize historical rollups after prune");
    assert_eq!(rerun_summary.materialized_invocation_batches, 0);
    let rerun_total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(pool)
    .await
    .expect("load hourly totals after rerun");
    assert_eq!(rerun_total_count, 1);
}

async fn assert_pruned_detail_rollup(pool: &SqlitePool, config: &AppConfig, archive_path: &Path) {
    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(pool)
    .await
    .expect("load invocation hourly total count after partial materialization");
    assert_eq!(total_count, 1);
    let breakdown_replay_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay
         WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target = ?2",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(pool)
    .await
    .expect("load usage breakdown replay marker for pruned archive");
    assert_eq!(breakdown_replay_markers, 1);
    let replayed_archive_sha256: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay
         WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target = ?2",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(pool)
    .await
    .expect("load usage breakdown replay identity");
    let manifest_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(pool)
    .await
    .expect("load replayed archive manifest identity");
    assert_eq!(
        replayed_archive_sha256.as_deref(),
        Some(manifest_sha256.as_str())
    );
    let missing_breakdown_archives =
        crate::stats::load_invocation_archives_missing_effective_rollup_target(
            pool,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            None,
        )
        .await
        .expect("load archives missing usage breakdown replay marker");
    assert!(missing_breakdown_archives.is_empty());
    let breakdown_row: (String, Option<i64>, String, String, i64, i64, i64, f64) = sqlx::query_as(
        "SELECT upstream_account_key, upstream_account_id, normalized_model,
                    normalized_reasoning_effort, request_count, success_count,
                    performance_total_tokens, cost_unknown
             FROM upstream_account_usage_breakdown_hourly",
    )
    .fetch_one(pool)
    .await
    .expect("load usage breakdown rollup row for pruned archive");
    assert_eq!(breakdown_row.0, "upstream:17");
    assert_eq!(breakdown_row.1, Some(17));
    assert_eq!(breakdown_row.2, "unknown");
    assert_eq!(breakdown_row.3, "");
    assert_eq!(breakdown_row.4, 1);
    assert_eq!(breakdown_row.5, 1);
    assert_eq!(breakdown_row.6, 12);
    assert_f64_close(breakdown_row.7, 0.12);
    let keyed_replay_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay
         WHERE dataset = 'codex_invocations' AND file_path = ?1
           AND target IN (?2, ?3, ?4)",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE)
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS)
    .bind(HOURLY_ROLLUP_TARGET_STICKY_KEYS)
    .fetch_one(pool)
    .await
    .expect("load keyed replay markers");
    assert_eq!(keyed_replay_markers, 0);
    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches
         WHERE dataset = 'codex_invocations' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load archive batch materialized timestamp for pruned detail archive");
    assert!(materialized_at.is_none());
    let snapshot = load_historical_rollup_backfill_snapshot(pool, config)
        .await
        .expect("load historical rollup snapshot after pruned detail materialization");
    assert_eq!(snapshot.legacy_archive_pending, 1);
    assert_eq!(
        snapshot.alert_level,
        HistoricalRollupBackfillAlertLevel::Critical
    );
}

async fn assert_next_bounded_materialization(
    pool: &SqlitePool,
    config: &AppConfig,
    second_archive_path: &Path,
    skipped_archive_count: usize,
    expected_total_count: Option<i64>,
) {
    let second_archive_materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(second_archive_path.to_string_lossy().to_string())
    .fetch_one(pool)
    .await
    .expect("load second archive materialized timestamp after bounded pass");
    assert!(second_archive_materialized_at.is_none());
    let summary = materialize_historical_rollups_bounded_from_skip(
        pool,
        config,
        false,
        Some(1),
        None,
        skipped_archive_count,
    )
    .await
    .expect("materialize next archive on the bounded pass");
    assert_eq!(summary.materialized_invocation_batches, 1);
    if let Some(expected_total_count) = expected_total_count {
        let total_count: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
        )
        .bind(SOURCE_PROXY)
        .fetch_one(pool)
        .await
        .expect("load invocation hourly total count after bounded pass");
        assert_eq!(total_count, expected_total_count);
    }
}

fn bounded_fully_blocked_occurred_times(config: &AppConfig) -> (String, String) {
    let first_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 62) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid first archived local hour");
    let second_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 31) as i64))
    .and_hms_opt(9, 0, 0)
    .expect("valid second archived local hour");
    (
        format_naive(
            first_archive_hour_local
                .checked_add_signed(ChronoDuration::minutes(10))
                .expect("valid first archived occurred_at"),
        ),
        format_naive(
            second_archive_hour_local
                .checked_add_signed(ChronoDuration::minutes(10))
                .expect("valid second archived occurred_at"),
        ),
    )
}

async fn seed_resume_progress_rollup(
    pool: &SqlitePool,
    archive_path: &Path,
    bucket_start_epoch: i64,
) {
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch, source, total_count, success_count, failure_count,
            total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms,
            first_byte_max_ms, first_byte_histogram
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .bind(12_i64)
    .bind(0.12_f64)
    .bind(1_i64)
    .bind(120.0_f64)
    .bind(120.0_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]")
    .execute(pool)
    .await
    .expect("seed previously materialized first row rollup");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id, updated_at)
         VALUES (?1, ?2, ?3, datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(1_i64)
    .execute(pool)
    .await
    .expect("seed historical rollup archive cursor");
}

async fn assert_resumed_rollup_totals(pool: &SqlitePool, archive_path: &Path) {
    let (total_count, total_tokens): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(total_count), 0), COALESCE(SUM(total_tokens), 0)
         FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(pool)
    .await
    .expect("load resumed invocation rollup totals");
    assert_eq!(total_count, 2);
    assert_eq!(total_tokens, 33);
    let progress_cursor: Option<i64> = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_archive_progress WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_optional(pool)
    .await
    .expect("load archive progress after resumed materialization");
    assert_eq!(progress_cursor, None);
}
use serde_json::json;

#[tokio::test]
pub(crate) async fn cleanup_expired_attempt_archive_keeps_a_missing_manifest_as_long_term_source_evidence()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("cleanup-missing-attempt-manifest").await;
    let missing_archive_path = temp_dir.join("missing-attempt-source.sqlite.gz");
    let coverage_end_at = shanghai_local_days_ago(400, 9, 0, 0);

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (1, 'pool_upstream_request_attempts', ?1, ?2, 'missing-attempt-sha', 1, ?3, ?4, ?4, '2000-01-01 00:00:00', datetime('now'), datetime('now'))
        "#,
    )
    .bind(&coverage_end_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert missing expired attempt archive manifest");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term statistics ready for attempt cleanup");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
        LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, 'missing-attempt-sha')",
        )
        .bind(target)
        .bind(missing_archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("mark missing attempt archive replay complete");
    }

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("retain missing attempt source manifest during expiry cleanup");
    assert_eq!(deleted, 0);
    let remaining_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&pool)
    .await
    .expect("count retained missing attempt manifest");
    assert_eq!(remaining_batches, 1);
    let remaining_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
    )
    .bind(missing_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("count retained missing attempt markers");
    assert_eq!(remaining_marker_count, 3);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn backfill_invocation_archive_expiries_uses_coverage_end_at() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-expiry-backfill").await;
    let coverage_end_at = shanghai_local_days_ago(45, 18, 30, 0);
    let created_at = format_utc_iso(Utc::now());

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), ?10)
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind(&coverage_end_at[..7])
    .bind(
        temp_dir
            .join("expiry.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert archive batch for expiry backfill");

    let updated = backfill_invocation_archive_expiries(&pool, &config)
        .await
        .expect("backfill archive expiry");
    assert_eq!(updated, 1);

    let expected = shanghai_archive_expiry_from_reference_timestamp(
        &coverage_end_at,
        config.invocation_archive_ttl_days,
    )
    .expect("compute expected archive expiry");
    let actual: Option<String> =
        sqlx::query_scalar("SELECT archive_expires_at FROM archive_batches WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("load archive expiry");
    assert_eq!(actual.as_deref(), Some(expected.as_str()));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn archive_expiry_backfill_respects_the_retention_candidate_budget() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-expiry-backfill-budget").await;
    config.retention_batch_rows = 2;
    let coverage_end_at = shanghai_local_days_ago(45, 18, 30, 0);
    let created_at = format_utc_iso(Utc::now());

    for id in 1_i64..=3 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                coverage_start_at, coverage_end_at, historical_rollups_materialized_at, created_at
            )
            VALUES (?1, 'codex_invocations', ?2, ?3, 'expiry-budget', 1, ?4,
                    ?5, ?5, datetime('now'), ?6)
            "#,
        )
        .bind(id)
        .bind(&coverage_end_at[..7])
        .bind(
            temp_dir
                .join(format!("expiry-budget-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(&coverage_end_at)
        .bind(&created_at)
        .execute(&pool)
        .await
        .expect("insert expiry backfill candidate");
    }

    let updated = backfill_invocation_archive_expiries(&pool, &config)
        .await
        .expect("backfill bounded archive expiries");
    let pending: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches WHERE archive_expires_at IS NULL")
            .fetch_one(&pool)
            .await
            .expect("count remaining archive expiry candidates");
    assert_eq!(updated, 2);
    assert_eq!(pending, 1);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn pending_legacy_invocation_archives_do_not_expire_before_materialization() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("archive-expiry-pending-legacy").await;
    config.invocation_archive_ttl_days = 0;
    let coverage_end_at = shanghai_local_days_ago(120, 8, 0, 0);
    let archive_path = temp_dir.join("pending-legacy.sqlite.gz");
    write_gzip_test_file(&archive_path, b"pending-legacy");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind(&coverage_end_at[..7])
    .bind(archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert pending legacy archive batch");

    let updated = backfill_invocation_archive_expiries(&pool, &config)
        .await
        .expect("backfill should skip pending legacy archive");
    assert_eq!(updated, 0);

    let dry_run_deleted = cleanup_expired_archive_batches(&pool, &config, true)
        .await
        .expect("dry-run should also skip pending legacy archive");
    assert_eq!(dry_run_deleted, 0);

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup should keep pending legacy archive");
    assert_eq!(deleted, 0);
    assert!(archive_path.exists());

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining pending legacy batches");
    assert_eq!(remaining_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn pending_legacy_forward_proxy_archives_do_not_expire_before_materialization() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("archive-expiry-pending-forward-proxy").await;
    let coverage_end_at = format_utc_iso(Utc::now() - ChronoDuration::days(120));
    let archive_path = temp_dir.join("pending-forward-proxy.sqlite.gz");
    write_gzip_test_file(&archive_path, b"pending-forward-proxy");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            archive_expires_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind("forward_proxy_attempts")
    .bind(&coverage_end_at[..7])
    .bind(archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .bind("2025-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("insert pending forward-proxy archive batch");

    let dry_run_deleted = cleanup_expired_archive_batches(&pool, &config, true)
        .await
        .expect("dry-run should skip pending forward-proxy archive");
    assert_eq!(dry_run_deleted, 0);

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup should keep pending forward-proxy archive");
    assert_eq!(deleted, 0);
    assert!(archive_path.exists());

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining pending forward-proxy batches");
    assert_eq!(remaining_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn prune_legacy_archive_batches_keeps_missing_invocation_manifest_while_backfill_pending()
 {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("prune-missing-invocation-manifest-pending").await;
    let missing_archive_path = temp_dir.join("missing-manifest.sqlite.gz");
    let coverage_end_at =
        shanghai_local_days_ago((config.invocation_max_days + 30) as i64, 9, 0, 0);
    let created_at = format_utc_iso(Utc::now());

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(771_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Pending manifest account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert pending manifest account");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind("codex_invocations")
    .bind(&coverage_end_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert missing invocation archive metadata");
    sqlx::query(
        r#"
        INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at)
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(1_i64)
    .bind(771_i64)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert pending manifest row");

    let prune_summary = prune_legacy_archive_batches(&pool, &config, false)
        .await
        .expect("prune should keep missing invocation manifest metadata while pending");
    assert_eq!(prune_summary.deleted_archive_batches, 0);
    assert_eq!(prune_summary.skipped_unmaterialized_batches, 1);

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches");
    assert_eq!(remaining_batches, 1);
    let remaining_manifest_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM archive_batch_upstream_activity")
            .fetch_one(&pool)
            .await
            .expect("count remaining manifest rows");
    assert_eq!(remaining_manifest_rows, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn prune_legacy_archive_batches_keeps_a_materialized_missing_manifest_after_source_loss()
 {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("prune-missing-invocation-manifest-finalize").await;
    let missing_archive_path = temp_dir.join("missing-finalizable-manifest.sqlite.gz");
    let coverage_end_at =
        shanghai_local_days_ago((config.invocation_max_days + 30) as i64, 9, 0, 0);

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id,
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now'))
        "#,
    )
    .bind(1_i64)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&coverage_end_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("missing-finalizable-sha")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert missing replayed invocation archive metadata");
    sqlx::query(
        "UPDATE long_term_stats_state SET status = 'error', last_error = 'terminal integrity proof reconciliation is incomplete' WHERE id = 1",
    )
        .execute(&pool)
        .await
        .expect("mark long-term statistics unavailable after source loss");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("missing-finalizable-sha")
    .execute(&pool)
    .await
    .expect("insert long-term replay marker");

    let prune_summary = prune_legacy_archive_batches(&pool, &config, false)
        .await
        .expect("retain missing source manifest");
    assert_eq!(prune_summary.deleted_archive_batches, 0);
    assert_eq!(prune_summary.skipped_unmaterialized_batches, 1);
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count retained archive batches");
    assert_eq!(remaining_batches, 1);
    let remaining_markers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .fetch_one(&pool)
    .await
    .expect("count retained replay markers");
    assert_eq!(remaining_markers, 1);

    let status: String =
        sqlx::query_scalar("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("load durable source-loss status");
    assert_eq!(status, "error");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_prune_preserves_upstream_account_id_for_archive_manifest() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("prune-preserve-upstream-account").await;
    config.invocation_archive_ttl_days = 365;
    let occurred_at = shanghai_local_days_ago(31, 14, 0, 0);

    insert_retention_invocation(
        &pool,
        "prune-preserve-upstream-account",
        &occurred_at,
        SOURCE_XY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\",\"upstreamAccountId\":771}"),
        "{\"ok\":true}",
        None,
        None,
        Some(111),
        Some(0.5),
    )
    .await;

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("prune invocation details while preserving upstream account id");

    let pruned_payload: Option<String> =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("prune-preserve-upstream-account")
            .fetch_one(&pool)
            .await
            .expect("load pruned payload");
    let pruned_payload = serde_json::from_str::<Value>(
        pruned_payload
            .as_deref()
            .expect("payload should keep upstream account id"),
    )
    .expect("parse pruned payload");
    assert_eq!(pruned_payload, json!({ "upstreamAccountId": 771 }));

    let archived_occurred_at = shanghai_local_days_ago(91, 14, 0, 0);
    sqlx::query("UPDATE codex_invocations SET occurred_at = ?1 WHERE invoke_id = ?2")
        .bind(&archived_occurred_at)
        .bind("prune-preserve-upstream-account")
        .execute(&pool)
        .await
        .expect("age pruned invocation into archive window");

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive pruned invocation");

    let manifest_row = sqlx::query_as::<_, (i64, String)>(
        r#"
        SELECT account_id, last_activity_at
        FROM archive_batch_upstream_activity
        WHERE account_id = ?1
        LIMIT 1
        "#,
    )
    .bind(771_i64)
    .fetch_one(&pool)
    .await
    .expect("load archive upstream activity manifest row");
    assert_eq!(manifest_row.0, 771);
    assert_eq!(manifest_row.1, archived_occurred_at);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_marks_batches_and_prune_removes_files() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-materialize-prune").await;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 2) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "historical-rollup-materialize-prune",
        &[(
            1_i64,
            "historical-rollup-materialize-prune",
            archived_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            12_i64,
            0.12_f64,
            Some(120.0),
        )],
    )
    .await;

    let snapshot_before = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup backlog before materialization");
    assert_eq!(snapshot_before.legacy_archive_pending, 1);
    assert!(snapshot_before.pending_buckets >= 1);

    let dry_run_summary = materialize_historical_rollups(&pool, &config, true)
        .await
        .expect("dry-run materialize historical rollups");
    assert_eq!(dry_run_summary.scanned_archive_batches, 1);
    assert_eq!(dry_run_summary.materialized_archive_batches, 1);

    let summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize historical rollups");
    assert_eq!(summary.materialized_invocation_batches, 1);
    assert_eq!(summary.materialized_forward_proxy_batches, 0);

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load materialized invocation hourly total count");
    assert_eq!(total_count, 1);

    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE dataset = 'codex_invocations' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archive batch materialized timestamp");
    assert!(materialized_at.is_some());

    let snapshot_after = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup backlog after materialization");
    assert_eq!(snapshot_after.legacy_archive_pending, 0);

    assert_materialized_archive_is_pruned_and_not_replayed(
        &pool,
        &config,
        &archive_path,
        archived_hour_local,
    )
    .await;

    let rerun_summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("rerun materialize historical rollups after prune");
    assert_eq!(rerun_summary.materialized_invocation_batches, 0);
    let rerun_total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load hourly totals after rerun");
    assert_eq!(rerun_total_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_skips_already_materialized_batches() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-skip-materialized").await;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 2) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let bucket_start_epoch =
        invocation_bucket_start_epoch(&archived_occurred_at).expect("invocation bucket epoch");

    seed_invocation_archive_batch(
        &pool,
        &config,
        "historical-rollup-skip-materialized",
        &[(
            1_i64,
            "historical-rollup-skip-materialized",
            archived_occurred_at.as_str(),
            SOURCE_PROXY,
            "success",
            12_i64,
            0.12_f64,
            Some(120.0),
        )],
    )
    .await;

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .execute(&pool)
    .await
    .expect("mark archive batch already materialized");
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
    .bind(12_i64)
    .bind(0.12_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&pool)
    .await
    .expect("seed already materialized invocation rollup");

    let snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load snapshot for already materialized batch");
    assert_eq!(snapshot.legacy_archive_pending, 0);

    let summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize should skip already materialized archive batch");
    assert_eq!(summary.materialized_invocation_batches, 0);

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation rollup total count after skipped replay");
    assert_eq!(total_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_replays_breakdown_for_pruned_detail_archives() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-pruned-detail-pending").await;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 2) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-pruned-detail-pending",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-pruned-detail-pending",
            occurred_at: archived_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17}"#),
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize historical rollups with pruned detail archive");
    assert_eq!(summary.materialized_invocation_batches, 0);
    assert_eq!(summary.blocked_archive_batches, 1);

    assert_pruned_detail_rollup(&pool, &config, &archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_bounded_counts_partially_blocked_archive_budget()
{
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-bounded-partial-budget").await;
    let (first_occurred_at, second_occurred_at) = bounded_fully_blocked_occurred_times(&config);

    seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-bounded-pruned-first",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-bounded-pruned-first",
            occurred_at: first_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17}"#),
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let second_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-bounded-full-second",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-bounded-full-second",
            occurred_at: second_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 21,
            cost: 0.21,
            ttfb_ms: Some(210.0),
            payload: Some(r#"{"upstreamAccountId":18}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let first_summary =
        materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
            .await
            .expect("materialize one partially blocked archive within budget");
    assert_eq!(first_summary.materialized_invocation_batches, 0);

    let total_count_after_first: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly total count after first bounded pass");
    assert_eq!(total_count_after_first, 1);

    assert_next_bounded_materialization(
        &pool,
        &config,
        &second_archive_path,
        first_summary.scanned_archive_batches,
        Some(2),
    )
    .await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_bounded_counts_fully_blocked_archive_budget() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-bounded-fully-blocked-budget").await;
    let (first_occurred_at, second_occurred_at) = bounded_fully_blocked_occurred_times(&config);

    let first_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-bounded-fully-blocked-first",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-bounded-fully-blocked-first",
            occurred_at: first_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17}"#),
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let second_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-bounded-fully-blocked-second",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-bounded-fully-blocked-second",
            occurred_at: second_occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 21,
            cost: 0.21,
            ttfb_ms: Some(210.0),
            payload: Some(r#"{"upstreamAccountId":18}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
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
        .bind(first_archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("insert replay marker for non-keyed invocation target");
    }

    let first_summary =
        materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
            .await
            .expect("fully blocked archive should still consume the one-archive budget");
    assert_eq!(first_summary.materialized_invocation_batches, 0);
    assert_eq!(first_summary.blocked_archive_batches, 1);

    assert_next_bounded_materialization(
        &pool,
        &config,
        &second_archive_path,
        first_summary.scanned_archive_batches,
        None,
    )
    .await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn replay_invocation_archives_into_hourly_rollups_respects_caller_elapsed_budget()
{
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-shared-elapsed-budget").await;
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
        "historical-rollup-shared-elapsed-budget",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-shared-elapsed-budget",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(r#"{"upstreamAccountId":17}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let mut tx = pool.begin().await.expect("begin transaction");
    let summary = replay_invocation_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now() - Duration::from_secs(1),
        Some(1),
        Some(Duration::from_millis(1)),
        0,
    )
    .await
    .expect("replay bounded invocation archives");
    tx.rollback().await.expect("rollback replay transaction");

    assert_eq!(summary.scanned_batches, 0);
    assert_eq!(summary.materialized_batches, 0);
    assert_eq!(summary.budget_consumed_batches, 0);

    let still_pending: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load archive materialized timestamp after bounded replay");
    assert!(
        still_pending.is_none(),
        "expired caller elapsed budget should leave the archive pending for a later pass"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_resumes_invocation_archive_from_saved_progress_cursor()
 {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-resume-progress-cursor").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let bucket_start_epoch = align_bucket_epoch(
        local_naive_to_utc(archive_hour_local, Shanghai).timestamp(),
        3_600,
        0,
    );
    let first_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let second_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(20))
            .expect("valid second archived occurred_at"),
    );

    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-resume-progress-cursor",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: "historical-rollup-resume-progress-cursor-1",
                occurred_at: first_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":17}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: "historical-rollup-resume-progress-cursor-2",
                occurred_at: second_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 21,
                cost: 0.21,
                ttfb_ms: Some(210.0),
                payload: Some(r#"{"upstreamAccountId":18}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
        ],
    )
    .await;

    seed_resume_progress_rollup(&pool, &archive_path, bucket_start_epoch).await;

    let summary = materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
        .await
        .expect("resume archive replay from saved cursor");
    assert_eq!(summary.materialized_invocation_batches, 1);

    assert_resumed_rollup_totals(&pool, &archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

struct BreakdownPrefixFixture {
    pool: SqlitePool,
    config: AppConfig,
    temp_dir: PathBuf,
    archive_path: PathBuf,
    bucket_start_epoch: i64,
    first_occurred_at: String,
    second_occurred_at: String,
}

async fn create_breakdown_prefix_fixture() -> BreakdownPrefixFixture {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-breakdown-prefix-catchup").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let bucket_start_epoch = align_bucket_epoch(
        local_naive_to_utc(archive_hour_local, Shanghai).timestamp(),
        3_600,
        0,
    );
    let first_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let second_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(20))
            .expect("valid second archived occurred_at"),
    );
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-breakdown-prefix-catchup",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: "historical-rollup-breakdown-prefix-catchup-1",
                occurred_at: first_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 12,
                cost: 0.12,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":17}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: "historical-rollup-breakdown-prefix-catchup-2",
                occurred_at: second_occurred_at.as_str(),
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: 21,
                cost: 0.21,
                ttfb_ms: Some(210.0),
                payload: Some(r#"{"upstreamAccountId":17}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            },
        ],
    )
    .await;
    BreakdownPrefixFixture {
        pool,
        config,
        temp_dir,
        archive_path,
        bucket_start_epoch,
        first_occurred_at,
        second_occurred_at,
    }
}

async fn update_breakdown_prefix_archive(fixture: &BreakdownPrefixFixture) {
    let archive_db_path = fixture
        .config
        .archive_dir
        .join("historical-rollup-breakdown-prefix-catchup.sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open mutable invocation archive sqlite");
    for (id, cost_input, cost_output) in [(1_i64, 0.05_f64, 0.07_f64), (2_i64, 0.08_f64, 0.13_f64)]
    {
        sqlx::query(
            "UPDATE codex_invocations
             SET model = 'gpt-5', cost_input = ?2, cost_cache_write = 0.0,
                 cost_cache_read = 0.0, cost_output = ?3, cost_reasoning = 0.0
             WHERE id = ?1",
        )
        .bind(id)
        .bind(cost_input)
        .bind(cost_output)
        .execute(&archive_pool)
        .await
        .expect("write archive usage breakdown detail fields");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &fixture.archive_path)
        .expect("refresh archive gzip with usage breakdown detail fields");
}

async fn seed_breakdown_prefix_rollups(fixture: &BreakdownPrefixFixture) {
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens,
          total_cost, first_byte_sample_count, first_byte_sum_ms, first_byte_max_ms, first_byte_histogram)
         VALUES (?1, ?2, 1, 1, 0, 12, 0.12, 1, 120.0, 120.0,
                 '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]')",
    )
    .bind(fixture.bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .execute(&fixture.pool)
    .await
    .expect("seed historical invocation rollup prefix");
    sqlx::query(
        "INSERT INTO upstream_account_usage_hourly
         (bucket_start_epoch, upstream_account_id, request_count, total_tokens, total_cost,
          input_tokens, output_tokens, cache_input_tokens, success_count, failure_count,
          first_seen_at, last_seen_at)
         VALUES (?1, ?2, 1, 12, 0.12, 0, 12, 0, 1, 0, ?3, ?4)",
    )
    .bind(fixture.bucket_start_epoch)
    .bind(17_i64)
    .bind(&fixture.first_occurred_at)
    .bind(&fixture.first_occurred_at)
    .execute(&fixture.pool)
    .await
    .expect("seed historical account usage prefix");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_progress
         (dataset, file_path, cursor_id, updated_at)
         VALUES (?1, ?2, ?3, datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(fixture.archive_path.to_string_lossy().to_string())
    .bind(1_i64)
    .execute(&fixture.pool)
    .await
    .expect("seed shared replay cursor from legacy targets");
}

async fn assert_breakdown_prefix_catchup(fixture: &BreakdownPrefixFixture) {
    let summary = materialize_historical_rollups_bounded(
        &fixture.pool,
        &fixture.config,
        false,
        Some(1),
        None,
    )
    .await
    .expect("materialize historical rollups with breakdown prefix catch-up");
    assert_eq!(summary.materialized_invocation_batches, 1);
    let totals = sqlx::query_as::<_, (i64, f64, f64, f64)>(
        "SELECT COALESCE(SUM(request_count), 0), COALESCE(SUM(cost_input), 0),
                COALESCE(SUM(cost_output), 0), COALESCE(SUM(cost_unknown), 0)
         FROM upstream_account_usage_breakdown_hourly
         WHERE upstream_account_id = ?1 AND normalized_model = 'gpt-5'",
    )
    .bind(17_i64)
    .fetch_one(&fixture.pool)
    .await
    .expect("load historical usage breakdown totals after catch-up");
    assert_eq!(totals.0, 2);
    assert_f64_close(totals.1, 0.13);
    assert_f64_close(totals.2, 0.20);
    assert_f64_close(totals.3, 0.0);
    let cursor: Option<i64> = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_archive_progress
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET)
    .bind(fixture.archive_path.to_string_lossy().to_string())
    .fetch_optional(&fixture.pool)
    .await
    .expect("load breakdown-specific archive progress after materialization");
    assert_eq!(cursor, None);
}

#[tokio::test]
pub(crate) async fn materialize_historical_rollups_backfills_usage_breakdown_prefix_behind_shared_cursor()
 {
    let fixture = create_breakdown_prefix_fixture().await;
    update_breakdown_prefix_archive(&fixture).await;
    seed_breakdown_prefix_rollups(&fixture).await;
    assert_breakdown_prefix_catchup(&fixture).await;
    cleanup_temp_test_dir(&fixture.temp_dir);
}
