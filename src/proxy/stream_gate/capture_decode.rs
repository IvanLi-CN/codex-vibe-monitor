use super::*;
pub(crate) fn build_response_capture_info_from_bytes(
    bytes: &[u8],
    request_is_stream: bool,
    decode_failure_reason: Option<String>,
) -> ResponseCaptureInfo {
    build_response_capture_info_from_bytes_with_sse_detection(
        bytes,
        request_is_stream,
        decode_failure_reason,
        true,
        true,
    )
}

fn build_response_capture_info_from_bytes_with_sse_detection(
    bytes: &[u8],
    request_is_stream: bool,
    decode_failure_reason: Option<String>,
    allow_sse_detection: bool,
    allow_compaction_detection: bool,
) -> ResponseCaptureInfo {
    if bytes.is_empty() {
        return ResponseCaptureInfo {
            model: None,
            usage: ParsedUsage::default(),
            usage_missing_reason: Some("empty_response".to_string()),
            contains_encrypted_content: false,
            service_tier: None,
            compaction_response_kind: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
        };
    }

    let looks_like_stream =
        request_is_stream || (allow_sse_detection && response_payload_looks_like_sse(bytes));
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
                    contains_encrypted_content: value_contains_encrypted_content(&value),
                    service_tier,
                    compaction_response_kind: allow_compaction_detection
                        .then(|| response_value_indicates_remote_v2_compaction(&value))
                        .and_then(|is_compaction| {
                            is_compaction.then_some(CompactionKind::RemoteV2)
                        }),
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
                    contains_encrypted_content:
                        best_effort_extract_encrypted_content_from_request_body_prefix(bytes),
                    service_tier,
                    compaction_response_kind: None,
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
    target: ProxyCaptureTarget,
    bytes: &[u8],
    request_is_stream: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_parse(bytes, content_encoding);
    build_response_capture_info_from_bytes_with_sse_detection(
        decoded_bytes.as_ref(),
        request_is_stream && target != ProxyCaptureTarget::StandaloneSearch,
        decode_failure_reason,
        target != ProxyCaptureTarget::StandaloneSearch,
        target != ProxyCaptureTarget::StandaloneSearch,
    )
}
pub(crate) fn parse_target_response_preview_payload(
    target: ProxyCaptureTarget,
    bytes: &[u8],
    request_is_stream: bool,
    content_encoding: Option<&str>,
) -> ResponseCaptureInfo {
    let (decoded_bytes, decode_failure_reason) =
        decode_response_payload_for_preview_parse(bytes, content_encoding);
    build_response_capture_info_from_bytes_with_sse_detection(
        decoded_bytes.as_ref(),
        request_is_stream && target != ProxyCaptureTarget::StandaloneSearch,
        decode_failure_reason,
        target != ProxyCaptureTarget::StandaloneSearch,
        target != ProxyCaptureTarget::StandaloneSearch,
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
