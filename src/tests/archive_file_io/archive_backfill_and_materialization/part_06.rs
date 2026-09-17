use super::*;

async fn recreate_legacy_invocation_rollup_table(pool: &SqlitePool) {
    sqlx::query("ALTER TABLE invocation_rollup_hourly RENAME TO invocation_rollup_hourly_current")
        .execute(pool)
        .await
        .expect("rename current invocation rollup table");
    sqlx::query(
        "CREATE TABLE invocation_rollup_hourly (
            bucket_start_epoch INTEGER NOT NULL, source TEXT NOT NULL,
            total_count INTEGER NOT NULL, success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL, total_tokens INTEGER NOT NULL,
            total_cost REAL NOT NULL, first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
            first_byte_sum_ms REAL NOT NULL DEFAULT 0, first_byte_max_ms REAL NOT NULL DEFAULT 0,
            first_byte_histogram TEXT NOT NULL DEFAULT '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (bucket_start_epoch, source)
        )",
    )
    .execute(pool)
    .await
    .expect("create legacy invocation rollup table");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly
         SELECT bucket_start_epoch, source, total_count, success_count, failure_count,
                total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms,
                first_byte_max_ms, first_byte_histogram, updated_at
         FROM invocation_rollup_hourly_current",
    )
    .execute(pool)
    .await
    .expect("copy legacy invocation rollup rows");
    sqlx::query("DROP TABLE invocation_rollup_hourly_current")
        .execute(pool)
        .await
        .expect("drop current invocation rollup table copy");
}

async fn seed_blank_node_health_archive(
    pool: &SqlitePool,
    archive_file_path: &str,
    binding_key: &str,
    occurred_at: &str,
    bucket_start_epoch: i64,
) {
    sqlx::query(
        "INSERT INTO archive_batches
         (dataset, month_key, file_path, sha256, row_count, status,
          coverage_start_at, coverage_end_at, created_at)
         VALUES ('pool_upstream_request_attempts', ?1, ?2, '', 1, ?3, ?4, ?4, datetime('now'))",
    )
    .bind(&occurred_at[..7])
    .bind(archive_file_path)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(occurred_at)
    .execute(pool)
    .await
    .expect("insert blank-manifest pool upstream archive batch");
    let archive_batch_id: i64 = sqlx::query_scalar(
        "SELECT id FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' AND file_path = ?1",
    )
    .bind(archive_file_path)
    .fetch_one(pool)
    .await
    .expect("load blank-manifest archive batch id");
    sqlx::query(
        "INSERT INTO pool_upstream_node_health_archive
         (archive_file_path, archived_row_id, occurred_at, proxy_binding_key_snapshot,
          is_success, latency_ms) VALUES (?1, 1, ?2, ?3, 0, NULL)",
    )
    .bind(archive_file_path)
    .bind(occurred_at)
    .bind(binding_key)
    .execute(pool)
    .await
    .expect("seed node health cache protected by blank manifest");
    sqlx::query(
        "INSERT INTO pool_upstream_node_health_hourly_archive
         (archive_identity, archive_batch_id, archive_file_path, proxy_binding_key_snapshot,
          bucket_start_epoch, success_count, failure_count)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, 1)",
    )
    .bind(pool_upstream_node_health_archive_identity_for_batch_id(
        archive_batch_id,
    ))
    .bind(archive_batch_id)
    .bind(archive_file_path)
    .bind(binding_key)
    .bind(bucket_start_epoch)
    .execute(pool)
    .await
    .expect("seed node health hourly cache protected by blank manifest");
    for target in [
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay
             (target, dataset, file_path, archive_sha256)
             VALUES (?1, 'pool_upstream_request_attempts', ?2, '')",
        )
        .bind(target)
        .bind(archive_file_path)
        .execute(pool)
        .await
        .expect("seed blank node health replay marker");
    }
}

