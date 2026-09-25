use super::*;

#[tokio::test]
async fn persist_and_broadcast_proxy_capture_runtime_snapshot_uses_memory_overlay_without_sync_db_write()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        prompt_cache_key: Some("pck-running".to_string()),
        requested_service_tier: Some("priority".to_string()),
        reasoning_effort: Some("high".to_string()),
        compaction_request_kind: Some(CompactionKind::RemoteV2),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "invoke-running";
    let occurred_at_owned = format_naive(Utc::now().naive_utc());
    let occurred_at = occurred_at_owned.as_str();
    let record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-running"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-01"),
        Some(3),
        Some(2),
        None,
        Some("gzip"),
        22.0,
        4.0,
        330.0,
        120.0,
    );

    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, record)
        .await
        .expect("runtime snapshot should store in memory and broadcast");
    state.proxy_runtime_invocations.backdate_for_test(
        invoke_id,
        occurred_at,
        std::time::Duration::from_secs(2 * 60 * 60),
    );

    let payload = rx
        .recv()
        .await
        .expect("runtime snapshot payload should arrive");
    let broadcast_record = match payload {
        BroadcastPayload::Records { records } => {
            assert_eq!(records.len(), 1);
            records.into_iter().next().expect("single running record")
        }
        other => panic!("expected records payload, got {other:?}"),
    };

    assert_eq!(
        broadcast_record.id, 0,
        "running snapshot should broadcast from memory before the bounded sqlite flush"
    );
    assert_eq!(broadcast_record.status.as_deref(), Some("running"));
    assert_eq!(broadcast_record.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(broadcast_record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(
        broadcast_record.compaction_request_kind.as_deref(),
        Some("remote_v2")
    );
    assert_eq!(broadcast_record.compaction_response_kind, None);
    assert_eq!(
        broadcast_record.proxy_display_name.as_deref(),
        Some("jp-relay-01")
    );
    assert_eq!(broadcast_record.upstream_account_id, Some(17));
    assert_eq!(
        broadcast_record.upstream_account_name.as_deref(),
        Some("pool-account-17")
    );
    assert_eq!(
        broadcast_record.response_content_encoding.as_deref(),
        Some("gzip")
    );
    assert_eq!(broadcast_record.pool_attempt_count, Some(3));
    assert_eq!(broadcast_record.pool_distinct_account_count, Some(2));
    assert_eq!(broadcast_record.pool_attempt_terminal_reason, None);
    assert_eq!(
        broadcast_record.prompt_cache_key.as_deref(),
        Some("pck-running")
    );
    assert_eq!(broadcast_record.billing_service_tier, None);
    assert_eq!(broadcast_record.t_total_ms, None);
    assert_eq!(broadcast_record.t_req_read_ms, Some(22.0));
    assert_eq!(broadcast_record.t_req_parse_ms, Some(4.0));
    assert_eq!(broadcast_record.t_upstream_connect_ms, Some(330.0));
    assert_eq!(broadcast_record.t_upstream_ttfb_ms, Some(120.0));

    let persisted_running_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count running db rows before flush");
    assert_eq!(
        persisted_running_count, 0,
        "runtime snapshot should not synchronously create a db placeholder"
    );

    let Json(response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("running invocation should be visible through memory overlay");

    assert_eq!(response.total, 1);
    assert_eq!(response.records.len(), 1);
    assert_eq!(response.records[0].id, 0);
    assert_eq!(response.records[0].status.as_deref(), Some("running"));
    assert_eq!(
        response.records[0].compaction_request_kind.as_deref(),
        Some("remote_v2")
    );
    assert_eq!(response.records[0].compaction_response_kind, None);
    assert_eq!(response.records[0].pool_attempt_count, Some(3));
    assert_eq!(response.records[0].pool_distinct_account_count, Some(2));
    assert_eq!(response.records[0].pool_attempt_terminal_reason, None);

    let Json(internal_scope_response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            upstream_scope: Some("internal".to_string()),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("pool running invocation should match internal memory overlay scope");
    assert_eq!(internal_scope_response.total, 1);
    assert_eq!(internal_scope_response.records.len(), 1);

    let Json(external_scope_response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            upstream_scope: Some("external".to_string()),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("pool running invocation should not match external memory overlay scope");
    assert_eq!(external_scope_response.total, 0);
    assert!(external_scope_response.records.is_empty());

    let Json(sticky_response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            sticky_key: Some("pck-running".to_string()),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("runtime sticky-key filter should use sticky fallback semantics");
    assert_eq!(sticky_response.total, 1);
    assert_eq!(sticky_response.records.len(), 1);

    let Json(max_tokens_response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            max_total_tokens: Some(0),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("runtime maxTotalTokens filter should match db null semantics");
    assert_eq!(max_tokens_response.total, 0);
    assert!(max_tokens_response.records.is_empty());

    let Json(max_ms_response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            max_total_ms: Some(0.0),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("runtime maxTotalMs filter should match db null semantics");
    assert_eq!(max_ms_response.total, 0);
    assert!(max_ms_response.records.is_empty());

    for index in 0..60 {
        seed_success_invocation_for_records_page(
            state.as_ref(),
            &format!("page-db-{index:02}"),
            &format!("2026-03-17 18:12:{:02}", index % 60),
        )
        .await;
    }
    let Json(second_page) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            page: Some(2),
            page_size: Some(50),
            sort_by: Some("occurredAt".to_string()),
            sort_order: Some("desc".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("runtime overlay should preserve current records pagination");
    assert_eq!(second_page.total, 61);
    assert_eq!(second_page.records.len(), 11);
    assert!(
        second_page
            .records
            .iter()
            .any(|record| record.invoke_id == "page-db-10"),
        "DB row at the page boundary should remain reachable while runtime overlay is present"
    );

    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    let persisted_running_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count running db rows after flush");
    assert_eq!(
        persisted_running_count, 0,
        "runtime snapshots should remain memory-only even after sqlite batch flush"
    );

    let repeat_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-running"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-01"),
        Some(3),
        Some(2),
        None,
        Some("gzip"),
        24.0,
        5.0,
        340.0,
        130.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, repeat_record)
        .await
        .expect("repeated runtime snapshot should only refresh memory");
    let repeated_running_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count running db rows after repeated runtime snapshot");
    assert_eq!(
        repeated_running_count, 0,
        "repeated runtime snapshots should never enqueue DB placeholder writes"
    );

    let switched_account_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-running"),
        true,
        Some(23),
        Some("pool-account-23"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-02"),
        Some(4),
        Some(3),
        None,
        Some("gzip"),
        26.0,
        6.0,
        350.0,
        140.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, switched_account_record)
        .await
        .expect("account-switched runtime snapshot should only refresh memory");

    {
        let Json(old_account_records) = list_invocations(
            State(state.clone()),
            Query(ListQuery {
                request_id: Some(invoke_id.to_string()),
                upstream_account_id: Some(17),
                page_size: Some(1),
                ..Default::default()
            }),
        )
        .await
        .expect("records should hide stale DB running placeholder after runtime account switch");
        assert_eq!(old_account_records.total, 0);
        assert!(old_account_records.records.is_empty());

        let Json(new_account_records) = list_invocations(
            State(state.clone()),
            Query(ListQuery {
                request_id: Some(invoke_id.to_string()),
                upstream_account_id: Some(23),
                page_size: Some(1),
                ..Default::default()
            }),
        )
        .await
        .expect("records should show current memory runtime account after switch");
        assert_eq!(new_account_records.total, 1);
        assert_eq!(new_account_records.records.len(), 1);
        assert_eq!(new_account_records.records[0].id, 0);
        assert_eq!(new_account_records.records[0].upstream_account_id, Some(23));

        let Json(summary) = fetch_invocation_summary(
            State(state.clone()),
            Query(ListQuery {
                request_id: Some(invoke_id.to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect("current summary should overlay running memory count");
        assert_eq!(summary.total_count, 1);
        assert_eq!(summary.success_count, 0);
        assert_eq!(summary.failure_count, 0);

        let Json(old_account_stats_summary) = fetch_summary_from_memory_snapshot(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("today".to_string()),
                limit: None,
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: Some(17),
            }),
        )
        .await
        .expect("stats summary should remove stale DB account count after runtime switch");
        assert_eq!(
            old_account_stats_summary.in_progress_conversation_count,
            Some(0)
        );
        assert_eq!(
            old_account_stats_summary.in_progress_retry_conversation_count,
            Some(0)
        );

        let Json(new_account_stats_summary) = fetch_summary_from_memory_snapshot(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some("today".to_string()),
                limit: None,
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: Some(23),
            }),
        )
        .await
        .expect("stats summary should move running memory count to the current account");
        assert_eq!(
            new_account_stats_summary.in_progress_conversation_count,
            Some(1)
        );
        assert_eq!(
            new_account_stats_summary.in_progress_retry_conversation_count,
            Some(1)
        );
    }

    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, occurred_at),
    )
    .await
    .expect("http terminal record should enqueue without waiting for sqlite");

    let terminal_before_flush = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count terminal db row before flush");
    assert_eq!(
        terminal_before_flush, 0,
        "terminal record should not synchronously block on sqlite persistence"
    );
    let Json(summary_before_terminal_flush) = fetch_invocation_summary(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("current summary should overlay queued terminal record before sqlite flush");
    assert_eq!(summary_before_terminal_flush.total_count, 1);
    assert_eq!(summary_before_terminal_flush.success_count, 1);
    assert_eq!(summary_before_terminal_flush.failure_count, 0);
    assert_eq!(summary_before_terminal_flush.total_tokens, 15);
    assert_eq!(summary_before_terminal_flush.token.total_tokens, 15);
    assert_eq!(summary_before_terminal_flush.token.cache_input_tokens, 2);
    assert_eq!(
        summary_before_terminal_flush.token.avg_tokens_per_request,
        15.0
    );
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .any(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at),
        "terminal overlay should remain until deferred derived writes flush"
    );

    let Json(after_terminal) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("terminal invocation should be loaded from db after write-controller flush");
    assert_eq!(after_terminal.total, 1);
    assert_eq!(after_terminal.records.len(), 1);
    assert!(after_terminal.records[0].id > 0);
    assert_eq!(after_terminal.records[0].status.as_deref(), Some("success"));
    assert!(
        after_terminal.records[0]
            .t_persist_ms
            .is_some_and(|value| value >= 0.0),
        "raw terminal persistence latency should remain recorded after async flush"
    );

    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .all(|record| record.invoke_id != invoke_id || record.occurred_at != occurred_at),
        "terminal overlay should be removed after deferred derived writes flush"
    );
}

#[tokio::test]
async fn admitted_proxy_capture_snapshot_is_visible_before_body_parse_and_later_enriched() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "invoke-admitted-running";
    let occurred_at = "2026-03-17 18:14:34";

    let admitted_record = build_admitted_proxy_capture_runtime_snapshot(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        Some("203.0.113.42"),
        Some("sticky-from-header"),
        Some("pck-from-header"),
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, admitted_record)
        .await
        .expect("admitted snapshot should store in memory and broadcast");

    let admitted_broadcast = recv_next_records(&mut rx)
        .await
        .into_iter()
        .next()
        .expect("single admitted record");
    assert_eq!(admitted_broadcast.id, 0);
    assert_eq!(admitted_broadcast.status.as_deref(), Some("running"));
    assert_eq!(
        admitted_broadcast.endpoint.as_deref(),
        Some("/v1/responses")
    );
    assert_eq!(
        admitted_broadcast.requester_ip.as_deref(),
        Some("203.0.113.42")
    );
    assert_eq!(
        admitted_broadcast.prompt_cache_key.as_deref(),
        Some("pck-from-header")
    );
    assert_eq!(
        admitted_broadcast.sticky_key.as_deref(),
        Some("sticky-from-header")
    );
    assert_eq!(admitted_broadcast.model, None);
    assert_eq!(admitted_broadcast.upstream_account_id, None);

    let enriched_request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        prompt_cache_key: Some("pck-from-body".to_string()),
        sticky_key: Some("sticky-from-body".to_string()),
        prompt_cache_key_attribution_source: Some("request".to_string()),
        contains_encrypted_content: true,
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let enriched_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &enriched_request_info,
        Some("203.0.113.42"),
        Some("sticky-from-body"),
        Some("pck-from-body"),
        true,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        31.0,
        7.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, enriched_record)
        .await
        .expect("body-parsed snapshot should enrich the same runtime row");

    let enriched_broadcast = recv_next_records(&mut rx)
        .await
        .into_iter()
        .next()
        .expect("single enriched record");
    assert_eq!(enriched_broadcast.id, 0);
    assert_eq!(enriched_broadcast.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(
        enriched_broadcast.prompt_cache_key.as_deref(),
        Some("pck-from-body")
    );
    assert_eq!(
        enriched_broadcast.sticky_key.as_deref(),
        Some("sticky-from-body")
    );

    let runtime_rows = state.proxy_runtime_invocations.snapshot();
    let matching_rows: Vec<_> = runtime_rows
        .iter()
        .filter(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at)
        .collect();
    assert_eq!(
        matching_rows.len(),
        1,
        "admitted and body-parsed snapshots should share one runtime key"
    );
    assert_eq!(matching_rows[0].model.as_deref(), Some("gpt-5.4"));

    let persisted_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count db rows after admitted runtime snapshots");
    assert_eq!(
        persisted_count, 0,
        "admitted runtime snapshots must not restore sqlite running writes"
    );
}

#[tokio::test]
async fn admitted_proxy_capture_snapshot_can_be_cleared_before_terminal_on_early_error() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "invoke-admitted-cleared";
    let occurred_at = "2026-03-17 18:15:34";

    let admitted_record = build_admitted_proxy_capture_runtime_snapshot(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        Some("203.0.113.43"),
        Some("sticky-from-header"),
        Some("pck-from-header"),
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, admitted_record)
        .await
        .expect("admitted snapshot should store in memory");

    assert_eq!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .into_iter()
            .filter(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at)
            .count(),
        1,
        "admitted runtime snapshot should be visible before early error cleanup"
    );

    remove_proxy_runtime_snapshot_by_key(
        &state,
        invoke_id,
        occurred_at,
        "test_early_routing_error",
    );

    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .into_iter()
            .all(|record| record.invoke_id != invoke_id || record.occurred_at != occurred_at),
        "early pre-attempt failures must clear admitted runtime snapshots"
    );
    let persisted_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count db rows after early cleanup");
    assert_eq!(
        persisted_count, 0,
        "early cleanup must not restore sqlite running writes"
    );
}

