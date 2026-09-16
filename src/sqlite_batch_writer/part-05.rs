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
) -> Result<PendingBatch> {
    let mut deferred_batch = PendingBatch::default();
    let mut should_invalidate_prompt_cache_conversations = false;
    let _dashboard_reconcile_guard = dashboard_reconcile_gate.lock().await;
    let mut persisted_terminals = Vec::with_capacity(batch.terminal_invocations.len());
    if !batch.terminal_invocations.is_empty() {
        let mut terminal_tx = pool.begin().await?;
        for terminal in batch.terminal_invocations.values() {
            let persisted = if terminal.raw_capture {
                let capture_started = terminal.capture_started.unwrap_or_else(Instant::now);
                persist_proxy_capture_record_tx(
                    terminal_tx.as_mut(),
                    capture_started,
                    terminal.record.clone(),
                    false,
                )
                .await
                .with_context(|| "flush terminal raw proxy invocation")?
            } else {
                persist_proxy_capture_runtime_record_tx(
                    terminal_tx.as_mut(),
                    terminal.record.clone(),
                    false,
                )
                .await
                .with_context(|| "flush terminal runtime proxy invocation")?
            };
            let derived_identity = if let Some(persisted) = persisted.as_ref() {
                if persisted
                    .prompt_cache_key
                    .as_deref()
                    .is_some_and(|key| !key.trim().is_empty())
                {
                    should_invalidate_prompt_cache_conversations = true;
                }
                Some((persisted.id, persisted.occurred_at.clone()))
            } else {
                let identity = load_persisted_invocation_identity_tx(
                    terminal_tx.as_mut(),
                    &terminal.record.invoke_id,
                    &terminal.record.occurred_at,
                )
                .await?;
                identity.map(|row| (row.id, terminal.record.occurred_at.clone()))
            };
            let (invocation_id, occurred_at) = derived_identity.ok_or_else(|| {
            anyhow!(
                "terminal write completed without a persisted identity: invoke_id={} occurred_at={}",
                terminal.record.invoke_id,
                terminal.record.occurred_at
            )
        })?;
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
        // Persist one compact Summary source descriptor in the same transaction as the terminal
        // rows.  It is intentionally identity-only: rolling recovery reconstructs the bounded
        // preview from the source table after a restart instead of duplicating raw payloads.
        let descriptor_entries = persisted_terminals
            .iter()
            .map(
                |(terminal, invocation_id, occurred_at, payload_metadata, _)| {
                    SummarySourceChangeEntry {
                        row_id: *invocation_id,
                        invoke_id: terminal.record.invoke_id.clone(),
                        occurred_at: occurred_at.clone(),
                        upstream_account_id: payload_metadata
                            .as_ref()
                            .and_then(|metadata| metadata.upstream_account_id),
                        current_rank: None,
                    }
                },
            )
            .collect::<Vec<_>>();
        if !descriptor_entries.is_empty() {
            let source_revision = persisted_terminals
                .iter()
                .filter_map(|(terminal, ..)| terminal.dashboard_terminal_sequence)
                .max()
                .unwrap_or_default();
            let descriptor =
                SummarySourceChangeDescriptor::terminal_batch(source_revision, descriptor_entries)?;
            append_summary_source_change_descriptor_tx(terminal_tx.as_mut(), &descriptor).await?;
        }
        terminal_tx.commit().await?;
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
        if payload_metadata
            .prompt_cache_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty())
        {
            should_invalidate_prompt_cache_conversations = true;
        }
        deferred_batch.push(SqliteBatchWrite::InvocationDerived(
            BatchedInvocationDerivedWrites {
                invocation_id,
                occurred_at,
                upstream_account_id: payload_metadata.upstream_account_id,
                terminal_overlay_key: Some((
                    terminal.record.invoke_id.clone(),
                    terminal.record.occurred_at.clone(),
                )),
            },
        ));
    }

    if !batch.startup_backfill_wake_tasks.is_empty() {
        let pricing_catalog = if let Some(pricing_catalog) = pricing_catalog {
            Some(pricing_catalog.read().await.clone())
        } else {
            None
        };
        let wake_tasks = batch
            .startup_backfill_wake_tasks
            .iter()
            .copied()
            .filter(|task| {
                let available = pricing_catalog.is_some()
                    || !matches!(task, StartupBackfillTask::ProxyCost);
                if !available {
                    warn!(
                        task = task.name(),
                        "skipping startup backfill wake because its runtime pricing catalog is unavailable"
                    );
                }
                available
            })
            .collect::<Vec<_>>();
        wake_startup_backfill_tasks_with_pricing_catalog(
            pool,
            &wake_tasks,
            pricing_catalog.as_ref(),
            "terminal_payload_repair_input",
        )
        .await?;
    }

    if batch.attempt_progress.is_empty()
        && batch.invocation_derived.is_empty()
        && batch.account_selected_touches.is_empty()
        && batch.system_task_finishes.is_empty()
    {
        if should_invalidate_prompt_cache_conversations
            && let Some(cache) = prompt_cache_conversation_cache
        {
            invalidate_prompt_cache_conversations_cache(cache).await;
        }
        return Ok(deferred_batch);
    }

    let mut tx = pool.begin().await?;

    for progress in batch.attempt_progress.values() {
        sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                phase = ?2,
                connect_latency_ms = CASE
                    WHEN ?4 IS NULL THEN connect_latency_ms
                    WHEN connect_latency_ms IS NULL OR connect_latency_ms < ?4 THEN ?4
                    ELSE connect_latency_ms
                END,
                first_byte_latency_ms = CASE
                    WHEN ?5 IS NULL THEN first_byte_latency_ms
                    WHEN first_byte_latency_ms IS NULL OR first_byte_latency_ms < ?5 THEN ?5
                    ELSE first_byte_latency_ms
                END,
                compact_support_status = COALESCE(?6, compact_support_status),
                compact_support_reason = COALESCE(?7, compact_support_reason)
            WHERE id = ?1
              AND status = ?3
              AND finished_at IS NULL
              AND (
                    COALESCE(phase, '') <> ?2
                    OR (?4 IS NOT NULL AND (connect_latency_ms IS NULL OR connect_latency_ms < ?4))
                    OR (?5 IS NOT NULL AND (first_byte_latency_ms IS NULL OR first_byte_latency_ms < ?5))
                    OR (?6 IS NOT NULL AND COALESCE(compact_support_status, '') <> ?6)
                    OR (?7 IS NOT NULL AND COALESCE(compact_support_reason, '') <> ?7)
                  )
            "#,
        )
        .bind(progress.attempt_id)
        .bind(&progress.phase)
        .bind(progress.pending_status)
        .bind(progress.connect_latency_ms)
        .bind(progress.first_byte_latency_ms)
        .bind(progress.compact_support_status.as_deref())
        .bind(progress.compact_support_reason.as_deref())
        .execute(tx.as_mut())
        .await?;
    }

    let mut terminal_overlay_keys = Vec::new();
    if !batch.invocation_derived.is_empty() {
        let target_invocation_id = batch
            .invocation_derived
            .keys()
            .next_back()
            .copied()
            .unwrap_or_default();
        let live_rollup_cursor_before =
            load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS)
                .await?;
        replay_live_invocation_hourly_rollups_until_tx(tx.as_mut(), target_invocation_id).await?;
        let live_rollup_cursor_after =
            load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS)
                .await?;
        let skipped_terminal_ids = batch
            .invocation_derived
            .keys()
            .filter(|invocation_id| **invocation_id <= live_rollup_cursor_before)
            .copied()
            .collect::<Vec<_>>();
        if !skipped_terminal_ids.is_empty() {
            recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &skipped_terminal_ids)
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
                tx.as_mut(),
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
        .execute(tx.as_mut())
        .await?;
    }

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
        .execute(tx.as_mut())
        .await?;
    }

    tx.commit().await?;

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

    if should_invalidate_prompt_cache_conversations
        && let Some(cache) = prompt_cache_conversation_cache
    {
        invalidate_prompt_cache_conversations_cache(cache).await;
    }
    Ok(deferred_batch)
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
