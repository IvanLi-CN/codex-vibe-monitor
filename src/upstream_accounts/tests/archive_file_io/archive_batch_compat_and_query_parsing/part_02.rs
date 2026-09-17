use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde_json::json;
#[test]
pub(crate) fn normalize_imported_oauth_credentials_ignores_non_string_unused_fields() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-non-string-unused".to_string(),
        file_name: "non-string-unused.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "non-string-unused@duckmail.sbs",
            "account_id": "acct_non_string_unused",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "non-string-unused@duckmail.sbs",
                Some("acct_non_string_unused"),
                Some("user_non_string_unused"),
                Some("team"),
            ),
            "last_refresh": {
                "at": "2026-03-18T00:00:00Z"
            },
            "token_type": 42
        })
        .to_string(),
    };

    let normalized =
        normalize_imported_oauth_credentials(&item).expect("normalize imported oauth credentials");
    assert_eq!(normalized.credentials.token_type.as_deref(), Some("Bearer"));
    assert_eq!(normalized.chatgpt_account_id, "acct_non_string_unused");
}

#[test]
pub(crate) fn normalize_imported_oauth_credentials_uses_access_token_exp_when_expired_blank() {
    let access_exp = 1_777_777_777;
    let id_exp = 1_666_666_666;
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-blank-expired".to_string(),
        file_name: "blank-expired.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "blank-expired@duckmail.sbs",
            "account_id": "acct_blank_expired",
            "expired": "",
            "access_token": test_jwt_token(json!({ "exp": access_exp })),
            "refresh_token": "refresh-token",
            "id_token": test_jwt_token(json!({
                "exp": id_exp,
                "email": "blank-expired@duckmail.sbs",
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acct_blank_expired",
                    "chatgpt_user_id": "user_blank_expired",
                    "chatgpt_plan_type": "team"
                }
            }))
        })
        .to_string(),
    };

    let normalized =
        normalize_imported_oauth_credentials(&item).expect("normalize imported oauth credentials");
    assert_eq!(normalized.token_expires_at, "2026-05-03T03:09:37Z");
}

#[test]
pub(crate) fn normalize_imported_oauth_credentials_uses_id_token_exp_when_expired_missing() {
    let id_exp = 1_666_666_666;
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-missing-expired".to_string(),
        file_name: "missing-expired.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "missing-expired@duckmail.sbs",
            "account_id": "acct_missing_expired",
            "access_token": "opaque-access-token",
            "refresh_token": "refresh-token",
            "id_token": test_jwt_token(json!({
                "exp": id_exp,
                "email": "missing-expired@duckmail.sbs",
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acct_missing_expired",
                    "chatgpt_user_id": "user_missing_expired",
                    "chatgpt_plan_type": "team"
                }
            }))
        })
        .to_string(),
    };

    let normalized =
        normalize_imported_oauth_credentials(&item).expect("normalize imported oauth credentials");
    assert_eq!(normalized.token_expires_at, "2022-10-25T02:57:46Z");
}

#[test]
pub(crate) fn normalize_imported_oauth_credentials_rejects_non_empty_invalid_expired() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-invalid-expired".to_string(),
        file_name: "invalid-expired.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "invalid-expired@duckmail.sbs",
            "account_id": "acct_invalid_expired",
            "expired": "not-a-date",
            "access_token": test_jwt_token(json!({ "exp": 1_777_777_777 })),
            "refresh_token": "refresh-token",
            "id_token": test_jwt_token(json!({
                "exp": 1_666_666_666,
                "email": "invalid-expired@duckmail.sbs",
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acct_invalid_expired",
                    "chatgpt_user_id": "user_invalid_expired",
                    "chatgpt_plan_type": "team"
                }
            }))
        })
        .to_string(),
    };

    let error = normalize_imported_oauth_credentials(&item)
        .expect_err("expected invalid expired timestamp");
    assert_eq!(error, "expired must be a valid RFC3339 timestamp");
}

#[test]
pub(crate) fn normalize_imported_oauth_credentials_rejects_missing_expired_without_token_exp() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-missing-expired-no-exp".to_string(),
        file_name: "missing-expired-no-exp.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "missing-expired-no-exp@duckmail.sbs",
            "account_id": "acct_missing_expired_no_exp",
            "access_token": "opaque-access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "missing-expired-no-exp@duckmail.sbs",
                Some("acct_missing_expired_no_exp"),
                Some("user_missing_expired_no_exp"),
                Some("team"),
            )
        })
        .to_string(),
    };

    let error = normalize_imported_oauth_credentials(&item)
        .expect_err("expected missing expiry to be rejected");
    assert_eq!(error, "expired is required when token exp is unavailable");
}

