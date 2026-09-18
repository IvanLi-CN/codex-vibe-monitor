#[tokio::test]
pub(crate) async fn admitted_proxy_capture_snapshot_can_be_cleared_before_terminal_on_early_error()
{
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
pub(crate) async fn admitted_proxy_capture_snapshot_is_cleared_when_cleanup_guard_drops_before_attempt()
 {
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
pub(crate) async fn admitted_proxy_capture_snapshot_is_terminalized_on_pre_attempt_error() {
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
pub(crate) async fn account_timeseries_replaces_stale_db_runtime_placeholder_after_account_switch()
{
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
    let initial_record = timeseries_runtime_record(
        invoke_id,
        &occurred_at,
        &request_info,
        17,
        [12.0, 2.0, 40.0, 80.0],
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, initial_record)
        .await
        .expect("initial runtime snapshot should enter memory");

    let updated_same_account_record = timeseries_runtime_record(
        invoke_id,
        &occurred_at,
        &request_info,
        17,
        [14.0, 4.0, 60.0, 100.0],
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, updated_same_account_record)
        .await
        .expect("same-account runtime update should refresh memory only");

    assert_updated_timeseries_timing(&state).await;

    let switched_record = timeseries_runtime_record(
        invoke_id,
        &occurred_at,
        &request_info,
        23,
        [13.0, 3.0, 50.0, 90.0],
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, switched_record)
        .await
        .expect("account-switched runtime snapshot should refresh memory only");

    assert_account_in_flight(&state, 17, 0).await;
    assert_account_in_flight(&state, 23, 1).await;
}

#[tokio::test]
pub(crate) async fn terminal_db_row_wins_over_stale_memory_runtime_overlay() {
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
    let running_record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
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
    ));

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
pub(crate) async fn queued_terminal_overlay_preserves_total_when_db_running_placeholder_exists() {
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
pub(crate) async fn clearing_terminal_tombstone_allows_enqueue_retry_without_running_regression() {
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

    let mut delayed_running = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
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
    ));
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
pub(crate) async fn route_failure_cleanup_skips_queued_terminal_runtime_overlay() {
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
pub(crate) async fn delayed_runtime_snapshot_after_terminal_does_not_reintroduce_running_overlay() {
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
    let running_record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
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
    ));

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
pub(crate) async fn persist_proxy_capture_record_finalizes_existing_running_row_in_place() {
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
    let running_record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
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
    ));

    let running = persist_proxy_capture_runtime_record(&state.pool, running_record)
        .await
        .expect("persist running record")
        .expect("running record should be inserted");
    assert!(running.id > 0);
    assert_eq!(running.status.as_deref(), Some("running"));

    let mut terminal_record = test_proxy_capture_record(invoke_id, occurred_at);
    attach_test_raw_metadata(&mut terminal_record);
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

    assert_finalized_raw_metadata(&state.pool, finalized.id).await;
}

#[tokio::test]
pub(crate) async fn persist_proxy_capture_record_repairs_proxy_interrupted_recovery_row_with_terminal_result()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "invoke-repair-interrupted-terminal";
    let occurred_at = "2026-03-17 18:13:35";
    let running_record = build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.89"),
        Some("sticky-repair-interrupted"),
        Some("pck-repair-interrupted"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        12.0,
        3.0,
        99.0,
        120.0,
    ));

    let running = persist_proxy_capture_runtime_record(&state.pool, running_record)
        .await
        .expect("persist running record")
        .expect("running record should be inserted");
    mark_and_assert_repairable_interrupted(&state.pool, running.id).await;

    let mut tx = state.pool.begin().await.expect("begin verification tx");
    let existing = load_persisted_invocation_identity_tx(tx.as_mut(), invoke_id, occurred_at)
        .await
        .expect("load persisted invocation identity")
        .expect("persisted invocation identity should exist");
    assert!(
        invocation_status_is_recoverable_proxy_interrupted(
            existing.status.as_deref(),
            existing.failure_kind.as_deref(),
        ),
        "persisted invocation identity should be recognized as repairable interrupted state",
    );
    tx.commit().await.expect("commit verification tx");

    let finalized = persist_proxy_capture_record(
        &state.pool,
        Instant::now(),
        test_proxy_capture_record(invoke_id, occurred_at),
    )
    .await
    .expect("finalize record")
    .expect("terminal update should repair proxy-interrupted recovery row");

    assert_eq!(finalized.id, running.id);
    assert_eq!(finalized.status.as_deref(), Some("success"));
    assert_eq!(finalized.failure_kind, None);
}

