use super::*;

#[cfg(test)]
pub(crate) fn broadcast_test_record_payload(state: &AppState, record: &ApiInvocation) {
    if state.broadcaster.receiver_count() > 0 {
        let _ = state.broadcaster.send(BroadcastPayload::Records {
            records: vec![record.clone()],
        });
    }
}

pub(crate) struct ProxyPayloadSummary<'a> {
    pub(crate) target: ProxyCaptureTarget,
    pub(crate) status: StatusCode,
    pub(crate) is_stream: bool,
    pub(crate) request_contains_encrypted_content: bool,
    pub(crate) response_contains_encrypted_content: bool,
    pub(crate) compaction_request_kind: Option<CompactionKind>,
    pub(crate) compaction_response_kind: Option<CompactionKind>,
    pub(crate) image_intent: Option<&'a str>,
    pub(crate) request_model: Option<&'a str>,
    pub(crate) requested_service_tier: Option<&'a str>,
    pub(crate) billing_service_tier: Option<&'a str>,
    pub(crate) reasoning_effort: Option<&'a str>,
    pub(crate) response_model: Option<&'a str>,
    pub(crate) usage_missing_reason: Option<&'a str>,
    pub(crate) request_parse_error: Option<&'a str>,
    pub(crate) request_compression_algorithm: Option<&'a str>,
    pub(crate) request_compression_mode: Option<&'a str>,
    pub(crate) request_compression_logical_body_bytes: Option<usize>,
    pub(crate) request_compression_transmitted_body_bytes: Option<usize>,
    pub(crate) request_compression_transmission_complete: Option<bool>,
    pub(crate) failure_kind: Option<&'a str>,
    pub(crate) requester_ip: Option<&'a str>,
    pub(crate) request_user_agent: Option<&'a str>,
    pub(crate) request_x_forwarded_for: Option<&'a str>,
    pub(crate) request_forwarded: Option<&'a str>,
    pub(crate) request_x_real_ip: Option<&'a str>,
    pub(crate) upstream_scope: &'a str,
    pub(crate) route_mode: &'a str,
    pub(crate) sticky_key: Option<&'a str>,
    pub(crate) prompt_cache_key: Option<&'a str>,
    pub(crate) prompt_cache_key_attribution_source: Option<&'a str>,
    pub(crate) client_fingerprint: Option<&'a str>,
    pub(crate) client_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<&'a str>,
    pub(crate) upstream_account_kind: Option<&'a str>,
    pub(crate) upstream_base_url_host: Option<&'a str>,
    pub(crate) oauth_account_header_attached: Option<bool>,
    pub(crate) oauth_account_id_shape: Option<&'a str>,
    pub(crate) oauth_forwarded_header_count: Option<usize>,
    pub(crate) oauth_forwarded_header_names: Option<&'a [String]>,
    pub(crate) oauth_fingerprint_version: Option<&'a str>,
    pub(crate) oauth_forwarded_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    pub(crate) oauth_prompt_cache_header_forwarded: Option<bool>,
    pub(crate) oauth_request_body_prefix_fingerprint: Option<&'a str>,
    pub(crate) oauth_request_body_prefix_bytes: Option<usize>,
    pub(crate) oauth_request_body_snapshot_kind: Option<&'a str>,
    pub(crate) oauth_responses_body_mode: Option<&'a str>,
    pub(crate) oauth_responses_rewrite: Option<&'a oauth_bridge::OauthResponsesRewriteSummary>,
    pub(crate) service_tier: Option<&'a str>,
    pub(crate) stream_terminal_event: Option<&'a str>,
    pub(crate) upstream_error_code: Option<&'a str>,
    pub(crate) upstream_error_message: Option<&'a str>,
    pub(crate) downstream_status_code: Option<StatusCode>,
    pub(crate) downstream_error_message: Option<&'a str>,
    pub(crate) upstream_request_id: Option<&'a str>,
    pub(crate) response_content_encoding: Option<&'a str>,
    pub(crate) stream_failure_origin: Option<&'a str>,
    pub(crate) upstream_read_error_kind: Option<&'a str>,
    pub(crate) content_encoding_chain: Option<&'a str>,
    pub(crate) forwarded_chunk_count: Option<usize>,
    pub(crate) forwarded_bytes: Option<usize>,
    pub(crate) usage_observed: Option<bool>,
    pub(crate) downstream_close_phase: Option<&'a str>,
    pub(crate) downstream_write_error_kind: Option<&'a str>,
    pub(crate) last_upstream_chunk_gap_ms: Option<u64>,
    pub(crate) upstream_approx_upload_bytes: Option<usize>,
    pub(crate) upstream_approx_download_bytes: Option<usize>,
    pub(crate) proxy_display_name: Option<&'a str>,
    pub(crate) proxy_weight_delta: Option<f64>,
    pub(crate) pool_attempt_count: Option<usize>,
    pub(crate) pool_distinct_account_count: Option<usize>,
    pub(crate) pool_attempt_terminal_reason: Option<&'a str>,
    pub(crate) blocked_binding: Option<&'a BlockedBindingDiagnostic>,
}

