#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_large_nonstream_json_error_preserves_prefixed_metadata() {
    #[derive(sqlx::FromRow)]
    struct PersistedErrorRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-prefixed-json-error");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=large-prefixed-json-error"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":false,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read large prefixed json error response body");

    let mut row: Option<PersistedErrorRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedErrorRow>(
            r#"
            SELECT
                status,
                error_message,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query large prefixed json error row");
        if row
            .as_ref()
            .and_then(|record| record.error_message.as_deref())
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("large prefixed json error row should exist");
    assert_eq!(row.status.as_deref(), Some("http_400"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|message| message.contains("prefix metadata should survive"))
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode large prefixed json error payload summary");
    assert_eq!(payload["serviceTier"].as_str(), Some("priority"));
    assert!(
        payload["usageMissingReason"]
            .as_str()
            .is_some_and(|reason| reason.contains(PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED))
    );
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
async fn proxy_capture_target_nonstream_usage_survives_response_raw_truncation() {
    #[derive(sqlx::FromRow)]
    struct PersistedCompactRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
        response_raw_path: Option<String>,
        response_raw_size: Option<i64>,
        response_raw_truncated: i64,
        response_raw_truncated_reason: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-compact-truncated-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    config.proxy_raw_max_bytes = Some(96);
    let state = test_state_from_config(config, true).await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "previous_response_id": "resp_prev_truncated",
        "input": [{ "role": "user", "content": "compact this thread" }]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses/compact".parse().expect("valid compact uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact proxy response body");

    let mut row: Option<PersistedCompactRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedCompactRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                response_raw_path,
                response_raw_size,
                response_raw_truncated,
                response_raw_truncated_reason,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query truncated compact capture row");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("truncated compact capture row should exist");

    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(139));
    assert_eq!(row.output_tokens, Some(438));
    assert_eq!(row.total_tokens, Some(577));
    assert_eq!(row.response_raw_truncated, 1);
    assert_eq!(
        row.response_raw_truncated_reason.as_deref(),
        Some("max_bytes_exceeded")
    );
    assert!(
        row.response_raw_size.is_some_and(|size| size > 96),
        "stored raw size should still reflect the full response length"
    );

    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read truncated compact raw response");
    assert!(
        raw_bytes.len() <= 96,
        "persisted compact raw bytes should respect the configured cap"
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode payload summary");
    assert!(payload["usageMissingReason"].is_null());

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
async fn resolve_default_source_scope_always_all() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let scope_before = resolve_default_source_scope(&pool)
        .await
        .expect("scope before insert");
    assert_eq!(scope_before, InvocationSourceScope::All);

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, raw_response
        )
        VALUES (?1, ?2, ?3, ?4)
        "#,
    )
    .bind("proxy-test-1")
    .bind("2026-02-22 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert proxy invocation");

    let scope_after = resolve_default_source_scope(&pool)
        .await
        .expect("scope after insert");
    assert_eq!(scope_after, InvocationSourceScope::All);
}

#[tokio::test]
async fn list_invocations_projects_payload_context_fields() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-context-1")
    .bind("2026-02-25 10:00:00")
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind(
        r#"{"endpoint":"/v1/responses","failureKind":"upstream_stream_error","requesterIp":"198.51.100.77","promptCacheKey":"pck-list-1","routeMode":"pool","upstreamAccountId":17,"upstreamAccountName":"pool-account-17","responseContentEncoding":"gzip, br","requestedServiceTier":"priority","serviceTier":null,"service_tier":"priority","proxyDisplayName":"jp-relay-01","proxyWeightDelta":-0.68,"reasoningEffort":"high"}"#,
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert proxy invocation");

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(10),
            model: None,
            status: None,
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should succeed");

    let record = response
        .records
        .into_iter()
        .find(|item| item.invoke_id == "proxy-context-1")
        .expect("inserted invocation should be present");
    assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(
        record.failure_kind.as_deref(),
        Some("upstream_stream_error")
    );
    assert_eq!(record.requester_ip.as_deref(), Some("198.51.100.77"));
    assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-list-1"));
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
    assert_eq!(record.requested_service_tier.as_deref(), Some("priority"));
    assert_eq!(record.service_tier.as_deref(), Some("priority"));
    assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-01"));
    assert_eq!(record.proxy_weight_delta, Some(-0.68));
    assert_eq!(record.reasoning_effort.as_deref(), Some("high"));
}

#[tokio::test]
async fn list_invocations_filters_by_sticky_key_and_upstream_account_id() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, payload) in [
        (
            "sticky-filter-fallback",
            json!({
                "promptCacheKey": "sticky-filter-key",
                "upstreamAccountId": 7,
            }),
        ),
        (
            "sticky-filter-wrong-account",
            json!({
                "stickyKey": "sticky-filter-key",
                "upstreamAccountId": 8,
            }),
        ),
        (
            "sticky-filter-wrong-key",
            json!({
                "stickyKey": "other-sticky-key",
                "upstreamAccountId": 7,
            }),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-11 10:00:00")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(payload.to_string())
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert sticky filter invocation");
    }

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            sticky_key: Some("sticky-filter-key".to_string()),
            upstream_account_id: Some(7),
            page: Some(1),
            page_size: Some(20),
            ..Default::default()
        }),
    )
    .await
    .expect("sticky key + upstream account filter should succeed");

    assert_eq!(response.total, 1);
    assert_eq!(response.records[0].invoke_id, "sticky-filter-fallback");
    assert_eq!(
        response.records[0].prompt_cache_key.as_deref(),
        Some("sticky-filter-key")
    );
    assert_eq!(response.records[0].upstream_account_id, Some(7));
}

