pub(crate) fn invocation_archive_target_needs_full_payload(target: &str) -> bool {
    matches!(
        target,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE
            | HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS
            | HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE
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
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(tx).await?;

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
            reasoning_tokens,
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
            {} AS first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
         FROM codex_invocations
         WHERE occurred_at >= ?1
           AND occurred_at < ?2
         ORDER BY id ASC",
        upstream_account_id_sql, first_token_ms_sql,
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
    rebuild_parallel_work_rollups_for_hours_tx(tx, &bucket_epochs).await?;
    let mut bucket_watermarks = BTreeMap::<i64, i64>::new();
    for row in rows
        .iter()
        .filter(|row| invocation_row_counts_toward_account_activity_v2(row))
    {
        let bucket_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        bucket_watermarks
            .entry(bucket_epoch)
            .and_modify(|cursor_id| *cursor_id = (*cursor_id).max(row.id))
            .or_insert(row.id);
    }
    for (bucket_epoch, cursor_id) in bucket_watermarks {
        save_account_activity_v2_bucket_repair_watermark_tx(tx, bucket_epoch, cursor_id).await?;
    }
    Ok(())
}

pub(crate) async fn replay_live_invocation_hourly_rollups(pool: &Pool<Sqlite>) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let account_activity_v2_cursor = load_hourly_rollup_live_progress(
        pool,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    )
    .await?;
    let rows = {
        let mut conn = pool.acquire().await?;
        let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
            "codex_invocations",
            load_pool_attempt_fallback_capability_tx(&mut conn).await?,
        );
        let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(&mut conn).await?;
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
            reasoning_tokens,
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
            {} AS first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
            upstream_account_id_sql, first_token_ms_sql,
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
    upsert_live_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &rows,
        account_activity_v2_cursor,
        cursor_id,
    )
    .await?;
    save_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id)
        .await?;
    if account_activity_v2_cursor == cursor_id {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
            last_id,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_invocation_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let account_activity_v2_cursor = load_hourly_rollup_live_progress_tx(
        tx,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    )
    .await?;
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(tx).await?;
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
            reasoning_tokens,
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
            {} AS first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
        upstream_account_id_sql, first_token_ms_sql,
    ))
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_live_invocation_hourly_rollups_tx(tx, &rows, account_activity_v2_cursor, cursor_id)
        .await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id).await?;
    if account_activity_v2_cursor == cursor_id {
        save_hourly_rollup_live_progress_tx(
            tx,
            INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
            last_id,
        )
        .await?;
    }
    Ok(rows.len() as u64)
}

