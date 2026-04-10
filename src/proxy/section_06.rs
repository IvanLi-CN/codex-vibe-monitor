fn upstream_account_name_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("upstreamAccountName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn prompt_cache_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("promptCacheKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn sticky_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("stickyKey")
        .or_else(|| value.get("promptCacheKey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn shanghai_now_string() -> String {
    format_naive(Utc::now().with_timezone(&Shanghai).naive_local())
}

fn terminal_pool_upstream_request_attempt_phase(status: &str) -> &'static str {
    if status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
    }
}

async fn insert_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    upstream_account_id: Option<i64>,
    upstream_route_key: Option<&str>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: Option<&str>,
    finished_at: Option<&str>,
    status: &str,
    phase: Option<&str>,
    http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<i64> {
    let result = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            phase,
            http_status,
            failure_kind,
            error_message,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id,
            compact_support_status,
            compact_support_reason
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24
        )
        "#,
    )
    .bind(&trace.invoke_id)
    .bind(&trace.occurred_at)
    .bind(&trace.endpoint)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(trace.sticky_key.as_deref())
    .bind(upstream_account_id)
    .bind(upstream_route_key)
    .bind(attempt_index)
    .bind(distinct_account_index)
    .bind(same_account_retry_index)
    .bind(trace.requester_ip.as_deref())
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(phase)
    .bind(http_status.map(|value| i64::from(value.as_u16())))
    .bind(failure_kind)
    .bind(error_message)
    .bind(connect_latency_ms)
    .bind(first_byte_latency_ms)
    .bind(stream_latency_ms)
    .bind(upstream_request_id)
    .bind(compact_support_status)
    .bind(compact_support_reason)
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

async fn begin_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    let attempt_id = match insert_pool_upstream_request_attempt(
        pool,
        trace,
        Some(upstream_account_id),
        Some(upstream_route_key),
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        Some(started_at),
        None,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    {
        Ok(attempt_id) => Some(attempt_id),
        Err(err) => {
            warn!(
                invoke_id = trace.invoke_id,
                error = %err,
                "failed to persist pending pool attempt"
            );
            None
        }
    };

    PendingPoolAttemptRecord {
        attempt_id,
        invoke_id: trace.invoke_id.clone(),
        occurred_at: trace.occurred_at.clone(),
        endpoint: trace.endpoint.clone(),
        sticky_key: trace.sticky_key.clone(),
        requester_ip: trace.requester_ip.clone(),
        upstream_account_id,
        upstream_route_key: upstream_route_key.to_string(),
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        started_at: started_at.to_string(),
        connect_latency_ms: 0.0,
        first_byte_latency_ms: 0.0,
        compact_support_status: None,
        compact_support_reason: None,
    }
}

