pub(crate) const INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD: f64 = 0.000001;
pub(crate) const INVOCATION_COST_AUDIT_REASON_RECORDED_COST_MISSING: &str = "recorded_cost_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_RECORDED_PRICE_VERSION_MISSING: &str =
    "recorded_price_version_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_PRICING_MODE_UNKNOWN: &str = "pricing_mode_unknown";
pub(crate) const INVOCATION_COST_AUDIT_REASON_USAGE_MISSING: &str = "usage_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_MODEL_MISSING: &str = "model_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_MODEL_PRICING_MISSING: &str = "model_pricing_missing";
pub(crate) const INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED: &str = "price_version_changed";
pub(crate) const INVOCATION_COST_AUDIT_REASON_TOTAL_MISMATCH: &str = "total_mismatch";

fn resolve_invocation_cache_write_tokens(record: &ApiInvocation) -> Option<i64> {
    record.cache_write_tokens.or_else(|| {
        record
            .input_tokens
            .map(|input| input.saturating_sub(record.cache_input_tokens.unwrap_or_default().max(0)))
    })
}

fn invocation_has_usage_evidence(record: &ApiInvocation) -> bool {
    [
        record.input_tokens,
        record.output_tokens,
        record.cache_input_tokens,
        record.reasoning_tokens,
        record.total_tokens,
        resolve_invocation_cache_write_tokens(record),
    ]
    .into_iter()
    .any(|value| value.is_some())
        || record.cost.is_some()
        || record.cost_input.is_some()
        || record.cost_cache_write.is_some()
        || record.cost_cache_read.is_some()
        || record.cost_output.is_some()
        || record.cost_reasoning.is_some()
}

fn invocation_usage_for_cost_audit(record: &ApiInvocation) -> ParsedUsage {
    ParsedUsage {
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cache_input_tokens: record.cache_input_tokens,
        reasoning_tokens: record.reasoning_tokens,
        total_tokens: record.total_tokens,
    }
}

