use super::*;

pub(crate) async fn query_hourly_backed_summary_since_with_config(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    invocation_max_days: u64,
    start: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    query_hourly_backed_summary_range_with_config(
        pool,
        relay,
        invocation_max_days,
        start,
        Utc::now(),
        source_scope,
    )
    .await
}

pub(crate) async fn query_hourly_backed_summary_range_with_config(
    pool: &Pool<Sqlite>,
    relay: Option<&CrsStatsConfig>,
    invocation_max_days: u64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    if start >= end {
        return Ok(StatsTotals::default());
    }

    let retention_cutoff = shanghai_retention_cutoff(invocation_max_days);
    if start >= retention_cutoff {
        return query_combined_totals(pool, relay, StatsFilter::Range(start, end), source_scope)
            .await
            .map_err(Into::into);
    }

    let mut totals = StatsTotals::default();
    let range_plan = build_hourly_rollup_exact_range_plan(start, end, retention_cutoff)?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        ensure_invocation_summary_rollups_ready_best_effort(pool).await?;
        let mut tx = pool.begin().await?;
        let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
        let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
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
        }
        let mut exact_records =
            query_invocation_exact_records_tx(tx.as_mut(), &range_plan, source_scope, snapshot_id)
                .await?;
        exact_records.extend(
            query_invocation_full_hour_tail_records_tx(
                tx.as_mut(),
                &range_plan,
                source_scope,
                rollup_live_cursor,
                snapshot_id,
            )
            .await?,
        );
        for record in &exact_records {
            add_invocation_record_to_summary_totals(&mut totals, record);
        }
    } else {
        let snapshot_id = resolve_invocation_snapshot_id(pool, source_scope).await?;
        let exact_records =
            query_invocation_exact_records(pool, &range_plan, source_scope, snapshot_id).await?;
        for record in &exact_records {
            add_invocation_record_to_summary_totals(&mut totals, record);
        }
    }
    let relay_totals =
        if let Some(effective_range) = effective_range_for_hourly_rollup_plan(&range_plan)? {
            query_crs_totals(
                pool,
                relay,
                &StatsFilter::Range(effective_range.start, effective_range.end),
                source_scope,
            )
            .await?
        } else {
            StatsTotals::default()
        };
    Ok(totals.add(relay_totals))
}

pub(crate) async fn query_hourly_backed_summary_since(
    state: &AppState,
    start: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    query_hourly_backed_summary_since_with_config(
        &state.pool,
        state.config.crs_stats.as_ref(),
        state.config.invocation_max_days,
        start,
        source_scope,
    )
    .await
    .map_err(Into::into)
}

pub(crate) async fn query_hourly_backed_summary_range(
    state: &AppState,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals, ApiError> {
    query_hourly_backed_summary_range_with_config(
        &state.pool,
        state.config.crs_stats.as_ref(),
        state.config.invocation_max_days,
        start,
        end,
        source_scope,
    )
    .await
    .map_err(Into::into)
}
