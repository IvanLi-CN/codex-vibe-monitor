use super::*;

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
