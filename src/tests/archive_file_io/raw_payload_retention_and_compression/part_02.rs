use super::*;
use serde_json::json;

async fn assert_pending_attempt_row(pool: &SqlitePool, attempt_id: i64) {
    let row = sqlx::query_as::<_, (i64, String, Option<String>, Option<String>)>(
        "SELECT id, status, phase, finished_at FROM pool_upstream_request_attempts WHERE id = ?1",
    )
    .bind(attempt_id)
    .fetch_one(pool)
    .await
    .expect("load pending attempt row");
    assert_eq!(row.0, attempt_id);
    assert_eq!(row.1, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING);
    assert_eq!(
        row.2.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING)
    );
    assert_eq!(row.3, None);
}

async fn assert_finalized_attempt_row(pool: &SqlitePool, attempt_id: i64) {
    let row = sqlx::query_as::<_, (i64, String, Option<String>, Option<i64>, Option<String>, Option<f64>, Option<f64>, Option<f64>, Option<String>)>(
        "SELECT id, status, phase, http_status, finished_at, connect_latency_ms,
                first_byte_latency_ms, stream_latency_ms, upstream_request_id
         FROM pool_upstream_request_attempts WHERE invoke_id = 'pending-attempt-update' ORDER BY id ASC",
    )
    .fetch_one(pool)
    .await
    .expect("load finalized attempt row");
    assert_eq!(row.0, attempt_id);
    assert_eq!(row.1, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_eq!(
        row.2.as_deref(),
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED)
    );
    assert_eq!(row.3, Some(200));
    assert_eq!(row.4.as_deref(), Some("2026-03-23 20:49:05"));
    assert_eq!(row.5, Some(42.5));
    assert_eq!(row.6, Some(15.0));
    assert_eq!(row.7, Some(188.4));
    assert_eq!(row.8.as_deref(), Some("req_pool_123"));
}

#[tokio::test]
pub(crate) async fn imported_oauth_validation_job_delete_removes_completed_job() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let client = reqwest::Client::new();
    let create_response = post_same_origin_json(
        &client,
        addr,
        "/api/pool/upstream-accounts/oauth/imports/validation-jobs",
        json!({
            "groupName": test_required_group_name(),
            "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
            "items": [
                {
                    "sourceId": "invalid-source",
                    "fileName": "broken@duckmail.sbs.json",
                    "content": "{not-json",
                }
            ]
        })
        .to_string(),
    )
    .await;
    assert_eq!(create_response.status(), StatusCode::OK);
    let created: Value = create_response
        .json()
        .await
        .expect("read create job payload");
    let job_id = created.get("jobId").and_then(Value::as_str).expect("jobId");

    let events = collect_sse_events_until(
        &client,
        addr,
        &format!("/api/pool/upstream-accounts/oauth/imports/validation-jobs/{job_id}/events"),
        &["completed", "failed", "cancelled"],
        4,
    )
    .await;
    assert_eq!(
        events.first().map(|event| event.0.as_str()),
        Some("snapshot")
    );
    assert_eq!(
        events.last().map(|event| event.0.as_str()),
        Some("completed")
    );

    let delete_response = delete_same_origin(
        &client,
        addr,
        &format!("/api/pool/upstream-accounts/oauth/imports/validation-jobs/{job_id}"),
    )
    .await;
    assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

    let response = client
        .get(format!(
            "http://{addr}/api/pool/upstream-accounts/oauth/imports/validation-jobs/{job_id}/events"
        ))
        .send()
        .await
        .expect("lookup request should succeed");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
