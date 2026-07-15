use super::*;
use serde_json::json;

pub(crate) async fn spawn_kaisoumail_test_harness(
    domains: &str,
    emails: Vec<(String, String, Option<String>)>,
) -> KaisouMailTestHarness {
    fn mailbox_parts_for_test(address: &str) -> (String, String) {
        let (local, domain) = address.split_once('@').expect("test mailbox address");
        (local.to_string(), domain.to_string())
    }

    async fn meta_handler(
        State(state): State<KaisouMailStubState>,
    ) -> axum::Json<serde_json::Value> {
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
    let state = Arc::new(AppState {
        config,
        sqlite_batch_writer: SqliteBatchWriter::spawn_for_test(),
        pool_account_selection_runtime: Arc::new(PoolAccountSelectionRuntime::default()),
        proxy_runtime_invocations: Arc::new(ProxyRuntimeInvocationStore::default()),
        pool,
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
        dashboard_activity_live_broadcast_seq: Arc::new(AtomicU64::new(0)),
        dashboard_activity_live_broadcast_running: Arc::new(AtomicBool::new(false)),
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
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        system_status_cache: Arc::new(Mutex::new(SystemStatusCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    });

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

fn test_account_tag_summary(id: i64, name: &str, concurrency_limit: i64) -> AccountTagSummary {
    let mut routing_rule = test_tag_routing_rule();
    routing_rule.concurrency_limit = concurrency_limit;
    AccountTagSummary {
        id,
        name: name.to_string(),
        routing_rule,
        system_key: None,
        protected: false,
    }
}

fn test_effective_routing_rule(concurrency_limit: i64) -> EffectiveRoutingRule {
    EffectiveRoutingRule {
        allow_cut_out: true,
        allow_cut_in: true,
        priority_tier: TagPriorityTier::Normal,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        concurrency_limit,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        available_models: vec![],
        available_models_defined: false,
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
            concurrency_limit: "root".to_string(),
            upstream_429_retry: "root".to_string(),
            available_models: "root".to_string(),
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
fn image_intent_routes_to_image_compatible_accounts() {
    assert!(account_accepts_requested_image_intent(
        ImageIntent::Yes,
        ImageToolRewriteMode::KeepOriginal,
        ImageToolCapability::Unknown,
    ));
    assert!(account_accepts_requested_image_intent(
        ImageIntent::Yes,
        ImageToolRewriteMode::KeepOriginal,
        ImageToolCapability::Supported,
    ));
    assert!(!account_accepts_requested_image_intent(
        ImageIntent::Yes,
        ImageToolRewriteMode::KeepOriginal,
        ImageToolCapability::Unsupported,
    ));
    assert!(account_accepts_requested_image_intent(
        ImageIntent::Yes,
        ImageToolRewriteMode::FillMissing,
        ImageToolCapability::Unsupported,
    ));
    assert!(account_accepts_requested_image_intent(
        ImageIntent::Yes,
        ImageToolRewriteMode::ForceAdd,
        ImageToolCapability::Unsupported,
    ));
    assert!(!account_accepts_requested_image_intent(
        ImageIntent::Yes,
        ImageToolRewriteMode::ForceRemove,
        ImageToolCapability::Supported,
    ));
    assert!(account_accepts_requested_image_intent(
        ImageIntent::DirectImage,
        ImageToolRewriteMode::ForceRemove,
        ImageToolCapability::Supported,
    ));
    assert!(account_accepts_requested_image_intent(
        ImageIntent::DirectImage,
        ImageToolRewriteMode::ForceRemove,
        ImageToolCapability::Unknown,
    ));
    assert!(!account_accepts_requested_image_intent(
        ImageIntent::DirectImage,
        ImageToolRewriteMode::ForceAdd,
        ImageToolCapability::Unsupported,
    ));
    assert!(account_accepts_requested_image_intent(
        ImageIntent::Unknown,
        ImageToolRewriteMode::ForceRemove,
        ImageToolCapability::Unsupported,
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
async fn account_attempt_list_reads_models_from_invocation_payload_without_schema_columns() {
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
            invoke_id, occurred_at, endpoint, route_mode, upstream_account_id,
            attempt_index, distinct_account_index, same_account_retry_index,
            started_at, finished_at, status, phase, http_status, created_at
        )
        VALUES (?1, ?2, '/v1/responses', 'pool', ?3, 1, 1, 0, ?2, ?2, 'success', 'completed', 200, ?2)
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
            ) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
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
    load_tag_detail(pool, inserted_id)
        .await?
        .ok_or_else(|| anyhow!("tag not found after insert"))
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
async fn find_existing_import_match_loads_compact_support_fields() {
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
async fn find_existing_import_match_prefers_chatgpt_user_id_for_shared_account_ids() {
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
async fn find_existing_import_match_falls_back_to_account_id_only_for_legacy_missing_user_id() {
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
struct SequencedOauthSyncServerState {
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
struct ProxyOnlyOauthSyncServerState {
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
struct TokenFailureOauthServerState {
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
struct BlockingUsageServerState {
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

async fn wait_for_atomic_usize(flag: &AtomicUsize, expected: usize) {
    timeout(Duration::from_secs(8), async {
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
async fn maintenance_pass_waits_for_brief_background_busy_slot() {
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
        "Waited Maintenance OAuth",
        "waited-maintenance@example.com",
        "org_waited_maintenance",
        "user_waited_maintenance",
    )
    .await;

    let gate = Arc::new(crate::db_pressure::DbPressureGate::new(
        1,
        Duration::from_secs(1),
    ));
    let held = gate
        .try_begin_background("startup_backfill")
        .expect("hold background slot");
    let pass = tokio::spawn({
        let gate = gate.clone();
        let state = state.clone();
        async move {
            run_upstream_account_maintenance_once_with_gate(
                state,
                gate.as_ref(),
                Duration::from_millis(500),
            )
            .await
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !started.load(Ordering::SeqCst),
        "maintenance sync should wait while the only background slot is busy"
    );

    drop(held);
    pass.await
        .expect("maintenance pass should not panic")
        .expect("maintenance pass should dispatch after the slot is released");

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
async fn maintenance_pass_reconciles_legacy_upstream_rejected_cooldown_rows() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Legacy Rejected Cooldown OAuth",
        "legacy-rejected@example.com",
        "org_legacy_rejected",
        "user_legacy_rejected",
    )
    .await;
    let failed_at = Utc::now() - ChronoDuration::hours(1);
    let failed_at_iso = format_utc_iso(failed_at);

    sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET status = ?2,
                last_error = ?3,
                last_error_at = ?4,
                last_route_failure_at = ?4,
                last_route_failure_kind = ?5,
                last_action = ?6,
                last_action_source = ?7,
                last_action_reason_code = ?8,
                last_action_reason_message = ?9,
                last_action_http_status = 402,
                last_action_at = ?4,
                cooldown_until = NULL
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind(UPSTREAM_ACCOUNT_STATUS_ERROR)
        .bind("usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}")
        .bind(&failed_at_iso)
        .bind(PROXY_FAILURE_UPSTREAM_HTTP_402)
        .bind(UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE)
        .bind(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE)
        .bind("upstream_http_402")
        .bind("usage endpoint returned 402 Payment Required: {\"detail\":{\"code\":\"deactivated_workspace\"}}")
        .execute(&state.pool)
        .await
        .expect("seed legacy maintenance rejected row");

    run_upstream_account_maintenance_once(state.clone())
        .await
        .expect("run maintenance pass");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load reconciled row")
        .expect("reconciled row exists");
    assert_eq!(
        after.cooldown_until,
        Some(format_utc_iso(
            failed_at
                + ChronoDuration::seconds(
                    UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS,
                ),
        ))
    );
    assert_eq!(after.last_synced_at, None);
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

    let state =
        test_app_state_with_usage_base_and_parallelism(&format!("http://{addr}/backend-api"), 1)
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
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: Some("updated while maintenance runs".to_string()),
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: Some(false),
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: None,
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
            refresh_token: Some("imported-refresh-token".to_string()),
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
        maintenance_proxy_snapshot: None,
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
                        email: OptionalField::Missing,
                        group_name: None,
                        group_bound_proxy_keys: None,
                        group_node_shunt_enabled: None,
                        group_single_account_rotation_enabled: None,
                        note: Some("queued note".to_string()),
                        group_note: None,
                        concurrency_limit: None,
                        upstream_base_url: OptionalField::Missing,
                        bound_proxy_keys: OptionalField::Missing,
                        enabled: None,
                        is_mother: None,
                        api_key: None,
                        local_primary_limit: None,
                        local_secondary_limit: None,
                        local_limit_unit: None,
                        tag_ids: None,
                        routing_rule: None,
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

    let state =
        test_app_state_with_usage_base_and_parallelism(&format!("http://{addr}/backend-api"), 1)
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

    let state =
        test_app_state_with_usage_base_and_parallelism(&format!("http://{addr}/backend-api"), 1)
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
async fn queued_high_frequency_maintenance_sync_recomputes_priority_fallback_before_execution() {
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

    let state =
        test_app_state_with_usage_base_and_parallelism(&format!("http://{addr}/backend-api"), 1)
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

    let first: Result<AccountSubmitOutcome<()>, AccountCommandDispatchError<anyhow::Error>> = state
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
async fn ensure_upstream_accounts_schema_migrates_legacy_block_policy_to_no_new_priority() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    sqlx::query(
        r#"
            CREATE TABLE pool_upstream_accounts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                provider TEXT NOT NULL DEFAULT 'codex',
                display_name TEXT NOT NULL,
                status TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                email TEXT,
                chatgpt_account_id TEXT,
                last_synced_at TEXT,
                last_successful_sync_at TEXT,
                policy_block_new_conversations INTEGER,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy account table");
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_accounts (
                kind, display_name, status, policy_block_new_conversations, created_at, updated_at
            ) VALUES
                ('api_key', 'legacy-block', 'active', 1, datetime('now'), datetime('now')),
                ('api_key', 'legacy-allow', 'active', 0, datetime('now'), datetime('now')),
                ('api_key', 'legacy-inherit', 'active', NULL, datetime('now'), datetime('now'))
            "#,
    )
    .execute(&pool)
    .await
    .expect("insert legacy account policies");
    sqlx::query(
        r#"
            CREATE TABLE pool_upstream_account_group_notes (
                group_name TEXT PRIMARY KEY,
                note TEXT NOT NULL,
                policy_block_new_conversations INTEGER,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy group table");
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_group_notes (
                group_name, note, policy_block_new_conversations, created_at, updated_at
            ) VALUES
                ('legacy-block-group', '', 1, datetime('now'), datetime('now')),
                ('legacy-allow-group', '', 0, datetime('now'), datetime('now')),
                ('legacy-inherit-group', '', NULL, datetime('now'), datetime('now'))
            "#,
    )
    .execute(&pool)
    .await
    .expect("insert legacy group policies");

    ensure_upstream_accounts_schema(&pool)
        .await
        .expect("upgrade legacy policy columns");

    let account_values = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
            SELECT display_name, policy_priority_tier
            FROM pool_upstream_accounts
            ORDER BY display_name
            "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load account policies");
    assert_eq!(
        account_values,
        vec![
            ("legacy-allow".to_string(), None),
            ("legacy-block".to_string(), Some("no_new".to_string())),
            ("legacy-inherit".to_string(), None),
        ]
    );

    let group_values = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
            SELECT group_name, policy_priority_tier
            FROM pool_upstream_account_group_notes
            ORDER BY group_name
            "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load group policies");
    assert_eq!(
        group_values,
        vec![
            ("legacy-allow-group".to_string(), None),
            ("legacy-block-group".to_string(), Some("no_new".to_string())),
            ("legacy-inherit-group".to_string(), None),
        ]
    );
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
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());

    assert!(
        !maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "active upstream-rejected cooldown should suppress maintenance scheduling"
    );
}

#[test]
fn maintenance_plan_prefers_explicit_upstream_rejected_cooldown_until() {
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
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());
    candidate.cooldown_until = Some("2026-04-13T17:00:00Z".to_string());

    assert!(
        !maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "explicit cooldown_until should be the canonical maintenance suppression signal"
    );
}

#[test]
fn maintenance_plan_is_due_when_reset_window_passes_even_during_upstream_rejected_cooldown() {
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
        Some("2026-04-13T11:00:00Z"),
        Some("2026-04-13T11:59:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.primary_resets_at = Some(format_utc_iso(now - ChronoDuration::minutes(1)));
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "reset-due accounts should bypass the temporary upstream-rejected cooldown"
    );
}

#[test]
fn maintenance_plan_ignores_wrapped_upstream_auth_errors_for_cooldown_blocking() {
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
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_error = Some(
        "oauth_upstream_rejected_request: pool upstream responded with 403: Forbidden".to_string(),
    );
    candidate.last_action_reason_code = Some("upstream_http_403".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH.to_string());

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "wrapped upstream auth errors should not enter the maintenance cooldown path"
    );
}

#[test]
fn resolve_due_maintenance_dispatch_plans_does_not_let_cooldown_blocked_accounts_consume_priority_slots()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };

    let mut cooldown_blocked = maintenance_candidates(
        1,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:58:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(5.0),
        Some(5.0),
    );
    cooldown_blocked.last_action_reason_code = Some("upstream_http_402".to_string());
    cooldown_blocked.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    cooldown_blocked.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    cooldown_blocked.last_action_at = Some("2026-04-13T11:58:00Z".to_string());

    let healthy_due = maintenance_candidates(
        2,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![cooldown_blocked, healthy_due],
        settings,
        Duration::from_secs(900),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].account_id, 2);
    assert_eq!(plans[0].tier, MaintenanceTier::Priority);
    assert_eq!(
        plans[0].sync_interval_secs,
        settings.primary_sync_interval_secs
    );
}

#[test]
fn maintenance_plan_is_due_for_call_driven_upstream_rejected_errors() {
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
        77,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_at = Some("2026-04-13T11:00:00Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.last_error = Some("deactivated_workspace".to_string());

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "ordinary routed 402s should still allow maintenance to retry promptly"
    );
}

#[test]
fn maintenance_plan_is_due_for_generic_403_upstream_rejected_text() {
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
        88,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:55:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:55:00Z".to_string());
    candidate.last_action_reason_code = Some("upstream_http_403".to_string());
    candidate.last_route_failure_at = Some("2026-04-13T11:55:00Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH.to_string());
    candidate.last_error = Some(
        "usage endpoint returned 403 Forbidden: upstream rejected request by policy".to_string(),
    );

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "generic 403 text that happens to mention upstream rejected should not trigger the 402-only maintenance cooldown"
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
fn resolve_due_maintenance_dispatch_plans_keeps_credits_exhausted_accounts_out_of_high_frequency() {
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

#[test]
fn maintenance_reset_due_ignores_deferred_egress_throttle_after_reset() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        15,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED.to_string());

    assert!(
        maintenance_reset_due(&candidate, now),
        "egress-deferred maintenance should not consume the post-reset catch-up sync"
    );
}

#[test]
fn maintenance_interval_is_due_respects_deferred_egress_throttle_anchor() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        16,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:00:00Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED.to_string());

    assert!(
        !maintenance_interval_is_due(&candidate, 300, now),
        "ordinary maintenance should still use egress-deferred actions as the retry anchor"
    );
}

#[test]
fn resolve_due_maintenance_dispatch_plans_requeues_reset_due_after_deferred_egress_throttle() {
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
        17,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED.to_string());

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
            "priorityTier must be one of: primary, normal, fallback, no_new".to_string(),
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
fn build_effective_routing_rule_intersects_available_models_and_collects_system_denies() {
    let mut first = test_account_tag_summary(1, "first", 0);
    first.routing_rule.available_models = vec!["gpt-5.5".to_string(), "gpt-5.4-mini".to_string()];
    let mut second = test_account_tag_summary(2, "second", 0);
    second.routing_rule.available_models = vec!["gpt-5.4-mini".to_string(), "gpt-4.1".to_string()];
    second.system_key = Some("unsupported_model:gpt-5.5".to_string());

    let rule = build_effective_routing_rule(&[first, second]);

    assert_eq!(rule.available_models, vec!["gpt-5.4-mini".to_string()]);
    assert!(rule.available_models_defined);
    assert_eq!(rule.field_sources.available_models, "tag");
    assert_eq!(rule.system_denied_models, vec!["gpt-5.5".to_string()]);
    assert_eq!(rule.field_sources.system_denied_models, "system");
}

#[test]
fn build_effective_routing_rule_intersects_available_models_by_alias() {
    let mut first = test_account_tag_summary(1, "first", 0);
    first.routing_rule.available_models = vec!["gpt-5.5-2026-01-15".to_string()];
    let mut second = test_account_tag_summary(2, "second", 0);
    second.routing_rule.available_models = vec!["gpt-5.5".to_string()];

    let rule = build_effective_routing_rule(&[first, second]);

    assert_eq!(
        rule.available_models,
        vec!["gpt-5.5-2026-01-15".to_string()]
    );
    assert!(rule.available_models_defined);
    assert!(account_accepts_requested_model(Some("gpt-5.5"), &rule));
}

#[test]
fn build_effective_routing_rule_keeps_disjoint_tag_model_intersection_as_deny_all() {
    let mut first = test_account_tag_summary(1, "first", 0);
    first.routing_rule.available_models = vec!["gpt-4o".to_string()];
    let mut second = test_account_tag_summary(2, "second", 0);
    second.routing_rule.available_models = vec!["o3".to_string()];

    let rule = build_effective_routing_rule(&[first, second]);

    assert!(rule.available_models_defined);
    assert!(rule.available_models.is_empty());
    assert!(!account_accepts_requested_model(Some("gpt-4o"), &rule));
    assert!(!account_accepts_requested_model(Some("o3"), &rule));
}

#[test]
fn apply_tag_layer_routing_policy_preserves_inherited_available_models_when_tags_do_not_define_them()
 {
    let mut inherited = test_effective_routing_rule(0);
    inherited.available_models = vec!["gpt-5.5".to_string()];
    inherited.available_models_defined = true;
    inherited.field_sources.available_models = "group".to_string();

    let tag_rule = build_effective_routing_rule(&[test_account_tag_summary(1, "tag", 0)]);

    apply_tag_layer_routing_policy(&mut inherited, &tag_rule);

    assert_eq!(inherited.available_models, vec!["gpt-5.5".to_string()]);
    assert!(inherited.available_models_defined);
    assert_eq!(inherited.field_sources.available_models, "group");
}

#[test]
fn apply_tag_layer_routing_policy_intersects_tag_models_with_inherited_group_models() {
    let mut inherited = test_effective_routing_rule(0);
    inherited.available_models = vec!["gpt-4o".to_string(), "gpt-5.5".to_string()];
    inherited.available_models_defined = true;
    inherited.field_sources.available_models = "group".to_string();

    let mut tag = test_account_tag_summary(1, "tag", 0);
    tag.routing_rule.available_models = vec!["gpt-5.5".to_string(), "o3".to_string()];
    let tag_rule = build_effective_routing_rule(&[tag]);

    apply_tag_layer_routing_policy(&mut inherited, &tag_rule);

    assert_eq!(inherited.available_models, vec!["gpt-5.5".to_string()]);
    assert!(inherited.available_models_defined);
    assert_eq!(inherited.field_sources.available_models, "tag");
}

#[test]
fn apply_tag_layer_routing_policy_intersects_inherited_models_by_alias() {
    let mut inherited = test_effective_routing_rule(0);
    inherited.available_models = vec!["gpt-5.5-2026-01-15".to_string()];
    inherited.available_models_defined = true;
    inherited.field_sources.available_models = "group".to_string();

    let mut tag = test_account_tag_summary(1, "tag", 0);
    tag.routing_rule.available_models = vec!["gpt-5.5".to_string(), "o3".to_string()];
    let tag_rule = build_effective_routing_rule(&[tag]);

    apply_tag_layer_routing_policy(&mut inherited, &tag_rule);

    assert_eq!(
        inherited.available_models,
        vec!["gpt-5.5-2026-01-15".to_string()]
    );
    assert!(inherited.available_models_defined);
    assert_eq!(inherited.field_sources.available_models, "tag");
    assert!(account_accepts_requested_model(Some("gpt-5.5"), &inherited));
}

#[test]
fn apply_tag_layer_routing_policy_keeps_group_tag_disjoint_models_as_deny_all() {
    let mut inherited = test_effective_routing_rule(0);
    inherited.available_models = vec!["gpt-4o".to_string()];
    inherited.available_models_defined = true;
    inherited.field_sources.available_models = "group".to_string();

    let mut tag = test_account_tag_summary(1, "tag", 0);
    tag.routing_rule.available_models = vec!["gpt-5.5".to_string()];
    let tag_rule = build_effective_routing_rule(&[tag]);

    apply_tag_layer_routing_policy(&mut inherited, &tag_rule);

    assert!(inherited.available_models_defined);
    assert!(inherited.available_models.is_empty());
    assert!(!account_accepts_requested_model(Some("gpt-4o"), &inherited));
    assert!(!account_accepts_requested_model(
        Some("gpt-5.5"),
        &inherited
    ));
    assert_eq!(inherited.field_sources.available_models, "tag");
}

#[test]
fn account_accepts_requested_model_supports_exact_alias_and_system_deny() {
    let mut rule = test_effective_routing_rule(0);
    rule.available_models = vec!["gpt-5.5-2026-01-15".to_string()];
    rule.available_models_defined = true;
    assert!(account_accepts_requested_model(Some("gpt-5.5"), &rule));
    assert!(account_accepts_requested_model(
        Some("gpt-5.5-2026-01-15"),
        &rule
    ));
    assert!(!account_accepts_requested_model(Some("gpt-4.1"), &rule));

    rule.system_denied_models = vec!["gpt-5.5".to_string()];
    assert!(!account_accepts_requested_model(
        Some("gpt-5.5-2026-01-15"),
        &rule
    ));
    assert!(account_accepts_requested_model(None, &rule));
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
async fn load_effective_routing_rule_for_account_uses_tag_layer_over_group_limit() {
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
    let relaxed_tag = insert_test_tag(&pool, "alpha-relaxed", &relaxed_rule)
        .await
        .expect("insert relaxed tag");

    let mut strict_rule = test_tag_routing_rule();
    strict_rule.concurrency_limit = 2;
    let strict_tag = insert_test_tag(&pool, "alpha-strict", &strict_rule)
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
            single_account_rotation_enabled: false,
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
    assert_eq!(rule.field_sources.concurrency_limit, "tag");
    assert_eq!(
        rule.source_tag_ids,
        vec![relaxed_tag.summary.id, strict_tag.summary.id]
    );
}

#[tokio::test]
async fn load_effective_routing_rule_for_account_reads_tag_available_models_from_db() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Tag Model Constraint").await;

    let mut tag_rule = test_tag_routing_rule();
    tag_rule.available_models = vec!["gpt-5.5".to_string()];
    let tag = insert_test_tag(&pool, "tag-model-constraint", &tag_rule)
        .await
        .expect("insert model tag");
    sync_account_tag_links(&pool, account_id, &[tag.summary.id])
        .await
        .expect("attach model tag");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert_eq!(rule.available_models, vec!["gpt-5.5".to_string()]);
    assert!(rule.available_models_defined);
    assert_eq!(rule.field_sources.available_models, "tag");
    assert!(account_accepts_requested_model(Some("gpt-5.5"), &rule));
    assert!(!account_accepts_requested_model(Some("gpt-4.1"), &rule));
}

#[tokio::test]
async fn ensure_account_has_unsupported_model_tag_creates_generic_system_deny_tag() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Unsupported Model Learn").await;

    ensure_account_has_unsupported_model_tag(&pool, account_id, "gpt-5.4-mini")
        .await
        .expect("learn unsupported model deny");

    let row: (Option<String>, i64) = sqlx::query_as(
        r#"
            SELECT tag.system_key, tag.protected
            FROM pool_upstream_account_tags link
            INNER JOIN pool_tags tag ON tag.id = link.tag_id
            WHERE link.account_id = ?1
            "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load linked deny tag");

    assert_eq!(row.0.as_deref(), Some("unsupported_model:gpt-5.4-mini"));
    assert_eq!(row.1, 1);
}

#[tokio::test]
async fn load_effective_routing_rule_for_account_applies_group_tag_account_overrides() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Layered Policy").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET group_name = ?2,
                policy_allow_cut_in = 1,
                policy_fast_mode_rewrite_mode = 'force_remove',
                policy_upstream_429_retry_enabled = 1,
                policy_upstream_429_max_retries = 4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("layered")
    .execute(&pool)
    .await
    .expect("assign account override");

    let mut tag_rule = test_tag_routing_rule();
    tag_rule.allow_cut_in = false;
    tag_rule.priority_tier = TagPriorityTier::Fallback;
    tag_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::FillMissing;
    tag_rule.concurrency_limit = 3;
    tag_rule.upstream_429_retry_enabled = true;
    tag_rule.upstream_429_max_retries = 2;
    let tag = insert_test_tag(&pool, "layered-tag", &tag_rule)
        .await
        .expect("insert layered tag");
    sync_account_tag_links(&pool, account_id, &[tag.summary.id])
        .await
        .expect("attach layered tag");

    let mut conn = pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "layered",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 8,
        },
    )
    .await
    .expect("save legacy group metadata");
    drop(conn);
    sqlx::query(
        r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_priority_tier = 'primary',
                policy_fast_mode_rewrite_mode = 'force_add',
                policy_concurrency_limit = 5
            WHERE group_name = 'layered'
            "#,
    )
    .execute(&pool)
    .await
    .expect("save group policy override");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load layered effective routing rule");

    assert_eq!(rule.priority_tier, TagPriorityTier::Fallback);
    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceRemove
    );
    assert_eq!(rule.concurrency_limit, 3);
    assert_eq!(rule.field_sources.concurrency_limit, "tag");
    assert!(rule.allow_cut_in);
    assert_eq!(rule.field_sources.allow_cut_in, "account");
    assert!(rule.upstream_429_retry_enabled);
    assert_eq!(rule.upstream_429_max_retries, 4);
    assert_eq!(rule.field_sources.priority_tier, "tag");
    assert_eq!(rule.field_sources.fast_mode_rewrite_mode, "account");
    assert_eq!(rule.field_sources.upstream_429_retry, "account");
}

#[tokio::test]
async fn load_effective_routing_rule_for_account_lets_tag_disable_group_retry() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Tag Retry Disable").await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind("retry-group")
        .execute(&pool)
        .await
        .expect("assign group name");

    let tag_rule = test_tag_routing_rule();
    assert!(!tag_rule.upstream_429_retry_enabled);
    let tag = insert_test_tag(&pool, "retry-disabled-tag", &tag_rule)
        .await
        .expect("insert retry-disabled tag");
    sync_account_tag_links(&pool, account_id, &[tag.summary.id])
        .await
        .expect("attach retry-disabled tag");

    let mut conn = pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "retry-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);
    sqlx::query(
        r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_upstream_429_retry_enabled = 1,
                policy_upstream_429_max_retries = 5
            WHERE group_name = 'retry-group'
            "#,
    )
    .execute(&pool)
    .await
    .expect("save group retry policy");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert!(!rule.upstream_429_retry_enabled);
    assert_eq!(rule.upstream_429_max_retries, 0);
    assert_eq!(rule.field_sources.upstream_429_retry, "tag");
}

#[tokio::test]
async fn load_effective_routing_rule_for_account_allows_account_block_override_to_clear_group() {
    let pool = test_pool().await;
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_group_notes (
                group_name,
                note,
                policy_priority_tier,
                created_at,
                updated_at
            ) VALUES ('blocked-group', '', 'no_new', '2026-03-15T00:00:00Z', '2026-03-15T00:00:00Z')
            "#,
    )
    .execute(&pool)
    .await
    .expect("save group block policy");
    let account_id = insert_api_key_account(&pool, "Account Block Override").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET group_name = 'blocked-group',
                policy_priority_tier = 'normal'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("save account routing policy");
    let tag = insert_test_tag(&pool, "block-tag", &test_tag_routing_rule())
        .await
        .expect("insert block tag");
    sync_account_tag_links(&pool, account_id, &[tag.summary.id])
        .await
        .expect("attach block tag");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert_eq!(rule.priority_tier, TagPriorityTier::Normal);
    assert_eq!(rule.field_sources.priority_tier, "account");
}

#[tokio::test]
async fn update_upstream_account_preserves_account_policy_when_routing_rule_is_missing() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Preserve Account Policy").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_allow_cut_in = 0,
                policy_fast_mode_rewrite_mode = 'force_add',
                policy_upstream_429_retry_enabled = 1,
                policy_upstream_429_max_retries = 3
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account policy");

    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: Some("metadata only".to_string()),
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: Some(false),
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: None,
            },
        )
        .await
        .expect("metadata-only update");

    let rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load preserved policy");
    assert!(!rule.allow_cut_in);
    assert_eq!(rule.field_sources.allow_cut_in, "account");
    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceAdd
    );
    assert_eq!(rule.field_sources.fast_mode_rewrite_mode, "account");
    assert!(rule.upstream_429_retry_enabled);
    assert_eq!(rule.upstream_429_max_retries, 3);
    assert_eq!(rule.field_sources.upstream_429_retry, "account");
    assert!(!detail.summary.effective_routing_rule.allow_cut_in);
    assert_eq!(
        detail.summary.effective_routing_rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceAdd
    );
    assert_eq!(
        detail
            .summary
            .effective_routing_rule
            .field_sources
            .fast_mode_rewrite_mode,
        "account"
    );
}

