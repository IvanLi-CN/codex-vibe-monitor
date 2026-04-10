#[tokio::test]
async fn bootstrap_hourly_rollups_ignores_missing_invocation_archive_batch() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("hourly-rollup-missing-invocation-archive").await;
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
async fn bootstrap_hourly_rollups_ignores_missing_forward_proxy_archive_batch() {
    let (pool, _config, temp_dir) =
        retention_test_pool_and_config("hourly-rollup-missing-forward-proxy-archive").await;
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
async fn retention_compacts_old_quota_snapshots_by_shanghai_day() {
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
async fn retention_orphan_sweep_skips_fresh_raw_files() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-orphan-grace").await;
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
async fn retention_orphan_sweep_anchors_relative_raw_dir_to_database_parent() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().await;
    let temp_dir = make_temp_test_dir("retention-orphan-db-parent");
    let db_root = temp_dir.join("db-root");
    let cwd_root = temp_dir.join("cwd-root");
    fs::create_dir_all(&db_root).expect("create db root");
    fs::create_dir_all(&cwd_root).expect("create cwd root");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd_root);

    let db_path = db_root.join("codex-vibe-monitor.db");
    fs::File::create(&db_path).expect("create sqlite file");
    let pool = SqlitePool::connect(&sqlite_url_for_path(&db_path))
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
async fn retention_dry_run_does_not_mutate_database_or_files() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-dry-run").await;
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
async fn retention_compresses_cold_raw_payloads_and_updates_paths() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-cold-compress-live").await;
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
async fn retention_cold_compression_repair_keeps_relative_db_paths() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-cold-compress-relative-repair").await;
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
async fn retention_skips_cold_compression_for_archive_eligible_rows() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-cold-compress-skip-archive").await;
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
async fn retention_continues_when_one_cold_compression_file_fails() {
    use std::os::unix::fs::PermissionsExt;

    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-cold-compress-continue-on-error").await;
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
async fn retention_compresses_other_file_when_same_invocation_request_fails() {
    use std::os::unix::fs::PermissionsExt;

    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-cold-compress-same-row-continue-on-error").await;
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
async fn retention_dry_run_estimates_cold_raw_compression_without_mutating_files() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-cold-compress-dry-run").await;
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
async fn retention_cold_compression_scans_batches_in_occurred_at_order() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-cold-compress-order").await;
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
async fn maintenance_raw_compression_cli_supports_dry_run_and_live_modes() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("maintenance-raw-compression-cli").await;
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
async fn retention_archives_rows_with_compressed_raw_payload_files() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-archive-compressed-raw").await;
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

async fn spawn_test_crs_stats_server(
    release_request: Arc<Notify>,
    request_count: Arc<AtomicUsize>,
) -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        "/apiStats/api/user-model-stats",
        post(move || {
            let release_request = release_request.clone();
            let request_count = request_count.clone();
            async move {
                request_count.fetch_add(1, Ordering::SeqCst);
                release_request.notified().await;
                (
                    StatusCode::OK,
                    Json(json!({
                        "success": true,
                        "period": "daily",
                        "data": [],
                    })),
                )
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind crs stats test server");
    let addr = listener.local_addr().expect("crs stats test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("crs stats test server should run");
    });

    (format!("http://{addr}/"), handle)
}

#[cfg(unix)]
#[tokio::test]
async fn terminate_child_process_prefers_sigterm_when_process_exits_cleanly() {
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
async fn terminate_child_process_falls_back_to_force_kill_when_grace_period_is_exhausted() {
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
async fn spawn_http_server_leaves_health_unready_until_runtime_declares_readiness() {
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
async fn http_server_graceful_shutdown_stops_accepting_new_connections() {
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

fn build_large_imported_oauth_validate_body(item_count: usize, padding_len: usize) -> String {
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

fn build_large_imported_oauth_import_body(item_count: usize, padding_len: usize) -> String {
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

async fn post_same_origin_json(
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

async fn delete_same_origin(
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

fn parse_sse_frame(frame: &str) -> Option<(String, String)> {
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

async fn collect_sse_events_until(
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
async fn imported_oauth_validate_route_accepts_large_request_body() {
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
async fn imported_oauth_import_route_accepts_large_request_body() {
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
async fn imported_oauth_validation_job_stream_replays_snapshot_and_completed_terminal_event() {
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

#[tokio::test]
async fn imported_oauth_validation_job_delete_removes_completed_job() {
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

    let delete_response = delete_same_origin(
        &client,
        addr,
        &format!("/api/pool/upstream-accounts/oauth/imports/validation-jobs/{job_id}"),
    )
    .await;
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let response = client
        .get(format!(
            "http://{addr}/api/pool/upstream-accounts/oauth/imports/validation-jobs/{job_id}/events"
        ))
        .send()
        .await
        .expect("lookup request should succeed");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
async fn run_runtime_until_shutdown_waits_for_inflight_scheduler_poll() {
    let release_request = Arc::new(Notify::new());
    let request_count = Arc::new(AtomicUsize::new(0));
    let (crs_base, crs_handle) =
        spawn_test_crs_stats_server(release_request.clone(), request_count.clone()).await;

    let mut config = test_config();
    config.crs_stats = Some(CrsStatsConfig {
        base_url: Url::parse(&crs_base).expect("valid crs base url"),
        api_id: "test-api".to_string(),
        period: "daily".to_string(),
        poll_interval: Duration::from_secs(3600),
    });
    config.request_timeout = Duration::from_secs(5);
    config.poll_interval = Duration::from_millis(25);
    config.max_parallel_polls = 1;
    let state = test_state_from_config(config, false).await;

    let shutdown = Arc::new(Notify::new());
    let shutdown_for_runtime = shutdown.clone();
    let state_for_runtime = state.clone();
    let runtime_handle = tokio::spawn(async move {
        run_runtime_until_shutdown(state_for_runtime, Instant::now(), async move {
            shutdown_for_runtime.notified().await;
        })
        .await
    });

    tokio::time::timeout(Duration::from_secs(2), async {
        while request_count.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("scheduler should start an in-flight poll");
    shutdown.notify_waiters();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(
        !runtime_handle.is_finished(),
        "runtime should wait for the in-flight scheduler poll to finish"
    );
    assert_eq!(request_count.load(Ordering::SeqCst), 1);

    release_request.notify_waiters();
    runtime_handle
        .await
        .expect("runtime task should join")
        .expect("runtime should shutdown cleanly");

    assert!(state.shutdown.is_cancelled());
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    crs_handle.abort();
}

#[tokio::test]
async fn scheduler_does_not_start_a_new_poll_after_shutdown_while_waiting_for_permit() {
    let release_request = Arc::new(Notify::new());
    let request_count = Arc::new(AtomicUsize::new(0));
    let (crs_base, crs_handle) =
        spawn_test_crs_stats_server(release_request.clone(), request_count.clone()).await;

    let mut config = test_config();
    config.crs_stats = Some(CrsStatsConfig {
        base_url: Url::parse(&crs_base).expect("valid crs base url"),
        api_id: "test-api".to_string(),
        period: "daily".to_string(),
        poll_interval: Duration::from_secs(3600),
    });
    config.request_timeout = Duration::from_secs(5);
    config.poll_interval = Duration::from_millis(25);
    config.max_parallel_polls = 1;
    let state = test_state_from_config(config, false).await;

    let scheduler_handle = spawn_scheduler(state.clone(), state.shutdown.clone());
    tokio::time::timeout(Duration::from_secs(2), async {
        while request_count.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("scheduler should start its initial poll");

    tokio::time::sleep(Duration::from_millis(100)).await;
    state.shutdown.cancel();
    release_request.notify_waiters();

    tokio::time::timeout(Duration::from_secs(2), scheduler_handle)
        .await
        .expect("scheduler should drain promptly after shutdown")
        .expect("scheduler task should join cleanly");

    assert_eq!(
        request_count.load(Ordering::SeqCst),
        1,
        "shutdown should prevent a queued follow-up poll from starting once the permit is released"
    );
    crs_handle.abort();
}

#[tokio::test]
async fn drain_runtime_after_shutdown_waits_for_summary_quota_broadcast_workers() {
    let state = test_state_from_config(test_config(), false).await;
    let (started_tx_a, started_rx_a) = tokio::sync::oneshot::channel();
    let (started_tx_b, started_rx_b) = tokio::sync::oneshot::channel();
    let release_a = Arc::new(Notify::new());
    let release_b = Arc::new(Notify::new());
    let worker_a = tokio::spawn({
        let release_a = release_a.clone();
        async move {
            started_tx_a
                .send(())
                .expect("first broadcast worker should report when it starts");
            release_a.notified().await;
        }
    });
    let worker_b = tokio::spawn({
        let release_b = release_b.clone();
        async move {
            started_tx_b
                .send(())
                .expect("second broadcast worker should report when it starts");
            release_b.notified().await;
        }
    });
    {
        let mut guard = state.proxy_summary_quota_broadcast_handle.lock().await;
        guard.extend([worker_a, worker_b]);
    }

    let drain_handle = tokio::spawn({
        let state = state.clone();
        async move {
            drain_runtime_after_shutdown(state, None, None, None, None, None, None, None).await
        }
    });

    started_rx_a
        .await
        .expect("first broadcast worker should start before the drain waits on it");
    started_rx_b
        .await
        .expect("second broadcast worker should start before the drain waits on it");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !drain_handle.is_finished(),
        "runtime drain should wait for every tracked summary/quota broadcast worker"
    );

    release_a.notify_waiters();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !drain_handle.is_finished(),
        "runtime drain should keep waiting until the last tracked summary/quota broadcast worker exits"
    );

    release_b.notify_waiters();
    drain_handle
        .await
        .expect("drain task should join")
        .expect("runtime drain should finish once every broadcast worker does");
    assert!(
        state
            .proxy_summary_quota_broadcast_handle
            .lock()
            .await
            .is_empty(),
        "runtime drain should clear all tracked summary/quota broadcast workers"
    );
}

#[tokio::test]
async fn run_runtime_until_shutdown_exits_when_shutdown_token_is_cancelled_directly() {
    let state = test_state_from_config(test_config(), false).await;
    state.shutdown.cancel();

    tokio::time::timeout(
        Duration::from_secs(1),
        run_runtime_until_shutdown(state.clone(), Instant::now(), std::future::pending::<()>()),
    )
    .await
    .expect("direct shutdown token cancellation should not hang runtime drain")
    .expect("runtime should exit cleanly after direct shutdown token cancellation");

    assert!(state.shutdown.is_cancelled());
}

#[tokio::test]
async fn run_runtime_until_shutdown_skips_startup_work_when_shutdown_is_already_requested() {
    let request_count = Arc::new(AtomicUsize::new(0));
    let release_request = Arc::new(Notify::new());
    let (crs_base, crs_handle) =
        spawn_test_crs_stats_server(release_request.clone(), request_count.clone()).await;

    let mut config = test_config();
    config.crs_stats = Some(CrsStatsConfig {
        base_url: Url::parse(&crs_base).expect("valid crs base url"),
        api_id: "test-api".to_string(),
        period: "daily".to_string(),
        poll_interval: Duration::from_secs(3600),
    });
    config.request_timeout = Duration::from_secs(5);
    config.poll_interval = Duration::from_millis(25);
    config.max_parallel_polls = 1;
    let state = test_state_from_config(config, false).await;

    run_runtime_until_shutdown(state.clone(), Instant::now(), async {})
        .await
        .expect("runtime should exit cleanly when shutdown is already requested");

    assert!(state.shutdown.is_cancelled());
    assert_eq!(request_count.load(Ordering::SeqCst), 0);
    release_request.notify_waiters();
    crs_handle.abort();
}

#[tokio::test]
async fn run_runtime_until_shutdown_skips_xray_route_sync_when_shutdown_is_already_requested() {
    let runtime_dir = make_temp_test_dir("runtime-shutdown-xray-sync");
    fs::remove_dir_all(&runtime_dir).expect("remove temp runtime dir before startup");

    let mut config = test_config();
    config.xray_binary = "/path/to/non-existent-xray".to_string();
    config.xray_runtime_dir = runtime_dir.clone();
    let state = test_state_from_config(config, false).await;

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: vec!["vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#vless".to_string()],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        });
    }

    run_runtime_until_shutdown(state.clone(), Instant::now(), async {})
        .await
        .expect("runtime should exit cleanly when shutdown is already requested");

    assert!(state.shutdown.is_cancelled());
    assert!(
        !runtime_dir.exists(),
        "shutdown should skip xray route sync side effects when startup never begins"
    );
}

#[tokio::test]
async fn run_startup_stage_until_shutdown_skips_stage_when_shutdown_arrives_before_first_poll() {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::task::{Context, Poll};

    struct ReadyOnSecondPollFuture {
        polls: Arc<AtomicUsize>,
    }

    impl Future for ReadyOnSecondPollFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            let poll_count = self.polls.fetch_add(1, Ordering::SeqCst);
            if poll_count == 0 {
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        }
    }

    let shutdown_polls = Arc::new(AtomicUsize::new(0));
    let shutdown_signal = ReadyOnSecondPollFuture {
        polls: shutdown_polls.clone(),
    }
    .shared();
    let cancel = CancellationToken::new();
    let stage_started = Arc::new(AtomicBool::new(false));

    let outcome = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, {
        let stage_started = stage_started.clone();
        async move {
            stage_started.store(true, Ordering::SeqCst);
            13_u8
        }
    })
    .await;

    assert!(matches!(outcome, StartupStageOutcome::SkippedByShutdown));
    assert!(cancel.is_cancelled());
    assert!(
        !stage_started.load(Ordering::SeqCst),
        "shutdown should skip startup work that has not started polling yet"
    );
    assert_eq!(
        shutdown_polls.load(Ordering::SeqCst),
        2,
        "the shutdown future should only need the initial probe and the shutdown branch poll"
    );
}

#[tokio::test]
async fn run_startup_stage_until_shutdown_preserves_stage_result_when_shutdown_arrives_after_stage()
{
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll};

    struct FlagShutdownFuture {
        ready: Arc<AtomicBool>,
    }

    impl Future for FlagShutdownFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.ready.load(Ordering::SeqCst) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }

    let shutdown_ready = Arc::new(AtomicBool::new(false));
    let shutdown_signal = FlagShutdownFuture {
        ready: shutdown_ready.clone(),
    }
    .shared();
    let cancel = CancellationToken::new();

    let outcome = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        shutdown_ready.store(true, Ordering::SeqCst);
        42_u8
    })
    .await;

    match outcome {
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            assert_eq!(result, 42);
            assert!(shutdown_requested);
            assert!(cancel.is_cancelled());
        }
        StartupStageOutcome::SkippedByShutdown => {
            panic!("stage result should be preserved when shutdown arrives after stage completion")
        }
    }
}

#[tokio::test]
async fn run_startup_stage_until_shutdown_waits_for_stage_completion_when_shutdown_arrives_mid_stage()
 {
    let shutdown = Arc::new(Notify::new());
    let shutdown_signal = {
        let shutdown = shutdown.clone();
        async move {
            shutdown.notified().await;
        }
        .shared()
    };
    let cancel = CancellationToken::new();
    let release_stage = Arc::new(Notify::new());
    let (stage_started_tx, stage_started_rx) = tokio::sync::oneshot::channel();

    let shutdown_task = {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            stage_started_rx
                .await
                .expect("stage should signal when startup work begins");
            shutdown.notify_waiters();
        })
    };
    let release_task = {
        let cancel = cancel.clone();
        let release_stage = release_stage.clone();
        tokio::spawn(async move {
            cancel.cancelled().await;
            release_stage.notify_waiters();
        })
    };

    let outcome = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async move {
        stage_started_tx
            .send(())
            .expect("stage start signal should be sent exactly once");
        release_stage.notified().await;
        7_u8
    })
    .await;

    shutdown_task
        .await
        .expect("shutdown trigger task should finish");
    release_task
        .await
        .expect("stage release task should finish");

    match outcome {
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            assert_eq!(result, 7);
            assert!(shutdown_requested);
            assert!(cancel.is_cancelled());
        }
        StartupStageOutcome::SkippedByShutdown => {
            panic!("stage should finish after shutdown begins once startup work is already running")
        }
    }
}

#[tokio::test]
async fn bootstrap_probe_round_skips_work_when_shutdown_is_in_progress() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::OK).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize forward proxy url");
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize forward proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid upstream base url"),
    )
    .await;
    state.shutdown.cancel();

    spawn_forward_proxy_bootstrap_probe_round(
        state.clone(),
        vec![ForwardProxyEndpoint {
            key: proxy_key.clone(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            display_name: normalized_proxy.clone(),
            protocol: ForwardProxyProtocol::Http,
            endpoint_url: Some(Url::parse(&normalized_proxy).expect("valid normalized proxy url")),
            raw_url: Some(normalized_proxy.clone()),
        }],
        "test-shutdown",
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    let probe_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    assert_eq!(probe_count, 0);

    proxy_handle.abort();
}

#[tokio::test]
async fn forward_proxy_bootstrap_probe_round_stops_mid_probe_when_shutdown_begins() {
    let request_started = Arc::new(Notify::new());
    let release_request = Arc::new(Notify::new());
    let (proxy_url, proxy_handle) = spawn_test_blocking_forward_proxy_status(
        StatusCode::OK,
        request_started.clone(),
        release_request.clone(),
    )
    .await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize forward proxy url");
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize forward proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid upstream base url"),
    )
    .await;

    spawn_forward_proxy_bootstrap_probe_round(
        state.clone(),
        vec![ForwardProxyEndpoint {
            key: proxy_key.clone(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            display_name: normalized_proxy.clone(),
            protocol: ForwardProxyProtocol::Http,
            endpoint_url: Some(Url::parse(&normalized_proxy).expect("valid normalized proxy url")),
            raw_url: Some(normalized_proxy.clone()),
        }],
        "test-shutdown-mid-probe",
    );

    tokio::time::timeout(Duration::from_secs(1), request_started.notified())
        .await
        .expect("bootstrap probe should reach the forward proxy before shutdown");
    state.shutdown.cancel();
    release_request.notify_waiters();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await,
        0,
        "shutdown should stop an in-flight bootstrap probe without recording a probe attempt"
    );

    proxy_handle.abort();
}

#[tokio::test]
async fn finish_summary_quota_broadcast_idle_flushes_pending_tail_when_shutdown_arrives() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;
    let mut rx = state.broadcaster.subscribe();

    state
        .proxy_summary_quota_broadcast_seq
        .store(2, Ordering::Release);
    state
        .proxy_summary_quota_broadcast_running
        .store(true, Ordering::Release);
    state.shutdown.cancel();

    let should_continue = finish_summary_quota_broadcast_idle(
        SummaryQuotaBroadcastIdleContext {
            latest_broadcast_seq: state.proxy_summary_quota_broadcast_seq.as_ref(),
            broadcast_running: state.proxy_summary_quota_broadcast_running.as_ref(),
            shutdown: &state.shutdown,
            pool: &state.pool,
            broadcaster: &state.broadcaster,
            broadcast_state_cache: state.broadcast_state_cache.as_ref(),
            relay_config: state.config.crs_stats.as_ref(),
            invocation_max_days: state.config.invocation_max_days,
            invoke_id: "idle-shutdown-tail",
        },
        1,
    )
    .await;

    assert!(
        !should_continue,
        "shutdown tail should flush inline instead of trying to restart the broadcast worker"
    );
    assert!(
        !state
            .proxy_summary_quota_broadcast_running
            .load(Ordering::Acquire),
        "shutdown tail flush should leave the worker idle"
    );

    let mut saw_quota = false;
    let mut summary_windows = HashSet::new();
    let expected_summary_windows = summary_broadcast_specs().len();
    for _ in 0..8 {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for shutdown idle-tail broadcast event")
            .expect("broadcast channel should stay open");
        match payload {
            BroadcastPayload::Summary { window, .. } => {
                summary_windows.insert(window);
            }
            BroadcastPayload::Quota { snapshot } => {
                saw_quota = true;
                assert_eq!(snapshot.total_requests, 9);
            }
            BroadcastPayload::Records { .. }
            | BroadcastPayload::Version { .. }
            | BroadcastPayload::PoolAttempts { .. } => {}
        }

        if saw_quota && summary_windows.len() == expected_summary_windows {
            break;
        }
    }

    assert!(
        saw_quota,
        "shutdown idle-tail flush should emit the latest quota snapshot"
    );
    assert_eq!(
        summary_windows.len(),
        expected_summary_windows,
        "shutdown idle-tail flush should emit every summary window"
    );
}

#[tokio::test]
async fn persist_and_broadcast_proxy_capture_flushes_follow_up_when_shutdown_begins_after_record_event()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;

    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "shutdown-tail-broadcast";
    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &now_local),
    )
    .await
    .expect("persist proxy capture before shutdown");
    state.shutdown.cancel();

    let mut saw_record = false;
    let mut saw_quota = false;
    let mut summary_windows = HashSet::new();
    let expected_summary_windows = summary_broadcast_specs().len();
    for _ in 0..16 {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for shutdown tail broadcast event")
            .expect("broadcast channel should stay open");
        match payload {
            BroadcastPayload::Records { records } => {
                saw_record |= records
                    .into_iter()
                    .any(|record| record.invoke_id == invoke_id);
            }
            BroadcastPayload::Summary { window, .. } => {
                summary_windows.insert(window);
            }
            BroadcastPayload::Quota { snapshot } => {
                saw_quota = true;
                assert_eq!(snapshot.total_requests, 9);
            }
            BroadcastPayload::Version { .. } | BroadcastPayload::PoolAttempts { .. } => {}
        }

        if saw_record && saw_quota && summary_windows.len() == expected_summary_windows {
            break;
        }
    }

    assert!(
        saw_record,
        "shutdown tail path should still emit the persisted record"
    );
    assert!(
        saw_quota,
        "shutdown tail path should flush the latest quota snapshot"
    );
    assert_eq!(
        summary_windows.len(),
        expected_summary_windows,
        "shutdown tail path should flush every summary window"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !state
            .proxy_summary_quota_broadcast_running
            .load(Ordering::Acquire),
        "summary/quota broadcast worker should quiesce after flushing the shutdown tail"
    );
}

