struct SummaryProjectionBuildWindow {
    build_started_at: Instant,
    end: DateTime<Utc>,
    archive_start: DateTime<Utc>,
    live_start: DateTime<Utc>,
    protected_boundary_buckets: HashSet<i64>,
    hourly_rollup_totals: HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_non_success_tokens: HashMap<(i64, Option<i64>), i64>,
    hourly_rollup_usage: HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    has_any_completed_archive: bool,
}

async fn load_summary_projection_build_window(
    state: &AppState,
    pool: &Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    end: DateTime<Utc>,
) -> Result<SummaryProjectionBuildWindow> {
    let archive_start = end - summary_projection_exact_horizon(state.config.invocation_max_days);
    let live_start = end - SUMMARY_PROJECTION_MIN_EXACT_HORIZON;
    let protected_boundary_buckets = summary_projection_boundary_buckets(end);
    let rollup_range = (
        align_bucket_epoch(archive_start.timestamp(), 3_600, 0),
        align_bucket_epoch(end.timestamp(), 3_600, 0) + 3_600,
    );
    let (hourly_rollup_totals, hourly_rollup_non_success_tokens) =
        load_summary_projection_rollup_totals_in_range(pool, Some(rollup_range)).await?;
    let hourly_rollup_usage =
        load_summary_projection_rollup_usage_in_range(pool, Some(rollup_range)).await?;
    info!(
        ?mode,
        stage = "rollup_load",
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build stage completed"
    );
    #[cfg(test)]
    pause_summary_projection_test_interleave(
        pool,
        mode,
        SummaryProjectionTestInterleaveStage::AfterRollupLoad,
    )
    .await?;
    let has_any_completed_archive =
        !crate::stats::load_completed_invocation_archive_paths_in_range_bounded(pool, None, 1)
            .await?
            .is_empty();
    Ok(SummaryProjectionBuildWindow {
        build_started_at,
        end,
        archive_start,
        live_start,
        protected_boundary_buckets,
        hourly_rollup_totals,
        hourly_rollup_non_success_tokens,
        hourly_rollup_usage,
        has_any_completed_archive,
    })
}

struct SummaryProjectionCurrentArchiveAdmissionSetup {
    current_archive_admission: Vec<crate::stats::ArchiveBatchPathRow>,
    current_archive_account_manifest_refreshed_paths: HashSet<String>,
    current_archive_admission_exceeded: bool,
}

async fn load_summary_projection_current_archive_admission_setup(
    pool: &Pool<Sqlite>,
    live_start: DateTime<Utc>,
    has_any_completed_archive: bool,
) -> Result<SummaryProjectionCurrentArchiveAdmissionSetup> {
    let current_archive_admission = if has_any_completed_archive {
        load_summary_projection_current_archive_admission(pool, live_start).await?
    } else {
        Vec::new()
    };
    let current_archive_admission_paths = current_archive_admission
        .iter()
        .map(|archive| archive.file_path().to_string())
        .collect::<Vec<_>>();
    let current_archive_account_manifest_refreshed_paths =
        load_summary_projection_archive_manifest_refreshed_paths(
            pool,
            &current_archive_admission_paths,
        )
        .await?;
    let current_archive_admission_exceeded =
        current_archive_admission.len() > SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES;
    Ok(SummaryProjectionCurrentArchiveAdmissionSetup {
        current_archive_admission,
        current_archive_account_manifest_refreshed_paths,
        current_archive_admission_exceeded,
    })
}

struct SummaryProjectionAllTimeArchiveAdmissionSetup {
    all_time_archives: Vec<crate::stats::ArchiveBatchPathRow>,
    all_time_archive_admission_exceeded: bool,
    all_time_manifest_high_watermark_id: Option<i64>,
    all_time_archive_manifest_sha256: HashMap<String, String>,
    all_time_archive_replay_coverage: HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    all_time_archive_account_ids_by_file: HashMap<String, HashSet<i64>>,
    all_time_archive_account_manifest_refreshed_paths: HashSet<String>,
    all_time_account_manifest_admission_attempted: bool,
    all_time_account_manifest_admission_exceeded: bool,
}

