use super::*;

#[cfg(test)]
pub(crate) type DashboardTopologyFramesByTopic = BTreeMap<String, Vec<Arc<SerializedTopicFrame>>>;
#[cfg(test)]
pub(crate) type DashboardTopologySseFrameObservations =
    HashMap<u64, DashboardTopologyFramesByTopic>;

#[derive(Debug)]
pub(crate) struct SubscriptionHub {
    pub(crate) state: Mutex<SubscriptionHubState>,
    // Summary projection hydration is single-flight per service instance. Keeping this with the
    // hub avoids suppressing bootstrap for an independent AppState (including test fixtures).
    pub(crate) summary_projection_refresh: tokio::sync::Mutex<()>,
    // Historical coverage recovery has independent lifetime and database admission from the
    // rolling Projection refresh. It still needs one owner so a cadence tick cannot duplicate a
    // page already being reduced by the recovery worker.
    pub(crate) summary_coverage_recovery: tokio::sync::Mutex<()>,
    // Prevent concurrent catalog/workload owners from duplicating an initial SQLite build.
    pub(crate) managed_task_refresh_serial: tokio::sync::Mutex<()>,
    pub(crate) broadcaster: broadcast::Sender<SubscriptionDispatchEvent>,
    pub(crate) runtime_mutation_bus: Arc<RuntimeMutationBus>,
    pub(crate) runtime_topic_recovery_notify: Arc<Notify>,
    pub(crate) internal_broadcast_listener_count: AtomicUsize,
    pub(crate) serialization_count: AtomicU64,
    pub(crate) dashboard_topology_counters: DashboardDeliveryTopologyCounters,
    #[cfg(test)]
    pub(crate) dashboard_topology_sse_frame_observations:
        Mutex<DashboardTopologySseFrameObservations>,
}

