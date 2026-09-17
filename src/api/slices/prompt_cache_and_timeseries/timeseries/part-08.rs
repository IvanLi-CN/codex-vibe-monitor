async fn build_timeseries_account_hourly_rollup_baseline(
    state: &AppState,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: &RangeWindow,
    bucket_selection: &TimeseriesBucketSelection,
    upstream_account_id: i64,
) -> Result<TimeseriesHourlyRollupBaseline, ApiError> {
    let bucket_seconds = bucket_selection.bucket_seconds;
    debug_assert!(bucket_seconds >= 3_600);
    let range_plan = build_hourly_rollup_exact_range_plan(
        range_window.start,
        range_window.end,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    )?;
    let mut aggregates = BTreeMap::new();
    fill_timeseries_buckets(
        &mut aggregates,
        range_window.start,
        range_window.end,
        bucket_seconds,
        reporting_tz,
    )?;

    let (snapshot_id, hourly_rows, exact_records, archive_overlap_ids) =
        load_account_hourly_baseline_rows(
            state,
            &range_plan,
            range_window,
            source_scope,
            upstream_account_id,
        )
        .await?;

    add_rollup_rows_to_timeseries_aggregates(
        &mut aggregates,
        hourly_rows,
        bucket_seconds,
        reporting_tz,
    )?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid account archived timeseries start epoch"))
            })?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid account archived timeseries end epoch"))
            })?;
        let archived_rows =
            crate::stats::query_unmaterialized_upstream_account_archive_hourly_rollup_deltas(
                &state.pool,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
                source_scope,
                Some((archived_start, archived_end)),
                Some(&archive_overlap_ids),
                upstream_account_id,
            )
            .await?;
        add_rollup_rows_to_timeseries_aggregates(
            &mut aggregates,
            archived_rows,
            bucket_seconds,
            reporting_tz,
        )?;
    }
    add_terminal_timeseries_records(&mut aggregates, exact_records, bucket_seconds, reporting_tz)?;

    Ok(TimeseriesHourlyRollupBaseline {
        snapshot_id,
        aggregates,
    })
}

async fn load_account_hourly_baseline_rows(
    state: &AppState,
    range_plan: &HourlyRollupExactRangePlan,
    range_window: &RangeWindow,
    source_scope: InvocationSourceScope,
    upstream_account_id: i64,
) -> Result<
    (
        i64,
        Vec<UpstreamAccountStatsRollupRecord>,
        Vec<InvocationAggregateRecord>,
        HashSet<i64>,
    ),
    ApiError,
> {
    let mut tx = state.pool.begin().await?;
    let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
    let hourly_rows = if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range
    {
        query_upstream_account_stats_rollup_range_tx(
            tx.as_mut(),
            "upstream_account_stats_hourly",
            range_start_epoch,
            range_end_epoch,
            source_scope,
            upstream_account_id,
        )
        .await?
    } else {
        Vec::new()
    };
    let mut exact_records = Vec::new();
    let boundary_snapshot_id = rollup_live_cursor.min(snapshot_id);
    if !range_plan.live_exact_ranges.is_empty() && boundary_snapshot_id > 0 {
        exact_records.extend(
            query_invocation_exact_records_for_account_tx(
                tx.as_mut(),
                range_plan,
                source_scope,
                boundary_snapshot_id,
                upstream_account_id,
            )
            .await?,
        );
    }
    let mut archive_overlap_ids = HashSet::new();
    if rollup_live_cursor < snapshot_id {
        let tail_range_plan = HourlyRollupExactRangePlan {
            full_hour_range: None,
            live_exact_ranges: exact_utc_range(range_window.start, range_window.end)?
                .into_iter()
                .collect(),
        };
        let tail_records = query_invocation_exact_records_tx_for_account(
            tx.as_mut(),
            &tail_range_plan,
            source_scope,
            snapshot_id,
            upstream_account_id,
            rollup_live_cursor,
        )
        .await?;
        archive_overlap_ids.extend(tail_records.iter().map(|record| record.id));
        exact_records.extend(tail_records);
    }
    Ok((snapshot_id, hourly_rows, exact_records, archive_overlap_ids))
}

pub(crate) async fn fetch_timeseries_from_hourly_rollups(
    state: Arc<AppState>,
    _params: TimeseriesQuery,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: RangeWindow,
    bucket_selection: TimeseriesBucketSelection,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let baseline = build_timeseries_hourly_rollup_baseline(
        state.as_ref(),
        reporting_tz,
        source_scope,
        &range_window,
        &bucket_selection,
        true,
    )
    .await?;
    let fill_start_epoch = align_reporting_bucket_epoch(
        range_window.start.timestamp(),
        bucket_selection.bucket_seconds,
        reporting_tz,
    )?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(
        range_window.end,
        bucket_selection.bucket_seconds,
        reporting_tz,
    )?;
    build_timeseries_response(TimeseriesResponseInput {
        start_dt: range_window.start,
        end_dt: range_window.display_end,
        bucket_seconds: bucket_selection.bucket_seconds,
        snapshot_id: baseline.snapshot_id,
        bucket_selection,
        aggregates: baseline.aggregates,
        fill_start_epoch,
        fill_end_epoch,
        reporting_tz,
    })
}
fn timeseries_projection_requires_exact_rebuild(
    deltas: &[(i64, TimeseriesTerminalDelta)],
    existing_max_row_id: i64,
) -> bool {
    let mut seen_row_ids = HashSet::with_capacity(deltas.len());
    deltas
        .iter()
        .any(|(row_id, _)| *row_id <= existing_max_row_id || !seen_row_ids.insert(*row_id))
}