#[tokio::test]
pub(crate) async fn node_health_archives_with_blank_manifest_sha_stay_quarantined() {
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

    seed_blank_node_health_archive(
        &pool,
        &archive_file_path,
        binding_key,
        &occurred_at,
        bucket_start_epoch,
    )
    .await;

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
pub(crate) async fn missing_pool_node_health_archives_clear_stale_cached_rows_before_marking_replayed()
 {
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

    assert_missing_node_health_archive_replayed(&pool, &archive_file_path).await;

    cleanup_temp_test_dir(&temp_dir);
}

async fn assert_missing_node_health_archive_replayed(pool: &SqlitePool, archive_file_path: &str) {
    let summary = backfill_pool_upstream_node_health_archives(pool, None, None)
        .await
        .expect("backfill should clear stale cached rows for missing archives");
    assert_eq!(summary.pending_batches, 0);
    let cached_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_node_health_archive WHERE archive_file_path = ?1",
    )
    .bind(archive_file_path)
    .fetch_one(pool)
    .await
    .expect("count cached rows after missing archive replay");
    assert_eq!(cached_rows, 0);
    let replayed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'pool_upstream_request_attempts' AND file_path = ?2",
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(archive_file_path)
    .fetch_one(pool)
    .await
    .expect("count replay marker for missing archive");
    assert_eq!(replayed, 1);
}

