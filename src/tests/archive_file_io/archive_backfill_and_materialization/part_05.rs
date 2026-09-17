use super::*;

async fn seed_same_bucket_partial_breakdown(
    pool: &SqlitePool,
    config: &AppConfig,
) -> (String, i64) {
    let first_local = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 10, 0);
    let second_local = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 20, 0);
    insert_retention_invocation(
        pool,
        "bootstrap-reopens-same-bucket-group-a",
        &first_local,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Replay A\",\"responseModel\":\"gpt-5\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    insert_retention_invocation(
        pool,
        "bootstrap-reopens-same-bucket-group-b",
        &second_local,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":18,\"upstreamAccountName\":\"Replay B\",\"responseModel\":\"gpt-4o\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(64),
        Some(0.64),
    )
    .await;
    sync_hourly_rollups_from_live_tables(pool)
        .await
        .expect("seed live hourly rollups before same-bucket repair");
    let retention = run_data_retention_maintenance(pool, config, Some(false), None)
        .await
        .expect("archive old rows before same-bucket partial breakdown repair");
    assert_eq!(retention.invocation_rows_archived, 2);
    let archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load invocation archive path");
    let bucket_epoch = local_naive_to_utc(
        parse_shanghai_local_naive(&first_local).expect("valid same-bucket local timestamp"),
        Shanghai,
    )
    .timestamp()
        / 3_600
        * 3_600;
    sqlx::query(
        "DELETE FROM upstream_account_usage_breakdown_hourly
         WHERE bucket_start_epoch = ?1 AND upstream_account_id = ?2",
    )
    .bind(bucket_epoch)
    .bind(18_i64)
    .execute(pool)
    .await
    .expect("drop one same-bucket breakdown group to mimic partial backfill");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay
         WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_path)
    .execute(pool)
    .await
    .expect("invalidate the usage breakdown proof after removing one breakdown group");
    sqlx::query(
        "INSERT INTO hourly_rollup_materialized_buckets
         (target, bucket_start_epoch, source, materialized_at)
         VALUES (?1, ?2, ?3, datetime('now'))
         ON CONFLICT(target, bucket_start_epoch, source) DO NOTHING",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(bucket_epoch)
    .bind(SOURCE_PROXY)
    .execute(pool)
    .await
    .expect("seed same-bucket invocation materialized marker");
    (archive_path, bucket_epoch)
}

async fn assert_missing_archive_preserved(pool: &SqlitePool, config: &AppConfig) {
    let snapshot = load_historical_rollup_backfill_snapshot(pool, config)
        .await
        .expect("load snapshot before skipping missing archive");
    assert_eq!(snapshot.legacy_archive_pending, 0);
    let summary = materialize_historical_rollups(pool, config, false)
        .await
        .expect("materialization should skip missing archive file");
    assert_eq!(summary.materialized_invocation_batches, 0);
    let total_count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(pool)
    .await
    .expect("load retained invocation rollup total");
    assert_eq!(
        total_count, 7,
        "skipped materialization must keep prior rollups"
    );
    let prune_summary = prune_legacy_archive_batches(pool, config, false)
        .await
        .expect("prune should retain stale metadata without long-term replay");
    assert_eq!(prune_summary.deleted_archive_batches, 0);
    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(pool)
        .await
        .expect("count remaining archive batches after pruning missing metadata");
    assert_eq!(remaining_batches, 1);
}

async fn assert_retained_live_breakdown(pool: &SqlitePool, archive_path: &Path, bucket_epoch: i64) {
    let retained_rows: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(request_count), 0) FROM upstream_account_usage_breakdown_hourly
         WHERE bucket_start_epoch = ?1 AND upstream_account_id = ?2",
    )
    .bind(bucket_epoch)
    .bind(17_i64)
    .fetch_one(pool)
    .await
    .expect("load retained live breakdown rows after repair");
    assert_eq!(retained_rows, 1);
    let retained_cost: f64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(cost_unknown), 0.0) FROM upstream_account_usage_breakdown_hourly
         WHERE bucket_start_epoch = ?1 AND upstream_account_id = ?2",
    )
    .bind(bucket_epoch)
    .bind(17_i64)
    .fetch_one(pool)
    .await
    .expect("load retained live breakdown cost after repair");
    assert_f64_close(retained_cost, 0.42);
    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches
         WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(archive_path.to_string_lossy().to_string())
    .fetch_one(pool)
    .await
    .expect("load materialized timestamp after repair");
    assert!(materialized_at.is_none());
}

