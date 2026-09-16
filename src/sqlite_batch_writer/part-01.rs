impl PendingBatch {
    fn add_estimate(&mut self, bytes: usize, is_terminal: bool) {
        self.estimated_bytes = self.estimated_bytes.saturating_add(bytes);
        if is_terminal {
            self.terminal_estimated_bytes = self.terminal_estimated_bytes.saturating_add(bytes);
        }
    }

    fn replace_estimate(&mut self, old_bytes: usize, new_bytes: usize, is_terminal: bool) {
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

    fn recalculate_estimates(&mut self) {
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

    fn is_empty(&self) -> bool {
        self.terminal_invocations.is_empty()
            && self.attempt_progress.is_empty()
            && self.invocation_derived.is_empty()
            && self.account_selected_touches.is_empty()
            && self.system_task_finishes.is_empty()
            && self.startup_backfill_wake_tasks.is_empty()
    }

    fn has_p2(&self) -> bool {
        !self.attempt_progress.is_empty()
            || !self.invocation_derived.is_empty()
            || !self.account_selected_touches.is_empty()
            || !self.system_task_finishes.is_empty()
            || !self.startup_backfill_wake_tasks.is_empty()
    }

    fn logical_rows(&self) -> usize {
        self.terminal_invocations.len()
            + self.attempt_progress.len()
            + self.invocation_derived.len()
            + self.account_selected_touches.len()
            + self.system_task_finishes.len()
            + usize::from(!self.startup_backfill_wake_tasks.is_empty())
    }

    fn age(&self) -> Duration {
        self.oldest_at
            .map(|oldest| oldest.elapsed())
            .unwrap_or_default()
    }

    fn push(&mut self, write: SqliteBatchWrite) -> usize {
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
                        let preserved_sequence = terminal
                            .dashboard_terminal_sequence
                            .or(entry.get().dashboard_terminal_sequence);
                        let mut terminal = terminal;
                        terminal.dashboard_terminal_sequence = preserved_sequence;
                        terminal
                            .terminal_projection_event_ids
                            .extend(entry.get().terminal_projection_event_ids.iter().copied());
                        terminal.terminal_projection_event_ids.sort_unstable();
                        terminal.terminal_projection_event_ids.dedup();
                        for task in entry.get().startup_backfill_tasks.iter().copied() {
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
                let old = self.attempt_progress.insert(progress.attempt_id, progress);
                if let Some(old) = old {
                    let old_bytes = estimated_attempt_progress_memory_bytes(&old);
                    self.replace_estimate(old_bytes, write_bytes, false);
                    self.coalesced_rows += 1;
                    old_bytes
                } else {
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

    fn push_accounted(&mut self, write: SqliteBatchWrite, accounting: &PendingQueueAccounting) {
        let accounted_depth_before = self.logical_rows().saturating_add(1);
        let accounted_before = self
            .estimated_memory_bytes()
            .saturating_add(write.estimated_memory_bytes());
        self.push(write);
        accounting.replace_batch(
            accounted_depth_before,
            self.logical_rows(),
            accounted_before,
            self.estimated_memory_bytes(),
        );
    }

    fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    fn take_p1_terminals(&mut self) -> Self {
        let terminal_invocations = std::mem::take(&mut self.terminal_invocations);
        if terminal_invocations.is_empty() {
            return Self::default();
        }
        let estimated_bytes = self.terminal_estimated_bytes;
        self.estimated_bytes = self.estimated_bytes.saturating_sub(estimated_bytes);
        self.terminal_estimated_bytes = self
            .terminal_estimated_bytes
            .saturating_sub(estimated_bytes);
        if !self.has_p2() {
            self.oldest_at = None;
        }
        Self {
            enqueued_rows: terminal_invocations.len(),
            estimated_bytes,
            terminal_estimated_bytes: estimated_bytes,
            terminal_invocations,
            oldest_at: self.oldest_at,
            ..Self::default()
        }
    }

    fn take_p1_terminal_chunk(&mut self, max_rows: usize, max_bytes: usize) -> Self {
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
        let mut chunk = Self {
            terminal_invocations,
            oldest_at: self.oldest_at,
            retained_for_retry: self.retained_for_retry,
            ..Self::default()
        };
        chunk.recalculate_estimates();
        chunk.enqueued_rows = chunk.logical_rows();
        self.enqueued_rows = self.logical_rows();
        if self.is_empty() {
            self.oldest_at = None;
        }
        chunk
    }

    fn take_p2(&mut self) -> Self {
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

    fn take_p2_chunk(&mut self, max_rows: usize, max_bytes: usize) -> Self {
        if !self.has_p2() || max_rows == 0 || max_bytes == 0 {
            return Self::default();
        }

        let oldest_at = self.oldest_at;
        let mut chunk = Self {
            oldest_at,
            retained_for_retry: self.retained_for_retry,
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
        }
        chunk
    }

    fn add_startup_backfill_wake_tasks(&mut self, tasks: &[StartupBackfillTask]) {
        for task in tasks {
            if !self.startup_backfill_wake_tasks.contains(task) {
                self.startup_backfill_wake_tasks.push(*task);
            }
        }
        self.recalculate_estimates();
    }

    fn merge_p2(&mut self, mut other: Self) {
        self.attempt_progress.extend(other.attempt_progress.drain());
        self.invocation_derived.extend(other.invocation_derived);
        self.account_selected_touches
            .extend(other.account_selected_touches.drain());
        self.system_task_finishes
            .extend(other.system_task_finishes.drain());
        self.add_startup_backfill_wake_tasks(&other.startup_backfill_wake_tasks);
        self.enqueued_rows = self.enqueued_rows.saturating_add(other.enqueued_rows);
        self.coalesced_rows = self.coalesced_rows.saturating_add(other.coalesced_rows);
        self.recalculate_estimates();
        self.oldest_at = match (self.oldest_at, other.oldest_at) {
            (Some(current), Some(other)) => Some(current.min(other)),
            (current, other) => current.or(other),
        };
    }

    fn merge_all(&mut self, mut other: Self) {
        let terminal_invocations = std::mem::take(&mut other.terminal_invocations);
        let terminal_count = terminal_invocations.len();
        for terminal in terminal_invocations.into_values() {
            self.push(SqliteBatchWrite::TerminalInvocation(terminal));
        }
        other.enqueued_rows = other.enqueued_rows.saturating_sub(terminal_count);
        self.merge_p2(other);
    }

    #[cfg(test)]
    fn into_writes(self) -> Vec<SqliteBatchWrite> {
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

#[derive(Debug)]
pub(crate) struct RetainedBatch {
    batch: PendingBatch,
    failed: bool,
    p2_retryable_failure: bool,
    p2_lock_failure: bool,
    p2_defer: Option<P2DeferReason>,
    quarantined_system_task_ids: Vec<i64>,
}

impl RetainedBatch {
    fn new(mut batch: PendingBatch, failed: bool) -> Self {
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

    fn p2_deferred(mut batch: PendingBatch, reason: P2DeferReason) -> Self {
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

    fn p2_failed(mut batch: PendingBatch, retryable_failure: bool, lock_failure: bool) -> Self {
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

    fn with_quarantined_system_task_ids(mut self, ids: Vec<i64>) -> Self {
        self.quarantined_system_task_ids = ids;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum P2DeferReason {
    PressureCooldown(u64),
    BackgroundBusy { observed_generation: u64 },
}

fn estimated_option_string_bytes(value: &Option<String>) -> usize {
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

fn estimated_attempt_progress_memory_bytes(progress: &BatchedAttemptProgress) -> usize {
    std::mem::size_of::<BatchedAttemptProgress>()
        .saturating_add(progress.phase.capacity())
        .saturating_add(estimated_option_string_bytes(
            &progress.compact_support_status,
        ))
        .saturating_add(estimated_option_string_bytes(
            &progress.compact_support_reason,
        ))
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
    fn estimated_memory_bytes(&self) -> usize {
        self.estimated_bytes
    }
}

#[derive(Debug)]
pub(crate) struct SqliteBatchWriter {
    write_sender: mpsc::Sender<SqliteBatchWrite>,
    queued_p1_count: Arc<AtomicUsize>,
    p1_priority_gate: Arc<std::sync::Mutex<()>>,
    control_sender: mpsc::Sender<SqliteBatchWriterControl>,
    accounting: Arc<PendingQueueAccounting>,
    dropped_writes: Arc<AtomicU64>,
    terminal_runtime_store: Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache:
        Arc<std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>>,
    summary_delta_hub: Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    terminal_projection_hub: Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    terminal_journal: Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    database_path: std::path::PathBuf,
    dashboard_reconcile_gate: Arc<Mutex<()>>,
    #[cfg(test)]
    prompt_cache_conversation_cache: Option<Arc<Mutex<PromptCacheConversationsCacheState>>>,
    handle: Mutex<Option<JoinHandle<()>>>,
    journal_sync_shutdown: CancellationToken,
    journal_sync_handle: Mutex<Option<JoinHandle<()>>>,
    #[cfg(test)]
    buffered_writes: Option<Arc<std::sync::Mutex<Vec<SqliteBatchWrite>>>>,
    #[cfg(test)]
    auto_flush_terminal_for_test: std::sync::atomic::AtomicBool,
}
