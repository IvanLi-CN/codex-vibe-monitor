#[tokio::test]
pub(crate) async fn list_invocations_projects_payload_context_fields() {
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
        r#"{"endpoint":"/v1/responses","failureKind":"upstream_stream_error","requesterIp":"198.51.100.77","promptCacheKey":"pck-list-1","routeMode":"pool","upstreamAccountId":17,"upstreamAccountName":"pool-account-17","responseContentEncoding":"gzip, br","transport":"websocket","requestedServiceTier":"priority","serviceTier":null,"service_tier":"priority","proxyDisplayName":"jp-relay-01","proxyWeightDelta":-0.68,"reasoningEffort":"high"}"#,
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
    assert_eq!(record.transport.as_deref(), Some("websocket"));
    assert_eq!(record.requested_service_tier.as_deref(), Some("priority"));
    assert_eq!(record.service_tier.as_deref(), Some("priority"));
    assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-01"));
    assert_eq!(record.proxy_weight_delta, Some(-0.68));
    assert_eq!(record.reasoning_effort.as_deref(), Some("high"));
}

#[tokio::test]
pub(crate) async fn list_invocations_filters_by_sticky_key_and_upstream_account_id() {
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
pub(crate) async fn invocation_queries_filter_upstream_scope_and_treat_legacy_rows_as_external() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_upstream_scope_invocations(&state.pool).await;

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

async fn seed_upstream_scope_invocations(pool: &Pool<Sqlite>) {
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
        .execute(pool)
        .await
        .expect("insert invocation row");
    }
}

#[tokio::test]
pub(crate) async fn list_invocations_filters_extended_route_and_diagnostics_dimensions() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_route_diagnostics_invocations(&state.pool).await;

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            upstream_scope: Some("internal".to_string()),
            proxy_display_name: Some("tokyo-edge-a".to_string()),
            transport: Some("websocket".to_string()),
            service_tier: Some("priority".to_string()),
            reasoning_effort: Some("high".to_string()),
            page: Some(1),
            page_size: Some(20),
            ..Default::default()
        }),
    )
    .await
    .expect("extended route diagnostics filters should succeed");

    assert_eq!(response.total, 1);
    assert_eq!(response.records[0].invoke_id, "route-diagnostics-hit");
    assert_eq!(
        response.records[0].proxy_display_name.as_deref(),
        Some("tokyo-edge-a")
    );
    assert_eq!(response.records[0].transport.as_deref(), Some("websocket"));
    assert_eq!(
        response.records[0].service_tier.as_deref(),
        Some("priority")
    );
    assert_eq!(
        response.records[0].reasoning_effort.as_deref(),
        Some("high")
    );
}

async fn seed_route_diagnostics_invocations(pool: &Pool<Sqlite>) {
    for (invoke_id, payload) in [
        (
            "route-diagnostics-hit",
            json!({
                "upstreamScope": "internal",
                "routeMode": "pool",
                "proxyDisplayName": "tokyo-edge-a",
                "transport": "websocket",
                "serviceTier": "priority",
                "reasoningEffort": "high",
                "upstreamAccountId": 42,
                "upstreamAccountName": "Pool Alpha",
                "stickyKey": "sticky-a"
            }),
        ),
        (
            "route-diagnostics-wrong-transport",
            json!({
                "upstreamScope": "internal",
                "routeMode": "pool",
                "proxyDisplayName": "tokyo-edge-a",
                "transport": "http",
                "serviceTier": "priority",
                "reasoningEffort": "high",
                "upstreamAccountId": 42,
                "upstreamAccountName": "Pool Alpha",
                "stickyKey": "sticky-a"
            }),
        ),
        (
            "route-diagnostics-wrong-tier",
            json!({
                "upstreamScope": "external",
                "routeMode": "forward_proxy",
                "proxyDisplayName": "tokyo-edge-b",
                "transport": "websocket",
                "serviceTier": "flex",
                "reasoningEffort": "medium",
                "upstreamAccountId": 77,
                "upstreamAccountName": "Pool Beta",
                "stickyKey": "sticky-b"
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
        .execute(pool)
        .await
        .expect("insert route diagnostics invocation");
    }
}

#[tokio::test]
pub(crate) async fn list_invocations_response_omits_raw_expires_at_field() {
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
pub(crate) async fn list_invocations_tolerates_malformed_payload_json() {
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
pub(crate) async fn list_invocations_ignores_non_numeric_proxy_weight_delta() {
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
pub(crate) async fn list_invocations_preserves_historical_xy_records() {
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
pub(crate) async fn list_invocations_legacy_limit_query_skips_snapshot_shape() {
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
pub(crate) async fn list_invocations_keeps_snapshot_stable_across_pagination_and_sorting() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    for (invoke_id, occurred_at, total_tokens) in [
        ("snapshot-1", "2026-03-10 08:00:00", 100_i64),
        ("snapshot-2", "2026-03-10 08:01:00", 200_i64),
        ("snapshot-3", "2026-03-10 08:02:00", 300_i64),
    ] {
        insert_snapshot_invocation(&state.pool, invoke_id, occurred_at, total_tokens).await;
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

    insert_snapshot_invocation(&state.pool, "snapshot-4", "2026-03-10 08:03:00", 50).await;

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

async fn insert_snapshot_invocation(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: &str,
    total_tokens: i64,
) {
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
    .execute(pool)
    .await
    .expect("insert snapshot row");
}

#[tokio::test]
pub(crate) async fn list_invocations_failure_class_filter_matches_resolved_classification() {
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
pub(crate) async fn list_invocations_status_failed_matches_http_failure_statuses() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_status_filter_invocations(&state.pool).await;

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

    assert_exact_status_filters(state).await;
}

async fn seed_status_filter_invocations(pool: &Pool<Sqlite>) {
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
        .execute(pool)
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
    .execute(pool)
    .await
    .expect("insert legacy null-status failure row");
}

async fn assert_exact_status_filters(state: Arc<AppState>) {
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

use super::*;
