struct PromptCacheManualBindingResponseByKey {
    prompt_cache_key: String,
    response: PromptCacheConversationManualBindingResponse,
}

fn prompt_cache_manual_binding_response_from_row(
    row: PromptCacheConversationManualBindingSummaryRow,
) -> Option<PromptCacheManualBindingResponseByKey> {
    let prompt_cache_key = row.prompt_cache_key.trim().to_string();
    if prompt_cache_key.is_empty() {
        return None;
    }

    match row.binding_kind.as_str() {
        PROMPT_CACHE_BINDING_KIND_GROUP => {
            let group_name = normalize_trimmed_optional_string(row.group_name)?;
            Some(PromptCacheManualBindingResponseByKey {
                prompt_cache_key,
                response: PromptCacheConversationManualBindingResponse {
                    binding_kind: "group".to_string(),
                    group_name: Some(group_name),
                    upstream_account_id: None,
                    upstream_account_name: None,
                },
            })
        }
        PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => {
            let upstream_account_name =
                normalize_trimmed_optional_string(row.upstream_account_name.clone());
            let upstream_account_id = row.upstream_account_id;
            if upstream_account_name.is_none() && upstream_account_id.is_none() {
                return None;
            }
            Some(PromptCacheManualBindingResponseByKey {
                prompt_cache_key,
                response: PromptCacheConversationManualBindingResponse {
                    binding_kind: "upstreamAccount".to_string(),
                    group_name: None,
                    upstream_account_id,
                    upstream_account_name,
                },
            })
        }
        _ => None,
    }
}

