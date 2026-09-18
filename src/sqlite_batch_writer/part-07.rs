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

#[derive(Clone, Copy)]
struct SqliteBatchFlushDependencies<'a>(
    Option<&'a Arc<Mutex<PromptCacheConversationsCacheState>>>,
    &'a Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    &'a Arc<std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>>,
    &'a Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    &'a Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    &'a Arc<Mutex<()>>,
    &'a Arc<std::sync::Mutex<Option<TerminalJournal>>>,
);

async fn flush_pending_batch_accounted(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    batch: PendingBatch,
    reason: FlushReason,
    dependencies: SqliteBatchFlushDependencies<'_>,
) -> Option<RetainedBatch> {
    let SqliteBatchFlushDependencies(_, terminal_runtime_store, _, _, _, _, terminal_journal) =
        dependencies;
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
        dependencies,
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

async fn flush_pending_batch(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    batch: PendingBatch,
    reason: FlushReason,
    dependencies: SqliteBatchFlushDependencies<'_>,
) -> Option<RetainedBatch> {
    flush_pending_batch_converged(
        accounting,
        pool,
        pricing_catalog,
        batch,
        reason,
        dependencies,
    )
    .await
}

#[derive(Debug)]
struct PendingBatchFlushStats {
    started: Instant,
    enqueued_rows: usize,
    coalesced_rows: usize,
    terminal_invocation_count: usize,
    attempt_count: usize,
    invocation_count: usize,
    account_touch_count: usize,
    system_task_count: usize,
    system_task_scope: String,
    oldest_age_ms: u64,
    flush_reason: FlushReason,
}

impl PendingBatchFlushStats {
    fn from_batch(batch: &PendingBatch, reason: FlushReason) -> Self {
        Self {
            started: Instant::now(),
            enqueued_rows: batch.enqueued_rows,
            coalesced_rows: batch.coalesced_rows,
            terminal_invocation_count: batch.terminal_invocations.len(),
            attempt_count: batch.attempt_progress.len(),
            invocation_count: batch.invocation_derived.len(),
            account_touch_count: batch.account_selected_touches.len(),
            system_task_count: batch.system_task_finishes.len(),
            system_task_scope: summarize_system_task_batch_scope(batch),
            oldest_age_ms: batch.age().as_millis() as u64,
            flush_reason: reason,
        }
    }
}

async fn flush_pending_batch_converged(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    mut batch: PendingBatch,
    reason: FlushReason,
    dependencies: SqliteBatchFlushDependencies<'_>,
) -> Option<RetainedBatch> {
    if batch.is_empty() {
        return None;
    }
    let stats = PendingBatchFlushStats::from_batch(&batch, reason);
    let p2_pending_before_p1 = batch.has_p2();
    batch = match flush_p1_stage(
        accounting,
        pool,
        pricing_catalog,
        batch,
        dependencies,
        &stats,
    )
    .await
    {
        Ok(batch) => batch,
        Err(batch) => return Some(RetainedBatch::new(batch, true)),
    };
    if batch.is_empty() {
        return None;
    }
    if !p2_pending_before_p1 && !reason.bypass_pressure_gate() {
        return Some(RetainedBatch::new(batch, false));
    }
    flush_p2_stage(
        accounting,
        pool,
        pricing_catalog,
        batch,
        dependencies,
        stats,
    )
    .await
}

async fn flush_p1_stage(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    mut batch: PendingBatch,
    dependencies: SqliteBatchFlushDependencies<'_>,
    stats: &PendingBatchFlushStats,
) -> Result<PendingBatch, PendingBatch> {
    let p1_batch = batch.take_p1_terminals();
    if p1_batch.is_empty() {
        return Ok(batch);
    }
    let SqliteBatchFlushDependencies(_, _, _, _, _, _, terminal_journal) = dependencies;
    let transaction_id = format!("p1-{}", stats.started.elapsed().as_nanos());
    let permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    let lock_wait_ms = permit.lock_wait().as_millis() as u64;
    let execute_started = Instant::now();
    let initial_result =
        flush_pending_batch_inner(pool, &p1_batch, pricing_catalog, dependencies).await;
    let (p1_result, poison_record_count) = match initial_result {
        Err(err) if !is_sqlite_lock_error(&err) => {
            match isolate_deterministic_p1_failure(pool, pricing_catalog, &p1_batch, dependencies)
                .await
            {
                Ok((deferred, count)) => (Ok(deferred), count),
                Err(error) => (Err(error), 0),
            }
        }
        result => (result, 0),
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
            acknowledge_p1_journal_records(terminal_journal, &p1_batch);
            batch.merge_p2(deferred);
            Ok(batch)
        }
        Err(err) => {
            crate::db_pressure::global_db_pressure_gate()
                .record_error("sqlite_batch_writer_p1", &err);
            warn_p1_flush_failure(
                &err,
                stats,
                permit.write_class(),
                transaction_id,
                &p1_batch,
                lock_wait_ms,
                execute_started,
            );
            batch.terminal_invocations = p1_batch.terminal_invocations;
            batch.recalculate_estimates();
            Err(batch)
        }
    }
}

