use super::*;

#[tokio::test]
async fn raw_compression_budget_stops_after_first_batch_when_budget_is_exhausted() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-catchup-budget").await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;
    config.retention_batch_rows = 1;

    for (invoke_id, hour, file_name) in [
        ("budget-oldest", 8, "budget-oldest.bin"),
        ("budget-middle", 9, "budget-middle.bin"),
        ("budget-newest", 10, "budget-newest.bin"),
    ] {
        let raw_path = config.proxy_raw_dir.join(file_name);
        fs::write(&raw_path, invoke_id.as_bytes()).expect("write budget raw file");
        insert_retention_invocation(
            &pool,
            invoke_id,
            &shanghai_local_days_ago(2, hour, 0, 0),
            SOURCE_PROXY,
            "failed",
            Some("{\"endpoint\":\"/v1/responses\"}"),
            "{\"ok\":false}",
            Some(&raw_path),
            None,
            Some(10),
            Some(0.01),
        )
        .await;
    }

    let first_pass = compress_cold_proxy_raw_payloads_with_budget(
        &pool,
        &config,
        config.database_path.parent(),
        false,
        Some(Duration::ZERO),
    )
    .await
    .expect("run raw compression with zero catchup budget");
    assert_eq!(first_pass.files_considered, 1);
    assert_eq!(first_pass.files_compressed, 1);

    let remaining_after_first: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE request_raw_codec = 'identity'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining raw backlog after first pass");
    assert_eq!(remaining_after_first, 2);

    let catchup = compress_cold_proxy_raw_payloads_with_budget(
        &pool,
        &config,
        config.database_path.parent(),
        false,
        None,
    )
    .await
    .expect("run unrestricted raw compression catchup");
    assert_eq!(catchup.files_considered, 2);
    assert_eq!(catchup.files_compressed, 2);

    let remaining_after_catchup: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE request_raw_codec = 'identity'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining raw backlog after catchup");
    assert_eq!(remaining_after_catchup, 0);

    cleanup_temp_test_dir(&temp_dir);
}
