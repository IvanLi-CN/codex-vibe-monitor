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
    pub(crate) database_pressure: crate::db_pressure::DbPressureSnapshot,
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

fn runtime_projection_deferred(
    terminal: &crate::terminal_projection::TerminalProjectionHealth,
    long_term: &crate::long_term_stats::LongTermProjectionHealth,
) -> bool {
    terminal.hard_limit_reason.is_some() || long_term.last_defer_reason.is_some()
}

fn runtime_cursor_growth(
    terminal: &crate::terminal_projection::TerminalProjectionHealth,
    long_term: &crate::long_term_stats::LongTermProjectionHealth,
) -> bool {
    terminal.last_persisted_row_id > long_term.cursor_row_id
        || (terminal.timeseries_consumer_active
            && terminal.last_persisted_row_id > terminal.timeseries_cursor_row_id)
}

fn runtime_writer_pressure_active(
    writer: &PendingQueueAccountingSnapshot,
    database: &crate::db_pressure::DbPressureSnapshot,
    coordinator: &crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinatorSnapshot,
) -> bool {
    writer.p2_deferred_age_ms > 0
        || database.pressure_cooldown_remaining_ms > 0
        || coordinator.p1_waiter_count > 0
        || coordinator.interactive_waiter_count > 0
        || coordinator.p2_waiter_count > 0
        || coordinator.maintenance_waiter_count > 0
}

fn dashboard_projection_cadence_missed(
    counters: &crate::app_state::DashboardRuntimeTopologyCounterSnapshot,
) -> bool {
    [counters.current, counters.network, counters.terminal]
        .into_iter()
        .any(|slice| slice.cadence_miss_count > 0)
}

fn runtime_prompt_cache_signals(
    projection: &PromptCacheTopicProjectionHealthSnapshot,
) -> (bool, bool, bool, bool) {
    (
        projection.failed_or_stale_topic_count > 0,
        projection.live_path_db_read_count > 0,
        projection.bounded_cold_recovery_topic_count > 0,
        projection.pressure_deferred_topic_count > 0,
    )
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
    let event_bus = state.subscription_hub.runtime_mutation_bus_health();
    let backfill = startup_backfill_health_snapshot();
    let terminal_projection = state.terminal_projection_hub.health();
    let long_term_projection = state.long_term_projection_runtime.lock().await.health();
    let projection_cadence_missed =
        dashboard_projection_cadence_missed(&dashboard_projection.slice_counters);
    let (
        prompt_cache_failed_or_stale,
        prompt_cache_live_path_db_read,
        prompt_cache_bounded_cold_recovery,
        prompt_cache_pressure_deferred,
    ) = runtime_prompt_cache_signals(&prompt_cache_projection);
    let dashboard_hot_topics = state
        .subscription_hub
        .dashboard_hot_topic_health(dashboard_projection.slice_counters)
        .await;
    let projection_deferred = dashboard_projection.last_defer_reason.is_some()
        || runtime_projection_deferred(&terminal_projection, &long_term_projection);
    let cursor_growth = runtime_cursor_growth(&terminal_projection, &long_term_projection);
    let writer_pressure_active = runtime_writer_pressure_active(
        &writer_accounting,
        &database_pressure,
        &proxy_sqlite_write_coordinator,
    );
    let state = runtime_pressure_state(
        writer_accounting.state == "degraded",
        memory.pressure_level != "normal"
            || dashboard_projection.state == "degraded"
            || projection_cadence_missed
            || state
                .subscription_hub
                .dashboard_delivery_has_degraded_signal()
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
        database_pressure,
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
