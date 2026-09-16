#[cfg(not(test))]
async fn run_terminal_journal_sync(
    terminal_journal: Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    shutdown: CancellationToken,
) {
    let mut ticker = interval(crate::terminal_journal::TERMINAL_JOURNAL_SYNC_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                if let Ok(mut guard) = terminal_journal.lock()
                    && let Some(journal) = guard.as_mut()
                    && let Err(err) = journal.force_sync()
                {
                    warn!(error = %err, "terminal journal final group commit failed during shutdown");
                }
                return;
            }
            _ = ticker.tick() => {
                if let Ok(mut guard) = terminal_journal.lock()
                    && let Some(journal) = guard.as_mut()
                    && let Some(group_commit_elapsed_ms) = journal.sync_if_due()
                {
                    let stats = journal.stats();
                    debug!(
                        group_commit_elapsed_ms,
                        journal_pending_records = stats.pending_records,
                        journal_pending_bytes = stats.pending_bytes,
                        journal_segment_count = stats.segment_count,
                        "terminal journal group commit completed"
                    );
                }
            }
        }
    }
}

fn drain_terminal_journal_deferred_writes(
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    pending: &mut PendingBatch,
    accounting: &PendingQueueAccounting,
    max_writes: usize,
    queued_p1_count: &AtomicUsize,
) {
    if max_writes == 0 {
        return;
    }
    if let Ok(mut guard) = terminal_journal.lock()
        && let Some(journal) = guard.as_mut()
    {
        let deferred_capacity = max_writes.saturating_sub(journal.deferred_write_count());
        journal.queue_replay_for_dispatch(deferred_capacity);
        for terminal in journal.take_deferred_writes(max_writes) {
            decrement_queued_p1_count(queued_p1_count);
            let write = SqliteBatchWrite::TerminalInvocation(terminal);
            accounting.enqueue(write.estimated_memory_bytes());
            accounting.retry_deferred();
            pending.push_accounted(write, accounting);
        }
        for finish in
            journal.take_system_task_finishes(max_writes.saturating_sub(pending.logical_rows()))
        {
            let write = SqliteBatchWrite::SystemTaskFinish(finish);
            accounting.enqueue(write.estimated_memory_bytes());
            accounting.retry_deferred();
            pending.push_accounted(write, accounting);
        }
    }
}

pub(crate) fn drain_queued_batch_writes(
    write_receiver: &mut mpsc::Receiver<SqliteBatchWrite>,
    pending: &mut PendingBatch,
    accounting: &PendingQueueAccounting,
    max_messages: usize,
    queued_p1_count: &AtomicUsize,
) -> usize {
    let mut drained = 0_usize;
    for _ in 0..max_messages {
        match write_receiver.try_recv() {
            Ok(write) => {
                if is_p1_terminal_write(&write) {
                    decrement_queued_p1_count(queued_p1_count);
                }
                pending.push_accounted(write, accounting);
                drained = drained.saturating_add(1);
            }
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                break;
            }
        }
    }
    drained
}

