use super::*;

#[tokio::test]
async fn shared_raw_path_replacement_updates_all_owners_in_bounded_batches() {
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
async fn shared_raw_blob_keeps_file_reference_until_last_owner_is_removed() {
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
async fn retention_finalization_keeps_compression_variant_with_another_owner() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("retention-shared-compression-variant").await;
    let compressed_path = config.proxy_raw_dir.join("shared-response.bin.gz");
    let identity_db_path = Path::new("proxy_raw_payloads/shared-response.bin");
    let identity_path = config
        .database_path
        .parent()
        .expect("database parent")
        .join(identity_db_path);
    fs::write(&compressed_path, b"compressed-owner").expect("write compressed raw");
    fs::write(&identity_path, b"identity-owner").expect("write identity raw");

    insert_retention_invocation(
        &pool,
        "retention-compressed-owner",
        &shanghai_local_days_ago(91, 12, 0, 0),
        SOURCE_PROXY,
        "success",
        None,
        "{}",
        None,
        Some(&compressed_path),
        Some(1),
        Some(0.0),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "retention-identity-owner",
        &shanghai_local_days_ago(1, 12, 0, 0),
        SOURCE_PROXY,
        "success",
        None,
        "{}",
        None,
        Some(identity_db_path),
        Some(1),
        Some(0.0),
    )
    .await;

    let archived = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("archive old compressed owner");
    assert_eq!(archived.0, 1);
    assert!(
        compressed_path.exists(),
        "the archived path remains while its alternate variant is still owned"
    );
    assert!(
        identity_path.exists(),
        "the alternate path remains owned by the recent invocation"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn schema_backfill_links_existing_pool_attempt_response_raw() {
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
async fn cold_compression_updates_every_shared_raw_blob_owner_before_removing_old_file() {
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
async fn bootstrap_hourly_rollups_ignores_missing_invocation_archive_batch() {
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
async fn bootstrap_hourly_rollups_ignores_missing_forward_proxy_archive_batch() {
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
