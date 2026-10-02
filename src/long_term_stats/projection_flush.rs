use super::*;

pub(super) async fn flush_long_term_projection(
    state: &AppState,
    trigger: &'static str,
) -> Result<LongTermProjectionFlushOutcome> {
    let Some(_execution_lease) =
        crate::maintenance_store::try_acquire_task_execution("long_term_projection")
    else {
        return Ok(LongTermProjectionFlushOutcome::DeferredByPressure {
            retry_at: Some(Instant::now() + Duration::from_secs(1)),
        });
    };
    let observation = crate::TaskExecutionObservation::begin(
        "long_term_projection",
        &crate::maintenance_store::task_title_for_observation("long_term_projection"),
        trigger,
        crate::maintenance_store::task_execution_class("long_term_projection"),
        "processing",
    );
    let result = flush_long_term_projection_unlocked(state, trigger).await;
    let status = match &result {
        Ok(LongTermProjectionFlushOutcome::Completed) => "success",
        Ok(LongTermProjectionFlushOutcome::DeferredByPressure { .. }) => "skipped",
        Err(_) => "failed",
    };
    observation.finish_with_status(status);
    result
}
