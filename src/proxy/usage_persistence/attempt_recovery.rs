use super::*;

pub(crate) enum PoolAttemptRecoveryScope<'a> {
    AllPending,
    Ids(&'a [i64]),
    SpecificEarlyPhase {
        attempt_id: i64,
    },
    StaleEarlyPhase {
        responses_started_before: &'a str,
        compact_started_before: &'a str,
        default_started_before: &'a str,
    },
}

pub(crate) async fn recover_pool_upstream_request_attempts_with_scope(
    pool: &Pool<Sqlite>,
    scope: PoolAttemptRecoveryScope<'_>,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let mut tx = crate::observability::diagnostics::wait(
        crate::observability::diagnostics::Resource::DbPool,
        pool.begin(),
    )
    .await?;
    let recovered =
        recover_pool_upstream_request_attempts_with_scope_tx(tx.as_mut(), scope).await?;
    tx.commit().await?;
    Ok(recovered)
}

pub(crate) async fn recover_pool_upstream_request_attempts_with_scope_tx(
    tx: &mut SqliteConnection,
    scope: PoolAttemptRecoveryScope<'_>,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let finished_at = shanghai_now_string();
    let recovered = match scope {
        PoolAttemptRecoveryScope::AllPending => {
            sqlx::query_as::<_, RecoveredPoolAttemptRow>(
                r#"
                UPDATE pool_upstream_request_attempts
                SET
                    finished_at = COALESCE(finished_at, ?1),
                    status = ?2,
                    phase = ?3,
                    failure_kind = COALESCE(failure_kind, ?4),
                    error_message = COALESCE(error_message, ?5)
                WHERE status = ?6
                  AND finished_at IS NULL
                RETURNING id, invoke_id, occurred_at, sticky_key, upstream_account_id
                "#,
            )
            .bind(finished_at)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
            .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
            .bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
            .fetch_all(&mut *tx)
            .await?
        }
        PoolAttemptRecoveryScope::Ids(ids) => {
            if ids.is_empty() {
                Vec::new()
            } else {
                let mut query = QueryBuilder::<Sqlite>::new(
                    r#"
                    UPDATE pool_upstream_request_attempts
                    SET
                        finished_at = COALESCE(finished_at, "#,
                );
                query.push_bind(&finished_at);
                query.push(
                    r#", status = "#,
                );
                query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE);
                query.push(
                    r#", phase = "#,
                );
                query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED);
                query.push(
                    r#", failure_kind = COALESCE(failure_kind, "#,
                );
                query.push_bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED);
                query.push(
                    r#"), error_message = COALESCE(error_message, "#,
                );
                query.push_bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE);
                query.push(
                    r#")
                    WHERE id IN ("#,
                );
                {
                    let mut separated = query.separated(", ");
                    for id in ids {
                        separated.push_bind(id);
                    }
                    separated.push_unseparated(r#")"#);
                }
                query.push(
                    r#"
                      AND status = "#,
                );
                query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
                query.push(
                    r#"
                      AND finished_at IS NULL
                    RETURNING id, invoke_id, occurred_at, sticky_key, upstream_account_id
                    "#,
                );
                query
                    .build_query_as::<RecoveredPoolAttemptRow>()
                    .fetch_all(&mut *tx)
                    .await?
            }
        }
        PoolAttemptRecoveryScope::SpecificEarlyPhase { attempt_id } => {
            sqlx::query_as::<_, RecoveredPoolAttemptRow>(
                r#"
                UPDATE pool_upstream_request_attempts
                SET
                    finished_at = COALESCE(finished_at, ?1),
                    status = ?2,
                    phase = ?3,
                    failure_kind = COALESCE(failure_kind, ?4),
                    error_message = COALESCE(error_message, ?5)
                WHERE id = ?6
                  AND status = ?7
                  AND finished_at IS NULL
                  AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
                RETURNING id, invoke_id, occurred_at, sticky_key, upstream_account_id
                "#,
            )
            .bind(finished_at)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
            .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
            .bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
            .bind(attempt_id)
            .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
            .fetch_all(&mut *tx)
            .await?
        }
        PoolAttemptRecoveryScope::StaleEarlyPhase {
            responses_started_before,
            compact_started_before,
            default_started_before,
        } => {
            let candidates = load_stale_pool_upstream_request_attempt_candidate_rows_tx(
                tx,
                responses_started_before,
                compact_started_before,
                default_started_before,
            )
            .await?;
            if candidates.is_empty() {
                Vec::new()
            } else {
                let candidate_ids = candidates.iter().map(|row| row.id).collect::<Vec<_>>();
                recover_stale_pool_upstream_request_attempt_candidates_tx(
                    tx,
                    &candidate_ids,
                    &finished_at,
                    responses_started_before,
                    compact_started_before,
                    default_started_before,
                )
                .await?
            }
        }
    };

    for row in &recovered {
        crate::schema::analytics_changes::append_attempt_upsert_for_row_tx(tx, row.id).await?;
    }
    Ok(recovered)
}

