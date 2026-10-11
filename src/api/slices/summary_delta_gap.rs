use super::*;

#[derive(Clone, Copy)]
pub(crate) enum SummaryDeltaGapKind {
    Any,
    Source,
    Terminal,
}

pub(crate) fn summary_source_gap_allows_degraded_read(
    projection: &SummaryProjection,
    gaps: &[DeltaGapProof],
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
) -> bool {
    if deltas.is_empty() {
        return false;
    }
    // Scoped source proofs identify a missing or mismatched row; a replayed delta alone cannot
    // prove its historical coverage. Only an intersecting broad compaction proof has the
    // degraded overlay contract, and an intersecting scoped proof keeps the read fail-closed.
    let source_gap_affects_selection = |gap: &DeltaGapProof| {
        gap.source_gap
            && (upstream_account_id.is_none()
                || gap.upstream_account_id.is_none()
                || gap.upstream_account_id == upstream_account_id)
            && summary_delta_gap_proof_affects_selection(
                projection,
                gap,
                deltas,
                window,
                reporting_tz,
                upstream_account_id,
            )
    };
    let has_intersecting_compaction_gap = gaps
        .iter()
        .any(|gap| gap.compaction_gap && source_gap_affects_selection(gap));
    let has_intersecting_scoped_gap = gaps
        .iter()
        .any(|gap| !gap.compaction_gap && source_gap_affects_selection(gap));
    has_intersecting_compaction_gap
        && !has_intersecting_scoped_gap
        && summary_delta_affects_selection(
            projection,
            deltas,
            window,
            reporting_tz,
            upstream_account_id,
        )
}

pub(crate) fn summary_source_gap_status(
    projection: &SummaryProjection,
    gaps: &[DeltaGapProof],
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
) -> (bool, bool) {
    let pending = summary_delta_gap_affects_selection(
        projection,
        gaps,
        deltas,
        window,
        reporting_tz,
        upstream_account_id,
        SummaryDeltaGapKind::Source,
    );
    (
        pending,
        pending
            && !summary_source_gap_allows_degraded_read(
                projection,
                gaps,
                deltas,
                window,
                reporting_tz,
                upstream_account_id,
            ),
    )
}

fn summary_delta_gap_proof_affects_selection(
    projection: &SummaryProjection,
    gap: &DeltaGapProof,
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
) -> bool {
    if let SummaryWindow::Current(limit) = window {
        return projection.delta_gap_affects_current_selection(
            gap,
            (*limit).max(0) as usize,
            upstream_account_id,
            deltas,
        );
    }
    let range = summary_window_range(window, reporting_tz, Utc::now())
        .ok()
        .flatten();
    let Some((start, end)) = range else {
        return true;
    };
    parse_to_utc_datetime(&gap.occurred_at)
        .is_none_or(|occurred_at| occurred_at >= start && occurred_at < end)
}

pub(crate) fn summary_delta_gap_affects_selection(
    projection: &SummaryProjection,
    gaps: &[DeltaGapProof],
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
    gap_kind: SummaryDeltaGapKind,
) -> bool {
    let gap_matches = |gap: &DeltaGapProof| match gap_kind {
        SummaryDeltaGapKind::Any => true,
        SummaryDeltaGapKind::Source => gap.source_gap,
        SummaryDeltaGapKind::Terminal => !gap.source_gap,
    };
    if let SummaryWindow::Current(_) = window {
        return gaps.iter().any(|gap| {
            gap_matches(gap)
                && (upstream_account_id.is_none()
                    || gap.upstream_account_id.is_none()
                    || gap.upstream_account_id == upstream_account_id)
                && summary_delta_gap_proof_affects_selection(
                    projection,
                    gap,
                    deltas,
                    window,
                    reporting_tz,
                    upstream_account_id,
                )
        });
    }
    gaps.iter().any(|gap| {
        if !gap_matches(gap) {
            return false;
        }
        if upstream_account_id.is_some()
            && gap.upstream_account_id.is_some()
            && gap.upstream_account_id != upstream_account_id
        {
            return false;
        }
        summary_delta_gap_proof_affects_selection(
            projection,
            gap,
            deltas,
            window,
            reporting_tz,
            upstream_account_id,
        )
    })
}
