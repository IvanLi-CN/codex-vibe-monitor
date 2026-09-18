INSERT INTO pool_upstream_request_attempts (
    attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
    routing_source, routing_selection_audit_json, upstream_base_url_host,
    group_name_snapshot, proxy_binding_key_snapshot, request_model,
    upstream_request_model, model_mapping_pattern, upstream_account_id,
    upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index,
    requester_ip, started_at, finished_at, status, phase, http_status,
    downstream_http_status, failure_kind, error_message, downstream_error_message,
    connect_latency_ms, first_byte_latency_ms, stream_latency_ms, upstream_request_id,
    upstream_request_compression_algorithm, upstream_request_compression_mode,
    upstream_request_logical_body_bytes, upstream_request_transmitted_body_bytes,
    upstream_request_header_bytes_approx, upstream_response_body_bytes,
    upstream_response_header_bytes_approx, compact_support_status, compact_support_reason
)
VALUES (
    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
    ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31,
    ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41, ?42
)