#[tokio::test]
async fn update_upstream_account_clears_individual_account_policy_override() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Clear Account Policy").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_allow_cut_in = 0,
                policy_fast_mode_rewrite_mode = 'force_add',
                policy_available_models_json = '[]'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account policy");

    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                bound_proxy_keys: OptionalField::Missing,
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
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Null,
                    priority_tier: OptionalField::Missing,
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
            },
        )
        .await
        .expect("clear account policy field");

    let stored = sqlx::query_as::<_, (Option<i64>, Option<String>, Option<String>)>(
            "SELECT policy_allow_cut_in, policy_fast_mode_rewrite_mode, policy_available_models_json FROM pool_upstream_accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load stored policy");
    assert_eq!(stored.0, None);
    assert_eq!(stored.1.as_deref(), Some("force_add"));
    assert_eq!(stored.2.as_deref(), Some("[]"));
}

#[tokio::test]
async fn update_upstream_account_patches_one_timeout_without_clearing_other_overrides() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Patch Timeout Policy").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_responses_first_byte_timeout_secs = 180,
                policy_compact_first_byte_timeout_secs = 300,
                policy_image_first_byte_timeout_secs = 360,
                policy_responses_stream_timeout_secs = 1800,
                policy_compact_stream_timeout_secs = 300
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account timeout overrides");
    assert_eq!(
        load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load seeded account")
            .expect("seeded account exists")
            .policy_image_first_byte_timeout_secs,
        Some(360)
    );

    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                bound_proxy_keys: OptionalField::Missing,
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
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Missing,
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: Some(UpdateRoutingTimeoutSettingsRequest {
                        responses_first_byte_timeout_secs: OptionalField::Missing,
                        compact_first_byte_timeout_secs: OptionalField::Missing,
                        image_first_byte_timeout_secs: OptionalField::Missing,
                        responses_stream_timeout_secs: OptionalField::Value(1900),
                        compact_stream_timeout_secs: OptionalField::Missing,
                    }),
                }),
            },
        )
        .await
        .expect("patch one account timeout field");

    let stored_image_timeout: Option<i64> = sqlx::query_scalar(
        "SELECT policy_image_first_byte_timeout_secs FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load stored image timeout override");
    assert_eq!(stored_image_timeout, Some(360));

    let response_rule = detail.summary.effective_routing_rule;
    assert_eq!(
        response_rule.timeouts.responses_first_byte_timeout_secs,
        Some(180)
    );
    assert_eq!(
        response_rule.timeouts.compact_first_byte_timeout_secs,
        Some(300)
    );
    assert_eq!(
        response_rule.timeouts.image_first_byte_timeout_secs,
        Some(360)
    );
    assert_eq!(
        response_rule.timeouts.responses_stream_timeout_secs,
        Some(1900)
    );
    assert_eq!(
        response_rule.timeouts.compact_stream_timeout_secs,
        Some(300)
    );
    assert_eq!(
        response_rule
            .timeout_field_sources
            .responses_first_byte_timeout_secs,
        "account"
    );
    assert_eq!(
        response_rule
            .timeout_field_sources
            .responses_stream_timeout_secs,
        "account"
    );

    let stored = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>)>(
            "SELECT policy_responses_first_byte_timeout_secs, policy_compact_first_byte_timeout_secs, policy_image_first_byte_timeout_secs, policy_responses_stream_timeout_secs, policy_compact_stream_timeout_secs FROM pool_upstream_accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load stored timeout policy");
    assert_eq!(
        stored,
        (Some(180), Some(300), Some(360), Some(1900), Some(300))
    );

    let reloaded_rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("reload effective routing rule");
    assert_eq!(
        reloaded_rule.timeouts.responses_first_byte_timeout_secs,
        Some(180)
    );
    assert_eq!(
        reloaded_rule.timeouts.compact_first_byte_timeout_secs,
        Some(300)
    );
    assert_eq!(
        reloaded_rule.timeouts.image_first_byte_timeout_secs,
        Some(360)
    );
    assert_eq!(
        reloaded_rule.timeouts.responses_stream_timeout_secs,
        Some(1900)
    );
    assert_eq!(
        reloaded_rule.timeouts.compact_stream_timeout_secs,
        Some(300)
    );
}