#[tokio::test]
async fn invocation_queries_filter_upstream_scope_and_treat_legacy_rows_as_external() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, model, payload) in [
        (
            "scope-internal",
            "model-internal",
            Some(json!({
                "upstreamScope": "internal",
                "routeMode": "pool",
                "stickyKey": "sticky-int-1",
                "upstreamAccountId": 7,
                "upstreamAccountName": "pool-account-a"
            })),
        ),
        (
            "scope-external",
            "model-external",
            Some(json!({
                "upstreamScope": "external",
                "routeMode": "forward_proxy",
                "proxyDisplayName": "proxy-a"
            })),
        ),
        (
            "scope-legacy",
            "model-legacy",
            Some(json!({
                "proxyDisplayName": "proxy-b"
            })),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                model,
                payload,
                status,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-11 10:00:00")
        .bind(SOURCE_PROXY)
        .bind(model)
        .bind(payload.map(|value| value.to_string()))
        .bind("success")
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert invocation row");
    }

    let Json(internal_list) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            upstream_scope: Some("internal".to_string()),
            page: Some(1),
            page_size: Some(20),
            ..Default::default()
        }),
    )
    .await
    .expect("internal scope query should succeed");
    assert_eq!(internal_list.total, 1);
    assert_eq!(internal_list.records[0].invoke_id, "scope-internal");
    assert_eq!(internal_list.records[0].prompt_cache_key, None);

    let Json(external_summary) = fetch_invocation_summary(
        State(state.clone()),
        Query(ListQuery {
            upstream_scope: Some("external".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("external scope summary should succeed");
    assert_eq!(external_summary.total_count, 2);
    assert_eq!(external_summary.success_count, 2);

    let Json(internal_suggestions) = fetch_invocation_suggestions(
        State(state),
        Query(ListQuery {
            upstream_scope: Some("internal".to_string()),
            suggest_field: Some("model".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("internal scope suggestions should succeed");

    let values = internal_suggestions
        .model
        .items
        .iter()
        .map(|item| item.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(values, vec!["model-internal"]);
}

#[tokio::test]
async fn list_invocations_response_omits_raw_expires_at_field() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("proxy-no-raw-expires")
    .bind("2026-02-25 10:02:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert proxy invocation");

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(10),
            model: None,
            status: None,
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should succeed");

    let record = response
        .records
        .into_iter()
        .find(|item| item.invoke_id == "proxy-no-raw-expires")
        .expect("inserted invocation should be present");
    let json = serde_json::to_value(&record).expect("serialize invocation record");
    assert!(
        json.get("rawExpiresAt").is_none(),
        "rawExpiresAt should not be exposed by the API anymore"
    );
}

#[tokio::test]
async fn list_invocations_tolerates_malformed_payload_json() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-context-malformed")
    .bind("2026-02-25 10:01:00")
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind("not-json")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert malformed payload invocation");

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(10),
            model: None,
            status: None,
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should tolerate malformed payload");

    let record = response
        .records
        .into_iter()
        .find(|item| item.invoke_id == "proxy-context-malformed")
        .expect("inserted invocation should be present");
    assert_eq!(record.endpoint, None);
    assert_eq!(record.failure_kind, None);
    assert_eq!(record.requester_ip, None);
    assert_eq!(record.prompt_cache_key, None);
    assert_eq!(record.requested_service_tier, None);
    assert_eq!(record.service_tier, None);
    assert_eq!(record.proxy_weight_delta, None);
    assert_eq!(record.reasoning_effort, None);
}

#[tokio::test]
async fn list_invocations_ignores_non_numeric_proxy_weight_delta() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-context-delta-text")
    .bind("2026-02-25 10:02:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(
        "{\"endpoint\":\"/v1/responses\",\"proxyDisplayName\":\"jp-relay-02\",\"proxyWeightDelta\":\"abc\"}",
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert non-numeric proxyWeightDelta invocation");

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(10),
            model: None,
            status: None,
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should ignore non-numeric proxyWeightDelta");

    let record = response
        .records
        .into_iter()
        .find(|item| item.invoke_id == "proxy-context-delta-text")
        .expect("inserted invocation should be present");
    assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-02"));
    assert_eq!(record.proxy_weight_delta, None);
}

#[tokio::test]
async fn list_invocations_preserves_historical_xy_records() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            total_tokens,
            cost,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("xy-history-1")
    .bind("2026-02-25 10:03:00")
    .bind(SOURCE_XY)
    .bind("gpt-5.3-codex")
    .bind(16_i64)
    .bind(0.0042_f64)
    .bind("success")
    .bind(r#"{"serviceTier":"priority"}"#)
    .bind(r#"{"legacy":true}"#)
    .execute(&state.pool)
    .await
    .expect("insert historical xy invocation");

    let Json(response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            limit: Some(10),
            model: None,
            status: None,
            ..Default::default()
        }),
    )
    .await
    .expect("list invocations should keep historical xy rows");

    let record = response
        .records
        .into_iter()
        .find(|item| item.invoke_id == "xy-history-1")
        .expect("historical xy row should be returned");
    assert_eq!(record.source, SOURCE_XY);
    assert_eq!(record.service_tier.as_deref(), Some("priority"));
    assert_eq!(record.requested_service_tier, None);
}

#[tokio::test]
async fn list_invocations_legacy_limit_query_skips_snapshot_shape() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, occurred_at) in [
        ("legacy-stream-1", "2026-03-10 07:00:00"),
        ("legacy-stream-2", "2026-03-10 07:01:00"),
        ("legacy-stream-3", "2026-03-10 07:02:00"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert legacy stream row");
    }

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(2),
            ..Default::default()
        }),
    )
    .await
    .expect("legacy list query should succeed");

    assert_eq!(response.snapshot_id, 0);
    assert_eq!(response.total, 2);
    assert_eq!(response.page, 1);
    assert_eq!(response.page_size, 2);
    assert_eq!(response.records.len(), 2);
    assert_eq!(response.records[0].invoke_id, "legacy-stream-3");
    assert_eq!(response.records[1].invoke_id, "legacy-stream-2");
}

#[tokio::test]
async fn list_invocations_keeps_snapshot_stable_across_pagination_and_sorting() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, occurred_at, total_tokens) in [
        ("snapshot-1", "2026-03-10 08:00:00", 100_i64),
        ("snapshot-2", "2026-03-10 08:01:00", 200_i64),
        ("snapshot-3", "2026-03-10 08:02:00", 300_i64),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                total_tokens,
                status,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(total_tokens)
        .bind("success")
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert snapshot seed row");
    }

    let Json(first_page) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            page: Some(1),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("initial snapshot query should succeed");

    assert_eq!(first_page.snapshot_id, 3);
    assert_eq!(first_page.total, 3);
    assert_eq!(first_page.records.len(), 1);
    assert_eq!(first_page.records[0].invoke_id, "snapshot-3");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("snapshot-4")
    .bind("2026-03-10 08:03:00")
    .bind(SOURCE_PROXY)
    .bind(50_i64)
    .bind("success")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert newer row after snapshot");

    let Json(second_page) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            page: Some(2),
            page_size: Some(1),
            snapshot_id: Some(first_page.snapshot_id),
            ..Default::default()
        }),
    )
    .await
    .expect("second page should honor snapshot");
    assert_eq!(second_page.snapshot_id, first_page.snapshot_id);
    assert_eq!(second_page.total, 3);
    assert_eq!(second_page.records[0].invoke_id, "snapshot-2");

    let Json(sorted_page) = list_invocations(
        State(state),
        Query(ListQuery {
            page: Some(1),
            page_size: Some(1),
            snapshot_id: Some(first_page.snapshot_id),
            sort_by: Some("totalTokens".to_string()),
            sort_order: Some("asc".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("sorting within snapshot should succeed");
    assert_eq!(sorted_page.total, 3);
    assert_eq!(sorted_page.records[0].invoke_id, "snapshot-1");
}

#[tokio::test]
async fn list_invocations_failure_class_filter_matches_resolved_classification() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, status, error_message) in [
        ("filter-client", "http_401", None),
        (
            "filter-abort",
            "failed",
            Some("[downstream_closed] user cancelled"),
        ),
        ("filter-running", "running", None),
        ("filter-service", "failed", None),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-10 08:00:00")
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(error_message)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert failure row");
    }

    let Json(client_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            failure_class: Some("client_failure".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("client failure class filter should succeed");

    assert_eq!(client_filtered.total, 1);
    assert_eq!(client_filtered.records[0].invoke_id, "filter-client");
    assert_eq!(
        client_filtered.records[0].failure_class.as_deref(),
        Some("client_failure")
    );

    let Json(abort_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            failure_class: Some("client_abort".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("client abort failure class filter should succeed");

    assert_eq!(abort_filtered.total, 1);
    assert_eq!(abort_filtered.records[0].invoke_id, "filter-abort");
    assert_eq!(
        abort_filtered.records[0].failure_class.as_deref(),
        Some("client_abort")
    );

    let Json(service_filtered) = list_invocations(
        State(state),
        Query(ListQuery {
            failure_class: Some("service_failure".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("service failure class filter should succeed");

    assert_eq!(service_filtered.total, 1);
    assert_eq!(service_filtered.records[0].invoke_id, "filter-service");
    assert_eq!(
        service_filtered.records[0].failure_class.as_deref(),
        Some("service_failure")
    );
}

#[tokio::test]
async fn list_invocations_status_failed_matches_http_failure_statuses() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, status) in [
        ("status-success", "success"),
        ("status-running", "running"),
        ("status-pending", "pending"),
        ("status-interrupted", "interrupted"),
        ("status-failed", "failed"),
        ("status-http401", "http_401"),
        ("status-http502", "http_502"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-10 08:00:00")
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert status row");
    }

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            error_message,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("status-legacy-null")
    .bind("2026-03-10 08:00:00")
    .bind(SOURCE_PROXY)
    .bind("upstream exploded")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy null-status failure row");

    let Json(failed_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            status: Some("failed".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("failed status filter should succeed");

    assert_eq!(failed_filtered.total, 4);
    let actual = failed_filtered
        .records
        .into_iter()
        .map(|record| record.invoke_id)
        .collect::<HashSet<_>>();
    let expected = [
        "status-failed",
        "status-http401",
        "status-http502",
        "status-legacy-null",
    ]
    .into_iter()
    .map(String::from)
    .collect::<HashSet<_>>();
    assert_eq!(actual, expected);

    let Json(running_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            status: Some("running".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("running status filter should still use exact match");

    assert_eq!(running_filtered.total, 1);
    assert_eq!(running_filtered.records[0].invoke_id, "status-running");

    let Json(interrupted_filtered) = list_invocations(
        State(state),
        Query(ListQuery {
            status: Some("interrupted".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("interrupted status filter should use exact match");

    assert_eq!(interrupted_filtered.total, 1);
    assert_eq!(
        interrupted_filtered.records[0].invoke_id,
        "status-interrupted"
    );
}

#[tokio::test]
async fn list_invocations_status_success_excludes_resolved_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, status, error_message, failure_kind, failure_class) in [
        ("status-success-clean", Some("success"), None, None, None),
        (
            "status-success-trimmed",
            Some(" SUCCESS "),
            None,
            None,
            None,
        ),
        (
            "status-success-explicit-failure-class",
            Some("success"),
            Some("upstream exploded"),
            None,
            Some("service_failure"),
        ),
        (
            "status-success-legacy-failure-kind",
            Some("success"),
            Some("[upstream_response_failed] server_error"),
            Some("upstream_response_failed"),
            None,
        ),
        ("status-success-failed", Some("failed"), None, None, None),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                failure_kind,
                failure_class,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-10 08:00:00")
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(error_message)
        .bind(failure_kind)
        .bind(failure_class)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert success status row");
    }

    let Json(success_filtered) = list_invocations(
        State(state),
        Query(ListQuery {
            status: Some("success".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("success status filter should succeed");

    let actual = success_filtered
        .records
        .into_iter()
        .map(|record| record.invoke_id)
        .collect::<HashSet<_>>();
    let expected = ["status-success-clean", "status-success-trimmed"]
        .into_iter()
        .map(String::from)
        .collect::<HashSet<_>>();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn list_invocations_status_sort_uses_normalized_status_values() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, occurred_at, status, error_message, failure_kind) in [
        (
            "status-sort-trimmed-success",
            "2026-03-10 08:02:00",
            Some(" SUCCESS "),
            None,
            None,
        ),
        (
            "status-sort-success",
            "2026-03-10 08:01:00",
            Some("success"),
            None,
            None,
        ),
        (
            "status-sort-failed",
            "2026-03-10 08:03:00",
            Some("failed"),
            None,
            None,
        ),
        (
            "status-sort-legacy-success-failure",
            "2026-03-10 08:04:00",
            Some("success"),
            Some("[upstream_response_failed] server_error"),
            Some("upstream_response_failed"),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                failure_kind,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(error_message)
        .bind(failure_kind)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert status sort row");
    }

    let Json(sorted) = list_invocations(
        State(state),
        Query(ListQuery {
            page: Some(1),
            page_size: Some(20),
            sort_by: Some("status".to_string()),
            sort_order: Some("asc".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("status sort should succeed");

    let actual = sorted
        .records
        .into_iter()
        .map(|record| record.invoke_id)
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            "status-sort-legacy-success-failure".to_string(),
            "status-sort-failed".to_string(),
            "status-sort-trimmed-success".to_string(),
            "status-sort-success".to_string(),
        ]
    );
}

#[tokio::test]
async fn fetch_invocation_summary_reports_new_records_count_for_applied_filters() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            total_tokens,
            cache_input_tokens,
            cost,
            status,
            t_upstream_ttfb_ms,
            t_total_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind("summary-base")
    .bind("2026-03-10 09:00:00")
    .bind(SOURCE_PROXY)
    .bind("gpt-5.4")
    .bind(120_i64)
    .bind(20_i64)
    .bind(0.012_f64)
    .bind("success")
    .bind(100.0_f64)
    .bind(250.0_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert base summary row");

    let Json(initial_list) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            model: Some("gpt-5.4".to_string()),
            page: Some(1),
            page_size: Some(20),
            ..Default::default()
        }),
    )
    .await
    .expect("seed list query should succeed");

    for (invoke_id, model) in [
        ("summary-new-match", "gpt-5.4"),
        ("summary-new-other", "gpt-5.3-codex"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                model,
                total_tokens,
                cache_input_tokens,
                cost,
                status,
                t_upstream_ttfb_ms,
                t_total_ms,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-10 09:05:00")
        .bind(SOURCE_PROXY)
        .bind(model)
        .bind(240_i64)
        .bind(40_i64)
        .bind(0.024_f64)
        .bind("failed")
        .bind(180.0_f64)
        .bind(500.0_f64)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert post-snapshot row");
    }

    let Json(summary) = fetch_invocation_summary(
        State(state),
        Query(ListQuery {
            model: Some("gpt-5.4".to_string()),
            snapshot_id: Some(initial_list.snapshot_id),
            ..Default::default()
        }),
    )
    .await
    .expect("summary query should succeed");

    assert_eq!(summary.snapshot_id, initial_list.snapshot_id);
    assert_eq!(summary.new_records_count, 1);
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.token.request_count, 1);
    assert_eq!(summary.token.total_tokens, 120);
    assert_eq!(summary.token.cache_input_tokens, 20);
    assert_f64_close(summary.token.avg_tokens_per_request, 120.0);
    assert_f64_close(summary.network.avg_ttfb_ms.unwrap_or_default(), 100.0);
    assert_f64_close(summary.network.p95_total_ms.unwrap_or_default(), 250.0);
    assert_eq!(summary.exception.failure_count, 0);
}

#[tokio::test]
async fn fetch_invocation_summary_resolves_failure_class_for_legacy_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, status, error_message) in [
        ("legacy-service", "failed", None),
        ("legacy-client", "http_401", None),
        (
            "legacy-abort",
            "failed",
            Some("[downstream_closed] user cancelled"),
        ),
        ("legacy-running", "running", None),
        ("legacy-pending", "pending", None),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-10 09:00:00")
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(error_message)
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert legacy failure row");
    }

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            failure_kind,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("legacy-429")
    .bind("2026-03-10 09:00:00")
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind("upstream_http_429")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy 429 row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            failure_kind,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("legacy-stream-success")
    .bind("2026-03-10 09:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("[upstream_response_failed] server_error")
    .bind("upstream_response_failed")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy stream failure row");

    let Json(summary) = fetch_invocation_summary(State(state), Query(ListQuery::default()))
        .await
        .expect("summary query should succeed");

    assert_eq!(summary.total_count, 7);
    assert_eq!(summary.failure_count, 5);
    assert_eq!(summary.exception.failure_count, 5);
    assert_eq!(summary.exception.service_failure_count, 3);
    assert_eq!(summary.exception.client_failure_count, 1);
    assert_eq!(summary.exception.client_abort_count, 1);
    assert_eq!(summary.exception.actionable_failure_count, 3);
}

#[tokio::test]
async fn fetch_invocation_summary_normalizes_top_level_success_and_failure_counts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("summary-success-trimmed")
    .bind("2026-03-10 09:00:00")
    .bind(SOURCE_PROXY)
    .bind(" SUCCESS ")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert trimmed success row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("summary-http-200-success")
    .bind("2026-03-10 09:00:30")
    .bind(SOURCE_PROXY)
    .bind("http_200")
    .bind("")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy http_200 success row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            error_message,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("summary-null-status-failure")
    .bind("2026-03-10 09:01:00")
    .bind(SOURCE_PROXY)
    .bind("upstream exploded")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert null-status failure row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            failure_kind,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("summary-legacy-success-failure")
    .bind("2026-03-10 09:02:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("[upstream_response_failed] server_error")
    .bind("upstream_response_failed")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert legacy success failure row");

    let Json(summary) = fetch_invocation_summary(State(state), Query(ListQuery::default()))
        .await
        .expect("summary query should succeed");

    assert_eq!(summary.total_count, 4);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 2);
    assert_eq!(summary.exception.failure_count, 2);
    assert_eq!(summary.exception.service_failure_count, 2);
}

#[tokio::test]
async fn fetch_invocation_summary_keeps_zero_ms_network_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            t_upstream_ttfb_ms,
            t_total_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("summary-zero-network")
    .bind("2026-03-10 09:10:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(0.0_f64)
    .bind(0.0_f64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert zero-ms summary row");

    let Json(summary) = fetch_invocation_summary(State(state), Query(ListQuery::default()))
        .await
        .expect("summary query with zero-ms samples should succeed");

    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.network.avg_ttfb_ms, Some(0.0));
    assert_eq!(summary.network.p95_ttfb_ms, Some(0.0));
    assert_eq!(summary.network.avg_total_ms, Some(0.0));
    assert_eq!(summary.network.p95_total_ms, Some(0.0));
}

#[tokio::test]
async fn fetch_invocation_summary_returns_zero_values_for_empty_results() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(summary) = fetch_invocation_summary(
        State(state),
        Query(ListQuery {
            model: Some("missing-model".to_string()),
            snapshot_id: Some(999),
            ..Default::default()
        }),
    )
    .await
    .expect("empty summary query should succeed");

    assert_eq!(summary.snapshot_id, 999);
    assert_eq!(summary.new_records_count, 0);
    assert_eq!(summary.total_count, 0);
    assert_eq!(summary.success_count, 0);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 0);
    assert_f64_close(summary.total_cost, 0.0);
    assert_eq!(summary.token.request_count, 0);
    assert_eq!(summary.token.total_tokens, 0);
    assert_f64_close(summary.token.avg_tokens_per_request, 0.0);
    assert_eq!(summary.token.cache_input_tokens, 0);
    assert_f64_close(summary.token.total_cost, 0.0);
    assert_eq!(summary.network.avg_ttfb_ms, None);
    assert_eq!(summary.network.p95_ttfb_ms, None);
    assert_eq!(summary.network.avg_total_ms, None);
    assert_eq!(summary.network.p95_total_ms, None);
    assert_eq!(summary.exception.failure_count, 0);
    assert_eq!(summary.exception.service_failure_count, 0);
    assert_eq!(summary.exception.client_failure_count, 0);
    assert_eq!(summary.exception.client_abort_count, 0);
    assert_eq!(summary.exception.actionable_failure_count, 0);
}

#[tokio::test]
async fn fetch_invocation_new_records_count_requires_snapshot_id() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let error = fetch_invocation_new_records_count(State(state), Query(ListQuery::default()))
        .await
        .expect_err("new-count query should reject missing snapshot id");

    match error {
        ApiError::BadRequest(err) => {
            assert_eq!(err.to_string(), "snapshotId is required");
        }
        other => panic!("expected BadRequest, got: {other:?}"),
    }
}

#[tokio::test]
async fn fetch_invocation_new_records_count_uses_snapshot_boundary() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            total_tokens,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("new-count-base")
    .bind("2026-03-10 09:00:00")
    .bind(SOURCE_PROXY)
    .bind("gpt-5.4")
    .bind(120_i64)
    .bind("success")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert base invocation row");

    let Json(initial_list) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            model: Some("gpt-5.4".to_string()),
            page: Some(1),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("initial list query should succeed");

    for (invoke_id, occurred_at, model) in [
        ("new-count-match", "2026-03-10 09:05:00", "gpt-5.4"),
        ("new-count-other", "2026-03-10 09:06:00", "gpt-5.3"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                model,
                total_tokens,
                status,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(model)
        .bind(120_i64)
        .bind("success")
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert post-snapshot invocation row");
    }

    let Json(new_count) = fetch_invocation_new_records_count(
        State(state),
        Query(ListQuery {
            model: Some("gpt-5.4".to_string()),
            snapshot_id: Some(initial_list.snapshot_id),
            ..Default::default()
        }),
    )
    .await
    .expect("new count query should succeed");

    assert_eq!(new_count.snapshot_id, initial_list.snapshot_id);
    assert_eq!(new_count.new_records_count, 1);
}

#[tokio::test]
async fn list_invocations_total_tokens_range_filters_exclude_null_values() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("tokens-null")
    .bind("2026-03-10 09:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert null total_tokens row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("tokens-set")
    .bind("2026-03-10 09:01:00")
    .bind(SOURCE_PROXY)
    .bind(10_i64)
    .bind("success")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert total_tokens row");

    let Json(max_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            max_total_tokens: Some(0),
            ..Default::default()
        }),
    )
    .await
    .expect("list query with maxTotalTokens should succeed");

    assert_eq!(max_filtered.total, 0);
    assert!(max_filtered.records.is_empty());

    let Json(min_filtered) = list_invocations(
        State(state),
        Query(ListQuery {
            min_total_tokens: Some(0),
            ..Default::default()
        }),
    )
    .await
    .expect("list query with minTotalTokens should succeed");

    assert_eq!(min_filtered.total, 1);
    assert_eq!(min_filtered.records.len(), 1);
    assert_eq!(min_filtered.records[0].invoke_id, "tokens-set");
    assert_eq!(min_filtered.records[0].total_tokens, Some(10));
}

#[tokio::test]
async fn list_invocations_total_ms_range_filters_exclude_null_values() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("ms-null")
    .bind("2026-03-10 09:10:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert null t_total_ms row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            t_total_ms,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("ms-set")
    .bind("2026-03-10 09:11:00")
    .bind(SOURCE_PROXY)
    .bind(50.0_f64)
    .bind("success")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert t_total_ms row");

    let Json(max_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            max_total_ms: Some(0.0),
            ..Default::default()
        }),
    )
    .await
    .expect("list query with maxTotalMs should succeed");

    assert_eq!(max_filtered.total, 0);
    assert!(max_filtered.records.is_empty());

    let Json(min_filtered) = list_invocations(
        State(state),
        Query(ListQuery {
            min_total_ms: Some(0.0),
            ..Default::default()
        }),
    )
    .await
    .expect("list query with minTotalMs should succeed");

    assert_eq!(min_filtered.total, 1);
    assert_eq!(min_filtered.records.len(), 1);
    assert_eq!(min_filtered.records[0].invoke_id, "ms-set");
    assert_f64_close(min_filtered.records[0].t_total_ms.unwrap_or_default(), 50.0);
}

#[tokio::test]
async fn fetch_invocation_suggestions_orders_by_count_and_respects_time_bounds() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, occurred_at, model) in [
        (
            "suggest-alpha-1",
            "2026-03-10 09:00:00",
            Some("model-alpha"),
        ),
        (
            "suggest-alpha-2",
            "2026-03-10 09:05:00",
            Some("model-alpha"),
        ),
        ("suggest-beta-1", "2026-03-10 09:06:00", Some("model-beta")),
        ("suggest-old-1", "2026-03-09 09:00:00", Some("model-old")),
        ("suggest-null", "2026-03-10 09:08:00", None),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                model,
                status,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(model)
        .bind("success")
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert suggestion invocation row");
    }

    let Json(suggestions) = fetch_invocation_suggestions(
        State(state),
        Query(ListQuery {
            from: Some("2026-03-10T00:00:00Z".to_string()),
            to: Some("2026-03-11T00:00:00Z".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("suggestions query should succeed");

    assert!(
        suggestions
            .model
            .items
            .iter()
            .all(|item| item.value != "model-old"),
        "model suggestions should exclude rows outside the time window"
    );

    let first = suggestions
        .model
        .items
        .first()
        .expect("model suggestions should include matching rows");
    assert_eq!(first.value, "model-alpha");
    assert_eq!(first.count, 2);
    assert!(
        suggestions
            .model
            .items
            .iter()
            .all(|item| !item.value.is_empty()),
        "suggestions should not contain empty values"
    );
    assert!(!suggestions.model.has_more);
}

#[tokio::test]
async fn fetch_invocation_suggestions_filters_active_bucket_before_limit() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for index in 0..35 {
        let model = format!("model-hot-{index:02}");
        for occurrence in 0..2 {
            sqlx::query(
                r#"
                INSERT INTO codex_invocations (
                    invoke_id,
                    occurred_at,
                    source,
                    model,
                    status,
                    raw_response
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                "#,
            )
            .bind(format!("suggest-hot-{index:02}-{occurrence}"))
            .bind(format!("2026-03-10 10:{index:02}:{occurrence:02}"))
            .bind(SOURCE_PROXY)
            .bind(&model)
            .bind("success")
            .bind("{}")
            .execute(&state.pool)
            .await
            .expect("insert hot suggestion row");
        }
    }

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("suggest-needle")
    .bind("2026-03-10 11:00:00")
    .bind(SOURCE_PROXY)
    .bind("model-needle")
    .bind("success")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert needle suggestion row");

    let Json(suggestions) = fetch_invocation_suggestions(
        State(state),
        Query(ListQuery {
            suggest_field: Some("model".to_string()),
            suggest_query: Some("needle".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("filtered suggestions query should succeed");

    let model_values = suggestions
        .model
        .items
        .iter()
        .map(|item| item.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(model_values, vec!["model-needle"]);
    assert!(!suggestions.model.has_more);
    assert!(suggestions.endpoint.items.is_empty());
    assert!(suggestions.failure_kind.items.is_empty());
    assert!(suggestions.prompt_cache_key.items.is_empty());
    assert!(suggestions.requester_ip.items.is_empty());
}

#[tokio::test]
async fn fetch_invocation_suggestions_use_snapshot_and_keep_other_filters() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, occurred_at, model, proxy) in [
        (
            "suggest-snapshot-alpha",
            "2026-03-10 09:00:00",
            "model-alpha",
            "proxy-a",
        ),
        (
            "suggest-snapshot-beta",
            "2026-03-10 09:01:00",
            "model-beta",
            "proxy-a",
        ),
        (
            "suggest-snapshot-gamma",
            "2026-03-10 08:59:00",
            "model-gamma",
            "proxy-b",
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                model,
                payload,
                status,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .bind(SOURCE_PROXY)
        .bind(model)
        .bind(json!({ "proxyDisplayName": proxy }).to_string())
        .bind("success")
        .bind("{}")
        .execute(&state.pool)
        .await
        .expect("insert suggestion snapshot row");
    }

    let Json(initial_list) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            page: Some(1),
            page_size: Some(20),
            ..Default::default()
        }),
    )
    .await
    .expect("seed list query should succeed");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            payload,
            status,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("suggest-snapshot-delta")
    .bind("2026-03-10 09:03:00")
    .bind(SOURCE_PROXY)
    .bind("model-delta")
    .bind(json!({ "proxyDisplayName": "proxy-a" }).to_string())
    .bind("success")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert post-snapshot suggestion row");

    let Json(suggestions) = fetch_invocation_suggestions(
        State(state),
        Query(ListQuery {
            snapshot_id: Some(initial_list.snapshot_id),
            model: Some("model-alpha".to_string()),
            from: Some("2026-03-10T01:00:00Z".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("suggestions query should succeed");

    let model_values = suggestions
        .model
        .items
        .iter()
        .map(|item| item.value.as_str())
        .collect::<Vec<_>>();
    assert!(model_values.contains(&"model-alpha"));
    assert!(model_values.contains(&"model-beta"));
    assert!(
        !model_values.contains(&"model-delta"),
        "suggestions should stay inside the frozen snapshot"
    );
    assert!(
        !model_values.contains(&"model-gamma"),
        "suggestions should keep the other applied time-range filter"
    );
}

#[tokio::test]
async fn stats_endpoints_preserve_historical_xy_records() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("xy-history-stats-1")
    .bind(&occurred_at)
    .bind(SOURCE_XY)
    .bind(16_i64)
    .bind(0.0042_f64)
    .bind("success")
    .bind(r#"{"serviceTier":"priority"}"#)
    .bind(r#"{"legacy":true}"#)
    .execute(&state.pool)
    .await
    .expect("insert historical xy stats row");

    let Json(stats) = fetch_stats(State(state.clone()))
        .await
        .expect("fetch_stats should include historical xy rows");
    assert_eq!(stats.total_count, 1);
    assert_eq!(stats.success_count, 1);
    assert_eq!(stats.failure_count, 0);
    assert_eq!(stats.total_tokens, 16);
    assert_f64_close(stats.total_cost, 0.0042);

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: None,
        }),
    )
    .await
    .expect("fetch_summary should include historical xy rows");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 16);
    assert_f64_close(summary.total_cost, 0.0042);

    let Json(timeseries) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: None,
        }),
    )
    .await
    .expect("fetch_timeseries should include historical xy rows");
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_count)
            .sum::<i64>(),
        1
    );
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_tokens)
            .sum::<i64>(),
        16
    );
    assert_f64_close(
        timeseries
            .points
            .iter()
            .map(|point| point.total_cost)
            .sum::<f64>(),
        0.0042,
    );
}

#[test]
fn normalize_prompt_cache_conversation_limit_accepts_whitelist_values_only() {
    assert_eq!(normalize_prompt_cache_conversation_limit(None), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(20)), 20);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(50)), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(100)), 100);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(10)), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(200)), 50);
}

