use super::*;
use serde_json::json;

fn assert_encrypted_owner_blocked_proxy_error(
    response: &ProxyErrorResponse,
    owner_account_id: i64,
    owner_label: impl Into<String>,
    prompt_cache_key: &str,
) {
    let owner_label = owner_label.into();
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.message,
        format!(
            "encrypted session owner routing is constrained to upstream account {owner_label} but that account is currently unavailable"
        )
    );
    assert_eq!(
        response.code.as_deref(),
        Some(PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED)
    );
    assert_eq!(
        response.blocked_binding,
        Some(BlockedBindingDiagnostic {
            constraint_source: BlockedBindingConstraintSource::EncryptedSessionOwner,
            upstream_account_id: owner_account_id,
            upstream_account_label: owner_label,
            prompt_cache_key: Some(prompt_cache_key.to_string()),
            recovery_action: BlockedBindingRecoveryAction::ClearAndResetAffinity,
        })
    );
}

fn run_future_with_large_stack<T, Fut>(future: Fut) -> T
where
    T: Send + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    std::thread::Builder::new()
        .name("live-first-owner-guard-large-stack".to_string())
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

async fn run_future_with_large_stack_async<T, Fut>(future: Fut) -> T
where
    T: Send + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || run_future_with_large_stack(future))
        .await
        .expect("join large-stack test worker")
}

