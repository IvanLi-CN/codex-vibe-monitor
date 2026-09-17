pub(crate) fn run_oauth_future_with_large_stack<T, Fut>(future: Fut) -> T
where
    T: Send + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    std::thread::Builder::new()
        .name("oauth-route-large-stack".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build large-stack test runtime")
                .block_on(future)
        })
        .expect("spawn large-stack test worker")
        .join()
        .expect("join large-stack test worker")
}

#[tokio::test]
pub(crate) async fn oauth_streaming_passthrough_backfills_body_prefix_from_replay_status() {
    let crypto_key: [u8; 32] = Sha256::digest(b"test-upstream-account-secret").into();
    let mut debug = Some(oauth_bridge::OauthResponsesDebugInfo {
        fingerprint_version: Some("v1"),
        forwarded_header_names: vec!["session_id".to_string()],
        forwarded_header_fingerprints: Some(BTreeMap::from([(
            "session_id".to_string(),
            "0123456789abcdef".to_string(),
        )])),
        prompt_cache_header_forwarded: false,
        request_body_prefix_fingerprint: None,
        request_body_prefix_bytes: None,
        request_body_snapshot_kind: None,
        responses_body_mode: None,
        rewrite: oauth_bridge::OauthResponsesRewriteSummary::default(),
    });
    let (status_tx, status_rx) = watch::channel(PoolReplayBodyStatus::Reading);
    status_tx
        .send(PoolReplayBodyStatus::Complete(
            PoolReplayBodySnapshot::Memory(Bytes::from_static(
                br#"{"messages":[{"role":"user","content":"hello"}]}"#,
            )),
        ))
        .expect("send replay completion");

    maybe_backfill_oauth_request_debug_from_replay_status(
        &mut debug,
        &"/v1/chat/completions"
            .parse()
            .expect("valid chat completions uri"),
        &status_rx,
        Some(&crypto_key),
    )
    .await;

    let debug = debug.expect("debug should remain present");
    assert!(
        debug
            .request_body_prefix_bytes
            .expect("body prefix bytes should be backfilled")
            > 0
    );
    assert_eq!(
        debug
            .request_body_prefix_fingerprint
            .as_ref()
            .map(String::len),
        Some(16)
    );
}

