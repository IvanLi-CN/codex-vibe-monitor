use super::*;

#[test]
fn decode_response_payload_for_usage_decompresses_gzip_stream() {
    let raw = [
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":123,\"output_tokens\":45,\"total_tokens\":168,\"input_tokens_details\":{\"cached_tokens\":7},\"output_tokens_details\":{\"reasoning_tokens\":4}}}}",
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let (decoded, decode_error) = decode_response_payload_for_usage(&compressed, Some("gzip"));
    assert!(decode_error.is_none());

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, decoded.as_ref(), true, None);
    assert_eq!(parsed.usage.input_tokens, Some(123));
    assert_eq!(parsed.usage.output_tokens, Some(45));
    assert_eq!(parsed.usage.total_tokens, Some(168));
    assert_eq!(parsed.usage.cache_input_tokens, Some(7));
    assert_eq!(parsed.usage.reasoning_tokens, Some(4));
}

fn encode_brotli_payload(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    {
        let mut writer = CompressorWriter::new(&mut output, 4096, 5, 22);
        writer.write_all(bytes).expect("write brotli payload");
    }
    output
}

#[test]
fn decode_response_payload_for_usage_decompresses_brotli_stream() {
    let raw = br#"{"usage":{"input_tokens":9,"output_tokens":4,"total_tokens":13}}"#;
    let compressed = encode_brotli_payload(raw);

    let (decoded, decode_error) = decode_response_payload_for_usage(&compressed, Some("br"));
    assert!(decode_error.is_none());
    assert_eq!(decoded.as_ref(), raw);
}

#[test]
fn decode_response_payload_for_usage_decompresses_deflate_streams() {
    let raw = br#"{"usage":{"input_tokens":11,"output_tokens":5,"total_tokens":16}}"#;

    let mut zlib_encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    zlib_encoder.write_all(raw).expect("write zlib payload");
    let zlib_compressed = zlib_encoder.finish().expect("finish zlib payload");

    let (decoded_zlib, decode_error_zlib) =
        decode_response_payload_for_usage(&zlib_compressed, Some("deflate"));
    assert!(decode_error_zlib.is_none());
    assert_eq!(decoded_zlib.as_ref(), raw);

    let mut raw_encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    raw_encoder
        .write_all(raw)
        .expect("write raw deflate payload");
    let raw_compressed = raw_encoder.finish().expect("finish raw deflate payload");

    let (decoded_raw, decode_error_raw) =
        decode_response_payload_for_usage(&raw_compressed, Some("deflate"));
    assert!(decode_error_raw.is_none());
    assert_eq!(decoded_raw.as_ref(), raw);
}

#[test]
fn decode_response_payload_for_usage_decompresses_stacked_content_encodings() {
    let raw = br#"{"usage":{"input_tokens":21,"output_tokens":8,"total_tokens":29}}"#;
    let mut gzip_encoder = GzEncoder::new(Vec::new(), Compression::default());
    gzip_encoder
        .write_all(raw)
        .expect("write stacked gzip payload");
    let gzip_compressed = gzip_encoder.finish().expect("finish stacked gzip payload");
    let stacked = encode_brotli_payload(&gzip_compressed);

    let (decoded, decode_error) = decode_response_payload_for_usage(&stacked, Some("gzip, br"));
    assert!(decode_error.is_none());
    assert_eq!(decoded.as_ref(), raw);
}
