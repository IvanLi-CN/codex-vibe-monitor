use super::*;
#[derive(Debug, FromRow)]
pub(crate) struct SummaryProjectionArchiveRow {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) model: Option<String>,
    pub(crate) response_model: Option<String>,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) reasoning_tokens: i64,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) total_tokens: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_input: Option<f64>,
    pub(crate) cost_cache_write: Option<f64>,
    pub(crate) cost_cache_read: Option<f64>,
    pub(crate) cost_output: Option<f64>,
    pub(crate) cost_reasoning: Option<f64>,
    pub(crate) status: String,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct SummaryProjectionRollupRow {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) non_success_tokens: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct SummaryProjectionAccountTotalsRow {
    pub(crate) upstream_account_id: i64,
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_cost: f64,
    pub(crate) total_tokens: i64,
    pub(crate) non_success_cost: f64,
}

impl SummaryProjectionAccountTotalsRow {
    pub(crate) fn into_totals(self) -> StatsTotals {
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

pub(crate) async fn load_summary_projection_account_rollup_totals(
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

pub(crate) async fn load_summary_projection_live_tail_account_totals(
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

pub(crate) async fn admit_summary_projection_live_tail_account_ids(
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

pub(crate) async fn load_summary_projection_live_account_totals(
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

pub(crate) async fn summary_projection_live_history_within_aggregate_budget(
    pool: &Pool<Sqlite>,
) -> Result<Option<crate::stats::BoundedLiveInvocationIds>> {
    summary_projection_live_history_within_aggregate_budget_with_limit(
        pool,
        SUMMARY_PROJECTION_MAX_LIVE_AGGREGATE_ROWS,
    )
    .await
}

pub(crate) async fn summary_projection_live_history_within_aggregate_budget_with_limit(
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

pub(crate) async fn count_summary_projection_rollup_rows<'e, E>(
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

pub(crate) async fn load_summary_projection_rollup_totals(
    pool: &Pool<Sqlite>,
) -> Result<(
    HashMap<(i64, Option<i64>), StatsTotals>,
    HashMap<(i64, Option<i64>), i64>,
)> {
    load_summary_projection_rollup_totals_in_range(pool, None).await
}

pub(crate) fn summary_projection_rollup_admission_exceeded(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.starts_with("summary projection ")
        && message.contains("rollup")
        && message.contains("budget exceeded")
}

pub(crate) async fn load_summary_projection_rollup_totals_in_range(
    pool: &Pool<Sqlite>,
    range: Option<(i64, i64)>,
) -> Result<(
    HashMap<(i64, Option<i64>), StatsTotals>,
    HashMap<(i64, Option<i64>), i64>,
)> {
    count_summary_projection_rollup_rows(pool, "invocation_rollup_hourly", range).await?;
    count_summary_projection_rollup_rows(pool, "upstream_account_stats_hourly", range).await?;
    let mut totals = HashMap::<(i64, Option<i64>), StatsTotals>::new();
    let mut non_success_tokens = HashMap::<(i64, Option<i64>), i64>::new();
    let mut rollup_bytes = 0usize;
    let mut overall_query = QueryBuilder::<Sqlite>::new(
        "SELECT bucket_start_epoch, total_count, success_count, failure_count, total_tokens, \
         total_cost, COALESCE(non_success_cost, 0.0) AS non_success_cost, 0 AS non_success_tokens \
         FROM invocation_rollup_hourly",
    );
    if let Some((start, end)) = range {
        overall_query
            .push(" WHERE bucket_start_epoch >= ")
            .push_bind(start)
            .push(" AND bucket_start_epoch < ")
            .push_bind(end);
    }
    let overall_rows = overall_query
        .build_query_as::<SummaryProjectionRollupRow>()
        .fetch_all(pool)
        .await
        .context("summary projection overall rollup hydration failed")?;
    for row in overall_rows {
        rollup_bytes = rollup_bytes.saturating_add(256);
        if rollup_bytes > SUMMARY_PROJECTION_MAX_ROLLUP_BYTES {
            return Err(anyhow!(
                "summary projection rollup memory budget exceeded ({SUMMARY_PROJECTION_MAX_ROLLUP_BYTES} bytes)"
            ));
        }
        let bucket_totals = StatsTotals {
            total_count: row.total_count,
            success_count: row.success_count,
            failure_count: row.failure_count,
            total_tokens: row.total_tokens,
            total_cost: row.total_cost,
            non_success_cost: row.non_success_cost,
        };
        let key = (row.bucket_start_epoch, None);
        let previous = totals.get(&key).copied().unwrap_or_default();
        totals.insert(key, previous.add(bucket_totals));
    }
    #[derive(Debug, FromRow)]
    struct AccountRollupRow {
        upstream_account_id: i64,
        bucket_start_epoch: i64,
        total_count: i64,
        success_count: i64,
        failure_count: i64,
        total_tokens: i64,
        total_cost: f64,
        non_success_cost: f64,
        non_success_tokens: i64,
    }
    let mut account_query = QueryBuilder::<Sqlite>::new(
        "SELECT upstream_account_id, bucket_start_epoch, total_count, success_count, \
         failure_count, total_tokens, total_cost, COALESCE(non_success_cost, 0.0) AS non_success_cost, \
         COALESCE(activity_v2_non_success_tokens, 0) AS non_success_tokens \
         FROM upstream_account_stats_hourly WHERE upstream_account_id <> 0",
    );
    if let Some((start, end)) = range {
        account_query
            .push(" AND bucket_start_epoch >= ")
            .push_bind(start)
            .push(" AND bucket_start_epoch < ")
            .push_bind(end);
    }
    let account_rows = account_query
        .build_query_as::<AccountRollupRow>()
        .fetch_all(pool)
        .await
        .context("summary projection account rollup hydration failed")?;
    for row in account_rows {
        rollup_bytes = rollup_bytes.saturating_add(256);
        if rollup_bytes > SUMMARY_PROJECTION_MAX_ROLLUP_BYTES {
            return Err(anyhow!(
                "summary projection rollup memory budget exceeded ({SUMMARY_PROJECTION_MAX_ROLLUP_BYTES} bytes)"
            ));
        }
        if row.upstream_account_id > 0 {
            let bucket_totals = StatsTotals {
                total_count: row.total_count,
                success_count: row.success_count,
                failure_count: row.failure_count,
                total_tokens: row.total_tokens,
                total_cost: row.total_cost,
                non_success_cost: row.non_success_cost,
            };
            let key = (row.bucket_start_epoch, Some(row.upstream_account_id));
            let previous = totals.get(&key).copied().unwrap_or_default();
            totals.insert(key, previous.add(bucket_totals));
            non_success_tokens.insert(
                key,
                non_success_tokens
                    .get(&key)
                    .copied()
                    .unwrap_or_default()
                    .saturating_add(row.non_success_tokens),
            );
        }
        *non_success_tokens
            .entry((row.bucket_start_epoch, None))
            .or_default() += row.non_success_tokens;
    }
    Ok((totals, non_success_tokens))
}

pub(crate) async fn load_summary_projection_rollup_usage(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<(i64, Option<i64>), UsageBreakdownResponse>> {
    load_summary_projection_rollup_usage_in_range(pool, None).await
}

pub(crate) async fn load_summary_projection_rollup_usage_in_range(
    pool: &Pool<Sqlite>,
    range: Option<(i64, i64)>,
) -> Result<HashMap<(i64, Option<i64>), UsageBreakdownResponse>> {
    let mut connection = pool.acquire().await?;
    count_summary_projection_rollup_rows(
        &mut *connection,
        "upstream_account_usage_breakdown_hourly",
        range,
    )
    .await?;
    let (start_epoch, end_epoch) = range.unwrap_or((i64::MIN, i64::MAX));
    let rows = query_upstream_account_usage_breakdown_hourly_rollup_range_bounded_tx(
        &mut connection,
        start_epoch,
        end_epoch,
        InvocationSourceScope::All,
        None,
        SUMMARY_PROJECTION_MAX_ROLLUP_BYTES,
    )
    .await
    .map_err(|error| anyhow!("summary projection usage rollup hydration failed: {error:?}"))?;

    let mut accumulators = HashMap::<(i64, Option<i64>), UsageBreakdownAccumulator>::new();
    let mut rollup_bytes = 0usize;
    for row in rows {
        rollup_bytes = rollup_bytes.saturating_add(
            256usize
                .saturating_add(row.model.len())
                .saturating_add(row.reasoning_effort.as_ref().map_or(0, String::len)),
        );
        if rollup_bytes > SUMMARY_PROJECTION_MAX_ROLLUP_BYTES {
            return Err(anyhow!(
                "summary projection usage rollup memory budget exceeded ({SUMMARY_PROJECTION_MAX_ROLLUP_BYTES} bytes)"
            ));
        }
        let bucket_start_epoch = row.bucket_start_epoch;
        let upstream_account_id = row.upstream_account_id;
        let aggregate = UpstreamAccountUsageBreakdownAggregateRow {
            upstream_account_id,
            model: row.model,
            reasoning_effort: row.reasoning_effort,
            request_count: row.request_count,
            success_count: row.success_count,
            failure_count: row.failure_count,
            cache_write_tokens: row.cache_write_tokens,
            cache_read_tokens: row.cache_read_tokens,
            output_tokens: row.output_tokens,
            cost_input: row.cost_input,
            cost_cache_write: row.cost_cache_write,
            cost_cache_read: row.cost_cache_read,
            cost_output: row.cost_output,
            cost_reasoning: row.cost_reasoning,
            cost_unknown: row.cost_unknown,
            has_cost: row.has_cost,
            performance_total_tokens: row.performance_total_tokens,
            performance_stream_output_tokens: row.performance_stream_output_tokens,
            performance_stream_duration_ms: row.performance_stream_duration_ms,
            performance_response_sample_count: row.performance_response_sample_count,
            performance_response_sum_ms: row.performance_response_sum_ms,
            performance_first_byte_sample_count: row.performance_first_byte_sample_count,
            performance_first_byte_sum_ms: row.performance_first_byte_sum_ms,
            performance_first_token_sample_count: row.performance_first_token_sample_count,
            performance_first_token_sum_ms: row.performance_first_token_sum_ms,
            performance_usage_duration_sample_count: row.performance_usage_duration_sample_count,
            performance_usage_duration_sum_ms: row.performance_usage_duration_sum_ms,
        };
        // The rollup table stores account-scoped rows, including a NULL account row for
        // account-less invocations.  The global projection must include every row exactly once;
        // a NULL row is already the global key and must not be inserted twice.
        accumulators
            .entry((bucket_start_epoch, None))
            .or_default()
            .add_aggregate_row(&aggregate);
        if let Some(account_id) = upstream_account_id {
            accumulators
                .entry((bucket_start_epoch, Some(account_id)))
                .or_default()
                .add_aggregate_row(&aggregate);
        }
    }
    Ok(accumulators
        .into_iter()
        .map(|(key, accumulator)| (key, accumulator.into_response()))
        .collect())
}

pub(crate) fn summary_projection_archive_column(
    column: &str,
    available: bool,
    fallback: &str,
) -> String {
    if available {
        format!("{column} AS {column}")
    } else {
        format!("{fallback} AS {column}")
    }
}

pub(crate) fn summary_projection_archive_overlap_range(
    archive: &crate::stats::ArchiveBatchPathRow,
    requested_range: ExactUtcRange,
) -> Option<ExactUtcRange> {
    let coverage_start = archive
        .coverage_start_at()
        .and_then(parse_to_utc_datetime)
        .unwrap_or(requested_range.start);
    let coverage_end =
        summary_projection_archive_coverage_end_exclusive(archive).unwrap_or(requested_range.end);
    let start = coverage_start.max(requested_range.start);
    let end = coverage_end.min(requested_range.end);
    (start < end).then_some(ExactUtcRange { start, end })
}

pub(crate) fn summary_projection_archive_coverage_end_exclusive(
    archive: &crate::stats::ArchiveBatchPathRow,
) -> Option<DateTime<Utc>> {
    // Archive manifests record their final row inclusively, while every exact projection range
    // is half-open. Archive rows use second-granularity timestamps, so advance the endpoint by
    // one second before issuing the `< end` archive query.
    archive
        .coverage_end_at()
        .and_then(parse_to_utc_datetime)
        .and_then(|end| end.checked_add_signed(ChronoDuration::seconds(1)))
}

pub(crate) fn summary_projection_archive_is_fully_within_exact_horizon(
    archive: &crate::stats::ArchiveBatchPathRow,
    exact_horizon: ExactUtcRange,
) -> bool {
    let Some(start) = archive.coverage_start_at().and_then(parse_to_utc_datetime) else {
        return false;
    };
    let Some(end) = summary_projection_archive_coverage_end_exclusive(archive) else {
        return false;
    };
    start >= exact_horizon.start && end <= exact_horizon.end
}

pub(crate) fn summary_projection_archive_is_fully_represented_for_current(
    archive: &crate::stats::ArchiveBatchPathRow,
    exact_horizon: ExactUtcRange,
) -> bool {
    let Some(start) = archive.coverage_start_at().and_then(parse_to_utc_datetime) else {
        return false;
    };
    let Some(end) = summary_projection_archive_coverage_end_exclusive(archive) else {
        return false;
    };
    start >= exact_horizon.start && end <= exact_horizon.end
}

pub(crate) fn summary_projection_range_is_fully_within_exact_horizon(
    range: ExactUtcRange,
    exact_horizon: ExactUtcRange,
) -> bool {
    range.start >= exact_horizon.start && range.end <= exact_horizon.end
}

pub(crate) fn summary_projection_archive_has_exact_all_time_source(
    archive: &crate::stats::ArchiveBatchPathRow,
    actual_coverage_ranges: &HashMap<String, ExactUtcRange>,
    exact_horizon: ExactUtcRange,
) -> bool {
    summary_projection_archive_is_fully_within_exact_horizon(archive, exact_horizon)
        || actual_coverage_ranges
            .get(archive.file_path())
            .is_some_and(|range| {
                summary_projection_range_is_fully_within_exact_horizon(*range, exact_horizon)
            })
}

pub(crate) fn summary_projection_archive_coverage_range(
    archive: &crate::stats::ArchiveBatchPathRow,
) -> Option<ExactUtcRange> {
    let start = archive
        .coverage_start_at()
        .and_then(parse_to_utc_datetime)?;
    let end = summary_projection_archive_coverage_end_exclusive(archive)?;
    (start < end).then_some(ExactUtcRange { start, end })
}

pub(crate) fn summary_projection_archive_raw_admission_exceeded(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    summary_projection_resident_record_budget_exceeded(error)
        || message.contains("archive byte budget exceeded before preview fetch")
        // The shared archive reader names this bounded admission error after its
        // raw/unmaterialized mode. Materialized archives use the same reader for exact rolling
        // repairs and newest-N candidates, where it must make only that selection unavailable.
        || message.contains("unmaterialized archive exact-record budget")
        || message.contains("paged boundary raw archive admission exceeded")
        || (message.contains("exact-record budget")
            && message.contains("unmaterialized archive data"))
}

pub(crate) fn summary_projection_all_time_materialized_scope_coverage(
    archives: &[crate::stats::ArchiveBatchPathRow],
    replay_coverage: &HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    archive_account_manifest_refreshed_paths: &HashSet<String>,
    archive_account_ids_by_file: &HashMap<String, HashSet<i64>>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
) -> Result<(bool, bool)> {
    let mut global_covered = true;
    let mut accounts_covered = true;

    for archive in archives {
        if !archive.has_materialized_historical_rollups() {
            continue;
        }
        let replay = summary_projection_effective_replay_coverage(
            archive,
            replay_coverage
                .get(archive.file_path())
                .copied()
                .unwrap_or_default(),
        );
        let Some(range) = summary_projection_archive_coverage_range(archive) else {
            // Legacy manifests lack a finite durable coverage range. Materialization alone does
            // not prove the compact aggregate was derived from this immutable archive, so every
            // scope still requires an identity-matching replay marker.
            global_covered &= replay.overall;
            accounts_covered &= replay.account_stats
                && archive_account_manifest_refreshed_paths.contains(archive.file_path());
            continue;
        };
        if summary_projection_exact_range_fits_bucket_budget(range).is_err() {
            return Ok((false, false));
        }
        // A missing manifest cannot prove that the archive contains no account-scoped rows.
        // Serving its global compact aggregate to an account request would silently omit those
        // rows when the account rollup is absent. The refresh marker additionally proves this
        // otherwise bounded list was completed, rather than an interrupted prefix.
        let account_ids = archive_account_ids_by_file.get(archive.file_path());
        if account_ids.is_none() {
            accounts_covered = false;
        }
        accounts_covered &= replay.account_stats
            && archive_account_manifest_refreshed_paths.contains(archive.file_path());
        let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        while bucket <= last_bucket {
            // An aggregate row alone can represent an interrupted replay prefix. Its durable
            // replay marker is the completion proof, so require both before replacing the
            // exact archive source outside the bounded raw horizon.
            global_covered &= replay.overall && hourly_rollup_totals.contains_key(&(bucket, None));
            if let Some(account_ids) = account_ids.filter(|account_ids| !account_ids.is_empty()) {
                accounts_covered &= account_ids.iter().all(|account_id| {
                    hourly_rollup_totals.contains_key(&(bucket, Some(*account_id)))
                });
            }
            if !global_covered && !accounts_covered {
                return Ok((false, false));
            }
            bucket = bucket.saturating_add(3_600);
        }
    }

    Ok((global_covered, accounts_covered))
}

pub(crate) async fn load_summary_projection_archive_coverage_range(
    pool: &Pool<Sqlite>,
) -> Result<Option<ExactUtcRange>> {
    let (first, last) = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT MIN(occurred_at), MAX(occurred_at) FROM codex_invocations",
    )
    .fetch_one(pool)
    .await?;
    let (Some(first), Some(last)) = (first.as_deref(), last.as_deref()) else {
        return Ok(None);
    };
    let start = parse_to_utc_datetime(first)
        .ok_or_else(|| anyhow!("summary projection archive coverage start is invalid"))?;
    let end = parse_to_utc_datetime(last)
        .and_then(|last| last.checked_add_signed(ChronoDuration::seconds(1)))
        .ok_or_else(|| anyhow!("summary projection archive coverage end is invalid"))?;
    (start < end)
        .then_some(ExactUtcRange { start, end })
        .ok_or_else(|| anyhow!("summary projection archive coverage range is empty"))
        .map(Some)
}

pub(crate) fn summary_projection_archive_has_coverage_bounds(
    archive: &crate::stats::ArchiveBatchPathRow,
) -> bool {
    archive
        .coverage_start_at()
        .and_then(parse_to_utc_datetime)
        .is_some()
        && archive
            .coverage_end_at()
            .and_then(parse_to_utc_datetime)
            .is_some()
}

pub(crate) fn summary_projection_effective_replay_coverage(
    _archive: &crate::stats::ArchiveBatchPathRow,
    coverage: SummaryProjectionArchiveReplayCoverage,
) -> SummaryProjectionArchiveReplayCoverage {
    coverage
}

pub(crate) fn summary_projection_archive_exact_ranges(
    archive_has_materialized_rollups: bool,
    exact_range: ExactUtcRange,
    protected_boundary_buckets: &HashSet<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: &HashSet<i64>,
) -> Vec<ExactUtcRange> {
    summary_projection_archive_exact_ranges_with_coverage(
        archive_has_materialized_rollups,
        None,
        None,
        None,
        exact_range,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        known_account_ids,
    )
}

pub(crate) fn summary_projection_archive_exact_ranges_with_coverage(
    archive_has_materialized_rollups: bool,
    overall_rollup_archive_replayed: Option<bool>,
    account_rollup_archive_replayed: Option<bool>,
    usage_rollup_archive_replayed: Option<bool>,
    exact_range: ExactUtcRange,
    protected_boundary_buckets: &HashSet<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: &HashSet<i64>,
) -> Vec<ExactUtcRange> {
    if !archive_has_materialized_rollups {
        return vec![exact_range];
    }

    let first_bucket = align_bucket_epoch(exact_range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(exact_range.end.timestamp(), 3_600, 0);
    let first_full_bucket = ceil_hour_epoch(exact_range.start.timestamp());
    let mut exact_buckets = protected_boundary_buckets.clone();
    let mut bucket = first_bucket;
    while bucket <= last_bucket {
        let bucket_end = bucket.saturating_add(3_600);
        let is_full_bucket =
            bucket >= first_full_bucket && bucket_end <= exact_range.end.timestamp();
        if is_full_bucket && !exact_buckets.contains(&bucket) {
            let global_covered = hourly_rollup_totals.contains_key(&(bucket, None))
                && overall_rollup_archive_replayed.is_none_or(|replayed| replayed);
            let global_usage_covered = hourly_rollup_usage.contains_key(&(bucket, None))
                && usage_rollup_archive_replayed.is_none_or(|replayed| replayed);
            // Durable archive replay markers prove each response dimension consumed this whole
            // archive. Without them, a present rollup key is not enough: it may be stale or
            // partial, and requiring every durable account would also turn inactive accounts
            // into a full-archive exact read on every refresh.
            let accounts_covered = match (
                account_rollup_archive_replayed,
                usage_rollup_archive_replayed,
            ) {
                (Some(account_replayed), Some(usage_replayed)) => {
                    account_replayed && usage_replayed
                }
                _ => {
                    known_account_ids.iter().all(|account_id| {
                        hourly_rollup_totals.contains_key(&(bucket, Some(*account_id)))
                            && hourly_rollup_usage.contains_key(&(bucket, Some(*account_id)))
                    }) && !known_account_ids.is_empty()
                }
            };
            if !global_covered || !global_usage_covered || !accounts_covered {
                exact_buckets.insert(bucket);
            }
        }
        bucket = bucket.saturating_add(3_600);
    }

    let mut sorted_buckets = exact_buckets.into_iter().collect::<Vec<_>>();
    sorted_buckets.sort_unstable();
    let mut ranges = Vec::<ExactUtcRange>::new();
    for bucket in sorted_buckets {
        let start = Utc.timestamp_opt(bucket, 0).single();
        let end = Utc.timestamp_opt(bucket.saturating_add(3_600), 0).single();
        let (Some(start), Some(end)) = (start, end) else {
            continue;
        };
        let start = start.max(exact_range.start);
        let end = end.min(exact_range.end);
        if start >= end {
            continue;
        }
        if let Some(previous) = ranges.last_mut()
            && previous.end >= start
        {
            previous.end = previous.end.max(end);
        } else {
            ranges.push(ExactUtcRange { start, end });
        }
    }
    ranges
}

pub(crate) fn summary_projection_merge_exact_ranges(
    mut ranges: Vec<ExactUtcRange>,
) -> Vec<ExactUtcRange> {
    ranges.sort_by_key(|range| range.start);
    let mut merged = Vec::<ExactUtcRange>::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut()
            && previous.end >= range.start
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

pub(crate) async fn load_summary_projection_unrepresented_current_archive_coverage(
    pool: &Pool<Sqlite>,
    represented_archive_paths: &HashSet<String>,
    exact_horizon: ExactUtcRange,
) -> Result<(Vec<ExactUtcRange>, Option<DateTime<Utc>>, bool)> {
    // A current selection can only ignore an archive after its complete bounded coverage has
    // been materialized into the resident candidate view. Keep the exclusion bind set small;
    // a larger fully represented set is still safe, but its SQL proof must fail closed rather
    // than exceed SQLite's parameter budget.
    if represented_archive_paths.len() > SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE {
        return Ok((Vec::new(), None, true));
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT month_key, coverage_start_epoch, coverage_end_epoch \
         FROM archive_batches WHERE dataset = ",
    );
    query
        .push_bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .push(" AND status = ")
        .push_bind(ARCHIVE_STATUS_COMPLETED)
        .push(" AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'");
    if !represented_archive_paths.is_empty() {
        query.push(" AND file_path NOT IN (");
        let mut separated = query.separated(", ");
        for path in represented_archive_paths {
            separated.push_bind(path);
        }
        separated.push_unseparated(")");
    }
    let rows: Vec<(String, Option<i64>, Option<i64>)> =
        query.build_query_as().fetch_all(pool).await?;
    let mut finite_ranges = Vec::new();
    let mut has_unknown_coverage = false;
    let mut latest_coverage_end = None;
    let source_row_count = rows.len();
    let mut horizon_fallback_count = 0usize;
    for (month_key, start_epoch, end_epoch) in rows {
        let known_range = match (start_epoch, end_epoch) {
            (Some(start_epoch), Some(end_epoch)) => {
                let start = Utc.timestamp_opt(start_epoch, 0).single();
                let end = Utc.timestamp_opt(end_epoch, 0).single();
                match (start, end) {
                    (Some(start), Some(end)) if start <= end => Some((
                        start,
                        end.checked_add_signed(ChronoDuration::seconds(1))
                            .unwrap_or(end),
                    )),
                    _ => {
                        horizon_fallback_count = horizon_fallback_count.saturating_add(1);
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some((start, end)) = known_range {
            latest_coverage_end = latest_coverage_end.max(Some(end));
            let localized = ExactUtcRange {
                start: start.max(exact_horizon.start),
                end: end.min(exact_horizon.end),
            };
            if localized.start < localized.end {
                finite_ranges.push(localized);
            }
            continue;
        }

        // A parseable month key gives a finite conservative range that the request path can
        // compare with its actual Nth current row. A malformed identity remains broad
        // fail-closed because no temporal boundary can be proven.
        let Some(buckets) =
            crate::stats::archive_bucket_start_epochs_from_bounds(Some(&month_key), None, None)
                .ok()
                .filter(|buckets| !buckets.is_empty())
        else {
            has_unknown_coverage = true;
            continue;
        };
        let start_epoch = *buckets.iter().min().expect("non-empty archive buckets");
        let end_epoch = buckets
            .iter()
            .max()
            .copied()
            .unwrap_or(start_epoch)
            .saturating_add(3_600);
        let (Some(start), Some(end)) = (
            Utc.timestamp_opt(start_epoch, 0).single(),
            Utc.timestamp_opt(end_epoch, 0).single(),
        ) else {
            has_unknown_coverage = true;
            continue;
        };
        let localized = ExactUtcRange {
            start: start.max(exact_horizon.start),
            end: end.min(exact_horizon.end),
        };
        if localized.start < localized.end {
            finite_ranges.push(localized);
        }
        latest_coverage_end = latest_coverage_end.max(Some(end));
    }
    info!(
        stage = "unrepresented_current_archive_coverage_summary",
        source_row_count,
        range_count = finite_ranges.len(),
        horizon_fallback_count,
        has_unknown_coverage,
        "summary projection unrepresented current archive coverage summarized"
    );
    Ok((
        summary_projection_merge_exact_ranges(finite_ranges),
        latest_coverage_end,
        has_unknown_coverage,
    ))
}

pub(crate) async fn load_summary_projection_current_archive_admission(
    pool: &Pool<Sqlite>,
    current_start: DateTime<Utc>,
) -> Result<Vec<crate::stats::ArchiveBatchPathRow>> {
    // Current is ordered by recency, not archive creation order. Admit the newest bounded
    // manifest prefix so a large backlog of old materialized batches cannot hide a current
    // candidate that must be hydrated off-request.
    let rows = sqlx::query_as::<_, crate::stats::ArchiveBatchPathRow>(
        "SELECT file_path, month_key, coverage_start_at, coverage_end_at, \
         historical_rollups_materialized_at, NULL AS needs_overall, NULL AS needs_failures \
         FROM archive_batches \
         WHERE dataset = ?1 AND status = ?2 \
           AND sha256 IS NOT NULL AND TRIM(sha256) <> '' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
           AND (COALESCE(coverage_start_epoch, CAST(strftime('%s', coverage_start_at) AS INTEGER)) IS NULL \
                OR COALESCE(coverage_end_epoch, CAST(strftime('%s', coverage_end_at) AS INTEGER)) IS NULL \
                OR COALESCE(coverage_end_epoch, CAST(strftime('%s', coverage_end_at) AS INTEGER)) >= ?3) \
         ORDER BY COALESCE(coverage_end_epoch, CAST(strftime('%s', coverage_end_at) AS INTEGER)) DESC, id DESC \
         LIMIT ?4",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(current_start.timestamp())
    .bind((SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES.saturating_add(1)) as i64)
    .fetch_all(pool)
    .await
    .map_err(anyhow::Error::from)?;
    Ok(rows)
}

pub(crate) fn summary_projection_exact_bucket_ranges(buckets: &HashSet<i64>) -> Vec<ExactUtcRange> {
    summary_projection_merge_exact_ranges(
        buckets
            .iter()
            .filter_map(|bucket| {
                let start = Utc.timestamp_opt(*bucket, 0).single()?;
                let end = Utc
                    .timestamp_opt(bucket.saturating_add(3_600), 0)
                    .single()?;
                Some(ExactUtcRange { start, end })
            })
            .collect(),
    )
}

pub(crate) fn summary_projection_ensure_exact_bucket_budget(buckets: &HashSet<i64>) -> Result<()> {
    if buckets.len() > SUMMARY_PROJECTION_MAX_EXACT_BUCKETS {
        return Err(anyhow!(
            "summary projection exact bucket budget ({SUMMARY_PROJECTION_MAX_EXACT_BUCKETS}) exceeded"
        ));
    }
    Ok(())
}

pub(crate) fn summary_projection_exact_range_fits_bucket_budget(
    range: ExactUtcRange,
) -> Result<()> {
    let first_bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
    let bucket_count = last_bucket
        .saturating_sub(first_bucket)
        .saturating_div(3_600)
        .saturating_add(1) as usize;
    if bucket_count > SUMMARY_PROJECTION_MAX_EXACT_BUCKETS {
        return Err(anyhow!(
            "summary projection exact range bucket budget ({SUMMARY_PROJECTION_MAX_EXACT_BUCKETS}) exceeded"
        ));
    }
    Ok(())
}

pub(crate) fn summary_projection_extend_exact_buckets_for_ranges(
    buckets: &mut HashSet<i64>,
    ranges: impl IntoIterator<Item = ExactUtcRange>,
) -> Result<()> {
    for range in ranges {
        summary_projection_exact_range_fits_bucket_budget(range)?;
        let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        while bucket <= last_bucket {
            buckets.insert(bucket);
            summary_projection_ensure_exact_bucket_budget(buckets)?;
            bucket = bucket.saturating_add(3_600);
        }
    }
    Ok(())
}

pub(crate) fn summary_projection_mark_unavailable_archive_ranges(
    buckets: &mut BTreeSet<i64>,
    ranges: impl IntoIterator<Item = ExactUtcRange>,
) -> Result<()> {
    for range in ranges {
        if range.start >= range.end {
            continue;
        }
        let first_bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        let bucket_count = last_bucket
            .saturating_sub(first_bucket)
            .saturating_div(3_600)
            .saturating_add(1) as usize;
        if bucket_count > SUMMARY_PROJECTION_MAX_EXACT_BUCKETS {
            return Err(anyhow!(
                "summary projection unavailable archive range budget ({SUMMARY_PROJECTION_MAX_EXACT_BUCKETS}) exceeded"
            ));
        }
        let mut bucket = first_bucket;
        while bucket <= last_bucket {
            if !buckets.contains(&bucket) && buckets.len() >= SUMMARY_PROJECTION_MAX_EXACT_BUCKETS {
                return Err(anyhow!(
                    "summary projection unavailable archive range budget ({SUMMARY_PROJECTION_MAX_EXACT_BUCKETS}) exceeded"
                ));
            }
            buckets.insert(bucket);
            bucket = bucket.saturating_add(3_600);
        }
    }
    Ok(())
}

pub(crate) fn summary_projection_admit_paged_boundary_raw_archive(
    admitted_archives: &mut usize,
) -> Result<()> {
    if *admitted_archives >= SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        return Err(anyhow!(
            "summary projection paged boundary raw archive admission exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES})"
        ));
    }
    *admitted_archives += 1;
    Ok(())
}

pub(crate) fn summary_projection_unavailable_bucket_ranges(
    buckets: BTreeSet<i64>,
) -> Vec<ExactUtcRange> {
    let mut ranges = Vec::<ExactUtcRange>::new();
    for bucket in buckets {
        let Some(start) = Utc.timestamp_opt(bucket, 0).single() else {
            continue;
        };
        let end = start + ChronoDuration::hours(1);
        if let Some(previous) = ranges.last_mut()
            && previous.end >= start
        {
            previous.end = previous.end.max(end);
        } else {
            ranges.push(ExactUtcRange { start, end });
        }
    }
    ranges
}

pub(crate) fn summary_projection_split_ranges_by_buckets(
    ranges: impl IntoIterator<Item = ExactUtcRange>,
    selected_buckets: &HashSet<i64>,
) -> (Vec<ExactUtcRange>, Vec<ExactUtcRange>) {
    let mut selected = Vec::new();
    let mut deferred = Vec::new();
    for range in ranges {
        let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        while bucket <= last_bucket {
            let bucket_start = Utc
                .timestamp_opt(bucket, 0)
                .single()
                .expect("valid summary archive bucket start");
            let segment = ExactUtcRange {
                start: range.start.max(bucket_start),
                end: range.end.min(bucket_start + ChronoDuration::hours(1)),
            };
            if segment.start < segment.end {
                if selected_buckets.contains(&bucket) {
                    selected.push(segment);
                } else {
                    deferred.push(segment);
                }
            }
            bucket = bucket.saturating_add(3_600);
        }
    }
    (
        summary_projection_merge_exact_ranges(selected),
        summary_projection_merge_exact_ranges(deferred),
    )
}

pub(crate) fn summary_projection_partial_rollup_ranges(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Vec<ExactUtcRange> {
    if start >= end {
        return Vec::new();
    }
    let first_full_hour = Utc
        .timestamp_opt(ceil_hour_epoch(start.timestamp()), 0)
        .single()
        .expect("valid summary partial range start");
    let last_full_hour = Utc
        .timestamp_opt(align_bucket_epoch(end.timestamp(), 3_600, 0), 0)
        .single()
        .expect("valid summary partial range end");
    let mut partial_ranges = Vec::new();
    if start < first_full_hour {
        partial_ranges.push(ExactUtcRange {
            start,
            end: end.min(first_full_hour),
        });
    }
    if first_full_hour <= last_full_hour && last_full_hour < end {
        partial_ranges.push(ExactUtcRange {
            start: last_full_hour,
            end,
        });
    }
    partial_ranges
}

pub(crate) fn summary_projection_mark_unavailable_archive_ranges_by_requirement(
    unavailable_ranges: &mut BTreeSet<i64>,
    unavailable_boundary_ranges: &mut Vec<ExactUtcRange>,
    archive_has_materialized_rollups: bool,
    exact_ranges: &[ExactUtcRange],
    exact_bucket_requirements: &HashSet<i64>,
) -> Result<()> {
    if !archive_has_materialized_rollups {
        return summary_projection_mark_unavailable_archive_ranges(
            unavailable_ranges,
            exact_ranges.iter().copied(),
        );
    }

    let mut strict_ranges = Vec::new();
    let mut boundary_ranges = Vec::new();
    for range in exact_ranges {
        let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
        let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
        while bucket <= last_bucket {
            let bucket_start = Utc
                .timestamp_opt(bucket, 0)
                .single()
                .expect("valid summary archive bucket start");
            let bucket_end = bucket_start + ChronoDuration::hours(1);
            let segment = ExactUtcRange {
                start: range.start.max(bucket_start),
                end: range.end.min(bucket_end),
            };
            if segment.start < segment.end {
                if exact_bucket_requirements.contains(&bucket) {
                    strict_ranges.push(segment);
                } else {
                    boundary_ranges.push(segment);
                }
            }
            bucket = bucket.saturating_add(3_600);
        }
    }
    summary_projection_mark_unavailable_archive_ranges(unavailable_ranges, strict_ranges)?;
    unavailable_boundary_ranges.extend(boundary_ranges);
    Ok(())
}

pub(crate) fn summary_projection_mark_exact_replacement_buckets(
    archive_has_materialized_rollups: bool,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    exact_range: ExactUtcRange,
    _protected_boundary_buckets: &HashSet<i64>,
    exact_global_total_rollup_buckets: &mut HashSet<i64>,
    exact_account_total_rollup_buckets: &mut HashSet<i64>,
    exact_global_usage_rollup_buckets: &mut HashSet<i64>,
    exact_account_usage_rollup_buckets: &mut HashSet<i64>,
) -> Result<()> {
    if !archive_has_materialized_rollups {
        return Ok(());
    }
    // All-time aggregation consumes whole compact buckets. Any archive which lacks durable
    // replay proof therefore replaces every bucket it intersects, including a partial archive
    // start/end hour. The builder hydrates every archive/live contribution for that bucket
    // before suppressing its compact aggregate.
    let first_bucket = align_bucket_epoch(exact_range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(exact_range.end.timestamp().saturating_sub(1), 3_600, 0);
    summary_projection_exact_range_fits_bucket_budget(exact_range)?;
    let mut bucket = first_bucket;
    while bucket <= last_bucket {
        if !replay_coverage.overall {
            exact_global_total_rollup_buckets.insert(bucket);
            summary_projection_ensure_exact_bucket_budget(exact_global_total_rollup_buckets)?;
        }
        if !replay_coverage.account_stats {
            exact_account_total_rollup_buckets.insert(bucket);
            summary_projection_ensure_exact_bucket_budget(exact_account_total_rollup_buckets)?;
        }
        if !replay_coverage.usage_breakdown {
            exact_global_usage_rollup_buckets.insert(bucket);
            exact_account_usage_rollup_buckets.insert(bucket);
            summary_projection_ensure_exact_bucket_budget(exact_global_usage_rollup_buckets)?;
            summary_projection_ensure_exact_bucket_budget(exact_account_usage_rollup_buckets)?;
        }
        bucket = bucket.saturating_add(3_600);
    }
    Ok(())
}

pub(crate) fn summary_projection_mark_account_exact_replacement_buckets(
    archive_has_materialized_rollups: bool,
    exact_range: ExactUtcRange,
    exact_account_total_rollup_buckets: &mut HashSet<i64>,
) -> Result<()> {
    if !archive_has_materialized_rollups {
        return Ok(());
    }
    // A stale account manifest makes the account compact aggregate unverifiable even when the
    // global replay is complete. The paged raw fallback owns this whole bucket for account
    // totals, while the independently replay-proven account usage rollup remains canonical.
    summary_projection_exact_range_fits_bucket_budget(exact_range)?;
    let mut bucket = align_bucket_epoch(exact_range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(exact_range.end.timestamp().saturating_sub(1), 3_600, 0);
    while bucket <= last_bucket {
        exact_account_total_rollup_buckets.insert(bucket);
        summary_projection_ensure_exact_bucket_budget(exact_account_total_rollup_buckets)?;
        bucket = bucket.saturating_add(3_600);
    }
    Ok(())
}

pub(crate) async fn merge_summary_projection_archive_records(
    pool: &Pool<Sqlite>,
    persisted_live_ids: &HashSet<SummarySourceIdentity>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    archive_has_materialized_rollups: bool,
    exact_range: ExactUtcRange,
    protected_boundary_buckets: &HashSet<i64>,
    usage_rollup_cursor: Option<i64>,
    known_account_ids: &HashSet<i64>,
    records_by_invoke_id: &mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &mut usize,
) -> Result<()> {
    let mut exact_record_bytes = records_by_invoke_id
        .values()
        .map(|record| summary_projection_preview_row_bytes(&record.row))
        .sum::<usize>();
    ensure_summary_projection_resident_record_bytes(exact_record_bytes, 0)?;
    merge_summary_projection_archive_records_with_coverage(
        pool,
        pool,
        persisted_live_ids,
        hourly_rollup_totals,
        hourly_rollup_usage,
        archive_has_materialized_rollups,
        None,
        None,
        None,
        exact_range,
        protected_boundary_buckets,
        usage_rollup_cursor,
        known_account_ids,
        records_by_invoke_id,
        exact_record_budget,
        &mut exact_record_bytes,
        0,
        None,
    )
    .await
}

/// Load a verified V2 Snapshot page from the primary SQLite database. This is an off-request
/// recovery source used only when the immutable archive file is unavailable; the HTTP/SSE path
/// never calls it.
pub(crate) async fn load_summary_projection_snapshot_records(
    pool: &Pool<Sqlite>,
    archive_file_path: &str,
    exact_ranges: &[ExactUtcRange],
) -> Result<Option<Vec<SummaryProjectionArchiveRow>>> {
    let Some((archive_batch_id, manifest_sha256, manifest_row_count, manifest_start, manifest_end)) =
        sqlx::query_as::<_, (i64, String, i64, Option<String>, Option<String>)>(
            "SELECT id, sha256, row_count, coverage_start_at, coverage_end_at \
             FROM archive_batches WHERE dataset = 'codex_invocations' \
             AND status = 'completed' \
             AND sha256 IS NOT NULL AND TRIM(sha256) <> '' \
             AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
             AND file_path = ?1 ORDER BY id DESC LIMIT 1",
        )
        .bind(archive_file_path)
        .fetch_optional(pool)
        .await
        .context("summary archive Snapshot identity lookup failed")?
    else {
        return Ok(None);
    };
    if manifest_sha256.trim().is_empty() || manifest_row_count < 0 {
        return Ok(None);
    }
    let pages = sqlx::query(
        "SELECT page_index, coverage_start, coverage_end, row_count, payload, payload_bytes, \
                snapshot_sha256, format_version \
         FROM summary_archive_snapshot \
         WHERE archive_batch_id = ?1 AND manifest_sha256 = ?2 ORDER BY page_index ASC",
    )
    .bind(archive_batch_id)
    .bind(&manifest_sha256)
    .fetch_all(pool)
    .await
    .context("summary archive Snapshot page lookup failed")?;
    if pages.is_empty() {
        return Ok(None);
    }
    let manifest_start = manifest_start.and_then(|value| parse_snapshot_coverage_at(&value));
    let manifest_end = manifest_end.and_then(|value| parse_snapshot_coverage_at(&value));
    let mut records = Vec::new();
    let mut total_rows = 0_i64;
    let mut first_page_start = None;
    let mut last_page_end = None;
    let mut previous_page_end = None;
    let mut previous_record_key = None;
    let mut seen_ids = HashSet::new();
    for (expected_page, page) in pages.into_iter().enumerate() {
        if page.get::<i64, _>("page_index") != i64::try_from(expected_page).unwrap_or(-1)
            || page.get::<i64, _>("format_version") != SUMMARY_ARCHIVE_SNAPSHOT_V2
        {
            return Ok(None);
        }
        let payload = page.get::<Vec<u8>, _>("payload");
        let mut hasher = sha2::Sha256::new();
        hasher.update(&payload);
        if page.get::<String, _>("snapshot_sha256") != format!("{:x}", hasher.finalize())
            || page.get::<i64, _>("payload_bytes") != i64::try_from(payload.len()).unwrap_or(-1)
        {
            return Ok(None);
        }
        let decoded = match decode_summary_archive_snapshot_v2_payload(&payload) {
            Ok(records) => records,
            Err(_) => return Ok(None),
        };
        if i64::try_from(decoded.len()).unwrap_or(-1) != page.get::<i64, _>("row_count") {
            return Ok(None);
        }
        let Some(coverage_start) = parse_snapshot_coverage_at(page.get("coverage_start")) else {
            return Ok(None);
        };
        let Some(coverage_end) = parse_snapshot_coverage_at(page.get("coverage_end")) else {
            return Ok(None);
        };
        if coverage_start > coverage_end
            || previous_page_end.is_some_and(|previous_end| coverage_start < previous_end)
        {
            return Ok(None);
        }
        first_page_start.get_or_insert(coverage_start);
        last_page_end = Some(coverage_end);
        previous_page_end = Some(coverage_end);
        total_rows = total_rows.saturating_add(page.get::<i64, _>("row_count"));
        for record in &decoded {
            let Some(occurred_at) = parse_snapshot_coverage_at(&record.occurred_at) else {
                return Ok(None);
            };
            if occurred_at < coverage_start
                || occurred_at > coverage_end
                || previous_record_key.is_some_and(|previous| (occurred_at, record.id) < previous)
                || !seen_ids.insert(record.id)
            {
                return Ok(None);
            }
            previous_record_key = Some((occurred_at, record.id));
        }
        records.extend(decoded);
    }
    if total_rows != manifest_row_count
        || manifest_start.is_some_and(|start| first_page_start != Some(start))
        || manifest_end.is_some_and(|end| last_page_end != Some(end))
    {
        return Ok(None);
    }
    let rows = records
        .into_iter()
        .filter(|record| {
            parse_to_utc_datetime(&record.occurred_at).is_some_and(|occurred_at| {
                exact_ranges
                    .iter()
                    .any(|range| occurred_at >= range.start && occurred_at < range.end)
            })
        })
        .map(|record| SummaryProjectionArchiveRow {
            id: record.id,
            invoke_id: record.invoke_id,
            occurred_at: record.occurred_at,
            source: record.source,
            model: record.model,
            response_model: record.response_model,
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cache_input_tokens: record.cache_input_tokens,
            reasoning_tokens: record.reasoning_tokens,
            reasoning_effort: record.reasoning_effort,
            total_tokens: record.total_tokens,
            cost: record.cost,
            cost_input: record.cost_input,
            cost_cache_write: record.cost_cache_write,
            cost_cache_read: record.cost_cache_read,
            cost_output: record.cost_output,
            cost_reasoning: record.cost_reasoning,
            status: record.status,
            error_message: record.error_message,
            failure_kind: record.failure_kind,
            failure_class: record.failure_class,
            upstream_account_id: record.upstream_account_id,
        })
        .collect::<Vec<_>>();
    Ok(Some(rows))
}

pub(crate) async fn merge_summary_projection_archive_records_with_coverage(
    archive_pool: &Pool<Sqlite>,
    _live_pool: &Pool<Sqlite>,
    persisted_live_ids: &HashSet<SummarySourceIdentity>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    archive_has_materialized_rollups: bool,
    overall_rollup_archive_replayed: Option<bool>,
    account_rollup_archive_replayed: Option<bool>,
    usage_rollup_archive_replayed: Option<bool>,
    exact_range: ExactUtcRange,
    protected_boundary_buckets: &HashSet<i64>,
    usage_rollup_cursor: Option<i64>,
    known_account_ids: &HashSet<i64>,
    records_by_invoke_id: &mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &mut usize,
    exact_record_bytes: &mut usize,
    resident_current_record_bytes: usize,
    snapshot_rows: Option<Vec<SummaryProjectionArchiveRow>>,
) -> Result<()> {
    let mut columns = HashMap::new();
    for column in [
        "source",
        "model",
        "payload",
        "input_tokens",
        "output_tokens",
        "cache_input_tokens",
        "reasoning_tokens",
        "total_tokens",
        "cost",
        "cost_input",
        "cost_cache_write",
        "cost_cache_read",
        "cost_output",
        "cost_reasoning",
        "status",
        "error_message",
        "failure_kind",
        "failure_class",
    ] {
        columns.insert(
            column,
            crate::stats::sqlite_table_has_column(archive_pool, "codex_invocations", column)
                .await?,
        );
    }
    let has = |column| columns.get(column).copied().unwrap_or(false);
    let payload_expression = |path: &str, cast: &str| {
        if has("payload") {
            format!(
                "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '{path}') AS {cast}) END"
            )
        } else {
            "NULL".to_string()
        }
    };

    let response_model = payload_expression("$.responseModel", "TEXT");
    let reasoning_effort = payload_expression("$.reasoningEffort", "TEXT");
    let upstream_account_id = payload_expression("$.upstreamAccountId", "INTEGER");
    let query = format!(
        "SELECT id, invoke_id, occurred_at, \
         COALESCE({}, '') AS source, {}, {} AS response_model, \
         COALESCE({}, 0) AS input_tokens, COALESCE({}, 0) AS output_tokens, \
         COALESCE({}, 0) AS cache_input_tokens, COALESCE({}, 0) AS reasoning_tokens, \
         {} AS reasoning_effort, COALESCE({}, 0) AS total_tokens, {}, {}, {}, {}, {}, {}, \
         COALESCE({}, '') AS status, {}, {}, {}, {} AS upstream_account_id \
         FROM codex_invocations",
        if has("source") { "source" } else { "NULL" },
        summary_projection_archive_column("model", has("model"), "NULL"),
        response_model,
        if has("input_tokens") {
            "input_tokens"
        } else {
            "NULL"
        },
        if has("output_tokens") {
            "output_tokens"
        } else {
            "NULL"
        },
        if has("cache_input_tokens") {
            "cache_input_tokens"
        } else {
            "NULL"
        },
        if has("reasoning_tokens") {
            "reasoning_tokens"
        } else {
            "NULL"
        },
        reasoning_effort,
        if has("total_tokens") {
            "total_tokens"
        } else {
            "NULL"
        },
        summary_projection_archive_column("cost", has("cost"), "NULL"),
        summary_projection_archive_column("cost_input", has("cost_input"), "NULL"),
        summary_projection_archive_column("cost_cache_write", has("cost_cache_write"), "NULL"),
        summary_projection_archive_column("cost_cache_read", has("cost_cache_read"), "NULL"),
        summary_projection_archive_column("cost_output", has("cost_output"), "NULL"),
        summary_projection_archive_column("cost_reasoning", has("cost_reasoning"), "NULL"),
        if has("status") { "status" } else { "NULL" },
        summary_projection_archive_column("error_message", has("error_message"), "NULL"),
        summary_projection_archive_column("failure_kind", has("failure_kind"), "NULL"),
        summary_projection_archive_column("failure_class", has("failure_class"), "NULL"),
        upstream_account_id,
    );
    let exact_ranges = summary_projection_archive_exact_ranges_with_coverage(
        archive_has_materialized_rollups,
        overall_rollup_archive_replayed,
        account_rollup_archive_replayed,
        usage_rollup_archive_replayed,
        exact_range,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        known_account_ids,
    );
    if snapshot_rows.is_none() {
        ensure_summary_projection_archive_text_budget(
            archive_pool,
            &exact_ranges,
            &columns,
            SUMMARY_PROJECTION_MAX_PREVIEW_BYTES.saturating_sub(resident_current_record_bytes),
        )
        .await?;
    }
    let mut bind_index = 1usize;
    let mut bounded_query = query;
    bounded_query.push_str(" WHERE ");
    if exact_ranges.is_empty() {
        bounded_query.push_str("0 = 1");
    } else {
        for (index, _) in exact_ranges.iter().enumerate() {
            if index > 0 {
                bounded_query.push_str(" OR ");
            }
            bounded_query.push_str(&format!(
                "(occurred_at >= ?{} AND occurred_at < ?{})",
                bind_index,
                bind_index + 1
            ));
            bind_index += 2;
        }
    }
    bounded_query.push_str(&format!(" LIMIT ?{bind_index}"));
    let archive_read_limit = (summary_projection_exact_record_limit() + 1) as i64;
    let mut archive_query = sqlx::query_as::<_, SummaryProjectionArchiveRow>(&bounded_query);
    for range in &exact_ranges {
        archive_query = archive_query
            .bind(db_occurred_at_lower_bound(range.start))
            .bind(db_occurred_at_upper_bound(range.end));
    }
    let rows = if let Some(snapshot_rows) = snapshot_rows {
        snapshot_rows
    } else {
        archive_query
            .bind(archive_read_limit)
            .fetch_all(archive_pool)
            .await?
    };
    let rows_seen = rows.len();
    if !archive_has_materialized_rollups && rows_seen > summary_projection_exact_record_limit() {
        return Err(anyhow!(
            "summary projection unmaterialized archive exact-record budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS}) exceeded"
        ));
    }
    for row in rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
        // Persisted/live rows are the richer, authoritative copy. A materialized archive is
        // covered globally, but account coverage is independent: account rollups can lag the
        // global target and must retain this exact archive copy for account-scoped reads.
        if persisted_live_ids.contains(&SummarySourceIdentity {
            row_id: row.id,
            invoke_id: row.invoke_id.clone(),
            occurred_at: row.occurred_at.clone(),
        }) || summary_projection_records_contains_identity(
            records_by_invoke_id,
            row.id,
            &row.invoke_id,
            &row.occurred_at,
        ) {
            continue;
        }
        let full_rollup_start = ceil_hour_epoch(exact_range.start.timestamp());
        let full_rollup_end = align_bucket_epoch(exact_range.end.timestamp(), 3_600, 0);
        let bucket_is_full_rollup = bucket >= full_rollup_start
            && bucket < full_rollup_end
            && !protected_boundary_buckets.contains(&bucket);
        // Replay coverage applies to the entire immutable archive. A protected rolling-window
        // boundary may still be a complete hour for an all-time or differently aligned request,
        // so a missing marker must retain the exact source for every scope rather than trusting
        // the compact bucket there.
        let global_rollup_covered = (archive_has_materialized_rollups
            || overall_rollup_archive_replayed.is_some_and(|replayed| replayed))
            && hourly_rollup_totals.contains_key(&(bucket, None))
            && overall_rollup_archive_replayed.is_none_or(|replayed| replayed);
        let account_rollup_covered = archive_has_materialized_rollups
            && hourly_rollup_totals.contains_key(&(bucket, row.upstream_account_id))
            && account_rollup_archive_replayed.is_none_or(|replayed| replayed);
        let usage_cursor_covers_row = usage_rollup_cursor.is_some_and(|cursor| row.id <= cursor);
        let usage_global_rollup_covered = hourly_rollup_usage.contains_key(&(bucket, None))
            && ((archive_has_materialized_rollups
                && usage_rollup_archive_replayed.is_none_or(|replayed| replayed))
                || (!archive_has_materialized_rollups && usage_cursor_covers_row));
        let usage_account_rollup_covered = hourly_rollup_usage
            .contains_key(&(bucket, row.upstream_account_id))
            && (archive_has_materialized_rollups
                && usage_rollup_archive_replayed.is_none_or(|replayed| replayed)
                || usage_cursor_covers_row);
        let account_archive_totals_fallback_included = false;
        if bucket_is_full_rollup
            && global_rollup_covered
            && account_rollup_covered
            && usage_global_rollup_covered
            && usage_account_rollup_covered
        {
            continue;
        }
        *exact_record_bytes =
            exact_record_bytes.saturating_add(summary_projection_archive_row_bytes(&row));
        ensure_summary_projection_resident_record_bytes(
            *exact_record_bytes,
            resident_current_record_bytes,
        )?;
        *exact_record_budget = exact_record_budget.saturating_add(1);
        if *exact_record_budget > summary_projection_exact_record_limit() {
            return Err(anyhow!(
                "summary projection exact-record budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS}) exceeded by unmaterialized archive data"
            ));
        }
        let record = SummaryProjectionRecord {
            occurred_at,
            global_rollup_covered,
            account_rollup_covered,
            usage_global_rollup_covered,
            usage_account_rollup_covered,
            is_persisted_live_record: false,
            is_archive_record: true,
            archive_has_materialized_rollups,
            account_archive_totals_fallback_included,
            row: UpstreamAccountInvocationPreviewRow {
                upstream_account_id: row.upstream_account_id,
                id: row.id,
                invoke_id: row.invoke_id,
                prompt_cache_key: None,
                occurred_at: row.occurred_at,
                conversation_created_at: None,
                status: row.status,
                live_phase: None,
                failure_class: row.failure_class,
                route_mode: None,
                model: row.model,
                request_model: None,
                response_model: row.response_model,
                total_tokens: row.total_tokens,
                cost: row.cost,
                cost_input: row.cost_input,
                cost_cache_write: row.cost_cache_write,
                cost_cache_read: row.cost_cache_read,
                cost_output: row.cost_output,
                cost_reasoning: row.cost_reasoning,
                source: Some(row.source),
                input_tokens: Some(row.input_tokens),
                output_tokens: Some(row.output_tokens),
                cache_input_tokens: Some(row.cache_input_tokens),
                reasoning_tokens: Some(row.reasoning_tokens),
                reasoning_effort: row.reasoning_effort,
                error_message: row.error_message,
                downstream_status_code: None,
                downstream_error_message: None,
                failure_kind: row.failure_kind,
                is_actionable: None,
                proxy_display_name: None,
                upstream_account_name: None,
                upstream_account_plan_type: None,
                response_content_encoding: None,
                request_compression_algorithm: None,
                transport: None,
                requested_service_tier: None,
                service_tier: None,
                billing_service_tier: None,
                t_req_read_ms: None,
                t_req_parse_ms: None,
                t_upstream_connect_ms: None,
                t_upstream_ttfb_ms: None,
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
                t_total_ms: None,
                endpoint: None,
                compaction_request_kind: None,
                compaction_response_kind: None,
                image_intent: None,
            },
        };
        let record_key = summary_projection_record_insert_key(records_by_invoke_id, &record);
        records_by_invoke_id.insert(record_key, record);
    }
    Ok(())
}

impl SummarySnapshotEntry {
    pub(crate) fn ready(response: StatsResponse) -> Self {
        Self {
            response: Some(response),
            refreshed_at: Some(Instant::now()),
        }
    }

    pub(crate) fn fresh_response(&self) -> Option<StatsResponse> {
        self.refreshed_at
            .filter(|refreshed_at| refreshed_at.elapsed() <= SUMMARY_SNAPSHOT_MAX_STALE)
            .and_then(|_| self.response.clone())
    }

    pub(crate) fn needs_refresh(&self) -> bool {
        self.fresh_response().is_none()
    }
}

pub(crate) async fn load_summary_projection_rollup_live_cursor(pool: &Pool<Sqlite>) -> Result<i64> {
    let mut connection = pool.acquire().await?;
    load_invocation_summary_rollup_live_cursor_tx(&mut connection)
        .await
        .map_err(|error| {
            anyhow!("summary projection live rollup cursor hydration failed: {error:?}")
        })
}

pub(crate) async fn load_summary_projection_account_rollup_live_cursor(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>> {
    // Account activity v2 advances independently from the global invocation rollup.  A global
    // cursor therefore cannot be used to suppress an account-scoped exact row when this marker
    // is behind it.
    sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress \
         WHERE dataset = 'invocation_account_activity_v2_repair_live_cursor'",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection account rollup cursor hydration failed")
}

pub(crate) async fn load_summary_projection_archive_account_ids(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
) -> Result<HashSet<i64>> {
    if !crate::stats::sqlite_table_has_column(pool, "codex_invocations", "payload").await? {
        return Ok(HashSet::new());
    }
    let lower = db_occurred_at_lower_bound(range.start);
    let upper = db_occurred_at_upper_bound(range.end);
    // Keep the result itself bounded. DISTINCT prevents a high-row-count archive with only a
    // handful of accounts from consuming the exact-record budget during account discovery.
    let rows = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT DISTINCT CASE WHEN json_valid(payload) \
                    THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) \
                    END \
         FROM codex_invocations \
         WHERE occurred_at >= ?1 AND occurred_at < ?2 \
         LIMIT ?3",
    )
    .bind(lower)
    .bind(upper)
    .bind((summary_projection_exact_record_limit() + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection archive account discovery failed")?;
    if rows.len() > summary_projection_exact_record_limit() {
        return Err(anyhow!(
            "summary projection archive account discovery exceeded bounded account budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    Ok(rows
        .into_iter()
        .flatten()
        .filter(|account_id| *account_id > 0)
        .collect())
}

pub(crate) async fn load_summary_projection_archive_row_counts(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashMap<String, i64>> {
    if archive_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let mut counts = HashMap::new();
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT file_path, row_count FROM archive_batches \
             WHERE dataset = 'codex_invocations' AND status = 'completed' AND file_path IN (",
        );
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        query.push(")");
        counts.extend(
            query
                .build_query_as::<(String, i64)>()
                .fetch_all(pool)
                .await
                .context("summary projection archive row-count hydration failed")?,
        );
    }
    Ok(counts)
}

pub(crate) async fn load_summary_projection_archive_manifest_sha256(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashMap<String, String>> {
    if archive_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let expected_count = archive_paths.iter().collect::<HashSet<_>>().len();
    let mut sha256_by_file_path = HashMap::with_capacity(expected_count);
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT file_path, sha256 FROM archive_batches WHERE dataset = 'codex_invocations' \
             AND status = 'completed' AND sha256 IS NOT NULL AND TRIM(sha256) <> '' \
             AND file_path IN (",
        );
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        query.push(")");
        for (file_path, sha256) in query
            .build_query_as::<(String, String)>()
            .fetch_all(pool)
            .await
            .context("summary projection archive manifest SHA hydration failed")?
        {
            if sha256.trim().is_empty()
                || sha256_by_file_path
                    .insert(file_path.clone(), sha256)
                    .is_some()
            {
                return Err(anyhow!(
                    "summary projection archive manifest SHA is missing or ambiguous for {file_path}"
                ));
            }
        }
    }
    if sha256_by_file_path.len() != expected_count {
        return Err(anyhow!(
            "summary projection archive manifest SHA hydration omitted snapshot rows"
        ));
    }
    Ok(sha256_by_file_path)
}

pub(crate) fn summary_projection_archive_file_is_readable(file_path: &str) -> bool {
    let Ok(file) = fs::File::open(file_path) else {
        return false;
    };
    let mut decoder = GzDecoder::new(file);
    io::copy(&mut decoder, &mut io::sink()).is_ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SummaryProjectionArchiveFileIdentity {
    Verified,
    Unavailable,
}

pub(crate) fn verify_summary_projection_archive_file_path_sha256(
    file_path: &str,
    expected_sha256: &str,
) -> Result<SummaryProjectionArchiveFileIdentity> {
    let actual_sha256 = match crate::maintenance::sha256_hex_file(Path::new(file_path)) {
        Ok(actual_sha256) => actual_sha256,
        // An unreadable archive contributes no raw data. Preserve the existing exact fallback:
        // materialized rollups remain usable, while an unmaterialized source is marked
        // unavailable by the strict archive readers below.
        Err(_) => return Ok(SummaryProjectionArchiveFileIdentity::Unavailable),
    };
    if actual_sha256 != expected_sha256 {
        if !summary_projection_archive_file_is_readable(file_path) {
            return Ok(SummaryProjectionArchiveFileIdentity::Unavailable);
        }
        return Err(anyhow!(
            "summary projection archive changed during hydration for {}",
            file_path
        ));
    }
    Ok(SummaryProjectionArchiveFileIdentity::Verified)
}

pub(crate) fn verify_summary_projection_archive_file_sha256(
    archive: &crate::stats::ArchiveBatchPathRow,
    expected_sha256: &str,
) -> Result<SummaryProjectionArchiveFileIdentity> {
    verify_summary_projection_archive_file_path_sha256(archive.file_path(), expected_sha256)
}

pub(crate) fn require_summary_projection_archive_file_sha256(
    archive: &crate::stats::ArchiveBatchPathRow,
    expected_sha256: &str,
) -> Result<()> {
    match verify_summary_projection_archive_file_sha256(archive, expected_sha256)? {
        SummaryProjectionArchiveFileIdentity::Verified => Ok(()),
        SummaryProjectionArchiveFileIdentity::Unavailable => Err(anyhow!(
            "summary projection archive became unavailable during hydration for {}",
            archive.file_path()
        )),
    }
}

pub(crate) fn verify_summary_projection_archive_file_paths_sha256(
    archive_paths: &[String],
    expected_sha256_by_file_path: &HashMap<String, String>,
) -> Result<Vec<String>> {
    let mut verified_paths = Vec::with_capacity(archive_paths.len());
    for file_path in archive_paths {
        let expected_sha256 = expected_sha256_by_file_path.get(file_path).ok_or_else(|| {
            anyhow!(
                "summary projection archive manifest SHA is missing for {}",
                file_path
            )
        })?;
        if verify_summary_projection_archive_file_path_sha256(file_path, expected_sha256)?
            == SummaryProjectionArchiveFileIdentity::Verified
        {
            verified_paths.push(file_path.clone());
        }
    }
    Ok(verified_paths)
}

pub(crate) fn require_summary_projection_archive_file_paths_sha256(
    archive_paths: &[String],
    expected_sha256_by_file_path: &HashMap<String, String>,
) -> Result<()> {
    for file_path in archive_paths {
        let expected_sha256 = expected_sha256_by_file_path.get(file_path).ok_or_else(|| {
            anyhow!(
                "summary projection archive manifest SHA is missing for {}",
                file_path
            )
        })?;
        match verify_summary_projection_archive_file_path_sha256(file_path, expected_sha256)? {
            SummaryProjectionArchiveFileIdentity::Verified => {}
            SummaryProjectionArchiveFileIdentity::Unavailable => {
                return Err(anyhow!(
                    "summary projection archive became unavailable during hydration for {}",
                    file_path
                ));
            }
        }
    }
    Ok(())
}

pub(crate) struct SummaryProjectionAllTimeArchiveScanPaths {
    pub(crate) global: Vec<String>,
    pub(crate) account: Vec<String>,
    pub(crate) global_unmaterialized_count: usize,
    pub(crate) global_unmaterialized: Vec<String>,
}

pub(crate) async fn load_summary_projection_all_time_archive_scan_paths(
    pool: &Pool<Sqlite>,
) -> Result<SummaryProjectionAllTimeArchiveScanPaths> {
    // Every downstream legacy fallback is keyed by file path. If a path has multiple completed
    // manifest identities, selecting the latest row could attach the wrong proof or snapshot to
    // an older archive. Refuse the ambiguous authority and leave the affected all-time scope
    // unavailable until recovery can establish an identity-bound V2 proof.
    if sqlx::query_scalar::<_, String>(
        "SELECT file_path FROM archive_batches \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
         GROUP BY file_path \
         HAVING COUNT(DISTINCT COALESCE(NULLIF(TRIM(sha256), ''), '<missing>')) > 1 \
         ORDER BY file_path LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection archive identity ambiguity check failed")?
    .is_some()
    {
        return Err(anyhow!(
            "summary projection archive identity is ambiguous for a completed path"
        ));
    }
    if sqlx::query_scalar::<_, String>(
        "SELECT file_path FROM archive_batches \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror' \
         GROUP BY file_path, sha256 \
         HAVING COUNT(*) > 1 \
         ORDER BY file_path LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection duplicate archive identity check failed")?
    .is_some()
    {
        return Err(anyhow!(
            "summary projection archive identity is duplicated for a completed path"
        ));
    }
    // These are precisely the completed archive paths that the two generic all-time
    // aggregators may inflate: the global pass handles missing invocation replay, and the
    // account pass additionally handles unmaterialized archives missing account replay.
    let rows = sqlx::query_as::<_, (String, i64, i64, i64)>(
        "SELECT batches.file_path, \
                MAX(NOT EXISTS ( \
                    SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                    WHERE replay.target = ?1 \
                      AND replay.dataset = 'codex_invocations' \
                      AND replay.file_path = batches.file_path \
                      AND batches.sha256 IS NOT NULL \
                      AND TRIM(batches.sha256) <> '' \
                      AND replay.archive_sha256 = batches.sha256 \
                ) OR ( \
                    batches.historical_rollups_materialized_at IS NULL \
                    AND ( \
                        NOT EXISTS ( \
                            SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                            WHERE replay.target = ?2 \
                              AND replay.dataset = 'codex_invocations' \
                              AND replay.file_path = batches.file_path \
                              AND batches.sha256 IS NOT NULL \
                              AND TRIM(batches.sha256) <> '' \
                              AND replay.archive_sha256 = batches.sha256 \
                        ) \
                        OR NOT EXISTS ( \
                            SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                            WHERE replay.target = ?3 \
                              AND replay.dataset = 'codex_invocations' \
                              AND replay.file_path = batches.file_path \
                              AND batches.sha256 IS NOT NULL \
                              AND TRIM(batches.sha256) <> '' \
                              AND replay.archive_sha256 = batches.sha256 \
                        ) \
                    ) \
                )) AS needs_global_archive_scan, \
                MAX(batches.historical_rollups_materialized_at IS NULL AND NOT EXISTS ( \
                    SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                    WHERE replay.target = ?2 \
                      AND replay.dataset = 'codex_invocations' \
                      AND replay.file_path = batches.file_path \
                      AND batches.sha256 IS NOT NULL \
                      AND TRIM(batches.sha256) <> '' \
                      AND replay.archive_sha256 = batches.sha256 \
                )) AS needs_account_archive_scan, \
                MAX(batches.historical_rollups_materialized_at IS NULL) AS is_unmaterialized \
         FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
         AND batches.status = 'completed' \
         AND batches.sha256 IS NOT NULL AND TRIM(batches.sha256) <> '' \
         AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
         AND (NOT EXISTS ( \
                   SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                   WHERE replay.target = ?1 \
                     AND replay.dataset = 'codex_invocations' \
                     AND replay.file_path = batches.file_path \
                     AND batches.sha256 IS NOT NULL \
                     AND TRIM(batches.sha256) <> '' \
                     AND replay.archive_sha256 = batches.sha256 \
               ) OR ( \
                   batches.historical_rollups_materialized_at IS NULL \
                   AND (NOT EXISTS ( \
                       SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                       WHERE replay.target = ?2 \
                         AND replay.dataset = 'codex_invocations' \
                         AND replay.file_path = batches.file_path \
                         AND batches.sha256 IS NOT NULL \
                         AND TRIM(batches.sha256) <> '' \
                         AND replay.archive_sha256 = batches.sha256 \
                   ) OR NOT EXISTS ( \
                       SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                       WHERE replay.target = ?3 \
                         AND replay.dataset = 'codex_invocations' \
                         AND replay.file_path = batches.file_path \
                         AND batches.sha256 IS NOT NULL \
                         AND TRIM(batches.sha256) <> '' \
                         AND replay.archive_sha256 = batches.sha256 \
                   )) \
               )) \
         GROUP BY batches.file_path \
         ORDER BY MIN(batches.month_key) ASC, MIN(batches.created_at) ASC, MIN(batches.id) ASC \
         LIMIT ?4",
    )
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .bind((SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection all-time archive scan admission failed")?;
    if rows.len() > SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        return Err(anyhow!(
            "summary projection all-time archive scan cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES})"
        ));
    }
    let mut global = Vec::new();
    let mut account = Vec::new();
    let mut global_unmaterialized_count = 0usize;
    let mut global_unmaterialized = Vec::new();
    for (file_path, needs_global_archive_scan, needs_account_archive_scan, is_unmaterialized) in
        rows
    {
        if needs_global_archive_scan != 0 {
            if is_unmaterialized != 0 {
                global_unmaterialized_count += 1;
                global_unmaterialized.push(file_path.clone());
            }
            global.push(file_path.clone());
        }
        if needs_account_archive_scan != 0 {
            account.push(file_path);
        }
    }
    Ok(SummaryProjectionAllTimeArchiveScanPaths {
        global,
        account,
        global_unmaterialized_count,
        global_unmaterialized,
    })
}

pub(crate) async fn load_summary_projection_archive_manifest_accounts(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<Vec<(String, i64)>> {
    if archive_paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut accounts = Vec::new();
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT batches.file_path, activity.account_id \
             FROM archive_batches AS batches \
             JOIN archive_batch_upstream_activity AS activity \
               ON activity.archive_batch_id = batches.id \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND activity.account_id > 0 \
               AND batches.file_path IN (",
        );
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        query.push(")");
        let remaining = summary_projection_exact_record_limit().saturating_sub(accounts.len());
        query
            .push(" LIMIT ")
            .push_bind((remaining.saturating_add(1)) as i64);
        let rows = query
            .build_query_as::<(String, i64)>()
            .fetch_all(pool)
            .await
            .context("summary projection archive account manifest hydration failed")?;
        if rows.len() > remaining {
            return Err(anyhow!(
                "summary projection archive account manifest exceeded bounded row budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
            ));
        }
        accounts.extend(rows);
    }
    Ok(accounts)
}

pub(crate) async fn load_summary_projection_archive_manifest_refreshed_paths(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashSet<String>> {
    if archive_paths.is_empty() {
        return Ok(HashSet::new());
    }

    let mut refreshed_paths = HashSet::new();
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            r#"
            SELECT file_path
            FROM archive_batches
            WHERE dataset = 'codex_invocations'
              AND status = 'completed'
              AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
              AND upstream_activity_manifest_refreshed_at IS NOT NULL
              AND file_path IN (
            "#,
        );
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        let rows = query
            .push(")")
            .build_query_scalar::<String>()
            .fetch_all(pool)
            .await
            .context("summary projection archive account manifest completion hydration failed")?;
        refreshed_paths.extend(rows);
    }
    Ok(refreshed_paths)
}

pub(crate) async fn load_summary_projection_archive_replay_coverage(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashMap<String, SummaryProjectionArchiveReplayCoverage>> {
    if archive_paths.is_empty() {
        return Ok(HashMap::new());
    }
    let mut coverage = HashMap::<String, SummaryProjectionArchiveReplayCoverage>::new();
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT replay.file_path, replay.target FROM hourly_rollup_archive_replay AS replay \
             INNER JOIN archive_batches AS batches \
               ON batches.dataset = replay.dataset \
              AND batches.file_path = replay.file_path \
              AND batches.status = 'completed' \
              AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
              AND batches.sha256 IS NOT NULL \
              AND TRIM(batches.sha256) <> '' \
              AND batches.sha256 = replay.archive_sha256 \
             WHERE replay.dataset = 'codex_invocations' AND replay.target IN (",
        );
        query
            .push_bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
            .push(", ")
            .push_bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
            .push(", ")
            .push_bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
            .push(") AND replay.file_path IN (");
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        query.push(")");
        let rows = query
            .build_query_as::<(String, String)>()
            .fetch_all(pool)
            .await
            .context("summary projection archive replay coverage hydration failed")?;
        for (file_path, target) in rows {
            let entry = coverage.entry(file_path).or_default();
            if target == HOURLY_ROLLUP_TARGET_INVOCATIONS {
                entry.overall = true;
            }
            if target == HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY {
                entry.account_stats = true;
            }
            if target == HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN {
                entry.usage_breakdown = true;
            }
        }
    }
    Ok(coverage)
}

pub(crate) async fn summary_projection_completed_manifest_high_watermark(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(id) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    )
    .fetch_one(pool)
    .await
    .context("summary projection completed manifest high-watermark hydration failed")
}

pub(crate) async fn summary_projection_coverage_revision(pool: &Pool<Sqlite>) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(revision, 0) FROM summary_coverage_revision WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection coverage revision hydration failed")
    .map(|revision| revision.unwrap_or_default())
}

pub(crate) async fn summary_projection_account_coverage_revision(
    pool: &Pool<Sqlite>,
) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(revision, 0) FROM summary_account_coverage_revision WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection account coverage revision hydration failed")
    .map(|revision| revision.unwrap_or_default())
}
