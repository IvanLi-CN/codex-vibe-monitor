async fn seed_pending_streaming_invocation(pool: &SqlitePool, invoke_id: &str, occurred_at: &str) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            payload,
            raw_response,
            request_raw_codec,
            first_token_ms
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(r#"{"endpoint":"/v1/responses"}"#)
    .bind(r#"{}"#)
    .bind("identity")
    .bind(720.0)
    .execute(pool)
    .await
    .expect("insert running invocation row");
}

async fn seed_pending_streaming_attempt(
    pool: &SqlitePool,
    trace: &PoolUpstreamAttemptTraceContext,
    occurred_at: &str,
) {
    insert_pool_upstream_request_attempt_with_scope(
        pool,
        trace,
        None,
        None,
        None,
        None,
        Some(42),
        Some("https://api.openai.com"),
        2,
        1,
        0,
        Some(occurred_at),
        None,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE),
        None,
        None,
        None,
        None,
        None,
        Some(10.0),
        Some(180.0),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("insert pending streaming pool attempt row");
}

async fn seed_failed_predecessor_attempt(
    pool: &SqlitePool,
    trace: &PoolUpstreamAttemptTraceContext,
    occurred_at: &str,
) {
    insert_pool_upstream_request_attempt_with_scope(
        pool,
        trace,
        None,
        None,
        None,
        None,
        Some(41),
        Some("https://api.openai.com"),
        1,
        1,
        0,
        Some(occurred_at),
        Some(occurred_at),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED),
        Some(StatusCode::BAD_GATEWAY),
        None,
        Some("upstream_failure"),
        Some("first attempt failed"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("insert failed predecessor pool attempt row");
}

#[tokio::test]
pub(crate) async fn query_pool_attempt_records_from_live_keeps_pending_streaming_ttft() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");
    let invoke_id = "pool-pending-streaming-query";
    let occurred_at = "2026-07-15 12:05:00";
    seed_pending_streaming_invocation(&pool, invoke_id, occurred_at).await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: None,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: None,
    };
    seed_pending_streaming_attempt(&pool, &trace, occurred_at).await;

    let rows = query_pool_attempt_records_from_live(&pool, invoke_id)
        .await
        .expect("query pool attempt records");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].first_token_ms, Some(720.0));
    assert_eq!(rows[0].stream_latency_ms, None);
    seed_failed_predecessor_attempt(&pool, &trace, occurred_at).await;

    let retry_rows = query_pool_attempt_records_from_live(&pool, invoke_id)
        .await
        .expect("query retry pool attempt records");
    assert_eq!(retry_rows.len(), 2);
    assert_eq!(retry_rows[0].first_token_ms, None);
    assert_eq!(retry_rows[1].first_token_ms, Some(720.0));
}

#[test]
pub(crate) fn prepare_target_request_body_extracts_reasoning_effort_for_responses() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "reasoning": {
            "effort": "high"
        }
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert_eq!(info.reasoning_effort.as_deref(), Some("high"));
}

#[test]
pub(crate) fn prepare_target_request_body_detects_remote_v2_compaction_requests() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "context_management": [
            {
                "type": "compaction",
                "compact_threshold": 0.82
            }
        ]
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert_eq!(
        info.compaction_request_kind
            .map(CompactionKind::as_payload_str),
        Some("remote_v2")
    );
}

#[test]
pub(crate) fn prepare_target_request_body_detects_remote_v2_compaction_request_object_shape() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "context_management": {
            "type": "compaction",
            "compact_threshold": 0.82
        }
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert_eq!(
        info.compaction_request_kind
            .map(CompactionKind::as_payload_str),
        Some("remote_v2")
    );
}

#[test]
pub(crate) fn prepare_target_request_body_extracts_reasoning_effort_for_chat_completions() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "messages": [{"role": "user", "content": "hi"}],
        "reasoning_effort": "medium"
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::ChatCompletions, body, true);

    assert_eq!(info.reasoning_effort.as_deref(), Some("medium"));
}

#[test]
pub(crate) fn extract_requested_service_tier_from_request_body_reads_top_level_aliases() {
    let snake_case = json!({ "service_tier": " Priority " });
    let camel_case = json!({ "serviceTier": "PRIORITY" });

    assert_eq!(
        extract_requested_service_tier_from_request_body(&snake_case).as_deref(),
        Some("priority")
    );
    assert_eq!(
        extract_requested_service_tier_from_request_body(&camel_case).as_deref(),
        Some("priority")
    );
}

#[test]
pub(crate) fn extract_requested_service_tier_from_request_body_ignores_nested_or_non_string_values()
{
    let nested = json!({
        "response": { "service_tier": "priority" },
        "metadata": { "serviceTier": "priority" }
    });
    let non_string = json!({ "service_tier": true });
    let blank = json!({ "serviceTier": "   " });

    assert_eq!(
        extract_requested_service_tier_from_request_body(&nested),
        None
    );
    assert_eq!(
        extract_requested_service_tier_from_request_body(&non_string),
        None
    );
    assert_eq!(
        extract_requested_service_tier_from_request_body(&blank),
        None
    );
}

