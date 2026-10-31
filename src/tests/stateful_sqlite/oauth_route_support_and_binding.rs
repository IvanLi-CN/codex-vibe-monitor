use super::*;
use serde_json::json;

pub(super) fn run_oauth_future_with_large_stack<T, Fut>(future: Fut) -> T
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

struct ResetTestOauthCodexUpstreamBaseUrl;

impl Drop for ResetTestOauthCodexUpstreamBaseUrl {
    fn drop(&mut self) {
        oauth_bridge::reset_test_oauth_codex_upstream_base_url_now();
    }
}

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
        request_body_snapshot_kind: None,
        request_body_logical_bytes: None,
        request_body_transmitted_bytes: None,
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

#[test]
fn pool_route_oauth_body_sticky_binding_applies_before_first_send() {
    run_oauth_future_with_large_stack(async move {
        let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;
        let _upstream_reset_guard = ResetTestOauthCodexUpstreamBaseUrl;

        let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
        oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("{upstream_base}/backend-api/codex"))
                .expect("valid oauth base url"),
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
    });
}

#[test]
fn pool_route_oauth_live_candidate_prefers_account_proxy_and_honors_conversation_override() {
    run_oauth_future_with_large_stack(async move {
        let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;
        let _upstream_reset_guard = ResetTestOauthCodexUpstreamBaseUrl;

        let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
        oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("{upstream_base}/backend-api/codex"))
                .expect("valid oauth base url"),
        )
        .await;
        let (account_proxy_url, account_proxy_observations, account_proxy_handle) =
            spawn_test_http_connect_proxy_with_observations(&upstream_base).await;
        let (group_proxy_url, group_proxy_observations, group_proxy_handle) =
            spawn_test_http_connect_proxy_with_observations(&upstream_base).await;
        let account_proxy_key =
            normalize_single_proxy_key(&account_proxy_url).expect("normalize account proxy key");
        let group_proxy_key =
            normalize_single_proxy_key(&group_proxy_url).expect("normalize group proxy key");

        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        apply_forward_proxy_settings_without_bootstrap(
            &state,
            ForwardProxySettings {
                proxy_urls: vec![account_proxy_url, group_proxy_url],
                ..ForwardProxySettings::default()
            },
        )
        .await;
        let account_id = insert_test_pool_oauth_account(&state, "Bound OAuth", "oauth-bound").await;
        sqlx::query("UPDATE pool_upstream_accounts SET bound_proxy_keys_json = ?1 WHERE id = ?2")
            .bind(
                serde_json::to_string(&vec![account_proxy_key]).expect("encode account proxy key"),
            )
            .bind(account_id)
            .execute(&state.pool)
            .await
            .expect("bind account proxy key");
        sqlx::query(
            "UPDATE pool_upstream_account_group_notes SET bound_proxy_keys_json = ?1 WHERE group_name = ?2",
        )
        .bind(serde_json::to_string(&vec![group_proxy_key.clone()]).expect("encode group proxy key"))
        .bind(test_required_group_name())
        .execute(&state.pool)
        .await
        .expect("bind group proxy key");

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
            Body::from(br#"{"model":"gpt-5.4","input":"account route"}"#.to_vec()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read account proxy response"),
        )
        .expect("decode account proxy response");
        assert_eq!(
            payload["authorization"].as_str(),
            Some("Bearer oauth-bound")
        );
        assert_eq!(
            account_proxy_observations
                .request_count
                .load(Ordering::SeqCst),
            1
        );
        assert_eq!(
            group_proxy_observations
                .request_count
                .load(Ordering::SeqCst),
            0
        );
        assert_eq!(
            account_proxy_observations
                .connect_count
                .load(Ordering::SeqCst),
            1,
            "the account-bound OAuth request should use the local CONNECT proxy"
        );
        assert!(
            account_proxy_observations
                .request_lines
                .lock()
                .expect("lock account proxy request lines")
                .iter()
                .all(|line| line.starts_with("CONNECT ")),
            "account proxy should receive a CONNECT request: {:?}",
            account_proxy_observations
                .request_lines
                .lock()
                .expect("lock account proxy request lines")
        );
        assert!(
            group_proxy_observations
                .request_lines
                .lock()
                .expect("lock group proxy request lines")
                .is_empty(),
            "group proxy should not receive the first OAuth request"
        );

        let conversation_key = "oauth-conversation-proxy-override";
        let binding: UpdatePromptCacheConversationBindingRequest = serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
            "forwardProxyKey": group_proxy_key,
        }))
        .expect("deserialize conversation proxy binding");
        let Json(_) = patch_prompt_cache_conversation_binding(
            State(state.clone()),
            AxumPath(conversation_key.to_string()),
            Json(binding),
        )
        .await
        .expect("save conversation proxy binding");

        let response = proxy_openai_v1(
            State(state),
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
            Body::from(
                serde_json::to_vec(&json!({
                    "model": "gpt-5.4",
                    "input": "conversation route",
                    "stickyKey": conversation_key,
                    "promptCacheKey": conversation_key,
                }))
                .expect("serialize conversation route body"),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read conversation proxy response"),
        )
        .expect("decode conversation proxy response");
        assert_eq!(
            payload["authorization"].as_str(),
            Some("Bearer oauth-bound")
        );
        assert_eq!(
            account_proxy_observations
                .request_count
                .load(Ordering::SeqCst),
            1
        );
        assert_eq!(
            group_proxy_observations
                .request_count
                .load(Ordering::SeqCst),
            1
        );
        assert_eq!(
            group_proxy_observations
                .connect_count
                .load(Ordering::SeqCst),
            1,
            "the conversation override should use the local CONNECT proxy"
        );
        assert!(
            group_proxy_observations
                .request_lines
                .lock()
                .expect("lock group proxy request lines")
                .iter()
                .all(|line| line.starts_with("CONNECT ")),
            "conversation override should send a CONNECT request through the group proxy"
        );

        upstream_handle.abort();
        account_proxy_handle.abort();
        group_proxy_handle.abort();
    });
}

