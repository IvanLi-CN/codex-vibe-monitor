use super::*;
use serde_json::json;

#[tokio::test]
async fn pool_early_phase_orphan_cleanup_guard_disarm_keeps_invocation_running_without_persisted_attempt_row()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-disarm-without-attempt-row";
    let occurred_at = "2026-03-23 21:09:06";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.24"),
        Some("sticky-guard-disarm"),
        Some("pck-guard-disarm"),
        true,
        Some(account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        10.0,
        2.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let pending = PendingPoolAttemptRecord {
        attempt_id: None,
        attempt_public_id: None,
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-disarm".to_string()),
        routing_source: None,
        routing_selection_audit_json: None,
        sticky_affinity_generation: None,
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
        upstream_request_model: None,
        model_mapping_pattern: None,
        request_summary_json: None,
        group_name_snapshot: None,
        proxy_binding_key_snapshot: None,
        upstream_account_id: account_id,
        upstream_route_key: "route-primary".to_string(),
        attempt_index: 1,
        distinct_account_index: 1,
        same_account_retry_index: 1,
        started_at: occurred_at.to_string(),
        connect_latency_ms: 0.0,
        first_byte_latency_ms: 0.0,
        upstream_request_compression_algorithm: None,
        upstream_request_compression_mode: None,
        upstream_request_logical_body_bytes: None,
        upstream_request_transmitted_body_bytes: None,
        upstream_request_header_bytes_approx: None,
        upstream_response_body_bytes: None,
        upstream_response_header_bytes_approx: None,
        compact_support_status: None,
        compact_support_reason: None,
        response_raw_path: None,
        response_raw_codec: Some("identity".to_string()),
        response_raw_size: None,
        response_raw_truncated: false,
        response_raw_truncated_reason: None,
        response_content_encoding: None,
    };

    {
        let mut guard = PoolEarlyPhaseOrphanCleanupGuard::new(state.clone(), pending);
        guard.disarm();
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    let invocation_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count invocation rows after disarmed guard drop");

    assert_eq!(
        invocation_count, 0,
        "disarmed memory-only running guard should not create a DB invocation row"
    );
}

#[tokio::test]
async fn finalize_deferred_pool_early_phase_cleanup_guard_after_terminal_invocation_marks_terminal_even_without_final_attempt_persist()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let pending = PendingPoolAttemptRecord {
        attempt_id: Some(43),
        attempt_public_id: None,
        invoke_id: "guard-complete-terminal-after-invocation".to_string(),
        occurred_at: "2026-03-23 21:10:10".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-complete-after-invocation".to_string()),
        routing_source: None,
        routing_selection_audit_json: None,
        sticky_affinity_generation: None,
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
        upstream_request_model: None,
        model_mapping_pattern: None,
        request_summary_json: None,
        group_name_snapshot: None,
        proxy_binding_key_snapshot: None,
        upstream_account_id: 18,
        upstream_route_key: "route-primary".to_string(),
        attempt_index: 1,
        distinct_account_index: 1,
        same_account_retry_index: 1,
        started_at: "2026-03-23 21:10:10".to_string(),
        connect_latency_ms: 5.0,
        first_byte_latency_ms: 12.0,
        upstream_request_compression_algorithm: None,
        upstream_request_compression_mode: None,
        upstream_request_logical_body_bytes: None,
        upstream_request_transmitted_body_bytes: None,
        upstream_request_header_bytes_approx: None,
        upstream_response_body_bytes: None,
        upstream_response_header_bytes_approx: None,
        compact_support_status: None,
        compact_support_reason: None,
        response_raw_path: None,
        response_raw_codec: Some("identity".to_string()),
        response_raw_size: None,
        response_raw_truncated: false,
        response_raw_truncated_reason: None,
        response_content_encoding: None,
    };

    let mut guard = Some(PoolEarlyPhaseOrphanCleanupGuard::new(state, pending));
    guard
        .as_mut()
        .expect("guard should exist")
        .mark_first_byte_observed(12.0);

    finalize_deferred_pool_early_phase_cleanup_guard_after_terminal_invocation(&mut guard, true);

    let guard = guard.expect("guard should still be present");
    assert!(guard.first_byte_observed);
    assert!(guard.terminal_outcome_observed);
    assert!(!guard.armed);
}