#[tokio::test]
pub(crate) async fn pool_route_oauth_body_sticky_binding_applies_before_first_send() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let _primary_id =
        insert_test_pool_oauth_account(&state, "Primary OAuth", "oauth-primary").await;
    let secondary_id =
        insert_test_pool_oauth_account(&state, "Secondary OAuth", "oauth-secondary").await;
    record_pool_route_success(
        &state.pool,
        secondary_id,
        Utc::now(),
        Some("sticky-oauth-body"),
        None,
    )
    .await
    .expect("seed oauth sticky route");

    let request_body = serde_json::to_vec(&json!({
        "messages": [{
            "role": "user",
            "content": "hello",
        }],
        "stickyKey": "sticky-oauth-body",
    }))
    .expect("serialize oauth sticky body");
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(4);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from(request_body))).await;
    });

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read oauth sticky response"),
    )
    .expect("decode oauth sticky response");
    assert_eq!(
        payload["authorization"].as_str(),
        Some("Bearer oauth-secondary")
    );
    let selected_at: Vec<(i64, Option<String>)> = sqlx::query_as(
        r#"
        SELECT id, last_selected_at
        FROM pool_upstream_accounts
        WHERE display_name IN ('Primary OAuth', 'Secondary OAuth')
        ORDER BY id
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load oauth account selection timestamps");
    assert_eq!(selected_at.len(), 2);
    assert_eq!(selected_at[0].1, None);
    assert!(
        selected_at[1].1.is_some(),
        "sticky-selected oauth account should be marked"
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
pub(crate) async fn pool_route_oauth_standalone_search_is_forwarded_and_recorded() {
    #[derive(Debug, sqlx::FromRow)]
    struct InvocationRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
        payload: Option<String>,
    }

    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;
    let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id =
        insert_test_pool_oauth_account(&state, "Standalone Search OAuth", "oauth-search").await;
    let request_body = br#"{"model":"gpt-5.4","query":"hello","results":[]}"#;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/alpha/search?cursor=next"
                .parse()
                .expect("valid standalone search uri"),
        ),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(request_body.to_vec()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let forwarded: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read oauth standalone search response"),
    )
    .expect("decode oauth standalone search response");
    assert_eq!(forwarded["path"], "/backend-api/codex/alpha/search");
    assert_eq!(forwarded["query"], "cursor=next");
    assert_eq!(
        forwarded["body"],
        String::from_utf8_lossy(request_body).as_ref()
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    let invocation = sqlx::query_as::<_, InvocationRow>(
        r#"
        SELECT status, input_tokens, output_tokens, total_tokens, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load oauth standalone search invocation");
    assert_eq!(invocation.status.as_deref(), Some("success"));
    assert!(invocation.input_tokens.is_none());
    assert!(invocation.output_tokens.is_none());
    assert!(invocation.total_tokens.is_none());
    let payload: Value = serde_json::from_str(
        invocation
            .payload
            .as_deref()
            .expect("oauth standalone search payload should be present"),
    )
    .expect("decode oauth standalone search payload");
    assert_eq!(payload["endpoint"], "/v1/alpha/search");
    assert_eq!(payload["upstreamAccountId"], account_id);
    assert_ne!(payload["failureKind"], "oauth_unsupported_route");

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
pub(crate) async fn pool_route_oauth_compact_passthrough_preserves_prompt_cache_headers() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_oauth_account(&state, "Compact OAuth", "oauth-compact").await;

    let payload = send_oauth_compact_request(state.clone()).await;
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/responses/compact")
    );
    assert_eq!(
        payload["xOpenAiPromptCacheKeyHeader"],
        "prompt-cache-oauth-compact"
    );
    assert_eq!(payload["clientTraceId"], "trace-oauth-compact");
    assert_eq!(payload["sessionIdHeader"], "session-oauth-compact");
    assert_eq!(
        payload["traceparentHeader"],
        "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"
    );
    assert_eq!(
        payload["xClientRequestIdHeader"],
        "client-request-oauth-compact"
    );
    assert_eq!(payload["xCodexTurnMetadataHeader"], "{\"compact\":true}");
    assert_eq!(payload["originatorHeader"], "Codex Desktop");
    let forwarded = payload["forwardedHeaderNames"]
        .as_array()
        .expect("forwarded headers");
    assert!(
        forwarded
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "session_id")
    );
    assert!(
        forwarded
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-client-request-id")
    );

    assert_oauth_compact_persistence(&state.pool, account_id).await;

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

async fn send_oauth_compact_request(state: Arc<AppState>) -> Value {
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses/compact".parse().expect("valid compact uri")),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-openai-prompt-cache-key"),
                HeaderValue::from_static("prompt-cache-oauth-compact"),
            ),
            (
                HeaderName::from_static("x-client-trace-id"),
                HeaderValue::from_static("trace-oauth-compact"),
            ),
            (
                HeaderName::from_static("session_id"),
                HeaderValue::from_static("session-oauth-compact"),
            ),
            (
                HeaderName::from_static("traceparent"),
                HeaderValue::from_static("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
            ),
            (
                HeaderName::from_static("x-client-request-id"),
                HeaderValue::from_static("client-request-oauth-compact"),
            ),
            (
                HeaderName::from_static("x-codex-turn-metadata"),
                HeaderValue::from_static("{\"compact\":true}"),
            ),
            (
                HeaderName::from_static("originator"),
                HeaderValue::from_static("Codex Desktop"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "compact me"}]
            }))
            .expect("serialize oauth compact body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read oauth compact response"),
    )
    .expect("decode oauth compact response")
}

