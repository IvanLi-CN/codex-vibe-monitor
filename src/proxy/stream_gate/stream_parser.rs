use super::*;
pub(crate) struct StreamResponsePayloadParser {
    stream_semantics_enabled: bool,
    model: Option<String>,
    usage: ParsedUsage,
    service_tier: Option<String>,
    service_tier_rank: u8,
    contains_encrypted_content: bool,
    compaction_response_kind: Option<CompactionKind>,
    stream_terminal_event: Option<String>,
    successful_terminal_seen: bool,
    upstream_error_code: Option<String>,
    upstream_error_message: Option<String>,
    upstream_request_id: Option<String>,
    usage_found: bool,
    parse_error_seen: bool,
    pending_event_name: Option<String>,
    saw_stream_fields: bool,
    first_token_observed: bool,
}

impl Default for StreamResponsePayloadParser {
    fn default() -> Self {
        Self {
            stream_semantics_enabled: true,
            model: None,
            usage: ParsedUsage::default(),
            service_tier: None,
            service_tier_rank: 0,
            contains_encrypted_content: false,
            compaction_response_kind: None,
            stream_terminal_event: None,
            successful_terminal_seen: false,
            upstream_error_code: None,
            upstream_error_message: None,
            upstream_request_id: None,
            usage_found: false,
            parse_error_seen: false,
            pending_event_name: None,
            saw_stream_fields: false,
            first_token_observed: false,
        }
    }
}