#[tokio::test]
async fn complete_deferred_pool_early_phase_cleanup_guard_marks_terminal_and_disarms() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "guard-complete-terminal";
    let occurred_at = "2026-03-23 21:10:09";
    let running_record = build_admitted_proxy_capture_runtime_snapshot(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        None,
        None,
        Some("pck-guard-complete-terminal"),
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(state.as_ref(), running_record)
        .await
        .expect("store ordinary invocation runtime snapshot");
    let pending = PendingPoolAttemptRecord {
        attempt_id: Some(42),
        attempt_public_id: None,
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-complete".to_string()),
        routing_source: None,
        routing_selection_audit_json: None,
        sticky_affinity_generation: None,
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
        upstream_request_model: None,
        model_mapping_pattern: None,
        request_summary_json: None,
        group_name_snapshot: None,
        proxy_binding_key_snapshot: None,
        upstream_account_id: 17,
        upstream_route_key: "route-primary".to_string(),
        attempt_index: 1,
        distinct_account_index: 1,
        same_account_retry_index: 1,
        started_at: "2026-03-23 21:10:09".to_string(),
        connect_latency_ms: 5.0,
        first_byte_latency_ms: 12.0,
        upstream_request_compression_algorithm: None,
        upstream_request_compression_mode: None,
        upstream_request_logical_body_bytes: None,
        upstream_request_transmitted_body_bytes: None,
        upstream_request_header_bytes_approx: None,
        upstream_response_body_bytes: None,
        upstream_response_header_bytes_approx: None,
        compact_support_status: None,
        compact_support_reason: None,
        response_raw_path: None,
        response_raw_codec: Some("identity".to_string()),
        response_raw_size: None,
        response_raw_truncated: false,
        response_raw_truncated_reason: None,
        response_content_encoding: None,
    };

    let mut guard = Some(PoolEarlyPhaseOrphanCleanupGuard::new(
        state.clone(),
        pending,
    ));
    guard
        .as_mut()
        .expect("guard should exist")
        .mark_first_byte_observed(12.0);

    complete_deferred_pool_early_phase_cleanup_guard(&mut guard);

    let guard = guard.expect("guard should still be present");
    assert!(guard.first_byte_observed);
    assert!(guard.terminal_outcome_observed);
    assert!(!guard.armed);
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .any(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at),
        "ordinary invocation runtime snapshots must remain owned by terminal persistence"
    );
}

