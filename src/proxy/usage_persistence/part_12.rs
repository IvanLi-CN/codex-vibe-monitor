pub(crate) fn persisted_invocation_allows_proxy_record_update(
    existing_status: Option<&str>,
    existing_failure_kind: Option<&str>,
    incoming_status: &str,
) -> bool {
    invocation_status_is_in_flight(existing_status)
        || (!invocation_status_is_in_flight(Some(incoming_status))
            && invocation_status_is_recoverable_proxy_interrupted(
                existing_status,
                existing_failure_kind,
            ))
}

pub(crate) async fn load_persisted_api_invocation_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    let mut record =
        sqlx::query_as::<_, ApiInvocation>(include_str!("load_persisted_api_invocation.sql"))
            .bind(invoke_id)
            .bind(occurred_at)
            .fetch_one(&mut *tx)
            .await?;
    hydrate_api_invocation_blocked_binding(&mut record);
    Ok(record)
}
pub(crate) async fn touch_invocation_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    payload: Option<&str>,
) -> Result<()> {
    touch_upstream_account_last_activity_tx(
        tx,
        occurred_at,
        upstream_account_id_from_payload(payload),
    )
    .await
}

pub(crate) async fn touch_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    upstream_account_id: Option<i64>,
) -> Result<()> {
    if let Some(upstream_account_id) = upstream_account_id {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = CASE
                WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                ELSE last_activity_at
            END
            WHERE id = ?2
            "#,
        )
        .bind(occurred_at)
        .bind(upstream_account_id)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn persist_and_broadcast_proxy_capture_runtime_snapshot(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let started = Instant::now();
    let persisted_record = api_invocation_from_runtime_record(&record);
    let invoke_id = persisted_record.invoke_id.clone();
    let occurred_at = persisted_record.occurred_at.clone();
    let store_outcome = state
        .proxy_runtime_invocations
        .upsert(persisted_record.clone());
    if store_outcome.skipped_terminal {
        let elapsed_ms = started.elapsed().as_millis() as u64;
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %occurred_at,
            elapsed_ms,
            runtime_store_running_count = store_outcome.running_count,
            runtime_store_pruned_count = store_outcome.pruned_count,
            running_snapshot_db_write_skipped = true,
            running_snapshot_skipped_after_terminal = true,
            "stale running proxy capture snapshot skipped after terminal persistence"
        );
        return Ok(());
    }
    state
        .dashboard_network_speed_cache
        .observe_dashboard_activity_runtime_snapshot(&persisted_record, Utc::now());
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &persisted_record,
            RuntimeMutationKind::RuntimeUpsert,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &persisted_record);
    schedule_dashboard_activity_live_snapshot(state);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    debug!(
        invoke_id = %invoke_id,
        occurred_at = %occurred_at,
        elapsed_ms,
        runtime_store_running_count = store_outcome.running_count,
        runtime_store_pruned_count = store_outcome.pruned_count,
        running_snapshot_db_write_skipped = true,
        running_snapshot_recovery_placeholder_enqueued = false,
        "running proxy capture snapshot stored in memory and broadcast"
    );

    Ok(())
}

pub(crate) fn broadcast_proxy_capture_first_token_runtime_snapshot(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    first_token_ms: f64,
) {
    if !first_token_ms.is_finite() || first_token_ms < 0.0 {
        return;
    }
    let Some(mut record) = state
        .proxy_runtime_invocations
        .snapshot()
        .into_iter()
        .find(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at)
    else {
        return;
    };
    if record.first_token_ms.is_some() {
        return;
    }
    record.first_token_ms = Some(first_token_ms);
    let outcome = state.proxy_runtime_invocations.upsert(record.clone());
    if outcome.skipped_terminal {
        return;
    }
    state
        .dashboard_network_speed_cache
        .observe_dashboard_activity_runtime_snapshot(&record, Utc::now());
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::LifecyclePhase,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    schedule_dashboard_activity_live_snapshot(state);
}

pub(crate) fn remove_proxy_runtime_snapshot_for_terminal(
    state: &AppState,
    record: &ApiInvocation,
) -> bool {
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(&record.invoke_id, &record.occurred_at);
    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id = %record.invoke_id,
        occurred_at = %record.occurred_at,
        terminal_overlay_emitted = true,
        terminal_removed_runtime_snapshot = remove_outcome.removed,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        "terminal proxy capture record stored in memory runtime overlay"
    );
    remove_outcome.already_terminal
}

