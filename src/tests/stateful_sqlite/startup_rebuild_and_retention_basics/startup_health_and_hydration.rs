use super::super::*;
#[tokio::test]
async fn health_check_reports_starting_until_startup_is_ready() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18080").expect("valid upstream url"),
    )
    .await;
    state.startup_ready.store(false, Ordering::Release);
    let response = health_check(State(state.clone())).await.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read health body");
    assert_eq!(std::str::from_utf8(&body).expect("utf8 body"), "starting");
    state.startup_ready.store(true, Ordering::Release);
    let response = health_check(State(state)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read health body");
    assert_eq!(std::str::from_utf8(&body).expect("utf8 body"), "ok");
}

#[tokio::test]
async fn http_readiness_is_published_before_prompt_cache_materialization_completes() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18080").expect("valid upstream url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        ) VALUES ('readiness-pending-invocation', '2026-09-01 00:00:00', ?1, 'success', 12, 0.25, ?2, '{}')
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(json!({"promptCacheKey": "readiness-pending-key"}).to_string())
    .execute(&state.pool)
    .await
    .expect("insert readiness migration fixture");
    assert!(
        !prompt_cache_conversation_materialization_is_complete(&state.pool)
            .await
            .expect("check pending prompt-cache materialization")
    );
    state.startup_ready.store(false, Ordering::Release);
    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_delay(
            state.clone(),
            Instant::now(),
            Duration::from_secs(30),
        );
    let response = health_check(State(state.clone())).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !prompt_cache_conversation_materialization_is_complete(&state.pool)
            .await
            .expect("readiness must not imply prompt-cache materialization completion")
    );
    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), hydration_handle)
        .await
        .expect("startup hydration should stop after readiness test shutdown")
        .expect("startup hydration should join after readiness test shutdown");
    state.pool.close().await;
}
#[tokio::test]
async fn startup_hot_read_hydration_keeps_health_ready_under_sqlite_pool_pressure() {
    let _target_timing = test_timing_target(
        "tests::stateful_sqlite::startup_rebuild_and_retention_basics::startup_health_and_hydration::startup_hot_read_hydration_keeps_health_ready_under_sqlite_pool_pressure",
    );
    let _whole_test = test_timing_span("whole_test");
    let (mut state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
        "startup-hot-read-readiness-pressure",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    enable_test_admin_auth(&mut state);
    state.startup_ready.store(false, Ordering::Release);
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server before startup hydration");
    let mut held_connections = Vec::new();
    for _ in 0..5 {
        held_connections.push(
            state
                .pool
                .acquire()
                .await
                .expect("saturate sqlite pool before startup hydration"),
        );
    }
    let builder = reqwest::Client::builder().no_proxy(); // Setup outside readiness timing.
    let client = builder.build().expect("local readiness client");
    let hydration_handle = crate::test_timed!(
        "production_spawn",
        publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_deadline(
            state.clone(),
            Instant::now(),
            Duration::from_secs(4),
        )
    );
    let health = tokio::time::timeout(
        Duration::from_millis(250),
        client.get(format!("http://{addr}/health")).send(),
    )
    .await
    .expect("health must not wait for SQLite hydration")
    .expect("health endpoint should respond while SQLite pool is saturated");
    assert_eq!(health.status(), StatusCode::OK);
    for endpoint in ["/api/stats/summary?window=today", "/api/system/status"] {
        let response = tokio::time::timeout(
            Duration::from_millis(250),
            client
                .get(format!("http://{addr}{endpoint}"))
                .header(reqwest::header::AUTHORIZATION, test_admin_authorization())
                .send(),
        )
        .await
        .expect("hot-read request must not wait for SQLite during startup")
        .expect("hot-read endpoint should return its unavailable contract");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    timed_test_phase("explicit_wait", tokio::time::sleep(Duration::from_secs(5))).await;
    assert!(
        !hydration_handle.is_finished(),
        "the background worker retries after its bounded hydration attempt times out"
    );
    drop(held_connections);
    crate::test_timed!(
        "production_operation",
        tokio::time::timeout(Duration::from_secs(5), hydration_handle)
            .await
            .expect("background startup hydration should recover after SQLite pressure clears")
            .expect("background startup hydration task should join")
    );

    for endpoint in ["/api/stats/summary?window=today", "/api/system/status"] {
        let response = client
            .get(format!("http://{addr}{endpoint}"))
            .header(reqwest::header::AUTHORIZATION, test_admin_authorization())
            .send()
            .await
            .expect("hydrated hot-read endpoint should respond");
        assert_eq!(response.status(), StatusCode::OK);
    }
    state.shutdown.cancel();
    server_handle
        .await
        .expect("http server should stop after startup pressure test");
    state.pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}
#[tokio::test]
async fn startup_summary_hydration_retries_after_an_in_flight_refresh() {
    let (state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
        "startup-summary-hydration-single-flight",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let refresh_guard = state
        .subscription_hub
        .try_lock_summary_projection_refresh()
        .expect("hold the summary maintenance refresh lock");
    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration(state.clone(), Instant::now());
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state.system_status_cache.lock().await.latest.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("system status hydration should complete while summary refresh is in flight");
    assert!(
        !hydration_handle.is_finished(),
        "startup hydration must retry instead of treating a coalesced summary refresh as complete"
    );
    drop(refresh_guard);
    tokio::time::timeout(Duration::from_secs(3), hydration_handle)
        .await
        .expect("startup hydration should retry after the in-flight refresh releases")
        .expect("startup hydration task should join after the retry");
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_backfill_defers_legacy_mirror_work_until_cold_summary_is_published() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18080").expect("valid upstream URL"),
    )
    .await;
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(30));

    // Closing the pool turns this into a zero-SQL assertion. Cold startup mirror recovery owns
    // the first proof sweep, so the generic queue must defer before it asks SQLite for progress.
    state.pool.close().await;
    let ran = run_startup_backfill_task_if_due_with_gate(
        &state,
        StartupBackfillTask::LegacyDetailMirrors,
        &gate,
    )
    .await
    .expect("cold Summary ownership should defer the generic legacy mirror task without SQLite");

    assert!(!ran);
    assert_eq!(gate.snapshot().background_skips, 0);
}

#[tokio::test]
async fn startup_summary_hydration_publishes_before_legacy_mirror_recovery() {
    let (state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
        "startup-summary-legacy-mirror-recovery",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let archive_path = seed_invocation_archive_batch_with_details(
        &state.pool,
        &state.config,
        "startup-summary-legacy-mirror",
        &[SeedInvocationArchiveBatchRow {
            id: 41,
            invoke_id: "startup-summary-legacy-mirror-41",
            occurred_at: occurred_at.as_str(),
            source: SOURCE_PROXY,
            status: "success",
            total_tokens: 12,
            cost: 0.12,
            ttfb_ms: Some(120.0),
            payload: Some("{}"),
            detail_level: DETAIL_LEVEL_FULL,
            error_message: None,
            failure_kind: None,
            failure_class: Some("none"),
            is_actionable: Some(0),
        }],
    )
    .await;
    let archive_file_path = archive_path.to_string_lossy().to_string();
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            id, invoke_id, occurred_at, source, status, total_tokens, cost,
            t_upstream_ttfb_ms, payload, detail_level, failure_class, is_actionable,
            raw_response, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, '{}', ?3)
        "#,
    )
    .bind(41_i64)
    .bind("startup-summary-legacy-mirror-41")
    .bind(&occurred_at)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(12_i64)
    .bind(0.12_f64)
    .bind(120.0_f64)
    .bind("{}")
    .bind(DETAIL_LEVEL_FULL)
    .bind("none")
    .bind(0_i64)
    .execute(&state.pool)
    .await
    .expect("retain canonical live identity for the legacy detail mirror");

    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_deadline(
            state.clone(),
            Instant::now(),
            Duration::from_secs(4),
        );
    tokio::time::timeout(Duration::from_secs(8), hydration_handle)
        .await
        .expect("Summary hydration must not wait for legacy mirror recovery")
        .expect("startup hydration coordinator should join");

    let source_kind: String = sqlx::query_scalar(
        "SELECT summary_source_kind FROM archive_batches WHERE dataset = ?1 AND file_path = ?2",
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .bind(&archive_file_path)
    .fetch_one(&state.pool)
    .await
    .expect("load deferred archive source role");
    assert_eq!(source_kind, SUMMARY_ARCHIVE_SOURCE_KIND_UNKNOWN);
    assert!(state.subscription_hub.summary_projection().await.is_some());

    state.pool.close().await;
    let summary = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect_err("an unclassified archive must remain range-local unavailable after publication");
    let ApiError::Unavailable(summary) = summary else {
        panic!("an unclassified archive must use the unavailable response: {summary:?}");
    };
    assert!(
        format!("{summary}").contains("current source is unavailable"),
        "the unavailable response must preserve the exact source boundary: {summary}"
    );

    state.shutdown.cancel();
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn startup_system_status_hydrates_while_summary_hydration_is_delayed() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18080").expect("valid upstream url"),
    )
    .await;
    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_delay(
            state.clone(),
            Instant::now(),
            Duration::from_secs(2),
        );

    tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            if state.system_status_cache.lock().await.latest.is_some() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("system status hydration must not wait for the delayed summary worker");
    assert!(
        !hydration_handle.is_finished(),
        "the returned coordinator must continue joining the delayed summary worker"
    );

    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), hydration_handle)
        .await
        .expect("startup hydration coordinator should cancel both independent workers")
        .expect("startup hydration coordinator should join after shutdown");
    state.pool.close().await;
}

