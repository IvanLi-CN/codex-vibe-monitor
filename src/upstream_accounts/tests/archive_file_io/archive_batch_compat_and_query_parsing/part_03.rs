use super::*;
use anyhow::anyhow;
use axum::http::header;
use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use serde_json::json;

type KaisouMailEmail = (String, String, Option<String>);

macro_rules! test_upstream_account_row {
    ($kind:expr, $upstream_base_url:expr) => {
        UpstreamAccountRow {
            id: 1,
            kind: $kind.to_string(),
            provider: UPSTREAM_ACCOUNT_PROVIDER_CODEX.to_string(),
            display_name: "Test".to_string(),
            group_name: None,
            bound_proxy_keys_json: None,
            model_mappings_json: None,
            is_mother: 0,
            note: None,
            status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
            enabled: 1,
            external_client_id: None,
            external_source_account_id: None,
            email: None,
            verified_email: None,
            chatgpt_account_id: None,
            chatgpt_user_id: None,
            plan_type: None,
            plan_type_observed_at: None,
            masked_api_key: None,
            encrypted_credentials: None,
            has_refresh_token: Some(1),
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
            policy_responses_first_byte_timeout_secs: None,
            policy_compact_first_byte_timeout_secs: None,
            policy_image_first_byte_timeout_secs: None,
            policy_responses_stream_timeout_secs: None,
            policy_compact_stream_timeout_secs: None,
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
            response_endpoint_capability: None,
            response_endpoint_capability_observed_at: None,
            response_endpoint_capability_reason: None,
            policy_response_endpoint_capability_override: None,
            chat_completions_capability: None,
            chat_completions_capability_observed_at: None,
            chat_completions_capability_reason: None,
            policy_chat_completions_capability_override: None,
            image_endpoint_capability: None,
            image_endpoint_capability_observed_at: None,
            image_endpoint_capability_reason: None,
            policy_image_endpoint_capability_override: None,
            response_image_tool_capability: None,
            response_image_tool_capability_observed_at: None,
            response_image_tool_capability_reason: None,
            policy_response_image_tool_capability_override: None,
            codex_imagegen_capability: None,
            codex_imagegen_capability_observed_at: None,
            codex_imagegen_capability_reason: None,
            policy_codex_imagegen_capability_override: None,
            standalone_search_capability: None,
            standalone_search_capability_observed_at: None,
            standalone_search_capability_reason: None,
            policy_standalone_search_capability_override: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            policy_allow_cut_out: None,
            policy_allow_cut_in: None,
            policy_priority_tier: None,
            policy_fast_mode_rewrite_mode: None,
            policy_image_tool_rewrite_mode: None,
            policy_codex_imagegen_rewrite_mode: None,
            policy_request_compression_algorithm: None,
            policy_concurrency_limit: None,
            policy_upstream_429_retry_enabled: None,
            policy_upstream_429_max_retries: None,
            policy_available_models_json: None,
            policy_available_models_mode: None,
            policy_status_change_upstream_http_401: None,
            policy_status_change_upstream_http_402: None,
            policy_status_change_upstream_http_403: None,
            policy_status_change_reauth_required: None,
            policy_status_change_upstream_http_429_rate_limit: None,
            policy_status_change_upstream_http_429_quota_exhausted: None,
            policy_status_change_usage_snapshot_exhausted: None,
            policy_status_change_quota_still_exhausted: None,
            policy_status_change_transport_failure: None,
            policy_status_change_upstream_server_overloaded: None,
            policy_status_change_upstream_http_5xx: None,
            upstream_base_url: $upstream_base_url.map(str::to_string),
            created_at: "2026-03-15T00:00:00Z".to_string(),
            updated_at: "2026-03-15T00:00:00Z".to_string(),
        }
    };
}

