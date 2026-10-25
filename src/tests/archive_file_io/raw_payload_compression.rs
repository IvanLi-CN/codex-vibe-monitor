use super::*;

#[tokio::test]
async fn retention_compresses_cold_raw_payloads_and_updates_paths() {
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
async fn retention_cold_compression_repair_keeps_relative_db_paths() {
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
async fn retention_skips_cold_compression_for_archive_eligible_rows() {
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
async fn retention_continues_when_one_cold_compression_file_fails() {
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
async fn retention_compresses_other_file_when_same_invocation_request_fails() {
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
async fn retention_dry_run_estimates_cold_raw_compression_without_mutating_files() {
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
async fn retention_cold_compression_scans_batches_in_occurred_at_order() {
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
async fn retention_cold_compression_budget_counts_missing_rows() {
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
async fn maintenance_raw_compression_cli_supports_dry_run_and_live_modes() {
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
async fn retention_archives_rows_with_compressed_raw_payload_files() {
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