async fn update_pool_upstream_request_attempt_phase(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<bool> {
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET phase = ?2
        WHERE id = ?1
          AND status = ?3
          AND finished_at IS NULL
          AND COALESCE(phase, '') <> ?2
        "#,
    )
    .bind(attempt_id)
    .bind(phase)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

async fn persist_pool_upstream_request_attempt_first_byte_progress(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    connect_latency_ms: f64,
    first_byte_latency_ms: f64,
) -> Result<bool> {
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            connect_latency_ms = CASE
                WHEN connect_latency_ms IS NULL OR connect_latency_ms < ?2 THEN ?2
                ELSE connect_latency_ms
            END,
            first_byte_latency_ms = CASE
                WHEN first_byte_latency_ms IS NULL OR first_byte_latency_ms < ?3 THEN ?3
                ELSE first_byte_latency_ms
            END
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .bind(connect_latency_ms)
    .bind(first_byte_latency_ms)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

async fn advance_pool_upstream_request_attempt_phase(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<()> {
    if !update_pool_upstream_request_attempt_phase(&state.pool, pending, phase).await? {
        return Ok(());
    }

    broadcast_pool_upstream_attempts_snapshot(state, &pending.invoke_id).await
}

enum PoolAttemptRecoveryScope<'a> {
    AllPending,
    SpecificEarlyPhase { attempt_id: i64 },
    StaleEarlyPhase {
        responses_started_before: &'a str,
        compact_started_before: &'a str,
        default_started_before: &'a str,
    },
}

async fn recover_pool_upstream_request_attempts_with_scope(
    pool: &Pool<Sqlite>,
    scope: PoolAttemptRecoveryScope<'_>,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let mut tx = pool.begin().await?;
    let recovered = recover_pool_upstream_request_attempts_with_scope_tx(tx.as_mut(), scope).await?;
    tx.commit().await?;
    Ok(recovered)
}

async fn recover_pool_upstream_request_attempts_with_scope_tx(
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

    Ok(recovered)
}

async fn load_stale_pool_upstream_request_attempt_candidate_rows_tx(
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

async fn recover_stale_pool_upstream_request_attempt_candidates_tx(
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
async fn recover_stale_pool_upstream_request_attempt_candidates(
    pool: &Pool<Sqlite>,
    candidate_ids: &[i64],
    finished_at: &str,
    responses_started_before: &str,
    compact_started_before: &str,
    default_started_before: &str,
) -> Result<Vec<RecoveredPoolAttemptRow>> {
    let mut tx = pool.begin().await?;
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

async fn recover_orphaned_pool_upstream_request_attempts(pool: &Pool<Sqlite>) -> Result<u64> {
    Ok(
        recover_pool_upstream_request_attempts_with_scope(pool, PoolAttemptRecoveryScope::AllPending)
            .await?
            .len() as u64,
    )
}

enum ProxyInvocationRecoveryScope<'a> {
    AllInFlight,
    Selectors(&'a [InvocationRecoverySelector]),
}

async fn recover_proxy_invocations_with_scope(
    pool: &Pool<Sqlite>,
    scope: ProxyInvocationRecoveryScope<'_>,
) -> Result<Vec<RecoveredInvocationRow>> {
    let mut tx = pool.begin().await?;
    let rows = recover_proxy_invocations_with_scope_tx(tx.as_mut(), scope).await?;
    tx.commit().await?;
    Ok(rows)
}

async fn recover_proxy_invocations_with_scope_tx(
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

async fn recover_orphaned_proxy_invocations(pool: &Pool<Sqlite>) -> Result<u64> {
    Ok(
        recover_proxy_invocations_with_scope(pool, ProxyInvocationRecoveryScope::AllInFlight)
            .await?
            .len() as u64,
    )
}

fn stale_started_before_string(timeout: Duration, grace: Duration) -> String {
    let cutoff = Utc::now().with_timezone(&Shanghai).naive_local()
        - ChronoDuration::from_std(timeout + grace)
            .expect("pool orphan recovery cutoff should fit chrono duration");
    format_naive(cutoff)
}

async fn load_persisted_api_invocation(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    let mut tx = pool.begin().await?;
    let invocation = load_persisted_api_invocation_tx(tx.as_mut(), invoke_id, occurred_at).await?;
    tx.commit().await?;
    Ok(invocation)
}

async fn broadcast_recovered_proxy_invocations(
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

    if records.iter().any(|record| {
        record
            .prompt_cache_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }

    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let summary_invoke_id = records[0].invoke_id.clone();
    state
        .broadcaster
        .send(BroadcastPayload::Records { records })
        .map_err(|err| anyhow!("failed to broadcast recovered proxy invocation records: {err}"))?;
    broadcast_proxy_capture_follow_up(
        &state.pool,
        &state.broadcaster,
        state.broadcast_state_cache.as_ref(),
        state.config.crs_stats.as_ref(),
        state.config.invocation_max_days,
        &summary_invoke_id,
    )
    .await;

    Ok(())
}

fn pool_routing_reservation_key_for_invoke_id(invoke_id: &str) -> Option<String> {
    let request_id = invoke_id.strip_prefix("proxy-")?.split('-').next()?;
    request_id
        .parse::<u64>()
        .ok()
        .map(build_pool_routing_reservation_key)
}

fn pool_route_orphan_recovery_failure_message(recovery_trigger: &str) -> String {
    format!(
        "pool request was interrupted before completion and recovered via {recovery_trigger}"
    )
}

async fn clean_up_pool_route_after_orphan_recovery(
    state: &AppState,
    invoke_id: &str,
    sticky_key: Option<&str>,
    upstream_account_id: Option<i64>,
    recovery_trigger: &'static str,
    record_route_failure: bool,
) {
    if let Some(reservation_key) = pool_routing_reservation_key_for_invoke_id(invoke_id) {
        release_pool_routing_reservation(state, &reservation_key);
    }

    if !record_route_failure {
        return;
    }

    let Some(account_id) = upstream_account_id else {
        return;
    };
    let error_message = pool_route_orphan_recovery_failure_message(recovery_trigger);
    if let Err(err) = record_pool_route_transport_failure(
        &state.pool,
        account_id,
        sticky_key,
        &error_message,
        Some(invoke_id),
    )
    .await
    {
        warn!(
            invoke_id,
            account_id,
            recovery_trigger,
            error = %err,
            "failed to record pool route transport failure during orphan recovery cleanup"
        );
    }
}

async fn clean_up_recovered_pool_routes(
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
        let record_route_failure = recovered_invocation_keys
            .contains(&(row.invoke_id.as_str(), row.occurred_at.as_str()));
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

async fn recover_guard_dropped_pool_early_phase_orphan(
    state: &AppState,
    pending_attempt_record: PendingPoolAttemptRecord,
    first_byte_observed: bool,
    terminal_outcome_observed: bool,
) -> Result<()> {
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

    let recovered_invocations = if pending_attempt_record.attempt_id.is_none()
        || !recovered_attempts.is_empty()
    {
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

    if recovered_attempts.is_empty() && recovered_invocations.is_empty() {
        return Ok(());
    }

    let record_route_failure = !recovered_invocations.is_empty()
        && (pending_attempt_record.attempt_id.is_none() || !recovered_attempts.is_empty());
    clean_up_pool_route_after_orphan_recovery(
        state,
        &pending_attempt_record.invoke_id,
        pending_attempt_record.sticky_key.as_deref(),
        Some(pending_attempt_record.upstream_account_id),
        "drop_guard",
        record_route_failure,
    )
    .await;

    if !recovered_attempts.is_empty()
        && let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &pending_attempt_record.invoke_id).await
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

async fn recover_guard_dropped_pool_invocation_orphan(
    state: &AppState,
    selector: InvocationRecoverySelector,
    recovery_trigger: &'static str,
) -> Result<()> {
    let recovered_invocations = recover_proxy_invocations_with_scope(
        &state.pool,
        ProxyInvocationRecoveryScope::Selectors(std::slice::from_ref(&selector)),
    )
    .await?;

    if recovered_invocations.is_empty() {
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

async fn recover_guard_dropped_pool_terminal_invocation_orphan(
    state: &AppState,
    selector: InvocationRecoverySelector,
) -> Result<()> {
    recover_guard_dropped_pool_invocation_orphan(
        state,
        selector,
        "terminal_invocation_drop_guard",
    )
    .await
}

pub(crate) async fn recover_stale_pool_early_phase_orphans_runtime(
    state: &AppState,
) -> Result<PoolOrphanRecoveryOutcome> {
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
        return Ok(PoolOrphanRecoveryOutcome::default());
    }

    let selectors: Vec<_> = recovered_attempts
        .iter()
        .map(|row| InvocationRecoverySelector::new(row.invoke_id.clone(), row.occurred_at.clone()))
        .collect();
    let recovered_invocations = recover_proxy_invocations_with_scope_tx(
        tx.as_mut(),
        ProxyInvocationRecoveryScope::Selectors(&selectors),
    )
    .await?;
    tx.commit().await?;

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

async fn broadcast_pool_upstream_attempts_snapshot(
    state: &AppState,
    invoke_id: &str,
) -> Result<()> {
    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let attempts = query_pool_attempt_records_from_live(&state.pool, invoke_id)
        .await
        .map_err(|err| anyhow!("failed to load live pool attempts for SSE broadcast: {err:?}"))?;
    state
        .broadcaster
        .send(BroadcastPayload::PoolAttempts {
            invoke_id: invoke_id.to_string(),
            attempts,
        })
        .map_err(|err| anyhow!("failed to broadcast pool attempts snapshot: {err}"))?;
    Ok(())
}

async fn broadcast_pool_attempt_started_runtime_snapshot(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    runtime_snapshot: &PoolAttemptRuntimeSnapshotContext,
    account: &PoolResolvedAccount,
    attempt_count: usize,
    distinct_account_count: usize,
) {
    let running_record = build_running_proxy_capture_record(
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

async fn finalize_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    finished_at: &str,
    status: &str,
    http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<()> {
    let terminal_phase = terminal_pool_upstream_request_attempt_phase(status);
    let compact_support_status =
        compact_support_status.or(pending.compact_support_status.as_deref());
    let compact_support_reason =
        compact_support_reason.or(pending.compact_support_reason.as_deref());
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: pending.invoke_id.clone(),
        occurred_at: pending.occurred_at.clone(),
        endpoint: pending.endpoint.clone(),
        sticky_key: pending.sticky_key.clone(),
        requester_ip: pending.requester_ip.clone(),
    };
    if let Some(attempt_id) = pending.attempt_id {
        let result = sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                finished_at = ?2,
                status = ?3,
                phase = ?4,
                http_status = ?5,
                failure_kind = ?6,
                error_message = ?7,
                connect_latency_ms = ?8,
                first_byte_latency_ms = ?9,
                stream_latency_ms = ?10,
                upstream_request_id = ?11,
                compact_support_status = ?12,
                compact_support_reason = ?13
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .bind(finished_at)
        .bind(status)
        .bind(terminal_phase)
        .bind(http_status.map(|value| i64::from(value.as_u16())))
        .bind(failure_kind)
        .bind(error_message)
        .bind(connect_latency_ms)
        .bind(first_byte_latency_ms)
        .bind(stream_latency_ms)
        .bind(upstream_request_id)
        .bind(compact_support_status)
        .bind(compact_support_reason)
        .execute(pool)
        .await?;

        if result.rows_affected() > 0 {
            return Ok(());
        }
    }

    insert_pool_upstream_request_attempt(
        pool,
        &trace,
        Some(pending.upstream_account_id),
        Some(pending.upstream_route_key.as_str()),
        pending.attempt_index,
        pending.distinct_account_index,
        pending.same_account_retry_index,
        Some(pending.started_at.as_str()),
        Some(finished_at),
        status,
        Some(terminal_phase),
        http_status,
        failure_kind,
        error_message,
        connect_latency_ms,
        first_byte_latency_ms,
        stream_latency_ms,
        upstream_request_id,
        compact_support_status,
        compact_support_reason,
    )
    .await
    .map(|_| ())
}

async fn insert_pool_upstream_terminal_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let finished_at = shanghai_now_string();
    let upstream_route_key = final_error
        .account
        .as_ref()
        .map(|account| account.upstream_route_key());
    insert_pool_upstream_request_attempt(
        pool,
        trace,
        final_error
            .account
            .as_ref()
            .map(|account| account.account_id),
        upstream_route_key.as_deref(),
        attempt_index,
        distinct_account_index,
        0,
        Some(finished_at.as_str()),
        Some(finished_at.as_str()),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED),
        Some(final_error.status),
        Some(failure_kind),
        Some(final_error.message.as_str()),
        None,
        None,
        None,
        final_error.upstream_request_id.as_deref(),
        None,
        None,
    )
    .await
    .map(|_| ())
}

async fn insert_and_broadcast_pool_upstream_terminal_attempt(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    insert_pool_upstream_terminal_attempt(
        &state.pool,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    )
    .await?;
    broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await?;
    Ok(())
}

fn prompt_cache_upstream_account_rollup_key(
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
) -> String {
    let normalized_name = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (upstream_account_id, normalized_name) {
        (Some(account_id), Some(account_name)) => format!("id:{account_id}|name:{account_name}"),
        (Some(account_id), None) => format!("id:{account_id}"),
        (None, Some(account_name)) => format!("name:{account_name}"),
        (None, None) => "unknown".to_string(),
    }
}

async fn load_hourly_rollup_live_progress(pool: &Pool<Sqlite>, dataset: &str) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0))
}

async fn load_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0))
}

async fn save_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    cursor_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = MAX(hourly_rollup_live_progress.cursor_id, excluded.cursor_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(dataset)
    .bind(cursor_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn mark_hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            replayed_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3 LIMIT 1",
        )
        .bind(target)
        .bind(dataset)
        .bind(file_path)
        .fetch_optional(&mut *tx)
        .await?
        .is_some(),
    )
}

fn normalized_oauth_account_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn looks_like_uuid_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        let is_hyphen = matches!(idx, 8 | 13 | 18 | 23);
        if is_hyphen {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn oauth_account_id_shape(value: Option<&str>) -> &'static str {
    match normalized_oauth_account_id(value) {
        None => "empty",
        Some(value) if value.starts_with("org_") => "org",
        Some(value) if looks_like_uuid_shape(value) => "uuid",
        Some(_) => "other",
    }
}

fn oauth_account_header_attached_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<bool> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(normalized_oauth_account_id(chatgpt_account_id.as_deref()).is_some())
}

fn oauth_account_id_shape_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<&'static str> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(oauth_account_id_shape(chatgpt_account_id.as_deref()))
}

struct ProxyPayloadSummary<'a> {
    target: ProxyCaptureTarget,
    status: StatusCode,
    is_stream: bool,
    request_model: Option<&'a str>,
    requested_service_tier: Option<&'a str>,
    billing_service_tier: Option<&'a str>,
    reasoning_effort: Option<&'a str>,
    response_model: Option<&'a str>,
    usage_missing_reason: Option<&'a str>,
    request_parse_error: Option<&'a str>,
    failure_kind: Option<&'a str>,
    requester_ip: Option<&'a str>,
    upstream_scope: &'a str,
    route_mode: &'a str,
    sticky_key: Option<&'a str>,
    prompt_cache_key: Option<&'a str>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&'a str>,
    upstream_account_kind: Option<&'a str>,
    upstream_base_url_host: Option<&'a str>,
    oauth_account_header_attached: Option<bool>,
    oauth_account_id_shape: Option<&'a str>,
    oauth_forwarded_header_count: Option<usize>,
    oauth_forwarded_header_names: Option<&'a [String]>,
    oauth_fingerprint_version: Option<&'a str>,
    oauth_forwarded_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    oauth_prompt_cache_header_forwarded: Option<bool>,
    oauth_request_body_prefix_fingerprint: Option<&'a str>,
    oauth_request_body_prefix_bytes: Option<usize>,
    oauth_request_body_snapshot_kind: Option<&'a str>,
    oauth_responses_body_mode: Option<&'a str>,
    oauth_responses_rewrite: Option<&'a oauth_bridge::OauthResponsesRewriteSummary>,
    service_tier: Option<&'a str>,
    stream_terminal_event: Option<&'a str>,
    upstream_error_code: Option<&'a str>,
    upstream_error_message: Option<&'a str>,
    upstream_request_id: Option<&'a str>,
    response_content_encoding: Option<&'a str>,
    proxy_display_name: Option<&'a str>,
    proxy_weight_delta: Option<f64>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&'a str>,
}

