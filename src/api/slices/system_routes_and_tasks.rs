use super::*;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tracing::{debug, warn};

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

pub(crate) const SYSTEM_STATUS_CACHE_TTL_SECS: u64 = 60;
const SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE: i64 = 128;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemStatusMetric {
    pub(crate) count: u64,
    pub(crate) bytes: u64,
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

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemRawMetricsHealth {
    pub(crate) state: String,
    pub(crate) inventory_cursor: i64,
    pub(crate) updated_age_ms: Option<u64>,
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
    pub(crate) proxy_sqlite_write_coordinator:
        crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinatorSnapshot,
    pub(crate) dashboard_projection: RuntimeProjectionHealthSnapshot,
    pub(crate) delivery: DashboardDeliveryTopologyCounterSnapshot,
    pub(crate) dashboard_hot_topics: DashboardHotTopicsHealthSnapshot,
    pub(crate) request_pipeline: RequestPipelineHealthSnapshot,
    pub(crate) prompt_cache_projection: PromptCacheTopicProjectionHealthSnapshot,
    pub(crate) retention_write_health: RetentionWriteHealthSnapshot,
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
        || proxy_sqlite_write_coordinator.p1_waiter_count > 0
        || proxy_sqlite_write_coordinator.interactive_waiter_count > 0
        || proxy_sqlite_write_coordinator.p2_waiter_count > 0;
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
            || event_bus.state == "degraded"
            || backfill.state == "degraded",
        projection_deferred
            || cursor_growth
            || writer_pressure_active
            || prompt_cache_pressure_deferred
            || prompt_cache_bounded_cold_recovery
            || retention_write_health.state == "deferred"
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
        proxy_sqlite_write_coordinator,
        dashboard_projection,
        delivery,
        dashboard_hot_topics,
        request_pipeline,
        prompt_cache_projection,
        retention_write_health,
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
}

#[derive(Debug, Clone)]
pub(crate) struct SystemTaskRunHandle {
    pub(crate) id: i64,
    pub(crate) task_kind: SystemTaskKind,
    pub(crate) trigger_kind: String,
    pub(crate) started_at: Instant,
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
    raw_count: i64,
    raw_bytes: i64,
    request_raw_count: i64,
    request_raw_bytes: i64,
    response_raw_count: i64,
    response_raw_bytes: i64,
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
    Ok(Some(format_utc_iso(parsed)))
}

pub(crate) fn count_file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
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
    metric.bytes = metric.bytes.saturating_add(count_file_size(&candidate));
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

pub(crate) fn count_database_bytes(db_path: &Path) -> u64 {
    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
    count_file_size(db_path)
        .saturating_add(count_file_size(&wal_path))
        .saturating_add(count_file_size(&shm_path))
}

pub(crate) fn sum_directory_bytes(root: &Path) -> u64 {
    let mut total = 0_u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(child),
                Ok(kind) if kind.is_file() => {
                    total =
                        total.saturating_add(entry.metadata().map(|meta| meta.len()).unwrap_or(0));
                }
                _ => {}
            }
        }
    }
    total
}

pub(crate) fn sum_path_bytes(path: &Path) -> u64 {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata.len(),
        Ok(metadata) if metadata.is_dir() => sum_directory_bytes(path),
        _ => 0,
    }
}

