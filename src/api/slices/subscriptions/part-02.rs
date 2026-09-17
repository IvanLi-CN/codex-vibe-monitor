#[derive(Debug, Default)]
struct SummaryDeltaJournal {
    // The immutable Projection has already proven every sequence through this watermark. The
    // next journal tail must begin immediately after it, even when the resident queue is empty.
    base_cursor: SummaryDeltaCursor,
    cursor: SummaryDeltaCursor,
    pending: BTreeMap<u64, DashboardActivityTerminalDelta>,
    pending_bytes: usize,
    rolled_back_sequences: BTreeSet<u64>,
    entries: VecDeque<SummaryDeltaEntry>,
    bytes: usize,
    // A terminal-journal replay can commit after Bootstrap has published. Its source row is
    // exact, but it has no in-process dashboard sequence to join the contiguous live journal.
    // Keep it in the same bounded resident budget and layer it into rolling reads separately.
    replayed_entries: VecDeque<DashboardActivityTerminalDelta>,
    replayed_bytes: usize,
    last_reconciled_terminal_watermark: u64,
    reconciliation_completed: bool,
    source_compaction_gap: bool,
    overflowed_through_sequence: Option<u64>,
    gap_proofs: VecDeque<DeltaGapProof>,
    gap_proof_budget_exhausted: bool,
}

impl SummaryDeltaJournal {
    fn contains_same_identity(
        left: &DashboardActivityTerminalDelta,
        right: &DashboardActivityTerminalDelta,
    ) -> bool {
        let (Some(left_row_id), Some(right_row_id)) =
            (left.persisted_row_id, right.persisted_row_id)
        else {
            return false;
        };
        left_row_id == right_row_id
            && left.invoke_id == right.invoke_id
            && left.occurred_at == right.occurred_at
    }

    fn contains_conflicting_row_identity(&self, delta: &DashboardActivityTerminalDelta) -> bool {
        let Some(row_id) = delta.persisted_row_id else {
            return false;
        };
        self.pending
            .values()
            .chain(self.entries.iter().map(|entry| &entry.delta))
            .chain(self.replayed_entries.iter())
            .any(|existing| {
                existing.persisted_row_id == Some(row_id)
                    && !Self::contains_same_identity(existing, delta)
            })
    }

    fn compatible_pending_identity(
        pending: &DashboardActivityTerminalDelta,
        acknowledged: &DashboardActivityTerminalDelta,
    ) -> bool {
        pending.invoke_id == acknowledged.invoke_id
            && pending.occurred_at == acknowledged.occurred_at
            && (pending.persisted_row_id.is_none()
                || acknowledged.persisted_row_id.is_none()
                || pending.persisted_row_id == acknowledged.persisted_row_id)
    }

