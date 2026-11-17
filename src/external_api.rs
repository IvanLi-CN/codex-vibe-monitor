use super::*;
use crate::primary_store::PrimaryStateStore;
use axum::extract::Path as AxumPath;
use rand::RngCore;

const PRIMARY_EXTERNAL_API_KEYS_KEY: &str = "core.external_api_keys.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrimaryExternalApiKeyRecord {
    id: i64,
    client_id: String,
    name: String,
    secret_hash: String,
    secret_prefix: String,
    status: String,
    last_used_at: Option<String>,
    created_at: String,
    updated_at: String,
    rotated_from_key_id: Option<i64>,
}

async fn load_primary_external_api_keys(
    store: &dyn PrimaryStateStore,
) -> Result<Vec<PrimaryExternalApiKeyRecord>> {
    let Some(value) = store.get_json(PRIMARY_EXTERNAL_API_KEYS_KEY).await? else {
        return Ok(Vec::new());
    };
    serde_json::from_value(value).context("invalid primary external API key state")
}

pub(crate) async fn list_primary_external_api_keys(
    store: &dyn PrimaryStateStore,
) -> Result<ExternalApiKeyListResponse, (StatusCode, String)> {
    let records = load_primary_external_api_keys(store)
        .await
        .map_err(internal_error_tuple_local)?;
    Ok(ExternalApiKeyListResponse {
        items: records
            .iter()
            .filter(|record| record.status != EXTERNAL_API_KEY_STATUS_ROTATED)
            .map(primary_external_api_key_summary)
            .collect(),
    })
}

async fn save_primary_external_api_keys(
    store: &dyn PrimaryStateStore,
    records: &[PrimaryExternalApiKeyRecord],
) -> Result<()> {
    store
        .put_json(
            PRIMARY_EXTERNAL_API_KEYS_KEY,
            &serde_json::to_value(records)?,
        )
        .await
}

fn primary_external_api_key_summary(record: &PrimaryExternalApiKeyRecord) -> ExternalApiKeySummary {
    ExternalApiKeySummary {
        id: record.id,
        name: record.name.clone(),
        status: record.status.clone(),
        prefix: record.secret_prefix.clone(),
        last_used_at: record.last_used_at.clone(),
        created_at: record.created_at.clone(),
        updated_at: record.updated_at.clone(),
    }
}

