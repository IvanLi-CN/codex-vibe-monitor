pub(crate) const HEADER_STICKY_EARLY_STICKY_SCAN_BYTES: usize = 64 * 1024;

pub(crate) fn best_effort_extract_sticky_key_from_request_body_prefix(bytes: &[u8]) -> Option<String> {
    const STICKY_KEY_PATTERNS: &[&[u8]] = &[
        br#""sticky_key""#,
        br#""stickyKey""#,
        br#""prompt_cache_key""#,
        br#""promptCacheKey""#,
    ];

    fn find_subslice(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
        haystack[start..]
            .windows(needle.len())
            .position(|window| window == needle)
            .map(|offset| start + offset)
    }

    fn skip_ascii_whitespace(bytes: &[u8], mut index: usize) -> usize {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        index
    }

    fn parse_json_string(bytes: &[u8], index: usize) -> Option<(String, usize)> {
        if bytes.get(index) != Some(&b'"') {
            return None;
        }
        let mut parsed = String::new();
        let mut cursor = index + 1;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'"' => return Some((parsed, cursor + 1)),
                b'\\' => {
                    cursor += 1;
                    let escaped = *bytes.get(cursor)?;
                    match escaped {
                        b'"' | b'\\' | b'/' => parsed.push(escaped as char),
                        b'b' => parsed.push('\u{0008}'),
                        b'f' => parsed.push('\u{000C}'),
                        b'n' => parsed.push('\n'),
                        b'r' => parsed.push('\r'),
                        b't' => parsed.push('\t'),
                        // Stay conservative for unicode escapes; missing the key is better
                        // than returning a false early decision.
                        b'u' => return None,
                        _ => return None,
                    }
                }
                byte if byte.is_ascii_control() => return None,
                byte if byte.is_ascii() => parsed.push(byte as char),
                _ => return None,
            }
            cursor += 1;
        }
        None
    }

    for pattern in STICKY_KEY_PATTERNS {
        let mut cursor = 0usize;
        while let Some(key_start) = find_subslice(bytes, pattern, cursor) {
            let mut value_start = skip_ascii_whitespace(bytes, key_start + pattern.len());
            if bytes.get(value_start) != Some(&b':') {
                cursor = key_start + pattern.len();
                continue;
            }
            value_start = skip_ascii_whitespace(bytes, value_start + 1);
            let Some((value, next_index)) = parse_json_string(bytes, value_start) else {
                cursor = key_start + pattern.len();
                continue;
            };
            let normalized = value.trim();
            if !normalized.is_empty() {
                return Some(normalized.to_string());
            }
            cursor = next_index;
        }
    }
    None
}

pub(crate) fn prepare_target_request_body(
    target: ProxyCaptureTarget,
    body: Vec<u8>,
    auto_include_usage: bool,
) -> (Vec<u8>, RequestCaptureInfo, bool) {
    let mut info = RequestCaptureInfo {
        model: None,
        sticky_key: None,
        prompt_cache_key: None,
        requested_service_tier: None,
        reasoning_effort: None,
        is_stream: false,
        parse_error: None,
    };

    if body.is_empty() {
        return (body, info, false);
    }

    let mut value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => {
            info.parse_error = Some(format!("request_json_parse_error:{err}"));
            return (body, info, false);
        }
    };

    info.model = value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    info.sticky_key = extract_sticky_key_from_request_body(&value);
    info.prompt_cache_key = extract_prompt_cache_key_from_request_body(&value);
    info.reasoning_effort = extract_reasoning_effort_from_request_body(target, &value);
    info.is_stream = value
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut rewritten = false;
    if target.should_auto_include_usage()
        && info.is_stream
        && auto_include_usage
        && let Some(object) = value.as_object_mut()
    {
        let stream_options = object
            .entry("stream_options".to_string())
            .or_insert_with(|| json!({}));
        if let Some(stream_options_obj) = stream_options.as_object_mut() {
            stream_options_obj.insert("include_usage".to_string(), Value::Bool(true));
            rewritten = true;
        } else {
            object.insert(
                "stream_options".to_string(),
                json!({ "include_usage": true }),
            );
            rewritten = true;
        }
    }

    info.requested_service_tier = extract_requested_service_tier_from_request_body(&value);

    if rewritten {
        match serde_json::to_vec(&value) {
            Ok(rewritten_body) => (rewritten_body, info, true),
            Err(err) => {
                let mut fallback = info;
                fallback.parse_error = Some(format!("request_json_rewrite_error:{err}"));
                (body, fallback, false)
            }
        }
    } else {
        (body, info, false)
    }
}

pub(crate) fn proxy_upstream_send_timeout_for_capture_target(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    capture_target: Option<ProxyCaptureTarget>,
) -> Duration {
    match capture_target {
        Some(ProxyCaptureTarget::Responses) => timeouts.responses_first_byte_timeout,
        Some(ProxyCaptureTarget::ResponsesCompact) => timeouts.compact_first_byte_timeout,
        _ => timeouts.default_send_timeout,
    }
}

pub(crate) fn pool_upstream_first_chunk_timeout(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    original_uri: &Uri,
    method: &Method,
) -> Duration {
    match capture_target_for_request(original_uri.path(), method) {
        Some(ProxyCaptureTarget::Responses) => timeouts.responses_first_byte_timeout,
        Some(ProxyCaptureTarget::ResponsesCompact) => timeouts.compact_first_byte_timeout,
        _ => timeouts.default_first_byte_timeout,
    }
}

pub(crate) fn pool_upstream_responses_total_timeout(
    config: &AppConfig,
    original_uri: &Uri,
    method: &Method,
) -> Option<Duration> {
    pool_uses_responses_timeout_failover_policy(original_uri, method)
        .then_some(config.pool_upstream_responses_total_timeout)
}

pub(crate) fn proxy_capture_target_stream_timeout(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    capture_target: ProxyCaptureTarget,
) -> Option<Duration> {
    match capture_target {
        ProxyCaptureTarget::Responses => Some(timeouts.responses_stream_timeout),
        ProxyCaptureTarget::ResponsesCompact => Some(timeouts.compact_stream_timeout),
        ProxyCaptureTarget::ChatCompletions => None,
    }
}

pub(crate) fn pool_upstream_send_timeout(
    original_uri: &Uri,
    method: &Method,
    send_timeout: Duration,
    pre_first_byte_timeout: Duration,
) -> Duration {
    if pool_uses_responses_timeout_failover_policy(original_uri, method) {
        pre_first_byte_timeout
    } else {
        send_timeout.min(pre_first_byte_timeout)
    }
}

pub(crate) fn pool_uses_responses_timeout_failover_policy(original_uri: &Uri, method: &Method) -> bool {
    method == Method::POST
        && matches!(
            original_uri.path(),
            "/v1/responses" | "/v1/responses/compact"
        )
}

