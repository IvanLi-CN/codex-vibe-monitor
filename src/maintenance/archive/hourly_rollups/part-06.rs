#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ActiveAccountActivityV2RepairOutcome {
    pub(crate) priority_bucket_count: usize,
    pub(crate) repaired_bucket_count: usize,
    pub(crate) elapsed_ms: u64,
}

pub(crate) async fn repair_active_account_activity_v2_coverage(
    pool: &Pool<Sqlite>,
) -> Result<ActiveAccountActivityV2RepairOutcome> {
    let started_at = Instant::now();
    if started_at.elapsed() >= ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET {
        return Ok(ActiveAccountActivityV2RepairOutcome::default());
    }
    let Some(remaining_budget) =
        ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET.checked_sub(started_at.elapsed())
    else {
        return Ok(ActiveAccountActivityV2RepairOutcome::default());
    };
    let mut generation_tx = match timeout(remaining_budget, pool.begin()).await {
        Ok(tx) => tx?,
        Err(_) => return Ok(ActiveAccountActivityV2RepairOutcome::default()),
    };
    ensure_account_activity_v2_repair_generation_tx(generation_tx.as_mut()).await?;
    generation_tx.commit().await?;
    let current_bucket = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let Some(missing_buckets) =
        select_active_account_activity_v2_priority_buckets(pool, current_bucket, started_at)
            .await?
    else {
        return Ok(ActiveAccountActivityV2RepairOutcome {
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            ..ActiveAccountActivityV2RepairOutcome::default()
        });
    };
    let priority_bucket_count = missing_buckets.len();
    let mut repaired_bucket_count = 0;
    for bucket_start_epoch in missing_buckets {
        if !repair_active_account_activity_v2_bucket(pool, bucket_start_epoch, started_at).await? {
            break;
        }
        repaired_bucket_count += 1;
    }

    let outcome = ActiveAccountActivityV2RepairOutcome {
        priority_bucket_count,
        repaired_bucket_count,
        elapsed_ms: started_at.elapsed().as_millis() as u64,
    };
    if repaired_bucket_count > 0 {
        tracing::info!(
            priority_mode = "active_dashboard_coverage",
            coverage_priority_bucket_count = priority_bucket_count,
            repaired_bucket_count,
            priority_batch_limit = ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUCKET_LIMIT,
            priority_elapsed_budget_ms =
                ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET.as_millis() as u64,
            elapsed_ms = outcome.elapsed_ms,
            wake_reason = "active_window_coverage_hole",
            "repaired active Dashboard account activity v2 coverage"
        );
    }
    Ok(outcome)
}

const RESET_ACTIVE_ACCOUNT_ACTIVITY_V2_BUCKET_SQL: &str = r#"
UPDATE upstream_account_stats_hourly
SET activity_v2_request_count = 0,
    activity_v2_success_count = 0,
    activity_v2_failure_count = 0,
    activity_v2_non_success_count = 0,
    activity_v2_total_tokens = 0,
    activity_v2_success_tokens = 0,
    activity_v2_non_success_tokens = 0,
    activity_v2_failure_tokens = 0,
    activity_v2_failure_cost = 0,
    activity_v2_non_success_cost = 0,
    activity_v2_cache_input_tokens = 0,
    activity_v2_total_cost = 0,
    activity_v2_first_response_sample_count = 0,
    activity_v2_first_response_sum_ms = 0,
    activity_v2_first_token_sample_count = 0,
    activity_v2_first_token_sum_ms = 0,
    activity_v2_first_token_max_ms = 0,
    activity_v2_first_token_histogram = '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
    activity_v2_total_latency_sample_count = 0,
    activity_v2_total_latency_sum_ms = 0,
    activity_v2_last_invocation_at = NULL,
    activity_v2_latest_unkeyed_conversation_at = NULL,
    activity_v2_latest_first_response_at = NULL,
    activity_v2_latest_first_response_ms = NULL,
    activity_v2_latest_total_latency_at = NULL,
    activity_v2_latest_total_latency_ms = NULL,
    updated_at = datetime('now')
WHERE bucket_start_epoch = ?1
"#;

async fn repair_active_account_activity_v2_bucket(
    pool: &Pool<Sqlite>,
    bucket_start_epoch: i64,
    started_at: Instant,
) -> Result<bool> {
    let Some(remaining_budget) =
        ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET.checked_sub(started_at.elapsed())
    else {
        return Ok(false);
    };
    let mut tx = match timeout(remaining_budget, pool.begin()).await {
        Ok(tx) => tx?,
        Err(_) => return Ok(false),
    };
    ensure_account_activity_v2_repair_generation_tx(tx.as_mut()).await?;
    sqlx::query(RESET_ACTIVE_ACCOUNT_ACTIVITY_V2_BUCKET_SQL)
        .bind(bucket_start_epoch)
        .execute(tx.as_mut())
        .await?;
    let rows =
        load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx.as_mut(), &[bucket_start_epoch])
            .await?;
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &rows,
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
    )
    .await?;
    let bucket_max_id = rows
        .iter()
        .filter(|row| invocation_row_counts_toward_account_activity_v2(row))
        .map(|row| row.id)
        .max()
        .unwrap_or_default();
    save_account_activity_v2_bucket_repair_watermark_tx(
        tx.as_mut(),
        bucket_start_epoch,
        bucket_max_id,
    )
    .await?;
    mark_hourly_rollup_bucket_materialized_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
        bucket_start_epoch,
        HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
    )
    .await?;
    tx.commit().await?;
    Ok(true)
}

fn invocation_row_counts_toward_account_activity_v2(row: &InvocationHourlySourceRecord) -> bool {
    !matches!(
        row.status
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "running" | "pending"
    )
}

