#[tokio::test]
pub(crate) async fn pool_route_oauth_compact_stream_timeout_does_not_cap_send_phase_before_first_byte()
 {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(600);
    config.pool_upstream_responses_total_timeout = Duration::from_millis(300);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_oauth_account(&state, "Compact OAuth", "oauth-compact").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses/compact?mode=delay"
                .parse()
                .expect("valid compact uri"),
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
        Body::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "compact me slowly"}]
            }))
            .expect("serialize oauth compact timeout body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read oauth compact success response"),
    )
    .expect("decode oauth compact success response");
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/responses/compact"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    let failure_kind: Option<String> = sqlx::query_scalar(
        r#"
        SELECT json_extract(payload, '$.failureKind')
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load oauth compact success failure kind");
    assert_eq!(failure_kind, None);

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_route_oauth_responses_file_backed_body_above_rewrite_limit_stays_passthrough()
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
        insert_test_pool_oauth_account(&state, "Responses OAuth", "oauth-responses").await;
    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(515151),
    });
    let input = "x".repeat(OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES + 256);
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "max_output_tokens": 256,
        "client_metadata": {
            "x-codex-installation-id": "downstream-installation-id",
        },
        "input": input,
    }))
    .expect("serialize oversized oauth responses body");
    tokio::fs::write(&temp_file.path, &body)
        .await
        .expect("write oversized oauth responses body");

    let account = oauth_passthrough_account(account_id, "Responses OAuth", "oauth-responses");

    let upstream = send_pool_request_with_failover(
        state,
        515151,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::File {
            temp_file,
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
    .expect("oversized oauth responses body should stay on oauth route");

    let (observed_account_id, payload, request_debug) = decode_oauth_passthrough(upstream).await;
    assert_eq!(observed_account_id, account_id);
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/responses")
    );
    assert_eq!(payload["received"]["stream"].as_bool(), Some(false));
    assert_eq!(payload["received"]["max_output_tokens"].as_u64(), Some(256));
    assert!(payload["received"].get("instructions").is_none());
    assert!(payload["received"].get("store").is_none());
    assert_eq!(
        payload["received"]["input"].as_str().map(str::len),
        Some(OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES + 256)
    );
    assert_eq!(
        payload["received"]["client_metadata"]["x-codex-installation-id"].as_str(),
        Some("downstream-installation-id")
    );
    assert_eq!(request_debug.request_body_snapshot_kind, Some("file"));
    assert_eq!(
        request_debug.responses_body_mode,
        Some("large_body_passthrough")
    );
    assert_eq!(
        request_debug.rewrite,
        oauth_bridge::OauthResponsesRewriteSummary::default()
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
pub(crate) async fn pool_route_oauth_responses_compressed_file_backed_body_stays_passthrough() {
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
        insert_test_pool_oauth_account(&state, "Compressed OAuth", "oauth-compressed").await;
    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(525252),
    });
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "max_output_tokens": 64,
        "client_metadata": {
            "x-codex-installation-id": "compressed-downstream-installation-id",
        },
        "input": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 2048),
    }))
    .expect("serialize compressed oauth responses body");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, &body).expect("write compressed oauth responses body");
    let compressed_body = encoder
        .finish()
        .expect("finish compressed oauth responses body");
    tokio::fs::write(&temp_file.path, &compressed_body)
        .await
        .expect("write compressed oauth responses body");

    let account = oauth_passthrough_account(account_id, "Compressed OAuth", "oauth-compressed");

    let upstream = send_pool_request_with_failover(
        state,
        525252,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                http_header::CONTENT_ENCODING,
                HeaderValue::from_static("gzip"),
            ),
        ]),
        Some(PoolReplayBodySnapshot::File {
            temp_file,
            size: compressed_body.len(),
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
    .expect("compressed oauth responses body should stay on oauth route");

    let (observed_account_id, payload, request_debug) = decode_oauth_passthrough(upstream).await;
    assert_eq!(observed_account_id, account_id);
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/responses")
    );
    assert_eq!(payload["contentEncodingHeader"].as_str(), Some("gzip"));
    assert_eq!(payload["received"]["stream"].as_bool(), Some(false));
    assert_eq!(payload["received"]["max_output_tokens"].as_u64(), Some(64));
    assert_eq!(
        payload["received"]["client_metadata"]["x-codex-installation-id"].as_str(),
        Some("compressed-downstream-installation-id")
    );
    assert_eq!(request_debug.request_body_snapshot_kind, Some("file"));
    assert_eq!(
        request_debug.responses_body_mode,
        Some("large_body_passthrough")
    );
    assert_eq!(
        request_debug.rewrite,
        oauth_bridge::OauthResponsesRewriteSummary::default()
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

fn oauth_passthrough_account(
    account_id: i64,
    display_name: &str,
    access_token: &str,
) -> PoolResolvedAccount {
    PoolResolvedAccount {
        account_id,
        display_name: display_name.to_string(),
        kind: "oauth_codex".to_string(),
        auth: PoolResolvedAuth::Oauth {
            access_token: access_token.to_string(),
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

async fn decode_oauth_passthrough(
    upstream: PoolUpstreamResponse,
) -> (i64, Value, oauth_bridge::OauthResponsesDebugInfo) {
    assert_eq!(upstream.response.status(), StatusCode::OK);
    let account_id = upstream.account.account_id;
    let debug = upstream
        .oauth_responses_debug
        .expect("oauth responses debug");
    let mut body = upstream.first_chunk.unwrap_or_default().to_vec();
    body.extend_from_slice(
        &upstream
            .response
            .into_bytes()
            .await
            .expect("read oauth response"),
    );
    let payload = serde_json::from_slice(&body).expect("decode oauth response");
    (account_id, payload, debug)
}

#[tokio::test]
pub(crate) async fn analyze_large_file_backed_replay_snapshot_once_for_pool_routing() {
    let request_body = serde_json::to_vec(&json!({
        "stickyKey": "sticky-large-file",
        "metadata": {},
        "promptCacheKey": "cache-large-file",
        "model": "gpt-5.5",
        "input": [
            { "type": "encrypted_content" },
            "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 128),
        ],
    }))
    .expect("serialize sticky replay body");
    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(737373),
    });
    tokio::fs::write(&temp_file.path, &request_body)
        .await
        .expect("write sticky replay temp file");
    let snapshot = PoolReplayBodySnapshot::File {
        temp_file,
        size: request_body.len(),
    };

    let analysis = analyze_replay_snapshot_for_pool_routing(
        &snapshot,
        Some(ProxyCaptureTarget::Responses),
        737373,
        "test",
    )
    .await;

    assert_eq!(analysis.sticky_key.as_deref(), Some("sticky-large-file"));
    assert_eq!(
        analysis.prompt_cache_key.as_deref(),
        Some("cache-large-file")
    );
    assert_eq!(analysis.requested_model.as_deref(), Some("gpt-5.5"));
    assert!(analysis.contains_encrypted_content);
    assert_eq!(analysis.file_read_count, 1);
    assert_eq!(analysis.json_parse_count, 1);
    assert_eq!(analysis.parse_outcome, "parsed");
}

#[tokio::test]
pub(crate) async fn replay_route_analysis_preserves_sticky_projection_type_semantics() {
    for request_body in [
        br#"{"metadata":{"stickyKey":"metadata-sticky"},"stickyKey":"body-sticky"}"#.as_slice(),
        br#"{"stickyKey":"  ","promptCacheKey":"fallback-sticky"}"#.as_slice(),
        br#"{"stickyKey":42,"promptCacheKey":"must-not-route"}"#.as_slice(),
        br#"{"stickyKey":"must-not-route","promptCacheKey":42}"#.as_slice(),
        br#"{"metadata":null,"stickyKey":"must-not-route"}"#.as_slice(),
        br#"{"stickyKey":"owner-a","stickyKey":"owner-b"}"#.as_slice(),
        br#"{"metadata":{"stickyKey":"owner-a","stickyKey":"owner-b"}}"#.as_slice(),
    ] {
        let snapshot = PoolReplayBodySnapshot::Memory(Bytes::copy_from_slice(request_body));
        let analysis = analyze_replay_snapshot_for_pool_routing(
            &snapshot,
            Some(ProxyCaptureTarget::Responses),
            737374,
            "test",
        )
        .await;

        assert_eq!(
            analysis.sticky_key,
            extract_sticky_key_from_request_body_projection(request_body)
        );
    }
}

#[test]
pub(crate) fn summarize_pool_upstream_http_failure_ignores_html_bodies() {
    let (code, message, request_id, summary) = summarize_pool_upstream_http_failure(
        StatusCode::UNAUTHORIZED,
        None,
        b"<html>blocked</html>",
    );
    assert_eq!(code, None);
    assert_eq!(message, None);
    assert_eq!(request_id, None);
    assert_eq!(summary, "pool upstream responded with 401");
}

#[test]
pub(crate) fn summarize_pool_upstream_http_failure_prefers_request_id_header() {
    let body = br#"{"error":{"message":"Missing scopes: api.responses.write","code":"insufficient_permissions"}}"#;
    let (code, message, request_id, summary) =
        summarize_pool_upstream_http_failure(StatusCode::FORBIDDEN, Some("req_123abc"), body);
    assert_eq!(code.as_deref(), Some("insufficient_permissions"));
    assert_eq!(
        message.as_deref(),
        Some("Missing scopes: api.responses.write")
    );
    assert_eq!(request_id.as_deref(), Some("req_123abc"));
    assert_eq!(
        summary,
        "pool upstream responded with 403: Missing scopes: api.responses.write"
    );
}

#[test]
pub(crate) fn summarize_pool_upstream_http_failure_keeps_plaintext_reauth_signal() {
    let (code, message, request_id, summary) = summarize_pool_upstream_http_failure(
        StatusCode::UNAUTHORIZED,
        None,
        b"invalid_grant: please sign in again",
    );
    assert_eq!(code, None);
    assert_eq!(
        message.as_deref(),
        Some("invalid_grant: please sign in again")
    );
    assert_eq!(request_id, None);
    assert_eq!(
        summary,
        "pool upstream responded with 401: invalid_grant: please sign in again"
    );
}

#[test]
pub(crate) fn summarize_pool_upstream_http_failure_reads_nested_request_id() {
    let body = br#"{"response":{"error":{"message":"Request failed","code":"bad_request","request_id":"req_nested_123"}}}"#;
    let (code, message, request_id, summary) =
        summarize_pool_upstream_http_failure(StatusCode::BAD_REQUEST, None, body);
    assert_eq!(code.as_deref(), Some("bad_request"));
    assert_eq!(message.as_deref(), Some("Request failed"));
    assert_eq!(request_id.as_deref(), Some("req_nested_123"));
    assert_eq!(summary, "pool upstream responded with 400: Request failed");
}

#[test]
pub(crate) fn summarize_pool_upstream_http_failure_reads_top_level_error_fields() {
    let body =
        br#"{"message":"Gateway says no","code":"gateway_forbidden","request_id":"req_top_456"}"#;
    let (code, message, request_id, summary) =
        summarize_pool_upstream_http_failure(StatusCode::FORBIDDEN, None, body);
    assert_eq!(code.as_deref(), Some("gateway_forbidden"));
    assert_eq!(message.as_deref(), Some("Gateway says no"));
    assert_eq!(request_id.as_deref(), Some("req_top_456"));
    assert_eq!(summary, "pool upstream responded with 403: Gateway says no");
}

#[test]
pub(crate) fn pool_route_uses_account_specific_upstream_base_url() {
    run_oauth_future_with_large_stack(async move {
        let (global_upstream_base, global_upstream_handle) = spawn_test_upstream().await;
        let (account_upstream_base, account_upstream_handle) =
            spawn_test_upstream_with_prefix("/gateway").await;
        let state = test_state_with_openai_base(
            Url::parse(&global_upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Gateway Key",
            "upstream-primary",
            None,
            None,
            Some(&account_upstream_base),
        )
        .await;

        let echo_response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/echo?from=pool".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    http_header::ACCEPT_ENCODING,
                    HeaderValue::from_static("gzip, br"),
                ),
            ]),
            Body::from(br#"{"model":"gpt-5","input":"hello"}"#.to_vec()),
        )
        .await;

        assert_eq!(echo_response.status(), StatusCode::CREATED);
        let echo_body = to_bytes(echo_response.into_body(), usize::MAX)
            .await
            .expect("read echo response body");
        let echo_payload: Value = serde_json::from_slice(&echo_body).expect("decode echo payload");
        assert_eq!(
            echo_payload["path"].as_str(),
            Some("/gateway/v1/echo"),
            "the request should be routed through the account-specific upstream base path",
        );
        assert_eq!(
            echo_payload["authorization"].as_str(),
            Some("Bearer upstream-primary")
        );
        assert_eq!(echo_payload["acceptEncoding"].as_str(), Some("gzip, br"));

        global_upstream_handle.abort();
        account_upstream_handle.abort();
    });
}

#[test]
pub(crate) fn pool_route_honors_existing_body_sticky_binding_for_non_capture_requests() {
    run_oauth_future_with_large_stack(async move {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let _primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        let secondary_id =
            insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
        record_pool_route_success(
            &state.pool,
            secondary_id,
            Utc::now(),
            Some("sticky-body-001"),
            None,
        )
        .await
        .expect("seed sticky route");
        let request_body =
            br#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-body-001"}"#.to_vec();

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/echo?sticky=body".parse().expect("valid uri")),
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
                (
                    http_header::CONTENT_LENGTH,
                    HeaderValue::from_str(&request_body.len().to_string())
                        .expect("valid content length"),
                ),
            ]),
            Body::from(request_body),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
        assert_eq!(
            payload["authorization"].as_str(),
            Some("Bearer upstream-secondary")
        );

        upstream_handle.abort();
    });
}

