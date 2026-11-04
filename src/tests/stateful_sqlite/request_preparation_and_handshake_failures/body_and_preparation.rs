use super::*;
use serde_json::json;

#[test]
fn prepare_target_request_body_injects_include_usage_for_chat_stream() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-4o-mini",
        "stream": true,
        "messages": [{"role":"user","content":"hi"}]
    }))
    .expect("serialize request body");
    let (rewritten, info, did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::ChatCompletions, body, true);
    assert!(did_rewrite);
    assert!(info.is_stream);
    assert_eq!(info.model.as_deref(), Some("gpt-4o-mini"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
    assert_eq!(
        payload
            .pointer("/stream_options/include_usage")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn prepare_target_request_body_extracts_prompt_cache_key_from_metadata() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "metadata": {
            "prompt_cache_key": "pck-from-body"
        }
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert_eq!(info.prompt_cache_key.as_deref(), Some("pck-from-body"));
}

#[test]
fn prepare_target_request_body_extracts_sticky_key_aliases_from_metadata() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "metadata": {
            "sticky_key": "sticky-from-body"
        }
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert_eq!(info.sticky_key.as_deref(), Some("sticky-from-body"));
    assert_eq!(info.prompt_cache_key, None);
}

#[test]
fn prepare_target_request_body_extracts_requested_service_tier_without_rewriting_when_disabled() {
    let expected = json!({
        "model": "gpt-5.3-codex",
        "serviceTier": " Priority ",
        "stream": false
    });
    let body = serde_json::to_vec(&expected).expect("serialize request body");

    let (rewritten, info, did_rewrite) =
        prepare_target_request_body(ProxyCaptureTarget::Responses, body, true);

    assert!(!did_rewrite);
    assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode body");
    assert_eq!(payload, expected);
}

#[test]
fn rewrite_request_service_tier_for_fast_mode_fill_missing_injects_priority_for_responses() {
    let mut payload = json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "input": "hi"
    });

    let did_rewrite = rewrite_request_service_tier_for_fast_mode(
        &mut payload,
        TagFastModeRewriteMode::FillMissing,
    );

    assert!(did_rewrite);
    assert_eq!(payload["service_tier"].as_str(), Some("priority"));
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn rewrite_request_service_tier_for_fast_mode_fill_missing_preserves_existing_alias() {
    let mut payload = json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "serviceTier": "flex"
    });

    let did_rewrite = rewrite_request_service_tier_for_fast_mode(
        &mut payload,
        TagFastModeRewriteMode::FillMissing,
    );

    assert!(!did_rewrite);
    assert_eq!(payload["serviceTier"].as_str(), Some("flex"));
    assert!(payload.get("service_tier").is_none());
}