pub(crate) async fn load_stale_pool_upstream_request_attempt_candidate_rows_tx(
    tx: &mut SqliteConnection,
    responses_started_before: &str,
    compact_started_before: &str,
    default_started_before: &str,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    sqlx::query_as::<_, RecoveredPoolAttemptRow>(
        r#"
        SELECT id, invoke_id, occurred_at, sticky_key, upstream_account_id
        FROM pool_upstream_request_attempts
        WHERE status = ?1
          AND finished_at IS NULL
          AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
          AND COALESCE(first_byte_latency_ms, 0) <= 0
          AND NOT EXISTS (
                SELECT 1
                FROM codex_invocations inv
                WHERE inv.source = ?2
                  AND inv.invoke_id = pool_upstream_request_attempts.invoke_id
                  AND inv.occurred_at = pool_upstream_request_attempts.occurred_at
                  AND COALESCE(inv.t_upstream_ttfb_ms, 0) > 0
          )
          AND (
                started_at IS NULL
                OR (
                    endpoint = '/v1/responses'
                    AND started_at <= ?3
                )
                OR (
                    endpoint = '/v1/responses/compact'
                    AND started_at <= ?4
                )
                OR (
                    COALESCE(endpoint, '') NOT IN ('/v1/responses', '/v1/responses/compact')
                    AND started_at <= ?5
                )
          )
        "#,
    )
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .bind(SOURCE_PROXY)
    .bind(responses_started_before)
    .bind(compact_started_before)
    .bind(default_started_before)
    .fetch_all(&mut *tx)
    .await
    .map_err(Into::into)
}

pub(crate) async fn recover_stale_pool_upstream_request_attempt_candidates_tx(
    tx: &mut SqliteConnection,
    candidate_ids: &[i64],
    finished_at: &str,
    responses_started_before: &str,
    compact_started_before: &str,
    default_started_before: &str,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut recovered = Vec::new();
    for chunk in candidate_ids.chunks(POOL_ATTEMPT_RECOVERY_SELECTOR_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                finished_at = COALESCE(finished_at, "#,
        );
        query.push_bind(finished_at);
        query.push(
            r#"),
                status = "#,
        );
        query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE);
        query.push(
            r#",
                phase = "#,
        );
        query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED);
        query.push(
            r#",
                failure_kind = COALESCE(failure_kind, "#,
        );
        query.push_bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED);
        query.push(
            r#"),
                error_message = COALESCE(error_message, "#,
        );
        query.push_bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE);
        query.push(
            r#")
            WHERE id IN ("#,
        );
        let mut separated = query.separated(", ");
        for id in chunk {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        query.push(
            r#"
              AND status = "#,
        );
        query.push_bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
        query.push(
            r#"
              AND finished_at IS NULL
              AND LOWER(TRIM(COALESCE(phase, ''))) IN ('connecting', 'sending_request', 'waiting_first_byte')
              AND COALESCE(first_byte_latency_ms, 0) <= 0
              AND NOT EXISTS (
                    SELECT 1
                    FROM codex_invocations inv
                    WHERE inv.source = "#,
        );
        query.push_bind(SOURCE_PROXY);
        query.push(
            r#"
                      AND inv.invoke_id = pool_upstream_request_attempts.invoke_id
                      AND inv.occurred_at = pool_upstream_request_attempts.occurred_at
                      AND COALESCE(inv.t_upstream_ttfb_ms, 0) > 0
              )
              AND (
                    started_at IS NULL
                    OR (
                        endpoint = '/v1/responses'
                        AND started_at <= "#,
        );
        query.push_bind(responses_started_before);
        query.push(
            r#"
                    )
                    OR (
                        endpoint = '/v1/responses/compact'
                        AND started_at <= "#,
        );
        query.push_bind(compact_started_before);
        query.push(
            r#"
                    )
                    OR (
                        COALESCE(endpoint, '') NOT IN ('/v1/responses', '/v1/responses/compact')
                        AND started_at <= "#,
        );
        query.push_bind(default_started_before);
        query.push(
            r#"
                    )
              )
            RETURNING id, invoke_id, occurred_at, sticky_key, upstream_account_id
            "#,
        );
        recovered.extend(
            query
                .build_query_as::<RecoveredPoolAttemptRow>()
                .fetch_all(&mut *tx)
                .await?,
        );
    }

    Ok(recovered)
}

