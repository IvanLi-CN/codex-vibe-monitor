struct PersistedTerminal {
    terminal: BatchedTerminalInvocationWrite,
    invocation_id: i64,
    occurred_at: String,
    payload_metadata: Option<crate::TerminalPayloadMetadata>,
    summary_delta: Option<(crate::DashboardActivityTerminalDelta, bool)>,
}

async fn flush_pending_batch_inner(
    pool: &Pool<Sqlite>,
    batch: &PendingBatch,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    dependencies: SqliteBatchFlushDependencies<'_>,
) -> Result<PendingBatch> {
    let SqliteBatchFlushDependencies(prompt_cache, runtime_store, _, _, _, reconcile_guard, _) =
        dependencies;
    let _dashboard_reconcile_guard = reconcile_guard.lock().await;
    let (persisted_terminals, mut invalidate_prompt_cache) =
        persist_terminal_batch(pool, batch).await?;
    let mut deferred_batch = PendingBatch::default();
    publish_persisted_terminals(
        &mut deferred_batch,
        persisted_terminals,
        dependencies,
        &mut invalidate_prompt_cache,
    )
    .await;
    wake_batch_startup_backfill_tasks(pool, pricing_catalog, batch).await?;

    if batch.has_non_terminal_writes() {
        let mut tx = pool.begin().await?;
        flush_attempt_progress_tx(tx.as_mut(), batch).await?;
        let terminal_overlay_keys =
            flush_invocation_derived_tx(tx.as_mut(), batch, &mut deferred_batch).await?;
        flush_account_selected_touches_tx(tx.as_mut(), batch).await?;
        flush_system_task_finishes_tx(tx.as_mut(), batch).await?;
        tx.commit().await?;
        remove_terminal_overlays(runtime_store, terminal_overlay_keys);
    }
    if invalidate_prompt_cache && let Some(cache) = prompt_cache {
        invalidate_prompt_cache_conversations_cache(cache).await;
    }
    Ok(deferred_batch)
}

impl PendingBatch {
    fn has_non_terminal_writes(&self) -> bool {
        !self.attempt_progress.is_empty()
            || !self.invocation_derived.is_empty()
            || !self.account_selected_touches.is_empty()
            || !self.system_task_finishes.is_empty()
    }
}

async fn persist_terminal_batch(
    pool: &Pool<Sqlite>,
    batch: &PendingBatch,
) -> Result<(Vec<PersistedTerminal>, bool)> {
    if batch.terminal_invocations.is_empty() {
        return Ok((Vec::new(), false));
    }
    let mut tx = pool.begin().await?;
    let mut persisted = Vec::with_capacity(batch.terminal_invocations.len());
    let mut invalidate_prompt_cache = false;
    for terminal in batch.terminal_invocations.values() {
        let record = persist_terminal_record_tx(tx.as_mut(), terminal).await?;
        invalidate_prompt_cache |= record
            .payload_metadata
            .as_ref()
            .and_then(|metadata| metadata.prompt_cache_key.as_deref())
            .is_some_and(|key| !key.trim().is_empty());
        persisted.push(record);
    }
    append_terminal_summary_descriptor_tx(tx.as_mut(), &persisted).await?;
    tx.commit().await?;
    Ok((persisted, invalidate_prompt_cache))
}

