use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;

#[test]
fn parse_access_token_expiry_reads_jwt_exp_and_rejects_invalid_payloads() {
    let header = URL_SAFE_NO_PAD.encode(b"{}");
    let numeric_payload = URL_SAFE_NO_PAD.encode(json!({"exp": 1_800_000_000}).to_string());
    let numeric_token = format!("{header}.{numeric_payload}.{header}");
    assert_eq!(
        parse_access_token_expiry(&numeric_token),
        Utc.timestamp_opt(1_800_000_000, 0).single()
    );

    let string_payload = URL_SAFE_NO_PAD.encode(json!({"exp": "1800000001"}).to_string());
    let string_token = format!("{header}.{string_payload}.{header}");
    assert_eq!(
        parse_access_token_expiry(&string_token),
        Utc.timestamp_opt(1_800_000_001, 0).single()
    );
    assert!(parse_access_token_expiry("opaque-access-token").is_none());
}

#[test]
fn normalize_usage_snapshot_distinguishes_single_and_partial_windows() {
    let single_window = normalize_usage_snapshot(&json!({
        "rateLimit": {
            "primaryWindow": {"usedPercent": 12.5}
        }
    }))
    .expect("normalize single window");
    assert_eq!(single_window.observed_window_candidates, 1);
    assert_eq!(single_window.parsed_window_count, 1);
    assert_eq!(single_window.window_parse_failure_count, 0);
    assert_eq!(
        single_window
            .primary
            .as_ref()
            .map(|window| window.window_duration_mins),
        Some(0)
    );

    let partial_windows = normalize_usage_snapshot(&json!({
        "rateLimit": {
            "primaryWindow": {"usedPercent": 12.5},
            "secondaryWindow": {"usedPercent": "not-a-number"}
        }
    }))
    .expect("normalize partial windows");
    assert_eq!(partial_windows.observed_window_candidates, 2);
    assert_eq!(partial_windows.parsed_window_count, 1);
    assert_eq!(partial_windows.window_parse_failure_count, 1);
}

#[test]
fn oauth_token_lifecycle_metadata_uses_explicit_expiry_sources() {
    let observed_at = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
    let access_token = format!(
        "e30.{}.sig",
        URL_SAFE_NO_PAD.encode(json!({"exp": 1_800_007_200}).to_string())
    );
    let response = OAuthTokenResponse {
        access_token: access_token.clone(),
        refresh_token: Some("rotated-refresh".to_string()),
        id_token: None,
        token_type: Some("Bearer".to_string()),
        expires_in: 3_600,
        refresh_token_expires_at: Some("2027-01-02T03:04:05Z".to_string()),
        refresh_token_expires_in: None,
    };
    let metadata = oauth_token_response_lifecycle_metadata(&response, &access_token, observed_at);
    assert_eq!(metadata.access_token_source.as_deref(), Some("jwt_exp"));
    assert_eq!(
        metadata.refresh_token_expires_at.as_deref(),
        Some("2027-01-02T03:04:05Z")
    );
    assert_eq!(
        metadata.refresh_token_source.as_deref(),
        Some("oauth_refresh_token_expires_at")
    );
    assert_eq!(metadata.refresh_token_state.as_deref(), Some("known"));
    assert!(metadata.refresh_token_rotated);
    assert!(metadata.refresh_token_metadata_update);
    let mut credentials = StoredOauthCredentials {
        access_token: "old-access-token".to_string(),
        refresh_token: Some("old-refresh-token".to_string()),
        id_token: "id-token".to_string(),
        token_type: Some("Bearer".to_string()),
    };
    let applied_expires_at = apply_oauth_token_response(
        &mut credentials,
        OAuthTokenResponse {
            access_token,
            refresh_token: Some("rotated-refresh".to_string()),
            id_token: None,
            token_type: Some("Bearer".to_string()),
            expires_in: 3_600,
            refresh_token_expires_at: None,
            refresh_token_expires_in: None,
        },
    );
    assert_eq!(applied_expires_at, "2027-01-15T10:00:00Z");

    let opaque_response = OAuthTokenResponse {
        access_token: "opaque-access-token".to_string(),
        refresh_token: None,
        id_token: None,
        token_type: None,
        expires_in: 3_600,
        refresh_token_expires_at: None,
        refresh_token_expires_in: None,
    };
    let opaque_metadata = oauth_token_response_lifecycle_metadata(
        &opaque_response,
        &opaque_response.access_token,
        observed_at,
    );
    assert_eq!(
        opaque_metadata.access_token_source.as_deref(),
        Some("oauth_expires_in")
    );
    assert_eq!(
        opaque_metadata.refresh_token_state.as_deref(),
        Some("not_provided")
    );
    assert!(!opaque_metadata.refresh_token_metadata_update);
}
