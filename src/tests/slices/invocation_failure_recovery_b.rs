#[tokio::test]
async fn pool_route_existing_sticky_owner_preserves_last_failure_after_cutout_alternate_fails() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptStatusRow {
        status: String,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_static_failure_responses_upstream(&[
            ("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR),
            ("Bearer upstream-secondary", StatusCode::BAD_GATEWAY),
        ])
        .await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(180),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-existing-owner-preserve-after-cutout-failure",
        primary_id,
        &format_utc_iso(Utc::now()),
    )
    .await;
    set_test_account_generic_route_cooldown(&state.pool, primary_id, 120).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-existing-owner-preserve-after-cutout-failure"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        response.headers().get(http_header::RETRY_AFTER).is_none(),
        "cut-out failure preservation should not regress into pool wait semantics"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("pool upstream responded with 502")),
        "unexpected preserved alternate failure payload: {payload}"
    );

    wait_for_pool_attempt_row_count(&state.pool, 5).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 5);

    let attempt_rows = sqlx::query_as::<_, AttemptStatusRow>(
        r#"
        SELECT status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load preserved cut-out alternate attempt rows");
    assert_eq!(attempt_rows.len(), 5);
    assert_eq!(
        attempt_rows[4].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL
    );
    assert_eq!(
        attempt_rows[4].failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

    assert_eq!(
        load_test_sticky_route_account_id(
            &state.pool,
            "sticky-existing-owner-preserve-after-cutout-failure",
        )
        .await,
        Some(primary_id),
        "sticky binding should stay on the original owner when cut-out never succeeds",
    );
    assert_ne!(secondary_id, primary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_existing_sticky_owner_preserves_last_failure_after_distinct_budget_exhausts() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptStatusRow {
        status: String,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_static_failure_responses_upstream(&[
            ("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR),
            ("Bearer upstream-secondary", StatusCode::BAD_GATEWAY),
            ("Bearer upstream-tertiary", StatusCode::BAD_GATEWAY),
        ])
        .await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(180),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    let tertiary_id =
        insert_test_pool_api_key_account(&state, "Tertiary", "upstream-tertiary").await;
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-existing-owner-preserve-after-distinct-budget",
        primary_id,
        &format_utc_iso(Utc::now()),
    )
    .await;
    set_test_account_generic_route_cooldown(&state.pool, primary_id, 120).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-existing-owner-preserve-after-distinct-budget"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(
        response.headers().get(http_header::RETRY_AFTER).is_none(),
        "distinct-budget exhaustion should preserve the last concrete upstream failure"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("pool upstream responded with 502")),
        "unexpected preserved distinct-budget failure payload: {payload}"
    );

    wait_for_pool_attempt_row_count(&state.pool, 6).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 6);

    let attempt_rows = sqlx::query_as::<_, AttemptStatusRow>(
        r#"
        SELECT status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load preserved distinct-budget attempt rows");
    assert_eq!(attempt_rows.len(), 6);
    assert_eq!(
        attempt_rows[5].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL
    );
    assert_eq!(
        attempt_rows[5].failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(1));
    drop(attempts);

    assert_eq!(
        load_test_sticky_route_account_id(
            &state.pool,
            "sticky-existing-owner-preserve-after-distinct-budget",
        )
        .await,
        Some(primary_id),
        "sticky binding should stay on the original owner when distinct-budget cut-out never succeeds",
    );
    assert_ne!(secondary_id, primary_id);
    assert_ne!(tertiary_id, primary_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_no_candidate_after_wait_preserves_last_upstream_failure() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::INTERNAL_SERVER_ERROR)],
    )
    .await;
    let state = test_state_with_openai_base_and_pool_no_available_wait(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        Duration::from_millis(60),
        Duration::from_millis(10),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

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
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-no-candidate-preserve-last-error"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert!(
        started.elapsed() >= Duration::from_millis(50),
        "request should wait roughly the bounded window before failing"
    );
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        response.headers().get(http_header::RETRY_AFTER).is_none(),
        "preserved upstream failures should not advertise pool Retry-After"
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("pool upstream responded with 500")),
        "unexpected preserved upstream failure payload: {payload}"
    );

    wait_for_pool_attempt_row_count(&state.pool, 3).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 3);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_skips_ungrouped_account_when_grouped_alternate_exists() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let ungrouped_id =
        insert_test_pool_api_key_account(&state, "Ungrouped", "upstream-primary").await;
    let grouped_id =
        insert_test_pool_api_key_account(&state, "Grouped", "upstream-secondary").await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
        .bind(ungrouped_id)
        .execute(&state.pool)
        .await
        .expect("clear ungrouped account group");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-ungrouped-fresh"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

    let route_account_id =
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-ungrouped-fresh")
            .await
            .expect("sticky route should bind to grouped alternate");
    assert_eq!(route_account_id, grouped_id);
    assert_ne!(route_account_id, ungrouped_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_returns_specific_ungrouped_error_when_all_candidates_are_ungrouped() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let ungrouped_id =
        insert_test_pool_api_key_account(&state, "Ungrouped", "upstream-primary").await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
        .bind(ungrouped_id)
        .execute(&state.pool)
        .await
        .expect("clear ungrouped account group");

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-ungrouped-only"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get(http_header::RETRY_AFTER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|value| value.contains("upstream account is not assigned to a group")),
        "unexpected error payload: {payload:?}"
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_cuts_out_from_ungrouped_sticky_account_when_allowed() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let sticky_source_id =
        insert_test_pool_api_key_account(&state, "Ungrouped Sticky", "upstream-primary").await;
    let grouped_id =
        insert_test_pool_api_key_account(&state, "Grouped", "upstream-secondary").await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
        .bind(sticky_source_id)
        .execute(&state.pool)
        .await
        .expect("clear sticky source group");
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-ungrouped-cut-out",
        sticky_source_id,
        &sticky_seen_at,
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
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-ungrouped-cut-out"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

    let mut route_account_id =
        load_test_sticky_route_account_id(&state.pool, "sticky-ungrouped-cut-out").await;
    for _ in 0..20 {
        if route_account_id == Some(grouped_id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
        route_account_id =
            load_test_sticky_route_account_id(&state.pool, "sticky-ungrouped-cut-out").await;
    }
    let route_account_id = route_account_id.expect("sticky route should be persisted");
    assert_eq!(route_account_id, grouped_id);
    assert_ne!(route_account_id, sticky_source_id);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_returns_ungrouped_error_for_sticky_account_when_cut_out_is_forbidden() {
    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
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
        "sticky-ungrouped-forbidden",
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
    .bind("ungrouped-no-cut-out")
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

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-ungrouped-forbidden"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get(http_header::RETRY_AFTER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failure payload");
    assert!(
        payload["error"]
            .as_str()
            .is_some_and(|value| value.contains("upstream account is not assigned to a group")),
        "unexpected error payload: {payload:?}"
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), None);
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-ungrouped-forbidden").await,
        Some(sticky_source_id)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_returns_429_after_three_distinct_accounts_hit_upstream_429() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        http_status: Option<i64>,
        failure_kind: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) = spawn_pool_rate_limit_responses_upstream(&[
        ("Bearer upstream-primary", 99),
        ("Bearer upstream-secondary", 99),
        ("Bearer upstream-tertiary", 99),
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
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-429-budget"}"#
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
    assert_eq!(
        payload["error"].as_str(),
        Some(POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE)
    );

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(1));
    drop(attempts);

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, status, http_status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load attempt rows");
    assert_eq!(attempt_rows.len(), 4);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(attempt_rows[1].distinct_account_index, 2);
    assert_eq!(attempt_rows[1].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[1].failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert_eq!(attempt_rows[2].attempt_index, 3);
    assert_eq!(attempt_rows[2].distinct_account_index, 3);
    assert_eq!(attempt_rows[2].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[2].failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert_eq!(attempt_rows[3].attempt_index, 4);
    assert_eq!(attempt_rows[3].distinct_account_index, 3);
    assert_eq!(attempt_rows[3].same_account_retry_index, 0);
    assert_eq!(
        attempt_rows[3].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL
    );
    assert_eq!(
        attempt_rows[3].http_status,
        Some(StatusCode::TOO_MANY_REQUESTS.as_u16() as i64)
    );
    assert_eq!(
        attempt_rows[3].failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED)
    );

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
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_persists_attempt_rows_and_summary_fields() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        upstream_account_id: Option<i64>,
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        http_status: Option<i64>,
        failure_kind: Option<String>,
        stream_latency_ms: Option<f64>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (upstream_base, _attempts, upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer upstream-primary", 2)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-attempts-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read capture response body");

    wait_for_codex_invocations(&state.pool, 1).await;
    for _ in 0..20 {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id LIKE 'proxy-%'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("count attempt rows");
        if count >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT
            upstream_account_id,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status,
            http_status,
            failure_kind,
            stream_latency_ms
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load persisted attempt rows");
    assert_eq!(attempt_rows.len(), 3);
    assert_eq!(attempt_rows[0].upstream_account_id, Some(primary_id));
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(
        attempt_rows[0].failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX),
    );
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(attempt_rows[2].same_account_retry_index, 3);
    assert_eq!(
        attempt_rows[2].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );
    assert_eq!(attempt_rows[2].http_status, Some(200));
    assert!(attempt_rows[2].stream_latency_ms.unwrap_or_default() >= 0.0);

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
    .expect("load persisted invocation payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(3));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert!(payload["poolAttemptTerminalReason"].is_null());

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_stops_after_three_distinct_accounts() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptStatusRow {
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[
        ("Bearer upstream-primary", 99),
        ("Bearer upstream-secondary", 99),
        ("Bearer upstream-tertiary", 99),
        ("Bearer upstream-quaternary", 0),
    ])
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    insert_test_pool_api_key_account(&state, "Tertiary", "upstream-tertiary").await;
    insert_test_pool_api_key_account(&state, "Quaternary", "upstream-quaternary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-attempts-002"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failure response body");

    wait_for_codex_invocations(&state.pool, 1).await;
    for _ in 0..20 {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id LIKE 'proxy-%'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("count budget attempt rows");
        if count >= 6 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let attempt_status_rows = sqlx::query_as::<_, AttemptStatusRow>(
        r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load attempt status rows");
    assert_eq!(attempt_status_rows.len(), 6);
    assert_eq!(attempt_status_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_status_rows[1].same_account_retry_index, 2);
    assert_eq!(attempt_status_rows[2].same_account_retry_index, 3);
    assert_eq!(attempt_status_rows[3].same_account_retry_index, 1);
    assert_eq!(attempt_status_rows[4].same_account_retry_index, 1);
    assert_eq!(attempt_status_rows[4].attempt_index, 5);
    assert_eq!(attempt_status_rows[4].distinct_account_index, 3);
    assert_eq!(
        attempt_status_rows[5].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    assert_eq!(attempt_status_rows[5].attempt_index, 6);
    assert_eq!(attempt_status_rows[5].distinct_account_index, 3);
    assert_eq!(
        attempt_status_rows[5].failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED),
    );

    let attempts = attempts.lock().expect("lock attempt counters");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-quaternary").copied(), None);
    drop(attempts);

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
    .expect("load exhausted invocation payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("exhausted payload should be present"),
    )
    .expect("decode exhausted payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(5));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED),
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_failover_reapplies_account_fast_mode_from_original_body() {
    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (failing_base, _failing_attempts, failing_handle) =
        spawn_pool_static_failure_responses_upstream(&[(
            "Bearer route-remove",
            StatusCode::INTERNAL_SERVER_ERROR,
        )])
        .await;
    let (capture_base, captured_requests, capture_handle) =
        spawn_capture_target_body_upstream().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;

    let force_remove_tag_payload = serde_json::from_value::<CreateTagRequest>(json!({
        "name": "force-remove-primary",
        "guardEnabled": false,
        "allowCutOut": true,
        "allowCutIn": true,
        "priorityTier": "primary",
        "fastModeRewriteMode": "force_remove",
    }))
    .expect("deserialize force-remove tag payload");
    let Json(_force_remove_tag) = create_tag(
        State(state.clone()),
        HeaderMap::new(),
        Json(force_remove_tag_payload),
    )
    .await
    .expect("create force-remove tag");
    let force_remove_tag_id: i64 = sqlx::query_scalar("SELECT id FROM pool_tags WHERE name = ?1")
        .bind("force-remove-primary")
        .fetch_one(&state.pool)
        .await
        .expect("load force-remove tag id");
    let fill_missing_tag_payload = serde_json::from_value::<CreateTagRequest>(json!({
        "name": "fill-missing-normal",
        "guardEnabled": false,
        "allowCutOut": true,
        "allowCutIn": true,
        "priorityTier": "normal",
        "fastModeRewriteMode": "fill_missing",
    }))
    .expect("deserialize fill-missing tag payload");
    let Json(_fill_missing_tag) = create_tag(
        State(state.clone()),
        HeaderMap::new(),
        Json(fill_missing_tag_payload),
    )
    .await
    .expect("create fill-missing tag");
    let fill_missing_tag_id: i64 = sqlx::query_scalar("SELECT id FROM pool_tags WHERE name = ?1")
        .bind("fill-missing-normal")
        .fetch_one(&state.pool)
        .await
        .expect("load fill-missing tag id");

    let first_account_payload = serde_json::from_value::<CreateApiKeyAccountRequest>(json!({
        "displayName": "Route Remove",
        "groupName": test_required_group_name(),
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "upstreamBaseUrl": failing_base,
        "apiKey": "route-remove",
        "tagIds": [force_remove_tag_id],
    }))
    .expect("deserialize first api-key account payload");
    let Json(_first_account) = create_api_key_account(
        State(state.clone()),
        HeaderMap::new(),
        Json(first_account_payload),
    )
    .await
    .expect("create first pool account");
    let second_account_payload = serde_json::from_value::<CreateApiKeyAccountRequest>(json!({
        "displayName": "Route Fill",
        "groupName": test_required_group_name(),
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "upstreamBaseUrl": capture_base,
        "apiKey": "route-fill",
        "tagIds": [fill_missing_tag_id],
    }))
    .expect("deserialize second api-key account payload");
    let Json(_second_account) = create_api_key_account(
        State(state.clone()),
        HeaderMap::new(),
        Json(second_account_payload),
    )
    .await
    .expect("create second pool account");

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "serviceTier": "flex",
        "input": "hello"
    }))
    .expect("serialize failover request body");
    let response = proxy_openai_v1(
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
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failover response body");
    let response_payload: Value =
        serde_json::from_slice(&response_body).expect("decode failover response body");
    assert_eq!(
        response_payload["received"]["serviceTier"].as_str(),
        Some("flex")
    );
    assert!(response_payload["received"].get("service_tier").is_none());

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_upstream_request_attempts(&state.pool, 2).await;

    let captured = captured_requests.lock().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["serviceTier"].as_str(), Some("flex"));
    assert!(captured[0].get("service_tier").is_none());
    drop(captured);

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
    .expect("load persisted failover payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("persisted payload should exist"),
    )
    .expect("decode persisted failover payload");
    assert_eq!(payload["requestedServiceTier"].as_str(), Some("flex"));
    assert!(
        payload["poolAttemptCount"]
            .as_i64()
            .is_some_and(|count| count >= 2)
    );
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));

    failing_handle.abort();
    capture_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_fast_fill_missing_large_body_recomputes_content_length() {
    #[derive(Debug, sqlx::FromRow)]
    struct PersistedRow {
        payload: Option<String>,
        request_raw_path: Option<String>,
    }

    let (capture_base, captured_requests, capture_handle) =
        spawn_capture_target_body_upstream().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;

    let fill_missing_tag_id =
        create_test_fast_mode_tag(&state, "fill-missing-large-fast", "fill_missing", "primary")
            .await;
    create_test_tagged_pool_api_key_account(
        &state,
        "Large Fast Account",
        "upstream-large-fast",
        &capture_base,
        &[fill_missing_tag_id],
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": [{
            "role": "user",
            "content": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 4096)
        }],
        "stickyKey": "sticky-fast-large-success"
    }))
    .expect("serialize large fast request body");
    let response = proxy_openai_v1(
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
            (
                http_header::CONTENT_LENGTH,
                HeaderValue::from_str(&request_body.len().to_string())
                    .expect("valid content length"),
            ),
        ]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read large fast response body");
    let response_payload: Value =
        serde_json::from_slice(&response_body).expect("decode large fast response body");
    assert_eq!(
        response_payload["received"]["service_tier"].as_str(),
        Some("priority")
    );
    assert!(response_payload["received"].get("serviceTier").is_none());

    wait_for_codex_invocations(&state.pool, 1).await;
    let captured = captured_requests.lock().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["service_tier"].as_str(), Some("priority"));
    assert!(captured[0].get("serviceTier").is_none());
    drop(captured);

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT payload, request_raw_path
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load persisted large fast row");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("persisted payload should exist"),
    )
    .expect("decode persisted large fast payload");
    assert_eq!(payload["requestedServiceTier"].as_str(), Some("priority"));

    let request_raw = read_proxy_raw_bytes(
        row.request_raw_path
            .as_deref()
            .expect("large fast request raw path should exist"),
        state.config.database_path.parent(),
    )
    .expect("read large fast request raw");
    let request_payload: Value =
        serde_json::from_slice(&request_raw).expect("decode large fast request raw");
    assert_eq!(request_payload["service_tier"].as_str(), Some("priority"));
    assert!(request_payload.get("serviceTier").is_none());

    capture_handle.abort();
}

