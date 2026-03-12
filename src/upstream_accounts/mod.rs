use super::*;
use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit},
};
use axum::{
    extract::{Path as AxumPath, Query},
    http::header,
    response::Html,
};
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};

pub(crate) const ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET: &str =
    "UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID: &str = "UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_OAUTH_ISSUER: &str = "UPSTREAM_ACCOUNTS_OAUTH_ISSUER";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_USAGE_BASE_URL: &str = "UPSTREAM_ACCOUNTS_USAGE_BASE_URL";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS: &str =
    "UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS: &str =
    "UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS: &str =
    "UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS: &str =
    "UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS";

pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER: &str = "https://auth.openai.com";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_USAGE_BASE_URL: &str = "https://chatgpt.com/backend-api";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS: u64 = 10 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS: u64 = 5 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS: u64 = 15 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS: u64 = 30;

const UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX: &str = "oauth_codex";
const UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX: &str = "api_key_codex";
const UPSTREAM_ACCOUNT_PROVIDER_CODEX: &str = "codex";
const UPSTREAM_ACCOUNT_STATUS_ACTIVE: &str = "active";
const UPSTREAM_ACCOUNT_STATUS_SYNCING: &str = "syncing";
const UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH: &str = "needs_reauth";
const UPSTREAM_ACCOUNT_STATUS_ERROR: &str = "error";
const UPSTREAM_ACCOUNT_STATUS_DISABLED: &str = "disabled";
const LOGIN_SESSION_STATUS_PENDING: &str = "pending";
const LOGIN_SESSION_STATUS_COMPLETED: &str = "completed";
const LOGIN_SESSION_STATUS_FAILED: &str = "failed";
const LOGIN_SESSION_STATUS_EXPIRED: &str = "expired";
const DEFAULT_OAUTH_SCOPE: &str = "openid profile email offline_access";
const OAUTH_AUDIENCE: &str = "https://api.openai.com/v1";
const DEFAULT_USAGE_LIMIT_ID: &str = "codex";
const DEFAULT_API_KEY_LIMIT_UNIT: &str = "requests";
const USAGE_PATH_STYLE_CHATGPT: &str = "/wham/usage";
const USAGE_PATH_STYLE_CODEX_API: &str = "/api/codex/usage";

#[derive(Debug)]
pub(crate) struct UpstreamAccountsRuntime {
    pub(crate) crypto_key: Option<[u8; 32]>,
    sync_lock: Arc<Mutex<()>>,
}

impl UpstreamAccountsRuntime {
    pub(crate) fn from_env() -> Result<Self> {
        let crypto_key = match env::var(ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET) {
            Ok(value) if !value.trim().is_empty() => Some(derive_secret_key(&value)),
            Ok(_) => {
                return Err(anyhow!(
                    "{} must not be empty when configured",
                    ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET
                ));
            }
            Err(env::VarError::NotPresent) => None,
            Err(err) => {
                return Err(anyhow!(
                    "failed to read {}: {err}",
                    ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET
                ));
            }
        };

        Ok(Self {
            crypto_key,
            sync_lock: Arc::new(Mutex::new(())),
        })
    }

    pub(crate) fn writes_enabled(&self) -> bool {
        self.crypto_key.is_some()
    }

