use super::*;

#[tokio::test]
async fn changing_subscription_sources_clears_the_cached_snapshot() {
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source("http://127.0.0.1:7890\n".to_string()).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let original_settings = ForwardProxySettings {
        proxy_urls: Vec::new(),
        subscription_urls: vec![subscription_url.clone()],
        subscription_update_interval_secs: 3600,
        insert_direct: true,
    };
    save_forward_proxy_settings(&state.pool, original_settings.clone())
        .await
        .expect("save original subscription settings");
    assert!(
        save_forward_proxy_subscription_snapshot(
            &state.pool,
            &original_settings.subscription_urls,
            &["http://127.0.0.1:7890".to_string()],
        )
        .await
        .expect("save subscription snapshot")
    );

    save_forward_proxy_settings(
        &state.pool,
        ForwardProxySettings {
            subscription_urls: Vec::new(),
            ..original_settings.clone()
        },
    )
    .await
    .expect("remove subscription source");

    assert!(
        load_forward_proxy_subscription_snapshot(&state.pool, &original_settings.subscription_urls)
            .await
            .expect("load removed subscription snapshot")
            .is_empty(),
        "re-adding a removed subscription must not resurrect its previous cache"
    );

    subscription_handle.abort();
}

#[tokio::test]
async fn old_forward_proxy_settings_schema_gains_subscription_snapshot_column() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    sqlx::query("DROP TRIGGER trg_forward_proxy_settings_clear_subscription_snapshot")
        .execute(&state.pool)
        .await
        .expect("remove the snapshot invalidation added by the current schema");
    sqlx::query("ALTER TABLE forward_proxy_settings DROP COLUMN subscription_snapshot_json")
        .execute(&state.pool)
        .await
        .expect("simulate the pre-snapshot schema");

    ensure_schema(&state.pool)
        .await
        .expect("upgrade the existing forward proxy settings table");
    let snapshot_json = sqlx::query_scalar::<_, String>(
        "SELECT subscription_snapshot_json FROM forward_proxy_settings WHERE id = 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("read migrated subscription snapshot default");
    assert_eq!(snapshot_json, "{}");

    let settings = ForwardProxySettings {
        subscription_urls: vec!["https://subscription.invalid/feed".to_string()],
        ..ForwardProxySettings::default()
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("save subscription settings");
    assert!(
        save_forward_proxy_subscription_snapshot(
            &state.pool,
            &settings.subscription_urls,
            &["http://127.0.0.1:7890".to_string()],
        )
        .await
        .expect("save subscription snapshot")
    );
    sqlx::query("UPDATE forward_proxy_settings SET subscription_urls_json = '[]' WHERE id = 1")
        .execute(&state.pool)
        .await
        .expect("simulate an older reader changing subscription settings");
    let snapshot_json = sqlx::query_scalar::<_, String>(
        "SELECT subscription_snapshot_json FROM forward_proxy_settings WHERE id = 1",
    )
    .fetch_one(&state.pool)
    .await
    .expect("read invalidated subscription snapshot");
    assert_eq!(snapshot_json, "{}");
}
