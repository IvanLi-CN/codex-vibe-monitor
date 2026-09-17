async fn build_summary_projection_once(
    state: &AppState,
    pool: &Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    previous_all_time: Option<PreviousSummaryProjectionAllTime>,
    durable_terminal_sequence_watermark: u64,
) -> Result<SummaryProjection> {
    let prepared = prepare_summary_projection_build(
        state,
        pool,
        mode,
        previous_all_time,
        durable_terminal_sequence_watermark,
    )
    .await?;
    let finalization = hydrate_summary_projection_build(state, pool, mode, prepared).await?;
    finalize_summary_projection(finalization).await
}
