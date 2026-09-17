struct SummaryProjectionBoundaryArchiveHydrationState<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    archive_start: DateTime<Utc>,
    end: DateTime<Utc>,
    build_started_at: Instant,
    archive_replay_coverage: &'a HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    archive_row_counts: &'a HashMap<String, i64>,
    archive_manifest_sha256: &'a HashMap<String, String>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    exact_archive_buckets: &'a HashSet<i64>,
    exact_bucket_requirements: &'a HashSet<i64>,
    usage_rollup_progress: &'a HashMap<String, i64>,
    known_account_ids: &'a HashSet<i64>,
    persisted_live_ids: &'a HashSet<SummarySourceIdentity>,
    current_record_bytes: usize,
    records_by_invoke_id: &'a mut HashMap<String, SummaryProjectionRecord>,
    exact_record_budget: &'a mut usize,
    exact_record_bytes: &'a mut usize,
    unavailable_unmaterialized_archive_buckets: &'a mut BTreeSet<i64>,
    unavailable_boundary_archive_ranges: &'a mut Vec<ExactUtcRange>,
    unavailable_unmaterialized_archive_current_ranges: &'a mut Vec<ExactUtcRange>,
    all_time_source_unavailable_from_archive_ranges: &'a mut bool,
}

async fn hydrate_summary_projection_boundary_archives(
    mut input: SummaryProjectionBoundaryArchiveHydrationState<'_>,
    archives: Vec<crate::stats::ArchiveBatchPathRow>,
) -> Result<()> {
    info!(
        ?input.mode,
        stage = "boundary_archive_hydration",
        "summary projection build stage started"
    );
    for archive in archives {
        hydrate_summary_projection_boundary_archive(&mut input, archive).await?;
    }
    info!(
        ?input.mode,
        stage = "boundary_archive_hydration",
        elapsed_ms = input.build_started_at.elapsed().as_millis() as u64,
        "summary projection build stage completed"
    );
    Ok(())
}

async fn merge_summary_projection_archive_snapshot_fallback(
    input: &mut SummaryProjectionBoundaryArchiveHydrationState<'_>,
    archive: &crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    archive_rollups_are_exact: bool,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    exact_ranges: &[ExactUtcRange],
    stage: &str,
) -> Result<bool> {
    let Some(snapshot_rows) =
        load_summary_projection_snapshot_records(input.pool, archive.file_path(), exact_ranges)
            .await?
    else {
        return Ok(false);
    };
    let snapshot_merge = merge_summary_projection_archive_records_with_coverage(
        SummaryProjectionArchiveMergeInput {
            archive_pool: input.pool,
            persisted_live_ids: input.persisted_live_ids,
            hourly_rollup_totals: input.hourly_rollup_totals,
            hourly_rollup_usage: input.hourly_rollup_usage,
            archive_has_materialized_rollups: archive_rollups_are_exact,
            overall_rollup_archive_replayed: Some(replay_coverage.overall),
            account_rollup_archive_replayed: Some(replay_coverage.account_stats),
            usage_rollup_archive_replayed: Some(replay_coverage.usage_breakdown),
            exact_range: archive_range,
            protected_boundary_buckets: input.exact_archive_buckets,
            usage_rollup_cursor: input
                .usage_rollup_progress
                .get(archive.file_path())
                .copied(),
            known_account_ids: input.known_account_ids,
            records_by_invoke_id: input.records_by_invoke_id,
            exact_record_budget: input.exact_record_budget,
            exact_record_bytes: input.exact_record_bytes,
            resident_current_record_bytes: input.current_record_bytes,
            snapshot_rows: Some(snapshot_rows),
        },
    )
    .await;
    if snapshot_merge.is_ok() {
        debug!(
            ?input.mode,
            archive = archive.file_path(),
            stage,
            "used verified Summary Archive Snapshot V2 fallback"
        );
        return Ok(true);
    }
    Ok(false)
}