async fn isolate_deterministic_p1_failure(
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    p1_batch: &PendingBatch,
    dependencies: SqliteBatchFlushDependencies<'_>,
) -> Result<(PendingBatch, usize)> {
    let SqliteBatchFlushDependencies(_, _, _, _, _, _, terminal_journal) = dependencies;
    let mut deferred = PendingBatch::default();
    let mut poison_record_count = 0_usize;
    for terminal in p1_batch.terminal_invocations.values() {
        let mut singleton = PendingBatch::default();
        singleton.push(SqliteBatchWrite::TerminalInvocation(terminal.clone()));
        match flush_pending_batch_inner(pool, &singleton, pricing_catalog, dependencies).await {
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
                    .unwrap_or_else(|| Err(anyhow!("terminal journal unavailable for quarantine")));
                quarantine_result?;
                poison_record_count = poison_record_count.saturating_add(1);
                warn!(
                    invoke_id = %terminal.record.invoke_id,
                    occurred_at = %terminal.record.occurred_at,
                    error = %singleton_err,
                    poison_record_count,
                    "quarantined deterministic P1 terminal record"
                );
            }
            Err(singleton_err) => return Err(singleton_err),
        }
    }
    Ok((deferred, poison_record_count))
}

fn acknowledge_p1_journal_records(
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    p1_batch: &PendingBatch,
) {
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
}

fn warn_p1_flush_failure(
    err: &anyhow::Error,
    stats: &PendingBatchFlushStats,
    write_class: &str,
    transaction_id: String,
    p1_batch: &PendingBatch,
    lock_wait_ms: u64,
    execute_started: Instant,
) {
    warn!(
        error = %err,
        flush_priority = "P1",
        terminal_invocation_count = stats.terminal_invocation_count,
        oldest_age_ms = stats.oldest_age_ms,
        elapsed_ms = stats.started.elapsed().as_millis() as u64,
        flush_reason = stats.flush_reason.as_str(),
        write_class,
        transaction_id,
        batch_rows = p1_batch.logical_rows(),
        batch_bytes = p1_batch.estimated_memory_bytes(),
        lock_wait_ms,
        execute_ms = execute_started.elapsed().as_millis() as u64,
        "sqlite batch writer P1 terminal flush failed"
    );
}

#[derive(Debug)]
struct SystemTaskFlushResult {
    deferred_batch: PendingBatch,
    failure: Option<anyhow::Error>,
    lock_failure: bool,
}

async fn flush_p2_stage(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    mut batch: PendingBatch,
    dependencies: SqliteBatchFlushDependencies<'_>,
    stats: PendingBatchFlushStats,
) -> Option<RetainedBatch> {
    let SqliteBatchFlushDependencies(_, terminal_runtime_store, _, _, _, _, terminal_journal) =
        dependencies;
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
    let permit = match try_begin_p2_flush(
        &mut write_permit,
        &batch,
        stats.flush_reason,
        observed_eligibility_generation,
    ) {
        Ok(permit) => permit,
        Err(reason) => {
            drop(write_permit);
            accounting.p2_pressure_deferred();
            return Some(RetainedBatch::p2_deferred(batch, reason));
        }
    };
    accounting.p2_attempted();
    let system_task_batch = split_system_task_batch(&mut batch);
    let mut isolated = flush_isolated_system_task_batch(
        pool,
        pricing_catalog,
        system_task_batch.as_ref(),
        dependencies,
        &stats,
    )
    .await;
    let main_result = flush_pending_batch_inner(pool, &batch, pricing_catalog, dependencies).await;
    match main_result {
        Ok(main_deferred) => {
            isolated.deferred_batch.merge_p2(main_deferred);
            if let Some(retained) = finish_p2_system_task_result(
                &mut isolated.deferred_batch,
                system_task_batch,
                isolated.failure.as_ref(),
                isolated.lock_failure,
                terminal_journal,
            ) {
                drop(permit);
                drop(write_permit);
                return Some(retained);
            }
        }
        Err(err) => {
            crate::db_pressure::global_db_pressure_gate()
                .record_error("sqlite_batch_writer_p2", &err);
            warn!(
                error = %err,
                flush_priority = "P2",
                p2_deferred_count = batch.logical_rows(),
                elapsed_ms = stats.started.elapsed().as_millis() as u64,
                flush_reason = stats.flush_reason.as_str(),
                "sqlite batch writer P2 flush failed"
            );
            drop(permit);
            drop(write_permit);
            return finish_p2_flush_error(
                batch,
                system_task_batch,
                isolated,
                err,
                terminal_journal,
                terminal_runtime_store,
            );
        }
    }
    drop(permit);
    drop(write_permit);
    log_p2_flush_success(&stats, &isolated.deferred_batch);
    if isolated.deferred_batch.is_empty() {
        None
    } else {
        Some(RetainedBatch::new(isolated.deferred_batch, false))
    }
}