pub(crate) const EXTERNAL_API_KEY_STATUS_ACTIVE: &str = "active";
pub(crate) const EXTERNAL_API_KEY_STATUS_DISABLED: &str = "disabled";
pub(crate) const EXTERNAL_API_KEY_STATUS_ROTATED: &str = "rotated";
pub(crate) const EXTERNAL_API_KEY_SECRET_PREFIX_LEN: usize = 12;
pub(crate) const EXTERNAL_API_KEY_NAME_MAX_LEN: usize = 80;
pub(crate) const EXTERNAL_API_KEY_CLIENT_ID_HEX_LEN: usize = 16;
pub(crate) const EXTERNAL_API_KEY_CREATE_MAX_ATTEMPTS: usize = 8;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ExternalApiKeyRow {
    id: i64,
    client_id: String,
    name: String,
    secret_hash: String,
    secret_prefix: String,
    status: String,
    last_used_at: Option<String>,
    created_at: String,
    updated_at: String,
    rotated_from_key_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalApiKeySummary {
    id: i64,
    name: String,
    status: String,
    prefix: String,
    last_used_at: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<ExternalApiKeyRow> for ExternalApiKeySummary {
    fn from(value: ExternalApiKeyRow) -> Self {
        Self {
            id: value.id,
            name: value.name,
            status: value.status,
            prefix: value.secret_prefix,
            last_used_at: value.last_used_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalApiKeyListResponse {
    items: Vec<ExternalApiKeySummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalApiKeyMutationResponse {
    key: ExternalApiKeySummary,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExternalApiKeySecretResponse {
    key: ExternalApiKeySummary,
    secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateExternalApiKeyRequest {
    pub(crate) name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ExternalApiPrincipal {
    client_id: String,
}

pub(crate) fn hash_external_api_key_secret(secret: &str) -> String {
    let digest = Sha256::digest(secret.as_bytes());
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

pub(crate) fn normalize_optional_text_local(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn random_hex_local(size: usize) -> Result<String, (StatusCode, String)> {
    let byte_len = size.div_ceil(2);
    let mut bytes = vec![0_u8; byte_len];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut output = String::with_capacity(byte_len * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output.truncate(size);
    Ok(output)
}

pub(crate) fn internal_error_tuple_local(err: impl ToString) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        err.to_string().trim().to_string(),
    )
}

pub(crate) fn normalize_external_api_key_name(raw: &str) -> Result<String, (StatusCode, String)> {
    let Some(name) = normalize_optional_text_local(Some(raw.to_string())) else {
        return Err((StatusCode::BAD_REQUEST, "name is required".to_string()));
    };
    if name.len() > EXTERNAL_API_KEY_NAME_MAX_LEN {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("name must not exceed {EXTERNAL_API_KEY_NAME_MAX_LEN} characters"),
        ));
    }
    Ok(name)
}

pub(crate) fn build_external_api_key_secret(
    client_id: &str,
) -> Result<(String, String, String), (StatusCode, String)> {
    let short_client_id = client_id
        .strip_prefix("external_client_")
        .unwrap_or(client_id)
        .chars()
        .take(8)
        .collect::<String>();
    let public_prefix_nonce = random_hex_local(4)?;
    let secret = format!(
        "cvm_ext_{public_prefix_nonce}_{short_client_id}_{}",
        random_hex_local(20)?
    );
    let prefix = secret
        .chars()
        .take(EXTERNAL_API_KEY_SECRET_PREFIX_LEN)
        .collect::<String>();
    let secret_hash = hash_external_api_key_secret(&secret);
    Ok((secret, secret_hash, prefix))
}

pub(crate) fn is_external_api_key_uniqueness_error(err: &sqlx::Error) -> bool {
    let message = err.to_string();
    message.contains("UNIQUE constraint failed: external_api_keys.client_id")
        || message.contains("UNIQUE constraint failed: external_api_keys.secret_hash")
}

pub(crate) async fn list_external_api_key_rows(
    pool: &Pool<Sqlite>,
) -> Result<Vec<ExternalApiKeyRow>> {
    sqlx::query_as::<_, ExternalApiKeyRow>(
        r#"
        SELECT
            id,
            client_id,
            name,
            secret_hash,
            secret_prefix,
            status,
            last_used_at,
            created_at,
            updated_at,
            rotated_from_key_id
        FROM external_api_keys
        WHERE status != ?1
        ORDER BY created_at ASC, id ASC
        "#,
    )
    .bind(EXTERNAL_API_KEY_STATUS_ROTATED)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_external_api_key_row(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<ExternalApiKeyRow>> {
    sqlx::query_as::<_, ExternalApiKeyRow>(
        r#"
        SELECT
            id,
            client_id,
            name,
            secret_hash,
            secret_prefix,
            status,
            last_used_at,
            created_at,
            updated_at,
            rotated_from_key_id
        FROM external_api_keys
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_external_api_key_row_by_secret_hash(
    pool: &Pool<Sqlite>,
    secret_hash: &str,
) -> Result<Option<ExternalApiKeyRow>> {
    sqlx::query_as::<_, ExternalApiKeyRow>(
        r#"
        SELECT
            id,
            client_id,
            name,
            secret_hash,
            secret_prefix,
            status,
            last_used_at,
            created_at,
            updated_at,
            rotated_from_key_id
        FROM external_api_keys
        WHERE secret_hash = ?1
        LIMIT 1
        "#,
    )
    .bind(secret_hash)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn ensure_external_api_key_name_available(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    name: &str,
    exclude_id: Option<i64>,
) -> Result<(), (StatusCode, String)> {
    let conflict = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM external_api_keys
        WHERE status != ?1
          AND lower(trim(name)) = lower(trim(?2))
          AND (?3 IS NULL OR id != ?3)
        ORDER BY id ASC
        LIMIT 1
        "#,
    )
    .bind(EXTERNAL_API_KEY_STATUS_ROTATED)
    .bind(name)
    .bind(exclude_id)
    .fetch_optional(executor)
    .await
    .map_err(internal_error_tuple_local)?;
    if conflict.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "external API key name must be unique".to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn create_external_api_key_inner(
    state: Arc<AppState>,
    payload: CreateExternalApiKeyRequest,
) -> Result<ExternalApiKeySecretResponse, (StatusCode, String)> {
    let name = normalize_external_api_key_name(&payload.name)?;
    for _ in 0..EXTERNAL_API_KEY_CREATE_MAX_ATTEMPTS {
        let now_iso = format_utc_iso(Utc::now());
        let client_id = format!(
            "external_client_{}",
            random_hex_local(EXTERNAL_API_KEY_CLIENT_ID_HEX_LEN)?
        );
        let (secret, secret_hash, secret_prefix) = build_external_api_key_secret(&client_id)?;
        let mut tx = state
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(internal_error_tuple_local)?;
        ensure_external_api_key_name_available(tx.as_mut(), &name, None).await?;
        let inserted_id = match sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO external_api_keys (
                client_id,
                name,
                secret_hash,
                secret_prefix,
                status,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
            RETURNING id
            "#,
        )
        .bind(&client_id)
        .bind(&name)
        .bind(&secret_hash)
        .bind(&secret_prefix)
        .bind(EXTERNAL_API_KEY_STATUS_ACTIVE)
        .bind(&now_iso)
        .fetch_one(tx.as_mut())
        .await
        {
            Ok(value) => value,
            Err(err) if is_external_api_key_uniqueness_error(&err) => continue,
            Err(err) => return Err(internal_error_tuple_local(err)),
        };
        tx.commit().await.map_err(internal_error_tuple_local)?;
        let row = load_external_api_key_row(&state.pool, inserted_id)
            .await
            .map_err(internal_error_tuple_local)?
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    "external API key not found after creation".to_string(),
                )
            })?;
        return Ok(ExternalApiKeySecretResponse {
            key: row.into(),
            secret,
        });
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "failed to allocate unique external API key".to_string(),
    ))
}

pub(crate) async fn rotate_external_api_key_inner(
    state: Arc<AppState>,
    id: i64,
) -> Result<ExternalApiKeySecretResponse, (StatusCode, String)> {
    let existing = load_external_api_key_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple_local)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "external API key not found".to_string(),
            )
        })?;
    if existing.status == EXTERNAL_API_KEY_STATUS_ROTATED {
        return Err((
            StatusCode::CONFLICT,
            "rotated external API keys cannot be rotated again".to_string(),
        ));
    }
    let now_iso = format_utc_iso(Utc::now());
    let (secret, secret_hash, secret_prefix) = build_external_api_key_secret(&existing.client_id)?;
    let row = {
        let mut tx = state
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(internal_error_tuple_local)?;
        let current_status = sqlx::query_scalar::<_, String>(
            r#"
            SELECT status
            FROM external_api_keys
            WHERE id = ?1
            LIMIT 1
            "#,
        )
        .bind(existing.id)
        .fetch_optional(tx.as_mut())
        .await
        .map_err(internal_error_tuple_local)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "external API key not found".to_string(),
            )
        })?;
        if current_status == EXTERNAL_API_KEY_STATUS_ROTATED {
            return Err((
                StatusCode::CONFLICT,
                "external API key was already rotated".to_string(),
            ));
        }
        ensure_external_api_key_name_available(tx.as_mut(), &existing.name, Some(existing.id))
            .await?;
        sqlx::query(
            r#"
            UPDATE external_api_keys
            SET status = ?2,
                updated_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(existing.id)
        .bind(EXTERNAL_API_KEY_STATUS_ROTATED)
        .bind(&now_iso)
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple_local)?;
        let inserted_id = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO external_api_keys (
                client_id,
                name,
                secret_hash,
                secret_prefix,
                status,
                created_at,
                updated_at,
                rotated_from_key_id
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)
            RETURNING id
            "#,
        )
        .bind(&existing.client_id)
        .bind(&existing.name)
        .bind(&secret_hash)
        .bind(&secret_prefix)
        .bind(EXTERNAL_API_KEY_STATUS_ACTIVE)
        .bind(&now_iso)
        .bind(existing.id)
        .fetch_one(tx.as_mut())
        .await
        .map_err(|err| {
            if is_external_api_key_uniqueness_error(&err) {
                (
                    StatusCode::CONFLICT,
                    "external API key was already rotated".to_string(),
                )
            } else {
                internal_error_tuple_local(err)
            }
        })?;
        tx.commit().await.map_err(internal_error_tuple_local)?;
        load_external_api_key_row(&state.pool, inserted_id)
            .await
            .map_err(internal_error_tuple_local)?
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    "external API key not found after rotation".to_string(),
                )
            })?
    };
    Ok(ExternalApiKeySecretResponse {
        key: row.into(),
        secret,
    })
}

