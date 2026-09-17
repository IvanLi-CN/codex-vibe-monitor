struct SummaryProjectionBuildInitialState {
    build_started_at: Instant,
    end: DateTime<Utc>,
    all_time_was_fully_rebuilt: bool,
    previous_all_time: PreviousSummaryProjectionAllTime,
    window: SummaryProjectionBuildWindow,
    current_archive: SummaryProjectionCurrentArchiveAdmissionSetup,
    all_time_archive: SummaryProjectionAllTimeArchiveAdmissionSetup,
    live_indexes: SummaryProjectionLiveIndexes,
    boundary: SummaryProjectionBoundaryAdmissionSetup,
    metadata: SummaryProjectionArchiveMetadata,
}

async fn load_summary_projection_build_initial_state(
    state: &AppState,
    pool: &Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    previous_all_time: Option<PreviousSummaryProjectionAllTime>,
) -> Result<SummaryProjectionBuildInitialState> {
    let build_started_at = Instant::now();
    info!(
        ?mode,
        stage = "rollup_load",
        "summary projection build stage started"
    );
    let end = Utc::now() + ChronoDuration::seconds(1);
    let previous_all_time = previous_all_time.unwrap_or_default();
    let all_time_was_fully_rebuilt = mode.includes_all_time()
        && summary_projection_manifest_admission_retry_is_due(
            previous_all_time.all_time_manifest_admission_blocked_at,
            Instant::now(),
        );
    let window =
        load_summary_projection_build_window(state, pool, mode, build_started_at, end).await?;
    let current_archive = load_summary_projection_current_archive_admission_setup(
        pool,
        window.live_start,
        window.has_any_completed_archive,
    )
    .await?;
    let all_time_archive = load_summary_projection_all_time_archive_admission_setup(
        pool,
        all_time_was_fully_rebuilt,
        window.has_any_completed_archive,
        previous_all_time.all_time_account_manifest_admission_blocked_at,
    )
    .await?;
    let live_sources = load_summary_projection_live_source_setup(
        pool,
        all_time_was_fully_rebuilt,
        window.has_any_completed_archive,
    )
    .await?;
    let mut live_indexes =
        admit_summary_projection_live_indexes(SummaryProjectionLiveIndexesInput {
            state,
            pool,
            mode,
            build_started_at,
            end,
            live_start: window.live_start,
            hourly_rollup_totals: &window.hourly_rollup_totals,
            hourly_rollup_usage: &window.hourly_rollup_usage,
            live_sources,
            all_time_archive_account_ids_by_file: &all_time_archive
                .all_time_archive_account_ids_by_file,
        })
        .await?;
    let exact_horizon = ExactUtcRange {
        start: window.archive_start,
        end,
    };
    let boundary = admit_summary_projection_boundary_manifests(
        pool,
        mode,
        build_started_at,
        exact_horizon,
        &window.hourly_rollup_totals,
        &window.hourly_rollup_usage,
    )
    .await?;
    let metadata =
        load_summary_projection_archive_metadata(SummaryProjectionArchiveMetadataInput {
            pool,
            archives: &boundary.archives,
            archive_account_ids_by_file: previous_all_time.archive_account_ids_by_file.clone(),
            archive_coverage_ranges_by_file: previous_all_time
                .archive_coverage_ranges_by_file
                .clone(),
            known_account_ids: &mut live_indexes.known_account_ids,
        })
        .await?;
    Ok(SummaryProjectionBuildInitialState {
        build_started_at,
        end,
        all_time_was_fully_rebuilt,
        previous_all_time,
        window,
        current_archive,
        all_time_archive,
        live_indexes,
        boundary,
        metadata,
    })
}