fn try_begin_p2_flush(
    write_permit: &mut crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
    batch: &PendingBatch,
    flush_reason: FlushReason,
    observed_eligibility_generation: u64,
) -> Result<Option<crate::db_pressure::DbBackgroundPermit>, P2DeferReason> {
    if flush_reason.bypass_pressure_gate() {
        return Ok(None);
    }
    match crate::db_pressure::global_db_pressure_gate()
        .try_begin_background("sqlite_batch_writer_p2")
    {
        Ok(permit) => Ok(Some(permit)),
        Err(deny_reason) => {
            if matches!(
                deny_reason,
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy
            ) {
                write_permit.suppress_background_eligibility_wakeup();
            }
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
            debug!(
                deny_reason = %deny_reason,
                flush_priority = "P2",
                p2_deferred_count = batch.logical_rows(),
                "sqlite batch writer deferred P2 flush because pressure gate is closed"
            );
            Err(reason)
        }
    }
}

fn split_system_task_batch(batch: &mut PendingBatch) -> Option<PendingBatch> {
    if batch.system_task_finishes.is_empty() {
        return None;
    }
    let mut system_task_batch = PendingBatch {
        oldest_at: batch.oldest_at,
        ..PendingBatch::default()
    };
    system_task_batch.system_task_finishes = std::mem::take(&mut batch.system_task_finishes);
    system_task_batch.recalculate_estimates();
    system_task_batch.enqueued_rows = system_task_batch.logical_rows();
    batch.recalculate_estimates();
    Some(system_task_batch)
}

async fn flush_isolated_system_task_batch(
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    system_task_batch: Option<&PendingBatch>,
    dependencies: SqliteBatchFlushDependencies<'_>,
    stats: &PendingBatchFlushStats,
) -> SystemTaskFlushResult {
    let Some(system_task_batch) = system_task_batch else {
        return SystemTaskFlushResult {
            deferred_batch: PendingBatch::default(),
            failure: None,
            lock_failure: false,
        };
    };
    match flush_pending_batch_inner(pool, system_task_batch, pricing_catalog, dependencies).await {
        Ok(deferred_batch) => SystemTaskFlushResult {
            deferred_batch,
            failure: None,
            lock_failure: false,
        },
        Err(err) => {
            crate::db_pressure::global_db_pressure_gate()
                .record_error("sqlite_batch_writer_p2", &err);
            warn!(
                error = %err,
                flush_priority = "P2",
                p2_deferred_count = system_task_batch.logical_rows(),
                elapsed_ms = stats.started.elapsed().as_millis() as u64,
                flush_reason = stats.flush_reason.as_str(),
                system_task_scope = %summarize_system_task_batch_scope(system_task_batch),
                "sqlite batch writer P2 system-task flush failed"
            );
            SystemTaskFlushResult {
                deferred_batch: PendingBatch::default(),
                lock_failure: is_sqlite_lock_error(&err),
                failure: Some(err),
            }
        }
    }
}

fn finish_p2_system_task_result(
    deferred_batch: &mut PendingBatch,
    system_task_batch: Option<PendingBatch>,
    system_task_error: Option<&anyhow::Error>,
    system_task_lock_failure: bool,
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
) -> Option<RetainedBatch> {
    let system_task_error = system_task_error?;
    let system_task_batch_ref = system_task_batch
        .as_ref()
        .expect("system task failure must retain its isolated batch");
    let retryable = crate::db_pressure::is_db_pressure_error(system_task_error);
    if retryable {
        let mut retry_batch =
            system_task_batch.expect("system task failure must retain its isolated batch");
        retry_batch.merge_p2(std::mem::take(deferred_batch));
        return Some(RetainedBatch::p2_failed(
            retry_batch,
            true,
            system_task_lock_failure,
        ));
    }
    let quarantined_ids = system_task_batch_ref
        .system_task_finishes
        .keys()
        .copied()
        .collect::<Vec<_>>();
    if let Err(quarantine_error) =
        quarantine_system_task_batch(terminal_journal, system_task_batch_ref, system_task_error)
    {
        warn!(
            error = %quarantine_error,
            flush_priority = "P2",
            system_task_scope = %summarize_system_task_batch_scope(system_task_batch_ref),
            "system-task quarantine failed; retaining completion for retry"
        );
        let mut retry_batch =
            system_task_batch.expect("system task failure must retain its isolated batch");
        retry_batch.merge_p2(std::mem::take(deferred_batch));
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
        RetainedBatch::new(std::mem::take(deferred_batch), false)
    };
    Some(retained.with_quarantined_system_task_ids(quarantined_ids))
}

