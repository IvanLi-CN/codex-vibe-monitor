#[derive(Debug, FromRow)]
struct SummaryProjectionArchiveRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    source: String,
    model: Option<String>,
    response_model: Option<String>,
    input_tokens: i64,
    output_tokens: i64,
    cache_input_tokens: i64,
    reasoning_tokens: i64,
    reasoning_effort: Option<String>,
    total_tokens: i64,
    cost: Option<f64>,
    cost_input: Option<f64>,
    cost_cache_write: Option<f64>,
    cost_cache_read: Option<f64>,
    cost_output: Option<f64>,
    cost_reasoning: Option<f64>,
    status: String,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    upstream_account_id: Option<i64>,
}

#[derive(Debug, FromRow)]
struct SummaryProjectionRollupRow {
    bucket_start_epoch: i64,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    non_success_cost: f64,
    non_success_tokens: i64,
}

#[derive(Debug, FromRow)]
struct SummaryProjectionAccountTotalsRow {
    upstream_account_id: i64,
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_cost: f64,
    total_tokens: i64,
    non_success_cost: f64,
}

impl SummaryProjectionAccountTotalsRow {
    fn into_totals(self) -> StatsTotals {
        StatsTotals {
            total_count: self.total_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_cost: self.total_cost,
            total_tokens: self.total_tokens,
            non_success_cost: self.non_success_cost,
        }
    }
}

async fn load_summary_projection_account_rollup_totals(
    pool: &Pool<Sqlite>,
    exact_replacement_buckets: &HashSet<i64>,
) -> Result<HashMap<i64, StatsTotals>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT upstream_account_id, COALESCE(SUM(total_count), 0) AS total_count, \
                COALESCE(SUM(success_count), 0) AS success_count, \
                COALESCE(SUM(failure_count), 0) AS failure_count, \
                COALESCE(SUM(total_cost), 0.0) AS total_cost, \
                COALESCE(SUM(total_tokens), 0) AS total_tokens, \
                COALESCE(SUM(non_success_cost), 0.0) AS non_success_cost \
         FROM upstream_account_stats_hourly \
         WHERE upstream_account_id > 0",
    );
    if !exact_replacement_buckets.is_empty() {
        query.push(" AND bucket_start_epoch NOT IN (");
        let mut separated = query.separated(", ");
        for bucket in exact_replacement_buckets {
            separated.push_bind(*bucket);
        }
        separated.push_unseparated(")");
    }
    query
        .push(" GROUP BY upstream_account_id LIMIT ")
        .push_bind((SUMMARY_PROJECTION_MAX_ACCOUNTS + 1) as i64);
    let rows = query
        .build_query_as::<SummaryProjectionAccountTotalsRow>()
        .fetch_all(pool)
        .await
        .context("summary projection all-time account rollup hydration failed")?;
    if rows.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
        return Err(anyhow!(
            "summary projection account rollup cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ACCOUNTS})"
        ));
    }
    Ok(rows
        .into_iter()
        .map(|row| (row.upstream_account_id, row.into_totals()))
        .collect())
}

async fn load_summary_projection_live_tail_account_totals(
    pool: &Pool<Sqlite>,
    account_rollup_live_cursor: Option<i64>,
    upper_bound_id: Option<i64>,
) -> Result<HashMap<i64, StatsTotals>> {
    // Account activity has its own durable cursor. A missing marker means that no account
    // prefix is proven to be represented by the account rollup, so archive-backed all-time
    // hydration must not silently add the entire live table as a second source.
    let (Some(account_rollup_live_cursor), Some(upper_bound_id)) =
        (account_rollup_live_cursor, upper_bound_id)
    else {
        return Ok(HashMap::new());
    };
    let account_expression = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    let query = format!(
        "SELECT {account_expression} AS upstream_account_id, {} \
         FROM codex_invocations \
         WHERE id > ?1 AND id <= ?2 AND {account_expression} > 0 \
         GROUP BY upstream_account_id \
         LIMIT ?3",
        crate::stats::stats_success_failure_select_sql(),
    );
    let rows = sqlx::query_as::<_, SummaryProjectionAccountTotalsRow>(&query)
        .bind(account_rollup_live_cursor)
        .bind(upper_bound_id)
        .bind((SUMMARY_PROJECTION_MAX_ACCOUNTS + 1) as i64)
        .fetch_all(pool)
        .await
        .context("summary projection live-tail account hydration failed")?;
    if rows.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
        return Err(anyhow!(
            "summary projection live-tail account cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ACCOUNTS})"
        ));
    }
    Ok(rows
        .into_iter()
        .map(|row| (row.upstream_account_id, row.into_totals()))
        .collect())
}

