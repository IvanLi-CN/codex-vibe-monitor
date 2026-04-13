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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
}

#[tokio::test]
async fn proxy_openai_v1_chunked_json_without_header_sticky_uses_live_first_attempt() {
    let mut config = test_config();
    config.openai_proxy_request_read_timeout = Duration::from_millis(80);
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
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(16);
    let first_chunk = format!(
        "{{\"model\":\"gpt-5\",\"input\":\"{}",
        "x".repeat(HEADER_STICKY_EARLY_STICKY_SCAN_BYTES + 256)
    );
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from(first_chunk))).await;
        tokio::time::sleep(Duration::from_millis(120)).await;
        let _ = tx.send(Ok(Bytes::from_static(b"\"}"))).await;
    });

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let started = Instant::now();
    let response = proxy_openai_v1_via_pool(
        state.clone(),
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
    .expect("chunked via-pool request should succeed via live first attempt");
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_millis(120),
        "live first attempt should not wait for the entire chunked body, elapsed={elapsed:?}"
    );
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
        latest_attempt.2,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        "successful live-first requests should land as real success attempts",
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
}

#[tokio::test]
async fn proxy_openai_v1_header_sticky_recovers_after_wait_starts() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&upstream_base).expect("valid upstream base url");
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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
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
        .await
        .expect("delayed release task should join");

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(
        elapsed < Duration::from_millis(260),
        "late account recovery should still terminate near the original total-timeout window, elapsed={elapsed:?}"
    );
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
    wait_for_pool_upstream_request_attempts(&state.pool, 1).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);
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
        body_reroute_rx.await.expect("body reroute signal should arrive");
        tokio::time::sleep(Duration::from_millis(80)).await;
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
        elapsed < Duration::from_millis(300),
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
