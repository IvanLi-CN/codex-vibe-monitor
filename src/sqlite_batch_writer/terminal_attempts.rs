use super::*;

impl SqliteBatchWriter {
    pub(super) fn preserve_unconfirmed_runtime_terminal(
        &self,
        terminal: &BatchedTerminalInvocationWrite,
        accepted: bool,
    ) {
        let terminals = [terminal];
        let reason = if accepted {
            "runtime terminal journal confirmation failed"
        } else {
            "runtime terminal journal append failed"
        };
        let mut persisted = false;
        warn!(
            invoke_id = %terminal.record.invoke_id,
            occurred_at = %terminal.record.occurred_at,
            reason,
            "runtime terminal was not confirmed; preserving a supplemental recovery obligation"
        );
        if let Ok(mut guard) = self.terminal_journal.lock()
            && let Some(journal) = guard.as_mut()
        {
            match journal.quarantine_shutdown_batch(&terminals, &[], reason) {
                Ok(()) => {
                    persisted = true;
                    if !journal.remember_shutdown_recovery(&terminals) {
                        warn!(
                            invoke_id = %terminal.record.invoke_id,
                            occurred_at = %terminal.record.occurred_at,
                            "runtime terminal recovery memory is at capacity; durable fact retained for restart replay"
                        );
                    }
                }
                Err(error) => warn!(
                    %error,
                    invoke_id = %terminal.record.invoke_id,
                    occurred_at = %terminal.record.occurred_at,
                    reason,
                    "runtime terminal recovery sink failed"
                ),
            }
        }
        if !persisted
            && let Err(error) = TerminalJournal::quarantine_shutdown_batch_at_database_path(
                &self.database_path,
                &terminals,
                &[],
                reason,
            )
        {
            warn!(
                %error,
                invoke_id = %terminal.record.invoke_id,
                occurred_at = %terminal.record.occurred_at,
                reason,
                "independent runtime terminal recovery sink failed"
            );
        }
    }
}

pub(super) async fn persist_runtime_attempt_snapshot_for_invocation_tx(
    connection: &mut SqliteConnection,
    identity: &crate::storage::traffic::TrafficFactIdentity,
    snapshot: &RuntimeAttemptSnapshot,
) -> Result<()> {
    identity.validate()?;
    if snapshot.invoke_id != identity.invoke_id || snapshot.occurred_at != identity.occurred_at {
        anyhow::bail!(
            "traffic attempt identity does not match invocation {}: attempt={}/{} invocation={}/{}",
            identity.invocation_id,
            snapshot.invoke_id,
            snapshot.occurred_at,
            identity.invoke_id,
            identity.occurred_at
        );
    }
    persist_runtime_attempt_snapshot_tx_with_invocation_id(
        connection,
        snapshot,
        Some(identity.invocation_id),
    )
    .await
}

pub(super) async fn persist_runtime_attempt_snapshot_for_invocation_postgres_tx(
    connection: &mut sqlx::PgConnection,
    identity: &crate::storage::traffic::TrafficFactIdentity,
    snapshot: &RuntimeAttemptSnapshot,
) -> Result<()> {
    identity.validate()?;
    if snapshot.invoke_id != identity.invoke_id || snapshot.occurred_at != identity.occurred_at {
        anyhow::bail!(
            "traffic attempt identity does not match invocation {}: attempt={}/{} invocation={}/{}",
            identity.invocation_id,
            snapshot.invoke_id,
            snapshot.occurred_at,
            identity.invoke_id,
            identity.occurred_at
        );
    }
    persist_runtime_attempt_snapshot_postgres_tx_with_invocation_id(
        connection,
        snapshot,
        Some(identity.invocation_id),
    )
    .await
}

pub(super) async fn persist_runtime_attempt_snapshot_tx(
    connection: &mut SqliteConnection,
    snapshot: &RuntimeAttemptSnapshot,
) -> Result<()> {
    persist_runtime_attempt_snapshot_tx_with_invocation_id(connection, snapshot, None).await
}