#[tokio::test]
async fn persist_and_broadcast_proxy_capture_skips_summary_worker_during_shutdown() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    state.shutdown.cancel();

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record("shutdown-broadcast", &format_utc_iso(Utc::now())),
    )
    .await
    .expect("persist proxy capture during shutdown");

    let payload = tokio::time::timeout(Duration::from_millis(100), rx.recv())
        .await
        .expect("shutdown path should still emit the persisted record")
        .expect("broadcast channel should stay open");
    assert!(
        matches!(payload, BroadcastPayload::Records { .. }),
        "shutdown path should keep the live record event aligned with persisted data"
    );
    assert!(
        !state
            .proxy_summary_quota_broadcast_running
            .load(Ordering::Acquire),
        "summary/quota broadcast worker should not stay active during shutdown"
    );
}

#[tokio::test]
async fn finalize_pool_upstream_request_attempt_updates_pending_row_in_place() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-update".to_string(),
        occurred_at: "2026-03-23 20:49:00".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-pending".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };

    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:00",
    )
    .await;
    let attempt_id = pending
        .attempt_id
        .expect("pending attempt should be inserted immediately");

    let pending_row = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>)>(
        r#"
        SELECT id, status, phase, finished_at
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("load pending attempt row");
    assert_eq!(pending_row.0, attempt_id);
    assert_eq!(pending_row.1, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        pending_row.2.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING)
    );
    assert_eq!(pending_row.3, None);

    finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        "2026-03-23 20:49:05",
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(StatusCode::OK),
        None,
        None,
        Some(42.5),
        Some(15.0),
        Some(188.4),
        Some("req_pool_123"),
        None,
        None,
    )
    .await
    .expect("finalize pending attempt");

    let rows = sqlx::query_as::<
        _,
        (
            i64,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<String>,
        ),
    >(
        r#"
        SELECT
            id,
            status,
            phase,
            http_status,
            finished_at,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
        ORDER BY id ASC
        "#,
    )
    .bind("pending-attempt-update")
    .fetch_all(&state.pool)
    .await
    .expect("load finalized attempt rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, attempt_id);
    assert_eq!(rows[0].1, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_eq!(
        rows[0].2.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
    );
    assert_eq!(rows[0].3, Some(200));
    assert_eq!(rows[0].4.as_deref(), Some("2026-03-23 20:49:05"));
    assert_eq!(rows[0].5, Some(42.5));
    assert_eq!(rows[0].6, Some(15.0));
    assert_eq!(rows[0].7, Some(188.4));
    assert_eq!(rows[0].8.as_deref(), Some("req_pool_123"));
}

#[tokio::test]
async fn broadcast_pool_upstream_attempts_snapshot_emits_pending_attempts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-broadcast".to_string(),
        occurred_at: "2026-03-23 20:49:02".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-broadcast".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let _pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:02",
    )
    .await;

    let mut rx = state.broadcaster.subscribe();
    broadcast_pool_upstream_attempts_snapshot(state.as_ref(), "pending-attempt-broadcast")
        .await
        .expect("broadcast pool attempt snapshot");

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for pool-attempt snapshot")
        .expect("broadcast channel should stay open");
    match payload {
        BroadcastPayload::PoolAttempts {
            invoke_id,
            attempts,
        } => {
            assert_eq!(invoke_id, "pending-attempt-broadcast");
            assert_eq!(attempts.len(), 1);
            assert_eq!(
                attempts[0].status,
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
            );
            assert_eq!(
                attempts[0].phase,
                POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING
            );
            assert_eq!(attempts[0].finished_at, None);
            assert_eq!(attempts[0].upstream_account_id, Some(account_id));
        }
        other => panic!("expected pool-attempts payload, got {other:?}"),
    }
}

