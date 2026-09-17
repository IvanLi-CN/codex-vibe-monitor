pub(crate) async fn test_upstream_slow_first_chunk() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")), 1))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_hang() -> impl IntoResponse {
    tokio::time::sleep(Duration::from_secs(2)).await;
    StatusCode::NO_CONTENT
}

pub(crate) async fn test_upstream_redirect() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            http_header::LOCATION,
            HeaderValue::from_static("/v1/echo?from=redirect"),
        )],
        Body::empty(),
    )
}

pub(crate) async fn test_upstream_external_redirect() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            http_header::LOCATION,
            HeaderValue::from_static("https://example.org/outside"),
        )],
        Body::empty(),
    )
}

pub(crate) async fn test_upstream_chat_external_redirect() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            http_header::LOCATION,
            HeaderValue::from_static("https://example.org/outside"),
        )],
        Body::empty(),
    )
}

pub(crate) async fn test_upstream_responses_gzip_stream() -> impl IntoResponse {
    let payload = [
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}\n\n",
    ]
    .concat();

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(payload.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    (
        StatusCode::OK,
        [
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            ),
            (
                http_header::CONTENT_ENCODING,
                HeaderValue::from_static("gzip"),
            ),
        ],
        Body::from(compressed),
    )
}

pub(crate) fn encode_deflate_payload(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("write deflate payload");
    encoder.finish().expect("finish deflate payload")
}

pub(crate) fn encode_response_payload(bytes: &[u8], encoding: &str) -> Vec<u8> {
    match encoding {
        "gzip" => {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder
                .write_all(bytes)
                .expect("write gzip response payload");
            encoder.finish().expect("finish gzip response payload")
        }
        "br" => {
            let mut output = Vec::new();
            {
                let mut writer = brotli::CompressorWriter::new(&mut output, 4096, 5, 22);
                writer
                    .write_all(bytes)
                    .expect("write brotli response payload");
            }
            output
        }
        "deflate" => encode_deflate_payload(bytes),
        other => panic!("unsupported content encoding fixture: {other}"),
    }
}

pub(crate) fn encoded_stream_fixture_payload() -> String {
    [
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_encoded_fixture\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_encoded_fixture\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"usage\":{\"input_tokens\":17,\"output_tokens\":6,\"total_tokens\":23}}}\n\n",
    ]
    .concat()
}

pub(crate) async fn test_upstream_responses_truncated_encoded_stream(
    encoding: &'static str,
) -> Response {
    let mut encoded =
        encode_response_payload(encoded_stream_fixture_payload().as_bytes(), encoding);
    let split_at = encoded.len().saturating_div(2).clamp(1, encoded.len());
    let corrupt_index = split_at
        .saturating_div(2)
        .min(encoded.len().saturating_sub(1));
    encoded[corrupt_index] ^= 0x5a;
    let first_chunk = Bytes::copy_from_slice(&encoded[..split_at]);
    let chunks = stream::unfold(0usize, move |state| {
        let first_chunk = first_chunk.clone();
        async move {
            match state {
                0 => Some((Ok::<Bytes, io::Error>(first_chunk), 1)),
                1 => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    Some((
                        Err::<Bytes, io::Error>(io::Error::other(format!(
                            "{encoding}-truncated-after-first-chunk"
                        ))),
                        2,
                    ))
                }
                _ => None,
            }
        }
    });

    Response::builder()
        .status(StatusCode::OK)
        .header(http_header::CONTENT_TYPE, "text/event-stream")
        .header(http_header::CONTENT_ENCODING, encoding)
        .body(Body::from_stream(chunks))
        .expect("build truncated encoded stream response")
}

pub(crate) async fn test_upstream_responses_corrupt_encoded_complete_stream(
    encoding: &'static str,
) -> Response {
    let mut encoded =
        encode_response_payload(encoded_stream_fixture_payload().as_bytes(), encoding);
    let corrupt_len = encoded.len().saturating_div(2).clamp(1, encoded.len());
    encoded.truncate(corrupt_len);

    Response::builder()
        .status(StatusCode::OK)
        .header(http_header::CONTENT_TYPE, "text/event-stream")
        .header(http_header::CONTENT_ENCODING, encoding)
        .body(Body::from(encoded))
        .expect("build corrupt encoded stream response")
}