pub(crate) async fn disable_external_api_key_inner(
    state: Arc<AppState>,
    id: i64,
) -> Result<ExternalApiKeyMutationResponse, (StatusCode, String)> {
    let existing = load_external_api_key_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple_local)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "external API key not found".to_string(),
            )
        })?;
    if existing.status == EXTERNAL_API_KEY_STATUS_ROTATED {
        return Err((
            StatusCode::CONFLICT,
            "rotated external API keys cannot be disabled".to_string(),
        ));
    }
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE external_api_keys
        SET status = ?2,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(id)
    .bind(EXTERNAL_API_KEY_STATUS_DISABLED)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .map_err(internal_error_tuple_local)?;
    let updated = load_external_api_key_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple_local)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "external API key not found".to_string(),
            )
        })?;
    Ok(ExternalApiKeyMutationResponse {
        key: updated.into(),
    })
}

pub(crate) fn bearer_token_from_headers(headers: &HeaderMap) -> Option<&str> {
    let header = headers.get(header::AUTHORIZATION)?;
    let raw = header.to_str().ok()?;
    let (scheme, token) = raw.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    Some(token)
}

pub(crate) fn require_browser_same_origin_settings_write(
    headers: &HeaderMap,
) -> Result<(), (StatusCode, String)> {
    if headers.get(header::ORIGIN).is_none() {
        return Err((
            StatusCode::FORBIDDEN,
            "external API key writes require browser same-origin authentication".to_string(),
        ));
    }
    if !is_same_origin_settings_write(headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn authenticate_external_api_key(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ExternalApiPrincipal, (StatusCode, String)> {
    let Some(secret) = bearer_token_from_headers(headers) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "missing or invalid external API key".to_string(),
        ));
    };
    let secret_hash = hash_external_api_key_secret(secret);
    if state.config.primary_database_backend == PrimaryDatabaseBackend::Postgres {
        let mut records = load_primary_external_api_keys(state.primary_state_store.as_ref())
            .await
            .map_err(internal_error_tuple_local)?;
        let Some(record) = records
            .iter_mut()
            .find(|record| record.secret_hash == secret_hash)
        else {
            return Err((
                StatusCode::UNAUTHORIZED,
                "missing or invalid external API key".to_string(),
            ));
        };
        if record.status != EXTERNAL_API_KEY_STATUS_ACTIVE {
            return Err((
                StatusCode::FORBIDDEN,
                "external API key is disabled".to_string(),
            ));
        }
        record.last_used_at = Some(format_utc_iso(Utc::now()));
        record.updated_at = record.last_used_at.clone().unwrap_or_default();
        let client_id = record.client_id.clone();
        save_primary_external_api_keys(state.primary_state_store.as_ref(), &records)
            .await
            .map_err(internal_error_tuple_local)?;
        return Ok(ExternalApiPrincipal { client_id });
    }
    let row = load_external_api_key_row_by_secret_hash(&state.pool, &secret_hash)
        .await
        .map_err(internal_error_tuple_local)?
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                "missing or invalid external API key".to_string(),
            )
        })?;
    if row.status != EXTERNAL_API_KEY_STATUS_ACTIVE {
        return Err((
            StatusCode::FORBIDDEN,
            "external API key is disabled".to_string(),
        ));
    }
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE external_api_keys
        SET last_used_at = ?2
        WHERE id = ?1
        "#,
    )
    .bind(row.id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .map_err(internal_error_tuple_local)?;
    Ok(ExternalApiPrincipal {
        client_id: row.client_id,
    })
}