#[tokio::test]
async fn advance_pool_upstream_request_attempt_phase_updates_and_broadcasts_snapshot() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-phase-advance".to_string(),
        occurred_at: "2026-03-23 20:49:06".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-phase-advance".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:06",
    )
    .await;

    let mut rx = state.broadcaster.subscribe();
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance phase and broadcast snapshot");

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for advanced phase snapshot")
        .expect("broadcast channel should stay open");
    match payload {
        BroadcastPayload::PoolAttempts {
            invoke_id,
            attempts,
        } => {
            assert_eq!(invoke_id, "pending-attempt-phase-advance");
            assert_eq!(attempts.len(), 1);
            assert_eq!(
                attempts[0].phase,
                POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST
            );
            assert_eq!(
                attempts[0].status,
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
            );
        }
        other => panic!("expected phase-advance pool-attempt snapshot, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_invocation_pool_attempts_returns_live_pending_attempts_without_parent_invocation() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-live-fetch".to_string(),
        occurred_at: "2026-03-23 20:49:02".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-live-fetch".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let _pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:02",
    )
    .await;

    let Json(attempts) = fetch_invocation_pool_attempts(
        State(state.clone()),
        axum::extract::Path("pending-attempt-live-fetch".to_string()),
    )
    .await
    .expect("fetch live pending attempts");

    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        attempts[0].phase,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING
    );
    assert_eq!(attempts[0].finished_at, None);
    assert_eq!(attempts[0].upstream_account_id, Some(account_id));
}