fn invocation_billable_model(record: &ApiInvocation) -> Option<&str> {
    record
        .response_model
        .as_deref()
        .or(record.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn resolve_invocation_pricing_mode(
    price_version: Option<&str>,
) -> Result<ProxyPricingMode, &'static str> {
    let normalized = price_version
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(INVOCATION_COST_AUDIT_REASON_RECORDED_PRICE_VERSION_MISSING)?;
    if normalized.ends_with(REQUESTED_TIER_PRICE_VERSION_SUFFIX) {
        return Ok(ProxyPricingMode::RequestedTier);
    }
    if normalized.ends_with(RESPONSE_TIER_PRICE_VERSION_SUFFIX) {
        return Ok(ProxyPricingMode::ResponseTier);
    }
    if normalized.ends_with(EXPLICIT_BILLING_PRICE_VERSION_SUFFIX) {
        return Ok(ProxyPricingMode::ExplicitBilling);
    }
    Err(INVOCATION_COST_AUDIT_REASON_PRICING_MODE_UNKNOWN)
}

fn build_invocation_cost_audit_breakdown(
    total: Option<f64>,
    input: Option<f64>,
    cache_write: Option<f64>,
    cache_read: Option<f64>,
    output: Option<f64>,
    reasoning: Option<f64>,
    include_breakdown: bool,
) -> Option<InvocationCostAuditBreakdown> {
    let has_components = [input, cache_write, cache_read, output, reasoning]
        .into_iter()
        .any(|value| value.is_some());
    if total.is_none() && !(include_breakdown && has_components) {
        return None;
    }
    Some(InvocationCostAuditBreakdown {
        input: include_breakdown.then_some(input).flatten(),
        cache_write: include_breakdown.then_some(cache_write).flatten(),
        cache_read: include_breakdown.then_some(cache_read).flatten(),
        output: include_breakdown.then_some(output).flatten(),
        reasoning: include_breakdown.then_some(reasoning).flatten(),
        total,
    })
}

fn build_recorded_invocation_cost_breakdown(
    record: &ApiInvocation,
    include_breakdown: bool,
) -> Option<InvocationCostAuditBreakdown> {
    build_invocation_cost_audit_breakdown(
        record.cost,
        record.cost_input,
        record.cost_cache_write,
        record.cost_cache_read,
        record.cost_output,
        record.cost_reasoning,
        include_breakdown,
    )
}

fn build_local_invocation_cost_breakdown(
    record: &ApiInvocation,
    catalog: &PricingCatalog,
    include_breakdown: bool,
) -> (
    Option<InvocationCostAuditBreakdown>,
    Option<String>,
    Option<&'static str>,
) {
    let pricing_mode = match resolve_invocation_pricing_mode(record.price_version.as_deref()) {
        Ok(mode) => mode,
        Err(reason) => return (None, None, Some(reason)),
    };
    let local_price_version = Some(proxy_price_version(&catalog.version, pricing_mode));
    let usage = invocation_usage_for_cost_audit(record);
    if !has_billable_usage(&usage) {
        return (
            None,
            local_price_version,
            Some(INVOCATION_COST_AUDIT_REASON_USAGE_MISSING),
        );
    }
    let Some(model) = invocation_billable_model(record) else {
        return (
            None,
            local_price_version,
            Some(INVOCATION_COST_AUDIT_REASON_MODEL_MISSING),
        );
    };
    let (breakdown, _, _) = estimate_proxy_cost_breakdown(
        catalog,
        Some(model),
        &usage,
        record.billing_service_tier.as_deref(),
        pricing_mode,
    );
    let Some(breakdown) = breakdown else {
        return (
            None,
            local_price_version,
            Some(INVOCATION_COST_AUDIT_REASON_MODEL_PRICING_MISSING),
        );
    };
    (
        build_invocation_cost_audit_breakdown(
            Some(breakdown.total()),
            Some(breakdown.input),
            Some(breakdown.cache_write),
            Some(breakdown.cache_read),
            Some(breakdown.output),
            Some(breakdown.reasoning),
            include_breakdown,
        ),
        local_price_version,
        None,
    )
}

fn build_invocation_cost_audit(
    record: &ApiInvocation,
    catalog: &PricingCatalog,
    include_breakdown: bool,
) -> Option<InvocationCostAudit> {
    let recorded = build_recorded_invocation_cost_breakdown(record, include_breakdown);
    let recorded_total = recorded.as_ref().and_then(|value| value.total);
    let (local, local_price_version, unavailable_reason) =
        build_local_invocation_cost_breakdown(record, catalog, include_breakdown);
    let local_total = local.as_ref().and_then(|value| value.total);
    let absolute_diff_usd = match (recorded_total, local_total) {
        (Some(recorded_total), Some(local_total)) => Some((recorded_total - local_total).abs()),
        _ => None,
    };
    let mismatch =
        absolute_diff_usd.is_some_and(|diff| diff > INVOCATION_COST_AUDIT_MISMATCH_EPSILON_USD);
    let recorded_price_version = normalize_query_text(record.price_version.as_deref());
    let reason = if mismatch {
        if recorded_price_version.as_deref() != local_price_version.as_deref() {
            Some(INVOCATION_COST_AUDIT_REASON_PRICE_VERSION_CHANGED.to_string())
        } else {
            Some(INVOCATION_COST_AUDIT_REASON_TOTAL_MISMATCH.to_string())
        }
    } else if recorded_total.is_none() && local_total.is_some() {
        Some(INVOCATION_COST_AUDIT_REASON_RECORDED_COST_MISSING.to_string())
    } else if recorded_total.is_some() && local_total.is_none() {
        unavailable_reason.map(str::to_string)
    } else {
        None
    };
    if recorded.is_none()
        && local.is_none()
        && recorded_price_version.is_none()
        && local_price_version.is_none()
        && reason.is_none()
    {
        return None;
    }
    Some(InvocationCostAudit {
        recorded,
        local,
        mismatch,
        reason,
        absolute_diff_usd,
        recorded_price_version,
        local_price_version,
    })
}

fn apply_invocation_cost_audits(records: &mut [ApiInvocation], catalog: &PricingCatalog) {
    for record in records {
        record.cost_audit = build_invocation_cost_audit(record, catalog, false);
    }
}

fn build_invocation_usage_summary(
    record: &ApiInvocation,
    cost_audit: &InvocationCostAudit,
) -> Value {
    let recorded = cost_audit
        .recorded
        .as_ref()
        .map(|breakdown| json!(breakdown));
    let local = cost_audit.local.as_ref().map(|breakdown| json!(breakdown));
    json!({
        "inputTokens": record.input_tokens,
        "cacheWriteTokens": resolve_invocation_cache_write_tokens(record),
        "cacheInputTokens": record.cache_input_tokens,
        "outputTokens": record.output_tokens,
        "reasoningTokens": record.reasoning_tokens,
        "totalTokens": record.total_tokens,
        "cost": record.cost,
        "tokens": {
            "input": record.input_tokens,
            "cacheWrite": resolve_invocation_cache_write_tokens(record),
            "cacheRead": record.cache_input_tokens,
            "output": record.output_tokens,
            "reasoning": record.reasoning_tokens,
            "total": record.total_tokens,
        },
        "costs": {
            "recorded": recorded,
            "local": local,
        },
        "audit": cost_audit,
    })
}

fn build_workflow_hero(
    record: &ApiInvocation,
    payload: Option<&Value>,
    timeline_attempt_count: usize,
) -> InvocationWorkflowHero {
    InvocationWorkflowHero {
        record_id: record.id,
        invoke_id: record.invoke_id.clone(),
        prompt_cache_key: record.prompt_cache_key.clone(),
        route_mode: record.route_mode.clone(),
        endpoint: record.endpoint.clone(),
        request_model: record.request_model.clone(),
        response_model: record
            .response_model
            .clone()
            .or_else(|| record.model.clone()),
        final_status: record.status.clone(),
        failure_class: record.failure_class.clone(),
        downstream_status_code: record.downstream_status_code,
        upstream_account_id: record.upstream_account_id,
        upstream_account_name: record.upstream_account_name.clone(),
        total_duration_ms: record.t_total_ms,
        timeline_attempt_count,
        pool_attempt_count: record.pool_attempt_count,
        total_tokens: record.total_tokens,
        cost: record.cost,
        occurred_at: Some(record.occurred_at.clone()),
        pool_routing_no_candidate_audit: payload_clone(payload, &["poolRoutingNoCandidateAudit"])
            .and_then(|value| serde_json::from_value(value).ok()),
    }
}

fn build_attempt_request_summary(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
) -> Value {
    let compression = request_compression_value(
        attempt.upstream_request_compression_algorithm.clone(),
        attempt.upstream_request_compression_mode.clone(),
        derive_request_compression_fields(
            attempt.upstream_request_logical_body_bytes,
            attempt.upstream_request_transmitted_body_bytes,
            attempt.upstream_request_header_bytes_approx,
            attempt.upstream_response_body_bytes,
            attempt.upstream_response_header_bytes_approx,
            attempt.http_status.is_some()
                || attempt
                    .status
                    .eq_ignore_ascii_case(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS),
        ),
    );
    let image_tool_rewrite = payload_clone(payload, &["imageToolRewrite"]);
    let codex_imagegen_rewrite = payload_clone(payload, &["codexImagegenRewrite"]);
    let mut summary = json!({
        "endpoint": attempt.endpoint.clone(),
        "routeMode": record.route_mode.clone(),
        "transport": record.transport.clone(),
        "requestModel": record.request_model.clone(),
        "responseModel": record.response_model.as_ref().or(record.model.as_ref()),
        "requestedServiceTier": record.requested_service_tier.clone(),
        "reasoningEffort": record.reasoning_effort.clone(),
        "compactionRequestKind": record.compaction_request_kind.clone(),
        "imageIntent": record.image_intent.clone(),
        "promptCacheKey": record.prompt_cache_key.clone(),
        "stickyKey": attempt.sticky_key.as_ref().or(record.sticky_key.as_ref()),
        "requesterIp": attempt.requester_ip.as_ref().or(record.requester_ip.as_ref()),
        "account": {
            "id": attempt.upstream_account_id,
            "name": attempt.upstream_account_name.as_ref().or(record.upstream_account_name.as_ref()),
        },
        "routing": {
            "upstreamRouteKey": attempt.upstream_route_key.clone(),
            "proxyBindingKey": attempt.proxy_binding_key_snapshot.clone(),
            "proxyDisplayName": record.proxy_display_name.clone(),
            "upstreamScope": payload_string(payload, &["upstreamScope"]),
            "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
            "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
            "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
        },
        "headers": build_request_header_snapshot(payload),
        "client": build_request_client_snapshot(payload),
        "compression": compression,
        "bodyCapture": {
            "availableAtInvocationLevel": record.request_raw_path.is_some(),
            "size": record.request_raw_size,
            "truncated": record.request_raw_truncated.unwrap_or_default() != 0,
            "truncatedReason": record.request_raw_truncated_reason.clone(),
            "detailLevel": record.detail_level.clone(),
            "detailPruneReason": record.detail_prune_reason.clone(),
        },
    });
    if let Some(image_tool_rewrite) = image_tool_rewrite {
        summary["imageToolRewrite"] = image_tool_rewrite;
    }
    if let Some(codex_imagegen_rewrite) = codex_imagegen_rewrite {
        summary["codexImagegenRewrite"] = codex_imagegen_rewrite;
    }
    summary
}

fn build_attempt_response_summary(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
    is_final_attempt: bool,
) -> Value {
    let fields = build_attempt_response_summary_fields(record, attempt, payload, is_final_attempt);
    json!({
        "status": attempt.status.clone(),
        "phase": attempt.phase.clone(),
        "httpStatus": attempt.http_status,
        "compactionResponseKind": is_final_attempt.then(|| record.compaction_response_kind.clone()).flatten(),
        "failureKind": attempt.failure_kind.as_ref().or(fields.invocation_failure_kind.as_ref()),
        "errorMessage": attempt.error_message.as_ref().or(fields.invocation_error_message.as_ref()),
        "downstreamErrorMessage": attempt
            .downstream_error_message
            .as_ref()
            .or(fields.invocation_downstream_error_message.as_ref()),
        "upstreamRequestId": attempt.upstream_request_id.as_ref().or(fields.invocation_upstream_request_id.as_ref()),
        "upstreamErrorCode": is_final_attempt.then(|| record.upstream_error_code.clone()).flatten(),
        "upstreamErrorMessage": is_final_attempt.then(|| record.upstream_error_message.clone()).flatten(),
        "streamTerminalEvent": is_final_attempt.then(|| record.stream_terminal_event.clone()).flatten(),
        "responseContentEncoding": fields.invocation_response_content_encoding,
        "serviceTier": is_final_attempt.then(|| record.service_tier.clone()).flatten(),
        "billingServiceTier": is_final_attempt.then(|| record.billing_service_tier.clone()).flatten(),
        "headers": build_response_header_snapshot(record, Some(attempt), payload, is_final_attempt),
        "delivery": build_response_delivery_snapshot(fields.invocation_response_payload.as_ref()),
        "compactSupport": {
            "status": attempt.compact_support_status.clone(),
            "reason": attempt.compact_support_reason.clone(),
        },
        "latencyMs": {
            "connect": finite_nonnegative_timing(attempt.connect_latency_ms.or_else(|| is_final_attempt.then_some(record.t_upstream_connect_ms).flatten())),
            "firstByte": finite_nonnegative_timing(attempt.first_byte_latency_ms.or_else(|| is_final_attempt.then_some(record.t_upstream_ttfb_ms).flatten())),
            "stream": if is_final_attempt {
                final_attempt_has_stream_evidence(attempt)
                    .then_some(finite_positive_timing(attempt.stream_latency_ms))
                    .flatten()
            } else {
                finite_positive_timing(attempt.stream_latency_ms)
            },
            "requestRead": is_final_attempt.then_some(finite_nonnegative_timing(record.t_req_read_ms)).flatten(),
            "requestParse": is_final_attempt.then_some(finite_nonnegative_timing(record.t_req_parse_ms)).flatten(),
            "responseParse": is_final_attempt.then_some(finite_nonnegative_timing(record.t_resp_parse_ms)).flatten(),
            "persist": is_final_attempt.then_some(finite_nonnegative_timing(record.t_persist_ms)).flatten(),
            "total": is_final_attempt.then_some(finite_nonnegative_timing(record.t_total_ms)).flatten(),
        },
        "responseBodyCapture": {
            "availableAtAttemptLevel": fields.attempt_response_body_captured,
            "availableAtInvocationLevel": is_final_attempt && record.response_raw_path.is_some(),
            "size": fields.response_body_capture_size,
            "truncated": fields.response_body_capture_truncated,
            "truncatedReason": fields.response_body_capture_truncated_reason,
            "detailLevel": fields.response_body_capture_detail_level,
            "detailPruneReason": is_final_attempt.then(|| record.detail_prune_reason.clone()).flatten(),
            "unavailableReason": (!(fields.attempt_response_body_captured || (is_final_attempt && record.response_raw_path.is_some())))
                .then_some("attempt_response_body_not_captured"),
        },
        "usage": usage_cost_audit.map(|audit| build_invocation_usage_summary(record, audit)),
    })
}

struct AttemptResponseSummaryFields {
    invocation_failure_kind: Option<String>,
    invocation_error_message: Option<String>,
    invocation_downstream_error_message: Option<String>,
    invocation_upstream_request_id: Option<String>,
    invocation_response_content_encoding: Option<String>,
    invocation_response_payload: Option<Value>,
    attempt_response_body_captured: bool,
    response_body_capture_size: Option<i64>,
    response_body_capture_truncated: bool,
    response_body_capture_truncated_reason: Option<String>,
    response_body_capture_detail_level: Option<String>,
}

fn build_attempt_response_summary_fields(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
    is_final_attempt: bool,
) -> AttemptResponseSummaryFields {
    let attempt_response_body_captured = attempt.response_raw_path.is_some();
    let invocation_response_payload = is_final_attempt.then(|| payload.cloned()).flatten();
    let response_body_capture_truncated = if attempt_response_body_captured {
        attempt.response_raw_truncated.unwrap_or_default() != 0
    } else if is_final_attempt && record.response_raw_path.is_some() {
        record.response_raw_truncated.unwrap_or_default() != 0
    } else {
        false
    };
    let response_body_capture_detail_level = if attempt_response_body_captured {
        Some(record.detail_level.clone())
    } else if is_final_attempt && record.response_raw_path.is_some() {
        Some("full".to_string())
    } else {
        Some("attempt_metrics".to_string())
    };
    AttemptResponseSummaryFields {
        invocation_failure_kind: is_final_attempt
            .then(|| record.failure_kind.clone())
            .flatten(),
        invocation_error_message: is_final_attempt
            .then(|| record.error_message.clone())
            .flatten(),
        invocation_downstream_error_message: is_final_attempt
            .then(|| record.downstream_error_message.clone())
            .flatten(),
        invocation_upstream_request_id: is_final_attempt
            .then(|| record.upstream_request_id.clone())
            .flatten(),
        invocation_response_content_encoding: attempt.response_content_encoding.clone().or_else(
            || {
                is_final_attempt
                    .then(|| record.response_content_encoding.clone())
                    .flatten()
            },
        ),
        invocation_response_payload,
        attempt_response_body_captured,
        response_body_capture_size: attempt
            .response_raw_size
            .or_else(|| {
                is_final_attempt
                    .then_some(record.response_raw_size)
                    .flatten()
            })
            .or(attempt.upstream_response_body_bytes),
        response_body_capture_truncated,
        response_body_capture_truncated_reason: attempt
            .response_raw_truncated_reason
            .clone()
            .or_else(|| {
                is_final_attempt
                    .then(|| record.response_raw_truncated_reason.clone())
                    .flatten()
            }),
        response_body_capture_detail_level,
    }
}

fn build_workflow_attempt_from_row(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
    is_final_attempt: bool,
) -> InvocationWorkflowAttempt {
    let (
        invocation_failure_kind,
        invocation_error_message,
        invocation_downstream_error_message,
        invocation_upstream_request_id,
    ) = workflow_attempt_invocation_fields(record, is_final_attempt);
    let (request_summary, response_summary) = build_workflow_attempt_summaries(
        record,
        attempt,
        payload,
        usage_cost_audit,
        is_final_attempt,
    );

    InvocationWorkflowAttempt {
        synthetic: false,
        attempt_id: attempt.attempt_id.clone(),
        occurred_at: attempt.occurred_at.clone(),
        endpoint: attempt.endpoint.clone(),
        sticky_key: attempt.sticky_key.clone(),
        routing_source: attempt.routing_source.clone(),
        routing_selection_audit: attempt
            .routing_selection_audit_json
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok()),
        upstream_account_id: attempt.upstream_account_id,
        upstream_account_name: attempt.upstream_account_name.clone(),
        request_model: record.request_model.clone(),
        response_model: record
            .response_model
            .clone()
            .or_else(|| record.model.clone()),
        upstream_route_key: attempt.upstream_route_key.clone(),
        proxy_binding_key_snapshot: attempt.proxy_binding_key_snapshot.clone(),
        attempt_index: attempt.attempt_index,
        distinct_account_index: attempt.distinct_account_index,
        same_account_retry_index: attempt.same_account_retry_index,
        requester_ip: attempt
            .requester_ip
            .clone()
            .or_else(|| record.requester_ip.clone()),
        started_at: normalize_optional_timestamp(attempt.started_at.as_deref()),
        finished_at: normalize_optional_timestamp(attempt.finished_at.as_deref()),
        status: attempt.status.clone(),
        phase: attempt.phase.clone(),
        http_status: attempt.http_status,
        downstream_http_status: attempt.downstream_http_status,
        failure_kind: attempt.failure_kind.clone().or(invocation_failure_kind),
        error_message: attempt.error_message.clone().or(invocation_error_message),
        downstream_error_message: attempt
            .downstream_error_message
            .clone()
            .or(invocation_downstream_error_message),
        connect_latency_ms: finite_nonnegative_timing(attempt.connect_latency_ms.or_else(|| {
            is_final_attempt
                .then_some(record.t_upstream_connect_ms)
                .flatten()
        })),
        first_token_ms: is_final_attempt
            .then(|| {
                final_attempt_has_first_token_evidence(record, attempt)
                    .then_some(finite_nonnegative_timing(record.first_token_ms))
                    .flatten()
            })
            .flatten(),
        first_byte_latency_ms: finite_nonnegative_timing(attempt.first_byte_latency_ms.or_else(
            || {
                is_final_attempt
                    .then_some(record.t_upstream_ttfb_ms)
                    .flatten()
            },
        )),
        stream_latency_ms: if is_final_attempt {
            final_attempt_has_stream_evidence(attempt)
                .then_some(finite_positive_timing(attempt.stream_latency_ms))
                .flatten()
        } else {
            finite_positive_timing(attempt.stream_latency_ms)
        },
        upstream_request_id: attempt
            .upstream_request_id
            .clone()
            .or(invocation_upstream_request_id),
        request_summary,
        response_summary,
    }
}

