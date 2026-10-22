use super::*;

fn sub2api_request(credentials: Value) -> ImportOauthCredentialFileRequest {
    ImportOauthCredentialFileRequest {
        source_id: "sub2api-member".to_string(),
        file_name: "sub2api-account.json".to_string(),
        content: json!({
            "name": "An arbitrary display name",
            "platform": "openai",
            "type": "oauth",
            "expires_at": 1,
            "credentials": credentials,
        })
        .to_string(),
    }
}

fn sub2api_credentials() -> Value {
    json!({
        "email": "member@example.com",
        "chatgpt_account_id": "acct_shared",
        "chatgpt_user_id": "user_member",
        "plan_type": "team",
        "access_token": "opaque-access-token",
        "expires_at": "2030-01-01T00:00:00Z",
    })
}

#[test]
fn sub2api_oauth_without_id_token_preserves_identity_in_existing_stored_shape() {
    let normalized = normalize_imported_oauth_credentials(&sub2api_request(sub2api_credentials()))
        .expect("normalize credentials without ID or refresh tokens");
    assert_eq!(normalized.email, "member@example.com");
    assert_eq!(normalized.chatgpt_account_id, "acct_shared");
    assert_eq!(normalized.chatgpt_user_id.as_deref(), Some("user_member"));
    assert_eq!(normalized.token_expires_at, "2030-01-01T00:00:00Z");
    assert_eq!(normalized.credentials.refresh_token, None);
    let stored = serde_json::to_value(StoredCredentials::Oauth(normalized.credentials))
        .expect("serialize current stored credential format");
    let StoredCredentials::Oauth(restored) =
        serde_json::from_value(stored).expect("read credentials using the unchanged storage model")
    else {
        panic!("expected OAuth credentials");
    };
    let claims = parse_chatgpt_jwt_claims(&restored.id_token).expect("parse adapted identity");
    assert_eq!(claims.chatgpt_user_id.as_deref(), Some("user_member"));
    assert_eq!(claims.chatgpt_plan_type.as_deref(), Some("team"));
}

#[test]
fn sub2api_oauth_derives_identity_and_expiry_from_access_token() {
    let payload = json!({
        "exp": 1893456000,
        "https://api.openai.com/profile": {"email": "token@example.com"},
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct_token",
            "chatgpt_user_id": "user_token",
            "chatgpt_plan_type": "k12",
        },
    });
    let token = format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(payload.to_string()));
    let normalized = normalize_imported_oauth_credentials(&sub2api_request(json!({
        "access_token": token,
    })))
    .expect("derive identity from access token");
    assert_eq!(normalized.email, "token@example.com");
    assert_eq!(normalized.chatgpt_account_id, "acct_token");
    assert_eq!(normalized.chatgpt_user_id.as_deref(), Some("user_token"));
    assert_eq!(normalized.claims.chatgpt_plan_type.as_deref(), Some("k12"));
    assert_eq!(normalized.token_expires_at, "2030-01-01T00:00:00Z");
}

#[test]
fn sub2api_oauth_rejects_identity_conflict_when_unrelated_claim_has_wrong_type() {
    let payload = json!({
        "email": "token@example.com",
        "https://api.openai.com/auth": {
            "chatgpt_account_id": "acct_shared",
            "chatgpt_user_id": "user_member",
            "chatgpt_plan_type": 123,
        },
        "exp": 1893456000,
    });
    let token = format!("e30.{}.sig", URL_SAFE_NO_PAD.encode(payload.to_string()));
    let mut credentials = sub2api_credentials();
    credentials["email"] = json!("different@example.com");
    credentials["access_token"] = json!(token);
    let error = normalize_imported_oauth_credentials(&sub2api_request(credentials))
        .expect_err("reject identity conflict despite malformed optional claim");
    assert!(
        error.contains("email does not match token identity"),
        "{error}"
    );
}

#[test]
fn sub2api_oauth_accepts_numeric_token_expiry() {
    for expiry in [
        json!(1893456000),
        json!(1893456000000_i64),
        json!("1893456000"),
        json!("1893456000000"),
    ] {
        let mut credentials = sub2api_credentials();
        credentials["expires_at"] = expiry;
        let normalized = normalize_imported_oauth_credentials(&sub2api_request(credentials))
            .expect("normalize numeric token expiry");
        assert_eq!(normalized.token_expires_at, "2030-01-01T00:00:00Z");
    }
}

#[test]
fn sub2api_oauth_rejects_token_identity_conflicts() {
    for field in ["email", "chatgpt_account_id", "chatgpt_user_id"] {
        let mut credentials = sub2api_credentials();
        credentials["id_token"] = json!(test_id_token(
            "member@example.com",
            Some("acct_shared"),
            Some("user_member"),
            Some("team"),
        ));
        credentials[field] = json!("different-identity");
        let error = normalize_imported_oauth_credentials(&sub2api_request(credentials))
            .expect_err("reject conflicting token identity");
        assert!(error.contains("does not match token identity"), "{error}");
    }
}