pub(crate) fn less_compressible_test_string(target_len: usize) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(target_len);
    let mut value = 0x1234_abcd_u32;
    while text.len() < target_len {
        value = value.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let _ = write!(&mut text, "{value:08x}");
    }
    text.truncate(target_len);
    text
}

pub(crate) async fn test_upstream_responses_gzip_stream_without_event_stream_header()
-> impl IntoResponse {
    let payload = [
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test_no_ct\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test_no_ct\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":19,\"output_tokens\":6,\"total_tokens\":25}}}\n\n",
    ]
    .concat();

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(payload.as_bytes())
        .expect("write gzip payload without event-stream header");
    let compressed = encoder
        .finish()
        .expect("finish gzip payload without event-stream header");

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_ENCODING,
            HeaderValue::from_static("gzip"),
        )],
        Body::from(compressed),
    )
}

pub(crate) async fn test_upstream_responses_large_gzip_stream_without_event_stream_header()
-> impl IntoResponse {
    let large_delta = less_compressible_test_string(RAW_RESPONSE_PREVIEW_LIMIT * 8);
    let payload = [
        "event: response.created\n".to_string(),
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test_no_ct_large\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}\n\n".to_string(),
        format!(
            "event: response.output_text.delta\n\
             data: {}\n\n",
            serde_json::to_string(&json!({
                "type": "response.output_text.delta",
                "delta": large_delta,
            }))
            .expect("serialize large gzip delta payload")
        ),
        "event: response.completed\n".to_string(),
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test_no_ct_large\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":23,\"output_tokens\":7,\"total_tokens\":30}}}\n\n".to_string(),
    ]
    .concat();

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(payload.as_bytes())
        .expect("write large gzip payload without event-stream header");
    let compressed = encoder
        .finish()
        .expect("finish large gzip payload without event-stream header");
    assert!(
        compressed.len() > RAW_RESPONSE_PREVIEW_LIMIT,
        "large gzip payload should exceed preview cap"
    );

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_ENCODING,
            HeaderValue::from_static("gzip"),
        )],
        Body::from(compressed),
    )
}

pub(crate) async fn test_upstream_responses_slow_success_stream() -> impl IntoResponse {
    let first = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_slow_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
    );
    let second = concat!(
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_slow_test\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15}}}\n\n",
    );
    let chunks = stream::unfold(0usize, move |state| async move {
        match state {
            0 => Some((Ok::<_, Infallible>(Bytes::from_static(first.as_bytes())), 1)),
            1 => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Some((
                    Ok::<_, Infallible>(Bytes::from_static(second.as_bytes())),
                    2,
                ))
            }
            _ => None,
        }
    });

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_responses_completed_then_stream_error() -> impl IntoResponse {
    let completed = concat!(
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_post_terminal_error\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15}}}\n\n",
    );
    let chunks = stream::unfold(0u8, move |state| async move {
        match state {
            0 => Some((
                Ok::<_, std::io::Error>(Bytes::from_static(completed.as_bytes())),
                1,
            )),
            1 => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Some((
                    Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        "synthetic post-terminal upstream reset",
                    )),
                    2,
                ))
            }
            _ => None,
        }
    });

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_responses_large_stream() -> impl IntoResponse {
    let large_delta = "x".repeat(RAW_RESPONSE_PREVIEW_LIMIT + 8 * 1024);
    let first = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_large_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
    );
    let second = format!(
        "event: response.output_text.delta\n\
         data: {}\n\n",
        serde_json::to_string(&json!({
            "type": "response.output_text.delta",
            "delta": large_delta,
        }))
        .expect("serialize large delta payload")
    );
    let third = concat!(
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_large_test\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":42,\"output_tokens\":13,\"total_tokens\":55}}}\n\n",
    );
    let chunks = stream::iter(vec![
        Ok::<_, Infallible>(Bytes::from_static(first.as_bytes())),
        Ok::<_, Infallible>(Bytes::from(second)),
        Ok::<_, Infallible>(Bytes::from_static(third.as_bytes())),
    ]);

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_responses_large_terminal_stream() -> impl IntoResponse {
    let large_terminal_text =
        less_compressible_test_string(STREAM_RESPONSE_LINE_BUFFER_LIMIT + 64 * 1024);
    let first = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_large_terminal_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
    );
    let second = format!(
        "event: response.completed\n\
         data: {}\n\n",
        serde_json::to_string(&json!({
            "type": "response.completed",
            "response": {
                "id": "resp_large_terminal_test",
                "model": "gpt-5.4",
                "status": "completed",
                "service_tier": "priority",
                "usage": {
                    "input_tokens": 77,
                    "output_tokens": 19,
                    "total_tokens": 96,
                },
                "output": [{
                    "type": "output_text",
                    "text": large_terminal_text,
                }],
            },
        }))
        .expect("serialize large terminal payload")
    );
    assert!(
        second.len() > STREAM_RESPONSE_LINE_BUFFER_LIMIT,
        "terminal SSE event should exceed the hot-path line buffer limit"
    );
    let chunks = stream::iter(vec![
        Ok::<_, Infallible>(Bytes::from_static(first.as_bytes())),
        Ok::<_, Infallible>(Bytes::from(second)),
    ]);

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_responses_oversized_delta_stream() -> impl IntoResponse {
    let oversized_delta =
        less_compressible_test_string(STREAM_RESPONSE_LINE_BUFFER_LIMIT + 64 * 1024);
    let first = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_oversized_delta_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
    );
    let second = format!(
        "event: response.output_text.delta\n\
         data: {}\n\n",
        serde_json::to_string(&json!({
            "type": "response.output_text.delta",
            "delta": oversized_delta,
        }))
        .expect("serialize oversized delta payload")
    );
    assert!(
        second.len() > STREAM_RESPONSE_LINE_BUFFER_LIMIT,
        "oversized SSE delta should exceed the hot-path line buffer limit"
    );
    let third = concat!(
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_oversized_delta_test\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":61,\"output_tokens\":17,\"total_tokens\":78}}}\n\n",
    );
    let chunks = stream::iter(vec![
        Ok::<_, Infallible>(Bytes::from_static(first.as_bytes())),
        Ok::<_, Infallible>(Bytes::from(second)),
        Ok::<_, Infallible>(Bytes::from_static(third.as_bytes())),
    ]);

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