#[test]
pub(crate) fn normalize_imported_oauth_credentials_rejects_id_token_mismatch() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-2".to_string(),
        file_name: "mismatch.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "mismatch@duckmail.sbs",
            "account_id": "acct_imported",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "different@duckmail.sbs",
                Some("acct_imported"),
                Some("user_imported"),
                Some("team"),
            )
        })
        .to_string(),
    };

    let error =
        normalize_imported_oauth_credentials(&item).expect_err("expected imported oauth mismatch");
    assert_eq!(error, "email does not match id_token");
}

#[tokio::test]
pub(crate) async fn imported_oauth_validation_job_caches_successful_probe_for_import_reuse() {
    let binding = ResolvedRequiredGroupProxyBinding {
        group_name: "import-group".to_string(),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        node_shunt_enabled: false,
    };
    let job = Arc::new(ImportedOauthValidationJob::new(
        ImportedOauthValidationResponse {
            input_files: 1,
            unique_in_input: 1,
            duplicate_in_input: 0,
            rows: vec![ImportedOauthValidationRow {
                source_id: "source-1".to_string(),
                file_name: "alpha.json".to_string(),
                email: None,
                chatgpt_account_id: None,
                chatgpt_user_id: None,
                display_name: None,
                token_expires_at: None,
                matched_account: None,
                status: "pending".to_string(),
                detail: None,
                attempts: 0,
            }],
        },
        &binding,
    ));
    let normalized = NormalizedImportedOauthCredentials {
        source_id: "source-1".to_string(),
        file_name: "alpha.json".to_string(),
        email: "alpha@duckmail.sbs".to_string(),
        display_name: "alpha@duckmail.sbs".to_string(),
        chatgpt_account_id: "acct_alpha".to_string(),
        chatgpt_user_id: Some("user_alpha".to_string()),
        token_expires_at: "2026-03-20T00:00:00Z".to_string(),
        credentials: StoredOauthCredentials {
            access_token: "access-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
            id_token: test_id_token(
                "alpha@duckmail.sbs",
                Some("acct_alpha"),
                Some("user_alpha"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        },
        claims: test_claims("alpha@duckmail.sbs", Some("acct_alpha"), Some("user_alpha")),
    };
    let probe = ImportedOauthProbeOutcome {
        token_expires_at: "2026-03-20T00:00:00Z".to_string(),
        credentials: normalized.credentials.clone(),
        claims: normalized.claims.clone(),
        usage_snapshot: None,
        maintenance_proxy_snapshot: None,
        exhausted: false,
        usage_snapshot_warning: Some("usage snapshot unavailable during validation".to_string()),
    };

    update_imported_oauth_validation_job_row(
        &job,
        0,
        ImportedOauthValidationRow {
            source_id: "source-1".to_string(),
            file_name: "alpha.json".to_string(),
            email: Some("alpha@duckmail.sbs".to_string()),
            chatgpt_account_id: Some("acct_alpha".to_string()),
            chatgpt_user_id: Some("user_alpha".to_string()),
            display_name: Some("alpha@duckmail.sbs".to_string()),
            token_expires_at: Some("2026-03-20T00:00:00Z".to_string()),
            matched_account: None,
            status: IMPORT_VALIDATION_STATUS_OK.to_string(),
            detail: probe.usage_snapshot_warning.clone(),
            attempts: 1,
        },
        Some(ImportedOauthValidatedImportData { normalized, probe }),
    )
    .await;

    let cached = job
        .validated_imports
        .lock()
        .await
        .get("source-1")
        .cloned()
        .expect("cached validated import");
    assert_eq!(cached.normalized.email, "alpha@duckmail.sbs");
    assert_eq!(cached.normalized.chatgpt_account_id, "acct_alpha");
    assert_eq!(
        cached.probe.credentials.refresh_token.as_deref(),
        Some("refresh-token")
    );
}

#[tokio::test]
pub(crate) async fn imported_oauth_validation_job_only_consumes_node_shunt_slots_after_success() {
    let usage_requests = Arc::new(AtomicUsize::new(0));
    let token_requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(imported_oauth_usage_handler))
        .route("/oauth/token", post(imported_oauth_token_handler))
        .with_state(ImportedOauthValidationServerState {
            usage_requests: usage_requests.clone(),
            token_requests: token_requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind imported validation server");
    let addr = listener
        .local_addr()
        .expect("imported validation server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve imported validation server");
    });
    let origin = format!("http://{addr}");

    let state =
        test_app_state_with_usage_and_oauth_base(&format!("{origin}/backend-api"), &origin).await;
    let Json(response) = create_imported_oauth_validation_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(ValidateImportedOauthAccountsRequest {
            group_name: Some("import-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            items: vec![
                imported_oauth_item(
                    "source-bad",
                    "bad.json",
                    "bad@duckmail.sbs",
                    "acct_bad",
                    "2026-03-20T00:00:00Z",
                    "bad-refresh",
                ),
                imported_oauth_item(
                    "source-good",
                    "good.json",
                    "good@duckmail.sbs",
                    "acct_good",
                    "2099-04-20T00:00:00Z",
                    "good-refresh",
                ),
            ],
        }),
    )
    .await
    .expect("start imported oauth validation job");
    let job = state
        .upstream_accounts
        .get_validation_job(&response.job_id)
        .await
        .expect("validation job should exist");
    let _terminal = wait_for_imported_oauth_validation_job_terminal(&job).await;

    let rows = job.snapshot.lock().await.rows.clone();
    let bad_row = rows
        .iter()
        .find(|row| row.source_id == "source-bad")
        .expect("bad row");
    let good_row = rows
        .iter()
        .find(|row| row.source_id == "source-good")
        .expect("good row");

    assert_eq!(bad_row.status, IMPORT_VALIDATION_STATUS_INVALID);
    assert!(
        bad_row
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("refresh token rejected")
    );
    assert_eq!(good_row.status, IMPORT_VALIDATION_STATUS_OK);
    assert_ne!(
        good_row.detail.as_deref(),
        Some(group_node_shunt_unassigned_error_message())
    );
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);

    server.abort();
}

