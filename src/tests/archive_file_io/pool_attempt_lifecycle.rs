use super::*;

#[tokio::test]
async fn finalize_pool_upstream_request_attempt_updates_pending_row_in_place() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-update".to_string(),
        occurred_at: "2026-03-23 20:49:00".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-pending".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };

    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:00",
    )
    .await;
    let attempt_id = pending
        .attempt_id
        .expect("pending attempt should be inserted immediately");

    let pending_row = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>)>(
        r#"
        SELECT id, status, phase, finished_at
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("load pending attempt row");
    assert_eq!(pending_row.0, attempt_id);
    assert_eq!(pending_row.1, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        pending_row.2.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING)
    );
    assert_eq!(pending_row.3, None);

    finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        "2026-03-23 20:49:05",
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(StatusCode::OK),
        None,
        None,
        None,
        None,
        Some(42.5),
        Some(15.0),
        Some(188.4),
        Some("req_pool_123"),
        None,
        None,
    )
    .await
    .expect("finalize pending attempt");

    let rows = sqlx::query_as::<
        _,
        (
            i64,
            String,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<String>,
        ),
    >(
        r#"
        SELECT
            id,
            status,
            phase,
            http_status,
            finished_at,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
        ORDER BY id ASC
        "#,
    )
    .bind("pending-attempt-update")
    .fetch_all(&state.pool)
    .await
    .expect("load finalized attempt rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, attempt_id);
    assert_eq!(rows[0].1, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_eq!(
        rows[0].2.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
    );
    assert_eq!(rows[0].3, Some(200));
    assert_eq!(rows[0].4.as_deref(), Some("2026-03-23 20:49:05"));
    assert_eq!(rows[0].5, Some(42.5));
    assert_eq!(rows[0].6, Some(15.0));
    assert_eq!(rows[0].7, Some(188.4));
    assert_eq!(rows[0].8.as_deref(), Some("req_pool_123"));
}

#[tokio::test]
async fn begin_pool_upstream_request_attempt_with_scope_persists_group_and_proxy_snapshots() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Scoped", "upstream-scoped").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-scope".to_string(),
        occurred_at: "2026-03-23 20:59:00".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-scope".to_string()),
        requester_ip: Some("192.168.31.9".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };

    let pending = begin_pool_upstream_request_attempt_with_scope(
        &state.pool,
        &trace,
        Some("prod"),
        Some(FORWARD_PROXY_DIRECT_KEY),
        account_id,
        "route-scoped",
        1,
        1,
        0,
        "2026-03-23 20:59:00",
    )
    .await;
    let attempt_id = pending
        .attempt_id
        .expect("pending scoped attempt should be inserted immediately");

    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        r#"
        SELECT group_name_snapshot, proxy_binding_key_snapshot
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("load scoped attempt row");
    assert_eq!(row.0.as_deref(), Some("prod"));
    assert_eq!(row.1.as_deref(), Some(FORWARD_PROXY_DIRECT_KEY));
}

