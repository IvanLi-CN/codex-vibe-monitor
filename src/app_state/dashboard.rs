use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use serde::Serialize;
use tokio::sync::Notify;

use crate::{
    ApiInvocation, DashboardActivityLiveAccount, DashboardActivityLiveSnapshot,
    DashboardActivityTerminalDelta, DashboardNetworkByteTotals, DashboardNetworkProjectionSlice,
    DashboardNetworkScopeKey, DashboardNetworkSpeedCache, InvocationPhaseCountsResponse,
    InvocationSourceScope, SOURCE_PROXY, normalize_trimmed_optional_string_local,
    normalized_runtime_text, normalized_wait_ms, reserve_dashboard_activity_live_revision,
    runtime_record_live_phase_with_retry,
};

use super::invocation_store::{ProxyRuntimeInvocationStoreInner, RuntimeInvocationKey};

pub(crate) const DASHBOARD_RUNTIME_PROJECTION_COALESCE: Duration = Duration::from_millis(250);
pub(crate) const DASHBOARD_RUNTIME_NETWORK_PROJECTION_COALESCE: Duration = Duration::from_secs(1);
pub(crate) const DASHBOARD_RUNTIME_TERMINAL_PROJECTION_COALESCE: Duration = Duration::from_secs(5);
const DASHBOARD_RUNTIME_TERMINAL_MAX_PENDING: usize = 10_000;
const DASHBOARD_RUNTIME_TERMINAL_MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;

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