#[test]
fn normalize_prompt_cache_conversation_activity_hours_accepts_whitelist_values_only() {
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(None),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(1)),
        Some(1)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(3)),
        Some(3)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(6)),
        Some(6)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(12)),
        Some(12)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(24)),
        Some(24)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(2)),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(48)),
        None
    );
}

#[test]
fn normalize_prompt_cache_conversation_activity_minutes_accepts_precise_five_minutes_only() {
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(None),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(Some(5)),
        Some(5)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(Some(1)),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(Some(10)),
        None
    );
}

#[test]
fn resolve_prompt_cache_conversation_selection_rejects_mutually_exclusive_params() {
    let err = resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit: Some(20),
        activity_hours: Some(3),
        activity_minutes: None,
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
    })
    .expect_err("selection should reject mutually exclusive params");

    match err {
        ApiError::BadRequest(inner) => {
            let message = inner.to_string();
            assert!(message.contains("mutually exclusive"));
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[test]
fn resolve_prompt_cache_conversation_selection_rejects_activity_hours_and_minutes_combo() {
    let err = resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit: None,
        activity_hours: Some(3),
        activity_minutes: Some(5),
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
    })
    .expect_err("selection should reject mixed hour and minute windows");

    match err {
        ApiError::BadRequest(inner) => {
            let message = inner.to_string();
            assert!(message.contains("mutually exclusive"));
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[tokio::test]
async fn prompt_cache_conversations_groups_recent_keys_and_uses_history_totals() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        key: Option<&str>,
        status: &str,
        total_tokens: i64,
        cost: f64,
    ) {
        let payload = match key {
            Some(key) => json!({ "promptCacheKey": key }).to_string(),
            None => "{}".to_string(),
        };
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(payload)
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    // key-a: active in 24h + older history
    insert_row(
        &state.pool,
        "pck-a-history",
        now - ChronoDuration::hours(48),
        Some("pck-a"),
        "success",
        100,
        1.0,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-a-24h-1",
        now - ChronoDuration::hours(2),
        Some("pck-a"),
        "success",
        20,
        0.2,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-a-24h-2",
        now - ChronoDuration::hours(1),
        Some("pck-a"),
        "failed",
        30,
        0.3,
    )
    .await;

    // key-b: newer created_at so it should rank before key-a.
    insert_row(
        &state.pool,
        "pck-b-24h-1",
        now - ChronoDuration::hours(10),
        Some("pck-b"),
        "success",
        10,
        0.1,
    )
    .await;

    // key-c: not active in last 24h; should be excluded.
    insert_row(
        &state.pool,
        "pck-c-history",
        now - ChronoDuration::hours(72),
        Some("pck-c"),
        "success",
        8,
        0.08,
    )
    .await;

    // missing key in last 24h; should be ignored.
    insert_row(
        &state.pool,
        "pck-missing-24h",
        now - ChronoDuration::minutes(40),
        None,
        "success",
        999,
        9.99,
    )
    .await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
        }),
    )
    .await
    .expect("prompt cache conversation stats should succeed");

    assert_eq!(
        response.selection_mode,
        PromptCacheConversationSelectionMode::Count
    );
    assert_eq!(response.selected_limit, Some(20));
    assert_eq!(response.selected_activity_hours, None);
    assert_eq!(
        response.implicit_filter.kind,
        Some(PromptCacheConversationImplicitFilterKind::InactiveOutside24h)
    );
    assert_eq!(response.implicit_filter.filtered_count, 1);
    assert_eq!(response.conversations.len(), 2);
    assert_eq!(response.conversations[0].prompt_cache_key, "pck-b");
    assert_eq!(response.conversations[1].prompt_cache_key, "pck-a");

    let key_a = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-a")
        .expect("pck-a should be included");
    assert_eq!(key_a.request_count, 3);
    assert_eq!(key_a.total_tokens, 150);
    assert!((key_a.total_cost - 1.5).abs() < 1e-9);
    assert_eq!(key_a.last24h_requests.len(), 2);
    assert_eq!(key_a.last24h_requests[0].request_tokens, 20);
    assert_eq!(key_a.last24h_requests[0].cumulative_tokens, 20);
    assert!(key_a.last24h_requests[0].is_success);
    assert_eq!(key_a.last24h_requests[0].outcome, "success");
    assert_eq!(key_a.last24h_requests[1].request_tokens, 30);
    assert_eq!(key_a.last24h_requests[1].cumulative_tokens, 50);
    assert!(!key_a.last24h_requests[1].is_success);
    assert_eq!(key_a.last24h_requests[1].outcome, "failure");
}

