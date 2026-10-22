use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

type ImportRecord = serde_json::Map<String, Value>;

fn import_text(record: &ImportRecord, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        record
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    })
}

fn import_expiry(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str()
        && let Some(date) = parse_rfc3339_utc(text.trim())
    {
        return Some(format_utc_iso(date));
    }
    let epoch = value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())?;
    if !epoch.is_finite() {
        return None;
    }
    let seconds = if epoch > 1e11 { epoch / 1000.0 } else { epoch };
    // Stored token expiry uses whole seconds throughout the existing OAuth model.
    DateTime::<Utc>::from_timestamp(seconds.floor() as i64, 0).map(format_utc_iso)
}

fn import_claims(claims: ChatgptJwtClaims) -> ChatgptJwtClaims {
    ChatgptJwtClaims {
        email: normalize_optional_text(claims.email),
        chatgpt_account_id: normalize_optional_text(claims.chatgpt_account_id),
        chatgpt_user_id: normalize_optional_text(claims.chatgpt_user_id),
        chatgpt_plan_type: normalize_optional_text(claims.chatgpt_plan_type),
    }
}

pub(super) fn normalize_sub2api_oauth_account(record: &ImportRecord) -> Result<Value, String> {
    let credentials = record
        .get("credentials")
        .and_then(Value::as_object)
        .ok_or_else(|| "sub2api oauth account is missing credentials".to_string())?;
    let access_token = import_text(credentials, &["access_token", "accessToken"])
        .ok_or_else(|| "access_token is required".to_string())?;
    let input_id_token = import_text(credentials, &["id_token", "idToken"]);
    if ["id_token", "idToken"].iter().any(|key| {
        credentials
            .get(*key)
            .is_some_and(|value| !value.is_null() && !value.is_string())
    }) {
        return Err("id_token must be a JWT string".to_string());
    }
    let id_claims = import_claims(
        input_id_token
            .as_deref()
            .map(parse_chatgpt_jwt_claims)
            .transpose()
            .map_err(|error| format!("failed to parse id_token: {error}"))?
            .unwrap_or_default(),
    );
    let access_claims = import_claims(parse_chatgpt_jwt_claims(&access_token).unwrap_or_default());
    let email = import_text(credentials, &["email"])
        .or_else(|| import_text(record, &["email"]))
        .or_else(|| id_claims.email.clone())
        .or_else(|| access_claims.email.clone())
        .ok_or_else(|| "email is required".to_string())?;
    let account_id = import_text(
        credentials,
        &["chatgpt_account_id", "account_id", "chatgptAccountId"],
    )
    .or_else(|| import_text(record, &["account_id"]))
    .or_else(|| id_claims.chatgpt_account_id.clone())
    .or_else(|| access_claims.chatgpt_account_id.clone())
    .ok_or_else(|| "account_id is required".to_string())?;
    let user_id = import_text(
        credentials,
        &["chatgpt_user_id", "user_id", "chatgptUserId"],
    )
    .or_else(|| id_claims.chatgpt_user_id.clone())
    .or_else(|| access_claims.chatgpt_user_id.clone());
    let plan_type = import_text(credentials, &["plan_type", "chatgpt_plan_type"])
        .or_else(|| id_claims.chatgpt_plan_type.clone())
        .or_else(|| access_claims.chatgpt_plan_type.clone());
    for claims in [&id_claims, &access_claims] {
        for (field, actual, expected) in [
            ("email", Some(email.as_str()), claims.email.as_deref()),
            (
                "account_id",
                Some(account_id.as_str()),
                claims.chatgpt_account_id.as_deref(),
            ),
            (
                "chatgpt_user_id",
                user_id.as_deref(),
                claims.chatgpt_user_id.as_deref(),
            ),
        ] {
            if let (Some(actual), Some(expected)) = (actual, expected) {
                let matches = if field == "email" {
                    actual.eq_ignore_ascii_case(expected.trim())
                } else {
                    actual == expected.trim()
                };
                if !matches {
                    return Err(format!("{field} does not match token identity"));
                }
            }
        }
    }
    // The account-level expires_at is an account policy, not the token expiry.
    let explicit_expiry = credentials
        .get("expires_at")
        .into_iter()
        .chain(credentials.get("expired"))
        .chain(record.get("expired"))
        .find(|value| {
            !value.is_null() && !value.as_str().is_some_and(|text| text.trim().is_empty())
        });
    let expired = if let Some(value) = explicit_expiry {
        import_expiry(value).ok_or_else(|| "invalid OAuth token expiry".to_string())?
    } else {
        resolve_imported_token_expires_at(
            None,
            &access_token,
            input_id_token.as_deref().unwrap_or(""),
        )?
    };
    let id_token = input_id_token.unwrap_or_else(|| {
        // Match Session import's compatibility envelope; this grants no upstream authorization.
        let auth = json!({
            "chatgpt_account_id": account_id,
            "chatgpt_user_id": user_id,
            "chatgpt_plan_type": plan_type,
        });
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT","cpa_synthetic":true}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            json!({
                "email": email,
                "iat": Utc::now().timestamp(),
                "exp": parse_rfc3339_utc(&expired).map(|date| date.timestamp()),
                "https://api.openai.com/auth": auth,
            })
            .to_string(),
        );
        format!("{header}.{payload}.signature")
    });
    Ok(json!({
        "email": email,
        "account_id": account_id,
        "chatgpt_user_id": user_id,
        "plan_type": plan_type,
        "access_token": access_token,
        "refresh_token": import_text(credentials, &["refresh_token", "refreshToken"]),
        "id_token": id_token,
        "token_type": import_text(credentials, &["token_type", "tokenType"]),
        "expired": expired,
    }))
}