async fn assert_oauth_compact_persistence(pool: &Pool<Sqlite>, account_id: i64) {
    wait_for_codex_invocations(pool, 1).await;
    let (payload,): (Option<String>,) = sqlx::query_as(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("load persisted invocation payload");
    let payload_json: Value = serde_json::from_str(
        payload
            .as_deref()
            .expect("payload should be persisted for oauth compact"),
    )
    .expect("decode persisted invocation payload");
    assert_eq!(payload_json["upstreamAccountId"].as_i64(), Some(account_id));
    assert_eq!(
        payload_json["endpoint"].as_str(),
        Some("/v1/responses/compact")
    );
    assert_eq!(payload_json["oauthFingerprintVersion"].as_str(), Some("v1"));
    assert_eq!(payload_json["oauthPromptCacheHeaderForwarded"], true);
    assert_eq!(payload_json["oauthResponsesRewrite"]["applied"], false);
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["session_id"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["traceparent"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["x-client-request-id"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["x-codex-turn-metadata"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert_eq!(
        payload_json["oauthForwardedHeaderFingerprints"]["originator"]
            .as_str()
            .map(str::len),
        Some(16)
    );
    assert!(
        payload_json["oauthRequestBodyPrefixBytes"]
            .as_u64()
            .expect("compact body prefix byte count")
            > 0
    );
    assert_eq!(
        payload_json["oauthRequestBodyPrefixFingerprint"]
            .as_str()
            .map(str::len),
        Some(16)
    );
}

#[tokio::test]
pub(crate) async fn pool_route_oauth_observability_omits_fingerprints_without_crypto_key() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_responses_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let oauth_response = oauth_bridge::send_oauth_upstream_request(
        &reqwest::Client::new(),
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([
            (
                HeaderName::from_static("session_id"),
                HeaderValue::from_static("session-no-crypto"),
            ),
            (
                HeaderName::from_static("traceparent"),
                HeaderValue::from_static("00-11111111111111111111111111111111-2222222222222222-01"),
            ),
        ]),
        oauth_bridge::OauthUpstreamRequestBody::Bytes(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello"
            }))
            .expect("serialize oauth responses body"),
        )),
        Duration::from_secs(5),
        Duration::from_secs(5),
        Some(7),
        "oauth-no-crypto",
        Some("02355c9d-fb23-4517-a96d-35e5f6758e9e"),
        None,
        None,
    )
    .await;

    assert_eq!(oauth_response.response.status(), StatusCode::OK);
    let body = to_bytes(oauth_response.response.into_body(), usize::MAX)
        .await
        .expect("read oauth response body");
    let payload_json: Value = serde_json::from_slice(&body).expect("decode oauth response body");
    assert_eq!(
        payload_json["path"].as_str(),
        Some("/backend-api/codex/responses")
    );

    let request_debug = oauth_response
        .request_debug
        .expect("oauth request debug should be present");
    assert!(request_debug.fingerprint_version.is_none());
    assert!(request_debug.forwarded_header_fingerprints.is_none());
    assert!(request_debug.request_body_prefix_fingerprint.is_none());
    assert!(request_debug.request_body_prefix_bytes.is_none());
    assert_eq!(request_debug.request_body_snapshot_kind, Some("memory"));
    assert_eq!(
        request_debug.responses_body_mode,
        Some("small_body_rewrite")
    );
    let serialized_debug = serde_json::to_string(&request_debug).expect("serialize request debug");
    assert!(
        !serialized_debug.contains("session-no-crypto"),
        "request debug should not leak raw header values"
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
pub(crate) async fn pool_route_large_oauth_responses_file_backed_body_rewrites_and_replaces_installation_id()
 {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_responses_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_oauth_account(&state, "Large Responses OAuth", "oauth-large").await;
    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(626262),
    });
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "max_output_tokens": 256,
        "client_metadata": {
            "x-codex-installation-id": "downstream-installation-id",
            "other": "keep-me",
        },
        "input": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 4096),
    }))
    .expect("serialize large oauth responses body");
    tokio::fs::write(&temp_file.path, &body)
        .await
        .expect("write large oauth responses body");

    let account = large_oauth_resolved_account(account_id);

    let upstream = send_pool_request_with_failover(PoolFailoverRequest {
        state,
        proxy_request_id: 626262,
        method: Method::POST,
        original_uri: &"/v1/responses".parse().expect("valid uri"),
        headers: &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        body: Some(PoolReplayBodySnapshot::File {
            temp_file,
            size: body.len(),
        }),
        handshake_timeout: Duration::from_secs(5),
        trace_context: None,
        runtime_snapshot_context: None,
        sticky_key: None,
        preferred_account: Some(account),
        failover_progress: PoolFailoverProgress::default(),
        same_account_attempts: 1,
    })
    .await
    .expect("large oauth responses body should stay on oauth route");

    assert_large_oauth_rewrite(upstream, account_id).await;

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

