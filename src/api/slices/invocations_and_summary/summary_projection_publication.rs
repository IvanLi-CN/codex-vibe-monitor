async fn publish_summary_all_time_projection_checkpoint(
    state: &AppState,
    checkpoint: SummaryAllTimeProjectionCheckpointRow,
) -> Result<()> {
    let Some(mut context) =
        prepare_summary_projection_checkpoint_publication(state, checkpoint).await?
    else {
        return Ok(());
    };
    if !finalize_summary_projection_checkpoint_global(&mut context).await? {
        return Ok(());
    }
    finalize_summary_projection_checkpoint_accounts(&mut context).await?;
    publish_summary_projection_checkpoint_finish(context).await
}
