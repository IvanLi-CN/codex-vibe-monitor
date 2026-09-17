struct SummaryProjectionBoundaryPlanningInput<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    exact_horizon: ExactUtcRange,
    paged_boundary_manifest_high_watermark_id: Option<i64>,
    protected_boundary_buckets: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    known_account_ids: &'a mut HashSet<i64>,
    exact_archive_buckets: &'a mut HashSet<i64>,
    exact_global_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a mut HashSet<i64>,
    unavailable_unmaterialized_archive_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_account_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_current_ranges: &'a mut Vec<ExactUtcRange>,
}

struct SummaryProjectionBoundaryPagePlanningInput<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    exact_horizon: ExactUtcRange,
    page: SummaryProjectionBoundaryManifestPage,
    known_account_ids: &'a mut HashSet<i64>,
    protected_boundary_buckets: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    exact_archive_buckets: &'a mut HashSet<i64>,
    exact_global_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a mut HashSet<i64>,
    unavailable_unmaterialized_archive_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_account_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_current_ranges: &'a mut Vec<ExactUtcRange>,
}

struct SummaryProjectionBoundaryArchivePlanningInput<'a> {
    mode: SummaryProjectionBuildMode,
    archive: &'a crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_ids: &'a HashSet<i64>,
    account_manifest_complete: bool,
    protected_boundary_buckets: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    exact_archive_buckets: &'a mut HashSet<i64>,
    exact_global_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a mut HashSet<i64>,
    unavailable_unmaterialized_archive_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_account_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_current_ranges: &'a mut Vec<ExactUtcRange>,
}

struct SummaryProjectionBoundaryManifestArchivePlan {
    archive_range: ExactUtcRange,
    archive_has_materialized_rollups: bool,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_replayed: Option<bool>,
    usage_replayed: Option<bool>,
    raw_fallback_ranges: Vec<ExactUtcRange>,
    global_bucket_requirements: HashSet<i64>,
    account_bucket_requirements: HashSet<i64>,
}

struct SummaryProjectionBoundaryFallbackInput<'a> {
    archive: &'a crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_ids: &'a HashSet<i64>,
    account_manifest_complete: bool,
    exact_archive_buckets: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
}

struct SummaryProjectionBoundaryUnavailableInput<'a> {
    archive: &'a crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_replayed: Option<bool>,
    usage_replayed: Option<bool>,
    account_ids: &'a HashSet<i64>,
    exact_archive_buckets: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    raw_fallback_ranges: Vec<ExactUtcRange>,
    global_bucket_requirements: HashSet<i64>,
    account_bucket_requirements: HashSet<i64>,
    unavailable_unmaterialized_archive_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_account_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_account_current_ranges: &'a mut Vec<ExactUtcRange>,
}

async fn plan_summary_projection_boundary_manifests(
    input: SummaryProjectionBoundaryPlanningInput<'_>,
) -> Result<()> {
    let SummaryProjectionBoundaryPlanningInput {
        pool,
        mode,
        build_started_at,
        exact_horizon,
        paged_boundary_manifest_high_watermark_id,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        known_account_ids,
        exact_archive_buckets,
        exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets,
        unavailable_unmaterialized_archive_buckets,
        unavailable_boundary_archive_ranges,
        unavailable_unmaterialized_archive_current_ranges,
        unavailable_unmaterialized_archive_account_buckets,
        unavailable_boundary_archive_account_ranges,
        unavailable_unmaterialized_archive_account_current_ranges,
    } = input;
    let Some(high_watermark_id) = paged_boundary_manifest_high_watermark_id else {
        return Ok(());
    };
    info!(
        ?mode,
        stage = "boundary_manifest_page_planning",
        "summary projection build stage started"
    );
    let mut after_id = None;
    loop {
        let page = load_summary_projection_boundary_manifest_page(
            pool,
            Some(exact_horizon),
            after_id,
            high_watermark_id,
        )
        .await?;
        if page.archives.is_empty() {
            break;
        }
        let next_after_id = page.next_after_id;
        plan_summary_projection_boundary_page(SummaryProjectionBoundaryPagePlanningInput {
            pool,
            mode,
            exact_horizon,
            page,
            known_account_ids,
            protected_boundary_buckets,
            hourly_rollup_totals,
            hourly_rollup_usage,
            exact_archive_buckets,
            exact_global_total_rollup_buckets,
            exact_account_total_rollup_buckets,
            exact_global_usage_rollup_buckets,
            exact_account_usage_rollup_buckets,
            unavailable_unmaterialized_archive_buckets,
            unavailable_boundary_archive_ranges,
            unavailable_unmaterialized_archive_current_ranges,
            unavailable_unmaterialized_archive_account_buckets,
            unavailable_boundary_archive_account_ranges,
            unavailable_unmaterialized_archive_account_current_ranges,
        })
        .await?;
        let Some(next_after_id) = next_after_id else {
            break;
        };
        after_id = Some(next_after_id);
    }
    info!(
        ?mode,
        stage = "boundary_manifest_page_planning",
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build stage completed"
    );
    Ok(())
}

