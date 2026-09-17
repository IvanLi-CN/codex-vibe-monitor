pub(crate) fn resolve_prompt_cache_conversation_chart_range_start(
    range_end: DateTime<Utc>,
    earliest_created_at: Option<&str>,
) -> String {
    let floor = range_end - ChronoDuration::hours(PROMPT_CACHE_CONVERSATION_CHART_MAX_HOURS);
    let created_at = earliest_created_at
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|value| value.with_timezone(&Utc));
    let chart_start = match created_at {
        Some(created_at) if created_at > floor => created_at,
        _ => floor,
    };
    format_utc_iso(chart_start)
}

pub(crate) fn normalize_trimmed_optional_string(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn prompt_cache_invocation_preview_from_row(
    row: PromptCacheConversationInvocationPreviewRow,
) -> PromptCacheConversationInvocationPreviewResponse {
    PromptCacheConversationInvocationPreviewResponse {
        id: row.id,
        invoke_id: row.invoke_id,
        prompt_cache_key: Some(row.prompt_cache_key),
        occurred_at: row.occurred_at,
        status: row.status,
        live_phase: normalize_trimmed_optional_string(row.live_phase),
        failure_class: normalize_trimmed_optional_string(row.failure_class),
        route_mode: normalize_trimmed_optional_string(row.route_mode),
        model: normalize_trimmed_optional_string(row.model),
        request_model: normalize_trimmed_optional_string(row.request_model),
        response_model: normalize_trimmed_optional_string(row.response_model),
        total_tokens: row.total_tokens.max(0),
        cost: row.cost,
        proxy_display_name: normalize_trimmed_optional_string(row.proxy_display_name),
        upstream_account_id: row.upstream_account_id,
        upstream_account_name: normalize_trimmed_optional_string(row.upstream_account_name),
        upstream_account_plan_type: normalize_trimmed_optional_string(
            row.upstream_account_plan_type,
        ),
        endpoint: normalize_trimmed_optional_string(row.endpoint),
        compaction_request_kind: normalize_trimmed_optional_string(row.compaction_request_kind),
        compaction_response_kind: normalize_trimmed_optional_string(row.compaction_response_kind),
        image_intent: normalize_trimmed_optional_string(row.image_intent),
        source: normalize_trimmed_optional_string(row.source),
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_input_tokens: row.cache_input_tokens,
        reasoning_tokens: row.reasoning_tokens,
        reasoning_effort: normalize_trimmed_optional_string(row.reasoning_effort),
        error_message: normalize_trimmed_optional_string(row.error_message),
        downstream_status_code: row.downstream_status_code,
        downstream_error_message: normalize_trimmed_optional_string(row.downstream_error_message),
        failure_kind: normalize_trimmed_optional_string(row.failure_kind),
        blocked_binding: None,
        is_actionable: row.is_actionable.map(|value| value != 0),
        response_content_encoding: normalize_trimmed_optional_string(row.response_content_encoding),
        request_compression_algorithm: normalize_trimmed_optional_string(
            row.request_compression_algorithm,
        ),
        transport: normalize_trimmed_optional_string(row.transport),
        requested_service_tier: normalize_trimmed_optional_string(row.requested_service_tier),
        service_tier: normalize_trimmed_optional_string(row.service_tier),
        billing_service_tier: normalize_trimmed_optional_string(row.billing_service_tier),
        t_req_read_ms: row.t_req_read_ms,
        t_req_parse_ms: row.t_req_parse_ms,
        t_upstream_connect_ms: row.t_upstream_connect_ms,
        t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
        first_token_ms: row.first_token_ms,
        t_upstream_stream_ms: row.t_upstream_stream_ms,
        t_resp_parse_ms: row.t_resp_parse_ms,
        t_persist_ms: row.t_persist_ms,
        t_total_ms: row.t_total_ms,
    }
}

pub(crate) fn upstream_account_invocation_preview_from_row(
    row: UpstreamAccountInvocationPreviewRow,
) -> PromptCacheConversationInvocationPreviewResponse {
    PromptCacheConversationInvocationPreviewResponse {
        id: row.id,
        invoke_id: row.invoke_id,
        prompt_cache_key: row.prompt_cache_key,
        occurred_at: row.occurred_at,
        status: row.status,
        live_phase: normalize_trimmed_optional_string(row.live_phase),
        failure_class: normalize_trimmed_optional_string(row.failure_class),
        route_mode: normalize_trimmed_optional_string(row.route_mode),
        model: normalize_trimmed_optional_string(row.model),
        request_model: normalize_trimmed_optional_string(row.request_model),
        response_model: normalize_trimmed_optional_string(row.response_model),
        total_tokens: row.total_tokens.max(0),
        cost: row.cost,
        proxy_display_name: normalize_trimmed_optional_string(row.proxy_display_name),
        upstream_account_id: row.upstream_account_id,
        upstream_account_name: normalize_trimmed_optional_string(row.upstream_account_name),
        upstream_account_plan_type: normalize_trimmed_optional_string(
            row.upstream_account_plan_type,
        ),
        endpoint: normalize_trimmed_optional_string(row.endpoint),
        compaction_request_kind: normalize_trimmed_optional_string(row.compaction_request_kind),
        compaction_response_kind: normalize_trimmed_optional_string(row.compaction_response_kind),
        image_intent: normalize_trimmed_optional_string(row.image_intent),
        source: normalize_trimmed_optional_string(row.source),
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_input_tokens: row.cache_input_tokens,
        reasoning_tokens: row.reasoning_tokens,
        reasoning_effort: normalize_trimmed_optional_string(row.reasoning_effort),
        error_message: normalize_trimmed_optional_string(row.error_message),
        downstream_status_code: row.downstream_status_code,
        downstream_error_message: normalize_trimmed_optional_string(row.downstream_error_message),
        failure_kind: normalize_trimmed_optional_string(row.failure_kind),
        blocked_binding: None,
        is_actionable: row.is_actionable.map(|value| value != 0),
        response_content_encoding: normalize_trimmed_optional_string(row.response_content_encoding),
        request_compression_algorithm: normalize_trimmed_optional_string(
            row.request_compression_algorithm,
        ),
        transport: normalize_trimmed_optional_string(row.transport),
        requested_service_tier: normalize_trimmed_optional_string(row.requested_service_tier),
        service_tier: normalize_trimmed_optional_string(row.service_tier),
        billing_service_tier: normalize_trimmed_optional_string(row.billing_service_tier),
        t_req_read_ms: row.t_req_read_ms,
        t_req_parse_ms: row.t_req_parse_ms,
        t_upstream_connect_ms: row.t_upstream_connect_ms,
        t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
        first_token_ms: row.first_token_ms,
        t_upstream_stream_ms: row.t_upstream_stream_ms,
        t_resp_parse_ms: row.t_resp_parse_ms,
        t_persist_ms: row.t_persist_ms,
        t_total_ms: row.t_total_ms,
    }
}

pub(crate) fn resolve_prompt_cache_upstream_account_label(
    upstream_account_name: Option<&str>,
    upstream_account_id: Option<i64>,
) -> String {
    if let Some(name) = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return name.to_string();
    }
    if let Some(account_id) = upstream_account_id {
        return format!("账号 #{account_id}");
    }
    "—".to_string()
}

