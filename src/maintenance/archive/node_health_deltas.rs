use super::*;

/// Establish an exact baseline once per selected file batch. Later source transactions
/// update only the cache identities they replace, never regroup the complete monthly cache.
pub(crate) async fn cache_pool_node_health_batch_chunk_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
    archive_file_path: &str,
    ids: &[i64],
    first_chunk: bool,
) -> Result<()> {
    if first_chunk {
        cache_pool_upstream_node_health_archive_rows_from_live_ids_tx(tx, archive_file_path, ids)
            .await?;
        refresh_pool_upstream_node_health_hourly_archive_rows_from_cache_tx(
            tx,
            archive_batch_id,
            archive_file_path,
        )
        .await?;
        return Ok(());
    }
    let before = load_chunk_rows(tx, archive_file_path, ids).await?;
    cache_pool_upstream_node_health_archive_rows_from_live_ids_tx(tx, archive_file_path, ids)
        .await?;
    let after = load_chunk_rows(tx, archive_file_path, ids).await?;
    let mut deltas: BTreeMap<(String, i64), (i64, i64)> = BTreeMap::new();
    accumulate_deltas(&mut deltas, before, -1)?;
    accumulate_deltas(&mut deltas, after, 1)?;
    let identity = pool_upstream_node_health_archive_identity_for_batch_id(archive_batch_id);
    for ((binding, bucket), (successes, failures)) in deltas {
        if successes == 0 && failures == 0 {
            continue;
        }
        let totals: (i64, i64) = sqlx::query_as(
            "INSERT INTO pool_upstream_node_health_hourly_archive
             (archive_identity, archive_batch_id, archive_file_path, proxy_binding_key_snapshot,
              bucket_start_epoch, success_count, failure_count, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,datetime('now'))
             ON CONFLICT(archive_identity, proxy_binding_key_snapshot, bucket_start_epoch)
             DO UPDATE SET success_count = success_count + excluded.success_count,
                           failure_count = failure_count + excluded.failure_count,
                           updated_at = datetime('now')
             RETURNING success_count, failure_count",
        )
        .bind(&identity)
        .bind(archive_batch_id)
        .bind(archive_file_path)
        .bind(&binding)
        .bind(bucket)
        .bind(successes)
        .bind(failures)
        .fetch_one(&mut *tx)
        .await?;
        if totals.0 < 0 || totals.1 < 0 {
            bail!("node-health archive hourly baseline changed during task-local conversion");
        }
        if totals == (0, 0) {
            sqlx::query(
                "DELETE FROM pool_upstream_node_health_hourly_archive
                 WHERE archive_identity=?1 AND proxy_binding_key_snapshot=?2 AND bucket_start_epoch=?3",
            )
            .bind(&identity)
            .bind(&binding)
            .bind(bucket)
            .execute(&mut *tx)
            .await?;
        }
    }
    Ok(())
}

async fn load_chunk_rows(
    tx: &mut SqliteConnection,
    archive_file_path: &str,
    ids: &[i64],
) -> Result<Vec<PoolUpstreamNodeHealthArchiveRecord>> {
    Ok(sqlx::query_as(
        "SELECT archived_row_id, occurred_at, proxy_binding_key_snapshot, is_success, latency_ms
         FROM pool_upstream_node_health_archive
         WHERE archive_file_path=?1 AND archived_row_id IN (SELECT value FROM json_each(?2))",
    )
    .bind(archive_file_path)
    .bind(serde_json::to_string(ids)?)
    .fetch_all(&mut *tx)
    .await?)
}

fn accumulate_deltas(
    deltas: &mut BTreeMap<(String, i64), (i64, i64)>,
    rows: Vec<PoolUpstreamNodeHealthArchiveRecord>,
    sign: i64,
) -> Result<()> {
    for row in rows {
        let bucket = invocation_bucket_start_epoch(&row.occurred_at)?;
        let delta = deltas
            .entry((row.proxy_binding_key_snapshot, bucket))
            .or_default();
        if row.is_success != 0 {
            delta.0 += sign;
        } else {
            delta.1 += sign;
        }
    }
    Ok(())
}
