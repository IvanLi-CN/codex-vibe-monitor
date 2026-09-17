use super::*;
use anyhow::anyhow;
use serde::Deserialize;
use serde_json::json;

fn mailbox_parts_for_test(address: &str) -> (String, String) {
    let (local, domain) = address.split_once('@').expect("test mailbox address");
    (local.to_string(), domain.to_string())
}

async fn meta_handler(State(state): State<KaisouMailStubState>) -> axum::Json<serde_json::Value> {
    axum::Json(json!({
        "domains": state.domains,
        "supportsUnlimitedMailboxTtl": true,
        "defaultMailboxTtlMinutes": 60,
        "minMailboxTtlMinutes": 60,
        "maxMailboxTtlMinutes": 525600,
    }))
}

async fn list_mailboxes_handler(
    State(state): State<KaisouMailStubState>,
) -> axum::Json<serde_json::Value> {
    let emails = state.emails.lock().await.clone();
    axum::Json(json!({
        "mailboxes": emails.into_iter().map(|(id, address, expires_at)| json!({
            "id": id,
            "address": address,
            "expiresAt": expires_at,
        })).collect::<Vec<_>>(),
    }))
}

async fn create_mailbox_handler(
    State(state): State<KaisouMailStubState>,
    axum::Json(payload): axum::Json<Value>,
) -> axum::Json<serde_json::Value> {
    let index = state
        .next_generated_id
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;
    state.create_requests.lock().await.push(payload);
    let email = format!("upstream-generated-{index}@mailbox.kaisoumail.test");
    let id = format!("generated_{index}");
    state
        .emails
        .lock()
        .await
        .push((id.clone(), email.clone(), None));
    axum::Json(json!({ "id": id, "address": email, "expiresAt": null }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnsureMailboxRequest {
    address: String,
}

async fn ensure_mailbox_handler(
    State(state): State<KaisouMailStubState>,
    axum::Json(payload): axum::Json<EnsureMailboxRequest>,
) -> axum::Json<serde_json::Value> {
    let mut emails = state.emails.lock().await;
    let existing_index = emails
        .iter()
        .position(|(_, address, _)| address == &payload.address);
    let (id, address, expires_at) = match existing_index {
        Some(index) => {
            let id = emails[index].0.clone();
            let address = emails[index].1.clone();
            let expires_at = Some("2026-06-01T00:00:00.000Z".to_string());
            emails[index].2 = expires_at.clone();
            (id, address, expires_at)
        }
        None => {
            let index = state
                .next_generated_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            let id = format!("generated_{index}");
            state
                .generated_requests
                .lock()
                .await
                .push(mailbox_parts_for_test(&payload.address));
            emails.push((id.clone(), payload.address.clone(), None));
            (id, payload.address, None)
        }
    };
    axum::Json(json!({ "id": id, "address": address, "expiresAt": expires_at }))
}

async fn messages_handler() -> axum::Json<serde_json::Value> {
    axum::Json(json!({ "messages": [] }))
}

async fn delete_mailbox_handler(
    State(state): State<KaisouMailStubState>,
    axum::extract::Path(email_id): axum::extract::Path<String>,
) -> axum::http::StatusCode {
    state.deleted_ids.lock().await.push(email_id.clone());
    state
        .emails
        .lock()
        .await
        .retain(|(existing_id, _, _)| existing_id != &email_id);
    axum::http::StatusCode::NO_CONTENT
}

fn build_kaisoumail_app_state(
    config: AppConfig,
    pool: SqlitePool,
    http_clients: HttpClients,
    broadcaster: broadcast::Sender<BroadcastPayload>,
    proxy_raw_async_writer_limit: usize,
) -> Arc<AppState> {
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
        #[cfg(test)]
        pool_routing_test_data_version_connection: Arc::new(Mutex::new(None)),
        pool_model_routing_cache_write_lock: Arc::new(Mutex::new(())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        fallback_proxy_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    })
}

pub(crate) async fn spawn_kaisoumail_test_harness(
    domains: &str,
    emails: Vec<(String, String, Option<String>)>,
) -> KaisouMailTestHarness {
    let stub = KaisouMailStubState {
        domains: domains
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.trim_start_matches('@').to_ascii_lowercase())
            .collect(),
        emails: Arc::new(Mutex::new(emails)),
        create_requests: Arc::new(Mutex::new(Vec::new())),
        generated_requests: Arc::new(Mutex::new(Vec::new())),
        deleted_ids: Arc::new(Mutex::new(Vec::new())),
        next_generated_id: Arc::new(AtomicUsize::new(0)),
    };
    let app = Router::new()
        .route("/api/meta", get(meta_handler))
        .route(
            "/api/mailboxes",
            get(list_mailboxes_handler).post(create_mailbox_handler),
        )
        .route("/api/mailboxes/ensure", post(ensure_mailbox_handler))
        .route("/api/mailboxes/:email_id", delete(delete_mailbox_handler))
        .route("/api/messages", get(messages_handler))
        .with_state(stub.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind kaisoumail test listener");
    let addr = listener.local_addr().expect("kaisoumail listener addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve kaisoumail test app");
    });

    let mut config =
        usage_snapshot_test_config("https://chatgpt.com/backend-api", "codex-vibe-monitor/test");
    config.upstream_accounts_kaisoumail = Some(UpstreamAccountsKaisouMailConfig {
        base_url: Url::parse(&format!("http://{addr}")).expect("valid kaisoumail test url"),
        api_key: "test-kaisoumail-key".to_string(),
    });
    let http_clients = HttpClients::build(&config).expect("build http clients");
    let (broadcaster, _) = broadcast::channel(8);
    let proxy_raw_async_writer_limit = proxy_raw_async_writer_limit(&config);
    let pool = test_pool().await;
    let state = build_kaisoumail_app_state(
        config,
        pool,
        http_clients,
        broadcaster,
        proxy_raw_async_writer_limit,
    );

    KaisouMailTestHarness {
        state,
        stub,
        server,
    }
}

