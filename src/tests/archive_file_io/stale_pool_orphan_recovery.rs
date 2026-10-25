use super::*;
use serde_json::json;

#[tokio::test]
async fn recover_proxy_invocations_with_selector_batches_handles_large_selector_sets() {
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

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_only_recovers_stale_early_rows() {
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

    let cases = [
        (
            "stale-early-phase",
            "2026-03-23 21:10:01",
            stale_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
            false,
            0.0,
        ),
        (
            "fresh-early-phase",
            "2026-03-23 21:10:02",
            fresh_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
            false,
            0.0,
        ),
        (
            "stale-early-phase-post-first-byte",
            "2026-03-23 21:10:25",
            stale_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
            true,
            120.0,
        ),
        (
            "stale-early-phase-attempt-first-byte-progress",
            "2026-03-23 21:10:26",
            stale_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
            true,
            0.0,
        ),
        (
            "stale-streaming-phase",
            "2026-03-23 21:10:03",
            stale_started.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
            false,
            120.0,
        ),
    ];

    for (invoke_id, occurred_at, started_at, phase, persist_attempt_first_byte_progress, ttfb_ms) in
        cases
    {
        let running_record = build_running_proxy_capture_record(
            invoke_id,
            occurred_at,
            ProxyCaptureTarget::Responses,
            &request_info,
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
            ttfb_ms,
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
            sticky_key: Some("sticky-sweeper".to_string()),
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
            started_at,
        )
        .await;
        advance_pool_upstream_request_attempt_phase(state.as_ref(), &pending, phase)
            .await
            .expect("advance attempt into target phase");
        if persist_attempt_first_byte_progress {
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
        SELECT COUNT(*)
        FROM codex_invocations
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

    let stale_post_first_byte_invocation_count = sqlx::query_scalar::<_, i64>(
        r#"
            SELECT COUNT(*)
            FROM codex_invocations
            WHERE invoke_id = 'stale-early-phase-post-first-byte'
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count stale post-first-byte invocation rows");
    assert_eq!(stale_post_first_byte_invocation_count, 0);

    let stale_attempt_first_byte_progress_attempt =
        sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
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
        .expect("load stale attempt-first-byte-progress attempt");
    assert_eq!(
        stale_attempt_first_byte_progress_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        stale_attempt_first_byte_progress_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(stale_attempt_first_byte_progress_attempt.2, Some(120.0));

    let stale_attempt_first_byte_progress_invocation_count = sqlx::query_scalar::<_, i64>(
        r#"
            SELECT COUNT(*)
            FROM codex_invocations
            WHERE invoke_id = 'stale-early-phase-attempt-first-byte-progress'
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count stale attempt-first-byte-progress invocation rows");
    assert_eq!(stale_attempt_first_byte_progress_invocation_count, 0);

    let fresh_attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = 'fresh-early-phase'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load fresh attempt");
    assert_eq!(
        fresh_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        fresh_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );

    let streaming_attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE invoke_id = 'stale-streaming-phase'
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load stale streaming attempt");
    assert_eq!(
        streaming_attempt.0,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING
    );
    assert_eq!(
        streaming_attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );

    let streaming_invocation_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE invoke_id = 'stale-streaming-phase'
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("count stale streaming invocation rows");
    assert_eq!(streaming_invocation_count, 0);
}

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_skips_active_live_attempts() {
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

    let running_record = build_running_proxy_capture_record(
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
        sticky_key: Some("sticky-active-live-attempt".to_string()),
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
    .expect("load active attempt after stale recovery");
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
    .expect("count active invocation rows after stale recovery");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST)
    );
    assert_eq!(invocation_count, 0);
}

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_rolls_back_attempts_when_invocation_recovery_fails()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "stale-sweeper-atomic-recovery-failure";
    let occurred_at = "2026-03-23 21:10:31";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(
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
        sticky_key: Some("sticky-sweeper-atomic".to_string()),
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

    sqlx::query(
        r#"
        CREATE TRIGGER fail_stale_sweeper_invocation_recovery
        BEFORE UPDATE ON codex_invocations
        WHEN OLD.invoke_id = 'stale-sweeper-atomic-recovery-failure'
        BEGIN
            SELECT RAISE(FAIL, 'simulated stale sweeper invocation recovery failure');
        END;
        "#,
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

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after failed stale sweeper");
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

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_recovers_stale_attempts_after_invocation_finished()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "stale-sweeper-finalized-invocation";
    let occurred_at = "2026-03-23 21:10:31";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.32"),
        Some("sticky-sweeper-finalized"),
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
        sticky_key: Some("sticky-sweeper-finalized".to_string()),
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
    .expect("finalize invocation before stale recovery");

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("recover stale attempt after invocation already finished");
    assert_eq!(
        outcome,
        PoolOrphanRecoveryOutcome {
            recovered_attempts: 1,
            recovered_invocations: 0,
        }
    );

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, phase, failure_kind
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after finalized-invocation stale recovery");
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
    .expect("load finalized invocation after stale recovery");
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
    .expect("load route state after finalized-invocation stale recovery");

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
    assert_eq!(route_state.0, None);
    assert_eq!(route_state.1, None);
    assert_eq!(route_state.2, 0);
}

#[tokio::test]
#[expect(
    clippy::await_holding_lock,
    reason = "Mock reservation logs intentionally stay locked until async assertions observe requests."
)]
async fn recover_stale_pool_early_phase_orphans_runtime_clears_pool_routing_reservation_and_records_route_failure()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let invoke_id = "proxy-5252-1775776407001";
    let occurred_at = "2026-03-23 21:10:32";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.34"),
        Some("sticky-sweeper-reservation"),
        Some("pck-sweeper-reservation"),
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
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-reservation".to_string()),
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
    let reservation_key = build_pool_routing_reservation_key(5252);
    reserve_test_pool_routing_account(&state, &reservation_key, account_id).await;

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("recover stale early-phase orphan and clean up route state");
    assert_eq!(
        outcome,
        PoolOrphanRecoveryOutcome {
            recovered_attempts: 1,
            recovered_invocations: 0,
        }
    );

    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    assert!(
        !reservations.contains_key(&reservation_key),
        "runtime sweeper should clear the in-memory reservation",
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
    .expect("load account route state after stale sweeper");

    assert!(route_state.0.is_none());
    assert_eq!(route_state.2, 0);
    assert!(route_state.1.is_none());
}

#[tokio::test]
async fn recover_stale_pool_early_phase_orphans_runtime_records_route_failures_for_every_recovered_attempt()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let first_account_id =
        insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let second_account_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    let invoke_id = "stale-multi-attempt-route-failures";
    let occurred_at = "2026-03-23 21:10:33";
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.35"),
        Some("sticky-sweeper-multi-attempt"),
        Some("pck-sweeper-multi-attempt"),
        true,
        Some(first_account_id),
        Some("Primary"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(2),
        Some(2),
        None,
        None,
        10.0,
        2.0,
        5.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let first_pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &PoolUpstreamAttemptTraceContext {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some("sticky-sweeper-multi-attempt".to_string()),
            requester_ip: Some("192.168.31.6".to_string()),
            upstream_base_url_host: None,
            request_model: None,
        },
        first_account_id,
        "route-primary",
        1,
        1,
        1,
        &stale_started,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &first_pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance first attempt into sending-request");

    let second_pending = begin_pool_upstream_request_attempt(
        &state.pool,
        &PoolUpstreamAttemptTraceContext {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some("sticky-sweeper-multi-attempt".to_string()),
            requester_ip: Some("192.168.31.6".to_string()),
            upstream_base_url_host: None,
            request_model: None,
        },
        second_account_id,
        "route-secondary",
        2,
        2,
        1,
        &stale_started,
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &second_pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
    )
    .await
    .expect("advance second attempt into sending-request");

    let outcome = recover_stale_pool_early_phase_orphans_runtime(state.as_ref())
        .await
        .expect("recover stale attempts across multiple accounts");
    assert_eq!(
        outcome,
        PoolOrphanRecoveryOutcome {
            recovered_attempts: 2,
            recovered_invocations: 0,
        }
    );

    let first_route_state = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
        r#"
        SELECT last_route_failure_kind, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(first_account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load first account route state after stale recovery");
    let second_route_state = sqlx::query_as::<_, (Option<String>, Option<String>, i64)>(
        r#"
        SELECT last_route_failure_kind, last_error, consecutive_route_failures
        FROM pool_upstream_accounts
        WHERE id = ?1
        "#,
    )
    .bind(second_account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load second account route state after stale recovery");

    for route_state in [&first_route_state, &second_route_state] {
        assert!(route_state.0.is_none());
        assert_eq!(route_state.2, 0);
        assert!(route_state.1.is_none());
    }
}

#[tokio::test]
async fn recover_stale_pool_upstream_request_attempt_candidates_rechecks_phase_before_update() {
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
    let invoke_id = "stale-race-phase-recheck";
    let occurred_at = "2026-03-23 21:10:35";
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.29"),
        Some("sticky-sweeper-race"),
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
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-race".to_string()),
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

    let attempt_id = pending.attempt_id.expect("pending attempt id");
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE,
    )
    .await
    .expect("move attempt out of early phase before guarded update");

    SqliteBatchWriter::flush_for_test(
        &state.pool,
        vec![SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
            attempt_id,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE.to_string()),
            connect_latency_ms: None,
            first_byte_latency_ms: None,
            compact_support_status: None,
            compact_support_reason: None,
            ..Default::default()
        })],
    )
    .await;
    let finished_at = shanghai_now_string();
    let recovered = recover_stale_pool_upstream_request_attempt_candidates(
        &state.pool,
        &[attempt_id],
        finished_at.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
    )
    .await
    .expect("guarded candidate update should re-check current phase");
    assert!(recovered.is_empty());

    let attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after guarded stale update");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE)
    );
}

#[tokio::test]
async fn recover_stale_pool_upstream_request_attempt_candidates_rechecks_attempt_first_byte_progress_before_update()
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
    let invoke_id = "stale-race-attempt-first-byte-recheck";
    let occurred_at = "2026-03-23 21:10:36";
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.30"),
        Some("sticky-sweeper-race-attempt-progress"),
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
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("persist running invocation");

    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: Some("sticky-sweeper-race-attempt-progress".to_string()),
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
        stale_started.as_str(),
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await
    .expect("advance attempt into waiting-first-byte");

    let attempt_id = pending.attempt_id.expect("pending attempt id");
    SqliteBatchWriter::flush_for_test(
        &state.pool,
        vec![SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
            attempt_id,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE.to_string()),
            connect_latency_ms: None,
            first_byte_latency_ms: None,
            compact_support_status: None,
            compact_support_reason: None,
            ..Default::default()
        })],
    )
    .await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET first_byte_latency_ms = 120.0
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .execute(&state.pool)
    .await
    .expect("persist attempt first-byte progress before guarded update");

    let finished_at = shanghai_now_string();
    let recovered = recover_stale_pool_upstream_request_attempt_candidates(
        &state.pool,
        &[attempt_id],
        finished_at.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
    )
    .await
    .expect("guarded candidate update should re-check attempt first-byte progress");
    assert!(recovered.is_empty());

    let attempt = sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
        r#"
        SELECT status, phase, first_byte_latency_ms
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after guarded stale update");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(attempt.2, Some(120.0));
}