fn mark_summary_projection_boundary_archive_unavailable(
    input: &mut SummaryProjectionBoundaryArchiveHydrationState<'_>,
    archive_range: ExactUtcRange,
    archive_rollups_are_exact: bool,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    exact_ranges: &[ExactUtcRange],
) -> Result<()> {
    if !archive_rollups_are_exact && !replay_coverage.supports_unavailable_archive() {
        *input.all_time_source_unavailable_from_archive_ranges = true;
    }
    summary_projection_mark_unavailable_archive_ranges_by_requirement(
        input.unavailable_unmaterialized_archive_buckets,
        input.unavailable_boundary_archive_ranges,
        archive_rollups_are_exact,
        exact_ranges,
        input.exact_bucket_requirements,
    )?;
    input
        .unavailable_unmaterialized_archive_current_ranges
        .push(archive_range);
    Ok(())
}

async fn merge_summary_projection_boundary_archive_raw(
    input: &mut SummaryProjectionBoundaryArchiveHydrationState<'_>,
    archive: &crate::stats::ArchiveBatchPathRow,
    archive_range: ExactUtcRange,
    archive_rollups_are_exact: bool,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    exact_ranges: &[ExactUtcRange],
    manifest_sha256: &str,
) -> Result<()> {
    let Some((archive_pool, temp_cleanup)) =
        crate::stats::open_invocation_archive_batch_pool(archive, "summary-projection").await?
    else {
        if merge_summary_projection_archive_snapshot_fallback(
            input,
            archive,
            archive_range,
            archive_rollups_are_exact,
            replay_coverage,
            exact_ranges,
            "archive_snapshot_v2_fallback",
        )
        .await?
        {
            return Ok(());
        }
        mark_summary_projection_boundary_archive_unavailable(
            input,
            archive_range,
            archive_rollups_are_exact,
            replay_coverage,
            exact_ranges,
        )?;
        return Ok(());
    };
    require_summary_projection_archive_file_sha256(archive, manifest_sha256)?;
    let records_before_archive = input.records_by_invoke_id.clone();
    let exact_record_budget_before_archive = *input.exact_record_budget;
    let exact_record_bytes_before_archive = *input.exact_record_bytes;
    let merge_result = merge_summary_projection_archive_records_with_coverage(
        SummaryProjectionArchiveMergeInput {
            archive_pool: &archive_pool,
            persisted_live_ids: input.persisted_live_ids,
            hourly_rollup_totals: input.hourly_rollup_totals,
            hourly_rollup_usage: input.hourly_rollup_usage,
            archive_has_materialized_rollups: archive_rollups_are_exact,
            overall_rollup_archive_replayed: Some(replay_coverage.overall),
            account_rollup_archive_replayed: Some(replay_coverage.account_stats),
            usage_rollup_archive_replayed: Some(replay_coverage.usage_breakdown),
            exact_range: archive_range,
            protected_boundary_buckets: input.exact_archive_buckets,
            usage_rollup_cursor: input
                .usage_rollup_progress
                .get(archive.file_path())
                .copied(),
            known_account_ids: input.known_account_ids,
            records_by_invoke_id: input.records_by_invoke_id,
            exact_record_budget: input.exact_record_budget,
            exact_record_bytes: input.exact_record_bytes,
            resident_current_record_bytes: input.current_record_bytes,
            snapshot_rows: None,
        },
    )
    .await;
    require_summary_projection_archive_file_sha256(archive, manifest_sha256)?;
    archive_pool.close().await;
    drop(temp_cleanup);
    let Err(error) = merge_result else {
        return Ok(());
    };
    if !summary_projection_archive_raw_admission_exceeded(&error) {
        return Err(anyhow!(
            "summary projection archive hydration failed for {}: {error:?}",
            archive.file_path()
        ));
    }
    *input.records_by_invoke_id = records_before_archive;
    *input.exact_record_budget = exact_record_budget_before_archive;
    *input.exact_record_bytes = exact_record_bytes_before_archive;
    mark_summary_projection_boundary_archive_unavailable(
        input,
        archive_range,
        archive_rollups_are_exact,
        replay_coverage,
        exact_ranges,
    )?;
    Ok(())
}

struct SummaryProjectionBoundaryArchivePlan {
    archive_range: ExactUtcRange,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    archive_rollups_are_exact: bool,
    exact_ranges: Vec<ExactUtcRange>,
    manifest_sha256: String,
}

