use super::*;
pub(crate) async fn dashboard_activity_materialized_archive_fallback_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    skipped_materialized_ranges: Vec<ExactUtcRange>,
) -> Result<StatsTotals, ApiError> {
    let mut fallback_totals = StatsTotals::default();
    for skipped_range in skipped_materialized_ranges {
        let rollup_totals = query_hourly_backed_summary_range(
            state,
            skipped_range.start,
            skipped_range.end,
            source_scope,
        )
        .await?;
        let live_rows = query_live_upstream_account_activity_aggregate_rows(
            &state.pool,
            source_scope,
            skipped_range,
            true,
            DashboardActivityExcludedInvocationIdsFilter::None,
        )
        .await?;
        let live_totals = dashboard_activity_stats_totals_from_aggregate_rows(&live_rows);
        fallback_totals = fallback_totals.add(dashboard_activity_stats_totals_subtract(
            rollup_totals,
            live_totals,
        ));
    }
    Ok(fallback_totals)
}

pub(crate) async fn dashboard_activity_materialized_archive_first_token_fallback_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    skipped_materialized_ranges: &[ExactUtcRange],
) -> Result<(i64, f64), ApiError> {
    let mut sample_count = 0_i64;
    let mut sum_ms = 0.0_f64;
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    for skipped_range in skipped_materialized_ranges {
        let range_plan = build_hourly_rollup_exact_range_plan(
            skipped_range.start,
            skipped_range.end,
            retention_cutoff,
        )?;
        let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range else {
            continue;
        };
        let full_hour_range = dashboard_activity_full_hour_exact_range(range_plan.full_hour_range)?
            .expect("non-empty full-hour epoch range should produce an exact range");
        let rollup_rows = crate::stats::query_invocation_hourly_rollup_range(
            &state.pool,
            range_start_epoch,
            range_end_epoch,
            source_scope,
        )
        .await?;
        let rollup_count = rollup_rows
            .iter()
            .map(|row| row.first_token_sample_count.max(0))
            .sum::<i64>();
        let rollup_sum = rollup_rows
            .iter()
            .map(|row| row.first_token_sum_ms.max(0.0))
            .sum::<f64>();
        let live_rows = query_live_upstream_account_activity_aggregate_rows(
            &state.pool,
            source_scope,
            full_hour_range,
            true,
            DashboardActivityExcludedInvocationIdsFilter::None,
        )
        .await?;
        let live_count = live_rows
            .iter()
            .map(|row| row.first_token_sample_count.max(0))
            .sum::<i64>();
        let live_sum = live_rows
            .iter()
            .map(|row| row.first_token_sum_ms.max(0.0))
            .sum::<f64>();
        sample_count += rollup_count.saturating_sub(live_count).max(0);
        sum_ms += (rollup_sum - live_sum).max(0.0);
    }
    Ok((sample_count, sum_ms))
}