#[tokio::test]
async fn recover_stale_pool_upstream_request_attempt_candidates_rechecks_invocation_first_byte_progress_before_update()
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
    let invoke_id = "stale-race-invocation-first-byte-recheck";
    let occurred_at = "2026-03-23 21:10:37";
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.31"),
        Some("sticky-sweeper-race-invocation-progress"),
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
        sticky_key: Some("sticky-sweeper-race-invocation-progress".to_string()),
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
        stale_started.as_str(),
    )
    .await;
    advance_pool_upstream_request_attempt_phase(
        state.as_ref(),
        &pending,
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE,
    )
    .await
    .expect("advance attempt into waiting-first-byte");

    let attempt_id = pending.attempt_id.expect("pending attempt id");
    SqliteBatchWriter::flush_for_test(
        &state.pool,
        vec![SqliteBatchWrite::AttemptProgress(BatchedAttemptProgress {
            attempt_id,
            pending_status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
            phase: Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE.to_string()),
            connect_latency_ms: None,
            first_byte_latency_ms: None,
            compact_support_status: None,
            compact_support_reason: None,
            ..Default::default()
        })],
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
            payload,
            t_upstream_ttfb_ms
        )
        VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, 120.0)
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
    .expect("insert legacy invocation first-byte progress before guarded update");

    let finished_at = shanghai_now_string();
    let recovered = recover_stale_pool_upstream_request_attempt_candidates(
        &state.pool,
        &[attempt_id],
        finished_at.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
    )
    .await
    .expect("guarded candidate update should re-check invocation first-byte progress");
    assert!(recovered.is_empty());

    let attempt = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, phase
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
    )
    .bind(attempt_id)
    .fetch_one(&state.pool)
    .await
    .expect("load attempt after guarded stale update");
    let invocation = sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
        r#"
        SELECT status, failure_kind, t_upstream_ttfb_ms
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
    .expect("load invocation after guarded stale update");

    assert_eq!(attempt.0, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        attempt.1.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE)
    );
    assert_eq!(invocation.0, INVOCATION_STATUS_RUNNING);
    assert_eq!(invocation.1, None);
    assert_eq!(invocation.2, Some(120.0));
}

