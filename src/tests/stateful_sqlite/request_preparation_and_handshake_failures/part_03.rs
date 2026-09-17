#[test]
pub(crate) fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_for_dated_model_suffix() {
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
        Some("gpt-5.4-2026-03-01"),
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
pub(crate) fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_for_other_models() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4o".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
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
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4o"),
        &usage,
        Some("default"),
        ProxyPricingMode::ResponseTier,
    );

    let expected = ((272_001.0 * 2.5) + (1_000.0 * 15.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
pub(crate) fn estimate_proxy_cost_applies_requested_tier_priority_multiplier_and_price_version_suffix()
 {
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
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: Some(50),
        total_tokens: Some(1_200),
    };

    let (cost, estimated, price_version) = estimate_proxy_cost(
        &catalog,
        Some("gpt-5.4"),
        &usage,
        Some("priority"),
        ProxyPricingMode::RequestedTier,
    );

    let base_expected =
        ((600.0 * 2.5) + (400.0 * 0.25) + (200.0 * 15.0) + (50.0 * 20.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - (base_expected * 2.0)).abs() < 1e-12);
    assert!(estimated);
    assert_eq!(price_version.as_deref(), Some("unit-test@requested-tier"));
}

#[test]
pub(crate) fn resolve_proxy_billing_service_tier_and_pricing_mode_prefers_requested_tier_for_api_keys()
 {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        Some("priority"),
        Some("default"),
        Some("api_key_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("priority"));
    assert_eq!(pricing_mode, ProxyPricingMode::RequestedTier);
}

#[test]
pub(crate) fn resolve_proxy_billing_service_tier_and_pricing_mode_prefers_explicit_billing_metadata()
 {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        Some("default"),
        Some("priority"),
        Some("priority"),
        Some("api_key_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("default"));
    assert_eq!(pricing_mode, ProxyPricingMode::ExplicitBilling);
}

#[test]
pub(crate) fn resolve_proxy_billing_service_tier_and_pricing_mode_keeps_response_tier_for_non_api_keys()
 {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        Some("priority"),
        Some("default"),
        Some("oauth_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("default"));
    assert_eq!(pricing_mode, ProxyPricingMode::ResponseTier);
}

#[test]
pub(crate) fn resolve_proxy_billing_service_tier_and_pricing_mode_falls_back_to_response_tier_when_api_keys_request_is_missing()
 {
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        None,
        Some("default"),
        Some("api_key_codex"),
    );

    assert_eq!(billing_service_tier.as_deref(), Some("default"));
    assert_eq!(pricing_mode, ProxyPricingMode::ResponseTier);
}

#[test]
pub(crate) fn parse_target_response_payload_decodes_gzip_stream_usage() {
    let raw = [
        "event: response.created",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
        "",
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        "",
    ]
    .join("\n");

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        &compressed,
        true,
        Some("gzip"),
    );
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.input_tokens, Some(12));
    assert_eq!(parsed.usage.output_tokens, Some(3));
    assert_eq!(parsed.usage.cache_input_tokens, Some(2));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
pub(crate) fn parse_target_response_payload_decodes_multi_value_content_encoding() {
    let raw = [
        "event: response.created",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
        "",
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"flex\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        "",
    ]
    .join("\n");

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        &compressed,
        true,
        Some("identity, gzip"),
    );
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.input_tokens, Some(12));
    assert_eq!(parsed.usage.output_tokens, Some(3));
    assert_eq!(parsed.usage.cache_input_tokens, Some(2));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert_eq!(parsed.service_tier.as_deref(), Some("flex"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
pub(crate) fn parse_target_response_payload_detects_sse_without_request_stream_hint() {
    let raw = [
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","service_tier":"priority","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), false, None);

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
pub(crate) fn parse_target_response_payload_prefers_terminal_stream_service_tier_over_initial_auto()
{
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"auto"}}"#,
        "",
        "event: response.in_progress",
        r#"data: {"type":"response.in_progress","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"auto"}}"#,
        "",
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.4","status":"completed","service_tier":"default","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), true, None);

    assert_eq!(parsed.service_tier.as_deref(), Some("default"));
    assert_eq!(parsed.usage.total_tokens, Some(15));
}

