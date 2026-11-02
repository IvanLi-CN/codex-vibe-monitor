use super::*;
use serde_json::json;

#[test]
fn refresh_error_classification_distinguishes_access_and_refresh_tokens() {
    let (usage_code, _) = classify_refresh_error(
        "usage endpoint returned 401: access token revoked",
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
    );
    assert_ne!(usage_code, "refresh_token_invalid");

    let (credential_code, _) = classify_refresh_error(
        "OAuth token endpoint returned 400: refresh token revoked",
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    );
    assert_eq!(credential_code, "refresh_token_invalid");
    let (invalid_token_code, _) = classify_refresh_error(
        r#"OAuth token endpoint returned 400: {"error":"invalid_token"}"#,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    );
    assert_eq!(invalid_token_code, "refresh_token_invalid");
    let nested_invalid_token = extract_error_message(
        r#"{"error":{"code":"invalid_token","message":"token is no longer valid"}}"#,
    );
    assert!(nested_invalid_token.contains("invalid_token"));
    let (nested_invalid_token_code, _) = classify_refresh_error(
        &nested_invalid_token,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    );
    assert_eq!(nested_invalid_token_code, "refresh_token_invalid");
    assert_eq!(
        refresh_http_status("OAuth token endpoint returned 400: invalid_grant"),
        Some(StatusCode::BAD_REQUEST)
    );
    let (infrastructure_code, _) = classify_refresh_error(
        "TLS certificate has expired while connecting to the token endpoint",
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    );
    assert_ne!(infrastructure_code, "refresh_token_invalid");
    let (rate_limit_code, _) = classify_refresh_error(
        "HTTP 429 rate_limit_exceeded: requests per minute limit exceeded",
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
    );
    assert_eq!(rate_limit_code, "upstream_http_429_rate_limit");
    let (session_expired_code, _) = classify_refresh_error(
        "OAuth token endpoint returned 400: session expired",
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    );
    assert_eq!(session_expired_code, "refresh_token_invalid");
    let (session_expired_underscore_code, _) = classify_refresh_error(
        "OAuth token endpoint returned 400: session_expired",
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    );
    assert_eq!(session_expired_underscore_code, "refresh_token_invalid");
    let (canonical_quota_code, _) = classify_refresh_error(
        "HTTP 429: The usage limit has been reached",
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
    );
    assert_eq!(canonical_quota_code, "upstream_http_429_quota_exhausted");
    let (insufficient_quota_code, _) = classify_refresh_error(
        "HTTP 429: insufficient_quota",
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
    );
    assert_eq!(insufficient_quota_code, "upstream_http_429_quota_exhausted");
    let callback = render_callback_page(false, "<title>", "<script>alert(1)</script>");
    assert!(!callback.contains("<title><title>"));
    assert!(callback.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!is_explicit_reauth_error_message("access token expired"));
    assert!(is_explicit_reauth_error_message("refresh token expired"));
}

#[tokio::test]
async fn refresh_state_claim_prevents_concurrent_attempts_for_one_operation() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Refresh Claim").await;

    let first_lease = begin_upstream_account_refresh_attempt_with_lease(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        "sync_maintenance",
    )
    .await
    .expect("first refresh claim")
    .expect("first refresh lease");
    assert!(
        !begin_upstream_account_refresh_attempt(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            "sync_maintenance",
        )
        .await
        .expect("second refresh claim")
    );

    sqlx::query(
        "UPDATE pool_upstream_account_refresh_state SET last_attempt_at = ?1 WHERE account_id = ?2 AND operation = ?3",
    )
    .bind(format_utc_iso(
        Utc::now() - ChronoDuration::seconds(UPSTREAM_ACCOUNT_REFRESH_RETRY_INTERVAL_SECS + 1),
    ))
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS)
    .execute(&pool)
    .await
    .expect("age refresh claim");
    let second_lease = begin_upstream_account_refresh_attempt_with_lease(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        "sync_maintenance",
    )
    .await
    .expect("stale refresh claim")
    .expect("second refresh lease");
    assert_ne!(first_lease, second_lease);
    assert!(
        !complete_upstream_account_refresh_success_with_lease(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            "sync_maintenance",
            None,
            Some(&first_lease),
        )
        .await
        .expect("stale refresh completion")
    );
    assert!(
        complete_upstream_account_refresh_success_with_lease(
            &pool,
            account_id,
            UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
            "sync_maintenance",
            None,
            Some(&second_lease),
        )
        .await
        .expect("current refresh completion")
    );
}