#[test]
pub(crate) fn capture_target_pool_route_prefers_account_upstream_base_for_redirect_rewrite() {
    let global = Url::parse("https://api.openai.com/").expect("global upstream base url");
    let account = PoolResolvedAccount {
        account_id: 8,
        display_name: "Gateway Key".to_string(),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer upstream-primary".to_string(),
        },
        upstream_base_url: Url::parse("https://proxy.example.com/gateway")
            .expect("account upstream base url"),
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

    assert_eq!(
        location_rewrite_upstream_base(Some(&account), &global).as_str(),
        "https://proxy.example.com/gateway"
    );
    assert_eq!(
        location_rewrite_upstream_base(None, &global).as_str(),
        "https://api.openai.com/"
    );
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_keeps_unclassified_response_failed_stream_diagnostic_only()
 {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    record_pool_route_success(
        &state.pool,
        account_id,
        Utc::now(),
        Some("sticky-cap-logical"),
        None,
    )
    .await
    .expect("seed sticky route");

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
        "stickyKey": "sticky-cap-logical"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=response_failed"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");

    wait_for_codex_invocations(&state.pool, 1).await;
    assert_unclassified_failure_route_state(&state.pool, account_id).await;

    upstream_handle.abort();
}

#[derive(sqlx::FromRow)]
struct UnclassifiedRouteStateRow {
    status: String,
    last_error: Option<String>,
    consecutive_route_failures: i64,
}