#[tokio::test]
async fn insert_and_broadcast_pool_upstream_terminal_attempt_emits_final_snapshot() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "terminal-attempt-broadcast".to_string(),
        occurred_at: "2026-03-23 20:49:03".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-terminal".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let final_error = PoolUpstreamError {
        account: None,
        status: StatusCode::BAD_GATEWAY,
        message: "pool budget exhausted after failover".to_string(),
        failure_kind: PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        upstream_request_id: Some("req_terminal_123".to_string()),
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            3,
            3,
            Some(PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    };

    let mut rx = state.broadcaster.subscribe();
    insert_and_broadcast_pool_upstream_terminal_attempt(
        state.as_ref(),
        &trace,
        &final_error,
        4,
        3,
        PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED,
    )
    .await
    .expect("insert and broadcast terminal attempt");

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for terminal pool-attempt snapshot")
        .expect("broadcast channel should stay open");
    match payload {
        BroadcastPayload::PoolAttempts {
            invoke_id,
            attempts,
        } => {
            assert_eq!(invoke_id, "terminal-attempt-broadcast");
            assert_eq!(attempts.len(), 1);
            assert_eq!(
                attempts[0].status,
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL
            );
            assert_eq!(
                attempts[0].phase,
                POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
            );
            assert_eq!(attempts[0].attempt_index, 4);
            assert_eq!(
                attempts[0].failure_kind.as_deref(),
                Some(PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED)
            );
            assert_eq!(
                attempts[0].upstream_request_id.as_deref(),
                Some("req_terminal_123")
            );
        }
        other => panic!("expected terminal pool-attempt snapshot, got {other:?}"),
    }
}