pub(crate) fn pool_timeout_budget_with_total_limit(
    timeout: Duration,
    total_timeout: Option<Duration>,
    total_timeout_started_at: Option<Instant>,
) -> Option<Duration> {
    match (total_timeout, total_timeout_started_at) {
        (Some(total_timeout), Some(started_at)) => {
            remaining_timeout_budget(total_timeout, started_at.elapsed())
                .map(|remaining| remaining.min(timeout))
        }
        // A running total-timeout only exists once we have an anchor instant.
        // Pre-attempt no-account waiting uses an explicit deadline earlier in the flow;
        // real upstream attempts without an anchor should keep their own per-phase budgets.
        (Some(_), None) => Some(timeout),
        (None, _) => Some(timeout),
    }
}

pub(crate) fn pool_total_timeout_exhausted(total_timeout: Duration, started_at: Instant) -> bool {
    timeout_budget_exhausted(total_timeout, started_at.elapsed())
}

pub(crate) fn pool_total_timeout_exhausted_message(total_timeout: Duration) -> String {
    format!(
        "pool upstream total timeout exhausted after {}ms",
        total_timeout.as_millis()
    )
}

pub(crate) fn build_pool_total_timeout_exhausted_error(
    total_timeout: Duration,
    last_error: Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    let mut final_error = last_error.unwrap_or(PoolUpstreamError {
        account: None,
        status: StatusCode::GATEWAY_TIMEOUT,
        message: pool_total_timeout_exhausted_message(total_timeout),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: PoolAttemptSummary::default(),
        requested_service_tier: None,
        request_body_for_capture: None,
    });
    final_error.status = StatusCode::GATEWAY_TIMEOUT;
    final_error.message = pool_total_timeout_exhausted_message(total_timeout);
    final_error.failure_kind = PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED;
    final_error.upstream_error_code = None;
    final_error.upstream_error_message = None;
    final_error.upstream_request_id = None;
    final_error.attempt_summary = pool_attempt_summary(
        attempt_count,
        distinct_account_count,
        Some(PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED.to_string()),
    );
    final_error
}

pub(crate) fn pool_pre_attempt_total_timeout_error(total_timeout: Duration) -> (StatusCode, String) {
    (
        StatusCode::GATEWAY_TIMEOUT,
        pool_total_timeout_exhausted_message(total_timeout),
    )
}

pub(crate) fn pool_uses_responses_family_retry_budget_policy(original_uri: &Uri, method: &Method) -> bool {
    method == Method::POST
        && matches!(
            original_uri.path(),
            "/v1/responses" | "/v1/responses/compact"
        )
}

pub(crate) fn pool_same_account_attempt_budget(
    original_uri: &Uri,
    method: &Method,
    distinct_account_count: usize,
    initial_same_account_attempts: u8,
) -> u8 {
    if pool_uses_responses_family_retry_budget_policy(original_uri, method) {
        if distinct_account_count <= 1 {
            initial_same_account_attempts.max(1)
        } else {
            1
        }
    } else if distinct_account_count <= 1 {
        initial_same_account_attempts.max(1)
    } else {
        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS
    }
}

pub(crate) const POOL_RESPONSES_FAMILY_INITIAL_OVERLOAD_ATTEMPT_BUDGET: u8 = 4;

pub(crate) fn pool_overload_same_account_attempt_budget(
    original_uri: &Uri,
    method: &Method,
    distinct_account_count: usize,
    same_account_attempt_budget: u8,
) -> u8 {
    if pool_uses_responses_family_retry_budget_policy(original_uri, method)
        && distinct_account_count <= 1
    {
        same_account_attempt_budget.max(POOL_RESPONSES_FAMILY_INITIAL_OVERLOAD_ATTEMPT_BUDGET)
    } else {
        same_account_attempt_budget
    }
}

pub(crate) fn pool_error_message_indicates_proxy_timeout(message: &str) -> bool {
    let message_lower = message.trim().to_ascii_lowercase();
    message_lower.contains("request timed out after")
        || message_lower.contains("upstream handshake timed out after")
}

pub(crate) fn pool_failure_is_timeout_shaped(failure_kind: &str, message: &str) -> bool {
    matches!(
        failure_kind,
        PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
            | PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
            | PROXY_FAILURE_UPSTREAM_STREAM_ERROR
            | PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
    ) && pool_error_message_indicates_proxy_timeout(message)
}

pub(crate) fn pool_account_forward_proxy_scope(
    account: &PoolResolvedAccount,
) -> std::result::Result<ForwardProxyRouteScope, String> {
    Ok(account.forward_proxy_scope.clone())
}

pub(crate) async fn select_pool_account_forward_proxy_client(
    state: &AppState,
    account: &PoolResolvedAccount,
) -> Result<(ForwardProxyRouteScope, SelectedForwardProxy, Client), String> {
    let scope = pool_account_forward_proxy_scope(account)?;
    let selected_proxy = select_forward_proxy_for_scope(state, &scope)
        .await
        .map_err(|err| match &scope {
            ForwardProxyRouteScope::BoundGroup { group_name, .. }
                if err
                    .to_string()
                    .contains("bound forward proxy group has no selectable nodes") =>
            {
                format!(
                    "upstream account group \"{group_name}\" has no selectable bound forward proxy nodes"
                )
            }
            _ => format!("failed to select forward proxy node: {err}"),
        })?;
    let client = match state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
    {
        Ok(client) => client,
        Err(err) => {
            record_forward_proxy_scope_result(
                state,
                &scope,
                &selected_proxy.key,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
            return Err(format!("failed to initialize forward proxy client: {err}"));
        }
    };
    Ok((scope, selected_proxy, client))
}

pub(crate) async fn record_pool_account_forward_proxy_result(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    selected_proxy: &SelectedForwardProxy,
    result: ForwardProxyRouteResultKind,
) {
    record_forward_proxy_scope_result(state, scope, &selected_proxy.key, result).await;
}

pub(crate) fn extract_sticky_key_from_request_body(value: &Value) -> Option<String> {
    const STICKY_KEY_POINTERS: &[&str] = &[
        "/metadata/sticky_key",
        "/metadata/stickyKey",
        "/metadata/prompt_cache_key",
        "/metadata/promptCacheKey",
        "/sticky_key",
        "/stickyKey",
        "/prompt_cache_key",
        "/promptCacheKey",
    ];

    for pointer in STICKY_KEY_POINTERS {
        if let Some(sticky_key) = value.pointer(pointer).and_then(|v| v.as_str()) {
            let normalized = sticky_key.trim();
            if !normalized.is_empty() {
                return Some(normalized.to_string());
            }
        }
    }
    None
}

pub(crate) fn extract_prompt_cache_key_from_request_body(value: &Value) -> Option<String> {
    for pointer in [
        "/metadata/prompt_cache_key",
        "/metadata/promptCacheKey",
        "/prompt_cache_key",
        "/promptCacheKey",
    ] {
        if let Some(prompt_cache_key) = value.pointer(pointer).and_then(|v| v.as_str()) {
            let normalized = prompt_cache_key.trim();
            if !normalized.is_empty() {
                return Some(normalized.to_string());
            }
        }
    }
    None
}

pub(crate) fn extract_requested_service_tier_from_request_body(value: &Value) -> Option<String> {
    ["/service_tier", "/serviceTier"]
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(|entry| entry.as_str()))
        .and_then(normalize_service_tier)
}