fn workflow_attempt_invocation_fields(
    record: &ApiInvocation,
    is_final_attempt: bool,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    (
        is_final_attempt
            .then(|| record.failure_kind.clone())
            .flatten(),
        is_final_attempt
            .then(|| record.error_message.clone())
            .flatten(),
        is_final_attempt
            .then(|| record.downstream_error_message.clone())
            .flatten(),
        is_final_attempt
            .then(|| record.upstream_request_id.clone())
            .flatten(),
    )
}

fn build_workflow_attempt_summaries(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
    is_final_attempt: bool,
) -> (Option<Value>, Option<Value>) {
    let request_summary = merge_attempt_request_summary_json(
        attempt.request_summary_json.as_deref(),
        build_attempt_request_summary(record, attempt, payload),
    );
    let response_summary =
        parse_summary_json_or_fallback(attempt.response_summary_json.as_deref(), || {
            build_attempt_response_summary(
                record,
                attempt,
                payload,
                usage_cost_audit,
                is_final_attempt,
            )
        })
        .map(|summary| sanitize_response_summary_timing(summary, attempt, is_final_attempt));
    (request_summary, response_summary)
}

fn build_synthetic_workflow_attempt(
    record: &ApiInvocation,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
) -> InvocationWorkflowAttempt {
    let request_compression = request_compression_value(
        payload_string(payload, &["requestCompressionAlgorithm"]),
        payload_string(payload, &["requestCompressionMode"]),
        derive_request_compression_from_payload(payload),
    );
    let request_summary = json!({
        "endpoint": record.endpoint.clone(),
        "routeMode": record.route_mode.clone(),
        "transport": record.transport.clone(),
        "requestModel": record.request_model.clone(),
        "responseModel": record.response_model.as_ref().or(record.model.as_ref()),
        "requestedServiceTier": record.requested_service_tier.clone(),
        "reasoningEffort": record.reasoning_effort.clone(),
        "compactionRequestKind": record.compaction_request_kind.clone(),
        "imageIntent": record.image_intent.clone(),
        "promptCacheKey": record.prompt_cache_key.clone(),
        "stickyKey": record.sticky_key.clone(),
        "requesterIp": record.requester_ip.clone(),
        "account": {
            "id": record.upstream_account_id,
            "name": record.upstream_account_name.clone(),
        },
        "routing": {
            "proxyDisplayName": record.proxy_display_name.clone(),
            "upstreamScope": payload_string(payload, &["upstreamScope"]),
            "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
            "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
            "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
        },
        "headers": build_request_header_snapshot(payload),
        "client": build_request_client_snapshot(payload),
        "compression": request_compression,
        "bodyCapture": {
            "availableAtInvocationLevel": record.request_raw_path.is_some(),
            "size": record.request_raw_size,
            "truncated": record.request_raw_truncated.unwrap_or_default() != 0,
            "truncatedReason": record.request_raw_truncated_reason.clone(),
            "detailLevel": record.detail_level.clone(),
            "detailPruneReason": record.detail_prune_reason.clone(),
        },
    });
    let response_summary = build_synthetic_response_summary(record, payload, usage_cost_audit);
    InvocationWorkflowAttempt {
        synthetic: true,
        attempt_id: None,
        occurred_at: record.occurred_at.clone(),
        endpoint: record.endpoint.clone().unwrap_or_default(),
        sticky_key: record.sticky_key.clone(),
        routing_source: None,
        routing_selection_audit: None,
        upstream_account_id: record.upstream_account_id,
        upstream_account_name: record.upstream_account_name.clone(),
        request_model: record.request_model.clone(),
        response_model: record
            .response_model
            .clone()
            .or_else(|| record.model.clone()),
        upstream_route_key: None,
        proxy_binding_key_snapshot: None,
        attempt_index: 1,
        distinct_account_index: 1,
        same_account_retry_index: 1,
        requester_ip: record.requester_ip.clone(),
        started_at: Some(record.occurred_at.clone()),
        finished_at: record.t_total_ms.and_then(|total| {
            parse_to_utc_datetime(&record.occurred_at).and_then(|occurred_at| {
                chrono::Duration::from_std(Duration::from_secs_f64(total.max(0.0) / 1000.0))
                    .ok()
                    .map(|delta| format_utc_iso(occurred_at + delta))
            })
        }),
        status: record
            .status
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        phase: record.live_phase.clone(),
        http_status: None,
        downstream_http_status: record.downstream_status_code,
        failure_kind: record.failure_kind.clone(),
        error_message: record.error_message.clone(),
        downstream_error_message: record.downstream_error_message.clone(),
        connect_latency_ms: finite_nonnegative_timing(record.t_upstream_connect_ms),
        first_token_ms: finite_nonnegative_timing(record.first_token_ms),
        first_byte_latency_ms: finite_nonnegative_timing(record.t_upstream_ttfb_ms),
        stream_latency_ms: finite_positive_timing(record.t_upstream_stream_ms),
        upstream_request_id: record.upstream_request_id.clone(),
        request_summary: Some(request_summary),
        response_summary: Some(response_summary),
    }
}

