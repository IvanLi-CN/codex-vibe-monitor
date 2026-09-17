/// Runs the durable historical supervisor independently from rolling Projection maintenance.
/// It retains priority only while the startup's finite durable obligation set is being recovered;
/// afterwards periodic turns are best-effort and cannot delay current/rolling refresh.
pub(crate) fn spawn_summary_coverage_recovery_maintenance(
    state: Arc<AppState>,
    startup_priority: crate::db_pressure::DbBackgroundPriorityReservation,
) {
    tokio::spawn(async move {
        if let Err(error) =
            SummaryCoverageRecoverySupervisor::run_with_startup_priority_reservation(
                state.as_ref(),
                startup_priority,
            )
            .await
        {
            warn!(error = ?error, "initial summary historical coverage recovery deferred");
        }

        let mut cadence = tokio::time::interval(SUMMARY_SNAPSHOT_REFRESH_INTERVAL);
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = cadence.tick() => {
                    if let Err(error) = SummaryCoverageRecoverySupervisor::run(state.as_ref()).await {
                        warn!(error = ?error, "summary historical coverage recovery deferred");
                    }
                }
            }
        }
    });
}

async fn renew_summary_projection_freshness_if_generation_matches(
    state: &AppState,
) -> Result<bool> {
    let generation_fence = load_summary_projection_generation_fence(state).await?;
    let renewed = state
        .subscription_hub
        .renew_summary_projection_freshness_if_generation_matches(generation_fence)
        .await;
    if renewed {
        debug!("summary projection freshness renewed for unchanged durable generation");
    }
    Ok(renewed)
}

async fn renew_summary_projection_freshness_if_live_tail_matches(state: &AppState) -> Result<bool> {
    let generation_fence = load_summary_projection_generation_fence(state).await?;
    let renewed = state
        .subscription_hub
        .renew_summary_projection_freshness_if_live_tail_matches(generation_fence)
        .await;
    if renewed {
        debug!(
            "summary projection freshness renewed for unchanged live tail during coverage recovery"
        );
    }
    Ok(renewed)
}

async fn renew_summary_projection_freshness_if_coverage_matches(state: &AppState) -> Result<bool> {
    let generation_fence = load_summary_projection_generation_fence(state).await?;
    let renewed = state
        .subscription_hub
        .renew_summary_projection_freshness_if_coverage_matches(generation_fence)
        .await;
    if renewed {
        debug!("summary all-time coverage freshness renewed for unchanged coverage fence");
    }
    Ok(renewed)
}

/// Reconstruct the bounded durable source tail after a restart or when the in-process delta
/// queue has been dropped.  Only descriptor identities are read from the journal; the canonical
/// source row is hydrated in 400-id chunks and exposed through the existing in-memory replay
/// overlay.  This function deliberately never invokes the full live admission builder.
const SUMMARY_LIVE_TAIL_RECONCILIATION_SCOPE: &str = "summary-global";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SummaryLiveTailReconciliationCheckpoint {
    format_version: i64,
    recovery_epoch: i64,
    base_projection_revision: i64,
    target_terminal_watermark: u64,
    target_source_cursor: u64,
    next_source_cursor: u64,
    state: String,
}

async fn load_summary_live_tail_reconciliation_checkpoint(
    pool: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Option<SummaryLiveTailReconciliationCheckpoint>> {
    let row = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, String)>(
        "SELECT format_version, recovery_epoch, base_projection_revision, \
                target_terminal_watermark, target_source_cursor, next_source_cursor, state \
         FROM summary_live_tail_reconciliation_checkpoint WHERE scope = ?1",
    )
    .bind(SUMMARY_LIVE_TAIL_RECONCILIATION_SCOPE)
    .fetch_optional(pool)
    .await
    .context("load Summary live-tail reconciliation checkpoint")?;
    row.map(
        |(
            format_version,
            recovery_epoch,
            base_projection_revision,
            target_terminal_watermark,
            target_source_cursor,
            next_source_cursor,
            state,
        )| {
            Ok(SummaryLiveTailReconciliationCheckpoint {
                format_version,
                recovery_epoch,
                base_projection_revision,
                target_terminal_watermark: u64::try_from(target_terminal_watermark)
                    .context("live-tail checkpoint target watermark is negative")?,
                target_source_cursor: u64::try_from(target_source_cursor)
                    .context("live-tail checkpoint target cursor is negative")?,
                next_source_cursor: u64::try_from(next_source_cursor)
                    .context("live-tail checkpoint next cursor is negative")?,
                state,
            })
        },
    )
    .transpose()
}

