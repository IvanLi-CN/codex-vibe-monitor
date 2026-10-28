use super::*;
use serde_json::json;

#[tokio::test]
async fn pool_route_large_oauth_responses_file_backed_body_rewrites_and_replaces_installation_id() {
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
    let account = PoolResolvedAccount {
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
        routing_control_fingerprint: None,
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
        oauth_request_compression_algorithm: RequestCompressionAlgorithm::Zstd,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
    };

    let upstream = send_pool_request_with_failover(
        state,
        626262,
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
    .expect("large oauth responses body should stay on oauth route");

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

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_oauth_responses_file_backed_body_above_rewrite_limit_stays_passthrough() {
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
    let account = PoolResolvedAccount {
        account_id,
        display_name: "Responses OAuth".to_string(),
        kind: "oauth_codex".to_string(),
        auth: PoolResolvedAuth::Oauth {
            access_token: "oauth-responses".to_string(),
            chatgpt_account_id: Some("org_test".to_string()),
        },
        upstream_base_url: oauth_bridge::oauth_codex_upstream_base_url()
            .expect("oauth upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        routing_control_fingerprint: None,
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
        oauth_request_compression_algorithm: RequestCompressionAlgorithm::Zstd,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
    };

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
    let request_debug = upstream
        .oauth_responses_debug
        .expect("oauth responses debug should be present");
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
async fn pool_route_oauth_responses_compressed_file_backed_body_stays_passthrough() {
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
    let account = PoolResolvedAccount {
        account_id,
        display_name: "Compressed OAuth".to_string(),
        kind: "oauth_codex".to_string(),
        auth: PoolResolvedAuth::Oauth {
            access_token: "oauth-compressed".to_string(),
            chatgpt_account_id: Some("org_test".to_string()),
        },
        upstream_base_url: oauth_bridge::oauth_codex_upstream_base_url()
            .expect("oauth upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        routing_control_fingerprint: None,
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
        oauth_request_compression_algorithm: RequestCompressionAlgorithm::Zstd,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
    };

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
    let request_debug = upstream
        .oauth_responses_debug
        .expect("oauth responses debug should be present");
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
async fn analyze_large_file_backed_replay_snapshot_once_for_pool_routing() {
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
async fn replay_route_analysis_preserves_sticky_projection_type_semantics() {
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