fn large_oauth_resolved_account(account_id: i64) -> PoolResolvedAccount {
    PoolResolvedAccount {
        account_id,
        display_name: "Large Responses OAuth".to_string(),
        kind: "oauth_codex".to_string(),
        auth: PoolResolvedAuth::Oauth {
            access_token: "oauth-large".to_string(),
            chatgpt_account_id: Some("org_test".to_string()),
        },
        upstream_base_url: oauth_bridge::oauth_codex_upstream_base_url()
            .expect("oauth upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        sticky_affinity_generation: None,
        routing_selection_audit: None,
        priority_handoff_permit: None,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        single_account_rotation_enabled: false,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
    }
}

async fn assert_large_oauth_rewrite(upstream: PoolUpstreamResponse, account_id: i64) {
    assert_eq!(upstream.account.account_id, account_id);
    assert_eq!(upstream.response.status(), StatusCode::OK);
    let mut response_body = upstream.first_chunk.clone().unwrap_or_default().to_vec();
    response_body.extend_from_slice(
        &upstream
            .response
            .into_bytes()
            .await
            .expect("read oauth passthrough response"),
    );
    let payload: Value =
        serde_json::from_slice(&response_body).expect("decode oauth passthrough response");
    assert_eq!(
        payload["received"]["stream"].as_bool(),
        Some(true),
        "responses requests should be normalized to streaming upstream even for file-backed bodies",
    );
    assert!(payload["received"].get("max_output_tokens").is_none());
    assert_eq!(payload["received"]["instructions"], "");
    assert_eq!(payload["received"]["store"], false);
    assert_eq!(payload["received"]["client_metadata"]["other"], "keep-me");
    let rewritten_installation_id =
        payload["received"]["client_metadata"]["x-codex-installation-id"]
            .as_str()
            .expect("rewritten installation id should be present");
    assert_ne!(rewritten_installation_id, "downstream-installation-id");
    assert_eq!(rewritten_installation_id.len(), 36);
    assert_eq!(rewritten_installation_id.chars().nth(8), Some('-'));
    assert_eq!(rewritten_installation_id.chars().nth(13), Some('-'));
    assert_eq!(rewritten_installation_id.chars().nth(18), Some('-'));
    assert_eq!(rewritten_installation_id.chars().nth(23), Some('-'));
    let request_debug = upstream
        .oauth_responses_debug
        .expect("oauth responses debug should be present");
    assert_eq!(
        request_debug.responses_body_mode,
        Some("small_body_rewrite")
    );
    assert_eq!(request_debug.request_body_snapshot_kind, Some("memory"));
    assert_eq!(
        request_debug.rewrite,
        oauth_bridge::OauthResponsesRewriteSummary {
            applied: true,
            added_instructions: true,
            added_store: true,
            forced_stream_true: true,
            removed_max_output_tokens: true,
            rewrote_installation_id: true,
            removed_installation_id: false,
        }
    );
}

#[tokio::test]
pub(crate) async fn oauth_responses_timeout_marks_transport_failure_header() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) =
        spawn_oauth_codex_delayed_headers_upstream(Duration::from_millis(250)).await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let oauth_response = oauth_bridge::send_oauth_upstream_request(
        &reqwest::Client::new(),
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        oauth_bridge::OauthUpstreamRequestBody::Bytes(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello"
            }))
            .expect("serialize oauth responses body"),
        )),
        Duration::from_millis(100),
        Duration::from_millis(120),
        Some(7),
        "oauth-timeout",
        Some("02355c9d-fb23-4517-a96d-35e5f6758e9e"),
        None,
        None,
    )
    .await;

    assert_eq!(oauth_response.response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        oauth_bridge::oauth_transport_failure_kind(oauth_response.response.headers()),
        Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT),
    );
    let body = to_bytes(oauth_response.response.into_body(), usize::MAX)
        .await
        .expect("read oauth timeout response body");
    let payload = String::from_utf8_lossy(&body);
    assert!(payload.contains("timed out"));

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
pub(crate) async fn pool_route_oauth_responses_timeout_switches_to_alternate_route() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;
    let (state, oauth_id, slow_upstream_handle, fast_upstream_handle) =
        setup_oauth_timeout_failover().await;

    send_oauth_timeout_failover_request(state.clone()).await;
    wait_for_codex_invocations(&state.pool, 1).await;
    assert_oauth_timeout_failover_persistence(&state.pool, oauth_id).await;

    slow_upstream_handle.abort();
    fast_upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