pub(crate) fn rewrite_request_service_tier_for_fast_mode(
    value: &mut Value,
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };

    match fast_mode_rewrite_mode {
        TagFastModeRewriteMode::KeepOriginal => false,
        TagFastModeRewriteMode::ForceRemove => {
            let removed_snake = object.remove("service_tier").is_some();
            let removed_camel = object.remove("serviceTier").is_some();
            removed_snake || removed_camel
        }
        TagFastModeRewriteMode::FillMissing => {
            let has_existing_service_tier =
                object.contains_key("service_tier") || object.contains_key("serviceTier");
            if has_existing_service_tier {
                false
            } else {
                object.insert(
                    "service_tier".to_string(),
                    Value::String("priority".to_string()),
                );
                true
            }
        }
        TagFastModeRewriteMode::ForceAdd => {
            let mut rewritten = object.remove("serviceTier").is_some();
            if object.get("service_tier").and_then(|entry| entry.as_str()) != Some("priority") {
                object.insert(
                    "service_tier".to_string(),
                    Value::String("priority".to_string()),
                );
                rewritten = true;
            }
            rewritten
        }
    }
}

pub(crate) fn extract_reasoning_effort_from_request_body(
    target: ProxyCaptureTarget,
    value: &Value,
) -> Option<String> {
    let raw = match target {
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact => {
            value.pointer("/reasoning/effort").and_then(|v| v.as_str())
        }
        ProxyCaptureTarget::ChatCompletions => {
            value.get("reasoning_effort").and_then(|v| v.as_str())
        }
    }?;

    let normalized = raw.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

pub(crate) fn build_response_capture_info_from_bytes(
    bytes: &[u8],
    request_is_stream: bool,
    decode_failure_reason: Option<String>,
) -> ResponseCaptureInfo {
    if bytes.is_empty() {
        return ResponseCaptureInfo {
            model: None,
            usage: ParsedUsage::default(),
            usage_missing_reason: Some("empty_response".to_string()),
            service_tier: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
        };
    }

    let looks_like_stream = request_is_stream || response_payload_looks_like_sse(bytes);
    let mut response_info = if looks_like_stream {
        parse_stream_response_payload(bytes)
    } else {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(value) => {
                let model = extract_model_from_payload(&value);
                let usage = extract_usage_from_payload(&value).unwrap_or_default();
                let service_tier = extract_service_tier_from_payload(&value);
                let usage_missing_reason = if usage.total_tokens.is_none()
                    && usage.input_tokens.is_none()
                    && usage.output_tokens.is_none()
                {
                    Some("usage_missing_in_response".to_string())
                } else {
                    None
                };
                ResponseCaptureInfo {
                    model,
                    usage,
                    usage_missing_reason,
                    service_tier,
                    stream_terminal_event: None,
                    upstream_error_code: extract_upstream_error_code(&value),
                    upstream_error_message: extract_upstream_error_message(&value),
                    upstream_request_id: extract_upstream_request_id(&value),
                }
            }
            Err(_) => {
                let model = extract_partial_json_model(bytes);
                let service_tier = extract_partial_json_service_tier(bytes);
                let upstream_error_code = extract_partial_json_string_field(bytes, &["code"]);
                let upstream_error_message = extract_partial_json_string_field(bytes, &["message"]);
                let upstream_request_id =
                    extract_partial_json_string_field(bytes, &["request_id", "requestId"]).or_else(
                        || {
                            upstream_error_message
                                .as_deref()
                                .and_then(extract_request_id_from_message)
                        },
                    );
                ResponseCaptureInfo {
                    model,
                    usage: ParsedUsage::default(),
                    usage_missing_reason: Some("response_not_json".to_string()),
                    service_tier,
                    stream_terminal_event: None,
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                }
            }
        }
    };

    if let Some(reason) = decode_failure_reason {
        let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
            format!("response_decode_failed:{reason};{existing}")
        } else {
            format!("response_decode_failed:{reason}")
        };
        response_info.usage_missing_reason = Some(combined_reason);
    }

    response_info
}

pub(crate) fn parse_target_response_payload(
    _target: ProxyCaptureTarget,
    bytes: &[u8],
    request_is_stream: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_parse(bytes, content_encoding);
    build_response_capture_info_from_bytes(
        decoded_bytes.as_ref(),
        request_is_stream,
        decode_failure_reason,
    )
}
pub(crate) fn parse_target_response_preview_payload(
    _target: ProxyCaptureTarget,
    bytes: &[u8],
    request_is_stream: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_preview_parse(bytes, content_encoding);
    build_response_capture_info_from_bytes(
        decoded_bytes.as_ref(),
        request_is_stream,
        decode_failure_reason,
    )
}

pub(crate) fn response_payload_looks_like_sse(bytes: &[u8]) -> bool {
    String::from_utf8_lossy(bytes)
        .lines()
        .find_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.is_empty() {
                None
            } else {
                Some(
                    trimmed.starts_with("data:")
                        || trimmed.starts_with("event:")
                        || trimmed.starts_with("id:")
                        || trimmed.starts_with("retry:"),
                )
            }
        })
        .unwrap_or(false)
}

pub(crate) fn response_payload_looks_like_sse_after_decode(
    bytes: &[u8],
    content_encoding: Option<&str>,
) -> bool {
    let (decoded, _) = decode_response_payload_for_preview_parse(bytes, content_encoding);
    response_payload_looks_like_sse(decoded.as_ref())
}

#[cfg(test)]
pub(crate) static RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn reset_response_capture_raw_fallback_counters() {
    RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS.store(0, Ordering::Relaxed);
    RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.store(0, Ordering::Relaxed);
}

#[cfg(test)]
pub(crate) fn response_capture_raw_fallback_counts() -> (usize, usize) {
    (
        RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS.load(Ordering::Relaxed),
        RESPONSE_CAPTURE_RAW_PARSE_FALLBACK_CALLS.load(Ordering::Relaxed),
    )
}

#[allow(dead_code)]
pub(crate) fn response_payload_looks_like_sse_from_raw_file(
    path: &Path,
    content_encoding: Option<&str>,
) -> std::result::Result<bool, String> {
    let mut reader = open_decoded_response_reader(path, content_encoding)?;
    let mut decoded_prefix = Vec::new();
    reader
        .by_ref()
        .take((RAW_RESPONSE_PREVIEW_LIMIT + 1) as u64)
        .read_to_end(&mut decoded_prefix)
        .map_err(|err| err.to_string())?;
    Ok(response_payload_looks_like_sse(&decoded_prefix))
}

