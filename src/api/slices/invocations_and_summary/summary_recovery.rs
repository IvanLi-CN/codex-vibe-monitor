use super::*;

pub(crate) const SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE: &str = "dashboard";
pub(crate) const SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE: &str = "Asia/Shanghai";
pub(crate) const SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE: usize = 512;
pub(crate) const SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const SUMMARY_COMPACTED_SOURCE_RECOVERY_ERROR_MESSAGE_BYTES: usize = 64 * 1024;
pub(crate) const SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET: Duration =
    Duration::from_millis(250);
pub(crate) const SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET: Duration = Duration::from_secs(2);
pub(crate) const SUMMARY_RECOVERY_PENDING_CODE: &str = "summary_recovery_pending";
pub(crate) const SUMMARY_RECOVERY_FAILED_CODE: &str = "summary_recovery_failed";

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(crate) enum SummaryCompactedSourceRecoveryState {
    #[default]
    Pending,
    Running,
    Deferred,
    Failed,
    Completed,
}

impl SummaryCompactedSourceRecoveryState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Deferred => "deferred",
            Self::Failed => "failed",
            Self::Completed => "completed",
        }
    }

    pub(crate) const fn unavailable_code(self) -> &'static str {
        match self {
            Self::Failed => SUMMARY_RECOVERY_FAILED_CODE,
            Self::Pending | Self::Running | Self::Deferred | Self::Completed => {
                SUMMARY_RECOVERY_PENDING_CODE
            }
        }
    }

    pub(crate) fn from_storage(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "deferred" => Self::Deferred,
            "failed" => Self::Failed,
            "completed" => Self::Completed,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SummaryDashboardRecoveryEntry {
    pub(crate) totals: StatsTotals,
    pub(crate) non_success_tokens: i64,
    pub(crate) usage_breakdown: UsageBreakdownResponse,
}

#[derive(Debug, Clone)]
pub(crate) struct SummaryDashboardRecoverySnapshot {
    pub(crate) time_zone: String,
    pub(crate) window_ranges_json: String,
    pub(crate) generation_fence: SummaryProjectionGenerationFence,
    pub(crate) entries: HashMap<(String, Option<i64>), SummaryDashboardRecoveryEntry>,
    pub(crate) bounded_record_count: usize,
}

impl UsageBreakdownAccumulator {
    pub(crate) fn add_compacted_values(
        &mut self,
        model: String,
        reasoning_effort: Option<String>,
        cache_write_tokens: i64,
        cache_read_tokens: i64,
        output_tokens: i64,
        costs: UsageCostBreakdownResponse,
        has_cost: bool,
    ) {
        self.cache_write_tokens += cache_write_tokens;
        self.cache_read_tokens += cache_read_tokens;
        self.output_tokens += output_tokens;
        if has_cost {
            self.has_cost = true;
            self.costs.input += costs.input;
            self.costs.cache_write += costs.cache_write;
            self.costs.cache_read += costs.cache_read;
            self.costs.output += costs.output;
            self.costs.reasoning += costs.reasoning;
            self.costs.unknown += costs.unknown;
        }

        let model_entry = self
            .models
            .entry(UsageBreakdownGroupKey {
                model,
                reasoning_effort,
            })
            .or_default();
        model_entry.cache_write_tokens += cache_write_tokens;
        model_entry.cache_read_tokens += cache_read_tokens;
        model_entry.output_tokens += output_tokens;
        if has_cost {
            model_entry.has_cost = true;
            model_entry.costs.input += costs.input;
            model_entry.costs.cache_write += costs.cache_write;
            model_entry.costs.cache_read += costs.cache_read;
            model_entry.costs.output += costs.output;
            model_entry.costs.reasoning += costs.reasoning;
            model_entry.costs.unknown += costs.unknown;
        }
    }
}

impl SummaryProjection {
    pub(crate) fn response_for_dashboard_recovery(
        &self,
        window: &SummaryWindow,
        range: Option<(DateTime<Utc>, DateTime<Utc>)>,
        now: DateTime<Utc>,
        entry: &SummaryDashboardRecoveryEntry,
        upstream_account_id: Option<i64>,
    ) -> StatsResponse {
        let mut response = entry.totals.into_response();
        response.non_success_cost = Some(entry.totals.non_success_cost);
        if range.is_some() {
            response.usage_breakdown = Some(entry.usage_breakdown.clone());
        }
        let policy = summary_live_augmentation_policy(window, range, now);
        if policy.include_in_progress {
            let in_progress = self
                .in_progress_by_account
                .get(&upstream_account_id)
                .copied()
                .unwrap_or_default();
            response.in_progress_conversation_count = Some(in_progress.in_progress_count);
            response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
            response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
            response.in_progress_phase_counts = Some(in_progress.phase_counts);
        }
        if policy.include_non_success_tokens {
            response.non_success_tokens = Some(entry.non_success_tokens);
        }
        response.maintenance = self.maintenance.clone();
        response
    }
}

pub(crate) fn summary_dashboard_recovery_window_name(
    window: &SummaryWindow,
) -> Option<&'static str> {
    match window {
        SummaryWindow::Calendar(name) if name == "today" => Some("today"),
        SummaryWindow::Calendar(name) if name == "yesterday" => Some("yesterday"),
        SummaryWindow::PreviousFullDays(7) => Some("previous7d"),
        _ => None,
    }
}