fn build_synthetic_response_summary(
    record: &ApiInvocation,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
) -> Value {
    json!({
        "status": record.status.clone(),
        "phase": record.live_phase.clone(),
        "downstreamHttpStatus": record.downstream_status_code,
        "failureKind": record.failure_kind.clone(),
        "errorMessage": record.error_message.clone(),
        "downstreamErrorMessage": record.downstream_error_message.clone(),
        "upstreamRequestId": record.upstream_request_id.clone(),
        "upstreamErrorCode": record.upstream_error_code.clone(),
        "upstreamErrorMessage": record.upstream_error_message.clone(),
        "streamTerminalEvent": record.stream_terminal_event.clone(),
        "responseContentEncoding": record.response_content_encoding.clone(),
        "serviceTier": record.service_tier.clone(),
        "billingServiceTier": record.billing_service_tier.clone(),
        "headers": build_response_header_snapshot(record, None, payload, true),
        "delivery": build_response_delivery_snapshot(payload),
        "latencyMs": {
            "connect": finite_nonnegative_timing(record.t_upstream_connect_ms),
            "firstByte": finite_nonnegative_timing(record.t_upstream_ttfb_ms),
            "stream": finite_positive_timing(record.t_upstream_stream_ms),
            "requestRead": finite_nonnegative_timing(record.t_req_read_ms),
            "requestParse": finite_nonnegative_timing(record.t_req_parse_ms),
            "responseParse": finite_nonnegative_timing(record.t_resp_parse_ms),
            "persist": finite_nonnegative_timing(record.t_persist_ms),
            "total": finite_nonnegative_timing(record.t_total_ms),
        },
        "responseBodyCapture": {
            "availableAtInvocationLevel": record.response_raw_path.is_some(),
            "size": record.response_raw_size,
            "truncated": record.response_raw_truncated.unwrap_or_default() != 0,
            "truncatedReason": record.response_raw_truncated_reason.clone(),
            "detailLevel": record.detail_level.clone(),
            "detailPruneReason": record.detail_prune_reason.clone(),
        },
        "usage": usage_cost_audit.map(|audit| build_invocation_usage_summary(record, audit)),
    })
}