#[tokio::test]
async fn update_upstream_account_writes_positive_new_conversation_policy() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Positive Account Policy").await;

    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Value("no_new".to_string()),
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
            },
        )
        .await
        .expect("save positive new conversation policy");

    let stored = sqlx::query_scalar::<_, Option<String>>(
        "SELECT policy_priority_tier FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load stored policy");
    assert_eq!(stored, Some("no_new".to_string()));

    let rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load effective routing rule");
    assert_eq!(rule.priority_tier, TagPriorityTier::NoNew);
    assert_eq!(rule.field_sources.priority_tier, "account");
}

#[tokio::test]
async fn update_upstream_account_preserves_priority_tier_when_omitted() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Preserve Legacy Block").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_priority_tier = 'no_new'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed positive and legacy new conversation policy");

    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Value(false),
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Missing,
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
            },
        )
        .await
        .expect("save unrelated account policy field");

    let stored = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
            "SELECT policy_priority_tier, policy_allow_cut_out FROM pool_upstream_accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load stored policy");
    assert_eq!(stored, (Some("no_new".to_string()), Some(0)));
}

#[tokio::test]
async fn update_upstream_account_accepts_no_new_priority_write() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Legacy Block Write").await;

    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Value("no_new".to_string()),
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
            },
        )
        .await
        .expect("save legacy block new conversations policy");

    let stored = sqlx::query_scalar::<_, Option<String>>(
        "SELECT policy_priority_tier FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load stored policy");
    assert_eq!(stored, Some("no_new".to_string()));

    let rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load effective routing rule");
    assert_eq!(rule.priority_tier, TagPriorityTier::NoNew);
    assert_eq!(rule.field_sources.priority_tier, "account");
}

