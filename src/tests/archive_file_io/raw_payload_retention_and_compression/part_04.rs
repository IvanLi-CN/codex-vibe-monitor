use super::*;
use serde_json::json;

fn assert_runtime_snapshot_present(state: &AppState, invoke_id: &str, occurred_at: &str) {
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .any(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at),
        "runtime snapshot should remain visible"
    );
}

fn assert_runtime_snapshot_absent(state: &AppState, invoke_id: &str, occurred_at: &str) {
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .all(|record| record.invoke_id != invoke_id || record.occurred_at != occurred_at),
        "runtime snapshot should be removed"
    );
}

fn build_test_pool_resolved_account(account_id: i64, upstream_base: &str) -> PoolResolvedAccount {
    PoolResolvedAccount {
        account_id,
        display_name: "Primary".to_string(),
        kind: "api_key_codex".to_string(),
        auth: PoolResolvedAuth::ApiKey {
            authorization: "Bearer primary".to_string(),
        },
        upstream_base_url: Url::parse(upstream_base).expect("valid upstream base url"),
        routing_source: PoolRoutingSelectionSource::FreshAssignment,
        sticky_affinity_generation: None,
        priority_handoff_permit: None,
        routing_selection_audit: None,
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
    }
}

async fn wait_for_interrupted_attempt(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
) -> (String, Option<String>, Option<String>) {
    for poll_count in 0..=100 {
        let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
            "SELECT status, phase, failure_kind FROM pool_upstream_request_attempts
             WHERE invoke_id = ?1 AND occurred_at = ?2 ORDER BY id DESC LIMIT 1",
        )
        .bind(invoke_id)
        .bind(occurred_at)
        .fetch_one(pool)
        .await
        .expect("load attempt after suppressed streaming-phase update");
        if attempt.0 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE || poll_count == 100
        {
            return attempt;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    unreachable!("interrupted attempt polling returns at the timeout boundary")
}

async fn count_test_invocation_rows(pool: &SqlitePool, invoke_id: &str, occurred_at: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(pool)
    .await
    .expect("count invocation rows after suppressed streaming-phase update")
}

async fn assert_active_recovery_untouched(pool: &SqlitePool, invoke_id: &str, occurred_at: &str) {
    let attempt = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, phase FROM pool_upstream_request_attempts
         WHERE invoke_id = ?1 AND occurred_at = ?2 ORDER BY id DESC LIMIT 1",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(pool)
    .await
    .expect("load active attempt after stale recovery");
    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );
    assert_eq!(
        count_test_invocation_rows(pool, invoke_id, occurred_at).await,
        0
    );
}

async fn assert_failed_recovery_rolled_back(pool: &SqlitePool, invoke_id: &str, occurred_at: &str) {
    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT status, phase, failure_kind FROM pool_upstream_request_attempts
         WHERE invoke_id = ?1 ORDER BY id DESC LIMIT 1",
    )
    .bind(invoke_id)
    .fetch_one(pool)
    .await
    .expect("load attempt after failed stale sweeper");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, failure_kind FROM codex_invocations
         WHERE invoke_id = ?1 AND occurred_at = ?2 ORDER BY id DESC LIMIT 1",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(pool)
    .await
    .expect("load invocation after failed stale sweeper");
    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
}

async fn suppress_streaming_phase_update(pool: &SqlitePool) {
    sqlx::query(
        "CREATE TRIGGER ignore_streaming_phase_update
         BEFORE UPDATE ON pool_upstream_request_attempts
         WHEN OLD.invoke_id = 'proxy-6768-1775776408000'
           AND NEW.phase = 'streaming_response'
         BEGIN SELECT RAISE(IGNORE); END;",
    )
    .execute(pool)
    .await
    .expect("create trigger that suppresses streaming-phase persistence");
}

#[tokio::test]
pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan_clears_pool_routing_reservation_and_records_route_failure()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-4242-1775776407000";
    let occurred_at = "2026-03-23 21:10:08";
    seed_running_proxy_invocation(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard-reservation",
    )
    .await;

    let pending = begin_test_pool_attempt(
        state.as_ref(),
        account_id,
        invoke_id,
        occurred_at,
        occurred_at,
        "sticky-guard-reservation",
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await;
    let reservation_key = build_pool_routing_reservation_key(4242);
    reserve_test_pool_routing_account(&state, &reservation_key, account_id).await;

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending, false, false)
        .await
        .expect("recover dropped guard orphan and clean up route state");

    {
        let reservations = state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned");
        assert!(
            !reservations.contains_key(&reservation_key),
            "guard recovery should clear the in-memory reservation",
        );
    }

    let route_state = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
        r#"
        SELECT last_route_failure_kind, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load account route state after guard recovery");

    assert!(route_state.0.is_none());
    assert_eq!(route_state.2, 0);
    assert!(route_state.1.is_none());
}