fn take_next_bounded_batch(pending: &mut PendingBatch) -> PendingBatch {
    if !pending.terminal_invocations.is_empty() {
        pending.take_p1_terminal_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
    } else {
        pending.take_p2_chunk(SQLITE_BATCH_MAX_ROWS, SQLITE_BATCH_MAX_BYTES)
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The accounting wrapper mirrors the single-writer ownership boundaries."
)]
async fn flush_pending_batch_accounted(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    batch: PendingBatch,
    reason: FlushReason,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache: &Arc<
        std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>,
    >,
    summary_delta_hub: &Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    terminal_projection_hub: &Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: &Arc<Mutex<()>>,
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
) -> Option<RetainedBatch> {
    let was_retained_retry = batch.retained_for_retry;
    let submitted_system_task_ids = batch
        .system_task_finishes
        .keys()
        .copied()
        .collect::<Vec<_>>();
    let submitted_depth = batch.logical_rows();
    let submitted_bytes = batch.estimated_memory_bytes();
    let result = flush_pending_batch(
        accounting,
        pool,
        pricing_catalog,
        batch,
        reason,
        prompt_cache_conversation_cache,
        terminal_runtime_store,
        dashboard_activity_snapshot_cache,
        summary_delta_hub,
        terminal_projection_hub,
        dashboard_reconcile_gate,
        terminal_journal,
    )
    .await;
    let completed_system_task_ids =
        successfully_flushed_system_task_ids(&submitted_system_task_ids, result.as_ref());
    if !completed_system_task_ids.is_empty()
        && let Ok(mut guard) = terminal_journal.lock()
        && let Some(journal) = guard.as_mut()
        && let Err(err) = journal.acknowledge_system_task_finishes(&completed_system_task_ids)
    {
        warn!(
            error = %err,
            completed_system_task_count = completed_system_task_ids.len(),
            "failed to acknowledge recovered system-task finishes"
        );
    }
    let discard_non_retryable_p2 = result.as_ref().is_some_and(|retained| {
        retained.failed
            && !retained.p2_retryable_failure
            && retained.batch.terminal_invocations.is_empty()
    });
    if discard_non_retryable_p2 {
        let discarded_overlay_count = result
            .as_ref()
            .map(|retained| {
                cleanup_discarded_p2_runtime_overlays(&retained.batch, terminal_runtime_store)
            })
            .unwrap_or_default();
        warn!(
            flush_priority = "P2",
            submitted_depth,
            submitted_bytes,
            discarded_overlay_count,
            "discarded non-retryable P2 batch after deterministic failure; durable source remains authoritative"
        );
    }
    if was_retained_retry && result.as_ref().is_some_and(|retained| retained.failed) {
        accounting.retry_deferred();
    }
    let retained_bytes = result
        .as_ref()
        .filter(|_| !discard_non_retryable_p2)
        .map(|retained| retained.batch.estimated_memory_bytes())
        .unwrap_or_default();
    let retained_depth = result
        .as_ref()
        .filter(|_| !discard_non_retryable_p2)
        .map(|retained| retained.batch.logical_rows())
        .unwrap_or_default();
    accounting.complete(
        submitted_depth,
        retained_depth,
        submitted_bytes,
        retained_bytes,
    );
    if discard_non_retryable_p2 {
        None
    } else {
        result
    }
}

fn successfully_flushed_system_task_ids(
    submitted_ids: &[i64],
    result: Option<&RetainedBatch>,
) -> Vec<i64> {
    submitted_ids
        .iter()
        .copied()
        .filter(|run_id| {
            !result.is_some_and(|retained| retained.quarantined_system_task_ids.contains(run_id))
        })
        .filter(|run_id| {
            result.is_none_or(|retained| !retained.batch.system_task_finishes.contains_key(run_id))
        })
        .collect()
}

fn cleanup_discarded_p2_runtime_overlays(
    batch: &PendingBatch,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
) -> usize {
    let Some(runtime_store) = terminal_runtime_store
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
    else {
        return 0;
    };
    batch
        .invocation_derived
        .values()
        .filter_map(|derived| derived.terminal_overlay_key.as_ref())
        .filter(|(invoke_id, occurred_at)| {
            runtime_store.remove_persisted_terminal_overlay(invoke_id, occurred_at)
        })
        .count()
}

fn quarantine_system_task_batch(
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    batch: &PendingBatch,
    error: &anyhow::Error,
) -> Result<usize> {
    let mut journal = terminal_journal
        .lock()
        .map_err(|_| anyhow::anyhow!("terminal journal lock poisoned"))?;
    let journal = journal
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("terminal journal unavailable for quarantine"))?;
    let error = format!("{error:#}");
    let finishes = batch.system_task_finishes.values().collect::<Vec<_>>();
    journal.quarantine_system_task_finishes(&finishes, &error)?;
    let run_ids = finishes
        .iter()
        .map(|finish| finish.run_id)
        .collect::<Vec<_>>();
    journal.remove_deferred_system_task_finishes(&run_ids);
    Ok(batch.system_task_finishes.len())
}

