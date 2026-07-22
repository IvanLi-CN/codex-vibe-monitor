use super::*;
use anyhow::anyhow;
use sqlx::FromRow;
use tracing::warn;

#[path = "hourly_rollup_support.rs"]
mod archive_hourly_rollup_support;
pub(crate) use archive_hourly_rollup_support::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PoolAttemptFallbackCapability {
    Unavailable,
    IdOnly,
    Full,
}

fn payload_upstream_account_id_sql(invocation_ref: &str) -> String {
    format!(
        "CASE WHEN json_valid({invocation_ref}.payload) THEN CAST(json_extract({invocation_ref}.payload, '$.upstreamAccountId') AS INTEGER) END"
    )
}

fn invocation_upstream_account_id_with_attempt_id_fallback_sql(invocation_ref: &str) -> String {
    let payload_sql = payload_upstream_account_id_sql(invocation_ref);
    format!(
        "COALESCE(\
           {payload_sql}, \
           (SELECT attempt.upstream_account_id \
              FROM pool_upstream_request_attempts attempt \
             WHERE attempt.invoke_id = {invocation_ref}.invoke_id \
               AND attempt.occurred_at = {invocation_ref}.occurred_at \
               AND attempt.upstream_account_id IS NOT NULL \
             ORDER BY attempt.id DESC \
             LIMIT 1)\
         )"
    )
}

async fn load_pool_attempt_fallback_capability_tx(
    tx: &mut SqliteConnection,
) -> Result<PoolAttemptFallbackCapability> {
    let has_attempt_table = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'pool_upstream_request_attempts' LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if !has_attempt_table {
        return Ok(PoolAttemptFallbackCapability::Unavailable);
    }

    let has_attempt_index = sqlx::query_scalar::<_, String>(
        "SELECT name FROM pragma_table_info('pool_upstream_request_attempts') WHERE name = 'attempt_index' LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    Ok(if has_attempt_index {
        PoolAttemptFallbackCapability::Full
    } else {
        PoolAttemptFallbackCapability::IdOnly
    })
}

fn live_invocation_upstream_account_id_sql(
    invocation_ref: &str,
    capability: PoolAttemptFallbackCapability,
) -> String {
    match capability {
        PoolAttemptFallbackCapability::Unavailable => {
            payload_upstream_account_id_sql(invocation_ref)
        }
        PoolAttemptFallbackCapability::IdOnly => {
            invocation_upstream_account_id_with_attempt_id_fallback_sql(invocation_ref)
        }
        PoolAttemptFallbackCapability::Full => {
            crate::api::invocation_upstream_account_id_with_attempt_fallback_sql(invocation_ref)
        }
    }
}

pub(crate) async fn mark_retention_archived_hourly_rollup_targets_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    invocation_rows: &[InvocationHourlySourceRecord],
    forward_proxy_rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    match dataset {
        "codex_invocations" => {
            mark_retention_archived_invocation_hourly_rollup_targets_tx(tx, invocation_rows)
                .await?;
        }
        "forward_proxy_attempts" => {
            mark_forward_proxy_hourly_rollup_buckets_materialized_tx(tx, forward_proxy_rows)
                .await?;
        }
        _ => {}
    }
    Ok(())
}

async fn subtract_upstream_account_usage_breakdown_hourly_rows_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    let mut breakdowns = BTreeMap::new();
    for row in rows {
        accumulate_upstream_account_usage_breakdown_rollup(&mut breakdowns, row)?;
    }

    for (
        (
            bucket_start_epoch,
            source,
            upstream_account_key,
            _upstream_account_id,
            normalized_model,
            normalized_reasoning_effort,
        ),
        delta,
    ) in breakdowns
    {
        sqlx::query(
            r#"
            UPDATE upstream_account_usage_breakdown_hourly
            SET
                request_count = MAX(request_count - ?6, 0),
                success_count = MAX(success_count - ?7, 0),
                failure_count = MAX(failure_count - ?8, 0),
                cache_write_tokens = MAX(cache_write_tokens - ?9, 0),
                cache_read_tokens = MAX(cache_read_tokens - ?10, 0),
                output_tokens = MAX(output_tokens - ?11, 0),
                cost_input = MAX(cost_input - ?12, 0.0),
                cost_cache_write = MAX(cost_cache_write - ?13, 0.0),
                cost_cache_read = MAX(cost_cache_read - ?14, 0.0),
                cost_output = MAX(cost_output - ?15, 0.0),
                cost_reasoning = MAX(cost_reasoning - ?16, 0.0),
                cost_unknown = MAX(cost_unknown - ?17, 0.0),
                has_cost = MAX(has_cost - ?18, 0),
                performance_total_tokens = MAX(performance_total_tokens - ?19, 0),
                performance_stream_output_tokens = MAX(performance_stream_output_tokens - ?20, 0),
                performance_stream_duration_ms = MAX(performance_stream_duration_ms - ?21, 0.0),
                performance_response_sample_count = MAX(performance_response_sample_count - ?22, 0),
                performance_response_sum_ms = MAX(performance_response_sum_ms - ?23, 0.0),
                performance_first_byte_sample_count = MAX(performance_first_byte_sample_count - ?24, 0),
                performance_first_byte_sum_ms = MAX(performance_first_byte_sum_ms - ?25, 0.0),
                performance_usage_duration_sample_count = MAX(performance_usage_duration_sample_count - ?26, 0),
                performance_usage_duration_sum_ms = MAX(performance_usage_duration_sum_ms - ?27, 0.0),
                updated_at = datetime('now')
            WHERE bucket_start_epoch = ?1
              AND source = ?2
              AND upstream_account_key = ?3
              AND normalized_model = ?4
              AND normalized_reasoning_effort = ?5
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(&source)
        .bind(&upstream_account_key)
        .bind(&normalized_model)
        .bind(&normalized_reasoning_effort)
        .bind(delta.request_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.cache_write_tokens)
        .bind(delta.cache_read_tokens)
        .bind(delta.output_tokens)
        .bind(delta.cost_input)
        .bind(delta.cost_cache_write)
        .bind(delta.cost_cache_read)
        .bind(delta.cost_output)
        .bind(delta.cost_reasoning)
        .bind(delta.cost_unknown)
        .bind(delta.has_cost)
        .bind(delta.performance_total_tokens)
        .bind(delta.performance_stream_output_tokens)
        .bind(delta.performance_stream_duration_ms)
        .bind(delta.performance_response_sample_count)
        .bind(delta.performance_response_sum_ms)
        .bind(delta.performance_first_byte_sample_count)
        .bind(delta.performance_first_byte_sum_ms)
        .bind(delta.performance_usage_duration_sample_count)
        .bind(delta.performance_usage_duration_sum_ms)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            DELETE FROM upstream_account_usage_breakdown_hourly
            WHERE bucket_start_epoch = ?1
              AND source = ?2
              AND upstream_account_key = ?3
              AND normalized_model = ?4
              AND normalized_reasoning_effort = ?5
              AND request_count = 0
              AND success_count = 0
              AND failure_count = 0
              AND cache_write_tokens = 0
              AND cache_read_tokens = 0
              AND output_tokens = 0
              AND cost_input = 0.0
              AND cost_cache_write = 0.0
              AND cost_cache_read = 0.0
              AND cost_output = 0.0
              AND cost_reasoning = 0.0
              AND cost_unknown = 0.0
              AND has_cost = 0
              AND performance_total_tokens = 0
              AND performance_stream_output_tokens = 0
              AND performance_stream_duration_ms = 0.0
              AND performance_response_sample_count = 0
              AND performance_response_sum_ms = 0.0
              AND performance_first_byte_sample_count = 0
              AND performance_first_byte_sum_ms = 0.0
              AND performance_usage_duration_sample_count = 0
              AND performance_usage_duration_sum_ms = 0.0
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(&source)
        .bind(&upstream_account_key)
        .bind(&normalized_model)
        .bind(&normalized_reasoning_effort)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

async fn mark_retention_archived_invocation_hourly_rollup_targets_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    let mut overall_targets = HashSet::new();
    let mut upstream_account_usage_targets = HashSet::new();
    let mut sticky_targets = HashSet::new();
    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        overall_targets.insert((bucket_start_epoch, row.source.clone()));
        upstream_account_usage_targets.insert(bucket_start_epoch);
        sticky_targets.insert(bucket_start_epoch);
    }

    let live_targets = load_live_invocation_bucket_targets_tx(tx, &overall_targets).await?;
    let live_proxy_buckets = live_targets
        .iter()
        .filter_map(|(bucket_start_epoch, source)| {
            (source == SOURCE_PROXY).then_some(*bucket_start_epoch)
        })
        .collect::<HashSet<_>>();

    for (bucket_start_epoch, source) in overall_targets {
        if live_targets.contains(&(bucket_start_epoch, source.clone())) {
            continue;
        }
        for target in [
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        ] {
            mark_hourly_rollup_bucket_materialized_tx(tx, target, bucket_start_epoch, &source)
                .await?;
        }
        if source == SOURCE_PROXY && !live_proxy_buckets.contains(&bucket_start_epoch) {
            mark_hourly_rollup_bucket_materialized_tx(
                tx,
                HOURLY_ROLLUP_TARGET_PROXY_PERF,
                bucket_start_epoch,
                SOURCE_PROXY,
            )
            .await?;
        }
    }

    subtract_upstream_account_usage_breakdown_hourly_rows_tx(tx, rows).await?;

    for bucket_start_epoch in upstream_account_usage_targets {
        if live_targets
            .iter()
            .any(|(live_bucket_start_epoch, _)| *live_bucket_start_epoch == bucket_start_epoch)
        {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }

    for bucket_start_epoch in sticky_targets {
        if live_proxy_buckets.contains(&bucket_start_epoch) {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_STICKY_KEYS,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }

    Ok(())
}

pub(crate) const POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET: &str =
    "pool_upstream_node_health_archive";
pub(crate) const POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET: &str =
    "pool_upstream_node_health_hourly_archive";
pub(crate) const INVOCATION_USAGE_BREAKDOWN_ARCHIVE_PROGRESS_DATASET: &str =
    "codex_invocations_usage_breakdown_archive_progress";
pub(crate) const INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET: &str =
    "invocation_usage_breakdown_rollup_repair";
pub(crate) const INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET: &str =
    "invocation_usage_breakdown_rollup_repair_live_cursor";
const INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE: i64 = 1;

pub(crate) fn pool_upstream_node_health_archive_identity_for_batch_id(
    archive_batch_id: i64,
) -> String {
    format!("batch:{archive_batch_id}")
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PoolUpstreamNodeHealthArchiveRecord {
    pub(crate) archived_row_id: i64,
    pub(crate) occurred_at: String,
    pub(crate) proxy_binding_key_snapshot: String,
    pub(crate) is_success: i64,
    pub(crate) latency_ms: Option<f64>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PoolUpstreamNodeHealthHourlyArchiveRollupRow {
    pub(crate) proxy_binding_key_snapshot: String,
    pub(crate) bucket_start_epoch: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
}

pub(crate) async fn cache_pool_upstream_node_health_archive_rows_from_live_ids_tx(
    tx: &mut SqliteConnection,
    archive_file_path: &str,
    ids: &[i64],
) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }

    let mut rows_affected = 0_u64;
    for chunk in ids.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            r#"
            INSERT INTO pool_upstream_node_health_archive (
                archive_file_path,
                archived_row_id,
                occurred_at,
                proxy_binding_key_snapshot,
                is_success,
                latency_ms,
                updated_at
            )
            SELECT
                "#,
        );
        query
            .push_bind(archive_file_path)
            .push(
                r#",
                id,
                occurred_at,
                proxy_binding_key_snapshot,
                CASE WHEN status = "#,
            )
            .push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
            .push(
                r#" THEN 1 ELSE 0 END,
                CASE
                    WHEN status = "#,
            )
            .push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
            .push(
                r#" THEN COALESCE(first_byte_latency_ms, connect_latency_ms, stream_latency_ms)
                    ELSE NULL
                END,
                datetime('now')
            FROM pool_upstream_request_attempts
            WHERE id IN ("#,
            );
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(id);
            }
        }
        query.push(
            r#")
              AND proxy_binding_key_snapshot IS NOT NULL
              AND finished_at IS NOT NULL
              AND status != "#,
        );
        query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL);
        query.push(
            r#"
            ON CONFLICT(archive_file_path, archived_row_id) DO UPDATE SET
                occurred_at = excluded.occurred_at,
                proxy_binding_key_snapshot = excluded.proxy_binding_key_snapshot,
                is_success = excluded.is_success,
                latency_ms = excluded.latency_ms,
                updated_at = datetime('now')
            "#,
        );
        rows_affected += query.build().execute(&mut *tx).await?.rows_affected();
    }

    Ok(rows_affected)
}

pub(crate) async fn load_pool_upstream_node_health_archive_rows_chunk(
    archive_pool: &Pool<Sqlite>,
    start_after_id: i64,
) -> Result<(Vec<PoolUpstreamNodeHealthArchiveRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, PoolUpstreamNodeHealthArchiveRecord>(
        r#"
        SELECT
            id AS archived_row_id,
            occurred_at,
            proxy_binding_key_snapshot,
            CASE WHEN status = ?1 THEN 1 ELSE 0 END AS is_success,
            CASE
                WHEN status = ?1 THEN COALESCE(first_byte_latency_ms, connect_latency_ms, stream_latency_ms)
                ELSE NULL
            END AS latency_ms
        FROM pool_upstream_request_attempts
        WHERE id > ?2
          AND proxy_binding_key_snapshot IS NOT NULL
          AND finished_at IS NOT NULL
          AND status != ?3
        ORDER BY id ASC
        LIMIT ?4
        "#,
    )
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
    .bind(start_after_id)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL)
    .bind(BACKFILL_BATCH_SIZE + 1)
    .fetch_all(archive_pool)
    .await?;

    let has_more = rows.len() as i64 > BACKFILL_BATCH_SIZE;
    if has_more {
        rows.truncate(BACKFILL_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn upsert_pool_upstream_node_health_archive_rows_tx(
    tx: &mut SqliteConnection,
    archive_file_path: &str,
    rows: &[PoolUpstreamNodeHealthArchiveRecord],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    for chunk in rows.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO pool_upstream_node_health_archive (archive_file_path, archived_row_id, occurred_at, proxy_binding_key_snapshot, is_success, latency_ms, updated_at) ",
        );
        query.push_values(chunk, |mut row, value| {
            row.push_bind(archive_file_path)
                .push_bind(value.archived_row_id)
                .push_bind(&value.occurred_at)
                .push_bind(&value.proxy_binding_key_snapshot)
                .push_bind(value.is_success)
                .push_bind(value.latency_ms)
                .push("datetime('now')");
        });
        query.push(
            " ON CONFLICT(archive_file_path, archived_row_id) DO UPDATE SET \
              occurred_at = excluded.occurred_at, \
              proxy_binding_key_snapshot = excluded.proxy_binding_key_snapshot, \
              is_success = excluded.is_success, \
              latency_ms = excluded.latency_ms, \
              updated_at = datetime('now')",
        );
        query.build().execute(&mut *tx).await?;
    }

    Ok(())
}

