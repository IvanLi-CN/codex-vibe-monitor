use super::*;

pub(super) async fn persist_runtime_attempt_snapshot_tx(
    connection: &mut SqliteConnection,
    snapshot: &RuntimeAttemptSnapshot,
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
            response_content_encoding
        )
        VALUES (
            ?1, ?2, ?3, ?4, 'pool', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
            ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36,
            ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44, ?45, ?46, ?47, ?48
        )
        ON CONFLICT DO UPDATE SET
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
            response_content_encoding = COALESCE(excluded.response_content_encoding, pool_upstream_request_attempts.response_content_encoding)
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
    .execute(&mut *connection)
    .await?;
    Ok(())
}