#[allow(dead_code)]
pub(crate) fn response_payload_looks_like_sse_from_capture(
    resp_raw: &RawPayloadMeta,
    preview_bytes: &[u8],
    content_encoding: Option<&str>,
) -> bool {
    #[cfg(test)]
    RESPONSE_CAPTURE_RAW_SSE_HINT_FALLBACK_CALLS.fetch_add(1, Ordering::Relaxed);

    if response_payload_looks_like_sse_after_decode(preview_bytes, content_encoding) {
        return true;
    }

    if preview_bytes.len() < RAW_RESPONSE_PREVIEW_LIMIT && content_encoding.is_none() {
        return false;
    }

    let Some(path) = resp_raw.path.as_deref() else {
        return false;
    };

    response_payload_looks_like_sse_from_raw_file(&PathBuf::from(path), content_encoding)
        .unwrap_or(false)
}

pub(crate) fn decode_response_payload_for_parse<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    decode_response_payload(bytes, content_encoding, false)
}

pub(crate) fn decode_response_payload_for_preview_parse<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        return (Cow::Borrowed(bytes), None);
    }

    let mut decoded = bytes.to_vec();
    for encoding in encodings.iter().rev() {
        match decode_single_content_encoding_lossy(decoded.as_slice(), encoding) {
            Ok((next, None)) => decoded = next,
            Ok((next, Some(err))) => return (Cow::Owned(next), Some(format!("{encoding}:{err}"))),
            Err(err) => return (Cow::Borrowed(bytes), Some(format!("{encoding}:{err}"))),
        }
    }

    (Cow::Owned(decoded), None)
}

pub(crate) fn read_decoder_lossy(
    mut reader: impl Read,
) -> std::result::Result<(Vec<u8>, Option<String>), String> {
    let mut decoded = Vec::new();
    match reader.read_to_end(&mut decoded) {
        Ok(_) => Ok((decoded, None)),
        Err(err) if !decoded.is_empty() => Ok((decoded, Some(err.to_string()))),
        Err(err) => Err(err.to_string()),
    }
}

pub(crate) fn decode_single_content_encoding_lossy(
    bytes: &[u8],
    encoding: &str,
) -> std::result::Result<(Vec<u8>, Option<String>), String> {
    match encoding {
        "identity" => Ok((bytes.to_vec(), None)),
        "gzip" | "x-gzip" => read_decoder_lossy(GzDecoder::new(bytes)),
        "br" => read_decoder_lossy(BrotliDecompressor::new(bytes, 4096)),
        "deflate" => {
            let mut zlib_decoder = ZlibDecoder::new(bytes);
            let mut decoded = Vec::new();
            match zlib_decoder.read_to_end(&mut decoded) {
                Ok(_) => Ok((decoded, None)),
                Err(zlib_err) if !decoded.is_empty() => Ok((decoded, Some(zlib_err.to_string()))),
                Err(zlib_err) => {
                    let mut raw_decoder = DeflateDecoder::new(bytes);
                    let mut raw_decoded = Vec::new();
                    match raw_decoder.read_to_end(&mut raw_decoded) {
                        Ok(_) => Ok((raw_decoded, None)),
                        Err(raw_err) if !raw_decoded.is_empty() => {
                            Ok((raw_decoded, Some(raw_err.to_string())))
                        }
                        Err(raw_err) => Err(format!("zlib={zlib_err}; raw={raw_err}")),
                    }
                }
            }
        }
        other => Err(format!("unsupported_content_encoding:{other}")),
    }
}

#[derive(Default)]
pub(crate) struct StreamResponsePayloadParser {
    model: Option<String>,
    usage: ParsedUsage,
    service_tier: Option<String>,
    service_tier_rank: u8,
    stream_terminal_event: Option<String>,
    upstream_error_code: Option<String>,
    upstream_error_message: Option<String>,
    upstream_request_id: Option<String>,
    usage_found: bool,
    parse_error_seen: bool,
    pending_event_name: Option<String>,
    saw_stream_fields: bool,
}

impl StreamResponsePayloadParser {
    fn ingest_line(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.starts_with("event:") {
            self.saw_stream_fields = true;
            self.pending_event_name = Some(trimmed.trim_start_matches("event:").trim().to_string());
            return;
        }
        if !trimmed.starts_with("data:") {
            return;
        }
        self.saw_stream_fields = true;
        let payload = trimmed.trim_start_matches("data:").trim();
        if payload.is_empty() || payload == "[DONE]" {
            self.pending_event_name = None;
            return;
        }
        match serde_json::from_str::<Value>(payload) {
            Ok(value) => {
                let event_name = self.pending_event_name.take();
                if self.model.is_none() {
                    self.model = extract_model_from_payload(&value);
                }
                if let Some(service_tier) = extract_service_tier_from_payload(&value) {
                    let rank = stream_payload_service_tier_rank(event_name.as_deref(), &value);
                    if should_overwrite_stream_service_tier(
                        self.service_tier.as_deref(),
                        self.service_tier_rank,
                        &service_tier,
                        rank,
                    ) {
                        self.service_tier = Some(service_tier);
                        self.service_tier_rank = rank;
                    }
                }
                if let Some(parsed_usage) = extract_usage_from_payload(&value) {
                    self.usage = parsed_usage;
                    self.usage_found = true;
                }
                if stream_payload_indicates_failure(event_name.as_deref(), &value) {
                    let candidate = event_name
                        .clone()
                        .or_else(|| extract_stream_payload_type(&value))
                        .unwrap_or_else(|| "response.failed".to_string());
                    if self.stream_terminal_event.is_none() || candidate == "response.failed" {
                        self.stream_terminal_event = Some(candidate);
                    }
                }
                if self.upstream_error_code.is_none() {
                    self.upstream_error_code = extract_upstream_error_code(&value);
                }
                if self.upstream_error_message.is_none() {
                    self.upstream_error_message = extract_upstream_error_message(&value);
                }
                if self.upstream_request_id.is_none() {
                    self.upstream_request_id = extract_upstream_request_id(&value);
                }
            }
            Err(_) => {
                self.pending_event_name = None;
                self.parse_error_seen = true;
            }
        }
    }

    fn finish(self) -> ResponseCaptureInfo {
        let usage_missing_reason = if self.usage_found {
            None
        } else if self.stream_terminal_event.is_some() {
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string())
        } else if self.parse_error_seen {
            Some("stream_event_parse_error".to_string())
        } else {
            Some("usage_missing_in_stream".to_string())
        };

        ResponseCaptureInfo {
            model: self.model,
            usage: self.usage,
            usage_missing_reason,
            service_tier: self.service_tier,
            stream_terminal_event: self.stream_terminal_event,
            upstream_error_code: self.upstream_error_code,
            upstream_error_message: self.upstream_error_message,
            upstream_request_id: self.upstream_request_id,
        }
    }
}

pub(crate) struct StreamResponsePayloadParseOutcome {
    response_info: ResponseCaptureInfo,
    saw_stream_fields: bool,
}

