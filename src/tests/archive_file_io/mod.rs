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

    for id in 1..=archive_count {
        let invoke_id = format!("summary-rollup-bounded-shared-bucket-{id}");
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
        .await;
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

    cleanup_temp_test_dir(&temp_dir);
}