pub(crate) fn compute_other_files_bytes(
    config: &AppConfig,
    archive_dir: &Path,
    raw_dir: &Path,
) -> u64 {
    let db_path = &config.database_path;
    let db_wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let db_shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
    let mut seen = HashSet::new();

    // Keep "other files" scoped to runtime-owned storage that does not already
    // have a dedicated metric on the system status page.
    [config.xray_runtime_dir.clone()]
        .into_iter()
        .filter(|path| !path.as_os_str().is_empty())
        .filter(|path| seen.insert(path.clone()))
        .filter(|path| {
            let candidate = path.as_path();
            candidate != db_path
                && candidate != db_wal_path.as_path()
                && candidate != db_shm_path.as_path()
                && candidate != archive_dir
                && candidate != raw_dir
        })
        .map(|path| sum_path_bytes(&path))
        .sum()
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

pub(crate) async fn refresh_system_raw_payload_metrics_inventory(state: &AppState) -> Result<()> {
    let memory_baseline = state.memory_diagnostics.begin_operation(state).await;
    let result = refresh_system_raw_payload_metrics_inventory_inner(state).await;
    let load_row_count = result.as_ref().copied().unwrap_or_default();
    state
        .memory_diagnostics
        .observe_operation(
            state,
            "system_raw_payload_metrics_inventory",
            memory_baseline,
            load_row_count,
            true,
        )
        .await;
    result.map(|_| ())
}

async fn refresh_system_raw_payload_metrics_inventory_inner(state: &AppState) -> Result<u64> {
    let gate = crate::db_pressure::global_db_pressure_gate();
    let _permit = match gate.try_begin_background("system_raw_metrics_inventory") {
        Ok(permit) => permit,
        Err(reason) => {
            set_system_raw_metrics_health_override(state, Some("deferred")).await;
            debug!(
                metrics_source = "inventory",
                gate_outcome = "deferred",
                defer_reason = "writer_pressure",
                reason = %reason,
                "system raw metrics inventory deferred by database pressure"
            );
            return Ok(0);
        }
    };
    let snapshot = sqlx::query_as::<_, SystemRawPayloadMetricsRow>(
        "SELECT inventory_state, inventory_cursor, link_inventory_cursor, raw_count, raw_bytes, request_raw_count, request_raw_bytes, response_raw_count, response_raw_bytes, updated_at FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(&state.pool)
    .await?;
    if snapshot.inventory_state == "resetting" {
        set_system_raw_metrics_health_override(state, Some("preparing")).await;
        debug!(
            metrics_source = "inventory",
            "system raw metrics inventory is waiting for retention reset batches"
        );
        return Ok(0);
    }
    let rows = sqlx::query_as::<_, SystemRawPayloadInventoryRow>(
        r#"
        SELECT id, request_raw_path, response_raw_path
        FROM codex_invocations
        WHERE id > ?1
          AND (request_raw_path IS NOT NULL OR response_raw_path IS NOT NULL)
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(snapshot.inventory_cursor)
    .bind(SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE)
    .fetch_all(&state.pool)
    .await?;
    let link_rows = sqlx::query_as::<_, SystemRawPayloadBlobLinkRow>(
        r#"
        SELECT id, raw_path, raw_role
        FROM proxy_raw_payload_blob_links
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(snapshot.link_inventory_cursor)
    .bind(SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE)
    .fetch_all(&state.pool)
    .await?;

    let fallback_root = state.config.database_path.parent();
    let mut paths = HashMap::<String, (i64, bool, bool)>::new();
    for row in &rows {
        for (raw_path, is_request) in [
            (row.request_raw_path.as_deref(), true),
            (row.response_raw_path.as_deref(), false),
        ] {
            let Some(raw_path) = raw_path else {
                continue;
            };
            let Some(candidate) = resolved_raw_path_read_candidates(raw_path, fallback_root)
                .into_iter()
                .find(|candidate| candidate.exists())
            else {
                continue;
            };
            let entry = paths
                .entry(candidate.to_string_lossy().to_string())
                .or_insert((count_file_size(&candidate) as i64, false, false));
            if is_request {
                entry.1 = true;
            } else {
                entry.2 = true;
            }
        }
    }
    for row in &link_rows {
        let Some(candidate) = resolved_raw_path_read_candidates(&row.raw_path, fallback_root)
            .into_iter()
            .find(|candidate| candidate.exists())
        else {
            continue;
        };
        let entry = paths
            .entry(candidate.to_string_lossy().to_string())
            .or_insert((count_file_size(&candidate) as i64, false, false));
        match row.raw_role.as_str() {
            "request" => entry.1 = true,
            "response" => entry.2 = true,
            _ => {}
        }
    }

    let mut deltas = (0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64);
    let mut tx = state.pool.begin().await?;
    for (path, (byte_size, request_seen, response_seen)) in paths {
        let delta = record_system_raw_payload_inventory_path(
            &mut tx,
            &path,
            byte_size,
            request_seen,
            response_seen,
        )
        .await?;
        deltas.0 += delta.0;
        deltas.1 += delta.1;
        deltas.2 += delta.2;
        deltas.3 += delta.3;
        deltas.4 += delta.4;
        deltas.5 += delta.5;
    }
    let next_cursor = rows
        .last()
        .map(|row| row.id)
        .unwrap_or(snapshot.inventory_cursor);
    let next_link_cursor = link_rows
        .last()
        .map(|row| row.id)
        .unwrap_or(snapshot.link_inventory_cursor);
    let state_name = if rows.len() < SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE as usize
        && link_rows.len() < SYSTEM_RAW_METRICS_INVENTORY_BATCH_SIZE as usize
    {
        "ready"
    } else {
        "preparing"
    };
    sqlx::query(
        r#"
        UPDATE system_raw_payload_metrics
        SET inventory_state = ?1,
            inventory_cursor = ?2,
            link_inventory_cursor = ?3,
            raw_count = raw_count + ?4,
            raw_bytes = raw_bytes + ?5,
            request_raw_count = request_raw_count + ?6,
            request_raw_bytes = request_raw_bytes + ?7,
            response_raw_count = response_raw_count + ?8,
            response_raw_bytes = response_raw_bytes + ?9,
            updated_at = datetime('now')
        WHERE singleton = 1
        "#,
    )
    .bind(state_name)
    .bind(next_cursor)
    .bind(next_link_cursor)
    .bind(deltas.0)
    .bind(deltas.1)
    .bind(deltas.2)
    .bind(deltas.3)
    .bind(deltas.4)
    .bind(deltas.5)
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    set_system_raw_metrics_health_override(state, None).await;
    debug!(
        metrics_source = "inventory",
        inventory_state = state_name,
        inventory_cursor = next_cursor,
        link_inventory_cursor = next_link_cursor,
        legacy_row_count = rows.len(),
        link_row_count = link_rows.len(),
        discovered_path_count = deltas.0,
        "system raw metrics inventory batch completed"
    );
    Ok(rows.len().saturating_add(link_rows.len()) as u64)
}

pub(crate) async fn set_system_raw_metrics_health_override(
    state: &AppState,
    override_state: Option<&str>,
) {
    let override_state = override_state.map(str::to_string);
    let mut cache = state.system_status_cache.lock().await;
    if cache.raw_metrics_health_override == override_state {
        return;
    }
    cache.raw_metrics_health_override = override_state;
    cache.latest = None;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SystemRawPayloadInventoryResetOutcome {
    pub(crate) removed_path_count: usize,
    pub(crate) complete: bool,
}

pub(crate) async fn reset_system_raw_payload_metrics_inventory_batch(
    state: &AppState,
    max_paths: usize,
) -> Result<SystemRawPayloadInventoryResetOutcome> {
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        r#"
        UPDATE system_raw_payload_metrics
        SET inventory_state = 'resetting',
            inventory_cursor = 0,
            link_inventory_cursor = 0,
            raw_count = 0,
            raw_bytes = 0,
            request_raw_count = 0,
            request_raw_bytes = 0,
            response_raw_count = 0,
            response_raw_bytes = 0,
            updated_at = datetime('now')
        WHERE singleton = 1
        "#,
    )
    .execute(tx.as_mut())
    .await?;
    let removed_path_count = sqlx::query(
        r#"
        DELETE FROM system_raw_payload_inventory_paths
        WHERE raw_path IN (
            SELECT raw_path
            FROM system_raw_payload_inventory_paths
            ORDER BY raw_path ASC
            LIMIT ?1
        )
        "#,
    )
    .bind(max_paths.max(1) as i64)
    .execute(tx.as_mut())
    .await?
    .rows_affected() as usize;
    let complete =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM system_raw_payload_inventory_paths")
            .fetch_one(tx.as_mut())
            .await?
            == 0;
    if complete {
        sqlx::query(
            "UPDATE system_raw_payload_metrics SET inventory_state = 'preparing', updated_at = datetime('now') WHERE singleton = 1",
        )
        .execute(tx.as_mut())
        .await?;
    }
    tx.commit().await?;
    set_system_raw_metrics_health_override(state, Some("preparing")).await;
    debug!(
        metrics_source = "inventory",
        removed_path_count,
        complete,
        "system raw metrics inventory reset batch completed after retention"
    );
    Ok(SystemRawPayloadInventoryResetOutcome {
        removed_path_count,
        complete,
    })
}

pub(crate) fn spawn_system_raw_payload_metrics_inventory(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(error) = refresh_system_raw_payload_metrics_inventory(state.as_ref()).await {
                set_system_raw_metrics_health_override(state.as_ref(), Some("error")).await;
                warn!(error = %error, "system raw metrics inventory batch failed");
            }
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
        }
    })
}

pub(crate) async fn load_system_status_uncached(state: &AppState) -> Result<SystemStatusResponse> {
    let runtime_pressure_health = load_runtime_pressure_health(state).await;
    let invocation_status = sqlx::query_as::<_, SystemInvocationStatusAggRow>(
        r#"
        SELECT
            COUNT(*) AS live_invocations_count,
            COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'warning_success') THEN 1 ELSE 0 END), 0) AS success_count,
            COALESCE(SUM(CASE WHEN LOWER(TRIM(COALESCE(status, ''))) NOT IN ('success', 'warning_success') THEN 1 ELSE 0 END), 0) AS non_success_count
        FROM codex_invocations
        "#,
    )
    .fetch_one(&state.pool)
    .await?;

    let archived = sqlx::query_as::<_, SystemArchiveAggRow>(
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
    .await?;

    let raw_metrics = sqlx::query_as::<_, SystemRawPayloadMetricsRow>(
        "SELECT inventory_state, inventory_cursor, link_inventory_cursor, raw_count, raw_bytes, request_raw_count, request_raw_bytes, response_raw_count, response_raw_bytes, updated_at FROM system_raw_payload_metrics WHERE singleton = 1",
    )
    .fetch_one(&state.pool)
    .await?;
    let raw_metrics_state = state
        .system_status_cache
        .lock()
        .await
        .raw_metrics_health_override
        .clone()
        .unwrap_or_else(|| raw_metrics.inventory_state.clone());

    let archive_dir = resolved_archive_dir(&state.config);
    let raw_dir = state.config.resolved_proxy_raw_dir();
    let archived_paths = sqlx::query_scalar::<_, String>(
        r#"
        SELECT file_path
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = 'completed'
        "#,
    )
    .fetch_all(&state.pool)
    .await?;
    let mut seen_paths = std::collections::HashSet::new();
    let archive_bytes = archived_paths
        .into_iter()
        .filter(|path| seen_paths.insert(path.clone()))
        .map(PathBuf::from)
        .map(|path| count_file_size(&path))
        .sum();
    let database_bytes = count_database_bytes(&state.config.database_path);
    let other_files_bytes = compute_other_files_bytes(&state.config, &archive_dir, &raw_dir);
    let terminal_health = state.terminal_projection_hub.health();
    let long_term_health = state.long_term_projection_runtime.lock().await.health();
    let runtime_record_count = state.proxy_runtime_invocations.runtime_record_count() as u64;
    debug!(
        db_invocation_row_count = invocation_status.live_invocations_count.unwrap_or(0).max(0),
        runtime_record_count,
        "system status invocation counts keep database rows separate from runtime memory records"
    );

    Ok(SystemStatusResponse {
        live_invocations_count: invocation_status.live_invocations_count.unwrap_or(0).max(0) as u64,
        success_count: invocation_status.success_count.unwrap_or(0).max(0) as u64,
        non_success_count: invocation_status.non_success_count.unwrap_or(0).max(0) as u64,
        completed_archive_batches_count: archived
            .completed_archive_batches_count
            .unwrap_or(0)
            .max(0) as u64,
        archived_bodies: SystemStatusMetric {
            count: archived.archived_count.unwrap_or(0).max(0) as u64,
            bytes: archive_bytes,
        },
        raw_bodies: SystemStatusMetric {
            count: raw_metrics.raw_count.max(0) as u64,
            bytes: raw_metrics.raw_bytes.max(0) as u64,
        },
        request_raw_bodies: SystemStatusMetric {
            count: raw_metrics.request_raw_count.max(0) as u64,
            bytes: raw_metrics.request_raw_bytes.max(0) as u64,
        },
        response_raw_bodies: SystemStatusMetric {
            count: raw_metrics.response_raw_count.max(0) as u64,
            bytes: raw_metrics.response_raw_bytes.max(0) as u64,
        },
        database_bytes,
        other_files_bytes,
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
            state: raw_metrics_state,
            inventory_cursor: raw_metrics.inventory_cursor,
            updated_age_ms: None,
        },
        runtime_pressure_health: Some(runtime_pressure_health),
        refreshed_at: format_utc_iso(Utc::now()),
    })
}

