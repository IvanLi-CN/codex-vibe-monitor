pub(crate) async fn rebuild_invocation_summary_rollups_from_live_rows(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    seen_ids: &mut HashSet<i64>,
    targets: &[&str],
    start_after_id: i64,
) -> Result<i64> {
    let mut cursor_id = start_after_id;
    loop {
        let mut rows = load_live_invocation_hourly_source_rows_after_id(
            &mut *tx,
            cursor_id,
            source_scope,
            BACKFILL_BATCH_SIZE,
        )
        .await?;
        if rows.is_empty() {
            break;
        }
        cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
        rows.retain(|row| seen_ids.insert(row.id));
        if rows.is_empty() {
            continue;
        }
        upsert_invocation_hourly_rollups_tx(tx, &rows, targets).await?;
    }
    Ok(cursor_id)
}

pub(crate) async fn mark_materialized_invocation_summary_archive_replayed_tx(
    tx: &mut SqliteConnection,
    archive_row: &ArchiveBatchPathRow,
) -> Result<()> {
    for target in INVOCATION_SUMMARY_ROLLUP_TARGETS {
        mark_hourly_rollup_archive_replayed_tx(
            tx,
            target,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            &archive_row.file_path,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn hourly_rollup_progress_exists(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    dataset: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM hourly_rollup_live_progress WHERE dataset = ?1 LIMIT 1",
    )
    .bind(dataset)
    .fetch_optional(executor)
    .await?
    .is_some())
}

pub(crate) async fn invocation_summary_repair_live_cursor_state(
    pool: &Pool<Sqlite>,
) -> Result<(bool, bool, i64, i64)> {
    let repair_marker_done =
        load_hourly_rollup_live_progress(pool, INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
            .await?
            >= INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE;
    let repair_live_cursor_exists = hourly_rollup_progress_exists(
        pool,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
    )
    .await?;
    let shared_live_cursor =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_live_cursor = if repair_live_cursor_exists {
        load_hourly_rollup_live_progress(
            pool,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        )
        .await?
    } else {
        0
    };
    Ok((
        repair_marker_done,
        repair_live_cursor_exists,
        shared_live_cursor,
        repair_live_cursor,
    ))
}

pub(crate) async fn invocation_summary_repair_live_cursor_state_tx(
    tx: &mut SqliteConnection,
) -> Result<(bool, bool, i64, i64)> {
    let repair_marker_done =
        load_hourly_rollup_live_progress_tx(tx, INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET)
            .await?
            >= INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE;
    let repair_live_cursor_exists = hourly_rollup_progress_exists(
        &mut *tx,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
    )
    .await?;
    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_live_cursor = if repair_live_cursor_exists {
        load_hourly_rollup_live_progress_tx(
            tx,
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        )
        .await?
    } else {
        0
    };
    Ok((
        repair_marker_done,
        repair_live_cursor_exists,
        shared_live_cursor,
        repair_live_cursor,
    ))
}

pub(crate) async fn repair_invocation_summary_rollups(pool: &Pool<Sqlite>) -> Result<()> {
    let (repair_marker_done, repair_live_cursor_exists, shared_live_cursor, repair_live_cursor) =
        invocation_summary_repair_live_cursor_state(pool).await?;
    if repair_marker_done && repair_live_cursor_exists && repair_live_cursor >= shared_live_cursor {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let (repair_marker_done, repair_live_cursor_exists, shared_live_cursor, repair_live_cursor) =
        invocation_summary_repair_live_cursor_state_tx(tx.as_mut()).await?;
    if repair_marker_done && repair_live_cursor_exists && repair_live_cursor >= shared_live_cursor {
        tx.rollback().await?;
        return Ok(());
    }
    if repair_marker_done && repair_live_cursor_exists && repair_live_cursor < shared_live_cursor {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
            shared_live_cursor,
        )
        .await?;
        tx.commit().await?;
        return Ok(());
    }

    let archive_rows = load_completed_invocation_archive_paths(tx.as_mut()).await?;
    let preserve_materialized_archives = archive_rows.iter().any(|archive_row| {
        archive_row.historical_rollups_materialized_at.is_some()
            && !PathBuf::from(&archive_row.file_path).exists()
    });
    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;

    if !preserve_materialized_archives {
        sqlx::query("DELETE FROM invocation_rollup_hourly")
            .execute(tx.as_mut())
            .await?;
        sqlx::query("DELETE FROM invocation_failure_rollup_hourly")
            .execute(tx.as_mut())
            .await?;
    }

    let mut seen_ids = HashSet::new();
    let mut cleared_rollup_buckets = ClearedSummaryRollupBuckets::default();
    for archive_row in &archive_rows {
        let preserve_materialized_archive =
            archive_row.historical_rollups_materialized_at.is_some()
                && !PathBuf::from(&archive_row.file_path).exists();
        if preserve_materialized_archive {
            mark_materialized_invocation_summary_archive_replayed_tx(tx.as_mut(), archive_row)
                .await?;
            continue;
        }
        rebuild_invocation_summary_rollups_from_archive_batch(
            tx.as_mut(),
            archive_row,
            InvocationSourceScope::All,
            &mut seen_ids,
            &mut cleared_rollup_buckets,
            &INVOCATION_SUMMARY_ROLLUP_TARGETS,
            preserve_materialized_archives,
        )
        .await?;
    }
    let mut restored_live_rows = load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
        tx.as_mut(),
        &cleared_rollup_buckets.overall,
        InvocationSourceScope::All,
        shared_live_cursor,
    )
    .await?;
    restored_live_rows.retain(|row| !seen_ids.contains(&row.id));
    if !restored_live_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &restored_live_rows,
            &INVOCATION_SUMMARY_ROLLUP_TARGETS,
        )
        .await?;
    }
    let live_cursor_id = rebuild_invocation_summary_rollups_from_live_rows(
        tx.as_mut(),
        InvocationSourceScope::All,
        &mut seen_ids,
        &INVOCATION_SUMMARY_ROLLUP_TARGETS,
        if preserve_materialized_archives {
            shared_live_cursor
        } else {
            0
        },
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
        live_cursor_id.max(shared_live_cursor),
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DATASET,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_DONE,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn backfill_missing_invocation_summary_archive_rollups(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let archive_rows = load_invocation_archives_missing_summary_rollup_markers(pool).await?;
    if archive_rows.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let archive_rows = load_invocation_archives_missing_summary_rollup_markers(tx.as_mut()).await?;
    if archive_rows.is_empty() {
        tx.rollback().await?;
        return Ok(());
    }

    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let mut seen_ids = HashSet::new();
    let mut cleared_rollup_buckets = ClearedSummaryRollupBuckets::default();
    for archive_row in &archive_rows {
        let needs_overall = archive_row.needs_overall.unwrap_or_default() != 0;
        let needs_failures = archive_row.needs_failures.unwrap_or_default() != 0;
        let mut targets = Vec::new();
        if needs_overall {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATIONS);
        }
        if needs_failures {
            targets.push(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES);
        }
        if targets.is_empty() {
            continue;
        }
        let archive_path = PathBuf::from(&archive_row.file_path);
        if archive_row.historical_rollups_materialized_at.is_some() && !archive_path.exists() {
            for target in &targets {
                mark_hourly_rollup_archive_replayed_tx(
                    tx.as_mut(),
                    target,
                    HOURLY_ROLLUP_DATASET_INVOCATIONS,
                    &archive_row.file_path,
                )
                .await?;
            }
            continue;
        }
        rebuild_invocation_summary_rollups_from_archive_batch(
            tx.as_mut(),
            archive_row,
            InvocationSourceScope::All,
            &mut seen_ids,
            &mut cleared_rollup_buckets,
            &targets,
            true,
        )
        .await?;
    }
    let mut restored_overall_live_rows =
        load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
            tx.as_mut(),
            &cleared_rollup_buckets.overall,
            InvocationSourceScope::All,
            shared_live_cursor,
        )
        .await?;
    restored_overall_live_rows.retain(|row| !seen_ids.contains(&row.id));
    if !restored_overall_live_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &restored_overall_live_rows,
            &[HOURLY_ROLLUP_TARGET_INVOCATIONS],
        )
        .await?;
    }
    let mut restored_failure_live_rows =
        load_live_invocation_summary_rows_for_cleared_buckets_up_to_id(
            tx.as_mut(),
            &cleared_rollup_buckets.failures,
            InvocationSourceScope::All,
            shared_live_cursor,
        )
        .await?;
    restored_failure_live_rows.retain(|row| !seen_ids.contains(&row.id));
    if !restored_failure_live_rows.is_empty() {
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &restored_failure_live_rows,
            &[HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn ensure_invocation_summary_rollups_ready(pool: &Pool<Sqlite>) -> Result<()> {
    if load_completed_invocation_archive_paths(pool)
        .await?
        .is_empty()
    {
        return Ok(());
    }

    repair_invocation_summary_rollups(pool).await?;
    backfill_missing_invocation_summary_archive_rollups(pool).await?;
    Ok(())
}

pub(crate) async fn ensure_invocation_summary_rollups_ready_best_effort(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    match ensure_invocation_summary_rollups_ready(pool).await {
        Ok(()) => Ok(()),
        Err(err) if is_missing_invocation_summary_archive_error(&err) => {
            warn!(
                error = %err,
                "skipping invocation summary rollup repair because an archive batch file is missing; reusing current rollups for historical range queries"
            );
            Ok(())
        }
        Err(err) if is_unreadable_invocation_summary_archive_error(&err) => {
            warn!(
                error = %err,
                "skipping invocation summary rollup repair because an archive batch is unreadable; reusing current rollups for historical range queries"
            );
            Ok(())
        }
        Err(err) => Err(err),
    }
}

pub(crate) struct AllTimeRollupTotals {
    totals: StatsTotals,
    live_tail_ids: HashSet<i64>,
}

pub(crate) async fn query_invocation_all_time_rollup_totals(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<AllTimeRollupTotals> {
    let non_success_cost_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "non_success_cost").await? {
            "COALESCE(SUM(non_success_cost), 0.0) AS non_success_cost"
        } else {
            "0.0 AS non_success_cost"
        };
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT
            COALESCE(SUM(total_count), 0) AS total_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_cost), 0.0) AS total_cost,
            COALESCE(SUM(total_tokens), 0) AS total_tokens,
            {non_success_cost_expr}
        FROM invocation_rollup_hourly
        WHERE 1 = 1
        "#,
    ));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    let mut totals = StatsTotals::from(query.build_query_as::<StatsRow>().fetch_one(pool).await?);
    let live_progress_cursor =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_live_cursor = load_hourly_rollup_live_progress(
        pool,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
    )
    .await?;
    let tail_cursor = live_progress_cursor.max(repair_live_cursor);
    // Without a durable live cursor, the rollup may already include an unknown
    // prefix. Do not add a raw tail and risk double-counting; retention advances
    // this cursor atomically only after it materializes a contiguous prefix.
    if tail_cursor <= 0 {
        return Ok(AllTimeRollupTotals {
            totals,
            live_tail_ids: HashSet::new(),
        });
    }

    let tail_query = match source_scope {
        InvocationSourceScope::ProxyOnly => format!(
            "SELECT {} FROM codex_invocations WHERE id > ?1 AND source = ?2",
            stats_success_failure_select_sql()
        ),
        InvocationSourceScope::All => format!(
            "SELECT {} FROM codex_invocations WHERE id > ?1",
            stats_success_failure_select_sql()
        ),
    };
    let tail = match source_scope {
        InvocationSourceScope::ProxyOnly => {
            sqlx::query_as::<_, StatsRow>(&tail_query)
                .bind(tail_cursor)
                .bind(SOURCE_PROXY)
                .fetch_one(pool)
                .await?
        }
        InvocationSourceScope::All => {
            sqlx::query_as::<_, StatsRow>(&tail_query)
                .bind(tail_cursor)
                .fetch_one(pool)
                .await?
        }
    };
    totals = totals.add(StatsTotals::from(tail));
    let live_tail_ids = load_live_invocation_ids_after_id(pool, source_scope, tail_cursor).await?;
    Ok(AllTimeRollupTotals {
        totals,
        live_tail_ids,
    })
}