async fn load_summary_projection_all_time_archive_admission_setup(
    pool: &Pool<Sqlite>,
    all_time_was_fully_rebuilt: bool,
    has_any_completed_archive: bool,
    previous_all_time_account_manifest_admission_blocked_at: Option<Instant>,
) -> Result<SummaryProjectionAllTimeArchiveAdmissionSetup> {
    let (
        all_time_archives,
        all_time_archive_admission_exceeded,
        all_time_manifest_high_watermark_id,
    ) = if all_time_was_fully_rebuilt && has_any_completed_archive {
        let archives = crate::stats::load_completed_invocation_archive_paths_in_range_bounded(
            pool,
            None,
            SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES,
        )
        .await?;
        if archives.len() > SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
            match summary_projection_overflowed_all_time_manifests_have_complete_rollups(pool)
                .await?
            {
                Some(high_watermark_id) => (Vec::new(), false, Some(high_watermark_id)),
                None => (Vec::new(), true, None),
            }
        } else {
            (archives, false, None)
        }
    } else {
        (Vec::new(), false, None)
    };
    let all_time_archive_paths = all_time_archives
        .iter()
        .map(|archive| archive.file_path().to_string())
        .collect::<Vec<_>>();
    let all_time_archive_manifest_sha256 =
        load_summary_projection_archive_manifest_sha256(pool, &all_time_archive_paths).await?;
    let all_time_archive_replay_coverage =
        load_summary_projection_archive_replay_coverage(pool, &all_time_archive_paths).await?;
    let mut all_time_archive_account_ids_by_file = HashMap::<String, HashSet<i64>>::new();
    let mut all_time_archive_account_manifest_refreshed_paths = HashSet::new();
    let all_time_account_manifest_admission_attempted = all_time_was_fully_rebuilt
        && summary_projection_manifest_admission_retry_is_due(
            previous_all_time_account_manifest_admission_blocked_at,
            Instant::now(),
        );
    let mut all_time_account_manifest_admission_exceeded =
        previous_all_time_account_manifest_admission_blocked_at.is_some()
            && !all_time_account_manifest_admission_attempted;
    if all_time_account_manifest_admission_attempted
        && all_time_manifest_high_watermark_id.is_none()
    {
        match load_summary_projection_archive_manifest_account_sets(pool, &all_time_archive_paths)
            .await
        {
            Ok(accounts) => {
                all_time_archive_account_ids_by_file = accounts;
                all_time_archive_account_manifest_refreshed_paths =
                    load_summary_projection_archive_manifest_refreshed_paths(
                        pool,
                        &all_time_archive_paths,
                    )
                    .await?;
            }
            Err(error)
                if error.to_string().starts_with(
                    "summary projection archive account manifest exceeded bounded row budget",
                ) =>
            {
                all_time_account_manifest_admission_exceeded = true;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(SummaryProjectionAllTimeArchiveAdmissionSetup {
        all_time_archives,
        all_time_archive_admission_exceeded,
        all_time_manifest_high_watermark_id,
        all_time_archive_manifest_sha256,
        all_time_archive_replay_coverage,
        all_time_archive_account_ids_by_file,
        all_time_archive_account_manifest_refreshed_paths,
        all_time_account_manifest_admission_attempted,
        all_time_account_manifest_admission_exceeded,
    })
}

struct SummaryProjectionLiveSourceSetup {
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    completed_manifest_high_watermark_id: Option<i64>,
    live_history_admission: Option<crate::stats::BoundedLiveInvocationIds>,
    live_high_watermark_id: i64,
    live_preview_cache: HashMap<i64, UpstreamAccountInvocationPreviewRow>,
}

async fn load_summary_projection_live_source_setup(
    pool: &Pool<Sqlite>,
    all_time_was_fully_rebuilt: bool,
    has_any_completed_archive: bool,
) -> Result<SummaryProjectionLiveSourceSetup> {
    let rollup_live_cursor = load_summary_projection_rollup_live_cursor(pool).await?;
    let account_rollup_live_cursor =
        load_summary_projection_account_rollup_live_cursor(pool).await?;
    let completed_manifest_high_watermark_id =
        summary_projection_completed_manifest_high_watermark(pool).await?;
    let live_history_admission = if all_time_was_fully_rebuilt && !has_any_completed_archive {
        summary_projection_live_history_within_aggregate_budget(pool).await?
    } else {
        None
    };
    let live_high_watermark_id = load_summary_projection_live_high_watermark(pool).await?;
    Ok(SummaryProjectionLiveSourceSetup {
        rollup_live_cursor,
        account_rollup_live_cursor,
        completed_manifest_high_watermark_id,
        live_history_admission,
        live_high_watermark_id,
        live_preview_cache: HashMap::new(),
    })
}

struct SummaryProjectionLiveExactAdmissionInput<'a> {
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    range: ExactUtcRange,
    high_watermark_id: i64,
    preview_cache: &'a mut HashMap<i64, UpstreamAccountInvocationPreviewRow>,
}

async fn query_summary_projection_live_exact_admission(
    input: SummaryProjectionLiveExactAdmissionInput<'_>,
) -> Result<SummaryProjectionLiveAdmission> {
    info!(
        ?input.mode,
        stage = "live_exact_admission",
        "summary projection build stage started"
    );
    let admission =
        query_summary_projection_live_rows_with_budget(SummaryProjectionLiveRowsQuery {
            pool: input.pool,
            source_scope: InvocationSourceScope::All,
            range: input.range,
            high_watermark_id: input.high_watermark_id,
            min_id_exclusive: None,
            upstream_account_id: None,
            limit: summary_projection_exact_record_limit() + 1,
            in_progress_only: false,
            preview_cache: input.preview_cache,
            telemetry: UpstreamAccountActivityPreviewReadTelemetry {
                route: "summary_projection",
                builder: "bounded_exact_horizon",
                purpose: "summary_projection_exact_horizon",
            },
        })
        .await?;
    info!(
        ?input.mode,
        stage = "live_exact_admission",
        elapsed_ms = input.build_started_at.elapsed().as_millis() as u64,
        "summary projection build stage completed"
    );
    Ok(admission)
}

fn materialize_summary_projection_exact_live_rows(
    rows: Vec<UpstreamAccountInvocationPreviewRow>,
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
) -> Result<(HashMap<String, SummaryProjectionRecord>, usize)> {
    let mut records_by_invoke_id = HashMap::new();
    for row in rows {
        if let Some((_record_key, record)) = summary_projection_live_record_from_preview(
            row,
            rollup_live_cursor,
            account_rollup_live_cursor,
            hourly_rollup_totals,
            hourly_rollup_usage,
        ) {
            let record_key = summary_projection_record_insert_key(&records_by_invoke_id, &record);
            records_by_invoke_id.insert(record_key, record);
        }
    }
    let exact_record_bytes = records_by_invoke_id
        .values()
        .map(|record| summary_projection_preview_row_bytes(&record.row))
        .sum::<usize>();
    ensure_summary_projection_resident_record_bytes(exact_record_bytes, 0)?;
    Ok((records_by_invoke_id, exact_record_bytes))
}

async fn collect_summary_projection_known_account_ids(
    pool: &Pool<Sqlite>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    all_time_archive_account_ids_by_file: &HashMap<String, HashSet<i64>>,
    records_by_invoke_id: &HashMap<String, SummaryProjectionRecord>,
    current_records_by_invoke_id: &HashMap<String, SummaryProjectionRecord>,
) -> Result<HashSet<i64>> {
    let mut known_account_ids = records_by_invoke_id
        .values()
        .filter_map(|record| record.row.upstream_account_id)
        .filter(|account_id| *account_id > 0)
        .collect::<HashSet<_>>();
    known_account_ids.extend(
        current_records_by_invoke_id
            .values()
            .filter_map(|record| record.row.upstream_account_id)
            .filter(|account_id| *account_id > 0),
    );
    known_account_ids.extend(
        all_time_archive_account_ids_by_file
            .values()
            .flat_map(|account_ids| account_ids.iter().copied()),
    );
    known_account_ids.extend(load_summary_projection_durable_account_ids(pool).await?);
    known_account_ids.extend(
        hourly_rollup_totals
            .keys()
            .filter_map(|(_, account_id)| *account_id)
            .filter(|account_id| *account_id > 0),
    );
    known_account_ids.extend(
        hourly_rollup_usage
            .keys()
            .filter_map(|(_, account_id)| *account_id)
            .filter(|account_id| *account_id > 0),
    );
    if known_account_ids.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
        return Err(anyhow!(
            "summary projection account coverage exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ACCOUNTS})"
        ));
    }
    Ok(known_account_ids)
}