    fn absorb_committed_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
        if let Some(pending) = self.pending.remove(&delta.terminal_sequence) {
            self.pending_bytes = self.pending_bytes.saturating_sub(pending.estimated_bytes);
        }
        self.entries
            .retain(|entry| !Self::contains_same_identity(&entry.delta, delta));
        self.replayed_entries
            .retain(|entry| !Self::contains_same_identity(entry, delta));
        self.bytes = self
            .entries
            .iter()
            .map(|entry| entry.delta.estimated_bytes)
            .sum();
        self.replayed_bytes = self
            .replayed_entries
            .iter()
            .map(|entry| entry.estimated_bytes)
            .sum();
        self.cursor = self.cursor.max(SummaryDeltaCursor(delta.terminal_sequence));
        self.base_cursor = self
            .base_cursor
            .max(SummaryDeltaCursor(delta.terminal_sequence));
        self.last_reconciled_terminal_watermark = self
            .last_reconciled_terminal_watermark
            .max(delta.terminal_sequence);
    }

    // Registration happens before the asynchronous SQLite enqueue. Pending entries establish
    // the expected sequence and retain a bounded recovery proof, but they are never exposed to
    // Summary reads until the writer calls `append` after commit.
    fn register_pending(&mut self, delta: DashboardActivityTerminalDelta) -> bool {
        let sequence = delta.terminal_sequence;
        if self.overflowed_through_sequence.is_some() {
            self.note_gap(&delta);
            return false;
        }
        if let Some(existing) = self.pending.get(&sequence) {
            if Self::contains_same_identity(existing, &delta) {
                return true;
            }
            self.overflowed_through_sequence = Some(sequence);
            self.note_unknown_terminal_gap(sequence);
            self.note_gap(&delta);
            return false;
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.cursor == SummaryDeltaCursor(sequence))
            || sequence <= self.cursor.max(self.base_cursor).0
        {
            self.overflowed_through_sequence = Some(sequence);
            self.note_unknown_terminal_gap(sequence);
            self.note_gap(&delta);
            return false;
        }
        let exceeds_count = self
            .entries
            .len()
            .saturating_add(self.replayed_entries.len())
            .saturating_add(self.pending.len())
            >= SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS;
        let exceeds_bytes = self
            .bytes
            .saturating_add(self.replayed_bytes)
            .saturating_add(self.pending_bytes)
            .saturating_add(delta.estimated_bytes)
            > SUMMARY_TERMINAL_OVERLAY_MAX_BYTES;
        if exceeds_count || exceeds_bytes {
            self.overflowed_through_sequence = Some(sequence);
            self.note_gap(&delta);
            return false;
        }
        self.pending_bytes = self.pending_bytes.saturating_add(delta.estimated_bytes);
        self.pending.insert(sequence, delta);
        true
    }

    fn rollback_pending(&mut self, sequence: Option<u64>) {
        let Some(sequence) = sequence else {
            return;
        };
        if let Some(delta) = self.pending.remove(&sequence) {
            self.pending_bytes = self.pending_bytes.saturating_sub(delta.estimated_bytes);
        }
        let mut known_through = self.cursor.max(self.base_cursor).0;
        if sequence <= known_through {
            return;
        }
        self.rolled_back_sequences.insert(sequence);
        while self
            .rolled_back_sequences
            .remove(&known_through.saturating_add(1))
        {
            known_through = known_through.saturating_add(1);
        }
        self.cursor = self.cursor.max(SummaryDeltaCursor(known_through));
    }

    fn commit_skipped_sequences(&mut self) {
        let mut known_through = self.cursor.max(self.base_cursor).0;
        while self
            .rolled_back_sequences
            .remove(&known_through.saturating_add(1))
        {
            known_through = known_through.saturating_add(1);
        }
        self.cursor = self.cursor.max(SummaryDeltaCursor(known_through));
    }

    fn acknowledge_pending(&mut self, delta: DashboardActivityTerminalDelta) -> bool {
        if let Some(pending) = self.pending.remove(&delta.terminal_sequence) {
            self.pending_bytes = self.pending_bytes.saturating_sub(pending.estimated_bytes);
            if !Self::compatible_pending_identity(&pending, &delta)
                || delta.persisted_row_id.is_none()
            {
                if delta.persisted_row_id.is_some() {
                    self.note_gap(&delta);
                } else {
                    self.overflowed_through_sequence = Some(delta.terminal_sequence);
                    self.note_unknown_terminal_gap(delta.terminal_sequence);
                }
                return false;
            }
        }
        self.commit_skipped_sequences();
        self.append(delta.clone(), delta.terminal_sequence)
    }

    fn note_unknown_terminal_gap(&mut self, cursor: u64) {
        self.cursor = self.cursor.max(SummaryDeltaCursor(cursor));
        self.retain_gap_proof(DeltaGapProof {
            cursor: SummaryDeltaCursor(cursor),
            terminal_sequence: Some(cursor),
            upstream_account_id: None,
            occurred_at: String::new(),
            row_id: None,
            invoke_id: None,
        });
    }

    fn note_unknown_source_cursor_gap(&mut self, cursor: u64) {
        // A source-journal compaction cursor has no terminal sequence and remains broad until the
        // source tail is rebuilt.
        self.retain_gap_proof(DeltaGapProof {
            cursor: SummaryDeltaCursor(cursor),
            terminal_sequence: None,
            upstream_account_id: None,
            occurred_at: String::new(),
            row_id: None,
            invoke_id: None,
        });
        self.source_compaction_gap = true;
    }

    fn clear_source_compaction_gap(&mut self) {
        if !self.source_compaction_gap || self.gap_proof_budget_exhausted {
            return;
        }
        self.gap_proofs
            .retain(|proof| proof.terminal_sequence.is_some());
        self.source_compaction_gap = false;
        self.gap_proof_budget_exhausted = false;
    }

    fn note_gap(&mut self, delta: &DashboardActivityTerminalDelta) {
        self.cursor = self.cursor.max(SummaryDeltaCursor(delta.terminal_sequence));
        self.retain_gap_proof(DeltaGapProof {
            cursor: SummaryDeltaCursor(delta.terminal_sequence),
            terminal_sequence: Some(delta.terminal_sequence),
            upstream_account_id: delta.upstream_account_id,
            occurred_at: delta.occurred_at.clone(),
            row_id: delta.persisted_row_id,
            invoke_id: delta.persisted_row_id.map(|_| delta.invoke_id.clone()),
        });
    }

    fn retain_gap_proof(&mut self, proof: DeltaGapProof) {
        if self.gap_proof_budget_exhausted {
            return;
        }
        if self.gap_proofs.len() >= SUMMARY_DELTA_JOURNAL_MAX_GAP_PROOFS {
            // Dropping the oldest scoped proof could make an old account/range look exact.
            // Keep one irreversible broad proof until a durable projection consumes the gap.
            self.gap_proofs.clear();
            self.gap_proofs.push_back(DeltaGapProof {
                cursor: proof.cursor,
                terminal_sequence: None,
                upstream_account_id: None,
                occurred_at: String::new(),
                row_id: None,
                invoke_id: None,
            });
            self.gap_proof_budget_exhausted = true;
            return;
        }
        self.gap_proofs.push_back(proof);
    }

    fn append_replayed(&mut self, delta: DashboardActivityTerminalDelta) -> bool {
        if delta.persisted_row_id.is_none() {
            self.note_unknown_terminal_gap(delta.terminal_sequence);
            return false;
        }
        if self.contains_conflicting_row_identity(&delta) {
            self.note_gap(&delta);
            return false;
        }
        if self
            .entries
            .iter()
            .map(|entry| &entry.delta)
            .chain(self.replayed_entries.iter())
            .any(|entry| Self::contains_same_identity(entry, &delta))
        {
            return true;
        }
        if self.overflowed_through_sequence.is_some() {
            self.note_gap(&delta);
            return false;
        }
        let exceeds_count = self
            .entries
            .len()
            .saturating_add(self.replayed_entries.len())
            .saturating_add(self.pending.len())
            >= SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS;
        let exceeds_bytes = self
            .bytes
            .saturating_add(self.replayed_bytes)
            .saturating_add(self.pending_bytes)
            .saturating_add(delta.estimated_bytes)
            > SUMMARY_TERMINAL_OVERLAY_MAX_BYTES;
        if exceeds_count || exceeds_bytes {
            self.overflowed_through_sequence = Some(delta.terminal_sequence);
            self.note_gap(&delta);
            return false;
        }
        self.replayed_bytes = self.replayed_bytes.saturating_add(delta.estimated_bytes);
        self.replayed_entries.push_back(delta);
        true
    }

    // Entries arrive from the terminal slice only after its SQLite transaction has committed.
    // Only a repeat of the same immutable terminal identity is harmless. An old or conflicting
    // sequence is a fail-closed gap because it could otherwise replace an exact terminal without
    // a rebuild.
    fn append(&mut self, delta: DashboardActivityTerminalDelta, slice_high_watermark: u64) -> bool {
        if delta.persisted_row_id.is_none() {
            self.overflowed_through_sequence = Some(slice_high_watermark);
            self.note_unknown_terminal_gap(slice_high_watermark);
            return false;
        }
        if self.contains_conflicting_row_identity(&delta) {
            let expected_next = self.cursor.max(self.base_cursor).0.saturating_add(1);
            if delta.terminal_sequence != expected_next || slice_high_watermark != expected_next {
                self.overflowed_through_sequence = Some(slice_high_watermark);
                self.note_unknown_terminal_gap(slice_high_watermark);
            }
            self.note_gap(&delta);
            return false;
        }
        if let Some(existing) = self
            .entries
            .iter()
            .find(|entry| entry.cursor == SummaryDeltaCursor(delta.terminal_sequence))
        {
            if Self::contains_same_identity(&existing.delta, &delta) {
                return true;
            }
            self.overflowed_through_sequence = Some(slice_high_watermark);
            self.note_unknown_terminal_gap(slice_high_watermark);
            self.note_gap(&delta);
            return false;
        }
        if self.overflowed_through_sequence.is_some() {
            self.overflowed_through_sequence = Some(
                self.overflowed_through_sequence
                    .unwrap_or_default()
                    .max(slice_high_watermark),
            );
            self.note_gap(&delta);
            return false;
        }
        // A replayed durable identity may be ACKed later by the normal terminal writer. Promote
        // that exact row into the ordered journal instead of retaining two copies.
        if self
            .replayed_entries
            .iter()
            .any(|entry| Self::contains_same_identity(entry, &delta))
        {
            self.replayed_entries
                .retain(|entry| !Self::contains_same_identity(entry, &delta));
            self.replayed_bytes = self
                .replayed_entries
                .iter()
                .map(|entry| entry.estimated_bytes)
                .sum();
        }
        let known_through = self.cursor.max(self.base_cursor);
        if delta.terminal_sequence <= known_through.0 {
            self.overflowed_through_sequence = Some(slice_high_watermark);
            self.note_unknown_terminal_gap(slice_high_watermark);
            self.note_gap(&delta);
            return false;
        }
        let expected_next = known_through.0.saturating_add(1);
        if delta.terminal_sequence != expected_next {
            self.overflowed_through_sequence = Some(slice_high_watermark);
            self.note_unknown_terminal_gap(slice_high_watermark);
            self.note_gap(&delta);
            return false;
        }
        let exceeds_count = self
            .entries
            .len()
            .saturating_add(self.replayed_entries.len())
            >= SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS;
        let exceeds_bytes = self
            .bytes
            .saturating_add(self.replayed_bytes)
            .saturating_add(delta.estimated_bytes)
            > SUMMARY_TERMINAL_OVERLAY_MAX_BYTES;
        if exceeds_count || exceeds_bytes {
            self.overflowed_through_sequence = Some(slice_high_watermark);
            self.note_gap(&delta);
            return false;
        }
        self.bytes = self.bytes.saturating_add(delta.estimated_bytes);
        self.cursor = SummaryDeltaCursor(delta.terminal_sequence);
        self.entries.push_back(SummaryDeltaEntry {
            cursor: SummaryDeltaCursor(delta.terminal_sequence),
            delta,
        });
        true
    }

    fn retain_replayed_for_reconciliation(
        &mut self,
        deltas: &[DashboardActivityTerminalDelta],
        projection: &SummaryProjection,
        target_terminal_watermark: u64,
        source_tail_complete: bool,
    ) -> bool {
        for delta in deltas {
            if delta.persisted_row_id.is_some_and(|row_id| {
                projection.contains_persisted_live_terminal_identity(
                    row_id,
                    &delta.invoke_id,
                    &delta.occurred_at,
                )
            }) {
                continue;
            }
            if self
                .entries
                .iter()
                .map(|entry| &entry.delta)
                .chain(self.replayed_entries.iter())
                .any(|entry| Self::contains_same_identity(entry, delta))
            {
                continue;
            }
            let exceeds_count = self
                .entries
                .len()
                .saturating_add(self.replayed_entries.len())
                .saturating_add(self.pending.len())
                >= SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS;
            let exceeds_bytes = self
                .bytes
                .saturating_add(self.replayed_bytes)
                .saturating_add(self.pending_bytes)
                .saturating_add(delta.estimated_bytes)
                > SUMMARY_TERMINAL_OVERLAY_MAX_BYTES;
            if exceeds_count || exceeds_bytes {
                self.overflowed_through_sequence = Some(
                    self.overflowed_through_sequence
                        .unwrap_or_default()
                        .max(delta.terminal_sequence),
                );
                self.note_gap(delta);
                continue;
            }
            self.replayed_bytes = self.replayed_bytes.saturating_add(delta.estimated_bytes);
            self.replayed_entries.push_back(delta.clone());
        }

        if !source_tail_complete {
            return false;
        }
        let target_covers_overflow = self
            .overflowed_through_sequence
            .is_none_or(|overflowed| target_terminal_watermark >= overflowed);
        if !target_covers_overflow {
            return false;
        }
        let replayed_identities = self
            .replayed_entries
            .iter()
            .filter_map(SummarySourceIdentity::from_delta)
            .collect::<HashSet<_>>();
        self.gap_proofs.retain(|proof| {
            if proof.row_id.is_none() {
                // No durable identity means the reconciliation cannot prove which source row
                // was missing. Keep the broad fail-closed marker until a projection rebuild with
                // authoritative identities absorbs it; a terminal watermark alone is not proof.
                return true;
            }
            let covered_by_projection = proof.invoke_id.as_deref().is_some_and(|invoke_id| {
                projection.contains_persisted_live_terminal_identity(
                    proof.row_id.unwrap_or(i64::MIN),
                    invoke_id,
                    &proof.occurred_at,
                )
            });
            !(covered_by_projection
                || replayed_identities.iter().any(|identity| {
                    Some(identity.row_id) == proof.row_id
                        && proof.invoke_id.as_deref() == Some(identity.invoke_id.as_str())
                        && proof.occurred_at == identity.occurred_at
                }))
        });
        if self.gap_proofs.is_empty() {
            self.overflowed_through_sequence = None;
            self.gap_proof_budget_exhausted = false;
            self.last_reconciled_terminal_watermark = self
                .last_reconciled_terminal_watermark
                .max(target_terminal_watermark);
            self.reconciliation_completed = true;
            true
        } else {
            false
        }
    }
}

