async fn record_account_update_action(
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

async fn client_for_required_proxy_scope(
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

async fn exchange_authorization_code_for_required_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokenResponse> {
    let client = client_for_required_proxy_scope(state, scope).await?;
    exchange_authorization_code(&client, &state.config, code, code_verifier, redirect_uri).await
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

async fn refresh_oauth_tokens_for_required_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    refresh_token: &str,
) -> Result<OAuthTokenResponse> {
    let client = client_for_required_proxy_scope(state, scope).await?;
    refresh_oauth_tokens(&client, &state.config, refresh_token).await
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
    .with_context(|| {
        format!(
            "initial usage snapshot attempt with configured user agent failed: {primary_error:#}"
        )
    })
}

fn usage_snapshot_error_is_network_failure(err: &anyhow::Error) -> bool {
    let normalized = err.to_string().to_ascii_lowercase();
    normalized.contains("failed to request usage snapshot")
        || normalized.contains("failed to read usage snapshot response")
        || normalized.contains("timed out")
        || normalized.contains("connection")
        || normalized.contains("transport")
}

fn usage_snapshot_error_skips_browser_user_agent_retry(err: &anyhow::Error) -> bool {
    let normalized = err.to_string().to_ascii_lowercase();
    normalized.contains("deactivated_workspace")
        || normalized.contains("upstream_http_402")
        || normalized.contains("payment required")
        || normalized.contains("usage endpoint returned 402")
        || normalized.contains("upstream rejected")
}

async fn fetch_usage_snapshot_via_forward_proxy(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
) -> Result<NormalizedUsageSnapshot> {
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
    .with_context(|| {
        format!(
            "initial usage snapshot attempt with configured user agent failed: {primary_error:#}"
        )
    })
}

async fn request_usage_snapshot_with_user_agent_via_forward_proxy(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    config: &AppConfig,
    access_token: &str,
    chatgpt_account_id: Option<&str>,
    user_agent: &str,
) -> Result<NormalizedUsageSnapshot> {
    let selected_proxy = select_forward_proxy_for_scope(state, scope).await?;
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
            return Err(err).context("failed to initialize usage snapshot forward proxy client");
        }
    };

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
}

async fn request_usage_snapshot_with_user_agent(
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

fn build_manual_callback_redirect_uri() -> Result<String> {
    let mut url =
        Url::parse("http://localhost").context("failed to build localhost callback URL")?;
    let _ = url.set_port(Some(DEFAULT_MANUAL_OAUTH_CALLBACK_PORT));
    url.set_path("/auth/callback");
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

fn decode_jwt_payload(token: &str, token_name: &str) -> Result<Vec<u8>> {
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

fn parse_chatgpt_jwt_claims(id_token: &str) -> Result<ChatgptJwtClaims> {
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

fn parse_jwt_expiration_utc(token: &str, token_name: &str) -> Option<DateTime<Utc>> {
    let payload_bytes = decode_jwt_payload(token, token_name).ok()?;
    let claims: JwtExpiryClaims = serde_json::from_slice(&payload_bytes).ok()?;
    claims
        .exp
        .and_then(|exp| DateTime::<Utc>::from_timestamp(exp, 0))
}

fn resolve_imported_token_expires_at(
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

fn validate_group_note_target(
    group_name: Option<&str>,
    has_group_note: bool,
) -> Result<(), (StatusCode, String)> {
    if has_group_note && group_name.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "groupNote requires groupName".to_string(),
        ));
    }
    Ok(())
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_optional_upstream_base_url(
    value: Option<String>,
) -> Result<Option<String>, (StatusCode, String)> {
    let Some(raw) = normalize_optional_text(value) else {
        return Ok(None);
    };
    let parsed = Url::parse(&raw).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "upstreamBaseUrl must be a valid absolute URL".to_string(),
        )
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.cannot_be_a_base()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "upstreamBaseUrl must be a valid absolute URL".to_string(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "upstreamBaseUrl must not include query or fragment".to_string(),
        ));
    }
    Ok(Some(parsed.to_string()))
}

fn resolve_pool_account_upstream_base_url(
    row: &UpstreamAccountRow,
    global_upstream_base_url: &Url,
) -> Result<Url> {
    if row.kind == UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX {
        return oauth_codex_upstream_base_url();
    }
    if row.kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        return Ok(global_upstream_base_url.clone());
    }

    row.upstream_base_url
        .as_deref()
        .map(Url::parse)
        .transpose()
        .context("account upstreamBaseUrl is invalid")?
        .map_or_else(|| Ok(global_upstream_base_url.clone()), Ok)
}