#[tokio::test]
async fn recover_stale_pool_upstream_request_attempt_candidates_batches_large_candidate_sets() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let selector_count = POOL_ATTEMPT_RECOVERY_SELECTOR_BATCH_SIZE + 125;
    let stale_started = format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local() - ChronoDuration::minutes(10),
    );
    let cutoff = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut candidate_ids = Vec::with_capacity(selector_count);

    for index in 0..selector_count {
        let trace = PoolUpstreamAttemptTraceContext {
            invoke_id: format!("stale-candidate-batch-{index}"),
            occurred_at: format!("2026-03-23 22:{:02}:{:02}", (index / 60) % 60, index % 60),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some(format!("sticky-candidate-batch-{index}")),
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
            &stale_started,
        )
        .await;
        advance_pool_upstream_request_attempt_phase(
            state.as_ref(),
            &pending,
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST,
        )
        .await
        .expect("advance candidate attempt into sending-request");
        candidate_ids.push(pending.attempt_id.expect("pending attempt id"));
    }

    let finished_at = shanghai_now_string();
    let recovered = recover_stale_pool_upstream_request_attempt_candidates(
        &state.pool,
        &candidate_ids,
        finished_at.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
        cutoff.as_str(),
    )
    .await
    .expect("recover stale attempt candidates across multiple chunks");

    assert_eq!(recovered.len(), selector_count);

    let recovered_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE invoke_id LIKE 'stale-candidate-batch-%'
          AND status = ?1
          AND failure_kind = ?2
        "#,
    )
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
    .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    .fetch_one(&state.pool)
    .await
    .expect("count recovered stale attempt rows");

    assert_eq!(recovered_count as usize, selector_count);
}