#[tokio::test]
async fn update_upstream_account_does_not_change_priority_tier_when_omitted() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Legacy Only Missing").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_priority_tier = 'no_new'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed legacy-only new conversation policy");

    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Value(false),
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Missing,
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
            },
        )
        .await
        .expect("save unrelated account policy field");

    let stored = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
            "SELECT policy_priority_tier, policy_allow_cut_out FROM pool_upstream_accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load stored policy");
    assert_eq!(stored, (Some("no_new".to_string()), Some(0)));
}

#[tokio::test]
async fn update_upstream_account_persists_empty_available_models_as_deny_all() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Deny All Models").await;

    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Missing,
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Value(vec![]),
                    status_change_reasons: None,
                    timeouts: None,
                }),
            },
        )
        .await
        .expect("save empty model override");

    let rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load effective routing rule");
    assert!(rule.available_models_defined);
    assert!(rule.available_models.is_empty());
    assert_eq!(rule.field_sources.available_models, "account");
}

#[tokio::test]
async fn update_upstream_account_rejects_invalid_routing_policy_enums() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Invalid Account Policy").await;

    let err = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Value("normal".to_string()),
                    fast_mode_rewrite_mode: OptionalField::Value("always_fast".to_string()),
                    image_tool_rewrite_mode: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
            },
        )
        .await
        .expect_err("invalid routing policy enum should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "fastModeRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add"
    );
}

