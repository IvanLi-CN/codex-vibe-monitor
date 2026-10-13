use super::*;

#[tokio::test]
async fn runtime_routing_replays_confirmed_facts_and_deduplicates_after_commit() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("fixture URL")).await;
    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    let account = insert_test_pool_api_key_account(&state, "Replay Route", "fixture-token").await;
    let runtime = runtime_route_facts(&state).await.expect("runtime");
    let pause = runtime.pause_materialization_for_test().await;
    record_runtime_pool_route_success(
        &state,
        account,
        Utc::now(),
        Some("replay-sticky"),
        None,
        None,
        "/v1/responses",
        ImageIntent::No,
        None,
        None,
        Some(0),
        Some("gpt-test"),
    )
    .await
    .expect("durably accept routing outcome");
    let bytes = std::fs::read(
        state
            .config
            .proxy_raw_dir
            .join("route-journal/proxy-route-facts-v1.events"),
    )
    .expect("read fixture journal");
    assert!(
        !String::from_utf8_lossy(&bytes).contains("replay-sticky"),
        "conversation identifiers must be encrypted"
    );
    drop(pause);
    runtime.shutdown_and_sync().await;
    state.upstream_accounts.route_facts.lock().await.take();
    let restored = runtime_route_facts(&state)
        .await
        .expect("restore synced routing journal");
    let pause = restored.pause_materialization_for_test().await;
    assert_eq!(
        load_runtime_sticky_route(&state, "replay-sticky", Some("gpt-test"))
            .await
            .expect("restored projection")
            .0
            .map(|route| route.account_id),
        Some(account)
    );
    assert_eq!(restored.health_snapshot().replayed, 1);
    drop(pause);
    restored
        .flush_for_test(&state.pool)
        .await
        .expect("materialize restored fact");
    restored
        .flush_for_test(&state.pool)
        .await
        .expect("duplicate flush");
    let actions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id=?1 AND action='route_recovered'")
        .bind(account).fetch_one(&state.pool).await.expect("inspect recovery identity");
    assert_eq!(actions, 1);
    assert_eq!(restored.health_snapshot().pending_events, 0);
    restored.shutdown_and_sync().await;
    state.shutdown.cancel();
}

#[tokio::test]
async fn runtime_sticky_clear_preserves_a_reassigned_owner() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("fixture URL")).await;
    let failed = insert_test_pool_api_key_account(&state, "Failed Owner", "fixture-a").await;
    let owner = insert_test_pool_api_key_account(&state, "New Owner", "fixture-b").await;
    upsert_sticky_route(&state.pool, "new-owner", owner, &format_utc_iso(Utc::now()))
        .await
        .expect("seed new owner");
    record_runtime_pool_route_http_failure(
        &state,
        failed,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        true,
        Some("new-owner"),
        StatusCode::TOO_MANY_REQUESTS,
        "fixture too many requests",
        None,
        "/v1/responses",
        ImageIntent::No,
        None,
        None,
        None,
        Some("gpt-test"),
    )
    .await
    .expect("record old-owner failure");
    assert_eq!(
        load_runtime_sticky_route(&state, "new-owner", Some("gpt-test"))
            .await
            .expect("binding")
            .0
            .map(|route| route.account_id),
        Some(owner)
    );
    if let Some(runtime) = state.upstream_accounts.route_facts.lock().await.clone() {
        runtime.shutdown_and_sync().await;
    }
    state.shutdown.cancel();
}

#[tokio::test]
async fn runtime_pending_sticky_is_fenced_by_control_aba() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("fixture URL")).await;
    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    let account = insert_test_pool_api_key_account(&state, "Control Fence", "fixture-token").await;
    let runtime = runtime_route_facts(&state)
        .await
        .expect("initialize owned runtime");
    let _pause = runtime.pause_materialization_for_test().await;
    record_runtime_pool_route_success(
        &state,
        account,
        Utc::now(),
        Some("control-aba"),
        None,
        None,
        "/v1/responses",
        ImageIntent::No,
        None,
        None,
        Some(0),
        None,
    )
    .await
    .expect("accept sticky");
    sqlx::query("UPDATE pool_upstream_accounts SET enabled=0 WHERE id=?1")
        .bind(account)
        .execute(&state.pool)
        .await
        .expect("disable fixture");
    sqlx::query("UPDATE pool_upstream_accounts SET enabled=1 WHERE id=?1")
        .bind(account)
        .execute(&state.pool)
        .await
        .expect("reenable fixture");
    let (sticky, _) = load_runtime_sticky_route(&state, "control-aba", None)
        .await
        .expect("read fenced projection");
    assert!(
        sticky.is_none(),
        "pre-control sticky must not revive after disable/enable"
    );
    state.shutdown.cancel();
}