async fn persist_runtime_attempt_snapshot_tx_with_invocation_id(
    connection: &mut SqliteConnection,
    snapshot: &RuntimeAttemptSnapshot,
    invocation_id: Option<i64>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id,
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            routing_source,
            routing_selection_audit_json,
            upstream_base_url_host,
            group_name_snapshot,
            proxy_binding_key_snapshot,
            request_model,
            upstream_request_model,
            model_mapping_pattern,
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
            request_summary_json,
            response_raw_path,
            response_raw_codec,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            response_content_encoding,
            invocation_id
        )
        VALUES (
            ?1, ?2, ?3, ?4, 'pool', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
            ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36,
            ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44, ?45, ?46, ?47, ?48, ?49
        )
        ON CONFLICT (attempt_public_id) WHERE (attempt_public_id IS NOT NULL) DO UPDATE SET
            invoke_id = excluded.invoke_id,
            occurred_at = excluded.occurred_at,
            endpoint = excluded.endpoint,
            route_mode = excluded.route_mode,
            sticky_key = COALESCE(excluded.sticky_key, pool_upstream_request_attempts.sticky_key),
            routing_source = COALESCE(excluded.routing_source, pool_upstream_request_attempts.routing_source),
            routing_selection_audit_json = COALESCE(excluded.routing_selection_audit_json, pool_upstream_request_attempts.routing_selection_audit_json),
            upstream_base_url_host = excluded.upstream_base_url_host,
            group_name_snapshot = COALESCE(excluded.group_name_snapshot, pool_upstream_request_attempts.group_name_snapshot),
            proxy_binding_key_snapshot = COALESCE(excluded.proxy_binding_key_snapshot, pool_upstream_request_attempts.proxy_binding_key_snapshot),
            request_model = COALESCE(excluded.request_model, pool_upstream_request_attempts.request_model),
            upstream_request_model = COALESCE(excluded.upstream_request_model, pool_upstream_request_attempts.upstream_request_model),
            model_mapping_pattern = COALESCE(excluded.model_mapping_pattern, pool_upstream_request_attempts.model_mapping_pattern),
            upstream_account_id = excluded.upstream_account_id,
            upstream_route_key = excluded.upstream_route_key,
            attempt_index = excluded.attempt_index,
            distinct_account_index = excluded.distinct_account_index,
            same_account_retry_index = excluded.same_account_retry_index,
            requester_ip = COALESCE(excluded.requester_ip, pool_upstream_request_attempts.requester_ip),
            started_at = excluded.started_at,
            finished_at = excluded.finished_at,
            status = excluded.status,
            phase = excluded.phase,
            http_status = excluded.http_status,
            downstream_http_status = excluded.downstream_http_status,
            failure_kind = COALESCE(excluded.failure_kind, pool_upstream_request_attempts.failure_kind),
            error_message = COALESCE(excluded.error_message, pool_upstream_request_attempts.error_message),
            downstream_error_message = COALESCE(excluded.downstream_error_message, pool_upstream_request_attempts.downstream_error_message),
            connect_latency_ms = excluded.connect_latency_ms,
            first_byte_latency_ms = excluded.first_byte_latency_ms,
            stream_latency_ms = excluded.stream_latency_ms,
            upstream_request_id = COALESCE(excluded.upstream_request_id, pool_upstream_request_attempts.upstream_request_id),
            upstream_request_compression_algorithm = excluded.upstream_request_compression_algorithm,
            upstream_request_compression_mode = excluded.upstream_request_compression_mode,
            upstream_request_logical_body_bytes = excluded.upstream_request_logical_body_bytes,
            upstream_request_transmitted_body_bytes = excluded.upstream_request_transmitted_body_bytes,
            upstream_request_header_bytes_approx = excluded.upstream_request_header_bytes_approx,
            upstream_response_body_bytes = excluded.upstream_response_body_bytes,
            upstream_response_header_bytes_approx = excluded.upstream_response_header_bytes_approx,
            compact_support_status = excluded.compact_support_status,
            compact_support_reason = excluded.compact_support_reason,
            request_summary_json = COALESCE(excluded.request_summary_json, pool_upstream_request_attempts.request_summary_json),
            response_raw_path = COALESCE(excluded.response_raw_path, pool_upstream_request_attempts.response_raw_path),
            response_raw_codec = COALESCE(excluded.response_raw_codec, pool_upstream_request_attempts.response_raw_codec),
            response_raw_size = COALESCE(excluded.response_raw_size, pool_upstream_request_attempts.response_raw_size),
            response_raw_truncated = excluded.response_raw_truncated,
            response_raw_truncated_reason = COALESCE(excluded.response_raw_truncated_reason, pool_upstream_request_attempts.response_raw_truncated_reason),
            response_content_encoding = COALESCE(excluded.response_content_encoding, pool_upstream_request_attempts.response_content_encoding),
            invocation_id = COALESCE(excluded.invocation_id, pool_upstream_request_attempts.invocation_id)
        "#,
    )
    .bind(&snapshot.attempt_public_id)
    .bind(&snapshot.invoke_id)
    .bind(&snapshot.occurred_at)
    .bind(&snapshot.endpoint)
    .bind(snapshot.sticky_key.as_deref())
    .bind(snapshot.routing_source.as_deref())
    .bind(snapshot.routing_selection_audit_json.as_deref())
    .bind(snapshot.upstream_base_url_host.as_deref())
    .bind(snapshot.group_name_snapshot.as_deref())
    .bind(snapshot.proxy_binding_key_snapshot.as_deref())
    .bind(snapshot.request_model.as_deref())
    .bind(snapshot.upstream_request_model.as_deref())
    .bind(snapshot.model_mapping_pattern.as_deref())
    .bind(snapshot.upstream_account_id)
    .bind(&snapshot.upstream_route_key)
    .bind(snapshot.attempt_index)
    .bind(snapshot.distinct_account_index)
    .bind(snapshot.same_account_retry_index)
    .bind(snapshot.requester_ip.as_deref())
    .bind(&snapshot.started_at)
    .bind(snapshot.finished_at.as_deref())
    .bind(&snapshot.status)
    .bind(snapshot.phase.as_deref())
    .bind(snapshot.http_status)
    .bind(snapshot.downstream_http_status)
    .bind(snapshot.failure_kind.as_deref())
    .bind(snapshot.error_message.as_deref())
    .bind(snapshot.downstream_error_message.as_deref())
    .bind(snapshot.connect_latency_ms)
    .bind(snapshot.first_byte_latency_ms)
    .bind(snapshot.stream_latency_ms)
    .bind(snapshot.upstream_request_id.as_deref())
    .bind(snapshot.upstream_request_compression_algorithm.as_deref())
    .bind(snapshot.upstream_request_compression_mode.as_deref())
    .bind(snapshot.upstream_request_logical_body_bytes)
    .bind(snapshot.upstream_request_transmitted_body_bytes)
    .bind(snapshot.upstream_request_header_bytes_approx)
    .bind(snapshot.upstream_response_body_bytes)
    .bind(snapshot.upstream_response_header_bytes_approx)
    .bind(snapshot.compact_support_status.as_deref())
    .bind(snapshot.compact_support_reason.as_deref())
    .bind(snapshot.request_summary_json.as_deref())
    .bind(snapshot.response_raw_path.as_deref())
    .bind(snapshot.response_raw_codec.as_deref().unwrap_or("identity"))
    .bind(snapshot.response_raw_size)
    .bind(i64::from(snapshot.response_raw_truncated))
    .bind(snapshot.response_raw_truncated_reason.as_deref())
    .bind(snapshot.response_content_encoding.as_deref())
    .bind(invocation_id)
    .execute(&mut *connection)
    .await?;
    Ok(())
}