pub(crate) async fn drain_runtime_after_shutdown_waits_for_summary_quota_broadcast_workers() {
    let state = test_state_from_config(test_config(), false).await;
    let (started_tx_a, started_rx_a) = tokio::sync::oneshot::channel();
    let (started_tx_b, started_rx_b) = tokio::sync::oneshot::channel();
    let release_a = Arc::new(Notify::new());
    let release_b = Arc::new(Notify::new());
    let worker_a = tokio::spawn({
        let release_a = release_a.clone();
        async move {
            started_tx_a
                .send(())
                .expect("first broadcast worker should report when it starts");
            release_a.notified().await;
        }
    });
    let worker_b = tokio::spawn({
        let release_b = release_b.clone();
        async move {
            started_tx_b
                .send(())
                .expect("second broadcast worker should report when it starts");
            release_b.notified().await;
        }
    });
    {
        let mut guard = state.proxy_summary_quota_broadcast_handle.lock().await;
        guard.extend([worker_a, worker_b]);
    }

    let drain_handle = tokio::spawn({
        let state = state.clone();
        async move {
            drain_runtime_after_shutdown(state, None, None, None, None, None, None, None, None)
                .await
        }
    });

    started_rx_a
        .await
        .expect("first broadcast worker should start before the drain waits on it");
    started_rx_b
        .await
        .expect("second broadcast worker should start before the drain waits on it");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !drain_handle.is_finished(),
        "runtime drain should wait for every tracked summary/quota broadcast worker"
    );

    release_a.notify_waiters();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !drain_handle.is_finished(),
        "runtime drain should keep waiting until the last tracked summary/quota broadcast worker exits"
    );

    release_b.notify_waiters();
    drain_handle
        .await
        .expect("drain task should join")
        .expect("runtime drain should finish once every broadcast worker does");
    assert!(
        state
            .proxy_summary_quota_broadcast_handle
            .lock()
            .await
            .is_empty(),
        "runtime drain should clear all tracked summary/quota broadcast workers"
    );
}

#[tokio::test]
pub(crate) async fn run_runtime_until_shutdown_exits_when_shutdown_token_is_cancelled_directly() {
    let state = test_state_from_config(test_config(), false).await;
    state.shutdown.cancel();

    tokio::time::timeout(
        Duration::from_secs(1),
        run_runtime_until_shutdown(
            state.clone(),
            Instant::now(),
            false,
            std::future::pending::<()>(),
        ),
    )
    .await
    .expect("direct shutdown token cancellation should not hang runtime drain")
    .expect("runtime should exit cleanly after direct shutdown token cancellation");

    assert!(state.shutdown.is_cancelled());
}

#[tokio::test]
pub(crate) async fn run_runtime_until_shutdown_skips_xray_route_sync_when_shutdown_is_already_requested()
 {
    let runtime_dir = make_temp_test_dir("runtime-shutdown-xray-sync");
    fs::remove_dir_all(&runtime_dir).expect("remove temp runtime dir before startup");

    let mut config = test_config();
    config.xray_binary = "/path/to/non-existent-xray".to_string();
    config.xray_runtime_dir = runtime_dir.clone();
    let state = test_state_from_config(config, false).await;

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: vec!["vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#vless".to_string()],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        });
    }

    run_runtime_until_shutdown(state.clone(), Instant::now(), false, async {})
        .await
        .expect("runtime should exit cleanly when shutdown is already requested");

    assert!(state.shutdown.is_cancelled());
    assert!(
        !runtime_dir.exists(),
        "shutdown should skip xray route sync side effects when startup never begins"
    );
}

#[tokio::test]
pub(crate) async fn run_startup_stage_until_shutdown_skips_stage_when_shutdown_arrives_before_first_poll()
 {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::task::{Context, Poll};

    struct ReadyOnSecondPollFuture {
        polls: Arc<AtomicUsize>,
    }

    impl Future for ReadyOnSecondPollFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            let poll_count = self.polls.fetch_add(1, Ordering::SeqCst);
            if poll_count == 0 {
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        }
    }

    let shutdown_polls = Arc::new(AtomicUsize::new(0));
    let shutdown_signal = ReadyOnSecondPollFuture {
        polls: shutdown_polls.clone(),
    }
    .shared();
    let cancel = CancellationToken::new();
    let stage_started = Arc::new(AtomicBool::new(false));

    let outcome = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, {
        let stage_started = stage_started.clone();
        async move {
            stage_started.store(true, Ordering::SeqCst);
            13_u8
        }
    })
    .await;

    assert!(matches!(outcome, StartupStageOutcome::SkippedByShutdown));
    assert!(cancel.is_cancelled());
    assert!(
        !stage_started.load(Ordering::SeqCst),
        "shutdown should skip startup work that has not started polling yet"
    );
    assert_eq!(
        shutdown_polls.load(Ordering::SeqCst),
        2,
        "the shutdown future should only need the initial probe and the shutdown branch poll"
    );
}

