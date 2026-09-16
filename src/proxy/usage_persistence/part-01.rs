pub(crate) async fn begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    group_name_snapshot: Option<&str>,
    proxy_binding_key_snapshot: Option<&str>,
    routing_source: Option<PoolRoutingSelectionSource>,
    routing_selection_audit: Option<&PoolRoutingSelectionAudit>,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let routing_source_value = routing_source.map(PoolRoutingSelectionSource::as_persisted_str);
    let routing_selection_audit_json =
        routing_selection_audit.and_then(|audit| serde_json::to_string(audit).ok());
    let attempt_id = match insert_pool_upstream_request_attempt_with_scope(
        pool,
        trace,
        group_name_snapshot,
        proxy_binding_key_snapshot,
        routing_source_value,
        routing_selection_audit_json.as_deref(),
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
        Ok(attempt_id) => {
            if let Err(err) = observe_model_route_seen_admitted(
                pool,
                upstream_account_id,
                trace.request_model.as_deref(),
            )
            .await
            {
                warn!(
                    account_id = upstream_account_id,
                    model = ?trace.request_model,
                    error = %err,
                    "failed to observe model route at attempt start"
                );
            }
            Some(attempt_id)
        }
        Err(err) => {
            warn!(
                invoke_id = trace.invoke_id,
                error = %err,
                "failed to persist pending pool attempt"
            );
            None
        }
    };

    if routing_source == Some(PoolRoutingSelectionSource::PriorityHandoff) {
        remember_priority_handoff_attempt(
            attempt_id,
            Some(trace.invoke_id.as_str()),
            upstream_account_id,
            trace.request_model.as_deref(),
            routing_selection_audit_json.as_deref(),
        );
    }

    let attempt_public_id = if let Some(attempt_id) = attempt_id {
        match sqlx::query_scalar::<_, Option<String>>(
            "SELECT attempt_public_id FROM pool_upstream_request_attempts WHERE id = ?1",
        )
        .bind(attempt_id)
        .fetch_optional(pool)
        .await
        {
            Ok(value) => value.flatten(),
            Err(err) => {
                warn!(
                    attempt_id,
                    error = %err,
                    "failed to load pool attempt public id after insert"
                );
                None
            }
        }
    } else {
        None
    };

    PendingPoolAttemptRecord {
        attempt_id,
        attempt_public_id,
        invoke_id: trace.invoke_id.clone(),
        occurred_at: trace.occurred_at.clone(),
        endpoint: trace.endpoint.clone(),
        sticky_key: trace.sticky_key.clone(),
        routing_source: routing_source_value.map(ToOwned::to_owned),
        routing_selection_audit_json,
        requester_ip: trace.requester_ip.clone(),
        upstream_base_url_host: trace.upstream_base_url_host.clone(),
        group_name_snapshot: group_name_snapshot.map(ToOwned::to_owned),
        proxy_binding_key_snapshot: proxy_binding_key_snapshot.map(ToOwned::to_owned),
        request_model: trace.request_model.clone(),
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
        upstream_request_compression_algorithm: None,
        upstream_request_compression_mode: None,
        upstream_request_logical_body_bytes: None,
        upstream_request_transmitted_body_bytes: None,
        upstream_request_header_bytes_approx: None,
        upstream_response_body_bytes: None,
        upstream_response_header_bytes_approx: None,
        response_raw_path: None,
        response_raw_codec: None,
        response_raw_size: None,
        response_raw_truncated: false,
        response_raw_truncated_reason: None,
        response_content_encoding: None,
    }
}

pub(crate) fn set_pending_pool_upstream_request_attempt_response_capture(
    pending: &mut PendingPoolAttemptRecord,
    meta: &RawPayloadMeta,
    content_encoding: Option<&str>,
) {
    pending.response_raw_path = meta.path.clone();
    pending.response_raw_codec = Some(raw_payload_meta_codec(meta).to_string());
    pending.response_raw_size = Some(meta.size_bytes);
    pending.response_raw_truncated = meta.truncated;
    pending.response_raw_truncated_reason = meta.truncated_reason.clone();
    pending.response_content_encoding = content_encoding
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
}

pub(crate) fn pool_attempt_response_capture_key(pending: &PendingPoolAttemptRecord) -> String {
    pending.attempt_public_id.clone().unwrap_or_else(|| {
        format!(
            "{}-attempt-{}-{}-{}",
            pending.invoke_id,
            pending.attempt_index,
            pending.distinct_account_index,
            pending.same_account_retry_index,
        )
    })
}