pub(crate) async fn load_system_status_cached(state: &AppState) -> Result<SystemStatusResponse> {
    loop {
        let mut cached_response = None;
        let wait_for = {
            let mut cache = state.system_status_cache.lock().await;
            if let Some(entry) = cache.latest.as_ref()
                && entry.cached_at.elapsed() < Duration::from_secs(SYSTEM_STATUS_CACHE_TTL_SECS)
            {
                cached_response = Some(entry.response.clone());
                None
            } else if let Some(signal) = cache.in_flight.clone() {
                cache.waiter_count = cache.waiter_count.saturating_add(1);
                Some(signal.subscribe())
            } else {
                let (signal, _) = watch::channel(false);
                cache.in_flight = Some(signal);
                None
            }
        };

        if let Some(mut response) = cached_response {
            response.runtime_pressure_health = Some(load_runtime_pressure_health(state).await);
            return Ok(response);
        }

        if let Some(mut signal) = wait_for {
            let _ = signal.changed().await;
            continue;
        }

        let response = load_system_status_uncached(state).await;
        let mut cache = state.system_status_cache.lock().await;
        if let Ok(response) = &response {
            cache.latest = Some(SystemStatusCacheEntry {
                cached_at: Instant::now(),
                response: response.clone(),
            });
        }
        let waiter_count = cache.waiter_count;
        cache.waiter_count = 0;
        if let Some(signal) = cache.in_flight.take() {
            let _ = signal.send(true);
        }
        debug!(
            metrics_source = "system_status_cache",
            cache_ttl_ms = SYSTEM_STATUS_CACHE_TTL_SECS * 1_000,
            singleflight_waiter_count = waiter_count,
            "system status cache refresh completed"
        );
        return response;
    }
}