pub(crate) async fn create_primary_external_api_key(
    store: &dyn PrimaryStateStore,
    payload: CreateExternalApiKeyRequest,
) -> Result<ExternalApiKeySecretResponse, (StatusCode, String)> {
    let name = normalize_external_api_key_name(&payload.name)?;
    let mut records = load_primary_external_api_keys(store)
        .await
        .map_err(internal_error_tuple_local)?;
    if records.iter().any(|record| {
        record.status != EXTERNAL_API_KEY_STATUS_ROTATED && record.name.eq_ignore_ascii_case(&name)
    }) {
        return Err((
            StatusCode::CONFLICT,
            "external API key name must be unique".to_string(),
        ));
    }
    let id = records.iter().map(|record| record.id).max().unwrap_or(0) + 1;
    let client_id = format!(
        "external_client_{}",
        random_hex_local(EXTERNAL_API_KEY_CLIENT_ID_HEX_LEN)?
    );
    let (secret, secret_hash, secret_prefix) = build_external_api_key_secret(&client_id)?;
    let now = format_utc_iso(Utc::now());
    let record = PrimaryExternalApiKeyRecord {
        id,
        client_id,
        name,
        secret_hash,
        secret_prefix,
        status: EXTERNAL_API_KEY_STATUS_ACTIVE.to_string(),
        last_used_at: None,
        created_at: now.clone(),
        updated_at: now,
        rotated_from_key_id: None,
    };
    let response = ExternalApiKeySecretResponse {
        key: primary_external_api_key_summary(&record),
        secret,
    };
    records.push(record);
    save_primary_external_api_keys(store, &records)
        .await
        .map_err(internal_error_tuple_local)?;
    Ok(response)
}