#[derive(Debug, Default)]
pub(crate) struct SubscriptionHubState {
    pub(crate) topics: HashMap<String, CachedSubscriptionTopic>,
    pub(crate) active_topics: HashMap<String, SubscriptionTopic>,
    pub(crate) active_topic_dependencies: HashMap<RuntimeTopicDependency, HashSet<String>>,
    pub(crate) active_subscribers: HashMap<String, usize>,
    pub(crate) active_topic_names: HashMap<String, usize>,
    pub(crate) dashboard_live_subscriber_count: usize,
    pub(crate) server_push_subscribers: HashMap<String, usize>,
    pub(crate) server_push_tasks: HashSet<String>,
    pub(crate) dashboard_current_slice: Option<Arc<DashboardCurrentProjectionSlice>>,
    pub(crate) dashboard_network_slice: Option<Arc<DashboardNetworkProjectionSlice>>,
    pub(crate) dashboard_terminal_slice: Option<Arc<DashboardTerminalProjectionSlice>>,
    pub(crate) summary_snapshots: HashMap<SummarySnapshotKey, SummarySnapshotEntry>,
    pub(crate) summary_projection: Option<Arc<SummaryProjection>>,
    // Compacted-source recovery publishes a bounded, day-boundary-only snapshot separately from
    // the general projection so 30d/all-time coverage keeps its existing fail-closed boundary.
    pub(crate) summary_dashboard_recovery: Option<Arc<SummaryDashboardRecoverySnapshot>>,
    pub(crate) summary_compacted_source_recovery_state: SummaryCompactedSourceRecoveryState,
    // Terminal slices are ephemeral delivery batches. The Summary-owned journal only receives
    // entries after the writer's SQLite commit ACK, so it is a bounded exact delta between an
    // immutable base projection and the latest rolling view.
    pub(crate) summary_delta_journal: SummaryDeltaJournal,
    // SQLite source descriptors are consumed once per process lifetime. A restart starts at
    // zero and reconstructs the bounded durable tail before any RollingDelta refresh.
    pub(crate) summary_source_change_cursor: u64,
    pub(crate) summary_live_tail_readiness: SummaryLiveTailReadiness,
    // A cold Bootstrap fallback records the durable source fence which made its historical
    // admission unavailable. This is process-local retry suppression; coverage recovery still
    // owns the durable proof. Changed coverage reopens Bootstrap; live changes use the tail.
    pub(crate) summary_bootstrap_fallback_fence: Option<SummaryProjectionGenerationFence>,
    // All-time coverage can lag independently of rolling coverage. Keep its replay budget
    // separate so an all-time archive gap cannot make healthy rolling topics unavailable.
    pub(crate) summary_terminal_overlay_all_time: VecDeque<DashboardActivityTerminalDelta>,
    pub(crate) summary_terminal_overlay_all_time_bytes: usize,
    // A shared all-time queue can overflow on one account while another account remains
    // serviceable. Retain the dropped-through proof per account so a global rebuild cannot
    // silently clear an account's fail-closed marker.
    pub(crate) summary_terminal_overlay_all_time_overflowed_through_account: HashMap<i64, u64>,
    // Once the account marker cap is reached, retain one bounded proof for all further account
    // keys. Account reads remain fail-closed unless their own projection proves this watermark.
    pub(crate) summary_terminal_overlay_all_time_overflowed_through_unknown_account: Option<u64>,
    pub(crate) summary_terminal_overlay_all_time_overflowed_through_sequence: Option<u64>,
    pub(crate) summary_http_interest_at: Option<Instant>,
    pub(crate) summary_http_all_time_interest_at: Option<Instant>,
    pub(crate) summary_projection_revision: u64,
    pub(crate) prompt_cache_prebaseline_records:
        HashMap<String, BTreeMap<String, PromptCacheTopicDelta>>,
    pub(crate) prompt_cache_prebaseline_key_hydrations: HashMap<String, BTreeSet<String>>,
    pub(crate) parallel_work_prebaseline_mutations:
        HashMap<String, BTreeMap<String, RuntimeInvocationMutation>>,
    pub(crate) runtime_topic_recovery_generation: u64,
    pub(crate) runtime_topic_recovery_queue: VecDeque<(String, u64)>,
    pub(crate) runtime_topic_recovery_queued: HashSet<String>,
    pub(crate) runtime_topic_recovery_running: bool,
    pub(crate) managed_task_refresh_generations: HashMap<String, u64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SummaryLiveTailReadiness {
    pub(crate) reason: String,
    pub(crate) stage: String,
    pub(crate) gap_count: u64,
    pub(crate) watermark: u64,
    pub(crate) epoch: u64,
    pub(crate) elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct SummaryDeltaCursor(pub(crate) u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SummarySourceIdentity {
    pub(crate) row_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

impl SummarySourceIdentity {
    pub(crate) fn from_delta(delta: &DashboardActivityTerminalDelta) -> Option<Self> {
        delta.persisted_row_id.map(|row_id| Self {
            row_id,
            invoke_id: delta.invoke_id.clone(),
            occurred_at: delta.occurred_at.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SummaryDeltaEntry {
    pub(crate) cursor: SummaryDeltaCursor,
    pub(crate) delta: DashboardActivityTerminalDelta,
}

// A dropped, conflicting, or out-of-order journal entry never permits a stale global response.
// The reconciliation path consumes this proof to restrict only selections that could include it.
#[derive(Debug, Clone)]
pub(crate) struct DeltaGapProof {
    pub(crate) cursor: SummaryDeltaCursor,
    // A journal cursor and a terminal sequence are different domains. `None` means this proof
    // is source-scoped rather than tied to a terminal watermark.
    pub(crate) terminal_sequence: Option<u64>,
    // A broad proof intentionally clears its concrete terminal sequence. Preserve whether that
    // broad proof came from source-journal loss so terminal-only overflow can remain degraded.
    pub(crate) source_gap: bool,
    // Only the proof emitted by source-journal compaction may be retired by compaction cleanup.
    // Scoped source proofs must survive a concurrent cleanup attempt.
    pub(crate) compaction_gap: bool,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) occurred_at: String,
    pub(crate) row_id: Option<i64>,
    pub(crate) invoke_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SummaryRollingDeltaSnapshot {
    pub(crate) projection: Arc<SummaryProjection>,
    pub(crate) entries: Vec<DashboardActivityTerminalDelta>,
    pub(crate) gaps: Vec<DeltaGapProof>,
}

#[derive(Debug, Default)]
pub(crate) struct SummaryDeltaJournal {
    // The immutable Projection has already proven every sequence through this watermark. The
    // next journal tail must begin immediately after it, even when the resident queue is empty.
    pub(crate) base_cursor: SummaryDeltaCursor,
    pub(crate) cursor: SummaryDeltaCursor,
    pub(crate) pending: BTreeMap<u64, DashboardActivityTerminalDelta>,
    pub(crate) pending_bytes: usize,
    pub(crate) rolled_back_sequences: BTreeSet<u64>,
    // A committed terminal can arrive before its predecessor when independent SQLite batches
    // finish out of order. Keep it bounded and invisible to Summary reads until the prefix closes.
    pub(crate) committed_reorder: BTreeMap<u64, DashboardActivityTerminalDelta>,
    pub(crate) committed_reorder_bytes: usize,
    pub(crate) entries: VecDeque<SummaryDeltaEntry>,
    pub(crate) bytes: usize,
    // A terminal-journal replay can commit after Bootstrap has published. Its source row is
    // exact, but it has no in-process dashboard sequence to join the contiguous live journal.
    // Keep it in the same bounded resident budget and layer it into rolling reads separately.
    pub(crate) replayed_entries: VecDeque<DashboardActivityTerminalDelta>,
    pub(crate) replayed_bytes: usize,
    pub(crate) last_reconciled_terminal_watermark: u64,
    pub(crate) reconciliation_completed: bool,
    pub(crate) source_compaction_gap: bool,
    pub(crate) overflowed_through_sequence: Option<u64>,
    pub(crate) gap_proofs: VecDeque<DeltaGapProof>,
    pub(crate) gap_proof_budget_exhausted: bool,
    pub(crate) gap_generation: u64,
}

impl SummaryDeltaJournal {
    pub(crate) fn contains_same_identity(
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

    pub(crate) fn contains_conflicting_row_identity(
        &self,
        delta: &DashboardActivityTerminalDelta,
    ) -> bool {
        let Some(row_id) = delta.persisted_row_id else {
            return false;
        };
        self.pending
            .values()
            .chain(self.entries.iter().map(|entry| &entry.delta))
            .chain(self.committed_reorder.values())
            .chain(self.replayed_entries.iter())
            .any(|existing| {
                existing.persisted_row_id == Some(row_id)
                    && !Self::contains_same_identity(existing, delta)
            })
    }

    fn resident_delta_count(&self) -> usize {
        self.entries
            .len()
            .saturating_add(self.replayed_entries.len())
            .saturating_add(self.pending.len())
            .saturating_add(self.committed_reorder.len())
    }

    fn resident_delta_bytes(&self) -> usize {
        self.bytes
            .saturating_add(self.replayed_bytes)
            .saturating_add(self.pending_bytes)
            .saturating_add(self.committed_reorder_bytes)
    }

    fn has_capacity_for(&self, delta: &DashboardActivityTerminalDelta) -> bool {
        self.resident_delta_count() < SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS
            && self
                .resident_delta_bytes()
                .saturating_add(delta.estimated_bytes)
                <= SUMMARY_TERMINAL_OVERLAY_MAX_BYTES
    }

    pub(crate) fn compatible_pending_identity(
        pending: &DashboardActivityTerminalDelta,
        acknowledged: &DashboardActivityTerminalDelta,
    ) -> bool {
        pending.invoke_id == acknowledged.invoke_id
            && pending.occurred_at == acknowledged.occurred_at
            && (pending.persisted_row_id.is_none()
                || acknowledged.persisted_row_id.is_none()
                || pending.persisted_row_id == acknowledged.persisted_row_id)
    }

    pub(crate) fn absorb_committed_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
        if let Some(pending) = self.pending.remove(&delta.terminal_sequence) {
            self.pending_bytes = self.pending_bytes.saturating_sub(pending.estimated_bytes);
        }
        self.entries
            .retain(|entry| !Self::contains_same_identity(&entry.delta, delta));
        self.committed_reorder
            .retain(|_, entry| !Self::contains_same_identity(entry, delta));
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
        self.committed_reorder_bytes = self
            .committed_reorder
            .values()
            .map(|entry| entry.estimated_bytes)
            .sum();
        self.cursor = self.cursor.max(SummaryDeltaCursor(delta.terminal_sequence));
        self.base_cursor = self
            .base_cursor
            .max(SummaryDeltaCursor(delta.terminal_sequence));
        self.last_reconciled_terminal_watermark = self
            .last_reconciled_terminal_watermark
            .max(delta.terminal_sequence);
        self.drain_committed_reorder();
    }

    // Registration happens before the asynchronous SQLite enqueue. Pending entries establish
    // the expected sequence and retain a bounded recovery proof, but they are never exposed to
    // Summary reads until the writer calls `append` after commit.
    pub(crate) fn register_pending(&mut self, delta: DashboardActivityTerminalDelta) -> bool {
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
        if let Some(existing) = self.committed_reorder.get(&sequence) {
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
        if !self.has_capacity_for(&delta) {
            self.overflowed_through_sequence = Some(sequence);
            self.note_gap(&delta);
            return false;
        }
        self.pending_bytes = self.pending_bytes.saturating_add(delta.estimated_bytes);
        self.pending.insert(sequence, delta);
        true
    }

    pub(crate) fn rollback_pending(&mut self, sequence: Option<u64>) {
        let Some(sequence) = sequence else {
            return;
        };
        if let Some(delta) = self.pending.remove(&sequence) {
            self.pending_bytes = self.pending_bytes.saturating_sub(delta.estimated_bytes);
        }
        let known_through = self.cursor.max(self.base_cursor).0;
        if sequence <= known_through {
            return;
        }
        self.rolled_back_sequences.insert(sequence);
        self.advance_rolled_back_prefix();
        self.drain_committed_reorder();
    }

    pub(crate) fn commit_skipped_sequences(&mut self) {
        self.advance_rolled_back_prefix();
        self.drain_committed_reorder();
    }

    fn advance_rolled_back_prefix(&mut self) {
        let mut known_through = self.cursor.max(self.base_cursor).0;
        while self
            .rolled_back_sequences
            .remove(&known_through.saturating_add(1))
        {
            known_through = known_through.saturating_add(1);
        }
        self.cursor = self.cursor.max(SummaryDeltaCursor(known_through));
    }

    pub(crate) fn acknowledge_pending(&mut self, delta: DashboardActivityTerminalDelta) -> bool {
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

    fn drain_committed_reorder(&mut self) -> usize {
        if self.overflowed_through_sequence.is_some() {
            return 0;
        }
        self.advance_rolled_back_prefix();
        let mut advanced = 0;
        loop {
            let expected_next = self.cursor.max(self.base_cursor).0.saturating_add(1);
            let Some(delta) = self.committed_reorder.remove(&expected_next) else {
                break;
            };
            self.committed_reorder_bytes = self
                .committed_reorder_bytes
                .saturating_sub(delta.estimated_bytes);
            self.bytes = self.bytes.saturating_add(delta.estimated_bytes);
            self.cursor = SummaryDeltaCursor(expected_next);
            self.entries.push_back(SummaryDeltaEntry {
                cursor: SummaryDeltaCursor(expected_next),
                delta,
            });
            advanced += 1;
        }
        if advanced > 0 {
            crate::observability::contention::record_recovery(
                crate::observability::contention::RecoveryDiagnostic::SummaryReorderAdvanced,
            );
        }
        advanced
    }

    pub(crate) fn note_unknown_terminal_gap(&mut self, cursor: u64) {
        self.cursor = self.cursor.max(SummaryDeltaCursor(cursor));
        self.retain_gap_proof(DeltaGapProof {
            cursor: SummaryDeltaCursor(cursor),
            terminal_sequence: Some(cursor),
            source_gap: false,
            compaction_gap: false,
            upstream_account_id: None,
            occurred_at: String::new(),
            row_id: None,
            invoke_id: None,
        });
    }

    pub(crate) fn note_unknown_source_cursor_gap(&mut self, cursor: u64) {
        // A source-journal compaction cursor has no terminal sequence and remains broad until the
        // source tail is rebuilt.
        self.retain_gap_proof(DeltaGapProof {
            cursor: SummaryDeltaCursor(cursor),
            terminal_sequence: None,
            source_gap: true,
            compaction_gap: true,
            upstream_account_id: None,
            occurred_at: String::new(),
            row_id: None,
            invoke_id: None,
        });
        self.source_compaction_gap = true;
    }

    pub(crate) fn clear_source_compaction_gap(&mut self) {
        if !self.source_compaction_gap || self.gap_proof_budget_exhausted {
            return;
        }
        self.gap_proofs.retain(|proof| !proof.compaction_gap);
        self.source_compaction_gap = false;
        self.gap_proof_budget_exhausted = false;
        self.gap_generation = self.gap_generation.saturating_add(1);
    }

    pub(crate) fn note_gap(&mut self, delta: &DashboardActivityTerminalDelta) {
        self.cursor = self.cursor.max(SummaryDeltaCursor(delta.terminal_sequence));
        self.retain_gap_proof(DeltaGapProof {
            cursor: SummaryDeltaCursor(delta.terminal_sequence),
            terminal_sequence: Some(delta.terminal_sequence),
            source_gap: false,
            compaction_gap: false,
            upstream_account_id: delta.upstream_account_id,
            occurred_at: delta.occurred_at.clone(),
            row_id: delta.persisted_row_id,
            invoke_id: delta.persisted_row_id.map(|_| delta.invoke_id.clone()),
        });
    }

    pub(crate) fn retain_gap_proof(&mut self, proof: DeltaGapProof) {
        self.gap_generation = self.gap_generation.saturating_add(1);
        if self.gap_proof_budget_exhausted {
            if proof.source_gap {
                for existing in &mut self.gap_proofs {
                    existing.source_gap = true;
                }
            }
            return;
        }
        if self.gap_proofs.len() >= SUMMARY_DELTA_JOURNAL_MAX_GAP_PROOFS {
            // Dropping the oldest scoped proof could make an old account/range look exact.
            // Keep one irreversible broad proof until a durable projection consumes the gap.
            let source_gap =
                proof.source_gap || self.gap_proofs.iter().any(|existing| existing.source_gap);
            self.gap_proofs.clear();
            self.gap_proofs.push_back(DeltaGapProof {
                cursor: proof.cursor,
                terminal_sequence: None,
                source_gap,
                compaction_gap: false,
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

    pub(crate) fn append_replayed(&mut self, delta: DashboardActivityTerminalDelta) -> bool {
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
            .chain(self.committed_reorder.values())
            .chain(self.replayed_entries.iter())
            .any(|entry| Self::contains_same_identity(entry, &delta))
        {
            return true;
        }
        if self.overflowed_through_sequence.is_some() {
            self.note_gap(&delta);
            return false;
        }
        if !self.has_capacity_for(&delta) {
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
    pub(crate) fn append(
        &mut self,
        delta: DashboardActivityTerminalDelta,
        slice_high_watermark: u64,
    ) -> bool {
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
        if let Some(existing) = self.committed_reorder.get(&delta.terminal_sequence) {
            if Self::contains_same_identity(existing, &delta) {
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
        if delta.terminal_sequence > expected_next {
            if !self.has_capacity_for(&delta) {
                self.overflowed_through_sequence = Some(slice_high_watermark);
                self.note_gap(&delta);
                return false;
            }
            self.committed_reorder_bytes = self
                .committed_reorder_bytes
                .saturating_add(delta.estimated_bytes);
            self.committed_reorder
                .insert(delta.terminal_sequence, delta);
            crate::observability::contention::record_recovery(
                crate::observability::contention::RecoveryDiagnostic::SummaryReorderBuffered,
            );
            return true;
        }
        if delta.terminal_sequence < expected_next {
            self.overflowed_through_sequence = Some(slice_high_watermark);
            self.note_unknown_terminal_gap(slice_high_watermark);
            self.note_gap(&delta);
            return false;
        }
        if !self.has_capacity_for(&delta) {
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
        self.drain_committed_reorder();
        true
    }

    pub(crate) fn retain_replayed_for_reconciliation(
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
                .chain(self.committed_reorder.values())
                .chain(self.replayed_entries.iter())
                .any(|entry| Self::contains_same_identity(entry, delta))
            {
                continue;
            }
            if !self.has_capacity_for(delta) {
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
        let gap_count = self.gap_proofs.len();
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
        if self.gap_proofs.len() != gap_count {
            self.gap_generation = self.gap_generation.saturating_add(1);
        }
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

pub(crate) fn record_all_time_account_overflow_marker(
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
pub(crate) fn append_summary_all_time_delta(
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
pub(crate) struct CachedSubscriptionTopic {
    pub(crate) topic: SubscriptionTopic,
    pub(crate) descriptor: SubscriptionTopicDescriptor,
    pub(crate) schema_epoch: String,
    pub(crate) cursor: u64,
    pub(crate) snapshot_built_at: Instant,
    pub(crate) refresh_scheduled: bool,
    pub(crate) conversation_overview_refresh_scheduled: bool,
    pub(crate) conversation_overview_refresh_in_flight: bool,
    pub(crate) conversation_overview_refresh_pending: bool,
    pub(crate) upstream_account_attempt_refresh_scheduled: bool,
    pub(crate) upstream_account_attempt_refresh_in_flight: bool,
    pub(crate) upstream_account_attempt_refresh_pending: bool,
    pub(crate) upstream_account_attempt_refresh_generation: u64,
    pub(crate) dirty: bool,
    pub(crate) runtime_topic_recovery_generation: u64,
    pub(crate) runtime_topic_recovery_retry_at: Option<Instant>,
    pub(crate) summary_refresh_scheduled: bool,
    pub(crate) summary_refresh_in_flight: bool,
    pub(crate) summary_pending_event_count: u64,
    pub(crate) summary_retry_backoff_ms: u64,
    pub(crate) summary_gap_generation: u64,
    pub(crate) summary_gap_degraded: bool,
    pub(crate) parallel_work_refresh_scheduled: bool,
    pub(crate) prompt_cache_refresh_scheduled: bool,
    pub(crate) prompt_cache_reconcile_scheduled: bool,
    pub(crate) prompt_cache_key_hydration_scheduled: bool,
    pub(crate) prompt_cache_pending_records: BTreeMap<String, PromptCacheTopicDelta>,
    pub(crate) prompt_cache_pending_key_hydrations: BTreeSet<String>,
    pub(crate) prompt_cache_candidate_refill_required: bool,
    pub(crate) prompt_cache_applied_terminal_ids: HashSet<String>,
    pub(crate) prompt_cache_coalesced_event_count: u64,
    pub(crate) prompt_cache_full_hydration_count: u64,
    pub(crate) prompt_cache_bounded_key_hydration_count: u64,
    pub(crate) prompt_cache_baseline_at: Option<Instant>,
    pub(crate) prompt_cache_baseline_row_id: i64,
    pub(crate) prompt_cache_response_source: &'static str,
    pub(crate) prompt_cache_reconcile_required: bool,
    pub(crate) prompt_cache_pressure_deferred: bool,
    pub(crate) latest_live_snapshot: Option<DashboardActivityLiveSnapshot>,
    pub(crate) calendar_anchor: Option<String>,
    pub(crate) continuity_reset_cursor: Option<u64>,
    pub(crate) dashboard_materializer: Option<DashboardTopicMaterializer>,
    pub(crate) dashboard_base_revision: u64,
    pub(crate) dashboard_materialized_revision: Option<DashboardTopicRevision>,
    pub(crate) snapshot_payload: Value,
    pub(crate) snapshot_frame: Arc<SerializedTopicFrame>,
    pub(crate) snapshot_bytes: usize,
    pub(crate) replay_events: VecDeque<ReplayableTopicEvent>,
    pub(crate) replay_bytes: usize,
}

impl CachedSubscriptionTopic {
    pub(crate) fn invalidate_upstream_account_attempt_refresh(&mut self) {
        self.upstream_account_attempt_refresh_generation = self
            .upstream_account_attempt_refresh_generation
            .saturating_add(1);
        self.upstream_account_attempt_refresh_scheduled = false;
        self.upstream_account_attempt_refresh_in_flight = false;
        self.upstream_account_attempt_refresh_pending = false;
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeTopicWork {
    pub(crate) topic: SubscriptionTopic,
    pub(crate) terminal_event_count: u64,
    pub(crate) includes_invocation_mutation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum RuntimeTopicDependency {
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
pub(crate) struct PromptCacheTopicDelta {
    pub(crate) row_id: i64,
    pub(crate) identity: String,
    pub(crate) invoke_id: String,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) occurred_at: String,
    pub(crate) is_runtime_removed: bool,
    pub(crate) status: String,
    pub(crate) is_terminal: bool,
    pub(crate) is_success: bool,
    pub(crate) request_tokens: i64,
    pub(crate) cost: f64,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reported_cache_write_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) cost_input: Option<f64>,
    pub(crate) cost_cache_write: Option<f64>,
    pub(crate) cost_cache_read: Option<f64>,
    pub(crate) cost_output: Option<f64>,
    pub(crate) cost_reasoning: Option<f64>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) preview: Option<PromptCacheConversationInvocationPreviewResponse>,
}

pub(crate) fn prompt_cache_hydration_changed_pending_keys(
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

pub(crate) struct PromptCacheBaselineBuild {
    pub(crate) baseline_row_id: i64,
    pub(crate) persisted_identities: HashSet<String>,
    pub(crate) runtime_overlay_terminal_identities: HashSet<String>,
}

pub(crate) struct ParallelWorkBaselineBuild {
    pub(crate) persisted_identities: HashSet<String>,
}

pub(crate) async fn capture_prompt_cache_baseline_runtime_records(
    state: &AppState,
    topic: &SubscriptionTopic,
) -> Result<Vec<ApiInvocation>, ApiError> {
    let SubscriptionTopic::PromptCacheWindow { selection, .. } = topic else {
        return Ok(Vec::new());
    };
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_start_bound =
        db_occurred_at_lower_bound(Utc::now() - selection.activity_window_duration());
    Ok(runtime_prompt_cache_overlay_records(
        state,
        source_scope,
        &range_start_bound,
        None,
    ))
}

pub(crate) fn prompt_cache_baseline_runtime_terminal_identities(
    topic: &SubscriptionTopic,
    payload: &BuiltSubscriptionTopicPayload,
    runtime_records: &[ApiInvocation],
) -> HashSet<String> {
    if !matches!(topic, SubscriptionTopic::PromptCacheWindow { .. }) {
        return HashSet::new();
    }
    let selected_keys = payload
        .snapshot_payload()
        .get("conversations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|conversation| {
            conversation
                .get("promptCacheKey")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect::<HashSet<_>>();
    runtime_records
        .iter()
        .filter(|record| {
            !prompt_cache_runtime_record_is_in_flight(record)
                && prompt_invocation_status_counts_toward_terminal_totals(record.status.as_deref())
                && record
                    .prompt_cache_key
                    .as_deref()
                    .is_some_and(|key| selected_keys.contains(key))
        })
        .map(runtime_prompt_cache_overlay_identity)
        .collect()
}

impl PromptCacheTopicDelta {
    #[cfg(test)]
    pub(crate) fn from_record(record: &ApiInvocation) -> Result<Option<Self>, ApiError> {
        Ok(PromptCacheRuntimeProjection::from_record(record)
            .as_ref()
            .and_then(Self::from_runtime_projection))
    }

    pub(crate) fn from_runtime_mutation(
        mutation: &RuntimeInvocationMutation,
        runtime_projection: Option<&PromptCacheRuntimeProjection>,
    ) -> Result<Option<Self>, ApiError> {
        if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
            return Ok(Self::from_runtime_removal(mutation));
        }
        Ok(runtime_projection.and_then(Self::from_runtime_projection))
    }

    pub(crate) fn from_runtime_projection(
        projection: &PromptCacheRuntimeProjection,
    ) -> Option<Self> {
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
            input_tokens: projection.input_tokens,
            output_tokens: projection.output_tokens,
            cache_input_tokens: projection.cache_input_tokens,
            reported_cache_write_tokens: projection.reported_cache_write_tokens,
            reasoning_tokens: projection.reasoning_tokens,
            cost_input: projection.cost_input,
            cost_cache_write: projection.cost_cache_write,
            cost_cache_read: projection.cost_cache_read,
            cost_output: projection.cost_output,
            cost_reasoning: projection.cost_reasoning,
            upstream_account_id: preview.upstream_account_id,
            upstream_account_name: preview.upstream_account_name.clone(),
            preview: Some(preview),
        })
    }

    pub(crate) fn from_runtime_removal(mutation: &RuntimeInvocationMutation) -> Option<Self> {
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
            input_tokens: None,
            output_tokens: None,
            cache_input_tokens: None,
            reported_cache_write_tokens: None,
            reasoning_tokens: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            upstream_account_id: mutation.upstream_account_id,
            upstream_account_name: None,
            preview: None,
        })
    }
}
