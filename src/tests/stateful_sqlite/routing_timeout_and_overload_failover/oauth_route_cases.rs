use super::*;

#[tokio::test]
async fn pool_route_marks_oauth_missing_scopes_as_error_and_persists_upstream_details() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    #[derive(sqlx::FromRow)]
    struct RouteStateRow {
        status: String,
        last_error: Option<String>,
    }

    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        payload: Option<String>,
    }

    let scope_message = "You have insufficient permissions for this operation. Missing scopes: api.responses.write.";
    let (upstream_base, upstream_handle) = spawn_oauth_codex_http_failure(
        StatusCode::UNAUTHORIZED,
        Some("missing_scopes"),
        scope_message,
    )
    .await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_oauth_account(&state, "Scope OAuth", "oauth-scope").await;
    record_pool_route_success(
        &state.pool,
        account_id,
        Utc::now(),
        Some("sticky-scope-001"),
        None,
    )
    .await
    .expect("seed sticky route");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-scope-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure body");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|value| value.contains("Missing scopes: api.responses.write"))
    );

    let route_state = sqlx::query_as::<_, RouteStateRow>(
        r#"
        SELECT status, last_error
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load route state");
    assert_eq!(route_state.status, "error");
    assert!(
        route_state
            .last_error
            .as_deref()
            .is_some_and(|value| value.contains("Missing scopes: api.responses.write"))
    );
    assert!(
        load_test_sticky_route_account_id(&state.pool, "sticky-scope-001")
            .await
            .is_none(),
        "permission failures should detach the sticky binding",
    );
    let clear_event = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT action, routing_context_json, sticky_before_json
        FROM prompt_cache_conversation_operation_events
        WHERE prompt_cache_key = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind("sticky-scope-001")
    .fetch_one(&state.pool)
    .await
    .expect("automatic sticky clear event should be persisted");
    assert_eq!(clear_event.0, "stickyTargetCleared");
    assert!(
        clear_event
            .1
            .as_deref()
            .is_some_and(|value| value.contains("upstream_http_401"))
    );
    assert!(clear_event.2.is_some());

    wait_for_codex_invocations(&state.pool, 1).await;
    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load invocation payload");
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("payload should be persisted for failed capture"),
    )
    .expect("decode invocation payload");
    assert_eq!(payload_json["upstreamAccountId"].as_i64(), Some(account_id));
    assert_eq!(
        payload_json["upstreamErrorCode"].as_str(),
        Some("missing_scopes")
    );
    assert!(
        payload_json["upstreamErrorMessage"]
            .as_str()
            .is_some_and(|value| value.contains("Missing scopes: api.responses.write"))
    );
    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_marks_explicit_invalidated_oauth_as_needs_reauth() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_http_failure(
        StatusCode::FORBIDDEN,
        Some("token_invalidated"),
        "Authentication token has been invalidated, please sign in again.",
    )
    .await;
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
        insert_test_pool_oauth_account(&state, "Invalidated OAuth", "oauth-invalidated").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello"}"#.as_bytes().to_vec()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load oauth account status");
    assert_eq!(status, "needs_reauth");

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_marks_invalid_grant_error_code_as_needs_reauth() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_http_failure(
        StatusCode::UNAUTHORIZED,
        Some("invalid_grant"),
        "Unauthorized",
    )
    .await;
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
        insert_test_pool_oauth_account(&state, "Grant OAuth", "oauth-invalid-grant").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello"}"#.as_bytes().to_vec()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let status: String =
        sqlx::query_scalar("SELECT status FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load oauth account status");
    assert_eq!(status, "needs_reauth");

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_oauth_passthrough_replays_large_file_backed_body() {
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
    let account_id = insert_test_pool_oauth_account(&state, "Large OAuth", "oauth-large").await;

    let body = serde_json::to_vec(&json!({
        "messages": [{
            "role": "user",
            "content": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 4096),
        }],
    }))
    .expect("serialize large oauth passthrough body");
    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(424242),
    });
    tokio::fs::write(&temp_file.path, &body)
        .await
        .expect("write replay temp file");

    let account = PoolResolvedAccount {
        account_id,
        display_name: "Large OAuth".to_string(),
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
    };

    let upstream = send_pool_request_with_failover(
        state,
        424242,
        Method::POST,
        &"/v1/chat/completions".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::File {
            temp_file: temp_file.clone(),
            size: body.len(),
        }),
        Duration::from_secs(5),
        None,
        None,
        None,
        Some(account),
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect("oauth passthrough should succeed");

    assert_eq!(upstream.response.status(), StatusCode::OK);
    let mut response_body = upstream.first_chunk.clone().unwrap_or_default().to_vec();
    response_body.extend_from_slice(
        &upstream
            .response
            .into_bytes()
            .await
            .expect("read oauth passthrough response"),
    );
    let payload =
        serde_json::from_slice::<Value>(&response_body).expect("decode oauth passthrough response");
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/chat/completions")
    );
    assert_eq!(
        payload["authorization"].as_str(),
        Some("Bearer oauth-large")
    );
    assert_eq!(payload["chatgptAccountId"].as_str(), Some("org_test"));
    assert_eq!(payload["bodyLength"].as_u64(), Some(body.len() as u64));

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_oauth_responses_sends_uuid_account_header_and_persists_observability() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        payload: Option<String>,
    }

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
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_oauth_account_with_chatgpt_account_id(
        &state,
        "UUID OAuth",
        "oauth-uuid",
        "02355c9d-fb23-4517-a96d-35e5f6758e9e",
    )
    .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-openai-prompt-cache-key"),
                HeaderValue::from_static("prompt-cache-oauth-responses"),
            ),
            (
                HeaderName::from_static("x-client-trace-id"),
                HeaderValue::from_static("trace-oauth-responses"),
            ),
            (
                HeaderName::from_static("session_id"),
                HeaderValue::from_static("session-oauth-responses"),
            ),
            (
                HeaderName::from_static("traceparent"),
                HeaderValue::from_static("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00"),
            ),
            (
                HeaderName::from_static("x-client-request-id"),
                HeaderValue::from_static("client-request-oauth-responses"),
            ),
            (
                HeaderName::from_static("x-codex-turn-metadata"),
                HeaderValue::from_static("{\"turn\":42}"),
            ),
            (
                HeaderName::from_static("originator"),
                HeaderValue::from_static("Codex Desktop"),
            ),
            (
                HeaderName::from_static("chatgpt-account-id"),
                HeaderValue::from_static("client-should-not-win"),
            ),
        ]),
        Body::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello"
            }))
            .expect("serialize oauth responses body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read oauth response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode oauth response body");
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/responses")
    );
    assert_eq!(payload["authorization"].as_str(), Some("Bearer oauth-uuid"));
    assert_eq!(
        payload["chatgptAccountId"].as_str(),
        Some("02355c9d-fb23-4517-a96d-35e5f6758e9e")
    );
    assert_eq!(
        payload["xOpenAiPromptCacheKeyHeader"].as_str(),
        Some("prompt-cache-oauth-responses")
    );
    assert_eq!(
        payload["clientTraceId"].as_str(),
        Some("trace-oauth-responses")
    );
    assert_eq!(
        payload["sessionIdHeader"].as_str(),
        Some("session-oauth-responses")
    );
    assert_eq!(
        payload["traceparentHeader"].as_str(),
        Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00")
    );
    assert_eq!(
        payload["xClientRequestIdHeader"].as_str(),
        Some("client-request-oauth-responses")
    );
    assert_eq!(
        payload["xCodexTurnMetadataHeader"].as_str(),
        Some("{\"turn\":42}")
    );
    assert_eq!(payload["originatorHeader"].as_str(), Some("Codex Desktop"));
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-openai-prompt-cache-key")
    );
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-client-trace-id")
    );
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "session_id")
    );
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "traceparent")
    );
    assert!(
        payload["forwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-client-request-id")
    );
    assert_eq!(payload["received"]["stream"], true);
    assert_eq!(payload["received"]["store"], false);
    assert_eq!(payload["received"]["instructions"], "");

    wait_for_codex_invocations(&state.pool, 1).await;
    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load persisted invocation payload");
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("payload should be persisted for oauth responses"),
    )
    .expect("decode persisted invocation payload");
    assert_eq!(payload_json["upstreamAccountId"].as_i64(), Some(account_id));
    assert_eq!(payload_json["oauthAccountHeaderAttached"], true);
    assert_eq!(payload_json["oauthAccountIdShape"].as_str(), Some("uuid"));
    assert_eq!(payload_json["endpoint"].as_str(), Some("/v1/responses"));
    assert_eq!(payload_json["oauthFingerprintVersion"].as_str(), Some("v1"));
    assert_eq!(payload_json["oauthPromptCacheHeaderForwarded"], true);
    assert_eq!(
        payload_json["oauthRequestBodySnapshotKind"].as_str(),
        Some("memory")
    );
    assert_eq!(
        payload_json["oauthResponsesBodyMode"].as_str(),
        Some("small_body_rewrite")
    );
    assert!(
        payload_json["oauthForwardedHeaderCount"]
            .as_u64()
            .expect("forwarded header count")
            >= 2
    );
    assert!(
        payload_json["oauthForwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-openai-prompt-cache-key")
    );
    assert!(
        payload_json["oauthForwardedHeaderNames"]
            .as_array()
            .expect("forwarded header names")
            .iter()
            .filter_map(Value::as_str)
            .any(|name| name == "x-client-trace-id")
    );
    assert!(
        payload_json["oauthRequestBodyPrefixBytes"]
            .as_u64()
            .expect("body prefix byte count")
            > 0
    );
    assert!(
        payload_json["oauthRequestBodyPrefixFingerprint"]
            .as_str()
            .expect("body prefix fingerprint")
            .len()
            == 16
    );
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
    assert!(payload_json["oauthForwardedHeaderFingerprints"]["x-client-trace-id"].is_null());
    assert!(
        !row.payload
            .as_deref()
            .expect("persisted payload text")
            .contains("session-oauth-responses")
    );
    assert_eq!(payload_json["oauthResponsesRewrite"]["applied"], true);
    assert_eq!(
        payload_json["oauthResponsesRewrite"]["addedInstructions"],
        true
    );
    assert_eq!(payload_json["oauthResponsesRewrite"]["addedStore"], true);
    assert_eq!(
        payload_json["oauthResponsesRewrite"]["forcedStreamTrue"],
        true
    );
    assert_eq!(
        payload_json["oauthResponsesRewrite"]["removedMaxOutputTokens"],
        false
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}
