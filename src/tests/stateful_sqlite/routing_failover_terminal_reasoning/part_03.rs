#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_fast_fill_missing_large_body_recomputes_content_length()
 {
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
    let response = proxy_pool_json(&state, "/v1/responses", request_body).await;

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

    assert_fast_success_persistence(&state).await;

    capture_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_openai_v1_responses_fast_fill_missing_transport_failure_persists_rewritten_request_raw()
 {
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
    let response = proxy_pool_json(&state, "/v1/responses", request_body).await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failed large fast response body");
    let response_text = String::from_utf8_lossy(&response_body);
    assert!(
        response_text.contains("failed to contact upstream"),
        "unexpected failed large fast response body: {response_text}"
    );

    assert_fast_failure_persistence(&state).await;
}

#[tokio::test]
pub(crate) async fn pool_route_responses_compact_retries_follow_up_accounts_before_switching() {
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
    let response = proxy_pool_json(&state, "/v1/responses/compact", request_body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact retry body");

    wait_for_codex_invocations(&state.pool, 1).await;
    assert_compact_retry_observations(&state.pool, &attempts).await;

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_route_compact_502_returns_cvm_id_and_attempt_observations() {
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
    let response = proxy_pool_json(&state, "/v1/responses/compact", request_body).await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let cvm_id = response
        .headers()
        .get(CVM_INVOKE_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .expect("cvm id header should be present");
    assert!(proxy_invoke_id_has_short_format(&cvm_id));
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact 502 response");
    let payload: Value = serde_json::from_slice(&body).expect("decode compact 502 payload");
    assert_eq!(payload["cvmId"].as_str(), Some(cvm_id.as_str()));
    assert_eq!(
        payload["error"].as_str(),
        Some("pool distinct-account retry budget exhausted"),
    );

    assert_compact_unsupported_observations(&state, &cvm_id, &attempts).await;

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn pool_route_chat_completions_keeps_three_attempts_for_follow_up_accounts() {
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
    let response = proxy_pool_json(&state, "/v1/chat/completions", request_body).await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read chat completions retry body");

    wait_for_codex_invocations(&state.pool, 1).await;
    assert_chat_retry_observations(&state.pool, &attempts).await;

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_preserves_auth_failure_terminal_reason() {
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
pub(crate) async fn pool_route_fails_over_on_unsupported_model_bad_request() {
    let (upstream_base, attempts, upstream_handle) =
        spawn_unsupported_model_failover_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.5","input":"hello","stickyKey":"sticky-unsupported-model-failover"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let cvm_id = response
        .headers()
        .get(CVM_INVOKE_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .expect("failover response should expose x-cvm-invoke-id");
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failover success body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failover success body");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");

    assert_unsupported_model_failover_persistence(
        &state,
        &cvm_id,
        primary_id,
        secondary_id,
        &attempts,
    )
    .await;

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn capture_target_pool_route_timeout_switches_to_alternate_upstream_route() {
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
    assert_timeout_switch_attempts(
        &state.pool,
        slow_id,
        fast_id,
        &slow_upstream_base,
        &fast_upstream_base,
    )
    .await;
    assert_timeout_switch_payload(&state.pool).await;

    slow_upstream_handle.abort();
    fast_upstream_handle.abort();
}

use super::*;
#[derive(Debug, sqlx::FromRow)]
struct FastRequestRow {
    status: Option<String>,
    error_message: Option<String>,
    payload: Option<String>,
    request_raw_path: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct RetryAttemptRow {
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: String,
}

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

async fn load_part_03_latest_payload(pool: &SqlitePool) -> Value {
    let payload: Option<String> =
        sqlx::query_scalar("SELECT payload FROM codex_invocations ORDER BY id DESC LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("load latest invocation payload");
    serde_json::from_str(
        payload
            .as_deref()
            .expect("invocation payload should be present"),
    )
    .expect("decode invocation payload")
}

async fn proxy_pool_json(state: &Arc<AppState>, path: &str, request_body: Vec<u8>) -> Response {
    proxy_openai_v1(
        State(state.clone()),
        OriginalUri(path.parse().expect("valid proxy uri")),
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
    .await
}

async fn assert_fast_success_persistence(state: &Arc<AppState>) {
    let row = sqlx::query_as::<_, FastRequestRow>(
        "SELECT NULL AS status, NULL AS error_message, payload, request_raw_path \
         FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load persisted large fast row");
    let payload: Value =
        serde_json::from_str(row.payload.as_deref().expect("payload should exist"))
            .expect("decode persisted large fast payload");
    assert_eq!(payload["requestedServiceTier"].as_str(), Some("priority"));
    assert_rewritten_fast_request_raw(state, &row, "large fast");
}

fn assert_rewritten_fast_request_raw(state: &Arc<AppState>, row: &FastRequestRow, label: &str) {
    let request_raw = read_proxy_raw_bytes(
        row.request_raw_path
            .as_deref()
            .unwrap_or_else(|| panic!("{label} request raw path should exist")),
        state.config.database_path.parent(),
    )
    .unwrap_or_else(|error| panic!("read {label} request raw: {error}"));
    let request_payload: Value = serde_json::from_slice(&request_raw)
        .unwrap_or_else(|error| panic!("decode {label} request raw: {error}"));
    assert_eq!(request_payload["service_tier"].as_str(), Some("priority"));
    assert!(request_payload.get("serviceTier").is_none());
}

async fn assert_fast_failure_persistence(state: &Arc<AppState>) {
    wait_for_codex_invocations(&state.pool, 1).await;
    let row = sqlx::query_as::<_, FastRequestRow>(
        "SELECT status, error_message, payload, request_raw_path \
         FROM codex_invocations ORDER BY id DESC LIMIT 1",
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
    let payload: Value =
        serde_json::from_str(row.payload.as_deref().expect("payload should exist"))
            .expect("decode failed large fast payload");
    assert_eq!(payload["requestModel"].as_str(), Some("gpt-5.4"));
    assert_eq!(payload["requestedServiceTier"].as_str(), Some("priority"));
    assert_eq!(
        payload["failureKind"].as_str(),
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    assert_rewritten_fast_request_raw(state, &row, "failed large fast");
}

async fn load_retry_attempt_rows(pool: &SqlitePool, expected: usize) -> Vec<RetryAttemptRow> {
    wait_for_pool_attempt_row_count(pool, expected as i64).await;
    sqlx::query_as::<_, RetryAttemptRow>(
        "SELECT attempt_index, distinct_account_index, same_account_retry_index, status \
         FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(pool)
    .await
    .expect("load retry attempt rows")
}

fn assert_attempt_counters(
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    expected: &[(&str, usize)],
) {
    let attempts = attempts.lock().expect("lock attempt counters");
    for (authorization, count) in expected {
        assert_eq!(attempts.get(*authorization).copied(), Some(*count));
    }
}

async fn assert_compact_retry_observations(
    pool: &SqlitePool,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    let rows = load_retry_attempt_rows(pool, 7).await;
    assert_eq!(rows.len(), 7);
    let expected_indexes = [(1, 1), (1, 2), (1, 3), (2, 1), (2, 2), (2, 3), (3, 1)];
    for (row, (distinct, retry)) in rows.iter().zip(expected_indexes) {
        assert_eq!(row.distinct_account_index, distinct);
        assert_eq!(row.same_account_retry_index, retry);
    }
    assert_eq!(rows[6].attempt_index, 7);
    assert_eq!(rows[6].status, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_attempt_counters(
        attempts,
        &[
            ("Bearer upstream-primary", 3),
            ("Bearer upstream-secondary", 3),
            ("Bearer upstream-tertiary", 1),
        ],
    );
    let payload = load_part_03_latest_payload(pool).await;
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(7));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert!(payload["poolAttemptTerminalReason"].is_null());
}

async fn assert_chat_retry_observations(
    pool: &SqlitePool,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    let rows = load_retry_attempt_rows(pool, 6).await;
    assert_eq!(rows.len(), 6);
    let expected_indexes = [(1, 1), (1, 2), (1, 3), (2, 1), (2, 2), (2, 3)];
    for (row, (distinct, retry)) in rows.iter().zip(expected_indexes) {
        assert_eq!(row.distinct_account_index, distinct);
        assert_eq!(row.same_account_retry_index, retry);
    }
    assert_eq!(rows[5].attempt_index, 6);
    assert_eq!(rows[5].status, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_attempt_counters(
        attempts,
        &[
            ("Bearer upstream-primary", 3),
            ("Bearer upstream-secondary", 3),
        ],
    );
    let payload = load_part_03_latest_payload(pool).await;
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(6));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
    assert!(payload["poolAttemptTerminalReason"].is_null());
}

async fn assert_compact_unsupported_observations(
    state: &Arc<AppState>,
    cvm_id: &str,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 9).await;
    let invocation_status: Option<String> =
        sqlx::query_scalar("SELECT status FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
            .bind(cvm_id)
            .fetch_optional(&state.pool)
            .await
            .expect("load compact invocation status");
    assert_eq!(invocation_status.as_deref(), Some("http_502"));
    let Json(rows) = fetch_invocation_pool_attempts(
        State(state.clone()),
        axum::extract::Path(cvm_id.to_string()),
    )
    .await
    .expect("fetch invocation pool attempts");
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| {
        row.compact_support_status.as_deref() == Some(COMPACT_SUPPORT_STATUS_UNSUPPORTED)
    }));
    assert!(
        rows[0]
            .compact_support_reason
            .as_deref()
            .is_some_and(|value| value.contains("No available channel for model"))
    );
    let support_states = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT compact_support_status, compact_support_reason \
         FROM pool_upstream_accounts ORDER BY id ASC",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load compact support states");
    assert_eq!(support_states.len(), 3);
    assert!(
        support_states
            .iter()
            .all(|row| row.0 == COMPACT_SUPPORT_STATUS_UNSUPPORTED)
    );
    assert_attempt_counters(
        attempts,
        &[
            ("Bearer upstream-primary", 1),
            ("Bearer upstream-secondary", 1),
            ("Bearer upstream-tertiary", 1),
        ],
    );
}

async fn unsupported_model_failover_upstream(
    attempts: Arc<StdMutex<HashMap<String, usize>>>,
    headers: HeaderMap,
) -> Response {
    let authorization = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let attempt = {
        let mut attempts = attempts.lock().expect("lock unsupported-model attempts");
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

async fn spawn_unsupported_model_failover_upstream() -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    tokio::task::JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let app = Router::new().route(
        "/v1/responses",
        post({
            let attempts = attempts.clone();
            move |headers| unsupported_model_failover_upstream(attempts.clone(), headers)
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind unsupported-model failover upstream");
    let addr = listener
        .local_addr()
        .expect("unsupported-model failover upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("unsupported-model failover upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

async fn assert_unsupported_model_failover_persistence(
    state: &Arc<AppState>,
    cvm_id: &str,
    primary_id: i64,
    secondary_id: i64,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    wait_for_pool_attempt_terminal_observation(
        &state.pool,
        cvm_id,
        1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
        400,
    )
    .await;
    wait_for_pool_attempt_terminal_observation(
        &state.pool,
        cvm_id,
        2,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        200,
    )
    .await;
    let rows = sqlx::query_as::<_, (i64, Option<i64>, Option<String>)>(
        "SELECT attempt_index, http_status, failure_kind \
         FROM pool_upstream_request_attempts WHERE invoke_id = ?1 ORDER BY attempt_index ASC",
    )
    .bind(cvm_id)
    .fetch_all(&state.pool)
    .await
    .expect("load unsupported-model attempt rows");
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].0, rows[0].1), (1, Some(400)));
    assert!(rows[0].2.is_some());
    assert_eq!(
        (rows[1].0, rows[1].1, rows[1].2.as_deref()),
        (2, Some(200), None)
    );
    assert_attempt_counters(
        attempts,
        &[
            ("Bearer upstream-primary", 1),
            ("Bearer upstream-secondary", 1),
        ],
    );
    assert_unsupported_model_routing_state(state, primary_id, secondary_id).await;
}

async fn assert_unsupported_model_routing_state(
    state: &Arc<AppState>,
    primary_id: i64,
    secondary_id: i64,
) {
    let primary_tags = sqlx::query_scalar::<_, String>(
        "SELECT tag.system_key FROM pool_tags tag \
         JOIN pool_upstream_account_tags link ON link.tag_id = tag.id \
         WHERE link.account_id = ?1 AND tag.system_key IS NOT NULL ORDER BY tag.system_key ASC",
    )
    .bind(primary_id)
    .fetch_all(&state.pool)
    .await
    .expect("load primary account system tags");
    assert!(
        !primary_tags
            .iter()
            .any(|tag| tag == "unsupported_model:gpt-5.5")
    );
    let route = load_model_routing_states(&state.pool, primary_id)
        .await
        .expect("load primary model route")
        .into_iter()
        .find(|route| route.model == "gpt-5.5")
        .expect("primary model route should be learned dynamically");
    assert_eq!(route.failure_count, 1);
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-unsupported-model-failover").await,
        Some(secondary_id)
    );
}

async fn assert_timeout_switch_attempts(
    pool: &SqlitePool,
    slow_id: i64,
    fast_id: i64,
    slow_base: &str,
    fast_base: &str,
) {
    wait_for_pool_upstream_request_attempts(pool, 2).await;
    let rows = sqlx::query_as::<_, AttemptRouteRow>(
        "SELECT upstream_account_id, upstream_route_key, attempt_index, \
         distinct_account_index, same_account_retry_index, status, failure_kind \
         FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(pool)
    .await
    .expect("load timeout-switch attempt rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].upstream_account_id, Some(slow_id));
    assert_eq!(rows[0].attempt_index, 1);
    assert_eq!(rows[0].distinct_account_index, 1);
    assert_eq!(rows[0].same_account_retry_index, 1);
    assert_eq!(
        rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        rows[0].failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
    );
    assert_eq!(rows[1].upstream_account_id, Some(fast_id));
    assert_eq!(
        (rows[1].attempt_index, rows[1].distinct_account_index),
        (2, 2)
    );
    assert_eq!(rows[1].same_account_retry_index, 1);
    assert_eq!(rows[1].status, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_eq!(
        rows[0].upstream_route_key.as_deref(),
        Some(canonical_pool_upstream_route_key(&Url::parse(slow_base).expect("slow url")).as_str())
    );
    assert_eq!(
        rows[1].upstream_route_key.as_deref(),
        Some(canonical_pool_upstream_route_key(&Url::parse(fast_base).expect("fast url")).as_str())
    );
}

async fn assert_timeout_switch_payload(pool: &SqlitePool) {
    let payload = load_part_03_latest_payload(pool).await;
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(2));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
    assert!(payload["poolAttemptTerminalReason"].is_null());
}