#[tokio::test]
async fn recover_orphaned_pool_upstream_request_attempts_marks_pending_rows_terminal() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-recovery".to_string(),
        occurred_at: "2026-03-23 20:49:04".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-recovery".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };

    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:04",
    )
    .await;

    let affected = recover_orphaned_pool_upstream_request_attempts(&state.pool)
        .await
        .expect("recover pending attempts");
    assert_eq!(affected, 1);

    let recovered_row = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
        SELECT status, phase, finished_at, failure_kind, error_message
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load recovered pending attempt");

    assert_eq!(
        recovered_row.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        recovered_row.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert!(
        recovered_row
            .2
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(
        recovered_row.3.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
    assert_eq!(
        recovered_row.4.as_deref(),
        Some(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
    );
}

#[tokio::test]
async fn recover_orphaned_pool_upstream_request_attempts_keeps_startup_sequence_recoverable() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "startup-recovered-pending-attempt";
    let occurred_at = "2026-03-23 20:49:14";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.26"),
        Some("sticky-startup-recovery"),
        Some("pck-startup-recovery"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-startup-recovery".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    let recovered_invocations = recover_orphaned_proxy_invocations(&state.pool)
        .await
        .expect("recover orphaned invocations first");
    assert_eq!(recovered_invocations, 1);

    let recovered_attempts = recover_orphaned_pool_upstream_request_attempts(&state.pool)
        .await
        .expect("recover orphaned pending attempts after invocation cleanup");
    assert_eq!(recovered_attempts, 1);

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load recovered pending attempt");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load startup-recovered invocation");

    assert_eq!(
        attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert_eq!(
        attempt.2.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
    assert_eq!(invocation.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        invocation.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );
}

#[tokio::test]
async fn recover_orphaned_pool_upstream_request_attempts_recovers_terminal_invocation_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "startup-skip-successful-invocation";
    let occurred_at = "2026-03-23 20:49:24";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.27"),
        Some("sticky-startup-success"),
        Some("pck-startup-success"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        15.0,
        120.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist success candidate invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-startup-success".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await
    .expect("advance attempt into waiting-first-byte");

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET status = 'success',
            error_message = NULL,
            failure_kind = NULL,
            failure_class = NULL,
            is_actionable = 0
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("finalize invocation before startup attempt recovery");

    let recovered_attempts = recover_orphaned_pool_upstream_request_attempts(&state.pool)
        .await
        .expect("recover startup attempt rows for terminal invocation");
    assert_eq!(recovered_attempts, 1);

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after skipped startup recovery");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load terminal invocation after startup attempt recovery");

    assert_eq!(
        attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert_eq!(
        attempt.2.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
    assert_eq!(invocation.0, "success");
    assert_eq!(invocation.1, None);
}

#[tokio::test]
async fn recover_orphaned_proxy_invocations_marks_running_rows_interrupted() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            raw_response,
            payload
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)
        "#,
    )
    .bind("recovered-running-invocation")
    .bind("2026-03-23 21:01:02")
    .bind(SOURCE_PROXY)
    .bind(INVOCATION_STATUS_RUNNING)
    .bind("{}")
    .bind("{\"endpoint\":\"/v1/responses\"}")
    .execute(&state.pool)
    .await
    .expect("insert running invocation");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            raw_response,
            payload
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)
        "#,
    )
    .bind("xy-running-invocation")
    .bind("2026-03-23 21:01:03")
    .bind(SOURCE_XY)
    .bind(INVOCATION_STATUS_RUNNING)
    .bind("{}")
    .bind("{\"endpoint\":\"/v1/chat/completions\"}")
    .execute(&state.pool)
    .await
    .expect("insert non-proxy running invocation");

    let affected = recover_orphaned_proxy_invocations(&state.pool)
        .await
        .expect("recover orphaned invocations");
    assert_eq!(affected, 1);

    let recovered = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        ),
    >(
        r#"
        SELECT status, error_message, failure_kind, failure_class, is_actionable
        FROM codex_invocations
        WHERE invoke_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind("recovered-running-invocation")
    .fetch_one(&state.pool)
    .await
    .expect("load recovered invocation");

    assert_eq!(recovered.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(recovered.1.as_deref(), Some(INVOCATION_INTERRUPTED_MESSAGE));
    assert_eq!(
        recovered.2.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );
    assert_eq!(recovered.3.as_deref(), Some(FAILURE_CLASS_SERVICE));
    assert_eq!(recovered.4, Some(1));

    let xy_row = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, error_message
        FROM codex_invocations
        WHERE invoke_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind("xy-running-invocation")
    .fetch_one(&state.pool)
    .await
    .expect("load non-proxy invocation");

    assert_eq!(xy_row.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(xy_row.1, None);
}

#[tokio::test]
async fn pool_early_phase_orphan_cleanup_guard_recovers_dropped_sending_request_attempt() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recovered-pending-attempt";
    let occurred_at = "2026-03-23 21:09:04";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.21"),
        Some("sticky-guard"),
        Some("pck-guard"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), false, false)
        .await
        .expect("recover dropped early-phase attempt");

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load recovered pending attempt");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load recovered invocation");

    assert_eq!(
        attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert_eq!(
        attempt.2.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
    assert_eq!(invocation.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        invocation.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_without_persisted_attempt_row_interrupts_invocation()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-skip-without-attempt-row";
    let occurred_at = "2026-03-23 21:09:05";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.23"),
        Some("sticky-guard-skip"),
        Some("pck-guard-skip"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let pending = PendingPoolAttemptRecord {
        attempt_id: None,
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-skip".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_account_id: account_id,
        upstream_route_key: "route-primary".to_string(),
        attempt_index: 1,
        distinct_account_index: 1,
        same_account_retry_index: 1,
        started_at: occurred_at.to_string(),
        connect_latency_ms: 0.0,
        first_byte_latency_ms: 0.0,
        compact_support_status: None,
        compact_support_reason: None,
    };

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending, false, false)
        .await
        .expect("recover dropped guard without persisted attempt row");

    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after dropped guard recovery");

    assert_eq!(invocation.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        invocation.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_skips_streaming_response_attempt_after_task_drop()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recover-streaming-response";
    let occurred_at = "2026-03-23 21:09:05";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.24"),
        Some("sticky-guard-streaming"),
        Some("pck-guard-streaming"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-streaming".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
    )
    .await
    .expect("advance attempt into streaming-response");

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), true, false)
        .await
        .expect("skip dropped streaming-response guard after task exit");

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load streaming attempt after dropped guard");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after dropped streaming guard");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_recovers_attempt_after_invocation_finalized()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-skip-finalized-invocation";
    let occurred_at = "2026-03-23 21:09:55";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.25"),
        Some("sticky-guard-final"),
        Some("pck-guard-final"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        5.0,
        120.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-final".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await
    .expect("advance attempt into waiting-first-byte");

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET status = 'success',
            error_message = NULL,
            failure_kind = NULL,
            failure_class = NULL,
            is_actionable = 0
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("finalize invocation before dropped guard recovery");

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), false, false)
        .await
        .expect("recover dropped guard attempt after finalized invocation");

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load finalized-invocation attempt after dropped guard");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load finalized invocation after dropped guard");

    assert_eq!(
        attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert_eq!(
        attempt.2.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
    assert_eq!(invocation.0, "success");
    assert_eq!(invocation.1, None);
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_skips_post_first_byte_terminal_attempt() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-skip-post-first-byte";
    let occurred_at = "2026-03-23 21:10:05";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.28"),
        Some("sticky-guard-post-first-byte"),
        Some("pck-guard-post-first-byte"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-post-first-byte".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await
    .expect("advance attempt into waiting-first-byte");

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), true, true)
        .await
        .expect("skip dropped guard recovery after first byte");

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load post-first-byte attempt after dropped guard");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after skipped post-first-byte guard");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_recovers_post_first_byte_nonterminal_attempt()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recover-post-first-byte-nonterminal";
    let occurred_at = "2026-03-23 21:10:06";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.29"),
        Some("sticky-guard-post-first-byte-recover"),
        Some("pck-guard-post-first-byte-recover"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-post-first-byte-recover".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await
    .expect("advance attempt into waiting-first-byte");

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), true, false)
        .await
        .expect("recover dropped guard after first byte before terminal outcome");

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load post-first-byte nonterminal attempt after dropped guard");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after recovering post-first-byte nonterminal guard");

    assert_eq!(
        attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert_eq!(
        attempt.2.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
    assert_eq!(invocation.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        invocation.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );
}

#[tokio::test]
async fn recover_guard_dropped_pool_terminal_invocation_orphan_repairs_running_invocation_after_attempt_already_finalized()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recover-invocation-after-final-attempt";
    let occurred_at = "2026-03-23 21:10:06";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.39"),
        Some("sticky-guard-final-attempt"),
        Some("pck-guard-final-attempt"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-final-attempt".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
    )
    .await
    .expect("advance attempt into streaming-response");
    finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        shanghai_now_string().as_str(),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(StatusCode::OK),
        None,
        None,
        Some(10.0),
        Some(20.0),
        Some(30.0),
        Some("upstream-req-1"),
        None,
        None,
    )
    .await
    .expect("persist terminal attempt before dropped guard recovery");

    recover_guard_dropped_pool_terminal_invocation_orphan(
        state.as_ref(),
        InvocationRecoverySelector::from(&pending),
    )
    .await
    .expect("recover running invocation after attempt already finalized");

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load terminal attempt after dropped guard recovery");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after repairing dropped guard");
    let route_state = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
        r#"
        SELECT last_route_failure_kind, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load route state after dropped guard repair");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        invocation.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );
    assert_eq!(route_state.0, None);
    assert_eq!(route_state.1, None);
    assert_eq!(route_state.2, 0);
}