#[tokio::test]
async fn proxy_openai_v1_via_pool_waits_for_initial_account_resolution_before_sending() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(180),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1_via_pool(
            request_state,
            4242,
            &"/v1/chat/completions".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("sticky-via-pool-wait"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5","messages":[]}"#.as_bytes().to_vec()),
            runtime_timeouts,
            None,
        )
        .await
    });

    let pool = state.pool.clone();
    let release_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        set_test_account_status(&pool, delayed_id, "active").await;
    });

    tokio::time::sleep(Duration::from_millis(20)).await;
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

    let response = request_task
        .await
        .expect("via-pool request task should join")
        .expect("via-pool request should succeed");
    release_task
        .await
        .expect("delayed account release task should join");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read via-pool response");
    let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
    assert_eq!(payload["authorization"], "Bearer upstream-delayed");
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-delayed").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_body_only_sticky_stream_waits_only_once_before_503() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_millis(120),
        Duration::from_millis(20),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        let _ = tx
            .send(Ok(Bytes::from_static(
                b"\"messages\":[],\"stickyKey\":\"stream-body-sticky\"}",
            )))
            .await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let wait_started_task = std::thread::spawn(move || {
        wait_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("body-only sticky request should signal once the bounded wait starts");
        Instant::now()
    });
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        5242,
        &"/v1/chat/completions".parse().expect("valid uri"),
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
        runtime_timeouts,
        None,
    )
    .await;
    let wait_started_at = wait_started_task
        .join()
        .expect("wait-start watcher thread should join");
    let elapsed_since_wait_start = wait_started_at.elapsed();

    assert!(
        elapsed_since_wait_start < Duration::from_millis(260),
        "body-only sticky streaming request should finish after one bounded wait window once waiting starts, elapsed_since_wait_start={elapsed_since_wait_start:?}"
    );
    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        message, POOL_NO_AVAILABLE_ACCOUNT_MESSAGE,
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn proxy_openai_v1_chunked_codex_lite_keeps_live_first_and_audits_keep_original() {
    let mut config = test_config();
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");

    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    let (release_tail_tx, release_tail_rx) = tokio::sync::oneshot::channel::<()>();
    let first_chunk = format!(
        "{{\"model\":\"gpt-5\",\"input\":\"{}",
        "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
    );
    let body_task = tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
        let _ = release_tail_rx.await;
        let _ = tx.send(Ok(Bytes::from_static(b"\"}"))).await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1_via_pool(
            request_state,
            5342,
            &"/v1/responses".parse().expect("valid uri"),
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
                    HeaderName::from_static("x-openai-internal-codex-responses-lite"),
                    HeaderValue::from_static("true"),
                ),
            ]),
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
            runtime_timeouts,
            None,
        )
        .await
    });
    let response = timeout(Duration::from_secs(1), request_task)
        .await
        .expect("live-first request should resolve before the trailing chunk is released")
        .expect("live-first request task should join")
        .expect("chunked via-pool request should succeed via live first attempt");
    let _ = release_tail_tx.send(());
    body_task
        .await
        .expect("chunked request body sender should join");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read via-pool response");
    let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    let latest_attempt = timeout(Duration::from_secs(1), async {
        loop {
            let row =
                sqlx::query_as::<_, (Option<String>, Option<String>, String, Option<String>)>(
                    r#"
                SELECT group_name_snapshot, proxy_binding_key_snapshot, status, request_summary_json
                FROM pool_upstream_request_attempts
                ORDER BY id DESC
                LIMIT 1
                "#,
                )
                .fetch_one(&state.pool)
                .await
                .expect("load latest pool attempt");
            if row.2 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
                break row;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wait for live-first pool attempt success");
    assert_eq!(
        latest_attempt.0.as_deref(),
        Some(test_required_group_name()),
        "live-first grouped stats should snapshot the resolved group name",
    );
    assert_eq!(
        latest_attempt.1.as_deref(),
        Some(FORWARD_PROXY_DIRECT_KEY),
        "live-first grouped stats should persist the canonical binding key",
    );
    assert_eq!(
        latest_attempt.2, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        "successful live-first requests should land as real success attempts",
    );
    let request_summary: Value = serde_json::from_str(
        latest_attempt
            .3
            .as_deref()
            .expect("Codex live-first request should persist an audit summary"),
    )
    .expect("decode Codex live-first audit summary");
    assert_eq!(
        request_summary["codexImagegenRewrite"]["protocol"],
        "responses_lite"
    );
    assert_eq!(
        request_summary["codexImagegenRewrite"]["mode"],
        "keep_original"
    );
    assert_eq!(
        request_summary["codexImagegenRewrite"]["outcome"],
        "no_change"
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_capture_responses_sends_the_live_treatment_before_request_eof() {
    let mut config = test_config();
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    config.proxy_enforce_stream_include_usage = false;
    let (upstream_base, _attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let live_settings: UpdatePoolRoutingSettingsRequest = serde_json::from_value(json!({
        "liveRequestStreaming": {
            "enabled": true,
            "treatmentPercent": 100,
        },
    }))
    .expect("deserialize live request streaming settings");
    let _ =
        update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(live_settings))
            .await
            .expect("enable live request streaming treatment");

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    let (release_tail_tx, release_tail_rx) = tokio::sync::oneshot::channel::<()>();
    let first_chunk = "{\"model\":\"gpt-5\",\"input\":\"ready\"}\n".to_string();
    let body_task = tokio::spawn(async move {
        tx.send(Ok(Bytes::from(first_chunk)))
            .await
            .expect("send request prefix");
        release_tail_rx.await.expect("release request tail");
        tx.send(Ok(Bytes::from_static(b" \n")))
            .await
            .expect("send request tail");
    });
    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1(
            State(request_state),
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
                (
                    HeaderName::from_static("x-openai-internal-codex-responses-lite"),
                    HeaderValue::from_static("true"),
                ),
            ]),
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        )
        .await
    });

    timeout(
        Duration::from_secs(1),
        wait_for_pool_upstream_request_attempts(&state.pool, 1),
    )
    .await
    .expect("live treatment should start an upstream attempt before request eof");
    let _ = release_tail_tx.send(());
    body_task.await.expect("request body task should join");
    let response = request_task
        .await
        .expect("capture request task should join");
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read capture response body");
    let (transport_mode, upstream_first_byte_ms, overlap_ms) =
        timeout(Duration::from_secs(1), async {
            loop {
                let row = sqlx::query_as::<_, (Option<String>, Option<f64>, Option<f64>)>(
                    r#"
                SELECT
                    json_extract(payload, '$.requestBodyTransportMode'),
                    json_extract(payload, '$.upstreamRequestFirstByteMs'),
                    json_extract(payload, '$.requestUpstreamOverlapMs')
                FROM codex_invocations
                WHERE json_extract(payload, '$.liveFirstExperimentVariant') = 'treatment'
                ORDER BY id DESC
                LIMIT 1
                "#,
                )
                .fetch_optional(&state.pool)
                .await
                .expect("query live treatment invocation");
                if let Some(row) = row {
                    break row;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("live treatment invocation should persist");
    assert_eq!(transport_mode.as_deref(), Some("live_first"));
    assert!(upstream_first_byte_ms.is_some_and(|value| value >= 0.0));
    assert!(overlap_ms.is_some_and(|value| value > 0.0));

    upstream_handle.abort();
}

#[test]
fn proxy_openai_v1_responses_live_first_failover_restores_full_retry_budget_for_follow_up_accounts()
{
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[
            ("Bearer upstream-primary", 99),
            ("Bearer upstream-secondary", 2),
            ("Bearer upstream-tertiary", 0),
        ])
        .await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(180),
            Duration::from_millis(10),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
        insert_test_pool_api_key_account(&state, "Tertiary", "upstream-tertiary").await;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        let first_chunk = format!(
            "{{\"model\":\"gpt-5\",\"input\":\"{}",
            "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
        );
        tokio::spawn(async move {
            let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = tx.send(Ok(Bytes::from_static(b"\"}"))).await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            5343,
            &"/v1/responses".parse().expect("valid uri"),
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
            runtime_timeouts,
            None,
        )
        .await
        .expect("live-first responses request should eventually succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool responses response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool responses body");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");
        assert_eq!(payload["attempt"], 3);

        let attempts = attempts.lock().expect("lock live-first failover attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
        assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), None);

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_live_first_unsupported_model_bad_request_fails_over() {
    run_future_with_large_stack(async move {
        async fn unsupported_model_live_first_upstream(
            attempts: Arc<StdMutex<HashMap<String, usize>>>,
            headers: HeaderMap,
        ) -> Response {
            let authorization = headers
                .get(http_header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let attempt = {
                let mut attempts = attempts
                    .lock()
                    .expect("lock live-first unsupported-model attempts");
                let entry = attempts.entry(authorization.clone()).or_insert(0);
                *entry += 1;
                *entry
            };
            if authorization == "Bearer upstream-primary" {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": {
                            "code": "unsupported_model",
                            "message": "unsupported model: gpt-5.5",
                        },
                        "attempt": attempt,
                    })),
                )
                    .into_response();
            }
            (
                StatusCode::OK,
                Json(json!({
                    "ok": true,
                    "authorization": authorization,
                    "attempt": attempt,
                })),
            )
                .into_response()
        }

        let attempts = Arc::new(StdMutex::new(HashMap::new()));
        let app = Router::new().route(
            "/v1/chat/completions",
            post({
                let attempts = attempts.clone();
                move |headers| unsupported_model_live_first_upstream(attempts.clone(), headers)
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind live-first unsupported-model upstream");
        let addr = listener
            .local_addr()
            .expect("live-first unsupported-model upstream addr");
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("live-first unsupported-model upstream should run");
        });
        let mut config = test_config();
        config.openai_proxy_request_read_timeout = Duration::from_millis(260);
        config.openai_upstream_base_url =
            Url::parse(&format!("http://{addr}")).expect("valid upstream base url");

        let state = test_state_from_config_with_pool_no_available_wait(
            config,
            true,
            PoolNoAvailableWaitSettings {
                timeout: Duration::from_millis(80),
                retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
            },
        )
        .await;
        {
            let mut settings = state.proxy_model_settings.write().await;
            settings.websocket_enabled = true;
            settings.upstream_websocket_default_enabled = true;
        }
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
        let secondary_id =
            insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        let first_chunk = format!(
            "{{\"model\":\"gpt-5.5\",\"input\":\"{}",
            "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
        );
        tokio::spawn(async move {
            let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
            tokio::time::sleep(Duration::from_millis(130)).await;
            let _ = tx.send(Ok(Bytes::from_static(b"\"}"))).await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6343,
            &"/v1/chat/completions".parse().expect("valid uri"),
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
            runtime_timeouts,
            None,
        )
        .await
        .expect("live-first unsupported-model request should fail over");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read live-first unsupported-model response");
        let payload: Value =
            serde_json::from_slice(&body).expect("decode live-first unsupported-model response");
        assert_eq!(payload["authorization"], "Bearer upstream-secondary");

        wait_for_pool_upstream_request_attempts(&state.pool, 2).await;
        wait_for_pool_attempt_status(
            &state.pool,
            1,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
        )
        .await;
        wait_for_pool_attempt_status(&state.pool, 2, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
            .await;

        {
            let attempts = attempts
                .lock()
                .expect("lock live-first unsupported-model attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
            assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
        }

        let primary_tags = sqlx::query_scalar::<_, String>(
            r#"
        SELECT tag.system_key
        FROM pool_tags tag
        JOIN pool_upstream_account_tags link ON link.tag_id = tag.id
        WHERE link.account_id = ?1
          AND tag.system_key IS NOT NULL
        ORDER BY tag.system_key ASC
        "#,
        )
        .bind(primary_id)
        .fetch_all(&state.pool)
        .await
        .expect("load live-first primary system tags");
        assert!(
            !primary_tags
                .iter()
                .any(|tag| tag == "unsupported_model:gpt-5.5")
        );
        let primary_model_route = load_model_routing_states(&state.pool, primary_id)
            .await
            .expect("load primary model route")
            .into_iter()
            .find(|route| route.model == "gpt-5.5")
            .expect("primary model route should be learned dynamically");
        assert_eq!(primary_model_route.failure_count, 1);
        assert_eq!(
            load_test_sticky_route_account_id(&state.pool, "sticky-unsupported-model-failover")
                .await,
            None,
        );
        assert_ne!(primary_id, secondary_id);

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_responses_live_first_failover_preserves_prompt_cache_group_binding() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_retry_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(10),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let bound_group = "live-first-bound-group";
        let other_group = "live-first-other-group";
        ensure_test_group_binding(&state.pool, bound_group, None).await;
        ensure_test_group_binding(&state.pool, other_group, None).await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Primary",
            "upstream-primary",
            Some(bound_group),
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Secondary",
            "upstream-secondary",
            Some(other_group),
            None,
            None,
        )
        .await;
        let prompt_cache_key = "pck-live-first-bound-group";
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            created_at,
            updated_at
        )
        VALUES (?1, 'group', ?2, NULL, ?3, ?3)
        "#,
        )
        .bind(prompt_cache_key)
        .bind(bound_group)
        .bind(&now_iso)
        .execute(&state.pool)
        .await
        .expect("insert prompt cache group binding");

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        let first_chunk = format!(
            "{{\"model\":\"gpt-5\",\"promptCacheKey\":\"{prompt_cache_key}\",\"input\":\"{}",
            "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
        );
        tokio::spawn(async move {
            let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = tx.send(Ok(Bytes::from_static(b"\"}"))).await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            5344,
            &"/v1/responses".parse().expect("valid uri"),
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
            runtime_timeouts,
            None,
        )
        .await
        .expect_err("binding-constrained live-first failover should not use other groups");

        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        let attempts = attempts.lock().expect("lock live-first binding attempts");
        assert!(matches!(
            attempts.get("Bearer upstream-primary").copied(),
            Some(count) if count > 0
        ));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn proxy_openai_v1_responses_waits_for_body_before_encrypted_owner_guard() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer upstream-primary", 99)]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "upstream-primary",
        None,
        None,
        None,
    )
    .await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Encrypted Owner",
        "upstream-owner",
        None,
        None,
        None,
    )
    .await;
    set_test_account_status(&state.pool, owner_account_id, "needs_reauth").await;
    let prompt_cache_key = "pck-live-first-replay-owner-guard";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist encrypted owner lock");

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    let first_chunk = format!(
        "{{\"model\":\"gpt-5\",\"input\":\"{}",
        "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
    );
    let second_chunk = format!(
        "\",\"promptCacheKey\":\"{prompt_cache_key}\",\"encrypted_content\":{{\"ciphertext\":\"abc\"}}}}"
    );
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = tx.send(Ok(Bytes::from(second_chunk))).await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        5344,
        &"/v1/responses".parse().expect("valid uri"),
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
        runtime_timeouts,
        None,
    )
    .await
    .expect_err("live-first replay should stop at encrypted owner guard");

    assert_encrypted_owner_blocked_proxy_error(
        &response,
        owner_account_id,
        format!("#{owner_account_id}"),
        prompt_cache_key,
    );

    let attempts = attempts
        .lock()
        .expect("lock live-first replay owner guard attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);

    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

    upstream_handle.abort();
}

#[test]
fn proxy_openai_v1_responses_live_first_success_persists_encrypted_owner() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(10),
        )
        .await;
        enable_encrypted_session_owner_routing_for_test(&state).await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let owner_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Live First Encrypted Owner",
            "upstream-primary",
            None,
            None,
            None,
        )
        .await;
        let prompt_cache_key = "pck-live-first-success-owner";

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        let first_chunk = format!(
            "{{\"model\":\"gpt-5\",\"promptCacheKey\":\"{prompt_cache_key}\",\"input\":[{{\"type\":\"encrypted_content\",\"encrypted_content\":\"opaque-owner-bound-content\"}}],\"tail\":\"{}",
            "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
        );
        tokio::spawn(async move {
            let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = tx.send(Ok(Bytes::from_static(b"\"}"))).await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            5345,
            &"/v1/responses".parse().expect("valid uri"),
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
            runtime_timeouts,
            None,
        )
        .await
        .expect("live-first encrypted request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read live-first encrypted response");
        let payload: Value =
            serde_json::from_slice(&body).expect("decode live-first encrypted response");
        assert_eq!(payload["authorization"], "Bearer upstream-primary");

        let mut persisted_owner_account_id = None;
        for _ in 0..50 {
            persisted_owner_account_id = sqlx::query_scalar(
                r#"
            SELECT owner_upstream_account_id
            FROM prompt_cache_encrypted_session_owners
            WHERE prompt_cache_key = ?1
            "#,
            )
            .bind(prompt_cache_key)
            .fetch_optional(&state.pool)
            .await
            .expect("query live-first encrypted owner row");
            if persisted_owner_account_id.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert_eq!(persisted_owner_account_id, Some(owner_account_id));

        let attempts = attempts.lock().expect("lock live-first encrypted attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_responses_live_first_response_encryption_persists_encrypted_owner() {
    run_future_with_large_stack(async move {
        async fn response_encryption_live_first_upstream(
            attempts: Arc<StdMutex<HashMap<String, usize>>>,
            headers: HeaderMap,
        ) -> Response {
            let authorization = headers
                .get(http_header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            {
                let mut attempts = attempts
                    .lock()
                    .expect("lock live-first response-encryption attempts");
                let entry = attempts.entry(authorization.clone()).or_insert(0);
                *entry += 1;
            }
            (
                StatusCode::OK,
                Json(json!({
                    "authorization": authorization,
                    "output": [
                        {
                            "type": "encrypted_content",
                            "encrypted_content": "opaque-owner-bound-content"
                        }
                    ]
                })),
            )
                .into_response()
        }

        let attempts = Arc::new(StdMutex::new(HashMap::new()));
        let app = Router::new().route(
            "/v1/responses",
            post({
                let attempts = attempts.clone();
                move |headers| response_encryption_live_first_upstream(attempts.clone(), headers)
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind live-first response-encryption upstream");
        let addr = listener
            .local_addr()
            .expect("live-first response-encryption upstream addr");
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("live-first response-encryption upstream should run");
        });
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&format!("http://{addr}")).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(10),
        )
        .await;
        enable_encrypted_session_owner_routing_for_test(&state).await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let owner_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Live First Response Encrypted Owner",
            "upstream-primary",
            None,
            None,
            None,
        )
        .await;
        let prompt_cache_key = "pck-live-first-response-encrypted-owner";

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        let first_chunk = format!(
            "{{\"model\":\"gpt-5\",\"promptCacheKey\":\"{prompt_cache_key}\",\"input\":\"{}",
            "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
        );
        tokio::spawn(async move {
            let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = tx.send(Ok(Bytes::from_static(b"\"}"))).await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            5345,
            &"/v1/responses".parse().expect("valid uri"),
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
            runtime_timeouts,
            None,
        )
        .await
        .expect("live-first response-encryption request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read live-first response-encryption response");
        let payload: Value =
            serde_json::from_slice(&body).expect("decode live-first response-encryption response");
        assert_eq!(payload["authorization"], "Bearer upstream-primary");

        let mut persisted_owner_account_id = None;
        for _ in 0..50 {
            persisted_owner_account_id = sqlx::query_scalar(
                r#"
            SELECT owner_upstream_account_id
            FROM prompt_cache_encrypted_session_owners
            WHERE prompt_cache_key = ?1
            "#,
            )
            .bind(prompt_cache_key)
            .fetch_optional(&state.pool)
            .await
            .expect("query live-first response-encryption owner row");
            if persisted_owner_account_id.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert_eq!(persisted_owner_account_id, Some(owner_account_id));

        let attempts = attempts
            .lock()
            .expect("lock live-first response-encryption attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_responses_prebuffered_success_persists_encrypted_owner() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(10),
        )
        .await;
        enable_encrypted_session_owner_routing_for_test(&state).await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let owner_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Prebuffered Encrypted Owner",
            "upstream-primary",
            None,
            None,
            None,
        )
        .await;
        let prompt_cache_key = "pck-prebuffered-success-owner";

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
        state.clone(),
        5346,
        &"/v1/responses".parse().expect("valid uri"),
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
            format!(
                "{{\"model\":\"gpt-5\",\"promptCacheKey\":\"{prompt_cache_key}\",\"input\":[{{\"type\":\"encrypted_content\",\"encrypted_content\":\"opaque-owner-bound-content\"}}]}}"
            )
            .into_bytes(),
        ),
        runtime_timeouts,
        None,
    )
    .await
    .expect("prebuffered encrypted request should succeed");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read prebuffered encrypted response");
        let payload: Value =
            serde_json::from_slice(&body).expect("decode prebuffered encrypted response");
        assert_eq!(payload["authorization"], "Bearer upstream-primary");

        let mut persisted_owner_account_id = None;
        for _ in 0..50 {
            persisted_owner_account_id = sqlx::query_scalar(
                r#"
            SELECT owner_upstream_account_id
            FROM prompt_cache_encrypted_session_owners
            WHERE prompt_cache_key = ?1
            "#,
            )
            .bind(prompt_cache_key)
            .fetch_optional(&state.pool)
            .await
            .expect("query prebuffered encrypted owner row");
            if persisted_owner_account_id.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert_eq!(persisted_owner_account_id, Some(owner_account_id));

        let attempts = attempts
            .lock()
            .expect("lock prebuffered encrypted attempts");
        assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_responses_header_prompt_cache_key_preserves_group_binding() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_retry_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(10),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let bound_group = "header-bound-group";
        let other_group = "header-other-group";
        ensure_test_group_binding(&state.pool, bound_group, None).await;
        ensure_test_group_binding(&state.pool, other_group, None).await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Primary",
            "upstream-primary",
            Some(bound_group),
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Secondary",
            "upstream-secondary",
            Some(other_group),
            None,
            None,
        )
        .await;
        let prompt_cache_key = "pck-header-bound-group";
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            created_at,
            updated_at
        )
        VALUES (?1, 'group', ?2, NULL, ?3, ?3)
        "#,
        )
        .bind(prompt_cache_key)
        .bind(bound_group)
        .bind(&now_iso)
        .execute(&state.pool)
        .await
        .expect("insert header prompt cache group binding");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            5345,
            &"/v1/responses".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-prompt-cache-key"),
                    HeaderValue::from_static(prompt_cache_key),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5","input":"header-only"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect_err("header binding should not fail over outside its group");

        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        let attempts = attempts.lock().expect("lock header binding attempts");
        assert!(matches!(
            attempts.get("Bearer upstream-primary").copied(),
            Some(count) if count > 0
        ));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_bodyless_header_prompt_cache_key_preserves_group_binding() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_retry_upstream(&[("Bearer upstream-primary", 99)]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(10),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let bound_group = "bodyless-header-bound-group";
        let other_group = "bodyless-header-other-group";
        ensure_test_group_binding(&state.pool, bound_group, None).await;
        ensure_test_group_binding(&state.pool, other_group, None).await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Primary",
            "upstream-primary",
            Some(bound_group),
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Secondary",
            "upstream-secondary",
            Some(other_group),
            None,
            None,
        )
        .await;
        let prompt_cache_key = "pck-bodyless-header-bound-group";
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            INSERT INTO prompt_cache_conversation_bindings (
                prompt_cache_key,
                binding_kind,
                group_name,
                upstream_account_id,
                created_at,
                updated_at
            )
            VALUES (?1, 'group', ?2, NULL, ?3, ?3)
            "#,
        )
        .bind(prompt_cache_key)
        .bind(bound_group)
        .bind(&now_iso)
        .execute(&state.pool)
        .await
        .expect("insert bodyless header prompt cache group binding");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            5346,
            &"/v1/models".parse().expect("valid uri"),
            Method::GET,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-prompt-cache-key"),
                    HeaderValue::from_static(prompt_cache_key),
                ),
            ]),
            Body::empty(),
            runtime_timeouts,
            None,
        )
        .await;
        let Err(response) = response else {
            panic!("bodyless header binding should not fail over outside its group");
        };

        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        let attempts = attempts
            .lock()
            .expect("lock bodyless header binding attempts");
        assert!(matches!(
            attempts.get("Bearer upstream-primary").copied(),
            Some(count) if count > 0
        ));
        assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn proxy_openai_v1_bodyless_header_prompt_cache_key_preserves_encrypted_owner_lock() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Encrypted Owner",
        "upstream-owner",
        None,
        None,
        None,
    )
    .await;
    let _secondary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "upstream-secondary",
        None,
        None,
        None,
    )
    .await;
    set_test_account_status(&state.pool, owner_account_id, "needs_reauth").await;
    let prompt_cache_key = "pck-bodyless-header-encrypted-owner-lock";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist encrypted owner lock");

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = run_future_with_large_stack_async({
        let state = state.clone();
        async move {
            proxy_openai_v1_via_pool(
                state,
                5347,
                &"/v1/models".parse().expect("valid uri"),
                Method::GET,
                HeaderMap::from_iter([
                    (
                        http_header::AUTHORIZATION,
                        HeaderValue::from_static("Bearer pool-live-key"),
                    ),
                    (
                        HeaderName::from_static("x-prompt-cache-key"),
                        HeaderValue::from_static(prompt_cache_key),
                    ),
                ]),
                Body::empty(),
                runtime_timeouts,
                None,
            )
            .await
            .expect_err("bodyless encrypted owner lock should not reroute to another account")
        }
    })
    .await;

    assert_encrypted_owner_blocked_proxy_error(
        &response,
        owner_account_id,
        format!("#{owner_account_id}"),
        prompt_cache_key,
    );

    let attempts = attempts
        .lock()
        .expect("lock bodyless encrypted owner lock attempts");
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_bodyless_header_prompt_cache_key_rate_limited_owner_returns_owner_unavailable()
 {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Encrypted Owner",
        "upstream-owner",
        None,
        None,
        None,
    )
    .await;
    let _secondary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "upstream-secondary",
        None,
        None,
        None,
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_concurrency_limit = 1 WHERE id = ?1")
        .bind(owner_account_id)
        .execute(&state.pool)
        .await
        .expect("set encrypted owner account concurrency limit");
    let now_iso = format_utc_iso(Utc::now());
    insert_test_pool_limit_sample(&state, owner_account_id, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "pck-bodyless-header-encrypted-owner-rate-limited-active",
        owner_account_id,
        &now_iso,
    )
    .await
    .expect("seed active sticky route for rate-limited owner");
    let prompt_cache_key = "pck-bodyless-header-encrypted-owner-rate-limited";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist encrypted owner lock");

    let (binding_constraint, owner_auto_guard_active) =
        load_via_pool_effective_routing_constraint(state.as_ref(), Some(prompt_cache_key), false)
            .await
            .expect("load rate-limited encrypted owner routing constraint");
    assert!(matches!(
        binding_constraint,
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(account_id))
            if account_id == owner_account_id
    ));
    assert!(owner_auto_guard_active);

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = run_future_with_large_stack_async({
        let state = state.clone();
        async move {
            proxy_openai_v1_via_pool(
                state,
                5348,
                &"/v1/models".parse().expect("valid uri"),
                Method::GET,
                HeaderMap::from_iter([
                    (
                        http_header::AUTHORIZATION,
                        HeaderValue::from_static("Bearer pool-live-key"),
                    ),
                    (
                        HeaderName::from_static("x-prompt-cache-key"),
                        HeaderValue::from_static(prompt_cache_key),
                    ),
                ]),
                Body::empty(),
                runtime_timeouts,
                None,
            )
            .await
            .expect_err("rate-limited encrypted owner lock should not reroute to another account")
        }
    })
    .await;

    assert_encrypted_owner_blocked_proxy_error(
        &response,
        owner_account_id,
        format!("#{owner_account_id}"),
        prompt_cache_key,
    );

    let attempts = attempts
        .lock()
        .expect("lock bodyless encrypted owner rate-limited attempts");
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_bodyless_header_prompt_cache_key_same_account_binding_newer_than_owner_still_returns_owner_unavailable()
 {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Encrypted Owner",
        "upstream-owner",
        None,
        None,
        None,
    )
    .await;
    let _secondary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "upstream-secondary",
        None,
        None,
        None,
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_concurrency_limit = 1 WHERE id = ?1")
        .bind(owner_account_id)
        .execute(&state.pool)
        .await
        .expect("set encrypted owner account concurrency limit");
    let now_iso = format_utc_iso(Utc::now());
    insert_test_pool_limit_sample(&state, owner_account_id, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "pck-bodyless-header-encrypted-owner-same-account-active",
        owner_account_id,
        &now_iso,
    )
    .await
    .expect("seed active sticky route for same-account owner");
    let prompt_cache_key = "pck-bodyless-header-encrypted-owner-same-account";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist encrypted owner lock");
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key,
            binding_kind,
            group_name,
            upstream_account_id,
            created_at,
            updated_at
        )
        VALUES (?1, 'upstream_account', NULL, ?2, datetime('now', '+1 second'), datetime('now', '+1 second'))
        ON CONFLICT(prompt_cache_key) DO UPDATE SET
            binding_kind = excluded.binding_kind,
            group_name = NULL,
            upstream_account_id = excluded.upstream_account_id,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(prompt_cache_key)
    .bind(owner_account_id)
    .execute(&state.pool)
    .await
    .expect("persist same-account binding newer than owner");

    let (binding_constraint, owner_auto_guard_active) =
        load_via_pool_effective_routing_constraint(state.as_ref(), Some(prompt_cache_key), false)
            .await
            .expect("load same-account encrypted owner routing constraint");
    assert!(matches!(
        binding_constraint,
        Some(PromptCacheConversationBindingConstraint::UpstreamAccount(account_id))
            if account_id == owner_account_id
    ));
    assert!(owner_auto_guard_active);

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = run_future_with_large_stack_async({
        let state = state.clone();
        async move {
            proxy_openai_v1_via_pool(
                state,
                5349,
                &"/v1/models".parse().expect("valid uri"),
                Method::GET,
                HeaderMap::from_iter([
                    (
                        http_header::AUTHORIZATION,
                        HeaderValue::from_static("Bearer pool-live-key"),
                    ),
                    (
                        HeaderName::from_static("x-prompt-cache-key"),
                        HeaderValue::from_static(prompt_cache_key),
                    ),
                ]),
                Body::empty(),
                runtime_timeouts,
                None,
            )
            .await
            .expect_err("same-account newer binding should still keep encrypted owner guard")
        }
    })
    .await;

    assert_encrypted_owner_blocked_proxy_error(
        &response,
        owner_account_id,
        format!("#{owner_account_id}"),
        prompt_cache_key,
    );

    let attempts = attempts
        .lock()
        .expect("lock same-account newer binding owner attempts");
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_prepare_preserves_encrypted_owner_lock() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Encrypted Owner",
        "upstream-owner",
        None,
        None,
        None,
    )
    .await;
    let _secondary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "upstream-secondary",
        None,
        None,
        None,
    )
    .await;
    set_test_account_status(&state.pool, owner_account_id, "needs_reauth").await;
    let prompt_cache_key = "pck-websocket-encrypted-owner-lock";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist encrypted owner lock");
    let (binding_constraint, owner_auto_guard_active) =
        load_via_pool_effective_routing_constraint(state.as_ref(), Some(prompt_cache_key), false)
            .await
            .expect("load websocket effective routing constraint");

    let err = prepare_upstream_websocket(
        state.clone(),
        5351,
        &"/v1/realtime?model=gpt-5-realtime"
            .parse()
            .expect("valid uri"),
        &HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-prompt-cache-key"),
                HeaderValue::from_static(prompt_cache_key),
            ),
        ]),
        &resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts"),
        Some(prompt_cache_key),
        Some("gpt-5-realtime"),
        Some(prompt_cache_key),
        binding_constraint,
        None,
        owner_auto_guard_active,
        &PoolUpstreamAttemptTraceContext {
            invoke_id: "pool-ws-5351".to_string(),
            occurred_at: shanghai_now_string(),
            endpoint: "/v1/realtime".to_string(),
            sticky_key: Some(prompt_cache_key.to_string()),
            requester_ip: None,
            upstream_base_url_host: None,
            request_model: None,
        },
        None,
    )
    .await;
    let Err(err) = err else {
        panic!("websocket encrypted owner lock should not reroute");
    };
    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(err.message, ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE);

    let attempts = attempts
        .lock()
        .expect("lock websocket encrypted owner lock attempts");
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_prepare_rate_limited_owner_returns_owner_unavailable() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Encrypted Owner",
        "upstream-owner",
        None,
        None,
        None,
    )
    .await;
    let _secondary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "upstream-secondary",
        None,
        None,
        None,
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_concurrency_limit = 1 WHERE id = ?1")
        .bind(owner_account_id)
        .execute(&state.pool)
        .await
        .expect("set websocket encrypted owner account concurrency limit");
    insert_test_pool_limit_sample(&state, owner_account_id, Some(20.0), Some(20.0)).await;
    let prompt_cache_key = "pck-websocket-encrypted-owner-rate-limited";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist encrypted owner lock");
    let (binding_constraint, owner_auto_guard_active) =
        load_via_pool_effective_routing_constraint(state.as_ref(), Some(prompt_cache_key), false)
            .await
            .expect("load websocket effective routing constraint");

    let err = prepare_upstream_websocket(
        state.clone(),
        5352,
        &"/v1/realtime?model=gpt-5-realtime"
            .parse()
            .expect("valid uri"),
        &HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-prompt-cache-key"),
                HeaderValue::from_static(prompt_cache_key),
            ),
        ]),
        &resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts"),
        Some(prompt_cache_key),
        Some("gpt-5-realtime"),
        Some(prompt_cache_key),
        binding_constraint,
        None,
        owner_auto_guard_active,
        &PoolUpstreamAttemptTraceContext {
            invoke_id: "pool-ws-5352".to_string(),
            occurred_at: shanghai_now_string(),
            endpoint: "/v1/realtime".to_string(),
            sticky_key: Some(prompt_cache_key.to_string()),
            requester_ip: None,
            upstream_base_url_host: None,
            request_model: None,
        },
        None,
    )
    .await;
    let Err(err) = err else {
        panic!("websocket encrypted owner rate limit should not reroute");
    };
    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(err.message, ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE);

    let attempts = attempts
        .lock()
        .expect("lock websocket encrypted owner rate-limited attempts");
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    let owner_unavailable_attempts: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE failure_kind = ?1
        "#,
    )
    .bind(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE)
    .fetch_one(&state.pool)
    .await
    .expect("load websocket encrypted owner rate-limited terminal attempt");
    assert_eq!(owner_unavailable_attempts, 0);

    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_prepare_no_candidate_persists_invocation_audit_without_attempt() {
    let (upstream_base, _attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket capacity holder",
        "upstream-websocket-capacity-holder",
        None,
        None,
        None,
    )
    .await;
    let model = "gpt-websocket-no-candidate";
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed websocket model route");
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable queue overflow mode");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(account_id)
    .bind(model)
    .execute(&state.pool)
    .await
    .expect("limit websocket model route to one reservation");

    let reservation = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/realtime",
        crate::ImageIntent::Unknown,
        false,
        Some("websocket-no-candidate-holder"),
    )
    .await
    .expect("reserve only websocket candidate");
    assert!(matches!(reservation, PoolAccountResolution::Resolved(_)));

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pool-ws-no-candidate-audit".to_string(),
        occurred_at: shanghai_now_string(),
        endpoint: "/v1/realtime".to_string(),
        sticky_key: None,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: Some(model.to_string()),
    };
    let result = prepare_upstream_websocket(
        state.clone(),
        5354,
        &format!("/v1/realtime?model={model}")
            .parse()
            .expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        &resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts"),
        None,
        Some(model),
        None,
        None,
        None,
        false,
        &trace,
        None,
    )
    .await;
    let Err(err) = result else {
        panic!("websocket capacity conflict should not connect upstream");
    };
    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind(&trace.invoke_id)
            .fetch_one(&state.pool)
            .await
            .expect("load websocket no-candidate invocation payload");
    let payload: Value = serde_json::from_str(&payload).expect("decode invocation payload");
    assert_eq!(payload["poolAttemptCount"], 0);
    assert_eq!(
        payload["poolRoutingNoCandidateAudit"]["terminalReasonCode"],
        "modelConcurrencyLimit"
    );
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id = ?1",
    )
    .bind(&trace.invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("count websocket no-candidate attempts");
    assert_eq!(attempt_count, 0);

    release_pool_routing_reservation(&state, "websocket-no-candidate-holder");
    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_owner_guard_no_candidate_persists_invocation_audit_without_attempt() {
    let (upstream_base, _attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket owner capacity holder",
        "upstream-websocket-owner-holder",
        None,
        None,
        None,
    )
    .await;
    let model = "gpt-websocket-owner-no-candidate";
    let prompt_cache_key = "pck-websocket-owner-no-candidate";
    observe_model_route_seen(&state.pool, owner_account_id, Some(model))
        .await
        .expect("seed websocket owner model route");
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist websocket encrypted owner");
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable queue overflow mode");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(owner_account_id)
    .bind(model)
    .execute(&state.pool)
    .await
    .expect("limit websocket owner model route to one reservation");
    let holder = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &[],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/realtime",
        crate::ImageIntent::Unknown,
        false,
        Some("websocket-owner-no-candidate-holder"),
    )
    .await
    .expect("reserve only websocket owner candidate");
    assert!(matches!(holder, PoolAccountResolution::Resolved(_)));
    let (binding_constraint, owner_auto_guard_active) =
        load_via_pool_effective_routing_constraint(state.as_ref(), Some(prompt_cache_key), false)
            .await
            .expect("load websocket owner routing constraint");
    assert!(owner_auto_guard_active);

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pool-ws-owner-no-candidate-audit".to_string(),
        occurred_at: shanghai_now_string(),
        endpoint: "/v1/realtime".to_string(),
        sticky_key: Some(prompt_cache_key.to_string()),
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: Some(model.to_string()),
    };
    let result = prepare_upstream_websocket(
        state.clone(),
        5356,
        &format!("/v1/realtime?model={model}")
            .parse()
            .expect("valid uri"),
        &HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-prompt-cache-key"),
                HeaderValue::from_static(prompt_cache_key),
            ),
        ]),
        &resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts"),
        Some(prompt_cache_key),
        Some(model),
        Some(prompt_cache_key),
        binding_constraint,
        None,
        owner_auto_guard_active,
        &trace,
        None,
    )
    .await;
    let Err(err) = result else {
        panic!("websocket owner capacity conflict should not connect upstream");
    };
    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(err.message, ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind(&trace.invoke_id)
            .fetch_one(&state.pool)
            .await
            .expect("load websocket owner no-candidate invocation payload");
    let payload: Value = serde_json::from_str(&payload).expect("decode invocation payload");
    assert_eq!(payload["poolAttemptCount"], 0);
    assert_eq!(
        payload["failureKind"],
        PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE
    );
    assert_eq!(
        payload["downstreamErrorMessage"],
        ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE
    );
    assert_eq!(
        payload["poolAttemptTerminalReason"],
        PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE
    );
    assert_eq!(
        payload["poolRoutingNoCandidateAudit"]["terminalReasonCode"],
        "modelConcurrencyLimit"
    );
    let (failure_kind, error_message): (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT failure_kind, error_message FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind(&trace.invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load websocket owner no-candidate terminal fields");
    assert_eq!(
        failure_kind.as_deref(),
        Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE)
    );
    assert!(
        error_message.as_deref().is_some_and(|message| {
            message.contains(ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE)
        })
    );
    let invocation_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
            .bind(&trace.invoke_id)
            .fetch_one(&state.pool)
            .await
            .expect("load websocket owner invocation id");
    let Json(workflow) =
        fetch_invocation_workflow_detail(State(state.clone()), axum::extract::Path(invocation_id))
            .await
            .expect("load websocket owner workflow detail");
    let workflow = serde_json::to_value(workflow).expect("serialize workflow detail");
    let final_failure = workflow["timeline"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["kind"] == "systemFinalFailure")
        })
        .expect("workflow should contain a system final failure");
    assert_eq!(final_failure["responseBody"]["available"], true);
    let response_body = final_failure["responseBody"]["bodyText"]
        .as_str()
        .expect("workflow final failure should expose its body");
    let response_body: Value =
        serde_json::from_str(response_body).expect("decode workflow failure body");
    assert_eq!(
        response_body["error"],
        ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE
    );
    assert_eq!(
        response_body["code"],
        PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE
    );
    let attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id = ?1",
    )
    .bind(&trace.invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("count websocket owner no-candidate attempts");
    assert_eq!(attempt_count, 0);

    release_pool_routing_reservation(&state, "websocket-owner-no-candidate-holder");
    upstream_handle.abort();
}