#[derive(Clone)]
struct ImportedOauthValidationServerState {
    usage_requests: Arc<AtomicUsize>,
    token_requests: Arc<AtomicUsize>,
}

async fn imported_oauth_usage_handler(
    State(state): State<ImportedOauthValidationServerState>,
) -> (StatusCode, String) {
    state.usage_requests.fetch_add(1, Ordering::SeqCst);
    (
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 8,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        })
        .to_string(),
    )
}

async fn imported_oauth_token_handler(
    State(state): State<ImportedOauthValidationServerState>,
    axum::extract::Form(form): axum::extract::Form<std::collections::HashMap<String, String>>,
) -> (StatusCode, String) {
    state.token_requests.fetch_add(1, Ordering::SeqCst);
    let refresh_token = form.get("refresh_token").cloned().unwrap_or_default();
    if refresh_token == "bad-refresh" {
        return (
            StatusCode::BAD_REQUEST,
            json!({
                "error": "invalid_grant",
                "error_description": "refresh token rejected"
            })
            .to_string(),
        );
    }

    (
        StatusCode::OK,
        json!({
            "access_token": "refreshed-access",
            "refresh_token": "refreshed-refresh",
            "id_token": test_id_token(
                "fallback@duckmail.sbs",
                Some("acct_fallback"),
                Some("user_fallback"),
                Some("team"),
            ),
            "token_type": "Bearer",
            "expires_in": 3600,
        })
        .to_string(),
    )
}

fn imported_oauth_item(
    source_id: &str,
    file_name: &str,
    email: &str,
    account_id: &str,
    expires_at: &str,
    refresh_token: &str,
) -> ImportOauthCredentialFileRequest {
    ImportOauthCredentialFileRequest {
        source_id: source_id.to_string(),
        file_name: file_name.to_string(),
        content: json!({
            "type": "codex",
            "email": email,
            "account_id": account_id,
            "expired": expires_at,
            "access_token": format!("access-{source_id}"),
            "refresh_token": refresh_token,
            "id_token": test_id_token(
                email,
                Some(account_id),
                Some(format!("user_{source_id}").as_str()),
                Some("team"),
            ),
        })
        .to_string(),
    }
}