async fn plan_summary_projection_boundary_page(
    input: SummaryProjectionBoundaryPagePlanningInput<'_>,
) -> Result<()> {
    let SummaryProjectionBoundaryPagePlanningInput {
        pool,
        mode,
        exact_horizon,
        page,
        known_account_ids,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        exact_archive_buckets,
        exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets,
        unavailable_unmaterialized_archive_buckets,
        unavailable_boundary_archive_ranges,
        unavailable_unmaterialized_archive_current_ranges,
        unavailable_unmaterialized_archive_account_buckets,
        unavailable_boundary_archive_account_ranges,
        unavailable_unmaterialized_archive_account_current_ranges,
    } = input;
    let page_paths = page
        .archives
        .iter()
        .map(|archive| archive.file_path().to_string())
        .collect::<Vec<_>>();
    let page_replay_coverage =
        load_summary_projection_archive_replay_coverage(pool, &page_paths).await?;
    let page_account_ids_by_file =
        load_summary_projection_archive_manifest_account_sets(pool, &page_paths).await?;
    for account_ids in page_account_ids_by_file.values() {
        known_account_ids.extend(account_ids.iter().copied());
    }
    if known_account_ids.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
        return Err(anyhow!(
            "summary projection paged boundary account cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ACCOUNTS})"
        ));
    }
    for archive in &page.archives {
        let Some(archive_range) = summary_projection_archive_overlap_range(archive, exact_horizon)
        else {
            continue;
        };
        let replay_coverage = summary_projection_effective_replay_coverage(
            archive,
            page_replay_coverage
                .get(archive.file_path())
                .copied()
                .unwrap_or_default(),
        );
        let account_ids = page_account_ids_by_file
            .get(archive.file_path())
            .expect("page account manifest includes every requested archive path");
        plan_summary_projection_boundary_manifest_archive(
            SummaryProjectionBoundaryArchivePlanningInput {
                mode,
                archive,
                archive_range,
                replay_coverage,
                account_ids,
                account_manifest_complete: page
                    .account_manifest_refreshed_paths
                    .contains(archive.file_path()),
                protected_boundary_buckets,
                hourly_rollup_totals,
                hourly_rollup_usage,
                exact_archive_buckets,
                exact_global_total_rollup_buckets,
                exact_account_total_rollup_buckets,
                exact_global_usage_rollup_buckets,
                exact_account_usage_rollup_buckets,
                unavailable_unmaterialized_archive_buckets,
                unavailable_boundary_archive_ranges,
                unavailable_unmaterialized_archive_current_ranges,
                unavailable_unmaterialized_archive_account_buckets,
                unavailable_boundary_archive_account_ranges,
                unavailable_unmaterialized_archive_account_current_ranges,
            },
        )?;
    }
    Ok(())
}

struct SummaryProjectionBoundaryReplacementPlanningInput<'a> {
    mode: SummaryProjectionBuildMode,
    archive: &'a crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_manifest_complete: bool,
    protected_boundary_buckets: &'a HashSet<i64>,
    exact_global_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_total_rollup_buckets: &'a mut HashSet<i64>,
    exact_global_usage_rollup_buckets: &'a mut HashSet<i64>,
    exact_account_usage_rollup_buckets: &'a mut HashSet<i64>,
}

