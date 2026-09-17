struct SummaryProjectionFinalizationRanges {
    unavailable_unmaterialized_archive_ranges: Vec<ExactUtcRange>,
    unavailable_boundary_archive_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_ranges: Vec<ExactUtcRange>,
    unavailable_boundary_archive_account_ranges: Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_current_ranges: Vec<ExactUtcRange>,
    unavailable_exact_live_ranges: Vec<ExactUtcRange>,
    unavailable_exact_live_account_ranges: HashMap<i64, Vec<ExactUtcRange>>,
    current_archive_latest_coverage_end: Option<DateTime<Utc>>,
    current_archive_has_unknown_coverage: bool,
    current_account_source_unavailable: bool,
}

struct SummaryProjectionFinalizationRangesInput<'a> {
    pool: &'a Pool<Sqlite>,
    exact_horizon: ExactUtcRange,
    list_limit_max: usize,
    current_complete_archive_paths: &'a HashSet<String>,
    current_records: &'a [SummaryProjectionRecord],
    unavailable_unmaterialized_archive_buckets: &'a BTreeSet<i64>,
    unavailable_unmaterialized_archive_exact_ranges: &'a [ExactUtcRange],
    unavailable_boundary_archive_ranges: &'a [ExactUtcRange],
    unavailable_unmaterialized_archive_current_ranges: &'a [ExactUtcRange],
    unavailable_unmaterialized_archive_account_buckets: &'a BTreeSet<i64>,
    unavailable_unmaterialized_archive_account_exact_ranges: &'a [ExactUtcRange],
    unavailable_boundary_archive_account_ranges: &'a [ExactUtcRange],
    unavailable_unmaterialized_archive_account_current_ranges: &'a [ExactUtcRange],
    unavailable_exact_live_buckets: &'a BTreeSet<i64>,
    unavailable_exact_live_account_buckets: &'a HashMap<i64, BTreeSet<i64>>,
    current_archive_admission_exceeded: bool,
    mode: SummaryProjectionBuildMode,
}

async fn load_summary_projection_finalization_ranges(
    input: SummaryProjectionFinalizationRangesInput<'_>,
) -> Result<SummaryProjectionFinalizationRanges> {
    let SummaryProjectionFinalizationRangesInput {
        pool,
        exact_horizon,
        list_limit_max,
        current_complete_archive_paths,
        current_records,
        unavailable_unmaterialized_archive_buckets,
        unavailable_unmaterialized_archive_exact_ranges,
        unavailable_boundary_archive_ranges,
        unavailable_unmaterialized_archive_current_ranges,
        unavailable_unmaterialized_archive_account_buckets,
        unavailable_unmaterialized_archive_account_exact_ranges,
        unavailable_boundary_archive_account_ranges,
        unavailable_unmaterialized_archive_account_current_ranges,
        unavailable_exact_live_buckets,
        unavailable_exact_live_account_buckets,
        current_archive_admission_exceeded,
        mode,
    } = input;
    let mut global_ranges = summary_projection_unavailable_bucket_ranges(
        unavailable_unmaterialized_archive_buckets.clone(),
    );
    global_ranges.extend(
        unavailable_unmaterialized_archive_exact_ranges
            .iter()
            .copied(),
    );
    let global_ranges = summary_projection_merge_exact_ranges(global_ranges);
    let boundary_ranges =
        summary_projection_merge_exact_ranges(unavailable_boundary_archive_ranges.to_vec());
    let mut current_ranges = summary_projection_merge_exact_ranges(
        unavailable_unmaterialized_archive_current_ranges.to_vec(),
    );
    let mut account_ranges = summary_projection_unavailable_bucket_ranges(
        unavailable_unmaterialized_archive_account_buckets.clone(),
    );
    account_ranges.extend(
        unavailable_unmaterialized_archive_account_exact_ranges
            .iter()
            .copied(),
    );
    let account_ranges = summary_projection_merge_exact_ranges(account_ranges);
    let account_boundary_ranges =
        summary_projection_merge_exact_ranges(unavailable_boundary_archive_account_ranges.to_vec());
    let account_current_ranges = summary_projection_merge_exact_ranges(
        unavailable_unmaterialized_archive_account_current_ranges.to_vec(),
    );
    let exact_live_ranges =
        summary_projection_unavailable_bucket_ranges(unavailable_exact_live_buckets.clone());
    let exact_live_account_ranges = unavailable_exact_live_account_buckets
        .iter()
        .map(|(account_id, buckets)| {
            (
                *account_id,
                summary_projection_unavailable_bucket_ranges(buckets.clone()),
            )
        })
        .collect();
    let (unrepresented_ranges, latest_coverage_end, has_unknown_coverage) =
        load_summary_projection_unrepresented_current_archive_coverage(
            pool,
            current_complete_archive_paths,
            exact_horizon,
        )
        .await?;
    current_ranges.extend(unrepresented_ranges);
    log_summary_projection_current_archive_gap(
        mode,
        list_limit_max,
        current_records,
        &current_ranges,
        current_archive_admission_exceeded,
        has_unknown_coverage,
    );
    Ok(SummaryProjectionFinalizationRanges {
        unavailable_unmaterialized_archive_ranges: global_ranges,
        unavailable_boundary_archive_ranges: boundary_ranges,
        unavailable_unmaterialized_archive_current_ranges: current_ranges,
        unavailable_unmaterialized_archive_account_ranges: account_ranges,
        unavailable_boundary_archive_account_ranges: account_boundary_ranges,
        unavailable_unmaterialized_archive_account_current_ranges: account_current_ranges,
        unavailable_exact_live_ranges: exact_live_ranges,
        unavailable_exact_live_account_ranges: exact_live_account_ranges,
        current_archive_latest_coverage_end: latest_coverage_end,
        current_archive_has_unknown_coverage: has_unknown_coverage,
        current_account_source_unavailable: current_archive_admission_exceeded
            || has_unknown_coverage
            || latest_coverage_end.is_some(),
    })
}

