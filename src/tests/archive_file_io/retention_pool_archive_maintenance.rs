use super::*;
#[tokio::test]
async fn historical_rollup_backfill_stays_critical_until_legacy_invocations_materialized() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("historical-rollup-backfill-critical").await;
    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_max_days + 2) as i64))
    .and_hms_opt(8, 0, 0)
    .expect("valid archived local hour");
    let archived_occurred_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(10))
            .expect("valid archived occurred_at"),
    );
    seed_invocation_archive_batch(
        &pool,
        &config,
        "historical-rollup-backfill-critical",
        &[(
            1_i64,
            "historical-rollup-backfill-critical",
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
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(align_bucket_epoch(Utc::now().timestamp(), 3_600, 0))
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("seed unrelated materialized bucket marker");

    let snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load historical rollup backlog snapshot");
    assert_eq!(snapshot.legacy_archive_pending, 1);
    assert!(snapshot.last_materialized_hour.is_none());
    assert_eq!(
        snapshot.alert_level,
        HistoricalRollupBackfillAlertLevel::Critical
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn prune_legacy_archive_batches_keeps_detail_prune_backups_within_live_window() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-prune-detail-backup").await;
    let pruned_occurred_at = shanghai_local_days_ago(
        (config.invocation_success_full_days + 5)
            .try_into()
            .expect("detail prune age should fit in i64"),
        14,
        0,
        0,
    );
    insert_retention_invocation(
        &pool,
        "historical-rollup-prune-detail-backup",
        &pruned_occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"promptCacheKey\":\"detail-backup\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(11),
        Some(0.11),
    )
    .await;

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention detail prune");

    let (archive_path, materialized_at, detail_level): (String, Option<String>, String) =
        sqlx::query_as(
            r#"
            SELECT b.file_path, b.historical_rollups_materialized_at, i.detail_level
            FROM archive_batches AS b
            JOIN codex_invocations AS i
              ON i.invoke_id = ?1
            WHERE b.dataset = 'codex_invocations'
            ORDER BY b.id DESC
            LIMIT 1
            "#,
        )
        .bind("historical-rollup-prune-detail-backup")
        .fetch_one(&pool)
        .await
        .expect("load detail prune archive metadata");
    assert_eq!(detail_level, DETAIL_LEVEL_STRUCTURED_ONLY);
    assert!(
        materialized_at.is_some(),
        "detail-prune archive still participates in rollup materialization"
    );
    assert!(
        Path::new(&archive_path).exists(),
        "detail backup archive should exist"
    );

    let prune_dry_run = prune_archive_batches(&pool, &config, true)
        .await
        .expect("dry-run prune should retain detail backup archive");
    assert_eq!(prune_dry_run.expired_archive_batches_deleted, 0);
    assert_eq!(prune_dry_run.legacy_archive_batches_deleted, 0);
    assert!(
        Path::new(&archive_path).exists(),
        "dry-run should not remove archive"
    );

    let prune_summary = prune_archive_batches(&pool, &config, false)
        .await
        .expect("prune should keep detail backup archive");
    assert_eq!(prune_summary.expired_archive_batches_deleted, 0);
    assert_eq!(prune_summary.legacy_archive_batches_deleted, 0);
    assert!(
        Path::new(&archive_path).exists(),
        "detail backup archive must remain"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_live_mirror_does_not_require_summary_proof() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-prune-expired-detail-mirror").await;
    config.invocation_archive_ttl_days = 365;
    let occurred_at = shanghai_local_days_ago(31, 14, 30, 0);
    insert_retention_invocation(
        &pool,
        "expired-detail-mirror",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"promptCacheKey\":\"expired-detail-mirror\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(11),
        Some(0.11),
    )
    .await;

    run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention detail prune");
    let (archive_id, archive_path): (i64, String) = sqlx::query_as(
        "SELECT id, file_path FROM archive_batches WHERE dataset = 'codex_invocations' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load live mirror manifest");
    let source_kind: String =
        sqlx::query_scalar("SELECT summary_source_kind FROM archive_batches WHERE id = ?1")
            .bind(archive_id)
            .fetch_one(&pool)
            .await
            .expect("load live mirror source role");
    assert_eq!(source_kind, SUMMARY_ARCHIVE_SOURCE_KIND_LIVE_MIRROR);
    assert!(crate::maintenance::retention_archive_path_is_within_root(
        &config,
        Path::new(&archive_path)
    ));
    assert!(crate::maintenance::retention_archive_path_is_owned(
        &config,
        Path::new(&archive_path)
    ));
    assert!(Path::new(&archive_path).exists());
    sqlx::query(
        "UPDATE archive_batches SET archive_expires_at = '2000-01-01 00:00:00' WHERE id = ?1",
    )
    .bind(archive_id)
    .execute(&pool)
    .await
    .expect("expire live mirror manifest");

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup expired live mirror");
    assert_eq!(deleted, 1);
    assert!(!Path::new(&archive_path).exists());
    let manifest_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches WHERE id = ?1")
            .bind(archive_id)
            .fetch_one(&pool)
            .await
            .expect("count expired live mirror manifest");
    assert_eq!(manifest_count, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn materialize_historical_rollups_skips_missing_archives_and_preserves_existing_rollups() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-materialize-atomic").await;
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
    let missing_archive_path =
        archive_batch_file_path(&config, "codex_invocations", &archived_occurred_at[..7])
            .expect("resolve missing archive path");

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
    .bind(7_i64)
    .bind(6_i64)
    .bind(1_i64)
    .bind(77_i64)
    .bind(7.7_f64)
    .bind("[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&pool)
    .await
    .expect("seed existing invocation rollup");
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
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&archived_occurred_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&archived_occurred_at)
    .bind(&archived_occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing archive manifest");

    let snapshot_before = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load snapshot before skipping missing archive");
    assert_eq!(snapshot_before.legacy_archive_pending, 0);

    let summary = materialize_historical_rollups(&pool, &config, false)
        .await
        .expect("materialization should skip missing archive file");
    assert_eq!(summary.materialized_invocation_batches, 0);

    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load retained invocation rollup total");
    assert_eq!(
        total_count, 7,
        "skipped materialization must keep prior rollups"
    );

    let prune_summary = prune_legacy_archive_batches(&pool, &config, false)
        .await
        .expect("prune should retain stale metadata without long-term replay");
    assert_eq!(prune_summary.deleted_archive_batches, 0);

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches after pruning missing metadata");
    assert_eq!(remaining_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn recompute_invocation_hourly_rollups_ignores_archive_manifests_for_live_buckets() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("historical-rollup-recompute-live-only").await;
    let live_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days((config.invocation_success_full_days + 5) as i64))
    .and_hms_opt(9, 20, 0)
    .expect("valid live historical local hour");
    let live_occurred_at = format_naive(live_local);
    insert_retention_invocation(
        &pool,
        "historical-rollup-recompute-live-only",
        &live_occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"promptCacheKey\":\"live-key\"}"),
        "{}",
        None,
        None,
        Some(5),
        Some(0.5),
    )
    .await;

    let live_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
            .bind("historical-rollup-recompute-live-only")
            .fetch_one(&pool)
            .await
            .expect("load live invocation id");
    let bucket_start_epoch =
        invocation_bucket_start_epoch(&live_occurred_at).expect("invocation bucket epoch");
    let missing_archive_path =
        archive_batch_file_path(&config, "codex_invocations", &live_occurred_at[..7])
            .expect("resolve missing archive path");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            historical_rollups_materialized_at,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), ?7, ?8, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind(&live_occurred_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&live_occurred_at)
    .bind(&live_occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing archive manifest for same live month");

    let mut tx = pool.begin().await.expect("begin recompute tx");
    sqlx::query("UPDATE codex_invocations SET total_tokens = ?1, cost = ?2 WHERE id = ?3")
        .bind(15_i64)
        .bind(1.5_f64)
        .bind(live_id)
        .execute(tx.as_mut())
        .await
        .expect("update live invocation before recompute");
    recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[live_id])
        .await
        .expect("recompute invocation hourly rollups without archive dependency");
    tx.commit().await.expect("commit recompute tx");

    let after = sqlx::query_as::<_, StatsRow>(
        r#"
        SELECT
            COALESCE(SUM(total_count), 0) AS total_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_cost), 0.0) AS total_cost,
            COALESCE(SUM(total_tokens), 0) AS total_tokens,
            COALESCE(SUM(non_success_cost), 0.0) AS non_success_cost
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch = ?1 AND source = ?2
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load post-recompute bucket totals");
    assert_eq!(after.total_count, 1);
    assert_eq!(after.success_count, Some(1));
    assert_eq!(after.failure_count, Some(0));
    assert_eq!(after.total_tokens, 15);
    assert_f64_close(after.total_cost, 1.5);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archives_and_cleans_up_pool_upstream_request_attempts() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-pool-attempts").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 30;
    config.retention_batch_rows = 10;

    let old_occurred_at = shanghai_local_days_ago(10, 9, 30, 0);
    let recent_occurred_at = shanghai_local_days_ago(1, 9, 30, 0);
    insert_retention_pool_upstream_request_attempt(
        &pool,
        "retention-pool-attempts-old",
        &old_occurred_at,
        Some(7),
        1,
        1,
        1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(200),
        None,
        Some(&old_occurred_at),
        Some(&old_occurred_at),
    )
    .await;
    insert_retention_pool_upstream_request_attempt(
        &pool,
        "retention-pool-attempts-recent",
        &recent_occurred_at,
        Some(8),
        1,
        1,
        1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(200),
        None,
        Some(&recent_occurred_at),
        Some(&recent_occurred_at),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "retention-pool-attempts-old",
        &old_occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":7}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run pool attempt retention");
    assert_eq!(summary.pool_upstream_request_attempt_rows_archived, 1);
    assert_eq!(summary.archive_batches_deleted, 0);

    let remaining_old_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE occurred_at < ?1",
    )
    .bind(shanghai_local_cutoff_string(
        config.pool_upstream_request_attempts_retention_days,
    ))
    .fetch_one(&pool)
    .await
    .expect("count old pool attempt rows");
    assert_eq!(remaining_old_rows, 0);

    let recent_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id = ?1",
    )
    .bind("retention-pool-attempts-recent")
    .fetch_one(&pool)
    .await
    .expect("count recent pool attempt rows");
    assert_eq!(recent_rows, 1);

    let archive_batch = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT file_path, archive_expires_at
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load pool attempt archive batch");
    let archive_path = PathBuf::from(&archive_batch.0);
    assert!(archive_path.exists(), "archive file should exist");
    assert!(
        archive_batch.1.is_some(),
        "archive batch should carry expiry"
    );

    sqlx::query(
        "UPDATE archive_batches SET archive_expires_at = ?1 WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .bind("2000-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("expire archive batch");

    sqlx::query("UPDATE long_term_stats_state SET status = 'empty' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats empty before attempt archive cleanup");
    let archive_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load pool attempt archive checksum");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(&archive_batch.0)
    .bind(archive_sha256)
    .execute(&pool)
    .await
    .expect("mark pool attempt archive replay complete");

    let cleanup_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run pool attempt archive ttl cleanup");
    assert_eq!(cleanup_summary.archive_batches_deleted, 1);
    assert!(
        !archive_path.exists(),
        "expired pool attempt archive file should be removed"
    );

    let remaining_archive_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining pool attempt archive batches");
    assert_eq!(remaining_archive_batches, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_accumulates_pool_upstream_hourly_history_when_reusing_monthly_batch_file() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-pool-hourly-accumulates").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 365;
    config.retention_batch_rows = 1;

    let binding_key = "fpn-retention-hourly-accumulates";
    let archive_month_prefix = (Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::days(31))
    .format("%Y-%m")
    .to_string();
    let first_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-10 09:30:00"))
        .expect("first attempt timestamp should parse");
    let second_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-11 09:30:00"))
        .expect("second attempt timestamp should parse");
    seed_pool_upstream_attempt_at(
        &pool,
        "retention-pool-hourly-accumulates-success",
        first_attempt_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let first_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run first pool upstream retention pass");
    assert_eq!(first_summary.pool_upstream_request_attempt_rows_archived, 1);

    seed_pool_upstream_attempt_at(
        &pool,
        "retention-pool-hourly-accumulates-failure",
        second_attempt_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let second_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run second pool upstream retention pass");
    assert_eq!(
        second_summary.pool_upstream_request_attempt_rows_archived,
        1
    );

    let archive_batches =
        sqlx::query_as::<_, (i64, String, String, Option<String>, Option<String>, i64)>(
            r#"
        SELECT id, month_key, file_path, day_key, part_key, row_count
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
        ORDER BY id
        "#,
        )
        .fetch_all(&pool)
        .await
        .expect("load retained pool upstream archive batches");
    let archive_batch_count = archive_batches.len() as i64;
    assert_eq!(
        archive_batch_count, 1,
        "both retention passes should append into the same monthly archive batch: {:?}",
        archive_batches
    );

    let materialized = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count
        FROM pool_upstream_node_health_hourly_archive
        WHERE proxy_binding_key_snapshot = ?1
        "#,
    )
    .bind(binding_key)
    .fetch_one(&pool)
    .await
    .expect("load materialized pool upstream hourly history");
    assert_eq!(materialized.0, 1);
    assert_eq!(materialized.1, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_keeps_preexisting_pool_node_health_month_archives_pending_after_append() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-pool-hourly-preexisting-pending").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 365;
    config.retention_batch_rows = 1;

    let binding_key = "fpn-retention-hourly-preexisting-pending";
    let archive_month_prefix = (Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::days(31))
    .format("%Y-%m")
    .to_string();
    let first_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-12 09:30:00"))
        .expect("first attempt timestamp should parse");
    let second_attempt_at = parse_to_utc_datetime(&format!("{archive_month_prefix}-13 09:30:00"))
        .expect("second attempt timestamp should parse");

    seed_pool_upstream_attempt_at(
        &pool,
        "retention-pool-hourly-preexisting-pending-success",
        first_attempt_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let first_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run first pool upstream retention pass");
    assert_eq!(first_summary.pool_upstream_request_attempt_rows_archived, 1);

    let archive_path = sqlx::query_scalar::<_, String>(
        r#"
        SELECT file_path
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load existing pool upstream month archive path");

    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(&pool)
        .await
        .expect("clear cached pool node health archive rows");
    sqlx::query("DELETE FROM pool_upstream_node_health_hourly_archive")
        .execute(&pool)
        .await
        .expect("clear cached pool node health hourly archive rows");
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'pool_upstream_request_attempts'
          AND target IN (?1, ?2)
          AND file_path = ?3
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("clear pool node health replay markers for existing archive");
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = NULL
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("clear historical rollup materialized timestamp");

    seed_pool_upstream_attempt_at(
        &pool,
        "retention-pool-hourly-preexisting-pending-failure",
        second_attempt_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;

    let second_summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run second pool upstream retention pass");
    assert_eq!(
        second_summary.pool_upstream_request_attempt_rows_archived,
        1
    );

    let pending_cache_batches = pending_pool_upstream_node_health_archive_batches(&pool)
        .await
        .expect("count pending cached pool node health archives");
    let pending_hourly_batches = pending_pool_upstream_node_health_hourly_archive_batches(&pool)
        .await
        .expect("count pending hourly pool node health archives");
    assert_eq!(pending_cache_batches, 1);
    assert_eq!(pending_hourly_batches, 1);

    let replay_markers = sqlx::query_as::<_, (String, i64)>(
        r#"
        SELECT target, COUNT(*) AS replay_count
        FROM hourly_rollup_archive_replay
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
          AND target IN (?2, ?3)
        GROUP BY target
        ORDER BY target
        "#,
    )
    .bind(&archive_path)
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .fetch_all(&pool)
    .await
    .expect("load replay markers for existing pool node health archive");
    assert!(
        replay_markers.is_empty(),
        "preexisting month archives should stay pending after append until full backfill runs: {:?}",
        replay_markers
    );

    let materialized_at: Option<String> = sqlx::query_scalar(
        r#"
        SELECT historical_rollups_materialized_at
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load historical rollup materialized timestamp after append");
    assert!(materialized_at.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn refreshing_pool_node_health_hourly_cache_from_row_cache_is_idempotent() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-hourly-cache-idempotent").await;
    let archive_batch_id = 42_i64;
    let archive_file_path = temp_dir
        .join("hourly-cache-idempotent.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let binding_key = "fpn-hourly-cache-idempotent";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(40))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(15)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );
    let bucket_start_epoch = align_bucket_epoch(
        parse_shanghai_local_naive(&occurred_at)
            .expect("parse shanghai occurred_at")
            .and_local_timezone(Shanghai)
            .single()
            .expect("localize shanghai occurred_at")
            .with_timezone(&Utc)
            .timestamp(),
        3600,
        0,
    );

    for (archived_row_id, is_success) in [(1_i64, 1_i64), (2_i64, 0_i64)] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_node_health_archive (
                archive_file_path,
                archived_row_id,
                occurred_at,
                proxy_binding_key_snapshot,
                is_success,
                latency_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, NULL)
            "#,
        )
        .bind(&archive_file_path)
        .bind(archived_row_id)
        .bind(&occurred_at)
        .bind(binding_key)
        .bind(is_success)
        .execute(&pool)
        .await
        .expect("insert cached pool node health row");
    }

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_node_health_hourly_archive (
            archive_identity,
            archive_batch_id,
            archive_file_path,
            proxy_binding_key_snapshot,
            bucket_start_epoch,
            success_count,
            failure_count
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(pool_upstream_node_health_archive_identity_for_batch_id(
        archive_batch_id,
    ))
    .bind(archive_batch_id)
    .bind(&archive_file_path)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .bind(9_i64)
    .bind(4_i64)
    .execute(&pool)
    .await
    .expect("insert stale cached hourly row");

    for _ in 0..2 {
        let mut tx = pool.begin().await.expect("begin hourly refresh tx");
        refresh_pool_upstream_node_health_hourly_archive_rows_from_cache_tx(
            tx.as_mut(),
            archive_batch_id,
            &archive_file_path,
        )
        .await
        .expect("refresh hourly rows from cached rows");
        tx.commit().await.expect("commit hourly refresh tx");
    }

    let refreshed = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_batch_id = ?1
          AND proxy_binding_key_snapshot = ?2
          AND bucket_start_epoch = ?3
        "#,
    )
    .bind(archive_batch_id)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .fetch_one(&pool)
    .await
    .expect("load refreshed hourly cache row");
    assert_eq!(refreshed.0, 1);
    assert_eq!(refreshed.1, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pool_node_health_hourly_backfill_waits_for_cache_replay_and_refreshes_from_cache() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-hourly-cache-replay-gate").await;
    let archive_file_path = temp_dir
        .join("hourly-cache-replay-gate.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let binding_key = "fpn-hourly-cache-replay-gate";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(45))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(10)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );
    let month_key = archive_month_key_from_day_key(&occurred_at[..10])
        .expect("derive archive month key from occurred_at");
    let bucket_start_epoch = align_bucket_epoch(
        parse_shanghai_local_naive(&occurred_at)
            .expect("parse shanghai occurred_at")
            .and_local_timezone(Shanghai)
            .single()
            .expect("localize shanghai occurred_at")
            .with_timezone(&Utc)
            .timestamp(),
        3600,
        0,
    );

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
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&month_key)
    .bind(&archive_file_path)
    .bind("hourly-cache-replay-gate")
    .bind(2_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert pool upstream archive batch");
    let archive_batch_id: i64 = sqlx::query_scalar(
        r#"
        SELECT id
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        LIMIT 1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load pool upstream archive batch id");

    for (archived_row_id, is_success) in [(1_i64, 1_i64), (2_i64, 0_i64)] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_node_health_archive (
                archive_file_path,
                archived_row_id,
                occurred_at,
                proxy_binding_key_snapshot,
                is_success,
                latency_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, NULL)
            "#,
        )
        .bind(&archive_file_path)
        .bind(archived_row_id)
        .bind(&occurred_at)
        .bind(binding_key)
        .bind(is_success)
        .execute(&pool)
        .await
        .expect("insert cached pool node health row");
    }

    let first = backfill_pool_upstream_node_health_hourly_archives(&pool, None, None)
        .await
        .expect("hourly backfill should wait for cache replay");
    assert_eq!(first.materialized_batches, 0);
    assert_eq!(first.pending_batches, 1);
    let hourly_rows_before: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_batch_id = ?1
        "#,
    )
    .bind(archive_batch_id)
    .fetch_one(&pool)
    .await
    .expect("count hourly rows before cache replay completes");
    assert_eq!(hourly_rows_before, 0);

    let archive_sha256: String = sqlx::query_scalar(
        r#"
        SELECT sha256
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load pool upstream archive checksum");
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
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind("pool_upstream_request_attempts")
    .bind(&archive_file_path)
    .bind(&archive_sha256)
    .execute(&pool)
    .await
    .expect("mark cached pool node health replay complete");

    let second = backfill_pool_upstream_node_health_hourly_archives(&pool, None, None)
        .await
        .expect("hourly backfill should refresh from cached pool node health rows");
    assert_eq!(second.materialized_batches, 1);
    assert_eq!(second.pending_batches, 0);

    let refreshed = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_batch_id = ?1
          AND proxy_binding_key_snapshot = ?2
          AND bucket_start_epoch = ?3
        "#,
    )
    .bind(archive_batch_id)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .fetch_one(&pool)
    .await
    .expect("load refreshed hourly cache row");
    assert_eq!(refreshed.0, 1);
    assert_eq!(refreshed.1, 1);

    let hourly_replayed: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
          AND file_path = ?2
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count hourly replay markers after cache refresh");
    assert_eq!(hourly_replayed, 1);

    sqlx::query(
        r#"
        UPDATE archive_batches
        SET sha256 = 'replaced-hourly-cache-replay-gate'
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .execute(&pool)
    .await
    .expect("replace pool upstream archive checksum");

    assert_eq!(
        pending_pool_upstream_node_health_archive_batches(&pool)
            .await
            .expect("load cache archives pending after checksum replacement"),
        1,
        "a stale cache replay marker must reopen the archive"
    );
    assert_eq!(
        pending_pool_upstream_node_health_hourly_archive_batches(&pool)
            .await
            .expect("load hourly archives pending after checksum replacement"),
        1,
        "a stale hourly replay marker must reopen the archive"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn node_health_cache_replacement_discards_removed_archive_attempts_before_marking_sha_b() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("node-health-cache-replacement").await;
    let archive_path = temp_dir.join("node-health-cache-replacement.sqlite.gz");
    let archive_db_path = temp_dir.join("node-health-cache-replacement.sqlite");
    let archive_file_path = archive_path.to_string_lossy().to_string();
    let binding_key = "node-health-cache-replacement";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(45))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(10)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );

    fs::File::create(&archive_db_path).expect("create replacement archive sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open replacement archive sqlite");
    let create_sql = POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create replacement archive schema");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            id,
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            proxy_binding_key_snapshot,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            finished_at,
            status,
            first_byte_latency_ms,
            created_at
        )
        VALUES (1, 'node-health-cache-replacement-keep', ?1, '/v1/responses', 'pool', ?2, 0, 0, 0, ?1, ?3, 120.0, ?1)
        "#,
    )
    .bind(&occurred_at)
    .bind(binding_key)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
    .execute(&archive_pool)
    .await
    .expect("write surviving replacement archive attempt");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress replacement archive");
    let replacement_sha = sha256_hex_file(&archive_path).expect("hash replacement archive");

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
            created_at
        )
        VALUES ('pool_upstream_request_attempts', ?1, ?2, ?3, 1, ?4, ?5, ?5, datetime('now'))
        "#,
    )
    .bind(&occurred_at[..7])
    .bind(&archive_file_path)
    .bind(&replacement_sha)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("publish replacement archive manifest");

    for (archived_row_id, is_success) in [(1_i64, 1_i64), (2_i64, 0_i64)] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_node_health_archive (
                archive_file_path,
                archived_row_id,
                occurred_at,
                proxy_binding_key_snapshot,
                is_success,
                latency_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, NULL)
            "#,
        )
        .bind(&archive_file_path)
        .bind(archived_row_id)
        .bind(&occurred_at)
        .bind(binding_key)
        .bind(is_success)
        .execute(&pool)
        .await
        .expect("seed stale node health cache row");
    }
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
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
            VALUES (?1, 'pool_upstream_request_attempts', ?2, 'archive-sha-a', datetime('now'))
            "#,
        )
        .bind(target)
        .bind(&archive_file_path)
        .execute(&pool)
        .await
        .expect("seed stale node health replay marker");
    }

    let cache_summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
        .await
        .expect("replace stale node health cache from archive SHA B");
    assert_eq!(cache_summary.materialized_batches, 1);
    assert_eq!(cache_summary.pending_batches, 0);
    let cached = sqlx::query_as::<_, (i64, i64, i64)>(
        r#"
        SELECT
            COUNT(*),
            COALESCE(SUM(is_success), 0),
            COALESCE(SUM(CASE WHEN is_success = 0 THEN 1 ELSE 0 END), 0)
        FROM pool_upstream_node_health_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load replaced node health cache rows");
    assert_eq!(cached, (1, 1, 0));

    let hourly_summary = backfill_pool_upstream_node_health_hourly_archives(&pool, None, None)
        .await
        .expect("rebuild node health hourly rows from replaced cache");
    assert_eq!(hourly_summary.materialized_batches, 1);
    assert_eq!(hourly_summary.pending_batches, 0);
    let hourly = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT COALESCE(SUM(success_count), 0), COALESCE(SUM(failure_count), 0)
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load node health hourly rows after cache replacement");
    assert_eq!(hourly, (1, 0));
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
    ] {
        let marked_sha: String = sqlx::query_scalar(
            "SELECT archive_sha256 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'pool_upstream_request_attempts' AND file_path = ?2",
        )
        .bind(target)
        .bind(&archive_file_path)
        .fetch_one(&pool)
        .await
        .expect("load replacement replay marker");
        assert_eq!(marked_sha, replacement_sha);
    }

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn node_health_archives_with_blank_manifest_sha_stay_quarantined() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("node-health-blank-manifest-quarantine").await;
    let archive_file_path = temp_dir
        .join("node-health-blank-manifest-quarantine.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let binding_key = "node-health-blank-manifest-quarantine";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(45))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(10)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );
    let month_key = archive_month_key_from_day_key(&occurred_at[..10])
        .expect("derive archive month key from occurred_at");
    let bucket_start_epoch = align_bucket_epoch(
        parse_shanghai_local_naive(&occurred_at)
            .expect("parse shanghai occurred_at")
            .and_local_timezone(Shanghai)
            .single()
            .expect("localize shanghai occurred_at")
            .with_timezone(&Utc)
            .timestamp(),
        3600,
        0,
    );

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
            created_at
        )
        VALUES ('pool_upstream_request_attempts', ?1, ?2, '', 1, ?3, ?4, ?4, datetime('now'))
        "#,
    )
    .bind(&month_key)
    .bind(&archive_file_path)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert blank-manifest pool upstream archive batch");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load blank-manifest archive batch id");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_node_health_archive (
            archive_file_path,
            archived_row_id,
            occurred_at,
            proxy_binding_key_snapshot,
            is_success,
            latency_ms
        )
        VALUES (?1, 1, ?2, ?3, 0, NULL)
        "#,
    )
    .bind(&archive_file_path)
    .bind(&occurred_at)
    .bind(binding_key)
    .execute(&pool)
    .await
    .expect("seed node health cache protected by blank manifest");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_node_health_hourly_archive (
            archive_identity,
            archive_batch_id,
            archive_file_path,
            proxy_binding_key_snapshot,
            bucket_start_epoch,
            success_count,
            failure_count
        )
        VALUES (?1, ?2, ?3, ?4, ?5, 0, 1)
        "#,
    )
    .bind(pool_upstream_node_health_archive_identity_for_batch_id(
        archive_batch_id,
    ))
    .bind(archive_batch_id)
    .bind(&archive_file_path)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .execute(&pool)
    .await
    .expect("seed node health hourly cache protected by blank manifest");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, '')",
        )
        .bind(target)
        .bind(&archive_file_path)
        .execute(&pool)
        .await
        .expect("seed blank node health replay marker");
    }

    for _ in 0..2 {
        let cache = backfill_pool_upstream_node_health_archives(&pool, None, None)
            .await
            .expect("blank-manifest cache archive must remain quarantined");
        assert_eq!(cache.materialized_batches, 0);
        assert_eq!(cache.pending_batches, 1);
        let hourly = backfill_pool_upstream_node_health_hourly_archives(&pool, None, None)
            .await
            .expect("blank-manifest hourly archive must remain quarantined");
        assert_eq!(hourly.materialized_batches, 0);
        assert_eq!(hourly.pending_batches, 1);
    }

    let cached: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(is_success), 0) FROM pool_upstream_node_health_archive WHERE archive_file_path = ?1",
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load quarantined node health cache");
    assert_eq!(cached, (1, 0));
    let hourly: (i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(success_count), 0), COALESCE(SUM(failure_count), 0) FROM pool_upstream_node_health_hourly_archive WHERE archive_file_path = ?1",
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load quarantined node health hourly cache");
    assert_eq!(hourly, (1, 0, 1));
    let replay_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1 AND archive_sha256 = ''",
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count blank replay markers left quarantined");
    assert_eq!(replay_marker_count, 2);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn missing_pool_node_health_archives_remain_pending_with_stale_cached_rows() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-missing-archive-clears-cache").await;
    let archive_file_path = temp_dir
        .join("missing-node-health-archive.sqlite.gz")
        .to_string_lossy()
        .to_string();
    let binding_key = "fpn-missing-archive-clears-cache";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::days(45))
            .with_timezone(&Shanghai)
            .naive_local()
            .with_minute(0)
            .expect("set minute")
            .with_second(0)
            .expect("set second"),
    );

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
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&occurred_at[..7])
    .bind(&archive_file_path)
    .bind("missing-node-health-archive")
    .bind(2_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing pool upstream archive manifest");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_node_health_archive (
            archive_file_path,
            archived_row_id,
            occurred_at,
            proxy_binding_key_snapshot,
            is_success,
            latency_ms
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(&archive_file_path)
    .bind(1_i64)
    .bind(&occurred_at)
    .bind(binding_key)
    .bind(1_i64)
    .bind(120.0_f64)
    .execute(&pool)
    .await
    .expect("seed stale cached node health row");

    let summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
        .await
        .expect("backfill should retain missing archives pending");
    assert_eq!(summary.pending_batches, 1);

    let cached_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count cached rows after missing archive replay");
    assert_eq!(cached_rows, 1);

    let replayed: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
          AND file_path = ?2
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("count replay marker for missing archive");
    assert_eq!(replayed, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_pool_upstream_archives_waits_for_cache_replay_completion() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("pool-node-health-cleanup-cache-replay-gate").await;
    let coverage_end_at = shanghai_local_days_ago(14, 9, 0, 0);
    let archive_file_path = archive_batch_file_path(
        &config,
        "pool_upstream_request_attempts",
        &coverage_end_at[..7],
    )
    .expect("resolve expired pool upstream archive path");
    fs::create_dir_all(archive_file_path.parent().expect("archive parent"))
        .expect("create archive parent for cleanup gate");
    fs::write(&archive_file_path, b"placeholder-archive")
        .expect("seed raw archive file for cleanup gate");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            historical_rollups_materialized_at,
            archive_expires_at,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&coverage_end_at[..7])
    .bind(archive_file_path.to_string_lossy().to_string())
    .bind("cache-replay-gate")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(shanghai_local_days_ago(1, 0, 0, 0))
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert expired pool upstream archive batch");
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .bind("pool_upstream_request_attempts")
    .bind(archive_file_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("mark hourly replay complete while cache replay stays pending");

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup should respect pending cache replay");
    assert_eq!(deleted, 0);
    assert!(
        archive_file_path.exists(),
        "raw archive must remain until cache replay completes"
    );

    let remaining_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining pool upstream archive batches");
    assert_eq!(remaining_batches, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn cleanup_expired_pool_upstream_archives_preserves_recent_exact_window_history() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("pool-node-health-cleanup-window-gate").await;
    let coverage_end_at = shanghai_local_days_ago(2, 9, 0, 0);
    let archive_file_path = archive_batch_file_path(
        &config,
        "pool_upstream_request_attempts",
        &coverage_end_at[..7],
    )
    .expect("resolve recent pool upstream archive path");
    fs::create_dir_all(archive_file_path.parent().expect("archive parent"))
        .expect("create archive parent for window gate");
    fs::write(&archive_file_path, b"placeholder-archive")
        .expect("seed raw archive file for window gate");

    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset,
            month_key,
            file_path,
            sha256,
            row_count,
            status,
            historical_rollups_materialized_at,
            archive_expires_at,
            coverage_start_at,
            coverage_end_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), ?7, ?8, ?9, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&coverage_end_at[..7])
    .bind(archive_file_path.to_string_lossy().to_string())
    .bind("recent-window-gate")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(shanghai_local_days_ago(1, 0, 0, 0))
    .bind(&coverage_end_at)
    .bind(&coverage_end_at)
    .execute(&pool)
    .await
    .expect("insert recent expired pool upstream archive batch");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
            VALUES (?1, ?2, ?3, datetime('now'))
            "#,
        )
        .bind(target)
        .bind("pool_upstream_request_attempts")
        .bind(archive_file_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("mark node health replay complete");
    }

    let deleted = cleanup_expired_archive_batches(&pool, &config, false)
        .await
        .expect("cleanup should keep recent exact node health history");
    assert_eq!(deleted, 0);
    assert!(
        archive_file_path.exists(),
        "raw archive should remain while Live/Settings windows can still overlap it"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pool_upstream_node_health_archive_backfill_reuses_stable_temp_db_when_budget_is_hit() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-archive-temp-reuse").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 30;
    let row_count = (BACKFILL_BATCH_SIZE as usize * 6) + 17;
    config.retention_batch_rows = row_count;

    let old_occurred_at = shanghai_local_days_ago(10, 9, 30, 0);
    let old_base = parse_shanghai_local_naive(&old_occurred_at).expect("valid shanghai local");
    for idx in 0..row_count {
        let occurred_at = format_naive(old_base + ChronoDuration::seconds(idx as i64));
        insert_retention_pool_upstream_request_attempt(
            &pool,
            &format!("pool-node-health-archive-temp-reuse-old-{idx}"),
            &occurred_at,
            Some(7),
            1,
            1,
            1,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            Some(200),
            None,
            Some(&occurred_at),
            Some(&occurred_at),
        )
        .await;
    }

    super::retention_task_local_batches::archive_two_attempt_tasks(&pool, &config, row_count).await;

    let archive_path = PathBuf::from(
        sqlx::query_scalar::<_, String>(
            r#"
            SELECT file_path
            FROM archive_batches
            WHERE dataset = 'pool_upstream_request_attempts'
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load pool attempt archive batch path"),
    );
    let temp_path = pool_upstream_node_health_archive_temp_path(&archive_path);

    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(&pool)
        .await
        .expect("clear cached node health archive rows");
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .execute(&pool)
    .await
    .expect("clear node health archive replay markers");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_progress WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .execute(&pool)
    .await
    .expect("clear node health archive replay progress");

    let first = backfill_pool_upstream_node_health_archives(
        &pool,
        Some(1),
        Some(std::time::Duration::from_millis(1)),
    )
    .await
    .expect("run budget-limited node health archive backfill");
    assert!(first.hit_budget);
    assert!(
        temp_path.exists(),
        "budget-limited backfill should keep a stable temp sqlite for reuse"
    );

    let second = backfill_pool_upstream_node_health_archives(
        &pool,
        Some(1),
        Some(std::time::Duration::from_millis(1)),
    )
    .await
    .expect("rerun budget-limited node health archive backfill");
    let matching_temp_files = fs::read_dir(
        temp_path
            .parent()
            .expect("stable temp sqlite should have a parent directory"),
    )
    .expect("list archive temp directory")
    .filter_map(|entry| entry.ok())
    .filter(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .ends_with(".pool-upstream-node-health.sqlite")
    })
    .count();
    assert!(
        matching_temp_files <= 1,
        "budget-limited retries must not leak multiple temp sqlite files"
    );
    if second.hit_budget {
        assert!(
            temp_path.exists(),
            "stable temp sqlite should be reused across passes"
        );
        let final_summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
            .await
            .expect("finish node health archive backfill");
        assert!(!final_summary.hit_budget);
        assert_eq!(final_summary.pending_batches, 0);
        assert!(
            !temp_path.exists(),
            "completed node health archive replay should clean up the stable temp sqlite"
        );
    } else {
        assert_eq!(second.pending_batches, 0);
        assert!(
            !temp_path.exists(),
            "a completed retry should clean up the stable temp sqlite instead of leaking it"
        );
    }

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pool_upstream_node_health_archive_backfill_keeps_missing_archives_pending() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-missing-archive").await;
    let missing_occurred_at = shanghai_local_days_ago(45, 9, 0, 0);
    let month_key = missing_occurred_at[..7].to_string();
    let missing_archive_path =
        archive_batch_file_path(&config, "pool_upstream_request_attempts", &month_key)
            .expect("resolve missing pool node health archive path");
    let missing_temp_path = pool_upstream_node_health_archive_temp_path(&missing_archive_path);
    fs::create_dir_all(
        missing_temp_path
            .parent()
            .expect("missing archive temp path should have a parent"),
    )
    .expect("create missing archive temp directory");
    fs::write(&missing_temp_path, b"stale temp sqlite")
        .expect("seed stale missing-archive temp sqlite");
    fs::write(
        temp_sqlite_source_meta_path(&missing_temp_path),
        "stale-manifest-sha",
    )
    .expect("seed stale missing-archive temp sidecar");

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
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
        "#,
    )
    .bind("pool_upstream_request_attempts")
    .bind(&month_key)
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&missing_occurred_at)
    .bind(&missing_occurred_at)
    .execute(&pool)
    .await
    .expect("insert missing pool node health archive manifest");

    let pending_before = pending_pool_upstream_node_health_archive_batches(&pool)
        .await
        .expect("count pending pool node health archive batches before replay");
    assert_eq!(pending_before, 1);

    let summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
        .await
        .expect("missing pool node health archive should remain pending");
    assert!(!summary.hit_budget);
    assert_eq!(summary.pending_batches, 1);

    let replay_marked: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
          AND file_path = ?2
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(missing_archive_path.to_string_lossy().to_string())
    .fetch_one(&pool)
    .await
    .expect("count replay markers for missing pool node health archive");
    assert_eq!(replay_marked, 0);
    assert!(!missing_temp_path.exists());
    assert!(!temp_sqlite_source_meta_path(&missing_temp_path).exists());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn pool_upstream_node_health_archive_backfill_refreshes_stale_temp_after_archive_rewrite() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-archive-refresh-temp").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 30;
    let row_count = (BACKFILL_BATCH_SIZE as usize * 6) + 17;
    config.retention_batch_rows = row_count;
    let binding_key = "stale-temp-rewrite-binding";

    let old_occurred_at = shanghai_local_days_ago(10, 11, 0, 0);
    let old_base = parse_shanghai_local_naive(&old_occurred_at).expect("valid shanghai local");
    for idx in 0..row_count {
        let occurred_at = Shanghai
            .from_local_datetime(&(old_base + ChronoDuration::seconds(idx as i64)))
            .single()
            .expect("valid shanghai local old pool attempt")
            .with_timezone(&Utc);
        seed_pool_upstream_attempt_at(
            &pool,
            &format!("pool-node-health-archive-refresh-temp-old-{idx}"),
            occurred_at,
            Some(binding_key),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        )
        .await;
    }

    super::retention_task_local_batches::archive_two_attempt_tasks(&pool, &config, row_count).await;

    let archive_path = PathBuf::from(
        sqlx::query_scalar::<_, String>(
            r#"
            SELECT file_path
            FROM archive_batches
            WHERE dataset = 'pool_upstream_request_attempts'
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load rewritten pool attempt archive batch path"),
    );
    let temp_path = pool_upstream_node_health_archive_temp_path(&archive_path);

    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(&pool)
        .await
        .expect("clear cached node health archive rows");
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE target = ?1
          AND dataset = 'pool_upstream_request_attempts'
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .execute(&pool)
    .await
    .expect("clear node health archive replay markers");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_progress WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .execute(&pool)
    .await
    .expect("clear node health archive replay progress");

    let mut first = None;
    let mut partial_cursor = 0_i64;
    for budget_ms in [1_u64, 5, 10, 20, 40] {
        let summary = backfill_pool_upstream_node_health_archives(
            &pool,
            Some(1),
            Some(std::time::Duration::from_millis(budget_ms)),
        )
        .await
        .expect("run budget-limited node health archive backfill before archive rewrite");
        partial_cursor = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_archive_progress WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
        )
        .bind(archive_path.to_string_lossy().to_string())
        .fetch_optional(&pool)
        .await
        .expect("load partial node health archive replay cursor")
        .unwrap_or(0);
        first = Some(summary);
        if partial_cursor > 0 {
            break;
        }
    }
    let first = first.expect("expected at least one budget-limited replay attempt");
    assert!(first.hit_budget);
    assert!(
        partial_cursor > 0,
        "stale-temp rewrite coverage requires a partial replay cursor before the month archive is rewritten"
    );
    assert!(
        temp_path.exists(),
        "budget-limited replay should keep a temp sqlite"
    );

    std::thread::sleep(std::time::Duration::from_millis(10));
    let appended_occurred_at = Shanghai
        .from_local_datetime(&(old_base + ChronoDuration::seconds(row_count as i64 + 60)))
        .single()
        .expect("valid shanghai local appended pool attempt")
        .with_timezone(&Utc);
    seed_pool_upstream_attempt_at(
        &pool,
        "pool-node-health-archive-refresh-temp-appended",
        appended_occurred_at,
        Some(binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let second_retention = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("rerun retention so the month archive gets rewritten with a new appended row");
    assert_eq!(
        second_retention.pool_upstream_request_attempt_rows_archived,
        1
    );

    let final_summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
        .await
        .expect("finish node health archive replay after archive rewrite");
    assert!(!final_summary.hit_budget);
    assert_eq!(final_summary.pending_batches, 0);

    let cached_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_node_health_archive")
            .fetch_one(&pool)
            .await
            .expect("count cached node health archive rows after archive rewrite");
    assert_eq!(
        cached_rows,
        (row_count + 1) as i64,
        "rewritten month archives must invalidate stale temp sqlite copies so appended rows are replayed"
    );
    assert!(
        !temp_path.exists(),
        "completed replay after archive rewrite should clean up the temp sqlite"
    );

    cleanup_temp_test_dir(&temp_dir);
}
