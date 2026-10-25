use super::*;

#[tokio::test]
async fn recover_orphaned_proxy_invocations_marks_running_rows_interrupted() {
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
            error_message,
            raw_response,
            payload
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)
        "#,
    )
    .bind("recovered-running-invocation")
    .bind("2026-03-23 21:01:02")
    .bind(SOURCE_PROXY)
    .bind(INVOCATION_STATUS_RUNNING)
    .bind("{}")
    .bind("{\"endpoint\":\"/v1/responses\"}")
    .execute(&state.pool)
    .await
    .expect("insert running invocation");

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
    .bind("xy-running-invocation")
    .bind("2026-03-23 21:01:03")
    .bind(SOURCE_XY)
    .bind(INVOCATION_STATUS_RUNNING)
    .bind("{}")
    .bind("{\"endpoint\":\"/v1/chat/completions\"}")
    .execute(&state.pool)
    .await
    .expect("insert non-proxy running invocation");

    let affected = recover_orphaned_proxy_invocations(&state.pool)
        .await
        .expect("recover orphaned invocations");
    assert_eq!(affected, 1);

    let recovered = sqlx::query_as::<
        _,
        (
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
        ),
    >(
        r#"
        SELECT status, error_message, failure_kind, failure_class, is_actionable
        FROM codex_invocations
        WHERE invoke_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind("recovered-running-invocation")
    .fetch_one(&state.pool)
    .await
    .expect("load recovered invocation");

    assert_eq!(recovered.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(recovered.1.as_deref(), Some(INVOCATION_INTERRUPTED_MESSAGE));
    assert_eq!(
        recovered.2.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );
    assert_eq!(recovered.3.as_deref(), Some(FAILURE_CLASS_SERVICE));
    assert_eq!(recovered.4, Some(1));

    let xy_row = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, error_message
        FROM codex_invocations
        WHERE invoke_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind("xy-running-invocation")
    .fetch_one(&state.pool)
    .await
    .expect("load non-proxy invocation");

    assert_eq!(xy_row.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(xy_row.1, None);
}

#[tokio::test]
async fn pool_early_phase_orphan_cleanup_guard_recovers_dropped_sending_request_attempt() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recovered-pending-attempt";
    let occurred_at = "2026-03-23 21:09:04";
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
        Some("198.51.100.21"),
        Some("sticky-guard"),
        Some("pck-guard"),
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

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard".to_string()),
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

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), false, false)
        .await
        .expect("recover dropped early-phase attempt");

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
    .expect("count recovered invocation rows");

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
    assert_eq!(invocation_count, 0);
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_without_persisted_attempt_row_interrupts_invocation()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-skip-without-attempt-row";
    let occurred_at = "2026-03-23 21:09:05";
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
        Some("198.51.100.23"),
        Some("sticky-guard-skip"),
        Some("pck-guard-skip"),
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
        sticky_key: Some("sticky-guard-skip".to_string()),
        routing_source: None,
        routing_selection_audit_json: None,
        sticky_affinity_generation: None,
        requester_ip: Some("192.168.31.6".to_string()),
        upstream_base_url_host: None,
        request_model: None,
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
        ..Default::default()
    };

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending, false, false)
        .await
        .expect("recover dropped guard without persisted attempt row");

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
    .expect("count invocation rows after dropped guard recovery");

    assert_eq!(invocation_count, 0);
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_skips_streaming_response_attempt_after_task_drop()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recover-streaming-response";
    let occurred_at = "2026-03-23 21:09:05";
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
        Some("sticky-guard-streaming"),
        Some("pck-guard-streaming"),
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
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-streaming".to_string()),
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
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
    )
    .await
    .expect("advance attempt into streaming-response");

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), true, false)
        .await
        .expect("skip dropped streaming-response guard after task exit");

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
    .expect("load streaming attempt after dropped guard");
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
    .expect("count invocation rows after dropped streaming guard");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation_count, 0);
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_recovers_attempt_after_invocation_finalized()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-skip-finalized-invocation";
    let occurred_at = "2026-03-23 21:09:55";
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
        Some("198.51.100.25"),
        Some("sticky-guard-final"),
        Some("pck-guard-final"),
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
        120.0,
    );
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
        sticky_key: Some("sticky-guard-final".to_string()),
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
    .expect("finalize invocation before dropped guard recovery");

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), false, false)
        .await
        .expect("recover dropped guard attempt after finalized invocation");

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
    .expect("load finalized-invocation attempt after dropped guard");
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
    .expect("load finalized invocation after dropped guard");

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

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_skips_post_first_byte_terminal_attempt() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-skip-post-first-byte";
    let occurred_at = "2026-03-23 21:10:05";
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
        Some("198.51.100.28"),
        Some("sticky-guard-post-first-byte"),
        Some("pck-guard-post-first-byte"),
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

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-post-first-byte".to_string()),
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

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), true, true)
        .await
        .expect("skip dropped guard recovery after first byte");

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
    .expect("load post-first-byte attempt after dropped guard");
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
    .expect("count invocation rows after skipped post-first-byte guard");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation_count, 0);
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_recovers_post_first_byte_nonterminal_attempt()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recover-post-first-byte-nonterminal";
    let occurred_at = "2026-03-23 21:10:06";
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
        Some("198.51.100.29"),
        Some("sticky-guard-post-first-byte-recover"),
        Some("pck-guard-post-first-byte-recover"),
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

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-post-first-byte-recover".to_string()),
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

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), true, false)
        .await
        .expect("recover dropped guard after first byte before terminal outcome");

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
    .expect("load post-first-byte nonterminal attempt after dropped guard");
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
    .expect("count invocation rows after recovering post-first-byte nonterminal guard");

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
    assert_eq!(invocation_count, 0);
}