pub(crate) async fn test_upstream_responses_failed_stream() -> impl IntoResponse {
    let payload = [
        "event: response.created
",
        r#"data: {"type":"response.created","response":{"id":"resp_fail_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "

",
        r#"data: {"type":"error","error":{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}"#,
        "

",
        "event: response.failed
",
        r#"data: {"type":"response.failed","response":{"id":"resp_fail_test","model":"gpt-5.4","status":"failed","error":{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}}"#,
        "

",
    ]
    .concat();

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from(payload),
    )
}

pub(crate) async fn test_upstream_responses_large_json_error() -> impl IntoResponse {
    let message = format!(
        "validation failed: {} tail-marker",
        "x".repeat(RAW_RESPONSE_PREVIEW_LIMIT + 8 * 1024)
    );
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": {
                "code": "invalid_request_error",
                "message": message
            }
        })),
    )
        .into_response()
}

pub(crate) async fn test_upstream_responses_large_prefixed_json_error() -> impl IntoResponse {
    let oversized_detail =
        less_compressible_test_string(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES + 8 * 1024);
    let body = format!(
        r#"{{"error":{{"code":"invalid_request_error","message":"prefix metadata should survive"}},"service_tier":"priority","detail":"{}"}}"#,
        oversized_detail
    );
    (
        StatusCode::BAD_REQUEST,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Body::from(body),
    )
}

fn test_upstream_response_mode(uri: &Uri) -> Option<&'static str> {
    const MODES: &[(&str, &str)] = &[
        ("response_failed", "mode=response_failed"),
        ("json_error", "mode=json-error"),
        (
            "large_prefixed_json_error",
            "mode=large-prefixed-json-error",
        ),
        ("slow_success", "mode=slow-success"),
        ("completed_stream_error", "mode=completed-stream-error"),
        ("large_stream", "mode=large-stream"),
        ("large_terminal_stream", "mode=large-terminal-stream"),
        ("oversized_delta_stream", "mode=oversized-delta-stream"),
        ("gzip_truncated_stream", "mode=gzip-truncated-stream"),
        ("br_truncated_stream", "mode=br-truncated-stream"),
        ("deflate_truncated_stream", "mode=deflate-truncated-stream"),
        ("gzip_corrupt_complete", "mode=gzip-corrupt-complete"),
        ("br_corrupt_complete", "mode=br-corrupt-complete"),
        ("deflate_corrupt_complete", "mode=deflate-corrupt-complete"),
        (
            "gzip_large_no_content_type",
            "mode=gzip-large-no-content-type",
        ),
        ("gzip_no_content_type", "mode=gzip-no-content-type"),
        ("gzip", "mode=gzip"),
        ("delay", "mode=delay"),
        ("no_content", "mode=no-content"),
    ];
    uri.query().and_then(|query| {
        MODES
            .iter()
            .find(|(_, needle)| query.contains(needle))
            .map(|(mode, _)| *mode)
    })
}