#[tokio::test]
async fn admitted_proxy_capture_snapshot_is_cleared_when_cleanup_guard_drops_before_attempt() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "invoke-admitted-drop-guard";
    let occurred_at = "2026-03-17 18:15:55";
    let guard = PoolInvocationCleanupGuard::new(
        state.clone(),
        InvocationRecoverySelector::new(invoke_id.to_string(), occurred_at.to_string()),
        "test_admitted_drop_guard",
    );

    let admitted_record = build_admitted_proxy_capture_runtime_snapshot(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        Some("203.0.113.45"),
        Some("sticky-from-header"),
        Some("pck-from-header"),
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, admitted_record)
        .await
        .expect("admitted snapshot should store in memory");
    let payload = rx
        .recv()
        .await
        .expect("admitted runtime snapshot should arrive");
    match payload {
        BroadcastPayload::Records { records } => {
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].status.as_deref(), Some("running"));
        }
        other => panic!("expected admitted records payload, got {other:?}"),
    }

    assert_eq!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .into_iter()
            .filter(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at)
            .count(),
        1,
        "admitted runtime snapshot should be visible before the cleanup guard drops"
    );

    drop(guard);
    for _ in 0..50 {
        let terminal_record = state
            .proxy_runtime_invocations
            .snapshot()
            .into_iter()
            .find(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at);
        if terminal_record
            .as_ref()
            .is_some_and(|record| record.status.as_deref() == Some(INVOCATION_STATUS_INTERRUPTED))
        {
            let terminal_broadcast = recv_next_records(&mut rx)
                .await
                .into_iter()
                .next()
                .expect("single terminal record");
            assert_eq!(terminal_broadcast.invoke_id, invoke_id);
            assert_eq!(terminal_broadcast.occurred_at, occurred_at);
            assert_eq!(
                terminal_broadcast.status.as_deref(),
                Some(INVOCATION_STATUS_INTERRUPTED)
            );
            assert_eq!(
                terminal_broadcast.failure_kind.as_deref(),
                Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
            );
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    panic!("drop guard should terminalize pre-attempt admitted runtime snapshot without a DB row");
}

#[tokio::test]
async fn admitted_proxy_capture_snapshot_is_terminalized_on_pre_attempt_error() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "invoke-admitted-terminalized";
    let occurred_at = "2026-03-17 18:16:34";

    let admitted_record = build_admitted_proxy_capture_runtime_snapshot(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        Some("203.0.113.44"),
        Some("sticky-from-header"),
        Some("pck-from-header"),
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, admitted_record)
        .await
        .expect("admitted snapshot should store in memory and broadcast");

    let admitted_broadcast = recv_next_records(&mut rx)
        .await
        .into_iter()
        .next()
        .expect("single admitted record");
    assert_eq!(admitted_broadcast.status.as_deref(), Some("running"));

    let mut terminal_record = test_proxy_capture_record(invoke_id, occurred_at);
    terminal_record.status = "http_502".to_string();
    terminal_record.error_message = Some(format!(
        "[{}] failed to resolve prompt cache conversation binding",
        PROXY_FAILURE_POOL_ROUTING_BLOCKED
    ));
    terminal_record.failure_kind = Some(PROXY_FAILURE_POOL_ROUTING_BLOCKED.to_string());
    persist_and_broadcast_proxy_capture(state.as_ref(), Instant::now(), terminal_record)
        .await
        .expect("pre-attempt terminal record should broadcast without waiting for sqlite");

    let terminal_broadcast = recv_next_records(&mut rx)
        .await
        .into_iter()
        .next()
        .expect("single terminal record");
    assert_eq!(terminal_broadcast.invoke_id, invoke_id);
    assert_eq!(terminal_broadcast.occurred_at, occurred_at);
    assert_eq!(terminal_broadcast.status.as_deref(), Some("http_502"));
    assert_eq!(
        terminal_broadcast.failure_kind.as_deref(),
        Some(PROXY_FAILURE_POOL_ROUTING_BLOCKED)
    );

    let Json(records_response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            page_size: Some(10),
            ..Default::default()
        }),
    )
    .await
    .expect("terminal overlay should replace admitted running row");
    assert_eq!(records_response.total, 1);
    assert_eq!(
        records_response.records[0].status.as_deref(),
        Some("http_502")
    );
}

