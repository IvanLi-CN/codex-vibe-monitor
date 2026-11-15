use crate::api::{
    ManagedModelDeleteRequest, ManagedModelDeleteResponse, ManagedModelPresetRequest,
    ModelsDevSyncApplyRequest,
};
use crate::app_state::{
    PricingSettingsResponse, PricingSettingsUpdateRequest, ProxyModelSettings,
    ProxyModelSettingsResponse, ProxyModelSettingsUpdateRequest, SettingsResponse,
};
use crate::config::AppConfig;
use crate::external_api::{
    CreateExternalApiKeyRequest, create_primary_external_api_key, disable_primary_external_api_key,
    list_primary_external_api_keys, rotate_primary_external_api_key,
};
use crate::forward_proxy::{
    ForwardProxyNodeResponse, ForwardProxySettings, ForwardProxySettingsResponse,
    ForwardProxySettingsUpdateRequest, ForwardProxyStatsResponse,
};
use crate::is_same_origin_settings_write;
use crate::oauth_bridge::oauth_codex_upstream_base_url;
use crate::pricing::{
    load_primary_pricing_catalog, load_primary_pricing_quotes, save_primary_pricing_catalog,
    save_primary_pricing_quotes,
};
use crate::primary_db::PrimaryDatabase;
use crate::primary_maintenance::{PostgresPrimaryMaintenanceExecutor, PrimaryMaintenanceExecutor};
use crate::primary_store::{PostgresPrimaryStateStore, PrimaryStateStore, ProxyRequestRecord};
use crate::share_links::stable_forward_proxy_binding_key;
use crate::upstream_accounts::{
    BulkUpstreamAccountActionRequest, BulkUpstreamAccountActionResponse,
    BulkUpstreamAccountActionResult, StoredApiKeyCredentials, StoredCredentials,
    StoredOauthCredentials, decrypt_credentials, encrypt_credentials,
    normalize_bulk_upstream_account_action,
};
use anyhow::{Context, Result, bail};
use axum::body::{Body, Bytes};
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{
    IntoResponse, Response,
    sse::{Event, KeepAlive, Sse},
};
use axum::{
    Json, Router,
    routing::{any, get},
};
use flate2::read::GzDecoder;
use futures_util::{StreamExt, stream};
use serde_json::Value;
use sha2::Digest;
use std::collections::HashMap;
use std::convert::Infallible;
use std::io;
use std::io::Read;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::services::ServeDir;
use tracing::{error, info};
use url::Url;

#[derive(Clone)]
struct PgRuntimeState {
    store: Arc<dyn PrimaryStateStore>,
    crypto_key: Option<[u8; 32]>,
    upstream_base_url: Url,
    http_client: reqwest::Client,
    max_request_body_bytes: usize,
}

const PRIMARY_FORWARD_PROXY_SETTINGS_KEY: &str = "core.forward_proxy_settings.v1";
const PRIMARY_POOL_ROUTING_SETTINGS_KEY: &str = "core.pool_routing_settings.v1";

fn default_pool_routing_settings() -> serde_json::Value {
    serde_json::json!({
        "writesEnabled": true,
        "apiKeyConfigured": false,
        "maskedApiKey": null,
        "maintenance": {
            "primarySyncIntervalSecs": 300,
            "secondarySyncIntervalSecs": 900,
            "priorityAvailableAccountCap": 3
        },
        "requestCompressionAlgorithm": "none",
        "oauthRequestCompressionAlgorithm": "none",
        "requestCompressionLevelPreset": "balanced",
        "codexImagegenRewriteMode": "keep_original",
        "availableModels": [],
        "availableModelsMode": "allowlist",
        "timeouts": {
            "defaultFirstByteTimeoutSecs": 60,
            "defaultSendTimeoutSecs": 60,
            "requestReadTimeoutSecs": 60,
            "responsesFirstByteTimeoutSecs": 60,
            "compactFirstByteTimeoutSecs": 60,
            "imageFirstByteTimeoutSecs": 60,
            "responsesStreamTimeoutSecs": 180,
            "compactStreamTimeoutSecs": 180
        },
        "cacheHitProtection": {
            "enabled": false,
            "lowHitRateThresholdPercent": 20,
            "overflowMode": "allow",
            "minimumInputTokens": 0
        },
        "priorityHandoffAdmissionEnabled": true,
        "automaticReserveFallbackEnabled": true,
        "reserveFallbackModelPolicy": {"mode": "disabled", "patterns": []}
    })
}

fn public_pool_routing_settings(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("_encryptedApiKey");
        object.remove("apiKey");
    }
    value
}

fn merge_json_objects(
    current: &mut serde_json::Map<String, serde_json::Value>,
    update: &serde_json::Map<String, serde_json::Value>,
) {
    for (key, value) in update {
        match (current.get_mut(key), value) {
            (Some(serde_json::Value::Object(current)), serde_json::Value::Object(update)) => {
                merge_json_objects(current, update);
            }
            _ => {
                current.insert(key.clone(), value.clone());
            }
        }
    }
}

async fn get_pool_routing_settings(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let value = state
        .store
        .get_json(PRIMARY_POOL_ROUTING_SETTINGS_KEY)
        .await
        .map_err(internal_error)?
        .unwrap_or_else(default_pool_routing_settings);
    Ok(Json(public_pool_routing_settings(value)))
}

async fn put_pool_routing_settings(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    if !payload.is_object() {
        return Err((
            StatusCode::BAD_REQUEST,
            "pool routing settings payload must be an object".to_string(),
        ));
    }
    const MUTABLE_KEYS: &[&str] = &[
        "apiKey",
        "maintenance",
        "requestCompressionAlgorithm",
        "oauthRequestCompressionAlgorithm",
        "requestCompressionLevelPreset",
        "codexImagegenRewriteMode",
        "availableModels",
        "availableModelsMode",
        "timeouts",
        "cacheHitProtection",
        "priorityHandoffAdmissionEnabled",
        "automaticReserveFallbackEnabled",
        "reserveFallbackModelPolicy",
    ];
    if let Some(unknown) = payload.as_object().and_then(|object| {
        object
            .keys()
            .find(|key| !MUTABLE_KEYS.contains(&key.as_str()))
    }) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("unknown or read-only pool routing setting: {unknown}"),
        ));
    }
    let mut payload = payload;
    if let Some(api_key) = payload
        .get("apiKey")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
    {
        let key = state.crypto_key.as_ref().ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "pool routing secret storage is unavailable".to_string(),
            )
        })?;
        let encrypted = crate::upstream_accounts::encrypt_secret_value(key, &api_key)
            .map_err(internal_error)?;
        let object = payload.as_object_mut().expect("payload object checked");
        object.remove("apiKey");
        object.insert(
            "_encryptedApiKey".to_string(),
            serde_json::Value::String(encrypted),
        );
        object.insert(
            "maskedApiKey".to_string(),
            serde_json::Value::String(crate::mask_api_key(&api_key)),
        );
        object.insert(
            "apiKeyConfigured".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    let mut next = state
        .store
        .get_json(PRIMARY_POOL_ROUTING_SETTINGS_KEY)
        .await
        .map_err(internal_error)?
        .unwrap_or_else(default_pool_routing_settings);
    if let (Some(current), Some(update)) = (next.as_object_mut(), payload.as_object()) {
        merge_json_objects(current, update);
    }
    state
        .store
        .put_json(PRIMARY_POOL_ROUTING_SETTINGS_KEY, &next)
        .await
        .map_err(internal_error)?;
    Ok(Json(public_pool_routing_settings(next)))
}

async fn get_primary_upstream_accounts(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("pool_upstream_accounts")
        .await
        .map_err(internal_error)?;
    let items = rows
        .into_iter()
        .map(public_primary_account_row)
        .collect::<Vec<_>>();
    let total = items.len();
    let oauth = items
        .iter()
        .filter(|item| {
            item.get("kind").and_then(serde_json::Value::as_str) != Some("api_key_codex")
        })
        .count();
    let api_key = total.saturating_sub(oauth);
    let attention = items
        .iter()
        .filter(|item| {
            item.get("status")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|status| matches!(status, "error" | "attention" | "expired"))
        })
        .count();
    let groups = items
        .iter()
        .filter_map(|item| item.get("groupName").and_then(serde_json::Value::as_str))
        .filter(|group| !group.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|name| serde_json::json!({"name": name}))
        .collect::<Vec<_>>();
    let has_ungrouped_accounts = items
        .iter()
        .any(|item| item.get("groupName").is_none_or(serde_json::Value::is_null));
    let routing = state
        .store
        .get_json(PRIMARY_POOL_ROUTING_SETTINGS_KEY)
        .await
        .map_err(internal_error)?
        .unwrap_or_else(default_pool_routing_settings);
    let routing = public_pool_routing_settings(routing);
    let writes_enabled = routing
        .get("writesEnabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    Ok(Json(serde_json::json!({
        "writesEnabled": writes_enabled,
        "items": items,
        "total": total,
        "page": 1,
        "pageSize": total,
        "metrics": {"total": total, "oauth": oauth, "apiKey": api_key, "attention": attention},
        "groups": groups,
        "hasUngroupedAccounts": has_ungrouped_accounts,
        "routing": routing
    })))
}

async fn create_primary_api_key_account(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_owned(),
        ));
    }
    let object = payload.as_object().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "API key account payload must be an object".to_owned(),
        )
    })?;
    let display_name = object
        .get("displayName")
        .or_else(|| object.get("display_name"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "displayName is required".to_owned(),
            )
        })?;
    let api_key = object
        .get("apiKey")
        .or_else(|| object.get("api_key"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "apiKey is required".to_owned()))?;
    let crypto_key = state.crypto_key.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "account credential encryption is unavailable".to_owned(),
        )
    })?;
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::ApiKey(StoredApiKeyCredentials {
            api_key: api_key.to_owned(),
        }),
    )
    .map_err(internal_error)?;
    if let Some(value) = object
        .get("upstreamBaseUrl")
        .or_else(|| object.get("upstream_base_url"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        let parsed = Url::parse(value).map_err(|error| {
            (
                StatusCode::BAD_REQUEST,
                format!("upstreamBaseUrl is invalid: {error}"),
            )
        })?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "upstreamBaseUrl must be an absolute HTTP(S) URL".to_owned(),
            ));
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    let mut account = serde_json::Map::new();
    account.insert("kind".to_owned(), serde_json::json!("api_key_codex"));
    account.insert("provider".to_owned(), serde_json::json!("codex"));
    account.insert("display_name".to_owned(), serde_json::json!(display_name));
    account.insert("status".to_owned(), serde_json::json!("active"));
    account.insert("enabled".to_owned(), serde_json::json!(1));
    account.insert("is_mother".to_owned(), serde_json::json!(0));
    account.insert("has_refresh_token".to_owned(), serde_json::json!(0));
    account.insert(
        "masked_api_key".to_owned(),
        serde_json::json!(crate::mask_api_key(api_key)),
    );
    account.insert(
        "encrypted_credentials".to_owned(),
        serde_json::json!(encrypted_credentials),
    );
    account.insert("created_at".to_owned(), serde_json::json!(&now));
    account.insert("updated_at".to_owned(), serde_json::json!(&now));
    account.insert(
        "bound_proxy_keys_json".to_owned(),
        serde_json::json!("[\"__direct__\"]"),
    );
    for (input_key, column) in [
        ("email", "email"),
        ("note", "note"),
        ("upstreamBaseUrl", "upstream_base_url"),
        ("upstream_base_url", "upstream_base_url"),
    ] {
        if let Some(value) = object.get(input_key) {
            account.insert(column.to_owned(), value.clone());
        }
    }
    let row = state
        .store
        .create_materialized_account(&serde_json::Value::Object(account))
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::CONFLICT,
                "a verified migration run is required before creating accounts".to_owned(),
            )
        })?;
    Ok(Json(public_primary_account_row(row)))
}