impl StreamResponsePayloadParser {
    fn ingest_line(&mut self, line: &str) {
        if !self.stream_semantics_enabled {
            return;
        }
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
                if !self.first_token_observed
                    && stream_payload_contains_nonempty_model_delta(event_name.as_deref(), &value)
                {
                    self.first_token_observed = true;
                }
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
                if value_contains_encrypted_content(&value) {
                    self.contains_encrypted_content = true;
                }
                if self.compaction_response_kind.is_none()
                    && response_value_indicates_remote_v2_compaction(&value)
                {
                    self.compaction_response_kind = Some(CompactionKind::RemoteV2);
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
                if stream_payload_indicates_successful_completion(event_name.as_deref(), &value) {
                    self.successful_terminal_seen = true;
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

    pub(crate) fn finish(self) -> ResponseCaptureInfo {
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
            contains_encrypted_content: self.contains_encrypted_content,
            service_tier: self.service_tier,
            compaction_response_kind: self.compaction_response_kind,
            stream_terminal_event: self.stream_terminal_event,
            upstream_error_code: self.upstream_error_code,
            upstream_error_message: self.upstream_error_message,
            upstream_request_id: self.upstream_request_id,
        }
    }
}

pub(crate) struct StreamResponsePayloadParseOutcome {
    pub(crate) response_info: ResponseCaptureInfo,
    pub(crate) saw_stream_fields: bool,
    pub(crate) successful_terminal_seen: bool,
}

pub(crate) struct StreamResponsePayloadChunkParser {
    parser: StreamResponsePayloadParser,
    line_buffer: Vec<u8>,
    discarding_oversized_line: bool,
    line_buffer_limit: usize,
    discarded_oversized_line: bool,
}

enum IncrementalResponsePayloadDecoderKind {
    Identity,
    Flate(Decompress),
    Gzip(WriteGzipDecoder<Vec<u8>>),
    DeflateProbe,
    Brotli(Box<DecompressorWriter<Vec<u8>>>),
    Chain(Vec<IncrementalResponsePayloadDecoder>),
    Failed,
}

pub(crate) struct IncrementalResponsePayloadDecoder {
    kind: IncrementalResponsePayloadDecoderKind,
    deflate_prefix: Vec<u8>,
    brotli_decoded_bytes: usize,
    decode_failure_reason: Option<String>,
}

impl IncrementalResponsePayloadDecoder {
    pub(crate) fn new(content_encoding: Option<&str>) -> Self {
        let encodings = parse_content_encodings(content_encoding);
        let kind = if encodings.len() > 1 {
            // Content-Encoding lists transformations in application order. Decode the
            // right-most transformation first, then pass the result through the remaining
            // stages so TTFT sees the original event stream incrementally.
            IncrementalResponsePayloadDecoderKind::Chain(
                encodings
                    .iter()
                    .rev()
                    .map(|encoding| Self::new(Some(encoding)))
                    .collect(),
            )
        } else {
            match encodings.as_slice() {
                [] => IncrementalResponsePayloadDecoderKind::Identity,
                [encoding] if encoding == "identity" => {
                    IncrementalResponsePayloadDecoderKind::Identity
                }
                [encoding] if encoding == "gzip" || encoding == "x-gzip" => {
                    IncrementalResponsePayloadDecoderKind::Gzip(WriteGzipDecoder::new(Vec::new()))
                }
                [encoding] if encoding == "deflate" => {
                    IncrementalResponsePayloadDecoderKind::DeflateProbe
                }
                [encoding] if encoding == "br" => IncrementalResponsePayloadDecoderKind::Brotli(
                    Box::new(DecompressorWriter::new(Vec::new(), 4096)),
                ),
                _ => IncrementalResponsePayloadDecoderKind::Failed,
            }
        };
        Self {
            kind,
            deflate_prefix: Vec::new(),
            brotli_decoded_bytes: 0,
            decode_failure_reason: None,
        }
    }

    fn record_decode_failure(&mut self, encoding: &str, error: impl std::fmt::Display) {
        if self.decode_failure_reason.is_none() {
            self.decode_failure_reason = Some(format!("{encoding}:{error}"));
        }
    }

    pub(crate) fn ingest(&mut self, bytes: &[u8]) -> Vec<u8> {
        if bytes.is_empty() {
            return Vec::new();
        }
        let kind = std::mem::replace(
            &mut self.kind,
            IncrementalResponsePayloadDecoderKind::Failed,
        );
        match kind {
            IncrementalResponsePayloadDecoderKind::Identity => {
                self.kind = IncrementalResponsePayloadDecoderKind::Identity;
                bytes.to_vec()
            }
            IncrementalResponsePayloadDecoderKind::Flate(mut decoder) => {
                match decode_flate_chunk(&mut decoder, bytes) {
                    Ok(decoded) => {
                        self.kind = IncrementalResponsePayloadDecoderKind::Flate(decoder);
                        decoded
                    }
                    Err(_) => {
                        self.record_decode_failure("deflate", "invalid compressed stream");
                        Vec::new()
                    }
                }
            }
            IncrementalResponsePayloadDecoderKind::Gzip(mut decoder) => {
                let output_before = decoder.get_ref().len();
                if decoder.write_all(bytes).is_err() {
                    self.record_decode_failure("gzip", "invalid compressed stream");
                    return Vec::new();
                }
                if decoder.flush().is_err() {
                    self.record_decode_failure("gzip", "invalid compressed stream");
                    return Vec::new();
                }
                let decoded = decoder.get_ref()[output_before..].to_vec();
                self.kind = IncrementalResponsePayloadDecoderKind::Gzip(decoder);
                decoded
            }
            IncrementalResponsePayloadDecoderKind::DeflateProbe => {
                self.deflate_prefix.extend_from_slice(bytes);
                if self.deflate_prefix.len() < 2 {
                    self.kind = IncrementalResponsePayloadDecoderKind::DeflateProbe;
                    return Vec::new();
                }
                let prefix = std::mem::take(&mut self.deflate_prefix);
                let mut decoder = if looks_like_zlib_header(&prefix) {
                    Decompress::new(true)
                } else {
                    Decompress::new(false)
                };
                match decode_flate_chunk(&mut decoder, &prefix) {
                    Ok(decoded) => {
                        self.kind = IncrementalResponsePayloadDecoderKind::Flate(decoder);
                        decoded
                    }
                    Err(_) => {
                        self.record_decode_failure("deflate", "invalid compressed stream");
                        Vec::new()
                    }
                }
            }
            IncrementalResponsePayloadDecoderKind::Brotli(mut decoder) => {
                if decoder.write_all(bytes).is_err() {
                    self.record_decode_failure("br", "invalid compressed stream");
                    return Vec::new();
                }
                let output = decoder.get_ref();
                let decoded = output[self.brotli_decoded_bytes..].to_vec();
                self.brotli_decoded_bytes = output.len();
                self.kind = IncrementalResponsePayloadDecoderKind::Brotli(decoder);
                decoded
            }
            IncrementalResponsePayloadDecoderKind::Chain(mut decoders) => {
                let mut decoded = bytes.to_vec();
                for decoder in &mut decoders {
                    if decoded.is_empty() {
                        break;
                    }
                    decoded = decoder.ingest(&decoded);
                }
                self.kind = IncrementalResponsePayloadDecoderKind::Chain(decoders);
                decoded
            }
            IncrementalResponsePayloadDecoderKind::Failed => Vec::new(),
        }
    }

    pub(crate) fn finish(&mut self) {
        let kind = std::mem::replace(
            &mut self.kind,
            IncrementalResponsePayloadDecoderKind::Failed,
        );
        match kind {
            IncrementalResponsePayloadDecoderKind::Gzip(mut decoder) => {
                if decoder.try_finish().is_err() {
                    self.record_decode_failure("gzip", "invalid compressed stream");
                }
                self.kind = IncrementalResponsePayloadDecoderKind::Gzip(decoder);
            }
            IncrementalResponsePayloadDecoderKind::Brotli(mut decoder) => {
                if decoder.close().is_err() {
                    self.record_decode_failure("br", "invalid compressed stream");
                }
                self.kind = IncrementalResponsePayloadDecoderKind::Brotli(decoder);
            }
            IncrementalResponsePayloadDecoderKind::Chain(mut decoders) => {
                for decoder in &mut decoders {
                    decoder.finish();
                }
                self.kind = IncrementalResponsePayloadDecoderKind::Chain(decoders);
            }
            other => self.kind = other,
        }
    }

    pub(crate) fn failure_reason(&self) -> Option<&str> {
        self.decode_failure_reason.as_deref().or_else(|| {
            if let IncrementalResponsePayloadDecoderKind::Chain(decoders) = &self.kind {
                decoders.iter().find_map(Self::failure_reason)
            } else {
                None
            }
        })
    }
}

fn looks_like_zlib_header(bytes: &[u8]) -> bool {
    bytes.len() >= 2
        && bytes[0] & 0x0f == 8
        && (u16::from(bytes[0]) << 8 | u16::from(bytes[1])) % 31 == 0
}

fn decode_flate_chunk(decoder: &mut Decompress, input: &[u8]) -> Result<Vec<u8>, ()> {
    let mut output = Vec::new();
    let mut input_offset = 0usize;
    while input_offset < input.len() {
        let input_before = decoder.total_in();
        let output_before = output.len();
        output.reserve(64 * 1024);
        decoder
            .decompress_vec(&input[input_offset..], &mut output, FlushDecompress::None)
            .map_err(|_| ())?;
        let consumed = decoder.total_in().saturating_sub(input_before) as usize;
        input_offset = input_offset.saturating_add(consumed);
        if consumed == 0 && output.len() == output_before {
            break;
        }
    }
    Ok(output)
}

impl Default for StreamResponsePayloadChunkParser {
    fn default() -> Self {
        Self::with_line_buffer_limit(STREAM_RESPONSE_LINE_BUFFER_LIMIT)
    }
}

impl StreamResponsePayloadChunkParser {
    pub(super) fn with_line_buffer_limit(line_buffer_limit: usize) -> Self {
        Self {
            parser: StreamResponsePayloadParser::default(),
            line_buffer: Vec::new(),
            discarding_oversized_line: false,
            line_buffer_limit,
            discarded_oversized_line: false,
        }
    }

    pub(crate) fn for_target(target: ProxyCaptureTarget) -> Self {
        let mut parser = Self::default();
        parser.parser.stream_semantics_enabled = target != ProxyCaptureTarget::StandaloneSearch;
        parser
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
        if self.parser.stream_semantics_enabled
            && Self::line_bytes_look_like_stream_field(&self.line_buffer)
        {
            self.parser.saw_stream_fields = true;
        }
        if self.parser.stream_semantics_enabled {
            self.parser.parse_error_seen = true;
        }
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

    pub(crate) fn ingest_bytes(&mut self, bytes: &[u8]) -> bool {
        let observed_before = self.parser.first_token_observed;
        if bytes.is_empty() {
            return false;
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
        !observed_before && self.parser.first_token_observed
    }

    pub(crate) fn successful_terminal_seen(&self) -> bool {
        self.parser.stream_semantics_enabled
            && (self.parser.successful_terminal_seen || self.pending_line_is_successful_terminal())
    }

    fn pending_line_is_successful_terminal(&self) -> bool {
        if self.discarding_oversized_line || self.line_buffer.is_empty() {
            return false;
        }
        let decoded = String::from_utf8_lossy(&self.line_buffer);
        let payload = decoded
            .trim()
            .strip_prefix("data:")
            .map(str::trim)
            .filter(|payload| !payload.is_empty() && *payload != "[DONE]");
        payload
            .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
            .is_some_and(|value| {
                stream_payload_indicates_successful_completion(
                    self.parser.pending_event_name.as_deref(),
                    &value,
                )
            })
    }

    pub(crate) fn flush_pending_line(&mut self) {
        if !self.parser.stream_semantics_enabled {
            self.line_buffer.clear();
        } else if self.discarding_oversized_line {
            self.parser.parse_error_seen = true;
        } else {
            self.flush_line();
        }
    }

    pub(crate) fn finish(mut self) -> StreamResponsePayloadParseOutcome {
        if self.discarding_oversized_line {
            self.parser.parse_error_seen = true;
        } else {
            self.flush_line();
        }
        let saw_stream_fields = self.parser.saw_stream_fields;
        let successful_terminal_seen = self.parser.successful_terminal_seen;
        StreamResponsePayloadParseOutcome {
            response_info: self.parser.finish(),
            saw_stream_fields,
            successful_terminal_seen,
        }
    }
}

pub(crate) fn stream_payload_contains_nonempty_model_delta(
    event_name: Option<&str>,
    value: &Value,
) -> bool {
    let event_type = value.get("type").and_then(Value::as_str).or(event_name);
    if event_type.is_some_and(is_responses_model_output_delta_event)
        && value
            .get("delta")
            .is_some_and(json_value_contains_nonempty_output_text)
    {
        return true;
    }

    value
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|choices| {
            choices.iter().any(|choice| {
                let Some(delta) = choice.get("delta") else {
                    return false;
                };
                ["content", "reasoning", "reasoning_content", "refusal"]
                    .iter()
                    .any(|key| {
                        delta
                            .get(*key)
                            .is_some_and(json_value_contains_nonempty_output_text)
                    })
                    || delta
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .is_some_and(|calls| {
                            calls.iter().any(|call| {
                                call.pointer("/function/arguments")
                                    .and_then(Value::as_str)
                                    .is_some_and(|arguments| !arguments.is_empty())
                            })
                        })
                    || delta
                        .pointer("/function_call/arguments")
                        .and_then(Value::as_str)
                        .is_some_and(|arguments| !arguments.is_empty())
            })
        })
}

fn is_responses_model_output_delta_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "response.output_text.delta"
            | "response.reasoning_text.delta"
            | "response.reasoning_summary_text.delta"
            | "response.refusal.delta"
            | "response.function_call_arguments.delta"
    )
}

fn json_value_contains_nonempty_output_text(value: &Value) -> bool {
    match value {
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => items.iter().any(json_value_contains_nonempty_output_text),
        Value::Object(object) => ["text", "content", "arguments"].iter().any(|key| {
            object
                .get(*key)
                .is_some_and(json_value_contains_nonempty_output_text)
        }),
        _ => false,
    }
}

#[cfg(test)]
mod ttft_tests {
    use super::*;