async fn persist_terminal_record_tx(
    connection: &mut SqliteConnection,
    terminal: &BatchedTerminalInvocationWrite,
) -> Result<PersistedTerminal> {
    let persisted = if terminal.raw_capture {
        let capture_started = terminal.capture_started.unwrap_or_else(Instant::now);
        persist_proxy_capture_record_tx(connection, capture_started, terminal.record.clone(), false)
            .await
            .with_context(|| "flush terminal raw proxy invocation")?
    } else {
        persist_proxy_capture_runtime_record_tx(connection, terminal.record.clone(), false)
            .await
            .with_context(|| "flush terminal runtime proxy invocation")?
    };
    let (invocation_id, occurred_at) = if let Some(record) = persisted.as_ref() {
        (record.id, record.occurred_at.clone())
    } else {
        let identity = load_persisted_invocation_identity_tx(
            connection,
            &terminal.record.invoke_id,
            &terminal.record.occurred_at,
        )
        .await?;
        let identity = identity.ok_or_else(|| {
            anyhow!(
                "terminal write completed without a persisted identity: invoke_id={} occurred_at={}",
                terminal.record.invoke_id,
                terminal.record.occurred_at
            )
        })?;
        (identity.id, terminal.record.occurred_at.clone())
    };
    let payload_metadata = persisted
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
            let replayed_after_restart = sequence == 0;
            if !replayed_after_restart {
                delta.terminal_sequence = sequence;
            }
            (delta, replayed_after_restart)
        })
    });
    Ok(PersistedTerminal {
        terminal: terminal.clone(),
        invocation_id,
        occurred_at,
        payload_metadata,
        summary_delta,
    })
}

async fn append_terminal_summary_descriptor_tx(
    connection: &mut SqliteConnection,
    persisted: &[PersistedTerminal],
) -> Result<()> {
    let entries = persisted
        .iter()
        .map(|record| SummarySourceChangeEntry {
            row_id: record.invocation_id,
            invoke_id: record.terminal.record.invoke_id.clone(),
            occurred_at: record.occurred_at.clone(),
            upstream_account_id: record
                .payload_metadata
                .as_ref()
                .and_then(|metadata| metadata.upstream_account_id),
            current_rank: None,
        })
        .collect::<Vec<_>>();
    let source_revision = persisted
        .iter()
        .filter_map(|record| record.terminal.dashboard_terminal_sequence)
        .max()
        .unwrap_or_default();
    let descriptor = SummarySourceChangeDescriptor::terminal_batch(source_revision, entries)?;
    append_summary_source_change_descriptor_tx(connection, &descriptor)
        .await
        .map(|_| ())
}

async fn publish_persisted_terminals(
    deferred_batch: &mut PendingBatch,
    mut persisted: Vec<PersistedTerminal>,
    dependencies: SqliteBatchFlushDependencies<'_>,
    invalidate_prompt_cache: &mut bool,
) {
    let SqliteBatchFlushDependencies(_, _, dashboard, summary, projection, _, _) = dependencies;
    persisted.sort_by_key(|record| {
        record
            .terminal
            .dashboard_terminal_sequence
            .unwrap_or(u64::MAX)
    });
    for record in persisted {
        let PersistedTerminal {
            terminal,
            invocation_id,
            occurred_at,
            payload_metadata,
            summary_delta,
        } = record;
        deferred_batch.add_startup_backfill_wake_tasks(&terminal.startup_backfill_tasks);
        if let Some(cache) = dashboard.lock().ok().and_then(|guard| guard.clone()) {
            acknowledge_dashboard_activity_terminal_record(
                &cache,
                &terminal.record.invoke_id,
                &terminal.record.occurred_at,
                invocation_id,
                terminal.dashboard_terminal_sequence,
            )
            .await;
        }
        if let (Some((summary_delta, replayed_after_restart)), Some(hub)) = (
            summary_delta,
            summary.lock().ok().and_then(|guard| guard.clone()),
        ) {
            if replayed_after_restart {
                hub.acknowledge_replayed_summary_delta(summary_delta).await;
            } else {
                hub.acknowledge_summary_delta(summary_delta).await;
            }
        }
        if let Some(hub) = projection.lock().ok().and_then(|guard| guard.clone()) {
            let event_ids = if terminal.terminal_projection_event_ids.is_empty() {
                vec![None]
            } else {
                terminal
                    .terminal_projection_event_ids
                    .iter()
                    .copied()
                    .map(Some)
                    .collect()
            };
            for event_id in event_ids {
                hub.acknowledge_persisted(
                    event_id,
                    &terminal.record.invoke_id,
                    &terminal.record.occurred_at,
                    invocation_id,
                );
            }
        }
        let payload_metadata = payload_metadata.unwrap_or_else(|| {
            crate::terminal_payload_metadata(terminal.record.payload.as_deref())
        });
        *invalidate_prompt_cache |= payload_metadata
            .prompt_cache_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty());
        deferred_batch.push(SqliteBatchWrite::InvocationDerived(
            BatchedInvocationDerivedWrites {
                invocation_id,
                occurred_at,
                upstream_account_id: payload_metadata.upstream_account_id,
                terminal_overlay_key: Some((
                    terminal.record.invoke_id,
                    terminal.record.occurred_at,
                )),
            },
        ));
    }
}