pub(crate) struct StreamResponsePayloadChunkParser {
    parser: StreamResponsePayloadParser,
    line_buffer: Vec<u8>,
    discarding_oversized_line: bool,
    line_buffer_limit: usize,
    discarded_oversized_line: bool,
}

impl Default for StreamResponsePayloadChunkParser {
    fn default() -> Self {
        Self::with_line_buffer_limit(STREAM_RESPONSE_LINE_BUFFER_LIMIT)
    }
}

impl StreamResponsePayloadChunkParser {
    fn with_line_buffer_limit(line_buffer_limit: usize) -> Self {
        Self {
            parser: StreamResponsePayloadParser::default(),
            line_buffer: Vec::new(),
            discarding_oversized_line: false,
            line_buffer_limit,
            discarded_oversized_line: false,
        }
    }

    fn line_bytes_look_like_stream_field(line: &[u8]) -> bool {
        let decoded = String::from_utf8_lossy(line);
        let trimmed = decoded.trim_start();
        trimmed.starts_with("data:")
            || trimmed.starts_with("event:")
            || trimmed.starts_with("id:")
            || trimmed.starts_with("retry:")
    }

    fn flush_line(&mut self) {
        if self.line_buffer.is_empty() {
            return;
        }
        let decoded = String::from_utf8_lossy(&self.line_buffer);
        self.parser.ingest_line(decoded.as_ref());
        self.line_buffer.clear();
    }

    fn start_discarding_oversized_line(&mut self) {
        if Self::line_bytes_look_like_stream_field(&self.line_buffer) {
            self.parser.saw_stream_fields = true;
        }
        self.parser.parse_error_seen = true;
        self.discarded_oversized_line = true;
        self.line_buffer.clear();
        self.discarding_oversized_line = true;
    }

    fn append_segment(&mut self, segment: &[u8], ends_line: bool) {
        if self.discarding_oversized_line {
            if ends_line {
                self.discarding_oversized_line = false;
            }
            return;
        }

        if self.line_buffer.len().saturating_add(segment.len()) > self.line_buffer_limit {
            self.start_discarding_oversized_line();
            if ends_line {
                self.discarding_oversized_line = false;
            }
            return;
        }

        self.line_buffer.extend_from_slice(segment);
        if ends_line {
            self.flush_line();
        }
    }

    fn ingest_bytes(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        let mut line_start = 0usize;
        for (idx, byte) in bytes.iter().enumerate() {
            if *byte == b'\n' {
                self.append_segment(&bytes[line_start..=idx], true);
                line_start = idx + 1;
            }
        }
        if line_start < bytes.len() {
            self.append_segment(&bytes[line_start..], false);
        }
    }

    fn finish(mut self) -> StreamResponsePayloadParseOutcome {
        if self.discarding_oversized_line {
            self.parser.parse_error_seen = true;
        } else {
            self.flush_line();
        }
        StreamResponsePayloadParseOutcome {
            saw_stream_fields: self.parser.saw_stream_fields,
            response_info: self.parser.finish(),
        }
    }
}

pub(crate) fn parse_stream_response_payload(bytes: &[u8]) -> ResponseCaptureInfo {
    let mut parser = StreamResponsePayloadChunkParser::default();
    parser.ingest_bytes(bytes);
    parser.finish().response_info
}

#[allow(dead_code)]
pub(crate) fn parse_stream_response_payload_from_reader<R: Read>(
    reader: R,
) -> io::Result<ResponseCaptureInfo> {
    let mut parser = StreamResponsePayloadChunkParser::with_line_buffer_limit(
        RAW_FILE_STREAM_RESPONSE_LINE_BUFFER_LIMIT,
    );
    let mut reader = io::BufReader::new(reader);
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        parser.ingest_bytes(&chunk[..read]);
    }
    Ok(parser.finish().response_info)
}

pub(crate) fn extract_stream_payload_type(value: &Value) -> Option<String> {
    value
        .get("type")
        .and_then(|entry| entry.as_str())
        .map(|entry| entry.to_string())
}

pub(crate) fn stream_payload_service_tier_rank(event_name: Option<&str>, value: &Value) -> u8 {
    match event_name.or_else(|| value.get("type").and_then(|entry| entry.as_str())) {
        Some("response.completed" | "response.failed") => 2,
        Some("response.created" | "response.in_progress") => 1,
        _ => 0,
    }
}

pub(crate) fn should_overwrite_stream_service_tier(
    current_tier: Option<&str>,
    current_rank: u8,
    next_tier: &str,
    next_rank: u8,
) -> bool {
    let Some(current_tier) = current_tier else {
        return true;
    };

    if next_rank > current_rank {
        return true;
    }
    if next_rank < current_rank {
        return false;
    }

    match (current_tier, next_tier) {
        (AUTO_SERVICE_TIER, AUTO_SERVICE_TIER) => true,
        (AUTO_SERVICE_TIER, _) => true,
        (_, AUTO_SERVICE_TIER) => false,
        _ => true,
    }
}

pub(crate) fn stream_payload_indicates_failure(event_name: Option<&str>, value: &Value) -> bool {
    matches!(event_name, Some("response.failed") | Some("error"))
        || value
            .get("type")
            .and_then(|entry| entry.as_str())
            .is_some_and(|kind| kind == "response.failed" || kind == "error")
        || value
            .pointer("/response/status")
            .and_then(|entry| entry.as_str())
            .is_some_and(|status| status.eq_ignore_ascii_case("failed"))
}

pub(crate) fn extract_upstream_error_object(value: &Value) -> Option<&Value> {
    value
        .get("error")
        .filter(|entry| entry.is_object())
        .or_else(|| {
            value
                .pointer("/response/error")
                .filter(|entry| entry.is_object())
        })
}

pub(crate) fn extract_upstream_error_code(value: &Value) -> Option<String> {
    extract_upstream_error_object(value)
        .and_then(|entry| entry.get("code"))
        .and_then(|entry| entry.as_str())
        .map(|entry| entry.to_string())
        .or_else(|| {
            value
                .get("code")
                .and_then(|entry| entry.as_str())
                .map(|entry| entry.to_string())
        })
}

pub(crate) fn extract_upstream_error_message(value: &Value) -> Option<String> {
    extract_upstream_error_object(value)
        .and_then(|entry| entry.get("message"))
        .and_then(|entry| entry.as_str())
        .map(|entry| entry.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|entry| entry.as_str())
                .map(|entry| entry.to_string())
        })
}

pub(crate) fn extract_upstream_request_id(value: &Value) -> Option<String> {
    extract_upstream_error_object(value)
        .and_then(|entry| {
            entry
                .get("request_id")
                .or_else(|| entry.get("requestId"))
                .and_then(|value| value.as_str())
        })
        .map(|entry| entry.to_string())
        .or_else(|| {
            value
                .get("request_id")
                .or_else(|| value.get("requestId"))
                .and_then(|entry| entry.as_str())
                .map(|entry| entry.to_string())
        })
        .or_else(|| {
            extract_upstream_error_message(value)
                .and_then(|message| extract_request_id_from_message(&message))
        })
}