fn log_summary_projection_current_archive_gap(
    mode: SummaryProjectionBuildMode,
    list_limit_max: usize,
    current_records: &[SummaryProjectionRecord],
    current_ranges: &[ExactUtcRange],
    current_archive_admission_exceeded: bool,
    current_archive_has_unknown_coverage: bool,
) {
    let mut timestamps = current_records
        .iter()
        .map(|record| record.occurred_at)
        .collect::<Vec<_>>();
    timestamps.sort_unstable_by(|left, right| right.cmp(left));
    let current_selection_cutoff_epoch = timestamps
        .get(list_limit_max.saturating_sub(1))
        .map(|timestamp| timestamp.timestamp());
    let current_gap_latest_end_epoch = current_ranges
        .iter()
        .map(|range| range.end.timestamp())
        .max();
    info!(
        ?mode,
        current_gap_range_count = current_ranges.len(),
        ?current_gap_latest_end_epoch,
        ?current_selection_cutoff_epoch,
        current_archive_admission_exceeded,
        current_archive_has_unknown_coverage,
        current_record_count = current_records.len(),
        stage = "current_archive_gap_proof",
        "summary projection current archive gap proof evaluated"
    );
}

struct SummaryProjectionFinalizationContext<'a> {
    input: SummaryProjectionFinalizationInput<'a>,
    revision: u64,
    refreshed_at: Option<Instant>,
    freshness: SummaryProjectionFreshness,
    terminal: SummaryProjectionTerminalCoverage,
    ranges: SummaryProjectionFinalizationRanges,
    generation_fence: SummaryProjectionGenerationFence,
    current_coverage_fence: SummaryCoverageFence,
    published_all_time_refreshed_at: Option<Instant>,
    all_time_oldest_account_refreshed_at: Option<Instant>,
    coverage_overlay: Option<SummaryCoverageOverlay>,
}