pub(crate) fn build_proxy_payload_summary(summary: ProxyPayloadSummary<'_>) -> String {
    let ProxyPayloadSummary {
        target,
        status,
        is_stream,
        request_contains_encrypted_content,
        response_contains_encrypted_content,
        compaction_request_kind,
        compaction_response_kind,
        image_intent,
        request_model,
        requested_service_tier,
        billing_service_tier,
        reasoning_effort,
        response_model,
        usage_missing_reason,
        request_parse_error,
        request_compression_algorithm,
        request_compression_mode,
        request_compression_logical_body_bytes,
        request_compression_transmitted_body_bytes,
        request_compression_transmission_complete,
        failure_kind,
        requester_ip,
        request_user_agent,
        request_x_forwarded_for,
        request_forwarded,
        request_x_real_ip,
        upstream_scope,
        route_mode,
        sticky_key,
        prompt_cache_key,
        prompt_cache_key_attribution_source,
        client_fingerprint,
        client_header_fingerprints,
        upstream_account_id,
        upstream_account_name,
        upstream_account_kind,
        upstream_base_url_host,
        oauth_account_header_attached,
        oauth_account_id_shape,
        oauth_forwarded_header_count,
        oauth_forwarded_header_names,
        oauth_fingerprint_version,
        oauth_forwarded_header_fingerprints,
        oauth_prompt_cache_header_forwarded,
        oauth_request_body_prefix_fingerprint,
        oauth_request_body_prefix_bytes,
        oauth_request_body_snapshot_kind,
        oauth_responses_body_mode,
        oauth_responses_rewrite,
        service_tier,
        stream_terminal_event,
        upstream_error_code,
        upstream_error_message,
        downstream_status_code,
        downstream_error_message,
        upstream_request_id,
        response_content_encoding,
        stream_failure_origin,
        upstream_read_error_kind,
        content_encoding_chain,
        forwarded_chunk_count,
        forwarded_bytes,
        usage_observed,
        downstream_close_phase,
        downstream_write_error_kind,
        last_upstream_chunk_gap_ms,
        upstream_approx_upload_bytes,
        upstream_approx_download_bytes,
        proxy_display_name,
        proxy_weight_delta,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
        blocked_binding,
    } = summary;
    let payload = json!({
        "endpoint": target.endpoint(),
        "statusCode": status.as_u16(),
        "isStream": is_stream,
        "requestContainsEncryptedContent": request_contains_encrypted_content,
        "responseContainsEncryptedContent": response_contains_encrypted_content,
        "compactionRequestKind": compaction_request_kind.map(CompactionKind::as_payload_str),
        "compactionResponseKind": compaction_response_kind.map(CompactionKind::as_payload_str),
        "imageIntent": image_intent,
        "requestModel": request_model,
        "requestedServiceTier": requested_service_tier,
        "billingServiceTier": billing_service_tier,
        "reasoningEffort": reasoning_effort,
        "responseModel": response_model,
        "usageMissingReason": usage_missing_reason,
        "requestParseError": request_parse_error,
        "requestCompressionAlgorithm": request_compression_algorithm,
        "requestCompressionMode": request_compression_mode,
        "requestCompressionLogicalBodyBytes": request_compression_logical_body_bytes,
        "requestCompressionTransmittedBodyBytes": request_compression_transmitted_body_bytes,
        "requestCompressionTransmissionComplete": request_compression_transmission_complete,
        "failureKind": failure_kind,
        "requesterIp": requester_ip,
        "requestUserAgent": request_user_agent,
        "requestXForwardedFor": request_x_forwarded_for,
        "requestForwarded": request_forwarded,
        "requestXRealIp": request_x_real_ip,
        "upstreamScope": upstream_scope,
        "routeMode": route_mode,
        "stickyKey": sticky_key,
        "promptCacheKey": prompt_cache_key,
        "promptCacheKeyAttributionSource": prompt_cache_key_attribution_source,
        "clientFingerprint": client_fingerprint,
        "clientHeaderFingerprints": client_header_fingerprints,
        "upstreamAccountId": upstream_account_id,
        "upstreamAccountName": upstream_account_name,
        "upstreamAccountKind": upstream_account_kind,
        "upstreamBaseUrlHost": upstream_base_url_host,
        "oauthAccountHeaderAttached": oauth_account_header_attached,
        "oauthAccountIdShape": oauth_account_id_shape,
        "oauthForwardedHeaderCount": oauth_forwarded_header_count,
        "oauthForwardedHeaderNames": oauth_forwarded_header_names,
        "oauthFingerprintVersion": oauth_fingerprint_version,
        "oauthForwardedHeaderFingerprints": oauth_forwarded_header_fingerprints,
        "oauthPromptCacheHeaderForwarded": oauth_prompt_cache_header_forwarded,
        "oauthRequestBodyPrefixFingerprint": oauth_request_body_prefix_fingerprint,
        "oauthRequestBodyPrefixBytes": oauth_request_body_prefix_bytes,
        "oauthRequestBodySnapshotKind": oauth_request_body_snapshot_kind,
        "oauthResponsesBodyMode": oauth_responses_body_mode,
        "oauthResponsesRewrite": oauth_responses_rewrite,
        "serviceTier": service_tier,
        "streamTerminalEvent": stream_terminal_event,
        "upstreamErrorCode": upstream_error_code,
        "upstreamErrorMessage": upstream_error_message,
        "downstreamStatusCode": downstream_status_code.map(|value| value.as_u16()),
        "downstreamErrorMessage": downstream_error_message,
        "upstreamRequestId": upstream_request_id,
        "responseContentEncoding": response_content_encoding,
        "streamFailureOrigin": stream_failure_origin,
        "upstreamReadErrorKind": upstream_read_error_kind,
        "contentEncodingChain": content_encoding_chain,
        "forwardedChunkCount": forwarded_chunk_count,
        "forwardedBytes": forwarded_bytes,
        "usageObserved": usage_observed,
        "downstreamClosePhase": downstream_close_phase,
        "downstreamWriteErrorKind": downstream_write_error_kind,
        "lastUpstreamChunkGapMs": last_upstream_chunk_gap_ms,
        "upstreamApproxUploadBytes": upstream_approx_upload_bytes,
        "upstreamApproxDownloadBytes": upstream_approx_download_bytes,
        "proxyDisplayName": proxy_display_name,
        "proxyWeightDelta": proxy_weight_delta,
        "poolAttemptCount": pool_attempt_count,
        "poolDistinctAccountCount": pool_distinct_account_count,
        "poolAttemptTerminalReason": pool_attempt_terminal_reason,
        "blockedBinding": blocked_binding,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

pub(crate) fn with_proxy_stream_terminal_diagnostics(
    payload: String,
    upstream_outcome: Option<&str>,
    post_terminal_upstream_read_error_kind: Option<&str>,
    post_terminal_upstream_read_error_message: Option<&str>,
    post_terminal_downstream_write_error_kind: Option<&str>,
    post_terminal_downstream_write_error_message: Option<&str>,
) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    for (key, entry) in [
        ("upstreamOutcome", upstream_outcome),
        (
            "postTerminalUpstreamReadErrorKind",
            post_terminal_upstream_read_error_kind,
        ),
        (
            "postTerminalUpstreamReadErrorMessage",
            post_terminal_upstream_read_error_message,
        ),
        (
            "postTerminalDownstreamWriteErrorKind",
            post_terminal_downstream_write_error_kind,
        ),
        (
            "postTerminalDownstreamWriteErrorMessage",
            post_terminal_downstream_write_error_message,
        ),
    ] {
        if let Some(entry) = entry {
            object.insert(key.to_string(), Value::String(entry.to_string()));
        }
    }
    serde_json::to_string(&value).unwrap_or(payload)
}

pub(crate) fn with_image_tool_rewrite_payload_summary(
    payload: String,
    image_tool_rewrite: Option<&Value>,
) -> String {
    let Some(image_tool_rewrite) = image_tool_rewrite else {
        return payload;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    object.insert("imageToolRewrite".to_string(), image_tool_rewrite.clone());
    serde_json::to_string(&value).unwrap_or(payload)
}

pub(crate) fn with_codex_imagegen_rewrite_payload_summary(
    payload: String,
    codex_imagegen_rewrite: Option<&Value>,
) -> String {
    let Some(codex_imagegen_rewrite) = codex_imagegen_rewrite else {
        return payload;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    object.insert(
        "codexImagegenRewrite".to_string(),
        codex_imagegen_rewrite.clone(),
    );
    serde_json::to_string(&value).unwrap_or(payload)
}

pub(crate) fn nullable_runtime_timing_value(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_existing_proxy_invocation_record_tx(
    tx: &mut SqliteConnection,
    id: i64,
    record: &ProxyCaptureRecord,
    raw_response: &str,
    resp_raw: &RawPayloadMeta,
    failure_kind: Option<&str>,
    failure_class: &str,
    is_actionable: bool,
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    first_token_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
    t_persist_ms: Option<f64>,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE codex_invocations
        SET
            source = ?2,
            model = ?3,
            input_tokens = ?4,
            output_tokens = ?5,
            cache_input_tokens = ?6,
            reported_cache_write_tokens = COALESCE(?43, reported_cache_write_tokens),
            reasoning_tokens = ?7,
            total_tokens = ?8,
            cost = ?9,
            cost_input = ?10,
            cost_cache_write = ?11,
            cost_cache_read = ?12,
            cost_output = ?13,
            cost_reasoning = ?14,
            cost_estimated = ?15,
            price_version = ?16,
            status = ?17,
            error_message = ?18,
            failure_kind = ?19,
            failure_class = ?20,
            is_actionable = ?21,
            payload = ?22,
            raw_response = ?23,
            request_raw_path = ?24,
            request_raw_codec = ?25,
            request_raw_size = ?26,
            request_raw_truncated = ?27,
            request_raw_truncated_reason = ?28,
            response_raw_path = ?29,
            response_raw_codec = ?30,
            response_raw_size = ?31,
            response_raw_truncated = ?32,
            response_raw_truncated_reason = ?33,
            t_total_ms = ?34,
            t_req_read_ms = ?35,
            t_req_parse_ms = ?36,
            t_upstream_connect_ms = ?37,
            t_upstream_ttfb_ms = ?38,
            first_token_ms = ?39,
            t_upstream_stream_ms = ?40,
            t_resp_parse_ms = ?41,
            t_persist_ms = ?42
        WHERE id = ?1
          AND (
                LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) = 'interrupted'
                    AND LOWER(TRIM(COALESCE(failure_kind, ''))) = 'proxy_interrupted'
                )
          )
        "#,
    )
    .bind(id)
    .bind(SOURCE_PROXY)
    .bind(&record.model)
    .bind(record.usage.input_tokens)
    .bind(record.usage.output_tokens)
    .bind(record.usage.cache_input_tokens)
    .bind(record.usage.reasoning_tokens)
    .bind(record.usage.total_tokens)
    .bind(record.cost)
    .bind(record.cost_breakdown.map(|value| value.input))
    .bind(record.cost_breakdown.map(|value| value.cache_write))
    .bind(record.cost_breakdown.map(|value| value.cache_read))
    .bind(record.cost_breakdown.map(|value| value.output))
    .bind(record.cost_breakdown.map(|value| value.reasoning))
    .bind(record.cost_estimated as i64)
    .bind(record.price_version.as_deref())
    .bind(&record.status)
    .bind(record.error_message.as_deref())
    .bind(failure_kind)
    .bind(failure_class)
    .bind(is_actionable as i64)
    .bind(record.payload.as_deref())
    .bind(raw_response)
    .bind(record.req_raw.path.as_deref())
    .bind(raw_payload_meta_codec(&record.req_raw))
    .bind(record.req_raw.size_bytes)
    .bind(record.req_raw.truncated as i64)
    .bind(record.req_raw.truncated_reason.as_deref())
    .bind(resp_raw.path.as_deref())
    .bind(raw_payload_meta_codec(resp_raw))
    .bind(resp_raw.size_bytes)
    .bind(resp_raw.truncated as i64)
    .bind(resp_raw.truncated_reason.as_deref())
    .bind(t_total_ms)
    .bind(t_req_read_ms)
    .bind(t_req_parse_ms)
    .bind(t_upstream_connect_ms)
    .bind(t_upstream_ttfb_ms)
    .bind(first_token_ms)
    .bind(t_upstream_stream_ms)
    .bind(t_resp_parse_ms)
    .bind(t_persist_ms)
    .bind(record.usage.reported_cache_write_tokens)
    .execute(&mut *tx)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub(crate) fn api_invocation_from_runtime_record(record: &ProxyCaptureRecord) -> ApiInvocation {
    let payload = record
        .payload
        .as_deref()
        .and_then(|payload| serde_json::from_str::<Value>(payload).ok());
    let payload_text = |key: &str| {
        payload
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };
    let payload_i64 = |key: &str| {
        payload
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(crate::proxy::json_value_to_i64)
    };
    let payload_f64 = |key: &str| {
        payload
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_f64)
    };
    let prompt_cache_key = payload_text("promptCacheKey");
    let sticky_key = payload
        .as_ref()
        .and_then(|value| {
            value
                .get("stickyKey")
                .or_else(|| value.get("promptCacheKey"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let upstream_account_id = payload
        .as_ref()
        .and_then(|value| value.get("upstreamAccountId"))
        .and_then(crate::proxy::json_value_to_i64);
    let upstream_account_name = payload_text("upstreamAccountName");
    let blocked_binding_json = payload
        .as_ref()
        .and_then(|value| value.get("blockedBinding"))
        .filter(|value| value.is_object())
        .and_then(|value| serde_json::to_string(value).ok());
    let blocked_binding = parse_blocked_binding_json(blocked_binding_json.as_deref());
    let failure = resolve_failure_classification(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        None,
        None,
    );
    ApiInvocation {
        id: 0,
        invoke_id: record.invoke_id.clone(),
        occurred_at: record.occurred_at.clone(),
        source: SOURCE_PROXY.to_string(),
        proxy_display_name: payload_text("proxyDisplayName"),
        model: record.model.clone(),
        request_model: payload_text("requestModel"),
        response_model: payload_text("responseModel"),
        input_tokens: record.usage.input_tokens,
        output_tokens: record.usage.output_tokens,
        cache_input_tokens: record.usage.cache_input_tokens,
        reasoning_tokens: record.usage.reasoning_tokens,
        reasoning_effort: payload_text("reasoningEffort"),
        total_tokens: record.usage.total_tokens,
        cost: record.cost,
        cost_input: record.cost_breakdown.map(|value| value.input),
        cost_cache_write: record.cost_breakdown.map(|value| value.cache_write),
        cost_cache_read: record.cost_breakdown.map(|value| value.cache_read),
        cost_output: record.cost_breakdown.map(|value| value.output),
        cost_reasoning: record.cost_breakdown.map(|value| value.reasoning),
        cache_write_tokens: record.usage.input_tokens.map(|input| {
            input.saturating_sub(record.usage.cache_input_tokens.unwrap_or_default().max(0))
        }),
        reported_cache_write_tokens: record.usage.reported_cache_write_tokens,
        status: Some(record.status.clone()),
        live_phase: None,
        error_message: record.error_message.clone(),
        downstream_status_code: payload_i64("downstreamStatusCode"),
        failure_kind: failure
            .failure_kind
            .clone()
            .or_else(|| record.failure_kind.clone()),
        blocked_binding,
        blocked_binding_json,
        stream_terminal_event: payload_text("streamTerminalEvent"),
        upstream_error_code: payload_text("upstreamErrorCode"),
        upstream_error_message: payload_text("upstreamErrorMessage"),
        downstream_error_message: payload_text("downstreamErrorMessage"),
        upstream_request_id: payload_text("upstreamRequestId"),
        failure_class: Some(failure.failure_class.as_str().to_string()),
        is_actionable: Some(failure.is_actionable),
        endpoint: payload_text("endpoint"),
        compaction_request_kind: payload_text("compactionRequestKind"),
        compaction_response_kind: payload_text("compactionResponseKind"),
        image_intent: payload_text("imageIntent"),
        requester_ip: payload_text("requesterIp"),
        prompt_cache_key,
        sticky_key,
        route_mode: payload_text("routeMode"),
        upstream_account_id,
        upstream_account_name,
        response_content_encoding: payload_text("responseContentEncoding"),
        request_compression_algorithm: payload_text("requestCompressionAlgorithm"),
        transport: None,
        pool_attempt_count: payload_i64("poolAttemptCount"),
        pool_distinct_account_count: payload_i64("poolDistinctAccountCount"),
        pool_attempt_terminal_reason: payload_text("poolAttemptTerminalReason"),
        requested_service_tier: payload_text("requestedServiceTier"),
        service_tier: payload_text("serviceTier"),
        billing_service_tier: payload_text("billingServiceTier"),
        proxy_weight_delta: payload_f64("proxyWeightDelta"),
        cost_estimated: Some(record.cost_estimated as i64),
        price_version: record.price_version.clone(),
        cost_audit: None,
        request_raw_path: record.req_raw.path.clone(),
        request_raw_size: Some(record.req_raw.size_bytes),
        request_raw_truncated: Some(record.req_raw.truncated as i64),
        request_raw_truncated_reason: record.req_raw.truncated_reason.clone(),
        response_raw_path: record.resp_raw.path.clone(),
        response_raw_size: Some(record.resp_raw.size_bytes),
        response_raw_truncated: Some(record.resp_raw.truncated as i64),
        response_raw_truncated_reason: record.resp_raw.truncated_reason.clone(),
        detail_level: DETAIL_LEVEL_FULL.to_string(),
        detail_pruned_at: None,
        detail_prune_reason: None,
        t_total_ms: nullable_runtime_timing_value(record.timings.t_total_ms),
        t_req_read_ms: nullable_runtime_timing_value(record.timings.t_req_read_ms),
        t_req_parse_ms: nullable_runtime_timing_value(record.timings.t_req_parse_ms),
        t_upstream_connect_ms: nullable_runtime_timing_value(record.timings.t_upstream_connect_ms),
        t_upstream_ttfb_ms: nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms),
        first_token_ms: record
            .timings
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0),
        t_upstream_stream_ms: nullable_runtime_timing_value(record.timings.t_upstream_stream_ms),
        t_resp_parse_ms: nullable_runtime_timing_value(record.timings.t_resp_parse_ms),
        t_persist_ms: nullable_runtime_timing_value(record.timings.t_persist_ms),
        created_at: format_utc_iso_millis(Utc::now()),
    }
}

pub(crate) async fn load_persisted_api_invocation_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    let mut record = sqlx::query_as::<_, ApiInvocation>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            source,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name,
            model,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requestModel') END AS request_model,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.responseModel') END AS response_model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reported_cache_write_tokens,
            reasoning_tokens,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.reasoningEffort') END AS reasoning_effort,
        total_tokens,
        cost,
        cost_input,
        cost_cache_write,
        cost_cache_read,
        cost_output,
        cost_reasoning,
        MAX(COALESCE(input_tokens, 0) - COALESCE(cache_input_tokens, 0), 0) AS cache_write_tokens,
        status,
            error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.downstreamStatusCode') END AS downstream_status_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.compactionRequestKind') END AS compaction_request_kind,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.compactionResponseKind') END AS compaction_response_kind,
            COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.blockedBinding') = 'object'
                THEN json_extract(payload, '$.blockedBinding')
            END AS blocked_binding_json,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.streamTerminalEvent') END AS stream_terminal_event,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorCode') END AS upstream_error_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorMessage') END AS upstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.downstreamErrorMessage') END AS downstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamRequestId') END AS upstream_request_id,
            failure_class,
            is_actionable,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key,
            CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END AS sticky_key,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.routeMode') END AS route_mode,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END AS upstream_account_id,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountName') END AS upstream_account_name,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.responseContentEncoding') END AS response_content_encoding,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolAttemptCount')
            END AS pool_attempt_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolDistinctAccountCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolDistinctAccountCount')
            END AS pool_distinct_account_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptTerminalReason') = 'text'
                THEN json_extract(payload, '$.poolAttemptTerminalReason')
            END AS pool_attempt_terminal_reason,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                THEN json_extract(payload, '$.requestedServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text'
                THEN json_extract(payload, '$.serviceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text'
                THEN json_extract(payload, '$.service_tier') END AS service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.billingServiceTier') = 'text'
                THEN json_extract(payload, '$.billingServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.billing_service_tier') = 'text'
                THEN json_extract(payload, '$.billing_service_tier') END AS billing_service_tier,
            CASE WHEN json_valid(payload)
              AND json_type(payload, '$.proxyWeightDelta') IN ('integer', 'real')
              THEN json_extract(payload, '$.proxyWeightDelta') END AS proxy_weight_delta,
            cost_estimated,
            price_version,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            detail_level,
            detail_pruned_at,
            detail_prune_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms,
            created_at
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&mut *tx)
    .await?;
    hydrate_api_invocation_blocked_binding(&mut record);
    Ok(record)
}