pub(crate) fn test_claims_with_plan_type(
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

pub(crate) fn test_claims(
    email: &str,
    chatgpt_account_id: Option<&str>,
    chatgpt_user_id: Option<&str>,
) -> ChatgptJwtClaims {
    test_claims_with_plan_type(email, chatgpt_account_id, chatgpt_user_id, Some("team"))
}

pub(crate) fn test_id_token(
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

pub(crate) fn test_jwt_token(payload: serde_json::Value) -> String {
    let encoded = URL_SAFE_NO_PAD.encode(b"{}");
    let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    format!("{encoded}.{body}.{encoded}")
}

pub(crate) fn test_tag_routing_rule() -> TagRoutingRule {
    TagRoutingRule {
        allow_cut_out: true,
        allow_cut_in: true,
        priority_tier: TagPriorityTier::Normal,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        concurrency_limit: 0,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        available_models: vec![],
    }
}

pub(crate) fn test_account_tag_summary(
    id: i64,
    name: &str,
    concurrency_limit: i64,
) -> AccountTagSummary {
    let mut routing_rule = test_tag_routing_rule();
    routing_rule.concurrency_limit = concurrency_limit;
    AccountTagSummary {
        id,
        name: name.to_string(),
        routing_rule,
        available_models_invalid: false,
        system_key: None,
        protected: false,
    }
}

pub(crate) fn test_effective_routing_rule(concurrency_limit: i64) -> EffectiveRoutingRule {
    EffectiveRoutingRule {
        allow_cut_out: true,
        allow_cut_in: true,
        priority_tier: TagPriorityTier::Normal,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        concurrency_limit,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        available_models: vec![],
        available_models_mode: AvailableModelsMode::Allowlist,
        available_models_defined: false,
        tag_available_models: None,
        status_change_reasons: default_status_change_reasons(),
        status_change_reason_field_sources: default_status_change_reason_field_sources("root"),
        system_denied_models: vec![],
        source_tag_ids: vec![],
        source_tag_names: vec![],
        field_sources: EffectiveRoutingRuleFieldSources {
            allow_cut_out: "root".to_string(),
            allow_cut_in: "root".to_string(),
            priority_tier: "root".to_string(),
            fast_mode_rewrite_mode: "root".to_string(),
            image_tool_rewrite_mode: "root".to_string(),
            codex_imagegen_rewrite_mode: "root".to_string(),
            request_compression_algorithm: "root".to_string(),
            concurrency_limit: "root".to_string(),
            upstream_429_retry: "root".to_string(),
            available_models: "root".to_string(),
            available_models_mode: "root".to_string(),
            system_denied_models: "root".to_string(),
        },
        timeouts: RoutingTimeoutSettings {
            responses_first_byte_timeout_secs: Some(120),
            compact_first_byte_timeout_secs: Some(300),
            image_first_byte_timeout_secs: Some(300),
            responses_stream_timeout_secs: Some(300),
            compact_stream_timeout_secs: Some(300),
        },
        timeout_field_sources: RoutingTimeoutFieldSources {
            responses_first_byte_timeout_secs: "root".to_string(),
            compact_first_byte_timeout_secs: "root".to_string(),
            image_first_byte_timeout_secs: "root".to_string(),
            responses_stream_timeout_secs: "root".to_string(),
            compact_stream_timeout_secs: "root".to_string(),
        },
    }
}

#[test]
pub(crate) fn request_capabilities_route_by_three_independent_axes() {
    assert!(account_accepts_request_capabilities(
        RequestCapabilityRequirements::from_endpoint_and_image_intent(
            "/v1/responses",
            ImageIntent::Yes
        ),
        CapabilitySupport::Unknown,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Supported,
        CapabilitySupport::Supported,
        CapabilitySupport::Unknown,
        CapabilitySupport::Unknown,
    ));
    assert!(!account_accepts_request_capabilities(
        RequestCapabilityRequirements::from_endpoint_and_image_intent(
            "/v1/responses",
            ImageIntent::Yes
        ),
        CapabilitySupport::Supported,
        CapabilitySupport::Supported,
        CapabilitySupport::Supported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unknown,
        CapabilitySupport::Unknown,
    ));
    assert!(account_accepts_request_capabilities(
        RequestCapabilityRequirements::from_endpoint_and_image_intent(
            "/v1/images/generations",
            ImageIntent::DirectImage,
        ),
        CapabilitySupport::Unsupported,
        CapabilitySupport::Supported,
        CapabilitySupport::Supported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unknown,
        CapabilitySupport::Unknown,
    ));
    assert!(!account_accepts_request_capabilities(
        RequestCapabilityRequirements::from_endpoint_and_image_intent(
            "/v1/images/generations",
            ImageIntent::DirectImage,
        ),
        CapabilitySupport::Supported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Supported,
        CapabilitySupport::Unknown,
        CapabilitySupport::Unknown,
    ));
    assert!(account_accepts_request_capabilities(
        RequestCapabilityRequirements::from_endpoint_and_image_intent(
            "/v1/chat/completions",
            ImageIntent::Unknown,
        ),
        CapabilitySupport::Unsupported,
        CapabilitySupport::Supported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unknown,
        CapabilitySupport::Unknown,
    ));
    assert!(!account_accepts_request_capabilities(
        RequestCapabilityRequirements::from_endpoint_and_image_intent(
            "/v1/chat/completions",
            ImageIntent::Unknown,
        ),
        CapabilitySupport::Supported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Supported,
        CapabilitySupport::Unknown,
        CapabilitySupport::Unknown,
    ));

    assert!(!account_accepts_request_capabilities(
        RequestCapabilityRequirements::from_endpoint_and_image_intent(
            "/v1/alpha/search",
            ImageIntent::Unknown,
        ),
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unknown,
        CapabilitySupport::Unsupported,
    ));
    assert!(account_accepts_request_capabilities(
        RequestCapabilityRequirements::from_endpoint_and_image_intent(
            "/v1/alpha/search/",
            ImageIntent::Unknown,
        ),
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unsupported,
        CapabilitySupport::Unknown,
        CapabilitySupport::Unsupported,
    ));
}

pub(crate) async fn insert_test_oauth_mailbox_session(
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

pub(crate) async fn insert_api_key_account(pool: &SqlitePool, display_name: &str) -> i64 {
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

#[tokio::test]
pub(crate) async fn account_attempt_list_reads_models_from_invocation_payload_without_schema_columns()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Attempt payload compatibility").await;
    let occurred_at = format_utc_iso(Utc::now());

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (invoke_id, occurred_at, source, model, payload, raw_response)
        VALUES (?1, ?2, 'proxy', 'gpt-5.4', ?3, '')
        "#,
    )
    .bind("attempt-payload-compat")
    .bind(&occurred_at)
    .bind(r#"{"requestModel":"gpt-5.3","responseModel":"gpt-5.4"}"#)
    .execute(&state.pool)
    .await
    .expect("insert invocation payload");

    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, upstream_account_id,
            attempt_index, distinct_account_index, same_account_retry_index,
            started_at, finished_at, status, phase, http_status, created_at
        )
        VALUES ('APAYLOAD', ?1, ?2, '/v1/responses', 'pool', ?3, 1, 1, 0, ?2, ?2, 'success', 'completed', 200, ?2)
        "#,
    )
    .bind("attempt-payload-compat")
    .bind(&occurred_at)
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert upstream attempt");

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
    .expect("list account attempts without request_model columns");

    let item = response.items.first().expect("one attempt");
    assert_eq!(item.model.as_deref(), Some("gpt-5.3"));
    assert_eq!(item.request_model.as_deref(), Some("gpt-5.3"));
    assert_eq!(item.response_model.as_deref(), Some("gpt-5.4"));
}

struct AttemptFilterFixture {
    attempt_id: &'static str,
    invoke_id: &'static str,
    account_id: i64,
    occurred_at: String,
    endpoint: &'static str,
    sticky_key: Option<&'static str>,
    model: &'static str,
    request_model: Option<&'static str>,
    response_model: Option<&'static str>,
    compaction_request_kind: Option<&'static str>,
    compaction_response_kind: Option<&'static str>,
    image_intent: Option<&'static str>,
}

fn attempt_filter_fixtures(
    account_id: i64,
    other_account_id: i64,
    now: chrono::DateTime<Utc>,
) -> Vec<AttemptFilterFixture> {
    vec![
        AttemptFilterFixture {
            attempt_id: "AFILT_REMOTE",
            invoke_id: "afilt_remote",
            account_id,
            occurred_at: format_utc_iso(now - ChronoDuration::minutes(1)),
            endpoint: "/v1/responses",
            sticky_key: Some("sticky-remote"),
            model: "gpt-5.5",
            request_model: Some("gpt-5.5"),
            response_model: Some("gpt-5.5-2026-07-01"),
            compaction_request_kind: Some("remote_v2"),
            compaction_response_kind: Some("remote_v2"),
            image_intent: Some("no"),
        },
        AttemptFilterFixture {
            attempt_id: "AFILT_REQUEST_ONLY_REMOTE",
            invoke_id: "afilt_request_only_remote",
            account_id,
            occurred_at: format_utc_iso(now - ChronoDuration::seconds(90)),
            endpoint: "/v1/responses",
            sticky_key: None,
            model: "gpt-5.7",
            request_model: Some("gpt-5.7"),
            response_model: Some("gpt-5.7"),
            compaction_request_kind: Some("remote_v2"),
            compaction_response_kind: None,
            image_intent: Some("no"),
        },
        AttemptFilterFixture {
            attempt_id: "AFILT_NORMAL",
            invoke_id: "afilt_normal",
            account_id,
            occurred_at: format_utc_iso(now - ChronoDuration::minutes(2)),
            endpoint: "/v1/responses",
            sticky_key: None,
            model: "gpt-5.4",
            request_model: Some("gpt-5.4"),
            response_model: Some("gpt-5.6"),
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: Some("no"),
        },
        AttemptFilterFixture {
            attempt_id: "AFILT_COMPACT",
            invoke_id: "afilt_compact",
            account_id,
            occurred_at: format_utc_iso(now - ChronoDuration::minutes(3)),
            endpoint: "/v1/responses/compact",
            sticky_key: Some("sticky-compact"),
            model: "gpt-5-compact",
            request_model: Some("gpt-5-compact-request"),
            response_model: None,
            compaction_request_kind: Some("compact"),
            compaction_response_kind: Some("compact"),
            image_intent: Some("no"),
        },
        AttemptFilterFixture {
            attempt_id: "AFILT_IMAGE",
            invoke_id: "afilt_image",
            account_id,
            occurred_at: format_utc_iso(now - ChronoDuration::minutes(4)),
            endpoint: "/v1/images/edits",
            sticky_key: Some("sticky-image"),
            model: "gpt-image-1",
            request_model: Some("gpt-image-1"),
            response_model: Some("gpt-image-1"),
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: Some("direct_image"),
        },
        AttemptFilterFixture {
            attempt_id: "AFILT_OTHER_IMAGE",
            invoke_id: "afilt_other_image",
            account_id: other_account_id,
            occurred_at: format_utc_iso(now - ChronoDuration::seconds(30)),
            endpoint: "/v1/images/generations",
            sticky_key: Some("sticky-other"),
            model: "gpt-image-1",
            request_model: Some("gpt-image-1"),
            response_model: Some("gpt-image-1"),
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: Some("yes"),
        },
    ]
}

async fn seed_attempt_filter_rows(state: &AppState, fixtures: &[AttemptFilterFixture]) {
    for fixture in fixtures {
        let payload = json!({
            "requestModel": fixture.request_model,
            "responseModel": fixture.response_model,
            "compactionRequestKind": fixture.compaction_request_kind,
            "compactionResponseKind": fixture.compaction_response_kind,
            "imageIntent": fixture.image_intent,
        })
        .to_string();

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (invoke_id, occurred_at, source, model, payload, raw_response)
            VALUES (?1, ?2, 'proxy', ?3, ?4, '')
            "#,
        )
        .bind(fixture.invoke_id)
        .bind(&fixture.occurred_at)
        .bind(fixture.model)
        .bind(&payload)
        .execute(&state.pool)
        .await
        .expect("insert invocation");

        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                attempt_public_id, invoke_id, occurred_at, endpoint, route_mode,
                sticky_key, upstream_account_id, attempt_index, distinct_account_index,
                same_account_retry_index, started_at, finished_at, status, phase,
                http_status, created_at
            )
            VALUES (
                ?1, ?2, ?3, ?4, 'pool',
                ?5, ?6, 1, 1,
                0, ?3, ?3, 'success', 'completed',
                200, ?3
            )
            "#,
        )
        .bind(fixture.attempt_id)
        .bind(fixture.invoke_id)
        .bind(&fixture.occurred_at)
        .bind(fixture.endpoint)
        .bind(fixture.sticky_key)
        .bind(fixture.account_id)
        .execute(&state.pool)
        .await
        .expect("insert account attempt");
    }
}