pub(crate) fn find_first_sse_event_boundary(bytes: &[u8]) -> Option<usize> {
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\n' && bytes[index + 1] == b'\n' {
            return Some(index + 2);
        }
        if index + 3 < bytes.len()
            && bytes[index] == b'\r'
            && bytes[index + 1] == b'\n'
            && bytes[index + 2] == b'\r'
            && bytes[index + 3] == b'\n'
        {
            return Some(index + 4);
        }
        index += 1;
    }
    None
}

pub(crate) fn initial_sse_event_kind(bytes: &[u8]) -> Option<String> {
    let mut event_name = None;
    let mut data_lines = Vec::new();
    for line in String::from_utf8_lossy(bytes).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(':') {
            continue;
        }
        if trimmed.starts_with("event:") {
            let candidate = trimmed.trim_start_matches("event:").trim();
            if !candidate.is_empty() {
                event_name = Some(candidate.to_string());
            }
            continue;
        }
        if trimmed.starts_with("data:") {
            let payload = trimmed.trim_start_matches("data:").trim();
            if !payload.is_empty() {
                data_lines.push(payload.to_string());
            }
        }
    }

    if data_lines.is_empty() {
        return event_name;
    }

    let payload = data_lines.join("\n");
    if payload == "[DONE]" {
        return Some("[DONE]".to_string());
    }

    serde_json::from_str::<Value>(&payload)
        .ok()
        .and_then(|value| event_name.clone().or_else(|| extract_stream_payload_type(&value)))
        .or(event_name)
}

pub(crate) fn build_retryable_overload_gate_outcome(
    upstream_error_code: Option<String>,
    upstream_error_message: Option<String>,
    upstream_request_id: Option<String>,
) -> PoolInitialResponseGateOutcome {
    let response_info = ResponseCaptureInfo {
        model: None,
        usage: ParsedUsage::default(),
        usage_missing_reason: Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string()),
        service_tier: None,
        stream_terminal_event: Some("response.failed".to_string()),
        upstream_error_code: upstream_error_code.clone(),
        upstream_error_message: upstream_error_message.clone(),
        upstream_request_id: upstream_request_id.clone(),
    };
    PoolInitialResponseGateOutcome::RetrySameAccount {
        message: format_upstream_response_failed_message(&response_info),
        upstream_error_code,
        upstream_error_message,
        upstream_request_id,
    }
}

pub(crate) enum PoolInitialResponsesSseEventDecision {
    ContinueMetadata,
    Forward,
    RetrySameAccount {
        upstream_error_code: Option<String>,
        upstream_error_message: Option<String>,
        upstream_request_id: Option<String>,
    },
}

pub(crate) fn classify_pool_initial_responses_sse_event(
    status: StatusCode,
    event_bytes: &[u8],
) -> PoolInitialResponsesSseEventDecision {
    let response_info = parse_stream_response_payload(event_bytes);
    if response_info_is_retryable_server_overloaded(status, &response_info) {
        return PoolInitialResponsesSseEventDecision::RetrySameAccount {
            upstream_error_code: response_info.upstream_error_code,
            upstream_error_message: response_info.upstream_error_message,
            upstream_request_id: response_info.upstream_request_id,
        };
    }

    match initial_sse_event_kind(event_bytes).as_deref() {
        None | Some("response.created" | "response.in_progress") => {
            PoolInitialResponsesSseEventDecision::ContinueMetadata
        }
        _ => PoolInitialResponsesSseEventDecision::Forward,
    }
}

pub(crate) fn rebuild_proxy_upstream_response_stream(
    status: StatusCode,
    headers: &HeaderMap,
    stream: Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>>,
) -> Result<ProxyUpstreamResponseBody, String> {
    let mut response_builder = Response::builder().status(status);
    for (name, value) in headers {
        response_builder = response_builder.header(name, value);
    }
    response_builder
        .body(Body::from_stream(stream))
        .map(ProxyUpstreamResponseBody::Axum)
        .map_err(|err| format!("failed to rebuild upstream response stream: {err}"))
}

pub(crate) enum PoolInitialResponseGateOutcome {
    Forward {
        response: ProxyUpstreamResponseBody,
        prefetched_bytes: Option<Bytes>,
    },
    RetrySameAccount {
        message: String,
        upstream_error_code: Option<String>,
        upstream_error_message: Option<String>,
        upstream_request_id: Option<String>,
    },
}

pub(crate) async fn gate_pool_initial_response_stream(
    response: ProxyUpstreamResponseBody,
    prefetched_first_chunk: Option<Bytes>,
    total_timeout: Duration,
    started: Instant,
) -> Result<PoolInitialResponseGateOutcome, String> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut stream = response.into_bytes_stream();
    let mut buffered = Vec::new();
    let mut scanned_bytes = 0usize;
    let mut saw_non_metadata_event = false;
    if let Some(chunk) = prefetched_first_chunk {
        buffered.extend_from_slice(&chunk);
    }

    let mut gate_stream_error: Option<io::Error> = None;
    loop {
        while let Some(relative_event_end) = find_first_sse_event_boundary(&buffered[scanned_bytes..]) {
            let event_end = scanned_bytes + relative_event_end;
            match classify_pool_initial_responses_sse_event(status, &buffered[scanned_bytes..event_end]) {
                PoolInitialResponsesSseEventDecision::ContinueMetadata => {
                    scanned_bytes = event_end;
                }
                PoolInitialResponsesSseEventDecision::Forward => {
                    scanned_bytes = event_end;
                    saw_non_metadata_event = true;
                    break;
                }
                PoolInitialResponsesSseEventDecision::RetrySameAccount {
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                } => {
                    return Ok(build_retryable_overload_gate_outcome(
                        upstream_error_code,
                        upstream_error_message,
                        upstream_request_id,
                    ));
                }
            }
        }
        if saw_non_metadata_event {
            break;
        }
        if buffered.len() >= RAW_RESPONSE_PREVIEW_LIMIT {
            break;
        }

        let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed())
        else {
            break;
        };
        let next_chunk = match timeout(timeout_budget, stream.next()).await {
            Ok(next_chunk) => next_chunk,
            Err(_) => break,
        };
        let Some(next_chunk) = next_chunk else {
            break;
        };
        match next_chunk {
            Ok(chunk) => buffered.extend_from_slice(&chunk),
            Err(err) => {
                gate_stream_error = Some(io::Error::other(err.to_string()));
                break;
            }
        }
    }

    let remaining_stream: Pin<
        Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>,
    > = if let Some(err) = gate_stream_error {
        Box::pin(stream::once(async move { Err(err) }))
    } else {
        stream
    };
    let rebuilt_response =
        rebuild_proxy_upstream_response_stream(status, &headers, remaining_stream)?;
    Ok(PoolInitialResponseGateOutcome::Forward {
        response: rebuilt_response,
        prefetched_bytes: (!buffered.is_empty()).then_some(Bytes::from(buffered)),
    })
}