pub(crate) fn canonical_pool_upstream_route_key(url: &Url) -> String {
    let mut normalized = url.clone();
    normalized.set_query(None);
    normalized.set_fragment(None);
    let scheme_default_port = match normalized.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    if normalized.port().is_some() && normalized.port() == scheme_default_port {
        let _ = normalized.set_port(None);
    }
    let normalized_path = normalized.path().trim_end_matches('/').to_string();
    if normalized_path.is_empty() {
        normalized.set_path("/");
    } else {
        normalized.set_path(&normalized_path);
    }
    normalized.to_string()
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

fn is_import_invalid_error_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    is_explicit_reauth_error_message(message)
        || is_scope_permission_error_message(message)
        || normalized.contains("returned 400")
        || normalized.contains("returned 401")
        || normalized.contains("returned 403")
}

fn persisted_usage_snapshot_is_exhausted(
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
    credits_has_credits: Option<bool>,
    credits_unlimited: Option<bool>,
    credits_balance: Option<&str>,
) -> bool {
    let primary_exhausted = primary_used_percent.is_some_and(|value| value >= 100.0);
    let secondary_exhausted = secondary_used_percent.is_some_and(|value| value >= 100.0);
    let credits_exhausted = credits_has_credits.is_some_and(|has_credits| has_credits)
        && !credits_unlimited.unwrap_or(false)
        && credits_balance
            .and_then(|value| value.parse::<f64>().ok())
            .is_some_and(|value| value <= 0.0);
    primary_exhausted || secondary_exhausted || credits_exhausted
}

fn imported_snapshot_is_exhausted(snapshot: &NormalizedUsageSnapshot) -> bool {
    persisted_usage_snapshot_is_exhausted(
        snapshot.primary.as_ref().map(|window| window.used_percent),
        snapshot
            .secondary
            .as_ref()
            .map(|window| window.used_percent),
        snapshot.credits.as_ref().map(|credits| credits.has_credits),
        snapshot.credits.as_ref().map(|credits| credits.unlimited),
        snapshot
            .credits
            .as_ref()
            .and_then(|credits| credits.balance.as_deref()),
    )
}

fn persisted_usage_sample_is_exhausted(sample: Option<&UpstreamAccountSampleRow>) -> bool {
    sample.is_some_and(|sample| {
        persisted_usage_snapshot_is_exhausted(
            sample.primary_used_percent,
            sample.secondary_used_percent,
            sample.credits_has_credits.map(|value| value != 0),
            sample.credits_unlimited.map(|value| value != 0),
            sample.credits_balance.as_deref(),
        )
    })
}

fn routing_candidate_snapshot_is_exhausted(candidate: &AccountRoutingCandidateRow) -> bool {
    persisted_usage_snapshot_is_exhausted(
        candidate.primary_used_percent,
        candidate.secondary_used_percent,
        candidate.credits_has_credits.map(|value| value != 0),
        candidate.credits_unlimited.map(|value| value != 0),
        candidate.credits_balance.as_deref(),
    )
}

fn imported_match_key(email: &str, account_id: &str) -> String {
    let normalized_account_id = account_id.trim().to_ascii_lowercase();
    if !normalized_account_id.is_empty() {
        return format!("account:{normalized_account_id}");
    }
    format!("email:{}", email.trim().to_ascii_lowercase())
}

fn import_match_summary_from_row(row: &UpstreamAccountRow) -> ImportedOauthMatchSummary {
    ImportedOauthMatchSummary {
        account_id: row.id,
        display_name: row.display_name.clone(),
        group_name: row.group_name.clone(),
        status: effective_account_status(row),
    }
}