pub(crate) async fn delete_pool_upstream_node_health_archive_rows_for_file_tx(
    tx: &mut SqliteConnection,
    archive_file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_node_health_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(archive_file_path)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn delete_pool_upstream_node_health_hourly_archive_rows_for_batch_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_node_health_hourly_archive
        WHERE archive_batch_id = ?1
        "#,
    )
    .bind(archive_batch_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn refresh_pool_upstream_node_health_hourly_archive_rows_from_cache_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
    archive_file_path: &str,
) -> Result<u64> {
    let rows = sqlx::query_as::<_, PoolUpstreamNodeHealthHourlyArchiveRollupRow>(
        r#"
        SELECT
            proxy_binding_key_snapshot,
            ((CASE
                WHEN instr(occurred_at, 'T') > 0
                    THEN CAST(strftime('%s', occurred_at) AS INTEGER)
                ELSE CAST(strftime('%s', occurred_at || '+08:00') AS INTEGER)
            END) / 3600) * 3600 AS bucket_start_epoch,
            SUM(is_success) AS success_count,
            SUM(CASE WHEN is_success = 0 THEN 1 ELSE 0 END) AS failure_count
        FROM pool_upstream_node_health_archive
        WHERE archive_file_path = ?1
        GROUP BY proxy_binding_key_snapshot, bucket_start_epoch
        "#,
    )
    .bind(archive_file_path)
    .fetch_all(&mut *tx)
    .await
    .with_context(|| {
        format!(
            "failed to rebuild cached pool upstream node health hourly rows for {}",
            archive_file_path
        )
    })?;

    replace_pool_upstream_node_health_hourly_archive_rows_tx(
        tx,
        archive_batch_id,
        archive_file_path,
        &rows,
    )
    .await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replace_pool_upstream_node_health_hourly_archive_rows_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
    archive_file_path: &str,
    rows: &[PoolUpstreamNodeHealthHourlyArchiveRollupRow],
) -> Result<()> {
    delete_pool_upstream_node_health_hourly_archive_rows_for_batch_tx(tx, archive_batch_id).await?;
    if rows.is_empty() {
        return Ok(());
    }

    let archive_identity =
        pool_upstream_node_health_archive_identity_for_batch_id(archive_batch_id);

    for chunk in rows.chunks(BACKFILL_ACCOUNT_BIND_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO pool_upstream_node_health_hourly_archive (archive_identity, archive_batch_id, archive_file_path, proxy_binding_key_snapshot, bucket_start_epoch, success_count, failure_count, updated_at) ",
        );
        query.push_values(chunk, |mut row, value| {
            row.push_bind(&archive_identity)
                .push_bind(archive_batch_id)
                .push_bind(archive_file_path)
                .push_bind(&value.proxy_binding_key_snapshot)
                .push_bind(value.bucket_start_epoch)
                .push_bind(value.success_count)
                .push_bind(value.failure_count)
                .push("datetime('now')");
        });
        query.push(
            " ON CONFLICT(archive_identity, proxy_binding_key_snapshot, bucket_start_epoch) DO UPDATE SET \
              archive_batch_id = excluded.archive_batch_id, \
              archive_file_path = excluded.archive_file_path, \
              success_count = excluded.success_count, \
              failure_count = excluded.failure_count, \
              updated_at = datetime('now')",
        );
        query.build().execute(&mut *tx).await?;
    }

    Ok(())
}

pub(crate) async fn load_archive_table_columns(
    pool: &Pool<Sqlite>,
    table_name: &str,
) -> Result<HashSet<String>> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await
        .with_context(|| format!("failed to inspect {table_name} schema"))?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    Ok(columns)
}

pub(crate) fn legacy_compatible_archive_select_expr(
    archive_columns: &HashSet<String>,
    column_name: &str,
) -> String {
    if archive_columns.contains(column_name) {
        column_name.to_string()
    } else {
        format!("NULL AS {column_name}")
    }
}

pub(crate) fn build_legacy_compatible_invocation_archive_query(
    archive_columns: &HashSet<String>,
) -> String {
    let model = legacy_compatible_archive_select_expr(archive_columns, "model");
    let input_tokens = legacy_compatible_archive_select_expr(archive_columns, "input_tokens");
    let output_tokens = legacy_compatible_archive_select_expr(archive_columns, "output_tokens");
    let cache_input_tokens =
        legacy_compatible_archive_select_expr(archive_columns, "cache_input_tokens");
    let upstream_account_id =
        legacy_compatible_archive_select_expr(archive_columns, "upstream_account_id");
    let cost_input = legacy_compatible_archive_select_expr(archive_columns, "cost_input");
    let cost_cache_write =
        legacy_compatible_archive_select_expr(archive_columns, "cost_cache_write");
    let cost_cache_read = legacy_compatible_archive_select_expr(archive_columns, "cost_cache_read");
    let cost_output = legacy_compatible_archive_select_expr(archive_columns, "cost_output");
    let cost_reasoning = legacy_compatible_archive_select_expr(archive_columns, "cost_reasoning");
    format!(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            {model},
            {input_tokens},
            {output_tokens},
            {cache_input_tokens},
            total_tokens,
            cost,
            {upstream_account_id},
            {cost_input},
            {cost_cache_write},
            {cost_cache_read},
            {cost_output},
            {cost_reasoning},
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#
    )
}

pub(crate) async fn mark_archive_batch_historical_rollups_materialized_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    delete_hourly_rollup_archive_progress_tx(tx, dataset, file_path).await?;
    Ok(())
}

pub(crate) async fn load_hourly_rollup_archive_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    file_path: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT cursor_id
        FROM hourly_rollup_archive_progress
        WHERE dataset = ?1
          AND file_path = ?2
        LIMIT 1
        "#,
    )
    .bind(dataset)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0)
    .max(0))
}

