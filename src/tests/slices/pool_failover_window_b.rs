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
    let (status, message) = response.expect_err("via-pool request should fail");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        message, POOL_NO_AVAILABLE_ACCOUNT_MESSAGE,
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn proxy_openai_v1_chunked_json_without_header_sticky_uses_live_first_attempt() {
    let mut config = test_config();
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");

    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(80),
            poll_interval: Duration::from_millis(10),
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
            let row = sqlx::query_as::<_, (Option<String>, Option<String>, String)>(
                r#"
                SELECT group_name_snapshot, proxy_binding_key_snapshot, status
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

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_responses_live_first_failover_restores_full_retry_budget_for_follow_up_accounts()
 {
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
}

#[tokio::test]
async fn proxy_openai_v1_live_first_unsupported_model_bad_request_fails_over() {
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
            poll_interval: Duration::from_millis(10),
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
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
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

    let attempts = attempts
        .lock()
        .expect("lock live-first unsupported-model attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

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
        primary_tags
            .iter()
            .any(|tag| tag == "unsupported_model:gpt-5.5"),
        "primary account should learn unsupported model tag: {primary_tags:?}",
    );
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-unsupported-model-failover").await,
        None,
    );
    assert_ne!(primary_id, secondary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_responses_live_first_failover_preserves_prompt_cache_group_binding() {
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

    assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
    let attempts = attempts.lock().expect("lock live-first binding attempts");
    assert!(matches!(
        attempts.get("Bearer upstream-primary").copied(),
        Some(count) if count > 0
    ));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    upstream_handle.abort();
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

    assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.1, ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE);

    let attempts = attempts
        .lock()
        .expect("lock live-first replay owner guard attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);

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
    .expect("load live-first replay owner-guard terminal attempt");
    assert_eq!(last_attempt.0, None);
    assert_eq!(
        last_attempt.1.as_deref(),
        Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_responses_live_first_success_persists_encrypted_owner() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
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
}

#[tokio::test]
async fn proxy_openai_v1_responses_live_first_response_encryption_persists_encrypted_owner() {
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
}

#[tokio::test]
async fn proxy_openai_v1_responses_prebuffered_success_persists_encrypted_owner() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(10),
    )
    .await;
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
}

#[tokio::test]
async fn proxy_openai_v1_responses_header_prompt_cache_key_preserves_group_binding() {
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

    assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
    let attempts = attempts.lock().expect("lock header binding attempts");
    assert!(matches!(
        attempts.get("Bearer upstream-primary").copied(),
        Some(count) if count > 0
    ));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_bodyless_header_prompt_cache_key_preserves_group_binding() {
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
    .await
    .expect_err("bodyless header binding should not fail over outside its group");

    assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
    let attempts = attempts
        .lock()
        .expect("lock bodyless header binding attempts");
    assert!(matches!(
        attempts.get("Bearer upstream-primary").copied(),
        Some(count) if count > 0
    ));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

    upstream_handle.abort();
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
    let response = proxy_openai_v1_via_pool(
        state.clone(),
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
    .expect_err("bodyless encrypted owner lock should not reroute to another account");

    assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.1, ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE);

    let attempts = attempts
        .lock()
        .expect("lock bodyless encrypted owner lock attempts");
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

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
    .expect("load encrypted owner terminal attempt");
    assert_eq!(last_attempt.0, None);
    assert_eq!(
        last_attempt.1.as_deref(),
        Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE)
    );

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

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
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
    .expect_err("rate-limited encrypted owner lock should not reroute to another account");

    assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.1, ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE);

    let attempts = attempts
        .lock()
        .expect("lock bodyless encrypted owner rate-limited attempts");
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

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
    .expect("load encrypted owner rate-limited terminal attempt");
    assert_eq!(last_attempt.0, None);
    assert_eq!(
        last_attempt.1.as_deref(),
        Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE)
    );

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

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
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
    .expect_err("same-account newer binding should still keep encrypted owner guard");

    assert_eq!(response.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.1, ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE);

    let attempts = attempts
        .lock()
        .expect("lock same-account newer binding owner attempts");
    assert_eq!(attempts.get("Bearer upstream-owner").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);

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
    .expect("load same-account newer binding terminal attempt");
    assert_eq!(last_attempt.0, None);
    assert_eq!(
        last_attempt.1.as_deref(),
        Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE)
    );

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
            poll_interval: Duration::from_millis(10),
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
        },
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
    .expect("load websocket encrypted owner terminal attempt");
    assert_eq!(last_attempt.0, None);
    assert_eq!(
        last_attempt.1.as_deref(),
        Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE)
    );

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
            poll_interval: Duration::from_millis(10),
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
        },
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
    .expect("load websocket encrypted owner rate-limited terminal attempt");
    assert_eq!(last_attempt.0, None);
    assert_eq!(
        last_attempt.1.as_deref(),
        Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn websocket_payload_owner_guard_blocks_mismatched_payload_owner() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
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
        image_tool_capability: ImageToolCapability::Unknown,
        upstream_base_url: Url::parse("https://api.example.test").expect("valid base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
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
        .route("/v1/realtime", get(websocket_echo_upstream))
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
            poll_interval: Duration::from_millis(10),
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

    let request = format!(
        "ws://{proxy_addr}/v1/realtime?model=gpt-5-realtime"
    )
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
                "type": "conversation.item.create",
                "promptCacheKey": prompt_cache_key,
                "item": {
                    "type": "message",
                    "content": [{
                        "type": "encrypted_content",
                        "encrypted_content": "opaque"
                    }]
                }
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
            poll_interval: Duration::from_millis(10),
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
    set_test_account_status(&state.pool, owner_account_id, "needs_reauth").await;
    let sticky_only_key = "sticky-only-websocket-key";
    upsert_prompt_cache_encrypted_session_owner(&state.pool, sticky_only_key, owner_account_id)
        .await
        .expect("persist sticky-only named encrypted owner row");

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

    let err = prepare_upstream_websocket(
        state.clone(),
        5353,
        &"/v1/realtime?model=gpt-5-realtime"
            .parse()
            .expect("valid uri"),
        &headers,
        &resolve_proxy_request_timeouts(state.as_ref(), true)
            .await
            .expect("resolve pool runtime timeouts"),
        sticky_key.as_deref(),
        Some("gpt-5-realtime"),
        websocket_effective_prompt_cache_key(prompt_cache_key.as_deref()),
        binding_constraint,
        None,
        owner_auto_guard_active,
        &PoolUpstreamAttemptTraceContext {
            invoke_id: "pool-ws-5353".to_string(),
            occurred_at: shanghai_now_string(),
            endpoint: "/v1/realtime".to_string(),
            sticky_key: trace_sticky_key,
            requester_ip: None,
        },
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
            poll_interval: Duration::from_millis(20),
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
    let (status, message) = response.expect_err("via-pool request should fail");
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
            poll_interval: Duration::from_millis(20),
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
    let (status, message) = response.expect_err("via-pool request should fail");
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(
        message, "request body read timed out after 80ms",
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_waits_for_blocked_policy_header_error() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        upstream_account_id: Option<i64>,
        failure_kind: Option<String>,
        error_message: Option<String>,
    }

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(200),
            poll_interval: Duration::from_millis(20),
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
            name, block_new_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, 0, 1, ?2, ?2)
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
    let (status, message) = response.expect_err("via-pool request should fail");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        message.contains("upstream account is not assigned to a group"),
        "unexpected via-pool failure: {message}"
    );
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
    let attempt_row = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT upstream_account_id, failure_kind, error_message
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load via-pool delayed assigned-blocked attempt");
    assert_eq!(attempt_row.upstream_account_id, Some(sticky_source_id));
    assert_eq!(
        attempt_row.failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED),
    );
    assert!(
        attempt_row
            .error_message
            .as_deref()
            .is_some_and(|value| value.contains("not assigned to a group"))
    );
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_same_value_short_circuits_blocked_policy_error() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        upstream_account_id: Option<i64>,
        failure_kind: Option<String>,
        error_message: Option<String>,
    }

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(200),
            poll_interval: Duration::from_millis(20),
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
            name, block_new_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, 0, 1, ?2, ?2)
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
    let (status, message) = response.expect_err("via-pool request should fail");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        message.contains("upstream account is not assigned to a group"),
        "unexpected via-pool failure: {message}"
    );
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
    let attempt_row = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT upstream_account_id, failure_kind, error_message
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load via-pool assigned-blocked attempt");
    assert_eq!(attempt_row.upstream_account_id, Some(sticky_source_id));
    assert_eq!(
        attempt_row.failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED),
    );
    assert!(
        attempt_row
            .error_message
            .as_deref()
            .is_some_and(|value| value.contains("not assigned to a group"))
    );
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_waits_for_body_sticky_override_before_failing() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(20),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
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
            poll_interval: Duration::from_millis(10),
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
    let (status, message) = response.expect_err("via-pool request should fail");
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
            poll_interval: Duration::from_millis(100),
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

    let pool = state.pool.clone();
    let runtime_handle = tokio::runtime::Handle::current();
    let release_task = std::thread::spawn(move || {
        wait_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("header sticky request should signal once the bounded wait starts");
        runtime_handle.block_on(async move {
            set_test_account_status(&pool, delayed_id, "active").await;
        });
    });

    let response = request_task
        .await
        .expect("header sticky request task should join");
    release_task
        .join()
        .expect("delayed release thread should join");

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
            poll_interval: Duration::from_millis(10),
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
    let (status, message) = response.expect_err("via-pool request should fail");
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

    let (status, message) = response.expect_err("via-pool request should fail");
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