#[test]
fn rewrite_request_service_tier_for_fast_mode_force_add_overrides_existing_tier() {
    let mut payload = json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "serviceTier": "flex",
        "messages": [{"role": "user", "content": "hi"}]
    });

    let did_rewrite =
        rewrite_request_service_tier_for_fast_mode(&mut payload, TagFastModeRewriteMode::ForceAdd);

    assert!(did_rewrite);
    assert_eq!(payload["service_tier"], "priority");
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn rewrite_request_service_tier_for_fast_mode_force_remove_deletes_both_aliases() {
    let mut payload = json!({
        "model": "gpt-5.3-codex",
        "service_tier": "priority",
        "serviceTier": "flex"
    });

    let did_rewrite = rewrite_request_service_tier_for_fast_mode(
        &mut payload,
        TagFastModeRewriteMode::ForceRemove,
    );

    assert!(did_rewrite);
    assert!(payload.get("service_tier").is_none());
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn pool_request_snapshot_preserves_content_length_only_for_file_backed_replays() {
    assert!(!pool_request_snapshot_preserves_content_length(
        &PoolReplayBodySnapshot::Empty
    ));
    assert!(!pool_request_snapshot_preserves_content_length(
        &PoolReplayBodySnapshot::Memory(Bytes::from_static(b"{}"))
    ));
    assert!(pool_request_snapshot_preserves_content_length(
        &PoolReplayBodySnapshot::File {
            temp_file: Arc::new(PoolReplayTempFile {
                path: PathBuf::from("/tmp/cvm-pool-replay-test.bin"),
            }),
            size: 2,
        }
    ));
}

#[tokio::test]
async fn prepare_pool_request_body_for_account_skips_fast_mode_rewrite_for_compact() {
    let expected = json!({
        "model": "gpt-5.1-codex-max",
        "serviceTier": "flex",
        "previous_response_id": "resp_prev_001",
        "input": [{
            "role": "user",
            "content": "compact this thread"
        }]
    });
    let body = Bytes::from(serde_json::to_vec(&expected).expect("serialize compact request body"));

    let prepared = prepare_pool_request_body_for_account(
        450450,
        Some(&PoolReplayBodySnapshot::Memory(body)),
        &"/v1/responses/compact".parse().expect("valid compact uri"),
        &Method::POST,
        None,
        TagFastModeRewriteMode::ForceAdd,
        crate::ImageToolRewriteMode::KeepOriginal,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("prepare compact pool request body");

    assert_eq!(prepared.requested_service_tier.as_deref(), Some("flex"));
    let request_body = prepared
        .request_body_for_capture
        .expect("capture request body should be materialized");
    let payload: Value = serde_json::from_slice(&request_body).expect("decode body");
    assert_eq!(payload, expected);
    assert!(payload.get("service_tier").is_none());
}

#[tokio::test]
async fn prepare_pool_request_body_for_account_preserves_file_snapshot_when_rewrite_is_noop() {
    let expected = json!({
        "model": "gpt-5.1-codex-max",
        "serviceTier": "flex",
        "input": "summarize this thread",
        "padding": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 64),
    });
    let body = Bytes::from(serde_json::to_vec(&expected).expect("serialize compact request body"));
    let snapshot = pool_replay_snapshot_from_bytes(450451, body.clone())
        .await
        .expect("build replay snapshot");
    assert_eq!(pool_request_snapshot_kind(&snapshot), "file");

    let prepared = prepare_pool_request_body_for_account(
        450451,
        Some(&snapshot),
        &"/v1/responses".parse().expect("valid responses uri"),
        &Method::POST,
        None,
        TagFastModeRewriteMode::KeepOriginal,
        crate::ImageToolRewriteMode::ForceRemove,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("prepare compact pool request body");

    assert_eq!(pool_request_snapshot_kind(&prepared.snapshot), "file");
    assert_eq!(prepared.requested_service_tier.as_deref(), Some("flex"));
    assert_eq!(
        prepared
            .request_body_for_capture
            .expect("capture request body should be materialized"),
        body
    );
}

#[tokio::test]
async fn prepare_pool_request_body_for_account_reports_rewritten_image_intent_after_force_remove() {
    let body = Bytes::from(
        serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "input": "draw a cat",
            "tools": [
                {
                    "type": "image_generation",
                    "output_format": "png"
                }
            ],
            "tool_choice": {
                "type": "image_generation"
            }
        }))
        .expect("serialize responses request body"),
    );

    let prepared = prepare_pool_request_body_for_account(
        488488,
        Some(&PoolReplayBodySnapshot::Memory(body)),
        &"/v1/responses".parse().expect("valid responses uri"),
        &Method::POST,
        None,
        TagFastModeRewriteMode::KeepOriginal,
        crate::ImageToolRewriteMode::ForceRemove,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("prepare responses pool request body");

    assert_eq!(prepared.requested_image_intent, crate::ImageIntent::No);
    let request_body = prepared
        .request_body_for_capture
        .expect("capture request body should be materialized");
    let payload: Value = serde_json::from_slice(&request_body).expect("decode body");
    assert!(
        !payload["tools"]
            .as_array()
            .expect("tools should remain an array")
            .iter()
            .any(|tool| tool["type"].as_str() == Some("image_generation"))
    );
    assert!(payload.get("tool_choice").is_none());
}

#[tokio::test]
async fn prepare_pool_request_body_for_account_keeps_large_rewrite_file_backed() {
    let body = Bytes::from(
        serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "input": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 64),
            "tools": [
                {
                    "type": "image_generation",
                    "output_format": "png"
                }
            ],
            "tool_choice": {
                "type": "image_generation"
            }
        }))
        .expect("serialize large responses request body"),
    );

    let prepared = prepare_pool_request_body_for_account(
        488489,
        Some(&PoolReplayBodySnapshot::Memory(body)),
        &"/v1/responses".parse().expect("valid responses uri"),
        &Method::POST,
        None,
        TagFastModeRewriteMode::KeepOriginal,
        crate::ImageToolRewriteMode::ForceRemove,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("prepare responses pool request body");

    assert_eq!(pool_request_snapshot_kind(&prepared.snapshot), "file");
    assert_eq!(prepared.requested_image_intent, crate::ImageIntent::No);
    let request_body = prepared
        .request_body_for_capture
        .expect("capture request body should be materialized");
    let payload: Value = serde_json::from_slice(&request_body).expect("decode body");
    assert!(
        !payload["tools"]
            .as_array()
            .expect("tools should remain an array")
            .iter()
            .any(|tool| tool["type"].as_str() == Some("image_generation"))
    );
    assert!(payload.get("tool_choice").is_none());
}

