use super::*;

async fn wait_for_imported_oauth_validation_job_terminal(
    job: &Arc<ImportedOauthValidationJob>,
) -> ImportedOauthValidationTerminalEvent {
    timeout(Duration::from_secs(15), async {
        loop {
            if let Some(terminal) = job.terminal_event.lock().await.clone() {
                return terminal;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("validation job should finish within timeout")
}

#[test]
fn normalize_imported_oauth_credentials_accepts_codex_export_json() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-1".to_string(),
        file_name: "2q5q6m3ow4a@duckmail.sbs.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "2q5q6m3ow4a@duckmail.sbs",
            "account_id": "acct_imported",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "2q5q6m3ow4a@duckmail.sbs",
                Some("acct_imported"),
                Some("user_imported"),
                Some("team"),
            ),
            "last_refresh": "2026-03-18T00:00:00Z"
        })
        .to_string(),
    };

    let normalized =
        normalize_imported_oauth_credentials(&item).expect("normalize imported oauth credentials");
    assert_eq!(normalized.source_id, "file-1");
    assert_eq!(normalized.file_name, "2q5q6m3ow4a@duckmail.sbs.json");
    assert_eq!(normalized.email, "2q5q6m3ow4a@duckmail.sbs");
    assert_eq!(normalized.chatgpt_account_id, "acct_imported");
    assert_eq!(normalized.display_name, "2q5q6m3ow4a@duckmail.sbs");
    assert_eq!(
        normalized.claims.chatgpt_user_id.as_deref(),
        Some("user_imported")
    );
}

#[test]
fn normalize_imported_oauth_credentials_accepts_non_codex_or_missing_type() {
    for (name, source_type) in [
        ("auth0", Some(json!("auth0"))),
        ("blank", Some(json!("  "))),
        ("missing", None),
    ] {
        let mut content = json!({
            "email": format!("{name}@duckmail.sbs"),
            "account_id": format!("acct_{name}"),
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                &format!("{name}@duckmail.sbs"),
                Some(&format!("acct_{name}")),
                Some(&format!("user_{name}")),
                Some("team"),
            ),
        });
        if let Some(source_type) = source_type {
            content["type"] = source_type;
        }
        let item = ImportOauthCredentialFileRequest {
            source_id: format!("file-{name}"),
            file_name: format!("{name}.json"),
            content: content.to_string(),
        };

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials with any type");

        assert_eq!(normalized.email, format!("{name}@duckmail.sbs"));
        assert_eq!(normalized.chatgpt_account_id, format!("acct_{name}"));
    }
}

#[test]
fn normalize_imported_oauth_credentials_accepts_sub2api_oauth_account_objects() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "sub2api-oauth".to_string(),
        file_name: "sub2api-oauth.json".to_string(),
        content: json!({
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "email": "student@example.com",
                "chatgpt_account_id": "acct_shared_k12",
                "chatgpt_user_id": "user_student",
                "plan_type": "k12",
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "id_token": test_id_token(
                    "student@example.com",
                    Some("acct_shared_k12"),
                    Some("user_student"),
                    Some("k12"),
                ),
                "expires_at": "2026-03-20T00:00:00Z"
            }
        })
        .to_string(),
    };

    let normalized = normalize_imported_oauth_credentials(&item)
        .expect("normalize imported sub2api oauth account");

    assert_eq!(normalized.email, "student@example.com");
    assert_eq!(normalized.chatgpt_account_id, "acct_shared_k12");
    assert_eq!(normalized.chatgpt_user_id.as_deref(), Some("user_student"));
    assert_eq!(normalized.claims.chatgpt_plan_type.as_deref(), Some("k12"));
}

#[test]
fn imported_match_key_prefers_chatgpt_user_id_before_email_or_account_id() {
    assert_eq!(
        imported_match_key(Some("user_member"), "member@example.com", "acct_shared"),
        "user:user_member"
    );
    assert_eq!(
        imported_match_key(None, "member@example.com", "acct_shared"),
        "account:acct_shared:email:member@example.com"
    );
    assert_eq!(
        imported_match_key(None, "", "acct_shared"),
        "account:acct_shared:email:"
    );
}

#[test]
fn normalize_imported_oauth_credentials_accepts_missing_or_blank_refresh_token() {
    for (name, refresh_token) in [
        ("missing", None),
        ("null", Some(serde_json::Value::Null)),
        ("blank", Some(json!("  "))),
    ] {
        let mut content = json!({
            "type": "codex",
            "email": format!("{name}@duckmail.sbs"),
            "account_id": format!("acct_{name}"),
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "id_token": test_id_token(
                &format!("{name}@duckmail.sbs"),
                Some(&format!("acct_{name}")),
                Some(&format!("user_{name}")),
                Some("team"),
            ),
        });
        if let Some(refresh_token) = refresh_token {
            content["refresh_token"] = refresh_token;
        }
        let item = ImportOauthCredentialFileRequest {
            source_id: format!("file-{name}"),
            file_name: format!("{name}.json"),
            content: content.to_string(),
        };

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials without refresh token");

        assert_eq!(normalized.credentials.refresh_token, None);
        assert!(!oauth_credentials_have_refresh_token(
            &normalized.credentials
        ));
    }
}

