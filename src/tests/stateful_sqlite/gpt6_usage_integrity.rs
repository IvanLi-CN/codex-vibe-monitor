use super::*;

#[test]
fn websocket_usage_event_accepts_cache_read_only_terminal_usage() {
    let event = parse_ws_usage_event(
        r#"{"type":"response.completed","response":{"id":"resp_cache_read_only","model":"gpt-6-sol","status":"completed","usage":{"input_tokens_details":{"cached_tokens":325}}}}"#,
    );

    assert!(event.is_some());
}

#[test]
fn parse_stream_response_payload_cache_read_only_update_preserves_prior_usage() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-6-sol","status":"in_progress","usage":{"input_tokens":1200,"output_tokens":40,"total_tokens":1240,"input_tokens_details":{"cached_tokens":300,"cache_write_tokens":50},"output_tokens_details":{"reasoning_tokens":10}}}}"#,
        "event: response.in_progress",
        r#"data: {"type":"response.in_progress","response":{"id":"resp_test","model":"gpt-6-sol","status":"in_progress","usage":{"input_tokens_details":{"cached_tokens":325}}}}"#,
    ]
    .join("\n");

    let parsed = parse_stream_response_payload(raw.as_bytes());

    assert_eq!(parsed.usage.input_tokens, Some(1_200));
    assert_eq!(parsed.usage.output_tokens, Some(40));
    assert_eq!(parsed.usage.total_tokens, Some(1_240));
    assert_eq!(parsed.usage.cache_input_tokens, Some(325));
    assert_eq!(parsed.usage.reasoning_tokens, Some(10));
    assert_eq!(parsed.usage.reported_cache_write_tokens, Some(50));
}

#[test]
fn websocket_usage_updates_accumulate_without_clearing_reported_details() {
    let initial = parse_usage_value(&serde_json::json!({
        "input_tokens": 1200,
        "output_tokens": 40,
        "total_tokens": 1240,
        "input_tokens_details": {
            "cached_tokens": 300,
            "cache_write_tokens": 50
        },
        "output_tokens_details": {"reasoning_tokens": 10}
    }));
    let cache_read_only = parse_usage_value(&serde_json::json!({
        "input_tokens_details": {"cached_tokens": 325}
    }));
    let mut usage = WebSocketUsageAccumulator::default();
    usage.update(initial);
    let accumulated = usage.update(cache_read_only);

    assert_eq!(accumulated.input_tokens, Some(1_200));
    assert_eq!(accumulated.output_tokens, Some(40));
    assert_eq!(accumulated.total_tokens, Some(1_240));
    assert_eq!(accumulated.cache_input_tokens, Some(325));
    assert_eq!(accumulated.reported_cache_write_tokens, Some(50));
    assert_eq!(accumulated.reasoning_tokens, Some(10));

    let terminal = parse_usage_value(&serde_json::json!({
        "input_tokens": 1300,
        "output_tokens": 100,
        "total_tokens": 1400
    }));
    usage.update(terminal);
    usage.update(ParsedUsage::default());
    let interrupted = usage.snapshot();

    assert_eq!(interrupted.input_tokens, Some(1_300));
    assert_eq!(interrupted.output_tokens, Some(100));
    assert_eq!(interrupted.total_tokens, Some(1_400));
    assert_eq!(interrupted.cache_input_tokens, Some(325));
    assert_eq!(interrupted.reported_cache_write_tokens, Some(50));
    assert_eq!(interrupted.reasoning_tokens, Some(10));
}

#[test]
fn estimate_gpt_6_returns_unknown_cost_when_cache_read_exceeds_input_without_exact_cache_write() {
    let catalog = default_pricing_catalog();
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        cache_input_tokens: Some(1_100),
        reported_cache_write_tokens: None,
        reasoning_tokens: Some(0),
        total_tokens: Some(1_100),
    };
    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-6-astra"),
        &usage,
        None,
        ProxyPricingMode::ResponseTier,
    );

    assert!(cost.is_none());
    assert!(!estimated);
}