async fn seed_attempt_filter_noise(state: &AppState, account_id: i64, now: chrono::DateTime<Utc>) {
    for index in 0..17 {
        let invoke_id = format!("afilt_remote_noise_{index:02}");
        let attempt_id = format!("AFILTNOISE{index:02}");
        let occurred_at = format_utc_iso(now - ChronoDuration::seconds(70 + index as i64));
        let payload = json!({
            "requestModel": "gpt-5.5",
            "responseModel": "gpt-5.5-2026-07-01",
            "compactionRequestKind": "remote_v2",
            "compactionResponseKind": "remote_v2",
            "imageIntent": "no",
        })
        .to_string();

        sqlx::query(
            r#"
            INSERT INTO codex_invocations (invoke_id, occurred_at, source, model, payload, raw_response)
            VALUES (?1, ?2, 'proxy', 'gpt-5.5', ?3, '')
            "#,
        )
        .bind(&invoke_id)
        .bind(&occurred_at)
        .bind(&payload)
        .execute(&state.pool)
        .await
        .expect("insert remote_v2 noise invocation");

        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                attempt_public_id, invoke_id, occurred_at, endpoint, route_mode,
                sticky_key, upstream_account_id, attempt_index, distinct_account_index,
                same_account_retry_index, started_at, finished_at, status, phase,
                http_status, created_at
            )
            VALUES (
                ?1, ?2, ?3, '/v1/responses', 'pool',
                'sticky-remote', ?4, 1, 1,
                0, ?3, ?3, 'success', 'completed',
                200, ?3
            )
            "#,
        )
        .bind(&attempt_id)
        .bind(&invoke_id)
        .bind(&occurred_at)
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("insert remote_v2 noise attempt");
    }
}