#[tokio::test]
async fn proxy_openai_v1_responses_prebuffer_body_wait_counts_total_timeout_from_request_start() {
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
            poll_interval: Duration::from_millis(10),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
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

    let (status, message) = response.expect_err("via-pool request should fail");
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
}

#[tokio::test]
async fn proxy_openai_v1_responses_streamed_body_counts_total_timeout_from_request_start_without_wait()
 {
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
        tokio::time::sleep(Duration::from_millis(180)).await;
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

    let (status, message) = response.expect_err("via-pool request should fail");
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert!(
        elapsed >= Duration::from_millis(70),
        "responses total timeout should still start at request admission, elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(160),
        "live first attempt should not wait for the entire streamed body before timing out, elapsed={elapsed:?}"
    );
    assert_eq!(
        message,
        pool_total_timeout_exhausted_message(Duration::from_millis(90)),
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
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
            poll_interval: Duration::from_millis(10),
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
        elapsed < Duration::from_millis(900),
        "late account recovery should still terminate within a loaded-runner sub-second timeout budget, elapsed={elapsed:?}"
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
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
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

    let pool = state.pool.clone();
    let primary_block_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        set_test_account_status(&pool, primary_id, "needs_reauth").await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
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

    primary_block_task
        .await
        .expect("primary block task should join");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read via-pool response");
    let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-replacement").copied(), None);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_body_override_beats_rate_limited_header() {
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
}

#[tokio::test]
async fn proxy_openai_v1_header_prompt_cache_binding_beats_rate_limited_sticky_terminal() {
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
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_rechecks_model_before_reusing_header_resolution() {
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
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_rechecks_image_intent_before_reusing_header_resolution() {
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
        "UPDATE pool_upstream_accounts SET policy_image_tool_rewrite_mode = ?2 WHERE id = ?1",
    )
    .bind(sticky_account_id)
    .bind("force_remove")
    .execute(&state.pool)
    .await
    .expect("mark sticky account as image-incompatible");

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
    assert_eq!(attempts.get("Bearer upstream-sticky-text-only").copied(), None);
    assert_eq!(
        attempts.get("Bearer upstream-image-fallback").copied(),
        Some(1)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_direct_image_prebuffer_preserves_image_capture_target_without_rewrite() {
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

    let app = Router::new().route(
        "/v1/images/generations",
        post(|headers, body| direct_image_echo_upstream(headers, body)),
    );
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
}

#[tokio::test]
async fn proxy_openai_v1_responses_force_add_failure_learns_image_unsupported() {
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

    let app = Router::new().route("/v1/responses", post(|body| image_unsupported_upstream(body)));
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

    let capability: String =
        sqlx::query_scalar("SELECT image_tool_capability FROM pool_upstream_accounts WHERE id = ?")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load image capability after force_add failure");
    assert_eq!(capability, "unsupported");

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_responses_live_first_waits_for_image_intent_before_filtered_resolution() {
    assert_live_first_waits_for_image_intent_before_filtered_resolution("/v1/responses", 6341)
        .await;
}

#[tokio::test]
async fn proxy_openai_v1_responses_compact_live_first_waits_for_image_intent_before_filtered_resolution(
) {
    assert_live_first_waits_for_image_intent_before_filtered_resolution(
        "/v1/responses/compact",
        6342,
    )
    .await;
}

#[tokio::test]
async fn proxy_openai_v1_responses_pool_runtime_and_terminal_records_persist_remote_v2_compaction_request_kind(
) {
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
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read remote v2 via-pool response");

    let running_record = loop {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for runtime records payload")
            .expect("broadcast channel should stay open");
        match payload {
            BroadcastPayload::Records { records } => {
                if let Some(record) = records.into_iter().find(|record| {
                    record.endpoint.as_deref() == Some("/v1/responses")
                        && record.compaction_request_kind.as_deref()
                            == Some("remote_v2")
                        && record.status.as_deref() == Some("running")
                }) {
                    break record;
                }
            }
            _ => {}
        }
    };
    assert_eq!(
        running_record.compaction_request_kind.as_deref(),
        Some("remote_v2")
    );
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            request_id: Some(running_record.invoke_id.clone()),
            page_size: Some(5),
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should include remote v2 request kind");
    let record = response
        .records
        .into_iter()
        .find(|record| record.invoke_id == running_record.invoke_id)
        .expect("persisted remote v2 invocation should exist");
    assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(record.compaction_request_kind.as_deref(), Some("remote_v2"));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_responses_pool_remote_v2_request_kind_survives_disabled_request_body_logging(
) {
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
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read remote v2 via-pool response");

    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let Json(response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            page_size: Some(10),
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should succeed");
    let record = response
        .records
        .into_iter()
        .find(|record| record.endpoint.as_deref() == Some("/v1/responses"))
        .expect("remote v2 invocation should exist");
    assert_eq!(record.compaction_request_kind.as_deref(), Some("remote_v2"));
    assert_eq!(record.request_raw_path, None);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_responses_pool_persists_image_intent_for_image_model_requests() {
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
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read responses image response");

    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            page_size: Some(5),
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should include image intent");
    let record = response
        .records
        .into_iter()
        .find(|record| {
            record.endpoint.as_deref() == Some("/v1/responses")
                && record.image_intent.as_deref() == Some("yes")
        })
        .expect("persisted image-intent invocation should exist");
    assert_eq!(record.image_intent.as_deref(), Some("yes"));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_direct_image_pool_persists_direct_image_intent() {
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

    let app = Router::new().route(
        "/v1/images/generations",
        post(|headers, body| direct_image_echo_upstream(headers, body)),
    );
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
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read direct image response");

    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let Json(response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            page_size: Some(10),
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should succeed");
    let record = response
        .records
        .into_iter()
        .find(|record| record.endpoint.as_deref() == Some("/v1/images/generations"))
        .expect("direct image invocation should exist");
    assert_eq!(record.image_intent.as_deref(), Some("direct_image"));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_responses_pool_image_intent_survives_disabled_request_body_logging() {
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
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read responses image response");

    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let Json(response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            page_size: Some(10),
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should succeed");
    let record = response
        .records
        .into_iter()
        .find(|record| record.endpoint.as_deref() == Some("/v1/responses"))
        .expect("responses image invocation should exist");
    assert_eq!(record.image_intent.as_deref(), Some("yes"));
    assert_eq!(record.request_raw_path, None);

    upstream_handle.abort();
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
            poll_interval: Duration::from_millis(10),
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
        "UPDATE pool_upstream_accounts SET policy_image_tool_rewrite_mode = ?2 WHERE id = ?1",
    )
    .bind(primary_id)
    .bind("force_remove")
    .execute(&state.pool)
    .await
    .expect("mark primary account as image-incompatible");

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

#[tokio::test]
async fn proxy_openai_v1_live_first_waits_for_full_model_before_filtered_resolution() {
    let mut config = test_config();
    config.openai_proxy_request_read_timeout = Duration::from_millis(260);
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");

    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(120),
            poll_interval: Duration::from_millis(10),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id =
        insert_test_pool_api_key_account(&state, "Primary GPT-4.1 only", "upstream-primary").await;
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

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-fallback").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_live_first_ignores_nested_prefix_model_before_top_level_model() {
    let mut config = test_config();
    config.openai_proxy_request_read_timeout = Duration::from_millis(260);
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");

    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(120),
            poll_interval: Duration::from_millis(10),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        },
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id =
        insert_test_pool_api_key_account(&state, "Primary GPT-4.1 only", "upstream-primary").await;
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

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-fallback").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_body_override_beats_blocked_policy_header() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(80),
        Duration::from_millis(20),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
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
            name, block_new_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, 0, 1, ?2, ?2)
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
            poll_interval: Duration::from_millis(20),
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

    let (status, message) = response.expect_err("via-pool request should fail");
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        message, "request body exceeds 24 bytes",
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_stream_waits_after_body_reroute_needs_account() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(220),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let initial_id = insert_test_pool_api_key_account(&state, "Initial", "upstream-initial").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
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
    let (status, message) = response.expect_err("via-pool request should fail");
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

#[tokio::test]
async fn pool_route_oauth_responses_replay_body_keeps_request_started_total_timeout_without_wait() {
    let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;

    let (upstream_base, upstream_handle) =
        spawn_oauth_codex_delayed_headers_upstream(Duration::from_millis(250)).await;
    oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse(&format!("{upstream_base}/backend-api/codex")).expect("valid oauth base url"),
    )
    .await;

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(90);
    config.openai_proxy_request_read_timeout = Duration::from_millis(500);
    config.openai_proxy_handshake_timeout = Duration::from_millis(400);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_oauth_account(&state, "Timeout OAuth", "oauth-timeout").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5.4\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(
                b"\"stream\":false,\"input\":\"hello\"}",
            )))
            .await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
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
    let elapsed = started.elapsed();

    let (status, message) = response.expect_err("oauth replay request should hit total timeout");
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        message,
        pool_total_timeout_exhausted_message(Duration::from_millis(90)),
        "unexpected oauth replay timeout error: {message}"
    );
    assert!(
        elapsed >= Duration::from_millis(70),
        "request should spend time buffering before exhausting the shared budget, elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(180),
        "replay send should keep the total timeout anchored at request start, elapsed={elapsed:?}"
    );

    upstream_handle.abort();
    oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
}