pub(crate) fn dashboard_activity_historical_live_gap_ranges(
    range: ExactUtcRange,
    retention_cutoff: DateTime<Utc>,
) -> Result<Vec<ExactUtcRange>, ApiError> {
    if range.start >= retention_cutoff {
        return Ok(Vec::new());
    }

    let full_hour_start_epoch = ceil_hour_epoch(range.start.timestamp());
    let full_hour_end_epoch = crate::stats::align_bucket_epoch(range.end.timestamp(), 3_600, 0);
    let full_hour_start = Utc
        .timestamp_opt(full_hour_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid dashboard activity gap start epoch")))?;
    let full_hour_end = Utc
        .timestamp_opt(full_hour_end_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid dashboard activity gap end epoch")))?;

    let mut gap_ranges = Vec::new();
    if let Some(gap) = exact_utc_range(
        range.start,
        range.end.min(full_hour_start).min(retention_cutoff),
    )? {
        gap_ranges.push(gap);
    }
    if let Some(gap) = exact_utc_range(
        range.start.max(full_hour_end),
        range.end.min(retention_cutoff),
    )? {
        gap_ranges.push(gap);
    }
    Ok(gap_ranges)
}

pub(crate) async fn dashboard_activity_historical_live_gap_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    retention_cutoff: DateTime<Utc>,
) -> Result<StatsTotals, ApiError> {
    let mut totals = StatsTotals::default();
    for gap_range in dashboard_activity_historical_live_gap_ranges(range, retention_cutoff)? {
        totals = totals.add(dashboard_activity_stats_totals_from_aggregate_rows(
            &query_live_upstream_account_activity_aggregate_rows(
                &state.pool,
                source_scope,
                gap_range,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?,
        ));
    }
    Ok(totals)
}

pub(crate) fn dashboard_activity_apply_materialized_archive_fallback_to_stats(
    stats: &mut StatsResponse,
    fallback_totals: StatsTotals,
) {
    if !dashboard_activity_stats_totals_has_values(fallback_totals) {
        return;
    }

    stats.total_count += fallback_totals.total_count;
    stats.success_count += fallback_totals.success_count;
    stats.failure_count += fallback_totals.failure_count;
    stats.total_cost += fallback_totals.total_cost;
    stats.total_tokens += fallback_totals.total_tokens;
    stats.non_success_cost =
        Some(stats.non_success_cost.unwrap_or_default() + fallback_totals.non_success_cost);
    // Materialized invocation rollups do not retain model/cost-breakdown or non-success-token
    // detail, so omit those partial fields when they would no longer align with top-level totals.
    stats.usage_breakdown = None;
    stats.non_success_tokens = None;
}

pub(crate) fn dashboard_activity_clear_materialized_archive_detail_fields(
    stats: &mut StatsResponse,
) {
    stats.usage_breakdown = None;
    stats.non_success_tokens = None;
}

#[derive(Debug, FromRow)]
pub(crate) struct DashboardActivityAccountStatsRollupAggregateRow {
    pub(crate) upstream_account_id: i64,
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_token_sample_count: i64,
    pub(crate) first_token_sum_ms: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DashboardActivityUsageFallbackTotals {
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) output_tokens: i64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct DashboardActivityAccountFallbackTotals {
    pub(crate) request_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) non_success_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) success_tokens: i64,
    pub(crate) non_success_tokens: i64,
    pub(crate) failure_tokens: i64,
    pub(crate) failure_cost: f64,
    pub(crate) non_success_cost: f64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_token_sample_count: i64,
    pub(crate) first_token_sum_ms: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
}

impl DashboardActivityAccountFallbackTotals {
    pub(crate) fn from_stats_totals(totals: StatsTotals) -> Self {
        let non_success_count = totals
            .total_count
            .saturating_sub(totals.success_count)
            .max(0);
        Self {
            request_count: totals.total_count,
            success_count: totals.success_count,
            failure_count: totals.failure_count,
            non_success_count: non_success_count.max(totals.failure_count),
            total_tokens: totals.total_tokens,
            success_tokens: if non_success_count == 0 {
                totals.total_tokens
            } else {
                0
            },
            non_success_tokens: if totals.success_count == 0 {
                totals.total_tokens
            } else {
                0
            },
            failure_tokens: if totals.success_count == 0 && totals.failure_count > 0 {
                totals.total_tokens
            } else {
                0
            },
            failure_cost: if totals.success_count == 0 && totals.failure_count > 0 {
                totals.non_success_cost
            } else {
                0.0
            },
            non_success_cost: totals.non_success_cost,
            total_cost: totals.total_cost,
            ..Self::default()
        }
    }

    pub(crate) fn from_rollup_minus_live(
        row: &DashboardActivityAccountStatsRollupAggregateRow,
        live: Option<&UpstreamAccountActivityAggregateRow>,
        live_usage: DashboardActivityUsageFallbackTotals,
    ) -> Self {
        let live_request_count = live.map_or(0, |row| row.request_count);
        let live_success_count = live.map_or(0, |row| row.success_count);
        let live_failure_count = live.map_or(0, |row| row.failure_count);
        let live_total_tokens = live.map_or(0, |row| row.total_tokens);
        let live_cache_input_tokens = live.map_or(0, |row| row.cache_input_tokens);
        let live_total_cost = live.map_or(0.0, |row| row.total_cost);
        let live_non_success_cost = live.map_or(0.0, |row| row.non_success_cost);
        let live_first_response_count =
            live.map_or(0, |row| row.first_response_byte_total_sample_count);
        let live_first_response_sum = live.map_or(0.0, |row| row.first_response_byte_total_sum_ms);
        let live_first_token_count = live.map_or(0, |row| row.first_token_sample_count);
        let live_first_token_sum = live.map_or(0.0, |row| row.first_token_sum_ms);
        let live_total_latency_count = live.map_or(0, |row| row.total_latency_sample_count);
        let live_total_latency_sum = live.map_or(0.0, |row| row.total_latency_sum_ms);

        let request_count = row.request_count.saturating_sub(live_request_count).max(0);
        let success_count = row.success_count.saturating_sub(live_success_count).max(0);
        let failure_count = row.failure_count.saturating_sub(live_failure_count).max(0);
        let non_success_count = request_count
            .saturating_sub(success_count)
            .max(failure_count);
        let total_tokens = row.total_tokens.saturating_sub(live_total_tokens).max(0);
        let non_success_cost = (row.non_success_cost - live_non_success_cost).max(0.0);
        let total_cost = (row.total_cost - live_total_cost).max(0.0);
        let cache_input_tokens = row
            .cache_input_tokens
            .saturating_sub(live_cache_input_tokens)
            .max(0);
        let rollup_cache_write_tokens = row
            .input_tokens
            .max(0)
            .saturating_sub(row.cache_input_tokens.max(0))
            .max(0);
        let cache_write_tokens = rollup_cache_write_tokens
            .saturating_sub(live_usage.cache_write_tokens)
            .max(0);
        let cache_read_tokens = row
            .cache_input_tokens
            .max(0)
            .saturating_sub(live_usage.cache_read_tokens)
            .max(0);
        let output_tokens = row
            .output_tokens
            .max(0)
            .saturating_sub(live_usage.output_tokens)
            .max(0);

        Self {
            request_count,
            success_count,
            failure_count,
            non_success_count,
            total_tokens,
            success_tokens: if non_success_count == 0 {
                total_tokens
            } else {
                0
            },
            non_success_tokens: if success_count == 0 { total_tokens } else { 0 },
            failure_tokens: if success_count == 0 && failure_count > 0 {
                total_tokens
            } else {
                0
            },
            failure_cost: if success_count == 0 && failure_count > 0 {
                non_success_cost
            } else {
                0.0
            },
            non_success_cost,
            cache_input_tokens,
            total_cost,
            cache_write_tokens,
            cache_read_tokens,
            output_tokens,
            first_response_byte_total_sample_count: row
                .first_response_byte_total_sample_count
                .saturating_sub(live_first_response_count)
                .max(0),
            first_response_byte_total_sum_ms: (row.first_response_byte_total_sum_ms
                - live_first_response_sum)
                .max(0.0),
            first_token_sample_count: row
                .first_token_sample_count
                .saturating_sub(live_first_token_count)
                .max(0),
            first_token_sum_ms: (row.first_token_sum_ms - live_first_token_sum).max(0.0),
            total_latency_sample_count: row
                .total_latency_sample_count
                .saturating_sub(live_total_latency_count)
                .max(0),
            total_latency_sum_ms: (row.total_latency_sum_ms - live_total_latency_sum).max(0.0),
        }
    }

    pub(crate) fn add_assign(&mut self, other: Self) {
        self.request_count += other.request_count;
        self.success_count += other.success_count;
        self.failure_count += other.failure_count;
        self.non_success_count += other.non_success_count;
        self.total_tokens += other.total_tokens;
        self.success_tokens += other.success_tokens;
        self.non_success_tokens += other.non_success_tokens;
        self.failure_tokens += other.failure_tokens;
        self.failure_cost += other.failure_cost;
        self.non_success_cost += other.non_success_cost;
        self.cache_input_tokens += other.cache_input_tokens;
        self.total_cost += other.total_cost;
        self.cache_write_tokens += other.cache_write_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.output_tokens += other.output_tokens;
        self.first_response_byte_total_sample_count += other.first_response_byte_total_sample_count;
        self.first_response_byte_total_sum_ms += other.first_response_byte_total_sum_ms;
        self.first_token_sample_count += other.first_token_sample_count;
        self.first_token_sum_ms += other.first_token_sum_ms;
        self.total_latency_sample_count += other.total_latency_sample_count;
        self.total_latency_sum_ms += other.total_latency_sum_ms;
    }

    pub(crate) fn stats_totals(self) -> StatsTotals {
        StatsTotals {
            total_count: self.request_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_cost: self.total_cost,
            total_tokens: self.total_tokens,
            non_success_cost: self.non_success_cost,
        }
    }
}

pub(crate) fn dashboard_activity_residual_first_token_totals(
    global_sample_count: i64,
    global_sum_ms: f64,
    account_totals: &HashMap<Option<i64>, DashboardActivityAccountFallbackTotals>,
) -> (i64, f64) {
    let account_sample_count = account_totals
        .values()
        .map(|totals| totals.first_token_sample_count)
        .sum::<i64>();
    let account_sum_ms = account_totals
        .values()
        .map(|totals| totals.first_token_sum_ms)
        .sum::<f64>();
    (
        global_sample_count
            .saturating_sub(account_sample_count)
            .max(0),
        (global_sum_ms - account_sum_ms).max(0.0),
    )
}

pub(crate) async fn query_dashboard_activity_account_stats_rollup_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<DashboardActivityAccountStatsRollupAggregateRow>, ApiError> {
    let range_start_epoch = ceil_hour_epoch(range.start.timestamp());
    let range_end_epoch = crate::stats::align_bucket_epoch(range.end.timestamp(), 3_600, 0);
    if range_start_epoch >= range_end_epoch {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            upstream_account_id,
            COALESCE(SUM(total_count), 0) AS request_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_tokens), 0) AS total_tokens,
            COALESCE(SUM(input_tokens), 0) AS input_tokens,
            COALESCE(SUM(output_tokens), 0) AS output_tokens,
            COALESCE(SUM(cache_input_tokens), 0) AS cache_input_tokens,
            CAST(COALESCE(SUM(total_cost), 0.0) AS REAL) AS total_cost,
            CAST(COALESCE(SUM(non_success_cost), 0.0) AS REAL) AS non_success_cost,
            COALESCE(SUM(first_response_byte_total_sample_count), 0) AS first_response_byte_total_sample_count,
            CAST(COALESCE(SUM(first_response_byte_total_sum_ms), 0.0) AS REAL) AS first_response_byte_total_sum_ms,
            COALESCE(SUM(first_token_sample_count), 0) AS first_token_sample_count,
            CAST(COALESCE(SUM(first_token_sum_ms), 0.0) AS REAL) AS first_token_sum_ms,
            COALESCE(SUM(total_latency_sample_count), 0) AS total_latency_sample_count,
            CAST(COALESCE(SUM(total_latency_sum_ms), 0.0) AS REAL) AS total_latency_sum_ms
        FROM upstream_account_stats_hourly
        WHERE bucket_start_epoch >=
        "#,
    );
    query
        .push_bind(range_start_epoch)
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY upstream_account_id");
    Ok(query
        .build_query_as::<DashboardActivityAccountStatsRollupAggregateRow>()
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn dashboard_activity_materialized_archive_account_fallback_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    skipped_materialized_ranges: &[ExactUtcRange],
) -> Result<HashMap<Option<i64>, DashboardActivityAccountFallbackTotals>, ApiError> {
    let mut fallback_by_account =
        HashMap::<Option<i64>, DashboardActivityAccountFallbackTotals>::new();
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    for skipped_range in skipped_materialized_ranges {
        let range_plan = build_hourly_rollup_exact_range_plan(
            skipped_range.start,
            skipped_range.end,
            retention_cutoff,
        )?;
        if let Some(full_hour_range) =
            dashboard_activity_full_hour_exact_range(range_plan.full_hour_range)?
        {
            let rollup_rows = query_dashboard_activity_account_stats_rollup_aggregate_rows(
                &state.pool,
                source_scope,
                full_hour_range,
            )
            .await?;
            let live_full_hour_rows = query_live_upstream_account_activity_aggregate_rows(
                &state.pool,
                source_scope,
                full_hour_range,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?;
            let live_full_hour_by_account = live_full_hour_rows
                .into_iter()
                .map(|row| (row.upstream_account_id, row))
                .collect::<HashMap<_, _>>();
            let mut live_full_hour_usage_by_account =
                HashMap::<Option<i64>, DashboardActivityUsageFallbackTotals>::new();
            for row in query_live_upstream_account_usage_breakdown_rows(
                &state.pool,
                source_scope,
                full_hour_range,
                true,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?
            {
                let entry = live_full_hour_usage_by_account
                    .entry(row.upstream_account_id)
                    .or_default();
                entry.cache_write_tokens += row.cache_write_tokens;
                entry.cache_read_tokens += row.cache_read_tokens;
                entry.output_tokens += row.output_tokens;
            }

            for row in rollup_rows {
                let account_id = Some(row.upstream_account_id);
                let totals = DashboardActivityAccountFallbackTotals::from_rollup_minus_live(
                    &row,
                    live_full_hour_by_account.get(&account_id),
                    live_full_hour_usage_by_account
                        .get(&account_id)
                        .copied()
                        .unwrap_or_default(),
                );
                if !dashboard_activity_stats_totals_has_values(totals.stats_totals()) {
                    continue;
                }
                fallback_by_account
                    .entry(account_id)
                    .or_default()
                    .add_assign(totals);
            }
        }
    }
    Ok(fallback_by_account)
}

pub(crate) fn dashboard_activity_merge_account_fallback_totals(
    entry: &mut UpstreamAccountActivityAccumulator,
    totals: DashboardActivityAccountFallbackTotals,
) {
    entry.request_count += totals.request_count;
    entry.success_count += totals.success_count;
    entry.failure_count += totals.failure_count;
    entry.non_success_count += totals.non_success_count;
    entry.total_tokens += totals.total_tokens;
    entry.success_tokens += totals.success_tokens;
    entry.non_success_tokens += totals.non_success_tokens;
    entry.failure_tokens += totals.failure_tokens;
    entry.failure_cost += totals.failure_cost;
    entry.non_success_cost += totals.non_success_cost;
    entry.cache_input_tokens += totals.cache_input_tokens;
    entry.total_cost += totals.total_cost;
    entry.first_response_byte_total_sample_count += totals.first_response_byte_total_sample_count;
    entry.first_response_byte_total_sum_ms += totals.first_response_byte_total_sum_ms;
    entry.first_token_sample_count += totals.first_token_sample_count;
    entry.first_token_sum_ms += totals.first_token_sum_ms;
    entry.total_latency_sample_count += totals.total_latency_sample_count;
    entry.total_latency_sum_ms += totals.total_latency_sum_ms;
    entry.usage_breakdown.add_coarse_rollup_totals(
        totals.cache_write_tokens,
        totals.cache_read_tokens,
        totals.output_tokens,
        totals.total_cost,
    );
}

pub(crate) fn dashboard_activity_source_scope_cache_key(
    source_scope: InvocationSourceScope,
) -> &'static str {
    match source_scope {
        InvocationSourceScope::ProxyOnly => "proxy_only",
        InvocationSourceScope::All => "all",
    }
}

#[cfg(test)]
mod dashboard_activity_ttft_fallback_tests {
    use super::*;

    #[test]
    fn residual_archive_fallback_keeps_unassigned_ttft_samples() {
        let account_totals = HashMap::from([(
            Some(7),
            DashboardActivityAccountFallbackTotals {
                first_token_sample_count: 2,
                first_token_sum_ms: 500.0,
                ..DashboardActivityAccountFallbackTotals::default()
            },
        )]);

        assert_eq!(
            dashboard_activity_residual_first_token_totals(3, 900.0, &account_totals),
            (1, 400.0)
        );
    }
}