#[tokio::test]
async fn prepare_pool_request_body_for_account_keeps_responses_lite_body_when_codex_policy_keeps_original()
 {
    let expected = json!({
        "model": "gpt-5.6-codex",
        "input": {
            "additional_tools": [{
                "type": "image_gen.imagegen",
                "output_format": "png"
            }]
        },
        "tools": [{"type": "function", "name": "existing_tool"}],
        "tool_choice": {"type": "function", "name": "existing_tool"}
    });
    let body = Bytes::from(serde_json::to_vec(&expected).expect("serialize Lite request body"));

    let prepared = prepare_pool_request_body_for_account(
        488490,
        Some(&PoolReplayBodySnapshot::Memory(body.clone())),
        &"/v1/responses".parse().expect("valid responses uri"),
        &Method::POST,
        None,
        TagFastModeRewriteMode::KeepOriginal,
        crate::ImageToolRewriteMode::KeepOriginal,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        Some(CodexImagegenProtocol::Lite),
        None,
        None,
        None,
    )
    .await
    .expect("prepare Lite pool request body");

    assert!(!prepared.snapshot_is_decoded);
    assert_eq!(
        prepared
            .request_body_for_capture
            .expect("capture request body should be preserved"),
        body
    );
}

#[tokio::test]
async fn prepare_pool_request_body_for_account_keeps_compressed_and_file_backed_lite_requests() {
    let expected = json!({
        "model": "gpt-5.6-codex",
        "input": {"additional_tools": [{"type": "image_gen.imagegen"}]},
        "padding": "x".repeat(POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES + 64),
    });
    let plain = Bytes::from(serde_json::to_vec(&expected).expect("serialize Lite request body"));
    let snapshot = pool_replay_snapshot_from_bytes(488491, plain.clone())
        .await
        .expect("build replay snapshot");
    assert_eq!(pool_request_snapshot_kind(&snapshot), "file");

    let file_prepared = prepare_pool_request_body_for_account(
        488491,
        Some(&snapshot),
        &"/v1/responses".parse().expect("valid responses uri"),
        &Method::POST,
        None,
        TagFastModeRewriteMode::KeepOriginal,
        crate::ImageToolRewriteMode::ForceRemove,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        Some(CodexImagegenProtocol::Lite),
        None,
        None,
        None,
    )
    .await
    .expect("prepare file-backed Lite request body");
    assert_eq!(pool_request_snapshot_kind(&file_prepared.snapshot), "file");
    assert_eq!(
        file_prepared
            .snapshot
            .to_bytes()
            .await
            .expect("file-backed Lite snapshot should be readable"),
        plain
    );

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&plain)
        .expect("write compressed Lite request body");
    let compressed = Bytes::from(
        encoder
            .finish()
            .expect("finish compressed Lite request body"),
    );
    let gzip_prepared = prepare_pool_request_body_for_account(
        488492,
        Some(&PoolReplayBodySnapshot::Memory(compressed.clone())),
        &"/v1/responses".parse().expect("valid responses uri"),
        &Method::POST,
        Some("gzip"),
        TagFastModeRewriteMode::KeepOriginal,
        crate::ImageToolRewriteMode::ForceAdd,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        Some(CodexImagegenProtocol::Lite),
        None,
        None,
        None,
    )
    .await
    .expect("prepare compressed Lite request body");
    assert!(!gzip_prepared.snapshot_is_decoded);
    assert_eq!(
        gzip_prepared
            .request_body_for_capture
            .expect("compressed Lite request should be preserved"),
        compressed
    );
}

