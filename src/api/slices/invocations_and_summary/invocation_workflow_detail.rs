use super::*;

pub(crate) async fn fetch_invocation_pool_attempts(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(invoke_id): axum::extract::Path<String>,
) -> Result<Json<Vec<ApiPoolUpstreamRequestAttempt>>, ApiError> {
    Ok(Json(
        query_pool_attempt_records_from_live(&state.pool, &invoke_id).await?,
    ))
}

#[derive(Debug, FromRow)]
struct InvocationWorkflowIdentityRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    timeline_json: Option<String>,
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationWorkflowAttemptRow {
    pub(crate) attempt_row_id: i64,
    pub(crate) attempt_id: Option<String>,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) routing_source: Option<String>,
    pub(crate) routing_selection_audit_json: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) upstream_route_key: Option<String>,
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    pub(crate) requester_ip: Option<String>,
    pub(crate) started_at: Option<String>,
    pub(crate) finished_at: Option<String>,
    pub(crate) status: String,
    pub(crate) phase: Option<String>,
    pub(crate) http_status: Option<i64>,
    pub(crate) downstream_http_status: Option<i64>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) connect_latency_ms: Option<f64>,
    pub(crate) first_byte_latency_ms: Option<f64>,
    pub(crate) stream_latency_ms: Option<f64>,
    pub(crate) upstream_request_id: Option<String>,
    pub(crate) upstream_request_compression_algorithm: Option<String>,
    pub(crate) upstream_request_compression_mode: Option<String>,
    pub(crate) upstream_request_logical_body_bytes: Option<i64>,
    pub(crate) upstream_request_transmitted_body_bytes: Option<i64>,
    pub(crate) upstream_request_header_bytes_approx: Option<i64>,
    pub(crate) upstream_response_body_bytes: Option<i64>,
    pub(crate) upstream_response_header_bytes_approx: Option<i64>,
    pub(crate) compact_support_status: Option<String>,
    pub(crate) compact_support_reason: Option<String>,
    pub(crate) request_summary_json: Option<String>,
    pub(crate) response_summary_json: Option<String>,
    pub(crate) response_raw_path: Option<String>,
    pub(crate) response_raw_codec: Option<String>,
    pub(crate) response_raw_size: Option<i64>,
    pub(crate) response_raw_truncated: Option<i64>,
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowDetailResponse {
    pub(crate) hero: InvocationWorkflowHero,
    pub(crate) timeline: Vec<InvocationWorkflowTimelineEntry>,
    pub(crate) reconstructed: bool,
    pub(crate) partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) partial_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowHero {
    pub(crate) record_id: i64,
    pub(crate) invoke_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) route_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) final_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) downstream_status_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) total_duration_ms: Option<f64>,
    pub(crate) timeline_attempt_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pool_attempt_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) occurred_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pool_routing_no_candidate_audit: Option<PoolRoutingNoCandidateAudit>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowAttempt {
    pub(crate) synthetic: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attempt_id: Option<String>,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sticky_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_selection_audit: Option<PoolRoutingSelectionAudit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_route_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) requester_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<String>,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) http_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) downstream_http_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) downstream_error_message: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_finite_nonnegative_timing"
    )]
    pub(crate) connect_latency_ms: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_finite_nonnegative_timing"
    )]
    pub(crate) first_token_ms: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_finite_nonnegative_timing"
    )]
    pub(crate) first_byte_latency_ms: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_finite_positive_timing"
    )]
    pub(crate) stream_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_summary: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_summary: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowResponseBody {
    pub(crate) available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowTimelineEntry {
    pub(crate) block_id: String,
    pub(crate) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) occurred_at: Option<String>,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attempt: Option<InvocationWorkflowAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_body: Option<InvocationWorkflowResponseBody>,
}

fn normalize_optional_timestamp(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim();
    if normalized.is_empty() {
        return None;
    }
    parse_to_utc_datetime(normalized)
        .map(format_utc_iso)
        .or_else(|| Some(normalized.to_string()))
}

fn parse_summary_json_or_fallback(
    raw: Option<&str>,
    fallback: impl FnOnce() -> Value,
) -> Option<Value> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => serde_json::from_str::<Value>(raw)
            .ok()
            .or_else(|| Some(fallback())),
        None => Some(fallback()),
    }
}

fn sanitize_response_summary_timing(
    mut summary: Value,
    attempt: &InvocationWorkflowAttemptRow,
    is_final_attempt: bool,
) -> Value {
    let Some(latency) = summary.get_mut("latencyMs").and_then(Value::as_object_mut) else {
        return summary;
    };

    for key in [
        "connect",
        "firstByte",
        "requestRead",
        "requestParse",
        "responseParse",
        "persist",
        "total",
    ] {
        if latency.get(key).is_some_and(|value| {
            value
                .as_f64()
                .is_none_or(|value| !value.is_finite() || value < 0.0)
        }) {
            latency.insert(key.to_string(), Value::Null);
        }
    }

    if is_final_attempt || latency.contains_key("stream") {
        let stream = if is_final_attempt {
            final_attempt_has_stream_evidence(attempt)
                .then_some(finite_positive_timing(attempt.stream_latency_ms))
                .flatten()
        } else {
            latency
                .get("stream")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value > 0.0)
        };
        latency.insert(
            "stream".to_string(),
            stream.map_or(Value::Null, |value| json!(value)),
        );
    }

    summary
}

fn merge_attempt_request_summary_json(raw: Option<&str>, fallback: Value) -> Option<Value> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Some(fallback);
    };
    let Ok(stored) = serde_json::from_str::<Value>(raw) else {
        return Some(fallback);
    };
    let (Some(mut fallback), Some(stored)) = (fallback.as_object().cloned(), stored.as_object())
    else {
        return Some(fallback);
    };
    for (key, value) in stored {
        fallback.insert(key.clone(), value.clone());
    }
    Some(Value::Object(fallback))
}

fn parse_optional_json_value(raw: Option<&str>) -> Option<Value> {
    raw.and_then(|value| {
        let normalized = value.trim();
        if normalized.is_empty() {
            None
        } else {
            serde_json::from_str::<Value>(normalized).ok()
        }
    })
}

fn payload_value<'a>(payload: Option<&'a Value>, keys: &[&str]) -> Option<&'a Value> {
    let object = payload?.as_object()?;
    keys.iter().find_map(|key| object.get(*key))
}

