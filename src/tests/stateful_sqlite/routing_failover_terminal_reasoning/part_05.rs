#[test]
fn capture_target_pool_route_timeout_uses_ungrouped_api_key_transit_as_alternate() {
    run_routing_future_with_large_stack(async move {
        let (shared_upstream_base, shared_upstream_handle) =
            spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
        let (alternate_upstream_base, alternate_attempts, alternate_upstream_handle) =
            spawn_pool_retry_upstream(&[("Bearer route-ungrouped-alt", 0)]).await;
        let mut config = test_config();
        config.openai_upstream_base_url =
            Url::parse("https://api.openai.com/").expect("valid upstream base url");
        config.pool_upstream_responses_attempt_timeout = Duration::from_millis(120);
        let state = test_state_from_config(config, true).await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        insert_test_pool_api_key_account_with_options(
            &state,
            "Shared Route A",
            "route-shared-a-blocked-alt",
            None,
            None,
            Some(shared_upstream_base.as_str()),
        )
        .await;
        let ungrouped_id = insert_test_pool_api_key_account_with_options(
            &state,
            "Ungrouped Alternate",
            "route-ungrouped-alt",
            None,
            None,
            Some(alternate_upstream_base.as_str()),
        )
        .await;
        sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
            .bind(ungrouped_id)
            .execute(&state.pool)
            .await
            .expect("clear ungrouped alternate group");

        let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-blocked-policy-no-alt"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
        assert_transit_timeout_alternate(&state, &alternate_attempts, response).await;

        shared_upstream_handle.abort();
        alternate_upstream_handle.abort();
    });
}

#[derive(Debug, sqlx::FromRow)]
struct TimeoutAttemptRouteRow {
    attempt_index: i64,
    same_account_retry_index: i64,
    status: String,
}

#[derive(Debug, sqlx::FromRow)]
struct TimeoutPayloadRow {
    error_message: Option<String>,
    payload: Option<String>,
}

async fn assert_transit_timeout_alternate(
    state: &Arc<AppState>,
    alternate_attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    response: axum::response::Response,
) {
    let response_status = response.status();
    assert!(response.headers().get(http_header::RETRY_AFTER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout transit alternate response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode timeout transit alternate response body");
    assert_eq!(response_status, StatusCode::OK, "{response_payload}");
    assert_eq!(response_payload["ok"].as_bool(), Some(true));
    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 2).await;
    let attempt_rows = sqlx::query_as::<_, TimeoutAttemptRouteRow>(
        "SELECT attempt_index, same_account_retry_index, status FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(&state.pool).await.expect("load timeout transit alternate rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );
    let row = sqlx::query_as::<_, TimeoutPayloadRow>(
        "SELECT error_message, payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load timeout transit alternate payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout transit alternate payload should be present"),
    )
    .expect("decode timeout transit alternate payload");
    assert!(row.error_message.is_none());
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(2));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
    assert!(payload["poolAttemptTerminalReason"].is_null());
    assert_eq!(
        alternate_attempts
            .lock()
            .expect("lock alternate attempts")
            .get("Bearer route-ungrouped-alt")
            .copied(),
        Some(1)
    );
}

#[tokio::test]
async fn capture_target_pool_route_timeout_ignores_broken_same_route_groups() {
    let (shared_upstream_base, shared_upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(120);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Shared Route A",
        "route-shared-a-invalid-group",
        None,
        None,
        Some(shared_upstream_base.as_str()),
    )
    .await;
    let broken_same_route_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Broken Shared Route",
        "route-shared-b-invalid-group",
        None,
        None,
        Some(shared_upstream_base.as_str()),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(broken_same_route_id)
        .bind("broken-shared-route-group")
        .execute(&state.pool)
        .await
        .expect("mark broken same-route account");
    let exhausted_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Exhausted Other Route",
        "route-exhausted-invalid-group",
        None,
        None,
        Some("https://exhausted.example.com/backend-api/codex"),
    )
    .await;
    insert_test_pool_limit_sample(&state, exhausted_id, Some(100.0), Some(0.0)).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-no-alt-invalid-group"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_no_alternate_timeout(&state, response).await;

    shared_upstream_handle.abort();
}

async fn assert_no_alternate_timeout(state: &Arc<AppState>, response: axum::response::Response) {
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout no-alternate response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode timeout no-alternate response body");
    assert!(response_payload["error"].as_str().is_some_and(|message| {
        message.contains("no alternate upstream route is available after timeout")
    }));
    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 2).await;
    let attempt_rows = sqlx::query_as::<_, TimeoutAttemptRouteRow>(
        "SELECT attempt_index, same_account_retry_index, status FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    ).fetch_all(&state.pool).await.expect("load timeout no-alternate rows");
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    let row = sqlx::query_as::<_, TimeoutPayloadRow>(
        "SELECT error_message, payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load timeout no-alternate payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout payload should be present"),
    )
    .expect("decode timeout payload");
    assert!(row.error_message.as_deref().is_some_and(|message| {
        message.contains("no alternate upstream route is available after timeout")
    }));
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT)
    );
    assert!(payload["upstreamErrorMessage"].is_null());
}