pub(crate) async fn invalidate_system_status_cache(state: &AppState) {
    let mut cache = state.system_status_cache.lock().await;
    cache.latest = None;
}

pub(crate) async fn begin_system_task_run(
    pool: &Pool<Sqlite>,
    task_kind: SystemTaskKind,
    trigger_kind: impl Into<String>,
    summary: Option<String>,
) -> Result<SystemTaskRunHandle> {
    let started_at = format_utc_iso(Utc::now());
    let trigger_kind = trigger_kind.into();
    let id = sqlx::query_scalar::<_, i64>(
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
    .bind(task_kind.as_str())
    .bind(&trigger_kind)
    .bind(SystemTaskStatus::Running.as_str())
    .bind(summary)
    .bind(&started_at)
    .fetch_one(pool)
    .await?;

    Ok(SystemTaskRunHandle {
        id,
        task_kind,
        trigger_kind,
        started_at: Instant::now(),
    })
}

pub(crate) async fn finish_system_task_run(
    pool: &Pool<Sqlite>,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) {
    let finished_at = format_utc_iso(Utc::now());
    let duration_ms = handle
        .started_at
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    if let Err(err) = sqlx::query(
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
            error = %err,
            "failed to finalize system task run"
        );
    }
}

pub(crate) async fn finish_system_task_run_batched(
    state: &AppState,
    handle: &SystemTaskRunHandle,
    status: SystemTaskStatus,
    summary: Option<String>,
    detail: Option<String>,
) {
    let finished_at = format_utc_iso(Utc::now());
    let duration_ms = handle
        .started_at
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    if state
        .sqlite_batch_writer
        .enqueue(SqliteBatchWrite::SystemTaskFinish(
            BatchedSystemTaskFinish {
                run_id: handle.id,
                task_kind: handle.task_kind,
                trigger_kind: handle.trigger_kind.clone(),
                status,
                summary: summary.clone(),
                detail: detail.clone(),
                finished_at,
                duration_ms,
            },
        ))
    {
        return;
    }

    finish_system_task_run(&state.pool, handle, status, summary, detail).await;
}