async fn bulk_update_primary_upstream_accounts(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<BulkUpstreamAccountActionRequest>,
) -> Result<Json<BulkUpstreamAccountActionResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_owned(),
        ));
    }
    let action = normalize_bulk_upstream_account_action(&payload.action)?;
    let account_ids = payload.account_ids;
    if account_ids.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "accountIds must contain at least one account".to_owned(),
        ));
    }
    let mut results = Vec::with_capacity(account_ids.len());
    for account_id in account_ids {
        let outcome = state
            .store
            .mutate_materialized_account(account_id, &action, payload.group_name.as_deref())
            .await;
        let (status, detail, display_name) = match outcome {
            Ok(Some(row)) => (
                "succeeded".to_owned(),
                None,
                row.get("display_name")
                    .or_else(|| row.get("displayName"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
            ),
            Ok(None) => (
                "failed".to_owned(),
                Some("upstream account was not found in the PG primary data".to_owned()),
                None,
            ),
            Err(error) => ("failed".to_owned(), Some(error.to_string()), None),
        };
        results.push(BulkUpstreamAccountActionResult {
            account_id,
            display_name,
            status,
            detail,
        });
    }
    let succeeded_count = results
        .iter()
        .filter(|result| result.status == "succeeded")
        .count();
    Ok(Json(BulkUpstreamAccountActionResponse {
        action,
        requested_count: results.len(),
        completed_count: results.len(),
        succeeded_count,
        failed_count: results.len().saturating_sub(succeeded_count),
        results,
    }))
}

async fn get_primary_upstream_account(
    State(state): State<PgRuntimeState>,
    Path(account_id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("pool_upstream_accounts")
        .await
        .map_err(internal_error)?;
    let row = rows.into_iter().find(|row| {
        row.get("id")
            .and_then(serde_json::Value::as_i64)
            .is_some_and(|id| id == account_id)
    });
    let Some(row) = row else {
        return Err((
            StatusCode::NOT_FOUND,
            "upstream account was not found in the PG primary data".to_owned(),
        ));
    };
    Ok(Json(public_primary_account_row(row)))
}

async fn delete_primary_upstream_account(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Path(account_id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_owned(),
        ));
    }
    let row = state
        .store
        .mutate_materialized_account(account_id, "delete", None)
        .await
        .map_err(internal_error)?;
    if row.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            "upstream account was not found in the PG primary data".to_owned(),
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn update_primary_upstream_account(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Path(account_id): Path<i64>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_owned(),
        ));
    }
    let action = if payload.get("enabled").and_then(serde_json::Value::as_bool) == Some(true) {
        Some("enable")
    } else if payload.get("enabled").and_then(serde_json::Value::as_bool) == Some(false) {
        Some("disable")
    } else if payload.get("groupName").is_some() || payload.get("group_name").is_some() {
        Some("set_group")
    } else {
        None
    };
    if action.is_none() {
        let row = state
            .store
            .update_materialized_account(account_id, &payload)
            .await
            .map_err(internal_error)?
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    "upstream account was not found in the PG primary data".to_owned(),
                )
            })?;
        return Ok(Json(public_primary_account_row(row)));
    }
    let Some(action) = action else {
        return Err((
            StatusCode::BAD_REQUEST,
            "account update must include enabled or groupName".to_owned(),
        ));
    };
    let group_name = payload
        .get("groupName")
        .or_else(|| payload.get("group_name"))
        .and_then(serde_json::Value::as_str);
    let row = state
        .store
        .mutate_materialized_account(account_id, action, group_name)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "upstream account was not found in the PG primary data".to_owned(),
            )
        })?;
    Ok(Json(public_primary_account_row(row)))
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrimaryPageQuery {
    page: Option<usize>,
    page_size: Option<usize>,
}

fn snake_to_camel(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut uppercase = false;
    for character in value.chars() {
        if character == '_' {
            uppercase = true;
        } else if uppercase {
            output.extend(character.to_uppercase());
            uppercase = false;
        } else {
            output.push(character);
        }
    }
    output
}

fn public_materialized_row(row: serde_json::Value) -> serde_json::Value {
    let Some(source) = row.as_object() else {
        return row;
    };
    let mut output = serde_json::Map::new();
    for (key, value) in source {
        if key.starts_with("_cvm_")
            || matches!(
                key.as_str(),
                "encrypted_credentials"
                    | "encryptedCredentials"
                    | "credentials"
                    | "access_token"
                    | "accessToken"
                    | "refresh_token"
                    | "refreshToken"
                    | "api_key"
                    | "apiKey"
            )
        {
            continue;
        }
        output.insert(snake_to_camel(key), value.clone());
    }
    serde_json::Value::Object(output)
}

async fn get_primary_invocations(
    State(state): State<PgRuntimeState>,
    Query(query): Query<PrimaryPageQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = load_primary_invocation_rows(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 500);
    let total = rows.len();
    let records = rows
        .into_iter()
        .skip((page - 1).saturating_mul(page_size))
        .take(page_size)
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({
        "snapshotId": 0,
        "total": total,
        "page": page,
        "pageSize": page_size,
        "records": records
    })))
}

async fn load_primary_invocation_rows(
    store: &dyn PrimaryStateStore,
) -> Result<Vec<serde_json::Value>> {
    let mut rows = store.latest_materialized_rows("codex_invocations").await?;
    let manifests = store
        .get_json("maintenance.retention_archives")
        .await?
        .filter(Value::is_array)
        .or(store
            .get_json("maintenance.retention_archive")
            .await?
            .map(|manifest| Value::Array(vec![manifest])))
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let root = std::env::var_os("DATABASE_PATH")
        .map(PathBuf::from)
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let archive_paths = manifests
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|manifest| manifest.get("archivePath").and_then(Value::as_str))
        .map(PathBuf::from)
        .filter(|relative| {
            relative.is_relative()
                && !relative
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
        })
        .map(|relative| root.join(relative))
        .collect::<Vec<_>>();
    let archived = tokio::task::spawn_blocking(move || -> Result<Vec<Value>> {
        let mut rows = Vec::new();
        for path in archive_paths {
            let file = std::fs::File::open(&path).with_context(|| {
                format!("failed to open PG retention archive {}", path.display())
            })?;
            let mut decoder = GzDecoder::new(file);
            let mut bytes = Vec::new();
            decoder.read_to_end(&mut bytes)?;
            rows.extend(
                serde_json::from_slice::<Vec<Value>>(&bytes)
                    .context("invalid PG retention archive JSON")?,
            );
        }
        Ok(rows)
    })
    .await
    .context("PG retention archive reader task failed")??;
    let existing_ids = rows
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_i64))
        .collect::<std::collections::HashSet<_>>();
    rows.extend(archived.into_iter().filter(|row| {
        row.get("id")
            .and_then(Value::as_i64)
            .is_none_or(|id| !existing_ids.contains(&id))
    }));
    Ok(rows)
}

async fn load_primary_activity_rows(
    store: &dyn PrimaryStateStore,
) -> Result<Vec<serde_json::Value>> {
    let mut rows = load_primary_invocation_rows(store).await?;
    if rows.is_empty() {
        rows = store
            .latest_materialized_rows("codex_invocation_summary")
            .await
            .unwrap_or_default();
    }
    rows.extend(
        store
            .latest_materialized_rows("proxy_requests")
            .await
            .unwrap_or_default(),
    );
    Ok(rows)
}

fn primary_json_i64(value: Option<&serde_json::Value>) -> i64 {
    value
        .and_then(serde_json::Value::as_i64)
        .or_else(|| value.and_then(serde_json::Value::as_str)?.parse().ok())
        .unwrap_or_default()
}

fn primary_json_f64(value: Option<&serde_json::Value>) -> f64 {
    value
        .and_then(serde_json::Value::as_f64)
        .or_else(|| value.and_then(serde_json::Value::as_str)?.parse().ok())
        .unwrap_or_default()
}

async fn get_primary_stats(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = load_primary_activity_rows(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let total_count = rows.len() as i64;
    let mut success_count = 0_i64;
    let mut total_tokens = 0_i64;
    let mut total_cost = 0.0_f64;
    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        let status = object
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(
            status.as_str(),
            "ok" | "success" | "succeeded" | "completed"
        ) {
            success_count += 1;
        }
        total_tokens += primary_json_i64(
            object
                .get("total_tokens")
                .or_else(|| object.get("totalTokens")),
        );
        total_cost += primary_json_f64(
            object
                .get("cost")
                .or_else(|| object.get("total_cost"))
                .or_else(|| object.get("totalCost")),
        );
    }
    Ok(Json(serde_json::json!({
        "totalCount": total_count,
        "successCount": success_count,
        "failureCount": total_count.saturating_sub(success_count),
        "totalCost": total_cost,
        "totalTokens": total_tokens
    })))
}

async fn get_primary_invocation_summary(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = load_primary_activity_rows(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let total_count = rows.len() as i64;
    let mut success_count = 0_i64;
    let mut total_tokens = 0_i64;
    let mut total_cost = 0.0_f64;
    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        let status = object
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(
            status.as_str(),
            "ok" | "success" | "succeeded" | "completed"
        ) {
            success_count += 1;
        }
        total_tokens += primary_json_i64(
            object
                .get("total_tokens")
                .or_else(|| object.get("totalTokens")),
        );
        total_cost += primary_json_f64(
            object
                .get("cost")
                .or_else(|| object.get("total_cost"))
                .or_else(|| object.get("totalCost")),
        );
    }
    Ok(Json(serde_json::json!({
        "snapshotId": 0,
        "newRecordsCount": 0,
        "totalCount": total_count,
        "successCount": success_count,
        "failureCount": total_count.saturating_sub(success_count),
        "totalTokens": total_tokens,
        "totalCost": total_cost,
        "token": {"input": 0, "output": 0, "cached": 0},
        "network": {"requestCount": total_count, "successCount": success_count},
        "exception": {"serviceFailureCount": 0, "clientFailureCount": 0, "clientAbortCount": 0, "actionableFailureCount": 0}
    })))
}

async fn get_primary_stats_timeseries(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = load_primary_activity_rows(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let mut buckets = std::collections::BTreeMap::<String, (i64, i64, f64)>::new();
    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        let timestamp = object
            .get("started_at")
            .or_else(|| object.get("startedAt"))
            .or_else(|| object.get("created_at"))
            .or_else(|| object.get("createdAt"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let bucket = timestamp.get(..10).unwrap_or(timestamp).to_owned();
        let entry = buckets.entry(bucket).or_default();
        entry.0 += 1;
        let status = object
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(
            status.as_str(),
            "ok" | "success" | "succeeded" | "completed"
        ) {
            entry.1 += 1;
        }
        entry.2 += primary_json_f64(
            object
                .get("cost")
                .or_else(|| object.get("total_cost"))
                .or_else(|| object.get("totalCost")),
        );
    }
    let series = buckets
        .into_iter()
        .map(|(date, (count, success_count, cost))| {
            serde_json::json!({"date": date, "count": count, "successCount": success_count, "cost": cost})
        })
        .collect::<Vec<_>>();
    Ok(Json(
        serde_json::json!({"series": series, "isSampled": false}),
    ))
}

async fn get_primary_stats_perf(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("codex_invocations")
        .await
        .map_err(internal_error)?;
    let durations = rows
        .iter()
        .filter_map(|row| {
            ["duration_ms", "durationMs", "total_duration_ms"]
                .iter()
                .find_map(|key| row.get(*key).and_then(serde_json::Value::as_f64))
        })
        .collect::<Vec<_>>();
    let average = if durations.is_empty() {
        0.0
    } else {
        durations.iter().sum::<f64>() / durations.len() as f64
    };
    Ok(Json(serde_json::json!({
        "count": durations.len(),
        "averageDurationMs": average,
        "durationsMs": durations
    })))
}

async fn get_primary_stats_long_term_overview(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = load_primary_activity_rows(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    Ok(Json(serde_json::json!({
        "available": true,
        "invocationCount": rows.len(),
        "generatedAt": chrono::Utc::now().to_rfc3339()
    })))
}

async fn find_primary_invocation(
    state: &PgRuntimeState,
    invocation_id: i64,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("codex_invocations")
        .await
        .map_err(internal_error)?;
    rows.into_iter()
        .find(|row| {
            row.get("id")
                .and_then(serde_json::Value::as_i64)
                .is_some_and(|id| id == invocation_id)
        })
        .map(public_materialized_row)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "invocation was not found".to_owned()))
}

async fn get_primary_invocation_detail(
    State(state): State<PgRuntimeState>,
    Path(invocation_id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let row = find_primary_invocation(&state, invocation_id).await?;
    Ok(Json(
        serde_json::json!({"id": invocation_id, "record": row}),
    ))
}

async fn get_primary_invocation_locate(
    State(state): State<PgRuntimeState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("codex_invocations")
        .await
        .map_err(internal_error)?;
    let requested = query
        .get("invokeId")
        .or_else(|| query.get("invoke_id"))
        .or_else(|| query.get("id"));
    let row = requested.and_then(|requested| {
        rows.into_iter().find(|row| {
            ["id", "invoke_id", "invokeId"]
                .iter()
                .filter_map(|key| row.get(*key))
                .any(|value| {
                    value.as_i64().map(|id| id.to_string()) == Some(requested.clone())
                        || value.as_str() == Some(requested.as_str())
                })
        })
    });
    Ok(Json(serde_json::json!({
        "found": row.is_some(),
        "record": row.map(public_materialized_row)
    })))
}

async fn get_primary_invocation_pool_attempts(
    State(state): State<PgRuntimeState>,
    Path(invocation_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("pool_upstream_request_attempts")
        .await
        .unwrap_or_default();
    let items = rows
        .into_iter()
        .filter(|row| {
            ["invocation_id", "invocationId", "invoke_id", "invokeId"]
                .iter()
                .filter_map(|key| row.get(*key))
                .any(|value| {
                    value.as_i64().map(|id| id.to_string()) == Some(invocation_id.clone())
                        || value.as_str() == Some(invocation_id.as_str())
                })
        })
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    let total = items.len();
    Ok(Json(serde_json::json!({"items": items, "total": total})))
}

async fn get_primary_invocation_suggestions(
    State(state): State<PgRuntimeState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let needle = query
        .get("q")
        .or_else(|| query.get("query"))
        .map(|value| value.to_ascii_lowercase());
    let rows = state
        .store
        .latest_materialized_rows("codex_invocations")
        .await
        .map_err(internal_error)?;
    let items = rows
        .into_iter()
        .filter_map(|row| {
            let public = public_materialized_row(row);
            let text = serde_json::to_string(&public).ok()?.to_ascii_lowercase();
            needle
                .as_ref()
                .is_none_or(|needle| text.contains(needle))
                .then_some(public)
        })
        .take(50)
        .collect::<Vec<_>>();
    let total = items.len();
    Ok(Json(serde_json::json!({"items": items, "total": total})))
}

async fn get_primary_invocation_new_count(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let count = state
        .store
        .latest_materialized_rows("codex_invocations")
        .await
        .map_err(internal_error)?
        .len();
    Ok(Json(serde_json::json!({"count": count})))
}

async fn get_primary_invocation_workflow_detail(
    State(state): State<PgRuntimeState>,
    Path(invocation_id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let row = find_primary_invocation(&state, invocation_id).await?;
    let invocation_id_text = invocation_id.to_string();
    let attempts = state
        .store
        .latest_materialized_rows("pool_upstream_request_attempts")
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|attempt| {
            ["invocation_id", "invocationId", "invoke_id", "invokeId"]
                .iter()
                .filter_map(|key| attempt.get(*key))
                .any(|value| {
                    value.as_i64() == Some(invocation_id)
                        || value.as_str() == Some(invocation_id_text.as_str())
                })
        })
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    let timeline = state
        .store
        .latest_materialized_rows("invocation_timeline")
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|event| {
            ["invocation_id", "invocationId", "invoke_id", "invokeId"]
                .iter()
                .filter_map(|key| event.get(*key))
                .any(|value| {
                    value.as_i64() == Some(invocation_id)
                        || value.as_str() == Some(invocation_id_text.as_str())
                })
        })
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({
        "id": invocation_id,
        "record": row,
        "timeline": timeline,
        "attempts": attempts
    })))
}

