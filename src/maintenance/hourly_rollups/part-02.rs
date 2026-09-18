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

pub(crate) fn build_invocation_archive_rows_chunk_query(
    archive_columns: &HashSet<String>,
) -> String {
    let input_tokens = legacy_compatible_archive_select_expr(archive_columns, "input_tokens");
    let output_tokens = legacy_compatible_archive_select_expr(archive_columns, "output_tokens");
    let cache_input_tokens =
        legacy_compatible_archive_select_expr(archive_columns, "cache_input_tokens");
    let model = legacy_compatible_archive_select_expr(archive_columns, "model");
    let cost_input = legacy_compatible_archive_select_expr(archive_columns, "cost_input");
    let cost_cache_write =
        legacy_compatible_archive_select_expr(archive_columns, "cost_cache_write");
    let cost_cache_read = legacy_compatible_archive_select_expr(archive_columns, "cost_cache_read");
    let cost_output = legacy_compatible_archive_select_expr(archive_columns, "cost_output");
    let cost_reasoning = legacy_compatible_archive_select_expr(archive_columns, "cost_reasoning");
    let first_token_ms = legacy_compatible_archive_select_expr(archive_columns, "first_token_ms");
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
            {first_token_ms},
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

/// Summary Snapshot V2 has a chronological proof contract, so its archive pager must use the
/// same deterministic key as the proof validator. Keep the generic historical-rollup pager
/// above ID-ordered because its checkpoint schema and replay semantics are independent.
pub(crate) fn build_invocation_archive_rows_time_chunk_query(
    archive_columns: &HashSet<String>,
) -> String {
    build_invocation_archive_rows_chunk_query(archive_columns).replace(
        "WHERE id > ?1\n        ORDER BY id ASC\n        LIMIT ?2",
        "WHERE julianday(occurred_at) IS NOT NULL\n          AND (?1 IS NULL\n               OR julianday(occurred_at) > julianday(?1)\n               OR (julianday(occurred_at) = julianday(?1) AND id > ?2))\n        ORDER BY julianday(occurred_at) ASC, id ASC\n        LIMIT ?3",
    )
}

pub(crate) async fn load_invocation_archive_rows_time_chunk(
    archive_pool: &Pool<Sqlite>,
    query_sql: &str,
    cursor_occurred_at: Option<&str>,
    cursor_id: i64,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(query_sql)
        .bind(cursor_occurred_at)
        .bind(cursor_id.max(0))
        .bind(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE + 1)
        .fetch_all(archive_pool)
        .await?;
    let has_more = rows.len() > HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize;
    if has_more {
        rows.truncate(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn load_invocation_archive_rows_chunk(
    archive_pool: &Pool<Sqlite>,
    query_sql: &str,
    start_after_id: i64,
) -> Result<(Vec<InvocationHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(query_sql)
        .bind(start_after_id)
        .bind(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE + 1)
        .fetch_all(archive_pool)
        .await?;
    let has_more = rows.len() > HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize;
    if has_more {
        rows.truncate(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn load_forward_proxy_archive_rows_chunk(
    archive_pool: &Pool<Sqlite>,
    start_after_id: i64,
) -> Result<(Vec<ForwardProxyAttemptHourlySourceRecord>, bool)> {
    let mut rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
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
    .bind(start_after_id)
    .bind(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE + 1)
    .fetch_all(archive_pool)
    .await?;
    let has_more = rows.len() > HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize;
    if has_more {
        rows.truncate(HISTORICAL_ROLLUP_ARCHIVE_REPLAY_BATCH_SIZE as usize);
    }
    Ok((rows, has_more))
}

pub(crate) async fn load_archive_coverage_bounds(
    archive_pool: &Pool<Sqlite>,
    table_name: &str,
) -> Result<HistoricalRollupArchiveCoverageBoundsRow> {
    Ok(
        sqlx::query_as::<_, HistoricalRollupArchiveCoverageBoundsRow>(&format!(
            r#"
        SELECT
            MIN(occurred_at) AS coverage_start_at,
            MAX(occurred_at) AS coverage_end_at
        FROM {table_name}
        "#
        ))
        .fetch_one(archive_pool)
        .await?,
    )
}

pub(crate) async fn invocation_archive_has_pruned_success_details_in_db(
    archive_pool: &Pool<Sqlite>,
) -> Result<bool> {
    let success_like_sql = invocation_status_is_success_like_sql("status", "error_message");
    let query = format!(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM codex_invocations
            WHERE detail_level != ?1
              AND {success_like_sql}
              AND COALESCE(NULLIF(LOWER(TRIM(COALESCE(failure_class, ''))), ''), 'none') = 'none'
            LIMIT 1
        )
        "#
    );
    let exists = sqlx::query_scalar::<_, i64>(&query)
        .bind(DETAIL_LEVEL_FULL)
        .fetch_one(archive_pool)
        .await?;
    Ok(exists != 0)
}

pub(crate) async fn replay_invocation_archive_rows_into_hourly_rollups_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    pending_targets: &[&str],
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupArchiveReplayResult> {
    let mut start_after_id = initial_cursor_id.max(0);
    let archive_columns = load_sqlite_table_columns(archive_pool, "codex_invocations").await?;
    let query_sql = build_invocation_archive_rows_chunk_query(&archive_columns);
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }

        let (rows, has_more) =
            load_invocation_archive_rows_chunk(archive_pool, &query_sql, start_after_id).await?;
        if rows.is_empty() {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        upsert_invocation_hourly_rollups_tx(tx, &rows, pending_targets).await?;
        mark_invocation_hourly_rollup_buckets_materialized_tx(tx, &rows).await?;
        start_after_id = rows
            .last()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("missing invocation archive row id"))?;

        if !has_more {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }
    }
}

async fn replay_invocation_archive_rows_into_hourly_rollups_until_cursor_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    target_cursor_id: i64,
    pending_targets: &[&str],
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupArchiveReplayResult> {
    let mut start_after_id = initial_cursor_id.max(0);
    let target_cursor_id = target_cursor_id.max(0);
    if start_after_id >= target_cursor_id {
        return Ok(HistoricalRollupArchiveReplayResult {
            outcome: HistoricalRollupArchiveReplayOutcome::Completed,
            cursor_id: start_after_id,
        });
    }

    let archive_columns = load_sqlite_table_columns(archive_pool, "codex_invocations").await?;
    let query_sql = build_invocation_archive_rows_chunk_query(&archive_columns);
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }

        let (rows, has_more) =
            load_invocation_archive_rows_chunk(archive_pool, &query_sql, start_after_id).await?;
        let bounded_rows = rows
            .into_iter()
            .take_while(|row| row.id <= target_cursor_id)
            .collect::<Vec<_>>();
        if bounded_rows.is_empty() {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        upsert_invocation_hourly_rollups_tx(tx, &bounded_rows, pending_targets).await?;
        mark_invocation_hourly_rollup_buckets_materialized_tx(tx, &bounded_rows).await?;
        start_after_id = bounded_rows
            .last()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("missing invocation archive row id"))?;

        if start_after_id >= target_cursor_id || !has_more {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }
    }
}

pub(crate) async fn replay_forward_proxy_archive_rows_into_hourly_rollups_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    initial_cursor_id: i64,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<HistoricalRollupArchiveReplayResult> {
    let mut start_after_id = initial_cursor_id.max(0);
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }

        let (rows, has_more) =
            load_forward_proxy_archive_rows_chunk(archive_pool, start_after_id).await?;
        if rows.is_empty() {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &rows).await?;
        mark_forward_proxy_hourly_rollup_buckets_materialized_tx(tx, &rows).await?;
        start_after_id = rows
            .last()
            .map(|row| row.id)
            .ok_or_else(|| anyhow!("missing forward proxy archive row id"))?;

        if !has_more {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                cursor_id: start_after_id,
            });
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok(HistoricalRollupArchiveReplayResult {
                outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                cursor_id: start_after_id,
            });
        }
    }
}

