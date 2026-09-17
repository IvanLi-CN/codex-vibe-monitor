pub(crate) fn normalize_required_display_name(raw: &str) -> Result<String, (StatusCode, String)> {
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

pub(crate) fn validate_group_note_target(
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

pub(crate) fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn normalize_upstream_account_group_name(value: Option<String>) -> String {
    normalize_optional_text(value)
        .unwrap_or_else(|| DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME.to_string())
}

pub(crate) fn normalize_legacy_ungrouped_group_name(value: Option<String>) -> Option<String> {
    normalize_optional_text(value).filter(|value| value != DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME)
}

pub(crate) fn normalize_optional_upstream_base_url(
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

pub(crate) fn resolve_pool_account_upstream_base_url(
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

pub(crate) fn normalize_required_secret(
    raw: &str,
    field_name: &str,
) -> Result<String, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((StatusCode::BAD_REQUEST, format!("{field_name} is required")));
    }
    Ok(value.to_string())
}

pub(crate) fn normalize_limit_unit(value: Option<String>) -> String {
    value
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_API_KEY_LIMIT_UNIT.to_string())
}

pub(crate) fn validate_local_limits(
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

pub(crate) fn is_import_invalid_error_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    is_explicit_reauth_error_message(message)
        || is_scope_permission_error_message(message)
        || normalized.contains("returned 400")
        || normalized.contains("returned 401")
        || normalized.contains("returned 403")
}

pub(crate) fn persisted_usage_snapshot_is_exhausted(
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

pub(crate) fn imported_snapshot_is_exhausted(snapshot: &NormalizedUsageSnapshot) -> bool {
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

pub(crate) fn persisted_usage_sample_is_exhausted(
    sample: Option<&UpstreamAccountSampleRow>,
) -> bool {
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

pub(crate) fn routing_candidate_snapshot_is_exhausted(
    candidate: &AccountRoutingCandidateRow,
) -> bool {
    persisted_usage_snapshot_is_exhausted(
        candidate.primary_used_percent,
        candidate.secondary_used_percent,
        candidate.credits_has_credits.map(|value| value != 0),
        candidate.credits_unlimited.map(|value| value != 0),
        candidate.credits_balance.as_deref(),
    )
}

pub(crate) fn imported_match_key(
    chatgpt_user_id: Option<&str>,
    email: &str,
    account_id: &str,
) -> String {
    if let Some(normalized_user_id) = chatgpt_user_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
    {
        return format!("user:{normalized_user_id}");
    }
    let normalized_account_id = account_id.trim().to_ascii_lowercase();
    if !normalized_account_id.is_empty() {
        return format!("account:{normalized_account_id}");
    }
    format!("email:{}", email.trim().to_ascii_lowercase())
}

pub(crate) fn import_match_summary_from_row(row: &UpstreamAccountRow) -> ImportedOauthMatchSummary {
    ImportedOauthMatchSummary {
        account_id: row.id,
        display_name: row.display_name.clone(),
        group_name: row.group_name.clone(),
        status: effective_account_status(row),
    }
}

pub(crate) fn normalize_imported_oauth_credentials(
    item: &ImportOauthCredentialFileRequest,
) -> Result<NormalizedImportedOauthCredentials, String> {
    let source_id = normalize_optional_text(Some(item.source_id.clone()))
        .ok_or_else(|| "sourceId is required".to_string())?;
    let file_name = normalize_optional_text(Some(item.file_name.clone()))
        .ok_or_else(|| "fileName is required".to_string())?;
    let content = normalize_optional_text(Some(item.content.clone()))
        .ok_or_else(|| "content is required".to_string())?;
    let parsed: Value =
        serde_json::from_str(&content).map_err(|err| format!("invalid JSON: {err}"))?;
    let normalized_value = normalize_imported_oauth_record_value(&parsed)?;
    let email = normalize_required_secret(
        &required_imported_oauth_string(&normalized_value, &["email"], "email")?,
        "email",
    )
    .map_err(|(_, message)| message)?;
    let chatgpt_account_id = normalize_required_secret(
        &required_imported_oauth_string(
            &normalized_value,
            &["account_id", "chatgpt_account_id"],
            "account_id",
        )?,
        "account_id",
    )
    .map_err(|(_, message)| message)?;
    let access_token = normalize_required_secret(
        &required_imported_oauth_string(&normalized_value, &["access_token"], "access_token")?,
        "access_token",
    )
    .map_err(|(_, message)| message)?;
    let refresh_token = optional_imported_oauth_string(&normalized_value, &["refresh_token"]);
    let id_token = normalize_required_secret(
        &required_imported_oauth_string(&normalized_value, &["id_token"], "id_token")?,
        "id_token",
    )
    .map_err(|(_, message)| message)?;
    let token_expires_at = resolve_imported_token_expires_at(
        optional_imported_oauth_string(&normalized_value, &["expired"]).as_deref(),
        &access_token,
        &id_token,
    )?;
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
    let chatgpt_user_id = claims
        .chatgpt_user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok(NormalizedImportedOauthCredentials {
        source_id,
        file_name,
        email: email.clone(),
        display_name: email,
        chatgpt_account_id,
        chatgpt_user_id,
        token_expires_at,
        credentials: StoredOauthCredentials {
            access_token,
            refresh_token,
            id_token,
            token_type: optional_imported_oauth_string(&normalized_value, &["token_type"])
                .or_else(|| Some("Bearer".to_string())),
        },
        claims,
    })
}

fn required_imported_oauth_string(
    value: &Value,
    keys: &[&str],
    field_name: &str,
) -> Result<String, String> {
    optional_imported_oauth_json_text(value, keys)
        .ok_or_else(|| format!("{field_name} is required"))
}

fn optional_imported_oauth_string(value: &Value, keys: &[&str]) -> Option<String> {
    optional_imported_oauth_json_text(value, keys)
}

fn optional_imported_oauth_json_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|raw| match raw {
            Value::String(text) => normalize_optional_text(Some(text.clone())),
            _ => None,
        })
}

fn normalize_imported_oauth_record_value(value: &Value) -> Result<Value, String> {
    let Some(record) = value.as_object() else {
        return Err("expected one credential JSON object".to_string());
    };

    if record
        .get("platform")
        .and_then(value_as_string)
        .is_some_and(|value| value.eq_ignore_ascii_case("openai"))
        && record
            .get("type")
            .and_then(value_as_string)
            .is_some_and(|value| value.eq_ignore_ascii_case("oauth"))
    {
        let credentials = record
            .get("credentials")
            .and_then(Value::as_object)
            .ok_or_else(|| "sub2api oauth account is missing credentials".to_string())?;
        let mut normalized = serde_json::Map::new();
        for (target_key, source_key) in [
            ("email", "email"),
            ("access_token", "access_token"),
            ("refresh_token", "refresh_token"),
            ("id_token", "id_token"),
            ("token_type", "token_type"),
            ("expired", "expires_at"),
            ("plan_type", "plan_type"),
            ("chatgpt_user_id", "chatgpt_user_id"),
        ] {
            if let Some(value) = credentials.get(source_key) {
                normalized.insert(target_key.to_string(), value.clone());
            }
        }
        if let Some(value) = credentials
            .get("chatgpt_account_id")
            .or_else(|| credentials.get("account_id"))
        {
            normalized.insert("account_id".to_string(), value.clone());
        }
        return Ok(Value::Object(normalized));
    }

    if record
        .get("type")
        .and_then(value_as_string)
        .is_some_and(|value| value.eq_ignore_ascii_case("sub2api-data"))
    {
        return Err(
            "sub2api-data export packages must be expanded into individual OpenAI OAuth accounts before validation"
                .to_string(),
        );
    }

    Ok(value.clone())
}

pub(crate) fn normalize_optional_json_text(value: Option<serde_json::Value>) -> Option<String> {
    match value {
        Some(serde_json::Value::String(value)) => normalize_optional_text(Some(value)),
        _ => None,
    }
}

pub(crate) fn parse_rfc3339_utc(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

pub(crate) fn code_challenge_for_verifier(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

pub(crate) fn random_hex(size: usize) -> Result<String, (StatusCode, String)> {
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

#[cfg(test)]
pub(crate) fn random_base36(size: usize) -> Result<String, (StatusCode, String)> {
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

pub(crate) fn format_window_label(window_duration_mins: i64) -> String {
    match window_duration_mins {
        300 => "5h quota".to_string(),
        10_080 => "7d quota".to_string(),
        mins if mins % (60 * 24) == 0 => format!("{}d quota", mins / (60 * 24)),
        mins if mins % 60 == 0 => format!("{}h quota", mins / 60),
        mins => format!("{}m quota", mins),
    }
}

pub(crate) fn format_percent(value: f64) -> String {
    if (value.fract()).abs() < 0.05 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

pub(crate) fn format_compact_decimal(value: f64) -> String {
    let rounded = format!("{value:.2}");
    rounded
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub(crate) fn mask_api_key(api_key: &str) -> String {
    if api_key.len() <= 8 {
        return "••••••••".to_string();
    }
    format!("{}••••{}", &api_key[..4], &api_key[api_key.len() - 4..])
}

pub(crate) fn normalize_sticky_key_limit(raw: Option<i64>) -> i64 {
    match raw {
        Some(20 | 50 | 100) => raw.unwrap_or(DEFAULT_STICKY_KEY_LIMIT),
        _ => DEFAULT_STICKY_KEY_LIMIT,
    }
}

pub(crate) fn normalize_sticky_key_activity_hours(raw: Option<i64>) -> Option<i64> {
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

pub(crate) fn seconds_to_window_minutes(seconds: i64) -> i64 {
    (seconds + 59) / 60
}

pub(crate) fn optional_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(value_as_string)
}

pub(crate) fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(raw) => Some(raw.clone()),
        Value::Number(raw) => Some(raw.to_string()),
        _ => None,
    }
}

pub(crate) fn value_as_bool(value: &Value) -> Option<bool> {
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

pub(crate) fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(raw) => raw.as_f64(),
        Value::String(raw) => raw.parse::<f64>().ok(),
        _ => None,
    }
}

pub(crate) fn value_as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(raw) => raw.as_i64(),
        Value::String(raw) => raw.parse::<i64>().ok(),
        _ => None,
    }
}

pub(crate) fn value_as_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    value_as_i64(value).and_then(|seconds| Utc.timestamp_opt(seconds, 0).single())
}

pub(crate) fn extract_error_message(body: &str) -> String {
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

pub(crate) fn is_upstream_unavailable_error_message(message: &str) -> bool {
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

pub(crate) fn is_upstream_rejected_error_message(message: &str) -> bool {
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

pub(crate) fn maintenance_upstream_rejected_error_message(message: &str) -> bool {
    let msg = message.to_ascii_lowercase();
    msg.contains("deactivated_workspace")
        || msg.contains("upstream_http_402")
        || (msg.contains("oauth_upstream_rejected_request")
            && (msg.contains("payment required")
                || msg.contains("http 402")
                || msg.contains("http_402")
                || msg.contains("responded with 402")
                || msg.contains("returned 402")))
}

pub(crate) fn is_reauth_error(err: &anyhow::Error) -> bool {
    is_explicit_reauth_error_message(&err.to_string())
}

pub(crate) fn internal_error_tuple(err: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

pub(crate) fn request_runtime_error_tuple(err: impl ToString) -> (StatusCode, String) {
    let message = err.to_string();
    if is_group_node_shunt_unassigned_message(&message) {
        return (StatusCode::CONFLICT, message);
    }
    internal_error_tuple(message)
}

pub(crate) fn internal_error_html(err: impl ToString) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        render_callback_page(false, "OAuth callback failed", &err.to_string()),
    )
}

#[cfg(test)]
mod account_maintenance_egress_throttle_tests {
    use super::*;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        ensure_upstream_accounts_schema(&pool)
            .await
            .expect("ensure upstream account schema");
        pool
    }

    fn selected_proxy(key: &str) -> SelectedForwardProxy {
        SelectedForwardProxy {
            key: key.to_string(),
            source: "manual".to_string(),
            display_name: key.to_string(),
            endpoint_url: None,
            endpoint_url_raw: None,
            egress_ip: None,
        }
    }

    #[tokio::test]
    async fn reserve_slot_enforces_ten_seconds_per_egress_key() {
        let pool = test_pool().await;
        let proxy = selected_proxy("jp-edge-01");
        let other_proxy = selected_proxy("sg-edge-01");

        reserve_account_maintenance_egress_slot(&pool, &proxy)
            .await
            .expect("first egress should reserve");
        let err = reserve_account_maintenance_egress_slot(&pool, &proxy)
            .await
            .expect_err("second same-egress request should throttle");
        let throttle = err
            .downcast_ref::<AccountMaintenanceEgressThrottleError>()
            .expect("throttle error");
        assert_eq!(throttle.proxy_key, "jp-edge-01");
        assert!(throttle.retry_after_secs > 0);
        assert!(throttle.retry_after_secs <= 10);

        reserve_account_maintenance_egress_slot(&pool, &other_proxy)
            .await
            .expect("different egress should not be throttled");
    }

    #[tokio::test]
    async fn runtime_wait_retries_until_egress_slot_is_available() {
        let pool = test_pool().await;
        let proxy = selected_proxy("jp-edge-01");

        reserve_account_maintenance_egress_slot(&pool, &proxy)
            .await
            .expect("first egress should reserve");
        let nearly_available_at = format_utc_iso(Utc::now() - ChronoDuration::seconds(9));
        sqlx::query(
            r#"
            UPDATE pool_upstream_account_egress_throttle
            SET last_sent_at = ?2, updated_at = ?2
            WHERE egress_key = ?1
            "#,
        )
        .bind(&proxy.key)
        .bind(&nearly_available_at)
        .execute(&pool)
        .await
        .expect("seed near-expired egress throttle slot");

        reserve_account_maintenance_egress_slot_with_bounded_wait(&pool, &proxy, 3)
            .await
            .expect("runtime wait should retry once the egress slot is available");

        let err = reserve_account_maintenance_egress_slot(&pool, &proxy)
            .await
            .expect_err("runtime wait should reserve the egress slot before returning");
        assert!(
            err.downcast_ref::<AccountMaintenanceEgressThrottleError>()
                .is_some()
        );
    }

    #[tokio::test]
    async fn runtime_wait_preserves_deferred_path_after_budget_exhaustion() {
        let pool = test_pool().await;
        let proxy = selected_proxy("jp-edge-01");

        reserve_account_maintenance_egress_slot(&pool, &proxy)
            .await
            .expect("first egress should reserve");
        let err = reserve_account_maintenance_egress_slot_with_bounded_wait(&pool, &proxy, 0)
            .await
            .expect_err("exhausted wait budget should preserve throttle error");

        let throttle = err
            .downcast_ref::<AccountMaintenanceEgressThrottleError>()
            .expect("throttle error");
        assert_eq!(throttle.proxy_key, "jp-edge-01");
    }
}