#[tokio::test]
async fn account_timeseries_replaces_stale_db_runtime_placeholder_after_account_switch() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        prompt_cache_key: Some("pck-timeseries-account-switch".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "invoke-timeseries-account-switch";
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let initial_record = build_running_proxy_capture_record(
        invoke_id,
        &occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-timeseries-account-switch"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-01"),
        Some(1),
        Some(1),
        None,
        Some("gzip"),
        12.0,
        2.0,
        40.0,
        80.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, initial_record)
        .await
        .expect("initial runtime snapshot should enter memory");

    let updated_same_account_record = build_running_proxy_capture_record(
        invoke_id,
        &occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-timeseries-account-switch"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-01"),
        Some(1),
        Some(1),
        None,
        Some("gzip"),
        14.0,
        4.0,
        60.0,
        100.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, updated_same_account_record)
        .await
        .expect("same-account runtime update should refresh memory only");

    let Json(updated_account_timeseries) = fetch_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(17),
        }),
    )
    .await
    .expect("updated account timeseries should load");
    let updated_point = updated_account_timeseries
        .points
        .iter()
        .find(|point| point.in_flight_count == 1)
        .expect("updated in-flight point should be present");
    assert_eq!(
        updated_point.first_response_byte_total_avg_ms,
        Some(178.0),
        "same-key memory runtime update should replace stale DB placeholder timing"
    );

    let switched_record = build_running_proxy_capture_record(
        invoke_id,
        &occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-timeseries-account-switch"),
        true,
        Some(23),
        Some("pool-account-23"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-02"),
        Some(2),
        Some(2),
        None,
        Some("gzip"),
        13.0,
        3.0,
        50.0,
        90.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, switched_record)
        .await
        .expect("account-switched runtime snapshot should refresh memory only");

    let Json(old_account_timeseries) = fetch_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(17),
        }),
    )
    .await
    .expect("old account timeseries should load");
    let old_in_flight: i64 = old_account_timeseries
        .points
        .iter()
        .map(|point| point.in_flight_count)
        .sum();
    assert_eq!(
        old_in_flight, 0,
        "memory runtime account should subtract stale DB running placeholder from the old account"
    );

    let Json(new_account_timeseries) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(23),
        }),
    )
    .await
    .expect("new account timeseries should load");
    let new_in_flight: i64 = new_account_timeseries
        .points
        .iter()
        .map(|point| point.in_flight_count)
        .sum();
    assert_eq!(
        new_in_flight, 1,
        "memory runtime account should be counted on the current account"
    );
}

