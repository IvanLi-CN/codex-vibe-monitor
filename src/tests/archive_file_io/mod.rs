#[allow(unused_imports)]
use super::*;

pub(crate) use super::*;

mod archive_backfill_and_materialization;
mod gpt6_cache_write_migration;
mod maintenance_runtime_active_io;
mod maintenance_runtime_ownership;
mod prompt_cache_control_file_lock;
mod raw_compression_budget;
#[expect(
    clippy::await_holding_lock,
    reason = "Mock reservation logs intentionally stay locked until async assertions observe requests."
)]
mod raw_payload_retention_and_compression;
mod retention_batch_wait;
mod retention_capacity_benchmark;
mod retention_service_rate_benchmark;
mod retention_task_local_batches;
mod retention_task_work_ownership;
mod system_storage_measurement;

#[tokio::test]
async fn bounded_summary_archive_repair_preserves_same_bucket_totals_across_passes() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("summary-rollup-bounded-shared-bucket").await;
    let occurred_at = "2026-01-15 08:10:00";
    let archive_count = 129_i64;
    let mut archive_paths = Vec::new();

    for id in 1..=archive_count {
        let invoke_id = format!("summary-rollup-bounded-shared-bucket-{id}");
        archive_paths.push(
            seed_invocation_archive_batch_with_details(
                &pool,
                &config,
                &format!("summary-rollup-bounded-shared-bucket-{id}"),
                &[SeedInvocationArchiveBatchRow {
                    id,
                    invoke_id: &invoke_id,
                    occurred_at,
                    source: SOURCE_PROXY,
                    status: "success",
                    total_tokens: id,
                    cost: id as f64 / 100.0,
                    ttfb_ms: Some(100.0),
                    payload: Some("{}"),
                    detail_level: DETAIL_LEVEL_FULL,
                    error_message: None,
                    failure_kind: None,
                    failure_class: Some("none"),
                    is_actionable: Some(0),
                }],
            )
            .await,
        );
    }

    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("run first bounded Summary archive repair pass");
    let first_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay \
         WHERE target = ?1 AND dataset = 'codex_invocations'",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .fetch_one(&pool)
    .await
    .expect("count first bounded Summary archive repair markers");
    assert_eq!(first_marker_count, 128);

    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("run second bounded Summary archive repair pass");

    let bucket_start_epoch = invocation_bucket_start_epoch(occurred_at)
        .expect("derive shared Summary archive rollup bucket");
    let totals = sqlx::query_as::<_, (i64, i64, f64)>(
        "SELECT total_count, total_tokens, total_cost \
         FROM invocation_rollup_hourly \
         WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load shared-bucket Summary archive totals");
    assert_eq!(totals.0, archive_count);
    assert_eq!(totals.1, (1..=archive_count).sum::<i64>());
    assert_f64_close(
        totals.2,
        (1..=archive_count).map(|id| id as f64 / 100.0).sum(),
    );

    let marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay \
         WHERE target = ?1 AND dataset = 'codex_invocations'",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .fetch_one(&pool)
    .await
    .expect("count complete Summary archive repair markers");
    assert_eq!(marker_count, archive_count);

    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = 'stale-summary-sha' \
         WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(archive_paths[0].to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("seed stale marker beyond the bounded additive page");
    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("full Summary repair should converge beyond the additive page budget");

    let repaired_totals = sqlx::query_as::<_, (i64, i64, f64)>(
        "SELECT total_count, total_tokens, total_cost \
         FROM invocation_rollup_hourly \
         WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load totals after stale-marker full repair");
    assert_eq!(repaired_totals.0, archive_count);
    assert_eq!(repaired_totals.1, (1..=archive_count).sum::<i64>());
    assert_f64_close(
        repaired_totals.2,
        (1..=archive_count).map(|id| id as f64 / 100.0).sum(),
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_rollup_additive_repair_reuses_durable_seen_ids_after_page_commit() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("summary-rollup-additive-durable-seen-ids").await;
    let occurred_at = "2026-01-15 08:10:00";
    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "summary-rollup-additive-durable-seen-ids",
        &[(
            1_i64,
            "summary-rollup-additive-durable-seen-row",
            occurred_at,
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("seed the initial additive Summary replay");

    let file_path = archive_path.to_string_lossy().to_string();
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&file_path)
    .execute(&pool)
    .await
    .expect("remove replay markers to reopen additive repair");
    sqlx::query(
        "INSERT INTO hourly_rollup_repair_seen_invocation_ids (dataset, invocation_id) VALUES (?1, ?2)",
    )
    .bind("codex_invocations_summary_rollup_v2_seen_ids")
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("seed durable seen ID as if the prior page committed before a crash");

    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("reopen additive Summary repair from durable seen IDs");

    let bucket_start_epoch =
        invocation_bucket_start_epoch(occurred_at).expect("derive durable seen ID bucket");
    let total: i64 = sqlx::query_scalar(
        "SELECT total_count FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load additive Summary total after durable seen ID replay");
    assert_eq!(total, 1);
    let seen_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_repair_seen_invocation_ids WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2_seen_ids")
    .fetch_one(&pool)
    .await
    .expect("load durable seen IDs after marker commit");
    assert_eq!(seen_count, 0);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_rollup_repair_treats_null_and_blank_replay_sha_as_unknown() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("summary-rollup-unknown-replay-sha").await;
    let occurred_at = "2026-01-15 08:10:00";
    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "summary-rollup-unknown-replay-sha",
        &[(
            1_i64,
            "summary-rollup-unknown-replay-sha-row",
            occurred_at,
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    let file_path = archive_path.to_string_lossy().to_string();
    let archive_sha: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&file_path)
    .fetch_one(&pool)
    .await
    .expect("load unknown replay SHA fixture");
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&file_path)
    .execute(&pool)
    .await
    .expect("mark archive materialized for unknown replay SHA test");

    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("materialize initial Summary replay markers");
    let bucket_start_epoch =
        invocation_bucket_start_epoch(occurred_at).expect("derive unknown replay SHA bucket");
    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = CASE target \
         WHEN ?1 THEN NULL ELSE ' ' END \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(&file_path)
    .execute(&pool)
    .await
    .expect("seed NULL and blank replay SHAs");
    sqlx::query(
        "UPDATE invocation_rollup_hourly SET total_count = 0, total_tokens = 0, total_cost = 0 \
         WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .execute(&pool)
    .await
    .expect("corrupt Summary rollup behind unknown replay SHAs");

    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("fail-closed marker repair should rebuild the Summary rollup");

    let repaired_total: i64 = sqlx::query_scalar(
        "SELECT total_count FROM invocation_rollup_hourly \
         WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(bucket_start_epoch)
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load repaired Summary rollup after unknown replay SHAs");
    assert_eq!(repaired_total, 1);
    let repaired_marker_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM hourly_rollup_archive_replay \
         WHERE dataset = 'codex_invocations' AND file_path = ?1 \
           AND archive_sha256 = ?2",
    )
    .bind(&file_path)
    .bind(&archive_sha)
    .fetch_one(&pool)
    .await
    .expect("load repaired replay SHA markers");
    assert_eq!(repaired_marker_count, 2);
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_rollup_force_repair_pages_past_archive_batch_budget() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("summary-rollup-force-repair-pages").await;
    let archive_count = 4_097_i64;
    for id in 1..=archive_count {
        let file_path = config.archive_dir.join(format!(
            "summary-rollup-force-repair-missing-{id}.sqlite.gz"
        ));
        sqlx::query(
            "INSERT INTO archive_batches (id, dataset, month_key, file_path, sha256, row_count, \
             status, coverage_start_at, coverage_end_at, historical_rollups_materialized_at) \
             VALUES (?1, 'codex_invocations', '2020-01', ?2, ?3, 1, 'completed', \
                     '2020-01-15 00:00:00', '2020-01-15 00:00:00', datetime('now'))",
        )
        .bind(id)
        .bind(file_path.to_string_lossy().to_string())
        .bind(format!("summary-force-sha-{id}"))
        .execute(&pool)
        .await
        .expect("insert force-repair archive manifest");
    }
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay \
             (target, dataset, file_path, archive_sha256) \
             SELECT ?1, dataset, file_path, sha256 FROM archive_batches \
             WHERE dataset = 'codex_invocations' AND status = 'completed'",
        )
        .bind(target)
        .execute(&pool)
        .await
        .expect("seed complete force-repair replay markers");
    }
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay \
         WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .bind(
        config
            .archive_dir
            .join("summary-rollup-force-repair-missing-1.sqlite.gz")
            .to_string_lossy()
            .to_string(),
    )
    .execute(&pool)
    .await
    .expect("seed one missing force-repair marker");

    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("first force-repair archive page should commit");
    let first_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(crate::stats::INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET)
    .fetch_one(&pool)
    .await
    .expect("load first force-repair archive cursor");
    assert_eq!(first_cursor, 4_096);

    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("second force-repair archive page should commit");
    let second_cursor_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM hourly_rollup_live_progress WHERE dataset = ?1)",
    )
    .bind(crate::stats::INVOCATION_SUMMARY_ROLLUP_REPAIR_ARCHIVE_CURSOR_DATASET)
    .fetch_one(&pool)
    .await
    .expect("check completed force-repair archive cursor");
    assert_eq!(second_cursor_exists, 0);
    let repair_marker_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM hourly_rollup_live_progress WHERE dataset = ?1)",
    )
    .bind(crate::stats::INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
    .fetch_one(&pool)
    .await
    .expect("check completed force-repair marker");
    assert_eq!(repair_marker_exists, 0);
    let repair_incomplete_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM hourly_rollup_live_progress WHERE dataset = ?1)",
    )
    .bind(crate::stats::INVOCATION_SUMMARY_ROLLUP_REPAIR_INCOMPLETE_DATASET)
    .fetch_one(&pool)
    .await
    .expect("check incomplete force-repair marker");
    assert_eq!(repair_incomplete_exists, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_rollup_force_repair_deduplicates_source_ids_across_pages() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("summary-rollup-force-repair-cross-page-dedup").await;
    let occurred_at = "2026-01-15 08:10:00";
    let archive_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "summary-rollup-force-repair-cross-page-dedup-source",
        &[(
            1_i64,
            "summary-rollup-force-repair-cross-page-row",
            occurred_at,
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    let file_path = archive_path.to_string_lossy().to_string();
    let archive_sha: String = sqlx::query_scalar(
        "SELECT sha256 FROM archive_batches WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&file_path)
    .fetch_one(&pool)
    .await
    .expect("load cross-page dedup archive SHA");
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&file_path)
    .execute(&pool)
    .await
    .expect("mark cross-page dedup archive materialized");
    let duplicate_path = config
        .archive_dir
        .join("summary-rollup-force-repair-cross-page-dedup-duplicate.sqlite.gz");
    fs::copy(&archive_path, &duplicate_path).expect("copy cross-page dedup archive");
    let duplicate_file_path = duplicate_path.to_string_lossy().to_string();
    for id in 2_i64..=4_096_i64 {
        sqlx::query(
            "INSERT INTO archive_batches (id, dataset, month_key, file_path, sha256, row_count, status, historical_rollups_materialized_at, coverage_start_at, coverage_end_at) \
             VALUES (?1, 'codex_invocations', ?2, ?3, ?4, 1, 'completed', datetime('now'), ?5, ?5)",
        )
        .bind(id)
        .bind(format!("2020-{id:04}"))
        .bind(format!("{file_path}.missing-{id}"))
        .bind(format!("missing-sha-{id}"))
        .bind("2020-01-01 00:00:00")
        .execute(&pool)
        .await
        .expect("insert cross-page dedup missing materialized archive");
    }
    sqlx::query(
        "INSERT INTO archive_batches (id, dataset, month_key, file_path, sha256, row_count, status, historical_rollups_materialized_at, coverage_start_at, coverage_end_at) \
         VALUES (4097, 'codex_invocations', '2099-01', ?1, ?2, 1, 'completed', datetime('now'), ?3, ?3)",
    )
    .bind(&duplicate_file_path)
    .bind(&archive_sha)
    .bind(occurred_at)
    .execute(&pool)
    .await
    .expect("insert duplicate source archive on second repair page");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND file_path IN (?1, ?2)",
    )
    .bind(&file_path)
    .bind(&duplicate_file_path)
    .execute(&pool)
    .await
    .expect("open cross-page dedup repair");

    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("run first cross-page dedup repair page");
    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("run second cross-page dedup repair page");

    let total_count: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(total_count), 0) FROM invocation_rollup_hourly")
            .fetch_one(&pool)
            .await
            .expect("load cross-page deduplicated rollup total");
    assert_eq!(total_count, 1);
    let seen_id_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM hourly_rollup_repair_seen_invocation_ids")
            .fetch_one(&pool)
            .await
            .expect("load durable cross-page seen IDs");
    assert_eq!(seen_id_count, 1);
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_rollup_force_repair_clears_safe_buckets_before_later_pages() {
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("summary-rollup-force-repair-safe-bucket").await;
    let protected_at = "2026-01-15 08:10:00";
    let safe_at = "2026-01-16 08:10:00";
    let missing_path = seed_invocation_archive_batch(
        &pool,
        &config,
        "summary-rollup-force-repair-protected-source",
        &[(
            1_i64,
            "summary-rollup-force-repair-protected-row",
            protected_at,
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    let missing_file_path = missing_path.to_string_lossy().to_string();
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now'), \
             coverage_start_at = ?2, coverage_end_at = ?2 \
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&missing_file_path)
    .bind(protected_at)
    .execute(&pool)
    .await
    .expect("mark protected archive materialized");
    fs::remove_file(&missing_path).expect("remove protected materialized archive");
    let protected_bucket =
        invocation_bucket_start_epoch(protected_at).expect("derive protected bucket");
    let safe_archive_path = {
        for id in 2_i64..=4_096_i64 {
            sqlx::query(
                "INSERT INTO archive_batches (id, dataset, month_key, file_path, sha256, row_count, status, historical_rollups_materialized_at, coverage_start_at, coverage_end_at) \
                 VALUES (?1, 'codex_invocations', ?2, ?3, ?4, 1, 'completed', datetime('now'), ?5, ?5)",
            )
            .bind(id)
            .bind(format!("2020-{id:04}"))
            .bind(config.archive_dir.join(format!("safe-bucket-missing-{id}.sqlite.gz")).to_string_lossy().to_string())
            .bind(format!("safe-bucket-missing-sha-{id}"))
            .bind(protected_at)
            .execute(&pool)
            .await
            .expect("insert protected bucket filler archive");
        }
        seed_invocation_archive_batch(
            &pool,
            &config,
            "summary-rollup-force-repair-safe-source",
            &[(
                2_i64,
                "summary-rollup-force-repair-safe-row",
                safe_at,
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(120.0),
            )],
        )
        .await
    };
    let safe_file_path = safe_archive_path.to_string_lossy().to_string();
    sqlx::query(
        "UPDATE archive_batches SET coverage_start_at = ?2, coverage_end_at = ?2 \
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(&safe_file_path)
    .bind(safe_at)
    .execute(&pool)
    .await
    .expect("set safe archive coverage");
    let safe_bucket = invocation_bucket_start_epoch(safe_at).expect("derive safe bucket");
    let empty_histogram =
        encode_approx_histogram(&empty_approx_histogram()).expect("encode safe-bucket histogram");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms, first_byte_max_ms, first_byte_histogram) \
         VALUES (?1, ?2, 99, 99, 0, 990, 9.90, 0, 0, 0, ?3)",
    )
    .bind(safe_bucket)
    .bind(SOURCE_PROXY)
    .bind(empty_histogram)
    .execute(&pool)
    .await
    .expect("seed stale safe-bucket rollup");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, raw_response) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(10_i64)
    .bind("summary-rollup-force-repair-safe-live-row")
    .bind(safe_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(30_i64)
    .bind(0.30_f64)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("seed retained live safe-bucket row");
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, ?2, datetime('now')) \
         ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id, updated_at = excluded.updated_at",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(10_i64)
    .execute(&pool)
    .await
    .expect("seed shared live cursor for retained row");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms, first_byte_max_ms, first_byte_histogram) \
         VALUES (?1, ?2, 1, 1, 0, 10, 0.10, 0, 0, 0, ?3)",
    )
    .bind(protected_bucket)
    .bind(SOURCE_PROXY)
    .bind(encode_approx_histogram(&empty_approx_histogram()).expect("encode protected histogram"))
    .execute(&pool)
    .await
    .expect("seed preserved protected-bucket rollup");
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE dataset = 'codex_invocations' AND file_path IN (?1, ?2)",
    )
    .bind(&missing_file_path)
    .bind(&safe_file_path)
    .execute(&pool)
    .await
    .expect("open safe-bucket force repair");

    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("run first protected-bucket repair page");
    crate::stats::backfill_missing_invocation_summary_archive_rollups(&pool)
        .await
        .expect("run later safe-bucket repair page");

    let safe_total: i64 = sqlx::query_scalar(
        "SELECT total_count FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(safe_bucket)
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load rebuilt safe-bucket rollup");
    assert_eq!(safe_total, 2);
    let protected_total: i64 = sqlx::query_scalar(
        "SELECT total_count FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(protected_bucket)
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("load preserved protected-bucket rollup");
    assert_eq!(protected_total, 1);
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn summary_rollup_repair_deduplicates_restored_live_rows_after_full_rebuild() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;
    let shared_hour = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(430))
    .and_hms_opt(8, 0, 0)
    .expect("valid shared repair hour");
    let archived_at = format_naive(
        shared_hour
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived invocation time"),
    );
    let live_at = format_naive(
        shared_hour
            .checked_add_signed(ChronoDuration::minutes(45))
            .expect("live invocation time"),
    );

    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-repair-restored-live-dedup",
        &[(
            1_i64,
            "summary-repair-restored-live-archived",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, raw_response) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(10_i64)
    .bind("summary-repair-restored-live-live")
    .bind(&live_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(20_i64)
    .bind(0.20_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert live row in the shared repair bucket");
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, ?2, datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(10_i64)
    .execute(&state.pool)
    .await
    .expect("seed shared live cursor");

    crate::stats::ensure_invocation_summary_rollups_ready(&state.pool)
        .await
        .expect("run background Summary repair");

    let totals = sqlx::query_as::<_, (i64, i64, f64)>(
        "SELECT COALESCE(SUM(total_count), 0), COALESCE(SUM(total_tokens), 0), \
                COALESCE(SUM(total_cost), 0.0) \
         FROM invocation_rollup_hourly",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load repaired Summary totals");
    assert_eq!(totals.0, 2);
    assert_eq!(totals.1, 30);
    assert_f64_close(totals.2, 0.30);
}

#[tokio::test]
async fn summary_rollup_repair_persists_live_cursor_across_incomplete_retries() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;
    let shared_hour = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(430))
    .and_hms_opt(8, 0, 0)
    .expect("valid incomplete repair hour");
    let archived_at = format_naive(
        shared_hour
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("archived incomplete repair time"),
    );
    let live_at = format_naive(
        shared_hour
            .checked_add_signed(ChronoDuration::minutes(45))
            .expect("live incomplete repair time"),
    );
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-repair-incomplete-live-cursor",
        &[(
            1_i64,
            "summary-repair-incomplete-archived",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            10_i64,
            0.10_f64,
            Some(100.0),
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark archive as materialized before source loss");
    fs::remove_file(&archive_path).expect("remove materialized archive source");
    sqlx::query(
        "INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, raw_response) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(10_i64)
    .bind("summary-repair-incomplete-live-tail")
    .bind(&live_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(20_i64)
    .bind(0.20_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert live tail after shared cursor");
    sqlx::query(
        "INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at) \
         VALUES (?1, ?2, datetime('now'))",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(1_i64)
    .execute(&state.pool)
    .await
    .expect("seed shared cursor before live tail");

    crate::stats::ensure_invocation_summary_rollups_ready(&state.pool)
        .await
        .expect("run background Summary repair");

    let totals = sqlx::query_as::<_, (i64, i64, f64)>(
        "SELECT COALESCE(SUM(total_count), 0), COALESCE(SUM(total_tokens), 0), \
                COALESCE(SUM(total_cost), 0.0) \
         FROM invocation_rollup_hourly",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load incomplete-repair Summary totals");
    assert_eq!(totals.0, 1);
    assert_eq!(totals.1, 20);
    assert_f64_close(totals.2, 0.20);
    let repair_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind("codex_invocations_summary_rollup_v2_live_cursor")
    .fetch_one(&state.pool)
    .await
    .expect("load persisted incomplete-repair live cursor");
    assert_eq!(repair_cursor, 10);
}
