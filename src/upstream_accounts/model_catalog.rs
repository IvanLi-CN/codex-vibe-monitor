use super::*;

use std::collections::HashMap;

const MODEL_CATALOG_STATUS_NEVER: &str = "never";
const MODEL_CATALOG_STATUS_REFRESHING: &str = "refreshing";
const MODEL_CATALOG_STATUS_READY: &str = "ready";
const MODEL_CATALOG_STATUS_STALE: &str = "stale";
const MODEL_CATALOG_STATUS_FAILED: &str = "failed";
const MODEL_CATALOG_STALE_AFTER: ChronoDuration = ChronoDuration::hours(24);
const MODEL_CATALOG_MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MODEL_CATALOG_RETRY_ATTEMPTS: usize = 2;

static MODEL_CATALOG_LOCKS: Lazy<std::sync::Mutex<HashMap<i64, Arc<Mutex<()>>>>> =
    Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

#[derive(Debug, FromRow)]
struct ModelCatalogRow {
    models_json: String,
    status: String,
    last_attempted_at: Option<String>,
    last_successful_at: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
}

#[derive(Debug)]
struct CatalogRefreshFailure {
    status: StatusCode,
    code: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct UpstreamModelsPayload {
    data: Vec<UpstreamModelRecord>,
}

#[derive(Debug, Deserialize)]
struct UpstreamModelRecord {
    id: Option<String>,
}

pub(crate) async fn load_upstream_account_model_catalog(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<UpstreamAccountModelCatalog> {
    let Some(row) = sqlx::query_as::<_, ModelCatalogRow>(
        r#"
        SELECT models_json, status, last_attempted_at, last_successful_at,
               error_code, error_message
        FROM pool_upstream_account_model_catalogs
        WHERE account_id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(empty_model_catalog());
    };

    build_model_catalog(row)
}

pub(crate) async fn refresh_upstream_account_models(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }

    let detail = refresh_upstream_account_models_inner(state, id).await?;
    Ok(Json(detail))
}

async fn refresh_upstream_account_models_inner(
    state: Arc<AppState>,
    account_id: i64,
) -> Result<UpstreamAccountDetail, (StatusCode, String)> {
    let lock = account_model_catalog_lock(account_id);
    let _guard = lock.lock().await;

    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    if !matches!(
        row.kind.as_str(),
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX | UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX
    ) {
        return Err((
            StatusCode::BAD_REQUEST,
            "this account kind does not support model discovery".to_string(),
        ));
    }

    let attempted_at = format_utc_iso(Utc::now());
    mark_model_catalog_refreshing(&state.pool, account_id, &attempted_at)
        .await
        .map_err(internal_error_tuple)?;

    let discovery = discover_account_models(&state, &row).await;
    match discovery {
        Ok(models) => {
            persist_model_catalog_success(&state.pool, account_id, &models, &attempted_at)
                .await
                .map_err(internal_error_tuple)?;
        }
        Err(failure) => {
            persist_model_catalog_failure(
                &state.pool,
                account_id,
                &attempted_at,
                &failure.code,
                &failure.message,
            )
            .await
            .map_err(internal_error_tuple)?;
            return Err((failure.status, failure.message));
        }
    }

    load_upstream_account_detail_with_actual_usage(&state, account_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))
}

fn account_model_catalog_lock(account_id: i64) -> Arc<Mutex<()>> {
    let mut locks = MODEL_CATALOG_LOCKS
        .lock()
        .expect("model catalog lock registry is not poisoned");
    locks
        .entry(account_id)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn empty_model_catalog() -> UpstreamAccountModelCatalog {
    UpstreamAccountModelCatalog {
        models: Vec::new(),
        status: MODEL_CATALOG_STATUS_NEVER.to_string(),
        last_attempted_at: None,
        last_successful_at: None,
        error: None,
        stale: false,
    }
}

fn build_model_catalog(row: ModelCatalogRow) -> Result<UpstreamAccountModelCatalog> {
    let mut models = serde_json::from_str::<Vec<String>>(&row.models_json)
        .context("failed to decode upstream account model catalog")?;
    models = normalize_model_ids(models);
    let stale = row
        .last_successful_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .is_some_and(|timestamp| {
            Utc::now().signed_duration_since(timestamp) > MODEL_CATALOG_STALE_AFTER
        });
    let status = if row.status == MODEL_CATALOG_STATUS_READY && stale {
        MODEL_CATALOG_STATUS_STALE.to_string()
    } else {
        row.status
    };
    let error = row
        .error_code
        .zip(row.error_message)
        .map(|(code, message)| UpstreamAccountModelCatalogError { code, message });
    Ok(UpstreamAccountModelCatalog {
        models,
        status,
        last_attempted_at: row.last_attempted_at,
        last_successful_at: row.last_successful_at,
        error,
        stale,
    })
}

pub(crate) fn normalize_model_ids(models: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    models
        .into_iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty() && seen.insert(model.clone()))
        .collect()
}

