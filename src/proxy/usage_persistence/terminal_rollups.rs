use super::*;

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
        let attempts = match query_pool_attempt_records_from_live_with_runtime(
            &state.pool,
            &state.proxy_runtime_attempts,
            invoke_id,
        )
        .await
        {
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

pub(crate) async fn finalize_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    finished_at: &str,
    status: &str,
    http_status: Option<StatusCode>,
    downstream_http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    downstream_error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<()> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let terminal_phase = terminal_pool_upstream_request_attempt_phase(status);
    let compact_support_status =
        compact_support_status.or(pending.compact_support_status.as_deref());
    let compact_support_reason =
        compact_support_reason.or(pending.compact_support_reason.as_deref());
    let request_model = pending.request_model.as_deref();
    let upstream_request_model = pending.upstream_request_model.as_deref();
    let model_mapping_pattern = pending.model_mapping_pattern.as_deref();
    let request_summary_json = pending.request_summary_json.as_deref();
    let upstream_request_compression_algorithm =
        pending.upstream_request_compression_algorithm.as_deref();
    let upstream_request_compression_mode = pending.upstream_request_compression_mode.as_deref();
    let upstream_request_logical_body_bytes = pending.upstream_request_logical_body_bytes;
    let upstream_request_transmitted_body_bytes = pending.upstream_request_transmitted_body_bytes;
    let upstream_request_header_bytes_approx = pending.upstream_request_header_bytes_approx;
    let upstream_response_body_bytes = pending.upstream_response_body_bytes;
    let upstream_response_header_bytes_approx = pending.upstream_response_header_bytes_approx;
    let response_raw_capture_present = pending.response_raw_path.is_some()
        || pending.response_raw_size.is_some()
        || pending.response_raw_truncated
        || pending.response_raw_truncated_reason.is_some()
        || pending.response_content_encoding.is_some();
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: pending.invoke_id.clone(),
        occurred_at: pending.occurred_at.clone(),
        endpoint: pending.endpoint.clone(),
        sticky_key: pending.sticky_key.clone(),
        requester_ip: pending.requester_ip.clone(),
        upstream_base_url_host: pending.upstream_base_url_host.clone(),
        request_model: pending.request_model.clone(),
    };
    let mut tx = pool.begin().await?;
    if let Some(attempt_id) = pending.attempt_id {
        let result = sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                finished_at = ?2,
                status = ?3,
                phase = ?4,
                http_status = ?5,
                downstream_http_status = ?6,
                failure_kind = ?7,
                error_message = ?8,
                downstream_error_message = ?9,
                connect_latency_ms = ?10,
                first_byte_latency_ms = ?11,
                stream_latency_ms = ?12,
                upstream_request_id = ?13,
                compact_support_status = ?14,
                compact_support_reason = ?15,
                upstream_request_compression_algorithm = COALESCE(?16, upstream_request_compression_algorithm),
                upstream_request_compression_mode = COALESCE(?17, upstream_request_compression_mode),
                upstream_request_logical_body_bytes = COALESCE(?18, upstream_request_logical_body_bytes),
                upstream_request_transmitted_body_bytes = COALESCE(?19, upstream_request_transmitted_body_bytes),
                upstream_request_header_bytes_approx = COALESCE(?20, upstream_request_header_bytes_approx),
                upstream_response_body_bytes = COALESCE(?21, upstream_response_body_bytes),
                upstream_response_header_bytes_approx = COALESCE(?22, upstream_response_header_bytes_approx),
                upstream_base_url_host = COALESCE(?23, upstream_base_url_host),
                response_raw_path = COALESCE(?24, response_raw_path),
                response_raw_codec = COALESCE(?25, response_raw_codec),
                response_raw_size = COALESCE(?26, response_raw_size),
                response_raw_truncated = COALESCE(?27, response_raw_truncated),
                response_raw_truncated_reason = COALESCE(?28, response_raw_truncated_reason),
                response_content_encoding = COALESCE(?29, response_content_encoding),
                request_model = COALESCE(?30, request_model),
                upstream_request_model = COALESCE(?31, upstream_request_model),
                model_mapping_pattern = COALESCE(?32, model_mapping_pattern),
                request_summary_json = COALESCE(?33, request_summary_json)
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .bind(finished_at)
        .bind(status)
        .bind(terminal_phase)
        .bind(http_status.map(|value| i64::from(value.as_u16())))
        .bind(downstream_http_status.map(|value| i64::from(value.as_u16())))
        .bind(failure_kind)
        .bind(error_message)
        .bind(downstream_error_message)
        .bind(connect_latency_ms)
        .bind(first_byte_latency_ms)
        .bind(stream_latency_ms)
        .bind(upstream_request_id)
        .bind(compact_support_status)
        .bind(compact_support_reason)
        .bind(upstream_request_compression_algorithm)
        .bind(upstream_request_compression_mode)
        .bind(upstream_request_logical_body_bytes)
        .bind(upstream_request_transmitted_body_bytes)
        .bind(upstream_request_header_bytes_approx)
        .bind(upstream_response_body_bytes)
        .bind(upstream_response_header_bytes_approx)
        .bind(pending.upstream_base_url_host.as_deref())
        .bind(pending.response_raw_path.as_deref())
        .bind(pending.response_raw_codec.as_deref())
        .bind(pending.response_raw_size)
        .bind(response_raw_capture_present.then_some(if pending.response_raw_truncated {
            1_i64
        } else {
            0_i64
        }))
        .bind(pending.response_raw_truncated_reason.as_deref())
        .bind(pending.response_content_encoding.as_deref())
        .bind(request_model)
        .bind(upstream_request_model)
        .bind(model_mapping_pattern)
        .bind(request_summary_json)
        .execute(tx.as_mut())
        .await?;

        if result.rows_affected() > 0 {
            crate::schema::analytics_changes::append_attempt_upsert_for_row_tx(
                tx.as_mut(),
                attempt_id,
            )
            .await?;
            tx.commit().await?;
            return Ok(());
        }
    }

    let inserted_attempt_id = insert_pool_upstream_request_attempt_with_scope_tx(
        tx.as_mut(),
        &trace,
        pending.group_name_snapshot.as_deref(),
        pending.proxy_binding_key_snapshot.as_deref(),
        pending.routing_source.as_deref(),
        pending.routing_selection_audit_json.as_deref(),
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
        downstream_http_status,
        failure_kind,
        error_message,
        downstream_error_message,
        connect_latency_ms,
        first_byte_latency_ms,
        stream_latency_ms,
        upstream_request_id,
        upstream_request_compression_algorithm,
        upstream_request_compression_mode,
        upstream_request_logical_body_bytes,
        upstream_request_transmitted_body_bytes,
        upstream_request_header_bytes_approx,
        upstream_response_body_bytes,
        upstream_response_header_bytes_approx,
        compact_support_status,
        compact_support_reason,
    )
    .await?;
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            response_raw_path = ?2,
            response_raw_codec = COALESCE(?3, response_raw_codec),
            response_raw_size = ?4,
            response_raw_truncated = ?5,
            response_raw_truncated_reason = ?6,
            response_content_encoding = ?7,
            request_model = COALESCE(?8, request_model),
            upstream_request_model = COALESCE(?9, upstream_request_model),
            model_mapping_pattern = COALESCE(?10, model_mapping_pattern),
            request_summary_json = COALESCE(?11, request_summary_json)
        WHERE id = ?1
        "#,
    )
    .bind(inserted_attempt_id)
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
    .bind(request_model)
    .bind(upstream_request_model)
    .bind(model_mapping_pattern)
    .bind(request_summary_json)
    .execute(tx.as_mut())
    .await?;
    crate::schema::analytics_changes::append_attempt_upsert_for_row_tx(
        tx.as_mut(),
        inserted_attempt_id,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn insert_pool_upstream_terminal_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let _ = (
        pool,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    );
    // Terminal pool adjudications do not represent a real upstream dispatch and must not create
    // pseudo-attempt rows going forward. Historical rows remain readable via workflow
    // reconstruction, but new data should rely on invocation-level route + final failure.
    Ok(())
}

