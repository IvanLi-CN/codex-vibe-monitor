#[tokio::test]
pub(crate) async fn account_attempt_list_returns_workflow_entries_and_final_success_usage_only() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Attempt workflow parity").await;
    let occurred_at = format_utc_iso(Utc::now());
    let payload = account_attempt_workflow_payload();
    seed_account_attempt_invocation(&state, &occurred_at, &payload).await;
    seed_account_attempt_rows(&state, account_id, &occurred_at).await;

    let Json(response) = list_upstream_account_attempts(
        State(state),
        AxumPath(account_id),
        Query(ListUpstreamAccountAttemptsQuery {
            attempt_type: None,
            model: None,
            sticky_key: None,
            page: Some(1),
            page_size: Some(20),
        }),
    )
    .await
    .expect("list account workflow attempts");

    let success_item = response
        .items
        .iter()
        .find(|item| item.attempt_id == "ASUCC002")
        .expect("success attempt");
    assert!(success_item.invocation_record.as_ref().unwrap().id > 0);
    assert_eq!(success_item.logical_body_bytes, Some(217958));
    assert_eq!(success_item.transmitted_body_bytes, Some(53295));
    assert_eq!(success_item.approx_upload_bytes, Some(54319));
    assert_eq!(success_item.approx_download_bytes, Some(80000));
    assert_eq!(success_item.first_token_ms, Some(780.0));

    let success_entry = success_item.workflow_entry.as_ref().unwrap();
    let success_attempt = success_entry.attempt.as_ref().unwrap();
    let request_summary = success_attempt.request_summary.as_ref().unwrap();
    assert_eq!(
        request_summary["headers"]["xForwardedFor"],
        json!("192.168.31.6")
    );
    assert_eq!(request_summary["bodyCapture"]["size"], json!(217958));
    assert_eq!(request_summary["bodyCapture"]["detailLevel"], json!("full"));
    assert_eq!(request_summary["compression"]["algorithm"], json!("zstd"));
    assert_eq!(
        request_summary["compression"]["logicalBodyBytes"],
        json!(217958)
    );
    assert_eq!(
        request_summary["compression"]["transmittedBodyBytes"],
        json!(53295)
    );
    let response_summary = success_attempt.response_summary.as_ref().unwrap();
    assert_eq!(
        response_summary["headers"]["contentEncoding"],
        json!("identity")
    );
    assert_eq!(
        response_summary["responseBodyCapture"]["size"],
        json!(79224)
    );
    assert_eq!(response_summary["usage"]["cacheWriteTokens"], json!(2090));
    assert_eq!(response_summary["usage"]["cacheInputTokens"], json!(46952));
    assert_eq!(response_summary["usage"]["outputTokens"], json!(87));
    assert_eq!(response_summary["usage"]["cost"], json!(0.0364));

    let failure_item = response
        .items
        .iter()
        .find(|item| item.attempt_id == "AFAIL001")
        .expect("failure attempt");
    assert_eq!(failure_item.first_token_ms, None);
    let failure_response_summary = failure_item
        .workflow_entry
        .as_ref()
        .unwrap()
        .attempt
        .as_ref()
        .unwrap()
        .response_summary
        .as_ref()
        .unwrap();
    assert!(failure_response_summary["usage"].is_null());
    assert_eq!(
        failure_response_summary["responseBodyCapture"]["availableAtInvocationLevel"],
        json!(false)
    );
    assert_eq!(
        failure_response_summary["responseBodyCapture"]["size"],
        json!(79224)
    );
    assert_eq!(
        failure_response_summary["responseBodyCapture"]["unavailableReason"],
        json!("attempt_response_body_not_captured")
    );
}

