use super::*;

mod shutdown_rollback;
mod subscription_snapshot_invalidation;
mod subscription_snapshot_refresh;
mod weight_algorithms;

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
async fn concurrent_subscription_refreshes_serialize_fetch_and_apply() {
    let request_count = Arc::new(AtomicUsize::new(0));
    let (first_request_started_tx, mut first_request_started_rx) =
        tokio::sync::watch::channel(false);
    let first_fetch_active = Arc::new(AtomicBool::new(false));
    let second_lock_acquired_while_first_active = Arc::new(AtomicBool::new(false));
    let release_first_request = Arc::new(Notify::new());
    let app = Router::new().route(
        "/subscription",
        get({
            let request_count = request_count.clone();
            let first_request_started = first_request_started_tx.clone();
            let first_fetch_active = first_fetch_active.clone();
            let release_first_request = release_first_request.clone();
            move || {
                let request_index = request_count.fetch_add(1, Ordering::SeqCst);
                let first_request_started = first_request_started.clone();
                let first_fetch_active = first_fetch_active.clone();
                let release_first_request = release_first_request.clone();
                async move {
                    if request_index == 0 {
                        first_fetch_active.store(true, Ordering::SeqCst);
                        first_request_started.send_replace(true);
                        release_first_request.notified().await;
                        first_fetch_active.store(false, Ordering::SeqCst);
                        return (StatusCode::OK, "http://127.0.0.1:7890\n");
                    }
                    (StatusCode::OK, "http://127.0.0.1:7891\n")
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind serialized subscription source");
    let addr = listener
        .local_addr()
        .expect("serialized subscription source addr");
    let source_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serialized subscription source should run");
    });
    let subscription_url = format!("http://{addr}/subscription");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let settings = ForwardProxySettings {
        subscription_urls: vec![subscription_url],
        ..ForwardProxySettings::default()
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("save serialized subscription settings");
    state
        .forward_proxy
        .lock()
        .await
        .apply_settings(settings.clone());

    let first_started = async {
        while !*first_request_started_rx.borrow() {
            first_request_started_rx
                .changed()
                .await
                .expect("first refresh signal should remain open");
        }
    };
    let first_refresh = tokio::spawn(refresh_forward_proxy_subscriptions(
        state.clone(),
        true,
        None,
    ));
    tokio::time::timeout(Duration::from_secs(1), first_started)
        .await
        .expect("first refresh should reach the subscription source");

    let (second_refresh_lock_attempted_tx, second_refresh_lock_attempted_rx) =
        tokio::sync::oneshot::channel();
    install_forward_proxy_subscription_refresh_lock_attempt_hook(
        &state,
        second_refresh_lock_attempted_tx,
    );
    install_forward_proxy_subscription_refresh_lock_acquired_hook(
        &state,
        first_fetch_active,
        second_lock_acquired_while_first_active.clone(),
    );
    let second_refresh_state = state.clone();
    let second_refresh = tokio::spawn(refresh_forward_proxy_subscriptions(
        second_refresh_state,
        true,
        None,
    ));
    tokio::time::timeout(Duration::from_secs(1), second_refresh_lock_attempted_rx)
        .await
        .expect(
            "second refresh should attempt the shared lock before the first refresh is released",
        )
        .expect("second refresh lock-attempt signal should remain open");

    release_first_request.notify_one();
    first_refresh
        .await
        .expect("first refresh task should join")
        .expect("first refresh should succeed");
    second_refresh
        .await
        .expect("second refresh task should join")
        .expect("second refresh should succeed");

    assert!(
        !second_lock_acquired_while_first_active.load(Ordering::SeqCst),
        "a second refresh must not acquire the shared lock while the first fetch is active"
    );
    assert_eq!(request_count.load(Ordering::SeqCst), 2);
    let restored_subscription_sources = {
        state
            .forward_proxy
            .lock()
            .await
            .settings
            .subscription_urls
            .clone()
    };
    assert_eq!(
        load_forward_proxy_subscription_snapshot(&state.pool, &restored_subscription_sources)
            .await
            .expect("load serialized subscription snapshot"),
        vec!["http://127.0.0.1:7891"],
        "the newer refresh must own the durable snapshot"
    );
    assert_eq!(
        state.forward_proxy.lock().await.subscription_proxy_urls,
        vec!["http://127.0.0.1:7891"],
        "the newer refresh must own the active subscription nodes"
    );

    source_handle.abort();
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

    let Err((status, detail)) =
        post_forward_proxy_refresh_subscriptions(State(state.clone()), HeaderMap::new()).await
    else {
        panic!("a partial subscription refresh must retain the existing HTTP error contract");
    };
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(
        detail,
        "proxy_connect_error: proxy connection failed during validation"
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

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(original_settings.clone());
        manager.restore_subscription_urls(vec!["http://127.0.0.1:7890".to_string()]);
    }
    assert_eq!(
        snapshot_active_forward_proxy_endpoints(&*state.forward_proxy.lock().await)
            .into_iter()
            .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
            .count(),
        1,
        "the original subscription node should be active before source removal"
    );

    let Json(_) = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: Vec::new(),
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("remove subscription source");

    assert!(
        snapshot_active_forward_proxy_endpoints(&*state.forward_proxy.lock().await)
            .into_iter()
            .all(|endpoint| endpoint.source != FORWARD_PROXY_SOURCE_SUBSCRIPTION),
        "changing subscription sources must clear active derived nodes"
    );
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

#[tokio::test]
async fn malformed_or_mismatched_subscription_snapshot_is_ignored() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let settings = ForwardProxySettings {
        subscription_urls: vec!["https://subscription.example/feed".to_string()],
        ..ForwardProxySettings::default()
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("save subscription settings");

    sqlx::query("UPDATE forward_proxy_settings SET subscription_snapshot_json = ?1 WHERE id = 1")
        .bind("{malformed")
        .execute(&state.pool)
        .await
        .expect("save malformed subscription snapshot");
    assert!(
        load_forward_proxy_subscription_snapshot(&state.pool, &settings.subscription_urls)
            .await
            .expect("load malformed snapshot")
            .is_empty(),
        "malformed subscription snapshots must not restore endpoints"
    );

    sqlx::query(
        "UPDATE forward_proxy_settings SET subscription_snapshot_json = ?1 WHERE id = 1",
    )
    .bind(
        r#"{"subscription_urls":["https://different.example/feed"],"proxy_urls":["http://127.0.0.1:7890"]}"#,
    )
    .execute(&state.pool)
    .await
    .expect("save mismatched subscription snapshot");
    assert!(
        load_forward_proxy_subscription_snapshot(&state.pool, &settings.subscription_urls)
            .await
            .expect("load mismatched snapshot")
            .is_empty(),
        "snapshots for another source list must not restore endpoints"
    );

    sqlx::query(
        "UPDATE forward_proxy_settings SET subscription_snapshot_json = ?1 WHERE id = 1",
    )
    .bind(
        r#"{"subscription_urls":["https://subscription.example/feed"],"proxy_urls":["http://127.0.0.1:7890","not-a-proxy"]}"#,
    )
    .execute(&state.pool)
    .await
    .expect("save snapshot with an invalid endpoint");
    assert!(
        load_forward_proxy_subscription_snapshot(&state.pool, &settings.subscription_urls)
            .await
            .expect("load snapshot with invalid endpoint")
            .is_empty(),
        "a snapshot with any invalid endpoint must be ignored as a whole"
    );
}

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
