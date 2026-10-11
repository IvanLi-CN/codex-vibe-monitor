use super::*;

#[derive(Clone, Copy)]
pub(crate) enum SummaryDeltaGapKind {
    Any,
    Source,
    Terminal,
}

pub(crate) fn summary_source_gap_status(
    projection: &SummaryProjection,
    gaps: &[DeltaGapProof],
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
) -> (bool, bool) {
    let source_gap_matches_scope = |gap: &DeltaGapProof| {
        gap.source_gap
            && (upstream_account_id.is_none()
                || gap.upstream_account_id.is_none()
                || gap.upstream_account_id == upstream_account_id)
    };
    let (mut compaction, mut scoped_strict, mut scoped_unreconciled) = (false, false, false);
    for gap in gaps {
        if !source_gap_matches_scope(gap)
            || !summary_delta_gap_proof_affects_selection(
                projection,
                gap,
                deltas,
                window,
                reporting_tz,
                upstream_account_id,
                false,
            )
        {
            continue;
        }
        if gap.compaction_gap {
            compaction = true;
        } else {
            scoped_strict = true;
            if !matches!(window, SummaryWindow::Current(_))
                || !summary_delta_gap_has_valid_overlay_identity(gap, deltas, upstream_account_id)
            {
                scoped_unreconciled = true;
            }
        }
    }
    let scoped = if compaction {
        scoped_strict
    } else {
        scoped_unreconciled
    };
    let pending = compaction || scoped;
    let (valid_overlay, invalid_overlay) = summary_overlay_selection_status(
        projection,
        deltas,
        window,
        reporting_tz,
        upstream_account_id,
    );
    let allows_degraded = compaction && !scoped && valid_overlay && !invalid_overlay;
    (pending, pending && !allows_degraded)
}

pub(crate) fn summary_gap_affects_selection(
    projection: &SummaryProjection,
    gaps: &[DeltaGapProof],
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
) -> bool {
    summary_source_gap_status(
        projection,
        gaps,
        deltas,
        window,
        reporting_tz,
        upstream_account_id,
    )
    .0 || summary_delta_gap_affects_selection(
        projection,
        gaps,
        deltas,
        window,
        reporting_tz,
        upstream_account_id,
        SummaryDeltaGapKind::Terminal,
    )
}

fn summary_delta_gap_proof_affects_selection(
    projection: &SummaryProjection,
    gap: &DeltaGapProof,
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
    allow_overlay_identity: bool,
) -> bool {
    if let SummaryWindow::Current(limit) = window {
        let selection_deltas: &[DashboardActivityTerminalDelta] =
            if allow_overlay_identity { deltas } else { &[] };
        return projection.delta_gap_affects_current_selection(
            gap,
            (*limit).max(0) as usize,
            upstream_account_id,
            selection_deltas,
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

pub(crate) fn summary_delta_overlay_is_valid(delta: &DashboardActivityTerminalDelta) -> bool {
    delta.persisted_row_id.is_some()
        && !delta.invoke_id.trim().is_empty()
        && parse_to_utc_datetime(&delta.occurred_at).is_some()
}

pub(crate) fn summary_delta_gap_has_valid_overlay_identity(
    gap: &DeltaGapProof,
    deltas: &[DashboardActivityTerminalDelta],
    upstream_account_id: Option<i64>,
) -> bool {
    gap.row_id.is_some_and(|row_id| {
        deltas.iter().any(|delta| {
            summary_delta_overlay_is_valid(delta)
                && delta.persisted_row_id == Some(row_id)
                && gap.invoke_id.as_deref() == Some(delta.invoke_id.as_str())
                && gap.occurred_at == delta.occurred_at
                && upstream_account_id
                    .is_none_or(|account_id| delta.upstream_account_id == Some(account_id))
        })
    })
}

fn summary_overlay_selection_status(
    projection: &SummaryProjection,
    deltas: &[DashboardActivityTerminalDelta],
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
) -> (bool, bool) {
    let mut valid = false;
    let mut invalid = false;
    for delta in deltas.iter().filter(|delta| {
        upstream_account_id.is_none_or(|account_id| delta.upstream_account_id == Some(account_id))
    }) {
        if !summary_delta_affects_selection(
            projection,
            std::slice::from_ref(delta),
            window,
            reporting_tz,
            upstream_account_id,
        ) {
            continue;
        }
        if summary_delta_overlay_is_valid(delta) {
            valid = true;
        } else {
            invalid = true;
        }
    }
    (valid, invalid)
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
                    true,
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
            true,
        )
    })
}