async fn assert_attempt_filter_listing_basics(state: Arc<AppState>, account_id: i64) {
    let Json(first_page) = list_upstream_account_attempts(
        State(state.clone()),
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
    .expect("list first page");
    assert_eq!(first_page.total, 22);
    assert_eq!(first_page.items.len(), 20);
    assert_eq!(first_page.items[0].attempt_id, "AFILT_REMOTE");
    assert!(
        !first_page
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_IMAGE")
    );

    let Json(all_attempts) = list_upstream_account_attempts(
        State(state.clone()),
        AxumPath(account_id),
        Query(ListUpstreamAccountAttemptsQuery {
            attempt_type: None,
            model: None,
            sticky_key: None,
            page: Some(1),
            page_size: Some(100),
        }),
    )
    .await
    .expect("list all attempts");
    assert_eq!(all_attempts.total, 22);
    assert!(
        all_attempts
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_REMOTE")
    );
    assert!(
        all_attempts
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_NORMAL")
    );
    assert!(
        all_attempts
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_COMPACT")
    );
    assert!(
        all_attempts
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_IMAGE")
    );
    assert!(
        !all_attempts
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_OTHER_IMAGE")
    );
    assert_eq!(
        all_attempts
            .sticky_key_options
            .iter()
            .map(|option| option.value.as_str())
            .collect::<Vec<_>>(),
        vec![
            "sticky-remote",
            "__unbound__",
            "sticky-compact",
            "sticky-image"
        ]
    );

    assert_attempt_filter_listing_details(&all_attempts);
}

fn assert_attempt_filter_listing_details(response: &UpstreamAccountAttemptListResponse) {
    let image_item = response
        .items
        .iter()
        .find(|item| item.attempt_id == "AFILT_IMAGE")
        .expect("image attempt");
    assert_eq!(image_item.image_intent.as_deref(), Some("direct_image"));
    let remote_item = response
        .items
        .iter()
        .find(|item| item.attempt_id == "AFILT_REMOTE")
        .expect("remote_v2 attempt");
    assert_eq!(
        remote_item.compaction_request_kind.as_deref(),
        Some("remote_v2")
    );
    assert_eq!(
        remote_item.compaction_response_kind.as_deref(),
        Some("remote_v2")
    );
}

async fn assert_attempt_filter_listing_types(state: Arc<AppState>, account_id: i64) {
    let Json(image_only) = list_upstream_account_attempts(
        State(state.clone()),
        AxumPath(account_id),
        Query(ListUpstreamAccountAttemptsQuery {
            attempt_type: Some("image".to_string()),
            model: None,
            sticky_key: None,
            page: Some(1),
            page_size: Some(20),
        }),
    )
    .await
    .expect("filter image attempts");
    assert_eq!(image_only.total, 1);
    assert_eq!(image_only.items[0].attempt_id, "AFILT_IMAGE");

    let Json(remote_only) = list_upstream_account_attempts(
        State(state.clone()),
        AxumPath(account_id),
        Query(ListUpstreamAccountAttemptsQuery {
            attempt_type: Some("remote_v2".to_string()),
            model: None,
            sticky_key: None,
            page: Some(1),
            page_size: Some(20),
        }),
    )
    .await
    .expect("filter remote_v2 attempts");
    assert_eq!(remote_only.total, 18);
    assert!(
        remote_only
            .items
            .iter()
            .all(|item| { item.compaction_response_kind.as_deref() == Some("remote_v2") })
    );
    assert!(
        !remote_only
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_REQUEST_ONLY_REMOTE")
    );

    let Json(normal_only) = list_upstream_account_attempts(
        State(state.clone()),
        AxumPath(account_id),
        Query(ListUpstreamAccountAttemptsQuery {
            attempt_type: Some("normal".to_string()),
            model: None,
            sticky_key: None,
            page: Some(1),
            page_size: Some(20),
        }),
    )
    .await
    .expect("filter normal attempts");
    assert_eq!(normal_only.total, 2);
    assert!(
        normal_only
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_NORMAL")
    );
    assert!(
        normal_only
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_REQUEST_ONLY_REMOTE")
    );
}

async fn assert_attempt_filter_listing_models_and_sticky(state: Arc<AppState>, account_id: i64) {
    let Json(compact_only) = list_upstream_account_attempts(
        State(state.clone()),
        AxumPath(account_id),
        Query(ListUpstreamAccountAttemptsQuery {
            attempt_type: Some("compact".to_string()),
            model: None,
            sticky_key: None,
            page: Some(1),
            page_size: Some(20),
        }),
    )
    .await
    .expect("filter compact attempts");
    assert_eq!(compact_only.total, 1);
    assert_eq!(compact_only.items[0].attempt_id, "AFILT_COMPACT");

    for (model, expected_id, message) in [
        ("gpt-5.6", "AFILT_NORMAL", "filter by response model"),
        ("gpt-5-compact", "AFILT_COMPACT", "filter by legacy model"),
    ] {
        let Json(response) = list_upstream_account_attempts(
            State(state.clone()),
            AxumPath(account_id),
            Query(ListUpstreamAccountAttemptsQuery {
                attempt_type: None,
                model: Some(model.to_string()),
                sticky_key: None,
                page: Some(1),
                page_size: Some(20),
            }),
        )
        .await
        .expect(message);
        assert_eq!(response.total, 1);
        assert_eq!(response.items[0].attempt_id, expected_id);
    }

    let Json(unbound_only) = list_upstream_account_attempts(
        State(state.clone()),
        AxumPath(account_id),
        Query(ListUpstreamAccountAttemptsQuery {
            attempt_type: None,
            model: None,
            sticky_key: Some("__unbound__".to_string()),
            page: Some(1),
            page_size: Some(20),
        }),
    )
    .await
    .expect("filter by unbound sticky key");
    assert_eq!(unbound_only.total, 2);
    assert!(
        unbound_only
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_NORMAL")
    );
    assert!(
        unbound_only
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_REQUEST_ONLY_REMOTE")
    );
}

#[tokio::test]
pub(crate) async fn account_attempt_list_supports_type_model_and_sticky_key_filters_with_image_attempts()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Attempt filter coverage").await;
    let other_account_id = insert_api_key_account(&state.pool, "Other account scope").await;
    let now = Utc::now();
    let fixtures = attempt_filter_fixtures(account_id, other_account_id, now);
    seed_attempt_filter_rows(state.as_ref(), &fixtures).await;
    seed_attempt_filter_noise(state.as_ref(), account_id, now).await;
    assert_attempt_filter_listing_basics(state.clone(), account_id).await;
    assert_attempt_filter_listing_types(state.clone(), account_id).await;
    assert_attempt_filter_listing_models_and_sticky(state, account_id).await;
}