async fn wake_batch_startup_backfill_tasks(
    pool: &Pool<Sqlite>,
    pricing_catalog: Option<&Arc<RwLock<PricingCatalog>>>,
    batch: &PendingBatch,
) -> Result<()> {
    if batch.startup_backfill_wake_tasks.is_empty() {
        return Ok(());
    }
    let pricing_catalog = pricing_catalog.map(|catalog| async { catalog.read().await.clone() });
    let pricing_catalog = match pricing_catalog {
        Some(catalog) => Some(catalog.await),
        None => None,
    };
    let wake_tasks = batch
        .startup_backfill_wake_tasks
        .iter()
        .copied()
        .filter(|task| {
            let available = pricing_catalog.is_some() || !matches!(task, StartupBackfillTask::ProxyCost);
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
    .await
    .map(|_| ())
}

async fn flush_attempt_progress_tx(
    connection: &mut SqliteConnection,
    batch: &PendingBatch,
) -> Result<()> {
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
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

async fn flush_invocation_derived_tx(
    connection: &mut SqliteConnection,
    batch: &PendingBatch,
    deferred_batch: &mut PendingBatch,
) -> Result<Vec<(String, String)>> {
    if batch.invocation_derived.is_empty() {
        return Ok(Vec::new());
    }
    let target_invocation_id = batch
        .invocation_derived
        .keys()
        .next_back()
        .copied()
        .unwrap_or_default();
    let cursor_before =
        load_hourly_rollup_live_progress_tx(&mut *connection, HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .await?;
    replay_live_invocation_hourly_rollups_until_tx(&mut *connection, target_invocation_id).await?;
    let cursor_after =
        load_hourly_rollup_live_progress_tx(&mut *connection, HOURLY_ROLLUP_DATASET_INVOCATIONS)
            .await?;
    let skipped_ids = batch
        .invocation_derived
        .keys()
        .filter(|invocation_id| **invocation_id <= cursor_before)
        .copied()
        .collect::<Vec<_>>();
    if !skipped_ids.is_empty() {
        recompute_invocation_hourly_rollups_for_ids_tx(&mut *connection, &skipped_ids).await?;
    }
    let mut overlay_keys = Vec::new();
    for derived in batch.invocation_derived.values() {
        if derived.invocation_id > cursor_after {
            deferred_batch.push(SqliteBatchWrite::InvocationDerived(derived.clone()));
            continue;
        }
        if let Some(key) = derived.terminal_overlay_key.clone() {
            overlay_keys.push(key);
        }
        crate::touch_upstream_account_last_activity_tx(
            &mut *connection,
            &derived.occurred_at,
            derived.upstream_account_id,
        )
        .await?;
    }
    Ok(overlay_keys)
}

async fn flush_account_selected_touches_tx(
    connection: &mut SqliteConnection,
    batch: &PendingBatch,
) -> Result<()> {
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
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

async fn flush_system_task_finishes_tx(
    connection: &mut SqliteConnection,
    batch: &PendingBatch,
) -> Result<()> {
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
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

fn remove_terminal_overlays(
    runtime_store: &Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    overlay_keys: Vec<(String, String)>,
) {
    if overlay_keys.is_empty() {
        return;
    }
    let Some(runtime_store) = runtime_store
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
    else {
        return;
    };
    for (invoke_id, occurred_at) in overlay_keys {
        let removed = runtime_store.remove_persisted_terminal_overlay(&invoke_id, &occurred_at);
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %occurred_at,
            terminal_runtime_overlay_removed_after_derived_flush = removed,
            "removed persisted terminal record from memory runtime overlay after derived writes"
        );
    }
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