#[tokio::test]
async fn send_pool_request_with_failover_returns_owner_unavailable_for_encrypted_session_lock() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let owner_account_id = insert_test_pool_api_key_account(
        &state,
        "Encrypted Owner Unavailable",
        "upstream-owner-unavailable",
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_concurrency_limit = 1 WHERE id = ?1")
        .bind(owner_account_id)
        .execute(&state.pool)
        .await
        .expect("set encrypted owner account concurrency limit");
    let now_iso = format_utc_iso(Utc::now());
    insert_test_pool_limit_sample(&state, owner_account_id, Some(20.0), Some(20.0)).await;
    upsert_sticky_route(
        &state.pool,
        "encrypted-owner-unavailable-active-key",
        owner_account_id,
        &now_iso,
    )
    .await
    .expect("seed active sticky route for owner account");
    upsert_sticky_route(
        &state.pool,
        "encrypted-owner-unavailable-request-key",
        owner_account_id,
        &now_iso,
    )
    .await
    .expect("seed request sticky route for owner account");
    upsert_prompt_cache_encrypted_session_owner(
        &state.pool,
        "encrypted-owner-unavailable-prompt-cache-key",
        owner_account_id,
    )
    .await
    .expect("persist encrypted session owner");

    let err = send_pool_request_with_failover(
        state.clone(),
        710001,
        Method::POST,
        &"/v1/responses".parse().expect("valid uri"),
        &HeaderMap::from_iter([(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]),
        Some(PoolReplayBodySnapshot::Memory(Bytes::from(
            serde_json::to_vec(&json!({
                "model": "gpt-5.4",
                "input": [{
                    "type": "encrypted_content",
                    "encrypted_content": "opaque-owner-bound-content"
                }]
            }))
            .expect("serialize encrypted owner request body"),
        ))),
        Duration::from_secs(5),
        Some(PoolUpstreamAttemptTraceContext {
            invoke_id: "proxy-710001-encrypted-owner-unavailable".to_string(),
            occurred_at: shanghai_now_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: Some("encrypted-owner-unavailable-request-key".to_string()),
            requester_ip: None,
            upstream_base_url_host: None,
            request_model: None,
        }),
        Some(PoolAttemptRuntimeSnapshotContext {
            capture_target: ProxyCaptureTarget::Responses,
            request_info: RequestCaptureInfo {
                model: Some("gpt-5.4".to_string()),
                contains_encrypted_content: true,
                ..RequestCaptureInfo::default()
            },
            hosted_image_intent: None,
            prompt_cache_key: Some("encrypted-owner-unavailable-prompt-cache-key".to_string()),
            owner_auto_guard_active: true,
            t_req_read_ms: 1.0,
            t_req_parse_ms: 1.0,
        }),
        Some("encrypted-owner-unavailable-request-key"),
        None,
        PoolFailoverProgress::default(),
        1,
    )
    .await
    .expect_err("encrypted owner lock should fail before rerouting");

    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        err.message,
        "encrypted session owner routing is constrained to upstream account Encrypted Owner Unavailable but that account is currently unavailable"
    );
    assert_eq!(
        err.failure_kind,
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED
    );
    assert_eq!(err.upstream_error_code.as_deref(), None);
    assert_eq!(err.upstream_error_message.as_deref(), None);
    assert_eq!(
        err.blocked_binding,
        Some(BlockedBindingDiagnostic {
            constraint_source: BlockedBindingConstraintSource::EncryptedSessionOwner,
            upstream_account_id: owner_account_id,
            upstream_account_label: "Encrypted Owner Unavailable".to_string(),
            prompt_cache_key: Some("encrypted-owner-unavailable-prompt-cache-key".to_string()),
            recovery_action: BlockedBindingRecoveryAction::ClearAndResetAffinity,
        })
    );
    assert_eq!(
        err.account.as_ref().map(|account| account.account_id),
        None,
        "owner-unavailable guard should stop before picking an alternate account",
    );
}
