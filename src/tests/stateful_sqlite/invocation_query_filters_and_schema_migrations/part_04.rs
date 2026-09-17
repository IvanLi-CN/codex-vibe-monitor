#[tokio::test]
pub(crate) async fn fetch_invocation_new_records_count_requires_snapshot_id() {
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
pub(crate) async fn fetch_invocation_new_records_count_uses_snapshot_boundary() {
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
pub(crate) async fn list_invocations_total_tokens_range_filters_exclude_null_values() {
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
pub(crate) async fn list_invocations_total_ms_range_filters_exclude_null_values() {
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
pub(crate) async fn fetch_invocation_suggestions_orders_by_count_and_respects_time_bounds() {
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
pub(crate) async fn fetch_invocation_suggestions_include_extended_buckets_and_account_labels() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_extended_suggestion_invocations(&state.pool).await;

    let Json(suggestions) =
        fetch_invocation_suggestions(State(state.clone()), Query(ListQuery::default()))
            .await
            .expect("extended suggestions query should succeed");

    assert_eq!(suggestions.sticky_key.items[0].value, "sticky-alpha");
    assert_eq!(suggestions.proxy_display_name.items[0].value, "proxy-zeta");
    assert_eq!(suggestions.service_tier.items[0].value, "priority");
    assert_eq!(suggestions.reasoning_effort.items[0].value, "high");
    assert_eq!(suggestions.request_model.items[0].value, "gpt-5.4");
    assert_eq!(suggestions.response_model.items[0].value, "gpt-5.4-routing");
    assert_eq!(suggestions.upstream_account.items[0].value, "42");
    assert_eq!(
        suggestions.upstream_account.items[0].label.as_deref(),
        Some("Pool Alpha (#42)")
    );
    assert_eq!(suggestions.upstream_account.items[0].count, 2);

    let Json(filtered) = fetch_invocation_suggestions(
        State(state),
        Query(ListQuery {
            suggest_field: Some("upstreamAccount".to_string()),
            suggest_query: Some("42".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("upstream account suggestions should match by id");

    let values = filtered
        .upstream_account
        .items
        .iter()
        .map(|item| item.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(values, vec!["42"]);
    assert_eq!(
        filtered.upstream_account.items[0].label.as_deref(),
        Some("Pool Alpha (#42)")
    );
}

async fn seed_extended_suggestion_invocations(pool: &Pool<Sqlite>) {
    for (invoke_id, payload) in [
        (
            "suggest-extended-1",
            json!({
                "stickyKey": "sticky-alpha",
                "proxyDisplayName": "proxy-zeta",
                "upstreamAccountId": 42,
                "upstreamAccountName": "Pool Alpha",
                "serviceTier": "priority",
                "reasoningEffort": "high",
                "requestModel": "gpt-5.4",
                "responseModel": "gpt-5.4-routing"
            }),
        ),
        (
            "suggest-extended-2",
            json!({
                "stickyKey": "sticky-alpha",
                "proxyDisplayName": "proxy-zeta",
                "upstreamAccountId": 42,
                "upstreamAccountName": "Pool Alpha",
                "serviceTier": "priority",
                "reasoningEffort": "high",
                "requestModel": "gpt-5.4",
                "responseModel": "gpt-5.4-routing"
            }),
        ),
        (
            "suggest-extended-3",
            json!({
                "stickyKey": "sticky-beta",
                "proxyDisplayName": "proxy-eta",
                "upstreamAccountId": 77,
                "upstreamAccountName": "Pool Beta",
                "serviceTier": "flex",
                "reasoningEffort": "medium",
                "requestModel": "gpt-5-mini",
                "responseModel": "gpt-5-mini"
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
        .bind("2026-03-11 11:00:00")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(payload.to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert extended suggestion invocation");
    }
}

#[tokio::test]
pub(crate) async fn list_invocations_supports_model_target_multi_select_and_rerouted_filters() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    seed_model_filter_invocations(&state.pool).await;

    let Json(request_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            models: Some("gpt-5.4,gpt-5.5".to_string()),
            model_target: Some("request".to_string()),
            model_rerouted: Some("true".to_string()),
            reasoning_efforts: Some("high".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("request-side model filter should succeed");

    assert_eq!(request_filtered.total, 1);
    assert_eq!(request_filtered.records[0].invoke_id, "model-filter-match");

    let Json(response_filtered) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            models: Some("gpt-5.4-routing".to_string()),
            model_target: Some("response".to_string()),
            model_rerouted: Some("true".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("response-side model filter should succeed");

    assert_eq!(response_filtered.total, 1);
    assert_eq!(response_filtered.records[0].invoke_id, "model-filter-match");

    let Json(not_rerouted_filtered) = list_invocations(
        State(state),
        Query(ListQuery {
            models: Some("gpt-5.4".to_string()),
            model_target: Some("response".to_string()),
            model_rerouted: Some("false".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("not-rerouted model filter should succeed");

    assert_eq!(not_rerouted_filtered.total, 1);
    assert_eq!(
        not_rerouted_filtered.records[0].invoke_id,
        "model-filter-stable"
    );
}

async fn seed_model_filter_invocations(pool: &Pool<Sqlite>) {
    for (invoke_id, model, payload) in [
        (
            "model-filter-match",
            "gpt-5.4-routing",
            json!({
                "requestModel": "gpt-5.4",
                "responseModel": "gpt-5.4-routing",
                "reasoningEffort": "high"
            }),
        ),
        (
            "model-filter-stable",
            "gpt-5.4",
            json!({
                "requestModel": "gpt-5.4",
                "responseModel": "gpt-5.4",
                "reasoningEffort": "high"
            }),
        ),
        (
            "model-filter-other",
            "gpt-5.6",
            json!({
                "requestModel": "gpt-5.6",
                "responseModel": "gpt-5.6",
                "reasoningEffort": "medium"
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
                model,
                payload,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(invoke_id)
        .bind("2026-03-11 12:00:00")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(model)
        .bind(payload.to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert model filter invocation");
    }
}

#[tokio::test]
pub(crate) async fn fetch_invocation_suggestions_filters_active_bucket_before_limit() {
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
pub(crate) async fn fetch_invocation_suggestions_use_snapshot_and_keep_other_filters() {
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
        insert_suggestion_snapshot(&state.pool, invoke_id, occurred_at, model, proxy).await;
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

    insert_suggestion_snapshot(
        &state.pool,
        "suggest-snapshot-delta",
        "2026-03-10 09:03:00",
        "model-delta",
        "proxy-a",
    )
    .await;

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

async fn insert_suggestion_snapshot(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: &str,
    model: &str,
    proxy: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, model, payload, status, raw_response
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
    .execute(pool)
    .await
    .expect("insert suggestion snapshot row");
}

#[tokio::test]
pub(crate) async fn stats_endpoints_preserve_historical_xy_records() {
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

    let summary_query = SummaryQuery {
        window: Some("1d".to_string()),
        limit: None,
        time_zone: None,
        upstream_account_id: None,
    };
    let Json(summary) =
        fetch_summary_from_memory_snapshot(State(state.clone()), Query(summary_query))
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
            upstream_account_id: None,
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

#[tokio::test]
pub(crate) async fn yesterday_summary_and_timeseries_only_include_previous_local_day() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now_local = Utc::now().with_timezone(&Shanghai);
    let today = now_local.date_naive();
    let yesterday = today.pred_opt().expect("previous date");

    let yesterday_occurred = yesterday
        .and_hms_opt(12, 34, 0)
        .expect("valid yesterday timestamp");
    let today_occurred = today.and_hms_opt(12, 34, 0).expect("valid today timestamp");

    insert_local_day_invocation(
        &state.pool,
        LocalDayInvocation {
            invoke_id: "yesterday-local-noon",
            occurred_at: format_naive(yesterday_occurred),
            total_tokens: 11,
            cost: 0.11,
            raw_response: r#"{"range":"yesterday"}"#,
        },
    )
    .await;
    insert_local_day_invocation(
        &state.pool,
        LocalDayInvocation {
            invoke_id: "today-local-noon",
            occurred_at: format_naive(today_occurred),
            total_tokens: 22,
            cost: 0.22,
            raw_response: r#"{"range":"today"}"#,
        },
    )
    .await;

    assert_yesterday_summary(state.clone()).await;

    let Json(timeseries) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "yesterday".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch yesterday timeseries");
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
        11
    );
    assert_f64_close(
        timeseries
            .points
            .iter()
            .map(|point| point.total_cost)
            .sum::<f64>(),
        0.11,
    );

    let expected_start = format_utc_iso(local_midnight_utc(yesterday, Shanghai));
    let expected_end = format_utc_iso(local_midnight_utc(today, Shanghai));
    assert_eq!(timeseries.range_start, expected_start);
    assert_eq!(timeseries.range_end, expected_end);
    assert_eq!(timeseries.points.len(), 24);
    assert!(
        timeseries.points.iter().all(|point| {
            point.total_count == 0 || point.bucket_start.as_str() < expected_end.as_str()
        }),
        "non-zero yesterday buckets must stay inside the previous local day"
    );
}

async fn assert_yesterday_summary(state: Arc<AppState>) {
    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state),
        Query(SummaryQuery {
            window: Some("yesterday".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("fetch yesterday summary");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 11);
    assert_f64_close(summary.total_cost, 0.11);
}

struct LocalDayInvocation<'a> {
    invoke_id: &'a str,
    occurred_at: String,
    total_tokens: i64,
    cost: f64,
    raw_response: &'a str,
}

async fn insert_local_day_invocation(pool: &Pool<Sqlite>, row: LocalDayInvocation<'_>) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, total_tokens, cost, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(row.invoke_id)
    .bind(row.occurred_at)
    .bind(SOURCE_PROXY)
    .bind(row.total_tokens)
    .bind(row.cost)
    .bind("success")
    .bind(r#"{"serviceTier":"priority"}"#)
    .bind(row.raw_response)
    .execute(pool)
    .await
    .expect("insert local-day invocation");
}

use super::*;