async fn mark_model_catalog_refreshing(
    pool: &Pool<Sqlite>,
    account_id: i64,
    attempted_at: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_model_catalogs
            (account_id, models_json, status, last_attempted_at, updated_at)
        VALUES (?1, '[]', ?2, ?3, ?3)
        ON CONFLICT(account_id) DO UPDATE SET
            status = excluded.status,
            last_attempted_at = excluded.last_attempted_at,
            error_code = NULL,
            error_message = NULL,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(account_id)
    .bind(MODEL_CATALOG_STATUS_REFRESHING)
    .bind(attempted_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn persist_model_catalog_success(
    pool: &Pool<Sqlite>,
    account_id: i64,
    models: &[String],
    timestamp: &str,
) -> Result<()> {
    let models_json = serde_json::to_string(&normalize_model_ids(models.to_vec()))?;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_model_catalogs
            (account_id, models_json, status, last_attempted_at, last_successful_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?4, ?4)
        ON CONFLICT(account_id) DO UPDATE SET
            models_json = excluded.models_json,
            status = excluded.status,
            last_attempted_at = excluded.last_attempted_at,
            last_successful_at = excluded.last_successful_at,
            error_code = NULL,
            error_message = NULL,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(account_id)
    .bind(models_json)
    .bind(MODEL_CATALOG_STATUS_READY)
    .bind(timestamp)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn persist_model_catalog_failure(
    pool: &Pool<Sqlite>,
    account_id: i64,
    timestamp: &str,
    code: &str,
    message: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_model_catalogs
            (account_id, models_json, status, last_attempted_at, error_code, error_message, updated_at)
        VALUES (?1, '[]', ?2, ?3, ?4, ?5, ?3)
        ON CONFLICT(account_id) DO UPDATE SET
            status = excluded.status,
            last_attempted_at = excluded.last_attempted_at,
            error_code = excluded.error_code,
            error_message = excluded.error_message,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(account_id)
    .bind(MODEL_CATALOG_STATUS_FAILED)
    .bind(timestamp)
    .bind(code)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

async fn discover_account_models(
    state: &AppState,
    row: &UpstreamAccountRow,
) -> std::result::Result<Vec<String>, CatalogRefreshFailure> {
    let crypto_key = state
        .upstream_accounts
        .require_crypto_key()
        .map_err(|(_, message)| CatalogRefreshFailure {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "credentials_unavailable".to_string(),
            message,
        })?;
    let credentials = decrypt_credentials(
        crypto_key,
        row.encrypted_credentials
            .as_deref()
            .ok_or_else(|| CatalogRefreshFailure {
                status: StatusCode::BAD_GATEWAY,
                code: "credentials_missing".to_string(),
                message: "The account credentials are unavailable.".to_string(),
            })?,
    )
    .map_err(|_| CatalogRefreshFailure {
        status: StatusCode::BAD_GATEWAY,
        code: "credentials_invalid".to_string(),
        message: "The account credentials could not be read.".to_string(),
    })?;

    let scope = resolve_account_forward_proxy_scope(state, row, None)
        .await
        .map_err(|err| CatalogRefreshFailure {
            status: StatusCode::BAD_GATEWAY,
            code: "forward_proxy_unavailable".to_string(),
            message: safe_proxy_error_message(&err),
        })?;
    let base_url =
        resolve_pool_account_upstream_base_url(row, &state.config.openai_upstream_base_url)
            .map_err(|_| CatalogRefreshFailure {
                status: StatusCode::BAD_GATEWAY,
                code: "upstream_url_invalid".to_string(),
                message: "The account upstream URL is invalid.".to_string(),
            })?;
    let target_url =
        build_catalog_url(row.kind.as_str(), base_url).map_err(|_| CatalogRefreshFailure {
            status: StatusCode::BAD_GATEWAY,
            code: "upstream_url_invalid".to_string(),
            message: "The account upstream URL is invalid.".to_string(),
        })?;
    let (authorization, chatgpt_account_id) = match credentials {
        StoredCredentials::ApiKey(credentials)
            if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX =>
        {
            (format!("Bearer {}", credentials.api_key), None)
        }
        StoredCredentials::Oauth(credentials) if row.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX => (
            format!("Bearer {}", credentials.access_token),
            row.chatgpt_account_id.clone(),
        ),
        _ => {
            return Err(CatalogRefreshFailure {
                status: StatusCode::BAD_GATEWAY,
                code: "credentials_kind_mismatch".to_string(),
                message: "The account credentials do not match its provider kind.".to_string(),
            });
        }
    };

    let mut last_failure = None;
    for _attempt in 0..MODEL_CATALOG_RETRY_ATTEMPTS {
        let selected_proxy = match crate::select_forward_proxy_for_scope(state, &scope).await {
            Ok(proxy) => proxy,
            Err(err) => {
                last_failure = Some(CatalogRefreshFailure {
                    status: StatusCode::BAD_GATEWAY,
                    code: "forward_proxy_unavailable".to_string(),
                    message: safe_proxy_error_message(&err),
                });
                continue;
            }
        };
        let client = match state
            .http_clients
            .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        {
            Ok(client) => client,
            Err(_) => {
                last_failure = Some(CatalogRefreshFailure {
                    status: StatusCode::BAD_GATEWAY,
                    code: "forward_proxy_unavailable".to_string(),
                    message: "The forward proxy client could not be initialized.".to_string(),
                });
                continue;
            }
        };
        let mut request = client
            .get(target_url.clone())
            .header(header::AUTHORIZATION, authorization.clone());
        if row.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
            request = request.header("OpenAI-Beta", "responses=experimental");
            if let Some(account_id) = chatgpt_account_id.as_deref() {
                request = request.header("chatgpt-account-id", account_id);
            }
        }
        let response =
            match timeout(state.config.openai_proxy_handshake_timeout, request.send()).await {
                Ok(Ok(response)) => response,
                Ok(Err(_)) | Err(_) => {
                    last_failure = Some(CatalogRefreshFailure {
                        status: StatusCode::BAD_GATEWAY,
                        code: "upstream_unavailable".to_string(),
                        message: "The upstream model catalog could not be reached.".to_string(),
                    });
                    continue;
                }
            };
        let status = response.status();
        if !status.is_success() {
            last_failure = Some(CatalogRefreshFailure {
                status: if status.is_client_error() {
                    StatusCode::BAD_GATEWAY
                } else {
                    status
                },
                code: format!("upstream_http_{}", status.as_u16()),
                message: format!("The upstream returned HTTP {}.", status.as_u16()),
            });
            if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                continue;
            }
            break;
        }
        let Some(content_length) = response.content_length() else {
            return parse_catalog_response(response).await;
        };
        if content_length > MODEL_CATALOG_MAX_RESPONSE_BYTES as u64 {
            return Err(CatalogRefreshFailure {
                status: StatusCode::BAD_GATEWAY,
                code: "upstream_response_too_large".to_string(),
                message: "The upstream model catalog response is too large.".to_string(),
            });
        }
        return parse_catalog_response(response).await;
    }
    Err(last_failure.unwrap_or(CatalogRefreshFailure {
        status: StatusCode::BAD_GATEWAY,
        code: "upstream_unavailable".to_string(),
        message: "The upstream model catalog could not be reached.".to_string(),
    }))
}

fn build_catalog_url(kind: &str, mut base_url: Url) -> Result<Url> {
    if kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        base_url.set_path(&format!("{}/models", base_url.path().trim_end_matches('/')));
        base_url.set_query(Some(&format!(
            "client_version={}",
            crate::oauth_bridge::OAUTH_CODEX_MODELS_CLIENT_VERSION
        )));
        return Ok(base_url);
    }
    crate::proxy::build_proxy_upstream_url(&base_url, &Uri::from_static("/v1/models"))
}

async fn parse_catalog_response(
    response: reqwest::Response,
) -> std::result::Result<Vec<String>, CatalogRefreshFailure> {
    let bytes = timeout(Duration::from_secs(10), response.bytes())
        .await
        .map_err(|_| CatalogRefreshFailure {
            status: StatusCode::BAD_GATEWAY,
            code: "upstream_response_timeout".to_string(),
            message: "The upstream model catalog response timed out.".to_string(),
        })?
        .map_err(|_| CatalogRefreshFailure {
            status: StatusCode::BAD_GATEWAY,
            code: "upstream_response_invalid".to_string(),
            message: "The upstream model catalog response could not be read.".to_string(),
        })?;
    if bytes.len() > MODEL_CATALOG_MAX_RESPONSE_BYTES {
        return Err(CatalogRefreshFailure {
            status: StatusCode::BAD_GATEWAY,
            code: "upstream_response_too_large".to_string(),
            message: "The upstream model catalog response is too large.".to_string(),
        });
    }
    let payload = serde_json::from_slice::<UpstreamModelsPayload>(&bytes).map_err(|_| {
        CatalogRefreshFailure {
            status: StatusCode::BAD_GATEWAY,
            code: "upstream_response_invalid".to_string(),
            message: "The upstream model catalog response is invalid.".to_string(),
        }
    })?;
    Ok(normalize_model_ids(
        payload
            .data
            .into_iter()
            .filter_map(|record| record.id)
            .collect(),
    ))
}

fn safe_proxy_error_message(error: &anyhow::Error) -> String {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("bound forward proxy") || message.contains("forward proxy") {
        "The account forward proxy is unavailable.".to_string()
    } else {
        "The account forward proxy could not be selected.".to_string()
    }
}