#[tokio::test]
async fn terminal_db_row_wins_over_stale_memory_runtime_overlay() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        prompt_cache_key: Some("pck-terminal-wins".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "invoke-terminal-db-wins";
    let occurred_at = "2026-03-17 18:13:37";
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-terminal-wins"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-01"),
        Some(1),
        Some(1),
        None,
        None,
        12.0,
        3.0,
        99.0,
        120.0,
    );

    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("runtime snapshot should enter memory");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost,
            status,
            raw_response,
            detail_level
        )
        VALUES (?1, ?2, 'proxy', 'gpt-5.4', 1, 2, 0, 42, 0.01, 'success', '', 'full')
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("simulate terminal DB write before memory cleanup");

    let Json(records_response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            page_size: Some(10),
            ..Default::default()
        }),
    )
    .await
    .expect("terminal DB row should suppress stale memory running row");
    assert_eq!(records_response.total, 1);
    assert_eq!(records_response.records.len(), 1);
    assert!(records_response.records[0].id > 0);
    assert_eq!(
        records_response.records[0].status.as_deref(),
        Some("success")
    );

    let Json(summary_response) = fetch_invocation_summary(
        State(state),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("summary should not double-count stale memory running row over terminal DB fact");
    assert_eq!(summary_response.total_count, 1);
    assert_eq!(summary_response.success_count, 1);
}