#[tokio::test]
pub(crate) async fn imported_oauth_validation_job_probes_node_shunt_group_when_no_slot_is_available()
 {
    let (server, origin) = start_import_usage_server().await;

    let state = test_app_state_with_usage_base(&format!("{origin}/backend-api")).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Account",
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    set_test_account_group_name(&state.pool, occupying_account_id, Some("import-group")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "import-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt metadata");
    drop(conn);

    let Json(response) = create_imported_oauth_validation_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(ValidateImportedOauthAccountsRequest {
            group_name: Some("import-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            items: vec![ImportOauthCredentialFileRequest {
                source_id: "source-new".to_string(),
                file_name: "new-session.json".to_string(),
                content: json!({
                    "type": "codex",
                    "email": "new-session@example.com",
                    "account_id": "acct_new_session",
                    "expired": "2099-04-20T00:00:00Z",
                    "access_token": "access-new-session",
                    "id_token": test_id_token(
                        "new-session@example.com",
                        Some("acct_new_session"),
                        Some("user_new_session"),
                        Some("team"),
                    ),
                })
                .to_string(),
            }],
        }),
    )
    .await
    .expect("start imported oauth validation job");
    let job = state
        .upstream_accounts
        .get_validation_job(&response.job_id)
        .await
        .expect("validation job should exist");
    let _terminal = wait_for_imported_oauth_validation_job_terminal(&job).await;

    let rows = job.snapshot.lock().await.rows.clone();
    let row = rows.first().expect("validation row");
    assert_eq!(row.status, IMPORT_VALIDATION_STATUS_OK);
    assert_ne!(
        row.detail.as_deref(),
        Some(group_node_shunt_unassigned_error_message())
    );

    server.abort();
}

#[tokio::test]
pub(crate) async fn import_validated_oauth_accounts_persists_node_shunt_group_when_no_slot_is_available()
 {
    let (server, origin) = start_import_usage_server().await;

    let state = test_app_state_with_usage_base(&format!("{origin}/backend-api")).await;
    occupy_import_group_node_shunt_slot(&state).await;

    let imported = new_imported_oauth_item();
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::HOST,
        axum::http::HeaderValue::from_static("127.0.0.1:8080"),
    );

    let Json(response) = import_validated_oauth_accounts(
        State(state.clone()),
        headers,
        Json(ImportValidatedOauthAccountsRequest {
            items: vec![imported],
            selected_source_ids: vec!["source-new".to_string()],
            validation_job_id: None,
            group_name: Some("import-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            group_note: None,
            concurrency_limit: None,
            tag_ids: vec![],
        }),
    )
    .await
    .expect("import should persist even when node shunt slots are full");

    assert_eq!(response.summary.created, 1);
    assert_eq!(response.summary.failed, 0);
    let account_id = response
        .results
        .first()
        .and_then(|result| result.account_id)
        .expect("created account id");
    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert!(
        !assignments.account_proxy_keys.contains_key(&account_id),
        "new account should persist without stealing an occupied node shunt slot",
    );
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load imported account")
        .expect("imported account");
    let metadata = load_group_metadata(&state.pool, Some("import-group"))
        .await
        .expect("load group metadata");
    let err = resolve_account_forward_proxy_scope_from_assignments(
        row.id,
        row.group_name.as_deref(),
        &metadata,
        &assignments,
    )
    .expect_err("persisted account should remain unroutable until a node shunt slot opens");
    assert!(is_group_node_shunt_unassigned_message(&err.to_string()));

    server.abort();
}

async fn import_usage_handler() -> (StatusCode, String) {
    (
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 12,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        })
        .to_string(),
    )
}

async fn start_import_usage_server() -> (tokio::task::JoinHandle<()>, String) {
    let app = Router::new().route("/backend-api/wham/usage", get(import_usage_handler));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind imported validation server");
    let addr = listener
        .local_addr()
        .expect("imported validation server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve imported validation server");
    });
    (server, format!("http://{addr}"))
}

async fn occupy_import_group_node_shunt_slot(state: &Arc<AppState>) {
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Account",
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    set_test_account_group_name(&state.pool, occupying_account_id, Some("import-group")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "import-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt metadata");
}

fn new_imported_oauth_item() -> ImportOauthCredentialFileRequest {
    ImportOauthCredentialFileRequest {
        source_id: "source-new".to_string(),
        file_name: "new-session.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "new-session@example.com",
            "account_id": "acct_new_session",
            "expired": "2099-04-20T00:00:00Z",
            "access_token": "access-new-session",
            "id_token": test_id_token(
                "new-session@example.com",
                Some("acct_new_session"),
                Some("user_new_session"),
                Some("team"),
            ),
        })
        .to_string(),
    }
}

