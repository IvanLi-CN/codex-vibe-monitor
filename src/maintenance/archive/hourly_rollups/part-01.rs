pub(crate) fn parallel_work_minute_rollup_keep_start_epoch(now: DateTime<Utc>) -> Result<i64> {
    let local_date = now.with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(PARALLEL_WORK_MINUTE_ROLLUP_RETAINED_COMPLETE_SHANGHAI_DAYS);
    let local_midnight = local_date
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow!("invalid Shanghai minute-rollup retention boundary"))?;
    Shanghai
        .from_local_datetime(&local_midnight)
        .single()
        .ok_or_else(|| anyhow!("ambiguous Shanghai minute-rollup retention boundary"))
        .map(|boundary| boundary.with_timezone(&Utc).timestamp())
}

async fn upsert_parallel_work_minute_key_rollups_with_floor_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
    minute_floor_epoch: Option<i64>,
) -> Result<()> {
    let mut global_keys = BTreeSet::new();
    let mut account_keys = BTreeSet::new();
    for row in rows {
        let Some(prompt_cache_key) = prompt_cache_key_from_payload(row.payload.as_deref()) else {
            continue;
        };
        let minute_start_epoch = invocation_bucket_start_epoch_for_seconds(&row.occurred_at, 60)?;
        if minute_floor_epoch.is_some_and(|floor| minute_start_epoch < floor) {
            continue;
        }
        global_keys.insert((
            minute_start_epoch,
            row.source.clone(),
            prompt_cache_key.clone(),
        ));
        if let Some(upstream_account_id) = row.resolved_upstream_account_id() {
            account_keys.insert((
                minute_start_epoch,
                row.source.clone(),
                upstream_account_id,
                prompt_cache_key,
            ));
        }
    }

    for (minute_start_epoch, source, prompt_cache_key) in global_keys {
        sqlx::query(
            "INSERT OR IGNORE INTO parallel_work_minute_key_rollup \
             (minute_start_epoch, source, prompt_cache_key) VALUES (?1, ?2, ?3)",
        )
        .bind(minute_start_epoch)
        .bind(source)
        .bind(prompt_cache_key)
        .execute(&mut *tx)
        .await?;
    }
    for (minute_start_epoch, source, upstream_account_id, prompt_cache_key) in account_keys {
        sqlx::query(
            "INSERT OR IGNORE INTO parallel_work_upstream_account_minute_key_rollup \
             (minute_start_epoch, source, upstream_account_id, prompt_cache_key) \
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(minute_start_epoch)
        .bind(source)
        .bind(upstream_account_id)
        .bind(prompt_cache_key)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

pub(crate) async fn upsert_parallel_work_minute_key_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    upsert_parallel_work_minute_key_rollups_with_floor_tx(
        tx,
        rows,
        Some(parallel_work_minute_rollup_keep_start_epoch(Utc::now())?),
    )
    .await
}

pub(crate) async fn upsert_parallel_work_minute_key_rollups_including_expired_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    upsert_parallel_work_minute_key_rollups_with_floor_tx(tx, rows, None).await
}

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

async fn live_invocation_first_token_ms_sql_tx(tx: &mut SqliteConnection) -> Result<&'static str> {
    let has_column = sqlx::query_scalar::<_, String>(
        "SELECT name FROM pragma_table_info('codex_invocations') WHERE name = 'first_token_ms' LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    Ok(if has_column { "first_token_ms" } else { "NULL" })
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

const SUBTRACT_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN_SQL: &str = r#"
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
    performance_first_token_sample_count = MAX(performance_first_token_sample_count - ?26, 0),
    performance_first_token_sum_ms = MAX(performance_first_token_sum_ms - ?27, 0.0),
    performance_usage_duration_sample_count = MAX(performance_usage_duration_sample_count - ?28, 0),
    performance_usage_duration_sum_ms = MAX(performance_usage_duration_sum_ms - ?29, 0.0),
    updated_at = datetime('now')
WHERE bucket_start_epoch = ?1
  AND source = ?2
  AND upstream_account_key = ?3
  AND normalized_model = ?4
  AND normalized_reasoning_effort = ?5
"#;

const DELETE_EMPTY_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN_SQL: &str = r#"
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
  AND performance_first_token_sample_count = 0
  AND performance_first_token_sum_ms = 0.0
  AND performance_usage_duration_sample_count = 0
  AND performance_usage_duration_sum_ms = 0.0
"#;

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
        sqlx::query(SUBTRACT_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN_SQL)
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
            .bind(delta.performance_first_token_sample_count)
            .bind(delta.performance_first_token_sum_ms)
            .bind(delta.performance_usage_duration_sample_count)
            .bind(delta.performance_usage_duration_sum_ms)
            .execute(&mut *tx)
            .await?;

        sqlx::query(DELETE_EMPTY_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN_SQL)
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
pub(crate) const INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET: &str =
    "invocation_account_activity_v2_repair_live_cursor";
pub(crate) const INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET: &str =
    "codex_invocations_account_activity_v2_archive_progress";
pub(crate) const INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET: &str =
    "invocation_account_activity_v2_repair_generation";
const INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION: i64 = 1;
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
    let select = |column_name| legacy_compatible_archive_select_expr(archive_columns, column_name);
    let detail_level = if archive_columns.contains("detail_level") {
        "detail_level".to_string()
    } else {
        "'full' AS detail_level".to_string()
    };
    let status = select("status");
    let model = select("model");
    let has_complete_token_components = [
        "input_tokens",
        "output_tokens",
        "cache_input_tokens",
        "reasoning_tokens",
    ]
    .iter()
    .all(|column| archive_columns.contains(*column));
    let token_component = |column_name: &str| {
        if !has_complete_token_components {
            return format!("NULL AS {column_name}");
        }
        format!(
            "CASE WHEN input_tokens IS NULL OR output_tokens IS NULL OR cache_input_tokens IS NULL OR reasoning_tokens IS NULL THEN NULL ELSE {column_name} END AS {column_name}"
        )
    };
    let input_tokens = token_component("input_tokens");
    let output_tokens = token_component("output_tokens");
    let cache_input_tokens = token_component("cache_input_tokens");
    let reasoning_tokens = token_component("reasoning_tokens");
    let total_tokens = select("total_tokens");
    let cost = select("cost");
    let upstream_account_id = select("upstream_account_id");
    let cost_input = select("cost_input");
    let cost_cache_write = select("cost_cache_write");
    let cost_cache_read = select("cost_cache_read");
    let cost_output = select("cost_output");
    let cost_reasoning = select("cost_reasoning");
    let error_message = select("error_message");
    let failure_kind = select("failure_kind");
    let failure_class = select("failure_class");
    let is_actionable = select("is_actionable");
    let payload = select("payload");
    let t_total_ms = select("t_total_ms");
    let t_req_read_ms = select("t_req_read_ms");
    let t_req_parse_ms = select("t_req_parse_ms");
    let t_upstream_connect_ms = select("t_upstream_connect_ms");
    let t_upstream_ttfb_ms = select("t_upstream_ttfb_ms");
    let first_token_ms = select("first_token_ms");
    let t_upstream_stream_ms = select("t_upstream_stream_ms");
    let t_resp_parse_ms = select("t_resp_parse_ms");
    let t_persist_ms = select("t_persist_ms");
    format!(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            {status},
            {detail_level},
            {model},
            {input_tokens},
            {output_tokens},
            {cache_input_tokens},
            {reasoning_tokens},
            {total_tokens},
            {cost},
            {upstream_account_id},
            {cost_input},
            {cost_cache_write},
            {cost_cache_read},
            {cost_output},
            {cost_reasoning},
            {error_message},
            {failure_kind},
            {failure_class},
            {is_actionable},
            {payload},
            {t_total_ms},
            {t_req_read_ms},
            {t_req_parse_ms},
            {t_upstream_connect_ms},
            {t_upstream_ttfb_ms},
            {first_token_ms},
            {t_upstream_stream_ms},
            {t_resp_parse_ms},
            {t_persist_ms}
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