pub(crate) fn remove_proxy_runtime_snapshot_by_key(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    reason: &'static str,
) -> bool {
    state
        .dashboard_network_speed_cache
        .drop_dashboard_activity_invocation(invoke_id, occurred_at);
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);
    let removed_runtime_snapshot = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at);
    if let Some(record) = &removed_runtime_snapshot {
        state
            .subscription_hub
            .publish_runtime_mutation(RuntimeMutation::invocation(
                record,
                RuntimeMutationKind::RuntimeRemoved,
            ));
    }
    debug!(
        invoke_id,
        occurred_at,
        reason,
        terminal_removed_runtime_snapshot = removed_runtime_snapshot.is_some(),
        terminal_already_tombstoned = false,
        "non-terminal proxy runtime snapshot removed by key"
    );
    removed_runtime_snapshot.is_some()
}

pub(crate) fn terminalize_proxy_runtime_snapshot_by_key(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    reason: &'static str,
) -> bool {
    let Some(mut record) = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at)
    else {
        debug!(
            invoke_id,
            occurred_at,
            reason,
            terminal_removed_runtime_snapshot = false,
            terminal_already_tombstoned = false,
            "no non-terminal proxy runtime snapshot found for terminal cleanup"
        );
        return false;
    };

    record.status = Some(INVOCATION_STATUS_INTERRUPTED.to_string());
    record.error_message = Some(format!(
        "[{PROXY_FAILURE_INVOCATION_INTERRUPTED}] proxy request ended before a terminal record was written"
    ));
    record.failure_kind = Some(PROXY_FAILURE_INVOCATION_INTERRUPTED.to_string());
    record.failure_class = Some(FAILURE_CLASS_SERVICE.to_string());
    record.is_actionable = Some(true);
    record.pool_attempt_terminal_reason = Some(PROXY_FAILURE_INVOCATION_INTERRUPTED.to_string());
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(&record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);

    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id,
        occurred_at,
        reason,
        terminal_removed_runtime_snapshot = true,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        terminal_delta_skipped_runtime_only = true,
        "non-terminal proxy runtime snapshot terminalized by key"
    );
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::TerminalCommitted,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    true
}

pub(crate) fn terminalize_proxy_runtime_snapshot_with_error(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    status: StatusCode,
    failure_kind: &'static str,
    error_message: &str,
    reason: &'static str,
) -> bool {
    let Some(mut record) = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at)
    else {
        debug!(
            invoke_id,
            occurred_at,
            reason,
            terminal_overlay_emitted = false,
            terminal_removed_runtime_snapshot = false,
            "no non-terminal proxy runtime snapshot found for terminal error overlay"
        );
        return false;
    };

    record.status = Some(if status.is_server_error() {
        format!("http_{}", status.as_u16())
    } else {
        "failed".to_string()
    });
    record.error_message = Some(format!("[{failure_kind}] {error_message}"));
    record.failure_kind = Some(failure_kind.to_string());
    record.failure_class = Some(
        if status.is_client_error() {
            FAILURE_CLASS_CLIENT
        } else {
            FAILURE_CLASS_SERVICE
        }
        .to_string(),
    );
    record.is_actionable = Some(true);
    record.pool_attempt_terminal_reason = Some(failure_kind.to_string());
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(&record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);

    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id,
        occurred_at,
        reason,
        status = %status,
        failure_kind,
        terminal_overlay_emitted = true,
        terminal_removed_runtime_snapshot = true,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        terminal_delta_skipped_runtime_only = true,
        "non-terminal proxy runtime snapshot terminalized with error overlay"
    );
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::TerminalCommitted,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    true
}

pub(crate) async fn observe_successful_proxy_capture_model_route_cache(
    state: &AppState,
    record: &ProxyCaptureRecord,
) {
    if record.status != "success" {
        return;
    }

    let metadata = terminal_payload_metadata(record.payload.as_deref());
    match observe_proxy_cache_hit_if_success(state, record).await {
        Ok(outcome) => {
            if outcome.observed {
                state
                    .subscription_hub
                    .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
            }
            if outcome.availability_increased {
                let account_allows_publish = match metadata.upstream_account_id {
                    Some(account_id) => match pool_account_allows_model_route_availability_publish(
                        &state.pool,
                        account_id,
                    )
                    .await
                    {
                        Ok(allowed) => allowed,
                        Err(err) => {
                            warn!(
                                invoke_id = %record.invoke_id,
                                account_id,
                                error = %err,
                                "failed to verify account fence before publishing model route availability"
                            );
                            false
                        }
                    },
                    None => false,
                };
                if account_allows_publish {
                    publish_pool_routing_availability(state);
                } else {
                    debug!(
                        invoke_id = %record.invoke_id,
                        upstream_account_id = metadata.upstream_account_id,
                        "model cache observation increased capacity without publishing because the account remains fenced"
                    );
                }
            }
        }
        Err(err) => {
            warn!(
                invoke_id = %record.invoke_id,
                error = %err,
                "failed to observe model route cache hit"
            );
        }
    }
}