async fn store_summary_live_tail_reconciliation_checkpoint(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    checkpoint: SummaryLiveTailReconciliationCheckpoint,
) -> Result<()> {
    let mut transaction = pool
        .begin()
        .await
        .context("begin Summary live-tail reconciliation checkpoint")?;
    store_summary_live_tail_reconciliation_checkpoint_tx(&mut transaction, checkpoint).await?;
    transaction
        .commit()
        .await
        .context("commit Summary live-tail reconciliation checkpoint")?;
    Ok(())
}

async fn store_summary_live_tail_reconciliation_checkpoint_tx(
    connection: &mut sqlx::SqliteConnection,
    checkpoint: SummaryLiveTailReconciliationCheckpoint,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO summary_live_tail_reconciliation_checkpoint \
         (scope, format_version, recovery_epoch, base_projection_revision, \
          target_terminal_watermark, target_source_cursor, next_source_cursor, state, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now')) \
         ON CONFLICT(scope) DO UPDATE SET \
          format_version = excluded.format_version, \
          recovery_epoch = excluded.recovery_epoch, \
          base_projection_revision = excluded.base_projection_revision, \
          target_terminal_watermark = excluded.target_terminal_watermark, \
          target_source_cursor = excluded.target_source_cursor, \
          next_source_cursor = excluded.next_source_cursor, \
          state = excluded.state, updated_at = datetime('now')",
    )
    .bind(SUMMARY_LIVE_TAIL_RECONCILIATION_SCOPE)
    .bind(checkpoint.format_version)
    .bind(checkpoint.recovery_epoch)
    .bind(checkpoint.base_projection_revision)
    .bind(
        i64::try_from(checkpoint.target_terminal_watermark)
            .context("live-tail checkpoint target watermark overflow")?,
    )
    .bind(
        i64::try_from(checkpoint.target_source_cursor)
            .context("live-tail checkpoint target cursor overflow")?,
    )
    .bind(
        i64::try_from(checkpoint.next_source_cursor)
            .context("live-tail checkpoint next cursor overflow")?,
    )
    .bind(checkpoint.state)
    .execute(&mut *connection)
    .await
    .context("store Summary live-tail reconciliation checkpoint")?;
    Ok(())
}

async fn record_summary_live_tail_readiness(
    state: &AppState,
    reason: &str,
    stage: &str,
    gap_count: usize,
    watermark: u64,
    epoch: u64,
    started: Instant,
) {
    state
        .subscription_hub
        .set_summary_live_tail_readiness(SummaryLiveTailReadiness {
            reason: reason.to_string(),
            stage: stage.to_string(),
            gap_count: gap_count as u64,
            watermark,
            epoch,
            elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        })
        .await;
}

struct SummarySourceChangeTail {
    after_cursor: u64,
    last_cursor: u64,
    records: Vec<SummarySourceChangeRecord>,
}

enum SummarySourceChangeTailLoad {
    Reconciled(bool),
    Ready(SummarySourceChangeTail),
}