fn plan_summary_projection_boundary_archive(
    input: &mut SummaryProjectionBoundaryArchiveHydrationState<'_>,
    archive: &crate::stats::ArchiveBatchPathRow,
) -> Result<Option<SummaryProjectionBoundaryArchivePlan>> {
    let Some(archive_range) = summary_projection_archive_overlap_range(
        archive,
        ExactUtcRange {
            start: input.archive_start,
            end: input.end,
        },
    ) else {
        return Ok(None);
    };
    let replay_coverage = summary_projection_effective_replay_coverage(
        archive,
        input
            .archive_replay_coverage
            .get(archive.file_path())
            .copied()
            .unwrap_or_default(),
    );
    let archive_rollups_are_exact = archive.has_materialized_historical_rollups();
    let exact_ranges = summary_projection_archive_exact_ranges_with_coverage(
        archive_rollups_are_exact,
        SummaryProjectionArchiveReplayFlags {
            overall: Some(replay_coverage.overall),
            account: Some(replay_coverage.account_stats),
            usage: Some(replay_coverage.usage_breakdown),
        },
        archive_range,
        input.exact_archive_buckets,
        input.hourly_rollup_totals,
        input.hourly_rollup_usage,
        input.known_account_ids,
    );
    if exact_ranges.is_empty() {
        return Ok(None);
    }
    let row_count_exceeds_limit = input
        .archive_row_counts
        .get(archive.file_path())
        .is_some_and(|count| *count > summary_projection_exact_record_limit() as i64);
    if !archive_rollups_are_exact && row_count_exceeds_limit {
        return Err(anyhow!(
            "summary projection exact archive boundary exceeds bounded row budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    if archive_rollups_are_exact
        && row_count_exceeds_limit
        && summary_projection_archive_is_fully_within_exact_horizon(
            archive,
            ExactUtcRange {
                start: input.archive_start,
                end: input.end,
            },
        )
    {
        summary_projection_mark_unavailable_archive_ranges_by_requirement(
            input.unavailable_unmaterialized_archive_buckets,
            input.unavailable_boundary_archive_ranges,
            true,
            &exact_ranges,
            input.exact_bucket_requirements,
        )?;
        input
            .unavailable_unmaterialized_archive_current_ranges
            .push(archive_range);
        return Ok(None);
    }
    let manifest_sha256 = input
        .archive_manifest_sha256
        .get(archive.file_path())
        .ok_or_else(|| {
            anyhow!(
                "summary projection archive manifest SHA is missing for {}",
                archive.file_path()
            )
        })?
        .clone();
    Ok(Some(SummaryProjectionBoundaryArchivePlan {
        archive_range,
        replay_coverage,
        archive_rollups_are_exact,
        exact_ranges,
        manifest_sha256,
    }))
}

async fn hydrate_summary_projection_boundary_archive(
    input: &mut SummaryProjectionBoundaryArchiveHydrationState<'_>,
    archive: crate::stats::ArchiveBatchPathRow,
) -> Result<()> {
    let Some(plan) = plan_summary_projection_boundary_archive(input, &archive)? else {
        return Ok(());
    };
    if Path::new(archive.file_path()).exists()
        && verify_summary_projection_archive_file_sha256(&archive, &plan.manifest_sha256)?
            == SummaryProjectionArchiveFileIdentity::Unavailable
    {
        if merge_summary_projection_archive_snapshot_fallback(
            input,
            &archive,
            plan.archive_range,
            plan.archive_rollups_are_exact,
            plan.replay_coverage,
            &plan.exact_ranges,
            "archive_snapshot_v2_fallback",
        )
        .await?
        {
            return Ok(());
        }
        mark_summary_projection_boundary_archive_unavailable(
            input,
            plan.archive_range,
            plan.archive_rollups_are_exact,
            plan.replay_coverage,
            &plan.exact_ranges,
        )?;
        return Ok(());
    }
    merge_summary_projection_boundary_archive_raw(
        input,
        &archive,
        plan.archive_range,
        plan.archive_rollups_are_exact,
        plan.replay_coverage,
        &plan.exact_ranges,
        &plan.manifest_sha256,
    )
    .await
}
