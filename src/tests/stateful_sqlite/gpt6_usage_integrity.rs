use super::*;

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