fn record_all_time_account_overflow_marker(
    state: &mut SubscriptionHubState,
    account_id: i64,
    terminal_sequence: u64,
) {
    if let Some(existing) = state
        .summary_terminal_overlay_all_time_overflowed_through_account
        .get_mut(&account_id)
    {
        *existing = (*existing).max(terminal_sequence);
        return;
    }
    if state
        .summary_terminal_overlay_all_time_overflowed_through_account
        .len()
        < SUMMARY_TERMINAL_OVERLAY_MAX_ACCOUNT_OVERFLOW_MARKERS
    {
        state
            .summary_terminal_overlay_all_time_overflowed_through_account
            .insert(account_id, terminal_sequence);
        return;
    }
    state
        .summary_terminal_overlay_all_time_overflowed_through_unknown_account
        .get_or_insert(terminal_sequence);
    if let Some(existing) = state
        .summary_terminal_overlay_all_time_overflowed_through_unknown_account
        .as_mut()
    {
        *existing = (*existing).max(terminal_sequence);
    }
}

// Both the rolling journal and the all-time overlay are derived only from a committed terminal
// ACK. Keeping this admission here prevents a Dashboard pre-commit slice from making a Summary
// response observe a transaction that can still roll back.
fn append_summary_all_time_delta(
    state: &mut SubscriptionHubState,
    delta: &DashboardActivityTerminalDelta,
    slice_high_watermark: u64,
) {
    if let Some(overflowed_through) = state
        .summary_terminal_overlay_all_time_overflowed_through_sequence
        .as_mut()
    {
        *overflowed_through = (*overflowed_through).max(slice_high_watermark);
    }
    let identity_already_present = delta.persisted_row_id.is_some_and(|row_id| {
        state
            .summary_terminal_overlay_all_time
            .iter()
            .any(|existing| {
                existing.persisted_row_id == Some(row_id)
                    && existing.invoke_id == delta.invoke_id
                    && existing.occurred_at == delta.occurred_at
            })
    });
    let sequence_already_present = delta.terminal_sequence != 0
        && state
            .summary_terminal_overlay_all_time
            .iter()
            .any(|existing| existing.terminal_sequence == delta.terminal_sequence);
    if state
        .summary_terminal_overlay_all_time_overflowed_through_sequence
        .is_none()
        && !identity_already_present
        && !sequence_already_present
    {
        let exceeds_count =
            state.summary_terminal_overlay_all_time.len() >= SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS;
        let exceeds_bytes = state
            .summary_terminal_overlay_all_time_bytes
            .saturating_add(delta.estimated_bytes)
            > SUMMARY_TERMINAL_OVERLAY_MAX_BYTES;
        if exceeds_count || exceeds_bytes {
            state.summary_terminal_overlay_all_time_overflowed_through_sequence =
                Some(slice_high_watermark);
            if let Some(account_id) = delta.upstream_account_id {
                record_all_time_account_overflow_marker(state, account_id, delta.terminal_sequence);
            }
            tracing::warn!(
                pending_terminal_count = state.summary_terminal_overlay_all_time.len(),
                pending_terminal_bytes = state.summary_terminal_overlay_all_time_bytes,
                overflowed_through = slice_high_watermark,
                "all-time summary terminal overlay reached its bounded memory budget"
            );
        } else {
            state.summary_terminal_overlay_all_time_bytes = state
                .summary_terminal_overlay_all_time_bytes
                .saturating_add(delta.estimated_bytes);
            state
                .summary_terminal_overlay_all_time
                .push_back(delta.clone());
        }
    } else if state
        .summary_terminal_overlay_all_time_overflowed_through_sequence
        .is_some()
        && let Some(account_id) = delta.upstream_account_id
    {
        record_all_time_account_overflow_marker(state, account_id, delta.terminal_sequence);
    }
}