#[test]
fn http_prebuffered_no_candidate_persists_invocation_audit_without_attempt() {
    run_future_with_large_stack(async move {
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(10),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "HTTP capacity holder",
            "upstream-http-capacity-holder",
            None,
            None,
            None,
        )
        .await;
        let model = "gpt-http-no-candidate";
        observe_model_route_seen(&state.pool, account_id, Some(model))
            .await
            .expect("seed HTTP model route");
        sqlx::query(
            "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
        )
        .execute(&state.pool)
        .await
        .expect("enable queue overflow mode");
        sqlx::query(
            "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1 WHERE account_id = ?1 AND model = ?2",
        )
        .bind(account_id)
        .bind(model)
        .execute(&state.pool)
        .await
        .expect("limit HTTP model route to one reservation");
        let holder = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
            &state,
            None,
            Some(model),
            &[],
            &HashSet::new(),
            None,
            None,
            None,
            "/v1/responses",
            crate::ImageIntent::Unknown,
            false,
            Some("http-no-candidate-holder"),
        )
        .await
        .expect("reserve only HTTP candidate");
        assert!(matches!(holder, PoolAccountResolution::Resolved(_)));

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let err = proxy_openai_v1_via_pool(
            state.clone(),
            5355,
            &"/v1/responses".parse().expect("valid uri"),
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
            Body::from(format!(
                r#"{{"model":"{model}","stream":false,"input":"hello"}}"#
            )),
            runtime_timeouts,
            None,
        )
        .await
        .expect_err("occupied HTTP model route should return no candidate");
        assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);

        let payload: String = sqlx::query_scalar(
            "SELECT payload FROM codex_invocations WHERE invoke_id = 'pool-via-5355'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("load HTTP no-candidate invocation payload");
        let payload: Value = serde_json::from_str(&payload).expect("decode invocation payload");
        assert_eq!(payload["poolAttemptCount"], 0);
        assert_eq!(payload["requestModel"], model);
        assert_eq!(
            payload["poolRoutingNoCandidateAudit"]["terminalReasonCode"],
            "modelConcurrencyLimit"
        );
        let attempt_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id = 'pool-via-5355'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("count HTTP no-candidate attempts");
        assert_eq!(attempt_count, 0);

        let (invocation_id, persisted_model): (i64, Option<String>) = sqlx::query_as(
            "SELECT id, model FROM codex_invocations WHERE invoke_id = 'pool-via-5355'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("load HTTP no-candidate invocation model");
        assert_eq!(persisted_model.as_deref(), Some(model));
        let Json(workflow) = fetch_invocation_workflow_detail(
            State(state.clone()),
            axum::extract::Path(invocation_id),
        )
        .await
        .expect("load HTTP no-candidate workflow detail");
        let workflow = serde_json::to_value(workflow).expect("serialize workflow detail");
        let final_failure = workflow["timeline"]
            .as_array()
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry["kind"] == "systemFinalFailure")
            })
            .expect("workflow should expose the zero-attempt final failure");
        assert_eq!(final_failure["responseBody"]["available"], true);
        assert!(final_failure["responseBody"]["bodyText"].as_str().is_some());

        release_pool_routing_reservation(&state, "http-no-candidate-holder");
    });
}

#[tokio::test]
async fn healthy_pool_success_does_not_publish_availability_but_recovery_does() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Availability Gate", "availability-gate-key")
            .await;
    let availability = state.pool_routing_availability.subscribe();
    let initial_generation = *availability.borrow();

    record_pool_route_success_with_affinity_generation_and_broadcast(
        state.as_ref(),
        account_id,
        Utc::now(),
        None,
        None,
        Some("healthy-websocket-success"),
        None,
        None,
    )
    .await
    .expect("record healthy websocket-style success");
    record_pool_route_success_for_endpoint_with_image_intent_and_affinity_generation_for_attempt_and_broadcast(
        state.as_ref(),
        account_id,
        Utc::now(),
        None,
        None,
        Some("healthy-http-success"),
        "/v1/responses",
        crate::ImageIntent::Unknown,
        None,
        None,
        None,
    )
    .await
    .expect("record healthy HTTP-style success");
    assert_eq!(
        *availability.borrow(),
        initial_generation,
        "ordinary successful requests on an already healthy account must not wake pool waiters"
    );

    let failure_at = format_utc_iso(Utc::now() - chrono::Duration::seconds(1));
    sqlx::query(
        "UPDATE pool_upstream_accounts SET status = 'needs_reauth', last_error = 'temporary route failure', last_error_at = ?2, last_route_failure_at = ?2, last_route_failure_kind = 'temporary_http_5xx', cooldown_until = ?2, consecutive_route_failures = 1 WHERE id = ?1",
    )
    .bind(account_id)
    .bind(&failure_at)
    .execute(&state.pool)
    .await
    .expect("mark account unavailable before websocket-style recovery");
    record_pool_route_success_with_affinity_generation_and_broadcast(
        state.as_ref(),
        account_id,
        Utc::now(),
        None,
        None,
        Some("recovering-websocket-success"),
        None,
        None,
    )
    .await
    .expect("record websocket-style account recovery");
    let websocket_recovery_generation = *availability.borrow();
    assert_ne!(websocket_recovery_generation, initial_generation);

    let failure_at = format_utc_iso(Utc::now() - chrono::Duration::seconds(1));
    sqlx::query(
        "UPDATE pool_upstream_accounts SET status = 'needs_reauth', last_error = 'temporary route failure', last_error_at = ?2, last_route_failure_at = ?2, last_route_failure_kind = 'temporary_http_5xx', cooldown_until = ?2, consecutive_route_failures = 1 WHERE id = ?1",
    )
    .bind(account_id)
    .bind(&failure_at)
    .execute(&state.pool)
    .await
    .expect("mark account unavailable before HTTP-style recovery");
    record_pool_route_success_for_endpoint_with_image_intent_and_affinity_generation_for_attempt_and_broadcast(
        state.as_ref(),
        account_id,
        Utc::now(),
        None,
        None,
        Some("recovering-http-success"),
        "/v1/responses",
        crate::ImageIntent::Unknown,
        None,
        None,
        None,
    )
    .await
    .expect("record HTTP-style account recovery");
    assert_ne!(*availability.borrow(), websocket_recovery_generation);
}

#[tokio::test]
async fn disabled_or_deleted_account_recovery_does_not_publish_availability() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(
        &state,
        "Unavailable Recovery",
        "unavailable-recovery-key",
    )
    .await;
    let availability = state.pool_routing_availability.subscribe();
    let failure_at = format_utc_iso(Utc::now() - chrono::Duration::seconds(1));
    sqlx::query(
        "UPDATE pool_upstream_accounts SET enabled = 0, status = 'needs_reauth', last_error = 'temporary route failure', last_error_at = ?2, last_route_failure_at = ?2, last_route_failure_kind = 'temporary_http_5xx', cooldown_until = ?2, consecutive_route_failures = 1 WHERE id = ?1",
    )
    .bind(account_id)
    .bind(&failure_at)
    .execute(&state.pool)
    .await
    .expect("disable account before websocket-style recovery");
    let initial_generation = *availability.borrow();

    record_pool_route_success_with_affinity_generation_and_broadcast(
        state.as_ref(),
        account_id,
        Utc::now(),
        None,
        None,
        Some("disabled-websocket-style-recovery"),
        None,
        None,
    )
    .await
    .expect("record disabled websocket-style recovery");
    assert_eq!(
        *availability.borrow(),
        initial_generation,
        "a disabled account must not wake waiters after a recovery"
    );

    let failure_at = format_utc_iso(Utc::now() - chrono::Duration::seconds(1));
    sqlx::query(
        "UPDATE pool_upstream_accounts SET enabled = 1, deleted_at = ?3, status = 'needs_reauth', last_error = 'temporary route failure', last_error_at = ?2, last_route_failure_at = ?2, last_route_failure_kind = 'temporary_http_5xx', cooldown_until = ?2, consecutive_route_failures = 1 WHERE id = ?1",
    )
    .bind(account_id)
    .bind(&failure_at)
    .bind(shanghai_now_string())
    .execute(&state.pool)
    .await
    .expect("soft-delete account before HTTP-style recovery");

    record_pool_route_success_for_endpoint_with_image_intent_and_affinity_generation_for_attempt_and_broadcast(
        state.as_ref(),
        account_id,
        Utc::now(),
        None,
        None,
        Some("deleted-http-style-recovery"),
        "/v1/responses",
        crate::ImageIntent::Unknown,
        None,
        None,
        None,
    )
    .await
    .expect("record deleted HTTP-style recovery");
    assert_eq!(
        *availability.borrow(),
        initial_generation,
        "a deleted account must not wake waiters after a recovery"
    );
}

#[tokio::test]
async fn same_second_newer_account_failure_fences_success_without_publish() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(
        &state,
        "Same-second Failure Fence",
        "same-second-fence-key",
    )
    .await;
    let request_started_at = DateTime::parse_from_rfc3339("2026-08-19T02:03:04.100Z")
        .expect("valid request start")
        .with_timezone(&Utc);
    let newer_failure_at = "2026-08-19T02:03:04.900Z";
    let mut failure_tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("begin concurrent failure transaction");
    sqlx::query(
        "UPDATE pool_upstream_accounts SET status = 'needs_reauth', last_error = 'newer route failure', last_error_at = ?2, last_route_failure_at = ?2, last_route_failure_kind = 'temporary_http_5xx', cooldown_until = ?2, consecutive_route_failures = 1 WHERE id = ?1",
    )
    .bind(account_id)
    .bind(newer_failure_at)
    .execute(&mut *failure_tx)
    .await
    .expect("stage a failure later in the same second");

    let availability = state.pool_routing_availability.subscribe();
    let initial_generation = *availability.borrow();
    let success_state = state.clone();
    let success_task = tokio::spawn(async move {
        record_pool_route_success_with_affinity_generation_and_broadcast(
            success_state.as_ref(),
            account_id,
            request_started_at,
            None,
            None,
            Some("same-second-stale-success"),
            None,
            None,
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        !success_task.is_finished(),
        "success transaction must wait for the in-flight failure transaction"
    );
    failure_tx
        .commit()
        .await
        .expect("commit newer account failure");
    success_task
        .await
        .expect("join concurrent success recorder")
        .expect("stale success is ignored without failing persistence");

    let account: (String, Option<String>, Option<String>, i64) = sqlx::query_as(
        "SELECT status, last_error, last_route_failure_at, consecutive_route_failures FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load fenced account");
    assert_eq!(account.0, "needs_reauth");
    assert_eq!(account.1.as_deref(), Some("newer route failure"));
    assert_eq!(account.2.as_deref(), Some(newer_failure_at));
    assert_eq!(account.3, 1);
    assert_eq!(
        *availability.borrow(),
        initial_generation,
        "a stale success must neither recover the account nor wake pool waiters"
    );
}

#[tokio::test]
async fn legacy_second_precision_failure_in_request_second_fails_closed() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Legacy Failure Fence", "legacy-fence-key").await;
    let legacy_failure_at = "2026-08-19T02:03:04Z";
    sqlx::query(
        "UPDATE pool_upstream_accounts SET status = 'needs_reauth', last_error = 'ambiguous legacy failure', last_error_at = ?2, last_route_failure_at = ?2, last_route_failure_kind = 'temporary_http_5xx', consecutive_route_failures = 1 WHERE id = ?1",
    )
    .bind(account_id)
    .bind(legacy_failure_at)
    .execute(&state.pool)
    .await
    .expect("seed legacy second-precision failure");

    record_pool_route_success(
        &state.pool,
        account_id,
        DateTime::parse_from_rfc3339("2026-08-19T02:03:04.900Z")
            .expect("valid request start")
            .with_timezone(&Utc),
        None,
        Some("legacy-same-second-success"),
    )
    .await
    .expect("ambiguous legacy success is ignored");

    let stored_failure_at: Option<String> = sqlx::query_scalar(
        "SELECT last_route_failure_at FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load legacy failure fence");
    assert_eq!(stored_failure_at.as_deref(), Some(legacy_failure_at));
}

#[tokio::test]
async fn account_route_failure_writer_persists_millisecond_fence() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Precise Failure Fence", "precise-fence-key")
            .await;

    apply_pool_route_cooldown_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        None,
        "temporary upstream failure",
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX,
        StatusCode::BAD_GATEWAY,
        5,
        Some("precise-failure-fence"),
        None,
    )
    .await
    .expect("record precise account route failure");

    let failure_at: String = sqlx::query_scalar(
        "SELECT last_route_failure_at FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load precise account route failure");
    assert!(
        failure_at.contains('.'),
        "new account route failures must retain sub-second precision: {failure_at}"
    );
    let parsed_failure_at =
        parse_to_utc_datetime(&failure_at).expect("parse precise failure fence");
    let reloaded = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("reload precise failure row")
        .expect("precise failure row exists");
    assert_eq!(
        reloaded.last_route_failure_at.as_deref(),
        Some(failure_at.as_str())
    );
    assert_eq!(
        reloaded
            .last_route_failure_at
            .as_deref()
            .and_then(parse_to_utc_datetime),
        Some(parsed_failure_at),
        "SQLite reload must preserve the exact millisecond failure fence"
    );
}

#[tokio::test]
async fn shared_terminal_observer_handles_http_and_websocket_shaped_capture_records() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Cache Observer", "cache-observer-key").await;
    let model = "gpt-shared-terminal-cache-observer";
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable cache-hit protection");
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed shared observer route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        8,
    )
    .await
    .expect("make route cache-owned");

    let availability = state.pool_routing_availability.subscribe();
    let initial_generation = *availability.borrow();
    let metadata_free_record =
        test_proxy_capture_record("proxy-cache-observer-metadata-free", &shanghai_now_string());
    observe_successful_proxy_capture_model_route_cache(&state, &metadata_free_record).await;
    assert_eq!(
        *availability.borrow(),
        initial_generation,
        "a successful terminal without account/model metadata must not wake pool waiters"
    );

    let payload = json!({
        "endpoint": "/v1/responses",
        "requestModel": model,
        "routeMode": "pool",
        "upstreamAccountId": account_id,
    })
    .to_string();
    let mut http_record =
        test_proxy_capture_record("proxy-shared-cache-observer-http", &shanghai_now_string());
    http_record.model = Some(model.to_string());
    http_record.payload = Some(payload.clone());
    http_record.usage.input_tokens = None;
    http_record.usage.cache_input_tokens = None;
    observe_successful_proxy_capture_model_route_cache(&state, &http_record).await;
    assert_eq!(
        *availability.borrow(),
        initial_generation,
        "missing cache usage constrains capacity and must not wake pool waiters"
    );

    let missing_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load HTTP-observed route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("HTTP-observed route is visible");
    assert_eq!(missing_route.cache_concurrency_limit, Some(1));
    assert_eq!(
        missing_route.cache_usage_missing_reason.as_deref(),
        Some("missing_input_tokens")
    );

    let mut websocket_record = test_proxy_capture_record(
        "pool-ws-shared-cache-observer-websocket",
        &shanghai_now_string(),
    );
    websocket_record.model = Some(model.to_string());
    websocket_record.payload = Some(payload);
    websocket_record.usage.input_tokens = Some(3_840);
    websocket_record.usage.cache_input_tokens = Some(384);
    observe_successful_proxy_capture_model_route_cache(&state, &websocket_record).await;
    assert_ne!(
        *availability.borrow(),
        initial_generation,
        "a successful cache recovery that increases capacity must wake pool waiters"
    );

    let observed_route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load WebSocket-observed route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("WebSocket-observed route is visible");
    assert!(observed_route.cache_usage_missing_since.is_none());
    assert!(observed_route.cache_usage_missing_reason.is_none());
    assert_eq!(observed_route.cache_concurrency_limit, Some(2));
}

#[tokio::test]
async fn http_and_websocket_cache_recovery_do_not_publish_through_account_failure_fence() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(
        &state,
        "Terminal Account Fence",
        "terminal-account-fence-key",
    )
    .await;
    let model = "gpt-terminal-account-fence";
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable cache-hit protection");
    observe_model_route_seen(&state.pool, account_id, Some(model))
        .await
        .expect("seed model route");
    observe_model_route_cache_hit(
        &state.pool,
        account_id,
        Some(model),
        Some(3_840),
        Some(0),
        8,
    )
    .await
    .expect("make route cache-owned");

    let payload = json!({
        "endpoint": "/v1/responses",
        "requestModel": model,
        "routeMode": "pool",
        "upstreamAccountId": account_id,
    })
    .to_string();
    let mut missing_usage =
        test_proxy_capture_record("proxy-terminal-account-fence-seed", &shanghai_now_string());
    missing_usage.model = Some(model.to_string());
    missing_usage.payload = Some(payload.clone());
    missing_usage.usage.input_tokens = None;
    missing_usage.usage.cache_input_tokens = None;
    observe_successful_proxy_capture_model_route_cache(&state, &missing_usage).await;

    let failure_at = format_utc_iso_millis(Utc::now());
    sqlx::query(
        "UPDATE pool_upstream_accounts SET status = 'active', last_error = 'newer account route failure', last_error_at = ?2, last_route_failure_at = ?2, last_route_failure_kind = 'temporary_http_5xx', consecutive_route_failures = 1 WHERE id = ?1",
    )
    .bind(account_id)
    .bind(&failure_at)
    .execute(&state.pool)
    .await
    .expect("record account failure after request start");

    let availability = state.pool_routing_availability.subscribe();
    let initial_generation = *availability.borrow();
    for invoke_id in [
        "proxy-terminal-account-fence-http",
        "pool-ws-terminal-account-fence-websocket",
    ] {
        let mut record = test_proxy_capture_record(invoke_id, &shanghai_now_string());
        record.model = Some(model.to_string());
        record.payload = Some(payload.clone());
        record.usage.input_tokens = Some(3_840);
        record.usage.cache_input_tokens = Some(384);
        observe_successful_proxy_capture_model_route_cache(&state, &record).await;
        assert_eq!(
            *availability.borrow(),
            initial_generation,
            "{invoke_id} must not wake waiters through a newer account failure"
        );
    }

    let route = load_model_routing_states(&state.pool, account_id)
        .await
        .expect("load independently observed model route")
        .into_iter()
        .find(|route| route.model == model)
        .expect("model route exists");
    assert_eq!(route.cache_concurrency_limit, Some(3));
    let account_failure: Option<String> = sqlx::query_scalar(
        "SELECT last_route_failure_at FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load account failure fence");
    assert_eq!(account_failure.as_deref(), Some(failure_at.as_str()));
}

#[tokio::test]
async fn disabled_or_deleted_accounts_cannot_publish_model_route_availability() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(
        &state,
        "Availability Eligibility",
        "availability-eligibility-key",
    )
    .await;
    assert!(
        pool_account_allows_model_route_availability_publish(&state.pool, account_id)
            .await
            .expect("check active account")
    );

    sqlx::query("UPDATE pool_upstream_accounts SET enabled = 0 WHERE id = ?1")
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("disable account");
    assert!(
        !pool_account_allows_model_route_availability_publish(&state.pool, account_id)
            .await
            .expect("check disabled account")
    );

    sqlx::query("UPDATE pool_upstream_accounts SET enabled = 1, deleted_at = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind(shanghai_now_string())
        .execute(&state.pool)
        .await
        .expect("soft-delete account");
    assert!(
        !pool_account_allows_model_route_availability_publish(&state.pool, account_id)
            .await
            .expect("check deleted account")
    );
}

