use super::*;
mod raw_payload_inventory;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
pub(crate) use raw_payload_inventory::*;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tracing::{debug, warn};

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) const SYSTEM_STATUS_CACHE_TTL_SECS: u64 = 60;
// Refresh before the public 60-second cache ceiling and bound the durable scan so a successful
// maintenance pass cannot make the request-only memory snapshot temporarily unavailable.
const SYSTEM_STATUS_SNAPSHOT_REFRESH_LEAD: Duration = Duration::from_secs(5);
const SYSTEM_STATUS_SNAPSHOT_REFRESH_DEADLINE: Duration = Duration::from_secs(4);
const SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE: i64 = 128;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemStatusMetric {
    pub(crate) count: u64,
    pub(crate) bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemProjectionConsumerHealth {
    pub(crate) state: String,
    pub(crate) cursor_lag: i64,
    pub(crate) dirty_bucket_count: u64,
    pub(crate) pending_event_count: u64,
    pub(crate) last_flush_elapsed_ms: Option<u64>,
    pub(crate) last_flush_age_ms: Option<u64>,
    pub(crate) last_repair_scope: Option<String>,
    pub(crate) last_defer_reason: Option<String>,
    pub(crate) last_error_kind: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemProjectionHealth {
    pub(crate) terminal: SystemProjectionConsumerHealth,
    pub(crate) long_term: SystemProjectionConsumerHealth,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemRawMetricsHealth {
    pub(crate) state: String,
    pub(crate) inventory_cursor: i64,
    pub(crate) updated_age_ms: Option<u64>,
    pub(crate) physical_coverage: String,
}

impl Default for SystemRawMetricsHealth {
    fn default() -> Self {
        Self {
            state: "unknown".to_string(),
            inventory_cursor: 0,
            updated_age_ms: None,
            physical_coverage: RAW_METRICS_PHYSICAL_COVERAGE_UNKNOWN.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemRawCaptureHealth {
    pub(crate) state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    pub(crate) inventory_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) raw_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) available_bytes: Option<u64>,
    pub(crate) reserved_bytes: u64,
    pub(crate) raw_close_bytes: u64,
    pub(crate) raw_resume_bytes: u64,
    pub(crate) available_close_bytes: u64,
    pub(crate) available_resume_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expired_backlog_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) backlog_non_growing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemRuntimePressureProcess {
    pub(crate) rss_bytes: u64,
    pub(crate) rss_anon_bytes: u64,
    pub(crate) swap_bytes: u64,
    pub(crate) peak_rss_bytes: u64,
    pub(crate) threads: u64,
    pub(crate) managed_bytes: u64,
    pub(crate) unattributed_anon_bytes: u64,
    pub(crate) pressure_level: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemRuntimePressureAllocator {
    pub(crate) malloc_arena_max: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemRuntimePressureHealth {
    pub(crate) state: String,
    pub(crate) process: SystemRuntimePressureProcess,
    pub(crate) allocator: SystemRuntimePressureAllocator,
    pub(crate) writer_accounting: PendingQueueAccountingSnapshot,
    pub(crate) database_pressure: crate::db_pressure::DbPressureSnapshot,
    pub(crate) proxy_sqlite_write_coordinator:
        crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinatorSnapshot,
    pub(crate) dashboard_projection: RuntimeProjectionHealthSnapshot,
    pub(crate) delivery: DashboardDeliveryTopologyCounterSnapshot,
    pub(crate) dashboard_hot_topics: DashboardHotTopicsHealthSnapshot,
    pub(crate) request_pipeline: RequestPipelineHealthSnapshot,
    pub(crate) prompt_cache_projection: PromptCacheTopicProjectionHealthSnapshot,
    pub(crate) retention_write_health: RetentionWriteHealthSnapshot,
    pub(crate) retention_recovery: RetentionRecoveryHealthSnapshot,
    pub(crate) raw_orphan_sweep: RawOrphanSweepHealthSnapshot,
    pub(crate) raw_capture: SystemRawCaptureHealth,
    pub(crate) event_bus: RuntimeMutationBusHealth,
    pub(crate) backfill: StartupBackfillHealthSnapshot,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemStatusResponse {
    pub(crate) live_invocations_count: u64,
    pub(crate) success_count: u64,
    pub(crate) non_success_count: u64,
    pub(crate) completed_archive_batches_count: u64,
    pub(crate) archived_bodies: SystemStatusMetric,
    pub(crate) raw_bodies: SystemStatusMetric,
    pub(crate) request_raw_bodies: SystemStatusMetric,
    pub(crate) response_raw_bodies: SystemStatusMetric,
    pub(crate) database_bytes: u64,
    pub(crate) other_files_bytes: u64,
    pub(crate) projection_health: SystemProjectionHealth,
    pub(crate) raw_metrics_health: SystemRawMetricsHealth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) runtime_pressure_health: Option<SystemRuntimePressureHealth>,
    pub(crate) refreshed_at: String,
}

const RAW_METRICS_PHYSICAL_COVERAGE_PARTIAL: &str = "partial";
const RAW_METRICS_PHYSICAL_COVERAGE_UNKNOWN: &str = "unknown";

fn normalize_raw_metrics_state(state: &str) -> String {
    match state {
        "ready" | "preparing" | "deferred" | "error" | "unknown" => state.to_string(),
        "resetting" => "preparing".to_string(),
        _ => "unknown".to_string(),
    }
}

fn raw_metrics_physical_coverage(display_state: &str, inventory_state: &str) -> String {
    if display_state == "ready" && inventory_state == "ready" {
        RAW_METRICS_PHYSICAL_COVERAGE_PARTIAL.to_string()
    } else {
        RAW_METRICS_PHYSICAL_COVERAGE_UNKNOWN.to_string()
    }
}

fn raw_metrics_bytes_if_ready(
    display_state: &str,
    inventory_state: &str,
    bytes: i64,
) -> Option<u64> {
    (display_state == "ready" && inventory_state == "ready").then_some(bytes.max(0) as u64)
}

fn apply_raw_metrics_display_contract(response: &mut SystemStatusResponse) {
    if response.raw_metrics_health.state != "ready" {
        response.raw_bodies.bytes = None;
        response.request_raw_bodies.bytes = None;
        response.response_raw_bodies.bytes = None;
        response.raw_metrics_health.physical_coverage =
            RAW_METRICS_PHYSICAL_COVERAGE_UNKNOWN.to_string();
    }
}

fn runtime_pressure_state(
    accounting_error: bool,
    degraded_signal: bool,
    deferred_signal: bool,
) -> &'static str {
    if accounting_error {
        "accounting_error"
    } else if degraded_signal {
        "degraded"
    } else if deferred_signal {
        "deferred"
    } else {
        "healthy"
    }
}

pub(crate) async fn load_runtime_pressure_health(state: &AppState) -> SystemRuntimePressureHealth {
    let memory = state.memory_diagnostics.runtime_pressure_snapshot();
    let writer_accounting = state.sqlite_batch_writer.accounting_snapshot();
    let database_pressure = crate::db_pressure::global_db_pressure_gate().snapshot();
    let proxy_sqlite_write_coordinator =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .snapshot()
            .await;
    let active_subscriber_count = state
        .subscription_hub
        .dashboard_activity_live_subscriber_count()
        .await;
    let dashboard_projection = state
        .proxy_runtime_invocations
        .health_snapshot(active_subscriber_count);
    let delivery = state.subscription_hub.dashboard_topology_counters();
    let request_pipeline = state
        .proxy_runtime_invocations
        .request_pipeline_health_snapshot();
    let prompt_cache_projection = state
        .subscription_hub
        .prompt_cache_projection_health()
        .await;
    let retention_write_health = retention_write_health_snapshot();
    let retention_recovery = retention_recovery_health_snapshot();
    let raw_orphan_sweep = raw_orphan_sweep_health_snapshot();
    let raw_capture_snapshot = state.raw_capture_circuit.snapshot();
    let event_bus = state.subscription_hub.runtime_mutation_bus_health();
    let backfill = startup_backfill_health_snapshot();
    let terminal_projection = state.terminal_projection_hub.health();
    let long_term_projection = state.long_term_projection_runtime.lock().await.health();
    let projection_cadence_missed = [
        dashboard_projection.slice_counters.current,
        dashboard_projection.slice_counters.network,
        dashboard_projection.slice_counters.terminal,
    ]
    .into_iter()
    .any(|slice| slice.cadence_miss_count > 0);
    let delivery_degraded = state
        .subscription_hub
        .dashboard_delivery_has_degraded_signal();
    let prompt_cache_failed_or_stale = prompt_cache_projection.failed_or_stale_topic_count > 0;
    let prompt_cache_live_path_db_read = prompt_cache_projection.live_path_db_read_count > 0;
    let prompt_cache_bounded_cold_recovery =
        prompt_cache_projection.bounded_cold_recovery_topic_count > 0;
    let prompt_cache_pressure_deferred = prompt_cache_projection.pressure_deferred_topic_count > 0;
    let dashboard_hot_topics = state
        .subscription_hub
        .dashboard_hot_topic_health(dashboard_projection.slice_counters)
        .await;
    let projection_deferred = dashboard_projection.last_defer_reason.is_some()
        || terminal_projection.hard_limit_reason.is_some()
        || long_term_projection.last_defer_reason.is_some();
    let cursor_growth = terminal_projection.last_persisted_row_id
        > long_term_projection.cursor_row_id
        || (terminal_projection.timeseries_consumer_active
            && terminal_projection.last_persisted_row_id
                > terminal_projection.timeseries_cursor_row_id);
    let writer_pressure_active = writer_accounting.p2_deferred_age_ms > 0
        || database_pressure.pressure_cooldown_remaining_ms > 0
        || proxy_sqlite_write_coordinator.p1_waiter_count > 0
        || proxy_sqlite_write_coordinator.interactive_waiter_count > 0
        || proxy_sqlite_write_coordinator.p2_waiter_count > 0
        || proxy_sqlite_write_coordinator.maintenance_waiter_count > 0;
    let state = runtime_pressure_state(
        writer_accounting.state == "degraded",
        memory.pressure_level != "normal"
            || dashboard_projection.state == "degraded"
            || projection_cadence_missed
            || delivery_degraded
            || dashboard_hot_topics.state == "degraded"
            || prompt_cache_failed_or_stale
            || prompt_cache_live_path_db_read
            || retention_write_health.state == "degraded"
            || retention_recovery.state == "degraded"
            || event_bus.state == "degraded"
            || backfill.state == "degraded",
        projection_deferred
            || cursor_growth
            || writer_pressure_active
            || prompt_cache_pressure_deferred
            || prompt_cache_bounded_cold_recovery
            || retention_write_health.state == "deferred"
            || retention_recovery.state == "deferred"
            || dashboard_hot_topics.state == "deferred"
            || backfill.state == "deferred",
    )
    .to_string();
    SystemRuntimePressureHealth {
        state,
        process: SystemRuntimePressureProcess {
            rss_bytes: memory.process.rss_bytes,
            rss_anon_bytes: memory.process.rss_anon_bytes,
            swap_bytes: memory.process.swap_bytes,
            peak_rss_bytes: memory.process.peak_rss_bytes,
            threads: memory.process.threads,
            managed_bytes: memory.managed_bytes,
            unattributed_anon_bytes: memory.unattributed_anon_bytes,
            pressure_level: memory.pressure_level,
        },
        allocator: SystemRuntimePressureAllocator {
            malloc_arena_max: memory.malloc_arena_max,
        },
        writer_accounting,
        database_pressure,
        proxy_sqlite_write_coordinator,
        dashboard_projection,
        delivery,
        dashboard_hot_topics,
        request_pipeline,
        prompt_cache_projection,
        retention_write_health,
        retention_recovery,
        raw_orphan_sweep,
        raw_capture: SystemRawCaptureHealth {
            state: raw_capture_snapshot.state,
            reason: raw_capture_snapshot.reason,
            inventory_state: raw_capture_snapshot.inventory_state,
            raw_bytes: raw_capture_snapshot.physical_raw_bytes,
            available_bytes: raw_capture_snapshot.available_bytes,
            reserved_bytes: raw_capture_snapshot.reserved_bytes,
            raw_close_bytes: RAW_CAPTURE_CLOSE_BYTES,
            raw_resume_bytes: RAW_CAPTURE_RESUME_BYTES,
            available_close_bytes: RAW_CAPTURE_CLOSE_AVAILABLE_BYTES,
            available_resume_bytes: RAW_CAPTURE_RESUME_AVAILABLE_BYTES,
            expired_backlog_count: raw_capture_snapshot.expired_backlog_count,
            backlog_non_growing: raw_capture_snapshot.backlog_non_growing,
            updated_at: raw_capture_snapshot.updated_at,
        },
        event_bus,
        backfill,
    }
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemTaskRunResponse {
    pub(crate) id: i64,
    pub(crate) task_kind: String,
    pub(crate) trigger_kind: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
    #[serde(serialize_with = "serialize_local_or_utc_to_utc_iso")]
    pub(crate) started_at: String,
    #[serde(
        serialize_with = "serialize_opt_local_or_utc_to_utc_iso",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) finished_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemTaskRunsListResponse {
    pub(crate) items: Vec<SystemTaskRunResponse>,
    pub(crate) total: u64,
    pub(crate) page: u32,
    pub(crate) page_size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemTaskRunsQuery {
    pub(crate) task_kind: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) started_at_from: Option<String>,
    pub(crate) started_at_to: Option<String>,
    pub(crate) limit: Option<u32>,
    pub(crate) page: Option<u32>,
    pub(crate) page_size: Option<u32>,
    pub(crate) cursor: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SystemTaskRunCursor {
    started_at: String,
    id: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct SystemTaskRunHandle {
    pub(crate) id: i64,
    pub(crate) task_kind: SystemTaskKind,
    pub(crate) trigger_kind: String,
    pub(crate) started_at: Instant,
    pub(crate) observation: Option<crate::TaskExecutionObservation>,
}

#[derive(Debug, FromRow)]
pub(crate) struct SystemTaskRunRow {
    id: i64,
    task_kind: String,
    trigger_kind: String,
    status: String,
    summary: Option<String>,
    detail: Option<String>,
    started_at: String,
    finished_at: Option<String>,
    duration_ms: Option<i64>,
}

#[derive(Debug, Default, FromRow)]
pub(crate) struct SystemInvocationStatusAggRow {
    live_invocations_count: Option<i64>,
    success_count: Option<i64>,
    non_success_count: Option<i64>,
}

#[derive(Debug, Default, FromRow)]
pub(crate) struct SystemArchiveAggRow {
    completed_archive_batches_count: Option<i64>,
    archived_count: Option<i64>,
}

#[derive(Debug, Default, FromRow)]
pub(crate) struct SystemRawBodyPathRow {
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
}

#[derive(Debug, Default, FromRow)]
struct SystemRawPayloadMetricsRow {
    inventory_state: String,
    inventory_cursor: i64,
    link_inventory_cursor: i64,
    inventory_recheck_cursor: String,
    inventory_recheck_active: i64,
    raw_count: i64,
    raw_bytes: i64,
    raw_overflow_spool_bytes: i64,
    request_raw_count: i64,
    request_raw_bytes: i64,
    response_raw_count: i64,
    response_raw_bytes: i64,
    circuit_state: String,
    circuit_reason: Option<String>,
    circuit_available_bytes: Option<i64>,
    circuit_expired_backlog_count: Option<i64>,
    circuit_backlog_non_growing: Option<i64>,
    circuit_updated_at: Option<String>,
    circuit_recovery_pending: i64,
    updated_at: String,
}

#[derive(Debug, FromRow)]
struct SystemRawPayloadInventoryRow {
    id: i64,
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
}

#[derive(Debug, FromRow)]
struct SystemRawPayloadBlobLinkRow {
    id: i64,
    raw_path: String,
    raw_role: String,
}

#[derive(Debug, FromRow)]
struct SystemRawPayloadInventoryPathRow {
    byte_size: i64,
    request_seen: i64,
    response_seen: i64,
}

#[derive(Debug, FromRow)]
struct SystemRawPayloadInventoryRecheckRow {
    raw_path: String,
    byte_size: i64,
    request_seen: i64,
    response_seen: i64,
}

impl From<SystemTaskRunRow> for SystemTaskRunResponse {
    fn from(value: SystemTaskRunRow) -> Self {
        Self {
            id: value.id,
            task_kind: value.task_kind,
            trigger_kind: value.trigger_kind,
            status: value.status,
            summary: value.summary,
            detail: value.detail,
            started_at: value.started_at,
            finished_at: value.finished_at,
            duration_ms: value.duration_ms,
        }
    }
}

pub(crate) fn parse_system_task_run_bound(
    raw: Option<&str>,
    field_name: &str,
) -> Result<Option<String>, ApiError> {
    let Some(raw_value) = normalize_query_text(raw) else {
        return Ok(None);
    };
    let parsed = DateTime::parse_from_rfc3339(&raw_value)
        .with_context(|| format!("invalid {field_name}: {raw_value}"))
        .map_err(ApiError::bad_request)?
        .with_timezone(&Utc);
    Ok(Some(format_utc_iso_millis(parsed)))
}

fn parse_system_task_run_cursor(
    raw: Option<&str>,
) -> Result<Option<SystemTaskRunCursor>, ApiError> {
    let Some(raw_value) = normalize_query_text(raw) else {
        return Ok(None);
    };
    let decoded = URL_SAFE_NO_PAD
        .decode(raw_value)
        .context("invalid cursor encoding")
        .map_err(ApiError::bad_request)?;
    let cursor = serde_json::from_slice::<SystemTaskRunCursor>(&decoded)
        .context("invalid cursor payload")
        .map_err(ApiError::bad_request)?;
    if cursor.id < 1 {
        return Err(ApiError::bad_request(anyhow!("invalid cursor id")));
    }
    let started_at = DateTime::parse_from_rfc3339(&cursor.started_at)
        .map(|value| format_utc_iso_millis(value.with_timezone(&Utc)))
        .unwrap_or(cursor.started_at);
    Ok(Some(SystemTaskRunCursor {
        started_at,
        id: cursor.id,
    }))
}

fn encode_system_task_run_cursor(row: &SystemTaskRunRow) -> Result<String, ApiError> {
    let started_at = DateTime::parse_from_rfc3339(&row.started_at)
        .map(|value| format_utc_iso_millis(value.with_timezone(&Utc)))
        .unwrap_or_else(|_| row.started_at.clone());
    let payload = serde_json::to_vec(&SystemTaskRunCursor {
        started_at,
        id: row.id,
    })
    .context("failed to encode system task cursor")
    .map_err(ApiError::Internal)?;
    Ok(URL_SAFE_NO_PAD.encode(payload))
}

pub(crate) fn count_file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

struct SystemStatusFilesystemScan {
    cancellation: CancellationToken,
    deadline: Instant,
    #[cfg(test)]
    checkpoint: Option<Arc<dyn Fn() + Send + Sync>>,
    #[cfg(test)]
    blocking_operation: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl SystemStatusFilesystemScan {
    fn new(cancellation: CancellationToken, deadline: Instant) -> Self {
        Self {
            cancellation,
            deadline,
            #[cfg(test)]
            checkpoint: None,
            #[cfg(test)]
            blocking_operation: None,
        }
    }

    #[cfg(test)]
    fn with_test_checkpoint(
        cancellation: CancellationToken,
        deadline: Instant,
        checkpoint: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self {
            cancellation,
            deadline,
            checkpoint: Some(checkpoint),
            blocking_operation: None,
        }
    }

    #[cfg(test)]
    fn with_test_blocking_operation(
        mut self,
        blocking_operation: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        self.blocking_operation = Some(blocking_operation);
        self
    }

    fn check(&self) -> Result<()> {
        #[cfg(test)]
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint();
        }
        if self.cancellation.is_cancelled() {
            bail!("system status filesystem scan cancelled");
        }
        if Instant::now() >= self.deadline {
            bail!("system status filesystem scan exceeded its deadline");
        }
        Ok(())
    }

    fn before_filesystem_operation(&self) {
        #[cfg(test)]
        if let Some(blocking_operation) = &self.blocking_operation {
            blocking_operation();
        }
    }
}

#[derive(Debug)]
struct SystemStatusFilesystemBytes {
    archive_bytes: u64,
    database_bytes: u64,
    other_files_bytes: u64,
}

struct SystemStatusFilesystemScanInputs {
    archive_paths: Vec<String>,
    config: AppConfig,
    archive_dir: PathBuf,
    raw_dir: PathBuf,
}

fn count_file_size_with_scan(path: &Path, scan: &SystemStatusFilesystemScan) -> Result<u64> {
    scan.check()?;
    scan.before_filesystem_operation();
    Ok(fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
}

pub(crate) fn add_existing_raw_payload_bytes(
    raw_path: &str,
    fallback_root: Option<&Path>,
    seen_paths: &mut HashSet<PathBuf>,
    metric: &mut SystemStatusMetric,
) {
    let Some(candidate) = resolved_raw_path_read_candidates(raw_path, fallback_root)
        .into_iter()
        .find(|candidate| candidate.exists())
    else {
        return;
    };
    if !seen_paths.insert(candidate.clone()) {
        return;
    }
    metric.count = metric.count.saturating_add(1);
    metric.bytes = Some(
        metric
            .bytes
            .unwrap_or_default()
            .saturating_add(count_file_size(&candidate)),
    );
}

pub(crate) fn collect_existing_raw_payload_metrics(
    rows: &[SystemRawBodyPathRow],
    fallback_root: Option<&Path>,
) -> (SystemStatusMetric, SystemStatusMetric, SystemStatusMetric) {
    let mut total_seen_paths = HashSet::new();
    let mut request_seen_paths = HashSet::new();
    let mut response_seen_paths = HashSet::new();
    let mut total = SystemStatusMetric::default();
    let mut request = SystemStatusMetric::default();
    let mut response = SystemStatusMetric::default();

    for row in rows {
        if let Some(raw_path) = row.request_raw_path.as_deref() {
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut request_seen_paths,
                &mut request,
            );
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut total_seen_paths,
                &mut total,
            );
        }
        if let Some(raw_path) = row.response_raw_path.as_deref() {
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut response_seen_paths,
                &mut response,
            );
            add_existing_raw_payload_bytes(
                raw_path,
                fallback_root,
                &mut total_seen_paths,
                &mut total,
            );
        }
    }

    (total, request, response)
}

fn count_database_bytes_with_scan(
    db_path: &Path,
    scan: &SystemStatusFilesystemScan,
) -> Result<u64> {
    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
    Ok(count_file_size_with_scan(db_path, scan)?
        .saturating_add(count_file_size_with_scan(&wal_path, scan)?)
        .saturating_add(count_file_size_with_scan(&shm_path, scan)?))
}

fn sum_directory_bytes_with_scan(root: &Path, scan: &SystemStatusFilesystemScan) -> Result<u64> {
    let mut total = 0_u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        scan.check()?;
        scan.before_filesystem_operation();
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            scan.check()?;
            let child = entry.path();
            scan.check()?;
            scan.before_filesystem_operation();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(child),
                Ok(kind) if kind.is_file() => {
                    scan.check()?;
                    scan.before_filesystem_operation();
                    total =
                        total.saturating_add(entry.metadata().map(|meta| meta.len()).unwrap_or(0));
                }
                _ => {}
            }
        }
    }
    Ok(total)
}

fn sum_path_bytes_with_scan(path: &Path, scan: &SystemStatusFilesystemScan) -> Result<u64> {
    scan.check()?;
    scan.before_filesystem_operation();
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(metadata.len()),
        Ok(metadata) if metadata.is_dir() => sum_directory_bytes_with_scan(path, scan),
        _ => Ok(0),
    }
}

#[derive(Debug)]
struct SystemStatusFilesystemScanPermit {
    in_flight: Arc<AtomicBool>,
    abandoned: Arc<AtomicBool>,
}

impl SystemStatusFilesystemScanPermit {
    fn try_acquire(cache: &SystemStatusCacheState) -> Result<(Self, Arc<AtomicBool>)> {
        let in_flight = cache.filesystem_scan_in_flight.clone();
        if in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            debug!(
                filesystem_scan_in_flight = true,
                "system status filesystem scan deferred while an earlier scan is still running"
            );
            bail!("system status filesystem scan is already in flight");
        }
        let abandoned = Arc::new(AtomicBool::new(false));
        Ok((
            Self {
                in_flight,
                abandoned: abandoned.clone(),
            },
            abandoned,
        ))
    }
}

impl Drop for SystemStatusFilesystemScanPermit {
    fn drop(&mut self) {
        self.in_flight.store(false, Ordering::Release);
        if self.abandoned.load(Ordering::Acquire) {
            debug!("abandoned system status filesystem scan exited; scan admission released");
        }
    }
}

fn compute_other_files_bytes_with_scan(
    config: &AppConfig,
    archive_dir: &Path,
    raw_dir: &Path,
    scan: &SystemStatusFilesystemScan,
) -> Result<u64> {
    let db_path = &config.database_path;
    let db_wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let db_shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
    let mut seen = HashSet::new();

    // Keep "other files" scoped to runtime-owned storage that does not already
    // have a dedicated metric on the system status page.
    let mut total = 0_u64;
    for path in [config.xray_runtime_dir.clone()] {
        scan.check()?;
        if path.as_os_str().is_empty() || !seen.insert(path.clone()) {
            continue;
        }
        let candidate = path.as_path();
        if candidate != db_path
            && candidate != db_wal_path.as_path()
            && candidate != db_shm_path.as_path()
            && candidate != archive_dir
            && candidate != raw_dir
        {
            total = total.saturating_add(sum_path_bytes_with_scan(&path, scan)?);
        }
    }
    Ok(total)
}

fn collect_system_status_filesystem_bytes(
    inputs: SystemStatusFilesystemScanInputs,
    scan: SystemStatusFilesystemScan,
) -> Result<SystemStatusFilesystemBytes> {
    let SystemStatusFilesystemScanInputs {
        archive_paths,
        config,
        archive_dir,
        raw_dir,
    } = inputs;
    let mut seen_paths = HashSet::new();
    let mut archive_bytes = 0_u64;
    for path in archive_paths {
        scan.check()?;
        if seen_paths.insert(path.clone()) {
            archive_bytes =
                archive_bytes.saturating_add(count_file_size_with_scan(Path::new(&path), &scan)?);
        }
    }

    Ok(SystemStatusFilesystemBytes {
        archive_bytes,
        database_bytes: count_database_bytes_with_scan(&config.database_path, &scan)?,
        other_files_bytes: compute_other_files_bytes_with_scan(
            &config,
            &archive_dir,
            &raw_dir,
            &scan,
        )?,
    })
}

async fn collect_system_status_filesystem_bytes_in_blocking_task(
    archive_paths: Vec<String>,
    config: AppConfig,
    archive_dir: PathBuf,
    raw_dir: PathBuf,
    cache: &Arc<Mutex<SystemStatusCacheState>>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<SystemStatusFilesystemBytes> {
    let scan = SystemStatusFilesystemScan::new(cancellation.clone(), deadline);
    collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
        SystemStatusFilesystemScanInputs {
            archive_paths,
            config,
            archive_dir,
            raw_dir,
        },
        cache,
        cancellation,
        deadline,
        scan,
    )
    .await
}

async fn collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
    inputs: SystemStatusFilesystemScanInputs,
    cache: &Arc<Mutex<SystemStatusCacheState>>,
    cancellation: &CancellationToken,
    deadline: Instant,
    scan: SystemStatusFilesystemScan,
) -> Result<SystemStatusFilesystemBytes> {
    ensure_system_status_filesystem_scan_active(cancellation, deadline)?;
    let (permit, abandoned) = {
        let cache =
            await_system_status_refresh_operation(cancellation, async { Ok(cache.lock().await) })
                .await?;
        ensure_system_status_filesystem_scan_active(cancellation, deadline)?;
        SystemStatusFilesystemScanPermit::try_acquire(&cache)?
    };
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        collect_system_status_filesystem_bytes(inputs, scan)
    });

    await_system_status_filesystem_scan_task(task, cancellation, deadline, &abandoned).await
}