pub(crate) async fn save_hourly_rollup_archive_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    file_path: &str,
    cursor_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_progress (
            dataset,
            file_path,
            cursor_id,
            updated_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        ON CONFLICT(dataset, file_path) DO UPDATE SET
            cursor_id = MAX(hourly_rollup_archive_progress.cursor_id, excluded.cursor_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(dataset)
    .bind(file_path)
    .bind(cursor_id.max(0))
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn delete_hourly_rollup_archive_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_archive_progress
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn update_archive_batch_coverage_bounds_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET coverage_start_at = COALESCE(coverage_start_at, ?2),
            coverage_end_at = COALESCE(coverage_end_at, ?3)
        WHERE id = ?1
        "#,
    )
    .bind(archive_batch_id)
    .bind(coverage_start_at)
    .bind(coverage_end_at)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn mark_hourly_rollup_bucket_materialized_tx(
    tx: &mut SqliteConnection,
    target: &str,
    bucket_start_epoch: i64,
    source: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        ON CONFLICT(target, bucket_start_epoch, source) DO UPDATE SET
            materialized_at = datetime('now')
        "#,
    )
    .bind(target)
    .bind(bucket_start_epoch)
    .bind(source)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn mark_invocation_hourly_rollup_buckets_materialized_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    let mut overall_targets = HashSet::new();
    let mut upstream_account_usage_targets = HashSet::new();
    let mut sticky_targets = HashSet::new();
    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        overall_targets.insert((bucket_start_epoch, row.source.clone()));
        upstream_account_usage_targets.insert(bucket_start_epoch);
        sticky_targets.insert(bucket_start_epoch);
    }

    let live_targets = load_live_invocation_bucket_targets_tx(tx, &overall_targets).await?;
    let live_proxy_buckets = live_targets
        .iter()
        .filter_map(|(bucket_start_epoch, source)| {
            (source == SOURCE_PROXY).then_some(*bucket_start_epoch)
        })
        .collect::<HashSet<_>>();

    for (bucket_start_epoch, source) in overall_targets {
        if live_targets.contains(&(bucket_start_epoch, source.clone())) {
            continue;
        }
        for target in [
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        ] {
            mark_hourly_rollup_bucket_materialized_tx(tx, target, bucket_start_epoch, &source)
                .await?;
        }
        if source == SOURCE_PROXY && !live_proxy_buckets.contains(&bucket_start_epoch) {
            mark_hourly_rollup_bucket_materialized_tx(
                tx,
                HOURLY_ROLLUP_TARGET_PROXY_PERF,
                bucket_start_epoch,
                SOURCE_PROXY,
            )
            .await?;
        }
    }

    for bucket_start_epoch in upstream_account_usage_targets {
        if live_targets
            .iter()
            .any(|(live_bucket_start_epoch, _)| *live_bucket_start_epoch == bucket_start_epoch)
        {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }

    for bucket_start_epoch in sticky_targets {
        if live_proxy_buckets.contains(&bucket_start_epoch) {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_STICKY_KEYS,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }

    Ok(())
}

pub(crate) async fn load_live_invocation_bucket_targets_tx(
    tx: &mut SqliteConnection,
    bucket_targets: &HashSet<(i64, String)>,
) -> Result<HashSet<(i64, String)>> {
    if bucket_targets.is_empty() {
        return Ok(HashSet::new());
    }

    let min_bucket_epoch = bucket_targets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum invocation bucket epoch"))?;
    let max_bucket_epoch = bucket_targets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum invocation bucket epoch"))?;
    let min_bucket_start = Utc
        .timestamp_opt(min_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum invocation bucket epoch"))?;
    let max_bucket_end = Utc
        .timestamp_opt(max_bucket_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum invocation bucket epoch"))?;

    let rows = sqlx::query_as::<_, InvocationBucketPresenceRow>(
        r#"
        SELECT occurred_at, source
        FROM codex_invocations
        WHERE occurred_at >= ?1
          AND occurred_at < ?2
        ORDER BY id ASC
        "#,
    )
    .bind(db_occurred_at_lower_bound(min_bucket_start))
    .bind(db_occurred_at_lower_bound(max_bucket_end))
    .fetch_all(&mut *tx)
    .await?;

    let mut live_targets = HashSet::new();
    for row in rows {
        let key = (invocation_bucket_start_epoch(&row.occurred_at)?, row.source);
        if bucket_targets.contains(&key) {
            live_targets.insert(key);
        }
    }
    Ok(live_targets)
}

pub(crate) async fn mark_forward_proxy_hourly_rollup_buckets_materialized_tx(
    tx: &mut SqliteConnection,
    rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    let mut buckets = HashSet::new();
    for row in rows {
        buckets.insert(forward_proxy_attempt_bucket_start_epoch(&row.occurred_at)?);
    }
    for bucket_start_epoch in buckets {
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn upsert_invocation_rollups(
    tx: &mut sqlx::SqliteConnection,
    candidates: &[InvocationArchiveCandidate],
) -> Result<()> {
    let mut rollups: BTreeMap<(String, String), InvocationRollupDelta> = BTreeMap::new();
    for candidate in candidates {
        let stats_date = shanghai_day_key_from_local_naive(&candidate.occurred_at)?;
        let key = (stats_date, candidate.source.clone());
        let entry = rollups.entry(key).or_default();
        entry.total_count += 1;
        if matches!(candidate.status.as_deref(), Some("success")) {
            entry.success_count += 1;
        } else if candidate
            .status
            .as_deref()
            .is_some_and(|status| status != "success")
        {
            entry.failure_count += 1;
        }
        entry.total_tokens += candidate.total_tokens.unwrap_or_default();
        entry.total_cost += candidate.cost.unwrap_or_default();
    }

    for ((stats_date, source), delta) in rollups {
        sqlx::query(
            r#"
            INSERT INTO invocation_rollup_daily (
                stats_date,
                source,
                total_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
            ON CONFLICT(stats_date, source) DO UPDATE SET
                total_count = invocation_rollup_daily.total_count + excluded.total_count,
                success_count = invocation_rollup_daily.success_count + excluded.success_count,
                failure_count = invocation_rollup_daily.failure_count + excluded.failure_count,
                total_tokens = invocation_rollup_daily.total_tokens + excluded.total_tokens,
                total_cost = invocation_rollup_daily.total_cost + excluded.total_cost
            "#,
        )
        .bind(&stats_date)
        .bind(&source)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.total_tokens)
        .bind(delta.total_cost)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn upsert_invocation_hourly_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
    targets: &[&str],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let upsert_overall = targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATIONS);
    let upsert_failures = targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES);
    let upsert_perf = targets.contains(&HOURLY_ROLLUP_TARGET_PROXY_PERF);
    let upsert_prompt_cache = targets.contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE);
    let upsert_prompt_cache_upstream_accounts =
        targets.contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS);
    let upsert_upstream_account_usage =
        targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE);
    let upsert_upstream_account_usage_breakdown =
        targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN);
    let upsert_upstream_account_stats_hourly =
        targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY);
    let upsert_upstream_account_stats_minute =
        targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE);
    let upsert_sticky_keys = targets.contains(&HOURLY_ROLLUP_TARGET_STICKY_KEYS);

    let mut overall: BTreeMap<(i64, String), InvocationHourlyRollupDelta> = BTreeMap::new();
    let mut failures: BTreeMap<(i64, String, String, i64, String), i64> = BTreeMap::new();
    let mut perf: BTreeMap<(i64, String), ProxyPerfStageHourlyDelta> = BTreeMap::new();
    let mut prompt_cache: BTreeMap<(i64, String, String), KeyedConversationHourlyDelta> =
        BTreeMap::new();
    let mut prompt_cache_upstream_accounts: BTreeMap<
        (i64, String, String, String, Option<i64>, Option<String>),
        KeyedConversationHourlyDelta,
    > = BTreeMap::new();
    let mut upstream_account_usage: BTreeMap<(i64, i64), UpstreamAccountUsageHourlyDelta> =
        BTreeMap::new();
    let mut upstream_account_usage_breakdown: BTreeMap<
        (i64, String, String, Option<i64>, String, String),
        UpstreamAccountUsageBreakdownHourlyDelta,
    > = BTreeMap::new();
    let mut upstream_account_stats_hourly: BTreeMap<(i64, String, i64), UpstreamAccountStatsDelta> =
        BTreeMap::new();
    let mut upstream_account_stats_minute: BTreeMap<(i64, String, i64), UpstreamAccountStatsDelta> =
        BTreeMap::new();
    let mut sticky_keys: BTreeMap<(i64, i64, String), KeyedConversationHourlyDelta> =
        BTreeMap::new();

    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        if upsert_overall {
            accumulate_invocation_hourly_overall_rollups(&mut overall, std::slice::from_ref(row))?;
        }

        if upsert_failures {
            let classification = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            if invocation_status_counts_toward_terminal_totals(row.status.as_deref())
                && classification.failure_class != FailureClass::None
            {
                let error_category =
                    categorize_error(row.error_message.as_deref().unwrap_or_default());
                *failures
                    .entry((
                        bucket_start_epoch,
                        row.source.clone(),
                        classification.failure_class.as_str().to_string(),
                        classification.is_actionable as i64,
                        error_category,
                    ))
                    .or_default() += 1;
            }
        }

        if upsert_perf && row.source == SOURCE_PROXY {
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_TOTAL,
                row.t_total_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_REQUEST_READ,
                row.t_req_read_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_REQUEST_PARSE,
                row.t_req_parse_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_UPSTREAM_CONNECT,
                row.t_upstream_connect_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_UPSTREAM_FIRST_BYTE,
                row.t_upstream_ttfb_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_UPSTREAM_STREAM,
                row.t_upstream_stream_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_RESPONSE_PARSE,
                row.t_resp_parse_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_PERSISTENCE,
                row.t_persist_ms,
            );
        }

        if (upsert_prompt_cache || upsert_prompt_cache_upstream_accounts)
            && let Some(prompt_cache_key) = prompt_cache_key_from_payload(row.payload.as_deref())
        {
            let classification = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            let is_success_like = invocation_status_is_success_like(
                row.status.as_deref(),
                row.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None;
            if upsert_prompt_cache {
                let entry = keyed_conversation_delta(
                    &mut prompt_cache,
                    bucket_start_epoch,
                    &row.source,
                    &prompt_cache_key,
                    &row.occurred_at,
                );
                entry.request_count += 1;
                if is_success_like {
                    entry.success_count += 1;
                } else {
                    entry.failure_count += 1;
                }
                entry.total_tokens += row.total_tokens.unwrap_or_default();
                entry.total_cost += row.cost.unwrap_or_default();
            }

            if upsert_prompt_cache_upstream_accounts {
                let upstream_account_id = row.resolved_upstream_account_id();
                let upstream_account_name =
                    upstream_account_name_from_payload(row.payload.as_deref());
                let rollup_key = prompt_cache_upstream_account_rollup_key(
                    upstream_account_id,
                    upstream_account_name.as_deref(),
                );
                let entry = prompt_cache_upstream_accounts
                    .entry((
                        bucket_start_epoch,
                        row.source.clone(),
                        prompt_cache_key,
                        rollup_key,
                        upstream_account_id,
                        upstream_account_name.clone(),
                    ))
                    .or_insert_with(|| KeyedConversationHourlyDelta {
                        first_seen_at: row.occurred_at.clone(),
                        last_seen_at: row.occurred_at.clone(),
                        ..KeyedConversationHourlyDelta::default()
                    });
                if row.occurred_at < entry.first_seen_at {
                    entry.first_seen_at = row.occurred_at.clone();
                }
                if row.occurred_at > entry.last_seen_at {
                    entry.last_seen_at = row.occurred_at.clone();
                }
                entry.request_count += 1;
                if is_success_like {
                    entry.success_count += 1;
                } else {
                    entry.failure_count += 1;
                }
                entry.total_tokens += row.total_tokens.unwrap_or_default();
                entry.total_cost += row.cost.unwrap_or_default();
            }
        }

        if upsert_upstream_account_usage
            && let Some(upstream_account_id) = row.resolved_upstream_account_id()
        {
            let entry = upstream_account_usage
                .entry((bucket_start_epoch, upstream_account_id))
                .or_insert_with(|| UpstreamAccountUsageHourlyDelta {
                    first_seen_at: row.occurred_at.clone(),
                    last_seen_at: row.occurred_at.clone(),
                    ..UpstreamAccountUsageHourlyDelta::default()
                });
            if row.occurred_at < entry.first_seen_at {
                entry.first_seen_at = row.occurred_at.clone();
            }
            if row.occurred_at > entry.last_seen_at {
                entry.last_seen_at = row.occurred_at.clone();
            }
            entry.request_count += 1;
            let classification = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            if invocation_status_is_success_like(
                row.status.as_deref(),
                row.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None
            {
                entry.success_count += 1;
            } else if invocation_status_counts_toward_terminal_totals(row.status.as_deref())
                && classification.failure_class != FailureClass::None
            {
                entry.failure_count += 1;
            }
            entry.total_tokens += row.total_tokens.unwrap_or_default();
            let cost = row.cost.unwrap_or_default();
            entry.total_cost += cost;
            if invocation_counts_toward_non_success_usage(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            ) {
                entry.non_success_cost += cost;
            }
            entry.input_tokens += row.input_tokens.unwrap_or_default();
            entry.output_tokens += row.output_tokens.unwrap_or_default();
            entry.cache_input_tokens += row.cache_input_tokens.unwrap_or_default();
        }

        if upsert_upstream_account_usage_breakdown {
            accumulate_upstream_account_usage_breakdown_rollup(
                &mut upstream_account_usage_breakdown,
                row,
            )?;
        }

        if (upsert_upstream_account_stats_hourly || upsert_upstream_account_stats_minute)
            && let Some(upstream_account_id) = row.resolved_upstream_account_id()
        {
            if upsert_upstream_account_stats_hourly {
                let entry = upstream_account_stats_hourly
                    .entry((bucket_start_epoch, row.source.clone(), upstream_account_id))
                    .or_insert_with(|| UpstreamAccountStatsDelta {
                        first_byte_histogram: empty_approx_histogram(),
                        first_response_byte_total_histogram: empty_approx_histogram(),
                        ..UpstreamAccountStatsDelta::default()
                    });
                accumulate_upstream_account_stats_delta(entry, row);
            }

            if upsert_upstream_account_stats_minute {
                let minute_bucket_start_epoch =
                    invocation_bucket_start_epoch_for_seconds(&row.occurred_at, 60)?;
                let entry = upstream_account_stats_minute
                    .entry((
                        minute_bucket_start_epoch,
                        row.source.clone(),
                        upstream_account_id,
                    ))
                    .or_insert_with(|| UpstreamAccountStatsDelta {
                        first_byte_histogram: empty_approx_histogram(),
                        first_response_byte_total_histogram: empty_approx_histogram(),
                        ..UpstreamAccountStatsDelta::default()
                    });
                accumulate_upstream_account_stats_delta(entry, row);
            }
        }

        if upsert_sticky_keys
            && let (Some(upstream_account_id), Some(sticky_key)) = (
                row.resolved_upstream_account_id(),
                sticky_key_from_payload(row.payload.as_deref()),
            )
        {
            let entry = sticky_keys
                .entry((bucket_start_epoch, upstream_account_id, sticky_key))
                .or_insert_with(|| KeyedConversationHourlyDelta {
                    first_seen_at: row.occurred_at.clone(),
                    last_seen_at: row.occurred_at.clone(),
                    ..KeyedConversationHourlyDelta::default()
                });
            if row.occurred_at < entry.first_seen_at {
                entry.first_seen_at = row.occurred_at.clone();
            }
            if row.occurred_at > entry.last_seen_at {
                entry.last_seen_at = row.occurred_at.clone();
            }
            entry.request_count += 1;
            let classification = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            if invocation_status_is_success_like(
                row.status.as_deref(),
                row.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None
            {
                entry.success_count += 1;
            } else {
                entry.failure_count += 1;
            }
            entry.total_tokens += row.total_tokens.unwrap_or_default();
            entry.total_cost += row.cost.unwrap_or_default();
        }
    }

    if upsert_overall {
        #[derive(sqlx::FromRow)]
        struct InvocationRollupHistogramRow {
            first_byte_histogram: String,
            first_response_byte_total_histogram: String,
        }

        for ((bucket_start_epoch, source), delta) in overall {
            let current_histograms = sqlx::query_as::<_, InvocationRollupHistogramRow>(
                r#"
                SELECT
                    first_byte_histogram,
                    first_response_byte_total_histogram
                FROM invocation_rollup_hourly
                WHERE bucket_start_epoch = ?1 AND source = ?2
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .fetch_optional(&mut *tx)
            .await?;
            let mut merged_first_byte_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_byte_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_byte_histogram,
                &delta.first_byte_histogram,
            )?;
            let mut merged_first_response_byte_total_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_response_byte_total_histogram,
                &delta.first_response_byte_total_histogram,
            )?;
            sqlx::query(
                r#"
                INSERT INTO invocation_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    total_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    cache_input_tokens,
                    total_cost,
                    non_success_cost,
                    total_latency_sample_count,
                    total_latency_sum_ms,
                    first_byte_sample_count,
                    first_byte_sum_ms,
                    first_byte_max_ms,
                    first_byte_histogram,
                    first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms,
                    first_response_byte_total_histogram,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET
                    total_count = invocation_rollup_hourly.total_count + excluded.total_count,
                    success_count = invocation_rollup_hourly.success_count + excluded.success_count,
                    failure_count = invocation_rollup_hourly.failure_count + excluded.failure_count,
                    total_tokens = invocation_rollup_hourly.total_tokens + excluded.total_tokens,
                    cache_input_tokens = invocation_rollup_hourly.cache_input_tokens + excluded.cache_input_tokens,
                    total_cost = invocation_rollup_hourly.total_cost + excluded.total_cost,
                    non_success_cost = invocation_rollup_hourly.non_success_cost + excluded.non_success_cost,
                    total_latency_sample_count = invocation_rollup_hourly.total_latency_sample_count + excluded.total_latency_sample_count,
                    total_latency_sum_ms = invocation_rollup_hourly.total_latency_sum_ms + excluded.total_latency_sum_ms,
                    first_byte_sample_count = invocation_rollup_hourly.first_byte_sample_count + excluded.first_byte_sample_count,
                    first_byte_sum_ms = invocation_rollup_hourly.first_byte_sum_ms + excluded.first_byte_sum_ms,
                    first_byte_max_ms = MAX(invocation_rollup_hourly.first_byte_max_ms, excluded.first_byte_max_ms),
                    first_byte_histogram = excluded.first_byte_histogram,
                    first_response_byte_total_sample_count = invocation_rollup_hourly.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms = invocation_rollup_hourly.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms = MAX(invocation_rollup_hourly.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
                    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.cache_input_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.total_latency_sample_count)
            .bind(delta.total_latency_sum_ms)
            .bind(delta.first_byte_sample_count)
            .bind(delta.first_byte_sum_ms)
            .bind(delta.first_byte_max_ms)
            .bind(encode_approx_histogram(&merged_first_byte_histogram)?)
            .bind(delta.first_response_byte_total_sample_count)
            .bind(delta.first_response_byte_total_sum_ms)
            .bind(delta.first_response_byte_total_max_ms)
            .bind(encode_approx_histogram(
                &merged_first_response_byte_total_histogram,
            )?)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_failures {
        for (
            (bucket_start_epoch, source, failure_class, is_actionable, error_category),
            failure_count,
        ) in failures
        {
            sqlx::query(
                r#"
                INSERT INTO invocation_failure_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    failure_class,
                    is_actionable,
                    error_category,
                    failure_count,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, failure_class, is_actionable, error_category) DO UPDATE SET
                    failure_count = invocation_failure_rollup_hourly.failure_count + excluded.failure_count,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&failure_class)
            .bind(is_actionable)
            .bind(&error_category)
            .bind(failure_count)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_perf {
        for ((bucket_start_epoch, stage), delta) in perf {
            let current_histogram = sqlx::query_scalar::<_, String>(
                r#"
                SELECT histogram
                FROM proxy_perf_stage_hourly
                WHERE bucket_start_epoch = ?1 AND stage = ?2
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&stage)
            .fetch_optional(&mut *tx)
            .await?;
            let mut merged_histogram = current_histogram
                .as_deref()
                .map(decode_approx_histogram)
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(&mut merged_histogram, &delta.histogram)?;
            sqlx::query(
                r#"
                INSERT INTO proxy_perf_stage_hourly (
                    bucket_start_epoch,
                    stage,
                    sample_count,
                    sum_ms,
                    max_ms,
                    histogram,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                ON CONFLICT(bucket_start_epoch, stage) DO UPDATE SET
                    sample_count = proxy_perf_stage_hourly.sample_count + excluded.sample_count,
                    sum_ms = proxy_perf_stage_hourly.sum_ms + excluded.sum_ms,
                    max_ms = MAX(proxy_perf_stage_hourly.max_ms, excluded.max_ms),
                    histogram = excluded.histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&stage)
            .bind(delta.sample_count)
            .bind(delta.sum_ms)
            .bind(delta.max_ms)
            .bind(encode_approx_histogram(&merged_histogram)?)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_prompt_cache {
        for ((bucket_start_epoch, source, prompt_cache_key), delta) in prompt_cache {
            sqlx::query(
                r#"
                INSERT INTO prompt_cache_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    prompt_cache_key,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, prompt_cache_key) DO UPDATE SET
                    request_count = prompt_cache_rollup_hourly.request_count + excluded.request_count,
                    success_count = prompt_cache_rollup_hourly.success_count + excluded.success_count,
                    failure_count = prompt_cache_rollup_hourly.failure_count + excluded.failure_count,
                    total_tokens = prompt_cache_rollup_hourly.total_tokens + excluded.total_tokens,
                    total_cost = prompt_cache_rollup_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(prompt_cache_rollup_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(prompt_cache_rollup_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&prompt_cache_key)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_prompt_cache_upstream_accounts {
        for (
            (
                bucket_start_epoch,
                source,
                prompt_cache_key,
                upstream_account_key,
                upstream_account_id,
                upstream_account_name,
            ),
            delta,
        ) in prompt_cache_upstream_accounts
        {
            sqlx::query(
                r#"
                INSERT INTO prompt_cache_upstream_account_hourly (
                    bucket_start_epoch,
                    source,
                    prompt_cache_key,
                    upstream_account_key,
                    upstream_account_id,
                    upstream_account_name,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, prompt_cache_key, upstream_account_key) DO UPDATE SET
                    request_count = prompt_cache_upstream_account_hourly.request_count + excluded.request_count,
                    success_count = prompt_cache_upstream_account_hourly.success_count + excluded.success_count,
                    failure_count = prompt_cache_upstream_account_hourly.failure_count + excluded.failure_count,
                    total_tokens = prompt_cache_upstream_account_hourly.total_tokens + excluded.total_tokens,
                    total_cost = prompt_cache_upstream_account_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(prompt_cache_upstream_account_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(prompt_cache_upstream_account_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&prompt_cache_key)
            .bind(&upstream_account_key)
            .bind(upstream_account_id)
            .bind(upstream_account_name.as_deref())
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_upstream_account_usage {
        for ((bucket_start_epoch, upstream_account_id), delta) in upstream_account_usage {
            sqlx::query(
                r#"
                INSERT INTO upstream_account_usage_hourly (
                    bucket_start_epoch,
                    upstream_account_id,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    non_success_cost,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))
                ON CONFLICT(bucket_start_epoch, upstream_account_id) DO UPDATE SET
                    request_count = upstream_account_usage_hourly.request_count + excluded.request_count,
                    success_count = upstream_account_usage_hourly.success_count + excluded.success_count,
                    failure_count = upstream_account_usage_hourly.failure_count + excluded.failure_count,
                    total_tokens = upstream_account_usage_hourly.total_tokens + excluded.total_tokens,
                    total_cost = upstream_account_usage_hourly.total_cost + excluded.total_cost,
                    non_success_cost = upstream_account_usage_hourly.non_success_cost + excluded.non_success_cost,
                    input_tokens = upstream_account_usage_hourly.input_tokens + excluded.input_tokens,
                    output_tokens = upstream_account_usage_hourly.output_tokens + excluded.output_tokens,
                    cache_input_tokens = upstream_account_usage_hourly.cache_input_tokens + excluded.cache_input_tokens,
                    first_seen_at = MIN(upstream_account_usage_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(upstream_account_usage_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(upstream_account_id)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.input_tokens)
            .bind(delta.output_tokens)
            .bind(delta.cache_input_tokens)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_upstream_account_usage_breakdown {
        for (
            (
                bucket_start_epoch,
                source,
                upstream_account_key,
                upstream_account_id,
                normalized_model,
                normalized_reasoning_effort,
            ),
            delta,
        ) in upstream_account_usage_breakdown
        {
            sqlx::query(
                r#"
                INSERT INTO upstream_account_usage_breakdown_hourly (
                    bucket_start_epoch,
                    source,
                    upstream_account_key,
                    upstream_account_id,
                    normalized_model,
                    normalized_reasoning_effort,
                    request_count,
                    success_count,
                    failure_count,
                    cache_write_tokens,
                    cache_read_tokens,
                    output_tokens,
                    cost_input,
                    cost_cache_write,
                    cost_cache_read,
                    cost_output,
                    cost_reasoning,
                    cost_unknown,
                    has_cost,
                    performance_total_tokens,
                    performance_stream_output_tokens,
                    performance_stream_duration_ms,
                    performance_response_sample_count,
                    performance_response_sum_ms,
                    performance_first_byte_sample_count,
                    performance_first_byte_sum_ms,
                    performance_usage_duration_sample_count,
                    performance_usage_duration_sum_ms,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort) DO UPDATE SET
                    request_count = upstream_account_usage_breakdown_hourly.request_count + excluded.request_count,
                    success_count = upstream_account_usage_breakdown_hourly.success_count + excluded.success_count,
                    failure_count = upstream_account_usage_breakdown_hourly.failure_count + excluded.failure_count,
                    cache_write_tokens = upstream_account_usage_breakdown_hourly.cache_write_tokens + excluded.cache_write_tokens,
                    cache_read_tokens = upstream_account_usage_breakdown_hourly.cache_read_tokens + excluded.cache_read_tokens,
                    output_tokens = upstream_account_usage_breakdown_hourly.output_tokens + excluded.output_tokens,
                    cost_input = upstream_account_usage_breakdown_hourly.cost_input + excluded.cost_input,
                    cost_cache_write = upstream_account_usage_breakdown_hourly.cost_cache_write + excluded.cost_cache_write,
                    cost_cache_read = upstream_account_usage_breakdown_hourly.cost_cache_read + excluded.cost_cache_read,
                    cost_output = upstream_account_usage_breakdown_hourly.cost_output + excluded.cost_output,
                    cost_reasoning = upstream_account_usage_breakdown_hourly.cost_reasoning + excluded.cost_reasoning,
                    cost_unknown = upstream_account_usage_breakdown_hourly.cost_unknown + excluded.cost_unknown,
                    has_cost = upstream_account_usage_breakdown_hourly.has_cost + excluded.has_cost,
                    performance_total_tokens = upstream_account_usage_breakdown_hourly.performance_total_tokens + excluded.performance_total_tokens,
                    performance_stream_output_tokens = upstream_account_usage_breakdown_hourly.performance_stream_output_tokens + excluded.performance_stream_output_tokens,
                    performance_stream_duration_ms = upstream_account_usage_breakdown_hourly.performance_stream_duration_ms + excluded.performance_stream_duration_ms,
                    performance_response_sample_count = upstream_account_usage_breakdown_hourly.performance_response_sample_count + excluded.performance_response_sample_count,
                    performance_response_sum_ms = upstream_account_usage_breakdown_hourly.performance_response_sum_ms + excluded.performance_response_sum_ms,
                    performance_first_byte_sample_count = upstream_account_usage_breakdown_hourly.performance_first_byte_sample_count + excluded.performance_first_byte_sample_count,
                    performance_first_byte_sum_ms = upstream_account_usage_breakdown_hourly.performance_first_byte_sum_ms + excluded.performance_first_byte_sum_ms,
                    performance_usage_duration_sample_count = upstream_account_usage_breakdown_hourly.performance_usage_duration_sample_count + excluded.performance_usage_duration_sample_count,
                    performance_usage_duration_sum_ms = upstream_account_usage_breakdown_hourly.performance_usage_duration_sum_ms + excluded.performance_usage_duration_sum_ms,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&upstream_account_key)
            .bind(upstream_account_id)
            .bind(&normalized_model)
            .bind(&normalized_reasoning_effort)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.cache_write_tokens)
            .bind(delta.cache_read_tokens)
            .bind(delta.output_tokens)
            .bind(delta.cost_input)
            .bind(delta.cost_cache_write)
            .bind(delta.cost_cache_read)
            .bind(delta.cost_output)
            .bind(delta.cost_reasoning)
            .bind(delta.cost_unknown)
            .bind(delta.has_cost)
            .bind(delta.performance_total_tokens)
            .bind(delta.performance_stream_output_tokens)
            .bind(delta.performance_stream_duration_ms)
            .bind(delta.performance_response_sample_count)
            .bind(delta.performance_response_sum_ms)
            .bind(delta.performance_first_byte_sample_count)
            .bind(delta.performance_first_byte_sum_ms)
            .bind(delta.performance_usage_duration_sample_count)
            .bind(delta.performance_usage_duration_sum_ms)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_upstream_account_stats_hourly {
        #[derive(sqlx::FromRow)]
        struct AccountStatsHistogramRow {
            first_byte_histogram: String,
            first_response_byte_total_histogram: String,
        }

        for ((bucket_start_epoch, source, upstream_account_id), delta) in
            upstream_account_stats_hourly
        {
            let current_histograms = sqlx::query_as::<_, AccountStatsHistogramRow>(
                r#"
                SELECT
                    first_byte_histogram,
                    first_response_byte_total_histogram
                FROM upstream_account_stats_hourly
                WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = ?3
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
            .fetch_optional(&mut *tx)
            .await?;
            let mut merged_first_byte_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_byte_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_byte_histogram,
                &delta.first_byte_histogram,
            )?;
            let mut merged_first_response_byte_total_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_response_byte_total_histogram,
                &delta.first_response_byte_total_histogram,
            )?;
            sqlx::query(
                r#"
                INSERT INTO upstream_account_stats_hourly (
                    bucket_start_epoch,
                    source,
                    upstream_account_id,
                    total_count,
                    success_count,
                    failure_count,
                    in_flight_count,
                    total_tokens,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_cost,
                    non_success_cost,
                    total_latency_sample_count,
                    total_latency_sum_ms,
                    first_byte_sample_count,
                    first_byte_sum_ms,
                    first_byte_max_ms,
                    first_byte_histogram,
                    first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms,
                    first_response_byte_total_histogram,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, upstream_account_id) DO UPDATE SET
                    total_count = upstream_account_stats_hourly.total_count + excluded.total_count,
                    success_count = upstream_account_stats_hourly.success_count + excluded.success_count,
                    failure_count = upstream_account_stats_hourly.failure_count + excluded.failure_count,
                    in_flight_count = upstream_account_stats_hourly.in_flight_count + excluded.in_flight_count,
                    total_tokens = upstream_account_stats_hourly.total_tokens + excluded.total_tokens,
                    input_tokens = upstream_account_stats_hourly.input_tokens + excluded.input_tokens,
                    output_tokens = upstream_account_stats_hourly.output_tokens + excluded.output_tokens,
                    cache_input_tokens = upstream_account_stats_hourly.cache_input_tokens + excluded.cache_input_tokens,
                    total_cost = upstream_account_stats_hourly.total_cost + excluded.total_cost,
                    non_success_cost = upstream_account_stats_hourly.non_success_cost + excluded.non_success_cost,
                    total_latency_sample_count = upstream_account_stats_hourly.total_latency_sample_count + excluded.total_latency_sample_count,
                    total_latency_sum_ms = upstream_account_stats_hourly.total_latency_sum_ms + excluded.total_latency_sum_ms,
                    first_byte_sample_count = upstream_account_stats_hourly.first_byte_sample_count + excluded.first_byte_sample_count,
                    first_byte_sum_ms = upstream_account_stats_hourly.first_byte_sum_ms + excluded.first_byte_sum_ms,
                    first_byte_max_ms = MAX(upstream_account_stats_hourly.first_byte_max_ms, excluded.first_byte_max_ms),
                    first_byte_histogram = excluded.first_byte_histogram,
                    first_response_byte_total_sample_count = upstream_account_stats_hourly.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms = upstream_account_stats_hourly.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms = MAX(upstream_account_stats_hourly.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
                    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.in_flight_count)
            .bind(delta.total_tokens)
            .bind(delta.input_tokens)
            .bind(delta.output_tokens)
            .bind(delta.cache_input_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.total_latency_sample_count)
            .bind(delta.total_latency_sum_ms)
            .bind(delta.first_byte_sample_count)
            .bind(delta.first_byte_sum_ms)
            .bind(delta.first_byte_max_ms)
            .bind(encode_approx_histogram(&merged_first_byte_histogram)?)
            .bind(delta.first_response_byte_total_sample_count)
            .bind(delta.first_response_byte_total_sum_ms)
            .bind(delta.first_response_byte_total_max_ms)
            .bind(encode_approx_histogram(
                &merged_first_response_byte_total_histogram,
            )?)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_upstream_account_stats_minute {
        #[derive(sqlx::FromRow)]
        struct AccountMinuteStatsHistogramRow {
            first_byte_histogram: String,
            first_response_byte_total_histogram: String,
        }

        for ((bucket_start_epoch, source, upstream_account_id), delta) in
            upstream_account_stats_minute
        {
            let current_histograms = sqlx::query_as::<_, AccountMinuteStatsHistogramRow>(
                r#"
                SELECT
                    first_byte_histogram,
                    first_response_byte_total_histogram
                FROM upstream_account_stats_minute
                WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = ?3
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
            .fetch_optional(&mut *tx)
            .await?;
            let mut merged_first_byte_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_byte_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_byte_histogram,
                &delta.first_byte_histogram,
            )?;
            let mut merged_first_response_byte_total_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_response_byte_total_histogram,
                &delta.first_response_byte_total_histogram,
            )?;
            sqlx::query(
                r#"
                INSERT INTO upstream_account_stats_minute (
                    bucket_start_epoch,
                    source,
                    upstream_account_id,
                    total_count,
                    success_count,
                    failure_count,
                    in_flight_count,
                    total_tokens,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_cost,
                    non_success_cost,
                    total_latency_sample_count,
                    total_latency_sum_ms,
                    first_byte_sample_count,
                    first_byte_sum_ms,
                    first_byte_max_ms,
                    first_byte_histogram,
                    first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms,
                    first_response_byte_total_histogram,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, upstream_account_id) DO UPDATE SET
                    total_count = upstream_account_stats_minute.total_count + excluded.total_count,
                    success_count = upstream_account_stats_minute.success_count + excluded.success_count,
                    failure_count = upstream_account_stats_minute.failure_count + excluded.failure_count,
                    in_flight_count = upstream_account_stats_minute.in_flight_count + excluded.in_flight_count,
                    total_tokens = upstream_account_stats_minute.total_tokens + excluded.total_tokens,
                    input_tokens = upstream_account_stats_minute.input_tokens + excluded.input_tokens,
                    output_tokens = upstream_account_stats_minute.output_tokens + excluded.output_tokens,
                    cache_input_tokens = upstream_account_stats_minute.cache_input_tokens + excluded.cache_input_tokens,
                    total_cost = upstream_account_stats_minute.total_cost + excluded.total_cost,
                    non_success_cost = upstream_account_stats_minute.non_success_cost + excluded.non_success_cost,
                    total_latency_sample_count = upstream_account_stats_minute.total_latency_sample_count + excluded.total_latency_sample_count,
                    total_latency_sum_ms = upstream_account_stats_minute.total_latency_sum_ms + excluded.total_latency_sum_ms,
                    first_byte_sample_count = upstream_account_stats_minute.first_byte_sample_count + excluded.first_byte_sample_count,
                    first_byte_sum_ms = upstream_account_stats_minute.first_byte_sum_ms + excluded.first_byte_sum_ms,
                    first_byte_max_ms = MAX(upstream_account_stats_minute.first_byte_max_ms, excluded.first_byte_max_ms),
                    first_byte_histogram = excluded.first_byte_histogram,
                    first_response_byte_total_sample_count = upstream_account_stats_minute.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms = upstream_account_stats_minute.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms = MAX(upstream_account_stats_minute.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
                    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.in_flight_count)
            .bind(delta.total_tokens)
            .bind(delta.input_tokens)
            .bind(delta.output_tokens)
            .bind(delta.cache_input_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.total_latency_sample_count)
            .bind(delta.total_latency_sum_ms)
            .bind(delta.first_byte_sample_count)
            .bind(delta.first_byte_sum_ms)
            .bind(delta.first_byte_max_ms)
            .bind(encode_approx_histogram(&merged_first_byte_histogram)?)
            .bind(delta.first_response_byte_total_sample_count)
            .bind(delta.first_response_byte_total_sum_ms)
            .bind(delta.first_response_byte_total_max_ms)
            .bind(encode_approx_histogram(
                &merged_first_response_byte_total_histogram,
            )?)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_sticky_keys {
        for ((bucket_start_epoch, upstream_account_id, sticky_key), delta) in sticky_keys {
            sqlx::query(
                r#"
                INSERT INTO upstream_sticky_key_hourly (
                    bucket_start_epoch,
                    upstream_account_id,
                    sticky_key,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
                ON CONFLICT(bucket_start_epoch, upstream_account_id, sticky_key) DO UPDATE SET
                    request_count = upstream_sticky_key_hourly.request_count + excluded.request_count,
                    success_count = upstream_sticky_key_hourly.success_count + excluded.success_count,
                    failure_count = upstream_sticky_key_hourly.failure_count + excluded.failure_count,
                    total_tokens = upstream_sticky_key_hourly.total_tokens + excluded.total_tokens,
                    total_cost = upstream_sticky_key_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(upstream_sticky_key_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(upstream_sticky_key_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(upstream_account_id)
            .bind(&sticky_key)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    Ok(())
}

pub(crate) fn invocation_archive_target_needs_full_payload(target: &str) -> bool {
    matches!(
        target,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE
            | HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS
            | HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE
            | HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN
            | HOURLY_ROLLUP_TARGET_STICKY_KEYS
    )
}

pub(crate) async fn upsert_forward_proxy_attempt_hourly_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut deltas: BTreeMap<(String, i64), ForwardProxyAttemptHourlyDelta> = BTreeMap::new();
    for row in rows {
        let bucket_start_epoch = forward_proxy_attempt_bucket_start_epoch(&row.occurred_at)?;
        let entry = deltas
            .entry((row.proxy_key.clone(), bucket_start_epoch))
            .or_default();
        entry.attempts += 1;
        if row.is_success != 0 {
            entry.success_count += 1;
        } else {
            entry.failure_count += 1;
        }
        if let Some(latency_ms) = row.latency_ms
            && latency_ms.is_finite()
            && latency_ms >= 0.0
        {
            entry.latency_sample_count += 1;
            entry.latency_sum_ms += latency_ms;
            entry.latency_max_ms = entry.latency_max_ms.max(latency_ms);
        }
    }

    for ((proxy_key, bucket_start_epoch), delta) in deltas {
        sqlx::query(
            r#"
            INSERT INTO forward_proxy_attempt_hourly (
                proxy_key,
                bucket_start_epoch,
                attempts,
                success_count,
                failure_count,
                latency_sample_count,
                latency_sum_ms,
                latency_max_ms,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
            ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
                attempts = forward_proxy_attempt_hourly.attempts + excluded.attempts,
                success_count = forward_proxy_attempt_hourly.success_count + excluded.success_count,
                failure_count = forward_proxy_attempt_hourly.failure_count + excluded.failure_count,
                latency_sample_count = forward_proxy_attempt_hourly.latency_sample_count + excluded.latency_sample_count,
                latency_sum_ms = forward_proxy_attempt_hourly.latency_sum_ms + excluded.latency_sum_ms,
                latency_max_ms = MAX(forward_proxy_attempt_hourly.latency_max_ms, excluded.latency_max_ms),
                updated_at = datetime('now')
            "#,
        )
        .bind(&proxy_key)
        .bind(bucket_start_epoch)
        .bind(delta.attempts)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.latency_sample_count)
        .bind(delta.latency_sum_ms)
        .bind(delta.latency_max_ms)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn delete_hourly_rollup_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    table: &str,
    bucket_epochs: &[i64],
) -> Result<()> {
    if bucket_epochs.is_empty() {
        return Ok(());
    }
    let mut query =
        QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE bucket_start_epoch IN ("));
    {
        let mut separated = query.separated(", ");
        for bucket_epoch in bucket_epochs {
            separated.push_bind(bucket_epoch);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    Ok(())
}

pub(crate) async fn delete_rollup_rows_for_bucket_epochs_with_size_tx(
    tx: &mut SqliteConnection,
    table: &str,
    bucket_epochs: &[i64],
    bucket_seconds: i64,
) -> Result<()> {
    if bucket_epochs.is_empty() {
        return Ok(());
    }
    let normalized = if bucket_seconds == 3_600 {
        bucket_epochs.to_vec()
    } else {
        let mut values = Vec::new();
        for hour_epoch in bucket_epochs {
            let mut cursor = *hour_epoch;
            let hour_end = hour_epoch.saturating_add(3_600);
            while cursor < hour_end {
                values.push(cursor);
                cursor = cursor.saturating_add(bucket_seconds);
            }
        }
        values.sort_unstable();
        values.dedup();
        values
    };
    delete_hourly_rollup_rows_for_bucket_epochs_tx(tx, table, &normalized).await
}

pub(crate) async fn load_live_invocation_hourly_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    bucket_epochs: &[i64],
) -> Result<Vec<InvocationHourlySourceRecord>> {
    if bucket_epochs.is_empty() {
        return Ok(Vec::new());
    }

    let min_bucket_epoch = *bucket_epochs
        .iter()
        .min()
        .ok_or_else(|| anyhow!("missing minimum invocation bucket epoch"))?;
    let max_bucket_epoch = *bucket_epochs
        .iter()
        .max()
        .ok_or_else(|| anyhow!("missing maximum invocation bucket epoch"))?;
    let min_bucket_start = Utc
        .timestamp_opt(min_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum invocation bucket epoch"))?;
    let max_bucket_end = Utc
        .timestamp_opt(max_bucket_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum invocation bucket epoch"))?;
    let bucket_epoch_set = bucket_epochs.iter().copied().collect::<HashSet<_>>();
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx).await?,
    );

    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
        "SELECT \
            id,
            occurred_at,
            source,
            status,
            detail_level,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost,
            {} AS upstream_account_id,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
         FROM codex_invocations
         WHERE occurred_at >= ?1
           AND occurred_at < ?2
         ORDER BY id ASC",
        upstream_account_id_sql,
    ))
    .bind(db_occurred_at_lower_bound(min_bucket_start))
    .bind(db_occurred_at_lower_bound(max_bucket_end))
    .fetch_all(&mut *tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            invocation_bucket_start_epoch(&row.occurred_at)
                .map(|bucket_epoch| bucket_epoch_set.contains(&bucket_epoch))
                .unwrap_or(false)
        })
        .collect())
}

pub(crate) async fn recompute_invocation_hourly_rollups_for_ids_tx(
    tx: &mut SqliteConnection,
    ids: &[i64],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT occurred_at FROM codex_invocations WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
    }
    query.push(")");
    let occurred_rows = query
        .build_query_scalar::<String>()
        .fetch_all(&mut *tx)
        .await?;
    if occurred_rows.is_empty() {
        return Ok(());
    }

    let mut bucket_epochs = occurred_rows
        .iter()
        .map(|occurred_at| invocation_bucket_start_epoch(occurred_at))
        .collect::<Result<Vec<_>>>()?;
    bucket_epochs.sort_unstable();
    bucket_epochs.dedup();
    if bucket_epochs.is_empty() {
        return Ok(());
    }

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
        delete_hourly_rollup_rows_for_bucket_epochs_tx(tx, table, &bucket_epochs).await?;
    }
    delete_rollup_rows_for_bucket_epochs_with_size_tx(
        tx,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
        &bucket_epochs,
        60,
    )
    .await?;

    let rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx, &bucket_epochs).await?;
    upsert_invocation_hourly_rollups_tx(tx, &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS).await?;
    Ok(())
}

pub(crate) async fn replay_live_invocation_hourly_rollups(pool: &Pool<Sqlite>) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let rows = {
        let mut conn = pool.acquire().await?;
        let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
            "codex_invocations",
            load_pool_attempt_fallback_capability_tx(&mut conn).await?,
        );
        sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
            r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost,
            {} AS upstream_account_id,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
            upstream_account_id_sql,
        ))
        .bind(cursor_id)
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(&mut *conn)
        .await?
    };
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    let mut tx = pool.begin().await?;
    upsert_invocation_hourly_rollups_tx(tx.as_mut(), &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS)
        .await?;
    save_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id)
        .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_invocation_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx).await?,
    );
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost,
            {} AS upstream_account_id,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
        upstream_account_id_sql,
    ))
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_invocation_hourly_rollups_tx(tx, &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS).await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id).await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn repair_live_invocation_usage_breakdown_rollups(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    if load_hourly_rollup_live_progress(
        pool,
        INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
    )
    .await?
        >= INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE
    {
        return Ok(());
    }

    loop {
        let updated = repair_live_invocation_usage_breakdown_rollups_once(pool).await?;
        if updated == 0 {
            return Ok(());
        }
    }
}

async fn repair_live_invocation_usage_breakdown_rollups_once(pool: &Pool<Sqlite>) -> Result<u64> {
    let mut tx = pool.begin().await?;
    if load_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
    )
    .await?
        >= INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE
    {
        tx.rollback().await?;
        return Ok(0);
    }

    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_cursor = load_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET,
    )
    .await?;
    if repair_cursor >= shared_live_cursor {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
        tx.commit().await?;
        return Ok(0);
    }

    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx.as_mut()).await?,
    );
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost,
            {} AS upstream_account_id,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
          AND id <= ?2
        ORDER BY id ASC
        LIMIT ?3
        "#,
        upstream_account_id_sql,
    ))
    .bind(repair_cursor)
    .bind(shared_live_cursor)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;

    if rows.is_empty() {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET,
            shared_live_cursor,
        )
        .await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
        tx.commit().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(repair_cursor);
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &rows,
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET,
        last_id,
    )
    .await?;
    if last_id >= shared_live_cursor {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_forward_proxy_attempt_hourly_rollups(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    let mut tx = pool.begin().await?;
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_forward_proxy_attempt_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &rows).await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS, last_id)
        .await?;
    Ok(rows.len() as u64)
}

#[derive(Debug, Clone, FromRow)]
struct UpstreamHostNetworkMinuteSourceRow {
    id: i64,
    bucket_start_epoch: i64,
    source: String,
    upstream_base_url_host: String,
    upload_bytes: i64,
    download_bytes: i64,
}

fn upstream_host_network_direct_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxUploadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxUploadBytes') AS INTEGER) \
             END, \
             CASE WHEN {alias}.request_raw_size < 0 THEN 0 ELSE COALESCE({alias}.request_raw_size, 0) END, \
             0 \
           ) \
         END"
    )
}

fn upstream_host_network_direct_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             CASE \
               WHEN COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) < 0 THEN 0 \
               ELSE COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) \
             END, \
             0 \
           ) < 0 THEN 0 \
           ELSE COALESCE( \
             CASE \
               WHEN json_valid({alias}.payload) \
                 AND json_type({alias}.payload, '$.upstreamApproxDownloadBytes') IN ('integer', 'real') \
               THEN CAST(json_extract({alias}.payload, '$.upstreamApproxDownloadBytes') AS INTEGER) \
             END, \
             CASE \
               WHEN COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) < 0 THEN 0 \
               ELSE COALESCE( \
                 CASE \
                   WHEN json_valid({alias}.payload) \
                     AND json_type({alias}.payload, '$.forwardedBytes') IN ('integer', 'real') \
                   THEN CAST(json_extract({alias}.payload, '$.forwardedBytes') AS INTEGER) \
                 END, \
                 {alias}.response_raw_size, \
                 CAST(LENGTH({alias}.raw_response) AS INTEGER), \
                 0 \
               ) \
             END, \
             0 \
           ) \
         END"
    )
}