#[tokio::test]
async fn queued_terminal_overlay_preserves_total_when_db_running_placeholder_exists() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "invoke-terminal-overlay-over-running-db";
    let occurred_at = "2026-03-17 18:13:39";
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost,
            status,
            raw_response,
            detail_level
        )
        VALUES (?1, ?2, 'proxy', 'gpt-5.4', 0, 0, 0, 0, 0.0, 'running', '', 'full')
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("seed running DB placeholder");

    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, occurred_at),
    )
    .await
    .expect("terminal record should enqueue without sqlite flush");

    let Json(records_response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            page_size: Some(10),
            ..Default::default()
        }),
    )
    .await
    .expect("queued terminal overlay should replace DB running row");
    assert_eq!(records_response.total, 1);
    assert_eq!(records_response.records.len(), 1);
    assert_eq!(records_response.records[0].id, 0);
    assert_eq!(
        records_response.records[0].status.as_deref(),
        Some("success")
    );

    let Json(summary_response) = fetch_invocation_summary(
        State(state),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("current summary should count queued terminal overlay over DB running row");
    assert_eq!(summary_response.total_count, 1);
    assert_eq!(summary_response.success_count, 1);
    assert_eq!(summary_response.total_tokens, 15);
}

#[tokio::test]
async fn clearing_terminal_tombstone_allows_enqueue_retry_without_running_regression() {
    let store = ProxyRuntimeInvocationStore::default();
    let invoke_id = "invoke-terminal-retry-after-queue-full";
    let occurred_at = "2026-03-17 18:13:40";
    let terminal =
        api_invocation_from_runtime_record(&test_proxy_capture_record(invoke_id, occurred_at));

    let first_terminal = store.upsert_terminal(terminal.clone());
    assert!(
        !first_terminal.already_terminal,
        "first terminal overlay should not be considered duplicate"
    );
    assert!(
        store.clear_terminal_tombstone(invoke_id, occurred_at),
        "enqueue failure should clear the tombstone while keeping the terminal overlay"
    );

    let retry_terminal = store.upsert_terminal(terminal);
    assert!(
        !retry_terminal.already_terminal,
        "retry after queue-full should be allowed to enqueue again"
    );
    assert!(
        store.remove_non_terminal(invoke_id, occurred_at).is_none(),
        "drop-guard cleanup must not remove a queued terminal overlay before sqlite flush"
    );

    let mut delayed_running = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &RequestCaptureInfo::default(),
        Some("198.51.100.88"),
        None,
        None,
        false,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        0.0,
        0.0,
        0.0,
        0.0,
    );
    delayed_running.status = "running".to_string();
    let running_outcome = store.upsert(api_invocation_from_runtime_record(&delayed_running));
    assert!(
        running_outcome.skipped_terminal,
        "terminal overlay should still block delayed running snapshots even without a tombstone"
    );
    let snapshot = store.snapshot();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].status.as_deref(), Some("success"));
}

