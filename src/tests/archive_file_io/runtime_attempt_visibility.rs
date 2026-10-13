use super::*;

#[tokio::test]
async fn terminal_attempt_stays_visible_until_sqlite_commit() {
    let mut state = test_state_from_config(test_config(), true).await;
    let mutable = Arc::get_mut(&mut state).expect("fixture has one state owner");
    mutable.sqlite_batch_writer = SqliteBatchWriter::spawn(
        mutable.pool.clone(),
        mutable.shutdown.clone(),
        mutable.prompt_cache_conversation_cache.clone(),
        mutable.pricing_catalog.clone(),
        &mutable.config.database_path,
    )
    .expect("production file journal");
    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    let account = insert_test_pool_api_key_account(&state, "Visibility", "fixture-token").await;
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: "terminal-visibility".to_string(),
        occurred_at: "2026-10-13 12:00:00".to_string(),
        endpoint: "/v1/responses".to_string(),
        sticky_key: None,
        requester_ip: None,
        upstream_base_url_host: None,
        request_model: None,
    };
    let attempt = begin_pool_upstream_request_attempt_in_memory(
        &state,
        &trace,
        None,
        None,
        None,
        None,
        None,
        account,
        "https://example.invalid/",
        1,
        1,
        0,
        &trace.occurred_at,
    )
    .await
    .expect("admit runtime attempt");
    finalize_pool_upstream_request_attempt_in_memory(
        &state,
        &attempt,
        "2026-10-13 12:00:01",
        "success",
        Some(StatusCode::OK),
        Some(StatusCode::OK),
        None,
        None,
        None,
        Some(1.0),
        Some(2.0),
        Some(3.0),
        None,
        None,
        None,
    )
    .await
    .expect("finalize attempt in memory");
    let mut locked = state
        .pool
        .acquire()
        .await
        .expect("own SQLite pressure connection");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *locked)
        .await
        .expect("hold writer lock");
    persist_and_broadcast_proxy_capture_terminal_record(
        &state,
        test_proxy_capture_record(&trace.invoke_id, &trace.occurred_at),
    )
    .await
    .expect("confirm terminal journal without committing SQLite");
    let Json(before) = fetch_invocation_pool_attempts(
        State(state.clone()),
        axum::extract::Path(trace.invoke_id.clone()),
    )
    .await
    .expect("read unmaterialized attempt through HTTP handler");
    assert_eq!(
        before.len(),
        1,
        "confirmed final attempts remain visible before commit"
    );
    assert_eq!(
        before[0].attempt_id,
        attempt
            .attempt_public_id
            .clone()
            .expect("stable attempt id")
    );
    assert_eq!(
        before[0].finished_at.as_deref(),
        Some("2026-10-13 12:00:01")
    );
    let pause = state
        .proxy_runtime_attempts
        .pause_queries_after_sql_for_test()
        .await;
    let queries_before = state
        .proxy_runtime_attempts
        .query_after_sql_count_for_test();
    let racing_reader = tokio::spawn({
        let state = state.clone();
        let id = trace.invoke_id.clone();
        async move { fetch_invocation_pool_attempts(State(state), axum::extract::Path(id)).await }
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while state
            .proxy_runtime_attempts
            .query_after_sql_count_for_test()
            == queries_before
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("reader fetched the pre-commit SQLite snapshot");
    sqlx::query("ROLLBACK")
        .execute(&mut *locked)
        .await
        .expect("release writer lock");
    drop(locked);
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("commit terminal fact");
    assert_eq!(state.proxy_runtime_attempts.health_snapshot().records, 0);
    drop(pause);
    let Json(raced) = racing_reader
        .await
        .expect("join concurrent reader")
        .expect("read across commit");
    assert_eq!(
        raced.len(),
        1,
        "a query started before commit must retain the final snapshot"
    );
    assert_eq!(raced[0].attempt_id, before[0].attempt_id);
    let Json(after) = fetch_invocation_pool_attempts(
        State(state.clone()),
        axum::extract::Path(trace.invoke_id.clone()),
    )
    .await
    .expect("read committed attempt");
    assert_eq!(
        after.len(),
        1,
        "database and memory must not duplicate an attempt"
    );
    assert_eq!(after[0].attempt_id, before[0].attempt_id);
    assert!(
        after[0].id > 0,
        "read switches to the durable database fact"
    );
    assert_eq!(state.proxy_runtime_attempts.health_snapshot().records, 0);
    state.shutdown.cancel();
    state.sqlite_batch_writer.shutdown_and_drain().await;
}
