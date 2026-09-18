use super::*;
use serde_json::json;

pub(crate) async fn seed_pool_attempt_for_recovery(
    state: &Arc<AppState>,
    account_id: i64,
    invoke_id: &str,
    occurred_at: &str,
    sticky_key: &str,
    phase: &str,
) -> PendingPoolAttemptRecord {
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
        Some("198.51.100.33"),
        Some(sticky_key),
        Some("pck-recovery-fixture"),
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
    persist_and_broadcast_proxy_capture_runtime_snapshot(state, running_record)
        .await
        .expect("persist recovery fixture invocation");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some(sticky_key.to_string()),
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
        occurred_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(state.as_ref(), &pending, phase)
        .await
        .expect("advance recovery fixture attempt phase");
    pending
}

pub(crate) async fn begin_test_pool_attempt(
    state: &AppState,
    account_id: i64,
    invoke_id: &str,
    occurred_at: &str,
    started_at: &str,
    sticky_key: &str,
    phase: &str,
) -> PendingPoolAttemptRecord {
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some(sticky_key.to_string()),
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
        started_at,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(state, &pending, phase)
        .await
        .expect("advance test pool attempt phase");
    pending
}

pub(crate) async fn seed_running_proxy_invocation(
    state: &Arc<AppState>,
    account_id: i64,
    invoke_id: &str,
    occurred_at: &str,
    sticky_key: &str,
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
        Some("198.51.100.23"),
        Some(sticky_key),
        Some("pck-recovery-fixture"),
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
    persist_and_broadcast_proxy_capture_runtime_snapshot(state, running_record)
        .await
        .expect("persist running invocation");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
}

pub(crate) fn build_unpersisted_pool_attempt(
    account_id: i64,
    invoke_id: &str,
    occurred_at: &str,
    sticky_key: &str,
) -> PendingPoolAttemptRecord {
    PendingPoolAttemptRecord {
        attempt_id: None,
        attempt_public_id: None,
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some(sticky_key.to_string()),
        routing_source: None,
        routing_selection_audit_json: None,
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
    }
}

pub(crate) async fn load_pool_attempt_state(
    pool: &SqlitePool,
    pending: &PendingPoolAttemptRecord,
) -> (String, Option<String>, Option<String>) {
    sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(pending.attempt_id.expect("pending attempt id"))
    .fetch_one(pool)
    .await
    .expect("load pool attempt state")
}

pub(crate) async fn count_invocation_rows(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(pool)
    .await
    .expect("count invocation rows")
}

async fn assert_attempt_and_invocation_count(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    invoke_id: &str,
    occurred_at: &str,
    expected_attempt: (&str, &str, Option<&str>),
    expected_invocations: i64,
) {
    let attempt = load_pool_attempt_state(&state.pool, pending).await;
    assert_eq!(attempt.0, expected_attempt.0);
    assert_eq!(attempt.1.as_deref(), Some(expected_attempt.1));
    assert_eq!(attempt.2.as_deref(), expected_attempt.2);
    assert_eq!(
        count_invocation_rows(&state.pool, invoke_id, occurred_at).await,
        expected_invocations
    );
}

pub(crate) fn assert_interrupted_pool_attempt(state: &(String, Option<String>, Option<String>)) {
    assert_eq!(
        state.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        state.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    );
    assert_eq!(
        state.2.as_deref(),
        Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    );
}

pub(crate) async fn insert_running_invocation(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
) {
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
    .expect("insert running invocation");
}

pub(crate) async fn insert_success_invocation(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, error_message,
            raw_response, payload, failure_kind, failure_class, is_actionable
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
    .expect("insert successful invocation");
}

pub(crate) async fn create_guard_recovery_failure_trigger(pool: &SqlitePool) {
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
    .execute(pool)
    .await
    .expect("create failing guard recovery trigger");
}