#[tokio::test]
pub(crate) async fn pool_early_phase_orphan_cleanup_guard_disarm_keeps_invocation_running_without_persisted_attempt_row()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-disarm-without-attempt-row";
    let occurred_at = "2026-03-23 21:09:06";
    seed_running_proxy_invocation(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard-disarm",
    )
    .await;
    let pending =
        build_unpersisted_pool_attempt(account_id, invoke_id, occurred_at, "sticky-guard-disarm");

    {
        let mut guard = PoolEarlyPhaseOrphanCleanupGuard::new(state.clone(), pending);
        guard.disarm();
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_eq!(
        count_invocation_rows(&state.pool, invoke_id, occurred_at).await,
        0,
        "disarmed memory-only running guard should not create a DB invocation row"
    );
}

#[tokio::test]
pub(crate) async fn finalize_deferred_pool_early_phase_cleanup_guard_after_terminal_invocation_marks_terminal_even_without_final_attempt_persist()
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
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
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
pub(crate) async fn complete_deferred_pool_early_phase_cleanup_guard_marks_terminal_and_disarms() {
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
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
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
pub(crate) async fn attempt_completion_preserves_synthetic_runtime_until_request_cleanup() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "pool-via-4201";
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
    let ordinary_invoke_id = "ordinary-4201";
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
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: None,
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
    let request_guard = PoolViaRuntimeSnapshotCleanupGuard::new(state.clone(), 4201);

    complete_deferred_pool_early_phase_cleanup_guard(&mut attempt_guard);

    assert_runtime_snapshot_present(state.as_ref(), invoke_id, occurred_at);

    drop(request_guard);

    assert_runtime_snapshot_absent(state.as_ref(), invoke_id, occurred_at);
    assert_runtime_snapshot_present(state.as_ref(), ordinary_invoke_id, occurred_at);
}

#[tokio::test]
pub(crate) async fn send_pool_request_with_failover_defers_armed_guard_when_pending_attempt_row_was_not_persisted()
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

    let account = build_test_pool_resolved_account(account_id, &upstream_base);

    let mut upstream = send_pool_request_with_failover(PoolFailoverRequest {
        state: state.clone(),
        proxy_request_id: 6767,
        method: Method::POST,
        original_uri: &"/v1/responses".parse().expect("valid uri"),
        headers: &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        body: Some(PoolReplayBodySnapshot::Memory(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "hello"}]
            }))
            .expect("serialize request body"),
        ))),
        handshake_timeout: Duration::from_secs(5),
        trace_context: Some(trace),
        runtime_snapshot_context: Some(runtime_snapshot),
        sticky_key: Some("sticky-guard-deferred"),
        preferred_account: Some(account),
        failover_progress: PoolFailoverProgress::default(),
        same_account_attempts: 1,
    })
    .await
    .expect("request should still succeed without a persisted pending attempt row");

    assert_pending_attempt_insert_failure_response(&upstream);
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    assert_no_pending_attempt_invocation(&state.pool, invoke_id, occurred_at).await;

    disarm_pool_early_phase_cleanup_guard(&mut upstream.deferred_early_phase_cleanup_guard);
    upstream_handle.abort();
}

fn assert_pending_attempt_insert_failure_response(upstream: &PoolUpstreamResponse) {
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
}

async fn assert_no_pending_attempt_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
) {
    let invocation_count = count_test_invocation_rows(pool, invoke_id, occurred_at).await;
    assert_eq!(
        invocation_count, 0,
        "running runtime state should not create a codex_invocations row"
    );
}