#[tokio::test]
pub(crate) async fn imported_oauth_validation_job_keeps_node_shunt_group_blocked_without_selectable_nodes()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let Json(response) = create_imported_oauth_validation_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(ValidateImportedOauthAccountsRequest {
            group_name: Some("stale-import-group".to_string()),
            group_bound_proxy_keys: Some(vec!["stale-node".to_string()]),
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            items: vec![ImportOauthCredentialFileRequest {
                source_id: "source-stale".to_string(),
                file_name: "stale-session.json".to_string(),
                content: json!({
                    "type": "codex",
                    "email": "stale-session@example.com",
                    "account_id": "acct_stale_session",
                    "expired": "2099-04-20T00:00:00Z",
                    "access_token": "access-stale-session",
                    "id_token": test_id_token(
                        "stale-session@example.com",
                        Some("acct_stale_session"),
                        Some("user_stale_session"),
                        Some("team"),
                    ),
                })
                .to_string(),
            }],
        }),
    )
    .await
    .expect("start imported oauth validation job");
    let job = state
        .upstream_accounts
        .get_validation_job(&response.job_id)
        .await
        .expect("validation job should exist");
    let _terminal = wait_for_imported_oauth_validation_job_terminal(&job).await;

    let rows = job.snapshot.lock().await.rows.clone();
    let row = rows.first().expect("validation row");
    assert_eq!(row.status, IMPORT_VALIDATION_STATUS_ERROR);
    assert_eq!(
        row.detail.as_deref(),
        Some(group_node_shunt_unassigned_error_message())
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_allows_note_only_edits_when_node_shunt_group_has_no_selectable_nodes()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Node Shunt Guard").await;
    let group_name = "empty-node-shunt";
    let stale_proxy_key = "stale-node".to_string();
    set_test_account_group_name(&state.pool, account_id, Some(group_name)).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        group_name,
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![stale_proxy_key.clone()],
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    let Json(response) = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        AxumPath(group_name.to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: Some("still editable".to_string()),
            bound_proxy_keys: None,
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: None,
        }),
    )
    .await
    .expect("note-only edit should succeed even without selectable nodes");

    assert_eq!(response.group_name, group_name);
    assert_eq!(response.note.as_deref(), Some("still editable"));
    assert_eq!(response.bound_proxy_keys, vec![stale_proxy_key]);
    assert!(response.node_shunt_enabled);
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_rejects_clearing_bindings_while_node_shunt_stays_enabled()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Node Shunt Bound").await;
    set_test_account_group_name(&state.pool, account_id, Some("bound-node-shunt")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "bound-node-shunt",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        AxumPath("bound-node-shunt".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: Some(vec![]),
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: None,
        }),
    )
    .await
    .expect_err("node shunt group should reject clearing bindings");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        missing_group_bound_proxy_error_message("bound-node-shunt")
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_rejects_disabling_node_shunt_with_unselectable_bindings()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Node Shunt Disable").await;
    set_test_account_group_name(&state.pool, account_id, Some("disable-node-shunt")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "disable-node-shunt",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec!["stale-node".to_string()],
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        AxumPath("disable-node-shunt".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: None,
            node_shunt_enabled: Some(false),
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: None,
        }),
    )
    .await
    .expect_err("disabling node shunt should revalidate unselectable bindings");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "select at least one available proxy node or clear bindings before saving"
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_rejects_invalid_routing_policy_enums() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        AxumPath("invalid-policy".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: None,
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                allow_cut_out: OptionalField::Missing,
                allow_cut_in: OptionalField::Missing,
                priority_tier: OptionalField::Value("urgent".to_string()),
                fast_mode_rewrite_mode: OptionalField::Value("keep_original".to_string()),
                image_tool_rewrite_mode: OptionalField::Missing,
                codex_imagegen_rewrite_mode: OptionalField::Missing,
                request_compression_algorithm: OptionalField::Missing,
                concurrency_limit: OptionalField::Missing,
                upstream_429_retry_enabled: OptionalField::Missing,
                upstream_429_max_retries: OptionalField::Missing,
                available_models: OptionalField::Missing,
                available_models_mode: OptionalField::Missing,
                status_change_reasons: None,
                timeouts: None,
            }),
        }),
    )
    .await
    .expect_err("invalid routing policy enum should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "priorityTier must be one of: primary, normal, fallback, no_new"
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_clears_available_models_when_policy_submits_inherit()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "clear-model-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    sqlx::query(
        r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_available_models_json = '["gpt-5.5"]'
            WHERE group_name = 'clear-model-group'
            "#,
    )
    .execute(&state.pool)
    .await
    .expect("seed group available models");

    let _ = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath("clear-model-group".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: None,
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                allow_cut_out: OptionalField::Missing,
                allow_cut_in: OptionalField::Missing,
                priority_tier: OptionalField::Missing,
                fast_mode_rewrite_mode: OptionalField::Missing,
                image_tool_rewrite_mode: OptionalField::Missing,
                codex_imagegen_rewrite_mode: OptionalField::Missing,
                request_compression_algorithm: OptionalField::Missing,
                concurrency_limit: OptionalField::Missing,
                upstream_429_retry_enabled: OptionalField::Missing,
                upstream_429_max_retries: OptionalField::Missing,
                available_models: OptionalField::Null,
                available_models_mode: OptionalField::Missing,
                status_change_reasons: None,
                timeouts: None,
            }),
        }),
    )
    .await
    .expect("clear group available models");

    let stored = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT policy_available_models_json
            FROM pool_upstream_account_group_notes
            WHERE group_name = 'clear-model-group'
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load cleared group policy");
    assert_eq!(stored, None);
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_preserves_available_models_when_mode_changes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "preserve-model-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    sqlx::query(
        r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_available_models_json = '["gpt-5.5"]'
            WHERE group_name = 'preserve-model-group'
            "#,
    )
    .execute(&state.pool)
    .await
    .expect("seed group available models");

    let _ = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath("preserve-model-group".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: None,
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                allow_cut_out: OptionalField::Missing,
                allow_cut_in: OptionalField::Missing,
                priority_tier: OptionalField::Value("primary".to_string()),
                fast_mode_rewrite_mode: OptionalField::Missing,
                image_tool_rewrite_mode: OptionalField::Missing,
                codex_imagegen_rewrite_mode: OptionalField::Missing,
                request_compression_algorithm: OptionalField::Missing,
                concurrency_limit: OptionalField::Missing,
                upstream_429_retry_enabled: OptionalField::Missing,
                upstream_429_max_retries: OptionalField::Missing,
                available_models: OptionalField::Missing,
                available_models_mode: OptionalField::Value("denylist".to_string()),
                status_change_reasons: None,
                timeouts: None,
            }),
        }),
    )
    .await
    .expect("preserve omitted group available models");

    let stored = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT policy_available_models_json
            FROM pool_upstream_account_group_notes
            WHERE group_name = 'preserve-model-group'
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load preserved group policy");
    assert_eq!(stored.as_deref(), Some("[\"gpt-5.5\"]"));

    let stored_mode = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT policy_available_models_mode
            FROM pool_upstream_account_group_notes
            WHERE group_name = 'preserve-model-group'
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load preserved group policy mode");
    assert_eq!(stored_mode.as_deref(), Some("denylist"));
}