#[tokio::test]
async fn finalize_pool_upstream_request_attempt_fallback_preserves_scope_snapshots() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Scoped fallback", "upstream-fallback").await;
    let pending = PendingPoolAttemptRecord {
        attempt_id: None,
        attempt_public_id: None,
        invoke_id: "pending-attempt-scope-fallback".to_string(),
        occurred_at: "2026-03-23 21:00:00".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-scope-fallback".to_string()),
        routing_source: None,
        routing_selection_audit_json: None,
        sticky_affinity_generation: None,
        requester_ip: Some("192.168.31.10".to_string()),
        upstream_base_url_host: None,
        request_model: None,
        group_name_snapshot: Some("prod".to_string()),
        proxy_binding_key_snapshot: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
        upstream_account_id: account_id,
        upstream_route_key: "route-scoped-fallback".to_string(),
        attempt_index: 2,
        distinct_account_index: 1,
        same_account_retry_index: 0,
        started_at: "2026-03-23 21:00:00".to_string(),
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
        ..Default::default()
    };

    finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        "2026-03-23 21:00:01",
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        None,
        None,
        Some("fallback_transport_failure"),
        Some("fallback transport failure"),
        None,
        Some(11.0),
        Some(17.0),
        None,
        Some("req_scope_fallback"),
        None,
        None,
    )
    .await
    .expect("finalize fallback scoped attempt");

    let row = sqlx::query_as::<_, (Option<String>, Option<String>, String)>(
        r#"
        SELECT group_name_snapshot, proxy_binding_key_snapshot, status
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind("pending-attempt-scope-fallback")
    .bind("2026-03-23 21:00:00")
    .fetch_one(&state.pool)
    .await
    .expect("load fallback scoped attempt row");
    assert_eq!(row.0.as_deref(), Some("prod"));
    assert_eq!(row.1.as_deref(), Some(FORWARD_PROXY_DIRECT_KEY));
    assert_eq!(
        row.2,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
}

#[tokio::test]
async fn insert_pool_upstream_terminal_attempt_skips_pre_dispatch_pseudo_attempt_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Scoped terminal", "upstream-terminal-scoped")
            .await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "terminal-scoped-snapshots".to_string(),
        occurred_at: "2026-03-23 21:00:02".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-terminal-scoped".to_string()),
        requester_ip: Some("192.168.31.11".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };

    insert_pool_upstream_terminal_attempt(
        &state.pool,
        &trace,
        &PoolUpstreamError {
            account: Some(PoolResolvedAccount {
                account_id,
                display_name: "Scoped terminal".to_string(),
                kind: "api_key_codex".to_string(),
                auth: PoolResolvedAuth::ApiKey {
                    authorization: "Bearer sk-test-terminal".to_string(),
                },
                group_name: Some("prod".to_string()),
                bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string()],
                forward_proxy_scope: ForwardProxyRouteScope::pinned(FORWARD_PROXY_DIRECT_KEY),
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
                upstream_base_url: Url::parse("https://api.openai.com/")
                    .expect("valid upstream base"),
                routing_source: PoolRoutingSelectionSource::FreshAssignment,
                routing_control_fingerprint: None,
                sticky_affinity_generation: None,
                routing_selection_audit: None,
                priority_handoff_permit: None,
            }),
            status: StatusCode::BAD_GATEWAY,
            message: "terminal scoped failure".to_string(),
            canonical_error_message: Some("terminal scoped failure".to_string()),
            failure_kind: PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
            blocked_binding: None,
            connect_latency_ms: 0.0,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: Some("req_terminal_scope".to_string()),
            proxy_binding_key_snapshot: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
            oauth_responses_debug: None,
            attempt_summary: PoolAttemptSummary::default(),
            requested_service_tier: None,
            request_body_for_capture: None,
            codex_imagegen_rewrite: None,
        },
        3,
        1,
        PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
    )
    .await
    .expect("insert scoped terminal attempt");

    let row_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind("terminal-scoped-snapshots")
    .bind("2026-03-23 21:00:02")
    .fetch_one(&state.pool)
    .await
    .expect("count scoped terminal attempt rows");
    assert_eq!(row_count, 0);
}

#[tokio::test]
async fn insert_pool_upstream_terminal_attempt_skips_oauth_pre_dispatch_pseudo_attempt_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "terminal-oauth-transport-normalization".to_string(),
        occurred_at: "2026-03-23 20:49:06".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-terminal-oauth".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };

    insert_pool_upstream_terminal_attempt(
        &state.pool,
        &trace,
        &PoolUpstreamError {
            account: None,
            status: StatusCode::BAD_GATEWAY,
            message: "pool upstream responded with 502: oauth codex upstream handshake timed out"
                .to_string(),
            canonical_error_message: Some("oauth codex upstream handshake timed out".to_string()),
            failure_kind: PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
            blocked_binding: None,
            connect_latency_ms: 0.0,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: Some(
                "pool upstream responded with 502: oauth codex upstream handshake timed out"
                    .to_string(),
            ),
            upstream_request_id: Some("req_oauth_terminal_1".to_string()),
            proxy_binding_key_snapshot: None,
            oauth_responses_debug: None,
            attempt_summary: PoolAttemptSummary::default(),
            requested_service_tier: None,
            request_body_for_capture: None,
            codex_imagegen_rewrite: None,
        },
        2,
        1,
        PROXY_FAILURE_POOL_TOTAL_TIMEOUT_EXHAUSTED,
    )
    .await
    .expect("insert terminal attempt");

    let row_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
        "#,
    )
    .bind("terminal-oauth-transport-normalization")
    .fetch_one(&state.pool)
    .await
    .expect("count terminal oauth transport attempt rows");

    assert_eq!(row_count, 0);
}