pub(crate) async fn fetch_system_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SystemStatusResponse>, ApiError> {
    Ok(Json(load_system_status_cached(state.as_ref()).await?))
}

pub(crate) async fn list_system_task_runs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SystemTaskRunsQuery>,
) -> Result<Json<SystemTaskRunsListResponse>, ApiError> {
    let started_at_from =
        parse_system_task_run_bound(query.started_at_from.as_deref(), "startedAtFrom")?;
    let started_at_to = parse_system_task_run_bound(query.started_at_to.as_deref(), "startedAtTo")?;
    let page_size = query
        .page_size
        .unwrap_or(query.limit.unwrap_or(20))
        .clamp(1, 100);
    let page = query.page.unwrap_or(1).max(1);
    let limit = i64::from(page_size);
    let offset = i64::from(page.saturating_sub(1)) * limit;
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, task_kind, trigger_kind, status, summary, detail, started_at, finished_at, duration_ms FROM system_task_runs WHERE 1 = 1",
    );
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
    if let Some(started_at_from) = started_at_from.as_deref() {
        builder
            .push(" AND datetime(started_at) >= datetime(")
            .push_bind(started_at_from)
            .push(")");
    }
    if let Some(started_at_to) = started_at_to.as_deref() {
        builder
            .push(" AND datetime(started_at) <= datetime(")
            .push_bind(started_at_to)
            .push(")");
    }
    builder
        .push(" ORDER BY started_at DESC, id DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let rows = builder
        .build_query_as::<SystemTaskRunRow>()
        .fetch_all(&state.pool)
        .await?;

    let mut count_builder =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) as total FROM system_task_runs WHERE 1 = 1");
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
    if let Some(started_at_from) = started_at_from.as_deref() {
        count_builder
            .push(" AND datetime(started_at) >= datetime(")
            .push_bind(started_at_from)
            .push(")");
    }
    if let Some(started_at_to) = started_at_to.as_deref() {
        count_builder
            .push(" AND datetime(started_at) <= datetime(")
            .push_bind(started_at_to)
            .push(")");
    }
    let total = count_builder
        .build_query_scalar::<i64>()
        .fetch_one(&state.pool)
        .await?;

    Ok(Json(SystemTaskRunsListResponse {
        items: rows.into_iter().map(Into::into).collect(),
        total: total.max(0) as u64,
        page,
        page_size,
    }))
}

