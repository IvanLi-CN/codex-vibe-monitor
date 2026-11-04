use super::*;
use serde_json::json;

#[test]
fn parse_stream_response_payload_extracts_usage_and_model() {
    let raw = [
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}],\"usage\":null}",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[],\"service_tier\":\"priority\",\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":18}}",
        "data: [DONE]",
    ]
    .join("\n");
    let parsed = parse_stream_response_payload(raw.as_bytes());
    assert_eq!(parsed.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(parsed.usage.input_tokens, Some(11));
    assert_eq!(parsed.usage.output_tokens, Some(7));
    assert_eq!(parsed.usage.total_tokens, Some(18));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_usage_value_preserves_reported_cache_write_tokens() {
    let usage = parse_usage_value(&serde_json::json!({
        "input_tokens": 1000,
        "output_tokens": 200,
        "total_tokens": 1200,
        "input_tokens_details": {
            "cached_tokens": 300,
            "cache_write_tokens": 250
        }
    }));

    assert_eq!(usage.cache_input_tokens, Some(300));
    assert_eq!(usage.reported_cache_write_tokens, Some(250));
}

#[test]
fn parse_stream_response_payload_preserves_exact_cache_write_from_prior_usage_event() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-6-sol","status":"in_progress","usage":{"input_tokens":1000,"output_tokens":0,"total_tokens":1000,"input_tokens_details":{"cached_tokens":300,"cache_write_tokens":250}}}}"#,
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"id":"resp_test","model":"gpt-6-sol","status":"completed","usage":{"input_tokens":1000,"output_tokens":100,"total_tokens":1100,"input_tokens_details":{"cached_tokens":300}}}}"#,
    ]
    .join("\n");

    let parsed = parse_stream_response_payload(raw.as_bytes());

    assert_eq!(parsed.usage.input_tokens, Some(1_000));
    assert_eq!(parsed.usage.output_tokens, Some(100));
    assert_eq!(parsed.usage.reported_cache_write_tokens, Some(250));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_stream_response_payload_cache_detail_update_preserves_prior_usage() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-6-sol","status":"in_progress","usage":{"input_tokens":1200,"output_tokens":40,"total_tokens":1240,"input_tokens_details":{"cached_tokens":300,"cache_write_tokens":50},"output_tokens_details":{"reasoning_tokens":10}}}}"#,
        "event: response.in_progress",
        r#"data: {"type":"response.in_progress","response":{"id":"resp_test","model":"gpt-6-sol","status":"in_progress","usage":{"total_tokens":1240,"input_tokens_details":{"cached_tokens":325,"cache_write_tokens":75}}}}"#,
    ]
    .join("\n");

    let parsed = parse_stream_response_payload(raw.as_bytes());

    assert_eq!(parsed.usage.input_tokens, Some(1_200));
    assert_eq!(parsed.usage.output_tokens, Some(40));
    assert_eq!(parsed.usage.total_tokens, Some(1_240));
    assert_eq!(parsed.usage.cache_input_tokens, Some(325));
    assert_eq!(parsed.usage.reasoning_tokens, Some(10));
    assert_eq!(parsed.usage.reported_cache_write_tokens, Some(75));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_stream_response_payload_accepts_later_exact_cache_write_zero() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-6-sol","status":"in_progress","usage":{"input_tokens":1000,"output_tokens":0,"total_tokens":1000,"input_tokens_details":{"cached_tokens":300,"cache_write_tokens":250}}}}"#,
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"id":"resp_test","model":"gpt-6-sol","status":"completed","usage":{"input_tokens":1000,"output_tokens":100,"total_tokens":1100,"input_tokens_details":{"cached_tokens":300,"cache_write_tokens":0}}}}"#,
    ]
    .join("\n");

    let parsed = parse_stream_response_payload(raw.as_bytes());

    assert_eq!(parsed.usage.reported_cache_write_tokens, Some(0));
}

