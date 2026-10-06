use crate::{AppConfig, OnceCell, Result, Utc, anyhow, format_utc_iso_millis};
use chrono::{Datelike, Duration as ChronoDuration, Timelike};
use serde::Serialize;
use sqlx::{
    FromRow, Pool, Sqlite,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{
    collections::HashSet,
    path::PathBuf,
    str::FromStr,
    sync::atomic::{AtomicI64, Ordering},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;

const MIN_INTERVAL_SECS: i64 = 60;
const MAX_TASK_ERROR_DETAIL_CHARS: usize = 4_000;
const TASK_RUN_RETENTION_DAYS: i64 = 90;
const TASK_ERROR_RETENTION_DAYS: i64 = 30;
const TASK_HISTORY_CLEANUP_INTERVAL_MS: i64 = 5 * 60 * 1_000;
const TASK_PROGRESS_STALE_AFTER_SECS: i64 = 30;
const TASK_TIMELINE_RETENTION_HOURS: i64 = 48;
const INITIAL_TASK_DEFAULTS_MARKER: &str = "managed_task_defaults_v1";
const RETENTION_DEFAULT_SCHEDULE_MARKER: &str = "retention_default_schedule_v1";
const DEFAULT_RETENTION_INTERVAL_SECS: i64 = 3_600;
const LEGACY_BACKFILL_ENABLEMENT_MARKER: &str = "managed_task_legacy_enablement_v1";
const PROMPT_CACHE_CONTROL_ORIGIN_MARKER: &str = "prompt_cache_materialization_control_origin_v1";
const TASK_DISABLED_UNTIL_DAYS: i64 = 3650;
const DEFAULT_ENABLED_TASKS: &[&str] = &[
    "retention_archive",
    "upstream_account_maintenance",
    "forward_proxy_subscription_refresh",
    "pool_orphan_recovery",
    "startup_hourly_rollup_bootstrap",
    "system_status_snapshot",
    "invocation_timeline_snapshot",
    "summary_snapshot",
    "summary_coverage_recovery",
    "dashboard_runtime_projection_reconcile",
    "long_term_projection",
    "timeseries_minute_projection",
    "raw_payload_metrics_inventory",
    "prompt_cache_materialization",
];
static LAST_TASK_HISTORY_CLEANUP_MS: AtomicI64 = AtomicI64::new(0);
static ACTIVE_TASK_EXECUTIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

pub(crate) struct TaskExecutionLease {
    task_key: String,
}

impl Drop for TaskExecutionLease {
    fn drop(&mut self) {
        if let Some(active) = ACTIVE_TASK_EXECUTIONS.get()
            && let Ok(mut active) = active.lock()
        {
            active.remove(&self.task_key);
        }
    }
}

pub(crate) fn try_acquire_task_execution(task_key: &str) -> Option<TaskExecutionLease> {
    let canonical_task_key = task_key
        .strip_prefix("startup_backfill.")
        .map(|_| "startup_backfill")
        .unwrap_or(task_key);
    #[cfg(test)]
    {
        Some(TaskExecutionLease {
            task_key: canonical_task_key.to_string(),
        })
    }
    #[cfg(not(test))]
    {
        let active = ACTIVE_TASK_EXECUTIONS.get_or_init(|| Mutex::new(HashSet::new()));
        let mut active = active.lock().ok()?;
        if !active.insert(canonical_task_key.to_string()) {
            return None;
        }
        Some(TaskExecutionLease {
            task_key: canonical_task_key.to_string(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct MaintenanceStore {
    pub(crate) pool: Pool<Sqlite>,
    pub(crate) prompt_cache_materialization_control: Arc<PromptCacheMaterializationControl>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PromptCacheMaterializationControlSnapshot {
    pub(crate) enabled: bool,
    pub(crate) generation: u64,
}

#[derive(Debug, Default)]
struct PromptCacheMaterializationControlState {
    snapshot: Option<PromptCacheMaterializationControlSnapshot>,
    active_steps: u64,
}

#[derive(Debug, Default)]
pub(crate) struct PromptCacheMaterializationControl {
    state: Mutex<PromptCacheMaterializationControlState>,
    update_lock: AsyncMutex<()>,
}

pub(crate) struct PromptCacheMaterializationStep {
    control: Arc<PromptCacheMaterializationControl>,
    pub(crate) generation: u64,
}

pub(crate) enum PromptCacheMaterializationStepAdmission {
    Started(PromptCacheMaterializationStep),
    Disabled,
    GenerationChanged,
    Unavailable,
}

impl PromptCacheMaterializationControl {
    // Control commits and maintenance checkpoint finalization share this lock.
    // The short state mutex never covers SQL or waits for this async lock.
    pub(crate) async fn lock_current_generation(
        &self,
        expected_generation: u64,
    ) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        let guard = self.update_lock.lock().await;
        self.snapshot()
            .is_some_and(|snapshot| snapshot.enabled && snapshot.generation == expected_generation)
            .then_some(guard)
    }

    pub(crate) fn initialize(&self, enabled: bool) -> PromptCacheMaterializationControlSnapshot {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *state
            .snapshot
            .get_or_insert(PromptCacheMaterializationControlSnapshot {
                enabled,
                generation: 1,
            })
    }

    pub(crate) fn snapshot(&self) -> Option<PromptCacheMaterializationControlSnapshot> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot
    }

    pub(crate) fn with_current_generation<R>(
        &self,
        expected_generation: u64,
        action: impl FnOnce() -> R,
    ) -> Option<R> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .snapshot
            .is_some_and(|snapshot| snapshot.generation == expected_generation)
            .then(action)
    }

    pub(crate) fn publish_committed(
        &self,
        enabled: bool,
    ) -> PromptCacheMaterializationControlSnapshot {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let snapshot = match state.snapshot {
            Some(current) if current.enabled == enabled => current,
            Some(current) => PromptCacheMaterializationControlSnapshot {
                enabled,
                generation: current.generation.saturating_add(1),
            },
            None => PromptCacheMaterializationControlSnapshot {
                enabled,
                generation: 1,
            },
        };
        state.snapshot = Some(snapshot);
        snapshot
    }

    pub(crate) fn begin_step(
        self: &Arc<Self>,
        expected_generation: Option<u64>,
    ) -> PromptCacheMaterializationStepAdmission {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(snapshot) = state.snapshot else {
            return PromptCacheMaterializationStepAdmission::Unavailable;
        };
        if expected_generation.is_some_and(|generation| generation != snapshot.generation) {
            return PromptCacheMaterializationStepAdmission::GenerationChanged;
        }
        if !snapshot.enabled {
            return PromptCacheMaterializationStepAdmission::Disabled;
        }
        state.active_steps = state.active_steps.saturating_add(1);
        PromptCacheMaterializationStepAdmission::Started(PromptCacheMaterializationStep {
            control: Arc::clone(self),
            generation: snapshot.generation,
        })
    }
}

impl Drop for PromptCacheMaterializationStep {
    fn drop(&mut self) {
        let mut state = self
            .control
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active_steps = state.active_steps.saturating_sub(1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PromptCacheMaterializationControlUpdate {
    pub(crate) changed: bool,
    pub(crate) snapshot: PromptCacheMaterializationControlSnapshot,
}

#[derive(Debug, Clone, Default, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTask {
    pub(crate) task_key: String,
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) trigger_mode: String,
    pub(crate) enabled: bool,
    pub(crate) interval_secs: Option<i64>,
    pub(crate) cron_expr: Option<String>,
    pub(crate) next_trigger_at: Option<String>,
    pub(crate) next_catchup_at: Option<String>,
    pub(crate) catchup_reason: Option<String>,
    pub(crate) is_manual: bool,
    pub(crate) display_color_light: Option<String>,
    pub(crate) display_color_dark: Option<String>,
    #[serde(skip)]
    pub(crate) schedule_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[sqlx(skip)]
    pub(crate) effective_schedule: Option<ManagedTaskSchedule>,
    #[sqlx(skip)]
    pub(crate) trigger_kinds: Vec<String>,
    #[sqlx(skip)]
    pub(crate) effective_policy: String,
    #[sqlx(skip)]
    pub(crate) policy_source: String,
    #[sqlx(skip)]
    pub(crate) schedule_editable: bool,
    #[sqlx(skip)]
    pub(crate) schedule_capability_reason: Option<String>,
    #[sqlx(skip)]
    pub(crate) execution_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[sqlx(skip)]
    pub(crate) measurement_capabilities: Option<TaskMeasurementCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[sqlx(skip)]
    pub(crate) last_execution: Option<TaskExecutionSummary>,
    #[sqlx(skip)]
    pub(crate) execution_observation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskExecutionSummary {
    pub(crate) execution_uid: Option<String>,
    pub(crate) run_id: i64,
    pub(crate) trigger_kind: String,
    pub(crate) attempted_at: String,
    pub(crate) actual_started_at: Option<String>,
    pub(crate) finished_at: Option<String>,
    pub(crate) duration_ms: Option<i64>,
    pub(crate) status: String,
    pub(crate) result: String,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskMeasurementCapabilities {
    pub(crate) pending: TaskMetricCapability,
    pub(crate) discovered: TaskMetricCapability,
    pub(crate) processed: TaskMetricCapability,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskMetricCapability {
    pub(crate) supported: bool,
    pub(crate) unit: Option<String>,
    pub(crate) scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskWorkloadMetric {
    pub(crate) value: Option<i64>,
    pub(crate) unit: String,
    pub(crate) scope: String,
    pub(crate) range: String,
    pub(crate) observed_at: Option<String>,
    pub(crate) coverage: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskWorkloadSample {
    pub(crate) sample_id: String,
    pub(crate) execution_uid: String,
    pub(crate) managed_run_id: Option<i64>,
    pub(crate) task_key: String,
    pub(crate) trigger_kind: String,
    pub(crate) attempted_at: String,
    pub(crate) actual_started_at: Option<String>,
    pub(crate) finished_at: Option<String>,
    #[serde(default)]
    pub(crate) duration_ms: Option<i64>,
    pub(crate) status: String,
    pub(crate) reason: Option<String>,
    pub(crate) sequence: u64,
    pub(crate) pending: Option<TaskWorkloadMetric>,
    pub(crate) discovered: Option<TaskWorkloadMetric>,
    pub(crate) processed: Option<TaskWorkloadMetric>,
    pub(crate) subset_relation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskWorkloadCoverageGap {
    pub(crate) id: String,
    pub(crate) started_at: String,
    pub(crate) finished_at: Option<String>,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskWorkloadTrend {
    pub(crate) revision: i64,
    pub(crate) window_start: String,
    pub(crate) window_end: String,
    pub(crate) observed_at: String,
    pub(crate) sample_limit: usize,
    pub(crate) truncated: bool,
    pub(crate) capabilities: TaskMeasurementCapabilities,
    pub(crate) coverage: String,
    pub(crate) samples: Vec<TaskWorkloadSample>,
    pub(crate) coverage_gaps: Vec<TaskWorkloadCoverageGap>,
    pub(crate) latest_pending: Option<TaskWorkloadMetric>,
    pub(crate) latest_processed: Option<TaskWorkloadMetric>,
    pub(crate) latest_observed_at: Option<String>,
    pub(crate) processing_rate_per_second: Option<f64>,
    pub(crate) processing_rate_window: Option<String>,
    pub(crate) clearance_eta: Option<String>,
    pub(crate) clearance_estimate_window: Option<String>,
    pub(crate) clearance_estimate_coverage: Option<String>,
    pub(crate) clearance_estimate_reason: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTaskSchedule {
    pub(crate) source: String,
    pub(crate) interval_secs: Option<i64>,
    pub(crate) cron_expr: Option<String>,
    pub(crate) next_trigger_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskProgress {
    pub(crate) total: Option<i64>,
    pub(crate) completed: Option<i64>,
    pub(crate) phase: Option<String>,
    pub(crate) checkpoint: Option<String>,
    pub(crate) eta_seconds: Option<i64>,
    pub(crate) updated_at: Option<String>,
    pub(crate) freshness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_progress_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) wait_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_retry_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_inspection_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_catchup_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) catchup_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stages: Option<Vec<TaskStage>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskRun {
    pub(crate) id: i64,
    pub(crate) trigger_kind: String,
    pub(crate) started_at: String,
    pub(crate) finished_at: Option<String>,
    pub(crate) duration_ms: Option<i64>,
    pub(crate) status: String,
    pub(crate) summary: Option<String>,
    pub(crate) processed_count: Option<i64>,
    pub(crate) updated_count: Option<i64>,
    pub(crate) error_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) core_completion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskStage {
    pub(crate) name: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) elapsed_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) wait_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) checkpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTaskDetail {
    pub(crate) task: ManagedTask,
    pub(crate) progress: Option<TaskProgress>,
    pub(crate) recent_runs: Vec<TaskRun>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retention_backlog_trend: Option<Vec<RetentionBacklogTrendPoint>>,
    pub(crate) workload_trend: TaskWorkloadTrend,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionBacklogTrendPoint {
    pub(crate) bucket_start: String,
    pub(crate) state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) observed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) invocation_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_overdue_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) retention_days: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cutoff: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_max_invocation_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct RetentionBacklogObservation {
    pub(crate) bucket_start: String,
    pub(crate) observed_at: String,
    pub(crate) invocation_count: i64,
    pub(crate) max_overdue_seconds: Option<i64>,
    pub(crate) retention_days: i64,
    pub(crate) cutoff: String,
    pub(crate) source_max_invocation_id: Option<i64>,
}

#[derive(Debug, FromRow)]
struct TaskProgressRow {
    total: Option<i64>,
    completed: Option<i64>,
    phase: Option<String>,
    checkpoint: Option<String>,
    eta_seconds: Option<i64>,
    updated_at: Option<String>,
    freshness: String,
    unit: Option<String>,
    source_scope: Option<String>,
    last_progress_at: Option<String>,
    wait_reason: Option<String>,
    next_retry_at: Option<String>,
    next_inspection_at: Option<String>,
    next_catchup_at: Option<String>,
    catchup_state: Option<String>,
    stages: Option<String>,
}

#[derive(Debug, FromRow)]
struct TaskRunRow {
    id: i64,
    trigger_kind: String,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: Option<i64>,
    status: String,
    summary: Option<String>,
    processed_count: Option<i64>,
    updated_count: Option<i64>,
    error_detail: Option<String>,
    completion: Option<String>,
    core_completion: Option<String>,
    details: Option<String>,
}

#[derive(Debug, FromRow)]
struct LatestTaskExecutionRow {
    task_key: String,
    id: i64,
    execution_uid: Option<String>,
    trigger_kind: String,
    started_at: String,
    actual_started_at: Option<String>,
    finished_at: Option<String>,
    actual_finished_at: Option<String>,
    duration_ms: Option<i64>,
    actual_duration_ms: Option<i64>,
    status: String,
    error_detail: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct TaskWorkloadLegacyRun {
    id: i64,
    trigger_kind: String,
    started_at: String,
    actual_started_at: Option<String>,
    finished_at: Option<String>,
    actual_finished_at: Option<String>,
    duration_ms: Option<i64>,
    actual_duration_ms: Option<i64>,
    status: String,
    error_detail: Option<String>,
    execution_uid: Option<String>,
}

#[derive(Debug, FromRow)]
struct StoredTaskWorkloadRun {
    execution_uid: String,
    managed_run_id: Option<i64>,
    attempted_at: String,
    sequence: i64,
    status: String,
    sample_json: String,
}

fn restore_stored_task_workload_sample(
    task_key: &str,
    row: StoredTaskWorkloadRun,
) -> TaskWorkloadSample {
    let mut sample = match serde_json::from_str::<TaskWorkloadSample>(&row.sample_json) {
        Ok(sample) => sample,
        Err(_) => TaskWorkloadSample {
            sample_id: format!("{}:{task_key}", row.execution_uid),
            execution_uid: row.execution_uid.clone(),
            managed_run_id: row.managed_run_id,
            task_key: task_key.to_string(),
            trigger_kind: "unknown".to_string(),
            attempted_at: row.attempted_at.clone(),
            actual_started_at: None,
            finished_at: None,
            duration_ms: None,
            status: row.status.clone(),
            reason: Some("工作量记录损坏，计量未知".to_string()),
            sequence: 0,
            pending: None,
            discovered: None,
            processed: None,
            subset_relation: "unknown".to_string(),
        },
    };
    sample.sample_id = format!("{}:{task_key}", row.execution_uid);
    sample.execution_uid = row.execution_uid;
    sample.managed_run_id = row.managed_run_id;
    sample.task_key = task_key.to_string();
    sample.attempted_at = row.attempted_at;
    sample.sequence = sample.sequence.max(row.sequence.max(0) as u64);
    sample.status = row.status;
    sample
}

#[derive(Debug, FromRow)]
struct TaskWorkloadCoverageGapRow {
    segment_id: String,
    started_at: String,
    finished_at: Option<String>,
    reason: Option<String>,
}

struct TaskWorkloadSummary {
    latest_pending: Option<TaskWorkloadMetric>,
    latest_processed: Option<TaskWorkloadMetric>,
    latest_observed_at: Option<String>,
    processing_rate_per_second: Option<f64>,
    processing_rate_window: Option<String>,
    clearance_eta: Option<String>,
    clearance_estimate_window: Option<String>,
    clearance_estimate_coverage: Option<String>,
    clearance_estimate_reason: String,
}

fn calculate_task_workload_summary(
    samples: &[TaskWorkloadSample],
    enabled: bool,
    interval_secs: Option<i64>,
    capture_gap_at: Option<chrono::DateTime<Utc>>,
    now: chrono::DateTime<Utc>,
) -> TaskWorkloadSummary {
    let latest_pending = samples
        .iter()
        .rev()
        .find_map(|sample| sample.pending.as_ref())
        .filter(|metric| {
            metric.value.is_some()
                && metric.coverage == "exact"
                && capture_gap_at.is_none_or(|gap_at| {
                    metric
                        .observed_at
                        .as_deref()
                        .and_then(crate::stats::parse_to_utc_datetime)
                        .is_some_and(|observed_at| observed_at > gap_at)
                })
        })
        .cloned();
    let latest_processed = samples
        .iter()
        .rev()
        .find_map(|sample| sample.processed.as_ref())
        .filter(|metric| {
            metric.value.is_some()
                && metric.coverage == "window"
                && capture_gap_at.is_none_or(|gap_at| {
                    metric
                        .observed_at
                        .as_deref()
                        .and_then(crate::stats::parse_to_utc_datetime)
                        .is_some_and(|observed_at| observed_at > gap_at)
                })
        })
        .cloned();
    let latest_observed_at = latest_pending
        .as_ref()
        .and_then(|metric| metric.observed_at.clone())
        .or_else(|| {
            latest_processed
                .as_ref()
                .and_then(|metric| metric.observed_at.clone())
        });

    let mut rate_samples = Vec::new();
    let mut unbound_skips = Vec::new();
    let mut rate_identity: Option<(String, String)> = None;
    for sample in samples.iter().rev() {
        let attempt_time = sample
            .actual_started_at
            .as_deref()
            .or(Some(sample.attempted_at.as_str()))
            .and_then(crate::stats::parse_to_utc_datetime);
        if capture_gap_at.is_some_and(|gap_at| attempt_time.is_none_or(|time| time <= gap_at)) {
            break;
        }
        if sample.status == "running" {
            continue;
        }
        if sample.status == "skipped" && sample.actual_started_at.is_none() {
            let identity = match sample.processed.as_ref() {
                Some(metric) if metric.value == Some(0) && metric.coverage == "window" => {
                    (metric.unit.clone(), metric.scope.clone())
                }
                Some(_) => break,
                None => {
                    let Some(identity) = rate_identity.clone() else {
                        unbound_skips.push(sample);
                        continue;
                    };
                    identity
                }
            };
            if rate_identity
                .as_ref()
                .is_some_and(|expected| *expected != identity)
            {
                break;
            }
            rate_identity = Some(identity);
            rate_samples.append(&mut unbound_skips);
            rate_samples.push(sample);
            continue;
        }
        let Some(metric) = sample.processed.as_ref().filter(|metric| {
            metric.value.is_some()
                && metric.coverage == "window"
                && sample.actual_started_at.is_some()
                && sample.finished_at.is_some()
                && sample.status != "unknown"
        }) else {
            break;
        };
        let identity = (metric.unit.clone(), metric.scope.clone());
        if rate_identity
            .as_ref()
            .is_some_and(|expected| *expected != identity)
        {
            break;
        }
        rate_identity = Some(identity);
        rate_samples.append(&mut unbound_skips);
        rate_samples.push(sample);
        if rate_samples.len() == 20 {
            break;
        }
    }
    let rate_samples = rate_samples.into_iter().rev().collect::<Vec<_>>();
    let rate_count = rate_samples
        .iter()
        .filter_map(|sample| sample.processed.as_ref()?.value)
        .fold(0_i64, i64::saturating_add);
    let rate_start = rate_samples.iter().find_map(|sample| {
        sample
            .actual_started_at
            .as_deref()
            .or_else(|| (sample.status == "skipped").then_some(sample.attempted_at.as_str()))
            .and_then(crate::stats::parse_to_utc_datetime)
    });
    let rate_end = rate_samples
        .iter()
        .rev()
        .find_map(|sample| sample.finished_at.as_deref())
        .and_then(crate::stats::parse_to_utc_datetime);
    let (processing_rate_per_second, processing_rate_window) = match (rate_start, rate_end) {
        (Some(start), Some(end)) if end > start => {
            let elapsed = (end - start).num_milliseconds() as f64 / 1_000.0;
            if elapsed > 0.0 {
                (
                    Some(rate_count.max(0) as f64 / elapsed),
                    Some(format!(
                        "{} 次计数完整尝试，{} 至 {}；墙钟覆盖实际运行与轮间等待",
                        rate_samples.len(),
                        format_utc_iso_millis(start),
                        format_utc_iso_millis(end)
                    )),
                )
            } else {
                (None, None)
            }
        }
        _ => (None, None),
    };

    let freshness_limit_secs = interval_secs
        .map(|interval| interval.saturating_mul(2).max(120))
        .unwrap_or(15 * 60);
    let latest_pending_time = latest_pending
        .as_ref()
        .and_then(|metric| metric.observed_at.as_deref())
        .and_then(crate::stats::parse_to_utc_datetime);
    let latest_is_fresh = latest_pending_time.is_some_and(|observed_at| {
        let age = (now - observed_at).num_seconds();
        (0..=freshness_limit_secs).contains(&age)
            && capture_gap_at.is_none_or(|gap_at| observed_at > gap_at)
    });

    let mut estimate_samples = Vec::new();
    let mut estimate_identity: Option<(String, String, String)> = None;
    let estimate_cutoff = now - ChronoDuration::hours(24);
    for sample in samples.iter().rev() {
        let Some(metric) = sample.pending.as_ref() else {
            let attempted = crate::stats::parse_to_utc_datetime(&sample.attempted_at);
            if attempted.is_some_and(|time| time >= estimate_cutoff) {
                break;
            }
            break;
        };
        let Some(observed_at) = metric
            .observed_at
            .as_deref()
            .and_then(crate::stats::parse_to_utc_datetime)
        else {
            break;
        };
        if capture_gap_at.is_some_and(|gap_at| observed_at <= gap_at) {
            break;
        }
        if observed_at < estimate_cutoff {
            break;
        }
        if metric.value.is_none() || metric.coverage != "exact" {
            break;
        }
        let identity = (
            metric.unit.clone(),
            metric.scope.clone(),
            metric.range.clone(),
        );
        if estimate_identity
            .as_ref()
            .is_some_and(|expected| *expected != identity)
        {
            break;
        }
        estimate_identity = Some(identity);
        estimate_samples.push((observed_at, metric.value.unwrap_or_default()));
        if estimate_samples.len() == 20 {
            break;
        }
    }
    estimate_samples.reverse();

    let mut clearance_estimate_window = None;
    let mut clearance_estimate_coverage = None;
    let latest_blocked_by_gap = capture_gap_at
        .is_some_and(|gap_at| latest_pending_time.is_none_or(|observed_at| observed_at <= gap_at));
    let (clearance_eta, clearance_estimate_reason) = if latest_is_fresh
        && latest_pending
            .as_ref()
            .is_some_and(|metric| metric.value == Some(0))
    {
        (None, "cleared".to_string())
    } else if latest_blocked_by_gap {
        (None, "capture_gap".to_string())
    } else if !enabled {
        (None, "task_disabled".to_string())
    } else if !latest_is_fresh {
        (None, "stale_observation".to_string())
    } else if estimate_samples.len() < 5 {
        (None, "insufficient_samples".to_string())
    } else {
        let first = estimate_samples.first().map(|sample| sample.0);
        let last = estimate_samples.last().map(|sample| sample.0);
        let span_secs = first
            .zip(last)
            .map(|(first, last)| (last - first).num_seconds())
            .unwrap_or_default();
        if span_secs < 60 {
            (None, "insufficient_span".to_string())
        } else {
            let origin = first.unwrap_or(now);
            let points = estimate_samples
                .iter()
                .map(|(time, value)| {
                    (
                        (*time - origin).num_milliseconds() as f64 / 1_000.0,
                        *value as f64,
                    )
                })
                .collect::<Vec<_>>();
            let mean_x = points.iter().map(|point| point.0).sum::<f64>() / points.len() as f64;
            let mean_y = points.iter().map(|point| point.1).sum::<f64>() / points.len() as f64;
            let covariance = points
                .iter()
                .map(|(x, y)| (x - mean_x) * (y - mean_y))
                .sum::<f64>();
            let variance = points
                .iter()
                .map(|(x, _)| (x - mean_x).powi(2))
                .sum::<f64>();
            let slope = if variance > 0.0 {
                covariance / variance
            } else {
                0.0
            };
            let remaining = points.last().map(|point| point.1).unwrap_or_default();
            if slope >= 0.0 || remaining <= 0.0 {
                (None, "no_net_backlog_decline".to_string())
            } else {
                let seconds = remaining / -slope;
                let latest_observation = last.unwrap_or(now);
                let millis = (seconds * 1_000.0).ceil();
                let eta = if !millis.is_finite() || millis >= i64::MAX as f64 {
                    None
                } else {
                    latest_observation
                        .checked_add_signed(ChronoDuration::milliseconds(millis as i64))
                };
                if let Some(eta) = eta {
                    clearance_estimate_window = Some("最近 24 小时".to_string());
                    clearance_estimate_coverage = Some(format!(
                        "{} 个准确完整积压快照；{} 至 {}",
                        estimate_samples.len(),
                        format_utc_iso_millis(first.unwrap_or(now)),
                        format_utc_iso_millis(last.unwrap_or(now))
                    ));
                    (Some(format_utc_iso_millis(eta)), "estimated".to_string())
                } else {
                    (None, "estimate_out_of_range".to_string())
                }
            }
        }
    };

    TaskWorkloadSummary {
        latest_pending,
        latest_processed,
        latest_observed_at,
        processing_rate_per_second,
        processing_rate_window,
        clearance_eta,
        clearance_estimate_window,
        clearance_estimate_coverage,
        clearance_estimate_reason,
    }
}

#[derive(Debug, FromRow)]
struct RetentionBacklogObservationRow {
    bucket_start: String,
    observed_at: String,
    invocation_count: i64,
    max_overdue_seconds: Option<i64>,
    retention_days: i64,
    cutoff: String,
    source_max_invocation_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QueuedTaskRun {
    pub(crate) run_id: i64,
    pub(crate) task_key: String,
    pub(crate) title: String,
    pub(crate) trigger_kind: String,
    pub(crate) requested_at: String,
    pub(crate) waiting_ms: i64,
    pub(crate) position: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaskAdmissionWait {
    pub(crate) id: String,
    pub(crate) task_key: String,
    pub(crate) title: String,
    pub(crate) reason: String,
    pub(crate) started_at: String,
    pub(crate) waiting_ms: i64,
    pub(crate) retry_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineSegment {
    pub(crate) segment_id: String,
    pub(crate) kind: String,
    pub(crate) task_key: String,
    pub(crate) title: String,
    pub(crate) started_at: String,
    pub(crate) last_observed_at: String,
    pub(crate) finished_at: Option<String>,
    pub(crate) duration_ms: Option<i64>,
    pub(crate) status: String,
    pub(crate) trigger_kind: Option<String>,
    pub(crate) execution_class: Option<String>,
    pub(crate) reason: Option<String>,
    pub(crate) retry_at: Option<String>,
    pub(crate) active_child_task_key: Option<String>,
    pub(crate) active_child_title: Option<String>,
    pub(crate) managed_run_id: Option<i64>,
    pub(crate) session_id: String,
    pub(crate) revision: i64,
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineCoverage {
    pub(crate) session_id: String,
    pub(crate) started_at: String,
    pub(crate) last_seen_at: String,
    pub(crate) ended_at: Option<String>,
    pub(crate) dropped_events: i64,
}

pub(crate) const MANAGED_TASKS: &[(&str, &str, &str, &str, bool)] = &[
    (
        "retention_archive",
        "数据保留与归档",
        "按保留策略归档并清理历史数据",
        "interval",
        false,
    ),
    (
        "upstream_account_maintenance",
        "上游账号维护",
        "同步账号状态、配额与路由健康信息",
        "interval",
        false,
    ),
    (
        "forward_proxy_subscription_refresh",
        "正向代理订阅刷新",
        "刷新代理订阅并更新代理节点状态",
        "event",
        false,
    ),
    (
        "pool_orphan_recovery",
        "连接池孤儿记录恢复",
        "恢复超时或中断的连接池记录",
        "interval",
        false,
    ),
    (
        "startup_hourly_rollup_bootstrap",
        "启动时小时汇总补齐",
        "补齐启动阶段缺失的小时汇总数据",
        "startup",
        false,
    ),
    (
        "system_status_snapshot",
        "系统状态快照",
        "更新系统状态展示快照",
        "interval",
        false,
    ),
    (
        "invocation_timeline_snapshot",
        "调用时间线快照",
        "生成调用时间线展示所需的异步快照",
        "interval",
        false,
    ),
    (
        "summary_snapshot",
        "汇总快照",
        "更新统计汇总展示快照",
        "event",
        false,
    ),
    (
        "summary_coverage_recovery",
        "汇总覆盖恢复",
        "修复统计汇总的覆盖缺口",
        "interval",
        false,
    ),
    (
        "dashboard_runtime_projection_reconcile",
        "仪表盘运行投影校对",
        "校对仪表盘运行状态投影",
        "event",
        false,
    ),
    (
        "long_term_projection",
        "长期统计投影",
        "更新长期统计投影",
        "interval",
        false,
    ),
    (
        "timeseries_minute_projection",
        "分钟时序投影",
        "更新分钟级时序投影",
        "interval",
        false,
    ),
    (
        "raw_payload_metrics_inventory",
        "原始载荷指标盘点",
        "盘点原始请求与响应载荷的指标",
        "interval",
        false,
    ),
    (
        "prompt_cache_materialization",
        "Prompt 缓存物化",
        "将 Prompt 缓存会话信息物化到展示投影",
        "event",
        false,
    ),
    (
        "startup_backfill",
        "启动回填",
        "补齐历史字段并维护回填进度",
        "startup",
        false,
    ),
    (
        "raw_compression",
        "原始载荷压缩",
        "压缩冷数据原始载荷",
        "manual",
        true,
    ),
    (
        "archive_upstream_activity_manifest",
        "上游活动归档清单",
        "生成上游活动归档清单",
        "manual",
        true,
    ),
    (
        "materialize_historical_rollups",
        "历史汇总物化",
        "物化历史归档批次的统计汇总",
        "manual",
        true,
    ),
    (
        "verify_archive_storage",
        "归档存储校验",
        "校验归档文件、清单与记录一致性",
        "manual",
        true,
    ),
    (
        "prune_archive_batches",
        "归档批次清理",
        "清理符合安全条件的归档批次",
        "manual",
        true,
    ),
    (
        "prune_legacy_archive_batches",
        "旧归档批次清理",
        "清理符合条件的旧版归档批次",
        "manual",
        true,
    ),
];

pub(crate) const STARTUP_BACKFILL_TASKS: &[&str] = &[
    "proxy_usage",
    "prompt_cache_key",
    "prompt_cache_conversations_materialization",
    "requested_service_tier",
    "invocation_service_tier",
    "proxy_cost",
    "reasoning_effort",
    "failure_classification",
    "pool_attempt_public_id_live",
    "pool_attempt_public_id_archives",
    "upstream_activity_live",
    "upstream_activity_archives",
    "pool_upstream_node_health_archives",
    "account_activity_v2_coverage",
    "legacy_detail_mirrors",
    "historical_rollups",
];

fn startup_backfill_task_metadata(key: &str) -> (&'static str, &'static str) {
    match key {
        "proxy_usage" => ("代理用量回填", "回填代理用量字段并记录处理进度"),
        "prompt_cache_key" => ("Prompt 缓存键回填", "回填 Prompt 缓存键并建立关联"),
        "prompt_cache_conversations_materialization" => {
            ("Prompt 缓存会话物化", "物化 Prompt 缓存会话信息")
        }
        "requested_service_tier" => ("请求服务等级回填", "回填请求使用的服务等级"),
        "invocation_service_tier" => ("调用服务等级回填", "回填调用最终使用的服务等级"),
        "proxy_cost" => ("代理成本回填", "根据已记录的调用数据回填代理成本"),
        "reasoning_effort" => ("推理强度回填", "回填调用请求中的推理强度"),
        "failure_classification" => ("失败分类回填", "补齐失败类型与可处理性分类"),
        "pool_attempt_public_id_live" => ("在线连接池尝试 ID 回填", "回填在线连接池尝试的公共 ID"),
        "pool_attempt_public_id_archives" => {
            ("归档连接池尝试 ID 回填", "回填归档连接池尝试的公共 ID")
        }
        "upstream_activity_live" => ("在线上游活动回填", "回填在线上游活动记录"),
        "upstream_activity_archives" => ("归档上游活动回填", "回填归档上游活动记录"),
        "pool_upstream_node_health_archives" => {
            ("上游节点健康归档回填", "回填连接池上游节点的历史健康状态")
        }
        "account_activity_v2_coverage" => ("账号活动 v2 覆盖回填", "补齐账号活动 v2 的覆盖范围"),
        "legacy_detail_mirrors" => ("旧详情镜像回填", "维护旧详情字段的兼容镜像"),
        "historical_rollups" => ("历史汇总回填", "回填历史归档批次的统计汇总"),
        _ => ("启动回填子任务", "执行启动回填的一项历史字段补齐工作"),
    }
}

pub(crate) fn task_title_for_observation(task_key: &str) -> String {
    if let Some((_, title, _, _, _)) = MANAGED_TASKS.iter().find(|(key, ..)| *key == task_key) {
        return (*title).to_string();
    }
    if let Some(child) = task_key.strip_prefix("startup_backfill.") {
        return startup_backfill_task_metadata(child).0.to_string();
    }
    task_key.to_string()
}

pub(crate) fn task_execution_class(task_key: &str) -> Option<&'static str> {
    match task_key {
        "retention_archive"
        | "upstream_account_maintenance"
        | "pool_orphan_recovery"
        | "invocation_timeline_snapshot"
        | "raw_payload_metrics_inventory" => Some("maintenance_retention"),
        "dashboard_runtime_projection_reconcile"
        | "forward_proxy_subscription_refresh"
        | "long_term_projection"
        | "timeseries_minute_projection" => Some("p2_derived"),
        _ => None,
    }
}

pub(crate) fn task_enabled_by_default(task_key: &str, is_manual: bool) -> bool {
    !is_manual && DEFAULT_ENABLED_TASKS.contains(&task_key)
}

const EDITABLE_SCHEDULE_TASKS: &[&str] = &[
    "retention_archive",
    "upstream_account_maintenance",
    "pool_orphan_recovery",
    "system_status_snapshot",
    "invocation_timeline_snapshot",
    "dashboard_runtime_projection_reconcile",
];

fn task_trigger_kinds(task_key: &str, trigger_mode: &str, is_manual: bool) -> Vec<String> {
    if is_manual || trigger_mode == "manual" {
        return vec!["manual".to_string()];
    }
    let kinds: &[&str] = match task_key {
        "retention_archive" | "upstream_account_maintenance" => &["startup", "interval"],
        "forward_proxy_subscription_refresh" => &["startup", "interval"],
        "dashboard_runtime_projection_reconcile" => &["interval", "adaptive"],
        "summary_snapshot" | "prompt_cache_materialization" => &["event", "interval"],
        "summary_coverage_recovery" | "long_term_projection" => &["adaptive", "interval"],
        "timeseries_minute_projection" => &["startup", "interval", "adaptive"],
        "startup_backfill" => &["startup", "event", "adaptive"],
        key if key.starts_with("startup_backfill.") => &["event", "interval"],
        _ if trigger_mode == "startup" => &["startup"],
        _ if trigger_mode == "event" => &["event"],
        _ => &["interval"],
    };
    kinds.iter().map(|kind| (*kind).to_string()).collect()
}

fn task_policy_text(
    task_key: &str,
    interval_secs: Option<i64>,
    cron_expr: Option<&str>,
) -> (&'static str, String) {
    if let Some(cron) = cron_expr.map(str::trim).filter(|value| !value.is_empty()) {
        return ("运维自定义", format!("UTC cron：{cron}"));
    }
    if let Some(interval) = interval_secs {
        return ("运维自定义", format!("固定检查间隔：{interval} 秒"));
    }
    if MANAGED_TASKS
        .iter()
        .any(|(key, .., is_manual)| *key == task_key && *is_manual)
    {
        return ("系统规则", "手动触发；不适用周期计划".to_string());
    }
    match task_key {
        "system_status_snapshot" => (
            "系统默认",
            "固定检查间隔：55 秒（60 秒缓存上限，提前 5 秒）".to_string(),
        ),
        "upstream_account_maintenance" => (
            "系统默认",
            "固定检查间隔：60 秒；账号同步按账号策略".to_string(),
        ),
        "pool_orphan_recovery" => ("系统默认", "固定检查间隔：60 秒".to_string()),
        "invocation_timeline_snapshot" => ("系统默认", "固定检查间隔：60 秒".to_string()),
        "dashboard_runtime_projection_reconcile" => (
            "系统默认",
            "固定检查间隔：60 秒；受压力准入约束".to_string(),
        ),
        "retention_archive" => (
            "运行配置",
            "启动检查与保留策略周期；按配置判断是否有工作".to_string(),
        ),
        "forward_proxy_subscription_refresh" => {
            ("系统默认", "启动刷新与固定检查间隔：60 秒".to_string())
        }
        "long_term_projection" => (
            "系统默认",
            "自适应检查；60 秒刷新、300 秒修复、每日校验".to_string(),
        ),
        "timeseries_minute_projection" => ("系统默认", "启动、固定检查与压力准入唤醒".to_string()),
        "summary_snapshot" => ("系统默认", "事件唤醒；最小刷新间隔 10 秒".to_string()),
        "summary_coverage_recovery" => ("系统默认", "自适应恢复；按覆盖和压力准入唤醒".to_string()),
        "prompt_cache_materialization" => ("系统默认", "事件对账与 60 秒检查".to_string()),
        "startup_backfill" => (
            "系统默认",
            "启动监督器；按事件、检查点和压力准入唤醒".to_string(),
        ),
        key if key.starts_with("startup_backfill.") => {
            ("系统默认", "回填父任务按事件和检查点调度".to_string())
        }
        key if key.contains("hourly") => ("启动规则", "仅在服务启动阶段检查".to_string()),
        _ => ("系统默认", "由 worker 的固定或事件规则检查".to_string()),
    }
}

fn decorate_task(mut task: ManagedTask) -> ManagedTask {
    let editable = EDITABLE_SCHEDULE_TASKS.contains(&task.task_key.as_str());
    let policy_interval_secs = (task.schedule_source.as_deref() != Some("default"))
        .then_some(task.interval_secs)
        .flatten();
    let (policy_source, effective_policy) = task_policy_text(
        &task.task_key,
        policy_interval_secs,
        task.cron_expr.as_deref(),
    );
    task.trigger_kinds = task_trigger_kinds(&task.task_key, &task.trigger_mode, task.is_manual);
    task.effective_policy = effective_policy;
    task.policy_source = policy_source.to_string();
    task.schedule_editable = editable;
    task.schedule_capability_reason = if task.is_manual {
        Some("手动任务没有周期或 cron 计划".to_string())
    } else if editable {
        None
    } else if task.interval_secs.is_some()
        || task
            .cron_expr
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        Some("当前 worker 为事件、启动或自适应路径，保留已有覆盖但不支持新增覆盖".to_string())
    } else {
        Some("当前任务只读展示真实 worker 策略".to_string())
    };
    task.execution_class = task_execution_class(&task.task_key).map(str::to_string);
    task.measurement_capabilities = Some(task_measurement_capabilities(&task.task_key));
    task
}

fn metric_capability(
    supported: bool,
    unit: Option<&str>,
    scope: Option<&str>,
) -> TaskMetricCapability {
    TaskMetricCapability {
        supported,
        unit: unit.map(str::to_string),
        scope: scope.map(str::to_string),
    }
}

pub(crate) fn task_measurement_capabilities(task_key: &str) -> TaskMeasurementCapabilities {
    let retention_scope = "expired_invocations:retention_policy";
    if task_key == "retention_archive" {
        return TaskMeasurementCapabilities {
            pending: metric_capability(true, Some("invocation rows"), Some(retention_scope)),
            discovered: metric_capability(true, Some("invocation rows"), Some(retention_scope)),
            processed: metric_capability(true, Some("invocation rows"), Some(retention_scope)),
        };
    }

    let backfill_unit = match task_key.strip_prefix("startup_backfill.") {
        Some(
            "proxy_usage"
            | "prompt_cache_key"
            | "requested_service_tier"
            | "invocation_service_tier"
            | "proxy_cost"
            | "reasoning_effort"
            | "failure_classification",
        ) => Some("invocation rows"),
        Some("prompt_cache_conversations_materialization") => Some("conversation sessions"),
        Some("pool_attempt_public_id_live" | "pool_attempt_public_id_archives") => {
            Some("pool attempt rows")
        }
        Some("upstream_activity_live" | "upstream_activity_archives") => Some("accounts"),
        Some("account_activity_v2_coverage") => Some("account-activity buckets"),
        Some("legacy_detail_mirrors") => Some("archive detail paths"),
        Some("historical_rollups") => Some("archive rollup paths"),
        _ => None,
    };
    if let Some(unit) = backfill_unit {
        let discovered_supported =
            task_key == "startup_backfill.pool_upstream_node_health_archives";
        return TaskMeasurementCapabilities {
            pending: metric_capability(false, None, None),
            discovered: metric_capability(
                discovered_supported,
                discovered_supported.then_some(unit),
                discovered_supported.then_some(task_key),
            ),
            processed: metric_capability(true, Some(unit), Some(task_key)),
        };
    }

    let processed_unit = match task_key {
        "upstream_account_maintenance" => Some("maintenance plans"),
        "invocation_timeline_snapshot" => Some("timeline records"),
        "summary_coverage_recovery" => Some("coverage proofs"),
        "timeseries_minute_projection" => Some("minute projection keys"),
        "raw_payload_metrics_inventory" => Some("inventory paths"),
        "prompt_cache_materialization" => Some("conversation sessions"),
        "forward_proxy_subscription_refresh" => Some("subscription sources"),
        "startup_hourly_rollup_bootstrap" => Some("rollup writes"),
        "summary_snapshot" => Some("live terminal contributions"),
        "dashboard_runtime_projection_reconcile" => Some("reconciled runtime records"),
        "long_term_projection" => Some("terminal events"),
        "startup_backfill.pool_upstream_node_health_archives" => Some("archive batches"),
        "pool_orphan_recovery" => Some("pool attempts"),
        "raw_compression" => Some("raw payload files"),
        "archive_upstream_activity_manifest" => Some("archive batches"),
        "materialize_historical_rollups" => Some("archive batches"),
        "verify_archive_storage" => Some("archive manifest rows"),
        "prune_archive_batches" | "prune_legacy_archive_batches" => Some("archive batches"),
        _ => None,
    };
    if let Some(unit) = processed_unit {
        let discovered_supported = matches!(
            task_key,
            "upstream_account_maintenance"
                | "invocation_timeline_snapshot"
                | "summary_coverage_recovery"
                | "timeseries_minute_projection"
                | "raw_payload_metrics_inventory"
                | "forward_proxy_subscription_refresh"
                | "startup_hourly_rollup_bootstrap"
                | "summary_snapshot"
                | "dashboard_runtime_projection_reconcile"
                | "long_term_projection"
                | "startup_backfill.pool_upstream_node_health_archives"
        );
        return TaskMeasurementCapabilities {
            pending: metric_capability(false, None, None),
            discovered: metric_capability(
                discovered_supported,
                discovered_supported.then_some(unit),
                discovered_supported.then_some(task_key),
            ),
            processed: metric_capability(true, Some(unit), Some(task_key)),
        };
    }

    TaskMeasurementCapabilities {
        pending: metric_capability(false, None, None),
        discovered: metric_capability(false, None, None),
        processed: metric_capability(false, None, None),
    }
}

async fn recent_runs_for_workload_compatibility(
    pool: &Pool<Sqlite>,
    task_key: &str,
    window: Option<(&str, &str)>,
) -> Result<Vec<TaskWorkloadLegacyRun>> {
    let mut query = if window.is_some() {
        sqlx::query_as::<_, TaskWorkloadLegacyRun>(
            "SELECT id,trigger_kind,started_at,actual_started_at,finished_at,actual_finished_at,duration_ms,actual_duration_ms,status,error_detail,execution_uid
             FROM managed_task_runs
             WHERE task_key=? AND status NOT IN ('requested','queued')
               AND started_at>=? AND started_at<=?
               AND NOT EXISTS (SELECT 1 FROM managed_task_work_runs wr WHERE wr.managed_run_id=managed_task_runs.id)
             ORDER BY COALESCE(actual_started_at,started_at) DESC,id DESC LIMIT 200",
        )
        .bind(task_key)
    } else {
        sqlx::query_as::<_, TaskWorkloadLegacyRun>(
            "SELECT id,trigger_kind,started_at,actual_started_at,finished_at,actual_finished_at,duration_ms,actual_duration_ms,status,error_detail,execution_uid
             FROM managed_task_runs
             WHERE task_key=? AND status NOT IN ('requested','queued')
               AND NOT EXISTS (SELECT 1 FROM managed_task_work_runs wr WHERE wr.managed_run_id=managed_task_runs.id)
             ORDER BY COALESCE(actual_started_at,started_at) DESC,id DESC LIMIT 200",
        )
        .bind(task_key)
    };
    if let Some((window_start, window_end)) = window {
        query = query.bind(window_start).bind(window_end);
    }
    Ok(query.fetch_all(pool).await?)
}

pub(crate) async fn open(config: &AppConfig) -> Result<MaintenanceStore> {
    let database_path = config.maintenance_database_path();
    if let Some(parent) = database_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let url = format!("sqlite://{}", database_path.to_string_lossy());
    let options = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(2));
    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(options)
        .await?;
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;
    ensure_schema(&pool).await?;
    record_prompt_cache_materialization_control_origin(&pool).await?;
    seed_tasks(&pool).await?;
    ensure_task_colors(&pool).await?;
    Ok(MaintenanceStore::from_pool(pool))
}

async fn record_prompt_cache_materialization_control_origin(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO maintenance_metadata (key,value,updated_at)
         SELECT ?, CASE WHEN EXISTS (
             SELECT 1 FROM managed_tasks
             WHERE task_key='startup_backfill.prompt_cache_conversations_materialization'
         ) THEN 'maintenance' ELSE 'legacy' END, ?",
    )
    .bind(PROMPT_CACHE_CONTROL_ORIGIN_MARKER)
    .bind(format_utc_iso_millis(Utc::now()))
    .execute(pool)
    .await?;
    Ok(())
}

async fn ensure_task_colors(pool: &Pool<Sqlite>) -> Result<()> {
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT task_key,display_color_light,display_color_dark FROM managed_tasks ORDER BY task_key",
    )
    .fetch_all(pool)
    .await?;
    let mut used = HashSet::new();
    let mut next_hue = 0.0_f64;
    for (task_key, light, dark) in rows {
        if let (Some(light), Some(dark)) = (&light, &dark) {
            used.insert((light.clone(), dark.clone()));
            continue;
        }
        loop {
            let hue = next_hue % 360.0;
            next_hue += 137.507_764;
            let generated = (hsl_hex(hue, 0.72, 0.43), hsl_hex(hue, 0.76, 0.66));
            let colors = (
                light.clone().unwrap_or_else(|| generated.0.clone()),
                dark.clone().unwrap_or_else(|| generated.1.clone()),
            );
            if used.insert(colors.clone()) {
                sqlx::query(
                    "UPDATE managed_tasks SET display_color_light=COALESCE(display_color_light,?),display_color_dark=COALESCE(display_color_dark,?) WHERE task_key=?",
                )
                .bind(&generated.0)
                .bind(&generated.1)
                .bind(&task_key)
                .execute(pool)
                .await?;
                break;
            }
        }
    }
    Ok(())
}

fn hsl_hex(hue: f64, saturation: f64, lightness: f64) -> String {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue / 60.0;
    let x = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match sector as u8 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let offset = lightness - chroma / 2.0;
    format!(
        "#{:02x}{:02x}{:02x}",
        ((red + offset) * 255.0).round() as u8,
        ((green + offset) * 255.0).round() as u8,
        ((blue + offset) * 255.0).round() as u8,
    )
}

fn validate_cron_expr(expr: Option<&str>) -> Result<()> {
    let Some(expr) = expr.map(str::trim) else {
        return Ok(());
    };
    if expr.is_empty() {
        return Err(anyhow!(
            "cron expression must contain exactly five UTC fields"
        ));
    }
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 || fields.iter().any(|field| field.is_empty()) {
        return Err(anyhow!(
            "cron expression must contain exactly five UTC fields"
        ));
    }
    let ranges = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];
    for (field, (minimum, maximum)) in fields.iter().zip(ranges) {
        validate_cron_field(field, minimum, maximum)?;
    }
    Ok(())
}

fn validate_cron_field(field: &str, minimum: u32, maximum: u32) -> Result<()> {
    if field.is_empty() {
        return Err(anyhow!("cron expression contains an empty field"));
    }
    for part in field.split(',') {
        if part.is_empty() {
            return Err(anyhow!("cron expression contains an empty list item"));
        }
        let (base, step) = match part.split_once('/') {
            Some((base, step)) => {
                if step.is_empty() || step.contains('/') {
                    return Err(anyhow!("cron step is invalid"));
                }
                let step = step
                    .parse::<u32>()
                    .map_err(|_| anyhow!("cron step is invalid"))?;
                if step == 0 || step > maximum.saturating_sub(minimum) + 1 {
                    return Err(anyhow!("cron step is out of range"));
                }
                (base, step)
            }
            None => (part, 1),
        };
        if base == "*" {
            continue;
        }
        if let Some((start, end)) = base.split_once('-') {
            let start = start
                .parse::<u32>()
                .map_err(|_| anyhow!("cron range is invalid"))?;
            let end = end
                .parse::<u32>()
                .map_err(|_| anyhow!("cron range is invalid"))?;
            if start < minimum || end > maximum || start > end {
                return Err(anyhow!("cron range is out of range"));
            }
            continue;
        }
        let value = base
            .parse::<u32>()
            .map_err(|_| anyhow!("cron value is invalid"))?;
        if value < minimum || value > maximum {
            return Err(anyhow!("cron value is out of range"));
        }
        if step != 1 {
            return Err(anyhow!("cron step requires a wildcard or range"));
        }
    }
    Ok(())
}

fn cron_field_matches(field: &str, value: u32, minimum: u32, maximum: u32) -> bool {
    field.split(',').any(|part| {
        let (base, step) = part
            .split_once('/')
            .map_or((part, 1), |(base, step)| (base, step.parse().unwrap_or(0)));
        if step == 0 {
            return false;
        }
        if base == "*" {
            return (value - minimum).is_multiple_of(step);
        }
        if let Some((start, end)) = base.split_once('-') {
            let Ok(start) = start.parse::<u32>() else {
                return false;
            };
            let Ok(end) = end.parse::<u32>() else {
                return false;
            };
            return start >= minimum
                && end <= maximum
                && start <= end
                && value >= start
                && value <= end
                && (value - start).is_multiple_of(step);
        }
        base.parse::<u32>()
            .is_ok_and(|exact| exact == value && exact >= minimum && exact <= maximum)
    })
}

fn cron_field_is_unrestricted(field: &str) -> bool {
    field == "*"
}

fn cron_day_matches(dom_field: &str, dow_field: &str, candidate: chrono::DateTime<Utc>) -> bool {
    let dom_unrestricted = cron_field_is_unrestricted(dom_field);
    let dow_unrestricted = cron_field_is_unrestricted(dow_field);
    let dom_match = cron_field_matches(dom_field, candidate.day(), 1, 31);
    let dow_match = cron_field_matches(dow_field, candidate.weekday().num_days_from_sunday(), 0, 6);
    match (dom_unrestricted, dow_unrestricted) {
        (true, true) => true,
        (true, false) => dow_match,
        (false, true) => dom_match,
        (false, false) => dom_match || dow_match,
    }
}

pub(crate) fn sanitize_task_detail(value: &str) -> String {
    let mut sanitized = value.replace(['\r', '\n'], " ");
    for marker in [
        "Authorization:",
        "authorization=",
        "api_key=",
        "access_token=",
        "token=",
    ] {
        while let Some(start) = sanitized
            .to_ascii_lowercase()
            .find(&marker.to_ascii_lowercase())
        {
            let value_start = start + marker.len();
            let value_slice = &sanitized[value_start..];
            let secret_start = value_start + value_slice.len() - value_slice.trim_start().len();
            let value_end = sanitized[secret_start..]
                .find(|character: char| {
                    character.is_whitespace() || character == ',' || character == ';'
                })
                .map_or(sanitized.len(), |offset| secret_start + offset);
            sanitized.replace_range(start..value_end, "[REDACTED]");
        }
    }
    sanitized
        .chars()
        .take(MAX_TASK_ERROR_DETAIL_CHARS)
        .collect()
}

fn next_trigger_at(interval_secs: Option<i64>, cron_expr: Option<&str>) -> Option<String> {
    let now = Utc::now();
    if let Some(expr) = cron_expr.map(str::trim).filter(|value| !value.is_empty()) {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return None;
        }
        let base = now
            - ChronoDuration::seconds(i64::from(now.second()))
            - ChronoDuration::nanoseconds(i64::from(now.nanosecond()));
        for offset in 1..=(5 * 366 * 24 * 60) {
            let candidate = base + ChronoDuration::minutes(offset);
            if cron_field_matches(fields[0], candidate.minute(), 0, 59)
                && cron_field_matches(fields[1], candidate.hour(), 0, 23)
                && cron_field_matches(fields[3], candidate.month(), 1, 12)
                && cron_day_matches(fields[2], fields[4], candidate)
            {
                return Some(format_utc_iso_millis(candidate));
            }
        }
        return None;
    }
    interval_secs
        .filter(|seconds| *seconds >= MIN_INTERVAL_SECS)
        .map(|seconds| format_utc_iso_millis(now + ChronoDuration::seconds(seconds)))
}

fn floor_utc_hour(value: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    value
        - ChronoDuration::seconds(i64::from(value.minute() * 60 + value.second()))
        - ChronoDuration::nanoseconds(i64::from(value.nanosecond()))
}

fn decorate_effective_schedule(mut task: ManagedTask) -> ManagedTask {
    if task.is_manual {
        return task;
    }
    let (source, interval_secs, cron_expr) = if task.schedule_source.as_deref() == Some("default") {
        ("default", task.interval_secs, task.cron_expr.clone())
    } else if task
        .cron_expr
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        || task.interval_secs.is_some()
    {
        ("override", task.interval_secs, task.cron_expr.clone())
    } else {
        return task;
    };
    task.effective_schedule = Some(ManagedTaskSchedule {
        source: source.to_string(),
        interval_secs,
        cron_expr: cron_expr.clone(),
        next_trigger_at: task
            .next_trigger_at
            .clone()
            .or_else(|| next_trigger_at(interval_secs, cron_expr.as_deref())),
    });
    task
}

fn decode_task_stages(raw: Option<&str>) -> Option<Vec<TaskStage>> {
    raw.and_then(|value| serde_json::from_str(value).ok())
}

fn task_progress_from_row(row: TaskProgressRow) -> TaskProgress {
    let freshness = if row.freshness == "fresh"
        && row.updated_at.as_deref().is_some_and(|updated_at| {
            chrono::DateTime::parse_from_rfc3339(updated_at)
                .ok()
                .is_some_and(|updated_at| {
                    Utc::now()
                        .signed_duration_since(updated_at.with_timezone(&Utc))
                        .num_seconds()
                        > TASK_PROGRESS_STALE_AFTER_SECS
                })
        }) {
        "stale".to_string()
    } else {
        row.freshness.clone()
    };
    TaskProgress {
        total: row.total,
        completed: row.completed,
        phase: row.phase,
        checkpoint: row.checkpoint,
        eta_seconds: row.eta_seconds,
        updated_at: row.updated_at,
        freshness,
        unit: row.unit,
        source_scope: row.source_scope,
        last_progress_at: row.last_progress_at,
        wait_reason: row.wait_reason,
        next_retry_at: row.next_retry_at,
        next_inspection_at: row.next_inspection_at,
        next_catchup_at: row.next_catchup_at,
        catchup_state: row.catchup_state,
        stages: decode_task_stages(row.stages.as_deref()),
    }
}

fn task_run_from_row(row: TaskRunRow) -> TaskRun {
    TaskRun {
        id: row.id,
        trigger_kind: row.trigger_kind,
        started_at: row.started_at,
        finished_at: row.finished_at,
        duration_ms: row.duration_ms,
        status: row.status,
        summary: row.summary,
        processed_count: row.processed_count,
        updated_count: row.updated_count,
        error_detail: row.error_detail,
        completion: row.completion,
        core_completion: row.core_completion,
        details: row
            .details
            .and_then(|value| serde_json::from_str(&value).ok()),
    }
}

pub(crate) fn managed_startup_backfill_suffix(task_name: &str) -> Option<&'static str> {
    if task_name == crate::STARTUP_BACKFILL_TASK_PROXY_COST
        || task_name
            .strip_prefix(crate::STARTUP_BACKFILL_TASK_PROXY_COST)
            .is_some_and(|suffix| suffix.starts_with(':'))
    {
        return Some("proxy_cost");
    }

    [
        (crate::STARTUP_BACKFILL_TASK_PROXY_USAGE, "proxy_usage"),
        (
            crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_KEY,
            "prompt_cache_key",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION,
            "prompt_cache_conversations_materialization",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_REQUESTED_SERVICE_TIER,
            "requested_service_tier",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_INVOCATION_SERVICE_TIER,
            "invocation_service_tier",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_REASONING_EFFORT,
            "reasoning_effort",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_FAILURE_CLASSIFICATION,
            "failure_classification",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_LIVE,
            "pool_attempt_public_id_live",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_ARCHIVES,
            "pool_attempt_public_id_archives",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_LIVE,
            "upstream_activity_live",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES,
            "upstream_activity_archives",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_POOL_UPSTREAM_NODE_HEALTH_ARCHIVES,
            "pool_upstream_node_health_archives",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_ACCOUNT_ACTIVITY_V2_COVERAGE,
            "account_activity_v2_coverage",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_LEGACY_DETAIL_MIRRORS,
            "legacy_detail_mirrors",
        ),
        (
            crate::STARTUP_BACKFILL_TASK_HISTORICAL_ROLLUPS,
            "historical_rollups",
        ),
    ]
    .into_iter()
    .find_map(|(legacy_name, managed_suffix)| (task_name == legacy_name).then_some(managed_suffix))
}

async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    for statement in r#"
        CREATE TABLE IF NOT EXISTS managed_tasks (
          task_key TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL,
          trigger_mode TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
          interval_secs INTEGER, cron_expr TEXT, next_trigger_at TEXT,
          next_catchup_at TEXT, catchup_reason TEXT,
          is_manual INTEGER NOT NULL DEFAULT 0, schedule_source TEXT,
          display_color_light TEXT, display_color_dark TEXT,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS managed_task_progress (
          task_key TEXT PRIMARY KEY REFERENCES managed_tasks(task_key) ON DELETE CASCADE,
          total INTEGER, completed INTEGER, phase TEXT, checkpoint TEXT, eta_seconds INTEGER,
          updated_at TEXT, freshness TEXT NOT NULL DEFAULT 'fresh', unit TEXT, source_scope TEXT,
          last_progress_at TEXT, wait_reason TEXT, next_retry_at TEXT,
          next_inspection_at TEXT, next_catchup_at TEXT, catchup_state TEXT, stages TEXT
        );
        CREATE TABLE IF NOT EXISTS startup_backfill_progress (
          task_name TEXT PRIMARY KEY,
          cursor_id INTEGER NOT NULL DEFAULT 0,
          next_run_after TEXT,
          zero_update_streak INTEGER NOT NULL DEFAULT 0,
          last_started_at TEXT,
          last_finished_at TEXT,
          last_scanned INTEGER NOT NULL DEFAULT 0,
          last_updated INTEGER NOT NULL DEFAULT 0,
          last_status TEXT NOT NULL DEFAULT 'idle',
          suspension_reason TEXT,
          next_probe_at TEXT,
          wake_generation INTEGER NOT NULL DEFAULT 0,
          enabled INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE IF NOT EXISTS managed_task_runs (
          id INTEGER PRIMARY KEY AUTOINCREMENT, legacy_id INTEGER UNIQUE, task_key TEXT NOT NULL,
          trigger_kind TEXT NOT NULL DEFAULT 'unknown', started_at TEXT NOT NULL, finished_at TEXT,
          duration_ms INTEGER, status TEXT NOT NULL, summary TEXT,
          processed_count INTEGER, updated_count INTEGER, error_detail TEXT,
          completion TEXT, core_completion TEXT, details TEXT,
          execution_uid TEXT, actual_started_at TEXT, actual_finished_at TEXT,
          actual_duration_ms INTEGER
        );
        CREATE TABLE IF NOT EXISTS managed_task_work_runs (
          execution_uid TEXT NOT NULL,
          task_key TEXT NOT NULL,
          managed_run_id INTEGER,
          attempted_at TEXT NOT NULL,
          sequence INTEGER NOT NULL,
          status TEXT NOT NULL DEFAULT 'unknown',
          sample_json TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          PRIMARY KEY(execution_uid,task_key)
        );
        CREATE INDEX IF NOT EXISTS idx_managed_task_work_runs_task_attempted
          ON managed_task_work_runs(task_key,attempted_at DESC,execution_uid DESC);
        CREATE TABLE IF NOT EXISTS task_timeline_segments (
          segment_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, kind TEXT NOT NULL,
          task_key TEXT NOT NULL, title TEXT NOT NULL, started_at TEXT NOT NULL,
          last_observed_at TEXT NOT NULL, finished_at TEXT, duration_ms INTEGER,
          status TEXT NOT NULL, trigger_kind TEXT, execution_class TEXT,
          reason TEXT, retry_at TEXT, active_child_task_key TEXT, active_child_title TEXT,
          managed_run_id INTEGER, revision INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_task_timeline_segments_started
          ON task_timeline_segments(started_at, segment_id);
        CREATE INDEX IF NOT EXISTS idx_task_timeline_segments_open
          ON task_timeline_segments(kind, status, task_key);
        CREATE TABLE IF NOT EXISTS task_timeline_coverage (
          session_id TEXT PRIMARY KEY, started_at TEXT NOT NULL, last_seen_at TEXT NOT NULL,
          ended_at TEXT, dropped_events INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_managed_task_runs_task_started
          ON managed_task_runs(task_key, started_at DESC);
        CREATE TABLE IF NOT EXISTS maintenance_metadata (
          key TEXT PRIMARY KEY,
          value TEXT NOT NULL,
          updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS retention_backlog_hourly_observations (
          bucket_start TEXT PRIMARY KEY,
          observed_at TEXT NOT NULL,
          invocation_count INTEGER NOT NULL,
          max_overdue_seconds INTEGER,
          retention_days INTEGER NOT NULL,
          cutoff TEXT NOT NULL,
          source_max_invocation_id INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_retention_backlog_observations_observed
          ON retention_backlog_hourly_observations(observed_at);
    "#
    .split(';')
    .map(str::trim)
    .filter(|statement| !statement.is_empty())
    {
        sqlx::query(statement).execute(pool).await?;
    }
    // Existing maintenance databases may predate legacy_id; keep upgrades additive.
    let has_legacy_id: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM pragma_table_info('managed_task_runs') WHERE name = 'legacy_id'",
    )
    .fetch_optional(pool)
    .await?;
    if has_legacy_id.is_none() {
        sqlx::query("ALTER TABLE managed_task_runs ADD COLUMN legacy_id INTEGER")
            .execute(pool)
            .await?;
    }
    for (column, definition) in [
        ("trigger_kind", "TEXT NOT NULL DEFAULT 'unknown'"),
        ("summary", "TEXT"),
        ("completion", "TEXT"),
        ("core_completion", "TEXT"),
        ("details", "TEXT"),
        ("execution_uid", "TEXT"),
        ("actual_started_at", "TEXT"),
        ("actual_finished_at", "TEXT"),
        ("actual_duration_ms", "INTEGER"),
    ] {
        let present: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT 1 FROM pragma_table_info('managed_task_runs') WHERE name = '{column}'"
        ))
        .fetch_optional(pool)
        .await?;
        if present.is_none() {
            sqlx::query(&format!(
                "ALTER TABLE managed_task_runs ADD COLUMN {column} {definition}"
            ))
            .execute(pool)
            .await?;
        }
    }
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_managed_task_runs_execution_uid ON managed_task_runs(execution_uid)",
    )
    .execute(pool)
    .await?;
    for (column, definition) in [
        ("unit", "TEXT"),
        ("source_scope", "TEXT"),
        ("last_progress_at", "TEXT"),
        ("wait_reason", "TEXT"),
        ("next_retry_at", "TEXT"),
        ("next_inspection_at", "TEXT"),
        ("next_catchup_at", "TEXT"),
        ("catchup_state", "TEXT"),
        ("stages", "TEXT"),
    ] {
        let present: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT 1 FROM pragma_table_info('managed_task_progress') WHERE name = '{column}'"
        ))
        .fetch_optional(pool)
        .await?;
        if present.is_none() {
            sqlx::query(&format!(
                "ALTER TABLE managed_task_progress ADD COLUMN {column} {definition}"
            ))
            .execute(pool)
            .await?;
        }
    }
    let has_next_trigger_at: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM pragma_table_info('managed_tasks') WHERE name = 'next_trigger_at'",
    )
    .fetch_optional(pool)
    .await?;
    if has_next_trigger_at.is_none() {
        sqlx::query("ALTER TABLE managed_tasks ADD COLUMN next_trigger_at TEXT")
            .execute(pool)
            .await?;
    }
    let has_schedule_source: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM pragma_table_info('managed_tasks') WHERE name = 'schedule_source'",
    )
    .fetch_optional(pool)
    .await?;
    if has_schedule_source.is_none() {
        sqlx::query("ALTER TABLE managed_tasks ADD COLUMN schedule_source TEXT")
            .execute(pool)
            .await?;
    }
    for (column, definition) in [
        ("next_catchup_at", "TEXT"),
        ("catchup_reason", "TEXT"),
        ("display_color_light", "TEXT"),
        ("display_color_dark", "TEXT"),
    ] {
        let present: Option<i64> = sqlx::query_scalar(&format!(
            "SELECT 1 FROM pragma_table_info('managed_tasks') WHERE name = '{column}'"
        ))
        .fetch_optional(pool)
        .await?;
        if present.is_none() {
            sqlx::query(&format!(
                "ALTER TABLE managed_tasks ADD COLUMN {column} {definition}"
            ))
            .execute(pool)
            .await?;
        }
    }
    // Keep databases written by the old dual-field form deterministic: an explicit cron wins.
    // Recompute the persisted trigger at the same time; an interval-derived timestamp is not
    // valid once the cron field becomes authoritative.
    let mut schedule_repair = pool.begin().await?;
    let legacy_dual_schedule_rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT task_key,cron_expr FROM managed_tasks WHERE cron_expr IS NOT NULL AND trim(cron_expr) <> '' AND interval_secs IS NOT NULL",
    )
    .fetch_all(&mut *schedule_repair)
    .await?;
    sqlx::query(
        "UPDATE managed_tasks SET interval_secs=NULL WHERE cron_expr IS NOT NULL AND trim(cron_expr) <> '' AND interval_secs IS NOT NULL",
    )
    .execute(&mut *schedule_repair)
    .await?;
    for (task_key, cron_expr) in legacy_dual_schedule_rows {
        sqlx::query("UPDATE managed_tasks SET next_trigger_at=? WHERE task_key=?")
            .bind(next_trigger_at(None, cron_expr.as_deref()))
            .bind(task_key)
            .execute(&mut *schedule_repair)
            .await?;
    }
    schedule_repair.commit().await?;
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_task_runs_legacy_id ON managed_task_runs(legacy_id) WHERE legacy_id IS NOT NULL")
        .execute(pool)
        .await?;
    let work_run_status_column: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM pragma_table_info('managed_task_work_runs') WHERE name='status'",
    )
    .fetch_optional(pool)
    .await?;
    if work_run_status_column.is_none() {
        sqlx::query(
            "ALTER TABLE managed_task_work_runs ADD COLUMN status TEXT NOT NULL DEFAULT 'unknown'",
        )
        .execute(pool)
        .await?;
    }
    let superseded_at = format_utc_iso_millis(Utc::now());
    sqlx::query(
        "UPDATE managed_task_runs
         SET status='failed', finished_at=COALESCE(finished_at, ?1), duration_ms=COALESCE(duration_ms, 0),
             summary=COALESCE(summary, '重复的活动运行已停止'),
             error_detail=COALESCE(error_detail, '迁移时保留较早活动运行，重复记录已终止')
         WHERE id IN (
             SELECT duplicate.id
             FROM managed_task_runs duplicate
             JOIN managed_task_runs kept
               ON kept.task_key = duplicate.task_key
              AND kept.status IN ('running','requested')
              AND duplicate.status IN ('running','requested')
              AND kept.id < duplicate.id
         )",
    )
    .bind(&superseded_at)
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_managed_task_active_run ON managed_task_runs(task_key) WHERE status IN ('running','requested')",
    )
    .execute(pool)
    .await?;
    let now = Utc::now();
    let error_cutoff = format_utc_iso_millis(now - ChronoDuration::days(TASK_ERROR_RETENTION_DAYS));
    let run_cutoff = format_utc_iso_millis(now - ChronoDuration::days(TASK_RUN_RETENTION_DAYS));
    sqlx::query("UPDATE managed_task_runs SET error_detail=NULL WHERE started_at < ?")
        .bind(error_cutoff)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM managed_task_runs WHERE started_at < ?")
        .bind(run_cutoff)
        .execute(pool)
        .await?;
    sqlx::query(
        "DELETE FROM managed_task_work_runs
         WHERE rowid IN (
           SELECT rowid FROM (
             SELECT rowid, ROW_NUMBER() OVER (PARTITION BY task_key ORDER BY attempted_at DESC, execution_uid DESC) AS sample_rank
             FROM managed_task_work_runs WHERE status <> 'running'
           ) WHERE sample_rank > 200
         )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_tasks(pool: &Pool<Sqlite>) -> Result<()> {
    let now = format_utc_iso_millis(Utc::now());
    for (key, title, description, mode, manual) in MANAGED_TASKS {
        sqlx::query("INSERT INTO managed_tasks (task_key,title,description,trigger_mode,enabled,is_manual,updated_at) VALUES (?,?,?,?,?,?,?) ON CONFLICT(task_key) DO UPDATE SET title=excluded.title, description=excluded.description, trigger_mode=excluded.trigger_mode, is_manual=excluded.is_manual")
            .bind(key).bind(title).bind(description).bind(mode).bind(task_enabled_by_default(key, *manual) as i64).bind(*manual as i64).bind(&now).execute(pool).await?;
    }
    for key in STARTUP_BACKFILL_TASKS {
        let (title, description) = startup_backfill_task_metadata(key);
        let task_key = format!("startup_backfill.{key}");
        sqlx::query("INSERT INTO managed_tasks (task_key,title,description,trigger_mode,enabled,is_manual,updated_at) VALUES (?,?,?,?,?,?,?) ON CONFLICT(task_key) DO UPDATE SET title=excluded.title, description=excluded.description, trigger_mode=excluded.trigger_mode, is_manual=excluded.is_manual")
            .bind(&task_key).bind(title).bind(description).bind("event").bind(task_enabled_by_default(&task_key, false) as i64).bind(0_i64).bind(&now).execute(pool).await?;
    }
    Ok(())
}

static GLOBAL: OnceCell<std::sync::Arc<MaintenanceStore>> = OnceCell::const_new();

pub(crate) fn set_global(store: std::sync::Arc<MaintenanceStore>) {
    let _ = GLOBAL.set(store);
}

pub(crate) fn global() -> Option<&'static std::sync::Arc<MaintenanceStore>> {
    GLOBAL.get()
}

pub(crate) async fn legacy_worker_should_skip(task_key: &str) -> bool {
    let Some(store) = global() else {
        return false;
    };
    let Some((enabled, interval_secs, cron_expr, next_catchup_at)) =
        sqlx::query_as::<_, (bool, Option<i64>, Option<String>, Option<String>)>(
            "SELECT enabled,interval_secs,cron_expr,next_catchup_at FROM managed_tasks WHERE task_key=?",
        )
        .bind(task_key)
        .fetch_optional(&store.pool)
        .await
        .ok()
        .flatten()
    else {
        return false;
    };
    !enabled
        || next_catchup_at.is_some()
        || interval_secs.is_some()
        || cron_expr
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

impl MaintenanceStore {
    pub(crate) fn from_pool(pool: Pool<Sqlite>) -> Self {
        Self {
            pool,
            prompt_cache_materialization_control: Arc::new(
                PromptCacheMaterializationControl::default(),
            ),
        }
    }

    #[cfg(test)]
    pub(crate) async fn initialize_schema_for_test(&self) -> Result<()> {
        ensure_schema(&self.pool).await?;
        seed_tasks(&self.pool).await
    }

    pub(crate) async fn initialize_prompt_cache_materialization_control(
        &self,
        task_key: &str,
        task_name: &str,
    ) -> Result<PromptCacheMaterializationControlSnapshot> {
        if let Some(snapshot) = self.prompt_cache_materialization_control.snapshot() {
            return Ok(snapshot);
        }
        let mut transaction = self.pool.begin().await?;
        let enabled =
            sqlx::query_scalar::<_, bool>("SELECT enabled FROM managed_tasks WHERE task_key=?")
                .bind(task_key)
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or_else(|| anyhow!("managed prompt-cache materialization task not found"))?;
        let disabled_until = format_utc_iso_millis(Utc::now() + ChronoDuration::days(3650));
        sqlx::query(
            "INSERT OR IGNORE INTO startup_backfill_progress (
                task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,last_finished_at,
                last_scanned,last_updated,last_status,suspension_reason,next_probe_at,wake_generation,enabled
             ) VALUES (?,0,?,0,NULL,NULL,0,0,'idle',?,NULL,0,?)",
        )
        .bind(task_name)
        .bind((!enabled).then_some(disabled_until.as_str()))
        .bind((!enabled).then_some("operator_disabled"))
        .bind(enabled as i64)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE startup_backfill_progress
             SET enabled=?,
                 next_run_after=CASE
                     WHEN ?=0 THEN ?
                     WHEN suspension_reason='operator_disabled' THEN NULL
                     ELSE next_run_after
                 END,
                 suspension_reason=CASE
                     WHEN ?=0 THEN 'operator_disabled'
                     WHEN suspension_reason='operator_disabled' THEN NULL
                     ELSE suspension_reason
                 END,
                 next_probe_at=CASE
                     WHEN ?=0 OR suspension_reason='operator_disabled' THEN NULL
                     ELSE next_probe_at
                 END,
                 wake_generation=CASE
                     WHEN enabled != ? THEN wake_generation + 1
                     ELSE wake_generation
                 END
             WHERE task_name=? AND enabled != ?",
        )
        .bind(enabled as i64)
        .bind(enabled as i64)
        .bind(&disabled_until)
        .bind(enabled as i64)
        .bind(enabled as i64)
        .bind(enabled as i64)
        .bind(task_name)
        .bind(enabled as i64)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(self
            .prompt_cache_materialization_control
            .initialize(enabled))
    }

    pub(crate) async fn update_prompt_cache_materialization_control(
        &self,
        task_key: &str,
        task_name: &str,
        enabled: bool,
    ) -> Result<Option<PromptCacheMaterializationControlUpdate>> {
        let _update_guard = self
            .prompt_cache_materialization_control
            .update_lock
            .lock()
            .await;
        let mut transaction = self.pool.begin().await?;
        let Some((current_enabled, interval_secs, cron_expr, is_manual)) = sqlx::query_as::<
            _,
            (bool, Option<i64>, Option<String>, bool),
        >(
            "SELECT enabled,interval_secs,cron_expr,is_manual FROM managed_tasks WHERE task_key=?",
        )
        .bind(task_key)
        .fetch_optional(&mut *transaction)
        .await?
        else {
            transaction.commit().await?;
            return Ok(None);
        };
        if is_manual {
            return Err(anyhow!(
                "prompt-cache materialization cannot be a manual task"
            ));
        }
        let changed = current_enabled != enabled;
        let next_trigger_at = if enabled {
            next_trigger_at(interval_secs, cron_expr.as_deref())
        } else {
            None
        };
        sqlx::query(
            "UPDATE managed_tasks SET enabled=?,next_trigger_at=?,updated_at=? WHERE task_key=?",
        )
        .bind(enabled as i64)
        .bind(next_trigger_at)
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(task_key)
        .execute(&mut *transaction)
        .await?;

        let disabled_until = format_utc_iso_millis(Utc::now() + ChronoDuration::days(3650));
        sqlx::query(
            "INSERT OR IGNORE INTO startup_backfill_progress (
                task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,last_finished_at,
                last_scanned,last_updated,last_status,suspension_reason,next_probe_at,wake_generation,enabled
             ) VALUES (?,0,?,0,NULL,NULL,0,0,'idle',?,NULL,0,?)",
        )
        .bind(task_name)
        .bind((!current_enabled).then_some(disabled_until.as_str()))
        .bind((!current_enabled).then_some("operator_disabled"))
        .bind(current_enabled as i64)
        .execute(&mut *transaction)
        .await?;
        if changed {
            sqlx::query(
                "UPDATE startup_backfill_progress
                 SET enabled=?,
                     next_run_after=?,
                     suspension_reason=?,
                     next_probe_at=NULL,
                     wake_generation=wake_generation + 1
                 WHERE task_name=?",
            )
            .bind(enabled as i64)
            .bind((!enabled).then_some(disabled_until.as_str()))
            .bind((!enabled).then_some("operator_disabled"))
            .bind(task_name)
            .execute(&mut *transaction)
            .await?;
        } else {
            sqlx::query(
                "UPDATE startup_backfill_progress SET enabled=? WHERE task_name=? AND enabled != ?",
            )
            .bind(enabled as i64)
            .bind(task_name)
            .bind(enabled as i64)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        let snapshot = self
            .prompt_cache_materialization_control
            .publish_committed(enabled);
        Ok(Some(PromptCacheMaterializationControlUpdate {
            changed,
            snapshot,
        }))
    }

    pub(crate) async fn start_timeline_session(
        &self,
        session_id: &str,
        started_at: &str,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO maintenance_metadata(key,value,updated_at) VALUES('task_timeline_revision','0',?)")
            .bind(started_at)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE maintenance_metadata SET value=CAST(value AS INTEGER)+1,updated_at=? WHERE key='task_timeline_revision'")
            .bind(started_at)
            .execute(&mut *transaction)
            .await?;
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT CAST(value AS INTEGER) FROM maintenance_metadata WHERE key='task_timeline_revision'",
        )
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE task_timeline_segments SET status='interrupted',revision=? WHERE status IN ('running','waiting')",
        )
        .bind(revision)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE task_timeline_coverage SET ended_at=last_seen_at WHERE ended_at IS NULL",
        )
        .execute(&mut *transaction)
        .await?;
        let running_samples =
            sqlx::query_as::<_, (String, String, Option<i64>, String, i64, String)>(
                "SELECT execution_uid,task_key,managed_run_id,attempted_at,sequence,sample_json
             FROM managed_task_work_runs WHERE status='running'",
            )
            .fetch_all(&mut *transaction)
            .await?;
        for (execution_uid, task_key, managed_run_id, attempted_at, stored_sequence, sample_json) in
            running_samples
        {
            let parsed = serde_json::from_str::<TaskWorkloadSample>(&sample_json);
            let malformed = parsed.is_err();
            let mut sample = parsed.unwrap_or_else(|_| TaskWorkloadSample {
                sample_id: format!("{execution_uid}:{task_key}"),
                execution_uid: execution_uid.clone(),
                managed_run_id,
                task_key: task_key.clone(),
                trigger_kind: "unknown".to_string(),
                attempted_at: attempted_at.clone(),
                actual_started_at: None,
                finished_at: None,
                duration_ms: None,
                status: "unknown".to_string(),
                reason: None,
                sequence: 0,
                pending: None,
                discovered: None,
                processed: None,
                subset_relation: "unknown".to_string(),
            });
            let sample_sequence = sample.sequence.min(i64::MAX as u64) as i64;
            let sequence = stored_sequence.max(sample_sequence).saturating_add(1);
            sample.sample_id = format!("{execution_uid}:{task_key}");
            sample.execution_uid = execution_uid.clone();
            sample.managed_run_id = managed_run_id;
            sample.task_key = task_key.clone();
            sample.attempted_at = attempted_at;
            sample.status = "unknown".to_string();
            sample.reason = Some(if malformed {
                "服务重启时发现工作量记录损坏，计数未知".to_string()
            } else {
                "服务重启前运行未确认结束".to_string()
            });
            sample.sequence = sequence as u64;
            sqlx::query(
                "UPDATE managed_task_work_runs
                 SET status='unknown',sequence=?,sample_json=?,updated_at=?
                 WHERE execution_uid=? AND task_key=? AND status='running'",
            )
            .bind(sequence)
            .bind(serde_json::to_string(&sample)?)
            .bind(started_at)
            .bind(&execution_uid)
            .bind(&task_key)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "INSERT INTO task_timeline_coverage(session_id,started_at,last_seen_at,dropped_events) VALUES(?,?,?,0) ON CONFLICT(session_id) DO UPDATE SET started_at=excluded.started_at,last_seen_at=excluded.last_seen_at,ended_at=NULL,dropped_events=0",
        )
        .bind(session_id)
        .bind(started_at)
        .bind(started_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn write_timeline_batch(
        &self,
        session_id: &str,
        events: &[crate::task_timeline::TimelineEvent],
        dropped_events: u64,
        observed_at: &str,
        heartbeat: bool,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO maintenance_metadata(key,value,updated_at) VALUES('task_timeline_revision','0',?)")
            .bind(observed_at)
            .execute(&mut *transaction)
            .await?;
        let prior_dropped = sqlx::query_scalar::<_, i64>(
            "SELECT dropped_events FROM task_timeline_coverage WHERE session_id=?",
        )
        .bind(session_id)
        .fetch_optional(&mut *transaction)
        .await?
        .unwrap_or(0);
        let dropped_events = dropped_events.min(i64::MAX as u64) as i64;
        let revision_increments =
            (!events.is_empty() || heartbeat || dropped_events > prior_dropped) as i64;
        if revision_increments > 0 {
            sqlx::query("UPDATE maintenance_metadata SET value=CAST(value AS INTEGER)+?,updated_at=? WHERE key='task_timeline_revision'")
                .bind(revision_increments)
                .bind(observed_at)
                .execute(&mut *transaction)
                .await?;
        }
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT CAST(value AS INTEGER) FROM maintenance_metadata WHERE key='task_timeline_revision'",
        )
        .fetch_one(&mut *transaction)
        .await?;
        for event in events {
            match event {
                crate::task_timeline::TimelineEvent::ExecutionStarted {
                    id,
                    task_key,
                    title,
                    trigger_kind,
                    execution_class,
                    started_at,
                    managed_run_id,
                } => {
                    sqlx::query("INSERT OR IGNORE INTO task_timeline_segments(segment_id,session_id,kind,task_key,title,started_at,last_observed_at,status,trigger_kind,execution_class,managed_run_id,revision) VALUES(?,?,'execution',?,?,?,?, 'running',?,?,?,?)")
                        .bind(id).bind(session_id).bind(task_key).bind(title).bind(started_at)
                        .bind(started_at).bind(trigger_kind).bind(execution_class)
                        .bind(managed_run_id).bind(revision)
                        .execute(&mut *transaction).await?;
                    if let Some(run_id) = managed_run_id {
                        sqlx::query("UPDATE managed_task_runs SET execution_uid=?,actual_started_at=? WHERE id=?")
                            .bind(id).bind(started_at).bind(run_id)
                            .execute(&mut *transaction).await?;
                    }
                }
                crate::task_timeline::TimelineEvent::ExecutionFinished {
                    id,
                    finished_at,
                    duration_ms,
                    status,
                } => {
                    sqlx::query("UPDATE task_timeline_segments SET finished_at=?,last_observed_at=?,duration_ms=?,status=?,revision=? WHERE segment_id=? AND kind='execution'")
                        .bind(finished_at).bind(finished_at).bind((*duration_ms).min(i64::MAX as u64) as i64)
                        .bind(status).bind(revision).bind(id)
                        .execute(&mut *transaction).await?;
                    sqlx::query("UPDATE managed_task_runs SET actual_finished_at=?,actual_duration_ms=? WHERE execution_uid=?")
                        .bind(finished_at).bind((*duration_ms).min(i64::MAX as u64) as i64).bind(id)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::ExecutionUnknown {
                    id,
                    last_observed_at,
                } => {
                    sqlx::query("UPDATE task_timeline_segments SET finished_at=NULL,last_observed_at=?,duration_ms=NULL,status='unknown',revision=? WHERE segment_id=? AND kind='execution'")
                        .bind(last_observed_at).bind(revision).bind(id)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::ExecutionChildChanged {
                    id,
                    task_key,
                    title,
                } => {
                    sqlx::query("UPDATE task_timeline_segments SET active_child_task_key=?,active_child_title=?,revision=? WHERE segment_id=? AND kind='execution' AND status='running'")
                        .bind(task_key).bind(title).bind(revision).bind(id)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::CoverageGap {
                    id,
                    started_at,
                    finished_at,
                    reason,
                } => {
                    sqlx::query("INSERT INTO task_timeline_segments(segment_id,session_id,kind,task_key,title,started_at,last_observed_at,finished_at,status,reason,revision) VALUES(?,?,'coverage_gap','__timeline__','观测缺口',?,?,?,'unknown',?,?) ON CONFLICT(segment_id) DO UPDATE SET last_observed_at=excluded.last_observed_at,finished_at=excluded.finished_at,reason=excluded.reason,revision=excluded.revision")
                        .bind(id).bind(session_id).bind(started_at).bind(finished_at)
                        .bind(finished_at).bind(reason).bind(revision)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::DeferralStarted {
                    id,
                    task_key,
                    reason,
                    retry_at,
                    started_at,
                } => {
                    let title = task_title_for_observation(task_key);
                    sqlx::query("INSERT OR IGNORE INTO task_timeline_segments(segment_id,session_id,kind,task_key,title,started_at,last_observed_at,status,reason,retry_at,revision) VALUES(?,?,'deferral',?,?,?,?, 'waiting',?,?,?)")
                        .bind(id).bind(session_id).bind(task_key).bind(title).bind(started_at)
                        .bind(started_at).bind(reason).bind(retry_at).bind(revision)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::DeferralFinished { id, finished_at } => {
                    sqlx::query("UPDATE task_timeline_segments SET finished_at=?,last_observed_at=?,status='released',revision=? WHERE segment_id=? AND kind='deferral'")
                        .bind(finished_at).bind(finished_at).bind(revision).bind(id)
                        .execute(&mut *transaction).await?;
                }
                crate::task_timeline::TimelineEvent::WorkloadSampleChanged { sample } => {
                    let sample_json = serde_json::to_string(sample)?;
                    sqlx::query(
                        "INSERT INTO managed_task_work_runs
                         (execution_uid,task_key,managed_run_id,attempted_at,sequence,status,sample_json,updated_at)
                         VALUES(?,?,?,?,?,?,?,?)
                         ON CONFLICT(execution_uid,task_key) DO UPDATE SET
                           managed_run_id=excluded.managed_run_id,
                           attempted_at=excluded.attempted_at,
                           sequence=excluded.sequence,
                           status=excluded.status,
                           sample_json=excluded.sample_json,
                           updated_at=excluded.updated_at
                         WHERE excluded.sequence >= managed_task_work_runs.sequence",
                    )
                    .bind(&sample.execution_uid)
                    .bind(&sample.task_key)
                    .bind(sample.managed_run_id)
                    .bind(&sample.attempted_at)
                    .bind(sample.sequence.min(i64::MAX as u64) as i64)
                    .bind(&sample.status)
                    .bind(sample_json)
                    .bind(observed_at)
                    .execute(&mut *transaction)
                    .await?;
                }
            }
        }
        if heartbeat || !events.is_empty() || dropped_events > prior_dropped {
            sqlx::query("UPDATE task_timeline_coverage SET last_seen_at=?,dropped_events=MAX(dropped_events,?) WHERE session_id=?")
                .bind(observed_at).bind(dropped_events).bind(session_id)
                .execute(&mut *transaction).await?;
        }
        if heartbeat {
            sqlx::query("UPDATE task_timeline_segments SET last_observed_at=?,revision=? WHERE session_id=? AND status IN ('running','waiting')")
                .bind(observed_at).bind(revision).bind(session_id)
                .execute(&mut *transaction).await?;
        }
        let cutoff = format_utc_iso_millis(
            Utc::now() - ChronoDuration::hours(TASK_TIMELINE_RETENTION_HOURS),
        );
        sqlx::query("DELETE FROM task_timeline_segments WHERE status NOT IN ('running','waiting') AND COALESCE(finished_at,last_observed_at) < ?")
            .bind(&cutoff)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "DELETE FROM task_timeline_coverage WHERE ended_at IS NOT NULL AND ended_at < ?",
        )
        .bind(cutoff)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn finish_timeline_session(
        &self,
        session_id: &str,
        ended_at: &str,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE task_timeline_segments SET status='interrupted' WHERE session_id=? AND status IN ('running','waiting')")
            .bind(session_id).execute(&mut *transaction).await?;
        sqlx::query(
            "UPDATE task_timeline_coverage SET ended_at=?,last_seen_at=? WHERE session_id=?",
        )
        .bind(ended_at)
        .bind(ended_at)
        .bind(session_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn timeline_revision(&self) -> Result<i64> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT CAST(value AS INTEGER) FROM maintenance_metadata WHERE key='task_timeline_revision'",
        )
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0))
    }

    pub(crate) async fn list_timeline_segments(
        &self,
        from: &str,
        to: &str,
        watermark: i64,
        after_revision: Option<i64>,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<TimelineSegment>> {
        let rows = sqlx::query_as::<_, TimelineSegment>("SELECT segment_id,kind,task_key,title,started_at,last_observed_at,finished_at,duration_ms,status,trigger_kind,execution_class,reason,retry_at,active_child_task_key,active_child_title,managed_run_id,session_id,revision FROM task_timeline_segments WHERE started_at<=? AND COALESCE(finished_at,last_observed_at)>=? AND revision<=? AND (? IS NULL OR revision>?) ORDER BY started_at,segment_id LIMIT ? OFFSET ?")
            .bind(to).bind(from).bind(watermark).bind(after_revision).bind(after_revision)
            .bind(limit.min(500) as i64).bind(offset.min(i64::MAX as u64) as i64)
            .fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub(crate) async fn list_timeline_coverage(&self) -> Result<Vec<TimelineCoverage>> {
        Ok(sqlx::query_as::<_, TimelineCoverage>("SELECT session_id,started_at,last_seen_at,ended_at,dropped_events FROM task_timeline_coverage ORDER BY started_at DESC LIMIT 50")
            .fetch_all(&self.pool).await?)
    }

    pub(crate) async fn list_queued_runs(&self) -> Result<Vec<QueuedTaskRun>> {
        let rows = sqlx::query_as::<_, (i64, String, String, String, String)>("SELECT r.id,r.task_key,COALESCE(t.title,r.task_key),r.trigger_kind,r.started_at FROM managed_task_runs r LEFT JOIN managed_tasks t ON t.task_key=r.task_key WHERE r.status='requested' ORDER BY r.id LIMIT 500")
            .fetch_all(&self.pool).await?;
        let now = Utc::now();
        Ok(rows
            .into_iter()
            .enumerate()
            .map(
                |(index, (run_id, task_key, title, trigger_kind, requested_at))| {
                    let waiting_ms = chrono::DateTime::parse_from_rfc3339(&requested_at)
                        .map(|started| {
                            (now - started.with_timezone(&Utc))
                                .num_milliseconds()
                                .max(0)
                        })
                        .unwrap_or(0);
                    QueuedTaskRun {
                        run_id,
                        task_key,
                        title,
                        trigger_kind,
                        requested_at,
                        waiting_ms,
                        position: index as i64 + 1,
                    }
                },
            )
            .collect())
    }

    pub(crate) async fn list_current_task_deferrals(&self) -> Result<Vec<TaskAdmissionWait>> {
        let freshness_cutoff = format_utc_iso_millis(Utc::now() - ChronoDuration::seconds(60));
        let session_id = sqlx::query_scalar::<_, String>(
            "SELECT session_id FROM task_timeline_coverage WHERE ended_at IS NULL AND last_seen_at>=? ORDER BY started_at DESC LIMIT 1",
        )
        .bind(&freshness_cutoff)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("task timeline observation is unavailable"))?;
        let rows = sqlx::query_as::<_, (String, String, String, String, String, Option<String>)>("SELECT segment_id,task_key,title,reason,started_at,retry_at FROM task_timeline_segments WHERE kind='deferral' AND status='waiting' AND session_id=? AND last_observed_at>=? ORDER BY started_at,segment_id")
            .bind(session_id)
            .bind(&freshness_cutoff)
            .fetch_all(&self.pool).await?;
        let now = Utc::now();
        Ok(rows
            .into_iter()
            .map(|(id, task_key, title, reason, started_at, retry_at)| {
                let waiting_ms = chrono::DateTime::parse_from_rfc3339(&started_at)
                    .map(|started| {
                        (now - started.with_timezone(&Utc))
                            .num_milliseconds()
                            .max(0)
                    })
                    .unwrap_or(0);
                TaskAdmissionWait {
                    id,
                    task_key,
                    title,
                    reason,
                    started_at,
                    waiting_ms,
                    retry_at,
                }
            })
            .collect())
    }

    pub(crate) async fn claim_requested_run(
        &self,
    ) -> Result<Option<(i64, String, String, String)>> {
        let mut transaction = self.pool.begin().await?;
        let finished_at = format_utc_iso_millis(Utc::now());
        sqlx::query(
            "UPDATE managed_task_runs
             SET status='failed', finished_at=?, duration_ms=0,
                 summary='任务已停用，未执行', error_detail=NULL
             WHERE status='requested'
               AND task_key IN (
                   SELECT task_key FROM managed_tasks WHERE enabled=0 AND is_manual=0
               )",
        )
        .bind(&finished_at)
        .execute(&mut *transaction)
        .await?;
        let claimed = sqlx::query_as::<_, (i64, String, String, String)>(
            "UPDATE managed_task_runs
             SET status='running'
             WHERE id = (
                 SELECT id FROM managed_task_runs
                 WHERE status='requested'
                   AND task_key IN (
                       SELECT task_key FROM managed_tasks WHERE enabled!=0 OR is_manual!=0
                   )
                 ORDER BY id
                 LIMIT 1
             )
             AND status='requested'
             RETURNING id,task_key,started_at,trigger_kind",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((id, task_key, started_at, trigger_kind)) = claimed else {
            transaction.commit().await?;
            return Ok(None);
        };
        transaction.commit().await?;
        Ok(Some((id, task_key, started_at, trigger_kind)))
    }

    pub(crate) async fn recover_incomplete_runs(&self) -> Result<u64> {
        let finished_at = format_utc_iso_millis(Utc::now());
        let result = sqlx::query(
            "UPDATE managed_task_runs
             SET status='failed', finished_at=?, duration_ms=0,
                 summary=COALESCE(summary, ?),
                 error_detail=COALESCE(error_detail, ?)
             WHERE status='running'",
        )
        .bind(&finished_at)
        .bind("服务重启前运行未完成，已标记为失败")
        .bind("服务重启时回收未完成运行")
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub(crate) async fn validate_schedule(
        interval_secs: Option<i64>,
        cron_expr: Option<&str>,
    ) -> Result<()> {
        if interval_secs.is_some()
            && cron_expr
                .map(str::trim)
                .is_some_and(|value| !value.is_empty())
        {
            return Err(anyhow!("interval and cron schedule are mutually exclusive"));
        }
        Self::validate_interval(interval_secs).await?;
        validate_cron_expr(cron_expr)
    }

    pub(crate) async fn request_run(&self, task_key: &str) -> Result<i64> {
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM managed_tasks WHERE task_key=?")
                .bind(task_key)
                .fetch_optional(&self.pool)
                .await?;
        if exists.is_none() {
            return Err(anyhow!("managed task not found"));
        }
        let result = sqlx::query_scalar(
            "INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary,error_detail)
             SELECT task_key,'manual',?,'requested','手动运行请求',NULL
             FROM managed_tasks
             WHERE task_key=? AND (enabled!=0 OR is_manual!=0)
             RETURNING id",
        )
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(task_key)
        .fetch_optional(&self.pool)
        .await;
        let result = match result {
            Ok(Some(run_id)) => Ok(run_id),
            Ok(None) => Err(anyhow!("task is disabled; enable it before run-now")),
            Err(error) => Err(error.into()),
        };
        result.map_err(|error| {
            if error
                .to_string()
                .contains("UNIQUE constraint failed: managed_task_runs.task_key")
            {
                anyhow!("task already has an active run")
            } else {
                error
            }
        })
    }

    pub(crate) async fn migrate_legacy_state(&self, main_pool: &Pool<Sqlite>) -> Result<()> {
        let legacy_runs = sqlx::query_as::<_, (i64, String, String, String, Option<String>, Option<String>, String, Option<String>, Option<i64>)>(
            "SELECT id, task_kind, trigger_kind, status, summary, detail, started_at, finished_at, duration_ms FROM system_task_runs ORDER BY id",
        )
        .fetch_all(main_pool)
        .await?;
        let progress = sqlx::query_as::<_, (String, i64, Option<String>, i64, Option<String>, Option<String>, i64, i64, String, Option<String>, Option<String>, i64, i64)>(
            "SELECT task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,last_finished_at,last_scanned,last_updated,last_status,suspension_reason,next_probe_at,wake_generation,enabled FROM startup_backfill_progress",
        )
        .fetch_all(main_pool)
        .await?;
        let mut transaction = self.pool.begin().await?;
        for (
            id,
            task_kind,
            trigger_kind,
            status,
            summary,
            detail,
            started_at,
            finished_at,
            duration_ms,
        ) in legacy_runs
        {
            let task_key = match task_kind.as_str() {
                "hourly_rollup_bootstrap" => "startup_hourly_rollup_bootstrap",
                "startup_backfill" => "startup_backfill",
                other => other,
            };
            let was_active = matches!(status.as_str(), "running" | "requested");
            let migrated_status = if was_active {
                "failed"
            } else {
                status.as_str()
            };
            let migrated_summary = if was_active {
                Some(
                    summary
                        .as_deref()
                        .unwrap_or("服务重启前运行未完成，已标记为失败"),
                )
            } else {
                summary.as_deref()
            };
            let sanitized_detail = detail.as_deref().map(sanitize_task_detail);
            let migrated_detail = if was_active {
                Some(
                    sanitized_detail
                        .as_deref()
                        .unwrap_or("服务重启前运行未完成"),
                )
            } else {
                sanitized_detail.as_deref()
            };
            let migrated_finished_at = if was_active {
                Some(format_utc_iso_millis(Utc::now()))
            } else {
                finished_at
            };
            let migrated_duration_ms = if was_active { Some(0) } else { duration_ms };
            sqlx::query(
                "INSERT OR IGNORE INTO managed_task_runs (legacy_id,task_key,trigger_kind,started_at,finished_at,duration_ms,status,summary,error_detail) VALUES (?,?,?,?,?,?,?,?,?)",
            )
            .bind(id)
            .bind(task_key)
            .bind(trigger_kind)
            .bind(started_at)
            .bind(migrated_finished_at)
            .bind(migrated_duration_ms)
            .bind(migrated_status)
            .bind(migrated_summary)
            .bind(migrated_detail)
            .execute(&mut *transaction)
            .await?;
        }

        for (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status,
            suspension_reason,
            next_probe_at,
            wake_generation,
            enabled,
        ) in progress
        {
            sqlx::query(
                "INSERT OR IGNORE INTO startup_backfill_progress (task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,last_finished_at,last_scanned,last_updated,last_status,suspension_reason,next_probe_at,wake_generation,enabled) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(&task_name)
            .bind(cursor_id)
            .bind(&next_run_after)
            .bind(zero_update_streak)
            .bind(&last_started_at)
            .bind(&last_finished_at)
            .bind(last_scanned)
            .bind(last_updated)
            .bind(&last_status)
            .bind(&suspension_reason)
            .bind(&next_probe_at)
            .bind(wake_generation)
            .bind(enabled)
            .execute(&mut *transaction)
            .await?;
            let Some(managed_suffix) = managed_startup_backfill_suffix(&task_name) else {
                // Keep versioned or catalog-specific legacy rows without inventing a page.
                continue;
            };
            let task_key = format!("startup_backfill.{managed_suffix}");
            let updated_at = last_finished_at.or(last_started_at).or(next_run_after);
            let freshness = if enabled == 0 { "stale" } else { "fresh" };
            sqlx::query(
                "INSERT OR IGNORE INTO managed_task_progress (task_key,total,completed,phase,checkpoint,updated_at,freshness) VALUES (?,?,?,?,?,?,?)",
            )
            .bind(task_key)
            .bind(Option::<i64>::None)
            .bind(last_updated.max(last_scanned))
            .bind(last_status)
            .bind(cursor_id.to_string())
            .bind(updated_at)
            .bind(freshness)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn apply_initial_task_defaults(&self) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let now = format_utc_iso_millis(Utc::now());
        let mut changed = false;
        let retention_schedule_applied: Option<String> =
            sqlx::query_scalar("SELECT value FROM maintenance_metadata WHERE key=?")
                .bind(RETENTION_DEFAULT_SCHEDULE_MARKER)
                .fetch_optional(&mut *transaction)
                .await?;
        if retention_schedule_applied.is_none() {
            sqlx::query(
                "UPDATE managed_tasks
                 SET interval_secs=?,
                     next_trigger_at=CASE WHEN enabled!=0 THEN COALESCE(next_trigger_at, ?) ELSE NULL END,
                     schedule_source='default',
                     updated_at=?
                 WHERE task_key='retention_archive'
                   AND is_manual=0
                   AND interval_secs IS NULL
                   AND (cron_expr IS NULL OR trim(cron_expr)='')",
            )
            .bind(DEFAULT_RETENTION_INTERVAL_SECS)
            .bind(&now)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
            sqlx::query("INSERT INTO maintenance_metadata (key,value,updated_at) VALUES (?,?,?)")
                .bind(RETENTION_DEFAULT_SCHEDULE_MARKER)
                .bind("applied")
                .bind(&now)
                .execute(&mut *transaction)
                .await?;
            changed = true;
        }
        let defaults_applied: Option<String> =
            sqlx::query_scalar("SELECT value FROM maintenance_metadata WHERE key=?")
                .bind(INITIAL_TASK_DEFAULTS_MARKER)
                .fetch_optional(&mut *transaction)
                .await?;
        let legacy_enablement_reconciled: Option<String> =
            sqlx::query_scalar("SELECT value FROM maintenance_metadata WHERE key=?")
                .bind(LEGACY_BACKFILL_ENABLEMENT_MARKER)
                .fetch_optional(&mut *transaction)
                .await?;
        let prompt_cache_control_origin: Option<String> =
            sqlx::query_scalar("SELECT value FROM maintenance_metadata WHERE key=?")
                .bind(PROMPT_CACHE_CONTROL_ORIGIN_MARKER)
                .fetch_optional(&mut *transaction)
                .await?;
        if defaults_applied.is_some() && legacy_enablement_reconciled.is_some() {
            transaction.commit().await?;
            return Ok(changed);
        }

        // `seed_tasks` applies defaults only when a task row is first created. Do not
        // rewrite existing controls here: an upgrade must preserve operator choices.
        let disabled_until =
            format_utc_iso_millis(Utc::now() + ChronoDuration::days(TASK_DISABLED_UNTIL_DAYS));
        for task_name in [
            crate::STARTUP_BACKFILL_TASK_PROXY_USAGE,
            crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_KEY,
            crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION,
            crate::STARTUP_BACKFILL_TASK_REQUESTED_SERVICE_TIER,
            crate::STARTUP_BACKFILL_TASK_INVOCATION_SERVICE_TIER,
            crate::STARTUP_BACKFILL_TASK_PROXY_COST,
            crate::STARTUP_BACKFILL_TASK_REASONING_EFFORT,
            crate::STARTUP_BACKFILL_TASK_FAILURE_CLASSIFICATION,
            crate::STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_LIVE,
            crate::STARTUP_BACKFILL_TASK_POOL_ATTEMPT_PUBLIC_ID_ARCHIVES,
            crate::STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_LIVE,
            crate::STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES,
            crate::STARTUP_BACKFILL_TASK_POOL_UPSTREAM_NODE_HEALTH_ARCHIVES,
            crate::STARTUP_BACKFILL_TASK_ACCOUNT_ACTIVITY_V2_COVERAGE,
            crate::STARTUP_BACKFILL_TASK_LEGACY_DETAIL_MIRRORS,
            crate::STARTUP_BACKFILL_TASK_HISTORICAL_ROLLUPS,
        ] {
            let like_pattern = format!("{task_name}:%");
            let Some(managed_suffix) = managed_startup_backfill_suffix(task_name) else {
                continue;
            };
            let managed_key = format!("startup_backfill.{managed_suffix}");
            let legacy_is_enablement_source = task_name
                != crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION
                || prompt_cache_control_origin.as_deref() == Some("legacy")
                || prompt_cache_control_origin.is_none();
            if legacy_is_enablement_source {
                // A legacy progress bit may seed a newly created managed control once.
                sqlx::query(
                    "UPDATE managed_tasks
                     SET enabled = COALESCE(
                         (SELECT MAX(enabled) FROM startup_backfill_progress
                          WHERE task_name=? OR task_name LIKE ?),
                         enabled
                     )
                     WHERE task_key=?",
                )
                .bind(task_name)
                .bind(&like_pattern)
                .bind(&managed_key)
                .execute(&mut *transaction)
                .await?;
            }
            sqlx::query(
                "UPDATE startup_backfill_progress
                 SET enabled=COALESCE((SELECT enabled FROM managed_tasks WHERE task_key=?), 0),
                     next_run_after=CASE WHEN COALESCE((SELECT enabled FROM managed_tasks WHERE task_key=?), 0)=0 THEN ? ELSE next_run_after END,
                     suspension_reason=CASE WHEN COALESCE((SELECT enabled FROM managed_tasks WHERE task_key=?), 0)=0 THEN 'operator_disabled' ELSE suspension_reason END,
                     next_probe_at=CASE WHEN COALESCE((SELECT enabled FROM managed_tasks WHERE task_key=?), 0)=0 THEN NULL ELSE next_probe_at END
                 WHERE task_name=? OR task_name LIKE ?",
            )
            .bind(&managed_key)
            .bind(&managed_key)
            .bind(&disabled_until)
            .bind(&managed_key)
            .bind(&managed_key)
            .bind(task_name)
            .bind(like_pattern)
            .execute(&mut *transaction)
            .await?;
        }
        if defaults_applied.is_none() {
            sqlx::query(
                "INSERT OR IGNORE INTO maintenance_metadata (key,value,updated_at) VALUES (?,?,?)",
            )
            .bind(INITIAL_TASK_DEFAULTS_MARKER)
            .bind("applied")
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        if legacy_enablement_reconciled.is_none() {
            sqlx::query(
                "INSERT OR IGNORE INTO maintenance_metadata (key,value,updated_at) VALUES (?,?,?)",
            )
            .bind(LEGACY_BACKFILL_ENABLEMENT_MARKER)
            .bind("applied")
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(true)
    }

    pub(crate) async fn enqueue_due_runs(&self) -> Result<u64> {
        self.enqueue_due_runs_with_retention_enabled(true).await
    }

    pub(crate) async fn enqueue_due_runs_with_retention_enabled(
        &self,
        retention_enabled: bool,
    ) -> Result<u64> {
        let now = Utc::now();
        let now_text = format_utc_iso_millis(now);
        let mut transaction = self.pool.begin().await?;
        let due_tasks = sqlx::query_as::<
            _,
            (
                String,
                Option<i64>,
                Option<String>,
                Option<String>,
                Option<String>,
            ),
        >(
            "SELECT task_key,interval_secs,cron_expr
                    ,next_trigger_at,next_catchup_at
             FROM managed_tasks
             WHERE enabled=1 AND is_manual=0
               AND (? = 1 OR task_key != 'retention_archive')
               AND ((next_trigger_at IS NOT NULL AND next_trigger_at <= ?)
                 OR (next_catchup_at IS NOT NULL AND next_catchup_at <= ?))
             ORDER BY COALESCE(next_catchup_at,next_trigger_at), task_key",
        )
        .bind(retention_enabled)
        .bind(&now_text)
        .bind(&now_text)
        .fetch_all(&mut *transaction)
        .await?;
        let mut enqueued = 0_u64;
        for (task_key, interval_secs, cron_expr, current_trigger_at, current_catchup_at) in
            due_tasks
        {
            let inspection_due = current_trigger_at
                .as_deref()
                .is_some_and(|value| value <= now_text.as_str());
            let catchup_due = current_catchup_at
                .as_deref()
                .is_some_and(|value| value <= now_text.as_str());
            let active_run: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM managed_task_runs
                 WHERE task_key=? AND status IN ('running','requested') LIMIT 1",
            )
            .bind(&task_key)
            .fetch_optional(&mut *transaction)
            .await?;
            if active_run.is_some() {
                continue;
            }
            let next_trigger = if inspection_due {
                next_trigger_at(interval_secs, cron_expr.as_deref())
            } else {
                current_trigger_at.clone()
            };
            let trigger_kind = if catchup_due { "catchup" } else { "schedule" };
            let inserted = sqlx::query(
                "INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary)
                 VALUES (?,?,?,?,?)
                 ON CONFLICT DO NOTHING",
            )
            .bind(&task_key)
            .bind(trigger_kind)
            .bind(&now_text)
            .bind("requested")
            .bind(if catchup_due {
                "积压追赶触发"
            } else {
                "按计划触发"
            })
            .execute(&mut *transaction)
            .await?;
            enqueued += inserted.rows_affected();
            if inserted.rows_affected() > 0 {
                sqlx::query(
                    "UPDATE managed_tasks
                     SET next_trigger_at=?,
                         next_catchup_at=CASE WHEN ? THEN NULL ELSE next_catchup_at END,
                         catchup_reason=CASE WHEN ? THEN NULL ELSE catchup_reason END,
                         updated_at=?
                     WHERE task_key=?",
                )
                .bind(next_trigger)
                .bind(catchup_due)
                .bind(catchup_due)
                .bind(&now_text)
                .bind(task_key)
                .execute(&mut *transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        if enqueued > 0 {
            self.sync_retention_progress_schedule().await?;
        }
        Ok(enqueued)
    }

    pub(crate) async fn cleanup_expired_history_if_due(&self) -> Result<()> {
        let now_ms = Utc::now().timestamp_millis();
        let last_ms = LAST_TASK_HISTORY_CLEANUP_MS.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last_ms) < TASK_HISTORY_CLEANUP_INTERVAL_MS
            || LAST_TASK_HISTORY_CLEANUP_MS
                .compare_exchange(last_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
                .is_err()
        {
            return Ok(());
        }
        let now = Utc::now();
        let error_cutoff =
            format_utc_iso_millis(now - ChronoDuration::days(TASK_ERROR_RETENTION_DAYS));
        let run_cutoff = format_utc_iso_millis(now - ChronoDuration::days(TASK_RUN_RETENTION_DAYS));
        sqlx::query("UPDATE managed_task_runs SET error_detail=NULL WHERE started_at < ?")
            .bind(error_cutoff)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM managed_task_runs WHERE started_at < ?")
            .bind(run_cutoff)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "DELETE FROM managed_task_work_runs
             WHERE rowid IN (
               SELECT rowid FROM (
                 SELECT rowid, ROW_NUMBER() OVER (PARTITION BY task_key ORDER BY attempted_at DESC, execution_uid DESC) AS sample_rank
                 FROM managed_task_work_runs WHERE status <> 'running'
               ) WHERE sample_rank > 200
             )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn begin_run(
        &self,
        task_key: &str,
        started_at: &str,
        trigger_kind: &str,
        summary: Option<&str>,
    ) -> Result<i64> {
        let result = sqlx::query_scalar("INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary) VALUES (?,?,?,?,?) RETURNING id")
            .bind(task_key)
            .bind(trigger_kind)
            .bind(started_at)
            .bind("running")
            .bind(summary)
            .fetch_one(&self.pool)
            .await;
        result.map_err(|error| {
            if error
                .to_string()
                .contains("UNIQUE constraint failed: managed_task_runs.task_key")
            {
                anyhow!("task already has an active run")
            } else {
                error.into()
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn publish_progress(
        &self,
        task_key: &str,
        total: Option<i64>,
        completed: Option<i64>,
        phase: Option<&str>,
        checkpoint: Option<&str>,
        unit: Option<&str>,
        source_scope: Option<&str>,
        wait_reason: Option<&str>,
        next_retry_at: Option<&str>,
        stages: Option<&[TaskStage]>,
    ) -> Result<()> {
        let now = format_utc_iso_millis(Utc::now());
        let stages = stages.map(serde_json::to_string).transpose()?;
        sqlx::query(
            "INSERT INTO managed_task_progress(task_key,total,completed,phase,checkpoint,updated_at,freshness,unit,source_scope,last_progress_at,wait_reason,next_retry_at,next_inspection_at,next_catchup_at,catchup_state,stages)
             VALUES(?,?,?,?,?,?,'fresh',?,?,?,?,?,
                    (SELECT next_trigger_at FROM managed_tasks WHERE task_key=?),
                    (SELECT next_catchup_at FROM managed_tasks WHERE task_key=?),
                    (SELECT CASE WHEN enabled=0 THEN 'disabled' WHEN next_catchup_at IS NOT NULL THEN 'scheduled' ELSE 'idle' END FROM managed_tasks WHERE task_key=?),
                    ?)
             ON CONFLICT(task_key) DO UPDATE SET total=excluded.total,completed=excluded.completed,phase=excluded.phase,checkpoint=excluded.checkpoint,updated_at=excluded.updated_at,freshness='fresh',unit=excluded.unit,source_scope=excluded.source_scope,last_progress_at=excluded.last_progress_at,wait_reason=excluded.wait_reason,next_retry_at=excluded.next_retry_at,next_inspection_at=excluded.next_inspection_at,next_catchup_at=excluded.next_catchup_at,catchup_state=excluded.catchup_state,stages=excluded.stages",
        )
        .bind(task_key)
        .bind(total)
        .bind(completed)
        .bind(phase)
        .bind(checkpoint)
        .bind(&now)
        .bind(unit)
        .bind(source_scope)
        .bind(&now)
        .bind(wait_reason)
        .bind(next_retry_at)
        .bind(task_key)
        .bind(task_key)
        .bind(task_key)
        .bind(stages)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn finish_run(
        &self,
        id: i64,
        status: &str,
        finished_at: &str,
        duration_ms: i64,
        summary: Option<&str>,
        detail: Option<&str>,
    ) -> Result<()> {
        self.finish_run_with_observation(
            id,
            status,
            finished_at,
            duration_ms,
            summary,
            detail,
            None,
            None,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn finish_run_with_observation(
        &self,
        id: i64,
        status: &str,
        finished_at: &str,
        duration_ms: i64,
        summary: Option<&str>,
        detail: Option<&str>,
        completion: Option<&str>,
        core_completion: Option<&str>,
        details: Option<&serde_json::Value>,
    ) -> Result<()> {
        let sanitized = detail.map(sanitize_task_detail);
        let details = details.map(serde_json::to_string).transpose()?;
        let task_key: Option<String> =
            sqlx::query_scalar("SELECT task_key FROM managed_task_runs WHERE id=?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        let result = sqlx::query("UPDATE managed_task_runs SET status=?,finished_at=?,duration_ms=?,summary=COALESCE(?,summary),error_detail=?,completion=?,core_completion=?,details=? WHERE id=?")
            .bind(status)
            .bind(finished_at)
            .bind(duration_ms)
            .bind(summary)
            .bind(sanitized)
            .bind(completion)
            .bind(core_completion)
            .bind(&details)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(anyhow!("managed task run {id} was not found"));
        }
        if task_key.as_deref() == Some("retention_archive") {
            self.update_retention_catchup_after_finish(
                completion,
                details.as_deref(),
                &format_utc_iso_millis(Utc::now()),
            )
            .await?;
            self.sync_retention_progress_schedule().await?;
        }
        Ok(())
    }

    async fn update_retention_catchup_after_finish(
        &self,
        completion: Option<&str>,
        details: Option<&str>,
        now: &str,
    ) -> Result<()> {
        let Some(details) =
            details.and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        else {
            return Ok(());
        };
        let backlog_total = details.get("total").and_then(serde_json::Value::as_i64);
        let completed = details
            .get("invocationRowsArchived")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0)
            .max(0);
        let wait_reason = details
            .get("waitReason")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty());
        self.update_retention_catchup_from_summary(
            completion,
            backlog_total,
            completed as usize,
            wait_reason,
            now,
        )
        .await
    }

    pub(crate) async fn update_retention_catchup_from_summary(
        &self,
        completion: Option<&str>,
        backlog_total: Option<i64>,
        completed: usize,
        wait_reason: Option<&str>,
        now: &str,
    ) -> Result<()> {
        let completed = completed.min(i64::MAX as usize) as i64;
        // Missing observation is not an empty backlog. Requalify after the consumed claim
        // with the same reason-specific backoff, retaining uncertainty in the displayed total.
        let wait_reason =
            wait_reason.or_else(|| backlog_total.is_none().then_some("backlog_unknown"));
        let pending = backlog_total.map(|total| total.saturating_sub(completed));
        if pending.is_some_and(|value| value <= 0) {
            sqlx::query(
                "UPDATE managed_tasks
                 SET next_catchup_at=NULL, catchup_reason=NULL, updated_at=?
                 WHERE task_key='retention_archive' AND enabled!=0",
            )
            .bind(now)
            .execute(&self.pool)
            .await?;
            return Ok(());
        }
        if matches!(completion, Some("failed") | None) {
            return Ok(());
        }
        let delay_secs = match wait_reason {
            Some("retention_work_budget") => 1,
            Some("retention_write_admission") | Some("sqlite_pressure") => 30,
            Some("retention_recovery_failure") | Some("retry_backoff") => 300,
            Some(_) => 15,
            None => 1,
        };
        let next_catchup_at =
            format_utc_iso_millis(Utc::now() + ChronoDuration::seconds(delay_secs));
        sqlx::query(
            "UPDATE managed_tasks
             SET next_catchup_at=CASE
                     WHEN enabled=0 THEN NULL
                     WHEN next_catchup_at IS NULL OR next_catchup_at > ?1 THEN ?1
                     ELSE next_catchup_at
                 END,
                 catchup_reason=CASE WHEN enabled=0 THEN NULL ELSE ?2 END,
                 updated_at=?3
             WHERE task_key='retention_archive'",
        )
        .bind(&next_catchup_at)
        .bind(wait_reason.unwrap_or("backlog_remaining"))
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn sync_retention_progress_schedule(&self) -> Result<()> {
        sqlx::query(
            "INSERT INTO managed_task_progress(
                 task_key,updated_at,freshness,next_inspection_at,next_catchup_at,catchup_state
             )
             SELECT task_key,?1,'fresh',next_trigger_at,next_catchup_at,
                    CASE WHEN enabled=0 THEN 'disabled'
                         WHEN next_catchup_at IS NOT NULL THEN 'scheduled'
                         ELSE 'idle' END
             FROM managed_tasks
             WHERE task_key='retention_archive'
             ON CONFLICT(task_key) DO UPDATE SET
                 updated_at=excluded.updated_at,
                 freshness='fresh',
                 next_inspection_at=excluded.next_inspection_at,
                 next_catchup_at=excluded.next_catchup_at,
                 catchup_state=excluded.catchup_state",
        )
        .bind(format_utc_iso_millis(Utc::now()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub(crate) async fn record_retention_backlog_observation(
        &self,
        observation: RetentionBacklogObservation,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO retention_backlog_hourly_observations(
                 bucket_start,observed_at,invocation_count,max_overdue_seconds,
                 retention_days,cutoff,source_max_invocation_id
             ) VALUES(?,?,?,?,?,?,?)
             ON CONFLICT(bucket_start) DO UPDATE SET
                 observed_at=excluded.observed_at,
                 invocation_count=excluded.invocation_count,
                 max_overdue_seconds=excluded.max_overdue_seconds,
                 retention_days=excluded.retention_days,
                 cutoff=excluded.cutoff,
                 source_max_invocation_id=excluded.source_max_invocation_id
             WHERE excluded.observed_at >= retention_backlog_hourly_observations.observed_at",
        )
        .bind(&observation.bucket_start)
        .bind(&observation.observed_at)
        .bind(observation.invocation_count)
        .bind(observation.max_overdue_seconds)
        .bind(observation.retention_days)
        .bind(&observation.cutoff)
        .bind(observation.source_max_invocation_id)
        .execute(&self.pool)
        .await?;
        let cutoff_bucket = floor_utc_hour(Utc::now()) - ChronoDuration::days(8);
        sqlx::query("DELETE FROM retention_backlog_hourly_observations WHERE bucket_start < ?")
            .bind(format_utc_iso_millis(cutoff_bucket))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn retention_backlog_trend(
        &self,
        now: chrono::DateTime<Utc>,
    ) -> Result<Vec<RetentionBacklogTrendPoint>> {
        let bucket_end = floor_utc_hour(now) + ChronoDuration::hours(1);
        let bucket_start = bucket_end - ChronoDuration::days(7);
        let rows = sqlx::query_as::<_, RetentionBacklogObservationRow>(
            "SELECT bucket_start,observed_at,invocation_count,max_overdue_seconds,
                    retention_days,cutoff,source_max_invocation_id
             FROM retention_backlog_hourly_observations
             WHERE bucket_start >= ?1 AND bucket_start < ?2
             ORDER BY bucket_start",
        )
        .bind(format_utc_iso_millis(bucket_start))
        .bind(format_utc_iso_millis(bucket_end))
        .fetch_all(&self.pool)
        .await?;
        let mut by_bucket = rows
            .into_iter()
            .map(|row| (row.bucket_start.clone(), row))
            .collect::<std::collections::HashMap<_, _>>();
        let mut trend = Vec::with_capacity(7 * 24);
        let mut bucket = bucket_start;
        while bucket < bucket_end {
            let key = format_utc_iso_millis(bucket);
            if let Some(row) = by_bucket.remove(&key) {
                trend.push(RetentionBacklogTrendPoint {
                    bucket_start: row.bucket_start,
                    state: "observed".to_string(),
                    observed_at: Some(row.observed_at),
                    invocation_count: Some(row.invocation_count),
                    max_overdue_seconds: row.max_overdue_seconds,
                    retention_days: Some(row.retention_days),
                    cutoff: Some(row.cutoff),
                    source_max_invocation_id: row.source_max_invocation_id,
                });
            } else {
                trend.push(RetentionBacklogTrendPoint {
                    bucket_start: key,
                    state: "missing".to_string(),
                    observed_at: None,
                    invocation_count: None,
                    max_overdue_seconds: None,
                    retention_days: None,
                    cutoff: None,
                    source_max_invocation_id: None,
                });
            }
            bucket += ChronoDuration::hours(1);
        }
        Ok(trend)
    }

    pub(crate) async fn list_tasks(&self) -> Result<Vec<ManagedTask>> {
        let tasks = sqlx::query_as::<_, ManagedTask>("SELECT task_key,title,description,trigger_mode,enabled,interval_secs,cron_expr,next_trigger_at,next_catchup_at,catchup_reason,is_manual,display_color_light,display_color_dark,schedule_source FROM managed_tasks ORDER BY task_key")
            .fetch_all(&self.pool).await?;
        let runtime_snapshot = crate::task_runtime_observation::task_runtime_snapshot().ok();
        let mut decorated = tasks
            .into_iter()
            .map(decorate_effective_schedule)
            .map(decorate_task)
            .collect::<Vec<_>>();
        let latest_rows = sqlx::query_as::<_, LatestTaskExecutionRow>(
            "SELECT task_key,id,execution_uid,trigger_kind,started_at,actual_started_at,finished_at,
                    actual_finished_at,duration_ms,actual_duration_ms,status,error_detail
             FROM (
                 SELECT task_key,id,execution_uid,trigger_kind,started_at,actual_started_at,finished_at,
                        actual_finished_at,duration_ms,actual_duration_ms,status,error_detail,
                        ROW_NUMBER() OVER (PARTITION BY task_key ORDER BY started_at DESC,id DESC) AS row_number
                 FROM managed_task_runs
                 WHERE status NOT IN ('requested','queued')
             )
             WHERE row_number=1",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            let summary = TaskExecutionSummary {
                execution_uid: row.execution_uid,
                run_id: row.id,
                trigger_kind: row.trigger_kind,
                attempted_at: row.started_at,
                actual_started_at: row.actual_started_at,
                finished_at: row.actual_finished_at.or(row.finished_at),
                // Legacy duration_ms may include dispatcher wait; only the observer-owned
                // field proves the actual execution interval.
                duration_ms: row.actual_duration_ms,
                result: row.status.clone(),
                status: row.status,
                reason: row.error_detail,
            };
            (row.task_key, summary)
        })
        .collect::<std::collections::HashMap<_, _>>();
        for task in &mut decorated {
            task.last_execution = latest_rows.get(&task.task_key).cloned();
            if let (Some(summary), Some(snapshot)) =
                (task.last_execution.as_mut(), runtime_snapshot.as_ref())
                && summary.status == "running"
                && let Some(active) = snapshot
                    .active_runs
                    .iter()
                    .find(|active| active.task_key == task.task_key)
            {
                summary.actual_started_at = Some(active.started_at.clone());
                summary.duration_ms = Some(active.elapsed_ms.min(i64::MAX as u64) as i64);
            }
            task.execution_observation = if task.last_execution.is_some() {
                "observed".to_string()
            } else {
                "no recorded attempts".to_string()
            };
        }
        Ok(decorated)
    }

    pub(crate) async fn workload_window(
        &self,
        task_key: &str,
        limit: usize,
    ) -> Result<Option<TaskWorkloadTrend>> {
        let task = sqlx::query_as::<_, ManagedTask>("SELECT task_key,title,description,trigger_mode,enabled,interval_secs,cron_expr,next_trigger_at,next_catchup_at,catchup_reason,is_manual,display_color_light,display_color_dark,schedule_source FROM managed_tasks WHERE task_key=?")
            .bind(task_key)
            .fetch_optional(&self.pool)
            .await?
            .map(decorate_effective_schedule)
            .map(decorate_task);
        let Some(task) = task else {
            return Ok(None);
        };
        Ok(Some(
            self.workload_trend(&task, limit.clamp(1, 200), Some(24))
                .await?,
        ))
    }

    pub(crate) async fn detail(&self, task_key: &str) -> Result<Option<ManagedTaskDetail>> {
        let task = sqlx::query_as::<_, ManagedTask>("SELECT task_key,title,description,trigger_mode,enabled,interval_secs,cron_expr,next_trigger_at,next_catchup_at,catchup_reason,is_manual,display_color_light,display_color_dark,schedule_source FROM managed_tasks WHERE task_key=?")
        .bind(task_key).fetch_optional(&self.pool).await?;
        let Some(task) = task.map(decorate_effective_schedule).map(decorate_task) else {
            return Ok(None);
        };
        let progress = sqlx::query_as::<_, TaskProgressRow>("SELECT total,completed,phase,checkpoint,eta_seconds,updated_at,freshness,unit,source_scope,last_progress_at,wait_reason,next_retry_at,next_inspection_at,next_catchup_at,catchup_state,stages FROM managed_task_progress WHERE task_key=?")
            .bind(task_key).fetch_optional(&self.pool).await?.map(task_progress_from_row);
        let recent_runs = sqlx::query_as::<_, TaskRunRow>("SELECT id,trigger_kind,started_at,finished_at,duration_ms,status,summary,processed_count,updated_count,error_detail,completion,core_completion,details FROM managed_task_runs WHERE task_key=? ORDER BY started_at DESC LIMIT 100")
            .bind(task_key).fetch_all(&self.pool).await?.into_iter().map(task_run_from_row).collect();
        let retention_backlog_trend = if task_key == "retention_archive" {
            Some(self.retention_backlog_trend(Utc::now()).await?)
        } else {
            None
        };
        let workload_trend = self.workload_trend(&task, 100, None).await?;
        Ok(Some(ManagedTaskDetail {
            task,
            progress,
            recent_runs,
            retention_backlog_trend,
            workload_trend,
        }))
    }

    async fn workload_trend(
        &self,
        task: &ManagedTask,
        limit: usize,
        window_hours: Option<i64>,
    ) -> Result<TaskWorkloadTrend> {
        let task_key = &task.task_key;
        let window_end = Utc::now();
        let window_start = window_end - ChronoDuration::hours(window_hours.unwrap_or(24));
        let window_start_text = format_utc_iso_millis(window_start);
        let window_end_text = format_utc_iso_millis(window_end);
        let stored_running = if window_hours.is_some() {
            sqlx::query_as::<_, StoredTaskWorkloadRun>(
                "SELECT execution_uid,managed_run_id,attempted_at,sequence,status,sample_json
                 FROM managed_task_work_runs
                 WHERE task_key=? AND status='running' AND attempted_at>=? AND attempted_at<=?
                 ORDER BY attempted_at DESC,execution_uid DESC LIMIT 1",
            )
            .bind(task_key)
            .bind(&window_start_text)
            .bind(&window_end_text)
            .fetch_optional(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, StoredTaskWorkloadRun>(
                "SELECT execution_uid,managed_run_id,attempted_at,sequence,status,sample_json
                 FROM managed_task_work_runs
                 WHERE task_key=? AND status='running'
                 ORDER BY attempted_at DESC,execution_uid DESC LIMIT 1",
            )
            .bind(task_key)
            .fetch_optional(&self.pool)
            .await?
        };
        let stored_recent = if window_hours.is_some() {
            sqlx::query_as::<_, StoredTaskWorkloadRun>(
                "SELECT execution_uid,managed_run_id,attempted_at,sequence,status,sample_json
                 FROM managed_task_work_runs
                 WHERE task_key=? AND status<>'running' AND attempted_at>=? AND attempted_at<=?
                 ORDER BY attempted_at DESC,execution_uid DESC LIMIT ?",
            )
            .bind(task_key)
            .bind(&window_start_text)
            .bind(&window_end_text)
            .bind(limit.saturating_add(1) as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, StoredTaskWorkloadRun>(
                "SELECT execution_uid,managed_run_id,attempted_at,sequence,status,sample_json
                 FROM managed_task_work_runs
                 WHERE task_key=? AND status<>'running'
                 ORDER BY attempted_at DESC,execution_uid DESC LIMIT ?",
            )
            .bind(task_key)
            .bind(limit.saturating_add(1) as i64)
            .fetch_all(&self.pool)
            .await?
        };
        let stored = stored_running.into_iter().chain(stored_recent);
        let mut samples = stored
            .map(|row| restore_stored_task_workload_sample(task_key, row))
            .collect::<Vec<_>>();
        let mut known = samples
            .iter()
            .map(|sample| sample.sample_id.clone())
            .collect::<std::collections::HashSet<_>>();
        for run in recent_runs_for_workload_compatibility(
            &self.pool,
            task_key,
            window_hours
                .is_some()
                .then_some((&window_start_text, &window_end_text)),
        )
        .await?
        {
            let execution_uid = run
                .execution_uid
                .clone()
                .unwrap_or_else(|| format!("legacy:{}", run.id));
            let sample_id = format!("{execution_uid}:{task_key}");
            if known.insert(sample_id.clone()) {
                samples.push(TaskWorkloadSample {
                    sample_id,
                    execution_uid,
                    managed_run_id: Some(run.id),
                    task_key: task_key.to_string(),
                    trigger_kind: run.trigger_kind,
                    attempted_at: run.started_at,
                    actual_started_at: run.actual_started_at,
                    finished_at: run.actual_finished_at.or(run.finished_at),
                    // Do not relabel a legacy queue-inclusive duration as actual work time.
                    duration_ms: run.actual_duration_ms,
                    status: run.status,
                    reason: run.error_detail,
                    sequence: 0,
                    pending: None,
                    discovered: None,
                    processed: None,
                    subset_relation: "unknown".to_string(),
                });
            }
        }
        if let Some(active) = crate::task_runtime_observation::workload_sample(task_key) {
            if known.insert(active.sample_id.clone()) {
                samples.push(active);
            } else if let Some(existing) = samples
                .iter_mut()
                .find(|sample| sample.sample_id == active.sample_id)
            {
                *existing = active;
            }
        }
        if window_hours.is_some() {
            samples.retain(|sample| {
                crate::stats::parse_to_utc_datetime(&sample.attempted_at).is_some_and(
                    |attempted_at| attempted_at >= window_start && attempted_at <= window_end,
                )
            });
        }
        samples.sort_by(|left, right| {
            (right.status == "running")
                .cmp(&(left.status == "running"))
                .then_with(|| right.attempted_at.cmp(&left.attempted_at))
                .then_with(|| right.sample_id.cmp(&left.sample_id))
        });
        let truncated = samples.len() > limit;
        samples.truncate(limit);
        samples.sort_by(|left, right| {
            left.attempted_at
                .cmp(&right.attempted_at)
                .then_with(|| left.sample_id.cmp(&right.sample_id))
        });
        let coverage_floor = samples
            .first()
            .map(|sample| sample.attempted_at.clone())
            .unwrap_or_else(|| format_utc_iso_millis(Utc::now() - ChronoDuration::hours(24)));
        let mut coverage_gaps = sqlx::query_as::<_, TaskWorkloadCoverageGapRow>(
            "SELECT segment_id,started_at,finished_at,reason FROM task_timeline_segments
             WHERE (task_key=? OR task_key='__timeline__')
               AND kind='coverage_gap' AND last_observed_at>=?
             ORDER BY started_at DESC LIMIT 100",
        )
        .bind(task_key)
        .bind(coverage_floor)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| TaskWorkloadCoverageGap {
            id: row.segment_id,
            started_at: row.started_at,
            finished_at: row.finished_at,
            reason: row.reason,
        })
        .collect::<Vec<_>>();
        coverage_gaps.sort_by(|left, right| left.started_at.cmp(&right.started_at));
        let latest_capture_gap = coverage_gaps
            .iter()
            .filter_map(|gap| {
                gap.finished_at
                    .as_deref()
                    .or(Some(gap.started_at.as_str()))
                    .and_then(crate::stats::parse_to_utc_datetime)
            })
            .max();
        let capabilities = task
            .measurement_capabilities
            .clone()
            .unwrap_or_else(|| task_measurement_capabilities(task_key));
        let supported_metrics = [
            capabilities.pending.supported,
            capabilities.discovered.supported,
            capabilities.processed.supported,
        ];
        let coverage = if !coverage_gaps.is_empty() {
            "incomplete: recorder coverage gap"
        } else if samples.is_empty() {
            "no recorded attempts"
        } else if !supported_metrics.iter().any(|supported| *supported) {
            "not applicable"
        } else if samples.iter().any(|sample| {
            [&sample.pending, &sample.discovered, &sample.processed]
                .into_iter()
                .zip(supported_metrics)
                .any(|(metric, supported)| {
                    supported
                        && metric.as_ref().is_none_or(|metric| {
                            metric.value.is_none() || metric.coverage == "unknown"
                        })
                })
        }) {
            "some metrics unknown"
        } else {
            "recorded"
        };
        let summary = calculate_task_workload_summary(
            &samples,
            task.enabled,
            task.interval_secs,
            latest_capture_gap,
            Utc::now(),
        );
        let revision = self.timeline_revision().await?;
        Ok(TaskWorkloadTrend {
            revision,
            window_start: window_start_text,
            window_end: window_end_text,
            observed_at: format_utc_iso_millis(window_end),
            sample_limit: limit,
            truncated,
            capabilities,
            coverage: coverage.to_string(),
            samples,
            coverage_gaps,
            latest_pending: summary.latest_pending,
            latest_processed: summary.latest_processed,
            latest_observed_at: summary.latest_observed_at,
            processing_rate_per_second: summary.processing_rate_per_second,
            processing_rate_window: summary.processing_rate_window,
            clearance_eta: summary.clearance_eta,
            clearance_estimate_window: summary.clearance_estimate_window,
            clearance_estimate_coverage: summary.clearance_estimate_coverage,
            clearance_estimate_reason: summary.clearance_estimate_reason,
        })
    }

    pub(crate) async fn set_enabled(&self, task_key: &str, enabled: bool) -> Result<bool> {
        let Some((interval_secs, cron_expr, is_manual)) =
            sqlx::query_as::<_, (Option<i64>, Option<String>, bool)>(
                "SELECT interval_secs,cron_expr,is_manual FROM managed_tasks WHERE task_key=?",
            )
            .bind(task_key)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok(false);
        };
        let next_trigger_at = if enabled && !is_manual {
            next_trigger_at(interval_secs, cron_expr.as_deref())
        } else {
            None
        };
        sqlx::query(
            "UPDATE managed_tasks
             SET enabled=?, next_trigger_at=?,
                 next_catchup_at=CASE WHEN ? THEN next_catchup_at ELSE NULL END,
                 catchup_reason=CASE WHEN ? THEN catchup_reason ELSE NULL END,
                 updated_at=? WHERE task_key=?",
        )
        .bind(enabled as i64)
        .bind(next_trigger_at)
        .bind(enabled)
        .bind(enabled)
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(task_key)
        .execute(&self.pool)
        .await?;
        if task_key == "retention_archive" {
            self.sync_retention_progress_schedule().await?;
        }
        Ok(true)
    }

    pub(crate) async fn validate_interval(interval_secs: Option<i64>) -> Result<()> {
        if let Some(value) = interval_secs
            && value < MIN_INTERVAL_SECS
        {
            return Err(anyhow!(
                "interval must be at least {MIN_INTERVAL_SECS} seconds"
            ));
        }
        Ok(())
    }

    pub(crate) async fn set_schedule(
        &self,
        task_key: &str,
        interval_secs: Option<i64>,
        cron_expr: Option<&str>,
    ) -> Result<bool> {
        Self::validate_schedule(interval_secs, cron_expr).await?;
        let Some(enabled) = sqlx::query_scalar::<_, bool>(
            "SELECT enabled FROM managed_tasks WHERE task_key=? AND is_manual=0",
        )
        .bind(task_key)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(false);
        };
        let next_trigger_at = if enabled {
            next_trigger_at(interval_secs, cron_expr)
        } else {
            None
        };
        sqlx::query("UPDATE managed_tasks SET interval_secs=?, cron_expr=?, next_trigger_at=?, schedule_source='override', updated_at=? WHERE task_key=? AND is_manual=0")
        .bind(interval_secs).bind(cron_expr).bind(next_trigger_at).bind(format_utc_iso_millis(Utc::now())).bind(task_key).execute(&self.pool).await?;
        if task_key == "retention_archive" {
            self.sync_retention_progress_schedule().await?;
        }
        Ok(true)
    }

    pub(crate) async fn update_control(
        &self,
        task_key: &str,
        enabled: Option<bool>,
        interval_secs: Option<Option<i64>>,
        cron_expr: Option<Option<&str>>,
    ) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let Some((current_enabled, current_interval, current_cron, is_manual)) = sqlx::query_as::<
            _,
            (bool, Option<i64>, Option<String>, bool),
        >(
            "SELECT enabled,interval_secs,cron_expr,is_manual FROM managed_tasks WHERE task_key=?",
        )
        .bind(task_key)
        .fetch_optional(&mut *transaction)
        .await?
        else {
            transaction.commit().await?;
            return Ok(false);
        };
        let update_schedule = interval_secs.is_some() || cron_expr.is_some();
        if update_schedule && is_manual {
            transaction.commit().await?;
            return Err(anyhow!("manual or unknown task cannot be scheduled"));
        }
        let next_enabled = enabled.unwrap_or(current_enabled);
        let interval_value_provided = interval_secs.is_some_and(|value| value.is_some());
        let cron_value_provided = cron_expr.is_some_and(|value| value.is_some());
        if interval_value_provided && cron_value_provided {
            transaction.commit().await?;
            return Err(anyhow!("interval and cron schedule are mutually exclusive"));
        }
        let next_interval = if let Some(value) = interval_secs {
            value
        } else if cron_value_provided {
            None
        } else {
            current_interval
        };
        let next_cron = if let Some(value) = cron_expr {
            value.map(str::to_owned)
        } else if interval_value_provided {
            None
        } else {
            current_cron
        };
        if update_schedule {
            if !EDITABLE_SCHEDULE_TASKS.contains(&task_key)
                && (next_interval.is_some()
                    || next_cron
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty()))
            {
                transaction.commit().await?;
                return Err(anyhow!(
                    "task does not support a new interval or cron override"
                ));
            }
            Self::validate_schedule(next_interval, next_cron.as_deref()).await?;
        }
        let restore_retention_default = task_key == "retention_archive"
            && update_schedule
            && interval_secs == Some(None)
            && cron_expr == Some(None);
        let persisted_interval = if restore_retention_default {
            Some(DEFAULT_RETENTION_INTERVAL_SECS)
        } else {
            next_interval
        };
        let persisted_next_trigger_at = if next_enabled && !is_manual {
            next_trigger_at(persisted_interval, next_cron.as_deref())
        } else {
            None
        };
        sqlx::query(
            "UPDATE managed_tasks
             SET enabled=?,interval_secs=?,cron_expr=?,next_trigger_at=?,
                 next_catchup_at=CASE WHEN ? THEN next_catchup_at ELSE NULL END,
                 catchup_reason=CASE WHEN ? THEN catchup_reason ELSE NULL END,
                 schedule_source=CASE WHEN ? THEN 'default'
                                      WHEN ? THEN 'override'
                                      ELSE schedule_source END,
                 updated_at=? WHERE task_key=?",
        )
        .bind(next_enabled as i64)
        .bind(persisted_interval)
        .bind(next_cron)
        .bind(persisted_next_trigger_at)
        .bind(next_enabled)
        .bind(next_enabled)
        .bind(restore_retention_default)
        .bind(update_schedule)
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(task_key)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        if task_key == "retention_archive" {
            self.sync_retention_progress_schedule().await?;
        }
        Ok(true)
    }

    pub(crate) async fn restore_control_state(&self, task: &ManagedTask) -> Result<()> {
        sqlx::query(
            "UPDATE managed_tasks
             SET enabled=?,interval_secs=?,cron_expr=?,next_trigger_at=?,
                 next_catchup_at=CASE WHEN ? THEN next_catchup_at ELSE NULL END,
                 catchup_reason=CASE WHEN ? THEN catchup_reason ELSE NULL END,
                 updated_at=? WHERE task_key=?",
        )
        .bind(task.enabled as i64)
        .bind(task.interval_secs)
        .bind(task.cron_expr.as_deref())
        .bind(task.next_trigger_at.as_deref())
        .bind(task.enabled)
        .bind(task.enabled)
        .bind(format_utc_iso_millis(Utc::now()))
        .bind(&task.task_key)
        .execute(&self.pool)
        .await?;
        if task.task_key == "retention_archive" {
            self.sync_retention_progress_schedule().await?;
        }
        Ok(())
    }
}

pub(crate) fn path(config: &AppConfig) -> PathBuf {
    config.maintenance_database_path()
}

#[cfg(test)]
mod tests {
    use crate::format_utc_iso_millis;
    use chrono::TimeZone;
    use chrono::{Duration as ChronoDuration, Timelike, Utc};
    use sqlx::SqlitePool;
    use std::collections::HashSet;
    use std::sync::Arc;

    use super::{
        MANAGED_TASKS, MaintenanceStore, RetentionBacklogObservation, STARTUP_BACKFILL_TASKS,
        TaskWorkloadMetric, TaskWorkloadSample, calculate_task_workload_summary, cron_day_matches,
        ensure_schema, ensure_task_colors, floor_utc_hour, next_trigger_at, sanitize_task_detail,
        seed_tasks, task_enabled_by_default, task_measurement_capabilities, validate_cron_expr,
    };
    #[tokio::test]
    async fn schema_repair_preserves_duplicate_active_run_history() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect maintenance migration fixture");
        sqlx::query(
            "CREATE TABLE managed_tasks (
                task_key TEXT PRIMARY KEY, title TEXT NOT NULL, description TEXT NOT NULL,
                trigger_mode TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
                interval_secs INTEGER, cron_expr TEXT, next_trigger_at TEXT,
                is_manual INTEGER NOT NULL DEFAULT 0, schedule_source TEXT, updated_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create legacy task table");
        sqlx::query(
            "CREATE TABLE managed_task_runs (
                id INTEGER PRIMARY KEY AUTOINCREMENT, legacy_id INTEGER UNIQUE, task_key TEXT NOT NULL,
                trigger_kind TEXT NOT NULL DEFAULT 'unknown', started_at TEXT NOT NULL, finished_at TEXT,
                duration_ms INTEGER, status TEXT NOT NULL, summary TEXT, processed_count INTEGER,
                updated_count INTEGER, error_detail TEXT, completion TEXT, core_completion TEXT, details TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("create legacy task-run table");
        sqlx::query("INSERT INTO managed_tasks (task_key,title,description,trigger_mode,updated_at) VALUES ('retention_archive','Retention','Retention','interval','2026-10-01T00:00:00.000Z')")
            .execute(&pool)
            .await
            .expect("seed task");
        for status in ["running", "requested"] {
            sqlx::query("INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary) VALUES ('retention_archive','manual','2026-10-01T00:00:00.000Z',?,?)")
                .bind(status)
                .bind(format!("{status} run"))
                .execute(&pool)
                .await
                .expect("seed duplicate active run");
        }

        ensure_schema(&pool)
            .await
            .expect("repair maintenance schema");

        let rows: Vec<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT id,status,finished_at,error_detail FROM managed_task_runs ORDER BY id",
        )
        .fetch_all(&pool)
        .await
        .expect("load repaired run history");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, "running");
        assert_eq!(rows[1].1, "failed");
        assert!(rows[1].2.is_some());
        assert!(rows[1].3.is_some());
        let active_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM managed_task_runs WHERE task_key='retention_archive' AND status IN ('running','requested')",
        )
        .fetch_one(&pool)
        .await
        .expect("count active runs");
        assert_eq!(active_count, 1);
    }

    #[tokio::test]
    async fn timeline_upgrade_is_repeatable_and_preserves_task_controls_and_colors() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect timeline migration fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed managed task registry");
        sqlx::query("UPDATE managed_tasks SET enabled=0,interval_secs=123,cron_expr=NULL WHERE task_key='retention_archive'")
            .execute(&pool)
            .await
            .expect("set existing task control overrides");
        sqlx::query("UPDATE managed_tasks SET cron_expr='5 * * * *',interval_secs=NULL WHERE task_key='upstream_account_maintenance'")
            .execute(&pool)
            .await
            .expect("set a cron override");
        ensure_task_colors(&pool)
            .await
            .expect("persist task colors");
        let color_before: (String, String) = sqlx::query_as(
            "SELECT display_color_light,display_color_dark FROM managed_tasks WHERE task_key='retention_archive'",
        )
        .fetch_one(&pool)
        .await
        .expect("load persisted task colors");
        let distinct_colors: i64 = sqlx::query_scalar(
            "SELECT COUNT(DISTINCT display_color_light || ':' || display_color_dark) FROM managed_tasks",
        )
        .fetch_one(&pool)
        .await
        .expect("count task colors");
        assert_eq!(
            distinct_colors,
            (MANAGED_TASKS.len() + STARTUP_BACKFILL_TASKS.len()) as i64
        );

        ensure_schema(&pool)
            .await
            .expect("repeat maintenance schema upgrade");
        seed_tasks(&pool).await.expect("repeat task registry seed");
        ensure_task_colors(&pool)
            .await
            .expect("repeat task color assignment");
        let task: (bool, Option<i64>, Option<String>, String, String) = sqlx::query_as(
            "SELECT enabled,interval_secs,cron_expr,display_color_light,display_color_dark FROM managed_tasks WHERE task_key='retention_archive'",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated task");
        assert!(!task.0);
        assert_eq!(task.1, Some(123));
        assert_eq!(task.2, None);
        assert_eq!((task.3, task.4), color_before);
        let cron_override: (bool, Option<i64>, Option<String>) = sqlx::query_as(
            "SELECT enabled,interval_secs,cron_expr FROM managed_tasks WHERE task_key='upstream_account_maintenance'",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated cron override");
        assert!(cron_override.0);
        assert_eq!(cron_override.1, None);
        assert_eq!(cron_override.2.as_deref(), Some("5 * * * *"));
        let timeline_table_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='task_timeline_segments'",
        )
        .fetch_one(&pool)
        .await
        .expect("check timeline table");
        assert_eq!(timeline_table_exists, 1);
    }

    #[tokio::test]
    async fn timeline_history_records_actual_execution_and_restart_gaps() {
        // Keep observations inside the production history window on any calendar day.
        let base_time = Utc::now() - ChronoDuration::minutes(1);
        let timestamp =
            |seconds| format_utc_iso_millis(base_time + ChronoDuration::seconds(seconds));
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect timeline history fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed managed task registry");
        ensure_task_colors(&pool)
            .await
            .expect("seed stable task colors");
        let store = MaintenanceStore::from_pool(pool);
        sqlx::query("INSERT INTO managed_task_runs(task_key,trigger_kind,started_at,duration_ms,status) VALUES('retention_archive','manual',?,777,'running')")
            .bind(timestamp(0))
            .execute(&store.pool)
            .await
            .expect("insert legacy-semantics run");
        let run_id = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM managed_task_runs WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load managed run id");
        store
            .start_timeline_session("session-one", &timestamp(0))
            .await
            .expect("start first observation session");
        let events = [
            crate::task_timeline::TimelineEvent::ExecutionStarted {
                id: "execution-one".to_string(),
                task_key: "retention_archive".to_string(),
                title: "数据保留与归档".to_string(),
                trigger_kind: "manual".to_string(),
                execution_class: Some("maintenance_retention".to_string()),
                started_at: timestamp(1),
                managed_run_id: Some(run_id),
            },
            crate::task_timeline::TimelineEvent::DeferralStarted {
                id: "deferral-one".to_string(),
                task_key: "retention_archive".to_string(),
                reason: "pressure_cooldown".to_string(),
                retry_at: None,
                started_at: timestamp(2),
            },
        ];
        store
            .write_timeline_batch("session-one", &events, 0, &timestamp(3), false)
            .await
            .expect("write execution and deferral starts");
        let ends = [
            crate::task_timeline::TimelineEvent::ExecutionFinished {
                id: "execution-one".to_string(),
                finished_at: timestamp(6),
                duration_ms: 5_000,
                status: "failed".to_string(),
            },
            crate::task_timeline::TimelineEvent::DeferralFinished {
                id: "deferral-one".to_string(),
                finished_at: timestamp(5),
            },
        ];
        store
            .write_timeline_batch("session-one", &ends, 2, &timestamp(7), false)
            .await
            .expect("write execution and deferral finishes");
        let actual_fields: (String, i64, String, i64) = sqlx::query_as(
            "SELECT started_at,duration_ms,actual_started_at,actual_duration_ms FROM managed_task_runs WHERE id=?",
        )
        .bind(run_id)
        .fetch_one(&store.pool)
        .await
        .expect("read legacy and actual execution fields");
        assert_eq!(actual_fields.0, timestamp(0));
        assert_eq!(actual_fields.1, 777);
        assert_eq!(actual_fields.2, timestamp(1));
        assert_eq!(actual_fields.3, 5_000);

        let open_events = [
            crate::task_timeline::TimelineEvent::ExecutionStarted {
                id: "execution-unknown".to_string(),
                task_key: "retention_archive".to_string(),
                title: "数据保留与归档".to_string(),
                trigger_kind: "interval".to_string(),
                execution_class: None,
                started_at: timestamp(8),
                managed_run_id: None,
            },
            crate::task_timeline::TimelineEvent::ExecutionUnknown {
                id: "execution-unknown".to_string(),
                last_observed_at: timestamp(9),
            },
            crate::task_timeline::TimelineEvent::ExecutionStarted {
                id: "execution-open".to_string(),
                task_key: "pool_orphan_recovery".to_string(),
                title: "连接池孤儿记录恢复".to_string(),
                trigger_kind: "interval".to_string(),
                execution_class: None,
                started_at: timestamp(8),
                managed_run_id: None,
            },
            crate::task_timeline::TimelineEvent::DeferralStarted {
                id: "deferral-open".to_string(),
                task_key: "pool_orphan_recovery".to_string(),
                reason: "resource_busy".to_string(),
                retry_at: None,
                started_at: timestamp(8),
            },
            crate::task_timeline::TimelineEvent::CoverageGap {
                id: "write-gap".to_string(),
                started_at: timestamp(9),
                finished_at: timestamp(10),
                reason: "maintenance_store_write_unavailable".to_string(),
            },
        ];
        sqlx::query("CREATE TRIGGER reject_pending_timeline_write BEFORE INSERT ON task_timeline_segments BEGIN SELECT RAISE(ABORT, 'simulated maintenance write failure'); END")
            .execute(&store.pool)
            .await
            .expect("simulate shutdown persistence failure");
        let unpersisted = crate::task_timeline::TimelineEvent::ExecutionStarted {
            id: "lost-on-shutdown".to_string(),
            task_key: "retention_archive".to_string(),
            title: "数据保留与归档".to_string(),
            trigger_kind: "manual".to_string(),
            execution_class: None,
            started_at: timestamp(12),
            managed_run_id: None,
        };
        assert!(
            store
                .write_timeline_batch("session-one", &[unpersisted], 0, &timestamp(13), true)
                .await
                .is_err()
        );
        sqlx::query("DROP TRIGGER reject_pending_timeline_write")
            .execute(&store.pool)
            .await
            .expect("restore maintenance writes");
        store
            .write_timeline_batch("session-one", &open_events, 2, &timestamp(9), false)
            .await
            .expect("write open execution and deferral");
        store
            .start_timeline_session("session-two", &timestamp(20))
            .await
            .expect("recover open observations after restart");
        let coverage: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT session_id,last_seen_at,ended_at FROM task_timeline_coverage ORDER BY started_at",
        )
        .fetch_all(&store.pool)
        .await
        .expect("read coverage after failed shutdown persistence");
        assert_eq!(coverage[0].0, "session-one");
        assert_eq!(coverage[0].1, timestamp(9));
        assert_eq!(coverage[0].2.as_deref(), Some(timestamp(9).as_str()));
        assert_eq!(coverage[1].0, "session-two");
        assert_eq!(coverage[1].1, timestamp(20));
        let missing: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM task_timeline_segments WHERE segment_id='lost-on-shutdown'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("ensure unpersisted execution was not fabricated");
        assert_eq!(missing, 0);
        let recovered: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT segment_id,status,finished_at,last_observed_at FROM task_timeline_segments WHERE segment_id IN ('execution-open','deferral-open') ORDER BY segment_id",
        )
        .fetch_all(&store.pool)
        .await
        .expect("read restart-recovered observations");
        assert_eq!(recovered.len(), 2);
        assert!(recovered.iter().all(|row| row.1 == "interrupted"));
        assert!(recovered.iter().all(|row| row.2.is_none()));
        assert!(recovered.iter().all(|row| row.3 == timestamp(8)));
        let gap: (String, String, Option<String>, String) = sqlx::query_as(
            "SELECT kind,started_at,finished_at,reason FROM task_timeline_segments WHERE segment_id='write-gap'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read persisted maintenance-store gap");
        assert_eq!(gap.0, "coverage_gap");
        assert_eq!(gap.1, timestamp(9));
        assert_eq!(gap.2.as_deref(), Some(timestamp(10).as_str()));
        assert_eq!(gap.3, "maintenance_store_write_unavailable");
        let unknown: (Option<String>, Option<i64>, String) = sqlx::query_as(
            "SELECT finished_at,duration_ms,status FROM task_timeline_segments WHERE segment_id='execution-unknown'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read unknown execution terminal");
        assert_eq!(unknown.0, None);
        assert_eq!(unknown.1, None);
        assert_eq!(unknown.2, "unknown");
    }

    #[tokio::test]
    async fn dropped_event_gaps_are_persisted_as_bounded_intervals() {
        // Keep observations inside the production history window on any calendar day.
        let base_time = Utc::now() - ChronoDuration::minutes(1);
        let timestamp =
            |seconds| format_utc_iso_millis(base_time + ChronoDuration::seconds(seconds));
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect timeline gap fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed managed task registry");
        ensure_task_colors(&pool)
            .await
            .expect("seed stable task colors");
        let store = MaintenanceStore::from_pool(pool);
        store
            .start_timeline_session("drop-session", &timestamp(0))
            .await
            .expect("start drop session");
        let first = crate::task_timeline::TimelineEvent::CoverageGap {
            id: "drop-gap".to_string(),
            started_at: timestamp(5),
            finished_at: timestamp(6),
            reason: "event_channel_overflow".to_string(),
        };
        store
            .write_timeline_batch("drop-session", &[first], 1, &timestamp(6), false)
            .await
            .expect("persist first overflow interval");
        let extended = crate::task_timeline::TimelineEvent::CoverageGap {
            id: "drop-gap".to_string(),
            started_at: timestamp(5),
            finished_at: timestamp(9),
            reason: "event_channel_overflow".to_string(),
        };
        store
            .write_timeline_batch("drop-session", &[extended], 4, &timestamp(9), false)
            .await
            .expect("extend overflow interval");

        let persisted: (String, String, i64) = sqlx::query_as(
            "SELECT started_at,finished_at,revision FROM task_timeline_segments WHERE segment_id='drop-gap'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read localized overflow interval");
        let dropped_total: i64 = sqlx::query_scalar(
            "SELECT dropped_events FROM task_timeline_coverage WHERE session_id='drop-session'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read cumulative drop count");
        assert_eq!(persisted.0, timestamp(5));
        assert_eq!(persisted.1, timestamp(9));
        assert_eq!(persisted.2, 3);
        assert_eq!(dropped_total, 4);
    }

    #[tokio::test]
    async fn current_admission_waits_require_fresh_recorder_coverage() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect admission wait fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed managed task registry");
        ensure_task_colors(&pool).await.expect("seed task colors");
        let store = MaintenanceStore::from_pool(pool);
        let now = Utc::now();
        let observed_at = format_utc_iso_millis(now);
        store
            .start_timeline_session("admission-session", &observed_at)
            .await
            .expect("start fresh recorder coverage");
        let wait = crate::task_timeline::TimelineEvent::DeferralStarted {
            id: "active-admission-wait".to_string(),
            task_key: "retention_archive".to_string(),
            reason: "pressure_cooldown".to_string(),
            retry_at: None,
            started_at: observed_at.clone(),
        };
        store
            .write_timeline_batch("admission-session", &[wait], 0, &observed_at, false)
            .await
            .expect("persist active admission wait");
        let waits = store
            .list_current_task_deferrals()
            .await
            .expect("read fresh admission wait");
        assert_eq!(waits.len(), 1);
        assert_eq!(waits[0].id, "active-admission-wait");

        let stale_at = format_utc_iso_millis(now - ChronoDuration::minutes(2));
        sqlx::query(
            "UPDATE task_timeline_coverage SET last_seen_at=? WHERE session_id='admission-session'",
        )
        .bind(&stale_at)
        .execute(&store.pool)
        .await
        .expect("age recorder coverage");
        sqlx::query("UPDATE task_timeline_segments SET last_observed_at=? WHERE segment_id='active-admission-wait'")
            .bind(&stale_at)
            .execute(&store.pool)
            .await
            .expect("age the open wait interval");
        assert!(store.list_current_task_deferrals().await.is_err());
    }

    #[tokio::test]
    async fn timeline_pages_use_a_fixed_watermark_and_return_later_revisions_once() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect timeline paging fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed managed task registry");
        ensure_task_colors(&pool)
            .await
            .expect("seed stable task colors");
        let store = MaintenanceStore::from_pool(pool);
        let now = Utc::now();
        let started = (0..3)
            .map(
                |index| crate::task_timeline::TimelineEvent::ExecutionStarted {
                    id: format!("page-run-{index}"),
                    task_key: "retention_archive".to_string(),
                    title: "数据保留与归档".to_string(),
                    trigger_kind: "interval".to_string(),
                    execution_class: None,
                    started_at: format_utc_iso_millis(
                        now - ChronoDuration::seconds(60 - index * 20),
                    ),
                    managed_run_id: None,
                },
            )
            .collect::<Vec<_>>();
        let observed_at = format_utc_iso_millis(now);
        store
            .start_timeline_session("paging-session", &observed_at)
            .await
            .expect("start timeline paging session");
        store
            .write_timeline_batch("paging-session", &started, 0, &observed_at, false)
            .await
            .expect("write timeline paging rows");

        let first = crate::task_timeline::timeline_page(&store, None, None, None, None, 2)
            .await
            .expect("read first fixed-watermark page");
        assert_eq!(first.segments.len(), 2);
        let cursor = first.next_cursor.clone().expect("first page cursor");
        let second =
            crate::task_timeline::timeline_page(&store, Some(&cursor), None, None, None, 2)
                .await
                .expect("read second fixed-watermark page");
        assert_eq!(second.segments.len(), 1);
        assert!(second.next_cursor.is_none());
        assert_eq!(first.watermark, second.watermark);
        let ids = first
            .segments
            .iter()
            .chain(&second.segments)
            .map(|segment| segment.segment_id.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), 3);

        let finish = [crate::task_timeline::TimelineEvent::ExecutionFinished {
            id: "page-run-0".to_string(),
            finished_at: observed_at.clone(),
            duration_ms: 60_000,
            status: "success".to_string(),
        }];
        store
            .write_timeline_batch("paging-session", &finish, 0, &observed_at, false)
            .await
            .expect("finish one timeline row");
        let delta = crate::task_timeline::timeline_page(
            &store,
            None,
            Some(first.watermark),
            None,
            None,
            500,
        )
        .await
        .expect("read timeline revision delta");
        assert_eq!(delta.segments.len(), 1);
        assert_eq!(delta.segments[0].segment_id, "page-run-0");
        assert_eq!(delta.segments[0].status, "success");
        assert!(delta.watermark > first.watermark);
    }

    #[test]
    fn managed_task_registry_matches_the_operations_catalog() {
        assert_eq!(MANAGED_TASKS.len(), 21);
        assert_eq!(
            MANAGED_TASKS
                .iter()
                .filter(|(_, _, _, _, is_manual)| *is_manual)
                .count(),
            6
        );
        assert_eq!(STARTUP_BACKFILL_TASKS.len(), 16);
    }

    #[test]
    fn declares_only_measured_units_for_every_root_and_backfill_task() {
        for (task_key, ..) in MANAGED_TASKS {
            let capability = task_measurement_capabilities(task_key);
            for metric in [
                capability.pending,
                capability.discovered,
                capability.processed,
            ] {
                assert_eq!(metric.supported, metric.unit.is_some());
                assert_eq!(metric.supported, metric.scope.is_some());
            }
        }
        for suffix in STARTUP_BACKFILL_TASKS {
            let task_key = format!("startup_backfill.{suffix}");
            let capability = task_measurement_capabilities(&task_key);
            for metric in [
                capability.pending,
                capability.discovered,
                capability.processed,
            ] {
                assert_eq!(metric.supported, metric.unit.is_some(), "{task_key}");
                assert_eq!(metric.supported, metric.scope.is_some(), "{task_key}");
            }
        }

        assert_eq!(
            task_measurement_capabilities("startup_backfill.upstream_activity_live")
                .processed
                .unit
                .as_deref(),
            Some("accounts")
        );
        assert_eq!(
            task_measurement_capabilities(
                "startup_backfill.prompt_cache_conversations_materialization"
            )
            .processed
            .unit
            .as_deref(),
            Some("conversation sessions")
        );
        assert!(
            task_measurement_capabilities("startup_backfill.pool_upstream_node_health_archives")
                .processed
                .supported
        );
        assert_eq!(
            task_measurement_capabilities("retention_archive")
                .pending
                .unit
                .as_deref(),
            Some("invocation rows")
        );
    }

    fn workload_fixture_sample(
        index: usize,
        observed_at: chrono::DateTime<Utc>,
        pending: Option<i64>,
        processed: Option<i64>,
        status: &str,
        execution_times: (Option<String>, Option<String>),
        range: &str,
    ) -> TaskWorkloadSample {
        let (actual_started_at, finished_at) = execution_times;
        let observed_at = format_utc_iso_millis(observed_at);
        let metric = |value, coverage: &str| TaskWorkloadMetric {
            value,
            unit: "invocation rows".to_string(),
            scope: "expired_invocations:retention_policy:7days".to_string(),
            range: range.to_string(),
            observed_at: Some(observed_at.clone()),
            coverage: coverage.to_string(),
        };
        TaskWorkloadSample {
            sample_id: format!("workload-{index}"),
            execution_uid: format!("execution-{index}"),
            managed_run_id: Some(index as i64),
            task_key: "retention_archive".to_string(),
            trigger_kind: "interval".to_string(),
            attempted_at: observed_at.clone(),
            actual_started_at,
            finished_at,
            duration_ms: None,
            status: status.to_string(),
            reason: None,
            sequence: 1,
            pending: pending.map(|value| metric(Some(value), "exact")),
            discovered: None,
            processed: processed.map(|value| metric(Some(value), "window")),
            subset_relation: "unknown".to_string(),
        }
    }

    #[test]
    fn estimates_only_from_fresh_complete_same_range_backlog_samples() {
        let start = Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap();
        let mut samples = (0..5)
            .map(|index| {
                let observed = start + ChronoDuration::minutes(index as i64 * 2);
                workload_fixture_sample(
                    index,
                    observed,
                    Some(100 - index as i64 * 10),
                    Some(10),
                    "success",
                    (
                        Some(format_utc_iso_millis(observed + ChronoDuration::seconds(1))),
                        Some(format_utc_iso_millis(
                            observed + ChronoDuration::seconds(20),
                        )),
                    ),
                    "complete eligible range",
                )
            })
            .collect::<Vec<_>>();
        let now = start + ChronoDuration::minutes(9);
        let summary = calculate_task_workload_summary(&samples, true, Some(120), None, now);
        assert_eq!(
            summary
                .latest_pending
                .as_ref()
                .and_then(|metric| metric.value),
            Some(60)
        );
        assert!(summary.clearance_eta.is_some());
        assert_eq!(
            summary.clearance_estimate_window.as_deref(),
            Some("最近 24 小时")
        );
        assert!(
            summary
                .clearance_estimate_coverage
                .as_deref()
                .is_some_and(|value| value.contains("5 个准确完整积压快照"))
        );

        samples[2].pending.as_mut().unwrap().range = "different eligibility range".to_string();
        let changed_range = calculate_task_workload_summary(&samples, true, Some(120), None, now);
        assert!(changed_range.clearance_eta.is_none());

        let gap = start + ChronoDuration::minutes(5);
        let after_gap = calculate_task_workload_summary(&samples, true, Some(120), Some(gap), now);
        assert!(after_gap.clearance_eta.is_none());
        assert_eq!(after_gap.clearance_estimate_reason, "insufficient_samples");
    }

    #[test]
    fn rejects_clearance_estimates_outside_representable_datetime_range() {
        let start = Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap();
        let samples = (0..5)
            .map(|index| {
                let observed = start + ChronoDuration::minutes(index as i64 * 2);
                workload_fixture_sample(
                    index,
                    observed,
                    Some(9_000_000_000_000_000 - index as i64 * 100),
                    Some(10),
                    "success",
                    (
                        Some(format_utc_iso_millis(observed + ChronoDuration::seconds(1))),
                        Some(format_utc_iso_millis(
                            observed + ChronoDuration::seconds(20),
                        )),
                    ),
                    "complete eligible range",
                )
            })
            .collect::<Vec<_>>();
        let now = start + ChronoDuration::minutes(9);

        let summary = calculate_task_workload_summary(&samples, true, Some(120), None, now);

        assert!(summary.clearance_eta.is_none());
        assert_eq!(summary.clearance_estimate_reason, "estimate_out_of_range");
    }

    #[test]
    fn confirmed_skips_count_as_zero_without_fabricating_a_sample_metric() {
        let start = Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap();
        let first = workload_fixture_sample(
            0,
            start + ChronoDuration::minutes(1),
            None,
            Some(10),
            "success",
            (
                Some(format_utc_iso_millis(
                    start + ChronoDuration::minutes(1) + ChronoDuration::seconds(1),
                )),
                Some(format_utc_iso_millis(
                    start + ChronoDuration::minutes(1) + ChronoDuration::seconds(30),
                )),
            ),
            "complete eligible range",
        );
        let skipped = workload_fixture_sample(
            1,
            start + ChronoDuration::minutes(2),
            None,
            None,
            "skipped",
            (
                None,
                Some(format_utc_iso_millis(start + ChronoDuration::minutes(2))),
            ),
            "complete eligible range",
        );
        let leading_skipped = workload_fixture_sample(
            3,
            start,
            None,
            None,
            "skipped",
            (
                None,
                Some(format_utc_iso_millis(
                    start + ChronoDuration::milliseconds(500),
                )),
            ),
            "complete eligible range",
        );
        let last = workload_fixture_sample(
            2,
            start + ChronoDuration::minutes(5),
            None,
            Some(10),
            "success",
            (
                Some(format_utc_iso_millis(start + ChronoDuration::minutes(5))),
                Some(format_utc_iso_millis(
                    start + ChronoDuration::minutes(5) + ChronoDuration::seconds(30),
                )),
            ),
            "complete eligible range",
        );
        let trailing_skipped = workload_fixture_sample(
            4,
            start + ChronoDuration::minutes(6),
            None,
            None,
            "skipped",
            (
                None,
                Some(format_utc_iso_millis(start + ChronoDuration::minutes(6))),
            ),
            "complete eligible range",
        );
        assert!(skipped.processed.is_none());
        let summary = calculate_task_workload_summary(
            &[leading_skipped, first, skipped, last, trailing_skipped],
            true,
            Some(120),
            None,
            start + ChronoDuration::minutes(6),
        );
        assert!(
            summary
                .processing_rate_window
                .as_deref()
                .is_some_and(|value| {
                    value.starts_with("5 次计数完整尝试")
                        && value.contains(&format_utc_iso_millis(start))
                })
        );
        assert!(summary.processing_rate_per_second.is_some());
        assert!(summary.clearance_eta.is_none());
    }

    #[test]
    fn fresh_snapshot_remains_visible_after_a_confirmed_skip_without_extending_eta_window() {
        let start = Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap();
        let samples = (0..5)
            .map(|index| {
                let observed = start + ChronoDuration::minutes(index as i64 * 2);
                workload_fixture_sample(
                    index,
                    observed,
                    Some(100 - index as i64 * 10),
                    Some(10),
                    "success",
                    (
                        Some(format_utc_iso_millis(observed + ChronoDuration::seconds(1))),
                        Some(format_utc_iso_millis(
                            observed + ChronoDuration::seconds(20),
                        )),
                    ),
                    "complete eligible range",
                )
            })
            .chain(std::iter::once(workload_fixture_sample(
                5,
                start + ChronoDuration::minutes(9),
                None,
                None,
                "skipped",
                (
                    None,
                    Some(format_utc_iso_millis(start + ChronoDuration::minutes(9))),
                ),
                "complete eligible range",
            )))
            .collect::<Vec<_>>();

        let summary = calculate_task_workload_summary(
            &samples,
            true,
            Some(120),
            None,
            start + ChronoDuration::minutes(10),
        );
        assert_eq!(
            summary
                .latest_pending
                .as_ref()
                .and_then(|metric| metric.value),
            Some(60)
        );
        assert!(summary.clearance_eta.is_none());
        assert_eq!(summary.clearance_estimate_reason, "insufficient_samples");
    }

    #[tokio::test]
    async fn workload_migration_is_repeatable_and_protects_recent_and_running_samples() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect workload migration fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "UPDATE managed_tasks SET enabled=0,interval_secs=123,
                display_color_light='#123456',display_color_dark='#abcdef'
             WHERE task_key='retention_archive'",
        )
        .execute(&pool)
        .await
        .expect("seed persisted task settings");

        let start = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        for index in 0..205 {
            let attempted = start + ChronoDuration::seconds(index as i64);
            let sample = workload_fixture_sample(
                index,
                attempted,
                None,
                Some(index as i64),
                "success",
                (
                    Some(format_utc_iso_millis(attempted)),
                    Some(format_utc_iso_millis(
                        attempted + ChronoDuration::seconds(1),
                    )),
                ),
                "historical window",
            );
            sqlx::query(
                "INSERT INTO managed_task_work_runs
                 (execution_uid,task_key,managed_run_id,attempted_at,sequence,status,sample_json,updated_at)
                 VALUES(?,?,?,?,?,?,?,?)",
            )
            .bind(&sample.execution_uid)
            .bind(&sample.task_key)
            .bind(sample.managed_run_id)
            .bind(&sample.attempted_at)
            .bind(sample.sequence as i64)
            .bind(&sample.status)
            .bind(serde_json::to_string(&sample).unwrap())
            .bind(&sample.attempted_at)
            .execute(&pool)
            .await
            .expect("seed old workload sample");
        }
        let mut running = workload_fixture_sample(
            1000,
            start - ChronoDuration::days(1),
            None,
            None,
            "running",
            (
                Some(format_utc_iso_millis(start - ChronoDuration::days(1))),
                None,
            ),
            "active window",
        );
        running.execution_uid = "active-execution".to_string();
        running.sample_id = "active-execution:retention_archive".to_string();
        sqlx::query(
            "INSERT INTO managed_task_work_runs
             (execution_uid,task_key,attempted_at,sequence,status,sample_json,updated_at)
             VALUES(?,?,?,?,?,?,?)",
        )
        .bind(&running.execution_uid)
        .bind(&running.task_key)
        .bind(&running.attempted_at)
        .bind(running.sequence as i64)
        .bind(&running.status)
        .bind(serde_json::to_string(&running).unwrap())
        .bind(&running.attempted_at)
        .execute(&pool)
        .await
        .expect("seed active workload sample");
        for index in 0..101 {
            let attempted_at = format_utc_iso_millis(
                start - ChronoDuration::days(2) + ChronoDuration::seconds(index as i64),
            );
            sqlx::query(
                "INSERT INTO managed_task_work_runs
                 (execution_uid,task_key,attempted_at,sequence,status,sample_json,updated_at)
                 VALUES(?,?,?,?,?,?,?)",
            )
            .bind(format!("corrupt-execution-{index}"))
            .bind("retention_archive")
            .bind(&attempted_at)
            .bind(4_i64)
            .bind("running")
            .bind("not-json")
            .bind(&attempted_at)
            .execute(&pool)
            .await
            .expect("seed malformed running workload sample");
        }

        ensure_schema(&pool)
            .await
            .expect("repeat maintenance migration");
        ensure_schema(&pool).await.expect("repeat migration again");
        let preserved_settings =
            sqlx::query_as::<_, (bool, Option<i64>, Option<String>, Option<String>)>(
                "SELECT enabled,interval_secs,display_color_light,display_color_dark
             FROM managed_tasks WHERE task_key='retention_archive'",
            )
            .fetch_one(&pool)
            .await
            .expect("read persisted task settings");
        assert_eq!(
            preserved_settings,
            (
                false,
                Some(123),
                Some("#123456".to_string()),
                Some("#abcdef".to_string())
            )
        );
        let counts = sqlx::query_as::<_, (i64, i64)>(
            "SELECT SUM(status <> 'running'),SUM(status = 'running') FROM managed_task_work_runs",
        )
        .fetch_one(&pool)
        .await
        .expect("count protected workload samples");
        assert_eq!(counts, (200, 102));

        let store = MaintenanceStore::from_pool(pool);
        let detail = store
            .detail("retention_archive")
            .await
            .expect("read managed task detail")
            .expect("retention task exists");
        let trend = detail.workload_trend;
        assert_eq!(trend.samples.len(), 100);
        assert!(trend.samples.iter().any(|sample| {
            sample.sample_id == "active-execution:retention_archive" && sample.status == "running"
        }));
        assert!(
            !trend
                .samples
                .iter()
                .any(|sample| sample.sample_id == "execution-5:retention_archive")
        );
        assert_eq!(
            trend.samples.last().map(|sample| sample.sample_id.as_str()),
            Some("execution-204:retention_archive")
        );

        store
            .start_timeline_session("workload-restart", "2026-10-03T00:00:00.000Z")
            .await
            .expect("recover running workload sample after restart");
        let restarted = sqlx::query_scalar::<_, String>(
            "SELECT sample_json FROM managed_task_work_runs WHERE execution_uid='active-execution'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read recovered sample");
        let restarted: TaskWorkloadSample = serde_json::from_str(&restarted).unwrap();
        assert_eq!(restarted.status, "unknown");
        assert_eq!(restarted.sequence, 2);
        let recovered_counts = sqlx::query_as::<_, (i64, i64)>(
            "SELECT SUM(status='unknown'),SUM(status='running') FROM managed_task_work_runs",
        )
        .fetch_one(&store.pool)
        .await
        .expect("count recovered workload samples");
        assert_eq!(recovered_counts, (102, 0));
        let malformed = sqlx::query_scalar::<_, String>(
            "SELECT sample_json FROM managed_task_work_runs
             WHERE execution_uid='corrupt-execution-0' AND task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read rebuilt malformed sample");
        let malformed: TaskWorkloadSample = serde_json::from_str(&malformed).unwrap();
        assert_eq!(malformed.status, "unknown");
        assert_eq!(malformed.sequence, 5);
        assert!(malformed.pending.is_none());
        assert_eq!(
            malformed.reason.as_deref(),
            Some("服务重启时发现工作量记录损坏，计数未知")
        );
    }

    #[tokio::test]
    async fn workload_window_filters_old_samples_and_reports_limit_metadata() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect workload window fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let now = Utc::now();
        for (index, age) in [(0, 26_i64), (1, 2_i64), (2, 1_i64), (3, -2_i64)] {
            let mut sample = workload_fixture_sample(
                index,
                now - ChronoDuration::hours(age),
                Some(20 - index as i64),
                Some(index as i64),
                "success",
                (
                    Some(format_utc_iso_millis(now - ChronoDuration::hours(age))),
                    Some(format_utc_iso_millis(
                        now - ChronoDuration::hours(age) + ChronoDuration::seconds(1),
                    )),
                ),
                "run-window",
            );
            sample.discovered = sample.pending.clone().map(|mut metric| {
                metric.coverage = "window".to_string();
                metric
            });
            sqlx::query(
                "INSERT INTO managed_task_work_runs
                 (execution_uid,task_key,managed_run_id,attempted_at,sequence,status,sample_json,updated_at)
                 VALUES(?,?,?,?,?,?,?,?)",
            )
            .bind(&sample.execution_uid)
            .bind(&sample.task_key)
            .bind(sample.managed_run_id)
            .bind(&sample.attempted_at)
            .bind(sample.sequence as i64)
            .bind(&sample.status)
            .bind(serde_json::to_string(&sample).unwrap())
            .bind(&sample.attempted_at)
            .execute(&pool)
            .await
            .expect("seed workload window sample");
        }
        let gap_at = format_utc_iso_millis(now - ChronoDuration::hours(1));
        sqlx::query(
            "INSERT INTO task_timeline_segments
             (segment_id,session_id,kind,task_key,title,started_at,last_observed_at,finished_at,
              duration_ms,status,trigger_kind,execution_class,reason,retry_at,active_child_task_key,
              active_child_title,managed_run_id,revision)
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind("coverage-gap-global")
        .bind("coverage-gap-session")
        .bind("coverage_gap")
        .bind("__timeline__")
        .bind("长期统计投影")
        .bind(&gap_at)
        .bind(&gap_at)
        .bind(Option::<String>::None)
        .bind(Option::<i64>::None)
        .bind("unknown")
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Some("other task gap"))
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<i64>::None)
        .bind(1_i64)
        .execute(&pool)
        .await
        .expect("seed unrelated coverage gap");

        let store = MaintenanceStore::from_pool(pool);
        let trend = store
            .workload_window("retention_archive", 1)
            .await
            .expect("read workload window")
            .expect("retention task exists");
        assert_eq!(trend.sample_limit, 1);
        assert!(trend.truncated);
        assert_eq!(trend.samples.len(), 1);
        assert_eq!(trend.samples[0].execution_uid, "execution-2");
        assert!(trend.samples[0].attempted_at >= trend.window_start);
        assert_eq!(trend.window_end, trend.observed_at);
        assert_eq!(trend.coverage, "incomplete: recorder coverage gap");
        assert_eq!(trend.coverage_gaps.len(), 1);
        assert_eq!(trend.coverage_gaps[0].id, "coverage-gap-global");
        let empty_trend = store
            .workload_window("summary_snapshot", 1)
            .await
            .expect("read empty workload window")
            .expect("summary task exists");
        assert!(empty_trend.samples.is_empty());
        assert_eq!(empty_trend.coverage, "incomplete: recorder coverage gap");
    }

    #[tokio::test]
    async fn workload_store_ignores_out_of_order_sample_sequences() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect workload sequence fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        let store = MaintenanceStore::from_pool(pool);
        store
            .start_timeline_session("workload-sequence", "2026-10-03T00:00:00.000Z")
            .await
            .expect("start timeline session");
        let now = Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap();
        let mut latest = workload_fixture_sample(
            1,
            now,
            None,
            Some(7),
            "success",
            (
                Some(format_utc_iso_millis(now)),
                Some(format_utc_iso_millis(now + ChronoDuration::seconds(1))),
            ),
            "latest",
        );
        latest.execution_uid = "stable-execution".to_string();
        latest.sample_id = "stable-execution:retention_archive".to_string();
        latest.sequence = 8;
        store
            .write_timeline_batch(
                "workload-sequence",
                &[crate::task_timeline::TimelineEvent::WorkloadSampleChanged {
                    sample: Box::new(latest.clone()),
                }],
                0,
                "2026-10-03T00:00:02.000Z",
                false,
            )
            .await
            .expect("write newest workload event");
        latest.sequence = 3;
        latest.processed.as_mut().unwrap().value = Some(2);
        store
            .write_timeline_batch(
                "workload-sequence",
                &[crate::task_timeline::TimelineEvent::WorkloadSampleChanged {
                    sample: Box::new(latest),
                }],
                0,
                "2026-10-03T00:00:03.000Z",
                false,
            )
            .await
            .expect("write stale workload event");
        let sample_json = sqlx::query_scalar::<_, String>(
            "SELECT sample_json FROM managed_task_work_runs
             WHERE execution_uid='stable-execution' AND task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read monotonic sample");
        let sample: TaskWorkloadSample = serde_json::from_str(&sample_json).unwrap();
        assert_eq!(sample.sequence, 8);
        assert_eq!(
            sample.processed.as_ref().and_then(|metric| metric.value),
            Some(7)
        );
    }

    #[tokio::test]
    async fn workload_recorder_persists_runs_without_detail_subscribers() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect workload recorder fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        let store = MaintenanceStore::from_pool(pool);
        let attempted_at = Utc::now();
        let sample = workload_fixture_sample(
            0,
            attempted_at,
            Some(12),
            Some(4),
            "success",
            (
                Some(format_utc_iso_millis(
                    attempted_at + ChronoDuration::milliseconds(10),
                )),
                Some(format_utc_iso_millis(
                    attempted_at + ChronoDuration::seconds(1),
                )),
            ),
            "complete eligible range",
        );
        let execution_uid = sample.execution_uid.clone();

        crate::task_timeline::start_recorder(Arc::new(store.clone())).await;
        crate::task_timeline::workload_sample_changed(sample);
        crate::task_timeline::drain_after_shutdown().await;

        let stored = sqlx::query_scalar::<_, String>(
            "SELECT sample_json FROM managed_task_work_runs WHERE execution_uid=? AND task_key='retention_archive'",
        )
        .bind(execution_uid)
        .fetch_one(&store.pool)
        .await
        .expect("read asynchronously persisted workload sample");
        let stored: TaskWorkloadSample = serde_json::from_str(&stored).unwrap();
        assert_eq!(stored.status, "success");
        assert_eq!(
            stored.pending.as_ref().and_then(|metric| metric.value),
            Some(12)
        );
        assert_eq!(
            stored.processed.as_ref().and_then(|metric| metric.value),
            Some(4)
        );

        let ended_at = sqlx::query_scalar::<_, Option<String>>(
            "SELECT ended_at FROM task_timeline_coverage ORDER BY started_at DESC LIMIT 1",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read closed recorder session");
        assert!(ended_at.is_some());
    }

    #[tokio::test]
    async fn legacy_run_metrics_remain_unknown_and_associated_samples_deduplicate() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect legacy workload fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);
        let run_id = sqlx::query_scalar::<_, i64>(
            "INSERT INTO managed_task_runs
             (task_key,trigger_kind,started_at,actual_started_at,finished_at,actual_finished_at,status,execution_uid)
             VALUES('retention_archive','manual','2026-10-03T00:00:00.000Z',
                    '2026-10-03T00:00:01.000Z','2026-10-03T00:00:02.000Z',
                    '2026-10-03T00:00:02.000Z','success','legacy-execution')
             RETURNING id",
        )
        .fetch_one(&store.pool)
        .await
        .expect("seed legacy managed run");
        let detail = store.detail("retention_archive").await.unwrap().unwrap();
        assert_eq!(detail.workload_trend.samples.len(), 1);
        assert!(detail.workload_trend.samples[0].processed.is_none());

        let now = Utc.with_ymd_and_hms(2026, 10, 3, 0, 0, 1).unwrap();
        let mut sample = workload_fixture_sample(
            0,
            now,
            None,
            Some(4),
            "success",
            (
                Some(format_utc_iso_millis(now)),
                Some(format_utc_iso_millis(now + ChronoDuration::seconds(1))),
            ),
            "run window",
        );
        let unknown_metric = TaskWorkloadMetric {
            value: None,
            unit: "invocation rows".to_string(),
            scope: "expired_invocations:retention_policy:7days".to_string(),
            range: "run window".to_string(),
            observed_at: None,
            coverage: "unknown".to_string(),
        };
        sample.pending = Some(unknown_metric.clone());
        sample.discovered = Some(unknown_metric);
        sample.execution_uid = "legacy-execution".to_string();
        sample.sample_id = "legacy-execution:retention_archive".to_string();
        sample.managed_run_id = Some(run_id);
        sqlx::query(
            "INSERT INTO managed_task_work_runs
             (execution_uid,task_key,managed_run_id,attempted_at,sequence,status,sample_json,updated_at)
             VALUES(?,?,?,?,?,?,?,?)",
        )
        .bind(&sample.execution_uid)
        .bind(&sample.task_key)
        .bind(sample.managed_run_id)
        .bind(&sample.attempted_at)
        .bind(sample.sequence as i64)
        .bind(&sample.status)
        .bind(serde_json::to_string(&sample).unwrap())
        .bind(&sample.attempted_at)
        .execute(&store.pool)
        .await
        .expect("associate sample with legacy managed run");
        let detail = store.detail("retention_archive").await.unwrap().unwrap();
        assert_eq!(detail.workload_trend.samples.len(), 1);
        assert_eq!(detail.workload_trend.coverage, "some metrics unknown");
        assert_eq!(
            detail.workload_trend.samples[0]
                .processed
                .as_ref()
                .and_then(|metric| metric.value),
            Some(4)
        );
    }

    #[tokio::test]
    async fn malformed_terminal_workload_sample_keeps_attempt_identity_and_unknown_metrics() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect malformed workload fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);
        sqlx::query(
            "INSERT INTO managed_task_work_runs
             (execution_uid,task_key,managed_run_id,attempted_at,sequence,status,sample_json,updated_at)
             VALUES('corrupt-run','retention_archive',NULL,'2026-10-03T00:00:00.000Z',4,'failed','not-json','2026-10-03T00:00:02.000Z')",
        )
        .execute(&store.pool)
        .await
        .expect("seed malformed terminal sample");

        let detail = store.detail("retention_archive").await.unwrap().unwrap();
        assert_eq!(detail.workload_trend.samples.len(), 1);
        let sample = &detail.workload_trend.samples[0];
        assert_eq!(sample.sample_id, "corrupt-run:retention_archive");
        assert_eq!(sample.execution_uid, "corrupt-run");
        assert_eq!(sample.status, "failed");
        assert_eq!(sample.sequence, 4);
        assert!(sample.pending.is_none());
        assert!(sample.discovered.is_none());
        assert!(sample.processed.is_none());
        assert_eq!(detail.workload_trend.coverage, "some metrics unknown");
        assert_eq!(sample.reason.as_deref(), Some("工作量记录损坏，计量未知"));
    }

    #[test]
    fn disables_backfill_and_manual_tasks_by_default() {
        assert!(!task_enabled_by_default("startup_backfill", false));
        assert!(!task_enabled_by_default(
            "startup_backfill.proxy_usage",
            false
        ));
        assert!(!task_enabled_by_default("raw_compression", true));
        assert!(task_enabled_by_default("retention_archive", false));
        assert_eq!(
            MANAGED_TASKS
                .iter()
                .filter(|(key, _, _, _, is_manual)| task_enabled_by_default(key, *is_manual))
                .count(),
            14
        );
    }

    #[test]
    fn computes_interval_next_trigger_after_the_minimum_safety_window() {
        let before = Utc::now();
        let next = next_trigger_at(Some(60), None).expect("interval should produce a trigger");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next)
            .expect("next trigger should be RFC3339")
            .with_timezone(&Utc);

        assert!(parsed >= before + chrono::Duration::seconds(59));
        assert!(parsed <= before + chrono::Duration::seconds(61));
    }

    #[test]
    fn computes_the_next_utc_cron_minute() {
        let now = Utc::now();
        let next = next_trigger_at(None, Some("*/5 * * * *"))
            .expect("five-minute cron should produce a trigger");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next)
            .expect("next trigger should be RFC3339")
            .with_timezone(&Utc);

        assert!(parsed > now);
        assert_eq!(parsed.minute() % 5, 0);
        assert_eq!(parsed.second(), 0);
        assert_eq!(parsed.nanosecond(), 0);
    }

    #[test]
    fn cron_day_fields_follow_unrestricted_and_restricted_semantics() {
        let monday = chrono::Utc.with_ymd_and_hms(2026, 10, 12, 0, 0, 0).unwrap();
        let tuesday = chrono::Utc.with_ymd_and_hms(2026, 10, 13, 0, 0, 0).unwrap();
        assert!(cron_day_matches("*", "1", monday));
        assert!(cron_day_matches("12", "*", monday));
        assert!(cron_day_matches("*/2", "1", tuesday));
    }

    #[test]
    fn computes_a_next_trigger_beyond_one_year() {
        let now = chrono::Utc::now();
        let next = next_trigger_at(None, Some("0 0 29 2 *"))
            .expect("a valid leap-day cron should have a future trigger");
        let parsed = chrono::DateTime::parse_from_rfc3339(&next)
            .expect("next trigger should be RFC3339")
            .with_timezone(&chrono::Utc);
        assert!(parsed > now);
        assert!(parsed <= now + chrono::Duration::days(5 * 366));
    }

    #[test]
    fn rejects_cron_expressions_without_five_utc_fields() {
        assert!(validate_cron_expr(Some("*/5 * * *")).is_err());
        assert!(validate_cron_expr(Some("*/5 * * * *")).is_ok());
        assert!(validate_cron_expr(Some("   ")).is_err());
    }

    #[test]
    fn rejects_out_of_range_and_zero_step_cron_fields() {
        assert!(validate_cron_expr(Some("61 * * * *")).is_err());
        assert!(validate_cron_expr(Some("*/0 * * * *")).is_err());
        assert!(validate_cron_expr(Some("1-0 * * * *")).is_err());
    }

    #[test]
    fn sanitizes_and_bounds_task_error_details() {
        let detail = sanitize_task_detail("Authorization: secret\napi_key=abc, remaining");
        assert!(!detail.contains("secret"));
        assert!(!detail.contains("abc"));
        assert_eq!(sanitize_task_detail(&"x".repeat(5_000)).len(), 4_000);
    }

    #[tokio::test]
    async fn decorates_root_and_backfill_tasks_with_effective_policy_and_capability() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "INSERT INTO managed_task_runs
             (task_key,trigger_kind,started_at,actual_started_at,finished_at,actual_finished_at,
              duration_ms,actual_duration_ms,status,error_detail)
             VALUES('retention_archive','manual','2026-10-03T00:00:00.000Z',
                    '2026-10-03T00:00:05.000Z','2026-10-03T00:00:07.000Z',
                    '2026-10-03T00:00:06.000Z',777,2,'failed','committed failure')",
        )
        .execute(&pool)
        .await
        .expect("seed latest execution summary");
        sqlx::query(
            "INSERT INTO managed_task_runs
             (task_key,trigger_kind,started_at,finished_at,duration_ms,status,error_detail)
             VALUES('upstream_account_maintenance','interval','2026-10-03T00:00:00.000Z',
                    '2026-10-03T00:00:07.000Z',777,'failed','legacy duration')",
        )
        .execute(&pool)
        .await
        .expect("seed legacy duration-only execution");
        let store = MaintenanceStore::from_pool(pool);

        let tasks = store.list_tasks().await.expect("list decorated tasks");
        assert_eq!(
            tasks.len(),
            MANAGED_TASKS.len() + STARTUP_BACKFILL_TASKS.len()
        );
        assert!(tasks.iter().all(|task| {
            !task.trigger_kinds.is_empty()
                && !task.effective_policy.is_empty()
                && !task.policy_source.is_empty()
        }));
        let status = tasks
            .iter()
            .find(|task| task.task_key == "system_status_snapshot")
            .expect("system status task");
        assert!(status.effective_policy.contains("55 秒"));
        assert!(status.schedule_editable);
        let child = tasks
            .iter()
            .find(|task| task.task_key == "startup_backfill.proxy_usage")
            .expect("backfill child task");
        assert_eq!(child.trigger_kinds, vec!["event", "interval"]);
        assert!(!child.schedule_editable);
        assert!(child.schedule_capability_reason.is_some());
        let retention = tasks
            .iter()
            .find(|task| task.task_key == "retention_archive")
            .expect("retention task");
        let latest = retention
            .last_execution
            .as_ref()
            .expect("latest execution summary");
        assert_eq!(latest.attempted_at, "2026-10-03T00:00:00.000Z");
        assert_eq!(
            latest.actual_started_at.as_deref(),
            Some("2026-10-03T00:00:05.000Z")
        );
        assert_eq!(latest.duration_ms, Some(2));
        assert_eq!(latest.result, "failed");
        assert_eq!(latest.reason.as_deref(), Some("committed failure"));
        let status_without_run = tasks
            .iter()
            .find(|task| task.task_key == "system_status_snapshot")
            .expect("system status task");
        assert!(status_without_run.last_execution.is_none());
        assert_eq!(
            status_without_run.execution_observation,
            "no recorded attempts"
        );
        let legacy = tasks
            .iter()
            .find(|task| task.task_key == "upstream_account_maintenance")
            .and_then(|task| task.last_execution.as_ref())
            .expect("legacy execution summary");
        assert_eq!(legacy.duration_ms, None);
        let legacy_detail = store
            .detail("upstream_account_maintenance")
            .await
            .expect("load legacy workload detail")
            .expect("legacy task detail");
        assert_eq!(
            legacy_detail
                .workload_trend
                .samples
                .iter()
                .find(|sample| sample.managed_run_id.is_some())
                .and_then(|sample| sample.duration_ms),
            None
        );
    }

    #[tokio::test]
    async fn repairs_legacy_dual_schedule_trigger_atomically_after_transient_failure() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance migration test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "UPDATE managed_tasks SET interval_secs=120, cron_expr='*/5 * * * *', next_trigger_at='2000-01-01T00:02:00.000Z' WHERE task_key='dashboard_runtime_projection_reconcile'",
        )
        .execute(&pool)
        .await
        .expect("seed legacy dual schedule");

        sqlx::query(
            "CREATE TRIGGER fail_legacy_schedule_trigger_repair
             BEFORE UPDATE OF next_trigger_at ON managed_tasks
             WHEN OLD.task_key='dashboard_runtime_projection_reconcile'
             BEGIN SELECT RAISE(ABORT, 'injected schedule repair failure'); END",
        )
        .execute(&pool)
        .await
        .expect("inject schedule repair failure");

        let error = ensure_schema(&pool)
            .await
            .expect_err("injected trigger update must abort schedule repair");
        assert!(
            error
                .to_string()
                .contains("injected schedule repair failure")
        );
        let unchanged = sqlx::query_as::<_, (Option<i64>, Option<String>)>(
            "SELECT interval_secs,next_trigger_at FROM managed_tasks WHERE task_key='dashboard_runtime_projection_reconcile'",
        )
        .fetch_one(&pool)
        .await
        .expect("load schedule after failed migration");
        assert_eq!(unchanged.0, Some(120));
        assert_eq!(unchanged.1.as_deref(), Some("2000-01-01T00:02:00.000Z"));

        sqlx::query("DROP TRIGGER fail_legacy_schedule_trigger_repair")
            .execute(&pool)
            .await
            .expect("remove injected schedule repair failure");

        ensure_schema(&pool).await.expect("repair legacy schedule");
        let row = sqlx::query_as::<_, (Option<i64>, Option<String>, Option<String>)>(
            "SELECT interval_secs,cron_expr,next_trigger_at FROM managed_tasks WHERE task_key='dashboard_runtime_projection_reconcile'",
        )
        .fetch_one(&pool)
        .await
        .expect("load repaired schedule");
        assert_eq!(row.0, None);
        assert_eq!(row.1.as_deref(), Some("*/5 * * * *"));
        assert_ne!(row.2.as_deref(), Some("2000-01-01T00:02:00.000Z"));
        assert!(row.2.is_some());
    }

    #[tokio::test]
    async fn preserves_enabled_when_clearing_unsupported_override_and_rejects_new_one() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "UPDATE managed_tasks SET enabled=1, interval_secs=120 WHERE task_key='summary_snapshot'",
        )
        .execute(&pool)
        .await
        .expect("seed legacy unsupported override");
        let store = MaintenanceStore::from_pool(pool);

        let error = store
            .update_control("summary_snapshot", None, Some(Some(180)), None)
            .await
            .expect_err("unsupported task should reject a new override");
        assert!(error.to_string().contains("does not support"));
        assert!(
            store
                .update_control("summary_snapshot", None, Some(None), Some(None))
                .await
                .expect("clearing an existing override should succeed")
        );
        let row = sqlx::query_as::<_, (bool, Option<i64>, Option<String>)>(
            "SELECT enabled,interval_secs,cron_expr FROM managed_tasks WHERE task_key='summary_snapshot'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load cleared task control");
        assert_eq!(row, (true, None, None));
    }

    #[tokio::test]
    async fn switching_supported_schedule_types_clears_the_previous_override() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance schedule test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);

        store
            .update_control(
                "dashboard_runtime_projection_reconcile",
                None,
                Some(Some(120)),
                None,
            )
            .await
            .expect("set interval override");
        store
            .update_control(
                "dashboard_runtime_projection_reconcile",
                None,
                None,
                Some(Some("*/5 * * * *")),
            )
            .await
            .expect("switch to cron override");
        let cron_state = sqlx::query_as::<_, (Option<i64>, Option<String>)>(
            "SELECT interval_secs,cron_expr FROM managed_tasks WHERE task_key='dashboard_runtime_projection_reconcile'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load cron override");
        assert_eq!(cron_state, (None, Some("*/5 * * * *".to_string())));

        store
            .update_control(
                "dashboard_runtime_projection_reconcile",
                None,
                Some(Some(180)),
                None,
            )
            .await
            .expect("switch back to interval override");
        let interval_state = sqlx::query_as::<_, (Option<i64>, Option<String>)>(
            "SELECT interval_secs,cron_expr FROM managed_tasks WHERE task_key='dashboard_runtime_projection_reconcile'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load interval override");
        assert_eq!(interval_state, (Some(180), None));
    }

    #[tokio::test]
    async fn recovers_all_incomplete_runs_on_restart_idempotently() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary) VALUES ('retention_archive','startup','2026-09-30T00:00:00.000Z','running','started')",
        )
        .execute(&pool)
        .await
        .expect("insert incomplete managed task run");
        let fresh_started_at = crate::format_utc_iso_millis(Utc::now());
        sqlx::query(
            "INSERT INTO managed_task_runs (task_key,trigger_kind,started_at,status,summary) VALUES ('forward_proxy_subscription_refresh','startup',?1,'running','started')",
        )
        .bind(&fresh_started_at)
        .execute(&pool)
        .await
        .expect("insert fresh managed task run");

        let store = MaintenanceStore::from_pool(pool);
        assert_eq!(store.recover_incomplete_runs().await.unwrap(), 2);
        let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT status,finished_at,error_detail FROM managed_task_runs WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load recovered managed task run");
        assert_eq!(row.0, "failed");
        assert!(row.1.is_some());
        assert_eq!(row.2.as_deref(), Some("服务重启时回收未完成运行"));
        let fresh_status = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT status,finished_at,error_detail FROM managed_task_runs WHERE task_key='forward_proxy_subscription_refresh'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load fresh managed task run");
        assert_eq!(fresh_status.0, "failed");
        assert!(fresh_status.1.is_some());
        assert_eq!(fresh_status.2.as_deref(), Some("服务重启时回收未完成运行"));
        assert_eq!(store.recover_incomplete_runs().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn legacy_state_migration_rolls_back_every_write_on_failure() {
        let business_pool = legacy_backfill_pool(1, 99).await;
        sqlx::query(
            "INSERT INTO system_task_runs (id,task_kind,trigger_kind,status,started_at)
             VALUES (1,'retention_archive','startup','success','2026-10-03T00:00:00.000Z')",
        )
        .execute(&business_pool)
        .await
        .expect("seed legacy run");

        let maintenance_pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&maintenance_pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&maintenance_pool)
            .await
            .expect("seed maintenance task registry");
        sqlx::query(
            "CREATE TRIGGER fail_legacy_progress_import
             BEFORE INSERT ON managed_task_progress
             BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END",
        )
        .execute(&maintenance_pool)
        .await
        .expect("install migration failure trigger");

        let store = MaintenanceStore::from_pool(maintenance_pool);
        assert!(store.migrate_legacy_state(&business_pool).await.is_err());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM managed_task_runs")
                .fetch_one(&store.pool)
                .await
                .expect("count runs after migration rollback"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM startup_backfill_progress")
                .fetch_one(&store.pool)
                .await
                .expect("count legacy checkpoints after migration rollback"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM managed_task_progress")
                .fetch_one(&store.pool)
                .await
                .expect("count progress snapshots after migration rollback"),
            0
        );
    }

    #[tokio::test]
    async fn rejects_run_now_for_disabled_scheduled_tasks_but_allows_manual_tasks() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);

        let error = store
            .request_run("startup_backfill.proxy_usage")
            .await
            .expect_err("disabled scheduled task should reject run-now");
        assert!(error.to_string().contains("task is disabled"));

        let run_id = store
            .request_run("raw_compression")
            .await
            .expect("manual task should allow run-now while disabled");
        assert!(run_id > 0);
    }

    #[tokio::test]
    async fn updates_canonical_prompt_cache_backfill_control() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);

        assert!(
            store
                .update_control(
                    "startup_backfill.prompt_cache_conversations_materialization",
                    Some(true),
                    None,
                    None,
                )
                .await
                .expect("enable canonical prompt-cache control")
        );
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='startup_backfill.prompt_cache_conversations_materialization'",
            )
            .fetch_one(&store.pool)
            .await
            .expect("load canonical prompt-cache control")
        );
    }

    #[tokio::test]
    async fn prompt_cache_control_generation_stops_new_steps_without_revoking_started_steps() {
        let control = std::sync::Arc::new(super::PromptCacheMaterializationControl::default());
        let enabled = control.initialize(true);
        let step = match control.begin_step(Some(enabled.generation)) {
            super::PromptCacheMaterializationStepAdmission::Started(step) => step,
            _ => panic!("enabled generation should admit a step"),
        };

        let disabled = control.publish_committed(false);
        assert_eq!(disabled.generation, enabled.generation + 1);
        assert_eq!(step.generation, enabled.generation);
        assert!(matches!(
            control.begin_step(Some(enabled.generation)),
            super::PromptCacheMaterializationStepAdmission::GenerationChanged
        ));
        assert!(matches!(
            control.begin_step(None),
            super::PromptCacheMaterializationStepAdmission::Disabled
        ));

        drop(step);
        let resumed = control.publish_committed(true);
        assert_eq!(resumed.generation, disabled.generation + 1);
        let repeated = control.publish_committed(true);
        assert_eq!(repeated.generation, resumed.generation);
    }

    #[tokio::test]
    async fn claim_retires_requested_runs_after_a_task_is_disabled() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);

        assert!(
            store
                .set_enabled("startup_backfill.proxy_usage", true)
                .await
                .expect("enable scheduled task")
        );
        store
            .request_run("startup_backfill.proxy_usage")
            .await
            .expect("queue enabled task run");
        assert!(
            store
                .set_enabled("startup_backfill.proxy_usage", false)
                .await
                .expect("disable scheduled task")
        );

        assert!(store.claim_requested_run().await.unwrap().is_none());
        let row = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status,summary FROM managed_task_runs WHERE task_key='startup_backfill.proxy_usage'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load retired requested run");
        assert_eq!(row.0, "failed");
        assert_eq!(row.1.as_deref(), Some("任务已停用，未执行"));
    }

    #[tokio::test]
    async fn applies_initial_task_defaults_only_once() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "UPDATE managed_tasks SET enabled=1 WHERE task_key IN ('startup_backfill','raw_compression')",
        )
        .execute(&pool)
        .await
        .expect("simulate pre-default task controls");
        sqlx::query(
            "INSERT INTO startup_backfill_progress (task_name,enabled) VALUES ('proxy_usage_tokens_v1',1)",
        )
        .execute(&pool)
        .await
        .expect("insert backfill control row");

        let store = MaintenanceStore::from_pool(pool);
        assert!(store.apply_initial_task_defaults().await.unwrap());
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='startup_backfill'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='raw_compression'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='startup_backfill.proxy_usage'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT enabled FROM startup_backfill_progress WHERE task_name='proxy_usage_tokens_v1'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap(),
            1
        );

        sqlx::query("UPDATE managed_tasks SET enabled=1 WHERE task_key='raw_compression'")
            .execute(&store.pool)
            .await
            .expect("simulate operator enablement");
        assert!(!store.apply_initial_task_defaults().await.unwrap());
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='raw_compression'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
    }

    async fn legacy_backfill_pool(enabled: i64, cursor_id: i64) -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect legacy business pool");
        sqlx::query(
            "CREATE TABLE system_task_runs (
                id INTEGER PRIMARY KEY, task_kind TEXT NOT NULL, trigger_kind TEXT NOT NULL,
                status TEXT NOT NULL, summary TEXT, detail TEXT, started_at TEXT NOT NULL,
                finished_at TEXT, duration_ms INTEGER
            )",
        )
        .execute(&pool)
        .await
        .expect("create legacy run table");
        sqlx::query(
            "CREATE TABLE startup_backfill_progress (
                task_name TEXT PRIMARY KEY, cursor_id INTEGER NOT NULL, next_run_after TEXT,
                zero_update_streak INTEGER NOT NULL, last_started_at TEXT, last_finished_at TEXT,
                last_scanned INTEGER NOT NULL, last_updated INTEGER NOT NULL, last_status TEXT NOT NULL,
                suspension_reason TEXT, next_probe_at TEXT, wake_generation INTEGER NOT NULL,
                enabled INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create legacy progress table");
        sqlx::query(
            "INSERT INTO startup_backfill_progress (
                task_name,cursor_id,zero_update_streak,last_scanned,last_updated,last_status,
                wake_generation,enabled
             ) VALUES (?, ?, 0, 0, 0, 'idle', 0, ?)",
        )
        .bind(crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION)
        .bind(cursor_id)
        .bind(enabled)
        .execute(&pool)
        .await
        .expect("insert legacy progress row");
        pool
    }

    #[tokio::test]
    async fn existing_maintenance_prompt_cache_control_and_checkpoint_win_over_legacy_state() {
        let business_pool = legacy_backfill_pool(0, 99).await;
        let maintenance_pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect existing maintenance pool");
        ensure_schema(&maintenance_pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&maintenance_pool)
            .await
            .expect("seed maintenance task registry");
        sqlx::query(
            "UPDATE managed_tasks SET enabled=1
             WHERE task_key='startup_backfill.prompt_cache_conversations_materialization'",
        )
        .execute(&maintenance_pool)
        .await
        .expect("preserve existing managed enablement");
        sqlx::query(
            "INSERT INTO startup_backfill_progress (
                task_name,cursor_id,next_run_after,last_status,suspension_reason,
                wake_generation,enabled
             ) VALUES (?,41,'2026-10-03T00:00:00.000Z','stats_page_pending',
                       'stats_page_pending',7,1)",
        )
        .bind(crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION)
        .execute(&maintenance_pool)
        .await
        .expect("seed existing scheduler checkpoint");
        super::record_prompt_cache_materialization_control_origin(&maintenance_pool)
            .await
            .expect("record existing maintenance control origin");

        let store = MaintenanceStore::from_pool(maintenance_pool);
        store
            .migrate_legacy_state(&business_pool)
            .await
            .expect("migrate legacy state without replacing checkpoint");
        assert!(store.apply_initial_task_defaults().await.unwrap());

        let enabled: bool = sqlx::query_scalar(
            "SELECT enabled FROM managed_tasks
             WHERE task_key='startup_backfill.prompt_cache_conversations_materialization'",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        let checkpoint: (i64, Option<String>, String, Option<String>, i64, i64) = sqlx::query_as(
            "SELECT cursor_id,next_run_after,last_status,suspension_reason,wake_generation,enabled
                 FROM startup_backfill_progress WHERE task_name=?",
        )
        .bind(crate::STARTUP_BACKFILL_TASK_PROMPT_CACHE_CONVERSATIONS_MATERIALIZATION)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert!(enabled);
        assert_eq!(checkpoint.0, 41);
        assert_eq!(checkpoint.1.as_deref(), Some("2026-10-03T00:00:00.000Z"));
        assert_eq!(checkpoint.2, "stats_page_pending");
        assert_eq!(checkpoint.3.as_deref(), Some("stats_page_pending"));
        assert_eq!(checkpoint.4, 7);
        assert_eq!(checkpoint.5, 1);
        assert!(!store.apply_initial_task_defaults().await.unwrap());
    }

    #[tokio::test]
    async fn legacy_only_prompt_cache_control_is_imported_once_into_maintenance() {
        let business_pool = legacy_backfill_pool(1, 99).await;
        let maintenance_pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect new maintenance pool");
        ensure_schema(&maintenance_pool)
            .await
            .expect("create maintenance schema");
        super::record_prompt_cache_materialization_control_origin(&maintenance_pool)
            .await
            .expect("record missing managed control origin");
        seed_tasks(&maintenance_pool)
            .await
            .expect("seed new maintenance task registry");

        let store = MaintenanceStore::from_pool(maintenance_pool);
        store
            .migrate_legacy_state(&business_pool)
            .await
            .expect("migrate old business task state");
        assert!(store.apply_initial_task_defaults().await.unwrap());

        let enabled: bool = sqlx::query_scalar(
            "SELECT enabled FROM managed_tasks
             WHERE task_key='startup_backfill.prompt_cache_conversations_materialization'",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert!(enabled);
        assert!(!store.apply_initial_task_defaults().await.unwrap());
    }

    #[tokio::test]
    async fn retention_default_schedule_is_observable_and_overrideable() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);

        assert!(store.apply_initial_task_defaults().await.unwrap());
        let task = store
            .detail("retention_archive")
            .await
            .unwrap()
            .expect("retention task detail");
        let schedule = task
            .task
            .effective_schedule
            .expect("default schedule should be published");
        assert_eq!(schedule.source, "default");
        assert_eq!(schedule.interval_secs, Some(3_600));
        assert!(schedule.next_trigger_at.is_some());

        assert!(
            store
                .set_schedule("retention_archive", Some(1_800), None)
                .await
                .unwrap()
        );
        let task = store
            .detail("retention_archive")
            .await
            .unwrap()
            .expect("overridden retention task detail");
        assert_eq!(
            task.task
                .effective_schedule
                .expect("override schedule should be published")
                .source,
            "override"
        );
    }

    #[tokio::test]
    async fn reconciles_legacy_backfill_enablement_after_defaults_marker_exists() {
        let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        sqlx::query(
            "INSERT INTO maintenance_metadata (key,value,updated_at) VALUES ('managed_task_defaults_v1','applied','2026-10-01T00:00:00.000Z')",
        )
        .execute(&pool)
        .await
        .expect("seed existing defaults marker");
        sqlx::query(
            "INSERT INTO startup_backfill_progress (task_name,enabled) VALUES ('proxy_usage_tokens_v1',1)",
        )
        .execute(&pool)
        .await
        .expect("insert legacy enabled backfill row");

        let store = MaintenanceStore::from_pool(pool);
        assert!(store.apply_initial_task_defaults().await.unwrap());
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT enabled FROM managed_tasks WHERE task_key='startup_backfill.proxy_usage'",
            )
            .fetch_one(&store.pool)
            .await
            .unwrap()
        );
        assert!(!store.apply_initial_task_defaults().await.unwrap());
    }

    #[tokio::test]
    async fn migrates_versioned_backfill_keys_and_preserves_interrupted_history() {
        let main_pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect legacy main test pool");
        sqlx::query(
            "CREATE TABLE system_task_runs (id INTEGER PRIMARY KEY, task_kind TEXT NOT NULL, trigger_kind TEXT NOT NULL, status TEXT NOT NULL, summary TEXT, detail TEXT, started_at TEXT NOT NULL, finished_at TEXT, duration_ms INTEGER)",
        )
        .execute(&main_pool)
        .await
        .expect("create legacy task history table");
        sqlx::query(
            "CREATE TABLE startup_backfill_progress (task_name TEXT PRIMARY KEY, cursor_id INTEGER NOT NULL, next_run_after TEXT, zero_update_streak INTEGER NOT NULL, last_started_at TEXT, last_finished_at TEXT, last_scanned INTEGER NOT NULL, last_updated INTEGER NOT NULL, last_status TEXT NOT NULL, suspension_reason TEXT, next_probe_at TEXT, wake_generation INTEGER NOT NULL, enabled INTEGER NOT NULL)",
        )
        .execute(&main_pool)
        .await
        .expect("create legacy backfill table");
        for id in 1..=3 {
            sqlx::query(
                "INSERT INTO system_task_runs (id,task_kind,trigger_kind,status,started_at) VALUES (?,?,?,?,?)",
            )
            .bind(id)
            .bind("retention_archive")
            .bind("startup")
            .bind("running")
            .bind(format!("2026-09-30T00:00:0{id}.000Z"))
            .execute(&main_pool)
            .await
            .expect("insert interrupted legacy run");
        }
        for (id, task_name) in [
            (4, "proxy_usage_tokens_v1"),
            (5, "proxy_cost_v1:catalog-version"),
        ] {
            sqlx::query(
                "INSERT INTO startup_backfill_progress (task_name,cursor_id,next_run_after,zero_update_streak,last_started_at,last_finished_at,last_scanned,last_updated,last_status,suspension_reason,next_probe_at,wake_generation,enabled) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
            )
            .bind(task_name)
            .bind(id)
            .bind(Option::<String>::None)
            .bind(0_i64)
            .bind(Option::<String>::None)
            .bind(Some("2026-09-30T00:00:00.000Z"))
            .bind(10_i64)
            .bind(id)
            .bind("ok")
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(0_i64)
            .bind(1_i64)
            .execute(&main_pool)
            .await
            .expect("insert versioned legacy progress");
        }

        let maintenance_pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect maintenance test pool");
        ensure_schema(&maintenance_pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&maintenance_pool)
            .await
            .expect("seed maintenance task registry");
        let store = MaintenanceStore::from_pool(maintenance_pool);

        store
            .migrate_legacy_state(&main_pool)
            .await
            .expect("migrate legacy state");

        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM managed_task_runs")
                .fetch_one(&store.pool)
                .await
                .expect("count migrated runs"),
            3
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM managed_task_runs WHERE status IN ('running','requested')",
            )
            .fetch_one(&store.pool)
            .await
            .expect("count active migrated runs"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM managed_task_progress")
                .fetch_one(&store.pool)
                .await
                .expect("count managed progress snapshots"),
            2
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM managed_task_progress WHERE task_key IN ('startup_backfill.proxy_usage','startup_backfill.proxy_cost')",
            )
            .fetch_one(&store.pool)
            .await
            .expect("count canonical progress snapshots"),
            2
        );
        let migrated_run = sqlx::query_as::<_, (String, Option<String>, Option<i64>)>(
            "SELECT status,finished_at,duration_ms FROM managed_task_runs ORDER BY id LIMIT 1",
        )
        .fetch_one(&store.pool)
        .await
        .expect("load migrated interrupted run");
        assert_eq!(migrated_run.0, "failed");
        assert!(migrated_run.1.is_some());
        assert_eq!(migrated_run.2, Some(0));
    }

    #[tokio::test]
    async fn retention_partial_run_schedules_catchup_and_disable_clears_it() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect retention catch-up fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);
        let run_id = store
            .begin_run(
                "retention_archive",
                "2026-10-01T00:00:00.000Z",
                "schedule",
                None,
            )
            .await
            .expect("begin retention run");
        store
            .finish_run_with_observation(
                run_id,
                "success",
                "2026-10-01T00:00:01.000Z",
                1_000,
                None,
                None,
                Some("partial"),
                Some("partial"),
                Some(&serde_json::json!({
                    "total": 10,
                    "invocationRowsArchived": 2,
                    "waitReason": "retention_work_budget"
                })),
            )
            .await
            .expect("finish retention run");
        let scheduled = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT next_catchup_at,catchup_reason FROM managed_tasks WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read catch-up schedule");
        assert!(scheduled.0.is_some());
        assert_eq!(scheduled.1.as_deref(), Some("retention_work_budget"));

        store
            .set_enabled("retention_archive", false)
            .await
            .expect("disable retention task");
        let cleared: (Option<String>, Option<String>) = sqlx::query_as(
            "SELECT next_catchup_at,catchup_reason FROM managed_tasks WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read cleared catch-up schedule");
        assert_eq!(cleared, (None, None));
    }

    #[tokio::test]
    async fn retention_completed_run_keeps_catchup_when_backlog_remains() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect retention completed catch-up fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);
        let run_id = store
            .begin_run(
                "retention_archive",
                "2026-10-01T00:00:00.000Z",
                "catchup",
                None,
            )
            .await
            .expect("begin retention run");
        store
            .finish_run_with_observation(
                run_id,
                "success",
                "2026-10-01T00:00:01.000Z",
                1_000,
                None,
                None,
                Some("completed"),
                Some("completed"),
                Some(&serde_json::json!({
                    "total": 10,
                    "invocationRowsArchived": 2,
                    "waitReason": null
                })),
            )
            .await
            .expect("finish retention run");
        let scheduled = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT next_catchup_at,catchup_reason FROM managed_tasks WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read retained catch-up schedule");
        assert!(scheduled.0.is_some());
        assert_eq!(scheduled.1.as_deref(), Some("backlog_remaining"));
    }

    #[tokio::test]
    async fn retention_unknown_backlog_keeps_catchup_after_claim() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect retention unknown backlog fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);
        sqlx::query("UPDATE managed_tasks SET next_catchup_at='2000-01-01T00:00:00.000Z' WHERE task_key='retention_archive'")
            .execute(&store.pool)
            .await
            .expect("seed due catch-up");
        assert_eq!(store.enqueue_due_runs().await.unwrap(), 1);
        let run_id: i64 = sqlx::query_scalar(
            "SELECT id FROM managed_task_runs WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        store
            .finish_run_with_observation(
                run_id, "success", "2026-10-01T00:00:01.000Z", 1_000,
                None, None, Some("partial"), Some("partial"),
                Some(&serde_json::json!({"total": null, "invocationRowsArchived": 2, "waitReason": null})),
            )
            .await
            .expect("finish catch-up with unknown backlog");
        let scheduled = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT next_catchup_at,catchup_reason FROM managed_tasks WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read unknown backlog catch-up");
        assert!(scheduled.0.is_some());
        assert_eq!(scheduled.1.as_deref(), Some("backlog_unknown"));
        store
            .update_retention_catchup_from_summary(
                Some("completed"),
                Some(0),
                0,
                None,
                "2026-10-01T00:00:02.000Z",
            )
            .await
            .expect("clear catch-up only after exact empty observation");
        let cleared: Option<String> = sqlx::query_scalar(
            "SELECT next_catchup_at FROM managed_tasks WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert!(cleared.is_none());
    }

    #[tokio::test]
    async fn clearing_retention_override_restores_default_schedule() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect retention default restore fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);
        store
            .apply_initial_task_defaults()
            .await
            .expect("apply default retention schedule");
        store
            .update_control("retention_archive", None, Some(Some(1_800)), None)
            .await
            .expect("set retention override");
        store
            .update_control("retention_archive", None, Some(None), Some(None))
            .await
            .expect("restore default retention schedule");

        let task = store
            .detail("retention_archive")
            .await
            .expect("load restored retention task")
            .expect("retention task exists");
        let schedule = task
            .task
            .effective_schedule
            .expect("default schedule remains observable");
        assert_eq!(schedule.source, "default");
        assert_eq!(schedule.interval_secs, Some(3_600));
        assert!(schedule.next_trigger_at.is_some());
    }

    #[tokio::test]
    async fn due_retention_catchup_is_enqueued_once_and_keeps_inspection_schedule() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect retention scheduler fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);
        let now = Utc::now();
        let inspection = format_utc_iso_millis(now + ChronoDuration::hours(1));
        let catchup = format_utc_iso_millis(now - ChronoDuration::seconds(1));
        sqlx::query(
            "UPDATE managed_tasks
             SET next_trigger_at=?, next_catchup_at=?, catchup_reason=?
             WHERE task_key='retention_archive'",
        )
        .bind(&inspection)
        .bind(&catchup)
        .bind("retention_work_budget")
        .execute(&store.pool)
        .await
        .expect("seed due catch-up");

        assert_eq!(store.enqueue_due_runs().await.expect("enqueue catch-up"), 1);
        let run = sqlx::query_as::<_, (String, String)>(
            "SELECT trigger_kind,status FROM managed_task_runs
             WHERE task_key='retention_archive' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read catch-up run");
        assert_eq!(run, ("catchup".to_string(), "requested".to_string()));
        let schedule = sqlx::query_as::<_, (Option<String>, Option<String>)>(
            "SELECT next_trigger_at,next_catchup_at FROM managed_tasks
             WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("read advanced schedule");
        assert_eq!(schedule.0, Some(inspection));
        assert_eq!(schedule.1, None);
        assert_eq!(
            store
                .enqueue_due_runs()
                .await
                .expect("avoid duplicate catch-up"),
            0
        );
    }

    #[tokio::test]
    async fn disabled_runtime_retention_config_does_not_enqueue_automatic_runs() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect retention config gate fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);
        sqlx::query(
            "UPDATE managed_tasks
             SET next_trigger_at='2000-01-01T00:00:00.000Z',
                 next_catchup_at='2000-01-01T00:00:00.000Z'
             WHERE task_key='retention_archive'",
        )
        .execute(&store.pool)
        .await
        .expect("seed due retention schedule");

        assert_eq!(
            store
                .enqueue_due_runs_with_retention_enabled(false)
                .await
                .expect("skip retention when runtime config is disabled"),
            0
        );
        let runs: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM managed_task_runs WHERE task_key='retention_archive'",
        )
        .fetch_one(&store.pool)
        .await
        .expect("count skipped retention runs");
        assert_eq!(runs, 0);
    }

    #[tokio::test]
    async fn retention_detail_returns_observed_and_missing_hourly_buckets() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect retention trend fixture");
        ensure_schema(&pool)
            .await
            .expect("create maintenance schema");
        seed_tasks(&pool).await.expect("seed maintenance tasks");
        let store = MaintenanceStore::from_pool(pool);
        let now = Utc::now();
        let bucket = floor_utc_hour(now);
        store
            .record_retention_backlog_observation(RetentionBacklogObservation {
                bucket_start: format_utc_iso_millis(bucket),
                observed_at: format_utc_iso_millis(now),
                invocation_count: 42,
                max_overdue_seconds: Some(3600),
                retention_days: 7,
                cutoff: "2026-09-24 00:00:00".to_string(),
                source_max_invocation_id: Some(100),
            })
            .await
            .expect("record hourly retention observation");
        let detail = store
            .detail("retention_archive")
            .await
            .expect("load retention detail")
            .expect("retention task exists");
        let trend = detail
            .retention_backlog_trend
            .expect("retention trend is present");
        assert_eq!(trend.len(), 7 * 24);
        assert!(
            trend
                .iter()
                .any(|point| point.state == "observed" && point.invocation_count == Some(42))
        );
        assert!(trend.iter().any(|point| point.state == "missing"));

        store
            .record_retention_backlog_observation(RetentionBacklogObservation {
                bucket_start: format_utc_iso_millis(bucket),
                observed_at: format_utc_iso_millis(now - ChronoDuration::seconds(1)),
                invocation_count: 7,
                max_overdue_seconds: Some(99),
                retention_days: 7,
                cutoff: "2026-09-24 00:00:00".to_string(),
                source_max_invocation_id: Some(99),
            })
            .await
            .expect("ignore stale hourly retention observation");
        store
            .record_retention_backlog_observation(RetentionBacklogObservation {
                bucket_start: format_utc_iso_millis(bucket),
                observed_at: format_utc_iso_millis(now + ChronoDuration::seconds(1)),
                invocation_count: 0,
                max_overdue_seconds: None,
                retention_days: 7,
                cutoff: "2026-09-24 00:00:00".to_string(),
                source_max_invocation_id: Some(101),
            })
            .await
            .expect("record latest empty hourly retention observation");
        let latest = sqlx::query_as::<_, (i64, Option<i64>, Option<i64>)>(
            "SELECT invocation_count,max_overdue_seconds,source_max_invocation_id
             FROM retention_backlog_hourly_observations WHERE bucket_start=?",
        )
        .bind(format_utc_iso_millis(bucket))
        .fetch_one(&store.pool)
        .await
        .expect("read latest hourly retention observation");
        assert_eq!(latest, (0, None, Some(101)));
    }
}
