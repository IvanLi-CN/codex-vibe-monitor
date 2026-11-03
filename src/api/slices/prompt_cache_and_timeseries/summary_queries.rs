use super::*;
use anyhow::anyhow;

pub(crate) async fn query_postgres_summary_all(
    state: &AppState,
    runtime: &PgAnalyticsRuntime,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    source_snapshot_id: i64,
) -> Result<StatsResponse, ApiError> {
    let fact_scope = match (source_scope, upstream_account_id) {
        (InvocationSourceScope::All, None) => AnalyticsFactScope::All,
        (InvocationSourceScope::ProxyOnly, None) => AnalyticsFactScope::ProxyOnly,
        (InvocationSourceScope::All, Some(account_id)) => AnalyticsFactScope::Account(account_id),
        (InvocationSourceScope::ProxyOnly, Some(account_id)) => {
            AnalyticsFactScope::AccountProxyOnly(account_id)
        }
    };
    let read_query = AnalyticsFactReadQuery::all(fact_scope)
        .map_err(AnalyticsReadError::into_api_error)?
        .with_source_snapshot_id(source_snapshot_id)
        .map_err(AnalyticsReadError::into_api_error)?;
    let snapshot = query_bounded_facts(runtime, read_query)
        .await
        .map_err(AnalyticsReadError::into_api_error)?;
    let mut aggregate = AnalyticsSummaryAggregate::default();
    for fact in &snapshot.facts {
        aggregate
            .add_fact(fact)
            .map_err(AnalyticsReadError::into_api_error)?;
    }
    let mut response = aggregate.into_response(None, false);
    let policy = summary_live_augmentation_policy(&SummaryWindow::All, None, Utc::now());
    let augmentation = load_summary_live_augmentation(
        state,
        source_scope,
        upstream_account_id,
        None,
        policy,
        None,
    )
    .await?;
    finalize_postgres_summary_all_response(
        &mut response,
        augmentation,
        load_stats_maintenance_response(state).await?,
    );
    Ok(response)
}

fn finalize_postgres_summary_all_response(
    response: &mut StatsResponse,
    augmentation: SummaryLiveAugmentation,
    maintenance: StatsMaintenanceResponse,
) {
    apply_summary_live_augmentation(response, augmentation);
    // All-time SQLite summaries intentionally omit range-only usage detail. Keep the
    // PostgreSQL projection on the same response contract even though its bounded fact
    // aggregate computes usage internally.
    response.usage_breakdown = None;
    response.maintenance = Some(maintenance);
}

pub(crate) async fn query_hourly_backed_summary_since_with_config(
    pool: &Pool<Sqlite>,
    invocation_max_days: u64,
    start: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    query_hourly_backed_summary_range_with_config(
        pool,
        invocation_max_days,
        start,
        Utc::now(),
        source_scope,
    )
    .await
}

pub(crate) async fn query_hourly_backed_summary_range_with_config(
    pool: &Pool<Sqlite>,
    invocation_max_days: u64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    if start >= end {
        return Ok(StatsTotals::default());
    }

    let retention_cutoff = shanghai_retention_cutoff(invocation_max_days);
    // Even when the requested range sits inside the current retention window,
    // live rows alone are not a reliable source of truth: older days may have
    // already been materialized into hourly rollups / archives by an earlier,
    // shorter retention setting. Keep summary reads on the same rollup-backed
    // path as hourly timeseries so mixed archive/live windows stay consistent.
    let mut totals = StatsTotals::default();
    let range_plan = build_hourly_rollup_exact_range_plan(start, end, retention_cutoff)?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let archive_overlap_ids = {
            let mut tx = pool.begin().await?;
            let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
            let rollup_live_cursor =
                load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
            let rows = query_invocation_hourly_rollup_range_tx(
                tx.as_mut(),
                range_start_epoch,
                range_end_epoch,
                source_scope,
            )
            .await?;
            for row in rows {
                totals.total_count += row.total_count;
                totals.success_count += row.success_count;
                totals.failure_count += row.failure_count;
                totals.total_tokens += row.total_tokens;
                totals.total_cost += row.total_cost;
                totals.non_success_cost += row.non_success_cost;
            }
            let mut exact_records = query_invocation_exact_records_tx(
                tx.as_mut(),
                &range_plan,
                source_scope,
                snapshot_id,
            )
            .await?;
            let tail_records = query_invocation_full_hour_tail_records_tx(
                tx.as_mut(),
                &range_plan,
                source_scope,
                rollup_live_cursor,
                snapshot_id,
            )
            .await?;
            let archive_overlap_ids = tail_records
                .iter()
                .map(|record| record.id)
                .collect::<HashSet<_>>();
            exact_records.extend(tail_records);
            for record in &exact_records {
                add_invocation_record_to_summary_totals(&mut totals, record);
            }
            archive_overlap_ids
        };
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid archived summary range start epoch")))?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid archived summary range end epoch")))?;
        totals = totals.add(
            crate::stats::query_unmaterialized_invocation_archive_totals(
                pool,
                source_scope,
                Some((archived_start, archived_end)),
                Some(&archive_overlap_ids),
            )
            .await?,
        );
    } else {
        let snapshot_id = resolve_invocation_snapshot_id(pool, source_scope).await?;
        let exact_records =
            query_invocation_exact_records(pool, &range_plan, source_scope, snapshot_id).await?;
        for record in &exact_records {
            add_invocation_record_to_summary_totals(&mut totals, record);
        }
    }
    Ok(totals)
}