#[tokio::test]
async fn websocket_payload_owner_guard_blocks_mismatched_payload_owner() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    let owner_account_id =
        insert_test_pool_api_key_account(&state, "Owner", "upstream-owner").await;
    let secondary_account_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    upsert_prompt_cache_encrypted_session_owner(
        &state.pool,
        "pck-websocket-payload-owner",
        owner_account_id,
    )
    .await
    .expect("persist websocket payload owner");

    let secondary_account = PoolResolvedAccount {
        account_id: secondary_account_id,
        display_name: "Secondary".to_string(),
        kind: "api_key".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer upstream-secondary".to_string(),
        },
        group_name: None,
        bound_proxy_keys: Vec::new(),
        forward_proxy_scope: ForwardProxyRouteScope::Automatic,
        single_account_rotation_enabled: false,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::default(),
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
        upstream_base_url: Url::parse("https://api.example.test").expect("valid base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        sticky_affinity_generation: None,
        routing_selection_audit: None,
    };

    let outcome = inspect_ws_request_payload_guard(
        state.as_ref(),
        &secondary_account,
        None,
        br#"{"type":"conversation.item.create","promptCacheKey":"pck-websocket-payload-owner","item":{"type":"message","content":[{"type":"encrypted_content","encrypted_content":"opaque"}]}}"#,
    )
    .await
    .expect("inspect websocket payload guard");

    assert_eq!(
        outcome.prompt_cache_key.as_deref(),
        Some("pck-websocket-payload-owner")
    );
    assert!(outcome.contains_encrypted_content);
    assert!(outcome.owner_guard_blocked);
}

#[tokio::test]
async fn encrypted_owner_routing_disabled_ignores_existing_owner_guard() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.encrypted_session_owner_routing_enabled = false;
    }
    let owner_account_id =
        insert_test_pool_api_key_account(&state, "Owner", "upstream-owner").await;
    let prompt_cache_key = "pck-owner-routing-disabled";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist encrypted owner");

    let (constraint, owner_auto_guard_active) =
        load_via_pool_effective_routing_constraint(state.as_ref(), Some(prompt_cache_key), true)
            .await
            .expect("resolve prompt-cache routing with owner routing disabled");

    assert!(constraint.is_none());
    assert!(!owner_auto_guard_active);
}

#[tokio::test]
async fn encrypted_owner_routing_disabled_does_not_persist_success_owner() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.encrypted_session_owner_routing_enabled = false;
    }
    let account_id =
        insert_test_pool_api_key_account(&state, "Owner Disabled", "upstream-owner").await;
    let prompt_cache_key = "pck-owner-routing-disabled-success";

    let updated = confirm_prompt_cache_encrypted_session_owner_success_if_enabled(
        state.as_ref(),
        prompt_cache_key,
        account_id,
    )
    .await
    .expect("disabled owner routing should not fail");
    assert!(!updated);

    let owner: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT owner_upstream_account_id
        FROM prompt_cache_encrypted_session_owners
        WHERE prompt_cache_key = ?1
        "#,
    )
    .bind(prompt_cache_key)
    .fetch_optional(&state.pool)
    .await
    .expect("query encrypted owner row");
    assert_eq!(owner, None);
}

#[tokio::test]
async fn websocket_payload_owner_guard_disabled_does_not_block_mismatched_payload_owner() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.encrypted_session_owner_routing_enabled = false;
    }
    let owner_account_id =
        insert_test_pool_api_key_account(&state, "Owner", "upstream-owner").await;
    let secondary_account_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    upsert_prompt_cache_encrypted_session_owner(
        &state.pool,
        "pck-websocket-owner-routing-disabled",
        owner_account_id,
    )
    .await
    .expect("persist websocket payload owner");

    let secondary_account = PoolResolvedAccount {
        account_id: secondary_account_id,
        display_name: "Secondary".to_string(),
        kind: "api_key".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer upstream-secondary".to_string(),
        },
        group_name: None,
        bound_proxy_keys: Vec::new(),
        forward_proxy_scope: ForwardProxyRouteScope::Automatic,
        single_account_rotation_enabled: false,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::default(),
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
        upstream_base_url: Url::parse("https://api.example.test").expect("valid base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        sticky_affinity_generation: None,
        routing_selection_audit: None,
    };

    let outcome = inspect_ws_request_payload_guard(
        state.as_ref(),
        &secondary_account,
        None,
        br#"{"type":"conversation.item.create","promptCacheKey":"pck-websocket-owner-routing-disabled","item":{"type":"message","content":[{"type":"encrypted_content","encrypted_content":"opaque"}]}}"#,
    )
    .await
    .expect("inspect websocket payload guard");

    assert_eq!(
        outcome.prompt_cache_key.as_deref(),
        Some("pck-websocket-owner-routing-disabled")
    );
    assert!(outcome.contains_encrypted_content);
    assert!(!outcome.owner_guard_blocked);
}

