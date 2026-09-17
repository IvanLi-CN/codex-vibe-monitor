#[derive(Debug, sqlx::FromRow)]
struct PersistedPoolAttemptRow {
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
struct StandaloneSearchInvocationRow {
    model: Option<String>,
    status: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    payload: Option<String>,
    request_raw_path: Option<String>,
    response_raw_path: Option<String>,
    t_total_ms: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct DistinctBudgetAttemptRow {
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: String,
    failure_kind: Option<String>,
}

#[derive(sqlx::FromRow)]
struct InvocationPayloadRow {
    payload: Option<String>,
}

async fn load_latest_invocation_payload(pool: &SqlitePool) -> Value {
    let row = sqlx::query_as::<_, InvocationPayloadRow>(
        "SELECT payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load persisted invocation payload");
    serde_json::from_str(
        row.payload
            .as_deref()
            .expect("persisted payload should exist"),
    )
    .expect("decode persisted invocation payload")
}

async fn assert_standalone_search_rollup(pool: &SqlitePool) {
    backfill_invocation_rollup_hourly_from_sources(pool)
        .await
        .expect("rebuild standalone search rollup");
    backfill_invocation_rollup_hourly_from_sources(pool)
        .await
        .expect("replay standalone search rollup without duplication");
    let rollup = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        r#"SELECT COALESCE(SUM(total_count), 0), COALESCE(SUM(success_count), 0),
                  COALESCE(SUM(failure_count), 0), COALESCE(SUM(terminal_count), 0)
           FROM invocation_rollup_hourly WHERE source = ?1"#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(pool)
    .await
    .expect("load standalone search rollup");
    assert_eq!(rollup, (1, 1, 0, 1));
}

async fn insert_fast_mode_tag(
    pool: &SqlitePool,
    name: &str,
    priority_tier: &str,
    rewrite_mode: &str,
    now_iso: &str,
) -> i64 {
    sqlx::query_scalar(
        r#"INSERT INTO pool_tags (
               name, system_key, protected, allow_cut_out, allow_cut_in,
               priority_tier, fast_mode_rewrite_mode, concurrency_limit,
               upstream_429_retry_enabled, upstream_429_max_retries,
               available_models_json, created_at, updated_at
           ) VALUES (?1, NULL, 0, 1, 1, ?2, ?3, 0, 0, 0, '[]', ?4, ?4)
           RETURNING id"#,
    )
    .bind(name)
    .bind(priority_tier)
    .bind(rewrite_mode)
    .bind(now_iso)
    .fetch_one(pool)
    .await
    .expect("insert fast-mode tag")
}

async fn create_tagged_fast_mode_account(
    state: &Arc<AppState>,
    display_name: &str,
    upstream_base_url: &str,
    api_key: &str,
    tag_id: i64,
    now_iso: &str,
) {
    let payload = serde_json::from_value::<CreateApiKeyAccountRequest>(json!({
        "displayName": display_name,
        "upstreamBaseUrl": upstream_base_url,
        "apiKey": api_key,
    }))
    .expect("deserialize api-key account payload");
    let _ = create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect("create pool account");
    let account_id: i64 =
        sqlx::query_scalar("SELECT id FROM pool_upstream_accounts WHERE display_name = ?1")
            .bind(display_name)
            .fetch_one(&state.pool)
            .await
            .expect("load pool account id");
    restore_test_legacy_api_key_group(&state.pool, account_id, test_required_group_name(), false)
        .await;
    sqlx::query(
        r#"INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?3)"#,
    )
    .bind(account_id)
    .bind(tag_id)
    .bind(now_iso)
    .execute(&state.pool)
    .await
    .expect("attach fast-mode tag");
}

async fn assert_fast_mode_failover_capture(
    pool: &SqlitePool,
    captured_requests: &Arc<Mutex<Vec<Value>>>,
) {
    wait_for_codex_invocations(pool, 1).await;
    wait_for_pool_upstream_request_attempts(pool, 2).await;
    let captured = captured_requests.lock().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["serviceTier"].as_str(), Some("flex"));
    assert!(captured[0].get("service_tier").is_none());
    drop(captured);
    let payload = load_latest_invocation_payload(pool).await;
    assert_eq!(payload["requestedServiceTier"].as_str(), Some("flex"));
    assert!(
        payload["poolAttemptCount"]
            .as_i64()
            .is_some_and(|count| count >= 2)
    );
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
}