pub(crate) async fn query_invocation_totals(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    if matches!(filter, StatsFilter::All) {
        if load_completed_invocation_archive_paths(pool)
            .await?
            .is_empty()
        {
            return Ok(StatsTotals::from(
                query_stats_row(pool, StatsFilter::All, source_scope).await?,
            ));
        }

        // Read paths must stay query-only even when historical summary repair is still pending.
        // Background startup / follow-up maintenance is responsible for rebuilding stale archived
        // hourly rollups and summary replay markers; requests reuse the current materialized
        // rollups plus any still-unmaterialized archive batches instead of writing through here.
        let all_time = query_invocation_all_time_rollup_totals(pool, source_scope).await?;
        return Ok(all_time.totals.add(
            query_unmaterialized_invocation_archive_totals(
                pool,
                source_scope,
                None,
                Some(&all_time.live_tail_ids),
            )
            .await?,
        ));
    }

    Ok(StatsTotals::from(
        query_stats_row(pool, filter, source_scope).await?,
    ))
}

/// Aggregate the bounded live tail which follows the durable hourly-rollup cursor. This keeps
/// all-time projection hydration exact when those rows are older than the retained exact horizon,
/// without materializing the rows themselves into the canonical projection.
pub(crate) async fn query_live_invocation_totals_after_id(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    start_after_id: i64,
    upper_bound_id: i64,
) -> Result<StatsTotals> {
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query.push(stats_success_failure_select_sql());
    query
        .push(" FROM codex_invocations WHERE id > ")
        .push_bind(start_after_id)
        .push(" AND id <= ")
        .push_bind(upper_bound_id);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    let row = query.build_query_as::<StatsRow>().fetch_one(pool).await?;
    Ok(StatsTotals::from(row))
}