async fn load_summary_source_change_tail_for_recovery(
    state: &AppState,
) -> Result<SummarySourceChangeTailLoad> {
    let mut after_cursor = state.subscription_hub.summary_source_change_cursor().await;
    if after_cursor == 0 {
        after_cursor = load_summary_source_change_checkpoint(
            &state.pool,
            SUMMARY_SOURCE_CHANGE_CHECKPOINT_SCOPE,
        )
        .await?;
        if after_cursor > 0 {
            state
                .subscription_hub
                .advance_summary_source_change_cursor(after_cursor)
                .await;
        }
    }
    // The reconciliation checkpoint describes an in-flight read, not a durable publication.
    // Never use its next cursor to skip source descriptors: a crash can happen after this
    // checkpoint write and before the immutable Projection swap. Replaying the bounded tail is
    // idempotent; skipping it would lose a committed terminal from the served overlay.
    let compaction_boundary = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM summary_source_change_compaction_proof \
         WHERE first_cursor <= ?1 \
           AND (retained_after_cursor > (?2 + 1) OR retained_after_cursor <= 0 \
                OR NOT json_valid(proof_json) \
                OR json_type(json_extract(proof_json, '$.retainedAfterCursor')) NOT IN ('integer', 'real') \
                OR CAST(json_extract(proof_json, '$.retainedAfterCursor') AS INTEGER) <> retained_after_cursor))",
    )
    .bind(
        i64::try_from(after_cursor.saturating_add(1))
            .context("source compaction boundary overflow")?,
    )
    .bind(i64::try_from(after_cursor).context("source compaction cursor overflow")?)
    .fetch_one(&state.pool)
    .await
    .context("load summary source compaction proof")?;
    if compaction_boundary != 0 {
        state
            .subscription_hub
            .record_summary_source_change_gap(after_cursor.saturating_add(1))
            .await;
        // The descriptor prefix may have been compacted, but the canonical invocation table is
        // still an authoritative recovery source. Reconstruct it only when the complete tail
        // fits the existing bounded replay budget; otherwise retain the broad fail-closed proof.
        return Ok(SummarySourceChangeTailLoad::Reconciled(
            restore_compacted_summary_source_tail(state).await?,
        ));
    }
    let tail = load_summary_source_change_tail(
        &state.pool,
        after_cursor,
        SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_ENTRIES,
    )
    .await?;
    let Some(last_cursor) = tail.last().map(|record| record.cursor) else {
        return Ok(SummarySourceChangeTailLoad::Reconciled(false));
    };
    let mut expected_cursor = after_cursor.saturating_add(1);
    for record in &tail {
        if record.cursor != expected_cursor {
            state
                .subscription_hub
                .record_summary_source_change_gap(expected_cursor)
                .await;
            return Ok(SummarySourceChangeTailLoad::Reconciled(false));
        }
        expected_cursor = record.cursor.saturating_add(1);
    }
    Ok(SummarySourceChangeTailLoad::Ready(
        SummarySourceChangeTail {
            after_cursor,
            last_cursor,
            records: tail,
        },
    ))
}

async fn hydrate_summary_source_change_tail_rows(
    state: &AppState,
    records: &[SummarySourceChangeRecord],
) -> Result<HashMap<i64, ApiInvocation>> {
    let ids = records
        .iter()
        .flat_map(|record| record.descriptor.entries.iter().map(|entry| entry.row_id))
        .collect::<Vec<_>>();
    let mut rows_by_id = HashMap::<i64, ApiInvocation>::new();
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE) {
        if chunk.is_empty() {
            continue;
        }
        let mut query = build_invocation_select_query();
        query.push(" WHERE id IN (");
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
        }
        query.push(") ORDER BY id ASC");
        for row in query
            .build_query_as::<ApiInvocation>()
            .fetch_all(&state.pool)
            .await
            .context("summary source descriptor row reconstruction failed")?
        {
            rows_by_id.insert(row.id, row);
        }
    }
    Ok(rows_by_id)
}

struct SummarySourceChangeReconciliationContext {
    target_terminal_watermark: u64,
    base_projection_revision: i64,
    base_generation_fence: SummaryProjectionGenerationFence,
    recovery_epoch: i64,
}