fn upstream_host_network_pool_attempt_upload_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_request_header_bytes_approx, 0) + COALESCE({alias}.upstream_request_transmitted_body_bytes, 0) \
         END"
    )
}

fn upstream_host_network_pool_attempt_download_bytes_sql(alias: &str) -> String {
    format!(
        "CASE \
           WHEN COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) < 0 THEN 0 \
           ELSE COALESCE({alias}.upstream_response_header_bytes_approx, 0) + COALESCE({alias}.upstream_response_body_bytes, 0) \
         END"
    )
}

fn upstream_host_network_payload_host_sql(alias: &str) -> String {
    format!(
        "COALESCE( \
            NULLIF(LOWER(TRIM(CAST(CASE \
                WHEN json_valid({alias}.payload) AND json_type({alias}.payload, '$.upstreamBaseUrlHost') = 'text' \
                    THEN json_extract({alias}.payload, '$.upstreamBaseUrlHost') \
                WHEN json_valid({alias}.payload) AND json_type({alias}.payload, '$.upstream_base_url_host') = 'text' \
                    THEN json_extract({alias}.payload, '$.upstream_base_url_host') \
            END AS TEXT))), ''), \
            {unknown_host} \
        )",
        unknown_host =
            sql_quote(crate::dashboard_network_speed::DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST),
    )
}

