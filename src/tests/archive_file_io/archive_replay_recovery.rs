use super::archive_backfill_test_support::write_valid_invocation_archive;
use super::*;
use serde_json::json;
#[tokio::test]
async fn replay_usage_breakdown_archives_bounded_selection_reaches_after_blocked_prefix() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("usage-breakdown-bounded-selection").await;
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
                .join(format!("usage-breakdown-blocked-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(format!("usage-breakdown-sha-{id}"))
        .execute(&pool)
        .await
        .expect("seed usage breakdown bounded replay candidate");
    }

    let mut tx = pool
        .begin()
        .await
        .expect("begin usage breakdown bounded replay transaction");
    let first = replay_invocation_usage_breakdown_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now(),
        Some(64),
        None,
        0,
    )
    .await
    .expect("scan the first usage breakdown candidate page");
    assert_eq!(first.scanned_batches, 64);
    assert_eq!(first.blocked_batches, 64);

    let second = replay_invocation_usage_breakdown_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now(),
        Some(64),
        None,
        first.scanned_batches as usize,
    )
    .await
    .expect("scan beyond the first usage breakdown candidate page");
    assert_eq!(second.scanned_batches, 65);
    assert_eq!(second.skipped_batches, 64);
    assert_eq!(second.blocked_batches, 1);
    tx.rollback()
        .await
        .expect("rollback usage breakdown bounded replay transaction");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn replay_forward_proxy_archives_bounded_selection_reaches_after_blocked_prefix() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("forward-proxy-bounded-selection").await;
    for id in 1..=65_i64 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status
            )
            VALUES (?1, 'forward_proxy_attempts', '2026-01', ?2, ?3, 1, 'completed')
            "#,
        )
        .bind(id)
        .bind(
            temp_dir
                .join(format!("forward-proxy-blocked-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(format!("forward-proxy-sha-{id}"))
        .execute(&pool)
        .await
        .expect("seed forward proxy bounded replay candidate");
    }

    let mut tx = pool
        .begin()
        .await
        .expect("begin forward proxy bounded replay transaction");
    let first = replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now(),
        Some(64),
        None,
        0,
    )
    .await
    .expect("scan the first forward proxy candidate page");
    assert_eq!(first.scanned_batches, 64);
    assert_eq!(first.blocked_batches, 64);

    let second = replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now(),
        Some(64),
        None,
        first.scanned_batches as usize,
    )
    .await
    .expect("scan beyond the first forward proxy candidate page");
    assert_eq!(second.scanned_batches, 65);
    assert_eq!(second.skipped_batches, 64);
    assert_eq!(second.blocked_batches, 1);
    tx.rollback()
        .await
        .expect("rollback forward proxy bounded replay transaction");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_materialization_advances_past_missing_parent_prefix() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("usage-breakdown-missing-parent-prefix").await;
    for id in 1..=64_i64 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind
            )
            VALUES (?1, 'codex_invocations', '2025-01', ?2, ?3, 1, 'completed', 'unknown')
            "#,
        )
        .bind(id)
        .bind(
            temp_dir
                .join(format!("missing-parent-{id}"))
                .join("archive.sqlite.gz")
                .to_string_lossy()
                .to_string(),
        )
        .bind(format!("missing-parent-sha-{id}"))
        .execute(&pool)
        .await
        .expect("seed missing-parent usage breakdown candidate");
    }
    let occurred_at = shanghai_local_days_ago(120, 9, 0, 0);
    let valid_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "usage-breakdown-after-missing-parent-prefix",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "usage-breakdown-after-missing-parent-prefix",
            occurred_at: &occurred_at,
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 42,
            cost: 0.42,
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

    let first = materialize_usage_breakdown_historical_rollups_bounded_from_skip(
        &pool,
        &config,
        Some(64),
        Some(Duration::from_secs(6)),
        0,
    )
    .await
    .expect("process the missing-parent candidate page");
    assert_eq!(first.scanned_archive_batches, 64);
    assert_eq!(first.blocked_archive_batches, 64);

    let second = materialize_usage_breakdown_historical_rollups_bounded_from_skip(
        &pool,
        &config,
        Some(64),
        Some(Duration::from_secs(6)),
        first.scanned_archive_batches,
    )
    .await
    .expect("advance to the valid archive after the missing-parent prefix");
    assert_eq!(second.scanned_archive_batches, 65);
    assert_eq!(second.skipped_archive_batches, 64);
    assert_eq!(second.materialized_invocation_batches, 1);
    let usage_breakdown_marker_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND target = ?2
          AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(valid_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("count usage breakdown marker for the valid archive");
    assert_eq!(usage_breakdown_marker_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn historical_rollup_materialization_advances_past_missing_parent_prefix() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("historical-rollup-missing-parent-prefix").await;
    for id in 1..=64_i64 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind
            )
            VALUES (?1, 'codex_invocations', '2025-01', ?2, ?3, 1, 'completed', 'unknown')
            "#,
        )
        .bind(id)
        .bind(
            temp_dir
                .join(format!("historical-rollup-missing-parent-{id}"))
                .join("archive.sqlite.gz")
                .to_string_lossy()
                .to_string(),
        )
        .bind(format!("historical-rollup-missing-parent-sha-{id}"))
        .execute(&pool)
        .await
        .expect("seed historical rollup missing-parent candidate");
    }
    let occurred_at = shanghai_local_days_ago(120, 9, 0, 0);
    let valid_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "historical-rollup-after-missing-parent-prefix",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "historical-rollup-after-missing-parent-prefix",
            occurred_at: &occurred_at,
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 42,
            cost: 0.42,
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

    let first = materialize_historical_rollups_bounded_from_skip(
        &pool,
        &config,
        false,
        Some(64),
        Some(Duration::from_secs(6)),
        0,
    )
    .await
    .expect("process the generic missing-parent candidate page");
    assert_eq!(first.scanned_archive_batches, 64);
    assert_eq!(first.blocked_archive_batches, 64);

    let second = materialize_historical_rollups_bounded_from_skip(
        &pool,
        &config,
        false,
        Some(64),
        Some(Duration::from_secs(6)),
        first.scanned_archive_batches,
    )
    .await
    .expect("advance generic rollup to valid archive after missing-parent prefix");
    assert_eq!(second.scanned_archive_batches, 65);
    assert_eq!(second.skipped_archive_batches, 64);
    assert_eq!(second.materialized_invocation_batches, 1);
    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(valid_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("load materialized timestamp for the valid archive");
    assert!(materialized_at.is_some());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn upstream_account_archive_marker_repair_converges_across_multiple_pages() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("upstream-account-marker-multi-page").await;
    for id in 1..=65_i64 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, historical_rollups_materialized_at
            )
            VALUES (?1, 'codex_invocations', '2025-01', ?2, ?3, 1, 'completed',
                    'unknown', datetime('now'))
            "#,
        )
        .bind(id)
        .bind(
            temp_dir
                .join(format!("materialized-marker-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(format!("materialized-marker-sha-{id}"))
        .execute(&pool)
        .await
        .expect("seed materialized marker repair candidate");
        if id == 1 {
            sqlx::query(
                "INSERT INTO hourly_rollup_archive_replay \
                 (target, dataset, file_path, archive_sha256) \
                 VALUES (?1, 'codex_invocations', ?2, 'stale-marker-sha')",
            )
            .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
            .bind(
                temp_dir
                    .join("materialized-marker-1.sqlite.gz")
                    .to_string_lossy()
                    .to_string(),
            )
            .execute(&pool)
            .await
            .expect("seed stale upstream account archive marker");
        }
    }

    sqlx::query(
        r#"
        CREATE TRIGGER fail_upstream_account_marker_second_page
        BEFORE INSERT ON hourly_rollup_archive_replay
        WHEN NEW.file_path LIKE '%materialized-marker-65.sqlite.gz'
        BEGIN
            SELECT RAISE(ABORT, 'injected marker page failure');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("install second-page marker failure");

    let first_attempt = repair_materialized_upstream_account_archive_markers(&pool).await;
    assert!(first_attempt.is_err(), "second marker page should fail");
    for target in [
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
    ] {
        let marker_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1",
        )
        .bind(target)
        .fetch_one(&pool)
        .await
        .expect("count committed first-page upstream account markers");
        assert_eq!(
            marker_count, 64,
            "first marker page should commit for {target}"
        );
    }

    sqlx::query("DROP TRIGGER fail_upstream_account_marker_second_page")
        .execute(&pool)
        .await
        .expect("remove second-page marker failure");
    let repaired = repair_materialized_upstream_account_archive_markers(&pool)
        .await
        .expect("repair remaining materialized upstream account archive marker");
    assert_eq!(repaired, 1);

    for target in [
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
    ] {
        let marker_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1",
        )
        .bind(target)
        .fetch_one(&pool)
        .await
        .expect("count repaired upstream account archive markers");
        assert_eq!(
            marker_count, 65,
            "all candidates should be repaired for {target}"
        );
    }
    let repaired_sha: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay \
         WHERE target = ?1 AND dataset = 'codex_invocations' \
           AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(
        temp_dir
            .join("materialized-marker-1.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .fetch_one(&pool)
    .await
    .expect("load repaired stale upstream account archive marker");
    assert_eq!(
        repaired_sha.as_deref(),
        Some("stale-marker-sha"),
        "marker repair must preserve an existing stale marker for full rebuild repair"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_recovery_backlog_observation_refreshes_after_new_rows_and_cutoff_changes() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-recovery-backlog-cache").await;
    let initial_days = config.invocation_max_days;
    let first_occurred_at = shanghai_local_days_ago((initial_days + 10) as i64, 9, 0, 0);
    let second_occurred_at =
        shanghai_local_days_ago(initial_days.saturating_sub(1) as i64, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "retention-recovery-backlog-cache-first",
        &first_occurred_at,
        SOURCE_PROXY,
        "success",
        None,
        "{}",
        None,
        None,
        Some(1),
        Some(0.01),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "retention-recovery-backlog-cache-second",
        &second_occurred_at,
        SOURCE_PROXY,
        "success",
        None,
        "{}",
        None,
        None,
        Some(2),
        Some(0.02),
    )
    .await;

    let first = retention_test_refresh_recovery_counts(&pool, &config)
        .await
        .expect("load initial recovery backlog");
    assert_eq!(first.expired_backlog_count, Some(1));

    insert_retention_invocation(
        &pool,
        "retention-recovery-backlog-cache-third",
        &first_occurred_at,
        SOURCE_PROXY,
        "success",
        None,
        "{}",
        None,
        None,
        Some(3),
        Some(0.03),
    )
    .await;
    let refreshed_after_insert = retention_test_refresh_recovery_counts(&pool, &config)
        .await
        .expect("refresh recovery backlog after new row");
    assert_eq!(
        refreshed_after_insert.expired_backlog_count,
        Some(2),
        "a successful observation must include rows committed after the prior observation"
    );

    config.invocation_max_days = initial_days.saturating_sub(2);
    let refreshed = retention_test_refresh_recovery_counts(&pool, &config)
        .await
        .expect("refresh recovery backlog after cutoff change");
    assert_eq!(refreshed.expired_backlog_count, Some(3));

    let (second_pool, second_config, second_temp_dir) =
        retention_test_pool_and_config("retention-recovery-backlog-cache-isolation").await;
    let isolated = retention_test_refresh_recovery_counts(&second_pool, &second_config)
        .await
        .expect("load recovery backlog for a different database");
    assert_eq!(
        isolated.expired_backlog_count,
        Some(0),
        "a cache entry from another database must not be reused"
    );

    pool.close().await;
    second_pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
    cleanup_temp_test_dir(&second_temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_reopens_materialized_batches_missing_usage_breakdown_backfill() {
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

    let breakdown_replay_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target = ?2
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("count remaining breakdown replay markers");
    assert_eq!(breakdown_replay_markers, 0);

    let materialized_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT historical_rollups_materialized_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
        "#,
    )
    .bind(&invocation_archive_path)
    .fetch_one(&pool)
    .await
    .expect("load materialized timestamp after bootstrap");
    assert!(materialized_at.is_none());

    let backlog_snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load backlog after bootstrap repair");
    assert_eq!(backlog_snapshot.legacy_archive_pending, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_repairs_pre_upgrade_breakdown_rows_before_syncing_new_live_rows()
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

    let breakdown = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT request_count, success_count, performance_total_tokens
        FROM upstream_account_usage_breakdown_hourly
        WHERE upstream_account_id = ?1
          AND normalized_model = 'gpt-5'
        "#,
    )
    .bind(17_i64)
    .fetch_one(&pool)
    .await
    .expect("load repaired breakdown rollup row");
    assert_eq!(breakdown.0, 2);
    assert_eq!(breakdown.1, 2);
    assert_eq!(breakdown.2, 70);

    let shared_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .fetch_one(&pool)
            .await
            .expect("load shared invocation cursor after bootstrap");
    assert_eq!(shared_cursor, 42);

    let repair_cursor: i64 =
        sqlx::query_scalar("SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1")
            .bind(INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET)
            .fetch_one(&pool)
            .await
            .expect("load breakdown repair cursor after bootstrap");
    assert_eq!(repair_cursor, 41);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn bootstrap_hourly_rollups_repairs_legacy_account_replay_markers_when_breakdown_is_healthy()
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

    sqlx::query(
        r#"
        INSERT INTO upstream_account_usage_breakdown_hourly (
            bucket_start_epoch,
            source,
            upstream_account_key,
            upstream_account_id,
            normalized_model,
            normalized_reasoning_effort,
            request_count,
            success_count,
            failure_count,
            cache_write_tokens,
            cache_read_tokens,
            output_tokens,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            cost_unknown,
            has_cost,
            performance_total_tokens,
            performance_stream_output_tokens,
            performance_stream_duration_ms,
            performance_response_sample_count,
            performance_response_sum_ms,
            performance_first_byte_sample_count,
            performance_first_byte_sum_ms,
            performance_usage_duration_sample_count,
            performance_usage_duration_sum_ms,
            updated_at
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, '', 1, 1, 0, 0, 0, 42,
            0.42, 0.0, 0.0, 0.0, 0.0, 0.0, 1, 42, 42, 120.0, 1, 120.0, 1, 60.0, 1, 120.0,
            datetime('now')
        )
        "#,
    )
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .bind("upstream:17")
    .bind(17_i64)
    .bind("gpt-5")
    .execute(&pool)
    .await
    .expect("seed healthy breakdown rollup row");

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'codex_invocations'
          AND file_path = ?1
          AND target IN (?2, ?3, ?4)
        "#,
    )
    .bind(&invocation_archive_path)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .execute(&pool)
    .await
    .expect("drop legacy account replay markers");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should repair legacy account replay markers");

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
    .expect("count repaired legacy account replay markers");
    assert_eq!(account_target_markers, 4);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_reopens_a_replaced_archive_with_a_stale_replay_sha() {
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
    write_valid_invocation_archive(&archive_file, "initial-archive").await;
    let initial_sha256 = sha256_hex_file(&archive_file).expect("hash initial archive fixture");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
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
        VALUES (
            ?1,
            '2026-01',
            ?2,
            ?3,
            1,
            ?4,
            '2026-01-15 08:00:00',
            '2026-01-15 08:30:00',
            datetime('now'),
            datetime('now')
        )
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .bind(&initial_sha256)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("seed materialized archive manifest at SHA A");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            archive_sha256,
            replayed_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .bind(&initial_sha256)
    .execute(&pool)
    .await
    .expect("seed matching usage breakdown replay marker");

    let untouched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("matching marker must preserve materialized archive state");
    assert_eq!(untouched, 0);

    let protected_bucket_epoch =
        crate::stats::summary_rollup_bucket_start_epoch("2026-01-15 08:00:00")
            .expect("calculate protected archive bucket");
    sqlx::query(
        "INSERT INTO upstream_account_usage_breakdown_hourly \
         (bucket_start_epoch, source, upstream_account_key, normalized_model, request_count) \
         VALUES (?1, 'proxy', '17', 'gpt-5', 1)",
    )
    .bind(protected_bucket_epoch)
    .execute(&pool)
    .await
    .expect("seed last-good usage breakdown row");

    fs::write(&archive_file, b"corrupt archive bytes").expect("corrupt archive fixture");
    let mismatched_manifest_sha256 = sha256_hex_file(&archive_file)
        .expect("hash corrupt archive fixture")
        .chars()
        .enumerate()
        .map(|(index, character)| {
            if index == 0 {
                if character == '0' { '1' } else { '0' }
            } else {
                character
            }
        })
        .collect::<String>();
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = ?1
        WHERE dataset = ?2
          AND file_path = ?3
        "#,
    )
    .bind(&mismatched_manifest_sha256)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("publish corrupt archive manifest SHA");

    let blocked = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("unreadable replacement must be rejected");
    assert_eq!(blocked, 0);
    let deferred_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_repair_deferred \
         WHERE scope = 'invocation_archive_usage_breakdown'",
    )
    .fetch_one(&pool)
    .await
    .expect("count rejected archive deferred entries");
    assert_eq!(deferred_count, 1);
    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE file_path = ?1",
    )
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load materialized timestamp for unreadable archive");
    assert!(materialized_at.is_some());
    let replay_sha256: Option<String> = sqlx::query_scalar(
        "SELECT archive_sha256 FROM hourly_rollup_archive_replay \
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load last-good replay marker");
    assert_eq!(replay_sha256.as_deref(), Some(initial_sha256.as_str()));
    let protected_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM upstream_account_usage_breakdown_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(protected_bucket_epoch)
    .fetch_one(&pool)
    .await
    .expect("count last-good usage breakdown rows");
    assert_eq!(protected_rows, 1);

    write_valid_invocation_archive(&archive_file, "replacement-archive").await;
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
async fn bootstrap_repairs_null_legacy_account_replay_markers_to_manifest_sha() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("bootstrap-repairs-null-account-markers").await;
    let archive_path = temp_dir
        .join("archives")
        .join("codex_invocations")
        .join("bootstrap-repairs-null-account-markers.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let archive_file = PathBuf::from(&archive_path);
    fs::create_dir_all(
        archive_file
            .parent()
            .expect("archive fixture has a parent directory"),
    )
    .expect("create archive fixture directory");
    write_valid_invocation_archive(&archive_file, "legacy-null-marker").await;
    let manifest_sha = sha256_hex_file(&archive_file).expect("hash archive fixture");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
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
        VALUES (
            ?1,
            '2026-01',
            ?2,
            ?3,
            1,
            ?4,
            '2026-01-15 08:00:00',
            '2026-01-15 08:30:00',
            datetime('now'),
            datetime('now')
        )
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .bind(&manifest_sha)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("seed completed archive manifest");

    for target in [
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (
                target, dataset, file_path, archive_sha256, replayed_at
            )
            VALUES (?1, ?2, ?3, NULL, datetime('now'))
            "#,
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&archive_path)
        .execute(&pool)
        .await
        .expect("seed legacy null-SHA replay marker");
    }

    let mut tx = pool.begin().await.expect("begin marker repair transaction");
    assert!(
        crate::maintenance::mark_materialized_upstream_account_archive_replayed_tx(
            tx.as_mut(),
            &archive_path,
        )
        .await
        .expect("repair legacy null-SHA replay markers")
    );
    tx.commit().await.expect("commit marker repair transaction");

    let repaired_markers: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND file_path = ?2
          AND archive_sha256 = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .bind(&manifest_sha)
    .fetch_one(&pool)
    .await
    .expect("count repaired replay markers");
    assert_eq!(repaired_markers, 3);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn usage_breakdown_repair_rebuilds_transitive_boundary_overlap_closure() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("breakdown-transitive-boundary-closure").await;
    let archive_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 45) as i64))
    .and_hms_opt(7, 0, 0)
    .expect("valid archived hour");
    let transitive_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("valid transitive overlap timestamp"),
    );
    let boundary_start_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(1))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(30)))
            .expect("valid boundary archive start timestamp"),
    );
    let boundary_end_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(2))
            .expect("valid boundary archive end timestamp"),
    );
    let stale_occurred_at = format_naive(
        archive_hour_local
            .checked_add_signed(ChronoDuration::hours(2))
            .and_then(|value| value.checked_add_signed(ChronoDuration::minutes(10)))
            .expect("valid stale archive timestamp"),
    );

    let stale_archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "breakdown-closure-stale-root",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "breakdown-closure-stale-root",
            occurred_at: stale_occurred_at.as_str(),
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
        }],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "breakdown-closure-boundary-peer",
        &[
            SeedInvocationArchiveBatchRow {
                id: 1,
                invoke_id: "breakdown-closure-boundary-peer-start",
                occurred_at: boundary_start_at.as_str(),
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
            },
            SeedInvocationArchiveBatchRow {
                id: 2,
                invoke_id: "breakdown-closure-boundary-peer-end",
                occurred_at: boundary_end_at.as_str(),
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
            },
        ],
    )
    .await;
    seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "breakdown-closure-transitive-peer",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "breakdown-closure-transitive-peer",
            occurred_at: transitive_occurred_at.as_str(),
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
        }],
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
async fn startup_repair_quarantines_unverified_legacy_archive_without_replaying_rollups() {
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
async fn legacy_replay_markers_do_not_upgrade_from_sha_a_to_sha_b() {
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

    sqlx::query(
        r#"
        UPDATE hourly_rollup_archive_replay
        SET archive_sha256 = NULL
        WHERE dataset = ?1 AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("seed legacy null replay markers");
    sqlx::query(
        r#"
        UPDATE hourly_rollup_archive_replay
        SET archive_sha256 = ''
        WHERE target = ?1 AND dataset = ?2 AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("seed legacy blank replay marker");
    sqlx::query(
        "UPDATE archive_batches SET sha256 = 'replacement-archive-sha-b' WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("publish replacement manifest SHA B");

    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        let replay_sha256: Option<String> = sqlx::query_scalar(
            "SELECT archive_sha256 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
        )
        .bind(target)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&archive_file_path)
        .fetch_one(&pool)
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
        repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
            .await
            .expect("legacy marker quarantine must not reopen the archive"),
        0
    );
    let mut tx = pool.begin().await.expect("begin legacy proof check");
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file_path,
        )
        .await
        .expect("legacy NULL marker must fail closed"),
    );
    assert!(
        !hourly_rollup_archive_replayed_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_file_path,
        )
        .await
        .expect("legacy blank marker must fail closed"),
    );
    tx.commit().await.expect("commit legacy proof check");
    let after = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load totals after legacy marker upgrade");
    assert_eq!(after, before);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn stale_replay_sha_rebuilds_all_rollup_targets_without_double_counting() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("stale-replay-sha-full-rebuild").await;
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
    let archive_file_path = archive_path.to_string_lossy().to_string();

    let initial = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize original archive");
    assert_eq!(initial.materialized_invocation_batches, 1);
    let before = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(sample_count), 0) FROM proxy_perf_stage_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_rollup_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_upstream_account_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(activity_v2_request_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_minute WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_sticky_key_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("capture original global and account totals");
    assert_eq!(before, (1, 1, 1, 1, 1, 1, 1, 1, 1, 1));

    let replacement_db_path = temp_dir.join("stale-replay-sha-full-rebuild.sqlite");
    inflate_gzip_sqlite_file(&archive_path, &replacement_db_path)
        .expect("inflate replacement invocation archive");
    let replacement_pool = SqlitePool::connect(&test_sqlite_url_for_path(&replacement_db_path))
        .await
        .expect("open replacement invocation archive sqlite");
    sqlx::query("PRAGMA user_version = 1")
        .execute(&replacement_pool)
        .await
        .expect("rewrite replacement invocation archive metadata");
    replacement_pool.close().await;
    deflate_sqlite_file_to_gzip(&replacement_db_path, &archive_path)
        .expect("compress replacement invocation archive");
    let replacement_sha =
        sha256_hex_file(&archive_path).expect("hash replacement invocation archive");
    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE dataset = ?2 AND file_path = ?3")
        .bind(&replacement_sha)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(&archive_file_path)
        .execute(&pool)
        .await
        .expect("publish replacement archive manifest SHA");
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
        UPDATE hourly_rollup_archive_replay
        SET archive_sha256 = ?1
        WHERE target = ?2 AND dataset = ?3 AND file_path = ?4
        "#,
    )
    .bind(&replacement_sha)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("publish current usage breakdown marker while retaining a stale target marker");

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

    let rebuilt = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize the replacement archive after clearing old contributions");
    assert_eq!(rebuilt.materialized_invocation_batches, 1);
    let after = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(sample_count), 0) FROM proxy_perf_stage_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_rollup_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM prompt_cache_upstream_account_hourly),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(activity_v2_request_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_minute WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_sticky_key_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load rebuilt global and account totals");
    assert_eq!(after, before);

    let replacement_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = ?1 AND file_path = ?2 AND archive_sha256 = ?3",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .bind(&replacement_sha)
    .fetch_one(&pool)
    .await
    .expect("count replacement replay markers");
    assert_eq!(
        replacement_marker_count,
        INVOCATION_HOURLY_ROLLUP_TARGETS.len() as i64
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn startup_summary_proof_recovery_reopens_materialized_archive_and_replays_exactly() {
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

    let recovered = materialize_historical_rollups_startup_window(&pool, 0, Duration::from_secs(6))
        .await
        .expect("startup recovery must reopen and replay the verified archive");
    assert_eq!(recovered.summary.materialized_invocation_batches, 1);
    assert_eq!(recovered.changed_path_count, 1);
    let after = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            (SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly),
            (SELECT COALESCE(SUM(total_count), 0) FROM upstream_account_stats_hourly WHERE upstream_account_id = 17),
            (SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly WHERE upstream_account_id = 17)
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load Summary totals after startup recovery replay");
    assert_eq!(after, before);

    let summary_proof_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND file_path = ?2
          AND target IN (?3, ?4, ?5)
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(&pool)
    .await
    .expect("count complete Summary proofs after automatic replay");
    assert_eq!(
        summary_proof_count,
        SUMMARY_PROJECTION_ARCHIVE_REPLAY_TARGETS.len() as i64
    );

    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = 'authoritative' \
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("promote a fully proved archive to authoritative");
    let error = sqlx::query(
        "UPDATE archive_batches SET sha256 = 'replacement-sha-without-proof' \
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect_err("changing an authoritative archive identity must require new proofs");
    assert!(
        error
            .to_string()
            .contains("completed codex_invocations archive requires Summary publication proof")
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn authoritative_invocation_archive_sha_update_requires_current_summary_proof() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("authoritative-archive-sha-guard").await;
    let archive_path = seed_invocation_archive_batch_with_details(
        &pool,
        &config,
        "authoritative-archive-sha-guard",
        &[SeedInvocationArchiveBatchRow {
            id: 1,
            invoke_id: "authoritative-archive-sha-guard",
            occurred_at: "2025-01-15 08:10:00",
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some(
                r#"{"upstreamAccountId":17,"responseModel":"gpt-5","promptCacheKey":"authoritative-archive-sha-guard"}"#,
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

    let materialized = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialize archive before SHA guard check");
    assert_eq!(materialized.materialized_invocation_batches, 1);

    sqlx::query(
        "UPDATE archive_batches SET summary_source_kind = 'authoritative' \
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("promote a fully proved archive to authoritative");

    let error = sqlx::query(
        "UPDATE archive_batches SET sha256 = 'replacement-sha-without-proof' \
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect_err("changing an authoritative archive identity must require new proofs");
    assert!(
        error
            .to_string()
            .contains("completed codex_invocations archive requires Summary publication proof")
    );

    let persisted_sha: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load unchanged authoritative archive identity");
    assert_ne!(persisted_sha, "replacement-sha-without-proof");

    cleanup_temp_test_dir(&temp_dir);
}