async fn insert_account_workflow_invocation(pool: &SqlitePool, occurred_at: &str) {
    let payload = json!({
        "requestModel": "gpt-5.5", "responseModel": "gpt-5.5",
        "endpoint": "/v1/responses", "routeMode": "pool",
        "proxyDisplayName": "Direct", "requestUserAgent": "codex-vibe-monitor-test/1.0",
        "requestXForwardedFor": "192.168.31.6", "requesterIp": "192.168.31.6",
        "promptCacheKey": "019f89ab-b67e-71a2-9633-324247eec56e",
        "requestedServiceTier": "low", "reasoningEffort": "low",
        "responseContentEncoding": "identity", "forwardedChunkCount": 7, "usageObserved": true
    })
    .to_string();
    sqlx::query(
        "INSERT INTO codex_invocations
         (invoke_id, occurred_at, source, model, input_tokens, output_tokens, cache_input_tokens,
          reasoning_tokens, total_tokens, cost, status, failure_class, is_actionable, payload,
          raw_response, price_version, request_raw_path, request_raw_size, request_raw_truncated,
          response_raw_path, response_raw_size, response_raw_truncated, detail_level, t_total_ms,
          t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, first_token_ms,
          t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, created_at)
         VALUES (?1, ?2, 'proxy', 'gpt-5.5', 49042, 87, 46952, NULL, 48769, 0.0364, 'success',
          'none', 0, ?3, '', 'test@response-tier', 'request-body.json', 217958, 0,
          'response-body.json', 79224, 0, 'full', 3280, 11, 13, 45, 120, 780, 3120, 18, 22, ?2)",
    )
    .bind("ACCOUNTWF1")
    .bind(occurred_at)
    .bind(&payload)
    .execute(pool)
    .await
    .expect("insert successful invocation with usage");
}