#[derive(Debug, Clone)]
struct CachedSubscriptionTopic {
    topic: SubscriptionTopic,
    descriptor: SubscriptionTopicDescriptor,
    schema_epoch: String,
    cursor: u64,
    snapshot_built_at: Instant,
    refresh_scheduled: bool,
    conversation_overview_refresh_scheduled: bool,
    conversation_overview_refresh_in_flight: bool,
    conversation_overview_refresh_pending: bool,
    upstream_account_attempt_refresh_scheduled: bool,
    upstream_account_attempt_refresh_in_flight: bool,
    upstream_account_attempt_refresh_pending: bool,
    upstream_account_attempt_refresh_generation: u64,
    dirty: bool,
    runtime_topic_recovery_generation: u64,
    runtime_topic_recovery_retry_at: Option<Instant>,
    summary_refresh_scheduled: bool,
    summary_refresh_in_flight: bool,
    summary_pending_event_count: u64,
    summary_retry_backoff_ms: u64,
    parallel_work_refresh_scheduled: bool,
    prompt_cache_refresh_scheduled: bool,
    prompt_cache_reconcile_scheduled: bool,
    prompt_cache_key_hydration_scheduled: bool,
    prompt_cache_pending_records: BTreeMap<String, PromptCacheTopicDelta>,
    prompt_cache_pending_key_hydrations: BTreeSet<String>,
    prompt_cache_candidate_refill_required: bool,
    prompt_cache_applied_terminal_ids: HashSet<String>,
    prompt_cache_coalesced_event_count: u64,
    prompt_cache_full_hydration_count: u64,
    prompt_cache_bounded_key_hydration_count: u64,
    prompt_cache_baseline_at: Option<Instant>,
    prompt_cache_baseline_row_id: i64,
    prompt_cache_response_source: &'static str,
    prompt_cache_reconcile_required: bool,
    prompt_cache_pressure_deferred: bool,
    latest_live_snapshot: Option<DashboardActivityLiveSnapshot>,
    calendar_anchor: Option<String>,
    continuity_reset_cursor: Option<u64>,
    dashboard_materializer: Option<DashboardTopicMaterializer>,
    dashboard_base_revision: u64,
    dashboard_materialized_revision: Option<DashboardTopicRevision>,
    snapshot_payload: Value,
    snapshot_frame: Arc<SerializedTopicFrame>,
    snapshot_bytes: usize,
    replay_events: VecDeque<ReplayableTopicEvent>,
    replay_bytes: usize,
}