async fn prepare_summary_projection_finalization<'a>(
    mut input: SummaryProjectionFinalizationInput<'a>,
) -> Result<SummaryProjectionFinalizationContext<'a>> {
    let revision = input
        .state
        .subscription_hub
        .next_summary_projection_revision()
        .await;
    let all_time_completed_at = input.all_time_was_fully_rebuilt.then(Instant::now);
    if let Some(completed_at) = all_time_completed_at {
        for account_id in &input.rebuilt_all_time_account_ids {
            input
                .all_time_account_refreshed_at
                .insert(*account_id, completed_at);
        }
    }
    let freshness = build_summary_projection_finalization_freshness(&input);
    let terminal =
        build_summary_projection_terminal_coverage(SummaryProjectionTerminalCoverageInput {
            records: &input.records,
            current_records: &input.current_records,
            unavailable_exact_live_buckets: &input.unavailable_exact_live_buckets,
            unavailable_exact_live_account_buckets: &input.unavailable_exact_live_account_buckets,
            historical_global_covered_terminal_invoke_ids: &input
                .historical_global_covered_terminal_invoke_ids,
            all_time_was_fully_rebuilt: input.all_time_was_fully_rebuilt,
            global_all_time_source_unavailable: input.global_all_time_source_unavailable,
            previous_all_time_terminal_coverage_complete: input
                .previous_all_time_terminal_coverage_complete,
            durable_terminal_sequence_watermark: input.durable_terminal_sequence_watermark,
            previous_all_time_terminal_sequence_watermark: input
                .previous_all_time_terminal_sequence_watermark,
            account_all_time_unavailable: input.account_all_time_unavailable,
            rebuilt_all_time_account_ids: &input.rebuilt_all_time_account_ids,
            previous_all_time_account_terminal_sequence_watermarks: std::mem::take(
                &mut input.previous_all_time_account_terminal_sequence_watermarks,
            ),
            previous_all_time_account_persisted_live_terminal_invoke_ids: std::mem::take(
                &mut input.previous_all_time_account_persisted_live_terminal_invoke_ids,
            ),
        });
    let ranges =
        load_summary_projection_finalization_ranges(SummaryProjectionFinalizationRangesInput {
            pool: input.pool,
            exact_horizon: input.exact_horizon,
            list_limit_max: input.state.config.list_limit_max,
            current_complete_archive_paths: &input.current_complete_archive_paths,
            current_records: &input.current_records,
            unavailable_unmaterialized_archive_buckets: &input
                .unavailable_unmaterialized_archive_buckets,
            unavailable_unmaterialized_archive_exact_ranges: &input
                .unavailable_unmaterialized_archive_exact_ranges,
            unavailable_boundary_archive_ranges: &input.unavailable_boundary_archive_ranges,
            unavailable_unmaterialized_archive_current_ranges: &input
                .unavailable_unmaterialized_archive_current_ranges,
            unavailable_unmaterialized_archive_account_buckets: &input
                .unavailable_unmaterialized_archive_account_buckets,
            unavailable_unmaterialized_archive_account_exact_ranges: &input
                .unavailable_unmaterialized_archive_account_exact_ranges,
            unavailable_boundary_archive_account_ranges: &input
                .unavailable_boundary_archive_account_ranges,
            unavailable_unmaterialized_archive_account_current_ranges: &input
                .unavailable_unmaterialized_archive_account_current_ranges,
            unavailable_exact_live_buckets: &input.unavailable_exact_live_buckets,
            unavailable_exact_live_account_buckets: &input.unavailable_exact_live_account_buckets,
            current_archive_admission_exceeded: input.current_archive_admission_exceeded,
            mode: input.mode,
        })
        .await?;
    let generation_fence = SummaryProjectionGenerationFence {
        live_high_watermark_id: input.live_high_watermark_id,
        rollup_live_cursor: input.rollup_live_cursor,
        account_rollup_live_cursor: input.account_rollup_live_cursor,
        completed_manifest_high_watermark_id: input.completed_manifest_high_watermark_id,
        coverage_revision: summary_projection_coverage_revision(input.pool).await?,
        account_coverage_revision: summary_projection_account_coverage_revision(input.pool).await?,
        durable_terminal_sequence_watermark: input.durable_terminal_sequence_watermark,
    };
    let current_coverage_fence = generation_fence.coverage_fence();
    let published_all_time_refreshed_at =
        if input.all_time_was_fully_rebuilt && !input.global_all_time_source_unavailable {
            all_time_completed_at
        } else {
            input.previous_all_time_refreshed_at
        };
    Ok(SummaryProjectionFinalizationContext {
        input,
        revision,
        refreshed_at: Some(Instant::now()),
        freshness,
        terminal,
        ranges,
        generation_fence,
        current_coverage_fence,
        published_all_time_refreshed_at,
        all_time_oldest_account_refreshed_at: None,
        coverage_overlay: None,
    })
}

