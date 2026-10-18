use super::*;

#[tokio::test]
async fn health_is_ready_while_locked_recovery_resumes_after_release() {
    let (mut state, temp_dir, db_url) = file_backed_test_state_with_busy_timeout(
        "startup-recovery-coordinator-readiness",
        Duration::from_millis(200),
    )
    .await;
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, payload, raw_response) VALUES ('coordinator-pending-recovery', '2026-10-18T00:00:00Z', 'proxy', 'running', 0, '{}', '{}')",
    )
    .execute(&state.pool)
    .await
    .expect("insert coordinator recovery fixture");
    let invocation_id: i64 = sqlx::query_scalar(
        "SELECT id FROM codex_invocations WHERE invoke_id = 'coordinator-pending-recovery'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load coordinator recovery fixture id");
    let recovery_options = build_sqlite_connect_options(&db_url, Duration::from_millis(200))
        .expect("build startup recovery connection options");
    let mut lock_connection = SqliteConnection::connect_with(&recovery_options)
        .await
        .expect("connect startup recovery lock fixture");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_connection)
        .await
        .expect("hold startup recovery write lock");

    let prompt_cache = state.prompt_cache_conversation_cache.clone();
    let pool = state.pool.clone();
    let shutdown = state.shutdown.clone();
    let pricing_catalog = state.pricing_catalog.clone();
    let database_path = state.config.database_path.clone();
    let writer = SqliteBatchWriter::spawn(
        pool,
        shutdown,
        prompt_cache,
        pricing_catalog,
        &database_path,
    )
    .expect("spawn file-backed sqlite writer for coordinator fixture");
    writer.set_terminal_runtime_store(state.proxy_runtime_invocations.clone());
    writer.set_runtime_attempt_store(state.proxy_runtime_attempts.clone());
    writer.set_dashboard_activity_snapshot_cache(state.dashboard_activity_snapshot_cache.clone());
    writer.set_summary_delta_hub(state.subscription_hub.clone());
    writer.set_terminal_projection_hub(state.terminal_projection_hub.clone());
    Arc::get_mut(&mut state)
        .expect("coordinator fixture must be uniquely owned")
        .sqlite_batch_writer = writer;

    state.startup_ready.store(false, Ordering::Release);
    let app = build_app_router(state.clone());
    let recovery_handle = publish_http_readiness_and_spawn_recovery_for_test(
        state.clone(),
        Instant::now(),
        startup_recovery_fence_for_test(invocation_id, 0),
        recovery_options,
    );
    let health_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("build ready health request"),
        )
        .await
        .expect("serve ready health request");
    assert_eq!(health_response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(health_response.into_body(), usize::MAX)
            .await
            .expect("read ready health response body"),
        Bytes::from_static(b"ok")
    );

    tokio::time::sleep(Duration::from_millis(75)).await;
    let status_while_locked: String =
        sqlx::query_scalar("SELECT status FROM codex_invocations WHERE id = ?")
            .bind(invocation_id)
            .fetch_one(&state.pool)
            .await
            .expect("read locked recovery fixture status");
    assert_eq!(status_while_locked, "running");
    assert!(
        !recovery_handle.is_finished(),
        "recovery coordinator must remain pending behind the SQLite lock"
    );

    sqlx::query("COMMIT")
        .execute(&mut lock_connection)
        .await
        .expect("release startup recovery write lock");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status: String =
                sqlx::query_scalar("SELECT status FROM codex_invocations WHERE id = ?")
                    .bind(invocation_id)
                    .fetch_one(&state.pool)
                    .await
                    .expect("read recovered fixture status");
            if status == INVOCATION_STATUS_INTERRUPTED {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("recovery coordinator should resume after SQLite lock release");

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), recovery_handle)
        .await
        .expect("recovery coordinator should stop after fixture shutdown")
        .expect("recovery coordinator should join cleanly");
    state.sqlite_batch_writer.shutdown_and_drain().await;
    lock_connection
        .close()
        .await
        .expect("close startup recovery lock connection");
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn health_becomes_ready_before_pending_recovery_completes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, payload, raw_response) VALUES ('pending-startup-recovery', '2026-10-18T00:00:00Z', 'proxy', 'running', 0, '{}', '{}')",
    )
    .execute(&state.pool)
    .await
    .expect("insert pending startup recovery fixture");
    state.startup_ready.store(false, Ordering::Release);
    let app = build_app_router(state.clone());

    let business_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/system/status")
                .body(Body::empty())
                .expect("build business request"),
        )
        .await
        .expect("serve business request");
    assert_eq!(business_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        axum::body::to_bytes(business_response.into_body(), usize::MAX)
            .await
            .expect("read business response body"),
        Bytes::from_static(b"starting")
    );

    let health_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("build health request"),
        )
        .await
        .expect("serve health request");
    assert_eq!(health_response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        axum::body::to_bytes(health_response.into_body(), usize::MAX)
            .await
            .expect("read health response body"),
        Bytes::from_static(b"starting")
    );

    state.startup_ready.store(true, Ordering::Release);
    let ready_response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("build ready health request"),
        )
        .await
        .expect("serve ready health request");
    assert_eq!(ready_response.status(), StatusCode::OK);
    assert_eq!(
        axum::body::to_bytes(ready_response.into_body(), usize::MAX)
            .await
            .expect("read ready health response body"),
        Bytes::from_static(b"ok")
    );
}

#[tokio::test]
async fn id_scoped_recovery_does_not_touch_rows_outside_the_startup_fence() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    for invoke_id in ["startup-fence-before", "startup-fence-after"] {
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, payload, raw_response) VALUES (?, ?, 'proxy', 'running', 0, '{}', '{}')",
        )
        .bind(invoke_id)
        .bind(if invoke_id.ends_with("before") {
            "2026-10-18T00:00:00Z"
        } else {
            "2026-10-18T00:00:01Z"
        })
        .execute(&state.pool)
        .await
        .expect("insert startup recovery fixture");
    }
    let ids = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM codex_invocations WHERE invoke_id LIKE 'startup-fence-%' ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load startup recovery fixture ids");
    assert_eq!(ids.len(), 2);

    let recovered = crate::recover_proxy_invocations_with_scope(
        &state.pool,
        crate::ProxyInvocationRecoveryScope::Ids(&ids[..1]),
    )
    .await
    .expect("recover only fenced invocation ids");
    assert_eq!(recovered.len(), 1);

    let statuses = sqlx::query_as::<_, (i64, String)>(
        "SELECT id, status FROM codex_invocations WHERE invoke_id LIKE 'startup-fence-%' ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load startup recovery fixture statuses");
    assert_eq!(statuses[0].0, ids[0]);
    assert_eq!(statuses[0].1, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(statuses[1].0, ids[1]);
    assert_eq!(statuses[1].1, "running");
}