pub(crate) async fn query_hourly_backed_summary_range_for_account_with_config(
    pool: &Pool<Sqlite>,
    invocation_max_days: u64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: i64,
) -> Result<StatsTotals, ApiError> {
    if start >= end {
        return Ok(StatsTotals::default());
    }
    let retention_cutoff = shanghai_retention_cutoff(invocation_max_days);
    let range_plan = build_hourly_rollup_exact_range_plan(start, end, retention_cutoff)?;
    let mut tx = pool.begin().await?;
    let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
    let mut totals = StatsTotals::default();
    let archive_overlap_ids =
        if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
            let rows = query_upstream_account_stats_rollup_range_tx(
                tx.as_mut(),
                "upstream_account_stats_hourly",
                range_start_epoch,
                range_end_epoch,
                source_scope,
                upstream_account_id,
            )
            .await?;
            for row in rows {
                totals.total_count += row.total_count;
                totals.success_count += row.success_count;
                totals.failure_count += row.failure_count;
                totals.total_tokens += row.total_tokens;
                totals.total_cost += row.total_cost;
                totals.non_success_cost += row.non_success_cost;
            }
            let mut exact_records = if !range_plan.live_exact_ranges.is_empty() && snapshot_id > 0 {
                query_invocation_exact_records_for_account_tx(
                    tx.as_mut(),
                    &range_plan,
                    source_scope,
                    snapshot_id,
                    upstream_account_id,
                )
                .await?
            } else {
                Vec::new()
            };
            let tail_records = query_invocation_full_hour_tail_records_tx_for_account(
                tx.as_mut(),
                &range_plan,
                source_scope,
                rollup_live_cursor,
                snapshot_id,
                upstream_account_id,
            )
            .await?;
            let archive_overlap_ids = tail_records
                .iter()
                .map(|record| record.id)
                .collect::<HashSet<_>>();
            exact_records.extend(tail_records);
            for record in &exact_records {
                add_invocation_record_to_summary_totals(&mut totals, record);
            }
            archive_overlap_ids
        } else {
            let exact_records = if !range_plan.live_exact_ranges.is_empty() && snapshot_id > 0 {
                query_invocation_exact_records_for_account_tx(
                    tx.as_mut(),
                    &range_plan,
                    source_scope,
                    snapshot_id,
                    upstream_account_id,
                )
                .await?
            } else {
                Vec::new()
            };
            for record in &exact_records {
                add_invocation_record_to_summary_totals(&mut totals, record);
            }
            HashSet::new()
        };

    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!(
                    "invalid account archived summary range start epoch"
                ))
            })?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid account archived summary range end epoch"))
            })?;
        totals = totals.add(
            crate::stats::query_unmaterialized_upstream_account_archive_totals(
                pool,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
                source_scope,
                Some((archived_start, archived_end)),
                Some(&archive_overlap_ids),
                upstream_account_id,
            )
            .await?,
        );
    }
    Ok(totals)
}

pub(crate) async fn query_hourly_backed_summary_range_for_account(
    state: &AppState,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: i64,
) -> Result<StatsTotals, ApiError> {
    query_hourly_backed_summary_range_for_account_with_config(
        &state.pool,
        state.config.invocation_max_days,
        start,
        end,
        source_scope,
        upstream_account_id,
    )
    .await
}

pub(crate) async fn query_hourly_backed_summary_since(
    state: &AppState,
    start: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    query_hourly_backed_summary_since_with_config(
        &state.pool,
        state.config.invocation_max_days,
        start,
        source_scope,
    )
    .await
}

pub(crate) async fn query_hourly_backed_summary_range(
    state: &AppState,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    query_hourly_backed_summary_range_with_config(
        &state.pool,
        state.config.invocation_max_days,
        start,
        end,
        source_scope,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_summary_all_finalization_preserves_live_response_contract() {
        let mut response = AnalyticsSummaryAggregate::default().into_response(None, false);
        let expected_usage = UsageBreakdownResponse {
            cache_write_tokens: 1,
            cache_read_tokens: 2,
            output_tokens: 3,
            costs: None,
            models: Vec::new(),
        };
        response.usage_breakdown = Some(expected_usage.clone());
        let maintenance = StatsMaintenanceResponse {
            raw_compression_backlog: None,
            startup_backfill: None,
            historical_rollup_backfill: None,
        };
        finalize_postgres_summary_all_response(
            &mut response,
            SummaryLiveAugmentation::test_fixture(),
            maintenance.clone(),
        );

        assert_eq!(response.in_progress_conversation_count, Some(2));
        assert_eq!(response.in_progress_retry_conversation_count, Some(1));
        assert_eq!(response.in_progress_avg_wait_ms, Some(12.5));
        assert_eq!(
            response.in_progress_phase_counts,
            Some(InvocationPhaseCountsResponse {
                queued: 1,
                requesting: 1,
                responding: 0,
            })
        );
        assert_eq!(response.maintenance, Some(maintenance));
        assert_eq!(response.non_success_tokens, None);
        assert_eq!(response.usage_breakdown, None);
    }
}