struct SummaryProjectionBoundaryReplacementBuckets {
    global_total: HashSet<i64>,
    account_total: HashSet<i64>,
    global_usage: HashSet<i64>,
    account_usage: HashSet<i64>,
}

fn plan_summary_projection_boundary_replacement_buckets(
    input: SummaryProjectionBoundaryReplacementPlanningInput<'_>,
) -> Result<SummaryProjectionBoundaryReplacementBuckets> {
    let SummaryProjectionBoundaryReplacementPlanningInput {
        mode,
        archive,
        archive_range,
        replay_coverage,
        account_manifest_complete,
        protected_boundary_buckets,
        exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets,
    } = input;
    let mut paged_exact_global_total_buckets = HashSet::new();
    let mut paged_exact_account_total_buckets = HashSet::new();
    let mut paged_exact_global_usage_buckets = HashSet::new();
    let mut paged_exact_account_usage_buckets = HashSet::new();
    if summary_projection_archive_coverage_range(archive).is_some() {
        let (global_total, account_total, global_usage, account_usage) = if mode.includes_all_time()
        {
            (
                &mut *exact_global_total_rollup_buckets,
                &mut *exact_account_total_rollup_buckets,
                &mut *exact_global_usage_rollup_buckets,
                &mut *exact_account_usage_rollup_buckets,
            )
        } else {
            (
                &mut paged_exact_global_total_buckets,
                &mut paged_exact_account_total_buckets,
                &mut paged_exact_global_usage_buckets,
                &mut paged_exact_account_usage_buckets,
            )
        };
        summary_projection_mark_exact_replacement_buckets(
            SummaryProjectionExactReplacementInput {
                archive_has_materialized_rollups: archive.has_materialized_historical_rollups(),
                replay_coverage,
                exact_range: archive_range,
                _protected_boundary_buckets: protected_boundary_buckets,
                exact_global_total_rollup_buckets: global_total,
                exact_account_total_rollup_buckets: account_total,
                exact_global_usage_rollup_buckets: global_usage,
                exact_account_usage_rollup_buckets: account_usage,
            },
        )?;
    }
    if !account_manifest_complete {
        let account_total = if mode.includes_all_time() {
            exact_account_total_rollup_buckets
        } else {
            &mut paged_exact_account_total_buckets
        };
        summary_projection_mark_account_exact_replacement_buckets(
            archive.has_materialized_historical_rollups(),
            archive_range,
            account_total,
        )?;
    }
    Ok(SummaryProjectionBoundaryReplacementBuckets {
        global_total: paged_exact_global_total_buckets,
        account_total: paged_exact_account_total_buckets,
        global_usage: paged_exact_global_usage_buckets,
        account_usage: paged_exact_account_usage_buckets,
    })
}

