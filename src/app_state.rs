use super::*;

mod dashboard;
mod invocation_store;

use dashboard::DashboardRuntimeProjection;
pub(crate) use dashboard::{
    DASHBOARD_RUNTIME_NETWORK_PROJECTION_COALESCE, DASHBOARD_RUNTIME_PROJECTION_COALESCE,
    DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE, DashboardLiveProjection,
    DashboardNetworkProjectionCapture, DashboardProjectionCapture,
    DashboardProjectionPublishWindow, DashboardProjectionSlice, DashboardRuntimeBaselineRecord,
    DashboardRuntimeNetworkOpenBucketBaseline, DashboardRuntimeProjectionBaseline,
    DashboardRuntimeTopologyCounterSnapshot, DashboardTerminalProjectionCapture,
};
#[cfg(test)]
pub(crate) use dashboard::{
    DashboardProjectionSliceCounterSnapshot, dashboard_current_snapshot_content_eq,
    empty_dashboard_live_core,
};
#[cfg(test)]
pub(crate) use invocation_store::PROXY_RUNTIME_INVOCATION_STORE_MAX_AGE;
use invocation_store::RuntimeInvocationStore;
pub(crate) use invocation_store::{
    PromptCacheRuntimeProjection, RuntimeInvocationKey, RuntimeInvocationStoreRemoveOutcome,
    RuntimeInvocationStoreShutdownSummary, RuntimeInvocationStoreUpsertOutcome,
    runtime_store_record_is_terminal,
};

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
    invocation_store: RuntimeInvocationStore,
    mode: RuntimeProjectionMode,
    dashboard: DashboardRuntimeProjection,
    live_path_db_read_count: AtomicU64,
    build_count: AtomicU64,
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
            invocation_store: RuntimeInvocationStore::default(),
            mode,
            dashboard: DashboardRuntimeProjection::default(),
            live_path_db_read_count: AtomicU64::new(0),
            build_count: AtomicU64::new(0),
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
        self.dashboard.bind_network_speed_cache(cache)
    }

    pub(crate) fn dashboard_live_projection(&self) -> DashboardLiveProjection<'_> {
        self.dashboard.live_projection()
    }

    fn sync_dashboard_runtime_key(&self, key: &RuntimeInvocationKey, trigger: &'static str) {
        let Ok(runtime) = self.invocation_store.lock() else {
            return;
        };
        let record = runtime.records.get(key).map(|entry| &entry.record);
        self.dashboard
            .update_runtime_record(key.clone(), record, trigger);
    }

    fn rebuild_dashboard_runtime_records(&self, trigger: &'static str) {
        let Ok(runtime) = self.invocation_store.lock() else {
            return;
        };
        self.dashboard.rebuild_runtime_records(&runtime, trigger);
    }

    pub(crate) fn mark_dashboard_dirty(&self, trigger: &'static str) {
        self.dashboard.mark_dirty(trigger);
    }

    pub(crate) fn mark_dashboard_dirty_at(&self, trigger: &'static str, now: Instant) {
        self.dashboard.mark_dirty_at(trigger, now);
    }

    pub(crate) fn mark_dashboard_network_dirty(&self) {
        self.dashboard.mark_network_dirty();
    }

    fn mark_dashboard_network_dirty_at(&self, now: Instant) {
        self.dashboard.mark_network_dirty_at(now);
    }

    fn mark_dashboard_terminal_dirty(&self) {
        self.dashboard.mark_terminal_dirty();
    }

    fn mark_dashboard_terminal_dirty_at(&self, now: Instant) {
        self.dashboard.mark_terminal_dirty_at(now);
    }

    pub(crate) async fn wait_for_dashboard_publish_signal(&self) {
        self.dashboard.wait_for_publish_signal().await;
    }

    pub(crate) fn pending_dashboard_deadline(&self) -> Option<Instant> {
        self.dashboard.pending_deadline()
    }

    pub(crate) fn pending_dashboard_publish_window(
        &self,
    ) -> Option<DashboardProjectionPublishWindow> {
        self.dashboard.pending_publish_window()
    }

    pub(crate) fn has_pending_dashboard_terminal_publish(&self) -> bool {
        self.dashboard.has_pending_terminal_publish()
    }

    pub(crate) fn begin_dashboard_publish_window(
        &self,
        window: DashboardProjectionPublishWindow,
    ) -> Option<DashboardProjectionPublishWindow> {
        self.dashboard.begin_publish_window(window)
    }

    pub(crate) fn complete_dashboard_publish_window(
        &self,
        window: DashboardProjectionPublishWindow,
    ) {
        self.dashboard.complete_publish_window(window);
    }

    pub(crate) fn is_memory_ready(&self) -> bool {
        self.dashboard.is_memory_ready()
    }

    pub(crate) fn dashboard_generation(&self) -> u64 {
        self.dashboard.generation()
    }

    pub(crate) fn capture_memory_snapshot(&self) -> Result<DashboardProjectionCapture> {
        self.build_count.fetch_add(1, Ordering::Relaxed);
        self.dashboard.record_build();
        self.dashboard.capture_memory_snapshot()
    }

    pub(crate) fn capture_network_slice(&self) -> Result<DashboardNetworkProjectionCapture> {
        self.dashboard.capture_network_slice()
    }

    pub(crate) fn record_dashboard_terminal_delta(&self, delta: DashboardActivityTerminalDelta) {
        self.dashboard.record_terminal_delta(delta);
    }

    pub(crate) fn discard_dashboard_terminal_delta(&self, invoke_id: &str, occurred_at: &str) {
        self.dashboard
            .discard_terminal_delta(invoke_id, occurred_at);
    }

    pub(crate) fn capture_terminal_slice(&self) -> Option<DashboardTerminalProjectionCapture> {
        self.dashboard.capture_terminal_slice()
    }

    #[cfg(test)]
    pub(crate) fn pending_terminal_slice_count(&self) -> usize {
        self.dashboard.pending_terminal_slice_count()
    }

    pub(crate) fn legacy_live_snapshot(
        &self,
        current: DashboardActivityLiveSnapshot,
    ) -> DashboardActivityLiveSnapshot {
        self.dashboard.legacy_live_snapshot(current)
    }

    pub(crate) fn legacy_live_snapshot_for_network(
        &self,
        network: &DashboardNetworkProjectionSlice,
    ) -> Option<DashboardActivityLiveSnapshot> {
        self.dashboard.legacy_live_snapshot_for_network(network)
    }

    pub(crate) fn install_persistence_baseline_if_generation(
        &self,
        _snapshot: DashboardActivityLiveSnapshot,
        baseline: DashboardRuntimeProjectionBaseline,
        snapshot_origin: &'static str,
        expected_generation: u64,
    ) -> Result<Option<DashboardProjectionCapture>> {
        let runtime = self
            .invocation_store
            .lock()
            .map_err(|_| anyhow!("runtime invocation store lock is poisoned"))?;
        self.dashboard.install_persistence_baseline_if_generation(
            baseline,
            snapshot_origin,
            expected_generation,
            &runtime,
        )
    }

    pub(crate) fn last_good_capture(
        &self,
        snapshot_origin: &'static str,
    ) -> Option<DashboardProjectionCapture> {
        self.dashboard.last_good_capture(snapshot_origin)
    }

    pub(crate) fn mark_degraded(&self, reason: &'static str) {
        self.dashboard.mark_degraded(reason);
    }

    pub(crate) fn record_reconcile_failure(&self, reason: &'static str) {
        self.dashboard.record_reconcile_failure(reason);
    }

    pub(crate) fn record_reconcile_deferred(&self, reason: &'static str) {
        self.dashboard.record_reconcile_deferred(reason);
    }

    pub(crate) fn record_live_path_db_read(&self) {
        self.live_path_db_read_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_build(&self) {
        self.build_count.fetch_add(1, Ordering::Relaxed);
        self.dashboard.record_build();
    }

    pub(crate) fn record_current_slice_cadence_miss(&self) {
        self.dashboard.record_current_slice_cadence_miss();
    }

    pub(crate) fn record_network_slice_cadence_miss(&self) {
        self.dashboard.record_network_slice_cadence_miss();
    }

    pub(crate) fn record_terminal_slice_cadence_miss(&self) {
        self.dashboard.record_terminal_slice_cadence_miss();
    }

    #[cfg(test)]
    pub(crate) fn dashboard_topology_counters(&self) -> DashboardRuntimeTopologyCounterSnapshot {
        self.dashboard.topology_counters()
    }

    #[cfg(test)]
    pub(crate) fn reset_dashboard_topology_counters(&self) {
        self.dashboard.reset_topology_counters();
    }

    #[cfg(test)]
    pub(crate) fn set_live_core_for_test(&self, core: DashboardActivityLiveSnapshot) {
        self.dashboard.set_live_core_for_test(core);
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
        let parse_window = crate::proxy::request_semantic_cpu_attribution_snapshot();
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
            parse_window_count: parse_window.count,
            parse_window_cpu_ms: parse_window.cpu_ms,
            parse_window_wall_ms: parse_window.wall_ms,
            parse_window_bytes: parse_window.bytes,
        }
    }

    pub(crate) fn health_snapshot(
        &self,
        active_subscriber_count: usize,
    ) -> RuntimeProjectionHealthSnapshot {
        let projection = self.dashboard.health_snapshot();
        RuntimeProjectionHealthSnapshot {
            mode: self.mode.as_str().to_string(),
            state: projection.state.to_string(),
            producer_state: if self.producer_running.load(Ordering::Acquire) {
                "running".to_string()
            } else {
                "idle".to_string()
            },
            active_subscriber_count: active_subscriber_count as u64,
            live_path_db_read_count: self.live_path_db_read_count.load(Ordering::Relaxed),
            build_count: self.build_count.load(Ordering::Relaxed),
            revision: projection.revision,
            snapshot_origin: projection.snapshot_origin.to_string(),
            last_good_age_ms: projection.last_good_age_ms,
            degraded_reason: projection.degraded_reason,
            last_defer_reason: projection.last_defer_reason,
            slice_counters: projection.slice_counters,
        }
    }

    pub(crate) fn apply_network_overlay_to_snapshot(
        &self,
        snapshot: &mut DashboardActivityLiveSnapshot,
    ) {
        self.dashboard.apply_network_overlay_to_snapshot(snapshot);
    }

    pub(crate) fn runtime_record_count(&self) -> usize {
        self.invocation_store.runtime_record_count()
    }

    pub(crate) fn memory_estimate(&self) -> MemoryComponentEstimate {
        self.invocation_store.memory_estimate()
    }

    pub(crate) fn upsert(&self, record: ApiInvocation) -> RuntimeInvocationStoreUpsertOutcome {
        let key = RuntimeInvocationKey::new(record.invoke_id.clone(), record.occurred_at.clone());
        let outcome = self.invocation_store.upsert(record);
        if outcome.pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        } else if !outcome.skipped_terminal {
            self.sync_dashboard_runtime_key(&key, "runtime_upsert");
        }
        outcome
    }

    pub(crate) fn upsert_terminal(
        &self,
        record: ApiInvocation,
    ) -> RuntimeInvocationStoreRemoveOutcome {
        let key = RuntimeInvocationKey::new(record.invoke_id.clone(), record.occurred_at.clone());
        let (outcome, pruned_count) = self.invocation_store.upsert_terminal(record);
        if outcome.already_terminal {
            if pruned_count > 0 {
                self.rebuild_dashboard_runtime_records("runtime_prune");
            }
            return outcome;
        }
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        } else {
            self.dashboard
                .update_runtime_record(key, None, "terminal_delta");
        }
        self.mark_dashboard_terminal_dirty();
        outcome
    }

    pub(crate) fn clear_terminal_tombstone(&self, invoke_id: &str, occurred_at: &str) -> bool {
        let removed = self
            .invocation_store
            .clear_terminal_tombstone(invoke_id, occurred_at);
        if removed {
            let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
            self.sync_dashboard_runtime_key(&key, "terminal_rollback");
            self.mark_dashboard_terminal_dirty();
        }
        removed
    }

    pub(crate) fn contains_terminal(&self, invoke_id: &str, occurred_at: &str) -> bool {
        let (contains_terminal, pruned_count) = self
            .invocation_store
            .contains_terminal(invoke_id, occurred_at);
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
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let (removed, pruned_count) = self
            .invocation_store
            .remove_non_terminal(invoke_id, occurred_at);
        if removed.is_some() {
            self.dashboard
                .update_runtime_record(key, None, "runtime_remove");
        }
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        removed
    }

    pub(crate) fn remove_non_terminal_by_invoke_id(&self, invoke_id: &str) -> Vec<ApiInvocation> {
        let (removed, pruned_count) = self
            .invocation_store
            .remove_non_terminal_by_invoke_id(invoke_id);
        for (key, _) in &removed {
            self.dashboard
                .update_runtime_record(key.clone(), None, "runtime_remove");
        }
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        removed.into_iter().map(|(_, record)| record).collect()
    }

    pub(crate) fn remove_persisted_terminal_overlay(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        let key = RuntimeInvocationKey::new(invoke_id, occurred_at);
        let (removed, pruned_count) = self
            .invocation_store
            .remove_persisted_terminal_overlay(invoke_id, occurred_at);
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        } else {
            self.dashboard
                .update_runtime_record(key, None, "terminal_persisted");
        }
        self.mark_dashboard_terminal_dirty();
        removed
    }

    pub(crate) fn snapshot(&self) -> Vec<ApiInvocation> {
        let (snapshot, pruned_count) = self.invocation_store.snapshot();
        if pruned_count > 0 {
            self.rebuild_dashboard_runtime_records("runtime_prune");
        }
        snapshot
    }

    pub(crate) fn record_by_identity(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> Option<ApiInvocation> {
        self.invocation_store
            .record_by_identity(invoke_id, occurred_at)
    }

    pub(crate) fn prompt_cache_projection_by_identity(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> Option<PromptCacheRuntimeProjection> {
        self.invocation_store
            .prompt_cache_projection_by_identity(invoke_id, occurred_at)
    }

    #[cfg(test)]
    pub(crate) fn reset_full_record_clone_count(&self) {
        self.invocation_store.reset_full_record_clone_count();
    }

    #[cfg(test)]
    pub(crate) fn full_record_clone_count(&self) -> u64 {
        self.invocation_store.full_record_clone_count()
    }

    #[cfg(test)]
    pub(crate) fn backdate_for_test(&self, invoke_id: &str, occurred_at: &str, age: Duration) {
        self.invocation_store
            .backdate_for_test(invoke_id, occurred_at, age);
    }

    pub(crate) fn shutdown_summary(&self) -> RuntimeInvocationStoreShutdownSummary {
        self.invocation_store.shutdown_summary()
    }
}

#[derive(Debug)]
pub(crate) struct AppState {
    pub(crate) config: AppConfig,
    pub(crate) pool: Pool<Sqlite>,
    pub(crate) process_started_at_utc: DateTime<Utc>,
    pub(crate) performance_telemetry: Arc<PerformanceTelemetryRuntime>,
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
    pub(crate) raw_capture_circuit: Arc<RawCaptureCircuitBreaker>,
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
    pub(crate) pool_routing_availability: PoolRoutingAvailabilitySignal,
    pub(crate) pool_routing_runtime_cache: Arc<Mutex<Option<PoolRoutingRuntimeCache>>>,
    #[cfg(test)]
    pub(crate) pool_routing_test_data_version_connection:
        Arc<Mutex<Option<sqlx::pool::PoolConnection<Sqlite>>>>,
    pub(crate) pool_model_routing_cache_write_lock: Arc<Mutex<()>>,
    pub(crate) pool_live_attempt_ids: Arc<std::sync::Mutex<HashSet<i64>>>,
    pub(crate) pool_group_429_retry_delay_override: Option<Duration>,
    #[cfg(test)]
    pub(crate) fallback_proxy_429_retry_delay_override: Option<Duration>,
    pub(crate) pool_no_available_wait: PoolNoAvailableWaitSettings,
    pub(crate) upstream_accounts: Arc<UpstreamAccountsRuntime>,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolRoutingAvailabilitySignal {
    sender: tokio::sync::watch::Sender<u64>,
}

impl Default for PoolRoutingAvailabilitySignal {
    fn default() -> Self {
        let (sender, _receiver) = tokio::sync::watch::channel(0);
        Self { sender }
    }
}

impl PoolRoutingAvailabilitySignal {
    pub(crate) fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.sender.subscribe()
    }

    pub(crate) fn publish(&self) {
        self.sender.send_modify(|generation| {
            *generation = generation.wrapping_add(1);
        });
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PricingCatalog {
    pub(crate) version: String,
    pub(crate) models: HashMap<String, ModelPricing>,
}

#[cfg(test)]
mod pool_routing_prompt_route_cache_tests {
    use super::*;

    fn key(index: usize) -> PoolRoutingPromptRouteCacheKey {
        PoolRoutingPromptRouteCacheKey {
            prompt_cache_key: format!("prompt-{index}"),
            request_contains_encrypted_content: false,
            encrypted_session_owner_routing_enabled: false,
        }
    }

    #[test]
    fn routing_hot_cache_retains_negative_entries_and_evicts_lru() {
        let mut cache = PoolRoutingPromptRouteCache::default();
        for index in 0..POOL_ROUTING_PROMPT_ROUTE_CACHE_CAPACITY {
            cache.insert(key(index), None);
        }
        assert!(cache.get(&key(0)).is_some_and(|value| value.is_none()));
        cache.insert(key(POOL_ROUTING_PROMPT_ROUTE_CACHE_CAPACITY), None);

        assert!(cache.get(&key(0)).is_some_and(|value| value.is_none()));
        assert!(cache.get(&key(1)).is_none());
        assert!(
            cache
                .get(&key(POOL_ROUTING_PROMPT_ROUTE_CACHE_CAPACITY))
                .is_some_and(|value| value.is_none())
        );
    }

    #[test]
    fn routing_hot_cache_invalidates_all_variants_for_prompt_key() {
        let mut cache = PoolRoutingPromptRouteCache::default();
        let mut encrypted = key(1);
        encrypted.request_contains_encrypted_content = true;
        cache.insert(key(1), None);
        cache.insert(encrypted, None);
        cache.insert(key(2), None);

        cache.invalidate_prompt_cache_key("prompt-1");

        assert!(cache.get(&key(1)).is_none());
        assert!(cache.get(&key(2)).is_some_and(|value| value.is_none()));
    }

    #[test]
    fn routing_hot_cache_invalidates_every_sticky_model_variant() {
        let mut cache = PoolRoutingStickyRouteCache::default();
        let base = PoolRoutingStickyRouteCacheKey {
            sticky_key: "sticky-1".to_string(),
            model_key: None,
        };
        let model = PoolRoutingStickyRouteCacheKey {
            sticky_key: "sticky-1".to_string(),
            model_key: Some("gpt-5".to_string()),
        };
        let other = PoolRoutingStickyRouteCacheKey {
            sticky_key: "sticky-2".to_string(),
            model_key: None,
        };
        let negative = PoolRoutingStickyRouteCacheValue {
            route: None,
            affinity_generation: 7,
        };
        cache.insert(base.clone(), negative.clone());
        cache.insert(model.clone(), negative);
        cache.insert(
            other.clone(),
            PoolRoutingStickyRouteCacheValue {
                route: None,
                affinity_generation: 9,
            },
        );

        cache.invalidate_sticky_key("sticky-1");

        assert!(cache.get(&base).is_none());
        assert!(cache.get(&model).is_none());
        assert_eq!(
            cache
                .get(&other)
                .expect("other key remains cached")
                .affinity_generation,
            9
        );
    }
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
    pub(crate) image_models: Vec<String>,
    pub(crate) enabled_models: Vec<String>,
}

impl ProxyModelSettingsResponse {
    pub(crate) fn from_settings(value: ProxyModelSettings) -> Self {
        let models = PROXY_PRESET_MODEL_IDS
            .iter()
            .map(|model| (*model).to_string())
            .collect();
        Self::from_settings_with_models(value, models)
    }

    pub(crate) fn from_settings_with_models(
        value: ProxyModelSettings,
        models: Vec<String>,
    ) -> Self {
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
            models,
            image_models: PROXY_IMAGE_MODEL_IDS
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
    HourlyRollupBootstrap,
    ForwardProxySubscriptionRefresh,
}

impl SystemTaskKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::RetentionArchive => "retention_archive",
            Self::StartupBackfill => "startup_backfill",
            Self::HourlyRollupBootstrap => "startup_hourly_rollup_bootstrap",
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
    // The durable inventory value captured by the background refresh. An in-memory override
    // may temporarily replace the response state, then restore this value without request I/O.
    pub(crate) raw_metrics_inventory_state: String,
}

#[derive(Debug, Default)]
pub(crate) struct SystemStatusCacheState {
    pub(crate) latest: Option<SystemStatusCacheEntry>,
    pub(crate) in_flight: Option<watch::Sender<bool>>,
    // This admission survives a logically cancelled refresh until its blocking filesystem scan
    // actually exits. A started `spawn_blocking` filesystem call cannot be cancelled safely.
    pub(crate) filesystem_scan_in_flight: Arc<AtomicBool>,
    pub(crate) waiter_count: usize,
    pub(crate) raw_metrics_health_override: Option<String>,
}

pub(crate) fn default_enabled_preset_models() -> Vec<String> {
    PROXY_PRESET_MODEL_IDS
        .iter()
        .filter(|model| **model != "gpt-5.4-mini")
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn normalize_enabled_preset_models(enabled_models: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let normalized = enabled_models
        .into_iter()
        .filter_map(|model| {
            let model = model.trim().to_string();
            (!model.is_empty() && model.len() <= 128 && seen.insert(model.clone())).then_some(model)
        })
        .collect::<Vec<_>>();
    let enabled = normalized
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let static_models = PROXY_PRESET_MODEL_IDS
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    let mut ordered = PROXY_PRESET_MODEL_IDS
        .iter()
        .filter(|model| enabled.contains(**model))
        .map(|model| (*model).to_string())
        .collect::<Vec<_>>();
    ordered.extend(
        normalized
            .into_iter()
            .filter(|model| !static_models.contains(model.as_str())),
    );
    ordered
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
    pub(crate) models_dev: Client,
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

        let models_dev = Self::builder(Some(timeout), &user_agent)
            .pool_max_idle_per_host(config.shared_connection_parallelism)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("failed to construct models.dev HTTP client")?;

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
            models_dev,
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
        let mut core = empty_dashboard_live_core();
        core.in_progress_invocation_count = 1;
        hub.set_live_core_for_test(core);

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
