use super::*;

pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan(
    state: &AppState,
    pending_attempt_record: PendingPoolAttemptRecord,
    first_byte_observed: bool,
    terminal_outcome_observed: bool,
) -> Result<()> {
    if pending_attempt_record.attempt_public_id.is_some()
        && pending_attempt_record.attempt_id.is_none()
    {
        if first_byte_observed && terminal_outcome_observed {
            return Ok(());
        }
        let Some(id) = pending_attempt_record.attempt_public_id.as_deref() else {
            return Ok(());
        };
        let Some(mut snapshot) = state.proxy_runtime_attempts.get(
            &pending_attempt_record.invoke_id,
            &pending_attempt_record.occurred_at,
            id,
        ) else {
            return Ok(());
        };
        if snapshot.finished_at.is_some() {
            return Ok(());
        }
        snapshot.finished_at = Some(shanghai_now_string());
        snapshot.status = POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_INTERRUPTED.to_string();
        snapshot.phase = Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED.to_string());
        snapshot.failure_kind = Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED.to_string());
        snapshot.error_message = Some(POOL_ATTEMPT_INTERRUPTED_MESSAGE.to_string());
        let Some(snapshot) = state.proxy_runtime_attempts.claim_interruption(snapshot) else {
            return Ok(());
        };
        let mut record = crate::terminal_journal::interrupted_terminal_record(
            &snapshot.invoke_id,
            &snapshot.occurred_at,
            std::slice::from_ref(&snapshot),
        );
        record.payload = Some(json!({"source":"proxy","routeMode":"pool","endpoint":snapshot.endpoint,"upstreamAccountId":snapshot.upstream_account_id,"requestModel":snapshot.request_model}).to_string());
        persist_and_broadcast_proxy_capture(state, Instant::now(), record).await?;
        forget_priority_handoff_attempt_for_invoke(&pending_attempt_record.invoke_id);
        return Ok(());
    }
    state.sqlite_batch_writer.flush_now(&state.pool).await?;
    persist_pool_upstream_request_attempt_metadata_before_recovery(
        &state.pool,
        &pending_attempt_record,
    )
    .await?;

    if first_byte_observed && terminal_outcome_observed {
        info!(
            invoke_id = %pending_attempt_record.invoke_id,
            attempt_id = pending_attempt_record.attempt_id,
            first_byte_latency_ms = pending_attempt_record.first_byte_latency_ms,
            recovery_trigger = "drop_guard",
            "skipping guard-based orphan recovery because a terminal post-first-byte outcome was already observed"
        );
        return Ok(());
    }
    if first_byte_observed {
        info!(
            invoke_id = %pending_attempt_record.invoke_id,
            attempt_id = pending_attempt_record.attempt_id,
            first_byte_latency_ms = pending_attempt_record.first_byte_latency_ms,
            recovery_trigger = "drop_guard",
            "recovering post-first-byte orphan because the stream task ended before any terminal outcome was observed"
        );
    }

    let (recovered_attempts, recovered_invocations) = {
        let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
            .await;
        let dashboard_reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
        let _dashboard_reconcile_guard = dashboard_reconcile_gate.lock().await;
        let mut tx = crate::observability::diagnostics::wait(
            crate::observability::diagnostics::Resource::DbPool,
            state.pool.begin(),
        )
        .await?;
        let recovered_attempts = match pending_attempt_record.attempt_id {
            Some(attempt_id) => {
                recover_pool_upstream_request_attempts_with_scope_tx(
                    tx.as_mut(),
                    PoolAttemptRecoveryScope::SpecificEarlyPhase { attempt_id },
                )
                .await?
            }
            None => Vec::new(),
        };

        let recovered_invocations =
            if pending_attempt_record.attempt_id.is_none() || !recovered_attempts.is_empty() {
                let selector = InvocationRecoverySelector::from(&pending_attempt_record);
                recover_proxy_invocations_with_scope_tx(
                    tx.as_mut(),
                    ProxyInvocationRecoveryScope::Selectors(std::slice::from_ref(&selector)),
                )
                .await?
            } else {
                Vec::new()
            };
        tx.commit().await?;
        (recovered_attempts, recovered_invocations)
    };

    if pending_attempt_record.attempt_id.is_none() {
        discard_runtime_attempt_for_drop(state, &pending_attempt_record);
    }

    let should_clean_up_route = pending_attempt_record.attempt_id.is_none()
        || !recovered_attempts.is_empty()
        || !recovered_invocations.is_empty();
    let record_route_failure = (pending_attempt_record.attempt_id.is_none()
        || !recovered_attempts.is_empty())
        && should_record_route_failure_after_attempt_recovery(
            state,
            &pending_attempt_record.invoke_id,
            &pending_attempt_record.occurred_at,
            !recovered_invocations.is_empty(),
        )
        .await;

    if recovered_invocations.is_empty() {
        terminalize_proxy_runtime_snapshot_by_key(
            state,
            &pending_attempt_record.invoke_id,
            &pending_attempt_record.occurred_at,
            "drop_guard",
        );
        schedule_dashboard_activity_live_snapshot(state);
    } else {
        remove_proxy_runtime_snapshot_by_key(
            state,
            &pending_attempt_record.invoke_id,
            &pending_attempt_record.occurred_at,
            "drop_guard",
        );
        schedule_dashboard_activity_live_snapshot(state);
    }

    if should_clean_up_route {
        clean_up_pool_route_after_orphan_recovery(
            state,
            &pending_attempt_record.invoke_id,
            pending_attempt_record.sticky_key.as_deref(),
            Some(pending_attempt_record.upstream_account_id),
            "drop_guard",
            record_route_failure,
        )
        .await;
    }

    if recovered_attempts.is_empty() && recovered_invocations.is_empty() {
        return Ok(());
    }

    if !recovered_attempts.is_empty()
        && let Err(err) =
            broadcast_pool_upstream_attempts_snapshot(state, &pending_attempt_record.invoke_id)
                .await
    {
        warn!(
            invoke_id = %pending_attempt_record.invoke_id,
            error = %err,
            "failed to broadcast guard-recovered pool attempt snapshot"
        );
    }
    broadcast_recovered_proxy_invocations(state, &recovered_invocations).await?;

    info!(
        invoke_id = %pending_attempt_record.invoke_id,
        attempt_id = pending_attempt_record.attempt_id,
        recovered_attempts = recovered_attempts.len(),
        recovered_invocations = recovered_invocations.len(),
        recovery_trigger = "drop_guard",
        "recovered pool early-phase orphan after request future dropped"
    );

    Ok(())
}