#[tokio::test]
pub(crate) async fn send_pool_request_with_failover_disarms_guard_after_streaming_phase_is_persisted()
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

    let account = build_test_pool_resolved_account(account_id, &upstream_base);

    let upstream = send_pool_request_with_failover(PoolFailoverRequest {
        state: state.clone(),
        proxy_request_id: 6768,
        method: Method::POST,
        original_uri: &"/v1/responses".parse().expect("valid uri"),
        headers: &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        body: Some(PoolReplayBodySnapshot::Memory(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "hello"}]
            }))
            .expect("serialize request body"),
        ))),
        handshake_timeout: Duration::from_secs(5),
        trace_context: Some(trace),
        runtime_snapshot_context: Some(runtime_snapshot),
        sticky_key: Some("sticky-guard-streaming-phase-persisted"),
        preferred_account: Some(account),
        failover_progress: PoolFailoverProgress::default(),
        same_account_attempts: 1,
    })
    .await
    .expect("request should succeed when streaming-phase persistence succeeds");

    assert_eq!(upstream.response.status(), StatusCode::OK);
    assert!(
        upstream
            .pending_attempt_record
            .as_ref()
            .and_then(|pending| pending.attempt_id)
            .is_some()
    );
    assert!(upstream.deferred_early_phase_cleanup_guard.is_some());
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after persisted streaming-phase update");
    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );
    let mut upstream = upstream;
    disarm_pool_early_phase_cleanup_guard(&mut upstream.deferred_early_phase_cleanup_guard);
    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn send_pool_request_with_failover_keeps_early_phase_guard_armed_when_streaming_phase_was_not_persisted()
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

    suppress_streaming_phase_update(&state.pool).await;

    let account = build_test_pool_resolved_account(account_id, &upstream_base);

    let upstream = send_pool_request_with_failover(PoolFailoverRequest {
        state: state.clone(),
        proxy_request_id: 6768,
        method: Method::POST,
        original_uri: &"/v1/responses".parse().expect("valid uri"),
        headers: &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        body: Some(PoolReplayBodySnapshot::Memory(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{"role": "user", "content": "hello"}]
            }))
            .expect("serialize request body"),
        ))),
        handshake_timeout: Duration::from_secs(5),
        trace_context: Some(trace),
        runtime_snapshot_context: Some(runtime_snapshot),
        sticky_key: Some("sticky-guard-streaming-phase"),
        preferred_account: Some(account),
        failover_progress: PoolFailoverProgress::default(),
        same_account_attempts: 1,
    })
    .await
    .expect("request should still succeed when streaming-phase persistence is suppressed");

    assert_eq!(upstream.response.status(), StatusCode::OK);
    assert!(
        upstream
            .pending_attempt_record
            .as_ref()
            .and_then(|pending| pending.attempt_id)
            .is_some()
    );
    assert!(upstream.deferred_early_phase_cleanup_guard.is_some());

    drop(upstream);
    let attempt = wait_for_interrupted_attempt(&state.pool, invoke_id, occurred_at).await;
    assert_eq!(
        attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert_eq!(
        attempt.2.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
    assert_eq!(
        count_test_invocation_rows(&state.pool, invoke_id, occurred_at).await,
        0
    );

    upstream_handle.abort();
}

#[tokio::test]
pub(crate) async fn recover_proxy_invocations_with_selector_batches_handles_large_selector_sets() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let selector_count = PROXY_INVOCATION_RECOVERY_SELECTOR_BATCH_SIZE + 125;
    let mut selectors = Vec::with_capacity(selector_count);

    for index in 0..selector_count {
        let invoke_id = format!("selector-batch-invocation-{index}");
        let occurred_at = format!("2026-03-23 21:{:02}:{:02}", (index / 60) % 60, index % 60);
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                raw_response,
                payload
            )
            VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)
            "#,
        )
        .bind(&invoke_id)
        .bind(&occurred_at)
        .bind(SOURCE_PROXY)
        .bind(INVOCATION_STATUS_RUNNING)
        .bind("{}")
        .bind("{\"endpoint\":\"/v1/responses\"}")
        .execute(&state.pool)
        .await
        .expect("insert running proxy invocation");
        selectors.push(InvocationRecoverySelector::new(invoke_id, occurred_at));
    }

    let recovered = recover_proxy_invocations_with_scope(
        &state.pool,
        ProxyInvocationRecoveryScope::Selectors(&selectors),
    )
    .await
    .expect("recover large selector batch");

    assert_eq!(recovered.len(), selector_count);

    let interrupted_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE invoke_id LIKE 'selector-batch-invocation-%'
          AND status = ?1
          AND failure_kind = ?2
        "#,
    )
    .bind(INVOCATION_STATUS_INTERRUPTED)
    .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    .fetch_one(&state.pool)
    .await
    .expect("count interrupted selector rows");

    assert_eq!(interrupted_count as usize, selector_count);
}

struct OrphanRecoveryCase<'a> {
    invoke_id: &'a str,
    occurred_at: &'a str,
    started_at: &'a str,
    phase: &'a str,
    persist_attempt_first_byte_progress: bool,
    ttfb_ms: f64,
}