#[tokio::test]
async fn prepare_pool_request_body_for_account_decodes_gzip_before_rewrite() {
    let plain = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "serviceTier": "flex",
        "input": [{
            "role": "user",
            "content": "rewrite compressed request"
        }]
    }))
    .expect("serialize compressed request body");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&plain)
        .expect("write compressed request body");
    let compressed = encoder.finish().expect("finish compressed request body");

    let prepared = prepare_pool_request_body_for_account(
        490001,
        Some(&PoolReplayBodySnapshot::Memory(Bytes::from(compressed))),
        &"/v1/responses".parse().expect("valid responses uri"),
        &Method::POST,
        Some("gzip"),
        TagFastModeRewriteMode::ForceAdd,
        crate::ImageToolRewriteMode::KeepOriginal,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("prepare gzip-compressed pool request body");

    assert!(prepared.snapshot_is_decoded);
    assert_eq!(
        prepared.oauth_responses_source_encoding,
        Some(RequestBodyContentEncoding::Gzip)
    );
    assert_eq!(prepared.requested_service_tier.as_deref(), Some("priority"));
    let request_body = prepared
        .request_body_for_capture
        .expect("capture request body should be materialized");
    let payload: Value = serde_json::from_slice(&request_body).expect("decode rewritten body");
    assert_eq!(payload["service_tier"].as_str(), Some("priority"));
    assert!(payload.get("serviceTier").is_none());
    let decoded_snapshot = prepared
        .snapshot
        .to_bytes()
        .await
        .expect("read decoded rewrite snapshot");
    let snapshot_payload: Value =
        serde_json::from_slice(&decoded_snapshot).expect("decode rewritten snapshot");
    assert_eq!(snapshot_payload["service_tier"].as_str(), Some("priority"));
    assert!(snapshot_payload.get("serviceTier").is_none());
}

#[tokio::test]
async fn build_pool_upstream_request_body_follow_passthrough_preserves_gzip_snapshot() {
    let plain = Bytes::from_static(br#"{"model":"gpt-5","input":"hello"}"#);
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&plain)
        .expect("write follow passthrough gzip payload");
    let compressed = encoder
        .finish()
        .expect("finish follow passthrough gzip payload");
    let prepared = PreparedPoolRequestBody {
        snapshot: PoolReplayBodySnapshot::Memory(Bytes::from(compressed.clone())),
        request_body_for_capture: None,
        requested_service_tier: None,
        requested_image_intent: ImageIntent::Unknown,
        requested_hosted_image_intent: ImageIntent::Unknown,
        codex_imagegen_rewrite: None,
        oauth_responses_rewrite: None,
        oauth_responses_source_encoding: None,
        oauth_responses_wants_stream: None,
        snapshot_is_decoded: false,
    };

    let outbound = build_pool_upstream_request_body(
        &prepared,
        RequestCompressionAlgorithm::Follow,
        RequestCompressionLevelPreset::Balanced,
        Some("gzip"),
    )
    .await
    .expect("build passthrough gzip request body");

    assert_eq!(outbound.content_length, Some(compressed.len()));
    assert_eq!(outbound.content_encoding, RequestBodyContentEncoding::Gzip);
    assert_eq!(
        outbound.compression_mode,
        PoolRequestBodyCompressionMode::Passthrough
    );
}

#[tokio::test]
async fn build_pool_upstream_request_body_follow_rejects_unsupported_request_encoding() {
    let prepared = PreparedPoolRequestBody {
        snapshot: PoolReplayBodySnapshot::Memory(Bytes::from_static(
            br#"{"model":"gpt-5","input":"hello"}"#,
        )),
        request_body_for_capture: None,
        requested_service_tier: None,
        requested_image_intent: ImageIntent::Unknown,
        requested_hosted_image_intent: ImageIntent::Unknown,
        codex_imagegen_rewrite: None,
        oauth_responses_rewrite: None,
        oauth_responses_source_encoding: None,
        oauth_responses_wants_stream: None,
        snapshot_is_decoded: false,
    };

    let err = build_pool_upstream_request_body(
        &prepared,
        RequestCompressionAlgorithm::Follow,
        RequestCompressionLevelPreset::Balanced,
        Some("br"),
    )
    .await
    .expect_err("follow should reject unsupported request encoding");

    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert!(
        err.message
            .contains("unsupported request Content-Encoding: br"),
        "unexpected error: {}",
        err.message
    );
}