pub(crate) async fn query_invocation_hourly_rollup_range(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationHourlyRollupRecord>> {
    let input_tokens_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "input_tokens").await? {
            "COALESCE(input_tokens, 0) AS input_tokens"
        } else {
            "0 AS input_tokens"
        };
    let output_tokens_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "output_tokens").await? {
            "COALESCE(output_tokens, 0) AS output_tokens"
        } else {
            "0 AS output_tokens"
        };
    let cache_input_tokens_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "cache_input_tokens").await? {
            "COALESCE(cache_input_tokens, 0) AS cache_input_tokens"
        } else {
            "0 AS cache_input_tokens"
        };
    let reasoning_tokens_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "reasoning_tokens").await? {
            "COALESCE(reasoning_tokens, 0) AS reasoning_tokens"
        } else {
            "0 AS reasoning_tokens"
        };
    let non_success_cost_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "non_success_cost").await? {
            "COALESCE(non_success_cost, 0.0) AS non_success_cost"
        } else {
            "0.0 AS non_success_cost"
        };
    let total_latency_sample_count_expr = if sqlite_table_has_column(
        pool,
        "invocation_rollup_hourly",
        "total_latency_sample_count",
    )
    .await?
    {
        "COALESCE(total_latency_sample_count, 0) AS total_latency_sample_count"
    } else {
        "0 AS total_latency_sample_count"
    };
    let total_latency_sum_ms_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "total_latency_sum_ms").await?
        {
            "COALESCE(total_latency_sum_ms, 0.0) AS total_latency_sum_ms"
        } else {
            "0.0 AS total_latency_sum_ms"
        };
    let has_first_token_rollup =
        sqlite_table_has_column(pool, "invocation_rollup_hourly", "first_token_sample_count")
            .await?;
    let first_token_sample_count_expr = if has_first_token_rollup {
        "COALESCE(first_token_sample_count, 0) AS first_token_sample_count"
    } else {
        "0 AS first_token_sample_count"
    };
    let first_token_sum_ms_expr = if has_first_token_rollup {
        "COALESCE(first_token_sum_ms, 0.0) AS first_token_sum_ms"
    } else {
        "0.0 AS first_token_sum_ms"
    };
    let first_token_max_ms_expr = if has_first_token_rollup {
        "COALESCE(first_token_max_ms, 0.0) AS first_token_max_ms"
    } else {
        "0.0 AS first_token_max_ms"
    };
    let first_token_histogram_expr = if has_first_token_rollup {
        "COALESCE(first_token_histogram, '[]') AS first_token_histogram"
    } else {
        "'[]' AS first_token_histogram"
    };
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT
            bucket_start_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            {input_tokens_expr},
            {output_tokens_expr},
            {cache_input_tokens_expr},
            {reasoning_tokens_expr},
            total_cost,
            {non_success_cost_expr},
            {total_latency_sample_count_expr},
            {total_latency_sum_ms_expr},
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram,
            {first_token_sample_count_expr},
            {first_token_sum_ms_expr},
            {first_token_max_ms_expr},
            {first_token_histogram_expr}
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >=
        "#,
    ));
    query.push_bind(range_start_epoch);
    query
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY bucket_start_epoch ASC");

    query
        .build_query_as::<InvocationHourlyRollupRecord>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn sqlite_table_has_column(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
) -> Result<bool> {
    let escaped_table_name = table_name.replace('\'', "''");
    let pragma = format!("PRAGMA table_info('{escaped_table_name}')");
    let rows = sqlx::query(&pragma).fetch_all(pool).await?;
    Ok(rows.into_iter().any(|row| {
        row.try_get::<String, _>("name")
            .is_ok_and(|name| name == column_name)
    }))
}

