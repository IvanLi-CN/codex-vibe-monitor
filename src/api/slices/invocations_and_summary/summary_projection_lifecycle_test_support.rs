use super::*;

#[cfg(test)]
tokio::task_local! {
    static BOOTSTRAP_TIMEOUT_ELAPSED: Cell<bool>;
}

#[cfg(test)]
tokio::task_local! {
    static SUMMARY_PROJECTION_TEST_SETUP_DEADLINE: (
        SummaryProjectionTestInterleave,
        Duration,
        std::sync::Arc<tokio::sync::Notify>,
    );
}

#[cfg(test)]
pub(crate) fn mark_bootstrap_timeout() {
    let _ = BOOTSTRAP_TIMEOUT_ELAPSED.try_with(|elapsed| elapsed.set(true));
}

#[cfg(test)]
pub(crate) fn summary_projection_test_setup_deadline() -> Option<(
    SummaryProjectionTestInterleave,
    Duration,
    std::sync::Arc<tokio::sync::Notify>,
)> {
    SUMMARY_PROJECTION_TEST_SETUP_DEADLINE
        .try_with(|(interleave, setup_deadline, stage_reached)| {
            (interleave.clone(), *setup_deadline, stage_reached.clone())
        })
        .ok()
}

#[cfg(test)]
pub(crate) async fn observe_bootstrap_timeout<T>(
    future: impl std::future::Future<Output = T>,
) -> (T, bool) {
    BOOTSTRAP_TIMEOUT_ELAPSED
        .scope(Cell::new(false), async move {
            let result = future.await;
            (result, BOOTSTRAP_TIMEOUT_ELAPSED.with(Cell::get))
        })
        .await
}
#[cfg(test)]
pub(crate) fn note_summary_coverage_v2_window() {
    let _ = SUMMARY_COVERAGE_TEST_V2_WINDOW_CALLS.try_with(|calls| {
        calls.set(calls.get().saturating_add(1));
    });
}

#[cfg(test)]
pub(crate) fn note_summary_coverage_checkpoint_advance() {
    let _ = SUMMARY_COVERAGE_TEST_CHECKPOINT_ADVANCE_CALLS.try_with(|calls| {
        calls.set(calls.get().saturating_add(1));
    });
}

#[cfg(test)]
pub(crate) fn note_summary_projection_historical_identity_hydration(rows: usize) {
    let _ = SUMMARY_PROJECTION_TEST_HISTORICAL_IDENTITY_HYDRATION_ROWS.try_with(|count| {
        count.set(count.get().saturating_add(rows));
    });
}

#[cfg(test)]
pub(crate) fn note_summary_projection_boundary_archive_hydration() {
    let _ = SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS.try_with(|calls| {
        calls.set(calls.get().saturating_add(1));
    });
}
#[cfg(test)]
pub(crate) async fn refresh_summary_snapshots_with_test_setup_deadline(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
    setup_deadline: Duration,
    interleave: SummaryProjectionTestInterleave,
    stage_reached: std::sync::Arc<tokio::sync::Notify>,
) -> Result<()> {
    SUMMARY_PROJECTION_TEST_SETUP_DEADLINE
        .scope(
            (interleave, setup_deadline, stage_reached),
            refresh_summary_snapshots_with_deadline(state, mode, Some(setup_deadline), true),
        )
        .await
}
#[cfg(test)]
pub(crate) async fn await_summary_projection_with_test_setup_deadline(
    build: impl std::future::Future<Output = Result<SummaryProjection>>,
    setup_deadline: Duration,
    interleave: SummaryProjectionTestInterleave,
    stage_reached: std::sync::Arc<tokio::sync::Notify>,
) -> Result<SummaryProjection> {
    let mut build = Box::pin(build);
    let stage = interleave.wait_for_writer();
    let setup_timeout = tokio::time::sleep(setup_deadline);
    tokio::pin!(stage);
    tokio::pin!(setup_timeout);
    tokio::select! {
        result = &mut build => {
            stage_reached.notify_one();
            result
        }
        _ = &mut stage => {
            stage_reached.notify_one();
            tokio::time::timeout(SUMMARY_PROJECTION_BUILD_DEADLINE, &mut build)
                .await
                .map_err(|_| {
                    anyhow!(
                        "summary projection build exceeded {SUMMARY_PROJECTION_BUILD_DEADLINE:?}"
                    )
                })
                .and_then(|result| result)
        }
        _ = &mut setup_timeout => {
            stage_reached.notify_one();
            Err(anyhow!("summary projection setup exceeded {setup_deadline:?}"))
        }
    }
}
