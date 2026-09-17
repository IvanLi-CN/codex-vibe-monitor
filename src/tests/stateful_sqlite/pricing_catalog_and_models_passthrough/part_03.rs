#[tokio::test]
async fn proxy_capture_persist_and_broadcast_emits_records_and_dashboard_live() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;
    let _dashboard_lease = state
        .subscription_hub
        .register_test_topic_name("dashboard.activity.current")
        .await;
    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "proxy-sse-broadcast-success";
    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &now_local),
    )
    .await
    .expect("persist+broadcast should succeed");

    let mut saw_record = false;
    let mut captured_record: Option<ApiInvocation> = None;
    let mut saw_dashboard_live = false;
    for _ in 0..16 {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for proxy broadcast event")
            .expect("broadcast channel should stay open");
        match payload {
            BroadcastPayload::Records { records } => {
                if let Some(record) = records
                    .into_iter()
                    .find(|record| record.invoke_id == invoke_id)
                {
                    saw_record = true;
                    captured_record = Some(record);
                }
            }
            BroadcastPayload::Quota { snapshot } => {
                assert_eq!(snapshot.total_requests, 9);
            }
            BroadcastPayload::DashboardActivityLive { .. }
            | BroadcastPayload::DashboardCurrentSlice { .. } => {
                saw_dashboard_live = true;
            }
            BroadcastPayload::Version { .. }
            | BroadcastPayload::PoolAttempts { .. }
            | BroadcastPayload::PoolAttemptsSnapshotUnavailable { .. }
            | BroadcastPayload::DashboardNetworkSlice { .. }
            | BroadcastPayload::DashboardTerminalSlice { .. }
            | BroadcastPayload::PromptCacheConversationChanged { .. }
            | BroadcastPayload::PromptCacheConversationStickyRouteChanged { .. } => {}
        }

        if saw_record && saw_dashboard_live {
            break;
        }
    }

    assert!(saw_record, "records payload should be broadcast");
    assert!(
        saw_dashboard_live,
        "live dashboard snapshot should be scheduled after the record mutation"
    );
    let record = captured_record.expect("target records payload should include invoke id");
    assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(record.requester_ip.as_deref(), Some("198.51.100.77"));
    assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-broadcast-1"));
    assert_eq!(record.route_mode.as_deref(), Some("pool"));
    assert_eq!(record.upstream_account_id, Some(17));
    assert_eq!(
        record.upstream_account_name.as_deref(),
        Some("pool-account-17")
    );
    assert_eq!(
        record.response_content_encoding.as_deref(),
        Some("gzip, br")
    );
    assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-01"));
    assert_eq!(record.requested_service_tier.as_deref(), Some("priority"));
    assert_eq!(record.reasoning_effort.as_deref(), Some("high"));
    assert!(record.failure_kind.is_none());
}
#[tokio::test]
async fn proxy_capture_persist_and_broadcast_skips_duplicate_records() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let invoke_id = "proxy-sse-broadcast-duplicate";
    let mut rx = state.broadcaster.subscribe();

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &occurred_at),
    )
    .await
    .expect("initial persist+broadcast should succeed");

    drain_broadcast_messages(&mut rx).await;

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &occurred_at),
    )
    .await
    .expect("duplicate persist should not fail");

    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Ok(BroadcastPayload::Records { records })) => {
                assert!(
                    records.iter().all(|record| record.invoke_id != invoke_id),
                    "duplicate insert should not emit records payload for the same invoke_id"
                );
            }
            Ok(Ok(_)) => continue,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => continue,
        }
    }
}

#[tokio::test]
async fn proxy_capture_persist_and_broadcast_skips_follow_up_without_subscribers() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;
    let invoke_id = "proxy-sse-follow-up-no-subscribers";

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &now_local),
    )
    .await
    .expect("persist without subscribers should succeed");

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        state
            .proxy_summary_quota_broadcast_seq
            .load(Ordering::Acquire),
        0,
        "no-subscriber path should not enqueue summary/quota follow-up work"
    );
    assert!(
        !state
            .proxy_summary_quota_broadcast_running
            .load(Ordering::Acquire),
        "no-subscriber path should keep the summary/quota worker idle"
    );
}