#[tokio::test]
async fn load_upstream_account_detail_with_actual_usage_returns_layered_effective_policy() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Layered Detail").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET group_name = ?2,
                policy_fast_mode_rewrite_mode = 'force_remove'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("detail-layered")
    .execute(&state.pool)
    .await
    .expect("assign account detail policy");

    let mut tag_rule = test_tag_routing_rule();
    tag_rule.allow_cut_in = false;
    tag_rule.priority_tier = TagPriorityTier::Fallback;
    tag_rule.upstream_429_retry_enabled = true;
    tag_rule.upstream_429_max_retries = 2;
    let tag = insert_test_tag(&state.pool, "detail-layered-tag", &tag_rule)
        .await
        .expect("insert detail tag");
    sync_account_tag_links(&state.pool, account_id, &[tag.summary.id])
        .await
        .expect("attach detail tag");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "detail-layered",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 7,
        },
    )
    .await
    .expect("save detail group metadata");
    drop(conn);
    sqlx::query(
        r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_priority_tier = 'primary',
                policy_fast_mode_rewrite_mode = 'force_add',
                policy_concurrency_limit = 5
            WHERE group_name = 'detail-layered'
            "#,
    )
    .execute(&state.pool)
    .await
    .expect("save detail group policy");

    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .expect("load account detail")
        .expect("detail exists");
    let rule = detail.summary.effective_routing_rule;
    assert_eq!(rule.priority_tier, TagPriorityTier::Fallback);
    assert_eq!(rule.field_sources.priority_tier, "tag");
    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceRemove
    );
    assert_eq!(rule.field_sources.fast_mode_rewrite_mode, "account");
    assert_eq!(rule.concurrency_limit, 0);
    assert_eq!(rule.field_sources.concurrency_limit, "tag");
    assert!(!rule.allow_cut_in);
    assert_eq!(rule.field_sources.allow_cut_in, "tag");
    assert!(rule.upstream_429_retry_enabled);
    assert_eq!(rule.upstream_429_max_retries, 2);
    assert_eq!(rule.field_sources.upstream_429_retry, "tag");
}