#[tokio::test]
async fn pool_openai_v1_responses_fast_fill_missing_transport_failure_persists_rewritten_request_raw()
 {
    #[derive(Debug, sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
        request_raw_path: Option<String>,
    }

    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;

    let fill_missing_tag_id = create_test_fast_mode_tag(
        &state,
        "fill-missing-transport-fast",
        "fill_missing",
        "primary",
    )
    .await;
    create_test_tagged_pool_api_key_account(
        &state,
        "Broken Fast Account",
        "upstream-broken-fast",
        "http://127.0.0.1:1/",
        &[fill_missing_tag_id],
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "stream": false,
        "input": [{
            "role": "user",
            "content": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 4096)
        }],
        "stickyKey": "sticky-fast-large-failure"
    }))
    .expect("serialize failed large fast request body");
    let response = proxy_openai_v1(
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
            (
                http_header::CONTENT_LENGTH,
                HeaderValue::from_str(&request_body.len().to_string())
                    .expect("valid content length"),
            ),
        ]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failed large fast response body");
    let response_text = String::from_utf8_lossy(&response_body);
    assert!(
        response_text.contains("failed to contact upstream"),
        "unexpected failed large fast response body: {response_text}"
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, error_message, payload, request_raw_path
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load failed large fast row");
    assert_eq!(row.status.as_deref(), Some("http_502"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|message| { message.contains("[failed_contact_upstream]") })
    );

    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("failed large fast payload should exist"),
    )
    .expect("decode failed large fast payload");
    assert_eq!(payload["requestedServiceTier"].as_str(), Some("priority"));
    assert_eq!(
        payload["failureKind"].as_str(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );

    let request_raw = read_proxy_raw_bytes(
        row.request_raw_path
            .as_deref()
            .expect("failed large fast request raw path should exist"),
        state.config.database_path.parent(),
    )
    .expect("read failed large fast request raw");
    let request_payload: Value =
        serde_json::from_slice(&request_raw).expect("decode failed large fast request raw");
    assert_eq!(request_payload["service_tier"].as_str(), Some("priority"));
    assert!(request_payload.get("serviceTier").is_none());
}

