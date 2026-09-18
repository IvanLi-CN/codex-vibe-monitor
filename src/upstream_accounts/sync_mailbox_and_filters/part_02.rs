pub(crate) struct GroupRoutingRuleColumns<'a> {
    pub(crate) legacy_concurrency_limit: i64,
    pub(crate) legacy_upstream_429_retry_enabled: bool,
    pub(crate) legacy_upstream_429_max_retries: u8,
    pub(crate) policy_allow_cut_out: Option<i64>,
    pub(crate) policy_allow_cut_in: Option<i64>,
    pub(crate) policy_priority_tier: Option<&'a str>,
    pub(crate) policy_fast_mode_rewrite_mode: Option<&'a str>,
    pub(crate) policy_image_tool_rewrite_mode: Option<&'a str>,
    pub(crate) policy_codex_imagegen_rewrite_mode: Option<&'a str>,
    pub(crate) policy_request_compression_algorithm: Option<&'a str>,
    pub(crate) policy_concurrency_limit: Option<i64>,
    pub(crate) policy_upstream_429_retry_enabled: Option<i64>,
    pub(crate) policy_upstream_429_max_retries: Option<i64>,
    pub(crate) policy_available_models_json: Option<&'a str>,
    pub(crate) policy_available_models_mode: Option<&'a str>,
    pub(crate) policy_status_change_upstream_http_401: Option<i64>,
    pub(crate) policy_status_change_upstream_http_402: Option<i64>,
    pub(crate) policy_status_change_upstream_http_403: Option<i64>,
    pub(crate) policy_status_change_reauth_required: Option<i64>,
    pub(crate) policy_status_change_upstream_http_429_rate_limit: Option<i64>,
    pub(crate) policy_status_change_upstream_http_429_quota_exhausted: Option<i64>,
    pub(crate) policy_status_change_usage_snapshot_exhausted: Option<i64>,
    pub(crate) policy_status_change_quota_still_exhausted: Option<i64>,
    pub(crate) policy_status_change_transport_failure: Option<i64>,
    pub(crate) policy_status_change_upstream_server_overloaded: Option<i64>,
    pub(crate) policy_status_change_upstream_http_5xx: Option<i64>,
    pub(crate) policy_responses_first_byte_timeout_secs: Option<i64>,
    pub(crate) policy_compact_first_byte_timeout_secs: Option<i64>,
    pub(crate) policy_image_first_byte_timeout_secs: Option<i64>,
    pub(crate) policy_responses_stream_timeout_secs: Option<i64>,
    pub(crate) policy_compact_stream_timeout_secs: Option<i64>,
}

fn empty_group_routing_rule_columns() -> GroupRoutingRuleColumns<'static> {
    GroupRoutingRuleColumns {
        legacy_concurrency_limit: 0,
        legacy_upstream_429_retry_enabled: false,
        legacy_upstream_429_max_retries: 0,
        policy_allow_cut_out: None,
        policy_allow_cut_in: None,
        policy_priority_tier: None,
        policy_fast_mode_rewrite_mode: None,
        policy_image_tool_rewrite_mode: None,
        policy_codex_imagegen_rewrite_mode: None,
        policy_request_compression_algorithm: None,
        policy_concurrency_limit: None,
        policy_upstream_429_retry_enabled: None,
        policy_upstream_429_max_retries: None,
        policy_available_models_json: None,
        policy_available_models_mode: None,
        policy_status_change_upstream_http_401: None,
        policy_status_change_upstream_http_402: None,
        policy_status_change_upstream_http_403: None,
        policy_status_change_reauth_required: None,
        policy_status_change_upstream_http_429_rate_limit: None,
        policy_status_change_upstream_http_429_quota_exhausted: None,
        policy_status_change_usage_snapshot_exhausted: None,
        policy_status_change_quota_still_exhausted: None,
        policy_status_change_transport_failure: None,
        policy_status_change_upstream_server_overloaded: None,
        policy_status_change_upstream_http_5xx: None,
        policy_responses_first_byte_timeout_secs: None,
        policy_compact_first_byte_timeout_secs: None,
        policy_image_first_byte_timeout_secs: None,
        policy_responses_stream_timeout_secs: None,
        policy_compact_stream_timeout_secs: None,
    }
}

fn decode_group_legacy_routing_state(
    retry_raw: Option<i64>,
    max_retries_raw: Option<i64>,
) -> (bool, u8) {
    let retry_enabled = decode_group_upstream_429_retry_enabled(retry_raw.unwrap_or_default());
    let max_retries = normalize_group_upstream_429_retry_metadata(
        retry_enabled,
        decode_group_upstream_429_max_retries(max_retries_raw.unwrap_or_default()),
    );
    (retry_enabled, max_retries)
}