impl CachedSubscriptionTopic {
    fn invalidate_upstream_account_attempt_refresh(&mut self) {
        self.upstream_account_attempt_refresh_generation = self
            .upstream_account_attempt_refresh_generation
            .saturating_add(1);
        self.upstream_account_attempt_refresh_scheduled = false;
        self.upstream_account_attempt_refresh_in_flight = false;
        self.upstream_account_attempt_refresh_pending = false;
    }
}

#[derive(Debug, Clone)]
struct RuntimeTopicWork {
    topic: SubscriptionTopic,
    terminal_event_count: u64,
    includes_invocation_mutation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RuntimeTopicDependency {
    Invocation,
    ModelRouting,
    PromptCacheProjection,
    PromptCacheWindow,
    PromptCacheStickyWindow,
    DashboardWorkingConversationsProjection,
    Attempt(String),
    Binding(String),
    HistoryPromptCacheKey(String),
    HistoryStickyKey(String),
    StickyRoute(String),
}

#[derive(Debug, Clone, PartialEq)]
struct PromptCacheTopicDelta {
    row_id: i64,
    identity: String,
    invoke_id: String,
    prompt_cache_key: Option<String>,
    sticky_key: Option<String>,
    occurred_at: String,
    is_runtime_removed: bool,
    status: String,
    is_terminal: bool,
    is_success: bool,
    request_tokens: i64,
    cost: f64,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    preview: Option<PromptCacheConversationInvocationPreviewResponse>,
}

fn prompt_cache_hydration_changed_pending_keys(
    pending_at_hydration_start: &BTreeMap<String, PromptCacheTopicDelta>,
    pending_after_hydration: &BTreeMap<String, PromptCacheTopicDelta>,
    hydration_keys: &[String],
) -> BTreeSet<String> {
    pending_after_hydration
        .iter()
        .filter_map(|(identity, record)| {
            let key = record.prompt_cache_key.as_deref()?;
            (hydration_keys.iter().any(|candidate| candidate == key)
                && pending_at_hydration_start.get(identity) != Some(record))
            .then(|| key.to_string())
        })
        .collect()
}

struct PromptCacheBaselineBuild {
    baseline_row_id: i64,
    persisted_identities: HashSet<String>,
    runtime_overlay_terminal_identities: HashSet<String>,
}

struct ParallelWorkBaselineBuild {
    persisted_identities: HashSet<String>,
}

impl PromptCacheTopicDelta {
    #[cfg(test)]
    fn from_record(record: &ApiInvocation) -> Result<Option<Self>, ApiError> {
        Ok(PromptCacheRuntimeProjection::from_record(record)
            .as_ref()
            .and_then(Self::from_runtime_projection))
    }

