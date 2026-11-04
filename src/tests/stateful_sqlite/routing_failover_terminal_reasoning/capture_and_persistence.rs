use super::*;
use serde_json::json;

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
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts")
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
async fn capture_target_pool_standalone_search_records_one_invocation_and_rollup() {
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

    let invocation = sqlx::query_as::<_, InvocationRow>(
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

    backfill_invocation_rollup_hourly_from_sources(&state.pool)
        .await
        .expect("rebuild standalone search rollup");
    backfill_invocation_rollup_hourly_from_sources(&state.pool)
        .await
        .expect("replay standalone search rollup without duplication");
    let rollup = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        r#"
        SELECT COALESCE(SUM(total_count), 0),
               COALESCE(SUM(success_count), 0),
               COALESCE(SUM(failure_count), 0),
               COALESCE(SUM(terminal_count), 0)
        FROM invocation_rollup_hourly
        WHERE source = ?1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&state.pool)
    .await
    .expect("load standalone search rollup");
    assert_eq!(rollup, (1, 1, 0, 1));

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
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(9));
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

    let now_iso = format_utc_iso(Utc::now());
    let force_remove_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, system_key, protected, allow_cut_out, allow_cut_in,
            priority_tier, fast_mode_rewrite_mode, concurrency_limit, upstream_429_retry_enabled,
            upstream_429_max_retries, available_models_json, created_at, updated_at
        ) VALUES (?1, ?2, 0, 1, 1, 'primary', 'force_remove', 0, 0, 0, '[]', ?3, ?3)
        RETURNING id
        "#,
    )
    .bind("force-remove-primary")
    .bind(None::<String>)
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert force-remove tag");
    let _force_remove_tag_id: i64 = sqlx::query_scalar("SELECT id FROM pool_tags WHERE name = ?1")
        .bind("force-remove-primary")
        .fetch_one(&state.pool)
        .await
        .expect("load force-remove tag id");
    let fill_missing_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, system_key, protected, allow_cut_out, allow_cut_in,
            priority_tier, fast_mode_rewrite_mode, concurrency_limit, upstream_429_retry_enabled,
            upstream_429_max_retries, available_models_json, created_at, updated_at
        ) VALUES (?1, ?2, 0, 1, 1, 'normal', 'fill_missing', 0, 0, 0, '[]', ?3, ?3)
        RETURNING id
        "#,
    )
    .bind("fill-missing-normal")
    .bind(None::<String>)
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert fill-missing tag");
    let _fill_missing_tag_id: i64 = sqlx::query_scalar("SELECT id FROM pool_tags WHERE name = ?1")
        .bind("fill-missing-normal")
        .fetch_one(&state.pool)
        .await
        .expect("load fill-missing tag id");

    let first_account_payload = serde_json::from_value::<CreateApiKeyAccountRequest>(json!({
        "displayName": "Route Remove",
        "upstreamBaseUrl": failing_base,
        "apiKey": "route-remove",
    }))
    .expect("deserialize first api-key account payload");
    let Json(_first_account) = create_api_key_account(
        State(state.clone()),
        HeaderMap::new(),
        Json(first_account_payload),
    )
    .await
    .expect("create first pool account");
    let first_account_id: i64 =
        sqlx::query_scalar("SELECT id FROM pool_upstream_accounts WHERE display_name = ?1")
            .bind("Route Remove")
            .fetch_one(&state.pool)
            .await
            .expect("load first pool account id");
    restore_test_legacy_api_key_group(
        &state.pool,
        first_account_id,
        test_required_group_name(),
        false,
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(first_account_id)
    .bind(force_remove_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach force-remove tag");
    let second_account_payload = serde_json::from_value::<CreateApiKeyAccountRequest>(json!({
        "displayName": "Route Fill",
        "upstreamBaseUrl": capture_base,
        "apiKey": "route-fill",
    }))
    .expect("deserialize second api-key account payload");
    let Json(_second_account) = create_api_key_account(
        State(state.clone()),
        HeaderMap::new(),
        Json(second_account_payload),
    )
    .await
    .expect("create second pool account");
    let second_account_id: i64 =
        sqlx::query_scalar("SELECT id FROM pool_upstream_accounts WHERE display_name = ?1")
            .bind("Route Fill")
            .fetch_one(&state.pool)
            .await
            .expect("load second pool account id");
    restore_test_legacy_api_key_group(
        &state.pool,
        second_account_id,
        test_required_group_name(),
        false,
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(second_account_id)
    .bind(fill_missing_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach fill-missing tag");

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
    assert_eq!(payload["requestModel"].as_str(), Some("gpt-5.4"));
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
async fn pool_route_responses_compact_retries_follow_up_accounts_before_switching() {
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
    wait_for_pool_attempt_row_count(&state.pool, 7).await;

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
    assert_eq!(attempt_rows.len(), 7);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(attempt_rows[1].distinct_account_index, 1);
    assert_eq!(attempt_rows[1].same_account_retry_index, 2);
    assert_eq!(attempt_rows[2].distinct_account_index, 1);
    assert_eq!(attempt_rows[2].same_account_retry_index, 3);
    assert_eq!(attempt_rows[3].distinct_account_index, 2);
    assert_eq!(attempt_rows[3].same_account_retry_index, 1);
    assert_eq!(attempt_rows[4].distinct_account_index, 2);
    assert_eq!(attempt_rows[4].same_account_retry_index, 2);
    assert_eq!(attempt_rows[5].distinct_account_index, 2);
    assert_eq!(attempt_rows[5].same_account_retry_index, 3);
    assert_eq!(attempt_rows[6].distinct_account_index, 3);
    assert_eq!(attempt_rows[6].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[6].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    );

    let attempts = attempts.lock().expect("lock compact attempt counters");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(3));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(3));
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
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(7));
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

    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 9).await;

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
    assert_eq!(attempt_rows.len(), 3);
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
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
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
async fn pool_route_fails_over_on_unsupported_model_bad_request() {
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
    let upstream_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("unsupported-model failover upstream should run");
    });
    let upstream_base = format!("http://{addr}");
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

    wait_for_pool_attempt_terminal_observation(
        &state.pool,
        &cvm_id,
        1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
        400,
    )
    .await;
    wait_for_pool_attempt_terminal_observation(
        &state.pool,
        &cvm_id,
        2,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        200,
    )
    .await;
    let attempt_rows = sqlx::query_as::<_, (i64, Option<i64>, Option<String>)>(
        r#"
        SELECT attempt_index, http_status, failure_kind
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
        ORDER BY attempt_index ASC
        "#,
    )
    .bind(&cvm_id)
    .fetch_all(&state.pool)
    .await
    .expect("load unsupported-model attempt rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0].0, 1);
    assert_eq!(attempt_rows[0].1, Some(400));
    assert!(attempt_rows[0].2.is_some());
    assert_eq!(attempt_rows[1].0, 2);
    assert_eq!(attempt_rows[1].1, Some(200));
    assert_eq!(attempt_rows[1].2, None);

    let attempts = attempts.lock().expect("lock unsupported-model attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);

    let primary_tags = sqlx::query_scalar::<_, String>(
        r#"
        SELECT tag.system_key
        FROM pool_tags tag
        JOIN pool_upstream_account_tags link ON link.tag_id = tag.id
        WHERE link.account_id = ?1
          AND tag.system_key IS NOT NULL
        ORDER BY tag.system_key ASC
        "#,
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
    let primary_model_route = load_model_routing_states(&state.pool, primary_id)
        .await
        .expect("load primary model route")
        .into_iter()
        .find(|route| route.model == "gpt-5.5")
        .expect("primary model route should be learned dynamically");
    assert_eq!(primary_model_route.failure_count, 1);
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-unsupported-model-failover").await,
        Some(secondary_id)
    );

    upstream_handle.abort();
}