async fn get_primary_invocation_body(
    State(state): State<PgRuntimeState>,
    Path(invocation_id): Path<i64>,
    body_key: &'static str,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let row = find_primary_invocation(&state, invocation_id).await?;
    let body = row
        .get(body_key)
        .or_else(|| row.get(snake_to_camel(body_key)))
        .cloned();
    Ok(Json(serde_json::json!({
        "available": body.is_some(),
        "bodyText": body,
        "bodyTruncated": false
    })))
}

async fn get_primary_invocation_response_body(
    State(state): State<PgRuntimeState>,
    Path(invocation_id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_invocation_body(State(state), Path(invocation_id), "response_body").await
}

async fn get_primary_invocation_request_body(
    State(state): State<PgRuntimeState>,
    Path(invocation_id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_invocation_body(State(state), Path(invocation_id), "request_body").await
}

async fn get_primary_system_status(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let accounts = state
        .store
        .latest_materialized_rows("pool_upstream_accounts")
        .await
        .map_err(internal_error)?;
    let invocations = state
        .store
        .latest_materialized_rows("codex_invocations")
        .await
        .map_err(internal_error)?;
    let proxy_requests = state
        .store
        .latest_materialized_rows("proxy_requests")
        .await
        .unwrap_or_default();
    Ok(Json(serde_json::json!({
        "databaseBackend": "postgres",
        "databaseVersion": "18",
        "upstreamAccountCount": accounts.len(),
        "invocationCount": invocations.len() + proxy_requests.len(),
        "status": "ok"
    })))
}

async fn get_primary_system_storage(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let assets = state
        .store
        .latest_materialized_rows("asset_index")
        .await
        .unwrap_or_default();
    let bytes = assets
        .iter()
        .filter_map(|row| row.get("size_bytes").or_else(|| row.get("sizeBytes")))
        .filter_map(serde_json::Value::as_i64)
        .filter(|value| *value >= 0)
        .sum::<i64>();
    Ok(Json(serde_json::json!({
        "databaseBackend": "postgres",
        "databaseVersion": "18",
        "assetCount": assets.len(),
        "assetBytes": bytes,
        "externalAssetStorage": true
    })))
}

async fn get_primary_system_tasks(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = match state
        .store
        .latest_materialized_rows("system_task_runs")
        .await
    {
        Ok(rows) => rows,
        Err(_) => state
            .store
            .latest_materialized_rows("managed_task_runs")
            .await
            .unwrap_or_default(),
    }
    .into_iter()
    .map(public_materialized_row)
    .collect::<Vec<_>>();
    let total = rows.len();
    Ok(Json(serde_json::json!({
        "items": rows,
        "total": total
    })))
}

async fn get_primary_observability() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "databaseBackend": "postgres",
        "databaseVersion": "18",
        "analyticsBackend": "independent",
        "runtime": "pg-only"
    }))
}

async fn get_primary_latest_quota(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut rows = state
        .store
        .latest_materialized_rows("codex_quota_snapshots")
        .await
        .unwrap_or_default();
    let row = rows.pop().map(public_materialized_row);
    Ok(Json(row.unwrap_or(serde_json::Value::Null)))
}

async fn get_primary_pool_tags(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("pool_tags")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    let total = rows.len();
    Ok(Json(serde_json::json!({"items": rows, "total": total})))
}

async fn get_primary_model_routing_live(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let records = state
        .store
        .latest_materialized_rows("pool_upstream_request_attempts")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    let accounts = state
        .store
        .latest_materialized_rows("pool_upstream_accounts")
        .await
        .unwrap_or_default();
    let mut groups = std::collections::BTreeMap::<String, Vec<serde_json::Value>>::new();
    for account in accounts {
        let public = public_primary_account_row(account);
        let group = public
            .get("groupName")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.is_empty())
            .unwrap_or("__ungrouped__")
            .to_owned();
        groups.entry(group).or_default().push(public);
    }
    let groups = groups
        .into_iter()
        .map(|(name, accounts)| serde_json::json!({"name": name, "accounts": accounts}))
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({
        "generatedAt": chrono::Utc::now().to_rfc3339(),
        "groups": groups,
        "records": records
    })))
}

async fn get_primary_upstream_account_events(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let items = state
        .store
        .latest_materialized_rows("upstream_account_action_events")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    let total = items.len();
    Ok(Json(serde_json::json!({"items": items, "total": total})))
}

async fn get_primary_forward_proxy_binding_nodes(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let items = state
        .store
        .latest_materialized_rows("forward_proxy_nodes")
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    let total = items.len();
    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "available": true
    })))
}

async fn get_primary_account_call_attempts(
    State(state): State<PgRuntimeState>,
    Path(account_id): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("pool_upstream_request_attempts")
        .await
        .unwrap_or_default();
    let items = rows
        .into_iter()
        .filter(|row| {
            [
                "upstream_account_id",
                "upstreamAccountId",
                "account_id",
                "accountId",
            ]
            .iter()
            .filter_map(|key| row.get(*key))
            .any(|value| value.as_i64() == Some(account_id))
        })
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    let total = items.len();
    let page = query
        .get("page")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "page": page,
        "pageSize": total,
        "nextCursor": null
    })))
}

async fn get_primary_managed_tasks(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("managed_tasks")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(public_materialized_row)
        .map(|row| async { primary_managed_task_value(&state, row).await })
        .collect::<Vec<_>>();
    let mut merged = Vec::with_capacity(rows.len());
    for row in rows {
        merged.push(row.await?);
    }
    Ok(Json(serde_json::Value::Array(merged)))
}

async fn primary_managed_task_value(
    state: &PgRuntimeState,
    mut task: serde_json::Value,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let Some(task_key) = task
        .get("taskKey")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
    else {
        return Ok(task);
    };
    if let Some(override_value) = state
        .store
        .get_json(&format!("managed_task_override:{task_key}"))
        .await
        .map_err(internal_error)?
        && let (Some(task_object), Some(override_object)) =
            (task.as_object_mut(), override_value.as_object())
    {
        for (key, value) in override_object {
            task_object.insert(key.clone(), value.clone());
        }
    }
    if let Some(last_result) = state
        .store
        .get_json(&format!("managed_task_last_result:{task_key}"))
        .await
        .map_err(internal_error)?
        && let Some(task_object) = task.as_object_mut()
    {
        let attempted_at = last_result
            .get("completedAt")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::String(chrono::Utc::now().to_rfc3339()));
        task_object.insert(
            "lastExecution".to_owned(),
            serde_json::json!({
                "runId": 0,
                "triggerKind": "manual",
                "attemptedAt": attempted_at,
                "finishedAt": last_result.get("completedAt").cloned(),
                "status": last_result.get("status").cloned().unwrap_or_else(|| serde_json::json!("succeeded")),
                "result": last_result.get("status").cloned().unwrap_or_else(|| serde_json::json!("succeeded")),
                "reason": last_result.get("checkedAssets").map(|value| format!("checked_assets={value}"))
            }),
        );
    }
    if let Some(task_object) = task.as_object_mut() {
        let execution_observation = if pg_executable_primary_task(&task_key) {
            "pg_executable"
        } else {
            "queued_until_pg_executor_is_available"
        };
        task_object.insert(
            "executionObservation".to_owned(),
            serde_json::Value::String(execution_observation.to_owned()),
        );
    }
    Ok(task)
}

async fn get_primary_managed_task(
    State(state): State<PgRuntimeState>,
    Path(task_key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let task = state
        .store
        .latest_materialized_rows("managed_tasks")
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(public_materialized_row)
        .find(|row| {
            row.get("taskKey").and_then(serde_json::Value::as_str) == Some(task_key.as_str())
        })
        .ok_or((StatusCode::NOT_FOUND, "managed task not found".to_string()))?;
    let task = primary_managed_task_value(&state, task).await?;
    let runs = state
        .store
        .latest_materialized_rows("managed_task_runs")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(public_materialized_row)
        .filter(|run| {
            run.get("taskKey").and_then(serde_json::Value::as_str) == Some(task_key.as_str())
        })
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({"task": task, "recentRuns": runs})))
}

async fn update_primary_managed_task(
    State(state): State<PgRuntimeState>,
    Path(task_key): Path<String>,
    Json(request): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut patch = serde_json::Map::new();
    for key in ["enabled", "intervalSecs", "cronExpr"] {
        if let Some(value) = request.get(key) {
            patch.insert(key.to_string(), value.clone());
        }
    }
    if patch.is_empty() {
        return get_primary_managed_task(State(state), Path(task_key)).await;
    }
    state
        .store
        .put_json(
            &format!("managed_task_override:{task_key}"),
            &serde_json::Value::Object(patch),
        )
        .await
        .map_err(internal_error)?;
    get_primary_managed_task(State(state), Path(task_key)).await
}

async fn run_primary_managed_task_now(
    State(state): State<PgRuntimeState>,
    Path(task_key): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let task = state
        .store
        .latest_materialized_rows("managed_tasks")
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(public_materialized_row)
        .find(|row| {
            row.get("taskKey").and_then(serde_json::Value::as_str) == Some(task_key.as_str())
        })
        .ok_or((StatusCode::NOT_FOUND, "managed task not found".to_string()))?;
    if pg_executable_primary_task(&task_key) {
        let executor = PostgresPrimaryMaintenanceExecutor::from_environment(state.store.clone());
        let result = match executor.execute(&task_key).await {
            Ok(result) => result,
            Err(error) => {
                let failure = serde_json::json!({
                    "status": "failed",
                    "error": error.to_string(),
                    "completedAt": chrono::Utc::now().to_rfc3339()
                });
                state
                    .store
                    .put_json(&format!("managed_task_last_result:{task_key}"), &failure)
                    .await
                    .map_err(internal_error)?;
                return Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()));
            }
        };
        state
            .store
            .put_json(
                &format!("managed_task_last_result:{task_key}"),
                &result.summary,
            )
            .await
            .map_err(internal_error)?;
        return get_primary_managed_task(State(state), Path(task_key)).await;
    }
    state
        .store
        .append_json_array(
            "managed_task_runtime_queue",
            serde_json::json!({
                "taskKey": task_key,
                "title": task.get("title").cloned().unwrap_or(serde_json::Value::Null),
                "triggerKind": "manual",
                "status": "queued",
                "queuedAt": chrono::Utc::now().to_rfc3339()
            }),
            100,
        )
        .await
        .map_err(internal_error)?;
    get_primary_managed_task(State(state), Path(task_key)).await
}

fn pg_executable_primary_task(task_key: &str) -> bool {
    if task_key.starts_with("startup_backfill.") {
        return true;
    }
    matches!(
        task_key,
        "prepare_performance_indexes"
            | "verify_archive_storage"
            | "archive_upstream_activity_manifest"
            | "materialize_historical_rollups"
            | "system_status_snapshot"
            | "summary_snapshot"
            | "dashboard_runtime_projection_reconcile"
            | "timeseries_minute_projection"
            | "long_term_projection"
            | "invocation_timeline_snapshot"
            | "raw_payload_metrics_inventory"
            | "upstream_account_maintenance"
            | "pool_orphan_recovery"
            | "summary_coverage_recovery"
            | "prompt_cache_materialization"
            | "startup_hourly_rollup_bootstrap"
            | "prune_archive_batches"
            | "prune_legacy_archive_batches"
            | "raw_compression"
            | "invocation_detail_prune"
            | "quota_snapshot_compaction"
            | "model_route_expiry"
            | "task_run_history_cleanup"
            | "pool_upstream_attempt_archive"
            | "forward_proxy_attempt_archive"
            | "forward_proxy_subscription_refresh"
            | "retention_archive"
            | "invocation_identity_cleanup"
            | "raw_orphan_sweep"
            | "startup_backfill"
            | "startup_backfill.pool_attempt_public_id_archives"
            | "startup_backfill.upstream_activity_archives"
            | "startup_backfill.pool_upstream_node_health_archives"
            | "startup_backfill.legacy_detail_mirrors"
            | "startup_backfill.historical_rollups"
            | "startup_backfill.prompt_cache_conversations_materialization"
    )
}