async fn seed_orphan_recovery_case(
    state: &Arc<AppState>,
    account_id: i64,
    request_info: &RequestCaptureInfo,
    case: OrphanRecoveryCase<'_>,
) {
    let running_record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
        case.invoke_id,
        case.occurred_at,
        ProxyCaptureTarget::Responses,
        request_info,
        Some("198.51.100.22"),
        Some("sticky-sweeper"),
        None,
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
        5.0,
        case.ttfb_ms,
    ));
    persist_and_broadcast_proxy_capture_runtime_snapshot(state, running_record)
        .await
        .expect("persist running invocation");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: case.invoke_id.to_string(),
        occurred_at: case.occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        PoolAttemptStartScope {
            upstream_account_id: account_id,
            upstream_route_key: "route-primary",
            ..PoolAttemptStartScope::default()
        },
        PoolAttemptStartIndexes {
            attempt_index: 1,
            distinct_account_index: 1,
            same_account_retry_index: 1,
        },
        case.started_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(state.as_ref(), &pending, case.phase)
        .await
        .expect("advance attempt into target phase");
    if case.persist_attempt_first_byte_progress {
        sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET first_byte_latency_ms = 120.0
            WHERE id = ?1
            "#,
        )
        .bind(pending.attempt_id.expect("pending attempt id"))
        .execute(&state.pool)
        .await
        .expect("persist attempt first-byte progress");
    }
}

async fn seed_orphan_recovery_cases(
    state: &Arc<AppState>,
    account_id: i64,
    request_info: &RequestCaptureInfo,
    stale_started: &str,
    fresh_started: &str,
) {
    let cases = [
        OrphanRecoveryCase {
            invoke_id: "stale-early-phase",
            occurred_at: "2026-03-23 21:10:01",
            started_at: stale_started,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
            persist_attempt_first_byte_progress: false,
            ttfb_ms: 0.0,
        },
        OrphanRecoveryCase {
            invoke_id: "fresh-early-phase",
            occurred_at: "2026-03-23 21:10:02",
            started_at: fresh_started,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
            persist_attempt_first_byte_progress: false,
            ttfb_ms: 0.0,
        },
        OrphanRecoveryCase {
            invoke_id: "stale-early-phase-post-first-byte",
            occurred_at: "2026-03-23 21:10:25",
            started_at: stale_started,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
            persist_attempt_first_byte_progress: true,
            ttfb_ms: 120.0,
        },
        OrphanRecoveryCase {
            invoke_id: "stale-early-phase-attempt-first-byte-progress",
            occurred_at: "2026-03-23 21:10:26",
            started_at: stale_started,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
            persist_attempt_first_byte_progress: true,
            ttfb_ms: 0.0,
        },
        OrphanRecoveryCase {
            invoke_id: "stale-streaming-phase",
            occurred_at: "2026-03-23 21:10:03",
            started_at: stale_started,
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
            persist_attempt_first_byte_progress: false,
            ttfb_ms: 120.0,
        },
    ];
    for case in cases {
        seed_orphan_recovery_case(state, account_id, request_info, case).await;
    }
}

async fn assert_recovered_orphan_cases(state: &Arc<AppState>) {
    let stale_attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = 'stale-early-phase'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load stale recovered attempt");
    assert_eq!(
        stale_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        stale_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );

    let stale_invocation_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*) FROM codex_invocations
        WHERE invoke_id = 'stale-early-phase'
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count stale recovered invocation rows");
    assert_eq!(stale_invocation_count, 0);

    let stale_post_first_byte_attempt = sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
        r#"
            SELECT status, phase, first_byte_latency_ms
            FROM pool_upstream_request_attempts
            WHERE invoke_id = 'stale-early-phase-post-first-byte'
            ORDER BY id DESC
            LIMIT 1
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load stale post-first-byte attempt");
    assert_eq!(
        stale_post_first_byte_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        stale_post_first_byte_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(stale_post_first_byte_attempt.2, Some(120.0));
}

