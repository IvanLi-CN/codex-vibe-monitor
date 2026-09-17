#[tokio::test]
pub(crate) async fn validate_single_forward_proxy_candidate_keeps_xray_startup_running_during_shutdown()
 {
    let temp_root = make_temp_test_dir("xray-validation-shutdown");
    let runtime_dir = temp_root.join("runtime");
    let mut config = test_config();
    config.xray_binary = "/path/to/non-existent-xray".to_string();
    config.xray_runtime_dir = runtime_dir;
    let state = test_state_from_config(config, false).await;
    state.shutdown.cancel();

    let err = validate_single_forward_proxy_candidate(
        state.as_ref(),
        "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none".to_string(),
    )
    .await
    .expect_err("validation should fail on missing xray binary, not on shutdown cancellation");
    let message = format!("{err:#}");
    assert!(
        message.contains("failed to start xray binary"),
        "expected validation to keep running through xray startup, got: {message}"
    );
    assert!(
        !message.contains("shutdown is in progress"),
        "request-scoped validation should not reuse the global shutdown token: {message}"
    );
}

#[tokio::test]
pub(crate) async fn xray_supervisor_sync_endpoints_keeps_stale_instances_alive_when_shutdown_starts()
 {
    let temp_root = make_temp_test_dir("xray-sync-stale-shutdown");
    let runtime_dir = temp_root.join("runtime");
    let mut supervisor = XraySupervisor::new("/path/to/non-existent-xray".to_string(), runtime_dir);
    let config_path = temp_root.join("stale-xray.json");
    fs::write(&config_path, "{}").expect("write stale xray config placeholder");
    let child = Command::new("python3")
        .arg("-c")
        .arg(
            "import time
time.sleep(30)
",
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn stale xray child");
    supervisor.instances.insert(
        "stale-xray".to_string(),
        XrayInstance {
            local_proxy_url: Url::parse("socks5://127.0.0.1:1080").expect("valid local proxy url"),
            config_path: config_path.clone(),
            child,
        },
    );
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let mut endpoints = Vec::new();

    let err = supervisor
        .sync_endpoints(&mut endpoints, &shutdown)
        .await
        .expect_err("shutdown should interrupt xray sync before stale teardown");
    let message = format!("{err:#}");
    assert!(
        message.contains("shutdown is in progress"),
        "expected shutdown-specific sync error, got: {message}"
    );

    let stale_instance = supervisor
        .instances
        .get_mut("stale-xray")
        .expect("stale instance should be preserved until final shutdown drain");
    assert!(
        stale_instance
            .child
            .try_wait()
            .expect("poll stale child after interrupted sync")
            .is_none(),
        "shutdown-interrupted sync should not tear down stale xray instances early"
    );
    let outcome = terminate_child_process(
        &mut stale_instance.child,
        Duration::from_millis(100),
        "stale-test-child",
    )
    .await;
    assert!(
        matches!(
            outcome,
            ChildTerminationOutcome::Graceful | ChildTerminationOutcome::Forced
        ),
        "test cleanup should terminate the preserved stale child"
    );
}

#[tokio::test]
pub(crate) async fn xray_supervisor_ensure_instance_creates_runtime_dir_for_validation_path() {
    let temp_root = make_temp_test_dir("xray-runtime-create");
    let runtime_dir = temp_root.join("nested/runtime");
    let mut supervisor = XraySupervisor::new(
        "/path/to/non-existent-xray".to_string(),
        runtime_dir.clone(),
    );
    let endpoint = ForwardProxyEndpoint {
        key: "xray-validation-test".to_string(),
        source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
        display_name: "xray-validation-test".to_string(),
        protocol: ForwardProxyProtocol::Vless,
        endpoint_url: None,
        raw_url: Some(
            "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                .to_string(),
        ),
    };
    let expected_config_path = runtime_dir.join(format!(
        "forward-proxy-{:016x}.json",
        stable_hash_u64(&endpoint.key)
    ));

    let err = supervisor
        .ensure_instance_with_ready_timeout(
            &endpoint,
            Duration::from_millis(50),
            &CancellationToken::new(),
        )
        .await
        .expect_err("non-existent xray binary should fail to start");
    let message = format!("{err:#}");
    assert!(
        runtime_dir.is_dir(),
        "runtime dir should be created before writing xray config"
    );
    assert!(
        message.contains("failed to start xray binary"),
        "expected startup failure after config write path is available, got: {message}"
    );
    assert!(
        !message.contains("failed to write xray config"),
        "runtime dir creation regression: {message}"
    );
    assert!(
        !expected_config_path.exists(),
        "spawn failure should clean temporary xray config file"
    );

    let _ = fs::remove_dir_all(&temp_root);
}

#[cfg(unix)]
#[tokio::test]
pub(crate) async fn probe_forward_proxy_endpoint_returns_none_when_shutdown_interrupts_temporary_xray_startup()
 {
    use std::os::unix::fs::PermissionsExt;

    let temp_root = make_temp_test_dir("xray-probe-startup-shutdown");
    let runtime_dir = temp_root.join("runtime");
    let fake_xray = temp_root.join("fake-xray.sh");
    fs::write(
        &fake_xray,
        "#!/bin/sh
sleep 30
",
    )
    .expect("write fake xray binary");
    let mut perms = fs::metadata(&fake_xray)
        .expect("read fake xray metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake_xray, perms).expect("chmod fake xray binary");

    let mut config = test_config();
    config.xray_binary = fake_xray.to_string_lossy().to_string();
    config.xray_runtime_dir = runtime_dir.clone();
    let state = test_state_from_config(config, false).await;
    let endpoint = ForwardProxyEndpoint {
        key: "xray-probe-startup-shutdown".to_string(),
        source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
        display_name: "xray-probe-startup-shutdown".to_string(),
        protocol: ForwardProxyProtocol::Vless,
        endpoint_url: None,
        raw_url: Some(
            "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                .to_string(),
        ),
    };
    let shutdown = CancellationToken::new();
    let cancel_shutdown = shutdown.clone();
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel_shutdown.cancel();
    });

    let result = probe_forward_proxy_endpoint(
        state.as_ref(),
        &endpoint,
        Duration::from_secs(5),
        Some(&shutdown),
    )
    .await
    .expect("shutdown-interrupted xray startup should normalize to a skipped probe");

    cancel_task
        .await
        .expect("shutdown trigger task should finish");
    assert!(
        result.is_none(),
        "shutdown-interrupted temporary xray startup should be treated as a skipped probe"
    );
    assert!(
        state.xray_supervisor.lock().await.instances.is_empty(),
        "temporary xray validation instances should be cleaned up after shutdown"
    );
    if runtime_dir.exists() {
        let mut runtime_entries = fs::read_dir(&runtime_dir)
            .expect("read temporary xray runtime dir after shutdown")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect temporary xray runtime dir entries");
        runtime_entries.retain(|entry| entry.path().is_file());
        assert!(
            runtime_entries.is_empty(),
            "temporary xray shutdown path should not leave runtime files behind"
        );
    }

    cleanup_temp_test_dir(&temp_root);
}