fn build_summary_projection_finalization_freshness(
    input: &SummaryProjectionFinalizationInput<'_>,
) -> SummaryProjectionFreshness {
    SummaryProjectionFreshness {
        global_all_time_eligible: if input.all_time_was_fully_rebuilt {
            if input.global_all_time_source_unavailable {
                input.previous_global_all_time_eligible
                    && input.all_time_by_account.contains_key(&None)
            } else {
                input.all_time_by_account.contains_key(&None)
            }
        } else {
            input.previous_global_all_time_eligible
        },
        account_all_time_eligible: if input.all_time_was_fully_rebuilt {
            if input.account_all_time_unavailable {
                input.previous_account_all_time_eligible.clone()
            } else {
                input
                    .all_time_by_account
                    .keys()
                    .filter_map(|account_id| *account_id)
                    .collect()
            }
        } else {
            input.previous_account_all_time_eligible.clone()
        },
    }
}

fn apply_summary_projection_finalization_revocations(
    context: &mut SummaryProjectionFinalizationContext<'_>,
) {
    let input = &mut context.input;
    let revoke_global_all_time = input.previous_global_all_time_eligible
        && input
            .previous_global_all_time_coverage_fence
            .is_none_or(|published| {
                !published.global_sources_match(context.current_coverage_fence)
            })
        && (!input.all_time_was_fully_rebuilt
            || input.global_all_time_source_unavailable
            || !input.all_time_by_account.contains_key(&None));
    if revoke_global_all_time {
        input.all_time_by_account.remove(&None);
        context.freshness.global_all_time_eligible = false;
        context.published_all_time_refreshed_at = None;
        input.previous_global_all_time_coverage_fence = None;
        context.terminal.all_time_terminal_coverage_complete = false;
        context.terminal.all_time_terminal_sequence_watermark = 0;
        context
            .terminal
            .all_time_persisted_live_terminal_invoke_ids
            .clear();
    }
    let revoke_account_all_time = !input.previous_account_all_time_eligible.is_empty()
        && input
            .previous_account_all_time_coverage_fence
            .is_none_or(|published| {
                !published.account_sources_match(context.current_coverage_fence)
            })
        && (!input.all_time_was_fully_rebuilt
            || input.account_all_time_unavailable
            || context.freshness.account_all_time_eligible.is_empty());
    if revoke_account_all_time {
        input.all_time_by_account.retain(|scope, _| scope.is_none());
        context.freshness.account_all_time_eligible.clear();
        input.all_time_account_refreshed_at.clear();
        input.previous_account_all_time_coverage_fence = None;
        context
            .terminal
            .all_time_account_terminal_sequence_watermarks
            .clear();
        context
            .terminal
            .all_time_account_persisted_live_terminal_invoke_ids
            .clear();
    }
    context.all_time_oldest_account_refreshed_at = if revoke_account_all_time {
        None
    } else {
        input.all_time_account_refreshed_at.values().copied().min()
    };
    context.coverage_overlay = input
        .previous_coverage_overlay
        .take()
        .filter(|overlay| overlay.coverage_fence == context.generation_fence.coverage_fence());
}

async fn finish_summary_projection_finalization(
    mut context: SummaryProjectionFinalizationContext<'_>,
) -> Result<SummaryProjection> {
    apply_summary_projection_finalization_revocations(&mut context);
    let mode = context.input.mode;
    #[cfg(test)]
    let pool = context.input.pool;
    let elapsed_ms = context.input.build_started_at.elapsed().as_millis() as u64;
    let current_generation_fence =
        load_summary_projection_generation_fence(context.input.state).await?;
    if !current_generation_fence.coverage_sources_match(context.generation_fence)
        || !current_generation_fence
            .live_tail_cursor()
            .terminal_sources_match(context.generation_fence.live_tail_cursor())
    {
        return Err(SummaryProjectionAllTimeGenerationChanged.into());
    }
    let projection =
        build_summary_projection_finalization_projection(context, current_generation_fence);
    info!(
        ?mode,
        elapsed_ms,
        stage = "projection_materialization",
        "summary projection build stage completed"
    );
    #[cfg(test)]
    pause_summary_projection_test_interleave(
        pool,
        mode,
        SummaryProjectionTestInterleaveStage::BeforeProjectionPublication,
    )
    .await?;
    Ok(projection)
}