#[tokio::test]
async fn broadcast_pool_upstream_attempts_snapshot_emits_pending_attempts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-broadcast".to_string(),
        occurred_at: "2026-03-23 20:49:02".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-broadcast".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let _pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:02",
    )
    .await;

    let mut rx = state.broadcaster.subscribe();
    broadcast_pool_upstream_attempts_snapshot(state.as_ref(), "pending-attempt-broadcast")
        .await
        .expect("broadcast pool attempt snapshot");

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for pool-attempt snapshot")
        .expect("broadcast channel should stay open");
    match payload {
        BroadcastPayload::PoolAttempts {
            invoke_id,
            attempts,
        } => {
            assert_eq!(invoke_id, "pending-attempt-broadcast");
            assert_eq!(attempts.len(), 1);
            assert_eq!(
                attempts[0].status,
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
            );
            assert_eq!(
                attempts[0].phase,
                POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING
            );
            assert_eq!(attempts[0].finished_at, None);
            assert_eq!(attempts[0].upstream_account_id, Some(account_id));
        }
        other => panic!("expected pool-attempts payload, got {other:?}"),
    }
}

#[tokio::test]
async fn advance_pool_upstream_request_attempt_phase_buffers_progress_without_immediate_broadcast()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-phase-advance".to_string(),
        occurred_at: "2026-03-23 20:49:06".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-phase-advance".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:06",
    )
    .await;

    let mut rx = state.broadcaster.subscribe();
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("enqueue phase progress");

    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err(),
        "buffered phase progress should not broadcast a stale DB snapshot"
    );

    SqliteBatchWriter::flush_for_test(
        &state.pool,
        vec![SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
            attempt_id: pending.attempt_id.expect("pending attempt id"),
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST.to_string()),
            connect_latency_ms: None,
            first_byte_latency_ms: None,
            compact_support_status: None,
            compact_support_reason: None,
            ..Default::default()
        })],
    )
    .await;

    let row = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
        "#,
    )
    .bind("pending-attempt-phase-advance")
    .fetch_one(&state.pool)
    .await
    .expect("load buffered phase progress");

    assert_eq!(row.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        row.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );
}

#[tokio::test]
async fn fetch_invocation_pool_attempts_returns_live_pending_attempts_without_parent_invocation() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-live-fetch".to_string(),
        occurred_at: "2026-03-23 20:49:02".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-live-fetch".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let _sticky_pending = begin_pool_upstream_request_attempt_with_scope_and_routing_source(
        &state.pool,
        &trace,
        Some("live-fetch-group"),
        Some(FORWARD_PROXY_DIRECT_KEY),
        Some(PoolRoutingSelectionSource::StickyReuse),
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:02",
    )
    .await;
    let _fresh_pending = begin_pool_upstream_request_attempt_with_scope_and_routing_source(
        &state.pool,
        &PoolUpstreamAttemptTraceContext {
            sticky_key: None,
            requester_ip: Some("192.168.31.7".to_string()),
            ..trace.clone()
        },
        Some("live-fetch-group"),
        Some(FORWARD_PROXY_DIRECT_KEY),
        Some(PoolRoutingSelectionSource::FreshAssignment),
        account_id,
        "route-fallback",
        2,
        2,
        1,
        "2026-03-23 20:49:03",
    )
    .await;

    let Json(attempts) = fetch_invocation_pool_attempts(
        State(state.clone()),
        axum::extract::Path("pending-attempt-live-fetch".to_string()),
    )
    .await
    .expect("fetch live pending attempts");

    assert_eq!(attempts.len(), 2);
    assert_eq!(
        attempts[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        attempts[0].phase,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING
    );
    assert_eq!(attempts[0].finished_at, None);
    assert_eq!(attempts[0].upstream_account_id, Some(account_id));
    assert_eq!(attempts[0].routing_source.as_deref(), Some("stickyReuse"));
    assert_eq!(
        attempts[1].routing_source.as_deref(),
        Some("freshAssignment")
    );
}