#[tokio::test]
async fn pool_invocation_cleanup_guard_recovers_running_invocation_during_retry_backoff_gap() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-request-drop-backoff-gap";
    let occurred_at = "2026-03-23 21:10:06";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.40"),
        Some("sticky-request-drop-backoff"),
        Some("pck-request-drop-backoff"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-request-drop-backoff".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");
    finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        shanghai_now_string().as_str(),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        None,
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
        Some("failed to contact upstream: retryable backoff"),
        Some(10.0),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("persist retryable attempt before simulated backoff drop");

    {
        let _guard = PoolInvocationCleanupGuard::new(
            state.clone(),
            InvocationRecoverySelector::from(&pending),
            "request_drop_guard",
        );
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load finalized retryable attempt after request-drop guard");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after request-drop guard");

    assert_eq!(
        attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert_eq!(
        attempt.2.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    assert_eq!(invocation.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        invocation.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_rolls_back_attempt_when_invocation_recovery_fails()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-atomic-recovery-failure";
    let occurred_at = "2026-03-23 21:10:07";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.30"),
        Some("sticky-guard-atomic"),
        Some("pck-guard-atomic"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-atomic".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    sqlx::query(
        r#"
        CREATE TRIGGER fail_guard_atomic_invocation_recovery
        BEFORE UPDATE ON codex_invocations
        WHEN OLD.invoke_id = 'guard-atomic-recovery-failure'
        BEGIN
            SELECT RAISE(FAIL, 'simulated guard invocation recovery failure');
        END;
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("create failing guard recovery trigger");

    let err = recover_guard_dropped_pool_early_phase_orphan(
        state.as_ref(),
        pending.clone(),
        false,
        false,
    )
    .await
    .expect_err("guard recovery should fail when invocation update aborts");
    assert!(
        err.to_string()
            .contains("simulated guard invocation recovery failure"),
        "unexpected error: {err:?}"
    );

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after failed guard recovery");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after failed guard recovery");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_clears_pool_routing_reservation_and_records_route_failure()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-4242-1775776407000";
    let occurred_at = "2026-03-23 21:10:08";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.33"),
        Some("sticky-guard-reservation"),
        Some("pck-guard-reservation"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-reservation".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");
    let reservation_key = build_pool_routing_reservation_key(4242);
    reserve_test_pool_routing_account(&state, &reservation_key, account_id).await;

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending, false, false)
        .await
        .expect("recover dropped guard orphan and clean up route state");

    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    assert!(
        !reservations.contains_key(&reservation_key),
        "guard recovery should clear the in-memory reservation",
    );
    drop(reservations);

    let route_state = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
        r#"
        SELECT last_route_failure_kind, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load account route state after guard recovery");

    assert_eq!(
        route_state.0.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    assert_eq!(route_state.2, 1);
    assert!(
        route_state
            .1
            .as_deref()
            .is_some_and(|value| value.contains("drop_guard"))
    );
}

#[tokio::test]
async fn pool_early_phase_orphan_cleanup_guard_disarm_keeps_invocation_running_without_persisted_attempt_row()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-disarm-without-attempt-row";
    let occurred_at = "2026-03-23 21:09:06";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.24"),
        Some("sticky-guard-disarm"),
        Some("pck-guard-disarm"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let pending = PendingPoolAttemptRecord {
        attempt_id: None,
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-disarm".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_account_id: account_id,
        upstream_route_key: "route-primary".to_string(),
        attempt_index: 1,
        distinct_account_index: 1,
        same_account_retry_index: 1,
        started_at: occurred_at.to_string(),
        connect_latency_ms: 0.0,
        first_byte_latency_ms: 0.0,
        compact_support_status: None,
        compact_support_reason: None,
    };

    {
        let mut guard = PoolEarlyPhaseOrphanCleanupGuard::new(state.clone(), pending);
        guard.disarm();
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after disarmed guard drop");

    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
}

#[tokio::test]
async fn complete_deferred_pool_early_phase_cleanup_guard_marks_terminal_and_disarms() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let pending = PendingPoolAttemptRecord {
        attempt_id: Some(42),
        invoke_id: "guard-complete-terminal".to_string(),
        occurred_at: "2026-03-23 21:10:09".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-complete".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_account_id: 17,
        upstream_route_key: "route-primary".to_string(),
        attempt_index: 1,
        distinct_account_index: 1,
        same_account_retry_index: 1,
        started_at: "2026-03-23 21:10:09".to_string(),
        connect_latency_ms: 5.0,
        first_byte_latency_ms: 12.0,
        compact_support_status: None,
        compact_support_reason: None,
    };

    let mut guard = Some(PoolEarlyPhaseOrphanCleanupGuard::new(state, pending));
    guard
        .as_mut()
        .expect("guard should exist")
        .mark_first_byte_observed(12.0);

    complete_deferred_pool_early_phase_cleanup_guard(&mut guard);

    let guard = guard.expect("guard should still be present");
    assert!(guard.first_byte_observed);
    assert!(guard.terminal_outcome_observed);
    assert!(!guard.armed);
}

#[tokio::test]
async fn send_pool_request_with_failover_defers_armed_guard_when_pending_attempt_row_was_not_persisted()
 {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-6767-1775776407999";
    let occurred_at = "2026-03-23 21:10:09";
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-deferred".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let runtime_snapshot = PoolAttemptRuntimeSnapshotContext {
        capture_target: ProxyCaptureTarget::Responses,
        request_info: RequestCaptureInfo {
            model: Some("gpt-5.4".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        },
        prompt_cache_key: Some("pck-guard-deferred".to_string()),
        t_req_read_ms: 1.0,
        t_req_parse_ms: 1.0,
    };

    sqlx::query(
        r#"
        CREATE TRIGGER fail_pending_pool_attempt_insert
        BEFORE INSERT ON pool_upstream_request_attempts
        WHEN NEW.invoke_id = 'proxy-6767-1775776407999'
          AND NEW.status = 'pending'
        BEGIN
            SELECT RAISE(FAIL, 'simulated pending attempt insert failure');
        END;
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("create failing pending attempt insert trigger");

    let account = PoolResolvedAccount {
        account_id,
        display_name: "Primary".to_string(),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer primary".to_string(),
        },
        upstream_base_url: Url::parse(&upstream_base).expect("valid upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        group_upstream_429_retry_enabled: false,
        group_upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
    };

    let mut upstream = send_pool_request_with_failover(
        state.clone(),
        6767,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::Memory(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "hello"}]
            }))
            .expect("serialize request body"),
        ))),
        Duration::from_secs(5),
        Some(trace),
        Some(runtime_snapshot),
        Some("sticky-guard-deferred"),
        Some(account),
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect("request should still succeed without a persisted pending attempt row");

    assert_eq!(upstream.response.status(), StatusCode::OK);
    assert!(
        upstream
            .pending_attempt_record
            .as_ref()
            .is_some_and(|pending| pending.attempt_id.is_none())
    );
    assert!(
        upstream
            .deferred_early_phase_cleanup_guard
            .as_ref()
            .is_some_and(|guard| guard.first_byte_observed)
    );

    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after successful request without persisted attempt row");
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);

    disarm_pool_early_phase_cleanup_guard(&mut upstream.deferred_early_phase_cleanup_guard);
    upstream_handle.abort();
}

#[tokio::test]
async fn send_pool_request_with_failover_disarms_guard_after_streaming_phase_is_persisted() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-6768-1775776408000-persisted";
    let occurred_at = "2026-03-23 21:10:10";
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-streaming-phase-persisted".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let runtime_snapshot = PoolAttemptRuntimeSnapshotContext {
        capture_target: ProxyCaptureTarget::Responses,
        request_info: RequestCaptureInfo {
            model: Some("gpt-5.4".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        },
        prompt_cache_key: Some("pck-guard-streaming-phase-persisted".to_string()),
        t_req_read_ms: 1.0,
        t_req_parse_ms: 1.0,
    };

    let account = PoolResolvedAccount {
        account_id,
        display_name: "Primary".to_string(),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer primary".to_string(),
        },
        upstream_base_url: Url::parse(&upstream_base).expect("valid upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        group_upstream_429_retry_enabled: false,
        group_upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
    };

    let upstream = send_pool_request_with_failover(
        state.clone(),
        6768,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::Memory(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "hello"}]
            }))
            .expect("serialize request body"),
        ))),
        Duration::from_secs(5),
        Some(trace),
        Some(runtime_snapshot),
        Some("sticky-guard-streaming-phase-persisted"),
        Some(account),
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect("request should succeed when streaming-phase persistence succeeds");

    assert_eq!(upstream.response.status(), StatusCode::OK);
    assert!(
        upstream
            .pending_attempt_record
            .as_ref()
            .and_then(|pending| pending.attempt_id)
            .is_some()
    );
    assert!(upstream.deferred_early_phase_cleanup_guard.is_none());

    let attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after persisted streaming-phase update");
    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );
    upstream_handle.abort();
}