#[tokio::test]
async fn websocket_payload_only_prompt_cache_key_routes_first_upgrade_to_owner_account() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tungstenite::Message as TungsteniteMessage;

    async fn websocket_echo_upstream(
        ws: WebSocketUpgrade,
        State(attempts): State<Arc<StdMutex<HashMap<String, usize>>>>,
        headers: HeaderMap,
    ) -> Response {
        let authorization = headers
            .get(http_header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        {
            let mut attempts = attempts.lock().expect("lock websocket attempts");
            let entry = attempts.entry(authorization.clone()).or_insert(0);
            *entry += 1;
        }

        ws.on_upgrade(move |mut socket| async move {
            let response = json!({
                "type": "response.completed",
                "response": {
                    "status": "completed",
                    "output": [{
                        "type": "encrypted_content",
                        "encrypted_content": "opaque-owner-bound-content"
                    }],
                    "usage": {
                        "input_tokens": 3,
                        "output_tokens": 2,
                        "total_tokens": 5
                    }
                },
                "authorization": authorization
            })
            .to_string();
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    AxumWsMessage::Text(_) | AxumWsMessage::Binary(_) => {
                        let _ = socket.send(AxumWsMessage::Text(response.clone())).await;
                        let _ = socket.send(AxumWsMessage::Close(None)).await;
                        break;
                    }
                    AxumWsMessage::Close(_) => break,
                    _ => {}
                }
            }
        })
        .into_response()
    }

    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let upstream_app = Router::new()
        .route("/v1/responses", get(websocket_echo_upstream))
        .with_state(attempts.clone());
    let upstream_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket upstream");
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("websocket upstream addr");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(upstream_listener, upstream_app)
            .await
            .expect("websocket upstream should run");
    });

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&format!("http://{upstream_addr}")).expect("valid websocket upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    enable_encrypted_session_owner_routing_for_test(&state).await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket Owner",
        "upstream-owner",
        None,
        None,
        Some(&format!("http://{upstream_addr}")),
    )
    .await;
    let secondary_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket Secondary",
        "upstream-secondary",
        None,
        None,
        Some(&format!("http://{upstream_addr}")),
    )
    .await;
    let prompt_cache_key = "pck-websocket-payload-first-upgrade-owner";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, prompt_cache_key, owner_account_id)
        .await
        .expect("persist websocket payload owner");
    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(
        &state.pool,
        "sticky-websocket-secondary-preferred",
        secondary_account_id,
        &now_iso,
    )
    .await
    .expect("seed sticky route toward secondary");

    let proxy_app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone());
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket proxy server");
    let proxy_addr = proxy_listener
        .local_addr()
        .expect("websocket proxy server addr");
    let proxy_handle = tokio::spawn(async move {
        axum::serve(proxy_listener, proxy_app)
            .await
            .expect("websocket proxy server should run");
    });

    let request = format!("ws://{proxy_addr}/v1/responses?model=gpt-5-realtime")
        .into_client_request()
        .expect("websocket client request");
    let mut request = request;
    request.headers_mut().insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer pool-live-key"),
    );
    request.headers_mut().insert(
        HeaderName::from_static("x-sticky-key"),
        HeaderValue::from_static("sticky-websocket-secondary-preferred"),
    );
    let (mut client, response) = connect_async(request)
        .await
        .expect("connect websocket proxy");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    client
        .send(TungsteniteMessage::Text(
            json!({
                "type": "response.create",
                "model": "gpt-5-realtime",
                "promptCacheKey": prompt_cache_key,
                "input": [{
                    "role": "user",
                    "content": [{
                        "type": "encrypted_content",
                        "encrypted_content": "opaque"
                    }]
                }]
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send websocket prompt-cache payload");

    let message = client
        .next()
        .await
        .expect("receive websocket response")
        .expect("websocket response frame");
    let text = match message {
        TungsteniteMessage::Text(text) => text.to_string(),
        other => panic!("expected text websocket response, got {other:?}"),
    };
    let payload: Value = serde_json::from_str(&text).expect("decode websocket response");
    assert_eq!(payload["authorization"], "Bearer upstream-owner");

    let attempts = attempts.lock().expect("lock websocket owner attempts");
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    proxy_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_realtime_passthrough_does_not_wait_for_response_create_first_frame() {
    use futures_util::StreamExt;
    use tokio_tungstenite::connect_async;
    use tungstenite::Message as TungsteniteMessage;

    async fn realtime_session_upstream(ws: WebSocketUpgrade) -> Response {
        ws.on_upgrade(move |mut socket| async move {
            let _ = socket
                .send(AxumWsMessage::Text(
                    json!({
                        "type": "session.created",
                        "session": {
                            "id": "sess_passthrough",
                            "model": "gpt-5-realtime"
                        }
                    })
                    .to_string(),
                ))
                .await;
            let _ = socket.send(AxumWsMessage::Close(None)).await;
        })
        .into_response()
    }

    let upstream_app = Router::new().route("/v1/realtime", get(realtime_session_upstream));
    let upstream_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind realtime websocket upstream");
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("realtime websocket upstream addr");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(upstream_listener, upstream_app)
            .await
            .expect("realtime websocket upstream should run");
    });

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&format!("http://{upstream_addr}")).expect("valid websocket upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    config.openai_proxy_request_read_timeout = Duration::from_millis(80);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Realtime WebSocket",
        "upstream-realtime",
        None,
        None,
        Some(&format!("http://{upstream_addr}")),
    )
    .await;

    let proxy_app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone());
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind realtime websocket proxy server");
    let proxy_addr = proxy_listener
        .local_addr()
        .expect("realtime websocket proxy server addr");
    let proxy_handle = tokio::spawn(async move {
        axum::serve(proxy_listener, proxy_app)
            .await
            .expect("realtime websocket proxy server should run");
    });

    let mut request = format!("ws://{proxy_addr}/v1/realtime?model=gpt-5-realtime")
        .into_client_request()
        .expect("realtime websocket client request");
    request.headers_mut().insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer pool-live-key"),
    );

    let (mut client, response) = connect_async(request)
        .await
        .expect("connect realtime websocket proxy");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    let message = timeout(Duration::from_secs(1), client.next())
        .await
        .expect("realtime proxy should not wait for downstream response.create")
        .expect("receive realtime upstream message")
        .expect("realtime upstream message should be ok");
    let TungsteniteMessage::Text(text) = message else {
        panic!("expected realtime session.created text frame, got {message:?}");
    };
    let payload: Value = serde_json::from_str(&text).expect("decode realtime session event");
    assert_eq!(payload["type"], "session.created");
    assert_eq!(payload["session"]["id"], "sess_passthrough");

    proxy_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_response_first_frame_rejection_avoids_pseudo_attempt_persistence() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tungstenite::Message as TungsteniteMessage;

    let mut config = test_config();
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;

    let proxy_app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone());
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket proxy server");
    let proxy_addr = proxy_listener
        .local_addr()
        .expect("websocket proxy server addr");
    let proxy_handle = tokio::spawn(async move {
        axum::serve(proxy_listener, proxy_app)
            .await
            .expect("websocket proxy server should run");
    });

    let mut request = format!("ws://{proxy_addr}/v1/responses")
        .into_client_request()
        .expect("websocket client request");
    request.headers_mut().insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer pool-live-key"),
    );
    let (mut client, response) = connect_async(request)
        .await
        .expect("connect websocket proxy");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    client
        .send(TungsteniteMessage::Text(
            json!({
                "type": "conversation.item.create",
                "item": { "type": "message" }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send rejected websocket first frame");

    let close = client
        .next()
        .await
        .expect("receive websocket close")
        .expect("websocket close frame");
    let TungsteniteMessage::Close(Some(frame)) = close else {
        panic!("expected websocket close frame, got {close:?}");
    };
    assert!(frame.reason.contains("response.create"));

    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

    proxy_handle.abort();
}

#[tokio::test]
async fn websocket_response_create_turns_persist_usage_per_terminal() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tungstenite::Message as TungsteniteMessage;

    async fn websocket_multi_turn_upstream(
        ws: WebSocketUpgrade,
        State(turns): State<Arc<AtomicUsize>>,
    ) -> Response {
        ws.on_upgrade(move |mut socket| async move {
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    AxumWsMessage::Text(_) | AxumWsMessage::Binary(_) => {
                        let turn = turns.fetch_add(1, Ordering::SeqCst) + 1;
                        let response = json!({
                            "type": "response.completed",
                            "response": {
                                "id": format!("resp_ws_turn_{turn}"),
                                "status": "completed",
                                "model": "gpt-5-realtime",
                                "usage": {
                                    "input_tokens": 10 + turn as i64,
                                    "output_tokens": 2 + turn as i64,
                                    "total_tokens": 12 + (turn as i64 * 2)
                                }
                            }
                        })
                        .to_string();
                        let _ = socket.send(AxumWsMessage::Text(response)).await;
                        if turn >= 2 {
                            let _ = socket.send(AxumWsMessage::Close(None)).await;
                            break;
                        }
                    }
                    AxumWsMessage::Close(_) => break,
                    _ => {}
                }
            }
        })
        .into_response()
    }

    let turns = Arc::new(AtomicUsize::new(0));
    let upstream_app = Router::new()
        .route("/v1/responses", get(websocket_multi_turn_upstream))
        .with_state(turns.clone());
    let upstream_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket upstream");
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("websocket upstream addr");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(upstream_listener, upstream_app)
            .await
            .expect("websocket upstream should run");
    });

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&format!("http://{upstream_addr}")).expect("valid websocket upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket Multi Turn",
        "upstream-multi-turn",
        None,
        None,
        Some(&format!("http://{upstream_addr}")),
    )
    .await;

    let proxy_app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone());
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket proxy server");
    let proxy_addr = proxy_listener
        .local_addr()
        .expect("websocket proxy server addr");
    let proxy_handle = tokio::spawn(async move {
        axum::serve(proxy_listener, proxy_app)
            .await
            .expect("websocket proxy server should run");
    });

    let mut request = format!("ws://{proxy_addr}/v1/responses")
        .into_client_request()
        .expect("websocket client request");
    request.headers_mut().insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer pool-live-key"),
    );
    let (mut client, response) = connect_async(request)
        .await
        .expect("connect websocket proxy");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    for turn in 1..=2 {
        client
            .send(TungsteniteMessage::Text(
                json!({
                    "type": "response.create",
                    "model": "gpt-5-realtime",
                    "input": [{
                        "role": "user",
                        "content": [{ "type": "input_text", "text": format!("turn {turn}") }]
                    }]
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send websocket response.create");
        let message = client
            .next()
            .await
            .expect("receive websocket response")
            .expect("websocket response frame");
        let text = match message {
            TungsteniteMessage::Text(text) => text.to_string(),
            other => panic!("expected text websocket response, got {other:?}"),
        };
        assert!(text.contains(&format!("resp_ws_turn_{turn}")));
    }

    let rows = sqlx::query_as::<_, (Option<i64>, Option<i64>, String)>(
        r#"
        SELECT input_tokens, output_tokens, raw_response
        FROM codex_invocations
        WHERE source = ?1 AND payload LIKE '%"transport":"websocket"%'
        ORDER BY id
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_all(&state.pool)
    .await
    .expect("load websocket invocation rows");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, Some(11));
    assert_eq!(rows[0].1, Some(3));
    assert!(rows[0].2.contains("resp_ws_turn_1"));
    assert_eq!(rows[1].0, Some(12));
    assert_eq!(rows[1].1, Some(4));
    assert!(rows[1].2.contains("resp_ws_turn_2"));

    proxy_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_downstream_disconnect_drains_terminal_usage() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tungstenite::Message as TungsteniteMessage;

    async fn websocket_delayed_terminal_upstream(ws: WebSocketUpgrade) -> Response {
        ws.on_upgrade(move |mut socket| async move {
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    AxumWsMessage::Text(_) | AxumWsMessage::Binary(_) => {
                        tokio::time::sleep(Duration::from_millis(40)).await;
                        let response = json!({
                            "type": "response.completed",
                            "response": {
                                "id": "resp_ws_drain",
                                "status": "completed",
                                "model": "gpt-5-realtime",
                                "usage": {
                                    "input_tokens": 21,
                                    "output_tokens": 5,
                                    "total_tokens": 26
                                }
                            }
                        })
                        .to_string();
                        let _ = socket.send(AxumWsMessage::Text(response)).await;
                        let _ = socket.send(AxumWsMessage::Close(None)).await;
                        break;
                    }
                    AxumWsMessage::Close(_) => break,
                    _ => {}
                }
            }
        })
        .into_response()
    }

    let upstream_app =
        Router::new().route("/v1/responses", get(websocket_delayed_terminal_upstream));
    let upstream_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket upstream");
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("websocket upstream addr");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(upstream_listener, upstream_app)
            .await
            .expect("websocket upstream should run");
    });

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&format!("http://{upstream_addr}")).expect("valid websocket upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket Drain",
        "upstream-drain",
        None,
        None,
        Some(&format!("http://{upstream_addr}")),
    )
    .await;

    let proxy_app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone());
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket proxy server");
    let proxy_addr = proxy_listener
        .local_addr()
        .expect("websocket proxy server addr");
    let proxy_handle = tokio::spawn(async move {
        axum::serve(proxy_listener, proxy_app)
            .await
            .expect("websocket proxy server should run");
    });

    let mut request = format!("ws://{proxy_addr}/v1/responses")
        .into_client_request()
        .expect("websocket client request");
    request.headers_mut().insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer pool-live-key"),
    );
    let (mut client, response) = connect_async(request)
        .await
        .expect("connect websocket proxy");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    client
        .send(TungsteniteMessage::Text(
            json!({
                "type": "response.create",
                "model": "gpt-5-realtime",
                "input": [{
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "drain me" }]
                }]
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send websocket response.create");
    let _ = client.send(TungsteniteMessage::Close(None)).await;
    drop(client);

    let mut count = 0_i64;
    for _ in 0..20 {
        count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE raw_response LIKE '%resp_ws_drain%'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("count drained websocket invocation rows");
        if count == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(count, 1);

    let row = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT input_tokens, output_tokens FROM codex_invocations WHERE raw_response LIKE '%resp_ws_drain%'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load drained websocket invocation row");
    assert_eq!(row.0, Some(21));
    assert_eq!(row.1, Some(5));

    proxy_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_upstream_close_before_terminal_sends_retryable_close() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tungstenite::Message as TungsteniteMessage;

    async fn websocket_preterminal_close_upstream(ws: WebSocketUpgrade) -> Response {
        ws.on_upgrade(move |mut socket| async move {
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    AxumWsMessage::Text(_) | AxumWsMessage::Binary(_) => {
                        let _ = socket.send(AxumWsMessage::Close(None)).await;
                        break;
                    }
                    AxumWsMessage::Close(_) => break,
                    _ => {}
                }
            }
        })
        .into_response()
    }

    let upstream_app =
        Router::new().route("/v1/responses", get(websocket_preterminal_close_upstream));
    let upstream_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket upstream");
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("websocket upstream addr");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(upstream_listener, upstream_app)
            .await
            .expect("websocket upstream should run");
    });

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&format!("http://{upstream_addr}")).expect("valid websocket upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket Preterminal Close",
        "upstream-preterminal-close",
        None,
        None,
        Some(&format!("http://{upstream_addr}")),
    )
    .await;

    let proxy_app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone());
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket proxy server");
    let proxy_addr = proxy_listener
        .local_addr()
        .expect("websocket proxy server addr");
    let proxy_handle = tokio::spawn(async move {
        axum::serve(proxy_listener, proxy_app)
            .await
            .expect("websocket proxy server should run");
    });

    let mut request = format!("ws://{proxy_addr}/v1/responses")
        .into_client_request()
        .expect("websocket client request");
    request.headers_mut().insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer pool-live-key"),
    );
    let (mut client, response) = connect_async(request)
        .await
        .expect("connect websocket proxy");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    client
        .send(TungsteniteMessage::Text(
            json!({
                "type": "response.create",
                "model": "gpt-5-realtime",
                "input": [{
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "close early" }]
                }]
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send websocket response.create");

    let close = client
        .next()
        .await
        .expect("receive websocket close")
        .expect("websocket close frame");
    let TungsteniteMessage::Close(Some(frame)) = close else {
        panic!("expected retryable close frame, got {close:?}");
    };
    assert_eq!(u16::from(frame.code), 1013);
    assert!(frame.reason.contains("retry"));

    let mut attempt_row = ("pending".to_string(), None::<String>);
    for _ in 0..20 {
        attempt_row = sqlx::query_as::<_, (String, Option<String>)>(
            r#"
            SELECT status, error_message
            FROM pool_upstream_request_attempts
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_one(&state.pool)
        .await
        .expect("load websocket terminal attempt");
        if attempt_row.0 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        attempt_row.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert!(
        attempt_row
            .1
            .as_deref()
            .is_some_and(|message| message.contains("before response.completed"))
    );
    let mut no_ws_tag_count = 0;
    for _ in 0..20 {
        no_ws_tag_count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM pool_tags tag
            JOIN pool_upstream_account_tags link ON link.tag_id = tag.id
            WHERE link.account_id = ?1
              AND tag.system_key = 'unsupported_transport:websocket'
            "#,
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load websocket unsupported tag count");
        if no_ws_tag_count == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        no_ws_tag_count, 1,
        "preterminal upstream close should isolate account from future websocket routing"
    );

    proxy_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_handshake_failure_retries_next_candidate_with_retained_first_frame() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tungstenite::Message as TungsteniteMessage;

    async fn failing_ws_handshake(State(attempts): State<Arc<AtomicUsize>>) -> Response {
        attempts.fetch_add(1, Ordering::SeqCst);
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }

    async fn successful_ws_upstream(
        ws: WebSocketUpgrade,
        State(first_frames): State<Arc<StdMutex<Vec<String>>>>,
        headers: HeaderMap,
    ) -> Response {
        let authorization = headers
            .get(http_header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        ws.on_upgrade(move |mut socket| async move {
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    AxumWsMessage::Text(text) => {
                        first_frames
                            .lock()
                            .expect("lock first websocket frames")
                            .push(text.to_string());
                        let response = json!({
                            "type": "response.completed",
                            "response": {
                                "id": "resp_ws_failover",
                                "status": "completed",
                                "usage": {
                                    "input_tokens": 3,
                                    "output_tokens": 1,
                                    "total_tokens": 4
                                }
                            },
                            "authorization": authorization
                        })
                        .to_string();
                        let _ = socket.send(AxumWsMessage::Text(response)).await;
                        let _ = socket.send(AxumWsMessage::Close(None)).await;
                        break;
                    }
                    AxumWsMessage::Binary(_) => break,
                    AxumWsMessage::Close(_) => break,
                    _ => {}
                }
            }
        })
        .into_response()
    }

    let failed_attempts = Arc::new(AtomicUsize::new(0));
    let failing_app = Router::new()
        .route("/v1/responses", get(failing_ws_handshake))
        .with_state(failed_attempts.clone());
    let failing_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind failing websocket upstream");
    let failing_addr = failing_listener
        .local_addr()
        .expect("failing websocket upstream addr");
    let failing_handle = tokio::spawn(async move {
        axum::serve(failing_listener, failing_app)
            .await
            .expect("failing websocket upstream should run");
    });

    let first_frames = Arc::new(StdMutex::new(Vec::new()));
    let success_app = Router::new()
        .route("/v1/responses", get(successful_ws_upstream))
        .with_state(first_frames.clone());
    let success_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind successful websocket upstream");
    let success_addr = success_listener
        .local_addr()
        .expect("successful websocket upstream addr");
    let success_handle = tokio::spawn(async move {
        axum::serve(success_listener, success_app)
            .await
            .expect("successful websocket upstream should run");
    });

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&format!("http://{success_addr}")).expect("valid websocket upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let failing_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket Failing",
        "upstream-failing",
        None,
        None,
        Some(&format!("http://{failing_addr}")),
    )
    .await;
    let success_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket Success",
        "upstream-success",
        None,
        None,
        Some(&format!("http://{success_addr}")),
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    upsert_sticky_route(
        &state.pool,
        "sticky-websocket-failover",
        failing_account_id,
        &now_iso,
    )
    .await
    .expect("seed sticky route toward failing account");

    let proxy_app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone());
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket proxy server");
    let proxy_addr = proxy_listener
        .local_addr()
        .expect("websocket proxy server addr");
    let proxy_handle = tokio::spawn(async move {
        axum::serve(proxy_listener, proxy_app)
            .await
            .expect("websocket proxy server should run");
    });

    let mut request = format!("ws://{proxy_addr}/v1/responses")
        .into_client_request()
        .expect("websocket client request");
    request.headers_mut().insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer pool-live-key"),
    );
    request.headers_mut().insert(
        HeaderName::from_static("x-sticky-key"),
        HeaderValue::from_static("sticky-websocket-failover"),
    );
    let (mut client, response) = connect_async(request)
        .await
        .expect("connect websocket proxy");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);

    client
        .send(TungsteniteMessage::Text(
            json!({
                "type": "response.create",
                "model": "gpt-5-realtime",
                "input": [{
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "failover" }]
                }]
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send retained first websocket frame");

    let message = client
        .next()
        .await
        .expect("receive websocket response")
        .expect("websocket response frame");
    let text = match message {
        TungsteniteMessage::Text(text) => text.to_string(),
        other => panic!("expected text websocket response, got {other:?}"),
    };
    let payload: Value = serde_json::from_str(&text).expect("decode websocket response");
    assert_eq!(payload["authorization"], "Bearer upstream-success");
    let _ = client.next().await;
    assert_eq!(failed_attempts.load(Ordering::SeqCst), 1);
    assert_eq!(
        first_frames
            .lock()
            .expect("lock retained websocket frames")
            .len(),
        1
    );

    let mut attempt_rows = Vec::new();
    for _ in 0..20 {
        attempt_rows = sqlx::query_as::<_, (Option<i64>, String)>(
            r#"
            SELECT upstream_account_id, status
            FROM pool_upstream_request_attempts
            ORDER BY id
            "#,
        )
        .fetch_all(&state.pool)
        .await
        .expect("load websocket failover attempts");
        if attempt_rows.len() == 2
            && attempt_rows
                .iter()
                .all(|(_, status)| status != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].0, Some(failing_account_id));
    assert_eq!(
        attempt_rows[0].1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(attempt_rows[1].0, Some(success_account_id));
    assert_eq!(
        attempt_rows[1].1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );

    proxy_handle.abort();
    failing_handle.abort();
    success_handle.abort();
}

#[tokio::test]
async fn websocket_deferred_prepare_requires_upstream_subprotocol_match_before_relay() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;
    use tungstenite::Message as TungsteniteMessage;

    async fn websocket_protocol_capture_upstream(
        ws: WebSocketUpgrade,
        State(protocols): State<Arc<StdMutex<Vec<Option<String>>>>>,
        headers: HeaderMap,
    ) -> Response {
        protocols
            .lock()
            .expect("lock websocket protocol captures")
            .push(
                headers
                    .get(HeaderName::from_static("sec-websocket-protocol"))
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
            );
        ws.on_upgrade(move |mut socket| async move {
            while let Some(Ok(message)) = socket.next().await {
                match message {
                    AxumWsMessage::Text(_) => {
                        let response = json!({
                            "type": "response.completed",
                            "response": {
                                "id": "resp_ws_protocol",
                                "status": "completed",
                                "usage": {
                                    "input_tokens": 4,
                                    "output_tokens": 2,
                                    "total_tokens": 6
                                }
                            }
                        })
                        .to_string();
                        let _ = socket.send(AxumWsMessage::Text(response)).await;
                        let _ = socket.send(AxumWsMessage::Close(None)).await;
                        break;
                    }
                    AxumWsMessage::Binary(_) => break,
                    AxumWsMessage::Close(_) => break,
                    _ => {}
                }
            }
        })
        .into_response()
    }

    let captured_protocols = Arc::new(StdMutex::new(Vec::new()));
    let upstream_app = Router::new()
        .route("/v1/responses", get(websocket_protocol_capture_upstream))
        .with_state(captured_protocols.clone());
    let upstream_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket upstream");
    let upstream_addr = upstream_listener
        .local_addr()
        .expect("websocket upstream addr");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(upstream_listener, upstream_app)
            .await
            .expect("websocket upstream should run");
    });

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&format!("http://{upstream_addr}")).expect("valid websocket upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket Protocol",
        "upstream-protocol",
        None,
        None,
        Some(&format!("http://{upstream_addr}")),
    )
    .await;

    let proxy_app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1_with_connect_info))
        .with_state(state.clone());
    let proxy_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind websocket proxy server");
    let proxy_addr = proxy_listener
        .local_addr()
        .expect("websocket proxy server addr");
    let proxy_handle = tokio::spawn(async move {
        axum::serve(proxy_listener, proxy_app)
            .await
            .expect("websocket proxy server should run");
    });

    let mut request = format!("ws://{proxy_addr}/v1/responses")
        .into_client_request()
        .expect("websocket client request");
    request.headers_mut().insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer pool-live-key"),
    );
    request.headers_mut().insert(
        HeaderName::from_static("sec-websocket-protocol"),
        HeaderValue::from_static("responses.realtime.v1"),
    );
    let (mut client, response) = connect_async(request)
        .await
        .expect("connect websocket proxy");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    assert_eq!(
        response
            .headers()
            .get(HeaderName::from_static("sec-websocket-protocol"))
            .and_then(|value| value.to_str().ok()),
        Some("responses.realtime.v1")
    );

    client
        .send(TungsteniteMessage::Text(
            json!({
                "type": "response.create",
                "model": "gpt-5-realtime",
                "input": [{
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "protocol" }]
                }]
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send websocket response.create");
    let close = client
        .next()
        .await
        .expect("receive websocket close")
        .expect("websocket close frame");
    let TungsteniteMessage::Close(Some(frame)) = close else {
        panic!("expected retryable close frame, got {close:?}");
    };
    assert_eq!(u16::from(frame.code), 1013);
    assert!(frame.reason.contains("retry"));

    assert_eq!(
        captured_protocols
            .lock()
            .expect("lock websocket protocol captures")
            .as_slice(),
        &[Some("responses.realtime.v1".to_string())]
    );

    let attempt_row = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, error_message
        FROM pool_upstream_request_attempts
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load websocket subprotocol mismatch attempt");
    assert_eq!(
        attempt_row.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    let error_message = attempt_row.1.unwrap_or_default().to_ascii_lowercase();
    assert!(
        error_message.contains("subprotocol"),
        "unexpected websocket subprotocol mismatch attempt error: {error_message}"
    );

    proxy_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_prepare_does_not_treat_sticky_key_as_prompt_cache_key() {
    let (upstream_base, _attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.websocket_enabled = true;
        settings.upstream_websocket_default_enabled = true;
    }
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let owner_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Encrypted Owner",
        "upstream-owner",
        None,
        None,
        None,
    )
    .await;
    let sticky_only_failover_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "upstream-secondary",
        None,
        None,
        None,
    )
    .await;
    let capacity_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "WebSocket Capacity Holder",
        "upstream-capacity-holder",
        None,
        None,
        Some("https://websocket-capacity-holder.example.com/backend-api/codex"),
    )
    .await;
    set_test_account_status(&state.pool, owner_account_id, "needs_reauth").await;
    let sticky_only_key = "sticky-only-websocket-key";
    let model = "gpt-5-realtime";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, sticky_only_key, owner_account_id)
        .await
        .expect("persist sticky-only named encrypted owner row");
    upsert_sticky_route(
        &state.pool,
        sticky_only_key,
        sticky_only_failover_account_id,
        &format_utc_iso(Utc::now()),
    )
    .await
    .expect("seed sticky route toward the handshake-failing account");
    observe_model_route_seen(&state.pool, capacity_account_id, Some(model))
        .await
        .expect("seed capacity-holder model route");
    sqlx::query(
        "UPDATE pool_routing_settings SET cache_hit_protection_enabled = 1, cache_hit_overflow_mode = 'queue' WHERE id = 1",
    )
    .execute(&state.pool)
    .await
    .expect("enable websocket queue overflow mode");
    sqlx::query(
        "UPDATE pool_upstream_account_model_routes SET cache_concurrency_limit = 1 WHERE account_id = ?1 AND model = ?2",
    )
    .bind(capacity_account_id)
    .bind(model)
    .execute(&state.pool)
    .await
    .expect("limit capacity-holder model route");
    let holder = resolve_pool_account_for_request_with_route_requirement_and_image_intent_and_override_and_codex_imagegen_request_and_reservation(
        &state,
        None,
        Some(model),
        &[sticky_only_failover_account_id],
        &HashSet::new(),
        None,
        None,
        None,
        "/v1/realtime",
        crate::ImageIntent::Unknown,
        false,
        Some("websocket-retry-exhausted-capacity-holder"),
    )
    .await
    .expect("reserve the fallback websocket candidate");
    assert!(matches!(holder, PoolAccountResolution::Resolved(_)));

    let headers = HeaderMap::from_iter([
        (
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        ),
        (
            HeaderName::from_static("x-sticky-key"),
            HeaderValue::from_static(sticky_only_key),
        ),
    ]);
    let (sticky_key, prompt_cache_key) = websocket_routing_keys_from_headers(&headers);
    assert_eq!(sticky_key.as_deref(), Some(sticky_only_key));
    assert_eq!(prompt_cache_key, None);
    let trace_sticky_key = sticky_key.clone();

    let (binding_constraint, owner_auto_guard_active) = load_via_pool_effective_routing_constraint(
        state.as_ref(),
        websocket_effective_prompt_cache_key(prompt_cache_key.as_deref()),
        false,
    )
    .await
    .expect("load websocket effective routing constraint without prompt cache key");
    assert!(binding_constraint.is_none());
    assert!(!owner_auto_guard_active);

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pool-ws-5353".to_string(),
        occurred_at: shanghai_now_string(),
        endpoint: "/v1/realtime".to_string(),
        sticky_key: trace_sticky_key,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: Some(model.to_string()),
    };
    let err = prepare_upstream_websocket(
        state.clone(),
        5353,
        &format!("/v1/realtime?model={model}")
            .parse()
            .expect("valid uri"),
        &headers,
        &resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts"),
        sticky_key.as_deref(),
        Some(model),
        websocket_effective_prompt_cache_key(prompt_cache_key.as_deref()),
        binding_constraint,
        None,
        owner_auto_guard_active,
        &trace,
        None,
    )
    .await;
    let Err(err) = err else {
        panic!("websocket upstream should fail at the fake HTTP upstream, not owner guard");
    };

    assert_eq!(err.status, StatusCode::BAD_GATEWAY);
    assert!(
        err.message.contains("failed to contact websocket upstream"),
        "expected upstream handshake failure, got: {}",
        err.message
    );

    let last_attempt = sqlx::query_as::<_, (Option<i64>, Option<String>)>(
        r#"
        SELECT upstream_account_id, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load websocket sticky-only terminal attempt");
    assert_eq!(last_attempt.0, Some(sticky_only_failover_account_id));
    assert_ne!(
        last_attempt.1.as_deref(),
        Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE)
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    let (status, failure_kind, payload): (String, Option<String>, String) = sqlx::query_as(
        "SELECT status, failure_kind, payload FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind(&trace.invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load websocket retry-exhausted invocation");
    assert_eq!(status, "http_502");
    assert_eq!(
        failure_kind.as_deref(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    let payload: Value =
        serde_json::from_str(&payload).expect("decode websocket invocation payload");
    assert_eq!(payload["poolAttemptCount"], 1);
    assert_eq!(payload["poolDistinctAccountCount"], 1);
    assert_eq!(
        payload["poolAttemptTerminalReason"],
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
    );
    assert!(payload["poolRoutingNoCandidateAudit"].is_object());

    let invocation_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
            .bind(&trace.invoke_id)
            .fetch_one(&state.pool)
            .await
            .expect("load websocket retry-exhausted invocation id");
    let Json(workflow) =
        fetch_invocation_workflow_detail(State(state.clone()), axum::extract::Path(invocation_id))
            .await
            .expect("load websocket retry-exhausted workflow detail");
    let workflow = serde_json::to_value(workflow).expect("serialize workflow detail");
    let final_failure = workflow["timeline"]
        .as_array()
        .and_then(|entries| {
            entries
                .iter()
                .find(|entry| entry["kind"] == "systemFinalFailure")
        })
        .expect("workflow should contain the retry-exhausted final failure");
    assert_eq!(final_failure["responseBody"]["available"], true);
    let response_body = final_failure["responseBody"]["bodyText"]
        .as_str()
        .expect("workflow retry-exhausted failure should expose its body");
    let response_body: Value =
        serde_json::from_str(response_body).expect("decode retry-exhausted failure body");
    assert_eq!(response_body["error"], err.message);
    assert_eq!(response_body["code"], PROXY_FAILURE_FAILED_CONTACT_UPSTREAM);

    release_pool_routing_reservation(&state, "websocket-retry-exhausted-capacity-holder");
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_prefers_body_timeout_before_pool_wait() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_request_read_timeout = Duration::from_millis(80);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(200),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = tx.send(Ok(Bytes::from_static(b"\"messages\":[]}"))).await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6242,
        &"/v1/chat/completions".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("known-stream-sticky"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        runtime_timeouts,
        None,
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(180),
        "request body timeout should win before pool wait timeout, elapsed={elapsed:?}"
    );
    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(
        message, "request body read timed out after 80ms",
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_preserves_body_timeout_over_rate_limited_header() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_request_read_timeout = Duration::from_millis(80);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(200),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let rate_limited_id =
        insert_test_pool_api_key_account(&state, "Rate Limited", "upstream-rate-limited").await;
    set_test_account_rate_limited_cooldown(&state.pool, rate_limited_id, 120).await;
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "header-rate-limited-sticky",
        rate_limited_id,
        &sticky_seen_at,
    )
    .await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = tx.send(Ok(Bytes::from_static(b"\"messages\":[]}"))).await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6243,
        &"/v1/chat/completions".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("header-rate-limited-sticky"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        runtime_timeouts,
        None,
    )
    .await;
    assert!(
        wait_started_rx.try_recv().is_err(),
        "header sticky request should time out on body read before entering bounded pool wait"
    );
    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(
        message, "request body read timed out after 80ms",
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_waits_for_blocked_policy_header_error() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(200),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let sticky_source_id =
        insert_test_pool_api_key_account(&state, "Ungrouped Sticky", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Grouped", "upstream-secondary").await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
        .bind(sticky_source_id)
        .execute(&state.pool)
        .await
        .expect("clear sticky source group");
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "header-blocked-policy-sticky",
        sticky_source_id,
        &sticky_seen_at,
    )
    .await;

    let now_iso = format_utc_iso(Utc::now());
    let disallow_cut_out_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("via-pool-no-cut-out")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert no-cut-out tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(sticky_source_id)
    .bind(disallow_cut_out_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach no-cut-out tag");

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = tx.send(Ok(Bytes::from_static(b"\"messages\":[]}"))).await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6244,
        &"/v1/chat/completions".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("header-blocked-policy-sticky"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        runtime_timeouts,
        None,
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(140),
        "blocked policy should wait for the streamed body before failing, elapsed={elapsed:?}"
    );
    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        message.contains("upstream account is not assigned to a group"),
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_same_value_short_circuits_blocked_policy_error() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(200),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let sticky_source_id =
        insert_test_pool_api_key_account(&state, "Ungrouped Sticky", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Grouped", "upstream-secondary").await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
        .bind(sticky_source_id)
        .execute(&state.pool)
        .await
        .expect("clear sticky source group");
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "header-blocked-policy-sticky",
        sticky_source_id,
        &sticky_seen_at,
    )
    .await;

    let now_iso = format_utc_iso(Utc::now());
    let disallow_cut_out_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("via-pool-no-cut-out")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert no-cut-out tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(sticky_source_id)
    .bind(disallow_cut_out_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach no-cut-out tag");

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(40)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(
                b"\"stickyKey\":\"header-blocked-policy-sticky\",",
            )))
            .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = tx.send(Ok(Bytes::from_static(b"\"messages\":[]}"))).await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6247,
        &"/v1/chat/completions".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("header-blocked-policy-sticky"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        runtime_timeouts,
        None,
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(400),
        "same sticky value should fail before the rest of the streamed body finishes, elapsed={elapsed:?}"
    );
    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        message.contains("upstream account is not assigned to a group"),
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[test]
fn proxy_openai_v1_header_sticky_stream_waits_for_body_sticky_override_before_failing() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let blocked_id =
            insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
        let replacement_id =
            insert_test_pool_api_key_account(&state, "Replacement", "upstream-replacement").await;
        set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;
        let sticky_seen_at = format_utc_iso(Utc::now());
        upsert_test_sticky_route_at(
            &state.pool,
            "header-stale-sticky",
            blocked_id,
            &sticky_seen_at,
        )
        .await;
        upsert_test_sticky_route_at(
            &state.pool,
            "body-live-sticky",
            replacement_id,
            &sticky_seen_at,
        )
        .await;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
                .await;
            tokio::time::sleep(Duration::from_millis(170)).await;
            let _ = tx
                .send(Ok(Bytes::from_static(
                    b"\"messages\":[],\"stickyKey\":\"body-live-sticky\"}",
                )))
                .await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let started = Instant::now();
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6242,
            &"/v1/chat/completions".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("header-stale-sticky"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool request should succeed");
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(140),
            "request should wait for the body sticky override before resolving, elapsed={elapsed:?}"
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
        assert_eq!(payload["authorization"], "Bearer upstream-replacement");
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
        assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-blocked").copied(), None);
        assert_eq!(
            attempts.get("Bearer upstream-replacement").copied(),
            Some(1)
        );

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_responses_wait_timeout_respects_total_timeout_from_request_start()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(90);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(400),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(180)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(b"\"input\":\"hello\"}")))
            .await;
    });
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6243,
        &"/v1/responses".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("header-responses-sticky"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        runtime_timeouts,
        None,
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(60),
        "request should still wait briefly for bounded pool recovery, elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(180),
        "responses total timeout should short-circuit even while the body is still buffering, elapsed={elapsed:?}"
    );
    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        message,
        pool_total_timeout_exhausted_message(Duration::from_millis(90)),
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_recovers_after_wait_starts() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(650);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(850),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1(
            State(request_state),
            OriginalUri("/v1/responses".parse().expect("valid uri")),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("sticky-final-window-recovery"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5","input":"hello"}"#.as_bytes().to_vec()),
        )
        .await
    });

    tokio::task::spawn_blocking(move || {
        wait_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("header sticky request should signal once the bounded wait starts");
    })
    .await
    .expect("wait hook worker should join");
    set_test_account_status(&state.pool, delayed_id, "active").await;
    state.pool_routing_availability.publish();

    let response = request_task
        .await
        .expect("header sticky request task should join");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-delayed");
    wait_for_pool_attempt_row_count(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-delayed").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_responses_total_timeout_short_circuits_body_buffering() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(90);
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(400),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(220)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(b"\"input\":\"hello\"}")))
            .await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6246,
        &"/v1/responses".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("header-responses-sticky"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        runtime_timeouts,
        None,
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(60),
        "request should still wait briefly for bounded pool recovery, elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(180),
        "responses total timeout should short-circuit before body buffering completes, elapsed={elapsed:?}"
    );
    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        message,
        pool_total_timeout_exhausted_message(Duration::from_millis(90)),
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn proxy_openai_v1_responses_prebuffer_body_counts_total_timeout_from_request_start() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(90);
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let request_body = br#"{"model":"gpt-5","input":"hello"}"#.to_vec();
    let content_length =
        HeaderValue::from_str(&request_body.len().to_string()).expect("content length header");
    let slow_body = stream::unfold(Some(request_body), |state| async move {
        match state {
            Some(body) => {
                tokio::time::sleep(Duration::from_millis(180)).await;
                Some((Ok::<Bytes, Infallible>(Bytes::from(body)), None))
            }
            None => None,
        }
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6244,
        &"/v1/responses".parse().expect("valid uri"),
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
            (http_header::CONTENT_LENGTH, content_length),
        ]),
        Body::from_stream(slow_body),
        runtime_timeouts,
        None,
    )
    .await;
    let elapsed = started.elapsed();

    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert!(
        elapsed >= Duration::from_millis(160),
        "request should wait for the body upload before timing out, elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(280),
        "responses total timeout should include prebuffer body upload time, elapsed={elapsed:?}"
    );
    assert_eq!(
        message,
        pool_total_timeout_exhausted_message(Duration::from_millis(90)),
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[test]
fn proxy_openai_v1_responses_prebuffer_body_wait_counts_total_timeout_from_request_start() {
    run_future_with_large_stack(async move {
        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse("https://api.openai.com/").expect("valid upstream base url");
        config.pool_upstream_responses_total_timeout = Duration::from_millis(90);
        config.openai_proxy_request_read_timeout = Duration::from_millis(500);
        let state = test_state_from_config_with_pool_no_available_wait(
            config,
            true,
            PoolNoAvailableWaitSettings {
                timeout: Duration::from_millis(220),
                retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
            },
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let blocked_id =
            insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
        set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

        let request_body = br#"{"model":"gpt-5","input":"hello"}"#.to_vec();
        let content_length =
            HeaderValue::from_str(&request_body.len().to_string()).expect("content length header");
        let slow_body = stream::unfold(Some(request_body), |state| async move {
            match state {
                Some(body) => {
                    tokio::time::sleep(Duration::from_millis(70)).await;
                    Some((Ok::<Bytes, Infallible>(Bytes::from(body)), None))
                }
                None => None,
            }
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let started = Instant::now();
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6247,
            &"/v1/responses".parse().expect("valid uri"),
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
                (http_header::CONTENT_LENGTH, content_length),
            ]),
            Body::from_stream(slow_body),
            runtime_timeouts,
            None,
        )
        .await;
        let elapsed = started.elapsed();

        let err = response.expect_err("via-pool request should fail");
        let status = err.status;
        let message = err.message;
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert!(
            elapsed >= Duration::from_millis(70),
            "request should spend time uploading the buffered body, elapsed={elapsed:?}"
        );
        assert_eq!(
            message,
            pool_total_timeout_exhausted_message(Duration::from_millis(90)),
            "unexpected via-pool failure: {message}"
        );
        assert!(
            count_pool_upstream_request_attempts(&state.pool).await <= 1,
            "timeout may expire before the first upstream attempt is persisted on loaded runners"
        );
    });
}

#[test]
fn proxy_openai_v1_responses_streamed_body_counts_total_timeout_from_request_start_without_wait() {
    run_future_with_large_stack(async move {
        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse("https://api.openai.com/").expect("valid upstream base url");
        config.pool_upstream_responses_total_timeout = Duration::from_millis(90);
        config.openai_proxy_request_read_timeout = Duration::from_millis(500);
        let state = test_state_from_config(config, true).await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
                .await;
            // Keep the body completion far beyond the request timeout so the
            // assertion remains meaningful under a heavily loaded test runner.
            tokio::time::sleep(Duration::from_secs(2)).await;
            let _ = tx
                .send(Ok(Bytes::from_static(b"\"input\":\"hello\"}")))
                .await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let started = Instant::now();
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6245,
            &"/v1/responses".parse().expect("valid uri"),
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
            runtime_timeouts,
            None,
        )
        .await;
        let elapsed = started.elapsed();

        let err = response.expect_err("via-pool request should fail");
        let status = err.status;
        let message = err.message;
        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert!(
            elapsed >= Duration::from_millis(70),
            "responses total timeout should still start at request admission, elapsed={elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "live first attempt should not wait for the entire streamed body before timing out, elapsed={elapsed:?}"
        );
        assert_eq!(
            message,
            pool_total_timeout_exhausted_message(Duration::from_millis(90)),
            "unexpected via-pool failure: {message}"
        );
        assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
    });
}