pub(crate) async fn persist_and_broadcast_proxy_capture_terminal_record(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let enqueue_started = Instant::now();
    let persisted_record = api_invocation_from_runtime_record(&record);
    let invoke_id = persisted_record.invoke_id.clone();
    let duplicate_terminal = remove_proxy_runtime_snapshot_for_terminal(state, &persisted_record);
    if duplicate_terminal {
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %persisted_record.occurred_at,
            business_unblocked_record_write = true,
            "duplicate terminal proxy capture record skipped before sqlite enqueue"
        );
        schedule_proxy_capture_follow_up_after_terminal_enqueue(
            state,
            &invoke_id,
            "duplicate_runtime_terminal",
        );
        return Ok(());
    }
    observe_successful_proxy_capture_model_route_cache(state, &record).await;
    let projection = register_terminal_projection_before_enqueue(state, &persisted_record).await;
    let delta = &projection.dashboard;
    let startup_backfill_tasks = startup_backfill_tasks_for_terminal(&persisted_record);
    debug!(
        invoke_id = %invoke_id,
        terminal_delta_applied_selection_count = delta.applied_selection_count,
        terminal_delta_duplicate = delta.duplicate,
        terminal_delta_skipped_out_of_range_count = delta.skipped_out_of_range_count,
        response_source = "memory",
        "registered terminal record in dashboard activity read model before sqlite enqueue"
    );
    let terminal_enqueue =
        state
            .sqlite_batch_writer
            .enqueue_terminal(BatchedTerminalInvocationWrite {
                record,
                capture_started: None,
                raw_capture: false,
                dashboard_terminal_sequence: delta.terminal_sequence,
                terminal_projection_event_ids: projection.event_id.into_iter().collect(),
                startup_backfill_tasks,
            });
    let terminal_enqueued = terminal_enqueue.enqueued;
    if !terminal_enqueued {
        rollback_terminal_projection_before_enqueue(state, &persisted_record, &projection).await;
        let terminal_tombstone_cleared = state
            .proxy_runtime_invocations
            .clear_terminal_tombstone(&persisted_record.invoke_id, &persisted_record.occurred_at);
        warn!(
            invoke_id = %invoke_id,
            occurred_at = %persisted_record.occurred_at,
            enqueue_failed_by_class = "terminal_invocation",
            terminal_tombstone_cleared,
            durability_mode = terminal_enqueue.durability_mode.as_str(),
            journal_sequence = ?terminal_enqueue.journal_sequence,
            business_unblocked_record_write = true,
            record_flush_deferred_or_failed = "terminal_invocation_enqueue_failed",
            "terminal proxy capture record dropped by sqlite write controller"
        );
    } else {
        debug!(
            invoke_id = %invoke_id,
            terminal_record_enqueue_elapsed = enqueue_started.elapsed().as_millis() as u64,
            durability_mode = terminal_enqueue.durability_mode.as_str(),
            journal_sequence = ?terminal_enqueue.journal_sequence,
            journal_pending_records = terminal_enqueue.journal_pending_records,
            journal_pending_bytes = terminal_enqueue.journal_pending_bytes,
            business_unblocked_record_write = true,
            record_flush_deferred_or_failed = "terminal_invocation_enqueued_async",
            "terminal proxy capture record queued for sqlite write controller"
        );
    }
    #[cfg(test)]
    if terminal_enqueued && state.sqlite_batch_writer.auto_flush_terminal_for_test() {
        state
            .sqlite_batch_writer
            .flush_buffered_for_test(&state.pool)
            .await;
    }
    if terminal_enqueued {
        state
            .subscription_hub
            .publish_runtime_mutation(RuntimeMutation::invocation(
                &persisted_record,
                RuntimeMutationKind::TerminalCommitted,
            ));
        #[cfg(test)]
        broadcast_test_record_payload(state, &persisted_record);
        schedule_dashboard_activity_live_snapshot(state);
        schedule_proxy_capture_follow_up_after_terminal_enqueue(
            state,
            &invoke_id,
            "runtime_terminal",
        );
    }
    Ok(())
}