async fn assert_unrecovered_orphan_cases(state: &Arc<AppState>) {
    for invoke_id in [
        "stale-early-phase-post-first-byte",
        "stale-early-phase-attempt-first-byte-progress",
        "stale-streaming-phase",
    ] {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1",
        )
        .bind(invoke_id)
        .fetch_one(&state.pool)
        .await
        .expect("count unrecovered invocation rows");
        assert_eq!(count, 0);
    }

    let progress_attempt = sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
        r#"
        SELECT status, phase, first_byte_latency_ms
        FROM pool_upstream_request_attempts
        WHERE invoke_id = 'stale-early-phase-attempt-first-byte-progress'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load attempt-first-byte-progress attempt");
    assert_eq!(
        progress_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        progress_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(progress_attempt.2, Some(120.0));

    for (invoke_id, expected_phase) in [
        (
            "fresh-early-phase",
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
        ),
        (
            "stale-streaming-phase",
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
        ),
    ] {
        let attempt = sqlx::query_as::<_, (String, Option<String>)>(
            r#"
            SELECT status, phase
            FROM pool_upstream_request_attempts
            WHERE invoke_id = ?1
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind(invoke_id)
        .fetch_one(&state.pool)
        .await
        .expect("load unrecovered attempt");
        assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
        assert_eq!(attempt.1.as_deref(), Some(expected_phase));
    }
}

#[tokio::test]
pub(crate) async fn recover_stale_pool_early_phase_orphans_runtime_only_recovers_stale_early_rows()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let fresh_started = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_orphan_recovery_cases(
        &state,
        account_id,
        &request_info,
        &stale_started,
        &fresh_started,
    )
    .await;

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("recover stale early-phase orphans");
    assert_eq!(
        outcome,
        PoolOrphanRecoveryOutcome {
            recovered_attempts: 1,
            recovered_invocations: 0,
        }
    );
    assert_recovered_orphan_cases(&state).await;
    assert_unrecovered_orphan_cases(&state).await;
}
#[tokio::test]
pub(crate) async fn recover_stale_pool_early_phase_orphans_runtime_skips_active_live_attempts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "stale-active-live-attempt";
    let occurred_at = "2026-03-23 21:10:30";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.23"),
        Some("sticky-active-live-attempt"),
        None,
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
    ));
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-active-live-attempt".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        PoolAttemptStartScope {
            upstream_account_id: account_id,
            upstream_route_key: "route-primary",
            ..PoolAttemptStartScope::default()
        },
        PoolAttemptStartIndexes {
            attempt_index: 1,
            distinct_account_index: 1,
            same_account_retry_index: 1,
        },
        stale_started.as_str(),
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    let _live_attempt_activity_lease = PoolLiveAttemptActivityLease::new(
        state.clone(),
        pending.attempt_id.expect("pending attempt id"),
    );

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("skip active live attempt during stale recovery");
    assert_eq!(outcome, PoolOrphanRecoveryOutcome::default());

    assert_active_recovery_untouched(&state.pool, invoke_id, occurred_at).await;
}

#[tokio::test]
pub(crate) async fn recover_stale_pool_early_phase_orphans_runtime_rolls_back_attempts_when_invocation_recovery_fails()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "stale-sweeper-atomic-recovery-failure";
    let occurred_at = "2026-03-23 21:10:31";
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    persist_atomic_recovery_running_record(&state, account_id, invoke_id, occurred_at).await;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            raw_response,
            payload
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(INVOCATION_STATUS_RUNNING)
    .bind("{}")
    .bind("{\"endpoint\":\"/v1/responses\"}")
    .execute(&state.pool)
    .await
    .expect("insert legacy running invocation for rollback coverage");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-atomic".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        PoolAttemptStartScope {
            upstream_account_id: account_id,
            upstream_route_key: "route-primary",
            ..PoolAttemptStartScope::default()
        },
        PoolAttemptStartIndexes {
            attempt_index: 1,
            distinct_account_index: 1,
            same_account_retry_index: 1,
        },
        &stale_started,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    fail_invocation_recovery_and_assert_rollback(&state, invoke_id, occurred_at).await;
}

async fn persist_atomic_recovery_running_record(
    state: &Arc<AppState>,
    account_id: i64,
    invoke_id: &str,
    occurred_at: &str,
) {
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let running_record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.31"),
        Some("sticky-sweeper-atomic"),
        None,
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
        5.0,
        0.0,
    ));
    persist_and_broadcast_proxy_capture_runtime_snapshot(state, running_record)
        .await
        .expect("persist running invocation");
}

async fn fail_invocation_recovery_and_assert_rollback(
    state: &Arc<AppState>,
    invoke_id: &str,
    occurred_at: &str,
) {
    sqlx::query(
        "CREATE TRIGGER fail_stale_sweeper_invocation_recovery BEFORE UPDATE ON codex_invocations WHEN OLD.invoke_id = 'stale-sweeper-atomic-recovery-failure' BEGIN SELECT RAISE(FAIL, 'simulated stale sweeper invocation recovery failure'); END;",
    )
    .execute(&state.pool)
    .await
    .expect("create failing stale sweeper trigger");
    let err = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect_err("stale sweeper should fail when invocation recovery aborts");
    assert!(
        err.to_string()
            .contains("simulated stale sweeper invocation recovery failure"),
        "unexpected error: {err:?}"
    );
    assert_failed_recovery_rolled_back(&state.pool, invoke_id, occurred_at).await;
}