#[test]
pub(crate) fn parse_target_response_payload_does_not_downgrade_same_rank_stream_tier_to_auto() {
    let raw = [
        "event: response.created",
        r#"data: {"type":"response.created","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"default"}}"#,
        "",
        "event: response.in_progress",
        r#"data: {"type":"response.in_progress","response":{"model":"gpt-5.4","status":"in_progress","service_tier":"auto"}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), true, None);

    assert_eq!(parsed.service_tier.as_deref(), Some("default"));
}

#[test]
pub(crate) fn parse_target_response_payload_from_raw_file_falls_back_to_raw_deflate_streams() {
    let raw = [
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","usage":{"input_tokens":17,"output_tokens":4,"total_tokens":21}}}"#,
        "",
    ]
    .join("\n");

    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write raw deflate stream");
    let compressed = encoder.finish().expect("finish raw deflate stream");

    let temp_dir = make_temp_test_dir("raw-deflate-response");
    let raw_path = temp_dir.join("response.bin");
    fs::write(&raw_path, compressed).expect("write raw deflate response payload");

    let parsed = parse_target_response_payload_from_raw_file(
        ProxyCaptureTarget::Responses,
        &raw_path,
        true,
        Some("deflate"),
    )
    .expect("parse raw deflate response payload");

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.total_tokens, Some(21));
    assert!(parsed.usage_missing_reason.is_none());

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
pub(crate) fn parse_target_response_payload_reads_service_tier_from_response_object() {
    let raw = json!({
        "id": "resp_json_1",
        "response": {
            "model": "gpt-5.3-codex",
            "service_tier": "priority",
            "usage": {
                "input_tokens": 21,
                "output_tokens": 5,
                "total_tokens": 26
            }
        }
    });

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        serde_json::to_string(&raw)
            .expect("serialize raw payload")
            .as_bytes(),
        false,
        None,
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert_eq!(parsed.usage.total_tokens, Some(26));
}

#[test]
pub(crate) fn standalone_search_response_does_not_infer_usage_from_results() {
    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::StandaloneSearch,
        br#"{
            "model": "gpt-5.4",
            "results": [{"title": "result", "content": "hello"}]
        }"#,
        false,
        None,
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.4"));
    assert!(parsed.usage.input_tokens.is_none());
    assert!(parsed.usage.output_tokens.is_none());
    assert!(parsed.usage.total_tokens.is_none());
    assert_eq!(
        parsed.usage_missing_reason.as_deref(),
        Some("usage_missing_in_response")
    );

    let sse_like = parse_target_response_payload(
        ProxyCaptureTarget::StandaloneSearch,
        b"event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n",
        false,
        None,
    );
    assert!(sse_like.stream_terminal_event.is_none());
    assert_eq!(
        sse_like.usage_missing_reason.as_deref(),
        Some("response_not_json")
    );

    let mut search_stream_parser =
        StreamResponsePayloadChunkParser::for_target(ProxyCaptureTarget::StandaloneSearch);
    search_stream_parser.ingest_bytes(
        b"event: response.failed\ndata: {\"type\":\"response.failed\",\"object\":\"response.compaction\"}\n\n",
    );
    let search_stream_outcome = search_stream_parser.finish();
    assert!(!search_stream_outcome.saw_stream_fields);
    assert!(!search_stream_outcome.successful_terminal_seen);
    assert!(
        search_stream_outcome
            .response_info
            .compaction_response_kind
            .is_none()
    );
    assert!(
        search_stream_outcome
            .response_info
            .stream_terminal_event
            .is_none()
    );

    let forced_stream_hint = parse_target_response_payload(
        ProxyCaptureTarget::StandaloneSearch,
        br#"{"object":"response.compaction","results":[]}"#,
        true,
        None,
    );
    assert!(forced_stream_hint.compaction_response_kind.is_none());
    assert!(forced_stream_hint.stream_terminal_event.is_none());
}

#[test]
pub(crate) fn parse_target_response_payload_detects_remote_v2_compaction_stream_events() {
    let raw = [
        "event: response.output_item.added",
        r#"data: {"type":"response.output_item.added","item":{"id":"cmp_001","type":"compaction","encrypted_content":"enc"}} "#,
        "",
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), true, None);

    assert_eq!(
        parsed
            .compaction_response_kind
            .map(CompactionKind::as_payload_str),
        Some("remote_v2")
    );
}

#[test]
pub(crate) fn parse_target_response_payload_detects_response_compaction_json_shape() {
    let raw = json!({
        "id": "resp_compact_test",
        "object": "response.compaction",
        "output": [
            {
                "id": "cmp_001",
                "type": "compaction",
                "encrypted_content": "encrypted-summary"
            }
        ],
        "usage": {
            "input_tokens": 139,
            "output_tokens": 438,
            "total_tokens": 577
        }
    });

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        serde_json::to_string(&raw)
            .expect("serialize raw payload")
            .as_bytes(),
        false,
        None,
    );

    assert_eq!(
        parsed
            .compaction_response_kind
            .map(CompactionKind::as_payload_str),
        Some("remote_v2")
    );
}