fn ensure_system_status_filesystem_scan_active(
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<()> {
    if cancellation.is_cancelled() {
        bail!("system status filesystem scan cancelled");
    }
    if Instant::now() >= deadline {
        cancellation.cancel();
        bail!("system status filesystem scan exceeded its deadline");
    }
    Ok(())
}

async fn await_system_status_filesystem_scan_task(
    mut task: tokio::task::JoinHandle<Result<SystemStatusFilesystemBytes>>,
    cancellation: &CancellationToken,
    deadline: Instant,
    abandoned: &Arc<AtomicBool>,
) -> Result<SystemStatusFilesystemBytes> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            abandoned.store(true, Ordering::Release);
            // Abort only prevents queued work. A started blocking filesystem call remains alive,
            // holding its permit until it returns so retries cannot accumulate blocked workers.
            task.abort();
            drop(task);
            warn!(
                abandon_reason = "cancelled",
                "system status filesystem scan abandoned; retaining its scan admission until it exits"
            );
            bail!("system status filesystem scan cancelled");
        }
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
            cancellation.cancel();
            abandoned.store(true, Ordering::Release);
            // See the cancellation branch: this does not claim to interrupt started OS I/O.
            task.abort();
            drop(task);
            warn!(
                abandon_reason = "deadline",
                "system status filesystem scan abandoned; retaining its scan admission until it exits"
            );
            bail!("system status filesystem scan exceeded its deadline");
        }
        result = &mut task => {
            let filesystem_bytes = result
                .map_err(|error| anyhow!("system status filesystem scan task failed: {error}"))??;
            // A blocking syscall may return after the timer wakes. Do not let its successful
            // result escape toward snapshot publication once this refresh is no longer valid.
            ensure_system_status_filesystem_scan_active(cancellation, deadline)?;
            Ok(filesystem_bytes)
        }
    }
}

