fn group_routing_rule_from_group_list_row(
    row: &UpstreamAccountGroupListRow,
    upstream_429_retry_enabled: bool,
    upstream_429_max_retries: u8,
) -> GroupAccountRoutingRule {
    group_routing_rule_from_columns(GroupRoutingRuleColumns {
        legacy_concurrency_limit: row.concurrency_limit.unwrap_or_default(),
        legacy_upstream_429_retry_enabled: upstream_429_retry_enabled,
        legacy_upstream_429_max_retries: upstream_429_max_retries,
        policy_allow_cut_out: row.policy_allow_cut_out,
        policy_allow_cut_in: row.policy_allow_cut_in,
        policy_priority_tier: row.policy_priority_tier.as_deref(),
        policy_fast_mode_rewrite_mode: row.policy_fast_mode_rewrite_mode.as_deref(),
        policy_image_tool_rewrite_mode: row.policy_image_tool_rewrite_mode.as_deref(),
        policy_codex_imagegen_rewrite_mode: row.policy_codex_imagegen_rewrite_mode.as_deref(),
        policy_request_compression_algorithm: row.policy_request_compression_algorithm.as_deref(),
        policy_concurrency_limit: row.policy_concurrency_limit,
        policy_upstream_429_retry_enabled: row.policy_upstream_429_retry_enabled,
        policy_upstream_429_max_retries: row.policy_upstream_429_max_retries,
        policy_available_models_json: row.policy_available_models_json.as_deref(),
        policy_available_models_mode: row.policy_available_models_mode.as_deref(),
        policy_status_change_upstream_http_401: row.policy_status_change_upstream_http_401,
        policy_status_change_upstream_http_402: row.policy_status_change_upstream_http_402,
        policy_status_change_upstream_http_403: row.policy_status_change_upstream_http_403,
        policy_status_change_reauth_required: row.policy_status_change_reauth_required,
        policy_status_change_upstream_http_429_rate_limit: row
            .policy_status_change_upstream_http_429_rate_limit,
        policy_status_change_upstream_http_429_quota_exhausted: row
            .policy_status_change_upstream_http_429_quota_exhausted,
        policy_status_change_usage_snapshot_exhausted: row
            .policy_status_change_usage_snapshot_exhausted,
        policy_status_change_quota_still_exhausted: row.policy_status_change_quota_still_exhausted,
        policy_status_change_transport_failure: row.policy_status_change_transport_failure,
        policy_status_change_upstream_server_overloaded: row
            .policy_status_change_upstream_server_overloaded,
        policy_status_change_upstream_http_5xx: row.policy_status_change_upstream_http_5xx,
        policy_responses_first_byte_timeout_secs: row.policy_responses_first_byte_timeout_secs,
        policy_compact_first_byte_timeout_secs: row.policy_compact_first_byte_timeout_secs,
        policy_image_first_byte_timeout_secs: row.policy_image_first_byte_timeout_secs,
        policy_responses_stream_timeout_secs: row.policy_responses_stream_timeout_secs,
        policy_compact_stream_timeout_secs: row.policy_compact_stream_timeout_secs,
    })
}