#[tokio::test]
async fn pool_route_responses_compact_limits_follow_up_accounts_to_single_attempt() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[
        ("Bearer upstream-primary", 99),
        ("Bearer upstream-secondary", 99),
        ("Bearer upstream-tertiary", 0),
    ])
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    insert_test_pool_api_key_account(&state, "Tertiary", "upstream-tertiary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_001",
        "input": [{"role": "user", "content": "compact this thread"}],
    }))
    .expect("serialize compact request body");
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
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(request_body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact retry body");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 5).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load compact retry rows");
    assert_eq!(attempt_rows.len(), 5);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert_eq!(attempt_rows[2].distinct_account_index, 1);
    assert_eq!(attempt_rows[2].same_account_retry_index, 3);
    assert_eq!(attempt_rows[3].distinct_account_index, 2);
    assert_eq!(attempt_rows[3].same_account_retry_index, 1);
    assert_eq!(attempt_rows[4].distinct_account_index, 3);
    assert_eq!(attempt_rows[4].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[4].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );

    let attempts = attempts.lock().expect("lock compact attempt counters");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(1));
    drop(attempts);

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
    .expect("load compact invocation payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("compact payload should be present"),
    )
    .expect("decode compact payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(5));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert!(payload["poolAttemptTerminalReason"].is_null());

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_compact_502_returns_cvm_id_and_attempt_observations() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_compact_unsupported_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    insert_test_pool_api_key_account(&state, "Tertiary", "upstream-tertiary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "previous_response_id": "resp_prev_002",
        "input": [{"role": "user", "content": "compact this thread"}],
    }))
    .expect("serialize compact request body");
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
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let cvm_id = response
        .headers()
        .get(CVM_INVOKE_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .expect("cvm id header should be present");
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact 502 response");
    let payload: Value = serde_json::from_slice(&body).expect("decode compact 502 payload");
    assert_eq!(payload["cvmId"].as_str(), Some(cvm_id.as_str()));
    assert_eq!(
        payload["error"].as_str(),
        Some("pool distinct-account retry budget exhausted"),
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 6).await;

    let invocation_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
            .bind(&cvm_id)
            .fetch_optional(&state.pool)
            .await
            .expect("load compact invocation status");
    assert_eq!(invocation_status.as_deref(), Some("http_502"));

    let Json(attempt_rows) =
        fetch_invocation_pool_attempts(State(state.clone()), axum::extract::Path(cvm_id.clone()))
            .await
            .expect("fetch invocation pool attempts");
    assert_eq!(attempt_rows.len(), 6);
    assert_eq!(
        attempt_rows[0].compact_support_status.as_deref(),
        Some(COMPACT_SUPPORT_STATUS_UNSUPPORTED),
    );
    assert!(
        attempt_rows[0]
            .compact_support_reason
            .as_deref()
            .is_some_and(|value| value.contains("No available channel for model")),
    );
    assert_eq!(
        attempt_rows[1].compact_support_status.as_deref(),
        Some(COMPACT_SUPPORT_STATUS_UNSUPPORTED),
    );
    assert_eq!(
        attempt_rows[2].compact_support_status.as_deref(),
        Some(COMPACT_SUPPORT_STATUS_UNSUPPORTED),
    );
    assert_eq!(
        attempt_rows[3].compact_support_status.as_deref(),
        Some(COMPACT_SUPPORT_STATUS_UNSUPPORTED),
    );
    assert_eq!(
        attempt_rows[4].compact_support_status.as_deref(),
        Some(COMPACT_SUPPORT_STATUS_UNSUPPORTED),
    );
    assert_eq!(attempt_rows[5].compact_support_status, None);

    let account_support_states = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT compact_support_status, compact_support_reason
        FROM pool_upstream_accounts
        ORDER BY id ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load compact support states");
    assert_eq!(account_support_states.len(), 3);
    assert!(
        account_support_states
            .iter()
            .all(|row| row.0 == COMPACT_SUPPORT_STATUS_UNSUPPORTED)
    );

    let attempts = attempts.lock().expect("lock compact unsupported attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(1));
    drop(attempts);

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_chat_completions_keeps_three_attempts_for_follow_up_accounts() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRow {
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (upstream_base, attempts, upstream_handle) = spawn_pool_retry_upstream(&[
        ("Bearer upstream-primary", 99),
        ("Bearer upstream-secondary", 2),
    ])
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.4",
        "messages": [{"role": "user", "content": "hello"}],
    }))
    .expect("serialize chat completions body");
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/chat/completions"
                .parse()
                .expect("valid chat completions uri"),
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
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read chat completions retry body");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 6).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRow>(
        r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, status
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load chat completions retry rows");
    assert_eq!(attempt_rows.len(), 6);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert_eq!(attempt_rows[2].same_account_retry_index, 3);
    assert_eq!(attempt_rows[3].distinct_account_index, 2);
    assert_eq!(attempt_rows[3].same_account_retry_index, 1);
    assert_eq!(attempt_rows[4].same_account_retry_index, 2);
    assert_eq!(attempt_rows[5].same_account_retry_index, 3);
    assert_eq!(
        attempt_rows[5].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );

    let attempts = attempts
        .lock()
        .expect("lock chat completions attempt counters");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
    drop(attempts);

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
    .expect("load chat completions payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("chat completions payload should be present"),
    )
    .expect("decode chat completions payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(6));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
    assert!(payload["poolAttemptTerminalReason"].is_null());

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_preserves_auth_failure_terminal_reason() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptFailureRow {
        status: String,
        http_status: Option<i64>,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_pool_http_failure_upstream(
        StatusCode::UNAUTHORIZED,
        Some("invalid_token"),
        "token expired",
    )
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
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-attempts-auth-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read auth failure body");

    wait_for_codex_invocations(&state.pool, 1).await;

    let attempt_row = sqlx::query_as::<_, AttemptFailureRow>(
        r#"
        SELECT status, http_status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load auth failure attempt row");
    assert_eq!(
        attempt_row.status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(
        attempt_row.http_status,
        Some(i64::from(StatusCode::UNAUTHORIZED.as_u16())),
    );
    assert_eq!(
        attempt_row.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH),
    );

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
    .expect("load auth failure invocation payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("auth failure payload should be present"),
    )
    .expect("decode auth failure payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH),
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_timeout_switches_to_alternate_upstream_route() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_account_id: Option<i64>,
        upstream_route_key: Option<String>,
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (slow_upstream_base, slow_upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(750)).await;
    let (fast_upstream_base, _attempts, fast_upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-fast", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(250);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let slow_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Slow Route",
        "route-slow",
        None,
        None,
        Some(slow_upstream_base.as_str()),
    )
    .await;
    let fast_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fast Route",
        "route-fast",
        None,
        None,
        Some(fast_upstream_base.as_str()),
    )
    .await;

    let sticky_key = "sticky-timeout-switch-001";
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
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout-switch success body");

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_upstream_request_attempts(&state.pool, 2).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load timeout-switch attempt rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].upstream_account_id, Some(slow_id));
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(
        attempt_rows[0].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR),
    );
    assert_eq!(attempt_rows[1].upstream_account_id, Some(fast_id));
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(attempt_rows[1].distinct_account_index, 2);
    assert_eq!(attempt_rows[1].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    );
    assert_eq!(
        attempt_rows[0].upstream_route_key.as_deref(),
        Some(
            canonical_pool_upstream_route_key(
                &Url::parse(&slow_upstream_base).expect("valid slow route url")
            )
            .as_str()
        ),
    );
    assert_eq!(
        attempt_rows[1].upstream_route_key.as_deref(),
        Some(
            canonical_pool_upstream_route_key(
                &Url::parse(&fast_upstream_base).expect("valid fast route url")
            )
            .as_str()
        ),
    );

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
    .expect("load timeout-switch invocation payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout-switch payload should be present"),
    )
    .expect("decode timeout-switch payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(2));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
    assert!(payload["poolAttemptTerminalReason"].is_null());

    slow_upstream_handle.abort();
    fast_upstream_handle.abort();
}