pub(crate) fn resolve_prompt_cache_upstream_account_group_key(
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
) -> String {
    if let Some(account_id) = upstream_account_id {
        return format!("id:{account_id}");
    }
    if let Some(name) = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("name:{name}");
    }
    "unknown".to_string()
}

#[cfg(test)]
mod runtime_preview_progress_tests {
    use super::*;

    #[test]
    fn runtime_first_token_progress_overlays_the_stale_persisted_preview() {
        let (first_token_ms, live_phase) = merged_runtime_preview_progress(
            None,
            Some(INVOCATION_LIVE_PHASE_REQUESTING),
            Some(720.0),
            Some(INVOCATION_LIVE_PHASE_RESPONDING),
            false,
        );

        assert_eq!(first_token_ms, Some(720.0));
        assert_eq!(
            live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn missing_runtime_timing_does_not_regress_a_persisted_responding_preview() {
        let (first_token_ms, live_phase) = merged_runtime_preview_progress(
            Some(720.0),
            Some(INVOCATION_LIVE_PHASE_RESPONDING),
            None,
            Some(INVOCATION_LIVE_PHASE_REQUESTING),
            false,
        );

        assert_eq!(first_token_ms, Some(720.0));
        assert_eq!(
            live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn zero_millisecond_runtime_first_token_promotes_a_missing_preview() {
        let (first_token_ms, live_phase) = merged_runtime_preview_progress(
            None,
            Some(INVOCATION_LIVE_PHASE_REQUESTING),
            Some(0.0),
            Some(INVOCATION_LIVE_PHASE_RESPONDING),
            false,
        );

        assert_eq!(first_token_ms, Some(0.0));
        assert_eq!(
            live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn invalid_persisted_first_token_is_treated_as_unavailable() {
        for persisted in [Some(-1.0), Some(f64::INFINITY), Some(f64::NAN)] {
            let (first_token_ms, live_phase) = merged_runtime_preview_progress(
                persisted,
                Some(INVOCATION_LIVE_PHASE_RESPONDING),
                None,
                Some(INVOCATION_LIVE_PHASE_REQUESTING),
                false,
            );

            assert_eq!(first_token_ms, None);
            assert_eq!(live_phase, None);
        }
    }

    #[test]
    fn retry_runtime_progress_does_not_restore_persisted_attempt_timing() {
        let (first_token_ms, live_phase) = merged_runtime_preview_progress(
            Some(720.0),
            Some(INVOCATION_LIVE_PHASE_RESPONDING),
            None,
            Some(INVOCATION_LIVE_PHASE_REQUESTING),
            true,
        );

        assert_eq!(first_token_ms, None);
        assert_eq!(
            live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_REQUESTING)
        );
    }
}