fn normalize_imported_oauth_credentials(
    item: &ImportOauthCredentialFileRequest,
) -> Result<NormalizedImportedOauthCredentials, String> {
    let source_id = normalize_optional_text(Some(item.source_id.clone()))
        .ok_or_else(|| "sourceId is required".to_string())?;
    let file_name = normalize_optional_text(Some(item.file_name.clone()))
        .ok_or_else(|| "fileName is required".to_string())?;
    let content = normalize_optional_text(Some(item.content.clone()))
        .ok_or_else(|| "content is required".to_string())?;
    let parsed: ImportedOauthCredentialsFile =
        serde_json::from_str(&content).map_err(|err| format!("invalid JSON: {err}"))?;
    if !parsed.source_type.eq_ignore_ascii_case("codex") {
        return Err("type must be codex".to_string());
    }
    let email =
        normalize_required_secret(&parsed.email, "email").map_err(|(_, message)| message)?;
    let chatgpt_account_id = normalize_required_secret(&parsed.account_id, "account_id")
        .map_err(|(_, message)| message)?;
    let access_token = normalize_required_secret(&parsed.access_token, "access_token")
        .map_err(|(_, message)| message)?;
    let refresh_token = normalize_required_secret(&parsed.refresh_token, "refresh_token")
        .map_err(|(_, message)| message)?;
    let id_token =
        normalize_required_secret(&parsed.id_token, "id_token").map_err(|(_, message)| message)?;
    let token_expires_at =
        resolve_imported_token_expires_at(parsed.expired.as_deref(), &access_token, &id_token)?;
    let mut claims = parse_chatgpt_jwt_claims(&id_token)
        .map_err(|err| format!("failed to parse id_token: {err}"))?;
    if let Some(jwt_email) = claims.email.as_deref()
        && !jwt_email.trim().eq_ignore_ascii_case(&email)
    {
        return Err("email does not match id_token".to_string());
    }
    if let Some(jwt_account_id) = claims.chatgpt_account_id.as_deref()
        && jwt_account_id.trim() != chatgpt_account_id.trim()
    {
        return Err("account_id does not match id_token".to_string());
    }
    claims.email = Some(email.clone());
    claims.chatgpt_account_id = Some(chatgpt_account_id.clone());
    Ok(NormalizedImportedOauthCredentials {
        source_id,
        file_name,
        email: email.clone(),
        display_name: email,
        chatgpt_account_id,
        token_expires_at,
        credentials: StoredOauthCredentials {
            access_token,
            refresh_token,
            id_token,
            token_type: normalize_optional_json_text(parsed.token_type)
                .or_else(|| Some("Bearer".to_string())),
        },
        claims,
    })
}

fn normalize_optional_json_text(value: Option<serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(value)) => normalize_optional_text(Some(value)),
        _ => None,
    }
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

fn random_base36(size: usize) -> Result<String, (StatusCode, String)> {
    const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    const LETTERS: &[u8; 26] = b"abcdefghijklmnopqrstuvwxyz";
    const DIGITS: &[u8; 10] = b"0123456789";
    let mut rng = OsRng;
    let mut output = Vec::with_capacity(size);
    for _ in 0..size {
        let idx = rng.gen_range(0..ALPHABET.len());
        output.push(ALPHABET[idx]);
    }
    let mut digit_pos = None;
    if size > 0 {
        let pos = rng.gen_range(0..size);
        output[pos] = DIGITS[rng.gen_range(0..DIGITS.len())];
        digit_pos = Some(pos);
    }
    if size > 1 {
        let mut letter_pos = rng.gen_range(0..size);
        if Some(letter_pos) == digit_pos {
            letter_pos = (letter_pos + 1) % size;
        }
        output[letter_pos] = LETTERS[rng.gen_range(0..LETTERS.len())];
    }
    String::from_utf8(output).map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}