#[tokio::test]
pub(crate) async fn run_startup_stage_until_shutdown_preserves_stage_result_when_shutdown_arrives_after_stage()
 {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll};

    struct FlagShutdownFuture {
        ready: Arc<AtomicBool>,
    }

    impl Future for FlagShutdownFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.ready.load(Ordering::SeqCst) {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }

    let shutdown_ready = Arc::new(AtomicBool::new(false));
    let shutdown_signal = FlagShutdownFuture {
        ready: shutdown_ready.clone(),
    }
    .shared();
    let cancel = CancellationToken::new();

    let outcome = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async {
        shutdown_ready.store(true, Ordering::SeqCst);
        42_u8
    })
    .await;

    match outcome {
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            assert_eq!(result, 42);
            assert!(shutdown_requested);
            assert!(cancel.is_cancelled());
        }
        StartupStageOutcome::SkippedByShutdown => {
            panic!("stage result should be preserved when shutdown arrives after stage completion")
        }
    }
}

#[tokio::test]
pub(crate) async fn run_startup_stage_until_shutdown_waits_for_stage_completion_when_shutdown_arrives_mid_stage()
 {
    let shutdown = Arc::new(Notify::new());
    let shutdown_signal = {
        let shutdown = shutdown.clone();
        async move {
            shutdown.notified().await;
        }
        .shared()
    };
    let cancel = CancellationToken::new();
    let release_stage = Arc::new(Notify::new());
    let (stage_started_tx, stage_started_rx) = tokio::sync::oneshot::channel();

    let shutdown_task = {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            stage_started_rx
                .await
                .expect("stage should signal when startup work begins");
            shutdown.notify_waiters();
        })
    };
    let release_task = {
        let cancel = cancel.clone();
        let release_stage = release_stage.clone();
        tokio::spawn(async move {
            cancel.cancelled().await;
            release_stage.notify_waiters();
        })
    };

    let outcome = run_startup_stage_until_shutdown(&shutdown_signal, &cancel, async move {
        stage_started_tx
            .send(())
            .expect("stage start signal should be sent exactly once");
        release_stage.notified().await;
        7_u8
    })
    .await;

    shutdown_task
        .await
        .expect("shutdown trigger task should finish");
    release_task
        .await
        .expect("stage release task should finish");

    match outcome {
        StartupStageOutcome::Completed {
            result,
            shutdown_requested,
        } => {
            assert_eq!(result, 7);
            assert!(shutdown_requested);
            assert!(cancel.is_cancelled());
        }
        StartupStageOutcome::SkippedByShutdown => {
            panic!("stage should finish after shutdown begins once startup work is already running")
        }
    }
}

