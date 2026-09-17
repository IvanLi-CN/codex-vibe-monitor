#[derive(Debug, sqlx::FromRow)]
struct TimeoutAttemptRouteRow {
    upstream_route_key: Option<String>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: String,
    failure_kind: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct TimeoutPayloadRow {
    error_message: Option<String>,
    payload: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct StickyTimeoutAttemptRow {
    upstream_account_id: Option<i64>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: String,
    failure_kind: Option<String>,
}

async fn load_sticky_timeout_attempt_rows(pool: &SqlitePool) -> Vec<StickyTimeoutAttemptRow> {
    sqlx::query_as::<_, StickyTimeoutAttemptRow>(
        r#"SELECT upstream_account_id, attempt_index, distinct_account_index,
                  same_account_retry_index, status, failure_kind
           FROM pool_upstream_request_attempts ORDER BY attempt_index ASC"#,
    )
    .fetch_all(pool)
    .await
    .expect("load sticky timeout attempt rows")
}

async fn load_timeout_attempt_rows(pool: &SqlitePool) -> Vec<TimeoutAttemptRouteRow> {
    sqlx::query_as::<_, TimeoutAttemptRouteRow>(
        r#"SELECT upstream_route_key, attempt_index, distinct_account_index,
                  same_account_retry_index, status, failure_kind
           FROM pool_upstream_request_attempts ORDER BY attempt_index ASC"#,
    )
    .fetch_all(pool)
    .await
    .expect("load timeout attempt rows")
}

async fn load_timeout_payload(pool: &SqlitePool) -> (TimeoutPayloadRow, Value) {
    let row = sqlx::query_as::<_, TimeoutPayloadRow>(
        "SELECT error_message, payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load timeout invocation payload");
    let payload = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout invocation payload should be present"),
    )
    .expect("decode timeout invocation payload");
    (row, payload)
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_timeout_does_not_cut_out_from_no_cut_out_sticky_owner()
 {
    let (slow_upstream_base, slow_upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(750)).await;
    let (fast_upstream_base, attempts, fast_upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-fast-no-cut-out", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(250);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let slow_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Slow No Cut Out Route",
        "route-slow-no-cut-out",
        None,
        None,
        Some(slow_upstream_base.as_str()),
    )
    .await;
    let fast_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fast Route Behind No Cut Out",
        "route-fast-no-cut-out",
        None,
        None,
        Some(fast_upstream_base.as_str()),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_allow_cut_out = 0 WHERE id = ?1")
        .bind(slow_id)
        .execute(&state.pool)
        .await
        .expect("set source no cut-out policy");

    let sticky_key = "sticky-timeout-no-cut-out-owner";
    let sticky_seen_at = format_test_recent_active_timestamp(Utc::now());
    upsert_test_sticky_route_at(&state.pool, sticky_key, slow_id, &sticky_seen_at).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            format!(r#"{{"model":"gpt-5","input":"hello","stickyKey":"{sticky_key}"}}"#)
                .into_bytes(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read no-cut-out timeout failure body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode no-cut-out timeout failure body");
    assert!(
        response_payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("upstream")),
        "unexpected no-cut-out timeout payload: {response_payload}",
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 1).await;

    let attempt_rows = load_sticky_timeout_attempt_rows(&state.pool).await;
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(attempt_rows[0].upstream_account_id, Some(slow_id));
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    let attempts = attempts.lock().expect("lock fast attempts");
    assert_eq!(attempts.get("Bearer route-fast-no-cut-out").copied(), None);
    drop(attempts);
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, sticky_key).await,
        Some(slow_id)
    );
    assert_ne!(
        load_test_sticky_route_account_id(&state.pool, sticky_key).await,
        Some(fast_id)
    );

    slow_upstream_handle.abort();
    fast_upstream_handle.abort();
}

#[test]
pub(crate) fn canonical_pool_upstream_route_key_collapses_trailing_slashes() {
    let without_trailing_slash =
        Url::parse("https://route.example/base?foo=bar").expect("valid route url");
    let with_trailing_slash =
        Url::parse("https://route.example/base/?baz=qux#frag").expect("valid route url");

    assert_eq!(
        canonical_pool_upstream_route_key(&without_trailing_slash),
        canonical_pool_upstream_route_key(&with_trailing_slash),
    );
    assert_eq!(
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example/?foo=bar").expect("valid root route url")
        ),
        "https://route.example/",
    );
    assert_eq!(
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example:443/base").expect("valid default https port route")
        ),
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example/base").expect("valid https route")
        ),
    );
    assert_eq!(
        canonical_pool_upstream_route_key(
            &Url::parse("http://route.example:80/base").expect("valid default http port route")
        ),
        canonical_pool_upstream_route_key(
            &Url::parse("http://route.example/base").expect("valid http route")
        ),
    );
    assert_ne!(
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example:8443/base")
                .expect("valid non-default https port route")
        ),
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example/base").expect("valid https route")
        ),
    );
}

