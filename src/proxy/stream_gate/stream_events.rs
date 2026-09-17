use super::*;
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

pub(crate) fn stream_payload_indicates_successful_completion(
    event_name: Option<&str>,
    value: &Value,
) -> bool {
    matches!(
        (
            event_name,
            value.get("type").and_then(|entry| entry.as_str())
        ),
        (Some("response.completed"), Some("response.completed"))
    ) && value
        .pointer("/response/status")
        .and_then(|entry| entry.as_str())
        .is_some_and(|status| status == "completed")
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
        .and_then(|value| {
            event_name
                .clone()
                .or_else(|| extract_stream_payload_type(&value))
        })
        .or(event_name)
}

pub(crate) fn build_retryable_overload_gate_outcome(
    upstream_error_code: Option<String>,
    upstream_error_message: Option<String>,
    upstream_request_id: Option<String>,
    raw_body: Bytes,
) -> PoolInitialResponseGateOutcome {
    let response_info = ResponseCaptureInfo {
        model: None,
        contains_encrypted_content: false,
        usage: ParsedUsage::default(),
        usage_missing_reason: Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string()),
        service_tier: None,
        compaction_response_kind: None,
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
        raw_body,
    }
}
