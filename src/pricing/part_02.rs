#[derive(Clone, Copy)]
struct DefaultModelPricing {
    input_per_1m: f64,
    output_per_1m: f64,
    cache_input_per_1m: Option<f64>,
    cache_read_per_1m: Option<f64>,
    cache_write_per_1m: Option<f64>,
    reasoning_per_1m: Option<f64>,
    source: &'static str,
}

const DEFAULT_PRICING_MODELS: &[(&str, DefaultModelPricing)] = &[
    (
        "gpt-5.3-codex",
        DefaultModelPricing {
            input_per_1m: 1.75,
            output_per_1m: 14.0,
            cache_input_per_1m: Some(0.175),
            cache_read_per_1m: Some(0.175),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.2-codex",
        DefaultModelPricing {
            input_per_1m: 1.75,
            output_per_1m: 14.0,
            cache_input_per_1m: Some(0.175),
            cache_read_per_1m: Some(0.175),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.1-codex-max",
        DefaultModelPricing {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
            cache_input_per_1m: Some(0.125),
            cache_read_per_1m: Some(0.125),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.1-codex-mini",
        DefaultModelPricing {
            input_per_1m: 0.25,
            output_per_1m: 2.0,
            cache_input_per_1m: Some(0.025),
            cache_read_per_1m: Some(0.025),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.2",
        DefaultModelPricing {
            input_per_1m: 1.75,
            output_per_1m: 14.0,
            cache_input_per_1m: Some(0.175),
            cache_read_per_1m: Some(0.175),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.6-sol",
        DefaultModelPricing {
            input_per_1m: 5.0,
            output_per_1m: 30.0,
            cache_input_per_1m: Some(0.5),
            cache_read_per_1m: Some(0.5),
            cache_write_per_1m: Some(6.25),
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.6-terra",
        DefaultModelPricing {
            input_per_1m: 2.0,
            output_per_1m: 12.0,
            cache_input_per_1m: Some(0.20),
            cache_read_per_1m: Some(0.20),
            cache_write_per_1m: Some(2.5),
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.6-luna",
        DefaultModelPricing {
            input_per_1m: 0.20,
            output_per_1m: 1.20,
            cache_input_per_1m: Some(0.02),
            cache_read_per_1m: Some(0.02),
            cache_write_per_1m: Some(0.25),
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.4",
        DefaultModelPricing {
            input_per_1m: 2.5,
            output_per_1m: 15.0,
            cache_input_per_1m: Some(0.25),
            cache_read_per_1m: Some(0.25),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.4-mini",
        DefaultModelPricing {
            input_per_1m: 0.75,
            output_per_1m: 4.5,
            cache_input_per_1m: Some(0.075),
            cache_read_per_1m: Some(0.075),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.5",
        DefaultModelPricing {
            input_per_1m: 5.0,
            output_per_1m: 30.0,
            cache_input_per_1m: Some(0.5),
            cache_read_per_1m: Some(0.5),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5",
        DefaultModelPricing {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
            cache_input_per_1m: Some(0.125),
            cache_read_per_1m: Some(0.125),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5-mini",
        DefaultModelPricing {
            input_per_1m: 0.25,
            output_per_1m: 2.0,
            cache_input_per_1m: Some(0.025),
            cache_read_per_1m: Some(0.025),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5-nano",
        DefaultModelPricing {
            input_per_1m: 0.05,
            output_per_1m: 0.4,
            cache_input_per_1m: Some(0.005),
            cache_read_per_1m: Some(0.005),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.2-chat-latest",
        DefaultModelPricing {
            input_per_1m: 1.75,
            output_per_1m: 14.0,
            cache_input_per_1m: Some(0.175),
            cache_read_per_1m: Some(0.175),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.1-chat-latest",
        DefaultModelPricing {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
            cache_input_per_1m: Some(0.125),
            cache_read_per_1m: Some(0.125),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5-chat-latest",
        DefaultModelPricing {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
            cache_input_per_1m: Some(0.125),
            cache_read_per_1m: Some(0.125),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.1-codex",
        DefaultModelPricing {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
            cache_input_per_1m: Some(0.125),
            cache_read_per_1m: Some(0.125),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5-codex",
        DefaultModelPricing {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
            cache_input_per_1m: Some(0.125),
            cache_read_per_1m: Some(0.125),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.2-pro",
        DefaultModelPricing {
            input_per_1m: 21.0,
            output_per_1m: 168.0,
            cache_input_per_1m: None,
            cache_read_per_1m: None,
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.4-pro",
        DefaultModelPricing {
            input_per_1m: 30.0,
            output_per_1m: 180.0,
            cache_input_per_1m: None,
            cache_read_per_1m: None,
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5.5-pro",
        DefaultModelPricing {
            input_per_1m: 30.0,
            output_per_1m: 180.0,
            cache_input_per_1m: None,
            cache_read_per_1m: None,
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
    (
        "gpt-5-pro",
        DefaultModelPricing {
            input_per_1m: 15.0,
            output_per_1m: 120.0,
            cache_input_per_1m: None,
            cache_read_per_1m: None,
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official",
        },
    ),
];

pub(crate) fn default_pricing_catalog() -> PricingCatalog {
    let models = DEFAULT_PRICING_MODELS
        .iter()
        .map(|(model, pricing)| {
            (
                (*model).to_string(),
                ModelPricing {
                    input_per_1m: pricing.input_per_1m,
                    output_per_1m: pricing.output_per_1m,
                    cache_input_per_1m: pricing.cache_input_per_1m,
                    cache_read_per_1m: pricing.cache_read_per_1m,
                    cache_write_per_1m: pricing.cache_write_per_1m,
                    reasoning_per_1m: pricing.reasoning_per_1m,
                    source: pricing.source.to_string(),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    PricingCatalog {
        version: DEFAULT_PRICING_CATALOG_VERSION.to_string(),
        models,
    }
}