#[test]
fn pool_route_oauth_token_refresh_uses_account_proxy_before_live_send() {
    run_oauth_future_with_large_stack(async move {
        let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;
        let _upstream_reset_guard = ResetTestOauthCodexUpstreamBaseUrl;

        let token_requests = Arc::new(AtomicUsize::new(0));
        let final_authorization = Arc::new(std::sync::Mutex::new(None));
        let upstream_app = Router::new().fallback({
            let token_requests = token_requests.clone();
            let final_authorization = final_authorization.clone();
            any(move |request: axum::extract::Request| {
                let token_requests = token_requests.clone();
                let final_authorization = final_authorization.clone();
                async move {
                    let path = request.uri().path().to_string();
                    if path == "/oauth/token" {
                        token_requests.fetch_add(1, Ordering::SeqCst);
                        return (
                            StatusCode::OK,
                            Json(json!({
                                "access_token": "oauth-refreshed",
                                "refresh_token": "refresh-token-rotated",
                                "token_type": "Bearer",
                                "expires_in": 3600,
                            })),
                        )
                            .into_response();
                    }
                    let authorization = request
                        .headers()
                        .get(http_header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    if !path.is_empty() {
                        *final_authorization
                            .lock()
                            .expect("lock final oauth authorization") = authorization.clone();
                    }
                    (
                        StatusCode::OK,
                        Json(json!({"authorization": authorization})),
                    )
                        .into_response()
                }
            })
        });
        let upstream_listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind oauth refresh upstream");
        let upstream_addr = upstream_listener
            .local_addr()
            .expect("oauth refresh upstream addr");
        let upstream_handle = tokio::spawn(async move {
            axum::serve(upstream_listener, upstream_app)
                .await
                .expect("oauth refresh upstream should run");
        });
        let upstream_base = format!("http://{upstream_addr}");
        oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("{upstream_base}/backend-api/codex"))
                .expect("valid oauth refresh base url"),
        )
        .await;
        let (account_proxy_url, account_proxy_observations, account_proxy_handle) =
            spawn_test_http_connect_proxy_with_observations(&upstream_base).await;
        let (group_proxy_url, group_proxy_observations, group_proxy_handle) =
            spawn_test_http_connect_proxy_with_observations(&upstream_base).await;
        let account_proxy_key =
            normalize_single_proxy_key(&account_proxy_url).expect("normalize account proxy key");
        let group_proxy_key =
            normalize_single_proxy_key(&group_proxy_url).expect("normalize group proxy key");

        let mut state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        Arc::get_mut(&mut state)
            .expect("oauth refresh state should be uniquely owned")
            .config
            .upstream_accounts_oauth_issuer =
            Url::parse(&upstream_base).expect("valid local oauth issuer");
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        apply_forward_proxy_settings_without_bootstrap(
            &state,
            ForwardProxySettings {
                proxy_urls: vec![account_proxy_url, group_proxy_url],
                ..ForwardProxySettings::default()
            },
        )
        .await;
        let account_id =
            insert_test_pool_oauth_account(&state, "Refresh OAuth", "oauth-expired").await;
        sqlx::query(
            "UPDATE pool_upstream_accounts SET bound_proxy_keys_json = ?1, token_expires_at = ?2 WHERE id = ?3",
        )
        .bind(
            serde_json::to_string(&vec![account_proxy_key]).expect("encode account proxy key"),
        )
        .bind(format_utc_iso(Utc::now() - ChronoDuration::hours(1)))
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("expire and bind oauth account");
        sqlx::query(
            "UPDATE pool_upstream_account_group_notes SET bound_proxy_keys_json = ?1 WHERE group_name = ?2",
        )
        .bind(serde_json::to_string(&vec![group_proxy_key]).expect("encode group proxy key"))
        .bind(test_required_group_name())
        .execute(&state.pool)
        .await
        .expect("bind group proxy key");

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
            Body::from(br#"{"model":"gpt-5.4","input":"refresh route"}"#.to_vec()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            final_authorization
                .lock()
                .expect("lock final oauth authorization")
                .as_deref(),
            Some("Bearer oauth-refreshed"),
        );
        assert_eq!(token_requests.load(Ordering::SeqCst), 1);
        assert!(
            account_proxy_observations
                .request_count
                .load(Ordering::SeqCst)
                >= 3,
            "account proxy should carry token refresh and final upstream send"
        );
        assert!(
            account_proxy_observations
                .connect_count
                .load(Ordering::SeqCst)
                >= 2,
            "account proxy should carry the egress and final upstream CONNECTs: count={} lines={:?}",
            account_proxy_observations
                .connect_count
                .load(Ordering::SeqCst),
            account_proxy_observations
                .request_lines
                .lock()
                .expect("lock account proxy request lines")
        );
        let account_proxy_request_lines = account_proxy_observations
            .request_lines
            .lock()
            .expect("lock account proxy request lines");
        assert!(
            account_proxy_request_lines
                .iter()
                .any(|line| line.contains("/oauth/token")),
            "account proxy should carry OAuth token refresh: {:?}",
            account_proxy_request_lines
        );
        assert!(
            account_proxy_request_lines
                .iter()
                .any(|line| line.starts_with("CONNECT 127.0.0.1:")),
            "account proxy should carry the final OAuth upstream CONNECT: {:?}",
            account_proxy_request_lines
        );
        assert_eq!(
            group_proxy_observations
                .request_count
                .load(Ordering::SeqCst),
            0,
            "group proxy must not receive an OAuth account-bound refresh or send"
        );

        upstream_handle.abort();
        account_proxy_handle.abort();
        group_proxy_handle.abort();
    });
}