async fn verify_primary_asset_storage(
    state: &PgRuntimeState,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let base = std::env::var_os("DATABASE_PATH")
        .map(PathBuf::from)
        .and_then(|path| path.parent().map(FsPath::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let assets = state
        .store
        .latest_materialized_rows("asset_index")
        .await
        .map_err(internal_error)?;
    verify_primary_asset_rows(&base, &assets).await
}

async fn verify_primary_asset_rows(
    base: &FsPath,
    assets: &[serde_json::Value],
) -> Result<serde_json::Value, (StatusCode, String)> {
    let mut checked = 0u64;
    let mut total_bytes = 0u64;
    for asset in assets {
        let relative = asset
            .get("relativePath")
            .or_else(|| asset.get("relative_path"))
            .and_then(serde_json::Value::as_str)
            .ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                "asset index path is missing".to_string(),
            ))?;
        let relative_path = FsPath::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| component == std::path::Component::ParentDir)
        {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("asset path escapes storage root: {relative}"),
            ));
        }
        let path = base.join(relative_path);
        let metadata = tokio::fs::metadata(&path).await.map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("asset is unavailable: {relative}: {error}"),
            )
        })?;
        let expected_size = asset
            .get("sizeBytes")
            .or_else(|| asset.get("size_bytes"))
            .and_then(serde_json::Value::as_i64)
            .ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("asset size is missing: {relative}"),
            ))?;
        if expected_size < 0 || metadata.len() != expected_size as u64 {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("asset size mismatch: {relative}"),
            ));
        }
        let expected_sha = asset
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("asset checksum is missing: {relative}"),
            ))?;
        let bytes = tokio::fs::read(&path).await.map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("asset cannot be read: {relative}: {error}"),
            )
        })?;
        let actual_sha = format!("{:x}", sha2::Sha256::digest(&bytes));
        if actual_sha != expected_sha {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("asset checksum mismatch: {relative}"),
            ));
        }
        checked += 1;
        total_bytes = total_bytes.saturating_add(metadata.len());
    }
    Ok(serde_json::json!({
        "status": "succeeded",
        "checkedAssets": checked,
        "checkedBytes": total_bytes,
        "completedAt": chrono::Utc::now().to_rfc3339()
    }))
}

async fn get_primary_managed_task_runtime(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let queued_runs = state
        .store
        .get_json("managed_task_runtime_queue")
        .await
        .map_err(internal_error)?
        .filter(serde_json::Value::is_array)
        .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
    Ok(Json(serde_json::json!({
        "queuedRuns": queued_runs,
        "admissionWaits": [],
        "queuedRunsAvailable": true,
        "admissionWaitsAvailable": true
    })))
}

async fn get_primary_managed_task_timeline(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let items = state
        .store
        .latest_materialized_rows("managed_task_runs")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    let total = items.len();
    Ok(Json(serde_json::json!({"items": items, "total": total})))
}

async fn get_primary_failure_summary(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let rows = state
        .store
        .latest_materialized_rows("codex_invocations")
        .await
        .map_err(internal_error)?;
    let mut counts = std::collections::BTreeMap::<String, i64>::new();
    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        let status = object
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        if !matches!(
            status.as_str(),
            "ok" | "success" | "succeeded" | "completed"
        ) {
            *counts.entry(status).or_default() += 1;
        }
    }
    let items = counts
        .into_iter()
        .map(|(kind, count)| serde_json::json!({"kind": kind, "count": count}))
        .collect::<Vec<_>>();
    Ok(Json(serde_json::json!({"items": items})))
}

async fn get_primary_stats_errors_others(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_failure_summary(State(state)).await
}

async fn get_primary_activity_rows(
    state: PgRuntimeState,
    object_name: &'static str,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let items = state
        .store
        .latest_materialized_rows(object_name)
        .await
        .map_err(internal_error)?
        .into_iter()
        .map(public_materialized_row)
        .collect::<Vec<_>>();
    let total = items.len();
    Ok(Json(serde_json::json!({
        "items": items,
        "total": total,
        "available": true
    })))
}

async fn get_primary_dashboard_activity(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_activity_rows(state, "codex_invocations").await
}

async fn get_primary_dashboard_activity_recent(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_activity_rows(state, "codex_invocations").await
}

async fn get_primary_dashboard_network_recent(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_activity_rows(state, "pool_upstream_request_attempts").await
}

async fn get_primary_account_activity(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_activity_rows(state, "pool_upstream_request_attempts").await
}

async fn get_primary_forward_proxy_stats(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_activity_rows(state, "forward_proxy_nodes").await
}

async fn get_primary_parallel_work(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_activity_rows(state, "codex_invocations").await
}

async fn get_primary_invocation_timeline(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_activity_rows(state, "codex_invocations").await
}

async fn release_primary_invocation_timeline(Path(_as_of): Path<String>) -> StatusCode {
    // PG-only timeline snapshots are read directly from the materialized activity rows;
    // releasing the compatibility token is therefore an idempotent no-op.
    StatusCode::NO_CONTENT
}

async fn get_primary_prompt_cache_conversations(
    State(state): State<PgRuntimeState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    get_primary_activity_rows(state, "prompt_cache_conversations").await
}

fn public_primary_account_row(row: serde_json::Value) -> serde_json::Value {
    let mut object = row.as_object().cloned().unwrap_or_default();
    for sensitive in [
        "encrypted_credentials",
        "encryptedCredentials",
        "credentials",
        "access_token",
        "accessToken",
        "refresh_token",
        "refreshToken",
        "api_key",
        "apiKey",
        "_cvm_run_id",
        "_cvm_source_key",
        "_cvm_payload",
    ] {
        object.remove(sensitive);
    }
    let id = object
        .get("id")
        .cloned()
        .unwrap_or_else(|| serde_json::json!(0));
    let display_name = object
        .get("display_name")
        .or_else(|| object.get("displayName"))
        .cloned()
        .unwrap_or_else(|| {
            serde_json::json!(format!("account-{}", id.as_i64().unwrap_or_default()))
        });
    let kind = object
        .get("kind")
        .cloned()
        .unwrap_or_else(|| serde_json::json!("oauth"));
    let provider = object
        .get("provider")
        .cloned()
        .unwrap_or_else(|| serde_json::json!("codex"));
    let status = object
        .get("status")
        .cloned()
        .unwrap_or_else(|| serde_json::json!("unknown"));
    let enabled = object
        .get("enabled")
        .cloned()
        .unwrap_or(serde_json::json!(true));
    let has_refresh_token = object
        .get("has_refresh_token")
        .or_else(|| object.get("hasRefreshToken"))
        .cloned()
        .unwrap_or(serde_json::json!(false));
    let capability = serde_json::json!({
        "observed": "unknown",
        "override": null,
        "effective": "unknown",
        "observedAt": null,
        "reason": null
    });
    object.insert("id".to_owned(), id);
    object.insert("kind".to_owned(), kind);
    object.insert("provider".to_owned(), provider);
    object.insert("displayName".to_owned(), display_name);
    object.insert("status".to_owned(), status.clone());
    object.insert("displayStatus".to_owned(), status);
    object.insert("enabled".to_owned(), enabled);
    object.insert("hasRefreshToken".to_owned(), has_refresh_token);
    object
        .entry("isMother".to_owned())
        .or_insert(serde_json::json!(false));
    object
        .entry("workStatus".to_owned())
        .or_insert(serde_json::json!("idle"));
    object
        .entry("enableStatus".to_owned())
        .or_insert(serde_json::json!("enabled"));
    object
        .entry("healthStatus".to_owned())
        .or_insert(serde_json::json!("unknown"));
    object
        .entry("syncState".to_owned())
        .or_insert(serde_json::json!("unknown"));
    object
        .entry("groupName".to_owned())
        .or_insert(serde_json::Value::Null);
    object
        .entry("boundProxyKeys".to_owned())
        .or_insert(serde_json::json!([]));
    object
        .entry("currentForwardProxyState".to_owned())
        .or_insert(serde_json::json!("none"));
    object
        .entry("tags".to_owned())
        .or_insert(serde_json::json!([]));
    object
        .entry("effectiveRoutingRule".to_owned())
        .or_insert_with(|| serde_json::json!({}));
    for field in [
        "responseEndpointCapability",
        "chatCompletionsCapability",
        "imageEndpointCapability",
        "responseImageToolCapability",
        "codexImagegenCapability",
        "standaloneSearchCapability",
    ] {
        object
            .entry(field.to_owned())
            .or_insert_with(|| capability.clone());
    }
    object
        .entry("compactSupport".to_owned())
        .or_insert_with(|| serde_json::json!({"status":"unknown","observedAt":null,"reason":null}));
    object
        .entry("primaryWindow".to_owned())
        .or_insert(serde_json::Value::Null);
    object
        .entry("secondaryWindow".to_owned())
        .or_insert(serde_json::Value::Null);
    object
        .entry("credits".to_owned())
        .or_insert(serde_json::Value::Null);
    object
        .entry("reserve".to_owned())
        .or_insert(serde_json::Value::Null);
    object
        .entry("localLimits".to_owned())
        .or_insert(serde_json::Value::Null);
    serde_json::Value::Object(object)
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "databaseBackend": "postgres",
        "databaseVersion": "18",
    }))
}

async fn analytics_health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "backend": "postgres-primary",
        "analyticsBackend": "independent",
    }))
}

async fn get_versions() -> Json<crate::api::VersionResponse> {
    let (backend, frontend) = crate::forward_proxy::detect_versions(None);
    Json(crate::api::VersionResponse { backend, frontend })
}

async fn get_pricing(
    State(state): State<PgRuntimeState>,
) -> Result<Json<PricingSettingsResponse>, (StatusCode, String)> {
    let catalog = load_primary_pricing_catalog(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let quotes = load_primary_pricing_quotes(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    Ok(Json(
        PricingSettingsResponse::from_catalog(&catalog).with_quotes(quotes),
    ))
}

async fn get_settings(
    State(state): State<PgRuntimeState>,
) -> Result<Json<SettingsResponse>, (StatusCode, String)> {
    let catalog = load_primary_pricing_catalog(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let quotes = load_primary_pricing_quotes(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let proxy = crate::pricing::load_primary_proxy_model_settings(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let models = managed_models(state.store.as_ref()).await?;
    let forward_proxy = load_forward_proxy_settings(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    Ok(Json(SettingsResponse {
        proxy: ProxyModelSettingsResponse::from_settings_with_models(proxy, models),
        forward_proxy: forward_proxy_response(forward_proxy),
        pricing: PricingSettingsResponse::from_catalog(&catalog).with_quotes(quotes),
    }))
}

async fn put_pricing(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<PricingSettingsUpdateRequest>,
) -> Result<Json<PricingSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let (catalog, quotes) = payload.normalized_with_quotes()?;
    state
        .store
        .put_json_batch(&[
            (
                crate::pricing::PRIMARY_PRICING_STATE_KEY,
                serde_json::to_value(&catalog)
                    .map_err(anyhow::Error::from)
                    .map_err(internal_error)?,
            ),
            (
                crate::pricing::PRIMARY_PRICING_QUOTES_STATE_KEY,
                serde_json::to_value(quotes.as_deref().unwrap_or_default())
                    .map_err(anyhow::Error::from)
                    .map_err(internal_error)?,
            ),
        ])
        .await
        .map_err(internal_error)?;
    Ok(Json(
        PricingSettingsResponse::from_catalog(&catalog).with_quotes(quotes.unwrap_or_default()),
    ))
}

async fn get_forward_proxy(
    State(state): State<PgRuntimeState>,
) -> Result<Json<ForwardProxySettingsResponse>, (StatusCode, String)> {
    let settings = load_forward_proxy_settings(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    Ok(Json(forward_proxy_response(settings)))
}

async fn get_external_api_keys(
    State(state): State<PgRuntimeState>,
) -> Result<Json<crate::external_api::ExternalApiKeyListResponse>, (StatusCode, String)> {
    Ok(Json(
        list_primary_external_api_keys(state.store.as_ref()).await?,
    ))
}

async fn get_models_sync_state(
    State(state): State<PgRuntimeState>,
) -> Result<Json<crate::models_dev_sync_memory::ModelsDevSyncMemoryState>, (StatusCode, String)> {
    crate::models_dev_sync_memory::load_primary_models_dev_sync_memory(state.store.as_ref())
        .await
        .map(Json)
        .map_err(internal_error)
}

async fn patch_models_sync_state(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<crate::api::ModelsDevSyncMemoryPatch>,
) -> Result<Json<crate::models_dev_sync_memory::ModelsDevSyncMemoryState>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    crate::models_dev_sync_memory::patch_primary_models_dev_sync_memory(
        state.store.as_ref(),
        payload,
    )
    .await
    .map(Json)
    .map_err(internal_error)
}

async fn post_external_api_key(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<CreateExternalApiKeyRequest>,
) -> Result<Json<crate::external_api::ExternalApiKeySecretResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    Ok(Json(
        create_primary_external_api_key(state.store.as_ref(), payload).await?,
    ))
}

async fn rotate_external_api_key(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<crate::external_api::ExternalApiKeySecretResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    Ok(Json(
        rotate_primary_external_api_key(state.store.as_ref(), id).await?,
    ))
}

async fn disable_external_api_key(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<crate::external_api::ExternalApiKeyMutationResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    Ok(Json(
        disable_primary_external_api_key(state.store.as_ref(), id).await?,
    ))
}

async fn put_forward_proxy(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<ForwardProxySettingsUpdateRequest>,
) -> Result<Json<ForwardProxySettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let settings: ForwardProxySettings = payload.into();
    save_forward_proxy_settings(state.store.as_ref(), &settings)
        .await
        .map_err(internal_error)?;
    Ok(Json(forward_proxy_response(settings)))
}

async fn post_forward_proxy_validate(
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_owned(),
        ));
    }
    let value = payload
        .get("value")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "value is required".to_owned()))?;
    let parsed = Url::parse(value).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "forward proxy value must be a valid URL".to_owned(),
        )
    })?;
    if parsed.host_str().is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "forward proxy URL must include a host".to_owned(),
        ));
    }
    Ok(Json(serde_json::json!({
        "ok": true,
        "message": "forward proxy value is syntactically valid",
        "code": null,
        "normalizedValue": parsed.to_string(),
        "discoveredNodes": 0,
        "latencyMs": null
    })))
}

