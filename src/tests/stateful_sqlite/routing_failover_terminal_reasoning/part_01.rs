pub(crate) fn run_routing_future_with_large_stack<Fut>(future: Fut)
where
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name("routing-failover-large-stack".to_string())
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
        .expect("join large-stack test worker");
}

#[tokio::test]
pub(crate) async fn pool_route_existing_sticky_owner_preserves_last_failure_after_cutout_alternate_fails()
 {
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
    .expect("load preserved cut-out alternate attempt rows");
    assert_eq!(attempt_rows.len(), 6);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
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

type AttemptStatusRow = (String, Option<String>);

#[tokio::test]
pub(crate) async fn pool_route_existing_sticky_owner_preserves_last_failure_after_distinct_budget_exhausts()
 {
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

    wait_for_pool_attempt_row_count(&state.pool, 9).await;
    assert_eq!(count_pool_upstream_request_attempts(&state.pool).await, 9);

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
    assert_eq!(attempt_rows.len(), 9);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(3));
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
pub(crate) async fn pool_route_no_candidate_after_wait_preserves_last_upstream_failure() {
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
pub(crate) async fn pool_route_uses_ungrouped_transit_when_grouped_alternate_exists() {
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
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    let route_account_id =
        wait_for_test_sticky_route_account_id(&state.pool, "sticky-ungrouped-fresh")
            .await
            .expect("sticky route should bind to ungrouped transit");
    assert_eq!(route_account_id, ungrouped_id);
    assert_ne!(route_account_id, grouped_id);

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_route_uses_ungrouped_transit_when_it_is_only_candidate() {
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

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_route_keeps_ungrouped_sticky_transit_when_cut_out_is_allowed() {
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
    assert_eq!(payload["authorization"], "Bearer upstream-primary");
    assert_eq!(payload["attempt"], 1);

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    let route_account_id =
        load_test_sticky_route_account_id(&state.pool, "sticky-ungrouped-cut-out").await;
    let route_account_id = route_account_id.expect("sticky route should be persisted");
    assert_eq!(route_account_id, sticky_source_id);
    assert_ne!(route_account_id, grouped_id);

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_route_keeps_ungrouped_sticky_transit_when_cut_out_is_forbidden() {
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
            name,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, 1, ?2, ?2)
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

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response");
    assert_eq!(payload["authorization"], "Bearer upstream-primary");

    let attempts = attempts.lock().expect("lock attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), None);
    drop(attempts);

    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-ungrouped-forbidden").await,
        Some(sticky_source_id)
    );

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_route_returns_429_after_three_distinct_accounts_hit_upstream_429() {
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
    assert_eq!(attempt_rows.len(), 3);
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
    upstream_handle.abort();
}

pub(crate) async fn wait_for_pool_attempt_status(
    pool: &SqlitePool,
    attempt_index: i64,
    expected: &str,
) {
    for _ in 0..100 {
        let status: Option<String> = sqlx::query_scalar(
            r#"
            SELECT status
            FROM pool_upstream_request_attempts
            WHERE attempt_index = ?1
            "#,
        )
        .bind(attempt_index)
        .fetch_optional(pool)
        .await
        .expect("load pool attempt status");
        if status.as_deref() == Some(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for attempt {attempt_index} to reach status {expected}");
}

pub(crate) async fn wait_for_pool_attempt_terminal_observation(
    pool: &SqlitePool,
    invoke_id: &str,
    attempt_index: i64,
    expected_status: &str,
    expected_http_status: i64,
) {
    for _ in 0..100 {
        let observation: Option<(String, Option<i64>)> = sqlx::query_as(
            r#"
            SELECT status, http_status
            FROM pool_upstream_request_attempts
            WHERE invoke_id = ?1
              AND attempt_index = ?2
            "#,
        )
        .bind(invoke_id)
        .bind(attempt_index)
        .fetch_optional(pool)
        .await
        .expect("load pool attempt terminal observation");
        if observation.as_ref().is_some_and(|(status, http_status)| {
            status == expected_status && *http_status == Some(expected_http_status)
        }) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "timed out waiting for invocation {invoke_id} attempt {attempt_index} to reach {expected_status} with HTTP {expected_http_status}"
    );
}

#[tokio::test]
pub(crate) async fn pool_route_retries_upstream_413_once_on_same_account_then_succeeds() {
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
pub(crate) fn priority_handoff_does_not_retry_upstream_413() {
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
pub(crate) async fn pool_route_switches_account_after_upstream_413_retry_fails() {
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

use super::*;
