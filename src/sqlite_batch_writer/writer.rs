use super::*;

impl SqliteBatchWriter {
    pub(crate) fn bind_observability(&self, metrics: Arc<ObservabilityRuntime>) {
        self.accounting.observability.bind(metrics);
    }
    pub(crate) fn spawn(
        pool: Pool<Sqlite>,
        shutdown: CancellationToken,
        prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
        pricing_catalog: Arc<RwLock<PricingCatalog>>,
        database_path: &Path,
    ) -> Result<Arc<Self>> {
        let (write_sender, write_receiver) = mpsc::channel(SQLITE_BATCH_CHANNEL_CAPACITY);
        let queued_p1_count = Arc::new(AtomicUsize::new(0));
        let p1_priority_gate = Arc::new(std::sync::Mutex::new(()));
        let reliable_attempt_progress_overflow =
            Arc::new(std::sync::Mutex::new(PendingBatch::default()));
        let reliable_attempt_progress_notify = Arc::new(Notify::new());
        let (control_sender, control_receiver) = mpsc::channel(128);
        let accounting = Arc::new(PendingQueueAccounting::default());
        let dropped_writes = Arc::new(AtomicU64::new(0));
        let terminal_runtime_store = Arc::new(std::sync::Mutex::new(None));
        let dashboard_activity_snapshot_cache = Arc::new(std::sync::Mutex::new(None));
        let summary_delta_hub = Arc::new(std::sync::Mutex::new(None));
        let terminal_projection_hub = Arc::new(std::sync::Mutex::new(None));
        let terminal_journal = Some(
            TerminalJournal::open(database_path)
                .context("terminal journal recovery failed; refusing memory durability fallback")?,
        );
        let replay_writes = terminal_journal
            .as_ref()
            .map(|journal| journal.stats().replay_count)
            .unwrap_or_default();
        let terminal_journal = terminal_journal;
        let terminal_journal = Arc::new(TrackedTerminalJournal::new(terminal_journal));
        let dashboard_reconcile_gate = Arc::new(Mutex::new(()));
        let journal_sync_shutdown = shutdown.child_token();
        let journal_sync_handle = Some(tokio::spawn(run_terminal_journal_sync(
            terminal_journal.clone(),
            journal_sync_shutdown.clone(),
        )));
        let cache_for_task = prompt_cache_conversation_cache.clone();
        let handle = tokio::spawn(run_sqlite_batch_writer(
            pool,
            database_path.to_path_buf(),
            write_receiver,
            control_receiver,
            accounting.clone(),
            Some(cache_for_task),
            Some(pricing_catalog),
            terminal_runtime_store.clone(),
            dashboard_activity_snapshot_cache.clone(),
            summary_delta_hub.clone(),
            terminal_projection_hub.clone(),
            dashboard_reconcile_gate.clone(),
            terminal_journal.clone(),
            queued_p1_count.clone(),
            p1_priority_gate.clone(),
            reliable_attempt_progress_overflow.clone(),
            reliable_attempt_progress_notify.clone(),
        ));
        let writer = Arc::new(Self {
            write_sender,
            attempt_progress_sequence: AtomicU64::new(0),
            attempt_progress_send_gate: std::sync::Mutex::new(()),
            reliable_attempt_progress_overflow,
            reliable_attempt_progress_notify,
            queued_p1_count,
            p1_priority_gate,
            control_sender,
            accounting,
            dropped_writes,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            summary_delta_hub,
            terminal_projection_hub,
            terminal_journal,
            database_path: database_path.to_path_buf(),
            dashboard_reconcile_gate,
            #[cfg(test)]
            prompt_cache_conversation_cache: Some(prompt_cache_conversation_cache),
            handle: Mutex::new(Some(handle)),
            journal_sync_shutdown,
            journal_sync_handle: Mutex::new(journal_sync_handle),
            #[cfg(test)]
            buffered_writes: None,
            #[cfg(test)]
            auto_flush_terminal_for_test: std::sync::atomic::AtomicBool::new(true),
            #[cfg(test)]
            runtime_terminal_confirmation_count: AtomicUsize::new(0),
            #[cfg(test)]
            runtime_terminal_confirmation_gate: Mutex::new(()),
        });
        if replay_writes > 0 {
            warn!(
                replay_count = replay_writes,
                "deferred terminal journal replay until HTTP readiness"
            );
        }
        Ok(writer)
    }