fn plan_summary_projection_boundary_manifest_archive(
    input: SummaryProjectionBoundaryArchivePlanningInput<'_>,
) -> Result<()> {
    let SummaryProjectionBoundaryArchivePlanningInput {
        mode,
        archive,
        archive_range,
        replay_coverage,
        account_ids,
        account_manifest_complete,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        exact_archive_buckets,
        exact_global_total_rollup_buckets,
        exact_account_total_rollup_buckets,
        exact_global_usage_rollup_buckets,
        exact_account_usage_rollup_buckets,
        unavailable_unmaterialized_archive_buckets,
        unavailable_boundary_archive_ranges,
        unavailable_unmaterialized_archive_current_ranges,
        unavailable_unmaterialized_archive_account_buckets,
        unavailable_boundary_archive_account_ranges,
        unavailable_unmaterialized_archive_account_current_ranges,
    } = input;
    let replacement_buckets = plan_summary_projection_boundary_replacement_buckets(
        SummaryProjectionBoundaryReplacementPlanningInput {
            mode,
            archive,
            archive_range,
            replay_coverage,
            account_manifest_complete,
            protected_boundary_buckets,
            exact_global_total_rollup_buckets,
            exact_account_total_rollup_buckets,
            exact_global_usage_rollup_buckets,
            exact_account_usage_rollup_buckets,
        },
    )?;
    let SummaryProjectionBoundaryReplacementBuckets {
        global_total: paged_exact_global_total_buckets,
        account_total: paged_exact_account_total_buckets,
        global_usage: paged_exact_global_usage_buckets,
        account_usage: paged_exact_account_usage_buckets,
    } = replacement_buckets;
    let (account_replayed, usage_replayed, raw_fallback_ranges) =
        build_summary_projection_boundary_fallback(SummaryProjectionBoundaryFallbackInput {
            archive,
            archive_range,
            replay_coverage,
            account_ids,
            account_manifest_complete,
            exact_archive_buckets,
            hourly_rollup_totals,
            hourly_rollup_usage,
        });
    if mode.includes_all_time() {
        summary_projection_extend_exact_buckets_for_ranges(
            exact_archive_buckets,
            raw_fallback_ranges,
        )?;
        return Ok(());
    }
    let mut global_bucket_requirements = paged_exact_global_total_buckets;
    global_bucket_requirements.extend(paged_exact_global_usage_buckets);
    let mut account_bucket_requirements = paged_exact_account_total_buckets;
    account_bucket_requirements.extend(paged_exact_account_usage_buckets);
    apply_summary_projection_boundary_unavailability(SummaryProjectionBoundaryUnavailableInput {
        archive,
        archive_range,
        replay_coverage,
        account_replayed,
        usage_replayed,
        account_ids,
        exact_archive_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        raw_fallback_ranges,
        global_bucket_requirements,
        account_bucket_requirements,
        unavailable_unmaterialized_archive_buckets,
        unavailable_boundary_archive_ranges,
        unavailable_unmaterialized_archive_current_ranges,
        unavailable_unmaterialized_archive_account_buckets,
        unavailable_boundary_archive_account_ranges,
        unavailable_unmaterialized_archive_account_current_ranges,
    })?;
    Ok(())
}

fn build_summary_projection_boundary_fallback(
    input: SummaryProjectionBoundaryFallbackInput<'_>,
) -> (Option<bool>, Option<bool>, Vec<ExactUtcRange>) {
    let SummaryProjectionBoundaryFallbackInput {
        archive,
        archive_range,
        replay_coverage,
        account_ids,
        account_manifest_complete,
        exact_archive_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
    } = input;
    let (account_replayed, usage_replayed) = if !account_manifest_complete {
        (Some(false), Some(replay_coverage.usage_breakdown))
    } else if account_ids.is_empty() {
        (Some(true), Some(replay_coverage.usage_breakdown))
    } else {
        (None, None)
    };
    let raw_fallback_ranges = summary_projection_archive_exact_ranges_with_coverage(
        archive.has_materialized_historical_rollups(),
        SummaryProjectionArchiveReplayFlags {
            overall: Some(replay_coverage.overall),
            account: account_replayed,
            usage: usage_replayed,
        },
        archive_range,
        exact_archive_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        account_ids,
    );
    (account_replayed, usage_replayed, raw_fallback_ranges)
}