pub(crate) fn gate_pool_initial_compact_response(
    status: StatusCode,
    headers: &HeaderMap,
    prefetched_first_chunk: Option<&Bytes>,
) -> Option<PoolInitialResponseGateOutcome> {
    if status != StatusCode::OK {
        return None;
    }

    let first_chunk = prefetched_first_chunk?;
    if first_chunk.is_empty() {
        return None;
    }

    let content_encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok());
    let (decoded_first_chunk, _) =
        decode_response_payload_for_preview_parse(first_chunk.as_ref(), content_encoding);
    let value = serde_json::from_slice::<Value>(decoded_first_chunk.as_ref()).ok()?;
    let error_object = extract_upstream_error_object(&value)?;
    let upstream_error_code = error_object
        .get("code")
        .and_then(|entry| entry.as_str())
        .map(str::to_string);
    if !upstream_error_code_is_server_overloaded(upstream_error_code.as_deref()) {
        return None;
    }

    Some(build_retryable_overload_gate_outcome(
        upstream_error_code,
        extract_upstream_error_message(&value),
        extract_upstream_request_id(&value),
    ))
}

pub(crate) fn extract_request_id_from_message(message: &str) -> Option<String> {
    let lower_message = message.to_ascii_lowercase();
    let start = lower_message
        .find("request id ")
        .map(|index| index + "request id ".len())
        .or_else(|| {
            lower_message
                .find("request_id=")
                .map(|index| index + "request_id=".len())
        })
        .or_else(|| {
            lower_message
                .find("x-request-id: ")
                .map(|index| index + "x-request-id: ".len())
        })?;
    let tail = &message[start..];
    let request_id: String = tail
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '-' | '_'))
        .collect();
    if request_id.is_empty() {
        None
    } else {
        Some(request_id)
    }
}

pub(crate) fn decode_response_payload_for_usage<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
) -> (Cow<'a, [u8]>, Option<String>) {
    decode_response_payload(bytes, content_encoding, true)
}

pub(crate) fn decode_response_payload<'a>(
    bytes: &'a [u8],
    content_encoding: Option<&str>,
    allow_gzip_magic_fallback: bool,
) -> (Cow<'a, [u8]>, Option<String>) {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        if allow_gzip_magic_fallback && response_payload_looks_like_gzip_magic(bytes) {
            return decode_single_content_encoding(bytes, "gzip")
                .map(|decoded| (decoded, None))
                .unwrap_or_else(|err| {
                    (
                        Cow::Borrowed(bytes),
                        Some(format!("response_gzip_decode_error:{err}")),
                    )
                });
        }
        return (Cow::Borrowed(bytes), None);
    }

    let mut encodings = encodings.iter().rev();
    let first_encoding = encodings.next().expect("non-empty encodings checked above");
    let mut decoded = match decode_single_content_encoding(bytes, first_encoding) {
        Ok(next) => next.into_owned(),
        Err(err) => {
            return (
                Cow::Borrowed(bytes),
                Some(format!("{first_encoding}:{err}")),
            );
        }
    };
    for encoding in encodings {
        match decode_single_content_encoding(decoded.as_slice(), encoding) {
            Ok(next) => decoded = next.into_owned(),
            Err(err) => {
                return (Cow::Borrowed(bytes), Some(format!("{encoding}:{err}")));
            }
        }
    }
    (Cow::Owned(decoded), None)
}

pub(crate) fn parse_content_encodings(content_encoding: Option<&str>) -> Vec<String> {
    content_encoding
        .into_iter()
        .flat_map(|raw| raw.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

pub(crate) fn decode_single_content_encoding<'a>(
    bytes: &'a [u8],
    encoding: &str,
) -> std::result::Result<Cow<'a, [u8]>, String> {
    match encoding {
        "identity" => Ok(Cow::Borrowed(bytes)),
        "gzip" | "x-gzip" => decode_gzip_payload(bytes),
        "br" => decode_brotli_payload(bytes),
        "deflate" => decode_deflate_payload(bytes),
        other => Err(format!("unsupported_content_encoding:{other}")),
    }
}

pub(crate) fn decode_gzip_payload<'a>(bytes: &'a [u8]) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut decoder = GzDecoder::new(bytes);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    Ok(Cow::Owned(decoded))
}

pub(crate) fn decode_brotli_payload<'a>(bytes: &'a [u8]) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut decoder = BrotliDecompressor::new(bytes, 4096);
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|err| err.to_string())?;
    Ok(Cow::Owned(decoded))
}

pub(crate) fn decode_deflate_payload<'a>(bytes: &'a [u8]) -> std::result::Result<Cow<'a, [u8]>, String> {
    let mut zlib_decoder = ZlibDecoder::new(bytes);
    let mut decoded = Vec::new();
    match zlib_decoder.read_to_end(&mut decoded) {
        Ok(_) => Ok(Cow::Owned(decoded)),
        Err(zlib_err) => {
            let mut raw_decoder = DeflateDecoder::new(bytes);
            let mut raw_decoded = Vec::new();
            raw_decoder
                .read_to_end(&mut raw_decoded)
                .map_err(|raw_err| format!("zlib={zlib_err}; raw={raw_err}"))?;
            Ok(Cow::Owned(raw_decoded))
        }
    }
}

pub(crate) fn response_payload_looks_like_gzip_magic(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

pub(crate) fn extract_model_from_payload(value: &Value) -> Option<String> {
    value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .pointer("/response/model")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

pub(crate) fn extract_partial_json_string_field(bytes: &[u8], keys: &[&str]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    keys.iter().find_map(|key| {
        let pattern = format!(r#""{}"\s*:\s*"((?:\\.|[^"\\])*)""#, regex::escape(key));
        let regex = Regex::new(&pattern).ok()?;
        let captures = regex.captures(text)?;
        let value = captures.get(1)?.as_str();
        serde_json::from_str::<String>(&format!("\"{value}\"")).ok()
    })
}

pub(crate) fn extract_partial_json_model(bytes: &[u8]) -> Option<String> {
    extract_partial_json_string_field(bytes, &["model"])
}

pub(crate) fn extract_partial_json_service_tier(bytes: &[u8]) -> Option<String> {
    extract_partial_json_string_field(bytes, &["service_tier", "serviceTier"])
        .and_then(|value| normalize_service_tier(&value))
}

pub(crate) fn normalize_service_tier(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

pub(crate) const AUTO_SERVICE_TIER: &str = "auto";
pub(crate) const DEFAULT_SERVICE_TIER: &str = "default";
pub(crate) const PRIORITY_SERVICE_TIER: &str = "priority";
pub(crate) const API_KEYS_BILLING_ACCOUNT_KIND: &str = "api_key_codex";
pub(crate) const REQUESTED_TIER_PRICE_VERSION_SUFFIX: &str = "@requested-tier";
pub(crate) const RESPONSE_TIER_PRICE_VERSION_SUFFIX: &str = "@response-tier";
pub(crate) const EXPLICIT_BILLING_PRICE_VERSION_SUFFIX: &str = "@explicit-billing";
pub(crate) const SERVICE_TIER_STREAM_BACKFILL_VERSION: &str = "stream-terminal-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyPricingMode {
    ResponseTier,
    RequestedTier,
    ExplicitBilling,
}

impl ProxyPricingMode {
    fn price_version_suffix(self) -> &'static str {
        match self {
            Self::ResponseTier => RESPONSE_TIER_PRICE_VERSION_SUFFIX,
            Self::RequestedTier => REQUESTED_TIER_PRICE_VERSION_SUFFIX,
            Self::ExplicitBilling => EXPLICIT_BILLING_PRICE_VERSION_SUFFIX,
        }
    }
}

pub(crate) fn normalize_upstream_base_url_host(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.trim().to_ascii_lowercase()))
        .or_else(|| {
            let host = raw.trim().trim_end_matches('/').to_ascii_lowercase();
            if host.is_empty() || host.contains('/') {
                None
            } else {
                Some(host)
            }
        })
        .filter(|host| !host.is_empty())
}