#[tokio::test]
pub(crate) async fn create_api_key_account_rejects_group_fields() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Rejected Group Account",
        "apiKey": "sk-rejected-group",
        "groupName": "legacy-group"
    }))
    .expect("deserialize api key create request");
    let err = create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect_err("API key group fields must be rejected");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pool_upstream_accounts WHERE display_name = 'Rejected Group Account'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("count rejected account"),
        0
    );
}

#[tokio::test]
pub(crate) async fn create_api_key_account_reports_conflict_when_post_create_sync_lacks_node_shunt_slot()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let occupying_account_id =
        insert_api_key_account(&state.pool, "Existing Node Shunt Occupant").await;
    set_test_account_group_name(
        &state.pool,
        occupying_account_id,
        Some("node-shunt-create-blocked"),
    )
    .await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-create-blocked",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt create metadata");
    drop(conn);

    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Blocked Node Shunt Create",
        "apiKey": "sk-blocked-node-shunt-create",
        "groupName": "node-shunt-create-blocked"
    }))
    .expect("deserialize blocked api key create request");
    let err = create_api_key_account_inner(state, payload)
        .await
        .expect_err("create api key account should fail without a node slot");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("cannot use groups"));
}

#[tokio::test]
pub(crate) async fn update_upstream_account_persists_node_shunt_for_existing_multi_account_group() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Shared Group Target").await;
    let sibling_account_id = insert_api_key_account(&state.pool, "Shared Group Sibling").await;
    for grouped_account_id in [account_id, sibling_account_id] {
        set_test_account_group_name(&state.pool, grouped_account_id, Some("shared-update-group"))
            .await;
    }

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "shared-update-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save shared update group metadata");
    drop(conn);

    let err = update_upstream_account(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(account_id),
        Json(UpdateUpstreamAccountRequest {
            display_name: None,
            email: OptionalField::Missing,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            upstream_base_url: OptionalField::Missing,
            bound_proxy_keys: OptionalField::Missing,
            enabled: None,
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
            routing_rule: None,
            ..UpdateUpstreamAccountRequest::default()
        }),
    )
    .await
    .expect_err("API key group policy updates must be rejected");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    let metadata = load_group_metadata(&state.pool, Some("shared-update-group"))
        .await
        .expect("load shared update group metadata");
    assert!(!metadata.node_shunt_enabled);
}

