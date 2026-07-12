use super::*;
use anyhow::anyhow;

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