pub(crate) async fn touch_invocation_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    payload: Option<&str>,
) -> Result<()> {
    touch_upstream_account_last_activity_tx(
        tx,
        occurred_at,
        upstream_account_id_from_payload(payload),
    )
    .await
}

pub(crate) async fn touch_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    upstream_account_id: Option<i64>,
) -> Result<()> {
    if let Some(upstream_account_id) = upstream_account_id {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = CASE
                WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                ELSE last_activity_at
            END
            WHERE id = ?2
            "#,
        )
        .bind(occurred_at)
        .bind(upstream_account_id)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn persist_and_broadcast_proxy_capture_runtime_snapshot(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let started = Instant::now();
    let persisted_record = api_invocation_from_runtime_record(&record);
    let invoke_id = persisted_record.invoke_id.clone();
    let occurred_at = persisted_record.occurred_at.clone();
    let store_outcome = state
        .proxy_runtime_invocations
        .upsert(persisted_record.clone());
    if store_outcome.skipped_terminal {
        let elapsed_ms = started.elapsed().as_millis() as u64;
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %occurred_at,
            elapsed_ms,
            runtime_store_running_count = store_outcome.running_count,
            runtime_store_pruned_count = store_outcome.pruned_count,
            running_snapshot_db_write_skipped = true,
            running_snapshot_skipped_after_terminal = true,
            "stale running proxy capture snapshot skipped after terminal persistence"
        );
        return Ok(());
    }
    state
        .dashboard_network_speed_cache
        .observe_dashboard_activity_runtime_snapshot(&persisted_record, Utc::now());
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &persisted_record,
            RuntimeMutationKind::RuntimeUpsert,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &persisted_record);
    schedule_dashboard_activity_live_snapshot(state);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    debug!(
        invoke_id = %invoke_id,
        occurred_at = %occurred_at,
        elapsed_ms,
        runtime_store_running_count = store_outcome.running_count,
        runtime_store_pruned_count = store_outcome.pruned_count,
        running_snapshot_db_write_skipped = true,
        running_snapshot_recovery_placeholder_enqueued = false,
        "running proxy capture snapshot stored in memory and broadcast"
    );

    Ok(())
}