pub(crate) fn empty_dashboard_live_core() -> DashboardActivityLiveSnapshot {
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
    pub(crate) bucket_start: chrono::DateTime<chrono::Utc>,
    pub(crate) bucket_end: chrono::DateTime<chrono::Utc>,
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

#[derive(Debug, Clone)]
pub(crate) struct DashboardProjectionHealthSnapshot {
    pub(crate) state: &'static str,
    pub(crate) last_good_age_ms: Option<u64>,
    pub(crate) revision: u64,
    pub(crate) snapshot_origin: &'static str,
    pub(crate) degraded_reason: Option<String>,
    pub(crate) last_defer_reason: Option<String>,
    pub(crate) slice_counters: DashboardRuntimeTopologyCounterSnapshot,
}

#[derive(Debug)]
pub(crate) struct DashboardRuntimeProjection {
    state: Mutex<DashboardRuntimeProjectionState>,
    network_speed_cache: OnceLock<Arc<DashboardNetworkSpeedCache>>,
    publish_notify: Notify,
    topology_counters: DashboardRuntimeTopologyCounters,
}

impl Default for DashboardRuntimeProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl DashboardRuntimeProjection {
    pub(crate) fn new() -> Self {
        Self {
            state: Mutex::new(DashboardRuntimeProjectionState::default()),
            network_speed_cache: OnceLock::new(),
            publish_notify: Notify::new(),
            topology_counters: DashboardRuntimeTopologyCounters::default(),
        }
    }

    pub(crate) fn bind_network_speed_cache(
        &self,
        cache: Arc<DashboardNetworkSpeedCache>,
    ) -> Result<()> {
        if let Some(existing) = self.network_speed_cache.get() {
            return if Arc::ptr_eq(existing, &cache) {
                Ok(())
            } else {
                Err(anyhow!(
                    "runtime projection hub is already bound to another dashboard network cache"
                ))
            };
        }
        self.network_speed_cache
            .set(cache)
            .map_err(|_| anyhow!("dashboard network speed cache is already bound"))
    }

    pub(crate) fn live_projection(&self) -> DashboardLiveProjection<'_> {
        DashboardLiveProjection { projection: self }
    }

    pub(crate) fn update_runtime_record(
        &self,
        key: RuntimeInvocationKey,
        record: Option<&ApiInvocation>,
        trigger: &'static str,
    ) {
        let Ok(mut dashboard) = self.state.lock() else {
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
        self.publish_notify.notify_one();
    }

    pub(crate) fn rebuild_runtime_records(
        &self,
        runtime: &ProxyRuntimeInvocationStoreInner,
        trigger: &'static str,
    ) {
        let Ok(mut dashboard) = self.state.lock() else {
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
        self.publish_notify.notify_one();
    }

    pub(crate) fn mark_dirty(&self, trigger: &'static str) {
        self.mark_dirty_at(trigger, Instant::now());
    }

    pub(crate) fn mark_dirty_at(&self, trigger: &'static str, now: Instant) {
        let Ok(mut dashboard) = self.state.lock() else {
            return;
        };
        mark_dashboard_state_dirty(&mut dashboard, trigger, now);
        self.publish_notify.notify_one();
    }

    pub(crate) fn mark_network_dirty(&self) {
        self.mark_network_dirty_at(Instant::now());
    }

    pub(crate) fn mark_network_dirty_at(&self, now: Instant) {
        let Ok(mut dashboard) = self.state.lock() else {
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
        self.publish_notify.notify_one();
    }

    pub(crate) fn mark_terminal_dirty(&self) {
        self.mark_terminal_dirty_at(Instant::now());
    }

    pub(crate) fn mark_terminal_dirty_at(&self, now: Instant) {
        let Ok(mut dashboard) = self.state.lock() else {
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
        self.publish_notify.notify_one();
    }

    pub(crate) async fn wait_for_publish_signal(&self) {
        self.publish_notify.notified().await;
    }

    pub(crate) fn pending_deadline(&self) -> Option<Instant> {
        self.state
            .lock()
            .ok()
            .and_then(|dashboard| dashboard.pending_deadline)
    }

    pub(crate) fn pending_publish_window(&self) -> Option<DashboardProjectionPublishWindow> {
        let dashboard = self.state.lock().ok()?;
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

    pub(crate) fn has_pending_terminal_publish(&self) -> bool {
        self.state
            .lock()
            .map(|dashboard| dashboard.pending_terminal_deadline.is_some())
            .unwrap_or(false)
    }

    pub(crate) fn begin_publish_window(
        &self,
        window: DashboardProjectionPublishWindow,
    ) -> Option<DashboardProjectionPublishWindow> {
        let mut dashboard = self.state.lock().ok()?;
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

    pub(crate) fn complete_publish_window(&self, window: DashboardProjectionPublishWindow) {
        if let Ok(dashboard) = self.state.lock() {
            let generation = match window.slice {
                DashboardProjectionSlice::Current => dashboard.dirty_generation,
                DashboardProjectionSlice::Network => dashboard.network_dirty_generation,
                DashboardProjectionSlice::Terminal => dashboard.terminal_dirty_generation,
            };
            debug_assert!(generation >= window.generation);
        }
    }

    pub(crate) fn is_memory_ready(&self) -> bool {
        self.state
            .lock()
            .map(|dashboard| dashboard.memory_ready && dashboard.degraded_reason.is_none())
            .unwrap_or(false)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.state
            .lock()
            .map(|dashboard| dashboard.dirty_generation)
            .unwrap_or_default()
    }

    pub(crate) fn capture_memory_snapshot(&self) -> Result<DashboardProjectionCapture> {
        let candidate = self.live_projection().snapshot()?;
        let mut dashboard = self
            .state
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
        self.topology_counters
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
            .network_speed_cache
            .get()
            .ok_or_else(|| anyhow!("dashboard network speed cache is not bound"))?;
        let (network_open_buckets, known_account_ids) = {
            let dashboard = self
                .state
                .lock()
                .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?;
            let network_open_buckets = dashboard
                .persistence_baseline
                .as_ref()
                .map(|baseline| baseline.network_open_buckets.clone())
                .unwrap_or_default();
            let known_account_ids = dashboard
                .network_last_good
                .as_ref()
                .map(|slice| {
                    slice
                        .accounts
                        .iter()
                        .map(|account| account.upstream_account_id)
                        .collect()
                })
                .unwrap_or_default();
            (network_open_buckets, known_account_ids)
        };
        let candidate = DashboardNetworkProjectionSlice::from_memory(
            dashboard_network_speed_cache.as_ref(),
            &network_open_buckets,
            &known_account_ids,
        );
        self.topology_counters
            .network
            .build_count
            .fetch_add(1, Ordering::Relaxed);
        let mut dashboard = self
            .state
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
        self.topology_counters
            .network
            .revision_count
            .fetch_add(1, Ordering::Relaxed);
        dashboard.network_last_good = Some(snapshot.clone());
        Ok(DashboardNetworkProjectionCapture {
            slice: snapshot,
            changed: true,
        })
    }

    pub(crate) fn record_terminal_delta(&self, delta: DashboardActivityTerminalDelta) {
        let Ok(mut dashboard) = self.state.lock() else {
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
        self.publish_notify.notify_one();
    }

    pub(crate) fn discard_terminal_delta(&self, invoke_id: &str, occurred_at: &str) {
        let Ok(mut dashboard) = self.state.lock() else {
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
        let Ok(mut dashboard) = self.state.lock() else {
            return None;
        };
        self.topology_counters
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
        self.topology_counters
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
        self.state
            .lock()
            .map(|dashboard| dashboard.terminal_pending_deltas.len())
            .unwrap_or_default()
    }

    pub(crate) fn legacy_live_snapshot(
        &self,
        mut current: DashboardActivityLiveSnapshot,
    ) -> DashboardActivityLiveSnapshot {
        if let Ok(dashboard) = self.state.lock()
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
        let mut current = self.state.lock().ok()?.last_good.clone()?;
        apply_dashboard_network_slice_to_live_snapshot(&mut current, network);
        Some(self.commit_legacy_live_snapshot(current))
    }

    fn commit_legacy_live_snapshot(
        &self,
        mut candidate: DashboardActivityLiveSnapshot,
    ) -> DashboardActivityLiveSnapshot {
        let Ok(mut dashboard) = self.state.lock() else {
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
        mut baseline: DashboardRuntimeProjectionBaseline,
        snapshot_origin: &'static str,
        expected_generation: u64,
        runtime: &ProxyRuntimeInvocationStoreInner,
    ) -> Result<Option<DashboardProjectionCapture>> {
        let mut projection_records = baseline
            .records
            .iter()
            .cloned()
            .map(|record| (record.key.clone(), record))
            .collect::<HashMap<_, _>>();
        let baseline_records = projection_records.clone();
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
            .state
            .lock()
            .map_err(|_| anyhow!("runtime projection state lock is poisoned"))?;
        if dashboard.dirty_generation < expected_generation {
            tracing::warn!(
                expected_generation,
                actual_generation = dashboard.dirty_generation,
                "rejecting persistence baseline with an impossible generation rollback"
            );
            return Ok(None);
        }
        let snapshot_origin = if dashboard.dirty_generation > expected_generation {
            "reconcile_replayed"
        } else {
            snapshot_origin
        };
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
            self.topology_counters
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
        let mut dashboard = self.state.lock().ok()?;
        let snapshot = dashboard.last_good.clone()?;
        dashboard.last_snapshot_origin = Some(snapshot_origin);
        Some(DashboardProjectionCapture {
            snapshot,
            changed: false,
            snapshot_origin,
        })
    }

    pub(crate) fn mark_degraded(&self, reason: &'static str) {
        if let Ok(mut dashboard) = self.state.lock() {
            dashboard.degraded_reason = Some(reason);
        }
    }

    pub(crate) fn record_reconcile_failure(&self, reason: &'static str) {
        if let Ok(mut dashboard) = self.state.lock() {
            dashboard.reconcile_error = Some(reason);
            dashboard.last_reconcile_defer_reason = None;
        }
    }

    pub(crate) fn record_reconcile_deferred(&self, reason: &'static str) {
        if let Ok(mut dashboard) = self.state.lock() {
            dashboard.last_reconcile_defer_reason = Some(reason);
        }
    }

    pub(crate) fn record_build(&self) {
        self.topology_counters
            .current
            .build_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_current_slice_cadence_miss(&self) {
        self.topology_counters
            .current
            .cadence_miss_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_network_slice_cadence_miss(&self) {
        self.topology_counters
            .network
            .cadence_miss_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_terminal_slice_cadence_miss(&self) {
        self.topology_counters
            .terminal
            .cadence_miss_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn topology_counters(&self) -> DashboardRuntimeTopologyCounterSnapshot {
        self.topology_counters.snapshot()
    }

    #[cfg(test)]
    pub(crate) fn reset_topology_counters(&self) {
        self.topology_counters.reset();
    }

    #[cfg(test)]
    pub(crate) fn set_live_core_for_test(&self, core: DashboardActivityLiveSnapshot) {
        if let Ok(mut dashboard) = self.state.lock() {
            dashboard.live_core = Some(core);
        }
    }

    pub(crate) fn health_snapshot(&self) -> DashboardProjectionHealthSnapshot {
        let slice_counters = self.topology_counters.snapshot();
        let dashboard = self.state.lock().ok();
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
        DashboardProjectionHealthSnapshot {
            state,
            last_good_age_ms,
            revision: dashboard
                .as_ref()
                .and_then(|state| state.last_good.as_ref())
                .map_or(0, |snapshot| snapshot.revision),
            snapshot_origin: dashboard
                .as_ref()
                .and_then(|state| state.last_snapshot_origin)
                .unwrap_or("none"),
            degraded_reason: dashboard
                .as_ref()
                .and_then(|state| state.degraded_reason.or(state.reconcile_error))
                .map(str::to_string),
            last_defer_reason: dashboard
                .as_ref()
                .and_then(|state| state.last_reconcile_defer_reason)
                .map(str::to_string),
            slice_counters,
        }
    }

    pub(crate) fn apply_network_overlay_to_snapshot(
        &self,
        snapshot: &mut DashboardActivityLiveSnapshot,
    ) {
        if let Ok(dashboard) = self.state.lock()
            && let Some(network) = dashboard.network_last_good.as_ref()
        {
            apply_dashboard_network_slice_to_live_snapshot(snapshot, network);
        }
    }
}

pub(crate) struct DashboardLiveProjection<'a> {
    projection: &'a DashboardRuntimeProjection,
}

impl DashboardLiveProjection<'_> {
    pub(crate) fn snapshot(&self) -> Result<DashboardActivityLiveSnapshot> {
        self.projection
            .state
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

pub(crate) fn dashboard_current_snapshot_content_eq(
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
        && left.recent == right.recent
        && left.current_snapshot == right.current_snapshot
        && left.current_snapshot_by_account == right.current_snapshot_by_account
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