pub(crate) async fn persist_proxy_capture_runtime_record(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    persist_proxy_capture_runtime_record_core(pool, record, true).await
}

pub(crate) async fn persist_proxy_capture_runtime_record_core(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
    write_derived_inline: bool,
) -> Result<Option<ApiInvocation>> {
    let mut tx = pool.begin().await?;
    let persisted =
        persist_proxy_capture_runtime_record_tx(tx.as_mut(), record, write_derived_inline).await?;
    tx.commit().await?;
    Ok(persisted)
}

struct ProxyRuntimeRecordPersistence {
    record: ProxyCaptureRecord,
    raw_response: String,
    resp_raw: RawPayloadMeta,
    failure: FailureClassification,
    failure_kind: Option<String>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    first_token_ms: Option<f64>,
    core_write_started: Instant,
    created_at: String,
}

fn prepare_proxy_runtime_record_persistence(
    record: ProxyCaptureRecord,
) -> ProxyRuntimeRecordPersistence {
    let raw_response = if record.response_body_preview_enabled {
        record.raw_response.clone()
    } else {
        String::new()
    };
    let resp_raw = if record.response_body_preview_enabled {
        record.resp_raw.clone()
    } else {
        RawPayloadMeta {
            path: None,
            size_bytes: record.resp_raw.size_bytes,
            truncated: record.resp_raw.truncated,
            truncated_reason: record.resp_raw.truncated_reason.clone(),
        }
    };
    let failure = resolve_failure_classification(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        None,
        None,
    );
    let failure_kind = failure.failure_kind.clone();
    let t_req_read_ms = nullable_runtime_timing_value(record.timings.t_req_read_ms);
    let t_req_parse_ms = nullable_runtime_timing_value(record.timings.t_req_parse_ms);
    let t_upstream_connect_ms = nullable_runtime_timing_value(record.timings.t_upstream_connect_ms);
    let t_upstream_ttfb_ms = nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms);
    let first_token_ms = record
        .timings
        .first_token_ms
        .filter(|value| value.is_finite() && *value >= 0.0);
    ProxyRuntimeRecordPersistence {
        record,
        raw_response,
        resp_raw,
        failure,
        failure_kind,
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        t_upstream_ttfb_ms,
        first_token_ms,
        core_write_started: Instant::now(),
        created_at: format_utc_iso_millis(Utc::now()),
    }
}

async fn insert_proxy_capture_runtime_record(
    tx: &mut SqliteConnection,
    context: &ProxyRuntimeRecordPersistence,
) -> Result<Option<&'static str>> {
    let ProxyRuntimeRecordPersistence {
        record,
        raw_response,
        resp_raw,
        failure,
        failure_kind,
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        t_upstream_ttfb_ms,
        first_token_ms,
        core_write_started: _,
        created_at,
    } = context;

    let insert_result = sqlx::query(include_str!("insert_proxy_runtime_record.sql"))
        .bind(&record.invoke_id)
        .bind(&record.occurred_at)
        .bind(SOURCE_PROXY)
        .bind(&record.model)
        .bind(record.usage.input_tokens)
        .bind(record.usage.output_tokens)
        .bind(record.usage.cache_input_tokens)
        .bind(record.usage.reasoning_tokens)
        .bind(record.usage.total_tokens)
        .bind(record.cost)
        .bind(record.cost_breakdown.map(|value| value.input))
        .bind(record.cost_breakdown.map(|value| value.cache_write))
        .bind(record.cost_breakdown.map(|value| value.cache_read))
        .bind(record.cost_breakdown.map(|value| value.output))
        .bind(record.cost_breakdown.map(|value| value.reasoning))
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(raw_payload_meta_codec(&record.req_raw))
        .bind(record.req_raw.size_bytes)
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(resp_raw.path.as_deref())
        .bind(raw_payload_meta_codec(resp_raw))
        .bind(resp_raw.size_bytes)
        .bind(resp_raw.truncated as i64)
        .bind(resp_raw.truncated_reason.as_deref())
        .bind(None::<f64>)
        .bind(t_req_read_ms)
        .bind(t_req_parse_ms)
        .bind(t_upstream_connect_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(first_token_ms)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
    if insert_result.rows_affected() == 0 {
        return recover_proxy_capture_insert_race(tx, context).await;
    }
    Ok(Some("insert_missing"))
}