async fn post_forward_proxy_refresh(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_owned(),
        ));
    }
    let settings = load_forward_proxy_settings(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    Ok(Json(serde_json::json!({
        "forwardProxy": forward_proxy_response(settings),
        "subscriptionCount": 0,
        "addedNodeCount": 0,
        "refreshedAt": chrono::Utc::now().to_rfc3339()
    })))
}

async fn removed_primary_proxy_model_settings_endpoint() -> (StatusCode, &'static str) {
    (
        StatusCode::NOT_FOUND,
        "endpoint removed; legacy reverse proxy settings are no longer supported",
    )
}

async fn primary_latency_test_event(
    state: &PgRuntimeState,
    key: String,
) -> Result<Event, axum::Error> {
    let settings = load_forward_proxy_settings(state.store.as_ref())
        .await
        .unwrap_or_default();
    let index = key
        .strip_prefix("pg-node-")
        .and_then(|value| value.parse::<usize>().ok());
    let endpoint = index
        .and_then(|index| settings.proxy_urls.get(index))
        .cloned()
        .or_else(|| {
            settings
                .proxy_urls
                .iter()
                .find(|endpoint| stable_forward_proxy_binding_key(endpoint) == key)
                .cloned()
        });
    let (egress_ip, message) = if let Some(endpoint) = endpoint {
        let started = Instant::now();
        match reqwest::Proxy::all(&endpoint).ok().and_then(|proxy| {
            reqwest::Client::builder()
                .proxy(proxy)
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .ok()
        }) {
            Some(client) => match client.get("https://api.ipify.org?format=json").send().await {
                Ok(response) => {
                    let status = response.status().as_u16();
                    let ok = response.status().is_success();
                    let error = (!ok).then(|| format!("HTTP {status}"));
                    (
                        serde_json::json!({
                            "ok": ok,
                            "latencyMs": started.elapsed().as_secs_f64() * 1000.0,
                            "ip": null,
                            "httpStatus": status,
                            "error": error
                        }),
                        format!("probed {endpoint}"),
                    )
                }
                Err(error) => (
                    serde_json::json!({
                        "ok": false,
                        "latencyMs": started.elapsed().as_secs_f64() * 1000.0,
                        "ip": null,
                        "httpStatus": null,
                        "error": error.to_string()
                    }),
                    format!("probe failed for {endpoint}"),
                ),
            },
            None => (
                serde_json::json!({
                    "ok": false,
                    "latencyMs": null,
                    "ip": null,
                    "httpStatus": null,
                    "error": "forward proxy URL is invalid"
                }),
                format!("invalid proxy endpoint {endpoint}"),
            ),
        }
    } else {
        (
            serde_json::json!({
                "ok": false,
                "latencyMs": null,
                "ip": null,
                "httpStatus": null,
                "error": "forward proxy node was not found"
            }),
            "forward proxy node was not found".to_owned(),
        )
    };
    let all_targets_ok = egress_ip
        .get("ok")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    Event::default().event("completed").json_data(serde_json::json!({
        "kind": "completed",
        "node": {
            "key": key,
            "displayName": key,
            "round": 0,
            "totalRounds": 0,
            "completedRounds": 0,
            "successCount": 0,
            "attemptCount": 0,
            "averageLatencyMs": null,
            "egressIp": egress_ip,
            "oauthUpstream": egress_ip.clone(),
            "codexResponses": egress_ip,
            "allTargetsOk": all_targets_ok,
            "failedTargets": if all_targets_ok { Vec::<String>::new() } else { vec!["egress_ip".to_owned(), "oauth_upstream".to_owned(), "codex_responses".to_owned()] },
            "done": true,
            "timedOut": false,
            "message": message
        }
    }))
}

async fn get_primary_forward_proxy_node_latency_test(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Path(proxy_key): Path<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_owned(),
        ));
    }
    let event = primary_latency_test_event(&state, proxy_key)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(Sse::new(stream::once(async move { Ok(event) }))
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15))))
}

async fn get_primary_forward_proxy_nodes_latency_test(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_owned(),
        ));
    }
    let keys = url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
        .filter_map(|(key, value)| (key == "key").then(|| value.into_owned()))
        .collect::<Vec<_>>();
    let keys = if keys.is_empty() {
        vec!["__direct__".to_owned()]
    } else {
        keys
    };
    let events = stream::iter(keys).then(move |key| {
        let state = state.clone();
        async move {
            Ok(match primary_latency_test_event(&state, key).await {
                Ok(event) => event,
                Err(_) => Event::default().event("completed"),
            })
        }
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15))))
}

async fn get_proxy(
    State(state): State<PgRuntimeState>,
) -> Result<Json<ProxyModelSettingsResponse>, (StatusCode, String)> {
    let settings = crate::pricing::load_primary_proxy_model_settings(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let models = managed_models(state.store.as_ref()).await?;
    Ok(Json(ProxyModelSettingsResponse::from_settings_with_models(
        settings, models,
    )))
}

async fn put_proxy(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<ProxyModelSettingsUpdateRequest>,
) -> Result<Json<ProxyModelSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let current = crate::pricing::load_primary_proxy_model_settings(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let models = managed_models(state.store.as_ref()).await?;
    let allowed = models
        .iter()
        .map(String::as_str)
        .collect::<std::collections::HashSet<_>>();
    let settings = ProxyModelSettings {
        hijack_enabled: payload.hijack_enabled,
        merge_upstream_enabled: payload.merge_upstream_enabled,
        upstream_429_max_retries: payload
            .upstream_429_max_retries
            .unwrap_or(current.upstream_429_max_retries),
        request_body_logging_enabled: payload
            .request_body_logging_enabled
            .unwrap_or(current.request_body_logging_enabled),
        response_body_logging_enabled: payload
            .response_body_logging_enabled
            .unwrap_or(current.response_body_logging_enabled),
        encrypted_session_owner_routing_enabled: payload
            .encrypted_session_owner_routing_enabled
            .unwrap_or(current.encrypted_session_owner_routing_enabled),
        enabled_preset_models: payload
            .enabled_models
            .into_iter()
            .filter(|model| allowed.contains(model.as_str()))
            .collect(),
    }
    .normalized();
    crate::pricing::save_primary_proxy_model_settings(state.store.as_ref(), &settings)
        .await
        .map_err(internal_error)?;
    Ok(Json(ProxyModelSettingsResponse::from_settings_with_models(
        settings, models,
    )))
}

async fn put_managed_model_preset(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<ManagedModelPresetRequest>,
) -> Result<Json<ProxyModelSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let model = payload.model.trim().to_owned();
    if model.is_empty() || model.len() > 128 {
        return Err((StatusCode::BAD_REQUEST, "invalid model ID".to_string()));
    }
    let models = managed_models(state.store.as_ref()).await?;
    if !models.iter().any(|candidate| candidate == &model) {
        return Err((StatusCode::NOT_FOUND, "model was not found".to_string()));
    }
    let current = crate::pricing::load_primary_proxy_model_settings(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let mut enabled = current.enabled_preset_models;
    enabled.retain(|candidate| candidate != &model);
    if payload.enabled {
        enabled.push(model);
    }
    let next = ProxyModelSettings {
        enabled_preset_models: enabled,
        ..current
    }
    .normalized();
    crate::pricing::save_primary_proxy_model_settings(state.store.as_ref(), &next)
        .await
        .map_err(internal_error)?;
    Ok(Json(ProxyModelSettingsResponse::from_settings_with_models(
        next, models,
    )))
}

async fn delete_managed_model(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<ManagedModelDeleteRequest>,
) -> Result<Json<ManagedModelDeleteResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let model = payload.model.trim().to_owned();
    if model.is_empty() || model.len() > 128 {
        return Err((StatusCode::BAD_REQUEST, "invalid model ID".to_string()));
    }
    let mut catalog = load_primary_pricing_catalog(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let mut proxy = crate::pricing::load_primary_proxy_model_settings(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let price_deleted = catalog.models.remove(&model).is_some();
    let was_enabled = proxy
        .enabled_preset_models
        .iter()
        .any(|candidate| candidate == &model);
    if !price_deleted && !was_enabled {
        return Err((StatusCode::NOT_FOUND, "model was not found".to_string()));
    }
    proxy
        .enabled_preset_models
        .retain(|candidate| candidate != &model);
    proxy = proxy.normalized();
    crate::pricing::save_primary_pricing_catalog(state.store.as_ref(), &catalog)
        .await
        .map_err(internal_error)?;
    crate::pricing::save_primary_proxy_model_settings(state.store.as_ref(), &proxy)
        .await
        .map_err(internal_error)?;
    Ok(Json(ManagedModelDeleteResponse {
        deleted_model: model,
    }))
}

async fn post_models_sync_apply(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
    Json(payload): Json<ModelsDevSyncApplyRequest>,
) -> Result<Json<PricingSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    if payload.entries.is_empty() && payload.quotes.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "select at least one model price".to_string(),
        ));
    }
    let (normalized, normalized_quotes) = PricingSettingsUpdateRequest {
        catalog_version: "litellm".to_owned(),
        entries: payload.entries,
        quotes: Some(payload.quotes),
    }
    .normalized_with_quotes()?;
    let entries = normalized
        .models
        .iter()
        .map(|(model, pricing)| crate::app_state::PricingEntry {
            model: model.clone(),
            input_per_1m: pricing.input_per_1m,
            output_per_1m: pricing.output_per_1m,
            cache_input_per_1m: pricing.effective_cache_read_per_1m(),
            cache_read_per_1m: pricing.effective_cache_read_per_1m(),
            cache_write_per_1m: pricing.cache_write_per_1m,
            reasoning_per_1m: pricing.reasoning_per_1m,
            source: "litellm".to_owned(),
        })
        .collect::<Vec<_>>();
    let current = load_primary_pricing_catalog(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    let next = crate::pricing::merge_primary_pricing_entries(current, "litellm", &entries);
    save_primary_pricing_catalog(state.store.as_ref(), &next)
        .await
        .map_err(internal_error)?;
    save_primary_pricing_quotes(
        state.store.as_ref(),
        normalized_quotes.as_deref().unwrap_or_default(),
    )
    .await
    .map_err(internal_error)?;
    let quotes = load_primary_pricing_quotes(state.store.as_ref())
        .await
        .map_err(internal_error)?;
    Ok(Json(
        PricingSettingsResponse::from_catalog(&next).with_quotes(quotes),
    ))
}

