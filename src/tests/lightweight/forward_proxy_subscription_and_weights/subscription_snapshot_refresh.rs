use super::*;

#[tokio::test]
async fn cached_subscription_nodes_survive_restart_and_failed_refresh() {
    let (subscription_url, subscription_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let settings = ForwardProxySettings {
        proxy_urls: Vec::new(),
        subscription_urls: vec![subscription_url.clone()],
        subscription_update_interval_secs: 3600,
        insert_direct: true,
    };
    let cached_proxy_url = "http://127.0.0.1:7890".to_string();

    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("save subscription settings");
    save_forward_proxy_subscription_snapshot(
        &state.pool,
        &settings.subscription_urls,
        std::slice::from_ref(&cached_proxy_url),
    )
    .await
    .expect("save last successful subscription result");

    let restored_settings = load_forward_proxy_settings(&state.pool)
        .await
        .expect("load subscription settings");
    let restored_subscription_urls =
        load_forward_proxy_subscription_snapshot(&state.pool, &restored_settings.subscription_urls)
            .await
            .expect("load last successful subscription result");
    let runtime_rows = load_forward_proxy_runtime_states(&state.pool)
        .await
        .expect("load proxy runtime state");
    let mut restarted_manager = ForwardProxyManager::new(restored_settings, runtime_rows);
    restarted_manager.restore_subscription_urls(restored_subscription_urls);
    *state.forward_proxy.lock().await = restarted_manager;

    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("restore routes before refreshing subscriptions");
    assert_eq!(
        snapshot_active_forward_proxy_endpoints(&*state.forward_proxy.lock().await).len(),
        1,
        "the previous subscription node should be routable immediately after restart"
    );

    assert!(
        refresh_forward_proxy_subscriptions(state.clone(), true, None)
            .await
            .is_err(),
        "the fixture subscription must fail to refresh"
    );
    assert_eq!(
        snapshot_active_forward_proxy_endpoints(&*state.forward_proxy.lock().await).len(),
        1,
        "a failed refresh must retain the restored subscription node"
    );
    assert_eq!(
        load_forward_proxy_subscription_snapshot(&state.pool, &settings.subscription_urls)
            .await
            .expect("load retained subscription snapshot"),
        vec![cached_proxy_url],
        "a failed refresh must retain the durable subscription snapshot"
    );

    subscription_handle.abort();
}

#[tokio::test]
async fn successful_empty_subscription_refresh_clears_cached_nodes() {
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(String::new()).await;
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
    let cached_proxy_url = "http://127.0.0.1:7890".to_string();

    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("save subscription settings");
    assert!(
        save_forward_proxy_subscription_snapshot(
            &state.pool,
            &settings.subscription_urls,
            std::slice::from_ref(&cached_proxy_url),
        )
        .await
        .expect("save last successful subscription result")
    );
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings.clone());
        manager.restore_subscription_urls(vec![cached_proxy_url]);
    }

    refresh_forward_proxy_subscriptions(state.clone(), true, None)
        .await
        .expect("empty subscription refresh should succeed");
    assert!(
        snapshot_active_forward_proxy_endpoints(&*state.forward_proxy.lock().await).is_empty(),
        "an empty successful refresh should remove the cached endpoint immediately"
    );
    assert!(
        load_forward_proxy_subscription_snapshot(&state.pool, &settings.subscription_urls)
            .await
            .expect("load cleared snapshot")
            .is_empty(),
        "an empty successful refresh should clear the durable snapshot"
    );

    subscription_handle.abort();
}

#[tokio::test]
async fn partial_subscription_refresh_failure_preserves_cached_nodes() {
    let (empty_subscription_url, empty_subscription_handle) =
        spawn_test_subscription_source(String::new()).await;
    let (failed_subscription_url, failed_subscription_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let settings = ForwardProxySettings {
        proxy_urls: Vec::new(),
        subscription_urls: vec![empty_subscription_url, failed_subscription_url],
        subscription_update_interval_secs: 3600,
        insert_direct: true,
    };
    let cached_proxy_url = "http://127.0.0.1:7890".to_string();
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("save subscription settings");
    assert!(
        save_forward_proxy_subscription_snapshot(
            &state.pool,
            &settings.subscription_urls,
            std::slice::from_ref(&cached_proxy_url),
        )
        .await
        .expect("save last successful subscription result")
    );
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings.clone());
        manager.restore_subscription_urls(vec![cached_proxy_url.clone()]);
    }

    assert!(
        refresh_forward_proxy_subscriptions(state.clone(), true, None)
            .await
            .is_err(),
        "a partial subscription refresh must be reported as incomplete"
    );
    assert_eq!(
        snapshot_active_forward_proxy_endpoints(&*state.forward_proxy.lock().await).len(),
        1,
        "an incomplete refresh must preserve the previous active node"
    );
    assert_eq!(
        load_forward_proxy_subscription_snapshot(&state.pool, &settings.subscription_urls)
            .await
            .expect("load retained snapshot"),
        vec![cached_proxy_url],
        "an incomplete refresh must preserve the durable snapshot"
    );

    empty_subscription_handle.abort();
    failed_subscription_handle.abort();
}