async fn assert_refreshed_node_health_hourly(
    pool: &SqlitePool,
    archive_batch_id: i64,
    binding_key: &str,
    bucket_start_epoch: i64,
) {
    let refreshed = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COALESCE(SUM(success_count), 0), COALESCE(SUM(failure_count), 0)
         FROM pool_upstream_node_health_hourly_archive
         WHERE archive_batch_id = ?1 AND proxy_binding_key_snapshot = ?2 AND bucket_start_epoch = ?3",
    )
    .bind(archive_batch_id)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .fetch_one(pool)
    .await
    .expect("load refreshed hourly cache row");
    assert_eq!(refreshed, (1, 1));
}

#[tokio::test]
pub(crate) async fn bootstrap_hourly_rollups_reopens_same_bucket_partial_usage_breakdown_groups() {
    let (pool, config, temp_dir) = retention_memory_test_pool_and_config(
        "bootstrap-reopens-same-bucket-partial-breakdown-groups",
    )
    .await;
    let (invocation_archive_path, _bucket_epoch) =
        seed_same_bucket_partial_breakdown(&pool, &config).await;

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should reopen same-bucket partial breakdown groups");

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
    .expect("load materialized timestamp after same-bucket repair");
    assert!(materialized_at.is_none());

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
    .expect("count breakdown replay markers after same-bucket repair");
    assert_eq!(breakdown_replay_markers, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn usage_breakdown_repair_preserves_retained_live_rows_in_reopened_archive_bucket()
{
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-preserves-live-bucket").await;
    let live_local = shanghai_local_days_ago(0, 9, 50, 0);
    let coverage_start = shanghai_local_days_ago(0, 9, 5, 0);
    let coverage_end = shanghai_local_days_ago(0, 9, 20, 0);

    insert_retention_invocation(
        &pool,
        "breakdown-repair-retained-live",
        &live_local,
        SOURCE_PROXY,
        "success",
        Some("{\"upstreamAccountId\":17,\"upstreamAccountName\":\"Retained Live\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live breakdown rollup before repair");

    let bucket_epoch = local_naive_to_utc(
        parse_shanghai_local_naive(&live_local).expect("valid live local timestamp"),
        Shanghai,
    )
    .timestamp()
        / 3_600
        * 3_600;
    let seeded_live_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(request_count), 0)
        FROM upstream_account_usage_breakdown_hourly
        WHERE bucket_start_epoch = ?1
          AND upstream_account_id = ?2
        "#,
    )
    .bind(bucket_epoch)
    .bind(17_i64)
    .fetch_one(&pool)
    .await
    .expect("load seeded live breakdown rows");
    assert_eq!(seeded_live_rows, 1);

    let archive_path = temp_dir
        .join("archives")
        .join("codex_invocations")
        .join("partial-retained-live.sqlite.gz");
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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), datetime('now'))
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&coverage_start[..7])
    .bind(archive_path.to_string_lossy().to_string())
    .bind("partial-retained-live-sha")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&coverage_start)
    .bind(&coverage_end)
    .execute(&pool)
    .await
    .expect("seed materialized archive batch overlapping retained live bucket");

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("repair missing usage breakdown replay state");
    assert_eq!(touched, 1);

    assert_retained_live_breakdown(&pool, &archive_path, bucket_epoch).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn historical_rollup_backfill_stays_critical_until_legacy_invocations_materialized()
 {
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
pub(crate) async fn prune_legacy_archive_batches_keeps_detail_prune_backups_within_live_window() {
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
pub(crate) async fn materialize_historical_rollups_skips_missing_archives_and_preserves_existing_rollups()
 {
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

    assert_missing_archive_preserved(&pool, &config).await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn recompute_invocation_hourly_rollups_ignores_archive_manifests_for_live_buckets()
{
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
    let _missing_archive_path =
        archive_batch_file_path(&config, "codex_invocations", &live_occurred_at[..7])
            .expect("resolve missing archive path");
    seed_live_only_archive_manifest(&pool, &config, &live_occurred_at).await;

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

    assert_recomputed_live_rollup(&pool, bucket_start_epoch).await;

    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_live_only_archive_manifest(pool: &SqlitePool, config: &AppConfig, occurred_at: &str) {
    let missing_archive_path =
        archive_batch_file_path(config, "codex_invocations", &occurred_at[..7])
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
    .bind(&occurred_at[..7])
    .bind(missing_archive_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(occurred_at)
    .bind(occurred_at)
    .execute(pool)
    .await
    .expect("insert missing archive manifest for same live month");
}

async fn assert_recomputed_live_rollup(pool: &SqlitePool, bucket_start_epoch: i64) {
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
    .fetch_one(pool)
    .await
    .expect("load post-recompute bucket totals");
    assert_eq!(after.total_count, 1);
    assert_eq!(after.success_count, Some(1));
    assert_eq!(after.failure_count, Some(0));
    assert_eq!(after.total_tokens, 15);
    assert_f64_close(after.total_cost, 1.5);
}

#[tokio::test]
pub(crate) async fn retention_archives_and_cleans_up_pool_upstream_request_attempts() {
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

    assert_pool_attempt_retention_basics(&pool, &config).await;

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

    expire_and_cleanup_pool_attempt_archive(&pool, &config, &archive_batch, &archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

async fn assert_pool_attempt_retention_basics(pool: &SqlitePool, config: &AppConfig) {
    let remaining_old_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE occurred_at < ?1",
    )
    .bind(shanghai_local_cutoff_string(
        config.pool_upstream_request_attempts_retention_days,
    ))
    .fetch_one(pool)
    .await
    .expect("count old pool attempt rows");
    assert_eq!(remaining_old_rows, 0);
    let recent_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id = ?1",
    )
    .bind("retention-pool-attempts-recent")
    .fetch_one(pool)
    .await
    .expect("count recent pool attempt rows");
    assert_eq!(recent_rows, 1);
}

async fn expire_and_cleanup_pool_attempt_archive(
    pool: &SqlitePool,
    config: &AppConfig,
    archive_batch: &(String, Option<String>),
    archive_path: &Path,
) {
    sqlx::query(
        "UPDATE archive_batches SET archive_expires_at = ?1 WHERE dataset = 'pool_upstream_request_attempts'",
    )
    .bind("2000-01-01 00:00:00")
    .execute(pool)
    .await
    .expect("expire archive batch");
    sqlx::query("UPDATE long_term_stats_state SET status = 'empty' WHERE id = 1")
        .execute(pool)
        .await
        .expect("mark long-term stats empty before attempt archive cleanup");
    let archive_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load pool attempt archive checksum");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'pool_upstream_request_attempts', ?2, ?3)",
    )
    .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
    .bind(&archive_batch.0)
    .bind(archive_sha256)
    .execute(pool)
    .await
    .expect("mark pool attempt archive replay complete");
    let cleanup_summary = run_data_retention_maintenance(pool, config, Some(false), None)
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
    .fetch_one(pool)
    .await
    .expect("count remaining pool attempt archive batches");
    assert_eq!(remaining_archive_batches, 0);
}

#[tokio::test]
pub(crate) async fn retention_accumulates_pool_upstream_hourly_history_when_reusing_monthly_batch_file()
 {
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
pub(crate) async fn retention_keeps_preexisting_pool_node_health_month_archives_pending_after_append()
 {
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

    reset_preexisting_node_health_archive(&pool, &archive_path).await;

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

    assert_preexisting_node_health_archive_pending(&pool, &archive_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

async fn reset_preexisting_node_health_archive(pool: &SqlitePool, archive_path: &str) {
    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(pool)
        .await
        .expect("clear cached pool node health archive rows");
    sqlx::query("DELETE FROM pool_upstream_node_health_hourly_archive")
        .execute(pool)
        .await
        .expect("clear cached pool node health hourly archive rows");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE dataset = 'pool_upstream_request_attempts' AND target IN (?1, ?2) AND file_path = ?3",
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .bind(archive_path)
    .execute(pool)
    .await
    .expect("clear pool node health replay markers for existing archive");
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = NULL WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
    )
    .bind(archive_path)
    .execute(pool)
    .await
    .expect("clear historical rollup materialized timestamp");
}

async fn assert_preexisting_node_health_archive_pending(pool: &SqlitePool, archive_path: &str) {
    assert_eq!(
        pending_pool_upstream_node_health_archive_batches(pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        pending_pool_upstream_node_health_hourly_archive_batches(pool)
            .await
            .unwrap(),
        1
    );
    let replay_markers = sqlx::query_as::<_, (String, i64)>(
        "SELECT target, COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1 AND target IN (?2, ?3) GROUP BY target ORDER BY target",
    )
    .bind(archive_path)
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .fetch_all(pool)
    .await
    .expect("load replay markers for existing pool node health archive");
    assert!(
        replay_markers.is_empty(),
        "preexisting month archives should stay pending after append: {replay_markers:?}"
    );
    let materialized_at: Option<String> = sqlx::query_scalar(
        "SELECT historical_rollups_materialized_at FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
    )
    .bind(archive_path)
    .fetch_one(pool)
    .await
    .expect("load historical rollup materialized timestamp after append");
    assert!(materialized_at.is_none());
}

#[tokio::test]
pub(crate) async fn refreshing_pool_node_health_hourly_cache_from_row_cache_is_idempotent() {
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

    seed_hourly_cache_refresh_rows(
        &pool,
        &archive_file_path,
        binding_key,
        archive_batch_id,
        &occurred_at,
        bucket_start_epoch,
    )
    .await;

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

    assert_refreshed_node_health_hourly(&pool, archive_batch_id, binding_key, bucket_start_epoch)
        .await;

    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_hourly_cache_refresh_rows(
    pool: &SqlitePool,
    archive_file_path: &str,
    binding_key: &str,
    archive_batch_id: i64,
    occurred_at: &str,
    bucket_start_epoch: i64,
) {
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
        .bind(archive_file_path)
        .bind(archived_row_id)
        .bind(occurred_at)
        .bind(binding_key)
        .bind(is_success)
        .execute(pool)
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
    .bind(archive_file_path)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .bind(9_i64)
    .bind(4_i64)
    .execute(pool)
    .await
    .expect("insert stale cached hourly row");
}

struct PoolNodeHealthCacheFixture {
    pool: SqlitePool,
    temp_dir: PathBuf,
    archive_file_path: String,
    binding_key: &'static str,
    archive_batch_id: i64,
    bucket_start_epoch: i64,
}

async fn create_pool_node_health_cache_fixture() -> PoolNodeHealthCacheFixture {
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
        "INSERT INTO archive_batches
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, datetime('now'))",
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
        "SELECT id FROM archive_batches
         WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1 LIMIT 1",
    )
    .bind(&archive_file_path)
    .fetch_one(&pool)
    .await
    .expect("load pool upstream archive batch id");
    for (archived_row_id, is_success) in [(1_i64, 1_i64), (2_i64, 0_i64)] {
        sqlx::query(
            "INSERT INTO pool_upstream_node_health_archive
             (archive_file_path, archived_row_id, occurred_at, proxy_binding_key_snapshot,
              is_success, latency_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
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
    PoolNodeHealthCacheFixture {
        pool,
        temp_dir,
        archive_file_path,
        binding_key,
        archive_batch_id,
        bucket_start_epoch,
    }
}

async fn assert_pool_node_health_cache_waits(fixture: &PoolNodeHealthCacheFixture) {
    let first = backfill_pool_upstream_node_health_hourly_archives(&fixture.pool, None, None)
        .await
        .expect("hourly backfill should wait for cache replay");
    assert_eq!(first.materialized_batches, 0);
    assert_eq!(first.pending_batches, 1);
    let hourly_rows_before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_node_health_hourly_archive WHERE archive_batch_id = ?1",
    )
    .bind(fixture.archive_batch_id)
    .fetch_one(&fixture.pool)
    .await
    .expect("count hourly rows before cache replay completes");
    assert_eq!(hourly_rows_before, 0);
}

async fn complete_pool_node_health_cache_replay(fixture: &PoolNodeHealthCacheFixture) {
    let archive_sha256: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches
         WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
    )
    .bind(&fixture.archive_file_path)
    .fetch_one(&fixture.pool)
    .await
    .expect("load pool upstream archive checksum");
    sqlx::query(
        "INSERT INTO hourly_rollup_archive_replay
         (target, dataset, file_path, archive_sha256, replayed_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))",
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind("pool_upstream_request_attempts")
    .bind(&fixture.archive_file_path)
    .bind(&archive_sha256)
    .execute(&fixture.pool)
    .await
    .expect("mark cached pool node health replay complete");
    let second = backfill_pool_upstream_node_health_hourly_archives(&fixture.pool, None, None)
        .await
        .expect("hourly backfill should refresh from cached pool node health rows");
    assert_eq!(second.materialized_batches, 1);
    assert_eq!(second.pending_batches, 0);
}

async fn assert_pool_node_health_cache_refreshed(fixture: &PoolNodeHealthCacheFixture) {
    let refreshed = sqlx::query_as::<_, (i64, i64)>(
        "SELECT COALESCE(SUM(success_count), 0), COALESCE(SUM(failure_count), 0)
         FROM pool_upstream_node_health_hourly_archive
         WHERE archive_batch_id = ?1 AND proxy_binding_key_snapshot = ?2
           AND bucket_start_epoch = ?3",
    )
    .bind(fixture.archive_batch_id)
    .bind(fixture.binding_key)
    .bind(fixture.bucket_start_epoch)
    .fetch_one(&fixture.pool)
    .await
    .expect("load refreshed hourly cache row");
    assert_eq!(refreshed, (1, 1));
    let hourly_replayed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay
         WHERE target = ?1 AND dataset = 'pool_upstream_request_attempts' AND file_path = ?2",
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .bind(&fixture.archive_file_path)
    .fetch_one(&fixture.pool)
    .await
    .expect("count hourly replay markers after cache refresh");
    assert_eq!(hourly_replayed, 1);
}

async fn assert_pool_node_health_cache_reopens_on_sha_change(fixture: &PoolNodeHealthCacheFixture) {
    sqlx::query(
        "UPDATE archive_batches SET sha256 = 'replaced-hourly-cache-replay-gate'
         WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
    )
    .bind(&fixture.archive_file_path)
    .execute(&fixture.pool)
    .await
    .expect("replace pool upstream archive checksum");
    assert_eq!(
        pending_pool_upstream_node_health_archive_batches(&fixture.pool)
            .await
            .expect("load cache archives pending after checksum replacement"),
        1
    );
    assert_eq!(
        pending_pool_upstream_node_health_hourly_archive_batches(&fixture.pool)
            .await
            .expect("load hourly archives pending after checksum replacement"),
        1
    );
}

#[tokio::test]
pub(crate) async fn pool_node_health_hourly_backfill_waits_for_cache_replay_and_refreshes_from_cache()
 {
    let fixture = create_pool_node_health_cache_fixture().await;
    assert_pool_node_health_cache_waits(&fixture).await;
    complete_pool_node_health_cache_replay(&fixture).await;
    assert_pool_node_health_cache_refreshed(&fixture).await;
    assert_pool_node_health_cache_reopens_on_sha_change(&fixture).await;
    cleanup_temp_test_dir(&fixture.temp_dir);
}
async fn create_node_health_replacement_archive(
    pool: &SqlitePool,
    temp_dir: &Path,
    binding_key: &str,
    occurred_at: &str,
) -> (PathBuf, String, String) {
    let archive_path = temp_dir.join("node-health-cache-replacement.sqlite.gz");
    let archive_db_path = temp_dir.join("node-health-cache-replacement.sqlite");
    let archive_file_path = archive_path.to_string_lossy().to_string();
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
            id, invoke_id, occurred_at, endpoint, route_mode,
            proxy_binding_key_snapshot, attempt_index, distinct_account_index,
            same_account_retry_index, finished_at, status, first_byte_latency_ms, created_at
        )
        VALUES (1, 'node-health-cache-replacement-keep', ?1, '/v1/responses', 'pool', ?2, 0, 0, 0, ?1, ?3, 120.0, ?1)
        "#,
    )
    .bind(occurred_at)
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
        "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, created_at) VALUES ('pool_upstream_request_attempts', ?1, ?2, ?3, 1, ?4, ?5, ?5, datetime('now'))",
    )
    .bind(&occurred_at[..7])
    .bind(&archive_file_path)
    .bind(&replacement_sha)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(occurred_at)
    .execute(pool)
    .await
    .expect("publish replacement archive manifest");
    (archive_path, archive_file_path, replacement_sha)
}

async fn seed_stale_node_health_cache_rows(
    pool: &SqlitePool,
    archive_file_path: &str,
    occurred_at: &str,
    binding_key: &str,
) {
    for (archived_row_id, is_success) in [(1_i64, 1_i64), (2_i64, 0_i64)] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_node_health_archive (
                archive_file_path, archived_row_id, occurred_at,
                proxy_binding_key_snapshot, is_success, latency_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, NULL)
            "#,
        )
        .bind(archive_file_path)
        .bind(archived_row_id)
        .bind(occurred_at)
        .bind(binding_key)
        .bind(is_success)
        .execute(pool)
        .await
        .expect("seed stale node health cache row");
    }
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256, replayed_at) VALUES (?1, 'pool_upstream_request_attempts', ?2, 'archive-sha-a', datetime('now'))",
        )
        .bind(target)
        .bind(archive_file_path)
        .execute(pool)
        .await
        .expect("seed stale node health replay marker");
    }
}