pub(crate) fn broadcast_proxy_capture_first_token_runtime_snapshot(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    first_token_ms: f64,
) {
    if !first_token_ms.is_finite() || first_token_ms < 0.0 {
        return;
    }
    let Some(mut record) = state
        .proxy_runtime_invocations
        .snapshot()
        .into_iter()
        .find(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at)
    else {
        return;
    };
    if record.first_token_ms.is_some() {
        return;
    }
    record.first_token_ms = Some(first_token_ms);
    let outcome = state.proxy_runtime_invocations.upsert(record.clone());
    if outcome.skipped_terminal {
        return;
    }
    state
        .dashboard_network_speed_cache
        .observe_dashboard_activity_runtime_snapshot(&record, Utc::now());
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::LifecyclePhase,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    schedule_dashboard_activity_live_snapshot(state);
}

pub(crate) fn remove_proxy_runtime_snapshot_for_terminal(
    state: &AppState,
    record: &ApiInvocation,
) -> bool {
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(&record.invoke_id, &record.occurred_at);
    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id = %record.invoke_id,
        occurred_at = %record.occurred_at,
        terminal_overlay_emitted = true,
        terminal_removed_runtime_snapshot = remove_outcome.removed,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        "terminal proxy capture record stored in memory runtime overlay"
    );
    remove_outcome.already_terminal
}