async fn prepare_summary_source_change_reconciliation(
    state: &AppState,
    records: &[SummarySourceChangeRecord],
    last_cursor: u64,
    after_cursor: u64,
) -> Result<SummarySourceChangeReconciliationContext> {
    let target_terminal_watermark = records
        .iter()
        .filter(|record| record.descriptor.source_kind == "terminal_batch")
        .map(|record| record.descriptor.source_revision)
        .max()
        .unwrap_or_default();
    let base_projection_revision = state
        .subscription_hub
        .summary_projection()
        .await
        .map(|projection| projection.revision() as i64)
        .unwrap_or_default();
    let base_generation_fence = load_summary_projection_generation_fence(state).await?;
    let previous_epoch = load_summary_live_tail_reconciliation_checkpoint(&state.pool)
        .await?
        .map(|checkpoint| checkpoint.recovery_epoch)
        .unwrap_or_default();
    store_summary_live_tail_reconciliation_checkpoint(
        &state.pool,
        SummaryLiveTailReconciliationCheckpoint {
            format_version: 1,
            recovery_epoch: previous_epoch.saturating_add(1),
            base_projection_revision,
            target_terminal_watermark,
            target_source_cursor: last_cursor,
            next_source_cursor: after_cursor,
            state: "reconciling".to_string(),
        },
    )
    .await?;
    Ok(SummarySourceChangeReconciliationContext {
        target_terminal_watermark,
        base_projection_revision,
        base_generation_fence,
        recovery_epoch: previous_epoch.saturating_add(1),
    })
}

async fn replay_summary_source_change_tail(
    state: &AppState,
    records: Vec<SummarySourceChangeRecord>,
    rows_by_id: &HashMap<i64, ApiInvocation>,
    context: SummarySourceChangeReconciliationContext,
    last_cursor: u64,
    after_cursor: u64,
) -> Result<bool> {
    let mut restored = false;
    let mut complete = true;
    let mut replayed_deltas = Vec::new();
    for record in records {
        if record.descriptor.source_kind != "terminal_batch" {
            // Archive and rollup descriptors are durable fence evidence for the independent
            // historical reconciler. They must not be interpreted as live invocation rows.
            state
                .subscription_hub
                .mark_summary_projection_historical_recovery_required()
                .await;
            continue;
        }
        for entry in record.descriptor.entries {
            let Some(row) = rows_by_id.get(&entry.row_id) else {
                complete = false;
                state
                    .subscription_hub
                    .record_summary_source_change_scoped_gap(
                        record.cursor,
                        entry.upstream_account_id,
                        entry.occurred_at.clone(),
                        Some(entry.row_id),
                        Some(entry.invoke_id.clone()),
                    )
                    .await;
                continue;
            };
            if row.invoke_id != entry.invoke_id
                || row.occurred_at != entry.occurred_at
                || row.upstream_account_id != entry.upstream_account_id
            {
                complete = false;
                state
                    .subscription_hub
                    .record_summary_source_change_scoped_gap(
                        record.cursor,
                        entry.upstream_account_id,
                        entry.occurred_at.clone(),
                        Some(entry.row_id),
                        Some(entry.invoke_id.clone()),
                    )
                    .await;
                continue;
            }
            let mut delta = persisted_dashboard_activity_terminal_delta(row);
            delta.persisted_row_id = Some(row.id);
            replayed_deltas.push(delta);
            restored = true;
        }
    }
    let reconciled = state
        .subscription_hub
        .reconcile_summary_delta_tail(
            &replayed_deltas,
            context.target_terminal_watermark,
            complete,
            context.base_projection_revision as u64,
            context.base_generation_fence,
        )
        .await;
    if complete && reconciled {
        // The durable source cursor is intentionally committed by the caller only after the
        // immutable Projection freshness renewal succeeds.  Persisting it here would make a
        // crash between checkpoint and publication lose a committed tail on restart.
    } else {
        store_summary_live_tail_reconciliation_checkpoint(
            &state.pool,
            SummaryLiveTailReconciliationCheckpoint {
                format_version: 1,
                recovery_epoch: context.recovery_epoch,
                base_projection_revision: context.base_projection_revision,
                target_terminal_watermark: context.target_terminal_watermark,
                target_source_cursor: last_cursor,
                next_source_cursor: after_cursor,
                state: "deferred".to_string(),
            },
        )
        .await?;
    }
    Ok(restored || reconciled)
}

