use super::*;

pub(crate) const HEADER_STICKY_EARLY_STICKY_SCAN_BYTES: usize = 64 * 1024;

pub(crate) fn value_contains_compaction_output_item(value: &Value) -> bool {
    value
        .get("output")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == "compaction")
            })
        })
}

pub(crate) fn response_value_indicates_remote_v2_compaction(value: &Value) -> bool {
    value
        .get("object")
        .and_then(Value::as_str)
        .is_some_and(|object| object == "response.compaction")
        || value_contains_compaction_output_item(value)
        || value
            .get("response")
            .is_some_and(value_contains_compaction_output_item)
        || value.get("item").is_some_and(|item| {
            item.get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "compaction")
        })
}

pub(crate) fn best_effort_extract_json_string_for_patterns(
    bytes: &[u8],
    patterns: &[&[u8]],
) -> Option<String> {
    best_effort_extract_json_string_for_patterns_with_empty(bytes, patterns, false)
}

fn best_effort_extract_json_string_for_patterns_with_empty(
    bytes: &[u8],
    patterns: &[&[u8]],
    preserve_empty: bool,
) -> Option<String> {
    let mut cursor = skip_ascii_whitespace(bytes, 0);
    if bytes.get(cursor) != Some(&b'{') {
        return None;
    }
    cursor += 1;

    loop {
        cursor = skip_ascii_whitespace(bytes, cursor);
        match bytes.get(cursor) {
            Some(b'}') | None => return None,
            Some(b',') => {
                cursor += 1;
                continue;
            }
            Some(b'"') => {}
            Some(_) => return None,
        }

        let (key, next_index) = parse_json_string(bytes, cursor)?;
        let matches_pattern = patterns
            .iter()
            .any(|pattern| key_matches_pattern(&key, pattern));

        cursor = skip_ascii_whitespace(bytes, next_index);
        if bytes.get(cursor) != Some(&b':') {
            return None;
        }
        cursor = skip_ascii_whitespace(bytes, cursor + 1);

        cursor = if matches_pattern {
            if let Some((value, next_value_index)) = parse_json_string(bytes, cursor) {
                let normalized = value.trim();
                if preserve_empty || !normalized.is_empty() {
                    return Some(normalized.to_string());
                }
                next_value_index
            } else {
                skip_json_value(bytes, cursor)?
            }
        } else {
            skip_json_value(bytes, cursor)?
        };
    }
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

fn key_matches_pattern(key: &str, pattern: &[u8]) -> bool {
    pattern.len() >= 2
        && pattern.first() == Some(&b'"')
        && pattern.last() == Some(&b'"')
        && key.as_bytes() == &pattern[1..pattern.len() - 1]
}

fn skip_json_value(bytes: &[u8], index: usize) -> Option<usize> {
    match bytes.get(index)? {
        b'"' => parse_json_string(bytes, index).map(|(_, next_index)| next_index),
        b'{' | b'[' => skip_json_container(bytes, index),
        _ => skip_json_scalar(bytes, index),
    }
}

fn skip_json_container(bytes: &[u8], index: usize) -> Option<usize> {
    let mut stack = vec![bytes[index]];
    let mut cursor = index + 1;
    let mut in_string = false;
    let mut escaped = false;
    while cursor < bytes.len() {
        match consume_json_container_byte(bytes[cursor], &mut stack, &mut in_string, &mut escaped) {
            Ok(true) => return Some(cursor + 1),
            Ok(false) => {}
            Err(()) => return None,
        }
        cursor += 1;
    }
    None
}

fn consume_json_container_byte(
    byte: u8,
    stack: &mut Vec<u8>,
    in_string: &mut bool,
    escaped: &mut bool,
) -> Result<bool, ()> {
    if *in_string {
        if *escaped {
            *escaped = false;
        } else if byte == b'\\' {
            *escaped = true;
        } else if byte == b'"' {
            *in_string = false;
        }
        return Ok(false);
    }
    match byte {
        b'"' => *in_string = true,
        b'{' | b'[' => stack.push(byte),
        b'}' if stack.pop() != Some(b'{') => return Err(()),
        b']' if stack.pop() != Some(b'[') => return Err(()),
        b'}' | b']' if stack.is_empty() => return Ok(true),
        _ => {}
    }
    Ok(false)
}

fn skip_json_scalar(bytes: &[u8], index: usize) -> Option<usize> {
    let mut cursor = index;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b',' | b'}' | b']' => return Some(cursor),
            _ => cursor += 1,
        }
    }
    Some(cursor)
}