#[tokio::test]
async fn load_upstream_account_detail_with_actual_usage_populates_root_timeout_values() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Root Timeout Detail").await;
    sqlx::query(
        r#"
            UPDATE pool_routing_settings
            SET responses_first_byte_timeout_secs = 41,
                compact_first_byte_timeout_secs = 42,
                image_first_byte_timeout_secs = 45,
                responses_stream_timeout_secs = 43,
                compact_stream_timeout_secs = 44
            WHERE id = ?1
            "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .execute(&state.pool)
    .await
    .expect("save root timeout settings");

    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .expect("load account detail")
        .expect("detail exists");
    let rule = detail.summary.effective_routing_rule;

    assert_eq!(rule.timeouts.responses_first_byte_timeout_secs, Some(41));
    assert_eq!(rule.timeouts.compact_first_byte_timeout_secs, Some(42));
    assert_eq!(rule.timeouts.image_first_byte_timeout_secs, Some(45));
    assert_eq!(rule.timeouts.responses_stream_timeout_secs, Some(43));
    assert_eq!(rule.timeouts.compact_stream_timeout_secs, Some(44));
    assert_eq!(
        rule.timeout_field_sources.responses_first_byte_timeout_secs,
        "root"
    );
    assert_eq!(
        rule.timeout_field_sources.compact_first_byte_timeout_secs,
        "root"
    );
    assert_eq!(
        rule.timeout_field_sources.image_first_byte_timeout_secs,
        "root"
    );
    assert_eq!(
        rule.timeout_field_sources.responses_stream_timeout_secs,
        "root"
    );
    assert_eq!(
        rule.timeout_field_sources.compact_stream_timeout_secs,
        "root"
    );
}

