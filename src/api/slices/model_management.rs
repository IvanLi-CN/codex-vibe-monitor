use super::*;
use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::sync::Arc;
use tracing::warn;

const LITELLM_PRICE_CATALOG_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const LITELLM_MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelsDevSyncProvider {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) doc_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelsDevPriceCandidate {
    pub(crate) model: String,
    pub(crate) name: String,
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) doc_url: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) deprecation_date: Option<String>,
    pub(crate) input_per_1m: Option<f64>,
    pub(crate) output_per_1m: Option<f64>,
    pub(crate) cache_read_per_1m: Option<f64>,
    pub(crate) cache_write_per_1m: Option<f64>,
    pub(crate) reasoning_per_1m: Option<f64>,
    pub(crate) unsupported_dimensions: Vec<String>,
    pub(crate) importable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelsDevSyncPreview {
    pub(crate) fetched_at: String,
    pub(crate) source_url: String,
    pub(crate) source_version: String,
    pub(crate) provider_count: usize,
    pub(crate) candidate_count: usize,
    pub(crate) providers: Vec<ModelsDevSyncProvider>,
    pub(crate) candidates: Vec<ModelsDevPriceCandidate>,
    pub(crate) quotes: Vec<PricingQuote>,
    pub(crate) sync_state: crate::models_dev_sync_memory::ModelsDevSyncMemoryState,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelsDevSyncMemoryPatch {
    #[serde(default)]
    pub(crate) provider_selections: Vec<crate::models_dev_sync_memory::ProviderSelectionChange>,
    #[serde(default)]
    pub(crate) model_selections: Vec<crate::models_dev_sync_memory::ModelSelectionChange>,
    #[serde(default)]
    pub(crate) quote_provider_choices:
        Vec<crate::models_dev_sync_memory::QuoteProviderChoiceChange>,
    #[serde(default)]
    pub(crate) viewed_model_ids: Vec<String>,
}

pub(crate) async fn get_models_sync_state(
    State(state): State<Arc<AppState>>,
) -> Result<Json<crate::models_dev_sync_memory::ModelsDevSyncMemoryState>, (StatusCode, String)> {
    crate::models_dev_sync_memory::load_models_dev_sync_memory(&state.pool)
        .await
        .map(Json)
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}

pub(crate) async fn patch_models_sync_state(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ModelsDevSyncMemoryPatch>,
) -> Result<Json<crate::models_dev_sync_memory::ModelsDevSyncMemoryState>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    crate::models_dev_sync_memory::validate_patch_keys(&payload)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    crate::models_dev_sync_memory::patch_models_dev_sync_memory(&state.pool, payload)
        .await
        .map(Json)
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelsDevSyncApplyRequest {
    #[serde(default)]
    pub(crate) entries: Vec<PricingEntry>,
    #[serde(default)]
    pub(crate) quotes: Vec<PricingQuote>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedModelPresetRequest {
    pub(crate) model: String,
    pub(crate) enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedModelDeleteResponse {
    pub(crate) deleted_model: String,
}

pub(crate) async fn post_models_sync_preview(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ModelsDevSyncPreview>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let response = state
        .http_clients
        .price_catalog
        .get(LITELLM_PRICE_CATALOG_URL)
        .send()
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                format!("LiteLLM price catalog retrieval failed: {err}"),
            )
        })?;
    if !response.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("LiteLLM price catalog returned HTTP {}", response.status()),
        ));
    }
    let content_length = response.content_length();
    if content_length.is_some_and(|length| length > LITELLM_MAX_RESPONSE_BYTES as u64) {
        return Err((
            StatusCode::BAD_GATEWAY,
            "LiteLLM price catalog response exceeds the 32 MiB limit".to_string(),
        ));
    }
    let mut body = Vec::with_capacity(
        content_length
            .unwrap_or_default()
            .min(LITELLM_MAX_RESPONSE_BYTES as u64) as usize,
    );
    let mut response = response;
    while let Some(chunk) = response.chunk().await.map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("LiteLLM price catalog response read failed: {err}"),
        )
    })? {
        if !append_limited_response_chunk(&mut body, &chunk, LITELLM_MAX_RESPONSE_BYTES) {
            return Err((
                StatusCode::BAD_GATEWAY,
                "LiteLLM price catalog response exceeds the 32 MiB limit".to_string(),
            ));
        }
    }
    let payload: Value = serde_json::from_slice(&body).map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("LiteLLM price catalog response JSON is invalid: {err}"),
        )
    })?;
    let source_version = format!("sha256:{:x}", Sha256::digest(&body));
    let mut preview = parse_litellm_catalog(&payload, &source_version).map_err(|err| {
        warn!(error = %err, "failed to normalize LiteLLM price catalog");
        (
            StatusCode::BAD_GATEWAY,
            "LiteLLM price catalog format is invalid".to_string(),
        )
    })?;
    preview.sync_state = crate::models_dev_sync_memory::record_catalog_success(
        &state.pool,
        &preview.providers,
        &preview.candidates,
    )
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(preview))
}