    fn from_runtime_mutation(
        mutation: &RuntimeInvocationMutation,
        runtime_projection: Option<&PromptCacheRuntimeProjection>,
    ) -> Result<Option<Self>, ApiError> {
        if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
            return Ok(Self::from_runtime_removal(mutation));
        }
        Ok(runtime_projection.and_then(Self::from_runtime_projection))
    }

    fn from_runtime_projection(projection: &PromptCacheRuntimeProjection) -> Option<Self> {
        let preview = projection.preview.clone();
        let status = preview.status.clone();
        Some(Self {
            row_id: projection.row_id,
            identity: format!("{}\0{}", preview.invoke_id, preview.occurred_at),
            invoke_id: preview.invoke_id.clone(),
            prompt_cache_key: projection.prompt_cache_key.clone(),
            sticky_key: projection.sticky_key.clone(),
            occurred_at: parse_to_utc_datetime(&preview.occurred_at)
                .map(format_utc_iso)
                .unwrap_or_else(|| preview.occurred_at.clone()),
            is_runtime_removed: false,
            is_terminal: prompt_invocation_status_counts_toward_terminal_totals(Some(&status)),
            is_success: prompt_invocation_status_is_success_like(
                Some(&status),
                preview.error_message.as_deref(),
            ),
            status,
            request_tokens: preview.total_tokens.max(0),
            cost: preview.cost.unwrap_or_default(),
            upstream_account_id: preview.upstream_account_id,
            upstream_account_name: preview.upstream_account_name.clone(),
            preview: Some(preview),
        })
    }