    pub(crate) fn require_crypto_key(&self) -> Result<&[u8; 32], (StatusCode, String)> {
        self.crypto_key.as_ref().ok_or_else(|| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "account writes require {} to be configured",
                    ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET
                ),
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn test_instance() -> Self {
        Self {
            crypto_key: Some(derive_secret_key("test-upstream-account-secret")),
            sync_lock: Arc::new(Mutex::new(())),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountListResponse {
    writes_enabled: bool,
    items: Vec<UpstreamAccountSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountSummary {
    id: i64,
    kind: String,
    provider: String,
    display_name: String,
    group_name: Option<String>,
    status: String,
    enabled: bool,
    email: Option<String>,
    chatgpt_account_id: Option<String>,
    plan_type: Option<String>,
    masked_api_key: Option<String>,
    last_synced_at: Option<String>,
    last_successful_sync_at: Option<String>,
    last_error: Option<String>,
    last_error_at: Option<String>,
    token_expires_at: Option<String>,
    primary_window: Option<RateWindowSnapshot>,
    secondary_window: Option<RateWindowSnapshot>,
    credits: Option<CreditsSnapshot>,
    local_limits: Option<LocalLimitSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountDetail {
    #[serde(flatten)]
    summary: UpstreamAccountSummary,
    note: Option<String>,
    chatgpt_user_id: Option<String>,
    last_refreshed_at: Option<String>,
    history: Vec<UpstreamAccountHistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamAccountHistoryPoint {
    captured_at: String,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
    credits_balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateWindowSnapshot {
    used_percent: f64,
    used_text: String,
    limit_text: String,
    resets_at: Option<String>,
    window_duration_mins: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreditsSnapshot {
    has_credits: bool,
    unlimited: bool,
    balance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalLimitSnapshot {
    primary_limit: Option<f64>,
    secondary_limit: Option<f64>,
    limit_unit: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginSessionStatusResponse {
    login_id: String,
    status: String,
    auth_url: Option<String>,
    expires_at: String,
    account_id: Option<i64>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateOauthLoginSessionRequest {
    display_name: Option<String>,
    group_name: Option<String>,
    note: Option<String>,
    account_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateApiKeyAccountRequest {
    display_name: String,
    group_name: Option<String>,
    note: Option<String>,
    api_key: String,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
    local_limit_unit: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateUpstreamAccountRequest {
    display_name: Option<String>,
    group_name: Option<String>,
    note: Option<String>,
    enabled: Option<bool>,
    api_key: Option<String>,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
    local_limit_unit: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OauthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredApiKeyCredentials {
    api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredOauthCredentials {
    access_token: String,
    refresh_token: String,
    id_token: String,
    token_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum StoredCredentials {
    ApiKey(StoredApiKeyCredentials),
    Oauth(StoredOauthCredentials),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedCredentialsPayload {
    v: u8,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Clone)]
struct NormalizedUsageSnapshot {
    plan_type: Option<String>,
    limit_id: String,
    limit_name: Option<String>,
    primary: Option<NormalizedUsageWindow>,
    secondary: Option<NormalizedUsageWindow>,
    credits: Option<CreditsSnapshot>,
}

#[derive(Debug, Clone)]
struct NormalizedUsageWindow {
    used_percent: f64,
    window_duration_mins: i64,
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
    expires_in: i64,
}

#[derive(Debug, Clone, Default)]
struct ChatgptJwtClaims {
    email: Option<String>,
    chatgpt_plan_type: Option<String>,
    chatgpt_user_id: Option<String>,
    chatgpt_account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatgptJwtOuterClaims {
    #[serde(default)]
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    profile: Option<ChatgptJwtProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<ChatgptJwtAuthClaims>,
}

#[derive(Debug, Deserialize)]
struct ChatgptJwtProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatgptJwtAuthClaims {
    #[serde(default)]
    chatgpt_plan_type: Option<String>,
    #[serde(default)]
    chatgpt_user_id: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    chatgpt_account_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, FromRow)]
struct UpstreamAccountRow {
    id: i64,
    kind: String,
    provider: String,
    display_name: String,
    group_name: Option<String>,
    note: Option<String>,
    status: String,
    enabled: i64,
    email: Option<String>,
    chatgpt_account_id: Option<String>,
    chatgpt_user_id: Option<String>,
    plan_type: Option<String>,
    masked_api_key: Option<String>,
    encrypted_credentials: Option<String>,
    token_expires_at: Option<String>,
    last_refreshed_at: Option<String>,
    last_synced_at: Option<String>,
    last_successful_sync_at: Option<String>,
    last_error: Option<String>,
    last_error_at: Option<String>,
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
    local_limit_unit: Option<String>,
    created_at: String,
    updated_at: String,
}

#[allow(dead_code)]
#[derive(Debug, FromRow)]
struct UpstreamAccountSampleRow {
    captured_at: String,
    limit_id: Option<String>,
    limit_name: Option<String>,
    plan_type: Option<String>,
    primary_used_percent: Option<f64>,
    primary_window_minutes: Option<i64>,
    primary_resets_at: Option<String>,
    secondary_used_percent: Option<f64>,
    secondary_window_minutes: Option<i64>,
    secondary_resets_at: Option<String>,
    credits_has_credits: Option<i64>,
    credits_unlimited: Option<i64>,
    credits_balance: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, FromRow)]
struct OauthLoginSessionRow {
    login_id: String,
    account_id: Option<i64>,
    display_name: Option<String>,
    group_name: Option<String>,
    note: Option<String>,
    state: String,
    pkce_verifier: String,
    redirect_uri: String,
    status: String,
    auth_url: String,
    error_message: Option<String>,
    expires_at: String,
    consumed_at: Option<String>,
    created_at: String,
    updated_at: String,
}

pub(crate) async fn ensure_upstream_accounts_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_accounts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            kind TEXT NOT NULL,
            provider TEXT NOT NULL DEFAULT 'codex',
            display_name TEXT NOT NULL,
            group_name TEXT,
            note TEXT,
            status TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            email TEXT,
            chatgpt_account_id TEXT,
            chatgpt_user_id TEXT,
            plan_type TEXT,
            masked_api_key TEXT,
            encrypted_credentials TEXT,
            token_expires_at TEXT,
            last_refreshed_at TEXT,
            last_synced_at TEXT,
            last_successful_sync_at TEXT,
            last_error TEXT,
            last_error_at TEXT,
            local_primary_limit REAL,
            local_secondary_limit REAL,
            local_limit_unit TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_accounts table existence")?;

    ensure_nullable_text_column(pool, "pool_upstream_accounts", "group_name")
        .await
        .context("failed to ensure pool_upstream_accounts.group_name")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_accounts_kind_enabled
        ON pool_upstream_accounts (kind, enabled)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_accounts_kind_enabled")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_upstream_accounts_chatgpt_account_id
        ON pool_upstream_accounts (chatgpt_account_id)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_upstream_accounts_chatgpt_account_id")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_oauth_login_sessions (
            login_id TEXT PRIMARY KEY,
            account_id INTEGER,
            display_name TEXT,
            group_name TEXT,
            note TEXT,
            state TEXT NOT NULL UNIQUE,
            pkce_verifier TEXT NOT NULL,
            redirect_uri TEXT NOT NULL,
            status TEXT NOT NULL,
            auth_url TEXT NOT NULL,
            error_message TEXT,
            expires_at TEXT NOT NULL,
            consumed_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_oauth_login_sessions table existence")?;

    ensure_nullable_text_column(pool, "pool_oauth_login_sessions", "group_name")
        .await
        .context("failed to ensure pool_oauth_login_sessions.group_name")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS pool_upstream_account_limit_samples (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL,
            captured_at TEXT NOT NULL,
            limit_id TEXT,
            limit_name TEXT,
            plan_type TEXT,
            primary_used_percent REAL,
            primary_window_minutes INTEGER,
            primary_resets_at TEXT,
            secondary_used_percent REAL,
            secondary_window_minutes INTEGER,
            secondary_resets_at TEXT,
            credits_has_credits INTEGER,
            credits_unlimited INTEGER,
            credits_balance TEXT
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure pool_upstream_account_limit_samples table existence")?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_pool_limit_samples_account_captured_at
        ON pool_upstream_account_limit_samples (account_id, captured_at)
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure idx_pool_limit_samples_account_captured_at")?;

    Ok(())
}

pub(crate) fn spawn_upstream_account_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(state.config.upstream_accounts_sync_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("upstream account maintenance stopped");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(err) = run_upstream_account_maintenance_once(state.as_ref()).await {
                        warn!(error = %err, "failed to run upstream account maintenance");
                    }
                }
            }
        }
    })
}

async fn ensure_nullable_text_column(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();

    if columns.contains(column_name) {
        return Ok(());
    }

    let statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} TEXT");
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

pub(crate) async fn list_upstream_accounts(
    State(state): State<Arc<AppState>>,
) -> Result<Json<UpstreamAccountListResponse>, (StatusCode, String)> {
    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let items = load_upstream_account_summaries(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(UpstreamAccountListResponse {
        writes_enabled: state.upstream_accounts.writes_enabled(),
        items,
    }))
}

pub(crate) async fn get_upstream_account(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    let detail = load_upstream_account_detail(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(Json(detail))
}

pub(crate) async fn create_oauth_login_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateOauthLoginSessionRequest>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    if let Some(account_id) = payload.account_id {
        let Some(existing) = load_upstream_account_row(&state.pool, account_id)
            .await
            .map_err(internal_error_tuple)?
        else {
            return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
        };
        if existing.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
            return Err((
                StatusCode::BAD_REQUEST,
                "only OAuth accounts can be re-linked".to_string(),
            ));
        }
    }

    let redirect_uri = build_callback_redirect_uri(&headers)
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    let login_id = random_hex(16)?;
    let state_token = random_hex(32)?;
    let pkce_verifier = random_hex(64)?;
    let code_challenge = code_challenge_for_verifier(&pkce_verifier);
    let auth_url = build_oauth_authorize_url(
        &state.config.upstream_accounts_oauth_issuer,
        &state.config.upstream_accounts_oauth_client_id,
        &redirect_uri,
        &state_token,
        &code_challenge,
    )
    .map_err(internal_error_tuple)?;
    let now = Utc::now();
    let expires_at = now
        + ChronoDuration::seconds(state.config.upstream_accounts_login_session_ttl.as_secs() as i64);
    let now_iso = format_utc_iso(now);
    let expires_at_iso = format_utc_iso(expires_at);

    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, group_name, note, state, pkce_verifier, redirect_uri,
            status, auth_url, error_message, expires_at, consumed_at, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11, NULL, ?12, ?12)
        "#,
    )
    .bind(&login_id)
    .bind(payload.account_id)
    .bind(normalize_optional_text(payload.display_name))
    .bind(normalize_optional_text(payload.group_name))
    .bind(normalize_optional_text(payload.note))
    .bind(&state_token)
    .bind(&pkce_verifier)
    .bind(&redirect_uri)
    .bind(LOGIN_SESSION_STATUS_PENDING)
    .bind(&auth_url)
    .bind(&expires_at_iso)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .map_err(internal_error_tuple)?;

    Ok(Json(LoginSessionStatusResponse {
        login_id,
        status: LOGIN_SESSION_STATUS_PENDING.to_string(),
        auth_url: Some(auth_url),
        expires_at: expires_at_iso,
        account_id: payload.account_id,
        error: None,
    }))
}

pub(crate) async fn get_oauth_login_session(
    State(state): State<Arc<AppState>>,
    AxumPath(login_id): AxumPath<String>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let session = load_login_session_by_login_id(&state.pool, &login_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "login session not found".to_string()))?;
    Ok(Json(login_session_to_response(&session)))
}

pub(crate) async fn oauth_callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OauthCallbackQuery>,
) -> Response {
    match handle_oauth_callback(state.as_ref(), query).await {
        Ok(html) => (StatusCode::OK, Html(html)).into_response(),
        Err((status, html)) => (status, Html(html)).into_response(),
    }
}

pub(crate) async fn relogin_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<LoginSessionStatusResponse>, (StatusCode, String)> {
    let payload = CreateOauthLoginSessionRequest {
        display_name: None,
        group_name: None,
        note: None,
        account_id: Some(id),
    };
    create_oauth_login_session(State(state), headers, Json(payload)).await
}

pub(crate) async fn create_api_key_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateApiKeyAccountRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let display_name = normalize_required_display_name(&payload.display_name)?;
    validate_local_limits(payload.local_primary_limit, payload.local_secondary_limit)?;
    let api_key = normalize_required_secret(&payload.api_key, "apiKey")?;
    let group_name = normalize_optional_text(payload.group_name);
    let note = normalize_optional_text(payload.note);
    let limit_unit = normalize_limit_unit(payload.local_limit_unit);
    let masked_api_key = mask_api_key(&api_key);
    let now_iso = format_utc_iso(Utc::now());
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::ApiKey(StoredApiKeyCredentials { api_key }),
    )
    .map_err(internal_error_tuple)?;

    let inserted_id = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO pool_upstream_accounts (
            kind, provider, display_name, group_name, note, status, enabled, email, chatgpt_account_id,
            chatgpt_user_id, plan_type, masked_api_key, encrypted_credentials, token_expires_at,
            last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
            local_primary_limit, local_secondary_limit, local_limit_unit, created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, 1, NULL, NULL,
            NULL, NULL, ?7, ?8, NULL,
            NULL, NULL, NULL, NULL, NULL,
            ?9, ?10, ?11, ?12, ?12
        ) RETURNING id
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
    .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
    .bind(display_name)
    .bind(group_name)
    .bind(note)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(masked_api_key)
    .bind(encrypted_credentials)
    .bind(payload.local_primary_limit)
    .bind(payload.local_secondary_limit)
    .bind(limit_unit)
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .map_err(internal_error_tuple)?;

    let detail = sync_upstream_account_by_id(state.as_ref(), inserted_id, false)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(detail))
}

pub(crate) async fn update_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateUpstreamAccountRequest>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let crypto_key = state.upstream_accounts.require_crypto_key()?;
    let mut row = load_upstream_account_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;

    if let Some(display_name) = payload.display_name {
        row.display_name = normalize_required_display_name(&display_name)?;
    }
    if let Some(group_name) = payload.group_name {
        row.group_name = normalize_optional_text(Some(group_name));
    }
    if let Some(note) = payload.note {
        row.note = normalize_optional_text(Some(note));
    }
    if let Some(enabled) = payload.enabled {
        row.enabled = if enabled { 1 } else { 0 };
    }

    if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        if let Some(api_key) = payload.api_key {
            let api_key = normalize_required_secret(&api_key, "apiKey")?;
            row.masked_api_key = Some(mask_api_key(&api_key));
            row.encrypted_credentials = Some(
                encrypt_credentials(
                    crypto_key,
                    &StoredCredentials::ApiKey(StoredApiKeyCredentials { api_key }),
                )
                .map_err(internal_error_tuple)?,
            );
        }
        if payload.local_primary_limit.is_some() {
            row.local_primary_limit = payload.local_primary_limit;
        }
        if payload.local_secondary_limit.is_some() {
            row.local_secondary_limit = payload.local_secondary_limit;
        }
        if payload.local_limit_unit.is_some() {
            row.local_limit_unit = Some(normalize_limit_unit(payload.local_limit_unit));
        }
        validate_local_limits(row.local_primary_limit, row.local_secondary_limit)?;
    }

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET display_name = ?2,
            group_name = ?3,
            note = ?4,
            enabled = ?5,
            masked_api_key = ?6,
            encrypted_credentials = ?7,
            local_primary_limit = ?8,
            local_secondary_limit = ?9,
            local_limit_unit = ?10,
            updated_at = ?11
        WHERE id = ?1
        "#,
    )
    .bind(id)
    .bind(&row.display_name)
    .bind(&row.group_name)
    .bind(&row.note)
    .bind(row.enabled)
    .bind(&row.masked_api_key)
    .bind(&row.encrypted_credentials)
    .bind(row.local_primary_limit)
    .bind(row.local_secondary_limit)
    .bind(&row.local_limit_unit)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .map_err(internal_error_tuple)?;