#[tokio::test]
pub(crate) async fn cleanup_expired_pool_upstream_archives_waits_for_cache_replay_completion() {
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
pub(crate) async fn cleanup_expired_pool_upstream_archives_preserves_recent_exact_window_history() {
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
pub(crate) async fn pool_upstream_node_health_archive_backfill_reuses_stable_temp_db_when_budget_is_hit()
 {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-archive-temp-reuse").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 30;
    let row_count = (BACKFILL_BATCH_SIZE as usize * 6) + 17;
    config.retention_batch_rows = row_count;

    let old_occurred_at = shanghai_local_days_ago(10, 9, 30, 0);
    seed_temp_reuse_attempts(&pool, &old_occurred_at, row_count).await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run pool attempt retention");
    assert_eq!(
        summary.pool_upstream_request_attempt_rows_archived,
        row_count
    );

    let temp_path = prepare_node_health_temp_replay(&pool).await;

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

async fn seed_temp_reuse_attempts(pool: &SqlitePool, old_occurred_at: &str, row_count: usize) {
    let old_base = parse_shanghai_local_naive(old_occurred_at).expect("valid shanghai local");
    for idx in 0..row_count {
        let occurred_at = format_naive(old_base + ChronoDuration::seconds(idx as i64));
        insert_retention_pool_upstream_request_attempt(
            pool,
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
}

async fn prepare_node_health_temp_replay(pool: &SqlitePool) -> PathBuf {
    let archive_path = PathBuf::from(
        sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .expect("load pool attempt archive batch path"),
    );
    let temp_path = pool_upstream_node_health_archive_temp_path(&archive_path);
    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(pool)
        .await
        .expect("clear cached node health archive rows");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'pool_upstream_request_attempts'",
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .execute(pool)
    .await
    .expect("clear node health archive replay markers");
    sqlx::query("DELETE FROM hourly_rollup_archive_progress WHERE dataset = 'pool_upstream_request_attempts'")
        .execute(pool)
        .await
        .expect("clear node health archive replay progress");
    temp_path
}

#[tokio::test]
pub(crate) async fn pool_upstream_node_health_archive_backfill_marks_missing_archives_replayed() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-missing-archive").await;
    let missing_occurred_at = shanghai_local_days_ago(45, 9, 0, 0);
    let month_key = missing_occurred_at[..7].to_string();
    let missing_archive_path =
        archive_batch_file_path(&config, "pool_upstream_request_attempts", &month_key)
            .expect("resolve missing pool node health archive path");

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
        .expect("missing pool node health archive should be marked replayed");
    assert!(!summary.hit_budget);
    assert_eq!(summary.pending_batches, 0);

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
    assert_eq!(replay_marked, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn pool_upstream_node_health_archive_backfill_refreshes_stale_temp_after_archive_rewrite()
 {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("pool-node-health-archive-refresh-temp").await;
    config.pool_upstream_request_attempts_retention_days = 7;
    config.pool_upstream_request_attempts_archive_ttl_days = 30;
    let row_count = (BACKFILL_BATCH_SIZE as usize * 6) + 17;
    config.retention_batch_rows = row_count;
    let binding_key = "stale-temp-rewrite-binding";

    let old_occurred_at = shanghai_local_days_ago(10, 11, 0, 0);
    let old_base = parse_shanghai_local_naive(&old_occurred_at).expect("valid shanghai local");
    seed_refresh_temp_attempts(&pool, old_base, row_count, binding_key).await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run initial pool attempt retention");
    assert_eq!(
        summary.pool_upstream_request_attempt_rows_archived,
        row_count
    );

    let (archive_path, temp_path) = prepare_stale_temp_replay(&pool).await;

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

    assert_rewritten_node_health_archive_replayed(&pool, &temp_path, row_count).await;

    cleanup_temp_test_dir(&temp_dir);
}

async fn assert_rewritten_node_health_archive_replayed(
    pool: &SqlitePool,
    temp_path: &Path,
    row_count: usize,
) {
    let cached_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_node_health_archive")
            .fetch_one(pool)
            .await
            .expect("count cached node health archive rows after archive rewrite");
    assert_eq!(
        cached_rows,
        (row_count + 1) as i64,
        "rewritten month archives must invalidate stale temp sqlite copies so appended rows are replayed"
    );
    assert!(
        !temp_path.exists(),
        "completed replay should clean up the temp sqlite"
    );
}

async fn seed_refresh_temp_attempts(
    pool: &SqlitePool,
    old_base: NaiveDateTime,
    row_count: usize,
    binding_key: &str,
) {
    for idx in 0..row_count {
        let occurred_at = Shanghai
            .from_local_datetime(&(old_base + ChronoDuration::seconds(idx as i64)))
            .single()
            .expect("valid shanghai local old pool attempt")
            .with_timezone(&Utc);
        seed_pool_upstream_attempt_at(
            pool,
            &format!("pool-node-health-archive-refresh-temp-old-{idx}"),
            occurred_at,
            Some(binding_key),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        )
        .await;
    }
}

async fn prepare_stale_temp_replay(pool: &SqlitePool) -> (PathBuf, PathBuf) {
    let archive_path = PathBuf::from(
        sqlx::query_scalar::<_, String>(
            "SELECT file_path FROM archive_batches WHERE dataset = 'pool_upstream_request_attempts' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .expect("load rewritten pool attempt archive batch path"),
    );
    let temp_path = pool_upstream_node_health_archive_temp_path(&archive_path);
    sqlx::query("DELETE FROM pool_upstream_node_health_archive")
        .execute(pool)
        .await
        .expect("clear cached node health archive rows");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'pool_upstream_request_attempts'",
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .execute(pool)
    .await
    .expect("clear node health archive replay markers");
    sqlx::query("DELETE FROM hourly_rollup_archive_progress WHERE dataset = 'pool_upstream_request_attempts'")
        .execute(pool)
        .await
        .expect("clear node health archive replay progress");
    (archive_path, temp_path)
}

#[test]
pub(crate) fn cleanup_stale_archive_temp_files_removes_only_old_archive_residue() {
    let temp_dir = make_temp_test_dir("archive-temp-janitor");
    let mut config = test_config();
    config.archive_dir = temp_dir.join("archives");
    let archive_root = resolved_archive_dir(&config);
    let nested_dir = archive_root.join("codex_invocations/2026/03/25");
    fs::create_dir_all(&nested_dir).expect("create nested archive dir");

    let stale_temp = nested_dir.join("part-000001.sqlite.gz.1.partial.sqlite");
    let fresh_temp = nested_dir.join("part-000002.sqlite.gz.1.partial.sqlite");
    let official = nested_dir.join("part-000003.sqlite.gz");
    fs::write(&stale_temp, b"stale temp").expect("write stale temp");
    fs::write(&fresh_temp, b"fresh temp").expect("write fresh temp");
    fs::write(&official, b"official archive").expect("write official archive");
    set_file_mtime_seconds_ago(&stale_temp, DEFAULT_ARCHIVE_TEMP_MIN_AGE_SECS + 60);
    set_file_mtime_seconds_ago(&fresh_temp, 60);

    let summary = cleanup_stale_archive_temp_files(&config, false).expect("run archive janitor");
    assert_eq!(summary.stale_temp_files_removed, 1);
    assert!(summary.stale_temp_bytes_removed > 0);
    assert!(!stale_temp.exists(), "stale temp should be deleted");
    assert!(fresh_temp.exists(), "fresh temp should be kept");
    assert!(official.exists(), "official archive should be kept");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn verify_archive_storage_reports_missing_orphan_and_temp_files() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("verify-archive-storage").await;
    config.archive_dir = temp_dir.join("archives");
    let archive_root = resolved_archive_dir(&config);
    fs::create_dir_all(&archive_root).expect("create archive root");

    let day_key = "2025-01-01";
    let missing_path = archive_segment_file_path(
        &config,
        "codex_invocations",
        day_key,
        "part-000001",
        ArchiveFileCodec::Gzip,
    )
    .expect("resolve missing segment path");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, day_key, part_key, file_path, sha256, row_count, status, layout, codec, writer_version, cleanup_state, coverage_start_at, coverage_end_at, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind("2025-01")
    .bind(day_key)
    .bind("part-000001")
    .bind(missing_path.to_string_lossy().to_string())
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
    .bind(ARCHIVE_FILE_CODEC_GZIP)
    .bind(ARCHIVE_WRITER_VERSION_SEGMENT_V1)
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .bind("2025-01-01 00:00:00")
    .bind("2025-01-01 00:00:00")
    .execute(&pool)
    .await
    .expect("insert missing segment manifest");

    let orphan_path = archive_root.join("codex_invocations/2025/01/01/orphan.sqlite.gz");
    fs::create_dir_all(orphan_path.parent().expect("orphan parent")).expect("create orphan parent");
    fs::write(&orphan_path, b"orphan archive").expect("write orphan archive");
    let stale_temp =
        archive_root.join("codex_invocations/2025/01/01/part-000009.sqlite.gz.1.partial.sqlite");
    fs::write(&stale_temp, b"stale temp").expect("write stale temp");
    set_file_mtime_seconds_ago(&stale_temp, DEFAULT_ARCHIVE_TEMP_MIN_AGE_SECS + 60);

    let summary = verify_archive_storage(&pool, &config)
        .await
        .expect("verify archive storage");
    assert_eq!(summary.manifest_rows, 1);
    assert_eq!(summary.missing_files, 1);
    assert_eq!(summary.orphan_files, 1);
    assert_eq!(summary.stale_temp_files, 1);
    assert!(summary.stale_temp_bytes > 0);

    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_prune_segment(pool: &SqlitePool, config: &AppConfig) -> (PathBuf, String) {
    let segment_path = archive_segment_file_path(
        config,
        "codex_invocations",
        "2025-01-02",
        "part-000001",
        ArchiveFileCodec::Gzip,
    )
    .expect("resolve segment path");
    fs::create_dir_all(segment_path.parent().expect("segment parent"))
        .expect("create segment parent");
    let source_path = seed_invocation_archive_batch(
        pool,
        config,
        "prune-segment-source",
        &[(
            1_i64,
            "prune-segment-source",
            "2025-01-02 09:00:00",
            SOURCE_PROXY,
            "success",
            42_i64,
            0.42_f64,
            Some(120.0),
        )],
    )
    .await;
    fs::rename(&source_path, &segment_path).expect("move segment archive into fixture path");
    let sha256 = sha256_hex_file(&segment_path).expect("load segment archive checksum");
    sqlx::query(
        "UPDATE archive_batches SET day_key = '2025-01-02', part_key = 'part-000001', file_path = ?1, sha256 = ?2, layout = ?3, codec = ?4, writer_version = ?5, cleanup_state = ?6, coverage_start_at = '2025-01-02 09:00:00', coverage_end_at = '2025-01-02 09:00:00', archive_expires_at = '2000-01-01 00:00:00', historical_rollups_materialized_at = datetime('now') WHERE file_path = ?7",
    )
    .bind(segment_path.to_string_lossy().to_string())
    .bind(&sha256)
    .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
    .bind(ARCHIVE_FILE_CODEC_GZIP)
    .bind(ARCHIVE_WRITER_VERSION_SEGMENT_V1)
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .bind(source_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("adapt expired segment manifest to fixture path");
    (segment_path, sha256)
}

async fn seed_prune_legacy(pool: &SqlitePool, config: &AppConfig) -> (PathBuf, String) {
    let legacy_path = archive_batch_file_path(config, "codex_invocations", "2024-12")
        .expect("resolve legacy batch path");
    fs::create_dir_all(legacy_path.parent().expect("legacy parent")).expect("create legacy parent");
    let source_path = seed_invocation_archive_batch(
        pool,
        config,
        "prune-legacy-source",
        &[(
            2_i64,
            "prune-legacy-source",
            "2024-12-01 09:00:00",
            SOURCE_PROXY,
            "success",
            42_i64,
            0.42_f64,
            Some(120.0),
        )],
    )
    .await;
    fs::rename(&source_path, &legacy_path).expect("move legacy archive into fixture path");
    let sha256 = sha256_hex_file(&legacy_path).expect("load legacy archive checksum");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, sha256 = ?2, layout = ?3, codec = ?4, writer_version = ?5, cleanup_state = ?6, coverage_start_at = '2024-12-01 09:00:00', coverage_end_at = '2024-12-01 09:00:00', historical_rollups_materialized_at = datetime('now') WHERE file_path = ?7",
    )
    .bind(legacy_path.to_string_lossy().to_string())
    .bind(&sha256)
    .bind(ARCHIVE_LAYOUT_LEGACY_MONTH)
    .bind(ARCHIVE_FILE_CODEC_GZIP)
    .bind(ARCHIVE_WRITER_VERSION_LEGACY_MONTH_V1)
    .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
    .bind(source_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("adapt legacy archive manifest to fixture path");
    (legacy_path, sha256)
}

async fn seed_prune_replay_proofs(
    pool: &SqlitePool,
    segment_path: &Path,
    segment_sha256: &str,
    legacy_path: &Path,
    legacy_sha256: &str,
) {
    for (archive_path, archive_sha256) in [
        (
            segment_path.to_string_lossy().to_string(),
            segment_sha256.to_owned(),
        ),
        (
            legacy_path.to_string_lossy().to_string(),
            legacy_sha256.to_owned(),
        ),
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, ?3)",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(archive_path)
        .bind(archive_sha256)
        .execute(pool)
        .await
        .expect("mark legacy prune archive replay complete");
    }
    for (archive_path, archive_sha256, coverage) in [
        (
            segment_path.to_string_lossy().to_string(),
            segment_sha256,
            ("2025-01-02 09:00:00", "2025-01-02 09:00:00"),
        ),
        (
            legacy_path.to_string_lossy().to_string(),
            legacy_sha256,
            ("2024-12-01 09:00:00", "2024-12-01 09:00:00"),
        ),
    ] {
        let archive_batch_id: i64 = sqlx::query_scalar(
            "SELECT id FROM archive_batches WHERE file_path = ?1 AND sha256 = ?2",
        )
        .bind(&archive_path)
        .bind(archive_sha256)
        .fetch_one(pool)
        .await
        .expect("load prune archive batch id for Summary Snapshot proof");
        super::part_01::insert_summary_archive_snapshot_proof(
            pool,
            archive_batch_id,
            archive_sha256,
            coverage.0,
            coverage.1,
            1,
        )
        .await;
    }
}

#[tokio::test]
pub(crate) async fn prune_archive_batches_removes_expired_segments_and_legacy_batches() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("prune-archive-batches").await;
    config.archive_dir = temp_dir.join("archives");
    let archive_root = resolved_archive_dir(&config);
    fs::create_dir_all(&archive_root).expect("create archive root");

    let (segment_path, segment_sha256) = seed_prune_segment(&pool, &config).await;
    sqlx::query("UPDATE long_term_stats_state SET status = 'empty' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("mark long-term stats as ready for legacy prune fixture");

    let (legacy_path, legacy_sha256) = seed_prune_legacy(&pool, &config).await;
    seed_prune_replay_proofs(
        &pool,
        &segment_path,
        &segment_sha256,
        &legacy_path,
        &legacy_sha256,
    )
    .await;

    let summary = prune_archive_batches(&pool, &config, false)
        .await
        .expect("prune archive batches");
    assert_eq!(
        summary.expired_archive_batches_deleted + summary.legacy_archive_batches_deleted,
        2
    );
    assert!(!segment_path.exists(), "expired segment should be removed");
    assert!(!legacy_path.exists(), "legacy archive should be removed");

    let remaining_batches: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count remaining archive batches");
    assert_eq!(remaining_batches, 0);

    cleanup_temp_test_dir(&temp_dir);
}

async fn seed_retention_rollup_guards(pool: &SqlitePool, config: &AppConfig) -> String {
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);
    let bucket_start = local_naive_to_utc(
        parse_shanghai_local_naive(&old_invocation).expect("valid old invocation time"),
        Shanghai,
    );
    insert_invocation_hourly_rollup_bucket(pool, bucket_start, SOURCE_PROXY, 1, 1, 0, 42, 0.42)
        .await;
    sqlx::query(
        "INSERT INTO forward_proxy_attempt_hourly (proxy_key, bucket_start_epoch, attempts, success_count, failure_count, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
    )
    .bind("proxy-retention-accounted")
    .bind(bucket_start.timestamp())
    .bind(1_i64)
    .bind(1_i64)
    .bind(0_i64)
    .execute(pool)
    .await
    .expect("seed materialized forward proxy hourly rollup");
    for table in [
        "upstream_account_stats_hourly",
        "upstream_account_stats_minute",
    ] {
        let statement = format!(
            "INSERT INTO {table} (bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))"
        );
        sqlx::query(&statement)
            .bind(bucket_start.timestamp())
            .bind(SOURCE_PROXY)
            .bind(17_i64)
            .bind(1_i64)
            .bind(1_i64)
            .bind(0_i64)
            .execute(pool)
            .await
            .expect("seed account stats rollup guard");
    }
    old_invocation
}

async fn seed_retention_materialized_archives(
    pool: &SqlitePool,
    temp_dir: &Path,
    old_invocation: &str,
) {
    for (dataset, file_name) in [
        ("codex_invocations", "materialized-invocation.sqlite.gz"),
        (
            "forward_proxy_attempts",
            "materialized-forward-proxy.sqlite.gz",
        ),
    ] {
        sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, historical_rollups_materialized_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'), datetime('now'))",
        )
        .bind(dataset)
        .bind(&old_invocation[..7])
        .bind(temp_dir.join(file_name).to_string_lossy().to_string())
        .bind(format!("{dataset}-sha"))
        .bind(1_i64)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(old_invocation)
        .bind(old_invocation)
        .execute(pool)
        .await
        .expect("seed materialized archive batch");
    }
}

#[tokio::test]
pub(crate) async fn bootstrap_hourly_rollups_keeps_retention_materialized_totals_unchanged_while_reopening_missing_breakdown_backfill()
 {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("hourly-rollup-retention-accounted").await;
    let old_invocation = seed_retention_rollup_guards(&pool, &config).await;

    let invocation_total_before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly totals before retention");
    let forward_proxy_total_before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(attempts), 0) FROM forward_proxy_attempt_hourly WHERE proxy_key = ?1",
    )
    .bind("proxy-retention-accounted")
    .fetch_one(&pool)
    .await
    .expect("load forward proxy hourly totals before retention");

    seed_retention_materialized_archives(&pool, &temp_dir, &old_invocation).await;

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("replay hourly rollups after retention");
    let invocation_total_after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation hourly totals after bootstrap");
    let forward_proxy_total_after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(attempts), 0) FROM forward_proxy_attempt_hourly WHERE proxy_key = ?1",
    )
    .bind("proxy-retention-accounted")
    .fetch_one(&pool)
    .await
    .expect("load forward proxy hourly totals after bootstrap");

    assert_eq!(invocation_total_before, 1);
    assert_eq!(invocation_total_after, invocation_total_before);
    assert_eq!(forward_proxy_total_before, 1);
    assert_eq!(forward_proxy_total_after, forward_proxy_total_before);

    let invocation_materialized_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'codex_invocations' AND historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count materialized invocation archive batches");
    assert_eq!(invocation_materialized_batches, 0);

    let forward_proxy_materialized_batches: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'forward_proxy_attempts' AND historical_rollups_materialized_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count materialized forward proxy archive batches");
    assert_eq!(forward_proxy_materialized_batches, 1);

    let replay_marker_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM hourly_rollup_archive_replay")
            .fetch_one(&pool)
            .await
            .expect("count hourly rollup archive replay markers");
    assert_eq!(replay_marker_count, 0);

    let historical_backfill_snapshot = load_historical_rollup_backfill_snapshot(&pool, &config)
        .await
        .expect("load backlog after reopening missing breakdown backfill");
    assert_eq!(historical_backfill_snapshot.legacy_archive_pending, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn bootstrap_hourly_rollups_ignores_missing_replay_markers() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("hourly-rollup-missing-invocation-target").await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);
    let payload = r#"{"endpoint":"/v1/responses","promptCacheKey":"cache-replay","upstreamAccountId":17,"upstreamAccountName":"Replay Account","stickyKey":"sticky-replay"}"#;
    insert_retention_invocation(
        &pool,
        "hourly-rollup-missing-invocation-target",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some(payload),
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
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention before bootstrap replay");
    assert_eq!(summary.invocation_rows_archived, 1);

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap hourly rollups after retention");

    let archive_path: String = sqlx::query_scalar(
        "SELECT file_path FROM archive_batches WHERE dataset = 'codex_invocations' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load archived codex_invocations batch path");
    let invocation_total_before: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation totals before marker repair");

    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND target = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS)
    .bind(&archive_path)
    .execute(&pool)
    .await
    .expect("delete one invocation replay marker");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("bootstrap should replay only the missing target");

    let invocation_total_after: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load invocation totals after marker repair");
    let repaired_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND target = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS)
    .bind(&archive_path)
    .fetch_one(&pool)
    .await
    .expect("load repaired replay marker count");

    assert_eq!(invocation_total_before, 1);
    assert_eq!(invocation_total_after, invocation_total_before);
    assert_eq!(repaired_marker_count, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn ensure_schema_backfills_legacy_invocation_rollup_aggregate_columns() {
    let (pool, config, temp_dir) = retention_fresh_schema_test_pool_and_config(
        "legacy-rollup-first-response-byte-total-backfill",
    )
    .await;
    let old_invocation = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "legacy-rollup-first-response-byte-total-backfill",
        &old_invocation,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET t_req_read_ms = ?1,
            t_req_parse_ms = ?2,
            t_upstream_connect_ms = ?3,
            t_upstream_ttfb_ms = ?4
        WHERE invoke_id = ?5
        "#,
    )
    .bind(120.0_f64)
    .bind(80.0_f64)
    .bind(43_000.0_f64)
    .bind(690.0_f64)
    .bind("legacy-rollup-first-response-byte-total-backfill")
    .execute(&pool)
    .await
    .expect("seed staged latency fields");

    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollups before retention");
    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("archive old invocation before schema migration");
    assert_eq!(summary.invocation_rows_archived, 1);

    recreate_legacy_invocation_rollup_table(&pool).await;

    ensure_schema(&pool)
        .await
        .expect("ensure schema should backfill first-response-byte totals");

    let row = sqlx::query_as::<_, (i64, f64, f64, String)>(
        r#"
        SELECT
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram
        FROM invocation_rollup_hourly
        WHERE source = ?1
        LIMIT 1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load backfilled invocation rollup row");

    assert_eq!(
        row.0, 1,
        "legacy rollup row should gain one first-response sample"
    );
    assert_eq!(row.1, 43_890.0);
    assert_eq!(row.2, 43_890.0);
    assert_ne!(
        row.3, "[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]",
        "backfill should write a non-empty histogram"
    );

    let terminal = sqlx::query_as::<_, (i64, i64, f64, i64)>(
        r#"
        SELECT terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete
        FROM invocation_rollup_hourly
        WHERE source = ?1
        LIMIT 1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load terminal proof backfill");
    assert_eq!(terminal.0, 1);
    assert_eq!(terminal.1, 42);
    assert_eq!(terminal.2, 0.42);
    assert_eq!(terminal.3, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn ensure_schema_reconciles_legacy_rollups_when_sources_are_complete() {
    let (pool, _config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("legacy-rollup-terminal-proof-partial-source")
            .await;
    let occurred_at = shanghai_local_days_ago(3, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "legacy-rollup-terminal-proof-partial-source",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(42),
        Some(0.42),
    )
    .await;
    sync_hourly_rollups_from_live_tables(&pool)
        .await
        .expect("seed live hourly rollup");
    sqlx::query(
        "UPDATE invocation_rollup_hourly SET total_count = 2, total_tokens = 84, total_cost = 0.84 WHERE source = ?1",
    )
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("seed canonical totals that cannot be reconstructed from the remaining source");

    recreate_legacy_invocation_rollup_table(&pool).await;

    ensure_schema(&pool)
        .await
        .expect("ensure schema should reconcile a legacy canonical row against complete sources");

    let row = sqlx::query_as::<_, (i64, i64, f64, i64)>(
        r#"
        SELECT total_count, total_tokens, total_cost, terminal_proof_complete
        FROM invocation_rollup_hourly
        WHERE source = ?1
        LIMIT 1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load reconciled legacy rollup");
    assert_eq!(row.0, 1);
    assert_eq!(row.1, 42);
    assert_eq!(row.2, 0.42);
    assert_eq!(
        row.3, 1,
        "complete sources should restore the terminal integrity proof after replacing legacy totals"
    );

    cleanup_temp_test_dir(&temp_dir);
}
