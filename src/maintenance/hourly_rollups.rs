use super::*;

#[path = "hourly_rollup_archive_support.rs"]
mod hourly_rollup_archive_support;
pub(crate) use hourly_rollup_archive_support::*;

const LIVE_ROLLUP_LOCK_RETRY_MAX_ATTEMPTS: u32 = 3;
const LIVE_ROLLUP_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);
const LEGACY_PRUNED_PAYLOAD_MODE_STRUCTURED_ROLLUP_UNKNOWN_REASONING: &str =
    "structured_rollup_unknown_reasoning";
const LEGACY_PRUNED_PAYLOAD_MODE_BLOCKED_PAYLOAD_REQUIRED: &str = "blocked_payload_required";
const LEGACY_MATERIALIZED_UPSTREAM_ACCOUNT_ARCHIVE_REPLAY_TARGETS: [&str; 3] = [
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
    HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HourlyRollupRefreshScope {
    Full,
    SkipActiveAccountActivityV2CoverageRepair,
}

pub(crate) async fn sync_hourly_rollups_from_live_tables(pool: &Pool<Sqlite>) -> Result<()> {
    sync_hourly_rollups_from_live_tables_with_scope(pool, None, HourlyRollupRefreshScope::Full)
        .await
}

pub(crate) async fn sync_hourly_rollups_from_live_tables_with_parallel_work_coverage(
    pool: &Pool<Sqlite>,
    invocation_live_days: Option<u64>,
) -> Result<()> {
    sync_hourly_rollups_from_live_tables_with_scope(
        pool,
        invocation_live_days,
        HourlyRollupRefreshScope::Full,
    )
    .await
}

async fn sync_hourly_rollups_from_live_tables_with_scope(
    pool: &Pool<Sqlite>,
    invocation_live_days: Option<u64>,
    scope: HourlyRollupRefreshScope,
) -> Result<()> {
    let mut attempt = 1_u32;
    loop {
        match sync_hourly_rollups_from_live_tables_once(pool, invocation_live_days, scope).await {
            Ok(()) => return Ok(()),
            Err(err)
                if attempt < LIVE_ROLLUP_LOCK_RETRY_MAX_ATTEMPTS
                    && crate::is_sqlite_lock_error(&err) =>
            {
                warn!(
                    attempt,
                    max_attempts = LIVE_ROLLUP_LOCK_RETRY_MAX_ATTEMPTS,
                    retry_delay_ms = LIVE_ROLLUP_LOCK_RETRY_DELAY.as_millis() as u64,
                    error = %err,
                    "live hourly rollup sync hit sqlite lock; retrying"
                );
                attempt += 1;
                tokio::time::sleep(LIVE_ROLLUP_LOCK_RETRY_DELAY).await;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "live hourly rollup sync failed after {attempt}/{} attempt(s)",
                        LIVE_ROLLUP_LOCK_RETRY_MAX_ATTEMPTS
                    )
                });
            }
        }
    }
}

async fn sync_hourly_rollups_from_live_tables_once(
    pool: &Pool<Sqlite>,
    invocation_live_days: Option<u64>,
    scope: HourlyRollupRefreshScope,
) -> Result<()> {
    if scope == HourlyRollupRefreshScope::Full {
        repair_active_account_activity_v2_coverage(pool).await?;
    }
    loop {
        let updated = replay_live_invocation_hourly_rollups(pool).await?;
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated = replay_live_forward_proxy_attempt_hourly_rollups(pool).await?;
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated =
            replay_live_upstream_host_network_minute_rollups_from_invocations(pool).await?;
        if updated == 0 {
            break;
        }
    }
    loop {
        let updated =
            replay_live_upstream_host_network_minute_rollups_from_pool_attempts(pool).await?;
        if updated == 0 {
            break;
        }
    }
    let repaired_activity_v2_rows = repair_live_invocation_account_activity_v2_once(pool).await?;
    wake_account_activity_v2_coverage_repair(pool, repaired_activity_v2_rows).await?;
    if let Some(days) = invocation_live_days {
        maintain_parallel_work_rollups(pool, Some(shanghai_retention_cutoff(days).timestamp()))
            .await?;
    }
    Ok(())
}