#[tokio::test]
async fn stale_usage_refresh_lease_cannot_persist_snapshot() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Stale Usage Snapshot").await;
    let first_lease = begin_upstream_account_refresh_attempt_with_lease(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
        "sync_maintenance",
    )
    .await
    .expect("first usage claim")
    .expect("first usage lease");

    sqlx::query(
        "UPDATE pool_upstream_account_refresh_state SET last_attempt_at = ?1 WHERE account_id = ?2 AND operation = ?3",
    )
    .bind(format_utc_iso(
        Utc::now() - ChronoDuration::seconds(UPSTREAM_ACCOUNT_REFRESH_RETRY_INTERVAL_SECS + 1),
    ))
    .bind(account_id)
    .bind(UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS)
    .execute(&pool)
    .await
    .expect("age usage claim");

    let second_lease = begin_upstream_account_refresh_attempt_with_lease(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
        "sync_maintenance",
    )
    .await
    .expect("stale usage claim")
    .expect("second usage lease");
    let snapshot = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "codex".to_string(),
        limit_name: Some("Codex".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 12.0,
            window_duration_mins: 300,
            resets_at: Some("2026-03-17T14:00:00Z".to_string()),
        }),
        secondary: Some(NormalizedUsageWindow {
            used_percent: 24.0,
            window_duration_mins: 10_080,
            resets_at: Some("2026-03-18T00:00:00Z".to_string()),
        }),
        credits: None,
        ..Default::default()
    };

    assert!(
        !persist_usage_snapshot_with_lease(
            &pool,
            account_id,
            Some("team"),
            &snapshot,
            30,
            Some(&first_lease),
        )
        .await
        .expect("stale usage persistence")
    );
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pool_upstream_account_limit_samples WHERE account_id = ?1",
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("count usage snapshots");
    assert_eq!(count, 0);
    assert!(
        persist_usage_snapshot_with_lease(
            &pool,
            account_id,
            Some("team"),
            &snapshot,
            30,
            Some(&second_lease),
        )
        .await
        .expect("current usage persistence")
    );
}

#[tokio::test]
async fn credential_refresh_keeps_access_token_rejection_during_transient_retry() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Rejected Access Token").await;
    record_upstream_account_refresh_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        "usage_windows_refresh",
        "usage endpoint returned 401",
        "access_token_rejected",
        "额度接口拒绝当前 Access Token，下一轮独立凭据刷新将重试。",
        false,
        Some(StatusCode::UNAUTHORIZED),
    )
    .await
    .expect("record access token rejection");
    record_upstream_account_refresh_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        "maintenance",
        "token endpoint returned 503",
        "upstream_http_5xx",
        "上游返回 503，等待下一分钟重试。",
        false,
        Some(StatusCode::SERVICE_UNAVAILABLE),
    )
    .await
    .expect("record transient credential failure");

    let status = load_upstream_account_refresh_status(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await
    .expect("load credential refresh status");
    assert_eq!(status.error_code.as_deref(), Some("access_token_rejected"));
    assert!(
        status
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("upstream_http_5xx"))
    );
}