pub(crate) fn remove_proxy_runtime_snapshot_by_key(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    reason: &'static str,
) -> bool {
    state
        .dashboard_network_speed_cache
        .drop_dashboard_activity_invocation(invoke_id, occurred_at);
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);
    let removed_runtime_snapshot = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at);
    if let Some(record) = &removed_runtime_snapshot {
        state
            .subscription_hub
            .publish_runtime_mutation(RuntimeMutation::invocation(
                record,
                RuntimeMutationKind::RuntimeRemoved,
            ));
    }
    debug!(
        invoke_id,
        occurred_at,
        reason,
        terminal_removed_runtime_snapshot = removed_runtime_snapshot.is_some(),
        terminal_already_tombstoned = false,
        "non-terminal proxy runtime snapshot removed by key"
    );
    removed_runtime_snapshot.is_some()
}

pub(crate) fn terminalize_proxy_runtime_snapshot_by_key(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    reason: &'static str,
) -> bool {
    let Some(mut record) = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at)
    else {
        debug!(
            invoke_id,
            occurred_at,
            reason,
            terminal_removed_runtime_snapshot = false,
            terminal_already_tombstoned = false,
            "no non-terminal proxy runtime snapshot found for terminal cleanup"
        );
        return false;
    };

    record.status = Some(INVOCATION_STATUS_INTERRUPTED.to_string());
    record.error_message = Some(format!(
        "[{PROXY_FAILURE_INVOCATION_INTERRUPTED}] proxy request ended before a terminal record was written"
    ));
    record.failure_kind = Some(PROXY_FAILURE_INVOCATION_INTERRUPTED.to_string());
    record.failure_class = Some(FAILURE_CLASS_SERVICE.to_string());
    record.is_actionable = Some(true);
    record.pool_attempt_terminal_reason = Some(PROXY_FAILURE_INVOCATION_INTERRUPTED.to_string());
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(&record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);

    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id,
        occurred_at,
        reason,
        terminal_removed_runtime_snapshot = true,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        terminal_delta_skipped_runtime_only = true,
        "non-terminal proxy runtime snapshot terminalized by key"
    );
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::TerminalCommitted,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    true
}

pub(crate) fn terminalize_proxy_runtime_snapshot_with_error(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    status: StatusCode,
    failure_kind: &'static str,
    error_message: &str,
    reason: &'static str,
) -> bool {
    let Some(mut record) = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at)
    else {
        debug!(
            invoke_id,
            occurred_at,
            reason,
            terminal_overlay_emitted = false,
            terminal_removed_runtime_snapshot = false,
            "no non-terminal proxy runtime snapshot found for terminal error overlay"
        );
        return false;
    };

    record.status = Some(if status.is_server_error() {
        format!("http_{}", status.as_u16())
    } else {
        "failed".to_string()
    });
    record.error_message = Some(format!("[{failure_kind}] {error_message}"));
    record.failure_kind = Some(failure_kind.to_string());
    record.failure_class = Some(
        if status.is_client_error() {
            FAILURE_CLASS_CLIENT
        } else {
            FAILURE_CLASS_SERVICE
        }
        .to_string(),
    );
    record.is_actionable = Some(true);
    record.pool_attempt_terminal_reason = Some(failure_kind.to_string());
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(&record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);

    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id,
        occurred_at,
        reason,
        status = %status,
        failure_kind,
        terminal_overlay_emitted = true,
        terminal_removed_runtime_snapshot = true,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        terminal_delta_skipped_runtime_only = true,
        "non-terminal proxy runtime snapshot terminalized with error overlay"
    );
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::TerminalCommitted,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    true
}

