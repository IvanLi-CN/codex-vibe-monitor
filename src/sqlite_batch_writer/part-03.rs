#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_sqlite_batch_writer(
    pool: Pool<Sqlite>,
    database_path: std::path::PathBuf,
    mut write_receiver: mpsc::Receiver<SqliteBatchWrite>,
    mut control_receiver: mpsc::Receiver<SqliteBatchWriterControl>,
    accounting: Arc<PendingQueueAccounting>,
    prompt_cache_conversation_cache: Option<Arc<Mutex<PromptCacheConversationsCacheState>>>,
    pricing_catalog: Option<Arc<RwLock<PricingCatalog>>>,
    terminal_runtime_store: Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache: Arc<
        std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>,
    >,
    summary_delta_hub: Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    terminal_projection_hub: Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: Arc<Mutex<()>>,
    terminal_journal: Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    queued_p1_count: Arc<AtomicUsize>,
    p1_priority_gate: Arc<std::sync::Mutex<()>>,
) {
    let mut ticker = interval(SQLITE_BATCH_FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut deferred_ticker = interval(crate::terminal_journal::TERMINAL_JOURNAL_SYNC_INTERVAL);
    deferred_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut pending = PendingBatch::default();
    let mut control_closed = false;
    let mut p1_retry = P1RetryState::default();
    let mut p2_schedule = P2ScheduleState::default();
    let mut p2_eligibility_generation =
        crate::db_pressure::global_db_pressure_gate().eligibility_generation();
    let mut transaction_sequence = 0_u64;

    loop {
        drain_queued_writes_before_dispatch(
            &mut write_receiver,
            &mut pending,
            &accounting,
            &mut p2_schedule,
            SQLITE_BATCH_MAX_ROWS,
            &queued_p1_count,
        );
        tokio::select! {
            biased;
            _ = crate::db_pressure::global_db_pressure_gate()
                .wait_for_eligibility_change(p2_eligibility_generation),
                if pending.has_p2()
                    && p2_schedule.wake_reason == Some(P2WakeReason::BackgroundEligible) =>
            {
                p2_eligibility_generation = crate::db_pressure::global_db_pressure_gate()
                    .eligibility_generation();
                p2_schedule.wake_background_eligible();
                accounting.update_p2_schedule(&p2_schedule);
            }
            maybe_control = control_receiver.recv(), if !control_closed => {
                if let Some(control) = maybe_control {
                    match control {
                    SqliteBatchWriterControl::FlushNow {
                        queued_depth_snapshot,
                        responder,
                    } => {
                        let mut remaining_queued = queued_depth_snapshot;
                        let mut result = Ok(());
                        loop {
                            let drained = drain_queued_batch_writes(
                                &mut write_receiver,
                                &mut pending,
                                &accounting,
                                remaining_queued.min(SQLITE_BATCH_MAX_ROWS),
                                &queued_p1_count,
                            );
                            remaining_queued = remaining_queued.saturating_sub(drained);
                            drain_terminal_journal_deferred_writes(
                                &terminal_journal,
                                &mut pending,
                                &accounting,
                                SQLITE_BATCH_MAX_ROWS,
                                &queued_p1_count,
                            );
                            if pending.is_empty() {
                                if remaining_queued == 0 || drained == 0 {
                                    break;
                                }
                                continue;
                            }
                            let flush_batch = take_next_bounded_batch(&mut pending);
                            if flush_batch.is_empty() {
                                break;
                            }
                            if flush_batch.terminal_invocations.is_empty()
                                && flush_batch.estimated_memory_bytes() > SQLITE_BATCH_MAX_BYTES
                            {
                                warn!(
                                    flush_priority = "P2",
                                    batch_rows = flush_batch.logical_rows(),
                                    batch_bytes = flush_batch.estimated_memory_bytes(),
                                    max_batch_bytes = SQLITE_BATCH_MAX_BYTES,
                                    "isolating oversized single P2 write"
                                );
                            }
                            let Some(retained) = flush_pending_batch_accounted(
                                &accounting,
                                &pool,
                                pricing_catalog.as_ref(),
                                flush_batch,
                                FlushReason::Barrier,
                                prompt_cache_conversation_cache.as_ref(),
                                &terminal_runtime_store,
                                &dashboard_activity_snapshot_cache,
                                &summary_delta_hub,
                                &terminal_projection_hub,
                                &dashboard_reconcile_gate,
                                &terminal_journal,
                            )
                            .await
                            else {
                                p2_schedule.succeeded();
                                accounting.update_p2_schedule(&p2_schedule);
                                if pending.is_empty() && remaining_queued == 0 {
                                    break;
                                }
                                continue;
                            };
                            let logical_rows = retained.batch.logical_rows();
                            let failed = retained.failed;
                            if retained.failed && !retained.batch.terminal_invocations.is_empty() {
                                transaction_sequence = transaction_sequence.saturating_add(1);
                                let delay = p1_retry.failed(transaction_sequence);
                                warn!(
                                    write_class = "p1_terminal",
                                    retry_generation = p1_retry.generation as u64,
                                    next_retry_delay_ms = delay.as_millis() as u64,
                                    "scheduled retained P1 batch after forced flush failure"
                                );
                            } else {
                                p1_retry.succeeded();
                            }
                            match retained.p2_defer {
                                Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                                    p2_schedule.defer_pressure(
                                        Duration::from_millis(remaining_ms),
                                        P2WakeReason::PressureCooldownElapsed,
                                    );
                                }
                                Some(P2DeferReason::BackgroundBusy {
                                    observed_generation,
                                }) => {
                                    p2_eligibility_generation = observed_generation;
                                    p2_schedule.defer_until_background_eligible();
                                }
                                None if retained.failed
                                    && retained.p2_retryable_failure
                                    && retained.batch.has_p2()
                                    && retained.batch.terminal_invocations.is_empty() =>
                                {
                                    transaction_sequence = transaction_sequence.saturating_add(1);
                                    p2_schedule.failed(transaction_sequence);
                                    if retained.p2_lock_failure {
                                        accounting.p2_lock_retried();
                                    }
                                }
                                None if retained.batch.has_p2() => {
                                    p2_schedule.arm_if_idle(Instant::now());
                                }
                                None => p2_schedule.succeeded(),
                            }
                            accounting.update_p2_schedule(&p2_schedule);
                            let p2_deferred = retained.p2_defer.is_some();
                            let mut retained_batch = retained.batch;
                            retained_batch.merge_all(pending.take());
                            pending = retained_batch;
                            if failed {
                                result = Err(format!(
                                    "sqlite batch writer retained {logical_rows} logical rows after forced flush"
                                ));
                                break;
                            }
                            if p2_deferred {
                                break;
                            }
                            if pending.is_empty() && remaining_queued == 0 {
                                break;
                            }
                        }
                        let _ = responder.send(result);
                    }
                    SqliteBatchWriterControl::Shutdown { responder, .. } => {
                        write_receiver.close();
                        let mut result = Ok(());
                        let shutdown_deadline = Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE;
                        loop {
                            if Instant::now() >= shutdown_deadline {
                                let drained = drain_queued_batch_writes(
                                    &mut write_receiver,
                                    &mut pending,
                                    &accounting,
                                    SQLITE_BATCH_CHANNEL_CAPACITY,
                                    &queued_p1_count,
                                );
                                drain_terminal_journal_deferred_writes(
                                    &terminal_journal,
                                    &mut pending,
                                    &accounting,
                                    SQLITE_BATCH_CHANNEL_CAPACITY,
                                    &queued_p1_count,
                                );
                                let abandoned = std::mem::take(&mut pending);
                                let _ = release_shutdown_pending_batch(
                                    &accounting,
                                    &terminal_journal,
                                    &database_path,
                                    &abandoned,
                                    "shutdown drain deadline exceeded",
                                )
                                .map_err(|err| {
                                    result = Err(format!(
                                        "sqlite batch writer shutdown quarantine failed: {err:#}"
                                    ));
                                });
                                warn!(
                                    drained,
                                    "sqlite batch writer bounded shutdown drain abandoned remaining queued work"
                                );
                                break;
                            }
                            let drained = drain_queued_batch_writes(
                                &mut write_receiver,
                                &mut pending,
                                &accounting,
                                SQLITE_BATCH_MAX_ROWS,
                                &queued_p1_count,
                            );
                            drain_terminal_journal_deferred_writes(
                                &terminal_journal,
                                &mut pending,
                                &accounting,
                                SQLITE_BATCH_MAX_ROWS,
                                &queued_p1_count,
                            );
                            if pending.is_empty() {
                                if drained == 0 {
                                    break;
                                }
                                continue;
                            }
                            let flush_batch = take_next_bounded_batch(&mut pending);
                            let flush_rows = flush_batch.logical_rows();
                            let flush_bytes = flush_batch.estimated_memory_bytes();
                            let shutdown_quarantine = shutdown_recovery_batch(&flush_batch);
                            let retained = match timeout_at(
                                shutdown_deadline.into(),
                                flush_pending_batch_accounted(
                                    &accounting,
                                    &pool,
                                    pricing_catalog.as_ref(),
                                    flush_batch,
                                    FlushReason::Shutdown,
                                    prompt_cache_conversation_cache.as_ref(),
                                    &terminal_runtime_store,
                                    &dashboard_activity_snapshot_cache,
                                    &summary_delta_hub,
                                    &terminal_projection_hub,
                                    &dashboard_reconcile_gate,
                                    &terminal_journal,
                                ),
                            )
                            .await
                            {
                                Ok(Some(retained)) => retained,
                                Ok(None) => continue,
                                Err(_) => {
                                    if let Some(quarantine) = shutdown_quarantine.as_ref()
                                        && let Err(err) = quarantine_shutdown_batch(
                                            &terminal_journal,
                                            &database_path,
                                            quarantine,
                                            "shutdown flush deadline exceeded",
                                        )
                                    {
                                        warn!(error = %err, "shutdown system-task quarantine failed after flush timeout");
                                    }
                                    accounting.release(flush_rows, flush_bytes);
                                    result = Err(format!(
                                        "sqlite batch writer shutdown flush exceeded deadline with {flush_rows} rows"
                                    ));
                                    break;
                                }
                            };
                            let logical_rows = retained.batch.logical_rows();
                            let failed = retained.failed;
                            let retained_rows_before_merge = retained.batch.logical_rows();
                            let retained_bytes_before_merge = retained.batch.estimated_memory_bytes();
                            let pending_rows_before_merge = pending.logical_rows();
                            let pending_bytes_before_merge = pending.estimated_memory_bytes();
                            let p2_defer_reason = retained.p2_defer;
                            let p2_deferred = p2_defer_reason.is_some();
                            let p2_retryable_failure = retained.p2_retryable_failure;
                            let mut retained_batch = retained.batch;
                            retained_batch.merge_all(pending.take());
                            let merged_rows = retained_batch.logical_rows();
                            let merged_bytes = retained_batch.estimated_memory_bytes();
                            accounting.replace_batch(
                                retained_rows_before_merge + pending_rows_before_merge,
                                merged_rows,
                                retained_bytes_before_merge + pending_bytes_before_merge,
                                merged_bytes,
                            );
                            if failed {
                                let retry_delay = if !retained_batch.terminal_invocations.is_empty() {
                                    transaction_sequence = transaction_sequence.saturating_add(1);
                                    Some(p1_retry.failed(transaction_sequence))
                                } else if p2_retryable_failure {
                                    transaction_sequence = transaction_sequence.saturating_add(1);
                                    Some(p2_schedule.failed(transaction_sequence))
                                } else {
                                    None
                                };
                                if retry_delay.is_some() && Instant::now() < shutdown_deadline {
                                    pending = retained_batch;
                                    let delay = retry_delay
                                        .unwrap_or(Duration::from_millis(1))
                                        .min(shutdown_deadline.saturating_duration_since(Instant::now()));
                                    sleep(delay).await;
                                    continue;
                                }
                                result = Err(format!(
                                    "sqlite batch writer retained {logical_rows} logical rows after shutdown flush"
                                ));
                                accounting.release(merged_rows, merged_bytes);
                                break;
                            }
                            if !retained_batch.terminal_invocations.is_empty() {
                                pending = retained_batch;
                                continue;
                            }
                            if p2_deferred {
                                pending = retained_batch;
                                let delay = match p2_defer_reason {
                                    Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                                        Duration::from_millis(remaining_ms.max(1))
                                    }
                                    Some(P2DeferReason::BackgroundBusy { .. }) | None => {
                                        Duration::from_millis(250)
                                    }
                                };
                                sleep(delay.min(
                                    shutdown_deadline.saturating_duration_since(Instant::now()),
                                ))
                                .await;
                                continue;
                            }
                            if !retained_batch.is_empty() {
                                pending = retained_batch;
                                continue;
                            }
                            warn!(
                                logical_rows,
                                "sqlite batch writer completed shutdown drain"
                            );
                        }
                        if !pending.is_empty() {
                            let abandoned = std::mem::take(&mut pending);
                            if let Err(err) = release_shutdown_pending_batch(
                                &accounting,
                                &terminal_journal,
                                &database_path,
                                &abandoned,
                                "shutdown drain stopped after flush timeout",
                            ) {
                                result = Err(format!(
                                    "sqlite batch writer shutdown quarantine failed: {err:#}"
                                ));
                            }
                        }
                        let _ = responder.send(result);
                        return;
                    }
                    }
                } else {
                    control_closed = true;
                }
            }
            _ = wait_for_p2_deadline(p2_schedule.due_at),
                if p2_deadline_wait_armed(&pending, &p2_schedule, &queued_p1_count, &p1_retry) =>
            {
                let priority_guard = p1_priority_gate
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                drain_terminal_journal_deferred_writes(
                    &terminal_journal,
                    &mut pending,
                    &accounting,
                    SQLITE_BATCH_MAX_ROWS,
                    &queued_p1_count,
                );
                drain_queued_writes_before_p2_dispatch(
                    &mut write_receiver,
                    &mut pending,
                    &accounting,
                    &mut p2_schedule,
                    &queued_p1_count,
                );
                if queued_p1_count.load(Ordering::SeqCst) != 0 {
                    drop(priority_guard);
                    p2_schedule.arm_if_idle(Instant::now());
                    accounting.update_p2_schedule(&p2_schedule);
                    continue;
                }
                let now = Instant::now();
                let submitted_p1 = !pending.terminal_invocations.is_empty() && p1_retry.ready(now);
                let flush_batch = if submitted_p1 {
                    pending.take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
                } else {
                    pending.take_p2_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
                };
                drop(priority_guard);
                if flush_batch.terminal_invocations.is_empty()
                    && flush_batch.estimated_memory_bytes() > SQLITE_BATCH_MAX_BYTES
                {
                    warn!(
                        flush_priority = "P2",
                        batch_rows = flush_batch.logical_rows(),
                        batch_bytes = flush_batch.estimated_memory_bytes(),
                        max_batch_bytes = SQLITE_BATCH_MAX_BYTES,
                        "isolating oversized single P2 write"
                    );
                }
                if let Some(retained) =
                    flush_pending_batch_accounted(
                        &accounting,
                        &pool,
                        pricing_catalog.as_ref(),
                        flush_batch,
                        FlushReason::Interval,
                        prompt_cache_conversation_cache.as_ref(),
                        &terminal_runtime_store,
                        &dashboard_activity_snapshot_cache,
                        &summary_delta_hub,
                        &terminal_projection_hub,
                        &dashboard_reconcile_gate,
                        &terminal_journal,
                    )
                    .await
                {
                    if retained.failed && !retained.batch.terminal_invocations.is_empty() {
                        transaction_sequence = transaction_sequence.saturating_add(1);
                        let delay = p1_retry.failed(transaction_sequence);
                        warn!(
                            write_class = "p1_terminal",
                            retry_generation = p1_retry.generation as u64,
                            next_retry_delay_ms = delay.as_millis() as u64,
                            "scheduled retained P1 batch with exponential backoff"
                        );
                    } else if submitted_p1 {
                        p1_retry.succeeded();
                    }
                    match retained.p2_defer {
                        Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                            p2_schedule.defer_pressure(
                                Duration::from_millis(remaining_ms),
                                P2WakeReason::PressureCooldownElapsed,
                            );
                        }
                        Some(P2DeferReason::BackgroundBusy {
                            observed_generation,
                        }) => {
                            p2_eligibility_generation = observed_generation;
                            p2_schedule.defer_until_background_eligible();
                        }
                        None if retained.failed
                            && retained.p2_retryable_failure
                            && retained.batch.has_p2()
                            && retained.batch.terminal_invocations.is_empty() => {
                            transaction_sequence = transaction_sequence.saturating_add(1);
                            let delay = p2_schedule.failed(transaction_sequence);
                            if retained.p2_lock_failure {
                                accounting.p2_lock_retried();
                            }
                            warn!(
                                write_class = "p2_derived",
                                retry_generation = p2_schedule.generation as u64,
                                next_retry_delay_ms = delay.as_millis() as u64,
                                "scheduled retained P2 batch with exponential backoff"
                            );
                        }
                        None if retained.batch.has_p2() => {
                            p2_schedule.arm_if_idle(Instant::now());
                        }
                        None => p2_schedule.succeeded(),
                    }
                    if retained.batch.terminal_invocations.is_empty() {
                        pending.merge_p2(retained.batch);
                    } else {
                        let mut retained_batch = retained.batch;
                        retained_batch.merge_all(pending.take());
                        pending = retained_batch;
                    }
                    accounting.update_p2_schedule(&p2_schedule);
                } else {
                    if submitted_p1 {
                        p1_retry.succeeded();
                    }
                    p2_schedule.succeeded();
                    accounting.update_p2_schedule(&p2_schedule);
                }
            }
            _ = deferred_ticker.tick() => {
                let deferred_capacity = SQLITE_BATCH_MAX_ROWS.saturating_sub(pending.logical_rows());
                drain_terminal_journal_deferred_writes(
                    &terminal_journal,
                    &mut pending,
                    &accounting,
                    deferred_capacity,
                    &queued_p1_count,
                );
                if pending.has_p2() {
                    p2_schedule.arm_if_idle(Instant::now());
                    accounting.update_p2_schedule(&p2_schedule);
                }
            }
            maybe_write = write_receiver.recv() => {
                let Some(write) = maybe_write else {
                    let shutdown_deadline = Instant::now() + SQLITE_SHUTDOWN_DRAIN_DEADLINE;
                    loop {
                        if Instant::now() >= shutdown_deadline {
                            let abandoned = std::mem::take(&mut pending);
                            let _ = release_shutdown_pending_batch(
                                &accounting,
                                &terminal_journal,
                                &database_path,
                                &abandoned,
                                "receiver shutdown drain deadline exceeded",
                            )
                            .map_err(|err| {
                                warn!(error = %err, "receiver shutdown quarantine failed");
                            });
                            break;
                        }
                        drain_terminal_journal_deferred_writes(
                            &terminal_journal,
                            &mut pending,
                            &accounting,
                            SQLITE_BATCH_MAX_ROWS,
                            &queued_p1_count,
                        );
                        if pending.is_empty() {
                            break;
                        }
                        let flush_batch = take_next_bounded_batch(&mut pending);
                        let flush_rows = flush_batch.logical_rows();
                        let flush_bytes = flush_batch.estimated_memory_bytes();
                        let shutdown_quarantine = shutdown_recovery_batch(&flush_batch);
                        let retained = match timeout_at(
                            shutdown_deadline.into(),
                            flush_pending_batch_accounted(
                                &accounting,
                                &pool,
                                pricing_catalog.as_ref(),
                                flush_batch,
                                FlushReason::Shutdown,
                                prompt_cache_conversation_cache.as_ref(),
                                &terminal_runtime_store,
                                &dashboard_activity_snapshot_cache,
                                &summary_delta_hub,
                                &terminal_projection_hub,
                                &dashboard_reconcile_gate,
                                &terminal_journal,
                            ),
                        )
                        .await
                        {
                            Ok(Some(retained)) => retained,
                            Ok(None) => continue,
                            Err(_) => {
                                if let Some(quarantine) = shutdown_quarantine.as_ref()
                                    && let Err(err) = quarantine_shutdown_batch(
                                        &terminal_journal,
                                        &database_path,
                                        quarantine,
                                        "receiver shutdown flush deadline exceeded",
                                    )
                                {
                                    warn!(error = %err, "receiver shutdown system-task quarantine failed after flush timeout");
                                }
                                accounting.release(flush_rows, flush_bytes);
                                warn!(
                                    flush_rows,
                                    "sqlite batch writer receiver shutdown flush exceeded deadline"
                                );
                                break;
                            }
                        };
                        let retained_rows_before_merge = retained.batch.logical_rows();
                        let retained_bytes_before_merge = retained.batch.estimated_memory_bytes();
                        let pending_rows_before_merge = pending.logical_rows();
                        let pending_bytes_before_merge = pending.estimated_memory_bytes();
                        let p2_defer_reason = retained.p2_defer;
                        let p2_deferred = p2_defer_reason.is_some();
                        let p2_retryable_failure = retained.p2_retryable_failure;
                        let mut retained_batch = retained.batch;
                        retained_batch.merge_all(pending.take());
                        let merged_rows = retained_batch.logical_rows();
                        let merged_bytes = retained_batch.estimated_memory_bytes();
                        accounting.replace_batch(
                            retained_rows_before_merge + pending_rows_before_merge,
                            merged_rows,
                            retained_bytes_before_merge + pending_bytes_before_merge,
                            merged_bytes,
                        );
                        if retained.failed {
                            let retry_delay = if !retained_batch.terminal_invocations.is_empty() {
                                transaction_sequence = transaction_sequence.saturating_add(1);
                                Some(p1_retry.failed(transaction_sequence))
                            } else if p2_retryable_failure {
                                transaction_sequence = transaction_sequence.saturating_add(1);
                                Some(p2_schedule.failed(transaction_sequence))
                            } else {
                                None
                            };
                            if retry_delay.is_some() && Instant::now() < shutdown_deadline {
                                pending = retained_batch;
                                let delay = retry_delay
                                    .unwrap_or(Duration::from_millis(1))
                                    .min(shutdown_deadline.saturating_duration_since(Instant::now()));
                                sleep(delay).await;
                                continue;
                            }
                            accounting.release(merged_rows, merged_bytes);
                            warn!(
                                retained_rows = retained_batch.logical_rows(),
                                retained_bytes = retained_batch.estimated_memory_bytes(),
                                "sqlite batch writer released failed retained memory accounting after receiver shutdown"
                            );
                            break;
                        }
                        if !retained_batch.terminal_invocations.is_empty() {
                            pending = retained_batch;
                            continue;
                        }
                        if p2_deferred {
                            pending = retained_batch;
                            let delay = match p2_defer_reason {
                                Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                                    Duration::from_millis(remaining_ms.max(1))
                                }
                                Some(P2DeferReason::BackgroundBusy { .. }) | None => {
                                    Duration::from_millis(250)
                                }
                            };
                            sleep(delay.min(
                                shutdown_deadline.saturating_duration_since(Instant::now()),
                            ))
                            .await;
                            continue;
                        }
                        if !retained_batch.is_empty() {
                            pending = retained_batch;
                            continue;
                        }
                        break;
                    }
                    if !pending.is_empty() {
                        let abandoned = std::mem::take(&mut pending);
                        if let Err(err) = release_shutdown_pending_batch(
                            &accounting,
                            &terminal_journal,
                            &database_path,
                            &abandoned,
                            "receiver shutdown drain stopped after flush timeout",
                        ) {
                            warn!(error = %err, "receiver shutdown quarantine failed after flush timeout");
                        }
                    }
                    return;
                };
                if is_p1_terminal_write(&write) {
                    decrement_queued_p1_count(&queued_p1_count);
                }
                pending.push_accounted(write, &accounting);
                if pending.has_p2() {
                    p2_schedule.arm_if_idle(Instant::now());
                    accounting.update_p2_schedule(&p2_schedule);
                }
                if (pending.logical_rows() >= SQLITE_BATCH_MAX_ROWS
                    || pending.estimated_memory_bytes() >= SQLITE_BATCH_MAX_BYTES)
                    && (pending.terminal_invocations.is_empty() || p1_retry.ready(Instant::now()))
                    && (!pending.terminal_invocations.is_empty()
                        || p2_schedule.ready(Instant::now()))
                {
                    let priority_guard = p1_priority_gate
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    let p2_ready = p2_schedule.ready(Instant::now())
                        && queued_p1_count.load(Ordering::SeqCst) == 0;
                    let flush_batch = if !pending.terminal_invocations.is_empty() {
                        pending.take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
                    } else if p2_ready {
                        pending.take_p2_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
                    } else {
                        drop(priority_guard);
                        continue;
                    };
                    drop(priority_guard);
                    if flush_batch.terminal_invocations.is_empty()
                        && flush_batch.estimated_memory_bytes() > SQLITE_BATCH_MAX_BYTES
                    {
                        warn!(
                            flush_priority = "P2",
                            batch_rows = flush_batch.logical_rows(),
                            batch_bytes = flush_batch.estimated_memory_bytes(),
                            max_batch_bytes = SQLITE_BATCH_MAX_BYTES,
                            "isolating oversized single P2 write"
                        );
                    }
                    let submitted_p1 = !flush_batch.terminal_invocations.is_empty();
                    let submitted_p2 = flush_batch.has_p2();
                    if let Some(retained) =
                        flush_pending_batch_accounted(
                            &accounting,
                            &pool,
                            pricing_catalog.as_ref(),
                            flush_batch,
                            FlushReason::RowLimit,
                            prompt_cache_conversation_cache.as_ref(),
                            &terminal_runtime_store,
                            &dashboard_activity_snapshot_cache,
                            &summary_delta_hub,
                            &terminal_projection_hub,
                            &dashboard_reconcile_gate,
                            &terminal_journal,
                        )
                        .await
                    {
                        if retained.failed && !retained.batch.terminal_invocations.is_empty() {
                            transaction_sequence = transaction_sequence.saturating_add(1);
                            let delay = p1_retry.failed(transaction_sequence);
                            warn!(
                                write_class = "p1_terminal",
                                retry_generation = p1_retry.generation as u64,
                                next_retry_delay_ms = delay.as_millis() as u64,
                                "scheduled retained P1 batch with exponential backoff"
                            );
                        } else {
                            p1_retry.succeeded();
                        }
                        match retained.p2_defer {
                            Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                                p2_schedule.defer_pressure(
                                    Duration::from_millis(remaining_ms),
                                    P2WakeReason::PressureCooldownElapsed,
                                );
                            }
                            Some(P2DeferReason::BackgroundBusy {
                                observed_generation,
                            }) => {
                                p2_eligibility_generation = observed_generation;
                                p2_schedule.defer_until_background_eligible();
                            }
                            None if retained.failed
                                && retained.p2_retryable_failure
                                && retained.batch.has_p2()
                                && retained.batch.terminal_invocations.is_empty() => {
                                transaction_sequence = transaction_sequence.saturating_add(1);
                                p2_schedule.failed(transaction_sequence);
                                if retained.p2_lock_failure {
                                    accounting.p2_lock_retried();
                                }
                            }
                            None if retained.batch.has_p2() => {
                                p2_schedule.arm_if_idle(Instant::now());
                            }
                            None => p2_schedule.succeeded(),
                        }
                        accounting.update_p2_schedule(&p2_schedule);
                        if retained.batch.terminal_invocations.is_empty() {
                            pending.merge_p2(retained.batch);
                        } else {
                            let mut retained_batch = retained.batch;
                            retained_batch.merge_all(pending.take());
                            pending = retained_batch;
                        }
                    } else {
                        if submitted_p1 {
                            p1_retry.succeeded();
                        }
                        if submitted_p2 {
                            p2_schedule.succeeded();
                            accounting.update_p2_schedule(&p2_schedule);
                        }
                    }
                }
            }
            _ = ticker.tick() => {
                if pending.terminal_invocations.is_empty() || !p1_retry.ready(Instant::now()) {
                    continue;
                }
                let flush_reason = if pending.age() >= SQLITE_BATCH_MAX_AGE {
                    if pending.age() >= SQLITE_BATCH_STALE_WARN_AGE {
                        warn!(
                            logical_rows = pending.logical_rows(),
                            enqueued_rows = pending.enqueued_rows,
                            coalesced_rows = pending.coalesced_rows,
                            oldest_age_ms = pending.age().as_millis() as u64,
                            flush_reason = FlushReason::MaxAge.as_str(),
                            "sqlite batch writer pending terminal writes are stale under database pressure"
                        );
                    }
                    FlushReason::MaxAge
                } else {
                    FlushReason::Interval
                };
                let flush_batch = pending
                    .take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES);
                if let Some(retained) =
                    flush_pending_batch_accounted(
                        &accounting,
                        &pool,
                        pricing_catalog.as_ref(),
                        flush_batch,
                        flush_reason,
                        prompt_cache_conversation_cache.as_ref(),
                        &terminal_runtime_store,
                        &dashboard_activity_snapshot_cache,
                        &summary_delta_hub,
                        &terminal_projection_hub,
                        &dashboard_reconcile_gate,
                        &terminal_journal,
                    )
                    .await
                {
                    if retained.failed && !retained.batch.terminal_invocations.is_empty() {
                        transaction_sequence = transaction_sequence.saturating_add(1);
                        let delay = p1_retry.failed(transaction_sequence);
                        warn!(
                            write_class = "p1_terminal",
                            retry_generation = p1_retry.generation as u64,
                            next_retry_delay_ms = delay.as_millis() as u64,
                            "scheduled retained P1 batch with exponential backoff"
                        );
                    } else {
                        p1_retry.succeeded();
                    }
                    match retained.p2_defer {
                        Some(P2DeferReason::PressureCooldown(remaining_ms)) => {
                            p2_schedule.defer_pressure(
                                Duration::from_millis(remaining_ms),
                                P2WakeReason::PressureCooldownElapsed,
                            );
                        }
                        Some(P2DeferReason::BackgroundBusy {
                            observed_generation,
                        }) => {
                            p2_eligibility_generation = observed_generation;
                            p2_schedule.defer_until_background_eligible();
                        }
                        None if retained.batch.has_p2() => {
                            p2_schedule.arm_if_idle(Instant::now());
                        }
                        None => p2_schedule.succeeded(),
                    }
                    if retained.batch.terminal_invocations.is_empty() {
                        pending.merge_p2(retained.batch);
                    } else {
                        let mut retained_batch = retained.batch;
                        retained_batch.merge_all(pending.take());
                        pending = retained_batch;
                    }
                    accounting.update_p2_schedule(&p2_schedule);
                } else {
                    p1_retry.succeeded();
                }
            }
        }
    }
}