#[tokio::test]
async fn manual_usage_refresh_updates_windows_without_refreshing_credentials() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![(
                StatusCode::OK,
                json!({
                    "planType": "team",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 12,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        },
                        "secondaryWindow": {
                            "usedPercent": 18,
                            "windowDurationMins": 10080,
                            "resetsAt": 1771927200
                        }
                    }
                }),
            )],
            json!({
                "access_token": "must-not-be-requested",
                "refresh_token": "must-not-be-requested",
                "id_token": test_id_token(
                    "manual-usage@example.com",
                    Some("org_manual_usage"),
                    Some("user_manual_usage"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Manual Usage OAuth",
        "manual-usage@example.com",
        "org_manual_usage",
        "user_manual_usage",
    )
    .await;
    let before = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account before usage refresh")
        .expect("account exists before usage refresh");
    let before_credentials = before
        .encrypted_credentials
        .clone()
        .expect("encrypted credentials before usage refresh");
    sqlx::query("UPDATE pool_upstream_accounts SET status = ?2, updated_at = ?3 WHERE id = ?1")
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
        .bind(format_utc_iso(Utc::now()))
        .execute(&state.pool)
        .await
        .expect("mark account as needing reauthorization");

    state
        .upstream_accounts
        .account_ops
        .run_manual_usage_refresh(state.clone(), account_id)
        .await
        .expect("manual usage refresh should complete");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after usage refresh")
        .expect("account exists after usage refresh");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(
        after.encrypted_credentials.as_deref(),
        Some(before_credentials.as_str()),
        "usage refresh must not rewrite OAuth credentials"
    );
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
    assert_eq!(token_requests.load(Ordering::SeqCst), 0);

    let usage_status = load_upstream_account_refresh_status(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
    )
    .await
    .expect("load usage refresh state");
    assert_eq!(usage_status.state, UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED);
    assert!(usage_status.last_success_at.is_some());
    let credential_status = load_upstream_account_refresh_status(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await
    .expect("load credential refresh state");
    assert_eq!(credential_status.state, UPSTREAM_ACCOUNT_REFRESH_STATE_IDLE);

    let actions: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, source FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load usage refresh actions");
    assert_eq!(
        actions.first(),
        Some(&(
            UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_SUCCEEDED.to_string(),
            UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH.to_string(),
        ))
    );
    assert!(actions.iter().all(|(action, _)| {
        action != UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REFRESH_SUCCEEDED
            && action != UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REFRESH_FAILED
            && action != UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REAUTH_REQUIRED
    }));
    server.abort();
}

#[tokio::test]
async fn manual_credential_refresh_updates_credentials_without_refreshing_usage() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![],
            json!({
                "access_token": "manual-credential-access-token",
                "refresh_token": "manual-credential-rotated-refresh-token",
                "id_token": test_id_token(
                    "manual-credential@example.com",
                    Some("org_manual_credential"),
                    Some("user_manual_credential"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Manual Credential OAuth",
        "manual-credential@example.com",
        "org_manual_credential",
        "user_manual_credential",
    )
    .await;
    let before = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account before credential refresh")
        .expect("account exists before credential refresh");
    let before_credentials = before
        .encrypted_credentials
        .clone()
        .expect("encrypted credentials before credential refresh");

    state
        .upstream_accounts
        .account_ops
        .run_manual_credential_refresh(state.clone(), account_id)
        .await
        .expect("manual credential refresh should complete");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after credential refresh")
        .expect("account exists after credential refresh");
    assert_ne!(
        after.encrypted_credentials.as_deref(),
        Some(before_credentials.as_str()),
        "credential refresh must persist the new OAuth credentials"
    );
    assert_eq!(usage_requests.load(Ordering::SeqCst), 0);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);

    let decrypted = decrypt_credentials(
        crypto_key,
        after
            .encrypted_credentials
            .as_deref()
            .expect("encrypted credentials after credential refresh"),
    )
    .expect("decrypt refreshed credentials");
    let StoredCredentials::Oauth(credentials) = decrypted else {
        panic!("unexpected credential kind after manual credential refresh")
    };
    assert_eq!(credentials.access_token, "manual-credential-access-token");
    assert_eq!(
        credentials.refresh_token.as_deref(),
        Some("manual-credential-rotated-refresh-token")
    );

    let credential_status = load_upstream_account_refresh_status(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await
    .expect("load credential refresh state");
    assert_eq!(
        credential_status.state,
        UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED
    );
    assert!(credential_status.last_success_at.is_some());
    let next_attempt_at = credential_status
        .next_attempt_at
        .as_deref()
        .and_then(parse_rfc3339_utc)
        .expect("opaque access token should use expires_in for next refresh");
    let now = Utc::now();
    assert!(
        next_attempt_at > now + ChronoDuration::minutes(30)
            && next_attempt_at < now + ChronoDuration::hours(2),
        "opaque access token refresh should follow expires_in instead of the eight-day fallback: {next_attempt_at}"
    );
    let usage_status = load_upstream_account_refresh_status(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS,
    )
    .await
    .expect("load usage refresh state");
    assert_eq!(usage_status.state, UPSTREAM_ACCOUNT_REFRESH_STATE_IDLE);

    let actions: Vec<(String, String)> = sqlx::query_as(
        "SELECT action, source FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load credential refresh actions");
    assert_eq!(
        actions.first(),
        Some(&(
            UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REFRESH_SUCCEEDED.to_string(),
            UPSTREAM_ACCOUNT_ACTION_SOURCE_CREDENTIAL_REFRESH.to_string()
        ))
    );
    assert!(actions.iter().all(|(action, _)| {
        action != UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_SUCCEEDED
            && action != UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_PARTIAL
            && action != UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_FAILED
    }));
    server.abort();
}

#[tokio::test]
async fn manual_usage_refresh_reactivates_quota_exhausted_account_once_snapshot_recovers() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                },
                "secondaryWindow": {
                    "usedPercent": 18,
                    "windowDurationMins": 10080,
                    "resetsAt": 1771927200
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Recovered OAuth Manual Usage",
        "recovered-manual@example.com",
        "org_recovered_manual",
        "user_recovered_manual",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    state
        .upstream_accounts
        .account_ops
        .run_manual_usage_refresh(state.clone(), account_id)
        .await
        .expect("manual usage refresh");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after manual recovery")
        .expect("oauth row exists after manual recovery");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_SUCCEEDED)
    );
    server.abort();
}

#[tokio::test]
async fn oauth_sync_usage_auth_failure_retries_without_refreshing_credentials() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![
                    (
                        StatusCode::UNAUTHORIZED,
                        json!({
                            "error": {
                                "message": "Session cookie expired during usage snapshot"
                            }
                        }),
                    ),
                    (
                        StatusCode::FORBIDDEN,
                        json!({
                            "error": {
                                "message": "Authentication token has been invalidated, please sign in again"
                            }
                        }),
                    ),
                ],
                json!({
                    "access_token": "refreshed-access-token",
                    "refresh_token": "refresh-token-rotated",
                    "id_token": test_id_token(
                        "reauth-required@example.com",
                        Some("org_retry_reauth"),
                        Some("user_retry_reauth"),
                        Some("team"),
                    ),
                    "token_type": "Bearer",
                    "expires_in": 3600
                }),
            )
            .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Needs Reauth OAuth",
        "reauth-required@example.com",
        "org_retry_reauth",
        "user_retry_reauth",
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    let original_access_token = match decrypt_credentials(
        crypto_key,
        row.encrypted_credentials
            .as_deref()
            .expect("encrypted oauth credentials"),
    )
    .expect("decrypt original credentials")
    {
        StoredCredentials::Oauth(credentials) => credentials.access_token,
        StoredCredentials::ApiKey(_) => panic!("expected OAuth credentials"),
    };

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after retry failure")
        .expect("oauth row exists after retry failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert_eq!(after.last_action_http_status, Some(401));
    assert_eq!(
        after.last_error.as_deref(),
        Some("上游返回 HTTP 401，等待下一分钟重试。")
    );
    assert!(after.last_action_at.is_some());

    let decrypted = decrypt_credentials(
        crypto_key,
        after
            .encrypted_credentials
            .as_deref()
            .expect("encrypted oauth credentials"),
    )
    .expect("decrypt refreshed credentials");
    let StoredCredentials::Oauth(credentials) = decrypted else {
        panic!("unexpected credential kind after refresh")
    };
    assert_eq!(credentials.access_token, original_access_token);
    assert_eq!(credentials.refresh_token.as_deref(), Some("refresh-token"));

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        summary.health_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.reason_code.as_deref()),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert_eq!(
        detail.usage_window_refresh.error_code.as_deref(),
        Some("upstream_http_401")
    );
    assert_eq!(
        detail.credential_refresh.state,
        UPSTREAM_ACCOUNT_REFRESH_STATE_RETRYING
    );
    assert_eq!(
        detail.credential_refresh.error_code.as_deref(),
        Some("access_token_rejected")
    );
    assert!(detail.credential_refresh.next_attempt_at.is_some());
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
    assert_eq!(token_requests.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_usage_failure_does_not_retry_with_token_refresh() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![
                (
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                ),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "error": {
                            "message": "gateway temporarily unavailable"
                        }
                    }),
                ),
            ],
            json!({
                "access_token": "refreshed-temporary-token",
                "refresh_token": "refresh-token-rotated",
                "id_token": test_id_token(
                    "transport-failure@example.com",
                    Some("org_retry_gateway"),
                    Some("user_retry_gateway"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Gateway Failure OAuth",
        "transport-failure@example.com",
        "org_retry_gateway",
        "user_retry_gateway",
    )
    .await;
    seed_route_cooldown(
        &state.pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after gateway failure")
        .expect("oauth row exists after gateway failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert_eq!(after.last_action_http_status, Some(401));
    assert_eq!(
        after.last_error.as_deref(),
        Some("上游返回 HTTP 401，等待下一分钟重试。")
    );
    assert!(after.last_action_at.is_some());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert!(after.cooldown_until.is_some());
    assert_eq!(after.consecutive_route_failures, 1);

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.http_status),
        Some(401)
    );
    assert_eq!(
        detail.usage_window_refresh.error_code.as_deref(),
        Some("upstream_http_401")
    );
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
    assert_eq!(token_requests.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_usage_failure_preserves_quota_marker_without_token_refresh() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![
                (
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                ),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "error": {
                            "message": "gateway temporarily unavailable"
                        }
                    }),
                ),
            ],
            json!({
                "access_token": "refreshed-quota-preserving-token",
                "refresh_token": "refresh-token-rotated",
                "id_token": test_id_token(
                    "retry-quota-preserve@example.com",
                    Some("org_retry_quota_preserve"),
                    Some("user_retry_quota_preserve"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Gateway Quota Preserve OAuth",
        "retry-quota-preserve@example.com",
        "org_retry_quota_preserve",
        "user_retry_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after gateway failure")
        .expect("oauth row exists after gateway failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert_eq!(after.last_action_http_status, Some(401));
    assert_eq!(
        after.last_error.as_deref(),
        Some("上游返回 HTTP 401，等待下一分钟重试。")
    );
    assert!(after.last_action_at.is_some());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert!(after.last_route_failure_at.is_some());

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.usage_window_refresh.error_code.as_deref(),
        Some("upstream_http_401")
    );
    assert_eq!(
        detail.credential_refresh.state,
        UPSTREAM_ACCOUNT_REFRESH_STATE_RETRYING
    );
    assert_eq!(
        detail.credential_refresh.error_code.as_deref(),
        Some("access_token_rejected")
    );
    assert!(detail.credential_refresh.next_attempt_at.is_some());
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
    assert_eq!(token_requests.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn oauth_sync_usage_failure_preserves_quota_marker_without_refresh_attempt() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![(
                StatusCode::UNAUTHORIZED,
                json!({
                    "error": {
                        "message": "Session cookie expired during usage snapshot"
                    }
                }),
            )],
            json!({
                "unexpected": "shape"
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Refresh Failure Quota Preserve OAuth",
        "refresh-quota-preserve@example.com",
        "org_refresh_quota_preserve",
        "user_refresh_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after refresh failure")
        .expect("oauth row exists after refresh failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert_eq!(after.last_action_http_status, Some(401));
    assert_eq!(
        after.last_error.as_deref(),
        Some("上游返回 HTTP 401，等待下一分钟重试。")
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert!(after.last_route_failure_at.is_some());

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.usage_window_refresh.error_code.as_deref(),
        Some("upstream_http_401")
    );
    assert_eq!(
        detail.credential_refresh.state,
        UPSTREAM_ACCOUNT_REFRESH_STATE_RETRYING
    );
    assert_eq!(
        detail.credential_refresh.error_code.as_deref(),
        Some("access_token_rejected")
    );
    assert!(detail.credential_refresh.next_attempt_at.is_some());

    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
    assert_eq!(token_requests.load(Ordering::SeqCst), 0);
    server.abort();
}