async fn save_account_activity_v2_bucket_repair_watermark_tx(
    tx: &mut SqliteConnection,
    bucket_start_epoch: i64,
    cursor_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO account_activity_v2_bucket_repair_watermarks (
            bucket_start_epoch, cursor_id, updated_at
        )
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(bucket_start_epoch) DO UPDATE SET
            cursor_id = MAX(account_activity_v2_bucket_repair_watermarks.cursor_id, excluded.cursor_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(cursor_id.max(0))
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn load_account_activity_v2_bucket_repair_watermarks_for_rows_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<BTreeMap<i64, i64>> {
    let mut bucket_epochs = rows
        .iter()
        .map(|row| invocation_bucket_start_epoch(&row.occurred_at))
        .collect::<Result<Vec<_>>>()?;
    bucket_epochs.sort_unstable();
    bucket_epochs.dedup();
    if bucket_epochs.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT bucket_start_epoch, cursor_id FROM account_activity_v2_bucket_repair_watermarks WHERE bucket_start_epoch IN (",
    );
    {
        let mut separated = query.separated(", ");
        for bucket_epoch in bucket_epochs {
            separated.push_bind(bucket_epoch);
        }
    }
    query.push(")");
    Ok(query
        .build_query_as::<(i64, i64)>()
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>())
}

async fn ensure_account_activity_v2_repair_generation_tx(tx: &mut SqliteConnection) -> Result<()> {
    let generation = load_hourly_rollup_live_progress_tx(
        tx,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET,
    )
    .await?;
    if generation >= INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION {
        return Ok(());
    }

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_materialized_buckets
        WHERE target = ?1 AND source = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM account_activity_v2_bucket_repair_watermarks")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        UPDATE upstream_account_stats_hourly
        SET activity_v2_request_count = 0,
            activity_v2_success_count = 0,
            activity_v2_failure_count = 0,
            activity_v2_non_success_count = 0,
            activity_v2_total_tokens = 0,
            activity_v2_success_tokens = 0,
            activity_v2_non_success_tokens = 0,
            activity_v2_failure_tokens = 0,
            activity_v2_failure_cost = 0,
            activity_v2_non_success_cost = 0,
            activity_v2_cache_input_tokens = 0,
            activity_v2_total_cost = 0,
            activity_v2_first_response_sample_count = 0,
            activity_v2_first_response_sum_ms = 0,
            activity_v2_first_token_sample_count = 0,
            activity_v2_first_token_sum_ms = 0,
            activity_v2_first_token_max_ms = 0,
            activity_v2_first_token_histogram = '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            activity_v2_total_latency_sample_count = 0,
            activity_v2_total_latency_sum_ms = 0,
            activity_v2_last_invocation_at = NULL,
            activity_v2_latest_unkeyed_conversation_at = NULL,
            activity_v2_latest_first_response_at = NULL,
            activity_v2_latest_first_response_ms = NULL,
            activity_v2_latest_total_latency_at = NULL,
            activity_v2_latest_total_latency_ms = NULL,
            updated_at = datetime('now')
        "#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset = ?1")
        .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM hourly_rollup_archive_progress WHERE dataset = ?1")
        .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2")
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = NULL
        WHERE dataset = ?1
          AND status = 'completed'
          AND historical_rollups_materialized_at IS NOT NULL
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .execute(&mut *tx)
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION,
    )
    .await?;
    tracing::info!(
        repair_generation = INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION,
        "initialized account activity v2 repair generation"
    );
    Ok(())
}

async fn mark_live_account_activity_v2_coverage_tx(
    tx: &mut SqliteConnection,
    repair_cursor: i64,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    let current_bucket = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let mut bucket_epochs = rows
        .iter()
        .map(|row| invocation_bucket_start_epoch(&row.occurred_at))
        .collect::<Result<Vec<_>>>()?;
    bucket_epochs.sort_unstable();
    bucket_epochs.dedup();
    for bucket_start_epoch in bucket_epochs {
        if bucket_start_epoch >= current_bucket {
            continue;
        }
        let bucket_start = Utc
            .timestamp_opt(bucket_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid account activity coverage bucket start"))?;
        let bucket_end = bucket_start + ChronoDuration::hours(1);
        let bucket_max_id = sqlx::query_scalar::<_, Option<i64>>(
            r#"
            SELECT MAX(id)
            FROM codex_invocations
            WHERE occurred_at >= ?1 AND occurred_at < ?2
            "#,
        )
        .bind(db_occurred_at_lower_bound(bucket_start))
        .bind(db_occurred_at_upper_bound(bucket_end))
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or_default();
        if bucket_max_id > repair_cursor {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }
    Ok(())
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

    let rows = load_live_invocation_usage_breakdown_repair_rows(
        tx.as_mut(),
        repair_cursor,
        shared_live_cursor,
    )
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

async fn load_live_invocation_usage_breakdown_repair_rows(
    tx: &mut SqliteConnection,
    repair_cursor: i64,
    shared_live_cursor: i64,
) -> Result<Vec<InvocationHourlySourceRecord>> {
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(tx).await?;
    Ok(sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
        r#"
        SELECT
            id, occurred_at, source, status, detail_level, model, input_tokens, output_tokens,
            cache_input_tokens, reasoning_tokens, total_tokens, cost,
            {} AS upstream_account_id,
            cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
            error_message, failure_kind, failure_class, is_actionable, payload,
            t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
            t_upstream_ttfb_ms, {} AS first_token_ms, t_upstream_stream_ms,
            t_resp_parse_ms, t_persist_ms
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
    .fetch_all(tx)
    .await?)
}
