struct DashboardActivityAccountBaseBuild {
    retention_cutoff: chrono::DateTime<Utc>,
    model_performance_available: bool,
    source_scope: InvocationSourceScope,
    account_activity: HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    model_performance: ModelPerformanceAccumulator,
    materialized_archive_fallback_totals: StatsTotals,
    materialized_archive_details_limited: bool,
    current_snapshot_by_account: HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
    current_snapshot_summary: DashboardActivityCurrentSnapshot,
    model_performance_duration_overrides: Option<ModelPerformanceDurationOverrides>,
    account_aggregation_telemetry: AccountActivityRangeBuildTelemetry,
}

async fn load_dashboard_activity_usage_breakdown(
    state: &AppState,
    range_name: &str,
    range: ExactUtcRange,
    source_scope: InvocationSourceScope,
    builder_kind: DashboardActivityAccountBuilderKind,
    account_activity: &mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    model_performance: &mut ModelPerformanceAccumulator,
) -> Result<(), ApiError> {
    let rows = load_usage_breakdown_rows_for_range(
        state,
        source_scope,
        None,
        range,
        UsageBreakdownBuildTelemetry::from_range_route(
            if builder_kind == DashboardActivityAccountBuilderKind::UpstreamAccount {
                "upstream_account"
            } else {
                "dashboard"
            },
            range_name,
        ),
    )
    .await?
    .rows;
    for row in &rows {
        let entry = account_activity.entry(row.upstream_account_id).or_default();
        entry.usage_breakdown.add_aggregate_row(row);
        entry.model_performance.add_aggregate_row(row);
        model_performance.add_aggregate_row(row);
    }
    Ok(())
}