async fn insert_account_workflow_attempts(pool: &SqlitePool, account_id: i64, occurred_at: &str) {
    for (attempt_id, attempt_index, status, http_status, error_message) in [
        (
            "AFAIL001",
            1_i64,
            "http_failure",
            500_i64,
            Some("upstream returned a diagnostic payload"),
        ),
        ("ASUCC002", 2_i64, "success", 200_i64, None),
    ] {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts
             (attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
              routing_source, upstream_account_id, upstream_route_key, proxy_binding_key_snapshot,
              attempt_index, distinct_account_index, same_account_retry_index, requester_ip,
              started_at, finished_at, status, phase, http_status, failure_kind, error_message,
              downstream_error_message, connect_latency_ms, first_byte_latency_ms, stream_latency_ms,
              upstream_request_id, upstream_request_compression_algorithm,
              upstream_request_compression_mode, upstream_request_logical_body_bytes,
              upstream_request_transmitted_body_bytes, upstream_request_header_bytes_approx,
              upstream_response_body_bytes, upstream_response_header_bytes_approx, created_at)
             VALUES (?1, 'ACCOUNTWF1', ?2, '/v1/responses', 'pool', 'sticky-a', 'failover',
              ?3, 'route-direct', '__direct__', ?4, 1, ?4, '192.168.31.6', ?2, ?2, ?5,
              'completed', ?6, ?7, ?8, ?8, 45, 120, 3120, 'req_upstream_account_workflow',
              'zstd', 'recompressed', 217958, 53295, 1024, 79224, 776, ?2)",
        )
        .bind(attempt_id)
        .bind(occurred_at)
        .bind(account_id)
        .bind(attempt_index)
        .bind(status)
        .bind(http_status)
        .bind(error_message.map(|_| "upstream_response_failed"))
        .bind(error_message)
        .execute(pool)
        .await
        .expect("insert upstream attempt");
    }
}