#[tokio::test]
pub(crate) async fn api_key_group_migration_is_idempotent_and_preserves_oauth_groups() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let api_key_id = insert_api_key_account(&state.pool, "Legacy Relay Group").await;
    set_test_account_group_name(&state.pool, api_key_id, Some("legacy-relay-group")).await;
    sqlx::query("UPDATE pool_upstream_accounts SET is_mother = 1 WHERE id = ?1")
        .bind(api_key_id)
        .execute(&state.pool)
        .await
        .expect("mark legacy API key mother");
    let oauth_id = insert_oauth_account(&state.pool, "Official OAuth Member").await;
    set_test_account_group_name(&state.pool, oauth_id, Some("legacy-relay-group")).await;

    let Json(preflight) = preflight_api_key_group_migration(State(state.clone()))
        .await
        .expect("preflight migration");
    assert_eq!(preflight.api_key_count, 1);
    assert!(preflight.blocked_strategies.is_empty());
    assert!(preflight.can_migrate);

    let Json(migrated) = confirm_api_key_group_migration(
        State(state.clone()),
        HeaderMap::new(),
        Json(ConfirmApiKeyGroupMigrationRequest {
            confirmation_hash: "stale".to_string(),
            disabled_strategies: Vec::new(),
        }),
    )
    .await
    .expect("migration automatically disables obsolete group strategies");
    assert_eq!(migrated.migrated_count, 1);
    let api_key_state = sqlx::query_as::<_, (Option<String>, i64)>(
        "SELECT group_name, is_mother FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(api_key_id)
    .fetch_one(&state.pool)
    .await
    .expect("load migrated API key");
    assert_eq!(api_key_state, (None, 0));
    let oauth_group = sqlx::query_scalar::<_, Option<String>>(
        "SELECT group_name FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(oauth_id)
    .fetch_one(&state.pool)
    .await
    .expect("load OAuth group");
    assert_eq!(oauth_group.as_deref(), Some("legacy-relay-group"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND action = 'api_key_transit_proxy_binding_migrated'",
        )
        .bind(api_key_id)
        .fetch_one(&state.pool)
        .await
        .expect("count migration audit"),
        1
    );
    let Json(post_migration_preflight) = preflight_api_key_group_migration(State(state.clone()))
        .await
        .expect("preflight after migration");
    assert_eq!(post_migration_preflight.api_key_count, 0);
}

#[tokio::test]
pub(crate) async fn resolve_required_group_proxy_binding_for_write_allows_node_shunt_without_selectable_nodes()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let requested_bound_proxy_keys = vec!["stale-node".to_string()];
    let expected_bound_proxy_keys =
        canonicalize_forward_proxy_bound_keys(state.as_ref(), &requested_bound_proxy_keys)
            .await
            .expect("canonicalize bound proxy keys");
    let binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        Some("write-node-shunt".to_string()),
        Some(requested_bound_proxy_keys),
        Some(true),
    )
    .await
    .expect("node shunt writes should not require selectable nodes");

    assert_eq!(binding.group_name, "write-node-shunt");
    assert_eq!(binding.bound_proxy_keys, expected_bound_proxy_keys);
    assert!(binding.node_shunt_enabled);
}

#[tokio::test]
pub(crate) async fn resolve_required_group_proxy_binding_for_write_rejects_empty_bindings_for_node_shunt()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let err = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        Some("write-node-shunt".to_string()),
        Some(vec![]),
        Some(true),
    )
    .await
    .expect_err("node shunt writes should reject empty bindings");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        missing_group_bound_proxy_error_message("write-node-shunt")
    );
}