fn upstream_host_network_attempt_host_sql(alias: &str) -> String {
    format!(
        "COALESCE(NULLIF(LOWER(TRIM({alias}.upstream_base_url_host)), ''), {unknown_host})",
        unknown_host =
            sql_quote(crate::dashboard_network_speed::DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST),
    )
}

fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

async fn load_or_seed_hourly_rollup_live_progress_to_table_tail_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    max_id_sql: &str,
) -> Result<(i64, bool)> {
    if crate::stats::hourly_rollup_progress_exists(&mut *tx, dataset).await? {
        return Ok((
            load_hourly_rollup_live_progress_tx(tx, dataset).await?,
            false,
        ));
    }
    let cursor_id = sqlx::query_scalar::<_, Option<i64>>(max_id_sql)
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or(0)
        .max(0);
    save_hourly_rollup_live_progress_tx(tx, dataset, cursor_id).await?;
    Ok((cursor_id, true))
}

async fn upsert_upstream_host_network_minute_rows_tx(
    tx: &mut SqliteConnection,
    rows: &[UpstreamHostNetworkMinuteSourceRow],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut aggregates = BTreeMap::<
        (i64, String, String),
        crate::dashboard_network_speed::DashboardNetworkByteTotals,
    >::new();
    for row in rows {
        let entry = aggregates
            .entry((
                row.bucket_start_epoch,
                row.source.clone(),
                row.upstream_base_url_host.clone(),
            ))
            .or_default();
        entry.upload_bytes = entry.upload_bytes.saturating_add(row.upload_bytes.max(0));
        entry.download_bytes = entry
            .download_bytes
            .saturating_add(row.download_bytes.max(0));
    }

    for ((bucket_start_epoch, source, upstream_base_url_host), totals) in aggregates {
        sqlx::query(
            r#"
            INSERT INTO upstream_host_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upload_bytes,
                download_bytes,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
            ON CONFLICT(bucket_start_epoch, source, upstream_base_url_host) DO UPDATE SET
                upload_bytes = upstream_host_network_minute.upload_bytes + excluded.upload_bytes,
                download_bytes = upstream_host_network_minute.download_bytes + excluded.download_bytes,
                updated_at = datetime('now')
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .bind(upstream_base_url_host)
        .bind(totals.upload_bytes.max(0))
        .bind(totals.download_bytes.max(0))
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn replay_live_upstream_host_network_minute_rollups_from_invocations(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let (cursor_id, seeded) = load_or_seed_hourly_rollup_live_progress_to_table_tail_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT,
        "SELECT MAX(id) FROM codex_invocations",
    )
    .await?;
    if seeded {
        tx.commit().await?;
        return Ok(0);
    }

    let upload_bytes_sql = upstream_host_network_direct_upload_bytes_sql("inv");
    let download_bytes_sql = upstream_host_network_direct_download_bytes_sql("inv");
    let host_sql = upstream_host_network_payload_host_sql("inv");
    let rows = sqlx::query_as::<_, UpstreamHostNetworkMinuteSourceRow>(
        format!(
            r#"
            SELECT
                inv.id,
                ((unixepoch(inv.occurred_at || '+08:00') / 60) * 60) AS bucket_start_epoch,
                inv.source,
                {host_sql} AS upstream_base_url_host,
                {upload_bytes_sql} AS upload_bytes,
                {download_bytes_sql} AS download_bytes
            FROM codex_invocations AS inv
            WHERE inv.id > ?1
              AND (
                    COALESCE(
                        CASE
                            WHEN json_valid(inv.payload)
                                THEN TRIM(CAST(json_extract(inv.payload, '$.routeMode') AS TEXT))
                        END,
                        ''
                    ) <> ?2
                    OR NOT EXISTS (
                        SELECT 1
                        FROM pool_upstream_request_attempts AS attempts
                        WHERE attempts.invoke_id = inv.invoke_id
                          AND attempts.occurred_at = inv.occurred_at
                    )
              )
            ORDER BY inv.id ASC
            LIMIT ?3
            "#,
        )
        .as_str(),
    )
    .bind(cursor_id)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;
    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_upstream_host_network_minute_rows_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_upstream_host_network_minute_rollups_from_pool_attempts(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let (cursor_id, seeded) = load_or_seed_hourly_rollup_live_progress_to_table_tail_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_POOL_ATTEMPTS,
        "SELECT MAX(id) FROM pool_upstream_request_attempts",
    )
    .await?;
    if seeded {
        tx.commit().await?;
        return Ok(0);
    }

    let upload_bytes_sql = upstream_host_network_pool_attempt_upload_bytes_sql("attempts");
    let download_bytes_sql = upstream_host_network_pool_attempt_download_bytes_sql("attempts");
    let host_sql = upstream_host_network_attempt_host_sql("attempts");
    let rows = sqlx::query_as::<_, UpstreamHostNetworkMinuteSourceRow>(
        format!(
            r#"
            SELECT
                attempts.id,
                ((unixepoch(attempts.occurred_at || '+08:00') / 60) * 60) AS bucket_start_epoch,
                inv.source,
                {host_sql} AS upstream_base_url_host,
                {upload_bytes_sql} AS upload_bytes,
                {download_bytes_sql} AS download_bytes
            FROM pool_upstream_request_attempts AS attempts
            INNER JOIN codex_invocations AS inv
                ON inv.invoke_id = attempts.invoke_id
               AND inv.occurred_at = attempts.occurred_at
            WHERE attempts.id > ?1
            ORDER BY attempts.id ASC
            LIMIT ?2
            "#,
        )
        .as_str(),
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;
    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_upstream_host_network_minute_rows_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_POOL_ATTEMPTS,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn backfill_invocation_rollup_hourly_from_sources(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    let mut overall: BTreeMap<(i64, String), InvocationHourlyRollupDelta> = BTreeMap::new();
    let mut seen_ids = HashSet::new();

    for archive_file in archive_files {
        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during invocation hourly rollup backfill"
            );
            continue;
        }

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        let archive_columns =
            load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        let archive_query_sql = build_legacy_compatible_invocation_archive_query(&archive_columns);
        let mut archive_cursor_id = 0_i64;
        loop {
            let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&archive_query_sql)
                .bind(archive_cursor_id)
                .bind(BACKFILL_BATCH_SIZE)
                .fetch_all(&archive_pool)
                .await?;
            if rows.is_empty() {
                break;
            }
            archive_cursor_id = rows.last().map(|row| row.id).unwrap_or(archive_cursor_id);
            rows.retain(|row| seen_ids.insert(row.id));
            if rows.is_empty() {
                continue;
            }
            accumulate_invocation_hourly_overall_rollups(&mut overall, &rows)?;
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    let mut cursor_id = 0_i64;
    loop {
        let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms
            FROM codex_invocations
            WHERE id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(cursor_id)
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(pool)
        .await?;
        if rows.is_empty() {
            break;
        }
        cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
        rows.retain(|row| seen_ids.insert(row.id));
        if rows.is_empty() {
            continue;
        }
        accumulate_invocation_hourly_overall_rollups(&mut overall, &rows)?;
    }

    if overall.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await?;
    for ((bucket_start_epoch, source), delta) in &overall {
        sqlx::query(
            r#"
            INSERT INTO invocation_rollup_hourly (
                bucket_start_epoch,
                source,
                total_count,
                success_count,
                failure_count,
                total_tokens,
                cache_input_tokens,
                total_cost,
                non_success_cost,
                total_latency_sample_count,
                total_latency_sum_ms,
                first_byte_sample_count,
                first_byte_sum_ms,
                first_byte_max_ms,
                first_byte_histogram,
                first_response_byte_total_sample_count,
                first_response_byte_total_sum_ms,
                first_response_byte_total_max_ms,
                first_response_byte_total_histogram,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, datetime('now'))
            ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET
                total_count = excluded.total_count,
                success_count = excluded.success_count,
                failure_count = excluded.failure_count,
                total_tokens = excluded.total_tokens,
                cache_input_tokens = excluded.cache_input_tokens,
                total_cost = excluded.total_cost,
                non_success_cost = excluded.non_success_cost,
                total_latency_sample_count = excluded.total_latency_sample_count,
                total_latency_sum_ms = excluded.total_latency_sum_ms,
                first_byte_sample_count = excluded.first_byte_sample_count,
                first_byte_sum_ms = excluded.first_byte_sum_ms,
                first_byte_max_ms = excluded.first_byte_max_ms,
                first_byte_histogram = excluded.first_byte_histogram,
                first_response_byte_total_sample_count = excluded.first_response_byte_total_sample_count,
                first_response_byte_total_sum_ms = excluded.first_response_byte_total_sum_ms,
                first_response_byte_total_max_ms = excluded.first_response_byte_total_max_ms,
                first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                updated_at = datetime('now')
            "#,
        )
        .bind(*bucket_start_epoch)
        .bind(source)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.total_tokens)
        .bind(delta.cache_input_tokens)
        .bind(delta.total_cost)
        .bind(delta.non_success_cost)
        .bind(delta.total_latency_sample_count)
        .bind(delta.total_latency_sum_ms)
        .bind(delta.first_byte_sample_count)
        .bind(delta.first_byte_sum_ms)
        .bind(delta.first_byte_max_ms)
        .bind(encode_approx_histogram(&delta.first_byte_histogram)?)
        .bind(delta.first_response_byte_total_sample_count)
        .bind(delta.first_response_byte_total_sum_ms)
        .bind(delta.first_response_byte_total_max_ms)
        .bind(encode_approx_histogram(
            &delta.first_response_byte_total_histogram,
        )?)
        .execute(tx.as_mut())
        .await?;
    }
    tx.commit().await?;

    Ok(overall.len())
}

pub(crate) async fn rebuild_upstream_account_stats_rollups_from_sources(
    pool: &Pool<Sqlite>,
) -> Result<(usize, usize)> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    let mut seen_ids = HashSet::new();
    let mut source_rows = Vec::<InvocationHourlySourceRecord>::new();

    for archive_file in archive_files {
        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during upstream account stats rollup rebuild"
            );
            continue;
        }

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        let archive_columns =
            load_archive_table_columns(&archive_pool, "codex_invocations").await?;
        let archive_query_sql = build_legacy_compatible_invocation_archive_query(&archive_columns);
        let mut archive_cursor_id = 0_i64;
        loop {
            let mut archive_rows =
                sqlx::query_as::<_, InvocationHourlySourceRecord>(&archive_query_sql)
                    .bind(archive_cursor_id)
                    .bind(BACKFILL_BATCH_SIZE)
                    .fetch_all(&archive_pool)
                    .await?;
            if archive_rows.is_empty() {
                break;
            }
            archive_cursor_id = archive_rows
                .last()
                .map(|row| row.id)
                .unwrap_or(archive_cursor_id);
            archive_rows.retain(|row| seen_ids.insert(row.id));
            source_rows.extend(archive_rows);
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    let mut cursor_id = 0_i64;
    let mut live_conn = pool.acquire().await?;
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(&mut live_conn).await?,
    );
    loop {
        let mut live_rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                {} AS upstream_account_id,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms
            FROM codex_invocations
            WHERE id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
            upstream_account_id_sql,
        ))
        .bind(cursor_id)
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(&mut *live_conn)
        .await?;
        if live_rows.is_empty() {
            break;
        }
        cursor_id = live_rows.last().map(|row| row.id).unwrap_or(cursor_id);
        live_rows.retain(|row| seen_ids.insert(row.id));
        source_rows.extend(live_rows);
    }
    drop(live_conn);

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM upstream_account_stats_hourly")
        .execute(tx.as_mut())
        .await?;
    sqlx::query("DELETE FROM upstream_account_stats_minute")
        .execute(tx.as_mut())
        .await?;
    if !source_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &source_rows,
            &[
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
            ],
        )
        .await?;
    }
    if cursor_id > 0 {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            cursor_id,
        )
        .await?;
    }
    tx.commit().await?;

    let hourly_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_hourly")
            .fetch_one(pool)
            .await?;
    let minute_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM upstream_account_stats_minute")
            .fetch_one(pool)
            .await?;
    Ok((hourly_count.max(0) as usize, minute_count.max(0) as usize))
}