    #[test]
    fn ttft_ignores_preamble_empty_delta_and_failure() {
        let mut parser = StreamResponsePayloadChunkParser::default();
        assert!(!parser.ingest_bytes(
            b"data: {\"type\":\"response.created\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"\"}\n\n"
        ));
        assert!(!parser.ingest_bytes(
            b"data: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\"}}\n\n"
        ));
    }

    #[test]
    fn ttft_detects_first_nonempty_delta_across_chunks_once() {
        let mut parser = StreamResponsePayloadChunkParser::default();
        assert!(!parser.ingest_bytes(
            b"event: response.reasoning_summary_text.delta\ndata: {\"type\":\"response.reasoning_summary_text.delta\",\"del"
        ));
        assert!(parser.ingest_bytes(b"ta\":\"thinking\"}\n\n"));
        assert!(!parser.ingest_bytes(
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\n"
        ));
    }

    #[test]
    fn ttft_decodes_compressed_sse_before_parsing() {
        let raw = b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\n";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(raw).expect("write gzip SSE payload");
        let compressed = encoder.finish().expect("finish gzip SSE payload");

        let split = compressed.len() / 2;
        let mut decoder = IncrementalResponsePayloadDecoder::new(Some("gzip"));
        let mut parser = StreamResponsePayloadChunkParser::default();
        assert!(!parser.ingest_bytes(&decoder.ingest(&compressed[..split])));
        assert!(parser.ingest_bytes(&decoder.ingest(&compressed[split..])));
    }

