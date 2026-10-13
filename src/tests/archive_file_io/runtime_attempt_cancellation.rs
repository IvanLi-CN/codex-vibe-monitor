use super::*;

#[tokio::test]
async fn send_pool_request_with_failover_keeps_early_phase_guard_armed_when_runtime_phase_is_memory_only()
 {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-6768-1775776408000";
    let occurred_at = "2026-03-23 21:10:10";
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-streaming-phase".to_string()),
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
        prompt_cache_key: Some("pck-guard-streaming-phase".to_string()),
        owner_auto_guard_active: false,
        t_req_read_ms: 1.0,
        t_req_parse_ms: 1.0,
    };

    sqlx::query(
        r#"
        CREATE TRIGGER ignore_streaming_phase_update
        BEFORE UPDATE ON pool_upstream_request_attempts
        WHEN OLD.invoke_id = 'proxy-6768-1775776408000'
          AND NEW.phase = 'streaming_response'
        BEGIN
            SELECT RAISE(IGNORE);
        END;
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("create trigger that suppresses streaming-phase persistence");

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
        Some("sticky-guard-streaming-phase"),
        Some(account),
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect("request should still succeed when streaming-phase persistence is suppressed");

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
    let runtime_attempt = state
        .proxy_runtime_attempts
        .get(invoke_id, occurred_at, &attempt_public_id)
        .expect("streaming phase should remain in the runtime attempt store");
    assert_eq!(
        runtime_attempt.phase.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );
    assert!(upstream.deferred_early_phase_cleanup_guard.is_some());

    drop(upstream);
    let attempt_count = {
        let mut poll_count = 0;
        loop {
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
            .expect("load attempt after suppressed streaming-phase update");
            if attempt_count > 0 || poll_count >= 100 {
                break attempt_count;
            }
            poll_count += 1;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    assert_eq!(
        attempt_count, 1,
        "drop recovery must materialize exactly one terminal attempt"
    );
    let (status, finished_at, failure_kind): (String, Option<String>, Option<String>) =
        sqlx::query_as("SELECT status,finished_at,failure_kind FROM pool_upstream_request_attempts WHERE invoke_id=?1 AND occurred_at=?2")
            .bind(invoke_id).bind(occurred_at).fetch_one(&state.pool).await
            .expect("inspect recovered terminal attempt");
    assert_eq!(status, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_INTERRUPTED);
    assert!(finished_at.is_some());
    assert_eq!(
        failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );

    let invocation_count = {
        let mut poll_count = 0;
        loop {
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
            .expect("count invocation rows after suppressed streaming-phase update");
            if invocation_count == 1 || poll_count >= 100 {
                break invocation_count;
            }
            poll_count += 1;
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    assert_eq!(
        invocation_count, 1,
        "cancellation must persist exactly one interrupted invocation"
    );
    let status: String = sqlx::query_scalar(
        "SELECT status FROM codex_invocations WHERE invoke_id=?1 AND occurred_at=?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("inspect recovered invocation");
    assert_eq!(status, INVOCATION_STATUS_INTERRUPTED);

    upstream_handle.abort();
}