#[tokio::test]
pub(crate) async fn pool_route_retries_first_upstream_413_after_prior_5xx_same_account() {
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
pub(crate) async fn pool_route_retries_first_upstream_413_at_end_of_same_account_budget() {
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
pub(crate) async fn pool_route_returns_413_when_single_account_upstream_413_retry_fails() {
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
pub(crate) async fn pool_route_preserves_413_when_distinct_account_budget_exhausts() {
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
pub(crate) async fn pool_route_does_not_use_pool_wide_429_message_when_budget_exhaustion_is_mixed()
{
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

#[tokio::test]
pub(crate) async fn capture_target_pool_route_persists_attempt_rows_and_summary_fields() {
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
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts")
            .fetch_one(&state.pool)
            .await
            .expect("count attempt rows");
        if count >= 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let attempt_rows = sqlx::query_as::<_, PersistedPoolAttemptRow>(
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

    let payload = load_latest_invocation_payload(&state.pool).await;
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(3));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert!(payload["poolAttemptTerminalReason"].is_null());

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_standalone_search_records_one_invocation_and_rollup() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer upstream-primary", 2)]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/alpha/search?cursor=next"
                .parse()
                .expect("valid standalone search uri"),
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
        Body::from(br#"{"model":"gpt-5.4","query":"hello","results":[]}"#.to_vec()),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let response_payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read standalone search response"),
    )
    .expect("decode standalone search response");
    assert_eq!(response_payload["path"], "/v1/alpha/search");
    assert_eq!(response_payload["query"], "cursor=next");
    assert_eq!(response_payload["attempt"], 3);

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 3).await;
    let invocation_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&state.pool)
        .await
        .expect("count standalone search invocations");
    assert_eq!(invocation_count, 1);

    let invocation = sqlx::query_as::<_, StandaloneSearchInvocationRow>(
        r#"
        SELECT model, status, input_tokens, output_tokens, total_tokens, cost, payload,
               request_raw_path, response_raw_path, t_total_ms
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load standalone search invocation");
    assert_eq!(invocation.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(invocation.status.as_deref(), Some("success"));
    assert!(invocation.input_tokens.is_none());
    assert!(invocation.output_tokens.is_none());
    assert!(invocation.total_tokens.is_none());
    assert!(invocation.cost.is_none());
    assert!(invocation.t_total_ms.is_some_and(|value| value >= 0.0));
    assert!(invocation.request_raw_path.is_some());
    assert!(invocation.response_raw_path.is_some());
    let payload: Value = serde_json::from_str(
        invocation
            .payload
            .as_deref()
            .expect("standalone search payload should be present"),
    )
    .expect("decode standalone search payload");
    assert_eq!(payload["endpoint"], "/v1/alpha/search");
    assert_eq!(payload["isStream"], false);
    assert_eq!(payload["requestModel"], "gpt-5.4");
    assert!(payload["inputTokens"].is_null());
    assert!(payload["outputTokens"].is_null());
    assert!(payload["totalTokens"].is_null());

    assert_standalone_search_rollup(&state.pool).await;

    assert_eq!(
        attempts
            .lock()
            .expect("lock standalone search attempts")
            .get("Bearer upstream-primary")
            .copied(),
        Some(3)
    );
    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_standalone_search_records_upstream_404_as_failure() {
    let (upstream_base, _attempts, upstream_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer upstream-primary", StatusCode::NOT_FOUND)],
    )
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/alpha/search"
                .parse()
                .expect("valid standalone search uri"),
        ),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(br#"{"model":"gpt-5.4","query":"hello"}"#.to_vec()),
    )
    .await;
    assert!(!response.status().is_success());
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read standalone search upstream 404 response");

    wait_for_codex_invocations(&state.pool, 1).await;
    let (status, failure_kind, error_message, payload): (
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    ) = sqlx::query_as(
        "SELECT status, failure_kind, error_message, payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load standalone search 404 invocation");
    assert_ne!(status.as_deref(), Some("success"));
    assert!(
        failure_kind.is_some(),
        "404 failure kind missing: {error_message:?}"
    );
    assert_ne!(failure_kind.as_deref(), Some("oauth_unsupported_route"));
    assert!(
        error_message
            .as_deref()
            .is_some_and(|message| message.to_ascii_lowercase().contains("upstream")),
        "404 error message should identify upstream failure: {error_message:?}"
    );
    let payload: Value = payload
        .parse()
        .expect("decode standalone search 404 payload");
    assert_eq!(payload["endpoint"], "/v1/alpha/search");
    assert_ne!(payload["failureKind"], "oauth_unsupported_route");

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_standalone_search_keeps_structured_record_when_raw_logging_disabled()
 {
    #[derive(Debug, sqlx::FromRow)]
    struct InvocationRow {
        status: Option<String>,
        payload: Option<String>,
        request_raw_path: Option<String>,
        response_raw_path: Option<String>,
        t_total_ms: Option<f64>,
    }

    let (upstream_base, _attempts, upstream_handle) = spawn_pool_retry_upstream(&[]).await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
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
            response_body_logging_enabled: Some(false),
            encrypted_session_owner_routing_enabled: None,
            enabled_models: default_enabled_preset_models(),
        }),
    )
    .await
    .expect("disable standalone search raw logging");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/alpha/search"
                .parse()
                .expect("valid standalone search uri"),
        ),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(br#"{"model":"gpt-5.4","query":"hello"}"#.to_vec()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read standalone search no-raw response");

    wait_for_codex_invocations(&state.pool, 1).await;
    let invocation = sqlx::query_as::<_, InvocationRow>(
        r#"
        SELECT status, payload, request_raw_path, response_raw_path, t_total_ms
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load standalone search no-raw invocation");
    assert_eq!(invocation.status.as_deref(), Some("success"));
    assert!(invocation.request_raw_path.is_none());
    assert!(invocation.response_raw_path.is_none());
    assert!(invocation.t_total_ms.is_some_and(|value| value >= 0.0));
    let payload: Value = serde_json::from_str(
        invocation
            .payload
            .as_deref()
            .expect("standalone search no-raw payload should be present"),
    )
    .expect("decode standalone search no-raw payload");
    assert_eq!(payload["endpoint"], "/v1/alpha/search");
    assert_eq!(payload["statusCode"], 200);

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_stops_after_three_distinct_accounts() {
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
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts")
            .fetch_one(&state.pool)
            .await
            .expect("count budget attempt rows");
        if count >= 9 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let attempt_status_rows = sqlx::query_as::<_, DistinctBudgetAttemptRow>(
        r#"
        SELECT attempt_index, distinct_account_index, same_account_retry_index, status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        "#,
    )
    .fetch_all(&state.pool)
    .await
    .expect("load attempt status rows");
    assert_eq!(attempt_status_rows.len(), 9);
    assert_eq!(attempt_status_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_status_rows[1].same_account_retry_index, 2);
    assert_eq!(attempt_status_rows[2].same_account_retry_index, 3);
    assert_eq!(attempt_status_rows[3].same_account_retry_index, 1);
    assert_eq!(attempt_status_rows[4].same_account_retry_index, 2);
    assert_eq!(attempt_status_rows[5].same_account_retry_index, 3);
    assert_eq!(attempt_status_rows[6].same_account_retry_index, 1);
    assert_eq!(attempt_status_rows[7].same_account_retry_index, 2);
    assert_eq!(attempt_status_rows[8].same_account_retry_index, 3);
    assert_eq!(attempt_status_rows[8].attempt_index, 9);
    assert_eq!(attempt_status_rows[8].distinct_account_index, 3);
    let attempts = attempts.lock().expect("lock attempt counters");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-quaternary").copied(), None);
    drop(attempts);

    let payload = load_latest_invocation_payload(&state.pool).await;
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(9));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED),
    );

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_failover_reapplies_account_fast_mode_from_original_body()
 {
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

    let now_iso = format_utc_iso(Utc::now());
    let force_remove_tag_id = insert_fast_mode_tag(
        &state.pool,
        "force-remove-primary",
        "primary",
        "force_remove",
        &now_iso,
    )
    .await;
    let fill_missing_tag_id = insert_fast_mode_tag(
        &state.pool,
        "fill-missing-normal",
        "normal",
        "fill_missing",
        &now_iso,
    )
    .await;
    create_tagged_fast_mode_account(
        &state,
        "Route Remove",
        &failing_base,
        "route-remove",
        force_remove_tag_id,
        &now_iso,
    )
    .await;
    create_tagged_fast_mode_account(
        &state,
        "Route Fill",
        &capture_base,
        "route-fill",
        fill_missing_tag_id,
        &now_iso,
    )
    .await;

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

    assert_fast_mode_failover_capture(&state.pool, &captured_requests).await;

    failing_handle.abort();
    capture_handle.abort();
}

use super::*;
