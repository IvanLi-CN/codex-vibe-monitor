#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationsResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) snapshot_at: Option<String>,
    pub(crate) selection_mode: PromptCacheConversationSelectionMode,
    pub(crate) selected_limit: Option<i64>,
    pub(crate) selected_activity_hours: Option<i64>,
    pub(crate) selected_activity_minutes: Option<i64>,
    pub(crate) implicit_filter: PromptCacheConversationImplicitFilter,
    pub(crate) total_matched: Option<i64>,
    pub(crate) has_more: bool,
    pub(crate) next_cursor: Option<String>,
    pub(crate) conversations: Vec<PromptCacheConversationResponse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PromptCacheConversationSelectionMode {
    Count,
    ActivityWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PromptCacheConversationSelection {
    Count(i64),
    ActivityWindowHours(i64),
    ActivityWindowMinutes(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PromptCacheConversationDetailLevel {
    Full,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BlockedBindingConstraintSource {
    UpstreamAccountBinding,
    EncryptedSessionOwner,
}

impl BlockedBindingConstraintSource {
    pub(crate) fn from_query_param(raw: &str) -> Option<Self> {
        match raw.trim() {
            "upstreamAccountBinding" => Some(Self::UpstreamAccountBinding),
            "encryptedSessionOwner" => Some(Self::EncryptedSessionOwner),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BlockedBindingRecoveryAction {
    ClearAndResetAffinity,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlockedBindingDiagnostic {
    pub(crate) constraint_source: BlockedBindingConstraintSource,
    pub(crate) upstream_account_id: i64,
    pub(crate) upstream_account_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) recovery_action: BlockedBindingRecoveryAction,
}

pub(crate) fn blocked_binding_account_label(
    upstream_account_label: Option<&str>,
    upstream_account_id: i64,
) -> String {
    upstream_account_label
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("#{upstream_account_id}"))
}

pub(crate) fn parse_blocked_binding_json(raw: Option<&str>) -> Option<BlockedBindingDiagnostic> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }
    serde_json::from_str(value).ok()
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PromptCacheConversationBlockedBindingFilter {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) constraint_source: Option<BlockedBindingConstraintSource>,
}

impl PromptCacheConversationBlockedBindingFilter {
    pub(crate) fn is_active(&self) -> bool {
        self.upstream_account_id.is_some() || self.constraint_source.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PromptCacheConversationsRequest {
    pub(crate) selection: PromptCacheConversationSelection,
    pub(crate) detail_level: PromptCacheConversationDetailLevel,
    pub(crate) recent_invocation_limit: Option<i64>,
    pub(crate) page_size: Option<i64>,
    pub(crate) cursor: Option<String>,
    pub(crate) snapshot_at: Option<String>,
    pub(crate) blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
}

impl PromptCacheConversationsRequest {
    pub(crate) fn legacy(selection: PromptCacheConversationSelection) -> Self {
        Self {
            selection,
            detail_level: PromptCacheConversationDetailLevel::Full,
            recent_invocation_limit: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            blocked_binding_filter: None,
        }
    }

    pub(crate) fn uses_legacy_cache(&self) -> bool {
        self.detail_level == PromptCacheConversationDetailLevel::Full
            && self.recent_invocation_limit.is_none()
            && self.page_size.is_none()
            && self.cursor.is_none()
            && self.snapshot_at.is_none()
            && self.blocked_binding_filter.is_none()
    }
}

impl PromptCacheConversationSelection {
    pub(crate) fn selection_mode(self) -> PromptCacheConversationSelectionMode {
        match self {
            Self::Count(_) => PromptCacheConversationSelectionMode::Count,
            Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => {
                PromptCacheConversationSelectionMode::ActivityWindow
            }
        }
    }

    pub(crate) fn activity_window_duration(self) -> ChronoDuration {
        match self {
            Self::Count(_) => ChronoDuration::hours(24),
            Self::ActivityWindowHours(hours) => ChronoDuration::hours(hours),
            Self::ActivityWindowMinutes(minutes) => ChronoDuration::minutes(minutes),
        }
    }

    pub(crate) fn display_limit(self) -> i64 {
        match self {
            Self::Count(limit) => limit,
            Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => {
                PROMPT_CACHE_CONVERSATION_ACTIVITY_MODE_LIMIT
            }
        }
    }

    pub(crate) fn selected_limit(self) -> Option<i64> {
        match self {
            Self::Count(limit) => Some(limit),
            Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => None,
        }
    }

    pub(crate) fn selected_activity_hours(self) -> Option<i64> {
        match self {
            Self::Count(_) => None,
            Self::ActivityWindowHours(hours) => Some(hours),
            Self::ActivityWindowMinutes(_) => None,
        }
    }

    pub(crate) fn selected_activity_minutes(self) -> Option<i64> {
        match self {
            Self::Count(_) | Self::ActivityWindowHours(_) => None,
            Self::ActivityWindowMinutes(minutes) => Some(minutes),
        }
    }

    pub(crate) fn implicit_filter(
        self,
        filtered_count: i64,
    ) -> PromptCacheConversationImplicitFilter {
        let kind = if filtered_count > 0 {
            Some(match self {
                Self::Count(_) => PromptCacheConversationImplicitFilterKind::InactiveOutside24h,
                Self::ActivityWindowHours(_) | Self::ActivityWindowMinutes(_) => {
                    PromptCacheConversationImplicitFilterKind::CappedTo50
                }
            })
        } else {
            None
        };

        PromptCacheConversationImplicitFilter {
            kind,
            filtered_count: filtered_count.max(0),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationImplicitFilter {
    pub(crate) kind: Option<PromptCacheConversationImplicitFilterKind>,
    pub(crate) filtered_count: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PromptCacheConversationImplicitFilterKind {
    InactiveOutside24h,
    CappedTo50,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationResponse {
    pub(crate) prompt_cache_key: String,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) last_activity_at: String,
    #[serde(
        serialize_with = "serialize_opt_local_or_utc_to_utc_iso",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) last_terminal_at: Option<String>,
    #[serde(
        serialize_with = "serialize_opt_local_or_utc_to_utc_iso",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) last_in_flight_at: Option<String>,
    pub(crate) in_flight_phase_counts: InvocationPhaseCountsResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
    pub(crate) has_encrypted_session_owner: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) encrypted_owner_account_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) encrypted_owner_account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) encrypted_owner_group_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) manual_binding: Option<PromptCacheConversationManualBindingResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
    pub(crate) upstream_accounts: Vec<PromptCacheConversationUpstreamAccountResponse>,
    pub(crate) recent_invocations: Vec<PromptCacheConversationInvocationPreviewResponse>,
    pub(crate) last24h_requests: Vec<PromptCacheConversationRequestPointResponse>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationManualBindingResponse {
    pub(crate) binding_kind: String,
    pub(crate) group_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationInvocationPreviewResponse {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) prompt_cache_key: Option<String>,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) live_phase: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) route_mode: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) response_model: Option<String>,
    pub(crate) total_tokens: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) upstream_account_plan_type: Option<String>,
    pub(crate) endpoint: Option<String>,
    pub(crate) compaction_request_kind: Option<String>,
    pub(crate) compaction_response_kind: Option<String>,
    pub(crate) image_intent: Option<String>,
    pub(crate) source: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) downstream_status_code: Option<i64>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
    pub(crate) is_actionable: Option<bool>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) request_compression_algorithm: Option<String>,
    pub(crate) transport: Option<String>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) service_tier: Option<String>,
    pub(crate) billing_service_tier: Option<String>,
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_req_read_ms: Option<f64>,
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_req_parse_ms: Option<f64>,
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_upstream_connect_ms: Option<f64>,
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) first_token_ms: Option<f64>,
    #[serde(serialize_with = "serialize_opt_finite_positive_timing")]
    pub(crate) t_upstream_stream_ms: Option<f64>,
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_resp_parse_ms: Option<f64>,
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_persist_ms: Option<f64>,
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) t_total_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationUpstreamAccountResponse {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) request_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) last_activity_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PromptCacheConversationRequestPointResponse {
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) status: String,
    pub(crate) is_success: bool,
    pub(crate) outcome: String,
    pub(crate) request_tokens: i64,
    pub(crate) cumulative_tokens: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheConversationsCacheEntry {
    pub(crate) cached_at: Instant,
    pub(crate) generation: u64,
    pub(crate) response: PromptCacheConversationsResponse,
}

