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

async fn build_kaisoumail_test_state(
    config: AppConfig,
    http_clients: HttpClients,
    pool: SqlitePool,
) -> Arc<AppState> {
    let proxy_raw_async_writer_limit = proxy_raw_async_writer_limit(&config);
    let (broadcaster, _) = broadcast::channel(8);
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
    let pool = test_pool().await;
    let state = build_kaisoumail_test_state(config, http_clients, pool).await;

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

pub(crate) fn account_attempt_workflow_payload() -> String {
    json!({
        "requestModel": "gpt-5.5",
        "responseModel": "gpt-5.5",
        "endpoint": "/v1/responses",
        "routeMode": "pool",
        "proxyDisplayName": "Direct",
        "requestUserAgent": "codex-vibe-monitor-test/1.0",
        "requestXForwardedFor": "192.168.31.6",
        "requesterIp": "192.168.31.6",
        "promptCacheKey": "019f89ab-b67e-71a2-9633-324247eec56e",
        "requestedServiceTier": "low",
        "reasoningEffort": "low",
        "responseContentEncoding": "identity",
        "forwardedChunkCount": 7,
        "usageObserved": true
    })
    .to_string()
}

pub(crate) async fn seed_account_attempt_invocation(
    state: &AppState,
    occurred_at: &str,
    payload: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, model, input_tokens, output_tokens,
            cache_input_tokens, reasoning_tokens, total_tokens, cost, status,
            failure_class, is_actionable, payload, raw_response, price_version,
            request_raw_path, request_raw_size, request_raw_truncated,
            response_raw_path, response_raw_size, response_raw_truncated,
            detail_level, t_total_ms, t_req_read_ms, t_req_parse_ms,
            t_upstream_connect_ms, t_upstream_ttfb_ms, first_token_ms, t_upstream_stream_ms,
            t_resp_parse_ms, t_persist_ms, created_at
        )
        VALUES (
            ?1, ?2, 'proxy', 'gpt-5.5', 49042, 87,
            46952, NULL, 48769, 0.0364, 'success',
            'none', 0, ?3, '', 'test@response-tier',
            'request-body.json', 217958, 0,
            'response-body.json', 79224, 0,
            'full', 3280, 11, 13, 45, 120, 780, 3120,
            18, 22, ?2
        )
        "#,
    )
    .bind("ACCOUNTWF1")
    .bind(occurred_at)
    .bind(payload)
    .execute(&state.pool)
    .await
    .expect("insert successful invocation with usage");
}

pub(crate) async fn seed_account_attempt_rows(
    state: &AppState,
    account_id: i64,
    occurred_at: &str,
) {
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
            r#"
            INSERT INTO pool_upstream_request_attempts (
                attempt_public_id, invoke_id, occurred_at, endpoint, route_mode,
                sticky_key, routing_source, upstream_account_id, upstream_route_key,
                proxy_binding_key_snapshot, attempt_index, distinct_account_index,
                same_account_retry_index, requester_ip, started_at, finished_at,
                status, phase, http_status, failure_kind, error_message,
                downstream_error_message, connect_latency_ms, first_byte_latency_ms,
                stream_latency_ms, upstream_request_id,
                upstream_request_compression_algorithm, upstream_request_compression_mode,
                upstream_request_logical_body_bytes,
                upstream_request_transmitted_body_bytes,
                upstream_request_header_bytes_approx,
                upstream_response_body_bytes,
                upstream_response_header_bytes_approx,
                created_at
            )
            VALUES (
                ?1, 'ACCOUNTWF1', ?2, '/v1/responses', 'pool',
                'sticky-a', 'failover', ?3, 'route-direct', '__direct__', ?4, 1, ?4,
                '192.168.31.6', ?2, ?2, ?5, 'completed', ?6, ?7, ?8,
                ?8, 45, 120, 3120, 'req_upstream_account_workflow',
                'zstd', 'recompressed', 217958, 53295, 1024, 79224, 776, ?2
            )
            "#,
        )
        .bind(attempt_id)
        .bind(occurred_at)
        .bind(account_id)
        .bind(attempt_index)
        .bind(status)
        .bind(http_status)
        .bind(error_message.map(|_| "upstream_response_failed"))
        .bind(error_message)
        .execute(&state.pool)
        .await
        .expect("insert upstream attempt");
    }
}