#[tokio::test]
async fn runtime_drain_joins_pending_startup_hydration_after_http_readiness() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18080").expect("valid upstream url"),
    )
    .await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn HTTP server before startup hydration");
    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration_with_test_summary_delay(
            state.clone(),
            Instant::now(),
            Duration::from_secs(30),
        );

    let health = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond after readiness is published");
    assert_eq!(health.status(), StatusCode::OK);
    assert!(
        !hydration_handle.is_finished(),
        "the delayed startup hydration coordinator should still be pending at shutdown"
    );

    begin_runtime_shutdown(&state.shutdown);
    tokio::time::timeout(
        Duration::from_secs(1),
        drain_runtime_after_shutdown(
            state.clone(),
            Some(server_handle),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(hydration_handle),
        ),
    )
    .await
    .expect("runtime drain should not wait for abandoned startup hydration subwork")
    .expect("runtime drain should join the pending startup hydration coordinator");

    state.pool.close().await;
}

#[tokio::test]
async fn summary_startup_hydration_publishes_bounded_fallback_under_sqlite_pressure() {
    let _target_timing = test_timing_target(
        "tests::stateful_sqlite::startup_rebuild_and_retention_basics::startup_health_and_hydration::summary_startup_hydration_publishes_bounded_fallback_under_sqlite_pressure",
    );
    let _whole_test = test_timing_span("whole_test");
    let (state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
        "startup-summary-hydration-deadline",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let mut held_connections = Vec::new();
    for _ in 0..5 {
        held_connections.push(
            state
                .pool
                .acquire()
                .await
                .expect("saturate sqlite pool before summary hydration"),
        );
    }
    let hydration_state = state.clone();
    let hydration = tokio::spawn(async move {
        hydrate_summary_snapshots_with_deadline(hydration_state.as_ref(), Duration::from_secs(4))
            .await
    });
    timed_test_phase("explicit_wait", tokio::time::sleep(Duration::from_secs(5))).await;
    drop(held_connections);
    crate::test_timed!(
        "production_operation",
        tokio::time::timeout(Duration::from_secs(20), hydration)
            .await
            .expect("summary hydration must finish its bounded fallback after pressure clears")
            .expect("join summary hydration fallback")
            .expect("summary hydration fallback should publish without an internal error")
    );
    assert!(
        state.subscription_hub.summary_projection().await.is_some(),
        "saturated SQLite pool should still publish the bounded recent fallback"
    );
    state.pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn system_status_startup_hydration_has_a_finite_sqlite_pressure_deadline() {
    let _target_timing = test_timing_target(
        "tests::stateful_sqlite::startup_rebuild_and_retention_basics::startup_health_and_hydration::system_status_startup_hydration_has_a_finite_sqlite_pressure_deadline",
    );
    let _whole_test = test_timing_span("whole_test");
    let (state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
        "startup-system-status-hydration-deadline",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let mut held_connections = Vec::new();
    for _ in 0..5 {
        held_connections.push(
            state
                .pool
                .acquire()
                .await
                .expect("saturate sqlite pool before system status hydration"),
        );
    }

    let result = crate::test_timed!(
        "production_operation",
        tokio::time::timeout(
            Duration::from_secs(5),
            hydrate_system_status_snapshot(state.as_ref()),
        )
        .await
        .expect("system status hydration must finish its bounded attempt under SQLite pressure")
        .expect_err(
            "saturated SQLite pool should fail the bounded system status hydration attempt"
        )
    );
    assert!(
        result.to_string().contains("exceeded"),
        "system status hydration should report its finite refresh deadline: {result:#}"
    );

    drop(held_connections);
    state.pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn startup_hot_read_hydration_cancels_while_sqlite_pool_is_saturated() {
    let (state, temp_dir, _db_url) = file_backed_test_state_from_current_schema(
        "startup-hot-read-hydration-cancellation",
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .await;
    let mut held_connections = Vec::new();
    for _ in 0..5 {
        held_connections.push(
            state
                .pool
                .acquire()
                .await
                .expect("saturate sqlite pool before startup hydration"),
        );
    }

    let hydration_handle =
        publish_http_readiness_and_spawn_hot_read_hydration(state.clone(), Instant::now());
    state.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), hydration_handle)
        .await
        .expect("startup hydration should observe shutdown without waiting for SQLite")
        .expect("startup hydration task should join after shutdown");

    drop(held_connections);
    state.pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}
