struct SummaryProjectionBuildPreparation {
    initial: SummaryProjectionBuildInitialState,
    durable_terminal_sequence_watermark: u64,
    derived: SummaryProjectionBuildDerivedState,
}

async fn prepare_summary_projection_build(
    state: &AppState,
    pool: &Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    previous_all_time: Option<PreviousSummaryProjectionAllTime>,
    durable_terminal_sequence_watermark: u64,
) -> Result<SummaryProjectionBuildPreparation> {
    let initial =
        load_summary_projection_build_initial_state(state, pool, mode, previous_all_time).await?;
    let mut pipeline =
        SummaryProjectionBuildPipelineState::new(initial, durable_terminal_sequence_watermark);
    pipeline.initialize_coverage(mode);
    pipeline.promote_current_records(state, pipeline.initial.window.live_start)?;
    pipeline.discover_archive_accounts(mode).await?;
    pipeline.plan_archive_sources(pool, mode).await?;
    let exact_sources = pipeline.admit_exact_live_sources(state, pool, mode).await?;
    pipeline.hydrate_archives(pool, mode, exact_sources).await?;
    pipeline
        .admit_current_archives_and_overlay(state, pool, mode)
        .await?;
    Ok(pipeline.into_preparation())
}

async fn hydrate_summary_projection_build<'a>(
    state: &'a AppState,
    pool: &'a Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    prepared: SummaryProjectionBuildPreparation,
) -> Result<SummaryProjectionFinalizationInput<'a>> {
    hydrate_summary_projection_build_with_context(state, pool, mode, prepared).await
}
