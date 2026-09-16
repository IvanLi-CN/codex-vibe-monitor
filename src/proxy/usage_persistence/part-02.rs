pub(crate) async fn recover_proxy_invocations_with_scope_tx(
    tx: &mut SqliteConnection,
    scope: ProxyInvocationRecoveryScope<'_>,
) -> Result<Vec<RecoveredInvocationRow>> {
    let rows = match scope {
        ProxyInvocationRecoveryScope::AllInFlight => {
            sqlx::query_as::<_, RecoveredInvocationRow>(
                r#"
                UPDATE codex_invocations
                SET status = ?1,
                    error_message = ?2,
                    failure_kind = ?3,
                    failure_class = ?4,
                    is_actionable = 1
                WHERE source = ?5
                  AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                RETURNING id, invoke_id, occurred_at
                "#,
            )
            .bind(INVOCATION_STATUS_INTERRUPTED)
            .bind(INVOCATION_INTERRUPTED_MESSAGE)
            .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
            .bind(FAILURE_CLASS_SERVICE)
            .bind(SOURCE_PROXY)
            .fetch_all(&mut *tx)
            .await?
        }
        ProxyInvocationRecoveryScope::Selectors(selectors) => {
            let selectors: Vec<_> = selectors
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if selectors.is_empty() {
                return Ok(Vec::new());
            }

            let mut recovered = Vec::new();
            for chunk in selectors.chunks(PROXY_INVOCATION_RECOVERY_SELECTOR_BATCH_SIZE) {
                let mut query = QueryBuilder::<Sqlite>::new(
                    r#"
                    UPDATE codex_invocations
                    SET status = "#,
                );
                query.push_bind(INVOCATION_STATUS_INTERRUPTED);
                query.push(
                    r#",
                        error_message = "#,
                );
                query.push_bind(INVOCATION_INTERRUPTED_MESSAGE);
                query.push(
                    r#",
                        failure_kind = "#,
                );
                query.push_bind(PROXY_FAILURE_INVOCATION_INTERRUPTED);
                query.push(
                    r#",
                        failure_class = "#,
                );
                query.push_bind(FAILURE_CLASS_SERVICE);
                query.push(
                    r#",
                        is_actionable = 1
                    WHERE source = "#,
                );
                query.push_bind(SOURCE_PROXY);
                query.push(
                    r#"
                      AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                      AND (
                    "#,
                );
                let mut first = true;
                for selector in chunk {
                    if !first {
                        query.push(" OR ");
                    }
                    first = false;
                    query.push("(");
                    query.push("invoke_id = ");
                    query.push_bind(&selector.invoke_id);
                    query.push(" AND occurred_at = ");
                    query.push_bind(&selector.occurred_at);
                    query.push(")");
                }
                query.push(
                    r#"
                      )
                    RETURNING id, invoke_id, occurred_at
                    "#,
                );
                recovered.extend(
                    query
                        .build_query_as::<RecoveredInvocationRow>()
                        .fetch_all(&mut *tx)
                        .await?,
                );
            }
            recovered
        }
    };

    if !rows.is_empty() {
        let updated_ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
        recompute_invocation_hourly_rollups_for_ids_tx(&mut *tx, &updated_ids).await?;
        if let Some(max_id) = updated_ids.iter().copied().max() {
            save_hourly_rollup_live_progress_tx(
                &mut *tx,
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                max_id,
            )
            .await?;
        }
    }

    Ok(rows)
}

pub(crate) async fn recover_orphaned_proxy_invocations(pool: &Pool<Sqlite>) -> Result<u64> {
    Ok(
        recover_proxy_invocations_with_scope(pool, ProxyInvocationRecoveryScope::AllInFlight)
            .await?
            .len() as u64,
    )
}

pub(crate) fn stale_started_before_string(timeout: Duration, grace: Duration) -> String {
    let cutoff = Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::from_std(timeout + grace)
            .expect("pool orphan recovery cutoff should fit chrono duration");
    format_naive(cutoff)
}

pub(crate) async fn load_persisted_api_invocation(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    let mut tx = pool.begin().await?;
    let invocation = load_persisted_api_invocation_tx(tx.as_mut(), invoke_id, occurred_at).await?;
    tx.commit().await?;
    Ok(invocation)
}