#[tokio::test]
async fn query_pool_attempt_records_from_live_limits_ttft_to_the_final_attempt() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let invoke_id = "pool-request-compression-query";
    let occurred_at = "2026-07-15 12:00:00";
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
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses"}"#)
    .bind(r#"{"ok":true}"#)
    .bind("identity")
    .bind(840.0)
    .execute(&pool)
    .await
    .expect("insert invocation row");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: None,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: None,
    };
    insert_pool_upstream_request_attempt_with_scope(
        &pool,
        &trace,
        None,
        None,
        None,
        None,
        None,
        None,
        1,
        1,
        0,
        Some(occurred_at),
        Some(occurred_at),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED),
        Some(StatusCode::OK),
        None,
        None,
        None,
        None,
        Some(10.0),
        Some(20.0),
        Some(30.0),
        Some("req_predecessor"),
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
    .expect("insert predecessor pool attempt row");

    insert_pool_upstream_request_attempt_with_scope(
        &pool,
        &trace,
        None,
        None,
        None,
        None,
        None,
        Some("https://api.openai.com"),
        2,
        1,
        0,
        Some(occurred_at),
        Some(occurred_at),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED),
        Some(StatusCode::OK),
        None,
        None,
        None,
        None,
        Some(12.0),
        Some(24.0),
        Some(36.0),
        Some("req_compression_obs"),
        Some("gzip"),
        Some("recompressed"),
        Some(128),
        Some(64),
        Some(24),
        Some(512),
        Some(48),
        None,
        None,
    )
    .await
    .expect("insert pool attempt row");

    insert_pool_upstream_request_attempt_with_scope(
        &pool,
        &trace,
        None,
        None,
        None,
        None,
        None,
        None,
        3,
        1,
        0,
        Some(occurred_at),
        Some(occurred_at),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED),
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
        None,
        None,
        None,
        None,
    )
    .await
    .expect("insert budget pseudo-terminal pool attempt row");

    let rows = query_pool_attempt_records_from_live(&pool, invoke_id)
        .await
        .expect("query pool attempt records");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].first_token_ms, None);
    assert_eq!(rows[1].first_token_ms, Some(840.0));
    assert_eq!(rows[2].first_token_ms, None);
    assert_eq!(
        rows[1].downstream_request_content_encoding.as_deref(),
        Some("identity")
    );
    assert_eq!(
        rows[1].upstream_request_compression_algorithm.as_deref(),
        Some("gzip")
    );
    assert_eq!(
        rows[1].upstream_request_compression_mode.as_deref(),
        Some("recompressed")
    );
    assert_eq!(rows[1].logical_body_bytes, Some(128));
    assert_eq!(rows[1].transmitted_body_bytes, Some(64));
    assert_eq!(rows[1].saved_bytes, Some(64));
    assert_eq!(rows[1].ratio_pct, Some(-50.0));
    assert_eq!(rows[1].approx_upload_bytes, Some(88));
    assert_eq!(rows[1].approx_download_bytes, Some(560));
}

#[tokio::test]
async fn query_pool_attempt_records_from_live_keeps_pending_streaming_ttft() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let invoke_id = "pool-pending-streaming-query";
    let occurred_at = "2026-07-15 12:05:00";
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
    .execute(&pool)
    .await
    .expect("insert running invocation row");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: None,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: None,
    };
    insert_pool_upstream_request_attempt_with_scope(
        &pool,
        &trace,
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

    let rows = query_pool_attempt_records_from_live(&pool, invoke_id)
        .await
        .expect("query pool attempt records");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].first_token_ms, Some(720.0));
    assert_eq!(rows[0].stream_latency_ms, None);

    insert_pool_upstream_request_attempt_with_scope(
        &pool,
        &trace,
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

    let retry_rows = query_pool_attempt_records_from_live(&pool, invoke_id)
        .await
        .expect("query retry pool attempt records");
    assert_eq!(retry_rows.len(), 2);
    assert_eq!(retry_rows[0].first_token_ms, None);
    assert_eq!(retry_rows[1].first_token_ms, Some(720.0));
}

#[test]
fn prepare_target_request_body_extracts_reasoning_effort_for_responses() {
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
fn prepare_target_request_body_detects_remote_v2_compaction_requests() {
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
fn prepare_target_request_body_detects_remote_v2_compaction_request_object_shape() {
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
fn prepare_target_request_body_extracts_reasoning_effort_for_chat_completions() {
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
fn extract_requested_service_tier_from_request_body_reads_top_level_aliases() {
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
fn extract_requested_service_tier_from_request_body_ignores_nested_or_non_string_values() {
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
fn extract_requester_ip_uses_expected_header_priority() {
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
fn extract_prompt_cache_key_from_headers_reads_whitelist_keys() {
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
fn extract_sticky_key_from_headers_accepts_sticky_and_prompt_cache_aliases() {
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