pub(crate) async fn persist_pool_upstream_request_attempt_response_capture(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
) -> Result<()> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let attempt_id = match pending.attempt_id {
        Some(attempt_id) => Some(attempt_id),
        None => sqlx::query_scalar::<_, Option<i64>>(
            r#"
            SELECT id
            FROM pool_upstream_request_attempts
            WHERE invoke_id = ?1
              AND occurred_at = ?2
              AND attempt_index = ?3
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind(&pending.invoke_id)
        .bind(&pending.occurred_at)
        .bind(pending.attempt_index)
        .fetch_optional(pool)
        .await?
        .flatten(),
    };
    let Some(attempt_id) = attempt_id else {
        return Ok(());
    };
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            response_raw_path = ?2,
            response_raw_codec = COALESCE(?3, response_raw_codec),
            response_raw_size = ?4,
            response_raw_truncated = ?5,
            response_raw_truncated_reason = ?6,
            response_content_encoding = ?7
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .bind(pending.response_raw_path.as_deref())
    .bind(pending.response_raw_codec.as_deref())
    .bind(pending.response_raw_size)
    .bind(if pending.response_raw_truncated {
        1_i64
    } else {
        0_i64
    })
    .bind(pending.response_raw_truncated_reason.as_deref())
    .bind(pending.response_content_encoding.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) fn update_pending_pool_upstream_request_attempt_http_bytes(
    pending: &mut PendingPoolAttemptRecord,
    logical_body_bytes: Option<usize>,
    transmitted_body_bytes: Option<usize>,
    request_header_bytes_approx: Option<usize>,
    response_body_bytes: Option<usize>,
    response_header_bytes_approx: Option<usize>,
) {
    pending.upstream_request_logical_body_bytes =
        logical_body_bytes.and_then(|value| i64::try_from(value).ok());
    pending.upstream_request_transmitted_body_bytes =
        transmitted_body_bytes.and_then(|value| i64::try_from(value).ok());
    pending.upstream_request_header_bytes_approx =
        request_header_bytes_approx.and_then(|value| i64::try_from(value).ok());
    pending.upstream_response_body_bytes =
        response_body_bytes.and_then(|value| i64::try_from(value).ok());
    pending.upstream_response_header_bytes_approx =
        response_header_bytes_approx.and_then(|value| i64::try_from(value).ok());
}

pub(crate) async fn annotate_pool_upstream_request_attempt_request_compression(
    pool: &Pool<Sqlite>,
    pending: &mut PendingPoolAttemptRecord,
    algorithm: &str,
    mode: &str,
) -> Result<bool> {
    pending.upstream_request_compression_algorithm = Some(algorithm.to_string());
    pending.upstream_request_compression_mode = Some(mode.to_string());

    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;

    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            upstream_request_compression_algorithm = ?2,
            upstream_request_compression_mode = ?3
        WHERE id = ?1
          AND (
                COALESCE(upstream_request_compression_algorithm, '') <> ?2
                OR COALESCE(upstream_request_compression_mode, '') <> ?3
              )
        "#,
    )
    .bind(attempt_id)
    .bind(algorithm)
    .bind(mode)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Stores the bounded rewrite audit on the attempt that produced it. This keeps
/// failover timelines account-accurate instead of inheriting the final attempt's audit.
pub(crate) async fn annotate_pool_upstream_request_attempt_codex_imagegen_rewrite(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    codex_imagegen_rewrite: Option<&Value>,
) -> Result<bool> {
    let Some(codex_imagegen_rewrite) = codex_imagegen_rewrite else {
        return Ok(false);
    };
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;

    let existing = sqlx::query_scalar::<_, Option<String>>(
        "SELECT request_summary_json FROM pool_upstream_request_attempts WHERE id = ?1",
    )
    .bind(attempt_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let mut summary = existing
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Default::default()));
    let Some(summary) = summary.as_object_mut() else {
        return Ok(false);
    };
    if summary.get("codexImagegenRewrite") == Some(codex_imagegen_rewrite) {
        return Ok(false);
    }
    summary.insert(
        "codexImagegenRewrite".to_string(),
        codex_imagegen_rewrite.clone(),
    );
    let request_summary_json = serde_json::to_string(&Value::Object(summary.clone()))?;
    let result = sqlx::query(
        "UPDATE pool_upstream_request_attempts SET request_summary_json = ?2 WHERE id = ?1",
    )
    .bind(attempt_id)
    .bind(request_summary_json)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub(crate) async fn update_pool_upstream_request_attempt_phase(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<bool> {
    update_pool_upstream_request_attempt_progress(pool, pending, phase, None, None, None, None)
        .await
}

pub(crate) async fn update_pool_upstream_request_attempt_progress(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<bool> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let result = sqlx::query(
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
    .bind(attempt_id)
    .bind(phase)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .bind(connect_latency_ms)
    .bind(first_byte_latency_ms)
    .bind(compact_support_status)
    .bind(compact_support_reason)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub(crate) async fn persist_pool_upstream_request_attempt_first_byte_progress(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    connect_latency_ms: f64,
    first_byte_latency_ms: f64,
) -> Result<bool> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
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

pub(crate) async fn advance_pool_upstream_request_attempt_phase(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<()> {
    enqueue_pool_upstream_request_attempt_progress(state, pending, phase, None, None, None, None);
    Ok(())
}

pub(crate) fn enqueue_pool_upstream_request_attempt_progress(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> bool {
    let Some(attempt_id) = pending.attempt_id else {
        return false;
    };
    state
        .sqlite_batch_writer
        .enqueue(SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
            attempt_id,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: phase.to_string(),
            connect_latency_ms,
            first_byte_latency_ms,
            compact_support_status: compact_support_status.map(ToOwned::to_owned),
            compact_support_reason: compact_support_reason.map(ToOwned::to_owned),
        }))
}

pub(crate) enum PoolAttemptRecoveryScope<'a> {
    AllPending,
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
    let mut tx = pool.begin().await?;
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

pub(crate) enum ProxyInvocationRecoveryScope<'a> {
    AllInFlight,
    Selectors(&'a [InvocationRecoverySelector]),
}

pub(crate) async fn recover_proxy_invocations_with_scope(
    pool: &Pool<Sqlite>,
    scope: ProxyInvocationRecoveryScope<'_>,
) -> Result<Vec<RecoveredInvocationRow>> {
    let mut tx = pool.begin().await?;
    let rows = recover_proxy_invocations_with_scope_tx(tx.as_mut(), scope).await?;
    tx.commit().await?;
    Ok(rows)
}