async fn recover_proxy_capture_insert_race(
    tx: &mut SqliteConnection,
    context: &ProxyRuntimeRecordPersistence,
) -> Result<Option<&'static str>> {
    let ProxyRuntimeRecordPersistence {
        record,
        raw_response,
        resp_raw,
        failure,
        failure_kind,
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        t_upstream_ttfb_ms,
        first_token_ms,
        ..
    } = context;
    let Some(existing) =
        load_persisted_invocation_identity_tx(&mut *tx, &record.invoke_id, &record.occurred_at)
            .await?
    else {
        return Ok(None);
    };
    if !persisted_invocation_allows_proxy_record_update(
        existing.status.as_deref(),
        existing.failure_kind.as_deref(),
        &record.status,
    ) {
        return Ok(None);
    }
    let updated = update_existing_proxy_invocation_record_tx(
        &mut *tx,
        ProxyInvocationUpdateRequest {
            id: existing.id,
            record,
            raw_response,
            resp_raw,
            failure_kind: failure_kind.as_deref(),
            failure_class: failure.failure_class.as_str(),
            is_actionable: failure.is_actionable,
            t_total_ms: None,
            t_req_read_ms: *t_req_read_ms,
            t_req_parse_ms: *t_req_parse_ms,
            t_upstream_connect_ms: *t_upstream_connect_ms,
            t_upstream_ttfb_ms: *t_upstream_ttfb_ms,
            first_token_ms: *first_token_ms,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
        },
    )
    .await?;
    if updated {
        Ok(Some("update_race"))
    } else {
        Ok(None)
    }
}

async fn persist_proxy_capture_core_write(
    tx: &mut SqliteConnection,
    context: &ProxyRuntimeRecordPersistence,
) -> Result<Option<&'static str>> {
    let ProxyRuntimeRecordPersistence {
        record,
        raw_response,
        resp_raw,
        failure,
        failure_kind,
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        t_upstream_ttfb_ms,
        first_token_ms,
        core_write_started: _,
        created_at: _,
    } = context;
    let existing_identity =
        load_persisted_invocation_identity_tx(&mut *tx, &record.invoke_id, &record.occurred_at)
            .await?;
    if let Some(existing) = existing_identity.as_ref()
        && !persisted_invocation_allows_proxy_record_update(
            existing.status.as_deref(),
            existing.failure_kind.as_deref(),
            &record.status,
        )
    {
        return Ok(None);
    }

    let core_write_path = if let Some(existing) = existing_identity.as_ref() {
        let updated = update_existing_proxy_invocation_record_tx(
            &mut *tx,
            ProxyInvocationUpdateRequest {
                id: existing.id,
                record,
                raw_response,
                resp_raw,
                failure_kind: failure_kind.as_deref(),
                failure_class: failure.failure_class.as_str(),
                is_actionable: failure.is_actionable,
                t_total_ms: None,
                t_req_read_ms: *t_req_read_ms,
                t_req_parse_ms: *t_req_parse_ms,
                t_upstream_connect_ms: *t_upstream_connect_ms,
                t_upstream_ttfb_ms: *t_upstream_ttfb_ms,
                first_token_ms: *first_token_ms,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            },
        )
        .await?;
        if !updated {
            return Ok(None);
        }
        "update_existing"
    } else {
        let Some(insert_path) = insert_proxy_capture_runtime_record(tx, context).await? else {
            return Ok(None);
        };
        insert_path
    };
    Ok(Some(core_write_path))
}