fn payload_string(payload: Option<&Value>, keys: &[&str]) -> Option<String> {
    payload_value(payload, keys)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn payload_bool(payload: Option<&Value>, keys: &[&str]) -> Option<bool> {
    payload_value(payload, keys).and_then(Value::as_bool)
}

fn payload_i64(payload: Option<&Value>, keys: &[&str]) -> Option<i64> {
    payload_value(payload, keys).and_then(Value::as_i64)
}

fn payload_u64(payload: Option<&Value>, keys: &[&str]) -> Option<u64> {
    payload_value(payload, keys).and_then(Value::as_u64)
}

fn payload_f64(payload: Option<&Value>, keys: &[&str]) -> Option<f64> {
    payload_value(payload, keys).and_then(Value::as_f64)
}

fn payload_string_array(payload: Option<&Value>, keys: &[&str]) -> Option<Vec<String>> {
    payload_value(payload, keys)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|entries| !entries.is_empty())
}

fn payload_clone(payload: Option<&Value>, keys: &[&str]) -> Option<Value> {
    payload_value(payload, keys).cloned()
}

fn request_compression_value(
    algorithm: Option<String>,
    mode: Option<String>,
    derived: RequestCompressionDerivedFields,
) -> Value {
    json!({
        "algorithm": algorithm,
        "mode": mode,
        "logicalBodyBytes": derived.logical_body_bytes,
        "transmittedBodyBytes": derived.transmitted_body_bytes,
        "savedBytes": derived.saved_bytes,
        "ratioPct": derived.ratio_pct,
        "approxUploadBytes": derived.approx_upload_bytes,
        "approxDownloadBytes": derived.approx_download_bytes,
    })
}

fn derive_request_compression_from_payload(
    payload: Option<&Value>,
) -> RequestCompressionDerivedFields {
    let mut derived = derive_request_compression_fields(
        payload_i64(payload, &["requestCompressionLogicalBodyBytes"]),
        payload_i64(payload, &["requestCompressionTransmittedBodyBytes"]),
        None,
        None,
        None,
        payload_bool(payload, &["requestCompressionTransmissionComplete"]).unwrap_or(false),
    );
    derived.approx_upload_bytes =
        payload_i64(payload, &["upstreamApproxUploadBytes"]).filter(|value| *value >= 0);
    derived.approx_download_bytes =
        payload_i64(payload, &["upstreamApproxDownloadBytes"]).filter(|value| *value >= 0);
    derived
}

fn build_request_header_snapshot(payload: Option<&Value>) -> Value {
    json!({
        "userAgent": payload_string(payload, &["requestUserAgent"]),
        "xForwardedFor": payload_string(payload, &["requestXForwardedFor"]),
        "forwarded": payload_string(payload, &["requestForwarded"]),
        "xRealIp": payload_string(payload, &["requestXRealIp"]),
    })
}

fn build_request_routing_snapshot(
    record: &ApiInvocation,
    attempt: Option<&InvocationWorkflowAttemptRow>,
    payload: Option<&Value>,
) -> Value {
    json!({
        "routeMode": record.route_mode.clone(),
        "upstreamScope": payload_string(payload, &["upstreamScope"]),
        "stickyKey": attempt
            .and_then(|row| row.sticky_key.clone())
            .or_else(|| record.sticky_key.clone())
            .or_else(|| payload_string(payload, &["stickyKey"])),
        "promptCacheKey": record
            .prompt_cache_key
            .clone()
            .or_else(|| payload_string(payload, &["promptCacheKey"])),
        "proxyDisplayName": payload_string(payload, &["proxyDisplayName"]),
        "upstreamRouteKey": attempt
            .and_then(|row| row.upstream_route_key.clone())
            .or_else(|| payload_string(payload, &["upstreamRouteKey"])),
        "proxyBindingKey": attempt
            .and_then(|row| row.proxy_binding_key_snapshot.clone())
            .or_else(|| payload_string(payload, &["proxyBindingKey", "proxyBindingKeySnapshot"])),
        "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
        "clientHeaderFingerprints": payload_clone(payload, &["clientHeaderFingerprints"]),
        "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
        "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
    })
}

fn build_request_client_snapshot(payload: Option<&Value>) -> Value {
    json!({
        "requestContainsEncryptedContent": payload_bool(payload, &["requestContainsEncryptedContent"]),
        "requestParseError": payload_string(payload, &["requestParseError"]),
        "oauthAccountHeaderAttached": payload_bool(payload, &["oauthAccountHeaderAttached"]),
        "oauthAccountIdShape": payload_string(payload, &["oauthAccountIdShape"]),
        "oauthRequestBodyPrefixFingerprint": payload_string(payload, &["oauthRequestBodyPrefixFingerprint"]),
        "oauthRequestBodyPrefixBytes": payload_u64(payload, &["oauthRequestBodyPrefixBytes"]),
        "oauthRequestBodySnapshotKind": payload_string(payload, &["oauthRequestBodySnapshotKind"]),
        "oauthResponsesBodyMode": payload_string(payload, &["oauthResponsesBodyMode"]),
        "oauthResponsesRewrite": payload_clone(payload, &["oauthResponsesRewrite"]),
    })
}

fn build_response_header_snapshot(
    record: &ApiInvocation,
    attempt: Option<&InvocationWorkflowAttemptRow>,
    payload: Option<&Value>,
    allow_invocation_level_fields: bool,
) -> Value {
    let content_encoding = attempt
        .and_then(|attempt| attempt.response_content_encoding.clone())
        .or_else(|| {
            allow_invocation_level_fields
                .then(|| {
                    record
                        .response_content_encoding
                        .clone()
                        .or_else(|| payload_string(payload, &["responseContentEncoding"]))
                })
                .flatten()
        });
    let content_encoding_chain = allow_invocation_level_fields
        .then(|| payload_string(payload, &["contentEncodingChain"]))
        .flatten();
    let upstream_request_id = attempt
        .and_then(|attempt| attempt.upstream_request_id.clone())
        .or_else(|| {
            allow_invocation_level_fields
                .then(|| {
                    record
                        .upstream_request_id
                        .clone()
                        .or_else(|| payload_string(payload, &["upstreamRequestId"]))
                })
                .flatten()
        });

    json!({
        "contentEncoding": content_encoding,
        "contentEncodingChain": content_encoding_chain,
        "upstreamRequestId": upstream_request_id,
        "cvmInvokeId": Some(record.invoke_id.clone()),
    })
}

fn build_response_delivery_snapshot(payload: Option<&Value>) -> Value {
    json!({
        "forwardedChunkCount": payload_u64(payload, &["forwardedChunkCount"]),
        "forwardedBytes": payload_u64(payload, &["forwardedBytes"]),
        "usageObserved": payload_bool(payload, &["usageObserved"]),
        "downstreamClosePhase": payload_string(payload, &["downstreamClosePhase"]),
        "downstreamWriteErrorKind": payload_string(payload, &["downstreamWriteErrorKind"]),
        "lastUpstreamChunkGapMs": payload_u64(payload, &["lastUpstreamChunkGapMs"]),
        "streamFailureOrigin": payload_string(payload, &["streamFailureOrigin"]),
        "upstreamReadErrorKind": payload_string(payload, &["upstreamReadErrorKind"]),
        "responseContainsEncryptedContent": payload_bool(payload, &["responseContainsEncryptedContent"]),
    })
}