fn build_summary_projection_finalization_projection(
    context: SummaryProjectionFinalizationContext<'_>,
    generation_fence: SummaryProjectionGenerationFence,
) -> SummaryProjection {
    let SummaryProjectionFinalizationContext {
        mut input,
        revision,
        refreshed_at,
        freshness,
        mut terminal,
        ranges,
        current_coverage_fence,
        published_all_time_refreshed_at,
        all_time_oldest_account_refreshed_at,
        coverage_overlay,
        ..
    } = context;
    let mut projection = SummaryProjection::default();
    apply_summary_projection_finalization_identity_fields(
        &mut projection,
        &mut input,
        &mut terminal,
        current_coverage_fence,
        &freshness,
    );
    apply_summary_projection_finalization_rollup_fields(
        &mut projection,
        &mut input,
        coverage_overlay,
    );
    apply_summary_projection_finalization_history_fields(
        &mut projection,
        &mut input,
        published_all_time_refreshed_at,
        all_time_oldest_account_refreshed_at,
    );
    apply_summary_projection_finalization_gap_fields(&mut projection, &mut input, ranges);
    projection.in_progress_by_account = std::mem::take(&mut input.in_progress_by_account);
    projection.maintenance = Some(input.maintenance);
    projection.generation_fence = generation_fence;
    projection.freshness_lease = SummaryProjectionFreshnessLease::default();
    projection.refreshed_at = refreshed_at;
    projection.freshness = freshness;
    projection.revision = revision;
    projection
}

fn apply_summary_projection_finalization_identity_fields(
    projection: &mut SummaryProjection,
    input: &mut SummaryProjectionFinalizationInput<'_>,
    terminal: &mut SummaryProjectionTerminalCoverage,
    current_coverage_fence: SummaryCoverageFence,
    freshness: &SummaryProjectionFreshness,
) {
    projection.records = std::mem::take(&mut input.records);
    projection.current_records = std::mem::take(&mut input.current_records);
    projection.persisted_live_terminal_invoke_ids =
        std::mem::take(&mut terminal.persisted_live_terminal_invoke_ids);
    projection.all_time_terminal_coverage_complete = terminal.all_time_terminal_coverage_complete;
    projection.global_all_time_coverage_fence =
        if input.all_time_was_fully_rebuilt && freshness.global_all_time_eligible {
            Some(current_coverage_fence)
        } else {
            input.previous_global_all_time_coverage_fence
        };
    projection.account_all_time_coverage_fence =
        if input.all_time_was_fully_rebuilt && !input.account_all_time_unavailable {
            Some(current_coverage_fence)
        } else {
            input.previous_account_all_time_coverage_fence
        };
    projection.all_time_terminal_sequence_watermark = terminal.all_time_terminal_sequence_watermark;
    projection.all_time_persisted_live_terminal_invoke_ids =
        std::mem::take(&mut terminal.all_time_persisted_live_terminal_invoke_ids);
    projection.all_time_account_terminal_sequence_watermarks =
        std::mem::take(&mut terminal.all_time_account_terminal_sequence_watermarks);
    projection.all_time_account_persisted_live_terminal_invoke_ids =
        std::mem::take(&mut terminal.all_time_account_persisted_live_terminal_invoke_ids);
    projection.durable_terminal_sequence_watermark = input.durable_terminal_sequence_watermark;
}

