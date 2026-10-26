use super::*;

fn dashboard_activity_topic_descriptor() -> SubscriptionTopicDescriptor {
    SubscriptionTopicDescriptor {
        topic: "dashboard.activity.current".to_string(),
        params: BTreeMap::from([
            ("range".to_string(), "today".to_string()),
            ("timeZone".to_string(), "Asia/Shanghai".to_string()),
            ("recentLimit".to_string(), "2".to_string()),
            ("includeAccounts".to_string(), "true".to_string()),
            ("includeRecent".to_string(), "true".to_string()),
        ]),
    }
}

fn extract_subscription_snapshot_payload(prepared: PreparedSubscriptionConnection) -> Value {
    let mut initial = prepared.initial.into_iter();
    initial
        .next()
        .expect("subscription snapshot event")
        .frame
        .payload_value()
}

#[tokio::test]
async fn dashboard_activity_subscription_projection_slices_update_cached_frame_without_db_rebuild()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
        .expect("bind dashboard network cache");

    let initial_prepared = state
        .subscription_hub
        .prepare_connection(
            state.clone(),
            vec![dashboard_activity_topic_descriptor()],
            vec![],
        )
        .await
        .expect("prepare dashboard activity subscription snapshot");
    let initial_payload = extract_subscription_snapshot_payload(initial_prepared);
    let _dashboard_lease = state
        .subscription_hub
        .register_test_topic_name("dashboard.activity.current")
        .await;
    let initial_accounts = initial_payload
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(initial_accounts.iter().all(|account| {
        account.get("accountKey").and_then(Value::as_str) != Some("upstream:99")
    }));

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    state
        .proxy_runtime_invocations
        .upsert(crate::api::ApiInvocation {
            id: 93_001,
            invoke_id: "dashboard-subscription-live-running".to_string(),
            occurred_at: occurred_at.clone(),
            source: SOURCE_PROXY.to_string(),
            proxy_display_name: None,
            model: Some("gpt-5".to_string()),
            request_model: Some("gpt-5".to_string()),
            response_model: Some("gpt-5".to_string()),
            input_tokens: Some(0),
            output_tokens: Some(0),
            cache_input_tokens: Some(0),
            reported_cache_write_tokens: None,
            reasoning_tokens: Some(0),
            reasoning_effort: None,
            total_tokens: Some(0),
            cost: Some(0.0),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            cache_write_tokens: Some(0),
            status: Some("running".to_string()),
            live_phase: Some("requesting".to_string()),
            error_message: None,
            downstream_status_code: None,
            failure_kind: None,
            blocked_binding: None,
            blocked_binding_json: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: None,
            failure_class: None,
            is_actionable: None,
            endpoint: Some("/v1/responses".to_string()),
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
            requester_ip: None,
            prompt_cache_key: Some("pck-dashboard-subscription-live-running".to_string()),
            sticky_key: None,
            route_mode: None,
            upstream_account_id: Some(99),
            upstream_account_name: Some("Live Overlay Account".to_string()),
            response_content_encoding: None,
            request_compression_algorithm: None,
            transport: None,
            pool_attempt_count: None,
            pool_distinct_account_count: None,
            pool_attempt_terminal_reason: None,
            requested_service_tier: None,
            service_tier: None,
            billing_service_tier: None,
            proxy_weight_delta: None,
            cost_estimated: None,
            cost_audit: None,
            price_version: None,
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: None,
            response_raw_truncated_reason: None,
            detail_level: DETAIL_LEVEL_FULL.to_string(),
            detail_pruned_at: None,
            detail_prune_reason: None,
            t_total_ms: None,
            t_req_read_ms: Some(0.0),
            t_req_parse_ms: Some(0.0),
            t_upstream_connect_ms: Some(0.0),
            t_upstream_ttfb_ms: Some(0.0),
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
            created_at: occurred_at.clone(),
        });
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(
            &crate::api::ApiInvocation {
                id: 93_002,
                invoke_id: "dashboard-subscription-live-current".to_string(),
                occurred_at: occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                proxy_display_name: None,
                model: Some("gpt-5".to_string()),
                request_model: Some("gpt-5".to_string()),
                response_model: Some("gpt-5".to_string()),
                input_tokens: Some(100),
                output_tokens: Some(221),
                cache_input_tokens: Some(0),
                reported_cache_write_tokens: None,
                reasoning_tokens: Some(0),
                reasoning_effort: None,
                total_tokens: Some(321),
                cost: Some(0.32),
                cost_input: None,
                cost_cache_write: None,
                cost_cache_read: None,
                cost_output: None,
                cost_reasoning: None,
                cache_write_tokens: Some(0),
                status: Some("success".to_string()),
                live_phase: None,
                error_message: None,
                downstream_status_code: None,
                failure_kind: None,
                blocked_binding: None,
                blocked_binding_json: None,
                stream_terminal_event: None,
                upstream_error_code: None,
                upstream_error_message: None,
                downstream_error_message: None,
                upstream_request_id: None,
                failure_class: None,
                is_actionable: None,
                endpoint: Some("/v1/responses".to_string()),
                compaction_request_kind: None,
                compaction_response_kind: None,
                image_intent: None,
                requester_ip: None,
                prompt_cache_key: Some("pck-dashboard-subscription-live-current".to_string()),
                sticky_key: None,
                route_mode: None,
                upstream_account_id: Some(99),
                upstream_account_name: Some("Live Overlay Account".to_string()),
                response_content_encoding: None,
                request_compression_algorithm: None,
                transport: None,
                pool_attempt_count: None,
                pool_distinct_account_count: None,
                pool_attempt_terminal_reason: None,
                requested_service_tier: None,
                service_tier: None,
                billing_service_tier: None,
                proxy_weight_delta: None,
                cost_estimated: None,
                cost_audit: None,
                price_version: None,
                request_raw_path: None,
                request_raw_size: None,
                request_raw_truncated: None,
                request_raw_truncated_reason: None,
                response_raw_path: None,
                response_raw_size: None,
                response_raw_truncated: None,
                response_raw_truncated_reason: None,
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                detail_pruned_at: None,
                detail_prune_reason: None,
                t_total_ms: Some(640.0),
                t_req_read_ms: Some(0.0),
                t_req_parse_ms: Some(0.0),
                t_upstream_connect_ms: Some(0.0),
                t_upstream_ttfb_ms: Some(80.0),
                first_token_ms: None,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
                created_at: occurred_at.clone(),
            },
            Utc::now(),
        );

    let current_capture = state
        .proxy_runtime_invocations
        .capture_memory_snapshot()
        .expect("capture dashboard current projection slice");
    let network_capture = state
        .proxy_runtime_invocations
        .capture_network_slice()
        .expect("capture dashboard network projection slice");
    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardCurrentSlice {
                slice: Box::new(crate::DashboardCurrentProjectionSlice::from(
                    &current_capture.snapshot,
                )),
            },
        )
        .await;
    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardNetworkSlice {
                slice: Box::new(network_capture.slice),
            },
        )
        .await;

    let updated_prepared = state
        .subscription_hub
        .prepare_connection(
            state.clone(),
            vec![dashboard_activity_topic_descriptor()],
            vec![],
        )
        .await
        .expect("prepare updated dashboard activity subscription snapshot");
    let updated_payload = extract_subscription_snapshot_payload(updated_prepared);
    let updated_accounts = updated_payload
        .get("accounts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let live_only = updated_accounts
        .iter()
        .find(|account| account.get("accountKey").and_then(Value::as_str) == Some("upstream:99"))
        .expect("live overlay account should be injected into cached topic snapshot");
    assert_eq!(
        live_only
            .get("inProgressInvocationCount")
            .and_then(Value::as_i64),
        Some(1)
    );
    assert_eq!(
        live_only.get("tokensPerMinute").and_then(Value::as_f64),
        Some(321.0)
    );
    assert_eq!(
        live_only.get("spendRate").and_then(Value::as_f64),
        Some(0.32)
    );
}

