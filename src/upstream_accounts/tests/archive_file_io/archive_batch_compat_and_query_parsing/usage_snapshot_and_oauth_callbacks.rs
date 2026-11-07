use super::*;

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

pub(crate) fn usage_snapshot_test_config(base_url: &str, user_agent: &str) -> AppConfig {
    AppConfig {
        openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
        database_path: PathBuf::from(":memory:"),
        analytics_backend: AnalyticsBackend::Sqlite,
        admin_auth: None,
        postgres_database_url: None,
        postgres_migration_url: None,
        postgres_analytics_maintenance_url: None,
        postgres_max_connections: DEFAULT_POSTGRES_MAX_CONNECTIONS as u32,
        observability: ObservabilityConfig::default(),
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
        openai_proxy_image_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_IMAGE_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_request_read_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS,
        ),
        openai_proxy_max_request_body_bytes: DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        openai_proxy_encrypted_session_owner_routing_enabled:
            DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
        proxy_enforce_stream_include_usage: DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
        proxy_usage_backfill_on_startup: DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
        proxy_raw_max_bytes: DEFAULT_PROXY_RAW_MAX_BYTES,
        proxy_raw_dir: crate::tests::test_runtime_path("proxy-raw-tests"),
        proxy_raw_compression: DEFAULT_PROXY_RAW_COMPRESSION,
        proxy_raw_immediate_gzip_bytes: DEFAULT_PROXY_RAW_IMMEDIATE_GZIP_BYTES,
        proxy_raw_hot_secs: DEFAULT_PROXY_RAW_HOT_SECS,
        xray_binary: DEFAULT_XRAY_BINARY.to_string(),
        xray_runtime_dir: crate::tests::test_runtime_path("xray-forward-tests"),
        forward_proxy_algo: ForwardProxyAlgo::V1,
        max_parallel_polls: 2,
        shared_connection_parallelism: 1,
        http_bind: "127.0.0.1:0".parse().expect("valid socket address"),
        cors_allowed_origins: Vec::new(),
        public_metrics_cors_allowed_origins: vec![
            "https://ivanli.cc".to_string(),
            "http://127.0.0.1:12620".to_string(),
        ],
        list_limit_max: 100,
        user_agent: user_agent.to_string(),
        static_dir: None,
        public_origin: None,
        retention_dry_run: DEFAULT_RETENTION_DRY_RUN,
        retention_interval: Duration::from_secs(DEFAULT_RETENTION_INTERVAL_SECS),
        retention_batch_rows: DEFAULT_RETENTION_BATCH_ROWS,
        retention_catchup_budget: Duration::from_secs(DEFAULT_RETENTION_CATCHUP_BUDGET_SECS),
        archive_dir: crate::tests::test_runtime_path("archive-tests"),
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
        quota_snapshot_full_days: DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
        long_term_stats_hourly_retention_days: DEFAULT_LONG_TERM_STATS_HOURLY_RETENTION_DAYS,
        upstream_accounts_oauth_client_id: DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID.to_string(),
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
        upstream_accounts_history_retention_days: DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
        upstream_accounts_kaisoumail: None,
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

#[tokio::test]
async fn fetch_usage_snapshot_skips_browser_user_agent_retry_for_upstream_rejected_402() {
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
        state.requests.lock().await.push(user_agent);
        (
            StatusCode::PAYMENT_REQUIRED,
            json!({ "detail": { "code": "deactivated_workspace" } }).to_string(),
        )
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

    let err = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
        .await
        .expect_err("402 upstream rejected should stay terminal");
    assert!(
        err.to_string().contains("402 Payment Required"),
        "expected original 402 error, got: {err:#}"
    );

    let recorded = requests.lock().await.clone();
    assert_eq!(recorded, vec!["codex-vibe-monitor/0.2.0".to_string()]);

    server.abort();
}

#[tokio::test]
async fn fetch_usage_snapshot_retries_browser_user_agent_for_generic_403_upstream_rejected_text() {
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
        let is_browser_user_agent = user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT;
        state.requests.lock().await.push(user_agent);
        if is_browser_user_agent {
            (
                StatusCode::OK,
                json!({
                    "planType": "pro",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 9,
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
                "usage endpoint returned 403 Forbidden: upstream rejected request by policy"
                    .to_string(),
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
        .expect("generic 403 upstream-rejected text should still retry browser user agent");
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

#[tokio::test]
async fn fetch_usage_snapshot_retries_browser_user_agent_for_generic_402_pages() {
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
                            "usedPercent": 9,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        }
                    }
                })
                .to_string(),
            )
        } else {
            (StatusCode::PAYMENT_REQUIRED, "Payment Required".to_string())
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
        .expect("generic 402 should retry with browser user agent");
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

#[tokio::test]
async fn fetch_usage_snapshot_retries_browser_user_agent_for_wrapped_upstream_auth_error() {
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
        let is_browser_user_agent = user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT;
        state.requests.lock().await.push(user_agent);
        if is_browser_user_agent {
            (
                StatusCode::OK,
                json!({
                    "planType": "pro",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 9,
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
                "oauth_upstream_rejected_request: pool upstream responded with 403: Forbidden"
                    .to_string(),
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
        .expect("wrapped upstream auth error should still retry browser user agent");
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

#[tokio::test]
async fn fetch_usage_snapshot_preserves_terminal_browser_retry_402_for_classification() {
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
        let is_browser_user_agent = user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT;
        state.requests.lock().await.push(user_agent);
        if is_browser_user_agent {
            (
                StatusCode::PAYMENT_REQUIRED,
                json!({ "detail": { "code": "deactivated_workspace" } }).to_string(),
            )
        } else {
            (
                StatusCode::BAD_GATEWAY,
                "upstream usage endpoint temporary gateway failure".to_string(),
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

    let err = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
        .await
        .expect_err("terminal browser retry 402 should surface for sync classification");
    let err_text = err.to_string();
    assert!(
        err_text.contains("browser user agent retry failed"),
        "expected retry context in surfaced error, got: {err:#}"
    );
    assert!(
        err_text.contains("402 Payment Required"),
        "expected terminal browser retry 402 to survive to_string(), got: {err:#}"
    );
    assert!(
        err_text.contains("deactivated_workspace"),
        "expected terminal browser retry detail to survive to_string(), got: {err:#}"
    );

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
