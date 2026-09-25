use super::*;

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

    fn key_matches_pattern(key: &str, pattern: &[u8]) -> bool {
        pattern.len() >= 2
            && pattern.first() == Some(&b'"')
            && pattern.last() == Some(&b'"')
            && key.as_bytes() == &pattern[1..pattern.len() - 1]
    }

    fn skip_json_value(bytes: &[u8], index: usize) -> Option<usize> {
        match bytes.get(index)? {
            b'"' => parse_json_string(bytes, index).map(|(_, next_index)| next_index),
            b'{' | b'[' => {
                let mut stack = vec![bytes[index]];
                let mut cursor = index + 1;
                let mut in_string = false;
                let mut escaped = false;
                while cursor < bytes.len() {
                    let byte = bytes[cursor];
                    if in_string {
                        if escaped {
                            escaped = false;
                        } else if byte == b'\\' {
                            escaped = true;
                        } else if byte == b'"' {
                            in_string = false;
                        }
                        cursor += 1;
                        continue;
                    }
                    match byte {
                        b'"' => in_string = true,
                        b'{' | b'[' => stack.push(byte),
                        b'}' => {
                            if stack.pop() != Some(b'{') {
                                return None;
                            }
                            if stack.is_empty() {
                                return Some(cursor + 1);
                            }
                        }
                        b']' => {
                            if stack.pop() != Some(b'[') {
                                return None;
                            }
                            if stack.is_empty() {
                                return Some(cursor + 1);
                            }
                        }
                        _ => {}
                    }
                    cursor += 1;
                }
                None
            }
            _ => {
                let mut cursor = index;
                while cursor < bytes.len() {
                    match bytes[cursor] {
                        b',' | b'}' | b']' => return Some(cursor),
                        _ => cursor += 1,
                    }
                }
                Some(cursor)
            }
        }
    }

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
    let mut hosted_image_intent = match target {
        ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits => {
            ImageIntent::DirectImage
        }
        _ => ImageIntent::Unknown,
    };
    let mut info = RequestCaptureInfo {
        model: None,
        sticky_key: None,
        prompt_cache_key: None,
        prompt_cache_key_attribution_source: None,
        contains_encrypted_content: false,
        image_intent: Some(
            match target {
                ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits => {
                    ImageIntent::DirectImage
                }
                _ => ImageIntent::Unknown,
            }
            .as_str()
            .to_string(),
        ),
        requested_service_tier: None,
        reasoning_effort: None,
        compaction_request_kind: None,
        is_stream: false,
        parse_error: None,
    };

    if body.is_empty() {
        return (body, info, false, hosted_image_intent);
    }

    let mut value: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(err) => {
            info.parse_error = Some(format!("request_json_parse_error:{err}"));
            return (body, info, false, hosted_image_intent);
        }
    };

    info.model = value
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    info.sticky_key = extract_sticky_key_from_request_body(&value);
    info.prompt_cache_key = extract_prompt_cache_key_from_request_body(&value);
    if info.prompt_cache_key.is_some() {
        info.prompt_cache_key_attribution_source = Some("request".to_string());
    }
    info.reasoning_effort = extract_reasoning_effort_from_request_body(target, &value);
    info.contains_encrypted_content = value_contains_encrypted_content(&value);
    info.compaction_request_kind = match target {
        ProxyCaptureTarget::ResponsesCompact => Some(CompactionKind::Compact),
        ProxyCaptureTarget::Responses if request_declares_remote_v2_compaction(&value) => {
            Some(CompactionKind::RemoteV2)
        }
        _ => None,
    };
    info.is_stream = if target == ProxyCaptureTarget::StandaloneSearch {
        false
    } else {
        value
            .get("stream")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    info.image_intent = Some(
        infer_image_intent_from_request_body(target, &value)
            .as_str()
            .to_string(),
    );
    hosted_image_intent = infer_hosted_image_intent_from_request_body(target, &value);

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