async fn persist_proxy_capture_runtime_derived_tx(
    tx: &mut SqliteConnection,
    persistence: &ProxyRuntimeRecordPersistence,
    persisted_id: i64,
) -> Result<()> {
    let ProxyRuntimeRecordPersistence {
        record,
        failure,
        failure_kind,
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        t_upstream_ttfb_ms,
        first_token_ms,
        ..
    } = persistence;
    upsert_invocation_hourly_rollups_tx(
        &mut *tx,
        &[InvocationHourlySourceRecord {
            id: persisted_id,
            occurred_at: record.occurred_at.clone(),
            source: SOURCE_PROXY.to_string(),
            status: Some(record.status.clone()),
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            model: record.model.clone(),
            input_tokens: record.usage.input_tokens,
            output_tokens: record.usage.output_tokens,
            cache_input_tokens: record.usage.cache_input_tokens,
            reasoning_tokens: record.usage.reasoning_tokens,
            total_tokens: record.usage.total_tokens,
            cost: record.cost,
            upstream_account_id: crate::proxy::upstream_account_id_from_payload(
                record.payload.as_deref(),
            ),
            cost_input: record.cost_breakdown.map(|value| value.input),
            cost_cache_write: record.cost_breakdown.map(|value| value.cache_write),
            cost_cache_read: record.cost_breakdown.map(|value| value.cache_read),
            cost_output: record.cost_breakdown.map(|value| value.output),
            cost_reasoning: record.cost_breakdown.map(|value| value.reasoning),
            error_message: record.error_message.clone(),
            failure_kind: failure_kind.clone(),
            failure_class: Some(failure.failure_class.as_str().to_string()),
            is_actionable: Some(failure.is_actionable as i64),
            payload: record.payload.clone(),
            t_total_ms: None,
            t_req_read_ms: *t_req_read_ms,
            t_req_parse_ms: *t_req_parse_ms,
            t_upstream_connect_ms: *t_upstream_connect_ms,
            t_upstream_ttfb_ms: *t_upstream_ttfb_ms,
            first_token_ms: *first_token_ms,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
        }],
        &INVOCATION_HOURLY_ROLLUP_TARGETS,
    )
    .await?;
    save_hourly_rollup_live_progress_tx(&mut *tx, HOURLY_ROLLUP_DATASET_INVOCATIONS, persisted_id)
        .await?;
    touch_invocation_upstream_account_last_activity_tx(
        &mut *tx,
        &record.occurred_at,
        record.payload.as_deref(),
    )
    .await
}

pub(crate) async fn persist_proxy_capture_runtime_record_tx(
    tx: &mut SqliteConnection,
    record: ProxyCaptureRecord,
    write_derived_inline: bool,
) -> Result<Option<ApiInvocation>> {
    let persistence = prepare_proxy_runtime_record_persistence(record);
    let Some(core_write_path) = persist_proxy_capture_core_write(tx, &persistence).await? else {
        return Ok(None);
    };
    let ProxyRuntimeRecordPersistence {
        record,
        raw_response: _,
        resp_raw,
        failure: _,
        failure_kind: _,
        t_req_read_ms: _,
        t_req_parse_ms: _,
        t_upstream_connect_ms: _,
        t_upstream_ttfb_ms: _,
        first_token_ms: _,
        core_write_started,
        created_at: _,
    } = &persistence;
    let persisted_identity =
        load_persisted_invocation_identity_tx(&mut *tx, &record.invoke_id, &record.occurred_at)
            .await?
            .ok_or_else(|| {
                anyhow!("persisted proxy runtime invocation row disappeared after upsert")
            })?;
    if write_derived_inline {
        persist_proxy_capture_runtime_derived_tx(&mut *tx, &persistence, persisted_identity.id)
            .await?;
    }

    let persisted =
        load_persisted_api_invocation_tx(&mut *tx, &record.invoke_id, &record.occurred_at).await?;

    let core_write_elapsed_ms = core_write_started.elapsed().as_millis() as u64;
    if core_write_elapsed_ms >= 1_000 {
        warn!(
            invoke_id = %record.invoke_id,
            status = %record.status,
            core_write_path,
            request_raw_bytes = record.req_raw.size_bytes,
            response_raw_bytes = resp_raw.size_bytes,
            has_request_raw_path = record.req_raw.path.is_some(),
            has_response_raw_path = resp_raw.path.is_some(),
            elapsed_ms = core_write_elapsed_ms,
            "proxy capture core invocation write was slow"
        );
    } else {
        debug!(
            invoke_id = %record.invoke_id,
            status = %record.status,
            core_write_path,
            request_raw_bytes = record.req_raw.size_bytes,
            response_raw_bytes = resp_raw.size_bytes,
            has_request_raw_path = record.req_raw.path.is_some(),
            has_response_raw_path = resp_raw.path.is_some(),
            elapsed_ms = core_write_elapsed_ms,
            "proxy capture core invocation write completed"
        );
    }

    Ok(Some(persisted))
}
