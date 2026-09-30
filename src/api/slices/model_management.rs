use super::*;
use axum::extract::{Json, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::sync::Arc;
use tracing::warn;

const MODELS_DEV_API_URL: &str = "https://models.dev/api.json";
const MODELS_DEV_MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

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
    pub(crate) provider_count: usize,
    pub(crate) candidate_count: usize,
    pub(crate) providers: Vec<ModelsDevSyncProvider>,
    pub(crate) candidates: Vec<ModelsDevPriceCandidate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelsDevSyncApplyRequest {
    #[serde(default)]
    pub(crate) entries: Vec<PricingEntry>,
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
        .shared
        .get(MODELS_DEV_API_URL)
        .send()
        .await
        .map_err(|err| {
            (
                StatusCode::BAD_GATEWAY,
                format!("models.dev retrieval failed: {err}"),
            )
        })?;
    if !response.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("models.dev returned HTTP {}", response.status()),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MODELS_DEV_MAX_RESPONSE_BYTES as u64)
    {
        return Err((
            StatusCode::BAD_GATEWAY,
            "models.dev response exceeds the 32 MiB limit".to_string(),
        ));
    }
    let body = response.bytes().await.map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("models.dev response read failed: {err}"),
        )
    })?;
    if body.len() > MODELS_DEV_MAX_RESPONSE_BYTES {
        return Err((
            StatusCode::BAD_GATEWAY,
            "models.dev response exceeds the 32 MiB limit".to_string(),
        ));
    }
    let payload: Value = serde_json::from_slice(&body).map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("models.dev response JSON is invalid: {err}"),
        )
    })?;
    let preview = parse_models_dev_catalog(&payload).map_err(|err| {
        warn!(error = %err, "failed to normalize models.dev catalog");
        (
            StatusCode::BAD_GATEWAY,
            "models.dev catalog format is invalid".to_string(),
        )
    })?;
    Ok(Json(preview))
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
    if payload.entries.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "select at least one model price".to_string(),
        ));
    }
    let mut normalized = PricingSettingsUpdateRequest {
        catalog_version: "models.dev".to_string(),
        entries: payload.entries,
    }
    .normalized()?;
    for pricing in normalized.models.values_mut() {
        pricing.source = "models.dev".to_string();
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
            source: "models.dev".to_string(),
        })
        .collect::<Vec<_>>();

    let _update_guard = state.pricing_settings_update_lock.lock().await;
    upsert_synced_model_prices(&state.pool, &entries)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let next = load_pricing_catalog(&state.pool)
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
        "models_dev_pricing_sync",
    )
    .await
    {
        warn!(error = %err, "failed to wake ProxyCost backfill after models.dev price sync");
    }
    Ok(Json(PricingSettingsResponse::from_catalog(&next)))
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
    if price_deleted == 0 && model_deleted == 0 && !was_enabled {
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
    tx.commit()
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let next_pricing = load_pricing_catalog(&state.pool)
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
        provider_count: providers.len(),
        candidate_count,
        providers,
        candidates,
    })
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
        assert_eq!(candidate.input_per_1m, Some(1.0));
        assert_eq!(candidate.output_per_1m, Some(2.0));
        assert_eq!(candidate.cache_read_per_1m, Some(0.1));
        assert_eq!(candidate.cache_write_per_1m, Some(0.2));
        assert_eq!(candidate.reasoning_per_1m, Some(3.0));
        assert_eq!(candidate.unsupported_dimensions, ["batch", "input_audio"]);
        assert!(candidate.importable);
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
}