async fn assert_unclassified_failure_route_state(pool: &Pool<Sqlite>, account_id: i64) {
    let route_state = sqlx::query_as::<_, UnclassifiedRouteStateRow>(
        r#"
        SELECT status, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load route state");
    assert_eq!(route_state.status, "active");
    assert_eq!(route_state.consecutive_route_failures, 0);
    assert!(route_state.last_error.is_none());
    let model_route = sqlx::query_as::<_, (String, String, i64, Option<String>)>(
        "SELECT state, priority, consecutive_failures, last_failure_kind FROM pool_upstream_account_model_routes WHERE account_id = ?1 AND model = 'gpt-5.4'",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load logical failure model route");
    let latest_event = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT action, reason_code, model FROM pool_upstream_account_events WHERE account_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(account_id)
    .fetch_one(pool)
    .await
    .expect("load logical failure event");
    assert_eq!(model_route.0, MODEL_ROUTE_STATE_AVAILABLE);
    assert_eq!(model_route.1, MODEL_ROUTE_PRIORITY_NORMAL);
    assert_eq!(model_route.2, 0);
    assert!(model_route.3.is_none());
    assert_eq!(
        latest_event.0,
        UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED
    );
    assert_eq!(
        latest_event.1.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR)
    );
    assert!(latest_event.2.is_none());
    assert!(
        load_test_sticky_route_account_id(pool, "sticky-cap-logical")
            .await
            .is_some_and(|sticky_account_id| sticky_account_id == account_id),
        "logical stream failure should preserve sticky binding until cooldown begins",
    );
}

#[derive(sqlx::FromRow)]
pub(crate) struct LatestInvocationPayloadRow {
    pub(crate) invoke_id: String,
    pub(crate) status: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) payload: Option<String>,
}