async fn record_system_raw_payload_inventory_path(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    raw_path: &str,
    byte_size: i64,
    request_seen: bool,
    response_seen: bool,
) -> Result<(i64, i64, i64, i64, i64, i64)> {
    let existing = sqlx::query_as::<_, SystemRawPayloadInventoryPathRow>(
        r#"
        SELECT byte_size, request_seen, response_seen
        FROM system_raw_payload_inventory_paths
        WHERE raw_path = ?1
        "#,
    )
    .bind(raw_path)
    .fetch_optional(tx.as_mut())
    .await?;

    let Some(existing) = existing else {
        sqlx::query(
            r#"
            INSERT INTO system_raw_payload_inventory_paths (
                raw_path, byte_size, request_seen, response_seen
            )
            VALUES (?1, ?2, ?3, ?4)
            "#,
        )
        .bind(raw_path)
        .bind(byte_size)
        .bind(i64::from(request_seen))
        .bind(i64::from(response_seen))
        .execute(tx.as_mut())
        .await?;
        return Ok((
            1,
            byte_size,
            i64::from(request_seen),
            if request_seen { byte_size } else { 0 },
            i64::from(response_seen),
            if response_seen { byte_size } else { 0 },
        ));
    };

    let request_added = request_seen && existing.request_seen == 0;
    let response_added = response_seen && existing.response_seen == 0;
    if request_added || response_added {
        sqlx::query(
            r#"
            UPDATE system_raw_payload_inventory_paths
            SET request_seen = MAX(request_seen, ?2), response_seen = MAX(response_seen, ?3)
            WHERE raw_path = ?1
            "#,
        )
        .bind(raw_path)
        .bind(i64::from(request_seen))
        .bind(i64::from(response_seen))
        .execute(tx.as_mut())
        .await?;
    }
    Ok((
        0,
        0,
        i64::from(request_added),
        if request_added { existing.byte_size } else { 0 },
        i64::from(response_added),
        if response_added {
            existing.byte_size
        } else {
            0
        },
    ))
}