#[test]
pub(crate) fn extract_requester_ip_uses_expected_header_priority() {
    let mut preferred = HeaderMap::new();
    preferred.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static("198.51.100.10, 203.0.113.9"),
    );
    preferred.insert(
        HeaderName::from_static("x-real-ip"),
        HeaderValue::from_static("203.0.113.5"),
    );
    preferred.insert(
        HeaderName::from_static("forwarded"),
        HeaderValue::from_static("for=192.0.2.60;proto=https"),
    );
    assert_eq!(
        extract_requester_ip(&preferred, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
        Some("198.51.100.10")
    );

    let mut fallback_forwarded = HeaderMap::new();
    fallback_forwarded.insert(
        HeaderName::from_static("forwarded"),
        HeaderValue::from_static("for=\"[2001:db8::1]:443\";proto=https"),
    );
    assert_eq!(
        extract_requester_ip(&fallback_forwarded, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
        Some("2001:db8::1")
    );

    let no_headers = HeaderMap::new();
    assert_eq!(
        extract_requester_ip(&no_headers, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
        Some("127.0.0.1")
    );
}

#[test]
pub(crate) fn extract_prompt_cache_key_from_headers_reads_whitelist_keys() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-prompt-cache-key"),
        HeaderValue::from_static("pck-from-header"),
    );
    assert_eq!(
        extract_prompt_cache_key_from_headers(&headers).as_deref(),
        Some("pck-from-header")
    );
}

#[test]
pub(crate) fn extract_sticky_key_from_headers_accepts_sticky_and_prompt_cache_aliases() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-prompt-cache-key"),
        HeaderValue::from_static("pck-from-header"),
    );
    headers.insert(
        HeaderName::from_static("x-sticky-key"),
        HeaderValue::from_static("sticky-from-header"),
    );

    assert_eq!(
        extract_sticky_key_from_headers(&headers).as_deref(),
        Some("sticky-from-header")
    );
    assert_eq!(
        extract_prompt_cache_key_from_headers(&headers).as_deref(),
        Some("pck-from-header")
    );
}

#[test]
pub(crate) fn parse_stream_response_payload_extracts_usage_and_model() {
    let raw = [
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}],\"usage\":null}",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[],\"service_tier\":\"priority\",\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":18}}",
        "data: [DONE]",
    ]
    .join("\n");
    let parsed = parse_stream_response_payload(raw.as_bytes());
    assert_eq!(parsed.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(parsed.usage.input_tokens, Some(11));
    assert_eq!(parsed.usage.output_tokens, Some(7));
    assert_eq!(parsed.usage.total_tokens, Some(18));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
pub(crate) fn parse_stream_response_payload_extracts_terminal_failure_details() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"id":"resp_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "event: response.failed",
        r#"data: {"type":"response.failed","response":{"id":"resp_test","model":"gpt-5.4","status":"failed","error":{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}}"#,
    ]
    .join("\n");

    let parsed = parse_stream_response_payload(raw.as_bytes());
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(
        parsed.stream_terminal_event.as_deref(),
        Some("response.failed")
    );
    assert_eq!(parsed.upstream_error_code.as_deref(), Some("server_error"));
    assert!(
        parsed.upstream_error_message.as_deref().is_some_and(
            |message| message.contains("request ID 060a328d-5cb6-433c-9025-1da2d9c632f1")
        )
    );
    assert_eq!(
        parsed.upstream_request_id.as_deref(),
        Some("060a328d-5cb6-433c-9025-1da2d9c632f1")
    );
    assert_eq!(
        parsed.usage_missing_reason.as_deref(),
        Some("upstream_response_failed")
    );
}

#[test]
pub(crate) fn stream_response_parser_recognizes_only_strict_successful_completion() {
    let mut parser = StreamResponsePayloadChunkParser::default();
    parser.ingest_bytes(b"event: response.completed\n");
    assert!(!parser.successful_terminal_seen());
    parser.ingest_bytes(
        b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n",
    );
    assert!(parser.successful_terminal_seen());
    assert!(parser.finish().successful_terminal_seen);

    let mut eof_parser = StreamResponsePayloadChunkParser::default();
    eof_parser.ingest_bytes(b"event: response.completed\n");
    eof_parser.ingest_bytes(
        b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}",
    );
    assert!(eof_parser.successful_terminal_seen());

    for payload in [
        [
            "event: response.completed",
            r#"data: {"type":"response.completed","response":{"status":"failed"}}"#,
        ]
        .join("\n"),
        [
            "event: response.completed",
            r#"data: {"type":"response.failed","response":{"status":"completed"}}"#,
        ]
        .join("\n"),
        [
            "event: response.completed",
            r#"data: {"type":"response.completed","response":{}}"#,
        ]
        .join("\n"),
        [
            "event: response.completed",
            r#"data: {"type":"response.completed","response":{"status":"completed"}"#,
        ]
        .join("\n"),
        [
            "event: response.completed",
            r#"data: {"response":{"status":"completed"}}"#,
        ]
        .join("\n"),
        r#"data: {"type":"response.completed","response":{"status":"completed"}}"#.to_string(),
    ] {
        let mut parser = StreamResponsePayloadChunkParser::default();
        parser.ingest_bytes(payload.as_bytes());
        parser.flush_pending_line();
        assert!(
            !parser.successful_terminal_seen(),
            "invalid terminal payload must not establish success: {payload}"
        );
    }
}