#[tokio::test]
async fn send_pool_request_with_failover_keeps_early_phase_guard_armed_when_streaming_phase_was_not_persisted()
 {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-6768-1775776408000";
    let occurred_at = "2026-03-23 21:10:10";
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-streaming-phase".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let runtime_snapshot = PoolAttemptRuntimeSnapshotContext {
        capture_target: ProxyCaptureTarget::Responses,
        request_info: RequestCaptureInfo {
            model: Some("gpt-5.4".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        },
        prompt_cache_key: Some("pck-guard-streaming-phase".to_string()),
        t_req_read_ms: 1.0,
        t_req_parse_ms: 1.0,
    };

    sqlx::query(
        r#"
        CREATE TRIGGER ignore_streaming_phase_update
        BEFORE UPDATE ON pool_upstream_request_attempts
        WHEN OLD.invoke_id = 'proxy-6768-1775776408000'
          AND NEW.phase = 'streaming_response'
        BEGIN
            SELECT RAISE(IGNORE);
        END;
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("create trigger that suppresses streaming-phase persistence");

    let account = PoolResolvedAccount {
        account_id,
        display_name: "Primary".to_string(),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer primary".to_string(),
        },
        upstream_base_url: Url::parse(&upstream_base).expect("valid upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        group_upstream_429_retry_enabled: false,
        group_upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
    };

    let upstream = send_pool_request_with_failover(
        state.clone(),
        6768,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::Memory(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "hello"}]
            }))
            .expect("serialize request body"),
        ))),
        Duration::from_secs(5),
        Some(trace),
        Some(runtime_snapshot),
        Some("sticky-guard-streaming-phase"),
        Some(account),
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect("request should still succeed when streaming-phase persistence is suppressed");

    assert_eq!(upstream.response.status(), StatusCode::OK);
    assert!(
        upstream
            .pending_attempt_record
            .as_ref()
            .and_then(|pending| pending.attempt_id)
            .is_some()
    );
    assert!(upstream.deferred_early_phase_cleanup_guard.is_some());

    drop(upstream);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after suppressed streaming-phase update");
    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE);
    assert_eq!(attempt.1.as_deref(), Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED));
    assert_eq!(attempt.2.as_deref(), Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED));

    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after suppressed streaming-phase update");
    assert_eq!(invocation.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        invocation.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn recover_proxy_invocations_with_selector_batches_handles_large_selector_sets() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let selector_count = PROXY_INVOCATION_RECOVERY_SELECTOR_BATCH_SIZE + 125;
    let mut selectors = Vec::with_capacity(selector_count);

    for index in 0..selector_count {
        let invoke_id = format!("selector-batch-invocation-{index}");
        let occurred_at = format!("2026-03-23 21:{:02}:{:02}", (index / 60) % 60, index % 60);
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                raw_response,
                payload
            )
            VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)
            "#,
        )
        .bind(&invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind(INVOCATION_STATUS_RUNNING)
        .bind("{}")
        .bind("{\"endpoint\":\"/v1/responses\"}")
        .execute(&state.pool)
        .await
        .expect("insert running proxy invocation");
        selectors.push(InvocationRecoverySelector::new(invoke_id, occurred_at));
    }

    let recovered = recover_proxy_invocations_with_scope(
        &state.pool,
        ProxyInvocationRecoveryScope::Selectors(&selectors),
    )
    .await
    .expect("recover large selector batch");

    assert_eq!(recovered.len(), selector_count);

    let interrupted_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE invoke_id LIKE 'selector-batch-invocation-%'
          AND status = ?1
          AND failure_kind = ?2
        "#,
    )
    .bind(INVOCATION_STATUS_INTERRUPTED)
    .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    .fetch_one(&state.pool)
    .await
    .expect("count interrupted selector rows");

    assert_eq!(interrupted_count as usize, selector_count);
}

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_only_recovers_stale_early_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let fresh_started = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    let cases = [
        (
            "stale-early-phase",
            "2026-03-23 21:10:01",
            stale_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
            false,
            0.0,
        ),
        (
            "fresh-early-phase",
            "2026-03-23 21:10:02",
            fresh_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
            false,
            0.0,
        ),
        (
            "stale-early-phase-post-first-byte",
            "2026-03-23 21:10:25",
            stale_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
            false,
            120.0,
        ),
        (
            "stale-early-phase-attempt-first-byte-progress",
            "2026-03-23 21:10:26",
            stale_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
            true,
            0.0,
        ),
        (
            "stale-streaming-phase",
            "2026-03-23 21:10:03",
            stale_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
            false,
            120.0,
        ),
    ];

    for (invoke_id, occurred_at, started_at, phase, persist_attempt_first_byte_progress, ttfb_ms) in
        cases
    {
        let running_record = build_running_proxy_capture_record(
            invoke_id,
            occurred_at,
            ProxyCaptureTarget::Responses,
            &request_info,
            Some("198.51.100.22"),
            Some("sticky-sweeper"),
            None,
            true,
            Some(account_id),
            Some("Primary"),
            Some("api_key_codex"),
            Some("api.openai.com"),
            None,
            Some(1),
            Some(1),
            None,
            None,
            10.0,
            2.0,
            5.0,
            ttfb_ms,
        );
        persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
            .await
            .expect("persist running invocation");
        let trace = PoolUpstreamAttemptTraceContext {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some("sticky-sweeper".to_string()),
            requester_ip: Some("192.168.31.6".to_string()),
        };
        let pending = begin_pool_upstream_request_attempt(
            &state.pool,
            &trace,
            account_id,
            "route-primary",
            1,
            1,
            1,
            started_at,
        )
        .await;
        advance_pool_upstream_request_attempt_phase(state.as_ref(), &pending, phase)
            .await
            .expect("advance attempt into target phase");
        if persist_attempt_first_byte_progress {
            sqlx::query(
                r#"
                UPDATE pool_upstream_request_attempts
                SET first_byte_latency_ms = 120.0
                WHERE id = ?1
                "#,
            )
            .bind(pending.attempt_id.expect("pending attempt id"))
            .execute(&state.pool)
            .await
            .expect("persist attempt first-byte progress");
        }
    }

    let post_first_byte_ttfb = sqlx::query_scalar::<_, Option<f64>>(
        r#"
        SELECT t_upstream_ttfb_ms
        FROM codex_invocations
        WHERE invoke_id = 'stale-early-phase-post-first-byte'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load stale post-first-byte invocation ttfb");
    assert_eq!(post_first_byte_ttfb, Some(120.0));

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("recover stale early-phase orphans");
    assert_eq!(
        outcome,
        PoolOrphanRecoveryOutcome {
            recovered_attempts: 1,
            recovered_invocations: 1,
        }
    );

    let stale_attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = 'stale-early-phase'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load stale recovered attempt");
    assert_eq!(
        stale_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        stale_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );

    let stale_invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = 'stale-early-phase'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load stale recovered invocation");
    assert_eq!(stale_invocation.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        stale_invocation.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );

    let stale_post_first_byte_attempt = sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
        r#"
            SELECT status, phase, first_byte_latency_ms
            FROM pool_upstream_request_attempts
            WHERE invoke_id = 'stale-early-phase-post-first-byte'
            ORDER BY id DESC
            LIMIT 1
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load stale post-first-byte attempt");
    assert_eq!(
        stale_post_first_byte_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        stale_post_first_byte_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(stale_post_first_byte_attempt.2, None);

    let stale_post_first_byte_invocation =
        sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
            r#"
            SELECT status, failure_kind, t_upstream_ttfb_ms
            FROM codex_invocations
            WHERE invoke_id = 'stale-early-phase-post-first-byte'
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_one(&state.pool)
        .await
        .expect("load stale post-first-byte invocation");
    assert_eq!(
        stale_post_first_byte_invocation.0,
        INVOCATION_STATUS_RUNNING
    );
    assert_eq!(stale_post_first_byte_invocation.1, None);
    assert_eq!(stale_post_first_byte_invocation.2, Some(120.0));

    let stale_attempt_first_byte_progress_attempt =
        sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
            r#"
            SELECT status, phase, first_byte_latency_ms
            FROM pool_upstream_request_attempts
            WHERE invoke_id = 'stale-early-phase-attempt-first-byte-progress'
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_one(&state.pool)
        .await
        .expect("load stale attempt-first-byte-progress attempt");
    assert_eq!(
        stale_attempt_first_byte_progress_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        stale_attempt_first_byte_progress_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(stale_attempt_first_byte_progress_attempt.2, Some(120.0));

    let stale_attempt_first_byte_progress_invocation =
        sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
            r#"
            SELECT status, failure_kind, t_upstream_ttfb_ms
            FROM codex_invocations
            WHERE invoke_id = 'stale-early-phase-attempt-first-byte-progress'
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_one(&state.pool)
        .await
        .expect("load stale attempt-first-byte-progress invocation");
    assert_eq!(
        stale_attempt_first_byte_progress_invocation.0,
        INVOCATION_STATUS_RUNNING
    );
    assert_eq!(stale_attempt_first_byte_progress_invocation.1, None);

    let fresh_attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = 'fresh-early-phase'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load fresh attempt");
    assert_eq!(
        fresh_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        fresh_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );

    let streaming_attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = 'stale-streaming-phase'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load stale streaming attempt");
    assert_eq!(
        streaming_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        streaming_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );

    let streaming_invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = 'stale-streaming-phase'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load stale streaming invocation");
    assert_eq!(streaming_invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(streaming_invocation.1, None);
}

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_skips_active_live_attempts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "stale-active-live-attempt";
    let occurred_at = "2026-03-23 21:10:30";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.23"),
        Some("sticky-active-live-attempt"),
        None,
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-active-live-attempt".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        stale_started.as_str(),
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    let _live_attempt_activity_lease = PoolLiveAttemptActivityLease::new(
        state.clone(),
        pending.attempt_id.expect("pending attempt id"),
    );

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("skip active live attempt during stale recovery");
    assert_eq!(outcome, PoolOrphanRecoveryOutcome::default());

    let attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load active attempt after stale recovery");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load active invocation after stale recovery");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
}

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_rolls_back_attempts_when_invocation_recovery_fails()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "stale-sweeper-atomic-recovery-failure";
    let occurred_at = "2026-03-23 21:10:31";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.31"),
        Some("sticky-sweeper-atomic"),
        None,
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        5.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-atomic".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        &stale_started,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    sqlx::query(
        r#"
        CREATE TRIGGER fail_stale_sweeper_invocation_recovery
        BEFORE UPDATE ON codex_invocations
        WHEN OLD.invoke_id = 'stale-sweeper-atomic-recovery-failure'
        BEGIN
            SELECT RAISE(FAIL, 'simulated stale sweeper invocation recovery failure');
        END;
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("create failing stale sweeper trigger");

    let err = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect_err("stale sweeper should fail when invocation recovery aborts");
    assert!(
        err.to_string()
            .contains("simulated stale sweeper invocation recovery failure"),
        "unexpected error: {err:?}"
    );

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after failed stale sweeper");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after failed stale sweeper");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
}

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_recovers_stale_attempts_after_invocation_finished()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "stale-sweeper-finalized-invocation";
    let occurred_at = "2026-03-23 21:10:31";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.32"),
        Some("sticky-sweeper-finalized"),
        None,
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        5.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-finalized".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        &stale_started,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET status = 'success',
            error_message = NULL,
            failure_kind = NULL,
            failure_class = NULL,
            is_actionable = 0
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("finalize invocation before stale recovery");

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("recover stale attempt after invocation already finished");
    assert_eq!(
        outcome,
        PoolOrphanRecoveryOutcome {
            recovered_attempts: 1,
            recovered_invocations: 0,
        }
    );

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after finalized-invocation stale recovery");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load finalized invocation after stale recovery");
    let route_state = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
        r#"
        SELECT last_route_failure_kind, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load route state after finalized-invocation stale recovery");

    assert_eq!(
        attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert_eq!(
        attempt.2.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
    assert_eq!(invocation.0, "success");
    assert_eq!(invocation.1, None);
    assert_eq!(route_state.0, None);
    assert_eq!(route_state.1, None);
    assert_eq!(route_state.2, 0);
}

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_clears_pool_routing_reservation_and_records_route_failure()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-5252-1775776407001";
    let occurred_at = "2026-03-23 21:10:32";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.34"),
        Some("sticky-sweeper-reservation"),
        Some("pck-sweeper-reservation"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        5.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-reservation".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        &stale_started,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");
    let reservation_key = build_pool_routing_reservation_key(5252);
    reserve_test_pool_routing_account(&state, &reservation_key, account_id).await;

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("recover stale early-phase orphan and clean up route state");
    assert_eq!(
        outcome,
        PoolOrphanRecoveryOutcome {
            recovered_attempts: 1,
            recovered_invocations: 1,
        }
    );

    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    assert!(
        !reservations.contains_key(&reservation_key),
        "runtime sweeper should clear the in-memory reservation",
    );
    drop(reservations);

    let route_state = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
        r#"
        SELECT last_route_failure_kind, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load account route state after stale sweeper");

    assert_eq!(
        route_state.0.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    assert_eq!(route_state.2, 1);
    assert!(
        route_state
            .1
            .as_deref()
            .is_some_and(|value| value.contains("runtime_sweeper"))
    );
}

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_records_route_failures_for_every_recovered_attempt()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let first_account_id =
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let second_account_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    let invoke_id = "stale-multi-attempt-route-failures";
    let occurred_at = "2026-03-23 21:10:33";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.35"),
        Some("sticky-sweeper-multi-attempt"),
        Some("pck-sweeper-multi-attempt"),
        true,
        Some(first_account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(2),
        Some(2),
        None,
        None,
        10.0,
        2.0,
        5.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let first_pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &PoolUpstreamAttemptTraceContext {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some("sticky-sweeper-multi-attempt".to_string()),
            requester_ip: Some("192.168.31.6".to_string()),
        },
        first_account_id,
        "route-primary",
        1,
        1,
        1,
        &stale_started,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &first_pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance first attempt into sending-request");

    let second_pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &PoolUpstreamAttemptTraceContext {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some("sticky-sweeper-multi-attempt".to_string()),
            requester_ip: Some("192.168.31.6".to_string()),
        },
        second_account_id,
        "route-secondary",
        2,
        2,
        1,
        &stale_started,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &second_pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance second attempt into sending-request");

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("recover stale attempts across multiple accounts");
    assert_eq!(
        outcome,
        PoolOrphanRecoveryOutcome {
            recovered_attempts: 2,
            recovered_invocations: 1,
        }
    );

    let first_route_state = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
        r#"
        SELECT last_route_failure_kind, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(first_account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load first account route state after stale recovery");
    let second_route_state = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
        r#"
        SELECT last_route_failure_kind, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(second_account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load second account route state after stale recovery");

    for route_state in [&first_route_state, &second_route_state] {
        assert_eq!(
            route_state.0.as_deref(),
            Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
        );
        assert_eq!(route_state.2, 1);
        assert!(
            route_state
                .1
                .as_deref()
                .is_some_and(|value| value.contains("runtime_sweeper"))
        );
    }
}

#[tokio::test]
async fn recover_stale_pool_upstream_request_attempt_candidates_rechecks_phase_before_update() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "stale-race-phase-recheck";
    let occurred_at = "2026-03-23 21:10:35";
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.29"),
        Some("sticky-sweeper-race"),
        None,
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        5.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-race".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        stale_started.as_str(),
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    let attempt_id = pending.attempt_id.expect("pending attempt id");
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
    )
    .await
    .expect("move attempt out of early phase before guarded update");

    let finished_at = shanghai_now_string();
    let recovered = recover_stale_pool_upstream_request_attempt_candidates(
        &state.pool,
        &[attempt_id],
        finished_at.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
    )
    .await
    .expect("guarded candidate update should re-check current phase");
    assert!(recovered.is_empty());

    let attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after guarded stale update");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );
}

#[tokio::test]
async fn recover_stale_pool_upstream_request_attempt_candidates_rechecks_attempt_first_byte_progress_before_update()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "stale-race-attempt-first-byte-recheck";
    let occurred_at = "2026-03-23 21:10:36";
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.30"),
        Some("sticky-sweeper-race-attempt-progress"),
        None,
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        5.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-race-attempt-progress".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        stale_started.as_str(),
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await
    .expect("advance attempt into waiting-first-byte");

    let attempt_id = pending.attempt_id.expect("pending attempt id");
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET first_byte_latency_ms = 120.0
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .execute(&state.pool)
    .await
    .expect("persist attempt first-byte progress before guarded update");

    let finished_at = shanghai_now_string();
    let recovered = recover_stale_pool_upstream_request_attempt_candidates(
        &state.pool,
        &[attempt_id],
        finished_at.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
    )
    .await
    .expect("guarded candidate update should re-check attempt first-byte progress");
    assert!(recovered.is_empty());

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
        r#"
        SELECT status, phase, first_byte_latency_ms
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after guarded stale update");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(attempt.2, Some(120.0));
}