pub(crate) fn api_keys_billing_matches_context(upstream_account_kind: Option<&str>) -> bool {
    upstream_account_kind
        .map(str::trim)
        .is_some_and(|kind| kind.eq_ignore_ascii_case(API_KEYS_BILLING_ACCOUNT_KIND))
}

pub(crate) fn resolve_proxy_billing_service_tier_and_pricing_mode(
    explicit_billing_service_tier: Option<&str>,
    requested_service_tier: Option<&str>,
    response_service_tier: Option<&str>,
    upstream_account_kind: Option<&str>,
) -> (Option<String>, ProxyPricingMode) {
    if let Some(explicit_billing_service_tier) =
        explicit_billing_service_tier.and_then(normalize_service_tier)
    {
        return (
            Some(explicit_billing_service_tier),
            ProxyPricingMode::ExplicitBilling,
        );
    }

    let normalized_requested_service_tier = requested_service_tier.and_then(normalize_service_tier);
    let normalized_response_service_tier = response_service_tier.and_then(normalize_service_tier);
    if api_keys_billing_matches_context(upstream_account_kind)
        && normalized_requested_service_tier.is_some()
    {
        return (
            normalized_requested_service_tier,
            ProxyPricingMode::RequestedTier,
        );
    }

    (
        normalized_response_service_tier,
        ProxyPricingMode::ResponseTier,
    )
}

pub(crate) fn resolve_proxy_billing_service_tier_and_pricing_mode_for_account(
    explicit_billing_service_tier: Option<&str>,
    requested_service_tier: Option<&str>,
    response_service_tier: Option<&str>,
    account: Option<&PoolResolvedAccount>,
) -> (Option<String>, ProxyPricingMode) {
    resolve_proxy_billing_service_tier_and_pricing_mode(
        explicit_billing_service_tier,
        requested_service_tier,
        response_service_tier,
        account.map(|entry| entry.kind.as_str()),
    )
}

pub(crate) fn payload_summary_upstream_account_kind(account: Option<&PoolResolvedAccount>) -> Option<&str> {
    account.map(|entry| entry.kind.as_str())
}

pub(crate) fn payload_summary_upstream_base_url_host(account: Option<&PoolResolvedAccount>) -> Option<&str> {
    account.and_then(|entry| entry.upstream_base_url.host_str())
}

pub(crate) fn resolve_backfill_upstream_account_kind(
    snapshot_kind: Option<&str>,
    live_kind: Option<&str>,
    allow_live_fallback: bool,
) -> Option<String> {
    if let Some(value) = snapshot_kind.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(value.to_string());
    }

    if !allow_live_fallback {
        return None;
    }

    live_kind
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn allow_live_upstream_account_fallback(raw: Option<i64>) -> bool {
    raw == Some(1)
}

pub(crate) fn resolve_backfill_upstream_base_url_host(
    snapshot_host: Option<&str>,
    live_host: Option<&str>,
    allow_live_fallback: bool,
) -> Option<String> {
    snapshot_host.and_then(normalize_upstream_base_url_host).or_else(|| {
        allow_live_fallback
            .then_some(live_host)
            .flatten()
            .and_then(normalize_upstream_base_url_host)
    })
}

pub(crate) fn extract_service_tier_from_payload(value: &Value) -> Option<String> {
    [
        "/service_tier",
        "/serviceTier",
        "/response/service_tier",
        "/response/serviceTier",
    ]
    .iter()
    .find_map(|pointer| value.pointer(pointer).and_then(|v| v.as_str()))
    .and_then(normalize_service_tier)
}

pub(crate) fn extract_usage_from_payload(value: &Value) -> Option<ParsedUsage> {
    if let Some(usage) = value.get("usage") {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens.is_some()
            || parsed.input_tokens.is_some()
            || parsed.output_tokens.is_some()
        {
            return Some(parsed);
        }
    }
    if let Some(usage) = value.pointer("/response/usage") {
        let parsed = parse_usage_value(usage);
        if parsed.total_tokens.is_some()
            || parsed.input_tokens.is_some()
            || parsed.output_tokens.is_some()
        {
            return Some(parsed);
        }
    }
    None
}

pub(crate) fn parse_usage_value(value: &Value) -> ParsedUsage {
    let input_tokens = value
        .get("input_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| value.get("prompt_tokens").and_then(json_value_to_i64));
    let output_tokens = value
        .get("output_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| value.get("completion_tokens").and_then(json_value_to_i64));
    let cache_input_tokens = value
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| {
            value
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(json_value_to_i64)
        });
    let reasoning_tokens = value
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(json_value_to_i64)
        .or_else(|| {
            value
                .pointer("/completion_tokens_details/reasoning_tokens")
                .and_then(json_value_to_i64)
        });

    let mut parsed = ParsedUsage {
        input_tokens,
        output_tokens,
        cache_input_tokens,
        reasoning_tokens,
        total_tokens: value.get("total_tokens").and_then(json_value_to_i64),
    };

    if parsed.total_tokens.is_none() {
        parsed.total_tokens = match (parsed.input_tokens, parsed.output_tokens) {
            (Some(input), Some(output)) => Some(input + output),
            _ => None,
        };
    }

    parsed
}

pub(crate) fn json_value_to_i64(value: &Value) -> Option<i64> {
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    if let Some(v) = value.as_u64() {
        return i64::try_from(v).ok();
    }
    value.as_str().and_then(|v| v.parse::<i64>().ok())
}

pub(crate) fn upstream_account_id_from_payload(payload: Option<&str>) -> Option<i64> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value.get("upstreamAccountId").and_then(json_value_to_i64)
}
