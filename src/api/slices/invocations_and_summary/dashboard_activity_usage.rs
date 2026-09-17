async fn query_unmaterialized_archive_usage_breakdown_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    ranges: &[UsageBreakdownArchiveFallbackRange],
    upstream_account_id: Option<i64>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<
    (
        Vec<UpstreamAccountUsageBreakdownAggregateRow>,
        usize,
        &'static str,
        &'static str,
    ),
    ApiError,
> {
    if ranges.is_empty() {
        return Ok((Vec::new(), 0, "none", "none"));
    }

    let archive_lookup_range = ranges
        .iter()
        .map(|fallback_range| fallback_range.range)
        .reduce(|combined, range| ExactUtcRange {
            start: combined.start.min(range.start),
            end: combined.end.max(range.end),
        });
    let archive_rows = crate::stats::load_invocation_archives_missing_effective_rollup_target(
        pool,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
        archive_lookup_range.map(|range| (range.start, range.end)),
    )
    .await?;
    if archive_rows.is_empty() {
        return Ok((Vec::new(), 0, "none", "none"));
    }

    let archive_progress_by_file_path = load_usage_breakdown_archive_progress_by_file_path(
        pool,
        &archive_rows
            .iter()
            .map(|archive_row| archive_row.file_path().to_string())
            .collect::<Vec<_>>(),
    )
    .await?;
    let mut fallback_reason = "archive_rollup_marker_missing";
    let mut legacy_pruned_payload_mode = "none";
    let mut saw_partial_progress = false;
    let mut rows_by_key =
        HashMap::<UsageBreakdownAggregateMergeKey, UpstreamAccountUsageBreakdownAggregateRow>::new(
        );
    for archive_row in &archive_rows {
        let start_after_id = archive_progress_by_file_path
            .get(archive_row.file_path())
            .copied()
            .unwrap_or(0)
            .max(0);
        saw_partial_progress |= start_after_id > 0;
        let Some(batch) = query_unmaterialized_archive_usage_breakdown_batch(
            archive_row,
            ranges,
            source_scope,
            upstream_account_id,
            exclude_invocation_ids,
            start_after_id,
        )
        .await?
        else {
            fallback_reason = "archive_batch_unavailable";
            continue;
        };
        if batch.legacy_pruned_payload {
            legacy_pruned_payload_mode = "structured_rollup_unknown_reasoning";
        }
        for row in batch.rows {
            merge_usage_breakdown_aggregate_row_map(&mut rows_by_key, &row);
        }
    }
    if fallback_reason == "archive_rollup_marker_missing" && saw_partial_progress {
        fallback_reason = "archive_rollup_partial_progress";
    }

    Ok((
        rows_by_key.into_values().collect(),
        archive_rows.len(),
        fallback_reason,
        legacy_pruned_payload_mode,
    ))
}

async fn query_unmaterialized_archive_usage_breakdown_batch(
    archive_row: &crate::stats::ArchiveBatchPathRow,
    ranges: &[UsageBreakdownArchiveFallbackRange],
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    start_after_id: i64,
) -> Result<Option<UsageBreakdownArchiveBatchResult>, ApiError> {
    let Some((archive_pool, temp_cleanup)) =
        crate::stats::open_invocation_archive_batch_pool(archive_row, "usage-breakdown-fallback")
            .await?
    else {
        return Ok(None);
    };
    let legacy_pruned_payload =
        crate::maintenance::invocation_archive_has_pruned_success_details_in_db(&archive_pool)
            .await?;
    let has_cost_breakdown_columns =
        crate::stats::sqlite_table_has_column(&archive_pool, "codex_invocations", "cost_input")
            .await?;
    let exclude_filter = prepare_dashboard_activity_excluded_invocation_ids_filter(
        &archive_pool,
        exclude_invocation_ids,
    )
    .await?;
    let mut rows = Vec::new();
    for fallback_range in ranges {
        rows.extend(
            query_upstream_account_usage_breakdown_rows_from_executor(
                UpstreamAccountUsageBreakdownQueryInput {
                    executor: &archive_pool,
                    source_scope,
                    range: fallback_range.range,
                    has_cost_breakdown_columns,
                    use_attempt_fallback: false,
                    exclude_invocation_ids: exclude_filter,
                    start_after_id: (fallback_range.skip_replayed_prefix && start_after_id > 0)
                        .then_some(start_after_id),
                    snapshot_id: None,
                    upstream_account_id_filter: upstream_account_id,
                },
            )
            .await?,
        );
    }
    archive_pool.close().await;
    drop(temp_cleanup);
    Ok(Some(UsageBreakdownArchiveBatchResult {
        rows,
        legacy_pruned_payload,
    }))
}