#[tokio::test]
pub(crate) async fn bootstrap_probe_round_skips_work_when_shutdown_is_in_progress() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::OK).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize forward proxy url");
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize forward proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid upstream base url"),
    )
    .await;
    state.shutdown.cancel();

    spawn_forward_proxy_bootstrap_probe_round(
        state.clone(),
        vec![ForwardProxyEndpoint {
            key: proxy_key.clone(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            display_name: normalized_proxy.clone(),
            protocol: ForwardProxyProtocol::Http,
            endpoint_url: Some(Url::parse(&normalized_proxy).expect("valid normalized proxy url")),
            raw_url: Some(normalized_proxy.clone()),
        }],
        "test-shutdown",
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    let probe_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    assert_eq!(probe_count, 0);

    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn forward_proxy_bootstrap_probe_round_stops_mid_probe_when_shutdown_begins() {
    let request_started = Arc::new(Notify::new());
    let release_request = Arc::new(Notify::new());
    let (proxy_url, proxy_handle) = spawn_test_blocking_forward_proxy_status(
        StatusCode::OK,
        request_started.clone(),
        release_request.clone(),
    )
    .await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize forward proxy url");
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize forward proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid upstream base url"),
    )
    .await;

    spawn_forward_proxy_bootstrap_probe_round(
        state.clone(),
        vec![ForwardProxyEndpoint {
            key: proxy_key.clone(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            display_name: normalized_proxy.clone(),
            protocol: ForwardProxyProtocol::Http,
            endpoint_url: Some(Url::parse(&normalized_proxy).expect("valid normalized proxy url")),
            raw_url: Some(normalized_proxy.clone()),
        }],
        "test-shutdown-mid-probe",
    );

    tokio::time::timeout(Duration::from_secs(1), request_started.notified())
        .await
        .expect("bootstrap probe should reach the forward proxy before shutdown");
    state.shutdown.cancel();
    release_request.notify_waiters();
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await,
        0,
        "shutdown should stop an in-flight bootstrap probe without recording a probe attempt"
    );

    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn finish_summary_quota_broadcast_idle_flushes_pending_tail_when_shutdown_arrives()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;
    let mut rx = state.broadcaster.subscribe();

    state
        .proxy_summary_quota_broadcast_seq
        .store(2, Ordering::Release);
    state
        .proxy_summary_quota_broadcast_running
        .store(true, Ordering::Release);
    state.shutdown.cancel();

    let should_continue = finish_summary_quota_broadcast_idle(
        SummaryQuotaBroadcastIdleContext {
            latest_broadcast_seq: state.proxy_summary_quota_broadcast_seq.as_ref(),
            broadcast_running: state.proxy_summary_quota_broadcast_running.as_ref(),
            shutdown: &state.shutdown,
            pool: &state.pool,
            broadcaster: &state.broadcaster,
            broadcast_state_cache: state.broadcast_state_cache.as_ref(),
            subscription_hub: state.subscription_hub.as_ref(),
            invoke_id: "idle-shutdown-tail",
        },
        1,
    )
    .await;

    assert!(
        !should_continue,
        "shutdown tail should flush inline instead of trying to restart the broadcast worker"
    );
    assert!(
        !state
            .proxy_summary_quota_broadcast_running
            .load(Ordering::Acquire),
        "shutdown tail flush should leave the worker idle"
    );

    let mut saw_quota = false;
    for _ in 0..8 {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for shutdown idle-tail broadcast event")
            .expect("broadcast channel should stay open");
        match payload {
            BroadcastPayload::Quota { snapshot } => {
                saw_quota = true;
                assert_eq!(snapshot.total_requests, 9);
            }
            BroadcastPayload::Records { .. }
            | BroadcastPayload::Version { .. }
            | BroadcastPayload::PoolAttempts { .. }
            | BroadcastPayload::PoolAttemptsSnapshotUnavailable { .. }
            | BroadcastPayload::DashboardActivityLive { .. }
            | BroadcastPayload::DashboardCurrentSlice { .. }
            | BroadcastPayload::DashboardNetworkSlice { .. }
            | BroadcastPayload::DashboardTerminalSlice { .. }
            | BroadcastPayload::PromptCacheConversationChanged { .. }
            | BroadcastPayload::PromptCacheConversationStickyRouteChanged { .. } => {}
        }

        if saw_quota {
            break;
        }
    }

    assert!(
        saw_quota,
        "shutdown idle-tail flush should emit the latest quota snapshot"
    );
}

#[tokio::test]
pub(crate) async fn persist_and_broadcast_proxy_capture_flushes_follow_up_when_shutdown_begins_after_record_event()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;
    let _quota_lease = state
        .subscription_hub
        .register_test_topic_name("quota.current")
        .await;

    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "shutdown-tail-broadcast";
    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &now_local),
    )
    .await
    .expect("persist proxy capture before shutdown");
    state.shutdown.cancel();

    let mut saw_record = false;
    let mut saw_quota = false;
    for _ in 0..16 {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for shutdown tail broadcast event")
            .expect("broadcast channel should stay open");
        match payload {
            BroadcastPayload::Records { records } => {
                saw_record |= records
                    .into_iter()
                    .any(|record| record.invoke_id == invoke_id);
            }
            BroadcastPayload::Quota { snapshot } => {
                saw_quota = true;
                assert_eq!(snapshot.total_requests, 9);
            }
            BroadcastPayload::Version { .. }
            | BroadcastPayload::PoolAttempts { .. }
            | BroadcastPayload::PoolAttemptsSnapshotUnavailable { .. }
            | BroadcastPayload::DashboardActivityLive { .. }
            | BroadcastPayload::DashboardCurrentSlice { .. }
            | BroadcastPayload::DashboardNetworkSlice { .. }
            | BroadcastPayload::DashboardTerminalSlice { .. }
            | BroadcastPayload::PromptCacheConversationChanged { .. }
            | BroadcastPayload::PromptCacheConversationStickyRouteChanged { .. } => {}
        }

        if saw_record && saw_quota {
            break;
        }
    }

    assert!(
        saw_record,
        "shutdown tail path should still emit the persisted record"
    );
    assert!(
        saw_quota,
        "shutdown tail path should flush the latest quota snapshot"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !state
            .proxy_summary_quota_broadcast_running
            .load(Ordering::Acquire),
        "summary/quota broadcast worker should quiesce after flushing the shutdown tail"
    );
}