async fn seed_attempt_filter_fixtures(
    state: &AppState,
    account_id: i64,
    other_account_id: i64,
    now: chrono::DateTime<Utc>,
) {
    let fixtures = vec![
        AttemptFixture {
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
        AttemptFixture {
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
        AttemptFixture {
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
        AttemptFixture {
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
        AttemptFixture {
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
        AttemptFixture {
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
    ];
    for fixture in &fixtures {
        insert_attempt_fixture(&state.pool, fixture).await;
    }
    insert_remote_noise(&state.pool, account_id, now).await;
}

async fn query_attempts(
    state: Arc<AppState>,
    account_id: i64,
    attempt_type: Option<&str>,
    model: Option<&str>,
    sticky_key: Option<&str>,
    page_size: usize,
) -> UpstreamAccountAttemptListResponse {
    let Json(response) = list_upstream_account_attempts(
        State(state),
        AxumPath(account_id),
        Query(ListUpstreamAccountAttemptsQuery {
            attempt_type: attempt_type.map(str::to_string),
            model: model.map(str::to_string),
            sticky_key: sticky_key.map(str::to_string),
            page: Some(1),
            page_size: Some(page_size),
        }),
    )
    .await
    .expect("list account attempts");
    response
}

async fn assert_attempt_filter_page_and_metadata(state: Arc<AppState>, account_id: i64) {
    let first_page = query_attempts(state.clone(), account_id, None, None, None, 20).await;
    assert_eq!(first_page.total, 22);
    assert_eq!(first_page.items.len(), 20);
    assert_eq!(first_page.items[0].attempt_id, "AFILT_REMOTE");
    assert!(
        !first_page
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_IMAGE")
    );

    let all_attempts = query_attempts(state, account_id, None, None, None, 100).await;
    assert_eq!(all_attempts.total, 22);
    for attempt_id in [
        "AFILT_REMOTE",
        "AFILT_NORMAL",
        "AFILT_COMPACT",
        "AFILT_IMAGE",
    ] {
        assert!(
            all_attempts
                .items
                .iter()
                .any(|item| item.attempt_id == attempt_id)
        );
    }
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

    let image_item = all_attempts
        .items
        .iter()
        .find(|item| item.attempt_id == "AFILT_IMAGE")
        .expect("image attempt");
    assert_eq!(image_item.image_intent.as_deref(), Some("direct_image"));
    let remote_item = all_attempts
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

#[derive(Clone)]
struct AttemptFixture<'a> {
    attempt_id: &'a str,
    invoke_id: &'a str,
    account_id: i64,
    occurred_at: String,
    endpoint: &'a str,
    sticky_key: Option<&'a str>,
    model: &'a str,
    request_model: Option<&'a str>,
    response_model: Option<&'a str>,
    compaction_request_kind: Option<&'a str>,
    compaction_response_kind: Option<&'a str>,
    image_intent: Option<&'a str>,
}

async fn insert_attempt_fixture(pool: &SqlitePool, fixture: &AttemptFixture<'_>) {
    let payload = json!({
        "requestModel": fixture.request_model,
        "responseModel": fixture.response_model,
        "compactionRequestKind": fixture.compaction_request_kind,
        "compactionResponseKind": fixture.compaction_response_kind,
        "imageIntent": fixture.image_intent,
    })
    .to_string();
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, model, payload, raw_response) VALUES (?1, ?2, 'proxy', ?3, ?4, '')",
    )
    .bind(fixture.invoke_id)
    .bind(&fixture.occurred_at)
    .bind(fixture.model)
    .bind(payload)
    .execute(pool)
    .await
    .expect("insert invocation");
    sqlx::query(
        "INSERT INTO pool_upstream_request_attempts (attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key, upstream_account_id, attempt_index, distinct_account_index, same_account_retry_index, started_at, finished_at, status, phase, http_status, created_at) VALUES (?1, ?2, ?3, ?4, 'pool', ?5, ?6, 1, 1, 0, ?3, ?3, 'success', 'completed', 200, ?3)",
    )
    .bind(fixture.attempt_id)
    .bind(fixture.invoke_id)
    .bind(&fixture.occurred_at)
    .bind(fixture.endpoint)
    .bind(fixture.sticky_key)
    .bind(fixture.account_id)
    .execute(pool)
    .await
    .expect("insert account attempt");
}

async fn insert_remote_noise(pool: &SqlitePool, account_id: i64, now: DateTime<Utc>) {
    for index in 0..17 {
        let invoke_id = format!("afilt_remote_noise_{index:02}");
        let attempt_id = format!("AFILTNOISE{index:02}");
        let occurred_at = format_utc_iso(now - ChronoDuration::seconds(70 + index as i64));
        let fixture = AttemptFixture {
            attempt_id: &attempt_id,
            invoke_id: &invoke_id,
            account_id,
            occurred_at,
            endpoint: "/v1/responses",
            sticky_key: Some("sticky-remote"),
            model: "gpt-5.5",
            request_model: Some("gpt-5.5"),
            response_model: Some("gpt-5.5-2026-07-01"),
            compaction_request_kind: Some("remote_v2"),
            compaction_response_kind: Some("remote_v2"),
            image_intent: Some("no"),
        };
        insert_attempt_fixture(pool, &fixture).await;
    }
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

#[tokio::test]
pub(crate) async fn account_attempt_list_supports_type_model_and_sticky_key_filters_with_image_attempts()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Attempt filter coverage").await;
    let other_account_id = insert_api_key_account(&state.pool, "Other account scope").await;
    let now = Utc::now();
    seed_attempt_filter_fixtures(&state, account_id, other_account_id, now).await;

    assert_attempt_filter_page_and_metadata(state.clone(), account_id).await;

    let image_only = query_attempts(state.clone(), account_id, Some("image"), None, None, 20).await;
    assert_eq!(image_only.total, 1);
    assert_eq!(image_only.items[0].attempt_id, "AFILT_IMAGE");

    let remote_only =
        query_attempts(state.clone(), account_id, Some("remote_v2"), None, None, 20).await;
    assert_eq!(remote_only.total, 18);
    assert!(
        remote_only
            .items
            .iter()
            .all(|item| item.compaction_response_kind.as_deref() == Some("remote_v2"))
    );
    assert!(
        !remote_only
            .items
            .iter()
            .any(|item| item.attempt_id == "AFILT_REQUEST_ONLY_REMOTE")
    );

    let normal_only =
        query_attempts(state.clone(), account_id, Some("normal"), None, None, 20).await;
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

    let compact_only =
        query_attempts(state.clone(), account_id, Some("compact"), None, None, 20).await;
    assert_eq!(compact_only.total, 1);
    assert_eq!(compact_only.items[0].attempt_id, "AFILT_COMPACT");

    let response_model_match =
        query_attempts(state.clone(), account_id, None, Some("gpt-5.6"), None, 20).await;
    assert_eq!(response_model_match.total, 1);
    assert_eq!(response_model_match.items[0].attempt_id, "AFILT_NORMAL");

    let legacy_model_match = query_attempts(
        state.clone(),
        account_id,
        None,
        Some("gpt-5-compact"),
        None,
        20,
    )
    .await;
    assert_eq!(legacy_model_match.total, 1);
    assert_eq!(legacy_model_match.items[0].attempt_id, "AFILT_COMPACT");

    let unbound_only = query_attempts(state, account_id, None, None, Some("__unbound__"), 20).await;
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

use super::*;