struct SummaryProjectionLiveIndexesInput<'a> {
    state: &'a AppState,
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    end: DateTime<Utc>,
    live_start: DateTime<Utc>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    live_sources: SummaryProjectionLiveSourceSetup,
    all_time_archive_account_ids_by_file: &'a HashMap<String, HashSet<i64>>,
}

struct SummaryProjectionLiveIndexes {
    rollup_live_cursor: i64,
    account_rollup_live_cursor: Option<i64>,
    completed_manifest_high_watermark_id: Option<i64>,
    live_history_admission: Option<crate::stats::BoundedLiveInvocationIds>,
    live_high_watermark_id: i64,
    live_preview_cache: HashMap<i64, UpstreamAccountInvocationPreviewRow>,
    initial_live_admission_gaps: Vec<SummaryProjectionLiveCandidate>,
    initial_live_admission_overflow: Option<SummaryProjectionLiveCandidate>,
    records_by_invoke_id: HashMap<String, SummaryProjectionRecord>,
    exact_record_bytes: usize,
    current_records_by_invoke_id: HashMap<String, SummaryProjectionRecord>,
    current_record_bytes: usize,
    current_source_unavailable_from_rank: Option<usize>,
    current_account_source_unavailable_from_rank: HashMap<i64, usize>,
    recent_index_complete: bool,
    recent_index_overflow_at: Option<DateTime<Utc>>,
    known_account_ids: HashSet<i64>,
}

