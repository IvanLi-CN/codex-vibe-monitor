use super::*;

mod shutdown_rollback;
mod subscription_snapshot_invalidation;
mod subscription_snapshot_refresh;
mod weight_algorithms;

#[tokio::test]
async fn refresh_forward_proxy_subscriptions_triggers_bootstrap_probe_for_added_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let settings = ForwardProxySettings {
        proxy_urls: Vec::new(),
        subscription_urls: vec![subscription_url],
        subscription_update_interval_secs: 3600,
        insert_direct: true,
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("persist subscription settings");

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
    }
    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("sync forward proxy routes before subscription refresh");
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let success_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;

    refresh_forward_proxy_subscriptions(state.clone(), true, None)
        .await
        .expect("refresh subscriptions should succeed");
    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let success_count =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;
    assert!(
        success_count > success_count_before,
        "expected at least one successful bootstrap probe attempt from subscription refresh"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn refresh_forward_proxy_subscriptions_skips_probe_for_known_subscription_keys() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let settings = ForwardProxySettings {
        proxy_urls: Vec::new(),
        subscription_urls: vec![subscription_url],
        subscription_update_interval_secs: 3600,
        insert_direct: true,
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("persist subscription settings");

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
    }
    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("sync forward proxy routes before subscription refresh");

    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let known_keys = HashSet::from([proxy_key.clone()]);
    refresh_forward_proxy_subscriptions(state.clone(), true, Some(known_keys))
        .await
        .expect("refresh subscriptions should succeed");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let probe_count_after = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    assert_eq!(
        probe_count_after, probe_count_before,
        "known subscription keys should suppress startup-style reprobe"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn forward_proxy_settings_bootstrap_probe_failure_penalizes_runtime_weight() {
    let (proxy_url, proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let failure_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(false)).await;

    let _ = put_forward_proxy_settings(
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

    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let failure_count =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(false)).await;
    assert!(
        failure_count > failure_count_before,
        "expected at least one failed bootstrap probe attempt"
    );

    let runtime_weight = read_forward_proxy_runtime_weight(&state.pool, &proxy_key)
        .await
        .expect("runtime weight should exist");
    assert!(
        runtime_weight < 1.0,
        "expected failed bootstrap probe to penalize runtime weight; got {runtime_weight}"
    );

    proxy_handle.abort();
}
