use super::*;

#[derive(Debug, Clone, Copy)]
pub(crate) enum FlushReason {
    RowLimit,
    Interval,
    MaxAge,
    Barrier,
    Shutdown,
}

impl FlushReason {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::RowLimit => "row_limit",
            Self::Interval => "interval",
            Self::MaxAge => "max_age",
            Self::Barrier => "barrier",
            Self::Shutdown => "shutdown",
        }
    }

    pub(super) fn bypass_pressure_gate(self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedInvocationDerivedWrites {
    pub(crate) invocation_id: i64,
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) terminal_overlay_key: Option<(String, String)>,
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedTerminalInvocationWrite {
    pub(crate) enqueued_at: Option<Instant>,
    pub(crate) diagnostic: Option<Arc<crate::observability::diagnostics::PersistenceTicket>>,
    pub(crate) record: ProxyCaptureRecord,
    pub(crate) attempts: Vec<RuntimeAttemptSnapshot>,
    pub(crate) capture_started: Option<Instant>,
    pub(crate) raw_capture: bool,
    pub(crate) dashboard_terminal_sequence: Option<u64>,
    pub(crate) terminal_projection_event_ids: Vec<u64>,
    // Computed from the already-materialized terminal record before P1 admission. This keeps
    // event-driven repair discovery out of the SQLite transaction and avoids another payload parse.
    pub(crate) startup_backfill_tasks: Vec<StartupBackfillTask>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TerminalEnqueueOutcome {
    pub(crate) enqueued: bool,
    pub(crate) durability_mode: TerminalJournalDurabilityMode,
    pub(crate) journal_sequence: Option<u64>,
    pub(crate) journal_pending_records: usize,
    pub(crate) journal_pending_bytes: u64,
}

impl BatchedTerminalInvocationWrite {
    fn key(&self) -> String {
        format!(
            "{}\n{}\n{}",
            self.record.invoke_id, self.record.occurred_at, self.raw_capture
        )
    }

    pub(crate) fn estimated_memory_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(self.record.estimated_memory_bytes())
            .saturating_add(
                self.attempts
                    .iter()
                    .map(RuntimeAttemptSnapshot::estimated_memory_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.terminal_projection_event_ids
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .saturating_add(
                self.startup_backfill_tasks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<StartupBackfillTask>()),
            )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedAccountSelectedTouch {
    pub(crate) account_id: i64,
    pub(crate) selected_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct BatchedSystemTaskFinish {
    pub(crate) run_id: i64,
    pub(crate) task_key: String,
    pub(crate) task_kind: SystemTaskKind,
    pub(crate) trigger_kind: String,
    pub(crate) status: SystemTaskStatus,
    pub(crate) summary: Option<String>,
    pub(crate) detail: Option<String>,
    pub(crate) finished_at: String,
    pub(crate) duration_ms: i64,
    pub(crate) completion: Option<String>,
    pub(crate) core_completion: Option<String>,
    pub(crate) details: Option<serde_json::Value>,
}

#[derive(Debug)]
pub(crate) enum SqliteBatchWrite {
    TerminalInvocation(BatchedTerminalInvocationWrite),
    AttemptProgress(BatchedAttemptProgress),
    InvocationDerived(BatchedInvocationDerivedWrites),
    AccountSelectedTouch(BatchedAccountSelectedTouch),
    SystemTaskFinish(BatchedSystemTaskFinish),
    #[cfg(test)]
    StartupBackfillWake(StartupBackfillTask),
}

pub(crate) enum SqliteBatchWriterControl {
    FlushNow {
        queued_depth_snapshot: usize,
        responder: oneshot::Sender<Result<(), String>>,
    },
    Shutdown {
        queued_depth_snapshot: usize,
        responder: oneshot::Sender<Result<(), String>>,
    },
}

#[derive(Debug, Default, Clone)]
pub(crate) struct PendingBatch {
    pub(super) terminal_invocations: BTreeMap<String, BatchedTerminalInvocationWrite>,
    pub(super) attempt_progress: HashMap<i64, BatchedAttemptProgress>,
    pub(super) invocation_derived: BTreeMap<i64, BatchedInvocationDerivedWrites>,
    pub(super) account_selected_touches: HashMap<i64, BatchedAccountSelectedTouch>,
    pub(super) system_task_finishes: HashMap<i64, BatchedSystemTaskFinish>,
    pub(super) startup_backfill_wake_tasks: Vec<StartupBackfillTask>,
    pub(super) enqueued_rows: usize,
    pub(super) coalesced_rows: usize,
    pub(super) attempt_progress_coalesced_rows: usize,
    pub(super) estimated_bytes: usize,
    pub(super) terminal_estimated_bytes: usize,
    pub(super) oldest_at: Option<Instant>,
    pub(super) retained_for_retry: bool,
    pub(super) recovery_replay: bool,
}

impl PendingBatch {
    pub(super) fn add_estimate(&mut self, bytes: usize, is_terminal: bool) {
        self.estimated_bytes = self.estimated_bytes.saturating_add(bytes);
        if is_terminal {
            self.terminal_estimated_bytes = self.terminal_estimated_bytes.saturating_add(bytes);
        }
    }

    pub(super) fn replace_estimate(
        &mut self,
        old_bytes: usize,
        new_bytes: usize,
        is_terminal: bool,
    ) {
        self.estimated_bytes = self
            .estimated_bytes
            .saturating_sub(old_bytes)
            .saturating_add(new_bytes);
        if is_terminal {
            self.terminal_estimated_bytes = self
                .terminal_estimated_bytes
                .saturating_sub(old_bytes)
                .saturating_add(new_bytes);
        }
    }

    pub(super) fn recalculate_estimates(&mut self) {
        self.estimated_bytes = self
            .terminal_invocations
            .values()
            .map(BatchedTerminalInvocationWrite::estimated_memory_bytes)
            .sum::<usize>()
            .saturating_add(
                self.attempt_progress
                    .values()
                    .map(estimated_attempt_progress_memory_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.invocation_derived
                    .values()
                    .map(estimated_invocation_derived_memory_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.account_selected_touches
                    .values()
                    .map(estimated_account_selected_touch_memory_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.system_task_finishes
                    .values()
                    .map(estimated_system_task_finish_memory_bytes)
                    .sum::<usize>(),
            )
            .saturating_add(
                self.startup_backfill_wake_tasks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<StartupBackfillTask>()),
            );
        self.terminal_estimated_bytes = self
            .terminal_invocations
            .values()
            .map(BatchedTerminalInvocationWrite::estimated_memory_bytes)
            .sum();
    }

    pub(super) fn is_empty(&self) -> bool {
        self.terminal_invocations.is_empty()
            && self.attempt_progress.is_empty()
            && self.invocation_derived.is_empty()
            && self.account_selected_touches.is_empty()
            && self.system_task_finishes.is_empty()
            && self.startup_backfill_wake_tasks.is_empty()
    }

    pub(super) fn has_p2(&self) -> bool {
        !self.attempt_progress.is_empty()
            || !self.invocation_derived.is_empty()
            || !self.account_selected_touches.is_empty()
            || !self.system_task_finishes.is_empty()
            || !self.startup_backfill_wake_tasks.is_empty()
    }

    pub(super) fn logical_rows(&self) -> usize {
        self.terminal_invocations.len()
            + self.attempt_progress.len()
            + self.invocation_derived.len()
            + self.account_selected_touches.len()
            + self.system_task_finishes.len()
            + usize::from(!self.startup_backfill_wake_tasks.is_empty())
    }

    pub(super) fn age(&self) -> Duration {
        self.oldest_at
            .map(|oldest| oldest.elapsed())
            .unwrap_or_default()
    }

    pub(super) fn push(&mut self, write: SqliteBatchWrite) -> usize {
        let now = Instant::now();
        let write_bytes = write.estimated_memory_bytes();
        self.oldest_at.get_or_insert(now);
        self.enqueued_rows += 1;
        match write {
            SqliteBatchWrite::TerminalInvocation(terminal) => {
                let key = terminal.key();
                let estimate_change = match self.terminal_invocations.entry(key) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(terminal);
                        (0, write_bytes)
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        let old_bytes = entry.get().estimated_memory_bytes();
                        let existing = entry.get();
                        let preserved_sequence = terminal
                            .dashboard_terminal_sequence
                            .or(existing.dashboard_terminal_sequence);
                        let mut terminal = terminal;
                        match (&existing.diagnostic, &terminal.diagnostic) {
                            (Some(old), Some(new)) if !Arc::ptr_eq(old, new) => {
                                old.finish("coalesced_unavailable", false)
                            }
                            (Some(old), None) => terminal.diagnostic = Some(old.clone()),
                            _ => {}
                        }
                        terminal.dashboard_terminal_sequence = preserved_sequence;
                        terminal.attempts =
                            merge_terminal_attempts(&existing.attempts, terminal.attempts);
                        terminal
                            .terminal_projection_event_ids
                            .extend(existing.terminal_projection_event_ids.iter().copied());
                        terminal.terminal_projection_event_ids.sort_unstable();
                        terminal.terminal_projection_event_ids.dedup();
                        for task in existing.startup_backfill_tasks.iter().copied() {
                            if !terminal.startup_backfill_tasks.contains(&task) {
                                terminal.startup_backfill_tasks.push(task);
                            }
                        }
                        let new_bytes = terminal.estimated_memory_bytes();
                        entry.insert(terminal);
                        self.coalesced_rows += 1;
                        (old_bytes, new_bytes)
                    }
                };
                if estimate_change.0 == 0 {
                    self.add_estimate(estimate_change.1, true);
                } else {
                    self.replace_estimate(estimate_change.0, estimate_change.1, true);
                }
                estimate_change.0
            }
            SqliteBatchWrite::AttemptProgress(progress) => {
                let attempt_id = progress.attempt_id;
                if let Some(existing) = self.attempt_progress.get_mut(&attempt_id) {
                    let old_bytes = estimated_attempt_progress_memory_bytes(existing);
                    merge_attempt_progress(existing, progress);
                    let new_bytes = estimated_attempt_progress_memory_bytes(existing);
                    self.replace_estimate(old_bytes, new_bytes, false);
                    self.coalesced_rows += 1;
                    self.attempt_progress_coalesced_rows += 1;
                    old_bytes
                } else {
                    self.attempt_progress.insert(attempt_id, progress);
                    self.add_estimate(write_bytes, false);
                    0
                }
            }
            SqliteBatchWrite::InvocationDerived(derived) => {
                let old = self
                    .invocation_derived
                    .insert(derived.invocation_id, derived);
                if let Some(old) = old {
                    let old_bytes = estimated_invocation_derived_memory_bytes(&old);
                    self.replace_estimate(old_bytes, write_bytes, false);
                    self.coalesced_rows += 1;
                    old_bytes
                } else {
                    self.add_estimate(write_bytes, false);
                    0
                }
            }
            SqliteBatchWrite::AccountSelectedTouch(touch) => {
                match self.account_selected_touches.entry(touch.account_id) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(touch);
                        self.add_estimate(write_bytes, false);
                        0
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let existing = entry.get_mut();
                        let old_bytes = estimated_account_selected_touch_memory_bytes(existing);
                        if existing.selected_at < touch.selected_at {
                            *existing = touch;
                            self.replace_estimate(old_bytes, write_bytes, false);
                            self.coalesced_rows += 1;
                            old_bytes
                        } else {
                            self.coalesced_rows += 1;
                            write_bytes
                        }
                    }
                }
            }
            SqliteBatchWrite::SystemTaskFinish(finish) => {
                let old = self.system_task_finishes.insert(finish.run_id, finish);
                if let Some(old) = old {
                    let old_bytes = estimated_system_task_finish_memory_bytes(&old);
                    self.replace_estimate(old_bytes, write_bytes, false);
                    self.coalesced_rows += 1;
                    old_bytes
                } else {
                    self.add_estimate(write_bytes, false);
                    0
                }
            }
            #[cfg(test)]
            SqliteBatchWrite::StartupBackfillWake(task) => {
                if self.startup_backfill_wake_tasks.contains(&task) {
                    self.coalesced_rows += 1;
                    write_bytes
                } else {
                    self.startup_backfill_wake_tasks.push(task);
                    self.add_estimate(write_bytes, false);
                    0
                }
            }
        }
    }

    pub(super) fn push_accounted(
        &mut self,
        write: SqliteBatchWrite,
        accounting: &PendingQueueAccounting,
    ) {
        let p1_before = self.terminal_invocations.len() + usize::from(is_p1_terminal_write(&write));
        let accounted_depth_before = self.logical_rows().saturating_add(1);
        let accounted_before = self
            .estimated_memory_bytes()
            .saturating_add(write.estimated_memory_bytes());
        self.push(write);
        accounting.observe_p1_replace(p1_before, self.terminal_invocations.len());
        accounting.replace_batch(
            accounted_depth_before,
            self.logical_rows(),
            accounted_before,
            self.estimated_memory_bytes(),
        );
    }

    pub(super) fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    pub(super) fn take_p1_terminals(&mut self) -> Self {
        let terminal_invocations = std::mem::take(&mut self.terminal_invocations);
        if terminal_invocations.is_empty() {
            return Self::default();
        }
        let estimated_bytes = self.terminal_estimated_bytes;
        self.estimated_bytes = self.estimated_bytes.saturating_sub(estimated_bytes);
        self.terminal_estimated_bytes = self
            .terminal_estimated_bytes
            .saturating_sub(estimated_bytes);
        let recovery_replay = self.recovery_replay;
        if !self.has_p2() {
            self.oldest_at = None;
            self.recovery_replay = false;
        }
        Self {
            enqueued_rows: terminal_invocations.len(),
            estimated_bytes,
            terminal_estimated_bytes: estimated_bytes,
            terminal_invocations,
            oldest_at: self.oldest_at,
            recovery_replay,
            ..Self::default()
        }
    }

    pub(super) fn take_p1_terminal_chunk(&mut self, max_rows: usize, max_bytes: usize) -> Self {
        if self.terminal_invocations.is_empty() || max_rows == 0 || max_bytes == 0 {
            return Self::default();
        }
        // The map is keyed by invocation identity, but Summary consumes terminal entries by
        // the registration sequence assigned before persistence. Preserve that sequence across
        // batches whenever the pending set contains a contiguous prefix.
        let mut candidates = self.terminal_invocations.iter().collect::<Vec<_>>();
        candidates
            .sort_by_key(|(_, terminal)| terminal.dashboard_terminal_sequence.unwrap_or(u64::MAX));
        let mut selected_keys = Vec::new();
        let mut selected_bytes = 0_usize;
        for (key, terminal) in candidates {
            let bytes = terminal.estimated_memory_bytes();
            if selected_keys.len() >= max_rows {
                break;
            }
            if !selected_keys.is_empty() && selected_bytes.saturating_add(bytes) > max_bytes {
                break;
            }
            selected_keys.push(key.clone());
            selected_bytes = selected_bytes.saturating_add(bytes);
            if bytes > max_bytes {
                break;
            }
        }
        let terminal_invocations = selected_keys
            .into_iter()
            .filter_map(|key| {
                self.terminal_invocations
                    .remove(&key)
                    .map(|write| (key, write))
            })
            .collect::<BTreeMap<_, _>>();
        if terminal_invocations.is_empty() {
            return Self::default();
        }
        self.recalculate_estimates();
        let recovery_replay = self.recovery_replay;
        let mut chunk = Self {
            terminal_invocations,
            oldest_at: self.oldest_at,
            retained_for_retry: self.retained_for_retry,
            recovery_replay,
            ..Self::default()
        };
        chunk.recalculate_estimates();
        chunk.enqueued_rows = chunk.logical_rows();
        self.enqueued_rows = self.logical_rows();
        if self.is_empty() {
            self.oldest_at = None;
            self.recovery_replay = false;
        }
        chunk
    }

    pub(super) fn take_p2(&mut self) -> Self {
        let mut p2 = std::mem::take(self);
        let terminal_invocations = std::mem::take(&mut p2.terminal_invocations);
        let terminal_estimated_bytes = p2.terminal_estimated_bytes;
        p2.estimated_bytes = p2.estimated_bytes.saturating_sub(terminal_estimated_bytes);
        p2.terminal_estimated_bytes = 0;

        self.terminal_invocations = terminal_invocations;
        self.estimated_bytes = terminal_estimated_bytes;
        self.terminal_estimated_bytes = terminal_estimated_bytes;
        self.enqueued_rows = self.terminal_invocations.len();
        self.oldest_at = p2.oldest_at;
        p2
    }

    pub(super) fn take_p2_chunk(&mut self, max_rows: usize, max_bytes: usize) -> Self {
        if !self.has_p2() || max_rows == 0 || max_bytes == 0 {
            return Self::default();
        }

        let oldest_at = self.oldest_at;
        let recovery_replay = self.recovery_replay;
        let attempt_progress_coalesced_rows = self.attempt_progress_coalesced_rows;
        self.attempt_progress_coalesced_rows = 0;
        let mut chunk = Self {
            oldest_at,
            retained_for_retry: self.retained_for_retry,
            attempt_progress_coalesced_rows,
            recovery_replay,
            ..Self::default()
        };
        let mut selected_rows = 0_usize;
        let mut selected_bytes = 0_usize;
        let mut should_take = |bytes: usize| {
            if selected_rows >= max_rows {
                return false;
            }
            // A single logical write can exceed the batch budget. Keep it
            // isolated rather than combining it with any other P2 write;
            // the caller emits an explicit oversized-batch diagnostic.
            if selected_rows == 0 && bytes > max_bytes {
                selected_rows = 1;
                selected_bytes = bytes;
                return true;
            }
            if selected_rows > 0 && selected_bytes.saturating_add(bytes) > max_bytes {
                return false;
            }
            selected_rows += 1;
            selected_bytes = selected_bytes.saturating_add(bytes);
            true
        };

        let attempt_progress = std::mem::take(&mut self.attempt_progress);
        for (key, progress) in attempt_progress {
            let bytes = estimated_attempt_progress_memory_bytes(&progress);
            if should_take(bytes) {
                chunk.attempt_progress.insert(key, progress);
            } else {
                self.attempt_progress.insert(key, progress);
            }
        }
        let invocation_derived = std::mem::take(&mut self.invocation_derived);
        for (key, derived) in invocation_derived {
            let bytes = estimated_invocation_derived_memory_bytes(&derived);
            if should_take(bytes) {
                chunk.invocation_derived.insert(key, derived);
            } else {
                self.invocation_derived.insert(key, derived);
            }
        }
        let account_selected_touches = std::mem::take(&mut self.account_selected_touches);
        for (key, touch) in account_selected_touches {
            let bytes = estimated_account_selected_touch_memory_bytes(&touch);
            if should_take(bytes) {
                chunk.account_selected_touches.insert(key, touch);
            } else {
                self.account_selected_touches.insert(key, touch);
            }
        }
        let system_task_finishes = std::mem::take(&mut self.system_task_finishes);
        for (key, finish) in system_task_finishes {
            let bytes = estimated_system_task_finish_memory_bytes(&finish);
            if should_take(bytes) {
                chunk.system_task_finishes.insert(key, finish);
            } else {
                self.system_task_finishes.insert(key, finish);
            }
        }
        if !self.startup_backfill_wake_tasks.is_empty()
            && should_take(
                self.startup_backfill_wake_tasks
                    .capacity()
                    .saturating_mul(std::mem::size_of::<StartupBackfillTask>()),
            )
        {
            chunk.startup_backfill_wake_tasks =
                std::mem::take(&mut self.startup_backfill_wake_tasks);
        }

        self.recalculate_estimates();
        chunk.recalculate_estimates();
        chunk.enqueued_rows = chunk.logical_rows();
        self.enqueued_rows = self.logical_rows();
        chunk.coalesced_rows = 0;
        if self.is_empty() {
            self.oldest_at = None;
            self.recovery_replay = false;
        }
        chunk
    }

    pub(super) fn add_startup_backfill_wake_tasks(&mut self, tasks: &[StartupBackfillTask]) {
        for task in tasks {
            if !self.startup_backfill_wake_tasks.contains(task) {
                self.startup_backfill_wake_tasks.push(*task);
            }
        }
        self.recalculate_estimates();
    }

    pub(super) fn merge_p2(&mut self, mut other: Self) {
        self.recovery_replay |= other.recovery_replay;
        let attempt_progress = std::mem::take(&mut other.attempt_progress);
        for progress in attempt_progress.into_values() {
            let write_bytes = estimated_attempt_progress_memory_bytes(&progress);
            if let Some(existing) = self.attempt_progress.get_mut(&progress.attempt_id) {
                let old_bytes = estimated_attempt_progress_memory_bytes(existing);
                merge_attempt_progress(existing, progress);
                let new_bytes = estimated_attempt_progress_memory_bytes(existing);
                self.replace_estimate(old_bytes, new_bytes, false);
                self.coalesced_rows += 1;
                self.attempt_progress_coalesced_rows += 1;
            } else {
                self.add_estimate(write_bytes, false);
                self.attempt_progress.insert(progress.attempt_id, progress);
            }
        }
        self.invocation_derived.extend(other.invocation_derived);
        self.account_selected_touches
            .extend(other.account_selected_touches.drain());
        self.system_task_finishes
            .extend(other.system_task_finishes.drain());
        self.add_startup_backfill_wake_tasks(&other.startup_backfill_wake_tasks);
        self.enqueued_rows = self.enqueued_rows.saturating_add(other.enqueued_rows);
        self.coalesced_rows = self.coalesced_rows.saturating_add(other.coalesced_rows);
        self.attempt_progress_coalesced_rows = self
            .attempt_progress_coalesced_rows
            .saturating_add(other.attempt_progress_coalesced_rows);
        self.recalculate_estimates();
        self.oldest_at = match (self.oldest_at, other.oldest_at) {
            (Some(current), Some(other)) => Some(current.min(other)),
            (current, other) => current.or(other),
        };
    }

    pub(super) fn merge_all(&mut self, mut other: Self) {
        self.recovery_replay |= other.recovery_replay;
        let terminal_invocations = std::mem::take(&mut other.terminal_invocations);
        let terminal_count = terminal_invocations.len();
        for terminal in terminal_invocations.into_values() {
            self.push(SqliteBatchWrite::TerminalInvocation(terminal));
        }
        other.enqueued_rows = other.enqueued_rows.saturating_sub(terminal_count);
        self.merge_p2(other);
    }

    #[cfg(test)]
    pub(super) fn into_writes(self) -> Vec<SqliteBatchWrite> {
        let mut writes = Vec::with_capacity(self.logical_rows());
        writes.extend(
            self.terminal_invocations
                .into_values()
                .map(SqliteBatchWrite::TerminalInvocation),
        );
        writes.extend(
            self.attempt_progress
                .into_values()
                .map(SqliteBatchWrite::AttemptProgress),
        );
        writes.extend(
            self.invocation_derived
                .into_values()
                .map(SqliteBatchWrite::InvocationDerived),
        );
        writes.extend(
            self.account_selected_touches
                .into_values()
                .map(SqliteBatchWrite::AccountSelectedTouch),
        );
        writes.extend(
            self.system_task_finishes
                .into_values()
                .map(SqliteBatchWrite::SystemTaskFinish),
        );
        writes.extend(
            self.startup_backfill_wake_tasks
                .into_iter()
                .map(SqliteBatchWrite::StartupBackfillWake),
        );
        writes
    }
}

fn merge_terminal_attempts(
    existing: &[RuntimeAttemptSnapshot],
    incoming: Vec<RuntimeAttemptSnapshot>,
) -> Vec<RuntimeAttemptSnapshot> {
    let mut merged = BTreeMap::new();
    for attempt in existing.iter().cloned().chain(incoming) {
        let key = attempt.attempt_public_id.clone();
        if let Some(previous) = merged.remove(&key) {
            merged.insert(
                key,
                crate::app_state::merge_runtime_attempt_snapshot(&previous, attempt),
            );
        } else {
            merged.insert(key, attempt);
        }
    }
    let mut attempts = merged.into_values().collect::<Vec<_>>();
    attempts.sort_by_key(|attempt| {
        (
            attempt.attempt_index,
            attempt.distinct_account_index,
            attempt.same_account_retry_index,
        )
    });
    attempts
}

#[derive(Debug)]
pub(crate) struct RetainedBatch {
    pub(super) batch: PendingBatch,
    pub(super) failed: bool,
    pub(super) p2_retryable_failure: bool,
    pub(super) p2_lock_failure: bool,
    pub(super) p2_defer: Option<P2DeferReason>,
    pub(super) quarantined_system_task_ids: Vec<i64>,
}

impl RetainedBatch {
    pub(super) fn new(mut batch: PendingBatch, failed: bool) -> Self {
        batch.retained_for_retry = true;
        Self {
            batch,
            failed,
            p2_retryable_failure: false,
            p2_lock_failure: false,
            p2_defer: None,
            quarantined_system_task_ids: Vec::new(),
        }
    }

    pub(super) fn p2_deferred(mut batch: PendingBatch, reason: P2DeferReason) -> Self {
        batch.retained_for_retry = true;
        Self {
            batch,
            failed: false,
            p2_retryable_failure: false,
            p2_lock_failure: false,
            p2_defer: Some(reason),
            quarantined_system_task_ids: Vec::new(),
        }
    }

    pub(super) fn p2_failed(
        mut batch: PendingBatch,
        retryable_failure: bool,
        lock_failure: bool,
    ) -> Self {
        batch.retained_for_retry = true;
        Self {
            batch,
            failed: true,
            p2_retryable_failure: retryable_failure,
            p2_lock_failure: lock_failure,
            p2_defer: None,
            quarantined_system_task_ids: Vec::new(),
        }
    }

    pub(super) fn with_quarantined_system_task_ids(mut self, ids: Vec<i64>) -> Self {
        self.quarantined_system_task_ids = ids;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum P2DeferReason {
    PressureCooldown(u64),
    BackgroundBusy { observed_generation: u64 },
}

pub(super) fn estimated_option_string_bytes(value: &Option<String>) -> usize {
    value.as_ref().map_or(0, String::capacity)
}

impl SqliteBatchWrite {
    pub(crate) fn estimated_memory_bytes(&self) -> usize {
        match self {
            Self::TerminalInvocation(terminal) => terminal.estimated_memory_bytes(),
            Self::AttemptProgress(progress) => estimated_attempt_progress_memory_bytes(progress),
            Self::InvocationDerived(derived) => estimated_invocation_derived_memory_bytes(derived),
            Self::AccountSelectedTouch(touch) => {
                estimated_account_selected_touch_memory_bytes(touch)
            }
            Self::SystemTaskFinish(finish) => estimated_system_task_finish_memory_bytes(finish),
            #[cfg(test)]
            Self::StartupBackfillWake(_) => std::mem::size_of::<StartupBackfillTask>(),
        }
    }
}

fn estimated_invocation_derived_memory_bytes(derived: &BatchedInvocationDerivedWrites) -> usize {
    std::mem::size_of::<BatchedInvocationDerivedWrites>()
        .saturating_add(derived.occurred_at.capacity())
        .saturating_add(
            derived
                .terminal_overlay_key
                .as_ref()
                .map_or(0, |(invoke_id, occurred_at)| {
                    invoke_id.capacity().saturating_add(occurred_at.capacity())
                }),
        )
}

fn estimated_account_selected_touch_memory_bytes(touch: &BatchedAccountSelectedTouch) -> usize {
    std::mem::size_of::<BatchedAccountSelectedTouch>().saturating_add(touch.selected_at.capacity())
}

fn estimated_system_task_finish_memory_bytes(finish: &BatchedSystemTaskFinish) -> usize {
    std::mem::size_of::<BatchedSystemTaskFinish>()
        .saturating_add(finish.trigger_kind.capacity())
        .saturating_add(estimated_option_string_bytes(&finish.summary))
        .saturating_add(estimated_option_string_bytes(&finish.detail))
        .saturating_add(finish.finished_at.capacity())
}

impl PendingBatch {
    pub(super) fn estimated_memory_bytes(&self) -> usize {
        self.estimated_bytes
    }
}
