    async fn spawn_moemail_test_harness(
        email_domains: &str,
        emails: Vec<(String, String, Option<String>)>,
    ) -> MoeMailTestHarness {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct GenerateRequest {
            name: String,
            domain: String,
        }

        async fn config_handler(
            State(state): State<MoeMailStubState>,
        ) -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "defaultRole": "DUKE",
                "emailDomains": state.email_domains,
                "maxEmails": "20",
            }))
        }

        async fn list_emails_handler(
            State(state): State<MoeMailStubState>,
        ) -> axum::Json<serde_json::Value> {
            let emails = state.emails.lock().await.clone();
            axum::Json(json!({
                "emails": emails.into_iter().map(|(id, address, expires_at)| json!({
                    "id": id,
                    "address": address,
                    "expiresAt": expires_at,
                })).collect::<Vec<_>>(),
                "nextCursor": null,
            }))
        }

        async fn create_email_handler(
            State(state): State<MoeMailStubState>,
            axum::Json(payload): axum::Json<GenerateRequest>,
        ) -> axum::Json<serde_json::Value> {
            let index = state
                .next_generated_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            let email = format!("{}@{}", payload.name, payload.domain);
            let id = format!("generated_{index}");
            state
                .generated_requests
                .lock()
                .await
                .push((payload.name.clone(), payload.domain.clone()));
            state
                .emails
                .lock()
                .await
                .push((id.clone(), email.clone(), None));
            axum::Json(json!({ "id": id, "email": email }))
        }

        async fn messages_handler() -> axum::Json<serde_json::Value> {
            axum::Json(json!({ "messages": [] }))
        }

        async fn delete_email_handler(
            State(state): State<MoeMailStubState>,
            axum::extract::Path(email_id): axum::extract::Path<String>,
        ) -> axum::Json<serde_json::Value> {
            state.deleted_ids.lock().await.push(email_id.clone());
            state
                .emails
                .lock()
                .await
                .retain(|(existing_id, _, _)| existing_id != &email_id);
            axum::Json(json!({ "success": true }))
        }

        let stub = MoeMailStubState {
            email_domains: email_domains.to_string(),
            emails: Arc::new(Mutex::new(emails)),
            generated_requests: Arc::new(Mutex::new(Vec::new())),
            deleted_ids: Arc::new(Mutex::new(Vec::new())),
            next_generated_id: Arc::new(AtomicUsize::new(0)),
        };
        let app = Router::new()
            .route("/api/config", get(config_handler))
            .route("/api/emails", get(list_emails_handler))
            .route("/api/emails/generate", post(create_email_handler))
            .route(
                "/api/emails/:email_id",
                get(messages_handler).delete(delete_email_handler),
            )
            .with_state(stub.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind moemail test listener");
        let addr = listener.local_addr().expect("moemail listener addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve moemail test app");
        });

        let mut config = usage_snapshot_test_config(
            "https://chatgpt.com/backend-api",
            "codex-vibe-monitor/test",
        );
        config.upstream_accounts_moemail = Some(UpstreamAccountsMoeMailConfig {
            base_url: Url::parse(&format!("http://{addr}")).expect("valid moemail test url"),
            api_key: "test-moemail-key".to_string(),
            default_domain: "mail-tw.707079.xyz".to_string(),
        });
        let http_clients = HttpClients::build(&config).expect("build http clients");
        let (broadcaster, _) = broadcast::channel(8);
        let proxy_raw_async_writer_limit = proxy_raw_async_writer_limit(&config);
        let state = Arc::new(AppState {
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
            upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
        });

        MoeMailTestHarness {
            state,
            stub,
            server,
        }
    }

    fn test_claims_with_plan_type(
        email: &str,
        chatgpt_account_id: Option<&str>,
        chatgpt_user_id: Option<&str>,
        plan_type: Option<&str>,
    ) -> ChatgptJwtClaims {
        ChatgptJwtClaims {
            email: Some(email.to_string()),
            chatgpt_plan_type: plan_type.map(str::to_string),
            chatgpt_user_id: chatgpt_user_id.map(str::to_string),
            chatgpt_account_id: chatgpt_account_id.map(str::to_string),
        }
    }

    fn test_claims(
        email: &str,
        chatgpt_account_id: Option<&str>,
        chatgpt_user_id: Option<&str>,
    ) -> ChatgptJwtClaims {
        test_claims_with_plan_type(email, chatgpt_account_id, chatgpt_user_id, Some("team"))
    }

    fn test_id_token(
        email: &str,
        chatgpt_account_id: Option<&str>,
        chatgpt_user_id: Option<&str>,
        plan_type: Option<&str>,
    ) -> String {
        test_jwt_token(json!({
            "email": email,
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": plan_type,
                "chatgpt_user_id": chatgpt_user_id,
                "chatgpt_account_id": chatgpt_account_id,
            }
        }))
    }

    fn test_jwt_token(payload: serde_json::Value) -> String {
        let encoded = URL_SAFE_NO_PAD.encode(b"{}");
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        format!("{encoded}.{body}.{encoded}")
    }

    fn test_tag_routing_rule() -> TagRoutingRule {
        TagRoutingRule {
            guard_enabled: false,
            lookback_hours: None,
            max_conversations: None,
            allow_cut_out: true,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit: 0,
        }
    }

    fn test_account_tag_summary(id: i64, name: &str, concurrency_limit: i64) -> AccountTagSummary {
        let mut routing_rule = test_tag_routing_rule();
        routing_rule.concurrency_limit = concurrency_limit;
        AccountTagSummary {
            id,
            name: name.to_string(),
            routing_rule,
        }
    }

    fn test_effective_routing_rule(concurrency_limit: i64) -> EffectiveRoutingRule {
        EffectiveRoutingRule {
            guard_enabled: false,
            lookback_hours: None,
            max_conversations: None,
            allow_cut_out: true,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            concurrency_limit,
            source_tag_ids: vec![],
            source_tag_names: vec![],
            guard_rules: vec![],
        }
    }

    async fn insert_test_oauth_mailbox_session(
        pool: &SqlitePool,
        session_id: &str,
        email_address: &str,
        source: &str,
    ) {
        let now_iso = format_utc_iso(Utc::now());
        let expires_at = format_utc_iso(Utc::now() + ChronoDuration::days(1));
        let domain = email_address
            .split('@')
            .nth(1)
            .unwrap_or("mail-tw.707079.xyz");
        sqlx::query(
            r#"
            INSERT INTO pool_oauth_mailbox_sessions (
                session_id, remote_email_id, email_address, email_domain, mailbox_source,
                latest_code_value, latest_code_source, latest_code_updated_at,
                invite_subject, invite_copy_value, invite_copy_label, invite_updated_at,
                invited, last_message_id, created_at, updated_at, expires_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                NULL, NULL, NULL,
                NULL, NULL, NULL, NULL,
                0, NULL, ?6, ?6, ?7
            )
            "#,
        )
        .bind(session_id)
        .bind(format!("remote-{session_id}"))
        .bind(email_address)
        .bind(domain)
        .bind(source)
        .bind(&now_iso)
        .bind(&expires_at)
        .execute(pool)
        .await
        .expect("insert oauth mailbox session");
    }

    async fn insert_api_key_account(pool: &SqlitePool, display_name: &str) -> i64 {
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, note, status, enabled, email, chatgpt_account_id,
                chatgpt_user_id, plan_type, masked_api_key, encrypted_credentials, token_expires_at,
                last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
                local_primary_limit, local_secondary_limit, local_limit_unit, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, NULL, NULL, ?4, 1, NULL, NULL,
                NULL, NULL, ?5, ?6, NULL,
                NULL, NULL, NULL, NULL, NULL,
                NULL, NULL, NULL, ?7, ?7
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind("sk-test")
        .bind("encrypted")
        .bind(&now_iso)
        .fetch_one(pool)
        .await
        .expect("insert api key account")
    }

    async fn insert_oauth_account(pool: &SqlitePool, display_name: &str) -> i64 {
        ensure_test_group_binding(pool, test_required_group_name()).await;
        let now_iso = format_utc_iso(Utc::now());
        let token_expires_at = format_utc_iso(Utc::now() + ChronoDuration::days(30));
        sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, note, status, enabled, email, chatgpt_account_id,
                chatgpt_user_id, plan_type, masked_api_key, encrypted_credentials, token_expires_at,
                last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
                local_primary_limit, local_secondary_limit, local_limit_unit, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, NULL, ?5, 1, ?6, ?7,
                ?8, ?9, NULL, ?10, ?11,
                NULL, NULL, NULL, NULL, NULL,
                NULL, NULL, NULL, ?12, ?12
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(test_required_group_name())
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind("oauth@example.com")
        .bind("org_test")
        .bind("user_test")
        .bind("team")
        .bind("encrypted")
        .bind(&token_expires_at)
        .bind(&now_iso)
        .fetch_one(pool)
        .await
        .expect("insert oauth account")
    }

    async fn insert_syncable_oauth_account(
        pool: &SqlitePool,
        crypto_key: &[u8; 32],
        display_name: &str,
        email: &str,
        account_id: &str,
        user_id: &str,
    ) -> i64 {
        ensure_test_group_binding(pool, test_required_group_name()).await;
        let now_iso = format_utc_iso(Utc::now());
        let token_expires_at = format_utc_iso(Utc::now() + ChronoDuration::days(30));
        let encrypted_credentials = encrypt_credentials(
            crypto_key,
            &StoredCredentials::Oauth(StoredOauthCredentials {
                access_token: "access-token".to_string(),
                refresh_token: "refresh-token".to_string(),
                id_token: test_id_token(email, Some(account_id), Some(user_id), Some("team")),
                token_type: Some("Bearer".to_string()),
            }),
        )
        .expect("encrypt oauth credentials");
        sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO pool_upstream_accounts (
                kind, provider, display_name, group_name, note, status, enabled, email, chatgpt_account_id,
                chatgpt_user_id, plan_type, masked_api_key, encrypted_credentials, token_expires_at,
                last_refreshed_at, last_synced_at, last_successful_sync_at, last_error, last_error_at,
                local_primary_limit, local_secondary_limit, local_limit_unit, created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, NULL, ?5, 1, ?6, ?7,
                ?8, ?9, NULL, ?10, ?11,
                NULL, NULL, NULL, NULL, NULL,
                NULL, NULL, NULL, ?12, ?12
            ) RETURNING id
            "#,
        )
        .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
        .bind(UPSTREAM_ACCOUNT_PROVIDER_CODEX)
        .bind(display_name)
        .bind(test_required_group_name())
        .bind(UPSTREAM_ACCOUNT_STATUS_ACTIVE)
        .bind(email)
        .bind(account_id)
        .bind(user_id)
        .bind("team")
        .bind(encrypted_credentials)
        .bind(&token_expires_at)
        .bind(&now_iso)
        .fetch_one(pool)
        .await
        .expect("insert syncable oauth account")
    }

    async fn insert_test_pool_api_key_account_with_options(
        state: &Arc<AppState>,
        display_name: &str,
        api_key: &str,
        group_name: Option<&str>,
        upstream_base_url: Option<&str>,
    ) -> i64 {
        let normalized_group_name = group_name.unwrap_or(test_required_group_name());
        ensure_test_group_binding(&state.pool, normalized_group_name).await;
        let payload: CreateApiKeyAccountRequest = serde_json::from_value(serde_json::json!({
            "displayName": display_name,
            "apiKey": api_key,
            "groupName": normalized_group_name,
            "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
            "upstreamBaseUrl": upstream_base_url,
        }))
        .expect("deserialize api key account request");
        let Json(_) = create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("insert test pool api key account");
        sqlx::query_scalar("SELECT id FROM pool_upstream_accounts WHERE display_name = ?1")
            .bind(display_name)
            .fetch_one(&state.pool)
            .await
            .expect("load inserted test pool api key account id")
    }

    async fn spawn_usage_snapshot_server(
        status: StatusCode,
        body: serde_json::Value,
    ) -> (String, JoinHandle<()>) {
        async fn handler(
            State((status, body)): State<(StatusCode, Arc<String>)>,
        ) -> (StatusCode, String) {
            (status, (*body).clone())
        }

        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state((status, Arc::new(body.to_string())));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind usage snapshot server");
        let addr = listener.local_addr().expect("usage snapshot server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve usage snapshot server");
        });

        (format!("http://{addr}/backend-api"), server)
    }

    #[derive(Clone)]
    struct SequencedOauthSyncServerState {
        usage_responses: Arc<Mutex<std::collections::VecDeque<(StatusCode, String)>>>,
        usage_requests: Arc<AtomicUsize>,
        token_requests: Arc<AtomicUsize>,
        token_response: Arc<String>,
    }

    async fn spawn_sequenced_oauth_sync_server(
        usage_responses: Vec<(StatusCode, serde_json::Value)>,
        token_response: serde_json::Value,
    ) -> (
        String,
        String,
        Arc<AtomicUsize>,
        Arc<AtomicUsize>,
        JoinHandle<()>,
    ) {
        async fn usage_handler(
            State(state): State<SequencedOauthSyncServerState>,
        ) -> (StatusCode, String) {
            state.usage_requests.fetch_add(1, Ordering::SeqCst);
            let mut responses = state.usage_responses.lock().await;
            responses.pop_front().unwrap_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({
                        "error": {
                            "message": "unexpected extra usage request"
                        }
                    })
                    .to_string(),
                )
            })
        }

        async fn token_handler(
            State(state): State<SequencedOauthSyncServerState>,
        ) -> (StatusCode, String) {
            state.token_requests.fetch_add(1, Ordering::SeqCst);
            (StatusCode::OK, (*state.token_response).clone())
        }

        let usage_responses = usage_responses
            .into_iter()
            .map(|(status, body)| (status, body.to_string()))
            .collect::<std::collections::VecDeque<_>>();
        let usage_requests = Arc::new(AtomicUsize::new(0));
        let token_requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(usage_handler))
            .route("/oauth/token", post(token_handler))
            .with_state(SequencedOauthSyncServerState {
                usage_responses: Arc::new(Mutex::new(usage_responses)),
                usage_requests: usage_requests.clone(),
                token_requests: token_requests.clone(),
                token_response: Arc::new(token_response.to_string()),
            });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind sequenced oauth sync server");
        let addr = listener
            .local_addr()
            .expect("sequenced oauth sync server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve sequenced oauth sync server");
        });
        let origin = format!("http://{addr}");

        (
            format!("{origin}/backend-api"),
            origin,
            usage_requests,
            token_requests,
            server,
        )
    }

    #[derive(Clone)]
    struct ProxyOnlyOauthSyncServerState {
        usage_requests: Arc<AtomicUsize>,
        token_requests: Arc<AtomicUsize>,
    }

    async fn spawn_proxy_only_oauth_sync_server()
    -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>, JoinHandle<()>) {
        async fn handler(
            State(state): State<ProxyOnlyOauthSyncServerState>,
            request: axum::extract::Request,
        ) -> (StatusCode, String) {
            let uri_text = request.uri().to_string();
            let path = if uri_text.starts_with("http://") || uri_text.starts_with("https://") {
                Url::parse(&uri_text)
                    .map(|value| value.path().to_string())
                    .unwrap_or_else(|_| request.uri().path().to_string())
            } else {
                request.uri().path().to_string()
            };

            match (request.method().as_str(), path.as_str()) {
                ("GET", "/backend-api/wham/usage") => {
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
                ("POST", "/oauth/token") => {
                    state.token_requests.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::OK,
                        json!({
                            "access_token": "proxy-refreshed-access-token",
                            "refresh_token": "proxy-refreshed-refresh-token",
                            "id_token": test_id_token(
                                "proxy-refresh@example.com",
                                Some("org_proxy_refresh"),
                                Some("user_proxy_refresh"),
                                Some("team"),
                            ),
                            "token_type": "Bearer",
                            "expires_in": 3600
                        })
                        .to_string(),
                    )
                }
                _ => (
                    StatusCode::NOT_FOUND,
                    json!({
                        "error": {
                            "message": format!("unexpected proxy request: {} {}", request.method(), uri_text)
                        }
                    })
                    .to_string(),
                ),
            }
        }

        let usage_requests = Arc::new(AtomicUsize::new(0));
        let token_requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .fallback(any(handler))
            .with_state(ProxyOnlyOauthSyncServerState {
                usage_requests: usage_requests.clone(),
                token_requests: token_requests.clone(),
            });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind proxy-only oauth sync server");
        let addr = listener
            .local_addr()
            .expect("proxy-only oauth sync server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve proxy-only oauth sync server");
        });

        (
            format!("http://{addr}"),
            usage_requests,
            token_requests,
            server,
        )
    }

    #[derive(Clone)]
    struct TokenFailureOauthServerState {
        token_status: StatusCode,
        token_body: Arc<String>,
        token_requests: Arc<AtomicUsize>,
    }

    async fn spawn_token_failure_oauth_server(
        token_status: StatusCode,
        token_body: serde_json::Value,
    ) -> (String, String, Arc<AtomicUsize>, JoinHandle<()>) {
        async fn usage_handler() -> (StatusCode, String) {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({
                    "error": {
                        "message": "unexpected usage request during routing prepare test"
                    }
                })
                .to_string(),
            )
        }

        async fn token_handler(
            State(state): State<TokenFailureOauthServerState>,
        ) -> (StatusCode, String) {
            state.token_requests.fetch_add(1, Ordering::SeqCst);
            (state.token_status, (*state.token_body).clone())
        }

        let token_requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(usage_handler))
            .route("/oauth/token", post(token_handler))
            .with_state(TokenFailureOauthServerState {
                token_status,
                token_body: Arc::new(token_body.to_string()),
                token_requests: token_requests.clone(),
            });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind token failure oauth server");
        let addr = listener
            .local_addr()
            .expect("token failure oauth server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve token failure oauth server");
        });
        let origin = format!("http://{addr}");

        (
            format!("{origin}/backend-api"),
            origin,
            token_requests,
            server,
        )
    }

    #[derive(Clone)]
    struct BlockingUsageServerState {
        started: Arc<AtomicBool>,
        release: Arc<Notify>,
        requests: Arc<AtomicUsize>,
    }

    async fn spawn_blocking_usage_server() -> (
        String,
        Arc<AtomicBool>,
        Arc<Notify>,
        Arc<AtomicUsize>,
        JoinHandle<()>,
    ) {
        async fn handler(State(state): State<BlockingUsageServerState>) -> (StatusCode, String) {
            state.requests.fetch_add(1, Ordering::SeqCst);
            state.started.store(true, Ordering::SeqCst);
            state.release.notified().await;
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

        let started = Arc::new(AtomicBool::new(false));
        let release = Arc::new(Notify::new());
        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state(BlockingUsageServerState {
                started: started.clone(),
                release: release.clone(),
                requests: requests.clone(),
            });
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind blocking usage server");
        let addr = listener.local_addr().expect("blocking usage server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve blocking usage server");
        });

        (
            format!("http://{addr}/backend-api"),
            started,
            release,
            requests,
            server,
        )
    }

    async fn wait_for_atomic_true(flag: &AtomicBool) {
        timeout(Duration::from_secs(3), async {
            while !flag.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("flag should become true");
    }

    async fn wait_for_atomic_usize(flag: &AtomicUsize, expected: usize) {
        timeout(Duration::from_secs(3), async {
            while flag.load(Ordering::SeqCst) < expected {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("counter should reach expected value");
    }

    #[tokio::test]
    async fn maintenance_pass_dispatches_without_waiting_for_sync_completion() {
        let (base_url, started, release, requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Queued Maintenance OAuth",
            "queued-maintenance@example.com",
            "org_queued_maintenance",
            "user_queued_maintenance",
        )
        .await;

        let started_at = std::time::Instant::now();
        run_upstream_account_maintenance_once(state.clone())
            .await
            .expect("maintenance pass should dispatch");
        assert!(
            started_at.elapsed() < Duration::from_secs(1),
            "maintenance pass should return after dispatching work"
        );

        wait_for_atomic_true(started.as_ref()).await;
        release.notify_waiters();
        timeout(Duration::from_secs(1), async {
            while requests.load(Ordering::SeqCst) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("queued maintenance request should complete");
        server.abort();
    }

    #[tokio::test]
    async fn drain_background_tasks_waits_for_queued_maintenance_syncs() {
        let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Drain Maintenance OAuth",
            "drain-maintenance@example.com",
            "org_drain_maintenance",
            "user_drain_maintenance",
        )
        .await;

        run_upstream_account_maintenance_once(state.clone())
            .await
            .expect("maintenance pass should dispatch");
        wait_for_atomic_true(started.as_ref()).await;

        let mut drain_task = tokio::spawn({
            let runtime = state.upstream_accounts.clone();
            async move {
                runtime.drain_background_tasks().await;
            }
        });
        assert!(
            timeout(Duration::from_millis(150), &mut drain_task)
                .await
                .is_err(),
            "drain should wait for queued maintenance tasks"
        );

        release.notify_waiters();
        drain_task.await.expect("drain join should succeed");
        server.abort();
    }

    #[tokio::test]
    async fn maintenance_dispatch_respects_parallelism_limit() {
        async fn handler(
            State((requests, release)): State<(Arc<AtomicUsize>, Arc<Semaphore>)>,
        ) -> (StatusCode, String) {
            requests.fetch_add(1, Ordering::SeqCst);
            let _permit = release
                .acquire()
                .await
                .expect("test release semaphore should stay open");
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

        let requests = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Semaphore::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state((requests.clone(), release.clone()));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind bounded usage server");
        let addr = listener.local_addr().expect("bounded usage server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve bounded usage server");
        });

        let state = test_app_state_with_usage_base_and_parallelism(
            &format!("http://{addr}/backend-api"),
            1,
        )
        .await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Bounded Maintenance A",
            "bounded-a@example.com",
            "org_bounded_a",
            "user_bounded_a",
        )
        .await;
        insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Bounded Maintenance B",
            "bounded-b@example.com",
            "org_bounded_b",
            "user_bounded_b",
        )
        .await;

        run_upstream_account_maintenance_once(state.clone())
            .await
            .expect("dispatch maintenance pass");
        wait_for_atomic_usize(requests.as_ref(), 1).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "only one maintenance sync should reach the upstream at a time"
        );

        release.add_permits(1);
        wait_for_atomic_usize(requests.as_ref(), 2).await;
        release.add_permits(1);
        server.abort();
    }

    #[tokio::test]
    async fn maintenance_sync_does_not_block_unrelated_account_updates() {
        let (base_url, started, release, requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let maintenance_account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Maintenance OAuth",
            "maintenance@example.com",
            "org_maintenance",
            "user_maintenance",
        )
        .await;
        let updated_account_id = insert_api_key_account(&state.pool, "Unrelated API Key").await;

        let maintenance_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_maintenance_sync(state.clone(), maintenance_account_id)
                    .await
            }
        });
        wait_for_atomic_true(started.as_ref()).await;

        let started_at = std::time::Instant::now();
        state
            .upstream_accounts
            .account_ops
            .run_update_account(
                state.clone(),
                updated_account_id,
                UpdateUpstreamAccountRequest {
                    display_name: None,
                    group_name: None,
                    group_bound_proxy_keys: None,
                    group_node_shunt_enabled: None,
                    note: Some("updated while maintenance runs".to_string()),
                    group_note: None,
                    concurrency_limit: None,
                    upstream_base_url: OptionalField::Missing,
                    enabled: Some(false),
                    is_mother: None,
                    api_key: None,
                    local_primary_limit: None,
                    local_secondary_limit: None,
                    local_limit_unit: None,
                    tag_ids: None,
                },
            )
            .await
            .expect("update unrelated account");
        assert!(
            started_at.elapsed() < Duration::from_secs(1),
            "unrelated account update should not wait for maintenance"
        );

        let updated_row = load_upstream_account_row(&state.pool, updated_account_id)
            .await
            .expect("load updated account")
            .expect("updated account exists");
        assert_eq!(updated_row.enabled, 0);
        assert_eq!(
            updated_row.note.as_deref(),
            Some("updated while maintenance runs")
        );

        release.notify_waiters();
        assert_eq!(
            maintenance_task
                .await
                .expect("maintenance join")
                .expect("maintenance result"),
            MaintenanceDispatchOutcome::Executed
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn persist_imported_oauth_waits_for_inflight_maintenance() {
        let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Imported OAuth",
            "imported@example.com",
            "org_imported",
            "user_imported",
        )
        .await;

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

        let probe = ImportedOauthProbeOutcome {
            token_expires_at: format_utc_iso(Utc::now() + ChronoDuration::days(30)),
            credentials: StoredOauthCredentials {
                access_token: "imported-access-token".to_string(),
                refresh_token: "imported-refresh-token".to_string(),
                id_token: test_id_token(
                    "imported@example.com",
                    Some("org_imported"),
                    Some("user_imported"),
                    Some("team"),
                ),
                token_type: Some("Bearer".to_string()),
            },
            claims: test_claims(
                "imported@example.com",
                Some("org_imported"),
                Some("user_imported"),
            ),
            usage_snapshot: None,
            exhausted: false,
            usage_snapshot_warning: Some("usage snapshot unavailable".to_string()),
        };

        let mut import_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_persist_imported_oauth(state.clone(), account_id, probe)
                    .await
            }
        });
        assert!(
            timeout(Duration::from_millis(150), &mut import_task)
                .await
                .is_err(),
            "post-import updates should queue behind same-account maintenance"
        );

        release.notify_waiters();
        assert_eq!(
            maintenance_task
                .await
                .expect("maintenance join")
                .expect("maintenance result"),
            MaintenanceDispatchOutcome::Executed
        );
        assert_eq!(
            import_task
                .await
                .expect("import join")
                .expect("persist imported oauth"),
            Some("usage snapshot unavailable".to_string())
        );
        server.abort();
    }

    #[tokio::test]
    async fn same_account_updates_wait_for_inflight_maintenance() {
        let (base_url, started, release, _requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Serialized OAuth",
            "serialized@example.com",
            "org_serialized",
            "user_serialized",
        )
        .await;

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

        let mut update_task = tokio::spawn({
            let state = state.clone();
            async move {
                state
                    .upstream_accounts
                    .account_ops
                    .run_update_account(
                        state.clone(),
                        account_id,
                        UpdateUpstreamAccountRequest {
                            display_name: None,
                            group_name: None,
                            group_bound_proxy_keys: None,
                            group_node_shunt_enabled: None,
                            note: Some("queued note".to_string()),
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
                        },
                    )
                    .await
            }
        });
        assert!(
            timeout(Duration::from_millis(150), &mut update_task)
                .await
                .is_err(),
            "same-account update should queue behind maintenance"
        );

        let row_during_maintenance = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load row during maintenance")
            .expect("row exists");
        assert_eq!(row_during_maintenance.note, None);

        release.notify_waiters();
        assert_eq!(
            maintenance_task
                .await
                .expect("maintenance join")
                .expect("maintenance result"),
            MaintenanceDispatchOutcome::Executed
        );
        update_task
            .await
            .expect("update join")
            .expect("update result");

        let updated_row = load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load updated row")
            .expect("updated row exists");
        assert_eq!(updated_row.note.as_deref(), Some("queued note"));
        server.abort();
    }

    #[tokio::test]
    async fn maintenance_sync_deduplicates_same_account_work() {
        let (base_url, started, release, requests, server) = spawn_blocking_usage_server().await;
        let state = test_app_state_with_usage_base(&base_url).await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Deduped OAuth",
            "deduped@example.com",
            "org_deduped",
            "user_deduped",
        )
        .await;

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

        let second = state
            .upstream_accounts
            .account_ops
            .run_maintenance_sync(state.clone(), account_id)
            .await
            .expect("second maintenance result");
        assert_eq!(second, MaintenanceDispatchOutcome::Deduped);

        release.notify_waiters();
        assert_eq!(
            maintenance_task
                .await
                .expect("maintenance join")
                .expect("maintenance result"),
            MaintenanceDispatchOutcome::Executed
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn queued_maintenance_sync_revalidates_due_window_before_execution() {
        async fn handler(State(requests): State<Arc<AtomicUsize>>) -> (StatusCode, String) {
            requests.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::OK,
                json!({
                    "planType": "team",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        },
                        "secondaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 10080,
                            "resetsAt": 1771927200
                        }
                    }
                })
                .to_string(),
            )
        }

        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state(requests.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind usage server");
        let addr = listener.local_addr().expect("usage server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve usage server");
        });

        let state = test_app_state_with_usage_base_and_parallelism(
            &format!("http://{addr}/backend-api"),
            1,
        )
        .await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Queued Revalidation OAuth",
            "queued-revalidation@example.com",
            "org_queued_revalidation",
            "user_queued_revalidation",
        )
        .await;
        insert_limit_sample_with_usage(
            &state.pool,
            account_id,
            "2026-03-23T11:00:00Z",
            Some(12.0),
            Some(10.0),
        )
        .await;

        let due_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(10));
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&due_at)
        .execute(&state.pool)
        .await
        .expect("seed due sync time");

        let held_slot = state
            .upstream_accounts
            .account_ops
            .maintenance_slots
            .clone()
            .acquire_owned()
            .await
            .expect("hold maintenance slot");
        assert_eq!(
            state
                .upstream_accounts
                .account_ops
                .dispatch_maintenance_sync(
                    state.clone(),
                    MaintenanceDispatchPlan {
                        account_id,
                        tier: MaintenanceTier::Priority,
                        sync_interval_secs: 300,
                    },
                )
                .expect("queue maintenance plan"),
            MaintenanceQueueOutcome::Queued
        );

        let fresh_sync_at = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&fresh_sync_at)
        .execute(&state.pool)
        .await
        .expect("refresh sync time before queued maintenance executes");

        drop(held_slot);
        state.upstream_accounts.drain_background_tasks().await;

        assert_eq!(
            requests.load(Ordering::SeqCst),
            0,
            "queued maintenance should skip once a newer sync makes the plan stale"
        );
        server.abort();
    }

    #[tokio::test]
    async fn queued_high_frequency_maintenance_sync_revalidates_current_tier_before_execution() {
        async fn handler(State(requests): State<Arc<AtomicUsize>>) -> (StatusCode, String) {
            requests.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::OK,
                json!({
                    "planType": "team",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        },
                        "secondaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 10080,
                            "resetsAt": 1771927200
                        }
                    }
                })
                .to_string(),
            )
        }

        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state(requests.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind usage server");
        let addr = listener.local_addr().expect("usage server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve usage server");
        });

        let state = test_app_state_with_usage_base_and_parallelism(
            &format!("http://{addr}/backend-api"),
            1,
        )
        .await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Queued High Frequency OAuth",
            "queued-high-frequency@example.com",
            "org_queued_high_frequency",
            "user_queued_high_frequency",
        )
        .await;
        insert_limit_sample_with_usage(
            &state.pool,
            account_id,
            "2026-03-23T11:00:00Z",
            Some(12.0),
            Some(10.0),
        )
        .await;

        let due_at = format_utc_iso(Utc::now() - ChronoDuration::seconds(70));
        let working_selected_at = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2,
                last_selected_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&due_at)
        .bind(&working_selected_at)
        .execute(&state.pool)
        .await
        .expect("seed high-frequency due account");

        let held_slot = state
            .upstream_accounts
            .account_ops
            .maintenance_slots
            .clone()
            .acquire_owned()
            .await
            .expect("hold maintenance slot");
        assert_eq!(
            state
                .upstream_accounts
                .account_ops
                .dispatch_maintenance_sync(
                    state.clone(),
                    MaintenanceDispatchPlan {
                        account_id,
                        tier: MaintenanceTier::HighFrequency,
                        sync_interval_secs: MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
                    },
                )
                .expect("queue maintenance plan"),
            MaintenanceQueueOutcome::Queued
        );

        let idle_selected_at = format_utc_iso(
            Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES + 1),
        );
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_selected_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&idle_selected_at)
        .execute(&state.pool)
        .await
        .expect("drop account out of high-frequency tier before queued maintenance executes");

        drop(held_slot);
        state.upstream_accounts.drain_background_tasks().await;

        assert_eq!(
            requests.load(Ordering::SeqCst),
            0,
            "queued high-frequency maintenance should skip once the account falls back to a slower tier"
        );
        server.abort();
    }

    #[tokio::test]
    async fn queued_high_frequency_maintenance_sync_recomputes_priority_fallback_before_execution()
    {
        async fn handler(State(requests): State<Arc<AtomicUsize>>) -> (StatusCode, String) {
            requests.fetch_add(1, Ordering::SeqCst);
            (
                StatusCode::OK,
                json!({
                    "planType": "team",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        },
                        "secondaryWindow": {
                            "usedPercent": 8,
                            "windowDurationMins": 10080,
                            "resetsAt": 1771927200
                        }
                    }
                })
                .to_string(),
            )
        }

        let requests = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/backend-api/wham/usage", get(handler))
            .with_state(requests.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind usage server");
        let addr = listener.local_addr().expect("usage server addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve usage server");
        });

        let state = test_app_state_with_usage_base_and_parallelism(
            &format!("http://{addr}/backend-api"),
            1,
        )
        .await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        let account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Queued High Frequency Re-rank OAuth",
            "queued-high-frequency-rerank@example.com",
            "org_queued_high_frequency_rerank",
            "user_queued_high_frequency_rerank",
        )
        .await;
        insert_limit_sample_with_usage(
            &state.pool,
            account_id,
            "2026-03-23T11:00:00Z",
            Some(20.0),
            Some(20.0),
        )
        .await;

        let competing_account_id = insert_syncable_oauth_account(
            &state.pool,
            crypto_key,
            "Competing Priority OAuth",
            "competing-priority@example.com",
            "org_competing_priority",
            "user_competing_priority",
        )
        .await;
        insert_limit_sample_with_usage(
            &state.pool,
            competing_account_id,
            "2026-03-23T11:00:00Z",
            Some(5.0),
            Some(5.0),
        )
        .await;

        let due_at = format_utc_iso(Utc::now() - ChronoDuration::seconds(400));
        let working_selected_at = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2,
                last_selected_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&due_at)
        .bind(&working_selected_at)
        .execute(&state.pool)
        .await
        .expect("seed due high-frequency account");

        let competing_synced_at = format_utc_iso(Utc::now() - ChronoDuration::seconds(120));
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_synced_at = ?2,
                last_successful_sync_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(competing_account_id)
        .bind(&competing_synced_at)
        .execute(&state.pool)
        .await
        .expect("seed competing account sync time");

        let held_slot = state
            .upstream_accounts
            .account_ops
            .maintenance_slots
            .clone()
            .acquire_owned()
            .await
            .expect("hold maintenance slot");
        assert_eq!(
            state
                .upstream_accounts
                .account_ops
                .dispatch_maintenance_sync(
                    state.clone(),
                    MaintenanceDispatchPlan {
                        account_id,
                        tier: MaintenanceTier::HighFrequency,
                        sync_interval_secs: MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
                    },
                )
                .expect("queue maintenance plan"),
            MaintenanceQueueOutcome::Queued
        );

        let idle_selected_at = format_utc_iso(
            Utc::now() - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES + 1),
        );
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_selected_at = ?2
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(&idle_selected_at)
        .execute(&state.pool)
        .await
        .expect("drop queued account out of high-frequency tier");

        sqlx::query(
            r#"
            UPDATE pool_upstream_account_limit_samples
            SET primary_used_percent = 100.0,
                secondary_used_percent = 100.0
            WHERE account_id = ?1
            "#,
        )
        .bind(competing_account_id)
        .execute(&state.pool)
        .await
        .expect("remove competing account from available priority competition");

        drop(held_slot);
        state.upstream_accounts.drain_background_tasks().await;

        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "queued high-frequency maintenance should rerun against the current priority cadence after pool ranks change"
        );
        server.abort();
    }

    #[tokio::test]
    async fn maintenance_dedupe_flag_resets_after_panicking_job() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let account_id = 777_i64;

        let first: Result<AccountSubmitOutcome<()>, AccountCommandDispatchError<anyhow::Error>> =
            state
                .upstream_accounts
                .account_ops
                .submit_command(
                    state.clone(),
                    account_id,
                    AccountCommand::MaintenanceSync,
                    true,
                    |_state, _id| async move {
                        let _: Result<(), anyhow::Error> = Ok(());
                        panic!("simulated maintenance panic");
                    },
                )
                .await;
        assert!(matches!(
            first,
            Err(AccountCommandDispatchError::ActorUnavailable(
                AccountCommand::MaintenanceSync
            ))
        ));

        let second = state
            .upstream_accounts
            .account_ops
            .submit_command(
                state.clone(),
                account_id,
                AccountCommand::MaintenanceSync,
                true,
                |_state, _id| async move { Result::<(), anyhow::Error>::Ok(()) },
            )
            .await
            .expect("second maintenance command should be accepted");
        assert!(matches!(second, AccountSubmitOutcome::Completed(())));
        assert_eq!(state.upstream_accounts.account_ops.actor_count(), 0);
    }

    #[tokio::test]
    async fn ensure_upstream_accounts_schema_seeds_pool_routing_settings_for_new_database() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        ensure_upstream_accounts_schema(&pool)
            .await
            .expect("ensure schema");

        let config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
        let row = load_pool_routing_settings_seeded(&pool, &config)
            .await
            .expect("load seeded routing settings");

        assert_eq!(row.masked_api_key, None);
        assert_eq!(row.primary_sync_interval_secs, None);
        assert_eq!(row.secondary_sync_interval_secs, None);
        assert_eq!(row.priority_available_account_cap, None);
        assert_eq!(row.responses_first_byte_timeout_secs, None);
        assert_eq!(row.compact_first_byte_timeout_secs, None);
        assert_eq!(row.responses_stream_timeout_secs, None);
        assert_eq!(row.compact_stream_timeout_secs, None);
        assert_eq!(row.default_first_byte_timeout_secs, None);
        assert_eq!(row.upstream_handshake_timeout_secs, None);
        assert_eq!(row.request_read_timeout_secs, None);
    }

    #[tokio::test]
    async fn ensure_upstream_accounts_schema_upgrades_legacy_pool_routing_settings_before_seed() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        sqlx::query(
            r#"
            CREATE TABLE pool_routing_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                encrypted_api_key TEXT,
                masked_api_key TEXT,
                primary_sync_interval_secs INTEGER,
                secondary_sync_interval_secs INTEGER,
                priority_available_account_cap INTEGER,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create legacy pool_routing_settings");
        sqlx::query(
            r#"
            INSERT INTO pool_routing_settings (
                id,
                encrypted_api_key,
                masked_api_key,
                primary_sync_interval_secs,
                secondary_sync_interval_secs,
                priority_available_account_cap,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
            "#,
        )
        .bind(POOL_SETTINGS_SINGLETON_ID)
        .bind("legacy-ciphertext")
        .bind("sk-legacy")
        .bind(300_i64)
        .bind(2400_i64)
        .bind(99_i64)
        .execute(&pool)
        .await
        .expect("insert legacy pool routing row");

        ensure_upstream_accounts_schema(&pool)
            .await
            .expect("upgrade schema");

        let columns = sqlx::query("PRAGMA table_info('pool_routing_settings')")
            .fetch_all(&pool)
            .await
            .expect("load table info")
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect::<std::collections::HashSet<_>>();
        for column in [
            "responses_first_byte_timeout_secs",
            "compact_first_byte_timeout_secs",
            "responses_stream_timeout_secs",
            "compact_stream_timeout_secs",
            "default_first_byte_timeout_secs",
            "upstream_handshake_timeout_secs",
            "request_read_timeout_secs",
        ] {
            assert!(
                columns.contains(column),
                "expected upgraded schema to contain {column}"
            );
        }

        let config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
        let row = load_pool_routing_settings_seeded(&pool, &config)
            .await
            .expect("load upgraded routing settings");
        assert_eq!(row.encrypted_api_key.as_deref(), Some("legacy-ciphertext"));
        assert_eq!(row.masked_api_key.as_deref(), Some("sk-legacy"));
        assert_eq!(row.primary_sync_interval_secs, Some(300));
        assert_eq!(row.secondary_sync_interval_secs, Some(2400));
        assert_eq!(row.priority_available_account_cap, Some(99));
        assert_eq!(row.responses_first_byte_timeout_secs, None);
        assert_eq!(row.compact_first_byte_timeout_secs, None);
        assert_eq!(row.responses_stream_timeout_secs, None);
        assert_eq!(row.compact_stream_timeout_secs, None);
        assert_eq!(row.default_first_byte_timeout_secs, None);
        assert_eq!(row.upstream_handshake_timeout_secs, None);
        assert_eq!(row.request_read_timeout_secs, None);

        let resolved = resolve_pool_routing_timeouts(&pool, &config)
            .await
            .expect("resolve routing timeouts");
        let defaults = pool_routing_timeouts_from_config(&config);
        assert_eq!(
            resolved.responses_first_byte_timeout,
            defaults.responses_first_byte_timeout
        );
        assert_eq!(
            resolved.compact_first_byte_timeout,
            defaults.compact_first_byte_timeout
        );
        assert_eq!(
            resolved.responses_stream_timeout,
            defaults.responses_stream_timeout
        );
        assert_eq!(
            resolved.compact_stream_timeout,
            defaults.compact_stream_timeout
        );
        assert_eq!(
            resolved.default_first_byte_timeout,
            defaults.default_first_byte_timeout
        );
        assert_eq!(resolved.default_send_timeout, defaults.default_send_timeout);
        assert_eq!(resolved.request_read_timeout, defaults.request_read_timeout);
    }

    #[tokio::test]
    async fn update_pool_routing_settings_allows_maintenance_only_patch() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        save_pool_routing_api_key(&state.pool, crypto_key, "pool-live-key")
            .await
            .expect("seed pool api key");

        let payload: UpdatePoolRoutingSettingsRequest = serde_json::from_value(json!({
            "maintenance": {
                "secondarySyncIntervalSecs": 2400
            }
        }))
        .expect("deserialize maintenance patch");
        let Json(response) =
            update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(payload))
                .await
                .expect("update routing settings");
        let expected_mask = mask_api_key("pool-live-key");

        assert!(response.api_key_configured);
        assert_eq!(
            response.masked_api_key.as_deref(),
            Some(expected_mask.as_str())
        );
        assert_eq!(response.maintenance.primary_sync_interval_secs, 300);
        assert_eq!(response.maintenance.secondary_sync_interval_secs, 2400);
        assert_eq!(response.maintenance.priority_available_account_cap, 100);

        let stored = load_pool_routing_settings(&state.pool)
            .await
            .expect("load routing settings");
        assert!(stored.encrypted_api_key.is_some());
        assert_eq!(stored.secondary_sync_interval_secs, Some(2400));
    }

    #[tokio::test]
    async fn warm_pool_routing_runtime_cache_best_effort_skips_invalid_encrypted_api_key() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        sqlx::query(
            r#"
            UPDATE pool_routing_settings
            SET encrypted_api_key = ?1
            WHERE id = ?2
            "#,
        )
        .bind("not-a-valid-ciphertext")
        .bind(POOL_SETTINGS_SINGLETON_ID)
        .execute(&state.pool)
        .await
        .expect("poison encrypted api key");

        {
            let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
            *runtime_cache = None;
        }

        assert!(
            refresh_pool_routing_runtime_cache(state.as_ref())
                .await
                .is_err(),
            "invalid ciphertext should still fail direct refresh"
        );

        warm_pool_routing_runtime_cache_best_effort(state.as_ref()).await;

        assert!(
            state.pool_routing_runtime_cache.lock().await.is_none(),
            "best-effort startup warmup should leave the cache empty after decrypt failures"
        );
    }

    #[tokio::test]
    async fn refresh_pool_routing_runtime_cache_preserves_last_good_cache_after_decrypt_failure() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
        let crypto_key = state
            .upstream_accounts
            .crypto_key
            .as_ref()
            .expect("test crypto key");
        save_pool_routing_api_key(&state.pool, crypto_key, "pool-live-key")
            .await
            .expect("seed pool api key");

        let cache = refresh_pool_routing_runtime_cache(state.as_ref())
            .await
            .expect("populate runtime cache");
        assert_eq!(cache.api_key.as_deref(), Some("pool-live-key"));

        sqlx::query(
            r#"
            UPDATE pool_routing_settings
            SET encrypted_api_key = ?1
            WHERE id = ?2
            "#,
        )
        .bind("not-a-valid-ciphertext")
        .bind(POOL_SETTINGS_SINGLETON_ID)
        .execute(&state.pool)
        .await
        .expect("poison encrypted api key");

        assert!(
            refresh_pool_routing_runtime_cache(state.as_ref())
                .await
                .is_err(),
            "refresh should fail once the stored api key becomes unreadable"
        );
        let cached = state.pool_routing_runtime_cache.lock().await.clone();
        assert_eq!(
            cached.as_ref().and_then(|value| value.api_key.as_deref()),
            Some("pool-live-key"),
            "failed refreshes should keep the last working routing cache in memory"
        );
    }

    fn maintenance_candidates(
        id: i64,
        status: &str,
        last_synced_at: Option<&str>,
        last_error_at: Option<&str>,
        token_expires_at: Option<&str>,
        primary_used_percent: Option<f64>,
        secondary_used_percent: Option<f64>,
    ) -> MaintenanceCandidateRow {
        MaintenanceCandidateRow {
            id,
            status: status.to_string(),
            last_synced_at: last_synced_at.map(ToOwned::to_owned),
            last_action_source: None,
            last_action_at: None,
            last_selected_at: None,
            last_error_at: last_error_at.map(ToOwned::to_owned),
            last_error: None,
            last_route_failure_at: None,
            last_route_failure_kind: None,
            last_action_reason_code: None,
            cooldown_until: None,
            temporary_route_failure_streak_started_at: None,
            token_expires_at: token_expires_at.map(ToOwned::to_owned),
            primary_used_percent,
            primary_resets_at: None,
            secondary_used_percent,
            secondary_resets_at: None,
            credits_has_credits: None,
            credits_unlimited: None,
            credits_balance: None,
        }
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_prioritizes_forced_accounts_and_overflow() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 1,
        };
        let refresh_lead_time = Duration::from_secs(15 * 60);

        let mut recent_error = maintenance_candidates(
            3,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            None,
            Some("2026-03-23T11:58:30Z"),
            Some("2026-04-23T12:00:00Z"),
            Some(8.0),
            Some(8.0),
        );
        recent_error.last_action_source =
            Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
        recent_error.last_action_at = Some("2026-03-23T11:58:30Z".to_string());

        let mut stale_error = maintenance_candidates(
            4,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            None,
            Some("2026-03-23T11:50:00Z"),
            Some("2026-04-23T12:00:00Z"),
            Some(9.0),
            Some(9.0),
        );
        stale_error.last_action_source =
            Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
        stale_error.last_action_at = Some("2026-03-23T11:50:00Z".to_string());

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![
                maintenance_candidates(
                    1,
                    UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                    Some("2026-03-23T11:40:00Z"),
                    None,
                    Some("2026-04-23T12:00:00Z"),
                    Some(15.0),
                    Some(10.0),
                ),
                maintenance_candidates(
                    2,
                    UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                    Some("2026-03-23T11:20:00Z"),
                    None,
                    Some("2026-04-23T12:00:00Z"),
                    Some(12.0),
                    Some(22.0),
                ),
                recent_error,
                stale_error,
                maintenance_candidates(
                    5,
                    UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                    Some("2026-03-23T11:50:00Z"),
                    None,
                    Some("2026-04-23T12:00:00Z"),
                    Some(5.0),
                    None,
                ),
                maintenance_candidates(
                    6,
                    UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                    Some("2026-03-23T11:54:00Z"),
                    None,
                    Some("2026-03-23T12:10:00Z"),
                    Some(4.0),
                    Some(4.0),
                ),
            ],
            settings,
            refresh_lead_time,
            now,
        );

        let plan_map = plans
            .into_iter()
            .map(|plan| (plan.account_id, (plan.tier, plan.sync_interval_secs)))
            .collect::<HashMap<_, _>>();
        assert_eq!(plan_map.get(&1), Some(&(MaintenanceTier::Priority, 300)));
        assert_eq!(plan_map.get(&2), Some(&(MaintenanceTier::Secondary, 1800)));
        assert_eq!(plan_map.get(&4), Some(&(MaintenanceTier::Priority, 300)));
        assert_eq!(plan_map.get(&5), Some(&(MaintenanceTier::Priority, 300)));
        assert_eq!(plan_map.get(&6), Some(&(MaintenanceTier::Priority, 300)));
        assert!(!plan_map.contains_key(&3));
    }

    #[test]
    fn maintenance_plan_is_not_due_during_upstream_rejected_cooldown() {
        let now = Utc
            .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 1,
        };
        let mut candidate = maintenance_candidates(
            42,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            Some("2026-04-13T05:00:00Z"),
            Some("2026-04-13T05:00:00Z"),
            Some("2026-05-13T12:00:00Z"),
            Some(10.0),
            Some(10.0),
        );
        candidate.last_action_reason_code = Some("upstream_http_402".to_string());
        candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
        candidate.cooldown_until = Some(format_utc_iso(
            now + ChronoDuration::seconds(
                UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS,
            ),
        ));

        assert!(
            !maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
            "active upstream-rejected cooldown should suppress maintenance scheduling"
        );
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_keeps_refresh_due_accounts_on_primary_cadence() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 100,
        };

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![maintenance_candidates(
                7,
                UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                Some("2026-03-23T11:59:00Z"),
                None,
                Some("2026-03-23T12:10:00Z"),
                Some(6.0),
                Some(6.0),
            )],
            settings,
            Duration::from_secs(15 * 60),
            now,
        );

        assert!(
            plans.is_empty(),
            "refresh-due accounts should stay on the configured primary cadence until the interval elapses"
        );
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_routes_working_accounts_to_high_frequency() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 1,
        };
        let mut candidate = maintenance_candidates(
            8,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:58:30Z"),
            None,
            Some("2026-04-23T12:00:00Z"),
            Some(10.0),
            Some(10.0),
        );
        candidate.last_selected_at = Some("2026-03-23T11:59:30Z".to_string());

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![candidate],
            settings,
            Duration::from_secs(15 * 60),
            now,
        );

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].tier, MaintenanceTier::HighFrequency);
        assert_eq!(plans[0].sync_interval_secs, 60);
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_keeps_working_refresh_due_accounts_high_frequency() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 1,
        };
        let mut candidate = maintenance_candidates(
            81,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:58:30Z"),
            None,
            Some("2026-03-23T12:10:00Z"),
            Some(10.0),
            Some(10.0),
        );
        candidate.last_selected_at = Some("2026-03-23T11:59:30Z".to_string());

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![candidate],
            settings,
            Duration::from_secs(15 * 60),
            now,
        );

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].tier, MaintenanceTier::HighFrequency);
        assert_eq!(plans[0].sync_interval_secs, 60);
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_routes_degraded_accounts_to_high_frequency() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 1,
        };
        let mut candidate = maintenance_candidates(
            9,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:58:30Z"),
            Some("2026-03-23T11:59:45Z"),
            Some("2026-04-23T12:00:00Z"),
            Some(10.0),
            Some(10.0),
        );
        candidate.last_route_failure_at = Some("2026-03-23T11:59:45Z".to_string());
        candidate.last_route_failure_kind = Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
        candidate.last_action_reason_code =
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE.to_string());

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![candidate],
            settings,
            Duration::from_secs(15 * 60),
            now,
        );

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].tier, MaintenanceTier::HighFrequency);
        assert_eq!(plans[0].sync_interval_secs, 60);
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_keeps_credits_exhausted_accounts_out_of_high_frequency()
     {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 1,
        };
        let mut candidate = maintenance_candidates(
            91,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:58:30Z"),
            None,
            Some("2026-04-23T12:00:00Z"),
            Some(10.0),
            Some(10.0),
        );
        candidate.last_selected_at = Some("2026-03-23T11:59:30Z".to_string());
        candidate.credits_has_credits = Some(1);
        candidate.credits_unlimited = Some(0);
        candidate.credits_balance = Some("0".to_string());

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![candidate],
            settings,
            Duration::from_secs(15 * 60),
            now,
        );

        assert_eq!(plans.len(), 0);
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_triggers_reset_due_sync_before_interval() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 100,
        };
        let mut candidate = maintenance_candidates(
            10,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:58:30Z"),
            None,
            Some("2026-04-23T12:00:00Z"),
            Some(10.0),
            Some(10.0),
        );
        candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![candidate],
            settings,
            Duration::from_secs(15 * 60),
            now,
        );

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].tier, MaintenanceTier::Priority);
        assert_eq!(plans[0].sync_interval_secs, 300);
    }

    #[test]
    fn maintenance_reset_due_only_triggers_once_per_reset_boundary() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let mut before_reset_sync = maintenance_candidates(
            11,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:58:30Z"),
            None,
            Some("2026-04-23T12:00:00Z"),
            Some(10.0),
            Some(10.0),
        );
        before_reset_sync.secondary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
        assert!(maintenance_reset_due(&before_reset_sync, now));

        let mut after_reset_sync = before_reset_sync.clone();
        after_reset_sync.last_synced_at = Some("2026-03-23T11:59:30Z".to_string());
        assert!(!maintenance_reset_due(&after_reset_sync, now));
    }

    #[test]
    fn maintenance_reset_due_stops_after_post_reset_failure_even_if_status_stays_active() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let mut candidate = maintenance_candidates(
            12,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:58:30Z"),
            None,
            Some("2026-04-23T12:00:00Z"),
            Some(10.0),
            Some(10.0),
        );
        candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
        assert!(maintenance_reset_due(&candidate, now));

        candidate.last_synced_at = Some("2026-03-23T11:59:20Z".to_string());
        candidate.last_action_source =
            Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
        candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
        candidate.last_error_at = Some("2026-03-23T11:59:20Z".to_string());
        candidate.last_route_failure_at = Some("2026-03-23T11:59:20Z".to_string());
        candidate.last_route_failure_kind = Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
        candidate.last_action_reason_code =
            Some(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE.to_string());

        assert!(!maintenance_reset_due(&candidate, now));
    }

    #[test]
    fn resolve_due_maintenance_dispatch_plans_preserves_primary_cadence_after_call_driven_error() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let settings = PoolRoutingMaintenanceSettings {
            primary_sync_interval_secs: 300,
            secondary_sync_interval_secs: 1800,
            priority_available_account_cap: 100,
        };
        let mut candidate = maintenance_candidates(
            13,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            Some("2026-03-23T11:50:00Z"),
            Some("2026-03-23T11:58:30Z"),
            Some("2026-04-23T12:00:00Z"),
            Some(10.0),
            Some(10.0),
        );
        candidate.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
        candidate.last_action_at = Some("2026-03-23T11:58:30Z".to_string());

        let plans = resolve_due_maintenance_dispatch_plans(
            vec![candidate],
            settings,
            Duration::from_secs(15 * 60),
            now,
        );

        assert!(
            plans.is_empty(),
            "call-driven error transitions should still honor the configured primary cadence"
        );
    }

    #[test]
    fn maintenance_reset_due_ignores_call_driven_error_after_reset() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("valid time");
        let mut candidate = maintenance_candidates(
            14,
            UPSTREAM_ACCOUNT_STATUS_ERROR,
            Some("2026-03-23T11:58:30Z"),
            Some("2026-03-23T11:59:20Z"),
            Some("2026-04-23T12:00:00Z"),
            Some(10.0),
            Some(10.0),
        );
        candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
        candidate.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
        candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());

        assert!(
            maintenance_reset_due(&candidate, now),
            "call-driven failures should not consume the post-reset catch-up sync"
        );
    }

    fn test_routing_candidate(id: i64) -> AccountRoutingCandidateRow {
        AccountRoutingCandidateRow {
            id,
            plan_type: None,
            secondary_used_percent: None,
            secondary_window_minutes: None,
            secondary_resets_at: None,
            primary_used_percent: None,
            primary_window_minutes: None,
            primary_resets_at: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            credits_has_credits: None,
            credits_unlimited: None,
            credits_balance: None,
            last_selected_at: Some("2026-03-23T11:00:00Z".to_string()),
            active_sticky_conversations: 0,
            in_flight_reservations: 0,
        }
    }

    #[test]
    fn compare_routing_candidates_prefers_short_window_that_is_about_to_reset() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 26, 7, 0, 0)
            .single()
            .expect("valid now");
        let mut short_reset_soon = test_routing_candidate(1);
        short_reset_soon.plan_type = Some("team".to_string());
        short_reset_soon.primary_used_percent = Some(70.0);
        short_reset_soon.primary_window_minutes = Some(300);
        short_reset_soon.primary_resets_at = Some(format_utc_iso(now + ChronoDuration::minutes(5)));
        short_reset_soon.secondary_used_percent = Some(40.0);
        short_reset_soon.secondary_window_minutes = Some(7 * 24 * 60);
        short_reset_soon.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(1)));

        let mut long_only = test_routing_candidate(2);
        long_only.plan_type = Some("free".to_string());
        long_only.secondary_used_percent = Some(30.0);
        long_only.secondary_window_minutes = Some(7 * 24 * 60);
        long_only.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(6)));

        assert_eq!(
            compare_routing_candidates_at(&short_reset_soon, &long_only, now),
            std::cmp::Ordering::Less,
            "a short-window account that is about to reset should beat a lower-used long-only account whose reset is still far away",
        );
    }

    #[test]
    fn compare_routing_candidates_penalizes_far_from_reset_pressure() {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 26, 7, 0, 0)
            .single()
            .expect("valid now");
        let mut stretched_team = test_routing_candidate(1);
        stretched_team.plan_type = Some("team".to_string());
        stretched_team.primary_used_percent = Some(95.0);
        stretched_team.primary_window_minutes = Some(300);
        stretched_team.primary_resets_at = Some(format_utc_iso(now + ChronoDuration::minutes(250)));
        stretched_team.secondary_used_percent = Some(80.0);
        stretched_team.secondary_window_minutes = Some(7 * 24 * 60);
        stretched_team.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(6)));

        let mut healthier_long = test_routing_candidate(2);
        healthier_long.plan_type = Some("free".to_string());
        healthier_long.secondary_used_percent = Some(20.0);
        healthier_long.secondary_window_minutes = Some(7 * 24 * 60);
        healthier_long.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(3)));

        assert_eq!(
            compare_routing_candidates_at(&stretched_team, &healthier_long, now),
            std::cmp::Ordering::Greater,
            "near-exhausted windows with lots of time left should sort behind healthier long-window accounts",
        );
    }

    #[test]
    fn compare_routing_candidates_treats_zero_percent_single_window_as_limited() {
        let mut single_window = test_routing_candidate(1);
        single_window.primary_used_percent = Some(0.0);
        single_window.primary_window_minutes = Some(7 * 24 * 60);
        single_window.active_sticky_conversations = 2;

        let unlimited = test_routing_candidate(2);

        assert_eq!(
            compare_routing_candidates(&single_window, &unlimited),
            std::cmp::Ordering::Greater,
            "a single remote window sample, even at 0%, should still participate in the tighter long-window load caps",
        );
    }

    #[test]
    fn candidate_capacity_profile_tightens_for_long_only_accounts() {
        let mut long_only = test_routing_candidate(1);
        long_only.secondary_used_percent = Some(10.0);
        long_only.secondary_window_minutes = Some(7 * 24 * 60);
        let mut short_window = test_routing_candidate(2);
        short_window.primary_used_percent = Some(10.0);
        short_window.primary_window_minutes = Some(300);

        let long_only_capacity = long_only.capacity_profile();
        let short_window_capacity = short_window.capacity_profile();

        assert_eq!(long_only_capacity.soft_limit, 1);
        assert_eq!(long_only_capacity.hard_cap, 2);
        assert_eq!(short_window_capacity.soft_limit, 2);
        assert_eq!(short_window_capacity.hard_cap, 3);
    }

    #[test]
    fn candidate_capacity_profile_preserves_legacy_limit_signals_without_window_metadata() {
        let mut legacy_long_only = test_routing_candidate(1);
        legacy_long_only.secondary_used_percent = Some(10.0);

        let mut locally_limited = test_routing_candidate(2);
        locally_limited.local_secondary_limit = Some(100.0);

        let legacy_capacity = legacy_long_only.capacity_profile();
        let local_capacity = locally_limited.capacity_profile();

        assert_eq!(legacy_capacity.soft_limit, 1);
        assert_eq!(legacy_capacity.hard_cap, 2);
        assert_eq!(local_capacity.soft_limit, 1);
        assert_eq!(local_capacity.hard_cap, 2);
    }

    #[test]
    fn derive_work_status_only_counts_last_selected_within_five_minute_window() {
        let now = Utc
            .with_ymd_and_hms(2026, 4, 1, 12, 0, 0)
            .single()
            .expect("valid now");
        let recent_selected =
            format_utc_iso(now - ChronoDuration::minutes(4) - ChronoDuration::seconds(59));
        let stale_selected =
            format_utc_iso(now - ChronoDuration::minutes(5) - ChronoDuration::seconds(1));

        assert_eq!(
            derive_upstream_account_work_status(
                true,
                UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
                UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
                false,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(&recent_selected),
                now,
            ),
            UPSTREAM_ACCOUNT_WORK_STATUS_WORKING
        );
        assert_eq!(
            derive_upstream_account_work_status(
                true,
                UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
                UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
                false,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(&stale_selected),
                now,
            ),
            UPSTREAM_ACCOUNT_WORK_STATUS_IDLE
        );
    }

    #[test]
    fn normalize_concurrency_limit_rejects_values_outside_supported_range() {
        assert_eq!(
            normalize_concurrency_limit(Some(-1), "concurrencyLimit"),
            Err((
                StatusCode::BAD_REQUEST,
                "concurrencyLimit must be between 0 and 30".to_string(),
            ))
        );
        assert_eq!(
            normalize_concurrency_limit(Some(31), "concurrencyLimit"),
            Err((
                StatusCode::BAD_REQUEST,
                "concurrencyLimit must be between 0 and 30".to_string(),
            ))
        );
        assert_eq!(normalize_concurrency_limit(None, "concurrencyLimit"), Ok(0));
        assert_eq!(
            normalize_concurrency_limit(Some(30), "concurrencyLimit"),
            Ok(30)
        );
    }

    #[test]
    fn build_effective_routing_rule_uses_smallest_non_zero_concurrency_limit() {
        let tags = vec![
            test_account_tag_summary(1, "unlimited", 0),
            test_account_tag_summary(2, "soft", 6),
            test_account_tag_summary(3, "strict", 3),
        ];

        let rule = build_effective_routing_rule(&tags);

        assert_eq!(rule.concurrency_limit, 3);
        assert_eq!(rule.source_tag_ids, vec![1, 2, 3]);
        assert_eq!(
            rule.source_tag_names,
            vec![
                "unlimited".to_string(),
                "soft".to_string(),
                "strict".to_string(),
            ]
        );
    }

    #[test]
    fn normalize_tag_priority_tier_defaults_to_normal_and_rejects_invalid_values() {
        assert_eq!(
            normalize_tag_priority_tier(None),
            Ok(TagPriorityTier::Normal)
        );
        assert_eq!(
            normalize_tag_priority_tier(Some("primary")),
            Ok(TagPriorityTier::Primary)
        );
        assert_eq!(
            normalize_tag_priority_tier(Some("fallback")),
            Ok(TagPriorityTier::Fallback)
        );
        assert_eq!(
            normalize_tag_priority_tier(Some("unexpected")),
            Err((
                StatusCode::BAD_REQUEST,
                "priorityTier must be one of: primary, normal, fallback".to_string(),
            ))
        );
    }

    #[test]
    fn build_effective_routing_rule_uses_most_conservative_priority_tier() {
        let mut primary = test_account_tag_summary(1, "primary", 0);
        primary.routing_rule.priority_tier = TagPriorityTier::Primary;
        let mut normal = test_account_tag_summary(2, "normal", 0);
        normal.routing_rule.priority_tier = TagPriorityTier::Normal;
        let mut fallback = test_account_tag_summary(3, "fallback", 0);
        fallback.routing_rule.priority_tier = TagPriorityTier::Fallback;

        let rule = build_effective_routing_rule(&[primary, normal, fallback]);

        assert_eq!(rule.priority_tier, TagPriorityTier::Fallback);
    }

    #[test]
    fn normalize_tag_fast_mode_rewrite_mode_defaults_to_keep_original_and_rejects_invalid_values() {
        assert_eq!(
            normalize_tag_fast_mode_rewrite_mode(None),
            Ok(TagFastModeRewriteMode::KeepOriginal)
        );
        assert_eq!(
            normalize_tag_fast_mode_rewrite_mode(Some("force_add")),
            Ok(TagFastModeRewriteMode::ForceAdd)
        );
        assert_eq!(
            normalize_tag_fast_mode_rewrite_mode(Some("unexpected")),
            Err((
                StatusCode::BAD_REQUEST,
                "fastModeRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add".to_string(),
            ))
        );
    }

    #[test]
    fn build_effective_routing_rule_uses_most_conservative_fast_mode_rewrite_mode() {
        let mut keep_original = test_account_tag_summary(1, "keep", 0);
        keep_original.routing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::KeepOriginal;
        let mut fill_missing = test_account_tag_summary(2, "fill", 0);
        fill_missing.routing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::FillMissing;
        let mut force_add = test_account_tag_summary(3, "add", 0);
        force_add.routing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::ForceAdd;
        let mut force_remove = test_account_tag_summary(4, "remove", 0);
        force_remove.routing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::ForceRemove;

        let rule =
            build_effective_routing_rule(&[keep_original, fill_missing, force_add, force_remove]);

        assert_eq!(
            rule.fast_mode_rewrite_mode,
            TagFastModeRewriteMode::ForceRemove
        );
    }

    #[test]
    fn account_accepts_concurrency_limit_treats_zero_as_unlimited_and_allows_sticky_reuse() {
        let unlimited = test_effective_routing_rule(0);
        let limited = test_effective_routing_rule(2);

        assert!(account_accepts_concurrency_limit(
            99,
            PoolRoutingSelectionSource::FreshAssignment,
            &unlimited,
        ));
        assert!(account_accepts_concurrency_limit(
            1,
            PoolRoutingSelectionSource::FreshAssignment,
            &limited,
        ));
        assert!(!account_accepts_concurrency_limit(
            2,
            PoolRoutingSelectionSource::FreshAssignment,
            &limited,
        ));
        assert!(account_accepts_concurrency_limit(
            2,
            PoolRoutingSelectionSource::StickyReuse,
            &limited,
        ));
    }

    #[tokio::test]
    async fn load_effective_routing_rule_for_account_uses_strictest_group_and_tag_limit() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Group Tag Limit").await;
        sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
            .bind(account_id)
            .bind("alpha")
            .execute(&pool)
            .await
            .expect("assign group name");

        let mut relaxed_rule = test_tag_routing_rule();
        relaxed_rule.concurrency_limit = 6;
        let relaxed_tag = insert_tag(&pool, "alpha-relaxed", &relaxed_rule)
            .await
            .expect("insert relaxed tag");

        let mut strict_rule = test_tag_routing_rule();
        strict_rule.concurrency_limit = 2;
        let strict_tag = insert_tag(&pool, "alpha-strict", &strict_rule)
            .await
            .expect("insert strict tag");

        sync_account_tag_links(
            &pool,
            account_id,
            &[relaxed_tag.summary.id, strict_tag.summary.id],
        )
        .await
        .expect("attach tags");

        let mut conn = pool.acquire().await.expect("acquire metadata conn");
        save_group_metadata_record_conn(
            &mut conn,
            "alpha",
            UpstreamAccountGroupMetadata {
                note: None,
                bound_proxy_keys: vec![],
                node_shunt_enabled: false,
                upstream_429_retry_enabled: false,
                upstream_429_max_retries: 0,
                concurrency_limit: 4,
            },
        )
        .await
        .expect("save group metadata");
        drop(conn);

        let rule = load_effective_routing_rule_for_account(&pool, account_id)
            .await
            .expect("load effective routing rule");

        assert_eq!(rule.concurrency_limit, 2);
        assert_eq!(
            rule.source_tag_ids,
            vec![relaxed_tag.summary.id, strict_tag.summary.id]
        );
    }

    #[tokio::test]
    async fn load_effective_routing_rule_for_account_uses_most_conservative_tag_priority() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Priority Merge").await;

        let mut primary_rule = test_tag_routing_rule();
        primary_rule.priority_tier = TagPriorityTier::Primary;
        let primary_tag = insert_tag(&pool, "priority-primary", &primary_rule)
            .await
            .expect("insert primary tag");

        let mut fallback_rule = test_tag_routing_rule();
        fallback_rule.priority_tier = TagPriorityTier::Fallback;
        let fallback_tag = insert_tag(&pool, "priority-fallback", &fallback_rule)
            .await
            .expect("insert fallback tag");

        sync_account_tag_links(
            &pool,
            account_id,
            &[primary_tag.summary.id, fallback_tag.summary.id],
        )
        .await
        .expect("attach priority tags");

        let rule = load_effective_routing_rule_for_account(&pool, account_id)
            .await
            .expect("load effective routing rule");

        assert_eq!(rule.priority_tier, TagPriorityTier::Fallback);
        let mut source_tag_ids = rule.source_tag_ids.clone();
        source_tag_ids.sort_unstable();
        let mut expected_tag_ids = vec![primary_tag.summary.id, fallback_tag.summary.id];
        expected_tag_ids.sort_unstable();
        assert_eq!(source_tag_ids, expected_tag_ids);
    }

    #[tokio::test]
    async fn load_effective_routing_rule_for_account_uses_most_conservative_tag_fast_mode() {
        let pool = test_pool().await;
        let account_id = insert_api_key_account(&pool, "Fast Mode Merge").await;

        let mut fill_missing_rule = test_tag_routing_rule();
        fill_missing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::FillMissing;
        let fill_missing_tag = insert_tag(&pool, "fast-fill", &fill_missing_rule)
            .await
            .expect("insert fill-missing tag");

        let mut force_remove_rule = test_tag_routing_rule();
        force_remove_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::ForceRemove;
        let force_remove_tag = insert_tag(&pool, "fast-remove", &force_remove_rule)
            .await
            .expect("insert force-remove tag");

        sync_account_tag_links(
            &pool,
            account_id,
            &[fill_missing_tag.summary.id, force_remove_tag.summary.id],
        )
        .await
        .expect("attach fast-mode tags");

        let rule = load_effective_routing_rule_for_account(&pool, account_id)
            .await
            .expect("load effective routing rule");

        assert_eq!(
            rule.fast_mode_rewrite_mode,
            TagFastModeRewriteMode::ForceRemove
        );
    }

    #[tokio::test]
    async fn tag_fast_mode_rewrite_mode_round_trips_through_create_update_get_and_list() {
        let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

        let Json(created) = create_tag(
            State(state.clone()),
            HeaderMap::new(),
            Json(CreateTagRequest {
                name: "fast-mode-round-trip".to_string(),
                guard_enabled: false,
                lookback_hours: None,
                max_conversations: None,
                allow_cut_out: true,
                allow_cut_in: true,
                priority_tier: None,
                fast_mode_rewrite_mode: None,
                concurrency_limit: None,
            }),
        )
        .await
        .expect("create tag");
        assert_eq!(
            created.summary.routing_rule.fast_mode_rewrite_mode,
            TagFastModeRewriteMode::KeepOriginal
        );

        let Json(updated) = update_tag(
            State(state.clone()),
            HeaderMap::new(),
            AxumPath(created.summary.id),
            Json(UpdateTagRequest {
                name: None,
                guard_enabled: None,
                lookback_hours: None,
                max_conversations: None,
                allow_cut_out: None,
                allow_cut_in: None,
                priority_tier: None,
                fast_mode_rewrite_mode: Some("force_add".to_string()),
                concurrency_limit: None,
            }),
        )
        .await
        .expect("update tag");
        assert_eq!(
            updated.summary.routing_rule.fast_mode_rewrite_mode,
            TagFastModeRewriteMode::ForceAdd
        );

        let Json(loaded) = get_tag(State(state.clone()), AxumPath(created.summary.id))
            .await
            .expect("get tag");
        assert_eq!(
            loaded.summary.routing_rule.fast_mode_rewrite_mode,
            TagFastModeRewriteMode::ForceAdd
        );

        let Json(listed) = list_tags(
            State(state),
            Query(ListTagsQuery {
                search: None,
                has_accounts: None,
                guard_enabled: None,
                allow_cut_in: None,
                allow_cut_out: None,
            }),
        )
        .await
        .expect("list tags");
        let listed_tag = listed
            .items
            .iter()
            .find(|item| item.id == created.summary.id)
            .expect("listed tag");
        assert_eq!(
            listed_tag.routing_rule.fast_mode_rewrite_mode,
            TagFastModeRewriteMode::ForceAdd
        );
    }