struct UsageBreakdownArchiveBatchResult {
    rows: Vec<UpstreamAccountUsageBreakdownAggregateRow>,
    legacy_pruned_payload: bool,
}

fn build_usage_breakdown_archive_fallback_ranges(
    range: ExactUtcRange,
    range_plan: &HourlyRollupExactRangePlan,
    retention_cutoff: DateTime<Utc>,
) -> Result<Vec<UsageBreakdownArchiveFallbackRange>, ApiError> {
    let mut ranges = Vec::new();
    if let Some(full_hour_range) =
        dashboard_activity_full_hour_exact_range(range_plan.full_hour_range)?
    {
        push_usage_breakdown_archive_fallback_range(
            &mut ranges,
            full_hour_range.start,
            full_hour_range.end,
            true,
        )?;
        push_usage_breakdown_archive_fallback_range(
            &mut ranges,
            range.start,
            full_hour_range.start.min(range.end).min(retention_cutoff),
            false,
        )?;
        push_usage_breakdown_archive_fallback_range(
            &mut ranges,
            full_hour_range.end.max(range.start),
            range.end.min(retention_cutoff),
            false,
        )?;
    } else {
        push_usage_breakdown_archive_fallback_range(
            &mut ranges,
            range.start,
            range.end.min(retention_cutoff),
            false,
        )?;
    }
    Ok(ranges)
}

fn push_usage_breakdown_archive_fallback_range(
    ranges: &mut Vec<UsageBreakdownArchiveFallbackRange>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    skip_replayed_prefix: bool,
) -> Result<(), ApiError> {
    if start >= end {
        return Ok(());
    }
    ranges.push(UsageBreakdownArchiveFallbackRange {
        range: ExactUtcRange { start, end },
        skip_replayed_prefix,
    });
    Ok(())
}

async fn load_usage_breakdown_rows_for_range(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    telemetry: UsageBreakdownBuildTelemetry,
) -> Result<UsageBreakdownRowsBuildResult, ApiError> {
    let started_at = Instant::now();
    let has_cost_breakdown_columns =
        crate::stats::sqlite_table_has_column(&state.pool, "codex_invocations", "cost_input")
            .await?;
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let range_plan =
        build_hourly_rollup_exact_range_plan(range.start, range.end, retention_cutoff)?;
    let full_hour_bucket_count = range_plan
        .full_hour_range
        .map(|(start_epoch, end_epoch)| ((end_epoch - start_epoch).max(0) / 3_600) as usize)
        .unwrap_or(0);
    let (mut rows_by_key, partial_hour_row_count, rollup_row_count, archive_overlap_ids) =
        load_usage_breakdown_live_rows_for_range(
            state,
            source_scope,
            upstream_account_id,
            &range_plan,
            has_cost_breakdown_columns,
        )
        .await?;

    let archive_fallback_ranges =
        build_usage_breakdown_archive_fallback_ranges(range, &range_plan, retention_cutoff)?;
    let (archive_rows, archive_batch_count, fallback_reason, legacy_pruned_payload_mode) =
        if archive_fallback_ranges.is_empty() {
            (Vec::new(), 0, "none", "none")
        } else {
            query_unmaterialized_archive_usage_breakdown_rows(
                &state.pool,
                source_scope,
                &archive_fallback_ranges,
                upstream_account_id,
                (!archive_overlap_ids.is_empty()).then_some(&archive_overlap_ids),
            )
            .await?
        };
    for row in &archive_rows {
        merge_usage_breakdown_aggregate_row_map(&mut rows_by_key, row);
    }

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    emit_usage_breakdown_builder_telemetry(UsageBreakdownBuilderTelemetryInput {
        telemetry,
        source_scope,
        full_hour_bucket_count,
        rollup_row_count,
        partial_hour_row_count,
        archive_batch_count,
        fallback_reason,
        legacy_pruned_payload_mode,
        elapsed_ms,
    });

    Ok(UsageBreakdownRowsBuildResult {
        rows: rows_by_key.into_values().collect(),
        full_hour_bucket_count,
        rollup_row_count,
        partial_hour_row_count,
        archive_batch_count,
        fallback_reason,
        legacy_pruned_payload_mode,
    })
}