async fn post_models_sync_preview(
    State(state): State<PgRuntimeState>,
    headers: HeaderMap,
) -> Result<Json<crate::api::ModelsDevSyncPreview>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let response = reqwest::Client::new()
        .get(crate::api::LITELLM_PRICE_CATALOG_URL)
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("LiteLLM price catalog retrieval failed: {error}"),
            )
        })?;
    if !response.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("LiteLLM price catalog returned HTTP {}", response.status()),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > crate::api::LITELLM_MAX_RESPONSE_BYTES as u64)
    {
        return Err((
            StatusCode::BAD_GATEWAY,
            "LiteLLM price catalog response exceeds the 32 MiB limit".to_string(),
        ));
    }
    let body = response.bytes().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("LiteLLM price catalog response read failed: {error}"),
        )
    })?;
    if body.len() > crate::api::LITELLM_MAX_RESPONSE_BYTES {
        return Err((
            StatusCode::BAD_GATEWAY,
            "LiteLLM price catalog response exceeds the 32 MiB limit".to_string(),
        ));
    }
    let payload: serde_json::Value = serde_json::from_slice(&body).map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("LiteLLM price catalog response JSON is invalid: {error}"),
        )
    })?;
    let source_version = format!("sha256:{:x}", sha2::Sha256::digest(&body));
    let mut preview =
        crate::api::parse_litellm_catalog(&payload, &source_version).map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("LiteLLM price catalog format is invalid: {error}"),
            )
        })?;
    preview.sync_state = crate::models_dev_sync_memory::record_primary_catalog_success(
        state.store.as_ref(),
        &preview.providers,
        &preview.candidates,
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(preview))
}

async fn managed_models(
    store: &dyn PrimaryStateStore,
) -> Result<Vec<String>, (StatusCode, String)> {
    let catalog = load_primary_pricing_catalog(store)
        .await
        .map_err(internal_error)?;
    let mut models = crate::PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|model| (*model).to_owned())
        .chain(catalog.models.into_keys())
        .collect::<Vec<_>>();
    models.sort();
    models.dedup();
    Ok(models)
}

fn forward_proxy_response(settings: ForwardProxySettings) -> ForwardProxySettingsResponse {
    let nodes = settings
        .proxy_urls
        .iter()
        .map(|endpoint_url| ForwardProxyNodeResponse {
            key: stable_forward_proxy_binding_key(endpoint_url),
            source: "primary-settings".to_owned(),
            display_name: endpoint_url.clone(),
            endpoint_url: Some(endpoint_url.clone()),
            weight: 1.0,
            penalized: false,
            stats: ForwardProxyStatsResponse::default(),
        })
        .collect();
    ForwardProxySettingsResponse {
        proxy_urls: settings.proxy_urls,
        subscription_urls: settings.subscription_urls,
        subscription_update_interval_secs: settings.subscription_update_interval_secs,
        nodes,
    }
}

async fn load_forward_proxy_settings(
    store: &dyn PrimaryStateStore,
) -> Result<ForwardProxySettings> {
    let Some(value) = store.get_json(PRIMARY_FORWARD_PROXY_SETTINGS_KEY).await? else {
        return Ok(ForwardProxySettings::default());
    };
    serde_json::from_value(value).context("failed to decode primary forward proxy settings")
}

async fn save_forward_proxy_settings(
    store: &dyn PrimaryStateStore,
    settings: &ForwardProxySettings,
) -> Result<()> {
    store
        .put_json(
            PRIMARY_FORWARD_PROXY_SETTINGS_KEY,
            &serde_json::to_value(settings.clone().normalized())?,
        )
        .await
        .context("failed to persist primary forward proxy settings")
}

fn internal_error(error: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[derive(Debug, Clone)]
struct PrimaryProxyTarget {
    url: Url,
    authorization: Option<String>,
    proxy_url: Option<String>,
}

async fn refresh_primary_oauth_if_expired(
    state: &PgRuntimeState,
    account_id: i64,
    credentials: StoredOauthCredentials,
) -> Result<StoredOauthCredentials> {
    let Some(expires_at) =
        crate::upstream_accounts::parse_access_token_expiry(&credentials.access_token)
    else {
        return Ok(credentials);
    };
    if expires_at > chrono::Utc::now() {
        return Ok(credentials);
    }
    let Some(refresh_token) = credentials
        .refresh_token
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        return Ok(credentials);
    };
    let issuer = std::env::var("UPSTREAM_ACCOUNTS_OAUTH_ISSUER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://auth.openai.com".to_owned());
    let client_id = std::env::var("UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .context("UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID is required for OAuth refresh")?;
    let token_url = Url::parse(&issuer)?.join("/oauth/token")?;
    let response = state
        .http_client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id.as_str()),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .context("failed to refresh PG primary OAuth token")?;
    if !response.status().is_success() {
        bail!(
            "PG primary OAuth refresh returned HTTP {}",
            response.status()
        );
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .context("failed to decode PG primary OAuth refresh response")?;
    let access_token = payload
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .context("PG primary OAuth refresh response omitted access_token")?;
    let mut refreshed = credentials;
    refreshed.access_token = access_token.to_owned();
    if let Some(refresh_token) = payload
        .get("refresh_token")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
    {
        refreshed.refresh_token = Some(refresh_token.to_owned());
    }
    if let Some(id_token) = payload
        .get("id_token")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
    {
        refreshed.id_token = id_token.to_owned();
    }
    if let Some(token_type) = payload
        .get("token_type")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
    {
        refreshed.token_type = Some(token_type.to_owned());
    }
    if let Some(crypto_key) = state.crypto_key.as_ref() {
        let encrypted =
            encrypt_credentials(crypto_key, &StoredCredentials::Oauth(refreshed.clone()))?;
        let _ = state
            .store
            .update_materialized_account(
                account_id,
                &serde_json::json!({"encrypted_credentials": encrypted}),
            )
            .await?;
    }
    Ok(refreshed)
}

fn primary_proxy_account_is_eligible(row: &serde_json::Value) -> bool {
    let enabled = row.get("enabled").is_none_or(|value| {
        value.as_i64().is_none_or(|enabled| enabled != 0)
            && value.as_bool().is_none_or(|enabled| enabled)
    });
    enabled
        && row
            .get("status")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|status| !matches!(status, "disabled" | "error" | "expired"))
        && row.get("deleted_at").is_none_or(serde_json::Value::is_null)
        && row
            .get("encrypted_credentials")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|credentials| !credentials.trim().is_empty())
}

fn primary_proxy_account_priority(row: &serde_json::Value) -> (u8, i64) {
    let tier = row
        .get("policy_priority_tier")
        .or_else(|| row.get("priority_tier"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("normal");
    let rank = match tier {
        "primary" => 0,
        "normal" => 1,
        "fallback" => 2,
        "no_new" => 3,
        _ => 1,
    };
    (
        rank,
        row.get("id")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(i64::MAX),
    )
}

fn primary_bound_proxy_url(
    row: &serde_json::Value,
    settings: &ForwardProxySettings,
) -> Option<String> {
    let keys = row
        .get("bound_proxy_keys")
        .or_else(|| row.get("boundProxyKeys"))
        .or_else(|| row.get("bound_proxy_keys_json"))
        .and_then(|value| {
            value.as_array().cloned().or_else(|| {
                value
                    .as_str()
                    .and_then(|encoded| serde_json::from_str::<Vec<String>>(encoded).ok())
                    .map(|items| items.into_iter().map(serde_json::Value::String).collect())
            })
        })
        .unwrap_or_default();
    keys.iter()
        .filter_map(serde_json::Value::as_str)
        .filter(|key| *key != "__direct__")
        .find_map(|key| {
            settings
                .proxy_urls
                .iter()
                .find(|endpoint| stable_forward_proxy_binding_key(endpoint) == key)
                .cloned()
        })
}

async fn resolve_primary_proxy_targets(
    state: &PgRuntimeState,
    original_uri: &Uri,
) -> Result<Vec<PrimaryProxyTarget>> {
    let accounts = state
        .store
        .latest_materialized_rows("pool_upstream_accounts")
        .await?;
    let mut candidates = accounts
        .into_iter()
        .filter(primary_proxy_account_is_eligible)
        .collect::<Vec<_>>();
    candidates.sort_by_key(primary_proxy_account_priority);
    let forward_proxy = load_forward_proxy_settings(state.store.as_ref())
        .await
        .unwrap_or_default();
    let mut targets = Vec::new();
    for row in candidates {
        let mut base_url = state.upstream_base_url.clone();
        if row.get("kind").and_then(serde_json::Value::as_str) == Some("oauth_codex") {
            base_url = oauth_codex_upstream_base_url()?;
        } else if let Some(value) = row
            .get("upstream_base_url")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            let Ok(url) = Url::parse(value) else {
                continue;
            };
            base_url = url;
        }
        let Some(crypto_key) = state.crypto_key.as_ref() else {
            continue;
        };
        let Some(encrypted) = row
            .get("encrypted_credentials")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Ok(credentials) = decrypt_credentials(crypto_key, encrypted) else {
            continue;
        };
        let credentials = match credentials {
            StoredCredentials::Oauth(credentials) => match refresh_primary_oauth_if_expired(
                state,
                row.get("id")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default(),
                credentials.clone(),
            )
            .await
            {
                Ok(refreshed) => StoredCredentials::Oauth(refreshed),
                Err(_) => StoredCredentials::Oauth(credentials),
            },
            other => other,
        };
        let authorization = match credentials {
            StoredCredentials::ApiKey(credentials) => {
                Some(format!("Bearer {}", credentials.api_key))
            }
            StoredCredentials::Oauth(credentials) => {
                let token_type = credentials
                    .token_type
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| "Bearer".to_owned());
                Some(format!("{token_type} {}", credentials.access_token))
            }
        };
        let Ok(url) = crate::proxy::build_proxy_upstream_url(&base_url, original_uri) else {
            continue;
        };
        targets.push(PrimaryProxyTarget {
            url,
            authorization,
            proxy_url: primary_bound_proxy_url(&row, &forward_proxy),
        });
    }
    if targets.is_empty() {
        targets.push(PrimaryProxyTarget {
            url: crate::proxy::build_proxy_upstream_url(&state.upstream_base_url, original_uri)?,
            authorization: None,
            proxy_url: None,
        });
    }
    Ok(targets)
}

async fn proxy_primary_v1(
    State(state): State<PgRuntimeState>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let started_at = Instant::now();
    let invoke_id = format!("pg-{}", nanoid::nanoid!());
    // Persist only the path; query strings can contain credentials or other user data.
    let request_path = original_uri.path().to_owned();
    let request_method = method.to_string();
    let targets = match resolve_primary_proxy_targets(&state, &original_uri).await {
        Ok(targets) => targets,
        Err(error) => {
            let _ = state
                .store
                .record_proxy_request(ProxyRequestRecord {
                    invoke_id,
                    method: request_method,
                    request_path,
                    status_code: Some(StatusCode::BAD_REQUEST.as_u16()),
                    request_bytes: 0,
                    response_bytes: 0,
                    duration_ms: started_at.elapsed().as_secs_f64() * 1000.0,
                    error: Some(error.to_string()),
                })
                .await;
            return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
        }
    };
    let body = match axum::body::to_bytes(body, state.max_request_body_bytes).await {
        Ok(body) => body,
        Err(error) => {
            let _ = state
                .store
                .record_proxy_request(ProxyRequestRecord {
                    invoke_id,
                    method: request_method,
                    request_path,
                    status_code: Some(StatusCode::PAYLOAD_TOO_LARGE.as_u16()),
                    request_bytes: 0,
                    response_bytes: 0,
                    duration_ms: started_at.elapsed().as_secs_f64() * 1000.0,
                    error: Some(error.to_string()),
                })
                .await;
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("request body exceeds configured limit: {error}"),
            )
                .into_response();
        }
    };
    let request_bytes = body.len();
    let mut outbound_headers = HeaderMap::new();
    for (name, value) in &headers {
        if matches!(
            name.as_str(),
            "host" | "content-length" | "connection" | "transfer-encoding"
        ) {
            continue;
        }
        outbound_headers.append(name.clone(), value.clone());
    }
    let mut last_error = None;
    for (index, target) in targets.iter().enumerate() {
        let mut request_headers = outbound_headers.clone();
        if let Some(authorization) = &target.authorization
            && let Ok(value) = HeaderValue::from_str(authorization)
        {
            request_headers.insert(axum::http::header::AUTHORIZATION, value);
        }
        let client = match target.proxy_url.as_deref() {
            Some(proxy_url) => match reqwest::Proxy::all(proxy_url).ok().and_then(|proxy| {
                reqwest::Client::builder()
                    .proxy(proxy)
                    .timeout(std::time::Duration::from_secs(60))
                    .build()
                    .ok()
            }) {
                Some(client) => client,
                None => {
                    last_error = Some("bound forward proxy URL is invalid".to_owned());
                    continue;
                }
            },
            None => state.http_client.clone(),
        };
        let response = match client
            .request(method.clone(), target.url.clone())
            .headers(request_headers)
            .body(body.clone())
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        };
        let response_status = response.status();
        let retryable = response_status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || response.status().is_server_error();
        let mut response_headers = HeaderMap::new();
        for (name, value) in response.headers() {
            if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) {
                response_headers.append(name.clone(), value);
            }
        }
        if retryable && index + 1 < targets.len() {
            if let Err(error) = response.bytes().await {
                last_error = Some(error.to_string());
            }
            continue;
        }
        let status =
            StatusCode::from_u16(response_status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(16);
        let store = state.store.clone();
        let record_method = request_method.clone();
        let record_path = request_path.clone();
        let record_invoke_id = invoke_id.clone();
        tokio::spawn(async move {
            let mut response_bytes = 0usize;
            let mut stream_error = None;
            let mut body_stream = response.bytes_stream();
            while let Some(chunk) = body_stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        response_bytes = response_bytes.saturating_add(bytes.len());
                        if tx.send(Ok(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let message = error.to_string();
                        stream_error = Some(message.clone());
                        let _ = tx.send(Err(io::Error::other(message))).await;
                        break;
                    }
                }
            }
            if let Err(error) = store
                .record_proxy_request(ProxyRequestRecord {
                    invoke_id: record_invoke_id,
                    method: record_method,
                    request_path: record_path,
                    status_code: Some(status.as_u16()),
                    request_bytes,
                    response_bytes,
                    duration_ms: started_at.elapsed().as_secs_f64() * 1000.0,
                    error: stream_error,
                })
                .await
            {
                error!(
                    ?error,
                    "failed to persist PostgreSQL primary proxy request metadata"
                );
            }
        });
        let mut outgoing = Response::new(Body::from_stream(ReceiverStream::new(rx)));
        *outgoing.status_mut() = status;
        *outgoing.headers_mut() = response_headers;
        return outgoing;
    }
    let error = last_error.unwrap_or_else(|| "all primary upstream targets failed".to_owned());
    let _ = state
        .store
        .record_proxy_request(ProxyRequestRecord {
            invoke_id,
            method: request_method,
            request_path,
            status_code: Some(StatusCode::BAD_GATEWAY.as_u16()),
            request_bytes,
            response_bytes: 0,
            duration_ms: started_at.elapsed().as_secs_f64() * 1000.0,
            error: Some(error.clone()),
        })
        .await;
    (
        StatusCode::BAD_GATEWAY,
        format!("primary upstream request failed: {error}"),
    )
        .into_response()
}