async fn await_system_status_refresh_operation<T>(
    cancellation: &CancellationToken,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => bail!("system status snapshot refresh cancelled"),
        result = operation => result,
    }
}

async fn load_system_status_snapshot_uncached(
    state: &AppState,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(SystemStatusResponse, String)> {
    let runtime_pressure_health = await_system_status_refresh_operation(cancellation, async {
        Ok(load_runtime_pressure_health(state).await)
    })
    .await?;
    let invocation_status = await_system_status_refresh_operation(cancellation, async {
        Ok(sqlx::query_as::<_, SystemInvocationStatusAggRow>(
        r#"
        SELECT
            COUNT(*) AS live_invocations_count,
            COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'warning_success') THEN 1 ELSE 0 END), 0) AS success_count,
            COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) NOT IN ('success', 'warning_success') THEN 1 ELSE 0 END), 0) AS non_success_count
        FROM codex_invocations
        "#,
    )
    .fetch_one(&state.pool)
    .await?)
    })
    .await?;

    let archived = await_system_status_refresh_operation(cancellation, async {
        Ok(sqlx::query_as::<_, SystemArchiveAggRow>(
            r#"
        SELECT
            COUNT(*) AS completed_archive_batches_count,
            COALESCE(SUM(row_count), 0) AS archived_count
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
        "#,
        )
        .fetch_one(&state.pool)
        .await?)
    })
    .await?;

    let raw_metrics = await_system_status_refresh_operation(cancellation, async {
        Ok(sqlx::query_as::<_, SystemRawPayloadMetricsRow>(
        "SELECT inventory_state, inventory_cursor, link_inventory_cursor, inventory_recheck_cursor, inventory_recheck_active, raw_count, raw_bytes, raw_overflow_spool_bytes, request_raw_count, request_raw_bytes, response_raw_count, response_raw_bytes, circuit_state, circuit_reason, circuit_available_bytes, circuit_expired_backlog_count, circuit_backlog_non_growing, circuit_updated_at, circuit_recovery_pending, updated_at FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(&state.pool)
    .await?)
    })
    .await?;
    let raw_metrics_state = await_system_status_refresh_operation(cancellation, async {
        Ok(state
            .system_status_cache
            .lock()
            .await
            .raw_metrics_health_override
            .clone()
            .map(|state| normalize_raw_metrics_state(&state))
            .unwrap_or_else(|| normalize_raw_metrics_state(&raw_metrics.inventory_state)))
    })
    .await?;

    let archive_dir = resolved_archive_dir(&state.config);
    let raw_dir = state.config.resolved_proxy_raw_dir();
    let archived_paths = await_system_status_refresh_operation(cancellation, async {
        Ok(sqlx::query_scalar::<_, String>(
            r#"
        SELECT file_path
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
        "#,
        )
        .fetch_all(&state.pool)
        .await?)
    })
    .await?;
    let filesystem_bytes = collect_system_status_filesystem_bytes_in_blocking_task(
        archived_paths,
        state.config.clone(),
        archive_dir,
        raw_dir,
        &state.system_status_cache,
        cancellation,
        deadline,
    )
    .await?;
    let terminal_health = state.terminal_projection_hub.health();
    let long_term_health = await_system_status_refresh_operation(cancellation, async {
        Ok(state.long_term_projection_runtime.lock().await.health())
    })
    .await?;
    let runtime_record_count = state.proxy_runtime_invocations.runtime_record_count() as u64;
    debug!(
        db_invocation_row_count = invocation_status.live_invocations_count.unwrap_or(0).max(0),
        runtime_record_count,
        "system status invocation counts keep database rows separate from runtime memory records"
    );

    Ok((
        SystemStatusResponse {
            live_invocations_count: invocation_status.live_invocations_count.unwrap_or(0).max(0)
                as u64,
            success_count: invocation_status.success_count.unwrap_or(0).max(0) as u64,
            non_success_count: invocation_status.non_success_count.unwrap_or(0).max(0) as u64,
            completed_archive_batches_count: archived
                .completed_archive_batches_count
                .unwrap_or(0)
                .max(0) as u64,
            archived_bodies: SystemStatusMetric {
                count: archived.archived_count.unwrap_or(0).max(0) as u64,
                bytes: Some(filesystem_bytes.archive_bytes),
            },
            raw_bodies: SystemStatusMetric {
                count: raw_metrics.raw_count.max(0) as u64,
                bytes: raw_metrics_bytes_if_ready(
                    &raw_metrics_state,
                    &raw_metrics.inventory_state,
                    raw_metrics.raw_bytes,
                ),
            },
            request_raw_bodies: SystemStatusMetric {
                count: raw_metrics.request_raw_count.max(0) as u64,
                bytes: raw_metrics_bytes_if_ready(
                    &raw_metrics_state,
                    &raw_metrics.inventory_state,
                    raw_metrics.request_raw_bytes,
                ),
            },
            response_raw_bodies: SystemStatusMetric {
                count: raw_metrics.response_raw_count.max(0) as u64,
                bytes: raw_metrics_bytes_if_ready(
                    &raw_metrics_state,
                    &raw_metrics.inventory_state,
                    raw_metrics.response_raw_bytes,
                ),
            },
            database_bytes: filesystem_bytes.database_bytes,
            other_files_bytes: filesystem_bytes.other_files_bytes,
            projection_health: SystemProjectionHealth {
                terminal: SystemProjectionConsumerHealth {
                    state: if terminal_health.dirty_last_good {
                        "dirty_last_good".to_string()
                    } else {
                        "healthy".to_string()
                    },
                    cursor_lag: terminal_health
                        .last_persisted_row_id
                        .saturating_sub(terminal_health.long_term_cursor_row_id),
                    dirty_bucket_count: 0,
                    pending_event_count: terminal_health.pending_event_count as u64,
                    last_flush_elapsed_ms: None,
                    last_flush_age_ms: terminal_health.last_ack_age_ms,
                    last_repair_scope: None,
                    last_defer_reason: terminal_health.hard_limit_reason.map(str::to_string),
                    last_error_kind: None,
                },
                long_term: SystemProjectionConsumerHealth {
                    state: long_term_health.state,
                    cursor_lag: terminal_health
                        .last_persisted_row_id
                        .saturating_sub(long_term_health.cursor_row_id),
                    dirty_bucket_count: long_term_health.dirty_bucket_count as u64,
                    pending_event_count: long_term_health.pending_event_count as u64,
                    last_flush_elapsed_ms: long_term_health.last_flush_elapsed_ms,
                    last_flush_age_ms: long_term_health.last_flush_age_ms,
                    last_repair_scope: long_term_health.last_repair_scope,
                    last_defer_reason: long_term_health.last_defer_reason,
                    last_error_kind: long_term_health.last_error_kind,
                },
            },
            raw_metrics_health: SystemRawMetricsHealth {
                state: raw_metrics_state.clone(),
                inventory_cursor: raw_metrics.inventory_cursor,
                updated_age_ms: None,
                physical_coverage: raw_metrics_physical_coverage(
                    &raw_metrics_state,
                    &raw_metrics.inventory_state,
                ),
            },
            runtime_pressure_health: Some(runtime_pressure_health),
            refreshed_at: format_utc_iso(Utc::now()),
        },
        raw_metrics.inventory_state,
    ))
}

pub(crate) async fn load_system_status_uncached(state: &AppState) -> Result<SystemStatusResponse> {
    let cancellation = state.shutdown.child_token();
    Ok(load_system_status_snapshot_uncached(
        state,
        &cancellation,
        Instant::now() + SYSTEM_STATUS_SNAPSHOT_REFRESH_DEADLINE,
    )
    .await?
    .0)
}

pub(crate) async fn load_system_status_cached(state: &AppState) -> Result<SystemStatusResponse> {
    if let Some((response, snapshot_age)) = state
        .system_status_cache
        .lock()
        .await
        .latest
        .as_ref()
        .map(|entry| (entry.response.clone(), entry.cached_at.elapsed()))
        .filter(|(_, snapshot_age)| {
            *snapshot_age <= Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS)
        })
    {
        let snapshot_age_ms = snapshot_age.as_millis() as u64;
        debug!(
            metrics_source = "system_status_memory_snapshot",
            snapshot_age_ms, "serving system status from last-good memory snapshot"
        );
        let mut response = response;
        apply_raw_metrics_display_contract(&mut response);
        return Ok(response);
    }

    Err(anyhow!("system status snapshot is unavailable or stale"))
}

pub(crate) async fn invalidate_system_status_cache(state: &AppState) {
    // A failed background refresh must never turn a previously good response into an empty
    // request-side cache miss. The maintainer will refresh this last-good entry on its cadence.
    let cache = state.system_status_cache.lock().await;
    debug!(
        has_last_good = cache.latest.is_some(),
        "system status snapshot marked for background refresh"
    );
}

pub(crate) async fn hydrate_system_status_snapshot(state: &AppState) -> Result<()> {
    refresh_system_status_snapshot_with_deadline(state).await
}

async fn refresh_system_status_snapshot_with_deadline(state: &AppState) -> Result<()> {
    let cancellation = state.shutdown.child_token();
    let deadline = Instant::now() + SYSTEM_STATUS_SNAPSHOT_REFRESH_DEADLINE;
    let refresh = refresh_system_status_snapshot_with_cancellation(state, &cancellation, deadline);
    tokio::pin!(refresh);

    tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            cancellation.cancel();
            let _ = refresh.await;
            bail!("system status snapshot refresh cancelled");
        }
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
            cancellation.cancel();
            let _ = refresh.await;
            bail!("system status snapshot refresh exceeded its deadline");
        }
        result = &mut refresh => result,
    }
}