fn shutdown_recovery_batch(batch: &PendingBatch) -> Option<PendingBatch> {
    if batch.terminal_invocations.is_empty() && batch.system_task_finishes.is_empty() {
        return None;
    }
    let mut quarantine = PendingBatch {
        terminal_invocations: batch.terminal_invocations.clone(),
        system_task_finishes: batch.system_task_finishes.clone(),
        ..PendingBatch::default()
    };
    quarantine.recalculate_estimates();
    quarantine.enqueued_rows = quarantine.logical_rows();
    Some(quarantine)
}

fn quarantine_shutdown_batch(
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    database_path: &std::path::Path,
    batch: &PendingBatch,
    error: &str,
) -> Result<()> {
    if batch.terminal_invocations.is_empty() && batch.system_task_finishes.is_empty() {
        return Ok(());
    }
    let error = error.to_string();
    let terminals = batch.terminal_invocations.values().collect::<Vec<_>>();
    let finishes = batch.system_task_finishes.values().collect::<Vec<_>>();
    let mut errors = Vec::new();
    let mut recovery_persisted = false;
    match terminal_journal.lock() {
        Ok(mut guard) => match guard.as_mut() {
            Some(journal) => {
                if let Err(err) = journal.quarantine_terminals(&terminals, &error) {
                    errors.push(format!("terminal quarantine failed: {err:#}"));
                }
                if let Err(err) = journal.quarantine_system_task_finishes(&finishes, &error) {
                    errors.push(format!("system-task quarantine failed: {err:#}"));
                }
                if !errors.is_empty() {
                    match journal.quarantine_shutdown_batch(&terminals, &finishes, &error) {
                        Ok(()) => {
                            journal.remember_shutdown_recovery(&terminals);
                            recovery_persisted = true;
                        }
                        Err(err) => errors.push(format!("shutdown recovery failed: {err:#}")),
                    }
                }
            }
            None => errors.push("terminal journal unavailable for shutdown quarantine".to_string()),
        },
        Err(_) => errors.push("terminal journal lock poisoned".to_string()),
    }
    if recovery_persisted {
        warn!(errors = ?errors, "shutdown quarantine used the journal recovery sink");
        return Ok(());
    }
    if errors.is_empty() {
        Ok(())
    } else {
        match TerminalJournal::quarantine_shutdown_batch_at_database_path(
            database_path,
            &terminals,
            &finishes,
            &error,
        ) {
            Ok(()) => {
                if let Ok(mut guard) = terminal_journal.lock()
                    && let Some(journal) = guard.as_mut()
                {
                    journal.remember_shutdown_recovery(&terminals);
                }
                warn!(
                    errors = ?errors,
                    "shutdown quarantine used the independent recovery sink"
                );
                Ok(())
            }
            Err(fallback_error) => Err(anyhow!(
                "{}; independent recovery sink failed: {fallback_error:#}",
                errors.join("; ")
            )),
        }
    }
}

fn release_shutdown_pending_batch(
    accounting: &PendingQueueAccounting,
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    database_path: &std::path::Path,
    batch: &PendingBatch,
    reason: &str,
) -> Result<()> {
    let quarantine_error =
        quarantine_shutdown_batch(terminal_journal, database_path, batch, reason).err();
    accounting.release(batch.logical_rows(), batch.estimated_memory_bytes());
    quarantine_error.map_or(Ok(()), Err)
}

