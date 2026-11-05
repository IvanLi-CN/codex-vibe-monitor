use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredApiKeyCredentials {
    pub(crate) api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StoredOauthCredentials {
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    pub(crate) id_token: String,
    pub(crate) token_type: Option<String>,
}

pub(crate) fn normalize_oauth_refresh_token(value: Option<String>) -> Option<String> {
    normalize_optional_text(value)
}

pub(crate) fn oauth_refresh_token(credentials: &StoredOauthCredentials) -> Option<&str> {
    credentials
        .refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn oauth_credentials_have_refresh_token(credentials: &StoredOauthCredentials) -> bool {
    oauth_refresh_token(credentials).is_some()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum StoredCredentials {
    ApiKey(StoredApiKeyCredentials),
    Oauth(StoredOauthCredentials),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EncryptedCredentialsPayload {
    pub(crate) v: u8,
    pub(crate) nonce: String,
    pub(crate) ciphertext: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NormalizedUsageSnapshot {
    pub(crate) plan_type: Option<String>,
    pub(crate) limit_id: String,
    pub(crate) limit_name: Option<String>,
    pub(crate) allowed: Option<bool>,
    pub(crate) limit_reached: Option<bool>,
    pub(crate) primary: Option<NormalizedUsageWindow>,
    pub(crate) secondary: Option<NormalizedUsageWindow>,
    pub(crate) credits: Option<CreditsSnapshot>,
    pub(crate) reserve: Option<NormalizedReserveQuotaSnapshot>,
    pub(crate) observed_window_candidates: u8,
    pub(crate) parsed_window_count: u8,
    pub(crate) window_parse_failure_count: u8,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OAuthTokenResponse {
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    #[serde(default)]
    pub(crate) id_token: Option<String>,
    #[serde(default)]
    pub(crate) token_type: Option<String>,
    pub(crate) expires_in: i64,
    #[serde(default, alias = "refreshTokenExpiresAt")]
    pub(crate) refresh_token_expires_at: Option<String>,
    #[serde(default, alias = "refreshTokenExpiresIn")]
    pub(crate) refresh_token_expires_in: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ChatgptJwtClaims {
    pub(crate) email: Option<String>,
    pub(crate) chatgpt_plan_type: Option<String>,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) chatgpt_account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ImportedOauthCredentialsFile {
    #[serde(rename = "type")]
    #[serde(default)]
    pub(crate) _source_type: Option<serde_json::Value>,
    pub(crate) email: String,
    pub(crate) account_id: String,
    #[serde(default)]
    pub(crate) expired: Option<String>,
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<serde_json::Value>,
    pub(crate) id_token: String,
    #[serde(default)]
    #[serde(rename = "last_refresh")]
    pub(crate) _last_refresh: Option<serde_json::Value>,
    #[serde(default)]
    pub(crate) token_type: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedOauthValidatedImportData {
    pub(crate) input_digest: String,
    pub(crate) normalized: NormalizedImportedOauthCredentials,
    pub(crate) probe: ImportedOauthProbeOutcome,
}

pub(crate) struct PersistOauthCallbackInput {
    pub(crate) session: OauthLoginSessionRow,
    pub(crate) display_name: String,
    pub(crate) chosen_email: Option<String>,
    pub(crate) verified_email: Option<String>,
    pub(crate) claims: ChatgptJwtClaims,
    pub(crate) encrypted_credentials: String,
    pub(crate) has_refresh_token: bool,
    pub(crate) token_expires_at: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatgptJwtOuterClaims {
    #[serde(default)]
    pub(crate) email: Option<String>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    pub(crate) profile: Option<ChatgptJwtProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    pub(crate) auth: Option<ChatgptJwtAuthClaims>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JwtExpiryClaims {
    #[serde(default)]
    pub(crate) exp: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatgptJwtProfileClaims {
    #[serde(default)]
    pub(crate) email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatgptJwtAuthClaims {
    #[serde(default)]
    pub(crate) chatgpt_plan_type: Option<String>,
    #[serde(default)]
    pub(crate) chatgpt_user_id: Option<String>,
    #[serde(default)]
    pub(crate) user_id: Option<String>,
    #[serde(default)]
    pub(crate) chatgpt_account_id: Option<String>,
}