async fn refresh_system_status_snapshot(state: &AppState) -> Result<()> {
    let cancellation = state.shutdown.child_token();
    refresh_system_status_snapshot_with_cancellation(
        state,
        &cancellation,
        Instant::now() + SYSTEM_STATUS_SNAPSHOT_REFRESH_DEADLINE,
    )
    .await
}

#[derive(Debug)]
struct SystemStatusSnapshotRefreshFlight {
    cache: Arc<Mutex<SystemStatusCacheState>>,
}

impl SystemStatusSnapshotRefreshFlight {
    async fn begin(state: &AppState, cancellation: &CancellationToken) -> Result<Self> {
        let mut cache = await_system_status_refresh_operation(cancellation, async {
            Ok(state.system_status_cache.lock().await)
        })
        .await?;
        if cache.in_flight.is_some() {
            bail!("system status snapshot refresh is already in flight");
        }
        let (signal, _receiver) = watch::channel(false);
        cache.in_flight = Some(signal);
        Ok(Self {
            cache: state.system_status_cache.clone(),
        })
    }

    async fn finish(self) {
        let mut cache = self.cache.lock().await;
        if let Some(signal) = cache.in_flight.take() {
            let _ = signal.send(true);
        }
    }
}

async fn refresh_system_status_snapshot_with_cancellation(
    state: &AppState,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<()> {
    let flight = SystemStatusSnapshotRefreshFlight::begin(state, cancellation).await?;
    let result = async {
        let (response, raw_metrics_inventory_state) =
            load_system_status_snapshot_uncached(state, cancellation, deadline).await?;
        publish_system_status_snapshot(
            state,
            cancellation,
            deadline,
            response,
            raw_metrics_inventory_state,
        )
        .await
    }
    .await;
    flight.finish().await;
    result
}

fn ensure_system_status_snapshot_refresh_active(
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<()> {
    if cancellation.is_cancelled() {
        bail!("system status snapshot refresh cancelled");
    }
    if Instant::now() >= deadline {
        cancellation.cancel();
        bail!("system status snapshot refresh exceeded its deadline");
    }
    Ok(())
}

async fn publish_system_status_snapshot(
    state: &AppState,
    cancellation: &CancellationToken,
    deadline: Instant,
    mut response: SystemStatusResponse,
    raw_metrics_inventory_state: String,
) -> Result<()> {
    await_system_status_refresh_operation(cancellation, async {
        let mut cache = state.system_status_cache.lock().await;
        // The cache lock can be delayed past the refresh deadline. Recheck while holding it so a
        // completed filesystem scan cannot publish a stale snapshot.
        ensure_system_status_snapshot_refresh_active(cancellation, deadline)?;
        if let Some(override_state) = cache.raw_metrics_health_override.as_deref() {
            let display_state = normalize_raw_metrics_state(override_state);
            response.raw_metrics_health.state = display_state.clone();
            response.raw_metrics_health.physical_coverage =
                raw_metrics_physical_coverage(&display_state, &raw_metrics_inventory_state);
        }
        cache.latest = Some(SystemStatusCacheEntry {
            cached_at: Instant::now(),
            response,
            raw_metrics_inventory_state,
        });
        debug!(
            metrics_source = "system_status_memory_snapshot",
            cache_ttl_ms = SYSTEM_STATUS_CACHE_TTL_SECS * 1_000,
            "system status background snapshot refresh completed"
        );
        Ok(())
    })
    .await
}

fn system_status_snapshot_refresh_delay() -> Duration {
    Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS)
        .checked_sub(SYSTEM_STATUS_SNAPSHOT_REFRESH_LEAD)
        .expect("system status refresh lead must be shorter than the cache TTL")
}

fn system_status_snapshot_refresh_cadence_period() -> Duration {
    // Reapply the lead after every publication. A TTL-sized period only protects the first
    // scan and eventually schedules later refreshes at a last-good entry's expiry boundary.
    system_status_snapshot_refresh_delay()
}

pub(crate) fn spawn_system_status_snapshot_maintenance(state: Arc<AppState>) {
    tokio::spawn(async move {
        // Startup has already completed the first durable hydration before this producer is
        // spawned. Keep every bounded scan ahead of the public TTL rather than only advancing the
        // first tick; a 60-second period would eventually start a scan at the prior snapshot's
        // expiry boundary when a preceding scan completed after its scheduled tick.
        let mut cadence = tokio::time::interval_at(
            tokio::time::Instant::now() + system_status_snapshot_refresh_delay(),
            system_status_snapshot_refresh_cadence_period(),
        );
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = cadence.tick() => {}
            }
            if crate::maintenance_store::legacy_worker_should_skip("system_status_snapshot").await {
                continue;
            }
            let Some(_execution_lease) =
                crate::maintenance_store::try_acquire_task_execution("system_status_snapshot")
            else {
                continue;
            };
            let _observation = crate::TaskExecutionObservation::begin(
                "system_status_snapshot",
                &crate::maintenance_store::task_title_for_observation("system_status_snapshot"),
                "interval",
                crate::maintenance_store::task_execution_class("system_status_snapshot"),
                "processing",
            );
            if let Err(error) = refresh_system_status_snapshot_with_deadline(state.as_ref()).await {
                warn!(
                    ?error,
                    "system status background refresh failed; retaining last-good snapshot"
                );
            }
        }
    });
}

pub(crate) async fn begin_system_task_run(
    _pool: &Pool<Sqlite>,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    let started_at = format_utc_iso_millis(Utc::now());
    let trigger_kind = trigger_kind.into();
    let id = {
        #[cfg(test)]
        {
            let stored_task_kind = match task_kind {
                SystemTaskKind::HourlyRollupBootstrap => "hourly_rollup_bootstrap",
                _ => task_kind.as_str(),
            };
            sqlx::query_scalar::<_, i64>(
                r#"
                INSERT INTO system_task_runs (
                    task_kind,
                    trigger_kind,
                    status,
                    summary,
                    started_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5)
                RETURNING id
                "#,
            )
            .bind(stored_task_kind)
            .bind(&trigger_kind)
            .bind(SystemTaskStatus::Running.as_str())
            .bind(summary)
            .bind(&started_at)
            .fetch_one(_pool)
            .await?
        }
        #[cfg(not(test))]
        {
            let store = crate::maintenance_store::global().ok_or_else(|| {
                anyhow!("maintenance database unavailable; task run is not recorded")
            })?;
            store
                .begin_run(
                    task_kind.as_str(),
                    &started_at,
                    &trigger_kind,
                    summary.as_deref(),
                )
                .await?
        }
    };

    Ok(SystemTaskRunHandle {
        id,
        task_kind,
        trigger_kind: trigger_kind.clone(),
        started_at: Instant::now(),
        observation: None,
    })
}

/// Record a task start through the isolated maintenance database.
pub(crate) async fn begin_system_task_run_nonblocking(
    _database_url: &str,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    let trigger_kind = trigger_kind.into();
    let store = crate::maintenance_store::global()
        .ok_or_else(|| anyhow!("maintenance database unavailable; task run is not recorded"))?;
    let started_at = format_utc_iso_millis(Utc::now());
    let id = store
        .begin_run(
            task_kind.as_str(),
            &started_at,
            &trigger_kind,
            summary.as_deref(),
        )
        .await?;

    Ok(SystemTaskRunHandle {
        id,
        task_kind,
        trigger_kind: trigger_kind.clone(),
        started_at: Instant::now(),
        observation: None,
    })
}

pub(crate) async fn begin_system_task_run_admitted(
    state: &AppState,
    write_class: crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let _write_permit = coordinator.acquire(write_class).await;
    begin_system_task_run(&state.pool, task_kind, trigger_kind, summary).await
}