    #[test]
    fn ttft_decodes_stacked_content_encodings_before_parsing() {
        let raw = b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\n";
        let mut gzip_encoder = GzEncoder::new(Vec::new(), Compression::default());
        gzip_encoder.write_all(raw).expect("write gzip SSE payload");
        let gzip_compressed = gzip_encoder.finish().expect("finish gzip SSE payload");
        let mut stacked = Vec::new();
        {
            let mut brotli_encoder = brotli::CompressorWriter::new(&mut stacked, 4096, 5, 22);
            brotli_encoder
                .write_all(&gzip_compressed)
                .expect("write stacked brotli payload");
            brotli_encoder
                .flush()
                .expect("flush stacked brotli payload");
        }

        let mut decoder = IncrementalResponsePayloadDecoder::new(Some("gzip, br"));
        let mut parser = StreamResponsePayloadChunkParser::default();
        assert!(parser.ingest_bytes(&decoder.ingest(&stacked)));
    }

    #[test]
    fn ttft_detects_responses_tool_arguments_and_chat_content() {
        assert!(stream_payload_contains_nonempty_model_delta(
            None,
            &serde_json::json!({
                "type": "response.function_call_arguments.delta",
                "delta": "{\"city\":"
            }),
        ));
        assert!(stream_payload_contains_nonempty_model_delta(
            None,
            &serde_json::json!({
                "choices": [{"delta": {"content": "hello"}}]
            }),
        ));
        assert!(stream_payload_contains_nonempty_model_delta(
            None,
            &serde_json::json!({
                "choices": [{
                    "delta": {"function_call": {"arguments": "{\"city\":"}}
                }]
            }),
        ));
        assert!(!stream_payload_contains_nonempty_model_delta(
            None,
            &serde_json::json!({
                "choices": [{"delta": {"role": "assistant", "content": ""}}]
            }),
        ));
    }

    #[test]
    fn ttft_accepts_json_delta_type_with_generic_sse_event_name() {
        let mut parser = StreamResponsePayloadChunkParser::default();
        assert!(parser.ingest_bytes(
            b"event: message\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\n"
        ));
    }

    #[test]
    fn ttft_ignores_audio_and_unknown_delta_events() {
        for event_type in ["response.audio.delta", "response.metadata.delta"] {
            assert!(!stream_payload_contains_nonempty_model_delta(
                None,
                &serde_json::json!({
                    "type": event_type,
                    "delta": "non-empty"
                }),
            ));
        }
    }
}