async fn load_usage_breakdown_live_rows_for_range(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range_plan: &HourlyRollupExactRangePlan,
    has_cost_breakdown_columns: bool,
) -> Result<
    (
        HashMap<UsageBreakdownAggregateMergeKey, UpstreamAccountUsageBreakdownAggregateRow>,
        usize,
        usize,
        HashSet<i64>,
    ),
    ApiError,
> {
    let mut rows_by_key = HashMap::new();
    let mut partial_hour_row_count = 0usize;
    let mut rollup_row_count = 0usize;
    let archive_overlap_ids = HashSet::new();
    let mut tx = state.pool.begin().await?;
    let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let rollup_rows = query_upstream_account_usage_breakdown_hourly_rollup_range_tx(
            tx.as_mut(),
            range_start_epoch,
            range_end_epoch,
            source_scope,
            upstream_account_id,
        )
        .await?;
        rollup_row_count = rollup_rows.len();
        for row in &rollup_rows {
            merge_usage_breakdown_rollup_record_map(&mut rows_by_key, row);
        }
    }
    if snapshot_id == 0 {
        return Ok((
            rows_by_key,
            partial_hour_row_count,
            rollup_row_count,
            archive_overlap_ids,
        ));
    }
    let (live_rows, archive_overlap_ids) = query_usage_breakdown_live_exact_rows(
        tx.as_mut(),
        source_scope,
        upstream_account_id,
        range_plan,
        has_cost_breakdown_columns,
        snapshot_id,
        rollup_live_cursor,
    )
    .await?;
    partial_hour_row_count = usage_breakdown_request_count(&live_rows);
    for row in &live_rows {
        merge_usage_breakdown_aggregate_row_map(&mut rows_by_key, row);
    }

    Ok((
        rows_by_key,
        partial_hour_row_count,
        rollup_row_count,
        archive_overlap_ids,
    ))
}

