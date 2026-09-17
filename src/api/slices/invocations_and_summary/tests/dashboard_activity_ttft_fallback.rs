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

async fn query_dashboard_activity_account_stats_rollup_aggregate_rows(
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

async fn dashboard_activity_materialized_archive_account_fallback_totals(
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

fn dashboard_activity_merge_account_fallback_totals(
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

fn dashboard_activity_source_scope_cache_key(source_scope: InvocationSourceScope) -> &'static str {
    match source_scope {
        InvocationSourceScope::ProxyOnly => "proxy_only",
        InvocationSourceScope::All => "all",
    }
}

pub(crate) fn build_dashboard_activity_snapshot_selection(
    range: &str,
    exact_range: ExactUtcRange,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
) -> DashboardActivitySnapshotSelection {
    DashboardActivitySnapshotSelection {
        range: range.to_string(),
        range_anchor: dashboard_activity_snapshot_selection_anchor(
            range,
            exact_range,
            reporting_tz,
        ),
        time_zone: reporting_tz.to_string(),
        source_scope: dashboard_activity_source_scope_cache_key(source_scope).to_string(),
        recent_limit,
        include_accounts,
        include_recent,
    }
}