fn apply_summary_projection_boundary_unavailability(
    input: SummaryProjectionBoundaryUnavailableInput<'_>,
) -> Result<()> {
    let SummaryProjectionBoundaryUnavailableInput {
        archive,
        archive_range,
        replay_coverage,
        account_replayed,
        usage_replayed,
        account_ids,
        exact_archive_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        raw_fallback_ranges,
        mut global_bucket_requirements,
        mut account_bucket_requirements,
        unavailable_unmaterialized_archive_buckets,
        unavailable_boundary_archive_ranges,
        unavailable_unmaterialized_archive_current_ranges,
        unavailable_unmaterialized_archive_account_buckets,
        unavailable_boundary_archive_account_ranges,
        unavailable_unmaterialized_archive_account_current_ranges,
    } = input;
    let (global_compact_coverage_gaps, account_compact_coverage_gaps) =
        summary_projection_boundary_compact_coverage_gaps(SummaryProjectionBoundaryCoverageInput {
            archive,
            archive_range,
            replay_coverage,
            account_replayed,
            usage_replayed,
            account_ids,
            hourly_rollup_totals,
            hourly_rollup_usage,
        });
    summary_projection_extend_exact_buckets_for_ranges(
        &mut global_bucket_requirements,
        global_compact_coverage_gaps,
    )?;
    summary_projection_extend_exact_buckets_for_ranges(
        &mut account_bucket_requirements,
        account_compact_coverage_gaps,
    )?;
    let mut global_unavailable_ranges = summary_projection_archive_exact_ranges_with_coverage(
        archive.has_materialized_historical_rollups(),
        SummaryProjectionArchiveReplayFlags {
            overall: Some(replay_coverage.overall),
            account: Some(true),
            usage: Some(replay_coverage.usage_breakdown),
        },
        archive_range,
        exact_archive_buckets,
        hourly_rollup_totals,
        hourly_rollup_usage,
        account_ids,
    );
    let mut account_unavailable_ranges = raw_fallback_ranges;
    for (ranges, requirements) in [
        (&mut global_unavailable_ranges, &global_bucket_requirements),
        (
            &mut account_unavailable_ranges,
            &account_bucket_requirements,
        ),
    ] {
        for range in summary_projection_exact_bucket_ranges(requirements) {
            let range = ExactUtcRange {
                start: range.start.max(archive_range.start),
                end: range.end.min(archive_range.end),
            };
            if range.start < range.end {
                ranges.push(range);
            }
        }
    }
    let global_unavailable_ranges =
        summary_projection_merge_exact_ranges(global_unavailable_ranges);
    let account_unavailable_ranges =
        summary_projection_merge_exact_ranges(account_unavailable_ranges);
    summary_projection_mark_unavailable_archive_ranges_by_requirement(
        unavailable_unmaterialized_archive_buckets,
        unavailable_boundary_archive_ranges,
        archive.has_materialized_historical_rollups(),
        &global_unavailable_ranges,
        &global_bucket_requirements,
    )?;
    summary_projection_mark_unavailable_archive_ranges_by_requirement(
        unavailable_unmaterialized_archive_account_buckets,
        unavailable_boundary_archive_account_ranges,
        archive.has_materialized_historical_rollups(),
        &account_unavailable_ranges,
        &account_bucket_requirements,
    )?;
    if !global_unavailable_ranges.is_empty() {
        unavailable_unmaterialized_archive_current_ranges.push(archive_range);
    }
    if !account_unavailable_ranges.is_empty() {
        unavailable_unmaterialized_archive_account_current_ranges.push(archive_range);
    }
    Ok(())
}

struct SummaryProjectionBoundaryCoverageInput<'a> {
    archive: &'a crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_replayed: Option<bool>,
    usage_replayed: Option<bool>,
    account_ids: &'a HashSet<i64>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
}

fn summary_projection_boundary_compact_coverage_gaps(
    input: SummaryProjectionBoundaryCoverageInput<'_>,
) -> (Vec<ExactUtcRange>, Vec<ExactUtcRange>) {
    let global = summary_projection_archive_exact_ranges_with_coverage(
        input.archive.has_materialized_historical_rollups(),
        SummaryProjectionArchiveReplayFlags {
            overall: Some(input.replay_coverage.overall),
            account: Some(true),
            usage: Some(true),
        },
        input.archive_range,
        &HashSet::new(),
        input.hourly_rollup_totals,
        input.hourly_rollup_usage,
        input.account_ids,
    );
    let account = summary_projection_archive_exact_ranges_with_coverage(
        input.archive.has_materialized_historical_rollups(),
        SummaryProjectionArchiveReplayFlags {
            overall: Some(input.replay_coverage.overall),
            account: input.account_replayed,
            usage: input.usage_replayed,
        },
        input.archive_range,
        &HashSet::new(),
        input.hourly_rollup_totals,
        input.hourly_rollup_usage,
        input.account_ids,
    );
    (global, account)
}