fn generate_mailbox_local_name() -> Result<String, (StatusCode, String)> {
    const GIVEN_NAMES: &[&str] = &[
        "alex", "emma", "olivia", "liam", "sophia", "noah", "ava", "mia", "ethan", "nora", "lucas",
        "zoe",
    ];
    const FAMILY_NAMES: &[&str] = &[
        "carter", "ng", "morgan", "patel", "reed", "young", "kim", "bennett", "wong", "brooks",
    ];
    const ORG_NAMES: &[&str] = &[
        "northstar",
        "acorn",
        "harbor",
        "summit",
        "evergreen",
        "lattice",
        "brightpath",
        "aurora",
    ];
    const TEAM_NAMES: &[&str] = &[
        "ops", "research", "growth", "support", "finance", "design", "legal", "success",
    ];
    const UNIT_NAMES: &[&str] = &[
        "team", "desk", "hub", "group", "office", "lab", "studio", "center",
    ];

    let mut rng = OsRng;
    let suffix_len = rng.gen_range(3..=5);
    let suffix = random_base36(suffix_len)?;
    let maybe_join = |left: &str, right: &str, rng: &mut OsRng| match rng.gen_range(0..4) {
        0 => format!("{left}{right}"),
        1 => format!("{left}.{right}"),
        _ => format!("{left}-{right}"),
    };
    let mut local = match rng.gen_range(0..3) {
        0 => {
            let base = maybe_join(
                GIVEN_NAMES[rng.gen_range(0..GIVEN_NAMES.len())],
                FAMILY_NAMES[rng.gen_range(0..FAMILY_NAMES.len())],
                &mut rng,
            );
            maybe_join(&base, &suffix, &mut rng)
        }
        1 => {
            let base = maybe_join(
                ORG_NAMES[rng.gen_range(0..ORG_NAMES.len())],
                TEAM_NAMES[rng.gen_range(0..TEAM_NAMES.len())],
                &mut rng,
            );
            maybe_join(&base, &suffix, &mut rng)
        }
        _ => {
            let base = maybe_join(
                TEAM_NAMES[rng.gen_range(0..TEAM_NAMES.len())],
                UNIT_NAMES[rng.gen_range(0..UNIT_NAMES.len())],
                &mut rng,
            );
            maybe_join(&base, &suffix, &mut rng)
        }
    };
    if local.len() < 10 {
        local.push_str(&random_base36(10 - local.len())?);
    }
    Ok(local)
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

fn normalize_sticky_key_limit(raw: Option<i64>) -> i64 {
    match raw {
        Some(20 | 50 | 100) => raw.unwrap_or(DEFAULT_STICKY_KEY_LIMIT),
        _ => DEFAULT_STICKY_KEY_LIMIT,
    }
}

fn normalize_sticky_key_activity_hours(raw: Option<i64>) -> Option<i64> {
    match raw {
        Some(1 | 3 | 6 | 12 | 24) => raw,
        _ => None,
    }
}

pub(crate) fn resolve_sticky_key_selection(
    params: &AccountStickyKeysQuery,
) -> Result<AccountStickyKeySelection, (StatusCode, String)> {
    if params.limit.is_some() && params.activity_hours.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "provide either limit or activityHours, not both".to_string(),
        ));
    }
    if let Some(raw_hours) = params.activity_hours {
        let hours = normalize_sticky_key_activity_hours(Some(raw_hours)).ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "activityHours must be one of 1, 3, 6, 12, or 24".to_string(),
            )
        })?;
        return Ok(AccountStickyKeySelection::ActivityWindow(hours));
    }
    Ok(AccountStickyKeySelection::Count(
        normalize_sticky_key_limit(params.limit),
    ))
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

pub(crate) fn is_scope_permission_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("missing scopes")
        || msg.contains("insufficient permissions for this operation")
        || msg.contains("api.responses.write")
        || msg.contains("api.model.read")
}

fn is_upstream_unavailable_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("failed to contact oauth codex upstream")
        || msg.contains("oauth_upstream_unavailable")
        || msg.contains("failed to contact upstream")
        || msg.contains("connection refused")
        || msg.contains("connection reset")
        || msg.contains("timed out")
        || msg.contains("timeout")
        || msg.contains("temporarily unavailable")
        || msg.contains("service unavailable")
        || msg.contains("bad gateway")
        || msg.contains("gateway timeout")
        || msg.contains("upstream stream error")
        || msg.contains("upstream handshake timed out")
        || msg.contains("upstream response stream reported failure")
        || msg.contains("http 500")
        || msg.contains("http_500")
        || msg.contains("http 502")
        || msg.contains("http_502")
        || msg.contains("http 503")
        || msg.contains("http_503")
        || msg.contains("http 504")
        || msg.contains("http_504")
}

pub(crate) fn is_bridge_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("oauth bridge")
        || msg.contains("token exchange failed")
        || msg.contains("bridge upstream")
        || msg.contains("bridge token")
}

pub(crate) fn is_explicit_reauth_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("invalid_grant")
        || msg.contains("token has been invalidated")
        || msg.contains("token was invalidated")
        || msg.contains("invalidated oauth token")
        || msg.contains("refresh token expired")
        || msg.contains("refresh token revoked")
        || msg.contains("refresh token is invalid")
        || msg.contains("session expired")
        || msg.contains("please sign in again")
        || msg.contains("must sign in again")
        || msg.contains("re-authorize")
        || msg.contains("reauthorize")
}

fn is_upstream_rejected_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    is_scope_permission_error_message(message)
        || msg.contains("oauth_upstream_rejected_request")
        || msg.contains("upstream rejected")
        || msg.contains("forbidden")
        || msg.contains("unauthorized")
        || msg.contains("http 401")
        || msg.contains("http_401")
        || msg.contains("http 403")
        || msg.contains("http_403")
}

fn is_reauth_error(err: &anyhow::Error) -> bool {
    is_explicit_reauth_error_message(&err.to_string())
}

fn internal_error_tuple(err: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn request_runtime_error_tuple(err: impl ToString) -> (StatusCode, String) {
    let message = err.to_string();
    if is_group_node_shunt_unassigned_message(&message) {
        return (StatusCode::CONFLICT, message);
    }
    internal_error_tuple(message)
}

fn internal_error_html(err: impl ToString) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        render_callback_page(false, "OAuth callback failed", &err.to_string()),
    )
}