#[tokio::test]
async fn insert_and_broadcast_pool_upstream_terminal_attempt_skips_pre_dispatch_snapshots() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "terminal-attempt-broadcast".to_string(),
        occurred_at: "2026-03-23 20:49:03".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-terminal".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let final_error = PoolUpstreamError {
        account: None,
        status: StatusCode::BAD_GATEWAY,
        message: "pool budget exhausted after failover".to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED,
        blocked_binding: None,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: Some("req_terminal_123".to_string()),
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            3,
            3,
            Some(PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
        codex_imagegen_rewrite: None,
    };

    let mut rx = state.broadcaster.subscribe();
    insert_and_broadcast_pool_upstream_terminal_attempt(
        state.as_ref(),
        &trace,
        &final_error,
        4,
        3,
        PROXY_FAILURE_POOL_MAX_DISTINCT_ACCOUNTS_EXHAUSTED,
    )
    .await
    .expect("insert and broadcast terminal attempt");

    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err(),
        "pre-dispatch terminal attempts should not broadcast pseudo attempt snapshots"
    );
    let row_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pool_upstream_request_attempts WHERE invoke_id = ?1",
    )
    .bind("terminal-attempt-broadcast")
    .fetch_one(&state.pool)
    .await
    .expect("count terminal-attempt broadcast rows");
    assert_eq!(row_count, 0);
}

#[tokio::test]
async fn recover_orphaned_pool_upstream_request_attempts_marks_pending_rows_terminal() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "pending-attempt-recovery".to_string(),
        occurred_at: "2026-03-23 20:49:04".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-recovery".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };

    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        "2026-03-23 20:49:04",
    )
    .await;

    let affected = recover_orphaned_pool_upstream_request_attempts(&state.pool)
        .await
        .expect("recover pending attempts");
    assert_eq!(affected, 1);

    let recovered_row = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        r#"
        SELECT status, phase, finished_at, failure_kind, error_message
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load recovered pending attempt");

    assert_eq!(
        recovered_row.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        recovered_row.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert!(
        recovered_row
            .2
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(
        recovered_row.3.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
    assert_eq!(
        recovered_row.4.as_deref(),
        Some(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
    );
}

#[tokio::test]
async fn recover_orphaned_pool_upstream_request_attempts_keeps_startup_sequence_recoverable() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "startup-recovered-pending-attempt";
    let occurred_at = "2026-03-23 20:49:14";
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
        Some("198.51.100.26"),
        Some("sticky-startup-recovery"),
        Some("pck-startup-recovery"),
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
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-startup-recovery".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance attempt into sending-request");

    let recovered_invocations = recover_orphaned_proxy_invocations(&state.pool)
        .await
        .expect("recover orphaned invocations first");
    assert_eq!(recovered_invocations, 0);

    let recovered_attempts = recover_orphaned_pool_upstream_request_attempts(&state.pool)
        .await
        .expect("recover orphaned pending attempts after invocation cleanup");
    assert_eq!(recovered_attempts, 1);

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load recovered pending attempt");
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
    .expect("count startup-recovered invocation rows");

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
        invocation_count, 0,
        "memory-only running snapshots should not create DB invocation rows for startup recovery"
    );
}

#[tokio::test]
async fn recover_orphaned_pool_upstream_request_attempts_recovers_terminal_invocation_rows() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "startup-skip-successful-invocation";
    let occurred_at = "2026-03-23 20:49:24";
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
        Some("198.51.100.27"),
        Some("sticky-startup-success"),
        Some("pck-startup-success"),
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
        15.0,
        120.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist success candidate invocation");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-startup-success".to_string()),
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
    };
    let pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &trace,
        account_id,
        "route-primary",
        1,
        1,
        1,
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await
    .expect("advance attempt into waiting-first-byte");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            error_message,
            raw_response,
            payload,
            failure_kind,
            failure_class,
            is_actionable
        )
        VALUES (?1, ?2, ?3, 'success', NULL, ?4, ?5, NULL, NULL, 0)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind("{}")
    .bind("{\"endpoint\":\"/v1/responses\"}")
    .execute(&state.pool)
    .await
    .expect("finalize invocation before startup attempt recovery");

    let recovered_attempts = recover_orphaned_pool_upstream_request_attempts(&state.pool)
        .await
        .expect("recover startup attempt rows for terminal invocation");
    assert_eq!(recovered_attempts, 1);

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after skipped startup recovery");
    let invocation = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("load terminal invocation after startup attempt recovery");

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
    assert_eq!(invocation.0, "success");
    assert_eq!(invocation.1, None);
}