#[tokio::test]
async fn broadcast_quota_if_changed_skips_duplicate_payloads() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let first = QuotaSnapshotResponse {
        captured_at: "2026-03-07 10:00:00".to_string(),
        amount_limit: Some(100.0),
        used_amount: Some(10.0),
        remaining_amount: Some(90.0),
        period: Some("monthly".to_string()),
        period_reset_time: Some("2026-04-01 00:00:00".to_string()),
        expire_time: None,
        is_active: true,
        total_cost: 10.0,
        total_requests: 9,
        total_tokens: 150,
        last_request_time: Some("2026-03-07 10:00:00".to_string()),
        billing_type: Some("prepaid".to_string()),
        remaining_count: Some(91),
        used_count: Some(9),
        sub_type_name: Some("unit".to_string()),
    };

    assert!(
        broadcast_quota_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            first.clone(),
        )
        .await
        .expect("first quota broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for first quota payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Quota { snapshot } => {
            assert_eq!(*snapshot, first);
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    assert!(
        !broadcast_quota_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            first.clone(),
        )
        .await
        .expect("duplicate quota broadcast should succeed")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err()
    );

    let updated = QuotaSnapshotResponse {
        total_requests: 10,
        ..first
    };
    assert!(
        broadcast_quota_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            updated.clone(),
        )
        .await
        .expect("changed quota broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for updated quota payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Quota { snapshot } => {
            assert_eq!(*snapshot, updated);
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[tokio::test]
async fn capture_targets_reject_non_pool_requests_before_proxying() {
    let state = test_state_with_openai_base(
        Url::parse("https://example.invalid").expect("valid upstream base url"),
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": "hello",
                "stream": false
            }))
            .expect("serialize request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy error body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert_eq!(
        payload["error"].as_str(),
        Some("pool route key missing or invalid")
    );
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn read_request_body_timeout_returns_408() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state = test_state_with_openai_base_body_limit_and_read_timeout(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(50),
    )
    .await;

    let slow_body = stream::unfold(0u8, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<Bytes, Infallible>(Bytes::from_static(br#"{}"#)), 1))
            }
            _ => None,
        }
    });

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(slow_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("request body read timed out")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("failed"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[request_body_read_timeout]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("request_body_read_timeout")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn capture_target_retries_429_and_persists_single_invocation() {
    let (upstream_base, attempts, seen_payloads, upstream_handle) =
        spawn_retrying_capture_upstream(1, Some("0")).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.upstream_429_max_retries = 1;
    }

    let request_payload = json!({
        "model": "gpt-5.2-codex",
        "stream": false,
        "input": "hello",
    });
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(
            serde_json::to_vec(&request_payload).expect("serialize capture retry request body"),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode capture retry payload");
    assert_eq!(payload["attempt"], 2);
    assert_eq!(payload["received"], request_payload);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        seen_payloads
            .lock()
            .expect("lock retrying capture payloads")
            .clone(),
        vec![request_payload.clone(), request_payload.clone()]
    );

    let mut invocation_count: i64 = 0;
    let mut attempt_count: i64 = 0;
    let mut rate_limit_count: i64 = 0;
    for _ in 0..20 {
        invocation_count = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&state.pool)
            .await
            .expect("count persisted invocations");
        attempt_count = count_request_forward_proxy_attempts(&state.pool).await;
        rate_limit_count = count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        )
        .await;

        if invocation_count == 1 && attempt_count == 2 && rate_limit_count == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    assert_eq!(invocation_count, 1);
    assert_eq!(attempt_count, 2);
    assert_eq!(rate_limit_count, 1);

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn capture_target_client_body_disconnect_returns_400_with_failure_kind() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let disconnected_body = stream::iter(vec![Err::<Bytes, io::Error>(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "client disconnected",
    ))]);

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(disconnected_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("failed to read request body stream")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("failed"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[request_body_stream_error_client_closed]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("request_body_stream_error_client_closed")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn capture_target_stream_error_emits_failure_kind_and_persists() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.2-codex",
        "stream": true,
        "input": "hello"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("mid-stream upstream failure should surface to downstream");
    assert!(
        err.to_string().contains("upstream stream error"),
        "unexpected stream error text: {err}"
    );

    let mut row: Option<PersistedRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT status, error_message, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_stream_error]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("upstream_stream_error")
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn capture_target_response_failed_stream_persists_service_failure_details() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": true,
        "input": "hello"
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
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let text = String::from_utf8(body.to_vec()).expect("response body should be utf8");
    assert!(text.contains("response.failed"));

    assert_response_failed_stream_persisted(&state).await;

    upstream_handle.abort();
}

async fn assert_response_failed_stream_persisted(state: &Arc<AppState>) {
    let mut row: Option<PersistedResponseFailedRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedResponseFailedRow>(
            "SELECT status, error_message, payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("capture record should be persisted");
    assert_eq!(count_request_forward_proxy_attempts(&state.pool).await, 1);
    assert_eq!(
        count_request_forward_proxy_attempts_with_failure_kind(
            &state.pool,
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED,
        )
        .await,
        1
    );
    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_response_failed] server_error"))
    );
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload["failureKind"].as_str(),
        Some("upstream_response_failed")
    );
    assert_eq!(
        payload["streamTerminalEvent"].as_str(),
        Some("response.failed")
    );
    assert_eq!(payload["upstreamErrorCode"].as_str(), Some("server_error"));
    assert!(
        payload["upstreamErrorMessage"]
            .as_str()
            .is_some_and(|msg| msg.contains("request ID 060a328d-5cb6-433c-9025-1da2d9c632f1"))
    );
    assert_eq!(
        payload["upstreamRequestId"].as_str(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );
}