fn build_proxy_payload_summary(summary: ProxyPayloadSummary<'_>) -> String {
    let ProxyPayloadSummary {
        target,
        status,
        is_stream,
        request_model,
        requested_service_tier,
        billing_service_tier,
        reasoning_effort,
        response_model,
        usage_missing_reason,
        request_parse_error,
        failure_kind,
        requester_ip,
        upstream_scope,
        route_mode,
        sticky_key,
        prompt_cache_key,
        upstream_account_id,
        upstream_account_name,
        upstream_account_kind,
        upstream_base_url_host,
        oauth_account_header_attached,
        oauth_account_id_shape,
        oauth_forwarded_header_count,
        oauth_forwarded_header_names,
        oauth_fingerprint_version,
        oauth_forwarded_header_fingerprints,
        oauth_prompt_cache_header_forwarded,
        oauth_request_body_prefix_fingerprint,
        oauth_request_body_prefix_bytes,
        oauth_request_body_snapshot_kind,
        oauth_responses_body_mode,
        oauth_responses_rewrite,
        service_tier,
        stream_terminal_event,
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
        response_content_encoding,
        proxy_display_name,
        proxy_weight_delta,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
    } = summary;
    let payload = json!({
        "endpoint": target.endpoint(),
        "statusCode": status.as_u16(),
        "isStream": is_stream,
        "requestModel": request_model,
        "requestedServiceTier": requested_service_tier,
        "billingServiceTier": billing_service_tier,
        "reasoningEffort": reasoning_effort,
        "responseModel": response_model,
        "usageMissingReason": usage_missing_reason,
        "requestParseError": request_parse_error,
        "failureKind": failure_kind,
        "requesterIp": requester_ip,
        "upstreamScope": upstream_scope,
        "routeMode": route_mode,
        "stickyKey": sticky_key,
        "promptCacheKey": prompt_cache_key,
        "upstreamAccountId": upstream_account_id,
        "upstreamAccountName": upstream_account_name,
        "upstreamAccountKind": upstream_account_kind,
        "upstreamBaseUrlHost": upstream_base_url_host,
        "oauthAccountHeaderAttached": oauth_account_header_attached,
        "oauthAccountIdShape": oauth_account_id_shape,
        "oauthForwardedHeaderCount": oauth_forwarded_header_count,
        "oauthForwardedHeaderNames": oauth_forwarded_header_names,
        "oauthFingerprintVersion": oauth_fingerprint_version,
        "oauthForwardedHeaderFingerprints": oauth_forwarded_header_fingerprints,
        "oauthPromptCacheHeaderForwarded": oauth_prompt_cache_header_forwarded,
        "oauthRequestBodyPrefixFingerprint": oauth_request_body_prefix_fingerprint,
        "oauthRequestBodyPrefixBytes": oauth_request_body_prefix_bytes,
        "oauthRequestBodySnapshotKind": oauth_request_body_snapshot_kind,
        "oauthResponsesBodyMode": oauth_responses_body_mode,
        "oauthResponsesRewrite": oauth_responses_rewrite,
        "serviceTier": service_tier,
        "streamTerminalEvent": stream_terminal_event,
        "upstreamErrorCode": upstream_error_code,
        "upstreamErrorMessage": upstream_error_message,
        "upstreamRequestId": upstream_request_id,
        "responseContentEncoding": response_content_encoding,
        "proxyDisplayName": proxy_display_name,
        "proxyWeightDelta": proxy_weight_delta,
        "poolAttemptCount": pool_attempt_count,
        "poolDistinctAccountCount": pool_distinct_account_count,
        "poolAttemptTerminalReason": pool_attempt_terminal_reason,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn invocation_status_is_in_flight(status: Option<&str>) -> bool {
    matches!(
        status
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        INVOCATION_STATUS_RUNNING | INVOCATION_STATUS_PENDING
    )
}

fn invocation_status_is_recoverable_proxy_interrupted(
    status: Option<&str>,
    failure_kind: Option<&str>,
) -> bool {
    status
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case(INVOCATION_STATUS_INTERRUPTED)
        && failure_kind
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case(PROXY_FAILURE_INVOCATION_INTERRUPTED)
}

fn nullable_runtime_timing_value(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[derive(Debug, FromRow)]
struct PersistedInvocationIdentityRow {
    id: i64,
    status: Option<String>,
    failure_kind: Option<String>,
}

async fn load_persisted_invocation_identity_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Option<PersistedInvocationIdentityRow>> {
    sqlx::query_as::<_, PersistedInvocationIdentityRow>(
        r#"
        SELECT id, status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(Into::into)
}

async fn load_persisted_api_invocation_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    sqlx::query_as::<_, ApiInvocation>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            source,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.reasoningEffort') END AS reasoning_effort,
            total_tokens,
            cost,
            status,
            error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
            COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.streamTerminalEvent') END AS stream_terminal_event,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorCode') END AS upstream_error_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorMessage') END AS upstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamRequestId') END AS upstream_request_id,
            failure_class,
            is_actionable,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.routeMode') END AS route_mode,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END AS upstream_account_id,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountName') END AS upstream_account_name,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.responseContentEncoding') END AS response_content_encoding,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolAttemptCount')
            END AS pool_attempt_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolDistinctAccountCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolDistinctAccountCount')
            END AS pool_distinct_account_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptTerminalReason') = 'text'
                THEN json_extract(payload, '$.poolAttemptTerminalReason')
            END AS pool_attempt_terminal_reason,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                THEN json_extract(payload, '$.requestedServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text'
                THEN json_extract(payload, '$.serviceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text'
                THEN json_extract(payload, '$.service_tier') END AS service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.billingServiceTier') = 'text'
                THEN json_extract(payload, '$.billingServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.billing_service_tier') = 'text'
                THEN json_extract(payload, '$.billing_service_tier') END AS billing_service_tier,
            CASE WHEN json_valid(payload)
              AND json_type(payload, '$.proxyWeightDelta') IN ('integer', 'real')
              THEN json_extract(payload, '$.proxyWeightDelta') END AS proxy_weight_delta,
            cost_estimated,
            price_version,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            detail_level,
            detail_pruned_at,
            detail_prune_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms,
            created_at
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(Into::into)
}

async fn touch_invocation_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    payload: Option<&str>,
) -> Result<()> {
    if let Some(upstream_account_id) = upstream_account_id_from_payload(payload) {
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

async fn persist_and_broadcast_proxy_capture_runtime_snapshot(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let persisted = persist_proxy_capture_runtime_record(&state.pool, record).await?;
    let Some(persisted_record) = persisted else {
        return Ok(());
    };

    if persisted_record
        .prompt_cache_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }

    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let invoke_id = persisted_record.invoke_id.clone();
    if let Err(err) = state.broadcaster.send(BroadcastPayload::Records {
        records: vec![persisted_record],
    }) {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast runtime proxy capture snapshot"
        );
    }

    Ok(())
}

async fn persist_proxy_capture_runtime_record(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let failure = classify_invocation_failure(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
    );
    let failure_kind = record
        .failure_kind
        .clone()
        .or_else(|| failure.failure_kind.clone());
    let t_req_read_ms = nullable_runtime_timing_value(record.timings.t_req_read_ms);
    let t_req_parse_ms = nullable_runtime_timing_value(record.timings.t_req_parse_ms);
    let t_upstream_connect_ms = nullable_runtime_timing_value(record.timings.t_upstream_connect_ms);
    let t_upstream_ttfb_ms = nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms);
    let created_at = format_utc_iso_millis(Utc::now());
    let mut tx = pool.begin().await?;
    let insert_result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            raw_response,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms,
            created_at
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36
        )
        "#,
    )
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
    .bind(record.cost_estimated as i64)
    .bind(record.price_version.as_deref())
    .bind(&record.status)
    .bind(record.error_message.as_deref())
    .bind(failure_kind.as_deref())
    .bind(failure.failure_class.as_str())
    .bind(failure.is_actionable as i64)
    .bind(record.payload.as_deref())
    .bind(&record.raw_response)
    .bind(record.req_raw.path.as_deref())
    .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
    .bind(record.req_raw.truncated as i64)
    .bind(record.req_raw.truncated_reason.as_deref())
    .bind(record.resp_raw.path.as_deref())
    .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
    .bind(record.resp_raw.truncated as i64)
    .bind(record.resp_raw.truncated_reason.as_deref())
    .bind(None::<f64>)
    .bind(t_req_read_ms)
    .bind(t_req_parse_ms)
    .bind(t_upstream_connect_ms)
    .bind(t_upstream_ttfb_ms)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .bind(created_at)
    .execute(tx.as_mut())
    .await?;

    let invocation_id = if insert_result.rows_affected() > 0 {
        let inserted_id = insert_result.last_insert_rowid();
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &[InvocationHourlySourceRecord {
                id: inserted_id,
                occurred_at: record.occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some(record.status.clone()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                total_tokens: record.usage.total_tokens,
                cost: record.cost,
                error_message: record.error_message.clone(),
                failure_kind: failure_kind.clone(),
                failure_class: Some(failure.failure_class.as_str().to_string()),
                is_actionable: Some(failure.is_actionable as i64),
                payload: record.payload.clone(),
                t_total_ms: None,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            }],
            &INVOCATION_HOURLY_ROLLUP_TARGETS,
        )
        .await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            inserted_id,
        )
        .await?;
        touch_invocation_upstream_account_last_activity_tx(
            tx.as_mut(),
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
        inserted_id
    } else {
        let Some(existing) = load_persisted_invocation_identity_tx(
            tx.as_mut(),
            &record.invoke_id,
            &record.occurred_at,
        )
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let allow_terminal_repair = !invocation_status_is_in_flight(Some(record.status.as_str()))
            && invocation_status_is_recoverable_proxy_interrupted(
                existing.status.as_deref(),
                existing.failure_kind.as_deref(),
            );
        if !invocation_status_is_in_flight(existing.status.as_deref()) && !allow_terminal_repair {
            tx.commit().await?;
            return Ok(None);
        }

        let affected = sqlx::query(
            r#"
            UPDATE codex_invocations
            SET source = ?2,
                model = ?3,
                input_tokens = ?4,
                output_tokens = ?5,
                cache_input_tokens = ?6,
                reasoning_tokens = ?7,
                total_tokens = ?8,
                cost = ?9,
                cost_estimated = ?10,
                price_version = ?11,
                status = ?12,
                error_message = ?13,
                failure_kind = ?14,
                failure_class = ?15,
                is_actionable = ?16,
                payload = ?17,
                raw_response = ?18,
                request_raw_path = ?19,
                request_raw_size = ?20,
                request_raw_truncated = ?21,
                request_raw_truncated_reason = ?22,
                response_raw_path = ?23,
                response_raw_size = ?24,
                response_raw_truncated = ?25,
                response_raw_truncated_reason = ?26,
                t_total_ms = ?27,
                t_req_read_ms = ?28,
                t_req_parse_ms = ?29,
                t_upstream_connect_ms = ?30,
                t_upstream_ttfb_ms = ?31,
                t_upstream_stream_ms = ?32,
                t_resp_parse_ms = ?33,
                t_persist_ms = ?34
            WHERE id = ?1
              AND (
                    LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                    OR (
                        LOWER(TRIM(COALESCE(status, ''))) = 'interrupted'
                        AND LOWER(TRIM(COALESCE(failure_kind, ''))) = 'proxy_interrupted'
                    )
              )
            "#,
        )
        .bind(existing.id)
        .bind(SOURCE_PROXY)
        .bind(&record.model)
        .bind(record.usage.input_tokens)
        .bind(record.usage.output_tokens)
        .bind(record.usage.cache_input_tokens)
        .bind(record.usage.reasoning_tokens)
        .bind(record.usage.total_tokens)
        .bind(record.cost)
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(&record.raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(record.resp_raw.path.as_deref())
        .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
        .bind(record.resp_raw.truncated as i64)
        .bind(record.resp_raw.truncated_reason.as_deref())
        .bind(None::<f64>)
        .bind(t_req_read_ms)
        .bind(t_req_parse_ms)
        .bind(t_upstream_connect_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .execute(tx.as_mut())
        .await?
        .rows_affected();
        if affected == 0 {
            tx.commit().await?;
            return Ok(None);
        }
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[existing.id]).await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            existing.id,
        )
        .await?;
        touch_invocation_upstream_account_last_activity_tx(
            tx.as_mut(),
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
        existing.id
    };

    let persisted = load_persisted_api_invocation_tx(tx.as_mut(), &record.invoke_id, &record.occurred_at)
        .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        invocation_id,
    )
    .await?;
    tx.commit().await?;

    Ok(Some(persisted))
}