    let detail = load_upstream_account_detail(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(Json(detail))
}

pub(crate) async fn delete_upstream_account(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    sqlx::query("DELETE FROM pool_upstream_account_limit_samples WHERE account_id = ?1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    sqlx::query("DELETE FROM pool_oauth_login_sessions WHERE account_id = ?1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let affected = sqlx::query("DELETE FROM pool_upstream_accounts WHERE id = ?1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal_error_tuple)?
        .rows_affected();
    if affected == 0 {
        return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn sync_upstream_account(
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
    let detail = sync_upstream_account_by_id(state.as_ref(), id, true)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(detail))
}

async fn handle_oauth_callback(
    state: &AppState,
    query: OauthCallbackQuery,
) -> Result<String, (StatusCode, String)> {
    let Some(state_value) = normalize_optional_text(query.state.clone()) else {
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(false, "OAuth callback rejected", "Missing state parameter."),
        ));
    };

    let Some(session) = load_login_session_by_state(&state.pool, &state_value)
        .await
        .map_err(internal_error_html)?
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(
                false,
                "OAuth callback rejected",
                "Login session was not found.",
            ),
        ));
    };

    let now = Utc::now();
    let Some(expires_at) = parse_rfc3339_utc(&session.expires_at) else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            render_callback_page(
                false,
                "OAuth callback failed",
                "Stored session expiry is invalid.",
            ),
        ));
    };
    if session.status != LOGIN_SESSION_STATUS_PENDING {
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(
                false,
                "OAuth callback rejected",
                "This login session has already been consumed.",
            ),
        ));
    }
    if now > expires_at {
        mark_login_session_expired(&state.pool, &session.login_id)
            .await
            .map_err(internal_error_html)?;
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(
                false,
                "OAuth callback expired",
                "The login session has expired. Please try again.",
            ),
        ));
    }
    if let Some(error) = normalize_optional_text(query.error) {
        let detail = normalize_optional_text(query.error_description)
            .unwrap_or_else(|| "Authorization was cancelled or rejected.".to_string());
        fail_login_session(
            &state.pool,
            &session.login_id,
            &format!("{error}: {detail}"),
        )
        .await
        .map_err(internal_error_html)?;
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(false, "OAuth authorization failed", &detail),
        ));
    }
    let Some(code) = normalize_optional_text(query.code) else {
        fail_login_session(&state.pool, &session.login_id, "missing authorization code")
            .await
            .map_err(internal_error_html)?;
        return Err((
            StatusCode::BAD_REQUEST,
            render_callback_page(
                false,
                "OAuth callback rejected",
                "Missing authorization code.",
            ),
        ));
    };

    let token_response = exchange_authorization_code(
        &state.http_clients.shared,
        &state.config,
        &code,
        &session.pkce_verifier,
        &session.redirect_uri,
    )
    .await
    .map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            render_callback_page(false, "OAuth token exchange failed", &err.to_string()),
        )
    })?;

    let Some(id_token) = token_response.id_token.clone() else {
        fail_login_session(
            &state.pool,
            &session.login_id,
            "id_token missing in token exchange response",
        )
        .await
        .map_err(internal_error_html)?;
        return Err((
            StatusCode::BAD_GATEWAY,
            render_callback_page(
                false,
                "OAuth token exchange failed",
                "The token response did not include an id_token.",
            ),
        ));
    };
    let Some(refresh_token) = token_response.refresh_token.clone() else {
        fail_login_session(
            &state.pool,
            &session.login_id,
            "refresh_token missing in token exchange response",
        )
        .await
        .map_err(internal_error_html)?;
        return Err((
            StatusCode::BAD_GATEWAY,
            render_callback_page(
                false,
                "OAuth token exchange failed",
                "The token response did not include a refresh token.",
            ),
        ));
    };

    let claims = parse_chatgpt_jwt_claims(&id_token).map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            render_callback_page(false, "OAuth token parse failed", &err.to_string()),
        )
    })?;
    let crypto_key = state.upstream_accounts.crypto_key.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            render_callback_page(
                false,
                "Credential storage disabled",
                &format!(
                    "{} is required to persist OAuth credentials.",
                    ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET
                ),
            ),
        )
    })?;

    let token_expires_at =
        format_utc_iso(Utc::now() + ChronoDuration::seconds(token_response.expires_in.max(0)));
    let credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: token_response.access_token.clone(),
            refresh_token,
            id_token,
            token_type: token_response.token_type.clone(),
        }),
    )
    .map_err(internal_error_html)?;

    let default_display_name = claims
        .email
        .clone()
        .or_else(|| session.display_name.clone())
        .unwrap_or_else(|| "Codex OAuth".to_string());
    let display_name = session
        .display_name
        .clone()
        .and_then(|value| normalize_optional_text(Some(value)))
        .unwrap_or(default_display_name);
    let account_id = upsert_oauth_account(
        &state.pool,
        OauthAccountUpsert {
            account_id: session.account_id,
            display_name: &display_name,
            group_name: session.group_name.clone(),
            note: session.note.clone(),
            claims: &claims,
            encrypted_credentials: credentials,
            token_expires_at: &token_expires_at,
        },
    )
    .await
    .map_err(internal_error_html)?;
    complete_login_session(&state.pool, &session.login_id, account_id)
        .await
        .map_err(internal_error_html)?;

    if let Err(err) = sync_upstream_account_by_id(state, account_id, false).await {
        warn!(account_id, error = %err, "OAuth callback created account but initial sync failed");
    }

    Ok(render_callback_page(
        true,
        "OAuth login complete",
        "The upstream account is ready. You can close this window.",
    ))
}

