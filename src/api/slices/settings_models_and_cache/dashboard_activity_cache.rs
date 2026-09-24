use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct DashboardActivitySnapshotSelection {
    pub(crate) range: String,
    pub(crate) range_anchor: String,
    pub(crate) time_zone: String,
    pub(crate) source_scope: String,
    pub(crate) recent_limit: usize,
    pub(crate) include_accounts: bool,
    pub(crate) include_recent: bool,
}

pub(crate) fn dashboard_activity_selection_fingerprint(
    selection: &DashboardActivitySnapshotSelection,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    selection.hash(&mut hasher);
    hasher.finish()
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardActivitySnapshotCacheEntry {
    pub(crate) cached_at: Instant,
    /// Controls the next reconciliation attempt independently from the DB baseline age.
    pub(crate) last_reconcile_attempted_at: Instant,
    /// Only failed reconciliations may throttle an expiry-horizon rebuild. A healthy entry whose
    /// rolling coverage is exhausted must reconcile immediately.
    pub(crate) last_reconcile_failed: bool,
    /// The live invocation cursor observed immediately before the DB baseline build. Pending
    /// terminal deltas beyond this cursor are replayed before the entry is published.
    pub(crate) baseline_snapshot_cursor: i64,
    /// Rolling ranges may only reuse the baseline while their moving lower bound remains within
    /// the terminal rows captured for expiry subtraction. Calendar ranges do not need this gate.
    pub(crate) expiry_covered_until: Option<DateTime<Utc>>,
    pub(crate) expiry_terminal_deltas: VecDeque<DashboardActivityTerminalDelta>,
    pub(crate) expiry_delta_estimated_bytes: usize,
    pub(crate) response: DashboardActivitySnapshot,
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardActivityTerminalDelta {
    pub(crate) terminal_sequence: u64,
    /// Keeps the exact timeseries classification and latency samples available to the
    /// dashboard topic materializers without reloading an invocation from SQLite.
    pub(crate) timeseries: TimeseriesTerminalDelta,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) success: bool,
    pub(crate) failure: bool,
    pub(crate) total_tokens: i64,
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_read_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) has_cost: bool,
    pub(crate) total_cost: f64,
    pub(crate) cost_input: f64,
    pub(crate) cost_cache_write: f64,
    pub(crate) cost_cache_read: f64,
    pub(crate) cost_output: f64,
    pub(crate) cost_reasoning: f64,
    pub(crate) cost_unknown: f64,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    /// Bounded Dashboard projection data for retaining the existing recent-invocation contract
    /// without reloading SQLite or broadcasting a full invocation record.
    pub(crate) recent_invocation: PromptCacheConversationInvocationPreviewResponse,
    pub(crate) persisted_row_id: Option<i64>,
    pub(crate) estimated_bytes: usize,
}

impl DashboardActivityTerminalDelta {
    pub(crate) fn key(&self) -> (String, String) {
        (self.invoke_id.clone(), self.occurred_at.clone())
    }
}

/// Write-side state layered over the last DB-backed Dashboard snapshot.
///
/// Terminal records are accepted by the write controller before SQLite flushes. Keeping their
/// identities here lets the read side publish those deltas immediately without rebuilding the
/// full range for every terminal event.
#[derive(Debug, Default)]
pub(crate) struct DashboardActivityReadModel {
    pub(crate) applied_terminal_keys: HashMap<(String, String), Instant>,
    pub(crate) applied_terminal_key_order: VecDeque<((String, String), Instant)>,
    pub(crate) pending_terminal_deltas: VecDeque<DashboardActivityTerminalDelta>,
    pub(crate) pending_delta_estimated_bytes: usize,
    pub(crate) persisted_ack_pending_count: usize,
    pub(crate) delta_pruned_count: u64,
    pub(crate) hard_limit_reason: Option<&'static str>,
    pub(crate) hard_limit_sequence: Option<u64>,
    pub(crate) next_terminal_sequence: u64,
    pub(crate) settled_terminal_sequence: u64,
    pub(crate) settled_terminal_sequences: BTreeSet<u64>,
    pub(crate) sequence_gap_count: u64,
    pub(crate) terminal_delta_count: u64,
    pub(crate) duplicate_delta_count: u64,
    pub(crate) pending_terminal_overflow_count: u64,
}

#[derive(Debug)]
pub(crate) struct DashboardActivitySnapshotInFlight {
    pub(crate) signal: watch::Sender<bool>,
    pub(crate) waiter_count: usize,
    pub(crate) baseline_cursor: Option<i64>,
    pub(crate) routing_rules_generation: u64,
}