async fn upsert_live_invocation_hourly_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
    account_activity_v2_cursor: i64,
    shared_cursor: i64,
) -> Result<()> {
    let base_targets = INVOCATION_HOURLY_ROLLUP_TARGETS
        .iter()
        .copied()
        .filter(|target| *target != HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
        .collect::<Vec<_>>();
    upsert_invocation_hourly_rollups_tx(tx, rows, &base_targets).await?;

    if account_activity_v2_cursor == shared_cursor {
        let bucket_watermarks =
            load_account_activity_v2_bucket_repair_watermarks_for_rows_tx(tx, rows).await?;
        let v2_rows = rows
            .iter()
            .filter(|row| {
                invocation_bucket_start_epoch(&row.occurred_at)
                    .map(|bucket_epoch| {
                        row.id
                            > bucket_watermarks
                                .get(&bucket_epoch)
                                .copied()
                                .unwrap_or_default()
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        upsert_invocation_hourly_rollups_tx(
            tx,
            &v2_rows,
            &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn repair_live_invocation_account_activity_v2_once(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    ensure_account_activity_v2_repair_generation_tx(tx.as_mut()).await?;
    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_cursor = load_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    )
    .await?;

    if repair_cursor >= shared_live_cursor {
        tx.commit().await?;
        return Ok(0);
    }

    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx.as_mut()).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(tx.as_mut()).await?;
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
        r#"
        SELECT
            id, occurred_at, source, status, detail_level, model,
            input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost,
            {} AS upstream_account_id,
            cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
            error_message, failure_kind, failure_class, is_actionable, payload,
            t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
            t_upstream_ttfb_ms, {} AS first_token_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms
        FROM codex_invocations
        WHERE id > ?1 AND id <= ?2
        ORDER BY id ASC
        LIMIT ?3
        "#,
        upstream_account_id_sql, first_token_ms_sql,
    ))
    .bind(repair_cursor)
    .bind(shared_live_cursor)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;

    if rows.is_empty() {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
            shared_live_cursor,
        )
        .await?;
        tx.commit().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(repair_cursor);
    let bucket_watermarks =
        load_account_activity_v2_bucket_repair_watermarks_for_rows_tx(tx.as_mut(), &rows).await?;
    let rows_to_replay = rows
        .iter()
        .filter(|row| {
            invocation_bucket_start_epoch(&row.occurred_at)
                .map(|bucket_epoch| {
                    row.id
                        > bucket_watermarks
                            .get(&bucket_epoch)
                            .copied()
                            .unwrap_or_default()
                })
                .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &rows_to_replay,
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
        last_id,
    )
    .await?;
    mark_live_account_activity_v2_coverage_tx(tx.as_mut(), last_id, &rows).await?;
    tx.commit().await?;
    tracing::debug!(
        archive_replay_target = HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
        repair_cursor_before = repair_cursor,
        repair_cursor_after = last_id,
        shared_live_cursor,
        repaired_row_count = rows.len(),
        coverage_complete = last_id >= shared_live_cursor,
        "repaired live account activity v2 rollup batch"
    );
    Ok(rows.len() as u64)
}

const ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUCKET_LIMIT: usize = 2;
const ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET: Duration = Duration::from_secs(2);

pub(crate) fn build_active_account_activity_v2_archive_epoch_coverage_query(
    prefix: &'static str,
    oldest_bucket: i64,
    current_bucket: i64,
) -> QueryBuilder<'static, Sqlite> {
    let mut query = QueryBuilder::<Sqlite>::new(prefix);
    query.push(
        "SELECT coverage_start_epoch, coverage_end_epoch \
         FROM archive_batches INDEXED BY idx_archive_batches_invocation_coverage_epoch \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND coverage_start_epoch IS NOT NULL \
           AND coverage_end_epoch IS NOT NULL \
           AND coverage_start_epoch < ",
    );
    query.push_bind(current_bucket);
    query.push(" AND coverage_end_epoch >= ");
    query.push_bind(oldest_bucket);
    query
}

pub(crate) fn build_active_account_activity_v2_legacy_coverage_query(
    prefix: &'static str,
    active_month_keys: &[String],
) -> QueryBuilder<'static, Sqlite> {
    let mut query = QueryBuilder::<Sqlite>::new(prefix);
    query.push(
        "SELECT month_key \
         FROM archive_batches INDEXED BY idx_archive_batches_invocation_legacy_coverage_month \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND (coverage_start_at IS NULL OR coverage_end_at IS NULL)",
    );
    if active_month_keys.is_empty() {
        query.push(" AND 0");
        return query;
    }
    query.push(" AND month_key IN (");
    {
        let mut separated = query.separated(", ");
        for month_key in active_month_keys {
            separated.push_bind(month_key.clone());
        }
    }
    query.push(")");
    query
}

fn active_account_activity_v2_month_key(bucket_start_epoch: i64) -> Result<String> {
    Utc.timestamp_opt(bucket_start_epoch, 0)
        .single()
        .map(|bucket_start| {
            bucket_start
                .with_timezone(&Shanghai)
                .format("%Y-%m")
                .to_string()
        })
        .ok_or_else(|| anyhow!("invalid account activity v2 priority bucket start"))
}
