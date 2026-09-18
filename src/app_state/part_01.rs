#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingRuntimeCache {
    /// Monotonically changes whenever a routing write publishes a new snapshot.
    pub(crate) generation: u64,
    /// A routing-state write can invalidate the immutable snapshot without
    /// performing a database rebuild on the request that observed the write.
    /// The next reader rebuilds it through the shared cold-load lock.
    pub(crate) invalidated: bool,
    #[cfg(test)]
    pub(crate) sqlite_data_version: i64,
    pub(crate) api_key: Option<String>,
    pub(crate) request_compression: PoolRoutingRequestCompressionSettingsResolved,
    pub(crate) timeouts: PoolRoutingTimeoutSettingsResolved,
    pub(crate) cache_hit_protection: CacheHitProtectionSettings,
    pub(crate) model_routing: PoolModelRoutingRuntimeCache,
    pub(crate) prompt_route_cache: Arc<std::sync::Mutex<PoolRoutingPromptRouteCache>>,
    pub(crate) sticky_route_cache: Arc<std::sync::Mutex<PoolRoutingStickyRouteCache>>,
}

pub(crate) const POOL_ROUTING_PROMPT_ROUTE_CACHE_CAPACITY: usize = 16_384;
pub(crate) const POOL_ROUTING_STICKY_ROUTE_CACHE_CAPACITY: usize = 16_384;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct PoolRoutingPromptRouteCacheKey {
    pub(crate) prompt_cache_key: String,
    pub(crate) request_contains_encrypted_content: bool,
    pub(crate) encrypted_session_owner_routing_enabled: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingPromptRouteCacheValue {
    pub(crate) binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    pub(crate) owner_auto_guard_active: bool,
    pub(crate) conversation_override: Option<ConversationRoutingOverride>,
}

/// Bounded, negative-caching lookup table for high-cardinality conversation
/// keys. The owning runtime snapshot replaces this cache on routing writes.
#[derive(Debug, Default)]
pub(crate) struct PoolRoutingPromptRouteCache {
    generation: u64,
    values: HashMap<PoolRoutingPromptRouteCacheKey, Option<PoolRoutingPromptRouteCacheValue>>,
    recency: std::collections::VecDeque<PoolRoutingPromptRouteCacheKey>,
}

impl PoolRoutingPromptRouteCache {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn get(
        &mut self,
        key: &PoolRoutingPromptRouteCacheKey,
    ) -> Option<Option<PoolRoutingPromptRouteCacheValue>> {
        let value = self.values.get(key)?.clone();
        self.touch(key);
        Some(value)
    }

    pub(crate) fn insert(
        &mut self,
        key: PoolRoutingPromptRouteCacheKey,
        value: Option<PoolRoutingPromptRouteCacheValue>,
    ) {
        self.values.insert(key.clone(), value);
        self.touch(&key);
        while self.values.len() > POOL_ROUTING_PROMPT_ROUTE_CACHE_CAPACITY {
            let Some(oldest) = self.recency.pop_front() else {
                break;
            };
            self.values.remove(&oldest);
        }
    }

    pub(crate) fn invalidate_prompt_cache_key(&mut self, prompt_cache_key: &str) {
        self.generation = self.generation.saturating_add(1);
        self.values
            .retain(|key, _| key.prompt_cache_key != prompt_cache_key);
        self.recency
            .retain(|key| key.prompt_cache_key != prompt_cache_key);
    }

    fn touch(&mut self, key: &PoolRoutingPromptRouteCacheKey) {
        self.recency.retain(|existing| existing != key);
        self.recency.push_back(key.clone());
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct PoolRoutingStickyRouteCacheKey {
    pub(crate) sticky_key: String,
    pub(crate) model_key: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingStickyRouteCacheValue {
    pub(crate) route: Option<PoolStickyRouteRow>,
    pub(crate) affinity_generation: i64,
}

/// Bounded, negative-caching lookup table for sticky routing keys. Entries are
/// invalidated by the routing mutation broadcasts after the durable write.
#[derive(Debug, Default)]
pub(crate) struct PoolRoutingStickyRouteCache {
    generation: u64,
    values: HashMap<PoolRoutingStickyRouteCacheKey, PoolRoutingStickyRouteCacheValue>,
    recency: std::collections::VecDeque<PoolRoutingStickyRouteCacheKey>,
}

impl PoolRoutingStickyRouteCache {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn get(
        &mut self,
        key: &PoolRoutingStickyRouteCacheKey,
    ) -> Option<PoolRoutingStickyRouteCacheValue> {
        let value = self.values.get(key)?.clone();
        self.touch(key);
        Some(value)
    }

    pub(crate) fn insert(
        &mut self,
        key: PoolRoutingStickyRouteCacheKey,
        value: PoolRoutingStickyRouteCacheValue,
    ) {
        self.values.insert(key.clone(), value);
        self.touch(&key);
        while self.values.len() > POOL_ROUTING_STICKY_ROUTE_CACHE_CAPACITY {
            let Some(oldest) = self.recency.pop_front() else {
                break;
            };
            self.values.remove(&oldest);
        }
    }

    pub(crate) fn invalidate_sticky_key(&mut self, sticky_key: &str) {
        self.generation = self.generation.saturating_add(1);
        self.values.retain(|key, _| key.sticky_key != sticky_key);
        self.recency.retain(|key| key.sticky_key != sticky_key);
    }

    fn touch(&mut self, key: &PoolRoutingStickyRouteCacheKey) {
        self.recency.retain(|existing| existing != key);
        self.recency.push_back(key.clone());
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PoolModelRoutingRuntimeCache {
    pub(crate) generation: u64,
    pub(crate) mappings_by_account: HashMap<i64, Vec<CompiledModelMapping>>,
    pub(crate) routing_account_rows_by_id: HashMap<i64, std::sync::Arc<UpstreamAccountRow>>,
    pub(crate) routing_candidates: Vec<AccountRoutingCandidateRow>,
    pub(crate) effective_rules_by_account: HashMap<i64, EffectiveRoutingRule>,
    pub(crate) group_metadata_by_name: HashMap<String, UpstreamAccountGroupMetadata>,
    pub(crate) route_binding_failure_penalties: HashMap<String, i64>,
    pub(crate) transport_decode_sticky_escape_states:
        HashMap<i64, TransportDecodeStickyEscapeState>,
    pub(crate) model_route_runtime: HashMap<(i64, String), ModelRouteRuntimeSnapshot>,
    pub(crate) available_models: Vec<String>,
    pub(crate) warmed_model_account_ids: HashMap<String, Vec<i64>>,
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

/// Typed data needed by the active Prompt Cache projection. This deliberately excludes the
/// complete runtime record and its raw/detail fields.
#[derive(Debug, Clone)]
pub(crate) struct PromptCacheRuntimeProjection {
    pub(crate) row_id: i64,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) preview: PromptCacheConversationInvocationPreviewResponse,
}

impl PromptCacheRuntimeProjection {
    pub(crate) fn from_record(record: &ApiInvocation) -> Option<Self> {
        let prompt_cache_key =
            normalize_trimmed_optional_string_local(record.prompt_cache_key.clone());
        let sticky_key = normalize_trimmed_optional_string_local(record.sticky_key.clone());
        let preview_key = prompt_cache_key.clone().or_else(|| sticky_key.clone())?;
        Some(Self {
            row_id: record.id,
            prompt_cache_key,
            sticky_key,
            preview: prompt_cache_invocation_preview_from_runtime_record(record, preview_key),
        })
    }
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
    pub(crate) fn reject_removed_legacy_env() -> Result<()> {
        for env_name in [
            "DASHBOARD_RUNTIME_PROJECTION_MODE",
            "PROMPT_CACHE_TOPIC_PROJECTION_MODE",
        ] {
            if std::env::var(env_name)
                .ok()
                .is_some_and(|value| value.trim().eq_ignore_ascii_case("legacy"))
            {
                bail!(
                    "{env_name}=legacy is unsupported; the typed runtime mutation bus is mandatory"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn parse(value: Option<&str>) -> Result<Self> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None | Some("auto") => Ok(Self::Auto),
            Some(value) => bail!(
                "runtime projection mode `{value}` is unsupported; the typed runtime mutation bus is mandatory"
            ),
        }
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
    let is_retry = record.pool_attempt_count.unwrap_or_default() > 1
        || previous.is_some_and(|record| record.is_retry);
    Some(DashboardRuntimeBaselineRecord {
        key,
        upstream_account_id: record
            .upstream_account_id
            .or_else(|| previous.and_then(|record| record.upstream_account_id)),
        upstream_account_name: normalize_trimmed_optional_string_local(
            record.upstream_account_name.clone(),
        )
        .or_else(|| previous.and_then(|record| record.upstream_account_name.clone())),
        is_retry,
        live_phase: runtime_record_live_phase_with_retry(record, is_retry).map(str::to_string),
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardProjectionSliceCounterSnapshot {
    pub(crate) build_count: u64,
    pub(crate) revision_count: u64,
    pub(crate) cadence_miss_count: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
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
    pub(crate) slice_counters: DashboardRuntimeTopologyCounterSnapshot,
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
    pub(crate) parse_window_count: u64,
    pub(crate) parse_window_cpu_ms: u64,
    pub(crate) parse_window_wall_ms: u64,
    pub(crate) parse_window_bytes: u64,
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
    #[cfg(test)]
    full_record_clone_count: AtomicU64,
}

pub(crate) type ProxyRuntimeInvocationStore = RuntimeProjectionHub;

impl Default for RuntimeProjectionHub {
    fn default() -> Self {
        Self::new(RuntimeProjectionMode::Auto)
    }
}