    #[cfg(test)]
    pub(crate) fn spawn_for_test() -> Arc<Self> {
        Self::spawn_for_test_with_prompt_cache(Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )))
    }

    #[cfg(test)]
    pub(crate) fn spawn_for_test_with_prompt_cache(
        prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
    ) -> Arc<Self> {
        let (write_sender, _write_receiver) = mpsc::channel(1);
        let (control_sender, _control_receiver) = mpsc::channel(1);
        Arc::new(Self {
            write_sender,
            attempt_progress_sequence: AtomicU64::new(0),
            attempt_progress_send_gate: std::sync::Mutex::new(()),
            reliable_attempt_progress_overflow: Arc::new(std::sync::Mutex::new(
                PendingBatch::default(),
            )),
            reliable_attempt_progress_notify: Arc::new(Notify::new()),
            queued_p1_count: Arc::new(AtomicUsize::new(0)),
            p1_priority_gate: Arc::new(std::sync::Mutex::new(())),
            control_sender,
            accounting: Arc::new(PendingQueueAccounting::default()),
            dropped_writes: Arc::new(AtomicU64::new(0)),
            terminal_runtime_store: Arc::new(std::sync::Mutex::new(None)),
            dashboard_activity_snapshot_cache: Arc::new(std::sync::Mutex::new(None)),
            summary_delta_hub: Arc::new(std::sync::Mutex::new(None)),
            terminal_projection_hub: Arc::new(std::sync::Mutex::new(None)),
            terminal_journal: Arc::new(TrackedTerminalJournal::new(None)),
            database_path: std::path::PathBuf::from("test-sqlite-batch-writer.db"),
            dashboard_reconcile_gate: Arc::new(Mutex::new(())),
            prompt_cache_conversation_cache: Some(prompt_cache_conversation_cache),
            handle: Mutex::new(None),
            journal_sync_shutdown: CancellationToken::new(),
            journal_sync_handle: Mutex::new(None),
            buffered_writes: Some(Arc::new(std::sync::Mutex::new(Vec::new()))),
            auto_flush_terminal_for_test: std::sync::atomic::AtomicBool::new(true),
            runtime_terminal_confirmation_count: AtomicUsize::new(0),
            runtime_terminal_confirmation_gate: Mutex::new(()),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_auto_flush_terminal_for_test(&self, enabled: bool) {
        self.auto_flush_terminal_for_test
            .store(enabled, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn runtime_terminal_confirmation_count_for_test(&self) -> usize {
        self.runtime_terminal_confirmation_count
            .load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn fail_runtime_journal_for_test(&self) {
        self.terminal_journal
            .lock()
            .expect("test journal lock")
            .as_mut()
            .expect("test file journal")
            .fail_append_for_test();
    }

    #[cfg(test)]
    pub(crate) async fn pause_runtime_terminal_confirmation_for_test(
        &self,
    ) -> tokio::sync::MutexGuard<'_, ()> {
        self.runtime_terminal_confirmation_gate.lock().await
    }

    #[cfg(test)]
    pub(crate) fn auto_flush_terminal_for_test(&self) -> bool {
        self.auto_flush_terminal_for_test.load(Ordering::Relaxed)
    }

    pub(crate) fn set_terminal_runtime_store(
        &self,
        runtime_store: Arc<ProxyRuntimeInvocationStore>,
    ) {
        if let Ok(mut guard) = self.terminal_runtime_store.lock() {
            *guard = Some(runtime_store);
        }
    }

    pub(crate) fn set_runtime_attempt_store(&self, store: Arc<RuntimeAttemptStore>) {
        self.terminal_journal.bind_runtime_attempts(store);
    }

    pub(crate) fn set_dashboard_activity_snapshot_cache(
        &self,
        cache: Arc<Mutex<DashboardActivitySnapshotCacheState>>,
    ) {
        if let Ok(mut guard) = self.dashboard_activity_snapshot_cache.lock() {
            *guard = Some(cache);
        }
    }

    pub(crate) fn set_summary_delta_hub(&self, hub: Arc<SubscriptionHub>) {
        if let Ok(mut guard) = self.summary_delta_hub.lock() {
            *guard = Some(hub);
        }
    }

    pub(crate) fn set_terminal_projection_hub(&self, hub: Arc<TerminalProjectionHub>) {
        if let Ok(mut guard) = self.terminal_projection_hub.lock() {
            *guard = Some(hub);
        }
    }

    pub(crate) fn dashboard_reconcile_gate(&self) -> Arc<Mutex<()>> {
        self.dashboard_reconcile_gate.clone()
    }

    pub(crate) fn enqueue(&self, write: SqliteBatchWrite) -> bool {
        let is_p1 = is_p1_terminal_write(&write);
        let is_attempt_progress = is_attempt_progress_write(&write);
        let _attempt_progress_send_guard = is_attempt_progress.then(|| {
            self.attempt_progress_send_gate
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        });
        let write = self.stamp_attempt_progress(write);
        let estimated_bytes = write.estimated_memory_bytes();
        if is_attempt_progress {
            self.accounting.attempt_progress_enqueued();
        }
        let _p1_priority_guard = is_p1.then(|| {
            self.p1_priority_gate
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        });
        #[cfg(test)]
        if let Some(buffered_writes) = &self.buffered_writes {
            match buffered_writes.lock() {
                Ok(mut guard) => {
                    guard.push(write);
                    self.accounting.enqueue(estimated_bytes);
                    self.accounting.observe_p1_replace(0, usize::from(is_p1));
                    return true;
                }
                Err(err) => {
                    if is_attempt_progress {
                        self.accounting.attempt_progress_dropped();
                    }
                    self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        error = %err,
                        dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                        "sqlite batch writer test buffer poisoned; dropped derived write"
                    );
                    return false;
                }
            }
        }

        self.accounting.enqueue(estimated_bytes);
        self.accounting.observe_p1_replace(0, usize::from(is_p1));
        if is_p1 {
            self.queued_p1_count.fetch_add(1, Ordering::SeqCst);
        }
        match self.write_sender.try_send(write) {
            Ok(()) => true,
            Err(err) => {
                if is_p1 {
                    decrement_queued_p1_count(&self.queued_p1_count);
                }
                self.accounting.rollback_enqueue(estimated_bytes);
                self.accounting.observe_p1_replace(usize::from(is_p1), 0);
                if is_attempt_progress {
                    self.accounting.attempt_progress_dropped();
                }
                self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                warn!(
                    error = %err,
                    queue_depth = self.accounting.snapshot().pending_depth,
                    dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                    "sqlite batch writer queue full; dropped derived write"
                );
                false
            }
        }
    }

    pub(super) fn stamp_attempt_progress(&self, write: SqliteBatchWrite) -> SqliteBatchWrite {
        match write {
            SqliteBatchWrite::AttemptProgress(mut progress) => {
                let sequence = self
                    .attempt_progress_sequence
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                progress.stamp_sequence(sequence);
                SqliteBatchWrite::AttemptProgress(progress)
            }
            other => other,
        }
    }

    pub(crate) fn enqueue_terminal(
        &self,
        mut terminal: BatchedTerminalInvocationWrite,
    ) -> TerminalEnqueueOutcome {
        terminal.enqueued_at = Some(Instant::now());
        self.terminal_journal.pending.register(
            &terminal.record.invoke_id,
            &terminal.record.occurred_at,
            terminal.raw_capture,
        );
        #[cfg(test)]
        if self.buffered_writes.is_some() {
            let enqueued = self.enqueue(SqliteBatchWrite::TerminalInvocation(terminal));
            return TerminalEnqueueOutcome {
                enqueued,
                durability_mode: TerminalJournalDurabilityMode::MemoryOverflow,
                journal_sequence: None,
                journal_pending_records: 0,
                journal_pending_bytes: 0,
            };
        }

        let _p1_priority_guard = diagnostics::lock_priority_gate(&self.p1_priority_gate, &terminal);
        let recovery_terminal = terminal.clone();
        let journal = append_terminal_journal(&self.terminal_journal, &terminal);
        if matches!(
            journal.durability_mode,
            TerminalJournalDurabilityMode::Journal
        ) && journal.sequence.is_some()
        {
            let sync_result = self
                .terminal_journal
                .lock()
                .ok()
                .and_then(|mut guard| guard.as_mut().map(TerminalJournal::force_sync))
                .unwrap_or_else(|| Err(anyhow::anyhow!("terminal journal is unavailable")));
            if let Err(err) = sync_result {
                warn!(
                    error = %err,
                    invoke_id = %terminal.record.invoke_id,
                    occurred_at = %terminal.record.occurred_at,
                    "terminal journal durability acknowledgement failed"
                );
                let recovery_persisted = self
                    .terminal_journal
                    .lock()
                    .ok()
                    .and_then(|mut guard| {
                        guard.as_mut().map(|journal| {
                            journal.remember_shutdown_recovery(&[&recovery_terminal]);
                            journal
                                .quarantine_shutdown_batch(
                                    &[&recovery_terminal],
                                    &[],
                                    "terminal journal sync failed",
                                )
                                .is_ok()
                        })
                    })
                    .unwrap_or(false);
                if !recovery_persisted {
                    self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                }
                return TerminalEnqueueOutcome {
                    enqueued: false,
                    durability_mode: journal.durability_mode,
                    journal_sequence: journal.sequence,
                    journal_pending_records: journal.pending_records,
                    journal_pending_bytes: journal.pending_bytes,
                };
            }
        }
        let enqueued = if matches!(
            journal.durability_mode,
            TerminalJournalDurabilityMode::MemoryOverflow
        ) {
            false
        } else {
            self.enqueue_terminal_write(
                SqliteBatchWrite::TerminalInvocation(terminal),
                journal.durability_mode,
            )
        };
        if matches!(
            journal.durability_mode,
            TerminalJournalDurabilityMode::MemoryOverflow
        ) {
            let terminals = [&recovery_terminal];
            let mut recovery_persisted = false;
            if let Ok(mut guard) = self.terminal_journal.lock()
                && let Some(journal) = guard.as_mut()
            {
                match journal.quarantine_shutdown_batch(
                    &terminals,
                    &[],
                    "terminal journal memory-overflow recovery",
                ) {
                    Ok(()) => {
                        journal.remember_shutdown_recovery(&terminals);
                        recovery_persisted = true;
                    }
                    Err(err) => {
                        warn!(
                            error = %err,
                            invoke_id = %recovery_terminal.record.invoke_id,
                            occurred_at = %recovery_terminal.record.occurred_at,
                            "terminal memory-overflow recovery sink failed"
                        );
                        if TerminalJournal::quarantine_shutdown_batch_at_database_path(
                            &self.database_path,
                            &terminals,
                            &[],
                            "terminal journal memory-overflow recovery",
                        )
                        .is_ok()
                        {
                            journal.remember_shutdown_recovery(&terminals);
                            recovery_persisted = true;
                        }
                    }
                }
            }
            if !recovery_persisted
                && let Err(err) = TerminalJournal::quarantine_shutdown_batch_at_database_path(
                    &self.database_path,
                    &terminals,
                    &[],
                    "terminal journal memory-overflow recovery",
                )
            {
                warn!(
                    error = %err,
                    invoke_id = %recovery_terminal.record.invoke_id,
                    occurred_at = %recovery_terminal.record.occurred_at,
                    "terminal memory-overflow recovery sink failed"
                );
            }
        }
        TerminalEnqueueOutcome {
            enqueued,
            durability_mode: journal.durability_mode,
            journal_sequence: journal.sequence,
            journal_pending_records: journal.pending_records,
            journal_pending_bytes: journal.pending_bytes,
        }
    }

    pub(crate) async fn wait_for_runtime_journal(&self, sequence: Option<u64>) -> Result<()> {
        let Some(sequence) = sequence else {
            return Ok(());
        };
        loop {
            let synced = {
                let mut guard = self
                    .terminal_journal
                    .lock()
                    .map_err(|_| anyhow!("runtime journal lock poisoned"))?;
                let journal = guard.as_mut().context("runtime journal unavailable")?;
                if self.journal_sync_shutdown.is_cancelled() {
                    journal.force_sync()?;
                }
                #[cfg(test)]
                if self.auto_flush_terminal_for_test() {
                    journal.force_sync()?;
                }
                // The background group-commit task is intentionally best-effort. Under a
                // saturated executor, a terminal producer must still be able to advance the
                // same 20ms journal group without turning an already appended fact into an
                // unconfirmed request outcome.
                if !self.journal_sync_shutdown.is_cancelled() {
                    let _ = journal.sync_if_due();
                }
                journal.sequence_is_synced(sequence)?
            };
            if synced {
                return Ok(());
            }
            tokio::time::sleep(crate::terminal_journal::TERMINAL_JOURNAL_SYNC_INTERVAL).await;
        }
    }

    pub(crate) async fn enqueue_runtime_terminal(
        &self,
        mut terminal: BatchedTerminalInvocationWrite,
    ) -> TerminalEnqueueOutcome {
        #[cfg(test)]
        if self.buffered_writes.is_some() {
            return self.enqueue_terminal(terminal);
        }
        terminal.enqueued_at = Some(Instant::now());
        let journal = append_terminal_journal(&self.terminal_journal, &terminal);
        let accepted = matches!(
            journal.durability_mode,
            TerminalJournalDurabilityMode::Journal
        ) && journal.sequence.is_some();
        #[cfg(test)]
        drop(self.runtime_terminal_confirmation_gate.lock().await);
        let confirmed = accepted
            && self
                .wait_for_runtime_journal(journal.sequence)
                .await
                .is_ok();
        if !confirmed {
            self.preserve_unconfirmed_runtime_terminal(&terminal, accepted);
        }
        let enqueued = if confirmed {
            self.terminal_journal.pending.register(
                &terminal.record.invoke_id,
                &terminal.record.occurred_at,
                terminal.raw_capture,
            );
            self.enqueue_terminal_write(
                SqliteBatchWrite::TerminalInvocation(terminal),
                journal.durability_mode,
            )
        } else {
            false
        };
        #[cfg(test)]
        if enqueued {
            // Append counters include producers still awaiting synchronization; publish only
            // after both durable confirmation and materializer admission have completed.
            self.runtime_terminal_confirmation_count
                .fetch_add(1, Ordering::Release);
        }
        TerminalEnqueueOutcome {
            enqueued,
            durability_mode: journal.durability_mode,
            journal_sequence: journal.sequence,
            journal_pending_records: journal.pending_records,
            journal_pending_bytes: journal.pending_bytes,
        }
    }

    fn enqueue_terminal_write(
        &self,
        write: SqliteBatchWrite,
        durability_mode: TerminalJournalDurabilityMode,
    ) -> bool {
        let estimated_bytes = write.estimated_memory_bytes();
        let is_p1 = is_p1_terminal_write(&write);
        self.accounting.enqueue(estimated_bytes);
        self.accounting.observe_p1_replace(0, usize::from(is_p1));
        if is_p1 {
            self.queued_p1_count.fetch_add(1, Ordering::SeqCst);
        }
        match self.write_sender.try_send(write) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(write)) => {
                self.accounting.rollback_enqueue(estimated_bytes);
                self.accounting.observe_p1_replace(usize::from(is_p1), 0);
                let deferred = matches!(durability_mode, TerminalJournalDurabilityMode::Journal)
                    && self
                        .terminal_journal
                        .lock()
                        .ok()
                        .and_then(|mut journal| {
                            journal.as_mut().map(|journal| match write {
                                SqliteBatchWrite::TerminalInvocation(terminal) => {
                                    let key = (
                                        terminal.record.invoke_id.clone(),
                                        terminal.record.occurred_at.clone(),
                                        terminal.raw_capture,
                                    );
                                    journal.defer_write(terminal)
                                        || journal.defer_from_log(&key.0, &key.1, key.2)
                                }
                                _ => false,
                            })
                        })
                        .unwrap_or(false);
                if is_p1 && !deferred {
                    decrement_queued_p1_count(&self.queued_p1_count);
                }
                if !deferred {
                    self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                    warn!(
                        "terminal journal-backed deferred queue could not accept sqlite batch write"
                    );
                }
                deferred
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.accounting.rollback_enqueue(estimated_bytes);
                self.accounting.observe_p1_replace(usize::from(is_p1), 0);
                if is_p1 {
                    decrement_queued_p1_count(&self.queued_p1_count);
                }
                self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                warn!("terminal journal-backed retry could not reach closed sqlite batch writer");
                false
            }
        }
    }

    pub(crate) fn terminal_materialization_snapshot(&self) -> (usize, Duration) {
        self.terminal_journal.pending.materialization_snapshot()
    }

    pub(crate) fn terminal_journal_stats(&self) -> TerminalJournalStats {
        self.terminal_journal
            .lock()
            .ok()
            .and_then(|journal| journal.as_ref().map(TerminalJournal::stats))
            .unwrap_or(TerminalJournalStats {
                unavailable: true,
                ..TerminalJournalStats::default()
            })
    }

    pub(crate) fn start_terminal_journal_recovery(&self) -> TerminalJournalStats {
        let stats = self.terminal_journal.lock().ok().and_then(|mut journal| {
            journal.as_mut().map(|journal| {
                journal.start_recovery_dispatch_p1_reservation(SQLITE_BATCH_MAX_ROWS)
            })
        });
        let replay_count = stats.unwrap_or_default();
        self.queued_p1_count
            .fetch_add(replay_count, Ordering::SeqCst);
        self.terminal_journal_stats()
    }

    pub(crate) fn append_runtime_attempt_start(
        &self,
        snapshot: &RuntimeAttemptSnapshot,
    ) -> TerminalJournalAppendOutcome {
        let _p1_priority_guard = self
            .p1_priority_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let outcome = self.terminal_journal.lock().ok().and_then(|mut guard| {
            guard
                .as_mut()
                .map(|journal| journal.append_attempt_start(snapshot))
        });
        match outcome {
            Some(outcome) => outcome,
            None => {
                #[cfg(test)]
                {
                    // Lightweight AppState fixtures use an in-memory writer without a
                    // file-backed journal. Production writers always open the journal and take
                    // the fail-closed branch below.
                    TerminalJournalAppendOutcome {
                        durability_mode: TerminalJournalDurabilityMode::Journal,
                        sequence: None,
                        pending_records: 0,
                        pending_bytes: 0,
                    }
                }
                #[cfg(not(test))]
                {
                    TerminalJournalAppendOutcome {
                        durability_mode: TerminalJournalDurabilityMode::MemoryOverflow,
                        sequence: None,
                        pending_records: 0,
                        pending_bytes: 0,
                    }
                }
            }
        }
    }

    pub(crate) fn reserve_runtime_attempt_completion_growth(
        &self,
        snapshot: &RuntimeAttemptSnapshot,
    ) -> bool {
        let Ok(mut guard) = self.terminal_journal.lock() else {
            return false;
        };
        if let Some(journal) = guard.as_mut() {
            journal.reserve_attempt_completion_growth(snapshot)
        } else {
            // Only lightweight fixtures have no file journal.
            cfg!(test)
        }
    }

    pub(crate) fn acknowledge_runtime_attempt_start(&self, attempt_public_id: &str) -> bool {
        let acknowledged = self
            .terminal_journal
            .lock()
            .ok()
            .and_then(|mut guard| {
                guard
                    .as_mut()
                    .map(|journal| journal.acknowledge_attempt_start(attempt_public_id))
            })
            .unwrap_or(false);
        if !acknowledged {
            return false;
        }
        self.terminal_journal
            .lock()
            .ok()
            .and_then(|mut guard| guard.as_mut().map(|journal| journal.force_sync().is_ok()))
            .unwrap_or(false)
    }

    pub(crate) fn telemetry_snapshot(&self) -> (usize, usize, u64) {
        let accounting = self.accounting.snapshot();
        (
            accounting.pending_depth,
            accounting.pending_bytes,
            self.dropped_writes.load(Ordering::Relaxed),
        )
    }

    pub(crate) fn quarantine_system_task_finish(
        &self,
        finish: &BatchedSystemTaskFinish,
        error: &str,
    ) -> bool {
        let persisted = self
            .terminal_journal
            .lock()
            .ok()
            .and_then(|mut guard| {
                guard
                    .as_mut()
                    .map(|journal| journal.quarantine_system_task_finish(finish, error).is_ok())
            })
            .unwrap_or(false);
        if persisted {
            return true;
        }
        #[cfg(test)]
        if self.buffered_writes.is_some() {
            // Test-only buffered writers intentionally omit the filesystem journal.
            return true;
        }
        false
    }

    pub(crate) fn observed_queue_depths(&self) -> (usize, usize) {
        self.accounting.observed_queue_depths()
    }

    pub(crate) fn accounting_snapshot(&self) -> PendingQueueAccountingSnapshot {
        self.accounting.snapshot()
    }

    pub(crate) fn queued_p1_count(&self) -> usize {
        self.queued_p1_count.load(Ordering::Relaxed)
    }

    pub(crate) async fn flush_now(&self, _pool: &Pool<Sqlite>) -> Result<()> {
        #[cfg(test)]
        if self.buffered_writes.is_some() {
            self.flush_buffered_for_test(_pool).await;
            return Ok(());
        }

        let (sender, receiver) = oneshot::channel();
        let queued_depth_snapshot = self
            .write_sender
            .max_capacity()
            .saturating_sub(self.write_sender.capacity());
        if let Err(err) = self
            .control_sender
            .try_send(SqliteBatchWriterControl::FlushNow {
                queued_depth_snapshot,
                responder: sender,
            })
        {
            self.dropped_writes.fetch_add(1, Ordering::Relaxed);
            warn!(
                error = %err,
                queue_depth = self.accounting.snapshot().pending_depth,
                dropped_writes = self.dropped_writes.load(Ordering::Relaxed),
                "sqlite batch writer flush barrier could not be queued"
            );
            return Err(anyhow::anyhow!(
                "sqlite batch writer flush barrier could not be queued"
            ));
        }
        match receiver.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(anyhow::anyhow!(err)),
            Err(err) => Err(anyhow::anyhow!(
                "sqlite batch writer flush barrier dropped: {err}"
            )),
        }
    }

    pub(crate) async fn shutdown_and_drain(&self) {
        #[cfg(test)]
        if let Some(buffered_writes) = &self.buffered_writes {
            let retained = buffered_writes
                .lock()
                .map(|guard| guard.len())
                .unwrap_or_default();
            if retained > 0 {
                warn!(
                    retained,
                    "sqlite batch writer test buffer was not explicitly flushed before shutdown"
                );
            }
            return;
        }

        let Some(handle) = self.handle.lock().await.take() else {
            return;
        };
        let shutdown_deadline = tokio::time::Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE;
        let (sender, receiver) = oneshot::channel();
        let queued_depth_snapshot = self
            .write_sender
            .max_capacity()
            .saturating_sub(self.write_sender.capacity());
        let barrier_sent = match timeout_at(
            shutdown_deadline,
            self.control_sender
                .send(SqliteBatchWriterControl::Shutdown {
                    queued_depth_snapshot,
                    responder: sender,
                }),
        )
        .await
        {
            Ok(Ok(())) => true,
            Ok(Err(err)) => {
                warn!(error = %err, "sqlite batch writer shutdown barrier could not be queued");
                false
            }
            Err(_) => {
                warn!("sqlite batch writer shutdown barrier queue timed out");
                false
            }
        };
        if barrier_sent {
            match timeout_at(shutdown_deadline, receiver).await {
                Ok(Ok(Err(err))) => {
                    warn!(error = %err, "sqlite batch writer shutdown drain failed");
                }
                Ok(Err(err)) => {
                    warn!(error = %err, "sqlite batch writer shutdown responder dropped");
                }
                Err(_) => {
                    warn!("sqlite batch writer shutdown drain timed out");
                }
                Ok(Ok(Ok(()))) => {}
            }
        }
        let mut handle = handle;
        let worker_deadline = tokio::time::Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE;
        let worker_timed_out = match timeout_at(worker_deadline, &mut handle).await {
            Ok(Ok(())) => false,
            Ok(Err(err)) => {
                warn!(error = %err, "sqlite batch writer task failed during shutdown");
                false
            }
            Err(_) => {
                warn!(
                    "sqlite batch writer task exceeded shutdown deadline; retaining worker ownership for recovery"
                );
                true
            }
        };
        if worker_timed_out {
            // Do not abort a worker that still owns an in-flight batch. The worker's own
            // bounded shutdown path will quarantine that batch; keeping the handle lets a
            // later shutdown attempt observe completion instead of losing memory-overflow P1.
            *self.handle.lock().await = Some(handle);
            return;
        }
        let (abandoned_depth, abandoned_bytes) = self.accounting.clear_after_shutdown();
        self.queued_p1_count.store(0, Ordering::SeqCst);
        if abandoned_depth > 0 || abandoned_bytes > 0 {
            warn!(
                abandoned_depth,
                abandoned_bytes,
                "sqlite batch writer cleared accounting for shutdown-owned queued work; journal/P2 recovery remains authoritative"
            );
        }
        self.journal_sync_shutdown.cancel();
        if let Some(mut handle) = self.journal_sync_handle.lock().await.take() {
            match timeout_at(
                tokio::time::Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE,
                &mut handle,
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    warn!(error = %err, "terminal journal sync task failed during shutdown")
                }
                Err(_) => {
                    warn!("terminal journal sync task exceeded shutdown deadline; aborting");
                    handle.abort();
                    let _ = handle.await;
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn stats_snapshot(&self) -> (usize, u64) {
        (
            self.accounting.snapshot().pending_depth,
            self.dropped_writes.load(Ordering::Relaxed),
        )
    }

    #[cfg(test)]
    pub(crate) async fn flush_for_test(pool: &Pool<Sqlite>, writes: Vec<SqliteBatchWrite>) {
        Self::flush_for_test_with_pricing_catalog(pool, None, writes).await;
    }

    #[cfg(test)]
    pub(crate) async fn flush_for_test_with_pricing_catalog(
        pool: &Pool<Sqlite>,
        pricing_catalog: Option<Arc<RwLock<PricingCatalog>>>,
        writes: Vec<SqliteBatchWrite>,
    ) {
        let mut batch = PendingBatch::default();
        for write in writes {
            batch.push(write);
        }
        let terminal_runtime_store = Arc::new(std::sync::Mutex::new(None));
        let dashboard_activity_snapshot_cache = Arc::new(std::sync::Mutex::new(None));
        let summary_delta_hub = Arc::new(std::sync::Mutex::new(None));
        let terminal_projection_hub = Arc::new(std::sync::Mutex::new(None));
        let dashboard_reconcile_gate = Arc::new(Mutex::new(()));
        let deferred = flush_pending_batch_inner(
            pool,
            &batch,
            pricing_catalog.as_ref(),
            None,
            &terminal_runtime_store,
            &dashboard_activity_snapshot_cache,
            &summary_delta_hub,
            &terminal_projection_hub,
            &dashboard_reconcile_gate,
            None,
        )
        .await
        .expect("flush pending sqlite batch writes");
        if !deferred.is_empty() {
            flush_pending_batch_inner(
                pool,
                &deferred,
                pricing_catalog.as_ref(),
                None,
                &terminal_runtime_store,
                &dashboard_activity_snapshot_cache,
                &summary_delta_hub,
                &terminal_projection_hub,
                &dashboard_reconcile_gate,
                None,
            )
            .await
            .expect("flush deferred pending sqlite batch writes");
        }
    }

    #[cfg(test)]
    pub(crate) async fn flush_buffered_for_test(&self, pool: &Pool<Sqlite>) {
        let writes = self
            .buffered_writes
            .as_ref()
            .and_then(|buffered_writes| {
                buffered_writes
                    .lock()
                    .ok()
                    .map(|mut guard| guard.drain(..).collect::<Vec<_>>())
            })
            .unwrap_or_default();

        if !writes.is_empty() {
            let submitted_p1_count = writes.iter().filter(|w| is_p1_terminal_write(w)).count();
            let submitted_count = writes.len();
            let submitted_bytes = writes
                .iter()
                .map(SqliteBatchWrite::estimated_memory_bytes)
                .sum::<usize>();
            let mut batch = PendingBatch::default();
            for write in writes {
                batch.push(write);
            }
            let deferred = flush_pending_batch_inner(
                pool,
                &batch,
                None,
                self.prompt_cache_conversation_cache.as_ref(),
                &self.terminal_runtime_store,
                &self.dashboard_activity_snapshot_cache,
                &self.summary_delta_hub,
                &self.terminal_projection_hub,
                &self.dashboard_reconcile_gate,
                None,
            )
            .await
            .expect("flush buffered sqlite batch writes for test");
            for terminal in batch.terminal_invocations.values() {
                if let Some(cache) = &self.prompt_cache_conversation_cache {
                    cache
                        .lock()
                        .await
                        .identity_cache
                        .range_manager
                        .reconcile_persistence(&terminal.record.invoke_id);
                }
                self.terminal_journal.acknowledge(
                    &terminal.record.invoke_id,
                    &terminal.record.occurred_at,
                    terminal.raw_capture,
                    &terminal.attempts,
                );
            }
            let deferred_writes = deferred.into_writes();
            let retained_count = deferred_writes.len();
            let retained_bytes = deferred_writes
                .iter()
                .map(SqliteBatchWrite::estimated_memory_bytes)
                .sum::<usize>();
            self.accounting.transfer_p1_to_p2(retained_bytes);
            let retained = self
                .buffered_writes
                .as_ref()
                .and_then(|buffered_writes| buffered_writes.lock().ok())
                .map(|mut guard| {
                    guard.extend(deferred_writes);
                    (retained_count, retained_bytes)
                })
                .unwrap_or_default();
            self.accounting.observe_p1_replace(submitted_p1_count, 0);
            self.accounting
                .complete(submitted_count, retained.0, submitted_bytes, retained.1);
        }
    }
}