#[test]
fn sub2api_oauth_rejects_conflicting_duplicate_explicit_identity_fields() {
    for (field, record_key) in [
        ("email", "email"),
        ("account_id", "account_id"),
        ("chatgpt_user_id", "chatgpt_user_id"),
    ] {
        let mut credentials = sub2api_credentials();
        credentials[field] = json!("credential-value");
        let mut request = sub2api_request(credentials);
        let mut record: Value = serde_json::from_str(&request.content).expect("fixture JSON");
        record[record_key] = json!("record-value");
        request.content = record.to_string();
        let error = normalize_imported_oauth_credentials(&request)
            .expect_err("reject conflicting explicit identity fields");
        assert!(error.contains("conflicting explicit values"), "{error}");
    }
}

#[test]
fn sub2api_oauth_rejects_conflicting_identity_aliases_within_one_object() {
    for (first_key, second_key) in [
        ("chatgpt_account_id", "account_id"),
        ("chatgpt_user_id", "user_id"),
    ] {
        let mut credentials = sub2api_credentials();
        credentials[first_key] = json!("first-value");
        credentials[second_key] = json!("second-value");
        let error = normalize_imported_oauth_credentials(&sub2api_request(credentials))
            .expect_err("reject conflicting identity aliases");
        assert!(error.contains("conflicting explicit values"), "{error}");
    }
}

#[test]
fn sub2api_oauth_preserves_top_level_identity_aliases() {
    let mut request = sub2api_request(json!({
        "email": "member@example.com",
        "access_token": "opaque-access-token",
        "expires_at": "2030-01-01T00:00:00Z",
    }));
    let mut record: Value = serde_json::from_str(&request.content).expect("fixture JSON");
    record["chatgpt_account_id"] = json!("acct_top_level");
    record["chatgpt_user_id"] = json!("user_top_level");
    request.content = record.to_string();

    let normalized = normalize_imported_oauth_credentials(&request)
        .expect("preserve top-level identity aliases");
    assert_eq!(normalized.chatgpt_account_id, "acct_top_level");
    assert_eq!(
        normalized.chatgpt_user_id.as_deref(),
        Some("user_top_level")
    );
}

#[test]
fn sub2api_oauth_does_not_invent_identity_or_expiry() {
    for field in ["email", "chatgpt_account_id", "expires_at"] {
        let mut credentials = sub2api_credentials();
        credentials
            .as_object_mut()
            .expect("credential object")
            .remove(field);
        let error = normalize_imported_oauth_credentials(&sub2api_request(credentials))
            .expect_err("reject insufficient metadata");
        assert!(!error.contains("id_token is required"), "{error}");
    }
    let mut credentials = sub2api_credentials();
    credentials["expires_at"] = json!("not-a-date");
    assert!(normalize_imported_oauth_credentials(&sub2api_request(credentials)).is_err());
}

#[test]
fn sub2api_oauth_retains_member_metadata_when_supplied_id_token_omits_it() {
    let mut credentials = sub2api_credentials();
    let token = test_id_token("member@example.com", Some("acct_shared"), None, None);
    credentials["id_token"] = json!(token);
    let normalized = normalize_imported_oauth_credentials(&sub2api_request(credentials))
        .expect("preserve explicit member metadata");
    assert_eq!(normalized.credentials.id_token, token);
    assert_eq!(normalized.chatgpt_user_id.as_deref(), Some("user_member"));
    assert_eq!(normalized.claims.chatgpt_plan_type.as_deref(), Some("team"));
}

#[test]
fn sub2api_oauth_rejects_invalid_supplied_id_token() {
    for token in [json!("invalid-jwt"), json!(123)] {
        let mut credentials = sub2api_credentials();
        credentials["id_token"] = token;
        assert!(normalize_imported_oauth_credentials(&sub2api_request(credentials)).is_err());
    }
}

#[test]
fn sub2api_oauth_rejects_non_oauth_and_other_platforms() {
    for (platform, kind) in [("openai", "apikey"), ("claude", "oauth")] {
        let mut request = sub2api_request(sub2api_credentials());
        let mut record: Value = serde_json::from_str(&request.content).expect("fixture JSON");
        record["platform"] = json!(platform);
        record["type"] = json!(kind);
        request.content = record.to_string();
        let error = normalize_imported_oauth_credentials(&request).expect_err("unsupported record");
        assert_eq!(error, "only OpenAI OAuth sub2api accounts are supported");
    }
}