#[tokio::test]
async fn recover_guard_dropped_pool_terminal_invocation_orphan_repairs_running_invocation_after_attempt_already_finalized()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recover-invocation-after-final-attempt";
    let occurred_at = "2026-03-23 21:10:06";
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
        Some("198.51.100.39"),
        Some("sticky-guard-final-attempt"),
        Some("pck-guard-final-attempt"),
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
    .expect("insert legacy running invocation for terminal drop guard recovery");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-final-attempt".to_string()),
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
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
    )
    .await
    .expect("advance attempt into streaming-response");
    finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        shanghai_now_string().as_str(),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        Some(StatusCode::OK),
        None,
        None,
        None,
        None,
        Some(10.0),
        Some(20.0),
        Some(30.0),
        Some("upstream-req-1"),
        None,
        None,
    )
    .await
    .expect("persist terminal attempt before dropped guard recovery");

    recover_guard_dropped_pool_terminal_invocation_orphan(
        state.as_ref(),
        InvocationRecoverySelector::from(&pending),
    )
    .await
    .expect("recover running invocation after attempt already finalized");

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
    .expect("load terminal attempt after dropped guard recovery");
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
    .expect("load invocation after repairing dropped guard");
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
    .expect("load route state after dropped guard repair");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        invocation.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );
    assert_eq!(route_state.0, None);
    assert_eq!(route_state.1, None);
    assert_eq!(route_state.2, 0);
}

#[tokio::test]
async fn drop_guard_runtime_remove_does_not_tombstone_later_terminal_record() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "drop-guard-later-terminal";
    let occurred_at = "2026-03-23 21:10:16";
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
        Some("198.51.100.41"),
        Some("sticky-drop-guard-later-terminal"),
        Some("pck-drop-guard-later-terminal"),
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
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record.clone())
        .await
        .expect("store running invocation in memory");

    remove_proxy_runtime_snapshot_by_key(state.as_ref(), invoke_id, occurred_at, "drop_guard");

    let mut terminal_record = running_record;
    terminal_record.status = "success".to_string();
    terminal_record.usage.input_tokens = Some(2);
    terminal_record.usage.output_tokens = Some(3);
    terminal_record.usage.total_tokens = Some(5);
    terminal_record.cost = Some(0.02);
    persist_and_broadcast_proxy_capture_terminal_record(state.as_ref(), terminal_record)
        .await
        .expect("later terminal record should still enqueue after drop guard runtime cleanup");

    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2 AND status = 'success'
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count later terminal invocation rows");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn pool_invocation_cleanup_guard_recovers_running_invocation_during_retry_backoff_gap() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-request-drop-backoff-gap";
    let occurred_at = "2026-03-23 21:10:06";
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
        Some("198.51.100.40"),
        Some("sticky-request-drop-backoff"),
        Some("pck-request-drop-backoff"),
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

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-request-drop-backoff".to_string()),
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
    finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        shanghai_now_string().as_str(),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        None,
        None,
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
        Some("failed to contact upstream: retryable backoff"),
        None,
        Some(10.0),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    .expect("persist retryable attempt before simulated backoff drop");

    {
        let _guard = PoolInvocationCleanupGuard::new(
            state.clone(),
            InvocationRecoverySelector::from(&pending),
            "request_drop_guard",
            None,
        );
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

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
    .expect("load finalized retryable attempt after request-drop guard");
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
    .expect("count invocation rows after request-drop guard");

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
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
    );
    assert_eq!(
        invocation_count, 0,
        "memory-only running snapshots should not create request-drop DB invocation rows"
    );
}

#[tokio::test]
async fn recover_guard_dropped_pool_early_phase_orphan_rolls_back_attempt_when_invocation_recovery_fails()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-atomic-recovery-failure";
    let occurred_at = "2026-03-23 21:10:07";
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
        Some("198.51.100.30"),
        Some("sticky-guard-atomic"),
        Some("pck-guard-atomic"),
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
        sticky_key: Some("sticky-guard-atomic".to_string()),
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

    sqlx::query(
        r#"
        CREATE TRIGGER fail_guard_atomic_invocation_recovery
        BEFORE UPDATE ON codex_invocations
        WHEN OLD.invoke_id = 'guard-atomic-recovery-failure'
        BEGIN
            SELECT RAISE(FAIL, 'simulated guard invocation recovery failure');
        END;
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("create failing guard recovery trigger");

    let err = recover_guard_dropped_pool_early_phase_orphan(
        state.as_ref(),
        pending.clone(),
        false,
        false,
    )
    .await
    .expect_err("guard recovery should fail when invocation update aborts");
    assert!(
        err.to_string()
            .contains("simulated guard invocation recovery failure"),
        "unexpected error: {err:?}"
    );

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
    .expect("load attempt after failed guard recovery");
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
    .expect("load invocation after failed guard recovery");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "Mock reservation logs intentionally stay locked until async assertions observe requests."
)]
async fn recover_guard_dropped_pool_early_phase_orphan_clears_pool_routing_reservation_and_records_route_failure()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-4242-1775776407000";
    let occurred_at = "2026-03-23 21:10:08";
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
        Some("198.51.100.33"),
        Some("sticky-guard-reservation"),
        Some("pck-guard-reservation"),
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

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-guard-reservation".to_string()),
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
    let reservation_key = build_pool_routing_reservation_key(4242);
    reserve_test_pool_routing_account(&state, &reservation_key, account_id).await;

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending, false, false)
        .await
        .expect("recover dropped guard orphan and clean up route state");

    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    assert!(
        !reservations.contains_key(&reservation_key),
        "guard recovery should clear the in-memory reservation",
    );
    drop(reservations);

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
