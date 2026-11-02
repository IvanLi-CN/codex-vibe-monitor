use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamTokenLifecycle {
    pub(crate) access_token: UpstreamTokenLifecycleEntry,
    pub(crate) refresh_token: UpstreamRefreshTokenLifecycleEntry,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamTokenLifecycleEntry {
    pub(crate) expires_at: Option<String>,
    pub(crate) state: String,
    pub(crate) source: Option<String>,
    pub(crate) observed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpstreamRefreshTokenLifecycleEntry {
    pub(crate) present: bool,
    pub(crate) expires_at: Option<String>,
    pub(crate) state: String,
    pub(crate) source: Option<String>,
    pub(crate) observed_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct OAuthTokenLifecycleMetadata {
    pub(crate) access_token_source: Option<String>,
    pub(crate) access_token_observed_at: Option<String>,
    pub(crate) refresh_token_expires_at: Option<String>,
    pub(crate) refresh_token_source: Option<String>,
    pub(crate) refresh_token_state: Option<String>,
    pub(crate) refresh_token_observed_at: Option<String>,
    pub(crate) refresh_token_rotated: bool,
    pub(crate) refresh_token_metadata_update: bool,
}

pub(crate) fn apply_oauth_token_response(
    credentials: &mut StoredOauthCredentials,
    response: OAuthTokenResponse,
) -> String {
    credentials.access_token = response.access_token;
    let access_token_expires_at = parse_access_token_expiry(&credentials.access_token);
    if let Some(refresh_token) = response.refresh_token {
        credentials.refresh_token = normalize_oauth_refresh_token(Some(refresh_token));
    }
    if let Some(id_token) = response.id_token {
        credentials.id_token = id_token;
    }
    credentials.token_type = response.token_type;
    access_token_expires_at
        .map(format_utc_iso)
        .unwrap_or_else(|| {
            format_utc_iso(Utc::now() + ChronoDuration::seconds(response.expires_in.max(0)))
        })
}

#[derive(Debug, Clone)]
pub(crate) struct NormalizedImportedOauthCredentials {
    pub(crate) source_id: String,
    pub(crate) file_name: String,
    pub(crate) email: String,
    pub(crate) display_name: String,
    pub(crate) chatgpt_account_id: String,
    pub(crate) chatgpt_user_id: Option<String>,
    pub(crate) token_expires_at: String,
    pub(crate) refresh_token_expires_at: Option<String>,
    pub(crate) refresh_token_expiry_source: String,
    pub(crate) refresh_token_metadata_update: bool,
    pub(crate) credentials: StoredOauthCredentials,
    pub(crate) claims: ChatgptJwtClaims,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedOauthProbeOutcome {
    pub(crate) token_expires_at: String,
    pub(crate) refresh_token_expires_at: Option<String>,
    pub(crate) refresh_token_expiry_source: String,
    pub(crate) refresh_token_metadata_update: bool,
    pub(crate) credentials: StoredOauthCredentials,
    pub(crate) claims: ChatgptJwtClaims,
    pub(crate) usage_snapshot: Option<NormalizedUsageSnapshot>,
    pub(crate) maintenance_proxy_snapshot: Option<AccountMaintenanceProxySnapshot>,
    pub(crate) exhausted: bool,
    pub(crate) usage_snapshot_warning: Option<String>,
}