async fn restore_summary_source_change_tail(state: &AppState) -> Result<bool> {
    let tail = match load_summary_source_change_tail_for_recovery(state).await? {
        SummarySourceChangeTailLoad::Ready(tail) => tail,
        SummarySourceChangeTailLoad::Reconciled(result) => return Ok(result),
    };
    let rows_by_id = hydrate_summary_source_change_tail_rows(state, &tail.records).await?;
    let context = prepare_summary_source_change_reconciliation(
        state,
        &tail.records,
        tail.last_cursor,
        tail.after_cursor,
    )
    .await?;
    replay_summary_source_change_tail(
        state,
        tail.records,
        &rows_by_id,
        context,
        tail.last_cursor,
        tail.after_cursor,
    )
    .await
}

async fn restore_compacted_summary_source_tail(state: &AppState) -> Result<bool> {
    const MAX_REPLAY_DELTAS: usize = 10_000;
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations \
         WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
    )
    .fetch_one(&state.pool)
    .await
    .context("summary compacted source tail count failed")?;
    if count < 0 || count as usize > MAX_REPLAY_DELTAS {
        return Ok(false);
    }
    let mut query = build_invocation_select_query();
    query.push(" WHERE ");
    query.push(" LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    query.push(" ORDER BY occurred_at ASC, id ASC LIMIT ");
    query.push_bind(MAX_REPLAY_DELTAS as i64 + 1);
    let rows = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&state.pool)
        .await
        .context("summary compacted source tail reconstruction failed")?;
    if rows.len() != count as usize {
        return Ok(false);
    }
    let projection = state.subscription_hub.summary_projection().await;
    let Some(projection) = projection else {
        return Ok(false);
    };
    let (_, initial_gap_count) = state.subscription_hub.summary_delta_journal_counts().await;
    for row in rows {
        if projection.contains_persisted_live_terminal_identity(
            row.id,
            &row.invoke_id,
            &row.occurred_at,
        ) {
            continue;
        }
        let mut delta = persisted_dashboard_activity_terminal_delta(&row);
        delta.persisted_row_id = Some(row.id);
        // Replay entries intentionally use sequence zero; their durable identity is sufficient
        // for exact deduplication while the ordered journal cursor is unavailable after compaction.
        state
            .subscription_hub
            .acknowledge_replayed_summary_delta(delta)
            .await;
    }
    let (_, gap_count) = state.subscription_hub.summary_delta_journal_counts().await;
    if gap_count == initial_gap_count {
        state
            .subscription_hub
            .clear_summary_source_compaction_gap()
            .await;
        return Ok(true);
    }
    Ok(false)
}

/// Compatibility recovery for source writers predating the durable descriptor hook.  Bound the
/// read to the legal current prefix and leave a historical reconciliation marker so a missing
/// descriptor can never silently make an old range look exact.
async fn restore_legacy_summary_source_tail(state: &AppState) -> Result<bool> {
    let mut query = build_invocation_select_query();
    query.push(" ORDER BY occurred_at DESC, id DESC LIMIT ");
    query.push_bind(state.config.list_limit_max.min(400) as i64);
    let rows = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&state.pool)
        .await
        .context("legacy summary source tail reconstruction failed")?;
    let projection = state.subscription_hub.summary_projection().await;
    let Some(projection) = projection else {
        return Ok(false);
    };
    let mut restored = false;
    for row in rows {
        if projection.contains_persisted_live_terminal_identity(
            row.id,
            &row.invoke_id,
            &row.occurred_at,
        ) {
            continue;
        }
        let mut delta = persisted_dashboard_activity_terminal_delta(&row);
        delta.persisted_row_id = Some(row.id);
        state
            .subscription_hub
            .acknowledge_replayed_summary_delta(delta)
            .await;
        // A legacy writer can advance a historical row without a durable descriptor. Keep its
        // proof scoped to the row's time/account so an independent current selection remains
        // exact while the background historical reconciliation is in flight.
        state
            .subscription_hub
            .record_summary_source_change_scoped_gap(
                0,
                row.upstream_account_id,
                row.occurred_at.clone(),
                Some(row.id),
                Some(row.invoke_id.clone()),
            )
            .await;
        restored = true;
    }
    if restored {
        state
            .subscription_hub
            .mark_summary_projection_historical_recovery_required()
            .await;
    }
    Ok(restored)
}