#[tokio::test]
async fn attempt_completion_preserves_synthetic_runtime_until_request_cleanup() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "ABCDEFABCD";
    let occurred_at = "2026-03-23 21:10:11";
    let running_record = build_admitted_proxy_capture_runtime_snapshot(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        None,
        None,
        None,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(state.as_ref(), running_record)
        .await
        .expect("store synthetic pool runtime snapshot");
    let ordinary_invoke_id = "EFGHJKEFGH";
    let ordinary_record = build_admitted_proxy_capture_runtime_snapshot(
        ordinary_invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        None,
        None,
        Some("pck-ordinary-4201"),
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(state.as_ref(), ordinary_record)
        .await
        .expect("store ordinary runtime snapshot");
    let pending = PendingPoolAttemptRecord {
        attempt_id: Some(44),
        attempt_public_id: None,
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: None,
        routing_source: None,
        routing_selection_audit_json: None,
        sticky_affinity_generation: None,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: None,
        upstream_request_model: None,
        model_mapping_pattern: None,
        request_summary_json: None,
        group_name_snapshot: None,
        proxy_binding_key_snapshot: None,
        upstream_account_id: 19,
        upstream_route_key: "route-primary".to_string(),
        attempt_index: 1,
        distinct_account_index: 1,
        same_account_retry_index: 1,
        started_at: occurred_at.to_string(),
        connect_latency_ms: 5.0,
        first_byte_latency_ms: 12.0,
        upstream_request_compression_algorithm: None,
        upstream_request_compression_mode: None,
        upstream_request_logical_body_bytes: None,
        upstream_request_transmitted_body_bytes: None,
        upstream_request_header_bytes_approx: None,
        upstream_response_body_bytes: None,
        upstream_response_header_bytes_approx: None,
        compact_support_status: None,
        compact_support_reason: None,
        response_raw_path: None,
        response_raw_codec: Some("identity".to_string()),
        response_raw_size: None,
        response_raw_truncated: false,
        response_raw_truncated_reason: None,
        response_content_encoding: None,
    };
    let mut attempt_guard = Some(PoolEarlyPhaseOrphanCleanupGuard::new(
        state.clone(),
        pending,
    ));
    let request_guard = PoolViaRuntimeSnapshotCleanupGuard::new(state.clone(), invoke_id);

    complete_deferred_pool_early_phase_cleanup_guard(&mut attempt_guard);

    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .any(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at),
        "one completed upstream attempt must not remove request-scoped runtime visibility"
    );

    drop(request_guard);

    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .all(|record| record.invoke_id != invoke_id || record.occurred_at != occurred_at),
        "completed synthetic pool attempts must release their runtime snapshot"
    );
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .any(|record| record.invoke_id == ordinary_invoke_id),
        "request-scoped synthetic cleanup must preserve ordinary invocation runtime"
    );
}

#[tokio::test]
async fn send_pool_request_with_failover_defers_armed_guard_when_pending_attempt_row_was_not_persisted()
 {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-6767-1775776407999";
    let occurred_at = "2026-03-23 21:10:09";
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-deferred".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let runtime_snapshot = PoolAttemptRuntimeSnapshotContext {
        capture_target: ProxyCaptureTarget::Responses,
        request_info: RequestCaptureInfo {
            model: Some("gpt-5.4".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        },
        hosted_image_intent: None,
        prompt_cache_key: Some("pck-guard-deferred".to_string()),
        owner_auto_guard_active: false,
        t_req_read_ms: 1.0,
        t_req_parse_ms: 1.0,
    };

    sqlx::query(
        r#"
        CREATE TRIGGER fail_pending_pool_attempt_insert
        BEFORE INSERT ON pool_upstream_request_attempts
        WHEN NEW.invoke_id = 'proxy-6767-1775776407999'
          AND NEW.status = 'pending'
        BEGIN
            SELECT RAISE(FAIL, 'simulated pending attempt insert failure');
        END;
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("create failing pending attempt insert trigger");

    let account = PoolResolvedAccount {
        account_id,
        display_name: "Primary".to_string(),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer primary".to_string(),
        },
        upstream_base_url: Url::parse(&upstream_base).expect("valid upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        routing_control_fingerprint: None,
        sticky_affinity_generation: None,
        routing_selection_audit: None,
        priority_handoff_permit: None,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        single_account_rotation_enabled: false,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        oauth_request_compression_algorithm: RequestCompressionAlgorithm::Zstd,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
    };

    let mut upstream = send_pool_request_with_failover(
        state.clone(),
        6767,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::Memory(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "hello"}]
            }))
            .expect("serialize request body"),
        ))),
        Duration::from_secs(5),
        Some(trace),
        Some(runtime_snapshot),
        Some("sticky-guard-deferred"),
        Some(account),
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect("request should still succeed without a persisted pending attempt row");

    assert_eq!(upstream.response.status(), StatusCode::OK);
    assert!(
        upstream
            .pending_attempt_record
            .as_ref()
            .is_some_and(|pending| pending.attempt_id.is_none())
    );
    assert!(
        upstream
            .deferred_early_phase_cleanup_guard
            .as_ref()
            .is_some_and(|guard| guard.first_byte_observed)
    );
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let invocation_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count invocation after successful request without persisted attempt row");
    assert_eq!(
        invocation_count, 0,
        "running runtime state should not create a codex_invocations row"
    );

    disarm_pool_early_phase_cleanup_guard(&mut upstream.deferred_early_phase_cleanup_guard);
    upstream_handle.abort();
}

