#[tokio::test]
async fn oauth_streaming_passthrough_backfills_body_prefix_from_replay_status() {
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
async fn pool_route_oauth_body_sticky_binding_applies_before_first_send() {
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
async fn pool_route_oauth_compact_passthrough_preserves_prompt_cache_headers() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
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
    let account_id = insert_test_pool_oauth_account(&state, "Compact OAuth", "oauth-compact").await;

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
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read oauth compact response"),
    )
    .expect("decode oauth compact response");
    assert_eq!(
        payload["path"].as_str(),
        Some("/backend-api/codex/responses/compact")
    );
    assert_eq!(
        payload["xOpenAiPromptCacheKeyHeader"].as_str(),
        Some("prompt-cache-oauth-compact")
    );
    assert_eq!(
        payload["clientTraceId"].as_str(),
        Some("trace-oauth-compact")
    );
    assert_eq!(
        payload["sessionIdHeader"].as_str(),
        Some("session-oauth-compact")
    );
    assert_eq!(
        payload["traceparentHeader"].as_str(),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")
    );
    assert_eq!(
        payload["xClientRequestIdHeader"].as_str(),
        Some("client-request-oauth-compact")
    );
    assert_eq!(
        payload["xCodexTurnMetadataHeader"].as_str(),
        Some("{\"compact\":true}")
    );
    assert_eq!(payload["originatorHeader"].as_str(), Some("Codex Desktop"));
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
            .any(|name| name == "x-client-request-id")
    );

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
    assert!(
        payload_json["oauthRequestBodyPrefixFingerprint"]
            .as_str()
            .expect("compact body fingerprint")
            .len()
            == 16
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_oauth_observability_omits_fingerprints_without_crypto_key() {
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
    let serialized_debug = serde_json::to_string(&request_debug).expect("serialize request debug");
    assert!(
        !serialized_debug.contains("session-no-crypto"),
        "request debug should not leak raw header values"
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn oauth_responses_timeout_marks_transport_failure_header() {
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
async fn pool_route_oauth_responses_timeout_switches_to_alternate_route() {
    #[derive(sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (slow_upstream_base, slow_upstream_handle) =
        spawn_oauth_codex_delayed_headers_upstream(Duration::from_millis(250)).await;
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
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(120);
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

    wait_for_codex_invocations(&state.pool, 1).await;

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
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

    slow_upstream_handle.abort();
    fast_upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn pool_route_large_oauth_responses_falls_back_to_api_key_account() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let oauth_id =
        insert_test_pool_oauth_account(&state, "Oversized Responses OAuth", "oauth-oversized")
            .await;
    let api_key_id =
        insert_test_pool_api_key_account(&state, "Fallback API Key", "upstream-fallback").await;
    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(626262),
    });
    let body = vec![b'x'; OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES + 1];
    tokio::fs::write(&temp_file.path, &body)
        .await
        .expect("write oversized oauth responses body");

    let upstream = send_pool_request_with_failover(
        state.clone(),
        626262,
        Method::POST,
        &"/v1/responses?mode=delay".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::File {
            temp_file,
            size: body.len(),
            sticky_key: None,
        }),
        Duration::from_secs(5),
        None,
        None,
        None,
        None,
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect("api key account should handle oversized oauth responses body");

    assert_eq!(upstream.account.account_id, api_key_id);
    let selected_at: Vec<(i64, Option<String>)> = sqlx::query_as(
        r#"
        SELECT id, last_selected_at
        FROM pool_upstream_accounts
        WHERE id IN (?1, ?2)
        ORDER BY id
        "#,
    )
    .bind(oauth_id)
    .bind(api_key_id)
    .fetch_all(&state.pool)
    .await
    .expect("load selected timestamps");
    assert_eq!(selected_at.len(), 2);
    assert_eq!(selected_at[0], (oauth_id, None));
    assert!(selected_at[1].1.is_some());

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_oauth_compact_stream_timeout_does_not_cap_send_phase_before_first_byte() {
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
async fn pool_route_oauth_responses_rejects_large_file_backed_rewrite_body() {
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
    let account_id =
        insert_test_pool_oauth_account(&state, "Responses OAuth", "oauth-responses").await;
    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(515151),
    });
    let body = vec![b'x'; OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES + 1];
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
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        group_upstream_429_retry_enabled: false,
        group_upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
    };

    let err = send_pool_request_with_failover(
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
            sticky_key: None,
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
    .expect_err("oversized oauth responses body should be rejected");

    assert_eq!(err.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        err.message
            .contains("oauth /v1/responses request body exceeds"),
        "unexpected oversized oauth responses error: {}",
        err.message
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn extract_sticky_key_from_large_file_backed_replay_snapshot() {
    let temp_dir = make_temp_test_dir("sticky-large-replay");
    let sticky_body = serde_json::to_vec(&json!({
        "stickyKey": "sticky-large-file",
        "input": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 128),
    }))
    .expect("serialize sticky replay body");
    let temp_file = Arc::new(PoolReplayTempFile {
        path: temp_dir.join("sticky-large-file.bin"),
    });
    tokio::fs::write(&temp_file.path, &sticky_body)
        .await
        .expect("write sticky replay temp file");
    let snapshot = PoolReplayBodySnapshot::File {
        temp_file,
        size: sticky_body.len(),
        sticky_key: Some("sticky-large-file".to_string()),
    };

    assert_eq!(
        extract_sticky_key_from_replay_snapshot(&snapshot)
            .await
            .as_deref(),
        Some("sticky-large-file")
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn extract_sticky_key_from_large_file_backed_replay_snapshot_prefix() {
    let temp_dir = make_temp_test_dir("sticky-prefix-large-replay");
    let sticky_body = format!(
        r#"{{"stickyKey":"sticky-large-prefix","input":"{}"}}"#,
        "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 128)
    )
    .into_bytes();
    let temp_file = Arc::new(PoolReplayTempFile {
        path: temp_dir.join("sticky-large-prefix.bin"),
    });
    tokio::fs::write(&temp_file.path, &sticky_body)
        .await
        .expect("write sticky replay temp file");
    let snapshot = PoolReplayBodySnapshot::File {
        temp_file,
        size: sticky_body.len(),
        sticky_key: None,
    };

    assert_eq!(
        extract_sticky_key_from_replay_snapshot_prefix(&snapshot)
            .await
            .as_deref(),
        Some("sticky-large-prefix")
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn summarize_pool_upstream_http_failure_ignores_html_bodies() {
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
fn summarize_pool_upstream_http_failure_prefers_request_id_header() {
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
fn summarize_pool_upstream_http_failure_keeps_plaintext_reauth_signal() {
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
fn summarize_pool_upstream_http_failure_reads_nested_request_id() {
    let body = br#"{"response":{"error":{"message":"Request failed","code":"bad_request","request_id":"req_nested_123"}}}"#;
    let (code, message, request_id, summary) =
        summarize_pool_upstream_http_failure(StatusCode::BAD_REQUEST, None, body);
    assert_eq!(code.as_deref(), Some("bad_request"));
    assert_eq!(message.as_deref(), Some("Request failed"));
    assert_eq!(request_id.as_deref(), Some("req_nested_123"));
    assert_eq!(summary, "pool upstream responded with 400: Request failed");
}

#[test]
fn summarize_pool_upstream_http_failure_reads_top_level_error_fields() {
    let body =
        br#"{"message":"Gateway says no","code":"gateway_forbidden","request_id":"req_top_456"}"#;
    let (code, message, request_id, summary) =
        summarize_pool_upstream_http_failure(StatusCode::FORBIDDEN, None, body);
    assert_eq!(code.as_deref(), Some("gateway_forbidden"));
    assert_eq!(message.as_deref(), Some("Gateway says no"));
    assert_eq!(request_id.as_deref(), Some("req_top_456"));
    assert_eq!(summary, "pool upstream responded with 403: Gateway says no");
}

#[tokio::test]
async fn pool_route_uses_account_specific_upstream_base_url() {
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
}

#[tokio::test]
async fn pool_route_honors_existing_body_sticky_binding_for_non_capture_requests() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let _primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
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
}

#[test]
fn capture_target_pool_route_prefers_account_upstream_base_for_redirect_rewrite() {
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
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        group_upstream_429_retry_enabled: false,
        group_upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
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
async fn capture_target_pool_route_marks_response_failed_stream_as_route_failure() {
    #[derive(sqlx::FromRow)]
    struct RouteStateRow {
        status: String,
        last_error: Option<String>,
        consecutive_route_failures: i64,
    }

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
    let route_state = sqlx::query_as::<_, RouteStateRow>(
        r#"
        SELECT status, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load route state");
    assert_eq!(route_state.status, "active");
    assert_eq!(route_state.consecutive_route_failures, 1);
    assert!(
        route_state
            .last_error
            .as_deref()
            .is_some_and(|value| value.contains("upstream_response_failed"))
    );
    assert!(
        load_test_sticky_route_account_id(&state.pool, "sticky-cap-logical")
            .await
            .is_some_and(|sticky_account_id| sticky_account_id == account_id),
        "logical stream failure should preserve sticky binding until cooldown begins",
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_marks_server_overloaded_after_forward_as_retryable_without_cooldown()
 {
    #[derive(sqlx::FromRow)]
    struct RouteStateRow {
        status: String,
        last_action: Option<String>,
        last_action_reason_code: Option<String>,
        last_action_http_status: Option<i64>,
        cooldown_until: Option<String>,
        last_route_failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_late_response_failed_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "upstream-primary",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;
    record_pool_route_success(
        &state.pool,
        account_id,
        Utc::now(),
        Some("sticky-cap-overloaded-late"),
        None,
    )
    .await
    .expect("seed sticky route");

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
        "stickyKey": "sticky-cap-overloaded-late"
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
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read late overloaded response body");
    let body_text = String::from_utf8(body.to_vec()).expect("utf8 late overloaded body");
    assert!(body_text.contains("response.created"));
    assert!(body_text.contains("server_is_overloaded"));

    wait_for_codex_invocations(&state.pool, 1).await;
    let route_state = sqlx::query_as::<_, RouteStateRow>(
        r#"
        SELECT
            status,
            last_action,
            last_action_reason_code,
            last_action_http_status,
            cooldown_until,
            last_route_failure_kind
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load late overloaded route state");
    assert_eq!(route_state.status, "active");
    assert_eq!(
        route_state.last_action.as_deref(),
        Some("route_retryable_failure")
    );
    assert_eq!(
        route_state.last_action_reason_code.as_deref(),
        Some("upstream_server_overloaded")
    );
    assert_eq!(route_state.last_action_http_status, Some(200));
    assert!(route_state.cooldown_until.is_none());
    assert_eq!(
        route_state.last_route_failure_kind.as_deref(),
        Some("upstream_response_failed")
    );
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-cap-overloaded-late").await,
        Some(account_id),
        "retryable overload should keep the sticky binding",
    );

    let recent_actions: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT action
        FROM pool_upstream_account_events
        WHERE account_id = ?1
        ORDER BY id DESC
        LIMIT 5
        "#,
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load late overloaded events");
    assert!(
        !recent_actions
            .iter()
            .any(|action| action == "route_cooldown_started")
    );

    let attempts = attempts.lock().expect("lock late overloaded attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    drop(attempts);

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_openai_v1_e2e_stream_survives_short_request_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
        proxy_raw_async_semaphore: Arc::new(Semaphore::new(
            DEFAULT_PROXY_RAW_ASYNC_MAX_CONCURRENT_WRITERS,
        )),
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        pool_group_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        upstream_accounts: Arc::new(UpstreamAccountsRuntime::test_instance()),
    });

    let app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy test server");
    let addr = listener.local_addr().expect("proxy test server addr");
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("proxy test server should run");
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{addr}/v1/slow-stream"))
        .send()
        .await
        .expect("send proxy stream request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.bytes().await.expect("read proxied stream");
    assert_eq!(&body[..], b"chunk-achunk-b");

    server_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_e2e_stream_survives_short_request_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy test server");
    let addr = listener.local_addr().expect("proxy test server addr");
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("proxy test server should run");
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{addr}/v1/slow-stream"))
        .header(http_header::AUTHORIZATION, "Bearer pool-live-key")
        .send()
        .await
        .expect("send pool proxy stream request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.bytes().await.expect("read proxied stream");
    assert_eq!(&body[..], b"chunk-achunk-b");

    server_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_times_out_before_first_chunk_with_short_request_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy test server");
    let addr = listener.local_addr().expect("proxy test server addr");
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("proxy test server should run");
    });

    let client = reqwest::Client::new();
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        client
            .get(format!("http://{addr}/v1/slow-first-chunk"))
            .header(http_header::AUTHORIZATION, "Bearer pool-live-key")
            .send(),
    )
    .await
    .expect("pool request should not hang")
    .expect("send pool proxy request");

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = response.bytes().await.expect("read pool error body");
    let payload = String::from_utf8_lossy(&body);
    assert!(payload.contains("first upstream chunk"));

    server_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_stream_survives_short_request_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy test server");
    let addr = listener.local_addr().expect("proxy test server addr");
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("proxy test server should run");
    });

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello",
    }))
    .expect("serialize request body");
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/v1/responses?mode=slow-success"))
        .header(http_header::AUTHORIZATION, "Bearer pool-live-key")
        .header(http_header::CONTENT_TYPE, "application/json")
        .body(request_body)
        .send()
        .await
        .expect("send pool responses request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.bytes().await.expect("read pool responses body");
    let payload = String::from_utf8_lossy(&body);
    assert!(payload.contains("response.created"));
    assert!(payload.contains("response.completed"));
    assert!(payload.contains("resp_slow_test"));

    server_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_waits_for_first_chunk_beyond_request_timeout() {
    let (upstream_base, upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(100);
    // Full-suite load can delay the first streamed chunk far beyond the nominal 250ms fixture
    // sleep even though the request should still succeed once the chunk arrives.
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(1_200);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
    }))
    .expect("serialize request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state),
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
    .expect("responses pool request should not hang");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read responses pool body");
    let payload: Value = serde_json::from_slice(&body).expect("decode responses pool response");
    assert_eq!(payload["ok"].as_bool(), Some(true));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_waits_for_headers_beyond_handshake_timeout() {
    let (upstream_base, upstream_handle) =
        spawn_pool_delayed_headers_upstream(Duration::from_millis(250)).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(400);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
    }))
    .expect("serialize request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state),
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
    .expect("responses pool request should not hang");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read responses pool body");
    let payload: Value = serde_json::from_slice(&body).expect("decode responses pool response");
    assert_eq!(payload["ok"].as_bool(), Some(true));
    assert_eq!(payload["phase"].as_str(), Some("headers-delayed"));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_compact_waits_for_dedicated_first_chunk_timeout() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(400);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_001",
        "input": [{"role": "user", "content": "compact this thread"}],
    }))
    .expect("serialize compact request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state),
            OriginalUri(
                "/v1/responses/compact?mode=slow-first-chunk"
                    .parse()
                    .expect("valid uri"),
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
            Body::from(request_body),
        ),
    )
    .await
    .expect("compact pool request should not hang");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact pool body");
    let payload: Value = serde_json::from_slice(&body).expect("decode compact pool response");
    assert_eq!(
        payload.get("object").and_then(Value::as_str),
        Some("response.compaction")
    );
    assert_eq!(
        payload.get("id").and_then(Value::as_str),
        Some("resp_compact_test")
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_still_times_out_before_first_chunk() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_secs(5);
    config.openai_proxy_compact_handshake_timeout = Duration::from_millis(400);
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
    }))
    .expect("serialize responses request body");
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state),
            OriginalUri(
                "/v1/responses?mode=slow-first-chunk"
                    .parse()
                    .expect("valid uri"),
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
            Body::from(request_body),
        ),
    )
    .await
    .expect("responses pool request should not hang");

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read responses pool error body");
    let payload = String::from_utf8_lossy(&body);
    assert!(payload.contains("no alternate upstream route is available after timeout"));

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_responses_stream_timeout_applies_after_first_byte() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": "hello",
    }))
    .expect("serialize responses request body");
    let response = proxy_openai_v1(
        State(state),
        OriginalUri(
            "/v1/responses?mode=slow-stream-end"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("responses stream should time out after first byte");
    assert!(
        err.to_string()
            .contains("request timed out after 200ms while waiting for upstream stream completion")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_compact_stream_timeout_applies_after_first_byte() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_001",
        "input": [{"role": "user", "content": "compact this thread"}],
    }))
    .expect("serialize compact request body");
    let response = proxy_openai_v1(
        State(state),
        OriginalUri(
            "/v1/responses/compact?mode=slow-stream-end"
                .parse()
                .expect("valid compact uri"),
        ),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("compact stream should time out after first byte");
    assert!(
        err.to_string()
            .contains("request timed out after 200ms while waiting for upstream stream completion")
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_oauth_models_preserve_response_timeout_after_headers() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_slow_models_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let mut config = test_config();
    config.request_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_oauth_account(&state, "Primary OAuth", "oauth-primary").await;

    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
            State(state),
            OriginalUri("/v1/models".parse().expect("valid uri")),
            Method::GET,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::empty(),
        ),
    )
    .await
    .expect("oauth pool request should not hang");

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read oauth pool error body");
    let payload = String::from_utf8_lossy(&body);
    assert!(payload.contains("timed out"));

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