#[tokio::test]
pub(crate) async fn persist_and_broadcast_runtime_terminal_schedules_follow_up_after_flush() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;
    let _quota_lease = state
        .subscription_hub
        .register_test_topic_name("quota.current")
        .await;

    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "runtime-terminal-follow-up";
    persist_and_broadcast_proxy_capture_terminal_record(
        state.as_ref(),
        test_proxy_capture_record(invoke_id, &now_local),
    )
    .await
    .expect("queue runtime terminal record");

    let mut saw_record = false;
    let mut saw_quota = false;
    for _ in 0..16 {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for runtime terminal follow-up event")
            .expect("broadcast channel should stay open");
        match payload {
            BroadcastPayload::Records { records } => {
                saw_record |= records
                    .into_iter()
                    .any(|record| record.invoke_id == invoke_id);
            }
            BroadcastPayload::Quota { snapshot } => {
                saw_quota = true;
                assert_eq!(snapshot.total_requests, 9);
            }
            BroadcastPayload::Version { .. }
            | BroadcastPayload::PoolAttempts { .. }
            | BroadcastPayload::PoolAttemptsSnapshotUnavailable { .. }
            | BroadcastPayload::DashboardActivityLive { .. }
            | BroadcastPayload::DashboardCurrentSlice { .. }
            | BroadcastPayload::DashboardNetworkSlice { .. }
            | BroadcastPayload::DashboardTerminalSlice { .. }
            | BroadcastPayload::PromptCacheConversationChanged { .. }
            | BroadcastPayload::PromptCacheConversationStickyRouteChanged { .. } => {}
        }

        if saw_record && saw_quota {
            break;
        }
    }

    assert!(
        saw_record,
        "runtime terminal path should still emit the terminal record immediately"
    );
    assert!(
        saw_quota,
        "runtime terminal path should broadcast quota after terminal sqlite flush"
    );
}

#[tokio::test]
pub(crate) async fn persist_and_broadcast_proxy_capture_skips_summary_worker_during_shutdown() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    state.shutdown.cancel();

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record("shutdown-broadcast", &format_utc_iso(Utc::now())),
    )
    .await
    .expect("persist proxy capture during shutdown");

    let payload = tokio::time::timeout(Duration::from_millis(100), rx.recv())
        .await
        .expect("shutdown path should still emit the persisted record")
        .expect("broadcast channel should stay open");
    assert!(
        matches!(payload, BroadcastPayload::Records { .. }),
        "shutdown path should keep the live record event aligned with persisted data"
    );
    assert!(
        !state
            .proxy_summary_quota_broadcast_running
            .load(Ordering::Acquire),
        "summary/quota broadcast worker should not stay active during shutdown"
    );
}

#[tokio::test]
pub(crate) async fn broadcast_recovered_proxy_invocations_skips_follow_up_without_subscribers() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let invoke_id = "recovered-follow-up-no-subscribers";
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
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind(INVOCATION_STATUS_RUNNING)
    .bind("{}")
    .bind("{\"endpoint\":\"/v1/responses\"}")
    .execute(&state.pool)
    .await
    .expect("insert running invocation for recovery broadcast");

    let selectors = vec![InvocationRecoverySelector::new(
        invoke_id,
        occurred_at.clone(),
    )];
    let recovered = recover_proxy_invocations_with_scope(
        &state.pool,
        ProxyInvocationRecoveryScope::Selectors(&selectors),
    )
    .await
    .expect("recover invocation for runtime broadcast");
    assert_eq!(
        recovered.len(),
        1,
        "test setup should recover exactly one invocation"
    );

    broadcast_recovered_proxy_invocations(state.as_ref(), &recovered)
        .await
        .expect("broadcast recovered invocation without subscribers");

    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        state
            .proxy_summary_quota_broadcast_seq
            .load(Ordering::Acquire),
        0,
        "recovered no-subscriber path should not enqueue summary/quota follow-up work"
    );
    assert!(
        !state
            .proxy_summary_quota_broadcast_running
            .load(Ordering::Acquire),
        "recovered no-subscriber path should keep the summary/quota worker idle"
    );
}