async fn admit_summary_projection_live_indexes(
    input: SummaryProjectionLiveIndexesInput<'_>,
) -> Result<SummaryProjectionLiveIndexes> {
    let SummaryProjectionLiveIndexesInput {
        state,
        pool,
        mode,
        build_started_at,
        end,
        live_start,
        hourly_rollup_totals,
        hourly_rollup_usage,
        mut live_sources,
        all_time_archive_account_ids_by_file,
    } = input;
    let SummaryProjectionLiveAdmission {
        rows,
        gaps: initial_live_admission_gaps,
        overflow: initial_live_admission_overflow,
    } = query_summary_projection_live_exact_admission(SummaryProjectionLiveExactAdmissionInput {
        pool,
        mode,
        build_started_at,
        range: ExactUtcRange {
            start: live_start,
            end,
        },
        high_watermark_id: live_sources.live_high_watermark_id,
        preview_cache: &mut live_sources.live_preview_cache,
    })
    .await?;
    let (records_by_invoke_id, exact_record_bytes) =
        materialize_summary_projection_exact_live_rows(
            rows,
            live_sources.rollup_live_cursor,
            live_sources.account_rollup_live_cursor,
            hourly_rollup_totals,
            hourly_rollup_usage,
        )?;
    let recent_admission =
        admit_summary_projection_current_rows(SummaryProjectionCurrentAdmissionInput {
            state,
            pool,
            mode,
            build_started_at,
            end,
            live_high_watermark_id: live_sources.live_high_watermark_id,
            live_preview_cache: &mut live_sources.live_preview_cache,
        })
        .await?;
    let SummaryProjectionCurrentIndex {
        current_records_by_invoke_id,
        current_record_bytes,
        current_source_unavailable_from_rank,
        current_account_source_unavailable_from_rank,
        recent_index_complete,
        recent_index_overflow_at,
    } = materialize_summary_projection_current_index(SummaryProjectionCurrentIndexInput {
        mode,
        build_started_at,
        rollup_live_cursor: live_sources.rollup_live_cursor,
        account_rollup_live_cursor: live_sources.account_rollup_live_cursor,
        hourly_rollup_totals,
        hourly_rollup_usage,
        records_by_invoke_id: &records_by_invoke_id,
        exact_record_bytes,
        admission: recent_admission,
    })?;
    let known_account_ids = collect_summary_projection_known_account_ids(
        pool,
        hourly_rollup_totals,
        hourly_rollup_usage,
        all_time_archive_account_ids_by_file,
        &records_by_invoke_id,
        &current_records_by_invoke_id,
    )
    .await?;
    Ok(SummaryProjectionLiveIndexes {
        rollup_live_cursor: live_sources.rollup_live_cursor,
        account_rollup_live_cursor: live_sources.account_rollup_live_cursor,
        completed_manifest_high_watermark_id: live_sources.completed_manifest_high_watermark_id,
        live_history_admission: live_sources.live_history_admission,
        live_high_watermark_id: live_sources.live_high_watermark_id,
        live_preview_cache: live_sources.live_preview_cache,
        initial_live_admission_gaps,
        initial_live_admission_overflow,
        records_by_invoke_id,
        exact_record_bytes,
        current_records_by_invoke_id,
        current_record_bytes,
        current_source_unavailable_from_rank,
        current_account_source_unavailable_from_rank,
        recent_index_complete,
        recent_index_overflow_at,
        known_account_ids,
    })
}