async fn setup_oauth_timeout_failover() -> (Arc<AppState>, i64, JoinHandle<()>, JoinHandle<()>) {
    let (slow_upstream_base, slow_upstream_handle) =
        spawn_oauth_codex_delayed_headers_upstream(Duration::from_millis(700)).await;
    let (fast_upstream_base, _attempts, fast_upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-fast", 0)]).await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{slow_upstream_base}/backend-api/codex"))
            .expect("valid oauth base url"),
    )
    .await;

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(450);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let oauth_id = insert_test_pool_oauth_account(&state, "Timeout OAuth", "oauth-timeout").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Fast Route",
        "route-fast",
        None,
        None,
        Some(fast_upstream_base.as_str()),
    )
    .await;
    let sticky_last_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-oauth-timeout-switch",
        oauth_id,
        &sticky_last_seen_at,
    )
    .await;

    (state, oauth_id, slow_upstream_handle, fast_upstream_handle)
}

async fn send_oauth_timeout_failover_request(state: Arc<AppState>) {
    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
        "stickyKey": "sticky-oauth-timeout-switch",
    }))
    .expect("serialize request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from(request_body),
        ),
    )
    .await
    .expect("oauth timeout failover request should not hang");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read oauth timeout failover body");
    let payload: Value = serde_json::from_slice(&body).expect("decode oauth timeout failover body");
    assert_eq!(payload["ok"].as_bool(), Some(true));
}

async fn assert_oauth_timeout_failover_persistence(pool: &Pool<Sqlite>, oauth_id: i64) {
    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("load oauth timeout failover payload");
    let persisted_payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("oauth timeout failover payload should be present"),
    )
    .expect("decode oauth timeout failover payload");
    assert_eq!(persisted_payload["poolAttemptCount"].as_i64(), Some(2));
    assert_eq!(
        persisted_payload["poolDistinctAccountCount"].as_i64(),
        Some(2)
    );
    assert!(persisted_payload["poolAttemptTerminalReason"].is_null());
    let oauth_route_state = sqlx::query_as::<_, RouteStateRow>(
        r#"
        SELECT
            last_action_reason_code,
            last_action_http_status,
            last_route_failure_kind
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(oauth_id)
    .fetch_one(pool)
    .await
    .expect("load oauth timeout route state");
    assert_eq!(
        oauth_route_state.last_action_reason_code.as_deref(),
        Some("transport_failure")
    );
    assert_eq!(oauth_route_state.last_action_http_status, Some(502));
    assert_eq!(
        oauth_route_state.last_route_failure_kind.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    let failed_attempt = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT
            status,
            http_status,
            downstream_http_status,
            failure_kind,
            error_message,
            downstream_error_message
        FROM pool_upstream_request_attempts
        WHERE invoke_id = (
            SELECT invoke_id
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
        )
          AND status != ?1
        ORDER BY id ASC
        LIMIT 1
        "#,
    )
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
    .fetch_one(pool)
    .await
    .expect("load oauth timeout failed attempt");
    assert_eq!(
        failed_attempt.status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(failed_attempt.http_status, None);
    assert_eq!(failed_attempt.downstream_http_status, Some(502));
    assert_eq!(
        failed_attempt.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
    );
    assert!(
        failed_attempt
            .error_message
            .as_deref()
            .is_some_and(|value| !value.contains("pool upstream responded with 502"))
    );
    assert!(
        failed_attempt
            .downstream_error_message
            .as_deref()
            .is_some_and(|value| value.contains("pool upstream responded with 502"))
    );
}

#[derive(sqlx::FromRow)]
struct PersistedPayloadRow {
    payload: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AttemptRow {
    status: String,
    http_status: Option<i64>,
    downstream_http_status: Option<i64>,
    failure_kind: Option<String>,
    error_message: Option<String>,
    downstream_error_message: Option<String>,
}

#[derive(sqlx::FromRow)]
struct RouteStateRow {
    last_action_reason_code: Option<String>,
    last_action_http_status: Option<i64>,
    last_route_failure_kind: Option<String>,
}

use super::*;