use super::*;
fn timeseries_runtime_record(
    invoke_id: &str,
    occurred_at: &str,
    request_info: &RequestCaptureInfo,
    account_id: i64,
    timings: [f64; 4],
) -> ProxyCaptureRecord {
    let (account_name, proxy_name, attempt_count) = if account_id == 17 {
        ("pool-account-17", "jp-relay-01", 1)
    } else {
        ("pool-account-23", "jp-relay-02", 2)
    };
    build_running_proxy_capture_record(RunningProxyCaptureRecordRequest(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-timeseries-account-switch"),
        true,
        Some(account_id),
        Some(account_name),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some(proxy_name),
        Some(attempt_count),
        Some(attempt_count),
        None,
        Some("gzip"),
        timings[0],
        timings[1],
        timings[2],
        timings[3],
    ))
}

async fn fetch_account_timeseries(state: &Arc<AppState>, account_id: i64) -> TimeseriesResponse {
    let Json(timeseries) = fetch_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("1m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(account_id),
        }),
    )
    .await
    .expect("account timeseries should load");
    timeseries
}

async fn assert_updated_timeseries_timing(state: &Arc<AppState>) {
    let timeseries = fetch_account_timeseries(state, 17).await;
    let point = timeseries
        .points
        .iter()
        .find(|point| point.in_flight_count == 1)
        .expect("updated in-flight point should be present");
    assert_eq!(point.first_response_byte_total_avg_ms, Some(178.0));
}

async fn assert_account_in_flight(state: &Arc<AppState>, account_id: i64, expected: i64) {
    let timeseries = fetch_account_timeseries(state, account_id).await;
    let count = timeseries
        .points
        .iter()
        .map(|point| point.in_flight_count)
        .sum::<i64>();
    assert_eq!(count, expected);
}

fn attach_test_raw_metadata(record: &mut ProxyCaptureRecord) {
    record.req_raw = RawPayloadMeta {
        path: Some("proxy_raw_payloads/invoke-runtime-broadcast-request.bin".to_string()),
        size_bytes: 128,
        truncated: false,
        truncated_reason: None,
    };
    record.resp_raw = RawPayloadMeta {
        path: Some("proxy_raw_payloads/invoke-runtime-broadcast-response.bin.gz".to_string()),
        size_bytes: 256,
        truncated: false,
        truncated_reason: None,
    };
}

async fn assert_finalized_raw_metadata(pool: &SqlitePool, id: i64) {
    let row = sqlx::query_as::<
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
        "SELECT request_raw_path, request_raw_codec, request_raw_size, \
         response_raw_path, response_raw_codec, response_raw_size \
         FROM codex_invocations WHERE id = ?1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("load finalized raw metadata");
    assert_eq!(
        row.0.as_deref(),
        Some("proxy_raw_payloads/invoke-runtime-broadcast-request.bin")
    );
    assert_eq!(row.1.as_deref(), Some(RAW_CODEC_IDENTITY));
    assert_eq!(row.2, Some(128));
    assert_eq!(
        row.3.as_deref(),
        Some("proxy_raw_payloads/invoke-runtime-broadcast-response.bin.gz")
    );
    assert_eq!(row.4.as_deref(), Some(RAW_CODEC_GZIP));
    assert_eq!(row.5, Some(256));
}

async fn mark_and_assert_repairable_interrupted(pool: &SqlitePool, id: i64) {
    sqlx::query("UPDATE codex_invocations SET status = ?2, failure_kind = ?3 WHERE id = ?1")
        .bind(id)
        .bind(INVOCATION_STATUS_INTERRUPTED)
        .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
        .execute(pool)
        .await
        .expect("mark row as proxy-interrupted recovery artifact");
    let row = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, failure_kind FROM codex_invocations WHERE id = ?1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("load recovery artifact row");
    assert_eq!(row.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(row.1.as_deref(), Some(PROXY_FAILURE_INVOCATION_INTERRUPTED));
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE id = ?1 AND ( \
         LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending') OR ( \
         LOWER(TRIM(COALESCE(status, ''))) = ?2 AND \
         LOWER(TRIM(COALESCE(failure_kind, ''))) = ?3))",
    )
    .bind(id)
    .bind(INVOCATION_STATUS_INTERRUPTED)
    .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    .fetch_one(pool)
    .await
    .expect("count repairable rows");
    assert_eq!(count, 1);
}