#[tokio::test]
async fn build_account_sticky_keys_response_keeps_attached_keys_without_recent_activity() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("sticky-stale")
    .bind(account_id)
    .bind("2026-03-10 00:00:00")
    .bind("2026-03-10 00:00:00")
    .bind("2026-03-10 00:00:00")
    .execute(&state.pool)
    .await
    .expect("insert sticky route");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("sticky-stale-invoke")
    .bind("2026-03-10 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(123_i64)
    .bind(0.42_f64)
    .bind(
        json!({
            "stickyKey": "sticky-stale",
            "upstreamAccountId": account_id,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert stale sticky invocation");
    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky aggregate response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(20),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    assert_eq!(json["selectionMode"].as_str(), Some("count"));
    assert_eq!(json["selectedLimit"].as_i64(), Some(20));
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 1);
    let conversation = &conversations[0];
    assert_eq!(conversation["stickyKey"].as_str(), Some("sticky-stale"));
    assert_eq!(conversation["requestCount"].as_i64(), Some(1));
    assert_eq!(conversation["totalTokens"].as_i64(), Some(123));
    assert_eq!(
        conversation["last24hRequests"].as_array().map(Vec::len),
        Some(0)
    );
}

#[test]
fn resolve_sticky_key_selection_rejects_mutually_exclusive_params() {
    let err = crate::upstream_accounts::resolve_sticky_key_selection(&AccountStickyKeysQuery {
        limit: Some(20),
        activity_hours: Some(3),
    })
    .expect_err("selection should reject mutually exclusive params");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("provide either limit or activityHours"));
}

#[test]
fn resolve_sticky_key_selection_rejects_unsupported_activity_hours() {
    let err = crate::upstream_accounts::resolve_sticky_key_selection(&AccountStickyKeysQuery {
        limit: None,
        activity_hours: Some(2),
    })
    .expect_err("selection should reject unsupported activityHours");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(
        err.1
            .contains("activityHours must be one of 1, 3, 6, 12, or 24")
    );
}

#[tokio::test]
async fn build_account_sticky_keys_response_activity_window_filters_recent_keys_only() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let recent_time = format_naive(
        (Utc::now() - ChronoDuration::hours(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let stale_time = format_naive(
        (Utc::now() - ChronoDuration::hours(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    for (sticky_key, occurred_at) in [
        ("sticky-window-recent", recent_time.as_str()),
        ("sticky-window-stale", stale_time.as_str()),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(sticky_key)
        .bind(account_id)
        .bind(occurred_at)
        .bind(occurred_at)
        .bind(occurred_at)
        .execute(&state.pool)
        .await
        .expect("insert sticky route");
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("{sticky_key}-invoke"))
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(64_i64)
        .bind(0.08_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky activity-window response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::ActivityWindow(3),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    assert_eq!(json["selectionMode"].as_str(), Some("activityWindow"));
    assert_eq!(json["selectedLimit"], Value::Null);
    assert_eq!(json["selectedActivityHours"].as_i64(), Some(3));
    assert_eq!(
        json["implicitFilter"]["kind"].as_str(),
        Some("inactiveOutside24h")
    );
    assert_eq!(json["implicitFilter"]["filteredCount"].as_i64(), Some(1));
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 1);
    assert_eq!(
        conversations[0]["stickyKey"].as_str(),
        Some("sticky-window-recent")
    );
}

#[tokio::test]
async fn build_account_sticky_keys_response_activity_window_previews_respect_time_window() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let sticky_key = "sticky-window-preview";
    let recent_time = format_naive(
        (Utc::now() - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let stale_time = format_naive(
        (Utc::now() - ChronoDuration::hours(8))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(&stale_time)
    .bind(&recent_time)
    .bind(&recent_time)
    .execute(&state.pool)
    .await
    .expect("insert sticky route");

    for (invoke_id, occurred_at) in [
        ("sticky-window-preview-recent", recent_time.as_str()),
        ("sticky-window-preview-stale", stale_time.as_str()),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(64_i64)
        .bind(0.08_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky preview response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::ActivityWindow(3),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let previews = json["conversations"][0]["recentInvocations"]
        .as_array()
        .expect("recent invocations array");
    assert_eq!(previews.len(), 1);
    assert_eq!(
        previews[0]["invokeId"].as_str(),
        Some("sticky-window-preview-recent")
    );
}

#[tokio::test]
async fn build_account_sticky_keys_response_activity_window_caps_results_to_fifty() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let now = Utc::now();

    for index in 0..55 {
        let occurred_at = format_naive(
            (now - ChronoDuration::minutes(index as i64))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let sticky_key = format!("sticky-window-cap-{index}");
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(&sticky_key)
        .bind(account_id)
        .bind(&occurred_at)
        .bind(&occurred_at)
        .bind(&occurred_at)
        .execute(&state.pool)
        .await
        .expect("insert sticky route");
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("sticky-window-cap-invoke-{index}"))
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(64_i64)
        .bind(0.08_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky activity-window response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::ActivityWindow(3),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let conversations = json["conversations"]
        .as_array()
        .expect("sticky conversations array");
    assert_eq!(conversations.len(), 50);
    assert_eq!(json["implicitFilter"]["kind"].as_str(), Some("cappedTo50"));
    assert_eq!(json["implicitFilter"]["filteredCount"].as_i64(), Some(5));
}

#[tokio::test]
async fn build_account_sticky_keys_response_includes_recent_invocations_sorted_and_capped() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let sticky_key = "sticky-preview-cap";

    sqlx::query(
        r#"
        INSERT INTO pool_sticky_routes (sticky_key, account_id, created_at, updated_at, last_seen_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(sticky_key)
    .bind(account_id)
    .bind(format_naive(
        (Utc::now() - ChronoDuration::minutes(6))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .execute(&state.pool)
    .await
    .expect("insert sticky preview route");

    for minutes_ago in 0..6 {
        let occurred_at = format_naive(
            (Utc::now() - ChronoDuration::minutes(minutes_ago))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(format!("sticky-preview-invoke-{minutes_ago}"))
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(100_i64 + i64::from(minutes_ago))
        .bind(0.01_f64 + (minutes_ago as f64) * 0.001_f64)
        .bind(
            json!({
                "stickyKey": sticky_key,
                "upstreamAccountId": account_id,
                "endpoint": "/v1/responses",
                "routeMode": "sticky",
                "model": "gpt-5.4",
            })
            .to_string(),
        )
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert preview invocation");
    }

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("sync hourly rollups before sticky preview response");

    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(20),
    )
    .await
    .expect("build sticky response");
    let json = serde_json::to_value(&response).expect("serialize sticky response");
    let previews = json["conversations"][0]["recentInvocations"]
        .as_array()
        .expect("recent invocations array");
    assert_eq!(previews.len(), 5);
    assert_eq!(
        previews[0]["invokeId"].as_str(),
        Some("sticky-preview-invoke-0")
    );
    assert_eq!(
        previews[4]["invokeId"].as_str(),
        Some("sticky-preview-invoke-4")
    );
}

#[tokio::test]
async fn prompt_cache_views_ignore_sticky_only_internal_keys() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, payload) in [
        (
            "sticky-only",
            json!({
                "stickyKey": "sticky-only",
                "upstreamScope": "internal",
                "routeMode": "pool"
            }),
        ),
        (
            "prompt-cache",
            json!({
                "promptCacheKey": "pck-real",
                "upstreamScope": "external"
            }),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            (Utc::now() - ChronoDuration::minutes(5))
                .with_timezone(&Shanghai)
                .naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(10_i64)
        .bind(0.01_f64)
        .bind(payload.to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert prompt cache test invocation");
    }

    let Json(response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
        }),
    )
    .await
    .expect("prompt cache conversations should succeed");
    let keys = response
        .conversations
        .iter()
        .map(|item| item.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["pck-real".to_string()]);

    let Json(list_response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(20),
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should succeed");
    let sticky_record = list_response
        .records
        .into_iter()
        .find(|record| record.invoke_id == "sticky-only")
        .expect("sticky-only record should exist");
    assert!(sticky_record.prompt_cache_key.is_none());
}
