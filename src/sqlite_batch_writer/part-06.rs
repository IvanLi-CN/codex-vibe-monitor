struct SqliteBatchWriterLoop {
    pool: Pool<Sqlite>,
    database_path: std::path::PathBuf,
    write_receiver: mpsc::Receiver<SqliteBatchWrite>,
    control_receiver: mpsc::Receiver<SqliteBatchWriterControl>,
    accounting: Arc<PendingQueueAccounting>,
    prompt_cache_conversation_cache: Option<Arc<Mutex<PromptCacheConversationsCacheState>>>,
    pricing_catalog: Option<Arc<RwLock<PricingCatalog>>>,
    terminal_runtime_store: Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache:
        Arc<std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>>,
    summary_delta_hub: Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    terminal_projection_hub: Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: Arc<Mutex<()>>,
    terminal_journal: Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    queued_p1_count: Arc<AtomicUsize>,
    p1_priority_gate: Arc<std::sync::Mutex<()>>,
    ticker: tokio::time::Interval,
    deferred_ticker: tokio::time::Interval,
    pending: PendingBatch,
    control_closed: bool,
    p1_retry: P1RetryState,
    p2_schedule: P2ScheduleState,
    p2_eligibility_generation: u64,
    transaction_sequence: u64,
}

struct FlushBatchSummary {
    logical_rows: usize,
    failed: bool,
    p2_deferred: bool,
}

enum ShutdownBatchOutcome {
    Continue,
    Break,
}

impl SqliteBatchWriterLoop {
    fn from_config(config: SqliteBatchWriterConfig) -> Self {
        let SqliteBatchWriterConfig {
            pool,
            database_path,
            write_receiver,
            control_receiver,
            accounting,
            prompt_cache_conversation_cache,
            pricing_catalog,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            summary_delta_hub,
            terminal_projection_hub,
            dashboard_reconcile_gate,
            terminal_journal,
            queued_p1_count,
            p1_priority_gate,
        } = config;
        let mut ticker = interval(SQLITE_BATCH_FLUSH_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut deferred_ticker = interval(crate::terminal_journal::TERMINAL_JOURNAL_SYNC_INTERVAL);
        deferred_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        Self {
            pool,
            database_path,
            write_receiver,
            control_receiver,
            accounting,
            prompt_cache_conversation_cache,
            pricing_catalog,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            summary_delta_hub,
            terminal_projection_hub,
            dashboard_reconcile_gate,
            terminal_journal,
            queued_p1_count,
            p1_priority_gate,
            ticker,
            deferred_ticker,
            pending: PendingBatch::default(),
            control_closed: false,
            p1_retry: P1RetryState::default(),
            p2_schedule: P2ScheduleState::default(),
            p2_eligibility_generation: crate::db_pressure::global_db_pressure_gate()
                .eligibility_generation(),
            transaction_sequence: 0,
        }
    }

    fn flush_dependencies(&self) -> SqliteBatchFlushDependencies<'_> {
        SqliteBatchFlushDependencies(
            self.prompt_cache_conversation_cache.as_ref(),
            &self.terminal_runtime_store,
            &self.dashboard_activity_snapshot_cache,
            &self.summary_delta_hub,
            &self.terminal_projection_hub,
            &self.dashboard_reconcile_gate,
            &self.terminal_journal,
        )
    }

    async fn flush_batch(&self, batch: PendingBatch, reason: FlushReason) -> Option<RetainedBatch> {
        flush_pending_batch_accounted(
            &self.accounting,
            &self.pool,
            self.pricing_catalog.as_ref(),
            batch,
            reason,
            self.flush_dependencies(),
        )
        .await
    }