pub(crate) fn overlay_runtime_prompt_cache_invocation_previews(
    grouped_recent_invocations: &mut HashMap<
        String,
        Vec<PromptCacheConversationInvocationPreviewResponse>,
    >,
    runtime_overlay_records: &[ApiInvocation],
    selected_keys: &[String],
    recent_invocation_limit: i64,
) {
    if runtime_overlay_records.is_empty()
        || selected_keys.is_empty()
        || recent_invocation_limit <= 0
    {
        return;
    }
    let selected_keys = selected_keys.iter().collect::<HashSet<_>>();
    for record in runtime_overlay_records {
        let Some(prompt_cache_key) =
            normalize_trimmed_optional_string(record.prompt_cache_key.clone())
        else {
            continue;
        };
        if !selected_keys.contains(&prompt_cache_key) {
            continue;
        }
        let previews = grouped_recent_invocations
            .entry(prompt_cache_key.clone())
            .or_default();
        if let Some(preview) = previews.iter_mut().find(|preview| {
            preview.invoke_id == record.invoke_id && preview.occurred_at == record.occurred_at
        }) {
            overlay_runtime_preview_progress(preview, record);
            continue;
        }
        previews.push(prompt_cache_invocation_preview_from_runtime_record(
            record,
            prompt_cache_key,
        ));
    }

    for previews in grouped_recent_invocations.values_mut() {
        previews.sort_by(|left, right| {
            right
                .occurred_at
                .cmp(&left.occurred_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        previews.truncate(recent_invocation_limit as usize);
    }
}

fn overlay_runtime_preview_progress(
    preview: &mut PromptCacheConversationInvocationPreviewResponse,
    record: &ApiInvocation,
) {
    let (first_token_ms, live_phase) = merged_runtime_preview_progress(
        preview.first_token_ms,
        preview.live_phase.as_deref(),
        runtime_record_first_token_ms(record),
        runtime_record_live_phase(record),
        runtime_record_is_retry(record),
    );
    preview.first_token_ms = first_token_ms;
    preview.live_phase = live_phase;
}

fn merged_runtime_preview_progress(
    persisted_first_token_ms: Option<f64>,
    persisted_live_phase: Option<&str>,
    runtime_first_token_ms: Option<f64>,
    runtime_live_phase: Option<&str>,
    suppress_persisted_progress: bool,
) -> (Option<f64>, Option<String>) {
    let measured_persisted_first_token_ms =
        persisted_first_token_ms.filter(|value| value.is_finite() && *value >= 0.0);
    let measured_runtime_first_token_ms =
        runtime_first_token_ms.filter(|value| value.is_finite() && *value >= 0.0);
    let runtime_is_responding = measured_runtime_first_token_ms.is_some()
        && runtime_live_phase == Some(INVOCATION_LIVE_PHASE_RESPONDING);

    let first_token_ms = measured_runtime_first_token_ms.or_else(|| {
        (!suppress_persisted_progress)
            .then_some(measured_persisted_first_token_ms)
            .flatten()
    });
    let live_phase = runtime_is_responding
        .then_some(INVOCATION_LIVE_PHASE_RESPONDING.to_string())
        .or_else(|| {
            if suppress_persisted_progress {
                return runtime_live_phase.map(str::to_string);
            }
            if persisted_live_phase == Some(INVOCATION_LIVE_PHASE_RESPONDING)
                && first_token_ms.is_none()
            {
                None
            } else {
                persisted_live_phase.map(str::to_string)
            }
        });

    (first_token_ms, live_phase)
}

pub(crate) fn prompt_cache_invocation_preview_from_runtime_record(
    record: &ApiInvocation,
    prompt_cache_key: String,
) -> PromptCacheConversationInvocationPreviewResponse {
    let mut preview = invocation_preview_from_runtime_record(record);
    preview.prompt_cache_key = Some(prompt_cache_key);
    preview
}

pub(crate) fn invocation_preview_from_runtime_record(
    record: &ApiInvocation,
) -> PromptCacheConversationInvocationPreviewResponse {
    PromptCacheConversationInvocationPreviewResponse {
        id: record.id,
        invoke_id: record.invoke_id.clone(),
        prompt_cache_key: record.prompt_cache_key.clone(),
        occurred_at: record.occurred_at.clone(),
        status: record
            .status
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        live_phase: runtime_record_live_phase(record).map(str::to_string),
        failure_class: normalize_trimmed_optional_string(record.failure_class.clone()),
        route_mode: normalize_trimmed_optional_string(record.route_mode.clone()),
        model: normalize_trimmed_optional_string(record.model.clone()),
        request_model: normalize_trimmed_optional_string(record.request_model.clone()),
        response_model: normalize_trimmed_optional_string(record.response_model.clone()),
        total_tokens: record.total_tokens.unwrap_or_default().max(0),
        cost: record.cost,
        proxy_display_name: normalize_trimmed_optional_string(record.proxy_display_name.clone()),
        upstream_account_id: record.upstream_account_id,
        upstream_account_name: normalize_trimmed_optional_string(
            record.upstream_account_name.clone(),
        ),
        upstream_account_plan_type: None,
        endpoint: normalize_trimmed_optional_string(record.endpoint.clone()),
        compaction_request_kind: normalize_trimmed_optional_string(
            record.compaction_request_kind.clone(),
        ),
        compaction_response_kind: normalize_trimmed_optional_string(
            record.compaction_response_kind.clone(),
        ),
        image_intent: normalize_trimmed_optional_string(record.image_intent.clone()),
        source: normalize_trimmed_optional_string(Some(record.source.clone())),
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cache_input_tokens: record.cache_input_tokens,
        reasoning_tokens: record.reasoning_tokens,
        reasoning_effort: normalize_trimmed_optional_string(record.reasoning_effort.clone()),
        error_message: normalize_trimmed_optional_string(record.error_message.clone()),
        downstream_status_code: record.downstream_status_code,
        downstream_error_message: normalize_trimmed_optional_string(
            record.downstream_error_message.clone(),
        ),
        failure_kind: normalize_trimmed_optional_string(record.failure_kind.clone()),
        blocked_binding: record.blocked_binding.clone(),
        is_actionable: record.is_actionable,
        response_content_encoding: normalize_trimmed_optional_string(
            record.response_content_encoding.clone(),
        ),
        request_compression_algorithm: normalize_trimmed_optional_string(
            record.request_compression_algorithm.clone(),
        ),
        transport: normalize_trimmed_optional_string(record.transport.clone()),
        requested_service_tier: normalize_trimmed_optional_string(
            record.requested_service_tier.clone(),
        ),
        service_tier: normalize_trimmed_optional_string(record.service_tier.clone()),
        billing_service_tier: normalize_trimmed_optional_string(
            record.billing_service_tier.clone(),
        ),
        t_req_read_ms: record.t_req_read_ms,
        t_req_parse_ms: record.t_req_parse_ms,
        t_upstream_connect_ms: record.t_upstream_connect_ms,
        t_upstream_ttfb_ms: record.t_upstream_ttfb_ms,
        first_token_ms: runtime_record_first_token_ms(record),
        t_upstream_stream_ms: record.t_upstream_stream_ms,
        t_resp_parse_ms: record.t_resp_parse_ms,
        t_persist_ms: record.t_persist_ms,
        t_total_ms: record.t_total_ms,
    }
}