pub(crate) async fn rotate_primary_external_api_key(
    store: &dyn PrimaryStateStore,
    id: i64,
) -> Result<ExternalApiKeySecretResponse, (StatusCode, String)> {
    let mut records = load_primary_external_api_keys(store)
        .await
        .map_err(internal_error_tuple_local)?;
    let Some(existing) = records.iter_mut().find(|record| record.id == id) else {
        return Err((
            StatusCode::NOT_FOUND,
            "external API key was not found".to_string(),
        ));
    };
    if existing.status != EXTERNAL_API_KEY_STATUS_ACTIVE {
        return Err((
            StatusCode::CONFLICT,
            "external API key is not active".to_string(),
        ));
    }
    existing.status = EXTERNAL_API_KEY_STATUS_ROTATED.to_string();
    existing.updated_at = format_utc_iso(Utc::now());
    let client_id = existing.client_id.clone();
    let name = existing.name.clone();
    let new_id = records.iter().map(|record| record.id).max().unwrap_or(id) + 1;
    let (secret, secret_hash, secret_prefix) = build_external_api_key_secret(&client_id)?;
    let now = format_utc_iso(Utc::now());
    let record = PrimaryExternalApiKeyRecord {
        id: new_id,
        client_id,
        name,
        secret_hash,
        secret_prefix,
        status: EXTERNAL_API_KEY_STATUS_ACTIVE.to_string(),
        last_used_at: None,
        created_at: now.clone(),
        updated_at: now,
        rotated_from_key_id: Some(id),
    };
    let response = ExternalApiKeySecretResponse {
        key: primary_external_api_key_summary(&record),
        secret,
    };
    records.push(record);
    save_primary_external_api_keys(store, &records)
        .await
        .map_err(internal_error_tuple_local)?;
    Ok(response)
}

pub(crate) async fn disable_primary_external_api_key(
    store: &dyn PrimaryStateStore,
    id: i64,
) -> Result<ExternalApiKeyMutationResponse, (StatusCode, String)> {
    let mut records = load_primary_external_api_keys(store)
        .await
        .map_err(internal_error_tuple_local)?;
    let Some(record) = records.iter_mut().find(|record| record.id == id) else {
        return Err((
            StatusCode::NOT_FOUND,
            "external API key was not found".to_string(),
        ));
    };
    if record.status == EXTERNAL_API_KEY_STATUS_ROTATED {
        return Err((
            StatusCode::CONFLICT,
            "external API key is already rotated".to_string(),
        ));
    }
    record.status = EXTERNAL_API_KEY_STATUS_DISABLED.to_string();
    record.updated_at = format_utc_iso(Utc::now());
    let response = ExternalApiKeyMutationResponse {
        key: primary_external_api_key_summary(record),
    };
    save_primary_external_api_keys(store, &records)
        .await
        .map_err(internal_error_tuple_local)?;
    Ok(response)
}

pub(crate) async fn list_external_api_keys(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ExternalApiKeyListResponse>, (StatusCode, String)> {
    if state.config.primary_database_backend == PrimaryDatabaseBackend::Postgres {
        let records = load_primary_external_api_keys(state.primary_state_store.as_ref())
            .await
            .map_err(internal_error_tuple_local)?;
        return Ok(Json(ExternalApiKeyListResponse {
            items: records
                .iter()
                .filter(|record| record.status != EXTERNAL_API_KEY_STATUS_ROTATED)
                .map(primary_external_api_key_summary)
                .collect(),
        }));
    }
    let items = list_external_api_key_rows(&state.pool)
        .await
        .map_err(internal_error_tuple_local)?
        .into_iter()
        .map(ExternalApiKeySummary::from)
        .collect();
    Ok(Json(ExternalApiKeyListResponse { items }))
}

pub(crate) async fn create_external_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateExternalApiKeyRequest>,
) -> Result<Json<ExternalApiKeySecretResponse>, (StatusCode, String)> {
    require_browser_same_origin_settings_write(&headers)?;
    if state.config.primary_database_backend == PrimaryDatabaseBackend::Postgres {
        return Ok(Json(
            create_primary_external_api_key(state.primary_state_store.as_ref(), payload).await?,
        ));
    }
    Ok(Json(create_external_api_key_inner(state, payload).await?))
}