pub(crate) async fn observe_successful_proxy_capture_model_route_cache(
    state: &AppState,
    record: &ProxyCaptureRecord,
) {
    if record.status != "success" {
        return;
    }

    let metadata = terminal_payload_metadata(record.payload.as_deref());
    match observe_proxy_cache_hit_if_success(state, record).await {
        Ok(outcome) => {
            if outcome.observed {
                state
                    .subscription_hub
                    .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
            }
            if outcome.availability_increased {
                let account_allows_publish = match metadata.upstream_account_id {
                    Some(account_id) => match pool_account_allows_model_route_availability_publish(
                        &state.pool,
                        account_id,
                    )
                    .await
                    {
                        Ok(allowed) => allowed,
                        Err(err) => {
                            warn!(
                                invoke_id = %record.invoke_id,
                                account_id,
                                error = %err,
                                "failed to verify account fence before publishing model route availability"
                            );
                            false
                        }
                    },
                    None => false,
                };
                if account_allows_publish {
                    publish_pool_routing_availability(state);
                } else {
                    debug!(
                        invoke_id = %record.invoke_id,
                        upstream_account_id = metadata.upstream_account_id,
                        "model cache observation increased capacity without publishing because the account remains fenced"
                    );
                }
            }
        }
        Err(err) => {
            warn!(
                invoke_id = %record.invoke_id,
                error = %err,
                "failed to observe model route cache hit"
            );
        }
    }
}

pub(crate) async fn persist_and_broadcast_proxy_capture_terminal_record(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    crate::observability::observed_future(
        state.observability.enabled,
        "persist_and_broadcast_proxy_capture_terminal_record",
        async move {
            let enqueue_started = Instant::now();
            let persisted_record = api_invocation_from_runtime_record(&record);
            let invoke_id = persisted_record.invoke_id.clone();
            let duplicate_terminal =
                remove_proxy_runtime_snapshot_for_terminal(state, &persisted_record);
            if duplicate_terminal {
                debug!(
                    invoke_id = %invoke_id,
                    occurred_at = %persisted_record.occurred_at,
                    business_unblocked_record_write = true,
                    "duplicate terminal proxy capture record skipped before sqlite enqueue"
                );
                schedule_proxy_capture_follow_up_after_terminal_enqueue(
                    state,
                    &invoke_id,
                    "duplicate_runtime_terminal",
                );
                return Ok(());
            }
            if let Err(error) = record_model_route_observation(state, &record).await {
                state.proxy_runtime_invocations.clear_terminal_tombstone(
                    &persisted_record.invoke_id,
                    &persisted_record.occurred_at,
                );
                return Err(error).context(
                    "cache routing observation was not accepted; terminal can be retried",
                );
            }
            let projection =
                register_terminal_projection_before_enqueue(state, &persisted_record).await;
            let delta = &projection.dashboard;
            if !delta.duplicate {
                state
                    .observability
                    .proxy_terminal(&record, persisted_record.endpoint.as_deref());
            }
            let startup_backfill_tasks = startup_backfill_tasks_for_terminal(&persisted_record);
            state
                .sqlite_batch_writer
                .set_runtime_attempt_store(state.proxy_runtime_attempts.clone());
            let runtime_attempts = state
                .proxy_runtime_attempts
                .claim_for_invocation(&persisted_record.invoke_id, &persisted_record.occurred_at);
            debug!(
                invoke_id = %invoke_id,
                terminal_delta_applied_selection_count = delta.applied_selection_count,
                terminal_delta_duplicate = delta.duplicate,
                terminal_delta_skipped_out_of_range_count = delta.skipped_out_of_range_count,
                response_source = "memory",
                "registered terminal record in dashboard activity read model before sqlite enqueue"
            );
            let terminal_enqueue = state
                .sqlite_batch_writer
                .enqueue_runtime_terminal(BatchedTerminalInvocationWrite {
                    enqueued_at: None,
                    diagnostic: crate::observability::diagnostics::current()
                        .map(|c| c.persistence()),
                    record: record.clone(),
                    attempts: runtime_attempts.clone(),
                    capture_started: None,
                    raw_capture: false,
                    dashboard_terminal_sequence: delta.terminal_sequence,
                    terminal_projection_event_ids: projection.event_id.into_iter().collect(),
                    startup_backfill_tasks,
                })
                .await;
            let terminal_enqueued = terminal_enqueue.enqueued;
            if !terminal_enqueued {
                state.proxy_runtime_attempts.restore(runtime_attempts);
                rollback_terminal_projection_before_enqueue(state, &persisted_record, &projection)
                    .await;
                let terminal_tombstone_cleared =
                    state.proxy_runtime_invocations.clear_terminal_tombstone(
                        &persisted_record.invoke_id,
                        &persisted_record.occurred_at,
                    );
                warn!(
                    invoke_id = %invoke_id,
                    occurred_at = %persisted_record.occurred_at,
                    enqueue_failed_by_class = "terminal_invocation",
                    terminal_tombstone_cleared,
                    durability_mode = terminal_enqueue.durability_mode.as_str(),
                    journal_sequence = ?terminal_enqueue.journal_sequence,
                    business_unblocked_record_write = true,
                    record_flush_deferred_or_failed = "terminal_invocation_enqueue_failed",
                    "terminal proxy capture record dropped by sqlite write controller"
                );
                return Err(anyhow!("proxy capture terminal record could not be queued"));
            } else {
                debug!(
                    invoke_id = %invoke_id,
                    terminal_record_enqueue_elapsed = enqueue_started.elapsed().as_millis() as u64,
                    durability_mode = terminal_enqueue.durability_mode.as_str(),
                    journal_sequence = ?terminal_enqueue.journal_sequence,
                    journal_pending_records = terminal_enqueue.journal_pending_records,
                    journal_pending_bytes = terminal_enqueue.journal_pending_bytes,
                    business_unblocked_record_write = true,
                    record_flush_deferred_or_failed = "terminal_invocation_enqueued_async",
                    "terminal proxy capture record queued for sqlite write controller"
                );
            }
            #[cfg(test)]
            if terminal_enqueued && state.sqlite_batch_writer.auto_flush_terminal_for_test() {
                state
                    .sqlite_batch_writer
                    .flush_buffered_for_test(&state.pool)
                    .await;
            }
            if terminal_enqueued {
                state
                    .subscription_hub
                    .publish_runtime_mutation(RuntimeMutation::invocation(
                        &persisted_record,
                        RuntimeMutationKind::TerminalCommitted,
                    ));
                #[cfg(test)]
                broadcast_test_record_payload(state, &persisted_record);
            }
            if terminal_enqueued {
                schedule_dashboard_activity_live_snapshot(state);
                schedule_proxy_capture_follow_up_after_terminal_enqueue(
                    state,
                    &invoke_id,
                    "runtime_terminal",
                );
            }

            Ok(())
        },
    )
    .await
}

pub(crate) async fn persist_proxy_capture_runtime_record(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    persist_proxy_capture_runtime_record_core(pool, record, true).await
}