fn resolve_legacy_upstream_429_retry_enabled(
    policy_value: Option<i64>,
    legacy_value: bool,
) -> bool {
    policy_value.map(|value| value != 0).unwrap_or(legacy_value)
}

fn status_change_reasons_from_rule_columns(
    columns: &GroupRoutingRuleColumns<'_>,
) -> StatusChangeReasonSettings {
    status_change_reasons_from_columns(
        columns.policy_status_change_upstream_http_401,
        columns.policy_status_change_upstream_http_402,
        columns.policy_status_change_upstream_http_403,
        columns.policy_status_change_reauth_required,
        columns.policy_status_change_upstream_http_429_rate_limit,
        columns.policy_status_change_upstream_http_429_quota_exhausted,
        columns.policy_status_change_usage_snapshot_exhausted,
        columns.policy_status_change_quota_still_exhausted,
        columns.policy_status_change_transport_failure,
        columns.policy_status_change_upstream_server_overloaded,
        columns.policy_status_change_upstream_http_5xx,
    )
}

pub(crate) fn group_routing_rule_from_columns(
    columns: GroupRoutingRuleColumns<'_>,
) -> GroupAccountRoutingRule {
    let GroupRoutingRuleColumns {
        legacy_concurrency_limit,
        legacy_upstream_429_retry_enabled,
        legacy_upstream_429_max_retries,
        policy_allow_cut_out,
        policy_allow_cut_in,
        policy_priority_tier,
        policy_fast_mode_rewrite_mode,
        policy_image_tool_rewrite_mode,
        policy_codex_imagegen_rewrite_mode,
        policy_request_compression_algorithm,
        policy_concurrency_limit,
        policy_upstream_429_retry_enabled,
        policy_upstream_429_max_retries,
        policy_available_models_json,
        policy_available_models_mode,
        policy_responses_first_byte_timeout_secs,
        policy_compact_first_byte_timeout_secs,
        policy_image_first_byte_timeout_secs,
        policy_responses_stream_timeout_secs,
        policy_compact_stream_timeout_secs,
        ..
    } = columns;
    let upstream_429_retry_enabled = resolve_legacy_upstream_429_retry_enabled(
        policy_upstream_429_retry_enabled,
        legacy_upstream_429_retry_enabled,
    );
    GroupAccountRoutingRule {
        allow_cut_out: policy_allow_cut_out.map(|value| value != 0).unwrap_or(true),
        allow_cut_in: policy_allow_cut_in.map(|value| value != 0).unwrap_or(true),
        priority_tier: decode_tag_priority_tier(policy_priority_tier.unwrap_or("normal")),
        fast_mode_rewrite_mode: decode_tag_fast_mode_rewrite_mode(
            policy_fast_mode_rewrite_mode.unwrap_or("keep_original"),
        ),
        image_tool_rewrite_mode: decode_image_tool_rewrite_mode(
            policy_image_tool_rewrite_mode.unwrap_or("keep_original"),
        ),
        codex_imagegen_rewrite_mode: policy_codex_imagegen_rewrite_mode
            .map(decode_codex_imagegen_rewrite_mode),
        request_compression_algorithm: policy_request_compression_algorithm
            .map(decode_request_compression_algorithm),
        concurrency_limit: policy_concurrency_limit.unwrap_or(legacy_concurrency_limit),
        upstream_429_retry_enabled,
        upstream_429_max_retries: normalize_group_upstream_429_retry_metadata(
            upstream_429_retry_enabled,
            policy_upstream_429_max_retries
                .map(decode_group_upstream_429_max_retries)
                .unwrap_or(legacy_upstream_429_max_retries),
        ),
        available_models: if policy_available_models_json
            .is_some_and(|raw| serde_json::from_str::<Vec<String>>(raw).is_err())
        {
            Vec::new()
        } else {
            parse_string_array_json(policy_available_models_json)
        },
        available_models_mode: if policy_available_models_json
            .is_some_and(|raw| serde_json::from_str::<Vec<String>>(raw).is_err())
        {
            Some(AvailableModelsMode::Allowlist)
        } else {
            policy_available_models_mode
                .map(|value| AvailableModelsMode::from_str(Some(value)))
                .or_else(|| policy_available_models_json.map(|_| AvailableModelsMode::Allowlist))
        },
        available_models_defined: policy_available_models_json.is_some()
            || policy_available_models_mode.is_some(),
        status_change_reasons: status_change_reasons_from_rule_columns(&columns),
        timeouts: routing_timeout_settings_from_columns(
            policy_responses_first_byte_timeout_secs,
            policy_compact_first_byte_timeout_secs,
            policy_image_first_byte_timeout_secs,
            policy_responses_stream_timeout_secs,
            policy_compact_stream_timeout_secs,
        ),
    }
}
