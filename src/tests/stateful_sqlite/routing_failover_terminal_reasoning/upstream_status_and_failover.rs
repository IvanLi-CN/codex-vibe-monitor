use super::*;

#[tokio::test]
async fn pool_route_retries_upstream_413_once_on_same_account_then_succeeds() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        http_status: Option<i64>,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_sequential_failure_responses_upstream(vec![(
            "Bearer upstream-primary",
            vec![StatusCode::PAYLOAD_TOO_LARGE],
        )])
        .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-413-same-success"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read upstream 413 retry success body");
    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(2));
    drop(attempts);
    wait_for_pool_attempt_row_count(&state.pool, 2).await;
    wait_for_pool_attempt_status(&state.pool, 2, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
        .await;

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT distinct_account_index, same_account_retry_index, status, http_status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load upstream 413 same-account retry attempts");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[0].http_status, Some(413));
    assert_eq!(
        attempt_rows[0].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_413)
    );
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );

    upstream_handle.abort();
}

#[test]
fn priority_handoff_does_not_retry_upstream_413() {
    let _priority_handoff_guard = crate::upstream_accounts::priority_handoff_test_guard();
    run_routing_future_with_large_stack(async {
        let (upstream_base, attempts, upstream_handle) =
            spawn_pool_sequential_failure_responses_upstream(vec![
                (
                    "Bearer priority-handoff-target",
                    vec![StatusCode::PAYLOAD_TOO_LARGE],
                ),
                ("Bearer priority-handoff-source", vec![]),
            ])
            .await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let source_id = insert_test_pool_api_key_account(
            &state,
            "Priority Handoff Source",
            "priority-handoff-source",
        )
        .await;
        let target_id = insert_test_pool_api_key_account(
            &state,
            "Priority Handoff Target",
            "priority-handoff-target",
        )
        .await;
        sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
            .bind(source_id)
            .bind(TagPriorityTier::Fallback.as_str())
            .execute(&state.pool)
            .await
            .expect("set fallback handoff source priority");
        sqlx::query("UPDATE pool_upstream_accounts SET policy_priority_tier = ?2 WHERE id = ?1")
            .bind(target_id)
            .bind(TagPriorityTier::Primary.as_str())
            .execute(&state.pool)
            .await
            .expect("set primary handoff target priority");
        let sticky_key = "priority-handoff-413";
        upsert_test_sticky_route_at(
            &state.pool,
            sticky_key,
            source_id,
            &format_utc_iso(Utc::now()),
        )
        .await;

        let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            br#"{"model":"gpt-priority-handoff-413","input":"hello","stickyKey":"priority-handoff-413"}"#
                .to_vec(),
        ),
    )
    .await;

        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let _body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read priority handoff 413 body");
        wait_for_pool_attempt_row_count(&state.pool, 1).await;
        {
            let attempts = attempts.lock().expect("lock attempts");
            assert_eq!(
                attempts.get("Bearer priority-handoff-target").copied(),
                Some(1)
            );
            assert_eq!(
                attempts.get("Bearer priority-handoff-source").copied(),
                None
            );
        }
        assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 1);

        upstream_handle.abort();
    });
}

