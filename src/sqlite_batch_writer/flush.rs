use super::*;
#[expect(
    clippy::too_many_arguments,
    reason = "The accounting wrapper mirrors the single-writer ownership boundaries."
)]
pub(super) async fn flush_pending_batch_accounted(
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
    terminal_journal: &Arc<TrackedTerminalJournal>,
) -> Option<RetainedBatch> {
    let traffic_authority = crate::storage::traffic::TrafficFactStore::sqlite(pool.clone());
    flush_pending_batch_accounted_with_authority(
        accounting,
        pool,
        &traffic_authority,
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
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "The accounting wrapper mirrors the single-writer ownership boundaries."
)]
pub(super) async fn flush_pending_batch_accounted_with_authority(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    traffic_authority: &crate::storage::traffic::TrafficFactStore,
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
    terminal_journal: &Arc<TrackedTerminalJournal>,
) -> Option<RetainedBatch> {
    let was_retained_retry = batch.retained_for_retry;
    let submitted_system_task_ids = batch
        .system_task_finishes
        .keys()
        .copied()
        .collect::<Vec<_>>();
    let submitted_p1_depth = batch.terminal_invocations.len();
    let submitted_depth = batch.logical_rows();
    let submitted_bytes = batch.estimated_memory_bytes();
    if let Some(m) = accounting.observability.get() {
        m.duration(
            "cvm_sqlite_queue_wait_duration_seconds",
            &[(
                "class",
                if batch.terminal_invocations.is_empty() {
                    "p2_derived"
                } else {
                    "p1_terminal"
                },
            )],
            batch.age(),
        );
    }
    let flush_started = Instant::now();
    let flush = flush_pending_batch_with_authority(
        accounting,
        pool,
        traffic_authority,
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
    );
    let result = if let Some(metrics) = accounting.observability.runtime() {
        crate::observability::contention::with_performance(metrics, flush).await
    } else {
        flush.await
    };
    accounting.record_write_batch(
        submitted_depth,
        submitted_bytes,
        flush_started.elapsed().as_millis() as u64,
    );
    let completed_system_task_ids =
        successfully_flushed_system_task_ids(&submitted_system_task_ids, result.as_ref());
    if !completed_system_task_ids.is_empty()
        && let Ok(mut guard) = terminal_journal.lock()
        && let Some(journal) = guard.as_mut()
        && let Err(err) = journal.acknowledge_system_task_finishes(&completed_system_task_ids)
    {
        journal.defer_system_task_acknowledgements(&completed_system_task_ids);
        warn!(
            error = %err,
            completed_system_task_count = completed_system_task_ids.len(),
            "failed to acknowledge recovered system-task finishes; scheduled retry"
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
    if was_retained_retry && let Some(retained) = result.as_ref().filter(|retained| retained.failed)
    {
        if retained.batch.terminal_invocations.is_empty() {
            accounting.retry_p2_deferred();
        } else {
            accounting.retry_p1_deferred();
        }
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
    let retained_p1_depth = result
        .as_ref()
        .filter(|_| !discard_non_retryable_p2)
        .map(|r| r.batch.terminal_invocations.len())
        .unwrap_or_default();
    accounting.observe_p1_replace(submitted_p1_depth, retained_p1_depth);
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

pub(super) fn successfully_flushed_system_task_ids(
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

fn terminal_journal_ack_is_deferred(
    deferred: &PendingBatch,
    terminal: &BatchedTerminalInvocationWrite,
) -> bool {
    deferred.invocation_derived.values().any(|derived| {
        derived.terminal_journal_ack.as_ref().is_some_and(|ack| {
            ack.raw_capture == terminal.raw_capture
                && ack.record.invoke_id == terminal.record.invoke_id
                && ack.record.occurred_at == terminal.record.occurred_at
        })
    })
}

fn has_pending_terminal_journal_acks(batch: &PendingBatch) -> bool {
    batch
        .invocation_derived
        .values()
        .any(|derived| derived.terminal_journal_ack.is_some())
}

fn acknowledge_terminal_journal_acks(
    terminal_journal: &Arc<TrackedTerminalJournal>,
    batch: &PendingBatch,
) {
    for terminal in batch
        .invocation_derived
        .values()
        .filter_map(|derived| derived.terminal_journal_ack.as_ref())
    {
        terminal_journal.acknowledge(
            &terminal.record.invoke_id,
            &terminal.record.occurred_at,
            terminal.raw_capture,
            &terminal.attempts,
        );
    }
}

fn quarantine_system_task_batch(
    terminal_journal: &Arc<TrackedTerminalJournal>,
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
    let run_ids = finishes
        .iter()
        .map(|finish| finish.run_id)
        .collect::<Vec<_>>();
    let replayed_run_ids = run_ids
        .iter()
        .copied()
        .filter(|run_id| journal.system_task_recovery_replayed(*run_id))
        .collect::<Vec<_>>();
    journal.quarantine_system_task_finishes(&finishes, &error)?;
    journal.remove_deferred_system_task_finishes(&run_ids);
    journal.requeue_system_task_finishes_for_retry(&finishes, &replayed_run_ids);
    // The quarantine is durable and will be retried after restart; there is no in-process retry
    // path for a fresh deterministic failure, so do not leave unrelated tasks behind a global
    // gate. A replayed finish retains its owner-specific recovery fence until it is acknowledged.
    let fresh_run_ids = run_ids
        .iter()
        .copied()
        .filter(|run_id| !replayed_run_ids.contains(run_id))
        .collect::<Vec<_>>();
    journal.forget_system_task_recovery_for_runs(&fresh_run_ids);
    Ok(batch.system_task_finishes.len())
}

pub(super) fn shutdown_recovery_batch(batch: &PendingBatch) -> Option<PendingBatch> {
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

pub(super) fn quarantine_shutdown_batch(
    terminal_journal: &Arc<TrackedTerminalJournal>,
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

pub(super) fn release_shutdown_pending_batch(
    accounting: &PendingQueueAccounting,
    terminal_journal: &Arc<TrackedTerminalJournal>,
    database_path: &std::path::Path,
    batch: &PendingBatch,
    reason: &str,
) -> Result<()> {
    let quarantine_error =
        quarantine_shutdown_batch(terminal_journal, database_path, batch, reason).err();
    accounting.observe_p1_replace(batch.terminal_invocations.len(), 0);
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
    terminal_journal: &Arc<TrackedTerminalJournal>,
) -> Option<RetainedBatch> {
    let traffic_authority = crate::storage::traffic::TrafficFactStore::sqlite(pool.clone());
    flush_pending_batch_with_authority(
        accounting,
        pool,
        &traffic_authority,
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
    .await
}

#[expect(
    clippy::too_many_arguments,
    reason = "Flush dependencies mirror the single-writer ownership boundaries."
)]
pub(crate) async fn flush_pending_batch_with_authority(
    accounting: &PendingQueueAccounting,
    pool: &Pool<Sqlite>,
    traffic_authority: &crate::storage::traffic::TrafficFactStore,
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
    terminal_journal: &Arc<TrackedTerminalJournal>,
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
    if attempt_count > 0 {
        accounting.attempt_progress_coalesced(batch.attempt_progress_coalesced_rows);
        accounting.observe_attempt_progress_age(oldest_age_ms);
    }

    let flush_reason = reason.as_str();
    let recovery_replay = batch.recovery_replay;
    let p2_pending_before_p1 = batch.has_p2();
    let p1_batch = batch.take_p1_terminals();
    if !p1_batch.is_empty() {
        let transaction_id = format!("p1-{}", started.elapsed().as_nanos());
        let diagnostic_admission = diagnostics::begin_batch(&p1_batch);
        let permit = if recovery_replay {
            match timeout(
                crate::runtime::STARTUP_RECOVERY_ADMISSION_BUDGET,
                crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator().acquire(
                    crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal,
                ),
            )
            .await
            {
                Ok(permit) => permit,
                Err(_) => {
                    batch.terminal_invocations = p1_batch.terminal_invocations;
                    batch.recovery_replay = true;
                    batch.recalculate_estimates();
                    warn!(
                        admission_budget_ms =
                            crate::runtime::STARTUP_RECOVERY_ADMISSION_BUDGET.as_millis() as u64,
                        "startup terminal journal replay deferred before SQLite write admission"
                    );
                    return Some(RetainedBatch::new(batch, true));
                }
            }
        } else {
            crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
                .await
        };
        let admission_context = diagnostic_admission.coordinator_admitted();
        reference_batch(&p1_batch, admission_context);
        let lock_wait_ms = permit.lock_wait().as_millis() as u64;
        let execute_started = Instant::now();
        let initial_result = flush_pending_batch_inner_with_authority(
            pool,
            traffic_authority,
            &p1_batch,
            pricing_catalog,
            prompt_cache_conversation_cache,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            summary_delta_hub,
            terminal_projection_hub,
            dashboard_reconcile_gate,
            accounting.observability.get(),
        )
        .await;
        let mut poison_record_count = 0_usize;
        let mut quarantined_terminal_keys = HashSet::new();
        let p1_result = match initial_result {
            Err(err) if !is_batch_writer_sqlite_lock_error(&err) => {
                let mut deferred = PendingBatch::default();
                let mut isolation_error = None;
                for terminal in p1_batch.terminal_invocations.values() {
                    let mut singleton = PendingBatch::default();
                    singleton.push(SqliteBatchWrite::TerminalInvocation(terminal.clone()));
                    singleton.recovery_replay = recovery_replay;
                    match flush_pending_batch_inner_with_authority(
                        pool,
                        traffic_authority,
                        &singleton,
                        pricing_catalog,
                        prompt_cache_conversation_cache,
                        terminal_runtime_store,
                        dashboard_activity_snapshot_cache,
                        summary_delta_hub,
                        terminal_projection_hub,
                        dashboard_reconcile_gate,
                        accounting.observability.get(),
                    )
                    .await
                    {
                        Ok(singleton_deferred) => deferred.merge_p2(singleton_deferred),
                        Err(singleton_err)
                            if !is_batch_writer_sqlite_lock_error(&singleton_err) =>
                        {
                            let quarantine_result = terminal_journal
                                .lock()
                                .ok()
                                .and_then(|mut journal| {
                                    journal.as_mut().map(|journal| {
                                        let result = journal
                                            .quarantine(terminal, "terminal_materialization_failed")
                                            .and_then(|()| {
                                                journal.quarantine_shutdown_batch(
                                                    &[terminal],
                                                    &[],
                                                    "terminal_materialization_failed",
                                                )
                                            });
                                        if result.is_ok()
                                            && !journal.remember_shutdown_recovery(&[terminal])
                                        {
                                            return Err(anyhow!(
                                                "terminal shutdown recovery capacity exhausted"
                                            ));
                                        }
                                        result
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
                            quarantined_terminal_keys.insert((
                                terminal.record.invoke_id.clone(),
                                terminal.record.occurred_at.clone(),
                                terminal.raw_capture,
                            ));
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
                accounting.record_p1_ack_duration(
                    lock_wait_ms.saturating_add(execute_started.elapsed().as_millis() as u64),
                );

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
                for terminal in p1_batch.terminal_invocations.values() {
                    if let Some(cache) = prompt_cache_conversation_cache {
                        cache
                            .lock()
                            .await
                            .identity_cache
                            .range_manager
                            .reconcile_persistence(&terminal.record.invoke_id);
                    }
                    // Pending namespace protection covers the await above; only
                    // release it after the old in-memory reference is reconciled.
                    if !quarantined_terminal_keys.contains(&(
                        terminal.record.invoke_id.clone(),
                        terminal.record.occurred_at.clone(),
                        terminal.raw_capture,
                    )) && !terminal_journal_ack_is_deferred(&deferred, terminal)
                    {
                        terminal_journal.acknowledge(
                            &terminal.record.invoke_id,
                            &terminal.record.occurred_at,
                            terminal.raw_capture,
                            &terminal.attempts,
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
                batch.recovery_replay = recovery_replay;
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
        match flush_pending_batch_inner_with_authority(
            pool,
            traffic_authority,
            system_task_batch,
            pricing_catalog,
            prompt_cache_conversation_cache,
            terminal_runtime_store,
            dashboard_activity_snapshot_cache,
            summary_delta_hub,
            terminal_projection_hub,
            dashboard_reconcile_gate,
            accounting.observability.get(),
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
                system_task_lock_failure = is_batch_writer_sqlite_lock_error(&err);
                system_task_failure = Some(err);
            }
        }
    }
    // Retry lock/pressure failures, but quarantine deterministic SQL failures. Every finish is
    // journaled before enqueue, so quarantine remains durable without repeatedly stressing SQLite.
    let system_task_retryable_failure = system_task_failure
        .as_ref()
        .is_some_and(crate::db_pressure::is_db_pressure_error);

    let pending_terminal_journal_acks = has_pending_terminal_journal_acks(&batch);
    match flush_pending_batch_inner_with_authority(
        pool,
        traffic_authority,
        &batch,
        pricing_catalog,
        prompt_cache_conversation_cache,
        terminal_runtime_store,
        dashboard_activity_snapshot_cache,
        summary_delta_hub,
        terminal_projection_hub,
        dashboard_reconcile_gate,
        accounting.observability.get(),
    )
    .await
    {
        Ok(main_deferred) => {
            deferred_batch.merge_p2(main_deferred);
            if pending_terminal_journal_acks {
                // PostgreSQL facts and SQLite-owned observations have both committed. The
                // source journal is now safe to acknowledge; an acknowledgement failure leaves
                // the same terminal available for idempotent replay.
                acknowledge_terminal_journal_acks(terminal_journal, &batch);
            }
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
            if pending_terminal_journal_acks {
                // Never discard a terminal whose journal acknowledgement is still pending. If
                // the PG fact transaction committed before a later side effect failed, retrying
                // the full batch is idempotent and the journal remains the durable fallback after
                // a process restart.
                let mut retry_batch = if system_task_failure.is_some() {
                    system_task_batch.unwrap_or_default()
                } else {
                    PendingBatch::default()
                };
                retry_batch.merge_p2(deferred_batch);
                retry_batch.merge_p2(batch);
                return Some(RetainedBatch::p2_failed(
                    retry_batch,
                    true,
                    system_task_lock_failure || is_batch_writer_sqlite_lock_error(&err),
                ));
            }
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
                                is_batch_writer_sqlite_lock_error(&err),
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
                        system_task_lock_failure || is_batch_writer_sqlite_lock_error(&err),
                    ));
                }
                if !crate::db_pressure::is_db_pressure_error(&err) {
                    cleanup_discarded_p2_runtime_overlays(&batch, terminal_runtime_store);
                    return None;
                }
                return Some(RetainedBatch::p2_failed(
                    batch,
                    true,
                    is_batch_writer_sqlite_lock_error(&err),
                ));
            }
            let retryable_failure = crate::db_pressure::is_db_pressure_error(&err);
            return Some(RetainedBatch::p2_failed(
                batch,
                retryable_failure,
                is_batch_writer_sqlite_lock_error(&err),
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
        if !deferred_batch.attempt_progress.is_empty() {
            accounting.attempt_progress_deferred(deferred_batch.attempt_progress.len());
            accounting.observe_attempt_progress_age(deferred_batch.age().as_millis() as u64);
        }
        Some(RetainedBatch::new(deferred_batch, false))
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

#[expect(
    clippy::too_many_arguments,
    reason = "Flush dependencies mirror the single-writer ownership boundaries."
)]
pub(crate) async fn flush_pending_batch_inner(
    pool: &Pool<Sqlite>,
    batch: &PendingBatch,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache: &Arc<
        std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>,
    >,
    summary_delta_hub: &Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    terminal_projection_hub: &Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: &Arc<Mutex<()>>,
    observability: Option<&ObservabilityRuntime>,
) -> Result<PendingBatch> {
    let traffic_authority = crate::storage::traffic::TrafficFactStore::sqlite(pool.clone());
    flush_pending_batch_inner_with_authority(
        pool,
        &traffic_authority,
        batch,
        pricing_catalog,
        prompt_cache_conversation_cache,
        terminal_runtime_store,
        dashboard_activity_snapshot_cache,
        summary_delta_hub,
        terminal_projection_hub,
        dashboard_reconcile_gate,
        observability,
    )
    .await
}

async fn flush_sqlite_owned_p2_side_effects(
    pool: &Pool<Sqlite>,
    traffic_authority: &crate::storage::traffic::TrafficFactStore,
    batch: &PendingBatch,
    write_legacy_task_runs: bool,
) -> Result<()> {
    let _authority_lease = traffic_authority.acquire_write_authority().await?;
    // PostgreSQL facts commit before these SQLite-owned observations. Drain the durable outbox
    // first so a process crash between those commits leaves a replayable obligation.
    replay_account_selected_touch_outbox_under_authority(pool).await?;
    let has_derived_account_activity = batch
        .invocation_derived
        .values()
        .any(|derived| derived.upstream_account_id.is_some());
    if !has_derived_account_activity
        && (!write_legacy_task_runs || batch.system_task_finishes.is_empty())
    {
        return Ok(());
    }
    let mut transaction = pool.begin().await?;
    for derived in batch.invocation_derived.values() {
        crate::touch_upstream_account_last_activity_tx(
            &mut transaction,
            &derived.occurred_at,
            derived.upstream_account_id,
        )
        .await?;
    }
    if write_legacy_task_runs {
        for finish in batch.system_task_finishes.values() {
            sqlx::query(
                r#"
                UPDATE system_task_runs
                SET status = ?1,
                    summary = COALESCE(?2, summary),
                    detail = ?3,
                    finished_at = ?4,
                    duration_ms = ?5
                WHERE id = ?6
                "#,
            )
            .bind(finish.status.as_str())
            .bind(finish.summary.as_deref())
            .bind(finish.detail.as_deref())
            .bind(&finish.finished_at)
            .bind(finish.duration_ms)
            .bind(finish.run_id)
            .execute(&mut *transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    Ok(())
}

async fn configure_sqlite_capture_transaction(
    transaction: &mut crate::storage::traffic::TrafficFactTransaction,
    recovery_replay: bool,
) -> Result<()> {
    // SQLite's native capture triggers need recursive trigger execution to observe the implicit
    // DELETE emitted by INSERT OR REPLACE. Configure it on every writer transaction so capture
    // remains in the same transaction and generation as the business fact.
    sqlx::query("PRAGMA recursive_triggers = ON")
        .execute(transaction.sqlite_connection()?)
        .await
        .context("enable recursive SQLite capture triggers")
        .map(|_| ())?;
    if recovery_replay {
        sqlx::query(&format!(
            "PRAGMA busy_timeout = {}",
            crate::runtime::STARTUP_RECOVERY_BUSY_TIMEOUT.as_millis()
        ))
        .execute(transaction.sqlite_connection()?)
        .await
        .context("configure startup recovery SQLite busy timeout")
        .map(|_| ())?;
    }
    Ok(())
}

/// Persist account-selection observations before the PostgreSQL fact transaction commits.
///
/// SQLite and PostgreSQL cannot share a transaction here. The outbox is therefore committed
/// first, and its replay is monotonic and idempotent: a crash before or after the fact commit
/// can only leave a pending row that the next startup or flush will apply and acknowledge.
pub(crate) async fn persist_account_selected_touch_outbox(
    pool: &Pool<Sqlite>,
    batch: &PendingBatch,
) -> Result<()> {
    if batch.account_selected_touches.is_empty() {
        return Ok(());
    }

    let mut transaction = pool.begin().await?;
    for touch in batch.account_selected_touches.values() {
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_selected_touch_outbox (account_id, selected_at)
            VALUES (?1, ?2)
            ON CONFLICT (account_id, selected_at) DO NOTHING
            "#,
        )
        .bind(touch.account_id)
        .bind(&touch.selected_at)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// Replay and acknowledge all durable account-selection observations.
///
/// The account update and outbox deletion share one SQLite transaction. If the process dies
/// after the update but before commit, the row remains; if it dies after commit, both are done.
/// Replaying a duplicate row is safe because `last_selected_at` only moves forward.
pub(crate) async fn replay_account_selected_touch_outbox(
    pool: &Pool<Sqlite>,
    traffic_authority: &crate::storage::traffic::TrafficFactStore,
) -> Result<usize> {
    let _authority_lease = traffic_authority.acquire_write_authority().await?;
    replay_account_selected_touch_outbox_under_authority(pool).await
}

async fn replay_account_selected_touch_outbox_under_authority(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let rows = sqlx::query_as::<_, (i64, i64, String)>(
        r#"
        SELECT id, account_id, selected_at
        FROM pool_upstream_account_selected_touch_outbox
        ORDER BY id
        "#,
    )
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let mut transaction = pool.begin().await?;
    for (outbox_id, account_id, selected_at) in &rows {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_selected_at = CASE
                    WHEN last_selected_at IS NULL OR last_selected_at < ?2 THEN ?2
                    ELSE last_selected_at
                END,
                updated_at = CASE
                    WHEN updated_at IS NULL OR updated_at < ?2 THEN ?2
                    ELSE updated_at
                END
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(selected_at)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("DELETE FROM pool_upstream_account_selected_touch_outbox WHERE id = ?1")
            .bind(outbox_id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(rows.len())
}

#[expect(
    clippy::too_many_arguments,
    reason = "Flush dependencies mirror the single-writer ownership boundaries."
)]
async fn flush_pending_batch_inner_with_authority(
    pool: &Pool<Sqlite>,
    traffic_authority: &crate::storage::traffic::TrafficFactStore,
    batch: &PendingBatch,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
    terminal_runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache: &Arc<
        std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>,
    >,
    summary_delta_hub: &Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    terminal_projection_hub: &Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: &Arc<Mutex<()>>,
    observability: Option<&ObservabilityRuntime>,
) -> Result<PendingBatch> {
    crate::observability::observed_future(
        observability.is_some_and(|m| m.enabled),
        "sqlite_batch_flush",
        async move {
    let mut deferred_batch = PendingBatch::default();
    let mut prompt_cache_keys_touched = HashSet::new();
    let mut active_prompt_cache_key_releases = Vec::new();
    let recovery_replay = batch.recovery_replay;
    let mut terminal_backend_for_derived = crate::storage::core::BackendKind::Sqlite;
    let _dashboard_reconcile_guard = if recovery_replay {
        timeout(
            crate::runtime::STARTUP_RECOVERY_ADMISSION_BUDGET,
            dashboard_reconcile_gate.lock(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "startup recovery dashboard reconcile admission exceeded {}ms",
                crate::runtime::STARTUP_RECOVERY_ADMISSION_BUDGET.as_millis()
            )
        })?
    } else {
        dashboard_reconcile_gate.lock().await
    };
    let mut persisted_terminals = Vec::with_capacity(batch.terminal_invocations.len());
    if !batch.terminal_invocations.is_empty() {
        let mut diagnostic_batch = diagnostics::begin_batch(batch);
        let mut execute_started = Instant::now();
        let execute_result: Result<()> = async {
        let (mut terminal_tx, pool_acquisition_duration) = if recovery_replay {
            timeout(
                crate::runtime::STARTUP_RECOVERY_ADMISSION_BUDGET,
                traffic_authority.begin_with_pool_acquisition_duration(),
            )
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "startup recovery SQLite write admission exceeded {}ms",
                    crate::runtime::STARTUP_RECOVERY_ADMISSION_BUDGET.as_millis()
                )
            })??
        } else {
            traffic_authority
                .begin_with_pool_acquisition_duration()
                .await?
        };
        let terminal_backend = terminal_tx.backend_kind();
        terminal_backend_for_derived = terminal_backend;
        diagnostic_batch.pool_acquired();
        if let Some(m) = observability {
            m.duration(
                "cvm_sqlite_pool_acquire_duration_seconds",
                &[],
                pool_acquisition_duration,
            );
        }
        execute_started = Instant::now();
        let recovery_setup_result: Result<()> = if terminal_backend
            == crate::storage::core::BackendKind::Sqlite
        {
            configure_sqlite_capture_transaction(&mut terminal_tx, recovery_replay).await
        } else {
            Ok(())
        };
        let fact_result: Result<()> = if let Err(error) = recovery_setup_result {
            Err(error)
        } else {
            async {
        for terminal in batch.terminal_invocations.values() {
            let persisted = match terminal_backend {
                crate::storage::core::BackendKind::Sqlite => {
                    if terminal.raw_capture {
                        let capture_started = terminal.capture_started.unwrap_or_else(Instant::now);
                        persist_proxy_capture_record_tx(
                            terminal_tx.sqlite_connection()?,
                            capture_started,
                            terminal.record.clone(),
                            false,
                        )
                        .await
                        .with_context(|| "flush terminal raw proxy invocation")?
                    } else {
                        persist_proxy_capture_runtime_record_tx(
                            terminal_tx.sqlite_connection()?,
                            terminal.record.clone(),
                            false,
                        )
                        .await
                        .with_context(|| "flush terminal runtime proxy invocation")?
                    }
                }
                crate::storage::core::BackendKind::Postgres => {
                    if terminal.raw_capture {
                        persist_proxy_capture_record_postgres_tx(
                            terminal_tx.postgres_connection()?,
                            terminal.capture_started.unwrap_or_else(Instant::now),
                            terminal.record.clone(),
                        )
                        .await
                        .with_context(|| "flush terminal raw proxy invocation in PostgreSQL")?
                    } else {
                        persist_proxy_capture_runtime_record_postgres_tx(
                            terminal_tx.postgres_connection()?,
                            terminal.record.clone(),
                        )
                        .await
                        .with_context(|| "flush terminal runtime proxy invocation in PostgreSQL")?
                    }
                }
            };
            let derived_identity = if let Some(persisted) = persisted.as_ref() {
                Some((persisted.id, persisted.occurred_at.clone()))
            } else {
                let identity = match terminal_backend {
                    crate::storage::core::BackendKind::Sqlite => {
                        load_persisted_invocation_identity_tx(
                            terminal_tx.sqlite_connection()?,
                            &terminal.record.invoke_id,
                            &terminal.record.occurred_at,
                        )
                        .await?
                    }
                    crate::storage::core::BackendKind::Postgres => {
                        load_persisted_invocation_identity_postgres_tx(
                            terminal_tx.postgres_connection()?,
                            &terminal.record.invoke_id,
                            &terminal.record.occurred_at,
                        )
                        .await?
                    }
                };
                identity.map(|row| (row.id, terminal.record.occurred_at.clone()))
            };
            let (invocation_id, occurred_at) = derived_identity.ok_or_else(|| {
            anyhow!(
                "terminal write completed without a persisted identity: invoke_id={} occurred_at={}",
                terminal.record.invoke_id,
                terminal.record.occurred_at
            )
        })?;
            let fact_identity = crate::storage::traffic::TrafficFactIdentity::new(
                invocation_id,
                terminal.record.invoke_id.clone(),
                occurred_at.clone(),
            )?;
            for attempt in &terminal.attempts {
                let attempt_result = match terminal_backend {
                    crate::storage::core::BackendKind::Sqlite => {
                        persist_runtime_attempt_snapshot_for_invocation_tx(
                            terminal_tx.sqlite_connection()?,
                            &fact_identity,
                            attempt,
                        )
                        .await
                    }
                    crate::storage::core::BackendKind::Postgres => {
                        persist_runtime_attempt_snapshot_for_invocation_postgres_tx(
                            terminal_tx.postgres_connection()?,
                            &fact_identity,
                            attempt,
                        )
                        .await
                    }
                };
                attempt_result
                    .with_context(|| format!("flush terminal proxy attempt {}", attempt.attempt_public_id))?;
            }
            let payload_metadata =
                persisted
                    .as_ref()
                    .map(|record| crate::TerminalPayloadMetadata {
                        prompt_cache_key: record.prompt_cache_key.clone(),
                        upstream_account_id: record.upstream_account_id,
                        request_model: None,
                    });
            let summary_delta = terminal.dashboard_terminal_sequence.and_then(|sequence| {
                persisted.as_ref().map(|record| {
                    let mut delta = crate::persisted_dashboard_activity_terminal_delta(record);
                    delta.persisted_row_id = Some(invocation_id);
                    // A recovered terminal has no valid process-local sequence. Its committed
                    // row is still exact, so preserve it as an independent replay overlay.
                    let replayed_after_restart = sequence == 0;
                    if !replayed_after_restart {
                        delta.terminal_sequence = sequence;
                    }
                    (delta, replayed_after_restart)
                })
            });
            persisted_terminals.push((
                terminal,
                invocation_id,
                occurred_at,
                payload_metadata,
                summary_delta,
            ));
        }
        // Persist the compact recovery obligation atomically with its source rows.
        // Summary computation remains in the independently admitted derived consumer.
        if !persisted_terminals.is_empty() {
            let source_revision = persisted_terminals
                .iter()
                .filter_map(|(terminal, ..)| terminal.dashboard_terminal_sequence)
                .max()
                .unwrap_or_default();
            let entries = persisted_terminals
                .iter()
                .map(|(terminal, invocation_id, occurred_at, metadata, _)| {
                    SummarySourceChangeEntry {
                        row_id: *invocation_id,
                        invoke_id: terminal.record.invoke_id.clone(),
                        occurred_at: occurred_at.clone(),
                        upstream_account_id: metadata
                            .as_ref()
                            .and_then(|metadata| metadata.upstream_account_id),
                        current_rank: None,
                    }
                })
                .collect();
            let descriptor = SummarySourceChangeDescriptor::terminal_batch(source_revision, entries)?;
            match terminal_backend {
                crate::storage::core::BackendKind::Sqlite => {
                    append_summary_source_change_descriptor_tx(
                        terminal_tx.sqlite_connection()?,
                        &descriptor,
                    )
                    .await?;
                }
                crate::storage::core::BackendKind::Postgres => {
                    append_summary_source_change_descriptor_postgres_tx(
                        terminal_tx.postgres_connection()?,
                        &descriptor,
                    )
                    .await?;
                }
            }
        }
        Ok(())
        }
        .await
        };
        if recovery_replay {
            let restore_result: Result<()> = if terminal_backend
                == crate::storage::core::BackendKind::Sqlite
            {
                sqlx::query("PRAGMA busy_timeout = 30000")
                    .execute(terminal_tx.sqlite_connection()?)
                    .await
                    .context("restore SQLite busy timeout after startup recovery")
                    .map(|_| ())
            } else {
                Ok(())
            };
            match (fact_result, restore_result) {
                (Ok(()), Ok(())) => {}
                (Err(error), Ok(())) => return Err(error),
                (Ok(()), Err(error)) => return Err(error),
                (Err(error), Err(restore_error)) => {
                    return Err(anyhow!(
                        "startup recovery fact write failed: {error:#}; busy timeout restore failed: {restore_error:#}"
                    ));
                }
            }
        } else {
            fact_result?;
        }
        terminal_tx.commit().await?;
        Ok(())
        }.await;
        if let Some(m) = observability {
            let outcome = if execute_result.is_ok() {
                "success"
            } else {
                "error"
            };
            m.duration(
                "cvm_sqlite_batch_execute_duration_seconds",
                &[("class", "p1_terminal"), ("outcome", outcome)],
                execute_started.elapsed(),
            );
            m.counter(
                "cvm_sqlite_batches_total",
                &[("class", "p1_terminal"), ("outcome", outcome)],
                1,
            );
        }
        execute_result?;
        finish_committed_batch(batch, diagnostic_batch);
        if let Some(m) = observability {
            m.counter(
                "cvm_sqlite_written_rows_total",
                &[],
                batch.terminal_invocations.len() as u64,
            );
            m.counter(
                "cvm_sqlite_written_bytes_total",
                &[],
                batch.terminal_estimated_bytes as u64,
            );
            for terminal in batch.terminal_invocations.values() {
                if let Some(enqueued) = terminal.enqueued_at {
                    m.duration(
                        "cvm_terminal_enqueue_to_commit_duration_seconds",
                        &[],
                        enqueued.elapsed(),
                    );
                }
            }
        }
    }

    // The batch's coalescing key is invocation identity, while Summary's correctness proof is
    // the terminal sequence allocated at registration time. Commit is atomic, so publish its
    // ACKs in that sequence order rather than the map's identity order.
    persisted_terminals.sort_by_key(|(terminal, _, _, _, _)| {
        terminal.dashboard_terminal_sequence.unwrap_or(u64::MAX)
    });

    for (terminal, invocation_id, occurred_at, payload_metadata, summary_delta) in
        persisted_terminals
    {
        deferred_batch.add_startup_backfill_wake_tasks(&terminal.startup_backfill_tasks);
        let dashboard_cache = dashboard_activity_snapshot_cache
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let Some(cache) = dashboard_cache {
            acknowledge_dashboard_activity_terminal_record(
                &cache,
                &terminal.record.invoke_id,
                &terminal.record.occurred_at,
                invocation_id,
                terminal.dashboard_terminal_sequence,
            )
            .await;
        }
        let summary_delta_hub = summary_delta_hub
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let (Some((summary_delta, replayed_after_restart)), Some(hub)) =
            (summary_delta, summary_delta_hub)
        {
            if replayed_after_restart {
                hub.acknowledge_replayed_summary_delta(summary_delta).await;
            } else {
                hub.acknowledge_summary_delta(summary_delta).await;
            }
        }
        if let Some(hub) = terminal_projection_hub
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
        {
            if terminal.terminal_projection_event_ids.is_empty() {
                hub.acknowledge_persisted(
                    None,
                    &terminal.record.invoke_id,
                    &terminal.record.occurred_at,
                    invocation_id,
                );
            } else {
                for event_id in &terminal.terminal_projection_event_ids {
                    hub.acknowledge_persisted(
                        Some(*event_id),
                        &terminal.record.invoke_id,
                        &terminal.record.occurred_at,
                        invocation_id,
                    );
                }
            }
        }
        // The persistence helper already materialized this payload into the returned invocation.
        // Reuse those fields instead of parsing the same terminal payload again on the P2 path.
        let payload_metadata = payload_metadata.unwrap_or_else(|| {
            crate::terminal_payload_metadata(terminal.record.payload.as_deref())
        });
        // Recovered PostgreSQL journal records intentionally omit payload text. Attempts carry
        // the authoritative account identity, so use them to rebuild the SQLite observation
        // obligation after a restart.
        let upstream_account_id = payload_metadata.upstream_account_id.or_else(|| {
            terminal
                .attempts
                .iter()
                .rev()
                .find_map(|attempt| (attempt.upstream_account_id > 0).then_some(attempt.upstream_account_id))
        });
        if let Some(prompt_cache_key) = payload_metadata
            .prompt_cache_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            prompt_cache_keys_touched.insert(prompt_cache_key.to_string());
            active_prompt_cache_key_releases.push(prompt_cache_key.to_string());
        }
        deferred_batch.push(SqliteBatchWrite::InvocationDerived(
            BatchedInvocationDerivedWrites {
                invocation_id,
                occurred_at,
                upstream_account_id,
                terminal_overlay_key: Some((
                    terminal.record.invoke_id.clone(),
                    terminal.record.occurred_at.clone(),
                )),
                terminal_journal_ack: (terminal_backend_for_derived
                    == crate::storage::core::BackendKind::Postgres)
                .then(|| terminal.clone()),
            },
        ));
    }

    // The terminal transaction is durable at this point. Release its lease before any
    // rebuildable P2 work can fail, so a post-commit error cannot strand the conversation.
    if let Some(cache) = prompt_cache_conversation_cache {
        if !prompt_cache_keys_touched.is_empty() {
            invalidate_prompt_cache_conversations_cache(cache).await;
        }
        release_active_prompt_cache_conversations(cache, &active_prompt_cache_key_releases).await;
    }

    if !batch.startup_backfill_wake_tasks.is_empty() {
        let pricing_catalog_guard = if let Some(pricing_catalog) = pricing_catalog {
            Some(pricing_catalog.read().await)
        } else {
            None
        };
        let wake_tasks = &batch.startup_backfill_wake_tasks;
        if let Err(error) = notify_startup_backfill_tasks(
            pool,
            wake_tasks,
            pricing_catalog_guard.as_deref(),
            "terminal_payload_repair_input",
        )
        {
            debug!(
                error = %error,
                tasks = wake_tasks.len(),
                "startup backfill notification rejected after terminal commit; retaining wake obligation"
            );
            deferred_batch.add_startup_backfill_wake_tasks(wake_tasks);
        }
    }

    if batch.attempt_progress.is_empty()
        && batch.invocation_derived.is_empty()
        && batch.account_selected_touches.is_empty()
        && batch.system_task_finishes.is_empty()
    {
        return Ok(deferred_batch);
    }

    let maintenance_store = crate::maintenance_store::global().cloned();
    let write_legacy_task_runs = {
        #[cfg(test)]
        {
            maintenance_store.is_none()
        }
        #[cfg(not(test))]
        {
            false
        }
    };
    let (mut traffic_tx, pool_acquisition_duration) = if recovery_replay {
        timeout(
            crate::runtime::STARTUP_RECOVERY_ADMISSION_BUDGET,
            traffic_authority.begin_with_pool_acquisition_duration(),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "startup recovery SQLite write admission exceeded {}ms",
                crate::runtime::STARTUP_RECOVERY_ADMISSION_BUDGET.as_millis()
            )
        })??
    } else {
        traffic_authority
            .begin_with_pool_acquisition_duration()
            .await?
    };
    let traffic_backend = traffic_tx.backend_kind();
    if let Some(m) = observability {
        m.duration(
            "cvm_sqlite_pool_acquire_duration_seconds",
            &[],
            pool_acquisition_duration,
        );
    }
    if traffic_backend == crate::storage::core::BackendKind::Postgres
        && !batch.account_selected_touches.is_empty()
    {
        // Commit the SQLite outbox before the PG fact. A crash after the PG commit can then
        // replay the observation instead of losing last_selected_at.
        persist_account_selected_touch_outbox(pool, batch).await?;
    }
    let execute_started = Instant::now();
    let mut terminal_overlay_keys = Vec::new();
    let execute_result: Result<()> = async {
    let recovery_setup_result: Result<()> = if traffic_backend
        == crate::storage::core::BackendKind::Sqlite
    {
        configure_sqlite_capture_transaction(&mut traffic_tx, recovery_replay).await
    } else {
        Ok(())
    };
    let fact_result: Result<()> = if let Err(error) = recovery_setup_result {
        Err(error)
    } else {
        async {
    if traffic_backend == crate::storage::core::BackendKind::Postgres {
        // Analytics consumes the durable PostgreSQL source-change journal. The PG fact write is
        // therefore the source obligation; requeueing InvocationDerived here would make FlushNow
        // permanently non-convergent because this SQLite rollup path cannot consume PG facts.
        for progress in batch.attempt_progress.values() {
            attempt_progress::persist_postgres(traffic_tx.postgres_transaction()?, progress).await?;
        }
    } else {
    for progress in batch.attempt_progress.values() {
        attempt_progress::persist(traffic_tx.sqlite_transaction()?, progress).await?;
    }

    if !batch.invocation_derived.is_empty() {
        let target_invocation_id = batch
            .invocation_derived
            .keys()
            .next_back()
            .copied()
            .unwrap_or_default();
        let live_rollup_cursor_before = load_hourly_rollup_live_progress_tx(
            traffic_tx.sqlite_transaction()?.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
        )
        .await?;
        replay_live_invocation_hourly_rollups_until_tx(
            traffic_tx.sqlite_transaction()?.as_mut(),
            target_invocation_id,
        )
        .await?;
        let live_rollup_cursor_after =
            load_hourly_rollup_live_progress_tx(
                traffic_tx.sqlite_transaction()?.as_mut(),
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
            )
            .await?;
        let skipped_terminal_ids = batch
            .invocation_derived
            .keys()
            .filter(|invocation_id| **invocation_id <= live_rollup_cursor_before)
            .copied()
            .collect::<Vec<_>>();
        if !skipped_terminal_ids.is_empty() {
            recompute_invocation_hourly_rollups_for_ids_tx(
                traffic_tx.sqlite_transaction()?.as_mut(),
                &skipped_terminal_ids,
            )
            .await?;
        }
        for derived in batch.invocation_derived.values() {
            if derived.invocation_id > live_rollup_cursor_after {
                deferred_batch.push(SqliteBatchWrite::InvocationDerived(derived.clone()));
                continue;
            }
            if let Some(key) = derived.terminal_overlay_key.clone() {
                terminal_overlay_keys.push(key);
            }
            crate::touch_upstream_account_last_activity_tx(
                traffic_tx.sqlite_transaction()?.as_mut(),
                &derived.occurred_at,
                derived.upstream_account_id,
            )
            .await?;
        }
    }

    for touch in batch.account_selected_touches.values() {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_selected_at = CASE
                    WHEN last_selected_at IS NULL OR last_selected_at < ?2 THEN ?2
                    ELSE last_selected_at
                END,
                updated_at = CASE
                    WHEN updated_at IS NULL OR updated_at < ?2 THEN ?2
                    ELSE updated_at
                END
            WHERE id = ?1
            "#,
        )
        .bind(touch.account_id)
        .bind(&touch.selected_at)
        .execute(traffic_tx.sqlite_connection()?)
        .await?;
    }

    if write_legacy_task_runs {
        for finish in batch.system_task_finishes.values() {
            sqlx::query(
                r#"
                UPDATE system_task_runs
                SET status = ?1,
                    summary = COALESCE(?2, summary),
                    detail = ?3,
                    finished_at = ?4,
                    duration_ms = ?5
                WHERE id = ?6
                "#,
            )
            .bind(finish.status.as_str())
            .bind(finish.summary.as_deref())
            .bind(finish.detail.as_deref())
            .bind(&finish.finished_at)
            .bind(finish.duration_ms)
            .bind(finish.run_id)
            .execute(traffic_tx.sqlite_connection()?)
            .await?;
        }
    }
    }
    Ok(())

        }
        .await
    };
    if recovery_replay {
        let restore_result: Result<()> = if traffic_backend
            == crate::storage::core::BackendKind::Sqlite
        {
            sqlx::query("PRAGMA busy_timeout = 30000")
                .execute(traffic_tx.sqlite_connection()?)
                .await
                .context("restore SQLite busy timeout after startup recovery")
                .map(|_| ())
        } else {
            Ok(())
        };
        match (fact_result, restore_result) {
            (Ok(()), Ok(())) => {}
            (Err(error), Ok(())) => return Err(error),
            (Ok(()), Err(error)) => return Err(error),
            (Err(error), Err(restore_error)) => {
                return Err(anyhow!(
                    "startup recovery derived write failed: {error:#}; busy timeout restore failed: {restore_error:#}"
                ));
            }
        }
    } else {
        fact_result?;
    }
    traffic_tx.commit().await?;
    if traffic_backend == crate::storage::core::BackendKind::Postgres {
        flush_sqlite_owned_p2_side_effects(
            pool,
            traffic_authority,
            batch,
            write_legacy_task_runs,
        )
        .await?;
        terminal_overlay_keys.extend(
            batch
                .invocation_derived
                .values()
                .filter_map(|derived| derived.terminal_overlay_key.clone()),
        );
    }
    Ok(())
    }
    .await;
    if let Some(m) = observability {
        let outcome = if execute_result.is_ok() {
            "success"
        } else {
            "error"
        };
        m.duration(
            "cvm_sqlite_batch_execute_duration_seconds",
            &[("class", "p2_derived"), ("outcome", outcome)],
            execute_started.elapsed(),
        );
        m.counter(
            "cvm_sqlite_batches_total",
            &[("class", "p2_derived"), ("outcome", outcome)],
            1,
        );
    }
    execute_result?;
    if let Some(m) = observability {
        m.counter(
            "cvm_sqlite_written_rows_total",
            &[],
            batch.logical_rows() as u64,
        );
        m.counter(
            "cvm_sqlite_written_bytes_total",
            &[],
            batch.estimated_memory_bytes() as u64,
        );
    }

    if let Some(store) = maintenance_store {
        for finish in batch.system_task_finishes.values() {
            let result = if let Some(details) = finish.details.as_ref()
                && let (Some(seconds), Some(reason)) = (
                    details
                        .get("scheduleRetryAfterSecs")
                        .and_then(serde_json::Value::as_i64),
                    details
                        .get("scheduleRetryReason")
                        .and_then(serde_json::Value::as_str),
                ) {
                store
                    .finish_run_with_observation_and_retry(
                        finish.run_id,
                        finish.status.as_str(),
                        &finish.finished_at,
                        finish.duration_ms,
                        finish.summary.as_deref(),
                        finish.detail.as_deref(),
                        finish.completion.as_deref(),
                        finish.core_completion.as_deref(),
                        finish.details.as_ref(),
                        seconds,
                        reason,
                    )
                    .await
            } else {
                store
                    .finish_run_with_observation(
                        finish.run_id,
                        finish.status.as_str(),
                        &finish.finished_at,
                        finish.duration_ms,
                        finish.summary.as_deref(),
                        finish.detail.as_deref(),
                        finish.completion.as_deref(),
                        finish.core_completion.as_deref(),
                        finish.details.as_ref(),
                    )
                    .await
            };
            if let Err(error) = result {
                warn!(
                    run_id = finish.run_id,
                    error = %error,
                    "failed to finalize task history in maintenance database"
                );
                return Err(error);
            }
        }
    }

    if !terminal_overlay_keys.is_empty()
        && let Some(runtime_store) = terminal_runtime_store
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
    {
        for (invoke_id, occurred_at) in terminal_overlay_keys {
            let removed = runtime_store.remove_persisted_terminal_overlay(&invoke_id, &occurred_at);
            debug!(
                invoke_id = %invoke_id,
                occurred_at = %occurred_at,
                terminal_runtime_overlay_removed_after_derived_flush = removed,
                "removed persisted terminal record from memory runtime overlay after derived writes"
            );
        }
    }

    if !prompt_cache_keys_touched.is_empty()
        && let Some(cache) = prompt_cache_conversation_cache
    {
        invalidate_prompt_cache_conversations_cache(cache).await;
    }
    Ok(deferred_batch)
        },
    ).await
}

pub(crate) async fn replay_live_invocation_hourly_rollups_until_tx(
    tx: &mut SqliteConnection,
    target_invocation_id: i64,
) -> Result<u64> {
    let cursor = load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    if cursor >= target_invocation_id {
        return Ok(0);
    }
    replay_live_invocation_hourly_rollups_tx(tx).await
}
