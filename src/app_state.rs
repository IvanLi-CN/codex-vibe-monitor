use super::*;

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingRuntimeCache {
    pub(crate) api_key: Option<String>,
    pub(crate) request_compression: PoolRoutingRequestCompressionSettingsResolved,
    pub(crate) timeouts: PoolRoutingTimeoutSettingsResolved,
}

#[derive(Debug, Default)]
pub(crate) struct PoolAccountSelectionRuntime {
    pub(crate) selected_at: std::sync::Mutex<HashMap<i64, String>>,
}

impl PoolAccountSelectionRuntime {
    pub(crate) fn record_selected(&self, account_id: i64, selected_at: String) {
        if let Ok(mut guard) = self.selected_at.lock() {
            match guard.get(&account_id) {
                Some(existing) if existing >= &selected_at => {}
                _ => {
                    guard.insert(account_id, selected_at);
                }
            }
        }
    }

    pub(crate) fn latest_selected_at(
        &self,
        account_id: i64,
        persisted: Option<&str>,
    ) -> Option<String> {
        let runtime = self
            .selected_at
            .lock()
            .ok()
            .and_then(|guard| guard.get(&account_id).cloned());
        match (runtime, persisted) {
            (Some(runtime), Some(persisted)) if runtime.as_str() < persisted => {
                Some(persisted.to_string())
            }
            (Some(runtime), _) => Some(runtime),
            (None, Some(persisted)) => Some(persisted.to_string()),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct RuntimeInvocationKey {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

impl RuntimeInvocationKey {
    pub(crate) fn new(invoke_id: impl Into<String>, occurred_at: impl Into<String>) -> Self {
        Self {
            invoke_id: invoke_id.into(),
            occurred_at: occurred_at.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeInvocationEntry {
    pub(crate) record: ApiInvocation,
    pub(crate) updated_at: Instant,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeInvocationStoreUpsertOutcome {
    pub(crate) running_count: usize,
    pub(crate) pruned_count: usize,
    pub(crate) skipped_terminal: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeInvocationStoreShutdownSummary {
    pub(crate) running_count: usize,
    pub(crate) oldest_age_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeInvocationStoreRemoveOutcome {
    pub(crate) removed: bool,
    pub(crate) already_terminal: bool,
}

pub(crate) const DASHBOARD_RUNTIME_PROJECTION_MODE_ENV: &str = "DASHBOARD_RUNTIME_PROJECTION_MODE";
pub(crate) const DASHBOARD_RUNTIME_PROJECTION_COALESCE: Duration = Duration::from_millis(250);
pub(crate) const DASHBOARD_RUNTIME_NETWORK_PROJECTION_COALESCE: Duration = Duration::from_secs(1);
pub(crate) const DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE: Duration = Duration::from_secs(5);
const DASHBOARD_RUNTIME_TERMINAL_MAX_PENDING: usize = 10_000;
const DASHBOARD_RUNTIME_TERMINAL_MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeProjectionMode {
    Auto,
    Legacy,
}

impl RuntimeProjectionMode {
    pub(crate) fn parse(value: Option<&str>) -> Result<Self> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None | Some("auto") => Ok(Self::Auto),
            Some("legacy") => Ok(Self::Legacy),
            Some(value) => bail!(
                "invalid {DASHBOARD_RUNTIME_PROJECTION_MODE_ENV} value `{value}`; expected `auto` or `legacy`"
            ),
        }
    }

    pub(crate) fn from_env() -> Result<Self> {
        let value = std::env::var(DASHBOARD_RUNTIME_PROJECTION_MODE_ENV).ok();
        Self::parse(value.as_deref())
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Legacy => "legacy",
        }
    }
}

#[derive(Debug)]
struct DashboardRuntimeProjectionState {
    dirty_generation: u64,
    pending_deadline: Option<Instant>,
    network_dirty_generation: u64,
    pending_network_deadline: Option<Instant>,
    terminal_dirty_generation: u64,
    pending_terminal_deadline: Option<Instant>,
    terminal_published_generation: u64,
    current_revision: u64,
    network_revision: u64,
    terminal_revision: u64,
    terminal_pending_delta_bytes: usize,
    terminal_pending_deltas: VecDeque<DashboardActivityTerminalDelta>,
    last_good: Option<DashboardActivityLiveSnapshot>,
    network_last_good: Option<DashboardNetworkProjectionSlice>,
    legacy_last_good: Option<DashboardActivityLiveSnapshot>,
    last_good_at: Option<Instant>,
    last_snapshot_origin: Option<&'static str>,
    degraded_reason: Option<&'static str>,
    reconcile_error: Option<&'static str>,
    last_reconcile_defer_reason: Option<&'static str>,
    persistence_baseline: Option<DashboardRuntimeProjectionBaseline>,
    baseline_records: HashMap<RuntimeInvocationKey, DashboardRuntimeBaselineRecord>,
    projection_records: HashMap<RuntimeInvocationKey, DashboardRuntimeBaselineRecord>,
    live_core: Option<DashboardActivityLiveSnapshot>,
    source_scope: InvocationSourceScope,
    memory_ready: bool,
}

impl Default for DashboardRuntimeProjectionState {
    fn default() -> Self {
        Self {
            dirty_generation: 0,
            pending_deadline: None,
            network_dirty_generation: 0,
            pending_network_deadline: None,
            terminal_dirty_generation: 0,
            pending_terminal_deadline: None,
            terminal_published_generation: 0,
            current_revision: 0,
            network_revision: 0,
            terminal_revision: 0,
            terminal_pending_delta_bytes: 0,
            terminal_pending_deltas: VecDeque::new(),
            last_good: None,
            network_last_good: None,
            legacy_last_good: None,
            last_good_at: None,
            last_snapshot_origin: None,
            degraded_reason: None,
            reconcile_error: None,
            last_reconcile_defer_reason: None,
            persistence_baseline: None,
            baseline_records: HashMap::new(),
            projection_records: HashMap::new(),
            live_core: None,
            source_scope: InvocationSourceScope::All,
            memory_ready: false,
        }
    }
}

fn empty_dashboard_live_core() -> DashboardActivityLiveSnapshot {
    DashboardActivityLiveSnapshot {
        revision: 0,
        generated_at: String::new(),
        in_progress_invocation_count: 0,
        in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
        retry_invocation_count: 0,
        in_progress_wait_sum_ms: 0.0,
        in_progress_wait_sample_count: 0,
        network_live_bucket: None,
        network_realtime_rate: None,
        accounts: Vec::new(),
    }
}

fn dashboard_projection_record_from_invocation(
    key: RuntimeInvocationKey,
    record: &ApiInvocation,
    previous: Option<&DashboardRuntimeBaselineRecord>,
) -> Option<DashboardRuntimeBaselineRecord> {
    if !matches!(
        normalized_runtime_text(record.status.as_deref()).as_str(),
        "running" | "pending"
    ) {
        return None;
    }
    Some(DashboardRuntimeBaselineRecord {
        key,
        upstream_account_id: record
            .upstream_account_id
            .or_else(|| previous.and_then(|record| record.upstream_account_id)),
        upstream_account_name: normalize_trimmed_optional_string_local(
            record.upstream_account_name.clone(),
        )
        .or_else(|| previous.and_then(|record| record.upstream_account_name.clone())),
        is_retry: record.pool_attempt_count.unwrap_or_default() > 1
            || previous.is_some_and(|record| record.is_retry),
        live_phase: record
            .live_phase
            .clone()
            .or_else(|| runtime_invocation_live_phase(record).map(str::to_string)),
        wait_ms: normalized_wait_ms(record.t_upstream_ttfb_ms),
    })
}

fn update_dashboard_live_core(
    core: &mut DashboardActivityLiveSnapshot,
    record: &DashboardRuntimeBaselineRecord,
    add: bool,
) {
    let delta = if add { 1 } else { -1 };
    core.in_progress_invocation_count = (core.in_progress_invocation_count + delta).max(0);
    if add {
        core.in_progress_phase_counts
            .increment_phase_name(record.live_phase.as_deref());
    } else {
        core.in_progress_phase_counts
            .decrement_phase_name(record.live_phase.as_deref());
    }
    if record.is_retry {
        core.retry_invocation_count = (core.retry_invocation_count + delta).max(0);
    }
    if let Some(wait_ms) = normalized_wait_ms(record.wait_ms) {
        core.in_progress_wait_sum_ms =
            (core.in_progress_wait_sum_ms + if add { wait_ms } else { -wait_ms }).max(0.0);
        core.in_progress_wait_sample_count = (core.in_progress_wait_sample_count + delta).max(0);
    }

    let account_key = record
        .upstream_account_id
        .map(|id| format!("upstream:{id}"))
        .unwrap_or_else(|| "unassigned".to_string());
    let account_index = core
        .accounts
        .iter()
        .position(|account| account.account_key == account_key);
    let account_index = match (account_index, add) {
        (Some(index), _) => index,
        (None, true) => {
            core.accounts.push(DashboardActivityLiveAccount {
                account_key,
                upstream_account_id: record.upstream_account_id,
                upstream_account_name: record.upstream_account_name.clone(),
                in_progress_invocation_count: 0,
                in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
                retry_invocation_count: 0,
                in_progress_wait_sum_ms: 0.0,
                in_progress_wait_sample_count: 0,
                upload_bytes_per_second: 0.0,
                download_bytes_per_second: 0.0,
                network_live_bucket: None,
            });
            core.accounts.len() - 1
        }
        (None, false) => return,
    };
    let account = &mut core.accounts[account_index];
    if account.upstream_account_name.is_none() {
        account.upstream_account_name = record.upstream_account_name.clone();
    }
    account.in_progress_invocation_count = (account.in_progress_invocation_count + delta).max(0);
    if add {
        account
            .in_progress_phase_counts
            .increment_phase_name(record.live_phase.as_deref());
    } else {
        account
            .in_progress_phase_counts
            .decrement_phase_name(record.live_phase.as_deref());
    }
    if record.is_retry {
        account.retry_invocation_count = (account.retry_invocation_count + delta).max(0);
    }
    if let Some(wait_ms) = normalized_wait_ms(record.wait_ms) {
        account.in_progress_wait_sum_ms =
            (account.in_progress_wait_sum_ms + if add { wait_ms } else { -wait_ms }).max(0.0);
        account.in_progress_wait_sample_count =
            (account.in_progress_wait_sample_count + delta).max(0);
    }
    if !add && account.in_progress_invocation_count == 0 {
        core.accounts.swap_remove(account_index);
    }
}

fn mark_dashboard_state_dirty(
    dashboard: &mut DashboardRuntimeProjectionState,
    _trigger: &'static str,
    now: Instant,
) {
    dashboard.dirty_generation = dashboard.dirty_generation.saturating_add(1);
    dashboard.memory_ready = true;
    if dashboard.pending_deadline.is_none() {
        dashboard.pending_deadline = Some(now + DASHBOARD_RUNTIME_PROJECTION_COALESCE);
    }
}

fn mark_dashboard_projection_slice_dirty(
    generation: &mut u64,
    deadline: &mut Option<Instant>,
    now: Instant,
    cadence: Duration,
) {
    *generation = generation.saturating_add(1);
    if deadline.is_none() {
        *deadline = Some(now + cadence);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardRuntimeBaselineRecord {
    pub(crate) key: RuntimeInvocationKey,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) is_retry: bool,
    pub(crate) live_phase: Option<String>,
    pub(crate) wait_ms: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardRuntimeProjectionBaseline {
    pub(crate) records: Vec<DashboardRuntimeBaselineRecord>,
    pub(crate) source_scope: InvocationSourceScope,
    pub(crate) network_open_buckets:
        HashMap<DashboardNetworkScopeKey, DashboardRuntimeNetworkOpenBucketBaseline>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DashboardRuntimeNetworkOpenBucketBaseline {
    pub(crate) bucket_start: DateTime<Utc>,
    pub(crate) bucket_end: DateTime<Utc>,
    pub(crate) baseline_totals: DashboardNetworkByteTotals,
    pub(crate) memory_totals_at_install: DashboardNetworkByteTotals,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DashboardProjectionPublishWindow {
    pub(crate) slice: DashboardProjectionSlice,
    pub(crate) deadline: Instant,
    pub(crate) generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashboardProjectionSlice {
    Current,
    Network,
    Terminal,
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardProjectionCapture {
    pub(crate) snapshot: DashboardActivityLiveSnapshot,
    pub(crate) changed: bool,
    pub(crate) snapshot_origin: &'static str,
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardNetworkProjectionCapture {
    pub(crate) slice: DashboardNetworkProjectionSlice,
    pub(crate) changed: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardTerminalProjectionCapture {
    pub(crate) revision: u64,
    pub(crate) deltas: Vec<DashboardActivityTerminalDelta>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DashboardProjectionSliceCounterSnapshot {
    pub(crate) build_count: u64,
    pub(crate) revision_count: u64,
    pub(crate) cadence_miss_count: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DashboardRuntimeTopologyCounterSnapshot {
    pub(crate) current: DashboardProjectionSliceCounterSnapshot,
    pub(crate) network: DashboardProjectionSliceCounterSnapshot,
    pub(crate) terminal: DashboardProjectionSliceCounterSnapshot,
}

#[derive(Debug, Default)]
struct DashboardProjectionSliceCounters {
    build_count: AtomicU64,
    revision_count: AtomicU64,
    cadence_miss_count: AtomicU64,
}

impl DashboardProjectionSliceCounters {
    fn snapshot(&self) -> DashboardProjectionSliceCounterSnapshot {
        DashboardProjectionSliceCounterSnapshot {
            build_count: self.build_count.load(Ordering::Relaxed),
            revision_count: self.revision_count.load(Ordering::Relaxed),
            cadence_miss_count: self.cadence_miss_count.load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    fn reset(&self) {
        self.build_count.store(0, Ordering::Relaxed);
        self.revision_count.store(0, Ordering::Relaxed);
        self.cadence_miss_count.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug, Default)]
struct DashboardRuntimeTopologyCounters {
    current: DashboardProjectionSliceCounters,
    network: DashboardProjectionSliceCounters,
    terminal: DashboardProjectionSliceCounters,
}

impl DashboardRuntimeTopologyCounters {
    fn snapshot(&self) -> DashboardRuntimeTopologyCounterSnapshot {
        DashboardRuntimeTopologyCounterSnapshot {
            current: self.current.snapshot(),
            network: self.network.snapshot(),
            terminal: self.terminal.snapshot(),
        }
    }

    #[cfg(test)]
    fn reset(&self) {
        self.current.reset();
        self.network.reset();
        self.terminal.reset();
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeProjectionHealthSnapshot {
    pub(crate) mode: String,
    pub(crate) state: String,
    pub(crate) producer_state: String,
    pub(crate) active_subscriber_count: u64,
    pub(crate) live_path_db_read_count: u64,
    pub(crate) build_count: u64,
    pub(crate) revision: u64,
    pub(crate) snapshot_origin: String,
    pub(crate) last_good_age_ms: Option<u64>,
    pub(crate) degraded_reason: Option<String>,
    pub(crate) last_defer_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestPipelineHealthSnapshot {
    pub(crate) mode: String,
    pub(crate) last_snapshot_kind: String,
    pub(crate) semantic_parse_count: u64,
    pub(crate) whole_body_materialization_count: u64,
    pub(crate) rewrite_buffer_peak_bytes: u64,
    pub(crate) last_fallback_reason: Option<String>,
}

#[derive(Debug, Default)]
struct RequestPipelineLastState {
    snapshot_kind: String,
    fallback_reason: Option<String>,
}

#[derive(Debug)]
pub(crate) struct RuntimeProjectionHub {
    pub(crate) inner: std::sync::Mutex<ProxyRuntimeInvocationStoreInner>,
    mode: RuntimeProjectionMode,
    dashboard_network_speed_cache: std::sync::OnceLock<Arc<DashboardNetworkSpeedCache>>,
    dashboard: std::sync::Mutex<DashboardRuntimeProjectionState>,
    dashboard_publish_notify: tokio::sync::Notify,
    live_path_db_read_count: AtomicU64,
    build_count: AtomicU64,
    dashboard_topology_counters: DashboardRuntimeTopologyCounters,
    producer_running: AtomicBool,
    request_semantic_parse_count: AtomicU64,
    request_whole_body_materialization_count: AtomicU64,
    request_rewrite_buffer_peak_bytes: AtomicU64,
    request_pipeline_last: std::sync::Mutex<RequestPipelineLastState>,
}

pub(crate) type ProxyRuntimeInvocationStore = RuntimeProjectionHub;

impl Default for RuntimeProjectionHub {
    fn default() -> Self {
        Self::new(RuntimeProjectionMode::Auto)
    }
}

impl RuntimeProjectionHub {
    pub(crate) fn new(mode: RuntimeProjectionMode) -> Self {
        Self {
            inner: std::sync::Mutex::new(ProxyRuntimeInvocationStoreInner::default()),
            mode,
            dashboard_network_speed_cache: std::sync::OnceLock::new(),
            dashboard: std::sync::Mutex::new(DashboardRuntimeProjectionState::default()),
            dashboard_publish_notify: tokio::sync::Notify::new(),
            live_path_db_read_count: AtomicU64::new(0),
            build_count: AtomicU64::new(0),
            dashboard_topology_counters: DashboardRuntimeTopologyCounters::default(),
            producer_running: AtomicBool::new(false),
            request_semantic_parse_count: AtomicU64::new(0),
            request_whole_body_materialization_count: AtomicU64::new(0),
            request_rewrite_buffer_peak_bytes: AtomicU64::new(0),
            request_pipeline_last: std::sync::Mutex::new(RequestPipelineLastState::default()),
        }
    }

    pub(crate) fn mode(&self) -> RuntimeProjectionMode {
        self.mode
    }

    pub(crate) fn bind_dashboard_network_speed_cache(
        &self,
        cache: Arc<DashboardNetworkSpeedCache>,
    ) -> Result<()> {
        if let Some(existing) = self.dashboard_network_speed_cache.get() {
            return if Arc::ptr_eq(existing, &cache) {
                Ok(())
            } else {
                Err(anyhow!(
                    "runtime projection hub is already bound to another dashboard network cache"
                ))
            };
        }
        self.dashboard_network_speed_cache
            .set(cache)
            .map_err(|_| anyhow!("dashboard network speed cache is already bound"))
    }

    pub(crate) fn dashboard_live_projection(&self) -> DashboardLiveProjection<'_> {
        DashboardLiveProjection { hub: self }
    }

    fn update_dashboard_runtime_record(
        &self,
        key: RuntimeInvocationKey,
        record: Option<&ApiInvocation>,
        trigger: &'static str,
    ) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        if record.is_none() {
            dashboard.baseline_records.remove(&key);
        }
        let previous = dashboard.projection_records.remove(&key);
        if let Some(previous) = previous.as_ref() {
            update_dashboard_live_core(
                dashboard
                    .live_core
                    .get_or_insert_with(empty_dashboard_live_core),
                previous,
                false,
            );
        }
        let next = record.and_then(|record| {
            if dashboard.source_scope == InvocationSourceScope::ProxyOnly
                && record.source != SOURCE_PROXY
            {
                None
            } else {
                dashboard_projection_record_from_invocation(key.clone(), record, previous.as_ref())
            }
        });
        if let Some(next) = next {
            update_dashboard_live_core(
                dashboard
                    .live_core
                    .get_or_insert_with(empty_dashboard_live_core),
                &next,
                true,
            );
            dashboard.projection_records.insert(key, next);
        }
        dashboard
            .live_core
            .get_or_insert_with(empty_dashboard_live_core)
            .accounts
            .sort_by(|left, right| left.account_key.cmp(&right.account_key));
        mark_dashboard_state_dirty(&mut dashboard, trigger, Instant::now());
        self.dashboard_publish_notify.notify_one();
    }

    fn sync_dashboard_runtime_key(&self, key: &RuntimeInvocationKey, trigger: &'static str) {
        let Ok(runtime) = self.inner.lock() else {
            return;
        };
        let record = runtime.records.get(key).map(|entry| &entry.record);
        self.update_dashboard_runtime_record(key.clone(), record, trigger);
    }

    fn rebuild_dashboard_runtime_records(&self, trigger: &'static str) {
        let Ok(runtime) = self.inner.lock() else {
            return;
        };
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        let source_scope = dashboard.source_scope;
        let mut records = dashboard.baseline_records.clone();
        for key in runtime.terminal_tombstones.keys() {
            records.remove(key);
        }
        for key in runtime.projection_tombstones.keys() {
            records.remove(key);
        }
        for (key, entry) in &runtime.records {
            if runtime.terminal_tombstones.contains_key(key) {
                records.remove(key);
                continue;
            }
            if source_scope == InvocationSourceScope::ProxyOnly
                && entry.record.source != SOURCE_PROXY
            {
                continue;
            }
            let previous = records.get(key);
            match dashboard_projection_record_from_invocation(key.clone(), &entry.record, previous)
            {
                Some(projected) => {
                    records.insert(key.clone(), projected);
                }
                None => {
                    records.remove(key);
                }
            }
        }
        let mut core = empty_dashboard_live_core();
        for record in records.values() {
            update_dashboard_live_core(&mut core, record, true);
        }
        core.accounts
            .sort_by(|left, right| left.account_key.cmp(&right.account_key));
        dashboard.projection_records = records;
        dashboard.live_core = Some(core);
        mark_dashboard_state_dirty(&mut dashboard, trigger, Instant::now());
        self.dashboard_publish_notify.notify_one();
    }

    pub(crate) fn mark_dashboard_dirty(&self, trigger: &'static str) {
        self.mark_dashboard_dirty_at(trigger, Instant::now());
    }

    pub(crate) fn mark_dashboard_dirty_at(&self, _trigger: &'static str, now: Instant) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        mark_dashboard_state_dirty(&mut dashboard, _trigger, now);
        self.dashboard_publish_notify.notify_one();
    }

    pub(crate) fn mark_dashboard_network_dirty(&self) {
        self.mark_dashboard_network_dirty_at(Instant::now());
    }

    fn mark_dashboard_network_dirty_at(&self, now: Instant) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        let DashboardRuntimeProjectionState {
            network_dirty_generation,
            pending_network_deadline,
            ..
        } = &mut *dashboard;
        mark_dashboard_projection_slice_dirty(
            network_dirty_generation,
            pending_network_deadline,
            now,
            DASHBOARD_RUNTIME_NETWORK_PROJECTION_COALESCE,
        );
        self.dashboard_publish_notify.notify_one();
    }

    fn mark_dashboard_terminal_dirty(&self) {
        self.mark_dashboard_terminal_dirty_at(Instant::now());
    }

    fn mark_dashboard_terminal_dirty_at(&self, now: Instant) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        let DashboardRuntimeProjectionState {
            terminal_dirty_generation,
            pending_terminal_deadline,
            ..
        } = &mut *dashboard;
        mark_dashboard_projection_slice_dirty(
            terminal_dirty_generation,
            pending_terminal_deadline,
            now,
            DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE,
        );
        self.dashboard_publish_notify.notify_one();
    }

    pub(crate) async fn wait_for_dashboard_publish_signal(&self) {
        self.dashboard_publish_notify.notified().await;
    }

    pub(crate) fn pending_dashboard_deadline(&self) -> Option<Instant> {
        self.dashboard
            .lock()
            .ok()
            .and_then(|dashboard| dashboard.pending_deadline)
    }

    pub(crate) fn pending_dashboard_publish_window(
        &self,
    ) -> Option<DashboardProjectionPublishWindow> {
        let dashboard = self.dashboard.lock().ok()?;
        [
            (
                DashboardProjectionSlice::Current,
                dashboard.pending_deadline,
                dashboard.dirty_generation,
            ),
            (
                DashboardProjectionSlice::Network,
                dashboard.pending_network_deadline,
                dashboard.network_dirty_generation,
            ),
            (
                DashboardProjectionSlice::Terminal,
                dashboard.pending_terminal_deadline,
                dashboard.terminal_dirty_generation,
            ),
        ]
        .into_iter()
        .filter_map(|(slice, deadline, generation)| {
            deadline.map(|deadline| DashboardProjectionPublishWindow {
                slice,
                deadline,
                generation,
            })
        })
        .min_by_key(|window| window.deadline)
    }

    pub(crate) fn has_pending_dashboard_terminal_publish(&self) -> bool {
        self.dashboard
            .lock()
            .map(|dashboard| dashboard.pending_terminal_deadline.is_some())
            .unwrap_or(false)
    }

    pub(crate) fn begin_dashboard_publish_window(
        &self,
        window: DashboardProjectionPublishWindow,
    ) -> Option<DashboardProjectionPublishWindow> {
        let mut dashboard = self.dashboard.lock().ok()?;
        let generation = match window.slice {
            DashboardProjectionSlice::Current => {
                if dashboard.pending_deadline != Some(window.deadline) {
                    return None;
                }
                dashboard.pending_deadline = None;
                dashboard.dirty_generation
            }
            DashboardProjectionSlice::Network => {
                if dashboard.pending_network_deadline != Some(window.deadline) {
                    return None;
                }
                dashboard.pending_network_deadline = None;
                dashboard.network_dirty_generation
            }
            DashboardProjectionSlice::Terminal => {
                if dashboard.pending_terminal_deadline != Some(window.deadline) {
                    return None;
                }
                dashboard.pending_terminal_deadline = None;
                dashboard.terminal_dirty_generation
            }
        };
        Some(DashboardProjectionPublishWindow {
            slice: window.slice,
            deadline: window.deadline,
            generation,
        })
    }

    pub(crate) fn complete_dashboard_publish_window(
        &self,
        window: DashboardProjectionPublishWindow,
    ) {
        if let Ok(dashboard) = self.dashboard.lock() {
            let generation = match window.slice {
                DashboardProjectionSlice::Current => dashboard.dirty_generation,
                DashboardProjectionSlice::Network => dashboard.network_dirty_generation,
                DashboardProjectionSlice::Terminal => dashboard.terminal_dirty_generation,
            };
            debug_assert!(generation >= window.generation);
        }
    }

    pub(crate) fn is_memory_ready(&self) -> bool {
        self.dashboard
            .lock()
            .map(|dashboard| dashboard.memory_ready && dashboard.degraded_reason.is_none())
            .unwrap_or(false)
    }

    pub(crate) fn dashboard_generation(&self) -> u64 {
        self.dashboard
            .lock()
            .map(|dashboard| dashboard.dirty_generation)
            .unwrap_or_default()
    }

    pub(crate) fn capture_memory_snapshot(&self) -> Result<DashboardProjectionCapture> {
        let candidate = self.dashboard_live_projection().snapshot()?;
        self.build_count.fetch_add(1, Ordering::Relaxed);
        self.dashboard_topology_counters
            .current
            .build_count
            .fetch_add(1, Ordering::Relaxed);
        let mut dashboard = self
            .dashboard
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?;
        let changed = dashboard
            .last_good
            .as_ref()
            .is_none_or(|current| !dashboard_current_snapshot_content_eq(current, &candidate));
        if !changed {
            let revision = dashboard
                .last_good
                .as_ref()
                .expect("unchanged projection has a last-good snapshot")
                .revision;
            let mut snapshot = candidate;
            snapshot.revision = revision;
            dashboard.last_good = Some(snapshot.clone());
            dashboard.memory_ready = true;
            dashboard.degraded_reason = None;
            dashboard.last_snapshot_origin = Some("memory");
            return Ok(DashboardProjectionCapture {
                snapshot,
                changed: false,
                snapshot_origin: "memory",
            });
        }

        dashboard.current_revision = dashboard.current_revision.saturating_add(1);
        let mut snapshot = candidate;
        snapshot.revision = dashboard.current_revision;
        self.dashboard_topology_counters
            .current
            .revision_count
            .fetch_add(1, Ordering::Relaxed);
        dashboard.last_good = Some(snapshot.clone());
        dashboard.last_good_at = Some(Instant::now());
        dashboard.last_snapshot_origin = Some("memory");
        dashboard.degraded_reason = None;
        dashboard.memory_ready = true;
        Ok(DashboardProjectionCapture {
            snapshot,
            changed: true,
            snapshot_origin: "memory",
        })
    }

    pub(crate) fn capture_network_slice(&self) -> Result<DashboardNetworkProjectionCapture> {
        let dashboard_network_speed_cache = self
            .dashboard_network_speed_cache
            .get()
            .ok_or_else(|| anyhow!("dashboard network speed cache is not bound"))?;
        let network_open_buckets = self
            .dashboard
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?
            .persistence_baseline
            .as_ref()
            .map(|baseline| baseline.network_open_buckets.clone())
            .unwrap_or_default();
        let candidate = DashboardNetworkProjectionSlice::from_memory(
            dashboard_network_speed_cache.as_ref(),
            &network_open_buckets,
        );
        self.dashboard_topology_counters
            .network
            .build_count
            .fetch_add(1, Ordering::Relaxed);
        let mut dashboard = self
            .dashboard
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?;
        let changed = dashboard
            .network_last_good
            .as_ref()
            .is_none_or(|current| !dashboard_network_slice_content_eq(current, &candidate));
        if !changed {
            return Ok(DashboardNetworkProjectionCapture {
                slice: dashboard
                    .network_last_good
                    .clone()
                    .expect("unchanged network projection has a last-good snapshot"),
                changed: false,
            });
        }

        dashboard.network_revision = dashboard.network_revision.saturating_add(1);
        let mut snapshot = candidate;
        snapshot.revision = dashboard.network_revision;
        self.dashboard_topology_counters
            .network
            .revision_count
            .fetch_add(1, Ordering::Relaxed);
        dashboard.network_last_good = Some(snapshot.clone());
        Ok(DashboardNetworkProjectionCapture {
            slice: snapshot,
            changed: true,
        })
    }

    pub(crate) fn record_dashboard_terminal_delta(&self, delta: DashboardActivityTerminalDelta) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        if dashboard.terminal_pending_deltas.len() >= DASHBOARD_RUNTIME_TERMINAL_MAX_PENDING
            || dashboard
                .terminal_pending_delta_bytes
                .saturating_add(delta.estimated_bytes)
                > DASHBOARD_RUNTIME_TERMINAL_MAX_PENDING_BYTES
        {
            dashboard.degraded_reason = Some("terminal_slice_hard_limit");
            tracing::warn!(
                pending_delta_count = dashboard.terminal_pending_deltas.len(),
                pending_delta_estimated_bytes = dashboard.terminal_pending_delta_bytes,
                "dashboard terminal projection slice reached its hard limit"
            );
            return;
        }
        dashboard.terminal_pending_delta_bytes = dashboard
            .terminal_pending_delta_bytes
            .saturating_add(delta.estimated_bytes);
        dashboard.terminal_pending_deltas.push_back(delta);
        let now = Instant::now();
        let DashboardRuntimeProjectionState {
            terminal_dirty_generation,
            pending_terminal_deadline,
            ..
        } = &mut *dashboard;
        mark_dashboard_projection_slice_dirty(
            terminal_dirty_generation,
            pending_terminal_deadline,
            now,
            DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE,
        );
        self.dashboard_publish_notify.notify_one();
    }

    pub(crate) fn discard_dashboard_terminal_delta(&self, invoke_id: &str, occurred_at: &str) {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return;
        };
        let mut removed_bytes = 0usize;
        dashboard.terminal_pending_deltas.retain(|delta| {
            let retain = delta.invoke_id != invoke_id || delta.occurred_at != occurred_at;
            if !retain {
                removed_bytes = removed_bytes.saturating_add(delta.estimated_bytes);
            }
            retain
        });
        dashboard.terminal_pending_delta_bytes = dashboard
            .terminal_pending_delta_bytes
            .saturating_sub(removed_bytes);
    }

    pub(crate) fn capture_terminal_slice(&self) -> Option<DashboardTerminalProjectionCapture> {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            return None;
        };
        self.dashboard_topology_counters
            .terminal
            .build_count
            .fetch_add(1, Ordering::Relaxed);
        if dashboard.terminal_published_generation == dashboard.terminal_dirty_generation {
            return None;
        }
        dashboard.terminal_published_generation = dashboard.terminal_dirty_generation;
        let deltas = std::mem::take(&mut dashboard.terminal_pending_deltas)
            .into_iter()
            .collect::<Vec<_>>();
        dashboard.terminal_pending_delta_bytes = 0;
        if deltas.is_empty() {
            return None;
        }
        dashboard.terminal_revision = dashboard.terminal_revision.saturating_add(1);
        self.dashboard_topology_counters
            .terminal
            .revision_count
            .fetch_add(1, Ordering::Relaxed);
        Some(DashboardTerminalProjectionCapture {
            revision: dashboard.terminal_revision,
            deltas,
        })
    }

    #[cfg(test)]
    pub(crate) fn pending_terminal_slice_count(&self) -> usize {
        self.dashboard
            .lock()
            .map(|dashboard| dashboard.terminal_pending_deltas.len())
            .unwrap_or_default()
    }

    pub(crate) fn legacy_live_snapshot(
        &self,
        mut current: DashboardActivityLiveSnapshot,
    ) -> DashboardActivityLiveSnapshot {
        if let Ok(dashboard) = self.dashboard.lock()
            && let Some(network) = dashboard.network_last_good.as_ref()
        {
            apply_dashboard_network_slice_to_live_snapshot(&mut current, network);
        }
        self.commit_legacy_live_snapshot(current)
    }

    pub(crate) fn legacy_live_snapshot_for_network(
        &self,
        network: &DashboardNetworkProjectionSlice,
    ) -> Option<DashboardActivityLiveSnapshot> {
        let mut current = self.dashboard.lock().ok()?.last_good.clone()?;
        apply_dashboard_network_slice_to_live_snapshot(&mut current, network);
        Some(self.commit_legacy_live_snapshot(current))
    }

    fn commit_legacy_live_snapshot(
        &self,
        mut candidate: DashboardActivityLiveSnapshot,
    ) -> DashboardActivityLiveSnapshot {
        let Ok(mut dashboard) = self.dashboard.lock() else {
            candidate.revision = reserve_dashboard_activity_live_revision();
            return candidate;
        };
        if let Some(previous) = dashboard.legacy_last_good.as_ref()
            && dashboard_legacy_snapshot_content_eq(previous, &candidate)
        {
            return previous.clone();
        }
        candidate.revision = reserve_dashboard_activity_live_revision();
        dashboard.legacy_last_good = Some(candidate.clone());
        candidate
    }

    pub(crate) fn install_persistence_baseline_if_generation(
        &self,
        _snapshot: DashboardActivityLiveSnapshot,
        mut baseline: DashboardRuntimeProjectionBaseline,
        snapshot_origin: &'static str,
        _expected_generation: u64,
    ) -> Result<Option<DashboardProjectionCapture>> {
        let mut projection_records = baseline
            .records
            .iter()
            .cloned()
            .map(|record| (record.key.clone(), record))
            .collect::<HashMap<_, _>>();
        let baseline_records = projection_records.clone();
        let runtime = self
            .inner
            .lock()
            .map_err(|_| anyhow!("runtime invocation store lock is poisoned"))?;
        for key in runtime.terminal_tombstones.keys() {
            projection_records.remove(key);
        }
        for key in runtime.projection_tombstones.keys() {
            projection_records.remove(key);
        }
        for (key, entry) in &runtime.records {
            if runtime.terminal_tombstones.contains_key(key) {
                projection_records.remove(key);
                continue;
            }
            if baseline.source_scope == InvocationSourceScope::ProxyOnly
                && entry.record.source != SOURCE_PROXY
            {
                continue;
            }
            let previous = projection_records.get(key);
            match dashboard_projection_record_from_invocation(key.clone(), &entry.record, previous)
            {
                Some(record) => {
                    projection_records.insert(key.clone(), record);
                }
                None => {
                    projection_records.remove(key);
                }
            }
        }
        let mut dashboard = self
            .dashboard
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?;
        let mut core = empty_dashboard_live_core();
        for record in projection_records.values() {
            update_dashboard_live_core(&mut core, record, true);
        }
        core.accounts
            .sort_by(|left, right| left.account_key.cmp(&right.account_key));
        let mut snapshot = core.clone();
        let changed = dashboard
            .last_good
            .as_ref()
            .is_none_or(|current| !dashboard_current_snapshot_content_eq(current, &snapshot));
        let snapshot = if changed {
            dashboard.current_revision = dashboard.current_revision.saturating_add(1);
            snapshot.revision = dashboard.current_revision;
            self.dashboard_topology_counters
                .current
                .revision_count
                .fetch_add(1, Ordering::Relaxed);
            dashboard.last_good = Some(snapshot.clone());
            dashboard.last_good_at = Some(Instant::now());
            snapshot
        } else {
            snapshot.revision = dashboard
                .last_good
                .as_ref()
                .expect("unchanged baseline has a last-good snapshot")
                .revision;
            dashboard.last_good = Some(snapshot.clone());
            snapshot
        };
        dashboard.last_snapshot_origin = Some(snapshot_origin);
        dashboard.degraded_reason = None;
        dashboard.reconcile_error = None;
        dashboard.last_reconcile_defer_reason = None;
        dashboard.source_scope = baseline.source_scope;
        dashboard.baseline_records = baseline_records;
        dashboard.projection_records = projection_records;
        dashboard.live_core = Some(core);
        baseline.records.clear();
        dashboard.persistence_baseline = Some(baseline);
        dashboard.memory_ready = false;
        Ok(Some(DashboardProjectionCapture {
            snapshot,
            changed,
            snapshot_origin,
        }))
    }

    pub(crate) fn last_good_capture(
        &self,
        snapshot_origin: &'static str,
    ) -> Option<DashboardProjectionCapture> {
        let mut dashboard = self.dashboard.lock().ok()?;
        let snapshot = dashboard.last_good.clone()?;
        dashboard.last_snapshot_origin = Some(snapshot_origin);
        Some(DashboardProjectionCapture {
            snapshot,
            changed: false,
            snapshot_origin,
        })
    }

    pub(crate) fn mark_degraded(&self, reason: &'static str) {
        if let Ok(mut dashboard) = self.dashboard.lock() {
            dashboard.degraded_reason = Some(reason);
        }
    }

    pub(crate) fn record_reconcile_failure(&self, reason: &'static str) {
        if let Ok(mut dashboard) = self.dashboard.lock() {
            dashboard.reconcile_error = Some(reason);
            dashboard.last_reconcile_defer_reason = None;
        }
    }

    pub(crate) fn record_reconcile_deferred(&self, reason: &'static str) {
        if let Ok(mut dashboard) = self.dashboard.lock() {
            dashboard.last_reconcile_defer_reason = Some(reason);
        }
    }

    pub(crate) fn record_live_path_db_read(&self) {
        self.live_path_db_read_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_build(&self) {
        self.build_count.fetch_add(1, Ordering::Relaxed);
        self.dashboard_topology_counters
            .current
            .build_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_current_slice_cadence_miss(&self) {
        self.dashboard_topology_counters
            .current
            .cadence_miss_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_network_slice_cadence_miss(&self) {
        self.dashboard_topology_counters
            .network
            .cadence_miss_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_terminal_slice_cadence_miss(&self) {
        self.dashboard_topology_counters
            .terminal
            .cadence_miss_count
            .fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn dashboard_topology_counters(&self) -> DashboardRuntimeTopologyCounterSnapshot {
        self.dashboard_topology_counters.snapshot()
    }

    #[cfg(test)]
    pub(crate) fn reset_dashboard_topology_counters(&self) {
        self.dashboard_topology_counters.reset();
    }

    pub(crate) fn set_producer_running(&self, running: bool) {
        self.producer_running.store(running, Ordering::Release);
    }

    pub(crate) fn record_request_pipeline(
        &self,
        snapshot_kind: &str,
        semantic_parse_count: u8,
        whole_body_materialization_count: u8,
        rewrite_buffer_bytes: usize,
        fallback_reason: Option<&str>,
    ) {
        self.request_semantic_parse_count
            .fetch_add(u64::from(semantic_parse_count), Ordering::Relaxed);
        self.request_whole_body_materialization_count.fetch_add(
            u64::from(whole_body_materialization_count),
            Ordering::Relaxed,
        );
        self.request_rewrite_buffer_peak_bytes
            .fetch_max(rewrite_buffer_bytes as u64, Ordering::Relaxed);
        if let Ok(mut last) = self.request_pipeline_last.lock() {
            last.snapshot_kind.clear();
            last.snapshot_kind.push_str(snapshot_kind);
            last.fallback_reason = fallback_reason.map(str::to_string);
        }
    }

    pub(crate) fn request_pipeline_health_snapshot(&self) -> RequestPipelineHealthSnapshot {
        let last = self.request_pipeline_last.lock().ok();
        RequestPipelineHealthSnapshot {
            mode: request_semantic_pipeline_mode().as_str().to_string(),
            last_snapshot_kind: last
                .as_ref()
                .map(|last| last.snapshot_kind.as_str())
                .filter(|value| !value.is_empty())
                .unwrap_or("none")
                .to_string(),
            semantic_parse_count: self.request_semantic_parse_count.load(Ordering::Relaxed),
            whole_body_materialization_count: self
                .request_whole_body_materialization_count
                .load(Ordering::Relaxed),
            rewrite_buffer_peak_bytes: self
                .request_rewrite_buffer_peak_bytes
                .load(Ordering::Relaxed),
            last_fallback_reason: last.and_then(|last| last.fallback_reason.clone()),
        }
    }

    pub(crate) fn health_snapshot(
        &self,
        active_subscriber_count: usize,
    ) -> RuntimeProjectionHealthSnapshot {
        let dashboard = self.dashboard.lock().ok();
        let last_good_age_ms = dashboard
            .as_ref()
            .and_then(|state| state.last_good_at)
            .map(|captured_at| captured_at.elapsed().as_millis() as u64);
        let state = match dashboard.as_ref() {
            Some(state) if state.degraded_reason.is_some() || state.reconcile_error.is_some() => {
                "degraded"
            }
            Some(state) if state.memory_ready || state.last_good.is_some() => "healthy",
            _ => "cold",
        };
        RuntimeProjectionHealthSnapshot {
            mode: self.mode.as_str().to_string(),
            state: state.to_string(),
            producer_state: if self.producer_running.load(Ordering::Acquire) {
                "running".to_string()
            } else {
                "idle".to_string()
            },
            active_subscriber_count: active_subscriber_count as u64,
            live_path_db_read_count: self.live_path_db_read_count.load(Ordering::Relaxed),
            build_count: self.build_count.load(Ordering::Relaxed),
            revision: dashboard
                .as_ref()
                .and_then(|state| state.last_good.as_ref())
                .map_or(0, |snapshot| snapshot.revision),
            snapshot_origin: dashboard
                .as_ref()
                .and_then(|state| state.last_snapshot_origin)
                .unwrap_or("none")
                .to_string(),
            last_good_age_ms,
            degraded_reason: dashboard
                .as_ref()
                .and_then(|state| state.degraded_reason.or(state.reconcile_error))
                .map(str::to_string),
            last_defer_reason: dashboard
                .as_ref()
                .and_then(|state| state.last_reconcile_defer_reason)
                .map(str::to_string),
        }
    }
}

pub(crate) struct DashboardLiveProjection<'a> {
    hub: &'a RuntimeProjectionHub,
}

impl DashboardLiveProjection<'_> {
    pub(crate) fn snapshot(&self) -> Result<DashboardActivityLiveSnapshot> {
        self.hub
            .dashboard
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))
            .map(|dashboard| {
                dashboard
                    .live_core
                    .clone()
                    .unwrap_or_else(empty_dashboard_live_core)
            })
    }
}

fn dashboard_current_snapshot_content_eq(
    left: &DashboardActivityLiveSnapshot,
    right: &DashboardActivityLiveSnapshot,
) -> bool {
    left.in_progress_invocation_count == right.in_progress_invocation_count
        && left.in_progress_phase_counts == right.in_progress_phase_counts
        && left.retry_invocation_count == right.retry_invocation_count
        && left.in_progress_wait_sum_ms == right.in_progress_wait_sum_ms
        && left.in_progress_wait_sample_count == right.in_progress_wait_sample_count
        && left.accounts.len() == right.accounts.len()
        && left.accounts.iter().all(|left| {
            right
                .accounts
                .iter()
                .find(|right| right.account_key == left.account_key)
                .is_some_and(|right| {
                    left.upstream_account_id == right.upstream_account_id
                        && left.upstream_account_name == right.upstream_account_name
                        && left.in_progress_invocation_count == right.in_progress_invocation_count
                        && left.in_progress_phase_counts == right.in_progress_phase_counts
                        && left.retry_invocation_count == right.retry_invocation_count
                        && left.in_progress_wait_sum_ms == right.in_progress_wait_sum_ms
                        && left.in_progress_wait_sample_count == right.in_progress_wait_sample_count
                })
        })
}

fn dashboard_legacy_snapshot_content_eq(
    left: &DashboardActivityLiveSnapshot,
    right: &DashboardActivityLiveSnapshot,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    left.revision = 0;
    right.revision = 0;
    left.generated_at.clear();
    right.generated_at.clear();
    left == right
}

fn dashboard_network_slice_content_eq(
    left: &DashboardNetworkProjectionSlice,
    right: &DashboardNetworkProjectionSlice,
) -> bool {
    left.network_live_bucket == right.network_live_bucket
        && left.network_realtime_rate == right.network_realtime_rate
        && left.accounts.len() == right.accounts.len()
        && left.accounts.iter().all(|left| {
            right
                .accounts
                .iter()
                .find(|right| right.account_key == left.account_key)
                .is_some_and(|right| {
                    left.upload_bytes_per_second == right.upload_bytes_per_second
                        && left.download_bytes_per_second == right.download_bytes_per_second
                        && left.network_live_bucket == right.network_live_bucket
                })
        })
}

fn apply_dashboard_network_slice_to_live_snapshot(
    current: &mut DashboardActivityLiveSnapshot,
    network: &DashboardNetworkProjectionSlice,
) {
    current.network_live_bucket = network.network_live_bucket.clone();
    current.network_realtime_rate = network.network_realtime_rate.clone();
    for account in &mut current.accounts {
        let Some(network_account) = network
            .accounts
            .iter()
            .find(|candidate| candidate.account_key == account.account_key)
        else {
            account.upload_bytes_per_second = 0.0;
            account.download_bytes_per_second = 0.0;
            account.network_live_bucket = None;
            continue;
        };
        account.upload_bytes_per_second = network_account.upload_bytes_per_second;
        account.download_bytes_per_second = network_account.download_bytes_per_second;
        account.network_live_bucket = network_account.network_live_bucket.clone();
    }
}

#[derive(Debug, Default)]
pub(crate) struct ProxyRuntimeInvocationStoreInner {
    pub(crate) records: HashMap<RuntimeInvocationKey, RuntimeInvocationEntry>,
    pub(crate) terminal_tombstones: HashMap<RuntimeInvocationKey, Instant>,
    projection_tombstones: HashMap<RuntimeInvocationKey, Instant>,
}

pub(crate) const PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE: Duration =
    Duration::from_secs(6 * 60 * 60);
pub(crate) const PROXY_RUNTIME_INVOCATION_STORE_MAX_RECORDS: usize = 10_000;
pub(crate) const PROXY_RUNTIME_INVOCATION_TERMINAL_TOMBSTONE_MAX_RECORDS: usize = 50_000;

impl RuntimeProjectionHub {
    pub(crate) fn runtime_record_count(&self) -> usize {
        self.inner
            .lock()
            .map(|guard| guard.records.len())
            .unwrap_or_default()
    }

    pub(crate) fn memory_estimate(&self) -> MemoryComponentEstimate {
        let Ok(guard) = self.inner.lock() else {
            return MemoryComponentEstimate::default();
        };
        let record_bytes = guard
            .records
            .values()
            .map(|entry| entry.record.estimated_memory_bytes())
            .sum::<usize>();
        let key_bytes = guard
            .records
            .keys()
            .chain(guard.terminal_tombstones.keys())
            .chain(guard.projection_tombstones.keys())
            .map(|key| key.invoke_id.capacity() + key.occurred_at.capacity())
            .sum::<usize>();
        MemoryComponentEstimate {
            entries: guard
                .records
                .len()
                .saturating_add(guard.terminal_tombstones.len())
                .saturating_add(guard.projection_tombstones.len()),
            bytes: record_bytes.saturating_add(key_bytes).saturating_add(
                (guard.records.capacity()
                    + guard.terminal_tombstones.capacity()
                    + guard.projection_tombstones.capacity())
                .saturating_mul(std::mem::size_of::<usize>() * 2),
            ),
            detail_items: guard.records.len(),
        }
    }

    pub(crate) fn upsert(&self, record: ApiInvocation) -> RuntimeInvocationStoreUpsertOutcome {
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(record.invoke_id.clone(), record.occurred_at.clone());
        let Ok(mut guard) = self.inner.lock() else {
            return RuntimeInvocationStoreUpsertOutcome {
                running_count: 0,
                pruned_count: 0,
                skipped_terminal: false,
            };
        };
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        let terminal_overlay_exists = guard
            .records
            .get(&key)
            .is_some_and(|entry| runtime_store_record_is_terminal(&entry.record));
        if guard.terminal_tombstones.contains_key(&key) || terminal_overlay_exists {
            let outcome = RuntimeInvocationStoreUpsertOutcome {
                running_count: guard.records.len(),
                pruned_count,
                skipped_terminal: true,
            };
            drop(guard);
            if pruned_count > 0 {
                self.rebuild_dashboard_runtime_records("runtime_prune");
            }
            return outcome;
        }
        guard.projection_tombstones.remove(&key);
        guard.records.insert(
            key.clone(),
            RuntimeInvocationEntry {
                record,
                updated_at: now,
            },
        );
        let pruned_count =
            pruned_count + prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        let outcome = RuntimeInvocationStoreUpsertOutcome {
            running_count: guard.records.len(),
            pruned_count,
            skipped_terminal: false,
        };
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        } else {
            self.sync_dashboard_runtime_key(&key, "runtime_upsert");
        }
        outcome
    }

    pub(crate) fn upsert_terminal(
        &self,
        record: ApiInvocation,
    ) -> RuntimeInvocationStoreRemoveOutcome {
        let Ok(mut guard) = self.inner.lock() else {
            return RuntimeInvocationStoreRemoveOutcome {
                removed: false,
                already_terminal: false,
            };
        };
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(record.invoke_id.clone(), record.occurred_at.clone());
        let already_terminal = guard.terminal_tombstones.contains_key(&key);
        if already_terminal {
            let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
            drop(guard);
            if pruned_count > 0 {
                self.rebuild_dashboard_runtime_records("runtime_prune");
            }
            return RuntimeInvocationStoreRemoveOutcome {
                removed: false,
                already_terminal: true,
            };
        }
        let removed = guard
            .records
            .insert(
                key.clone(),
                RuntimeInvocationEntry {
                    record,
                    updated_at: now,
                },
            )
            .is_some();
        guard.terminal_tombstones.insert(key.clone(), now);
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        let outcome = RuntimeInvocationStoreRemoveOutcome {
            removed,
            already_terminal: false,
        };
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        } else {
            self.update_dashboard_runtime_record(key, None, "terminal_delta");
        }
        self.mark_dashboard_terminal_dirty();
        outcome
    }

    pub(crate) fn clear_terminal_tombstone(&self, invoke_id: &str, occurred_at: &str) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        let removed = guard
            .terminal_tombstones
            .remove(&RuntimeInvocationKey::new(invoke_id, occurred_at))
            .is_some();
        drop(guard);
        if removed {
            self.sync_dashboard_runtime_key(
                &RuntimeInvocationKey::new(invoke_id, occurred_at),
                "terminal_rollback",
            );
            self.mark_dashboard_terminal_dirty();
        }
        removed
    }

    pub(crate) fn contains_terminal(&self, invoke_id: &str, occurred_at: &str) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let contains_terminal = guard.terminal_tombstones.contains_key(&key)
            || guard
                .records
                .get(&key)
                .is_some_and(|entry| runtime_store_record_is_terminal(&entry.record));
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        contains_terminal
    }

    pub(crate) fn remove_non_terminal(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> Option<ApiInvocation> {
        let Ok(mut guard) = self.inner.lock() else {
            return None;
        };
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let should_remove = guard
            .records
            .get(&key)
            .is_some_and(|entry| !runtime_store_record_is_terminal(&entry.record));
        let removed = if should_remove {
            guard.records.remove(&key).map(|entry| entry.record)
        } else {
            None
        };
        if removed.is_some() {
            guard
                .projection_tombstones
                .insert(key.clone(), Instant::now());
        }
        let pruned_count = if removed.is_some() {
            prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now())
        } else {
            0
        };
        drop(guard);
        if removed.is_some() {
            self.update_dashboard_runtime_record(key, None, "runtime_remove");
        }
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        removed
    }

    pub(crate) fn remove_non_terminal_by_invoke_id(&self, invoke_id: &str) -> usize {
        let Ok(mut guard) = self.inner.lock() else {
            return 0;
        };
        let keys = guard
            .records
            .iter()
            .filter(|(key, entry)| {
                key.invoke_id == invoke_id && !runtime_store_record_is_terminal(&entry.record)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let removed_count = keys
            .iter()
            .filter(|key| guard.records.remove(key).is_some())
            .count();
        if removed_count > 0 {
            let now = Instant::now();
            for key in &keys {
                guard.projection_tombstones.insert(key.clone(), now);
            }
        }
        let pruned_count = if removed_count > 0 {
            prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now())
        } else {
            0
        };
        drop(guard);
        if removed_count > 0 {
            for key in keys {
                self.update_dashboard_runtime_record(key, None, "runtime_remove");
            }
        }
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        removed_count
    }

    pub(crate) fn remove_persisted_terminal_overlay(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        let Ok(mut guard) = self.inner.lock() else {
            return false;
        };
        let now = Instant::now();
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let removed = guard.records.remove(&key).is_some();
        guard.terminal_tombstones.insert(key.clone(), now);
        let pruned_count = prune_bounded_runtime_invocation_store_locked(&mut guard, now);
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        } else {
            self.update_dashboard_runtime_record(key, None, "terminal_persisted");
        }
        self.mark_dashboard_terminal_dirty();
        removed
    }

    pub(crate) fn snapshot(&self) -> Vec<ApiInvocation> {
        let Ok(mut guard) = self.inner.lock() else {
            return Vec::new();
        };
        let pruned_count =
            prune_bounded_runtime_invocation_store_locked(&mut guard, Instant::now());
        let snapshot = guard
            .records
            .values()
            .map(|entry| entry.record.clone())
            .collect();
        drop(guard);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        snapshot
    }

    #[cfg(test)]
    pub(crate) fn backdate_for_test(&self, invoke_id: &str, occurred_at: &str, age: Duration) {
        let Some(updated_at) = Instant::now().checked_sub(age) else {
            return;
        };
        if let Ok(mut guard) = self.inner.lock()
            && let Some(entry) = guard
                .records
                .get_mut(&RuntimeInvocationKey::new(invoke_id, occurred_at))
        {
            entry.updated_at = updated_at;
        }
    }

    pub(crate) fn shutdown_summary(&self) -> RuntimeInvocationStoreShutdownSummary {
        let Ok(guard) = self.inner.lock() else {
            return RuntimeInvocationStoreShutdownSummary {
                running_count: 0,
                oldest_age_ms: None,
            };
        };
        let now = Instant::now();
        RuntimeInvocationStoreShutdownSummary {
            running_count: guard.records.len(),
            oldest_age_ms: guard
                .records
                .values()
                .map(|entry| now.duration_since(entry.updated_at).as_millis() as u64)
                .max(),
        }
    }
}

impl ApiInvocation {
    pub(crate) fn estimated_memory_bytes(&self) -> usize {
        fn option_string_bytes(value: &Option<String>) -> usize {
            value.as_ref().map_or(0, String::capacity)
        }

        self.invoke_id.capacity()
            + self.occurred_at.capacity()
            + self.source.capacity()
            + self.detail_level.capacity()
            + option_string_bytes(&self.proxy_display_name)
            + option_string_bytes(&self.model)
            + option_string_bytes(&self.request_model)
            + option_string_bytes(&self.response_model)
            + option_string_bytes(&self.reasoning_effort)
            + option_string_bytes(&self.status)
            + option_string_bytes(&self.live_phase)
            + option_string_bytes(&self.error_message)
            + option_string_bytes(&self.failure_kind)
            + option_string_bytes(&self.blocked_binding_json)
            + option_string_bytes(&self.stream_terminal_event)
            + option_string_bytes(&self.upstream_error_code)
            + option_string_bytes(&self.upstream_error_message)
            + option_string_bytes(&self.downstream_error_message)
            + option_string_bytes(&self.upstream_request_id)
            + option_string_bytes(&self.failure_class)
            + option_string_bytes(&self.endpoint)
            + option_string_bytes(&self.compaction_request_kind)
            + option_string_bytes(&self.compaction_response_kind)
            + option_string_bytes(&self.image_intent)
            + option_string_bytes(&self.requester_ip)
            + option_string_bytes(&self.prompt_cache_key)
            + option_string_bytes(&self.sticky_key)
            + option_string_bytes(&self.route_mode)
            + option_string_bytes(&self.upstream_account_name)
            + option_string_bytes(&self.response_content_encoding)
            + option_string_bytes(&self.request_compression_algorithm)
            + option_string_bytes(&self.transport)
            + option_string_bytes(&self.pool_attempt_terminal_reason)
            + option_string_bytes(&self.requested_service_tier)
            + option_string_bytes(&self.service_tier)
            + option_string_bytes(&self.billing_service_tier)
            + option_string_bytes(&self.price_version)
            + option_string_bytes(&self.request_raw_path)
            + option_string_bytes(&self.request_raw_truncated_reason)
            + option_string_bytes(&self.response_raw_path)
            + option_string_bytes(&self.response_raw_truncated_reason)
            + option_string_bytes(&self.detail_pruned_at)
            + option_string_bytes(&self.detail_prune_reason)
            + self.created_at.capacity()
            + std::mem::size_of::<Self>()
    }
}

pub(crate) fn runtime_store_record_is_terminal(record: &ApiInvocation) -> bool {
    !matches!(
        record
            .status
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "running" | "pending"
    )
}

pub(crate) fn prune_bounded_runtime_invocation_store_locked(
    store: &mut ProxyRuntimeInvocationStoreInner,
    now: Instant,
) -> usize {
    let pruned_keys = prune_bounded_runtime_invocations_locked(
        &mut store.records,
        now,
        PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE,
        PROXY_RUNTIME_INVOCATION_STORE_MAX_RECORDS,
    );
    let pruned_count = pruned_keys.len();
    for key in pruned_keys {
        store.projection_tombstones.insert(key, now);
    }
    pruned_count
        + prune_bounded_runtime_tombstones_locked(
            &mut store.terminal_tombstones,
            now,
            PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE,
            PROXY_RUNTIME_INVOCATION_TERMINAL_TOMBSTONE_MAX_RECORDS,
        )
        + prune_bounded_runtime_tombstones_locked(
            &mut store.projection_tombstones,
            now,
            PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE,
            PROXY_RUNTIME_INVOCATION_TERMINAL_TOMBSTONE_MAX_RECORDS,
        )
}

pub(crate) fn prune_bounded_runtime_invocations_locked(
    records: &mut HashMap<RuntimeInvocationKey, RuntimeInvocationEntry>,
    now: Instant,
    max_age: Duration,
    max_records: usize,
) -> Vec<RuntimeInvocationKey> {
    let mut pruned_keys = Vec::new();
    records.retain(|key, entry| {
        let retain = now.duration_since(entry.updated_at) <= max_age;
        if !retain {
            pruned_keys.push(key.clone());
        }
        retain
    });
    if records.len() > max_records {
        let mut ranked_keys = records
            .iter()
            .map(|(key, entry)| (key.clone(), entry.updated_at))
            .collect::<Vec<_>>();
        ranked_keys.sort_by_key(|(_, updated_at)| *updated_at);
        let excess = records.len().saturating_sub(max_records);
        for (key, _) in ranked_keys.into_iter().take(excess) {
            records.remove(&key);
            pruned_keys.push(key);
        }
    }
    pruned_keys
}

pub(crate) fn prune_bounded_runtime_tombstones_locked(
    tombstones: &mut HashMap<RuntimeInvocationKey, Instant>,
    now: Instant,
    max_age: Duration,
    max_records: usize,
) -> usize {
    let before = tombstones.len();
    tombstones.retain(|_, terminal_at| now.duration_since(*terminal_at) <= max_age);
    if tombstones.len() > max_records {
        let mut ranked_keys = tombstones
            .iter()
            .map(|(key, terminal_at)| (key.clone(), *terminal_at))
            .collect::<Vec<_>>();
        ranked_keys.sort_by_key(|(_, terminal_at)| *terminal_at);
        let excess = tombstones.len().saturating_sub(max_records);
        for (key, _) in ranked_keys.into_iter().take(excess) {
            tombstones.remove(&key);
        }
    }
    before.saturating_sub(tombstones.len())
}

#[derive(Debug)]
pub(crate) struct AppState {
    pub(crate) config: AppConfig,
    pub(crate) pool: Pool<Sqlite>,
    pub(crate) process_started_at_utc: DateTime<Utc>,
    pub(crate) sqlite_batch_writer: Arc<SqliteBatchWriter>,
    pub(crate) pool_account_selection_runtime: Arc<PoolAccountSelectionRuntime>,
    pub(crate) proxy_runtime_invocations: Arc<RuntimeProjectionHub>,
    pub(crate) dashboard_network_speed_cache: Arc<DashboardNetworkSpeedCache>,
    pub(crate) oauth_installation_seed: [u8; 32],
    pub(crate) hourly_rollup_sync_lock: Arc<Mutex<()>>,
    pub(crate) http_clients: HttpClients,
    pub(crate) broadcaster: broadcast::Sender<BroadcastPayload>,
    pub(crate) broadcast_state_cache: Arc<Mutex<BroadcastStateCache>>,
    pub(crate) subscription_hub: Arc<SubscriptionHub>,
    pub(crate) proxy_summary_quota_broadcast_seq: Arc<AtomicU64>,
    pub(crate) proxy_summary_quota_broadcast_running: Arc<AtomicBool>,
    pub(crate) proxy_summary_quota_broadcast_handle: Arc<Mutex<Vec<JoinHandle<()>>>>,
    pub(crate) dashboard_activity_live_broadcast_seq: Arc<AtomicU64>,
    pub(crate) dashboard_activity_live_broadcast_running: Arc<AtomicBool>,
    pub(crate) startup_ready: Arc<AtomicBool>,
    pub(crate) shutdown: CancellationToken,
    pub(crate) semaphore: Arc<Semaphore>,
    pub(crate) proxy_request_in_flight: Arc<AtomicUsize>,
    pub(crate) proxy_raw_async_semaphore: Arc<Semaphore>,
    pub(crate) proxy_model_settings: Arc<RwLock<ProxyModelSettings>>,
    pub(crate) proxy_model_settings_update_lock: Arc<Mutex<()>>,
    pub(crate) forward_proxy: Arc<Mutex<ForwardProxyManager>>,
    pub(crate) xray_supervisor: Arc<Mutex<XraySupervisor>>,
    pub(crate) forward_proxy_settings_update_lock: Arc<Mutex<()>>,
    pub(crate) forward_proxy_subscription_refresh_lock: Arc<Mutex<()>>,
    pub(crate) pricing_settings_update_lock: Arc<Mutex<()>>,
    pub(crate) pricing_catalog: Arc<RwLock<PricingCatalog>>,
    pub(crate) prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    pub(crate) dashboard_activity_snapshot_cache: Arc<Mutex<DashboardActivitySnapshotCacheState>>,
    pub(crate) terminal_projection_hub: Arc<TerminalProjectionHub>,
    pub(crate) long_term_projection_runtime: Arc<Mutex<LongTermProjectionRuntime>>,
    pub(crate) memory_diagnostics: Arc<MemoryDiagnosticsRuntime>,
    pub(crate) maintenance_stats_cache: Arc<Mutex<StatsMaintenanceCacheState>>,
    pub(crate) system_status_cache: Arc<Mutex<SystemStatusCacheState>>,
    pub(crate) pool_routing_reservations:
        Arc<std::sync::Mutex<HashMap<String, PoolRoutingReservation>>>,
    pub(crate) pool_routing_runtime_cache: Arc<Mutex<Option<PoolRoutingRuntimeCache>>>,
    pub(crate) pool_live_attempt_ids: Arc<std::sync::Mutex<HashSet<i64>>>,
    pub(crate) pool_group_429_retry_delay_override: Option<Duration>,
    pub(crate) pool_no_available_wait: PoolNoAvailableWaitSettings,
    pub(crate) upstream_accounts: Arc<UpstreamAccountsRuntime>,
}

#[derive(Debug, Clone)]
pub(crate) struct PricingCatalog {
    pub(crate) version: String,
    pub(crate) models: HashMap<String, ModelPricing>,
}

impl Default for PricingCatalog {
    fn default() -> Self {
        Self {
            version: "unavailable".to_string(),
            models: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ModelPricing {
    pub(crate) input_per_1m: f64,
    pub(crate) output_per_1m: f64,
    #[serde(default)]
    pub(crate) cache_input_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) cache_read_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) cache_write_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) reasoning_per_1m: Option<f64>,
    #[serde(default = "default_pricing_source_custom")]
    pub(crate) source: String,
}

impl ModelPricing {
    pub(crate) fn effective_cache_read_per_1m(&self) -> Option<f64> {
        self.cache_read_per_1m.or(self.cache_input_per_1m)
    }

    pub(crate) fn has_explicit_cache_pricing_split(&self) -> bool {
        self.cache_read_per_1m.is_some() && self.cache_write_per_1m.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PricingEntry {
    pub(crate) model: String,
    pub(crate) input_per_1m: f64,
    pub(crate) output_per_1m: f64,
    #[serde(default)]
    pub(crate) cache_input_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) cache_read_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) cache_write_per_1m: Option<f64>,
    #[serde(default)]
    pub(crate) reasoning_per_1m: Option<f64>,
    #[serde(default = "default_pricing_source_custom")]
    pub(crate) source: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PricingSettingsUpdateRequest {
    pub(crate) catalog_version: String,
    #[serde(default)]
    pub(crate) entries: Vec<PricingEntry>,
}

impl PricingSettingsUpdateRequest {
    pub(crate) fn normalized(self) -> Result<PricingCatalog, (StatusCode, String)> {
        let version = normalize_pricing_catalog_version(self.catalog_version).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "catalogVersion must be a non-empty string".to_string(),
            )
        })?;
        let mut models = HashMap::new();
        for entry in self.entries {
            let model_id = entry.model.trim();
            if model_id.is_empty() || model_id.len() > 128 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid model id: {}", entry.model),
                ));
            }
            if !entry.input_per_1m.is_finite()
                || !entry.output_per_1m.is_finite()
                || entry.input_per_1m < 0.0
                || entry.output_per_1m < 0.0
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid pricing values for model: {model_id}"),
                ));
            }
            if let Some(cache) = entry.cache_input_per_1m
                && (!cache.is_finite() || cache < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid cacheInputPer1m for model: {model_id}"),
                ));
            }
            if let Some(cache) = entry.cache_read_per_1m
                && (!cache.is_finite() || cache < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid cacheReadPer1m for model: {model_id}"),
                ));
            }
            if let Some(cache) = entry.cache_write_per_1m
                && (!cache.is_finite() || cache < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid cacheWritePer1m for model: {model_id}"),
                ));
            }
            if let Some(reasoning) = entry.reasoning_per_1m
                && (!reasoning.is_finite() || reasoning < 0.0)
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("invalid reasoningPer1m for model: {model_id}"),
                ));
            }

            let cache_read_per_1m = entry.cache_read_per_1m.or(entry.cache_input_per_1m);
            let inserted = models.insert(
                model_id.to_string(),
                ModelPricing {
                    input_per_1m: entry.input_per_1m,
                    output_per_1m: entry.output_per_1m,
                    cache_input_per_1m: cache_read_per_1m,
                    cache_read_per_1m,
                    cache_write_per_1m: entry.cache_write_per_1m,
                    reasoning_per_1m: entry.reasoning_per_1m,
                    source: normalize_pricing_source(entry.source),
                },
            );
            if inserted.is_some() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("duplicate model id: {model_id}"),
                ));
            }
        }
        Ok(PricingCatalog { version, models })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PricingSettingsResponse {
    pub(crate) catalog_version: String,
    pub(crate) entries: Vec<PricingEntry>,
}