#[tokio::test]
async fn dashboard_activity_subscription_terminal_slice_updates_without_ttl_refresh() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let initial_prepared = state
        .subscription_hub
        .prepare_connection(
            state.clone(),
            vec![dashboard_activity_topic_descriptor()],
            vec![],
        )
        .await
        .expect("prepare initial dashboard activity subscription snapshot");
    let initial_payload = extract_subscription_snapshot_payload(initial_prepared);
    let _dashboard_lease = state
        .subscription_hub
        .register_test_topic_name("dashboard.activity.current")
        .await;
    assert_eq!(
        initial_payload
            .get("summary")
            .and_then(Value::as_object)
            .and_then(|summary| summary.get("stats"))
            .and_then(Value::as_object)
            .and_then(|stats| stats.get("totalCount"))
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        0
    );

    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(93_101_i64)
    .bind("dashboard-subscription-terminal-refresh")
    .bind(occurred_at.as_str())
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(100_i64)
    .bind(0.01_f64)
    .bind(json!({ "upstreamAccountId": 42_i64 }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert terminal invocation for dashboard topic materialization");

    let terminal_record = crate::api::ApiInvocation {
        id: 93_101_i64,
        invoke_id: "dashboard-subscription-terminal-refresh".to_string(),
        occurred_at: occurred_at.clone(),
        source: SOURCE_PROXY.to_string(),
        proxy_display_name: None,
        model: Some("gpt-5".to_string()),
        request_model: Some("gpt-5".to_string()),
        response_model: Some("gpt-5".to_string()),
        input_tokens: Some(50),
        output_tokens: Some(50),
        cache_input_tokens: Some(0),
        reported_cache_write_tokens: None,
        reasoning_tokens: Some(0),
        reasoning_effort: None,
        total_tokens: Some(100),
        cost: Some(0.01),
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        cache_write_tokens: Some(50),
        status: Some("success".to_string()),
        live_phase: None,
        error_message: None,
        downstream_status_code: None,
        failure_kind: None,
        blocked_binding: None,
        blocked_binding_json: None,
        stream_terminal_event: None,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        failure_class: None,
        is_actionable: None,
        endpoint: Some("/v1/responses".to_string()),
        compaction_request_kind: None,
        compaction_response_kind: None,
        image_intent: None,
        requester_ip: None,
        prompt_cache_key: Some("pck-dashboard-subscription-terminal-refresh".to_string()),
        sticky_key: None,
        route_mode: None,
        upstream_account_id: Some(42),
        upstream_account_name: Some("Deferred Refresh Account".to_string()),
        response_content_encoding: None,
        request_compression_algorithm: None,
        transport: None,
        pool_attempt_count: None,
        pool_distinct_account_count: None,
        pool_attempt_terminal_reason: None,
        requested_service_tier: None,
        service_tier: None,
        billing_service_tier: None,
        proxy_weight_delta: None,
        cost_estimated: None,
        cost_audit: None,
        price_version: None,
        request_raw_path: None,
        request_raw_size: None,
        request_raw_truncated: None,
        request_raw_truncated_reason: None,
        response_raw_path: None,
        response_raw_size: None,
        response_raw_truncated: None,
        response_raw_truncated_reason: None,
        detail_level: DETAIL_LEVEL_FULL.to_string(),
        detail_pruned_at: None,
        detail_prune_reason: None,
        t_total_ms: Some(100.0),
        t_req_read_ms: Some(0.0),
        t_req_parse_ms: Some(0.0),
        t_upstream_connect_ms: Some(0.0),
        t_upstream_ttfb_ms: Some(10.0),
        first_token_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
        created_at: occurred_at.clone(),
    };
    let terminal_delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal_record)
        .await
        .terminal_delta
        .expect("accept persisted terminal delta");
    state
        .subscription_hub
        .handle_internal_broadcast(
            state.clone(),
            BroadcastPayload::DashboardTerminalSlice {
                slice: Box::new(DashboardTerminalProjectionSlice {
                    revision: 1,
                    deltas: vec![terminal_delta],
                }),
            },
        )
        .await;

    let immediate_prepared = state
        .subscription_hub
        .prepare_connection(
            state.clone(),
            vec![dashboard_activity_topic_descriptor()],
            vec![],
        )
        .await
        .expect("prepare dashboard activity topic after terminal slice materialization");
    let immediate_payload = extract_subscription_snapshot_payload(immediate_prepared);
    assert_eq!(
        immediate_payload
            .get("summary")
            .and_then(Value::as_object)
            .and_then(|summary| summary.get("stats"))
            .and_then(Value::as_object)
            .and_then(|stats| stats.get("totalCount"))
            .and_then(Value::as_i64)
            .unwrap_or_default(),
        1
    );
}