#[tokio::test]
async fn recover_stale_pool_upstream_request_attempt_candidates_rechecks_invocation_first_byte_progress_before_update()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "stale-race-invocation-first-byte-recheck";
    let occurred_at = "2026-03-23 21:10:37";
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.31"),
        Some("sticky-sweeper-race-invocation-progress"),
        None,
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        5.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-race-invocation-progress".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        stale_started.as_str(),
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await
    .expect("advance attempt into waiting-first-byte");

    let attempt_id = pending.attempt_id.expect("pending attempt id");
    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET t_upstream_ttfb_ms = 120.0
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("persist invocation first-byte progress before guarded update");

    let finished_at = shanghai_now_string();
    let recovered = recover_stale_pool_upstream_request_attempt_candidates(
        &state.pool,
        &[attempt_id],
        finished_at.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
    )
    .await
    .expect("guarded candidate update should re-check invocation first-byte progress");
    assert!(recovered.is_empty());

    let attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after guarded stale update");
    let invocation = sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
        r#"
        SELECT status, failure_kind, t_upstream_ttfb_ms
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load invocation after guarded stale update");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
    assert_eq!(invocation.2, Some(120.0));
}

#[tokio::test]
async fn recover_stale_pool_upstream_request_attempt_candidates_batches_large_candidate_sets() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let selector_count = POOL_ATTEMPT_RECOVERY_SELECTOR_BATCH_SIZE + 125;
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut candidate_ids = Vec::with_capacity(selector_count);

    for index in 0..selector_count {
        let trace = PoolUpstreamAttemptTraceContext {
            invoke_id: format!("stale-candidate-batch-{index}"),
            occurred_at: format!("2026-03-23 22:{:02}:{:02}", (index / 60) % 60, index % 60),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some(format!("sticky-candidate-batch-{index}")),
            requester_ip: Some("192.168.31.6".to_string()),
        };
        let pending = begin_pool_upstream_request_attempt(
            &state.pool,
            &trace,
            account_id,
            "route-primary",
            1,
            1,
            1,
            &stale_started,
        )
        .await;
        advance_pool_upstream_request_attempt_phase(
            state.as_ref(),
            &pending,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
        )
        .await
        .expect("advance candidate attempt into sending-request");
        candidate_ids.push(pending.attempt_id.expect("pending attempt id"));
    }

    let finished_at = shanghai_now_string();
    let recovered = recover_stale_pool_upstream_request_attempt_candidates(
        &state.pool,
        &candidate_ids,
        finished_at.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
    )
    .await
    .expect("recover stale attempt candidates across multiple chunks");

    assert_eq!(recovered.len(), selector_count);

    let recovered_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE invoke_id LIKE 'stale-candidate-batch-%'
          AND status = ?1
          AND failure_kind = ?2
        "#,
    )
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
    .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    .fetch_one(&state.pool)
    .await
    .expect("count recovered stale attempt rows");

    assert_eq!(recovered_count as usize, selector_count);
}