async fn run_upstream_account_maintenance_once(state: &AppState) -> Result<()> {
    expire_pending_login_sessions(&state.pool).await?;
    let Some(_) = state.upstream_accounts.crypto_key else {
        return Ok(());
    };

    let _guard = state.upstream_accounts.sync_lock.lock().await;
    let account_ids = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_upstream_accounts
        WHERE kind = ?1 AND enabled = 1
        ORDER BY updated_at ASC, id ASC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
    .fetch_all(&state.pool)
    .await?;

    for account_id in account_ids {
        let Some(row) = load_upstream_account_row(&state.pool, account_id).await? else {
            continue;
        };
        if !should_maintain_account(&row, state) {
            continue;
        }
        if let Err(err) = sync_oauth_account(state, &row).await {
            warn!(account_id, error = %err, "failed to maintain upstream OAuth account");
        }
    }

    Ok(())
}

fn should_maintain_account(row: &UpstreamAccountRow, state: &AppState) -> bool {
    if row.kind != UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX || row.enabled == 0 {
        return false;
    }
    if row.status == UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH {
        return false;
    }
    let now = Utc::now();
    let sync_due = row
        .last_synced_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .map(|last| {
            now.signed_duration_since(last).num_seconds()
                >= state.config.upstream_accounts_sync_interval.as_secs() as i64
        })
        .unwrap_or(true);
    let refresh_due = row
        .token_expires_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .map(|expires| {
            expires
                <= now
                    + ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    )
        })
        .unwrap_or(true);
    sync_due || refresh_due || row.status == UPSTREAM_ACCOUNT_STATUS_ERROR
}

async fn sync_upstream_account_by_id(
    state: &AppState,
    id: i64,
    reject_disabled: bool,
) -> Result<UpstreamAccountDetail> {
    let row = load_upstream_account_row(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account not found"))?;

    if row.enabled == 0 {
        if reject_disabled {
            bail!("disabled accounts cannot be synced");
        }
        let detail = load_upstream_account_detail(&state.pool, id)
            .await?
            .ok_or_else(|| anyhow!("account not found"))?;
        return Ok(detail);
    }

    match row.kind.as_str() {
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX => sync_oauth_account(state, &row).await?,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX => sync_api_key_account(&state.pool, &row).await?,
        _ => bail!("unsupported account kind: {}", row.kind),
    }

    load_upstream_account_detail(&state.pool, id)
        .await?
        .ok_or_else(|| anyhow!("account not found after sync"))
}

async fn sync_api_key_account(pool: &Pool<Sqlite>, row: &UpstreamAccountRow) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_successful_sync_at = ?3,
            last_error = NULL,
            last_error_at = NULL,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(row.id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn sync_oauth_account(state: &AppState, row: &UpstreamAccountRow) -> Result<()> {
    set_account_status(&state.pool, row.id, UPSTREAM_ACCOUNT_STATUS_SYNCING, None).await?;
    let now = Utc::now();
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .ok_or_else(|| anyhow!("account writes are disabled"))?;
    let decrypted = decrypt_credentials(
        crypto_key,
        row.encrypted_credentials
            .as_deref()
            .ok_or_else(|| anyhow!("missing encrypted OAuth credentials"))?,
    )?;
    let StoredCredentials::Oauth(mut credentials) = decrypted else {
        bail!("unexpected credential kind for OAuth account")
    };

    let expires_at = row.token_expires_at.as_deref().and_then(parse_rfc3339_utc);
    let refresh_due = expires_at
        .map(|expires| {
            expires
                <= now
                    + ChronoDuration::seconds(
                        state.config.upstream_accounts_refresh_lead_time.as_secs() as i64,
                    )
        })
        .unwrap_or(true);

    if refresh_due {
        match refresh_oauth_tokens(
            &state.http_clients.shared,
            &state.config,
            &credentials.refresh_token,
        )
        .await
        {
            Ok(response) => {
                credentials.access_token = response.access_token;
                if let Some(refresh_token) = response.refresh_token {
                    credentials.refresh_token = refresh_token;
                }
                if let Some(id_token) = response.id_token {
                    credentials.id_token = id_token;
                }
                credentials.token_type = response.token_type;
                let token_expires_at = format_utc_iso(
                    Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)),
                );
                persist_oauth_credentials(
                    &state.pool,
                    row.id,
                    crypto_key,
                    &credentials,
                    &token_expires_at,
                )
                .await?;
            }
            Err(err) if is_reauth_error(&err) => {
                update_account_error(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                    &err.to_string(),
                )
                .await?;
                return Ok(());
            }
            Err(err) => {
                update_account_error(
                    &state.pool,
                    row.id,
                    UPSTREAM_ACCOUNT_STATUS_ERROR,
                    &err.to_string(),
                )
                .await?;
                return Ok(());
            }
        }
    }

    let latest_row = load_upstream_account_row(&state.pool, row.id)
        .await?
        .ok_or_else(|| anyhow!("account disappeared during sync"))?;
    let decrypted = decrypt_credentials(
        crypto_key,
        latest_row
            .encrypted_credentials
            .as_deref()
            .ok_or_else(|| anyhow!("missing encrypted OAuth credentials"))?,
    )?;
    let StoredCredentials::Oauth(credentials) = decrypted else {
        bail!("unexpected credential kind for OAuth account")
    };

    let usage_result = fetch_usage_snapshot(
        &state.http_clients.shared,
        &state.config,
        &credentials.access_token,
        latest_row.chatgpt_account_id.as_deref(),
    )
    .await;

    let snapshot = match usage_result {
        Ok(snapshot) => snapshot,
        Err(err) if err.to_string().contains("401") || err.to_string().contains("403") => {
            match refresh_oauth_tokens(
                &state.http_clients.shared,
                &state.config,
                &credentials.refresh_token,
            )
            .await
            {
                Ok(response) => {
                    let mut refreshed = credentials.clone();
                    refreshed.access_token = response.access_token;
                    if let Some(refresh_token) = response.refresh_token {
                        refreshed.refresh_token = refresh_token;
                    }
                    if let Some(id_token) = response.id_token {
                        refreshed.id_token = id_token;
                    }
                    refreshed.token_type = response.token_type;
                    let token_expires_at = format_utc_iso(
                        Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)),
                    );
                    persist_oauth_credentials(
                        &state.pool,
                        row.id,
                        crypto_key,
                        &refreshed,
                        &token_expires_at,
                    )
                    .await?;
                    fetch_usage_snapshot(
                        &state.http_clients.shared,
                        &state.config,
                        &refreshed.access_token,
                        latest_row.chatgpt_account_id.as_deref(),
                    )
                    .await?
                }
                Err(refresh_err) if is_reauth_error(&refresh_err) => {
                    update_account_error(
                        &state.pool,
                        row.id,
                        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
                        &refresh_err.to_string(),
                    )
                    .await?;
                    return Ok(());
                }
                Err(refresh_err) => {
                    update_account_error(
                        &state.pool,
                        row.id,
                        UPSTREAM_ACCOUNT_STATUS_ERROR,
                        &refresh_err.to_string(),
                    )
                    .await?;
                    return Ok(());
                }
            }
        }
        Err(err) => {
            update_account_error(
                &state.pool,
                row.id,
                UPSTREAM_ACCOUNT_STATUS_ERROR,
                &err.to_string(),
            )
            .await?;
            return Ok(());
        }
    };

    persist_usage_snapshot(
        &state.pool,
        &latest_row,
        &snapshot,
        state.config.upstream_accounts_history_retention_days,
    )
    .await?;
    mark_account_sync_success(&state.pool, row.id).await?;
    Ok(())
}

async fn persist_oauth_credentials(
    pool: &Pool<Sqlite>,
    account_id: i64,
    crypto_key: &[u8; 32],
    credentials: &StoredOauthCredentials,
    token_expires_at: &str,
) -> Result<()> {
    let claims = parse_chatgpt_jwt_claims(&credentials.id_token).unwrap_or_default();
    let encrypted =
        encrypt_credentials(crypto_key, &StoredCredentials::Oauth(credentials.clone()))?;
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET encrypted_credentials = ?2,
            token_expires_at = ?3,
            last_refreshed_at = ?4,
            email = COALESCE(?5, email),
            chatgpt_account_id = COALESCE(?6, chatgpt_account_id),
            chatgpt_user_id = COALESCE(?7, chatgpt_user_id),
            plan_type = COALESCE(?8, plan_type),
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(encrypted)
    .bind(token_expires_at)
    .bind(&now_iso)
    .bind(claims.email)
    .bind(claims.chatgpt_account_id)
    .bind(claims.chatgpt_user_id)
    .bind(claims.chatgpt_plan_type)
    .execute(pool)
    .await?;
    Ok(())
}