fn invocation_status_is_success_like(record: &ApiInvocation) -> bool {
    matches!(
        normalized_runtime_text(record.status.as_deref()).as_str(),
        "success" | "completed" | "warning_success"
    )
}

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

pub(crate) fn resolve_invocation_cache_write_tokens(record: &ApiInvocation) -> Option<i64> {
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

pub(crate) fn build_invocation_cost_audit(
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

pub(crate) fn apply_invocation_cost_audits(
    records: &mut [ApiInvocation],
    catalog: &PricingCatalog,
) {
    for record in records {
        record.cost_audit = build_invocation_cost_audit(record, catalog, false);
    }
}

pub(crate) fn build_invocation_usage_summary(
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

pub(crate) fn build_attempt_response_summary(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
    is_final_attempt: bool,
) -> Value {
    let invocation_failure_kind = is_final_attempt
        .then_some(record.failure_kind.as_ref())
        .flatten();
    let invocation_error_message = is_final_attempt
        .then_some(record.error_message.as_ref())
        .flatten();
    let invocation_downstream_error_message = is_final_attempt
        .then_some(record.downstream_error_message.as_ref())
        .flatten();
    let invocation_upstream_request_id = is_final_attempt
        .then_some(record.upstream_request_id.as_ref())
        .flatten();
    let invocation_response_content_encoding =
        attempt.response_content_encoding.clone().or_else(|| {
            is_final_attempt
                .then(|| record.response_content_encoding.clone())
                .flatten()
        });
    let invocation_response_payload = is_final_attempt.then_some(payload).flatten();
    let attempt_response_body_captured = attempt.response_raw_path.is_some();
    let response_body_capture_size = attempt
        .response_raw_size
        .or_else(|| {
            is_final_attempt
                .then_some(record.response_raw_size)
                .flatten()
        })
        .or(attempt.upstream_response_body_bytes);
    let response_body_capture_truncated = if attempt_response_body_captured {
        attempt.response_raw_truncated.unwrap_or_default() != 0
    } else if is_final_attempt && record.response_raw_path.is_some() {
        record.response_raw_truncated.unwrap_or_default() != 0
    } else {
        false
    };
    let response_body_capture_truncated_reason =
        attempt.response_raw_truncated_reason.clone().or_else(|| {
            is_final_attempt
                .then(|| record.response_raw_truncated_reason.clone())
                .flatten()
        });
    let response_body_capture_detail_level = if attempt_response_body_captured {
        Some(record.detail_level.clone())
    } else if is_final_attempt && record.response_raw_path.is_some() {
        Some("full".to_string())
    } else {
        Some("attempt_metrics".to_string())
    };

    json!({
        "status": attempt.status.clone(),
        "phase": attempt.phase.clone(),
        "httpStatus": attempt.http_status,
        "compactionResponseKind": is_final_attempt.then(|| record.compaction_response_kind.clone()).flatten(),
        "failureKind": attempt.failure_kind.as_ref().or(invocation_failure_kind),
        "errorMessage": attempt.error_message.as_ref().or(invocation_error_message),
        "downstreamErrorMessage": attempt
            .downstream_error_message
            .as_ref()
            .or(invocation_downstream_error_message),
        "upstreamRequestId": attempt.upstream_request_id.as_ref().or(invocation_upstream_request_id),
        "upstreamErrorCode": is_final_attempt.then(|| record.upstream_error_code.clone()).flatten(),
        "upstreamErrorMessage": is_final_attempt.then(|| record.upstream_error_message.clone()).flatten(),
        "streamTerminalEvent": is_final_attempt.then(|| record.stream_terminal_event.clone()).flatten(),
        "responseContentEncoding": invocation_response_content_encoding,
        "serviceTier": is_final_attempt.then(|| record.service_tier.clone()).flatten(),
        "billingServiceTier": is_final_attempt.then(|| record.billing_service_tier.clone()).flatten(),
        "headers": build_response_header_snapshot(record, Some(attempt), payload, is_final_attempt),
        "delivery": build_response_delivery_snapshot(invocation_response_payload),
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
            "availableAtAttemptLevel": attempt_response_body_captured,
            "availableAtInvocationLevel": is_final_attempt && record.response_raw_path.is_some(),
            "size": response_body_capture_size,
            "truncated": response_body_capture_truncated,
            "truncatedReason": response_body_capture_truncated_reason,
            "detailLevel": response_body_capture_detail_level,
            "detailPruneReason": is_final_attempt.then(|| record.detail_prune_reason.clone()).flatten(),
            "unavailableReason": (!(attempt_response_body_captured || (is_final_attempt && record.response_raw_path.is_some())))
                .then_some("attempt_response_body_not_captured"),
        },
        "usage": usage_cost_audit.map(|audit| build_invocation_usage_summary(record, audit)),
    })
}

pub(crate) fn build_workflow_attempt_from_row(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
    payload: Option<&Value>,
    usage_cost_audit: Option<&InvocationCostAudit>,
    is_final_attempt: bool,
) -> InvocationWorkflowAttempt {
    let invocation_failure_kind = is_final_attempt
        .then(|| record.failure_kind.clone())
        .flatten();
    let invocation_error_message = is_final_attempt
        .then(|| record.error_message.clone())
        .flatten();
    let invocation_downstream_error_message = is_final_attempt
        .then(|| record.downstream_error_message.clone())
        .flatten();
    let invocation_upstream_request_id = is_final_attempt
        .then(|| record.upstream_request_id.clone())
        .flatten();

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
        request_summary: merge_attempt_request_summary_json(
            attempt.request_summary_json.as_deref(),
            build_attempt_request_summary(record, attempt, payload),
        ),
        response_summary: parse_summary_json_or_fallback(
            attempt.response_summary_json.as_deref(),
            || {
                build_attempt_response_summary(
                    record,
                    attempt,
                    payload,
                    usage_cost_audit,
                    is_final_attempt,
                )
            },
        )
        .map(|summary| sanitize_response_summary_timing(summary, attempt, is_final_attempt)),
    }
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
    let response_summary = json!({
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
    });
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

pub(crate) fn last_success_like_attempt_row_id(
    attempt_rows: &[&InvocationWorkflowAttemptRow],
) -> Option<i64> {
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

pub(crate) fn final_real_attempt_row_id(attempts: &[&InvocationWorkflowAttemptRow]) -> Option<i64> {
    attempts
        .iter()
        .filter(|attempt| {
            normalized_runtime_text(Some(attempt.status.as_str())) != "budget_exhausted_final"
        })
        .max_by_key(|attempt| (attempt.attempt_index, attempt.attempt_row_id))
        .map(|attempt| attempt.attempt_row_id)
}

fn build_workflow_timeline_entries(
    record: &ApiInvocation,
    attempts: &[InvocationWorkflowAttempt],
    route_only_attempt: Option<&InvocationWorkflowAttempt>,
    failure_entry: Option<InvocationWorkflowTimelineEntry>,
) -> Vec<InvocationWorkflowTimelineEntry> {
    let mut entries = Vec::new();
    if let Some(route_only_attempt) = route_only_attempt {
        entries.push(build_routing_timeline_entry(
            route_only_attempt
                .attempt_id
                .clone()
                .map(|attempt_id| format!("route-{attempt_id}"))
                .unwrap_or_else(|| "route-terminal".to_string()),
            route_only_attempt,
        ));
    } else if attempts.len() == 1 && attempts[0].synthetic {
        let attempt = attempts[0].clone();
        entries.push(InvocationWorkflowTimelineEntry {
            block_id: "attempt-direct".to_string(),
            kind: "attempt".to_string(),
            occurred_at: Some(attempt.occurred_at.clone()),
            title: "Direct attempt".to_string(),
            subtitle: Some(attempt.endpoint.clone()),
            status: Some(attempt.status.clone()),
            attempt: Some(attempt),
            detail: None,
            response_body: None,
        });
    } else {
        let mut previous_finished_at: Option<DateTime<Utc>> = None;
        let mut previous_attempt_id: Option<String> = None;
        for attempt in attempts {
            if let Some(started_at) = attempt
                .started_at
                .as_deref()
                .and_then(parse_to_utc_datetime)
                && let Some(previous_finished) = previous_finished_at
            {
                let gap_ms = (started_at - previous_finished).num_milliseconds();
                if gap_ms > 0 {
                    entries.push(InvocationWorkflowTimelineEntry {
                        block_id: format!(
                            "wait-{}",
                            attempt
                                .attempt_id
                                .clone()
                                .unwrap_or_else(|| attempt.attempt_index.to_string())
                        ),
                        kind: "routingWait".to_string(),
                        occurred_at: Some(format_utc_iso(started_at)),
                        title: "Retry wait".to_string(),
                        subtitle: Some(format!("{} ms", gap_ms)),
                        status: None,
                        attempt: None,
                        detail: Some(json!({
                            "durationMs": gap_ms,
                            "fromAttemptId": previous_attempt_id.clone(),
                            "toAttemptId": attempt.attempt_id.clone(),
                        })),
                        response_body: None,
                    });
                }
            }

            entries.push(build_routing_timeline_entry(
                format!(
                    "route-{}",
                    attempt
                        .attempt_id
                        .clone()
                        .unwrap_or_else(|| attempt.attempt_index.to_string())
                ),
                attempt,
            ));

            entries.push(build_workflow_attempt_timeline_entry(attempt.clone()));

            previous_finished_at = attempt
                .finished_at
                .as_deref()
                .and_then(parse_to_utc_datetime);
            previous_attempt_id = attempt.attempt_id.clone();
        }
    }

    if let Some(failure_entry) = failure_entry {
        entries.push(failure_entry);
    }
    if entries.is_empty() && !invocation_status_is_success_like(record) {
        entries.push(InvocationWorkflowTimelineEntry {
            block_id: "failure-only".to_string(),
            kind: "systemFinalFailure".to_string(),
            occurred_at: Some(record.occurred_at.clone()),
            title: "Final downstream response".to_string(),
            subtitle: record.failure_kind.clone(),
            status: record.status.clone(),
            attempt: None,
            detail: Some(json!({
                "downstreamStatusCode": record.downstream_status_code,
                "failureKind": record.failure_kind.clone(),
                "errorMessage": record.error_message.clone(),
                "downstreamErrorMessage": record.downstream_error_message.clone(),
            })),
            response_body: None,
        });
    }
    entries
}

fn build_workflow_attempt_timeline_entry(
    attempt: InvocationWorkflowAttempt,
) -> InvocationWorkflowTimelineEntry {
    InvocationWorkflowTimelineEntry {
        block_id: format!(
            "attempt-{}",
            attempt
                .attempt_id
                .clone()
                .unwrap_or_else(|| attempt.attempt_index.to_string())
        ),
        kind: "attempt".to_string(),
        occurred_at: Some(attempt.occurred_at.clone()),
        title: format!("Attempt #{}", attempt.attempt_index),
        subtitle: Some(workflow_attempt_account_label(&attempt)),
        status: Some(attempt.status.clone()),
        attempt: Some(attempt),
        detail: None,
        response_body: None,
    }
}

pub(crate) async fn hydrate_upstream_account_attempt_workflow_entries(
    state: &AppState,
    records: &mut [ApiPoolUpstreamRequestAttempt],
) -> Result<(), ApiError> {
    if records.is_empty() {
        return Ok(());
    }

    let selectors = records
        .iter()
        .map(|record| (record.invoke_id.clone(), record.occurred_at.clone()))
        .collect::<HashSet<_>>();
    let pricing_catalog = state.pricing_catalog.read().await.clone();
    let mut hydrated = HashMap::<
        (String, String),
        Option<(
            ApiInvocation,
            HashMap<String, InvocationWorkflowTimelineEntry>,
        )>,
    >::new();

    for (invoke_id, occurred_at) in selectors {
        let record = match load_persisted_api_invocation(&state.pool, &invoke_id, &occurred_at)
            .await
        {
            Ok(record) => record,
            Err(err) => {
                debug!(
                    invoke_id,
                    occurred_at,
                    error = %err,
                    "skipping workflow hydration for account attempt without matching invocation"
                );
                hydrated.insert((invoke_id, occurred_at), None);
                continue;
            }
        };
        let body_row = fetch_invocation_response_body_row_by_id(&state.pool, record.id).await?;
        let payload_value = body_row
            .as_ref()
            .and_then(|row| parse_optional_json_value(row.payload.as_deref()));
        let attempt_rows = query_invocation_workflow_attempt_rows(
            &state.pool,
            &record.invoke_id,
            &record.occurred_at,
        )
        .await?;
        let real_attempt_rows = attempt_rows
            .iter()
            .filter(|attempt| !invocation_workflow_attempt_row_is_pseudo_terminal(attempt))
            .collect::<Vec<_>>();
        let final_attempt_row_id = final_real_attempt_row_id(&real_attempt_rows);
        let last_success_attempt_row_id = last_success_like_attempt_row_id(&real_attempt_rows);
        let usage_cost_audit = (invocation_status_is_success_like(&record)
            && invocation_has_usage_evidence(&record))
        .then(|| build_invocation_cost_audit(&record, &pricing_catalog, true))
        .flatten();
        let workflow_entries = real_attempt_rows
            .into_iter()
            .filter_map(|attempt_row| {
                let attempt = build_workflow_attempt_from_row(
                    &record,
                    attempt_row,
                    payload_value.as_ref(),
                    (last_success_attempt_row_id == Some(attempt_row.attempt_row_id))
                        .then_some(usage_cost_audit.as_ref())
                        .flatten(),
                    final_attempt_row_id == Some(attempt_row.attempt_row_id),
                );
                let attempt_id = attempt.attempt_id.clone()?;
                Some((attempt_id, build_workflow_attempt_timeline_entry(attempt)))
            })
            .collect::<HashMap<_, _>>();
        hydrated.insert(
            (record.invoke_id.clone(), record.occurred_at.clone()),
            Some((record, workflow_entries)),
        );
    }

    for item in records {
        let Some(Some((record, workflow_entries))) =
            hydrated.get(&(item.invoke_id.clone(), item.occurred_at.clone()))
        else {
            continue;
        };
        item.invocation_record = Some(record.clone());
        item.workflow_entry = workflow_entries.get(&item.attempt_id).cloned();
    }

    Ok(())
}

async fn load_invocation_workflow_identity(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<InvocationWorkflowIdentityRow>, ApiError> {
    sqlx::query_as::<_, InvocationWorkflowIdentityRow>(
        r#"
        SELECT id, invoke_id, occurred_at, timeline_json
        FROM codex_invocations
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)
}

async fn query_invocation_workflow_attempt_rows(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Vec<InvocationWorkflowAttemptRow>, ApiError> {
    sqlx::query_as::<_, InvocationWorkflowAttemptRow>(
        r#"
        SELECT
            attempts.id AS attempt_row_id,
            attempts.attempt_public_id AS attempt_id,
            attempts.invoke_id,
            attempts.occurred_at,
            attempts.endpoint,
            attempts.sticky_key,
            attempts.routing_source,
            attempts.routing_selection_audit_json,
            attempts.upstream_account_id,
            accounts.display_name AS upstream_account_name,
            attempts.upstream_route_key,
            attempts.proxy_binding_key_snapshot,
            attempts.attempt_index,
            attempts.distinct_account_index,
            attempts.same_account_retry_index,
            attempts.requester_ip,
            attempts.started_at,
            attempts.finished_at,
            attempts.status,
            COALESCE(
                attempts.phase,
                CASE
                    WHEN attempts.status = 'pending' THEN 'sending_request'
                    WHEN attempts.status = 'success' THEN 'completed'
                    ELSE 'failed'
                END
            ) AS phase,
            attempts.http_status,
            attempts.downstream_http_status,
            attempts.failure_kind,
            attempts.error_message,
            attempts.downstream_error_message,
            attempts.connect_latency_ms,
            attempts.first_byte_latency_ms,
            attempts.stream_latency_ms,
            attempts.upstream_request_id,
            attempts.upstream_request_compression_algorithm,
            attempts.upstream_request_compression_mode,
            attempts.upstream_request_logical_body_bytes,
            attempts.upstream_request_transmitted_body_bytes,
            attempts.upstream_request_header_bytes_approx,
            attempts.upstream_response_body_bytes,
            attempts.upstream_response_header_bytes_approx,
            attempts.compact_support_status,
            attempts.compact_support_reason,
            attempts.request_summary_json,
            attempts.response_summary_json,
            attempts.response_raw_path,
            attempts.response_raw_codec,
            attempts.response_raw_size,
            attempts.response_raw_truncated,
            attempts.response_raw_truncated_reason,
            attempts.response_content_encoding
        FROM pool_upstream_request_attempts AS attempts
        LEFT JOIN pool_upstream_accounts AS accounts
            ON accounts.id = attempts.upstream_account_id
        WHERE attempts.invoke_id = ?1
          AND attempts.occurred_at = ?2
        ORDER BY attempts.attempt_index ASC, attempts.id ASC
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

fn build_final_failure_timeline_entry(
    record: &ApiInvocation,
    body_row: Option<&InvocationResponseBodyRow>,
    raw_path_fallback_root: Option<&Path>,
) -> Option<InvocationWorkflowTimelineEntry> {
    if invocation_status_is_success_like(record) {
        return None;
    }

    let response_body = body_row.map(|row| {
        match resolve_response_body_text_from_row(row, raw_path_fallback_root) {
            Ok((text, _)) => InvocationWorkflowResponseBody {
                available: true,
                body_text: Some(text),
                unavailable_reason: None,
            },
            Err(reason) => InvocationWorkflowResponseBody {
                available: false,
                body_text: None,
                unavailable_reason: Some(reason),
            },
        }
    });

    let occurred_at = record
        .t_total_ms
        .and_then(|total| {
            parse_to_utc_datetime(&record.occurred_at).and_then(|started_at| {
                chrono::Duration::from_std(Duration::from_secs_f64(total.max(0.0) / 1000.0))
                    .ok()
                    .map(|delta| format_utc_iso(started_at + delta))
            })
        })
        .or_else(|| Some(record.occurred_at.clone()));

    Some(InvocationWorkflowTimelineEntry {
        block_id: "system-final-failure".to_string(),
        kind: "systemFinalFailure".to_string(),
        occurred_at,
        title: "Final downstream response".to_string(),
        subtitle: record
            .failure_kind
            .clone()
            .or_else(|| record.failure_class.clone()),
        status: record.status.clone(),
        attempt: None,
        detail: Some(json!({
            "invokeId": record.invoke_id.clone(),
            "downstreamStatusCode": record.downstream_status_code,
            "failureClass": record.failure_class.clone(),
            "failureKind": record.failure_kind.clone(),
            "errorMessage": record.error_message.clone(),
            "downstreamErrorMessage": record.downstream_error_message.clone(),
            "upstreamErrorCode": record.upstream_error_code.clone(),
            "upstreamErrorMessage": record.upstream_error_message.clone(),
            "upstreamRequestId": record.upstream_request_id.clone(),
            "streamTerminalEvent": record.stream_terminal_event.clone(),
            "responseContentEncoding": record.response_content_encoding.clone(),
        })),
        response_body,
    })
}

pub(crate) async fn fetch_invocation_workflow_detail(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<InvocationWorkflowDetailResponse>, ApiError> {
    let identity = load_invocation_workflow_identity(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::bad_request(anyhow!("record not found")))?;
    let record =
        load_persisted_api_invocation(&state.pool, &identity.invoke_id, &identity.occurred_at)
            .await
            .map_err(ApiError::from)?;
    let body_row = fetch_invocation_response_body_row_by_id(&state.pool, id).await?;
    let payload_value = body_row
        .as_ref()
        .and_then(|row| parse_optional_json_value(row.payload.as_deref()));
    let attempt_rows = query_invocation_workflow_attempt_rows(
        &state.pool,
        &identity.invoke_id,
        &identity.occurred_at,
    )
    .await?;
    let pseudo_attempt_rows = attempt_rows
        .iter()
        .filter(|attempt| invocation_workflow_attempt_row_is_pseudo_terminal(attempt))
        .collect::<Vec<_>>();
    let real_attempt_rows = attempt_rows
        .iter()
        .filter(|attempt| !invocation_workflow_attempt_row_is_pseudo_terminal(attempt))
        .collect::<Vec<_>>();
    let last_success_attempt_row_id = last_success_like_attempt_row_id(&real_attempt_rows);
    let pricing_catalog = state.pricing_catalog.read().await.clone();
    let usage_cost_audit = (invocation_status_is_success_like(&record)
        && invocation_has_usage_evidence(&record))
    .then(|| build_invocation_cost_audit(&record, &pricing_catalog, true))
    .flatten();
    let pool_route = normalized_runtime_text(record.route_mode.as_deref()) == "pool";
    let render_route_only = pool_route
        && !invocation_status_is_success_like(&record)
        && real_attempt_rows.is_empty()
        && (!pseudo_attempt_rows.is_empty() || record.pool_attempt_count.unwrap_or_default() == 0);
    let route_only_attempt = render_route_only.then(|| {
        pseudo_attempt_rows
            .last()
            .map(|attempt| {
                build_workflow_attempt_from_row(
                    &record,
                    attempt,
                    payload_value.as_ref(),
                    None,
                    false,
                )
            })
            .unwrap_or_else(|| {
                build_synthetic_workflow_attempt(&record, payload_value.as_ref(), None)
            })
    });
    let attempts = if real_attempt_rows.is_empty() {
        if route_only_attempt.is_some() {
            Vec::new()
        } else {
            vec![build_synthetic_workflow_attempt(
                &record,
                payload_value.as_ref(),
                usage_cost_audit.as_ref(),
            )]
        }
    } else {
        let final_attempt_row_id = final_real_attempt_row_id(&real_attempt_rows);
        real_attempt_rows
            .iter()
            .map(|attempt| {
                build_workflow_attempt_from_row(
                    &record,
                    attempt,
                    payload_value.as_ref(),
                    (last_success_attempt_row_id == Some(attempt.attempt_row_id))
                        .then_some(usage_cost_audit.as_ref())
                        .flatten(),
                    final_attempt_row_id == Some(attempt.attempt_row_id),
                )
            })
            .collect::<Vec<_>>()
    };
    let failure_entry = build_final_failure_timeline_entry(
        &record,
        body_row.as_ref(),
        state.config.database_path.parent(),
    );
    let partial = pool_route
        && record.pool_attempt_count.unwrap_or_default() > 0
        && real_attempt_rows.is_empty()
        && pseudo_attempt_rows.is_empty();
    let timeline_attempt_count = attempts.len();
    let response = InvocationWorkflowDetailResponse {
        hero: build_workflow_hero(&record, payload_value.as_ref(), timeline_attempt_count),
        timeline: build_workflow_timeline_entries(
            &record,
            &attempts,
            route_only_attempt.as_ref(),
            failure_entry,
        ),
        reconstructed: identity.timeline_json.is_none(),
        partial,
        partial_reason: partial.then(|| "attempt_rows_missing".to_string()),
    };
    Ok(Json(response))
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationResponseBodyRow {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) payload: Option<String>,
    pub(crate) raw_response: String,
    pub(crate) request_raw_path: Option<String>,
    pub(crate) request_raw_size: Option<i64>,
    pub(crate) request_raw_truncated: Option<i64>,
    pub(crate) request_raw_truncated_reason: Option<String>,
    pub(crate) response_raw_path: Option<String>,
    pub(crate) response_raw_size: Option<i64>,
    pub(crate) response_raw_truncated: Option<i64>,
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) detail_level: String,
    pub(crate) detail_prune_reason: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) upstream_request_id: Option<String>,
    pub(crate) attempt_public_id: Option<String>,
}

pub(crate) fn is_abnormal_invocation_failure(failure_class: Option<&str>) -> bool {
    matches!(
        failure_class
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some("service_failure" | "client_failure" | "client_abort")
    )
}

pub(crate) fn truncate_response_preview_text(value: &str) -> (String, bool) {
    let mut end = value.len();
    let mut count = 0usize;
    for (index, _) in value.char_indices() {
        if count == INVOCATION_RESPONSE_BODY_PREVIEW_CHAR_LIMIT {
            end = index;
            break;
        }
        count += 1;
    }
    if count < INVOCATION_RESPONSE_BODY_PREVIEW_CHAR_LIMIT {
        return (value.to_string(), false);
    }
    (value[..end].to_string(), true)
}

pub(crate) fn raw_response_fallback_reason(row: &InvocationResponseBodyRow) -> String {
    if row.response_raw_truncated_reason.as_deref() == Some("storage_suppressed") {
        "storage_suppressed".to_string()
    } else if row.attempt_public_id.is_some() && row.response_raw_path.is_none() {
        "attempt_response_body_not_captured".to_string()
    } else if row.detail_level == DETAIL_LEVEL_STRUCTURED_ONLY {
        "detail_pruned".to_string()
    } else if row.response_raw_truncated.unwrap_or_default() != 0 {
        row.response_raw_truncated_reason
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|reason| format!("preview_only:{reason}"))
            .unwrap_or_else(|| "preview_only".to_string())
    } else {
        "missing_body".to_string()
    }
}

pub(crate) fn raw_request_fallback_reason(row: &InvocationResponseBodyRow) -> String {
    if row.detail_level == DETAIL_LEVEL_STRUCTURED_ONLY {
        "detail_pruned".to_string()
    } else if row.request_raw_truncated_reason.as_deref() == Some("storage_suppressed") {
        "storage_suppressed".to_string()
    } else if row.request_raw_truncated.unwrap_or_default() != 0 {
        row.request_raw_truncated_reason
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "capture_unavailable".to_string())
    } else {
        "missing_body".to_string()
    }
}

pub(crate) fn resolve_request_body_text_from_row(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> Result<(String, bool), String> {
    let Some(path) = row.request_raw_path.as_deref() else {
        return Err(raw_request_fallback_reason(row));
    };

    match read_proxy_raw_bytes(path, raw_path_fallback_root) {
        Ok(bytes) => Ok((String::from_utf8_lossy(&bytes).to_string(), true)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err("raw_file_missing".to_string()),
        Err(err) => Err(format!("raw_file_unreadable:{err}")),
    }
}

pub(crate) fn resolve_response_body_text_from_row(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> Result<(String, bool), String> {
    if let Some(path) = row.response_raw_path.as_deref() {
        match read_proxy_raw_bytes(path, raw_path_fallback_root) {
            Ok(bytes) => {
                let (decoded, _) = decode_response_payload_for_usage(
                    &bytes,
                    row.response_content_encoding.as_deref(),
                );
                return Ok((String::from_utf8_lossy(decoded.as_ref()).to_string(), true));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let raw_preview = row.raw_response.trim();
                let preview_len = row.raw_response.len() as i64;
                if !raw_preview.is_empty()
                    && row.response_raw_size.unwrap_or(preview_len) <= preview_len
                {
                    return Ok((row.raw_response.clone(), false));
                }
                return Err("raw_file_missing".to_string());
            }
            Err(err) => {
                let raw_preview = row.raw_response.trim();
                let preview_len = row.raw_response.len() as i64;
                if !raw_preview.is_empty()
                    && row.response_raw_size.unwrap_or(preview_len) <= preview_len
                {
                    return Ok((row.raw_response.clone(), false));
                }
                return Err(format!("raw_file_unreadable:{err}"));
            }
        }
    }

    let raw_preview = row.raw_response.trim();
    if raw_preview.is_empty() {
        return Err(raw_response_fallback_reason(row));
    }

    let preview_len = row.raw_response.len() as i64;
    let fits_in_preview = row.response_raw_size.unwrap_or(preview_len) <= preview_len;
    if fits_in_preview && row.response_raw_truncated.unwrap_or_default() == 0 {
        return Ok((row.raw_response.clone(), false));
    }

    Err(raw_response_fallback_reason(row))
}

pub(crate) async fn fetch_invocation_response_body_row_by_id(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<InvocationResponseBodyRow>, ApiError> {
    let sql = format!(
        "SELECT \
         id, \
         invoke_id, \
         payload, \
         raw_response, \
         request_raw_path, \
         request_raw_size, \
         request_raw_truncated, \
         request_raw_truncated_reason, \
         response_raw_path, \
         response_raw_size, \
         response_raw_truncated, \
         response_raw_truncated_reason, \
         detail_level, \
         detail_prune_reason, \
         {response_content_encoding} AS response_content_encoding, \
         {resolved_failure} AS failure_class, \
         NULL AS upstream_request_id, \
         NULL AS attempt_public_id \
         FROM codex_invocations \
         WHERE id = ?1 \
         LIMIT 1",
        response_content_encoding = INVOCATION_RESPONSE_CONTENT_ENCODING_SQL,
        resolved_failure = INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
    );

    sqlx::query_as::<_, InvocationResponseBodyRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from)
}

fn build_request_body_capture_summary(
    row: &InvocationResponseBodyRow,
    capture_source: Option<&str>,
) -> Value {
    json!({
        "source": capture_source,
        "size": row.request_raw_size,
        "truncated": row.request_raw_truncated.unwrap_or_default() != 0,
        "truncatedReason": row.request_raw_truncated_reason.clone(),
        "detailLevel": Some(row.detail_level.clone()),
        "detailPruneReason": row.detail_prune_reason.clone(),
    })
}

fn build_response_body_capture_summary(
    row: &InvocationResponseBodyRow,
    capture_source: Option<&str>,
) -> Value {
    json!({
        "source": capture_source,
        "size": row.response_raw_size,
        "truncated": row.response_raw_truncated.unwrap_or_default() != 0,
        "truncatedReason": row.response_raw_truncated_reason.clone(),
        "detailLevel": Some(row.detail_level.clone()),
        "detailPruneReason": row.detail_prune_reason.clone(),
    })
}

fn build_request_body_routing_snapshot(
    row: &InvocationResponseBodyRow,
    payload: Option<&Value>,
) -> Value {
    json!({
        "routeMode": payload_string(payload, &["routeMode"]),
        "upstreamScope": payload_string(payload, &["upstreamScope"]),
        "stickyKey": payload_string(payload, &["stickyKey"]),
        "promptCacheKey": payload_string(payload, &["promptCacheKey"]),
        "proxyDisplayName": payload_string(payload, &["proxyDisplayName"]),
        "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
        "clientHeaderFingerprints": payload_clone(payload, &["clientHeaderFingerprints"]),
        "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
        "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
        "client": build_request_client_snapshot(payload),
        "invokeId": Some(row.invoke_id.clone()),
    })
}

fn build_response_body_header_snapshot(
    row: &InvocationResponseBodyRow,
    payload: Option<&Value>,
) -> Value {
    json!({
        "contentEncoding": row
            .response_content_encoding
            .clone()
            .or_else(|| payload_string(payload, &["responseContentEncoding"])),
        "contentEncodingChain": payload_string(payload, &["contentEncodingChain"]),
        "upstreamRequestId": row
            .upstream_request_id
            .clone()
            .or_else(|| payload_string(payload, &["upstreamRequestId"])),
        "cvmInvokeId": Some(row.invoke_id.clone()),
    })
}

fn build_request_body_response(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> InvocationResponseBodyResponse {
    let payload = parse_optional_json_value(row.payload.as_deref());
    let headers = Some(build_request_header_snapshot(payload.as_ref()));
    let routing = Some(build_request_body_routing_snapshot(row, payload.as_ref()));
    match resolve_request_body_text_from_row(row, raw_path_fallback_root) {
        Ok((body_text, from_full_body)) => InvocationResponseBodyResponse {
            available: true,
            body_text: Some(body_text),
            unavailable_reason: None,
            headers,
            routing,
            body_size: row.request_raw_size,
            body_truncated: Some(row.request_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.request_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: Some(
                if from_full_body {
                    "raw_file"
                } else {
                    "preview"
                }
                .to_string(),
            ),
        },
        Err(reason) => InvocationResponseBodyResponse {
            available: false,
            body_text: None,
            unavailable_reason: Some(reason),
            headers,
            routing,
            body_size: row.request_raw_size,
            body_truncated: Some(row.request_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.request_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: None,
        },
    }
}

fn build_response_body_response(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> InvocationResponseBodyResponse {
    build_response_body_response_with_source(row, raw_path_fallback_root, None)
}

pub(crate) fn build_response_body_response_with_source(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
    capture_source_override: Option<&str>,
) -> InvocationResponseBodyResponse {
    let payload = parse_optional_json_value(row.payload.as_deref());
    let headers = Some(build_response_body_header_snapshot(row, payload.as_ref()));
    let routing = Some(build_response_delivery_snapshot(payload.as_ref()));
    match resolve_response_body_text_from_row(row, raw_path_fallback_root) {
        Ok((body_text, from_full_body)) => InvocationResponseBodyResponse {
            available: true,
            body_text: Some(body_text),
            unavailable_reason: None,
            headers,
            routing,
            body_size: row.response_raw_size,
            body_truncated: Some(row.response_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.response_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: Some(
                if from_full_body {
                    capture_source_override.unwrap_or("raw_file")
                } else {
                    "preview"
                }
                .to_string(),
            ),
        },
        Err(reason) => InvocationResponseBodyResponse {
            available: false,
            body_text: None,
            unavailable_reason: Some(reason),
            headers,
            routing,
            body_size: row.response_raw_size,
            body_truncated: Some(row.response_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.response_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: None,
        },
    }
}

pub(crate) async fn fetch_invocation_attempt_response_body_row(
    pool: &Pool<Sqlite>,
    invocation_id: i64,
    attempt_public_id: &str,
) -> Result<Option<InvocationResponseBodyRow>, ApiError> {
    let sql = format!(
        r#"
        SELECT
            inv.id,
            inv.invoke_id,
            inv.payload,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.raw_response
                ELSE ''
            END AS raw_response,
            inv.request_raw_path,
            inv.request_raw_size,
            inv.request_raw_truncated,
            inv.request_raw_truncated_reason,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_raw_path IS NOT NULL THEN attempts.response_raw_path
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.response_raw_path
                ELSE NULL
            END AS response_raw_path,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_raw_path IS NOT NULL THEN attempts.response_raw_size
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.response_raw_size
                ELSE NULL
            END AS response_raw_size,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_raw_path IS NOT NULL THEN attempts.response_raw_truncated
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.response_raw_truncated
                ELSE NULL
            END AS response_raw_truncated,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_raw_path IS NOT NULL THEN attempts.response_raw_truncated_reason
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN inv.response_raw_truncated_reason
                ELSE NULL
            END AS response_raw_truncated_reason,
            inv.detail_level,
            inv.detail_prune_reason,
            CASE
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.response_content_encoding IS NOT NULL THEN attempts.response_content_encoding
                WHEN inv.detail_level != 'structured_only'
                    AND attempts.attempt_index = (
                    SELECT MAX(final_attempt.attempt_index)
                    FROM pool_upstream_request_attempts AS final_attempt
                    WHERE final_attempt.invoke_id = attempts.invoke_id
                      AND final_attempt.occurred_at = attempts.occurred_at
                      AND final_attempt.status != 'budget_exhausted_final'
                ) THEN {response_content_encoding}
                ELSE NULL
            END AS response_content_encoding,
            {resolved_failure} AS failure_class,
            attempts.upstream_request_id AS upstream_request_id,
            attempts.attempt_public_id AS attempt_public_id
        FROM codex_invocations AS inv
        JOIN pool_upstream_request_attempts AS attempts
            ON attempts.invoke_id = inv.invoke_id
           AND attempts.occurred_at = inv.occurred_at
        WHERE inv.id = ?1
          AND attempts.attempt_public_id = ?2
        LIMIT 1
        "#,
        response_content_encoding = INVOCATION_RESPONSE_CONTENT_ENCODING_SQL,
        resolved_failure = invocation_resolved_failure_class_sql_for("inv"),
    );
    sqlx::query_as::<_, InvocationResponseBodyRow>(&sql)
        .bind(invocation_id)
        .bind(attempt_public_id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from)
}

pub(crate) async fn fetch_invocation_record_detail(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<InvocationRecordDetailResponse>, ApiError> {
    let row = fetch_invocation_response_body_row_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::bad_request(anyhow!("record not found")))?;

    let abnormal_response_body = if is_abnormal_invocation_failure(row.failure_class.as_deref()) {
        match resolve_response_body_text_from_row(&row, state.config.database_path.parent()) {
            Ok((text, from_full_body)) => {
                let (preview_text, truncated) = truncate_response_preview_text(&text);
                Some(InvocationAbnormalResponseBodyPreview {
                    available: true,
                    preview_text: Some(preview_text),
                    has_more: truncated || from_full_body,
                    unavailable_reason: None,
                })
            }
            Err(reason) => Some(InvocationAbnormalResponseBodyPreview {
                available: false,
                preview_text: None,
                has_more: false,
                unavailable_reason: Some(reason),
            }),
        }
    } else {
        None
    };

    Ok(Json(InvocationRecordDetailResponse {
        id: row.id,
        abnormal_response_body,
    }))
}

pub(crate) async fn fetch_invocation_response_body(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<InvocationResponseBodyResponse>, ApiError> {
    let row = fetch_invocation_response_body_row_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::bad_request(anyhow!("record not found")))?;
    Ok(Json(build_response_body_response(
        &row,
        state.config.database_path.parent(),
    )))
}

pub(crate) async fn fetch_invocation_attempt_response_body(
    State(state): State<Arc<AppState>>,
    axum::extract::Path((id, attempt_public_id)): axum::extract::Path<(i64, String)>,
) -> Result<Json<InvocationResponseBodyResponse>, ApiError> {
    let row =
        fetch_invocation_attempt_response_body_row(&state.pool, id, attempt_public_id.as_str())
            .await?
            .ok_or_else(|| ApiError::bad_request(anyhow!("attempt not found")))?;
    let (has_attempt_capture, has_invocation_capture) = sqlx::query_as::<
        _,
        (Option<i64>, Option<i64>),
    >(
        "SELECT (inv.detail_level != 'structured_only' AND attempts.response_raw_path IS NOT NULL),
                (inv.detail_level != 'structured_only' AND inv.response_raw_path IS NOT NULL)
         FROM pool_upstream_request_attempts AS attempts
         JOIN codex_invocations AS inv
           ON inv.invoke_id = attempts.invoke_id
          AND inv.occurred_at = attempts.occurred_at
         WHERE inv.id = ?1
           AND attempts.attempt_public_id = ?2
         LIMIT 1",
    )
    .bind(id)
    .bind(attempt_public_id.as_str())
    .fetch_optional(&state.pool)
    .await?
    .unwrap_or_default();
    let has_attempt_capture = has_attempt_capture.unwrap_or_default() != 0;
    let has_invocation_capture = has_invocation_capture.unwrap_or_default() != 0;
    let source = Some(if has_attempt_capture {
        "attempt_raw_file"
    } else if has_invocation_capture && row.response_raw_path.is_some() {
        "invocation_raw_file"
    } else {
        "attempt_metrics"
    });
    Ok(Json(build_response_body_response_with_source(
        &row,
        state.config.database_path.parent(),
        source,
    )))
}

pub(crate) async fn fetch_invocation_request_body(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<InvocationResponseBodyResponse>, ApiError> {
    let row = fetch_invocation_response_body_row_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::bad_request(anyhow!("record not found")))?;
    Ok(Json(build_request_body_response(
        &row,
        state.config.database_path.parent(),
    )))
}