#[test]
fn parse_stream_response_payload_extracts_terminal_failure_details() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "event: response.failed",
        r#"data: {"type":"response.failed","response":{"id":"resp_test","model":"gpt-5.4","status":"failed","error":{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}}"#,
    ]
    .join("\n");

    let parsed = parse_stream_response_payload(raw.as_bytes());
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(
        parsed.stream_terminal_event.as_deref(),
        Some("response.failed")
    );
    assert_eq!(parsed.upstream_error_code.as_deref(), Some("server_error"));
    assert!(
        parsed.upstream_error_message.as_deref().is_some_and(
            |message| message.contains("request ID 060a328d-5cb6-433c-9025-1da2d9c632f1")
        )
    );
    assert_eq!(
        parsed.upstream_request_id.as_deref(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );
    assert_eq!(
        parsed.usage_missing_reason.as_deref(),
        Some("upstream_response_failed")
    );
}

#[test]
fn stream_response_parser_recognizes_only_strict_successful_completion() {
    let mut parser = StreamResponsePayloadChunkParser::default();
    parser.ingest_bytes(b"event: response.completed\n");
    assert!(!parser.successful_terminal_seen());
    parser.ingest_bytes(
        b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n",
    );
    assert!(parser.successful_terminal_seen());
    assert!(parser.finish().successful_terminal_seen);

    let mut eof_parser = StreamResponsePayloadChunkParser::default();
    eof_parser.ingest_bytes(b"event: response.completed\n");
    eof_parser.ingest_bytes(
        b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}",
    );
    assert!(eof_parser.successful_terminal_seen());

    for payload in [
        [
            "event: response.completed",
            r#"data: {"type":"response.completed","response":{"status":"failed"}}"#,
        ]
        .join("\n"),
        [
            "event: response.completed",
            r#"data: {"type":"response.failed","response":{"status":"completed"}}"#,
        ]
        .join("\n"),
        [
            "event: response.completed",
            r#"data: {"type":"response.completed","response":{}}"#,
        ]
        .join("\n"),
        [
            "event: response.completed",
            r#"data: {"type":"response.completed","response":{"status":"completed"}"#,
        ]
        .join("\n"),
        [
            "event: response.completed",
            r#"data: {"response":{"status":"completed"}}"#,
        ]
        .join("\n"),
        r#"data: {"type":"response.completed","response":{"status":"completed"}}"#.to_string(),
    ] {
        let mut parser = StreamResponsePayloadChunkParser::default();
        parser.ingest_bytes(payload.as_bytes());
        parser.flush_pending_line();
        assert!(
            !parser.successful_terminal_seen(),
            "invalid terminal payload must not establish success: {payload}"
        );
    }
}