struct SummaryProjectionBoundaryAdmissionSetup {
    archives: Vec<crate::stats::ArchiveBatchPathRow>,
    paged_boundary_manifest_high_watermark_id: Option<i64>,
    paged_boundary_manifest_unknown_coverage_ranges: Vec<ExactUtcRange>,
    paged_boundary_manifest_unknown_account_coverage_ranges: Vec<ExactUtcRange>,
}

async fn admit_summary_projection_boundary_manifests(
    pool: &Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    exact_horizon: ExactUtcRange,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
) -> Result<SummaryProjectionBoundaryAdmissionSetup> {
    info!(
        ?mode,
        stage = "boundary_manifest_admission",
        "summary projection build stage started"
    );
    let boundary_archive_admission =
        crate::stats::load_completed_invocation_archive_paths_in_range_bounded(
            pool,
            Some((exact_horizon.start, exact_horizon.end)),
            SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES,
        )
        .await?;
    info!(
        ?mode,
        stage = "boundary_manifest_admission",
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build stage completed"
    );
    let mut unknown_coverage_ranges = Vec::new();
    let mut unknown_account_coverage_ranges = Vec::new();
    let (archives, paged_boundary_manifest_high_watermark_id) = if boundary_archive_admission.len()
        > SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES
    {
        let Some(coverage) =
            summary_projection_overflowed_boundary_manifest_coverage(pool, exact_horizon).await?
        else {
            return Err(anyhow!(
                "summary projection overflowed boundary manifests lack a durable high watermark"
            ));
        };
        unknown_coverage_ranges = coverage.unknown_coverage_ranges;
        if !mode.includes_all_time() {
            let unproven = summary_projection_overflowed_boundary_unproven_ranges_scoped(
                pool,
                exact_horizon,
                coverage.high_watermark_id,
                Some(hourly_rollup_totals),
                Some(hourly_rollup_usage),
            )
            .await?;
            let mut global_unproven = unproven.global;
            global_unproven.retain(|range| range.start < range.end);
            unknown_coverage_ranges.extend(global_unproven);
            unknown_account_coverage_ranges.extend(unproven.account);
        }
        (Vec::new(), Some(coverage.high_watermark_id))
    } else {
        (boundary_archive_admission, None)
    };
    Ok(SummaryProjectionBoundaryAdmissionSetup {
        archives,
        paged_boundary_manifest_high_watermark_id,
        paged_boundary_manifest_unknown_coverage_ranges: unknown_coverage_ranges,
        paged_boundary_manifest_unknown_account_coverage_ranges: unknown_account_coverage_ranges,
    })
}

struct SummaryProjectionArchiveMetadataInput<'a> {
    pool: &'a Pool<Sqlite>,
    archives: &'a [crate::stats::ArchiveBatchPathRow],
    archive_account_ids_by_file: HashMap<String, HashSet<i64>>,
    archive_coverage_ranges_by_file: HashMap<String, ExactUtcRange>,
    known_account_ids: &'a mut HashSet<i64>,
}

struct SummaryProjectionArchiveMetadata {
    archive_account_ids_by_file: HashMap<String, HashSet<i64>>,
    archive_coverage_ranges_by_file: HashMap<String, ExactUtcRange>,
    archive_row_counts: HashMap<String, i64>,
    archive_manifest_sha256: HashMap<String, String>,
    archive_replay_coverage: HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    usage_rollup_progress: HashMap<String, i64>,
    cached_archive_account_id_count: usize,
}