async fn persist_usage_snapshot(
    pool: &Pool<Sqlite>,
    row: &UpstreamAccountRow,
    snapshot: &NormalizedUsageSnapshot,
    retention_days: u64,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_limit_samples (
            account_id, captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        "#,
    )
    .bind(row.id)
    .bind(&now_iso)
    .bind(&snapshot.limit_id)
    .bind(&snapshot.limit_name)
    .bind(snapshot.plan_type.clone().or_else(|| row.plan_type.clone()))
    .bind(snapshot.primary.as_ref().map(|value| value.used_percent))
    .bind(
        snapshot
            .primary
            .as_ref()
            .map(|value| value.window_duration_mins),
    )
    .bind(
        snapshot
            .primary
            .as_ref()
            .and_then(|value| value.resets_at.clone()),
    )
    .bind(snapshot.secondary.as_ref().map(|value| value.used_percent))
    .bind(
        snapshot
            .secondary
            .as_ref()
            .map(|value| value.window_duration_mins),
    )
    .bind(
        snapshot
            .secondary
            .as_ref()
            .and_then(|value| value.resets_at.clone()),
    )
    .bind(
        snapshot
            .credits
            .as_ref()
            .map(|value| if value.has_credits { 1 } else { 0 }),
    )
    .bind(
        snapshot
            .credits
            .as_ref()
            .map(|value| if value.unlimited { 1 } else { 0 }),
    )
    .bind(
        snapshot
            .credits
            .as_ref()
            .and_then(|value| value.balance.clone()),
    )
    .execute(pool)
    .await?;

    let retention_cutoff = format_utc_iso(Utc::now() - ChronoDuration::days(retention_days as i64));
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1 AND captured_at < ?2
        "#,
    )
    .bind(row.id)
    .bind(retention_cutoff)
    .execute(pool)
    .await?;
    Ok(())
}

struct OauthAccountUpsert<'a> {
    account_id: Option<i64>,
    display_name: &'a str,
    group_name: Option<String>,
    note: Option<String>,
    claims: &'a ChatgptJwtClaims,
    encrypted_credentials: String,
    token_expires_at: &'a str,
}

async fn upsert_oauth_account(pool: &Pool<Sqlite>, payload: OauthAccountUpsert<'_>) -> Result<i64> {
    let OauthAccountUpsert {
        account_id,
        display_name,
        group_name,
        note,
        claims,
        encrypted_credentials,
        token_expires_at,
    } = payload;
    let now_iso = format_utc_iso(Utc::now());
    let resolved_account_id = if let Some(account_id) = account_id {
        Some(account_id)
    } else if let Some(chatgpt_account_id) = claims.chatgpt_account_id.as_deref() {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM pool_upstream_accounts
            WHERE kind = ?1 AND chatgpt_account_id = ?2
            ORDER BY id ASC
            LIMIT 1
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(chatgpt_account_id)
        .fetch_optional(pool)
        .await?
    } else {
        None
    };

    if let Some(existing_id) = resolved_account_id {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET kind = ?2,
                provider = ?3,
                display_name = ?4,
                group_name = COALESCE(?5, group_name),
                note = ?6,
                status = ?7,
                enabled = 1,
                email = ?8,
                chatgpt_account_id = ?9,
                chatgpt_user_id = ?10,
                plan_type = ?11,
                encrypted_credentials = ?12,
                token_expires_at = ?13,
                last_refreshed_at = ?14,
                last_error = NULL,
                last_error_at = NULL,
                updated_at = ?14
            WHERE id = ?1
            "#,
        )
        .bind(existing_id)
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(group_name)
        .bind(note)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind(claims.email.clone())
        .bind(claims.chatgpt_account_id.clone())
        .bind(claims.chatgpt_user_id.clone())
        .bind(claims.chatgpt_plan_type.clone())
        .bind(encrypted_credentials)
        .bind(token_expires_at)
        .bind(&now_iso)
        .execute(pool)
        .await?;
        Ok(existing_id)
    } else {
        let inserted_account_id: i64 = sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, note, status, enabled,
                email, chatgpt_account_id, chatgpt_user_id, plan_type,
                masked_api_key, encrypted_credentials, token_expires_at,
                last_refreshed_at, last_synced_at, last_successful_sync_at,
                last_error, last_error_at, local_primary_limit, local_secondary_limit,
                local_limit_unit, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, 1,
                ?7, ?8, ?9, ?10,
                NULL, ?11, ?12,
                ?13, NULL, NULL,
                NULL, NULL, NULL, NULL,
                NULL, ?13, ?13
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(group_name)
        .bind(note)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind(claims.email.clone())
        .bind(claims.chatgpt_account_id.clone())
        .bind(claims.chatgpt_user_id.clone())
        .bind(claims.chatgpt_plan_type.clone())
        .bind(encrypted_credentials)
        .bind(token_expires_at)
        .bind(&now_iso)
        .fetch_one(pool)
        .await?;
        Ok(inserted_account_id)
    }
}

async fn load_upstream_account_summaries(
    pool: &Pool<Sqlite>,
) -> Result<Vec<UpstreamAccountSummary>> {
    let rows = sqlx::query_as::<_, UpstreamAccountRow>(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_error, last_error_at,
            local_primary_limit, local_secondary_limit, local_limit_unit,
            created_at, updated_at
        FROM pool_upstream_accounts
        ORDER BY updated_at DESC, id DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let latest = load_latest_usage_sample(pool, row.id).await?;
        items.push(build_summary_from_row(&row, latest.as_ref()));
    }
    Ok(items)
}