#[tokio::test]
async fn route_failure_cleanup_skips_queued_terminal_runtime_overlay() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let invoke_id = "queued-terminal-route-cleanup";
    let occurred_at = "2026-03-17 18:13:41";
    let terminal =
        api_invocation_from_runtime_record(&test_proxy_capture_record(invoke_id, occurred_at));

    state.proxy_runtime_invocations.upsert_terminal(terminal);

    assert!(
        !should_record_route_failure_after_attempt_recovery_for_test(
            state.as_ref(),
            invoke_id,
            occurred_at,
            false,
        )
        .await,
        "queued terminal overlay should prevent DB-missing cleanup from recording route failure"
    );
    assert!(
        !should_record_route_failure_after_attempt_recovery_for_test(
            state.as_ref(),
            invoke_id,
            occurred_at,
            true,
        )
        .await,
        "queued terminal overlay should also prevent recovered-invocation cleanup from penalizing the account"
    );
}

#[tokio::test]
async fn delayed_runtime_snapshot_after_terminal_does_not_reintroduce_running_overlay() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        prompt_cache_key: Some("pck-delayed-running".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "invoke-delayed-runtime-after-terminal";
    let occurred_at = "2026-03-17 18:13:36";
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-delayed-running"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-01"),
        Some(1),
        Some(1),
        None,
        None,
        12.0,
        3.0,
        99.0,
        120.0,
    );

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, occurred_at),
    )
    .await
    .expect("terminal record should enqueue before delayed runtime snapshot");
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;

    let mut rx = state.broadcaster.subscribe();
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, running_record)
        .await
        .expect("delayed runtime snapshot should be skipped after terminal persistence");

    let delayed_broadcast =
        tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
    assert!(
        delayed_broadcast.is_err(),
        "stale delayed running snapshot should not be rebroadcast after terminal persistence"
    );

    let Json(after_delayed_runtime) = list_invocations(
        State(state),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            page_size: Some(10),
            ..Default::default()
        }),
    )
    .await
    .expect("terminal invocation should remain visible without stale memory overlay");
    assert_eq!(after_delayed_runtime.total, 1);
    assert_eq!(after_delayed_runtime.records.len(), 1);
    assert!(after_delayed_runtime.records[0].id > 0);
    assert_eq!(
        after_delayed_runtime.records[0].status.as_deref(),
        Some("success")
    );
}