async fn load_summary_projection_archive_metadata(
    input: SummaryProjectionArchiveMetadataInput<'_>,
) -> Result<SummaryProjectionArchiveMetadata> {
    let SummaryProjectionArchiveMetadataInput {
        pool,
        archives,
        mut archive_account_ids_by_file,
        mut archive_coverage_ranges_by_file,
        known_account_ids,
    } = input;
    let archive_paths = archives
        .iter()
        .map(|archive| archive.file_path().to_string())
        .collect::<Vec<_>>();
    let archive_path_set = archive_paths.iter().cloned().collect::<HashSet<_>>();
    archive_account_ids_by_file.retain(|path, _| archive_path_set.contains(path));
    archive_coverage_ranges_by_file.retain(|path, _| archive_path_set.contains(path));
    ensure_summary_projection_archive_account_cache_budget(&archive_account_ids_by_file, "cache")?;
    let archive_row_counts =
        load_summary_projection_archive_row_counts(pool, &archive_paths).await?;
    let archive_manifest_sha256 =
        load_summary_projection_archive_manifest_sha256(pool, &archive_paths).await?;
    let archive_replay_coverage =
        load_summary_projection_archive_replay_coverage(pool, &archive_paths).await?;
    let manifest_accounts = load_summary_projection_archive_metadata_accounts(
        pool,
        &archive_paths,
        &archive_replay_coverage,
    )
    .await?;
    for (file_path, account_id) in manifest_accounts {
        if archive_path_set.contains(&file_path) {
            archive_account_ids_by_file
                .entry(file_path)
                .or_default()
                .insert(account_id);
        }
    }
    ensure_summary_projection_archive_account_cache_budget(
        &archive_account_ids_by_file,
        "manifest",
    )?;
    let cached_archive_account_id_count = archive_account_ids_by_file
        .values()
        .map(HashSet::len)
        .sum::<usize>();
    for account_ids in archive_account_ids_by_file.values() {
        known_account_ids.extend(account_ids.iter().copied());
    }
    let usage_rollup_progress =
        load_usage_breakdown_archive_progress_by_file_path(pool, &archive_paths)
            .await
            .map_err(|error| {
                anyhow!("summary projection usage progress hydration failed: {error:?}")
            })?;
    Ok(SummaryProjectionArchiveMetadata {
        archive_account_ids_by_file,
        archive_coverage_ranges_by_file,
        archive_row_counts,
        archive_manifest_sha256,
        archive_replay_coverage,
        usage_rollup_progress,
        cached_archive_account_id_count,
    })
}

async fn load_summary_projection_archive_metadata_accounts(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
    archive_replay_coverage: &HashMap<String, SummaryProjectionArchiveReplayCoverage>,
) -> Result<Vec<(String, i64)>> {
    if archive_paths.iter().all(|path| {
        archive_replay_coverage
            .get(path)
            .is_some_and(|coverage| coverage.account_stats && coverage.usage_breakdown)
    }) {
        return Ok(Vec::new());
    }
    load_summary_projection_archive_manifest_accounts(pool, archive_paths).await
}