#[test]
fn estimate_proxy_cost_subtracts_cached_tokens_from_base_input_rate() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-test".to_string(),
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: Some(0.5),
                cache_read_per_1m: Some(0.5),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, price_version) = estimate_proxy_cost(
        &catalog,
        Some("gpt-test"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((600.0 * 1.0) + (200.0 * 2.0) + (400.0 * 0.5)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
    assert_eq!(price_version.as_deref(), Some("unit-test@response-tier"));
}

#[test]
fn estimate_proxy_cost_keeps_full_input_when_cache_price_missing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-test".to_string(),
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-test"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((1_000.0 * 1.0) + (200.0 * 2.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_breakdown_uses_explicit_gpt_5_6_sol_cache_read_and_write_prices() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.6-sol".to_string(),
            ModelPricing {
                input_per_1m: 5.0,
                output_per_1m: 30.0,
                cache_input_per_1m: Some(0.5),
                cache_read_per_1m: Some(0.5),
                cache_write_per_1m: Some(6.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (breakdown, estimated, price_version) = estimate_proxy_cost_breakdown(
        &catalog,
        Some("gpt-5.6-sol"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((600.0 * 6.25) + (400.0 * 0.5) + (200.0 * 30.0)) / 1_000_000.0;
    let computed = breakdown.expect("cost breakdown should be present");
    assert_eq!(computed.input, 0.0);
    assert!((computed.cache_write - (600.0 * 6.25 / 1_000_000.0)).abs() < 1e-12);
    assert!((computed.cache_read - (400.0 * 0.5 / 1_000_000.0)).abs() < 1e-12);
    assert!((computed.output - (200.0 * 30.0 / 1_000_000.0)).abs() < 1e-12);
    assert_eq!(computed.reasoning, 0.0);
    assert!((computed.total() - expected).abs() < 1e-12);
    assert!(estimated);
    assert_eq!(price_version.as_deref(), Some("unit-test@response-tier"));
}

#[test]
fn estimate_proxy_cost_falls_back_to_dated_gpt_5_6_terra_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.6-terra".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 12.0,
                cache_input_per_1m: Some(0.20),
                cache_read_per_1m: Some(0.20),
                cache_write_per_1m: Some(2.5),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.6-terra-2026-07-08"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((600.0 * 2.5) + (400.0 * 0.20) + (200.0 * 12.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_falls_back_to_dated_gpt_5_6_luna_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.6-luna".to_string(),
            ModelPricing {
                input_per_1m: 0.20,
                output_per_1m: 1.20,
                cache_input_per_1m: Some(0.02),
                cache_read_per_1m: Some(0.02),
                cache_write_per_1m: Some(0.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.6-luna-2026-07-08"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((600.0 * 0.25) + (400.0 * 0.02) + (200.0 * 1.20)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_uses_dated_gpt_6_default_pricing_presets() {
    let catalog = default_pricing_catalog();
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    for (model, dated_model, expected) in [
        (
            "gpt-6-astra",
            "gpt-6-astra-2026-09-23",
            ((600.0 * 12.5) + (400.0 * 1.0) + (200.0 * 50.0)) / 1_000_000.0,
        ),
        (
            "gpt-6-sol",
            "gpt-6-sol-2026-09-23",
            ((600.0 * 2.5) + (400.0 * 0.2) + (200.0 * 10.0)) / 1_000_000.0,
        ),
        (
            "gpt-6-terra",
            "gpt-6-terra-2026-09-20",
            ((600.0 * 2.5) + (400.0 * 0.20) + (200.0 * 12.0)) / 1_000_000.0,
        ),
        (
            "gpt-6-luna",
            "gpt-6-luna-2026-09-23",
            ((600.0 * 0.125) + (400.0 * 0.01) + (200.0 * 0.5)) / 1_000_000.0,
        ),
    ] {
        assert!(catalog.models.contains_key(model));
        let (cost, estimated, _) = estimate_proxy_cost(
            &catalog,
            Some(dated_model),
            &usage,
            None,
            ProxyPricingMode::ResponseTier,
        );
        assert!(estimated, "{dated_model} should be estimated");
        assert!((cost.expect("dated GPT-6 cost should exist") - expected).abs() < 1e-12);
    }
}

#[test]
fn estimate_gpt_6_uses_exact_cache_write_long_context_and_output_rate_for_reasoning() {
    let catalog = default_pricing_catalog();
    for (model, input_price, read_price, write_price, output_price) in [
        ("gpt-6-astra", 10.0, 1.0, 12.5, 50.0),
        ("gpt-6-sol", 2.0, 0.2, 2.5, 10.0),
        ("gpt-6-luna", 0.1, 0.01, 0.125, 0.5),
    ] {
        let short_usage = ParsedUsage {
            input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS),
            output_tokens: Some(1_200),
            cache_input_tokens: Some(40_000),
            reported_cache_write_tokens: Some(50_000),
            reasoning_tokens: Some(200),
            total_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1_200),
        };
        let (short, estimated, _) = estimate_proxy_cost_breakdown(
            &catalog,
            Some(model),
            &short_usage,
            Some("standard"),
            ProxyPricingMode::ResponseTier,
        );
        let short = short.expect("short-context cost exists");
        let short_expected_input = (182_000.0 * input_price) / 1_000_000.0;
        let short_expected_write = (50_000.0 * write_price) / 1_000_000.0;
        let short_expected_read = (40_000.0 * read_price) / 1_000_000.0;
        let short_expected_output = (1_000.0 * output_price) / 1_000_000.0;
        let short_expected_reasoning = (200.0 * output_price) / 1_000_000.0;
        assert!(estimated);
        assert!((short.input - short_expected_input).abs() < 1e-12);
        assert!((short.cache_write - short_expected_write).abs() < 1e-12);
        assert!((short.cache_read - short_expected_read).abs() < 1e-12);
        assert!((short.output - short_expected_output).abs() < 1e-12);
        assert!((short.reasoning - short_expected_reasoning).abs() < 1e-12);

        let mut long_usage = short_usage.clone();
        long_usage.input_tokens = Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1);
        long_usage.total_tokens = Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1_201);
        let (long, estimated, _) = estimate_proxy_cost_breakdown(
            &catalog,
            Some(model),
            &long_usage,
            Some("standard"),
            ProxyPricingMode::ResponseTier,
        );
        let long = long.expect("long-context cost exists");
        assert!(estimated);
        assert!((long.input - (182_001.0 * input_price * 2.0 / 1_000_000.0)).abs() < 1e-12);
        assert!((long.cache_write - (50_000.0 * write_price * 2.0 / 1_000_000.0)).abs() < 1e-12);
        assert!((long.cache_read - (40_000.0 * read_price * 2.0 / 1_000_000.0)).abs() < 1e-12);
        assert!((long.output - (1_000.0 * output_price * 1.5 / 1_000_000.0)).abs() < 1e-12);
        assert!((long.reasoning - (200.0 * output_price * 1.5 / 1_000_000.0)).abs() < 1e-12);
    }
}

#[test]
fn estimate_explicit_cache_split_uses_exact_cache_write_for_existing_models() {
    let catalog = default_pricing_catalog();

    for (reported_cache_write_tokens, expected_input_tokens, expected_write_tokens) in
        [(300, 500, 300), (0, 800, 0)]
    {
        let usage = ParsedUsage {
            input_tokens: Some(1_000),
            output_tokens: Some(100),
            cache_input_tokens: Some(200),
            reported_cache_write_tokens: Some(reported_cache_write_tokens),
            reasoning_tokens: Some(0),
            total_tokens: Some(1_100),
        };
        let (breakdown, estimated, _) = estimate_proxy_cost_breakdown(
            &catalog,
            Some("gpt-5.6-sol"),
            &usage,
            None,
            ProxyPricingMode::ResponseTier,
        );
        let breakdown = breakdown.expect("exact cache-write cost exists");

        assert!(estimated);
        assert!(
            (breakdown.input - (expected_input_tokens as f64 * 5.0 / 1_000_000.0)).abs() < 1e-12
        );
        assert!(
            (breakdown.cache_write - (expected_write_tokens as f64 * 6.25 / 1_000_000.0)).abs()
                < 1e-12
        );
        assert!((breakdown.cache_read - (200.0 * 0.5 / 1_000_000.0)).abs() < 1e-12);
        assert!((breakdown.output - (100.0 * 30.0 / 1_000_000.0)).abs() < 1e-12);
    }

    let inconsistent_usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        cache_input_tokens: Some(200),
        reported_cache_write_tokens: Some(801),
        reasoning_tokens: Some(0),
        total_tokens: Some(1_100),
    };
    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.6-sol"),
        &inconsistent_usage,
        None,
        ProxyPricingMode::ResponseTier,
    );
    assert!(cost.is_none());
    assert!(!estimated);
}

