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

#[cfg(test)]
macro_rules! flush_dependencies {
    ($($dependency:expr),+ $(,)?) => {
        SqliteBatchFlushDependencies($($dependency),+)
    };
}

#[cfg(test)]
macro_rules! flush_pending_batch_inner {
    ($pool:expr, $batch:expr, $pricing_catalog:expr, $prompt_cache_conversation_cache:expr,
     $terminal_runtime_store:expr, $dashboard_activity_snapshot_cache:expr,
     $summary_delta_hub:expr, $terminal_projection_hub:expr,
     $dashboard_reconcile_gate:expr $(,)?) => {
        flush_pending_batch_inner(
            $pool,
            $batch,
            $pricing_catalog,
            flush_dependencies!(
                $prompt_cache_conversation_cache,
                $terminal_runtime_store,
                $dashboard_activity_snapshot_cache,
                $summary_delta_hub,
                $terminal_projection_hub,
                $dashboard_reconcile_gate,
                &std::sync::Arc::new(std::sync::Mutex::new(None)),
            ),
        )
    };
}

impl SqliteBatchWriter {
    pub(crate) fn spawn(
        pool: Pool<Sqlite>,
        shutdown: CancellationToken,
        prompt_cache_conversation_cache: Arc<Mutex<PromptCacheConversationsCacheState>>,
        pricing_catalog: Arc<RwLock<PricingCatalog>>,
        database_path: &Path,
    ) -> Arc<Self> {
        let (write_sender, write_receiver) = mpsc::channel(SQLITE_BATCH_CHANNEL_CAPACITY);
        let queued_p1_count = Arc::new(AtomicUsize::new(0));
        let p1_priority_gate = Arc::new(std::sync::Mutex::new(()));
        let (control_sender, control_receiver) = mpsc::channel(128);
        let accounting = Arc::new(PendingQueueAccounting::default());
        let dropped_writes = Arc::new(AtomicU64::new(0));
        let terminal_runtime_store = Arc::new(std::sync::Mutex::new(None));
        let dashboard_activity_snapshot_cache = Arc::new(std::sync::Mutex::new(None));
        let summary_delta_hub = Arc::new(std::sync::Mutex::new(None));
        let terminal_projection_hub = Arc::new(std::sync::Mutex::new(None));
        let terminal_journal = match TerminalJournal::open(database_path) {
            Ok(journal) => Some(journal),
            Err(err) => {
                warn!(error = %err, path = %database_path.display(), "terminal journal unavailable; using memory durability fallback");
                None
            }
        };
        let replay_writes = terminal_journal
            .as_ref()
            .map(|journal| journal.stats().replay_count)
            .unwrap_or_default();
        let mut terminal_journal = terminal_journal;
        if let Some(journal) = terminal_journal.as_mut() {
            journal.queue_replay_for_dispatch(SQLITE_BATCH_MAX_ROWS);
        }
        let terminal_journal = Arc::new(std::sync::Mutex::new(terminal_journal));
        queued_p1_count.store(replay_writes, Ordering::SeqCst);
        let dashboard_reconcile_gate = Arc::new(Mutex::new(()));
        let journal_sync_shutdown = shutdown.child_token();
        #[cfg(not(test))]
        let journal_sync_handle = Some(tokio::spawn(run_terminal_journal_sync(
            terminal_journal.clone(),
            journal_sync_shutdown.clone(),
        )));
        #[cfg(test)]
        let journal_sync_handle = None;
        let cache_for_task = prompt_cache_conversation_cache.clone();
        let handle = tokio::spawn(run_sqlite_batch_writer(SqliteBatchWriterConfig {
            pool,
            database_path: database_path.to_path_buf(),
            write_receiver,
            control_receiver,
            accounting: accounting.clone(),
            prompt_cache_conversation_cache: Some(cache_for_task),
            pricing_catalog: Some(pricing_catalog),
            terminal_runtime_store: terminal_runtime_store.clone(),
            dashboard_activity_snapshot_cache: dashboard_activity_snapshot_cache.clone(),
            summary_delta_hub: summary_delta_hub.clone(),
            terminal_projection_hub: terminal_projection_hub.clone(),
            dashboard_reconcile_gate: dashboard_reconcile_gate.clone(),
            terminal_journal: terminal_journal.clone(),
            queued_p1_count: queued_p1_count.clone(),
            p1_priority_gate: p1_priority_gate.clone(),
        }));
        let writer = Arc::new(Self {
            write_sender,
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
        });
        if replay_writes > 0 {
            warn!(
                replay_count = replay_writes,
                "requeued terminal journal records during startup"
            );
        }
        writer
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
            queued_p1_count: Arc::new(AtomicUsize::new(0)),
            p1_priority_gate: Arc::new(std::sync::Mutex::new(())),
            control_sender,
            accounting: Arc::new(PendingQueueAccounting::default()),
            dropped_writes: Arc::new(AtomicU64::new(0)),
            terminal_runtime_store: Arc::new(std::sync::Mutex::new(None)),
            dashboard_activity_snapshot_cache: Arc::new(std::sync::Mutex::new(None)),
            summary_delta_hub: Arc::new(std::sync::Mutex::new(None)),
            terminal_projection_hub: Arc::new(std::sync::Mutex::new(None)),
            terminal_journal: Arc::new(std::sync::Mutex::new(None)),
            database_path: std::path::PathBuf::from("test-sqlite-batch-writer.db"),
            dashboard_reconcile_gate: Arc::new(Mutex::new(())),
            prompt_cache_conversation_cache: Some(prompt_cache_conversation_cache),
            handle: Mutex::new(None),
            journal_sync_shutdown: CancellationToken::new(),
            journal_sync_handle: Mutex::new(None),
            buffered_writes: Some(Arc::new(std::sync::Mutex::new(Vec::new()))),
            auto_flush_terminal_for_test: std::sync::atomic::AtomicBool::new(true),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_auto_flush_terminal_for_test(&self, enabled: bool) {
        self.auto_flush_terminal_for_test
            .store(enabled, Ordering::Relaxed);
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
        let estimated_bytes = write.estimated_memory_bytes();
        let is_p1 = is_p1_terminal_write(&write);
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
                    return true;
                }
                Err(err) => {
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

    pub(crate) fn enqueue_terminal(
        &self,
        terminal: BatchedTerminalInvocationWrite,
    ) -> TerminalEnqueueOutcome {
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

        let _p1_priority_guard = self
            .p1_priority_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let recovery_terminal = terminal.clone();
        let journal = self
            .terminal_journal
            .lock()
            .ok()
            .and_then(|mut journal| {
                journal.as_mut().map(|journal| {
                    journal.append(
                        &terminal.record,
                        terminal.raw_capture,
                        terminal.capture_started,
                    )
                })
            })
            .unwrap_or(TerminalJournalAppendOutcome {
                durability_mode: TerminalJournalDurabilityMode::MemoryOverflow,
                sequence: None,
                pending_records: 0,
                pending_bytes: 0,
            });
        let enqueued = self.enqueue_terminal_write(
            SqliteBatchWrite::TerminalInvocation(terminal),
            journal.durability_mode,
        );
        if matches!(
            journal.durability_mode,
            TerminalJournalDurabilityMode::MemoryOverflow
        ) {
            self.recover_terminal_journal_memory_overflow(&recovery_terminal);
        }
        TerminalEnqueueOutcome {
            enqueued,
            durability_mode: journal.durability_mode,
            journal_sequence: journal.sequence,
            journal_pending_records: journal.pending_records,
            journal_pending_bytes: journal.pending_bytes,
        }
    }

    fn recover_terminal_journal_memory_overflow(
        &self,
        recovery_terminal: &BatchedTerminalInvocationWrite,
    ) {
        let terminals = [recovery_terminal];
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

    fn enqueue_terminal_write(
        &self,
        write: SqliteBatchWrite,
        durability_mode: TerminalJournalDurabilityMode,
    ) -> bool {
        let estimated_bytes = write.estimated_memory_bytes();
        let is_p1 = is_p1_terminal_write(&write);
        self.accounting.enqueue(estimated_bytes);
        if is_p1 {
            self.queued_p1_count.fetch_add(1, Ordering::SeqCst);
        }
        match self.write_sender.try_send(write) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(write)) => {
                self.accounting.rollback_enqueue(estimated_bytes);
                let deferred = matches!(durability_mode, TerminalJournalDurabilityMode::Journal)
                    && self
                        .terminal_journal
                        .lock()
                        .ok()
                        .and_then(|mut journal| {
                            journal.as_mut().map(|journal| match write {
                                SqliteBatchWrite::TerminalInvocation(terminal) => {
                                    journal.defer_write(terminal)
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
                if is_p1 {
                    decrement_queued_p1_count(&self.queued_p1_count);
                }
                self.dropped_writes.fetch_add(1, Ordering::Relaxed);
                warn!("terminal journal-backed retry could not reach closed sqlite batch writer");
                false
            }
        }
    }

    pub(crate) fn terminal_journal_stats(&self) -> TerminalJournalStats {
        self.terminal_journal
            .lock()
            .ok()
            .and_then(|journal| journal.as_ref().map(TerminalJournal::stats))
            .unwrap_or_default()
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

    pub(crate) fn accounting_snapshot(&self) -> PendingQueueAccountingSnapshot {
        self.accounting.snapshot()
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
        self.stop_terminal_journal_sync().await;
    }

    async fn stop_terminal_journal_sync(&self) {
        self.journal_sync_shutdown.cancel();
        let Some(mut handle) = self.journal_sync_handle.lock().await.take() else {
            return;
        };
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
        let deferred = flush_pending_batch_inner!(
            pool,
            &batch,
            pricing_catalog.as_ref(),
            None,
            &terminal_runtime_store,
            &dashboard_activity_snapshot_cache,
            &summary_delta_hub,
            &terminal_projection_hub,
            &dashboard_reconcile_gate,
        )
        .await
        .expect("flush pending sqlite batch writes");
        if !deferred.is_empty() {
            flush_pending_batch_inner!(
                pool,
                &deferred,
                pricing_catalog.as_ref(),
                None,
                &terminal_runtime_store,
                &dashboard_activity_snapshot_cache,
                &summary_delta_hub,
                &terminal_projection_hub,
                &dashboard_reconcile_gate,
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
            let submitted_count = writes.len();
            let submitted_bytes = writes
                .iter()
                .map(SqliteBatchWrite::estimated_memory_bytes)
                .sum::<usize>();
            let mut batch = PendingBatch::default();
            for write in writes {
                batch.push(write);
            }
            let deferred = flush_pending_batch_inner!(
                pool,
                &batch,
                None,
                self.prompt_cache_conversation_cache.as_ref(),
                &self.terminal_runtime_store,
                &self.dashboard_activity_snapshot_cache,
                &self.summary_delta_hub,
                &self.terminal_projection_hub,
                &self.dashboard_reconcile_gate,
            )
            .await
            .expect("flush buffered sqlite batch writes for test");
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
            self.accounting
                .complete(submitted_count, retained.0, submitted_bytes, retained.1);
        }
    }
}