async fn query_usage_breakdown_live_exact_rows(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range_plan: &HourlyRollupExactRangePlan,
    has_cost_breakdown_columns: bool,
    snapshot_id: i64,
    rollup_live_cursor: i64,
) -> Result<(Vec<UpstreamAccountUsageBreakdownAggregateRow>, HashSet<i64>), ApiError> {
    let mut rows = Vec::new();
    for exact_range in &range_plan.live_exact_ranges {
        rows.extend(
            query_live_upstream_account_usage_breakdown_rows_tx(
                UpstreamAccountUsageBreakdownQueryInput {
                    executor: &mut *connection,
                    source_scope,
                    range: *exact_range,
                    has_cost_breakdown_columns,
                    use_attempt_fallback: true,
                    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter::None,
                    start_after_id: None,
                    snapshot_id: Some(snapshot_id),
                    upstream_account_id_filter: upstream_account_id,
                },
            )
            .await?,
        );
    }
    let Some(full_hour_range) =
        dashboard_activity_full_hour_exact_range(range_plan.full_hour_range)?
    else {
        return Ok((rows, HashSet::new()));
    };
    rows.extend(
        query_live_upstream_account_usage_breakdown_rows_tx(
            UpstreamAccountUsageBreakdownQueryInput {
                executor: &mut *connection,
                source_scope,
                range: full_hour_range,
                has_cost_breakdown_columns,
                use_attempt_fallback: true,
                exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter::None,
                start_after_id: Some(rollup_live_cursor),
                snapshot_id: Some(snapshot_id),
                upstream_account_id_filter: upstream_account_id,
            },
        )
        .await?,
    );
    let archive_overlap_ids = if let Some(account_id) = upstream_account_id {
        query_invocation_full_hour_tail_records_tx_for_account(
            connection,
            range_plan,
            source_scope,
            rollup_live_cursor,
            snapshot_id,
            account_id,
        )
        .await?
        .into_iter()
        .map(|record| record.id)
        .collect()
    } else {
        query_invocation_full_hour_tail_records_tx(
            connection,
            range_plan,
            source_scope,
            rollup_live_cursor,
            snapshot_id,
        )
        .await?
        .into_iter()
        .map(|record| record.id)
        .collect()
    };
    Ok((rows, archive_overlap_ids))
}

pub(crate) async fn load_usage_breakdown_for_range(
    state: &AppState,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: ExactUtcRange,
    telemetry: SummaryRangeBuildTelemetry,
) -> Result<Option<UsageBreakdownResponse>, ApiError> {
    let mut usage_breakdown = UsageBreakdownAccumulator::default();
    let build = load_usage_breakdown_rows_for_range(
        state,
        source_scope,
        upstream_account_id,
        range,
        UsageBreakdownBuildTelemetry::from_summary(telemetry),
    )
    .await?;
    for row in &build.rows {
        usage_breakdown.add_aggregate_row(row);
    }
    Ok(Some(usage_breakdown.into_response()))
}

pub(crate) async fn query_upstream_account_activity_meta(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, UpstreamAccountActivityMetaRow>, ApiError> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT \
            id, kind, display_name, group_name, plan_type, status, enabled, \
            last_error, last_error_at, last_route_failure_at, last_route_failure_kind, \
            last_action_reason_code, last_action_reason_message, cooldown_until, \
            temporary_route_failure_streak_started_at, last_selected_at \
         FROM pool_upstream_accounts WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(*account_id);
        }
    }
    query.push(")");
    Ok(query
        .build_query_as::<UpstreamAccountActivityMetaRow>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| (row.id, row))
        .collect())
}

pub(crate) async fn query_upstream_account_in_progress_counts(
    state: &AppState,
    source_scope: InvocationSourceScope,
) -> Result<HashMap<Option<i64>, UpstreamAccountInProgressSummary>, ApiError> {
    query_upstream_account_in_progress_counts_from_runtime(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        source_scope,
    )
    .await
}

pub(crate) async fn query_upstream_account_in_progress_counts_from_runtime(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    source_scope: InvocationSourceScope,
) -> Result<HashMap<Option<i64>, UpstreamAccountInProgressSummary>, ApiError> {
    let baseline = query_dashboard_runtime_projection_baseline(pool, source_scope).await?;
    query_upstream_account_in_progress_counts_with_baseline(
        pool,
        proxy_runtime_invocations,
        source_scope,
        &baseline,
    )
    .await
}