pub(crate) async fn query_invocation_failure_hourly_rollup_range(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationFailureHourlyRollupRecord>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            failure_class,
            is_actionable,
            error_category,
            SUM(failure_count) AS failure_count
        FROM invocation_failure_rollup_hourly
        WHERE bucket_start_epoch >=
        "#,
    );
    query.push_bind(range_start_epoch);
    query
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY failure_class, is_actionable, error_category");

    query
        .build_query_as::<InvocationFailureHourlyRollupRecord>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_proxy_perf_stage_hourly_rollup_range(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<Vec<ProxyPerfStageHourlyRollupRecord>> {
    sqlx::query_as::<_, ProxyPerfStageHourlyRollupRecord>(
        r#"
        SELECT
            bucket_start_epoch,
            stage,
            sample_count,
            sum_ms,
            max_ms,
            histogram
        FROM proxy_perf_stage_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
        ORDER BY stage ASC, bucket_start_epoch ASC
        "#,
    )
    .bind(range_start_epoch)
    .bind(range_end_epoch)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn query_combined_totals(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    query_invocation_totals(pool, filter, source_scope).await
}

pub(crate) async fn resolve_default_source_scope(
    _pool: &Pool<Sqlite>,
) -> Result<InvocationSourceScope> {
    Ok(InvocationSourceScope::All)
}

#[derive(Debug)]
pub(crate) enum ApiError {
    BadRequest(anyhow::Error),
    Unavailable(anyhow::Error),
    Internal(anyhow::Error),
}

impl ApiError {
    pub(crate) fn bad_request<E>(err: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::BadRequest(err.into())
    }

    pub(crate) fn unavailable<E>(err: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::Unavailable(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, err) = match self {
            ApiError::BadRequest(err) => (StatusCode::BAD_REQUEST, err),
            ApiError::Unavailable(err) => (StatusCode::SERVICE_UNAVAILABLE, err),
            ApiError::Internal(err) => (StatusCode::INTERNAL_SERVER_ERROR, err),
        };
        let message = format!("{err}");
        (status, message).into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

// --- ISO8601 UTC helpers and serializers ---
pub(crate) fn format_utc_iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn format_utc_iso_millis(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn format_utc_iso_precise(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub(crate) fn parse_to_utc_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        if let Some(loc) = Shanghai.from_local_datetime(&naive).single() {
            return Some(loc.with_timezone(&Utc));
        }
        return Some(Utc.from_utc_datetime(&naive));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        if let Some(loc) = Shanghai.from_local_datetime(&naive).single() {
            return Some(loc.with_timezone(&Utc));
        }
        return Some(Utc.from_utc_datetime(&naive));
    }
    None
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_local_naive_to_utc_iso<S>(
    value: &String,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let iso = parse_to_utc_datetime(value)
        .map(format_utc_iso)
        .unwrap_or_else(|| value.clone());
    serializer.serialize_str(&iso)
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_local_or_utc_to_utc_iso<S>(
    value: &String,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serialize_local_naive_to_utc_iso(value, serializer)
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_opt_local_or_utc_to_utc_iso<S>(
    value: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(s) => serialize_local_naive_to_utc_iso(s, serializer),
        None => serializer.serialize_none(),
    }
}