pub(crate) async fn rotate_external_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<ExternalApiKeySecretResponse>, (StatusCode, String)> {
    require_browser_same_origin_settings_write(&headers)?;
    if state.config.primary_database_backend == PrimaryDatabaseBackend::Postgres {
        return Ok(Json(
            rotate_primary_external_api_key(state.primary_state_store.as_ref(), id).await?,
        ));
    }
    Ok(Json(rotate_external_api_key_inner(state, id).await?))
}

pub(crate) async fn disable_external_api_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<ExternalApiKeyMutationResponse>, (StatusCode, String)> {
    require_browser_same_origin_settings_write(&headers)?;
    if state.config.primary_database_backend == PrimaryDatabaseBackend::Postgres {
        return Ok(Json(
            disable_primary_external_api_key(state.primary_state_store.as_ref(), id).await?,
        ));
    }
    Ok(Json(disable_external_api_key_inner(state, id).await?))
}

pub(crate) async fn external_upsert_oauth_upstream_account_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(source_account_id): AxumPath<String>,
    Json(payload): Json<ExternalUpstreamAccountUpsertRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    let principal = authenticate_external_api_key(state.as_ref(), &headers).await?;
    let detail = external_upsert_oauth_upstream_account(
        state,
        ExternalAccountIdentity {
            client_id: principal.client_id,
            source_account_id,
        },
        payload,
    )
    .await?;
    Ok(Json(detail))
}

pub(crate) async fn external_patch_oauth_upstream_account_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(source_account_id): AxumPath<String>,
    Json(payload): Json<ExternalUpstreamAccountMetadataRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    let principal = authenticate_external_api_key(state.as_ref(), &headers).await?;
    let detail = external_patch_oauth_upstream_account(
        state,
        ExternalAccountIdentity {
            client_id: principal.client_id,
            source_account_id,
        },
        payload,
    )
    .await?;
    Ok(Json(detail))
}

pub(crate) async fn external_relogin_oauth_upstream_account_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(source_account_id): AxumPath<String>,
    Json(payload): Json<ExternalUpstreamAccountReloginRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    let principal = authenticate_external_api_key(state.as_ref(), &headers).await?;
    let detail = external_relogin_oauth_upstream_account(
        state,
        ExternalAccountIdentity {
            client_id: principal.client_id,
            source_account_id,
        },
        payload,
    )
    .await?;
    Ok(Json(detail))
}

#[cfg(test)]
mod primary_store_tests {
    use super::*;
    use crate::primary_store::SqlitePrimaryStateStore;

    #[tokio::test]
    async fn primary_external_api_keys_support_create_rotate_disable_and_auth() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        let store = SqlitePrimaryStateStore::new(pool);
        store.ensure_schema().await.unwrap();
        let created = create_primary_external_api_key(
            &store,
            CreateExternalApiKeyRequest {
                name: "primary-client".to_owned(),
            },
        )
        .await
        .unwrap();
        let mut records = load_primary_external_api_keys(&store).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, EXTERNAL_API_KEY_STATUS_ACTIVE);
        assert_eq!(
            records[0].secret_hash,
            hash_external_api_key_secret(&created.secret)
        );

        let rotated = rotate_primary_external_api_key(&store, created.key.id)
            .await
            .unwrap();
        records = load_primary_external_api_keys(&store).await.unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].status, EXTERNAL_API_KEY_STATUS_ROTATED);
        assert_eq!(rotated.key.status, EXTERNAL_API_KEY_STATUS_ACTIVE);

        let disabled = disable_primary_external_api_key(&store, rotated.key.id)
            .await
            .unwrap();
        assert_eq!(disabled.key.status, EXTERNAL_API_KEY_STATUS_DISABLED);
    }
}