fn build_running_proxy_capture_record(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    pool_route_active: bool,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
    upstream_account_kind: Option<&str>,
    upstream_base_url_host: Option<&str>,
    proxy_display_name: Option<&str>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&str>,
    response_content_encoding: Option<&str>,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
    t_upstream_connect_ms: f64,
    t_upstream_ttfb_ms: f64,
) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: request_info.model.clone(),
        usage: ParsedUsage::default(),
        cost: None,
        cost_estimated: false,
        price_version: None,
        status: "running".to_string(),
        error_message: None,
        failure_kind: None,
        payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
            target,
            status: StatusCode::OK,
            is_stream: request_info.is_stream,
            request_model: request_info.model.as_deref(),
            requested_service_tier: request_info.requested_service_tier.as_deref(),
            billing_service_tier: None,
            reasoning_effort: request_info.reasoning_effort.as_deref(),
            response_model: None,
            usage_missing_reason: None,
            request_parse_error: request_info.parse_error.as_deref(),
            failure_kind: None,
            requester_ip,
            upstream_scope: if pool_route_active {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_route_active {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key,
            prompt_cache_key,
            upstream_account_id,
            upstream_account_name,
            upstream_account_kind,
            upstream_base_url_host,
            oauth_account_header_attached: None,
            oauth_account_id_shape: None,
            oauth_forwarded_header_count: None,
            oauth_forwarded_header_names: None,
            oauth_fingerprint_version: None,
            oauth_forwarded_header_fingerprints: None,
            oauth_prompt_cache_header_forwarded: None,
            oauth_request_body_prefix_fingerprint: None,
            oauth_request_body_prefix_bytes: None,
            oauth_request_body_snapshot_kind: None,
            oauth_responses_body_mode: None,
            oauth_responses_rewrite: None,
            service_tier: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
            response_content_encoding,
            proxy_display_name,
            proxy_weight_delta: None,
            pool_attempt_count,
            pool_distinct_account_count,
            pool_attempt_terminal_reason,
        })),
        raw_response: "{}".to_string(),
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: StageTimings {
            t_total_ms: 0.0,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        },
    }
}