pub(crate) async fn try_begin_system_task_run_with_admission(
    state: &AppState,
    write_class: crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<Option<SystemTaskRunHandle>> {
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let Some(_write_permit) = coordinator.try_acquire(write_class) else {
        return Ok(None);
    };
    begin_system_task_run(&state.pool, task_kind, trigger_kind, summary)
        .await
        .map(Some)
}

pub(crate) async fn finish_system_task_run(
    pool: &Pool<Sqlite>,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) -> bool {
    if let Some(observation) = handle.observation.as_ref() {
        observation.finish();
    }
    let finished_at = format_utc_iso_millis(Utc::now());
    let duration_ms = handle
        .started_at
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    #[cfg(test)]
    {
        if let Err(error) = sqlx::query(
            r#"
            UPDATE system_task_runs
            SET status = ?1,
                summary = COALESCE(?2, summary),
                detail = ?3,
                finished_at = ?4,
                duration_ms = ?5
            WHERE id = ?6
            "#,
        )
        .bind(status.as_str())
        .bind(summary)
        .bind(detail)
        .bind(&finished_at)
        .bind(duration_ms)
        .bind(handle.id)
        .execute(pool)
        .await
        {
            warn!(
                task_kind = handle.task_kind.as_str(),
                trigger_kind = %handle.trigger_kind,
                error = %error,
                "failed to finalize system task run"
            );
            false
        } else {
            true
        }
    }
    #[cfg(not(test))]
    {
        let _ = pool;
        let Some(store) = crate::maintenance_store::global() else {
            warn!(
                task_kind = handle.task_kind.as_str(),
                trigger_kind = %handle.trigger_kind,
                "maintenance database unavailable; task run finish is stale"
            );
            return false;
        };
        store
            .finish_run(
                handle.id,
                status.as_str(),
                &finished_at,
                duration_ms,
                summary.as_deref(),
                detail.as_deref(),
            )
            .await
            .is_ok()
    }
}

pub(crate) async fn finish_system_task_run_batched(
    state: &AppState,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) {
    if !finish_system_task_run_reliably(state, None, handle, status, summary, detail).await {
        warn!(
            task_kind = handle.task_kind.as_str(),
            trigger_kind = %handle.trigger_kind,
            "failed to durably finalize system task run after bounded retries"
        );
    }
}

const SYSTEM_TASK_FINISH_RETRY_ATTEMPTS: usize = 4;
const SYSTEM_TASK_FINISH_RETRY_INTERVAL: Duration = Duration::from_millis(50);
const SYSTEM_TASK_FINISH_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(250);

pub(crate) async fn finish_system_task_run_reliably(
    state: &AppState,
    cancel: Option<&CancellationToken>,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) -> bool {
    if let Some(observation) = handle.observation.as_ref() {
        observation.finish();
    }
    #[cfg(test)]
    {
        let _ = cancel;
        if try_enqueue_system_task_run_finish(
            state,
            handle,
            status,
            summary.clone(),
            detail.clone(),
        ) {
            return true;
        }
        return finish_system_task_run(&state.pool, handle, status, summary, detail).await;
    }
    #[cfg(not(test))]
    {
        let Some(store) = crate::maintenance_store::global().cloned() else {
            warn!(
                task_kind = handle.task_kind.as_str(),
                trigger_kind = %handle.trigger_kind,
                "maintenance database unavailable; task run finish is not persisted"
            );
            return false;
        };
        let finished_at = format_utc_iso_millis(Utc::now());
        let duration_ms = handle
            .started_at
            .elapsed()
            .as_millis()
            .min(i64::MAX as u128) as i64;
        let recovery_finish = BatchedSystemTaskFinish {
            run_id: handle.id,
            task_kind: handle.task_kind,
            trigger_kind: handle.trigger_kind.clone(),
            status,
            summary: summary.clone(),
            detail: detail.clone(),
            finished_at: finished_at.clone(),
            duration_ms,
        };
        let status_text = status.as_str();
        let mut last_error = None;
        for attempt in 0..=SYSTEM_TASK_FINISH_RETRY_ATTEMPTS {
            let finish_result = if let Some(cancel) = cancel {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => None,
                    result = tokio::time::timeout(
                        SYSTEM_TASK_FINISH_ATTEMPT_TIMEOUT,
                        store.finish_run(
                            handle.id,
                            status_text,
                            &finished_at,
                            duration_ms,
                            summary.as_deref(),
                            detail.as_deref(),
                        ),
                    ) => Some(result),
                }
            } else {
                Some(
                    tokio::time::timeout(
                        SYSTEM_TASK_FINISH_ATTEMPT_TIMEOUT,
                        store.finish_run(
                            handle.id,
                            status_text,
                            &finished_at,
                            duration_ms,
                            summary.as_deref(),
                            detail.as_deref(),
                        ),
                    )
                    .await,
                )
            };
            let Some(finish_result) = finish_result else {
                break;
            };
            match finish_result {
                Ok(result) => match result {
                    Ok(()) => return true,
                    Err(error) => {
                        warn!(
                            task_kind = handle.task_kind.as_str(),
                            trigger_kind = %handle.trigger_kind,
                            attempt,
                            error = %error,
                            "failed to finalize task history in maintenance database"
                        );
                        last_error = Some(error);
                    }
                },
                Err(error) => {
                    let error = anyhow!(
                        "maintenance database task-history finish timed out after {} ms: {error}",
                        SYSTEM_TASK_FINISH_ATTEMPT_TIMEOUT.as_millis()
                    );
                    warn!(
                        task_kind = handle.task_kind.as_str(),
                        trigger_kind = %handle.trigger_kind,
                        attempt,
                        error = %error,
                        "timed out finalizing task history in maintenance database"
                    );
                    last_error = Some(error);
                }
            }
            if attempt < SYSTEM_TASK_FINISH_RETRY_ATTEMPTS {
                if let Some(cancel) = cancel {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(SYSTEM_TASK_FINISH_RETRY_INTERVAL) => {}
                    }
                } else {
                    tokio::time::sleep(SYSTEM_TASK_FINISH_RETRY_INTERVAL).await;
                }
            }
        }
        if let Some(error) = last_error {
            warn!(
                task_kind = handle.task_kind.as_str(),
                trigger_kind = %handle.trigger_kind,
                error = %error,
                "task-history finish exceeded bounded maintenance database retries"
            );
        }
        let persisted = state.sqlite_batch_writer.quarantine_system_task_finish(
            &recovery_finish,
            "task-history finish exceeded bounded maintenance database retries",
        );
        if !persisted {
            warn!(
                task_kind = handle.task_kind.as_str(),
                trigger_kind = %handle.trigger_kind,
                "failed to persist task-history finish recovery record"
            );
        }
        persisted
    }
}

#[cfg(test)]
pub(crate) fn try_enqueue_system_task_run_finish(
    state: &AppState,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) -> bool {
    let finished_at = format_utc_iso_millis(Utc::now());
    let duration_ms = handle
        .started_at
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    state
        .sqlite_batch_writer
        .enqueue(SqliteBatchWrite::SystemTaskFinish(
            BatchedSystemTaskFinish {
                run_id: handle.id,
                task_kind: handle.task_kind,
                trigger_kind: handle.trigger_kind.clone(),
                status,
                summary,
                detail,
                finished_at,
                duration_ms,
            },
        ))
}

