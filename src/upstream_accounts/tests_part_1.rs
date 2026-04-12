    #[allow(unused_imports)]
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::{any, get, post},
    };
    use sqlx::SqlitePool;
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::{Arc, atomic::AtomicUsize},
        time::Duration,
    };
    use tokio::{
        net::TcpListener,
        sync::{Mutex, Notify},
        time::timeout,
    };

    fn test_summary_with_statuses(
        work_status: &str,
        enable_status: &str,
        health_status: &str,
        sync_state: &str,
    ) -> UpstreamAccountSummary {
        UpstreamAccountSummary {
            id: 1,
            kind: UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX.to_string(),
            provider: UPSTREAM_ACCOUNT_PROVIDER_CODEX.to_string(),
            display_name: "Test account".to_string(),
            group_name: Some("alpha".to_string()),
            is_mother: false,
            status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
            display_status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
            enabled: enable_status == UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
            work_status: work_status.to_string(),
            enable_status: enable_status.to_string(),
            health_status: health_status.to_string(),
            sync_state: sync_state.to_string(),
            routing_block_reason_code: None,
            routing_block_reason_message: None,
            email: Some("tester@example.com".to_string()),
            chatgpt_account_id: Some("acct_test".to_string()),
            plan_type: Some("pro".to_string()),
            masked_api_key: None,
            last_synced_at: None,
            last_successful_sync_at: None,
            last_activity_at: None,
            active_conversation_count: 0,
            last_error: None,
            last_error_at: None,
            last_action: None,
            last_action_source: None,
            last_action_reason_code: None,
            last_action_reason_message: None,
            last_action_http_status: None,
            last_action_invoke_id: None,
            last_action_at: None,
            token_expires_at: None,
            primary_window: None,
            secondary_window: None,
            credits: None,
            local_limits: None,
            compact_support: CompactSupportState {
                status: "unknown".to_string(),
                observed_at: None,
                reason: None,
            },
            duplicate_info: None,
            tags: vec![],
            effective_routing_rule: EffectiveRoutingRule {
                guard_enabled: false,
                lookback_hours: None,
                max_conversations: None,
                allow_cut_out: true,
                allow_cut_in: true,
                priority_tier: TagPriorityTier::Normal,
                fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
                concurrency_limit: 0,
                source_tag_ids: vec![],
                source_tag_names: vec![],
                guard_rules: vec![],
            },
        }
    }

    #[test]
    fn derive_secret_key_is_stable() {
        let lhs = derive_secret_key("alpha");
        let rhs = derive_secret_key("alpha");
        assert_eq!(lhs, rhs);
    }

    #[test]
    fn credential_round_trip_works() {
        let key = derive_secret_key("top-secret");
        let encrypted = encrypt_credentials(
            &key,
            &StoredCredentials::ApiKey(StoredApiKeyCredentials {
                api_key: "sk-test-1234".to_string(),
            }),
        )
        .expect("encrypt credentials");
        let decrypted = decrypt_credentials(&key, &encrypted).expect("decrypt credentials");
        let StoredCredentials::ApiKey(value) = decrypted else {
            panic!("expected API key credentials")
        };
        assert_eq!(value.api_key, "sk-test-1234");
    }

    #[test]
    fn deserialize_optional_field_distinguishes_missing_null_and_value() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload {
            #[serde(default, deserialize_with = "deserialize_optional_field")]
            upstream_base_url: OptionalField<String>,
        }

        let missing: Payload = serde_json::from_value(json!({})).expect("deserialize missing");
        assert_eq!(missing.upstream_base_url, OptionalField::Missing);

        let null_value: Payload =
            serde_json::from_value(json!({ "upstreamBaseUrl": null })).expect("deserialize null");
        assert_eq!(null_value.upstream_base_url, OptionalField::Null);

        let string_value: Payload = serde_json::from_value(json!({
            "upstreamBaseUrl": "https://proxy.example.com/gateway"
        }))
        .expect("deserialize string");
        assert_eq!(
            string_value.upstream_base_url,
            OptionalField::Value("https://proxy.example.com/gateway".to_string())
        );
    }

    #[test]
    fn list_query_deserializes_repeated_status_filters() {
        let query = parse_list_upstream_accounts_query(
            &"/api/pool/upstream-accounts?workStatus=working&workStatus=rate_limited&workStatus=unavailable&enableStatus=enabled&healthStatus=normal&healthStatus=needs_reauth"
                .parse()
                .expect("parse uri"),
        )
        .expect("deserialize repeated filters");

        assert_eq!(
            query.work_status,
            vec![
                UPSTREAM_ACCOUNT_WORK_STATUS_WORKING.to_string(),
                UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED.to_string(),
                UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE.to_string(),
            ]
        );
        assert_eq!(
            query.enable_status,
            vec![UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED.to_string()]
        );
        assert_eq!(
            query.health_status,
            vec![
                UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string(),
                UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH.to_string(),
            ]
        );
    }

    #[test]
    fn list_query_keeps_single_status_filter_compatible() {
        let query = parse_list_upstream_accounts_query(
            &"/api/pool/upstream-accounts?workStatus=idle&enableStatus=disabled&healthStatus=normal"
                .parse()
                .expect("parse uri"),
        )
        .expect("deserialize single filters");

        assert_eq!(
            query.work_status,
            vec![UPSTREAM_ACCOUNT_WORK_STATUS_IDLE.to_string()]
        );
        assert_eq!(
            query.enable_status,
            vec![UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED.to_string()]
        );
        assert_eq!(
            query.health_status,
            vec![UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string()]
        );
    }

    #[test]
    fn explicit_split_filters_override_legacy_status_mapping() {
        let enable_filters = collect_normalized_upstream_account_filters(
            &[UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED.to_string()],
            Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED),
            normalize_upstream_account_enable_status_filter,
        );
        assert_eq!(enable_filters, vec![UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED]);

        let health_filters = collect_normalized_upstream_account_filters(
            &[UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string()],
            Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
            normalize_upstream_account_health_status_filter,
        );
        assert_eq!(health_filters, vec![UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL]);
    }

    #[test]
    fn matches_upstream_account_filters_uses_or_within_each_dimension() {
        let item = test_summary_with_statuses(
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
            UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
        );

        assert!(matches_upstream_account_filters(
            &item,
            &[
                UPSTREAM_ACCOUNT_WORK_STATUS_WORKING,
                UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
            ],
            &[UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED],
            &[
                UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
                UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
            ],
            Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
        ));

        assert!(!matches_upstream_account_filters(
            &item,
            &[UPSTREAM_ACCOUNT_WORK_STATUS_WORKING],
            &[UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED],
            &[UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL],
            Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
        ));
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

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials");
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

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials");
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

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials");
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

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials");
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

        let error = normalize_imported_oauth_credentials(&item)
            .expect_err("expected imported oauth mismatch");
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
            token_expires_at: "2026-03-20T00:00:00Z".to_string(),
            credentials: StoredOauthCredentials {
                access_token: "access-token".to_string(),
                refresh_token: "refresh-token".to_string(),
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
            exhausted: false,
            usage_snapshot_warning: Some(
                "usage snapshot unavailable during validation".to_string(),
            ),
        };

        update_imported_oauth_validation_job_row(
            &job,
            0,
            ImportedOauthValidationRow {
                source_id: "source-1".to_string(),
                file_name: "alpha.json".to_string(),
                email: Some("alpha@duckmail.sbs".to_string()),
                chatgpt_account_id: Some("acct_alpha".to_string()),
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
        assert_eq!(cached.probe.credentials.refresh_token, "refresh-token");
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
            axum::extract::Form(form): axum::extract::Form<
                std::collections::HashMap<String, String>,
            >,
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
            test_app_state_with_usage_and_oauth_base(&format!("{origin}/backend-api"), &origin)
                .await;
        let Json(response) = create_imported_oauth_validation_job(
            State(state.clone()),
            HeaderMap::new(),
            Json(ValidateImportedOauthAccountsRequest {
                group_name: Some("import-group".to_string()),
                group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
                group_node_shunt_enabled: Some(true),
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
                        "2026-04-20T00:00:00Z",
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
        timeout(Duration::from_secs(1), async {
            loop {
                if job.terminal_event.lock().await.is_some() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("validation job should finish");

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
    async fn update_upstream_account_group_allows_note_only_edits_when_node_shunt_group_has_no_selectable_nodes()
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
                upstream_429_retry_enabled: None,
                upstream_429_max_retries: None,
                concurrency_limit: None,
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
    async fn update_upstream_account_group_rejects_clearing_bindings_while_node_shunt_stays_enabled()
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
                upstream_429_retry_enabled: None,
                upstream_429_max_retries: None,
                concurrency_limit: None,
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
    async fn update_upstream_account_group_rejects_disabling_node_shunt_with_unselectable_bindings()
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
                upstream_429_retry_enabled: None,
                upstream_429_max_retries: None,
                concurrency_limit: None,
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
    async fn create_api_key_account_persists_node_shunt_for_existing_multi_account_group() {
        let (base_url, server) = spawn_usage_snapshot_server(
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
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let secondary_proxy_key = {
            let mut manager = state.forward_proxy.lock().await;
            let mut settings = ForwardProxySettings::default();
            settings.proxy_urls = vec!["http://127.0.0.1:18080".to_string()];
            manager.apply_settings(settings);
            manager
                .binding_nodes()
                .into_iter()
                .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
                .map(|node| node.key)
                .expect("secondary proxy binding key")
        };
        let existing_account_id =
            insert_api_key_account(&state.pool, "Existing Shared Group").await;
        set_test_account_group_name(&state.pool, existing_account_id, Some("shared-write-group"))
            .await;

        let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
        save_group_metadata_record_conn(
            &mut conn,
            "shared-write-group",
            UpstreamAccountGroupMetadata {
                note: None,
                bound_proxy_keys: vec![
                    FORWARD_PROXY_DIRECT_KEY.to_string(),
                    secondary_proxy_key.clone(),
                ],
                node_shunt_enabled: false,
                upstream_429_retry_enabled: false,
                upstream_429_max_retries: 0,
                concurrency_limit: 0,
            },
        )
        .await
        .expect("save shared group metadata");
        drop(conn);

        let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
            "displayName": "Created Shared Group Account",
            "apiKey": "sk-created-shared-group",
            "groupName": "shared-write-group",
            "groupBoundProxyKeys": [
                FORWARD_PROXY_DIRECT_KEY,
                secondary_proxy_key
            ],
            "groupNodeShuntEnabled": true
        }))
        .expect("deserialize api key create request");
        let Json(detail) =
            create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
                .await
                .expect("create api key account in existing group");

        assert_eq!(
            detail.summary.group_name.as_deref(),
            Some("shared-write-group")
        );
        let metadata = load_group_metadata(&state.pool, Some("shared-write-group"))
            .await
            .expect("load shared group metadata");
        assert!(metadata.node_shunt_enabled);

        server.abort();
    }

    #[tokio::test]
    async fn create_api_key_account_reports_conflict_when_post_create_sync_lacks_node_shunt_slot() {
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

        assert_eq!(err.0, StatusCode::CONFLICT);
        assert_eq!(err.1, group_node_shunt_unassigned_error_message());
    }

    #[tokio::test]
    async fn update_upstream_account_persists_node_shunt_for_existing_multi_account_group() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = insert_api_key_account(&state.pool, "Shared Group Target").await;
        let sibling_account_id = insert_api_key_account(&state.pool, "Shared Group Sibling").await;
        for grouped_account_id in [account_id, sibling_account_id] {
            set_test_account_group_name(
                &state.pool,
                grouped_account_id,
                Some("shared-update-group"),
            )
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
                upstream_429_retry_enabled: false,
                upstream_429_max_retries: 0,
                concurrency_limit: 0,
            },
        )
        .await
        .expect("save shared update group metadata");
        drop(conn);

        let Json(detail) = update_upstream_account(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath(account_id),
            Json(UpdateUpstreamAccountRequest {
                display_name: None,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: Some(true),
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
            }),
        )
        .await
        .expect("update shared group account");

        assert_eq!(
            detail.summary.group_name.as_deref(),
            Some("shared-update-group")
        );
        let metadata = load_group_metadata(&state.pool, Some("shared-update-group"))
            .await
            .expect("load shared update group metadata");
        assert!(metadata.node_shunt_enabled);
    }

    #[tokio::test]
    async fn resolve_required_group_proxy_binding_for_write_allows_node_shunt_without_selectable_nodes()
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
    async fn resolve_required_group_proxy_binding_for_write_rejects_empty_bindings_for_node_shunt()
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
    async fn build_imported_oauth_validation_response_returns_assignment_errors() {
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
    async fn create_bulk_upstream_account_sync_job_reuses_existing_running_job() {
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
    async fn finish_bulk_sync_job_completed_exposes_completed_status_in_events_and_response() {
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
    async fn finish_bulk_sync_job_failed_exposes_failed_status_in_events_and_response() {
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

    #[tokio::test]
    async fn finish_bulk_sync_job_cancelled_exposes_cancelled_status_in_events_and_response() {
        let job = Arc::new(BulkUpstreamAccountSyncJob::new(
            BulkUpstreamAccountSyncSnapshot {
                job_id: "job-cancelled".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
                rows: vec![BulkUpstreamAccountSyncRow {
                    account_id: 5,
                    display_name: "Existing OAuth".to_string(),
                    status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED.to_string(),
                    detail: Some("disabled accounts cannot be synced".to_string()),
                }],
            },
        ));
        let mut receiver = job.broadcaster.subscribe();

        finish_bulk_upstream_account_sync_job_cancelled(&job).await;

        match receiver.recv().await.expect("cancelled event") {
            BulkUpstreamAccountSyncJobEvent::Cancelled(payload) => {
                assert_eq!(
                    payload.snapshot.status,
                    BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED
                );
                assert_eq!(payload.counts.skipped, 1);
                assert_eq!(payload.counts.completed, 1);
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let response =
            build_bulk_upstream_account_sync_job_response("job-cancelled".to_string(), &job).await;
        assert_eq!(
            response.snapshot.status,
            BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED
        );
    }

    #[test]
    fn imported_snapshot_is_exhausted_when_any_limit_is_full_or_credits_are_empty() {
        let primary_exhausted = NormalizedUsageSnapshot {
            plan_type: Some("team".to_string()),
            limit_id: "limit-primary".to_string(),
            limit_name: Some("Primary".to_string()),
            primary: Some(NormalizedUsageWindow {
                used_percent: 100.0,
                window_duration_mins: 300,
                resets_at: Some("2026-03-20T05:00:00Z".to_string()),
            }),
            secondary: None,
            credits: None,
        };
        assert!(imported_snapshot_is_exhausted(&primary_exhausted));

        let credits_exhausted = NormalizedUsageSnapshot {
            plan_type: Some("team".to_string()),
            limit_id: "limit-credits".to_string(),
            limit_name: Some("Credits".to_string()),
            primary: Some(NormalizedUsageWindow {
                used_percent: 42.0,
                window_duration_mins: 300,
                resets_at: Some("2026-03-20T05:00:00Z".to_string()),
            }),
            secondary: Some(NormalizedUsageWindow {
                used_percent: 12.0,
                window_duration_mins: 10_080,
                resets_at: Some("2026-03-27T00:00:00Z".to_string()),
            }),
            credits: Some(CreditsSnapshot {
                has_credits: true,
                unlimited: false,
                balance: Some("0".to_string()),
            }),
        };
        assert!(imported_snapshot_is_exhausted(&credits_exhausted));
    }

    #[tokio::test]
    async fn resolve_pool_account_upstream_base_url_only_overrides_api_key_accounts() {
        let _upstream_lock = crate::oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;
        crate::oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;

        fn build_row(kind: &str, upstream_base_url: Option<&str>) -> UpstreamAccountRow {
            UpstreamAccountRow {
                id: 1,
                kind: kind.to_string(),
                provider: UPSTREAM_ACCOUNT_PROVIDER_CODEX.to_string(),
                display_name: "Test".to_string(),
                group_name: None,
                is_mother: 0,
                note: None,
                status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
                enabled: 1,
                email: None,
                chatgpt_account_id: None,
                chatgpt_user_id: None,
                plan_type: None,
                plan_type_observed_at: None,
                masked_api_key: None,
                encrypted_credentials: None,
                token_expires_at: None,
                last_refreshed_at: None,
                last_synced_at: None,
                last_successful_sync_at: None,
                last_activity_at: None,
                last_error: None,
                last_error_at: None,
                last_action: None,
                last_action_source: None,
                last_action_reason_code: None,
                last_action_reason_message: None,
                last_action_http_status: None,
                last_action_invoke_id: None,
                last_action_at: None,
                last_selected_at: None,
                last_route_failure_at: None,
                last_route_failure_kind: None,
                cooldown_until: None,
                consecutive_route_failures: 0,
                temporary_route_failure_streak_started_at: None,
                compact_support_status: None,
                compact_support_observed_at: None,
                compact_support_reason: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                upstream_base_url: upstream_base_url.map(str::to_string),
                created_at: "2026-03-15T00:00:00Z".to_string(),
                updated_at: "2026-03-15T00:00:00Z".to_string(),
            }
        }

        let global = Url::parse("https://api.openai.com/").expect("global upstream base url");
        let override_url = "https://proxy.example.com/gateway";
        crate::oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse("https://chatgpt.com/backend-api/codex").expect("oauth codex base"),
        )
        .await;

        let oauth_row = build_row(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX, Some(override_url));
        let oauth_resolved = resolve_pool_account_upstream_base_url(&oauth_row, &global)
            .expect("resolve oauth upstream base url");
        assert_eq!(
            oauth_resolved.as_str(),
            "https://chatgpt.com/backend-api/codex"
        );

        let api_key_row = build_row(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX, Some(override_url));
        let api_key_resolved = resolve_pool_account_upstream_base_url(&api_key_row, &global)
            .expect("resolve api key upstream base url");
        assert_eq!(
            api_key_resolved.as_str(),
            "https://proxy.example.com/gateway"
        );
    }

    #[test]
    fn parse_chatgpt_jwt_claims_extracts_identity_fields() {
        let payload = json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": "pro",
                "chatgpt_user_id": "user_123",
                "chatgpt_account_id": "org_123"
            }
        });
        let encoded = URL_SAFE_NO_PAD.encode(b"{}");
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        let token = format!("{encoded}.{body}.{encoded}");
        let claims = parse_chatgpt_jwt_claims(&token).expect("parse token");
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
        assert_eq!(claims.chatgpt_plan_type.as_deref(), Some("pro"));
        assert_eq!(claims.chatgpt_user_id.as_deref(), Some("user_123"));
        assert_eq!(claims.chatgpt_account_id.as_deref(), Some("org_123"));
    }

    #[test]
    fn build_usage_endpoint_url_preserves_backend_api_prefix() {
        let base = Url::parse("https://chatgpt.com/backend-api").expect("chatgpt base");
        let resolved = build_usage_endpoint_url(&base).expect("resolved usage url");
        assert_eq!(
            resolved.as_str(),
            "https://chatgpt.com/backend-api/wham/usage"
        );

        let base_with_slash =
            Url::parse("https://chatgpt.com/backend-api/").expect("chatgpt base with slash");
        let resolved_with_slash =
            build_usage_endpoint_url(&base_with_slash).expect("resolved usage url");
        assert_eq!(
            resolved_with_slash.as_str(),
            "https://chatgpt.com/backend-api/wham/usage"
        );
    }

    #[test]
    fn normalize_usage_snapshot_reads_windows_and_resets() {
        let payload = json!({
            "planType": "pro",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                },
                "secondaryWindow": {
                    "usedPercent": 18.5,
                    "windowDurationMins": 10080,
                    "resetsAt": 1771927200
                }
            },
            "credits": {
                "hasCredits": true,
                "unlimited": false,
                "balance": "9.99"
            }
        });
        let snapshot = normalize_usage_snapshot(&payload).expect("normalize snapshot");
        assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
        assert_eq!(
            snapshot.primary.as_ref().map(|value| value.used_percent),
            Some(42.0)
        );
        assert_eq!(
            snapshot.secondary.as_ref().map(|value| value.used_percent),
            Some(18.5)
        );
        assert_eq!(
            snapshot
                .credits
                .as_ref()
                .and_then(|value| value.balance.clone())
                .as_deref(),
            Some("9.99")
        );
    }

    fn usage_snapshot_test_config(base_url: &str, user_agent: &str) -> AppConfig {
        AppConfig {
            openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
            database_path: PathBuf::from(":memory:"),
            poll_interval: Duration::from_secs(10),
            request_timeout: Duration::from_secs(5),
            pool_upstream_responses_attempt_timeout: Duration::from_secs(
                DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
            ),
            pool_upstream_responses_total_timeout: Duration::from_secs(
                DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
            ),
            openai_proxy_handshake_timeout: Duration::from_secs(
                DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS,
            ),
            openai_proxy_compact_handshake_timeout: Duration::from_secs(
                DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS,
            ),
            openai_proxy_request_read_timeout: Duration::from_secs(
                DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS,
            ),
            openai_proxy_max_request_body_bytes: DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
            proxy_request_concurrency_limit: DEFAULT_PROXY_REQUEST_CONCURRENCY_LIMIT,
            proxy_request_concurrency_wait_timeout: Duration::from_millis(
                DEFAULT_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS,
            ),
            proxy_enforce_stream_include_usage: DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
            proxy_usage_backfill_on_startup: DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
            proxy_raw_max_bytes: DEFAULT_PROXY_RAW_MAX_BYTES,
            proxy_raw_dir: PathBuf::from("target/proxy-raw-tests"),
            proxy_raw_compression: DEFAULT_PROXY_RAW_COMPRESSION,
            proxy_raw_hot_secs: DEFAULT_PROXY_RAW_HOT_SECS,
            xray_binary: DEFAULT_XRAY_BINARY.to_string(),
            xray_runtime_dir: PathBuf::from("target/xray-forward-tests"),
            forward_proxy_algo: ForwardProxyAlgo::V1,
            max_parallel_polls: 2,
            shared_connection_parallelism: 1,
            http_bind: "127.0.0.1:0".parse().expect("valid socket address"),
            cors_allowed_origins: Vec::new(),
            list_limit_max: 100,
            user_agent: user_agent.to_string(),
            static_dir: None,
            retention_enabled: DEFAULT_RETENTION_ENABLED,
            retention_dry_run: DEFAULT_RETENTION_DRY_RUN,
            retention_interval: Duration::from_secs(DEFAULT_RETENTION_INTERVAL_SECS),
            retention_batch_rows: DEFAULT_RETENTION_BATCH_ROWS,
            retention_catchup_budget: Duration::from_secs(DEFAULT_RETENTION_CATCHUP_BUDGET_SECS),
            archive_dir: PathBuf::from("target/archive-tests"),
            codex_invocation_archive_layout: DEFAULT_CODEX_INVOCATION_ARCHIVE_LAYOUT,
            codex_invocation_archive_segment_granularity:
                DEFAULT_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY,
            invocation_archive_codec: DEFAULT_INVOCATION_ARCHIVE_CODEC,
            invocation_success_full_days: DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
            invocation_max_days: DEFAULT_INVOCATION_MAX_DAYS,
            invocation_archive_ttl_days: DEFAULT_INVOCATION_ARCHIVE_TTL_DAYS,
            forward_proxy_attempts_retention_days: DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
            pool_upstream_request_attempts_retention_days:
                DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS,
            pool_upstream_request_attempts_archive_ttl_days:
                DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
            stats_source_snapshots_retention_days: DEFAULT_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
            quota_snapshot_full_days: DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
            crs_stats: None,
            upstream_accounts_oauth_client_id: DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID
                .to_string(),
            upstream_accounts_oauth_issuer: Url::parse(DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER)
                .expect("valid oauth issuer"),
            upstream_accounts_usage_base_url: Url::parse(base_url).expect("valid usage base url"),
            upstream_accounts_login_session_ttl: Duration::from_secs(
                DEFAULT_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS,
            ),
            upstream_accounts_sync_interval: Duration::from_secs(
                DEFAULT_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
            ),
            upstream_accounts_refresh_lead_time: Duration::from_secs(
                DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS,
            ),
            upstream_accounts_history_retention_days:
                DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
            upstream_accounts_moemail: None,
        }
    }

    #[tokio::test]
    async fn fetch_usage_snapshot_retries_with_browser_user_agent() {
        #[derive(Clone)]
        struct UsageSnapshotTestState {
            requests: Arc<Mutex<Vec<String>>>,
        }

        async fn handler(
            State(state): State<UsageSnapshotTestState>,
            headers: HeaderMap,
        ) -> (StatusCode, String) {
            let user_agent = headers
                .get(header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            state.requests.lock().await.push(user_agent.clone());
            if user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT {
                (
                    StatusCode::OK,
                    json!({
                        "planType": "pro",
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
            } else {
                (
                    StatusCode::FORBIDDEN,
                    json!({ "detail": "blocked user agent" }).to_string(),
                )
            }
        }

        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state(UsageSnapshotTestState {
                requests: requests.clone(),
            });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test app");
        });

        let client = Client::builder().build().expect("client");
        let config = usage_snapshot_test_config(
            &format!("http://{addr}/backend-api"),
            "codex-vibe-monitor/0.2.0",
        );

        let snapshot = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
            .await
            .expect("fetch usage snapshot");

        assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
        let recorded = requests.lock().await.clone();
        assert_eq!(
            recorded,
            vec![
                "codex-vibe-monitor/0.2.0".to_string(),
                UPSTREAM_USAGE_BROWSER_USER_AGENT.to_string()
            ]
        );

        server.abort();
    }

    #[test]
    fn build_manual_callback_redirect_uri_targets_localhost() {
        let redirect = build_manual_callback_redirect_uri().expect("redirect uri");
        assert!(redirect.starts_with("http://localhost:"));
        assert!(redirect.ends_with("/auth/callback"));
    }

    #[test]
    fn parse_manual_oauth_callback_accepts_expected_redirect() {
        let query = parse_manual_oauth_callback(
            "http://localhost:37891/auth/callback?code=test-code&state=test-state",
            "http://localhost:37891/auth/callback",
        )
        .expect("callback query");
        assert_eq!(query.code.as_deref(), Some("test-code"));
        assert_eq!(query.state.as_deref(), Some("test-state"));
    }

    #[test]
    fn build_oauth_authorize_url_requests_official_scopes_and_audience() {
        let url = build_oauth_authorize_url(
            &Url::parse("https://auth.openai.com").expect("issuer"),
            "client-id",
            "http://localhost:1455/auth/callback",
            "state-token",
            "challenge",
        )
        .expect("build authorize url");
        let parsed = Url::parse(&url).expect("parse authorize url");
        let query = parsed.query_pairs().into_owned().collect::<HashMap<_, _>>();
        let scope = query
            .get("scope")
            .cloned()
            .expect("scope should be present");
        let scope_parts = scope.split_whitespace().collect::<Vec<_>>();

        assert_eq!(
            query.get("audience").map(String::as_str),
            Some(DEFAULT_OAUTH_AUDIENCE)
        );
        assert_eq!(
            query.get("prompt").map(String::as_str),
            Some(DEFAULT_OAUTH_PROMPT)
        );
        assert!(scope_parts.contains(&"openid"));
        assert!(scope_parts.contains(&"profile"));
        assert!(scope_parts.contains(&"email"));
        assert!(scope_parts.contains(&"offline_access"));
        assert_eq!(scope_parts.len(), 4);
    }

    #[test]
    fn is_reauth_error_requires_explicit_invalidated_signal() {
        assert!(is_reauth_error(&anyhow!(
            "OAuth token endpoint returned 400: invalid_grant"
        )));
        assert!(is_reauth_error(&anyhow!(
            "Authentication token has been invalidated, please sign in again"
        )));
        assert!(!is_reauth_error(&anyhow!(
            "usage endpoint returned 401: Missing scopes: api.responses.write"
        )));
        assert!(!is_reauth_error(&anyhow!(
            "pool upstream responded with 403: You have insufficient permissions for this operation."
        )));
    }

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        ensure_upstream_accounts_schema(&pool)
            .await
            .expect("ensure schema");
        pool
    }

    fn test_required_group_bound_proxy_keys() -> Vec<String> {
        vec![FORWARD_PROXY_DIRECT_KEY.to_string()]
    }

    fn test_required_group_name() -> &'static str {
        "test-direct-group"
    }

    async fn upsert_test_group_binding(
        pool: &SqlitePool,
        group_name: &str,
        bound_proxy_keys: Vec<String>,
    ) {
        let now_iso = format_utc_iso(Utc::now());
        let bound_proxy_keys_json =
            encode_group_bound_proxy_keys_json(&bound_proxy_keys).expect("encode test bindings");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_group_notes (
                group_name, note, bound_proxy_keys_json, created_at, updated_at
            ) VALUES (?1, '', ?2, ?3, ?3)
            ON CONFLICT(group_name) DO UPDATE SET
                bound_proxy_keys_json = excluded.bound_proxy_keys_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(group_name)
        .bind(bound_proxy_keys_json)
        .bind(&now_iso)
        .execute(pool)
        .await
        .expect("upsert test group binding");
    }

    async fn ensure_test_group_binding(pool: &SqlitePool, group_name: &str) {
        upsert_test_group_binding(pool, group_name, test_required_group_bound_proxy_keys()).await;
    }

    async fn set_test_account_group_name(
        pool: &SqlitePool,
        account_id: i64,
        group_name: Option<&str>,
    ) {
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET group_name = ?2,
                updated_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(group_name)
        .bind(&now_iso)
        .execute(pool)
        .await
        .expect("set test account group name");
    }

    async fn set_test_account_token_expires_at(
        pool: &SqlitePool,
        account_id: i64,
        token_expires_at: &str,
    ) {
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET token_expires_at = ?2,
                updated_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(token_expires_at)
        .bind(&now_iso)
        .execute(pool)
        .await
        .expect("set test account token expires at");
    }

    async fn test_app_state_with_usage_base(base_url: &str) -> Arc<AppState> {
        test_app_state_with_usage_base_and_parallelism(
            base_url,
            DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
        )
        .await
    }

    async fn test_app_state_with_usage_base_and_parallelism(
        base_url: &str,
        maintenance_parallelism: usize,
    ) -> Arc<AppState> {
        test_app_state_with_upstream_endpoints_and_parallelism(
            base_url,
            DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER,
            "codex-vibe-monitor/test",
            maintenance_parallelism,
        )
        .await
    }

    async fn test_app_state_with_usage_and_oauth_base(
        usage_base_url: &str,
        oauth_issuer: &str,
    ) -> Arc<AppState> {
        test_app_state_with_upstream_endpoints_and_parallelism(
            usage_base_url,
            oauth_issuer,
            UPSTREAM_USAGE_BROWSER_USER_AGENT,
            DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
        )
        .await
    }

    async fn test_app_state_with_upstream_endpoints_and_parallelism(
        usage_base_url: &str,
        oauth_issuer: &str,
        user_agent: &str,
        maintenance_parallelism: usize,
    ) -> Arc<AppState> {
        let mut config = usage_snapshot_test_config(usage_base_url, user_agent);
        config.upstream_accounts_oauth_issuer =
            Url::parse(oauth_issuer).expect("valid oauth issuer");
        test_app_state_with_config_and_parallelism(config, maintenance_parallelism).await
    }

    async fn test_app_state_with_config_and_parallelism(
        config: AppConfig,
        maintenance_parallelism: usize,
    ) -> Arc<AppState> {
        let http_clients = HttpClients::build(&config).expect("build http clients");
        let (broadcaster, _) = broadcast::channel(8);
        let proxy_raw_async_writer_limit = proxy_raw_async_writer_limit(&config);
        Arc::new(AppState {
            config,
            pool: test_pool().await,
            oauth_installation_seed: [0_u8; 32],
            http_clients,
            broadcaster,
            broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache {
                summaries: HashMap::new(),
                quota: None,
            })),
            proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
            proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
            proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
            startup_ready: Arc::new(AtomicBool::new(true)),
            shutdown: CancellationToken::new(),
            semaphore: Arc::new(Semaphore::new(4)),
            proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
            proxy_raw_async_semaphore: Arc::new(Semaphore::new(proxy_raw_async_writer_limit)),
            proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
            proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
            forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
                ForwardProxySettings::default(),
                Vec::new(),
            ))),
            xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
                "xray".to_string(),
                PathBuf::from("target/xray-supervisor-tests"),
            ))),
            forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
            forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
            pricing_settings_update_lock: Arc::new(Mutex::new(())),
            pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
            prompt_cache_conversation_cache: Arc::new(Mutex::new(
                PromptCacheConversationsCacheState {
                    entries: HashMap::new(),
                    in_flight: HashMap::new(),
                    generation: 0,
                },
            )),
            maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
            pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
            pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
            pool_group_429_retry_delay_override: None,
            pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
            hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
            upstream_accounts: Arc::new(
                UpstreamAccountsRuntime::test_instance_with_maintenance_parallelism(
                    maintenance_parallelism,
                ),
            ),
        })
    }

    async fn ensure_window_actual_usage_test_tables(pool: &SqlitePool) {
        sqlx::query(&codex_invocations_create_sql("codex_invocations"))
            .execute(pool)
            .await
            .expect("create codex_invocations table");
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS archive_batches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                dataset TEXT NOT NULL,
                month_key TEXT NOT NULL,
                day_key TEXT,
                part_key TEXT,
                file_path TEXT NOT NULL,
                status TEXT NOT NULL,
                coverage_start_at TEXT,
                coverage_end_at TEXT,
                created_at TEXT NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .expect("create archive_batches table");
    }

    fn shanghai_local_iso(timestamp: DateTime<Utc>) -> String {
        format_naive(timestamp.with_timezone(&Shanghai).naive_local())
    }

    async fn insert_window_actual_usage_invocation(
        pool: &SqlitePool,
        account_id: i64,
        occurred_at: &str,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        cache_input_tokens: Option<i64>,
        total_tokens: Option<i64>,
        cost: Option<f64>,
    ) {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                status,
                payload,
                raw_response,
                created_at
            ) VALUES (
                ?1,
                ?2,
                'test',
                ?3,
                ?4,
                ?5,
                ?6,
                ?7,
                'completed',
                ?8,
                '{}',
                ?2
            )
            "#,
        )
        .bind(format!("invoke-{}", random_base36(10).expect("invoke id")))
        .bind(occurred_at)
        .bind(input_tokens)
        .bind(output_tokens)
        .bind(cache_input_tokens)
        .bind(total_tokens)
        .bind(cost)
        .bind(json!({ "upstreamAccountId": account_id }).to_string())
        .execute(pool)
        .await
        .expect("insert codex_invocations row");
    }

    async fn seed_window_actual_usage_archive_batch(
        pool: &SqlitePool,
        archive_dir: &Path,
        batch_name: &str,
        rows: &[(
            i64,
            String,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<i64>,
            Option<f64>,
        )],
    ) -> PathBuf {
        std::fs::create_dir_all(archive_dir).expect("create archive dir");
        let archive_db_path = archive_dir.join(format!("{batch_name}.sqlite"));
        let archive_gzip_path = archive_dir.join(format!("{batch_name}.sqlite.gz"));
        let _ = std::fs::remove_file(&archive_db_path);
        let _ = std::fs::remove_file(&archive_gzip_path);
        std::fs::File::create(&archive_db_path).expect("create archive sqlite");

        let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
            .await
            .expect("open archive sqlite");
        let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
        sqlx::query(&create_sql)
            .execute(&archive_pool)
            .await
            .expect("create archive codex_invocations");

        for (index, row) in rows.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    id,
                    invoke_id,
                    occurred_at,
                    source,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_tokens,
                    cost,
                    status,
                    payload,
                    raw_response,
                    created_at
                ) VALUES (
                    ?1,
                    ?2,
                    ?3,
                    'test',
                    ?4,
                    ?5,
                    ?6,
                    ?7,
                    ?8,
                    'completed',
                    ?9,
                    '{}',
                    ?3
                )
                "#,
            )
            .bind(index as i64 + 1)
            .bind(format!(
                "archived-invoke-{}",
                random_base36(10).expect("archive invoke id")
            ))
            .bind(&row.1)
            .bind(row.2)
            .bind(row.3)
            .bind(row.4)
            .bind(row.5)
            .bind(row.6)
            .bind(json!({ "upstreamAccountId": row.0 }).to_string())
            .execute(&archive_pool)
            .await
            .expect("insert archive codex_invocations row");
        }

        archive_pool.close().await;
        deflate_sqlite_file_to_gzip(&archive_db_path, &archive_gzip_path)
            .expect("compress archive sqlite");

        let coverage_start_at = rows
            .iter()
            .map(|row| row.1.as_str())
            .min()
            .expect("archive coverage start");
        let coverage_end_at = rows
            .iter()
            .map(|row| row.1.as_str())
            .max()
            .expect("archive coverage end");
        let month_key = &coverage_start_at[..7];
        let day_key = &coverage_start_at[..10];

        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                day_key,
                part_key,
                file_path,
                status,
                coverage_start_at,
                coverage_end_at,
                created_at
            ) VALUES (
                'codex_invocations',
                ?1,
                ?2,
                'part-000',
                ?3,
                ?4,
                ?5,
                ?6,
                ?7
            )
            "#,
        )
        .bind(month_key)
        .bind(day_key)
        .bind(format!("{batch_name}.sqlite.gz"))
        .bind(ARCHIVE_STATUS_COMPLETED)
        .bind(coverage_start_at)
        .bind(coverage_end_at)
        .bind(coverage_end_at)
        .execute(pool)
        .await
        .expect("insert archive batch manifest");

        archive_gzip_path
    }

    fn assert_cost_close(actual: f64, expected: f64) {
        let diff = (actual - expected).abs();
        assert!(
            diff < 1e-9,
            "expected {expected}, got {actual}, diff={diff}"
        );
    }

    #[derive(Clone)]
    struct MoeMailStubState {
        email_domains: String,
        emails: Arc<Mutex<Vec<(String, String, Option<String>)>>>,
        generated_requests: Arc<Mutex<Vec<(String, String)>>>,
        deleted_ids: Arc<Mutex<Vec<String>>>,
        next_generated_id: Arc<AtomicUsize>,
    }

    struct MoeMailTestHarness {
        state: Arc<AppState>,
        stub: MoeMailStubState,
        server: tokio::task::JoinHandle<()>,
    }

    impl MoeMailTestHarness {
        fn abort(self) {
            self.server.abort();
        }
    }