pub(crate) async fn load_invocation_status_and_failure(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
) -> (String, Option<String>) {
    sqlx::query_as::<_, (String, Option<String>)>(
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
    .fetch_one(pool)
    .await
    .expect("load invocation status and failure")
}

pub(crate) async fn insert_running_source_invocation(
    state: &AppState,
    source: &str,
    invoke_id: &str,
    occurred_at: &str,
    endpoint: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, error_message,
            raw_response, payload
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(source)
    .bind(INVOCATION_STATUS_RUNNING)
    .bind("{}")
    .bind(json!({ "endpoint": endpoint }).to_string())
    .execute(&state.pool)
    .await
    .expect("insert source invocation");
}

pub(crate) fn assert_pool_orphan_recovery(
    actual: PoolOrphanRecoveryOutcome,
    recovered_attempts: usize,
    recovered_invocations: usize,
) {
    assert_eq!(
        actual,
        PoolOrphanRecoveryOutcome {
            recovered_attempts,
            recovered_invocations,
        }
    );
}

pub(crate) async fn assert_terminal_guard_repair(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    invoke_id: &str,
    occurred_at: &str,
    account_id: i64,
) {
    let attempt = load_pool_attempt_state(&state.pool, pending).await;
    let invocation = load_invocation_status_and_failure(&state.pool, invoke_id, occurred_at).await;
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
pub(crate) async fn recover_orphaned_pool_upstream_request_attempts_keeps_startup_sequence_recoverable()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "startup-recovered-pending-attempt";
    let occurred_at = "2026-03-23 20:49:14";
    let pending = seed_pool_attempt_for_recovery(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-startup-recovery",
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let recovered_invocations = recover_orphaned_proxy_invocations(&state.pool)
        .await
        .expect("recover orphaned invocations first");
    assert_eq!(recovered_invocations, 0);

    let recovered_attempts = recover_orphaned_pool_upstream_request_attempts(&state.pool)
        .await
        .expect("recover orphaned pending attempts after invocation cleanup");
    assert_eq!(recovered_attempts, 1);

    let attempt = load_pool_attempt_state(&state.pool, &pending).await;
    let invocation_count = count_invocation_rows(&state.pool, invoke_id, occurred_at).await;
    assert_interrupted_pool_attempt(&attempt);
    assert_eq!(
        invocation_count, 0,
        "memory-only running snapshots should not create DB invocation rows for startup recovery"
    );
}

#[tokio::test]
pub(crate) async fn recover_orphaned_pool_upstream_request_attempts_recovers_terminal_invocation_rows()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "startup-skip-successful-invocation";
    let occurred_at = "2026-03-23 20:49:24";
    let pending = seed_pool_attempt_for_recovery(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-startup-success",
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await;

    insert_success_invocation(state.as_ref(), invoke_id, occurred_at).await;

    let recovered_attempts = recover_orphaned_pool_upstream_request_attempts(&state.pool)
        .await
        .expect("recover startup attempt rows for terminal invocation");
    assert_eq!(recovered_attempts, 1);

    let attempt = load_pool_attempt_state(&state.pool, &pending).await;
    let invocation = load_invocation_status_and_failure(&state.pool, invoke_id, occurred_at).await;

    assert_interrupted_pool_attempt(&attempt);
    assert_eq!(invocation.0, "success");
    assert_eq!(invocation.1, None);
}

#[tokio::test]
pub(crate) async fn recover_orphaned_proxy_invocations_marks_running_rows_interrupted() {
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

    insert_running_source_invocation(
        state.as_ref(),
        SOURCE_XY,
        "xy-running-invocation",
        "2026-03-23 21:01:03",
        "/v1/chat/completions",
    )
    .await;

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
pub(crate) async fn pool_early_phase_orphan_cleanup_guard_recovers_dropped_sending_request_attempt()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recovered-pending-attempt";
    let occurred_at = "2026-03-23 21:09:04";
    let pending = seed_pool_attempt_for_recovery(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard",
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await;

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), false, false)
        .await
        .expect("recover dropped early-phase attempt");

    assert_attempt_and_invocation_count(
        state.as_ref(),
        &pending,
        invoke_id,
        occurred_at,
        (
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED,
            Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED),
        ),
        0,
    )
    .await;
}

#[tokio::test]
pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan_without_persisted_attempt_row_interrupts_invocation()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-skip-without-attempt-row";
    let occurred_at = "2026-03-23 21:09:05";
    seed_running_proxy_invocation(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard-skip",
    )
    .await;
    let pending =
        build_unpersisted_pool_attempt(account_id, invoke_id, occurred_at, "sticky-guard-skip");

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending, false, false)
        .await
        .expect("recover dropped guard without persisted attempt row");

    assert_eq!(
        count_invocation_rows(&state.pool, invoke_id, occurred_at).await,
        0
    );
}

#[tokio::test]
pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan_skips_streaming_response_attempt_after_task_drop()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recover-streaming-response";
    let occurred_at = "2026-03-23 21:09:05";
    let pending = seed_pool_attempt_for_recovery(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard-streaming",
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
    )
    .await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), true, false)
        .await
        .expect("skip dropped streaming-response guard after task exit");

    assert_attempt_and_invocation_count(
        state.as_ref(),
        &pending,
        invoke_id,
        occurred_at,
        (
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
            None,
        ),
        0,
    )
    .await;
}

#[tokio::test]
pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan_recovers_attempt_after_invocation_finalized()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-skip-finalized-invocation";
    let occurred_at = "2026-03-23 21:09:55";
    let pending = seed_pool_attempt_for_recovery(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard-final",
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    insert_success_invocation(state.as_ref(), invoke_id, occurred_at).await;

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), false, false)
        .await
        .expect("recover dropped guard attempt after finalized invocation");

    let attempt = load_pool_attempt_state(&state.pool, &pending).await;
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

    assert_interrupted_pool_attempt(&attempt);
    assert_eq!(invocation.0, "success");
    assert_eq!(invocation.1, None);
}

