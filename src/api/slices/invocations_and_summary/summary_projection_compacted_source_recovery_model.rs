use super::*;
use serde_json::from_str;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct CompactedSourceRecoveryWindow {
    pub(super) name: String,
    pub(super) start: String,
    pub(super) end: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct CompactedSourceRecoveryCheckpoint {
    pub(super) scope: String,
    pub(super) format_version: i64,
    pub(super) time_zone: String,
    pub(super) range_start: String,
    pub(super) range_end: String,
    pub(super) window_ranges_json: String,
    pub(super) cursor_occurred_at: Option<String>,
    pub(super) cursor_id: i64,
    pub(super) target_live_high_watermark_id: i64,
    pub(super) target_rollup_live_cursor: i64,
    pub(super) target_account_rollup_live_cursor: Option<i64>,
    pub(super) target_manifest_high_watermark_id: Option<i64>,
    pub(super) target_coverage_revision: i64,
    pub(super) target_account_coverage_revision: i64,
    pub(super) target_terminal_sequence: i64,
    pub(super) list_limit_max: i64,
    pub(super) processed_rows: i64,
    pub(super) state: String,
    pub(super) last_error_code: Option<String>,
    pub(super) last_error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CompactedSourceRecoveryIdentity {
    pub(super) scope: String,
    pub(super) format_version: i64,
    pub(super) time_zone: String,
    pub(super) range_start: String,
    pub(super) range_end: String,
    pub(super) window_ranges_json: String,
    pub(super) target_live_high_watermark_id: i64,
    pub(super) target_rollup_live_cursor: i64,
    pub(super) target_account_rollup_live_cursor: Option<i64>,
    pub(super) target_manifest_high_watermark_id: Option<i64>,
    pub(super) target_coverage_revision: i64,
    pub(super) target_account_coverage_revision: i64,
    pub(super) target_terminal_sequence: i64,
    pub(super) list_limit_max: i64,
}

impl From<&CompactedSourceRecoveryCheckpoint> for CompactedSourceRecoveryIdentity {
    fn from(checkpoint: &CompactedSourceRecoveryCheckpoint) -> Self {
        Self {
            scope: checkpoint.scope.clone(),
            format_version: checkpoint.format_version,
            time_zone: checkpoint.time_zone.clone(),
            range_start: checkpoint.range_start.clone(),
            range_end: checkpoint.range_end.clone(),
            window_ranges_json: checkpoint.window_ranges_json.clone(),
            target_live_high_watermark_id: checkpoint.target_live_high_watermark_id,
            target_rollup_live_cursor: checkpoint.target_rollup_live_cursor,
            target_account_rollup_live_cursor: checkpoint.target_account_rollup_live_cursor,
            target_manifest_high_watermark_id: checkpoint.target_manifest_high_watermark_id,
            target_coverage_revision: checkpoint.target_coverage_revision,
            target_account_coverage_revision: checkpoint.target_account_coverage_revision,
            target_terminal_sequence: checkpoint.target_terminal_sequence,
            list_limit_max: checkpoint.list_limit_max,
        }
    }
}

#[derive(Debug)]
pub(super) struct CompactedSourceRecoveryRuntime {
    pub(super) page_size: usize,
    pub(super) identity: Option<CompactedSourceRecoveryIdentity>,
    pub(super) retry_reset: bool,
}

impl Default for CompactedSourceRecoveryRuntime {
    fn default() -> Self {
        Self {
            page_size: SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
            identity: None,
            retry_reset: false,
        }
    }
}

impl CompactedSourceRecoveryRuntime {
    pub(super) fn observe_checkpoint(&mut self, checkpoint: &CompactedSourceRecoveryCheckpoint) {
        let identity = CompactedSourceRecoveryIdentity::from(checkpoint);
        if self.identity.as_ref() != Some(&identity) {
            self.identity = Some(identity);
            self.page_size = SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE;
            self.retry_reset = true;
        }
    }

    pub(super) fn halve_page_size(&mut self) {
        self.page_size = (self.page_size / 2).max(1);
    }

    pub(super) fn mark_progress(&mut self) {
        self.retry_reset = true;
    }

    pub(super) fn take_retry_reset(&mut self) -> bool {
        std::mem::take(&mut self.retry_reset)
    }
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct CompactedSourceRecoverySourceRow {
    pub(super) id: i64,
    pub(super) invoke_id: String,
    pub(super) occurred_at: String,
    pub(super) upstream_account_id: Option<i64>,
    pub(super) model: Option<String>,
    pub(super) response_model: Option<String>,
    pub(super) reasoning_effort: Option<String>,
    pub(super) input_tokens: Option<i64>,
    pub(super) output_tokens: Option<i64>,
    pub(super) cache_input_tokens: Option<i64>,
    pub(super) total_tokens: Option<i64>,
    pub(super) cost: Option<f64>,
    pub(super) cost_input: Option<f64>,
    pub(super) cost_cache_write: Option<f64>,
    pub(super) cost_cache_read: Option<f64>,
    pub(super) cost_output: Option<f64>,
    pub(super) cost_reasoning: Option<f64>,
    pub(super) status: Option<String>,
    pub(super) error_message: Option<String>,
    pub(super) failure_kind: Option<String>,
    pub(super) failure_class: Option<String>,
    pub(super) is_actionable: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct CompactedSourceRecoveryCandidate {
    pub(super) id: i64,
    pub(super) invoke_id: String,
    pub(super) occurred_at: String,
    pub(super) upstream_account_id: Option<i64>,
    pub(super) model: Option<String>,
    pub(super) response_model: Option<String>,
    pub(super) reasoning_effort: Option<String>,
    pub(super) input_tokens: Option<i64>,
    pub(super) output_tokens: Option<i64>,
    pub(super) cache_input_tokens: Option<i64>,
    pub(super) total_tokens: Option<i64>,
    pub(super) cost: Option<f64>,
    pub(super) cost_input: Option<f64>,
    pub(super) cost_cache_write: Option<f64>,
    pub(super) cost_cache_read: Option<f64>,
    pub(super) cost_output: Option<f64>,
    pub(super) cost_reasoning: Option<f64>,
    pub(super) status: Option<String>,
    pub(super) error_message: Option<String>,
    pub(super) failure_kind: Option<String>,
    pub(super) failure_class: Option<String>,
    pub(super) is_actionable: Option<i64>,
    pub(super) estimated_bytes: i64,
    pub(super) error_message_bytes: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct CompactedSourceRecoveryCandidateBudget {
    pub(super) id: i64,
    pub(super) occurred_at: String,
    pub(super) status: Option<String>,
    pub(super) failure_kind: Option<String>,
    pub(super) failure_class: Option<String>,
    pub(super) is_actionable: Option<i64>,
    pub(super) estimated_bytes: i64,
    pub(super) error_message_bytes: i64,
}

impl CompactedSourceRecoveryCandidateBudget {
    pub(super) fn into_selection_candidate(self) -> Option<CompactedSourceRecoveryCandidate> {
        if matches!(
            self.status.as_deref(),
            None | Some("running") | Some("pending")
        ) {
            return None;
        }
        Some(CompactedSourceRecoveryCandidate {
            id: self.id,
            invoke_id: String::new(),
            occurred_at: self.occurred_at,
            upstream_account_id: None,
            model: None,
            response_model: None,
            reasoning_effort: None,
            input_tokens: None,
            output_tokens: None,
            cache_input_tokens: None,
            total_tokens: None,
            cost: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            status: self.status,
            error_message: None,
            failure_kind: self.failure_kind,
            failure_class: self.failure_class,
            is_actionable: self.is_actionable,
            estimated_bytes: self.estimated_bytes,
            error_message_bytes: self.error_message_bytes,
        })
    }
}

impl CompactedSourceRecoveryCandidate {
    pub(super) fn into_source_row(self) -> CompactedSourceRecoverySourceRow {
        CompactedSourceRecoverySourceRow {
            id: self.id,
            invoke_id: self.invoke_id,
            occurred_at: self.occurred_at,
            upstream_account_id: self.upstream_account_id,
            model: self.model,
            response_model: self.response_model,
            reasoning_effort: self.reasoning_effort,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_input_tokens: self.cache_input_tokens,
            total_tokens: self.total_tokens,
            cost: self.cost,
            cost_input: self.cost_input,
            cost_cache_write: self.cost_cache_write,
            cost_cache_read: self.cost_cache_read,
            cost_output: self.cost_output,
            cost_reasoning: self.cost_reasoning,
            status: self.status,
            error_message: self.error_message,
            failure_kind: self.failure_kind,
            failure_class: self.failure_class,
            is_actionable: self.is_actionable,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct CompactedSourceRecoveryKey {
    pub(super) id: i64,
    pub(super) occurred_at: String,
}

#[derive(Debug, Default)]
pub(super) struct CompactedSourceRecoveryCandidatePage {
    pub(super) keys: Vec<CompactedSourceRecoveryKey>,
    pub(super) candidates: Vec<CompactedSourceRecoveryCandidate>,
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct CompactedSourceRecoveryAggregateDelta {
    pub(super) total_count: i64,
    pub(super) success_count: i64,
    pub(super) failure_count: i64,
    pub(super) total_tokens: i64,
    pub(super) total_cost: f64,
    pub(super) non_success_cost: f64,
    pub(super) non_success_tokens: i64,
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct CompactedSourceRecoveryUsageDelta {
    pub(super) cache_write_tokens: i64,
    pub(super) cache_read_tokens: i64,
    pub(super) output_tokens: i64,
    pub(super) cost_input: f64,
    pub(super) cost_cache_write: f64,
    pub(super) cost_cache_read: f64,
    pub(super) cost_output: f64,
    pub(super) cost_reasoning: f64,
    pub(super) cost_unknown: f64,
    pub(super) has_cost: i64,
}

#[derive(Debug, Default)]
pub(super) struct CompactedSourceRecoveryContributions {
    pub(super) aggregates: HashMap<(String, i64), CompactedSourceRecoveryAggregateDelta>,
    pub(super) usage: HashMap<(String, i64, String, String), CompactedSourceRecoveryUsageDelta>,
    pub(super) records: Vec<(String, i64, i64, String, String)>,
    pub(super) record_scopes: BTreeSet<(String, i64)>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum CompactedSourceRecoveryPageOutcome {
    Progress { completed: bool, deferred: bool },
    Reset,
    Stale,
}

pub(super) const COMPACTED_SOURCE_RECOVERY_CHECKPOINT_SELECT: &str = "SELECT scope, format_version, time_zone, range_start, range_end, window_ranges_json, \
            cursor_occurred_at, cursor_id, target_live_high_watermark_id, \
            target_rollup_live_cursor, target_account_rollup_live_cursor, \
            target_manifest_high_watermark_id, target_coverage_revision, \
            target_account_coverage_revision, target_terminal_sequence, list_limit_max, \
            processed_rows, state, last_error_code, last_error_message \
       FROM summary_compacted_source_recovery_checkpoint \
      WHERE scope = ?1";

pub(super) async fn load_compacted_source_recovery_checkpoint(
    pool: &sqlx::SqlitePool,
) -> Result<Option<CompactedSourceRecoveryCheckpoint>> {
    sqlx::query_as::<_, CompactedSourceRecoveryCheckpoint>(
        COMPACTED_SOURCE_RECOVERY_CHECKPOINT_SELECT,
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_optional(pool)
    .await
    .context("load compacted source recovery checkpoint")
}

pub(super) async fn load_compacted_source_recovery_checkpoint_tx(
    transaction: &mut sqlx::SqliteConnection,
) -> Result<Option<CompactedSourceRecoveryCheckpoint>> {
    sqlx::query_as::<_, CompactedSourceRecoveryCheckpoint>(
        COMPACTED_SOURCE_RECOVERY_CHECKPOINT_SELECT,
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_optional(transaction)
    .await
    .context("load compacted source recovery checkpoint in transaction")
}

pub(super) fn compacted_source_recovery_checkpoint_progress_matches(
    expected: &CompactedSourceRecoveryCheckpoint,
    current: &CompactedSourceRecoveryCheckpoint,
) -> bool {
    expected.scope == current.scope
        && expected.format_version == current.format_version
        && expected.time_zone == current.time_zone
        && expected.range_start == current.range_start
        && expected.range_end == current.range_end
        && expected.window_ranges_json == current.window_ranges_json
        && expected.cursor_occurred_at == current.cursor_occurred_at
        && expected.cursor_id == current.cursor_id
        && expected.target_live_high_watermark_id == current.target_live_high_watermark_id
        && expected.target_rollup_live_cursor == current.target_rollup_live_cursor
        && expected.target_account_rollup_live_cursor == current.target_account_rollup_live_cursor
        && expected.target_manifest_high_watermark_id == current.target_manifest_high_watermark_id
        && expected.target_coverage_revision == current.target_coverage_revision
        && expected.target_account_coverage_revision == current.target_account_coverage_revision
        && expected.target_terminal_sequence == current.target_terminal_sequence
        && expected.list_limit_max == current.list_limit_max
        && expected.processed_rows == current.processed_rows
}

pub(super) fn compacted_source_recovery_fence(
    checkpoint: &CompactedSourceRecoveryCheckpoint,
) -> SummaryProjectionGenerationFence {
    SummaryProjectionGenerationFence {
        live_high_watermark_id: checkpoint.target_live_high_watermark_id,
        rollup_live_cursor: checkpoint.target_rollup_live_cursor,
        account_rollup_live_cursor: checkpoint.target_account_rollup_live_cursor,
        completed_manifest_high_watermark_id: checkpoint.target_manifest_high_watermark_id,
        coverage_revision: checkpoint.target_coverage_revision,
        account_coverage_revision: checkpoint.target_account_coverage_revision,
        durable_terminal_sequence_watermark: u64::try_from(
            checkpoint.target_terminal_sequence.max(0),
        )
        .unwrap_or_default(),
    }
}

pub(super) fn compacted_source_recovery_windows_at(
    now: DateTime<Utc>,
) -> Result<Vec<CompactedSourceRecoveryWindow>> {
    let reporting_tz = SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE
        .parse::<Tz>()
        .context("parse compacted source recovery timezone")?;
    let today = crate::stats::resolve_range_window_at("today", reporting_tz, now)?;
    let yesterday = crate::stats::resolve_range_window_at("yesterday", reporting_tz, now)?;
    let (previous7d_start, previous7d_end) =
        crate::stats::previous_full_days_range_bounds(7, now, reporting_tz)
            .context("resolve previous7d compacted source recovery range")?;
    Ok(vec![
        CompactedSourceRecoveryWindow {
            name: "today".to_string(),
            start: today.start.to_rfc3339(),
            end: today.end.to_rfc3339(),
        },
        CompactedSourceRecoveryWindow {
            name: "yesterday".to_string(),
            start: yesterday.start.to_rfc3339(),
            end: yesterday.end.to_rfc3339(),
        },
        CompactedSourceRecoveryWindow {
            name: "previous7d".to_string(),
            start: previous7d_start.to_rfc3339(),
            end: previous7d_end.to_rfc3339(),
        },
    ])
}

pub(super) fn parse_compacted_source_recovery_windows(
    json: &str,
) -> Result<Vec<(String, DateTime<Utc>, DateTime<Utc>)>> {
    from_str::<Vec<CompactedSourceRecoveryWindow>>(json)
        .context("parse compacted source recovery window boundaries")?
        .into_iter()
        .map(|window| {
            let start = DateTime::parse_from_rfc3339(&window.start)
                .with_context(|| format!("parse {} recovery start", window.name))?
                .with_timezone(&Utc);
            let end = DateTime::parse_from_rfc3339(&window.end)
                .with_context(|| format!("parse {} recovery end", window.name))?
                .with_timezone(&Utc);
            if start >= end {
                return Err(anyhow!(
                    "invalid {} compacted source recovery range",
                    window.name
                ));
            }
            Ok((window.name, start, end))
        })
        .collect()
}

pub(super) fn compacted_source_recovery_windows_match(
    current: &[CompactedSourceRecoveryWindow],
    stored: &[CompactedSourceRecoveryWindow],
) -> bool {
    current.len() == stored.len()
        && current.iter().zip(stored).all(|(current, stored)| {
            current.name == stored.name
                && current.start == stored.start
                && (current.name == "today" || current.end == stored.end)
        })
}

pub(crate) fn compacted_source_recovery_window_ranges_match_at(
    stored_json: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let stored = from_str::<Vec<CompactedSourceRecoveryWindow>>(stored_json)
        .context("parse stored compacted source recovery window boundaries")?;
    Ok(compacted_source_recovery_windows_match(
        &compacted_source_recovery_windows_at(now)?,
        &stored,
    ))
}

pub(super) fn compacted_source_recovery_checkpoint_matches(
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    target_fence: SummaryProjectionGenerationFence,
    current_windows: &[CompactedSourceRecoveryWindow],
    list_limit_max: usize,
) -> Result<bool> {
    let stored_windows = parse_compacted_source_recovery_windows(&checkpoint.window_ranges_json)?
        .into_iter()
        .map(|(name, start, end)| CompactedSourceRecoveryWindow {
            name,
            start: start.to_rfc3339(),
            end: end.to_rfc3339(),
        })
        .collect::<Vec<_>>();
    Ok(checkpoint.format_version == 1
        && checkpoint.scope == SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE
        && checkpoint.time_zone == SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE
        && checkpoint.range_start
            == current_windows
                .iter()
                .filter_map(|window| {
                    DateTime::parse_from_rfc3339(&window.start)
                        .ok()
                        .map(|value| {
                            crate::stats::db_occurred_at_lower_bound(value.with_timezone(&Utc))
                        })
                })
                .min()
                .unwrap_or_default()
        && checkpoint.list_limit_max == i64::try_from(list_limit_max).unwrap_or(i64::MAX)
        && compacted_source_recovery_windows_match(current_windows, &stored_windows)
        && compacted_source_recovery_fence(checkpoint).coverage_sources_match(target_fence))
}