#[derive(Debug)]
pub(crate) struct PromptCacheConversationInFlight {
    pub(crate) signal: watch::Sender<bool>,
    pub(crate) generation: u64,
}

#[derive(Debug, Default)]
pub(crate) struct PromptCacheConversationsCacheState {
    pub(crate) entries:
        HashMap<PromptCacheConversationSelection, PromptCacheConversationsCacheEntry>,
    pub(crate) in_flight:
        HashMap<PromptCacheConversationSelection, PromptCacheConversationInFlight>,
    pub(crate) generation: u64,
}

#[derive(Debug)]
pub(crate) struct PromptCacheConversationFlightGuard {
    pub(crate) cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    pub(crate) selection: PromptCacheConversationSelection,
    pub(crate) generation: u64,
    pub(crate) active: bool,
}

impl PromptCacheConversationFlightGuard {
    pub(crate) fn new(
        cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
        selection: PromptCacheConversationSelection,
        generation: u64,
    ) -> Self {
        Self {
            cache,
            selection,
            generation,
            active: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PromptCacheConversationFlightGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }

        let cache = self.cache.clone();
        let selection = self.selection;
        let generation = self.generation;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut state = cache.lock().await;
                if let Some(in_flight) = state.in_flight.remove(&selection) {
                    if in_flight.generation != generation {
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
            if in_flight.generation != generation {
                state.in_flight.insert(selection, in_flight);
                return;
            }
            let _ = in_flight.signal.send(true);
        }
    }
}

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

pub(crate) async fn invalidate_prompt_cache_conversations_cache(
    cache: &Arc<Mutex<PromptCacheConversationsCacheState>>,
) {
    let in_flight = {
        let mut state = cache.lock().await;
        state.generation = state.generation.wrapping_add(1);
        state.entries.clear();
        std::mem::take(&mut state.in_flight)
    };

    for flight in in_flight.into_values() {
        let _ = flight.signal.send(true);
    }
}