pub(crate) async fn test_upstream_responses(uri: Uri) -> Response {
    match test_upstream_response_mode(&uri) {
        Some("response_failed") => test_upstream_responses_failed_stream()
            .await
            .into_response(),
        Some("json_error") => test_upstream_responses_large_json_error()
            .await
            .into_response(),
        Some("large_prefixed_json_error") => test_upstream_responses_large_prefixed_json_error()
            .await
            .into_response(),
        Some("slow_success") => test_upstream_responses_slow_success_stream()
            .await
            .into_response(),
        Some("completed_stream_error") => test_upstream_responses_completed_then_stream_error()
            .await
            .into_response(),
        Some("large_stream") => test_upstream_responses_large_stream().await.into_response(),
        Some("large_terminal_stream") => test_upstream_responses_large_terminal_stream()
            .await
            .into_response(),
        Some("oversized_delta_stream") => test_upstream_responses_oversized_delta_stream()
            .await
            .into_response(),
        Some("gzip_truncated_stream") => test_upstream_responses_truncated_encoded_stream("gzip")
            .await
            .into_response(),
        Some("br_truncated_stream") => test_upstream_responses_truncated_encoded_stream("br")
            .await
            .into_response(),
        Some("deflate_truncated_stream") => {
            test_upstream_responses_truncated_encoded_stream("deflate")
                .await
                .into_response()
        }
        Some("gzip_corrupt_complete") => {
            test_upstream_responses_corrupt_encoded_complete_stream("gzip")
                .await
                .into_response()
        }
        Some("br_corrupt_complete") => {
            test_upstream_responses_corrupt_encoded_complete_stream("br")
                .await
                .into_response()
        }
        Some("deflate_corrupt_complete") => {
            test_upstream_responses_corrupt_encoded_complete_stream("deflate")
                .await
                .into_response()
        }
        Some("gzip_large_no_content_type") => {
            test_upstream_responses_large_gzip_stream_without_event_stream_header()
                .await
                .into_response()
        }
        Some("gzip_no_content_type") => {
            test_upstream_responses_gzip_stream_without_event_stream_header()
                .await
                .into_response()
        }
        Some("gzip") => test_upstream_responses_gzip_stream().await.into_response(),
        Some("delay") => {
            tokio::time::sleep(Duration::from_millis(250)).await;
            (
                StatusCode::OK,
                Json(json!({
                    "id": "resp_delayed_test",
                    "object": "response",
                    "model": "gpt-5.3-codex",
                    "usage": {"input_tokens": 12, "output_tokens": 3, "total_tokens": 15}
                })),
            )
                .into_response()
        }
        Some("no_content") => StatusCode::NO_CONTENT.into_response(),
        _ => test_upstream_stream_mid_error().await.into_response(),
    }
}

pub(crate) async fn test_upstream_responses_compact(uri: Uri) -> impl IntoResponse {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    if uri
        .query()
        .is_some_and(|query| query.contains("mode=large-json"))
    {
        return (
            StatusCode::OK,
            Json(json!({
                "id": "resp_compact_large",
                "object": "response.compaction",
                "service_tier": "priority",
                "output": [
                    {
                        "id": "cmp_large_001",
                        "type": "compaction",
                        "encrypted_content": "z".repeat(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES + 8 * 1024)
                    }
                ],
                "usage": {
                    "input_tokens": 201,
                    "output_tokens": 99,
                    "total_tokens": 300
                }
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "id": "resp_compact_test",
            "object": "response.compaction",
            "output": [
                {
                    "id": "cmp_001",
                    "type": "compaction",
                    "encrypted_content": "encrypted-summary"
                }
            ],
            "usage": {
                "input_tokens": 139,
                "input_tokens_details": {
                    "cached_tokens": 11
                },
                "output_tokens": 438,
                "output_tokens_details": {
                    "reasoning_tokens": 64
                },
                "total_tokens": 577
            }
        })),
    )
        .into_response()
}

pub(crate) async fn test_upstream_models(uri: Uri) -> impl IntoResponse {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=error"))
    {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": "upstream model list unavailable"
            })),
        )
            .into_response();
    }

    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-body"))
    {
        let chunked = stream::unfold(0u8, |state| async move {
            match state {
                0 => Some((
                    Ok::<Bytes, Infallible>(Bytes::from_static(br#"{"object":"list","data":["#)),
                    1,
                )),
                1 => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    Some((
                        Ok::<Bytes, Infallible>(Bytes::from_static(
                            br#"{"id":"slow-model","object":"model"}]}"#,
                        )),
                        2,
                    ))
                }
                _ => None,
            }
        });
        return (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            Body::from_stream(chunked),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "object": "list",
            "data": [
                {
                    "id": "upstream-model-a",
                    "object": "model",
                    "owned_by": "upstream",
                    "created": 1712345678
                },
                {
                    "id": "gpt-5.2-codex",
                    "object": "model",
                    "owned_by": "upstream",
                    "created": 1712345679
                }
            ]
        })),
    )
        .into_response()
}