pub(crate) async fn replay_pool_upstream_node_health_archive_rows_tx_with_budget(
    tx: &mut SqliteConnection,
    archive_pool: &Pool<Sqlite>,
    archive_file_path: &str,
    initial_cursor_id: i64,
    started_at: Instant,
    max_elapsed: Option<Duration>,
) -> Result<(HistoricalRollupArchiveReplayResult, u64)> {
    let mut start_after_id = initial_cursor_id.max(0);
    let mut cached_rows = 0_u64;
    loop {
        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }

        let (rows, has_more) =
            load_pool_upstream_node_health_archive_rows_chunk(archive_pool, start_after_id).await?;
        if rows.is_empty() {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }

        upsert_pool_upstream_node_health_archive_rows_tx(tx, archive_file_path, &rows).await?;
        cached_rows += rows.len() as u64;
        start_after_id = rows
            .last()
            .map(|row| row.archived_row_id)
            .ok_or_else(|| anyhow!("missing pool upstream node health archive row id"))?;

        if !has_more {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::Completed,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }

        if historical_rollup_elapsed_budget_reached(started_at, max_elapsed) {
            return Ok((
                HistoricalRollupArchiveReplayResult {
                    outcome: HistoricalRollupArchiveReplayOutcome::HitBudget,
                    cursor_id: start_after_id,
                },
                cached_rows,
            ));
        }
    }
}
