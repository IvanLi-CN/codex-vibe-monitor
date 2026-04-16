    #[derive(Debug, FromRow)]
    struct ExternalApiKeyDbRow {
        id: i64,
        client_id: String,
        name: String,
        secret_hash: String,
        secret_prefix: String,
        status: String,
        last_used_at: Option<String>,
        rotated_from_key_id: Option<i64>,
    }

    async fn load_external_api_key_db_row(pool: &SqlitePool, id: i64) -> ExternalApiKeyDbRow {
        sqlx::query_as::<_, ExternalApiKeyDbRow>(
            r#"
            SELECT
                id,
                client_id,
                name,
                secret_hash,
                secret_prefix,
                status,
                last_used_at,
                rotated_from_key_id
            FROM external_api_keys
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("load external api key row")
    }

    async fn create_external_api_key_for_test(
        state: &Arc<AppState>,
        name: &str,
    ) -> (i64, String, ExternalApiKeyDbRow) {
        let Json(response) = create_external_api_key(
            State(state.clone()),
            external_api_key_settings_headers(),
            Json(CreateExternalApiKeyRequest {
                name: name.to_string(),
            }),
        )
        .await
        .expect("create external api key");
        let value = serde_json::to_value(&response).expect("serialize external api key response");
        let id = value["key"]["id"]
            .as_i64()
            .expect("response should include key id");
        let secret = value["secret"]
            .as_str()
            .expect("response should include secret")
            .to_string();
        let row = load_external_api_key_db_row(&state.pool, id).await;
        (id, secret, row)
    }

    fn external_api_key_settings_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            header::HeaderValue::from_static("127.0.0.1:8080"),
        );
        headers.insert(
            header::ORIGIN,
            header::HeaderValue::from_static("http://127.0.0.1:8080"),
        );
        headers
    }

    fn hash_external_api_key_secret_for_test(secret: &str) -> String {
        let digest = Sha256::digest(secret.as_bytes());
        let mut output = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut output, "{byte:02x}");
        }
        output
    }

    fn external_api_auth_headers(secret: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {secret}"))
                .expect("valid external auth header"),
        );
        headers
    }

    fn external_api_auth_headers_with_scheme(secret: &str, scheme: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("{scheme} {secret}"))
                .expect("valid external auth header"),
        );
        headers
    }

    fn test_external_oauth_credentials(
        email: &str,
        account_id: &str,
        user_id: &str,
        access_token: &str,
        refresh_token: &str,
    ) -> ExternalOauthCredentialsRequest {
        ExternalOauthCredentialsRequest {
            email: email.to_string(),
            access_token: access_token.to_string(),
            refresh_token: refresh_token.to_string(),
            id_token: test_id_token(email, Some(account_id), Some(user_id), Some("team")),
            token_type: Some("Bearer".to_string()),
            expired: Some(format_utc_iso(Utc::now() + ChronoDuration::days(30))),
        }
    }

    fn test_external_upsert_request(
        email: &str,
        account_id: &str,
        user_id: &str,
        access_token: &str,
        refresh_token: &str,
        display_name: &str,
        group_name: Option<&str>,
        note: Option<&str>,
    ) -> ExternalUpstreamAccountUpsertRequest {
        ExternalUpstreamAccountUpsertRequest {
            metadata: ExternalUpstreamAccountMetadataRequest {
                display_name: Some(display_name.to_string()),
                group_name: group_name.map(str::to_string),
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                note: note.map(str::to_string),
                group_note: None,
                concurrency_limit: None,
                enabled: Some(true),
                is_mother: Some(false),
                tag_ids: None,
            },
            oauth: test_external_oauth_credentials(
                email,
                account_id,
                user_id,
                access_token,
                refresh_token,
            ),
        }
    }

    #[tokio::test]
    async fn external_api_keys_support_rotate_disable_and_bearer_auth() {
        let (usage_base_url, server) = spawn_usage_snapshot_server(
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
        let state = test_app_state_with_usage_base(&usage_base_url).await;
        let create_without_browser_headers_err = create_external_api_key(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateExternalApiKeyRequest {
                name: "Missing Origin".to_string(),
            }),
        )
        .await
        .expect_err("missing browser same-origin headers should be rejected");
        assert_eq!(create_without_browser_headers_err.0, StatusCode::FORBIDDEN);

        let (key_id, secret, created_row) =
            create_external_api_key_for_test(&state, "Partner Alpha").await;

        assert_eq!(created_row.name, "Partner Alpha");
        assert_eq!(created_row.status, "active");
        assert_eq!(
            created_row.secret_prefix,
            secret.chars().take(12).collect::<String>()
        );
        assert_eq!(
            created_row.secret_hash,
            hash_external_api_key_secret_for_test(&secret)
        );
        assert_ne!(created_row.secret_hash, secret);
        assert!(created_row.last_used_at.is_none());

        let missing_err = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath("partner-source-1".to_string()),
            Json(test_external_upsert_request(
                "alpha@example.com",
                "org_partner_alpha",
                "user_partner_alpha",
                "alpha-access",
                "alpha-refresh",
                "Partner Alpha OAuth",
                None,
                Some("initial note"),
            )),
        )
        .await
        .expect_err("missing bearer token should be rejected");
        assert_eq!(missing_err.0, StatusCode::UNAUTHORIZED);

        let invalid_err = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers("cvm_ext_invalid"),
            AxumPath("partner-source-1".to_string()),
            Json(test_external_upsert_request(
                "alpha@example.com",
                "org_partner_alpha",
                "user_partner_alpha",
                "alpha-access",
                "alpha-refresh",
                "Partner Alpha OAuth",
                None,
                Some("initial note"),
            )),
        )
        .await
        .expect_err("unknown bearer token should be rejected");
        assert_eq!(invalid_err.0, StatusCode::UNAUTHORIZED);

        let _ = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers_with_scheme(&secret, "bearer"),
            AxumPath("partner-source-1".to_string()),
            Json(test_external_upsert_request(
                "alpha@example.com",
                "org_partner_alpha",
                "user_partner_alpha",
                "alpha-access",
                "alpha-refresh",
                "Partner Alpha OAuth",
                None,
                Some("initial note"),
            )),
        )
        .await
        .expect("active external key should authenticate");
        let used_row = load_external_api_key_db_row(&state.pool, key_id).await;
        assert!(used_row.last_used_at.is_some());

        let Json(rotated_response) =
            rotate_external_api_key(
                State(state.clone()),
                external_api_key_settings_headers(),
                AxumPath(key_id),
            )
            .await
            .expect("rotate external api key");
        let rotated_value =
            serde_json::to_value(&rotated_response).expect("serialize rotated api key");
        let rotated_id = rotated_value["key"]["id"]
            .as_i64()
            .expect("rotated response should include id");
        let rotated_secret = rotated_value["secret"]
            .as_str()
            .expect("rotated response should include secret")
            .to_string();
        let rotated_old_row = load_external_api_key_db_row(&state.pool, key_id).await;
        let rotated_new_row = load_external_api_key_db_row(&state.pool, rotated_id).await;
        assert_eq!(rotated_old_row.status, "rotated");
        assert_eq!(rotated_new_row.status, "active");
        assert_eq!(rotated_new_row.client_id, created_row.client_id);
        assert_eq!(rotated_new_row.rotated_from_key_id, Some(key_id));
        assert_ne!(rotated_new_row.secret_prefix, created_row.secret_prefix);

        let rotate_again_err =
            rotate_external_api_key(
                State(state.clone()),
                external_api_key_settings_headers(),
                AxumPath(key_id),
            )
            .await
            .expect_err("rotating the same key again should conflict");
        assert_eq!(rotate_again_err.0, StatusCode::CONFLICT);

        let rotated_old_secret_err = external_patch_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&secret),
            AxumPath("partner-source-1".to_string()),
            Json(ExternalUpstreamAccountMetadataRequest {
                note: Some("rotated old secret".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect_err("rotated secret should be forbidden");
        assert_eq!(rotated_old_secret_err.0, StatusCode::FORBIDDEN);

        let _ = external_patch_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&rotated_secret),
            AxumPath("partner-source-1".to_string()),
            Json(ExternalUpstreamAccountMetadataRequest {
                note: Some("patched by rotated secret".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect("rotated replacement secret should work");

        let Json(disabled_response) =
            disable_external_api_key(
                State(state.clone()),
                external_api_key_settings_headers(),
                AxumPath(rotated_id),
            )
            .await
            .expect("disable external api key");
        let disabled_value =
            serde_json::to_value(&disabled_response).expect("serialize disabled api key");
        assert_eq!(disabled_value["key"]["status"], "disabled");

        let disabled_err = external_patch_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&rotated_secret),
            AxumPath("partner-source-1".to_string()),
            Json(ExternalUpstreamAccountMetadataRequest {
                note: Some("disabled secret".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect_err("disabled secret should be forbidden");
        assert_eq!(disabled_err.0, StatusCode::FORBIDDEN);

        server.abort();
    }

    #[tokio::test]
    async fn external_api_keys_can_rotate_disabled_key() {
        let (usage_base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 11,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&usage_base_url).await;
        let (key_id, _secret, created_row) =
            create_external_api_key_for_test(&state, "Partner Disabled Rotate").await;

        let Json(_) = disable_external_api_key(
            State(state.clone()),
            external_api_key_settings_headers(),
            AxumPath(key_id),
        )
        .await
        .expect("disable external api key before rotate");

        let Json(rotated_response) = rotate_external_api_key(
            State(state.clone()),
            external_api_key_settings_headers(),
            AxumPath(key_id),
        )
        .await
        .expect("rotate disabled external api key");
        let rotated_value =
            serde_json::to_value(&rotated_response).expect("serialize rotated api key");
        let rotated_id = rotated_value["key"]["id"]
            .as_i64()
            .expect("rotated response should include key id");
        assert_ne!(rotated_id, key_id);
        assert_eq!(rotated_value["key"]["status"], "active");

        let previous_row = load_external_api_key_db_row(&state.pool, key_id).await;
        assert_eq!(previous_row.status, "rotated");
        let replacement_row = load_external_api_key_db_row(&state.pool, rotated_id).await;
        assert_eq!(replacement_row.status, "active");
        assert_eq!(replacement_row.client_id, created_row.client_id);
        assert_ne!(replacement_row.secret_prefix, created_row.secret_prefix);

        server.abort();
    }

    #[tokio::test]
    async fn external_oauth_upsert_is_idempotent_per_client_and_isolated_across_clients() {
        let (usage_base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 21,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&usage_base_url).await;
        let (_client_a_key_id, client_a_secret, client_a_row) =
            create_external_api_key_for_test(&state, "Partner Client A").await;
        let (_client_b_key_id, client_b_secret, client_b_row) =
            create_external_api_key_for_test(&state, "Partner Client B").await;

        let _ = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&client_a_secret),
            AxumPath("shared-source-001".to_string()),
            Json(test_external_upsert_request(
                "shared-a@example.com",
                "org_shared_a",
                "user_shared_a",
                "access-a-1",
                "refresh-a-1",
                "Shared Client A",
                None,
                Some("note-a-1"),
            )),
        )
        .await
        .expect("client A first upsert");
        let client_a_first = load_upstream_account_row_by_external_identity(
            &state.pool,
            &client_a_row.client_id,
            "shared-source-001",
        )
        .await
        .expect("load client A first account")
        .expect("client A first account should exist");

        let _ = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&client_a_secret),
            AxumPath("shared-source-001".to_string()),
            Json(test_external_upsert_request(
                "shared-a@example.com",
                "org_shared_a",
                "user_shared_a",
                "access-a-2",
                "refresh-a-2",
                "Shared Client A Updated",
                None,
                Some("note-a-2"),
            )),
        )
        .await
        .expect("client A second upsert should be idempotent");
        let client_a_second = load_upstream_account_row_by_external_identity(
            &state.pool,
            &client_a_row.client_id,
            "shared-source-001",
        )
        .await
        .expect("load client A second account")
        .expect("client A second account should exist");
        assert_eq!(client_a_second.id, client_a_first.id);
        assert_eq!(client_a_second.display_name, "Shared Client A Updated");
        assert_eq!(client_a_second.note.as_deref(), Some("note-a-2"));

        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let decrypted_client_a = decrypt_credentials(
            crypto_key,
            client_a_second
                .encrypted_credentials
                .as_deref()
                .expect("client A encrypted credentials"),
        )
        .expect("decrypt client A credentials");
        let StoredCredentials::Oauth(client_a_credentials) = decrypted_client_a else {
            panic!("client A should keep oauth credentials");
        };
        assert_eq!(client_a_credentials.access_token, "access-a-2");
        assert_eq!(client_a_credentials.refresh_token, "refresh-a-2");

        let _ = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&client_b_secret),
            AxumPath("shared-source-001".to_string()),
            Json(test_external_upsert_request(
                "shared-b@example.com",
                "org_shared_b",
                "user_shared_b",
                "access-b-1",
                "refresh-b-1",
                "Shared Client B",
                None,
                Some("note-b-1"),
            )),
        )
        .await
        .expect("client B upsert should create an isolated account");
        let client_b_account = load_upstream_account_row_by_external_identity(
            &state.pool,
            &client_b_row.client_id,
            "shared-source-001",
        )
        .await
        .expect("load client B account")
        .expect("client B account should exist");
        assert_ne!(client_b_account.id, client_a_first.id);

        let shared_source_count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM pool_upstream_accounts
            WHERE external_source_account_id = ?1
            "#,
        )
        .bind("shared-source-001")
        .fetch_one(&state.pool)
        .await
        .expect("count external source accounts");
        assert_eq!(shared_source_count, 2);

        server.abort();
    }

    #[tokio::test]
    async fn external_oauth_upsert_can_create_disabled_account() {
        let (usage_base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 17,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&usage_base_url).await;
        let (_key_id, secret, key_row) =
            create_external_api_key_for_test(&state, "Partner Disabled Create").await;

        let Json(detail) = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&secret),
            AxumPath("disabled-source-001".to_string()),
            Json(ExternalUpstreamAccountUpsertRequest {
                metadata: ExternalUpstreamAccountMetadataRequest {
                    display_name: Some("Disabled On Create".to_string()),
                    enabled: Some(false),
                    note: Some("created disabled".to_string()),
                    ..Default::default()
                },
                oauth: test_external_oauth_credentials(
                    "disabled@example.com",
                    "org_disabled_create",
                    "user_disabled_create",
                    "disabled-access-1",
                    "disabled-refresh-1",
                ),
            }),
        )
        .await
        .expect("create disabled external account");
        assert!(!detail.summary.enabled);

        let persisted = load_upstream_account_row_by_external_identity(
            &state.pool,
            &key_row.client_id,
            "disabled-source-001",
        )
        .await
        .expect("load disabled external account")
        .expect("disabled external account should exist");
        assert_eq!(persisted.enabled, 0);
        assert_eq!(persisted.note.as_deref(), Some("created disabled"));

        server.abort();
    }

    #[tokio::test]
    async fn external_oauth_patch_updates_metadata_without_overwriting_credentials() {
        let (usage_base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 9,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&usage_base_url).await;
        let (_key_id, secret, key_row) =
            create_external_api_key_for_test(&state, "Partner Patch").await;

        let _ = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&secret),
            AxumPath("patch-source-001".to_string()),
            Json(test_external_upsert_request(
                "patch@example.com",
                "org_patch",
                "user_patch",
                "patch-access-1",
                "patch-refresh-1",
                "Patch Original",
                None,
                Some("before patch"),
            )),
        )
        .await
        .expect("create patch target account");

        let before = load_upstream_account_row_by_external_identity(
            &state.pool,
            &key_row.client_id,
            "patch-source-001",
        )
        .await
        .expect("load patch target before patch")
        .expect("patch target should exist");
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let before_decrypted = decrypt_credentials(
            crypto_key,
            before
                .encrypted_credentials
                .as_deref()
                .expect("patch target encrypted credentials"),
        )
        .expect("decrypt patch target before patch");
        let StoredCredentials::Oauth(before_credentials) = before_decrypted else {
            panic!("patch target should use oauth credentials");
        };

        let _ = external_patch_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&secret),
            AxumPath("patch-source-001".to_string()),
            Json(ExternalUpstreamAccountMetadataRequest {
                display_name: Some("Patch Updated".to_string()),
                note: Some("after patch".to_string()),
                enabled: Some(false),
                is_mother: Some(true),
                ..Default::default()
            }),
        )
        .await
        .expect("patch metadata should succeed");

        let after = load_upstream_account_row_by_external_identity(
            &state.pool,
            &key_row.client_id,
            "patch-source-001",
        )
        .await
        .expect("load patch target after patch")
        .expect("patched target should exist");
        assert_eq!(after.display_name, "Patch Updated");
        assert_eq!(after.group_name, before.group_name);
        assert_eq!(after.note.as_deref(), Some("after patch"));
        assert_eq!(after.enabled, 0);
        assert_eq!(after.is_mother, 1);

        let after_decrypted = decrypt_credentials(
            crypto_key,
            after
                .encrypted_credentials
                .as_deref()
                .expect("patched target encrypted credentials"),
        )
        .expect("decrypt patch target after patch");
        let StoredCredentials::Oauth(after_credentials) = after_decrypted else {
            panic!("patched target should still use oauth credentials");
        };
        assert_eq!(after_credentials.access_token, before_credentials.access_token);
        assert_eq!(after_credentials.refresh_token, before_credentials.refresh_token);
        assert_eq!(after_credentials.id_token, before_credentials.id_token);

        server.abort();
    }

    #[tokio::test]
    async fn external_oauth_patch_waits_for_inflight_maintenance() {
        let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let (_key_id, secret, key_row) =
            create_external_api_key_for_test(&state, "Partner Patch Queue").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "External Patch Queue",
            "patch-queue@example.com",
            "org_patch_queue",
            "user_patch_queue",
        )
        .await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET external_client_id = ?2,
                external_source_account_id = ?3,
                updated_at = ?4
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&key_row.client_id)
        .bind("patch-queue-source")
        .bind(format_utc_iso(Utc::now()))
        .execute(&state.pool)
        .await
        .expect("bind external patch queue account");

        let maintenance_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_maintenance_sync(state.clone(), account_id)
                    .await
            }
        });
        wait_for_atomic_true(started.as_ref()).await;

        let mut patch_task = tokio::spawn({
            let state = state.clone();
            let secret = secret.clone();
            async move {
                external_patch_oauth_upstream_account_route(
                    State(state.clone()),
                    external_api_auth_headers(&secret),
                    AxumPath("patch-queue-source".to_string()),
                    Json(ExternalUpstreamAccountMetadataRequest {
                        note: Some("patched after maintenance".to_string()),
                        enabled: Some(false),
                        ..Default::default()
                    }),
                )
                .await
            }
        });
        assert!(
            timeout(Duration::from_millis(150), &mut patch_task)
                .await
                .is_err(),
            "external patch should wait behind same-account maintenance"
        );

        release.notify_waiters();
        assert_eq!(
            maintenance_task
                .await
                .expect("maintenance join")
                .expect("maintenance result"),
            MaintenanceDispatchOutcome::Executed
        );
        let _ = patch_task
            .await
            .expect("patch join")
            .expect("external patch should succeed");

        let updated = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load patched account")
            .expect("patched account should exist");
        assert_eq!(updated.enabled, 0);
        assert_eq!(
            updated.note.as_deref(),
            Some("patched after maintenance")
        );

        server.abort();
    }

    #[tokio::test]
    async fn external_oauth_upsert_keeps_existing_credentials_when_metadata_validation_fails() {
        let (usage_base_url, server) = spawn_usage_snapshot_server(
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 18,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            }),
        )
        .await;
        let state = test_app_state_with_usage_base(&usage_base_url).await;
        let (_key_id, secret, key_row) =
            create_external_api_key_for_test(&state, "Partner Atomicity").await;

        let _ = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&secret),
            AxumPath("atomic-source-001".to_string()),
            Json(test_external_upsert_request(
                "atomic@example.com",
                "org_atomic",
                "user_atomic",
                "atomic-access-1",
                "atomic-refresh-1",
                "Atomic Existing",
                None,
                Some("before atomic failure"),
            )),
        )
        .await
        .expect("create atomic target account");

        let _ = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&secret),
            AxumPath("atomic-source-conflict".to_string()),
            Json(test_external_upsert_request(
                "atomic-conflict@example.com",
                "org_atomic_conflict",
                "user_atomic_conflict",
                "atomic-conflict-access",
                "atomic-conflict-refresh",
                "Conflicting Display Name",
                None,
                Some("conflict holder"),
            )),
        )
        .await
        .expect("create conflicting display name holder");

        let before = load_upstream_account_row_by_external_identity(
            &state.pool,
            &key_row.client_id,
            "atomic-source-001",
        )
        .await
        .expect("load atomic target before failure")
        .expect("atomic target should exist");
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let before_decrypted = decrypt_credentials(
            crypto_key,
            before
                .encrypted_credentials
                .as_deref()
                .expect("atomic target encrypted credentials"),
        )
        .expect("decrypt atomic target before failure");
        let StoredCredentials::Oauth(before_credentials) = before_decrypted else {
            panic!("atomic target should use oauth credentials");
        };

        let err = external_upsert_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&secret),
            AxumPath("atomic-source-001".to_string()),
            Json(test_external_upsert_request(
                "atomic@example.com",
                "org_atomic",
                "user_atomic",
                "atomic-access-2",
                "atomic-refresh-2",
                "Conflicting Display Name",
                None,
                Some("after atomic failure"),
            )),
        )
        .await
        .expect_err("duplicate display name should reject the upsert");
        assert_eq!(err.0, StatusCode::CONFLICT);

        let after = load_upstream_account_row_by_external_identity(
            &state.pool,
            &key_row.client_id,
            "atomic-source-001",
        )
        .await
        .expect("load atomic target after failure")
        .expect("atomic target should still exist");
        assert_eq!(after.display_name, "Atomic Existing");
        assert_eq!(after.note.as_deref(), Some("before atomic failure"));

        let after_decrypted = decrypt_credentials(
            crypto_key,
            after
                .encrypted_credentials
                .as_deref()
                .expect("atomic target encrypted credentials after failure"),
        )
        .expect("decrypt atomic target after failure");
        let StoredCredentials::Oauth(after_credentials) = after_decrypted else {
            panic!("atomic target should still use oauth credentials");
        };
        assert_eq!(after_credentials.access_token, before_credentials.access_token);
        assert_eq!(after_credentials.refresh_token, before_credentials.refresh_token);
        assert_eq!(after_credentials.id_token, before_credentials.id_token);

        server.abort();
    }

    #[tokio::test]
    async fn external_oauth_relogin_repairs_needs_reauth_account_and_triggers_sync() {
        let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![
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
                        }),
                    ),
                    (
                        StatusCode::OK,
                        json!({
                            "planType": "team",
                            "rateLimit": {
                                "primaryWindow": {
                                    "usedPercent": 7,
                                    "windowDurationMins": 300,
                                    "resetsAt": 1771322400
                                }
                            }
                        }),
                    ),
                ],
                json!({
                    "access_token": "unused-refresh-access",
                    "refresh_token": "unused-refresh-token",
                    "id_token": test_id_token(
                        "repair@example.com",
                        Some("org_repair"),
                        Some("user_repair"),
                        Some("team"),
                    ),
                    "token_type": "Bearer",
                    "expires_in": 3600
                }),
            )
            .await;
        let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
        let (_key_id, secret, key_row) =
            create_external_api_key_for_test(&state, "Partner Relogin").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "External Repair Target",
            "repair@example.com",
            "org_repair",
            "user_repair",
        )
        .await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET external_client_id = ?2,
                external_source_account_id = ?3,
                status = ?4,
                enabled = 1,
                last_error = 'manual recovery required',
                last_error_at = ?5,
                updated_at = ?5
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&key_row.client_id)
        .bind("repair-source-001")
        .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
        .bind(format_utc_iso(Utc::now()))
        .execute(&state.pool)
        .await
        .expect("mark external repair target as needs reauth");

        let _ = external_relogin_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&secret),
            AxumPath("repair-source-001".to_string()),
            Json(ExternalUpstreamAccountReloginRequest {
                oauth: test_external_oauth_credentials(
                    "repair@example.com",
                    "org_repair",
                    "user_repair",
                    "repair-access-2",
                    "repair-refresh-2",
                ),
            }),
        )
        .await
        .expect("external relogin repair should succeed");

        let repaired = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load repaired account")
            .expect("repaired account should exist");
        assert_eq!(repaired.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(repaired.enabled, 1);
        assert!(repaired.last_synced_at.is_some());
        assert!(repaired.last_successful_sync_at.is_some());
        assert_ne!(repaired.last_action_reason_code.as_deref(), Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED));

        let decrypted = decrypt_credentials(
            crypto_key,
            repaired
                .encrypted_credentials
                .as_deref()
                .expect("repaired encrypted credentials"),
        )
        .expect("decrypt repaired credentials");
        let StoredCredentials::Oauth(credentials) = decrypted else {
            panic!("repaired account should keep oauth credentials");
        };
        assert_eq!(credentials.access_token, "repair-access-2");
        assert_eq!(credentials.refresh_token, "repair-refresh-2");

        assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
        assert!(token_requests.load(Ordering::SeqCst) <= 1);

        server.abort();
    }

    #[tokio::test]
    async fn external_oauth_relogin_preserves_disabled_account_state() {
        let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![(
                    StatusCode::OK,
                    json!({
                        "planType": "team",
                        "rateLimit": {
                            "primaryWindow": {
                                "usedPercent": 6,
                                "windowDurationMins": 300,
                                "resetsAt": 1771322400
                            }
                        }
                    }),
                )],
                json!({
                    "access_token": "unused-disabled-refresh-access",
                    "refresh_token": "unused-disabled-refresh-token",
                    "id_token": test_id_token(
                        "disabled-repair@example.com",
                        Some("org_disabled_repair"),
                        Some("user_disabled_repair"),
                        Some("team"),
                    ),
                    "token_type": "Bearer",
                    "expires_in": 3600
                }),
            )
            .await;
        let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
        let (_key_id, secret, key_row) =
            create_external_api_key_for_test(&state, "Partner Disabled Relogin").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "External Disabled Repair Target",
            "disabled-repair@example.com",
            "org_disabled_repair",
            "user_disabled_repair",
        )
        .await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET external_client_id = ?2,
                external_source_account_id = ?3,
                status = ?4,
                enabled = 0,
                last_error = 'manual recovery required',
                last_error_at = ?5,
                updated_at = ?5
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&key_row.client_id)
        .bind("disabled-repair-source-001")
        .bind(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH)
        .bind(format_utc_iso(Utc::now()))
        .execute(&state.pool)
        .await
        .expect("mark disabled external repair target as needs reauth");

        let _ = external_relogin_oauth_upstream_account_route(
            State(state.clone()),
            external_api_auth_headers(&secret),
            AxumPath("disabled-repair-source-001".to_string()),
            Json(ExternalUpstreamAccountReloginRequest {
                oauth: test_external_oauth_credentials(
                    "disabled-repair@example.com",
                    "org_disabled_repair",
                    "user_disabled_repair",
                    "disabled-repair-access-2",
                    "disabled-repair-refresh-2",
                ),
            }),
        )
        .await
        .expect("external relogin repair should preserve disabled state");

        let repaired = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load repaired disabled account")
            .expect("repaired disabled account should exist");
        assert_eq!(repaired.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
        assert_eq!(repaired.enabled, 0);
        assert!(repaired.last_synced_at.is_some());
        assert!(repaired.last_successful_sync_at.is_some());
        assert_ne!(
            repaired.last_action_reason_code.as_deref(),
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
        );

        let decrypted = decrypt_credentials(
            crypto_key,
            repaired
                .encrypted_credentials
                .as_deref()
                .expect("repaired disabled encrypted credentials"),
        )
        .expect("decrypt repaired disabled credentials");
        let StoredCredentials::Oauth(credentials) = decrypted else {
            panic!("repaired disabled account should keep oauth credentials");
        };
        assert_eq!(credentials.access_token, "disabled-repair-access-2");
        assert_eq!(credentials.refresh_token, "disabled-repair-refresh-2");

        assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
        assert!(token_requests.load(Ordering::SeqCst) <= 1);

        server.abort();
    }