#[test]
fn pool_route_oauth_standalone_search_is_forwarded_and_recorded() {
    run_oauth_future_with_large_stack(async move {
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
        let _upstream_reset_guard = ResetTestOauthCodexUpstreamBaseUrl;
        let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
        oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("{upstream_base}/backend-api/codex"))
                .expect("valid oauth base url"),
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
    });
}

#[test]
fn pool_route_oauth_compact_passthrough_preserves_prompt_cache_headers() {
    run_oauth_future_with_large_stack(async move {
        #[derive(sqlx::FromRow)]
        struct PersistedRow {
            payload: Option<String>,
        }

        let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;
        let _upstream_reset_guard = ResetTestOauthCodexUpstreamBaseUrl;

        let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
        oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("{upstream_base}/backend-api/codex"))
                .expect("valid oauth base url"),
        )
        .await;

        let state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let account_id =
            insert_test_pool_oauth_account(&state, "Compact OAuth", "oauth-compact").await;

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
                    HeaderValue::from_static(
                        "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01",
                    ),
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
    });
}

#[test]
fn pool_route_oauth_observability_omits_fingerprints_without_crypto_key() {
    run_oauth_future_with_large_stack(async move {
        let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;
        let _upstream_reset_guard = ResetTestOauthCodexUpstreamBaseUrl;

        let (upstream_base, upstream_handle) = spawn_oauth_codex_responses_capture_upstream().await;
        oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("{upstream_base}/backend-api/codex"))
                .expect("valid oauth base url"),
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
                    HeaderValue::from_static(
                        "00-11111111111111111111111111111111-2222222222222222-01",
                    ),
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
        let payload_json: Value =
            serde_json::from_slice(&body).expect("decode oauth response body");
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
        let serialized_debug =
            serde_json::to_string(&request_debug).expect("serialize request debug");
        assert!(
            !serialized_debug.contains("session-no-crypto"),
            "request debug should not leak raw header values"
        );

        upstream_handle.abort();
    });
}

#[test]
fn pool_route_uses_account_specific_upstream_base_url() {
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
fn pool_route_honors_existing_body_sticky_binding_for_non_capture_requests() {
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

    assert_eq!(
        location_rewrite_upstream_base(Some(&account), &global).as_str(),
        "https://proxy.example.com/gateway"
    );
    assert_eq!(
        location_rewrite_upstream_base(None, &global).as_str(),
        "https://api.openai.com/"
    );
}