#[test]
pub(crate) fn pool_failure_is_timeout_shaped_ignores_upstream_5xx_text_timeouts() {
    assert!(!pool_failure_is_timeout_shaped(
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        "pool upstream responded with 500: operation timed out after 30s"
    ));
    assert!(pool_failure_is_timeout_shaped(
        PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
        "upstream handshake timed out after 60000ms"
    ));
    assert!(pool_failure_is_timeout_shaped(
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        "request timed out after 120000ms while waiting for first upstream chunk"
    ));
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_timeout_returns_no_alternate_when_only_same_route_remains()
 {
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
        "route-shared-a",
        None,
        None,
        Some(shared_upstream_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Shared Route B",
        "route-shared-b",
        None,
        None,
        Some(shared_upstream_base.as_str()),
    )
    .await;
    let exhausted_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Exhausted Other Route",
        "route-exhausted",
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
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-no-alt-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout no-alternate response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode timeout no-alternate response body");
    assert!(
        response_payload["error"]
            .as_str()
            .expect("timeout no-alternate error should be present")
            .contains("no alternate upstream route is available after timeout")
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 1).await;

    let attempt_rows = load_timeout_attempt_rows(&state.pool).await;
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    let (row, payload) = load_timeout_payload(&state.pool).await;
    assert!(
        row.error_message.as_deref().is_some_and(
            |msg| msg.contains("no alternate upstream route is available after timeout")
        )
    );
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT),
    );
    assert!(payload["upstreamErrorMessage"].is_null());

    shared_upstream_handle.abort();
}

#[test]
pub(crate) fn capture_target_pool_route_timeout_uses_ungrouped_api_key_transit_as_alternate() {
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

        let attempt_rows = load_timeout_attempt_rows(&state.pool).await;
        assert_eq!(attempt_rows.len(), 2);
        assert_eq!(attempt_rows[0].attempt_index, 1);
        assert_eq!(attempt_rows[0].same_account_retry_index, 1);
        assert_eq!(
            attempt_rows[0].status,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        );
        assert_eq!(attempt_rows[1].attempt_index, 2);
        assert_eq!(
            attempt_rows[1].status,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        );

        let (row, payload) = load_timeout_payload(&state.pool).await;
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

        shared_upstream_handle.abort();
        alternate_upstream_handle.abort();
    });
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_timeout_ignores_broken_same_route_groups() {
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
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout no-alternate invalid-group response body");
    let response_payload: Value = serde_json::from_slice(&body)
        .expect("decode timeout no-alternate invalid-group response body");
    assert!(
        response_payload["error"]
            .as_str()
            .expect("timeout no-alternate invalid-group error should be present")
            .contains("no alternate upstream route is available after timeout")
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 2).await;

    let attempt_rows = load_timeout_attempt_rows(&state.pool).await;
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    let (row, payload) = load_timeout_payload(&state.pool).await;
    assert!(
        row.error_message.as_deref().is_some_and(
            |msg| msg.contains("no alternate upstream route is available after timeout")
        )
    );
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT),
    );
    assert!(payload["upstreamErrorMessage"].is_null());

    shared_upstream_handle.abort();
}

use super::*;