pub(crate) async fn fetch_system_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SystemStatusResponse>, ApiError> {
    Ok(Json(
        load_system_status_cached(state.as_ref())
            .await
            .map_err(ApiError::unavailable)?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheMaterializationControlRequest {
    enabled: bool,
}

pub(crate) async fn fetch_prompt_cache_materialization_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PromptCacheConversationMaterializationStatus>, ApiError> {
    Ok(Json(
        load_prompt_cache_conversation_materialization_status(&state.pool)
            .await
            .map_err(ApiError::from)?,
    ))
}

pub(crate) async fn update_prompt_cache_materialization_control(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PromptCacheMaterializationControlRequest>,
) -> Result<Json<PromptCacheConversationMaterializationStatus>, ApiError> {
    set_startup_backfill_task_enabled(
        &state.pool,
        StartupBackfillTask::PromptCacheConversationsMaterialization,
        payload.enabled,
    )
    .await
    .map_err(ApiError::from)?;
    fetch_prompt_cache_materialization_status(State(state)).await
}

pub(crate) async fn list_system_task_runs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SystemTaskRunsQuery>,
) -> Result<Json<SystemTaskRunsListResponse>, ApiError> {
    let started_at_from =
        parse_system_task_run_bound(query.started_at_from.as_deref(), "startedAtFrom")?;
    let started_at_to = parse_system_task_run_bound(query.started_at_to.as_deref(), "startedAtTo")?;
    let cursor = parse_system_task_run_cursor(query.cursor.as_deref())?;
    if cursor.is_some() && query.page.unwrap_or(1) > 1 {
        return Err(ApiError::bad_request(anyhow!(
            "cursor cannot be combined with page greater than 1"
        )));
    }
    let page_size = query
        .page_size
        .unwrap_or(query.limit.unwrap_or(20))
        .clamp(1, 100);
    let page = query.page.unwrap_or(1).max(1);
    let limit = i64::from(page_size);
    let offset = i64::from(page.saturating_sub(1)) * limit;
    let maintenance_store = crate::maintenance_store::global();
    let run_pool = maintenance_store
        .map(|store| &store.pool)
        .unwrap_or(&state.pool);
    let run_source = if maintenance_store.is_some() {
        "(SELECT id, task_key AS task_kind, trigger_kind, status, summary, error_detail AS detail, started_at, finished_at, duration_ms FROM managed_task_runs)"
    } else {
        "(SELECT id, task_kind, trigger_kind, status, summary, detail, started_at, finished_at, duration_ms FROM system_task_runs)"
    };
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, task_kind, trigger_kind, status, summary, detail, started_at, finished_at, duration_ms FROM ",
    );
    builder.push(run_source).push(" WHERE 1 = 1");
    if let Some(task_kind) = query
        .task_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder.push(" AND task_kind = ").push_bind(task_kind);
    }
    if let Some(status) = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder.push(" AND status = ").push_bind(status);
    }
    if started_at_from.is_some() || started_at_to.is_some() {
        builder.push(" AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) = started_at");
    }
    if let Some(started_at_from) = started_at_from.as_deref() {
        builder
            .push(" AND started_at >= ")
            .push_bind(started_at_from);
    }
    if let Some(started_at_to) = started_at_to.as_deref() {
        builder.push(" AND started_at <= ").push_bind(started_at_to);
    }
    if let Some(cursor) = cursor.as_ref() {
        builder
            .push(" AND (started_at < ")
            .push_bind(&cursor.started_at)
            .push(" OR (started_at = ")
            .push_bind(&cursor.started_at)
            .push(" AND id < ")
            .push_bind(cursor.id)
            .push("))");
    }
    builder
        .push(" ORDER BY started_at DESC, id DESC LIMIT ")
        .push_bind(limit + 1)
        .push(" OFFSET ")
        .push_bind(offset);
    let mut rows = builder
        .build_query_as::<SystemTaskRunRow>()
        .fetch_all(run_pool)
        .await?;

    let mut count_builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) as total FROM ");
    count_builder.push(run_source).push(" WHERE 1 = 1");
    if let Some(task_kind) = query
        .task_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        count_builder.push(" AND task_kind = ").push_bind(task_kind);
    }
    if let Some(status) = query
        .status
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        count_builder.push(" AND status = ").push_bind(status);
    }
    if started_at_from.is_some() || started_at_to.is_some() {
        count_builder.push(" AND strftime('%Y-%m-%dT%H:%M:%fZ', started_at) = started_at");
    }
    if let Some(started_at_from) = started_at_from.as_deref() {
        count_builder
            .push(" AND started_at >= ")
            .push_bind(started_at_from);
    }
    if let Some(started_at_to) = started_at_to.as_deref() {
        count_builder
            .push(" AND started_at <= ")
            .push_bind(started_at_to);
    }
    let total = count_builder
        .build_query_scalar::<i64>()
        .fetch_one(run_pool)
        .await?;

    let has_next_page = rows.len() > page_size as usize;
    if has_next_page {
        rows.pop();
    }
    let next_cursor = has_next_page
        .then(|| rows.last().map(encode_system_task_run_cursor))
        .flatten()
        .transpose()?;

    Ok(Json(SystemTaskRunsListResponse {
        items: rows.into_iter().map(Into::into).collect(),
        total: total.max(0) as u64,
        page,
        page_size,
        next_cursor,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTaskControlRequest {
    pub(crate) enabled: Option<bool>,
    // Nested options preserve the difference between an omitted schedule field
    // and an explicit null used to clear the current schedule.
    #[serde(default, deserialize_with = "crate::deserialize_optional_field")]
    pub(crate) interval_secs: crate::OptionalField<i64>,
    #[serde(default, deserialize_with = "crate::deserialize_optional_field")]
    pub(crate) cron_expr: crate::OptionalField<String>,
}

pub(crate) async fn list_managed_tasks(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::maintenance_store::ManagedTask>>, ApiError> {
    let Some(store) = crate::maintenance_store::global() else {
        return Err(ApiError::unavailable(anyhow!(
            "maintenance database unavailable"
        )));
    };
    store.list_tasks().await.map(Json).map_err(ApiError::from)
}

pub(crate) async fn get_managed_task_runtime(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<crate::TaskRuntimeSnapshot>, ApiError> {
    crate::task_runtime_snapshot()
        .map(Json)
        .map_err(ApiError::unavailable)
}

pub(crate) async fn get_managed_task(
    State(state): State<Arc<AppState>>,
    AxumPath(task_key): AxumPath<String>,
) -> Result<Json<crate::maintenance_store::ManagedTaskDetail>, ApiError> {
    let Some(store) = crate::maintenance_store::global() else {
        return Err(ApiError::unavailable(anyhow!(
            "maintenance database unavailable"
        )));
    };
    let mut detail = store
        .detail(&task_key)
        .await
        .map_err(ApiError::from)?
        .map(Json)
        .ok_or_else(|| ApiError::bad_request(anyhow!("managed task not found")))?;
    detail.0.performance = state
        .performance_telemetry
        .task_run_summary(&task_key)
        .await;
    Ok(Json(detail.0))
}

pub(crate) async fn update_managed_task(
    State(state): State<Arc<AppState>>,
    AxumPath(task_key): AxumPath<String>,
    Json(request): Json<ManagedTaskControlRequest>,
) -> Result<Json<crate::maintenance_store::ManagedTaskDetail>, ApiError> {
    let Some(store) = crate::maintenance_store::global() else {
        return Err(ApiError::unavailable(anyhow!(
            "maintenance database unavailable"
        )));
    };
    let ManagedTaskControlRequest {
        enabled,
        interval_secs,
        cron_expr,
    } = request;
    let interval_secs = match interval_secs {
        crate::OptionalField::Missing => None,
        crate::OptionalField::Null => Some(None),
        crate::OptionalField::Value(value) => Some(Some(value)),
    };
    let cron_expr = match cron_expr {
        crate::OptionalField::Missing => None,
        crate::OptionalField::Null => Some(None),
        crate::OptionalField::Value(value) => Some(Some(value)),
    };
    let cron_expr = cron_expr.as_ref().map(|value| value.as_deref());
    let previous_startup_control = if enabled.is_some() && task_key.starts_with("startup_backfill.")
    {
        store
            .detail(&task_key)
            .await
            .map_err(ApiError::from)?
            .map(|detail| detail.task)
    } else {
        None
    };
    if !store
        .update_control(&task_key, enabled, interval_secs, cron_expr)
        .await
        .map_err(ApiError::bad_request)?
    {
        return Err(ApiError::bad_request(anyhow!("managed task not found")));
    }
    if let (Some(enabled), Some(task_name)) = (enabled, task_key.strip_prefix("startup_backfill."))
        && let Some(task) = crate::StartupBackfillTask::from_managed_key(task_name)
        && let Err(progress_error) =
            crate::set_startup_backfill_progress_enabled(&state.pool, task, enabled).await
    {
        if let Some(previous) = previous_startup_control.as_ref()
            && let Err(rollback_error) = store.restore_control_state(previous).await
        {
            return Err(ApiError::from(anyhow!(
                "startup backfill control update failed: {progress_error}; rollback failed: {rollback_error}"
            )));
        }
        return Err(ApiError::from(anyhow!(
            "startup backfill control update failed: {progress_error}"
        )));
    }
    get_managed_task(State(state), AxumPath(task_key)).await
}

pub(crate) async fn run_managed_task_now(
    State(state): State<Arc<AppState>>,
    AxumPath(task_key): AxumPath<String>,
) -> Result<Json<crate::maintenance_store::ManagedTaskDetail>, ApiError> {
    let Some(store) = crate::maintenance_store::global() else {
        return Err(ApiError::unavailable(anyhow!(
            "maintenance database unavailable"
        )));
    };
    store
        .request_run(&task_key)
        .await
        .map_err(ApiError::conflict)?;
    get_managed_task(State(state), AxumPath(task_key)).await
}

pub(crate) fn summarize_retention_run_for_system_task(
    summary: &RetentionRunSummary,
) -> (String, String) {
    let brief = format!(
        "compressed={} archived_invocations={} released_prompt_cache_conversations={} pruned_details={} model_routes_pruned={} task_runs_pruned={} orphan_raw_removed={}",
        summary.raw_files_compressed,
        summary.invocation_rows_archived,
        summary.prompt_cache_conversations_released,
        summary.invocation_details_pruned,
        summary.model_route_rows_pruned,
        summary.system_task_run_rows_pruned,
        summary.orphan_raw_files_removed
    );
    let detail = format!(
        "dry_run={} completion={} core_completion={} budget_ms={} elapsed_ms={} settlement_ms={} budget_exhausted={} recoverable_failure={} wait_reason={:?} fatal_error={:?} orphan_cleanup_completed={} backlog_total={:?} observed_at={:?} source_max_invocation_id={:?} raw_candidates={} raw_compressed={} raw_bytes_before={} raw_bytes_after={} details_pruned={} invocation_rows_archived={} released_prompt_cache_conversations={} forward_proxy_attempt_rows_archived={} pool_attempt_rows_archived={} quota_rows_archived={} archive_batches_touched={} archive_batches_deleted={} raw_files_removed={} model_routes_pruned={} task_runs_pruned={} orphan_raw_files_removed={}",
        summary.dry_run,
        summary.completion(),
        summary.core_completion(),
        summary.work_budget_ms.unwrap_or_default(),
        summary.elapsed_ms.unwrap_or_default(),
        summary.settlement_ms.unwrap_or_default(),
        summary.budget_exhausted,
        summary.recoverable_failure,
        summary.wait_reason,
        summary.fatal_error,
        summary.orphan_cleanup_completed,
        summary.backlog_total,
        summary.backlog_observed_at,
        summary.source_max_invocation_id,
        summary.raw_files_compression_candidates,
        summary.raw_files_compressed,
        summary.raw_bytes_before,
        summary.raw_bytes_after,
        summary.invocation_details_pruned,
        summary.invocation_rows_archived,
        summary.prompt_cache_conversations_released,
        summary.forward_proxy_attempt_rows_archived,
        summary.pool_upstream_request_attempt_rows_archived,
        summary.quota_snapshot_rows_archived,
        summary.archive_batches_touched,
        summary.archive_batches_deleted,
        summary.raw_files_removed,
        summary.model_route_rows_pruned,
        summary.system_task_run_rows_pruned,
        summary.orphan_raw_files_removed
    );
    (brief, detail)
}

#[cfg(test)]
mod managed_task_control_contract_tests {
    use super::ManagedTaskControlRequest;
    use crate::OptionalField;

    #[test]
    fn schedule_patch_distinguishes_missing_null_and_value() {
        let missing: ManagedTaskControlRequest =
            serde_json::from_str(r#"{"enabled":true}"#).expect("decode missing schedule");
        assert!(matches!(missing.interval_secs, OptionalField::Missing));
        assert!(matches!(missing.cron_expr, OptionalField::Missing));

        let nulls: ManagedTaskControlRequest =
            serde_json::from_str(r#"{"enabled":true,"intervalSecs":null,"cronExpr":null}"#)
                .expect("decode null schedule");
        assert!(matches!(nulls.interval_secs, OptionalField::Null));
        assert!(matches!(nulls.cron_expr, OptionalField::Null));

        let values: ManagedTaskControlRequest =
            serde_json::from_str(r#"{"enabled":true,"intervalSecs":120,"cronExpr":"*/5 * * * *"}"#)
                .expect("decode schedule values");
        assert!(matches!(values.interval_secs, OptionalField::Value(120)));
        assert!(
            matches!(values.cron_expr, OptionalField::Value(ref value) if value == "*/5 * * * *")
        );
    }
}

#[cfg(test)]
mod runtime_pressure_health_tests {
    use super::*;
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    #[derive(Debug, Default)]
    struct BlockingFilesystemScanTestGate {
        started: AtomicBool,
        released: std::sync::Mutex<bool>,
        wake: std::sync::Condvar,
    }

    impl BlockingFilesystemScanTestGate {
        fn block(&self) {
            self.started.store(true, Ordering::Release);
            let mut released = self
                .released
                .lock()
                .expect("lock filesystem scan test gate");
            while !*released {
                released = self
                    .wake
                    .wait(released)
                    .expect("wait for filesystem scan test gate release");
            }
        }

        fn release(&self) {
            *self
                .released
                .lock()
                .expect("lock filesystem scan test gate") = true;
            self.wake.notify_all();
        }
    }

    async fn wait_for_blocking_filesystem_scan_start(gate: &BlockingFilesystemScanTestGate) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while !gate.started.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("filesystem scan should enter the controlled blocking operation");
    }

    async fn wait_for_filesystem_scan_admission_release(in_flight: &AtomicBool) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while in_flight.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("released filesystem scan should free its admission slot");
    }

    #[test]
    fn active_event_lag_and_writer_pressure_never_report_healthy() {
        assert_eq!(runtime_pressure_state(false, true, false), "degraded");
        assert_eq!(runtime_pressure_state(false, false, true), "deferred");
        assert_eq!(runtime_pressure_state(false, false, false), "healthy");
    }

    #[test]
    fn system_status_refresh_cadence_preserves_lead_before_cache_ceiling() {
        assert_eq!(
            system_status_snapshot_refresh_cadence_period(),
            system_status_snapshot_refresh_delay()
        );
        assert!(
            system_status_snapshot_refresh_cadence_period()
                + SYSTEM_STATUS_SNAPSHOT_REFRESH_DEADLINE
                < Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS)
        );
    }

    #[tokio::test]
    async fn filesystem_scan_deadline_wins_over_ready_result_without_snapshot_write() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let deadline = Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("a fresh monotonic instant can move back one millisecond");
        let scan_cancellation = CancellationToken::new();
        let scan_task = tokio::task::spawn_blocking(|| {
            Ok(SystemStatusFilesystemBytes {
                archive_bytes: 1,
                database_bytes: 2,
                other_files_bytes: 3,
            })
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !scan_task.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("controlled filesystem scan should complete");

        let error = await_system_status_filesystem_scan_task(
            scan_task,
            &scan_cancellation,
            deadline,
            &Arc::new(AtomicBool::new(false)),
        )
        .await
        .expect_err("a scan completing after its deadline must not escape the select");
        assert!(error.to_string().contains("exceeded its deadline"));

        let publication_cancellation = CancellationToken::new();
        let publication_error = publish_system_status_snapshot(
            state.as_ref(),
            &publication_cancellation,
            deadline,
            SystemStatusResponse {
                live_invocations_count: 0,
                success_count: 0,
                non_success_count: 0,
                completed_archive_batches_count: 0,
                archived_bodies: SystemStatusMetric::default(),
                raw_bodies: SystemStatusMetric::default(),
                request_raw_bodies: SystemStatusMetric::default(),
                response_raw_bodies: SystemStatusMetric::default(),
                database_bytes: 0,
                other_files_bytes: 0,
                projection_health: SystemProjectionHealth::default(),
                raw_metrics_health: SystemRawMetricsHealth::default(),
                runtime_pressure_health: None,
                refreshed_at: String::new(),
            },
            "ready".to_string(),
        )
        .await
        .expect_err("a deadline-expired result must not publish a snapshot");
        assert!(
            publication_error
                .to_string()
                .contains("exceeded its deadline"),
            "the cache lock must recheck the refresh deadline before publication"
        );
        assert!(
            state.system_status_cache.lock().await.latest.is_none(),
            "a rejected filesystem scan result must not write a status snapshot"
        );

        state.pool.close().await;
    }

    #[test]
    fn filesystem_scan_stops_at_deadline_during_directory_traversal() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-system-status-deadline-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("nested")).expect("create deadline scan fixture directory");
        for index in 0..8 {
            fs::write(
                root.join("nested").join(format!("payload-{index}")),
                b"payload",
            )
            .expect("write deadline scan fixture file");
        }

        let checkpoint_count = Arc::new(AtomicUsize::new(0));
        let checkpoint_count_for_hook = checkpoint_count.clone();
        let scan = SystemStatusFilesystemScan::with_test_checkpoint(
            CancellationToken::new(),
            Instant::now() + Duration::from_millis(250),
            Arc::new(move || {
                if checkpoint_count_for_hook.fetch_add(1, Ordering::SeqCst) >= 8 {
                    std::thread::sleep(Duration::from_millis(300));
                }
            }),
        );

        let error = sum_directory_bytes_with_scan(&root, &scan)
            .expect_err("expired filesystem scan must stop the active traversal");
        assert!(error.to_string().contains("exceeded its deadline"));
        assert!(
            checkpoint_count.load(Ordering::SeqCst) > 8,
            "the deadline hook must run after the first file metadata check"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn filesystem_scan_observes_cancellation_during_directory_traversal() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-system-status-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(root.join("nested")).expect("create scan fixture directory");
        for index in 0..8 {
            fs::write(
                root.join("nested").join(format!("payload-{index}")),
                b"payload",
            )
            .expect("write scan fixture file");
        }

        let cancellation = CancellationToken::new();
        let checkpoint_count = Arc::new(AtomicUsize::new(0));
        let checkpoint_cancellation = cancellation.clone();
        let checkpoint_count_for_hook = checkpoint_count.clone();
        let scan = SystemStatusFilesystemScan::with_test_checkpoint(
            cancellation,
            Instant::now() + Duration::from_secs(1),
            Arc::new(move || {
                if checkpoint_count_for_hook.fetch_add(1, Ordering::SeqCst) >= 8 {
                    checkpoint_cancellation.cancel();
                }
            }),
        );

        let error = sum_directory_bytes_with_scan(&root, &scan)
            .expect_err("cancellation during traversal must stop the scan");
        assert!(error.to_string().contains("cancelled"));
        assert!(
            checkpoint_count.load(Ordering::SeqCst) > 8,
            "the cancellation hook must run after the first file metadata check"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn filesystem_scan_abandonment_returns_promptly_and_releases_admission() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-system-status-blocking-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after the Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create blocking filesystem scan fixture directory");
        let archive_path = root.join("archive.sqlite");
        fs::write(&archive_path, b"archive").expect("write blocking filesystem scan fixture");
        let archive_paths = vec![archive_path.to_string_lossy().to_string()];
        let in_flight = state
            .system_status_cache
            .lock()
            .await
            .filesystem_scan_in_flight
            .clone();

        let deadline_gate = Arc::new(BlockingFilesystemScanTestGate::default());
        let deadline_gate_for_hook = deadline_gate.clone();
        let deadline_cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_millis(50);
        let deadline_scan =
            SystemStatusFilesystemScan::new(deadline_cancellation.clone(), deadline)
                .with_test_blocking_operation(Arc::new(move || deadline_gate_for_hook.block()));
        let mut deadline_task = {
            let cache = state.system_status_cache.clone();
            let config = state.config.clone();
            let archive_paths = archive_paths.clone();
            let root = root.clone();
            let cancellation = deadline_cancellation.clone();
            tokio::spawn(async move {
                collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
                    SystemStatusFilesystemScanInputs {
                        archive_paths,
                        config,
                        archive_dir: root.clone(),
                        raw_dir: root,
                    },
                    &cache,
                    &cancellation,
                    deadline,
                    deadline_scan,
                )
                .await
            })
        };
        wait_for_blocking_filesystem_scan_start(deadline_gate.as_ref()).await;
        let deadline_result =
            tokio::time::timeout(Duration::from_secs(1), &mut deadline_task).await;
        let held_after_deadline = in_flight.load(Ordering::Acquire);
        let retry_cancellation = CancellationToken::new();
        let retry_deadline = Instant::now() + Duration::from_secs(1);
        let retry_error = collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
            SystemStatusFilesystemScanInputs {
                archive_paths: archive_paths.clone(),
                config: state.config.clone(),
                archive_dir: root.clone(),
                raw_dir: root.clone(),
            },
            &state.system_status_cache,
            &retry_cancellation,
            retry_deadline,
            SystemStatusFilesystemScan::new(retry_cancellation.clone(), retry_deadline),
        )
        .await
        .expect_err("a detached filesystem scan must retain its only admission slot");
        deadline_gate.release();
        wait_for_filesystem_scan_admission_release(in_flight.as_ref()).await;

        let deadline_error = deadline_result
            .expect("filesystem deadline must return before the blocked syscall is released")
            .expect("filesystem deadline task should join")
            .expect_err("blocked filesystem scan must fail at its deadline");
        assert!(deadline_error.to_string().contains("exceeded its deadline"));
        assert!(
            held_after_deadline,
            "blocked scan must retain the admission slot"
        );
        assert!(retry_error.to_string().contains("already in flight"));

        let cancellation_gate = Arc::new(BlockingFilesystemScanTestGate::default());
        let cancellation_gate_for_hook = cancellation_gate.clone();
        let cancellation = CancellationToken::new();
        let cancellation_deadline = Instant::now() + Duration::from_secs(1);
        let cancellation_scan =
            SystemStatusFilesystemScan::new(cancellation.clone(), cancellation_deadline)
                .with_test_blocking_operation(Arc::new(move || cancellation_gate_for_hook.block()));
        let mut cancellation_task = {
            let cache = state.system_status_cache.clone();
            let config = state.config.clone();
            let archive_paths = archive_paths.clone();
            let root = root.clone();
            let cancellation = cancellation.clone();
            tokio::spawn(async move {
                collect_system_status_filesystem_bytes_in_blocking_task_with_scan(
                    SystemStatusFilesystemScanInputs {
                        archive_paths,
                        config,
                        archive_dir: root.clone(),
                        raw_dir: root,
                    },
                    &cache,
                    &cancellation,
                    cancellation_deadline,
                    cancellation_scan,
                )
                .await
            })
        };
        wait_for_blocking_filesystem_scan_start(cancellation_gate.as_ref()).await;
        cancellation.cancel();
        let cancellation_result =
            tokio::time::timeout(Duration::from_secs(1), &mut cancellation_task).await;
        let held_after_cancellation = in_flight.load(Ordering::Acquire);
        cancellation_gate.release();
        wait_for_filesystem_scan_admission_release(in_flight.as_ref()).await;

        let cancellation_error = cancellation_result
            .expect("filesystem cancellation must return before the blocked syscall is released")
            .expect("filesystem cancellation task should join")
            .expect_err("blocked filesystem scan must fail when cancelled");
        assert!(cancellation_error.to_string().contains("cancelled"));
        assert!(
            held_after_cancellation,
            "cancelled blocked scan must retain the admission slot"
        );
        assert!(
            state.system_status_cache.lock().await.latest.is_none(),
            "an abandoned filesystem scan must not publish a status snapshot"
        );

        let _ = fs::remove_dir_all(root);
        state.pool.close().await;
    }

    #[tokio::test]
    async fn system_status_refresh_single_flight_rejects_overlapping_attempts() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let cancellation = CancellationToken::new();
        let flight = SystemStatusSnapshotRefreshFlight::begin(state.as_ref(), &cancellation)
            .await
            .expect("begin first system status refresh");

        let error = SystemStatusSnapshotRefreshFlight::begin(state.as_ref(), &cancellation)
            .await
            .expect_err("overlapping system status refresh must be rejected");
        assert!(error.to_string().contains("already in flight"));

        flight.finish().await;
        state.pool.close().await;
    }

    #[tokio::test]
    async fn stale_status_snapshot_never_falls_back_to_request_sqlite() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        hydrate_system_status_snapshot(state.as_ref())
            .await
            .expect("hydrate status snapshot");
        {
            let mut cache = state.system_status_cache.lock().await;
            cache.latest.as_mut().expect("hydrated entry").cached_at =
                Instant::now() - Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS + 1);
        }
        state.pool.close().await;

        assert!(load_system_status_cached(state.as_ref()).await.is_err());
    }

    #[tokio::test]
    async fn failed_status_refresh_keeps_fresh_last_good_snapshot() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        hydrate_system_status_snapshot(state.as_ref())
            .await
            .expect("hydrate status snapshot");
        let expected = load_system_status_cached(state.as_ref())
            .await
            .expect("load hydrated status snapshot");
        state.pool.close().await;

        assert!(
            refresh_system_status_snapshot(state.as_ref())
                .await
                .is_err()
        );
        assert_eq!(
            load_system_status_cached(state.as_ref())
                .await
                .expect("serve last-good status snapshot"),
            expected
        );
    }
}