#[tokio::test]
pub(crate) async fn forward_proxy_settings_returns_service_unavailable_when_shutdown_interrupts_xray_sync()
 {
    let temp_root = make_temp_test_dir("forward-proxy-settings-shutdown");
    let runtime_dir = temp_root.join("runtime");
    let xray_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#vless".to_string();
    let normalized_proxy = normalize_single_proxy_url(&xray_url).expect("normalize xray proxy url");
    let proxy_key = normalize_single_proxy_key(&xray_url).expect("normalize xray proxy key");

    let mut config = test_config();
    config.xray_binary = "/path/to/non-existent-xray".to_string();
    config.xray_runtime_dir = runtime_dir;
    let state = test_state_from_config(config, false).await;
    state.shutdown.cancel();

    let err = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec![xray_url],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect_err("settings update should surface shutdown interruption");

    assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        err.1.contains("interrupted by shutdown"),
        "expected shutdown-specific error, got: {}",
        err.1
    );
    assert!(
        read_forward_proxy_runtime_weight(&state.pool, &proxy_key)
            .await
            .is_none(),
        "shutdown-interrupted route sync should not persist a partial runtime snapshot"
    );
    let saved_settings = load_forward_proxy_settings(&state.pool)
        .await
        .expect("read forward proxy settings after shutdown interruption");
    assert!(
        !saved_settings.proxy_urls.contains(&normalized_proxy),
        "shutdown-interrupted settings update should not persist the new proxy configuration"
    );
    let manager = state.forward_proxy.lock().await;
    assert!(
        !manager.settings.proxy_urls.contains(&normalized_proxy),
        "shutdown-interrupted settings update should roll back the in-memory proxy configuration"
    );
}

#[tokio::test]
pub(crate) async fn forward_proxy_settings_triggers_async_bootstrap_probe_for_added_manual_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let success_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;

    let Json(updated) = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec![proxy_url],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");
    assert!(updated.proxy_urls.contains(&normalized_proxy));

    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let success_count =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;
    assert!(
        success_count > success_count_before,
        "expected at least one successful bootstrap probe attempt"
    );

    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn forward_proxy_settings_does_not_probe_when_no_new_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let request = ForwardProxySettingsUpdateRequest {
        proxy_urls: vec![proxy_url.clone()],
        subscription_urls: Vec::new(),
        subscription_update_interval_secs: 3600,
        insert_direct: true,
    };
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: request.proxy_urls.clone(),
            subscription_urls: request.subscription_urls.clone(),
            subscription_update_interval_secs: request.subscription_update_interval_secs,
            insert_direct: request.insert_direct,
        }),
    )
    .await
    .expect("initial put forward proxy settings should succeed");
    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let first_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;

    let _ = put_forward_proxy_settings(State(state.clone()), HeaderMap::new(), Json(request))
        .await
        .expect("repeated put forward proxy settings should succeed");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let second_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    assert_eq!(
        second_count, first_count,
        "no newly added endpoint should not trigger extra bootstrap probe"
    );

    proxy_handle.abort();
}

#[tokio::test]
pub(crate) async fn forward_proxy_settings_does_not_reprobe_when_subscription_is_unchanged() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url.clone()],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("initial put forward proxy settings should succeed");
    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let first_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("repeated put forward proxy settings should succeed");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let second_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    assert_eq!(
        second_count, first_count,
        "unchanged subscription endpoints should not trigger extra bootstrap probes"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

use super::*;