async fn load_dashboard_activity_account_base(
    state: &AppState,
    range_name: &str,
    range: ExactUtcRange,
    builder_kind: DashboardActivityAccountBuilderKind,
) -> Result<DashboardActivityAccountBaseBuild, ApiError> {
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let model_performance_available = range.start >= retention_cutoff;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let mut account_activity = HashMap::<Option<i64>, UpstreamAccountActivityAccumulator>::new();
    let mut model_performance = ModelPerformanceAccumulator::default();
    let current_snapshot_by_account = if range_name == "yesterday" {
        load_dashboard_activity_current_minute_accumulators_by_account(state, source_scope, range)
            .await?
            .into_iter()
            .map(|(upstream_account_id, accumulator)| {
                (upstream_account_id, accumulator.into_current_snapshot())
            })
            .collect::<HashMap<_, _>>()
    } else {
        state
            .dashboard_network_speed_cache
            .snapshot_dashboard_activity_accounts(range.end)
    };
    let current_snapshot_summary =
        sum_dashboard_activity_current_snapshots(current_snapshot_by_account.values().copied());
    let model_performance_duration_overrides = if builder_kind
        == DashboardActivityAccountBuilderKind::DashboardFull
        && model_performance_available
    {
        Some(
            query_live_model_performance_duration_overrides(&state.pool, source_scope, range, true)
                .await?,
        )
    } else {
        None
    };
    let account_range_build = if range_name == "yesterday" {
        AccountActivityRangeBuild {
            rows: query_live_upstream_account_activity_aggregate_rows(
                &state.pool,
                source_scope,
                range,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?,
            telemetry: AccountActivityRangeBuildTelemetry {
                raw_fallback_range_count: 1,
                ..AccountActivityRangeBuildTelemetry::default()
            },
        }
    } else {
        load_upstream_account_activity_range_rows(
            state,
            source_scope,
            range,
            if builder_kind == DashboardActivityAccountBuilderKind::UpstreamAccount {
                "upstream_account"
            } else {
                "dashboard"
            },
        )
        .await?
    };
    let account_aggregation_telemetry = account_range_build.telemetry;
    for row in account_range_build.rows {
        let entry = account_activity.entry(row.upstream_account_id).or_default();
        merge_upstream_account_activity_aggregate_row(entry, &row);
    }
    load_dashboard_activity_usage_breakdown(
        state,
        range_name,
        range,
        source_scope,
        builder_kind,
        &mut account_activity,
        &mut model_performance,
    )
    .await?;
    Ok(DashboardActivityAccountBaseBuild {
        retention_cutoff,
        model_performance_available,
        source_scope,
        account_activity,
        model_performance,
        materialized_archive_fallback_totals: StatsTotals::default(),
        materialized_archive_details_limited: false,
        current_snapshot_summary,
        current_snapshot_by_account,
        model_performance_duration_overrides,
        account_aggregation_telemetry,
    })
}

fn merge_dashboard_activity_archive_aggregate_rows(
    account_activity: &mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    archived_aggregates: impl IntoIterator<Item = UpstreamAccountActivityAggregateRow>,
) {
    for row in archived_aggregates {
        let entry = account_activity.entry(row.upstream_account_id).or_default();
        merge_latest_optional_timestamp(
            &mut entry.latest_conversation_created_at,
            row.latest_conversation_created_at,
        );
        merge_latest_optional_timestamp(&mut entry.last_invocation_at, row.last_invocation_at);
        entry.request_count += row.request_count;
        entry.success_count += row.success_count;
        entry.failure_count += row.failure_count;
        entry.non_success_count += row.non_success_count;
        entry.total_tokens += row.total_tokens;
        entry.success_tokens += row.success_tokens;
        entry.non_success_tokens += row.non_success_tokens;
        entry.failure_tokens += row.failure_tokens;
        entry.failure_cost += row.failure_cost;
        entry.non_success_cost += row.non_success_cost;
        entry.cache_input_tokens += row.cache_input_tokens;
        entry.total_cost += row.total_cost;
        entry.first_response_byte_total_sample_count += row.first_response_byte_total_sample_count;
        entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
        entry.first_token_sample_count += row.first_token_sample_count;
        entry.first_token_sum_ms += row.first_token_sum_ms;
        entry.total_latency_sample_count += row.total_latency_sample_count;
        entry.total_latency_sum_ms += row.total_latency_sum_ms;
        merge_latest_timed_metric(
            &mut entry.latest_first_response_byte_total_at,
            &mut entry.latest_first_response_byte_total_ms,
            row.latest_first_response_byte_total_at,
            row.latest_first_response_byte_total_ms,
        );
        merge_latest_timed_metric(
            &mut entry.latest_avg_total_at,
            &mut entry.latest_avg_total_ms,
            row.latest_avg_total_at,
            row.latest_avg_total_ms,
        );
    }
}

async fn merge_dashboard_activity_archive_fallbacks(
    state: &AppState,
    source_scope: InvocationSourceScope,
    skipped_materialized_ranges: &[ExactUtcRange],
    builder_kind: DashboardActivityAccountBuilderKind,
    account_activity: &mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    materialized_archive_details_limited: &mut bool,
) -> Result<(), ApiError> {
    let global_fallback_totals = dashboard_activity_materialized_archive_fallback_totals(
        state,
        source_scope,
        skipped_materialized_ranges.to_vec(),
    )
    .await?;
    let (global_first_token_count, global_first_token_sum_ms) =
        dashboard_activity_materialized_archive_first_token_fallback_totals(
            state,
            source_scope,
            skipped_materialized_ranges,
        )
        .await?;
    if !dashboard_activity_stats_totals_has_values(global_fallback_totals)
        && global_first_token_count == 0
    {
        return Ok(());
    }
    if builder_kind == DashboardActivityAccountBuilderKind::DashboardFull {
        *materialized_archive_details_limited = true;
    }
    let mut account_fallback_totals =
        dashboard_activity_materialized_archive_account_fallback_totals(
            state,
            source_scope,
            skipped_materialized_ranges,
        )
        .await?;
    let account_fallback_sum = account_fallback_totals
        .values()
        .fold(StatsTotals::default(), |total, account_totals| {
            total.add(account_totals.stats_totals())
        });
    let residual_fallback_totals =
        dashboard_activity_stats_totals_subtract(global_fallback_totals, account_fallback_sum);
    let (residual_first_token_count, residual_first_token_sum_ms) =
        dashboard_activity_residual_first_token_totals(
            global_first_token_count,
            global_first_token_sum_ms,
            &account_fallback_totals,
        );
    if dashboard_activity_stats_totals_has_values(residual_fallback_totals)
        || residual_first_token_count > 0
    {
        let mut residual =
            DashboardActivityAccountFallbackTotals::from_stats_totals(residual_fallback_totals);
        residual.first_token_sample_count = residual_first_token_count;
        residual.first_token_sum_ms = residual_first_token_sum_ms;
        account_fallback_totals
            .entry(None)
            .or_default()
            .add_assign(residual);
    }
    for (upstream_account_id, totals) in account_fallback_totals {
        dashboard_activity_merge_account_fallback_totals(
            account_activity.entry(upstream_account_id).or_default(),
            totals,
        );
    }
    Ok(())
}

async fn merge_dashboard_activity_archive_data(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    retention_cutoff: chrono::DateTime<Utc>,
    builder_kind: DashboardActivityAccountBuilderKind,
    account_activity: &mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    materialized_archive_details_limited: &mut bool,
) -> Result<(), ApiError> {
    if range.start >= retention_cutoff {
        return Ok(());
    }
    let archive_rows = query_completed_invocation_archive_activity_aggregate_rows(
        &state.pool,
        source_scope,
        range,
    )
    .await?;
    let QueryCompletedInvocationArchiveActivityAggregateRows {
        aggregates,
        skipped_materialized_ranges,
    } = archive_rows;
    merge_dashboard_activity_archive_aggregate_rows(account_activity, aggregates);
    if skipped_materialized_ranges.is_empty() {
        return Ok(());
    }
    merge_dashboard_activity_archive_fallbacks(
        state,
        source_scope,
        &skipped_materialized_ranges,
        builder_kind,
        account_activity,
        materialized_archive_details_limited,
    )
    .await
}

fn apply_dashboard_activity_model_performance_overrides(
    account_activity: &mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    model_performance: &mut ModelPerformanceAccumulator,
    overrides: Option<ModelPerformanceDurationOverrides>,
) {
    let Some(overrides) = overrides else {
        return;
    };
    model_performance.wall_clock_usage_duration_ms = overrides.total_wall_clock_ms;
    for (group, wall_clock_usage_duration_ms) in overrides.by_group_wall_clock_ms {
        if let Some(entry) = model_performance.models.get_mut(&group) {
            entry.wall_clock_usage_duration_ms = Some(wall_clock_usage_duration_ms);
        }
    }
    for (upstream_account_id, wall_clock_usage_duration_ms) in overrides.by_account_wall_clock_ms {
        if let Some(entry) = account_activity.get_mut(&upstream_account_id) {
            entry.model_performance.wall_clock_usage_duration_ms =
                Some(wall_clock_usage_duration_ms);
        }
    }
    for (key, wall_clock_usage_duration_ms) in overrides.by_account_group_wall_clock_ms {
        if let Some(entry) = account_activity.get_mut(&key.upstream_account_id)
            && let Some(model_entry) = entry.model_performance.models.get_mut(&key.group)
        {
            model_entry.wall_clock_usage_duration_ms = Some(wall_clock_usage_duration_ms);
        }
    }
}

async fn load_dashboard_activity_rate_events(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    account_activity: &mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
) -> Result<(), ApiError> {
    for row in
        query_live_upstream_account_activity_rate_rows(&state.pool, source_scope, range).await?
    {
        if let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) {
            account_activity
                .entry(row.upstream_account_id)
                .or_default()
                .rate_usage_events
                .push(UpstreamAccountRateUsageEvent {
                    occurred_at_epoch_ms: occurred_at.timestamp_millis(),
                    total_tokens: row.total_tokens.max(0),
                    total_cost: row.total_cost.max(0.0),
                });
        }
    }
    Ok(())
}