pub(crate) async fn broadcast_recovered_proxy_invocations(
    state: &AppState,
    recovered: &[RecoveredInvocationRow],
) -> Result<()> {
    if recovered.is_empty() {
        return Ok(());
    }

    let selectors: Vec<_> = recovered
        .iter()
        .map(|row| InvocationRecoverySelector::new(row.invoke_id.clone(), row.occurred_at.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut records = Vec::new();
    for selector in selectors {
        match load_persisted_api_invocation(&state.pool, &selector.invoke_id, &selector.occurred_at)
            .await
        {
            Ok(record) => records.push(record),
            Err(err) => {
                warn!(
                    invoke_id = %selector.invoke_id,
                    occurred_at = %selector.occurred_at,
                    error = %err,
                    "failed to load recovered proxy invocation for runtime broadcast"
                );
            }
        }
    }

    if records.is_empty() {
        return Ok(());
    }

    invalidate_dashboard_activity_baselines_for_recovery(state).await;

    for record in &records {
        let delta = apply_dashboard_activity_terminal_record(state, record).await;
        debug!(
            invoke_id = %record.invoke_id,
            terminal_delta_applied_selection_count = delta.applied_selection_count,
            terminal_delta_duplicate = delta.duplicate,
            response_source = "memory",
            "applied recovered terminal record to dashboard activity read model"
        );
    }

    if records.iter().any(|record| {
        record
            .prompt_cache_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }

    let summary_invoke_id = records[0].invoke_id.clone();
    for record in &records {
        state
            .subscription_hub
            .publish_runtime_mutation(RuntimeMutation::invocation(
                record,
                RuntimeMutationKind::Recovery,
            ));
        #[cfg(test)]
        broadcast_test_record_payload(state, record);
    }
    schedule_dashboard_activity_live_snapshot(state);
    schedule_proxy_capture_follow_up_worker(state, &summary_invoke_id).await?;

    Ok(())
}

pub(crate) fn pool_routing_reservation_key_for_invoke_id(invoke_id: &str) -> Option<String> {
    let request_id = invoke_id
        .strip_prefix("proxy-")
        .or_else(|| invoke_id.strip_prefix("pool-ws-"))?
        .split('-')
        .next()?;
    request_id
        .parse::<u64>()
        .ok()
        .map(build_pool_routing_reservation_key)
}

pub(crate) async fn observe_proxy_cache_hit_if_success(
    state: &AppState,
    record: &ProxyCaptureRecord,
) -> Result<ModelRouteCacheObservationOutcome> {
    if record.status != "success" {
        return Ok(ModelRouteCacheObservationOutcome::default());
    }
    let metadata = terminal_payload_metadata(record.payload.as_deref());
    let Some(account_id) = metadata.upstream_account_id else {
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    let model = metadata
        .request_model
        .as_deref()
        .or(record.model.as_deref())
        .map(str::trim);
    let Some(model) = model else {
        return Ok(ModelRouteCacheObservationOutcome::default());
    };
    let reservation_held = pool_routing_reservation_key_for_invoke_id(&record.invoke_id)
        .is_some_and(|reservation_key| {
            pool_routing_reservation_matches_model(state, &reservation_key, account_id, Some(model))
        });
    let active_concurrency = pool_routing_model_reservation_count(state, account_id, Some(model))
        + if reservation_held { 0 } else { 1 };
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        record.usage.input_tokens,
        record.usage.cache_input_tokens,
        active_concurrency,
    )
    .await
}

pub(crate) fn pool_route_orphan_recovery_failure_message(recovery_trigger: &str) -> String {
    format!("pool request was interrupted before completion and recovered via {recovery_trigger}")
}

pub(crate) async fn clean_up_pool_route_after_orphan_recovery(
    state: &AppState,
    invoke_id: &str,
    sticky_key: Option<&str>,
    upstream_account_id: Option<i64>,
    recovery_trigger: &'static str,
    record_route_failure: bool,
) {
    let reservation_key = pool_routing_reservation_key_for_invoke_id(invoke_id);
    let mut reservation_released_after_failure = false;
    if record_route_failure && let Some(account_id) = upstream_account_id {
        let error_message = pool_route_orphan_recovery_failure_message(recovery_trigger);
        let result = if let Some(reservation_key) = reservation_key.as_deref() {
            reservation_released_after_failure = true;
            persist_pool_route_failure_then_release(
                state,
                reservation_key,
                record_pool_route_transport_failure(
                    &state.pool,
                    account_id,
                    sticky_key,
                    &error_message,
                    Some(invoke_id),
                ),
            )
            .await
        } else {
            record_pool_route_transport_failure(
                &state.pool,
                account_id,
                sticky_key,
                &error_message,
                Some(invoke_id),
            )
            .await
        };
        if let Err(err) = result {
            warn!(
                invoke_id,
                account_id,
                recovery_trigger,
                error = %err,
                "failed to record pool route transport failure during orphan recovery cleanup"
            );
        }
    }

    if !reservation_released_after_failure && let Some(reservation_key) = reservation_key {
        release_pool_routing_reservation(state, &reservation_key);
    }
}

pub(crate) async fn should_record_route_failure_after_attempt_recovery(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    recovered_invocation: bool,
) -> bool {
    if state
        .proxy_runtime_invocations
        .contains_terminal(invoke_id, occurred_at)
    {
        debug!(
            invoke_id,
            occurred_at,
            "skipping route failure cleanup because terminal runtime overlay already exists"
        );
        return false;
    }

    if recovered_invocation {
        return true;
    }

    let latest_status = sqlx::query_scalar::<_, String>(
        r#"
        SELECT status
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_optional(&state.pool)
    .await;

    match latest_status {
        Ok(Some(status)) => matches!(
            status.as_str(),
            INVOCATION_STATUS_RUNNING | INVOCATION_STATUS_PENDING
        ),
        Ok(None) => true,
        Err(err) => {
            warn!(
                invoke_id,
                occurred_at,
                error = %err,
                "failed to inspect invocation terminal state during pool orphan cleanup"
            );
            true
        }
    }
}

#[cfg(test)]
pub(crate) async fn should_record_route_failure_after_attempt_recovery_for_test(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    recovered_invocation: bool,
) -> bool {
    should_record_route_failure_after_attempt_recovery(
        state,
        invoke_id,
        occurred_at,
        recovered_invocation,
    )
    .await
}

pub(crate) async fn clean_up_recovered_pool_routes(
    state: &AppState,
    recovered_attempts: &[RecoveredPoolAttemptRow],
    recovered_invocations: &[RecoveredInvocationRow],
    recovery_trigger: &'static str,
) {
    let recovered_invocation_keys = recovered_invocations
        .iter()
        .map(|row| (row.invoke_id.as_str(), row.occurred_at.as_str()))
        .collect::<BTreeSet<_>>();
    for row in recovered_attempts {
        let recovered_invocation =
            recovered_invocation_keys.contains(&(row.invoke_id.as_str(), row.occurred_at.as_str()));
        let record_route_failure = should_record_route_failure_after_attempt_recovery(
            state,
            &row.invoke_id,
            &row.occurred_at,
            recovered_invocation,
        )
        .await;
        clean_up_pool_route_after_orphan_recovery(
            state,
            &row.invoke_id,
            row.sticky_key.as_deref(),
            row.upstream_account_id,
            recovery_trigger,
            record_route_failure,
        )
        .await;
    }
}

pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan(
    state: &AppState,
    pending_attempt_record: PendingPoolAttemptRecord,
    first_byte_observed: bool,
    terminal_outcome_observed: bool,
) -> Result<()> {
    state.sqlite_batch_writer.flush_now(&state.pool).await?;

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
        let mut tx = state.pool.begin().await?;
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

pub(crate) async fn recover_guard_dropped_pool_invocation_orphan(
    state: &AppState,
    selector: InvocationRecoverySelector,
    recovery_trigger: &'static str,
) -> Result<()> {
    state.sqlite_batch_writer.flush_now(&state.pool).await?;

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

pub(crate) async fn recover_guard_dropped_pool_terminal_invocation_orphan(
    state: &AppState,
    selector: InvocationRecoverySelector,
) -> Result<()> {
    recover_guard_dropped_pool_invocation_orphan(state, selector, "terminal_invocation_drop_guard")
        .await
}

pub(crate) async fn recover_stale_pool_early_phase_orphans_runtime(
    state: &AppState,
) -> Result<PoolOrphanRecoveryOutcome> {
    state.sqlite_batch_writer.flush_now(&state.pool).await?;

    let timeouts = resolve_pool_routing_timeouts(&state.pool, &state.config).await?;
    let responses_started_before = stale_started_before_string(
        timeouts.responses_first_byte_timeout,
        POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE,
    );
    let compact_started_before = stale_started_before_string(
        timeouts.compact_first_byte_timeout,
        POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE,
    );
    let default_started_before = stale_started_before_string(
        timeouts.default_first_byte_timeout,
        POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE,
    );
    let active_attempt_ids = state
        .pool_live_attempt_ids
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .clone();
    let recovery = {
        let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
            .await;
        let dashboard_reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
        let _dashboard_reconcile_guard = dashboard_reconcile_gate.lock().await;
        let mut tx = state.pool.begin().await?;
        let stale_candidates = load_stale_pool_upstream_request_attempt_candidate_rows_tx(
            tx.as_mut(),
            &responses_started_before,
            &compact_started_before,
            &default_started_before,
        )
        .await?;
        let candidate_ids = stale_candidates
            .into_iter()
            .filter(|row| !active_attempt_ids.contains(&row.id))
            .map(|row| row.id)
            .collect::<Vec<_>>();
        let finished_at = shanghai_now_string();
        let recovered_attempts = recover_stale_pool_upstream_request_attempt_candidates_tx(
            tx.as_mut(),
            &candidate_ids,
            finished_at.as_str(),
            &responses_started_before,
            &compact_started_before,
            &default_started_before,
        )
        .await?;
        if recovered_attempts.is_empty() {
            tx.commit().await?;
            None
        } else {
            let selectors: Vec<_> = recovered_attempts
                .iter()
                .map(|row| {
                    InvocationRecoverySelector::new(row.invoke_id.clone(), row.occurred_at.clone())
                })
                .collect();
            let recovered_invocations = recover_proxy_invocations_with_scope_tx(
                tx.as_mut(),
                ProxyInvocationRecoveryScope::Selectors(&selectors),
            )
            .await?;
            tx.commit().await?;
            Some((recovered_attempts, recovered_invocations))
        }
    };
    let Some((recovered_attempts, recovered_invocations)) = recovery else {
        return Ok(PoolOrphanRecoveryOutcome::default());
    };

    clean_up_recovered_pool_routes(
        state,
        &recovered_attempts,
        &recovered_invocations,
        "runtime_sweeper",
    )
    .await;

    for invoke_id in recovered_attempts
        .iter()
        .map(|row| row.invoke_id.as_str())
        .collect::<BTreeSet<_>>()
    {
        if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, invoke_id).await {
            warn!(
                invoke_id,
                error = %err,
                "failed to broadcast stale pool orphan recovery snapshot"
            );
        }
    }
    broadcast_recovered_proxy_invocations(state, &recovered_invocations).await?;

    let outcome = PoolOrphanRecoveryOutcome {
        recovered_attempts: recovered_attempts.len(),
        recovered_invocations: recovered_invocations.len(),
    };
    info!(
        recovered_attempts = outcome.recovered_attempts,
        recovered_invocations = outcome.recovered_invocations,
        recovery_trigger = "runtime_sweeper",
        "recovered stale pool early-phase orphans at runtime"
    );

    Ok(outcome)
}

pub(crate) async fn broadcast_pool_upstream_attempts_snapshot(
    state: &AppState,
    invoke_id: &str,
) -> Result<()> {
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::AttemptChanged {
            invoke_id: invoke_id.to_string(),
        });
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
    let has_account_attempt_topic = state
        .subscription_hub
        .has_active_topic_name("upstream-account-attempts.window")
        .await;
    let has_external_broadcaster_receiver = state
        .subscription_hub
        .has_external_broadcaster_receiver(state.broadcaster.receiver_count());
    if has_account_attempt_topic || has_external_broadcaster_receiver {
        let attempts = match query_pool_attempt_records_from_live(&state.pool, invoke_id).await {
            Ok(attempts) => attempts,
            Err(err) => {
                // A failed source query cannot identify the affected account. Preserve the
                // producer error while allowing active account topics to retain last-good and
                // enter their bounded dedicated recovery path.
                let _ = state
                    .broadcaster
                    .send(BroadcastPayload::PoolAttemptsSnapshotUnavailable {
                        invoke_id: invoke_id.to_string(),
                    });
                return Err(anyhow!("failed to load pool attempt snapshot: {err:?}"));
            }
        };
        let _ = state.broadcaster.send(BroadcastPayload::PoolAttempts {
            invoke_id: invoke_id.to_string(),
            attempts,
        });
    }
    Ok(())
}

pub(crate) async fn broadcast_pool_attempt_started_runtime_snapshot(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    runtime_snapshot: &PoolAttemptRuntimeSnapshotContext,
    account: &PoolResolvedAccount,
    attempt_count: usize,
    distinct_account_count: usize,
    request_compression_algorithm: Option<&str>,
) {
    let mut running_record = build_running_proxy_capture_record(
        &trace.invoke_id,
        &trace.occurred_at,
        runtime_snapshot.capture_target,
        &runtime_snapshot.request_info,
        trace.requester_ip.as_deref(),
        trace.sticky_key.as_deref(),
        runtime_snapshot.prompt_cache_key.as_deref(),
        true,
        Some(account.account_id),
        Some(account.display_name.as_str()),
        payload_summary_upstream_account_kind(Some(account)),
        payload_summary_upstream_base_url_host(Some(account)),
        None,
        Some(attempt_count),
        Some(distinct_account_count),
        None,
        None,
        runtime_snapshot.t_req_read_ms,
        runtime_snapshot.t_req_parse_ms,
        0.0,
        0.0,
    );
    set_proxy_capture_record_request_compression_algorithm(
        &mut running_record,
        request_compression_algorithm,
    );
    if let Err(err) =
        persist_and_broadcast_proxy_capture_runtime_snapshot(state, running_record).await
    {
        warn!(
            ?err,
            invoke_id = %trace.invoke_id,
            "failed to broadcast pool attempt start runtime snapshot"
        );
    }
    if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await {
        warn!(
            invoke_id = %trace.invoke_id,
            error = %err,
            "failed to broadcast pool attempt start snapshot"
        );
    }
}