#[expect(
    clippy::too_many_arguments,
    reason = "Flush dependencies mirror the single-writer ownership boundaries."
)]
pub(crate) async fn flush_pending_batch(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    mut batch: PendingBatch,
    reason: FlushReason,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache: &Arc<
        std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>,
    >,
    summary_delta_hub: &Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    terminal_projection_hub: &Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: &Arc<Mutex<()>>,
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
) -> Option<RetainedBatch> {
    if batch.is_empty() {
        return None;
    }
    let started = Instant::now();
    let enqueued_rows = batch.enqueued_rows;
    let coalesced_rows = batch.coalesced_rows;
    let terminal_invocation_count = batch.terminal_invocations.len();
    let attempt_count = batch.attempt_progress.len();
    let invocation_count = batch.invocation_derived.len();
    let account_touch_count = batch.account_selected_touches.len();
    let system_task_count = batch.system_task_finishes.len();
    let system_task_scope = summarize_system_task_batch_scope(&batch);
    let oldest_age_ms = batch.age().as_millis() as u64;

    let flush_reason = reason.as_str();
    let p2_pending_before_p1 = batch.has_p2();
    let p1_batch = batch.take_p1_terminals();
    if !p1_batch.is_empty() {
        let transaction_id = format!("p1-{}", started.elapsed().as_nanos());
        let permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
            .await;
        let lock_wait_ms = permit.lock_wait().as_millis() as u64;
        let execute_started = Instant::now();
        let initial_result = flush_pending_batch_inner(
            pool,
            &p1_batch,
            pricing_catalog,
            prompt_cache_conversation_cache,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            summary_delta_hub,
            terminal_projection_hub,
            dashboard_reconcile_gate,
        )
        .await;
        let mut poison_record_count = 0_usize;
        let p1_result = match initial_result {
            Err(err) if !is_sqlite_lock_error(&err) => {
                let mut deferred = PendingBatch::default();
                let mut isolation_error = None;
                for terminal in p1_batch.terminal_invocations.values() {
                    let mut singleton = PendingBatch::default();
                    singleton.push(SqliteBatchWrite::TerminalInvocation(terminal.clone()));
                    match flush_pending_batch_inner(
                        pool,
                        &singleton,
                        pricing_catalog,
                        prompt_cache_conversation_cache,
                        terminal_runtime_store,
                        dashboard_activity_snapshot_cache,
                        summary_delta_hub,
                        terminal_projection_hub,
                        dashboard_reconcile_gate,
                    )
                    .await
                    {
                        Ok(singleton_deferred) => deferred.merge_p2(singleton_deferred),
                        Err(singleton_err) if !is_sqlite_lock_error(&singleton_err) => {
                            let quarantine_result = terminal_journal
                                .lock()
                                .ok()
                                .and_then(|mut journal| {
                                    journal.as_mut().map(|journal| {
                                        journal.quarantine(terminal, &format!("{singleton_err:#}"))
                                    })
                                })
                                .unwrap_or_else(|| {
                                    Err(anyhow!("terminal journal unavailable for quarantine"))
                                });
                            if let Err(quarantine_err) = quarantine_result {
                                isolation_error = Some(quarantine_err);
                                break;
                            }
                            poison_record_count = poison_record_count.saturating_add(1);
                            warn!(
                                invoke_id = %terminal.record.invoke_id,
                                occurred_at = %terminal.record.occurred_at,
                                error = %singleton_err,
                                poison_record_count,
                                "quarantined deterministic P1 terminal record"
                            );
                        }
                        Err(singleton_err) => {
                            isolation_error = Some(singleton_err);
                            break;
                        }
                    }
                }
                match isolation_error {
                    Some(error) => Err(error),
                    None => Ok(deferred),
                }
            }
            result => result,
        };
        match p1_result {
            Ok(deferred) => {
                debug!(
                    write_class = permit.write_class(),
                    transaction_id,
                    batch_rows = p1_batch.logical_rows(),
                    batch_bytes = p1_batch.estimated_memory_bytes(),
                    lock_wait_ms,
                    execute_ms = execute_started.elapsed().as_millis() as u64,
                    poison_record_count,
                    "proxy sqlite coordinated P1 batch committed"
                );
                accounting.transfer_p1_to_p2(deferred.estimated_memory_bytes());
                if let Ok(mut journal) = terminal_journal.lock()
                    && let Some(journal) = journal.as_mut()
                {
                    for terminal in p1_batch.terminal_invocations.values() {
                        journal.acknowledge(
                            &terminal.record.invoke_id,
                            &terminal.record.occurred_at,
                            terminal.raw_capture,
                        );
                    }
                }
                batch.merge_p2(deferred);
            }
            Err(err) => {
                crate::db_pressure::global_db_pressure_gate()
                    .record_error("sqlite_batch_writer_p1", &err);
                warn!(
                    error = %err,
                    flush_priority = "P1",
                    terminal_invocation_count,
                    oldest_age_ms,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    flush_reason,
                    write_class = permit.write_class(),
                    transaction_id,
                    batch_rows = p1_batch.logical_rows(),
                    batch_bytes = p1_batch.estimated_memory_bytes(),
                    lock_wait_ms,
                    execute_ms = execute_started.elapsed().as_millis() as u64,
                    "sqlite batch writer P1 terminal flush failed"
                );
                batch.terminal_invocations = p1_batch.terminal_invocations;
                batch.recalculate_estimates();
                return Some(RetainedBatch::new(batch, true));
            }
        }
    }

    if batch.is_empty() {
        return None;
    }
    if !p2_pending_before_p1 && !reason.bypass_pressure_gate() {
        return Some(RetainedBatch::new(batch, false));
    }
    let observed_eligibility_generation =
        crate::db_pressure::global_db_pressure_gate().eligibility_generation();
    let Some(mut write_permit) =
        crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
    else {
        accounting.p2_pressure_deferred();
        return Some(RetainedBatch::p2_deferred(
            batch,
            P2DeferReason::BackgroundBusy {
                observed_generation: observed_eligibility_generation,
            },
        ));
    };

    let permit = if reason.bypass_pressure_gate() {
        None
    } else {
        match crate::db_pressure::global_db_pressure_gate()
            .try_begin_background("sqlite_batch_writer_p2")
        {
            Ok(permit) => Some(permit),
            Err(deny_reason) => {
                if matches!(
                    deny_reason,
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy
                ) {
                    write_permit.suppress_background_eligibility_wakeup();
                }
                drop(write_permit);
                accounting.p2_pressure_deferred();
                debug!(
                    deny_reason = %deny_reason,
                    flush_priority = "P2",
                    p2_deferred_count = batch.logical_rows(),
                    "sqlite batch writer deferred P2 flush because pressure gate is closed"
                );
                let reason = match deny_reason {
                    crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms } => {
                        P2DeferReason::PressureCooldown(remaining_ms)
                    }
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                        P2DeferReason::BackgroundBusy {
                            observed_generation: observed_eligibility_generation,
                        }
                    }
                };
                return Some(RetainedBatch::p2_deferred(batch, reason));
            }
        }
    };
    accounting.p2_attempted();

    // A deterministic failure in a derived write must not retain unrelated
    // system-task completions forever. Flush those completions separately so
    // their final state is durable even when another P2 write is discarded.
    let system_task_batch = if !batch.system_task_finishes.is_empty() {
        let mut system_task_batch = PendingBatch {
            oldest_at: batch.oldest_at,
            ..PendingBatch::default()
        };
        system_task_batch.system_task_finishes = std::mem::take(&mut batch.system_task_finishes);
        system_task_batch.recalculate_estimates();
        system_task_batch.enqueued_rows = system_task_batch.logical_rows();
        batch.recalculate_estimates();
        Some(system_task_batch)
    } else {
        None
    };

    let mut deferred_batch = PendingBatch::default();
    let mut system_task_failure = None;
    let mut system_task_lock_failure = false;
    if let Some(system_task_batch) = system_task_batch.as_ref() {
        match flush_pending_batch_inner(
            pool,
            system_task_batch,
            pricing_catalog,
            prompt_cache_conversation_cache,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            summary_delta_hub,
            terminal_projection_hub,
            dashboard_reconcile_gate,
        )
        .await
        {
            Ok(system_task_deferred) => deferred_batch.merge_p2(system_task_deferred),
            Err(err) => {
                crate::db_pressure::global_db_pressure_gate()
                    .record_error("sqlite_batch_writer_p2", &err);
                warn!(
                    error = %err,
                    flush_priority = "P2",
                    p2_deferred_count = system_task_batch.logical_rows(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    flush_reason,
                    system_task_scope = %summarize_system_task_batch_scope(system_task_batch),
                    "sqlite batch writer P2 system-task flush failed"
                );
                system_task_lock_failure = is_sqlite_lock_error(&err);
                system_task_failure = Some(err);
            }
        }
    }
    // Retry lock/pressure failures, but quarantine deterministic SQL failures. Every finish is
    // journaled before enqueue, so quarantine remains durable without repeatedly stressing SQLite.
    let system_task_retryable_failure = system_task_failure
        .as_ref()
        .is_some_and(crate::db_pressure::is_db_pressure_error);

    match flush_pending_batch_inner(
        pool,
        &batch,
        pricing_catalog,
        prompt_cache_conversation_cache,
        terminal_runtime_store,
        dashboard_activity_snapshot_cache,
        summary_delta_hub,
        terminal_projection_hub,
        dashboard_reconcile_gate,
    )
    .await
    {
        Ok(main_deferred) => {
            deferred_batch.merge_p2(main_deferred);
            if let Some(system_task_error) = system_task_failure.as_ref() {
                if !system_task_retryable_failure {
                    let system_task_batch_ref = system_task_batch
                        .as_ref()
                        .expect("system task failure must retain its isolated batch");
                    let quarantined_system_task_ids = system_task_batch_ref
                        .system_task_finishes
                        .keys()
                        .copied()
                        .collect::<Vec<_>>();
                    let quarantine_result = quarantine_system_task_batch(
                        terminal_journal,
                        system_task_batch_ref,
                        system_task_error,
                    );
                    if let Err(quarantine_error) = quarantine_result {
                        warn!(
                            error = %quarantine_error,
                            flush_priority = "P2",
                            system_task_scope = %summarize_system_task_batch_scope(
                                system_task_batch_ref
                            ),
                            "system-task quarantine failed; retaining completion for retry"
                        );
                        let mut retry_batch = system_task_batch
                            .expect("system task failure must retain its isolated batch");
                        retry_batch.merge_p2(deferred_batch);
                        drop(permit);
                        return Some(RetainedBatch::p2_failed(retry_batch, true, false));
                    }
                    warn!(
                        flush_priority = "P2",
                        system_task_scope = %summarize_system_task_batch_scope(system_task_batch_ref),
                        "quarantined deterministic system-task completion after failed finalization"
                    );
                    let retained = if deferred_batch.is_empty() {
                        RetainedBatch::new(PendingBatch::default(), false)
                    } else {
                        RetainedBatch::new(deferred_batch, false)
                    }
                    .with_quarantined_system_task_ids(quarantined_system_task_ids);
                    drop(permit);
                    return Some(retained);
                }
                let mut retry_batch =
                    system_task_batch.expect("system task failure must retain its isolated batch");
                retry_batch.merge_p2(deferred_batch);
                drop(permit);
                return Some(RetainedBatch::p2_failed(
                    retry_batch,
                    system_task_retryable_failure,
                    system_task_lock_failure,
                ));
            }
        }
        Err(err) => {
            crate::db_pressure::global_db_pressure_gate()
                .record_error("sqlite_batch_writer_p2", &err);
            warn!(
                error = %err,
                flush_priority = "P2",
                p2_deferred_count = batch.logical_rows(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                flush_reason,
                "sqlite batch writer P2 flush failed"
            );
            drop(permit);
            if !crate::db_pressure::is_db_pressure_error(&err)
                && let Some(system_task_error) = system_task_failure.as_ref()
            {
                cleanup_discarded_p2_runtime_overlays(&batch, terminal_runtime_store);
                let retry_batch =
                    system_task_batch.expect("system task failure must retain its isolated batch");
                let quarantined_system_task_ids = retry_batch
                    .system_task_finishes
                    .keys()
                    .copied()
                    .collect::<Vec<_>>();
                if !system_task_retryable_failure
                    && quarantine_system_task_batch(
                        terminal_journal,
                        &retry_batch,
                        system_task_error,
                    )
                    .is_ok()
                {
                    return Some(
                        RetainedBatch::new(PendingBatch::default(), false)
                            .with_quarantined_system_task_ids(quarantined_system_task_ids),
                    );
                }
                return Some(RetainedBatch::p2_failed(
                    retry_batch,
                    system_task_retryable_failure,
                    system_task_lock_failure,
                ));
            }
            if let Some(system_task_batch) = system_task_batch {
                if let Some(system_task_error) = system_task_failure.as_ref() {
                    let quarantined_system_task_ids = system_task_batch
                        .system_task_finishes
                        .keys()
                        .copied()
                        .collect::<Vec<_>>();
                    if !system_task_retryable_failure
                        && quarantine_system_task_batch(
                            terminal_journal,
                            &system_task_batch,
                            system_task_error,
                        )
                        .is_ok()
                    {
                        return Some(
                            RetainedBatch::p2_failed(
                                batch,
                                crate::db_pressure::is_db_pressure_error(&err),
                                is_sqlite_lock_error(&err),
                            )
                            .with_quarantined_system_task_ids(quarantined_system_task_ids),
                        );
                    }
                    let mut retry_batch = system_task_batch;
                    retry_batch.merge_p2(batch);
                    return Some(RetainedBatch::p2_failed(
                        retry_batch,
                        system_task_retryable_failure
                            || crate::db_pressure::is_db_pressure_error(&err),
                        system_task_lock_failure || is_sqlite_lock_error(&err),
                    ));
                }
                if !crate::db_pressure::is_db_pressure_error(&err) {
                    cleanup_discarded_p2_runtime_overlays(&batch, terminal_runtime_store);
                    return None;
                }
                return Some(RetainedBatch::p2_failed(
                    batch,
                    true,
                    is_sqlite_lock_error(&err),
                ));
            }
            let retryable_failure = crate::db_pressure::is_db_pressure_error(&err);
            return Some(RetainedBatch::p2_failed(
                batch,
                retryable_failure,
                is_sqlite_lock_error(&err),
            ));
        }
    }
    drop(write_permit);
    drop(permit);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        warn!(
            enqueued_rows,
            coalesced_rows,
            terminal_invocation_count,
            attempt_count,
            invocation_count,
            account_touch_count,
            system_task_count,
            system_task_scope = %system_task_scope,
            oldest_age_ms,
            elapsed_ms,
            flush_reason,
            "sqlite batch writer flush was slow"
        );
    } else {
        debug!(
            enqueued_rows,
            coalesced_rows,
            terminal_invocation_count,
            attempt_count,
            invocation_count,
            account_touch_count,
            system_task_count,
            system_task_scope = %system_task_scope,
            oldest_age_ms,
            elapsed_ms,
            flush_reason,
            "sqlite batch writer flushed derived writes"
        );
    }
    if deferred_batch.is_empty() {
        None
    } else {
        Some(RetainedBatch::new(deferred_batch, false))
    }
}