#[cfg(test)]
pub(crate) async fn recover_stale_pool_upstream_request_attempt_candidates(
    pool: &Pool<Sqlite>,
    candidate_ids: &[i64],
    finished_at: &str,
    responses_started_before: &str,
    compact_started_before: &str,
    default_started_before: &str,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let mut tx = crate::observability::diagnostics::wait(
        crate::observability::diagnostics::Resource::DbPool,
        pool.begin(),
    )
    .await?;
    let recovered = recover_stale_pool_upstream_request_attempt_candidates_tx(
        tx.as_mut(),
        candidate_ids,
        finished_at,
        responses_started_before,
        compact_started_before,
        default_started_before,
    )
    .await?;
    tx.commit().await?;
    Ok(recovered)
}

pub(crate) async fn recover_orphaned_pool_upstream_request_attempts(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    Ok(
        recover_pool_upstream_request_attempts_with_scope(
            pool,
            PoolAttemptRecoveryScope::AllPending,
        )
        .await?
        .len() as u64,
    )
}

pub(crate) async fn recover_pool_upstream_request_attempts_with_connection(
    connection: &mut SqliteConnection,
    scope: PoolAttemptRecoveryScope<'_>,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let mut tx = connection.begin().await?;
    let recovered =
        recover_pool_upstream_request_attempts_with_scope_tx(tx.as_mut(), scope).await?;
    tx.commit().await?;
    Ok(recovered)
}

pub(crate) enum ProxyInvocationRecoveryScope<'a> {
    AllInFlight,
    Ids(&'a [i64]),
    Selectors(&'a [InvocationRecoverySelector]),
}

pub(crate) async fn recover_proxy_invocations_with_scope(
    pool: &Pool<Sqlite>,
    scope: ProxyInvocationRecoveryScope<'_>,
) -> Result<Vec<RecoveredInvocationRow>> {
    let mut tx = crate::observability::diagnostics::wait(
        crate::observability::diagnostics::Resource::DbPool,
        pool.begin(),
    )
    .await?;
    let rows = recover_proxy_invocations_with_scope_tx(tx.as_mut(), scope).await?;
    tx.commit().await?;
    Ok(rows)
}

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
        ProxyInvocationRecoveryScope::Ids(ids) => {
            if ids.is_empty() {
                Vec::new()
            } else {
                let mut query = QueryBuilder::<Sqlite>::new(
                    r#"
                    UPDATE codex_invocations
                    SET status = "#,
                );
                query.push_bind(INVOCATION_STATUS_INTERRUPTED);
                query.push(r#", error_message = "#);
                query.push_bind(INVOCATION_INTERRUPTED_MESSAGE);
                query.push(r#", failure_kind = "#);
                query.push_bind(PROXY_FAILURE_INVOCATION_INTERRUPTED);
                query.push(r#", failure_class = "#);
                query.push_bind(FAILURE_CLASS_SERVICE);
                query.push(
                    r#", is_actionable = 1
                    WHERE id IN ("#,
                );
                {
                    let mut separated = query.separated(", ");
                    for id in ids {
                        separated.push_bind(id);
                    }
                    separated.push_unseparated(r#")"#);
                }
                query.push(
                    r#"
                      AND source = "#,
                );
                query.push_bind(SOURCE_PROXY);
                query.push(
                    r#"
                      AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                    RETURNING id, invoke_id, occurred_at
                    "#,
                );
                query
                    .build_query_as::<RecoveredInvocationRow>()
                    .fetch_all(&mut *tx)
                    .await?
            }
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

    for row in &rows {
        crate::schema::analytics_changes::append_invocation_upsert_for_row_tx(tx, row.id).await?;
    }

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

pub(crate) async fn recover_proxy_invocations_with_connection(
    connection: &mut SqliteConnection,
    scope: ProxyInvocationRecoveryScope<'_>,
) -> Result<Vec<RecoveredInvocationRow>> {
    let mut tx = connection.begin().await?;
    let rows = recover_proxy_invocations_with_scope_tx(tx.as_mut(), scope).await?;
    tx.commit().await?;
    Ok(rows)
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
    let mut tx = crate::observability::diagnostics::wait(
        crate::observability::diagnostics::Resource::DbPool,
        pool.begin(),
    )
    .await?;
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

pub(crate) async fn recover_guard_dropped_pool_invocation_orphan(
    state: &AppState,
    selector: InvocationRecoverySelector,
    recovery_trigger: &'static str,
) -> Result<()> {
    recover_guard_dropped_pool_invocation_orphan_with_prompt_cache_key(
        state,
        selector,
        recovery_trigger,
        None,
    )
    .await
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
    let recovery = {
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
        // The write guard spans candidate selection and the update transaction. Request-side
        // attempt registration takes a read guard until its live lease is installed, so capture
        // the live set only after taking the write guard.
        let _attempt_recovery_gate = crate::runtime::pool_attempt_recovery_gate().write().await;
        let active_attempt_ids = state
            .pool_live_attempt_ids
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
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
