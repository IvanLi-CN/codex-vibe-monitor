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
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

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
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

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
    let started = Instant::now();
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
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(200),
        "body-only sticky streaming request should honor a single bounded wait window, elapsed={elapsed:?}"
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
async fn proxy_openai_v1_via_pool_retries_after_live_body_replay_completes() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer upstream-live", 1)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Live Replay", "upstream-live").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(
                br#"{"model":"gpt-5","messages":[{"role":"user","#,
            )))
            .await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(br#""content":"hello"}]}"#)))
            .await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
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
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        runtime_timeouts,
        None,
    )
    .await
    .expect("via-pool request should succeed after replay retry");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read via-pool response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode via-pool response");
    assert_eq!(payload["authorization"], "Bearer upstream-live");
    assert_eq!(payload["attempt"], 2);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-live").copied(), Some(2));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_via_pool_live_retry_uses_request_start_for_total_timeout() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer upstream-live-timeout", 1)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(40);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-timeout-key").await;
    insert_test_pool_api_key_account(&state, "Live Timeout", "upstream-live-timeout").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(
                br#"{"model":"gpt-5","input":[{"role":"user","content":"hello"}],"#,
            )))
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(br#""temperature":0}"#)))
            .await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let (status, message) = proxy_openai_v1_via_pool(
        state.clone(),
        6244,
        &"/v1/responses".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-timeout-key"),
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
    .expect_err("live retry should exhaust total timeout from request start");

    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        message,
        pool_total_timeout_exhausted_message(Duration::from_millis(40))
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert!(
        attempts
            .get("Bearer upstream-live-timeout")
            .copied()
            .unwrap_or_default()
            <= 1,
        "live timeout path should not complete a successful retry"
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_via_pool_live_first_attempt_preserves_body_sticky_route() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-sticky-live-key").await;
    let default_id =
        insert_test_pool_api_key_account(&state, "Default Live Route", "route-default").await;
    let sticky_id =
        insert_test_pool_api_key_account(&state, "Sticky Live Route", "route-sticky").await;

    let resolved_without_sticky =
        match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
            .await
            .expect("resolve default live route")
        {
            PoolAccountResolution::Resolved(account) => account.account_id,
            other => panic!("expected resolved default route, got {other:?}"),
        };
    let preferred_sticky_account_id = if resolved_without_sticky == default_id {
        sticky_id
    } else {
        default_id
    };
    let expected_authorization = if preferred_sticky_account_id == default_id {
        "Bearer route-default"
    } else {
        "Bearer route-sticky"
    };
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-live-body-route",
        preferred_sticky_account_id,
        &format_test_recent_active_timestamp(Utc::now()),
    )
    .await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(
                br#"{"model":"gpt-5","messages":[{"role":"user","content":"hello"}],"stickyKey":"sticky-live-body-route","#,
            )))
            .await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(br#""temperature":0}"#)))
            .await;
    });

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
                HeaderValue::from_static("Bearer pool-sticky-live-key"),
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
    .expect("live first attempt should honor body sticky route");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read sticky live response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode sticky live response");
    assert_eq!(payload["authorization"], expected_authorization);
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock live sticky attempts");
    assert_eq!(attempts.get(expected_authorization).copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_via_pool_live_first_attempt_does_not_wait_for_eof_after_sticky_probe_prefix_is_exhausted()
 {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-no-sticky-live-key").await;
    insert_test_pool_api_key_account(&state, "No Sticky Live Route", "route-no-sticky").await;

    let first_chunk = format!(
        r#"{{"model":"gpt-5","input":"{}","#,
        "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 32)
    );

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
        tokio::time::sleep(Duration::from_millis(180)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(br#""temperature":0}"#)))
            .await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = tokio::time::timeout(
        Duration::from_millis(120),
        proxy_openai_v1_via_pool(
            state.clone(),
            6246,
            &"/v1/responses".parse().expect("valid uri"),
            Method::POST,
            HeaderMap::from_iter([
                (
                    http_header::AUTHORIZATION,
                    HeaderValue::from_static("Bearer pool-no-sticky-live-key"),
                ),
                (
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                ),
            ]),
            Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
            runtime_timeouts,
            None,
        ),
    )
    .await
    .expect("live first attempt should not wait for body EOF after the sticky probe prefix is exhausted")
    .expect("live first attempt should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read no-sticky live response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode no-sticky live response");
    assert_eq!(payload["authorization"], "Bearer route-no-sticky");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock no-sticky live attempts");
    assert_eq!(attempts.get("Bearer route-no-sticky").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_via_pool_live_first_attempt_preserves_body_sticky_route_after_slow_prefix()
{
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_request_read_timeout = Duration::from_millis(200);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-sticky-live-key").await;
    let default_id =
        insert_test_pool_api_key_account(&state, "Default Slow Live Route", "route-default").await;
    let sticky_id =
        insert_test_pool_api_key_account(&state, "Sticky Slow Live Route", "route-sticky").await;

    let resolved_without_sticky =
        match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
            .await
            .expect("resolve default slow live route")
        {
            PoolAccountResolution::Resolved(account) => account.account_id,
            other => panic!("expected resolved default route, got {other:?}"),
        };
    let preferred_sticky_account_id = if resolved_without_sticky == default_id {
        sticky_id
    } else {
        default_id
    };
    let expected_authorization = if preferred_sticky_account_id == default_id {
        "Bearer route-default"
    } else {
        "Bearer route-sticky"
    };
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-live-slow-body-route",
        preferred_sticky_account_id,
        &format_test_recent_active_timestamp(Utc::now()),
    )
    .await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(
                br#"{"model":"gpt-5","messages":[{"role":"user","content":"hello"}],"#,
            )))
            .await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(
                br#""stickyKey":"sticky-live-slow-body-route","temperature":0}"#,
            )))
            .await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6248,
        &"/v1/chat/completions".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-sticky-live-key"),
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
    .expect("slow live first attempt should still honor body sticky route");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read slow sticky live response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode slow sticky live response");
    assert_eq!(payload["authorization"], expected_authorization);
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock slow live sticky attempts");
    assert_eq!(attempts.get(expected_authorization).copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_via_pool_large_prebuffered_body_does_not_reread_for_late_sticky_route() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-large-sticky-key").await;
    let default_id =
        insert_test_pool_api_key_account(&state, "Default Large Route", "route-large-default")
            .await;
    let sticky_id =
        insert_test_pool_api_key_account(&state, "Sticky Large Route", "route-large-sticky").await;

    let resolved_without_sticky =
        match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
            .await
            .expect("resolve default route")
        {
            PoolAccountResolution::Resolved(account) => account.account_id,
            other => panic!("expected resolved default route, got {other:?}"),
        };
    let preferred_sticky_account_id = if resolved_without_sticky == default_id {
        sticky_id
    } else {
        default_id
    };
    let expected_default_authorization = if resolved_without_sticky == default_id {
        "Bearer route-large-default"
    } else {
        "Bearer route-large-sticky"
    };
    let unexpected_late_sticky_authorization = if preferred_sticky_account_id == default_id {
        "Bearer route-large-default"
    } else {
        "Bearer route-large-sticky"
    };
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-large-late-route",
        preferred_sticky_account_id,
        &format_test_recent_active_timestamp(Utc::now()),
    )
    .await;

    let request_body = format!(
        r#"{{"model":"gpt-5","input":"{}","stickyKey":"sticky-large-late-route"}}"#,
        "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 128)
    );
    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6245,
        &"/v1/responses".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-large-sticky-key"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(request_body.into_bytes()),
        runtime_timeouts,
        None,
    )
    .await
    .expect("large prebuffered request should fall back without rereading the full file");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read large sticky response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode large sticky response");
    assert_eq!(
        payload["authorization"].as_str(),
        Some(expected_default_authorization)
    );
    assert_ne!(
        payload["authorization"].as_str(),
        Some(unexpected_late_sticky_authorization)
    );
    assert!(
        payload["attempt"]
            .as_i64()
            .is_some_and(|attempt| attempt >= 1),
        "expected at least one upstream attempt"
    );

    let attempts = attempts.lock().expect("lock large sticky attempts");
    assert!(
        attempts
            .get(expected_default_authorization)
            .copied()
            .is_some_and(|count| count >= 1),
        "default route should receive at least one attempt"
    );
    assert_eq!(
        attempts
            .get(unexpected_late_sticky_authorization)
            .copied()
            .unwrap_or_default(),
        0
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_via_pool_live_retry_ignores_late_file_backed_sticky_route() {
    let request_body = format!(
        r#"{{"model":"gpt-5","input":"{}","stickyKey":"sticky-large-late-route"}}"#,
        "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 128)
    );
    let first_chunk_end = request_body
        .find(r#"","stickyKey":"#)
        .expect("large request should contain late sticky key marker");
    let first_chunk = request_body[..first_chunk_end].as_bytes().to_vec();
    let second_chunk = request_body[first_chunk_end..].as_bytes().to_vec();

    let probe_state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:1").expect("valid dummy url"))
            .await;
    seed_pool_routing_api_key(&probe_state, "pool-large-live-retry-key").await;
    let probe_default_id = insert_test_pool_api_key_account(
        &probe_state,
        "Default Large Retry Route",
        "route-large-retry-default",
    )
    .await;
    let probe_sticky_id = insert_test_pool_api_key_account(
        &probe_state,
        "Sticky Large Retry Route",
        "route-large-retry-sticky",
    )
    .await;

    let resolved_without_sticky =
        match resolve_pool_account_for_request(probe_state.as_ref(), None, &[], &HashSet::new())
            .await
            .expect("resolve default retry route")
        {
            PoolAccountResolution::Resolved(account) => account.account_id,
            other => panic!("expected resolved default retry route, got {other:?}"),
        };
    let expected_default_authorization = if resolved_without_sticky == probe_default_id {
        "Bearer route-large-retry-default"
    } else {
        "Bearer route-large-retry-sticky"
    };
    let _ = probe_sticky_id;
    drop(probe_state);

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_retry_upstream_after_body_read(&[(expected_default_authorization, 1)]).await;

    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-large-live-retry-key").await;
    let default_id = insert_test_pool_api_key_account(
        &state,
        "Default Large Retry Route",
        "route-large-retry-default",
    )
    .await;
    let sticky_id = insert_test_pool_api_key_account(
        &state,
        "Sticky Large Retry Route",
        "route-large-retry-sticky",
    )
    .await;

    let resolved_without_sticky =
        match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
            .await
            .expect("resolve default retry route")
        {
            PoolAccountResolution::Resolved(account) => account.account_id,
            other => panic!("expected resolved default retry route, got {other:?}"),
        };
    let preferred_sticky_account_id = if resolved_without_sticky == default_id {
        sticky_id
    } else {
        default_id
    };
    let expected_default_authorization = if resolved_without_sticky == default_id {
        "Bearer route-large-retry-default"
    } else {
        "Bearer route-large-retry-sticky"
    };
    let unexpected_late_sticky_authorization = if preferred_sticky_account_id == default_id {
        "Bearer route-large-retry-default"
    } else {
        "Bearer route-large-retry-sticky"
    };
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-large-late-route",
        preferred_sticky_account_id,
        &format_test_recent_active_timestamp(Utc::now()),
    )
    .await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        let _ = tx.send(Ok(Bytes::from(second_chunk))).await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        6247,
        &"/v1/responses".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-large-live-retry-key"),
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
    .expect("live retry should ignore late sticky key from file-backed replay");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read large live retry response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode large live retry response");
    assert_eq!(
        payload["authorization"].as_str(),
        Some(expected_default_authorization)
    );
    assert_ne!(
        payload["authorization"].as_str(),
        Some(unexpected_late_sticky_authorization)
    );
    assert!(
        payload["attempt"]
            .as_i64()
            .is_some_and(|attempt| attempt >= 2),
        "expected the default route to be retried after the first live attempt failed"
    );

    let attempts = attempts.lock().expect("lock large live retry attempts");
    assert!(
        attempts
            .get(expected_default_authorization)
            .copied()
            .is_some_and(|count| count >= 2),
        "default route should receive the retry attempts"
    );
    assert_eq!(
        attempts
            .get(unexpected_late_sticky_authorization)
            .copied()
            .unwrap_or_default(),
        0
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn continue_or_retry_pool_live_request_respects_total_timeout_while_waiting_for_replay() {
    let mut config = test_config();
    config.pool_upstream_responses_total_timeout = Duration::from_millis(40);
    let state = test_state_from_config(config, true).await;
    let original_uri = "/v1/responses".parse::<Uri>().expect("valid responses uri");
    let account = PoolResolvedAccount {
        account_id: 91,
        display_name: "Live Retry Timeout".to_string(),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer live-retry-timeout".to_string(),
        },
        upstream_base_url: Url::parse("https://api.openai.com/").expect("valid upstream base url"),
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
    let (_status_tx, status_rx) = tokio::sync::watch::channel(PoolReplayBodyStatus::Reading);
    let err = continue_or_retry_pool_live_request(
        state,
        7001,
        Method::POST,
        &original_uri,
        &HeaderMap::new(),
        Duration::from_millis(20),
        account,
        None,
        Some(Instant::now() - Duration::from_millis(50)),
        None,
        &status_rx,
        &CancellationToken::new(),
        Duration::from_millis(40),
        PoolUpstreamError {
            account: None,
            status: StatusCode::BAD_GATEWAY,
            message: "upstream stream error before first chunk".to_string(),
            canonical_error_message: None,
            failure_kind: PROXY_FAILURE_UPSTREAM_STREAM_ERROR,
            connect_latency_ms: 0.0,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: None,
            oauth_responses_debug: None,
            attempt_summary: pool_attempt_summary(1, 1, None),
            requested_service_tier: None,
            request_body_for_capture: None,
        },
    )
    .await
    .expect_err("waiting for replay past total timeout should fail");

    assert_eq!(err.status, StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        err.message,
        "pool upstream total timeout exhausted after 40ms"
    );
    assert_eq!(err.failure_kind, PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED);
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
    let started = Instant::now();
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
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(180),
        "body timeout should still fire first, elapsed={elapsed:?}"
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
            name, guard_enabled, lookback_hours, max_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, NULL, NULL, 0, 1, ?2, ?2)
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
            name, guard_enabled, lookback_hours, max_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, NULL, NULL, 0, 1, ?2, ?2)
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
        tokio::time::sleep(Duration::from_millis(220)).await;
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
        elapsed < Duration::from_millis(180),
        "same sticky value should fail before the rest of the streamed body finishes, elapsed={elapsed:?}"
    );
    let (status, message) = response.expect_err("via-pool request should fail");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        message.contains("upstream account is not assigned to a group"),
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
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
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

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
        elapsed < Duration::from_millis(150),
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
    assert!(
        elapsed < Duration::from_millis(160),
        "responses total timeout should include body upload plus no-account wait, elapsed={elapsed:?}"
    );
    assert_eq!(
        message,
        pool_total_timeout_exhausted_message(Duration::from_millis(90)),
        "unexpected via-pool failure: {message}"
    );
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);
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
        elapsed >= Duration::from_millis(60),
        "request should still spend the configured total-timeout budget before failing, elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(160),
        "responses total timeout should short-circuit before streamed body buffering completes, elapsed={elapsed:?}"
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
        spawn_pool_delayed_headers_upstream(Duration::from_millis(50)).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_total_timeout = Duration::from_millis(90);
    let state = test_state_from_config_with_pool_no_available_wait(
        config,
        true,
        PoolNoAvailableWaitSettings {
            timeout: Duration::from_millis(200),
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

    let pool = state.pool.clone();
    let delayed_release_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(70)).await;
        set_test_account_status(&pool, delayed_id, "active").await;
    });

    // Full-suite timer contention can stretch this path well beyond the nominal 90ms budget,
    // so keep the assertion focused on bounded completion plus the timeout-shaped response.
    let response = tokio::time::timeout(
        Duration::from_secs(3),
        proxy_openai_v1(
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
        ),
    )
    .await
    .expect("remaining total-timeout request should not hang");
    delayed_release_task
        .await
        .expect("delayed release task should join");

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(pool_total_timeout_exhausted_message(Duration::from_millis(90)).as_str())
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
        tokio::time::sleep(Duration::from_millis(20)).await;
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
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

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
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-rate-limited").copied(), None);
    assert_eq!(
        attempts.get("Bearer upstream-replacement").copied(),
        Some(1)
    );

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
            name, guard_enabled, lookback_hours, max_conversations,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, NULL, NULL, 0, 1, ?2, ?2)
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
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

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
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(170)).await;
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
        tokio::time::sleep(Duration::from_millis(280)).await;
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
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 0);

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
        Duration::from_millis(120),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let blocked_id = insert_test_pool_api_key_account(&state, "Blocked", "upstream-blocked").await;
    let delayed_id = insert_test_pool_api_key_account(&state, "Delayed", "upstream-delayed").await;
    set_test_account_status(&state.pool, blocked_id, "needs_reauth").await;
    set_test_account_status(&state.pool, delayed_id, "needs_reauth").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    tokio::spawn(async move {
        let _ = tx
            .send(Ok(Bytes::from_static(b"{\"model\":\"gpt-5\",")))
            .await;
        tokio::time::sleep(Duration::from_millis(90)).await;
        let _ = tx
            .send(Ok(Bytes::from_static(
                b"\"messages\":[],\"stickyKey\":\"body-reroute-sticky\"}",
            )))
            .await;
    });

    let pool = state.pool.clone();
    let delayed_release_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(170)).await;
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
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx)),
        runtime_timeouts,
        None,
    )
    .await;
    let elapsed = started.elapsed();

    delayed_release_task
        .await
        .expect("delayed release task should join");

    assert!(
        elapsed < Duration::from_millis(160),
        "rerouted sticky requests should not reset the bounded wait window, elapsed={elapsed:?}"
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
        Duration::from_millis(50),
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
