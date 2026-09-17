use super::*;
use crate::oauth_bridge::oauth_codex_upstream_base_url;
use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit},
};
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};

pub(crate) const ACCOUNT_MAINTENANCE_EGRESS_MIN_INTERVAL_SECS: i64 = 10;
pub(crate) const ACCOUNT_MAINTENANCE_EGRESS_RUNTIME_WAIT_MAX_SECS: u64 = 180;

#[derive(Debug, Clone)]
pub(crate) struct AccountMaintenanceEgressThrottleError {
    pub(crate) proxy_key: String,
    pub(crate) proxy_display_name: String,
    pub(crate) proxy_egress_ip: Option<String>,
    pub(crate) retry_after_secs: u64,
}

impl std::fmt::Display for AccountMaintenanceEgressThrottleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "maintenance egress via {} is throttled for another {} seconds",
            self.proxy_display_name, self.retry_after_secs
        )
    }
}

impl std::error::Error for AccountMaintenanceEgressThrottleError {}

#[derive(Debug, Clone)]
pub(crate) struct AccountMaintenanceProxySnapshot {
    pub(crate) proxy_key: String,
    pub(crate) proxy_display_name: String,
    pub(crate) proxy_egress_ip: Option<String>,
}

impl AccountMaintenanceProxySnapshot {
    fn from_selected_proxy(selected_proxy: &SelectedForwardProxy) -> Self {
        Self {
            proxy_key: selected_proxy.key.clone(),
            proxy_display_name: selected_proxy.display_name.clone(),
            proxy_egress_ip: selected_proxy.egress_ip.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct AccountMaintenanceProxyAwareError {
    proxy_snapshot: AccountMaintenanceProxySnapshot,
    message: String,
}

impl std::fmt::Display for AccountMaintenanceProxyAwareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AccountMaintenanceProxyAwareError {}

pub(crate) fn maintenance_proxy_snapshot_from_error(
    err: &anyhow::Error,
) -> Option<AccountMaintenanceProxySnapshot> {
    err.chain().find_map(|cause| {
        cause
            .downcast_ref::<AccountMaintenanceProxyAwareError>()
            .map(|value| value.proxy_snapshot.clone())
    })
}

pub(crate) async fn record_account_update_action(
    pool: &Pool<Sqlite>,
    account_id: i64,
    message: &str,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    record_upstream_account_action(
        pool,
        account_id,
        UpstreamAccountActionPayload {
            action: UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED,
            source: UPSTREAM_ACCOUNT_ACTION_SOURCE_ACCOUNT_UPDATE,
            reason_code: Some(UPSTREAM_ACCOUNT_ACTION_REASON_ACCOUNT_UPDATED),
            reason_message: Some(message),
            http_status: None,
            failure_kind: None,
            invoke_id: None,
            sticky_key: None,
            occurred_at: &now_iso,
        },
    )
    .await
}

pub(crate) async fn exchange_authorization_code(
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

pub(crate) async fn client_for_required_proxy_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Result<Client> {
    let selected_proxy = select_forward_proxy_for_scope(state, scope)
        .await
        .map_err(|err| map_required_group_proxy_selection_error(scope, err))?;
    state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        .context("failed to initialize required forward proxy client")
}

pub(crate) async fn client_for_required_maintenance_proxy_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Result<(Client, AccountMaintenanceProxySnapshot)> {
    let mut selected_proxy = select_forward_proxy_for_scope(state, scope)
        .await
        .map_err(|err| map_required_group_proxy_selection_error(scope, err))?;
    selected_proxy.egress_ip =
        crate::forward_proxy::load_forward_proxy_egress_ip_snapshot(state, &selected_proxy)
            .await
            .unwrap_or(None);
    let client = state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        .context("failed to initialize required forward proxy client")?;
    reserve_account_maintenance_egress_slot_for_runtime(&state.pool, &selected_proxy).await?;
    selected_proxy.egress_ip =
        crate::forward_proxy::refresh_forward_proxy_egress_ip_if_stale(state, &selected_proxy)
            .await
            .unwrap_or_else(|_| selected_proxy.egress_ip.clone());
    let snapshot = AccountMaintenanceProxySnapshot::from_selected_proxy(&selected_proxy);
    Ok((client, snapshot))
}

pub(crate) async fn reserve_account_maintenance_egress_slot(
    pool: &Pool<Sqlite>,
    selected_proxy: &SelectedForwardProxy,
) -> Result<()> {
    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to open egress throttle transaction")?;
    let existing_last_sent_at = sqlx::query_scalar::<_, String>(
        r#"
        SELECT last_sent_at
        FROM pool_upstream_account_egress_throttle
        WHERE egress_key = ?1
        "#,
    )
    .bind(&selected_proxy.key)
    .fetch_optional(&mut *tx)
    .await
    .context("failed to load egress throttle state")?;

    if let Some(last_sent_at) = existing_last_sent_at
        && let Some(last_sent_at) = parse_rfc3339_utc(&last_sent_at)
    {
        let elapsed_secs = now.signed_duration_since(last_sent_at).num_seconds();
        if elapsed_secs < ACCOUNT_MAINTENANCE_EGRESS_MIN_INTERVAL_SECS {
            let retry_after_secs =
                (ACCOUNT_MAINTENANCE_EGRESS_MIN_INTERVAL_SECS - elapsed_secs).max(1) as u64;
            tx.rollback()
                .await
                .context("failed to roll back throttled egress reservation")?;
            return Err(anyhow!(AccountMaintenanceEgressThrottleError {
                proxy_key: selected_proxy.key.clone(),
                proxy_display_name: selected_proxy.display_name.clone(),
                proxy_egress_ip: selected_proxy.egress_ip.clone(),
                retry_after_secs,
            }));
        }
    }

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_egress_throttle (
            egress_key, last_sent_at, created_at, updated_at
        ) VALUES (?1, ?2, ?2, ?2)
        ON CONFLICT(egress_key) DO UPDATE SET
            last_sent_at = excluded.last_sent_at,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&selected_proxy.key)
    .bind(&now_iso)
    .execute(&mut *tx)
    .await
    .context("failed to update egress throttle state")?;
    tx.commit()
        .await
        .context("failed to commit egress throttle reservation")?;
    Ok(())
}

pub(crate) async fn reserve_account_maintenance_egress_slot_for_runtime(
    pool: &Pool<Sqlite>,
    selected_proxy: &SelectedForwardProxy,
) -> Result<()> {
    #[cfg(test)]
    {
        if std::env::var_os("CVM_ENFORCE_ACCOUNT_MAINTENANCE_EGRESS_THROTTLE_IN_RUNTIME_TESTS")
            .is_none()
        {
            return Ok(());
        }
    }
    reserve_account_maintenance_egress_slot_with_bounded_wait(
        pool,
        selected_proxy,
        ACCOUNT_MAINTENANCE_EGRESS_RUNTIME_WAIT_MAX_SECS,
    )
    .await
}

pub(crate) async fn reserve_account_maintenance_egress_slot_with_bounded_wait(
    pool: &Pool<Sqlite>,
    selected_proxy: &SelectedForwardProxy,
    max_wait_secs: u64,
) -> Result<()> {
    let mut waited_secs = 0u64;
    loop {
        match reserve_account_maintenance_egress_slot(pool, selected_proxy).await {
            Ok(()) => return Ok(()),
            Err(err) => {
                let Some(throttle) = err.downcast_ref::<AccountMaintenanceEgressThrottleError>()
                else {
                    return Err(err);
                };
                if waited_secs >= max_wait_secs {
                    return Err(err);
                }
                let remaining_wait_budget = max_wait_secs - waited_secs;
                let wait_secs = throttle.retry_after_secs.min(remaining_wait_budget).max(1);
                sleep(Duration::from_secs(wait_secs)).await;
                waited_secs += wait_secs;
            }
        }
    }
}

pub(crate) async fn exchange_authorization_code_for_required_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse> {
    let client = client_for_required_proxy_scope(state, scope).await?;
    exchange_authorization_code(&client, &state.config, code, code_verifier, redirect_uri).await
}

pub(crate) async fn refresh_oauth_tokens(
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

pub(crate) async fn refresh_oauth_tokens_for_required_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    refresh_token: &str,
) -> Result<OAuthTokenResponse> {
    let (client, proxy_snapshot) =
        client_for_required_maintenance_proxy_scope(state, scope).await?;
    match refresh_oauth_tokens(&client, &state.config, refresh_token).await {
        Ok(response) => Ok(response),
        Err(err) => Err(anyhow!(AccountMaintenanceProxyAwareError {
            proxy_snapshot,
            message: err.to_string(),
        })),
    }
}

pub(crate) async fn parse_token_response(
    response: reqwest::Response,
) -> Result<OAuthTokenResponse> {
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

pub(crate) fn build_usage_endpoint_url(base_url: &Url) -> Result<Url> {
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

pub(crate) async fn fetch_usage_snapshot(
    client: &Client,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<NormalizedUsageSnapshot> {
    let primary_result = request_usage_snapshot_with_user_agent(
        client,
        config,
        access_token,
        chatgpt_account_id,
        &config.user_agent,
    )
    .await;

    if primary_result.is_ok() || config.user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT {
        return primary_result;
    }

    let primary_error = match primary_result {
        Ok(snapshot) => return Ok(snapshot),
        Err(err) => err,
    };
    if usage_snapshot_error_skips_browser_user_agent_retry(&primary_error) {
        return Err(primary_error);
    }

    warn!(
        error = ?primary_error,
        configured_user_agent = %config.user_agent,
        fallback_user_agent = %UPSTREAM_USAGE_BROWSER_USER_AGENT,
        "usage snapshot request failed; retrying with browser user agent"
    );

    request_usage_snapshot_with_user_agent(
        client,
        config,
        access_token,
        chatgpt_account_id,
        UPSTREAM_USAGE_BROWSER_USER_AGENT,
    )
    .await
    .map_err(|retry_error| {
        usage_snapshot_browser_user_agent_retry_error(primary_error, retry_error)
    })
}

pub(crate) fn usage_snapshot_error_is_network_failure(err: &anyhow::Error) -> bool {
    let normalized = err.to_string().to_ascii_lowercase();
    normalized.contains("failed to request usage snapshot")
        || normalized.contains("failed to read usage snapshot response")
        || normalized.contains("timed out")
        || normalized.contains("connection")
        || normalized.contains("transport")
}

pub(crate) fn usage_snapshot_error_skips_browser_user_agent_retry(err: &anyhow::Error) -> bool {
    maintenance_upstream_rejected_error_message(&err.to_string())
}

pub(crate) fn usage_snapshot_browser_user_agent_retry_error(
    primary_error: anyhow::Error,
    retry_error: anyhow::Error,
) -> anyhow::Error {
    let primary_context = format!(
        "initial usage snapshot attempt with configured user agent failed: {primary_error:#}"
    );
    if usage_snapshot_error_skips_browser_user_agent_retry(&retry_error) {
        anyhow!("{primary_context}; browser user agent retry failed: {retry_error:#}")
    } else {
        retry_error.context(primary_context)
    }
}

pub(crate) async fn fetch_usage_snapshot_via_forward_proxy(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<(NormalizedUsageSnapshot, AccountMaintenanceProxySnapshot)> {
    let primary_result = request_usage_snapshot_with_user_agent_via_forward_proxy(
        state,
        scope,
        config,
        access_token,
        chatgpt_account_id,
        &config.user_agent,
    )
    .await;

    if primary_result.is_ok() || config.user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT {
        return primary_result;
    }

    let primary_error = match primary_result {
        Ok(snapshot) => return Ok(snapshot),
        Err(err) => err,
    };
    if usage_snapshot_error_skips_browser_user_agent_retry(&primary_error) {
        return Err(primary_error);
    }

    warn!(
        error = ?primary_error,
        configured_user_agent = %config.user_agent,
        fallback_user_agent = %UPSTREAM_USAGE_BROWSER_USER_AGENT,
        "usage snapshot request failed; retrying with browser user agent"
    );

    request_usage_snapshot_with_user_agent_via_forward_proxy(
        state,
        scope,
        config,
        access_token,
        chatgpt_account_id,
        UPSTREAM_USAGE_BROWSER_USER_AGENT,
    )
    .await
    .map_err(|retry_error| {
        usage_snapshot_browser_user_agent_retry_error(primary_error, retry_error)
    })
}

pub(crate) async fn request_usage_snapshot_with_user_agent_via_forward_proxy(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    user_agent: &str,
) -> Result<(NormalizedUsageSnapshot, AccountMaintenanceProxySnapshot)> {
    let mut selected_proxy = select_forward_proxy_for_scope(state, scope).await?;
    selected_proxy.egress_ip =
        crate::forward_proxy::load_forward_proxy_egress_ip_snapshot(state, &selected_proxy)
            .await
            .unwrap_or(None);
    let proxy_snapshot = AccountMaintenanceProxySnapshot::from_selected_proxy(&selected_proxy);
    let client = match state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
    {
        Ok(client) => client,
        Err(err) => {
            record_forward_proxy_scope_result(
                state,
                scope,
                &selected_proxy.key,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
            return Err(anyhow!(AccountMaintenanceProxyAwareError {
                proxy_snapshot,
                message: err
                    .context("failed to initialize usage snapshot forward proxy client")
                    .to_string(),
            }));
        }
    };
    reserve_account_maintenance_egress_slot_for_runtime(&state.pool, &selected_proxy).await?;
    selected_proxy.egress_ip =
        crate::forward_proxy::refresh_forward_proxy_egress_ip_if_stale(state, &selected_proxy)
            .await
            .unwrap_or_else(|_| selected_proxy.egress_ip.clone());
    let proxy_snapshot = AccountMaintenanceProxySnapshot::from_selected_proxy(&selected_proxy);

    let result = request_usage_snapshot_with_user_agent(
        &client,
        config,
        access_token,
        chatgpt_account_id,
        user_agent,
    )
    .await;

    match &result {
        Ok(_) => {
            record_forward_proxy_scope_result(
                state,
                scope,
                &selected_proxy.key,
                ForwardProxyRouteResultKind::CompletedRequest,
            )
            .await;
        }
        Err(err) if usage_snapshot_error_is_network_failure(err) => {
            record_forward_proxy_scope_result(
                state,
                scope,
                &selected_proxy.key,
                ForwardProxyRouteResultKind::NetworkFailure,
            )
            .await;
        }
        Err(_) => {}
    }

    result
        .map(|snapshot| (snapshot, proxy_snapshot.clone()))
        .map_err(|err| {
            anyhow!(AccountMaintenanceProxyAwareError {
                proxy_snapshot,
                message: err.to_string(),
            })
        })
}

pub(crate) async fn request_usage_snapshot_with_user_agent(
    client: &Client,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    user_agent: &str,
) -> Result<NormalizedUsageSnapshot> {
    let url = build_usage_endpoint_url(&config.upstream_accounts_usage_base_url)
        .context("failed to build usage endpoint")?;
    let mut request = client
        .get(url)
        .bearer_auth(access_token)
        .header(header::USER_AGENT, user_agent);
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

pub(crate) fn normalize_usage_snapshot(value: &Value) -> Result<NormalizedUsageSnapshot> {
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

pub(crate) fn normalize_usage_window(
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

pub(crate) fn normalize_credits_snapshot(value: &Value) -> Result<CreditsSnapshot> {
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

pub(crate) fn build_oauth_authorize_url(
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
        .append_pair("audience", DEFAULT_OAUTH_AUDIENCE)
        .append_pair("scope", DEFAULT_OAUTH_SCOPE)
        .append_pair("prompt", DEFAULT_OAUTH_PROMPT)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state_token)
        .append_pair("originator", OAUTH_ORIGINATOR);
    Ok(url.to_string())
}

pub(crate) fn build_manual_callback_redirect_uri() -> Result<String> {
    let mut url =
        Url::parse("http://localhost").context("failed to build localhost callback URL")?;
    let _ = url.set_port(Some(DEFAULT_MANUAL_OAUTH_CALLBACK_PORT));
    url.set_path("/auth/callback");
    Ok(url.to_string())
}

pub(crate) fn derive_secret_key(secret: &str) -> [u8; 32] {
    let digest = Sha256::digest(secret.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

#[allow(deprecated)]
pub(crate) fn encrypt_credentials(
    key: &[u8; 32],
    credentials: &StoredCredentials,
) -> Result<String> {
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
pub(crate) fn decrypt_credentials(key: &[u8; 32], payload: &str) -> Result<StoredCredentials> {
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

pub(crate) fn decode_jwt_payload(token: &str, token_name: &str) -> Result<Vec<u8>> {
    let mut parts = token.split('.');
    let (_header, payload, _sig) = match (parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(payload), Some(sig))
            if !header.is_empty() && !payload.is_empty() && !sig.is_empty() =>
        {
            (header, payload, sig)
        }
        _ => bail!("invalid {token_name} format"),
    };
    URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| BASE64_STANDARD.decode(payload))
        .with_context(|| format!("failed to decode {token_name} payload"))
}

pub(crate) fn parse_chatgpt_jwt_claims(id_token: &str) -> Result<ChatgptJwtClaims> {
    let payload_bytes = decode_jwt_payload(id_token, "id_token")?;
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

pub(crate) fn parse_jwt_expiration_utc(token: &str, token_name: &str) -> Option<DateTime<Utc>> {
    let payload_bytes = decode_jwt_payload(token, token_name).ok()?;
    let claims: JwtExpiryClaims = serde_json::from_slice(&payload_bytes).ok()?;
    claims
        .exp
        .and_then(|exp| DateTime::<Utc>::from_timestamp(exp, 0))
}

pub(crate) fn resolve_imported_token_expires_at(
    expired: Option<&str>,
    access_token: &str,
    id_token: &str,
) -> Result<String, String> {
    if let Some(expired) = expired.map(str::trim).filter(|value| !value.is_empty()) {
        return parse_rfc3339_utc(expired)
            .map(format_utc_iso)
            .ok_or_else(|| "expired must be a valid RFC3339 timestamp".to_string());
    }

    parse_jwt_expiration_utc(access_token, "access_token")
        .or_else(|| parse_jwt_expiration_utc(id_token, "id_token"))
        .map(format_utc_iso)
        .ok_or_else(|| "expired is required when token exp is unavailable".to_string())
}

pub(crate) fn render_callback_page(success: bool, title: &str, message: &str) -> String {
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