pub(crate) async fn load_latest_invocation_payload_row(
    state: &AppState,
) -> (LatestInvocationPayloadRow, Value) {
    let row = sqlx::query_as::<_, LatestInvocationPayloadRow>(
        r#"
        SELECT invoke_id, status, error_message, failure_kind, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load latest invocation");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("latest invocation payload should exist"),
    )
    .expect("decode latest invocation payload");
    (row, payload)
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_keeps_late_logical_failure_when_downstream_disconnects()
 {
    let (upstream_base, _attempts, upstream_handle) =
        spawn_pool_late_response_failed_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    record_pool_route_success(
        &state.pool,
        account_id,
        Utc::now(),
        Some("sticky-cap-disconnect"),
        None,
    )
    .await
    .expect("seed sticky route");

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
        "stickyKey": "sticky-cap-disconnect"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    drop(response);

    wait_for_codex_invocations(&state.pool, 1).await;

    assert_late_logical_failure_invocation(&state.pool).await;
    assert_late_logical_failure_attempt(&state.pool).await;

    upstream_handle.abort();
}

#[derive(sqlx::FromRow)]
struct LateLogicalInvocationRow {
    invoke_id: String,
    occurred_at: String,
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    payload: Option<String>,
}

async fn assert_late_logical_failure_invocation(pool: &Pool<Sqlite>) {
    let invocation = sqlx::query_as::<_, LateLogicalInvocationRow>(
        r#"
        SELECT invoke_id, occurred_at, status, error_message, failure_kind, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("load latest invocation");
    assert_eq!(invocation.status.as_deref(), Some("http_200"));
    assert_eq!(
        invocation.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    );
    assert!(
        invocation
            .error_message
            .as_deref()
            .is_some_and(|value| value.contains("upstream_response_failed"))
    );
    let invocation_payload: Value = serde_json::from_str(
        invocation
            .payload
            .as_deref()
            .expect("latest invocation payload should exist"),
    )
    .expect("decode latest invocation payload");
    assert_eq!(
        invocation_payload["downstreamStatusCode"].as_i64(),
        Some(200)
    );
    assert!(
        invocation_payload["downstreamErrorMessage"]
            .as_str()
            .is_some_and(
                |value| value.contains("downstream closed while streaming upstream response")
            )
    );
    let reloaded =
        load_persisted_api_invocation(pool, &invocation.invoke_id, &invocation.occurred_at)
            .await
            .expect("reload persisted api invocation");
    assert_eq!(reloaded.downstream_status_code, Some(200));
    assert!(
        reloaded.downstream_error_message.as_deref().is_some_and(
            |value| value.contains("downstream closed while streaming upstream response")
        )
    );
}

#[derive(sqlx::FromRow)]
struct LateLogicalAttemptRow {
    status: String,
    downstream_http_status: Option<i64>,
    failure_kind: Option<String>,
    error_message: Option<String>,
    downstream_error_message: Option<String>,
}

async fn assert_late_logical_failure_attempt(pool: &Pool<Sqlite>) {
    let attempt = sqlx::query_as::<_, LateLogicalAttemptRow>(
        r#"
        SELECT status, downstream_http_status, failure_kind, error_message, downstream_error_message
        FROM pool_upstream_request_attempts
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("load latest pool attempt");
    assert_eq!(
        attempt.status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(attempt.downstream_http_status, Some(200));
    assert_eq!(
        attempt.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    );
    assert!(
        attempt
            .error_message
            .as_deref()
            .is_some_and(|value| value.contains("upstream_response_failed"))
    );
    assert!(
        attempt.downstream_error_message.as_deref().is_some_and(
            |value| value.contains("downstream closed while streaming upstream response")
        )
    );
}

use super::*;