pub(crate) async fn insert_oauth_account(pool: &SqlitePool, display_name: &str) -> i64 {
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

pub(crate) async fn insert_test_tag(
    pool: &SqlitePool,
    name: &str,
    rule: &TagRoutingRule,
) -> Result<TagDetail> {
    let now_iso = format_utc_iso(Utc::now());
    let inserted_id = sqlx::query_scalar::<_, i64>(
        r#"
            INSERT INTO pool_tags (
                name, system_key, protected, allow_cut_out, allow_cut_in, priority_tier,
                fast_mode_rewrite_mode, concurrency_limit, upstream_429_retry_enabled,
                upstream_429_max_retries, available_models_json, created_at, updated_at
            ) VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
            RETURNING id
            "#,
    )
    .bind(name)
    .bind(format!("test:{name}"))
    .bind(if rule.allow_cut_out { 1 } else { 0 })
    .bind(if rule.allow_cut_in { 1 } else { 0 })
    .bind(rule.priority_tier.as_str())
    .bind(rule.fast_mode_rewrite_mode.as_str())
    .bind(rule.concurrency_limit)
    .bind(if rule.upstream_429_retry_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(i64::from(rule.upstream_429_max_retries))
    .bind(encode_string_array_json(&rule.available_models)?)
    .bind(&now_iso)
    .fetch_one(pool)
    .await?;
    let mut detail = load_tag_detail(pool, inserted_id)
        .await?
        .ok_or_else(|| anyhow!("tag not found after insert"))?;
    sqlx::query("UPDATE pool_tags SET system_key = NULL, protected = 0 WHERE id = ?1")
        .bind(inserted_id)
        .execute(pool)
        .await?;
    detail.summary.system_key = None;
    detail.summary.protected = false;
    Ok(detail)
}

pub(crate) async fn insert_legacy_custom_tag(
    pool: &SqlitePool,
    name: &str,
    rule: &TagRoutingRule,
) -> i64 {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query_scalar::<_, i64>(
        r#"
            INSERT INTO pool_tags (
                name, allow_cut_out, allow_cut_in, priority_tier, fast_mode_rewrite_mode,
                concurrency_limit, upstream_429_retry_enabled, upstream_429_max_retries,
                available_models_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
            RETURNING id
            "#,
    )
    .bind(name)
    .bind(if rule.allow_cut_out { 1 } else { 0 })
    .bind(if rule.allow_cut_in { 1 } else { 0 })
    .bind(rule.priority_tier.as_str())
    .bind(rule.fast_mode_rewrite_mode.as_str())
    .bind(rule.concurrency_limit)
    .bind(if rule.upstream_429_retry_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(i64::from(rule.upstream_429_max_retries))
    .bind(encode_string_array_json(&rule.available_models).expect("encode legacy custom tag"))
    .bind(&now_iso)
    .fetch_one(pool)
    .await
    .expect("insert legacy custom tag")
}

#[tokio::test]
pub(crate) async fn find_existing_import_match_loads_compact_support_fields() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Compact support OAuth").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET compact_support_status = ?2,
                compact_support_observed_at = ?3,
                compact_support_reason = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("supported")
    .bind("2026-05-24T00:00:00Z")
    .bind("validation probe")
    .execute(&pool)
    .await
    .expect("seed compact support fields");

    let row = find_existing_import_match(&pool, Some("user_test"), "org_test", "oauth@example.com")
        .await
        .expect("find existing import match")
        .expect("existing OAuth account should match");

    assert_eq!(row.id, account_id);
    assert_eq!(row.compact_support_status.as_deref(), Some("supported"));
    assert_eq!(
        row.compact_support_observed_at.as_deref(),
        Some("2026-05-24T00:00:00Z")
    );
    assert_eq!(
        row.compact_support_reason.as_deref(),
        Some("validation probe")
    );
}

#[tokio::test]
pub(crate) async fn find_existing_import_match_prefers_chatgpt_user_id_for_shared_account_ids() {
    let pool = test_pool().await;
    let first_id = insert_oauth_account(&pool, "Shared Org First").await;
    let second_id = insert_oauth_account(&pool, "Shared Org Second").await;

    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET email = ?2,
                chatgpt_account_id = ?3,
                chatgpt_user_id = ?4
            WHERE id = ?1
            "#,
    )
    .bind(first_id)
    .bind("member-one@example.com")
    .bind("org_shared_k12")
    .bind("user_member_one")
    .execute(&pool)
    .await
    .expect("update first shared-org row");
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET email = ?2,
                chatgpt_account_id = ?3,
                chatgpt_user_id = ?4
            WHERE id = ?1
            "#,
    )
    .bind(second_id)
    .bind("member-two@example.com")
    .bind("org_shared_k12")
    .bind("user_member_two")
    .execute(&pool)
    .await
    .expect("update second shared-org row");

    let row = find_existing_import_match(
        &pool,
        Some("user_member_two"),
        "org_shared_k12",
        "member-one@example.com",
    )
    .await
    .expect("find existing import match by user id")
    .expect("shared-org member should match");

    assert_eq!(row.id, second_id);
    assert_eq!(row.email.as_deref(), Some("member-two@example.com"));
}

#[tokio::test]
pub(crate) async fn find_existing_import_match_falls_back_to_account_id_only_for_legacy_missing_user_id()
 {
    let pool = test_pool().await;
    let legacy_id = insert_oauth_account(&pool, "Legacy Shared Org").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET email = ?2,
                chatgpt_account_id = ?3,
                chatgpt_user_id = NULL
            WHERE id = ?1
            "#,
    )
    .bind(legacy_id)
    .bind("legacy@example.com")
    .bind("legacy_shared_org")
    .execute(&pool)
    .await
    .expect("update legacy row");

    let row = find_existing_import_match(&pool, None, "legacy_shared_org", "different@example.com")
        .await
        .expect("find legacy import match")
        .expect("legacy account should match by account id");

    assert_eq!(row.id, legacy_id);
    assert_eq!(row.chatgpt_user_id.as_deref(), None);
}