#[test]
pub(crate) fn estimate_proxy_cost_subtracts_cached_tokens_from_base_input_rate() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-test".to_string(),
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: Some(0.5),
                cache_read_per_1m: Some(0.5),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, price_version) = estimate_proxy_cost(
        &catalog,
        Some("gpt-test"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((600.0 * 1.0) + (200.0 * 2.0) + (400.0 * 0.5)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
    assert_eq!(price_version.as_deref(), Some("unit-test@response-tier"));
}

#[test]
pub(crate) fn estimate_proxy_cost_keeps_full_input_when_cache_price_missing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-test".to_string(),
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-test"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((1_000.0 * 1.0) + (200.0 * 2.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_breakdown_uses_explicit_gpt_5_6_sol_cache_read_and_write_prices()
{
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.6-sol".to_string(),
            ModelPricing {
                input_per_1m: 5.0,
                output_per_1m: 30.0,
                cache_input_per_1m: Some(0.5),
                cache_read_per_1m: Some(0.5),
                cache_write_per_1m: Some(6.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (breakdown, estimated, price_version) = estimate_proxy_cost_breakdown(
        &catalog,
        Some("gpt-5.6-sol"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((600.0 * 6.25) + (400.0 * 0.5) + (200.0 * 30.0)) / 1_000_000.0;
    let computed = breakdown.expect("cost breakdown should be present");
    assert_eq!(computed.input, 0.0);
    assert!((computed.cache_write - (600.0 * 6.25 / 1_000_000.0)).abs() < 1e-12);
    assert!((computed.cache_read - (400.0 * 0.5 / 1_000_000.0)).abs() < 1e-12);
    assert!((computed.output - (200.0 * 30.0 / 1_000_000.0)).abs() < 1e-12);
    assert_eq!(computed.reasoning, 0.0);
    assert!((computed.total() - expected).abs() < 1e-12);
    assert!(estimated);
    assert_eq!(price_version.as_deref(), Some("unit-test@response-tier"));
}

#[test]
pub(crate) fn estimate_proxy_cost_falls_back_to_dated_gpt_5_6_terra_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.6-terra".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 12.0,
                cache_input_per_1m: Some(0.20),
                cache_read_per_1m: Some(0.20),
                cache_write_per_1m: Some(2.5),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.6-terra-2026-07-08"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((600.0 * 2.5) + (400.0 * 0.20) + (200.0 * 12.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_falls_back_to_dated_gpt_5_6_luna_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.6-luna".to_string(),
            ModelPricing {
                input_per_1m: 0.20,
                output_per_1m: 1.20,
                cache_input_per_1m: Some(0.02),
                cache_read_per_1m: Some(0.02),
                cache_write_per_1m: Some(0.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.6-luna-2026-07-08"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((600.0 * 0.25) + (400.0 * 0.02) + (200.0 * 1.20)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_falls_back_to_dated_model_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1000),
        output_tokens: Some(500),
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1500),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.2-2025-12-11"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((1000.0 * 2.0) + (500.0 * 3.0)) / 1_000_000.0;
    assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_prefers_exact_model_over_dated_model_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([
            (
                "gpt-5.2".to_string(),
                ModelPricing {
                    input_per_1m: 1.0,
                    output_per_1m: 1.0,
                    cache_input_per_1m: None,
                    cache_read_per_1m: None,
                    cache_write_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
            (
                "gpt-5.2-2025-12-11".to_string(),
                ModelPricing {
                    input_per_1m: 4.0,
                    output_per_1m: 5.0,
                    cache_input_per_1m: None,
                    cache_read_per_1m: None,
                    cache_write_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
        ]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1000),
        output_tokens: Some(1000),
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(2000),
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.2-2025-12-11"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((1000.0 * 4.0) + (1000.0 * 5.0)) / 1_000_000.0;
    assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_at_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((271_000.0 * 2.5) + (1_000.0 * 0.25) + (1_000.0 * 15.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_above_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_to_reasoning_cost() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: None,
                reasoning_per_1m: Some(20.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: Some(2_000),
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let reasoning_part = (2_000.0 * 20.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_above_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4-pro".to_string(),
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(999_999),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4-pro"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = (272_001.0 * 30.0) / 1_000_000.0;
    let output_part = (1_000.0 * 180.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_for_dated_model_suffix()
 {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4-pro".to_string(),
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: Some(90.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(999_999),
        reasoning_tokens: Some(2_000),
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4-pro-2026-03-01"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let input_part = (272_001.0 * 30.0) / 1_000_000.0;
    let output_part = (1_000.0 * 180.0) / 1_000_000.0;
    let reasoning_part = (2_000.0 * 90.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

use super::*;