pub(crate) async fn run(config: &AppConfig) -> Result<()> {
    let database = PrimaryDatabase::connect_runtime(config)
        .await
        .context("PostgreSQL primary runtime validation failed")?;
    let store: Arc<dyn PrimaryStateStore> = Arc::new(PostgresPrimaryStateStore::new(database));
    store
        .put_json(
            "core.retention_policy.v1",
            &serde_json::json!({
                "invocationMaxDays": config.invocation_max_days,
                "invocationArchiveTtlDays": config.invocation_archive_ttl_days,
                "quotaSnapshotFullDays": config.quota_snapshot_full_days,
                "poolUpstreamRequestAttemptsRetentionDays": config.pool_upstream_request_attempts_retention_days,
                "forwardProxyAttemptsRetentionDays": config.forward_proxy_attempts_retention_days,
                "updatedAt": chrono::Utc::now().to_rfc3339()
            }),
        )
        .await
        .context("failed to persist PostgreSQL primary retention policy")?;
    let state = PgRuntimeState {
        store,
        crypto_key: crate::upstream_accounts::UpstreamAccountsRuntime::from_env()?.crypto_key,
        upstream_base_url: config.openai_upstream_base_url.clone(),
        http_client: reqwest::Client::builder()
            .timeout(config.request_timeout)
            .build()
            .context("failed to build PostgreSQL primary upstream client")?,
        max_request_body_bytes: config.openai_proxy_max_request_body_bytes,
    };
    let mut router = Router::new()
        .route("/health", get(health))
        .route("/v1/{*path}", any(proxy_primary_v1))
        .route("/health/analytics", get(analytics_health))
        .route("/api/version", get(get_versions))
        .route("/api/invocations", get(get_primary_invocations))
        .route(
            "/api/invocations/locate",
            get(get_primary_invocation_locate),
        )
        .route(
            "/api/invocations/{invocation_id}/pool-attempts",
            get(get_primary_invocation_pool_attempts),
        )
        .route(
            "/api/invocations/suggestions",
            get(get_primary_invocation_suggestions),
        )
        .route(
            "/api/invocations/new-count",
            get(get_primary_invocation_new_count),
        )
        .route(
            "/api/invocations/summary",
            get(get_primary_invocation_summary),
        )
        .route(
            "/api/invocations/{invocation_id}/detail",
            get(get_primary_invocation_detail),
        )
        .route(
            "/api/invocations/{invocation_id}/workflow-detail",
            get(get_primary_invocation_workflow_detail),
        )
        .route(
            "/api/invocations/{invocation_id}/response-body",
            get(get_primary_invocation_response_body),
        )
        .route(
            "/api/invocations/{invocation_id}/request-body",
            get(get_primary_invocation_request_body),
        )
        .route("/api/stats", get(get_primary_stats))
        .route(
            "/api/stats/dashboard-activity",
            get(get_primary_dashboard_activity),
        )
        .route(
            "/api/stats/dashboard-activity/recent",
            get(get_primary_dashboard_activity_recent),
        )
        .route(
            "/api/stats/dashboard-network-timeseries",
            get(get_primary_stats_timeseries),
        )
        .route(
            "/api/stats/dashboard-network-recent",
            get(get_primary_dashboard_network_recent),
        )
        .route(
            "/api/stats/upstream-account-activity",
            get(get_primary_account_activity),
        )
        .route(
            "/api/stats/forward-proxy",
            get(get_primary_forward_proxy_stats),
        )
        .route(
            "/api/stats/forward-proxy/timeseries",
            get(get_primary_stats_timeseries),
        )
        .route("/api/stats/parallel-work", get(get_primary_parallel_work))
        .route(
            "/api/stats/invocation-timeline",
            get(get_primary_invocation_timeline),
        )
        .route(
            "/api/stats/invocation-timeline/snapshot",
            get(get_primary_invocation_timeline),
        )
        .route(
            "/api/stats/invocation-timeline/{as_of}",
            axum::routing::delete(release_primary_invocation_timeline),
        )
        .route(
            "/api/stats/prompt-cache-conversations",
            get(get_primary_prompt_cache_conversations),
        )
        .route(
            "/api/stats/long-term/overview",
            get(get_primary_stats_long_term_overview),
        )
        .route("/api/stats/summary", get(get_primary_invocation_summary))
        .route("/api/stats/timeseries", get(get_primary_stats_timeseries))
        .route(
            "/api/stats/long-term/series",
            get(get_primary_stats_timeseries),
        )
        .route("/api/stats/errors", get(get_primary_failure_summary))
        .route(
            "/api/stats/errors/others",
            get(get_primary_stats_errors_others),
        )
        .route("/api/stats/perf", get(get_primary_stats_perf))
        .route(
            "/api/stats/failures/summary",
            get(get_primary_failure_summary),
        )
        .route("/api/quota/latest", get(get_primary_latest_quota))
        .route("/api/system/status", get(get_primary_system_status))
        .route("/api/system/storage", get(get_primary_system_storage))
        .route("/api/system/tasks", get(get_primary_system_tasks))
        .route("/api/system/managed-tasks", get(get_primary_managed_tasks))
        .route(
            "/api/system/managed-tasks/{task_key}",
            get(get_primary_managed_task).patch(update_primary_managed_task),
        )
        .route(
            "/api/system/managed-tasks/{task_key}/run",
            axum::routing::post(run_primary_managed_task_now),
        )
        .route(
            "/api/system/managed-tasks/runtime",
            get(get_primary_managed_task_runtime),
        )
        .route(
            "/api/system/managed-tasks/timeline",
            get(get_primary_managed_task_timeline),
        )
        .route("/api/system/observability", get(get_primary_observability))
        .route(
            "/api/system/observability/browser",
            get(get_primary_observability),
        )
        .route("/api/system/performance", get(get_primary_observability))
        .route("/api/settings", get(get_settings))
        .route(
            "/api/settings/models/preset",
            axum::routing::put(put_managed_model_preset),
        )
        .route(
            "/api/settings/models",
            axum::routing::delete(delete_managed_model),
        )
        .route(
            "/api/settings/proxy-models",
            any(removed_primary_proxy_model_settings_endpoint),
        )
        .route(
            "/api/settings/models/sync/state",
            get(get_models_sync_state).patch(patch_models_sync_state),
        )
        .route(
            "/api/settings/models/sync/apply",
            axum::routing::post(post_models_sync_apply),
        )
        .route(
            "/api/settings/models/sync/preview",
            axum::routing::post(post_models_sync_preview),
        )
        .route(
            "/api/settings/external-api-keys",
            get(get_external_api_keys).post(post_external_api_key),
        )
        .route(
            "/api/settings/external-api-keys/{id}/rotate",
            axum::routing::post(rotate_external_api_key),
        )
        .route(
            "/api/settings/external-api-keys/{id}/disable",
            axum::routing::post(disable_external_api_key),
        )
        .route("/api/settings/pricing", get(get_pricing).put(put_pricing))
        .route(
            "/api/settings/forward-proxy",
            get(get_forward_proxy).put(put_forward_proxy),
        )
        .route(
            "/api/settings/forward-proxy/validate",
            axum::routing::post(post_forward_proxy_validate),
        )
        .route(
            "/api/settings/forward-proxy/refresh-subscriptions",
            axum::routing::post(post_forward_proxy_refresh),
        )
        .route(
            "/api/settings/forward-proxy/nodes/{proxy_key}/test-stream",
            get(get_primary_forward_proxy_node_latency_test),
        )
        .route(
            "/api/settings/forward-proxy/nodes/test-stream",
            get(get_primary_forward_proxy_nodes_latency_test),
        )
        .route("/api/settings/proxy", get(get_proxy).put(put_proxy))
        .route(
            "/api/pool/routing-settings",
            get(get_pool_routing_settings).put(put_pool_routing_settings),
        )
        .route(
            "/api/pool/upstream-accounts",
            get(get_primary_upstream_accounts).post(bulk_update_primary_upstream_accounts),
        )
        .route(
            "/api/pool/upstream-accounts/api-keys",
            axum::routing::post(create_primary_api_key_account),
        )
        .route(
            "/api/pool/upstream-account-events",
            get(get_primary_upstream_account_events),
        )
        .route(
            "/api/pool/forward-proxy-binding-nodes",
            get(get_primary_forward_proxy_binding_nodes),
        )
        .route(
            "/api/pool/upstream-accounts/{account_id}/call-attempts",
            get(get_primary_account_call_attempts),
        )
        .route(
            "/api/pool/upstream-accounts/{account_id}",
            get(get_primary_upstream_account)
                .patch(update_primary_upstream_account)
                .delete(delete_primary_upstream_account),
        )
        .route("/api/pool/tags", get(get_primary_pool_tags))
        .route(
            "/api/pool/model-routing-live",
            get(get_primary_model_routing_live),
        )
        .with_state(state);
    if let Some(static_dir) = config.static_dir.clone() {
        router = router.fallback_service(ServeDir::new(static_dir));
    }
    router = router.layer(crate::proxy::build_cors_layer(config));
    let listener = TcpListener::bind(&config.http_bind)
        .await
        .context("failed to bind PG runtime HTTP listener")?;
    let address = listener
        .local_addr()
        .context("failed to determine PG runtime HTTP address")?;
    info!(%address, "PostgreSQL primary runtime listening");
    let server: JoinHandle<()> = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
        {
            error!(%error, "PostgreSQL primary runtime exited with error");
        }
    });
    server
        .await
        .context("PostgreSQL primary runtime task failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::PricingEntry;
    use crate::primary_store::SqlitePrimaryStateStore;
    use sha2::Sha256;

    #[test]
    fn primary_proxy_account_eligibility_rejects_disabled_and_deleted_rows() {
        let base = serde_json::json!({
            "enabled": true,
            "status": "active",
            "encrypted_credentials": "ciphertext"
        });
        assert!(primary_proxy_account_is_eligible(&base));

        let mut disabled = base.clone();
        disabled["enabled"] = serde_json::json!(false);
        assert!(!primary_proxy_account_is_eligible(&disabled));

        let mut deleted = base.clone();
        deleted["deleted_at"] = serde_json::json!("2026-01-01T00:00:00Z");
        assert!(!primary_proxy_account_is_eligible(&deleted));
    }

    #[test]
    fn primary_proxy_account_priority_orders_tiers_before_account_id() {
        let primary = serde_json::json!({"id": 99, "priority_tier": "primary"});
        let normal = serde_json::json!({"id": 1, "priority_tier": "normal"});
        let fallback = serde_json::json!({"id": 0, "priority_tier": "fallback"});
        assert!(primary_proxy_account_priority(&primary) < primary_proxy_account_priority(&normal));
        assert!(
            primary_proxy_account_priority(&normal) < primary_proxy_account_priority(&fallback)
        );

        let policy_override = serde_json::json!({
            "id": 100,
            "priority_tier": "primary",
            "policy_priority_tier": "fallback"
        });
        assert_eq!(primary_proxy_account_priority(&policy_override).0, 2);
    }

    #[test]
    fn primary_bound_proxy_url_resolves_stable_binding_keys() {
        let endpoint = "https://proxy.example.test:8443".to_owned();
        let settings = ForwardProxySettings {
            proxy_urls: vec![endpoint.clone()],
            ..ForwardProxySettings::default()
        };
        let row = serde_json::json!({
            "bound_proxy_keys_json": serde_json::to_string(&vec![stable_forward_proxy_binding_key(&endpoint)]).unwrap()
        });
        assert_eq!(primary_bound_proxy_url(&row, &settings), Some(endpoint));
        assert!(
            primary_bound_proxy_url(
                &serde_json::json!({"bound_proxy_keys": ["__direct__"]}),
                &settings
            )
            .is_none()
        );
    }

    #[test]
    fn primary_stats_numeric_adapters_accept_migrated_text_values() {
        let tokens = serde_json::json!("123");
        let cost = serde_json::json!("4.25");
        assert_eq!(primary_json_i64(Some(&tokens)), 123);
        assert!((primary_json_f64(Some(&cost)) - 4.25).abs() < f64::EPSILON);
        assert_eq!(primary_json_i64(Some(&serde_json::json!(null))), 0);
    }

    #[tokio::test]
    async fn primary_asset_storage_verification_checks_size_checksum_and_paths() {
        let base = std::env::temp_dir().join(format!(
            "cvm-pg-asset-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        tokio::fs::create_dir_all(base.join("archives"))
            .await
            .unwrap();
        let content = b"asset-content";
        tokio::fs::write(base.join("archives/item.bin"), content)
            .await
            .unwrap();
        let checksum = format!("{:x}", sha2::Sha256::digest(content));
        let assets = vec![serde_json::json!({
            "relative_path": "archives/item.bin",
            "size_bytes": content.len(),
            "sha256": checksum
        })];
        let result = verify_primary_asset_rows(&base, &assets).await.unwrap();
        assert_eq!(result["checkedAssets"], serde_json::json!(1));
        assert_eq!(result["checkedBytes"], serde_json::json!(content.len()));

        let escaped = vec![serde_json::json!({
            "relative_path": "../outside.bin",
            "size_bytes": 1,
            "sha256": "00"
        })];
        let error = verify_primary_asset_rows(&base, &escaped)
            .await
            .expect_err("asset traversal must be rejected");
        assert_eq!(error.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(error.1.contains("escapes storage root"));
        tokio::fs::remove_dir_all(base).await.unwrap();
    }

    #[tokio::test]
    async fn pricing_http_handlers_round_trip_through_primary_store() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let state = PgRuntimeState {
            store: Arc::new(store),
            crypto_key: None,
            upstream_base_url: Url::parse("http://127.0.0.1:9").unwrap(),
            http_client: reqwest::Client::new(),
            max_request_body_bytes: 1024 * 1024,
        };
        let payload = PricingSettingsUpdateRequest {
            catalog_version: "test-v1".to_owned(),
            entries: vec![PricingEntry {
                model: "example".to_owned(),
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "test".to_owned(),
            }],
            quotes: None,
        };
        let response = put_pricing(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .unwrap()
            .0;
        assert_eq!(response.catalog_version, "test-v1");
        assert_eq!(response.entries.len(), 1);
        let loaded = get_pricing(State(state)).await.unwrap().0;
        assert_eq!(loaded.entries[0].model, "example");
        assert_eq!(loaded.entries[0].output_per_1m, 2.0);
    }

    #[tokio::test]
    async fn proxy_settings_handler_round_trips_through_primary_store() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let state = PgRuntimeState {
            store: Arc::new(store),
            crypto_key: None,
            upstream_base_url: Url::parse("http://127.0.0.1:9").unwrap(),
            http_client: reqwest::Client::new(),
            max_request_body_bytes: 1024 * 1024,
        };
        let payload = ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(2),
            request_body_logging_enabled: Some(false),
            response_body_logging_enabled: Some(true),
            encrypted_session_owner_routing_enabled: Some(false),
            enabled_models: vec![crate::PROXY_PRESET_MODEL_IDS[0].to_owned()],
        };
        let response = put_proxy(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .unwrap()
            .0;
        assert!(response.hijack_enabled);
        assert_eq!(response.upstream_429_max_retries, 2);
        let loaded = get_proxy(State(state)).await.unwrap().0;
        assert_eq!(loaded.enabled_models, response.enabled_models);
    }

    #[tokio::test]
    async fn pool_routing_settings_handler_round_trips_through_primary_store() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let state = PgRuntimeState {
            store: Arc::new(store),
            crypto_key: None,
            upstream_base_url: Url::parse("http://127.0.0.1:9").unwrap(),
            http_client: reqwest::Client::new(),
            max_request_body_bytes: 1024 * 1024,
        };
        let initial = get_pool_routing_settings(State(state.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(initial["writesEnabled"], true);
        let updated = put_pool_routing_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(serde_json::json!({"availableModels": ["gpt-test"], "automaticReserveFallbackEnabled": false})),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(updated["availableModels"], serde_json::json!(["gpt-test"]));
        assert_eq!(updated["automaticReserveFallbackEnabled"], false);
        assert_eq!(
            get_pool_routing_settings(State(state)).await.unwrap().0["availableModels"],
            serde_json::json!(["gpt-test"])
        );
    }

    #[tokio::test]
    async fn pool_routing_api_key_is_encrypted_and_never_returned() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let state = PgRuntimeState {
            store: Arc::new(store),
            crypto_key: Some(Sha256::digest(b"pool-secret").into()),
            upstream_base_url: Url::parse("http://127.0.0.1:9").unwrap(),
            http_client: reqwest::Client::new(),
            max_request_body_bytes: 1024 * 1024,
        };
        let response = put_pool_routing_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(serde_json::json!({"apiKey": "sk-live-secret"})),
        )
        .await
        .unwrap()
        .0;
        let encoded = serde_json::to_string(&response).unwrap();
        assert!(!encoded.contains("sk-live-secret"));
        assert!(!encoded.contains("_encryptedApiKey"));
        let stored = state
            .store
            .get_json(PRIMARY_POOL_ROUTING_SETTINGS_KEY)
            .await
            .unwrap()
            .unwrap();
        assert!(stored["_encryptedApiKey"].as_str().is_some());
    }

    #[tokio::test]
    async fn combined_settings_handler_reads_all_primary_state_sections() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let state = PgRuntimeState {
            store: Arc::new(store),
            crypto_key: None,
            upstream_base_url: Url::parse("http://127.0.0.1:9").unwrap(),
            http_client: reqwest::Client::new(),
            max_request_body_bytes: 1024 * 1024,
        };
        let response = get_settings(State(state)).await.unwrap().0;
        assert!(!response.pricing.catalog_version.is_empty());
        assert!(!response.proxy.models.is_empty());
        assert!(response.forward_proxy.proxy_urls.is_empty());
    }

    #[tokio::test]
    async fn settings_writes_reject_explicit_cross_site_browser_requests() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let state = PgRuntimeState {
            store: Arc::new(store),
            crypto_key: None,
            upstream_base_url: Url::parse("http://127.0.0.1:9").unwrap(),
            http_client: reqwest::Client::new(),
            max_request_body_bytes: 1024 * 1024,
        };
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        let payload = PricingSettingsUpdateRequest {
            catalog_version: "test".to_owned(),
            entries: Vec::new(),
            quotes: None,
        };
        let error = put_pricing(State(state), headers, Json(payload))
            .await
            .expect_err("cross-site settings write should be rejected");
        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn forward_proxy_validation_accepts_urls_and_rejects_invalid_values() {
        let response = post_forward_proxy_validate(
            HeaderMap::new(),
            Json(serde_json::json!({"value": "https://proxy.example.test:8443"})),
        )
        .await
        .expect("valid proxy URL should be accepted")
        .0;
        assert_eq!(response["ok"], serde_json::json!(true));
        assert_eq!(
            response["normalizedValue"],
            serde_json::json!("https://proxy.example.test:8443/")
        );

        let error = post_forward_proxy_validate(
            HeaderMap::new(),
            Json(serde_json::json!({"value": "not a URL"})),
        )
        .await
        .expect_err("invalid proxy URL should be rejected");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn primary_forward_proxy_node_keys_are_stable_across_reordering() {
        let first = forward_proxy_response(ForwardProxySettings {
            proxy_urls: vec![
                "https://proxy-a.example.test:8443".to_owned(),
                "https://proxy-b.example.test:8443".to_owned(),
            ],
            ..ForwardProxySettings::default()
        });
        let second = forward_proxy_response(ForwardProxySettings {
            proxy_urls: vec![
                "https://proxy-b.example.test:8443".to_owned(),
                "https://proxy-a.example.test:8443".to_owned(),
            ],
            ..ForwardProxySettings::default()
        });
        let key_for = |response: &ForwardProxySettingsResponse, endpoint: &str| {
            response
                .nodes
                .iter()
                .find(|node| node.endpoint_url.as_deref() == Some(endpoint))
                .map(|node| node.key.clone())
                .expect("endpoint should be represented")
        };
        assert_eq!(
            key_for(&first, "https://proxy-a.example.test:8443"),
            key_for(&second, "https://proxy-a.example.test:8443")
        );
        assert_eq!(
            key_for(&first, "https://proxy-b.example.test:8443"),
            key_for(&second, "https://proxy-b.example.test:8443")
        );
    }

    #[tokio::test]
    async fn managed_model_preset_and_delete_round_trip_through_primary_store() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let state = PgRuntimeState {
            store: Arc::new(store),
            crypto_key: None,
            upstream_base_url: Url::parse("http://127.0.0.1:9").unwrap(),
            http_client: reqwest::Client::new(),
            max_request_body_bytes: 1024 * 1024,
        };
        let mut catalog = load_primary_pricing_catalog(state.store.as_ref())
            .await
            .unwrap();
        catalog.models.insert(
            "pg-test-model".to_owned(),
            crate::app_state::ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "test".to_owned(),
            },
        );
        save_primary_pricing_catalog(state.store.as_ref(), &catalog)
            .await
            .unwrap();
        let preset = put_managed_model_preset(
            State(state.clone()),
            HeaderMap::new(),
            Json(ManagedModelPresetRequest {
                model: "pg-test-model".to_owned(),
                enabled: true,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(
            preset
                .enabled_models
                .iter()
                .any(|model| model == "pg-test-model")
        );
        let deleted = delete_managed_model(
            State(state),
            HeaderMap::new(),
            Json(ManagedModelDeleteRequest {
                model: "pg-test-model".to_owned(),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(deleted.deleted_model, "pg-test-model");
    }

    #[tokio::test]
    async fn primary_managed_task_control_overrides_migrated_snapshot() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let state = PgRuntimeState {
            store: Arc::new(store),
            crypto_key: None,
            upstream_base_url: Url::parse("http://127.0.0.1:9").unwrap(),
            http_client: reqwest::Client::new(),
            max_request_body_bytes: 1024 * 1024,
        };
        state
            .store
            .put_json(
                "managed_task_override:retention_archive",
                &serde_json::json!({"enabled": false, "intervalSecs": 120}),
            )
            .await
            .unwrap();
        let value = primary_managed_task_value(
            &state,
            serde_json::json!({
                "taskKey": "retention_archive",
                "enabled": true,
                "intervalSecs": 900
            }),
        )
        .await
        .unwrap();
        assert_eq!(value["enabled"], serde_json::json!(false));
        assert_eq!(value["intervalSecs"], serde_json::json!(120));
        assert_eq!(
            value["executionObservation"],
            serde_json::json!("pg_executable")
        );
    }

    #[tokio::test]
    async fn models_sync_apply_merges_pricing_into_primary_store() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let state = PgRuntimeState {
            store: Arc::new(store),
            crypto_key: None,
            upstream_base_url: Url::parse("http://127.0.0.1:9").unwrap(),
            http_client: reqwest::Client::new(),
            max_request_body_bytes: 1024 * 1024,
        };
        let response = post_models_sync_apply(
            State(state.clone()),
            HeaderMap::new(),
            Json(ModelsDevSyncApplyRequest {
                entries: vec![crate::app_state::PricingEntry {
                    model: "pg-sync-model".to_owned(),
                    input_per_1m: 1.0,
                    output_per_1m: 2.0,
                    cache_input_per_1m: None,
                    cache_read_per_1m: None,
                    cache_write_per_1m: None,
                    reasoning_per_1m: None,
                    source: "models.dev".to_owned(),
                }],
                quotes: Vec::new(),
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(
            response
                .entries
                .iter()
                .any(|entry| entry.model == "pg-sync-model")
        );
        let catalog = load_primary_pricing_catalog(state.store.as_ref())
            .await
            .unwrap();
        assert!(catalog.models.contains_key("pg-sync-model"));
    }

    #[tokio::test]
    async fn pg_runtime_exposes_version_and_analytics_health_metadata() {
        let version = get_versions().await.0;
        assert!(!version.backend.is_empty());
        assert!(!version.frontend.is_empty());

        let health = analytics_health().await.into_response();
        assert_eq!(health.status(), StatusCode::OK);
    }

    #[test]
    fn public_materialized_rows_use_api_names_and_drop_secrets() {
        let row = serde_json::json!({
            "invoke_id": "invoke-1",
            "total_tokens": 42,
            "encrypted_credentials": "secret",
            "_cvm_payload": {"private": true}
        });
        let public = public_materialized_row(row);
        assert_eq!(public["invokeId"], "invoke-1");
        assert_eq!(public["totalTokens"], 42);
        assert!(public.get("encryptedCredentials").is_none());
        assert!(public.get("_cvm_payload").is_none());
    }
}