#[tokio::test]
async fn pool_route_switches_account_after_upstream_413_retry_fails() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        http_status: Option<i64>,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_sequential_failure_responses_upstream(vec![
            (
                "Bearer upstream-primary",
                vec![StatusCode::PAYLOAD_TOO_LARGE, StatusCode::PAYLOAD_TOO_LARGE],
            ),
            ("Bearer upstream-secondary", vec![]),
        ])
        .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-413-switch-success"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read upstream 413 switch success body");
    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(2));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);
    wait_for_pool_attempt_row_count(&state.pool, 3).await;
    wait_for_pool_attempt_status(&state.pool, 3, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
        .await;

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT distinct_account_index, same_account_retry_index, status, http_status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load upstream 413 switch attempts");
    assert_eq!(attempt_rows.len(), 3);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[0].http_status, Some(413));
    assert_eq!(
        attempt_rows[0].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_413)
    );
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert_eq!(attempt_rows[1].http_status, Some(413));
    assert_eq!(
        attempt_rows[1].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_413)
    );
    assert_eq!(attempt_rows[2].distinct_account_index, 2);
    assert_eq!(attempt_rows[2].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[2].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_retries_first_upstream_413_after_prior_5xx_same_account() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        same_account_retry_index: i64,
        status: String,
        http_status: Option<i64>,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_sequential_failure_responses_upstream(vec![(
            "Bearer upstream-primary",
            vec![
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::PAYLOAD_TOO_LARGE,
            ],
        )])
        .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-413-after-5xx"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read upstream mixed 5xx and 413 retry success body");
    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    drop(attempts);
    wait_for_pool_attempt_row_count(&state.pool, 3).await;
    wait_for_pool_attempt_status(&state.pool, 3, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
        .await;

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT same_account_retry_index, status, http_status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load upstream mixed 5xx and 413 attempts");
    assert_eq!(attempt_rows.len(), 3);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[0].http_status, Some(500));
    assert_eq!(
        attempt_rows[0].failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    );
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert_eq!(attempt_rows[1].http_status, Some(413));
    assert_eq!(
        attempt_rows[1].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_413)
    );
    assert_eq!(attempt_rows[2].same_account_retry_index, 3);
    assert_eq!(
        attempt_rows[2].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_retries_first_upstream_413_at_end_of_same_account_budget() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        same_account_retry_index: i64,
        status: String,
        http_status: Option<i64>,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_sequential_failure_responses_upstream(vec![(
            "Bearer upstream-primary",
            vec![
                StatusCode::INTERNAL_SERVER_ERROR,
                StatusCode::BAD_GATEWAY,
                StatusCode::PAYLOAD_TOO_LARGE,
            ],
        )])
        .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-413-budget-tail"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read upstream 413 budget-tail retry success body");
    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(4));
    drop(attempts);
    wait_for_pool_attempt_row_count(&state.pool, 4).await;
    wait_for_pool_attempt_status(&state.pool, 4, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
        .await;

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT same_account_retry_index, status, http_status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load upstream 413 budget-tail attempts");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[0].http_status, Some(500));
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert_eq!(attempt_rows[1].http_status, Some(502));
    assert_eq!(attempt_rows[2].same_account_retry_index, 3);
    assert_eq!(attempt_rows[2].http_status, Some(413));
    assert_eq!(
        attempt_rows[2].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_413)
    );
    assert_eq!(attempt_rows[3].same_account_retry_index, 4);
    assert_eq!(
        attempt_rows[3].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_returns_413_when_single_account_upstream_413_retry_fails() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_sequential_failure_responses_upstream(vec![(
            "Bearer upstream-primary",
            vec![StatusCode::PAYLOAD_TOO_LARGE, StatusCode::PAYLOAD_TOO_LARGE],
        )])
        .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-413-single-final"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read upstream 413 body");
    let payload: Value = serde_json::from_slice(&body).expect("decode upstream 413 payload");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("pool upstream responded with 413")),
        "unexpected upstream 413 payload: {payload}"
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(2));
    drop(attempts);

    let failure_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE http_status = 413 AND failure_kind = ?1
        "#,
    )
    .bind(PROXY_FAILURE_UPSTREAM_HTTP_413)
    .fetch_one(&state.pool)
    .await
    .expect("count upstream 413 attempts");
    assert_eq!(failure_count, 2);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_preserves_413_when_distinct_account_budget_exhausts() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        http_status: Option<i64>,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_sequential_failure_responses_upstream(vec![
            (
                "Bearer upstream-primary",
                vec![StatusCode::PAYLOAD_TOO_LARGE, StatusCode::PAYLOAD_TOO_LARGE],
            ),
            (
                "Bearer upstream-secondary",
                vec![StatusCode::PAYLOAD_TOO_LARGE, StatusCode::PAYLOAD_TOO_LARGE],
            ),
            (
                "Bearer upstream-tertiary",
                vec![StatusCode::PAYLOAD_TOO_LARGE, StatusCode::PAYLOAD_TOO_LARGE],
            ),
        ])
        .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    insert_test_pool_api_key_account(&state, "Tertiary", "upstream-tertiary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-413-budget-final"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(2));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(2));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(2));
    drop(attempts);

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT distinct_account_index, same_account_retry_index, status, http_status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load upstream 413 budget attempts");
    assert_eq!(attempt_rows.len(), 6);
    for row in &attempt_rows {
        assert_eq!(row.http_status, Some(413));
        assert_eq!(
            row.failure_kind.as_deref(),
            Some(PROXY_FAILURE_UPSTREAM_HTTP_413)
        );
    }
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert_eq!(attempt_rows[2].distinct_account_index, 2);
    assert_eq!(attempt_rows[2].same_account_retry_index, 1);
    assert_eq!(attempt_rows[3].distinct_account_index, 2);
    assert_eq!(attempt_rows[3].same_account_retry_index, 2);
    assert_eq!(attempt_rows[4].distinct_account_index, 3);
    assert_eq!(attempt_rows[4].same_account_retry_index, 1);
    assert_eq!(attempt_rows[5].distinct_account_index, 3);
    assert_eq!(attempt_rows[5].same_account_retry_index, 2);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_does_not_use_pool_wide_429_message_when_budget_exhaustion_is_mixed() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_static_failure_responses_upstream(&[
            ("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR),
            (
                "Bearer upstream-secondary",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
            ("Bearer upstream-tertiary", StatusCode::TOO_MANY_REQUESTS),
        ])
        .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    insert_test_pool_api_key_account(&state, "Tertiary", "upstream-tertiary").await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-mixed-budget"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert_ne!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(1));

    upstream_handle.abort();
}