fn resolve_invocation_proxy_display_name(
    selected_proxy: Option<&SelectedForwardProxy>,
) -> Option<String> {
    selected_proxy.map(|proxy| proxy.display_name.clone())
}

fn summarize_response_content_encoding(content_encoding: Option<&str>) -> String {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        "identity".to_string()
    } else {
        encodings.join(", ")
    }
}

#[derive(Default)]
struct RawResponsePreviewBuffer {
    bytes: Vec<u8>,
}

impl RawResponsePreviewBuffer {
    fn append(&mut self, chunk: &[u8]) {
        let remaining = RAW_RESPONSE_PREVIEW_LIMIT.saturating_sub(self.bytes.len());
        if remaining == 0 || chunk.is_empty() {
            return;
        }
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn into_preview(self) -> String {
        build_raw_response_preview(&self.bytes)
    }
}

struct BoundedResponseParseBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded_limit: bool,
}

impl BoundedResponseParseBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded_limit: false,
        }
    }

    fn append(&mut self, chunk: &[u8]) {
        if self.exceeded_limit || chunk.is_empty() {
            return;
        }

        let remaining = self.limit.saturating_sub(self.bytes.len());
        let take_len = remaining.min(chunk.len());
        if take_len > 0 {
            self.bytes.extend_from_slice(&chunk[..take_len]);
        }
        if take_len < chunk.len() {
            self.exceeded_limit = true;
        }
    }

    fn into_response_info(
        self,
        target: ProxyCaptureTarget,
        content_encoding: Option<&str>,
    ) -> ResponseCaptureInfo {
        let mut response_info =
            parse_target_response_payload(target, &self.bytes, false, content_encoding);
        if self.exceeded_limit {
            merge_response_capture_reason(
                &mut response_info,
                PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
            );
        }
        response_info
    }
}