fn workflow_attempt_account_label(attempt: &InvocationWorkflowAttempt) -> String {
    attempt
        .upstream_account_name
        .clone()
        .or_else(|| attempt.upstream_account_id.map(|id| format!("账号 #{id}")))
        .unwrap_or_else(|| "未定账号".to_string())
}

fn workflow_route_subtitle(attempt: &InvocationWorkflowAttempt) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(model) = attempt
        .request_model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(model.to_string());
    }
    if let Some(proxy) = attempt
        .proxy_binding_key_snapshot
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(proxy.to_string());
    }
    if let Some(route_key) = attempt
        .upstream_route_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(route_key.to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn workflow_route_title(attempt: &InvocationWorkflowAttempt) -> String {
    let account_label = workflow_attempt_account_label(attempt);
    if account_label == "未定账号" {
        "Route resolution".to_string()
    } else {
        format!("Route {account_label}")
    }
}

fn build_routing_detail(request_summary: Option<&Value>) -> Option<Value> {
    let request = request_summary?.clone();
    let request_headers = request
        .as_object()
        .and_then(|value| value.get("headers"))
        .cloned();
    let request_body = request
        .as_object()
        .and_then(|value| value.get("bodyCapture"))
        .cloned();
    Some(json!({
        "request": request,
        "requestHeaders": request_headers,
        "requestBody": request_body,
    }))
}

fn build_routing_timeline_entry(
    block_id: String,
    attempt: &InvocationWorkflowAttempt,
) -> InvocationWorkflowTimelineEntry {
    InvocationWorkflowTimelineEntry {
        block_id,
        kind: "routingDecision".to_string(),
        occurred_at: attempt
            .started_at
            .clone()
            .or_else(|| Some(attempt.occurred_at.clone())),
        title: workflow_route_title(attempt),
        subtitle: workflow_route_subtitle(attempt)
            .or_else(|| (!attempt.endpoint.trim().is_empty()).then(|| attempt.endpoint.clone())),
        status: None,
        attempt: None,
        detail: build_routing_detail(attempt.request_summary.as_ref()),
        response_body: None,
    }
}

fn invocation_workflow_attempt_row_is_pseudo_terminal(
    attempt: &InvocationWorkflowAttemptRow,
) -> bool {
    normalized_runtime_text(Some(attempt.status.as_str())) == "budget_exhausted_final"
        && attempt.same_account_retry_index == 0
        && normalize_optional_timestamp(attempt.started_at.as_deref())
            == normalize_optional_timestamp(attempt.finished_at.as_deref())
        && attempt
            .connect_latency_ms
            .is_none_or(|value| !value.is_finite() || value <= 0.0)
        && attempt
            .first_byte_latency_ms
            .is_none_or(|value| !value.is_finite() || value <= 0.0)
        && attempt
            .stream_latency_ms
            .is_none_or(|value| !value.is_finite() || value <= 0.0)
        && attempt
            .upstream_request_id
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        && attempt
            .request_summary_json
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
        && attempt
            .response_summary_json
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
}

fn last_success_like_attempt_row_id(attempt_rows: &[&InvocationWorkflowAttemptRow]) -> Option<i64> {
    attempt_rows
        .iter()
        .filter(|attempt| {
            matches!(
                normalized_runtime_text(Some(attempt.status.as_str())).as_str(),
                "success" | "completed" | "warning_success"
            )
        })
        .max_by_key(|attempt| (attempt.attempt_index, attempt.attempt_row_id))
        .map(|attempt| attempt.attempt_row_id)
}

fn final_real_attempt_row_id(attempts: &[&InvocationWorkflowAttemptRow]) -> Option<i64> {
    attempts
        .iter()
        .filter(|attempt| {
            normalized_runtime_text(Some(attempt.status.as_str())) != "budget_exhausted_final"
        })
        .max_by_key(|attempt| (attempt.attempt_index, attempt.attempt_row_id))
        .map(|attempt| attempt.attempt_row_id)
}