#[tokio::test]
async fn send_pool_request_with_failover_keeps_runtime_streaming_phase_without_persisted_attempt_row()
 {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-6768-1775776408000-persisted";
    let occurred_at = "2026-03-23 21:10:10";
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-streaming-phase-persisted".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let runtime_snapshot = PoolAttemptRuntimeSnapshotContext {
        capture_target: ProxyCaptureTarget::Responses,
        request_info: RequestCaptureInfo {
            model: Some("gpt-5.4".to_string()),
            is_stream: true,
            ..RequestCaptureInfo::default()
        },
        hosted_image_intent: None,
        prompt_cache_key: Some("pck-guard-streaming-phase-persisted".to_string()),
        owner_auto_guard_active: false,
        t_req_read_ms: 1.0,
        t_req_parse_ms: 1.0,
    };

    let account = PoolResolvedAccount {
        account_id,
        display_name: "Primary".to_string(),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer primary".to_string(),
        },
        upstream_base_url: Url::parse(&upstream_base).expect("valid upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        routing_control_fingerprint: None,
        sticky_affinity_generation: None,
        routing_selection_audit: None,
        priority_handoff_permit: None,
        group_name: Some(test_required_group_name().to_string()),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        forward_proxy_scope: ForwardProxyRouteScope::from_group_binding(
            Some(test_required_group_name()),
            test_required_group_bound_proxy_keys(),
        ),
        single_account_rotation_enabled: false,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        oauth_request_compression_algorithm: RequestCompressionAlgorithm::Zstd,
        response_endpoint_capability: CapabilitySupport::Unknown,
        chat_completions_capability: CapabilitySupport::Unknown,
        image_endpoint_capability: CapabilitySupport::Unknown,
        response_image_tool_capability: CapabilitySupport::Unknown,
        codex_imagegen_capability: CapabilitySupport::Unknown,
        standalone_search_capability: CapabilitySupport::Unknown,
    };

    let upstream = send_pool_request_with_failover(
        state.clone(),
        6768,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::Memory(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "hello"}]
            }))
            .expect("serialize request body"),
        ))),
        Duration::from_secs(5),
        Some(trace),
        Some(runtime_snapshot),
        Some("sticky-guard-streaming-phase-persisted"),
        Some(account),
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect("request should succeed when streaming-phase persistence succeeds");

    assert_eq!(upstream.response.status(), StatusCode::OK);
    assert!(
        upstream
            .pending_attempt_record
            .as_ref()
            .is_some_and(|pending| pending.attempt_id.is_none())
    );
    let attempt_public_id = upstream
        .pending_attempt_record
        .as_ref()
        .and_then(|pending| pending.attempt_public_id.clone())
        .expect("runtime attempt should have a stable public id");
    assert!(upstream.deferred_early_phase_cleanup_guard.is_some());

    let live_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id=?1 AND occurred_at=?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("inspect live attempt before cancellation");
    assert_eq!(live_count, 0, "running attempts must remain in memory");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let runtime_attempt = state
        .proxy_runtime_attempts
        .get(invoke_id, occurred_at, &attempt_public_id)
        .expect("streaming phase should remain in the runtime attempt store");
    assert_eq!(
        runtime_attempt.phase.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );

    let attempt_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count attempts after runtime streaming-phase update");
    assert_eq!(attempt_count, 0);
    let mut upstream = upstream;
    disarm_pool_early_phase_cleanup_guard(&mut upstream.deferred_early_phase_cleanup_guard);
    upstream_handle.abort();
}