    fn from_runtime_removal(mutation: &RuntimeInvocationMutation) -> Option<Self> {
        if mutation.prompt_cache_key.is_none() && mutation.sticky_key.is_none() {
            return None;
        }
        let occurred_at = parse_to_utc_datetime(&mutation.identity.occurred_at)
            .map(format_utc_iso)
            .unwrap_or_else(|| mutation.identity.occurred_at.clone());
        Some(Self {
            row_id: mutation.row_id.unwrap_or_default(),
            identity: format!(
                "{}\0{}",
                mutation.identity.invoke_id, mutation.identity.occurred_at
            ),
            invoke_id: mutation.identity.invoke_id.clone(),
            prompt_cache_key: mutation.prompt_cache_key.clone(),
            sticky_key: mutation.sticky_key.clone(),
            occurred_at,
            is_runtime_removed: true,
            status: "unknown".to_string(),
            is_terminal: mutation.is_terminal,
            is_success: false,
            request_tokens: 0,
            cost: 0.0,
            upstream_account_id: mutation.upstream_account_id,
            upstream_account_name: None,
            preview: None,
        })
    }
}

#[derive(Debug, Clone)]
struct ReplayableTopicEvent {
    frame: Arc<SerializedTopicFrame>,
    bytes: usize,
    emitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DashboardTopicRevision {
    base_revision: u64,
    current_revision: Option<u64>,
    network_revision: Option<u64>,
    terminal_revision: Option<u64>,
    routing_revision: u64,
}

#[derive(Debug)]
struct DashboardActivityMaterializerState {
    base: DashboardActivityTopicMaterializedBase,
    rebase_range_start: Option<DateTime<Utc>>,
    current_revision: Option<u64>,
    network_revision: Option<u64>,
    terminal_revision: Option<u64>,
    routing_revision: u64,
}

impl DashboardActivityMaterializerState {
    fn new(base: DashboardActivityTopicMaterializedBase) -> Self {
        let rebase_range_start = parse_to_utc_datetime(&base.response().range_start);
        Self {
            base,
            rebase_range_start,
            current_revision: None,
            network_revision: None,
            terminal_revision: None,
            routing_revision: 0,
        }
    }
}
