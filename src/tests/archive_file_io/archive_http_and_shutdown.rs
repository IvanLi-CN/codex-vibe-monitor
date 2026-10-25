use super::*;
use futures_util::FutureExt;
use serde_json::json;

#[cfg(unix)]
#[tokio::test]
async fn terminate_child_process_prefers_sigterm_when_process_exits_cleanly() {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("trap 'exit 0' TERM; while :; do sleep 0.1; done")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sigterm-friendly child");

    let outcome = terminate_child_process(&mut child, Duration::from_secs(1), "test-child").await;

    assert_eq!(outcome, ChildTerminationOutcome::Graceful);
    assert!(
        child
            .try_wait()
            .expect("poll child after terminate")
            .is_some()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn terminate_child_process_falls_back_to_force_kill_when_grace_period_is_exhausted() {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("trap '' TERM; while :; do sleep 1 & wait $!; done")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn TERM-ignoring child for forced shutdown fallback");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let outcome =
        terminate_child_process(&mut child, Duration::from_millis(100), "test-child").await;

    assert_eq!(outcome, ChildTerminationOutcome::Forced);
    assert!(
        child
            .try_wait()
            .expect("poll child after force kill")
            .is_some()
    );
}
#[tokio::test]
async fn spawn_http_server_leaves_health_unready_until_runtime_declares_readiness() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");

    assert!(
        !state.startup_ready.load(Ordering::Acquire),
        "HTTP startup should not mark the app ready before runtime startup completes"
    );
    let response = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond while startup is incomplete");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
async fn http_server_graceful_shutdown_stops_accepting_new_connections() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let healthy_response = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond before shutdown");
    assert_eq!(healthy_response.status(), StatusCode::OK);

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");

    let err = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect_err("server should stop accepting new connections after shutdown");
    assert!(err.is_connect() || err.is_timeout());
}

fn build_large_imported_oauth_validate_body(item_count: usize, padding_len: usize) -> String {
    let items = (0..item_count)
        .map(|index| {
            json!({
                "sourceId": format!("source-{index}"),
                "fileName": format!("user-{index}@duckmail.sbs.json"),
                "content": json!({
                    "type": "codex",
                    "email": format!("user-{index}@duckmail.sbs"),
                    "account_id": format!("acct_{index}"),
                    "expired": "2026-03-20T00:00:00.000Z",
                    "access_token": "access-token",
                    "refresh_token": "refresh-token",
                    "id_token": "header.payload.signature",
                    "padding": "x".repeat(padding_len),
                })
                .to_string(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "groupName": test_required_group_name(),
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "items": items,
    })
    .to_string()
}

fn build_large_imported_oauth_import_body(item_count: usize, padding_len: usize) -> String {
    let mut selected_source_ids = Vec::with_capacity(item_count);
    let items = (0..item_count)
        .map(|index| {
            let source_id = format!("source-{index}");
            selected_source_ids.push(source_id.clone());
            json!({
                "sourceId": source_id,
                "fileName": format!("user-{index}@duckmail.sbs.json"),
                "content": json!({
                    "type": "codex",
                    "email": format!("user-{index}@duckmail.sbs"),
                    "account_id": format!("acct_{index}"),
                    "expired": "2026-03-20T00:00:00.000Z",
                    "access_token": "access-token",
                    "refresh_token": "refresh-token",
                    "id_token": "header.payload.signature",
                    "padding": "x".repeat(padding_len),
                })
                .to_string(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "groupName": test_required_group_name(),
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "items": items,
        "selectedSourceIds": selected_source_ids,
        "tagIds": [],
    })
    .to_string()
}

async fn post_same_origin_json(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    body: String,
) -> reqwest::Response {
    let origin = format!("http://{addr}");
    client
        .post(format!("{origin}{path}"))
        .header(reqwest::header::ORIGIN, &origin)
        .header(reqwest::header::REFERER, format!("{origin}/"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .expect("request should succeed")
}

async fn delete_same_origin(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
) -> reqwest::Response {
    let origin = format!("http://{addr}");
    client
        .delete(format!("{origin}{path}"))
        .header(reqwest::header::ORIGIN, &origin)
        .header(reqwest::header::REFERER, format!("{origin}/"))
        .send()
        .await
        .expect("request should succeed")
}

fn parse_sse_frame(frame: &str) -> Option<(String, String)> {
    let mut event_name = None::<String>;
    let mut data_lines = Vec::new();
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim().to_string());
        }
    }
    match (event_name, data_lines.is_empty()) {
        (Some(event_name), false) => Some((event_name, data_lines.join("\n"))),
        _ => None,
    }
}

async fn collect_sse_events_until(
    client: &reqwest::Client,
    addr: SocketAddr,
    path: &str,
    terminal_event_names: &[&str],
    max_events: usize,
) -> Vec<(String, String)> {
    let response = client
        .get(format!("http://{addr}{path}"))
        .send()
        .await
        .expect("sse request should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.starts_with("text/event-stream")),
        Some(true)
    );

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut events = Vec::new();
    while events.len() < max_events {
        let next_chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("sse stream should produce data before timeout");
        let chunk = next_chunk
            .expect("sse stream should stay open")
            .expect("sse chunk should decode");
        buffer.push_str(
            &std::str::from_utf8(&chunk)
                .expect("sse chunk should be utf-8")
                .replace("\r\n", "\n"),
        );
        while let Some(split_index) = buffer.find("\n\n") {
            let frame = buffer[..split_index].to_string();
            buffer = buffer[(split_index + 2)..].to_string();
            if let Some(event) = parse_sse_frame(&frame) {
                let is_terminal = terminal_event_names
                    .iter()
                    .any(|candidate| *candidate == event.0);
                events.push(event);
                if is_terminal || events.len() >= max_events {
                    return events;
                }
            }
        }
    }
    events
}

#[tokio::test]
async fn imported_oauth_validate_route_accepts_large_request_body() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let client = reqwest::Client::new();
    let small_body = build_large_imported_oauth_validate_body(1, 64);
    let small_response = post_same_origin_json(
        &client,
        addr,
        "/api/pool/upstream-accounts/oauth/imports/validate",
        small_body,
    )
    .await;
    assert_eq!(small_response.status(), StatusCode::OK);

    let large_body = build_large_imported_oauth_validate_body(934, 4096);
    assert!(
        large_body.len() > 4_000_000,
        "expected test payload to exceed 4 MiB, got {} bytes",
        large_body.len()
    );
    let large_response = post_same_origin_json(
        &client,
        addr,
        "/api/pool/upstream-accounts/oauth/imports/validate",
        large_body,
    )
    .await;
    assert_eq!(large_response.status(), StatusCode::OK);
    let large_text = large_response.text().await.expect("read validate body");
    assert!(
        !large_text.contains("Failed to buffer the request body"),
        "validate route should no longer reject large import payloads before business handling"
    );
    assert!(large_text.contains("\"rows\""));

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
async fn imported_oauth_import_route_accepts_large_request_body() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");
    state.startup_ready.store(true, Ordering::Release);

    let client = reqwest::Client::new();
    let large_body = build_large_imported_oauth_import_body(934, 4096);
    assert!(
        large_body.len() > 4_000_000,
        "expected test payload to exceed 4 MiB, got {} bytes",
        large_body.len()
    );
    let response = post_same_origin_json(
        &client,
        addr,
        "/api/pool/upstream-accounts/oauth/imports",
        large_body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let text = response.text().await.expect("read import body");
    assert!(
        !text.contains("Failed to buffer the request body"),
        "import route should no longer reject large import payloads before business handling"
    );
    assert!(text.contains("\"summary\""));

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
async fn imported_oauth_validation_job_stream_replays_snapshot_and_completed_terminal_event() {
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
    assert_eq!(
        created
            .get("snapshot")
            .and_then(|snapshot| snapshot.get("rows"))
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .and_then(|row| row.get("status"))
            .and_then(Value::as_str),
        Some("pending")
    );

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
    let completed_payload: Value = serde_json::from_str(&events.last().expect("completed event").1)
        .expect("completed event should be valid json");
    assert_eq!(
        completed_payload
            .get("snapshot")
            .and_then(|snapshot| snapshot.get("rows"))
            .and_then(Value::as_array)
            .and_then(|rows| rows.first())
            .and_then(|row| row.get("status"))
            .and_then(Value::as_str),
        Some("invalid")
    );

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");
}

#[tokio::test]
async fn imported_oauth_validation_job_delete_removes_completed_job() {
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
async fn drain_runtime_after_shutdown_waits_for_summary_quota_broadcast_workers() {
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
async fn run_runtime_until_shutdown_exits_when_shutdown_token_is_cancelled_directly() {
    let state = test_state_from_config(test_config(), false).await;
    state.shutdown.cancel();

    tokio::time::timeout(
        Duration::from_secs(1),
        run_runtime_until_shutdown(
            state.clone(),
            Instant::now(),
            false,
            None,
            std::future::pending::<()>(),
        ),
    )
    .await
    .expect("direct shutdown token cancellation should not hang runtime drain")
    .expect("runtime should exit cleanly after direct shutdown token cancellation");

    assert!(state.shutdown.is_cancelled());
}

#[tokio::test]
async fn run_runtime_until_shutdown_skips_xray_route_sync_when_shutdown_is_already_requested() {
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

    run_runtime_until_shutdown(state.clone(), Instant::now(), false, None, async {})
        .await
        .expect("runtime should exit cleanly when shutdown is already requested");

    assert!(state.shutdown.is_cancelled());
    assert!(
        !runtime_dir.exists(),
        "shutdown should skip xray route sync side effects when startup never begins"
    );
}

#[tokio::test]
async fn run_startup_stage_until_shutdown_skips_stage_when_shutdown_arrives_before_first_poll() {
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
async fn run_startup_stage_until_shutdown_preserves_stage_result_when_shutdown_arrives_after_stage()
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
async fn run_startup_stage_until_shutdown_waits_for_stage_completion_when_shutdown_arrives_mid_stage()
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
async fn bootstrap_probe_round_skips_work_when_shutdown_is_in_progress() {
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
async fn forward_proxy_bootstrap_probe_round_stops_mid_probe_when_shutdown_begins() {
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
async fn finish_summary_quota_broadcast_idle_flushes_pending_tail_when_shutdown_arrives() {
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
async fn persist_and_broadcast_proxy_capture_flushes_follow_up_when_shutdown_begins_after_record_event()
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
async fn persist_and_broadcast_runtime_terminal_schedules_follow_up_after_flush() {
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
    let record = test_proxy_capture_record(invoke_id, &now_local);
    persist_and_broadcast_proxy_capture_terminal_record(state.as_ref(), record)
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
async fn persist_and_broadcast_proxy_capture_skips_summary_worker_during_shutdown() {
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
async fn broadcast_recovered_proxy_invocations_skips_follow_up_without_subscribers() {
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