pub(crate) async fn query_dashboard_runtime_projection_baseline(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<DashboardRuntimeProjectionBaseline, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
        upstream_account_id: Option<i64>,
        retry_count: i64,
        upstream_ttfb_ms: Option<f64>,
        live_phase: Option<String>,
    }

    let resolved_upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("inv");
    let retry_sql = invocation_account_retry_after_failure_with_attempt_fallback_sql(
        resolved_upstream_account_id_sql.as_str(),
        source_scope,
    );
    let final_first_token_timing_sql = final_pool_invocation_timing_sql("inv", "first_token_ms");
    let final_live_phase_sql =
        invocation_live_phase_sql_with_timing_sql("inv", final_first_token_timing_sql.as_str());
    let mut db_key_query = QueryBuilder::<Sqlite>::new(
        "SELECT inv.invoke_id AS invoke_id, inv.occurred_at AS occurred_at, ",
    );
    db_key_query
        .push(resolved_upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(retry_sql.as_str())
        .push(" AS retry_count, live.upstream_ttfb_ms AS upstream_ttfb_ms, ");
    db_key_query.push(final_live_phase_sql.as_str()).push(
        " AS live_phase \
         FROM invocation_in_progress_live live \
         JOIN codex_invocations inv ON inv.id = live.invocation_id \
         WHERE 1 = 1",
    );
    if source_scope == InvocationSourceScope::ProxyOnly {
        db_key_query
            .push(" AND live.source = ")
            .push_bind(SOURCE_PROXY);
    }
    let db_rows = db_key_query
        .build_query_as::<RuntimeKeyRow>()
        .fetch_all(pool)
        .await?;
    Ok(DashboardRuntimeProjectionBaseline {
        records: db_rows
            .into_iter()
            .map(|row| DashboardRuntimeBaselineRecord {
                key: RuntimeInvocationKey::new(row.invoke_id, row.occurred_at),
                upstream_account_id: row.upstream_account_id,
                upstream_account_name: None,
                is_retry: row.retry_count > 0,
                live_phase: row.live_phase,
                wait_ms: row.upstream_ttfb_ms,
            })
            .collect(),
        source_scope,
        network_open_buckets: HashMap::new(),
    })
}

