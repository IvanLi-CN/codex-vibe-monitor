use super::*;
use serde_json::json;

#[tokio::test]
pub(crate) async fn shared_raw_path_replacement_updates_all_owners_in_bounded_batches() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("shared-raw-path-replacement-bounded").await;
    config.retention_batch_rows = 2;
    let old_path = temp_dir.join("shared-response.bin");
    let next_path = temp_dir.join("shared-response.bin.gz");
    let occurred_at = shanghai_local_days_ago(1, 12, 0, 0);

    for index in 0..5 {
        insert_retention_invocation(
            &pool,
            &format!("shared-raw-path-owner-{index}"),
            &occurred_at,
            SOURCE_PROXY,
            "success",
            None,
            "{}",
            None,
            Some(&old_path),
            Some(1),
            Some(0.0),
        )
        .await;
    }

    assert!(
        replace_proxy_raw_path_references(
            &pool,
            &config,
            old_path.to_string_lossy().as_ref(),
            next_path.to_string_lossy().as_ref(),
            RAW_CODEC_GZIP,
        )
        .await
        .expect("replace shared raw path references")
    );
    let remaining_old_references: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations WHERE response_raw_path = ?1")
            .bind(old_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("count old raw references");
    let updated_references: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE response_raw_path = ?1 AND response_raw_codec = ?2",
    )
    .bind(next_path.to_string_lossy().as_ref())
    .bind(RAW_CODEC_GZIP)
    .fetch_one(&pool)
    .await
    .expect("count updated raw references");
    assert_eq!(remaining_old_references, 0);
    assert_eq!(updated_references, 5);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn shared_raw_blob_keeps_file_reference_until_last_owner_is_removed() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("shared-raw-blob-links").await;
    let shared_path = config.proxy_raw_dir.join("shared-response.zst");
    let occurred_at = shanghai_local_days_ago(1, 12, 0, 0);
    insert_retention_invocation(
        &pool,
        "shared-raw-invocation",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        None,
        "{}",
        None,
        Some(&shared_path),
        Some(1),
        Some(0.0),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id, occurred_at, endpoint, route_mode, attempt_index,
            distinct_account_index, same_account_retry_index, status,
            response_raw_path, response_raw_codec, response_raw_size
        ) VALUES (?1, ?2, '/v1/responses', 'pool', 0, 0, 0, 'success', ?3, 'zstd', 1)
        "#,
    )
    .bind("shared-raw-invocation")
    .bind(&occurred_at)
    .bind(shared_path.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .expect("insert paired attempt raw link");

    let link_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_raw_payload_blob_links WHERE raw_path = ?1")
            .bind(shared_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("count shared links");
    assert_eq!(link_count, 2);

    sqlx::query("UPDATE codex_invocations SET response_raw_path = NULL WHERE invoke_id = ?1")
        .bind("shared-raw-invocation")
        .execute(&pool)
        .await
        .expect("release invocation raw link");
    let link_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_raw_payload_blob_links WHERE raw_path = ?1")
            .bind(shared_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("count attempt-only link");
    assert_eq!(link_count, 1);

    sqlx::query(
        "UPDATE pool_upstream_request_attempts SET response_raw_path = NULL WHERE invoke_id = ?1",
    )
    .bind("shared-raw-invocation")
    .execute(&pool)
    .await
    .expect("release attempt raw link");
    let blob_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM proxy_raw_payload_blobs WHERE raw_path = ?1")
            .bind(shared_path.to_string_lossy().as_ref())
            .fetch_one(&pool)
            .await
            .expect("count released blob");
    assert_eq!(blob_count, 0);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn schema_backfill_links_existing_pool_attempt_response_raw() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("legacy-attempt-raw-link").await;
    let shared_path = config.proxy_raw_dir.join("legacy-attempt-response.zst");
    let occurred_at = shanghai_local_days_ago(1, 12, 0, 0);
    insert_retention_invocation(
        &pool,
        "legacy-attempt-link",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        None,
        "{}",
        None,
        None,
        Some(1),
        Some(0.0),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id, occurred_at, endpoint, route_mode, attempt_index,
            distinct_account_index, same_account_retry_index, status,
            response_raw_path, response_raw_codec, response_raw_size
        ) VALUES (?1, ?2, '/v1/responses', 'pool', 0, 0, 0, 'success', ?3, 'zstd', 1)
        "#,
    )
    .bind("legacy-attempt-link")
    .bind(&occurred_at)
    .bind(shared_path.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .expect("insert legacy attempt raw");

    // Simulate an upgrade from before the blob-link triggers existed.
    sqlx::query("DELETE FROM proxy_raw_payload_blob_links")
        .execute(&pool)
        .await
        .expect("clear current links");
    sqlx::query("DELETE FROM proxy_raw_payload_blobs")
        .execute(&pool)
        .await
        .expect("clear current blobs");
    sqlx::query("DELETE FROM proxy_raw_payload_blob_link_migrations")
        .execute(&pool)
        .await
        .expect("clear migration marker");
    ensure_schema(&pool).await.expect("run legacy link seed");

    let link_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM proxy_raw_payload_blob_links WHERE raw_path = ?1 AND owner_kind = 'attempt'",
    )
    .bind(shared_path.to_string_lossy().as_ref())
    .fetch_one(&pool)
    .await
    .expect("count backfilled attempt link");
    assert_eq!(link_count, 1);

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn cold_compression_updates_every_shared_raw_blob_owner_before_removing_old_file()
{
    let (pool, mut config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("shared-raw-blob-cold-compression").await;
    let shared_path = config.proxy_raw_dir.join("shared-response.bin");
    fs::write(&shared_path, b"shared response raw").expect("write shared raw");
    let occurred_at = shanghai_local_days_ago(1, 12, 0, 0);
    insert_retention_invocation(
        &pool,
        "shared-raw-compression",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        None,
        "{}",
        None,
        Some(&shared_path),
        Some(1),
        Some(0.0),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id, occurred_at, endpoint, route_mode, attempt_index,
            distinct_account_index, same_account_retry_index, status,
            response_raw_path, response_raw_codec, response_raw_size
        ) VALUES (?1, ?2, '/v1/responses', 'pool', 0, 0, 0, 'success', ?3, 'identity', 19)
        "#,
    )
    .bind("shared-raw-compression")
    .bind(&occurred_at)
    .bind(shared_path.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .expect("insert paired attempt raw link");
    config.proxy_raw_hot_secs = 0;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;

    compress_cold_proxy_raw_payloads(&pool, &config, None, false)
        .await
        .expect("cold compress shared raw");

    let invocation_path: String =
        sqlx::query_scalar("SELECT response_raw_path FROM codex_invocations WHERE invoke_id = ?1")
            .bind("shared-raw-compression")
            .fetch_one(&pool)
            .await
            .expect("load invocation path");
    let attempt_path: String = sqlx::query_scalar(
        "SELECT response_raw_path FROM pool_upstream_request_attempts WHERE invoke_id = ?1",
    )
    .bind("shared-raw-compression")
    .fetch_one(&pool)
    .await
    .expect("load attempt path");
    assert_eq!(invocation_path, attempt_path);
    assert!(invocation_path.ends_with(".gz"));
    assert!(Path::new(&invocation_path).exists());
    assert!(!shared_path.exists());

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn bootstrap_hourly_rollups_ignores_missing_invocation_archive_batch() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("hourly-rollup-missing-invocation-archive").await;
    let missing_archive = temp_dir.join("missing-codex-invocations.sqlite.gz");
    let missing_archive_path = missing_archive.to_string_lossy().to_string();

    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("codex_invocations")
    .bind("2025-01")
    .bind(&missing_archive_path)
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("insert missing codex_invocations archive manifest");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("missing codex_invocations archive batch should not affect bootstrap");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn bootstrap_hourly_rollups_ignores_missing_forward_proxy_archive_batch() {
    let (pool, _config, temp_dir) =
        retention_memory_test_pool_and_config("hourly-rollup-missing-forward-proxy-archive").await;
    let missing_archive = temp_dir.join("missing-forward-proxy-attempts.sqlite.gz");
    let missing_archive_path = missing_archive.to_string_lossy().to_string();

    sqlx::query(
        r#"
        INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
        "#,
    )
    .bind("forward_proxy_attempts")
    .bind("2025-01")
    .bind(&missing_archive_path)
    .bind("deadbeef")
    .bind(1_i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .execute(&pool)
    .await
    .expect("insert missing forward_proxy_attempts archive manifest");

    bootstrap_hourly_rollups(&pool)
        .await
        .expect("missing forward_proxy_attempts archive batch should not affect bootstrap");

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_compacts_old_quota_snapshots_by_shanghai_day() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-quota").await;
    let same_day_early = utc_naive_from_shanghai_local_days_ago(40, 8, 0, 0);
    let same_day_late = utc_naive_from_shanghai_local_days_ago(40, 23, 0, 0);
    let next_day = utc_naive_from_shanghai_local_days_ago(39, 9, 0, 0);
    seed_quota_snapshot(&pool, &same_day_early).await;
    seed_quota_snapshot(&pool, &same_day_late).await;
    seed_quota_snapshot(&pool, &next_day).await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run quota compaction");
    assert_eq!(summary.quota_snapshot_rows_archived, 1);

    let remaining: Vec<String> = sqlx::query_scalar(
        "SELECT captured_at FROM codex_quota_snapshots ORDER BY captured_at ASC",
    )
    .fetch_all(&pool)
    .await
    .expect("load remaining quota snapshots");
    assert_eq!(remaining, vec![same_day_late.clone(), next_day.clone()]);

    let quota_batch_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'codex_quota_snapshots'",
    )
    .fetch_one(&pool)
    .await
    .expect("count quota archive batches");
    assert_eq!(quota_batch_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_orphan_sweep_skips_fresh_raw_files() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-orphan-grace").await;
    let orphan = config.proxy_raw_dir.join("fresh-orphan.bin");
    fs::write(&orphan, b"fresh-orphan").expect("write fresh orphan");

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention with fresh orphan");
    assert_eq!(summary.orphan_raw_files_removed, 0);
    assert!(orphan.exists());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test(flavor = "current_thread")]
pub(crate) async fn retention_orphan_sweep_anchors_relative_raw_dir_to_database_parent() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().await;
    let temp_dir = make_temp_test_dir("retention-orphan-db-parent");
    let db_root = temp_dir.join("db-root");
    let cwd_root = temp_dir.join("cwd-root");
    fs::create_dir_all(&db_root).expect("create db root");
    fs::create_dir_all(&cwd_root).expect("create cwd root");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd_root);

    let db_path = db_root.join("codex-vibe-monitor.db");
    fs::File::create(&db_path).expect("create sqlite file");
    let pool = SqlitePool::connect(&test_sqlite_url_for_path(&db_path))
        .await
        .expect("connect retention sqlite");
    ensure_schema(&pool).await.expect("ensure retention schema");

    let mut config = test_config();
    config.database_path = db_path;
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    let anchored_dir = config.resolved_proxy_raw_dir();
    fs::create_dir_all(&anchored_dir).expect("create anchored raw dir");
    let anchored_orphan = anchored_dir.join("anchored-orphan.bin");
    fs::write(&anchored_orphan, b"anchored-orphan").expect("write anchored orphan");
    set_file_mtime_seconds_ago(&anchored_orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);

    let cwd_raw_dir = cwd_root.join("proxy_raw_payloads");
    fs::create_dir_all(&cwd_raw_dir).expect("create cwd raw dir");
    let cwd_orphan = cwd_raw_dir.join("cwd-orphan.bin");
    fs::write(&cwd_orphan, b"cwd-orphan").expect("write cwd orphan");
    set_file_mtime_seconds_ago(&cwd_orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);

    let removed = sweep_orphan_proxy_raw_files(&pool, &config, None, false)
        .await
        .expect("run orphan sweep");

    assert_eq!(removed, 1);
    assert!(
        !anchored_orphan.exists(),
        "orphan sweep should clean the database-anchored raw dir"
    );
    assert!(
        cwd_orphan.exists(),
        "orphan sweep should stop scanning cwd-relative stray files"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_dry_run_does_not_mutate_database_or_files() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-dry-run").await;
    let response_raw = config.proxy_raw_dir.join("dry-run-response.bin");
    let orphan = config.proxy_raw_dir.join("dry-run-orphan.bin");
    fs::write(&response_raw, b"dry-run-response").expect("write dry-run response raw");
    fs::write(&orphan, b"dry-run-orphan").expect("write dry-run orphan");
    set_file_mtime_seconds_ago(&orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
    let occurred_at = shanghai_local_days_ago(91, 7, 0, 0);
    insert_retention_invocation(
        &pool,
        "dry-run-old",
        &occurred_at,
        SOURCE_XY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        Some(&response_raw),
        Some(111),
        Some(0.9),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(true), None)
        .await
        .expect("run dry-run retention");
    assert!(summary.dry_run);
    assert_eq!(summary.invocation_rows_archived, 1);
    assert_eq!(summary.archive_batches_touched, 1);
    assert_eq!(summary.raw_files_removed, 1);
    assert_eq!(summary.orphan_raw_files_removed, 1);
    assert!(response_raw.exists());
    assert!(orphan.exists());

    let row = sqlx::query(
        "SELECT detail_level, payload, raw_response FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("dry-run-old")
    .fetch_one(&pool)
    .await
    .expect("load dry-run invocation");
    assert_eq!(row.get::<String, _>("detail_level"), DETAIL_LEVEL_FULL);
    assert!(row.get::<Option<String>, _>("payload").is_some());
    assert_eq!(row.get::<String, _>("raw_response"), "{\"ok\":true}");

    let archive_batch_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count dry-run archive batches");
    assert_eq!(archive_batch_count, 0);

    let archive_files = fs::read_dir(&config.archive_dir)
        .expect("read archive dir")
        .count();
    assert_eq!(archive_files, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_compresses_cold_raw_payloads_and_updates_paths() {
    let (pool, mut config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-cold-compress-live").await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;

    let request_raw = config.proxy_raw_dir.join("cold-request.bin");
    let response_raw = config.proxy_raw_dir.join("cold-response.bin");
    fs::write(&request_raw, b"{\"type\":\"request\"}").expect("write cold request raw");
    fs::write(&response_raw, b"{\"type\":\"response\"}").expect("write cold response raw");

    let occurred_at = shanghai_local_days_ago(2, 10, 0, 0);
    insert_retention_invocation(
        &pool,
        "cold-compress-live",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&request_raw),
        Some(&response_raw),
        Some(55),
        Some(0.12),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run cold compression retention");
    assert_eq!(summary.raw_files_compression_candidates, 2);
    assert_eq!(summary.raw_files_compressed, 2);
    assert!(summary.raw_bytes_before > 0);
    assert!(summary.raw_bytes_after > 0);
    assert_eq!(summary.raw_bytes_after_estimated, 0);
    assert!(!request_raw.exists());
    assert!(!response_raw.exists());

    let compressed_request = PathBuf::from(format!("{}.gz", request_raw.display()));
    let compressed_response = PathBuf::from(format!("{}.gz", response_raw.display()));
    assert!(compressed_request.exists());
    assert!(compressed_response.exists());

    let row = sqlx::query(
        r#"
        SELECT
            request_raw_path,
            request_raw_size,
            response_raw_path,
            response_raw_size
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("cold-compress-live")
    .fetch_one(&pool)
    .await
    .expect("load cold compressed row");

    let request_raw_path = row.get::<Option<String>, _>("request_raw_path");
    let response_raw_path = row.get::<Option<String>, _>("response_raw_path");
    assert_eq!(
        request_raw_path.as_deref(),
        Some(compressed_request.to_string_lossy().as_ref())
    );
    assert_eq!(
        response_raw_path.as_deref(),
        Some(compressed_response.to_string_lossy().as_ref())
    );
    assert_eq!(
        row.get::<Option<i64>, _>("request_raw_size"),
        Some(b"{\"type\":\"request\"}".len() as i64)
    );
    assert_eq!(
        row.get::<Option<i64>, _>("response_raw_size"),
        Some(b"{\"type\":\"response\"}".len() as i64)
    );
    assert_eq!(
        read_proxy_raw_bytes(
            request_raw_path.as_deref().expect("request raw path"),
            config.database_path.parent(),
        )
        .expect("read compressed request raw"),
        b"{\"type\":\"request\"}"
    );
    assert_eq!(
        read_proxy_raw_bytes(
            response_raw_path.as_deref().expect("response raw path"),
            config.database_path.parent(),
        )
        .expect("read compressed response raw"),
        b"{\"type\":\"response\"}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_cold_compression_repair_keeps_relative_db_paths() {
    let (pool, mut config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-cold-compress-relative-repair")
            .await;
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");
    config.proxy_raw_compression = RawCompressionCodec::Gzip;

    let relative_raw = PathBuf::from("proxy_raw_payloads/relative-repair.bin");
    let compressed_raw = temp_dir.join(format!("{}.gz", relative_raw.display()));
    if let Some(parent) = compressed_raw.parent() {
        fs::create_dir_all(parent).expect("create compressed raw parent");
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(b"{\"type\":\"relative-repair\"}")
        .expect("write compressed payload");
    fs::write(
        &compressed_raw,
        encoder.finish().expect("finish compressed payload"),
    )
    .expect("write compressed raw file");

    let outcome = maybe_compress_proxy_raw_path(
        &pool,
        1,
        "request_raw_path",
        Some(relative_raw.to_str().expect("utf-8 relative path")),
        RawCompressionCodec::Gzip,
        config.database_path.parent(),
        false,
    )
    .await
    .expect("repair relative raw path");

    assert_eq!(
        outcome.new_db_path.as_deref(),
        Some("proxy_raw_payloads/relative-repair.bin.gz")
    );
    assert!(!outcome.compressed);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_skips_cold_compression_for_archive_eligible_rows() {
    let (pool, mut config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-cold-compress-skip-archive").await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;

    let request_raw = config.proxy_raw_dir.join("archive-eligible.bin");
    fs::write(&request_raw, b"{\"type\":\"archive-eligible\"}")
        .expect("write archive-eligible raw");

    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 1) as i64, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "cold-compress-skip-archive",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&request_raw),
        None,
        Some(12),
        Some(0.03),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention for archive-eligible row");

    assert_eq!(summary.raw_files_compression_candidates, 0);
    assert_eq!(summary.raw_files_compressed, 0);
    assert_eq!(summary.invocation_rows_archived, 1);
    assert!(!request_raw.exists());
    assert!(!PathBuf::from(format!("{}.gz", request_raw.display())).exists());

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[tokio::test]
pub(crate) async fn retention_continues_when_one_cold_compression_file_fails() {
    use std::os::unix::fs::PermissionsExt;

    let (pool, mut config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-cold-compress-continue-on-error")
            .await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;

    let broken_raw = config.proxy_raw_dir.join("broken-request.bin");
    let good_raw = config.proxy_raw_dir.join("good-request.bin");
    fs::write(&broken_raw, b"{\"type\":\"broken\"}").expect("write broken raw");
    fs::write(&good_raw, b"{\"type\":\"good\"}").expect("write good raw");

    let mut broken_permissions = fs::metadata(&broken_raw)
        .expect("read broken raw metadata")
        .permissions();
    broken_permissions.set_mode(0o000);
    fs::set_permissions(&broken_raw, broken_permissions).expect("chmod broken raw");

    let occurred_at = shanghai_local_days_ago(2, 8, 0, 0);
    insert_retention_invocation(
        &pool,
        "cold-compress-broken",
        &occurred_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":false}",
        Some(&broken_raw),
        None,
        Some(21),
        Some(0.04),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "cold-compress-good",
        &occurred_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":false}",
        Some(&good_raw),
        None,
        Some(22),
        Some(0.05),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention with cold-compression failure");

    let mut repaired_permissions = fs::metadata(&broken_raw)
        .expect("read broken raw metadata after run")
        .permissions();
    repaired_permissions.set_mode(0o644);
    fs::set_permissions(&broken_raw, repaired_permissions).expect("restore broken raw permissions");

    assert_eq!(summary.raw_files_compression_candidates, 1);
    assert_eq!(summary.raw_files_compressed, 1);
    assert!(broken_raw.exists(), "broken file should be left in place");
    assert!(!PathBuf::from(format!("{}.gz", broken_raw.display())).exists());
    assert!(!good_raw.exists(), "good file should be replaced by gzip");
    assert!(PathBuf::from(format!("{}.gz", good_raw.display())).exists());

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[tokio::test]
pub(crate) async fn retention_compresses_other_file_when_same_invocation_request_fails() {
    use std::os::unix::fs::PermissionsExt;

    let (pool, mut config, temp_dir) = retention_fresh_schema_test_pool_and_config(
        "retention-cold-compress-same-row-continue-on-error",
    )
    .await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;

    let broken_request = config.proxy_raw_dir.join("same-row-broken-request.bin");
    let good_response = config.proxy_raw_dir.join("same-row-good-response.bin");
    fs::write(&broken_request, b"{\"type\":\"broken-request\"}").expect("write broken request raw");
    fs::write(&good_response, b"{\"type\":\"good-response\"}").expect("write good response raw");

    let mut broken_permissions = fs::metadata(&broken_request)
        .expect("read broken request metadata")
        .permissions();
    broken_permissions.set_mode(0o000);
    fs::set_permissions(&broken_request, broken_permissions).expect("chmod broken request raw");

    let occurred_at = shanghai_local_days_ago(2, 8, 30, 0);
    insert_retention_invocation(
        &pool,
        "cold-compress-same-row-partial",
        &occurred_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":false}",
        Some(&broken_request),
        Some(&good_response),
        Some(30),
        Some(0.06),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run retention with same-row cold-compression failure");

    let mut repaired_permissions = fs::metadata(&broken_request)
        .expect("read broken request metadata after run")
        .permissions();
    repaired_permissions.set_mode(0o644);
    fs::set_permissions(&broken_request, repaired_permissions)
        .expect("restore broken request permissions");

    assert_eq!(summary.raw_files_compression_candidates, 1);
    assert_eq!(summary.raw_files_compressed, 1);
    assert!(
        broken_request.exists(),
        "broken request should be left in place"
    );
    assert!(!PathBuf::from(format!("{}.gz", broken_request.display())).exists());
    assert!(
        !good_response.exists(),
        "good response should be replaced by gzip"
    );
    let compressed_response = PathBuf::from(format!("{}.gz", good_response.display()));
    assert!(
        compressed_response.exists(),
        "good response should be compressed"
    );

    let row = sqlx::query(
        "SELECT request_raw_path, response_raw_path FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("cold-compress-same-row-partial")
    .fetch_one(&pool)
    .await
    .expect("load same-row partial cold compression row");
    assert_eq!(
        row.try_get::<Option<String>, _>("request_raw_path")
            .expect("decode request path")
            .as_deref(),
        Some(broken_request.to_string_lossy().as_ref())
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("response_raw_path")
            .expect("decode response path")
            .as_deref(),
        Some(compressed_response.to_string_lossy().as_ref())
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_dry_run_estimates_cold_raw_compression_without_mutating_files() {
    let (pool, mut config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-cold-compress-dry-run").await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;

    let request_raw = config.proxy_raw_dir.join("cold-dry-run-request.bin");
    fs::write(&request_raw, b"{\"type\":\"dry-run\"}").expect("write dry-run request raw");
    let occurred_at = shanghai_local_days_ago(2, 9, 0, 0);
    insert_retention_invocation(
        &pool,
        "cold-compress-dry-run",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&request_raw),
        None,
        Some(33),
        Some(0.08),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(true), None)
        .await
        .expect("run cold compression dry-run");
    assert!(summary.dry_run);
    assert_eq!(summary.raw_files_compression_candidates, 1);
    assert_eq!(summary.raw_files_compressed, 0);
    assert!(summary.raw_bytes_before > 0);
    assert_eq!(summary.raw_bytes_after, 0);
    assert!(summary.raw_bytes_after_estimated > 0);
    assert!(request_raw.exists());
    assert!(!PathBuf::from(format!("{}.gz", request_raw.display())).exists());

    let row = sqlx::query("SELECT request_raw_path FROM codex_invocations WHERE invoke_id = ?1")
        .bind("cold-compress-dry-run")
        .fetch_one(&pool)
        .await
        .expect("load dry-run cold row");
    assert_eq!(
        row.get::<Option<String>, _>("request_raw_path").as_deref(),
        Some(request_raw.to_string_lossy().as_ref())
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_cold_compression_scans_batches_in_occurred_at_order() {
    let (pool, mut config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-cold-compress-order").await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;
    config.retention_batch_rows = 1;

    let newest = config.proxy_raw_dir.join("order-newest.bin");
    let middle = config.proxy_raw_dir.join("order-middle.bin");
    let oldest = config.proxy_raw_dir.join("order-oldest.bin");
    fs::write(&newest, b"newest").expect("write newest raw");
    fs::write(&middle, b"middle").expect("write middle raw");
    fs::write(&oldest, b"oldest").expect("write oldest raw");

    insert_retention_invocation(
        &pool,
        "order-newest",
        &shanghai_local_days_ago(2, 23, 0, 0),
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&newest),
        None,
        Some(10),
        Some(0.01),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "order-middle",
        &shanghai_local_days_ago(3, 12, 0, 0),
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&middle),
        None,
        Some(10),
        Some(0.01),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "order-oldest",
        &shanghai_local_days_ago(4, 8, 0, 0),
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&oldest),
        None,
        Some(10),
        Some(0.01),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run ordered cold compression");
    assert_eq!(summary.raw_files_compression_candidates, 3);
    assert_eq!(summary.raw_files_compressed, 3);

    let compressed_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE request_raw_path LIKE '%.bin.gz'",
    )
    .fetch_one(&pool)
    .await
    .expect("count compressed ordered rows");
    assert_eq!(compressed_count, 3);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_cold_compression_budget_counts_missing_rows() {
    let (pool, mut config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-cold-compress-missing-budget").await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;
    config.retention_batch_rows = 1;

    let missing = config.proxy_raw_dir.join("budget-missing.bin");
    let good = config.proxy_raw_dir.join("budget-good.bin");
    fs::write(&good, b"good").expect("write good raw");

    insert_retention_invocation(
        &pool,
        "budget-missing",
        &shanghai_local_days_ago(4, 8, 0, 0),
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&missing),
        None,
        Some(10),
        Some(0.01),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "budget-good",
        &shanghai_local_days_ago(3, 8, 0, 0),
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&good),
        None,
        Some(10),
        Some(0.01),
    )
    .await;

    let summary = compress_cold_proxy_raw_payloads_with_budget(
        &pool,
        &config,
        config.database_path.parent(),
        false,
        Some(Duration::ZERO),
    )
    .await
    .expect("run cold compression with zero catchup budget");

    assert_eq!(summary.files_considered, 0);
    assert_eq!(summary.files_compressed, 0);
    assert!(
        good.exists(),
        "second row should not be compressed once budget is spent"
    );
    assert!(!PathBuf::from(format!("{}.gz", good.display())).exists());

    let stored_path: Option<String> =
        sqlx::query_scalar("SELECT request_raw_path FROM codex_invocations WHERE invoke_id = ?1")
            .bind("budget-good")
            .fetch_one(&pool)
            .await
            .expect("load good row path after budgeted run");
    assert_eq!(
        stored_path.as_deref(),
        Some(good.to_string_lossy().as_ref())
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn maintenance_raw_compression_cli_supports_dry_run_and_live_modes() {
    let (pool, mut config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("maintenance-raw-compression-cli").await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;

    let request_raw = config.proxy_raw_dir.join("maintenance-cli-request.bin");
    fs::write(&request_raw, b"{\"cli\":true}").expect("write maintenance cli raw");
    let occurred_at = shanghai_local_days_ago(2, 9, 15, 0);
    insert_retention_invocation(
        &pool,
        "maintenance-cli-row",
        &occurred_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":false}",
        Some(&request_raw),
        None,
        Some(9),
        Some(0.02),
    )
    .await;

    run_cli_command(
        &pool,
        &config,
        &CliCommand::Maintenance(MaintenanceCliArgs {
            command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: true }),
        }),
    )
    .await
    .expect("run maintenance raw compression dry-run");
    assert!(request_raw.exists());
    assert!(!PathBuf::from(format!("{}.gz", request_raw.display())).exists());

    run_cli_command(
        &pool,
        &config,
        &CliCommand::Maintenance(MaintenanceCliArgs {
            command: MaintenanceCommand::RawCompression(MaintenanceDryRunArgs { dry_run: false }),
        }),
    )
    .await
    .expect("run maintenance raw compression live");
    let compressed = PathBuf::from(format!("{}.gz", request_raw.display()));
    assert!(compressed.exists());
    assert!(!request_raw.exists());
    let codec: String =
        sqlx::query_scalar("SELECT request_raw_codec FROM codex_invocations WHERE invoke_id = ?1")
            .bind("maintenance-cli-row")
            .fetch_one(&pool)
            .await
            .expect("load maintenance cli codec");
    assert_eq!(codec, RAW_CODEC_GZIP);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
pub(crate) async fn retention_archives_rows_with_compressed_raw_payload_files() {
    let (pool, config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-archive-compressed-raw").await;
    let response_raw = config
        .proxy_raw_dir
        .join("archive-compressed-response.bin.gz");
    write_gzip_test_file(&response_raw, b"{\"type\":\"archived\"}");
    let occurred_at = shanghai_local_days_ago(91, 7, 30, 0);

    insert_retention_invocation(
        &pool,
        "archive-compressed-raw",
        &occurred_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        Some(&response_raw),
        Some(88),
        Some(0.42),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false), None)
        .await
        .expect("run archive with compressed raw");
    assert_eq!(summary.invocation_rows_archived, 1);
    assert!(summary.raw_files_removed >= 1);
    assert!(!response_raw.exists());

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[tokio::test]
pub(crate) async fn terminate_child_process_prefers_sigterm_when_process_exits_cleanly() {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("trap 'exit 0' TERM; while :; do sleep 0.1; done")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sigterm-friendly child");

    let outcome = terminate_child_process(&mut child, Duration::from_secs(1), "test-child").await;

    assert_eq!(outcome, ChildTerminationOutcome::Graceful);
    assert!(
        child
            .try_wait()
            .expect("poll child after terminate")
            .is_some()
    );
}

#[cfg(unix)]
#[tokio::test]
pub(crate) async fn terminate_child_process_falls_back_to_force_kill_when_grace_period_is_exhausted()
 {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("trap '' TERM; while :; do sleep 1 & wait $!; done")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn TERM-ignoring child for forced shutdown fallback");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let outcome =
        terminate_child_process(&mut child, Duration::from_millis(100), "test-child").await;

    assert_eq!(outcome, ChildTerminationOutcome::Forced);
    assert!(
        child
            .try_wait()
            .expect("poll child after force kill")
            .is_some()
    );
}
#[tokio::test]
pub(crate) async fn spawn_http_server_leaves_health_unready_until_runtime_declares_readiness() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");

    assert!(
        !state.startup_ready.load(Ordering::Acquire),
        "HTTP startup should not mark the app ready before runtime startup completes"
    );
    let response = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond while startup is incomplete");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
pub(crate) async fn http_server_graceful_shutdown_stops_accepting_new_connections() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let healthy_response = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond before shutdown");
    assert_eq!(healthy_response.status(), StatusCode::OK);

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");

    let err = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect_err("server should stop accepting new connections after shutdown");
    assert!(err.is_connect() || err.is_timeout());
}

pub(crate) fn build_large_imported_oauth_validate_body(
    item_count: usize,
    padding_len: usize,
) -> String {
    let items = (0..item_count)
        .map(|index| {
            json!({
                "sourceId": format!("source-{index}"),
                "fileName": format!("user-{index}@duckmail.sbs.json"),
                "content": json!({
                    "type": "codex",
                    "email": format!("user-{index}@duckmail.sbs"),
                    "account_id": format!("acct_{index}"),
                    "expired": "2026-03-20T00:00:00.000Z",
                    "access_token": "access-token",
                    "refresh_token": "refresh-token",
                    "id_token": "header.payload.signature",
                    "padding": "x".repeat(padding_len),
                })
                .to_string(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "groupName": test_required_group_name(),
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "items": items,
    })
    .to_string()
}

pub(crate) fn build_large_imported_oauth_import_body(
    item_count: usize,
    padding_len: usize,
) -> String {
    let mut selected_source_ids = Vec::with_capacity(item_count);
    let items = (0..item_count)
        .map(|index| {
            let source_id = format!("source-{index}");
            selected_source_ids.push(source_id.clone());
            json!({
                "sourceId": source_id,
                "fileName": format!("user-{index}@duckmail.sbs.json"),
                "content": json!({
                    "type": "codex",
                    "email": format!("user-{index}@duckmail.sbs"),
                    "account_id": format!("acct_{index}"),
                    "expired": "2026-03-20T00:00:00.000Z",
                    "access_token": "access-token",
                    "refresh_token": "refresh-token",
                    "id_token": "header.payload.signature",
                    "padding": "x".repeat(padding_len),
                })
                .to_string(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "groupName": test_required_group_name(),
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "items": items,
        "selectedSourceIds": selected_source_ids,
        "tagIds": [],
    })
    .to_string()
}

pub(crate) async fn post_same_origin_json(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: String,
) -> reqwest::Response {
    let origin = format!("http://{addr}");
    client
        .post(format!("{origin}{path}"))
        .header(reqwest::header::ORIGIN, &origin)
        .header(reqwest::header::REFERER, format!("{origin}/"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .expect("request should succeed")
}

pub(crate) async fn delete_same_origin(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
) -> reqwest::Response {
    let origin = format!("http://{addr}");
    client
        .delete(format!("{origin}{path}"))
        .header(reqwest::header::ORIGIN, &origin)
        .header(reqwest::header::REFERER, format!("{origin}/"))
        .send()
        .await
        .expect("request should succeed")
}

pub(crate) fn parse_sse_frame(frame: &str) -> Option<(String, String)> {
    let mut event_name = None::<String>;
    let mut data_lines = Vec::new();
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim().to_string());
        }
    }
    match (event_name, data_lines.is_empty()) {
        (Some(event_name), false) => Some((event_name, data_lines.join("\n"))),
        _ => None,
    }
}

pub(crate) async fn collect_sse_events_until(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    terminal_event_names: &[&str],
    max_events: usize,
) -> Vec<(String, String)> {
    let response = client
        .get(format!("http://{addr}{path}"))
        .send()
        .await
        .expect("sse request should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.starts_with("text/event-stream")),
        Some(true)
    );

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut events = Vec::new();
    while events.len() < max_events {
        let next_chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("sse stream should produce data before timeout");
        let chunk = next_chunk
            .expect("sse stream should stay open")
            .expect("sse chunk should decode");
        buffer.push_str(
            &std::str::from_utf8(&chunk)
                .expect("sse chunk should be utf-8")
                .replace("\r\n", "\n"),
        );
        while let Some(split_index) = buffer.find("\n\n") {
            let frame = buffer[..split_index].to_string();
            buffer = buffer[(split_index + 2)..].to_string();
            if let Some(event) = parse_sse_frame(&frame) {
                let is_terminal = terminal_event_names
                    .iter()
                    .any(|candidate| *candidate == event.0);
                events.push(event);
                if is_terminal || events.len() >= max_events {
                    return events;
                }
            }
        }
    }
    events
}

#[tokio::test]
pub(crate) async fn imported_oauth_validate_route_accepts_large_request_body() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let client = reqwest::Client::new();
    let small_body = build_large_imported_oauth_validate_body(1, 64);
    let small_response = post_same_origin_json(
        &client,
        addr,
        "/api/pool/upstream-accounts/oauth/imports/validate",
        small_body,
    )
    .await;
    assert_eq!(small_response.status(), StatusCode::OK);

    let large_body = build_large_imported_oauth_validate_body(934, 4096);
    assert!(
        large_body.len() > 4_000_000,
        "expected test payload to exceed 4 MiB, got {} bytes",
        large_body.len()
    );
    let large_response = post_same_origin_json(
        &client,
        addr,
        "/api/pool/upstream-accounts/oauth/imports/validate",
        large_body,
    )
    .await;
    assert_eq!(large_response.status(), StatusCode::OK);
    let large_text = large_response.text().await.expect("read validate body");
    assert!(
        !large_text.contains("Failed to buffer the request body"),
        "validate route should no longer reject large import payloads before business handling"
    );
    assert!(large_text.contains("\"rows\""));

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
pub(crate) async fn imported_oauth_import_route_accepts_large_request_body() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let client = reqwest::Client::new();
    let large_body = build_large_imported_oauth_import_body(934, 4096);
    assert!(
        large_body.len() > 4_000_000,
        "expected test payload to exceed 4 MiB, got {} bytes",
        large_body.len()
    );
    let response = post_same_origin_json(
        &client,
        addr,
        "/api/pool/upstream-accounts/oauth/imports",
        large_body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let text = response.text().await.expect("read import body");
    assert!(
        !text.contains("Failed to buffer the request body"),
        "import route should no longer reject large import payloads before business handling"
    );
    assert!(text.contains("\"summary\""));

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
pub(crate) async fn imported_oauth_validation_job_stream_replays_snapshot_and_completed_terminal_event()
 {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let client = reqwest::Client::new();
    let create_response = post_same_origin_json(
        &client,
        addr,
        "/api/pool/upstream-accounts/oauth/imports/validation-jobs",
        json!({
            "groupName": test_required_group_name(),
            "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
            "items": [
                {
                    "sourceId": "invalid-source",
                    "fileName": "broken@duckmail.sbs.json",
                    "content": "{not-json",
                }
            ]
        })
        .to_string(),
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::OK);
    let created: Value = create_response
        .json()
        .await
        .expect("read create job payload");
    let job_id = created.get("jobId").and_then(Value::as_str).expect("jobId");
    assert_eq!(
        created
            .get("snapshot")
            .and_then(|snapshot| snapshot.get("rows"))
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .and_then(|row| row.get("status"))
            .and_then(Value::as_str),
        Some("pending")
    );

    let events = collect_sse_events_until(
        &client,
        addr,
        &format!("/api/pool/upstream-accounts/oauth/imports/validation-jobs/{job_id}/events"),
        &["completed", "failed", "cancelled"],
        4,
    )
    .await;
    assert_eq!(
        events.first().map(|event| event.0.as_str()),
        Some("snapshot")
    );
    assert_eq!(
        events.last().map(|event| event.0.as_str()),
        Some("completed")
    );
    let completed_payload: Value = serde_json::from_str(&events.last().expect("completed event").1)
        .expect("completed event should be valid json");
    assert_eq!(
        completed_payload
            .get("snapshot")
            .and_then(|snapshot| snapshot.get("rows"))
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .and_then(|row| row.get("status"))
            .and_then(Value::as_str),
        Some("invalid")
    );

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}