pub(crate) async fn dashboard_recovery_response_if_ready(
    state: &AppState,
    projection: &SummaryProjection,
    window: &SummaryWindow,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
    deltas: &[DashboardActivityTerminalDelta],
) -> Result<Option<StatsResponse>, ApiError> {
    let Some(window_name) = summary_dashboard_recovery_window_name(window) else {
        return Ok(None);
    };
    if reporting_tz.to_string() != SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE {
        return Ok(None);
    }
    let Some(snapshot) = state.subscription_hub.summary_dashboard_recovery().await else {
        return Ok(None);
    };
    let now = Utc::now();
    if snapshot.time_zone != SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE
        || !super::summary_projection_lifecycle::compacted_source_recovery_window_ranges_match_at(
            &snapshot.window_ranges_json,
            now,
        )?
        || !snapshot
            .generation_fence
            .coverage_sources_match(projection.generation_fence())
    {
        return Ok(None);
    }
    let entry = snapshot
        .entries
        .get(&(window_name.to_string(), upstream_account_id))
        .cloned()
        .unwrap_or_else(|| SummaryDashboardRecoveryEntry {
            totals: StatsTotals::default(),
            non_success_tokens: 0,
            usage_breakdown: UsageBreakdownResponse {
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                output_tokens: 0,
                costs: None,
                models: Vec::new(),
            },
        });
    let range = summary_window_range(window, reporting_tz, now)?;
    let mut response =
        projection.response_for_dashboard_recovery(window, range, now, &entry, upstream_account_id);
    let target_live_tail = snapshot.generation_fence.live_tail_cursor();
    let has_unfenced_replayed_delta = deltas.iter().any(|delta| {
        delta
            .persisted_row_id
            .is_some_and(|row_id| row_id <= target_live_tail.live_high_watermark_id)
            && delta.terminal_sequence <= target_live_tail.durable_terminal_sequence_watermark
            && upstream_account_id
                .is_none_or(|account_id| delta.upstream_account_id == Some(account_id))
            && range.is_some_and(|(start, end)| {
                parse_to_utc_datetime(&delta.occurred_at)
                    .is_none_or(|occurred_at| occurred_at >= start && occurred_at < end)
            })
    });
    if has_unfenced_replayed_delta {
        // A restarted durable-tail replay may have a process-local sequence that is lower than
        // the snapshot target. If its row id is inside the target, applying it could double count
        // a row already reduced into the snapshot, while ignoring it could miss a late terminal
        // transition; withholding the snapshot preserves exact/unavailable.
        return Ok(None);
    }
    let live_deltas = deltas
        .iter()
        .filter(|delta| {
            delta.terminal_sequence > target_live_tail.durable_terminal_sequence_watermark
                || delta
                    .persisted_row_id
                    .is_some_and(|row_id| row_id > target_live_tail.live_high_watermark_id)
                || (delta.terminal_sequence == 0 && delta.persisted_row_id.is_none())
        })
        .cloned()
        .collect();
    let mut terminal_sequence = target_live_tail.durable_terminal_sequence_watermark;
    apply_dashboard_terminal_slice_to_summary_response(
        &mut response,
        &mut terminal_sequence,
        window,
        reporting_tz,
        InvocationSourceScope::All,
        upstream_account_id,
        &DashboardTerminalProjectionSlice {
            revision: 0,
            deltas: live_deltas,
        },
    );
    Ok(Some(response))
}

pub(crate) async fn summary_source_gap_unavailable_error(
    state: &AppState,
    window: &SummaryWindow,
    reporting_tz: Tz,
    recovery_applicable: bool,
    message: &'static str,
) -> ApiError {
    // The recovery checkpoint is intentionally fixed to Asia/Shanghai by the Spec. Other
    // reporting zones keep the generic unavailable contract until a matching staged range exists.
    if recovery_applicable
        && reporting_tz.to_string() == SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE
        && summary_dashboard_recovery_window_name(window).is_some()
    {
        let code = state
            .subscription_hub
            .summary_compacted_source_recovery_state()
            .await
            .unavailable_code();
        ApiError::unavailable_with_code(code, anyhow!(message))
    } else {
        ApiError::unavailable(anyhow!(message))
    }
}
