use super::*;

pub(crate) async fn recover_guard_dropped_pool_invocation_orphan_with_prompt_cache_key(
    state: &AppState,
    selector: InvocationRecoverySelector,
    recovery_trigger: &'static str,
    prompt_cache_key: Option<String>,
) -> Result<()> {
    let mut flush_completed = false;
    let mut flush_error = None;
    let result = async {
        for attempt in 1..=3 {
            match state.sqlite_batch_writer.flush_now(&state.pool).await {
                Ok(()) => {
                    flush_completed = true;
                    break;
                }
                Err(err) => {
                    flush_error = Some(err);
                    if attempt < 3 {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        }
        if !flush_completed {
            return Err(flush_error.expect("dropped invocation flush should record its error"));
        }

        let recovered_invocations = {
            let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
                .await;
            let dashboard_reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
            let _dashboard_reconcile_guard = dashboard_reconcile_gate.lock().await;
            recover_proxy_invocations_with_scope(
                &state.pool,
                ProxyInvocationRecoveryScope::Selectors(std::slice::from_ref(&selector)),
            )
            .await?
        };

        if recovered_invocations.is_empty() {
            terminalize_proxy_runtime_snapshot_by_key(
                state,
                &selector.invoke_id,
                &selector.occurred_at,
                recovery_trigger,
            );
            schedule_dashboard_activity_live_snapshot(state);
            return Ok(());
        }

        info!(
            invoke_id = %selector.invoke_id,
            occurred_at = %selector.occurred_at,
            recovered_invocations = recovered_invocations.len(),
            recovery_trigger,
            "recovered pool invocation orphan after request future dropped"
        );

        broadcast_recovered_proxy_invocations(state, &recovered_invocations).await
    }
    .await;

    if flush_completed {
        state
            .prompt_cache_conversation_cache
            .lock()
            .await
            .identity_cache
            .range_manager
            .reconcile_persistence(&selector.invoke_id);
    }
    if let Some(prompt_cache_key) = prompt_cache_key {
        if flush_completed {
            release_active_prompt_cache_conversation(
                &state.prompt_cache_conversation_cache,
                &prompt_cache_key,
            )
            .await;
        } else {
            warn!(
                prompt_cache_key_fingerprint = %prompt_cache_key_fingerprint(&prompt_cache_key),
                error = ?result.as_ref().err(),
                "retaining prompt-cache conversation lease until dropped-invocation batch flush succeeds"
            );
        }
    }

    result
}