#[test]
fn estimate_gpt_6_tier_rules_only_apply_to_actual_supported_tiers() {
    let catalog = default_pricing_catalog();
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        cache_input_tokens: Some(100),
        reported_cache_write_tokens: Some(200),
        reasoning_tokens: Some(0),
        total_tokens: Some(1_100),
    };
    let (standard, _, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-6-sol"),
        &usage,
        Some("priority"),
        ProxyPricingMode::RequestedTier,
    );
    let (priority, _, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-6-sol"),
        &usage,
        Some("priority"),
        ProxyPricingMode::ResponseTier,
    );
    assert!(
        (priority.expect("priority cost") - standard.expect("standard cost") * 2.0).abs() < 1e-12
    );

    for tier in ["batch", "flex", "regional", "unknown-tier"] {
        let (cost, estimated, _) = estimate_proxy_cost(
            &catalog,
            Some("gpt-6-sol"),
            &usage,
            Some(tier),
            ProxyPricingMode::ResponseTier,
        );
        assert!(cost.is_none(), "actual {tier} tier remains unpriced");
        assert!(!estimated);
    }
}

#[test]
fn estimate_gpt_6_returns_unknown_cost_for_inconsistent_exact_cache_write_usage() {
    let catalog = default_pricing_catalog();
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(100),
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: Some(601),
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

#[test]
fn estimate_proxy_cost_rejects_invalid_or_preview_gpt_6_variants() {
    let catalog = default_pricing_catalog();
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    for model in [
        "gpt-6-terra-2026-99-99",
        "gpt-6-terra-2026-02-29",
        "gpt-6-terra-preview",
    ] {
        let (cost, estimated, _) = estimate_proxy_cost(
            &catalog,
            Some(model),
            &usage,
            Some("default"),
            ProxyPricingMode::ResponseTier,
        );
        assert!(cost.is_none(), "{model} should remain unpriced");
        assert!(!estimated, "{model} should not be estimated");
    }
}

#[test]
fn estimate_proxy_cost_falls_back_to_dated_model_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1000),
        output_tokens: Some(500),
        cache_input_tokens: None,
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1500),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.2-2025-12-11"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((1000.0 * 2.0) + (500.0 * 3.0)) / 1_000_000.0;
    assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_prefers_exact_model_over_dated_model_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([
            (
                "gpt-5.2".to_string(),
                ModelPricing {
                    input_per_1m: 1.0,
                    output_per_1m: 1.0,
                    cache_input_per_1m: None,
                    cache_read_per_1m: None,
                    cache_write_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
            (
                "gpt-5.2-2025-12-11".to_string(),
                ModelPricing {
                    input_per_1m: 4.0,
                    output_per_1m: 5.0,
                    cache_input_per_1m: None,
                    cache_read_per_1m: None,
                    cache_write_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
        ]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1000),
        output_tokens: Some(1000),
        cache_input_tokens: None,
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(2000),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.2-2025-12-11"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((1000.0 * 4.0) + (1000.0 * 5.0)) / 1_000_000.0;
    assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_at_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((271_000.0 * 2.5) + (1_000.0 * 0.25) + (1_000.0 * 15.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_above_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_to_reasoning_cost() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: None,
                reasoning_per_1m: Some(20.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reported_cache_write_tokens: None,
        reasoning_tokens: Some(2_000),
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let reasoning_part = (2_000.0 * 20.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_above_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4-pro".to_string(),
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(999_999),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4-pro"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = (272_001.0 * 30.0) / 1_000_000.0;
    let output_part = (1_000.0 * 180.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_for_dated_model_suffix() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4-pro".to_string(),
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: Some(90.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(999_999),
        reported_cache_write_tokens: None,
        reasoning_tokens: Some(2_000),
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4-pro-2026-03-01"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = (272_001.0 * 30.0) / 1_000_000.0;
    let output_part = (1_000.0 * 180.0) / 1_000_000.0;
    let reasoning_part = (2_000.0 * 90.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_for_dated_model_suffix() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4-2026-03-01"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_for_other_models() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4o".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: None,
        reported_cache_write_tokens: None,
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4o"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((272_001.0 * 2.5) + (1_000.0 * 15.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_requested_tier_priority_multiplier_and_price_version_suffix() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: None,
                reasoning_per_1m: Some(20.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reported_cache_write_tokens: None,
        reasoning_tokens: Some(50),
        total_tokens: Some(1_200),
    };

    let (cost, estimated, price_version) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("priority"),
        ProxyPricingMode::RequestedTier,
    );

    let base_expected =
        ((600.0 * 2.5) + (400.0 * 0.25) + (200.0 * 15.0) + (50.0 * 20.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - (base_expected * 2.0)).abs() < 1e-12);
    assert!(estimated);
    assert_eq!(price_version.as_deref(), Some("unit-test@requested-tier"));
}

#[test]
fn resolve_proxy_billing_service_tier_and_pricing_mode_prefers_requested_tier_for_api_keys() {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        Some("priority"),
        Some("default"),
        Some("api_key_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("priority"));
    assert_eq!(pricing_mode, ProxyPricingMode::RequestedTier);
}

#[test]
fn resolve_proxy_billing_service_tier_and_pricing_mode_prefers_explicit_billing_metadata() {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        Some("default"),
        Some("priority"),
        Some("priority"),
        Some("api_key_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("default"));
    assert_eq!(pricing_mode, ProxyPricingMode::ExplicitBilling);
}

#[test]
fn resolve_proxy_billing_service_tier_and_pricing_mode_keeps_response_tier_for_non_api_keys() {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        Some("priority"),
        Some("default"),
        Some("oauth_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("default"));
    assert_eq!(pricing_mode, ProxyPricingMode::ResponseTier);
}

#[test]
fn resolve_proxy_billing_service_tier_and_pricing_mode_falls_back_to_response_tier_when_api_keys_request_is_missing()
 {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        None,
        Some("default"),
        Some("api_key_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("default"));
    assert_eq!(pricing_mode, ProxyPricingMode::ResponseTier);
}

#[test]
fn parse_target_response_payload_decodes_gzip_stream_usage() {
    let raw = [
        "event: response.created",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
        "",
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        "",
    ]
    .join("\n");

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        &compressed,
        true,
        Some("gzip"),
    );
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.input_tokens, Some(12));
    assert_eq!(parsed.usage.output_tokens, Some(3));
    assert_eq!(parsed.usage.cache_input_tokens, Some(2));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_target_response_payload_decodes_multi_value_content_encoding() {
    let raw = [
        "event: response.created",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
        "",
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"flex\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        "",
    ]
    .join("\n");

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        &compressed,
        true,
        Some("identity, gzip"),
    );
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.input_tokens, Some(12));
    assert_eq!(parsed.usage.output_tokens, Some(3));
    assert_eq!(parsed.usage.cache_input_tokens, Some(2));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert_eq!(parsed.service_tier.as_deref(), Some("flex"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_target_response_payload_detects_sse_without_request_stream_hint() {
    let raw = [
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","service_tier":"priority","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), false, None);

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_target_response_payload_prefers_terminal_stream_service_tier_over_initial_auto() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"auto"}}"#,
        "",
        "event: response.in_progress",
        r#"data: {"type":"response.in_progress","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"auto"}}"#,
        "",
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.4","status":"completed","service_tier":"default","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), true, None);

    assert_eq!(parsed.service_tier.as_deref(), Some("default"));
    assert_eq!(parsed.usage.total_tokens, Some(15));
}

#[test]
fn parse_target_response_payload_does_not_downgrade_same_rank_stream_tier_to_auto() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"default"}}"#,
        "",
        "event: response.in_progress",
        r#"data: {"type":"response.in_progress","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"auto"}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), true, None);

    assert_eq!(parsed.service_tier.as_deref(), Some("default"));
}

#[test]
fn parse_target_response_payload_from_raw_file_falls_back_to_raw_deflate_streams() {
    let raw = [
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","usage":{"input_tokens":17,"output_tokens":4,"total_tokens":21}}}"#,
        "",
    ]
    .join("\n");

    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write raw deflate stream");
    let compressed = encoder.finish().expect("finish raw deflate stream");

    let temp_dir = make_temp_test_dir("raw-deflate-response");
    let raw_path = temp_dir.join("response.bin");
    fs::write(&raw_path, compressed).expect("write raw deflate response payload");

    let parsed = parse_target_response_payload_from_raw_file(
        ProxyCaptureTarget::Responses,
        &raw_path,
        true,
        Some("deflate"),
    )
    .expect("parse raw deflate response payload");

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.total_tokens, Some(21));
    assert!(parsed.usage_missing_reason.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn parse_target_response_payload_reads_service_tier_from_response_object() {
    let raw = json!({
        "id": "resp_json_1",
        "response": {
            "model": "gpt-5.3-codex",
            "service_tier": "priority",
            "usage": {
                "input_tokens": 21,
                "output_tokens": 5,
                "total_tokens": 26
            }
        }
    });

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        serde_json::to_string(&raw)
            .expect("serialize raw payload")
            .as_bytes(),
        false,
        None,
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert_eq!(parsed.usage.total_tokens, Some(26));
}

#[test]
fn standalone_search_response_does_not_infer_usage_from_results() {
    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::StandaloneSearch,
        br#"{
            "model": "gpt-5.4",
            "results": [{"title": "result", "content": "hello"}]
        }"#,
        false,
        None,
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.4"));
    assert!(parsed.usage.input_tokens.is_none());
    assert!(parsed.usage.output_tokens.is_none());
    assert!(parsed.usage.total_tokens.is_none());
    assert_eq!(
        parsed.usage_missing_reason.as_deref(),
        Some("usage_missing_in_response")
    );

    let sse_like = parse_target_response_payload(
        ProxyCaptureTarget::StandaloneSearch,
        b"event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n",
        false,
        None,
    );
    assert!(sse_like.stream_terminal_event.is_none());
    assert_eq!(
        sse_like.usage_missing_reason.as_deref(),
        Some("response_not_json")
    );

    let mut search_stream_parser =
        StreamResponsePayloadChunkParser::for_target(ProxyCaptureTarget::StandaloneSearch);
    search_stream_parser.ingest_bytes(
        b"event: response.failed\ndata: {\"type\":\"response.failed\",\"object\":\"response.compaction\"}\n\n",
    );
    let search_stream_outcome = search_stream_parser.finish();
    assert!(!search_stream_outcome.saw_stream_fields);
    assert!(!search_stream_outcome.successful_terminal_seen);
    assert!(
        search_stream_outcome
            .response_info
            .compaction_response_kind
            .is_none()
    );
    assert!(
        search_stream_outcome
            .response_info
            .stream_terminal_event
            .is_none()
    );

    let forced_stream_hint = parse_target_response_payload(
        ProxyCaptureTarget::StandaloneSearch,
        br#"{"object":"response.compaction","results":[]}"#,
        true,
        None,
    );
    assert!(forced_stream_hint.compaction_response_kind.is_none());
    assert!(forced_stream_hint.stream_terminal_event.is_none());
}

#[test]
fn parse_target_response_payload_detects_remote_v2_compaction_stream_events() {
    let raw = [
        "event: response.output_item.added",
        r#"data: {"type":"response.output_item.added","item":{"id":"cmp_001","type":"compaction","encrypted_content":"enc"}} "#,
        "",
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), true, None);

    assert_eq!(
        parsed
            .compaction_response_kind
            .map(CompactionKind::as_payload_str),
        Some("remote_v2")
    );
}

#[test]
fn parse_target_response_payload_detects_response_compaction_json_shape() {
    let raw = json!({
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
            "output_tokens": 438,
            "total_tokens": 577
        }
    });

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        serde_json::to_string(&raw)
            .expect("serialize raw payload")
            .as_bytes(),
        false,
        None,
    );

    assert_eq!(
        parsed
            .compaction_response_kind
            .map(CompactionKind::as_payload_str),
        Some("remote_v2")
    );
}

#[test]
fn compact_responses_keep_legacy_compaction_kind_even_with_compaction_payload_shape() {
    let raw = json!({
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
            "output_tokens": 438,
            "total_tokens": 577
        }
    });

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::ResponsesCompact,
        serde_json::to_string(&raw)
            .expect("serialize raw payload")
            .as_bytes(),
        false,
        None,
    );

    assert_eq!(
        parsed
            .compaction_response_kind
            .map(CompactionKind::as_payload_str),
        Some("remote_v2")
    );

    assert_eq!(
        resolve_compaction_response_kind_for_payload(
            ProxyCaptureTarget::ResponsesCompact,
            parsed.compaction_response_kind
        )
        .map(CompactionKind::as_payload_str),
        Some("compact")
    );
}

#[test]
fn parse_target_response_payload_records_decode_failure_reason() {
    let raw = [
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2,\"total_tokens\":12}}}",
        "data: [DONE]",
    ]
    .join("\n");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        raw.as_bytes(),
        true,
        Some("gzip"),
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.total_tokens, Some(12));
    assert!(
        parsed
            .usage_missing_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("response_decode_failed:gzip:"))
    );
}