pub(crate) async fn insert_and_broadcast_pool_upstream_terminal_attempt(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let _ = (
        state,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    );
    Ok(())
}

pub(crate) fn prompt_cache_upstream_account_rollup_key(
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

pub(crate) async fn load_hourly_rollup_live_progress(
    pool: &Pool<Sqlite>,
    dataset: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0))
}

pub(crate) async fn load_hourly_rollup_live_progress_tx(
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

pub(crate) async fn save_hourly_rollup_live_progress_tx(
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

pub(crate) async fn mark_hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    let result = sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            archive_sha256,
            replayed_at
        )
        SELECT ?1, ?2, ?3, batches.sha256, datetime('now')
        FROM archive_batches AS batches
        WHERE batches.dataset = ?2
          AND batches.status IN ('completed', 'materializing')
          AND batches.file_path = ?3
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
        ON CONFLICT(target, dataset, file_path) DO UPDATE SET
            archive_sha256 = excluded.archive_sha256,
            replayed_at = excluded.replayed_at
        "#,
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() != 1 {
        bail!("archive manifest changed before replay marker could be persisted: {file_path}");
    }
    Ok(())
}

pub(crate) async fn hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
            SELECT 1
            FROM hourly_rollup_archive_replay AS replay
            INNER JOIN archive_batches AS batches
                ON batches.dataset = replay.dataset
               AND batches.file_path = replay.file_path
               AND batches.status = 'completed'
               AND batches.sha256 IS NOT NULL
               AND TRIM(batches.sha256) <> ''
               AND batches.sha256 = replay.archive_sha256
            WHERE replay.target = ?1
              AND replay.dataset = ?2
              AND replay.file_path = ?3
            LIMIT 1
            "#,
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

pub(crate) fn normalized_oauth_account_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(crate) fn looks_like_uuid_shape(value: &str) -> bool {
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

pub(crate) fn oauth_account_id_shape(value: Option<&str>) -> &'static str {
    match normalized_oauth_account_id(value) {
        None => "empty",
        Some(value) if value.starts_with("org_") => "org",
        Some(value) if looks_like_uuid_shape(value) => "uuid",
        Some(_) => "other",
    }
}

pub(crate) fn oauth_account_header_attached_for_account(
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

pub(crate) fn oauth_account_id_shape_for_account(
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
