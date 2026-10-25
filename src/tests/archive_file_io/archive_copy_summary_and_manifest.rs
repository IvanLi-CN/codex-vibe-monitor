use super::archive_backfill_test_support::{
    budgeted_archive_io_test_dir, write_valid_invocation_archive,
};
use super::*;
use serde_json::json;

#[tokio::test]
async fn inflate_gzip_sqlite_file_with_budget_stops_mid_inflate_when_elapsed_budget_is_exhausted() {
    let temp_dir = budgeted_archive_io_test_dir("historical-rollup-budgeted-inflate");
    let source_path = temp_dir.join("archive.sqlite.gz");
    let destination_path = temp_dir.join("archive.sqlite");
    let payload = vec![b'a'; HISTORICAL_ROLLUP_ARCHIVE_INFLATE_BUFFER_BYTES * 4];

    {
        let output = fs::File::create(&source_path).expect("create gzip source");
        let mut encoder = GzEncoder::new(io::BufWriter::new(output), Compression::default());
        encoder.write_all(&payload).expect("write gzip payload");
        let mut writer = encoder.finish().expect("finish gzip payload");
        writer.flush().expect("flush gzip payload");
    }

    let completed = inflate_gzip_sqlite_file_with_budget(
        &source_path,
        &destination_path,
        Instant::now() - Duration::from_millis(25),
        Some(Duration::from_millis(1)),
    )
    .await
    .expect("inflate with budget");

    assert!(!completed, "expired elapsed budget should stop inflate");
    let written = fs::metadata(&destination_path)
        .expect("inflated temp file should exist")
        .len() as usize;
    assert!(
        written > 0,
        "budgeted inflate should still write at least one chunk"
    );
    assert!(
        written < payload.len(),
        "expired elapsed budget should stop before the whole sqlite copy completes"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_temp_copy_is_bound_to_manifest_sha_not_size_and_mtime() {
    let temp_dir = budgeted_archive_io_test_dir("historical-rollup-manifest-sha-binding");
    let archive_path = temp_dir.join("archive.sqlite.gz");
    write_valid_invocation_archive(&archive_path, "same-size-a").await;
    let manifest_sha = sha256_hex_file(&archive_path).expect("hash original archive");
    let temp_path = temp_dir.join("archive.sqlite");

    let archive_pool =
        open_historical_rollup_archive_pool(&archive_path, &temp_path, &manifest_sha)
            .await
            .expect("open archive temp copy");
    archive_pool.close().await;
    assert_eq!(
        load_historical_rollup_temp_source_signature(&temp_path).as_deref(),
        Some(manifest_sha.as_str())
    );

    let original_metadata = fs::metadata(&archive_path).expect("inspect original archive");
    let original_mtime = original_metadata
        .modified()
        .expect("read original archive mtime");
    let mut replacement = fs::read(&archive_path).expect("read original archive bytes");
    let last = replacement.last_mut().expect("archive has bytes");
    *last ^= 0x01;
    fs::write(&archive_path, &replacement).expect("write same-size replacement archive");
    filetime::set_file_mtime(
        &archive_path,
        filetime::FileTime::from_system_time(original_mtime),
    )
    .expect("restore replacement archive mtime");

    let replacement_metadata = fs::metadata(&archive_path).expect("inspect replacement archive");
    assert_eq!(replacement_metadata.len(), original_metadata.len());
    assert_eq!(
        replacement_metadata
            .modified()
            .expect("read replacement archive mtime"),
        original_mtime
    );
    assert_ne!(
        sha256_hex_file(&archive_path).expect("hash replacement archive"),
        manifest_sha
    );

    assert!(
        open_historical_rollup_archive_pool(&archive_path, &temp_path, &manifest_sha)
            .await
            .is_err(),
        "same-size/same-mtime replacement must not reuse a manifest-bound temp copy"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn invocation_archive_validation_cleans_stable_temp_db_after_success() {
    let temp_dir = budgeted_archive_io_test_dir("invocation-archive-validation-cleanup");
    let archive_path = temp_dir.join("archive.sqlite.gz");
    write_valid_invocation_archive(&archive_path, "stable-temp").await;
    let manifest_sha256 = sha256_hex_file(&archive_path).expect("hash invocation archive");
    let temp_path = invocation_archive_replay_temp_path(&archive_path);
    remove_temp_sqlite_artifacts(&temp_path);

    assert_eq!(
        invocation_archive_file_is_readable_with_budget(
            &archive_path,
            &manifest_sha256,
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
async fn historical_rollup_replay_paths_are_stable_and_source_bound() {
    let temp_dir = budgeted_archive_io_test_dir("historical-rollup-stable-temp");
    let archive_path = temp_dir.join("archive.sqlite.gz");
    write_valid_invocation_archive(&archive_path, "stable-temp").await;
    let manifest_sha256 = sha256_hex_file(&archive_path).expect("hash invocation archive");

    let invocation_temp_path = invocation_archive_replay_temp_path(&archive_path);
    let forward_proxy_temp_path = forward_proxy_archive_replay_temp_path(&archive_path);
    assert_ne!(invocation_temp_path, forward_proxy_temp_path);
    for temp_path in [&invocation_temp_path, &forward_proxy_temp_path] {
        remove_temp_sqlite_artifacts(temp_path);
        let archive_pool = open_historical_rollup_archive_pool_with_budget(
            &archive_path,
            temp_path,
            &manifest_sha256,
            Instant::now(),
            None,
        )
        .await
        .expect("open stable archive temp db")
        .expect("unbounded stable archive open should start replay");
        archive_pool.close().await;
        let source_signature = load_historical_rollup_temp_source_signature(temp_path)
            .expect("persist source signature for stable archive temp db");

        let archive_pool = open_historical_rollup_archive_pool_with_budget(
            &archive_path,
            temp_path,
            &manifest_sha256,
            Instant::now(),
            Some(Duration::from_secs(1)),
        )
        .await
        .expect("reuse stable archive temp db")
        .expect("budget should allow stable archive reuse");
        archive_pool.close().await;
        assert_eq!(
            load_historical_rollup_temp_source_signature(temp_path),
            Some(source_signature)
        );
        remove_temp_sqlite_artifacts(temp_path);
    }

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn historical_rollup_elapsed_budget_reached_respects_unbounded_mode() {
    assert!(!historical_rollup_elapsed_budget_reached(
        Instant::now(),
        None
    ));
}

#[test]
fn invocation_archive_replay_treats_missing_first_token_column_as_null() {
    let legacy_query = build_invocation_archive_rows_chunk_query(&HashSet::new());
    assert!(legacy_query.contains("NULL AS first_token_ms"));

    let modern_query =
        build_invocation_archive_rows_chunk_query(&HashSet::from(["first_token_ms".to_string()]));
    assert!(modern_query.contains("first_token_ms,"));
    assert!(!modern_query.contains("NULL AS first_token_ms"));
}

#[tokio::test]
async fn replay_budget_exhaustion_before_the_first_row_has_no_progress() {
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open archive pool");
    sqlx::query("CREATE TABLE forward_proxy_attempts (id INTEGER PRIMARY KEY)")
        .execute(&archive_pool)
        .await
        .expect("create forward proxy archive schema");
    let mut tx = SqliteConnection::connect("sqlite::memory:")
        .await
        .expect("open rollup transaction connection");

    let replay = replay_forward_proxy_archive_rows_into_hourly_rollups_tx_with_budget(
        &mut tx,
        &archive_pool,
        0,
        Instant::now() - Duration::from_millis(1),
        Some(Duration::ZERO),
    )
    .await
    .expect("stop before replaying archive rows");

    assert_eq!(
        replay.outcome,
        HistoricalRollupArchiveReplayOutcome::HitBudget
    );
    assert_eq!(replay.cursor_id, 0);
    assert!(!historical_rollup_replay_made_progress(replay, 0));
}

#[test]
fn candidate_with_prior_replay_progress_remains_actionable_after_later_budget_exhaustion() {
    assert!(historical_rollup_candidate_changed(true, false, false));
    assert!(!historical_rollup_candidate_changed(false, false, false));
}

#[tokio::test]
async fn legacy_summary_snapshot_backfill_materializes_v2_before_raw_source_loss() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("summary-snapshot-v2-backfill").await;
    let archive_db_path = temp_dir.join("legacy-summary-v2-backfill.sqlite");
    let archive_path = temp_dir.join("legacy-summary-v2-backfill.sqlite.gz");
    fs::File::create(&archive_db_path).expect("create legacy Summary archive sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open legacy Summary archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create legacy Summary archive schema");
    let occurred_at = "2026-08-01 00:00:00";
    sqlx::query(
        "INSERT INTO codex_invocations
         (id, invoke_id, occurred_at, raw_response, created_at, payload)
         VALUES (1, 'legacy-summary-v2-row', ?1, '{}', ?1, NULL)",
    )
    .bind(occurred_at)
    .execute(&archive_pool)
    .await
    .expect("insert legacy Summary archive row");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress legacy Summary archive");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash legacy Summary archive");
    sqlx::query(
        "INSERT INTO archive_batches
         (id, dataset, month_key, file_path, sha256, row_count, status,
          summary_source_kind, coverage_start_at, coverage_end_at)
         VALUES (1, 'codex_invocations', '2026-08', ?1, ?2, 1, 'completed',
                 'unknown', ?3, ?3)",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .bind(occurred_at)
    .execute(&pool)
    .await
    .expect("seed legacy Summary archive manifest");

    let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(5))
        .await
        .expect("backfill legacy Summary archive Snapshot V2");
    assert_eq!(result.materialized_archive_batches, 1);
    assert!(
        summary_archive_snapshot_has_proof(&pool, 1, &archive_sha256)
            .await
            .expect("validate backfilled V2 proof")
    );
    let (snapshot_format, snapshot_payload): (i64, Vec<u8>) = sqlx::query_as(
        "SELECT format_version, payload FROM summary_archive_snapshot
         WHERE archive_batch_id = 1 AND manifest_sha256 = ?1",
    )
    .bind(&archive_sha256)
    .fetch_one(&pool)
    .await
    .expect("load backfilled Snapshot format");
    assert_eq!(snapshot_format, SUMMARY_ARCHIVE_SNAPSHOT_V2);
    let snapshot_records = decode_summary_archive_snapshot_v2_payload(&snapshot_payload)
        .expect("decode backfilled Snapshot payload");
    assert_eq!(snapshot_records[0].invoke_id, "legacy-summary-v2-row");

    fs::remove_file(&archive_path).expect("remove legacy raw source after V2 proof");
    let second = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("reuse V2 authority after raw source loss");
    assert_eq!(second.materialized_archive_batches, 0);
    assert_eq!(second.unavailable_archive_batches, 0);
    assert!(
        summary_archive_snapshot_has_proof(&pool, 1, &archive_sha256)
            .await
            .expect("retain V2 proof after raw source loss")
    );
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn legacy_detail_mirror_clean_cycle_idles_until_archive_completion_wake() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("legacy-detail-mirror-clean-cycle").await;
    let first_path = temp_dir.join("legacy-detail-mirror-first.sqlite.gz");
    sqlx::query(
        "INSERT INTO archive_batches
         (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind)
         VALUES (1, 'codex_invocations', '2026-08', ?1, 'first', 0, 'completed', 'unknown')",
    )
    .bind(first_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("seed completed legacy mirror candidate");

    let clean = reconcile_legacy_detail_mirrors_startup_window(&pool, 1, Duration::from_secs(1))
        .await
        .expect("finish a clean legacy mirror cycle");
    assert_eq!(clean.candidate_count, 0);
    assert_eq!(clean.next_cursor_id, 1);
    assert!(!clean.wrapped);

    let second_path = temp_dir.join("legacy-detail-mirror-second.sqlite.gz");
    sqlx::query(
        "INSERT INTO archive_batches
         (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind)
         VALUES (2, 'codex_invocations', '2026-08', ?1, 'second', 0, 'completed', 'unknown')",
    )
    .bind(second_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("seed a newly completed unknown archive");
    let wake_candidate = reconcile_legacy_detail_mirrors_startup_window(&pool, 1, Duration::ZERO)
        .await
        .expect("bound the pass after an archive completion wake");
    assert_eq!(wake_candidate.candidate_count, 1);
    assert!(wake_candidate.hit_budget);
    assert!(!wake_candidate.wrapped);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_snapshot_v2_backfill_uses_occurred_at_id_order() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("summary-snapshot-v2-time-order").await;
    let archive_db_path = temp_dir.join("legacy-summary-v2-time-order.sqlite");
    let archive_path = temp_dir.join("legacy-summary-v2-time-order.sqlite.gz");
    fs::File::create(&archive_db_path).expect("create legacy Summary archive sqlite");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open legacy Summary archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create legacy Summary archive schema");

    // The first ID is deliberately the newest event. An ID-ordered pager puts it in the first
    // page ahead of older rows and cannot satisfy the V2 chronological proof across pages.
    let row_count = 401_i64;
    let base = parse_shanghai_local_naive("2026-08-01 00:00:00").expect("valid base timestamp");
    for id in 1..=row_count {
        let offset = if id == 1 { row_count } else { id - 2 };
        let occurred_at = format_naive(base + ChronoDuration::seconds(offset));
        sqlx::query(
            "INSERT INTO codex_invocations
             (id, invoke_id, occurred_at, raw_response, created_at, payload)
             VALUES (?1, ?2, ?3, '{}', ?3, NULL)",
        )
        .bind(id)
        .bind(format!("summary-time-order-{id}"))
        .bind(&occurred_at)
        .execute(&archive_pool)
        .await
        .expect("insert legacy Summary archive row");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress legacy Summary archive");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash legacy Summary archive");
    let coverage_start = format_naive(base);
    let coverage_end = format_naive(base + ChronoDuration::seconds(row_count));
    sqlx::query(
        "INSERT INTO archive_batches
         (id, dataset, month_key, file_path, sha256, row_count, status,
          summary_source_kind, coverage_start_at, coverage_end_at)
         VALUES (1, 'codex_invocations', '2026-08', ?1, ?2, ?3, 'completed',
                 'unknown', ?4, ?5)",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&archive_sha256)
    .bind(row_count)
    .bind(&coverage_start)
    .bind(&coverage_end)
    .execute(&pool)
    .await
    .expect("seed legacy Summary archive manifest");

    let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(5))
        .await
        .expect("backfill legacy Summary archive Snapshot V2");
    assert_eq!(result.materialized_archive_batches, 1);
    assert!(
        summary_archive_snapshot_has_proof(&pool, 1, &archive_sha256)
            .await
            .expect("validate chronological V2 proof")
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_snapshot_backfill_reactivates_after_manifest_sha_change() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("summary-snapshot-v2-sha-reactivation").await;
    let archive_path = temp_dir.join("summary-snapshot-v2-sha-reactivation.sqlite.gz");
    fs::write(&archive_path, b"not-a-sqlite-archive").expect("write invalid archive fixture");
    let initial_sha = sha256_hex_file(&archive_path).expect("hash archive fixture");
    let stale_sha = format!("{initial_sha}-stale");
    sqlx::query(
        "INSERT INTO archive_batches
         (id, dataset, month_key, file_path, sha256, row_count, status,
          summary_source_kind, coverage_start_at, coverage_end_at)
         VALUES (1, 'codex_invocations', '2026-08', ?1, ?2, 1, 'completed',
                 'unknown', '2026-08-01 00:00:00', '2026-08-01 01:00:00')",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(&stale_sha)
    .execute(&pool)
    .await
    .expect("seed stale Summary archive manifest");

    let first = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("run stale SHA backfill");
    assert_eq!(first.unavailable_archive_batches, 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT failure_kind FROM summary_archive_snapshot_backfill_outcome
             WHERE archive_batch_id = 1 AND manifest_sha256 = ?1",
        )
        .bind(&stale_sha)
        .fetch_one(&pool)
        .await
        .expect("load stale SHA outcome"),
        "manifest_sha_mismatch"
    );

    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE id = 1")
        .bind(&initial_sha)
        .execute(&pool)
        .await
        .expect("update manifest SHA");
    let second = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("run reactivated SHA backfill");
    assert_eq!(second.unavailable_archive_batches, 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT failure_kind FROM summary_archive_snapshot_backfill_outcome
             WHERE archive_batch_id = 1 AND manifest_sha256 = ?1",
        )
        .bind(&initial_sha)
        .fetch_one(&pool)
        .await
        .expect("load reactivated SHA outcome"),
        "source_open_failed"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_manifest_refresh_dedupes_duplicate_account_rows_from_archive_file() {
    let (pool, config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-refresh-dedupe").await;
    let primary_account_id = 996_i64;
    let secondary_account_id = 997_i64;
    let created_at = format_utc_iso(Utc::now());
    for (account_id, display_name) in [
        (primary_account_id, "Manifest duplicate primary"),
        (secondary_account_id, "Manifest duplicate secondary"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, kind, provider, display_name, status, enabled, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(account_id)
        .bind("api_key_codex")
        .bind("codex")
        .bind(display_name)
        .bind("active")
        .bind(1_i64)
        .bind(&created_at)
        .bind(&created_at)
        .execute(&pool)
        .await
        .expect("insert manifest refresh account");
    }

    let base_occurred_at = parse_shanghai_local_naive(&shanghai_local_days_ago(120, 8, 0, 0))
        .expect("valid shanghai local");
    let month_key = format_naive(base_occurred_at)[..7].to_string();
    let archive_path = archive_batch_file_path(&config, "codex_invocations", &month_key)
        .expect("resolve archive manifest refresh path");
    fs::create_dir_all(archive_path.parent().expect("archive parent"))
        .expect("create archive manifest refresh parent");

    let archive_db_path = temp_dir.join("archive-manifest-refresh-dedupe.sqlite");
    fs::File::create(&archive_db_path).expect("create archive sqlite file");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create archive schema");

    let repeated_rows = BACKFILL_ACCOUNT_BIND_BATCH_SIZE + 5;
    let mut primary_latest = String::new();
    let mut secondary_latest = String::new();
    for idx in 0..repeated_rows {
        let occurred_at = format_naive(base_occurred_at + ChronoDuration::seconds(idx as i64));
        primary_latest = occurred_at.clone();
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, raw_response, created_at, payload
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(idx as i64 + 1)
        .bind(format!("manifest-refresh-primary-{idx}"))
        .bind(&occurred_at)
        .bind("{}")
        .bind(&occurred_at)
        .bind(json!({ "upstreamAccountId": primary_account_id }).to_string())
        .execute(&archive_pool)
        .await
        .expect("insert repeated primary manifest row");
    }
    for idx in 0..2 {
        let occurred_at = format_naive(
            base_occurred_at + ChronoDuration::seconds(repeated_rows as i64 + idx as i64 + 1),
        );
        secondary_latest = occurred_at.clone();
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id, invoke_id, occurred_at, raw_response, created_at, payload
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(repeated_rows as i64 + idx as i64 + 1)
        .bind(format!("manifest-refresh-secondary-{idx}"))
        .bind(&occurred_at)
        .bind("{}")
        .bind(&occurred_at)
        .bind(json!({ "upstreamAccountId": secondary_account_id }).to_string())
        .execute(&archive_pool)
        .await
        .expect("insert repeated secondary manifest row");
    }
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress manifest refresh archive");

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
    .bind(&month_key)
    .bind(archive_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_path).expect("archive sha"))
    .bind((repeated_rows + 2) as i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(format_naive(base_occurred_at))
    .bind(secondary_latest.clone())
    .execute(&pool)
    .await
    .expect("insert manifest refresh batch");

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("refresh manifest rows for duplicate accounts");
    assert_eq!(refresh.pending_batches, 1);
    assert_eq!(refresh.refreshed_batches, 1);
    assert_eq!(refresh.account_rows_written, 2);

    let manifest_rows = sqlx::query_as::<_, (i64, String)>(
        r#"
        SELECT account_id, last_activity_at
        FROM archive_batch_upstream_activity
        ORDER BY account_id ASC
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load refreshed manifest rows");
    assert_eq!(
        manifest_rows,
        vec![
            (primary_account_id, primary_latest),
            (secondary_account_id, secondary_latest),
        ]
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_manifest_refresh_respects_the_retention_candidate_budget() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-refresh-budget").await;
    config.retention_batch_rows = 1;
    let created_at = format_utc_iso(Utc::now());

    for id in 1_i64..=3 {
        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status, created_at
            )
            VALUES (?1, 'codex_invocations', '2026-01', ?2, 'missing', 1, ?3, ?4)
            "#,
        )
        .bind(id)
        .bind(
            temp_dir
                .join(format!("missing-manifest-{id}.sqlite.gz"))
                .to_string_lossy()
                .to_string(),
        )
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(&created_at)
        .execute(&pool)
        .await
        .expect("insert missing manifest candidate");
    }

    let refresh = refresh_archive_upstream_activity_manifest(&pool, &config, false)
        .await
        .expect("refresh bounded manifest candidates");
    let pending_after_refresh: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE upstream_activity_manifest_refreshed_at IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count pending manifest candidates");
    assert_eq!(refresh.pending_batches, 1);
    assert_eq!(refresh.missing_files, 1);
    assert_eq!(pending_after_refresh, 3);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn archive_manifest_activity_replacement_clears_and_writes_in_micro_batches() {
    let (pool, mut config, temp_dir) =
        retention_memory_test_pool_and_config("archive-manifest-activity-micro-batches").await;
    config.retention_batch_rows = 2;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            id, dataset, month_key, file_path, sha256, row_count, status, created_at
        )
        VALUES (99, 'codex_invocations', '2026-01', 'memory://manifest-activity', 'manifest', 5, ?1, ?2)
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&created_at)
    .execute(&pool)
    .await
    .expect("insert archive manifest batch");
    for account_id in 10_i64..15 {
        sqlx::query(
            "INSERT INTO archive_batch_upstream_activity (archive_batch_id, account_id, last_activity_at) VALUES (99, ?1, ?2)",
        )
        .bind(account_id)
        .bind("2026-01-01 00:00:00")
        .execute(&pool)
        .await
        .expect("seed stale manifest activity");
    }

    let next_values = (20_i64..25)
        .map(|account_id| (account_id, format!("2026-01-01 00:00:{account_id:02}")))
        .collect::<Vec<_>>();
    let written = replace_archive_batch_upstream_activity_in_micro_batches(
        &pool,
        &config,
        99,
        &next_values,
        0,
    )
    .await
    .expect("replace manifest activity in micro batches");
    let actual = sqlx::query_as::<_, (i64, String)>(
        "SELECT account_id, last_activity_at FROM archive_batch_upstream_activity WHERE archive_batch_id = 99 ORDER BY account_id ASC",
    )
    .fetch_all(&pool)
    .await
    .expect("load rebuilt manifest activity");
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT upstream_activity_manifest_refreshed_at FROM archive_batches WHERE id = 99",
    )
    .fetch_one(&pool)
    .await
    .expect("load manifest refresh marker");
    assert_eq!(written, Some(5));
    assert_eq!(actual, next_values);
    assert!(marker.is_some());

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
