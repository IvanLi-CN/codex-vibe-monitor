pub(crate) fn build_raw_response_preview(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "{}".to_string();
    }
    let preview = if bytes.len() > RAW_RESPONSE_PREVIEW_LIMIT {
        &bytes[..RAW_RESPONSE_PREVIEW_LIMIT]
    } else {
        bytes
    };
    String::from_utf8_lossy(preview).to_string()
}

pub(crate) fn extract_error_message_from_response(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    value
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

pub(crate) fn summarize_plaintext_upstream_error(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if text.starts_with('<')
        || lower.starts_with("<!doctype")
        || lower.starts_with("<html")
        || lower.starts_with("<body")
    {
        return None;
    }
    Some(text.chars().take(240).collect())
}

pub(crate) fn extract_error_message_from_response_preview(bytes: &[u8]) -> Option<String> {
    extract_error_message_from_response(bytes).or_else(|| summarize_plaintext_upstream_error(bytes))
}

pub(crate) fn merge_response_capture_reason(
    response_info: &mut ResponseCaptureInfo,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
        format!("{reason};{existing}")
    } else {
        reason
    };
    response_info.usage_missing_reason = Some(combined_reason);
}

#[cfg(test)]
mod raw_overflow_spool_tests {
    use super::*;

    #[test]
    fn bounded_streaming_ingress_marks_capture_unavailable_without_blocking() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let mut writer = AsyncStreamingRawPayloadWriter {
            tx: Some(tx),
            meta_rx: None,
            observed_size_bytes: 0,
            local_truncated_reason: None,
            local_truncated: false,
            spool: None,
        };

        writer.append(b"first");
        writer.append(b"second");

        assert!(writer.local_truncated);
        assert_eq!(
            writer.local_truncated_reason.as_deref(),
            Some("capture_unavailable:ingress_queue_full")
        );
    }

    #[test]
    fn overflow_spool_preserves_configured_max_bytes_reason() {
        assert_eq!(
            raw_overflow_payload_limit_reason(Some(1024), 1024),
            "max_bytes_exceeded"
        );
        assert_eq!(
            raw_overflow_payload_limit_reason(Some(2048), 1024),
            "spool_capacity_exceeded"
        );
    }
}