pub(crate) async fn insert_syncable_oauth_account(
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
            refresh_token: Some("refresh-token".to_string()),
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

pub(crate) async fn insert_test_pool_api_key_account_with_options(
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
        "upstreamBaseUrl": upstream_base_url,
    }))
    .expect("deserialize api key account request");
    let Json(_) = create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect("insert test pool api key account");
    let account_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM pool_upstream_accounts WHERE display_name = ?1",
    )
    .bind(display_name)
    .fetch_one(&state.pool)
    .await
    .expect("load inserted test pool api key account id");
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind(normalized_group_name)
        .execute(&state.pool)
        .await
        .expect("restore legacy API key group fixture");
    account_id
}

pub(crate) async fn spawn_usage_snapshot_server(
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
pub(crate) struct SequencedOauthSyncServerState {
    usage_responses: Arc<Mutex<std::collections::VecDeque<(StatusCode, String)>>>,
    usage_requests: Arc<AtomicUsize>,
    token_requests: Arc<AtomicUsize>,
    token_response: Arc<String>,
}

pub(crate) async fn spawn_sequenced_oauth_sync_server(
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
pub(crate) struct ProxyOnlyOauthSyncServerState {
    usage_requests: Arc<AtomicUsize>,
    token_requests: Arc<AtomicUsize>,
}

pub(crate) async fn spawn_proxy_only_oauth_sync_server()
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
pub(crate) struct TokenFailureOauthServerState {
    token_status: StatusCode,
    token_body: Arc<String>,
    token_requests: Arc<AtomicUsize>,
}

pub(crate) async fn spawn_token_failure_oauth_server(
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
pub(crate) struct BlockingUsageServerState {
    started: Arc<AtomicBool>,
    release: Arc<Notify>,
    requests: Arc<AtomicUsize>,
}

pub(crate) async fn spawn_blocking_usage_server() -> (
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

pub(crate) async fn wait_for_atomic_true(flag: &AtomicBool) {
    timeout(Duration::from_secs(8), async {
        while !flag.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("flag should become true");
}

pub(crate) async fn wait_for_atomic_usize(flag: &AtomicUsize, expected: usize) {
    timeout(Duration::from_secs(8), async {
        while flag.load(Ordering::SeqCst) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("counter should reach expected value");
}

#[tokio::test]
pub(crate) async fn maintenance_pass_dispatches_without_waiting_for_sync_completion() {
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

use super::*;
