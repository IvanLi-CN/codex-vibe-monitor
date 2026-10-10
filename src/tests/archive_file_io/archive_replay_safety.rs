use super::archive_backfill_and_materialization::{
    budgeted_archive_io_test_dir, write_valid_invocation_archive,
};
use super::*;

#[tokio::test]
async fn invocation_archive_validation_preserves_reusable_copy_at_open_budget_boundary() {
    let temp_dir = budgeted_archive_io_test_dir("invocation-archive-open-budget-reuse");
    let archive_path = temp_dir.join("archive.sqlite.gz");
    write_valid_invocation_archive(&archive_path, "open-budget-reuse").await;
    let manifest_sha = sha256_hex_file(&archive_path).expect("hash invocation archive");
    let temp_path = invocation_archive_replay_temp_path(&archive_path);
    let archive_pool =
        open_historical_rollup_archive_pool(&archive_path, &temp_path, &manifest_sha)
            .await
            .expect("prepare complete reusable archive copy");
    archive_pool.close().await;
    let original_copy = fs::read(&temp_path).expect("read completed archive copy");
    let result = HISTORICAL_ROLLUP_TEST_SKIP_ARCHIVE_OPEN
        .scope(
            (),
            invocation_archive_file_is_readable_with_budget(
                &archive_path,
                &manifest_sha,
                Instant::now(),
                None,
            ),
        )
        .await;
    assert_eq!(result, InvocationArchiveReadability::BudgetExhausted);
    assert_eq!(
        fs::read(&temp_path).expect("retained archive copy"),
        original_copy
    );
    assert_eq!(
        load_historical_rollup_temp_source_signature(&temp_path),
        Some(manifest_sha.clone())
    );
    assert_eq!(
        invocation_archive_file_is_readable_with_budget(
            &archive_path,
            &manifest_sha,
            Instant::now(),
            None,
        )
        .await,
        InvocationArchiveReadability::Readable
    );
    assert!(!temp_path.exists());
    assert!(!temp_sqlite_source_meta_path(&temp_path).exists());
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn rejected_invocation_manifest_sha_cleans_stale_temp_and_sidecar() {
    let temp_dir = budgeted_archive_io_test_dir("invocation-archive-rejected-cleanup");
    let archive_path = temp_dir.join("archive.sqlite.gz");
    write_valid_invocation_archive(&archive_path, "rejected-stale-temp").await;
    let temp_path = invocation_archive_replay_temp_path(&archive_path);
    let meta_path = temp_sqlite_source_meta_path(&temp_path);
    fs::write(&temp_path, b"stale temp copy").expect("write stale temp copy");
    fs::write(&meta_path, "stale-manifest-sha").expect("write stale temp sidecar");

    assert_eq!(
        invocation_archive_file_is_readable_with_budget(
            &archive_path,
            "rejected-manifest-sha",
            Instant::now(),
            None,
        )
        .await,
        InvocationArchiveReadability::Rejected
    );
    assert!(!temp_path.exists());
    assert!(!meta_path.exists());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn forward_proxy_archive_replay_materializes_a_real_gzip_sqlite_fixture() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("forward-proxy-real-archive-replay").await;
    let archive_db_path = temp_dir.join("forward-proxy.sqlite");
    let archive_path = temp_dir.join("forward-proxy.sqlite.gz");
    fs::File::create(&archive_db_path).expect("create forward proxy archive sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open forward proxy archive sqlite");
    sqlx::query(
        r#"
        CREATE TABLE forward_proxy_attempts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            proxy_key TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            is_success INTEGER NOT NULL,
            latency_ms REAL,
            failure_kind TEXT,
            is_probe INTEGER NOT NULL DEFAULT 0
        )
        "#,
    )
    .execute(&archive_pool)
    .await
    .expect("create forward proxy archive schema");
    sqlx::query(
        "INSERT INTO forward_proxy_attempts (proxy_key, occurred_at, is_success, latency_ms) \
         VALUES ('fixture-proxy', '2026-01-15 08:15:00', 1, 125.5)",
    )
    .execute(&archive_pool)
    .await
    .expect("insert forward proxy archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress forward proxy archive fixture");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash forward proxy archive");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status,
            coverage_start_at, coverage_end_at
        )
        VALUES ('forward_proxy_attempts', '2026-01', ?1, ?2, 1, 'completed', ?3, ?3)
        "#,
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .bind("2026-01-15 08:15:00")
    .execute(&pool)
    .await
    .expect("seed forward proxy archive manifest");

    let temp_path = forward_proxy_archive_replay_temp_path(&archive_path);
    let archive_pool =
        open_historical_rollup_archive_pool(&archive_path, &temp_path, &archive_sha256)
            .await
            .expect("prepare forward proxy reusable copy");
    archive_pool.close().await;
    let mut tx = pool
        .begin()
        .await
        .expect("begin deferred forward proxy replay");
    let deferred = HISTORICAL_ROLLUP_TEST_SKIP_ARCHIVE_OPEN
        .scope(
            (),
            replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
                tx.as_mut(),
                Instant::now(),
                None,
                None,
                0,
            ),
        )
        .await
        .expect("defer replay between selection and archive open");
    tx.commit()
        .await
        .expect("commit unstarted forward proxy replay");
    assert!(deferred.hit_budget);
    assert_eq!(deferred.materialized_batches, 0);
    assert!(temp_path.is_file());
    assert_eq!(
        load_historical_rollup_temp_source_signature(&temp_path),
        Some(archive_sha256.clone())
    );

    let mut tx = pool.begin().await.expect("begin forward proxy replay");
    let summary = replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
        tx.as_mut(),
        Instant::now(),
        None,
        None,
        0,
    )
    .await
    .expect("replay forward proxy archive fixture");
    tx.commit().await.expect("commit forward proxy replay");

    assert_eq!(summary.materialized_batches, 1);
    let attempts: i64 = sqlx::query_scalar(
        "SELECT attempts FROM forward_proxy_attempt_hourly WHERE proxy_key = 'fixture-proxy'",
    )
    .fetch_one(&pool)
    .await
    .expect("load replayed forward proxy rollup");
    assert_eq!(attempts, 1);
    assert!(!temp_path.exists());
    assert!(!temp_sqlite_source_meta_path(&temp_path).exists());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn repair_materialized_breakdown_reopens_overlapping_replayed_batches() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("breakdown-repair-overlap").await;
    let bucket_start_epoch =
        invocation_bucket_start_epoch("2026-07-01 15:05:00").expect("derive bucket start epoch");
    let first_archive_path = temp_dir.join("usage-breakdown-overlap-first.sqlite.gz");
    let second_archive_path = temp_dir.join("usage-breakdown-overlap-second.sqlite.gz");
    write_valid_invocation_archive(&first_archive_path, "usage-breakdown-overlap-first").await;
    write_valid_invocation_archive(&second_archive_path, "usage-breakdown-overlap-second").await;
    let first_file_path = first_archive_path.to_string_lossy().into_owned();
    let second_file_path = second_archive_path.to_string_lossy().into_owned();
    let first_sha256 = sha256_hex_file(Path::new(&first_file_path)).expect("hash first archive");
    let second_sha256 = sha256_hex_file(Path::new(&second_file_path)).expect("hash second archive");

    for (file_path, coverage_start_at, coverage_end_at, replayed, cursor_id, sha256) in [
        (
            first_file_path.as_str(),
            "2026-07-01 15:05:00",
            "2026-07-01 15:15:00",
            false,
            101_i64,
            first_sha256.as_str(),
        ),
        (
            second_file_path.as_str(),
            "2026-07-01 15:25:00",
            "2026-07-01 15:35:00",
            true,
            202_i64,
            second_sha256.as_str(),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                status,
                sha256,
                row_count,
                coverage_start_at,
                coverage_end_at,
                historical_rollups_materialized_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind("2026-07")
        .bind(file_path)
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(sha256)
        .bind(1_i64)
        .bind(coverage_start_at)
        .bind(coverage_end_at)
        .execute(&pool)
        .await
        .expect("insert archive batch");

        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id, updated_at)
            VALUES (?1, ?2, ?3, datetime('now'))
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .bind(cursor_id)
        .execute(&pool)
        .await
        .expect("insert archive progress");

        if replayed {
            sqlx::query(
                r#"
                INSERT INTO hourly_rollup_archive_replay (
                    target, dataset, file_path, archive_sha256, replayed_at
                )
                VALUES (?1, ?2, ?3, ?4, datetime('now'))
                "#,
            )
            .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .bind(file_path)
            .bind(sha256)
            .execute(&pool)
            .await
            .expect("insert replay marker");
        }
    }

    for (upstream_account_key, upstream_account_id, normalized_model) in [
        ("upstream:17", Some(17_i64), "gpt-5"),
        ("upstream:18", Some(18_i64), "gpt-5-mini"),
    ] {
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
                failure_count
            )
            VALUES (?1, ?2, ?3, ?4, ?5, '', 1, 1, 0)
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(SOURCE_PROXY)
        .bind(upstream_account_key)
        .bind(upstream_account_id)
        .bind(normalized_model)
        .execute(&pool)
        .await
        .expect("seed breakdown rollup row");
    }

    #[cfg(unix)]
    {
        let publisher_fence = retention_archive_file_lock(&first_archive_path)
            .expect("hold the archive publisher directory fence");
        let deferred = tokio::time::timeout(
            Duration::from_secs(1),
            repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool),
        )
        .await
        .expect("busy archive fencing must not wait while holding SQLite")
        .expect("defer busy archive repair");
        assert_eq!(deferred, 0);
        let retained_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM upstream_account_usage_breakdown_hourly WHERE bucket_start_epoch = ?1",
        )
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("read last-good rows while the archive publisher is fenced");
        assert_eq!(retained_rows, 2);
        drop(publisher_fence);
    }

    let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
        .await
        .expect("repair materialized usage breakdown state");
    assert_eq!(touched, 2);

    let remaining_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM upstream_account_usage_breakdown_hourly WHERE bucket_start_epoch = ?1",
    )
    .bind(bucket_start_epoch)
    .fetch_one(&pool)
    .await
    .expect("count remaining breakdown rows");
    assert_eq!(remaining_rows, 0);

    for file_path in [first_file_path.as_str(), second_file_path.as_str()] {
        let replay_marker_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
        )
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .expect("count replay markers after repair");
        assert_eq!(replay_marker_count, 0);

        let materialized_at: Option<String> = sqlx::query_scalar(
            "SELECT historical_rollups_materialized_at FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .expect("load archive materialized state after repair");
        assert!(materialized_at.is_none());

        let progress_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hourly_rollup_archive_progress WHERE dataset = ?1 AND file_path = ?2",
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .expect("count archive progress rows after repair");
        assert_eq!(progress_count, 0);
    }

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