pub(crate) fn summarize_retention_run_for_system_task(
    summary: &RetentionRunSummary,
) -> (String, String) {
    let brief = format!(
        "compressed={} archived_invocations={} pruned_details={} model_routes_pruned={} orphan_raw_removed={}",
        summary.raw_files_compressed,
        summary.invocation_rows_archived,
        summary.invocation_details_pruned,
        summary.model_route_rows_pruned,
        summary.orphan_raw_files_removed
    );
    let detail = format!(
        "dry_run={} raw_candidates={} raw_compressed={} raw_bytes_before={} raw_bytes_after={} details_pruned={} invocation_rows_archived={} forward_proxy_attempt_rows_archived={} pool_attempt_rows_archived={} quota_rows_archived={} archive_batches_touched={} archive_batches_deleted={} raw_files_removed={} model_routes_pruned={} orphan_raw_files_removed={}",
        summary.dry_run,
        summary.raw_files_compression_candidates,
        summary.raw_files_compressed,
        summary.raw_bytes_before,
        summary.raw_bytes_after,
        summary.invocation_details_pruned,
        summary.invocation_rows_archived,
        summary.forward_proxy_attempt_rows_archived,
        summary.pool_upstream_request_attempt_rows_archived,
        summary.quota_snapshot_rows_archived,
        summary.archive_batches_touched,
        summary.archive_batches_deleted,
        summary.raw_files_removed,
        summary.model_route_rows_pruned,
        summary.orphan_raw_files_removed
    );
    (brief, detail)
}

#[cfg(test)]
mod runtime_pressure_health_tests {
    use super::runtime_pressure_state;

    #[test]
    fn active_event_lag_and_writer_pressure_never_report_healthy() {
        assert_eq!(runtime_pressure_state(false, true, false), "degraded");
        assert_eq!(runtime_pressure_state(false, false, true), "deferred");
        assert_eq!(runtime_pressure_state(false, false, false), "healthy");
    }
}