#[test]
fn canonical_pool_upstream_route_key_collapses_trailing_slashes() {
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
fn pool_failure_is_timeout_shaped_ignores_upstream_5xx_text_timeouts() {
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
async fn capture_target_pool_route_timeout_returns_no_alternate_when_only_same_route_remains() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_route_key: Option<String>,
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        error_message: Option<String>,
        payload: Option<String>,
    }

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
    wait_for_pool_attempt_row_count(&state.pool, 2).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load timeout no-alternate rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    assert_eq!(
        attempt_rows[1].failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT),
    );
    assert_eq!(attempt_rows[1].same_account_retry_index, 0);
    assert_eq!(
        attempt_rows[0].upstream_route_key,
        attempt_rows[1].upstream_route_key,
    );
    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load timeout no-alternate payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout no-alternate payload should be present"),
    )
    .expect("decode timeout no-alternate payload");
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

#[tokio::test]
async fn capture_target_pool_route_timeout_surfaces_blocked_policy_terminal() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_route_key: Option<String>,
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        error_message: Option<String>,
        payload: Option<String>,
    }

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
        Some("https://ungrouped.example.com/backend-api/codex"),
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
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().get(http_header::RETRY_AFTER).is_none());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout blocked-policy response body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode timeout blocked-policy response body");
    assert!(
        response_payload["error"]
            .as_str()
            .expect("timeout blocked-policy error should be present")
            .contains("upstream account is not assigned to a group")
    );

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 2).await;

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load timeout blocked-policy rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    assert_eq!(
        attempt_rows[1].failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT),
    );
    assert_eq!(attempt_rows[1].same_account_retry_index, 0);

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load timeout blocked-policy payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout blocked-policy payload should be present"),
    )
    .expect("decode timeout blocked-policy payload");
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("upstream account is not assigned to a group"))
    );
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT),
    );
    assert!(payload["upstreamErrorMessage"].is_null());

    shared_upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_route_timeout_ignores_broken_same_route_groups() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_route_key: Option<String>,
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        error_message: Option<String>,
        payload: Option<String>,
    }

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

    let attempt_rows = sqlx::query_as::<_, AttemptRouteRow>(
        r#"
        SELECT
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            status,
            failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load timeout no-alternate invalid-group rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    );
    assert_eq!(attempt_rows[1].attempt_index, 2);
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(
        attempt_rows[1].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    assert_eq!(
        attempt_rows[1].failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_NO_ALTERNATE_UPSTREAM_AFTER_TIMEOUT),
    );
    assert_eq!(attempt_rows[1].same_account_retry_index, 0);
    assert_eq!(
        attempt_rows[0].upstream_route_key,
        attempt_rows[1].upstream_route_key,
    );

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load timeout no-alternate invalid-group payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("timeout no-alternate invalid-group payload should be present"),
    )
    .expect("decode timeout no-alternate invalid-group payload");
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