#[tokio::test]
async fn runtime_model_failure_is_visible_without_sqlite_writes() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("fixture URL")).await;
    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    let account = insert_test_pool_api_key_account(&state, "Runtime Model", "fixture-token").await;
    let mut lock = state.pool.acquire().await.expect("fixture writer");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(lock.as_mut())
        .await
        .expect("hold fixture writer");
    for _ in 0..5 {
        tokio::time::timeout(
            Duration::from_millis(250),
            record_runtime_pool_route_http_failure(
                &state,
                account,
                UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
                false,
                None,
                StatusCode::SERVICE_UNAVAILABLE,
                "fixture service unavailable",
                None,
                "/v1/responses",
                ImageIntent::No,
                None,
                None,
                None,
                Some("gpt-test"),
            ),
        )
        .await
        .expect("confirmation bypasses database writer")
        .expect("journal accepts failure");
    }
    let penalties = load_runtime_model_penalties(&state, &[account], Some("gpt-test"))
        .await
        .expect("read memory model state");
    assert_eq!(penalties.get(&account), Some(&ModelRoutePenalty::Excluded));
    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_model_routes WHERE account_id=?1",
    )
    .bind(account)
    .fetch_one(&state.pool)
    .await
    .expect("inspect durable model facts");
    assert_eq!(stored, 0);
    sqlx::query("ROLLBACK")
        .execute(lock.as_mut())
        .await
        .expect("release owned writer");
    state.shutdown.cancel();
}

#[tokio::test]
async fn runtime_sticky_success_is_visible_while_sqlite_materialization_is_locked() {
    let state =
        test_state_with_openai_base(Url::parse("http://127.0.0.1:9").expect("local fixture URL"))
            .await;
    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    let account_id =
        insert_test_pool_api_key_account(&state, "Runtime Sticky", "fixture-token").await;
    let initial = resolve_pool_account_for_request(
        state.as_ref(),
        Some("runtime-sticky"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve initial account");
    assert!(matches!(initial, PoolAccountResolution::Resolved(_)));
    let mut blocker = state.pool.acquire().await.expect("acquire fixture writer");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(blocker.as_mut())
        .await
        .expect("hold materialization lock");

    tokio::time::timeout(
        Duration::from_millis(250),
        record_runtime_pool_route_success(
            state.as_ref(),
            account_id,
            Utc::now(),
            Some("runtime-sticky"),
            None,
            Some("runtime-sticky-invoke"),
            "/v1/responses",
            ImageIntent::No,
            None,
            None,
            Some(0),
            None,
        ),
    )
    .await
    .expect("route confirmation must not wait for SQLite")
    .expect("accept route observation");
    let stored: Option<i64> =
        sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key=?1")
            .bind("runtime-sticky")
            .fetch_optional(&state.pool)
            .await
            .expect("inspect unmaterialized fixture");
    assert!(
        stored.is_none(),
        "SQLite remains unchanged while the writer is locked"
    );
    let next = resolve_pool_account_for_request(
        state.as_ref(),
        Some("runtime-sticky"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve using accepted runtime fact");
    let PoolAccountResolution::Resolved(next) = next else {
        panic!("accepted runtime sticky must remain routable");
    };
    assert_eq!(next.account_id, account_id);
    assert_eq!(next.routing_source, PoolRoutingSelectionSource::StickyReuse);
    sqlx::query("ROLLBACK")
        .execute(blocker.as_mut())
        .await
        .expect("release owned fixture lock");
    state.shutdown.cancel();
}