#[tokio::test]
pub(crate) async fn build_imported_oauth_validation_response_returns_assignment_errors() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let binding = ResolvedRequiredGroupProxyBinding {
        group_name: "import-group".to_string(),
        bound_proxy_keys: vec!["stale-node".to_string()],
        node_shunt_enabled: true,
    };
    let items = vec![ImportOauthCredentialFileRequest {
        source_id: "source-1".to_string(),
        file_name: "alpha.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "alpha@duckmail.sbs",
            "account_id": "acct_alpha",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "alpha@duckmail.sbs",
                Some("acct_alpha"),
                Some("user_alpha"),
                Some("team"),
            ),
        })
        .to_string(),
    }];

    state.pool.close().await;

    let error = build_imported_oauth_validation_response(state.as_ref(), &items, &binding)
        .await
        .expect_err("assignment build failures should not be swallowed");
    assert!(
        error.to_string().contains("closed") || error.to_string().contains("pool"),
        "unexpected error: {error:#}"
    );
}

#[tokio::test]
pub(crate) async fn create_bulk_upstream_account_sync_job_reuses_existing_running_job() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let snapshot = BulkUpstreamAccountSyncSnapshot {
        job_id: "running-job".to_string(),
        status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
        rows: vec![BulkUpstreamAccountSyncRow {
            account_id: 5,
            display_name: "Existing OAuth".to_string(),
            status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_PENDING.to_string(),
            detail: None,
        }],
    };
    let counts = compute_bulk_upstream_account_sync_counts(&snapshot.rows);
    state
        .upstream_accounts
        .insert_bulk_sync_job(
            snapshot.job_id.clone(),
            Arc::new(BulkUpstreamAccountSyncJob::new(snapshot.clone())),
        )
        .await;

    let response = create_bulk_upstream_account_sync_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(BulkUpstreamAccountSyncJobRequest {
            account_ids: vec![9, 11],
        }),
    )
    .await
    .expect("reuse running bulk sync job")
    .0;

    assert_eq!(response.job_id, "running-job");
    assert_eq!(response.snapshot.job_id, "running-job");
    assert_eq!(response.snapshot.rows.len(), 1);
    assert_eq!(response.snapshot.rows[0].account_id, 5);
    assert_eq!(response.counts.total, counts.total);
    assert_eq!(response.counts.completed, counts.completed);
    assert_eq!(state.upstream_accounts.bulk_sync_jobs.lock().await.len(), 1);
}

#[tokio::test]
pub(crate) async fn finish_bulk_sync_job_completed_exposes_completed_status_in_events_and_response()
{
    let job = Arc::new(BulkUpstreamAccountSyncJob::new(
        BulkUpstreamAccountSyncSnapshot {
            job_id: "job-completed".to_string(),
            status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
            rows: vec![BulkUpstreamAccountSyncRow {
                account_id: 5,
                display_name: "Existing OAuth".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED.to_string(),
                detail: None,
            }],
        },
    ));
    let mut receiver = job.broadcaster.subscribe();

    finish_bulk_upstream_account_sync_job_completed(&job).await;

    match receiver.recv().await.expect("completed event") {
        BulkUpstreamAccountSyncJobEvent::Completed(payload) => {
            assert_eq!(
                payload.snapshot.status,
                BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED
            );
            assert_eq!(payload.counts.completed, 1);
            assert_eq!(payload.counts.failed, 0);
            assert_eq!(payload.counts.skipped, 0);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let response =
        build_bulk_upstream_account_sync_job_response("job-completed".to_string(), &job).await;
    assert_eq!(
        response.snapshot.status,
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED
    );
}

#[tokio::test]
pub(crate) async fn finish_bulk_sync_job_failed_exposes_failed_status_in_events_and_response() {
    let job = Arc::new(BulkUpstreamAccountSyncJob::new(
        BulkUpstreamAccountSyncSnapshot {
            job_id: "job-failed".to_string(),
            status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
            rows: vec![BulkUpstreamAccountSyncRow {
                account_id: 5,
                display_name: "Existing OAuth".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED.to_string(),
                detail: Some("upstream rejected".to_string()),
            }],
        },
    ));
    let mut receiver = job.broadcaster.subscribe();

    finish_bulk_upstream_account_sync_job_failed(&job, "job failed".to_string()).await;

    match receiver.recv().await.expect("failed event") {
        BulkUpstreamAccountSyncJobEvent::Failed(payload) => {
            assert_eq!(
                payload.snapshot.status,
                BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED
            );
            assert_eq!(payload.counts.failed, 1);
            assert_eq!(payload.error, "job failed");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let response =
        build_bulk_upstream_account_sync_job_response("job-failed".to_string(), &job).await;
    assert_eq!(
        response.snapshot.status,
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED
    );
}