pub(super) async fn persist_runtime_attempt_snapshot_postgres_tx_with_invocation_id(
    connection: &mut sqlx::PgConnection,
    snapshot: &RuntimeAttemptSnapshot,
    invocation_id: Option<i64>,
) -> Result<()> {
    let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "INSERT INTO pool_upstream_request_attempts (\
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,\
            routing_source, routing_selection_audit_json, upstream_base_url_host,\
            group_name_snapshot, proxy_binding_key_snapshot, request_model,\
            upstream_request_model, model_mapping_pattern, upstream_account_id,\
            upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index,\
            requester_ip, started_at, finished_at, status, phase, http_status,\
            downstream_http_status, failure_kind, error_message, downstream_error_message,\
            connect_latency_ms, first_byte_latency_ms, stream_latency_ms, upstream_request_id,\
            upstream_request_compression_algorithm, upstream_request_compression_mode,\
            upstream_request_logical_body_bytes, upstream_request_transmitted_body_bytes,\
            upstream_request_header_bytes_approx, upstream_response_body_bytes,\
            upstream_response_header_bytes_approx, compact_support_status, compact_support_reason,\
            request_summary_json, response_raw_path, response_raw_codec, response_raw_size,\
            response_raw_truncated, response_raw_truncated_reason, response_content_encoding,\
            invocation_id\
        ) VALUES (",
    );
    {
        let mut values = query.separated(", ");
        values.push_bind(&snapshot.attempt_public_id);
        values.push_bind(&snapshot.invoke_id);
        values.push_bind(&snapshot.occurred_at);
        values.push_bind(&snapshot.endpoint);
        values.push_bind("pool");
        values.push_bind(snapshot.sticky_key.as_deref());
        values.push_bind(snapshot.routing_source.as_deref());
        values.push_bind(snapshot.routing_selection_audit_json.as_deref());
        values.push_bind(snapshot.upstream_base_url_host.as_deref());
        values.push_bind(snapshot.group_name_snapshot.as_deref());
        values.push_bind(snapshot.proxy_binding_key_snapshot.as_deref());
        values.push_bind(snapshot.request_model.as_deref());
        values.push_bind(snapshot.upstream_request_model.as_deref());
        values.push_bind(snapshot.model_mapping_pattern.as_deref());
        values.push_bind(snapshot.upstream_account_id);
        values.push_bind(&snapshot.upstream_route_key);
        values.push_bind(snapshot.attempt_index);
        values.push_bind(snapshot.distinct_account_index);
        values.push_bind(snapshot.same_account_retry_index);
        values.push_bind(snapshot.requester_ip.as_deref());
        values.push_bind(&snapshot.started_at);
        values.push_bind(snapshot.finished_at.as_deref());
        values.push_bind(&snapshot.status);
        values.push_bind(snapshot.phase.as_deref());
        values.push_bind(snapshot.http_status);
        values.push_bind(snapshot.downstream_http_status);
        values.push_bind(snapshot.failure_kind.as_deref());
        values.push_bind(snapshot.error_message.as_deref());
        values.push_bind(snapshot.downstream_error_message.as_deref());
        values.push_bind(snapshot.connect_latency_ms);
        values.push_bind(snapshot.first_byte_latency_ms);
        values.push_bind(snapshot.stream_latency_ms);
        values.push_bind(snapshot.upstream_request_id.as_deref());
        values.push_bind(snapshot.upstream_request_compression_algorithm.as_deref());
        values.push_bind(snapshot.upstream_request_compression_mode.as_deref());
        values.push_bind(snapshot.upstream_request_logical_body_bytes);
        values.push_bind(snapshot.upstream_request_transmitted_body_bytes);
        values.push_bind(snapshot.upstream_request_header_bytes_approx);
        values.push_bind(snapshot.upstream_response_body_bytes);
        values.push_bind(snapshot.upstream_response_header_bytes_approx);
        values.push_bind(snapshot.compact_support_status.as_deref());
        values.push_bind(snapshot.compact_support_reason.as_deref());
        values.push_bind(snapshot.request_summary_json.as_deref());
        values.push_bind(snapshot.response_raw_path.as_deref());
        values.push_bind(snapshot.response_raw_codec.as_deref().unwrap_or("identity"));
        values.push_bind(snapshot.response_raw_size);
        values.push_bind(snapshot.response_raw_truncated);
        values.push_bind(snapshot.response_raw_truncated_reason.as_deref());
        values.push_bind(snapshot.response_content_encoding.as_deref());
        values.push_bind(invocation_id);
    }
    query.push(
        r#")
        ON CONFLICT DO UPDATE SET
            invoke_id = EXCLUDED.invoke_id,
            occurred_at = EXCLUDED.occurred_at,
            endpoint = EXCLUDED.endpoint,
            route_mode = EXCLUDED.route_mode,
            sticky_key = COALESCE(EXCLUDED.sticky_key, pool_upstream_request_attempts.sticky_key),
            routing_source = COALESCE(EXCLUDED.routing_source, pool_upstream_request_attempts.routing_source),
            routing_selection_audit_json = COALESCE(EXCLUDED.routing_selection_audit_json, pool_upstream_request_attempts.routing_selection_audit_json),
            upstream_base_url_host = EXCLUDED.upstream_base_url_host,
            group_name_snapshot = COALESCE(EXCLUDED.group_name_snapshot, pool_upstream_request_attempts.group_name_snapshot),
            proxy_binding_key_snapshot = COALESCE(EXCLUDED.proxy_binding_key_snapshot, pool_upstream_request_attempts.proxy_binding_key_snapshot),
            request_model = COALESCE(EXCLUDED.request_model, pool_upstream_request_attempts.request_model),
            upstream_request_model = COALESCE(EXCLUDED.upstream_request_model, pool_upstream_request_attempts.upstream_request_model),
            model_mapping_pattern = COALESCE(EXCLUDED.model_mapping_pattern, pool_upstream_request_attempts.model_mapping_pattern),
            upstream_account_id = EXCLUDED.upstream_account_id,
            upstream_route_key = EXCLUDED.upstream_route_key,
            attempt_index = EXCLUDED.attempt_index,
            distinct_account_index = EXCLUDED.distinct_account_index,
            same_account_retry_index = EXCLUDED.same_account_retry_index,
            requester_ip = COALESCE(EXCLUDED.requester_ip, pool_upstream_request_attempts.requester_ip),
            started_at = EXCLUDED.started_at,
            finished_at = EXCLUDED.finished_at,
            status = EXCLUDED.status,
            phase = EXCLUDED.phase,
            http_status = EXCLUDED.http_status,
            downstream_http_status = EXCLUDED.downstream_http_status,
            failure_kind = COALESCE(EXCLUDED.failure_kind, pool_upstream_request_attempts.failure_kind),
            error_message = COALESCE(EXCLUDED.error_message, pool_upstream_request_attempts.error_message),
            downstream_error_message = COALESCE(EXCLUDED.downstream_error_message, pool_upstream_request_attempts.downstream_error_message),
            connect_latency_ms = EXCLUDED.connect_latency_ms,
            first_byte_latency_ms = EXCLUDED.first_byte_latency_ms,
            stream_latency_ms = EXCLUDED.stream_latency_ms,
            upstream_request_id = COALESCE(EXCLUDED.upstream_request_id, pool_upstream_request_attempts.upstream_request_id),
            upstream_request_compression_algorithm = EXCLUDED.upstream_request_compression_algorithm,
            upstream_request_compression_mode = EXCLUDED.upstream_request_compression_mode,
            upstream_request_logical_body_bytes = EXCLUDED.upstream_request_logical_body_bytes,
            upstream_request_transmitted_body_bytes = EXCLUDED.upstream_request_transmitted_body_bytes,
            upstream_request_header_bytes_approx = EXCLUDED.upstream_request_header_bytes_approx,
            upstream_response_body_bytes = EXCLUDED.upstream_response_body_bytes,
            upstream_response_header_bytes_approx = EXCLUDED.upstream_response_header_bytes_approx,
            compact_support_status = EXCLUDED.compact_support_status,
            compact_support_reason = EXCLUDED.compact_support_reason,
            request_summary_json = COALESCE(EXCLUDED.request_summary_json, pool_upstream_request_attempts.request_summary_json),
            response_raw_path = COALESCE(EXCLUDED.response_raw_path, pool_upstream_request_attempts.response_raw_path),
            response_raw_codec = COALESCE(EXCLUDED.response_raw_codec, pool_upstream_request_attempts.response_raw_codec),
            response_raw_size = COALESCE(EXCLUDED.response_raw_size, pool_upstream_request_attempts.response_raw_size),
            response_raw_truncated = EXCLUDED.response_raw_truncated,
            response_raw_truncated_reason = COALESCE(EXCLUDED.response_raw_truncated_reason, pool_upstream_request_attempts.response_raw_truncated_reason),
            response_content_encoding = COALESCE(EXCLUDED.response_content_encoding, pool_upstream_request_attempts.response_content_encoding),
            invocation_id = COALESCE(EXCLUDED.invocation_id, pool_upstream_request_attempts.invocation_id)
        RETURNING id, response_raw_path, response_raw_codec, response_raw_size
        "#,
    );
    let (attempt_id, response_raw_path, response_raw_codec, response_raw_size) = query
        .build_query_as::<(i64, Option<String>, Option<String>, Option<i64>)>()
        .fetch_one(&mut *connection)
        .await?;
    super::postgres::replace_postgres_attempt_raw_links(
        connection,
        attempt_id,
        response_raw_path.as_deref(),
        response_raw_codec.as_deref(),
        response_raw_size,
    )
    .await?;
    Ok(())
}