struct DashboardActivityRecentRowsInput<'a> {
    state: &'a AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    recent_limit: usize,
    include_recent: bool,
    retention_cutoff: chrono::DateTime<Utc>,
    builder_kind: DashboardActivityAccountBuilderKind,
    account_activity: &'a mut HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    build_telemetry: &'a mut DashboardActivityBuildTelemetry,
}

async fn load_dashboard_activity_recent_rows(
    input: DashboardActivityRecentRowsInput<'_>,
) -> Result<(), ApiError> {
    let DashboardActivityRecentRowsInput {
        state,
        source_scope,
        range,
        recent_limit,
        include_recent,
        retention_cutoff,
        builder_kind,
        account_activity,
        build_telemetry,
    } = input;
    if !include_recent {
        return Ok(());
    }
    let hydrated_rows =
        query_live_upstream_account_activity_preview_rows_per_account_limit_with_stats(
            &state.pool,
            source_scope,
            range,
            recent_limit,
            builder_kind.preview_read_telemetry(),
        )
        .await?;
    *build_telemetry = DashboardActivityBuildTelemetry {
        preview_read_mode: "bounded_per_account",
        candidate_preview_id_count: hydrated_rows.candidate_preview_id_count,
        hydrated_preview_row_count: hydrated_rows.hydrated_preview_row_count,
        account_aggregation: build_telemetry.account_aggregation,
    };
    let mut recent_rows_by_account =
        HashMap::<Option<i64>, Vec<PromptCacheConversationInvocationPreviewResponse>>::new();
    let mut recent_rows = hydrated_rows.rows;
    overlay_runtime_upstream_account_activity_preview_rows(
        state,
        &mut recent_rows,
        source_scope,
        range,
    );
    overlay_runtime_terminal_upstream_account_activity_preview_rows(
        state,
        &mut recent_rows,
        source_scope,
        range,
    )
    .await?;
    let live_ids = recent_rows.iter().map(|row| row.id).collect::<HashSet<_>>();
    for row in recent_rows {
        merge_upstream_account_activity_recent_row_metadata(account_activity, &row);
        recent_rows_by_account
            .entry(row.upstream_account_id)
            .or_default()
            .push(upstream_account_invocation_preview_from_row(row));
    }
    if range.start < retention_cutoff {
        let mut archived_rows = crate::stats::query_completed_invocation_archive_preview_rows(
            &state.pool,
            source_scope,
            range,
            Some(&live_ids),
        )
        .await?;
        let archived_row_ids = archived_rows.iter().map(|row| row.id).collect::<Vec<_>>();
        let persisted_live_ids = query_live_upstream_account_activity_existing_invocation_ids(
            &state.pool,
            source_scope,
            &archived_row_ids,
        )
        .await?;
        archived_rows.retain(|row| !persisted_live_ids.contains(&row.id));
        for row in archived_rows {
            merge_upstream_account_activity_recent_row_metadata(account_activity, &row);
            recent_rows_by_account
                .entry(row.upstream_account_id)
                .or_default()
                .push(upstream_account_invocation_preview_from_row(row));
        }
    }
    for (upstream_account_id, mut recent_rows) in recent_rows_by_account {
        let entry = account_activity.entry(upstream_account_id).or_default();
        recent_rows.append(&mut entry.recent_invocations);
        let mut seen_keys = HashSet::with_capacity(recent_rows.len());
        recent_rows.retain(|invocation| {
            seen_keys.insert((invocation.invoke_id.clone(), invocation.occurred_at.clone()))
        });
        recent_rows.sort_by(|left, right| {
            right
                .occurred_at
                .cmp(&left.occurred_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        recent_rows.truncate(recent_limit);
        entry.recent_invocations = recent_rows;
    }
    Ok(())
}

struct DashboardActivityAccountResponseBuildInput<'a> {
    range_name: &'a str,
    range: ExactUtcRange,
    model_performance_available: bool,
    current_snapshot_by_account: &'a HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
    account_activity: HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
    model_performance: ModelPerformanceAccumulator,
    in_progress_counts: HashMap<Option<i64>, UpstreamAccountInProgressSummary>,
    account_meta: &'a HashMap<i64, UpstreamAccountActivityMetaRow>,
    effective_routing_rules: &'a HashMap<i64, crate::upstream_accounts::EffectiveRoutingRule>,
}

async fn load_dashboard_activity_account_metadata(
    pool: &Pool<Sqlite>,
    account_activity: &HashMap<Option<i64>, UpstreamAccountActivityAccumulator>,
) -> Result<
    (
        HashMap<i64, UpstreamAccountActivityMetaRow>,
        HashMap<i64, crate::upstream_accounts::EffectiveRoutingRule>,
    ),
    ApiError,
> {
    let account_ids = account_activity
        .keys()
        .filter_map(|id| *id)
        .collect::<Vec<_>>();
    let account_meta = query_upstream_account_activity_meta(pool, &account_ids).await?;
    let effective_routing_rules =
        crate::upstream_accounts::load_effective_routing_rules_for_accounts(pool, &account_ids)
            .await?;
    Ok((account_meta, effective_routing_rules))
}

fn build_dashboard_activity_account_responses(
    input: DashboardActivityAccountResponseBuildInput<'_>,
) -> (
    Vec<DashboardActivityAccountResponse>,
    Option<f64>,
    Option<f64>,
    ModelPerformanceResponse,
    HashMap<Option<i64>, ModelPerformanceAccumulator>,
    HashMap<Option<i64>, DashboardActivityAccountLatencyAccumulator>,
) {
    let DashboardActivityAccountResponseBuildInput {
        range_name,
        range,
        model_performance_available,
        current_snapshot_by_account,
        account_activity,
        model_performance,
        in_progress_counts,
        account_meta,
        effective_routing_rules,
    } = input;
    let mut latest_first_response_byte_total_in_range = LatestTimedMetricValue::default();
    let mut latest_avg_total_in_range = LatestTimedMetricValue::default();
    for aggregate in account_activity.values() {
        latest_first_response_byte_total_in_range.update(
            aggregate.latest_first_response_byte_total_at.clone(),
            aggregate.latest_first_response_byte_total_ms,
        );
        latest_avg_total_in_range.update(
            aggregate.latest_avg_total_at.clone(),
            aggregate.latest_avg_total_ms,
        );
    }
    let account_model_performance_accumulators = account_activity
        .iter()
        .map(|(upstream_account_id, aggregate)| {
            (*upstream_account_id, aggregate.model_performance.clone())
        })
        .collect();
    let account_latency_accumulators = account_activity
        .iter()
        .map(|(upstream_account_id, aggregate)| {
            (
                *upstream_account_id,
                DashboardActivityAccountLatencyAccumulator::from_aggregate(aggregate),
            )
        })
        .collect();
    let mut accounts = account_activity
        .into_iter()
        .map(|(upstream_account_id, aggregate)| {
            build_dashboard_activity_account_response(DashboardActivityAccountResponseInput {
                range_name,
                range,
                model_performance_available,
                current_snapshot_by_account,
                account_meta,
                effective_routing_rules,
                in_progress_counts: &in_progress_counts,
                upstream_account_id,
                aggregate,
            })
        })
        .collect::<Vec<_>>();
    sort_dashboard_activity_accounts(&mut accounts);
    (
        accounts,
        latest_first_response_byte_total_in_range.value,
        latest_avg_total_in_range.value,
        model_performance.into_response(range, model_performance_available),
        account_model_performance_accumulators,
        account_latency_accumulators,
    )
}

async fn load_dashboard_activity_account_build_result(
    state: &AppState,
    range_name: &str,
    range: ExactUtcRange,
    recent_limit: usize,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
    builder_kind: DashboardActivityAccountBuilderKind,
) -> Result<DashboardActivityAccountBuildResult, ApiError> {
    let mut base =
        load_dashboard_activity_account_base(state, range_name, range, builder_kind).await?;
    merge_dashboard_activity_archive_data(
        state,
        base.source_scope,
        range,
        base.retention_cutoff,
        builder_kind,
        &mut base.account_activity,
        &mut base.materialized_archive_details_limited,
    )
    .await?;
    apply_dashboard_activity_model_performance_overrides(
        &mut base.account_activity,
        &mut base.model_performance,
        base.model_performance_duration_overrides.take(),
    );
    let mut build_telemetry = DashboardActivityBuildTelemetry {
        account_aggregation: base.account_aggregation_telemetry,
        ..DashboardActivityBuildTelemetry::default()
    };
    load_dashboard_activity_rate_events(
        state,
        base.source_scope,
        range,
        &mut base.account_activity,
    )
    .await?;
    load_dashboard_activity_recent_rows(DashboardActivityRecentRowsInput {
        state,
        source_scope: base.source_scope,
        range,
        recent_limit,
        include_recent,
        retention_cutoff: base.retention_cutoff,
        builder_kind,
        account_activity: &mut base.account_activity,
        build_telemetry: &mut build_telemetry,
    })
    .await?;
    let in_progress_counts = if range_name == "yesterday" {
        HashMap::new()
    } else {
        match in_progress_counts_override {
            Some(counts) => counts,
            None => query_upstream_account_in_progress_counts(state, base.source_scope).await?,
        }
    };
    for upstream_account_id in in_progress_counts.keys() {
        base.account_activity
            .entry(*upstream_account_id)
            .or_default();
    }
    let (account_meta, effective_routing_rules) =
        load_dashboard_activity_account_metadata(&state.pool, &base.account_activity).await?;
    let summary_model_performance_accumulator = base.model_performance.clone();
    let (
        accounts,
        latest_first_response_byte_total_in_range,
        latest_avg_total_in_range,
        summary_model_performance,
        account_model_performance_accumulators,
        account_latency_accumulators,
    ) = build_dashboard_activity_account_responses(DashboardActivityAccountResponseBuildInput {
        range_name,
        range,
        model_performance_available: base.model_performance_available,
        current_snapshot_by_account: &base.current_snapshot_by_account,
        account_activity: base.account_activity,
        model_performance: base.model_performance,
        in_progress_counts,
        account_meta: &account_meta,
        effective_routing_rules: &effective_routing_rules,
    });
    Ok(DashboardActivityAccountBuildResult {
        accounts,
        current_snapshot_summary: base.current_snapshot_summary,
        latest_first_response_byte_total_in_range,
        latest_avg_total_in_range,
        summary_model_performance,
        summary_model_performance_accumulator,
        account_model_performance_accumulators,
        account_latency_accumulators,
        materialized_archive_fallback_totals: base.materialized_archive_fallback_totals,
        materialized_archive_details_limited: base.materialized_archive_details_limited,
        build_telemetry,
    })
}