#[tokio::test]
async fn probe_imported_oauth_credentials_skips_refresh_without_refresh_token() {
    #[derive(Clone)]
    struct ProbeServerState {
        usage_requests: Arc<AtomicUsize>,
        token_requests: Arc<AtomicUsize>,
    }

    async fn usage_handler(State(state): State<ProbeServerState>) -> (StatusCode, String) {
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

    async fn token_handler(State(state): State<ProbeServerState>) -> (StatusCode, String) {
        state.token_requests.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "refresh endpoint should not be called" }).to_string(),
        )
    }

    let usage_requests = Arc::new(AtomicUsize::new(0));
    let token_requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(usage_handler))
        .route("/oauth/token", post(token_handler))
        .with_state(ProbeServerState {
            usage_requests: usage_requests.clone(),
            token_requests: token_requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind imported probe server");
    let addr = listener.local_addr().expect("imported probe server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve imported probe server");
    });
    let origin = format!("http://{addr}");
    let state =
        test_app_state_with_usage_and_oauth_base(&format!("{origin}/backend-api"), &origin).await;
    let normalized = normalize_imported_oauth_credentials(&ImportOauthCredentialFileRequest {
        source_id: "source-no-rt".to_string(),
        file_name: "no-rt.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "no-rt@duckmail.sbs",
            "account_id": "acct_no_rt",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-no-rt",
            "id_token": test_id_token(
                "no-rt@duckmail.sbs",
                Some("acct_no_rt"),
                Some("user_no_rt"),
                Some("team"),
            ),
        })
        .to_string(),
    })
    .expect("normalize no refresh token import");
    let scope = ForwardProxyRouteScope::Automatic;

    let outcome = probe_imported_oauth_credentials(&state, &normalized, &scope, &scope)
        .await
        .expect("probe no refresh token import");

    assert!(!oauth_credentials_have_refresh_token(&outcome.credentials));
    assert_eq!(token_requests.load(Ordering::SeqCst), 0);
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);

    server.abort();
}

#[test]
fn normalize_imported_oauth_credentials_ignores_non_string_unused_fields() {
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
fn normalize_imported_oauth_credentials_uses_access_token_exp_when_expired_blank() {
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
fn normalize_imported_oauth_credentials_uses_id_token_exp_when_expired_missing() {
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
fn normalize_imported_oauth_credentials_rejects_non_empty_invalid_expired() {
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
fn normalize_imported_oauth_credentials_rejects_missing_expired_without_token_exp() {
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
fn normalize_imported_oauth_credentials_rejects_id_token_mismatch() {
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
async fn imported_oauth_validation_job_caches_successful_probe_for_import_reuse() {
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
        refresh_token_expires_at: None,
        refresh_token_expiry_source: "not_provided".to_string(),
        refresh_token_metadata_update: true,
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
        refresh_token_expires_at: None,
        refresh_token_expiry_source: "not_provided".to_string(),
        refresh_token_metadata_update: true,
        credentials: normalized.credentials.clone(),
        claims: normalized.claims.clone(),
        usage_snapshot: None,
        maintenance_proxy_snapshot: None,
        exhausted: false,
        usage_snapshot_warning: Some("usage snapshot unavailable during validation".to_string()),
    };
    let validated_item = ImportOauthCredentialFileRequest {
        source_id: "source-1".to_string(),
        file_name: "alpha.json".to_string(),
        content: "validated-payload".to_string(),
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
        Some(ImportedOauthValidatedImportData {
            input_digest: imported_oauth_validation_input_digest(&validated_item),
            normalized,
            probe,
        }),
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

    let mut cache = HashMap::new();
    cache.insert("source-1".to_string(), cached.clone());
    assert!(cached_imported_oauth_validation_result(&cache, &validated_item).is_some());
    let changed_item = ImportOauthCredentialFileRequest {
        source_id: "source-1".to_string(),
        file_name: "alpha.json".to_string(),
        content: "changed-payload".to_string(),
    };
    assert!(cached_imported_oauth_validation_result(&cache, &changed_item).is_none());
}

#[tokio::test]
async fn imported_oauth_validation_job_only_consumes_node_shunt_slots_after_success() {
    #[derive(Clone)]
    struct ImportedOauthValidationServerState {
        usage_requests: Arc<AtomicUsize>,
        token_requests: Arc<AtomicUsize>,
    }

    async fn usage_handler(
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

    async fn token_handler(
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

    fn imported_item(
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

    let usage_requests = Arc::new(AtomicUsize::new(0));
    let token_requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(usage_handler))
        .route("/oauth/token", post(token_handler))
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
                imported_item(
                    "source-bad",
                    "bad.json",
                    "bad@duckmail.sbs",
                    "acct_bad",
                    "2026-03-20T00:00:00Z",
                    "bad-refresh",
                ),
                imported_item(
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

#[tokio::test]
async fn imported_oauth_validation_job_probes_node_shunt_group_when_no_slot_is_available() {
    async fn usage_handler() -> (StatusCode, String) {
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

    let app = Router::new().route("/backend-api/wham/usage", get(usage_handler));
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
async fn import_validated_oauth_accounts_persists_node_shunt_group_when_no_slot_is_available() {
    async fn usage_handler() -> (StatusCode, String) {
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

    let app = Router::new().route("/backend-api/wham/usage", get(usage_handler));
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

    let imported = ImportOauthCredentialFileRequest {
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
    };
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

#[tokio::test]
async fn imported_oauth_validation_job_keeps_node_shunt_group_blocked_without_selectable_nodes() {
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