struct StreamingRawPayloadWriter {
    path: PathBuf,
    max_bytes: Option<usize>,
    written_bytes: usize,
    meta: RawPayloadMeta,
    file: Option<tokio::fs::File>,
}

impl StreamingRawPayloadWriter {
    fn new(config: &AppConfig, invoke_id: &str, kind: &str) -> Self {
        let path = config
            .resolved_proxy_raw_dir()
            .join(format!("{invoke_id}-{kind}.bin"));
        Self {
            path,
            max_bytes: config.proxy_raw_max_bytes,
            written_bytes: 0,
            meta: RawPayloadMeta::default(),
            file: None,
        }
    }

    async fn ensure_file(&mut self) -> io::Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        let Some(parent) = self.path.parent() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("raw payload path has no parent: {}", self.path.display()),
            ));
        };
        tokio::fs::create_dir_all(parent).await?;
        let file = tokio::fs::File::create(&self.path).await?;
        self.meta.path = Some(self.path.to_string_lossy().to_string());
        self.file = Some(file);
        Ok(())
    }

    fn mark_max_bytes_exceeded(&mut self) {
        self.meta.truncated = true;
        self.meta
            .truncated_reason
            .get_or_insert_with(|| "max_bytes_exceeded".to_string());
    }

    async fn record_write_failure(&mut self, err: io::Error) {
        self.meta.truncated = true;
        self.meta.truncated_reason = Some(format!("write_failed:{err}"));
        self.file = None;
        if self.meta.path.is_some() {
            let _ = tokio::fs::remove_file(&self.path).await;
            self.meta.path = None;
        }
    }

    async fn append(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        self.meta.size_bytes = self.meta.size_bytes.saturating_add(bytes.len() as i64);

        if self
            .meta
            .truncated_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("write_failed:"))
        {
            return;
        }

        if let Err(err) = self.ensure_file().await {
            self.record_write_failure(err).await;
            return;
        }

        let write_len = if let Some(limit) = self.max_bytes {
            let remaining = limit.saturating_sub(self.written_bytes);
            if remaining == 0 {
                self.mark_max_bytes_exceeded();
                return;
            }
            let write_len = remaining.min(bytes.len());
            if write_len < bytes.len() {
                self.mark_max_bytes_exceeded();
            }
            write_len
        } else {
            bytes.len()
        };

        if write_len == 0 {
            return;
        }

        if let Some(file) = self.file.as_mut() {
            if let Err(err) = file.write_all(&bytes[..write_len]).await {
                self.record_write_failure(err).await;
                return;
            }
            self.written_bytes = self.written_bytes.saturating_add(write_len);
        }
    }

    async fn finish(mut self) -> RawPayloadMeta {
        if let Some(file) = self.file.as_mut()
            && let Err(err) = file.flush().await
        {
            self.record_write_failure(err).await;
        }
        self.meta
    }
}

fn build_raw_response_preview(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "{}".to_string();
    }
    let preview = if bytes.len() > RAW_RESPONSE_PREVIEW_LIMIT {
        &bytes[..RAW_RESPONSE_PREVIEW_LIMIT]
    } else {
        bytes
    };
    String::from_utf8_lossy(preview).to_string()
}

fn extract_error_message_from_response(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    value
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

fn summarize_plaintext_upstream_error(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if text.starts_with('<')
        || lower.starts_with("<!doctype")
        || lower.starts_with("<html")
        || lower.starts_with("<body")
    {
        return None;
    }
    Some(text.chars().take(240).collect())
}

fn extract_error_message_from_response_preview(bytes: &[u8]) -> Option<String> {
    extract_error_message_from_response(bytes).or_else(|| summarize_plaintext_upstream_error(bytes))
}

fn merge_response_capture_reason(
    response_info: &mut ResponseCaptureInfo,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
        format!("{reason};{existing}")
    } else {
        reason
    };
    response_info.usage_missing_reason = Some(combined_reason);
}