#[tokio::test]
async fn pool_route_waited_initial_account_still_uses_remaining_total_timeout_budget() {
    let (upstream_base, upstream_handle) =
        spawn_pool_delayed_headers_upstream(Duration::from_millis(250)).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(300);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(600),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let delayed_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Delayed",
        "upstream-delayed",
        None,
        None,
        Some(upstream_base.as_str()),
    )
    .await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let wait_started_rx = crate::proxy::register_pool_no_available_wait_hook(&state);
    let pool = state.pool.clone();
    let runtime_handle = tokio::runtime::Handle::current();
    let delayed_release_task = std::thread::spawn(move || {
        wait_started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("request should signal once the bounded wait starts");
        std::thread::sleep(Duration::from_millis(120));
        runtime_handle.block_on(async move {
            set_test_account_status(&pool, delayed_id, "active").await;
        });
    });

    let started = Instant::now();
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-wait-remaining-total-timeout"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    let elapsed = started.elapsed();

    delayed_release_task
        .join()
        .expect("delayed release thread should join");

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(
        elapsed < Duration::from_secs(2),
        "late account recovery should still terminate within a loaded-runner bounded timeout budget, elapsed={elapsed:?}"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(pool_total_timeout_exhausted_message(Duration::from_millis(300)).as_str())
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_preserves_pre_resolved_account_after_body() {
    let attempts = Arc::new(StdMutex::new(HashMap::<String, usize>::new()));
    let request_started = Arc::new(tokio::sync::Notify::new());
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let release_rx = Arc::new(StdMutex::new(Some(release_rx)));
    let app = Router::new().route(
        "/v1/chat/completions",
        post({
            let attempts = attempts.clone();
            let request_started = request_started.clone();
            let release_rx = release_rx.clone();
            move |headers: HeaderMap| {
                let attempts = attempts.clone();
                let request_started = request_started.clone();
                let release_rx = release_rx.clone();
                async move {
                    let authorization = headers
                        .get(http_header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    {
                        let mut attempt_counts = attempts.lock().expect("lock attempts");
                        *attempt_counts.entry(authorization.clone()).or_insert(0) += 1;
                    }
                    request_started.notify_one();
                    let release_rx = release_rx
                        .lock()
                        .expect("lock release receiver")
                        .take()
                        .expect("single release receiver");
                    let _ = release_rx.await;
                    (
                        StatusCode::OK,
                        Json(json!({
                            "authorization": authorization,
                        })),
                    )
                        .into_response()
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind delayed sticky upstream");
    let addr = listener.local_addr().expect("delayed sticky upstream addr");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("delayed sticky upstream should run");
    });
    let upstream_base = format!("http://{addr}");
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(220),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Replacement", "upstream-replacement").await;
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "header-stale-sticky",
        primary_id,
        &sticky_seen_at,
    )
    .await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(170)).await;
        let _ = tx.send(Ok(Bytes::from_static(b"\"messages\":[]}"))).await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let request_state = state.clone();
    let request_task = tokio::spawn(async move {
        proxy_openai_v1_via_pool(
            request_state,
            6342,
            &"/v1/chat/completions".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("header-stale-sticky"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
            runtime_timeouts,
            None,
        )
        .await
    });

    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    request_started.notified().await;
    set_test_account_status(&state.pool, primary_id, "needs_reauth").await;
    release_tx
        .send(())
        .expect("release delayed sticky upstream response");

    let response = request_task
        .await
        .expect("via-pool request task should join")
        .expect("via-pool request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read via-pool response");
    let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-replacement").copied(), None);

    upstream_handle.abort();
}

#[test]
fn proxy_openai_v1_header_sticky_stream_body_override_beats_rate_limited_header() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let rate_limited_id =
            insert_test_pool_api_key_account(&state, "Rate Limited", "upstream-rate-limited").await;
        let replacement_id =
            insert_test_pool_api_key_account(&state, "Replacement", "upstream-replacement").await;
        set_test_account_rate_limited_cooldown(&state.pool, rate_limited_id, 120).await;
        let sticky_seen_at = format_utc_iso(Utc::now());
        upsert_test_sticky_route_at(
            &state.pool,
            "header-rate-limited-sticky",
            rate_limited_id,
            &sticky_seen_at,
        )
        .await;
        upsert_test_sticky_route_at(
            &state.pool,
            "body-live-sticky",
            replacement_id,
            &sticky_seen_at,
        )
        .await;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
                .await;
            tokio::time::sleep(Duration::from_millis(170)).await;
            let _ = tx
                .send(Ok(Bytes::from_static(
                    b"\"messages\":[],\"stickyKey\":\"body-live-sticky\"}",
                )))
                .await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let started = Instant::now();
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6245,
            &"/v1/chat/completions".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("header-rate-limited-sticky"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool request should succeed");
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(140),
            "request should wait for the body sticky override before resolving, elapsed={elapsed:?}"
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
        assert_eq!(payload["authorization"], "Bearer upstream-replacement");
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
        assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-rate-limited").copied(), None);
        assert_eq!(
            attempts.get("Bearer upstream-replacement").copied(),
            Some(1)
        );

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_header_prompt_cache_binding_beats_rate_limited_sticky_terminal() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let sticky_group = "header-binding-sticky-group";
        let bound_group = "header-binding-bound-group";
        ensure_test_group_binding(&state.pool, sticky_group, None).await;
        ensure_test_group_binding(&state.pool, bound_group, None).await;
        let sticky_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Header Binding Sticky Rate Limited",
            "upstream-rate-limited",
            Some(sticky_group),
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Header Binding Replacement",
            "upstream-replacement",
            Some(bound_group),
            None,
            None,
        )
        .await;
        set_test_account_rate_limited_cooldown(&state.pool, sticky_account_id, 120).await;
        let sticky_seen_at = format_utc_iso(Utc::now());
        upsert_test_sticky_route_at(
            &state.pool,
            "header-binding-rate-limited-sticky",
            sticky_account_id,
            &sticky_seen_at,
        )
        .await;
        let prompt_cache_key = "pck-header-binding-beats-sticky-terminal";
        let now_iso = format_utc_iso(Utc::now());
        sqlx::query(
            r#"
            INSERT INTO prompt_cache_conversation_bindings (
                prompt_cache_key,
                binding_kind,
                group_name,
                upstream_account_id,
                created_at,
                updated_at
            )
            VALUES (?1, 'group', ?2, NULL, ?3, ?3)
            "#,
        )
        .bind(prompt_cache_key)
        .bind(bound_group)
        .bind(&now_iso)
        .execute(&state.pool)
        .await
        .expect("insert prompt cache group binding");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6246,
            &"/v1/chat/completions".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("header-binding-rate-limited-sticky"),
                ),
                (
                    HeaderName::from_static("x-prompt-cache-key"),
                    HeaderValue::from_static(prompt_cache_key),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5","messages":[]}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("prompt cache binding should route around sticky terminal");

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
        assert_eq!(payload["authorization"], "Bearer upstream-replacement");
        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-rate-limited").copied(), None);
        assert_eq!(
            attempts.get("Bearer upstream-replacement").copied(),
            Some(1)
        );

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_header_sticky_rechecks_model_before_reusing_header_resolution() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let sticky_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Header Sticky Only GPT-4.1",
            "upstream-sticky-only",
            None,
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Fallback GPT-5.5",
            "upstream-fallback",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            "UPDATE pool_upstream_accounts SET policy_available_models_json = ?2 WHERE id = ?1",
        )
        .bind(sticky_account_id)
        .bind(r#"["gpt-4.1"]"#)
        .execute(&state.pool)
        .await
        .expect("restrict sticky account model policy");

        let sticky_seen_at = format_utc_iso(Utc::now());
        upsert_test_sticky_route_at(
            &state.pool,
            "header-model-sensitive-sticky",
            sticky_account_id,
            &sticky_seen_at,
        )
        .await;

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6247,
            &"/v1/chat/completions".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("header-model-sensitive-sticky"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5.5","messages":[]}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool request should succeed");

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
        assert_eq!(payload["authorization"], "Bearer upstream-fallback");
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-sticky-only").copied(), None);
        assert_eq!(attempts.get("Bearer upstream-fallback").copied(), Some(1));

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_header_sticky_rechecks_image_intent_before_reusing_header_resolution() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let sticky_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Header Sticky Text Only",
            "upstream-sticky-text-only",
            None,
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Fallback Image Capable",
            "upstream-image-fallback",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET policy_response_image_tool_capability_override = 'unsupported'
            WHERE id = ?1
            "#,
        )
        .bind(sticky_account_id)
        .execute(&state.pool)
        .await
        .expect("mark sticky account as response image-tool incompatible");

        let sticky_seen_at = format_utc_iso(Utc::now());
        upsert_test_sticky_route_at(
            &state.pool,
            "header-image-sensitive-sticky",
            sticky_account_id,
            &sticky_seen_at,
        )
        .await;

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6248,
            &"/v1/responses".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("header-image-sensitive-sticky"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-image-1","input":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool image request should succeed");

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool image response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool image response");
        assert_eq!(payload["authorization"], "Bearer upstream-image-fallback");
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(
            attempts.get("Bearer upstream-sticky-text-only").copied(),
            None
        );
        assert_eq!(
            attempts.get("Bearer upstream-image-fallback").copied(),
            Some(1)
        );

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_header_sticky_rechecks_codex_imagegen_capability_before_reuse() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let sticky_account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Header Sticky Codex Imagegen Incompatible",
            "upstream-sticky-codex-imagegen-incompatible",
            None,
            None,
            None,
        )
        .await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Fallback Codex Imagegen Compatible",
            "upstream-codex-imagegen-fallback",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET policy_codex_imagegen_rewrite_mode = 'force_add',
                codex_imagegen_capability = 'unsupported'
            WHERE id = ?1
            "#,
        )
        .bind(sticky_account_id)
        .execute(&state.pool)
        .await
        .expect("mark sticky account as Codex imagegen incompatible");

        let sticky_seen_at = format_utc_iso(Utc::now());
        upsert_test_sticky_route_at(
            &state.pool,
            "header-codex-imagegen-sensitive-sticky",
            sticky_account_id,
            &sticky_seen_at,
        )
        .await;

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6249,
            &"/v1/responses".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("header-codex-imagegen-sensitive-sticky"),
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
            Body::from(r#"{"model":"gpt-5.1-codex","input":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool Codex request should succeed");

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
        assert_eq!(
            payload["authorization"],
            "Bearer upstream-codex-imagegen-fallback"
        );
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(
            attempts
                .get("Bearer upstream-sticky-codex-imagegen-incompatible")
                .copied(),
            None
        );
        assert_eq!(
            attempts
                .get("Bearer upstream-codex-imagegen-fallback")
                .copied(),
            Some(1)
        );

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_direct_image_prebuffer_preserves_image_capture_target_without_rewrite() {
    run_future_with_large_stack(async move {
        async fn direct_image_echo_upstream(headers: HeaderMap, body: Bytes) -> Response {
            let authorization = headers
                .get(http_header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let request_body: Value = serde_json::from_slice(&body).expect("decode upstream body");
            (
                StatusCode::OK,
                Json(json!({
                    "authorization": authorization,
                    "requestBody": request_body,
                })),
            )
                .into_response()
        }

        let app = Router::new().route("/v1/images/generations", post(direct_image_echo_upstream));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind direct image upstream");
        let upstream_base = format!("http://{}", listener.local_addr().expect("local addr"));
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("direct image upstream server should run");
        });
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Direct Image Force Add",
            "upstream-direct-image",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            "UPDATE pool_upstream_accounts SET policy_image_tool_rewrite_mode = ?2 WHERE id = ?1",
        )
        .bind(account_id)
        .bind("force_add")
        .execute(&state.pool)
        .await
        .expect("mark direct image account force_add");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6343,
            &"/v1/images/generations".parse().expect("valid uri"),
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
            Body::from(r#"{"model":"gpt-image-1","prompt":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool direct image request should succeed");

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool direct image response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool image response");
        assert_eq!(payload["authorization"], "Bearer upstream-direct-image");
        assert_eq!(payload["requestBody"]["model"], "gpt-image-1");
        assert_eq!(payload["requestBody"]["prompt"], "draw a cat");
        assert!(payload["requestBody"].get("tools").is_none());
        assert!(payload["requestBody"].get("tool_choice").is_none());

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_responses_force_add_failure_learns_response_image_tool_unsupported() {
    run_future_with_large_stack(async {
        async fn image_unsupported_upstream(body: Bytes) -> impl IntoResponse {
            let request_body: Value = serde_json::from_slice(&body).expect("decode upstream body");
            assert!(
                request_body["tools"]
                    .as_array()
                    .expect("tools should be injected")
                    .iter()
                    .any(|tool| tool["type"].as_str() == Some("image_generation")),
                "force_add should send an image tool upstream: {request_body:?}"
            );
            (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "code": "unsupported_tool",
                        "message": "image_generation is not supported for this account",
                        "type": "invalid_request_error"
                    }
                })),
            )
                .into_response()
        }

        let app = Router::new().route("/v1/responses", post(image_unsupported_upstream));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind image unsupported upstream");
        let upstream_base = format!("http://{}", listener.local_addr().expect("local addr"));
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("image unsupported upstream server should run");
        });
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Responses Force Add Unsupported",
            "upstream-force-add-unsupported",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            "UPDATE pool_upstream_accounts SET policy_image_tool_rewrite_mode = ?2 WHERE id = ?1",
        )
        .bind(account_id)
        .bind("force_add")
        .execute(&state.pool)
        .await
        .expect("mark responses account force_add");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6344,
            &"/v1/responses".parse().expect("valid uri"),
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
            Body::from(r#"{"model":"gpt-5.1-codex","input":"hello"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool responses request should return a route failure");
        assert!(response.status().is_server_error() || response.status().is_client_error());

        let capability: String = sqlx::query_scalar(
            "SELECT response_image_tool_capability FROM pool_upstream_accounts WHERE id = ?",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load response image-tool capability after force_add failure");
        assert_eq!(capability, "unsupported");

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_codex_imagegen_502_learns_namespace_incompatibility_without_retry() {
    run_future_with_large_stack(async {
        async fn incompatible_upstream(
            State(attempts): State<Arc<AtomicUsize>>,
            body: Bytes,
        ) -> impl IntoResponse {
            let attempt_index = attempts.fetch_add(1, Ordering::SeqCst);
            let request_body: Value = serde_json::from_slice(&body).expect("decode upstream body");
            assert!(
                request_body["tools"]
                    .as_array()
                    .expect("tools should be injected")
                    .iter()
                    .any(|tool| tool["type"].as_str() == Some("namespace")
                        && tool["name"].as_str() == Some("image_gen")),
                "Codex imagegen namespace should be injected: {request_body:?}"
            );
            if attempt_index == 0 {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({
                        "error": {
                            "message": "Upstream request failed",
                            "type": "upstream_error"
                        }
                    })),
                )
                    .into_response()
            } else {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "error": {
                            "message": "temporary upstream overload",
                            "type": "upstream_error"
                        }
                    })),
                )
                    .into_response()
            }
        }

        let attempts = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/v1/responses", post(incompatible_upstream))
            .with_state(attempts.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind incompatible upstream");
        let upstream_base = format!("http://{}", listener.local_addr().expect("local addr"));
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("incompatible upstream server should run");
        });
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Codex Imagegen Incompatible",
            "upstream-codex-imagegen-incompatible",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET policy_codex_imagegen_rewrite_mode = ?2,
                policy_codex_imagegen_capability_override = 'supported'
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .bind("force_add")
        .execute(&state.pool)
        .await
        .expect("mark account for Codex imagegen injection");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let error = proxy_openai_v1_via_pool(
            state.clone(),
            6345,
            &"/v1/responses".parse().expect("valid uri"),
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
                    HeaderName::from_static("originator"),
                    HeaderValue::from_static("Codex Desktop"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5.1-codex","input":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect_err("via-pool request should preserve the upstream 502");
        assert_eq!(error.status, StatusCode::BAD_GATEWAY);
        assert_eq!(error.code.as_deref(), Some("upstream_http_5xx"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        let (capability, route_failures, override_value): (String, i64, Option<String>) = sqlx::query_as(
            "SELECT codex_imagegen_capability, consecutive_route_failures, policy_codex_imagegen_capability_override FROM pool_upstream_accounts WHERE id = ?",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load Codex imagegen capability after 502");
        assert_eq!(capability, "unsupported");
        assert_eq!(route_failures, 0);
        assert_eq!(override_value, None);

        sqlx::query(
            "UPDATE pool_upstream_accounts SET policy_codex_imagegen_capability_override = 'supported' WHERE id = ?",
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("permit one explicit non-502 Codex imagegen retest");
        let error = proxy_openai_v1_via_pool(
            state.clone(),
            6347,
            &"/v1/responses".parse().expect("valid uri"),
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
                    HeaderName::from_static("originator"),
                    HeaderValue::from_static("Codex Desktop"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5.1-codex","input":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect_err("non-502 injected retest should fail through the pool");
        assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        let override_value: Option<String> = sqlx::query_scalar(
            "SELECT policy_codex_imagegen_capability_override FROM pool_upstream_accounts WHERE id = ?",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load Codex imagegen override after non-502 retest");
        assert_eq!(override_value, None);

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_successful_codex_imagegen_retest_restores_observed_capability() {
    run_future_with_large_stack(async {
        async fn compatible_upstream(body: Bytes) -> impl IntoResponse {
            let request_body: Value = serde_json::from_slice(&body).expect("decode upstream body");
            assert!(
                request_body["tools"]
                    .as_array()
                    .expect("tools should be injected")
                    .iter()
                    .any(|tool| tool["type"].as_str() == Some("namespace")
                        && tool["name"].as_str() == Some("image_gen")),
                "Codex imagegen namespace should be injected: {request_body:?}"
            );
            (StatusCode::OK, Json(json!({"ok": true}))).into_response()
        }

        let app = Router::new().route("/v1/responses", post(compatible_upstream));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind compatible upstream");
        let upstream_base = format!("http://{}", listener.local_addr().expect("local addr"));
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("compatible upstream server should run");
        });
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Codex Imagegen Retest",
            "upstream-codex-imagegen-retest",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET policy_codex_imagegen_rewrite_mode = 'force_add',
                codex_imagegen_capability = 'unsupported',
                policy_codex_imagegen_capability_override = 'supported'
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("permit one explicit Codex imagegen retest");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6346,
            &"/v1/responses".parse().expect("valid uri"),
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
                    HeaderName::from_static("originator"),
                    HeaderValue::from_static("Codex Desktop"),
                ),
            ]),
            Body::from(r#"{"model":"gpt-5.1-codex","input":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("explicit retest should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("consume successful retest response");
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
        let attempt_summary: Option<String> = sqlx::query_scalar(
            "SELECT request_summary_json FROM pool_upstream_request_attempts ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&state.pool)
        .await
        .expect("load request attempt summary");
        assert!(
            attempt_summary
                .as_deref()
                .is_some_and(|summary| summary.contains("\"outcome\":\"injected\"")),
            "successful retest attempt should retain its injected namespace audit: {attempt_summary:?}"
        );

        let (capability, reason, override_value): (String, Option<String>, Option<String>) = sqlx::query_as(
            "SELECT codex_imagegen_capability, codex_imagegen_capability_reason, policy_codex_imagegen_capability_override FROM pool_upstream_accounts WHERE id = ?",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load Codex imagegen capability after retest");
        assert_eq!(capability, "supported");
        assert_eq!(
            reason.as_deref(),
            Some("Codex imagegen namespace request succeeded")
        );
        assert_eq!(override_value, None);

        upstream_handle.abort();
    });
}

#[test]
fn codex_imagegen_supported_retest_claim_is_atomic() {
    run_future_with_large_stack(async {
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse("http://127.0.0.1:1").expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        let account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Codex Imagegen Atomic Retest",
            "upstream-codex-imagegen-atomic-retest",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET codex_imagegen_capability = 'unsupported',
                policy_codex_imagegen_capability_override = 'supported'
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("permit one explicit Codex imagegen retest");

        let first_pool = state.pool.clone();
        let second_pool = state.pool.clone();
        let (first, second) = tokio::join!(
            claim_codex_imagegen_supported_retest_override(&first_pool, account_id),
            claim_codex_imagegen_supported_retest_override(&second_pool, account_id),
        );
        let claims = [
            first.expect("first retest claim should succeed"),
            second.expect("second retest claim should succeed"),
        ];
        assert_eq!(
            claims
                .iter()
                .filter(|claim| **claim == CodexImagegenRetestClaim::Claimed)
                .count(),
            1
        );
        assert_eq!(
            claims
                .iter()
                .filter(|claim| **claim == CodexImagegenRetestClaim::AlreadyClaimed)
                .count(),
            1
        );
    });
}

#[test]
fn proxy_openai_v1_image_edits_ignores_response_image_tool_capability_gate() {
    run_future_with_large_stack(async move {
        async fn direct_image_echo_upstream(headers: HeaderMap, body: Bytes) -> Response {
            let authorization = headers
                .get(http_header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let request_body: Value = serde_json::from_slice(&body).expect("decode upstream body");
            (
                StatusCode::OK,
                Json(json!({
                    "authorization": authorization,
                    "requestBody": request_body,
                })),
            )
                .into_response()
        }

        let app = Router::new().route("/v1/images/edits", post(direct_image_echo_upstream));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind direct image edits upstream");
        let upstream_base = format!("http://{}", listener.local_addr().expect("local addr"));
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("direct image edits upstream server should run");
        });
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Image Edits Endpoint Only",
            "upstream-image-edits-only",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET image_endpoint_capability = 'supported',
                response_image_tool_capability = 'unsupported'
            WHERE id = ?1
            "#,
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("seed direct image edits capability split");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6345,
            &"/v1/images/edits".parse().expect("valid uri"),
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
            Body::from(r#"{"model":"gpt-image-1","prompt":"repair this cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("direct image edits request should not be filtered by response image-tool gate");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read direct image edits response");
        let payload: Value = serde_json::from_slice(&body).expect("decode direct image edits body");
        assert_eq!(payload["authorization"], "Bearer upstream-image-edits-only");
        assert_eq!(payload["requestBody"]["model"], "gpt-image-1");

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_responses_image_requests_ignore_image_endpoint_capability_gate() {
    run_future_with_large_stack(async {
        async fn responses_image_echo_upstream(headers: HeaderMap, body: Bytes) -> Response {
            let authorization = headers
                .get(http_header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let request_body: Value = serde_json::from_slice(&body).expect("decode upstream body");
            (
                StatusCode::OK,
                Json(json!({
                    "authorization": authorization,
                    "requestBody": request_body,
                })),
            )
                .into_response()
        }

        let app = Router::new().route("/v1/responses", post(responses_image_echo_upstream));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind responses image upstream");
        let upstream_base = format!("http://{}", listener.local_addr().expect("local addr"));
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("responses image upstream server should run");
        });
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let account_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Responses Image Tool Only",
            "upstream-responses-image-only",
            None,
            None,
            None,
        )
        .await;
        sqlx::query(
            r#"
        UPDATE pool_upstream_accounts
        SET response_endpoint_capability = 'supported',
            image_endpoint_capability = 'unsupported',
            response_image_tool_capability = 'supported'
        WHERE id = ?1
        "#,
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("seed responses image capability split");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6346,
            &"/v1/responses".parse().expect("valid uri"),
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
            Body::from(r#"{"model":"gpt-image-1","input":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("responses image request should not be filtered by image endpoint gate");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read responses image response");
        let payload: Value = serde_json::from_slice(&body).expect("decode responses image body");
        assert_eq!(
            payload["authorization"],
            "Bearer upstream-responses-image-only"
        );
        assert_eq!(payload["requestBody"]["model"], "gpt-image-1");

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn proxy_openai_v1_chat_and_responses_use_independent_endpoint_capability_gates() {
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(20),
    )
    .await;

    let responses_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Responses Only",
        "upstream-responses-only",
        None,
        None,
        None,
    )
    .await;
    let chat_account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Chat Only",
        "upstream-chat-only",
        None,
        None,
        None,
    )
    .await;

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET response_endpoint_capability = 'supported',
            chat_completions_capability = 'unsupported'
        WHERE id = ?1
        "#,
    )
    .bind(responses_account_id)
    .execute(&state.pool)
    .await
    .expect("seed responses-only capability split");
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET response_endpoint_capability = 'unsupported',
            chat_completions_capability = 'supported'
        WHERE id = ?1
        "#,
    )
    .bind(chat_account_id)
    .execute(&state.pool)
    .await
    .expect("seed chat-only capability split");

    let mut chat_wait_deadline = None;
    let chat_resolution = resolve_pool_account_for_request_with_wait_and_image_intent(
        state.as_ref(),
        None,
        Some("gpt-5.4"),
        &[],
        &HashSet::new(),
        None,
        false,
        &mut chat_wait_deadline,
        None,
        "/v1/chat/completions",
        ImageIntent::Unknown,
    )
    .await
    .expect("resolve chat completions account");
    let PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(chat_account)) =
        chat_resolution
    else {
        panic!("chat completions request should resolve to a supported account");
    };
    assert_eq!(chat_account.account_id, chat_account_id);

    let mut responses_wait_deadline = None;
    let responses_resolution = resolve_pool_account_for_request_with_wait_and_image_intent(
        state.as_ref(),
        None,
        Some("gpt-5.4"),
        &[],
        &HashSet::new(),
        None,
        false,
        &mut responses_wait_deadline,
        None,
        "/v1/responses",
        ImageIntent::Unknown,
    )
    .await
    .expect("resolve responses account");
    let PoolAccountResolutionWithWait::Resolution(PoolAccountResolution::Resolved(
        responses_account,
    )) = responses_resolution
    else {
        panic!("responses request should resolve to a supported account");
    };
    assert_eq!(responses_account.account_id, responses_account_id);

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    assert!(runtime_timeouts.request_read_timeout > Duration::from_secs(0));
}

#[test]
fn proxy_openai_v1_responses_live_first_waits_for_image_intent_before_filtered_resolution() {
    run_future_with_large_stack(async {
        assert_live_first_waits_for_image_intent_before_filtered_resolution("/v1/responses", 6341)
            .await;
    });
}

#[test]
fn proxy_openai_v1_responses_compact_live_first_waits_for_image_intent_before_filtered_resolution()
{
    run_future_with_large_stack(async {
        assert_live_first_waits_for_image_intent_before_filtered_resolution(
            "/v1/responses/compact",
            6342,
        )
        .await;
    });
}

#[test]
fn proxy_openai_v1_responses_pool_runtime_exposes_remote_v2_compaction_request_kind() {
    run_future_with_large_stack(async move {
        let (upstream_base, _attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Remote V2 Account",
            "upstream-remote-v2",
            None,
            None,
            None,
        )
        .await;

        let mut rx = state.broadcaster.subscribe();
        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
        state.clone(),
        6941,
        &"/v1/responses".parse().expect("valid uri"),
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
            r#"{"model":"gpt-5.4","stream":true,"input":"summarize this","context_management":[{"type":"compaction","compact_threshold":1234}]}"#,
        ),
        runtime_timeouts,
        None,
    )
    .await
    .expect("via-pool remote v2 request should succeed");
        assert_eq!(response.status(), StatusCode::OK);

        let running_record = loop {
            let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
                .await
                .expect("timed out waiting for runtime records payload")
                .expect("broadcast channel should stay open");
            if let BroadcastPayload::Records { records } = payload
                && let Some(record) = records.into_iter().find(|record| {
                    record.endpoint.as_deref() == Some("/v1/responses")
                        && record.compaction_request_kind.as_deref() == Some("remote_v2")
                        && record.status.as_deref() == Some("running")
                })
            {
                break record;
            }
        };
        assert_eq!(
            running_record.compaction_request_kind.as_deref(),
            Some("remote_v2")
        );
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read remote v2 via-pool response");
        assert!(
            state
                .proxy_runtime_invocations
                .snapshot()
                .iter()
                .all(|record| record.invoke_id != running_record.invoke_id),
            "completed via-pool synthetic runtime should be removed"
        );

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_responses_pool_runtime_remote_v2_survives_disabled_request_body_logging() {
    run_future_with_large_stack(async move {
        let (upstream_base, _attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Remote V2 No Request Raw",
            "upstream-remote-v2-no-request-raw",
            None,
            None,
            None,
        )
        .await;

        let _ = put_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: true,
                merge_upstream_enabled: true,
                fast_mode_rewrite_mode: None,
                upstream_429_max_retries: None,
                websocket_enabled: None,
                upstream_websocket_default_enabled: None,
                request_body_logging_enabled: Some(false),
                response_body_logging_enabled: Some(true),
                encrypted_session_owner_routing_enabled: None,
                enabled_models: default_enabled_preset_models(),
            }),
        )
        .await
        .expect("disable request body logging");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
        state.clone(),
        6942,
        &"/v1/responses".parse().expect("valid uri"),
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
            r#"{"model":"gpt-5.4","stream":true,"input":"summarize this","context_management":[{"type":"compaction","compact_threshold":1234}]}"#,
        ),
        runtime_timeouts,
        None,
    )
    .await
    .expect("via-pool remote v2 request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let record = state
            .proxy_runtime_invocations
            .snapshot()
            .into_iter()
            .find(|record| record.invoke_id == "pool-via-6942")
            .expect("remote v2 runtime invocation should exist while streaming");
        assert_eq!(record.compaction_request_kind.as_deref(), Some("remote_v2"));
        assert_eq!(record.request_raw_path, None);
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read remote v2 via-pool response");

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_responses_pool_runtime_exposes_image_intent_for_image_model_requests() {
    run_future_with_large_stack(async move {
        let (upstream_base, _attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Responses Image Intent",
            "upstream-responses-image-intent",
            None,
            None,
            None,
        )
        .await;

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6943,
            &"/v1/responses".parse().expect("valid uri"),
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
            Body::from(r#"{"model":"gpt-image-1","input":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool responses image request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let record = state
            .proxy_runtime_invocations
            .snapshot()
            .into_iter()
            .find(|record| record.invoke_id == "pool-via-6943")
            .expect("image-intent runtime invocation should exist while streaming");
        assert_eq!(record.image_intent.as_deref(), Some("yes"));
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read responses image response");

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_direct_image_pool_runtime_exposes_direct_image_intent() {
    run_future_with_large_stack(async move {
        async fn direct_image_echo_upstream(headers: HeaderMap, body: Bytes) -> Response {
            let authorization = headers
                .get(http_header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let request_body: Value = serde_json::from_slice(&body).expect("decode upstream body");
            (
                StatusCode::OK,
                Json(json!({
                    "authorization": authorization,
                    "requestBody": request_body,
                })),
            )
                .into_response()
        }

        let app = Router::new().route("/v1/images/generations", post(direct_image_echo_upstream));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind direct image upstream");
        let upstream_base = format!("http://{}", listener.local_addr().expect("local addr"));
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("direct image upstream server should run");
        });
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Direct Image Intent",
            "upstream-direct-image-intent",
            None,
            None,
            None,
        )
        .await;

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6944,
            &"/v1/images/generations".parse().expect("valid uri"),
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
            Body::from(r#"{"model":"gpt-image-1","prompt":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool direct image request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let record = state
            .proxy_runtime_invocations
            .snapshot()
            .into_iter()
            .find(|record| record.invoke_id == "pool-via-6944")
            .expect("direct image runtime invocation should exist while streaming");
        assert_eq!(record.image_intent.as_deref(), Some("direct_image"));
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read direct image response");

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_direct_image_timeout_returns_504_without_retry() {
    run_future_with_large_stack(async move {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_upstream = attempts.clone();
        let app = Router::new().route(
            "/v1/images/generations",
            post(move |body: Bytes| {
                let attempts = attempts_for_upstream.clone();
                async move {
                    if !body
                        .windows(b"draw a cat".len())
                        .any(|window| window == b"draw a cat")
                    {
                        return (StatusCode::OK, Json(json!({ "data": [] }))).into_response();
                    }
                    attempts.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    (StatusCode::OK, Json(json!({ "data": [] }))).into_response()
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind slow direct image upstream");
        let upstream_base = format!("http://{}", listener.local_addr().expect("local addr"));
        let upstream_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("slow direct image upstream should run");
        });
        let mut state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        Arc::get_mut(&mut state)
            .expect("test state is uniquely owned")
            .config
            .openai_proxy_image_handshake_timeout = Duration::from_millis(50);
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Direct Image Timeout",
            "upstream-direct-image-timeout",
            None,
            None,
            None,
        )
        .await;
        attempts.store(0, Ordering::SeqCst);

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let err = proxy_openai_v1_via_pool(
            state.clone(),
            7044,
            &"/v1/images/generations".parse().expect("valid uri"),
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
            Body::from(r#"{"model":"gpt-image-1","prompt":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect_err("slow direct image request should time out");

        assert_eq!(err.status, StatusCode::GATEWAY_TIMEOUT);
        assert!(
            err.message
                .contains(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        let persisted_attempts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id = 'pool-via-7044'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("count direct image attempts");
        assert_eq!(persisted_attempts, 1);
        assert!(
            state
                .pool_routing_reservations
                .lock()
                .expect("lock routing reservations")
                .is_empty(),
            "timed-out direct image request must release its routing reservation"
        );

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_responses_pool_runtime_image_intent_survives_disabled_request_body_logging() {
    run_future_with_large_stack(async move {
        let (upstream_base, _attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Responses Image Intent No Request Raw",
            "upstream-responses-image-no-request-raw",
            None,
            None,
            None,
        )
        .await;

        let _ = put_proxy_settings(
            State(state.clone()),
            HeaderMap::new(),
            Json(ProxyModelSettingsUpdateRequest {
                hijack_enabled: true,
                merge_upstream_enabled: true,
                fast_mode_rewrite_mode: None,
                upstream_429_max_retries: None,
                websocket_enabled: None,
                upstream_websocket_default_enabled: None,
                request_body_logging_enabled: Some(false),
                response_body_logging_enabled: Some(true),
                encrypted_session_owner_routing_enabled: None,
                enabled_models: default_enabled_preset_models(),
            }),
        )
        .await
        .expect("disable request body logging");

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6945,
            &"/v1/responses".parse().expect("valid uri"),
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
            Body::from(r#"{"model":"gpt-image-1","input":"draw a cat"}"#),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool responses image request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let record = state
            .proxy_runtime_invocations
            .snapshot()
            .into_iter()
            .find(|record| record.invoke_id == "pool-via-6945")
            .expect("responses image runtime invocation should exist while streaming");
        assert_eq!(record.image_intent.as_deref(), Some("yes"));
        assert_eq!(record.request_raw_path, None);
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read responses image response");

        upstream_handle.abort();
    });
}

async fn assert_live_first_waits_for_image_intent_before_filtered_resolution(
    path: &str,
    proxy_request_id: u64,
) {
    let mut config = test_config();
    config.openai_proxy_request_read_timeout = Duration::from_millis(260);
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");

    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(120),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary Text Only",
        "upstream-primary",
        None,
        None,
        None,
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Fallback Image Capable",
        "upstream-fallback",
        None,
        None,
        None,
    )
    .await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_response_image_tool_capability_override = 'unsupported'
        WHERE id = ?1
        "#,
    )
    .bind(primary_id)
    .execute(&state.pool)
    .await
    .expect("mark primary account as response image-tool incompatible");

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    let first_chunk = format!(
        "{{\"model\":\"gpt-5.5\",\"input\":\"{}",
        "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
    );
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
        tokio::time::sleep(Duration::from_millis(130)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(
                b"\",\"tools\":[{\"type\":\"image_generation\"}]}",
            )))
            .await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        proxy_request_id,
        &path.parse().expect("valid uri"),
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
        runtime_timeouts,
        None,
    )
    .await
    .expect("via-pool request should wait for delayed image intent");
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(100),
        "image-sensitive request should wait for delayed image intent, elapsed={elapsed:?}"
    );
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read via-pool response");
    let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
    assert_eq!(payload["authorization"], "Bearer upstream-fallback");
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-fallback").copied(), Some(1));

    upstream_handle.abort();
}

#[test]
fn proxy_openai_v1_live_first_waits_for_full_model_before_filtered_resolution() {
    run_future_with_large_stack(async move {
        let mut config = test_config();
        config.openai_proxy_request_read_timeout = Duration::from_millis(260);
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        config.openai_upstream_base_url =
            Url::parse(&upstream_base).expect("valid upstream base url");

        let state = test_state_from_config_with_pool_no_available_wait(
            config,
            true,
            PoolNoAvailableWaitSettings {
                timeout: Duration::from_millis(120),
                retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
            },
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary GPT-4.1 only", "upstream-primary")
                .await;
        insert_test_pool_api_key_account(&state, "Fallback GPT-5.5", "upstream-fallback").await;
        sqlx::query(
            "UPDATE pool_upstream_accounts SET policy_available_models_json = ?2 WHERE id = ?1",
        )
        .bind(primary_id)
        .bind(r#"["gpt-4.1"]"#)
        .execute(&state.pool)
        .await
        .expect("restrict primary account model policy");

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        let first_chunk = format!(
            "{{\"input\":\"{}",
            "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
        );
        tokio::spawn(async move {
            let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
            tokio::time::sleep(Duration::from_millis(130)).await;
            let _ = tx
                .send(Ok(Bytes::from_static(b"\",\"model\":\"gpt-5.5\"}")))
                .await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let started = Instant::now();
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6342,
            &"/v1/chat/completions".parse().expect("valid uri"),
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
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool request should succeed after reading the full model");
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(100),
            "model-constrained request should wait for the delayed model field, elapsed={elapsed:?}"
        );
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
        assert_eq!(payload["authorization"], "Bearer upstream-fallback");
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
            assert_eq!(attempts.get("Bearer upstream-fallback").copied(), Some(1));
        }

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_live_first_ignores_nested_prefix_model_before_top_level_model() {
    run_future_with_large_stack(async move {
        let mut config = test_config();
        config.openai_proxy_request_read_timeout = Duration::from_millis(260);
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        config.openai_upstream_base_url =
            Url::parse(&upstream_base).expect("valid upstream base url");

        let state = test_state_from_config_with_pool_no_available_wait(
            config,
            true,
            PoolNoAvailableWaitSettings {
                timeout: Duration::from_millis(120),
                retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
            },
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let primary_id =
            insert_test_pool_api_key_account(&state, "Primary GPT-4.1 only", "upstream-primary")
                .await;
        insert_test_pool_api_key_account(&state, "Fallback GPT-5.5", "upstream-fallback").await;
        sqlx::query(
            "UPDATE pool_upstream_accounts SET policy_available_models_json = ?2 WHERE id = ?1",
        )
        .bind(primary_id)
        .bind(r#"["gpt-4.1"]"#)
        .execute(&state.pool)
        .await
        .expect("restrict primary account model policy");

        let nested_prefix = format!(
            "{{\"input\":\"{{\\\"model\\\":\\\"gpt-4o\\\"}}{}",
            "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
        );
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        tokio::spawn(async move {
            let _ = tx.send(Ok(Bytes::from(nested_prefix))).await;
            tokio::time::sleep(Duration::from_millis(130)).await;
            let _ = tx
                .send(Ok(Bytes::from_static(b"\",\"model\":\"gpt-5.5\"}")))
                .await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let started = Instant::now();
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6344,
            &"/v1/chat/completions".parse().expect("valid uri"),
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
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool request should wait for the top-level model");
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(100),
            "nested prefix model should not trigger early routing, elapsed={elapsed:?}"
        );
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
        assert_eq!(payload["authorization"], "Bearer upstream-fallback");
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;

        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
            assert_eq!(attempts.get("Bearer upstream-fallback").copied(), Some(1));
        }

        upstream_handle.abort();
    });
}

#[test]
fn proxy_openai_v1_header_sticky_stream_body_override_beats_blocked_policy_header() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(80),
            Duration::from_millis(20),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let blocked_id =
            insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
        let replacement_id =
            insert_test_pool_api_key_account(&state, "Replacement", "upstream-replacement").await;
        sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
            .bind(blocked_id)
            .execute(&state.pool)
            .await
            .expect("clear blocked group");

        let now_iso = format_utc_iso(Utc::now());
        let disallow_cut_out_tag_id: i64 = sqlx::query_scalar(
            r#"
        INSERT INTO pool_tags (
            name,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, 1, ?2, ?2)
        RETURNING id
        "#,
        )
        .bind("via-pool-no-cut-out")
        .bind(&now_iso)
        .fetch_one(&state.pool)
        .await
        .expect("insert no-cut-out tag");
        sqlx::query(
            r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
        )
        .bind(blocked_id)
        .bind(disallow_cut_out_tag_id)
        .bind(&now_iso)
        .execute(&state.pool)
        .await
        .expect("attach no-cut-out tag");

        let sticky_seen_at = format_utc_iso(Utc::now());
        upsert_test_sticky_route_at(
            &state.pool,
            "header-blocked-policy-sticky",
            blocked_id,
            &sticky_seen_at,
        )
        .await;
        upsert_test_sticky_route_at(
            &state.pool,
            "body-live-sticky",
            replacement_id,
            &sticky_seen_at,
        )
        .await;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
                .await;
            tokio::time::sleep(Duration::from_millis(170)).await;
            let _ = tx
                .send(Ok(Bytes::from_static(
                    b"\"messages\":[],\"stickyKey\":\"body-live-sticky\"}",
                )))
                .await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let started = Instant::now();
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            6246,
            &"/v1/chat/completions".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("header-blocked-policy-sticky"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool request should succeed");
        let elapsed = started.elapsed();

        assert!(
            elapsed >= Duration::from_millis(140),
            "request should wait for the body sticky override before resolving, elapsed={elapsed:?}"
        );
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
        assert_eq!(payload["authorization"], "Bearer upstream-replacement");
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
        assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-blocked").copied(), None);
        assert_eq!(
            attempts.get("Bearer upstream-replacement").copied(),
            Some(1)
        );

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_prefers_body_too_large_before_pool_wait() {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_max_request_body_bytes = 24;
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(200),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;

    let body = Body::from_stream(tokio_stream::iter(vec![Ok::<Bytes, io::Error>(
        Bytes::from_static(
            b"{\"model\":\"gpt-5\",\"messages\":[{\"role\":\"user\",\"content\":\"too large\"}]}",
        ),
    )]));

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6243,
        &"/v1/chat/completions".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("known-stream-sticky"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        body,
        runtime_timeouts,
        None,
    )
    .await;

    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        message, "request body exceeds 24 bytes",
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[test]
fn proxy_openai_v1_header_sticky_stream_waits_after_body_reroute_needs_account() {
    run_future_with_large_stack(async move {
        let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
        let state = test_state_with_openai_base_and_pool_no_available_wait(
            Url::parse(&upstream_base).expect("valid upstream base url"),
            Duration::from_millis(220),
            Duration::from_millis(10),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let initial_id =
            insert_test_pool_api_key_account(&state, "Initial", "upstream-initial").await;
        let delayed_id =
            insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
        set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
        let (body_reroute_tx, body_reroute_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = tx
                .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
                .await;
            tokio::time::sleep(Duration::from_millis(170)).await;
            let _ = body_reroute_tx.send(());
            let _ = tx
                .send(Ok(Bytes::from_static(
                    b"\"messages\":[],\"stickyKey\":\"body-reroute-sticky\"}",
                )))
                .await;
        });

        let pool = state.pool.clone();
        let initial_block_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            set_test_account_status(&pool, initial_id, "needs_reauth").await;
        });

        let pool = state.pool.clone();
        let delayed_release_task = tokio::spawn(async move {
            body_reroute_rx
                .await
                .expect("body reroute signal should arrive");
            tokio::time::sleep(Duration::from_millis(20)).await;
            set_test_account_status(&pool, delayed_id, "active").await;
        });

        let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts");
        let response = proxy_openai_v1_via_pool(
            state.clone(),
            7242,
            &"/v1/chat/completions".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-live-key"),
                ),
                (
                    HeaderName::from_static("x-sticky-key"),
                    HeaderValue::from_static("header-reroute-sticky"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
            runtime_timeouts,
            None,
        )
        .await
        .expect("via-pool request should succeed");

        initial_block_task
            .await
            .expect("initial account block task should join");
        delayed_release_task
            .await
            .expect("delayed account release task should join");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read via-pool response");
        let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
        assert_eq!(payload["authorization"], "Bearer upstream-delayed");
        wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
        assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

        let attempts = attempts.lock().expect("lock attempts");
        assert_eq!(attempts.get("Bearer upstream-initial").copied(), None);
        assert_eq!(attempts.get("Bearer upstream-delayed").copied(), Some(1));

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_reroute_preserves_original_wait_window() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(200),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let (body_reroute_tx, body_reroute_rx) = tokio::sync::oneshot::channel::<()>();
    let body = Body::from_stream(futures_util::stream::unfold(
        (0_u8, Some(body_reroute_tx)),
        |(step, body_reroute_tx)| async move {
            match step {
                0 => Some((
                    Ok::<Bytes, io::Error>(Bytes::from_static(b"{\"model\":\"gpt-5\",")),
                    (1, body_reroute_tx),
                )),
                1 => {
                    tokio::time::sleep(Duration::from_millis(140)).await;
                    if let Some(body_reroute_tx) = body_reroute_tx {
                        let _ = body_reroute_tx.send(());
                    }
                    Some((
                        Ok::<Bytes, io::Error>(Bytes::from_static(
                            b"\"messages\":[],\"stickyKey\":\"body-reroute-sticky\"}",
                        )),
                        (2, None),
                    ))
                }
                _ => None,
            }
        },
    ));

    let pool = state.pool.clone();
    let delayed_release_task = tokio::spawn(async move {
        body_reroute_rx
            .await
            .expect("body reroute signal should arrive");
        tokio::time::sleep(Duration::from_millis(120)).await;
        set_test_account_status(&pool, delayed_id, "active").await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        7342,
        &"/v1/chat/completions".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("header-reroute-sticky"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        body,
        runtime_timeouts,
        None,
    )
    .await;
    let elapsed = started.elapsed();

    delayed_release_task
        .await
        .expect("delayed release task should join");

    assert!(
        elapsed < Duration::from_millis(600),
        "rerouted sticky requests should finish without waiting through another full bounded window, elapsed={elapsed:?}"
    );
    let err = response.expect_err("via-pool request should fail");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(message, POOL_NO_AVAILABLE_ACCOUNT_MESSAGE);
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-blocked").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-delayed").copied(), None);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_oauth_passthrough_streams_without_eager_prebuffering() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) = spawn_oauth_codex_capture_upstream().await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let state = test_state_with_openai_base_body_limit_and_read_timeout(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(200),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_oauth_account(&state, "Streaming OAuth", "oauth-streaming").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from_static(b"{\"messages\":["))).await;
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(
                b"{\"role\":\"user\",\"content\":\"hello\"}]}",
            )))
            .await;
    });

    let uri = "/v1/chat/completions".parse().expect("valid uri");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        42,
        &uri,
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("sticky-oauth-stream"),
            ),
            (
                HeaderName::from_static("x-prompt-cache-key"),
                HeaderValue::from_static("prompt-cache-oauth-stream"),
            ),
            (
                HeaderName::from_static("x-openai-prompt-cache-key"),
                HeaderValue::from_static("prompt-cache-oauth-stream-openai"),
            ),
            (
                HeaderName::from_static("x-client-trace-id"),
                HeaderValue::from_static("trace-oauth-stream"),
            ),
            (
                HeaderName::from_static("session_id"),
                HeaderValue::from_static("session-oauth-stream"),
            ),
            (
                HeaderName::from_static("traceparent"),
                HeaderValue::from_static("00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01"),
            ),
            (
                HeaderName::from_static("x-client-request-id"),
                HeaderValue::from_static("client-request-oauth-stream"),
            ),
            (
                HeaderName::from_static("x-codex-turn-metadata"),
                HeaderValue::from_static("{\"stream\":true}"),
            ),
            (
                HeaderName::from_static("originator"),
                HeaderValue::from_static("Codex Desktop"),
            ),
            (
                HeaderName::from_static("x-forwarded-for"),
                HeaderValue::from_static("203.0.113.8"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        pool_routing_timeouts_from_config(&state.config),
        None,
    )
    .await
    .expect("oauth pool passthrough response");

    let status = response.status();
    let response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read oauth passthrough response");
    assert_eq!(
        status,
        StatusCode::OK,
        "unexpected oauth passthrough response: {}",
        String::from_utf8_lossy(&response_body)
    );
    let payload: Value =
        serde_json::from_slice(&response_body).expect("decode oauth passthrough response");
    assert_eq!(
        payload["authorization"].as_str(),
        Some("Bearer oauth-streaming")
    );
    assert!(payload["stickyKeyHeader"].is_null());
    assert_eq!(
        payload["promptCacheKeyHeader"].as_str(),
        Some("prompt-cache-oauth-stream")
    );
    assert_eq!(
        payload["xOpenAiPromptCacheKeyHeader"].as_str(),
        Some("prompt-cache-oauth-stream-openai")
    );
    assert_eq!(
        payload["clientTraceId"].as_str(),
        Some("trace-oauth-stream")
    );
    assert!(payload["forwardedFor"].is_null());
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
            .any(|name| name == "x-client-request-id")
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}

fn test_live_first_pool_account(
    request_compression_algorithm: RequestCompressionAlgorithm,
) -> PoolResolvedAccount {
    PoolResolvedAccount {
        account_id: 1,
        display_name: "Live First".to_string(),
        kind: UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX.to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer upstream-live-first".to_string(),
        },
        group_name: None,
        bound_proxy_keys: Vec::new(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(None, Vec::new()),
        single_account_rotation_enabled: false,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
        upstream_base_url: Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        sticky_affinity_generation: None,
        routing_selection_audit: None,
    }
}

#[test]
fn pool_account_supports_live_request_body_rejects_non_identity_request_compression() {
    let account = test_live_first_pool_account(RequestCompressionAlgorithm::Gzip);

    assert!(!pool_account_supports_live_request_body(
        &account,
        &"/v1/chat/completions".parse().expect("valid uri"),
        &Method::POST,
        &HeaderMap::new(),
    ));
}

#[test]
fn pool_account_supports_live_request_body_rejects_downstream_content_encoding() {
    let account = test_live_first_pool_account(RequestCompressionAlgorithm::Identity);
    let headers = HeaderMap::from_iter([(
        http_header::CONTENT_ENCODING,
        HeaderValue::from_static("gzip"),
    )]);

    assert!(!pool_account_supports_live_request_body(
        &account,
        &"/v1/chat/completions".parse().expect("valid uri"),
        &Method::POST,
        &headers,
    ));
}

#[test]
fn pool_account_supports_live_request_body_allows_codex_keep_original() {
    let account = test_live_first_pool_account(RequestCompressionAlgorithm::Identity);
    let headers = HeaderMap::from_iter([(
        HeaderName::from_static("x-openai-internal-codex-responses-lite"),
        HeaderValue::from_static("true"),
    )]);

    assert!(pool_account_supports_live_request_body(
        &account,
        &"/v1/responses".parse().expect("valid uri"),
        &Method::POST,
        &headers,
    ));
}

#[test]
fn pool_account_supports_live_request_body_transforms_codex_rewrite() {
    let mut account = test_live_first_pool_account(RequestCompressionAlgorithm::Identity);
    account.codex_imagegen_rewrite_mode = crate::CodexImagegenRewriteMode::ForceAdd;
    let headers = HeaderMap::from_iter([(
        HeaderName::from_static("x-openai-internal-codex-responses-lite"),
        HeaderValue::from_static("true"),
    )]);

    assert!(pool_account_supports_live_request_body(
        &account,
        &"/v1/responses".parse().expect("valid uri"),
        &Method::POST,
        &headers,
    ));
}

#[test]
fn pool_account_supports_live_responses_oauth_and_request_compression() {
    let mut account = test_live_first_pool_account(RequestCompressionAlgorithm::Zstd);
    account.auth = PoolResolvedAuth::Oauth {
        access_token: "oauth-live-first".to_string(),
        chatgpt_account_id: Some("account-live-first".to_string()),
    };
    let headers = HeaderMap::from_iter([(
        http_header::CONTENT_ENCODING,
        HeaderValue::from_static("gzip"),
    )]);

    assert!(pool_account_supports_live_request_body(
        &account,
        &"/v1/responses".parse().expect("valid uri"),
        &Method::POST,
        &headers,
    ));
}

#[tokio::test]
async fn pool_route_oauth_responses_replay_body_keeps_request_started_total_timeout_without_wait() {
    const TOTAL_TIMEOUT: Duration = Duration::from_secs(5);
    const BODY_REPLAY_DELAY: Duration = Duration::from_secs(3);
    const UPSTREAM_HEADER_DELAY: Duration = Duration::from_secs(3);

    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) =
        spawn_oauth_codex_delayed_headers_upstream(UPSTREAM_HEADER_DELAY).await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = TOTAL_TIMEOUT;
    config.openai_proxy_request_read_timeout = Duration::from_secs(10);
    config.openai_proxy_handshake_timeout = Duration::from_secs(10);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_oauth_account(&state, "Timeout OAuth", "oauth-timeout").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5.4\",")))
            .await;
        tokio::time::sleep(BODY_REPLAY_DELAY).await;
        let _ = tx
            .send(Ok(Bytes::from_static(
                b"\"stream\":false,\"input\":\"hello\"}",
            )))
            .await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6420,
        &"/v1/responses".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            ),
            (
                HeaderName::from_static("x-sticky-key"),
                HeaderValue::from_static("sticky-oauth-replay-timeout"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        runtime_timeouts,
        None,
    )
    .await;

    let err = response.expect_err("oauth replay request should hit total timeout");
    let status = err.status;
    let message = err.message;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        message,
        pool_total_timeout_exhausted_message(TOTAL_TIMEOUT),
        "unexpected oauth replay timeout error: {message}"
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}