impl PricingSettingsResponse {
    pub(crate) fn from_catalog(catalog: &PricingCatalog) -> Self {
        let mut entries = catalog
            .models
            .iter()
            .map(|(model, pricing)| {
                let cache_read_per_1m = pricing.effective_cache_read_per_1m();
                PricingEntry {
                    model: model.clone(),
                    input_per_1m: pricing.input_per_1m,
                    output_per_1m: pricing.output_per_1m,
                    cache_input_per_1m: cache_read_per_1m,
                    cache_read_per_1m,
                    cache_write_per_1m: pricing.cache_write_per_1m,
                    reasoning_per_1m: pricing.reasoning_per_1m,
                    source: pricing.source.clone(),
                }
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.model.cmp(&b.model));
        Self {
            catalog_version: catalog.version.clone(),
            entries,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyModelSettings {
    pub(crate) hijack_enabled: bool,
    pub(crate) merge_upstream_enabled: bool,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) websocket_enabled: bool,
    pub(crate) upstream_websocket_default_enabled: bool,
    pub(crate) request_body_logging_enabled: bool,
    pub(crate) response_body_logging_enabled: bool,
    pub(crate) encrypted_session_owner_routing_enabled: bool,
    pub(crate) enabled_preset_models: Vec<String>,
}

pub(crate) fn normalize_proxy_upstream_429_max_retries(value: u8) -> u8 {
    value.min(MAX_PROXY_UPSTREAM_429_MAX_RETRIES)
}

pub(crate) fn decode_proxy_upstream_429_max_retries(raw: Option<i64>) -> u8 {
    raw.and_then(|value| u8::try_from(value).ok())
        .map(normalize_proxy_upstream_429_max_retries)
        .unwrap_or(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES)
}

impl Default for ProxyModelSettings {
    fn default() -> Self {
        Self {
            hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            merge_upstream_enabled: DEFAULT_PROXY_MODELS_MERGE_UPSTREAM_ENABLED,
            upstream_429_max_retries: DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES,
            websocket_enabled: DEFAULT_OPENAI_PROXY_WEBSOCKET_ENABLED,
            upstream_websocket_default_enabled:
                DEFAULT_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED,
            request_body_logging_enabled: true,
            response_body_logging_enabled: true,
            encrypted_session_owner_routing_enabled:
                DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
            enabled_preset_models: default_enabled_preset_models(),
        }
    }
}

impl ProxyModelSettings {
    pub(crate) fn normalized(self) -> Self {
        let merge_upstream_enabled = if self.hijack_enabled {
            self.merge_upstream_enabled
        } else {
            false
        };
        Self {
            hijack_enabled: self.hijack_enabled,
            merge_upstream_enabled,
            upstream_429_max_retries: normalize_proxy_upstream_429_max_retries(
                self.upstream_429_max_retries,
            ),
            websocket_enabled: self.websocket_enabled,
            upstream_websocket_default_enabled: self.upstream_websocket_default_enabled,
            request_body_logging_enabled: self.request_body_logging_enabled,
            response_body_logging_enabled: self.response_body_logging_enabled,
            encrypted_session_owner_routing_enabled: self.encrypted_session_owner_routing_enabled,
            enabled_preset_models: normalize_enabled_preset_models(self.enabled_preset_models),
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ProxyModelSettingsRow {
    pub(crate) hijack_enabled: i64,
    pub(crate) merge_upstream_enabled: i64,
    pub(crate) upstream_429_max_retries: Option<i64>,
    pub(crate) openai_proxy_websocket_enabled: Option<i64>,
    pub(crate) openai_proxy_upstream_websocket_default_enabled: Option<i64>,
    pub(crate) request_body_logging_enabled: Option<i64>,
    pub(crate) response_body_logging_enabled: Option<i64>,
    pub(crate) encrypted_session_owner_routing_enabled: Option<i64>,
    pub(crate) enabled_preset_models_json: Option<String>,
}

impl From<ProxyModelSettingsRow> for ProxyModelSettings {
    fn from(value: ProxyModelSettingsRow) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled != 0,
            merge_upstream_enabled: value.merge_upstream_enabled != 0,
            upstream_429_max_retries: decode_proxy_upstream_429_max_retries(
                value.upstream_429_max_retries,
            ),
            websocket_enabled: value.openai_proxy_websocket_enabled.unwrap_or(0) != 0,
            upstream_websocket_default_enabled: value
                .openai_proxy_upstream_websocket_default_enabled
                .unwrap_or(0)
                != 0,
            request_body_logging_enabled: value.request_body_logging_enabled.unwrap_or(1) != 0,
            response_body_logging_enabled: value.response_body_logging_enabled.unwrap_or(1) != 0,
            encrypted_session_owner_routing_enabled: value
                .encrypted_session_owner_routing_enabled
                .unwrap_or(DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED as i64)
                != 0,
            enabled_preset_models: decode_enabled_preset_models(
                value.enabled_preset_models_json.as_deref(),
            ),
        }
        .normalized()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyModelSettingsUpdateRequest {
    pub(crate) hijack_enabled: bool,
    pub(crate) merge_upstream_enabled: bool,
    #[serde(default)]
    pub(crate) fast_mode_rewrite_mode: Option<String>,
    #[serde(default)]
    pub(crate) upstream_429_max_retries: Option<u8>,
    #[serde(default)]
    pub(crate) websocket_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) upstream_websocket_default_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) request_body_logging_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) response_body_logging_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) encrypted_session_owner_routing_enabled: Option<bool>,
    #[serde(default = "default_enabled_preset_models")]
    pub(crate) enabled_models: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyModelSettingsResponse {
    pub(crate) hijack_enabled: bool,
    pub(crate) merge_upstream_enabled: bool,
    pub(crate) fast_mode_rewrite_mode: String,
    pub(crate) upstream_429_max_retries: u8,
    pub(crate) websocket_enabled: bool,
    pub(crate) upstream_websocket_default_enabled: bool,
    pub(crate) request_body_logging_enabled: bool,
    pub(crate) response_body_logging_enabled: bool,
    pub(crate) encrypted_session_owner_routing_enabled: bool,
    pub(crate) default_hijack_enabled: bool,
    pub(crate) models: Vec<String>,
    pub(crate) enabled_models: Vec<String>,
}

impl ProxyModelSettingsResponse {
    pub(crate) fn from_settings(value: ProxyModelSettings) -> Self {
        Self {
            hijack_enabled: value.hijack_enabled,
            merge_upstream_enabled: value.merge_upstream_enabled,
            fast_mode_rewrite_mode: "disabled".to_string(),
            upstream_429_max_retries: value.upstream_429_max_retries,
            websocket_enabled: value.websocket_enabled,
            upstream_websocket_default_enabled: value.upstream_websocket_default_enabled,
            request_body_logging_enabled: value.request_body_logging_enabled,
            response_body_logging_enabled: value.response_body_logging_enabled,
            encrypted_session_owner_routing_enabled: value.encrypted_session_owner_routing_enabled,
            default_hijack_enabled: DEFAULT_PROXY_MODELS_HIJACK_ENABLED,
            models: PROXY_PRESET_MODEL_IDS
                .iter()
                .map(|model| (*model).to_string())
                .collect(),
            enabled_models: value.enabled_preset_models,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsResponse {
    pub(crate) proxy: ProxyModelSettingsResponse,
    pub(crate) forward_proxy: ForwardProxySettingsResponse,
    pub(crate) pricing: PricingSettingsResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemTaskKind {
    RetentionArchive,
    StartupBackfill,
    ForwardProxySubscriptionRefresh,
}

impl SystemTaskKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RetentionArchive => "retention_archive",
            Self::StartupBackfill => "startup_backfill",
            Self::ForwardProxySubscriptionRefresh => "forward_proxy_subscription_refresh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SystemTaskStatus {
    Running,
    Success,
    Failed,
    Skipped,
}

impl SystemTaskStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SystemStatusCacheEntry {
    pub(crate) cached_at: Instant,
    pub(crate) response: SystemStatusResponse,
}

#[derive(Debug, Default)]
pub(crate) struct SystemStatusCacheState {
    pub(crate) latest: Option<SystemStatusCacheEntry>,
    pub(crate) in_flight: Option<watch::Sender<bool>>,
    pub(crate) waiter_count: usize,
    pub(crate) raw_metrics_health_override: Option<String>,
}

pub(crate) fn default_enabled_preset_models() -> Vec<String> {
    PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn normalize_enabled_preset_models(enabled_models: Vec<String>) -> Vec<String> {
    let enabled_set: HashSet<&str> = enabled_models.iter().map(String::as_str).collect();
    PROXY_PRESET_MODEL_IDS
        .iter()
        .filter(|model| enabled_set.contains(**model))
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn decode_enabled_preset_models(raw: Option<&str>) -> Vec<String> {
    match raw {
        Some(serialized) => serde_json::from_str::<Vec<String>>(serialized)
            .map(normalize_enabled_preset_models)
            .unwrap_or_else(|_| default_enabled_preset_models()),
        None => default_enabled_preset_models(),
    }
}

pub(crate) fn default_pricing_source_custom() -> String {
    "custom".to_string()
}

pub(crate) fn normalize_pricing_catalog_version(raw: String) -> Option<String> {
    let normalized = raw.trim().to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(crate) fn normalize_pricing_source(raw: String) -> String {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        default_pricing_source_custom()
    } else {
        normalized
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HttpClients {
    pub(crate) shared: Client,
    pub(crate) pool_upstream: Client,
    pub(crate) proxy: Client,
    pub(crate) timeout: Duration,
    pub(crate) user_agent: String,
}

impl HttpClients {
    pub(crate) fn build(config: &AppConfig) -> Result<Self> {
        let timeout = config.request_timeout;
        let user_agent = config.user_agent.clone();

        let shared = Self::builder(Some(timeout), &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .build()
            .context("failed to construct shared HTTP client")?;

        // Pool live upstream traffic can legitimately stream well past REQUEST_TIMEOUT_SECS.
        // Handshake and upload budgets are enforced by route-specific timeout wrappers instead.
        let pool_upstream = Self::builder(None, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .build()
            .context("failed to construct pool upstream HTTP client")?;

        let proxy = Self::builder(None, &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .connect_timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to construct proxy HTTP client")?;

        Ok(Self {
            shared,
            pool_upstream,
            proxy,
            timeout,
            user_agent,
        })
    }

    pub(crate) fn client_for_parallelism(&self, force_new_connection: bool) -> Result<Client> {
        if force_new_connection {
            let client = Self::builder(Some(self.timeout), &self.user_agent)
                .pool_max_idle_per_host(0)
                .build()
                .context("failed to construct dedicated HTTP client")?;
            Ok(client)
        } else {
            Ok(self.shared.clone())
        }
    }

    pub(crate) fn client_for_pool_upstream(&self) -> Client {
        self.pool_upstream.clone()
    }

    pub(crate) fn client_for_forward_proxy(&self, endpoint_url: Option<&Url>) -> Result<Client> {
        let Some(endpoint_url) = endpoint_url else {
            return Ok(self.proxy.clone());
        };

        Self::builder(None, &self.user_agent)
            .pool_max_idle_per_host(2)
            .connect_timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .proxy(
                Proxy::all(endpoint_url.as_str())
                    .with_context(|| format!("invalid forward proxy endpoint: {endpoint_url}"))?,
            )
            .build()
            .context("failed to construct forward proxy HTTP client")
    }

    pub(crate) fn builder(timeout: Option<Duration>, user_agent: &str) -> ClientBuilder {
        let builder = Client::builder()
            .user_agent(user_agent)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(90))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .http2_keep_alive_timeout(Duration::from_secs(30))
            .http2_keep_alive_while_idle(true);

        if let Some(timeout) = timeout {
            builder.timeout(timeout)
        } else {
            builder
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_projection_slices_keep_independent_non_extending_deadlines() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        let started = Instant::now();

        hub.mark_dashboard_dirty_at("test", started);
        hub.mark_dashboard_network_dirty_at(started);
        hub.mark_dashboard_terminal_dirty_at(started);
        hub.mark_dashboard_dirty_at("test", started + Duration::from_millis(100));
        hub.mark_dashboard_network_dirty_at(started + Duration::from_millis(500));
        hub.mark_dashboard_terminal_dirty_at(started + Duration::from_secs(2));

        let current = hub
            .pending_dashboard_publish_window()
            .expect("current slice deadline");
        assert_eq!(current.slice, DashboardProjectionSlice::Current);
        assert_eq!(
            current.deadline,
            started + DASHBOARD_RUNTIME_PROJECTION_COALESCE
        );
        hub.complete_dashboard_publish_window(
            hub.begin_dashboard_publish_window(current)
                .expect("consume current window"),
        );

        let network = hub
            .pending_dashboard_publish_window()
            .expect("network slice deadline");
        assert_eq!(network.slice, DashboardProjectionSlice::Network);
        assert_eq!(
            network.deadline,
            started + DASHBOARD_RUNTIME_NETWORK_PROJECTION_COALESCE
        );
        hub.complete_dashboard_publish_window(
            hub.begin_dashboard_publish_window(network)
                .expect("consume network window"),
        );

        let terminal = hub
            .pending_dashboard_publish_window()
            .expect("terminal slice deadline");
        assert_eq!(terminal.slice, DashboardProjectionSlice::Terminal);
        assert_eq!(
            terminal.deadline,
            started + DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE
        );
        hub.complete_dashboard_publish_window(
            hub.begin_dashboard_publish_window(terminal)
                .expect("consume terminal window"),
        );

        assert!(hub.capture_terminal_slice().is_none());
        assert!(hub.capture_terminal_slice().is_none());
        let counters = hub.dashboard_topology_counters();
        assert_eq!(counters.terminal.build_count, 2);
        assert_eq!(counters.terminal.revision_count, 0);
    }

    #[test]
    fn current_slice_comparison_ignores_network_only_changes() {
        let account = DashboardActivityLiveAccount {
            account_key: "upstream:7".to_string(),
            upstream_account_id: Some(7),
            upstream_account_name: Some("account-7".to_string()),
            in_progress_invocation_count: 1,
            in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
            retry_invocation_count: 0,
            in_progress_wait_sum_ms: 0.0,
            in_progress_wait_sample_count: 0,
            upload_bytes_per_second: 1.0,
            download_bytes_per_second: 2.0,
            network_live_bucket: None,
        };
        let mut current = empty_dashboard_live_core();
        current.accounts.push(account.clone());
        let mut network_only = current.clone();
        network_only.accounts[0].upload_bytes_per_second = 99.0;
        network_only.accounts[0].download_bytes_per_second = 101.0;

        assert!(dashboard_current_snapshot_content_eq(
            &current,
            &network_only
        ));

        network_only.accounts[0].in_progress_invocation_count = 2;
        assert!(!dashboard_current_snapshot_content_eq(
            &current,
            &network_only
        ));
    }

    #[test]
    fn current_and_network_slices_use_independent_revisions() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        let network_cache = Arc::new(DashboardNetworkSpeedCache::new(Utc::now()));
        hub.bind_dashboard_network_speed_cache(network_cache.clone())
            .expect("bind network cache");
        {
            let mut dashboard = hub.dashboard.lock().expect("dashboard state");
            let mut core = empty_dashboard_live_core();
            core.in_progress_invocation_count = 1;
            dashboard.live_core = Some(core);
        }

        let current = hub.capture_memory_snapshot().expect("current slice");
        network_cache.record_request_bytes(
            "independent-revision",
            "2026-08-06 10:00:00",
            None,
            Some("api.openai.com"),
            128,
            Utc::now(),
        );
        let network = hub.capture_network_slice().expect("network slice");

        assert_eq!(current.snapshot.revision, 1);
        assert_eq!(network.slice.revision, 1);
    }
}