fn apply_summary_projection_finalization_rollup_fields(
    projection: &mut SummaryProjection,
    input: &mut SummaryProjectionFinalizationInput<'_>,
    coverage_overlay: Option<SummaryCoverageOverlay>,
) {
    projection.hourly_buckets = std::mem::take(&mut input.hourly_buckets);
    projection.recent_indexes = std::mem::take(&mut input.recent_indexes);
    projection.recent_index_complete = input.recent_index_complete;
    projection.recent_index_overflow_at = input.recent_index_overflow_at;
    projection.hourly_rollup_totals = std::mem::take(&mut input.hourly_rollup_totals);
    projection.hourly_rollup_non_success_tokens =
        std::mem::take(&mut input.hourly_rollup_non_success_tokens);
    projection.hourly_rollup_usage = std::mem::take(&mut input.hourly_rollup_usage);
    projection.exact_global_total_rollup_buckets =
        std::mem::take(&mut input.exact_global_total_rollup_buckets);
    projection.exact_account_total_rollup_buckets =
        std::mem::take(&mut input.exact_account_total_rollup_buckets);
    projection.exact_global_usage_rollup_buckets =
        std::mem::take(&mut input.exact_global_usage_rollup_buckets);
    projection.exact_account_usage_rollup_buckets =
        std::mem::take(&mut input.exact_account_usage_rollup_buckets);
    projection.rollup_live_cursor = input.rollup_live_cursor;
    projection.historical_live_coverage = input.historical_live_coverage.take();
    projection.coverage_overlay = coverage_overlay;
}

fn apply_summary_projection_finalization_history_fields(
    projection: &mut SummaryProjection,
    input: &mut SummaryProjectionFinalizationInput<'_>,
    all_time_refreshed_at: Option<Instant>,
    all_time_oldest_account_refreshed_at: Option<Instant>,
) {
    projection.all_time_by_account = std::mem::take(&mut input.all_time_by_account);
    projection.all_time_refreshed_at = all_time_refreshed_at;
    projection.all_time_manifest_admission_blocked_at = if input.all_time_was_fully_rebuilt {
        input.all_time_archive_admission_exceeded.then(Instant::now)
    } else {
        input.previous_all_time_manifest_admission_blocked_at
    };
    projection.all_time_account_manifest_admission_blocked_at =
        if input.all_time_account_manifest_admission_attempted {
            input
                .all_time_account_manifest_admission_exceeded
                .then(Instant::now)
        } else {
            input.previous_all_time_account_manifest_admission_blocked_at
        };
    projection.all_time_account_refreshed_at =
        std::mem::take(&mut input.all_time_account_refreshed_at);
    projection.all_time_oldest_account_refreshed_at = all_time_oldest_account_refreshed_at;
    projection.all_time_account_ids_with_projection_data =
        std::mem::take(&mut input.account_ids_with_projection_data);
    projection.known_account_ids = std::mem::take(&mut input.account_ids);
    projection.archive_account_ids_by_file = std::mem::take(&mut input.archive_account_ids_by_file);
    projection.archive_coverage_ranges_by_file =
        std::mem::take(&mut input.archive_actual_coverage_ranges);
}

fn apply_summary_projection_finalization_gap_fields(
    projection: &mut SummaryProjection,
    input: &mut SummaryProjectionFinalizationInput<'_>,
    ranges: SummaryProjectionFinalizationRanges,
) {
    projection.unavailable_unmaterialized_archive_ranges =
        ranges.unavailable_unmaterialized_archive_ranges;
    projection.unavailable_boundary_archive_ranges = ranges.unavailable_boundary_archive_ranges;
    projection.unavailable_unmaterialized_archive_current_ranges =
        ranges.unavailable_unmaterialized_archive_current_ranges;
    projection.unavailable_unmaterialized_archive_account_ranges =
        ranges.unavailable_unmaterialized_archive_account_ranges;
    projection.unavailable_boundary_archive_account_ranges =
        ranges.unavailable_boundary_archive_account_ranges;
    projection.unavailable_unmaterialized_archive_account_current_ranges =
        ranges.unavailable_unmaterialized_archive_account_current_ranges;
    projection.unavailable_exact_live_ranges = ranges.unavailable_exact_live_ranges;
    projection.unavailable_exact_live_account_ranges = ranges.unavailable_exact_live_account_ranges;
    projection.current_source_unavailable = input.current_source_unavailable;
    projection.current_source_unavailable_from_rank = input.current_source_unavailable_from_rank;
    projection.current_account_source_unavailable_from_rank =
        std::mem::take(&mut input.current_account_source_unavailable_from_rank);
    projection.current_archive_latest_coverage_end = ranges.current_archive_latest_coverage_end;
    projection.current_archive_has_unknown_coverage = ranges.current_archive_has_unknown_coverage;
    projection.current_archive_admission_exceeded = input.current_archive_admission_exceeded;
    projection.current_account_source_unavailable = ranges.current_account_source_unavailable;
}