#[tokio::test]
async fn persist_proxy_capture_record_finalizes_existing_running_row_in_place() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: false,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "invoke-runtime-broadcast";
    let occurred_at = "2026-03-17 18:13:34";
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        None,
        false,
        None,
        None,
        None,
        None,
        Some("edge-runtime"),
        None,
        None,
        None,
        None,
        12.0,
        3.0,
        99.0,
        0.0,
    );

    let running = persist_proxy_capture_runtime_record(&state.pool, running_record)
        .await
        .expect("persist running record")
        .expect("running record should be inserted");
    assert!(running.id > 0);
    assert_eq!(running.status.as_deref(), Some("running"));

    let mut terminal_record = test_proxy_capture_record(invoke_id, occurred_at);
    terminal_record.req_raw = RawPayloadMeta {
        path: Some("proxy_raw_payloads/invoke-runtime-broadcast-request.bin".to_string()),
        size_bytes: 128,
        truncated: false,
        truncated_reason: None,
    };
    terminal_record.resp_raw = RawPayloadMeta {
        path: Some("proxy_raw_payloads/invoke-runtime-broadcast-response.bin.gz".to_string()),
        size_bytes: 256,
        truncated: false,
        truncated_reason: None,
    };
    let finalized = persist_proxy_capture_record(&state.pool, Instant::now(), terminal_record)
        .await
        .expect("finalize record")
        .expect("terminal update should reuse running row");

    assert_eq!(finalized.id, running.id);
    assert_eq!(finalized.status.as_deref(), Some("success"));
    let duplicate_count = sqlx::query_scalar::<_, i64>(
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
    .expect("count invocation rows");
    assert_eq!(duplicate_count, 1);

    let raw_row = sqlx::query_as::<
        _,
        (
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<i64>,
        ),
    >(
        r#"
        SELECT
            request_raw_path,
            request_raw_codec,
            request_raw_size,
            response_raw_path,
            response_raw_codec,
            response_raw_size
        FROM codex_invocations
        WHERE id = ?1
        "#,
    )
    .bind(finalized.id)
    .fetch_one(&state.pool)
    .await
    .expect("load finalized raw metadata");
    assert_eq!(
        raw_row.0.as_deref(),
        Some("proxy_raw_payloads/invoke-runtime-broadcast-request.bin")
    );
    assert_eq!(raw_row.1.as_deref(), Some(RAW_CODEC_IDENTITY));
    assert_eq!(raw_row.2, Some(128));
    assert_eq!(
        raw_row.3.as_deref(),
        Some("proxy_raw_payloads/invoke-runtime-broadcast-response.bin.gz")
    );
    assert_eq!(raw_row.4.as_deref(), Some(RAW_CODEC_GZIP));
    assert_eq!(raw_row.5, Some(256));
}
