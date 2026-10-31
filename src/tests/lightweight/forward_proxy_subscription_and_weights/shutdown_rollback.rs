use super::*;

async fn seed_subscription_state(
    state: &crate::AppState,
    settings: &ForwardProxySettings,
    cached_proxy_url: &str,
) {
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("save previous subscription settings");
    assert!(
        save_forward_proxy_subscription_snapshot(
            &state.pool,
            &settings.subscription_urls,
            &[cached_proxy_url.to_string()],
        )
        .await
        .expect("save previous subscription snapshot")
    );
    let mut manager = state.forward_proxy.lock().await;
    manager.apply_settings(settings.clone());
    manager.restore_subscription_urls(vec![cached_proxy_url.to_string()]);
}

fn update_request(settings: ForwardProxySettings) -> ForwardProxySettingsUpdateRequest {
    ForwardProxySettingsUpdateRequest {
        proxy_urls: settings.proxy_urls,
        subscription_urls: settings.subscription_urls,
        subscription_update_interval_secs: settings.subscription_update_interval_secs,
        insert_direct: settings.insert_direct,
    }
}

fn test_xray_url() -> String {
    "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#vless".to_string()
}

#[tokio::test]
async fn shutdown_interruption_restores_subscription_settings_and_snapshot() {
    let mut config = test_config();
    config.xray_binary = "/path/to/non-existent-xray".to_string();
    let state = test_state_from_config(config, false).await;
    let previous_settings = ForwardProxySettings {
        proxy_urls: Vec::new(),
        subscription_urls: vec!["https://old-subscription.invalid/feed".to_string()],
        subscription_update_interval_secs: 3600,
        insert_direct: true,
    };
    let cached_proxy_url = "http://127.0.0.1:7890";
    seed_subscription_state(&state, &previous_settings, cached_proxy_url).await;
    let next_settings = ForwardProxySettings {
        proxy_urls: vec![test_xray_url()],
        subscription_urls: vec!["https://new-subscription.invalid/feed".to_string()],
        ..previous_settings.clone()
    };
    state.shutdown.cancel();

    let err = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(update_request(next_settings)),
    )
    .await
    .expect_err("shutdown should interrupt route synchronization");

    assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    let saved_settings = load_forward_proxy_settings(&state.pool)
        .await
        .expect("load rolled-back settings");
    assert_eq!(
        saved_settings.subscription_urls,
        previous_settings.subscription_urls
    );
    assert_eq!(
        load_forward_proxy_subscription_snapshot(&state.pool, &previous_settings.subscription_urls)
            .await
            .expect("load restored subscription snapshot"),
        vec![cached_proxy_url]
    );
    let manager = state.forward_proxy.lock().await;
    assert_eq!(
        manager.settings.subscription_urls,
        previous_settings.subscription_urls
    );
    assert_eq!(manager.subscription_proxy_urls, vec![cached_proxy_url]);
}

#[tokio::test]
async fn failed_snapshot_rollback_keeps_persisted_and_active_settings_aligned() {
    let mut config = test_config();
    config.xray_binary = "/path/to/non-existent-xray".to_string();
    let state = test_state_from_config(config, false).await;
    let previous_settings = ForwardProxySettings {
        proxy_urls: Vec::new(),
        subscription_urls: vec!["https://old-subscription.invalid/feed".to_string()],
        subscription_update_interval_secs: 3600,
        insert_direct: true,
    };
    seed_subscription_state(&state, &previous_settings, "http://127.0.0.1:7890").await;
    let next_settings = ForwardProxySettings {
        proxy_urls: vec![test_xray_url()],
        subscription_urls: vec!["https://new-subscription.invalid/feed".to_string()],
        ..previous_settings
    };
    sqlx::query(
        r#"
        CREATE TRIGGER fail_subscription_snapshot_restore
        BEFORE UPDATE OF subscription_snapshot_json ON forward_proxy_settings
        WHEN NEW.subscription_snapshot_json != '{}'
        BEGIN
            SELECT RAISE(ABORT, 'injected snapshot restore failure');
        END
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("install deterministic snapshot rollback failure");
    state.shutdown.cancel();

    let err = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(update_request(next_settings.clone())),
    )
    .await
    .expect_err("snapshot restoration failure should be reported");

    assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(err.1.contains("atomically roll back"));
    let saved_settings = load_forward_proxy_settings(&state.pool)
        .await
        .expect("load settings after atomic rollback failure");
    assert_eq!(
        saved_settings.subscription_urls,
        next_settings.subscription_urls
    );
    assert!(
        load_forward_proxy_subscription_snapshot(&state.pool, &next_settings.subscription_urls)
            .await
            .expect("load snapshot after atomic rollback failure")
            .is_empty(),
        "a failed rollback transaction must leave the committed settings without a stale snapshot"
    );
    let manager = state.forward_proxy.lock().await;
    assert_eq!(
        manager.settings.subscription_urls,
        next_settings.subscription_urls
    );
    assert_eq!(manager.settings.proxy_urls, next_settings.proxy_urls);
    assert!(manager.subscription_proxy_urls.is_empty());
}
