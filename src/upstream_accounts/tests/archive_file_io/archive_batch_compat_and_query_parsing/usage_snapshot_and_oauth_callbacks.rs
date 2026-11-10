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
        request_timeout: Duration::from_secs(5),
        user_agent: user_agent.to_string(),
        upstream_accounts_usage_base_url: Url::parse(base_url).expect("valid usage base url"),
        ..crate::tests::test_config()
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