#[tokio::test]
async fn prompt_cache_last24h_requests_keep_null_status_rows_neutral() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_at = format_naive((now - ChronoDuration::minutes(20)).with_timezone(&Shanghai).naive_local());

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-neutral-success")
    .bind(occurred_at.clone())
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(20_i64)
    .bind(0.2_f64)
    .bind(json!({ "promptCacheKey": "pck-neutral" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert success prompt cache row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("pck-neutral-null-status")
    .bind(format_naive(
        (now - ChronoDuration::minutes(10))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(15_i64)
    .bind(0.15_f64)
    .bind(json!({ "promptCacheKey": "pck-neutral" }).to_string())
    .bind("{}")
    .bind("none")
    .bind("")
    .execute(&state.pool)
    .await
    .expect("insert null-status prompt cache row");

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
        }),
    )
    .await
    .expect("prompt cache neutral conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-neutral")
        .expect("neutral prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 2);
    assert_eq!(conversation.last24h_requests[0].outcome, "success");
    assert_eq!(conversation.last24h_requests[1].status, "unknown");
    assert!(!conversation.last24h_requests[1].is_success);
    assert_eq!(conversation.last24h_requests[1].outcome, "neutral");
}

#[tokio::test]
async fn prompt_cache_last24h_requests_treat_running_rows_with_failure_class_as_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("pck-running-failure")
    .bind(format_naive(
        (now - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(11_i64)
    .bind(0.11_f64)
    .bind(json!({ "promptCacheKey": "pck-running-failure" }).to_string())
    .bind("{}")
    .bind("service_failure")
    .bind("upstream stream error")
    .execute(&state.pool)
    .await
    .expect("insert running prompt cache failure row");

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
        }),
    )
    .await
    .expect("prompt cache running failure conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-running-failure")
        .expect("running failure prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].status, "running");
    assert!(!conversation.last24h_requests[0].is_success);
    assert_eq!(conversation.last24h_requests[0].outcome, "failure");
}