async fn load_upstream_account_detail(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<UpstreamAccountDetail>> {
    let Some(row) = load_upstream_account_row(pool, id).await? else {
        return Ok(None);
    };
    let latest = load_latest_usage_sample(pool, row.id).await?;
    let history_rows = sqlx::query_as::<_, UpstreamAccountSampleRow>(
        r#"
        SELECT
            captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1
        ORDER BY captured_at DESC
        LIMIT 128
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let mut history = history_rows
        .into_iter()
        .map(|sample| UpstreamAccountHistoryPoint {
            captured_at: sample.captured_at,
            primary_used_percent: sample.primary_used_percent,
            secondary_used_percent: sample.secondary_used_percent,
            credits_balance: sample.credits_balance,
        })
        .collect::<Vec<_>>();
    history.reverse();

    let summary = build_summary_from_row(&row, latest.as_ref());
    Ok(Some(UpstreamAccountDetail {
        summary,
        note: row.note,
        chatgpt_user_id: row.chatgpt_user_id,
        last_refreshed_at: row.last_refreshed_at,
        history,
    }))
}

async fn load_upstream_account_row(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<UpstreamAccountRow>> {
    sqlx::query_as::<_, UpstreamAccountRow>(
        r#"
        SELECT
            id, kind, provider, display_name, group_name, note, status, enabled, email,
            chatgpt_account_id, chatgpt_user_id, plan_type, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at,
            last_synced_at, last_successful_sync_at, last_error, last_error_at,
            local_primary_limit, local_secondary_limit, local_limit_unit,
            created_at, updated_at
        FROM pool_upstream_accounts
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_latest_usage_sample(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<Option<UpstreamAccountSampleRow>> {
    sqlx::query_as::<_, UpstreamAccountSampleRow>(
        r#"
        SELECT
            captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        FROM pool_upstream_account_limit_samples
        WHERE account_id = ?1
        ORDER BY captured_at DESC
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

fn build_summary_from_row(
    row: &UpstreamAccountRow,
    sample: Option<&UpstreamAccountSampleRow>,
) -> UpstreamAccountSummary {
    let local_limits = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        Some(LocalLimitSnapshot {
            primary_limit: row.local_primary_limit,
            secondary_limit: row.local_secondary_limit,
            limit_unit: row
                .local_limit_unit
                .clone()
                .unwrap_or_else(|| DEFAULT_API_KEY_LIMIT_UNIT.to_string()),
        })
    } else {
        None
    };
    let primary_window = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        build_api_key_window(
            row.local_primary_limit,
            row.local_limit_unit.as_deref(),
            300,
        )
    } else {
        sample.and_then(|value| {
            build_window_snapshot(
                value.primary_used_percent,
                value.primary_window_minutes,
                value.primary_resets_at.as_deref(),
            )
        })
    };
    let secondary_window = if row.kind == UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        build_api_key_window(
            row.local_secondary_limit,
            row.local_limit_unit.as_deref(),
            10_080,
        )
    } else {
        sample.and_then(|value| {
            build_window_snapshot(
                value.secondary_used_percent,
                value.secondary_window_minutes,
                value.secondary_resets_at.as_deref(),
            )
        })
    };
    let credits = sample.and_then(|value| {
        value
            .credits_has_credits
            .map(|has_credits| CreditsSnapshot {
                has_credits: has_credits != 0,
                unlimited: value.credits_unlimited.unwrap_or_default() != 0,
                balance: value.credits_balance.clone(),
            })
    });

    UpstreamAccountSummary {
        id: row.id,
        kind: row.kind.clone(),
        provider: row.provider.clone(),
        display_name: row.display_name.clone(),
        group_name: row.group_name.clone(),
        status: effective_account_status(row),
        enabled: row.enabled != 0,
        email: row.email.clone(),
        chatgpt_account_id: row.chatgpt_account_id.clone(),
        plan_type: row
            .plan_type
            .clone()
            .or_else(|| sample.and_then(|value| value.plan_type.clone())),
        masked_api_key: row.masked_api_key.clone(),
        last_synced_at: row.last_synced_at.clone(),
        last_successful_sync_at: row.last_successful_sync_at.clone(),
        last_error: row.last_error.clone(),
        last_error_at: row.last_error_at.clone(),
        token_expires_at: row.token_expires_at.clone(),
        primary_window,
        secondary_window,
        credits,
        local_limits,
    }
}

async fn load_login_session_by_login_id(
    pool: &Pool<Sqlite>,
    login_id: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    sqlx::query_as::<_, OauthLoginSessionRow>(
        r#"
        SELECT
            login_id, account_id, display_name, group_name, note, state, pkce_verifier, redirect_uri,
            status, auth_url, error_message, expires_at, consumed_at, created_at, updated_at
        FROM pool_oauth_login_sessions
        WHERE login_id = ?1
        LIMIT 1
        "#,
    )
    .bind(login_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn load_login_session_by_state(
    pool: &Pool<Sqlite>,
    state_value: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    sqlx::query_as::<_, OauthLoginSessionRow>(
        r#"
        SELECT
            login_id, account_id, display_name, group_name, note, state, pkce_verifier, redirect_uri,
            status, auth_url, error_message, expires_at, consumed_at, created_at, updated_at
        FROM pool_oauth_login_sessions
        WHERE state = ?1
        LIMIT 1
        "#,
    )
    .bind(state_value)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

async fn expire_pending_login_sessions(pool: &Pool<Sqlite>) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?1, updated_at = ?2
        WHERE status = ?3 AND expires_at < ?2
        "#,
    )
    .bind(LOGIN_SESSION_STATUS_EXPIRED)
    .bind(&now_iso)
    .bind(LOGIN_SESSION_STATUS_PENDING)
    .execute(pool)
    .await?;
    Ok(())
}

async fn complete_login_session(
    pool: &Pool<Sqlite>,
    login_id: &str,
    account_id: i64,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            account_id = ?3,
            consumed_at = ?4,
            updated_at = ?4
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_COMPLETED)
    .bind(account_id)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn fail_login_session(
    pool: &Pool<Sqlite>,
    login_id: &str,
    error_message: &str,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            error_message = ?3,
            consumed_at = ?4,
            updated_at = ?4
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_FAILED)
    .bind(error_message)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn mark_login_session_expired(pool: &Pool<Sqlite>, login_id: &str) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            updated_at = ?3
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_EXPIRED)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

fn login_session_to_response(row: &OauthLoginSessionRow) -> LoginSessionStatusResponse {
    LoginSessionStatusResponse {
        login_id: row.login_id.clone(),
        status: row.status.clone(),
        auth_url: if row.status == LOGIN_SESSION_STATUS_PENDING {
            Some(row.auth_url.clone())
        } else {
            None
        },
        expires_at: row.expires_at.clone(),
        account_id: row.account_id,
        error: row.error_message.clone(),
    }
}

fn effective_account_status(row: &UpstreamAccountRow) -> String {
    if row.enabled == 0 {
        UPSTREAM_ACCOUNT_STATUS_DISABLED.to_string()
    } else {
        row.status.clone()
    }
}

fn build_api_key_window(
    limit: Option<f64>,
    unit: Option<&str>,
    window_duration_mins: i64,
) -> Option<RateWindowSnapshot> {
    let limit_text = match limit {
        Some(value) => format!(
            "{} {}",
            format_compact_decimal(value),
            unit.unwrap_or(DEFAULT_API_KEY_LIMIT_UNIT)
        ),
        None => "—".to_string(),
    };
    Some(RateWindowSnapshot {
        used_percent: 0.0,
        used_text: format!("0 {}", unit.unwrap_or(DEFAULT_API_KEY_LIMIT_UNIT)),
        limit_text,
        resets_at: None,
        window_duration_mins,
    })
}

fn build_window_snapshot(
    used_percent: Option<f64>,
    window_duration_mins: Option<i64>,
    resets_at: Option<&str>,
) -> Option<RateWindowSnapshot> {
    let used_percent = used_percent?;
    let window_duration_mins = window_duration_mins?;
    Some(RateWindowSnapshot {
        used_percent,
        used_text: format!("{}%", format_percent(used_percent)),
        limit_text: format_window_label(window_duration_mins),
        resets_at: resets_at.map(ToOwned::to_owned),
        window_duration_mins,
    })
}

async fn set_account_status(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    last_error: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = ?3,
            last_error_at = CASE WHEN ?3 IS NULL THEN last_error_at ELSE ?4 END,
            updated_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(status)
    .bind(last_error)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn update_account_error(
    pool: &Pool<Sqlite>,
    account_id: i64,
    status: &str,
    error_message: &str,
) -> Result<()> {
    set_account_status(pool, account_id, status, Some(error_message)).await
}

async fn mark_account_sync_success(pool: &Pool<Sqlite>, account_id: i64) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_synced_at = ?3,
            last_successful_sync_at = ?3,
            last_error = NULL,
            last_error_at = NULL,
            updated_at = ?3
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

async fn exchange_authorization_code(
    client: &Client,
    config: &AppConfig,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse> {
    let url = config
        .upstream_accounts_oauth_issuer
        .join("/oauth/token")
        .context("failed to join OAuth token endpoint")?;
    let response = client
        .post(url)
        .form(&[
            ("grant_type", "authorization_code"),
            (
                "client_id",
                config.upstream_accounts_oauth_client_id.as_str(),
            ),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ])
        .send()
        .await
        .context("failed to exchange authorization code")?;
    parse_token_response(response).await
}

async fn refresh_oauth_tokens(
    client: &Client,
    config: &AppConfig,
    refresh_token: &str,
) -> Result<OAuthTokenResponse> {
    let url = config
        .upstream_accounts_oauth_issuer
        .join("/oauth/token")
        .context("failed to join OAuth token endpoint")?;
    let response = client
        .post(url)
        .form(&[
            ("grant_type", "refresh_token"),
            (
                "client_id",
                config.upstream_accounts_oauth_client_id.as_str(),
            ),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .context("failed to refresh OAuth token")?;
    parse_token_response(response).await
}

async fn parse_token_response(response: reqwest::Response) -> Result<OAuthTokenResponse> {
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read OAuth token response body")?;
    if !status.is_success() {
        let detail = extract_error_message(&body);
        bail!("OAuth token endpoint returned {}: {}", status, detail);
    }
    serde_json::from_str(&body).context("failed to decode OAuth token response")
}

fn build_usage_endpoint_url(base_url: &Url) -> Result<Url> {
    let usage_path = if base_url.path().contains("/backend-api") {
        USAGE_PATH_STYLE_CHATGPT
    } else {
        USAGE_PATH_STYLE_CODEX_API
    };
    let base_path = base_url.path().trim_end_matches('/');
    let resolved_path = if base_path.is_empty() || base_path == "/" {
        usage_path.to_string()
    } else {
        format!("{base_path}/{}", usage_path.trim_start_matches('/'))
    };
    let mut url = base_url.clone();
    url.set_path(&resolved_path);
    Ok(url)
}

async fn fetch_usage_snapshot(
    client: &Client,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<NormalizedUsageSnapshot> {
    let url = build_usage_endpoint_url(&config.upstream_accounts_usage_base_url)
        .context("failed to build usage endpoint")?;
    let mut request = client
        .get(url)
        .bearer_auth(access_token)
        .header(header::USER_AGENT, config.user_agent.clone());
    if let Some(account_id) = chatgpt_account_id
        && !account_id.trim().is_empty()
    {
        request = request.header("ChatGPT-Account-Id", account_id);
    }
    let response = request
        .send()
        .await
        .context("failed to request usage snapshot")?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read usage snapshot response")?;
    if !status.is_success() {
        bail!(
            "usage endpoint returned {}: {}",
            status,
            extract_error_message(&body)
        );
    }
    let value: Value =
        serde_json::from_str(&body).context("failed to decode usage snapshot JSON")?;
    normalize_usage_snapshot(&value)
}

fn normalize_usage_snapshot(value: &Value) -> Result<NormalizedUsageSnapshot> {
    let updated_at = optional_string(value, &["updated_at", "updatedAt"])
        .and_then(|value| parse_rfc3339_utc(&value));
    let limit = value
        .get("rate_limits_by_limit_id")
        .or_else(|| value.get("rateLimitsByLimitId"))
        .and_then(|value| value.get(DEFAULT_USAGE_LIMIT_ID))
        .or_else(|| value.get("rate_limit"))
        .or_else(|| value.get("rateLimit"))
        .unwrap_or(value);
    let primary = normalize_usage_window(
        limit
            .get("primary_window")
            .or_else(|| limit.get("primaryWindow")),
        updated_at,
    );
    let secondary = normalize_usage_window(
        limit
            .get("secondary_window")
            .or_else(|| limit.get("secondaryWindow")),
        updated_at,
    );
    let credits = value
        .get("credits")
        .map(normalize_credits_snapshot)
        .transpose()?;

    Ok(NormalizedUsageSnapshot {
        plan_type: optional_string(value, &["plan_type", "planType"]),
        limit_id: DEFAULT_USAGE_LIMIT_ID.to_string(),
        limit_name: Some(DEFAULT_USAGE_LIMIT_ID.to_string()),
        primary,
        secondary,
        credits,
    })
}

fn normalize_usage_window(
    value: Option<&Value>,
    updated_at: Option<DateTime<Utc>>,
) -> Option<NormalizedUsageWindow> {
    let value = value?;
    let used_percent = value
        .get("used_percent")
        .or_else(|| value.get("usedPercent"))
        .and_then(value_as_f64)?;
    let window_duration_mins = value
        .get("window_duration_mins")
        .or_else(|| value.get("windowDurationMins"))
        .and_then(value_as_i64)
        .or_else(|| {
            value
                .get("limit_window_seconds")
                .or_else(|| value.get("limitWindowSeconds"))
                .and_then(value_as_i64)
                .map(seconds_to_window_minutes)
        })?;
    let resets_at = value
        .get("resets_at")
        .or_else(|| value.get("resetsAt"))
        .and_then(value_as_timestamp)
        .map(format_utc_iso)
        .or_else(|| {
            let base = updated_at.unwrap_or_else(Utc::now);
            value
                .get("reset_after_seconds")
                .or_else(|| value.get("resetAfterSeconds"))
                .and_then(value_as_i64)
                .map(|seconds| format_utc_iso(base + ChronoDuration::seconds(seconds.max(0))))
        });
    Some(NormalizedUsageWindow {
        used_percent,
        window_duration_mins,
        resets_at,
    })
}

fn normalize_credits_snapshot(value: &Value) -> Result<CreditsSnapshot> {
    Ok(CreditsSnapshot {
        has_credits: value
            .get("has_credits")
            .or_else(|| value.get("hasCredits"))
            .and_then(value_as_bool)
            .unwrap_or(false),
        unlimited: value
            .get("unlimited")
            .and_then(value_as_bool)
            .unwrap_or(false),
        balance: value
            .get("balance")
            .or_else(|| value.get("creditBalance"))
            .and_then(value_as_string),
    })
}

fn build_oauth_authorize_url(
    issuer: &Url,
    client_id: &str,
    redirect_uri: &str,
    state_token: &str,
    code_challenge: &str,
) -> Result<String> {
    let mut url = issuer
        .join("/oauth/authorize")
        .context("failed to join OAuth authorize endpoint")?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", DEFAULT_OAUTH_SCOPE)
        .append_pair("state", state_token)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("audience", OAUTH_AUDIENCE)
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("prompt", "login");
    Ok(url.to_string())
}

fn build_callback_redirect_uri(headers: &HeaderMap) -> Result<String> {
    if let Some(origin_raw) = headers.get(header::ORIGIN)
        && let Ok(origin) = origin_raw.to_str()
        && let Ok(mut origin_url) = Url::parse(origin)
    {
        origin_url.set_path("/api/pool/upstream-accounts/oauth/callback");
        origin_url.set_query(None);
        origin_url.set_fragment(None);
        return Ok(origin_url.to_string());
    }

    let scheme = header_value_as_str(headers, "x-forwarded-proto")
        .filter(|value| matches!(*value, "http" | "https"))
        .unwrap_or("http");
    let Some((host, port)) = forwarded_or_host_authority(headers, scheme) else {
        bail!("failed to determine request origin for OAuth callback");
    };
    let mut url =
        Url::parse(&format!("{scheme}://{host}")).context("failed to build callback origin URL")?;
    if let Some(port) = port {
        let _ = url.set_port(Some(port));
    }
    url.set_path("/api/pool/upstream-accounts/oauth/callback");
    Ok(url.to_string())
}

fn derive_secret_key(secret: &str) -> [u8; 32] {
    let digest = Sha256::digest(secret.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

#[allow(deprecated)]
fn encrypt_credentials(key: &[u8; 32], credentials: &StoredCredentials) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| anyhow!("invalid AES key: {err}"))?;
    let plaintext = serde_json::to_vec(credentials).context("failed to serialize credentials")?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|err| anyhow!("failed to encrypt credentials: {err}"))?;
    serde_json::to_string(&EncryptedCredentialsPayload {
        v: 1,
        nonce: BASE64_STANDARD.encode(nonce),
        ciphertext: BASE64_STANDARD.encode(ciphertext),
    })
    .context("failed to encode encrypted credentials payload")
}

#[allow(deprecated)]
fn decrypt_credentials(key: &[u8; 32], payload: &str) -> Result<StoredCredentials> {
    let payload: EncryptedCredentialsPayload =
        serde_json::from_str(payload).context("failed to decode encrypted credentials payload")?;
    if payload.v != 1 {
        bail!(
            "unsupported encrypted credential payload version: {}",
            payload.v
        );
    }
    let nonce = BASE64_STANDARD
        .decode(payload.nonce)
        .context("failed to decode credential nonce")?;
    let ciphertext = BASE64_STANDARD
        .decode(payload.ciphertext)
        .context("failed to decode credential ciphertext")?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|err| anyhow!("invalid AES key: {err}"))?;
    let plaintext = cipher
        .decrypt(aes_gcm::Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|err| anyhow!("failed to decrypt credentials: {err}"))?;
    serde_json::from_slice(&plaintext).context("failed to decode credential JSON")
}

fn parse_chatgpt_jwt_claims(id_token: &str) -> Result<ChatgptJwtClaims> {
    let mut parts = id_token.split('.');
    let (_header, payload, _sig) = match (parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(payload), Some(sig))
            if !header.is_empty() && !payload.is_empty() && !sig.is_empty() =>
        {
            (header, payload, sig)
        }
        _ => bail!("invalid id_token format"),
    };
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| BASE64_STANDARD.decode(payload))
        .context("failed to decode id_token payload")?;
    let claims: ChatgptJwtOuterClaims =
        serde_json::from_slice(&payload_bytes).context("failed to parse id_token payload")?;
    Ok(ChatgptJwtClaims {
        email: claims
            .email
            .or_else(|| claims.profile.and_then(|value| value.email)),
        chatgpt_plan_type: claims
            .auth
            .as_ref()
            .and_then(|value| value.chatgpt_plan_type.clone()),
        chatgpt_user_id: claims.auth.as_ref().and_then(|value| {
            value
                .chatgpt_user_id
                .clone()
                .or_else(|| value.user_id.clone())
        }),
        chatgpt_account_id: claims
            .auth
            .as_ref()
            .and_then(|value| value.chatgpt_account_id.clone()),
    })
}

fn render_callback_page(success: bool, title: &str, message: &str) -> String {
    let accent = if success { "#0f8b6f" } else { "#d9485f" };
    let script = if success {
        "setTimeout(() => { try { window.close(); } catch (_) {} }, 1200);"
    } else {
        ""
    };
    format!(
        r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{title}</title>
    <style>
      body {{
        margin: 0;
        min-height: 100vh;
        display: grid;
        place-items: center;
        background: radial-gradient(circle at top, rgba(15,139,111,0.12), transparent 45%), #f5f7fb;
        color: #0f172a;
        font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }}
      .card {{
        width: min(92vw, 480px);
        padding: 28px;
        border-radius: 24px;
        background: rgba(255,255,255,0.94);
        box-shadow: 0 24px 80px rgba(15,23,42,0.14);
        border: 1px solid rgba(15,23,42,0.08);
      }}
      .badge {{
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 6px 12px;
        border-radius: 999px;
        font-size: 13px;
        font-weight: 700;
        color: {accent};
        background: rgba(255,255,255,0.75);
        border: 1px solid rgba(15,23,42,0.08);
      }}
      h1 {{ margin: 16px 0 12px; font-size: 24px; }}
      p {{ margin: 0; line-height: 1.7; color: rgba(15,23,42,0.78); }}
    </style>
  </head>
  <body>
    <main class="card">
      <div class="badge">{badge}</div>
      <h1>{title}</h1>
      <p>{message}</p>
    </main>
    <script>{script}</script>
  </body>
</html>"#,
        title = title,
        accent = accent,
        badge = if success {
            "Codex OAuth connected"
        } else {
            "Codex OAuth failed"
        },
        message = message,
        script = script,
    )
}

fn normalize_required_display_name(raw: &str) -> Result<String, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "displayName is required".to_string(),
        ));
    }
    if value.len() > 120 {
        return Err((
            StatusCode::BAD_REQUEST,
            "displayName must be <= 120 characters".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_required_secret(raw: &str, field_name: &str) -> Result<String, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((StatusCode::BAD_REQUEST, format!("{field_name} is required")));
    }
    Ok(value.to_string())
}

fn normalize_limit_unit(value: Option<String>) -> String {
    value
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_API_KEY_LIMIT_UNIT.to_string())
}

fn validate_local_limits(
    local_primary_limit: Option<f64>,
    local_secondary_limit: Option<f64>,
) -> Result<(), (StatusCode, String)> {
    for (label, value) in [
        ("localPrimaryLimit", local_primary_limit),
        ("localSecondaryLimit", local_secondary_limit),
    ] {
        if let Some(value) = value
            && value < 0.0
        {
            return Err((StatusCode::BAD_REQUEST, format!("{label} must be >= 0")));
        }
    }
    Ok(())
}

fn parse_rfc3339_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn code_challenge_for_verifier(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

fn random_hex(size: usize) -> Result<String, (StatusCode, String)> {
    let mut bytes = vec![0u8; size];
    OsRng.fill_bytes(&mut bytes);
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}")
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    }
    Ok(output)
}

fn format_window_label(window_duration_mins: i64) -> String {
    match window_duration_mins {
        300 => "5h quota".to_string(),
        10_080 => "7d quota".to_string(),
        mins if mins % (60 * 24) == 0 => format!("{}d quota", mins / (60 * 24)),
        mins if mins % 60 == 0 => format!("{}h quota", mins / 60),
        mins => format!("{}m quota", mins),
    }
}

fn format_percent(value: f64) -> String {
    if (value.fract()).abs() < 0.05 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

fn format_compact_decimal(value: f64) -> String {
    let rounded = format!("{value:.2}");
    rounded
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn mask_api_key(api_key: &str) -> String {
    if api_key.len() <= 8 {
        return "••••••••".to_string();
    }
    format!("{}••••{}", &api_key[..4], &api_key[api_key.len() - 4..])
}

fn seconds_to_window_minutes(seconds: i64) -> i64 {
    (seconds + 59) / 60
}

fn optional_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(value_as_string)
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => Some(raw.clone()),
        Value::Number(raw) => Some(raw.to_string()),
        _ => None,
    }
}

fn value_as_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(raw) => Some(*raw),
        Value::String(raw) => match raw.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(raw) => raw.as_f64(),
        Value::String(raw) => raw.parse::<f64>().ok(),
        _ => None,
    }
}