    async fn run(&mut self) {
        loop {
            drain_queued_writes_before_dispatch(
                &mut self.write_receiver,
                &mut self.pending,
                &self.accounting,
                &mut self.p2_schedule,
                SQLITE_BATCH_MAX_ROWS,
                &self.queued_p1_count,
            );
            tokio::select! {
                biased;
                _ = crate::db_pressure::global_db_pressure_gate()
                    .wait_for_eligibility_change(self.p2_eligibility_generation),
                    if self.pending.has_p2()
                        && self.p2_schedule.wake_reason == Some(P2WakeReason::BackgroundEligible) =>
                {
                    self.p2_eligibility_generation = crate::db_pressure::global_db_pressure_gate()
                        .eligibility_generation();
                    self.p2_schedule.wake_background_eligible();
                    self.accounting.update_p2_schedule(&self.p2_schedule);
                }
                maybe_control = self.control_receiver.recv(), if !self.control_closed => {
                    match maybe_control {
                        Some(control) => {
                            if self.handle_control(control).await {
                                return;
                            }
                        }
                        None => self.control_closed = true,
                    }
                }
                _ = wait_for_p2_deadline(self.p2_schedule.due_at),
                    if p2_deadline_wait_armed(
                        &self.pending,
                        &self.p2_schedule,
                        &self.queued_p1_count,
                        &self.p1_retry,
                    ) => self.handle_p2_deadline().await,
                _ = self.deferred_ticker.tick() => self.handle_deferred_tick(),
                maybe_write = self.write_receiver.recv() => {
                    if self.handle_write(maybe_write).await {
                        return;
                    }
                }
                _ = self.ticker.tick() => self.handle_terminal_tick().await,
            }
        }
    }

    async fn handle_control(&mut self, control: SqliteBatchWriterControl) -> bool {
        match control {
            SqliteBatchWriterControl::FlushNow {
                queued_depth_snapshot,
                responder,
            } => {
                self.handle_flush_now(queued_depth_snapshot, responder)
                    .await;
                false
            }
            SqliteBatchWriterControl::Shutdown { responder, .. } => {
                self.handle_shutdown(responder).await;
                true
            }
        }
    }

    fn handle_deferred_tick(&mut self) {
        let deferred_capacity = SQLITE_BATCH_MAX_ROWS.saturating_sub(self.pending.logical_rows());
        drain_terminal_journal_deferred_writes(
            &self.terminal_journal,
            &mut self.pending,
            &self.accounting,
            deferred_capacity,
            &self.queued_p1_count,
        );
        if self.pending.has_p2() {
            self.p2_schedule.arm_if_idle(Instant::now());
            self.accounting.update_p2_schedule(&self.p2_schedule);
        }
    }

    async fn handle_flush_now(
        &mut self,
        queued_depth_snapshot: usize,
        responder: oneshot::Sender<Result<(), String>>,
    ) {
        let mut remaining_queued = queued_depth_snapshot;
        let mut result = Ok(());
        loop {
            let drained = drain_queued_batch_writes(
                &mut self.write_receiver,
                &mut self.pending,
                &self.accounting,
                remaining_queued.min(SQLITE_BATCH_MAX_ROWS),
                &self.queued_p1_count,
            );
            remaining_queued = remaining_queued.saturating_sub(drained);
            drain_terminal_journal_deferred_writes(
                &self.terminal_journal,
                &mut self.pending,
                &self.accounting,
                SQLITE_BATCH_MAX_ROWS,
                &self.queued_p1_count,
            );
            if self.pending.is_empty() {
                if remaining_queued == 0 || drained == 0 {
                    break;
                }
                continue;
            }
            let flush_batch = take_next_bounded_batch(&mut self.pending);
            if flush_batch.is_empty() {
                break;
            }
            warn_about_oversized_p2_batch(&flush_batch);
            let Some(retained) = self.flush_batch(flush_batch, FlushReason::Barrier).await else {
                self.p2_schedule.succeeded();
                self.accounting.update_p2_schedule(&self.p2_schedule);
                if self.pending.is_empty() && remaining_queued == 0 {
                    break;
                }
                continue;
            };
            let summary = self.absorb_flushed_batch(retained, true, true);
            if summary.failed {
                result = Err(format!(
                    "sqlite batch writer retained {} logical rows after forced flush",
                    summary.logical_rows
                ));
                break;
            }
            if summary.p2_deferred {
                break;
            }
            if self.pending.is_empty() && remaining_queued == 0 {
                break;
            }
        }
        let _ = responder.send(result);
    }

