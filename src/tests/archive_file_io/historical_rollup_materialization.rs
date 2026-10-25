use super::archive_backfill_test_support::{
    insert_summary_archive_snapshot_proof, write_valid_invocation_archive,
};
use super::*;
use serde_json::json;
#[tokio::test]
async fn materialize_historical_rollups_marks_batches_and_prune_removes_files() {
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

    let archive_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'codex_invocations' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archive checksum for long-term replay marker");
    sqlx::query("UPDATE long_term_stats_state SET status = 'ready' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats ready for archive pruning");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(archive_sha256)
    .execute(&pool)
    .await
    .expect("mark archive replay complete for long-term stats");
    let (archive_batch_id, archive_sha256, coverage_start, coverage_end):
        (i64, String, String, String) = sqlx::query_as(
            "SELECT id, sha256, coverage_start_at, coverage_end_at FROM archive_batches WHERE file_path = ?1",
        )
        .bind(archive_path.to_string_lossy().to_string())
        .fetch_one(&pool)
        .await
        .expect("load archive identity for V2 cleanup proof");
    insert_summary_archive_snapshot_proof(
        &pool,
        archive_batch_id,
        &archive_sha256,
        &coverage_start,
        &coverage_end,
        1,
    )
    .await;

    let prune_dry_run = prune_legacy_archive_batches(&pool, &config, true)
        .await
        .expect("dry-run prune legacy archive batches");
    assert_eq!(prune_dry_run.deleted_archive_batches, 1);
    assert!(archive_path.exists(), "dry-run should keep archive file");

    let prune_summary = prune_legacy_archive_batches(&pool, &config, false)
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
    .fetch_one(&pool)
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
                .as_str()
        ),
        "legacy archive prune must persist the exact source boundary before deleting the source"
    );

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches after prune");
    assert_eq!(remaining_batches, 0);

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
async fn materialize_historical_rollups_skips_already_materialized_batches() {
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
async fn materialize_historical_rollups_replays_breakdown_for_pruned_detail_archives() {
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

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly total count after partial materialization");
    assert_eq!(total_count, 1);

    let breakdown_replay_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target = ?2
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("load usage breakdown replay marker for pruned archive");
    assert_eq!(breakdown_replay_markers, 1);
    let replayed_archive_sha256: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay \
         WHERE dataset = 'codex_invocations' AND file_path = ?1 AND target = ?2",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("load usage breakdown replay identity");
    let manifest_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load replayed archive manifest identity");
    assert_eq!(
        replayed_archive_sha256.as_deref(),
        Some(manifest_sha256.as_str())
    );

    let missing_breakdown_archives =
        crate::stats::load_invocation_archives_missing_effective_rollup_target(
            &pool,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            None,
        )
        .await
        .expect("load archives missing usage breakdown replay marker");
    assert!(
        missing_breakdown_archives.is_empty(),
        "replayed usage breakdown target should not trigger read-side archive fallback"
    );

    let breakdown_row: (String, Option<i64>, String, String, i64, i64, i64, f64) = sqlx::query_as(
        r#"
            SELECT
                upstream_account_key,
                upstream_account_id,
                normalized_model,
                normalized_reasoning_effort,
                request_count,
                success_count,
                performance_total_tokens,
                cost_unknown
            FROM upstream_account_usage_breakdown_hourly
            "#,
    )
    .fetch_one(&pool)
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
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target IN (?2, ?3, ?4)
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE)
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS)
    .bind(HOURLY_ROLLUP_TARGET_STICKY_KEYS)
    .fetch_one(&pool)
    .await
    .expect("load keyed replay markers");
    assert_eq!(keyed_replay_markers, 0);

    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE dataset = 'codex_invocations' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archive batch materialized timestamp for pruned detail archive");
    assert!(materialized_at.is_none());

    let snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup snapshot after pruned detail materialization");
    assert_eq!(snapshot.legacy_archive_pending, 1);
    assert_eq!(
        snapshot.alert_level,
        HistoricalRollupBackfillAlertLevel::Critical
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_bounded_counts_partially_blocked_archive_budget() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-bounded-partial-budget").await;
    let first_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 62) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid first archived local hour");
    let second_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 31) as i64))
    .and_hms_opt(9, 0, 0)
    .expect("valid second archived local hour");
    let first_occurred_at = format_naive(
        first_archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let second_occurred_at = format_naive(
        second_archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid second archived occurred_at"),
    );

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

    let second_archive_materialized_at_after_first: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(second_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load second archive materialized timestamp after first bounded pass");
    assert!(
        second_archive_materialized_at_after_first.is_none(),
        "the second archive should not be materialized once the first partially blocked archive consumes the one-archive budget"
    );

    let second_summary =
        materialize_historical_rollups_bounded_from_skip(
            &pool,
            &config,
            false,
            Some(1),
            None,
            first_summary.scanned_archive_batches,
        )
        .await
        .expect("materialize second archive on the next bounded pass after skipping the previously scanned backlog");
    assert_eq!(second_summary.materialized_invocation_batches, 1);

    let total_count_after_second: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly total count after second bounded pass");
    assert_eq!(total_count_after_second, 2);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_bounded_counts_fully_blocked_archive_budget() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-bounded-fully-blocked-budget").await;
    let first_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 62) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid first archived local hour");
    let second_archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 31) as i64))
    .and_hms_opt(9, 0, 0)
    .expect("valid second archived local hour");
    let first_occurred_at = format_naive(
        first_archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid first archived occurred_at"),
    );
    let second_occurred_at = format_naive(
        second_archive_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid second archived occurred_at"),
    );

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

    let second_archive_materialized_at_after_first: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(second_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load second archive materialized timestamp after fully blocked bounded pass");
    assert!(
        second_archive_materialized_at_after_first.is_none(),
        "a fully blocked first archive should still consume the one-archive budget"
    );

    let second_summary = materialize_historical_rollups_bounded_from_skip(
        &pool,
        &config,
        false,
        Some(1),
        None,
        first_summary.scanned_archive_batches,
    )
    .await
    .expect("second bounded pass should reach the next archive after skipping the blocked one");
    assert_eq!(second_summary.materialized_invocation_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn replay_invocation_archives_into_hourly_rollups_respects_caller_elapsed_budget() {
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
async fn materialize_historical_rollups_resumes_invocation_archive_from_saved_progress_cursor() {
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
    .bind(12_i64)
    .bind(0.12_f64)
    .bind(1_i64)
    .bind(120.0_f64)
    .bind(120.0_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]")
    .execute(&pool)
    .await
    .expect("seed previously materialized first row rollup");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_progress (
            dataset,
            file_path,
            cursor_id,
            updated_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("seed historical rollup archive cursor");

    let summary = materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
        .await
        .expect("resume archive replay from saved cursor");
    assert_eq!(summary.materialized_invocation_batches, 1);

    let (total_count, total_tokens): (i64, i64) = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            COALESCE(SUM(total_count), 0) AS total_count,
            COALESCE(SUM(total_tokens), 0) AS total_tokens
        FROM invocation_rollup_hourly
        WHERE source = ?1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load resumed invocation rollup totals");
    assert_eq!(
        total_count, 2,
        "resume should only apply the remaining archived row instead of replaying the whole archive"
    );
    assert_eq!(total_tokens, 33);

    let progress_cursor: Option<i64> = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_archive_progress WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_optional(&pool)
    .await
    .expect("load archive progress after resumed materialization");
    assert_eq!(progress_cursor, None);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_backfills_usage_breakdown_prefix_behind_shared_cursor() {
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
    let archive_db_path = config
        .archive_dir
        .join("historical-rollup-breakdown-prefix-catchup.sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open mutable invocation archive sqlite");
    for (id, cost_input, cost_output) in [(1_i64, 0.05_f64, 0.07_f64), (2_i64, 0.08_f64, 0.13_f64)]
    {
        sqlx::query(
            r#"
            UPDATE codex_invocations
            SET model = 'gpt-5',
                cost_input = ?2,
                cost_cache_write = 0.0,
                cost_cache_read = 0.0,
                cost_output = ?3,
                cost_reasoning = 0.0
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .bind(cost_input)
        .bind(cost_output)
        .execute(&archive_pool)
        .await
        .expect("write archive usage breakdown detail fields");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("refresh archive gzip with usage breakdown detail fields");
    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
        .bind(sha256_hex_file(&archive_path).expect("hash refreshed usage breakdown archive"))
        .bind(archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("refresh usage breakdown archive manifest hash");

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
    .bind(12_i64)
    .bind(0.12_f64)
    .bind(1_i64)
    .bind(120.0_f64)
    .bind(120.0_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1]")
    .execute(&pool)
    .await
    .expect("seed historical invocation rollup prefix");
    sqlx::query(
        r#"
        INSERT INTO upstream_account_usage_hourly (
            bucket_start_epoch,
            upstream_account_id,
            request_count,
            total_tokens,
            total_cost,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            success_count,
            failure_count,
            first_seen_at,
            last_seen_at
        )
        VALUES (?1, ?2, 1, 12, 0.12, 0, 12, 0, 1, 0, ?3, ?4)
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(17_i64)
    .bind(&first_occurred_at)
    .bind(&first_occurred_at)
    .execute(&pool)
    .await
    .expect("seed historical account usage prefix");

    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_progress (
            dataset,
            file_path,
            cursor_id,
            updated_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("seed shared replay cursor from legacy targets");

    let summary = materialize_historical_rollups_bounded(&pool, &config, false, Some(1), None)
        .await
        .expect("materialize historical rollups with breakdown prefix catch-up");
    assert_eq!(summary.materialized_invocation_batches, 1);

    let (request_count, cost_input, cost_output, cost_unknown): (i64, f64, f64, f64) =
        sqlx::query_as::<_, (i64, f64, f64, f64)>(
            r#"
        SELECT
            COALESCE(SUM(request_count), 0) AS request_count,
            COALESCE(SUM(cost_input), 0) AS cost_input,
            COALESCE(SUM(cost_output), 0) AS cost_output,
            COALESCE(SUM(cost_unknown), 0) AS cost_unknown
        FROM upstream_account_usage_breakdown_hourly
        WHERE upstream_account_id = ?1
          AND normalized_model = 'gpt-5'
        "#,
        )
        .bind(17_i64)
        .fetch_one(&pool)
        .await
        .expect("load historical usage breakdown totals after catch-up");
    assert_eq!(request_count, 2);
    assert_f64_close(cost_input, 0.13_f64);
    assert_f64_close(cost_output, 0.20_f64);
    assert_f64_close(cost_unknown, 0.0_f64);

    let breakdown_progress_cursor: Option<i64> = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_archive_progress WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET)
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_optional(&pool)
    .await
    .expect("load breakdown-specific archive progress after materialization");
    assert_eq!(breakdown_progress_cursor, None);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_bounded_skips_live_replay_when_elapsed_budget_is_zero() {
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

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly total count after zero-budget bounded pass");
    assert_eq!(
        total_count, 0,
        "zero elapsed budget should leave live rollup catch-up for a later pass"
    );

    let live_cursor: Option<i64> =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .fetch_optional(&pool)
            .await
            .expect("load invocation live cursor after zero-budget bounded pass");
    assert_eq!(
        live_cursor, None,
        "zero elapsed budget should not advance the shared live replay cursor"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_marks_replayed_batches_as_materialized_after_usage_breakdown_replay()
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

    let forward_proxy_archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'forward_proxy_attempts' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load forward-proxy archive path");
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
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(&forward_proxy_archive_path)
    .execute(&pool)
    .await
    .expect("insert forward-proxy replay marker");

    let snapshot_before = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load snapshot before marking replayed batches");
    assert_eq!(snapshot_before.legacy_archive_pending, 2);

    let materialize = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize should only mark replayed batches");
    assert_eq!(materialize.materialized_invocation_batches, 1);
    assert_eq!(materialize.materialized_forward_proxy_batches, 0);

    let materialized_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count replayed batches marked materialized");
    assert_eq!(materialized_batches, 2);

    let snapshot_after = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load snapshot after marking replayed batches");
    assert_eq!(snapshot_after.legacy_archive_pending, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_replays_usage_breakdown_when_account_targets_are_pending() {
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

    let account_target_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
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
    .fetch_one(&pool)
    .await
    .expect("count account replay markers");
    assert_eq!(account_target_markers, 4);

    let breakdown_row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_usage_breakdown_hourly")
            .fetch_one(&pool)
            .await
            .expect("count usage breakdown rollup rows after replay");
    assert!(breakdown_row_count > 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_priority_materialization_drains_backlog_without_clearing_blocked_batches()
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
async fn usage_breakdown_repair_skips_quarantined_prefix_and_reaches_verified_archive() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-bounded-prefix").await;
    let verified_archive_path = temp_dir.join("verified-prefix-65.sqlite.gz");
    write_valid_invocation_archive(&verified_archive_path, "verified-prefix").await;
    let verified_archive_path = verified_archive_path.to_string_lossy().to_string();
    let verified_archive_sha =
        sha256_hex_file(Path::new(&verified_archive_path)).expect("hash verified archive");
    for id in 1..=65_i64 {
        let (file_path, sha256) = if id == 65 {
            (verified_archive_path.clone(), verified_archive_sha.clone())
        } else {
            (
                temp_dir
                    .join(format!("archive-{id}.sqlite.gz"))
                    .to_string_lossy()
                    .to_string(),
                format!("sha-{id}"),
            )
        };
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, historical_rollups_materialized_at, created_at
            )
            VALUES (?1, 'codex_invocations', '2026-01', ?2, ?3, 1, 'completed',
                    'unknown', datetime('now'), datetime('now'))
            "#,
        )
        .bind(id)
        .bind(&file_path)
        .bind(&sha256)
        .execute(&pool)
        .await
        .expect("seed materialized archive batch");
        if id <= 64 {
            sqlx::query(
                r#"
                INSERT INTO hourly_rollup_archive_replay (
                    target, dataset, file_path, archive_sha256, replayed_at
                )
                VALUES (?1, 'codex_invocations', ?2, NULL, datetime('now'))
                "#,
            )
            .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
            .bind(&file_path)
            .execute(&pool)
            .await
            .expect("seed quarantined replay marker");
        }
    }

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("repair should skip quarantined prefix");
    assert_eq!(touched, 1);

    let verified_materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE id = 65",
    )
    .fetch_one(&pool)
    .await
    .expect("load verified archive state");
    assert!(verified_materialized_at.is_none());

    let quarantined_materialized_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE id < 65 AND historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count quarantined archive states");
    assert_eq!(quarantined_materialized_count, 64);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_prioritizes_recoverable_candidates_over_stale_prefix() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-stale-prefix").await;
    let verified_archive_path = temp_dir.join("verified-stale-prefix-65.sqlite.gz");
    write_valid_invocation_archive(&verified_archive_path, "verified-stale-prefix").await;
    let verified_archive_path = verified_archive_path.to_string_lossy().to_string();
    let verified_archive_sha = sha256_hex_file(Path::new(&verified_archive_path))
        .expect("hash verified stale-prefix archive");
    for id in 1..=65_i64 {
        let (file_path, sha256) = if id == 65 {
            (verified_archive_path.clone(), verified_archive_sha.clone())
        } else {
            (
                temp_dir
                    .join(format!("stale-prefix-{id}.sqlite.gz"))
                    .to_string_lossy()
                    .to_string(),
                format!("current-sha-{id}"),
            )
        };
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, historical_rollups_materialized_at, created_at
            )
            VALUES (?1, 'codex_invocations', '2026-01', ?2, ?3, 1, 'completed',
                    'unknown', datetime('now'), datetime('now'))
            "#,
        )
        .bind(id)
        .bind(&file_path)
        .bind(&sha256)
        .execute(&pool)
        .await
        .expect("seed stale-prefix materialized archive");
        if id <= 64 {
            sqlx::query(
                r#"
                INSERT INTO hourly_rollup_archive_replay (
                    target, dataset, file_path, archive_sha256, replayed_at
                )
                VALUES (?1, 'codex_invocations', ?2, ?3, datetime('now'))
                "#,
            )
            .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
            .bind(&file_path)
            .bind(format!("stale-sha-{id}"))
            .execute(&pool)
            .await
            .expect("seed stale replay marker");
        }
    }

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("repair should prioritize the recoverable candidate");
    assert_eq!(touched, 1);
    let clean_archive_materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE id = 65",
    )
    .fetch_one(&pool)
    .await
    .expect("load recoverable archive state");
    assert!(clean_archive_materialized_at.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_reaches_recoverable_stale_tail_after_cursor_page() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-stale-tail").await;
    let tail_path = temp_dir.join("stale-tail-65.sqlite.gz");
    write_valid_invocation_archive(&tail_path, "recoverable-stale-tail").await;
    let tail_path = tail_path.to_string_lossy().to_string();
    let tail_sha = sha256_hex_file(Path::new(&tail_path)).expect("hash recoverable tail");

    for id in 1..=65_i64 {
        let file_path = if id == 65 {
            tail_path.clone()
        } else {
            temp_dir
                .join(format!("unrecoverable-stale-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string()
        };
        let sha256 = if id == 65 {
            tail_sha.clone()
        } else {
            format!("current-sha-{id}")
        };
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, historical_rollups_materialized_at,
                coverage_start_at, coverage_end_at, created_at
            )
            VALUES (?1, 'codex_invocations', '2026-01', ?2, ?3, 1, 'completed',
                    'unknown', datetime('now'), ?4, ?5, datetime('now'))
            "#,
        )
        .bind(id)
        .bind(&file_path)
        .bind(&sha256)
        .bind((id == 65).then_some("2026-01-01 00:00:00"))
        .bind((id == 65).then_some("2026-01-01 00:30:00"))
        .execute(&pool)
        .await
        .expect("seed stale-tail materialized archive");
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (
                target, dataset, file_path, archive_sha256, replayed_at
            )
            VALUES (?1, 'codex_invocations', ?2, ?3, datetime('now'))
            "#,
        )
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
        .bind(&file_path)
        .bind(format!("stale-sha-{id}"))
        .execute(&pool)
        .await
        .expect("seed stale-tail replay marker");
    }

    let first_page = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("skip the first stale cursor page");
    assert_eq!(first_page, 0);
    let tail_materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE id = 65",
    )
    .fetch_one(&pool)
    .await
    .expect("load stale tail after first cursor page");
    assert!(tail_materialized_at.is_some());

    let second_page = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("repair the recoverable stale tail");
    assert_eq!(second_page, 1);
    let tail_materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE id = 65",
    )
    .fetch_one(&pool)
    .await
    .expect("load repaired stale tail");
    assert!(tail_materialized_at.is_none());
    let tail_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(&tail_path)
    .fetch_one(&pool)
    .await
    .expect("count repaired stale tail markers");
    assert_eq!(tail_marker_count, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_cursor_survives_file_backed_restart() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("breakdown-repair-file-backed-cursor").await;
    let mut seed_tx = pool
        .begin()
        .await
        .expect("begin file-backed repair cursor seed");
    for id in 1..=65_i64 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, historical_rollups_materialized_at, created_at
            )
            VALUES (?1, 'codex_invocations', '2026-02', ?2, ?3, 1, 'completed',
                    'unknown', datetime('now'), datetime('now'))
            "#,
        )
        .bind(id)
        .bind(
            temp_dir
                .join(format!("file-backed-cursor-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(format!("file-backed-cursor-sha-{id}"))
        .execute(&mut *seed_tx)
        .await
        .expect("seed file-backed repair cursor candidate");
    }
    seed_tx
        .commit()
        .await
        .expect("commit file-backed repair cursor seed");

    let first_attempt =
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
            .await
            .expect("repair first file-backed cursor page");
    assert_eq!(first_attempt, 0);
    let first_cursor_id: i64 = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_archive_repair_progress \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&pool)
    .await
    .expect("load first file-backed repair cursor");
    assert_eq!(first_cursor_id, 64);
    pool.close().await;

    let reopened_pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&test_sqlite_url_for_path(&config.database_path))
        .await
        .expect("reopen file-backed repair cursor database");
    let second_attempt =
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&reopened_pool)
            .await
            .expect("continue file-backed repair cursor after restart");
    assert_eq!(second_attempt, 0);
    let final_cursor: (i64, i64, String, String) = sqlx::query_as(
        "SELECT cursor_id, cursor_stale_rank, cursor_month_key, cursor_created_at \
         FROM hourly_rollup_archive_repair_progress \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&reopened_pool)
    .await
    .expect("load reset file-backed repair cursor");
    assert_eq!(final_cursor.0, 65);
    assert_eq!(final_cursor.1, 0);
    assert_eq!(final_cursor.2, "2026-02");
    assert!(!final_cursor.3.is_empty());
    let deferred_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_repair_deferred \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&reopened_pool)
    .await
    .expect("count rejected file-backed repair candidates");
    assert_eq!(deferred_count, 65);
    reopened_pool.close().await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_rebuilds_more_than_512_live_rows() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-large-live-bucket").await;
    let archive_path = temp_dir
        .join("archives")
        .join("codex_invocations")
        .join("large-live-bucket.sqlite.gz");
    fs::create_dir_all(
        archive_path
            .parent()
            .expect("large live bucket archive has a parent"),
    )
    .expect("create large live bucket archive directory");
    write_valid_invocation_archive(&archive_path, "large-live-bucket").await;
    let archive_path_string = archive_path.to_string_lossy().to_string();
    let archive_sha = sha256_hex_file(&archive_path).expect("hash large live bucket archive");
    let occurred_at = "2026-02-01 08:15:00";
    let payload = r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#;
    let mut seed_tx = pool.begin().await.expect("begin large live bucket seed");
    for index in 0..513_i64 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, model, input_tokens, output_tokens,
                cache_input_tokens, reasoning_tokens, total_tokens, cost, status,
                payload, raw_response
            )
            VALUES (?1, ?2, 'proxy', 'gpt-5', 12, 3, 0, 0, 15, 0.01,
                    'success', ?3, '{}')
            "#,
        )
        .bind(format!("large-live-bucket-{index}"))
        .bind(occurred_at)
        .bind(payload)
        .execute(&mut *seed_tx)
        .await
        .expect("seed large live bucket invocation");
    }
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status,
            summary_source_kind, coverage_start_at, coverage_end_at,
            historical_rollups_materialized_at, created_at
        )
        VALUES ('codex_invocations', '2026-02', ?1, ?2, 1, 'completed',
                'unknown', '2026-02-01 08:00:00', '2026-02-01 08:30:00',
                datetime('now'), datetime('now'))
        "#,
    )
    .bind(&archive_path_string)
    .bind(&archive_sha)
    .execute(&mut *seed_tx)
    .await
    .expect("seed large live bucket archive manifest");
    seed_tx
        .commit()
        .await
        .expect("commit large live bucket seed");

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("repair large live bucket without a row-count cutoff");
    assert_eq!(touched, 1);
    let bucket_epoch = crate::stats::summary_rollup_bucket_start_epoch(occurred_at)
        .expect("calculate large live bucket epoch");
    let request_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) \
         FROM upstream_account_usage_breakdown_hourly \
         WHERE bucket_start_epoch = ?1 AND upstream_account_id = 17",
    )
    .bind(bucket_epoch)
    .fetch_one(&pool)
    .await
    .expect("count large live bucket usage breakdown rows");
    assert_eq!(request_count, 513);
    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(&archive_path_string)
    .fetch_one(&pool)
    .await
    .expect("load large live bucket materialization state");
    assert!(materialized_at.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_reopens_overlap_closure_larger_than_query_page() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-large-overlap-closure").await;
    let base = parse_shanghai_local_naive("2026-08-01 00:00:00").expect("valid overlap base");
    let mut archive_paths = Vec::new();
    for index in 0..=65_i64 {
        let occurred_at = format_naive(base + ChronoDuration::hours(index));
        let batch_name = format!("large-overlap-{index}");
        let archive_path = seed_invocation_archive_batch_with_details(
            &pool,
            &config,
            &batch_name,
            &[SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: &batch_name,
                occurred_at: &occurred_at,
                source: SOURCE_PROXY,
                status: "success",
                total_tokens: index + 1,
                cost: 0.01,
                ttfb_ms: Some(120.0),
                payload: Some(r#"{"upstreamAccountId":17,"responseModel":"gpt-5"}"#),
                detail_level: DETAIL_LEVEL_FULL,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none"),
                is_actionable: Some(0),
            }],
        )
        .await;
        let file_path = archive_path.to_string_lossy().into_owned();
        let coverage_start_at = if index == 0 {
            format_naive(base)
        } else {
            occurred_at.clone()
        };
        let coverage_end_at = if index == 0 {
            format_naive(base + ChronoDuration::hours(66))
        } else {
            format_naive(base + ChronoDuration::hours(index + 1))
        };
        sqlx::query(
            r#"
            UPDATE archive_batches
            SET coverage_start_at = ?1,
                coverage_end_at = ?2,
                historical_rollups_materialized_at = datetime('now')
            WHERE dataset = 'codex_invocations' AND file_path = ?3
            "#,
        )
        .bind(&coverage_start_at)
        .bind(&coverage_end_at)
        .bind(&file_path)
        .execute(&pool)
        .await
        .expect("set large overlap archive coverage");
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (
                target, dataset, file_path, archive_sha256, replayed_at
            )
            VALUES (?1, 'codex_invocations', ?2, 'stale-overlap-sha', datetime('now'))
            "#,
        )
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
        .bind(&file_path)
        .execute(&pool)
        .await
        .expect("seed stale large overlap replay marker");
        let bucket_start_epoch =
            invocation_bucket_start_epoch(&occurred_at).expect("derive large overlap bucket start");
        sqlx::query(
            r#"
            INSERT INTO upstream_account_usage_breakdown_hourly (
                bucket_start_epoch, source, upstream_account_key, normalized_model,
                request_count, success_count, failure_count
            )
            VALUES (?1, ?2, ?3, ?4, 1, 1, 0)
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(SOURCE_PROXY)
        .bind(format!("large-overlap-account-{index}"))
        .bind("gpt-5")
        .execute(&pool)
        .await
        .expect("seed large overlap usage breakdown row");
        archive_paths.push(file_path);
    }

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("repair large overlap closure");
    assert_eq!(touched, archive_paths.len());

    let remaining_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM upstream_account_usage_breakdown_hourly \
         WHERE upstream_account_key LIKE 'large-overlap-account-%'",
    )
    .fetch_one(&pool)
    .await
    .expect("count cleared large overlap usage rows");
    assert_eq!(remaining_rows, 0);
    let remaining_materialized: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches \
         WHERE dataset = 'codex_invocations' \
           AND historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count large overlap materialized batches");
    assert_eq!(remaining_materialized, 0);
    let deferred_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_repair_deferred \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&pool)
    .await
    .expect("count deferred large overlap batches");
    assert_eq!(deferred_count, 0);
    let replay_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay \
         WHERE dataset = 'codex_invocations' \
           AND target = ?1",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("count cleared large overlap replay markers");
    assert_eq!(replay_marker_count, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_deferred_candidate_does_not_starve_recoverable_tail() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-deferred-tail").await;
    let deferred_path = temp_dir.join("deferred-tail-missing.sqlite.gz");
    let recoverable_path = temp_dir.join("deferred-tail-recoverable.sqlite.gz");
    write_valid_invocation_archive(&recoverable_path, "deferred-tail-recoverable").await;
    let recoverable_file_path = recoverable_path.to_string_lossy().into_owned();
    let recoverable_sha =
        sha256_hex_file(&recoverable_path).expect("hash recoverable tail archive");

    for (id, month_key, file_path, sha256) in [
        (
            1_i64,
            "2026-08",
            deferred_path.to_string_lossy().into_owned(),
            "deferred-tail-missing-sha".to_string(),
        ),
        (
            2_i64,
            "2026-09",
            recoverable_file_path.clone(),
            recoverable_sha.clone(),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, coverage_start_at, coverage_end_at,
                historical_rollups_materialized_at
            )
            VALUES (?1, 'codex_invocations', ?2, ?3, ?4, 1, 'completed',
                    'unknown', ?5, ?6, datetime('now'))
            "#,
        )
        .bind(id)
        .bind(month_key)
        .bind(file_path)
        .bind(sha256)
        .bind(format!("{month_key}-01 08:00:00"))
        .bind(format!("{month_key}-01 08:30:00"))
        .execute(&pool)
        .await
        .expect("seed deferred-tail archive batch");
    }
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_repair_deferred (
            scope, archive_id, stale_rank, month_key, created_at, file_path
        )
        VALUES ('invocation_archive_usage_breakdown', 1, 0, '2026-08', datetime('now'), ?1)
        "#,
    )
    .bind(deferred_path.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .expect("seed deferred-tail queue entry");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_repair_deferred (
            scope, archive_id, stale_rank, month_key, created_at, file_path
        )
        VALUES ('invocation_archive_usage_breakdown', 2, 0, '2026-09', datetime('now'), ?1)
        "#,
    )
    .bind(recoverable_file_path)
    .execute(&pool)
    .await
    .expect("seed recoverable deferred-tail queue entry");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_repair_progress (
            scope, cursor_id, cursor_stale_rank, cursor_month_key, cursor_created_at
        )
        VALUES ('invocation_archive_usage_breakdown', 2, 0, '2026-09', datetime('now'))
        "#,
    )
    .execute(&pool)
    .await
    .expect("seed deferred-tail cursor after fresh candidates");

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("repair recoverable tail while deferred candidate exists");
    assert_eq!(touched, 1);
    let recoverable_materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE id = 2",
    )
    .fetch_one(&pool)
    .await
    .expect("load recoverable tail materialization state");
    assert!(recoverable_materialized_at.is_none());
    let deferred_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_repair_deferred \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&pool)
    .await
    .expect("count deferred-tail queue entries");
    assert_eq!(deferred_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_discards_deferred_live_mirror_candidate() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-live-mirror-queue").await;
    let file_path = temp_dir.join("live-mirror-queued.sqlite.gz");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id, dataset, month_key, file_path, sha256, row_count, status,
            summary_source_kind, coverage_start_at, coverage_end_at,
            historical_rollups_materialized_at
        )
        VALUES (1, 'codex_invocations', '2026-08', ?1, 'live-mirror-sha', 1, 'completed',
                'live_mirror', '2026-08-01 08:00:00', '2026-08-01 08:30:00', datetime('now'))
        "#,
    )
    .bind(file_path.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .expect("seed live-mirror archive batch");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_repair_deferred (
            scope, archive_id, stale_rank, month_key, created_at, file_path
        )
        VALUES ('invocation_archive_usage_breakdown', 1, 0, '2026-08', datetime('now'), ?1)
        "#,
    )
    .bind(file_path.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .expect("seed live-mirror deferred queue entry");

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("discard deferred live-mirror candidate");
    assert_eq!(touched, 0);
    let deferred_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_repair_deferred \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&pool)
    .await
    .expect("count discarded live-mirror queue entries");
    assert_eq!(deferred_count, 0);
    assert!(
        !usage_breakdown_repair_is_pending(&pool)
            .await
            .expect("inspect cleared live-mirror repair state")
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_commits_selected_candidate_after_elapsed_budget() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-candidate-budget").await;
    let first_path = temp_dir.join("budgeted-candidate-first.sqlite.gz");
    let second_path = temp_dir.join("budgeted-candidate-second.sqlite.gz");
    write_valid_invocation_archive(&first_path, "budgeted-candidate-first").await;
    write_valid_invocation_archive(&second_path, "budgeted-candidate-second").await;

    for (id, path, month_key) in [
        (1_i64, &first_path, "2026-08"),
        (2_i64, &second_path, "2026-09"),
    ] {
        let path = path.to_string_lossy().into_owned();
        let sha256 = sha256_hex_file(Path::new(&path)).expect("hash budgeted candidate archive");
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, coverage_start_at, coverage_end_at,
                historical_rollups_materialized_at
            )
            VALUES (?1, 'codex_invocations', ?2, ?3, ?4, 1, 'completed',
                    'unknown', ?5, ?6, datetime('now'))
            "#,
        )
        .bind(id)
        .bind(month_key)
        .bind(path)
        .bind(sha256)
        .bind(format!("{month_key}-01 08:00:00"))
        .bind(format!("{month_key}-01 08:30:00"))
        .execute(&pool)
        .await
        .expect("seed budgeted candidate archive");
    }

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state_with_elapsed_budget_for_test(
        &pool,
        Duration::ZERO,
    )
    .await
    .expect("finish the selected candidate after the elapsed budget");
    assert_eq!(touched, 1);
    let materialized_states: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("load budgeted candidate materialization states");
    assert!(materialized_states[0].is_none());
    assert!(materialized_states[1].is_some());
    let cursor_id: i64 = sqlx::query_scalar(
        "SELECT cursor_id FROM hourly_rollup_archive_repair_progress \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&pool)
    .await
    .expect("load committed budgeted repair cursor");
    assert_eq!(cursor_id, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn replay_invocation_archives_bounded_selection_reaches_after_blocked_prefix() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("historical-rollup-bounded-selection").await;
    for id in 1..=65_i64 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind
            )
            VALUES (?1, 'codex_invocations', '2026-01', ?2, ?3, 1, 'completed', 'unknown')
            "#,
        )
        .bind(id)
        .bind(
            temp_dir
                .join(format!("blocked-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(format!("sha-{id}"))
        .execute(&pool)
        .await
        .expect("seed bounded replay candidate");
    }

    let mut tx = pool
        .begin()
        .await
        .expect("begin bounded replay transaction");
    let first = replay_invocation_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now(),
        Some(64),
        None,
        0,
    )
    .await
    .expect("scan the first bounded replay candidate page");
    assert_eq!(first.scanned_batches, 64);
    assert_eq!(first.blocked_batches, 64);

    let second = replay_invocation_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now(),
        Some(64),
        None,
        first.scanned_batches as usize,
    )
    .await
    .expect("scan beyond the first bounded replay candidate page");
    assert_eq!(second.scanned_batches, 65);
    assert_eq!(second.skipped_batches, 64);
    assert_eq!(second.blocked_batches, 1);
    tx.rollback()
        .await
        .expect("rollback bounded replay transaction");

    cleanup_temp_test_dir(&temp_dir);
}