fn append_limited_response_chunk(body: &mut Vec<u8>, chunk: &[u8], limit: usize) -> bool {
    let Some(next_len) = body.len().checked_add(chunk.len()) else {
        return false;
    };
    if next_len > limit {
        return false;
    }
    body.extend_from_slice(chunk);
    true
}

pub(crate) async fn post_models_sync_apply(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ModelsDevSyncApplyRequest>,
) -> Result<Json<PricingSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let ModelsDevSyncApplyRequest { entries, quotes } = payload;
    if entries.is_empty() && quotes.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "select at least one model price".to_string(),
        ));
    }
    let (mut normalized, mut normalized_quotes) = PricingSettingsUpdateRequest {
        catalog_version: "litellm".to_string(),
        entries,
        quotes: Some(quotes),
    }
    .normalized_with_quotes()?;
    for pricing in normalized.models.values_mut() {
        pricing.source = "litellm".to_string();
    }
    if let Some(quotes) = normalized_quotes.as_mut() {
        for quote in quotes {
            quote.source = "litellm".to_string();
        }
    }
    let entries = normalized
        .models
        .iter()
        .map(|(model, pricing)| PricingEntry {
            model: model.clone(),
            input_per_1m: pricing.input_per_1m,
            output_per_1m: pricing.output_per_1m,
            cache_input_per_1m: pricing.effective_cache_read_per_1m(),
            cache_read_per_1m: pricing.effective_cache_read_per_1m(),
            cache_write_per_1m: pricing.cache_write_per_1m,
            reasoning_per_1m: pricing.reasoning_per_1m,
            source: "litellm".to_string(),
        })
        .collect::<Vec<_>>();

    let _update_guard = state.pricing_settings_update_lock.lock().await;
    let next = upsert_synced_model_prices(
        &state.pool,
        &entries,
        normalized_quotes.as_deref().unwrap_or_default(),
    )
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    {
        let mut guard = state.pricing_catalog.write().await;
        *guard = next.clone();
    }
    if let Err(err) = wake_startup_backfill_tasks_with_pricing_catalog(
        &state.pool,
        &[StartupBackfillTask::ProxyCost],
        Some(&next),
        "litellm_pricing_sync",
    )
    .await
    {
        warn!(error = %err, "failed to wake ProxyCost backfill after LiteLLM price sync");
    }
    let quotes = load_pricing_quotes(&state.pool)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(
        PricingSettingsResponse::from_catalog(&next).with_quotes(quotes),
    ))
}

pub(crate) async fn put_managed_model_preset(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ManagedModelPresetRequest>,
) -> Result<Json<ProxyModelSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let model = normalize_managed_model_id(payload.model)?;
    let _update_guard = state.proxy_model_settings_update_lock.lock().await;
    let managed_models = load_managed_model_ids(&state.pool)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if !managed_models.iter().any(|candidate| candidate == &model) {
        return Err((StatusCode::NOT_FOUND, "model was not found".to_string()));
    }

    let current = state.proxy_model_settings.read().await.clone();
    let mut enabled = current.enabled_preset_models.clone();
    enabled.retain(|candidate| candidate != &model);
    if payload.enabled {
        enabled.push(model);
    }
    let next = ProxyModelSettings {
        enabled_preset_models: enabled,
        ..current
    }
    .normalized();
    save_proxy_model_settings(&state.pool, next.clone())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    *state.proxy_model_settings.write().await = next.clone();
    Ok(Json(ProxyModelSettingsResponse::from_settings_with_models(
        next,
        managed_models,
    )))
}