#[tokio::test]
async fn dashboard_activity_summary_only_excludes_archived_rows_for_live_ids() {
    let mut config = test_config();
    config.invocation_max_days = 0;
    let state = test_state_from_config(config, true).await;

    let range_start = Utc::now() - ChronoDuration::days(1);
    let overlap_at = format_naive(
        (range_start + ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(92_001_i64)
    .bind("dashboard-summary-only-live-overlap")
    .bind(overlap_at.as_str())
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(100_i64)
    .bind(0.10_f64)
    .bind(json!({ "promptCacheKey": "pck-dashboard-summary-only-live-overlap" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert live summary-only overlap invocation");

    seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "dashboard-summary-only-overlap-live-id",
        &[SeedInvocationArchiveBatchRow {
            id: 92_001_i64,
            invoke_id: "dashboard-summary-only-live-overlap",
            occurred_at: overlap_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 100_i64,
            cost: 0.10_f64,
            ttfb_ms: None,
            payload: Some(r#"{"promptCacheKey":"pck-dashboard-summary-only-live-overlap"}"#),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;

    let Json(response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "1d".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: false,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch summary-only dashboard activity with archive/live overlap");
    assert_eq!(response.summary.stats.total_count, 1);
    assert_eq!(response.summary.stats.total_tokens, 100);
    assert_f64_close(response.summary.stats.total_cost, 0.10);
}

#[tokio::test]
async fn dashboard_activity_cache_selection_reuses_today_snapshot_within_reconcile_interval() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(first_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch first rolling dashboard activity snapshot");

    let first_range_end =
        parse_to_utc_datetime(&first_response.range_end).expect("first range end should parse");
    tokio::time::sleep(std::time::Duration::from_millis(2_100)).await;

    let Json(second_response) = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(2),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: true,
            include_recent: Some(false),
        }),
    )
    .await
    .expect("fetch second rolling dashboard activity snapshot");

    let second_range_end =
        parse_to_utc_datetime(&second_response.range_end).expect("second range end should parse");
    assert!(second_range_end > first_range_end);
    assert!(second_response.snapshot_id > first_response.snapshot_id);
    let cache = state.dashboard_activity_snapshot_cache.lock().await;
    assert_eq!(cache.entries.len(), 1);
    let selection = cache
        .entries
        .keys()
        .next()
        .expect("cached dashboard selection");
    assert_eq!(selection.range, "today");
    assert!(!selection.range_anchor.is_empty());
    assert!(cache.in_flight.is_empty());
}