pub(crate) async fn node_health_cache_replacement_discards_removed_archive_attempts_before_marking_sha_b()
 {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("node-health-cache-replacement").await;
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

    let (archive_path, archive_file_path, replacement_sha) =
        create_node_health_replacement_archive(&pool, &temp_dir, binding_key, &occurred_at).await;

    seed_stale_node_health_cache_rows(&pool, &archive_file_path, &occurred_at, binding_key).await;

    let cache_summary = backfill_pool_upstream_node_health_archives(&pool, None, None)
        .await
        .expect("replace stale node health cache from archive SHA B");
    assert_eq!(cache_summary.materialized_batches, 1);
    assert_eq!(cache_summary.pending_batches, 0);
    assert_replaced_node_health_cache(&pool, &archive_file_path).await;

    let hourly_summary = backfill_pool_upstream_node_health_hourly_archives(&pool, None, None)
        .await
        .expect("rebuild node health hourly rows from replaced cache");
    assert_eq!(hourly_summary.materialized_batches, 1);
    assert_eq!(hourly_summary.pending_batches, 0);
    assert_replaced_node_health_hourly(&pool, &archive_file_path, &replacement_sha).await;

    assert!(
        archive_path.exists(),
        "replacement archive should remain available"
    );
    cleanup_temp_test_dir(&temp_dir);
}

async fn assert_replaced_node_health_cache(pool: &SqlitePool, archive_file_path: &str) {
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
    .bind(archive_file_path)
    .fetch_one(pool)
    .await
    .expect("load replaced node health cache rows");
    assert_eq!(cached, (1, 1, 0));
}

async fn assert_replaced_node_health_hourly(
    pool: &SqlitePool,
    archive_file_path: &str,
    replacement_sha: &str,
) {
    let hourly = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT COALESCE(SUM(success_count), 0), COALESCE(SUM(failure_count), 0)
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(archive_file_path)
    .fetch_one(pool)
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
        .bind(archive_file_path)
        .fetch_one(pool)
        .await
        .expect("load replacement replay marker");
        assert_eq!(marked_sha, replacement_sha);
    }
}
