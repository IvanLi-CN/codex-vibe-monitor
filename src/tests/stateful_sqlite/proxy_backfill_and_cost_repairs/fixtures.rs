use super::*;

pub(crate) struct ProxyCostRowSeed<'a> {
    pub(crate) invoke_id: &'a str,
    pub(crate) status: &'a str,
    pub(crate) model: &'a str,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cost: f64,
    pub(crate) price_version: &'a str,
    pub(crate) payload: &'a str,
    pub(crate) raw_response: &'a str,
}

pub(crate) async fn insert_proxy_cost_row(pool: &SqlitePool, seed: ProxyCostRowSeed<'_>) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
    )
    .bind(seed.invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind(seed.status)
    .bind(seed.model)
    .bind(seed.input_tokens)
    .bind(seed.output_tokens)
    .bind(seed.input_tokens + seed.output_tokens)
    .bind(seed.cost)
    .bind(1_i64)
    .bind(seed.price_version)
    .bind(seed.payload)
    .bind(seed.raw_response)
    .execute(pool)
    .await
    .expect("insert proxy cost row");
}

pub(crate) struct UpstreamAccountSeed<'a> {
    pub(crate) id: i64,
    pub(crate) display_name: &'a str,
    pub(crate) upstream_base_url: &'a str,
    pub(crate) created_at: &'a str,
}

pub(crate) async fn insert_api_key_upstream_account(
    pool: &SqlitePool,
    seed: UpstreamAccountSeed<'_>,
) {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, upstream_base_url, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(seed.id)
    .bind("api_key_codex")
    .bind("codex")
    .bind(seed.display_name)
    .bind(seed.upstream_base_url)
    .bind("active")
    .bind(1_i64)
    .bind(seed.created_at)
    .bind(seed.created_at)
    .execute(pool)
    .await
    .expect("insert api key upstream account");
}

pub(crate) fn unit_cost_backfill_catalog() -> PricingCatalog {
    PricingCatalog {
        version: "unit-cost-backfill".to_string(),
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
    }
}

pub(crate) fn openai_standard_backfill_catalog() -> PricingCatalog {
    PricingCatalog {
        version: "openai-standard-2026-02-23".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
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
    }
}