#[tokio::test]
async fn load_effective_routing_rule_for_account_uses_most_conservative_tag_priority() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Priority Merge").await;

    let mut primary_rule = test_tag_routing_rule();
    primary_rule.priority_tier = TagPriorityTier::Primary;
    let primary_tag = insert_test_tag(&pool, "priority-primary", &primary_rule)
        .await
        .expect("insert primary tag");

    let mut fallback_rule = test_tag_routing_rule();
    fallback_rule.priority_tier = TagPriorityTier::Fallback;
    let fallback_tag = insert_test_tag(&pool, "priority-fallback", &fallback_rule)
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
    let fill_missing_tag = insert_test_tag(&pool, "fast-fill", &fill_missing_rule)
        .await
        .expect("insert fill-missing tag");

    let mut force_remove_rule = test_tag_routing_rule();
    force_remove_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::ForceRemove;
    let force_remove_tag = insert_test_tag(&pool, "fast-remove", &force_remove_rule)
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
async fn list_tags_only_returns_system_tags_and_disables_writes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "System Tag Account").await;
    ensure_account_has_gpt55_unsupported_tag(&state.pool, account_id)
        .await
        .expect("seed system tag");
    let custom_tag_id = insert_legacy_custom_tag(
        &state.pool,
        "fast-mode-round-trip",
        &test_tag_routing_rule(),
    )
    .await;

    let Json(listed) = list_tags(
        State(state.clone()),
        Query(ListTagsQuery {
            search: None,
            has_accounts: None,
            allow_cut_in: None,
            allow_cut_out: None,
        }),
    )
    .await
    .expect("list tags");
    assert!(!listed.writes_enabled);
    assert!(
        listed
            .items
            .iter()
            .all(|item| item.system_key.as_deref().is_some())
    );
    assert!(
        listed
            .items
            .iter()
            .any(|item| item.system_key.as_deref() == Some("unsupported_model:gpt-5.5"))
    );
    assert!(listed.items.iter().all(|item| item.id != custom_tag_id));
}