async fn assert_account_workflow_attempts(state: Arc<AppState>, account_id: i64) {
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
    let success_attempt = success_item
        .workflow_entry
        .as_ref()
        .unwrap()
        .attempt
        .as_ref()
        .unwrap();
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
    let failure_summary = failure_item
        .workflow_entry
        .as_ref()
        .unwrap()
        .attempt
        .as_ref()
        .unwrap()
        .response_summary
        .as_ref()
        .unwrap();
    assert!(failure_summary["usage"].is_null());
    assert_eq!(
        failure_summary["responseBodyCapture"]["availableAtInvocationLevel"],
        json!(false)
    );
    assert_eq!(failure_summary["responseBodyCapture"]["size"], json!(79224));
    assert_eq!(
        failure_summary["responseBodyCapture"]["unavailableReason"],
        json!("attempt_response_body_not_captured")
    );
}

#[tokio::test]
pub(crate) async fn account_attempt_list_returns_workflow_entries_and_final_success_usage_only() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Attempt workflow parity").await;
    let occurred_at = format_utc_iso(Utc::now());
    insert_account_workflow_invocation(&state.pool, &occurred_at).await;
    insert_account_workflow_attempts(&state.pool, account_id, &occurred_at).await;
    assert_account_workflow_attempts(state, account_id).await;
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