#[derive(Debug, Default)]
pub(crate) struct DashboardActivitySnapshotCacheState {
    pub(crate) entries:
        HashMap<DashboardActivitySnapshotSelection, DashboardActivitySnapshotCacheEntry>,
    pub(crate) in_flight:
        HashMap<DashboardActivitySnapshotSelection, DashboardActivitySnapshotInFlight>,
    pub(crate) invalidation_reasons: HashMap<DashboardActivitySnapshotSelection, &'static str>,
    pub(crate) routing_rules_generation: u64,
    pub(crate) read_model: DashboardActivityReadModel,
    /// Serializes DB-backed baseline builds across distinct selections. Per-selection
    /// singleflight only coalesces identical requests and cannot prevent competing SQLite
    /// consistency transactions for different ranges.
    pub(crate) baseline_build_gate: Arc<Mutex<()>>,
}

pub(crate) async fn dashboard_activity_snapshot_cache_memory_estimate(
    cache: &Arc<Mutex<DashboardActivitySnapshotCacheState>>,
) -> MemoryComponentEstimate {
    let cache = cache.lock().await;
    let entry_count = cache.entries.len();
    let in_flight_count = cache.in_flight.len();
    let pending_delta_bytes = cache.read_model.pending_delta_estimated_bytes;
    let applied_key_bytes = cache
        .read_model
        .applied_terminal_keys
        .keys()
        .map(|(invoke_id, occurred_at)| invoke_id.capacity() + occurred_at.capacity())
        .sum::<usize>();
    let expiry_bytes = cache
        .entries
        .values()
        .map(|entry| {
            entry
                .expiry_delta_estimated_bytes
                .saturating_add(std::mem::size_of::<DashboardActivitySnapshotCacheEntry>())
                .saturating_add(dashboard_activity_snapshot_memory_estimate(&entry.response))
        })
        .sum::<usize>();
    MemoryComponentEstimate {
        entries: entry_count.saturating_add(in_flight_count),
        bytes: pending_delta_bytes
            .saturating_add(applied_key_bytes)
            .saturating_add(expiry_bytes)
            .saturating_add(
                entry_count
                    .saturating_add(in_flight_count)
                    .saturating_mul(std::mem::size_of::<usize>() * 8),
            ),
        detail_items: cache.read_model.pending_terminal_deltas.len(),
    }
}

#[derive(Debug)]
pub(crate) struct DashboardActivitySnapshotFlightGuard {
    pub(crate) cache: Arc<Mutex<DashboardActivitySnapshotCacheState>>,
    pub(crate) selection: DashboardActivitySnapshotSelection,
    pub(crate) routing_rules_generation: u64,
    pub(crate) active: bool,
}

impl DashboardActivitySnapshotFlightGuard {
    pub(crate) fn new(
        cache: Arc<Mutex<DashboardActivitySnapshotCacheState>>,
        selection: DashboardActivitySnapshotSelection,
        routing_rules_generation: u64,
    ) -> Self {
        Self {
            cache,
            selection,
            routing_rules_generation,
            active: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for DashboardActivitySnapshotFlightGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let cache = self.cache.clone();
        let selection = self.selection.clone();
        let routing_rules_generation = self.routing_rules_generation;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut state = cache.lock().await;
                if let Some(in_flight) = state.in_flight.remove(&selection) {
                    if in_flight.routing_rules_generation != routing_rules_generation {
                        state.in_flight.insert(selection, in_flight);
                        return;
                    }
                    let _ = in_flight.signal.send(true);
                }
            });
            return;
        }

        if let Ok(mut state) = cache.try_lock()
            && let Some(in_flight) = state.in_flight.remove(&selection)
        {
            if in_flight.routing_rules_generation != routing_rules_generation {
                state.in_flight.insert(selection, in_flight);
                return;
            }
            let _ = in_flight.signal.send(true);
        }
    }
}

pub(crate) async fn invalidate_dashboard_activity_snapshot_cache(
    cache: &Mutex<DashboardActivitySnapshotCacheState>,
    selection: &DashboardActivitySnapshotSelection,
    reason: &'static str,
) {
    let in_flight = {
        let mut state = cache.lock().await;
        state.entries.remove(selection);
        state.invalidation_reasons.insert(selection.clone(), reason);
        state.in_flight.remove(selection)
    };

    if let Some(flight) = in_flight {
        let _ = flight.signal.send(true);
    }
}

pub(crate) async fn invalidate_dashboard_activity_snapshots_with_accounts(
    cache: &Mutex<DashboardActivitySnapshotCacheState>,
    reason: &'static str,
) {
    let in_flight = {
        let mut state = cache.lock().await;
        state.routing_rules_generation = state.routing_rules_generation.saturating_add(1);
        state
            .entries
            .retain(|selection, _| !selection.include_accounts);
        let selections = state
            .in_flight
            .keys()
            .filter(|selection| selection.include_accounts)
            .cloned()
            .collect::<Vec<_>>();
        for selection in &selections {
            state.invalidation_reasons.insert(selection.clone(), reason);
        }
        selections
            .into_iter()
            .filter_map(|selection| state.in_flight.remove(&selection))
            .collect::<Vec<_>>()
    };

    for flight in in_flight {
        let _ = flight.signal.send(true);
    }
}