pub(crate) fn best_effort_extract_model_from_request_body_prefix(bytes: &[u8]) -> Option<String> {
    best_effort_extract_json_string_for_patterns_with_empty(bytes, &[br#""model""#], true)
}

pub(crate) fn best_effort_extract_sticky_key_from_request_body_prefix(
    bytes: &[u8],
) -> Option<String> {
    best_effort_extract_json_string_for_patterns(
        bytes,
        &[
            br#""sticky_key""#,
            br#""stickyKey""#,
            br#""prompt_cache_key""#,
            br#""promptCacheKey""#,
        ],
    )
}

pub(crate) fn best_effort_extract_prompt_cache_key_from_request_body_prefix(
    bytes: &[u8],
) -> Option<String> {
    best_effort_extract_json_string_for_patterns(
        bytes,
        &[br#""prompt_cache_key""#, br#""promptCacheKey""#],
    )
}

pub(crate) fn best_effort_extract_encrypted_content_from_request_body_prefix(bytes: &[u8]) -> bool {
    bytes
        .windows(br#""encrypted_content""#.len())
        .any(|window| window == br#""encrypted_content""#)
        || bytes
            .windows(br#""type":"encrypted_content""#.len())
            .any(|window| window == br#""type":"encrypted_content""#)
}

pub(crate) fn value_contains_encrypted_content(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key("encrypted_content")
                || map
                    .get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|value| value == "encrypted_content")
                || map.values().any(value_contains_encrypted_content)
        }
        Value::Array(items) => items.iter().any(value_contains_encrypted_content),
        _ => false,
    }
}

pub(crate) fn prepare_target_request_body(
    target: ProxyCaptureTarget,
    body: Vec<u8>,
    auto_include_usage: bool,
) -> (Vec<u8>, RequestCaptureInfo, bool) {
    let (body, info, rewritten, _) =
        prepare_target_request_body_with_hosted_intent(target, body, auto_include_usage);
    (body, info, rewritten)
}

pub(crate) fn prepare_target_request_body_with_hosted_intent(
    target: ProxyCaptureTarget,
    body: Vec<u8>,
    auto_include_usage: bool,
) -> (Vec<u8>, RequestCaptureInfo, bool, ImageIntent) {
    let mut info = initial_request_capture_info(target);

    if body.is_empty() {
        return (body, info, false, initial_hosted_image_intent(target));
    }

    let mut value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => {
            info.parse_error = Some(format!("request_json_parse_error:{err}"));
            return (body, info, false, initial_hosted_image_intent(target));
        }
    };

    let hosted_image_intent = populate_request_capture_info(target, &value, &mut info);
    let rewritten = rewrite_stream_options_for_usage(target, auto_include_usage, &mut value, &info);

    if rewritten {
        match serde_json::to_vec(&value) {
            Ok(rewritten_body) => (rewritten_body, info, true, hosted_image_intent),
            Err(err) => {
                let mut fallback = info;
                fallback.parse_error = Some(format!("request_json_rewrite_error:{err}"));
                (body, fallback, false, hosted_image_intent)
            }
        }
    } else {
        (body, info, false, hosted_image_intent)
    }
}

fn initial_hosted_image_intent(target: ProxyCaptureTarget) -> ImageIntent {
    match target {
        ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits => {
            ImageIntent::DirectImage
        }
        _ => ImageIntent::Unknown,
    }
}

fn initial_request_capture_info(target: ProxyCaptureTarget) -> RequestCaptureInfo {
    RequestCaptureInfo {
        model: None,
        sticky_key: None,
        prompt_cache_key: None,
        prompt_cache_key_attribution_source: None,
        contains_encrypted_content: false,
        image_intent: Some(initial_hosted_image_intent(target).as_str().to_string()),
        requested_service_tier: None,
        reasoning_effort: None,
        compaction_request_kind: None,
        is_stream: false,
        parse_error: None,
    }
}

fn populate_request_capture_info(
    target: ProxyCaptureTarget,
    value: &Value,
    info: &mut RequestCaptureInfo,
) -> ImageIntent {
    info.model = value
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string);
    info.sticky_key = extract_sticky_key_from_request_body(value);
    info.prompt_cache_key = extract_prompt_cache_key_from_request_body(value);
    if info.prompt_cache_key.is_some() {
        info.prompt_cache_key_attribution_source = Some("request".to_string());
    }
    info.reasoning_effort = extract_reasoning_effort_from_request_body(target, value);
    info.contains_encrypted_content = value_contains_encrypted_content(value);
    info.compaction_request_kind = match target {
        ProxyCaptureTarget::ResponsesCompact => Some(CompactionKind::Compact),
        ProxyCaptureTarget::Responses if request_declares_remote_v2_compaction(value) => {
            Some(CompactionKind::RemoteV2)
        }
        _ => None,
    };
    info.is_stream = target != ProxyCaptureTarget::StandaloneSearch
        && value
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    info.image_intent = Some(
        infer_image_intent_from_request_body(target, value)
            .as_str()
            .to_string(),
    );
    info.requested_service_tier = extract_requested_service_tier_from_request_body(value);
    infer_hosted_image_intent_from_request_body(target, value)
}

fn rewrite_stream_options_for_usage(
    target: ProxyCaptureTarget,
    auto_include_usage: bool,
    value: &mut Value,
    info: &RequestCaptureInfo,
) -> bool {
    if !target.should_auto_include_usage() || !info.is_stream || !auto_include_usage {
        return false;
    }
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    let stream_options = object
        .entry("stream_options".to_string())
        .or_insert_with(|| json!({}));
    if let Some(stream_options_obj) = stream_options.as_object_mut() {
        stream_options_obj.insert("include_usage".to_string(), Value::Bool(true));
    } else {
        object.insert(
            "stream_options".to_string(),
            json!({ "include_usage": true }),
        );
    }
    true
}

pub(crate) fn proxy_upstream_send_timeout_for_capture_target(
    timeouts: &PoolRoutingTimeoutSettingsResolved,
    capture_target: Option<ProxyCaptureTarget>,
) -> Duration {
    match capture_target {
        Some(ProxyCaptureTarget::Responses) => timeouts.responses_first_byte_timeout,
        Some(ProxyCaptureTarget::ResponsesCompact) => timeouts.compact_first_byte_timeout,
        Some(ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits) => {
            timeouts.image_first_byte_timeout
        }
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
        Some(ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits) => {
            timeouts.image_first_byte_timeout
        }
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
        ProxyCaptureTarget::ChatCompletions | ProxyCaptureTarget::StandaloneSearch => None,
        ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits => None,
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

pub(crate) fn pool_uses_responses_timeout_failover_policy(
    original_uri: &Uri,
    method: &Method,
) -> bool {
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
        codex_imagegen_rewrite: None,
        account: None,
        status: StatusCode::GATEWAY_TIMEOUT,
        message: pool_total_timeout_exhausted_message(total_timeout),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
        blocked_binding: None,
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

pub(crate) fn pool_pre_attempt_total_timeout_error(
    total_timeout: Duration,
) -> (StatusCode, String) {
    (
        StatusCode::GATEWAY_TIMEOUT,
        pool_total_timeout_exhausted_message(total_timeout),
    )
}

pub(crate) fn pool_uses_responses_family_retry_budget_policy(
    original_uri: &Uri,
    method: &Method,
) -> bool {
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
            initial_same_account_attempts.max(POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS)
        }
    } else if distinct_account_count <= 1 {
        initial_same_account_attempts.max(1)
    } else {
        POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS
    }
}

pub(crate) const POOL_RESPONSES_FAMILY_INITIAL_OVERLOAD_ATTEMPT_BUDGET: u8 = 4;
pub(crate) const POOL_INITIAL_RESPONSE_GATE_BUFFER_LIMIT: usize = STREAM_RESPONSE_LINE_BUFFER_LIMIT;

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
        ProxyCaptureTarget::ImageGenerations
        | ProxyCaptureTarget::ImageEdits
        | ProxyCaptureTarget::StandaloneSearch => None,
    }?;

    let normalized = raw.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

#[cfg(test)]
mod request_prefix_tests {
    use super::*;

    #[test]
    fn prefix_model_extractor_reads_top_level_model() {
        let payload = br#"{"model":"gpt-5.5","input":"hello"}"#;
        assert_eq!(
            best_effort_extract_model_from_request_body_prefix(payload).as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn prefix_model_extractor_prefers_top_level_model_over_nested_content() {
        let payload = br#"{"input":"{\"model\":\"gpt-4o\"}","model":"gpt-5.5"}"#;
        assert_eq!(
            best_effort_extract_model_from_request_body_prefix(payload).as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn prefix_model_extractor_preserves_an_explicit_empty_model() {
        let payload = br#"{"model":"","input":"hello"}"#;
        assert_eq!(
            best_effort_extract_model_from_request_body_prefix(payload),
            Some(String::new())
        );
    }

    #[test]
    fn prefix_model_extractor_ignores_nested_model_without_top_level_field() {
        let payload = br#"{"input":"{\"model\":\"gpt-4o\"}"}"#;
        assert_eq!(
            best_effort_extract_model_from_request_body_prefix(payload),
            None
        );
    }

    #[test]
    fn prefix_key_extractors_read_top_level_fields() {
        let payload = br#"{"stickyKey":"sticky-1","promptCacheKey":"cache-1"}"#;
        assert_eq!(
            best_effort_extract_sticky_key_from_request_body_prefix(payload).as_deref(),
            Some("sticky-1")
        );
        assert_eq!(
            best_effort_extract_prompt_cache_key_from_request_body_prefix(payload).as_deref(),
            Some("cache-1")
        );
    }

    #[test]
    fn prefix_key_extractors_ignore_nested_fields() {
        let payload =
            br#"{"input":"{\"stickyKey\":\"nested\",\"promptCacheKey\":\"nested-cache\"}"}"#;
        assert_eq!(
            best_effort_extract_sticky_key_from_request_body_prefix(payload),
            None
        );
        assert_eq!(
            best_effort_extract_prompt_cache_key_from_request_body_prefix(payload),
            None
        );
    }
}