#[tokio::test]
pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan_skips_post_first_byte_terminal_attempt()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-skip-post-first-byte";
    let occurred_at = "2026-03-23 21:10:05";
    let pending = seed_pool_attempt_for_recovery(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard-post-first-byte",
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), true, true)
        .await
        .expect("skip dropped guard recovery after first byte");

    assert_attempt_and_invocation_count(
        state.as_ref(),
        &pending,
        invoke_id,
        occurred_at,
        (
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
            None,
        ),
        0,
    )
    .await;
}

#[tokio::test]
pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan_recovers_post_first_byte_nonterminal_attempt()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recover-post-first-byte-nonterminal";
    let occurred_at = "2026-03-23 21:10:06";
    let pending = seed_pool_attempt_for_recovery(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard-post-first-byte-recover",
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await;
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    recover_guard_dropped_pool_early_phase_orphan(state.as_ref(), pending.clone(), true, false)
        .await
        .expect("recover dropped guard after first byte before terminal outcome");

    assert_attempt_and_invocation_count(
        state.as_ref(),
        &pending,
        invoke_id,
        occurred_at,
        (
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED,
            Some(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED),
        ),
        0,
    )
    .await;
}

#[tokio::test]
pub(crate) async fn recover_guard_dropped_pool_terminal_invocation_orphan_repairs_running_invocation_after_attempt_already_finalized()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-recover-invocation-after-final-attempt";
    let occurred_at = "2026-03-23 21:10:06";
    seed_running_proxy_invocation(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard-final-attempt",
    )
    .await;
    insert_running_invocation(state.as_ref(), invoke_id, occurred_at).await;

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
        PoolAttemptFinalization {
            finished_at: shanghai_now_string().as_str(),
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            http_status: Some(StatusCode::OK),
            downstream_http_status: None,
            failure_kind: None,
            error_message: None,
            downstream_error_message: None,
            connect_latency_ms: Some(10.0),
            first_byte_latency_ms: Some(20.0),
            stream_latency_ms: Some(30.0),
            upstream_request_id: Some("upstream-req-1"),
            compact_support_status: None,
            compact_support_reason: None,
        },
    )
    .await
    .expect("persist terminal attempt before dropped guard recovery");

    recover_guard_dropped_pool_terminal_invocation_orphan(
        state.as_ref(),
        InvocationRecoverySelector::from(&pending),
    )
    .await
    .expect("recover running invocation after attempt already finalized");

    assert_terminal_guard_repair(state.as_ref(), &pending, invoke_id, occurred_at, account_id)
        .await;
}

#[tokio::test]
pub(crate) async fn drop_guard_runtime_remove_does_not_tombstone_later_terminal_record() {
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
    let running_record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
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
    ));
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
pub(crate) async fn pool_invocation_cleanup_guard_recovers_running_invocation_during_retry_backoff_gap()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-request-drop-backoff-gap";
    let occurred_at = "2026-03-23 21:10:06";
    seed_running_proxy_invocation(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-request-drop-backoff",
    )
    .await;
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
        PoolAttemptFinalization {
            finished_at: shanghai_now_string().as_str(),
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            http_status: None,
            downstream_http_status: None,
            failure_kind: Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
            error_message: Some("failed to contact upstream: retryable backoff"),
            downstream_error_message: None,
            connect_latency_ms: Some(10.0),
            first_byte_latency_ms: None,
            stream_latency_ms: None,
            upstream_request_id: None,
            compact_support_status: None,
            compact_support_reason: None,
        },
    )
    .await
    .expect("persist retryable attempt before simulated backoff drop");

    {
        let _guard = PoolInvocationCleanupGuard::new(
            state.clone(),
            InvocationRecoverySelector::from(&pending),
            "request_drop_guard",
        );
    }

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert_attempt_and_invocation_count(
        state.as_ref(),
        &pending,
        invoke_id,
        occurred_at,
        (
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED,
            Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
        ),
        0,
    )
    .await;
}

#[tokio::test]
pub(crate) async fn recover_guard_dropped_pool_early_phase_orphan_rolls_back_attempt_when_invocation_recovery_fails()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "guard-atomic-recovery-failure";
    let occurred_at = "2026-03-23 21:10:07";
    seed_running_proxy_invocation(
        &state,
        account_id,
        invoke_id,
        occurred_at,
        "sticky-guard-atomic",
    )
    .await;
    insert_running_invocation(state.as_ref(), invoke_id, occurred_at).await;

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

    create_guard_recovery_failure_trigger(&state.pool).await;

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
    let invocation = load_invocation_status_and_failure(&state.pool, invoke_id, occurred_at).await;

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );
    assert_eq!(attempt.2, None);
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
}