pub(crate) async fn wake_account_activity_v2_coverage_repair(
    pool: &Pool<Sqlite>,
    repaired_activity_v2_rows: u64,
) -> Result<()> {
    if repaired_activity_v2_rows > 0 {
        wake_startup_backfill_coverage_repair(pool, "live_account_activity_v2_coverage_updated")
            .await?;
    }
    Ok(())
}

pub(crate) async fn mark_materialized_upstream_account_archive_replayed_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<()> {
    for target in LEGACY_MATERIALIZED_UPSTREAM_ACCOUNT_ARCHIVE_REPLAY_TARGETS {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            file_path,
        )
        .await?;
    }
    Ok(())
}

fn can_shortcut_legacy_materialized_upstream_account_targets(pending_targets: &[&str]) -> bool {
    !pending_targets.is_empty()
        && pending_targets.iter().all(|target| {
            LEGACY_MATERIALIZED_UPSTREAM_ACCOUNT_ARCHIVE_REPLAY_TARGETS.contains(target)
        })
}

pub(crate) async fn load_materialized_invocation_archives_missing_upstream_account_markers_tx(
    tx: &mut SqliteConnection,
) -> Result<Vec<String>> {
    sqlx::query_scalar(
        r#"
        SELECT batches.file_path
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND batches.historical_rollups_materialized_at IS NOT NULL
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
          AND (
                NOT EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?2
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                )
                OR NOT EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?3
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                )
                OR NOT EXISTS (
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?4
                      AND replay.dataset = batches.dataset
                      AND replay.file_path = batches.file_path
                )
          )
        ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE)
    .fetch_all(&mut *tx)
    .await
    .map_err(Into::into)
}

pub(crate) async fn repair_materialized_upstream_account_archive_markers(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let file_paths =
        load_materialized_invocation_archives_missing_upstream_account_markers_tx(tx.as_mut())
            .await?;
    for file_path in &file_paths {
        mark_materialized_upstream_account_archive_replayed_tx(tx.as_mut(), file_path).await?;
    }
    tx.commit().await?;
    Ok(file_paths.len())
}

