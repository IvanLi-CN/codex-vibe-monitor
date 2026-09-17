#[tokio::test]
async fn capture_target_pool_route_persists_attempt_rows_and_summary_fields() {
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

    assert_persisted_capture_attempts(&state.pool, primary_id).await;

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_pool_standalone_search_records_one_invocation_and_rollup() {
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

    assert_standalone_search_invocation(&state, &attempts, response).await;
    upstream_handle.abort();
}

async fn assert_standalone_search_invocation(
    state: &Arc<AppState>,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    response: axum::response::Response,
) {
    #[derive(Debug, sqlx::FromRow)]
    struct InvocationRow {
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
    let invocation = sqlx::query_as::<_, InvocationRow>("SELECT model, status, input_tokens, output_tokens, total_tokens, cost, payload, request_raw_path, response_raw_path, t_total_ms FROM codex_invocations ORDER BY id DESC LIMIT 1").fetch_one(&state.pool).await.expect("load standalone search invocation");
    assert_eq!(invocation.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(invocation.status.as_deref(), Some("success"));
    assert!(
        invocation.input_tokens.is_none()
            && invocation.output_tokens.is_none()
            && invocation.total_tokens.is_none()
            && invocation.cost.is_none()
    );
    assert!(invocation.t_total_ms.is_some_and(|value| value >= 0.0));
    assert!(invocation.request_raw_path.is_some() && invocation.response_raw_path.is_some());
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
    assert!(
        payload["inputTokens"].is_null()
            && payload["outputTokens"].is_null()
            && payload["totalTokens"].is_null()
    );
    backfill_invocation_rollup_hourly_from_sources(&state.pool)
        .await
        .expect("rebuild standalone search rollup");
    backfill_invocation_rollup_hourly_from_sources(&state.pool)
        .await
        .expect("replay standalone search rollup without duplication");
    let rollup: (i64, i64, i64, i64) = sqlx::query_as("SELECT COALESCE(SUM(total_count), 0), COALESCE(SUM(success_count), 0), COALESCE(SUM(failure_count), 0), COALESCE(SUM(terminal_count), 0) FROM invocation_rollup_hourly WHERE source = ?1").bind(SOURCE_PROXY).fetch_one(&state.pool).await.expect("load standalone search rollup");
    assert_eq!(rollup, (1, 1, 0, 1));
    assert_eq!(
        attempts
            .lock()
            .expect("lock standalone search attempts")
            .get("Bearer upstream-primary")
            .copied(),
        Some(3)
    );
}

#[tokio::test]
async fn capture_target_pool_standalone_search_records_upstream_404_as_failure() {
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
async fn capture_target_pool_standalone_search_keeps_structured_record_when_raw_logging_disabled() {
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

    assert_three_distinct_account_exhaustion(&state.pool, &attempts).await;

    upstream_handle.abort();
}

async fn assert_three_distinct_account_exhaustion(
    pool: &SqlitePool,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
) {
    wait_for_codex_invocations(pool, 1).await;
    for _ in 0..20 {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts")
            .fetch_one(pool)
            .await
            .expect("count budget attempt rows");
        if count >= 9 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let rows = sqlx::query_as::<_, (i64, i64, i64, String, Option<String>)>(
        "SELECT attempt_index, distinct_account_index, same_account_retry_index, status, failure_kind FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(pool)
    .await
    .expect("load attempt status rows");
    assert_eq!(rows.len(), 9);
    for (index, expected) in [1, 2, 3, 1, 2, 3, 1, 2, 3].into_iter().enumerate() {
        assert_eq!(rows[index].2, expected);
    }
    assert_eq!(rows[8].0, 9);
    assert_eq!(rows[8].1, 3);
    let attempts = attempts.lock().expect("lock attempt counters");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-tertiary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-quaternary").copied(), None);
    drop(attempts);
    let payload: Value = sqlx::query_scalar::<_, Option<String>>(
        "SELECT payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load exhausted invocation payload")
    .and_then(|payload| serde_json::from_str(&payload).ok())
    .expect("decode exhausted payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(9));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(3));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED),
    );
}

struct FastModeFailoverFixture {
    state: Arc<AppState>,
    failing_handle: JoinHandle<()>,
    capture_handle: JoinHandle<()>,
    captured_requests: Arc<Mutex<Vec<Value>>>,
    failing_base: String,
    capture_base: String,
}

async fn create_fast_mode_failover_fixture() -> FastModeFailoverFixture {
    let (failing_base, _attempts, failing_handle) = spawn_pool_static_failure_responses_upstream(
        &[("Bearer route-remove", StatusCode::INTERNAL_SERVER_ERROR)],
    )
    .await;
    let (capture_base, captured_requests, capture_handle) =
        spawn_capture_target_body_upstream().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    FastModeFailoverFixture {
        state,
        failing_handle,
        capture_handle,
        captured_requests,
        failing_base,
        capture_base,
    }
}

async fn create_fast_mode_tags(state: &Arc<AppState>) -> (i64, i64) {
    let now_iso = format_utc_iso(Utc::now());
    let force_remove_tag_id: i64 = sqlx::query_scalar(
        "INSERT INTO pool_tags
         (name, system_key, protected, allow_cut_out, allow_cut_in, priority_tier,
          fast_mode_rewrite_mode, concurrency_limit, upstream_429_retry_enabled,
          upstream_429_max_retries, available_models_json, created_at, updated_at)
         VALUES (?1, ?2, 0, 1, 1, 'primary', 'force_remove', 0, 0, 0, '[]', ?3, ?3)
         RETURNING id",
    )
    .bind("force-remove-primary")
    .bind(None::<String>)
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert force-remove tag");
    let fill_missing_tag_id: i64 = sqlx::query_scalar(
        "INSERT INTO pool_tags
         (name, system_key, protected, allow_cut_out, allow_cut_in, priority_tier,
          fast_mode_rewrite_mode, concurrency_limit, upstream_429_retry_enabled,
          upstream_429_max_retries, available_models_json, created_at, updated_at)
         VALUES (?1, ?2, 0, 1, 1, 'normal', 'fill_missing', 0, 0, 0, '[]', ?3, ?3)
         RETURNING id",
    )
    .bind("fill-missing-normal")
    .bind(None::<String>)
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert fill-missing tag");
    (force_remove_tag_id, fill_missing_tag_id)
}

async fn seed_fast_mode_failover_accounts(
    fixture: &FastModeFailoverFixture,
    force_remove_tag_id: i64,
    fill_missing_tag_id: i64,
) {
    let now_iso = format_utc_iso(Utc::now());
    for (display_name, base_url, api_key, tag_id) in [
        (
            "Route Remove",
            fixture.failing_base.as_str(),
            "route-remove",
            force_remove_tag_id,
        ),
        (
            "Route Fill",
            fixture.capture_base.as_str(),
            "route-fill",
            fill_missing_tag_id,
        ),
    ] {
        let payload = serde_json::from_value::<CreateApiKeyAccountRequest>(json!({
            "displayName": display_name,
            "upstreamBaseUrl": base_url,
            "apiKey": api_key,
        }))
        .expect("deserialize api-key account payload");
        let Json(account) = create_api_key_account(
            State(fixture.state.clone()),
            HeaderMap::new(),
            Json(payload),
        )
        .await
        .expect("create pool account");
        restore_test_legacy_api_key_group(
            &fixture.state.pool,
            account.summary.id,
            test_required_group_name(),
            false,
        )
        .await;
        sqlx::query(
            "INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)",
        )
        .bind(account.summary.id)
        .bind(tag_id)
        .bind(&now_iso)
        .execute(&fixture.state.pool)
        .await
        .expect("attach fast-mode tag");
    }
}

async fn assert_fast_mode_failover_persistence(fixture: &FastModeFailoverFixture) {
    wait_for_codex_invocations(&fixture.state.pool, 1).await;
    wait_for_pool_upstream_request_attempts(&fixture.state.pool, 2).await;
    let captured = fixture.captured_requests.lock().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0]["serviceTier"].as_str(), Some("flex"));
    assert!(captured[0].get("service_tier").is_none());
    drop(captured);
    let payload: Value = sqlx::query_scalar::<_, Option<String>>(
        "SELECT payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&fixture.state.pool)
    .await
    .expect("load persisted failover payload")
    .and_then(|payload| serde_json::from_str(&payload).ok())
    .expect("decode persisted failover payload");
    assert_eq!(payload["requestedServiceTier"].as_str(), Some("flex"));
    assert!(
        payload["poolAttemptCount"]
            .as_i64()
            .is_some_and(|count| count >= 2)
    );
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
}

#[tokio::test]
async fn pool_openai_v1_responses_failover_reapplies_account_fast_mode_from_original_body() {
    let fixture = create_fast_mode_failover_fixture().await;
    let (force_remove_tag_id, fill_missing_tag_id) = create_fast_mode_tags(&fixture.state).await;
    seed_fast_mode_failover_accounts(&fixture, force_remove_tag_id, fill_missing_tag_id).await;
    let response = proxy_openai_v1(
        State(fixture.state.clone()),
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
        Body::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.3-codex",
                "stream": false,
                "serviceTier": "flex",
                "input": "hello"
            }))
            .expect("serialize failover request body"),
        ),
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
    assert_fast_mode_failover_persistence(&fixture).await;
    fixture.failing_handle.abort();
    fixture.capture_handle.abort();
}