async fn admit_summary_projection_live_tail_account_ids(
    pool: &Pool<Sqlite>,
    account_rollup_live_cursor: Option<i64>,
) -> Result<Option<crate::stats::BoundedLiveInvocationIds>> {
    let Some(account_rollup_live_cursor) = account_rollup_live_cursor else {
        return Ok(None);
    };
    crate::stats::load_live_invocation_ids_after_id_bounded_snapshot(
        pool,
        InvocationSourceScope::All,
        account_rollup_live_cursor,
        summary_projection_exact_record_limit(),
    )
    .await
    .map(Some)
    .map_err(|error| anyhow!("summary projection account live-tail id hydration failed: {error:?}"))
}

#[cfg(test)]
pub(crate) async fn admit_summary_projection_live_tail_account_ids_for_test(
    pool: &Pool<Sqlite>,
    account_rollup_live_cursor: Option<i64>,
) -> Result<Option<crate::stats::BoundedLiveInvocationIds>> {
    admit_summary_projection_live_tail_account_ids(pool, account_rollup_live_cursor).await
}

async fn load_summary_projection_live_account_totals(
    pool: &Pool<Sqlite>,
    upper_bound_id: i64,
) -> Result<HashMap<i64, StatsTotals>> {
    let account_expression = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    let query = format!(
        "SELECT {account_expression} AS upstream_account_id, {} \
         FROM codex_invocations \
         WHERE id <= ?1 AND {account_expression} > 0 \
         GROUP BY upstream_account_id \
         LIMIT ?2",
        crate::stats::stats_success_failure_select_sql(),
    );
    let rows = sqlx::query_as::<_, SummaryProjectionAccountTotalsRow>(&query)
        .bind(upper_bound_id)
        .bind((SUMMARY_PROJECTION_MAX_ACCOUNTS + 1) as i64)
        .fetch_all(pool)
        .await
        .context("summary projection live account totals hydration failed")?;
    if rows.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
        return Err(anyhow!(
            "summary projection live account cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ACCOUNTS})"
        ));
    }
    Ok(rows
        .into_iter()
        .map(|row| (row.upstream_account_id, row.into_totals()))
        .collect())
}

async fn summary_projection_live_history_within_aggregate_budget(
    pool: &Pool<Sqlite>,
) -> Result<Option<crate::stats::BoundedLiveInvocationIds>> {
    summary_projection_live_history_within_aggregate_budget_with_limit(
        pool,
        SUMMARY_PROJECTION_MAX_LIVE_AGGREGATE_ROWS,
    )
    .await
}

async fn summary_projection_live_history_within_aggregate_budget_with_limit(
    pool: &Pool<Sqlite>,
    max_rows: usize,
) -> Result<Option<crate::stats::BoundedLiveInvocationIds>> {
    // Check the aggregate bound with SQLite's indexed count before allocating the candidate ID
    // set. Overflow only needs an unavailable proof; materializing `limit + 1` ids would make the
    // failed all-time path needlessly proportional to the retained live table.
    let live_row_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM codex_invocations WHERE id > ?1")
            .bind(0_i64)
            .fetch_one(pool)
            .await
            .context("summary projection live aggregate row count failed")?;
    if live_row_count > max_rows as i64 {
        return Ok(None);
    }
    match crate::stats::load_live_invocation_ids_after_id_bounded_snapshot(
        pool,
        InvocationSourceScope::All,
        0,
        max_rows,
    )
    .await
    {
        Ok(admission) => Ok(Some(admission)),
        Err(error)
            if error
                .to_string()
                .starts_with("summary live-tail id budget exceeded") =>
        {
            Ok(None)
        }
        Err(error) => Err(error.context("summary projection live aggregate admission failed")),
    }
}

async fn count_summary_projection_rollup_rows<'e, E>(
    executor: E,
    table_name: &'static str,
    range: Option<(i64, i64)>,
) -> Result<usize>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let mut query = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM ");
    // Callers pass only the two fixed rollup table names in this module. Keeping the table name
    // out of user-controlled SQL preserves the background-only hydration boundary.
    query.push(table_name);
    if let Some((start, end)) = range {
        query
            .push(" WHERE bucket_start_epoch >= ")
            .push_bind(start)
            .push(" AND bucket_start_epoch < ")
            .push_bind(end);
    }
    let count = query
        .build_query_scalar::<i64>()
        .fetch_one(executor)
        .await
        .with_context(|| format!("summary projection {table_name} row count failed"))?;
    let count = usize::try_from(count).context("summary projection rollup row count overflow")?;
    if count > SUMMARY_PROJECTION_MAX_ROLLUP_ROWS {
        return Err(anyhow!(
            "summary projection {table_name} row budget exceeded ({count} > {SUMMARY_PROJECTION_MAX_ROLLUP_ROWS})"
        ));
    }
    Ok(count)
}

async fn load_summary_projection_rollup_totals(
    pool: &Pool<Sqlite>,
) -> Result<(
    HashMap<(i64, Option<i64>), StatsTotals>,
    HashMap<(i64, Option<i64>), i64>,
)> {
    load_summary_projection_rollup_totals_in_range(pool, None).await
}

fn summary_projection_rollup_admission_exceeded(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.starts_with("summary projection ")
        && message.contains("rollup")
        && message.contains("budget exceeded")
}