pub(crate) async fn delete_managed_model(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ManagedModelDeleteRequest>,
) -> Result<Json<ManagedModelDeleteResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let model = normalize_managed_model_id(payload.model)?;
    let _pricing_guard = state.pricing_settings_update_lock.lock().await;
    let _proxy_guard = state.proxy_model_settings_update_lock.lock().await;
    let current_proxy = state.proxy_model_settings.read().await.clone();
    let mut next_proxy = current_proxy.clone();
    next_proxy
        .enabled_preset_models
        .retain(|candidate| candidate != &model);
    next_proxy = next_proxy.normalized();
    let enabled_json = serde_json::to_string(&next_proxy.enabled_preset_models)
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let price_deleted = sqlx::query("DELETE FROM pricing_settings_models WHERE model = ?1")
        .bind(&model)
        .execute(&mut *tx)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .rows_affected();
    let quote_deleted = sqlx::query("DELETE FROM pricing_model_quotes WHERE model = ?1")
        .bind(&model)
        .execute(&mut *tx)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .rows_affected();
    let model_deleted = sqlx::query("DELETE FROM managed_models WHERE model = ?1")
        .bind(&model)
        .execute(&mut *tx)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .rows_affected();
    let was_enabled = current_proxy
        .enabled_preset_models
        .iter()
        .any(|candidate| candidate == &model);
    if price_deleted == 0 && quote_deleted == 0 && model_deleted == 0 && !was_enabled {
        return Err((StatusCode::NOT_FOUND, "model was not found".to_string()));
    }
    sqlx::query("INSERT OR IGNORE INTO managed_model_suppressions (model) VALUES (?1)")
        .bind(&model)
        .execute(&mut *tx)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    sqlx::query(
        "UPDATE proxy_model_settings SET enabled_preset_models_json = ?1, updated_at = datetime('now') WHERE id = 1",
    )
    .bind(enabled_json)
    .execute(&mut *tx)
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    sqlx::query("UPDATE pricing_settings_meta SET updated_at = datetime('now') WHERE id = 1")
        .execute(&mut *tx)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let next_pricing = load_pricing_catalog_from_connection(&mut tx)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    tx.commit()
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    *state.pricing_catalog.write().await = next_pricing;
    *state.proxy_model_settings.write().await = next_proxy;
    Ok(Json(ManagedModelDeleteResponse {
        deleted_model: model,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedModelDeleteRequest {
    pub(crate) model: String,
}

fn normalize_managed_model_id(model: String) -> Result<String, (StatusCode, String)> {
    let model = model.trim().to_string();
    if model.is_empty() || model.len() > 128 {
        return Err((StatusCode::BAD_REQUEST, "invalid model ID".to_string()));
    }
    Ok(model)
}

fn parse_models_dev_catalog(payload: &Value) -> Result<ModelsDevSyncPreview, String> {
    let provider_map = payload
        .as_object()
        .ok_or_else(|| "provider directory must be an object".to_string())?;
    let mut providers = Vec::new();
    let mut candidates = Vec::new();
    for (provider_id, provider_value) in provider_map {
        let Some(provider) = provider_value.as_object() else {
            return Err(format!("provider {provider_id} must be an object"));
        };
        let provider_name = provider
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(provider_id)
            .to_string();
        let doc_url = provider
            .get("doc")
            .and_then(Value::as_str)
            .and_then(valid_http_url)
            .map(str::to_string);
        let Some(models) = provider.get("models") else {
            continue;
        };
        let Some(models) = models.as_object() else {
            return Err(format!("provider {provider_id} models must be an object"));
        };
        let mut provider_candidates = Vec::new();
        for (model_id, model_value) in models {
            let Some(model) = model_value.as_object() else {
                return Err(format!("model {provider_id}/{model_id} must be an object"));
            };
            let Some(cost) = model.get("cost").filter(|cost| !cost.is_null()) else {
                continue;
            };
            let Some(cost) = cost.as_object() else {
                return Err(format!(
                    "model {provider_id}/{model_id} cost must be an object"
                ));
            };
            let input_per_1m = model_cost(cost, "input", provider_id, model_id)?;
            let output_per_1m = model_cost(cost, "output", provider_id, model_id)?;
            let cache_read_per_1m = model_cost(cost, "cache_read", provider_id, model_id)?;
            let cache_write_per_1m = model_cost(cost, "cache_write", provider_id, model_id)?;
            let reasoning_per_1m = model_cost(cost, "reasoning", provider_id, model_id)?;
            let unsupported_dimensions = cost
                .keys()
                .filter(|key| {
                    !matches!(
                        key.as_str(),
                        "input" | "output" | "cache_read" | "cache_write" | "reasoning"
                    ) && !cost[*key].is_null()
                })
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            provider_candidates.push(ModelsDevPriceCandidate {
                model: model_id.clone(),
                name: model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(model_id)
                    .to_string(),
                provider_id: provider_id.clone(),
                provider_name: provider_name.clone(),
                doc_url: doc_url.clone(),
                status: model
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                deprecation_date: None,
                input_per_1m,
                output_per_1m,
                cache_read_per_1m,
                cache_write_per_1m,
                reasoning_per_1m,
                unsupported_dimensions,
                importable: input_per_1m.is_some() && output_per_1m.is_some(),
            });
        }
        if !provider_candidates.is_empty() {
            providers.push(ModelsDevSyncProvider {
                id: provider_id.clone(),
                name: provider_name,
                doc_url,
            });
            candidates.extend(provider_candidates);
        }
    }
    providers.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    candidates.sort_by(|a, b| {
        a.provider_name
            .cmp(&b.provider_name)
            .then_with(|| a.model.cmp(&b.model))
    });
    let candidate_count = candidates.len();
    Ok(ModelsDevSyncPreview {
        fetched_at: chrono::Utc::now().to_rfc3339(),
        source_url: "legacy".to_string(),
        source_version: "legacy".to_string(),
        provider_count: providers.len(),
        candidate_count,
        providers,
        candidates,
        quotes: Vec::new(),
        sync_state: crate::models_dev_sync_memory::ModelsDevSyncMemoryState::default(),
    })
}

fn parse_litellm_catalog(
    payload: &Value,
    source_version: &str,
) -> Result<ModelsDevSyncPreview, String> {
    let model_map = payload
        .as_object()
        .ok_or_else(|| "LiteLLM catalog must be an object".to_string())?;
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let mut provider_ids = BTreeSet::new();
    let mut candidates = Vec::new();
    let mut quotes = Vec::new();

    for (model_id, model_value) in model_map {
        let Some(model) = model_value.as_object() else {
            return Err(format!("model {model_id} must be an object"));
        };
        let Some(provider_id) = model.get("litellm_provider").and_then(Value::as_str) else {
            continue;
        };
        let provider_id = provider_id.trim();
        if provider_id.is_empty() || model_id == "sample_spec" || provider_id.starts_with("one of ")
        {
            continue;
        }
        let mut rates = Vec::new();
        for (key, value) in model {
            if key.contains("cost") {
                collect_litellm_rates(key, value, &mut rates);
            }
        }
        if rates.is_empty() {
            continue;
        }
        let source_url = model
            .get("source")
            .and_then(Value::as_str)
            .and_then(valid_http_url)
            .map(str::to_string);
        let status = model
            .get("status")
            .and_then(Value::as_str)
            .map(str::to_string);
        let deprecation_date = model
            .get("deprecation_date")
            .and_then(Value::as_str)
            .map(str::to_string);
        let quote = PricingQuote {
            model: model_id.clone(),
            provider_id: provider_id.to_string(),
            provider_name: provider_id.to_string(),
            source: "litellm".to_string(),
            source_version: source_version.to_string(),
            fetched_at: fetched_at.clone(),
            status: status.clone(),
            deprecation_date: deprecation_date.clone(),
            source_url: source_url.clone(),
            rates: rates.clone(),
        };
        let input_per_1m = select_litellm_token_rate(&rates, "input", "text");
        let output_per_1m = select_litellm_token_rate(&rates, "output", "text");
        let cache_read_per_1m = select_litellm_token_rate(&rates, "cacheRead", "text");
        let cache_write_per_1m = select_litellm_token_rate(&rates, "cacheWrite", "text");
        let reasoning_per_1m = select_litellm_token_rate(&rates, "reasoning", "text");
        let unsupported_dimensions = rates
            .iter()
            .filter(|rate| {
                !(rate.unit == "usd_per_1m_tokens"
                    && rate.tier.is_none()
                    && rate.threshold.is_none()
                    && rate
                        .modality
                        .as_deref()
                        .is_none_or(|modality| modality == "text"))
            })
            .map(|rate| rate.key.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        provider_ids.insert(provider_id.to_string());
        candidates.push(ModelsDevPriceCandidate {
            model: model_id.clone(),
            name: model
                .get("display_name")
                .or_else(|| model.get("model_name"))
                .or_else(|| model.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(model_id)
                .to_string(),
            provider_id: provider_id.to_string(),
            provider_name: provider_id.to_string(),
            doc_url: source_url,
            status,
            deprecation_date,
            input_per_1m,
            output_per_1m,
            cache_read_per_1m,
            cache_write_per_1m,
            reasoning_per_1m,
            unsupported_dimensions,
            importable: input_per_1m.is_some() && output_per_1m.is_some(),
        });
        quotes.push(quote);
    }

    let providers = provider_ids
        .into_iter()
        .map(|id| ModelsDevSyncProvider {
            id: id.clone(),
            name: id,
            doc_url: None,
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        a.provider_name
            .cmp(&b.provider_name)
            .then_with(|| a.model.cmp(&b.model))
    });
    quotes.sort_by(|a, b| {
        a.provider_id
            .cmp(&b.provider_id)
            .then_with(|| a.model.cmp(&b.model))
    });
    Ok(ModelsDevSyncPreview {
        fetched_at,
        source_url: LITELLM_PRICE_CATALOG_URL.to_string(),
        source_version: source_version.to_string(),
        provider_count: providers.len(),
        candidate_count: candidates.len(),
        providers,
        candidates,
        quotes,
        sync_state: crate::models_dev_sync_memory::ModelsDevSyncMemoryState::default(),
    })
}

fn litellm_rate(key: &str, source_value: f64) -> PricingRate {
    let source_unit = if key.contains("per_image_token") || key.contains("image_token_cost") {
        "usd_per_image_token"
    } else if key.contains("per_audio_token") || key.contains("audio_token_cost") {
        "usd_per_audio_token"
    } else if key.contains("per_video_token") || key.contains("video_token_cost") {
        "usd_per_video_token"
    } else if key.contains("per_reasoning_token") || key.contains("reasoning_token_cost") {
        "usd_per_reasoning_token"
    } else if key.contains("per_token") || key.contains("token_cost") {
        "usd_per_token"
    } else if key.contains("per_second") {
        "usd_per_second"
    } else if key.contains("per_minute") {
        "usd_per_minute"
    } else if key.contains("per_character") {
        "usd_per_character"
    } else if key.contains("per_1k_tokens") {
        "usd_per_1k_tokens"
    } else if key.contains("per_1k_calls") {
        "usd_per_1k_calls"
    } else if key.contains("per_gb_per_day") {
        "usd_per_gb_per_day"
    } else if key.contains("per_gb") {
        "usd_per_gb"
    } else if key.contains("per_page") {
        "usd_per_page"
    } else if key.contains("per_pixel") {
        "usd_per_pixel"
    } else if key.contains("per_query") {
        "usd_per_query"
    } else if key.contains("per_request") {
        "usd_per_request"
    } else if key.contains("per_session") {
        "usd_per_session"
    } else if key.contains("per_text_message") {
        "usd_per_text_message"
    } else if key.contains("per_message") {
        "usd_per_message"
    } else if key.contains("per_credit") {
        "usd_per_credit"
    } else if key.contains("per_day") {
        "usd_per_day"
    } else if key.contains("per_unit") {
        "usd_per_unit"
    } else if key.contains("per_image") {
        "usd_per_image"
    } else {
        "usd"
    };
    let token_unit = source_unit.ends_with("_token") || source_unit == "usd_per_1k_tokens";
    let (value, unit) = if source_unit == "usd_per_1k_tokens" {
        (source_value * 1_000.0, "usd_per_1m_tokens")
    } else if token_unit {
        (source_value * 1_000_000.0, "usd_per_1m_tokens")
    } else {
        (source_value, source_unit)
    };
    let dimension = if key.contains("reasoning") {
        Some("reasoning")
    } else if key.contains("cache_read") {
        Some("cacheRead")
    } else if key.contains("cache_creation") || key.contains("cache_write") {
        Some("cacheWrite")
    } else if key.starts_with("input") {
        Some("input")
    } else if key.starts_with("output") {
        Some("output")
    } else if key.starts_with("reasoning") {
        Some("reasoning")
    } else {
        None
    };
    let modality = if key.contains("image") {
        Some("image")
    } else if key.contains("audio") {
        Some("audio")
    } else if key.contains("video") {
        Some("video")
    } else if token_unit {
        Some("text")
    } else {
        None
    };
    let tier = [
        "batches",
        "batch",
        "flex",
        "priority",
        "standard",
        "decisions",
        "ultrafast",
        "balanced",
        "cache_hit",
        "above_1hr",
        "above_15s_interval",
        "above_8s_interval",
    ]
    .iter()
    .find(|tier| key.contains(**tier))
    .map(|tier| (*tier).to_string());
    let threshold = litellm_context_threshold(key);
    PricingRate {
        key: key.to_string(),
        source_value,
        source_unit: source_unit.to_string(),
        value,
        unit: unit.to_string(),
        dimension: dimension.map(str::to_string),
        modality: modality.map(str::to_string),
        tier,
        threshold,
    }
}

fn litellm_context_threshold(key: &str) -> Option<f64> {
    let parts = key.split('_').collect::<Vec<_>>();
    parts.windows(3).find_map(|window| {
        if window[0] != "above" || !window[2].starts_with("token") {
            return None;
        }
        let number = window[1]
            .strip_suffix('k')
            .or_else(|| window[1].strip_suffix('K'))?;
        number.parse::<f64>().ok().map(|value| value * 1_000.0)
    })
}

fn select_litellm_token_rate(
    rates: &[PricingRate],
    dimension: &str,
    modality: &str,
) -> Option<f64> {
    rates
        .iter()
        .find(|rate| {
            rate.dimension.as_deref() == Some(dimension)
                && rate.unit == "usd_per_1m_tokens"
                && rate.tier.is_none()
                && rate.threshold.is_none()
                && rate.modality.as_deref() == Some(modality)
        })
        .map(|rate| rate.value)
}

fn collect_litellm_rates(key: &str, value: &Value, rates: &mut Vec<PricingRate>) {
    if let Some(value) = value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        rates.push(litellm_rate(key, value));
        return;
    }
    if let Some(object) = value.as_object() {
        for (child_key, child_value) in object {
            collect_litellm_rates(&format!("{key}.{child_key}"), child_value, rates);
        }
    } else if let Some(array) = value.as_array() {
        for (index, child_value) in array.iter().enumerate() {
            collect_litellm_rates(&format!("{key}[{index}]"), child_value, rates);
        }
    }
}

fn model_cost(
    cost: &serde_json::Map<String, Value>,
    field: &str,
    provider_id: &str,
    model_id: &str,
) -> Result<Option<f64>, String> {
    let Some(value) = cost.get(field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let value = value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| format!("invalid {field} price for {provider_id}/{model_id}"))?;
    Ok(Some(value))
}

fn valid_http_url(value: &str) -> Option<&str> {
    reqwest::Url::parse(value)
        .ok()
        .filter(|url| matches!(url.scheme(), "https" | "http"))
        .map(|_| value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn models_dev_catalog_maps_supported_prices_and_reports_other_dimensions() {
        let parsed = parse_models_dev_catalog(&json!({
            "provider": {
                "name": "Provider",
                "doc": "https://provider.example/docs",
                "models": {
                    "model-a": {
                        "name": "Model A",
                        "status": "deprecated",
                        "cost": {
                            "input": 1.0,
                            "output": 2.0,
                            "cache_read": 0.1,
                            "cache_write": 0.2,
                            "reasoning": 3.0,
                            "input_audio": 4.0,
                            "batch": 0.5
                        }
                    }
                }
            }
        }))
        .expect("parse catalog");
        let candidate = &parsed.candidates[0];
        assert_eq!(candidate.status.as_deref(), Some("deprecated"));
        assert_eq!(candidate.input_per_1m, Some(1.0));
        assert_eq!(candidate.output_per_1m, Some(2.0));
        assert_eq!(candidate.cache_read_per_1m, Some(0.1));
        assert_eq!(candidate.cache_write_per_1m, Some(0.2));
        assert_eq!(candidate.reasoning_per_1m, Some(3.0));
        assert_eq!(candidate.unsupported_dimensions, ["batch", "input_audio"]);
        assert!(candidate.importable);
    }

    #[test]
    fn litellm_catalog_preserves_openai_image_25_modal_rates() {
        let parsed = parse_litellm_catalog(
            &json!({
                "gpt-image-2.5-flare": {
                    "litellm_provider": "openai",
                    "source": "https://developers.openai.com/api/docs/pricing",
                    "input_cost_per_token": 0.000005,
                    "cache_read_input_token_cost": 0.00000125,
                    "cache_read_input_image_token_cost": 0.000002,
                    "input_cost_per_image_token": 0.000008,
                    "output_cost_per_image_token": 0.00003
                },
                "gpt-image-2.5-sunburst": {
                    "litellm_provider": "openai",
                    "input_cost_per_token": 0.000005,
                    "cache_read_input_token_cost": 0.00000125,
                "cache_read_input_image_token_cost": 0.000002,
                    "input_cost_per_image_token": 0.000008,
                    "output_cost_per_image_token": 0.00003
                }
            }),
            "sha256:test",
        )
        .expect("LiteLLM fixture should parse");

        assert_eq!(parsed.candidates.len(), 2);
        assert_eq!(parsed.providers[0].id, "openai");
        for quote in &parsed.quotes {
            assert_eq!(quote.source, "litellm");
            assert!(
                quote
                    .rates
                    .iter()
                    .any(|rate| { rate.key == "input_cost_per_token" && rate.value == 5.0 })
            );
            assert!(
                quote.rates.iter().any(|rate| {
                    rate.key == "cache_read_input_token_cost" && rate.value == 1.25
                })
            );
            assert!(quote.rates.iter().any(|rate| {
                rate.key == "cache_read_input_image_token_cost" && rate.value == 2.0
            }));
            assert!(
                quote
                    .rates
                    .iter()
                    .any(|rate| { rate.key == "input_cost_per_image_token" && rate.value == 8.0 })
            );
            assert!(
                quote.rates.iter().any(|rate| {
                    rate.key == "output_cost_per_image_token" && rate.value == 30.0
                })
            );
        }
        assert!(parsed.candidates.iter().all(|candidate| {
            candidate.input_per_1m == Some(5.0)
                && candidate.output_per_1m.is_none()
                && !candidate.importable
        }));
    }

    #[test]
    fn litellm_catalog_normalizes_non_text_units_tiers_and_skips_sample_spec() {
        let parsed = parse_litellm_catalog(
            &json!({
                "sample_spec": {
                    "litellm_provider": "one of https://docs.litellm.ai/docs/providers",
                    "input_cost_per_token": 0.0,
                    "output_cost_per_token": 0.0
                },
                "multimodal-model": {
                    "litellm_provider": "provider",
                    "input_cost_per_audio_token": 0.000003,
                    "output_cost_per_video_token": 0.000004,
                    "output_cost_per_reasoning_token": 0.000005,
                    "input_cost_per_1k_tokens": 0.002,
                    "input_cost_per_token_above_272k_tokens_decisions": 0.000001,
                    "output_cost_per_image_0.5K": 0.01,
                    "file_search_cost_per_1k_calls": 0.002,
                    "vector_store_cost_per_gb_per_day": 0.03,
                    "input_cost_per_text_message": 0.004,
                    "search_context_cost_per_query": {
                        "search_context_size_low": 0.01,
                        "search_context_size_medium": 0.02,
                        "search_context_size_high": 0.03
                    },
                    "guardrail_cost_per_unit": {
                        "contentPolicyUnits": 0.00015
                    }
                },
                "threshold-only-model": {
                    "litellm_provider": "provider",
                    "input_cost_per_token_above_272k_tokens": 0.000001,
                    "output_cost_per_token_above_272k_tokens": 0.000002
                }
            }),
            "sha256:test",
        )
        .expect("LiteLLM fixture should parse");

        assert_eq!(parsed.candidates.len(), 2);
        let multimodal = parsed
            .candidates
            .iter()
            .find(|candidate| candidate.model == "multimodal-model")
            .expect("multimodal candidate");
        assert!(!multimodal.importable);
        let quote = parsed
            .quotes
            .iter()
            .find(|quote| quote.model == "multimodal-model")
            .expect("multimodal quote");
        let audio = quote
            .rates
            .iter()
            .find(|rate| rate.key == "input_cost_per_audio_token")
            .expect("audio rate");
        assert_eq!(audio.value, 3.0);
        assert_eq!(audio.unit, "usd_per_1m_tokens");
        assert_eq!(audio.modality.as_deref(), Some("audio"));
        let per_thousand = quote
            .rates
            .iter()
            .find(|rate| rate.key == "input_cost_per_1k_tokens")
            .expect("per-thousand token rate");
        assert_eq!(per_thousand.value, 2.0);
        assert_eq!(per_thousand.threshold, None);
        let reasoning = quote
            .rates
            .iter()
            .find(|rate| rate.key == "output_cost_per_reasoning_token")
            .expect("reasoning rate");
        assert_eq!(reasoning.value, 5.0);
        assert_eq!(reasoning.dimension.as_deref(), Some("reasoning"));
        let tiered = quote
            .rates
            .iter()
            .find(|rate| rate.key == "input_cost_per_token_above_272k_tokens_decisions")
            .expect("tiered rate");
        assert_eq!(tiered.value, 1.0);
        assert_eq!(tiered.tier.as_deref(), Some("decisions"));
        assert_eq!(tiered.threshold, Some(272_000.0));
        let resolution_rate = quote
            .rates
            .iter()
            .find(|rate| rate.key == "output_cost_per_image_0.5K")
            .expect("image resolution rate");
        assert_eq!(resolution_rate.unit, "usd_per_image");
        assert_eq!(resolution_rate.threshold, None);
        let calls_rate = quote
            .rates
            .iter()
            .find(|rate| rate.key == "file_search_cost_per_1k_calls")
            .expect("call rate");
        assert_eq!(calls_rate.unit, "usd_per_1k_calls");
        let storage_rate = quote
            .rates
            .iter()
            .find(|rate| rate.key == "vector_store_cost_per_gb_per_day")
            .expect("storage rate");
        assert_eq!(storage_rate.unit, "usd_per_gb_per_day");
        let text_message_rate = quote
            .rates
            .iter()
            .find(|rate| rate.key == "input_cost_per_text_message")
            .expect("text message rate");
        assert_eq!(text_message_rate.source_unit, "usd_per_text_message");
        assert_eq!(text_message_rate.unit, "usd_per_text_message");
        assert_eq!(text_message_rate.value, 0.004);
        for (key, value) in [
            (
                "search_context_cost_per_query.search_context_size_low",
                0.01,
            ),
            (
                "search_context_cost_per_query.search_context_size_medium",
                0.02,
            ),
            (
                "search_context_cost_per_query.search_context_size_high",
                0.03,
            ),
        ] {
            let rate = quote
                .rates
                .iter()
                .find(|rate| rate.key == key)
                .expect("nested query rate");
            assert_eq!(rate.source_value, value);
            assert_eq!(rate.unit, "usd_per_query");
        }
        let guardrail = quote
            .rates
            .iter()
            .find(|rate| rate.key == "guardrail_cost_per_unit.contentPolicyUnits")
            .expect("nested guardrail rate");
        assert_eq!(guardrail.source_value, 0.00015);
        assert_eq!(guardrail.unit, "usd_per_unit");
        let threshold_only = parsed
            .candidates
            .iter()
            .find(|candidate| candidate.model == "threshold-only-model")
            .expect("threshold-only candidate");
        assert_eq!(threshold_only.input_per_1m, None);
        assert_eq!(threshold_only.output_per_1m, None);
        assert!(!threshold_only.importable);
    }

    #[test]
    fn models_dev_catalog_marks_missing_input_or_output_as_unimportable() {
        let parsed = parse_models_dev_catalog(&json!({
            "provider": {
                "models": {
                    "model-a": { "cost": { "input_audio": 1.0 } }
                }
            }
        }))
        .expect("parse catalog");
        assert!(!parsed.candidates[0].importable);
        assert_eq!(parsed.candidates[0].status, None);
        assert_eq!(parsed.candidates[0].unsupported_dimensions, ["input_audio"]);
    }

    #[test]
    fn models_dev_catalog_rejects_invalid_supported_prices() {
        let parsed = parse_models_dev_catalog(&json!({
            "provider": {
                "models": {
                    "model-a": { "cost": { "input": -1.0, "output": 1.0 } }
                }
            }
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn models_dev_response_body_stops_at_the_configured_limit() {
        let mut body = Vec::new();
        assert!(append_limited_response_chunk(&mut body, b"1234", 4));
        assert!(!append_limited_response_chunk(&mut body, b"5", 4));
        assert_eq!(body, b"1234");
    }
}
