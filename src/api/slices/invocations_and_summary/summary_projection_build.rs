use super::*;

#[path = "summary_projection_build_adapters.rs"]
mod adapters;
#[path = "summary_projection_build_all_time.rs"]
mod all_time;
#[path = "summary_projection_build_all_time_helpers.rs"]
mod all_time_helpers;
#[path = "summary_projection_build_archive.rs"]
mod archive;
#[path = "summary_projection_build_current.rs"]
mod current;
#[path = "summary_projection_build_current_archive.rs"]
mod current_archive;
#[path = "summary_projection_build_entry.rs"]
mod entry;
#[path = "summary_projection_build_finalization.rs"]
mod finalization;
#[path = "summary_projection_build_finalization_helpers.rs"]
mod finalization_helpers;
#[path = "summary_projection_build_hydration.rs"]
mod hydration;
#[path = "summary_projection_build_live.rs"]
mod live;
#[path = "summary_projection_build_materialization.rs"]
mod materialization;
#[path = "summary_projection_build_paged_hydration.rs"]
mod paged_hydration;
#[path = "summary_projection_build_pipeline.rs"]
mod pipeline;
#[path = "summary_projection_build_pipeline_helpers.rs"]
mod pipeline_helpers;
#[path = "summary_projection_build_pipeline_hydration.rs"]
mod pipeline_hydration;
#[path = "summary_projection_build_pipeline_state.rs"]
mod pipeline_state;
#[path = "summary_projection_build_planning.rs"]
mod planning;
#[path = "summary_projection_build_runtime.rs"]
mod runtime;
#[path = "summary_projection_build_setup.rs"]
mod setup;
#[path = "summary_projection_build_terminal.rs"]
mod terminal;

pub(crate) use adapters::*;
pub(crate) use all_time::*;
pub(crate) use all_time_helpers::*;
pub(crate) use archive::*;
pub(crate) use current::*;
pub(crate) use current_archive::*;
pub(crate) use entry::*;
pub(crate) use finalization::*;
pub(crate) use finalization_helpers::*;
pub(crate) use hydration::*;
pub(crate) use live::*;
pub(crate) use materialization::*;
pub(crate) use paged_hydration::*;
pub(crate) use pipeline::*;
pub(crate) use pipeline_helpers::*;
pub(crate) use pipeline_hydration::*;
pub(crate) use pipeline_state::*;
pub(crate) use planning::*;
pub(crate) use runtime::*;
pub(crate) use setup::*;
pub(crate) use terminal::*;

pub(crate) async fn build_summary_projection_once(
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