pub(crate) async fn persist_proxy_capture_runtime_record_core(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
    write_derived_inline: bool,
) -> Result<Option<ApiInvocation>> {
    let mut tx = crate::observability::diagnostics::wait(
        crate::observability::diagnostics::Resource::DbPool,
        pool.begin(),
    )
    .await?;
    let persisted =
        persist_proxy_capture_runtime_record_tx(tx.as_mut(), record, write_derived_inline).await?;
    tx.commit().await?;
    Ok(persisted)
}

pub(crate) async fn persist_proxy_capture_runtime_record_tx(
    tx: &mut SqliteConnection,
    record: ProxyCaptureRecord,
    write_derived_inline: bool,
) -> Result<Option<ApiInvocation>> {
    let raw_response = if record.response_body_preview_enabled {
        record.raw_response.clone()
    } else {
        String::new()
    };
    let resp_raw = if record.response_body_preview_enabled {
        record.resp_raw.clone()
    } else {
        RawPayloadMeta {
            path: None,
            size_bytes: record.resp_raw.size_bytes,
            truncated: record.resp_raw.truncated,
            truncated_reason: record.resp_raw.truncated_reason.clone(),
            write_fence: None,
        }
    };
    let failure = resolve_failure_classification(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        None,
        None,
    );
    let failure_kind = failure.failure_kind.clone();
    let t_req_read_ms = nullable_runtime_timing_value(record.timings.t_req_read_ms);
    let t_req_parse_ms = nullable_runtime_timing_value(record.timings.t_req_parse_ms);
    let t_upstream_connect_ms = nullable_runtime_timing_value(record.timings.t_upstream_connect_ms);
    let t_upstream_ttfb_ms = nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms);
    let first_token_ms = record
        .timings
        .first_token_ms
        .filter(|value| value.is_finite() && *value >= 0.0);
    let core_write_started = Instant::now();
    let created_at = format_utc_iso_millis(Utc::now());
    let mut core_write_path = "insert_missing";
    let existing_identity =
        load_persisted_invocation_identity_tx(&mut *tx, &record.invoke_id, &record.occurred_at)
            .await?;
    if let Some(existing) = existing_identity.as_ref()
        && !persisted_invocation_allows_proxy_record_update(
            existing.status.as_deref(),
            existing.failure_kind.as_deref(),
            &record.status,
        )
    {
        return Ok(None);
    }

    if let Some(existing) = existing_identity.as_ref() {
        let updated = update_existing_proxy_invocation_record_tx(
            &mut *tx,
            existing.id,
            &record,
            &raw_response,
            &resp_raw,
            failure_kind.as_deref(),
            failure.failure_class.as_str(),
            failure.is_actionable,
            None,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms,
            None,
            None,
            None,
        )
        .await?;
        if !updated {
            return Ok(None);
        }
        core_write_path = "update_existing";
    } else {
        let insert_result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                model,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                reasoning_tokens,
                total_tokens,
                cost,
                cost_input,
                cost_cache_write,
                cost_cache_read,
                cost_output,
                cost_reasoning,
                cost_estimated,
                price_version,
                status,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                raw_response,
                request_raw_path,
                request_raw_codec,
                request_raw_size,
                request_raw_truncated,
                request_raw_truncated_reason,
                response_raw_path,
                response_raw_codec,
                response_raw_size,
                response_raw_truncated,
                response_raw_truncated_reason,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms,
                created_at,
                reported_cache_write_tokens
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36,
                ?37, ?38, ?39, ?40, ?41, ?42, ?43, ?44, ?45
            )
            "#,
        )
        .bind(&record.invoke_id)
        .bind(&record.occurred_at)
        .bind(SOURCE_PROXY)
        .bind(&record.model)
        .bind(record.usage.input_tokens)
        .bind(record.usage.output_tokens)
        .bind(record.usage.cache_input_tokens)
        .bind(record.usage.reasoning_tokens)
        .bind(record.usage.total_tokens)
        .bind(record.cost)
        .bind(record.cost_breakdown.map(|value| value.input))
        .bind(record.cost_breakdown.map(|value| value.cache_write))
        .bind(record.cost_breakdown.map(|value| value.cache_read))
        .bind(record.cost_breakdown.map(|value| value.output))
        .bind(record.cost_breakdown.map(|value| value.reasoning))
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(&raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(raw_payload_meta_codec(&record.req_raw))
        .bind(record.req_raw.size_bytes)
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(resp_raw.path.as_deref())
        .bind(raw_payload_meta_codec(&resp_raw))
        .bind(resp_raw.size_bytes)
        .bind(resp_raw.truncated as i64)
        .bind(resp_raw.truncated_reason.as_deref())
        .bind(None::<f64>)
        .bind(t_req_read_ms)
        .bind(t_req_parse_ms)
        .bind(t_upstream_connect_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(first_token_ms)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(created_at)
        .bind(record.usage.reported_cache_write_tokens)
        .execute(&mut *tx)
        .await?;
        if insert_result.rows_affected() == 0 {
            let Some(existing) = load_persisted_invocation_identity_tx(
                &mut *tx,
                &record.invoke_id,
                &record.occurred_at,
            )
            .await?
            else {
                return Ok(None);
            };
            if !persisted_invocation_allows_proxy_record_update(
                existing.status.as_deref(),
                existing.failure_kind.as_deref(),
                &record.status,
            ) {
                return Ok(None);
            }
            let updated = update_existing_proxy_invocation_record_tx(
                &mut *tx,
                existing.id,
                &record,
                &raw_response,
                &resp_raw,
                failure_kind.as_deref(),
                failure.failure_class.as_str(),
                failure.is_actionable,
                None,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                None,
                None,
                None,
            )
            .await?;
            if !updated {
                return Ok(None);
            }
            core_write_path = "update_race";
        }
    }

    let persisted_identity =
        load_persisted_invocation_identity_tx(&mut *tx, &record.invoke_id, &record.occurred_at)
            .await?
            .ok_or_else(|| {
                anyhow!("persisted proxy runtime invocation row disappeared after upsert")
            })?;
    if write_derived_inline {
        upsert_invocation_hourly_rollups_tx(
            &mut *tx,
            &[InvocationHourlySourceRecord {
                id: persisted_identity.id,
                occurred_at: record.occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some(record.status.clone()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                model: record.model.clone(),
                input_tokens: record.usage.input_tokens,
                output_tokens: record.usage.output_tokens,
                cache_input_tokens: record.usage.cache_input_tokens,
                reasoning_tokens: record.usage.reasoning_tokens,
                total_tokens: record.usage.total_tokens,
                cost: record.cost,
                upstream_account_id: crate::proxy::upstream_account_id_from_payload(
                    record.payload.as_deref(),
                ),
                cost_input: record.cost_breakdown.map(|value| value.input),
                cost_cache_write: record.cost_breakdown.map(|value| value.cache_write),
                cost_cache_read: record.cost_breakdown.map(|value| value.cache_read),
                cost_output: record.cost_breakdown.map(|value| value.output),
                cost_reasoning: record.cost_breakdown.map(|value| value.reasoning),
                error_message: record.error_message.clone(),
                failure_kind: failure_kind.clone(),
                failure_class: Some(failure.failure_class.as_str().to_string()),
                is_actionable: Some(failure.is_actionable as i64),
                payload: record.payload.clone(),
                t_total_ms: None,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            }],
            &INVOCATION_HOURLY_ROLLUP_TARGETS,
        )
        .await?;
        save_hourly_rollup_live_progress_tx(
            &mut *tx,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            persisted_identity.id,
        )
        .await?;
        touch_invocation_upstream_account_last_activity_tx(
            &mut *tx,
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
    }

    let persisted =
        load_persisted_api_invocation_tx(&mut *tx, &record.invoke_id, &record.occurred_at).await?;

    let core_write_elapsed_ms = core_write_started.elapsed().as_millis() as u64;
    if core_write_elapsed_ms >= 1_000 {
        warn!(
            invoke_id = %record.invoke_id,
            status = %record.status,
            core_write_path,
            request_raw_bytes = record.req_raw.size_bytes,
            response_raw_bytes = resp_raw.size_bytes,
            has_request_raw_path = record.req_raw.path.is_some(),
            has_response_raw_path = resp_raw.path.is_some(),
            elapsed_ms = core_write_elapsed_ms,
            "proxy capture core invocation write was slow"
        );
    } else {
        debug!(
            invoke_id = %record.invoke_id,
            status = %record.status,
            core_write_path,
            request_raw_bytes = record.req_raw.size_bytes,
            response_raw_bytes = resp_raw.size_bytes,
            has_request_raw_path = record.req_raw.path.is_some(),
            has_response_raw_path = resp_raw.path.is_some(),
            elapsed_ms = core_write_elapsed_ms,
            "proxy capture core invocation write completed"
        );
    }

    Ok(Some(persisted))
}

pub(crate) fn build_running_proxy_capture_record(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    pool_route_active: bool,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
    upstream_account_kind: Option<&str>,
    upstream_base_url_host: Option<&str>,
    proxy_display_name: Option<&str>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&str>,
    response_content_encoding: Option<&str>,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
    t_upstream_connect_ms: f64,
    t_upstream_ttfb_ms: f64,
) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: request_info.model.clone(),
        usage: ParsedUsage::default(),
        cost: None,
        cost_breakdown: None,
        cost_estimated: false,
        price_version: None,
        status: "running".to_string(),
        error_message: None,
        failure_kind: None,
        payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
            target,
            status: StatusCode::OK,
            is_stream: request_info.is_stream,
            request_contains_encrypted_content: request_info.contains_encrypted_content,
            response_contains_encrypted_content: false,
            compaction_request_kind: request_info.compaction_request_kind,
            compaction_response_kind: None,
            image_intent: request_info.image_intent.as_deref(),
            request_model: request_info.model.as_deref(),
            requested_service_tier: request_info.requested_service_tier.as_deref(),
            billing_service_tier: None,
            reasoning_effort: request_info.reasoning_effort.as_deref(),
            response_model: None,
            usage_missing_reason: None,
            request_parse_error: request_info.parse_error.as_deref(),
            request_compression_algorithm: None,
            request_compression_mode: None,
            request_compression_logical_body_bytes: None,
            request_compression_transmitted_body_bytes: None,
            request_compression_transmission_complete: None,
            failure_kind: None,
            requester_ip,
            request_user_agent: None,
            request_x_forwarded_for: None,
            request_forwarded: None,
            request_x_real_ip: None,
            upstream_scope: if pool_route_active {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_route_active {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key,
            prompt_cache_key,
            prompt_cache_key_attribution_source: request_info
                .prompt_cache_key_attribution_source
                .as_deref(),
            client_fingerprint: None,
            client_header_fingerprints: None,
            upstream_account_id,
            upstream_account_name,
            upstream_account_kind,
            upstream_base_url_host,
            oauth_account_header_attached: None,
            oauth_account_id_shape: None,
            oauth_forwarded_header_count: None,
            oauth_forwarded_header_names: None,
            oauth_fingerprint_version: None,
            oauth_forwarded_header_fingerprints: None,
            oauth_prompt_cache_header_forwarded: None,
            oauth_request_body_prefix_fingerprint: None,
            oauth_request_body_prefix_bytes: None,
            oauth_request_body_snapshot_kind: None,
            oauth_responses_body_mode: None,
            oauth_responses_rewrite: None,
            service_tier: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_status_code: None,
            downstream_error_message: None,
            upstream_request_id: None,
            response_content_encoding,
            stream_failure_origin: None,
            upstream_read_error_kind: None,
            content_encoding_chain: None,
            forwarded_chunk_count: None,
            forwarded_bytes: None,
            usage_observed: None,
            downstream_close_phase: None,
            downstream_write_error_kind: None,
            last_upstream_chunk_gap_ms: None,
            upstream_approx_upload_bytes: None,
            upstream_approx_download_bytes: None,
            proxy_display_name,
            proxy_weight_delta: None,
            pool_attempt_count,
            pool_distinct_account_count,
            pool_attempt_terminal_reason,
            blocked_binding: None,
        })),
        raw_response: "{}".to_string(),
        response_body_preview_enabled: false,
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: StageTimings {
            t_total_ms: 0.0,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            first_token_ms: None,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        },
    }
}

pub(crate) fn build_admitted_proxy_capture_runtime_snapshot(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
) -> ProxyCaptureRecord {
    let request_info = RequestCaptureInfo {
        sticky_key: sticky_key.map(ToOwned::to_owned),
        prompt_cache_key: prompt_cache_key.map(ToOwned::to_owned),
        prompt_cache_key_attribution_source: prompt_cache_key.map(|_| "request".to_string()),
        ..RequestCaptureInfo::default()
    };
    build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        target,
        &request_info,
        requester_ip,
        sticky_key,
        prompt_cache_key,
        true,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        0.0,
        0.0,
        0.0,
        0.0,
    )
}

pub(crate) fn resolve_invocation_proxy_display_name(
    selected_proxy: Option<&SelectedForwardProxy>,
) -> Option<String> {
    selected_proxy.map(|proxy| proxy.display_name.clone())
}

pub(crate) fn summarize_response_content_encoding(content_encoding: Option<&str>) -> String {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        "identity".to_string()
    } else {
        encodings.join(", ")
    }
}