#[tokio::test]
pub(crate) async fn finish_bulk_sync_job_cancelled_exposes_cancelled_status_in_events_and_response()
{
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
pub(crate) fn imported_snapshot_is_exhausted_when_any_limit_is_full_or_credits_are_empty() {
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
pub(crate) async fn resolve_pool_account_upstream_base_url_only_overrides_api_key_accounts() {
    let _upstream_lock = crate::oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;
    crate::oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;

    let global = Url::parse("https://api.openai.com/").expect("global upstream base url");
    let override_url = "https://proxy.example.com/gateway";
    crate::oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse("https://chatgpt.com/backend-api/codex").expect("oauth codex base"),
    )
    .await;

    let oauth_row =
        test_upstream_account_row!(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX, Some(override_url));
    let oauth_resolved = resolve_pool_account_upstream_base_url(&oauth_row, &global)
        .expect("resolve oauth upstream base url");
    assert_eq!(
        oauth_resolved.as_str(),
        "https://chatgpt.com/backend-api/codex"
    );

    let api_key_row =
        test_upstream_account_row!(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX, Some(override_url));
    let api_key_resolved = resolve_pool_account_upstream_base_url(&api_key_row, &global)
        .expect("resolve api key upstream base url");
    assert_eq!(
        api_key_resolved.as_str(),
        "https://proxy.example.com/gateway"
    );
}

#[test]
pub(crate) fn parse_chatgpt_jwt_claims_extracts_identity_fields() {
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
pub(crate) fn build_usage_endpoint_url_preserves_backend_api_prefix() {
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
pub(crate) fn normalize_usage_snapshot_reads_windows_and_resets() {
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
        openai_proxy_websocket_enabled: DEFAULT_OPENAI_PROXY_WEBSOCKET_ENABLED,
        openai_proxy_upstream_websocket_default_enabled:
            DEFAULT_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED,
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
        list_limit_max: 100,
        user_agent: user_agent.to_string(),
        static_dir: None,
        public_origin: None,
        retention_enabled: DEFAULT_RETENTION_ENABLED,
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
pub(crate) async fn fetch_usage_snapshot_retries_with_browser_user_agent() {
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
pub(crate) async fn fetch_usage_snapshot_skips_browser_user_agent_retry_for_upstream_rejected_402()
{
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
pub(crate) async fn fetch_usage_snapshot_retries_browser_user_agent_for_generic_403_upstream_rejected_text()
 {
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
pub(crate) async fn fetch_usage_snapshot_retries_browser_user_agent_for_generic_402_pages() {
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
pub(crate) async fn fetch_usage_snapshot_retries_browser_user_agent_for_wrapped_upstream_auth_error()
 {
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
pub(crate) async fn fetch_usage_snapshot_preserves_terminal_browser_retry_402_for_classification() {
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
pub(crate) fn build_manual_callback_redirect_uri_targets_localhost() {
    let redirect = build_manual_callback_redirect_uri().expect("redirect uri");
    assert!(redirect.starts_with("http://localhost:"));
    assert!(redirect.ends_with("/auth/callback"));
}

#[test]
pub(crate) fn parse_manual_oauth_callback_accepts_expected_redirect() {
    let query = parse_manual_oauth_callback(
        "http://localhost:37891/auth/callback?code=test-code&state=test-state",
        "http://localhost:37891/auth/callback",
    )
    .expect("callback query");
    assert_eq!(query.code.as_deref(), Some("test-code"));
    assert_eq!(query.state.as_deref(), Some("test-state"));
}

#[test]
pub(crate) fn build_oauth_authorize_url_requests_official_scopes_and_audience() {
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
pub(crate) fn is_reauth_error_requires_explicit_invalidated_signal() {
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

pub(crate) async fn test_pool() -> SqlitePool {
    if std::env::var_os(crate::tests::STATEFUL_SCHEMA_TEMPLATE_PATH_ENV).is_some() {
        return crate::tests::test_current_schema_pool().await;
    }

    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    crate::ensure_schema(&pool).await.expect("ensure schema");
    pool
}

pub(crate) fn test_required_group_bound_proxy_keys() -> Vec<String> {
    vec![FORWARD_PROXY_DIRECT_KEY.to_string()]
}

pub(crate) fn test_required_group_name() -> &'static str {
    "test-direct-group"
}

pub(crate) async fn upsert_test_group_binding(
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

pub(crate) async fn ensure_test_group_binding(pool: &SqlitePool, group_name: &str) {
    upsert_test_group_binding(pool, group_name, test_required_group_bound_proxy_keys()).await;
}

pub(crate) async fn set_test_account_group_name(
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

pub(crate) async fn set_test_account_token_expires_at(
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

pub(crate) async fn test_app_state_with_usage_base(base_url: &str) -> Arc<AppState> {
    test_app_state_with_usage_base_and_parallelism(
        base_url,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await
}

pub(crate) async fn test_app_state_with_usage_base_and_parallelism(
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

pub(crate) async fn test_app_state_with_usage_and_oauth_base(
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

pub(crate) async fn test_app_state_with_upstream_endpoints_and_parallelism(
    usage_base_url: &str,
    oauth_issuer: &str,
    user_agent: &str,
    maintenance_parallelism: usize,
) -> Arc<AppState> {
    let mut config = usage_snapshot_test_config(usage_base_url, user_agent);
    config.upstream_accounts_oauth_issuer = Url::parse(oauth_issuer).expect("valid oauth issuer");
    test_app_state_with_config_and_parallelism(config, maintenance_parallelism).await
}

pub(crate) async fn test_app_state_with_config_and_parallelism(
    config: AppConfig,
    maintenance_parallelism: usize,
) -> Arc<AppState> {
    let http_clients = HttpClients::build(&config).expect("build http clients");
    let (broadcaster, _) = broadcast::channel(8);
    let proxy_raw_async_writer_limit = proxy_raw_async_writer_limit(&config);
    let pool = test_pool().await;
    Arc::new(AppState {
        config,
        sqlite_batch_writer: SqliteBatchWriter::spawn_for_test(),
        pool_account_selection_runtime: Arc::new(PoolAccountSelectionRuntime::default()),
        proxy_runtime_invocations: Arc::new(ProxyRuntimeInvocationStore::default()),
        pool,
        oauth_installation_seed: [0_u8; 32],
        http_clients,
        broadcaster,
        subscription_hub: Arc::new(crate::SubscriptionHub::new()),
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache { quota: None })),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        dashboard_activity_live_broadcast_seq: Arc::new(AtomicU64::new(0)),
        dashboard_activity_live_broadcast_running: Arc::new(AtomicBool::new(false)),
        process_started_at_utc: chrono::Utc::now(),
        dashboard_network_speed_cache: Arc::new(
            crate::dashboard_network_speed::DashboardNetworkSpeedCache::new(chrono::Utc::now()),
        ),
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
        prompt_cache_conversation_cache: Arc::new(Mutex::new(PromptCacheConversationsCacheState {
            entries: HashMap::new(),
            in_flight: HashMap::new(),
            generation: 0,
        })),
        dashboard_activity_snapshot_cache: Arc::new(Mutex::new(
            DashboardActivitySnapshotCacheState::default(),
        )),
        terminal_projection_hub: Arc::new(crate::TerminalProjectionHub::default()),
        long_term_projection_runtime: Arc::new(Mutex::new(
            crate::LongTermProjectionRuntime::default(),
        )),
        memory_diagnostics: Arc::new(crate::MemoryDiagnosticsRuntime::default()),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        system_status_cache: Arc::new(Mutex::new(SystemStatusCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_availability: PoolRoutingAvailabilitySignal::default(),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        pool_routing_test_data_version_connection: Arc::new(Mutex::new(None)),
        pool_model_routing_cache_write_lock: Arc::new(Mutex::new(())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        fallback_proxy_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        upstream_accounts: Arc::new(
            UpstreamAccountsRuntime::test_instance_with_maintenance_parallelism(
                maintenance_parallelism,
            ),
        ),
    })
}

pub(crate) async fn ensure_window_actual_usage_test_tables(pool: &SqlitePool) {
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

pub(crate) fn shanghai_local_iso(timestamp: DateTime<Utc>) -> String {
    format_naive(timestamp.with_timezone(&Shanghai).naive_local())
}

pub(crate) struct WindowActualUsageInvocation<'a> {
    pub(crate) account_id: i64,
    pub(crate) occurred_at: &'a str,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
}

pub(crate) async fn insert_window_actual_usage_invocation(
    pool: &SqlitePool,
    invocation: WindowActualUsageInvocation<'_>,
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
    .bind(invocation.occurred_at)
    .bind(invocation.input_tokens)
    .bind(invocation.output_tokens)
    .bind(invocation.cache_input_tokens)
    .bind(invocation.total_tokens)
    .bind(invocation.cost)
    .bind(json!({ "upstreamAccountId": invocation.account_id }).to_string())
    .execute(pool)
    .await
    .expect("insert codex_invocations row");
}

type UsageArchiveRow = (
    i64,
    String,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<f64>,
);

pub(crate) async fn seed_window_actual_usage_archive_batch(
    pool: &SqlitePool,
    archive_dir: &Path,
    batch_name: &str,
    rows: &[UsageArchiveRow],
) -> PathBuf {
    let (archive_db_path, archive_gzip_path) = create_usage_archive_files(archive_dir, batch_name);
    populate_usage_archive_file(&archive_db_path, rows).await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_gzip_path)
        .expect("compress archive sqlite");
    insert_usage_archive_batch_manifest(pool, &archive_gzip_path, rows).await;
    archive_gzip_path
}

fn create_usage_archive_files(archive_dir: &Path, batch_name: &str) -> (PathBuf, PathBuf) {
    std::fs::create_dir_all(archive_dir).expect("create archive dir");
    let archive_db_path = archive_dir.join(format!("{batch_name}.sqlite"));
    let archive_gzip_path = archive_dir.join(format!("{batch_name}.sqlite.gz"));
    let _ = std::fs::remove_file(&archive_db_path);
    let _ = std::fs::remove_file(&archive_gzip_path);
    std::fs::File::create(&archive_db_path).expect("create archive sqlite");
    (archive_db_path, archive_gzip_path)
}

async fn populate_usage_archive_file(archive_db_path: &Path, rows: &[UsageArchiveRow]) {
    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(archive_db_path))
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
}

async fn insert_usage_archive_batch_manifest(
    pool: &SqlitePool,
    archive_gzip_path: &Path,
    rows: &[UsageArchiveRow],
) {
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
                sha256,
                row_count,
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
                ?7,
                ?8,
                ?9
            )
            "#,
    )
    .bind(month_key)
    .bind(day_key)
    .bind(archive_gzip_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(archive_gzip_path).expect("archive sha256"))
    .bind(rows.len() as i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(coverage_start_at)
    .bind(coverage_end_at)
    .bind(coverage_end_at)
    .execute(pool)
    .await
    .expect("insert archive batch manifest");
}

pub(crate) fn assert_cost_close(actual: f64, expected: f64) {
    let diff = (actual - expected).abs();
    assert!(
        diff < 1e-9,
        "expected {expected}, got {actual}, diff={diff}"
    );
}

#[derive(Clone)]
pub(crate) struct KaisouMailStubState {
    pub(crate) domains: Vec<String>,
    pub(crate) emails: Arc<Mutex<Vec<KaisouMailEmail>>>,
    pub(crate) create_requests: Arc<Mutex<Vec<Value>>>,
    pub(crate) generated_requests: Arc<Mutex<Vec<(String, String)>>>,
    pub(crate) deleted_ids: Arc<Mutex<Vec<String>>>,
    pub(crate) next_generated_id: Arc<AtomicUsize>,
}

pub(crate) struct KaisouMailTestHarness {
    pub(crate) state: Arc<AppState>,
    pub(crate) stub: KaisouMailStubState,
    pub(crate) server: tokio::task::JoinHandle<()>,
}

impl KaisouMailTestHarness {
    pub(crate) fn abort(self) {
        self.server.abort();
    }
}
