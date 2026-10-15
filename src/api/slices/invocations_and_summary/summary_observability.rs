use super::*;
use crate::observability::contention::{
    PendingKind, PerformanceCounter, PerformanceGauge, PerformanceHistogram, StatsProjection,
    SummaryReason, SummaryResult, current_performance,
};
use axum::Json;
use std::sync::Arc;
use std::time::Instant;

pub(super) fn summary_projection_pending_source_gaps(
    projection: &SummaryProjection,
    kind: StatsProjection,
) -> usize {
    let archive_gaps = projection
        .unavailable_unmaterialized_archive_ranges
        .len()
        .saturating_add(projection.unavailable_boundary_archive_ranges.len())
        .saturating_add(
            projection
                .unavailable_unmaterialized_archive_account_ranges
                .len(),
        )
        .saturating_add(projection.unavailable_boundary_archive_account_ranges.len());
    let live_gaps = projection
        .unavailable_exact_live_ranges
        .len()
        .saturating_add(
            projection
                .unavailable_exact_live_account_ranges
                .values()
                .map(Vec::len)
                .sum::<usize>(),
        );
    match kind {
        StatsProjection::SummaryRecent => live_gaps
            .saturating_add(
                projection
                    .unavailable_unmaterialized_archive_current_ranges
                    .len(),
            )
            .saturating_add(
                projection
                    .unavailable_unmaterialized_archive_account_current_ranges
                    .len(),
            )
            .saturating_add(usize::from(projection.current_source_unavailable))
            .saturating_add(usize::from(projection.current_account_source_unavailable)),
        StatsProjection::SummaryHistorical => archive_gaps.saturating_add(live_gaps),
        _ => 0,
    }
}

pub(super) fn record_summary_projection_publication_metrics(
    projection_kind: StatsProjection,
    pending_source_gaps: usize,
) {
    let Some(metrics) = current_performance().filter(|metrics| metrics.enabled) else {
        return;
    };
    metrics.set_performance(
        PerformanceGauge::ProjectionLastGood(projection_kind),
        Utc::now().timestamp() as f64,
    );
    metrics.set_performance(
        PerformanceGauge::ProjectionPending(projection_kind, PendingKind::SourceGaps),
        pending_source_gaps as f64,
    );
}

pub(super) struct SummaryResponseObservation {
    metrics: Option<Arc<crate::observability::ObservabilityRuntime>>,
    started: Instant,
    result: Option<(SummaryResult, SummaryReason)>,
}

impl SummaryResponseObservation {
    pub(super) fn new() -> Self {
        Self {
            metrics: current_performance().filter(|metrics| metrics.enabled),
            started: Instant::now(),
            result: None,
        }
    }

    pub(super) fn finish(&mut self, response: &Result<Json<StatsResponse>, ApiError>) {
        self.result = match response {
            Ok(Json(response)) => Some((
                if response.data_quality.is_some() {
                    SummaryResult::Degraded
                } else {
                    SummaryResult::Success
                },
                if response.data_quality.is_some() {
                    SummaryReason::SourceGap
                } else {
                    SummaryReason::None
                },
            )),
            Err(ApiError::BadRequest(_)) => None,
            Err(ApiError::Unavailable(error)) => {
                Some((SummaryResult::Unavailable, summary_error_reason(error)))
            }
            Err(ApiError::Conflict(_) | ApiError::Internal(_)) => {
                Some((SummaryResult::Unavailable, SummaryReason::Other))
            }
        };
    }
}

impl Drop for SummaryResponseObservation {
    fn drop(&mut self) {
        let (Some(metrics), Some((result, reason))) = (self.metrics.take(), self.result) else {
            return;
        };
        metrics.count_performance(PerformanceCounter::SummaryResponse(result, reason), 1);
        metrics.observe_performance(
            PerformanceHistogram::SummaryResponse(result),
            self.started.elapsed(),
        );
    }
}

fn summary_error_reason(error: &anyhow::Error) -> SummaryReason {
    let has_message = |needle: &str| {
        error
            .chain()
            .any(|cause| cause.to_string().contains(needle))
    };
    if has_message("has not completed hydration") || has_message("snapshot is not hydrated") {
        SummaryReason::Hydration
    } else if has_message("current rank unproven")
        || has_message("current source is unavailable")
        || has_message("outside the bounded recent index")
    {
        SummaryReason::CurrentRankUnproven
    } else if has_message("source journal") || has_message("persisted-live source") {
        SummaryReason::SourceGap
    } else {
        SummaryReason::Other
    }
}