#[tokio::test]
pub(crate) async fn finalize_pool_upstream_request_attempt_updates_pending_row_in_place() {
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
        "2026-03-23 20:49:00",
    )
    .await;
    let attempt_id = pending
        .attempt_id
        .expect("pending attempt should be inserted immediately");

    assert_pending_attempt_row(&state.pool, attempt_id).await;

    finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        PoolAttemptFinalization {
            finished_at: "2026-03-23 20:49:05",
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            http_status: Some(StatusCode::OK),
            downstream_http_status: None,
            failure_kind: None,
            error_message: None,
            downstream_error_message: None,
            connect_latency_ms: Some(42.5),
            first_byte_latency_ms: Some(15.0),
            stream_latency_ms: Some(188.4),
            upstream_request_id: Some("req_pool_123"),
            compact_support_status: None,
            compact_support_reason: None,
        },
    )
    .await
    .expect("finalize pending attempt");

    assert_finalized_attempt_row(&state.pool, attempt_id).await;
}

#[tokio::test]
pub(crate) async fn begin_pool_upstream_request_attempt_with_scope_persists_group_and_proxy_snapshots()
 {
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
        PoolAttemptStartScope {
            group_name_snapshot: Some("prod"),
            proxy_binding_key_snapshot: Some(FORWARD_PROXY_DIRECT_KEY),
            upstream_account_id: account_id,
            upstream_route_key: "route-scoped",
        },
        PoolAttemptStartIndexes {
            attempt_index: 1,
            distinct_account_index: 1,
            same_account_retry_index: 0,
        },
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
pub(crate) async fn finalize_pool_upstream_request_attempt_fallback_preserves_scope_snapshots() {
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
    };

    finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        PoolAttemptFinalization {
            finished_at: "2026-03-23 21:00:01",
            status: POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
            http_status: None,
            downstream_http_status: None,
            failure_kind: Some("fallback_transport_failure"),
            error_message: Some("fallback transport failure"),
            downstream_error_message: None,
            connect_latency_ms: Some(11.0),
            first_byte_latency_ms: Some(17.0),
            stream_latency_ms: None,
            upstream_request_id: Some("req_scope_fallback"),
            compact_support_status: None,
            compact_support_reason: None,
        },
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
pub(crate) async fn insert_pool_upstream_terminal_attempt_skips_pre_dispatch_pseudo_attempt_rows() {
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
pub(crate) async fn insert_pool_upstream_terminal_attempt_skips_oauth_pre_dispatch_pseudo_attempt_rows()
 {
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
pub(crate) async fn broadcast_pool_upstream_attempts_snapshot_emits_pending_attempts() {
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
pub(crate) async fn advance_pool_upstream_request_attempt_phase_buffers_progress_without_immediate_broadcast()
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
            phase: POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST.to_string(),
            connect_latency_ms: None,
            first_byte_latency_ms: None,
            compact_support_status: None,
            compact_support_reason: None,
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
pub(crate) async fn fetch_invocation_pool_attempts_returns_live_pending_attempts_without_parent_invocation()
 {
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
        PoolAttemptStartScope {
            group_name_snapshot: Some("live-fetch-group"),
            proxy_binding_key_snapshot: Some(FORWARD_PROXY_DIRECT_KEY),
            upstream_account_id: account_id,
            upstream_route_key: "route-primary",
        },
        Some(PoolRoutingSelectionSource::StickyReuse),
        PoolAttemptStartIndexes {
            attempt_index: 1,
            distinct_account_index: 1,
            same_account_retry_index: 1,
        },
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
        PoolAttemptStartScope {
            group_name_snapshot: Some("live-fetch-group"),
            proxy_binding_key_snapshot: Some(FORWARD_PROXY_DIRECT_KEY),
            upstream_account_id: account_id,
            upstream_route_key: "route-fallback",
        },
        Some(PoolRoutingSelectionSource::FreshAssignment),
        PoolAttemptStartIndexes {
            attempt_index: 2,
            distinct_account_index: 2,
            same_account_retry_index: 1,
        },
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
pub(crate) async fn insert_and_broadcast_pool_upstream_terminal_attempt_skips_pre_dispatch_snapshots()
 {
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
pub(crate) async fn recover_orphaned_pool_upstream_request_attempts_marks_pending_rows_terminal() {
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