fn ensure_summary_projection_archive_account_cache_budget(
    archive_account_ids_by_file: &HashMap<String, HashSet<i64>>,
    source: &str,
) -> Result<()> {
    let account_count = archive_account_ids_by_file
        .values()
        .map(HashSet::len)
        .sum::<usize>();
    if account_count > summary_projection_exact_record_limit() {
        return Err(anyhow!(
            "summary projection archive account {source} exceeded bounded budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    Ok(())
}

struct SummaryProjectionArchiveAccountDiscoveryState<'a> {
    mode: SummaryProjectionBuildMode,
    build_started_at: Instant,
    archive_start: DateTime<Utc>,
    end: DateTime<Utc>,
    archive_replay_coverage: &'a HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    archive_row_counts: &'a HashMap<String, i64>,
    archive_manifest_sha256: &'a HashMap<String, String>,
    archive_account_ids_by_file: &'a mut HashMap<String, HashSet<i64>>,
    archive_actual_coverage_ranges: &'a mut HashMap<String, ExactUtcRange>,
    known_account_ids: &'a mut HashSet<i64>,
    cached_archive_account_id_count: &'a mut usize,
    all_time_source_unavailable_from_archive_ranges: &'a mut bool,
}

fn prepare_summary_projection_archive_account_discovery(
    input: &mut SummaryProjectionArchiveAccountDiscoveryState<'_>,
    archive: &crate::stats::ArchiveBatchPathRow,
) -> Result<Option<(ExactUtcRange, SummaryProjectionArchiveReplayCoverage)>> {
    let Some(archive_range) = summary_projection_archive_overlap_range(
        archive,
        ExactUtcRange {
            start: input.archive_start,
            end: input.end,
        },
    ) else {
        return Ok(None);
    };
    let cached_coverage_is_sufficient = summary_projection_archive_has_coverage_bounds(archive)
        || input
            .archive_actual_coverage_ranges
            .contains_key(archive.file_path());
    if input
        .archive_account_ids_by_file
        .contains_key(archive.file_path())
        && cached_coverage_is_sufficient
    {
        if let Some(account_ids) = input.archive_account_ids_by_file.get(archive.file_path()) {
            input.known_account_ids.extend(account_ids.iter().copied());
        }
        if input.known_account_ids.len() > summary_projection_exact_record_limit() {
            return Err(anyhow!(
                "summary projection archive account coverage exceeded bounded budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
            ));
        }
        return Ok(None);
    }
    let row_count = input.archive_row_counts.get(archive.file_path()).copied();
    if !archive.has_materialized_historical_rollups()
        && row_count.is_some_and(|count| count > summary_projection_exact_record_limit() as i64)
    {
        return Err(anyhow!(
            "summary projection unmaterialized archive account discovery exceeded bounded row budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    let replay_coverage = summary_projection_effective_replay_coverage(
        archive,
        input
            .archive_replay_coverage
            .get(archive.file_path())
            .copied()
            .unwrap_or_default(),
    );
    if archive.has_materialized_historical_rollups()
        && row_count.is_some_and(|count| count > summary_projection_exact_record_limit() as i64)
        && replay_coverage.account_stats
        && replay_coverage.usage_breakdown
    {
        return Ok(None);
    }
    if archive.has_materialized_historical_rollups()
        && row_count.is_some_and(|count| count > summary_projection_exact_record_limit() as i64)
    {
        return Err(anyhow!(
            "summary projection cannot prove account coverage for a large materialized archive without complete replay coverage"
        ));
    }
    Ok(Some((archive_range, replay_coverage)))
}

async fn discover_summary_projection_archive_accounts(
    input: &mut SummaryProjectionArchiveAccountDiscoveryState<'_>,
    archive: &crate::stats::ArchiveBatchPathRow,
) -> Result<()> {
    let Some((archive_range, replay_coverage)) =
        prepare_summary_projection_archive_account_discovery(input, archive)?
    else {
        return Ok(());
    };
    let manifest_sha256 = input
        .archive_manifest_sha256
        .get(archive.file_path())
        .ok_or_else(|| {
            anyhow!(
                "summary projection archive manifest SHA is missing for {}",
                archive.file_path()
            )
        })?;
    if Path::new(archive.file_path()).exists() {
        verify_summary_projection_archive_file_sha256(archive, manifest_sha256)?;
    }
    let Some((archive_pool, temp_cleanup)) =
        crate::stats::open_invocation_archive_batch_pool(archive, "summary-projection").await?
    else {
        if !replay_coverage.supports_unavailable_archive()
            && !archive.has_materialized_historical_rollups()
        {
            *input.all_time_source_unavailable_from_archive_ranges = true;
        }
        return Ok(());
    };
    require_summary_projection_archive_file_sha256(archive, manifest_sha256)?;
    if !summary_projection_archive_has_coverage_bounds(archive)
        && let Some(actual_range) = load_summary_projection_archive_coverage_range(&archive_pool)
            .await
            .map_err(|error| {
                anyhow!(
                    "summary projection archive coverage hydration failed for {}: {error:?}",
                    archive.file_path()
                )
            })?
    {
        input
            .archive_actual_coverage_ranges
            .insert(archive.file_path().to_string(), actual_range);
    }
    let account_ids = load_summary_projection_archive_account_ids(&archive_pool, archive_range)
        .await
        .map_err(|error| {
            anyhow!(
                "summary projection archive account discovery failed for {}: {error:?}",
                archive.file_path()
            )
        })?;
    input.known_account_ids.extend(account_ids.iter().copied());
    *input.cached_archive_account_id_count =
        (*input.cached_archive_account_id_count).saturating_add(account_ids.len());
    if *input.cached_archive_account_id_count > summary_projection_exact_record_limit() {
        return Err(anyhow!(
            "summary projection archive account cache exceeded bounded budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    if input.known_account_ids.len() > summary_projection_exact_record_limit() {
        return Err(anyhow!(
            "summary projection archive account coverage exceeded bounded budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    input
        .archive_account_ids_by_file
        .insert(archive.file_path().to_string(), account_ids);
    require_summary_projection_archive_file_sha256(archive, manifest_sha256)?;
    archive_pool.close().await;
    drop(temp_cleanup);
    Ok(())
}

async fn discover_summary_projection_archive_account_ids(
    mut input: SummaryProjectionArchiveAccountDiscoveryState<'_>,
    archives: &[crate::stats::ArchiveBatchPathRow],
) -> Result<()> {
    info!(
        ?input.mode,
        stage = "archive_account_discovery",
        "summary projection build stage started"
    );
    if input.mode.includes_all_time() || archives.len() <= SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        for archive in archives {
            discover_summary_projection_archive_accounts(&mut input, archive).await?;
        }
    }
    info!(
        ?input.mode,
        stage = "archive_account_discovery",
        elapsed_ms = input.build_started_at.elapsed().as_millis() as u64,
        "summary projection archive account discovery completed"
    );
    Ok(())
}

struct SummaryProjectionArchiveReplacementBuckets {
    exact_global_total_rollup_buckets: HashSet<i64>,
    exact_account_total_rollup_buckets: HashSet<i64>,
    exact_global_usage_rollup_buckets: HashSet<i64>,
    exact_account_usage_rollup_buckets: HashSet<i64>,
}

fn mark_summary_projection_archive_replacement_buckets(
    archives: &[crate::stats::ArchiveBatchPathRow],
    archive_start: DateTime<Utc>,
    end: DateTime<Utc>,
    protected_boundary_buckets: &HashSet<i64>,
    archive_replay_coverage: &HashMap<String, SummaryProjectionArchiveReplayCoverage>,
) -> Result<SummaryProjectionArchiveReplacementBuckets> {
    let mut buckets = SummaryProjectionArchiveReplacementBuckets {
        exact_global_total_rollup_buckets: HashSet::new(),
        exact_account_total_rollup_buckets: HashSet::new(),
        exact_global_usage_rollup_buckets: HashSet::new(),
        exact_account_usage_rollup_buckets: HashSet::new(),
    };
    for archive in archives {
        let Some(archive_range) = summary_projection_archive_overlap_range(
            archive,
            ExactUtcRange {
                start: archive_start,
                end,
            },
        ) else {
            continue;
        };
        let replay_coverage = summary_projection_effective_replay_coverage(
            archive,
            archive_replay_coverage
                .get(archive.file_path())
                .copied()
                .unwrap_or_default(),
        );
        if summary_projection_archive_coverage_range(archive).is_some() {
            summary_projection_mark_exact_replacement_buckets(
                SummaryProjectionExactReplacementInput {
                    archive_has_materialized_rollups: archive.has_materialized_historical_rollups(),
                    replay_coverage,
                    exact_range: archive_range,
                    _protected_boundary_buckets: protected_boundary_buckets,
                    exact_global_total_rollup_buckets: &mut buckets
                        .exact_global_total_rollup_buckets,
                    exact_account_total_rollup_buckets: &mut buckets
                        .exact_account_total_rollup_buckets,
                    exact_global_usage_rollup_buckets: &mut buckets
                        .exact_global_usage_rollup_buckets,
                    exact_account_usage_rollup_buckets: &mut buckets
                        .exact_account_usage_rollup_buckets,
                },
            )?;
        }
    }
    Ok(buckets)
}

struct SummaryProjectionArchiveFallbackBucketInput<'a> {
    archives: &'a [crate::stats::ArchiveBatchPathRow],
    archive_start: DateTime<Utc>,
    end: DateTime<Utc>,
    known_account_ids: &'a HashSet<i64>,
    archive_replay_coverage: &'a HashMap<String, SummaryProjectionArchiveReplayCoverage>,
    hourly_rollup_totals: &'a HashMap<(i64, Option<i64>), StatsTotals>,
    hourly_rollup_usage: &'a HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    exact_archive_buckets: &'a mut HashSet<i64>,
}

fn extend_summary_projection_archive_fallback_buckets(
    input: SummaryProjectionArchiveFallbackBucketInput<'_>,
) -> Result<()> {
    for archive in input.archives {
        let Some(archive_range) = summary_projection_archive_overlap_range(
            archive,
            ExactUtcRange {
                start: input.archive_start,
                end: input.end,
            },
        ) else {
            continue;
        };
        let replay_coverage = summary_projection_effective_replay_coverage(
            archive,
            input
                .archive_replay_coverage
                .get(archive.file_path())
                .copied()
                .unwrap_or_default(),
        );
        summary_projection_exact_range_fits_bucket_budget(archive_range)?;
        let raw_fallback_ranges = summary_projection_archive_exact_ranges_with_coverage(
            archive.has_materialized_historical_rollups(),
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
        summary_projection_extend_exact_buckets_for_ranges(
            input.exact_archive_buckets,
            raw_fallback_ranges,
        )?;
    }
    Ok(())
}