async fn query_upstream_account_in_progress_counts_with_baseline(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    source_scope: InvocationSourceScope,
    baseline: &DashboardRuntimeProjectionBaseline,
) -> Result<HashMap<Option<i64>, UpstreamAccountInProgressSummary>, ApiError> {
    let mut counts = HashMap::<Option<i64>, UpstreamAccountInProgressSummary>::new();
    for row in &baseline.records {
        counts.entry(row.upstream_account_id).or_default().add(
            row.is_retry,
            row.live_phase.as_deref(),
            row.wait_ms,
        );
    }
    let db_runtime_keys = baseline
        .records
        .iter()
        .map(|row| {
            (
                (row.key.invoke_id.clone(), row.key.occurred_at.clone()),
                (
                    row.upstream_account_id,
                    row.is_retry,
                    row.live_phase.clone(),
                    row.wait_ms,
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    let runtime_snapshot = proxy_runtime_invocations.snapshot();
    let db_terminal_keys =
        query_terminal_db_keys_for_runtime_records(pool, &runtime_snapshot, None).await?;
    let mut runtime_overlay_row_count = 0_i64;
    for record in runtime_snapshot {
        if db_terminal_keys.contains(&(record.invoke_id.clone(), record.occurred_at.clone())) {
            continue;
        }
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            continue;
        }
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        let runtime_phase = runtime_record_live_phase(&record);
        if let Some((db_upstream_account_id, db_is_retry, db_phase, db_wait_ms)) =
            db_runtime_keys.get(&key)
        {
            let runtime_is_retry = runtime_record_is_retry(&record);
            let upstream_account_id = record.upstream_account_id.or(*db_upstream_account_id);
            if *db_upstream_account_id != upstream_account_id {
                if let Some(entry) = counts.get_mut(db_upstream_account_id) {
                    entry.subtract(*db_is_retry, db_phase.as_deref(), *db_wait_ms);
                }
                counts.entry(upstream_account_id).or_default().add(
                    runtime_is_retry,
                    runtime_phase,
                    record.t_upstream_ttfb_ms,
                );
                runtime_overlay_row_count += 1;
            } else {
                let entry = counts.entry(upstream_account_id).or_default();
                if runtime_is_retry && !*db_is_retry {
                    entry.retry_count += 1;
                }
                if runtime_phase != db_phase.as_deref() {
                    entry.phase_counts.decrement_phase_name(db_phase.as_deref());
                    entry.phase_counts.increment_phase_name(runtime_phase);
                    runtime_overlay_row_count += 1;
                }
                if runtime_record_ttfb_differs(record.t_upstream_ttfb_ms, *db_wait_ms) {
                    entry.wait_sum_ms = (entry.wait_sum_ms
                        - normalized_wait_ms(*db_wait_ms).unwrap_or_default())
                    .max(0.0)
                        + normalized_wait_ms(record.t_upstream_ttfb_ms).unwrap_or_default();
                    entry.wait_sample_count = entry
                        .wait_sample_count
                        .saturating_sub(i64::from(normalized_wait_ms(*db_wait_ms).is_some()))
                        + i64::from(normalized_wait_ms(record.t_upstream_ttfb_ms).is_some());
                    runtime_overlay_row_count += 1;
                }
            }
            continue;
        }
        counts.entry(record.upstream_account_id).or_default().add(
            runtime_record_is_retry(&record),
            runtime_phase,
            record.t_upstream_ttfb_ms,
        );
        runtime_overlay_row_count += 1;
    }
    emit_runtime_in_progress_overlay_telemetry(runtime_overlay_row_count);
    Ok(counts)
}

fn emit_runtime_in_progress_overlay_telemetry(runtime_overlay_row_count: i64) {
    if runtime_overlay_row_count > 0 {
        debug!(
            endpoint = "/api/upstream-account-activity",
            runtime_overlay_row_count, "overlayed memory runtime account in-progress counts"
        );
    }
}

pub(crate) async fn query_dashboard_activity_live_snapshot(
    state: &AppState,
    revision: u64,
) -> Result<DashboardActivityLiveSnapshot, ApiError> {
    query_dashboard_activity_live_snapshot_from_runtime(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        state.dashboard_network_speed_cache.as_ref(),
        revision,
    )
    .await
}

pub(crate) async fn query_dashboard_activity_live_snapshot_from_runtime(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    revision: u64,
) -> Result<DashboardActivityLiveSnapshot, ApiError> {
    query_dashboard_activity_live_snapshot_with_baseline_from_runtime(
        pool,
        proxy_runtime_invocations,
        dashboard_network_speed_cache,
        revision,
    )
    .await
    .map(|(snapshot, _)| snapshot)
}

pub(crate) async fn query_dashboard_activity_live_snapshot_with_baseline_from_runtime(
    pool: &Pool<Sqlite>,
    proxy_runtime_invocations: &ProxyRuntimeInvocationStore,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    revision: u64,
) -> Result<
    (
        DashboardActivityLiveSnapshot,
        DashboardRuntimeProjectionBaseline,
    ),
    ApiError,
> {
    let now = Utc::now();
    let source_scope = resolve_default_source_scope(pool).await?;
    flush_dashboard_network_socket_minute_rollups(pool, dashboard_network_speed_cache, now).await?;
    let mut baseline = query_dashboard_runtime_projection_baseline(pool, source_scope).await?;
    let counts = query_upstream_account_in_progress_counts_with_baseline(
        pool,
        proxy_runtime_invocations,
        source_scope,
        &baseline,
    )
    .await?;
    let account_rates = dashboard_network_speed_cache.snapshot_account_rates(now);
    let global_realtime_rate = build_dashboard_network_realtime_rate_response(
        dashboard_network_speed_cache
            .snapshot_scope_realtime_bytes(DashboardNetworkScopeKey::Global, now),
    );
    let live_buckets = load_dashboard_activity_live_buckets(
        pool,
        dashboard_network_speed_cache,
        source_scope,
        now,
        &counts,
        &account_rates,
    )
    .await?;
    let accounts = build_dashboard_activity_live_accounts(&counts, &account_rates, &live_buckets);
    let (global_live_bucket, network_open_buckets) = load_dashboard_activity_network_baseline(
        pool,
        dashboard_network_speed_cache,
        source_scope,
        now,
        &live_buckets,
    )
    .await?;
    baseline.network_open_buckets = network_open_buckets;

    let (
        in_progress_invocation_count,
        in_progress_phase_counts,
        retry_invocation_count,
        in_progress_wait_sum_ms,
        in_progress_wait_sample_count,
    ) = summarize_dashboard_activity_live_accounts(&accounts);

    Ok((
        DashboardActivityLiveSnapshot {
            revision,
            generated_at: format_utc_iso(now),
            in_progress_invocation_count,
            in_progress_phase_counts,
            retry_invocation_count,
            in_progress_wait_sum_ms,
            in_progress_wait_sample_count,
            network_live_bucket: Some(global_live_bucket),
            network_realtime_rate: Some(global_realtime_rate),
            accounts,
        },
        baseline,
    ))
}

async fn load_dashboard_activity_live_buckets(
    pool: &Pool<Sqlite>,
    cache: &DashboardNetworkSpeedCache,
    source_scope: InvocationSourceScope,
    now: DateTime<Utc>,
    counts: &HashMap<Option<i64>, UpstreamAccountInProgressSummary>,
    account_rates: &HashMap<Option<i64>, DashboardNetworkRateSnapshot>,
) -> Result<HashMap<Option<i64>, DashboardNetworkTimeseriesPointResponse>, ApiError> {
    let mut account_ids = counts.keys().copied().collect::<Vec<_>>();
    for account_id in account_rates.keys() {
        if !account_ids.contains(account_id) {
            account_ids.push(*account_id);
        }
    }
    let mut live_buckets = HashMap::new();
    for account_id in account_ids {
        live_buckets.insert(
            account_id,
            load_dashboard_network_live_bucket_point(
                pool,
                cache,
                source_scope,
                now,
                DashboardNetworkScopeKey::account_scope(account_id),
            )
            .await?,
        );
    }
    Ok(live_buckets)
}

fn build_dashboard_activity_live_accounts(
    counts: &HashMap<Option<i64>, UpstreamAccountInProgressSummary>,
    account_rates: &HashMap<Option<i64>, DashboardNetworkRateSnapshot>,
    live_buckets: &HashMap<Option<i64>, DashboardNetworkTimeseriesPointResponse>,
) -> Vec<DashboardActivityLiveAccount> {
    let mut accounts = counts
        .iter()
        .map(|(account_id, summary)| {
            build_dashboard_activity_live_account(
                *account_id,
                *summary,
                account_rates.get(account_id).copied().unwrap_or_default(),
                live_buckets.get(account_id).cloned(),
            )
        })
        .collect::<Vec<_>>();
    let existing_keys = accounts
        .iter()
        .map(|account| account.account_key.clone())
        .collect::<HashSet<_>>();
    for (account_id, rate) in account_rates {
        let account_key = dashboard_activity_account_key(*account_id);
        if !existing_keys.contains(&account_key) {
            accounts.push(build_dashboard_activity_live_account(
                *account_id,
                UpstreamAccountInProgressSummary::default(),
                *rate,
                live_buckets.get(account_id).cloned(),
            ));
        }
    }
    accounts.sort_by(|left, right| left.account_key.cmp(&right.account_key));
    accounts
}

fn dashboard_activity_account_key(account_id: Option<i64>) -> String {
    account_id
        .map(|id| format!("upstream:{id}"))
        .unwrap_or_else(|| "unassigned".to_string())
}

fn build_dashboard_activity_live_account(
    account_id: Option<i64>,
    summary: UpstreamAccountInProgressSummary,
    rate: DashboardNetworkRateSnapshot,
    live_bucket: Option<DashboardNetworkTimeseriesPointResponse>,
) -> DashboardActivityLiveAccount {
    DashboardActivityLiveAccount {
        account_key: dashboard_activity_account_key(account_id),
        upstream_account_id: account_id,
        upstream_account_name: None,
        in_progress_invocation_count: summary.in_progress_count,
        in_progress_phase_counts: summary.phase_counts,
        retry_invocation_count: summary.retry_count,
        in_progress_wait_sum_ms: summary.wait_sum_ms,
        in_progress_wait_sample_count: summary.wait_sample_count,
        upload_bytes_per_second: rate.upload_bytes_per_second,
        download_bytes_per_second: rate.download_bytes_per_second,
        network_live_bucket: live_bucket,
    }
}

async fn load_dashboard_activity_network_baseline(
    pool: &Pool<Sqlite>,
    cache: &DashboardNetworkSpeedCache,
    source_scope: InvocationSourceScope,
    now: DateTime<Utc>,
    live_buckets: &HashMap<Option<i64>, DashboardNetworkTimeseriesPointResponse>,
) -> Result<
    (
        DashboardNetworkTimeseriesPointResponse,
        HashMap<DashboardNetworkScopeKey, DashboardRuntimeNetworkOpenBucketBaseline>,
    ),
    ApiError,
> {
    let global_live_bucket = load_dashboard_network_live_bucket_point(
        pool,
        cache,
        source_scope,
        now,
        DashboardNetworkScopeKey::Global,
    )
    .await?;
    let mut network_open_buckets = live_buckets
        .iter()
        .map(|(account_id, live_bucket)| {
            let scope = DashboardNetworkScopeKey::account_scope(*account_id);
            let memory = cache.snapshot_open_bucket(scope, now);
            (
                scope,
                DashboardRuntimeNetworkOpenBucketBaseline {
                    bucket_start: memory.bucket_start,
                    bucket_end: memory.bucket_end,
                    baseline_totals: DashboardNetworkByteTotals {
                        upload_bytes: live_bucket.upload_bytes,
                        download_bytes: live_bucket.download_bytes,
                    },
                    memory_totals_at_install: memory.totals,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let global_memory = cache.snapshot_open_bucket(DashboardNetworkScopeKey::Global, now);
    network_open_buckets.insert(
        DashboardNetworkScopeKey::Global,
        DashboardRuntimeNetworkOpenBucketBaseline {
            bucket_start: global_memory.bucket_start,
            bucket_end: global_memory.bucket_end,
            baseline_totals: DashboardNetworkByteTotals {
                upload_bytes: global_live_bucket.upload_bytes,
                download_bytes: global_live_bucket.download_bytes,
            },
            memory_totals_at_install: global_memory.totals,
        },
    );
    Ok((global_live_bucket, network_open_buckets))
}

fn summarize_dashboard_activity_live_accounts(
    accounts: &[DashboardActivityLiveAccount],
) -> (i64, InvocationPhaseCountsResponse, i64, f64, i64) {
    let mut phase_counts = InvocationPhaseCountsResponse::default();
    let mut invocation_count = 0;
    let mut retry_count = 0;
    let mut wait_sum_ms = 0.0;
    let mut wait_sample_count = 0;
    for account in accounts {
        invocation_count += account.in_progress_invocation_count;
        retry_count += account.retry_invocation_count;
        wait_sum_ms += account.in_progress_wait_sum_ms;
        wait_sample_count += account.in_progress_wait_sample_count;
        phase_counts.queued += account.in_progress_phase_counts.queued;
        phase_counts.requesting += account.in_progress_phase_counts.requesting;
        phase_counts.responding += account.in_progress_phase_counts.responding;
    }
    (
        invocation_count,
        phase_counts,
        retry_count,
        wait_sum_ms,
        wait_sample_count,
    )
}