fn value_as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(raw) => raw.as_i64(),
        Value::String(raw) => raw.parse::<i64>().ok(),
        _ => None,
    }
}

fn value_as_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    value_as_i64(value).and_then(|seconds| Utc.timestamp_opt(seconds, 0).single())
}

fn extract_error_message(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body)
        && let Some(message) = value
            .get("error_description")
            .and_then(value_as_string)
            .or_else(|| value.get("message").and_then(value_as_string))
            .or_else(|| {
                value
                    .get("error")
                    .and_then(|value| value.get("message"))
                    .and_then(value_as_string)
            })
            .or_else(|| value.get("error").and_then(value_as_string))
    {
        return message;
    }
    body.trim().chars().take(240).collect()
}

fn is_reauth_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("400")
        || msg.contains("401")
        || msg.contains("403")
        || msg.contains("invalid_grant")
        || msg.contains("refresh token")
}

fn internal_error_tuple(err: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn internal_error_html(err: impl ToString) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        render_callback_page(false, "OAuth callback failed", &err.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_secret_key_is_stable() {
        let lhs = derive_secret_key("alpha");
        let rhs = derive_secret_key("alpha");
        assert_eq!(lhs, rhs);
    }

    #[test]
    fn credential_round_trip_works() {
        let key = derive_secret_key("top-secret");
        let encrypted = encrypt_credentials(
            &key,
            &StoredCredentials::ApiKey(StoredApiKeyCredentials {
                api_key: "sk-test-1234".to_string(),
            }),
        )
        .expect("encrypt credentials");
        let decrypted = decrypt_credentials(&key, &encrypted).expect("decrypt credentials");
        let StoredCredentials::ApiKey(value) = decrypted else {
            panic!("expected API key credentials")
        };
        assert_eq!(value.api_key, "sk-test-1234");
    }

    #[test]
    fn parse_chatgpt_jwt_claims_extracts_identity_fields() {
        let payload = json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": "pro",
                "chatgpt_user_id": "user_123",
                "chatgpt_account_id": "org_123"
            }
        });
        let encoded = URL_SAFE_NO_PAD.encode(b"{}");
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let token = format!("{encoded}.{body}.{encoded}");
        let claims = parse_chatgpt_jwt_claims(&token).expect("parse token");
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
        assert_eq!(claims.chatgpt_plan_type.as_deref(), Some("pro"));
        assert_eq!(claims.chatgpt_user_id.as_deref(), Some("user_123"));
        assert_eq!(claims.chatgpt_account_id.as_deref(), Some("org_123"));
    }

    #[test]
    fn build_usage_endpoint_url_preserves_backend_api_prefix() {
        let base = Url::parse("https://chatgpt.com/backend-api").expect("chatgpt base");
        let resolved = build_usage_endpoint_url(&base).expect("resolved usage url");
        assert_eq!(
            resolved.as_str(),
            "https://chatgpt.com/backend-api/wham/usage"
        );

        let base_with_slash =
            Url::parse("https://chatgpt.com/backend-api/").expect("chatgpt base with slash");
        let resolved_with_slash =
            build_usage_endpoint_url(&base_with_slash).expect("resolved usage url");
        assert_eq!(
            resolved_with_slash.as_str(),
            "https://chatgpt.com/backend-api/wham/usage"
        );
    }

    #[test]
    fn normalize_usage_snapshot_reads_windows_and_resets() {
        let payload = json!({
            "planType": "pro",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                },
                "secondaryWindow": {
                    "usedPercent": 18.5,
                    "windowDurationMins": 10080,
                    "resetsAt": 1771927200
                }
            },
            "credits": {
                "hasCredits": true,
                "unlimited": false,
                "balance": "9.99"
            }
        });
        let snapshot = normalize_usage_snapshot(&payload).expect("normalize snapshot");
        assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
        assert_eq!(
            snapshot.primary.as_ref().map(|value| value.used_percent),
            Some(42.0)
        );
        assert_eq!(
            snapshot.secondary.as_ref().map(|value| value.used_percent),
            Some(18.5)
        );
        assert_eq!(
            snapshot
                .credits
                .as_ref()
                .and_then(|value| value.balance.clone())
                .as_deref(),
            Some("9.99")
        );
    }

    #[test]
    fn build_callback_redirect_uri_prefers_origin_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("http://127.0.0.1:60080"),
        );
        let redirect = build_callback_redirect_uri(&headers).expect("redirect uri");
        assert_eq!(
            redirect,
            "http://127.0.0.1:60080/api/pool/upstream-accounts/oauth/callback"
        );
    }
}
