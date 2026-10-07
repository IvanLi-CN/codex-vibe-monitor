use super::*;

async fn seed_attempt(pool: &SqlitePool, id: i64, occurred_at: &str, binding: &str, status: &str) {
    sqlx::query("INSERT INTO pool_upstream_request_attempts(id,invoke_id,occurred_at,finished_at,endpoint,route_mode,attempt_index,distinct_account_index,same_account_retry_index,status,proxy_binding_key_snapshot) VALUES(?1,?2,?3,?3,'/v1/responses','pool',0,0,0,?4,?5)")
        .bind(id).bind(format!("delta-{id}")).bind(occurred_at).bind(status).bind(binding)
        .execute(pool).await.expect("attempt fixture");
}

async fn apply_chunk(pool: &SqlitePool, ids: &[i64], baseline: bool) {
    let mut tx = pool.begin().await.expect("chunk transaction");
    crate::maintenance::cache_pool_node_health_batch_chunk_tx(
        tx.as_mut(),
        42,
        "month.sqlite.gz",
        ids,
        baseline,
    )
    .await
    .expect("exact cache update");
    tx.commit().await.expect("chunk commit");
}

#[tokio::test]
async fn retention_node_health_chunk_deltas_match_full_reference_without_rewriting_other_buckets() {
    let (pool, _, temp_dir) = retention_test_pool_and_config("node-health-deltas").await;
    seed_attempt(&pool, 1, "2020-01-01 08:00:00", "untouched", "success").await;
    seed_attempt(&pool, 2, "2020-01-01 09:00:00", "changed", "success").await;
    apply_chunk(&pool, &[1, 2], true).await;
    // Any whole-file refresh would delete/rewrite this unaffected bucket. This is a
    // structural assertion, not a timing or CPU performance threshold.
    sqlx::query("CREATE TRIGGER preserve_unaffected_hour BEFORE DELETE ON pool_upstream_node_health_hourly_archive WHEN OLD.proxy_binding_key_snapshot='untouched' BEGIN SELECT RAISE(ABORT,'unrelated archive bucket rewritten'); END")
        .execute(&pool).await.expect("unaffected-bucket guard");
    sqlx::query("CREATE TRIGGER preserve_unaffected_hour_update BEFORE UPDATE ON pool_upstream_node_health_hourly_archive WHEN OLD.proxy_binding_key_snapshot='untouched' BEGIN SELECT RAISE(ABORT,'unrelated archive bucket updated'); END")
        .execute(&pool).await.expect("unaffected-bucket update guard");
    seed_attempt(&pool, 3, "2020-01-01 09:30:00", "changed", "failed").await;
    apply_chunk(&pool, &[3], false).await;
    apply_chunk(&pool, &[3], false).await; // Retry is not another contribution.
    sqlx::query("UPDATE pool_upstream_request_attempts SET proxy_binding_key_snapshot='moved',occurred_at='2020-01-01T02:30:00Z',status='success' WHERE id=3")
        .execute(&pool).await.expect("changed cached identity");
    apply_chunk(&pool, &[3], false).await;
    sqlx::query("UPDATE pool_upstream_request_attempts SET proxy_binding_key_snapshot='moved',occurred_at='2020-01-01T02:30:00Z',status='failed' WHERE id=2")
        .execute(&pool).await.expect("move final old-bucket row");
    apply_chunk(&pool, &[2], false).await;
    let actual: Vec<(String,i64,i64,i64)> = sqlx::query_as("SELECT proxy_binding_key_snapshot,bucket_start_epoch,success_count,failure_count FROM pool_upstream_node_health_hourly_archive WHERE archive_batch_id=42 ORDER BY 1,2")
        .fetch_all(&pool).await.expect("hourly cache");
    let reference: Vec<(String,i64,i64,i64)> = sqlx::query_as("SELECT proxy_binding_key_snapshot,((CASE WHEN instr(occurred_at,'T')>0 THEN CAST(strftime('%s',occurred_at) AS INTEGER) ELSE CAST(strftime('%s',occurred_at||'+08:00') AS INTEGER) END)/3600)*3600 AS bucket,SUM(is_success),SUM(CASE WHEN is_success=0 THEN 1 ELSE 0 END) FROM pool_upstream_node_health_archive WHERE archive_file_path='month.sqlite.gz' GROUP BY 1,2 ORDER BY 1,2")
        .fetch_all(&pool).await.expect("full exact reference");
    assert_eq!(actual, reference);
    assert_eq!(actual.len(), 2, "empty old buckets are removed");
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_node_health_delta_and_source_transition_rollback_together() {
    let (pool, _, temp_dir) = retention_test_pool_and_config("node-health-delta-rollback").await;
    seed_attempt(&pool, 1, "2020-01-01 08:00:00", "binding", "success").await;
    apply_chunk(&pool, &[1], true).await;
    seed_attempt(&pool, 2, "2020-01-01 08:30:00", "binding", "failed").await;
    let mut tx = pool.begin().await.expect("source conversion transaction");
    crate::maintenance::cache_pool_node_health_batch_chunk_tx(
        tx.as_mut(),
        42,
        "month.sqlite.gz",
        &[2],
        false,
    )
    .await
    .expect("transactional delta");
    sqlx::query("DELETE FROM pool_upstream_request_attempts WHERE id=2")
        .execute(tx.as_mut())
        .await
        .expect("source transition");
    tx.rollback().await.expect("injected abort");
    let totals: (i64,i64) = sqlx::query_as("SELECT success_count,failure_count FROM pool_upstream_node_health_hourly_archive WHERE archive_batch_id=42")
        .fetch_one(&pool).await.expect("committed baseline");
    assert_eq!(totals, (1, 0));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE id=2"
        )
        .fetch_one(&pool)
        .await
        .expect("source preserved"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pool_upstream_node_health_archive WHERE archived_row_id=2"
        )
        .fetch_one(&pool)
        .await
        .expect("cache rolled back"),
        0
    );
    apply_chunk(&pool, &[2], false).await;
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
