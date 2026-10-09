use super::*;

#[derive(Clone, Copy)]
pub(crate) enum SummaryDeltaGapKind {
    Any,
    Source,
    Terminal,
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
    if let SummaryWindow::Current(limit) = window {
        return gaps.iter().any(|gap| {
            gap_matches(gap)
                && (upstream_account_id.is_none()
                    || gap.upstream_account_id.is_none()
                    || gap.upstream_account_id == upstream_account_id)
                && projection.delta_gap_affects_current_selection(
                    gap,
                    (*limit).max(0) as usize,
                    upstream_account_id,
                    deltas,
                )
        });
    }
    let range = summary_window_range(window, reporting_tz, Utc::now())
        .ok()
        .flatten();
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
        let Some((start, end)) = range else {
            return true;
        };
        parse_to_utc_datetime(&gap.occurred_at)
            .is_none_or(|occurred_at| occurred_at >= start && occurred_at < end)
    })
}