pub(crate) async fn spawn_test_upstream() -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/echo", any(test_upstream_echo))
        .route("/v1/stream", any(test_upstream_stream))
        .route(
            "/v1/stream-first-error",
            any(test_upstream_stream_first_error),
        )
        .route("/v1/stream-mid-error", any(test_upstream_stream_mid_error))
        .route("/v1/429-mid-error", any(test_upstream_429_mid_error))
        .route("/v1/slow-stream", any(test_upstream_slow_stream))
        .route("/v1/slow-first-chunk", any(test_upstream_slow_first_chunk))
        .route("/v1/hang", any(test_upstream_hang))
        .route("/v1/models", get(test_upstream_models))
        .route("/v1/redirect", any(test_upstream_redirect))
        .route(
            "/v1/redirect-external",
            any(test_upstream_external_redirect),
        )
        .route(
            "/v1/chat/completions",
            any(test_upstream_chat_external_redirect),
        )
        .route("/v1/responses", any(test_upstream_responses))
        .route(
            "/v1/responses/compact",
            post(test_upstream_responses_compact),
        );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream test server");
    let addr = listener.local_addr().expect("upstream local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("upstream test server should run");
    });

    (format!("http://{addr}/"), handle)
}

pub(crate) async fn spawn_test_upstream_with_prefix(prefix: &str) -> (String, JoinHandle<()>) {
    let echo_path = format!("{prefix}/v1/echo");
    let redirect_path = format!("{prefix}/v1/redirect");
    let redirect_location = HeaderValue::from_str(&format!("{prefix}/v1/echo?from=redirect"))
        .expect("valid redirect location");

    let app = Router::new()
        .route(&echo_path, any(test_upstream_echo))
        .route(
            &redirect_path,
            any({
                let redirect_location = redirect_location.clone();
                move || {
                    let redirect_location = redirect_location.clone();
                    async move {
                        (
                            StatusCode::TEMPORARY_REDIRECT,
                            [(http_header::LOCATION, redirect_location)],
                            Body::empty(),
                        )
                    }
                }
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind prefixed upstream test server");
    let addr = listener.local_addr().expect("prefixed upstream local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("prefixed upstream test server should run");
    });

    (format!("http://{addr}{prefix}/"), handle)
}

pub(crate) async fn test_upstream_capture_target_echo(
    State(captured): State<Arc<Mutex<Vec<Value>>>>,
    uri: Uri,
    body: Bytes,
) -> Response {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let payload: Value = serde_json::from_slice(&body).expect("decode upstream captured body");
    captured.lock().await.push(payload.clone());
    let response_payload = json!({
        "id": "resp_test",
        "object": "response",
        "model": "gpt-5.3-codex",
        "service_tier": "priority",
        "usage": {
            "input_tokens": 12,
            "output_tokens": 3,
            "total_tokens": 15
        },
        "received": payload,
    });

    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-first-chunk"))
    {
        return chunked_json_response_with_delayed_first_chunk(
            response_payload,
            Duration::from_millis(250),
        );
    }
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-stream-end"))
    {
        return chunked_json_response_with_delayed_final_chunk(
            response_payload,
            Duration::from_millis(400),
        );
    }

    (StatusCode::OK, Json(response_payload)).into_response()
}

use super::*;