    fn absorb_flushed_batch(
        &mut self,
        retained: RetainedBatch,
        reset_p1_on_success: bool,
        retry_p2_on_failure: bool,
    ) -> FlushBatchSummary {
        let logical_rows = retained.batch.logical_rows();
        let failed = retained.failed;
        let p2_deferred = retained.p2_defer.is_some();
        self.update_flush_schedules(&retained, reset_p1_on_success, retry_p2_on_failure);
        if retained.batch.terminal_invocations.is_empty() {
            self.pending.merge_p2(retained.batch);
        } else {
            let mut retained_batch = retained.batch;
            retained_batch.merge_all(self.pending.take());
            self.pending = retained_batch;
        }
        self.accounting.update_p2_schedule(&self.p2_schedule);
        FlushBatchSummary {
            logical_rows,
            failed,
            p2_deferred,
        }
    }

    fn update_flush_schedules(
        &mut self,
        retained: &RetainedBatch,
        reset_p1_on_success: bool,
        retry_p2_on_failure: bool,
    ) {
        if retained.failed && !retained.batch.terminal_invocations.is_empty() {
            self.transaction_sequence = self.transaction_sequence.saturating_add(1);
            let delay = self.p1_retry.failed(self.transaction_sequence);
            warn!(
                write_class = "p1_terminal",
                retry_generation = self.p1_retry.generation as u64,
                next_retry_delay_ms = delay.as_millis() as u64,
                "scheduled retained P1 batch with exponential backoff"
            );
        } else if reset_p1_on_success {
            self.p1_retry.succeeded();
        }
        match retained.p2_defer {
            Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                self.p2_schedule.defer_pressure(
                    Duration::from_millis(remaining_ms),
                    P2WakeReason::PressureCooldownElapsed,
                );
            }
            Some(P2DeferReason::BackgroundBusy {
                observed_generation,
            }) => {
                self.p2_eligibility_generation = observed_generation;
                self.p2_schedule.defer_until_background_eligible();
            }
            None if retry_p2_on_failure
                && retained.failed
                && retained.p2_retryable_failure
                && retained.batch.has_p2()
                && retained.batch.terminal_invocations.is_empty() =>
            {
                self.transaction_sequence = self.transaction_sequence.saturating_add(1);
                self.p2_schedule.failed(self.transaction_sequence);
                if retained.p2_lock_failure {
                    self.accounting.p2_lock_retried();
                }
            }
            None if retained.batch.has_p2() => {
                self.p2_schedule.arm_if_idle(Instant::now());
            }
            None => self.p2_schedule.succeeded(),
        }
    }

    async fn handle_p2_deadline(&mut self) {
        let priority_guard = self
            .p1_priority_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        drain_terminal_journal_deferred_writes(
            &self.terminal_journal,
            &mut self.pending,
            &self.accounting,
            SQLITE_BATCH_MAX_ROWS,
            &self.queued_p1_count,
        );
        drain_queued_writes_before_p2_dispatch(
            &mut self.write_receiver,
            &mut self.pending,
            &self.accounting,
            &mut self.p2_schedule,
            &self.queued_p1_count,
        );
        if self.queued_p1_count.load(Ordering::SeqCst) != 0 {
            drop(priority_guard);
            self.p2_schedule.arm_if_idle(Instant::now());
            self.accounting.update_p2_schedule(&self.p2_schedule);
            return;
        }
        let now = Instant::now();
        let submitted_p1 =
            !self.pending.terminal_invocations.is_empty() && self.p1_retry.ready(now);
        let flush_batch = if submitted_p1 {
            self.pending
                .take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
        } else {
            self.pending
                .take_p2_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
        };
        drop(priority_guard);
        warn_about_oversized_p2_batch(&flush_batch);
        let Some(retained) = self.flush_batch(flush_batch, FlushReason::Interval).await else {
            if submitted_p1 {
                self.p1_retry.succeeded();
            }
            self.p2_schedule.succeeded();
            self.accounting.update_p2_schedule(&self.p2_schedule);
            return;
        };
        let summary = self.absorb_flushed_batch(retained, submitted_p1, true);
        if summary.failed && submitted_p1 {
            warn!("scheduled retained P1 batch after P2 deadline flush failure");
        }
    }

    async fn handle_write(&mut self, maybe_write: Option<SqliteBatchWrite>) -> bool {
        let Some(write) = maybe_write else {
            self.handle_receiver_closed().await;
            return true;
        };
        if is_p1_terminal_write(&write) {
            decrement_queued_p1_count(&self.queued_p1_count);
        }
        self.pending.push_accounted(write, &self.accounting);
        if self.pending.has_p2() {
            self.p2_schedule.arm_if_idle(Instant::now());
            self.accounting.update_p2_schedule(&self.p2_schedule);
        }
        if self.should_flush_row_limit() {
            self.handle_row_limit_flush().await;
        }
        false
    }

    fn should_flush_row_limit(&self) -> bool {
        (self.pending.logical_rows() >= SQLITE_BATCH_MAX_ROWS
            || self.pending.estimated_memory_bytes() >= SQLITE_BATCH_MAX_BYTES)
            && (self.pending.terminal_invocations.is_empty() || self.p1_retry.ready(Instant::now()))
            && (!self.pending.terminal_invocations.is_empty()
                || self.p2_schedule.ready(Instant::now()))
    }

    async fn handle_row_limit_flush(&mut self) {
        let priority_guard = self
            .p1_priority_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let p2_ready = self.p2_schedule.ready(Instant::now())
            && self.queued_p1_count.load(Ordering::SeqCst) == 0;
        let flush_batch = if !self.pending.terminal_invocations.is_empty() {
            self.pending
                .take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
        } else if p2_ready {
            self.pending
                .take_p2_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
        } else {
            drop(priority_guard);
            return;
        };
        drop(priority_guard);
        warn_about_oversized_p2_batch(&flush_batch);
        let submitted_p1 = !flush_batch.terminal_invocations.is_empty();
        let submitted_p2 = flush_batch.has_p2();
        let Some(retained) = self.flush_batch(flush_batch, FlushReason::RowLimit).await else {
            if submitted_p1 {
                self.p1_retry.succeeded();
            }
            if submitted_p2 {
                self.p2_schedule.succeeded();
                self.accounting.update_p2_schedule(&self.p2_schedule);
            }
            return;
        };
        self.absorb_flushed_batch(retained, true, true);
    }

    async fn handle_terminal_tick(&mut self) {
        if self.pending.terminal_invocations.is_empty() || !self.p1_retry.ready(Instant::now()) {
            return;
        }
        let flush_reason = if self.pending.age() >= SQLITE_BATCH_MAX_AGE {
            if self.pending.age() >= SQLITE_BATCH_STALE_WARN_AGE {
                warn!(
                    logical_rows = self.pending.logical_rows(),
                    enqueued_rows = self.pending.enqueued_rows,
                    coalesced_rows = self.pending.coalesced_rows,
                    oldest_age_ms = self.pending.age().as_millis() as u64,
                    flush_reason = FlushReason::MaxAge.as_str(),
                    "sqlite batch writer pending terminal writes are stale under database pressure"
                );
            }
            FlushReason::MaxAge
        } else {
            FlushReason::Interval
        };
        let flush_batch = self
            .pending
            .take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES);
        let Some(retained) = self.flush_batch(flush_batch, flush_reason).await else {
            self.p1_retry.succeeded();
            return;
        };
        self.absorb_flushed_batch(retained, true, false);
    }

    async fn handle_shutdown(&mut self, responder: oneshot::Sender<Result<(), String>>) {
        self.write_receiver.close();
        let result = self.drain_shutdown(false).await;
        let _ = responder.send(result);
    }

    async fn handle_receiver_closed(&mut self) {
        if let Err(err) = self.drain_shutdown(true).await {
            warn!(error = %err, "sqlite batch writer receiver shutdown drain failed");
        }
    }

    async fn drain_shutdown(&mut self, receiver_closed: bool) -> Result<(), String> {
        let shutdown_deadline = Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE;
        loop {
            if Instant::now() >= shutdown_deadline {
                self.release_expired_shutdown_batch(receiver_closed);
                break;
            }
            let drained = if receiver_closed {
                0
            } else {
                drain_queued_batch_writes(
                    &mut self.write_receiver,
                    &mut self.pending,
                    &self.accounting,
                    SQLITE_BATCH_MAX_ROWS,
                    &self.queued_p1_count,
                )
            };
            drain_terminal_journal_deferred_writes(
                &self.terminal_journal,
                &mut self.pending,
                &self.accounting,
                SQLITE_BATCH_MAX_ROWS,
                &self.queued_p1_count,
            );
            if self.pending.is_empty() {
                if receiver_closed || drained == 0 {
                    break;
                }
                continue;
            }
            match self
                .flush_shutdown_batch(shutdown_deadline, receiver_closed)
                .await?
            {
                ShutdownBatchOutcome::Continue => {}
                ShutdownBatchOutcome::Break => break,
            }
        }
        self.release_remaining_shutdown_batch(receiver_closed)
    }

    fn release_expired_shutdown_batch(&mut self, receiver_closed: bool) {
        if !receiver_closed {
            let drained = drain_queued_batch_writes(
                &mut self.write_receiver,
                &mut self.pending,
                &self.accounting,
                SQLITE_BATCH_CHANNEL_CAPACITY,
                &self.queued_p1_count,
            );
            drain_terminal_journal_deferred_writes(
                &self.terminal_journal,
                &mut self.pending,
                &self.accounting,
                SQLITE_BATCH_CHANNEL_CAPACITY,
                &self.queued_p1_count,
            );
            warn!(
                drained,
                "sqlite batch writer bounded shutdown drain abandoned remaining queued work"
            );
        }
    }

    async fn flush_shutdown_batch(
        &mut self,
        shutdown_deadline: Instant,
        receiver_closed: bool,
    ) -> Result<ShutdownBatchOutcome, String> {
        let flush_batch = take_next_bounded_batch(&mut self.pending);
        let flush_rows = flush_batch.logical_rows();
        let flush_bytes = flush_batch.estimated_memory_bytes();
        let shutdown_quarantine = shutdown_recovery_batch(&flush_batch);
        let retained = match timeout_at(
            shutdown_deadline.into(),
            self.flush_batch(flush_batch, FlushReason::Shutdown),
        )
        .await
        {
            Ok(Some(retained)) => retained,
            Ok(None) => return Ok(ShutdownBatchOutcome::Continue),
            Err(_) => {
                if let Some(quarantine) = shutdown_quarantine.as_ref()
                    && let Err(err) = quarantine_shutdown_batch(
                        &self.terminal_journal,
                        &self.database_path,
                        quarantine,
                        "shutdown flush deadline exceeded",
                    )
                {
                    warn!(error = %err, "shutdown system-task quarantine failed after flush timeout");
                }
                self.accounting.release(flush_rows, flush_bytes);
                if receiver_closed {
                    warn!(
                        flush_rows,
                        "sqlite batch writer receiver shutdown flush exceeded deadline"
                    );
                    return Ok(ShutdownBatchOutcome::Break);
                }
                return Err(format!(
                    "sqlite batch writer shutdown flush exceeded deadline with {flush_rows} rows"
                ));
            }
        };
        self.complete_shutdown_flush(retained, shutdown_deadline, receiver_closed)
            .await
    }

    async fn complete_shutdown_flush(
        &mut self,
        retained: RetainedBatch,
        shutdown_deadline: Instant,
        receiver_closed: bool,
    ) -> Result<ShutdownBatchOutcome, String> {
        let logical_rows = retained.batch.logical_rows();
        let retained_rows_before_merge = logical_rows;
        let retained_bytes_before_merge = retained.batch.estimated_memory_bytes();
        let pending_rows_before_merge = self.pending.logical_rows();
        let pending_bytes_before_merge = self.pending.estimated_memory_bytes();
        let p2_defer_reason = retained.p2_defer;
        let p2_deferred = p2_defer_reason.is_some();
        let p2_retryable_failure = retained.p2_retryable_failure;
        let mut retained_batch = retained.batch;
        retained_batch.merge_all(self.pending.take());
        let merged_rows = retained_batch.logical_rows();
        let merged_bytes = retained_batch.estimated_memory_bytes();
        self.accounting.replace_batch(
            retained_rows_before_merge + pending_rows_before_merge,
            merged_rows,
            retained_bytes_before_merge + pending_bytes_before_merge,
            merged_bytes,
        );
        if retained.failed {
            return self
                .finish_shutdown_failure(
                    retained_batch,
                    logical_rows,
                    merged_rows,
                    merged_bytes,
                    p2_retryable_failure,
                    shutdown_deadline,
                    receiver_closed,
                )
                .await;
        }
        if !retained_batch.terminal_invocations.is_empty() {
            self.pending = retained_batch;
            return Ok(ShutdownBatchOutcome::Continue);
        }
        if p2_deferred {
            self.pending = retained_batch;
            let delay = shutdown_retry_delay(p2_defer_reason);
            sleep(delay.min(shutdown_deadline.saturating_duration_since(Instant::now()))).await;
            return Ok(ShutdownBatchOutcome::Continue);
        }
        if !retained_batch.is_empty() {
            self.pending = retained_batch;
            return Ok(ShutdownBatchOutcome::Continue);
        }
        warn!(logical_rows, "sqlite batch writer completed shutdown drain");
        Ok(ShutdownBatchOutcome::Break)
    }

    async fn finish_shutdown_failure(
        &mut self,
        retained_batch: PendingBatch,
        logical_rows: usize,
        merged_rows: usize,
        merged_bytes: usize,
        p2_retryable_failure: bool,
        shutdown_deadline: Instant,
        receiver_closed: bool,
    ) -> Result<ShutdownBatchOutcome, String> {
        let retry_delay = if !retained_batch.terminal_invocations.is_empty() {
            self.transaction_sequence = self.transaction_sequence.saturating_add(1);
            Some(self.p1_retry.failed(self.transaction_sequence))
        } else if p2_retryable_failure {
            self.transaction_sequence = self.transaction_sequence.saturating_add(1);
            Some(self.p2_schedule.failed(self.transaction_sequence))
        } else {
            None
        };
        if retry_delay.is_some() && Instant::now() < shutdown_deadline {
            self.pending = retained_batch;
            let delay = retry_delay
                .unwrap_or(Duration::from_millis(1))
                .min(shutdown_deadline.saturating_duration_since(Instant::now()));
            sleep(delay).await;
            return Ok(ShutdownBatchOutcome::Continue);
        }
        self.accounting.release(merged_rows, merged_bytes);
        if receiver_closed {
            warn!(
                retained_rows = retained_batch.logical_rows(),
                retained_bytes = retained_batch.estimated_memory_bytes(),
                "sqlite batch writer released failed retained memory accounting after receiver shutdown"
            );
            return Ok(ShutdownBatchOutcome::Break);
        }
        Err(format!(
            "sqlite batch writer retained {logical_rows} logical rows after shutdown flush"
        ))
    }

    fn release_remaining_shutdown_batch(&mut self, receiver_closed: bool) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let abandoned = std::mem::take(&mut self.pending);
        match release_shutdown_pending_batch(
            &self.accounting,
            &self.terminal_journal,
            &self.database_path,
            &abandoned,
            "shutdown drain stopped after flush timeout",
        ) {
            Ok(()) => Ok(()),
            Err(err) if receiver_closed => {
                warn!(error = %err, "receiver shutdown quarantine failed after flush timeout");
                Ok(())
            }
            Err(err) => Err(format!(
                "sqlite batch writer shutdown quarantine failed: {err:#}"
            )),
        }
    }
}

fn warn_about_oversized_p2_batch(batch: &PendingBatch) {
    if batch.terminal_invocations.is_empty()
        && batch.estimated_memory_bytes() > SQLITE_BATCH_MAX_BYTES
    {
        warn!(
            flush_priority = "P2",
            batch_rows = batch.logical_rows(),
            batch_bytes = batch.estimated_memory_bytes(),
            max_batch_bytes = SQLITE_BATCH_MAX_BYTES,
            "isolating oversized single P2 write"
        );
    }
}

fn shutdown_retry_delay(reason: Option<P2DeferReason>) -> Duration {
    match reason {
        Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
            Duration::from_millis(remaining_ms.max(1))
        }
        Some(P2DeferReason::BackgroundBusy { .. }) | None => Duration::from_millis(250),
    }
}