#[test]
pub(crate) fn compact_responses_keep_legacy_compaction_kind_even_with_compaction_payload_shape() {
    let raw = json!({
        "id": "resp_compact_test",
        "object": "response.compaction",
        "output": [
            {
                "id": "cmp_001",
                "type": "compaction",
                "encrypted_content": "encrypted-summary"
            }
        ],
        "usage": {
            "input_tokens": 139,
            "output_tokens": 438,
            "total_tokens": 577
        }
    });

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::ResponsesCompact,
        serde_json::to_string(&raw)
            .expect("serialize raw payload")
            .as_bytes(),
        false,
        None,
    );

    assert_eq!(
        parsed
            .compaction_response_kind
            .map(CompactionKind::as_payload_str),
        Some("remote_v2")
    );

    assert_eq!(
        resolve_compaction_response_kind_for_payload(
            ProxyCaptureTarget::ResponsesCompact,
            parsed.compaction_response_kind
        )
        .map(CompactionKind::as_payload_str),
        Some("compact")
    );
}

#[test]
pub(crate) fn parse_target_response_payload_records_decode_failure_reason() {
    let raw = [
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2,\"total_tokens\":12}}}",
        "data: [DONE]",
    ]
    .join("\n");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        raw.as_bytes(),
        true,
        Some("gzip"),
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.total_tokens, Some(12));
    assert!(
        parsed
            .usage_missing_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("response_decode_failed:gzip:"))
    );
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_extracts_usage_from_gzip_response_stream() {
    #[derive(sqlx::FromRow)]
    struct PersistedUsageRow {
        source: String,
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        cache_input_tokens: Option<i64>,
        total_tokens: Option<i64>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static("198.51.100.42, 203.0.113.10"),
    );
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses?mode=gzip".parse().expect("valid uri")),
        Method::POST,
        headers,
        Body::from(
            r#"{"model":"gpt-5.3-codex","stream":true,"metadata":{"prompt_cache_key":"pck-gzip-1"},"input":"hello"}"#,
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");

    let mut row: Option<PersistedUsageRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedUsageRow>(
            r#"
            SELECT source, status, input_tokens, output_tokens, cache_input_tokens, total_tokens, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");

        if row
            .as_ref()
            .is_some_and(|record| record.input_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("capture record should exist");
    assert_eq!(row.source, SOURCE_PROXY);
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(12));
    assert_eq!(row.output_tokens, Some(3));
    assert_eq!(row.cache_input_tokens, Some(2));
    assert_eq!(row.total_tokens, Some(15));
    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode payload summary");
    assert_eq!(payload["endpoint"], "/v1/responses");
    assert!(payload["usageMissingReason"].is_null());
    assert_eq!(payload["requesterIp"], "198.51.100.42");
    assert_eq!(payload["promptCacheKey"], "pck-gzip-1");
    assert!(
        payload["proxyWeightDelta"].is_number(),
        "proxy weight delta should be recorded for fresh proxy attempts"
    );

    upstream_handle.abort();
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_gzip_stream_without_event_stream_header_still_extracts_usage()
 {
    #[derive(sqlx::FromRow)]
    struct PersistedUsageRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=gzip-no-content-type"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.3-codex","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");

    let mut row: Option<PersistedUsageRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedUsageRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query gzip stream usage row without event-stream header");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("gzip stream usage row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(19));
    assert_eq!(row.output_tokens, Some(6));
    assert_eq!(row.total_tokens, Some(25));
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
}

pub(crate) fn reset_proxy_capture_hot_path_raw_fallbacks() {
    reset_response_capture_raw_fallback_counters();
}

pub(crate) fn assert_proxy_capture_hot_path_skips_raw_fallbacks() {
    let (sse_hint_fallbacks, parse_fallbacks) = response_capture_raw_fallback_counts();
    assert_eq!(
        sse_hint_fallbacks, 0,
        "proxy capture hot path should not reread raw files for SSE hinting"
    );
    assert_eq!(
        parse_fallbacks, 0,
        "proxy capture hot path should not reread raw files for response parsing"
    );
}

#[derive(sqlx::FromRow)]
struct PersistedLargeGzipUsageRow {
    status: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    response_raw_path: Option<String>,
    response_raw_size: Option<i64>,
    payload: Option<String>,
}

async fn wait_for_large_gzip_usage_row(pool: &SqlitePool) -> PersistedLargeGzipUsageRow {
    for _ in 0..50 {
        let row = sqlx::query_as::<_, PersistedLargeGzipUsageRow>(
            r#"SELECT status, input_tokens, output_tokens, total_tokens,
                      response_raw_path, response_raw_size, payload
               FROM codex_invocations ORDER BY id DESC LIMIT 1"#,
        )
        .fetch_optional(pool)
        .await
        .expect("query large gzip stream usage row without event-stream header");
        if row
            .as_ref()
            .is_some_and(|record| record.response_raw_path.is_some() && record.payload.is_some())
        {
            return row.expect("checked row presence");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("large gzip stream usage row should exist");
}

#[derive(sqlx::FromRow)]
struct PersistedLargeStreamRow {
    status: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    raw_response: String,
    response_raw_path: Option<String>,
    response_raw_size: Option<i64>,
    payload: Option<String>,
}

async fn wait_for_large_stream_row(pool: &SqlitePool) -> PersistedLargeStreamRow {
    for _ in 0..50 {
        let row = sqlx::query_as::<_, PersistedLargeStreamRow>(
            r#"SELECT status, input_tokens, output_tokens, total_tokens, raw_response,
                      response_raw_path, response_raw_size, payload
               FROM codex_invocations ORDER BY id DESC LIMIT 1"#,
        )
        .fetch_optional(pool)
        .await
        .expect("query large stream capture row");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            return row.expect("checked row presence");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("large stream capture row should exist");
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_large_gzip_stream_without_event_stream_header_keeps_raw_capture_without_raw_reread()
 {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-gzip-stream-no-content-type");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=gzip-large-no-content-type"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.3-codex","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");

    let row = wait_for_large_gzip_usage_row(&state.pool).await;
    assert_eq!(row.status.as_deref(), Some("success"));
    assert!(
        row.response_raw_size
            .is_some_and(|size| size > RAW_RESPONSE_PREVIEW_LIMIT as i64)
    );
    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes = read_proxy_raw_bytes(response_raw_path, None)
        .expect("read persisted large gzip response raw");
    let (decoded_raw_bytes, decode_failure_reason) =
        decode_response_payload_for_usage(&raw_bytes, Some("gzip"));
    assert!(
        decode_failure_reason.is_none(),
        "persisted raw gzip response should remain decodable"
    );
    let raw_text = String::from_utf8(decoded_raw_bytes.into_owned())
        .expect("raw gzip response should decode to utf8");
    assert!(raw_text.contains("response.completed"));
    assert!(raw_text.contains("\"total_tokens\":30"));
    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode large gzip payload summary");
    match (row.input_tokens, row.output_tokens, row.total_tokens) {
        (Some(input), Some(output), Some(total)) => {
            assert_eq!(input, 23);
            assert_eq!(output, 7);
            assert_eq!(total, 30);
            assert!(payload["usageMissingReason"].is_null());
        }
        (None, None, None) => {
            assert!(
                payload["usageMissingReason"].as_str().is_some(),
                "bounded preview parsing may degrade metadata, but it should still record a reason"
            );
        }
        other => panic!("unexpected partial usage extraction state: {other:?}"),
    }
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_large_stream_keeps_preview_bounded_without_raw_reread() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-stream-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=large-stream"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let body_text = String::from_utf8(body.to_vec()).expect("stream body should be utf8");
    assert!(body_text.contains("response.completed"));

    let row = wait_for_large_stream_row(&state.pool).await;
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(42));
    assert_eq!(row.output_tokens, Some(13));
    assert_eq!(row.total_tokens, Some(55));
    assert_eq!(row.raw_response.len(), RAW_RESPONSE_PREVIEW_LIMIT);
    assert!(
        !row.raw_response.contains("response.completed"),
        "preview should not contain the terminal event once the delta exceeds the preview budget"
    );
    assert!(
        row.response_raw_size
            .is_some_and(|size| size > RAW_RESPONSE_PREVIEW_LIMIT as i64)
    );

    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read persisted large response raw");
    let raw_text = String::from_utf8(raw_bytes).expect("raw response should remain utf8");
    assert!(raw_text.contains("response.completed"));
    assert!(raw_text.len() > row.raw_response.len());

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode persisted payload summary");
    assert!(payload["usageMissingReason"].is_null());
    assert_eq!(payload["serviceTier"].as_str(), Some("priority"));
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

use super::*;