fn finish_p2_flush_error(
    batch: PendingBatch,
    system_task_batch: Option<PendingBatch>,
    isolated: SystemTaskFlushResult,
    err: anyhow::Error,
    terminal_journal: &Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
) -> Option<RetainedBatch> {
    let main_retryable = crate::db_pressure::is_db_pressure_error(&err);
    if let Some(system_task_batch) = system_task_batch {
        if let Some(system_task_error) = isolated.failure.as_ref() {
            let quarantined_ids = system_task_batch
                .system_task_finishes
                .keys()
                .copied()
                .collect::<Vec<_>>();
            if !crate::db_pressure::is_db_pressure_error(system_task_error)
                && quarantine_system_task_batch(
                    terminal_journal,
                    &system_task_batch,
                    system_task_error,
                )
                .is_ok()
            {
                return Some(
                    RetainedBatch::p2_failed(batch, main_retryable, is_sqlite_lock_error(&err))
                        .with_quarantined_system_task_ids(quarantined_ids),
                );
            }
            let mut retry_batch = system_task_batch;
            retry_batch.merge_p2(batch);
            return Some(RetainedBatch::p2_failed(
                retry_batch,
                isolated
                    .failure
                    .as_ref()
                    .is_some_and(crate::db_pressure::is_db_pressure_error)
                    || main_retryable,
                isolated.lock_failure || is_sqlite_lock_error(&err),
            ));
        }
        if !main_retryable {
            cleanup_discarded_p2_runtime_overlays(&batch, terminal_runtime_store);
            return None;
        }
        let mut retry_batch = system_task_batch;
        retry_batch.merge_p2(batch);
        return Some(RetainedBatch::p2_failed(
            retry_batch,
            true,
            is_sqlite_lock_error(&err),
        ));
    }
    Some(RetainedBatch::p2_failed(
        batch,
        main_retryable,
        is_sqlite_lock_error(&err),
    ))
}

fn log_p2_flush_success(stats: &PendingBatchFlushStats, deferred_batch: &PendingBatch) {
    let elapsed_ms = stats.started.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        warn!(
            enqueued_rows = stats.enqueued_rows,
            coalesced_rows = stats.coalesced_rows,
            terminal_invocation_count = stats.terminal_invocation_count,
            attempt_count = stats.attempt_count,
            invocation_count = stats.invocation_count,
            account_touch_count = stats.account_touch_count,
            system_task_count = stats.system_task_count,
            system_task_scope = %stats.system_task_scope,
            oldest_age_ms = stats.oldest_age_ms,
            elapsed_ms,
            flush_reason = stats.flush_reason.as_str(),
            "sqlite batch writer flush was slow"
        );
    } else {
        debug!(
            enqueued_rows = stats.enqueued_rows,
            coalesced_rows = stats.coalesced_rows,
            terminal_invocation_count = stats.terminal_invocation_count,
            attempt_count = stats.attempt_count,
            invocation_count = stats.invocation_count,
            account_touch_count = stats.account_touch_count,
            system_task_count = stats.system_task_count,
            system_task_scope = %stats.system_task_scope,
            oldest_age_ms = stats.oldest_age_ms,
            elapsed_ms,
            flush_reason = stats.flush_reason.as_str(),
            deferred_rows = deferred_batch.logical_rows(),
            "sqlite batch writer flushed derived writes"
        );
    }
}

pub(crate) fn summarize_system_task_batch_scope(batch: &PendingBatch) -> String {
    let mut values = batch
        .system_task_finishes
        .values()
        .take(3)
        .map(|finish| {
            format!(
                "{}:{}:{}",
                finish.task_kind.as_str(),
                finish.trigger_kind,
                finish.status.as_str()
            )
        })
        .collect::<Vec<_>>();
    if batch.system_task_finishes.len() > values.len() {
        values.push(format!(
            "+{}",
            batch.system_task_finishes.len() - values.len()
        ));
    }
    values.join(",")
}