#[cfg(test)]
mod upstream_host_network_minute_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                status TEXT,
                detail_level TEXT,
                model TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_input_tokens INTEGER,
                total_tokens INTEGER,
                cost REAL,
                cost_input REAL,
                cost_cache_write REAL,
                cost_cache_read REAL,
                cost_output REAL,
                cost_reasoning REAL,
                error_message TEXT,
                failure_kind TEXT,
                failure_class TEXT,
                is_actionable INTEGER,
                payload TEXT,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                t_upstream_stream_ms REAL,
                t_resp_parse_ms REAL,
                t_persist_ms REAL,
                response_raw_size INTEGER,
                raw_response TEXT NOT NULL DEFAULT '',
                request_raw_size INTEGER,
                source TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create codex_invocations table");
        sqlx::query(
            r#"
            CREATE TABLE pool_upstream_request_attempts (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                upstream_base_url_host TEXT,
                upstream_account_id INTEGER,
                attempt_index INTEGER,
                upstream_request_header_bytes_approx INTEGER,
                upstream_request_transmitted_body_bytes INTEGER,
                upstream_response_header_bytes_approx INTEGER,
                upstream_response_body_bytes INTEGER
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create pool_upstream_request_attempts table");
        sqlx::query(
            r#"
            CREATE TABLE upstream_host_network_minute (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_base_url_host TEXT NOT NULL,
                upload_bytes INTEGER NOT NULL DEFAULT 0,
                download_bytes INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (bucket_start_epoch, source, upstream_base_url_host)
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_host_network_minute table");
        sqlx::query(
            r#"
            CREATE TABLE upstream_account_usage_breakdown_hourly (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_account_key TEXT NOT NULL,
                upstream_account_id INTEGER,
                normalized_model TEXT NOT NULL,
                normalized_reasoning_effort TEXT NOT NULL DEFAULT '',
                request_count INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cost_input REAL NOT NULL DEFAULT 0,
                cost_cache_write REAL NOT NULL DEFAULT 0,
                cost_cache_read REAL NOT NULL DEFAULT 0,
                cost_output REAL NOT NULL DEFAULT 0,
                cost_reasoning REAL NOT NULL DEFAULT 0,
                cost_unknown REAL NOT NULL DEFAULT 0,
                has_cost INTEGER NOT NULL DEFAULT 0,
                performance_total_tokens INTEGER NOT NULL DEFAULT 0,
                performance_stream_output_tokens INTEGER NOT NULL DEFAULT 0,
                performance_stream_duration_ms REAL NOT NULL DEFAULT 0,
                performance_response_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_response_sum_ms REAL NOT NULL DEFAULT 0,
                performance_first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_first_byte_sum_ms REAL NOT NULL DEFAULT 0,
                performance_usage_duration_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_usage_duration_sum_ms REAL NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (
                    bucket_start_epoch,
                    source,
                    upstream_account_key,
                    normalized_model,
                    normalized_reasoning_effort
                )
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_account_usage_breakdown_hourly table");
        sqlx::query(
            r#"
            CREATE TABLE hourly_rollup_live_progress (
                dataset TEXT PRIMARY KEY,
                cursor_id INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create hourly_rollup_live_progress table");
        pool
    }

    async fn save_progress(pool: &Pool<Sqlite>, dataset: &str, cursor_id: i64) {
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_live_progress (dataset, cursor_id)
            VALUES (?1, ?2)
            ON CONFLICT(dataset) DO UPDATE SET cursor_id = excluded.cursor_id
            "#,
        )
        .bind(dataset)
        .bind(cursor_id)
        .execute(pool)
        .await
        .expect("save live progress");
    }

    #[tokio::test]
    async fn upstream_host_network_direct_rollup_seeds_cursor_without_backfill() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                payload,
                response_raw_size,
                raw_response,
                request_raw_size,
                source,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(7_i64)
        .bind("invoke-seed")
        .bind("2026-07-18 15:00:00")
        .bind(r#"{"upstreamBaseUrlHost":"api.openai.com","upstreamApproxUploadBytes":10,"upstreamApproxDownloadBytes":20}"#)
        .bind(0_i64)
        .bind("")
        .bind(0_i64)
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:00:00Z")
        .execute(&pool)
        .await
        .expect("insert seed invocation");

        let updated = replay_live_upstream_host_network_minute_rollups_from_invocations(&pool)
            .await
            .expect("seed direct rollup cursor");
        assert_eq!(updated, 0);

        let row_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM upstream_host_network_minute")
                .fetch_one(&pool)
                .await
                .expect("count upstream host minute rows");
        assert_eq!(row_count, 0);

        let cursor_id: i64 = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
        )
        .bind(HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT)
        .fetch_one(&pool)
        .await
        .expect("load seeded cursor");
        assert_eq!(cursor_id, 7);
    }

    #[tokio::test]
    async fn upstream_host_network_direct_rollup_persists_normalized_host_bytes() {
        let pool = test_pool().await;
        save_progress(&pool, HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_DIRECT, 0).await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                payload,
                response_raw_size,
                raw_response,
                request_raw_size,
                source,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(1_i64)
        .bind("invoke-direct-host")
        .bind("2026-07-18 15:01:00")
        .bind(r#"{"upstreamBaseUrlHost":"API.OpenAI.com","upstreamApproxUploadBytes":120,"upstreamApproxDownloadBytes":240}"#)
        .bind(0_i64)
        .bind("")
        .bind(0_i64)
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:01:00Z")
        .execute(&pool)
        .await
        .expect("insert direct invocation");

        let updated = replay_live_upstream_host_network_minute_rollups_from_invocations(&pool)
            .await
            .expect("replay direct host minute rollups");
        assert_eq!(updated, 1);

        let row = sqlx::query_as::<_, (i64, String, String, i64, i64)>(
            r#"
            SELECT
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upload_bytes,
                download_bytes
            FROM upstream_host_network_minute
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load direct host minute row");
        assert_eq!(row.2, "api.openai.com");
        assert_eq!(row.3, 120);
        assert_eq!(row.4, 240);
    }

    #[tokio::test]
    async fn upstream_host_network_pool_rollup_splits_retry_hosts() {
        let pool = test_pool().await;
        save_progress(
            &pool,
            HOURLY_ROLLUP_DATASET_UPSTREAM_HOST_NETWORK_POOL_ATTEMPTS,
            0,
        )
        .await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                payload,
                response_raw_size,
                raw_response,
                request_raw_size,
                source,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(1_i64)
        .bind("invoke-pool-hosts")
        .bind("2026-07-18 15:02:00")
        .bind(r#"{"routeMode":"pool"}"#)
        .bind(0_i64)
        .bind("")
        .bind(0_i64)
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:02:00Z")
        .execute(&pool)
        .await
        .expect("insert pool invocation");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                id,
                invoke_id,
                occurred_at,
                upstream_base_url_host,
                upstream_request_header_bytes_approx,
                upstream_request_transmitted_body_bytes,
                upstream_response_header_bytes_approx,
                upstream_response_body_bytes
            )
            VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8),
                (?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            "#,
        )
        .bind(1_i64)
        .bind("invoke-pool-hosts")
        .bind("2026-07-18 15:02:00")
        .bind("primary.example.com")
        .bind(20_i64)
        .bind(80_i64)
        .bind(10_i64)
        .bind(30_i64)
        .bind(2_i64)
        .bind("invoke-pool-hosts")
        .bind("2026-07-18 15:02:00")
        .bind("backup.example.com")
        .bind(15_i64)
        .bind(45_i64)
        .bind(5_i64)
        .bind(25_i64)
        .execute(&pool)
        .await
        .expect("insert pool attempts");

        let updated = replay_live_upstream_host_network_minute_rollups_from_pool_attempts(&pool)
            .await
            .expect("replay pool host minute rollups");
        assert_eq!(updated, 2);

        let rows = sqlx::query_as::<_, (String, i64, i64)>(
            r#"
            SELECT upstream_base_url_host, upload_bytes, download_bytes
            FROM upstream_host_network_minute
            ORDER BY upstream_base_url_host ASC
            "#,
        )
        .fetch_all(&pool)
        .await
        .expect("load pool host minute rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("backup.example.com".to_string(), 60, 30));
        assert_eq!(rows[1], ("primary.example.com".to_string(), 100, 40));
    }

    #[tokio::test]
    async fn live_invocation_hourly_rows_preserve_attempt_fallback_account_id() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                payload,
                raw_response,
                source,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
            "#,
        )
        .bind(11_i64)
        .bind("invoke-fallback-account")
        .bind("2026-07-18 15:03:00")
        .bind("success")
        .bind(DETAIL_LEVEL_FULL)
        .bind("gpt-5")
        .bind(40_i64)
        .bind(60_i64)
        .bind(10_i64)
        .bind(100_i64)
        .bind(0.25_f64)
        .bind(r#"{"routeMode":"pool","responseModel":"gpt-5"}"#)
        .bind("")
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:03:00Z")
        .execute(&pool)
        .await
        .expect("insert invocation without payload account");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                id,
                invoke_id,
                occurred_at,
                attempt_index,
                upstream_account_id
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(17_i64)
        .bind("invoke-fallback-account")
        .bind("2026-07-18 15:03:00")
        .bind(0_i64)
        .bind(42_i64)
        .execute(&pool)
        .await
        .expect("insert fallback attempt account");

        let bucket_start_epoch =
            invocation_bucket_start_epoch("2026-07-18 15:03:00").expect("bucket start");
        let mut tx = pool.begin().await.expect("begin tx");
        let rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(
            tx.as_mut(),
            &[bucket_start_epoch],
        )
        .await
        .expect("load live hourly rows");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].upstream_account_id, Some(42));
        assert_eq!(rows[0].resolved_upstream_account_id(), Some(42));
    }

    #[tokio::test]
    async fn usage_breakdown_live_repair_backfills_rows_behind_shared_cursor() {
        let pool = test_pool().await;
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                invoke_id,
                occurred_at,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                payload,
                raw_response,
                source,
                created_at
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                ?18, ?19, ?20
            )
            "#,
        )
        .bind(41_i64)
        .bind("invoke-upgrade-breakdown-repair")
        .bind("2026-07-18 15:04:00")
        .bind("success")
        .bind(DETAIL_LEVEL_FULL)
        .bind("gpt-5")
        .bind(40_i64)
        .bind(60_i64)
        .bind(10_i64)
        .bind(100_i64)
        .bind(0.25_f64)
        .bind(0.11_f64)
        .bind(0.02_f64)
        .bind(0.03_f64)
        .bind(0.09_f64)
        .bind(0.0_f64)
        .bind(r#"{"routeMode":"pool","responseModel":"gpt-5"}"#)
        .bind("")
        .bind(SOURCE_PROXY)
        .bind("2026-07-18T07:04:00Z")
        .execute(&pool)
        .await
        .expect("insert retained invocation behind shared cursor");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                id,
                invoke_id,
                occurred_at,
                attempt_index,
                upstream_account_id
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(21_i64)
        .bind("invoke-upgrade-breakdown-repair")
        .bind("2026-07-18 15:04:00")
        .bind(0_i64)
        .bind(42_i64)
        .execute(&pool)
        .await
        .expect("insert fallback attempt for retained invocation");
        save_progress(&pool, HOURLY_ROLLUP_DATASET_INVOCATIONS, 41).await;

        repair_live_invocation_usage_breakdown_rollups(&pool)
            .await
            .expect("repair missing usage breakdown live rows");

        let repaired = sqlx::query_as::<_, (Option<i64>, String, i64, f64)>(
            r#"
            SELECT upstream_account_id, normalized_model, request_count, cost_input
            FROM upstream_account_usage_breakdown_hourly
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load repaired usage breakdown rollup row");
        assert_eq!(repaired.0, Some(42));
        assert_eq!(repaired.1, "gpt-5");
        assert_eq!(repaired.2, 1);
        assert_eq!(repaired.3, 0.11_f64);

        let shared_cursor: i64 = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
        )
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .fetch_one(&pool)
        .await
        .expect("load shared invocation cursor");
        assert_eq!(shared_cursor, 41);

        let repair_cursor: i64 = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
        )
        .bind(INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET)
        .fetch_one(&pool)
        .await
        .expect("load breakdown repair cursor");
        assert_eq!(repair_cursor, 41);

        let repair_marker: i64 = sqlx::query_scalar(
            "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
        )
        .bind(INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET)
        .fetch_one(&pool)
        .await
        .expect("load breakdown repair marker");
        assert_eq!(
            repair_marker,
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE
        );
    }
}

#[cfg(test)]
mod retention_breakdown_materialization_tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                occurred_at TEXT NOT NULL,
                source TEXT NOT NULL,
                status TEXT,
                detail_level TEXT NOT NULL DEFAULT 'full',
                model TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                cache_input_tokens INTEGER,
                total_tokens INTEGER,
                cost REAL,
                cost_input REAL,
                cost_cache_write REAL,
                cost_cache_read REAL,
                cost_output REAL,
                cost_reasoning REAL,
                error_message TEXT,
                failure_kind TEXT,
                failure_class TEXT,
                is_actionable INTEGER,
                payload TEXT,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                t_upstream_stream_ms REAL,
                t_resp_parse_ms REAL,
                t_persist_ms REAL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create codex_invocations table");
        sqlx::query(
            r#"
            CREATE TABLE hourly_rollup_materialized_buckets (
                target TEXT NOT NULL,
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                materialized_at TEXT NOT NULL,
                PRIMARY KEY (target, bucket_start_epoch, source)
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create hourly_rollup_materialized_buckets table");
        sqlx::query(
            r#"
            CREATE TABLE upstream_account_usage_breakdown_hourly (
                bucket_start_epoch INTEGER NOT NULL,
                source TEXT NOT NULL,
                upstream_account_key TEXT NOT NULL,
                upstream_account_id INTEGER,
                normalized_model TEXT NOT NULL,
                normalized_reasoning_effort TEXT NOT NULL DEFAULT '',
                request_count INTEGER NOT NULL DEFAULT 0,
                success_count INTEGER NOT NULL DEFAULT 0,
                failure_count INTEGER NOT NULL DEFAULT 0,
                cache_write_tokens INTEGER NOT NULL DEFAULT 0,
                cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                output_tokens INTEGER NOT NULL DEFAULT 0,
                cost_input REAL NOT NULL DEFAULT 0,
                cost_cache_write REAL NOT NULL DEFAULT 0,
                cost_cache_read REAL NOT NULL DEFAULT 0,
                cost_output REAL NOT NULL DEFAULT 0,
                cost_reasoning REAL NOT NULL DEFAULT 0,
                cost_unknown REAL NOT NULL DEFAULT 0,
                has_cost INTEGER NOT NULL DEFAULT 0,
                performance_total_tokens INTEGER NOT NULL DEFAULT 0,
                performance_stream_output_tokens INTEGER NOT NULL DEFAULT 0,
                performance_stream_duration_ms REAL NOT NULL DEFAULT 0,
                performance_response_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_response_sum_ms REAL NOT NULL DEFAULT 0,
                performance_first_byte_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_first_byte_sum_ms REAL NOT NULL DEFAULT 0,
                performance_usage_duration_sample_count INTEGER NOT NULL DEFAULT 0,
                performance_usage_duration_sum_ms REAL NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (
                    bucket_start_epoch,
                    source,
                    upstream_account_key,
                    normalized_model,
                    normalized_reasoning_effort
                )
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create upstream_account_usage_breakdown_hourly table");
        sqlx::query(
            r#"
            CREATE TABLE archive_batches (
                id INTEGER PRIMARY KEY,
                dataset TEXT NOT NULL,
                month_key TEXT,
                file_path TEXT NOT NULL UNIQUE,
                status TEXT NOT NULL,
                coverage_start_at TEXT,
                coverage_end_at TEXT,
                historical_rollups_materialized_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create archive_batches table");
        sqlx::query(
            r#"
            CREATE TABLE hourly_rollup_archive_replay (
                target TEXT NOT NULL,
                dataset TEXT NOT NULL,
                file_path TEXT NOT NULL,
                replayed_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (target, dataset, file_path)
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create hourly_rollup_archive_replay table");
        sqlx::query(
            r#"
            CREATE TABLE hourly_rollup_archive_progress (
                dataset TEXT NOT NULL,
                file_path TEXT NOT NULL,
                cursor_id INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (dataset, file_path)
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create hourly_rollup_archive_progress table");
        pool
    }

    #[tokio::test]
    async fn retention_archived_bucket_clears_breakdown_rollup_without_marking_it_materialized() {
        let pool = test_pool().await;
        let occurred_at = format_naive(
            Utc.with_ymd_and_hms(2026, 7, 1, 12, 34, 56)
                .single()
                .expect("valid timestamp")
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let row = InvocationHourlySourceRecord {
            id: 1,
            occurred_at: occurred_at.clone(),
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: Some("gpt-5".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_input_tokens: Some(0),
            total_tokens: Some(30),
            cost: Some(0.1),
            upstream_account_id: None,
            cost_input: Some(0.02),
            cost_cache_write: Some(0.0),
            cost_cache_read: Some(0.0),
            cost_output: Some(0.08),
            cost_reasoning: Some(0.0),
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: None,
            payload: Some(
                json!({
                    "upstreamAccountId": 42_i64,
                    "responseModel": "gpt-5"
                })
                .to_string(),
            ),
            t_total_ms: Some(100.0),
            t_req_read_ms: Some(0.0),
            t_req_parse_ms: Some(0.0),
            t_upstream_connect_ms: Some(0.0),
            t_upstream_ttfb_ms: Some(10.0),
            t_upstream_stream_ms: Some(20.0),
            t_resp_parse_ms: Some(0.0),
            t_persist_ms: Some(0.0),
        };
        let bucket_start_epoch =
            invocation_bucket_start_epoch(&row.occurred_at).expect("derive bucket start epoch");
        sqlx::query(
            r#"
            INSERT INTO upstream_account_usage_breakdown_hourly (
                bucket_start_epoch,
                source,
                upstream_account_key,
                upstream_account_id,
                normalized_model,
                normalized_reasoning_effort,
                request_count,
                success_count,
                failure_count
            )
            VALUES (?1, ?2, ?3, ?4, ?5, '', 1, 1, 0)
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(SOURCE_PROXY)
        .bind("upstream:42")
        .bind(42_i64)
        .bind("gpt-5")
        .execute(&pool)
        .await
        .expect("seed breakdown rollup row");

        let mut tx = pool.begin().await.expect("begin transaction");
        mark_retention_archived_hourly_rollup_targets_tx(
            tx.as_mut(),
            "codex_invocations",
            &[row],
            &[],
        )
        .await
        .expect("mark retention archived hourly rollup targets");
        tx.commit().await.expect("commit transaction");

        let breakdown_row_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM upstream_account_usage_breakdown_hourly WHERE bucket_start_epoch = ?1",
        )
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("count retained breakdown rollup rows");
        assert_eq!(breakdown_row_count, 0);

        let breakdown_materialized_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hourly_rollup_materialized_buckets WHERE target = ?1 AND bucket_start_epoch = ?2",
        )
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("count breakdown materialized markers");
        assert_eq!(breakdown_materialized_count, 0);

        let usage_materialized_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hourly_rollup_materialized_buckets WHERE target = ?1 AND bucket_start_epoch = ?2",
        )
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("count usage materialized markers");
        assert_eq!(usage_materialized_count, 1);
    }

    #[tokio::test]
    async fn retention_partial_hour_removes_archived_breakdown_without_dropping_retained_live_rows()
    {
        let pool = test_pool().await;
        let archived_occurred_at = format_naive(
            Utc.with_ymd_and_hms(2026, 7, 1, 12, 5, 0)
                .single()
                .expect("valid timestamp")
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let live_occurred_at = format_naive(
            Utc.with_ymd_and_hms(2026, 7, 1, 12, 45, 0)
                .single()
                .expect("valid timestamp")
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let archived_row = InvocationHourlySourceRecord {
            id: 1,
            occurred_at: archived_occurred_at,
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: Some("gpt-5".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_input_tokens: Some(0),
            total_tokens: Some(30),
            cost: Some(0.1),
            upstream_account_id: None,
            cost_input: Some(0.02),
            cost_cache_write: Some(0.0),
            cost_cache_read: Some(0.0),
            cost_output: Some(0.08),
            cost_reasoning: Some(0.0),
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: None,
            payload: Some(
                json!({
                    "upstreamAccountId": 42_i64,
                    "responseModel": "gpt-5"
                })
                .to_string(),
            ),
            t_total_ms: Some(100.0),
            t_req_read_ms: Some(0.0),
            t_req_parse_ms: Some(0.0),
            t_upstream_connect_ms: Some(0.0),
            t_upstream_ttfb_ms: Some(10.0),
            t_upstream_stream_ms: Some(20.0),
            t_resp_parse_ms: Some(0.0),
            t_persist_ms: Some(0.0),
        };
        let bucket_start_epoch = invocation_bucket_start_epoch(&archived_row.occurred_at)
            .expect("derive bucket start epoch");

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                id,
                occurred_at,
                source,
                status,
                detail_level,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                payload,
                t_total_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
            "#,
        )
        .bind(2_i64)
        .bind(&live_occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(DETAIL_LEVEL_FULL)
        .bind("gpt-5-mini")
        .bind(50_i64)
        .bind(70_i64)
        .bind(5_i64)
        .bind(125_i64)
        .bind(0.2_f64)
        .bind(0.04_f64)
        .bind(0.01_f64)
        .bind(0.02_f64)
        .bind(0.13_f64)
        .bind(0.0_f64)
        .bind(
            json!({
                "upstreamAccountId": 43_i64,
                "responseModel": "gpt-5-mini"
            })
            .to_string(),
        )
        .bind(130.0_f64)
        .bind(13.0_f64)
        .bind(26.0_f64)
        .execute(&pool)
        .await
        .expect("insert retained live invocation");

        for (
            upstream_account_key,
            upstream_account_id,
            normalized_model,
            output_tokens,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
        ) in [
            (
                "upstream:42",
                42_i64,
                "gpt-5",
                20_i64,
                0.02_f64,
                0.0_f64,
                0.0_f64,
                0.08_f64,
                0.0_f64,
            ),
            (
                "upstream:43",
                43_i64,
                "gpt-5-mini",
                70_i64,
                0.04_f64,
                0.01_f64,
                0.02_f64,
                0.13_f64,
                0.0_f64,
            ),
        ] {
            sqlx::query(
                r#"
                INSERT INTO upstream_account_usage_breakdown_hourly (
                    bucket_start_epoch,
                    source,
                    upstream_account_key,
                    upstream_account_id,
                    normalized_model,
                    normalized_reasoning_effort,
                    request_count,
                    success_count,
                    failure_count,
                    output_tokens,
                    cost_input,
                    cost_cache_write,
                    cost_cache_read,
                    cost_output,
                    cost_reasoning,
                    has_cost
                )
                VALUES (?1, ?2, ?3, ?4, ?5, '', 1, 1, 0, ?6, ?7, ?8, ?9, ?10, ?11, 1)
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(SOURCE_PROXY)
            .bind(upstream_account_key)
            .bind(upstream_account_id)
            .bind(normalized_model)
            .bind(output_tokens)
            .bind(cost_input)
            .bind(cost_cache_write)
            .bind(cost_cache_read)
            .bind(cost_output)
            .bind(cost_reasoning)
            .execute(&pool)
            .await
            .expect("seed existing breakdown rollup row");
        }

        let mut tx = pool.begin().await.expect("begin transaction");
        mark_retention_archived_hourly_rollup_targets_tx(
            tx.as_mut(),
            "codex_invocations",
            &[archived_row],
            &[],
        )
        .await
        .expect("mark retention archived hourly rollup targets");
        tx.commit().await.expect("commit transaction");

        let archived_breakdown_row_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM upstream_account_usage_breakdown_hourly
            WHERE bucket_start_epoch = ?1
              AND upstream_account_key = 'upstream:42'
            "#,
        )
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("count archived account breakdown rows");
        assert_eq!(archived_breakdown_row_count, 0);

        let retained = sqlx::query_as::<_, (i64, i64, i64, f64, f64)>(
            r#"
            SELECT request_count, success_count, output_tokens, cost_output, cost_unknown
            FROM upstream_account_usage_breakdown_hourly
            WHERE bucket_start_epoch = ?1
              AND upstream_account_key = 'upstream:43'
              AND normalized_model = 'gpt-5-mini'
            "#,
        )
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("load retained live breakdown row");
        assert_eq!(retained.0, 1);
        assert_eq!(retained.1, 1);
        assert_eq!(retained.2, 70);
        assert_eq!(retained.3, 0.13_f64);
        assert_eq!(retained.4, 0.0_f64);

        let usage_materialized_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hourly_rollup_materialized_buckets WHERE target = ?1 AND bucket_start_epoch = ?2",
        )
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("count usage materialized markers");
        assert_eq!(usage_materialized_count, 0);
    }

    #[tokio::test]
    async fn retention_partial_hour_preserves_previously_replayed_archive_breakdown_rows() {
        let pool = test_pool().await;
        let archived_occurred_at = format_naive(
            Utc.with_ymd_and_hms(2026, 7, 1, 12, 5, 0)
                .single()
                .expect("valid timestamp")
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let archived_row = InvocationHourlySourceRecord {
            id: 1,
            occurred_at: archived_occurred_at,
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: Some("gpt-5".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(20),
            cache_input_tokens: Some(0),
            total_tokens: Some(30),
            cost: Some(0.1),
            upstream_account_id: None,
            cost_input: Some(0.02),
            cost_cache_write: Some(0.0),
            cost_cache_read: Some(0.0),
            cost_output: Some(0.08),
            cost_reasoning: Some(0.0),
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: None,
            payload: Some(
                json!({
                    "upstreamAccountId": 42_i64,
                    "responseModel": "gpt-5"
                })
                .to_string(),
            ),
            t_total_ms: Some(100.0),
            t_req_read_ms: Some(0.0),
            t_req_parse_ms: Some(0.0),
            t_upstream_connect_ms: Some(0.0),
            t_upstream_ttfb_ms: Some(10.0),
            t_upstream_stream_ms: Some(20.0),
            t_resp_parse_ms: Some(0.0),
            t_persist_ms: Some(0.0),
        };
        let bucket_start_epoch = invocation_bucket_start_epoch(&archived_row.occurred_at)
            .expect("derive bucket start epoch");

        for (upstream_account_key, upstream_account_id, normalized_model, output_tokens, cost) in [
            ("upstream:41", 41_i64, "gpt-5-previous", 15_i64, 0.07_f64),
            ("upstream:42", 42_i64, "gpt-5", 20_i64, 0.08_f64),
        ] {
            sqlx::query(
                r#"
                INSERT INTO upstream_account_usage_breakdown_hourly (
                    bucket_start_epoch,
                    source,
                    upstream_account_key,
                    upstream_account_id,
                    normalized_model,
                    normalized_reasoning_effort,
                    request_count,
                    success_count,
                    failure_count,
                    output_tokens,
                    cost_output,
                    has_cost
                )
                VALUES (?1, ?2, ?3, ?4, ?5, '', 1, 1, 0, ?6, ?7, 1)
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(SOURCE_PROXY)
            .bind(upstream_account_key)
            .bind(upstream_account_id)
            .bind(normalized_model)
            .bind(output_tokens)
            .bind(cost)
            .execute(&pool)
            .await
            .expect("seed breakdown rollup row");
        }
        sqlx::query(
            r#"
            INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
            VALUES (?1, ?2, ?3, datetime('now'))
            "#,
        )
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind("/tmp/previous-usage-breakdown.sqlite.gz")
        .execute(&pool)
        .await
        .expect("seed previous archive replay marker");

        let mut tx = pool.begin().await.expect("begin transaction");
        mark_retention_archived_hourly_rollup_targets_tx(
            tx.as_mut(),
            "codex_invocations",
            &[archived_row],
            &[],
        )
        .await
        .expect("mark retention archived hourly rollup targets");
        tx.commit().await.expect("commit transaction");

        let previous = sqlx::query_as::<_, (i64, i64, f64)>(
            r#"
            SELECT request_count, output_tokens, cost_output
            FROM upstream_account_usage_breakdown_hourly
            WHERE bucket_start_epoch = ?1
              AND upstream_account_key = 'upstream:41'
              AND normalized_model = 'gpt-5-previous'
            "#,
        )
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("load previous archive breakdown row");
        assert_eq!(previous, (1, 15, 0.07_f64));

        let current_archived_row_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM upstream_account_usage_breakdown_hourly
            WHERE bucket_start_epoch = ?1
              AND upstream_account_key = 'upstream:42'
            "#,
        )
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("count current archived breakdown rows");
        assert_eq!(current_archived_row_count, 0);

        let previous_replay_marker_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
        )
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .bind("/tmp/previous-usage-breakdown.sqlite.gz")
        .fetch_one(&pool)
        .await
        .expect("count previous archive replay marker");
        assert_eq!(previous_replay_marker_count, 1);
    }

    #[tokio::test]
    async fn breakdown_rollup_uses_resolved_upstream_account_id() {
        let pool = test_pool().await;
        let occurred_at = format_naive(
            Utc.with_ymd_and_hms(2026, 7, 1, 13, 4, 5)
                .single()
                .expect("valid timestamp")
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let row = InvocationHourlySourceRecord {
            id: 2,
            occurred_at,
            source: SOURCE_PROXY.to_string(),
            status: Some("success".to_string()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: Some("gpt-5".to_string()),
            input_tokens: Some(25),
            output_tokens: Some(35),
            cache_input_tokens: Some(5),
            total_tokens: Some(60),
            cost: Some(0.3),
            upstream_account_id: Some(42),
            cost_input: Some(0.1),
            cost_cache_write: Some(0.02),
            cost_cache_read: Some(0.03),
            cost_output: Some(0.15),
            cost_reasoning: Some(0.0),
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: None,
            payload: Some(
                json!({
                    "responseModel": "gpt-5"
                })
                .to_string(),
            ),
            t_total_ms: Some(120.0),
            t_req_read_ms: Some(0.0),
            t_req_parse_ms: Some(0.0),
            t_upstream_connect_ms: Some(0.0),
            t_upstream_ttfb_ms: Some(12.0),
            t_upstream_stream_ms: Some(24.0),
            t_resp_parse_ms: Some(0.0),
            t_persist_ms: Some(0.0),
        };

        let mut tx = pool.begin().await.expect("begin tx");
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &[row],
            &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
        )
        .await
        .expect("upsert breakdown rollup");
        tx.commit().await.expect("commit breakdown rollup");

        let stored = sqlx::query_as::<_, (String, Option<i64>, i64)>(
            r#"
            SELECT upstream_account_key, upstream_account_id, request_count
            FROM upstream_account_usage_breakdown_hourly
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load stored breakdown row");
        assert_eq!(stored.0, "upstream:42");
        assert_eq!(stored.1, Some(42));
        assert_eq!(stored.2, 1);
    }

    #[tokio::test]
    async fn breakdown_rollup_excludes_running_and_pending_rows() {
        let pool = test_pool().await;
        let occurred_at = format_naive(
            Utc.with_ymd_and_hms(2026, 7, 1, 14, 4, 5)
                .single()
                .expect("valid timestamp")
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let payload = json!({
            "upstreamAccountId": 42_i64,
            "responseModel": "gpt-5",
        })
        .to_string();

        let rows = [
            InvocationHourlySourceRecord {
                id: 10,
                occurred_at: occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some("success".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: Some("gpt-5".to_string()),
                input_tokens: Some(10),
                output_tokens: Some(20),
                cache_input_tokens: Some(0),
                total_tokens: Some(30),
                cost: Some(0.30),
                upstream_account_id: Some(42),
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none".to_string()),
                is_actionable: Some(0),
                payload: Some(payload.clone()),
                t_total_ms: Some(100.0),
                t_req_read_ms: Some(0.0),
                t_req_parse_ms: Some(0.0),
                t_upstream_connect_ms: Some(0.0),
                t_upstream_ttfb_ms: Some(10.0),
                t_upstream_stream_ms: Some(20.0),
                t_resp_parse_ms: Some(0.0),
                t_persist_ms: Some(0.0),
            },
            InvocationHourlySourceRecord {
                id: 11,
                occurred_at: occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some("failed".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: Some("gpt-5".to_string()),
                input_tokens: Some(11),
                output_tokens: Some(21),
                cache_input_tokens: Some(0),
                total_tokens: Some(32),
                cost: Some(0.20),
                upstream_account_id: Some(42),
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: Some("upstream stream error".to_string()),
                failure_kind: Some("upstream_response_failed".to_string()),
                failure_class: Some("service_failure".to_string()),
                is_actionable: Some(1),
                payload: Some(payload.clone()),
                t_total_ms: Some(101.0),
                t_req_read_ms: Some(0.0),
                t_req_parse_ms: Some(0.0),
                t_upstream_connect_ms: Some(0.0),
                t_upstream_ttfb_ms: Some(11.0),
                t_upstream_stream_ms: Some(21.0),
                t_resp_parse_ms: Some(0.0),
                t_persist_ms: Some(0.0),
            },
            InvocationHourlySourceRecord {
                id: 12,
                occurred_at: occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some("running".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: Some("gpt-5".to_string()),
                input_tokens: Some(12),
                output_tokens: Some(22),
                cache_input_tokens: Some(0),
                total_tokens: Some(34),
                cost: Some(0.40),
                upstream_account_id: Some(42),
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none".to_string()),
                is_actionable: Some(0),
                payload: Some(payload.clone()),
                t_total_ms: Some(102.0),
                t_req_read_ms: Some(0.0),
                t_req_parse_ms: Some(0.0),
                t_upstream_connect_ms: Some(0.0),
                t_upstream_ttfb_ms: Some(12.0),
                t_upstream_stream_ms: Some(22.0),
                t_resp_parse_ms: Some(0.0),
                t_persist_ms: Some(0.0),
            },
            InvocationHourlySourceRecord {
                id: 13,
                occurred_at,
                source: SOURCE_PROXY.to_string(),
                status: Some("pending".to_string()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: Some("gpt-5".to_string()),
                input_tokens: Some(13),
                output_tokens: Some(23),
                cache_input_tokens: Some(0),
                total_tokens: Some(36),
                cost: Some(0.50),
                upstream_account_id: Some(42),
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                error_message: None,
                failure_kind: None,
                failure_class: Some("none".to_string()),
                is_actionable: Some(0),
                payload: Some(payload),
                t_total_ms: Some(103.0),
                t_req_read_ms: Some(0.0),
                t_req_parse_ms: Some(0.0),
                t_upstream_connect_ms: Some(0.0),
                t_upstream_ttfb_ms: Some(13.0),
                t_upstream_stream_ms: Some(23.0),
                t_resp_parse_ms: Some(0.0),
                t_persist_ms: Some(0.0),
            },
        ];

        let mut tx = pool.begin().await.expect("begin tx");
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &rows,
            &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
        )
        .await
        .expect("upsert breakdown rollup");
        tx.commit().await.expect("commit breakdown rollup");

        let stored = sqlx::query_as::<_, (i64, i64, i64, i64, f64)>(
            r#"
            SELECT request_count, success_count, failure_count, output_tokens, cost_unknown
            FROM upstream_account_usage_breakdown_hourly
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("load stored breakdown row");
        assert_eq!(stored.0, 2);
        assert_eq!(stored.1, 1);
        assert_eq!(stored.2, 1);
        assert_eq!(stored.3, 41);
        assert_eq!(stored.4, 0.50_f64);
    }

    #[tokio::test]
    async fn repair_materialized_breakdown_reopens_overlapping_replayed_batches() {
        let pool = test_pool().await;
        let bucket_start_epoch = invocation_bucket_start_epoch("2026-07-01 15:05:00")
            .expect("derive bucket start epoch");
        let first_file_path = "/tmp/usage-breakdown-overlap-first.sqlite.gz";
        let second_file_path = "/tmp/usage-breakdown-overlap-second.sqlite.gz";

        for (file_path, coverage_start_at, coverage_end_at, replayed, cursor_id) in [
            (
                first_file_path,
                "2026-07-01 15:05:00",
                "2026-07-01 15:15:00",
                false,
                101_i64,
            ),
            (
                second_file_path,
                "2026-07-01 15:25:00",
                "2026-07-01 15:35:00",
                true,
                202_i64,
            ),
        ] {
            sqlx::query(
                r#"
                INSERT INTO archive_batches (
                    dataset,
                    month_key,
                    file_path,
                    status,
                    coverage_start_at,
                    coverage_end_at,
                    historical_rollups_materialized_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                "#,
            )
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .bind("2026-07")
            .bind(file_path)
            .bind(ARCHIVE_STATUS_COMPLETED)
            .bind(coverage_start_at)
            .bind(coverage_end_at)
            .execute(&pool)
            .await
            .expect("insert archive batch");

            sqlx::query(
                r#"
                INSERT INTO hourly_rollup_archive_progress (dataset, file_path, cursor_id, updated_at)
                VALUES (?1, ?2, ?3, datetime('now'))
                "#,
            )
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .bind(file_path)
            .bind(cursor_id)
            .execute(&pool)
            .await
            .expect("insert archive progress");

            if replayed {
                sqlx::query(
                    r#"
                    INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, replayed_at)
                    VALUES (?1, ?2, ?3, datetime('now'))
                    "#,
                )
                .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
                .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
                .bind(file_path)
                .execute(&pool)
                .await
                .expect("insert replay marker");
            }
        }

        for (upstream_account_key, upstream_account_id, normalized_model) in [
            ("upstream:17", Some(17_i64), "gpt-5"),
            ("upstream:18", Some(18_i64), "gpt-5-mini"),
        ] {
            sqlx::query(
                r#"
                INSERT INTO upstream_account_usage_breakdown_hourly (
                    bucket_start_epoch,
                    source,
                    upstream_account_key,
                    upstream_account_id,
                    normalized_model,
                    normalized_reasoning_effort,
                    request_count,
                    success_count,
                    failure_count
                )
                VALUES (?1, ?2, ?3, ?4, ?5, '', 1, 1, 0)
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(SOURCE_PROXY)
            .bind(upstream_account_key)
            .bind(upstream_account_id)
            .bind(normalized_model)
            .execute(&pool)
            .await
            .expect("seed breakdown rollup row");
        }

        let touched = repair_materialized_invocation_archive_usage_breakdown_backfill_state(&pool)
            .await
            .expect("repair materialized usage breakdown state");
        assert_eq!(touched, 2);

        let remaining_rows: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM upstream_account_usage_breakdown_hourly WHERE bucket_start_epoch = ?1",
        )
        .bind(bucket_start_epoch)
        .fetch_one(&pool)
        .await
        .expect("count remaining breakdown rows");
        assert_eq!(remaining_rows, 0);

        for file_path in [first_file_path, second_file_path] {
            let replay_marker_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
            )
            .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .bind(file_path)
            .fetch_one(&pool)
            .await
            .expect("count replay markers after repair");
            assert_eq!(replay_marker_count, 0);

            let materialized_at: Option<String> = sqlx::query_scalar(
                "SELECT historical_rollups_materialized_at FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
            )
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .bind(file_path)
            .fetch_one(&pool)
            .await
            .expect("load archive materialized state after repair");
            assert!(materialized_at.is_none());

            let progress_count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM hourly_rollup_archive_progress WHERE dataset = ?1 AND file_path = ?2",
            )
            .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .bind(file_path)
            .fetch_one(&pool)
            .await
            .expect("count archive progress rows after repair");
            assert_eq!(progress_count, 0);
        }
    }
}