async fn load_materialized_invocation_archives_for_usage_breakdown_repair_tx(
    tx: &mut SqliteConnection,
) -> Result<Vec<(String, Option<String>, Option<String>)>> {
    sqlx::query_as(
        r#"
        SELECT
            batches.file_path,
            batches.coverage_start_at,
            batches.coverage_end_at
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND batches.historical_rollups_materialized_at IS NOT NULL
        ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(&mut *tx)
    .await
    .map_err(Into::into)
}

async fn clear_usage_breakdown_rollup_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<()> {
    if bucket_start_epochs.is_empty() {
        return Ok(());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "DELETE FROM upstream_account_usage_breakdown_hourly WHERE bucket_start_epoch IN (",
    );
    let mut separated = query.separated(", ");
    for bucket_start_epoch in bucket_start_epochs {
        separated.push_bind(*bucket_start_epoch);
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    Ok(())
}

async fn clear_invocation_rollup_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<()> {
    let mut bucket_start_epochs = bucket_start_epochs.iter().copied().collect::<Vec<_>>();
    bucket_start_epochs.sort_unstable();
    bucket_start_epochs.dedup();

    for table in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
    ] {
        delete_hourly_rollup_rows_for_bucket_epochs_tx(tx, table, &bucket_start_epochs).await?;
    }
    delete_rollup_rows_for_bucket_epochs_with_size_tx(
        tx,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
        &bucket_start_epochs,
        60,
    )
    .await?;

    let mut query = QueryBuilder::<Sqlite>::new(
        "DELETE FROM hourly_rollup_materialized_buckets WHERE bucket_start_epoch IN (",
    );
    {
        let mut separated = query.separated(", ");
        for bucket_start_epoch in &bucket_start_epochs {
            separated.push_bind(*bucket_start_epoch);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;

    let retained_live_rows =
        load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
    upsert_invocation_hourly_rollups_tx(tx, &retained_live_rows, &INVOCATION_HOURLY_ROLLUP_TARGETS)
        .await?;
    rebuild_parallel_work_rollups_for_hours_tx(tx, &bucket_start_epochs).await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct InvocationArchiveCoverageRow {
    file_path: String,
    sha256: Option<String>,
    coverage_start_at: String,
    coverage_end_at: String,
}

#[derive(sqlx::FromRow)]
struct ForwardProxyArchiveCoverageRow {
    file_path: String,
    sha256: Option<String>,
    coverage_start_at: String,
    coverage_end_at: String,
}

async fn load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<Vec<InvocationArchiveCoverageRow>> {
    if bucket_start_epochs.is_empty() {
        return Ok(Vec::new());
    }

    let min_bucket_start_epoch = bucket_start_epochs
        .iter()
        .copied()
        .min()
        .ok_or_else(|| anyhow!("missing minimum usage breakdown bucket start epoch"))?;
    let max_bucket_start_epoch = bucket_start_epochs
        .iter()
        .copied()
        .max()
        .ok_or_else(|| anyhow!("missing maximum usage breakdown bucket start epoch"))?;
    let overlap_start = chrono::Utc
        .timestamp_opt(min_bucket_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum usage breakdown bucket start epoch"))?;
    let overlap_end = chrono::Utc
        .timestamp_opt(max_bucket_start_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum usage breakdown bucket start epoch"))?;
    let overlap_start =
        crate::stats::format_naive(overlap_start.with_timezone(&Shanghai).naive_local());
    let overlap_end =
        crate::stats::format_naive(overlap_end.with_timezone(&Shanghai).naive_local());

    sqlx::query_as(
        r#"
        SELECT file_path, sha256, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = ?1
          AND status = ?2
          AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
          AND coverage_start_at IS NOT NULL
          AND coverage_end_at IS NOT NULL
          AND coverage_end_at >= ?3
          AND coverage_start_at < ?4
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&overlap_start)
    .bind(&overlap_end)
    .fetch_all(&mut *tx)
    .await
    .map_err(Into::into)
}

fn forward_proxy_archive_bucket_start_epochs_from_bounds(
    coverage_start_at: &str,
    coverage_end_at: &str,
) -> Result<HashSet<i64>> {
    let coverage_start_epoch = align_bucket_epoch(
        parse_utc_naive(coverage_start_at)?.and_utc().timestamp(),
        3_600,
        0,
    );
    let coverage_end_epoch = align_bucket_epoch(
        parse_utc_naive(coverage_end_at)?.and_utc().timestamp(),
        3_600,
        0,
    );
    if coverage_end_epoch < coverage_start_epoch {
        return Ok(HashSet::new());
    }

    let mut bucket_start_epochs = HashSet::new();
    let mut current_epoch = coverage_start_epoch;
    while current_epoch <= coverage_end_epoch {
        bucket_start_epochs.insert(current_epoch);
        current_epoch += 3_600;
    }
    Ok(bucket_start_epochs)
}

async fn load_completed_forward_proxy_archives_overlapping_buckets_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<Vec<ForwardProxyArchiveCoverageRow>> {
    if bucket_start_epochs.is_empty() {
        return Ok(Vec::new());
    }

    let min_bucket_start_epoch = bucket_start_epochs
        .iter()
        .copied()
        .min()
        .ok_or_else(|| anyhow!("missing minimum forward proxy bucket start epoch"))?;
    let max_bucket_start_epoch = bucket_start_epochs
        .iter()
        .copied()
        .max()
        .ok_or_else(|| anyhow!("missing maximum forward proxy bucket start epoch"))?;
    let overlap_start = Utc
        .timestamp_opt(min_bucket_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum forward proxy bucket start epoch"))?
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let overlap_end = Utc
        .timestamp_opt(max_bucket_start_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum forward proxy bucket start epoch"))?
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();

    sqlx::query_as(
        r#"
        SELECT file_path, sha256, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = ?1
          AND status = ?2
          AND coverage_start_at IS NOT NULL
          AND coverage_end_at IS NOT NULL
          AND coverage_end_at >= ?3
          AND coverage_start_at < ?4
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&overlap_start)
    .bind(&overlap_end)
    .fetch_all(&mut *tx)
    .await
    .map_err(Into::into)
}

async fn archive_batch_has_completed_manifest_sha_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM archive_batches
        WHERE dataset = ?1
          AND file_path = ?2
          AND status = 'completed'
          AND sha256 IS NOT NULL
          AND TRIM(sha256) <> ''
        LIMIT 1
        "#,
    )
    .bind(dataset)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

async fn invocation_archive_has_materialized_rollups_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM archive_batches
        WHERE dataset = ?1
          AND file_path = ?2
          AND status = ?3
          AND historical_rollups_materialized_at IS NOT NULL
        LIMIT 1
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

async fn reset_invocation_archive_usage_breakdown_backfill_state_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND target = ?2
          AND file_path = ?3
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = NULL
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    delete_hourly_rollup_archive_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS, file_path)
        .await?;
    delete_hourly_rollup_archive_progress_tx(
        tx,
        INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET,
        file_path,
    )
    .await?;
    Ok(())
}

async fn invocation_archive_has_stale_replay_marker_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM hourly_rollup_archive_replay AS replay
        INNER JOIN archive_batches AS batches
            ON batches.dataset = replay.dataset
           AND batches.file_path = replay.file_path
           AND batches.status = 'completed'
        WHERE replay.dataset = ?1
          AND replay.file_path = ?2
          AND replay.archive_sha256 IS NOT NULL
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
          AND replay.archive_sha256 <> batches.sha256
        LIMIT 1
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

async fn invocation_archive_has_unverified_replay_marker_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM hourly_rollup_archive_replay
        WHERE dataset = ?1
          AND file_path = ?2
          AND (
                archive_sha256 IS NULL
                OR TRIM(archive_sha256) = ''
          )
        LIMIT 1
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

async fn forward_proxy_archive_has_stale_replay_marker_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM hourly_rollup_archive_replay AS replay
        INNER JOIN archive_batches AS batches
            ON batches.dataset = replay.dataset
           AND batches.file_path = replay.file_path
           AND batches.status = 'completed'
        WHERE replay.target = ?1
          AND replay.dataset = ?2
          AND replay.file_path = ?3
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
          AND (
                replay.archive_sha256 IS NULL
                OR TRIM(replay.archive_sha256) = ''
                OR replay.archive_sha256 <> batches.sha256
          )
        LIMIT 1
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
    .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

async fn pool_upstream_node_health_archive_has_stale_replay_marker_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT 1
        FROM hourly_rollup_archive_replay AS replay
        INNER JOIN archive_batches AS batches
            ON batches.dataset = replay.dataset
           AND batches.file_path = replay.file_path
           AND batches.status = 'completed'
        WHERE replay.target = ?1
          AND replay.dataset = 'pool_upstream_request_attempts'
          AND replay.file_path = ?2
          AND replay.archive_sha256 IS NOT NULL
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
          AND replay.archive_sha256 <> batches.sha256
        LIMIT 1
        "#,
    )
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

async fn reset_replaced_pool_upstream_node_health_archive_state_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
    file_path: &str,
) -> Result<()> {
    delete_pool_upstream_node_health_archive_rows_for_file_tx(tx, file_path).await?;
    delete_pool_upstream_node_health_hourly_archive_rows_for_batch_tx(tx, archive_batch_id).await?;
    delete_hourly_rollup_archive_progress_tx(tx, "pool_upstream_request_attempts", file_path)
        .await?;
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_replay
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
          AND target IN (?2, ?3)
        "#,
    )
    .bind(file_path)
    .bind(POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET)
    .bind(POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn reset_invocation_archive_replay_state_tx(
    tx: &mut SqliteConnection,
    file_paths: &[String],
) -> Result<()> {
    for file_path in file_paths {
        sqlx::query(
            r#"
            DELETE FROM hourly_rollup_archive_replay
            WHERE dataset = ?1 AND file_path = ?2
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            DELETE FROM hourly_rollup_archive_progress
            WHERE file_path = ?1
            "#,
        )
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            UPDATE archive_batches
            SET historical_rollups_materialized_at = NULL
            WHERE dataset = ?1 AND file_path = ?2
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn clear_forward_proxy_rollup_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    bucket_start_epochs: &HashSet<i64>,
) -> Result<()> {
    let mut bucket_start_epochs = bucket_start_epochs.iter().copied().collect::<Vec<_>>();
    bucket_start_epochs.sort_unstable();
    bucket_start_epochs.dedup();
    if bucket_start_epochs.is_empty() {
        return Ok(());
    }

    delete_hourly_rollup_rows_for_bucket_epochs_tx(
        tx,
        HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
        &bucket_start_epochs,
    )
    .await?;

    let mut query = QueryBuilder::<Sqlite>::new(
        "DELETE FROM hourly_rollup_materialized_buckets WHERE target = ",
    );
    query
        .push_bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
        .push(" AND bucket_start_epoch IN (");
    {
        let mut separated = query.separated(", ");
        for bucket_start_epoch in &bucket_start_epochs {
            separated.push_bind(*bucket_start_epoch);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;

    let min_bucket_start_epoch = *bucket_start_epochs
        .first()
        .ok_or_else(|| anyhow!("missing minimum forward proxy bucket start epoch"))?;
    let max_bucket_start_epoch = *bucket_start_epochs
        .last()
        .ok_or_else(|| anyhow!("missing maximum forward proxy bucket start epoch"))?;
    let min_occurred_at = Utc
        .timestamp_opt(min_bucket_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum forward proxy bucket start epoch"))?
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let max_occurred_at = Utc
        .timestamp_opt(max_bucket_start_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum forward proxy bucket start epoch"))?
        .naive_utc()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    let bucket_start_epoch_set = bucket_start_epochs.iter().copied().collect::<HashSet<_>>();
    let retained_live_rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT id, proxy_key, occurred_at, is_success, latency_ms
        FROM forward_proxy_attempts
        WHERE occurred_at >= ?1
          AND occurred_at < ?2
        ORDER BY id ASC
        "#,
    )
    .bind(&min_occurred_at)
    .bind(&max_occurred_at)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .filter(|row| {
        forward_proxy_attempt_bucket_start_epoch(&row.occurred_at)
            .map(|bucket_start_epoch| bucket_start_epoch_set.contains(&bucket_start_epoch))
            .unwrap_or(false)
    })
    .collect::<Vec<_>>();
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &retained_live_rows).await?;
    mark_forward_proxy_hourly_rollup_buckets_materialized_tx(tx, &retained_live_rows).await?;
    Ok(())
}

async fn reset_forward_proxy_archive_replay_state_tx(
    tx: &mut SqliteConnection,
    file_paths: &[String],
) -> Result<()> {
    for file_path in file_paths {
        sqlx::query(
            r#"
            DELETE FROM hourly_rollup_archive_replay
            WHERE target = ?1
              AND dataset = ?2
              AND file_path = ?3
            "#,
        )
        .bind(HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS)
        .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
        delete_hourly_rollup_archive_progress_tx(
            tx,
            HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
            file_path,
        )
        .await?;
        sqlx::query(
            r#"
            UPDATE archive_batches
            SET historical_rollups_materialized_at = NULL
            WHERE dataset = ?1
              AND file_path = ?2
            "#,
        )
        .bind(HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn reopen_replaced_materialized_forward_proxy_archive_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<Option<Vec<String>>> {
    let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at)
    else {
        return Ok(None);
    };
    let mut bucket_start_epochs =
        forward_proxy_archive_bucket_start_epochs_from_bounds(coverage_start_at, coverage_end_at)?;
    let mut reopened_file_paths = vec![file_path.to_string()];
    let mut reopened_file_path_set = HashSet::from([file_path.to_string()]);

    // Rebuild the transitive overlap closure so rows from a retained peer cannot survive the
    // replacement clear and then be added again by a later replay.
    loop {
        let overlapping_archives =
            load_completed_forward_proxy_archives_overlapping_buckets_tx(tx, &bucket_start_epochs)
                .await?;
        let mut expanded = false;
        for overlapping_archive in overlapping_archives {
            if overlapping_archive
                .sha256
                .as_deref()
                .is_none_or(|sha256| sha256.trim().is_empty())
            {
                return Ok(None);
            }
            if !reopened_file_path_set.insert(overlapping_archive.file_path.clone()) {
                continue;
            }
            reopened_file_paths.push(overlapping_archive.file_path);
            bucket_start_epochs.extend(forward_proxy_archive_bucket_start_epochs_from_bounds(
                &overlapping_archive.coverage_start_at,
                &overlapping_archive.coverage_end_at,
            )?);
            expanded = true;
        }
        if !expanded {
            break;
        }
    }

    clear_forward_proxy_rollup_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
    reset_forward_proxy_archive_replay_state_tx(tx, &reopened_file_paths).await?;
    Ok(Some(reopened_file_paths))
}

async fn reopen_replaced_materialized_invocation_archive_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<Option<Vec<String>>> {
    let (Some(coverage_start_at), Some(coverage_end_at)) = (coverage_start_at, coverage_end_at)
    else {
        return Ok(None);
    };
    let mut bucket_start_epochs = crate::stats::archive_bucket_start_epochs_from_bounds(
        None,
        Some(coverage_start_at),
        Some(coverage_end_at),
    )?;
    let mut reopened_file_paths = Vec::new();
    let mut reopened_file_path_set = HashSet::new();

    // Resetting an overlapping archive requires clearing its entire coverage before it can be
    // replayed. Keep expanding the overlap set until every reopened archive is represented.
    loop {
        let overlapping_archives =
            load_completed_invocation_archives_overlapping_usage_breakdown_buckets_tx(
                tx,
                &bucket_start_epochs,
            )
            .await?;
        let mut expanded = false;
        for overlapping_archive in overlapping_archives {
            let Some(expected_sha256) = overlapping_archive
                .sha256
                .as_deref()
                .filter(|sha256| !sha256.trim().is_empty())
            else {
                // An unverifiable overlap cannot be replayed after its rows are cleared.
                // Leave the complete closure quarantined rather than partially rebuilding it.
                return Ok(None);
            };
            if !reopened_file_path_set.insert(overlapping_archive.file_path.clone()) {
                continue;
            }
            let actual_sha256 = match crate::maintenance::sha256_hex_file(Path::new(
                &overlapping_archive.file_path,
            )) {
                Ok(value) => value,
                Err(_) => return Ok(None),
            };
            if actual_sha256 != expected_sha256 {
                return Ok(None);
            }
            reopened_file_paths.push(overlapping_archive.file_path);
            bucket_start_epochs.extend(crate::stats::archive_bucket_start_epochs_from_bounds(
                None,
                Some(&overlapping_archive.coverage_start_at),
                Some(&overlapping_archive.coverage_end_at),
            )?);
            expanded = true;
        }
        if !expanded {
            break;
        }
    }
    if !reopened_file_path_set.contains(file_path) {
        return Ok(None);
    }
    clear_invocation_rollup_rows_for_bucket_epochs_tx(tx, &bucket_start_epochs).await?;
    reset_invocation_archive_replay_state_tx(tx, &reopened_file_paths).await?;
    Ok(Some(reopened_file_paths))
}

include!("hourly_rollups/part-01.rs");
include!("hourly_rollups/part-02.rs");
include!("hourly_rollups/part-03.rs");
include!("hourly_rollups/part-04.rs");
include!("hourly_rollups/part-05.rs");
include!("hourly_rollups/part-06.rs");
include!("hourly_rollups/part-07.rs");
include!("hourly_rollups/part-08.rs");
